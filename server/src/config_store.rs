use crate::{
    error::{AppError, AppResult},
    state::AppState,
};
use axum::{Json, Router, extract::State, routing::get};
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

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v2/config", get(get_config).put(put_config))
}

#[utoipa::path(get, path = "/api/v2/config", tag = "config", responses((status = 200, body = ConfigResponse)))]
pub async fn get_config(State(state): State<AppState>) -> AppResult<Json<ConfigResponse>> {
    let rows: Vec<(String, Value)> =
        sqlx::query_as("SELECT key, value FROM app_settings ORDER BY key")
            .fetch_all(&state.pool)
            .await?;
    let mut settings = BTreeMap::new();
    for (key, value) in rows {
        settings.insert(key.clone(), mask_secret(&key, value));
    }
    Ok(Json(ConfigResponse { settings }))
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

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("password")
        || key.contains("cookie")
        || key.contains("secret")
        || key.contains("api_key")
}

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
        assert!(!is_sensitive_key("c115_cid_map"));
    }
}
