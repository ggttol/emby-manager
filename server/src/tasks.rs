use crate::{
    error::{AppError, AppResult},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

pub const ACTIVE_STATUSES: &[&str] = &["pending", "running"];

#[derive(Debug, Serialize, FromRow, utoipa::ToSchema)]
pub struct TaskRun {
    pub id: Uuid,
    pub kind: String,
    pub label: String,
    pub source: String,
    pub params: Value,
    pub status: String,
    pub progress: i64,
    pub total: i64,
    pub status_text: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub cancel_requested: bool,
    pub queued_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TaskListResponse {
    pub tasks: Vec<TaskRun>,
    pub active_count: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TaskCancelResponse {
    pub ok: bool,
}

#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct DemoTaskRequest {
    pub seconds: Option<u64>,
    pub label: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/tasks", get(list_tasks))
        .route("/api/v2/tasks/demo", post(demo_task))
        .route("/api/v2/tasks/{id}", get(get_task))
        .route("/api/v2/tasks/{id}/cancel", post(cancel_task))
}

pub async fn reconcile_interrupted(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE task_runs
         SET status = 'interrupted', status_text = '服务重启后中断', ended_at = now(), updated_at = now()
         WHERE status IN ('pending', 'running')",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_task(
    pool: &sqlx::PgPool,
    kind: &str,
    label: &str,
    total: i64,
) -> AppResult<TaskRun> {
    insert_task_with_meta(pool, kind, label, total, "manual", serde_json::json!({})).await
}

pub async fn insert_task_with_meta(
    pool: &sqlx::PgPool,
    kind: &str,
    label: &str,
    total: i64,
    source: &str,
    params: Value,
) -> AppResult<TaskRun> {
    let id = Uuid::new_v4();
    let task = sqlx::query_as::<_, TaskRun>(
        "INSERT INTO task_runs(id, kind, label, source, params, status, total)
         VALUES ($1, $2, $3, $4, $5, 'pending', $6)
         RETURNING *",
    )
    .bind(id)
    .bind(kind)
    .bind(label)
    .bind(source)
    .bind(params)
    .bind(total)
    .fetch_one(pool)
    .await?;
    Ok(task)
}

pub async fn set_total(pool: &sqlx::PgPool, id: Uuid, total: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE task_runs SET total = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(total.max(0))
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_running(
    pool: &sqlx::PgPool,
    id: Uuid,
    status_text: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE task_runs
         SET status = 'running', started_at = COALESCE(started_at, now()), status_text = $2, updated_at = now()
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(id)
    .bind(status_text)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_progress(
    pool: &sqlx::PgPool,
    id: Uuid,
    progress: i64,
    status_text: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE task_runs
         SET progress = $2, status_text = $3, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(progress.max(0))
    .bind(status_text)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn finish_done(pool: &sqlx::PgPool, id: Uuid, result: Value) -> Result<(), sqlx::Error> {
    finish_done_with_message(pool, id, "完成", result).await
}

pub async fn finish_done_with_message(
    pool: &sqlx::PgPool,
    id: Uuid,
    status_text: &str,
    result: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE task_runs
         SET status = 'done', progress = total, status_text = $2, result = $3, ended_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(status_text)
    .bind(result)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn finish_error(
    pool: &sqlx::PgPool,
    id: Uuid,
    error: &str,
    result: Option<Value>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE task_runs
         SET status = 'error', status_text = '失败', error = $2, result = $3, ended_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(error)
    .bind(result)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn finish_cancelled(pool: &sqlx::PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE task_runs
         SET status = 'cancelled', status_text = '已取消', ended_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn cancel_requested(pool: &sqlx::PgPool, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT cancel_requested FROM task_runs WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}

#[utoipa::path(get, path = "/api/v2/tasks", tag = "tasks", responses((status = 200, body = TaskListResponse)))]
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(query): Query<TaskListQuery>,
) -> AppResult<Json<TaskListResponse>> {
    let limit = query.limit.unwrap_or(20).clamp(1, 200);
    let tasks =
        sqlx::query_as::<_, TaskRun>("SELECT * FROM task_runs ORDER BY updated_at DESC LIMIT $1")
            .bind(limit)
            .fetch_all(&state.pool)
            .await?;
    let active_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM task_runs WHERE status = ANY($1)")
            .bind(ACTIVE_STATUSES)
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(TaskListResponse {
        tasks,
        active_count,
    }))
}

#[utoipa::path(get, path = "/api/v2/tasks/{id}", tag = "tasks", params(("id" = Uuid, Path)), responses((status = 200, body = TaskRun)))]
pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<TaskRun>> {
    let task = sqlx::query_as::<_, TaskRun>("SELECT * FROM task_runs WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("未知任务".to_string()))?;
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/tasks/{id}/cancel", tag = "tasks", params(("id" = Uuid, Path)), responses((status = 200, body = TaskCancelResponse)))]
pub async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<TaskCancelResponse>> {
    let updated = sqlx::query(
        "UPDATE task_runs SET cancel_requested = TRUE, status_text = '取消中...', updated_at = now()
         WHERE id = $1 AND status IN ('pending', 'running')",
    )
    .bind(id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    Ok(Json(TaskCancelResponse { ok: updated > 0 }))
}

#[utoipa::path(post, path = "/api/v2/tasks/demo", tag = "tasks", request_body = DemoTaskRequest, responses((status = 200, body = TaskRun)))]
pub async fn demo_task(
    State(state): State<AppState>,
    Json(req): Json<DemoTaskRequest>,
) -> AppResult<Json<TaskRun>> {
    let seconds = req.seconds.unwrap_or(5).clamp(1, 120);
    let label = req.label.unwrap_or_else(|| "演示任务".to_string());
    let task = insert_task(&state.pool, "demo", &label, seconds as i64).await?;
    spawn_demo(state, task.id, seconds);
    Ok(Json(task))
}

fn spawn_demo(state: AppState, id: Uuid, seconds: u64) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            return;
        };
        let _ = sqlx::query("UPDATE task_runs SET status = 'running', started_at = now(), status_text = '', updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&state.pool)
            .await;
        for i in 0..seconds {
            sleep(Duration::from_secs(1)).await;
            let cancel_requested: bool =
                sqlx::query_scalar("SELECT cancel_requested FROM task_runs WHERE id = $1")
                    .bind(id)
                    .fetch_one(&state.pool)
                    .await
                    .unwrap_or(false);
            if cancel_requested {
                let _ = sqlx::query("UPDATE task_runs SET status = 'cancelled', status_text = '已取消', ended_at = now(), updated_at = now() WHERE id = $1")
                    .bind(id)
                    .execute(&state.pool)
                    .await;
                return;
            }
            let _ = sqlx::query("UPDATE task_runs SET progress = $2, status_text = $3, updated_at = now() WHERE id = $1")
                .bind(id)
                .bind((i + 1) as i64)
                .bind(format!("处理中 {}/{}", i + 1, seconds))
                .execute(&state.pool)
                .await;
        }
        let _ = sqlx::query("UPDATE task_runs SET status = 'done', status_text = '完成', result = $2, ended_at = now(), updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(serde_json::json!({"ok": true}))
            .execute(&state.pool)
            .await;
    });
}
