use crate::{
    auth, autostrm, c115, catalog, config_store, dashboard, dedup, error::AppResult, gaps,
    insights, logs, media_fs, openapi::ApiDoc, posters, scheduler, settings::Settings,
    smart_actions, state::AppState, system, tasks, undo, users, wizard, zhuigeng,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{OriginalUri, State},
    http::{HeaderName, HeaderValue, Request, StatusCode, header},
    middleware,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{any, get},
};
use serde::Serialize;
use sqlx::PgPool;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use utoipa::OpenApi;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub database: &'static str,
}

pub fn router(pool: PgPool, settings: Settings) -> Router {
    router_with_state(AppState::new(pool, settings))
}

pub fn router_with_state(state: AppState) -> Router {
    let web_dist = state.settings.web_dist.clone();
    let index = web_dist.join("index.html");
    let auth_state = state.clone();

    let api = Router::new()
        .route("/health", get(health))
        .route("/api/v2/openapi.json", get(openapi_json))
        .merge(auth::public_router())
        .merge(auth::protected_router())
        .merge(config_store::router())
        .merge(tasks::router())
        .merge(scheduler::router())
        .merge(dashboard::router())
        .merge(smart_actions::router())
        .merge(catalog::router())
        .merge(system::router())
        .merge(autostrm::router())
        .merge(c115::router())
        .merge(dedup::router())
        .merge(media_fs::router())
        .merge(undo::router())
        .merge(logs::router())
        .merge(insights::router())
        .merge(gaps::router())
        .merge(posters::router())
        .merge(users::router())
        .merge(zhuigeng::router())
        .merge(wizard::router())
        .route("/api/v2/{*path}", any(api_not_found))
        .with_state(state);

    api.fallback_service(ServeDir::new(web_dist).not_found_service(ServeFile::new(index)))
        .layer(middleware::from_fn_with_state(
            auth_state,
            auth::require_api_auth,
        ))
        .layer(middleware::from_fn(security_headers))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}

#[utoipa::path(get, path = "/health", tag = "health", responses((status = 200, body = HealthResponse)))]
pub async fn health(State(state): State<AppState>) -> AppResult<Json<HealthResponse>> {
    sqlx::query("SELECT 1").execute(&state.pool).await?;
    Ok(Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        database: "ok",
    }))
}

async fn openapi_json() -> Response {
    (StatusCode::OK, Json(ApiDoc::openapi())).into_response()
}

async fn api_not_found(uri: OriginalUri) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "not_found",
            "message": format!("API route not found: {}", uri.0.path()),
        })),
    )
        .into_response()
}

async fn security_headers(req: Request<Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers
        .entry(header::X_CONTENT_TYPE_OPTIONS)
        .or_insert(HeaderValue::from_static("nosniff"));
    headers
        .entry(HeaderName::from_static("x-frame-options"))
        .or_insert(HeaderValue::from_static("DENY"));
    headers
        .entry(HeaderName::from_static("referrer-policy"))
        .or_insert(HeaderValue::from_static("same-origin"));
    headers
        .entry(HeaderName::from_static("permissions-policy"))
        .or_insert(HeaderValue::from_static(
            "camera=(), microphone=(), geolocation=()",
        ));
    headers.entry(HeaderName::from_static("content-security-policy")).or_insert(
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'",
        ),
    );
    response
}
