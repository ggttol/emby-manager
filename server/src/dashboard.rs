use crate::{
    config_store, dedup,
    emby::{EmbyClient, EmbyLibrary},
    error::{AppResult, redact_sensitive_text},
    posters::{self, PosterDetectRequest},
    state::AppState,
    zhuigeng,
};
use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use std::collections::BTreeMap;

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DashboardTodoResponse {
    pub noposter: usize,
    pub no_rating: usize,
    pub dups_auto: usize,
    pub dups_review: usize,
    pub airing_count: usize,
    pub airing_low_count: usize,
    pub noposter_by_lib: BTreeMap<String, usize>,
    pub no_rating_by_lib: BTreeMap<String, usize>,
    pub noposter_err: Option<String>,
    pub no_rating_err: Option<String>,
    pub dups_err: Option<String>,
    pub airing_err: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v2/dashboard/todo", get(dashboard_todo))
}

#[utoipa::path(get, path = "/api/v2/dashboard/todo", tag = "dashboard", responses((status = 200, body = DashboardTodoResponse)))]
pub async fn dashboard_todo(
    State(state): State<AppState>,
) -> AppResult<Json<DashboardTodoResponse>> {
    let mut response = DashboardTodoResponse {
        noposter: 0,
        no_rating: 0,
        dups_auto: 0,
        dups_review: 0,
        airing_count: 0,
        airing_low_count: 0,
        noposter_by_lib: BTreeMap::new(),
        no_rating_by_lib: BTreeMap::new(),
        noposter_err: None,
        no_rating_err: None,
        dups_err: None,
        airing_err: None,
    };

    match no_poster_todo(&state).await {
        Ok((total, by_lib)) => {
            response.noposter = total;
            response.noposter_by_lib = by_lib;
        }
        Err(err) => response.noposter_err = Some(redact_sensitive_text(&err.to_string())),
    }

    match no_rating_todo(&state).await {
        Ok((total, by_lib)) => {
            response.no_rating = total;
            response.no_rating_by_lib = by_lib;
        }
        Err(err) => response.no_rating_err = Some(redact_sensitive_text(&err.to_string())),
    }

    match dedup::analyze_duplicate_groups(&state.settings.strm_root) {
        Ok(dups) => {
            response.dups_auto = dups.dups.len();
            response.dups_review = dups.review.len();
        }
        Err(err) => response.dups_err = Some(redact_sensitive_text(&err.to_string())),
    }

    match zhuigeng::status(State(state.clone())).await {
        Ok(Json(status)) => {
            response.airing_count = status.continuing;
            response.airing_low_count = status.continuing;
        }
        Err(err) => {
            let message = err.to_string();
            if !is_optional_tmdb_config_error(&message) {
                response.airing_err = Some(redact_sensitive_text(&message));
            }
        }
    }

    Ok(Json(response))
}

fn is_optional_tmdb_config_error(message: &str) -> bool {
    message.contains("tmdb_api_key/tmdb_key 未配置")
        || message.contains("tmdb_base_url/tmdb_url 未配置")
}

async fn no_poster_todo(state: &AppState) -> AppResult<(usize, BTreeMap<String, usize>)> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let report = posters::detect_mismatched_posters(
        &client,
        PosterDetectRequest {
            lib: None,
            limit: Some(100_000),
            include_missing_primary: Some(true),
        },
    )
    .await?;

    let mut by_lib = BTreeMap::new();
    for item in report.items.iter().filter(|item| !item.has_poster) {
        *by_lib.entry(item.lib.clone()).or_insert(0) += 1;
    }
    Ok((report.missing_primary_total, by_lib))
}

async fn no_rating_todo(state: &AppState) -> AppResult<(usize, BTreeMap<String, usize>)> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let libraries = client.libraries().await?;
    let mut total = 0usize;
    let mut by_lib = BTreeMap::new();

    for library in libraries {
        let Some(parent_id) = library.id.as_deref().filter(|id| !id.trim().is_empty()) else {
            continue;
        };
        let page = client
            .cleanup_items(parent_id, dashboard_item_types(&library), 30_000)
            .await?;
        let count = page
            .items
            .iter()
            .filter(|item| item.community_rating.unwrap_or(0.0) <= 0.0)
            .count();
        if count > 0 {
            total += count;
            by_lib.insert(library.name.clone(), count);
        }
    }

    Ok((total, by_lib))
}

fn dashboard_item_types(library: &EmbyLibrary) -> &'static str {
    match library.library_type.to_ascii_lowercase().as_str() {
        "movies" | "movie" => "Movie",
        "tvshows" | "series" | "shows" => "Series",
        _ => "Movie,Series",
    }
}
