use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        HeaderMap, Method, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, SET_COOKIE},
    },
};
use emby_manager::{api, auth, db, settings::Settings};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, path::PathBuf};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn protected_api_rejects_missing_session_without_database() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://emby_manager:emby_manager@127.0.0.1:1/emby_manager_missing")
        .unwrap();
    let app = api::router(pool, test_settings());

    let (status, _, body) = send(&app, Method::GET, "/api/v2/tasks", None, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["code"], "unauthorized");

    let (status, _, body) = send(&app, Method::POST, "/api/v2/auth/logout", None, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, _, body) = send(&app, Method::GET, "/api/v2/auth/me", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["authenticated"], false);

    let (status, _, body) = send(&app, Method::GET, "/api/v2/openapi.json", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn cookie_sessions_require_csrf_but_bearer_mutations_do_not() {
    let Some(database_url) = auth_test_database_url() else {
        eprintln!(
            "skipping auth DB security test; set EMBY_MANAGER_AUTH_TEST_DATABASE_URL to enable it"
        );
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect auth security test database");
    db::migrate(&pool)
        .await
        .expect("run auth security migrations");

    let username = create_test_user(&pool).await;
    let app = api::router(pool, test_settings());

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
    let token = body["token"].as_str().unwrap().to_string();
    let csrf = body["csrf"].as_str().unwrap().to_string();
    let cookie = request_cookie(&headers);

    let (status, _, body) = send(
        &app,
        Method::GET,
        "/api/v2/tasks",
        None,
        &[(COOKIE.as_str(), cookie.clone())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _, body) = send(
        &app,
        Method::POST,
        "/api/v2/catalog/transfer-plan",
        Some(json!({"link": "magnet:?xt=urn:btih:abcdef", "cid": "123"})),
        &[(COOKIE.as_str(), cookie.clone())],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "csrf_required");

    let (status, _, body) = send(
        &app,
        Method::POST,
        "/api/v2/catalog/transfer-plan",
        Some(json!({"link": "magnet:?xt=urn:btih:abcdef", "cid": "123"})),
        &[
            (COOKIE.as_str(), cookie.clone()),
            ("x-csrf-token", csrf.clone()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _, body) = send(
        &app,
        Method::POST,
        "/api/v2/catalog/transfer-plan",
        Some(json!({"link": "magnet:?xt=urn:btih:abcdef", "cid": "123"})),
        &[(AUTHORIZATION.as_str(), format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, headers, body) = send(
        &app,
        Method::POST,
        "/api/v2/auth/logout",
        None,
        &[(COOKIE.as_str(), cookie.clone()), ("x-csrf-token", csrf)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let cleared = headers
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(cleared.contains("max-age=0"), "{cleared}");

    let (status, _, body) = send(
        &app,
        Method::GET,
        "/api/v2/tasks",
        None,
        &[(COOKIE.as_str(), cookie)],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn login_rate_limits_repeated_password_failures() {
    let Some(database_url) = auth_test_database_url() else {
        eprintln!(
            "skipping auth DB security test; set EMBY_MANAGER_AUTH_TEST_DATABASE_URL to enable it"
        );
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect auth security test database");
    db::migrate(&pool)
        .await
        .expect("run auth security migrations");

    let username = create_test_user(&pool).await;
    let app = api::router(pool, test_settings());
    for _ in 0..5 {
        let (status, _, body) = send(
            &app,
            Method::POST,
            "/api/v2/auth/login",
            Some(json!({
                "username": username,
                "password": "wrong"
            })),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    }

    let (status, _, body) = send(
        &app,
        Method::POST,
        "/api/v2/auth/login",
        Some(json!({
            "username": username,
            "password": "wrong"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["code"], "rate_limited");
}

#[tokio::test]
async fn authenticated_user_can_change_password_and_expire_other_sessions() {
    let Some(database_url) = auth_test_database_url() else {
        eprintln!(
            "skipping auth DB security test; set EMBY_MANAGER_AUTH_TEST_DATABASE_URL to enable it"
        );
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect auth security test database");
    db::migrate(&pool)
        .await
        .expect("run auth security migrations");

    let username = create_test_user(&pool).await;
    let app = api::router(pool.clone(), test_settings());

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
    let csrf = body["csrf"].as_str().unwrap().to_string();
    let cookie = request_cookie(&headers);

    let (status, second_headers, body) = send(
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
    let second_cookie = request_cookie(&second_headers);

    let (status, _, body) = send(
        &app,
        Method::POST,
        "/api/v2/auth/password",
        Some(json!({
            "current_password": "secret",
            "new_password": "newSecret123!"
        })),
        &[(COOKIE.as_str(), cookie.clone())],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "csrf_required");

    let (status, _, body) = send(
        &app,
        Method::POST,
        "/api/v2/auth/password",
        Some(json!({
            "current_password": "wrong",
            "new_password": "newSecret123!"
        })),
        &[
            (COOKIE.as_str(), cookie.clone()),
            ("x-csrf-token", csrf.clone()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, _, body) = send(
        &app,
        Method::POST,
        "/api/v2/auth/password",
        Some(json!({
            "current_password": "secret",
            "new_password": "short"
        })),
        &[
            (COOKIE.as_str(), cookie.clone()),
            ("x-csrf-token", csrf.clone()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, _, body) = send(
        &app,
        Method::POST,
        "/api/v2/auth/password",
        Some(json!({
            "current_password": "secret",
            "new_password": "newSecret123!"
        })),
        &[(COOKIE.as_str(), cookie.clone()), ("x-csrf-token", csrf)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["invalidated_sessions"], 1);

    let (status, _, body) = send(
        &app,
        Method::GET,
        "/api/v2/tasks",
        None,
        &[(COOKIE.as_str(), second_cookie)],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

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
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, _, body) = send(
        &app,
        Method::POST,
        "/api/v2/auth/login",
        Some(json!({
            "username": username,
            "password": "newSecret123!"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let stored: (String, bool) =
        sqlx::query_as("SELECT password_hash, legacy_hash FROM auth_users WHERE username = $1")
            .bind(&username)
            .fetch_one(&pool)
            .await
            .expect("fetch updated auth user");
    assert!(auth::verify_password("newSecret123!", &stored.0));
    assert!(!stored.1);
}

async fn create_test_user(pool: &PgPool) -> String {
    let username = format!("security_{}", Uuid::new_v4().simple());
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
    .expect("insert auth security test user");
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
                .expect("build auth security request"),
        )
        .await
        .expect("send auth security request");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read auth security response body");
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

fn auth_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_AUTH_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn test_settings() -> Settings {
    Settings {
        host: "127.0.0.1".to_string(),
        port: 0,
        database_url: "postgres://emby_manager:emby_manager@127.0.0.1:1/emby_manager_missing"
            .to_string(),
        web_dist: PathBuf::from("/tmp"),
        legacy_dir: PathBuf::from("/tmp"),
        bootstrap_password: "admin".to_string(),
        cd_root: PathBuf::from("/tmp/cd"),
        strm_root: PathBuf::from("/tmp/strm"),
        docker_bin: PathBuf::from("/usr/bin/docker"),
        task_concurrency: 1,
    }
}
