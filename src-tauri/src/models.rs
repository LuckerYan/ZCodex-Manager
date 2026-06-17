use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    pub id: i64,
    pub alias: String,
    pub email: Option<String>,
    pub user_id: Option<String>,
    pub display_name: Option<String>,
    pub active_provider: String,
    pub zai_access_token_enc: String,
    pub zcode_jwt_token_enc: String,
    pub user_info_json: Value,
    pub config_providers_json: Value,
    pub coding_plan_cache_json: Value,
    pub hot_switch_ready: bool,
    pub source: String,
    pub note: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_auth_at: Option<i64>,
    pub last_quota_refresh_at: Option<i64>,
    pub quota_plan_name: Option<String>,
    pub quota_plan_status: Option<String>,
    pub quota_plan_ends_at: Option<i64>,
    pub quota_models_json: Value,
    pub quota_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUpsert {
    pub alias: String,
    pub email: Option<String>,
    pub user_id: Option<String>,
    pub display_name: Option<String>,
    pub active_provider: String,
    pub zai_access_token_enc: String,
    pub zcode_jwt_token_enc: String,
    pub user_info_json: Value,
    pub config_providers_json: Value,
    pub coding_plan_cache_json: Value,
    pub hot_switch_ready: bool,
    pub source: String,
    pub note: Option<String>,
    pub last_auth_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelQuotaDto {
    pub model: String,
    pub show_name: String,
    pub total_units: i64,
    pub used_units: i64,
    pub remaining_units: i64,
    pub available_units: i64,
    pub remaining_percent: f64,
    pub period_end: Option<i64>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRowDto {
    pub id: i64,
    pub alias: String,
    pub email: Option<String>,
    pub user_id: Option<String>,
    pub display_name: Option<String>,
    pub hot_switch_ready: bool,
    pub is_active: bool,
    pub source: String,
    pub note: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_quota_refresh_at: Option<i64>,
    pub plan_name: Option<String>,
    pub plan_status: Option<String>,
    pub plan_ends_at: Option<i64>,
    pub models: Vec<ModelQuotaDto>,
    pub quota_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStartDto {
    pub flow_id: String,
    pub authorize_url: String,
    pub expires_at: i64,
    pub poll_interval_sec: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCompleteDto {
    pub account: AccountRowDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotSwitchResultDto {
    pub account_id: i64,
    pub alias: String,
    pub backup_dir: Option<String>,
    pub agent_pids: Vec<u32>,
    pub killed_count: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZcodeStatusDto {
    pub zcode_v2_dir: String,
    pub credentials_exists: bool,
    pub config_exists: bool,
    pub cache_exists: bool,
    pub agent_pids: Vec<u32>,
    /// ZCode 主进程是否正在运行（决定左下角指示灯）。
    pub zcode_running: bool,
    /// ZCode 是否带调试端口(9229)启动 —— 决定热切换能否自动刷新左下角 UI。
    pub cdp_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelaunchResultDto {
    pub exe_path: String,
    pub debug_port: u16,
    pub cdp_available: bool,
    pub message: String,
}

// ───────────────────────── 使用统计（读取 ZCode CLI db.sqlite）─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageDailyPoint {
    /// 本地时区日期 YYYY-MM-DD
    pub date: String,
    pub tokens: i64,
    pub requests: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageDailyModelPoint {
    /// 本地时区日期 YYYY-MM-DD
    pub date: String,
    pub model: String,
    pub tokens: i64,
    pub requests: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageModelRow {
    pub model: String,
    pub requests: i64,
    pub tokens: i64,
    pub input: i64,
    pub output: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageToolRow {
    pub tool: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageHeatCell {
    /// 本地时区日期 YYYY-MM-DD
    pub date: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStatsDto {
    pub available: bool,
    pub db_path: String,
    pub generated_at: i64,
    // 总览（仅统计 status='completed' 的请求）
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub model_request_count: i64,
    pub session_count: i64,
    pub project_count: i64,
    pub tool_call_count: i64,
    /// 最早/最近一次模型请求时间（毫秒时间戳）；无数据为 0
    pub first_at: i64,
    pub last_at: i64,
    // 明细
    pub daily: Vec<UsageDailyPoint>,
    pub daily_by_model: Vec<UsageDailyModelPoint>,
    pub by_model: Vec<UsageModelRow>,
    pub by_tool: Vec<UsageToolRow>,
    pub heatmap: Vec<UsageHeatCell>,
}
