use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        HeaderMap, Method, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
};
use emby_manager::{api, auth, db, settings::Settings};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, path::PathBuf};
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

static TEST_DB_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn config_export_masks_sensitive_settings() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(ctx) = test_app().await else {
        eprintln!(
            "skipping config API DB test; set EMBY_MANAGER_CONFIG_TEST_DATABASE_URL to enable it"
        );
        return;
    };
    seed_settings(
        &ctx.pool,
        vec![
            ("emby_url", json!("http://emby.local:8096/emby")),
            ("api_key", json!("real-api-key")),
            ("c115_cookie", json!("UID=real; SEID=real")),
            ("cd2_webhook_secret", json!("real-webhook-secret")),
            ("c115_cid_map", json!({"电影": "12345"})),
            (
                "custom_object",
                json!({"password": "nested-secret", "plain": "ok"}),
            ),
        ],
    )
    .await;

    let (status, _, body) = ctx
        .send_auth(Method::GET, "/api/v2/config/export", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["settings"]["emby_url"],
        json!("http://emby.local:8096/emby")
    );
    assert_eq!(body["settings"]["api_key"], json!("***"));
    assert_eq!(body["settings"]["c115_cookie"], json!("***"));
    assert_eq!(body["settings"]["cd2_webhook_secret"], json!("***"));
    assert_eq!(body["settings"]["c115_cid_map"]["电影"], json!("12345"));
    assert_eq!(body["settings"]["custom_object"]["password"], json!("***"));
    assert_eq!(body["settings"]["custom_object"]["plain"], json!("ok"));
}

#[tokio::test]
async fn config_import_dry_run_reports_without_writing() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(ctx) = test_app().await else {
        eprintln!(
            "skipping config API DB test; set EMBY_MANAGER_CONFIG_TEST_DATABASE_URL to enable it"
        );
        return;
    };
    seed_settings(
        &ctx.pool,
        vec![
            ("emby_url", json!("http://old.local:8096/emby")),
            ("api_key", json!("original-api-key")),
        ],
    )
    .await;

    let (status, _, body) = ctx
        .send_auth(
            Method::POST,
            "/api/v2/config/import",
            Some(json!({
                "dry_run": true,
                "settings": {
                    "emby_url": "https://new.local:8096/emby/",
                    "api_key": "***",
                    "auto_strm_debounce_sec": "12",
                    "database_url": "postgres://evil",
                    "random_key": 42
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["dry_run"], json!(true));
    assert_contains_string(&body["accepted"], "emby_url");
    assert_contains_string(&body["accepted"], "api_key");
    assert_contains_string(&body["accepted"], "auto_strm_debounce_sec");
    assert_eq!(rejection_reason(&body, "database_url"), Some("protected"));
    assert_eq!(rejection_reason(&body, "random_key"), Some("unknown"));
    assert!(body["applied"].as_array().unwrap().is_empty(), "{body}");
    assert!(
        warnings_contain(&body, "api_key"),
        "masked api_key should be reported as preserved: {body}"
    );

    assert_eq!(
        setting_value(&ctx.pool, "emby_url").await,
        Some(json!("http://old.local:8096/emby"))
    );
    assert_eq!(
        setting_value(&ctx.pool, "auto_strm_debounce_sec").await,
        None
    );
}

#[tokio::test]
async fn config_import_apply_writes_allowed_and_preserves_masked_sensitive_values() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(ctx) = test_app().await else {
        eprintln!(
            "skipping config API DB test; set EMBY_MANAGER_CONFIG_TEST_DATABASE_URL to enable it"
        );
        return;
    };
    seed_settings(
        &ctx.pool,
        vec![
            ("api_key", json!("original-api-key")),
            ("c115_cookie", json!("UID=old; SEID=old")),
        ],
    )
    .await;

    let (status, _, body) = ctx
        .send_auth(
            Method::POST,
            "/api/v2/config/import",
            Some(json!({
                "mode": "apply",
                "settings": {
                    "emby_url": "https://new.local:8096/emby/",
                    "api_key": "***",
                    "c115_cookie": "UID=new; SEID=new",
                    "auto_strm_enabled": true,
                    "auto_strm_debounce_sec": "12",
                    "cd2_mount_prefix": "/CloudNAS/CloudDrive/",
                    "c115_cid_map": {"电影": "12345"}
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["dry_run"], json!(false));
    assert_contains_string(&body["accepted"], "api_key");
    assert_contains_string(&body["applied"], "emby_url");
    assert_contains_string(&body["applied"], "c115_cookie");
    assert_contains_string(&body["applied"], "auto_strm_enabled");
    assert_contains_string(&body["applied"], "auto_strm_debounce_sec");
    assert_contains_string(&body["applied"], "cd2_mount_prefix");
    assert_contains_string(&body["applied"], "c115_cid_map");
    assert_not_contains_string(&body["applied"], "api_key");

    assert_eq!(
        setting_value(&ctx.pool, "emby_url").await,
        Some(json!("https://new.local:8096/emby"))
    );
    assert_eq!(
        setting_value(&ctx.pool, "api_key").await,
        Some(json!("original-api-key"))
    );
    assert_eq!(
        setting_value(&ctx.pool, "c115_cookie").await,
        Some(json!("UID=new; SEID=new"))
    );
    assert_eq!(
        setting_value(&ctx.pool, "auto_strm_enabled").await,
        Some(json!(true))
    );
    assert_eq!(
        setting_value(&ctx.pool, "auto_strm_debounce_sec").await,
        Some(json!(12))
    );
    assert_eq!(
        setting_value(&ctx.pool, "cd2_mount_prefix").await,
        Some(json!("/CloudNAS/CloudDrive"))
    );
    assert_eq!(
        setting_value(&ctx.pool, "c115_cid_map").await,
        Some(json!({"电影": "12345"}))
    );
}

struct TestApp {
    app: Router,
    pool: PgPool,
    token: String,
}

impl TestApp {
    async fn send_auth(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        send(
            &self.app,
            method,
            uri,
            body,
            &[(AUTHORIZATION.as_str(), format!("Bearer {}", self.token))],
        )
        .await
    }
}

async fn test_app() -> Option<TestApp> {
    let database_url = config_test_database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect config API test database");
    db::migrate(&pool).await.expect("run config API migrations");
    sqlx::query(
        "TRUNCATE sessions, auth_login_attempts, auth_users, app_settings RESTART IDENTITY CASCADE",
    )
    .execute(&pool)
    .await
    .expect("reset config API test tables");

    let username = create_test_user(&pool).await;
    let app = api::router(pool.clone(), test_settings(database_url));
    let (status, _, body) = send(
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
    let token = body["token"].as_str().expect("login token").to_string();

    Some(TestApp { app, pool, token })
}

async fn create_test_user(pool: &PgPool) -> String {
    let username = format!("config_{}", Uuid::new_v4().simple());
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
    .expect("insert config API test user");
    username
}

async fn seed_settings(pool: &PgPool, rows: Vec<(&str, Value)>) {
    for (key, value) in rows {
        sqlx::query(
            "INSERT INTO app_settings(key, value, updated_at) VALUES ($1, $2, now())
             ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .expect("seed app setting");
    }
}

async fn setting_value(pool: &PgPool, key: &str) -> Option<Value> {
    sqlx::query_scalar("SELECT value FROM app_settings WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .expect("read app setting")
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
                .expect("build config API request"),
        )
        .await
        .expect("send config API request");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read config API response body");
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
    };
    (status, headers, body)
}

fn config_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_CONFIG_TEST_DATABASE_URL") {
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
        bootstrap_password: "a-strong-preview-password".to_string(),
        cd_root: PathBuf::from("/tmp/cd"),
        strm_root: PathBuf::from("/tmp/strm"),
        docker_bin: PathBuf::from("/usr/bin/docker"),
        task_concurrency: 1,
    }
}

fn assert_contains_string(value: &Value, expected: &str) {
    assert!(
        value
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(expected))),
        "expected {value:?} to contain {expected:?}"
    );
}

fn assert_not_contains_string(value: &Value, expected: &str) {
    assert!(
        value
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item.as_str() != Some(expected))),
        "expected {value:?} not to contain {expected:?}"
    );
}

fn rejection_reason<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    body["rejected"]
        .as_array()?
        .iter()
        .find(|item| item["key"].as_str() == Some(key))?
        .get("reason")?
        .as_str()
}

fn warnings_contain(body: &Value, needle: &str) -> bool {
    body["warnings"].as_array().is_some_and(|warnings| {
        warnings
            .iter()
            .any(|warning| warning.as_str().is_some_and(|item| item.contains(needle)))
    })
}
