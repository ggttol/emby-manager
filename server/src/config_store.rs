use crate::{
    error::{AppError, AppResult},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::PgPool;
use std::collections::BTreeMap;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConfigResponse {
    pub settings: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ConfigUpdateRequest {
    pub settings: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ConfigImportRequest {
    pub settings: Option<BTreeMap<String, Value>>,
    pub cfg: Option<BTreeMap<String, Value>>,
    pub mode: Option<String>,
    pub dry_run: Option<bool>,
    pub apply: Option<bool>,
    pub confirm: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConfigImportRejected {
    pub key: String,
    pub reason: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConfigImportReport {
    pub accepted: Vec<String>,
    pub rejected: Vec<ConfigImportRejected>,
    pub warnings: Vec<String>,
    pub applied: Vec<String>,
    pub dry_run: bool,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/config", get(get_config).put(put_config))
        .route("/api/v2/config/export", get(export_config))
        .route("/api/v2/config/import", post(import_config))
}

#[utoipa::path(get, path = "/api/v2/config", tag = "config", responses((status = 200, body = ConfigResponse)))]
pub async fn get_config(State(state): State<AppState>) -> AppResult<Json<ConfigResponse>> {
    Ok(Json(ConfigResponse {
        settings: load_masked_settings(&state.pool).await?,
    }))
}

#[utoipa::path(get, path = "/api/v2/config/export", tag = "config", responses((status = 200, body = ConfigResponse)))]
pub async fn export_config(State(state): State<AppState>) -> AppResult<Json<ConfigResponse>> {
    Ok(Json(ConfigResponse {
        settings: load_masked_settings(&state.pool).await?,
    }))
}

async fn load_masked_settings(pool: &PgPool) -> AppResult<BTreeMap<String, Value>> {
    let rows: Vec<(String, Value)> =
        sqlx::query_as("SELECT key, value FROM app_settings ORDER BY key")
            .fetch_all(pool)
            .await?;
    let mut settings = BTreeMap::new();
    for (key, value) in rows {
        settings.insert(key.clone(), mask_secret(&key, value));
    }
    Ok(settings)
}

#[utoipa::path(put, path = "/api/v2/config", tag = "config", request_body = ConfigUpdateRequest, responses((status = 200, body = ConfigResponse)))]
pub async fn put_config(
    State(state): State<AppState>,
    Json(req): Json<ConfigUpdateRequest>,
) -> AppResult<Json<ConfigResponse>> {
    for (key, value) in req.settings {
        let Some(value) = normalize_update_value(&state.pool, &key, value).await? else {
            continue;
        };
        sqlx::query(
            "INSERT INTO app_settings(key, value, updated_at) VALUES ($1, $2, now())
             ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(&state.pool)
        .await?;
    }
    get_config(State(state)).await
}

#[utoipa::path(post, path = "/api/v2/config/import", tag = "config", request_body = ConfigImportRequest, responses((status = 200, body = ConfigImportReport)))]
pub async fn import_config(
    State(state): State<AppState>,
    Json(req): Json<ConfigImportRequest>,
) -> AppResult<Json<ConfigImportReport>> {
    let apply = import_should_apply(&req)?;
    let settings = import_payload_settings(req)?;
    let mut accepted_entries = Vec::new();
    let mut rejected = Vec::new();
    let mut warnings = Vec::new();

    for (key, value) in settings {
        if is_protected_import_key(&key) {
            rejected.push(ConfigImportRejected {
                key,
                reason: "protected".to_string(),
            });
            continue;
        }
        if !is_importable_config_key(&key) {
            rejected.push(ConfigImportRejected {
                key,
                reason: "unknown".to_string(),
            });
            continue;
        }

        match normalize_import_value(&state.pool, &key, value).await? {
            ImportValue::Accepted { value, warning } => {
                if let Some(warning) = warning {
                    warnings.push(warning);
                }
                accepted_entries.push(AcceptedImport { key, value });
            }
            ImportValue::Rejected(reason) => rejected.push(ConfigImportRejected { key, reason }),
        }
    }

    let accepted = accepted_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<Vec<_>>();
    let mut applied = Vec::new();
    if apply {
        for entry in &accepted_entries {
            let Some(value) = entry.value.clone() else {
                continue;
            };
            sqlx::query(
                "INSERT INTO app_settings(key, value, updated_at) VALUES ($1, $2, now())
                 ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
            )
            .bind(&entry.key)
            .bind(value)
            .execute(&state.pool)
            .await?;
            applied.push(entry.key.clone());
        }
    }

    Ok(Json(ConfigImportReport {
        accepted,
        rejected,
        warnings,
        applied,
        dry_run: !apply,
    }))
}

pub fn mask_secret(key: &str, value: Value) -> Value {
    if is_sensitive_key(key) {
        match value {
            Value::String(s) if !s.is_empty() => Value::String("***".to_string()),
            v => v,
        }
    } else if let Value::Object(obj) = value {
        Value::Object(mask_object(obj))
    } else {
        value
    }
}

fn mask_object(obj: Map<String, Value>) -> Map<String, Value> {
    obj.into_iter()
        .map(|(k, v)| {
            let masked = if is_sensitive_key(&k) {
                match v {
                    Value::String(s) if !s.is_empty() => Value::String("***".to_string()),
                    Value::Object(obj) => Value::Object(mask_object(obj)),
                    other => other,
                }
            } else if let Value::Object(obj) = v {
                Value::Object(mask_object(obj))
            } else {
                v
            };
            (k, masked)
        })
        .collect()
}

pub async fn get_raw(pool: &PgPool, key: &str) -> Result<Option<Value>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM app_settings WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
}

pub async fn get_string(pool: &PgPool, key: &str) -> Result<Option<String>, sqlx::Error> {
    Ok(get_raw(pool, key)
        .await?
        .and_then(|v| v.as_str().map(ToString::to_string)))
}

pub async fn get_string_or(pool: &PgPool, key: &str, default: &str) -> Result<String, sqlx::Error> {
    Ok(get_string(pool, key)
        .await?
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string()))
}

async fn normalize_update_value(
    pool: &PgPool,
    key: &str,
    value: Value,
) -> AppResult<Option<Value>> {
    if is_sensitive_key(key) && matches!(value.as_str(), Some("***")) {
        let existing = get_raw(pool, key).await?;
        return if existing.is_some() {
            Ok(None)
        } else {
            Err(AppError::BadRequest(format!(
                "敏感字段 {key} 不能保存脱敏占位符"
            )))
        };
    }
    Ok(Some(value))
}

struct AcceptedImport {
    key: String,
    value: Option<Value>,
}

enum ImportValue {
    Accepted {
        value: Option<Value>,
        warning: Option<String>,
    },
    Rejected(String),
}

fn import_payload_settings(req: ConfigImportRequest) -> AppResult<BTreeMap<String, Value>> {
    match (req.settings, req.cfg) {
        (Some(_), Some(_)) => Err(AppError::BadRequest(
            "settings 和 cfg 不能同时提供".to_string(),
        )),
        (Some(settings), None) | (None, Some(settings)) => Ok(settings),
        (None, None) => Err(AppError::BadRequest(
            "必须提供 settings 或 cfg 对象".to_string(),
        )),
    }
}

fn import_should_apply(req: &ConfigImportRequest) -> AppResult<bool> {
    let mut selected = None;
    if let Some(mode) = req.mode.as_deref() {
        match mode.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "apply" => merge_apply_choice(&mut selected, true)?,
            "dry_run" | "dryrun" => merge_apply_choice(&mut selected, false)?,
            other => {
                return Err(AppError::BadRequest(format!("不支持的导入模式: {other}")));
            }
        }
    }
    if let Some(dry_run) = req.dry_run {
        merge_apply_choice(&mut selected, !dry_run)?;
    }
    if let Some(apply) = req.apply {
        merge_apply_choice(&mut selected, apply)?;
    }
    if let Some(confirm) = req.confirm {
        merge_apply_choice(&mut selected, confirm)?;
    }
    Ok(selected.unwrap_or(false))
}

fn merge_apply_choice(selected: &mut Option<bool>, choice: bool) -> AppResult<()> {
    match *selected {
        Some(existing) if existing != choice => Err(AppError::BadRequest(
            "导入模式冲突: dry_run/apply/confirm 不一致".to_string(),
        )),
        Some(_) => Ok(()),
        None => {
            *selected = Some(choice);
            Ok(())
        }
    }
}

async fn normalize_import_value(pool: &PgPool, key: &str, value: Value) -> AppResult<ImportValue> {
    if is_sensitive_key(key) && matches!(value.as_str(), Some("***")) {
        let existing = get_raw(pool, key).await?;
        let warning = if existing.is_some() {
            format!("{key} 为脱敏占位符，已保留现有值")
        } else {
            format!("{key} 为脱敏占位符，但当前没有可保留的现有值")
        };
        return Ok(ImportValue::Accepted {
            value: None,
            warning: Some(warning),
        });
    }

    let normalized = match key {
        "emby_url" | "tmdb_base_url" | "tmdb_url" | "tg_resource_api_base_url" => {
            normalize_url(key, value)
        }
        "api_key"
        | "c115_cookie"
        | "cd2_webhook_secret"
        | "tmdb_api_key"
        | "tmdb_key"
        | "tg_resource_api_token" => normalize_string(key, value),
        "c115_cid_map" => normalize_cid_map(value),
        "trusted_proxies" => normalize_string_list(key, value),
        "auto_strm_enabled" | "auto_strm_fullauto" | "bind_token_ip" => normalize_bool(key, value),
        "auto_strm_debounce_sec" => normalize_int_range(key, value, 1, 120),
        "tmdb_timeout_secs" => normalize_int_range(key, value, 1, 60),
        "cd2_mount_prefix" => normalize_absolute_prefix(key, value),
        _ => Err(format!("不支持导入配置字段: {key}")),
    };

    Ok(match normalized {
        Ok(value) => ImportValue::Accepted {
            value: Some(value),
            warning: None,
        },
        Err(reason) => ImportValue::Rejected(reason),
    })
}

fn normalize_url(key: &str, value: Value) -> Result<Value, String> {
    let value = string_from_value(key, value)?
        .trim()
        .trim_end_matches('/')
        .to_string();
    if value.starts_with("http://") || value.starts_with("https://") {
        Ok(Value::String(value))
    } else {
        Err(format!("{key} 必须以 http:// 或 https:// 开头"))
    }
}

fn normalize_string(key: &str, value: Value) -> Result<Value, String> {
    Ok(Value::String(
        string_from_value(key, value)?.trim().to_string(),
    ))
}

fn normalize_cid_map(value: Value) -> Result<Value, String> {
    let Value::Object(obj) = value else {
        return Err("c115_cid_map 必须是对象".to_string());
    };
    let mut out = Map::new();
    for (lib, cid_value) in obj {
        let lib = lib.trim().to_string();
        if lib.is_empty() {
            continue;
        }
        let cid = match cid_value {
            Value::String(value) => value.trim().to_string(),
            Value::Number(value) => value.to_string(),
            _ => return Err(format!("c115_cid_map.{lib} 必须是正整数 cid")),
        };
        if cid.is_empty() {
            continue;
        }
        if !is_positive_integer(&cid) {
            return Err(format!("c115_cid_map.{lib} 必须是正整数 cid"));
        }
        out.insert(lib, Value::String(cid));
    }
    Ok(Value::Object(out))
}

fn normalize_string_list(key: &str, value: Value) -> Result<Value, String> {
    let values = match value {
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                let Some(item) = item.as_str() else {
                    return Err(format!("{key} 必须是字符串数组"));
                };
                let item = item.trim();
                if !item.is_empty() {
                    out.push(Value::String(item.to_string()));
                }
            }
            out
        }
        Value::String(raw) => raw
            .split(|ch: char| ch == ',' || ch == '，' || ch.is_whitespace())
            .filter_map(|item| {
                let item = item.trim();
                (!item.is_empty()).then(|| Value::String(item.to_string()))
            })
            .collect(),
        _ => return Err(format!("{key} 必须是字符串数组")),
    };
    Ok(Value::Array(values))
}

fn normalize_bool(key: &str, value: Value) -> Result<Value, String> {
    value
        .as_bool()
        .map(Value::Bool)
        .ok_or_else(|| format!("{key} 必须是 true/false"))
}

fn normalize_int_range(key: &str, value: Value, min: i64, max: i64) -> Result<Value, String> {
    let parsed = match value {
        Value::Number(value) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok())),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    };
    let Some(parsed) = parsed else {
        return Err(format!("{key} 必须是 {min}-{max} 的整数"));
    };
    if !(min..=max).contains(&parsed) {
        return Err(format!("{key} 必须是 {min}-{max} 的整数"));
    }
    Ok(Value::Number(serde_json::Number::from(parsed)))
}

fn normalize_absolute_prefix(key: &str, value: Value) -> Result<Value, String> {
    let value = string_from_value(key, value)?.trim().to_string();
    if !value.starts_with('/') {
        return Err(format!("{key} 必须是绝对路径"));
    }
    let value = value.trim_end_matches('/');
    Ok(Value::String(if value.is_empty() {
        "/".to_string()
    } else {
        value.to_string()
    }))
}

fn string_from_value(key: &str, value: Value) -> Result<String, String> {
    value
        .as_str()
        .map(ToString::to_string)
        .ok_or_else(|| format!("{key} 必须是字符串"))
}

fn is_positive_integer(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('1'..='9')) && chars.all(|ch| ch.is_ascii_digit())
}

fn is_importable_config_key(key: &str) -> bool {
    IMPORTABLE_CONFIG_KEYS
        .iter()
        .any(|candidate| *candidate == key)
}

fn is_protected_import_key(key: &str) -> bool {
    PROTECTED_IMPORT_KEYS
        .iter()
        .any(|candidate| *candidate == key)
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("password")
        || key.contains("cookie")
        || key.contains("secret")
        || key.contains("api_key")
        || key.contains("token")
}

const IMPORTABLE_CONFIG_KEYS: &[&str] = &[
    "emby_url",
    "api_key",
    "c115_cookie",
    "c115_cid_map",
    "trusted_proxies",
    "auto_strm_enabled",
    "auto_strm_fullauto",
    "auto_strm_debounce_sec",
    "cd2_mount_prefix",
    "cd2_webhook_secret",
    "tmdb_base_url",
    "tmdb_url",
    "tmdb_api_key",
    "tmdb_key",
    "tmdb_timeout_secs",
    "tg_resource_api_base_url",
    "tg_resource_api_token",
    "bind_token_ip",
];

const PROTECTED_IMPORT_KEYS: &[&str] = &[
    "schema_version",
    "password_hash",
    "last_password_change_at",
    "username",
    "host",
    "port",
    "database_url",
    "web_dist",
    "legacy_dir",
    "bootstrap_password",
    "cd_root",
    "strm_root",
    "docker_bin",
    "task_concurrency",
    "cd",
    "strm",
    "docker",
    "schedules",
    "schedule_jobs",
    "auth_users",
    "sessions",
    "openapi",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_nested_secret_keys() {
        let value = serde_json::json!({
            "plain": "ok",
            "nested": {
                "api_key": "secret",
                "password": "pw",
                "cookie": "uid=1"
            }
        });
        let masked = mask_secret("settings", value);
        assert_eq!(masked["plain"], "ok");
        assert_eq!(masked["nested"]["api_key"], "***");
        assert_eq!(masked["nested"]["password"], "***");
        assert_eq!(masked["nested"]["cookie"], "***");
    }

    #[test]
    fn detects_sensitive_keys() {
        assert!(is_sensitive_key("api_key"));
        assert!(is_sensitive_key("c115_cookie"));
        assert!(is_sensitive_key("bootstrap_password"));
        assert!(is_sensitive_key("tg_resource_api_token"));
        assert!(!is_sensitive_key("c115_cid_map"));
    }
}
