use serde_json::{Map, Value};

use crate::db::write_json;
use crate::error::AppResult;
use crate::zcode::paths::ZCodePaths;

pub fn read_config(paths: &ZCodePaths) -> AppResult<Value> {
    if !paths.config_path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(&paths.config_path)?)?)
}

pub fn read_providers(paths: &ZCodePaths) -> AppResult<Value> {
    let config = read_config(paths)?;
    Ok(config.get("provider").cloned().unwrap_or_else(|| Value::Object(Map::new())))
}

pub fn write_providers(paths: &ZCodePaths, providers: &Value) -> AppResult<()> {
    let mut config = read_config(paths)?;
    if !config.is_object() {
        config = Value::Object(Map::new());
    }
    if let Some(obj) = config.as_object_mut() {
        obj.insert("provider".to_string(), providers.clone());
    }
    write_json(&paths.config_path, &config)
}

pub fn read_coding_plan_cache(paths: &ZCodePaths) -> AppResult<Value> {
    if !paths.cache_path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(&paths.cache_path)?)?)
}

/// 用模板账号的 provider 快照为目标账号合成一份 config。
///
/// 依据(实测铁律,见 Python cmd_patch_config): `builtin:zai-start-plan` 的
/// `options.apiKey` 就等于该账号的 zcodejwttoken,两者完全相等。因此把模板
/// providers 里所有 `options.apiKey == 模板 jwt` 的项替换成目标 jwt,即得到
/// 目标账号可用的 config。其余 apiKey(如 zai/zai-coding-plan 的 49 字兑换 key)
/// 非 jwt 派生、且不是当前启用 provider,保持原样不影响热切换。
///
/// 返回 (合成后的 providers, 替换的 apiKey 个数)。
pub fn synthesize_providers(template_providers: &Value, template_jwt: &str, target_jwt: &str) -> (Value, usize) {
    let mut providers = template_providers.clone();
    let mut replaced = 0usize;
    if let Some(obj) = providers.as_object_mut() {
        for (_id, provider) in obj.iter_mut() {
            if let Some(opts) = provider.get_mut("options").and_then(Value::as_object_mut) {
                if opts.get("apiKey").and_then(Value::as_str) == Some(template_jwt) {
                    opts.insert("apiKey".to_string(), Value::String(target_jwt.to_string()));
                    replaced += 1;
                }
            }
        }
    }
    (providers, replaced)
}

pub fn write_coding_plan_cache(paths: &ZCodePaths, cache: &Value) -> AppResult<()> {
    write_json(&paths.cache_path, cache)
}
