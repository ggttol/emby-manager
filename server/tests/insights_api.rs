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

#[tokio::test]
async fn insights_endpoints_return_readonly_coverage_and_todos() {
    let Some(database_url) = insights_test_database_url() else {
        eprintln!(
            "skipping insights API test; set EMBY_MANAGER_INSIGHTS_TEST_DATABASE_URL to enable it"
        );
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let strm_root = seed_strm_tree(tmp.path());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect insights test database");
    db::migrate(&pool)
        .await
        .expect("run insights test migrations");
    reset_and_seed_db(&pool).await;

    let username = create_test_user(&pool).await;
    let app = api::router(
        pool.clone(),
        test_settings(&database_url, tmp.path(), strm_root),
    );

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
    let csrf = body["csrf"].as_str().unwrap().to_string();

    let (status, _, gaps) = send(
        &app,
        Method::POST,
        "/api/v2/gaps/scan",
        Some(json!({})),
        &[
            (COOKIE.as_str(), cookie.clone()),
            ("x-csrf-token", csrf.clone()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{gaps}");
    assert_eq!(gaps["complete_business_port"], false);
    assert_eq!(gaps["meta"]["readonly"], true);
    assert!(
        json_array_contains(&gaps["meta"]["source"], "task_runs"),
        "{gaps}"
    );
    assert_eq!(gaps["strm"]["strm_files"], 1);
    assert_eq!(gaps["autostrm"]["unmatched"]["total"], 1);
    assert!(
        gaps["todos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|todo| todo["area"] == "autostrm")
    );

    let (status, _, cleanup) = send(
        &app,
        Method::POST,
        "/api/v2/cleanup/suggest",
        Some(json!({})),
        &[(COOKIE.as_str(), cookie.clone()), ("x-csrf-token", csrf)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleanup}");
    assert_eq!(cleanup["complete_business_port"], false);
    assert_eq!(cleanup["catalog"]["duplicate_links"], 1);
    assert_eq!(cleanup["logs"]["errors_7d"], 1);
    assert_eq!(cleanup["strm"]["empty_directory_samples"][0], "Empty");
    assert_eq!(
        cleanup["strm"]["other_file_samples"][0],
        "Shows/Season 1/poster.jpg"
    );
    assert!(
        cleanup["todos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|todo| todo["area"] == "tasks" && todo["severity"] == "high"),
        "{cleanup}"
    );

    let (status, _, autostrm) = send(
        &app,
        Method::GET,
        "/api/v2/autostrm/status",
        None,
        &[(COOKIE.as_str(), cookie)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{autostrm}");
    assert_eq!(autostrm["complete_business_port"], false);
    assert_eq!(autostrm["seen"]["total"], 1);
    assert_eq!(autostrm["unmatched"]["without_emby_id"], 1);
    assert_eq!(autostrm["libraries"][0]["lib"], "Shows");
}

fn seed_strm_tree(base: &Path) -> PathBuf {
    let strm_root = base.join("strm");
    let season = strm_root.join("Shows").join("Season 1");
    let empty = strm_root.join("Empty");
    std::fs::create_dir_all(&season).unwrap();
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(season.join("E01.strm"), "http://example.invalid/E01.mkv").unwrap();
    std::fs::write(season.join("E01.ass"), "[Script Info]").unwrap();
    std::fs::write(season.join("poster.jpg"), "image").unwrap();
    strm_root
}

async fn reset_and_seed_db(pool: &PgPool) {
    sqlx::query(
        "TRUNCATE sessions, auth_users, task_runs, schedule_jobs, app_logs, catalog_items,
                  autostrm_seen, autostrm_unmatched
         RESTART IDENTITY CASCADE",
    )
    .execute(pool)
    .await
    .expect("reset insights test tables");

    sqlx::query(
        "INSERT INTO task_runs(id, kind, label, status, status_text, error, updated_at)
         VALUES
            ($1, 'cleanup', 'stale cleanup', 'running', 'still running', NULL, now() - interval '45 minutes'),
            ($2, 'gaps', 'gap scan', 'error', 'failed', 'boom', now() - interval '5 minutes'),
            ($3, 'catalog', 'import', 'done', 'done', NULL, now())",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed task_runs");

    sqlx::query(
        "INSERT INTO catalog_items(name, sheet, link, is_pkg, link_type)
         VALUES
            ('Show A', 's1', 'https://115.com/s/abc?password=1234', false, 'share115'),
            ('Show A', 's1', 'https://115.com/s/abc?password=1234', false, 'share115'),
            ('Movie B', 's2', 'magnet:?xt=urn:btih:abcdef', false, 'magnet')",
    )
    .execute(pool)
    .await
    .expect("seed catalog_items");

    sqlx::query(
        "INSERT INTO autostrm_seen(lib, top, mtime)
         VALUES ('Shows', 'Show A', 100)
         ON CONFLICT (lib, top) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed autostrm_seen");

    sqlx::query(
        "INSERT INTO autostrm_unmatched(lib, top, emby_id, name)
         VALUES ('Shows', 'Show Missing', NULL, 'Show Missing')
         ON CONFLICT (lib, top) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed autostrm_unmatched");

    sqlx::query(
        "INSERT INTO schedule_jobs(id, name, kind, schedule, enabled, last_status, last_error)
         VALUES ($1, 'nightly cleanup', 'cleanup', '{}'::jsonb, true, 'error', 'failed')",
    )
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed schedule_jobs");

    sqlx::query(
        "INSERT INTO app_logs(level, message)
         VALUES ('error', 'cleanup failed'), ('warn', 'check this')",
    )
    .execute(pool)
    .await
    .expect("seed app_logs");
}

async fn create_test_user(pool: &PgPool) -> String {
    let username = format!("insights_{}", Uuid::new_v4().simple());
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
    .expect("insert insights test user");
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
                .expect("build insights request"),
        )
        .await
        .expect("send insights request");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read insights response body");
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

fn json_array_contains(value: &Value, needle: &str) -> bool {
    value
        .as_array()
        .map(|items| items.iter().any(|item| item == needle))
        .unwrap_or(false)
}

fn insights_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_INSIGHTS_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn test_settings(database_url: &str, base: &Path, strm_root: PathBuf) -> Settings {
    Settings {
        host: "127.0.0.1".to_string(),
        port: 0,
        database_url: database_url.to_string(),
        web_dist: base.to_path_buf(),
        legacy_dir: base.to_path_buf(),
        bootstrap_password: "a-strong-preview-password".to_string(),
        cd_root: base.join("cd"),
        strm_root,
        docker_bin: PathBuf::from("/usr/bin/docker"),
        task_concurrency: 1,
    }
}
