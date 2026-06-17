use std::path::PathBuf;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct ZCodePaths {
    pub v2_dir: PathBuf,
    pub credentials_path: PathBuf,
    pub config_path: PathBuf,
    pub cache_path: PathBuf,
    /// ZCode v2 应用的任务索引库（仅任务/分组元数据，不含 token 统计）。
    pub tasks_index_path: PathBuf,
    /// ZCode CLI 自身的使用统计库（model_usage / session / tool_usage），「使用统计」数据源。
    pub cli_db_path: PathBuf,
}

pub fn zcode_paths() -> AppResult<ZCodePaths> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Path("无法定位 home 目录".to_string()))?;
    let zcode_root = home.join(".zcode");
    let v2_dir = zcode_root.join("v2");
    Ok(ZCodePaths {
        credentials_path: v2_dir.join("credentials.json"),
        config_path: v2_dir.join("config.json"),
        cache_path: v2_dir.join("coding-plan-cache.json"),
        tasks_index_path: v2_dir.join("tasks-index.sqlite"),
        cli_db_path: zcode_root.join("cli").join("db").join("db.sqlite"),
        v2_dir,
    })
}
