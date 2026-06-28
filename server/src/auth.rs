use crate::{
    error::{AppError, AppResult},
    settings::Settings,
    state::AppState,
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::PgPool;
use uuid::Uuid;

const LOGIN_WINDOW_SQL: &str = "5 minutes";
const LOGIN_MAX_FAILURES: i64 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthSource {
    Bearer,
    Cookie,
}

#[derive(Clone, Debug)]
pub struct AuthSession {
    pub token: String,
    pub user_id: Uuid,
    pub username: String,
    pub csrf: String,
    pub source: AuthSource,
}

#[derive(Clone, Debug)]
struct AuthCredentials {
    token: String,
    source: AuthSource,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    pub ok: bool,
    pub token: String,
    pub csrf: String,
    pub username: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MeResponse {
    pub authenticated: bool,
    pub username: Option<String>,
    pub csrf: Option<String>,
}

pub fn router() -> Router<AppState> {
    public_router().merge(protected_router())
}

pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/auth/login", post(login))
        .route("/api/v2/auth/me", get(me))
}

pub fn protected_router() -> Router<AppState> {
    Router::new().route("/api/v2/auth/logout", post(logout))
}

pub async fn ensure_default_admin(pool: &PgPool, settings: &Settings) -> anyhow::Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM auth_users")
        .fetch_one(pool)
        .await?;
    if count == 0 {
        let hash = hash_argon2(&settings.bootstrap_password)?;
        sqlx::query(
            "INSERT INTO auth_users(id, username, password_hash, legacy_hash) VALUES ($1, $2, $3, FALSE)",
        )
        .bind(Uuid::new_v4())
        .bind("admin")
        .bind(hash)
        .execute(pool)
        .await?;
        tracing::warn!(
            "created bootstrap admin user; change EMBY_MANAGER_BOOTSTRAP_PASSWORD before production use"
        );
    }
    Ok(())
}

#[utoipa::path(post, path = "/api/v2/auth/login", tag = "auth", request_body = LoginRequest, responses((status = 200, body = LoginResponse)))]
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> AppResult<impl IntoResponse> {
    let username = req.username.trim().to_string();
    let buckets = login_rate_buckets(&username, &headers);
    prune_login_attempts(&state.pool).await?;
    ensure_login_allowed(&state.pool, &buckets).await?;
    let row: Option<(Uuid, String, bool)> =
        sqlx::query_as("SELECT id, password_hash, legacy_hash FROM auth_users WHERE username = $1")
            .bind(&username)
            .fetch_optional(&state.pool)
            .await?;
    let Some((user_id, stored, legacy_hash)) = row else {
        record_login_failure(&state.pool, &buckets).await?;
        return Err(AppError::Unauthorized("用户名或密码错误".to_string()));
    };
    if !verify_password(&req.password, &stored) {
        record_login_failure(&state.pool, &buckets).await?;
        return Err(AppError::Unauthorized("用户名或密码错误".to_string()));
    }
    clear_login_failures(&state.pool, &buckets).await?;
    if legacy_hash {
        let upgraded = hash_argon2(&req.password)?;
        sqlx::query("UPDATE auth_users SET password_hash = $1, legacy_hash = FALSE, updated_at = now() WHERE id = $2")
            .bind(upgraded)
            .bind(user_id)
            .execute(&state.pool)
            .await?;
    }
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let csrf = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let expires_at = Utc::now() + Duration::days(30);
    sqlx::query(
        "INSERT INTO sessions(token, user_id, csrf, ip, expires_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&token)
    .bind(user_id)
    .bind(&csrf)
    .bind("")
    .bind(expires_at)
    .execute(&state.pool)
    .await?;
    let cookie = format!(
        "emby_mgr_rs={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        token,
        30 * 24 * 3600
    );
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(LoginResponse {
            ok: true,
            token,
            csrf,
            username,
        }),
    ))
}

fn login_rate_buckets(username: &str, headers: &HeaderMap) -> Vec<String> {
    let normalized_username = username
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut buckets = vec![format!("user:{normalized_username}")];
    if let Some(ip) = best_effort_client_ip(headers) {
        buckets.push(format!("ip:{ip}"));
    }
    buckets
}

fn best_effort_client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-real-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(96).collect())
}

async fn prune_login_attempts(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM auth_login_attempts
         WHERE failed_at < now() - ($1::text)::interval",
    )
    .bind(LOGIN_WINDOW_SQL)
    .execute(pool)
    .await?;
    Ok(())
}

async fn ensure_login_allowed(pool: &PgPool, buckets: &[String]) -> AppResult<()> {
    for bucket in buckets {
        let failures: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM auth_login_attempts
             WHERE bucket = $1 AND failed_at >= now() - ($2::text)::interval",
        )
        .bind(bucket)
        .bind(LOGIN_WINDOW_SQL)
        .fetch_one(pool)
        .await?;
        if failures >= LOGIN_MAX_FAILURES {
            return Err(AppError::RateLimited(
                "登录失败过多，请稍后再试".to_string(),
            ));
        }
    }
    Ok(())
}

async fn record_login_failure(pool: &PgPool, buckets: &[String]) -> Result<(), sqlx::Error> {
    for bucket in buckets {
        sqlx::query("INSERT INTO auth_login_attempts(bucket) VALUES ($1)")
            .bind(bucket)
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn clear_login_failures(pool: &PgPool, buckets: &[String]) -> Result<(), sqlx::Error> {
    for bucket in buckets {
        sqlx::query("DELETE FROM auth_login_attempts WHERE bucket = $1")
            .bind(bucket)
            .execute(pool)
            .await?;
    }
    Ok(())
}

#[utoipa::path(post, path = "/api/v2/auth/logout", tag = "auth", responses((status = 200)))]
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    if let Some(credentials) = credentials_from_headers(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token = $1")
            .bind(credentials.token)
            .execute(&state.pool)
            .await?;
    }
    Ok((
        [(
            header::SET_COOKIE,
            "emby_mgr_rs=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        )],
        Json(serde_json::json!({"ok": true})),
    ))
}

#[utoipa::path(get, path = "/api/v2/auth/me", tag = "auth", responses((status = 200, body = MeResponse)))]
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Json<MeResponse>> {
    let Some(credentials) = credentials_from_headers(&headers) else {
        return Ok(Json(MeResponse {
            authenticated: false,
            username: None,
            csrf: None,
        }));
    };
    if let Some(session) = load_session(&state.pool, credentials).await? {
        touch_session(&state.pool, &session.token).await?;
        Ok(Json(MeResponse {
            authenticated: true,
            username: Some(session.username),
            csrf: Some(session.csrf),
        }))
    } else {
        Ok(Json(MeResponse {
            authenticated: false,
            username: None,
            csrf: None,
        }))
    }
}

pub async fn require_api_auth(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    if is_public_request(req.method(), req.uri().path()) {
        return next.run(req).await;
    }
    if !req.uri().path().starts_with("/api/v2/") {
        return next.run(req).await;
    }

    match authenticate(&state.pool, req.method(), req.headers()).await {
        Ok(session) => {
            req.extensions_mut().insert(session);
            next.run(req).await
        }
        Err(failure) => failure.into_response(),
    }
}

async fn authenticate(
    pool: &PgPool,
    method: &Method,
    headers: &HeaderMap,
) -> Result<AuthSession, AuthFailure> {
    let credentials =
        credentials_from_headers(headers).ok_or(AuthFailure::Unauthorized("未登录或会话已过期"))?;
    let session = load_session(pool, credentials)
        .await
        .map_err(|_| AuthFailure::Internal("认证服务暂不可用"))?
        .ok_or(AuthFailure::Unauthorized("未登录或会话已过期"))?;

    if session.source == AuthSource::Cookie && is_mutating_method(method) {
        let header_csrf = headers
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if header_csrf.is_empty() || header_csrf != session.csrf {
            return Err(AuthFailure::Forbidden("CSRF token 缺失或不匹配"));
        }
    }

    touch_session(pool, &session.token)
        .await
        .map_err(|_| AuthFailure::Internal("认证服务暂不可用"))?;
    Ok(session)
}

async fn load_session(
    pool: &PgPool,
    credentials: AuthCredentials,
) -> Result<Option<AuthSession>, sqlx::Error> {
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT u.id, u.username, s.csrf
         FROM sessions s
         JOIN auth_users u ON u.id = s.user_id
         WHERE s.token = $1 AND s.expires_at > now()",
    )
    .bind(&credentials.token)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(user_id, username, csrf)| AuthSession {
        token: credentials.token,
        user_id,
        username,
        csrf,
        source: credentials.source,
    }))
}

async fn touch_session(pool: &PgPool, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE sessions SET last_seen_at = now() WHERE token = $1")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(())
}

fn credentials_from_headers(headers: &HeaderMap) -> Option<AuthCredentials> {
    if let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(t) = v.strip_prefix("Bearer ") {
            return Some(AuthCredentials {
                token: t.trim().to_string(),
                source: AuthSource::Bearer,
            });
        }
    }
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie.split(';').find_map(|part| {
        let (k, v) = part.trim().split_once('=')?;
        (k == "emby_mgr_rs").then(|| AuthCredentials {
            token: v.to_string(),
            source: AuthSource::Cookie,
        })
    })
}

fn is_public_request(method: &Method, path: &str) -> bool {
    matches!(
        (method, path),
        (&Method::GET, "/health")
            | (&Method::GET, "/api/v2/openapi.json")
            | (&Method::POST, "/api/v2/auth/login")
            | (&Method::GET, "/api/v2/auth/me")
            | (&Method::POST, "/api/v2/autostrm/webhook")
    )
}

fn is_mutating_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

#[derive(Debug)]
enum AuthFailure {
    Unauthorized(&'static str),
    Forbidden(&'static str),
    Internal(&'static str),
}

impl AuthFailure {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            AuthFailure::Unauthorized(message) => {
                (StatusCode::UNAUTHORIZED, "unauthorized", message)
            }
            AuthFailure::Forbidden(message) => (StatusCode::FORBIDDEN, "csrf_required", message),
            AuthFailure::Internal(message) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
            }
        };
        (
            status,
            Json(crate::error::ErrorBody {
                err: message.to_string(),
                code: code.to_string(),
            }),
        )
            .into_response()
    }
}

pub fn hash_argon2(plain: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .to_string())
}

pub fn verify_password(plain: &str, stored: &str) -> bool {
    if plain.is_empty() || stored.is_empty() {
        return false;
    }
    if stored.starts_with("pbkdf2_sha256$") {
        return verify_legacy_pbkdf2(plain, stored);
    }
    let Ok(parsed) = PasswordHash::new(stored) else {
        return false;
    };
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok()
}

fn verify_legacy_pbkdf2(plain: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() != 4 || parts[0] != "pbkdf2_sha256" {
        return false;
    }
    let Ok(iters) = parts[1].parse::<u32>() else {
        return false;
    };
    let Ok(salt) = hex::decode(parts[2]) else {
        return false;
    };
    let Ok(expected) = hex::decode(parts[3]) else {
        return false;
    };
    let actual = pbkdf2::pbkdf2_hmac_array::<Sha256, 32>(plain.as_bytes(), &salt, iters);
    expected.len() == actual.len()
        && expected
            .iter()
            .zip(actual)
            .fold(0_u8, |diff, (left, right)| diff | (*left ^ right))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_legacy_python_pbkdf2_hash() {
        let stored = "pbkdf2_sha256$200000$000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f$2f9a5868926f81696e0202fe00533133713115e866c99c0b62def7943d448a2c";
        assert!(verify_password("secret", stored));
        assert!(!verify_password("wrong", stored));
    }
}
