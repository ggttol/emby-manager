use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        HeaderMap, Method, StatusCode,
        header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
    },
};
use emby_manager::{api, auth, db, settings::Settings};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    env,
    path::{Path, PathBuf},
};
use tower::ServiceExt;
use uuid::Uuid;

static SMART_ACTIONS_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn smart_actions_get_endpoints_are_readonly() {
    let _guard = SMART_ACTIONS_DB_LOCK.lock().await;
    let Some(database_url) = smart_actions_test_database_url() else {
        eprintln!(
            "skipping smart actions API test; set EMBY_MANAGER_SMART_ACTIONS_TEST_DATABASE_URL to enable it"
        );
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let pool = connect_and_migrate(&database_url).await;
    reset_and_seed_db(&pool).await;

    let username = create_test_user(&pool).await;
    let app = api::router(pool.clone(), test_settings(&database_url, tmp.path()));
    let (status, headers, body) = send(
        &app,
        Method::POST,
        "/api/v2/auth/login",
        Some(json!({
            "username": username,
            "password": "secret"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let cookie = request_cookie(&headers);

    let before_tasks = table_count(&pool, "task_runs").await;
    let before_runs = table_count(&pool, "smart_action_runs").await;

    let (status, _, list) = send(
        &app,
        Method::GET,
        "/api/v2/smart-actions?limit=10",
        None,
        &[(COOKIE.as_str(), cookie.clone())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["total"], json!(1), "{list}");
    assert_eq!(list["actions"].as_array().unwrap().len(), 1, "{list}");
    assert_eq!(
        list["actions"][0]["action_type"],
        json!("task_retry_or_diagnose")
    );
    assert_eq!(list["actions"][0]["status"], json!("suggested"));
    assert_eq!(list["summary"]["total"], json!(1));
    let action_id = list["actions"][0]["id"].as_str().unwrap().to_string();

    let (status, _, summary) = send(
        &app,
        Method::GET,
        "/api/v2/smart-actions/summary",
        None,
        &[(COOKIE.as_str(), cookie.clone())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    assert_eq!(summary["summary"]["total"], json!(1), "{summary}");
    assert_eq!(summary["summary"]["suggested"], json!(1), "{summary}");

    let (status, _, detail) = send(
        &app,
        Method::GET,
        &format!("/api/v2/smart-actions/{action_id}"),
        None,
        &[(COOKIE.as_str(), cookie)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["action"]["id"], json!(action_id));
    assert_eq!(
        detail["action"]["source"],
        json!("task_runs"),
        "detail should read the generated dashboard action"
    );

    assert_eq!(
        table_count(&pool, "task_runs").await,
        before_tasks,
        "GET smart-actions endpoints must not create task_runs"
    );
    assert_eq!(
        table_count(&pool, "smart_action_runs").await,
        before_runs,
        "GET smart-actions endpoints must not persist smart_action_runs"
    );
}

#[tokio::test]
async fn smart_actions_migration_creates_tables_indexes_and_defaults() {
    let _guard = SMART_ACTIONS_DB_LOCK.lock().await;
    let Some(database_url) = smart_actions_test_database_url() else {
        eprintln!(
            "skipping smart actions migration test; set EMBY_MANAGER_SMART_ACTIONS_TEST_DATABASE_URL to enable it"
        );
        return;
    };
    let pool = connect_and_migrate(&database_url).await;

    for table in ["smart_action_runs", "smart_action_policies"] {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{table}"))
            .fetch_one(&pool)
            .await
            .expect("check smart actions table");
        assert!(exists, "{table} should exist after migrations");
    }

    for index in [
        "idx_smart_action_runs_status",
        "idx_smart_action_runs_action_type",
        "idx_smart_action_runs_subject",
        "idx_smart_action_runs_evidence",
        "idx_smart_action_runs_source",
    ] {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{index}"))
            .fetch_one(&pool)
            .await
            .expect("check smart actions index");
        assert!(exists, "{index} should exist after migrations");
    }

    let policy_defaults = column_defaults(&pool, "smart_action_policies").await;
    assert_eq!(
        policy_defaults
            .iter()
            .find(|(name, _)| name == "enabled")
            .map(|(_, default)| default.as_str()),
        Some("true")
    );
    assert_eq!(
        policy_defaults
            .iter()
            .find(|(name, _)| name == "mode")
            .map(|(_, default)| default.as_str()),
        Some("'confirm'::text")
    );
    assert_eq!(
        policy_defaults
            .iter()
            .find(|(name, _)| name == "max_risk")
            .map(|(_, default)| default.as_str()),
        Some("'medium'::text")
    );
    assert_eq!(
        policy_defaults
            .iter()
            .find(|(name, _)| name == "params")
            .map(|(_, default)| default.as_str()),
        Some("'{}'::jsonb")
    );
    assert_column_default_contains(&policy_defaults, "updated_at", "now()");

    let run_defaults = column_defaults(&pool, "smart_action_runs").await;
    assert_column_default_contains(&run_defaults, "created_at", "now()");
    assert_column_default_contains(&run_defaults, "updated_at", "now()");
    assert_column_default_contains(&run_defaults, "source", "smart_action_runs");
    assert_column_default_contains(&run_defaults, "tab", "smart-actions");
    assert_column_default_contains(&run_defaults, "action_label", "查看详情");
}

async fn connect_and_migrate(database_url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
        .expect("connect smart actions test database");
    db::migrate(&pool)
        .await
        .expect("run smart actions test migrations");
    pool
}

async fn reset_and_seed_db(pool: &PgPool) {
    sqlx::query(
        "TRUNCATE sessions, auth_users, smart_action_runs, smart_action_policies,
                  task_runs, schedule_jobs, app_logs, catalog_items, app_settings,
                  autostrm_seen, autostrm_unmatched
         RESTART IDENTITY CASCADE",
    )
    .execute(pool)
    .await
    .expect("reset smart actions test tables");

    sqlx::query(
        "INSERT INTO task_runs(id, kind, label, status, status_text, error, updated_at)
         VALUES ($1, 'cleanup', 'failed cleanup', 'error', 'failed', 'boom', now())",
    )
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed failed task_run");

    for (key, value) in [
        ("emby_url", json!("http://127.0.0.1:9")),
        ("api_key", json!("smart-actions-test-key")),
        ("tmdb_api_key", json!("")),
        ("tmdb_base_url", json!("")),
    ] {
        sqlx::query(
            "INSERT INTO app_settings(key, value, updated_at) VALUES ($1, $2, now())
             ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .expect("save optional smart actions config");
    }
}

async fn create_test_user(pool: &PgPool) -> String {
    let username = format!("smart_actions_{}", Uuid::new_v4().simple());
    let hash = auth::hash_argon2("secret").unwrap();
    sqlx::query(
        "INSERT INTO auth_users(id, username, password_hash, legacy_hash)
         VALUES ($1, $2, $3, FALSE)",
    )
    .bind(Uuid::new_v4())
    .bind(&username)
    .bind(hash)
    .execute(pool)
    .await
    .expect("insert smart actions test user");
    username
}

async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, String)],
) -> (StatusCode, HeaderMap, Value) {
    let body = body.map(|value| value.to_string()).unwrap_or_default();
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if !body.is_empty() {
        builder = builder.header(CONTENT_TYPE, "application/json");
    }
    for (name, value) in headers {
        builder = builder.header(*name, value.as_str());
    }

    let response = app
        .clone()
        .oneshot(
            builder
                .body(Body::from(body))
                .expect("build smart actions request"),
        )
        .await
        .expect("send smart actions request");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read smart actions response body");
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
    };
    (status, headers, body)
}

fn request_cookie(headers: &HeaderMap) -> String {
    headers
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .expect("login response set-cookie")
        .to_string()
}

async fn table_count(pool: &PgPool, table: &str) -> i64 {
    let sql = match table {
        "task_runs" => "SELECT COUNT(*) FROM task_runs",
        "smart_action_runs" => "SELECT COUNT(*) FROM smart_action_runs",
        _ => panic!("unexpected table name: {table}"),
    };
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .expect("count table rows")
}

async fn column_defaults(pool: &PgPool, table: &str) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT column_name, column_default
         FROM information_schema.columns
         WHERE table_schema = 'public'
           AND table_name = $1
           AND column_default IS NOT NULL",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .expect("load column defaults")
}

fn assert_column_default_contains(defaults: &[(String, String)], column: &str, needle: &str) {
    let default = defaults
        .iter()
        .find(|(name, _)| name == column)
        .map(|(_, default)| default.as_str())
        .unwrap_or_else(|| panic!("{column} should have a default"));
    assert!(
        default.contains(needle),
        "{column} default should contain {needle}, got {default}"
    );
}

fn smart_actions_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_SMART_ACTIONS_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn test_settings(database_url: &str, base: &Path) -> Settings {
    Settings {
        host: "127.0.0.1".to_string(),
        port: 0,
        database_url: database_url.to_string(),
        web_dist: base.to_path_buf(),
        legacy_dir: base.to_path_buf(),
        bootstrap_password: "a-strong-preview-password".to_string(),
        cd_root: base.join("cd"),
        strm_root: PathBuf::from("/path/that/does/not/exist"),
        docker_bin: PathBuf::from("/usr/bin/docker"),
        task_concurrency: 1,
    }
}
