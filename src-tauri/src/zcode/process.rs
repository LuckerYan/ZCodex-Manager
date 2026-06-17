use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::error::{AppError, AppResult};

#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
#[cfg(target_os = "windows")]
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};

fn run_with_timeout(program: &str, args: &[&str], timeout: Duration) -> AppResult<Output> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::Message(format!("{program} 查询超时")));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn find_agent_processes() -> AppResult<Vec<u32>> {
    if cfg!(not(target_os = "windows")) {
        return Ok(vec![]);
    }
    let script = r#"
[Console]::OutputEncoding=[Text.Encoding]::UTF8
Get-CimInstance Win32_Process -Filter "Name='ZCode.exe'" |
  Where-Object { $_.CommandLine -like '*zcode.cjs*' -and $_.CommandLine -like '*app-server*' -and $_.CommandLine -like '*--stdio*' } |
  ForEach-Object { $_.ProcessId }
"#;
    let out = run_with_timeout(
        "powershell",
        &["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script],
        // Windows PowerShell 冷启动 + Win32_Process/CIM 首次查询经常超过 1.5s。
        // 这里不是高频状态轮询路径，只在热切换时执行，宁可多等几秒也不要误报失败。
        Duration::from_millis(6000),
    )?;
    if !out.status.success() {
        return Err(AppError::Message(format!(
            "查询 ZCode agent 进程失败: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut pids = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(pid) = line.parse::<u32>() {
            pids.push(pid);
        }
    }
    Ok(pids)
}

pub fn kill_processes(pids: &[u32]) -> AppResult<usize> {
    if pids.is_empty() {
        return Ok(0);
    }
    let mut killed = 0usize;
    for pid in pids {
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()?;
        if status.success() {
            killed += 1;
        }
    }
    Ok(killed)
}

/// 枚举所有 ZCode.exe 进程,返回 (pid, ExecutablePath)。exe 可能为空串。
pub fn list_zcode_processes() -> AppResult<Vec<(u32, String)>> {
    if cfg!(not(target_os = "windows")) {
        return Ok(vec![]);
    }
    #[cfg(target_os = "windows")]
    {
        list_zcode_processes_windows()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(vec![])
    }
}

pub fn is_zcode_running() -> bool {
    if cfg!(not(target_os = "windows")) {
        return false;
    }
    list_zcode_processes().map(|p| !p.is_empty()).unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn list_zcode_processes_windows() -> AppResult<Vec<(u32, String)>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(AppError::Message("创建进程快照失败".to_string()));
    }

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        cntUsage: 0,
        th32ProcessID: 0,
        th32DefaultHeapID: 0,
        th32ModuleID: 0,
        cntThreads: 0,
        th32ParentProcessID: 0,
        pcPriClassBase: 0,
        dwFlags: 0,
        szExeFile: [0; 260],
    };
    let mut procs = Vec::new();

    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) != 0 };
    while ok {
        let len = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]);
        if exe_name.eq_ignore_ascii_case("ZCode.exe") {
            // Toolhelp 快照能稳定拿 PID 和 exe 名；完整路径在部分权限/打包场景会受限。
            // find_zcode_exe_path() 会在路径为空时回退到已知安装位置。
            procs.push((entry.th32ProcessID, String::new()));
        }
        ok = unsafe { Process32NextW(snapshot, &mut entry) != 0 };
    }
    unsafe {
        CloseHandle(snapshot);
    }
    Ok(procs)
}

/// 探测 ZCode.exe 完整路径:优先运行进程的 ExecutablePath,否则回退已知安装位置。
pub fn find_zcode_exe_path() -> Option<String> {
    if let Ok(procs) = list_zcode_processes() {
        for (_pid, exe) in procs {
            if !exe.is_empty() && std::path::Path::new(&exe).exists() {
                return Some(exe);
            }
        }
    }
    let mut candidates = vec![r"D:\Apps_Installers\ZCode\ZCode.exe".to_string()];
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(format!(r"{local}\Programs\ZCode\ZCode.exe"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        candidates.push(format!(r"{pf}\ZCode\ZCode.exe"));
    }
    candidates.into_iter().find(|c| std::path::Path::new(c).exists())
}

/// 优雅退出所有 ZCode.exe:先 taskkill(无 /F,触发 app 落盘),超时再强杀进程树。
/// ZCode 是单实例(second-instance),必须完全退出后才能带新 flag 重启。
pub fn quit_zcode() -> AppResult<()> {
    if !is_zcode_running() {
        return Ok(());
    }
    let _ = Command::new("taskkill").args(["/IM", "ZCode.exe"]).output();
    for _ in 0..24 {
        if !is_zcode_running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    // 兜底:强杀整棵进程树
    let _ = Command::new("taskkill").args(["/IM", "ZCode.exe", "/F", "/T"]).output();
    for _ in 0..12 {
        if !is_zcode_running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(AppError::Message("ZCode 未能退出,请手动关闭后重试".to_string()))
}

/// 启动 ZCode(脱离父进程,不阻塞)。args 形如 ["--remote-debugging-port=9229"]。
pub fn launch_zcode(exe: &str, args: &[String]) -> AppResult<()> {
    let path = std::path::Path::new(exe);
    if !path.exists() {
        return Err(AppError::Message(format!("ZCode 路径不存在: {exe}")));
    }
    let mut cmd = Command::new(exe);
    cmd.args(args);
    if let Some(parent) = path.parent() {
        cmd.current_dir(parent);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP:脱离 manager,独立生存
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn()?;
    Ok(())
}
