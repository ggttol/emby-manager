use crate::{
    c115::{self, C115Client, C115OfflineRequest, C115SaveRequest},
    config_store,
    emby::{EmbyClient, EmbyEpisode, EmbyItem, EmbyLibrary},
    error::{AppError, AppResult},
    gaps::series_gaps,
    state::AppState,
    tasks::{self, TaskRun},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

const C115_COOKIE_KEY: &str = "c115_cookie";
const C115_CID_MAP_KEY: &str = "c115_cid_map";
const C115_API_BASE_URL_KEY: &str = "c115_api_base_url";
const C115_SITE_BASE_URL_KEY: &str = "c115_site_base_url";
const TG_RESOURCE_API_BASE_URL_KEY: &str = "tg_resource_api_base_url";
const TG_RESOURCE_API_TOKEN_KEY: &str = "tg_resource_api_token";
const DEFAULT_TG_RESOURCE_API_BASE_URL: &str = "http://gaotao.cc:8100";
const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct CatalogSearchQuery {
    pub q: String,
    pub limit: Option<i64>,
    pub link_type: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CatalogItem {
    pub name: String,
    pub sheet: String,
    pub link: String,
    pub is_pkg: bool,
    pub link_type: String,
    pub transfer: bool,
    pub share: Option<String>,
    pub rc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<CatalogResourceRecommendation>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CatalogSearchResponse {
    pub items: Vec<CatalogItem>,
    pub total: usize,
    pub truncated: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct CatalogRemoteSearchQuery {
    pub q: String,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub disk_type: Option<String>,
    pub exact: Option<bool>,
    pub sort: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CatalogRemoteDiskTypeCount {
    pub disk_type: String,
    pub count: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CatalogRemoteSearchResponse {
    pub items: Vec<CatalogItem>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    pub has_more: bool,
    pub query: String,
    pub exact: bool,
    pub sort: String,
    pub truncated: bool,
    pub disk_types: Vec<CatalogRemoteDiskTypeCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<CatalogLibraryContextResponse>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct CatalogLibraryContextQuery {
    pub q: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CatalogResourceRecommendation {
    pub score: i32,
    pub level: String,
    pub action: String,
    pub reasons: Vec<String>,
    pub episode_ranges: Vec<String>,
    pub covers_missing: bool,
    pub duplicate_risk: bool,
    pub already_have: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CatalogLibraryContextResponse {
    pub ok: bool,
    pub query: String,
    pub total_matches: usize,
    pub truncated: bool,
    pub summary: CatalogLibraryContextSummary,
    pub items: Vec<CatalogLibraryContextItem>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct CatalogLibraryContextSummary {
    pub matched: bool,
    pub duplicate: bool,
    pub duplicate_groups: usize,
    pub libraries: Vec<String>,
    pub tmdb_ids: Vec<String>,
    pub years: Vec<i32>,
    pub episode_ranges: Vec<String>,
    pub missing_ranges: Vec<String>,
    pub max_episode: i32,
    pub total_episodes: usize,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CatalogLibraryContextItem {
    pub id: Option<String>,
    pub name: String,
    pub item_type: String,
    pub library: Option<String>,
    pub folder: Option<String>,
    pub path: Option<String>,
    pub year: Option<i32>,
    pub tmdb: Option<String>,
    pub has_primary_image: bool,
    pub duplicate: bool,
    pub episode_count: usize,
    pub episode_ranges: Vec<String>,
    pub missing_ranges: Vec<String>,
    pub max_episode: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CatalogStatsResponse {
    pub available: bool,
    pub total: i64,
    pub packages: i64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct CatalogDuplicateQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CatalogDuplicateGroup {
    pub key: String,
    pub count: i64,
    pub link_types: Vec<String>,
    pub sample_names: Vec<String>,
    pub sample_sheets: Vec<String>,
    pub sample_links: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CatalogLinkTypeCount {
    pub link_type: String,
    pub count: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CatalogDuplicatesResponse {
    pub ok: bool,
    pub readonly: bool,
    pub limit: i64,
    pub duplicate_link_groups: i64,
    pub duplicate_name_groups: i64,
    pub link_type_distribution: Vec<CatalogLinkTypeCount>,
    pub link_groups: Vec<CatalogDuplicateGroup>,
    pub name_groups: Vec<CatalogDuplicateGroup>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CatalogTransferPlanItem {
    pub name: Option<String>,
    pub sheet: Option<String>,
    pub link: String,
    pub is_pkg: Option<bool>,
    pub link_type: Option<String>,
    pub share: Option<String>,
    pub rc: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CatalogTransferPlanRequest {
    pub item: Option<CatalogTransferPlanItem>,
    pub link: Option<String>,
    pub label: Option<String>,
    pub name: Option<String>,
    pub sheet: Option<String>,
    pub is_pkg: Option<bool>,
    pub link_type: Option<String>,
    pub share: Option<String>,
    pub rc: Option<String>,
    pub pwd: Option<String>,
    pub lib: Option<String>,
    pub cid: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CatalogTransferAction {
    SaveShare,
    OfflineDownload,
    Unsupported,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogTransferTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lib: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogC115SavePayload {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lib: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogC115SavePlan {
    pub endpoint: String,
    pub method: String,
    pub share: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receive_code: Option<String>,
    pub payload: CatalogC115SavePayload,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogC115OfflinePayload {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lib: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogC115OfflinePlan {
    pub endpoint: String,
    pub method: String,
    pub protocol: String,
    pub payload: CatalogC115OfflinePayload,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogUnsupportedPlan {
    pub reason: String,
    pub link: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogTransferPlanResponse {
    pub ok: bool,
    pub action: CatalogTransferAction,
    pub link_type: String,
    pub transfer: bool,
    pub is_pkg: bool,
    pub target: CatalogTransferTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save: Option<CatalogC115SavePlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offline: Option<CatalogC115OfflinePlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<CatalogUnsupportedPlan>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CatalogTransferExecuteRequest {
    #[serde(default)]
    pub items: Vec<CatalogTransferPlanItem>,
    pub target: Option<CatalogTransferTarget>,
    pub item: Option<CatalogTransferPlanItem>,
    pub link: Option<String>,
    pub label: Option<String>,
    pub name: Option<String>,
    pub sheet: Option<String>,
    pub is_pkg: Option<bool>,
    pub link_type: Option<String>,
    pub share: Option<String>,
    pub rc: Option<String>,
    pub pwd: Option<String>,
    pub lib: Option<String>,
    pub cid: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/catalog/stats", get(catalog_stats))
        .route("/api/v2/catalog/search", get(catalog_search))
        .route("/api/v2/catalog/remote-search", get(catalog_remote_search))
        .route(
            "/api/v2/catalog/library-context",
            get(catalog_library_context),
        )
        .route("/api/v2/catalog/duplicates", get(catalog_duplicates))
        .route("/api/v2/catalog/transfer-plan", post(catalog_transfer_plan))
        .route(
            "/api/v2/catalog/transfer/execute",
            post(catalog_transfer_execute),
        )
}

#[utoipa::path(get, path = "/api/v2/catalog/stats", tag = "catalog", responses((status = 200, body = CatalogStatsResponse)))]
pub async fn catalog_stats(State(state): State<AppState>) -> AppResult<Json<CatalogStatsResponse>> {
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM catalog_items")
        .fetch_one(&state.pool)
        .await?;
    let packages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM catalog_items WHERE is_pkg")
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(CatalogStatsResponse {
        available: total > 0,
        total,
        packages,
    }))
}

#[utoipa::path(get, path = "/api/v2/catalog/duplicates", tag = "catalog", params(CatalogDuplicateQuery), responses((status = 200, body = CatalogDuplicatesResponse)))]
pub async fn catalog_duplicates(
    State(state): State<AppState>,
    Query(q): Query<CatalogDuplicateQuery>,
) -> AppResult<Json<CatalogDuplicatesResponse>> {
    let limit = q.limit.unwrap_or(20).clamp(1, 100);
    let duplicate_link_groups: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (
            SELECT link FROM catalog_items
            WHERE btrim(link) <> ''
            GROUP BY link
            HAVING COUNT(*) > 1
        ) groups",
    )
    .fetch_one(&state.pool)
    .await?;
    let duplicate_name_groups: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (
            SELECT name FROM catalog_items
            WHERE btrim(name) <> ''
            GROUP BY name
            HAVING COUNT(*) > 1
        ) groups",
    )
    .fetch_one(&state.pool)
    .await?;
    let link_type_distribution = sqlx::query_as::<_, (String, i64)>(
        "WITH duplicate_rows AS (
            SELECT link_type
            FROM catalog_items c
            WHERE EXISTS (
                SELECT 1 FROM catalog_items d
                WHERE d.link = c.link AND btrim(d.link) <> ''
                GROUP BY d.link
                HAVING COUNT(*) > 1
            )
            OR EXISTS (
                SELECT 1 FROM catalog_items d
                WHERE d.name = c.name AND btrim(d.name) <> ''
                GROUP BY d.name
                HAVING COUNT(*) > 1
            )
        )
        SELECT link_type, COUNT(*) AS count
        FROM duplicate_rows
        GROUP BY link_type
        ORDER BY count DESC, link_type",
    )
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(link_type, count)| CatalogLinkTypeCount { link_type, count })
    .collect();
    let link_groups = duplicate_groups(&state.pool, DuplicateKind::Link, limit).await?;
    let name_groups = duplicate_groups(&state.pool, DuplicateKind::Name, limit).await?;

    Ok(Json(CatalogDuplicatesResponse {
        ok: true,
        readonly: true,
        limit,
        duplicate_link_groups,
        duplicate_name_groups,
        link_type_distribution,
        link_groups,
        name_groups,
    }))
}

#[utoipa::path(get, path = "/api/v2/catalog/search", tag = "catalog", params(CatalogSearchQuery), responses((status = 200, body = CatalogSearchResponse)))]
pub async fn catalog_search(
    State(state): State<AppState>,
    Query(q): Query<CatalogSearchQuery>,
) -> AppResult<Json<CatalogSearchResponse>> {
    let limit = q.limit.unwrap_or(80).clamp(1, 200);
    let terms = split_terms(&q.q);
    if terms.is_empty() || terms.iter().all(|t| t.chars().count() < 2) {
        return Ok(Json(CatalogSearchResponse {
            items: vec![],
            total: 0,
            truncated: false,
        }));
    }
    let mut sql =
        "SELECT name, sheet, link, is_pkg, link_type FROM catalog_items WHERE ".to_string();
    let mut parts = Vec::new();
    for i in 0..terms.len() {
        parts.push(format!("name ILIKE ${}", i + 1));
    }
    let mut bind_count = terms.len();
    if matches!(q.link_type.as_deref(), Some("share115" | "magnet" | "ed2k")) {
        bind_count += 1;
        parts.push(format!("link_type = ${bind_count}"));
    }
    sql.push_str(&parts.join(" AND "));
    sql.push_str(" ORDER BY (link_type = 'share115') DESC, is_pkg ASC, length(name) ASC LIMIT $");
    sql.push_str(&(bind_count + 1).to_string());

    let mut query = sqlx::query_as::<_, (String, String, String, bool, String)>(&sql);
    for term in &terms {
        query = query.bind(format!("%{}%", escape_like(term)));
    }
    if let Some(link_type) = q
        .link_type
        .filter(|v| matches!(v.as_str(), "share115" | "magnet" | "ed2k"))
    {
        query = query.bind(link_type);
    }
    query = query.bind(limit + 1);
    let mut rows = query.fetch_all(&state.pool).await?;
    let truncated = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let items = rows
        .into_iter()
        .map(|(name, sheet, link, is_pkg, link_type)| {
            let (share, rc) = parse_share(&link);
            CatalogItem {
                transfer: link_type == "share115",
                name,
                sheet,
                link,
                is_pkg,
                link_type,
                share,
                rc,
                recommendation: None,
            }
        })
        .collect::<Vec<_>>();
    Ok(Json(CatalogSearchResponse {
        total: items.len(),
        items,
        truncated,
    }))
}

#[utoipa::path(get, path = "/api/v2/catalog/remote-search", tag = "catalog", params(CatalogRemoteSearchQuery), responses((status = 200, body = CatalogRemoteSearchResponse)))]
pub async fn catalog_remote_search(
    State(state): State<AppState>,
    Query(q): Query<CatalogRemoteSearchQuery>,
) -> AppResult<Json<CatalogRemoteSearchResponse>> {
    let keyword = q.q.trim().to_string();
    if keyword.is_empty() {
        return Ok(Json(CatalogRemoteSearchResponse {
            items: vec![],
            total: 0,
            limit: q.limit.unwrap_or(80).clamp(1, 500),
            offset: q.offset.unwrap_or(0).max(0),
            has_more: false,
            query: keyword.clone(),
            exact: q.exact.unwrap_or(false),
            sort: normalized_remote_sort(q.sort.as_deref()),
            truncated: false,
            disk_types: vec![],
            context: Some(empty_library_context(&keyword, "请输入关键词")),
        }));
    }

    let context = match build_catalog_library_context(&state, &keyword, 8).await {
        Ok(context) => context,
        Err(err) => unavailable_library_context(&keyword, err.to_string()),
    };
    let base_url = config_store::get_string_or(
        &state.pool,
        TG_RESOURCE_API_BASE_URL_KEY,
        DEFAULT_TG_RESOURCE_API_BASE_URL,
    )
    .await?;
    let token = config_store::get_string(&state.pool, TG_RESOURCE_API_TOKEN_KEY).await?;
    let mut response =
        fetch_tg_resource_search(&state.http, &base_url, token.as_deref(), q).await?;
    apply_catalog_recommendations(&mut response.items, &context);
    response.context = Some(context);
    Ok(Json(response))
}

#[utoipa::path(get, path = "/api/v2/catalog/library-context", tag = "catalog", params(CatalogLibraryContextQuery), responses((status = 200, body = CatalogLibraryContextResponse)))]
pub async fn catalog_library_context(
    State(state): State<AppState>,
    Query(q): Query<CatalogLibraryContextQuery>,
) -> AppResult<Json<CatalogLibraryContextResponse>> {
    let keyword = q.q.trim().to_string();
    if keyword.is_empty() {
        return Ok(Json(empty_library_context(&keyword, "请输入关键词")));
    }
    Ok(Json(
        build_catalog_library_context(&state, &keyword, q.limit.unwrap_or(8)).await?,
    ))
}

#[derive(Debug, Deserialize)]
struct TgResourceApiResponse {
    code: i64,
    message: String,
    data: Option<TgResourceSearchData>,
}

#[derive(Debug, Default, Deserialize)]
struct TgResourceSearchData {
    #[serde(default)]
    total: i64,
    #[serde(default)]
    limit: i64,
    #[serde(default)]
    offset: i64,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    query: String,
    #[serde(default)]
    exact: bool,
    #[serde(default)]
    sort: String,
    #[serde(default)]
    disk_types: Vec<TgResourceDiskTypeCount>,
    #[serde(default)]
    results: Vec<TgResourceResult>,
}

#[derive(Debug, Deserialize)]
struct TgResourceDiskTypeCount {
    disk_type: String,
    count: i64,
}

#[derive(Debug, Deserialize)]
struct TgResourceResult {
    title: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    disk_type: Option<String>,
    url: Option<String>,
    password: Option<String>,
    source: Option<String>,
    #[serde(default)]
    source_channels: Vec<String>,
}

async fn fetch_tg_resource_search(
    http: &reqwest::Client,
    base_url: &str,
    token: Option<&str>,
    q: CatalogRemoteSearchQuery,
) -> AppResult<CatalogRemoteSearchResponse> {
    let limit = q.limit.unwrap_or(80).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let exact = q.exact.unwrap_or(false);
    let sort = normalized_remote_sort(q.sort.as_deref());
    let keyword = q.q.trim().to_string();
    let disk_type = q
        .disk_type
        .as_deref()
        .and_then(non_empty_trimmed)
        .map(normalize_remote_disk_filter);

    let mut url = tg_resource_search_url(base_url)?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("kw", &keyword);
        pairs.append_pair("limit", &limit.to_string());
        pairs.append_pair("offset", &offset.to_string());
        pairs.append_pair("sort", &sort);
        if exact {
            pairs.append_pair("exact", "true");
        }
        if let Some(disk_type) = disk_type.as_deref() {
            pairs.append_pair("disk_type", disk_type);
        }
    }

    let mut request = http.get(url).header("Accept", "application/json");
    if let Some(token) = token.and_then(non_empty_trimmed) {
        request = request.bearer_auth(token);
    }

    let response = request
        .send()
        .await
        .map_err(|err| AppError::BadRequest(format!("TG 资源 API 请求失败: {err}")))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|err| AppError::BadRequest(format!("读取 TG 资源 API 响应失败: {err}")))?;
    if !status.is_success() {
        return Err(AppError::BadRequest(format!(
            "TG 资源 API 返回 HTTP {status}: {}",
            truncate_label(&String::from_utf8_lossy(&body), 180)
        )));
    }

    let parsed: TgResourceApiResponse = serde_json::from_slice(&body)
        .map_err(|err| AppError::BadRequest(format!("TG 资源 API JSON 解析失败: {err}")))?;
    if parsed.code != 0 {
        return Err(AppError::BadRequest(format!(
            "TG 资源 API 返回错误 {}: {}",
            parsed.code, parsed.message
        )));
    }

    let data = parsed.data.unwrap_or_default();
    let items = data
        .results
        .into_iter()
        .filter_map(tg_result_to_catalog_item)
        .collect::<Vec<_>>();
    let disk_types = data
        .disk_types
        .into_iter()
        .map(|item| CatalogRemoteDiskTypeCount {
            disk_type: item.disk_type,
            count: item.count,
        })
        .collect::<Vec<_>>();
    Ok(CatalogRemoteSearchResponse {
        items,
        total: data.total,
        limit: if data.limit > 0 { data.limit } else { limit },
        offset: data.offset.max(offset),
        has_more: data.has_more,
        query: if data.query.trim().is_empty() {
            keyword
        } else {
            data.query
        },
        exact: data.exact,
        sort: if data.sort.trim().is_empty() {
            sort
        } else {
            data.sort
        },
        truncated: data.has_more,
        disk_types,
        context: None,
    })
}

async fn build_catalog_library_context(
    state: &AppState,
    keyword: &str,
    limit: usize,
) -> AppResult<CatalogLibraryContextResponse> {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return Ok(empty_library_context(keyword, "请输入关键词"));
    }
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        return Ok(unavailable_library_context(
            keyword,
            "api_key 未配置，无法读取 Emby 本库情况".to_string(),
        ));
    }
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let libraries = client.libraries().await?;
    let result = client.search_items(keyword, "Movie,Series", limit).await?;
    let mut items = Vec::new();

    for item in result.items {
        items.push(context_item_from_emby(&client, &libraries, item).await);
    }
    mark_context_duplicates(&mut items);
    let summary = summarize_context(keyword, &items);
    Ok(CatalogLibraryContextResponse {
        ok: true,
        query: keyword.to_string(),
        total_matches: items.len(),
        truncated: result.truncated,
        items,
        warnings: Vec::new(),
        summary,
    })
}

async fn context_item_from_emby(
    client: &EmbyClient,
    libraries: &[EmbyLibrary],
    item: EmbyItem,
) -> CatalogLibraryContextItem {
    let id = item.id.clone();
    let name = item.name.clone().unwrap_or_else(|| "?".to_string());
    let item_type = item.item_type.clone().unwrap_or_else(|| "?".to_string());
    let library = infer_item_library(&item, libraries).map(|library| library.name.clone());
    let folder = item
        .path
        .as_deref()
        .and_then(|path| infer_folder_from_path(path, libraries));
    let tmdb = item.provider_id("Tmdb");
    let mut episode_count = 0usize;
    let mut episode_ranges = Vec::new();
    let mut missing_ranges = Vec::new();
    let mut max_episode = 0;
    let mut error = None;

    if item_type.eq_ignore_ascii_case("series")
        && let Some(series_id) = id.as_deref()
    {
        match client.episodes(series_id).await {
            Ok(episodes) => {
                let ranges = episode_ranges_from_episodes(&episodes);
                let gaps = series_gaps(&episodes);
                episode_count = gaps.have;
                episode_ranges = ranges;
                missing_ranges = missing_ranges_from_gaps(&gaps);
                max_episode = gaps.max_ep;
            }
            Err(err) => {
                error = Some(format!("读取剧集集数失败: {err}"));
            }
        }
    }

    let has_primary_image = item.has_primary_image();

    CatalogLibraryContextItem {
        id,
        name,
        item_type,
        library,
        folder,
        path: item.path,
        year: item.production_year,
        tmdb,
        has_primary_image,
        duplicate: false,
        episode_count,
        episode_ranges,
        missing_ranges,
        max_episode,
        error,
    }
}

fn empty_library_context(query: &str, note: &str) -> CatalogLibraryContextResponse {
    CatalogLibraryContextResponse {
        ok: true,
        query: query.to_string(),
        total_matches: 0,
        truncated: false,
        summary: CatalogLibraryContextSummary {
            matched: false,
            note: note.to_string(),
            ..CatalogLibraryContextSummary::default()
        },
        items: Vec::new(),
        warnings: Vec::new(),
    }
}

fn unavailable_library_context(query: &str, warning: String) -> CatalogLibraryContextResponse {
    CatalogLibraryContextResponse {
        ok: false,
        query: query.to_string(),
        total_matches: 0,
        truncated: false,
        summary: CatalogLibraryContextSummary {
            matched: false,
            note: "本库情况暂时无法读取".to_string(),
            ..CatalogLibraryContextSummary::default()
        },
        items: Vec::new(),
        warnings: vec![warning],
    }
}

fn mark_context_duplicates(items: &mut [CatalogLibraryContextItem]) {
    let mut counts = BTreeMap::<String, usize>::new();
    for item in items.iter() {
        let key = context_duplicate_key(item);
        *counts.entry(key).or_default() += 1;
    }
    for item in items {
        item.duplicate = counts
            .get(&context_duplicate_key(item))
            .is_some_and(|count| *count > 1);
    }
}

fn context_duplicate_key(item: &CatalogLibraryContextItem) -> String {
    if let Some(tmdb) = item.tmdb.as_deref().and_then(non_empty_trimmed) {
        return format!("tmdb:{tmdb}");
    }
    format!(
        "name:{}:{}",
        normalize_title_for_match(&item.name),
        item.year.unwrap_or_default()
    )
}

fn summarize_context(
    keyword: &str,
    items: &[CatalogLibraryContextItem],
) -> CatalogLibraryContextSummary {
    let mut libraries = Vec::new();
    let mut tmdb_ids = Vec::new();
    let mut years = Vec::new();
    let mut episode_ranges = Vec::new();
    let mut missing_ranges = Vec::new();
    let mut total_episodes = 0usize;
    let mut max_episode = 0i32;
    let duplicate_groups = {
        let mut keys = BTreeMap::<String, usize>::new();
        for item in items {
            *keys.entry(context_duplicate_key(item)).or_default() += 1;
        }
        keys.values().filter(|count| **count > 1).count()
    };

    for item in items {
        if let Some(library) = item.library.as_deref().and_then(non_empty_trimmed)
            && !libraries.iter().any(|existing| existing == library)
        {
            libraries.push(library.to_string());
        }
        if let Some(tmdb) = item.tmdb.as_deref().and_then(non_empty_trimmed)
            && !tmdb_ids.iter().any(|existing| existing == tmdb)
        {
            tmdb_ids.push(tmdb.to_string());
        }
        if let Some(year) = item.year
            && !years.contains(&year)
        {
            years.push(year);
        }
        for range in &item.episode_ranges {
            if !episode_ranges.iter().any(|existing| existing == range) {
                episode_ranges.push(range.clone());
            }
        }
        for range in &item.missing_ranges {
            if !missing_ranges.iter().any(|existing| existing == range) {
                missing_ranges.push(range.clone());
            }
        }
        total_episodes += item.episode_count;
        max_episode = max_episode.max(item.max_episode);
    }
    libraries.sort();
    tmdb_ids.sort();
    years.sort_unstable();

    let note = if items.is_empty() {
        format!("Emby 库里暂时没搜到「{keyword}」")
    } else if duplicate_groups > 0 && !missing_ranges.is_empty() {
        "库内已有重复且存在缺集，建议转存后进入去重确认".to_string()
    } else if duplicate_groups > 0 {
        "库内已有重复条目，转存前后都建议做去重确认".to_string()
    } else if !missing_ranges.is_empty() {
        "库内已有条目但存在缺集，可优先选补缺或全集资源".to_string()
    } else {
        "库内已有匹配条目，优先避免重复转存".to_string()
    };

    CatalogLibraryContextSummary {
        matched: !items.is_empty(),
        duplicate: duplicate_groups > 0,
        duplicate_groups,
        libraries,
        tmdb_ids,
        years,
        episode_ranges,
        missing_ranges,
        max_episode,
        total_episodes,
        note,
    }
}

fn apply_catalog_recommendations(
    items: &mut [CatalogItem],
    context: &CatalogLibraryContextResponse,
) {
    for item in items.iter_mut() {
        item.recommendation = Some(recommend_catalog_item(item, context));
    }
    items.sort_by(|left, right| {
        let left_score = left
            .recommendation
            .as_ref()
            .map(|recommendation| recommendation.score)
            .unwrap_or_default();
        let right_score = right
            .recommendation
            .as_ref()
            .map(|recommendation| recommendation.score)
            .unwrap_or_default();
        right_score
            .cmp(&left_score)
            .then_with(|| left.name.len().cmp(&right.name.len()))
    });
}

fn recommend_catalog_item(
    item: &CatalogItem,
    context: &CatalogLibraryContextResponse,
) -> CatalogResourceRecommendation {
    let spans = extract_episode_spans(&item.name);
    let episode_ranges = spans.iter().map(format_episode_span).collect::<Vec<_>>();
    let mut score = 0i32;
    let mut reasons = Vec::new();
    let is_115 = item.link_type == "share115";
    let duplicate_risk = context.summary.duplicate;
    let local_matched = context.summary.matched;
    let remote_max = spans.iter().map(|span| span.end).max().unwrap_or_default();
    let remote_single_existing = spans.len() == 1
        && spans[0].start == spans[0].end
        && remote_max > 0
        && local_matched
        && remote_max <= context.summary.max_episode
        && context.summary.missing_ranges.is_empty();
    let covers_missing = local_matched
        && (remote_max > context.summary.max_episode
            || (item.is_pkg
                && (!context.summary.missing_ranges.is_empty()
                    || context.summary.max_episode == 0)));

    if is_115 {
        score += 90;
        reasons.push("115 可直接转存".to_string());
    } else {
        score -= 80;
        reasons.push(format!(
            "{} 不能直接一条龙转存",
            linkTypeLabelForBackend(&item.link_type)
        ));
    }

    if !context.ok {
        reasons.push("本库情况未读取，仅按资源标题判断".to_string());
    } else if !local_matched {
        score += 35;
        reasons.push("Emby 库内未找到同名条目，可作为新资源".to_string());
    }

    if item.is_pkg {
        score += 35;
        reasons.push("标题像整包/全集".to_string());
    }
    if covers_missing {
        score += 80;
        if remote_max > context.summary.max_episode && context.summary.max_episode > 0 {
            reasons.push(format!(
                "资源到 E{remote_max}，本地到 E{}，适合补缺",
                context.summary.max_episode
            ));
        } else {
            reasons.push("整包资源适合补齐当前缺口".to_string());
        }
    }
    if remote_single_existing {
        score -= 90;
        reasons.push(format!("疑似单集 E{remote_max}，本地大概率已有"));
    }
    if let Some(tmdb) = declared_tmdb_id(&item.name)
        && context
            .summary
            .tmdb_ids
            .iter()
            .any(|existing| existing == &tmdb)
    {
        score += 60;
        reasons.push(format!("TMDb {tmdb} 与本库匹配"));
    }
    if let Some(year) = declared_year(&item.name)
        && context.summary.years.contains(&year)
    {
        score += 20;
        reasons.push(format!("{year} 年份与本库匹配"));
    }
    if duplicate_risk {
        score -= 10;
        reasons.push("库内已有重复，转存后建议进入去重确认".to_string());
    }

    let already_have = remote_single_existing && !covers_missing;
    let (level, action) = if !is_115 {
        ("skip", "暂不推荐")
    } else if already_have {
        ("skip", "可能已存在")
    } else if score >= 190 {
        ("best", "推荐转存")
    } else if score >= 130 {
        ("good", "可转存")
    } else if score >= 70 {
        ("warn", "谨慎确认")
    } else {
        ("skip", "低优先")
    };

    if reasons.is_empty() {
        reasons.push("没有足够上下文，需人工判断".to_string());
    }

    CatalogResourceRecommendation {
        score,
        level: level.to_string(),
        action: action.to_string(),
        reasons,
        episode_ranges,
        covers_missing,
        duplicate_risk,
        already_have,
    }
}

fn tg_resource_search_url(base_url: &str) -> AppResult<Url> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(AppError::BadRequest("TG 资源 API 地址不能为空".to_string()));
    }
    let endpoint = if base.ends_with("/api") {
        format!("{base}/v1/search")
    } else {
        format!("{base}/api/v1/search")
    };
    Url::parse(&endpoint)
        .map_err(|err| AppError::BadRequest(format!("TG 资源 API 地址无效: {endpoint} ({err})")))
}

fn tg_result_to_catalog_item(result: TgResourceResult) -> Option<CatalogItem> {
    let link = clean(result.url)?;
    let disk_type = first_clean([result.disk_type, result.kind]).unwrap_or_default();
    let link_type = normalize_remote_link_type(&disk_type, &link);
    let title = first_clean([result.title]).unwrap_or_else(|| link.clone());
    let (share, rc_from_link) = parse_share(&link);
    let rc = first_clean([result.password, rc_from_link]);
    let sheet = first_clean([
        result.source,
        (!result.source_channels.is_empty()).then(|| result.source_channels.join(",")),
    ])
    .unwrap_or_else(|| "TG Resource API".to_string());
    Some(CatalogItem {
        name: title.clone(),
        sheet,
        link,
        is_pkg: likely_remote_package_title(&title),
        transfer: link_type == "share115",
        link_type,
        share,
        rc,
        recommendation: None,
    })
}

fn normalize_remote_link_type(disk_type: &str, link: &str) -> String {
    match disk_type.trim().to_ascii_lowercase().as_str() {
        "115" | "share115" | "115cdn" => "share115".to_string(),
        "magnet" => "magnet".to_string(),
        "ed2k" => "ed2k".to_string(),
        "" => infer_type(link).to_string(),
        other => {
            let inferred = infer_type(link);
            if inferred == "other" {
                other.to_string()
            } else {
                inferred.to_string()
            }
        }
    }
}

fn normalize_remote_disk_filter(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    if value == "share115" {
        "115".to_string()
    } else {
        value
    }
}

fn normalized_remote_sort(value: Option<&str>) -> String {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("seen") => "seen".to_string(),
        _ => "resource".to_string(),
    }
}

fn likely_remote_package_title(title: &str) -> bool {
    let lower = title.to_ascii_lowercase().replace([' ', '_'], "");
    title.contains("全集")
        || title.contains("合集")
        || title.contains("完结")
        || title.contains("更新至")
        || (title.contains("全") && title.contains("集"))
        || (title.contains("共") && title.contains("集"))
        || lower.contains("e01-e")
        || lower.contains("e1-e")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EpisodeSpan {
    season: Option<i32>,
    start: i32,
    end: i32,
}

fn extract_episode_spans(title: &str) -> Vec<EpisodeSpan> {
    let mut spans = extract_sxe_spans(title);
    if spans.is_empty()
        && let Some(count) = extract_chinese_episode_count(title)
    {
        spans.push(EpisodeSpan {
            season: Some(1),
            start: 1,
            end: count,
        });
    }
    spans.sort_by(|left, right| {
        left.season
            .cmp(&right.season)
            .then_with(|| left.start.cmp(&right.start))
            .then_with(|| left.end.cmp(&right.end))
    });
    spans.dedup();
    spans
}

fn extract_sxe_spans(title: &str) -> Vec<EpisodeSpan> {
    let chars = title.chars().collect::<Vec<_>>();
    let mut spans = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if !matches!(chars[index], 's' | 'S') {
            index += 1;
            continue;
        }
        let Some((season, after_season)) = read_number(&chars, index + 1) else {
            index += 1;
            continue;
        };
        if after_season >= chars.len() || !matches!(chars[after_season], 'e' | 'E') {
            index += 1;
            continue;
        }
        let Some((start, mut end_index)) = read_number(&chars, after_season + 1) else {
            index += 1;
            continue;
        };
        let mut end = start;
        let mut cursor = skip_separators(&chars, end_index);
        if cursor > end_index {
            if cursor < chars.len() && matches!(chars[cursor], 'e' | 'E') {
                cursor += 1;
            }
            if let Some((parsed_end, parsed_index)) = read_number(&chars, cursor) {
                end = parsed_end.max(start);
                end_index = parsed_index;
            }
        }
        if start > 0 {
            spans.push(EpisodeSpan {
                season: Some(season),
                start,
                end,
            });
        }
        index = end_index.max(index + 1);
    }
    spans
}

fn read_number(chars: &[char], start: usize) -> Option<(i32, usize)> {
    let mut cursor = start;
    let mut value = String::new();
    while cursor < chars.len() && chars[cursor].is_ascii_digit() && value.len() < 4 {
        value.push(chars[cursor]);
        cursor += 1;
    }
    if value.is_empty() {
        None
    } else {
        Some((value.parse().ok()?, cursor))
    }
}

fn skip_separators(chars: &[char], start: usize) -> usize {
    let mut cursor = start;
    while cursor < chars.len() && matches!(chars[cursor], '-' | '–' | '—' | '~' | '至') {
        cursor += 1;
    }
    cursor
}

fn extract_chinese_episode_count(title: &str) -> Option<i32> {
    let chars = title.chars().collect::<Vec<_>>();
    for (index, ch) in chars.iter().enumerate() {
        if !matches!(ch, '集' | '话') {
            continue;
        }
        let mut cursor = index;
        let mut digits = String::new();
        while cursor > 0 {
            let prev = chars[cursor - 1];
            if prev.is_ascii_digit() {
                digits.insert(0, prev);
                cursor -= 1;
            } else {
                break;
            }
        }
        let value = digits.parse::<i32>().ok()?;
        let prefix = chars[..cursor].iter().collect::<String>();
        if value > 1
            && (prefix.ends_with('全')
                || prefix.ends_with('共')
                || prefix.ends_with("更新至")
                || prefix.ends_with('更')
                || title.contains("完结"))
        {
            return Some(value);
        }
    }
    None
}

fn format_episode_span(span: &EpisodeSpan) -> String {
    let season = span.season.unwrap_or(1).max(0);
    if span.start == span.end {
        format!("S{season:02}E{:02}", span.start)
    } else {
        format!("S{season:02}E{:02}-E{:02}", span.start, span.end)
    }
}

fn episode_ranges_from_episodes(episodes: &[EmbyEpisode]) -> Vec<String> {
    let mut by_season = BTreeMap::<Option<i32>, Vec<i32>>::new();
    for episode in episodes {
        if episode
            .location_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("Virtual"))
        {
            continue;
        }
        if let Some(number) = episode.index_number {
            by_season
                .entry(episode.parent_index_number.or(Some(1)))
                .or_default()
                .push(number);
        }
    }
    let mut ranges = Vec::new();
    for (season, mut values) in by_season {
        values.sort_unstable();
        values.dedup();
        for range in compact_ints_as_strings(&values) {
            ranges.push(format!("S{:02}E{}", season.unwrap_or(1).max(0), range));
        }
    }
    ranges
}

fn missing_ranges_from_gaps(gaps: &crate::gaps::SeriesGaps) -> Vec<String> {
    if gaps.mode == "absolute" {
        return gaps
            .gap_list
            .iter()
            .map(|range| format!("E{range}"))
            .collect();
    }
    let mut ranges = Vec::new();
    for season in &gaps.seasons {
        for range in &season.gaps {
            ranges.push(format!(
                "S{:02}E{}",
                season.season.unwrap_or(1).max(0),
                range
            ));
        }
    }
    ranges
}

fn compact_ints_as_strings(values: &[i32]) -> Vec<String> {
    if values.is_empty() {
        return Vec::new();
    }
    let mut xs = values.to_vec();
    xs.sort_unstable();
    xs.dedup();
    let mut out = Vec::new();
    let mut start = xs[0];
    let mut prev = xs[0];
    for &value in &xs[1..] {
        if value == prev + 1 {
            prev = value;
            continue;
        }
        out.push(if start == prev {
            start.to_string()
        } else {
            format!("{start}-{prev}")
        });
        start = value;
        prev = value;
    }
    out.push(if start == prev {
        start.to_string()
    } else {
        format!("{start}-{prev}")
    });
    out
}

fn infer_item_library<'a>(
    item: &EmbyItem,
    libraries: &'a [EmbyLibrary],
) -> Option<&'a EmbyLibrary> {
    let path = item.path.as_deref()?.replace('\\', "/");
    libraries.iter().find(|library| {
        library.paths.iter().any(|root| {
            let root = root.replace('\\', "/").trim_end_matches('/').to_string();
            !root.is_empty() && path.starts_with(&root)
        }) || library
            .folder
            .as_ref()
            .is_some_and(|folder| path.contains(&format!("/strm/{folder}/")))
    })
}

fn infer_folder_from_path(path: &str, libraries: &[EmbyLibrary]) -> Option<String> {
    let normalized = path.replace('\\', "/");
    for library in libraries {
        for root in &library.paths {
            let root = root.replace('\\', "/").trim_end_matches('/').to_string();
            if root.is_empty() || !normalized.starts_with(&root) {
                continue;
            }
            let rest = normalized[root.len()..].trim_start_matches('/');
            if let Some(folder) = rest
                .split('/')
                .next()
                .filter(|part| !part.trim().is_empty())
            {
                return Some(folder.to_string());
            }
        }
    }
    normalized
        .split("/strm/")
        .nth(1)
        .and_then(|rest| rest.split('/').nth(1))
        .filter(|part| !part.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            normalized
                .rsplit('/')
                .next()
                .filter(|part| !part.trim().is_empty())
                .map(ToString::to_string)
        })
}

fn normalize_title_for_match(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric() || is_cjk(*ch))
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn is_cjk(ch: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
}

fn declared_tmdb_id(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    for marker in ["tmdbid-", "tmdbid_", "tmdb-"] {
        if let Some(index) = lower.find(marker) {
            let start = index + marker.len();
            let digits = lower[start..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if !digits.is_empty() {
                return Some(digits);
            }
        }
    }
    None
}

fn declared_year(value: &str) -> Option<i32> {
    let chars = value.chars().collect::<Vec<_>>();
    for index in 0..chars.len().saturating_sub(3) {
        let candidate = chars[index..index + 4].iter().collect::<String>();
        let Ok(year) = candidate.parse::<i32>() else {
            continue;
        };
        if (1900..=2100).contains(&year) {
            return Some(year);
        }
    }
    None
}

#[allow(non_snake_case)]
fn linkTypeLabelForBackend(link_type: &str) -> String {
    match link_type {
        "share115" => "115".to_string(),
        "quark" => "夸克".to_string(),
        "baidu" => "百度".to_string(),
        "aliyun" => "阿里云".to_string(),
        "xunlei" => "迅雷".to_string(),
        "uc" => "UC".to_string(),
        "123" => "123".to_string(),
        "guangya" => "光亚".to_string(),
        other => other.to_string(),
    }
}

#[utoipa::path(post, path = "/api/v2/catalog/transfer-plan", tag = "catalog", request_body = CatalogTransferPlanRequest, responses((status = 200, body = CatalogTransferPlanResponse)))]
pub async fn catalog_transfer_plan(
    Json(req): Json<CatalogTransferPlanRequest>,
) -> AppResult<Json<CatalogTransferPlanResponse>> {
    Ok(Json(build_transfer_plan(req)?))
}

#[utoipa::path(post, path = "/api/v2/catalog/transfer/execute", tag = "catalog", request_body = CatalogTransferExecuteRequest, responses((status = 200, body = TaskRun)))]
pub async fn catalog_transfer_execute(
    State(state): State<AppState>,
    Json(req): Json<CatalogTransferExecuteRequest>,
) -> AppResult<Json<TaskRun>> {
    let plans = build_execute_plans(&req)?;
    let target = merged_execute_target(&req);
    let (target_cid, target_lib) = resolve_catalog_target_cid(&state.pool, &target).await?;
    let cookie =
        c115::require_c115_cookie(config_store::get_string(&state.pool, C115_COOKIE_KEY).await?)?;
    let (api_base, site_base) = c115_base_urls(&state.pool).await?;
    let label = catalog_transfer_task_label(&plans, &target_cid);
    let params = serde_json::to_value(&req).unwrap_or_else(|_| json!({}));
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "catalog_transfer_execute",
        &label,
        plans.len() as i64,
        "manual",
        params,
    )
    .await?;
    spawn_catalog_transfer_execute(
        state,
        CatalogTransferExecution {
            id: task.id,
            cookie,
            api_base,
            site_base,
            plans,
            target_cid,
            target_lib,
        },
    );
    Ok(Json(task))
}

fn build_execute_plans(
    req: &CatalogTransferExecuteRequest,
) -> AppResult<Vec<CatalogTransferPlanResponse>> {
    execute_plan_requests(req)?
        .into_iter()
        .map(build_transfer_plan)
        .collect()
}

fn execute_plan_requests(
    req: &CatalogTransferExecuteRequest,
) -> AppResult<Vec<CatalogTransferPlanRequest>> {
    let target = merged_execute_target(req);
    let mut requests = Vec::new();
    let batch_label = (req.items.len() == 1).then(|| req.label.clone()).flatten();

    for item in &req.items {
        requests.push(CatalogTransferPlanRequest {
            item: Some(item.clone()),
            link: None,
            label: batch_label.clone(),
            name: None,
            sheet: None,
            is_pkg: None,
            link_type: req.link_type.clone(),
            share: req.share.clone(),
            rc: req.rc.clone(),
            pwd: req.pwd.clone(),
            lib: target.lib.clone(),
            cid: target.cid.clone(),
        });
    }

    if req.item.is_some() || req.link.as_deref().and_then(non_empty_trimmed).is_some() {
        requests.push(CatalogTransferPlanRequest {
            item: req.item.clone(),
            link: req.link.clone(),
            label: req.label.clone(),
            name: req.name.clone(),
            sheet: req.sheet.clone(),
            is_pkg: req.is_pkg,
            link_type: req.link_type.clone(),
            share: req.share.clone(),
            rc: req.rc.clone(),
            pwd: req.pwd.clone(),
            lib: target.lib.clone(),
            cid: target.cid.clone(),
        });
    }

    if requests.is_empty() {
        return Err(AppError::BadRequest(
            "catalog transfer execute requires item(s) or link".to_string(),
        ));
    }
    Ok(requests)
}

fn merged_execute_target(req: &CatalogTransferExecuteRequest) -> CatalogTransferTarget {
    CatalogTransferTarget {
        lib: first_clean([
            req.target.as_ref().and_then(|target| target.lib.clone()),
            req.lib.clone(),
        ]),
        cid: first_clean([
            req.target.as_ref().and_then(|target| target.cid.clone()),
            req.cid.clone(),
        ]),
    }
}

async fn resolve_catalog_target_cid(
    pool: &sqlx::PgPool,
    target: &CatalogTransferTarget,
) -> AppResult<(String, Option<String>)> {
    if let Some(cid) = target.cid.as_deref().and_then(non_empty_trimmed) {
        return Ok((
            c115::validate_target_cid(cid)?,
            target
                .lib
                .as_deref()
                .and_then(non_empty_trimmed)
                .map(ToString::to_string),
        ));
    }

    let lib = target
        .lib
        .as_deref()
        .and_then(non_empty_trimmed)
        .ok_or_else(|| AppError::BadRequest("未指定目标库或 cid".to_string()))?;
    let map = catalog_cid_map(pool).await?;
    let cid = map
        .get(lib)
        .ok_or_else(|| AppError::BadRequest(format!("库「{lib}」没配 115 cid,去设置页填")))?;
    Ok((c115::validate_target_cid(cid)?, Some(lib.to_string())))
}

async fn catalog_cid_map(pool: &sqlx::PgPool) -> AppResult<BTreeMap<String, String>> {
    let Some(value) = config_store::get_raw(pool, C115_CID_MAP_KEY).await? else {
        return Ok(BTreeMap::new());
    };
    Ok(value
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .and_then(non_empty_trimmed)
                        .map(|cid| (key.clone(), cid.to_string()))
                })
                .collect()
        })
        .unwrap_or_default())
}

async fn c115_base_urls(pool: &sqlx::PgPool) -> AppResult<(String, String)> {
    let api_base = config_store::get_string_or(pool, C115_API_BASE_URL_KEY, c115::C115_API).await?;
    let site_base =
        config_store::get_string_or(pool, C115_SITE_BASE_URL_KEY, c115::C115_SITE).await?;
    Ok((api_base, site_base))
}

struct CatalogTransferExecution {
    id: Uuid,
    cookie: String,
    api_base: String,
    site_base: String,
    plans: Vec<CatalogTransferPlanResponse>,
    target_cid: String,
    target_lib: Option<String>,
}

fn spawn_catalog_transfer_execute(state: AppState, execution: CatalogTransferExecution) {
    tokio::spawn(async move {
        let CatalogTransferExecution {
            id,
            cookie,
            api_base,
            site_base,
            plans,
            target_cid,
            target_lib,
        } = execution;
        let Ok(_permit) = state.clouddrive_slot.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, id, "115 任务串行锁不可用", None).await;
            return;
        };
        if tasks::cancel_requested(&state.pool, id).await {
            let _ = tasks::finish_cancelled(&state.pool, id).await;
            return;
        }
        let _ = tasks::mark_running(&state.pool, id, "准备执行 115 转存/离线...").await;

        let client = C115Client::new_with_site(api_base, site_base, cookie, state.http.clone());
        let total = plans.len();
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut items = Vec::with_capacity(total);

        for (index, plan) in plans.iter().enumerate() {
            if tasks::cancel_requested(&state.pool, id).await {
                let _ = tasks::finish_cancelled(&state.pool, id).await;
                return;
            }
            let label = plan_display_label(plan);
            let _ = tasks::set_progress(
                &state.pool,
                id,
                index as i64,
                &format!(
                    "执行 {}/{}: {}",
                    index + 1,
                    total,
                    truncate_label(&label, 48)
                ),
            )
            .await;

            let item = match execute_catalog_transfer_plan(
                &client,
                plan,
                &target_cid,
                target_lib.as_deref(),
            )
            .await
            {
                Ok(response) => {
                    succeeded += 1;
                    json!({
                        "index": index,
                        "ok": true,
                        "action": &plan.action,
                        "link_type": &plan.link_type,
                        "label": &plan.label,
                        "target": &plan.target,
                        "response": response,
                    })
                }
                Err(error) => {
                    failed += 1;
                    json!({
                        "index": index,
                        "ok": false,
                        "action": &plan.action,
                        "link_type": &plan.link_type,
                        "label": &plan.label,
                        "target": &plan.target,
                        "error": error,
                    })
                }
            };
            items.push(item);
            let _ = tasks::set_progress(
                &state.pool,
                id,
                (index + 1) as i64,
                &format!("已处理 {}/{}", index + 1, total),
            )
            .await;
        }

        let result = json!({
            "ok": failed == 0,
            "total": total,
            "succeeded": succeeded,
            "failed": failed,
            "target": {
                "cid": target_cid,
                "lib": target_lib,
            },
            "items": items,
        });
        let status_text = if failed == 0 {
            "完成".to_string()
        } else {
            format!("完成，{failed} 项失败")
        };
        let _ = tasks::finish_done_with_message(&state.pool, id, &status_text, result).await;
    });
}

async fn execute_catalog_transfer_plan(
    client: &C115Client,
    plan: &CatalogTransferPlanResponse,
    target_cid: &str,
    target_lib: Option<&str>,
) -> Result<Value, String> {
    match &plan.action {
        CatalogTransferAction::SaveShare => {
            let save = plan
                .save
                .as_ref()
                .ok_or_else(|| "save plan payload is missing".to_string())?;
            let response = client
                .save_to_cid(
                    C115SaveRequest {
                        url: save.payload.url.clone(),
                        pwd: save.payload.pwd.clone(),
                        lib: save.payload.lib.clone(),
                        cid: save.payload.cid.clone(),
                        label: save.payload.label.clone(),
                        file_ids: None,
                    },
                    target_cid.to_string(),
                    target_lib.map(ToString::to_string),
                )
                .await
                .map_err(|err| err.to_string())?;
            if response.ok {
                Ok(serde_json::to_value(response).unwrap_or_else(|_| json!({})))
            } else {
                Err(response.msg)
            }
        }
        CatalogTransferAction::OfflineDownload => {
            let offline = plan
                .offline
                .as_ref()
                .ok_or_else(|| "offline plan payload is missing".to_string())?;
            let response = client
                .offline_add(
                    C115OfflineRequest {
                        url: offline.payload.url.clone(),
                        lib: offline.payload.lib.clone(),
                        cid: offline.payload.cid.clone(),
                        label: offline.payload.label.clone(),
                    },
                    target_cid.to_string(),
                    target_lib.map(ToString::to_string),
                )
                .await
                .map_err(|err| err.to_string())?;
            if response.ok {
                Ok(serde_json::to_value(response).unwrap_or_else(|_| json!({})))
            } else {
                Err(response.msg)
            }
        }
        CatalogTransferAction::Unsupported => Err(plan
            .unsupported
            .as_ref()
            .map(|unsupported| unsupported.reason.clone())
            .unwrap_or_else(|| "catalog link type is not supported".to_string())),
    }
}

fn catalog_transfer_task_label(plans: &[CatalogTransferPlanResponse], target_cid: &str) -> String {
    if plans.len() == 1 {
        return format!(
            "目录转存: {} -> cid={target_cid}",
            truncate_label(&plan_display_label(&plans[0]), 96)
        );
    }
    format!("目录转存: {} 项 -> cid={target_cid}", plans.len())
}

fn plan_display_label(plan: &CatalogTransferPlanResponse) -> String {
    plan.label
        .clone()
        .or_else(|| {
            plan.save
                .as_ref()
                .map(|save| save.payload.url.clone())
                .or_else(|| {
                    plan.offline
                        .as_ref()
                        .map(|offline| offline.payload.url.clone())
                })
        })
        .or_else(|| {
            plan.unsupported
                .as_ref()
                .map(|unsupported| unsupported.link.clone())
        })
        .unwrap_or_else(|| plan.link_type.clone())
}

fn truncate_label(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut out = value
        .chars()
        .take(limit.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

#[derive(Debug, Clone, Copy)]
enum DuplicateKind {
    Link,
    Name,
}

async fn duplicate_groups(
    pool: &sqlx::PgPool,
    kind: DuplicateKind,
    limit: i64,
) -> AppResult<Vec<CatalogDuplicateGroup>> {
    let key_column = match kind {
        DuplicateKind::Link => "link",
        DuplicateKind::Name => "name",
    };
    let sql = format!(
        "WITH groups AS (
            SELECT {key_column} AS key, COUNT(*)::bigint AS count
            FROM catalog_items
            WHERE btrim({key_column}) <> ''
            GROUP BY {key_column}
            HAVING COUNT(*) > 1
            ORDER BY COUNT(*) DESC, {key_column}
            LIMIT $1
        )
        SELECT
            g.key,
            g.count,
            ARRAY(
                SELECT DISTINCT c.link_type
                FROM catalog_items c
                WHERE c.{key_column} = g.key
                ORDER BY c.link_type
            ) AS link_types,
            ARRAY(
                SELECT DISTINCT c.name
                FROM catalog_items c
                WHERE c.{key_column} = g.key
                ORDER BY c.name
                LIMIT 3
            ) AS sample_names,
            ARRAY(
                SELECT DISTINCT c.sheet
                FROM catalog_items c
                WHERE c.{key_column} = g.key
                ORDER BY c.sheet
                LIMIT 3
            ) AS sample_sheets,
            ARRAY(
                SELECT DISTINCT c.link
                FROM catalog_items c
                WHERE c.{key_column} = g.key
                ORDER BY c.link
                LIMIT 3
            ) AS sample_links
        FROM groups g
        ORDER BY g.count DESC, g.key"
    );
    Ok(sqlx::query_as::<
        _,
        (
            String,
            i64,
            Vec<String>,
            Vec<String>,
            Vec<String>,
            Vec<String>,
        ),
    >(&sql)
    .bind(limit)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(
        |(key, count, link_types, sample_names, sample_sheets, sample_links)| {
            CatalogDuplicateGroup {
                key,
                count,
                link_types,
                sample_names,
                sample_sheets,
                sample_links,
            }
        },
    )
    .collect())
}

pub fn build_transfer_plan(
    req: CatalogTransferPlanRequest,
) -> AppResult<CatalogTransferPlanResponse> {
    let item_link = req.item.as_ref().map(|item| item.link.clone());
    let link = first_clean([req.link.clone(), item_link])
        .ok_or_else(|| AppError::BadRequest("catalog transfer plan requires link".to_string()))?;

    let item_link_type = req.item.as_ref().and_then(|item| item.link_type.clone());
    let link_type =
        normalize_link_type(first_clean([req.link_type.clone(), item_link_type]), &link);
    let transfer = link_type == "share115";

    let target = CatalogTransferTarget {
        lib: first_clean([req.lib.clone()]),
        cid: first_clean([req.cid.clone()]),
    };
    if target.lib.is_none() && target.cid.is_none() {
        return Err(AppError::BadRequest(
            "catalog transfer plan requires lib or cid".to_string(),
        ));
    }

    let item_name = req.item.as_ref().and_then(|item| item.name.clone());
    let label = first_clean([req.label.clone(), req.name.clone(), item_name]);
    let is_pkg = req
        .is_pkg
        .or_else(|| req.item.as_ref().and_then(|item| item.is_pkg))
        .unwrap_or(false);

    let item_share = req.item.as_ref().and_then(|item| item.share.clone());
    let item_rc = req.item.as_ref().and_then(|item| item.rc.clone());
    let (parsed_share, parsed_rc) = parse_share(&link);
    let share = first_clean([req.share.clone(), item_share, parsed_share]);
    let receive_code = first_clean([req.rc.clone(), req.pwd.clone(), item_rc, parsed_rc]);

    match link_type.as_str() {
        "share115" => {
            let Some(share) = share else {
                return Ok(unsupported_plan(
                    link_type,
                    transfer,
                    target,
                    label,
                    link,
                    is_pkg,
                    "115 share link is missing a share code",
                ));
            };
            Ok(CatalogTransferPlanResponse {
                ok: true,
                action: CatalogTransferAction::SaveShare,
                link_type,
                transfer,
                is_pkg,
                target: CatalogTransferTarget {
                    lib: target.lib.clone(),
                    cid: target.cid.clone(),
                },
                label: label.clone(),
                save: Some(CatalogC115SavePlan {
                    endpoint: "/api/v2/c115/save".to_string(),
                    method: "POST".to_string(),
                    share,
                    receive_code: receive_code.clone(),
                    payload: CatalogC115SavePayload {
                        url: link,
                        pwd: receive_code,
                        lib: target.lib,
                        cid: target.cid,
                        label,
                    },
                }),
                offline: None,
                unsupported: None,
            })
        }
        "magnet" | "ed2k" => Ok(CatalogTransferPlanResponse {
            ok: true,
            action: CatalogTransferAction::OfflineDownload,
            link_type: link_type.clone(),
            transfer,
            is_pkg,
            target: CatalogTransferTarget {
                lib: target.lib.clone(),
                cid: target.cid.clone(),
            },
            label: label.clone(),
            save: None,
            offline: Some(CatalogC115OfflinePlan {
                endpoint: "/api/v2/c115/offline".to_string(),
                method: "POST".to_string(),
                protocol: link_type,
                payload: CatalogC115OfflinePayload {
                    url: link,
                    lib: target.lib,
                    cid: target.cid,
                    label,
                },
            }),
            unsupported: None,
        }),
        _ => Ok(unsupported_plan(
            link_type,
            transfer,
            target,
            label,
            link,
            is_pkg,
            "catalog link type is not supported by 115 save/offline",
        )),
    }
}

pub fn split_terms(q: &str) -> Vec<String> {
    q.split_whitespace()
        .take(6)
        .map(|s| s.to_string())
        .collect()
}

fn escape_like(term: &str) -> String {
    term.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub fn infer_type(link: &str) -> &'static str {
    let lower = link.trim().to_ascii_lowercase();
    if lower.starts_with("magnet:") {
        "magnet"
    } else if lower.starts_with("ed2k:") {
        "ed2k"
    } else if lower.contains("/s/")
        && (lower.contains("115cdn.com")
            || lower.contains("115.com")
            || lower.contains("anxia.com"))
    {
        "share115"
    } else {
        "other"
    }
}

pub fn parse_share(link: &str) -> (Option<String>, Option<String>) {
    let link = link.trim();
    let share = link
        .split("/s/")
        .nth(1)
        .and_then(|rest| {
            let token: String = rest
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric())
                .collect();
            (!token.is_empty()).then_some(token)
        })
        .filter(|s| !s.is_empty());
    let rc = link
        .split(['?', '&'])
        .find_map(|part| {
            let (k, v) = part.split_once('=')?;
            matches!(k, "password" | "pwd").then(|| {
                v.split(|ch: char| ch == '#' || ch == '&' || ch.is_whitespace())
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string()
            })
        })
        .filter(|v| !v.is_empty());
    (share, rc)
}

fn first_clean<const N: usize>(values: [Option<String>; N]) -> Option<String> {
    values.into_iter().find_map(clean)
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn normalize_link_type(raw: Option<String>, link: &str) -> String {
    match raw
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("share115") => "share115".to_string(),
        Some("magnet") => "magnet".to_string(),
        Some("ed2k") => "ed2k".to_string(),
        Some("other") => "other".to_string(),
        _ => infer_type(link).to_string(),
    }
}

fn unsupported_plan(
    link_type: String,
    transfer: bool,
    target: CatalogTransferTarget,
    label: Option<String>,
    link: String,
    is_pkg: bool,
    reason: &str,
) -> CatalogTransferPlanResponse {
    CatalogTransferPlanResponse {
        ok: false,
        action: CatalogTransferAction::Unsupported,
        link_type,
        transfer,
        is_pkg,
        target,
        label,
        save: None,
        offline: None,
        unsupported: Some(CatalogUnsupportedPlan {
            reason: reason.to_string(),
            link,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn parses_115_share_and_receive_code() {
        let (share, rc) = parse_share("https://115.com/s/abc123?password=xy9z#anchor");
        assert_eq!(share.as_deref(), Some("abc123"));
        assert_eq!(rc.as_deref(), Some("xy9z"));
        assert_eq!(infer_type(" magnet:?xt=urn:btih:abc"), "magnet");
        assert_eq!(infer_type("https://anxia.com/s/swabc"), "share115");
    }

    #[test]
    fn builds_tg_resource_search_url_from_root_or_api_page() {
        assert_eq!(
            tg_resource_search_url("http://gaotao.cc:8100")
                .unwrap()
                .as_str(),
            "http://gaotao.cc:8100/api/v1/search"
        );
        assert_eq!(
            tg_resource_search_url("http://gaotao.cc:8100/api")
                .unwrap()
                .as_str(),
            "http://gaotao.cc:8100/api/v1/search"
        );
    }

    #[test]
    fn maps_tg_115_result_to_transferable_catalog_item() {
        let item = tg_result_to_catalog_item(TgResourceResult {
            title: Some("莫离 (2026) 更新至40集 4K".to_string()),
            kind: Some("115".to_string()),
            disk_type: Some("115".to_string()),
            url: Some("https://115cdn.com/s/swssmy63nbi".to_string()),
            password: Some("8888".to_string()),
            source: Some("tg:leoziyuan".to_string()),
            source_channels: vec![],
        })
        .unwrap();

        assert_eq!(item.link_type, "share115");
        assert!(item.transfer);
        assert!(item.is_pkg);
        assert_eq!(item.share.as_deref(), Some("swssmy63nbi"));
        assert_eq!(item.rc.as_deref(), Some("8888"));
        assert_eq!(item.sheet, "tg:leoziyuan");
    }

    #[test]
    fn keeps_non_115_tg_result_as_unsupported_catalog_type() {
        let item = tg_result_to_catalog_item(TgResourceResult {
            title: Some("莫离 完结 4K".to_string()),
            kind: Some("quark".to_string()),
            disk_type: Some("quark".to_string()),
            url: Some("https://pan.quark.cn/s/a70d06351fb0".to_string()),
            password: None,
            source: None,
            source_channels: vec!["kfcfoodcourt".to_string(), "yunpanx".to_string()],
        })
        .unwrap();

        assert_eq!(item.link_type, "quark");
        assert!(!item.transfer);
        assert_eq!(item.sheet, "kfcfoodcourt,yunpanx");
    }

    #[test]
    fn parses_episode_spans_from_common_resource_titles() {
        let spans = extract_episode_spans("The.First.Jasmine.2026.S01E01-E40.2160p");
        assert_eq!(
            spans,
            vec![EpisodeSpan {
                season: Some(1),
                start: 1,
                end: 40
            }]
        );

        let spans = extract_episode_spans("The.First.Jasmine.2026.S01E05.2160p");
        assert_eq!(
            spans,
            vec![EpisodeSpan {
                season: Some(1),
                start: 5,
                end: 5
            }]
        );

        let spans = extract_episode_spans("莫离 (2026) 更新至40集 4K");
        assert_eq!(
            spans,
            vec![EpisodeSpan {
                season: Some(1),
                start: 1,
                end: 40
            }]
        );
    }

    #[test]
    fn recommendations_rank_missing_package_above_existing_single_episode() {
        let context = CatalogLibraryContextResponse {
            ok: true,
            query: "莫离".to_string(),
            total_matches: 1,
            truncated: false,
            summary: CatalogLibraryContextSummary {
                matched: true,
                max_episode: 8,
                episode_ranges: vec!["S01E1-8".to_string()],
                note: "库内已有匹配条目".to_string(),
                ..CatalogLibraryContextSummary::default()
            },
            items: Vec::new(),
            warnings: Vec::new(),
        };
        let mut items = vec![
            test_catalog_item("莫离 S01E05 2160p", false),
            test_catalog_item("莫离 S01E01-E40 2160p", true),
        ];

        apply_catalog_recommendations(&mut items, &context);

        assert_eq!(items[0].name, "莫离 S01E01-E40 2160p");
        let package_recommendation = items[0].recommendation.as_ref().unwrap();
        assert_eq!(package_recommendation.level, "best");
        assert!(package_recommendation.covers_missing);

        let single_recommendation = items[1].recommendation.as_ref().unwrap();
        assert_eq!(single_recommendation.level, "skip");
        assert!(single_recommendation.already_have);
    }

    fn test_catalog_item(name: &str, is_pkg: bool) -> CatalogItem {
        CatalogItem {
            name: name.to_string(),
            sheet: "TG Resource API".to_string(),
            link: format!("https://115cdn.com/s/{name}"),
            is_pkg,
            link_type: "share115".to_string(),
            transfer: true,
            share: Some(name.to_string()),
            rc: None,
            recommendation: None,
        }
    }

    #[tokio::test]
    async fn fetches_tg_resource_search_with_token_and_maps_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(String::new()));
        let captured_request = Arc::clone(&captured);
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            *captured_request.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = r#"{
                "code": 0,
                "message": "success",
                "data": {
                    "total": 129,
                    "limit": 3,
                    "offset": 0,
                    "has_more": true,
                    "query": "莫离",
                    "exact": false,
                    "sort": "resource",
                    "disk_types": [{"disk_type":"115","count":129}],
                    "results": [{
                        "title": "莫离 (2026) 更新至40集 4K",
                        "type": "115",
                        "disk_type": "115",
                        "url": "https://115cdn.com/s/swssmy63nbi?password=8888",
                        "password": "8888",
                        "source": "tg:leoziyuan",
                        "source_channels": ["leoziyuan"]
                    }]
                }
            }"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let response = fetch_tg_resource_search(
            &reqwest::Client::new(),
            &format!("http://{addr}"),
            Some("secret-token"),
            CatalogRemoteSearchQuery {
                q: "莫离".to_string(),
                limit: Some(3),
                offset: None,
                disk_type: Some("share115".to_string()),
                exact: None,
                sort: None,
            },
        )
        .await
        .unwrap();

        handle.await.unwrap();
        let request = captured.lock().unwrap().clone();
        assert!(request.starts_with("GET /api/v1/search?"), "{request}");
        assert!(request.contains("kw=%E8%8E%AB%E7%A6%BB"), "{request}");
        assert!(request.contains("disk_type=115"), "{request}");
        let lower_request = request.to_ascii_lowercase();
        assert!(
            lower_request.contains("authorization: bearer secret-token"),
            "{request}"
        );
        assert_eq!(response.total, 129);
        assert!(response.has_more);
        assert_eq!(response.disk_types[0].disk_type, "115");
        assert_eq!(response.items[0].link_type, "share115");
        assert_eq!(response.items[0].rc.as_deref(), Some("8888"));
    }
}
