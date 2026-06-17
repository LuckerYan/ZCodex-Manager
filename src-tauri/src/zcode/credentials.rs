use serde_json::{Map, Value};

use crate::error::{AppError, AppResult};
use crate::models::AccountUpsert;
use crate::zcode::cipher::ZCodeCipher;
use crate::zcode::config::{read_coding_plan_cache, read_providers};
use crate::zcode::paths::{zcode_paths, ZCodePaths};

pub fn read_raw_credentials(paths: &ZCodePaths) -> AppResult<Map<String, Value>> {
    if !paths.credentials_path.exists() {
        return Err(AppError::Path(format!(
            "未找到 credentials.json: {}",
            paths.credentials_path.display()
        )));
    }
    let text = std::fs::read_to_string(&paths.credentials_path)?;
    let value: Value = serde_json::from_str(&text)?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| AppError::Json(serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::InvalidData, "credentials 根节点不是对象"))))
}

pub fn read_decrypted_credentials(paths: &ZCodePaths) -> AppResult<Map<String, Value>> {
    let raw = read_raw_credentials(paths)?;
    let cipher = ZCodeCipher::new()?;
    let mut out = Map::new();
    for (key, value) in raw {
        if let Some(s) = value.as_str() {
            let plain = cipher.decrypt(s)?;
            if let Ok(json_value) = serde_json::from_str::<Value>(&plain) {
                out.insert(key, json_value);
            } else {
                out.insert(key, Value::String(plain));
            }
        } else {
            out.insert(key, value);
        }
    }
    Ok(out)
}

pub fn write_credentials_from_account(
    paths: &ZCodePaths,
    zai_access_token: &str,
    zcode_jwt_token: &str,
    user_info: &Value,
) -> AppResult<()> {
    let cipher = ZCodeCipher::new()?;
    let mut plain = if paths.credentials_path.exists() {
        read_decrypted_credentials(paths)?
    } else {
        Map::new()
    };
    plain.insert("oauth:active_provider".to_string(), Value::String("zai".to_string()));
    plain.insert(
        "oauth:zai:access_token".to_string(),
        Value::String(zai_access_token.to_string()),
    );
    plain.insert(
        "zcodejwttoken".to_string(),
        Value::String(zcode_jwt_token.to_string()),
    );
    plain.insert("oauth:zai:user_info".to_string(), user_info.clone());

    let mut raw = Map::new();
    for (key, value) in plain {
        let plaintext = match value {
            Value::String(s) => s,
            other => serde_json::to_string(&other)?,
        };
        raw.insert(key, Value::String(cipher.encrypt(&plaintext)?));
    }
    if let Some(parent) = paths.credentials_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        &paths.credentials_path,
        serde_json::to_string_pretty(&Value::Object(raw))?,
    )?;
    Ok(())
}

pub fn import_current_account(alias: Option<String>, note: Option<String>) -> AppResult<AccountUpsert> {
    let paths = zcode_paths()?;
    let plain = read_decrypted_credentials(&paths)?;
    let access = plain
        .get("oauth:zai:access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Message("当前 credentials.json 中没有 oauth:zai:access_token".to_string()))?
        .to_string();
    let jwt = plain
        .get("zcodejwttoken")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Message("当前 credentials.json 中没有 zcodejwttoken".to_string()))?
        .to_string();
    let user_info = plain
        .get("oauth:zai:user_info")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let email = user_info.get("email").and_then(Value::as_str).map(str::to_string);
    let user_id = user_info.get("user_id").and_then(Value::as_str).map(str::to_string);
    let display_name = user_info.get("name").and_then(Value::as_str).map(str::to_string);
    let providers = read_providers(&paths).unwrap_or_else(|_| Value::Object(Map::new()));
    let cache = read_coding_plan_cache(&paths).unwrap_or_else(|_| Value::Object(Map::new()));
    let hot_switch_ready = providers.as_object().map(|m| !m.is_empty()).unwrap_or(false);
    let cipher = ZCodeCipher::new()?;
    Ok(AccountUpsert {
        alias: alias
            .or_else(|| email.clone())
            .unwrap_or_else(|| "current".to_string()),
        email,
        user_id,
        display_name,
        active_provider: plain
            .get("oauth:active_provider")
            .and_then(Value::as_str)
            .unwrap_or("zai")
            .to_string(),
        zai_access_token_enc: cipher.encrypt(&access)?,
        zcode_jwt_token_enc: cipher.encrypt(&jwt)?,
        user_info_json: user_info,
        config_providers_json: providers,
        coding_plan_cache_json: cache,
        hot_switch_ready,
        source: "imported_current".to_string(),
        note,
        last_auth_at: Some(chrono::Utc::now().timestamp()),
    })
}
