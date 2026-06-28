use crate::{
    auth, c115, catalog, config_store, error::AppResult, insights, logs, media_fs, openapi::ApiDoc,
    posters, scheduler, settings::Settings, state::AppState, system, tasks, undo,
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PlaceholderResponse {
    pub ok: bool,
    pub module: &'static str,
    pub message: &'static str,
}

pub fn router(pool: PgPool, settings: Settings) -> Router {
    let web_dist = settings.web_dist.clone();
    let index = web_dist.join("index.html");
    let state = AppState::new(pool, settings);
    let auth_state = state.clone();

    let api = Router::new()
        .route("/health", get(health))
        .route("/api/v2/openapi.json", get(openapi_json))
        .merge(auth::public_router())
        .merge(auth::protected_router())
        .merge(config_store::router())
        .merge(tasks::router())
        .merge(scheduler::router())
        .merge(catalog::router())
        .merge(system::router())
        .merge(c115::router())
        .merge(media_fs::router())
        .merge(undo::router())
        .merge(logs::router())
        .merge(insights::router())
        .merge(posters::router())
        .with_state(state);

    api.fallback_service(ServeDir::new(web_dist).not_found_service(ServeFile::new(index)))
        .layer(middleware::from_fn_with_state(
            auth_state,
            auth::require_api_auth,
        ))
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
