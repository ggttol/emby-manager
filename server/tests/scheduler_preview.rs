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
use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::sleep,
};
use uuid::Uuid;

static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn run_schedule_creates_real_task_and_finishes_done() {
    let _guard = DB_LOCK.lock().await;
    let Some(state) = test_state().await else {
        eprintln!(
            "skipping scheduler preview DB test; set EMBY_MANAGER_SCHEDULER_TEST_DATABASE_URL"
        );
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![FakeResponse::no_content()]).await;
    configure_emby(&state, &base_url).await;
    let job = insert_schedule(&state, "preview_ok", "scan_all", true).await;

    let response = scheduler::run_schedule(State(state.clone()), Path(job.id))
        .await
        .expect("run schedule should create a real task")
        .0;

    assert_eq!(response.task.kind, "scan_all");
    assert_eq!(response.task.source, "manual");
    assert_eq!(response.task.params["schedule_id"], json!(job.id));
    assert!(!response.task.label.contains("preview dry run"));

    let task = wait_for_task_status(&state, response.tid, "done").await;
    assert_eq!(task["source"], "manual");
    assert_eq!(task["kind"], "scan_all");
    assert_eq!(task["params"]["params"], json!({"scope": "test"}));
    assert_eq!(task["result"]["dry_run"], false);
    assert_eq!(task["result"]["preview"], false);
    assert_eq!(task["result"]["detail"]["action"], "refresh_library");
    assert_eq!(task["result"]["detail"]["refresh_code"], 204);
    assert!(
        task["status_text"]
            .as_str()
            .unwrap_or_default()
            .contains("scheduler worker 完成"),
        "{task}"
    );

    let schedule = wait_for_schedule_status(&state, job.id, "done").await;
    assert_eq!(schedule.last_status.as_deref(), Some("done"));
    assert_eq!(schedule.last_task_id, Some(response.tid));

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("POST /Library/Refresh?api_key=secret-key"),
        "{}",
        requests[0]
    );
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
async fn scheduler_tick_starts_due_real_task_once() {
    let _guard = DB_LOCK.lock().await;
    let Some(state) = test_state().await else {
        eprintln!(
            "skipping scheduler preview DB test; set EMBY_MANAGER_SCHEDULER_TEST_DATABASE_URL"
        );
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![FakeResponse::no_content()]).await;
    configure_emby(&state, &base_url).await;
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
    assert_eq!(task["result"]["dry_run"], false);
    assert_eq!(task["result"]["detail"]["action"], "refresh_library");

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);

    let started_again = scheduler::tick_due_schedules(&state, now)
        .await
        .expect("second tick due schedules");
    assert_eq!(started_again, 0);
}

async fn configure_emby(state: &AppState, base_url: &str) {
    for (key, value) in [
        ("emby_url", json!(base_url)),
        ("api_key", json!("secret-key")),
    ] {
        sqlx::query(
            "INSERT INTO app_settings(key, value, updated_at) VALUES ($1, $2, now())
             ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(&state.pool)
        .await
        .expect("save Emby config");
    }
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

async fn wait_for_schedule_status(state: &AppState, id: Uuid, status: &str) -> ScheduleJob {
    for _ in 0..50 {
        let schedule = load_schedule(state, id).await;
        if schedule.last_status.as_deref() == Some(status) {
            return schedule;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("schedule {id} did not reach {status}");
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
    sqlx::query("TRUNCATE task_runs, schedule_jobs, app_settings RESTART IDENTITY CASCADE")
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

#[derive(Clone)]
struct FakeResponse {
    status: &'static str,
    body: &'static str,
}

impl FakeResponse {
    fn no_content() -> Self {
        Self {
            status: "204 No Content",
            body: "",
        }
    }
}

async fn spawn_fake_emby(responses: Vec<FakeResponse>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);

    tokio::spawn(async move {
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            captured
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[..n]).to_string());

            let raw = format!(
                "HTTP/1.1 {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response.status,
                response.body.len(),
                response.body
            );
            socket.write_all(raw.as_bytes()).await.unwrap();
        }
    });

    (format!("http://{addr}"), requests)
}
