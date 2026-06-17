use chrono::Utc;
use serde_json::Value;
use std::path::PathBuf;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::models::{AccountRecord, HotSwitchResultDto, RelaunchResultDto, ZcodeStatusDto};
use crate::zcode::api::decrypt_db_token;
use crate::zcode::cdp;
use crate::zcode::config::{synthesize_providers, write_coding_plan_cache, write_providers};
use crate::zcode::credentials::write_credentials_from_account;
use crate::zcode::paths::{zcode_paths, ZCodePaths};
use crate::zcode::process::{
    find_agent_processes, find_zcode_exe_path, is_zcode_running, kill_processes, launch_zcode,
    quit_zcode,
};

pub fn status() -> AppResult<ZcodeStatusDto> {
    let paths = zcode_paths()?;
    Ok(ZcodeStatusDto {
        zcode_v2_dir: paths.v2_dir.to_string_lossy().to_string(),
        credentials_exists: paths.credentials_path.exists(),
        config_exists: paths.config_path.exists(),
        cache_exists: paths.cache_path.exists(),
        // 状态灯是高频轮询路径，只判断 ZCode 主窗口是否运行。
        // agent 子进程需要读取 CommandLine，Windows 上通常要走 CIM/PowerShell，放在热切换时再查，
        // 避免每 3 秒一次的状态刷新把 WebView 拖成“未响应”。
        agent_pids: Vec::new(),
        zcode_running: is_zcode_running(),
        cdp_available: cdp::is_available(cdp::CDP_PORT),
    })
}

pub fn hot_switch(db: &Db, account_id: i64, no_backup: bool) -> AppResult<HotSwitchResultDto> {
    let account = db.get_account(account_id)?;
    let paths = zcode_paths()?;
    let backup_dir = if no_backup { None } else { Some(backup_files(&paths)?) };

    let access = decrypt_db_token(&account.zai_access_token_enc)?;
    let jwt = decrypt_db_token(&account.zcode_jwt_token_enc)?;
    let user_info = normalized_user_info(&account);
    write_credentials_from_account(&paths, &access, &jwt, &user_info)?;

    // 决定要写入的 provider/cache。缺 config 快照(浏览器登录账号)时自动合成:
    // 否则 config.json 会保留上一个账号的 apiKey,agent 重启仍认作旧账号,ZCode UI 名不更新。
    let (providers, cache, synth_note) = resolve_config_to_write(db, &account, &jwt)?;
    if let Some(ref p) = providers {
        write_providers(&paths, p)?;
    }
    if let Some(ref c) = cache {
        if is_non_empty_object(c) {
            write_coding_plan_cache(&paths, c)?;
        }
    }

    let mut agent_warning = String::new();
    let (agents, killed) = match find_agent_processes() {
        Ok(agents) => {
            let killed = match kill_processes(&agents) {
                Ok(n) => n,
                Err(e) => {
                    agent_warning = format!(
                        "已写入账号凭证和 config，但重启 ZCode agent 失败：{e}；如当前对话仍用旧账号，请手动重启 ZCode。"
                    );
                    0
                }
            };
            (agents, killed)
        }
        Err(e) => {
            agent_warning = format!(
                "已写入账号凭证和 config，但查询 ZCode agent 进程失败：{e}；如当前对话仍用旧账号，请手动重启 ZCode。"
            );
            (Vec::new(), 0)
        }
    };

    // 写盘 + 重启 agent 后，不再让 ZCode renderer 整页 reload；只通过 CDP 局部更新左下角账号名。
    // 真实 DOM 形态来自 ZCode renderer: button[data-testid="login-trigger"] 内部的用户名 div。
    let ui_account_name = account
        .display_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| account.email.as_deref().filter(|s| !s.trim().is_empty()))
        .unwrap_or(&account.alias)
        .trim()
        .to_string();
    let ui_note = if cdp::is_available(cdp::CDP_PORT) {
        match cdp::update_account_name(cdp::CDP_PORT, &ui_account_name) {
            Ok(n) if n > 0 => format!("已局部更新 ZCode 左下角账号名（{n} 个窗口），未刷新页面。"),
            Ok(_) => "已写入账号凭证，但未在 ZCode 页面找到左下角账号名节点；页面未刷新。".to_string(),
            Err(e) => format!("局部更新 ZCode 左下角账号名失败：{e}；页面未刷新。"),
        }
    } else {
        "ZCode 未带调试端口启动，左下角账号名暂不会自动局部更新；点「调试模式重启 ZCode」启用。".to_string()
    };

    let agent_note = if !agent_warning.is_empty() {
        agent_warning
    } else if agents.is_empty() {
        "未发现运行中的 ZCode agent，下次生成 agent 时会读取新账号。".to_string()
    } else {
        format!("已重启 {killed}/{} 个 ZCode agent 子进程。", agents.len())
    };
    let message = format!("{synth_note}{agent_note} {ui_note}");
    db.record_switch(
        account_id,
        true,
        &message,
        backup_dir.as_ref().map(|p| p.to_string_lossy()).as_deref(),
    )?;
    Ok(HotSwitchResultDto {
        account_id,
        alias: account.alias,
        backup_dir: backup_dir.map(|p| p.to_string_lossy().to_string()),
        agent_pids: agents,
        killed_count: killed,
        message,
    })
}

/// 优雅退出 ZCode 并带 `--remote-debugging-port` 重启,启用热切换后的 UI 自动刷新。
/// ZCode 打包版默认不开该端口,且单实例运行时无法附加,故必须重启一次。
pub fn relaunch_zcode_debug() -> AppResult<RelaunchResultDto> {
    let port = cdp::CDP_PORT;
    // 趁 ZCode 还在跑,先拿到 exe 路径(退出后运行进程就查不到了)
    let exe = find_zcode_exe_path()
        .ok_or_else(|| AppError::Message("未找到 ZCode.exe 路径,无法重启".to_string()))?;
    quit_zcode()?;
    std::thread::sleep(std::time::Duration::from_millis(1200));
    launch_zcode(&exe, &[format!("--remote-debugging-port={port}")])?;

    // 等待 CDP 端口就绪(ZCode 启动需要时间)
    let mut available = false;
    for _ in 0..30 {
        if cdp::is_available(port) {
            available = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let message = if available {
        format!("ZCode 已带调试端口 {port} 重启,之后热切换会自动刷新左下角账号名。")
    } else {
        format!("ZCode 已重启,但调试端口 {port} 暂未就绪(可能仍在启动),稍后热切换会自动重试。")
    };
    Ok(RelaunchResultDto { exe_path: exe, debug_port: port, cdp_available: available, message })
}

/// 决定本次热切换要写入磁盘的 provider 配置与 coding-plan-cache，并返回给用户的提示。
///
/// - 账号自带 config 快照(import-current 抓到的)→ 直接用。
/// - 缺快照(浏览器登录账号 config_providers={})→ 取池里有 config 的账号当模板，
///   按 "start-plan apiKey == 账号 jwt" 铁律把模板的 jwt-apiKey 换成目标 jwt 自动合成，
///   并回存目标账号(下次直接用 + 标记可热切换)。
/// - 池里也没有模板 → 返回 (None, None, 提示)，退回仅换 credentials 的老行为。
fn resolve_config_to_write(
    db: &Db,
    account: &AccountRecord,
    target_jwt: &str,
) -> AppResult<(Option<Value>, Option<Value>, String)> {
    if is_non_empty_object(&account.config_providers_json) {
        let cache = if is_non_empty_object(&account.coding_plan_cache_json) {
            Some(account.coding_plan_cache_json.clone())
        } else {
            None
        };
        return Ok((Some(account.config_providers_json.clone()), cache, String::new()));
    }

    match db.find_template_with_config(account.id)? {
        Some(template) => {
            let template_jwt = decrypt_db_token(&template.zcode_jwt_token_enc)?;
            let (synth, replaced) =
                synthesize_providers(&template.config_providers_json, &template_jwt, target_jwt);
            // 回存到目标账号：下次热切换走"自带快照"分支，并把 UI 标记为可热切换
            db.set_account_config(account.id, &synth, &template.coding_plan_cache_json, replaced > 0)?;
            let note = format!(
                "(已用模板账号 [{}] 自动合成 config，替换 {replaced} 个 jwt-apiKey) ",
                template.alias
            );
            Ok((Some(synth), Some(template.coding_plan_cache_json.clone()), note))
        }
        None => Ok((
            None,
            None,
            "(⚠ 该账号无 config 快照且账号池中无可用模板，本次仅更换 credentials，ZCode UI 账号名可能不变；请先用 import-current 抓取一个同套餐账号作模板) ".to_string(),
        )),
    }
}

fn is_non_empty_object(value: &Value) -> bool {
    value.as_object().map(|m| !m.is_empty()).unwrap_or(false)
}

/// 部分从精简 JSON / 早期导入路径进入账号池的账号，表字段里有
/// display_name/email/user_id，但 user_info_json 仍是 `{}`。ZCode 左下角账号名
/// 和本管理器“当前”标记都依赖 credentials.json 里的 oauth:zai:user_info，
/// 因此热切换写 credentials 前必须把表字段补回 user_info。
fn normalized_user_info(account: &AccountRecord) -> Value {
    let mut user_info = account
        .user_info_json
        .as_object()
        .cloned()
        .unwrap_or_default();

    if !user_info.contains_key("name") {
        if let Some(name) = account.display_name.as_ref().filter(|s| !s.trim().is_empty()) {
            user_info.insert("name".to_string(), Value::String(name.clone()));
        }
    }
    if !user_info.contains_key("email") {
        if let Some(email) = account.email.as_ref().filter(|s| !s.trim().is_empty()) {
            user_info.insert("email".to_string(), Value::String(email.clone()));
        }
    }
    if !user_info.contains_key("user_id") {
        if let Some(user_id) = account.user_id.as_ref().filter(|s| !s.trim().is_empty()) {
            user_info.insert("user_id".to_string(), Value::String(user_id.clone()));
        }
    }

    Value::Object(user_info)
}

fn backup_files(paths: &ZCodePaths) -> AppResult<PathBuf> {
    let backup_dir = paths
        .v2_dir
        .join(format!("zcode-manager-hotswitch-backup-{}", Utc::now().format("%Y%m%d-%H%M%S")));
    std::fs::create_dir_all(&backup_dir)?;
    for path in [&paths.credentials_path, &paths.config_path, &paths.cache_path] {
        if path.exists() {
            if let Some(name) = path.file_name() {
                std::fs::copy(path, backup_dir.join(name))?;
            }
        }
    }
    Ok(backup_dir)
}
