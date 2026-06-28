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
    extract::{ConnectInfo, Extension, FromRequestParts, State},
    http::{HeaderMap, Method, Request, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use sqlx::PgPool;
use std::{
    convert::Infallible,
    net::{IpAddr, SocketAddr},
};
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ChangePasswordResponse {
    pub ok: bool,
    pub invalidated_sessions: u64,
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
    Router::new()
        .route("/api/v2/auth/logout", post(logout))
        .route("/api/v2/auth/password", post(change_password))
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
pub(crate) async fn login(
    State(state): State<AppState>,
    PeerIp(peer_ip): PeerIp,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> AppResult<impl IntoResponse> {
    let username = req.username.trim().to_string();
    let trusted_proxies = trusted_proxy_ips(&state.pool).await?;
    let client_ip = client_ip_for_request(&headers, peer_ip, &trusted_proxies);
    let buckets = login_rate_buckets(&username, client_ip);
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
    .bind(client_ip.map(|ip| ip.to_string()).unwrap_or_default())
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct PeerIp(Option<IpAddr>);

impl<S> FromRequestParts<S> for PeerIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip()),
        ))
    }
}

fn login_rate_buckets(username: &str, client_ip: Option<IpAddr>) -> Vec<String> {
    let normalized_username = username
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut buckets = vec![format!("user:{normalized_username}")];
    if let Some(ip) = client_ip {
        buckets.push(format!("ip:{ip}"));
    }
    buckets
}

async fn trusted_proxy_ips(pool: &PgPool) -> Result<Vec<IpAddr>, sqlx::Error> {
    let value: Option<Value> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'trusted_proxies'")
            .fetch_optional(pool)
            .await?;
    Ok(parse_trusted_proxy_ips(value.as_ref()))
}

fn parse_trusted_proxy_ips(value: Option<&Value>) -> Vec<IpAddr> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .filter_map(parse_ip_header_value)
            .collect(),
        Some(Value::String(raw)) => raw
            .split(|ch: char| ch == ',' || ch == '，' || ch.is_whitespace())
            .filter_map(parse_ip_header_value)
            .collect(),
        _ => Vec::new(),
    }
}

fn client_ip_for_request(
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
    trusted_proxies: &[IpAddr],
) -> Option<IpAddr> {
    let Some(peer_ip) = peer_ip else {
        return None;
    };
    if !trusted_proxies.contains(&peer_ip) {
        return Some(peer_ip);
    }
    Some(forwarded_client_ip(headers).unwrap_or(peer_ip))
}

fn forwarded_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    if let Some(ip) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').find_map(parse_ip_header_value))
    {
        return Some(ip);
    }
    if let Some(ip) = headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_ip_header_value)
    {
        return Some(ip);
    }
    None
}

fn parse_ip_header_value(value: &str) -> Option<IpAddr> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Ok(addr) = trimmed.parse::<SocketAddr>() {
        return Some(addr.ip());
    }
    trimmed
        .strip_prefix('[')
        .and_then(|value| value.split_once(']'))
        .and_then(|(ip, _)| ip.parse::<IpAddr>().ok())
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

#[utoipa::path(post, path = "/api/v2/auth/password", tag = "auth", request_body = ChangePasswordRequest, responses((status = 200, body = ChangePasswordResponse)))]
pub async fn change_password(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(req): Json<ChangePasswordRequest>,
) -> AppResult<Json<ChangePasswordResponse>> {
    let new_password = req.new_password.as_str();
    validate_new_password(new_password)?;

    let row: Option<(String,)> =
        sqlx::query_as("SELECT password_hash FROM auth_users WHERE id = $1")
            .bind(session.user_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((stored,)) = row else {
        return Err(AppError::Unauthorized("未登录或会话已过期".to_string()));
    };
    if !verify_password(&req.current_password, &stored) {
        return Err(AppError::Unauthorized("当前密码错误".to_string()));
    }
    if req.current_password == new_password {
        return Err(AppError::BadRequest("新密码不能和当前密码相同".to_string()));
    }

    let next_hash = hash_argon2(new_password)?;
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE auth_users
         SET password_hash = $1, legacy_hash = FALSE, updated_at = now()
         WHERE id = $2",
    )
    .bind(next_hash)
    .bind(session.user_id)
    .execute(&mut *tx)
    .await?;
    let deleted = sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND token <> $2")
        .bind(session.user_id)
        .bind(&session.token)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;

    Ok(Json(ChangePasswordResponse {
        ok: true,
        invalidated_sessions: deleted,
    }))
}

fn validate_new_password(password: &str) -> AppResult<()> {
    if password.chars().count() < 8 {
        return Err(AppError::BadRequest("新密码至少需要 8 个字符".to_string()));
    }
    if password.chars().count() > 256 {
        return Err(AppError::BadRequest("新密码过长".to_string()));
    }
    if password.chars().any(char::is_whitespace) {
        return Err(AppError::BadRequest("新密码不能包含空白字符".to_string()));
    }
    Ok(())
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
    use serde_json::json;

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(*name, value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn verifies_legacy_python_pbkdf2_hash() {
        let stored = "pbkdf2_sha256$200000$000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f$2f9a5868926f81696e0202fe00533133713115e866c99c0b62def7943d448a2c";
        assert!(verify_password("secret", stored));
        assert!(!verify_password("wrong", stored));
    }

    #[test]
    fn untrusted_peer_cannot_spoof_forwarded_headers() {
        let headers = headers(&[
            ("x-forwarded-for", "203.0.113.10, 10.0.0.1"),
            ("x-real-ip", "203.0.113.11"),
        ]);

        let actual = client_ip_for_request(&headers, Some(ip("198.51.100.20")), &[ip("10.0.0.1")]);

        assert_eq!(actual, Some(ip("198.51.100.20")));
    }

    #[test]
    fn trusted_peer_uses_first_valid_x_forwarded_for() {
        let headers = headers(&[
            ("x-forwarded-for", "unknown, 203.0.113.10:443, 198.51.100.9"),
            ("x-real-ip", "203.0.113.11"),
        ]);

        let actual = client_ip_for_request(&headers, Some(ip("10.0.0.1")), &[ip("10.0.0.1")]);

        assert_eq!(actual, Some(ip("203.0.113.10")));
    }

    #[test]
    fn trusted_peer_falls_back_to_x_real_ip() {
        let headers = headers(&[("x-real-ip", "2001:db8::8")]);

        let actual = client_ip_for_request(&headers, Some(ip("10.0.0.1")), &[ip("10.0.0.1")]);

        assert_eq!(actual, Some(ip("2001:db8::8")));
    }

    #[test]
    fn invalid_forwarded_headers_fall_back_to_peer_ip() {
        let headers = headers(&[("x-forwarded-for", "unknown, nope"), ("x-real-ip", "bad")]);

        let actual = client_ip_for_request(&headers, Some(ip("10.0.0.1")), &[ip("10.0.0.1")]);

        assert_eq!(actual, Some(ip("10.0.0.1")));
    }

    #[test]
    fn parses_trusted_proxy_settings_from_array_or_text() {
        assert_eq!(
            parse_trusted_proxy_ips(Some(&json!(["192.168.2.1", "bad", "[2001:db8::1]"]))),
            vec![ip("192.168.2.1"), ip("2001:db8::1")]
        );
        assert_eq!(
            parse_trusted_proxy_ips(Some(&json!("192.168.2.1, 10.0.0.1\n2001:db8::2"))),
            vec![ip("192.168.2.1"), ip("10.0.0.1"), ip("2001:db8::2")]
        );
    }

    #[test]
    fn parses_ip_header_values_with_socket_forms() {
        assert_eq!(
            parse_ip_header_value("203.0.113.10:443"),
            Some(ip("203.0.113.10"))
        );
        assert_eq!(
            parse_ip_header_value("[2001:db8::5]:443"),
            Some(ip("2001:db8::5"))
        );
        assert_eq!(
            parse_ip_header_value("[2001:db8::6]"),
            Some(ip("2001:db8::6"))
        );
        assert_eq!(parse_ip_header_value("unknown"), None);
    }
}
