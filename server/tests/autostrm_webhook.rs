use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue},
};
use emby_manager::{
    autostrm::{self, AutostrmWebhookQuery},
    db,
    settings::Settings,
    state::AppState,
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::{env, path::PathBuf, time::Duration};
use tempfile::TempDir;
use tokio::time::sleep;

#[tokio::test]
async fn webhook_generates_strm_and_records_seen() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping autostrm DB test; set EMBY_MANAGER_AUTOSTRM_TEST_DATABASE_URL");
        return;
    };
    configure_autostrm(&state, true).await;
    std::fs::create_dir_all(state.settings.cd_root.join("Movies/Movie [tmdbid-123]")).unwrap();
    std::fs::write(
        state
            .settings
            .cd_root
            .join("Movies/Movie [tmdbid-123]/Movie.mkv"),
        "video",
    )
    .unwrap();

    let response = autostrm::webhook(
        State(state.clone()),
        Query(AutostrmWebhookQuery { key: None }),
        secret_headers(),
        Json(json!({
            "data": [
                {
                    "action": "add",
                    "is_dir": false,
                    "source_file": "/CloudNAS/CloudDrive/Movies/Movie [tmdbid-123]/Movie.mkv"
                },
                {
                    "action": "delete",
                    "is_dir": false,
                    "source_file": "/CloudNAS/CloudDrive/Movies/Deleted/Gone.mkv"
                }
            ]
        })),
    )
    .await
    .expect("webhook should accept event")
    .0;

    assert_eq!(response.queued, 1);
    assert_eq!(response.ignored, 1);
    let task_id = response.tid.expect("task id");
    let task = wait_for_task_status(&state, task_id, "done").await;
    assert_eq!(task["result"]["processed"], 1);
    assert_eq!(task["result"]["new_strm"], 1);
    assert_eq!(task["result"]["unmatched"], 0);
    assert_eq!(
        std::fs::read_to_string(
            state
                .settings
                .strm_root
                .join("Movies/Movie [tmdbid-123]/Movie.strm")
        )
        .unwrap(),
        "/media/Movies/Movie [tmdbid-123]/Movie.mkv"
    );
    let seen_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM autostrm_seen WHERE lib = 'Movies'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(seen_count, 1);
}

#[tokio::test]
async fn webhook_reports_disabled_without_task() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping autostrm DB test; set EMBY_MANAGER_AUTOSTRM_TEST_DATABASE_URL");
        return;
    };
    configure_autostrm(&state, false).await;

    let response = autostrm::webhook(
        State(state),
        Query(AutostrmWebhookQuery { key: None }),
        secret_headers(),
        Json(json!({"data": []})),
    )
    .await
    .expect("disabled webhook should return ok")
    .0;

    assert!(response.disabled);
    assert_eq!(response.queued, 0);
    assert!(response.tid.is_none());
}

async fn configure_autostrm(state: &AppState, enabled: bool) {
    for (key, value) in [
        ("cd2_webhook_secret", json!("secret")),
        ("auto_strm_enabled", json!(enabled)),
        ("cd2_mount_prefix", json!("/CloudNAS/CloudDrive")),
    ] {
        sqlx::query(
            "INSERT INTO app_settings(key, value, updated_at) VALUES ($1, $2, now())
             ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(&state.pool)
        .await
        .expect("save autostrm config");
    }
}

fn secret_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-webhook-secret", HeaderValue::from_static("secret"));
    headers
}

async fn wait_for_task_status(state: &AppState, id: uuid::Uuid, status: &str) -> Value {
    for _ in 0..80 {
        let task: Value =
            sqlx::query_scalar("SELECT to_jsonb(task_runs) FROM task_runs WHERE id = $1")
                .bind(id)
                .fetch_one(&state.pool)
                .await
                .expect("load task");
        if task["status"] == status {
            return task;
        }
        if task["status"] == "error" {
            panic!("task {id} failed: {}", task["error"]);
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("task {id} did not reach {status}");
}

async fn test_state() -> Option<(TempDir, AppState)> {
    let database_url = autostrm_test_database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect autostrm test database");
    db::migrate(&pool)
        .await
        .expect("run autostrm test migrations");
    sqlx::query(
        "TRUNCATE task_runs, autostrm_seen, autostrm_unmatched, app_settings RESTART IDENTITY CASCADE",
    )
    .execute(&pool)
    .await
    .expect("reset autostrm test tables");
    let tmp = tempfile::tempdir().unwrap();
    let cd_root = tmp.path().join("cd");
    let strm_root = tmp.path().join("strm");
    std::fs::create_dir_all(&cd_root).unwrap();
    std::fs::create_dir_all(&strm_root).unwrap();
    Some((
        tmp,
        AppState::new(pool, test_settings(database_url, cd_root, strm_root)),
    ))
}

fn autostrm_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_AUTOSTRM_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn test_settings(database_url: String, cd_root: PathBuf, strm_root: PathBuf) -> Settings {
    Settings {
        host: "127.0.0.1".to_string(),
        port: 0,
        database_url,
        web_dist: PathBuf::from("/tmp"),
        legacy_dir: PathBuf::from("/tmp"),
        bootstrap_password: "admin".to_string(),
        cd_root,
        strm_root,
        docker_bin: PathBuf::from("/usr/bin/docker"),
        task_concurrency: 1,
    }
}
