use axum::extract::{Path, State};
use chrono::{Duration as ChronoDuration, Utc};
use emby_manager::{
    db,
    error::AppError,
    scheduler::{self, ScheduleJob},
    settings::Settings,
    state::AppState,
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::{env, path::PathBuf, time::Duration};
use tokio::time::sleep;
use uuid::Uuid;

static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn run_schedule_creates_preview_task_and_finishes_done() {
    let _guard = DB_LOCK.lock().await;
    let Some(state) = test_state().await else {
        eprintln!(
            "skipping scheduler preview DB test; set EMBY_MANAGER_SCHEDULER_TEST_DATABASE_URL"
        );
        return;
    };
    let job = insert_schedule(&state, "preview_ok", "scan_all", true).await;

    let response = scheduler::run_schedule(State(state.clone()), Path(job.id))
        .await
        .expect("run schedule should create a preview task")
        .0;

    assert_eq!(response.task.kind, "scan_all");
    assert_eq!(response.task.source, "manual");
    assert_eq!(response.task.params["schedule_id"], json!(job.id));
    assert!(response.task.label.contains("scheduler preview dry run"));

    let task = wait_for_task_status(&state, response.tid, "done").await;
    assert_eq!(task["source"], "manual");
    assert_eq!(task["kind"], "scan_all");
    assert_eq!(task["params"]["params"], json!({"scope": "test"}));
    assert_eq!(task["result"]["dry_run"], true);
    assert!(
        task["status_text"]
            .as_str()
            .unwrap_or_default()
            .contains("preview worker dry run"),
        "{task}"
    );

    let schedule = load_schedule(&state, job.id).await;
    assert_eq!(schedule.last_status.as_deref(), Some("done"));
    assert_eq!(schedule.last_task_id, Some(response.tid));
}

#[tokio::test]
async fn run_schedule_rejects_disabled_job_without_creating_task() {
    let _guard = DB_LOCK.lock().await;
    let Some(state) = test_state().await else {
        eprintln!(
            "skipping scheduler preview DB test; set EMBY_MANAGER_SCHEDULER_TEST_DATABASE_URL"
        );
        return;
    };
    let job = insert_schedule(&state, "preview_disabled", "scan_all", false).await;

    let err = scheduler::run_schedule(State(state.clone()), Path(job.id))
        .await
        .expect_err("disabled schedule should be rejected");
    assert!(matches!(err, AppError::Conflict(_)));
    assert_eq!(task_count_for_schedule(&state, job.id).await, 0);
}

#[tokio::test]
async fn run_schedule_rejects_unknown_kind_without_creating_task() {
    let _guard = DB_LOCK.lock().await;
    let Some(state) = test_state().await else {
        eprintln!(
            "skipping scheduler preview DB test; set EMBY_MANAGER_SCHEDULER_TEST_DATABASE_URL"
        );
        return;
    };
    let job = insert_schedule(&state, "preview_unknown", "unknown_preview_kind", true).await;

    let err = scheduler::run_schedule(State(state.clone()), Path(job.id))
        .await
        .expect_err("unknown schedule kind should be rejected");
    assert!(matches!(err, AppError::BadRequest(_)));
    assert_eq!(task_count_for_schedule(&state, job.id).await, 0);
}

#[tokio::test]
async fn scheduler_tick_starts_due_preview_task_once() {
    let _guard = DB_LOCK.lock().await;
    let Some(state) = test_state().await else {
        eprintln!(
            "skipping scheduler preview DB test; set EMBY_MANAGER_SCHEDULER_TEST_DATABASE_URL"
        );
        return;
    };
    let now = Utc::now();
    let job = sqlx::query_as::<_, ScheduleJob>(
        "INSERT INTO schedule_jobs(id, name, kind, params, schedule, enabled, last_run_at)
         VALUES ($1, $2, 'scan_all', $3, $4, true, $5)
         RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(format!("due_{}", Uuid::new_v4().simple()))
    .bind(json!({"scope": "tick"}))
    .bind(json!({"mode": "daily", "hour": 0, "minute": 0}))
    .bind(now - ChronoDuration::days(1))
    .fetch_one(&state.pool)
    .await
    .expect("insert due schedule");

    let started = scheduler::tick_due_schedules(&state, now)
        .await
        .expect("tick due schedules");

    assert_eq!(started, 1);
    let schedule = load_schedule(&state, job.id).await;
    let task_id = schedule.last_task_id.expect("last task id");
    let task = wait_for_task_status(&state, task_id, "done").await;
    assert_eq!(task["source"], "schedule");
    assert_eq!(task["params"]["schedule_id"], json!(job.id));
    assert_eq!(task["result"]["dry_run"], true);

    let started_again = scheduler::tick_due_schedules(&state, now)
        .await
        .expect("second tick due schedules");
    assert_eq!(started_again, 0);
}

async fn insert_schedule(state: &AppState, name: &str, kind: &str, enabled: bool) -> ScheduleJob {
    sqlx::query_as::<_, ScheduleJob>(
        "INSERT INTO schedule_jobs(id, name, kind, params, schedule, enabled)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(format!("{}_{}", name, Uuid::new_v4().simple()))
    .bind(kind)
    .bind(json!({"scope": "test"}))
    .bind(json!({"mode": "daily", "hour": 3, "minute": 0}))
    .bind(enabled)
    .fetch_one(&state.pool)
    .await
    .expect("insert schedule")
}

async fn load_schedule(state: &AppState, id: Uuid) -> ScheduleJob {
    sqlx::query_as::<_, ScheduleJob>("SELECT * FROM schedule_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .expect("load schedule")
}

async fn task_count_for_schedule(state: &AppState, schedule_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM task_runs WHERE params->>'schedule_id' = $1")
        .bind(schedule_id.to_string())
        .fetch_one(&state.pool)
        .await
        .expect("count tasks")
}

async fn wait_for_task_status(state: &AppState, id: Uuid, status: &str) -> Value {
    for _ in 0..50 {
        let task: Value =
            sqlx::query_scalar("SELECT to_jsonb(task_runs) FROM task_runs WHERE id = $1")
                .bind(id)
                .fetch_one(&state.pool)
                .await
                .expect("load task");
        if task["status"] == status {
            return task;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("task {id} did not reach {status}");
}

async fn test_state() -> Option<AppState> {
    let database_url = scheduler_test_database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect scheduler test database");
    db::migrate(&pool)
        .await
        .expect("run scheduler test migrations");
    sqlx::query("TRUNCATE task_runs, schedule_jobs RESTART IDENTITY CASCADE")
        .execute(&pool)
        .await
        .expect("reset scheduler test tables");
    Some(AppState::new(pool, test_settings(database_url)))
}

fn scheduler_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_SCHEDULER_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn test_settings(database_url: String) -> Settings {
    Settings {
        host: "127.0.0.1".to_string(),
        port: 0,
        database_url,
        web_dist: PathBuf::from("/tmp"),
        legacy_dir: PathBuf::from("/tmp"),
        bootstrap_password: "admin".to_string(),
        cd_root: PathBuf::from("/tmp/cd"),
        strm_root: PathBuf::from("/tmp/strm"),
        docker_bin: PathBuf::from("/usr/bin/docker"),
        task_concurrency: 1,
    }
}
