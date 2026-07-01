use crate::{
    config_store,
    emby::{EmbyClient, EmbyLibrary},
    error::{AppError, AppResult},
    media_fs, posters, smart_actions,
    state::AppState,
    tasks, zhuigeng,
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
use std::{
    path::Path as FsPath,
    time::{Duration as StdDuration, SystemTime, UNIX_EPOCH},
};
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

const SCHEDULER_POLL_SECONDS: u64 = 30;
const SCHEDULER_ADVISORY_LOCK_ID: i64 = 8_097_809_842;
const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const DEFAULT_POSTER_FIX_LIMIT: usize = 200;
const DEFAULT_SERIES_REFRESH_LIMIT: usize = 500;
const DEFAULT_INCREMENTAL_LIMIT: usize = 200;
const SCHEDULE_CANCELLED_SENTINEL: &str = "__schedule_cancelled__";

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
    "smart_actions_refresh",
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

pub fn spawn_scheduler_loop(state: AppState) {
    tokio::spawn(async move {
        tracing::info!(
            poll_seconds = SCHEDULER_POLL_SECONDS,
            "scheduler loop started"
        );
        let mut interval = tokio::time::interval(StdDuration::from_secs(SCHEDULER_POLL_SECONDS));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            match tick_due_schedules(&state, Utc::now()).await {
                Ok(started) if started > 0 => {
                    tracing::info!(started, "scheduler started due jobs");
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(error = %err, "scheduler tick failed");
                }
            }
        }
    });
}

pub async fn reconcile_interrupted(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE schedule_jobs sj
         SET last_status = 'interrupted',
             last_error = COALESCE(last_error, '服务重启后中断'),
             last_ended_at = now(),
             updated_at = now()
         WHERE last_status IN ('running', 'watch_timeout')
           AND (
             last_task_id IS NULL
             OR NOT EXISTS (
                SELECT 1 FROM task_runs tr
                WHERE tr.id = sj.last_task_id
                  AND tr.status IN ('pending', 'running')
             )
           )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn tick_due_schedules(state: &AppState, now: DateTime<Utc>) -> AppResult<usize> {
    let mut tx = state.pool.begin().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(SCHEDULER_ADVISORY_LOCK_ID)
        .fetch_one(&mut *tx)
        .await?;
    if !locked {
        return Ok(0);
    }

    let jobs: Vec<ScheduleJob> = sqlx::query_as(
        "SELECT * FROM schedule_jobs
         WHERE enabled = TRUE
         ORDER BY created_at ASC",
    )
    .fetch_all(&mut *tx)
    .await?;

    let due_ids = jobs
        .into_iter()
        .filter_map(|job| match is_due(&job.schedule, job.last_run_at, now) {
            Ok(true) => Some(job.id),
            Ok(false) => None,
            Err(err) => {
                tracing::warn!(schedule_id = %job.id, error = %err, "invalid schedule skipped");
                None
            }
        })
        .collect::<Vec<_>>();

    let mut started = 0usize;
    for schedule_id in due_ids {
        match start_schedule_task(state.clone(), schedule_id, "schedule").await {
            Ok(_) => started += 1,
            Err(AppError::Conflict(err)) => {
                tracing::info!(%schedule_id, error = %err, "due schedule skipped");
            }
            Err(err) => {
                tracing::warn!(%schedule_id, error = %err, "due schedule failed to start");
                mark_schedule_start_error(&state.pool, schedule_id, now, &err.to_string()).await?;
            }
        }
    }
    tx.commit().await?;
    Ok(started)
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
    let now = Utc::now();
    let last_run_at = is_due(&req.schedule, None, now)?.then_some(now);
    let row = sqlx::query_as::<_, ScheduleJob>(
        "INSERT INTO schedule_jobs(id, name, kind, params, schedule, enabled, last_run_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(req.name)
    .bind(req.kind)
    .bind(req.params.unwrap_or_else(|| serde_json::json!({})))
    .bind(req.schedule)
    .bind(req.enabled.unwrap_or(true))
    .bind(last_run_at)
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
    let existing: ScheduleJob = sqlx::query_as("SELECT * FROM schedule_jobs WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("schedule 不存在".to_string()))?;
    let schedule_changed = existing.schedule != req.schedule;
    let last_run_at = if schedule_changed {
        let now = Utc::now();
        is_due(&req.schedule, None, now)?.then_some(now)
    } else {
        existing.last_run_at
    };
    let row = sqlx::query_as::<_, ScheduleJob>(
        "UPDATE schedule_jobs
         SET name = $2, kind = $3, params = $4, schedule = $5, enabled = $6, last_run_at = $7, updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(req.name)
    .bind(req.kind)
    .bind(req.params.unwrap_or_else(|| serde_json::json!({})))
    .bind(req.schedule)
    .bind(req.enabled.unwrap_or(true))
    .bind(last_run_at)
    .fetch_one(&state.pool)
    .await?;
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
    let task = start_schedule_task(state, id, "manual").await?;
    Ok(Json(RunScheduleResponse { tid: task.id, task }))
}

pub async fn start_schedule_task(
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
    let label = format!("定时任务: {}", job.name);
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

    spawn_schedule_worker(
        state,
        job.id,
        task.id,
        job.kind,
        label,
        source.to_string(),
        job.params,
    );
    Ok(task)
}

fn spawn_schedule_worker(
    state: AppState,
    schedule_id: Uuid,
    task_id: Uuid,
    kind: String,
    label: String,
    source: String,
    params: Value,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            return;
        };
        if let Err(err) = run_schedule_worker(
            &state,
            schedule_id,
            task_id,
            &kind,
            &label,
            &source,
            &params,
        )
        .await
        {
            let message = err.to_string();
            if message == SCHEDULE_CANCELLED_SENTINEL {
                let _ = update_schedule_finished(&state.pool, schedule_id, "cancelled", None).await;
                return;
            }
            let _ = tasks::finish_error(
                &state.pool,
                task_id,
                &message,
                Some(serde_json::json!({
                    "ok": false,
                    "preview": false,
                    "dry_run": false,
                    "message": message,
                })),
            )
            .await;
            let _ =
                update_schedule_finished(&state.pool, schedule_id, "error", Some(&message)).await;
        }
    });
}

async fn run_schedule_worker(
    state: &AppState,
    schedule_id: Uuid,
    task_id: Uuid,
    kind: &str,
    label: &str,
    source: &str,
    params: &Value,
) -> AppResult<()> {
    let running_message = "scheduler worker running";
    tasks::mark_running(&state.pool, task_id, running_message).await?;
    check_schedule_cancelled(state, task_id).await?;

    let result = match kind {
        "scan_all" => run_scheduled_scan_all(state, task_id, params).await?,
        "zhuigeng_scan_airing" => {
            run_scheduled_zhuigeng_scan_airing(state, task_id, params).await?
        }
        "fix_posters_all" => run_scheduled_fix_posters_all(state, task_id, params).await?,
        "refresh_no_rating_all" => {
            run_scheduled_refresh_no_rating_all(state, task_id, params).await?
        }
        "monitor_incremental" => run_scheduled_monitor_incremental(state, task_id, params).await?,
        "smart_actions_refresh" => run_scheduled_smart_actions_refresh(state, task_id).await?,
        _ => {
            return Err(AppError::BadRequest(format!(
                "unknown schedule kind: {kind}"
            )));
        }
    };

    let done_message = format!("scheduler worker 完成: {kind}");
    tasks::finish_done_with_message(
        &state.pool,
        task_id,
        &done_message,
        serde_json::json!({
            "ok": true,
            "preview": false,
            "dry_run": false,
            "source": source,
            "kind": kind,
            "label": label,
            "message": done_message,
            "detail": result,
        }),
    )
    .await?;
    update_schedule_finished(&state.pool, schedule_id, "done", None).await?;
    Ok(())
}

async fn run_scheduled_smart_actions_refresh(state: &AppState, task_id: Uuid) -> AppResult<Value> {
    check_schedule_cancelled(state, task_id).await?;
    smart_actions::run_smart_actions_refresh_for_task(state, task_id).await
}

async fn run_scheduled_scan_all(
    state: &AppState,
    task_id: Uuid,
    params: &Value,
) -> AppResult<Value> {
    check_schedule_cancelled(state, task_id).await?;
    let client = scheduled_emby_client(state).await?;
    match media_fs::run_scheduled_scan_all_libraries(state, task_id, &client, params).await {
        Err(AppError::Conflict(message)) if message == media_fs::SCAN_TASK_CANCELLED_SENTINEL => {
            Err(AppError::Conflict(SCHEDULE_CANCELLED_SENTINEL.to_string()))
        }
        other => other,
    }
}

async fn run_scheduled_zhuigeng_scan_airing(
    state: &AppState,
    task_id: Uuid,
    _params: &Value,
) -> AppResult<Value> {
    let response = match zhuigeng::zhuigeng_scan_airing_for_state(state, Some(task_id)).await {
        Err(AppError::Conflict(message)) if message == zhuigeng::ZHUIGENG_SCAN_AIRING_CANCELLED => {
            return Err(AppError::Conflict(SCHEDULE_CANCELLED_SENTINEL.to_string()));
        }
        other => other?,
    };
    Ok(zhuigeng_scan_airing_schedule_detail(response))
}

pub fn zhuigeng_scan_airing_schedule_detail(
    response: zhuigeng::ZhuigengScanAiringResponse,
) -> Value {
    serde_json::json!({
        "action": "zhuigeng_scan_airing",
        "message": "已按 continuing 剧名串行扫描对应追更库",
        "total": response.total,
        "ok_count": response.ok_count,
        "error_count": response.error_count,
        "new_count": response.new_count,
        "copy_text": response.copy_text,
        "note": response.note,
        "results": response.results,
    })
}

async fn run_scheduled_fix_posters_all(
    state: &AppState,
    task_id: Uuid,
    params: &Value,
) -> AppResult<Value> {
    let client = scheduled_emby_client(state).await?;
    let limit = params_usize(params, "limit", DEFAULT_POSTER_FIX_LIMIT, 1, 1000);
    tasks::set_progress(&state.pool, task_id, 0, "扫描无海报项目").await?;
    let detected = posters::detect_mismatched_posters(
        &client,
        posters::PosterDetectRequest {
            lib: params
                .get("lib")
                .and_then(Value::as_str)
                .map(str::to_string),
            limit: Some(limit),
            include_missing_primary: Some(true),
        },
    )
    .await?;
    let targets = detected
        .items
        .into_iter()
        .filter(|item| !item.has_poster)
        .take(limit)
        .collect::<Vec<_>>();
    tasks::set_total(&state.pool, task_id, targets.len().max(1) as i64).await?;
    if targets.is_empty() {
        tasks::set_progress(&state.pool, task_id, 1, "没有需要自动修复的无海报项目").await?;
        return Ok(serde_json::json!({
            "action": "fix_posters_all",
            "total": 0,
            "ok_count": 0,
            "results": [],
        }));
    }

    let mut results = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        check_schedule_cancelled(state, task_id).await?;
        tasks::set_progress(
            &state.pool,
            task_id,
            index as i64,
            &format!("修海报 {}", target.emby_name),
        )
        .await?;
        results.push(
            posters::fix_poster_one(&state.pool, &client, &target.id, &target.item_type).await,
        );
        tasks::set_progress(
            &state.pool,
            task_id,
            (index + 1) as i64,
            &format!("海报修复 {}/{}", index + 1, targets.len()),
        )
        .await?;
        tokio::time::sleep(StdDuration::from_millis(500)).await;
    }
    let ok_count = results.iter().filter(|row| row.ok).count();
    Ok(serde_json::json!({
        "action": "fix_posters_all",
        "total": results.len(),
        "ok_count": ok_count,
        "results": results,
    }))
}

async fn run_scheduled_refresh_no_rating_all(
    state: &AppState,
    task_id: Uuid,
    params: &Value,
) -> AppResult<Value> {
    let client = scheduled_emby_client(state).await?;
    let limit = params_usize(params, "limit", DEFAULT_SERIES_REFRESH_LIMIT, 1, 5000);
    tasks::set_progress(&state.pool, task_id, 0, "读取剧集库").await?;
    let mut targets = Vec::new();
    for library in client.libraries().await? {
        if !is_tv_library(&library) {
            continue;
        }
        let Some(parent_id) = library.id.as_deref().filter(|id| !id.trim().is_empty()) else {
            continue;
        };
        for item in client
            .series(parent_id, limit.saturating_sub(targets.len()))
            .await?
        {
            let Some(id) = item.id else {
                continue;
            };
            targets.push((library.name.clone(), id, item.name.unwrap_or_default()));
            if targets.len() >= limit {
                break;
            }
        }
        if targets.len() >= limit {
            break;
        }
    }

    tasks::set_total(&state.pool, task_id, targets.len().max(1) as i64).await?;
    if targets.is_empty() {
        tasks::set_progress(&state.pool, task_id, 1, "没有找到剧集条目").await?;
        return Ok(serde_json::json!({
            "action": "refresh_no_rating_all",
            "refreshed": 0,
            "note": "Rust 当前未单独读取评分字段，本任务会刷新剧集元数据以补齐评分/海报/简介",
        }));
    }

    let mut refreshed = Vec::new();
    for (index, (lib, id, name)) in targets.iter().enumerate() {
        check_schedule_cancelled(state, task_id).await?;
        tasks::set_progress(
            &state.pool,
            task_id,
            index as i64,
            &format!("刷新元数据 {}", name),
        )
        .await?;
        let code = client.refresh_item(id, true, false).await?;
        refreshed.push(serde_json::json!({
            "lib": lib,
            "id": id,
            "name": name,
            "refresh_code": code,
        }));
        tasks::set_progress(
            &state.pool,
            task_id,
            (index + 1) as i64,
            &format!("元数据刷新 {}/{}", index + 1, targets.len()),
        )
        .await?;
        tokio::time::sleep(StdDuration::from_millis(500)).await;
    }

    Ok(serde_json::json!({
        "action": "refresh_no_rating_all",
        "refreshed": refreshed.len(),
        "note": "Rust 当前未单独读取评分字段，本任务会刷新剧集元数据以补齐评分/海报/简介",
        "items": refreshed,
    }))
}

async fn run_scheduled_monitor_incremental(
    state: &AppState,
    task_id: Uuid,
    params: &Value,
) -> AppResult<Value> {
    let limit = params_usize(params, "limit", DEFAULT_INCREMENTAL_LIMIT, 1, 5000);
    tasks::set_progress(&state.pool, task_id, 0, "扫描媒体根增量目录").await?;
    let due = collect_incremental_tops(state, limit).await?;
    tasks::set_total(&state.pool, task_id, due.len().max(1) as i64).await?;
    if due.is_empty() {
        tasks::set_progress(&state.pool, task_id, 1, "没有发现增量 top 目录").await?;
        return Ok(serde_json::json!({
            "action": "monitor_incremental",
            "processed": 0,
            "new_strm": 0,
            "attention": [],
        }));
    }

    let mut processed = 0usize;
    let mut new_strm = 0usize;
    let mut attention = Vec::new();
    for (index, top) in due.iter().enumerate() {
        check_schedule_cancelled(state, task_id).await?;
        tasks::set_progress(
            &state.pool,
            task_id,
            index as i64,
            &format!("补扫 {}/{}", top.lib, top.top),
        )
        .await?;
        let result =
            media_fs::generate_missing_strm_for_library(state, &top.lib, Some(&top.top), true)?;
        new_strm += result.new_count;
        attention.extend(result.attention);
        sqlx::query(
            "INSERT INTO autostrm_seen(lib, top, mtime, updated_at)
             VALUES ($1, $2, $3, now())
             ON CONFLICT(lib, top) DO UPDATE
             SET mtime = EXCLUDED.mtime, updated_at = now()",
        )
        .bind(&top.lib)
        .bind(&top.top)
        .bind(top.mtime)
        .execute(&state.pool)
        .await?;
        processed += 1;
        tasks::set_progress(
            &state.pool,
            task_id,
            (index + 1) as i64,
            &format!("增量补扫 {}/{}", index + 1, due.len()),
        )
        .await?;
        tokio::time::sleep(StdDuration::from_millis(500)).await;
    }

    Ok(serde_json::json!({
        "action": "monitor_incremental",
        "processed": processed,
        "new_strm": new_strm,
        "attention": attention,
    }))
}

async fn scheduled_emby_client(state: &AppState) -> AppResult<EmbyClient> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; set it via /api/v2/config before running schedules"
                .to_string(),
        ));
    }
    Ok(EmbyClient::new(emby_url, api_key, state.http.clone()))
}

async fn check_schedule_cancelled(state: &AppState, task_id: Uuid) -> AppResult<()> {
    if tasks::cancel_requested(&state.pool, task_id).await {
        tasks::finish_cancelled(&state.pool, task_id).await?;
        return Err(AppError::Conflict(SCHEDULE_CANCELLED_SENTINEL.to_string()));
    }
    Ok(())
}

fn params_usize(params: &Value, key: &str, default: usize, min: usize, max: usize) -> usize {
    params
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(default)
        .clamp(min, max)
}

fn is_tv_library(library: &EmbyLibrary) -> bool {
    let value = library.library_type.to_ascii_lowercase();
    value.contains("tv") || value.contains("show")
}

#[derive(Debug)]
struct IncrementalTop {
    lib: String,
    top: String,
    mtime: i64,
}

async fn collect_incremental_tops(
    state: &AppState,
    limit: usize,
) -> AppResult<Vec<IncrementalTop>> {
    let mut due = Vec::new();
    let root = &state.settings.cd_root;
    if !root.is_dir() {
        return Err(AppError::NotFound(format!(
            "媒体根不存在: {}",
            root.display()
        )));
    }
    let libs = read_dir_names(root)?;
    for lib in libs {
        let lib_path = root.join(&lib);
        if !lib_path.is_dir() {
            continue;
        }
        for top in read_dir_names(&lib_path)? {
            let top_path = lib_path.join(&top);
            if !top_path.is_dir() {
                continue;
            }
            let mtime = path_mtime_seconds(&top_path);
            let seen: Option<i64> =
                sqlx::query_scalar("SELECT mtime FROM autostrm_seen WHERE lib = $1 AND top = $2")
                    .bind(&lib)
                    .bind(&top)
                    .fetch_optional(&state.pool)
                    .await?;
            if seen.is_none_or(|value| mtime > value) {
                due.push(IncrementalTop {
                    lib: lib.clone(),
                    top,
                    mtime,
                });
            }
            if due.len() >= limit {
                return Ok(due);
            }
        }
    }
    Ok(due)
}

fn read_dir_names(path: &FsPath) -> AppResult<Vec<String>> {
    let mut names = std::fs::read_dir(path)
        .map_err(|err| AppError::Anyhow(err.into()))?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

fn path_mtime_seconds(path: &FsPath) -> i64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or_else(|_| SystemTime::now())
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
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

async fn mark_schedule_start_error(
    pool: &sqlx::PgPool,
    schedule_id: Uuid,
    now: DateTime<Utc>,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE schedule_jobs
         SET last_run_at = $2, last_ended_at = now(), last_status = 'error',
             last_error = $3, updated_at = now()
         WHERE id = $1",
    )
    .bind(schedule_id)
    .bind(now)
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

pub fn is_due(
    schedule: &Value,
    last_run_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    validate_schedule(schedule)?;
    let hour = schedule["hour"].as_u64().unwrap() as u32;
    let minute = schedule["minute"].as_u64().unwrap() as u32;
    let target_time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap();
    let target = now.date_naive().and_time(target_time).and_utc();
    if now < target {
        return Ok(false);
    }
    match schedule["mode"].as_str().unwrap() {
        "daily" => {}
        "weekly" => {
            let weekday = schedule["weekday"].as_i64().unwrap_or(0) as u32;
            if now.weekday().num_days_from_monday() != weekday {
                return Ok(false);
            }
        }
        "monthly" => {
            let configured = schedule["day"].as_u64().unwrap_or(1) as u32;
            let last = last_day_of_month(now.year(), now.month());
            if now.day() != configured.min(last) {
                return Ok(false);
            }
        }
        _ => return Ok(false),
    }
    if last_run_at.is_some_and(|last| same_schedule_period(schedule, last, now)) {
        return Ok(false);
    }
    Ok(true)
}

fn same_schedule_period(schedule: &Value, last: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    match schedule["mode"].as_str().unwrap_or("") {
        "daily" => last.date_naive() == now.date_naive(),
        "weekly" => last.iso_week() == now.iso_week(),
        "monthly" => last.year() == now.year() && last.month() == now.month(),
        _ => false,
    }
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
        assert!(validate_kind("smart_actions_refresh").is_ok());
        assert!(validate_kind("unknown").is_err());
    }

    #[test]
    fn due_runs_after_target_once_per_daily_period() {
        let sch = serde_json::json!({"mode":"daily","hour":3,"minute":0});
        let now = DateTime::parse_from_rfc3339("2026-05-28T03:10:00Z")
            .unwrap()
            .with_timezone(&Utc);

        assert!(is_due(&sch, None, now).unwrap());
        assert!(
            !is_due(
                &sch,
                Some(
                    DateTime::parse_from_rfc3339("2026-05-28T03:00:00Z")
                        .unwrap()
                        .with_timezone(&Utc)
                ),
                now
            )
            .unwrap()
        );
        assert!(
            is_due(
                &sch,
                Some(
                    DateTime::parse_from_rfc3339("2026-05-27T03:00:00Z")
                        .unwrap()
                        .with_timezone(&Utc)
                ),
                now
            )
            .unwrap()
        );
    }

    #[test]
    fn due_respects_weekly_and_monthly_periods() {
        let weekly = serde_json::json!({"mode":"weekly","hour":3,"minute":0,"weekday":3});
        let thursday = DateTime::parse_from_rfc3339("2026-05-28T03:10:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let friday = DateTime::parse_from_rfc3339("2026-05-29T03:10:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(is_due(&weekly, None, thursday).unwrap());
        assert!(!is_due(&weekly, None, friday).unwrap());

        let monthly = serde_json::json!({"mode":"monthly","hour":3,"minute":0,"day":31});
        let feb_end = DateTime::parse_from_rfc3339("2026-02-28T03:10:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(is_due(&monthly, None, feb_end).unwrap());
    }
}
