use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

mod db;
mod error;
mod models;
mod zcode;

use crate::db::{record_to_row, Db};
use crate::error::{AppError, AppResult};
use crate::models::{AccountRowDto, AccountUpsert, AuthCompleteDto, AuthStartDto, HotSwitchResultDto, RelaunchResultDto, UsageStatsDto, ZcodeStatusDto};
use crate::zcode::api::{decrypt_db_token, fetch_quota, init_browser_auth, new_poll_token, poll_browser_auth};
use crate::zcode::cipher::ZCodeCipher;
use crate::zcode::credentials::{import_current_account, read_decrypted_credentials};
use crate::zcode::paths::zcode_paths;
use crate::zcode::switcher;

pub struct AppState {
    db: Arc<Db>,
}

async fn run_blocking<T, F>(label: &'static str, task: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|e| AppError::Message(format!("{label} 后台任务失败: {e}")))?
}

#[tauri::command]
fn list_accounts(state: State<'_, AppState>) -> AppResult<Vec<AccountRowDto>> {
    let rows = state.db.list_accounts()?;
    // 读当前 credentials.json 找激活账号（Python cmd_list 同款逻辑）
    let active_credentials = zcode_paths()
        .ok()
        .and_then(|paths| read_decrypted_credentials(&paths).ok())
        .map(|plain| {
            let uid = plain
                .get("oauth:zai:user_info")
                .and_then(|v| v.get("user_id"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let jwt = plain
                .get("zcodejwttoken")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            (uid, jwt)
        });
    if let Some((active_uid, active_jwt)) = active_credentials {
        Ok(rows
            .into_iter()
            .map(|mut r| {
                let user_id_matches = active_uid
                    .as_deref()
                    .is_some_and(|uid| r.user_id.as_deref() == Some(uid));
                // 早期导入/精简导入账号可能表字段有 user_id，但 credentials 里的
                // oauth:zai:user_info 为空；此时用 zcodejwttoken 兜底判断当前账号。
                let token_matches = active_jwt.as_deref().is_some_and(|jwt| {
                    state
                        .db
                        .get_account(r.id)
                        .ok()
                        .and_then(|account| decrypt_db_token(&account.zcode_jwt_token_enc).ok())
                        .as_deref()
                        == Some(jwt)
                });
                if user_id_matches || token_matches {
                    r.is_active = true;
                }
                r
            })
            .collect())
    } else {
        Ok(rows)
    }
}

#[tauri::command]
async fn import_current(alias: Option<String>, note: Option<String>, state: State<'_, AppState>) -> AppResult<AccountRowDto> {
    let db = state.db.clone();
    run_blocking("同步当前账号", move || {
        let account = import_current_account(alias, note)?;
        let id = db.upsert_account(account)?;
        refresh_quota_inner(&db, id)
    })
    .await
}

#[tauri::command]
async fn start_browser_auth(_alias: Option<String>, _note: Option<String>, app: AppHandle) -> AppResult<AuthStartDto> {
    let dto = run_blocking("启动浏览器授权", move || {
        let poll_token = new_poll_token();
        let mut dto = init_browser_auth(&poll_token)?;
        // 把 poll_token 只返回给当前前端会话；不入库，ready 后才保存账号。
        // 为了少改 DTO schema，这里临时把 poll_token 拼进 authorize_url fragment 不安全；所以不这样做。
        // 实际返回通过 state 参数缺失不便，这里用 flow_id 承载 token: flow_id::poll_token，仅前端本地使用。
        let flow_id = dto.flow_id.clone();
        dto.flow_id = format!("{flow_id}::{poll_token}");
        Ok(dto)
    })
    .await?;
    if let Err(e) = app.opener().open_url(dto.authorize_url.clone(), None::<&str>) {
        eprintln!("ZCode Manager: 打开浏览器失败: {e}");
    }
    Ok(dto)
}

#[tauri::command]
async fn poll_auth(flow_id_token: String, alias: String, note: Option<String>, state: State<'_, AppState>) -> AppResult<Option<AuthCompleteDto>> {
    let db = state.db.clone();
    run_blocking("轮询授权状态", move || {
        let (flow_id, poll_token) = split_flow_token(&flow_id_token)?;
        match poll_browser_auth(flow_id, poll_token, &alias, note)? {
            Some(account) => {
                let id = db.upsert_account(account)?;
                let record = db.get_account(id)?;
                let row = record_to_row(record)?;
                Ok(Some(AuthCompleteDto { account: row }))
            }
            None => Ok(None),
        }
    })
    .await
}

#[tauri::command]
async fn refresh_quota(account_id: i64, state: State<'_, AppState>) -> AppResult<AccountRowDto> {
    let db = state.db.clone();
    run_blocking("刷新额度", move || refresh_quota_inner(&db, account_id)).await
}

#[tauri::command]
async fn refresh_all_quotas(state: State<'_, AppState>) -> AppResult<Vec<AccountRowDto>> {
    let db = state.db.clone();
    run_blocking("刷新全部额度", move || {
        let ids: Vec<i64> = db.list_accounts()?.into_iter().map(|a| a.id).collect();
        refresh_ids_parallel(db.clone(), ids);
        db.list_accounts()
    })
    .await
}

/// 仅刷新指定账号的额度（前端按勾选项调用）。
#[tauri::command]
async fn refresh_quotas(ids: Vec<i64>, state: State<'_, AppState>) -> AppResult<Vec<AccountRowDto>> {
    let db = state.db.clone();
    run_blocking("刷新所选额度", move || {
        refresh_ids_parallel(db.clone(), ids);
        db.list_accounts()
    })
    .await
}

fn refresh_ids_parallel(db: Arc<Db>, ids: Vec<i64>) {
    let handles: Vec<_> = ids
        .into_iter()
        .map(|id| {
            let db = db.clone();
            std::thread::spawn(move || {
                let _ = refresh_quota_inner(&db, id);
            })
        })
        .collect();
    for handle in handles {
        let _ = handle.join();
    }
}

fn refresh_quota_inner(db: &Db, account_id: i64) -> AppResult<AccountRowDto> {
    let account = db.get_account(account_id)?;
    let jwt = decrypt_db_token(&account.zcode_jwt_token_enc)?;
    match fetch_quota(&jwt) {
        Ok(summary) => {
            db.update_quota(
                account_id,
                summary.plan_name,
                summary.plan_status,
                summary.plan_ends_at,
                &summary.models,
                None,
            )?;
        }
        Err(e) => {
            db.update_quota_error(account_id, e.to_string())?;
        }
    }
    let record = db.get_account(account_id)?;
    record_to_row(record)
}

#[tauri::command]
async fn hot_switch(account_id: i64, no_backup: Option<bool>, state: State<'_, AppState>) -> AppResult<HotSwitchResultDto> {
    let db = state.db.clone();
    run_blocking("热切换", move || switcher::hot_switch(&db, account_id, no_backup.unwrap_or(false))).await
}

#[tauri::command]
async fn zcode_status() -> AppResult<ZcodeStatusDto> {
    run_blocking("读取 ZCode 状态", switcher::status).await
}

/// 读取 ZCode CLI 的使用统计（model_usage / session / tool_usage 聚合）。
#[tauri::command]
async fn read_usage_stats() -> AppResult<UsageStatsDto> {
    run_blocking("读取使用统计", crate::zcode::usage::read_usage_stats).await
}

#[tauri::command]
async fn relaunch_zcode_debug() -> AppResult<RelaunchResultDto> {
    run_blocking("调试模式重启 ZCode", switcher::relaunch_zcode_debug).await
}

#[tauri::command]
fn delete_accounts(ids: Vec<i64>, state: State<'_, AppState>) -> AppResult<usize> {
    state.db.delete_accounts(&ids)
}

/// 把所选账号导出到一个 JSON 文件（明文 token，便于跨机导入）。
#[tauri::command]
fn export_accounts_to_path(path: String, ids: Vec<i64>, state: State<'_, AppState>) -> AppResult<usize> {
    let cipher = ZCodeCipher::new()?;
    let mut out = Vec::new();
    for id in &ids {
        let r = state.db.get_account(*id)?;
        // 只导出身份标识 + token，去掉 user_info(含 base64 头像)/config 等大字段
        out.push(serde_json::json!({
            "alias": r.alias,
            "email": r.email,
            "user_id": r.user_id,
            "display_name": r.display_name,
            "zai_access_token": cipher.decrypt(&r.zai_access_token_enc)?,
            "zcode_jwt_token": cipher.decrypt(&r.zcode_jwt_token_enc)?,
        }));
    }
    let doc = serde_json::json!({
        "kind": "zcode-manager-accounts",
        "version": 1,
        "exported_at": chrono::Utc::now().timestamp(),
        "accounts": out,
    });
    std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
    Ok(ids.len())
}

/// 从 JSON 文件导入账号，兼容 {accounts:[...]} / 顶层数组 / 单个对象三种形态。
#[tauri::command]
fn import_accounts_from_path(path: String, state: State<'_, AppState>) -> AppResult<usize> {
    let text = std::fs::read_to_string(&path)?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| AppError::Message(format!("JSON 解析失败: {e}")))?;
    let items: Vec<serde_json::Value> = if let Some(arr) = value.get("accounts").and_then(|v| v.as_array()) {
        arr.clone()
    } else if let Some(arr) = value.as_array() {
        arr.clone()
    } else {
        vec![value]
    };
    if items.is_empty() {
        return Err(AppError::Message("文件中没有可导入的账号".to_string()));
    }
    let cipher = ZCodeCipher::new()?;
    let mut count = 0usize;
    for item in &items {
        let upsert = account_from_json(&cipher, item)?;
        state.db.upsert_account(upsert)?;
        count += 1;
    }
    Ok(count)
}

fn account_from_json(cipher: &ZCodeCipher, v: &serde_json::Value) -> AppResult<AccountUpsert> {
    let get_str = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let access_enc = match (get_str("zai_access_token"), get_str("zai_access_token_enc")) {
        (Some(p), _) => cipher.encrypt(&p)?,
        (None, Some(e)) => e,
        (None, None) => return Err(AppError::Message("账号缺少 zai_access_token".to_string())),
    };
    let jwt_enc = match (get_str("zcode_jwt_token"), get_str("zcode_jwt_token_enc")) {
        (Some(p), _) => cipher.encrypt(&p)?,
        (None, Some(e)) => e,
        (None, None) => return Err(AppError::Message("账号缺少 zcode_jwt_token".to_string())),
    };
    let alias = get_str("alias")
        .or_else(|| get_str("email"))
        .unwrap_or_else(|| "imported".to_string());
    Ok(AccountUpsert {
        alias,
        email: get_str("email"),
        user_id: get_str("user_id"),
        display_name: get_str("display_name"),
        active_provider: get_str("active_provider").unwrap_or_else(|| "zai".to_string()),
        zai_access_token_enc: access_enc,
        zcode_jwt_token_enc: jwt_enc,
        user_info_json: v.get("user_info").cloned().unwrap_or_else(|| serde_json::json!({})),
        config_providers_json: v.get("config_providers").cloned().unwrap_or_else(|| serde_json::json!({})),
        coding_plan_cache_json: v.get("coding_plan_cache").cloned().unwrap_or_else(|| serde_json::json!({})),
        hot_switch_ready: v.get("hot_switch_ready").and_then(|x| x.as_bool()).unwrap_or(false),
        source: get_str("source").unwrap_or_else(|| "imported_json".to_string()),
        note: get_str("note"),
        last_auth_at: Some(chrono::Utc::now().timestamp()),
    })
}

fn split_flow_token(flow_id_token: &str) -> AppResult<(&str, &str)> {
    flow_id_token
        .split_once("::")
        .ok_or_else(|| AppError::Message("auth flow 状态缺少 poll token".to_string()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = Db::open_default().expect("failed to open zcode-manager sqlite database");
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { db: Arc::new(db) })
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            import_current,
            start_browser_auth,
            poll_auth,
            refresh_quota,
            refresh_all_quotas,
            refresh_quotas,
            hot_switch,
            zcode_status,
            relaunch_zcode_debug,
            delete_accounts,
            export_accounts_to_path,
            import_accounts_from_path,
            read_usage_stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
