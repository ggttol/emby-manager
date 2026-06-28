use crate::{error::AppResult, state::AppState};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;

const DEFAULT_LIMIT: i64 = 200;
const MAX_LIMIT: i64 = 500;

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct LogListQuery {
    pub limit: Option<i64>,
    pub level: Option<String>,
}

impl LogListQuery {
    pub fn limit(&self) -> i64 {
        self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
    }

    pub fn normalized_level(&self) -> Option<String> {
        self.level
            .as_deref()
            .map(str::trim)
            .filter(|level| !level.is_empty())
            .map(str::to_ascii_lowercase)
    }
}

#[derive(Debug, Serialize, FromRow, utoipa::ToSchema)]
pub struct AppLogEntry {
    pub id: i64,
    pub level: String,
    pub message: String,
    pub detail: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LogListResponse {
    pub logs: Vec<AppLogEntry>,
    pub total: usize,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v2/logs", get(list_logs))
}

#[utoipa::path(get, path = "/api/v2/logs", tag = "logs", params(LogListQuery), responses((status = 200, body = LogListResponse)))]
pub async fn list_logs(
    State(state): State<AppState>,
    Query(query): Query<LogListQuery>,
) -> AppResult<Json<LogListResponse>> {
    let limit = query.limit();
    let level = query.normalized_level();
    let logs = if let Some(level) = level {
        sqlx::query_as::<_, AppLogEntry>(
            "SELECT id, level, message, detail, created_at
             FROM app_logs
             WHERE lower(level) = $1
             ORDER BY created_at DESC, id DESC
             LIMIT $2",
        )
        .bind(level)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as::<_, AppLogEntry>(
            "SELECT id, level, message, detail, created_at
             FROM app_logs
             ORDER BY created_at DESC, id DESC
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&state.pool)
        .await?
    };
    let total = logs.len();
    Ok(Json(LogListResponse { logs, total }))
}
