use crate::{
    error::{AppError, AppResult},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct UndoListQuery {
    pub limit: Option<i64>,
}

impl UndoListQuery {
    pub fn limit(&self) -> i64 {
        self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
    }
}

#[derive(Debug, Serialize, FromRow, utoipa::ToSchema)]
pub struct UndoEntry {
    pub id: Uuid,
    pub legacy_id: Option<String>,
    pub op: String,
    pub payload: Value,
    pub undone: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UndoListResponse {
    pub items: Vec<UndoEntry>,
    pub total: usize,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UndoExecuteRequest {
    pub id: Uuid,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UndoExecuteAction {
    ManualRestore,
    PendingPort,
    AlreadyUndone,
    Unsupported,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UndoExecuteResponse {
    pub ok: bool,
    pub id: Uuid,
    pub op: String,
    pub action: UndoExecuteAction,
    pub msg: String,
    pub lib: Option<String>,
    pub folder: Option<String>,
    pub hint: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/manage/undo", get(list_undo))
        .route("/api/v2/manage/undo/execute", post(exec_undo))
}

#[utoipa::path(get, path = "/api/v2/manage/undo", tag = "undo", params(UndoListQuery), responses((status = 200, body = UndoListResponse)))]
pub async fn list_undo(
    State(state): State<AppState>,
    Query(query): Query<UndoListQuery>,
) -> AppResult<Json<UndoListResponse>> {
    let limit = query.limit();
    let items = sqlx::query_as::<_, UndoEntry>(
        "SELECT id, legacy_id, op, payload, undone, created_at
         FROM undo_entries
         ORDER BY created_at DESC
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    let total = items.len();
    Ok(Json(UndoListResponse { items, total }))
}

#[utoipa::path(post, path = "/api/v2/manage/undo/execute", tag = "undo", request_body = UndoExecuteRequest, responses((status = 200, body = UndoExecuteResponse)))]
pub async fn exec_undo(
    State(state): State<AppState>,
    Json(req): Json<UndoExecuteRequest>,
) -> AppResult<Json<UndoExecuteResponse>> {
    let entry = sqlx::query_as::<_, UndoEntry>(
        "SELECT id, legacy_id, op, payload, undone, created_at
         FROM undo_entries
         WHERE id = $1",
    )
    .bind(req.id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("未知 undo id".to_string()))?;
    Ok(Json(build_execute_response(&entry)))
}

pub fn build_execute_response(entry: &UndoEntry) -> UndoExecuteResponse {
    if entry.undone {
        return UndoExecuteResponse {
            ok: false,
            id: entry.id,
            op: entry.op.clone(),
            action: UndoExecuteAction::AlreadyUndone,
            msg: "这条操作已经撤销过了,不会重复执行".to_string(),
            lib: payload_string(&entry.payload, &["lib", "from", "to"]),
            folder: payload_string(&entry.payload, &["folder", "lose_was", "lose_folder"]),
            hint: None,
        };
    }

    match entry.op.as_str() {
        "delete" | "smart_archive" | "replace" => manual_restore_response(entry),
        "move" => pending_response(
            entry,
            "move undo requires Rust manage/move write path; not executed yet",
        ),
        "rebind" => pending_response(
            entry,
            "poster rebind undo requires Rust poster apply port; not executed yet",
        ),
        _ => UndoExecuteResponse {
            ok: false,
            id: entry.id,
            op: entry.op.clone(),
            action: UndoExecuteAction::Unsupported,
            msg: format!("不支持撤销此操作: {}", entry.op),
            lib: payload_string(&entry.payload, &["lib", "from", "to"]),
            folder: payload_string(&entry.payload, &["folder"]),
            hint: None,
        },
    }
}

fn manual_restore_response(entry: &UndoEntry) -> UndoExecuteResponse {
    let folder = payload_string(&entry.payload, &["folder", "lose_was", "lose_folder"]);
    let lib = payload_string(&entry.payload, &["lib", "from", "to"]);
    let label = match entry.op.as_str() {
        "delete" => "删除",
        "smart_archive" => "智能归档删源",
        "replace" => "全替换删旧版",
        _ => entry.op.as_str(),
    };
    let folder_text = folder.as_deref().unwrap_or("");
    let hint = format!("115 web -> 回收站 -> 找「{folder_text}」-> 还原 -> 回到工具扫描对应库");
    UndoExecuteResponse {
        ok: false,
        id: entry.id,
        op: entry.op.clone(),
        action: UndoExecuteAction::ManualRestore,
        msg: format!("「{label}」已把 115 文件夹送入回收站,请先去 115 web 还原它,再用扫描补 strm"),
        lib,
        folder,
        hint: Some(hint),
    }
}

fn pending_response(entry: &UndoEntry, msg: &str) -> UndoExecuteResponse {
    UndoExecuteResponse {
        ok: false,
        id: entry.id,
        op: entry.op.clone(),
        action: UndoExecuteAction::PendingPort,
        msg: msg.to_string(),
        lib: payload_string(&entry.payload, &["lib", "from", "to"]),
        folder: payload_string(&entry.payload, &["folder"]),
        hint: Some("Rust 预览版未执行任何写操作,旧版 Python 仍可作为回滚路径".to_string()),
    }
}

fn payload_string(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        payload
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}
