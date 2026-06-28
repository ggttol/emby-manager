use crate::{
    error::{AppError, AppResult},
    state::AppState,
    tasks,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post, put},
};
use chrono::{DateTime, Datelike, Duration, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow, utoipa::ToSchema)]
pub struct ScheduleJob {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub params: Value,
    pub schedule: Value,
    pub enabled: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_ended_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub last_task_id: Option<Uuid>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ScheduleRequest {
    pub name: String,
    pub kind: String,
    pub params: Option<Value>,
    pub schedule: Value,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RunScheduleResponse {
    pub tid: Uuid,
    pub task: tasks::TaskRun,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeleteScheduleResponse {
    pub ok: bool,
}

pub const SUPPORTED_SCHEDULE_KINDS: &[&str] = &[
    "scan_all",
    "zhuigeng_scan_airing",
    "fix_posters_all",
    "refresh_no_rating_all",
    "monitor_incremental",
];

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v2/schedules",
            get(list_schedules).post(create_schedule),
        )
        .route(
            "/api/v2/schedules/{id}",
            put(update_schedule).delete(delete_schedule),
        )
        .route("/api/v2/schedules/{id}/run", post(run_schedule))
}

#[utoipa::path(get, path = "/api/v2/schedules", tag = "schedules", responses((status = 200, body = [ScheduleJob])))]
pub async fn list_schedules(State(state): State<AppState>) -> AppResult<Json<Vec<ScheduleJob>>> {
    let rows =
        sqlx::query_as::<_, ScheduleJob>("SELECT * FROM schedule_jobs ORDER BY created_at DESC")
            .fetch_all(&state.pool)
            .await?;
    Ok(Json(rows))
}

#[utoipa::path(post, path = "/api/v2/schedules", tag = "schedules", request_body = ScheduleRequest, responses((status = 200, body = ScheduleJob)))]
pub async fn create_schedule(
    State(state): State<AppState>,
    Json(req): Json<ScheduleRequest>,
) -> AppResult<Json<ScheduleJob>> {
    validate_kind(&req.kind)?;
    validate_schedule(&req.schedule)?;
    let row = sqlx::query_as::<_, ScheduleJob>(
        "INSERT INTO schedule_jobs(id, name, kind, params, schedule, enabled)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(req.name)
    .bind(req.kind)
    .bind(req.params.unwrap_or_else(|| serde_json::json!({})))
    .bind(req.schedule)
    .bind(req.enabled.unwrap_or(true))
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(row))
}

#[utoipa::path(put, path = "/api/v2/schedules/{id}", tag = "schedules", params(("id" = Uuid, Path)), request_body = ScheduleRequest, responses((status = 200, body = ScheduleJob)))]
pub async fn update_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<ScheduleRequest>,
) -> AppResult<Json<ScheduleJob>> {
    validate_kind(&req.kind)?;
    validate_schedule(&req.schedule)?;
    let row = sqlx::query_as::<_, ScheduleJob>(
        "UPDATE schedule_jobs
         SET name = $2, kind = $3, params = $4, schedule = $5, enabled = $6, updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(req.name)
    .bind(req.kind)
    .bind(req.params.unwrap_or_else(|| serde_json::json!({})))
    .bind(req.schedule)
    .bind(req.enabled.unwrap_or(true))
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("schedule 不存在".to_string()))?;
    Ok(Json(row))
}

#[utoipa::path(delete, path = "/api/v2/schedules/{id}", tag = "schedules", params(("id" = Uuid, Path)), responses((status = 200, body = DeleteScheduleResponse)))]
pub async fn delete_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<DeleteScheduleResponse>> {
    let rows = sqlx::query("DELETE FROM schedule_jobs WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    Ok(Json(DeleteScheduleResponse { ok: rows > 0 }))
}

#[utoipa::path(post, path = "/api/v2/schedules/{id}/run", tag = "schedules", params(("id" = Uuid, Path)), responses((status = 200, body = RunScheduleResponse)))]
pub async fn run_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<RunScheduleResponse>> {
    let job: ScheduleJob = sqlx::query_as("SELECT * FROM schedule_jobs WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("schedule 不存在".to_string()))?;
    if !job.enabled {
        return Err(AppError::Conflict("schedule 已禁用，不能运行".to_string()));
    }
    validate_kind(&job.kind)?;
    let task = start_schedule_preview_task(state, id, "manual").await?;
    Ok(Json(RunScheduleResponse { tid: task.id, task }))
}

pub async fn start_schedule_preview_task(
    state: AppState,
    schedule_id: Uuid,
    source: &str,
) -> AppResult<tasks::TaskRun> {
    if !matches!(source, "manual" | "schedule") {
        return Err(AppError::BadRequest(format!(
            "unknown schedule run source: {source}"
        )));
    }
    let mut tx = state.pool.begin().await?;
    let job: ScheduleJob = sqlx::query_as("SELECT * FROM schedule_jobs WHERE id = $1 FOR UPDATE")
        .bind(schedule_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound("schedule 不存在".to_string()))?;
    if !job.enabled {
        return Err(AppError::Conflict("schedule 已禁用，不能运行".to_string()));
    }
    validate_kind(&job.kind)?;
    if let Some(last_task_id) = job.last_task_id {
        let last_status: Option<String> =
            sqlx::query_scalar("SELECT status FROM task_runs WHERE id = $1")
                .bind(last_task_id)
                .fetch_optional(&mut *tx)
                .await?;
        if matches!(last_status.as_deref(), Some("pending" | "running")) {
            return Err(AppError::Conflict(
                "schedule 上一次任务仍在运行，跳过本次运行".to_string(),
            ));
        }
    }

    let task_params = serde_json::json!({
        "schedule_id": job.id,
        "schedule_name": job.name.clone(),
        "schedule": job.schedule.clone(),
        "params": job.params.clone(),
    });
    let label = format!("{}（scheduler preview dry run）", job.name);
    let task = sqlx::query_as::<_, tasks::TaskRun>(
        "INSERT INTO task_runs(id, kind, label, source, params, status, total)
         VALUES ($1, $2, $3, $4, $5, 'pending', 1)
         RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(&job.kind)
    .bind(&label)
    .bind(source)
    .bind(task_params)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE schedule_jobs
         SET last_run_at = now(), last_status = 'running', last_task_id = $2, last_error = NULL, updated_at = now()
         WHERE id = $1",
    )
    .bind(job.id)
    .bind(task.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    spawn_schedule_preview_worker(state, job.id, task.id, job.kind, label, source.to_string());
    Ok(task)
}

fn spawn_schedule_preview_worker(
    state: AppState,
    schedule_id: Uuid,
    task_id: Uuid,
    kind: String,
    label: String,
    source: String,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            return;
        };
        if let Err(err) =
            run_schedule_preview_worker(&state, schedule_id, task_id, &kind, &label, &source).await
        {
            let message = err.to_string();
            let _ = tasks::finish_error(
                &state.pool,
                task_id,
                &message,
                Some(serde_json::json!({
                    "ok": false,
                    "preview": true,
                    "dry_run": true,
                    "message": message,
                })),
            )
            .await;
            let _ =
                update_schedule_finished(&state.pool, schedule_id, "error", Some(&message)).await;
        }
    });
}

async fn run_schedule_preview_worker(
    state: &AppState,
    schedule_id: Uuid,
    task_id: Uuid,
    kind: &str,
    label: &str,
    source: &str,
) -> AppResult<()> {
    let running_message = "scheduler preview worker dry run: 不执行真实媒体变更";
    tasks::mark_running(&state.pool, task_id, running_message).await?;
    if tasks::cancel_requested(&state.pool, task_id).await {
        tasks::finish_cancelled(&state.pool, task_id).await?;
        update_schedule_finished(&state.pool, schedule_id, "cancelled", None).await?;
        return Ok(());
    }
    tasks::set_progress(&state.pool, task_id, 1, running_message).await?;
    let done_message = "scheduler preview worker dry run 完成：未执行真实业务";
    tasks::finish_done_with_message(
        &state.pool,
        task_id,
        done_message,
        serde_json::json!({
            "ok": true,
            "preview": true,
            "dry_run": true,
            "source": source,
            "kind": kind,
            "label": label,
            "message": done_message,
        }),
    )
    .await?;
    update_schedule_finished(&state.pool, schedule_id, "done", None).await?;
    Ok(())
}

async fn update_schedule_finished(
    pool: &sqlx::PgPool,
    schedule_id: Uuid,
    status: &str,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE schedule_jobs
         SET last_ended_at = now(), last_status = $2, last_error = $3, updated_at = now()
         WHERE id = $1",
    )
    .bind(schedule_id)
    .bind(status)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub fn validate_kind(kind: &str) -> AppResult<()> {
    if SUPPORTED_SCHEDULE_KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "unknown schedule kind: {kind}"
        )))
    }
}

pub fn validate_schedule(schedule: &Value) -> AppResult<()> {
    let mode = schedule.get("mode").and_then(Value::as_str).unwrap_or("");
    if !matches!(mode, "daily" | "weekly" | "monthly") {
        return Err(AppError::BadRequest(
            "mode 必须是 daily/weekly/monthly".to_string(),
        ));
    }
    let hour = schedule.get("hour").and_then(Value::as_i64).unwrap_or(-1);
    let minute = schedule.get("minute").and_then(Value::as_i64).unwrap_or(-1);
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return Err(AppError::BadRequest("hour/minute 越界".to_string()));
    }
    if mode == "weekly" {
        let weekday = schedule
            .get("weekday")
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        if !(0..=6).contains(&weekday) {
            return Err(AppError::BadRequest("weekday 必须 0-6".to_string()));
        }
    }
    if mode == "monthly" {
        let day = schedule.get("day").and_then(Value::as_i64).unwrap_or(-1);
        if !(1..=31).contains(&day) {
            return Err(AppError::BadRequest("day 必须 1-31".to_string()));
        }
    }
    Ok(())
}

pub fn next_run(schedule: &Value, now: DateTime<Utc>) -> AppResult<DateTime<Utc>> {
    validate_schedule(schedule)?;
    let hour = schedule["hour"].as_u64().unwrap() as u32;
    let minute = schedule["minute"].as_u64().unwrap() as u32;
    let target_time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap();
    let mut date = now.date_naive();
    for _ in 0..370 {
        let candidate = date.and_time(target_time).and_utc();
        let matches = match schedule["mode"].as_str().unwrap() {
            "daily" => true,
            "weekly" => {
                date.weekday().num_days_from_monday() as i64
                    == schedule["weekday"].as_i64().unwrap_or(0)
            }
            "monthly" => {
                let configured = schedule["day"].as_u64().unwrap_or(1) as u32;
                let last = last_day_of_month(date.year(), date.month());
                date.day() == configured.min(last)
            }
            _ => false,
        };
        if matches && candidate > now {
            return Ok(candidate);
        }
        date += Duration::days(1);
    }
    Err(AppError::BadRequest("无法计算 next_run".to_string()))
}

fn last_day_of_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    (chrono::NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - Duration::days(1)).day()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monthly_31_clamps_to_month_end() {
        let sch = serde_json::json!({"mode":"monthly","hour":3,"minute":0,"day":31});
        let now = DateTime::parse_from_rfc3339("2026-02-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let next = next_run(&sch, now).unwrap();
        assert_eq!(next.to_rfc3339(), "2026-02-28T03:00:00+00:00");
    }

    #[test]
    fn schedule_kind_must_be_known() {
        assert!(validate_kind("scan_all").is_ok());
        assert!(validate_kind("unknown").is_err());
    }
}
