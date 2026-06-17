use rand::RngCore;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::error::{AppError, AppResult};
use crate::models::{AccountUpsert, AuthStartDto, ModelQuotaDto};
use crate::zcode::cipher::ZCodeCipher;

const API_BASE: &str = "https://zcode.z.ai/api/v1";
const OAUTH_PROVIDER: &str = "zai";
const USER_AGENT: &str = "ZCode-Manager/0.1 (+https://zcode.ai; tauri)";

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    code: i64,
    msg: Option<String>,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct InitData {
    authorize_url: String,
    flow_id: String,
    expires_at: i64,
    poll_interval_sec: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct PollData {
    status: String,
    token: Option<String>,
    user: Option<Value>,
    zai: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSummary {
    pub plan_name: Option<String>,
    pub plan_status: Option<String>,
    pub plan_ends_at: Option<i64>,
    pub models: Vec<ModelQuotaDto>,
    pub raw_current: Value,
    pub raw_balance: Value,
}

#[derive(Debug, Serialize)]
struct InitRequestSer<'a> {
    provider: &'a str,
}

pub fn new_poll_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn init_browser_auth(poll_token: &str) -> AppResult<AuthStartDto> {
    let client = client()?;
    let resp = client
        .post(format!("{API_BASE}/oauth/cli/init"))
        .bearer_auth(poll_token)
        .json(&InitRequestSer { provider: OAUTH_PROVIDER })
        .send()?;
    let env: Envelope<InitData> = resp.json()?;
    if env.code != 0 {
        return Err(AppError::Api(env.msg.unwrap_or_else(|| format!("init code={}", env.code))));
    }
    let data = env.data.ok_or_else(|| AppError::Api("init 缺少 data".to_string()))?;
    Ok(AuthStartDto {
        flow_id: data.flow_id,
        authorize_url: data.authorize_url,
        expires_at: data.expires_at,
        poll_interval_sec: data.poll_interval_sec.unwrap_or(2),
    })
}

pub fn poll_browser_auth(flow_id: &str, poll_token: &str, alias: &str, note: Option<String>) -> AppResult<Option<AccountUpsert>> {
    let client = client()?;
    let resp = client
        .get(format!("{API_BASE}/oauth/cli/poll/{flow_id}"))
        .bearer_auth(poll_token)
        .send()?;
    let env: Envelope<PollData> = resp.json()?;
    if env.code != 0 {
        return Err(AppError::Api(env.msg.unwrap_or_else(|| format!("poll code={}", env.code))));
    }
    let data = env.data.ok_or_else(|| AppError::Api("poll 缺少 data".to_string()))?;
    match data.status.as_str() {
        "pending" => Ok(None),
        "failed" => Err(AppError::Api("授权失败 (status=failed)".to_string())),
        "ready" => {
            let jwt = data.token.ok_or_else(|| AppError::Api("ready 缺少 token".to_string()))?;
            let zai = data.zai.unwrap_or(Value::Object(Default::default()));
            let access = zai
                .get("access_token")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Api("ready 缺少 zai.access_token".to_string()))?
                .to_string();
            let user_info = data.user.unwrap_or(Value::Object(Default::default()));
            let email = user_info.get("email").and_then(Value::as_str).map(str::to_string);
            let user_id = user_info.get("user_id").and_then(Value::as_str).map(str::to_string);
            let display_name = user_info.get("name").and_then(Value::as_str).map(str::to_string);
            let cipher = ZCodeCipher::new()?;
            Ok(Some(AccountUpsert {
                alias: if alias.trim().is_empty() {
                    email.clone().unwrap_or_else(|| "browser-auth".to_string())
                } else {
                    alias.trim().to_string()
                },
                email,
                user_id,
                display_name,
                active_provider: "zai".to_string(),
                zai_access_token_enc: cipher.encrypt(&access)?,
                zcode_jwt_token_enc: cipher.encrypt(&jwt)?,
                user_info_json: user_info,
                config_providers_json: Value::Object(Default::default()),
                coding_plan_cache_json: Value::Object(Default::default()),
                hot_switch_ready: false,
                source: "browser_auth".to_string(),
                note,
                last_auth_at: Some(chrono::Utc::now().timestamp()),
            }))
        }
        other => Err(AppError::Api(format!("未知 poll status: {other}"))),
    }
}

pub fn fetch_quota(jwt: &str) -> AppResult<QuotaSummary> {
    let client = client()?;
    let current: Value = client
        .get(format!("{API_BASE}/zcode-plan/billing/current"))
        .bearer_auth(jwt)
        .send()?
        .json()?;
    assert_envelope_ok(&current, "current")?;
    let balance: Value = client
        .get(format!("{API_BASE}/zcode-plan/billing/balance"))
        .bearer_auth(jwt)
        .send()?
        .json()?;
    assert_envelope_ok(&balance, "balance")?;

    let plans = current
        .pointer("/data/plans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let first_plan = plans.first();
    let plan_name = first_plan
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let plan_status = first_plan
        .and_then(|p| p.get("status"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let plan_ends_at = first_plan
        .and_then(|p| p.get("ends_at"))
        .and_then(Value::as_i64);

    let mut models = Vec::new();
    if let Some(items) = balance.pointer("/data/balances").and_then(Value::as_array) {
        for item in items {
            let show_name = item
                .get("show_name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let model = item
                .get("capabilities")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .unwrap_or(&show_name)
                .trim_start_matches("model:")
                .to_string();
            let total = item.get("total_units").and_then(Value::as_i64).unwrap_or(0);
            let used = item.get("used_units").and_then(Value::as_i64).unwrap_or(0);
            let remaining = item.get("remaining_units").and_then(Value::as_i64).unwrap_or(0);
            let available = item.get("available_units").and_then(Value::as_i64).unwrap_or(0);
            let percent = if total > 0 {
                (remaining as f64 / total as f64 * 10000.0).round() / 100.0
            } else {
                0.0
            };
            models.push(ModelQuotaDto {
                model,
                show_name,
                total_units: total,
                used_units: used,
                remaining_units: remaining,
                available_units: available,
                remaining_percent: percent,
                period_end: item.get("period_end").and_then(Value::as_i64),
                expires_at: item.get("expires_at").and_then(Value::as_i64),
            });
        }
    }

    if plan_name.is_none() && models.is_empty() {
        return Err(AppError::Api(
            "接口未返回套餐或模型额度数据，可能该账号没有可用 Coding Plan、token 已失效或服务端返回空余额"
                .to_string(),
        ));
    }

    Ok(QuotaSummary {
        plan_name,
        plan_status,
        plan_ends_at,
        models,
        raw_current: current,
        raw_balance: balance,
    })
}

pub fn decrypt_db_token(token_enc: &str) -> AppResult<String> {
    ZCodeCipher::new()?.decrypt(token_enc)
}

fn client() -> AppResult<Client> {
    Ok(Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()?)
}

fn assert_envelope_ok(value: &Value, label: &str) -> AppResult<()> {
    let code = value.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        let msg = value
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("unknown api error");
        return Err(AppError::Api(format!("{label} code={code}: {msg}")));
    }
    Ok(())
}
