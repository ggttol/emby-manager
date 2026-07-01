use crate::{
    catalog::{
        self, CatalogItem, CatalogRemoteSearchQuery, CatalogRemoteSearchResponse,
        CatalogTransferPlanItem, extract_episode_spans,
    },
    config_store,
    error::{AppError, AppResult},
    media_fs,
    state::AppState,
    tasks::{self, TaskRun},
    wizard::{self, AddNewItem, AddNewRequest, AddNewTarget},
};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use reqwest::Client;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{sync::Semaphore, task::JoinSet};
use uuid::Uuid;

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 10;
const MAX_REQUEST_TIMEOUT_SECS: u64 = 60;
pub const ZHUIGENG_SCAN_AIRING_CANCELLED: &str = "__zhuigeng_scan_airing_cancelled__";

#[derive(Debug, Clone)]
pub struct ZhuigengConfig {
    pub emby_base_url: String,
    pub emby_api_key: String,
    pub tmdb_base_url: String,
    pub tmdb_api_key: String,
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengStatusResponse {
    pub ok: bool,
    pub items: Vec<ZhuigengItem>,
    pub continuing: usize,
    pub ended: usize,
    pub total: usize,
    pub copy_text: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengItem {
    pub lib: String,
    pub name: String,
    pub id: Option<String>,
    pub folder: String,
    pub tmdb: String,
    pub tmdb_status: String,
    pub state: String,
    pub continuing: bool,
    pub ended: bool,
    pub local_count: usize,
    pub local_latest: Option<String>,
    pub local_latest_episode: Option<String>,
    pub last_episode_to_air: Option<TmdbEpisodeSummary>,
    pub next_episode_to_air: Option<TmdbEpisodeSummary>,
    pub behind: usize,
    pub behind_hint: Option<String>,
    pub resource_hint: Option<String>,
    pub err: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct TmdbEpisodeSummary {
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub air_date: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengScanAiringResponse {
    pub ok: bool,
    pub total: usize,
    pub ok_count: usize,
    pub error_count: usize,
    pub new_count: usize,
    pub results: Vec<ZhuigengScanAiringRow>,
    pub copy_text: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengScanAiringRow {
    pub lib: String,
    pub series: Option<String>,
    pub name: String,
    pub id: Option<String>,
    pub tmdb: String,
    pub tmdb_status: String,
    pub keyword: String,
    pub status: String,
    pub behind: usize,
    pub hint: Option<String>,
    pub ok: bool,
    pub matched: usize,
    pub new_count: usize,
    pub attention: Vec<String>,
    pub error: Option<String>,
    pub err: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengGapsSummaryResponse {
    pub ok: bool,
    pub items: Vec<ZhuigengGapRow>,
    pub total: usize,
    pub copy_text: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengGapsSummaryTaskResult {
    pub ok: bool,
    pub items: Vec<ZhuigengGapRow>,
    pub total: usize,
    pub copy_text: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengGapRow {
    pub lib: String,
    pub name: String,
    pub id: Option<String>,
    pub tmdb: String,
    pub fmt: String,
    pub behind: usize,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengWorkbenchResponse {
    pub ok: bool,
    pub status: ZhuigengStatusResponse,
    pub rows: Vec<ZhuigengWorkbenchRow>,
    pub counts: ZhuigengWorkbenchCounts,
    pub copy_text: String,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct ZhuigengWorkbenchCounts {
    pub total: usize,
    pub healthy_airing: usize,
    pub update_needed: usize,
    pub archive_ready: usize,
    pub complete_after_update: usize,
    pub metadata_error: usize,
    pub target_error: usize,
    pub unknown: usize,
    pub behind_total: usize,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengWorkbenchRow {
    pub item: ZhuigengItem,
    pub lane: ZhuigengWorkbenchLane,
    pub priority: i32,
    pub action: String,
    pub resource_query: Option<String>,
    pub archiveable: bool,
    pub updateable: bool,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZhuigengWorkbenchLane {
    HealthyAiring,
    UpdateNeeded,
    ArchiveReady,
    CompleteAfterUpdate,
    MetadataError,
    TargetError,
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ZhuigengItemRef {
    pub lib: String,
    pub name: String,
    pub id: Option<String>,
    pub folder: Option<String>,
    pub tmdb: Option<String>,
    pub behind: Option<usize>,
    pub resource_hint: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ZhuigengResourcePlanRequest {
    pub item: ZhuigengItemRef,
    pub limit: Option<i64>,
    pub exact: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ZhuigengResourcePlanResponse {
    pub ok: bool,
    pub item: ZhuigengItemRef,
    pub query: String,
    pub missing_hint: Option<String>,
    pub fallback_queries: Vec<String>,
    pub search: CatalogRemoteSearchResponse,
    pub recommended: Option<CatalogItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ZhuigengUpdateExecuteRequest {
    pub item: ZhuigengItemRef,
    pub candidate: CatalogTransferPlanItem,
    pub target: Option<AddNewTarget>,
    pub delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ZhuigengArchiveExecuteRequest {
    pub to_lib: String,
    #[serde(default)]
    pub items: Vec<ZhuigengItemRef>,
    pub on_conflict: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ZhuigengArchiveExecuteResponse {
    pub ok: bool,
    pub total: usize,
    pub tasks: Vec<TaskRun>,
}

#[derive(Debug, Deserialize)]
struct EmbyVirtualFolderLite {
    #[serde(rename = "ItemId")]
    item_id: Option<String>,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "CollectionType")]
    collection_type: Option<String>,
    #[serde(rename = "Locations", default)]
    locations: Vec<String>,
    #[serde(rename = "LibraryOptions")]
    library_options: Option<EmbyLibraryOptionsLite>,
}

#[derive(Debug, Deserialize)]
struct EmbyLibraryOptionsLite {
    #[serde(rename = "PathInfos", default)]
    path_infos: Vec<EmbyPathInfoLite>,
}

#[derive(Debug, Deserialize)]
struct EmbyPathInfoLite {
    #[serde(rename = "Path")]
    path: Option<String>,
}

#[derive(Debug, Clone)]
struct ZhuigengLibrary {
    id: String,
    name: String,
    library_type: String,
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EmbyItemsPageLite {
    #[serde(rename = "Items", default)]
    items: Vec<EmbySeriesLite>,
}

#[derive(Debug, Deserialize)]
struct EmbySeriesLite {
    #[serde(rename = "Id")]
    id: Option<String>,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Path")]
    path: Option<String>,
    #[serde(rename = "ProviderIds", default)]
    provider_ids: BTreeMap<String, Value>,
    #[serde(rename = "Status")]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EmbyEpisodesPageLite {
    #[serde(rename = "Items", default)]
    items: Vec<EmbyEpisodeLite>,
}

#[derive(Debug, Deserialize)]
struct EmbyEpisodeLite {
    #[serde(rename = "ParentIndexNumber")]
    parent_index_number: Option<i32>,
    #[serde(rename = "IndexNumber")]
    index_number: Option<i32>,
    #[serde(rename = "LocationType")]
    location_type: Option<String>,
    #[serde(rename = "PremiereDate")]
    premiere_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TmdbTvResponse {
    status: Option<String>,
    last_episode_to_air: Option<TmdbEpisodeSummary>,
    next_episode_to_air: Option<TmdbEpisodeSummary>,
}

#[derive(Debug)]
struct LocalEpisodeSummary {
    count: usize,
    latest_date: Option<String>,
    latest_episode: Option<String>,
    have_by_season: BTreeMap<Option<i32>, Vec<i32>>,
    virtual_by_season: BTreeMap<Option<i32>, Vec<i32>>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/zhuigeng", get(status))
        .route("/api/v2/zhuigeng/workbench", get(workbench))
        .route("/api/v2/zhuigeng/resource-plan", post(resource_plan))
        .route("/api/v2/zhuigeng/update/execute", post(update_execute))
        .route("/api/v2/zhuigeng/archive/execute", post(archive_execute))
        .route("/api/v2/zhuigeng/scan-airing", post(scan_airing))
        .route("/api/v2/zhuigeng/scan_airing", post(scan_airing))
        .route("/api/v2/zhuigeng/gaps-summary", post(gaps_summary))
        .route("/api/v2/zhuigeng/gaps_summary", post(gaps_summary))
}

#[utoipa::path(get, path = "/api/v2/zhuigeng", tag = "zhuigeng", responses((status = 200, body = ZhuigengStatusResponse)))]
pub async fn status(State(state): State<AppState>) -> AppResult<Json<ZhuigengStatusResponse>> {
    let config = zhuigeng_config_from_state(&state).await?;
    Ok(Json(
        zhuigeng_status_with_config(config, state.http.clone()).await?,
    ))
}

#[utoipa::path(get, path = "/api/v2/zhuigeng/workbench", tag = "zhuigeng", responses((status = 200, body = ZhuigengWorkbenchResponse)))]
pub async fn workbench(
    State(state): State<AppState>,
) -> AppResult<Json<ZhuigengWorkbenchResponse>> {
    Ok(Json(zhuigeng_workbench_for_state(&state).await?))
}

pub async fn zhuigeng_workbench_for_state(
    state: &AppState,
) -> AppResult<ZhuigengWorkbenchResponse> {
    let config = zhuigeng_config_from_state(state).await?;
    let status = zhuigeng_status_with_config(config, state.http.clone()).await?;
    let status = enrich_zhuigeng_status_with_catalog(state, status).await;
    Ok(build_zhuigeng_workbench(status))
}

#[utoipa::path(post, path = "/api/v2/zhuigeng/resource-plan", tag = "zhuigeng", request_body = ZhuigengResourcePlanRequest, responses((status = 200, body = ZhuigengResourcePlanResponse)))]
pub async fn resource_plan(
    State(state): State<AppState>,
    Json(req): Json<ZhuigengResourcePlanRequest>,
) -> AppResult<Json<ZhuigengResourcePlanResponse>> {
    Ok(Json(zhuigeng_resource_plan_for_state(&state, req).await?))
}

#[utoipa::path(post, path = "/api/v2/zhuigeng/update/execute", tag = "zhuigeng", request_body = ZhuigengUpdateExecuteRequest, responses((status = 200, body = TaskRun)))]
pub async fn update_execute(
    State(state): State<AppState>,
    Json(req): Json<ZhuigengUpdateExecuteRequest>,
) -> AppResult<Json<TaskRun>> {
    let task = zhuigeng_update_execute_for_state(state, req).await?;
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/zhuigeng/archive/execute", tag = "zhuigeng", request_body = ZhuigengArchiveExecuteRequest, responses((status = 200, body = ZhuigengArchiveExecuteResponse)))]
pub async fn archive_execute(
    State(state): State<AppState>,
    Json(req): Json<ZhuigengArchiveExecuteRequest>,
) -> AppResult<Json<ZhuigengArchiveExecuteResponse>> {
    Ok(Json(zhuigeng_archive_execute_for_state(state, req).await?))
}

#[utoipa::path(post, path = "/api/v2/zhuigeng/scan-airing", tag = "zhuigeng", responses((status = 200, body = TaskRun)))]
pub async fn scan_airing(State(state): State<AppState>) -> AppResult<Json<tasks::TaskRun>> {
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "zhuigeng_scan_airing",
        "追更扫描在更剧",
        1,
        "api",
        serde_json::json!({}),
    )
    .await?;
    spawn_scan_airing_task(state, task.id);
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/zhuigeng/gaps-summary", tag = "zhuigeng", responses((status = 200, body = TaskRun)))]
pub async fn gaps_summary(State(state): State<AppState>) -> AppResult<Json<tasks::TaskRun>> {
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "zhuigeng_gaps_summary",
        "追更缺集汇总",
        1,
        "api",
        serde_json::json!({}),
    )
    .await?;
    spawn_gaps_summary_task(state, task.id);
    Ok(Json(task))
}

fn spawn_scan_airing_task(state: AppState, task_id: Uuid) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, task_id, "任务并发槽不可用", None).await;
            return;
        };
        let _ = tasks::mark_running(&state.pool, task_id, "开始追更扫描").await;
        match zhuigeng_scan_airing_for_state(&state, Some(task_id)).await {
            Ok(result) => {
                let _ = tasks::finish_done_with_message(
                    &state.pool,
                    task_id,
                    &format!("追更扫描完成: {}/{}", result.ok_count, result.total),
                    serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
                )
                .await;
            }
            Err(AppError::Conflict(message)) if message == ZHUIGENG_SCAN_AIRING_CANCELLED => {}
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, task_id, &err.to_string(), None).await;
            }
        }
    });
}

fn spawn_gaps_summary_task(state: AppState, task_id: Uuid) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, task_id, "任务并发槽不可用", None).await;
            return;
        };
        let _ = tasks::mark_running(&state.pool, task_id, "汇总追更缺集").await;
        match zhuigeng_gaps_summary_for_state(&state, Some(task_id)).await {
            Ok(result) => {
                let _ = tasks::finish_done_with_message(
                    &state.pool,
                    task_id,
                    &format!("追更缺集汇总完成: {}", result.total),
                    serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
                )
                .await;
            }
            Err(AppError::Conflict(message)) if message == ZHUIGENG_SCAN_AIRING_CANCELLED => {}
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, task_id, &err.to_string(), None).await;
            }
        }
    });
}

pub async fn zhuigeng_status_with_config(
    config: ZhuigengConfig,
    http: Client,
) -> AppResult<ZhuigengStatusResponse> {
    config.validate()?;
    let emby = ZhuigengEmbyClient::new(
        config.emby_base_url.clone(),
        config.emby_api_key.clone(),
        config.request_timeout,
        http.clone(),
    );
    let tmdb = config.has_tmdb_config().then(|| {
        TmdbClient::new(
            config.tmdb_base_url.clone(),
            config.tmdb_api_key.clone(),
            config.request_timeout,
            http,
        )
    });

    let libraries = emby.zhuigeng_libraries().await?;
    let mut items = Vec::new();
    for library in libraries {
        let series = emby.series(&library.id).await?;
        for item in series {
            let name = item.name.clone().unwrap_or_else(|| "(无名)".to_string());
            let folder = folder_from_series_path(item.path.as_deref(), &library);
            let tmdb_id = provider_id(&item.provider_ids, "Tmdb").unwrap_or_default();
            let (local, episode_err) = match item.id.as_deref() {
                Some(series_id) => match emby.episodes(series_id).await {
                    Ok(episodes) => (summarize_local_episodes(&episodes), None),
                    Err(err) => (summarize_local_episodes(&[]), Some(err.to_string())),
                },
                None => (
                    summarize_local_episodes(&[]),
                    Some("Emby Series 缺少 Id，无法读取本地集数".to_string()),
                ),
            };

            let mut row = if let Some(tmdb) = tmdb.as_ref() {
                match validate_tmdb_id(&tmdb_id) {
                    Ok(()) => match tmdb.tv(&tmdb_id).await {
                        Ok(meta) => build_item_from_tmdb(
                            &library, &item, &name, folder, tmdb_id, local, meta,
                        ),
                        Err(err) => build_error_item(
                            &library,
                            &item,
                            &name,
                            folder,
                            tmdb_id,
                            local,
                            err.to_string(),
                        ),
                    },
                    Err(err) => build_error_item(
                        &library,
                        &item,
                        &name,
                        folder,
                        tmdb_id,
                        local,
                        err.to_string(),
                    ),
                }
            } else {
                build_item_from_emby_status(&library, &item, &name, folder, tmdb_id, local)
            };
            if let Some(err) = episode_err {
                row.err = match row.err {
                    Some(existing) => Some(format!("{existing}; {err}")),
                    None => Some(err),
                };
            }
            items.push(row);
        }
    }

    items.sort_by(|left, right| {
        right
            .continuing
            .cmp(&left.continuing)
            .then_with(|| right.behind.cmp(&left.behind))
            .then_with(|| left.name.cmp(&right.name))
    });
    let continuing = items.iter().filter(|item| item.continuing).count();
    let ended = items.iter().filter(|item| item.ended).count();
    let copy_text = build_copy_text(&items, true);
    Ok(ZhuigengStatusResponse {
        ok: true,
        total: items.len(),
        items,
        continuing,
        ended,
        copy_text,
    })
}

pub async fn zhuigeng_scan_airing_with_config(
    config: ZhuigengConfig,
    http: Client,
) -> AppResult<ZhuigengScanAiringResponse> {
    let status = zhuigeng_status_with_config(config, http).await?;
    let results = status
        .items
        .iter()
        .filter(|item| item.continuing)
        .map(|item| ZhuigengScanAiringRow {
            lib: item.lib.clone(),
            series: item.id.clone(),
            name: item.name.clone(),
            id: item.id.clone(),
            tmdb: item.tmdb.clone(),
            tmdb_status: item.tmdb_status.clone(),
            keyword: item.name.clone(),
            status: "planned".to_string(),
            behind: item.behind,
            hint: item.behind_hint.clone(),
            ok: item.err.is_none(),
            matched: 0,
            new_count: 0,
            attention: Vec::new(),
            error: item.err.clone(),
            err: item.err.clone(),
        })
        .collect::<Vec<_>>();
    let ok_count = results.iter().filter(|row| row.ok).count();
    let error_count = results.len().saturating_sub(ok_count);
    Ok(ZhuigengScanAiringResponse {
        ok: true,
        total: results.len(),
        ok_count,
        error_count,
        new_count: 0,
        results,
        copy_text: status.copy_text,
        note: "已生成 continuing 追更执行计划；缺少 AppState 时不触发 STRM 文件扫描".to_string(),
    })
}

pub async fn zhuigeng_scan_airing_for_state(
    state: &AppState,
    task_id: Option<Uuid>,
) -> AppResult<ZhuigengScanAiringResponse> {
    let config = zhuigeng_config_from_state(state).await?;
    if let Some(task_id) = task_id {
        tasks::set_progress(&state.pool, task_id, 0, "读取 TMDb 追更状态").await?;
    }
    let mut response = zhuigeng_scan_airing_with_config(config, state.http.clone()).await?;
    let total = response.results.len().max(1) as i64;
    if let Some(task_id) = task_id {
        tasks::set_total(&state.pool, task_id, total).await?;
    }

    for index in 0..response.results.len() {
        check_zhuigeng_task_cancelled(state, task_id).await?;
        let label = format!(
            "扫描追更 {}/{}",
            response.results[index].lib, response.results[index].keyword
        );
        if let Some(task_id) = task_id {
            tasks::set_progress(&state.pool, task_id, index as i64, &label).await?;
        }

        {
            let _permit = state
                .clouddrive_slot
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| {
                    AppError::Conflict("CloudDrive 扫描槽已关闭，无法执行追更扫描".to_string())
                })?;
            execute_scan_airing_row(state, &mut response.results[index]);
        }

        if let Some(task_id) = task_id {
            tasks::set_progress(
                &state.pool,
                task_id,
                (index + 1) as i64,
                &format!("追更扫描 {}/{}", index + 1, response.results.len()),
            )
            .await?;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    response.ok_count = response.results.iter().filter(|row| row.ok).count();
    response.error_count = response.results.len().saturating_sub(response.ok_count);
    response.new_count = response.results.iter().map(|row| row.new_count).sum();
    response.ok = true;
    response.note = "已按 TMDb continuing 剧集串行用剧名扫描对应库并生成缺失 STRM".to_string();
    Ok(response)
}

pub fn execute_scan_airing_row(state: &AppState, row: &mut ZhuigengScanAiringRow) {
    match media_fs::generate_missing_strm_for_library(state, &row.lib, Some(&row.keyword), true) {
        Ok(result) => {
            row.matched = result.matched;
            row.new_count = result.new_count;
            row.attention = result.attention;
            row.status = if result.new_count > 0 {
                "generated".to_string()
            } else if result.matched == 0 {
                "not_found".to_string()
            } else {
                "done".to_string()
            };
            row.ok = true;
            row.error = None;
            row.err = None;
        }
        Err(err) => {
            let message = err.to_string();
            row.status = "error".to_string();
            row.ok = false;
            row.error = Some(message.clone());
            row.err = Some(message);
        }
    }
}

async fn check_zhuigeng_task_cancelled(state: &AppState, task_id: Option<Uuid>) -> AppResult<()> {
    let Some(task_id) = task_id else {
        return Ok(());
    };
    if tasks::cancel_requested(&state.pool, task_id).await {
        tasks::finish_cancelled(&state.pool, task_id).await?;
        return Err(AppError::Conflict(
            ZHUIGENG_SCAN_AIRING_CANCELLED.to_string(),
        ));
    }
    Ok(())
}

pub async fn zhuigeng_gaps_summary_with_config(
    config: ZhuigengConfig,
    http: Client,
) -> AppResult<ZhuigengGapsSummaryResponse> {
    let status = zhuigeng_status_with_config(config, http).await?;
    let items = status
        .items
        .iter()
        .filter(|item| item.continuing && item.behind > 0)
        .filter_map(|item| {
            Some(ZhuigengGapRow {
                lib: item.lib.clone(),
                name: item.name.clone(),
                id: item.id.clone(),
                tmdb: item.tmdb.clone(),
                fmt: item.resource_hint.clone()?,
                behind: item.behind,
            })
        })
        .collect::<Vec<_>>();
    let copy_text = items
        .iter()
        .map(|item| {
            if item.tmdb.trim().is_empty() {
                format!("求 {} — {}", item.name, item.fmt)
            } else {
                format!("求 {} [tmdb:{}] — {}", item.name, item.tmdb, item.fmt)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ZhuigengGapsSummaryResponse {
        ok: true,
        total: items.len(),
        items,
        copy_text,
    })
}

pub async fn zhuigeng_gaps_summary_for_state(
    state: &AppState,
    task_id: Option<Uuid>,
) -> AppResult<ZhuigengGapsSummaryTaskResult> {
    if let Some(task_id) = task_id {
        tasks::set_progress(&state.pool, task_id, 0, "读取追更状态").await?;
    }
    let config = zhuigeng_config_from_state(state).await?;
    let response = zhuigeng_gaps_summary_with_config(config, state.http.clone()).await?;
    if let Some(task_id) = task_id {
        check_zhuigeng_task_cancelled(state, Some(task_id)).await?;
        tasks::set_total(&state.pool, task_id, response.total.max(1) as i64).await?;
        tasks::set_progress(
            &state.pool,
            task_id,
            response.total as i64,
            &format!("已汇总 {} 条缺集", response.total),
        )
        .await?;
    }
    Ok(ZhuigengGapsSummaryTaskResult {
        ok: response.ok,
        items: response.items,
        total: response.total,
        copy_text: response.copy_text,
    })
}

async fn enrich_zhuigeng_status_with_catalog(
    state: &AppState,
    mut status: ZhuigengStatusResponse,
) -> ZhuigengStatusResponse {
    let semaphore = Arc::new(Semaphore::new(4));
    let mut jobs = JoinSet::new();
    for (index, item) in status.items.iter().enumerate() {
        if !should_probe_resource_catalog(item) {
            continue;
        }
        let item_ref = ZhuigengItemRef {
            lib: item.lib.clone(),
            name: item.name.clone(),
            id: item.id.clone(),
            folder: non_empty_trimmed(&item.folder).map(ToString::to_string),
            tmdb: non_empty_trimmed(&item.tmdb).map(ToString::to_string),
            behind: Some(item.behind),
            resource_hint: item.resource_hint.clone(),
        };
        let query = build_zhuigeng_resource_query(&item_ref);
        if query.trim().is_empty() {
            continue;
        }
        let state = state.clone();
        let series = item.name.clone();
        let semaphore = semaphore.clone();
        jobs.spawn(async move {
            let Ok(_permit) = semaphore.acquire_owned().await else {
                return (index, series, None);
            };
            match run_zhuigeng_catalog_search(&state, &query, 12, Some(false)).await {
                Ok(search) => (index, series, Some(search.items)),
                Err(err) => {
                    tracing::warn!(
                        series = %series,
                        error = %err,
                        "zhuigeng resource inference failed; keeping tmdb status"
                    );
                    (index, series, None)
                }
            }
        });
    }
    while let Some(joined) = jobs.join_next().await {
        if let Ok((index, _series, Some(items))) = joined
            && let Some(item) = status.items.get_mut(index)
        {
            apply_resource_episode_inference(item, &items);
        }
    }
    refresh_zhuigeng_status_counts(&mut status);
    status
}

fn should_probe_resource_catalog(item: &ZhuigengItem) -> bool {
    item.err.is_none()
        && item.local_count > 0
        && !item.name.trim().is_empty()
        && !item.lib.trim().is_empty()
        && (item.continuing || item.behind == 0 || item.last_episode_to_air.is_none())
}

fn refresh_zhuigeng_status_counts(status: &mut ZhuigengStatusResponse) {
    status.continuing = status.items.iter().filter(|item| item.continuing).count();
    status.ended = status.items.iter().filter(|item| item.ended).count();
    status.total = status.items.len();
    status.copy_text = build_copy_text(&status.items, false);
}

#[derive(Debug, Clone, Default)]
struct ResourceEpisodeInference {
    remote_max: i32,
    remote_title: Option<String>,
    complete_max: Option<i32>,
    complete_title: Option<String>,
}

fn apply_resource_episode_inference(item: &mut ZhuigengItem, resources: &[CatalogItem]) {
    let Some(inference) = infer_resource_episodes(resources) else {
        return;
    };
    let Some(local_max) = local_latest_episode_number(item) else {
        return;
    };
    if local_max <= 0 {
        return;
    }

    if inference.remote_max > local_max {
        let missing = ((local_max + 1)..=inference.remote_max)
            .map(|number| (Some(1), number))
            .collect::<Vec<_>>();
        let resource_hint = format_episode_segments(&missing);
        item.behind = missing.len();
        item.behind_hint = Some(format!(
            "资源侧发现更新到 E{}，本地到 E{} · {}",
            inference.remote_max, local_max, resource_hint
        ));
        item.resource_hint = Some(resource_hint);
        if inference.complete_max == Some(inference.remote_max) {
            mark_resource_inferred_ended(
                item,
                format!(
                    "资源侧检测到全 {} 集，本地还差 {} 集，补齐后可归档",
                    inference.remote_max, item.behind
                ),
            );
        } else {
            item.continuing = true;
            item.ended = false;
            item.state = "continuing".to_string();
        }
        return;
    }

    if let Some(complete_max) = inference.complete_max
        && complete_max <= local_max
    {
        let title = inference
            .complete_title
            .or(inference.remote_title)
            .unwrap_or_else(|| "资源标题".to_string());
        mark_resource_inferred_ended(
            item,
            format!("资源侧检测到全 {complete_max} 集，本地已到 E{local_max}，建议归档 · {title}"),
        );
        item.behind = 0;
        item.resource_hint = None;
    }
}

fn mark_resource_inferred_ended(item: &mut ZhuigengItem, hint: String) {
    item.continuing = false;
    item.ended = true;
    item.state = "ended_by_resource".to_string();
    item.behind_hint = Some(hint);
    if !item.tmdb_status.contains("资源推断完结") {
        item.tmdb_status = format!("{} · 资源推断完结", item.tmdb_status);
    }
}

fn infer_resource_episodes(resources: &[CatalogItem]) -> Option<ResourceEpisodeInference> {
    let mut inference = ResourceEpisodeInference::default();
    for resource in resources.iter().filter(|resource| resource.transfer) {
        let spans = extract_episode_spans(&resource.name);
        let Some(max_episode) = spans.iter().map(|span| span.end).max() else {
            continue;
        };
        if max_episode <= 0 {
            continue;
        }
        if max_episode > inference.remote_max {
            inference.remote_max = max_episode;
            inference.remote_title = Some(resource.name.clone());
        }
        let covers_from_start = spans
            .iter()
            .any(|span| span.start <= 1 && span.end == max_episode);
        if covers_from_start
            && resource_title_confirms_complete(&resource.name)
            && inference
                .complete_max
                .is_none_or(|current| max_episode > current)
        {
            inference.complete_max = Some(max_episode);
            inference.complete_title = Some(resource.name.clone());
        }
    }
    (inference.remote_max > 0 || inference.complete_max.is_some()).then_some(inference)
}

fn resource_title_confirms_complete(title: &str) -> bool {
    title.contains("完结")
        || title.contains("全集")
        || title.contains("全剧终")
        || title.contains("已完结")
        || ((title.contains('全') || title.contains('共')) && title.contains('集'))
}

fn local_latest_episode_number(item: &ZhuigengItem) -> Option<i32> {
    item.local_latest_episode
        .as_deref()
        .and_then(episode_number_from_label)
        .or_else(|| (item.local_count > 0).then_some(item.local_count as i32))
}

fn episode_number_from_label(label: &str) -> Option<i32> {
    let lower = label.to_ascii_lowercase();
    if let Some((_, tail)) = lower.rsplit_once('e') {
        let digits = tail
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if !digits.is_empty() {
            return digits.parse().ok();
        }
    }
    let digits = lower
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

pub fn build_zhuigeng_workbench(status: ZhuigengStatusResponse) -> ZhuigengWorkbenchResponse {
    let mut rows = status
        .items
        .iter()
        .cloned()
        .map(build_zhuigeng_workbench_row)
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.item.lib.cmp(&right.item.lib))
            .then_with(|| left.item.name.cmp(&right.item.name))
    });
    let counts = summarize_workbench_counts(&rows);
    let copy_text = status.copy_text.clone();
    let note = format!(
        "已按追更运营流分组: 需更新 {}，补齐后归档 {}，可归档 {}，异常 {}",
        counts.update_needed,
        counts.complete_after_update,
        counts.archive_ready,
        counts.metadata_error + counts.target_error
    );
    ZhuigengWorkbenchResponse {
        ok: status.ok,
        status,
        rows,
        counts,
        copy_text,
        note,
    }
}

fn build_zhuigeng_workbench_row(item: ZhuigengItem) -> ZhuigengWorkbenchRow {
    let mut blockers = Vec::new();
    if let Some(err) = item.err.as_deref().and_then(non_empty_trimmed) {
        blockers.push(err.to_string());
    }
    if item.tmdb.trim().is_empty() {
        blockers.push("缺少 TMDb Id，转存后可能无法自动刮削海报".to_string());
    }
    if item.lib.trim().is_empty() {
        blockers.push("缺少来源库，无法确定更新或归档目标".to_string());
    }
    if item.name.trim().is_empty() {
        blockers.push("缺少剧名，无法找资源".to_string());
    }

    let has_archive_target = item.id.as_deref().and_then(non_empty_trimmed).is_some()
        && !item.folder.as_str().trim().is_empty();
    let updateable = item.behind > 0
        && !item.name.as_str().trim().is_empty()
        && !item.lib.as_str().trim().is_empty();
    let lane = if item.err.is_some() {
        ZhuigengWorkbenchLane::MetadataError
    } else if item.lib.trim().is_empty() || item.name.trim().is_empty() {
        ZhuigengWorkbenchLane::TargetError
    } else if item.tmdb.trim().is_empty() {
        ZhuigengWorkbenchLane::MetadataError
    } else if item.ended && item.behind == 0 {
        if has_archive_target {
            ZhuigengWorkbenchLane::ArchiveReady
        } else {
            ZhuigengWorkbenchLane::TargetError
        }
    } else if item.ended && item.behind > 0 {
        ZhuigengWorkbenchLane::CompleteAfterUpdate
    } else if item.continuing && item.behind > 0 {
        ZhuigengWorkbenchLane::UpdateNeeded
    } else if item.continuing {
        ZhuigengWorkbenchLane::HealthyAiring
    } else {
        ZhuigengWorkbenchLane::Unknown
    };

    if matches!(lane, ZhuigengWorkbenchLane::TargetError) && !has_archive_target && item.ended {
        blockers.push("完结归档需要 Emby item id 和媒体文件夹名".to_string());
    }

    let resource_query = updateable.then(|| {
        build_zhuigeng_resource_query(&ZhuigengItemRef {
            lib: item.lib.clone(),
            name: item.name.clone(),
            id: item.id.clone(),
            folder: non_empty_trimmed(&item.folder).map(ToString::to_string),
            tmdb: non_empty_trimmed(&item.tmdb).map(ToString::to_string),
            behind: Some(item.behind),
            resource_hint: item.resource_hint.clone(),
        })
    });
    let archiveable = matches!(lane, ZhuigengWorkbenchLane::ArchiveReady);
    let priority = workbench_lane_priority(&lane) + (item.behind.min(99) as i32);
    let action = match lane {
        ZhuigengWorkbenchLane::HealthyAiring => "等待下一集".to_string(),
        ZhuigengWorkbenchLane::UpdateNeeded => "找资源并一条龙更新".to_string(),
        ZhuigengWorkbenchLane::ArchiveReady => "一键归档到完结库".to_string(),
        ZhuigengWorkbenchLane::CompleteAfterUpdate => "先补齐缺集，完成后归档".to_string(),
        ZhuigengWorkbenchLane::MetadataError => "先修复 TMDb/元数据".to_string(),
        ZhuigengWorkbenchLane::TargetError => "补齐路径或库配置".to_string(),
        ZhuigengWorkbenchLane::Unknown => "人工确认状态".to_string(),
    };

    ZhuigengWorkbenchRow {
        item,
        lane,
        priority,
        action,
        resource_query,
        archiveable,
        updateable,
        blockers,
    }
}

fn workbench_lane_priority(lane: &ZhuigengWorkbenchLane) -> i32 {
    match lane {
        ZhuigengWorkbenchLane::MetadataError => 900,
        ZhuigengWorkbenchLane::TargetError => 850,
        ZhuigengWorkbenchLane::CompleteAfterUpdate => 760,
        ZhuigengWorkbenchLane::UpdateNeeded => 720,
        ZhuigengWorkbenchLane::ArchiveReady => 640,
        ZhuigengWorkbenchLane::HealthyAiring => 200,
        ZhuigengWorkbenchLane::Unknown => 100,
    }
}

fn summarize_workbench_counts(rows: &[ZhuigengWorkbenchRow]) -> ZhuigengWorkbenchCounts {
    let mut counts = ZhuigengWorkbenchCounts {
        total: rows.len(),
        ..ZhuigengWorkbenchCounts::default()
    };
    for row in rows {
        counts.behind_total += row.item.behind;
        match row.lane {
            ZhuigengWorkbenchLane::HealthyAiring => counts.healthy_airing += 1,
            ZhuigengWorkbenchLane::UpdateNeeded => counts.update_needed += 1,
            ZhuigengWorkbenchLane::ArchiveReady => counts.archive_ready += 1,
            ZhuigengWorkbenchLane::CompleteAfterUpdate => counts.complete_after_update += 1,
            ZhuigengWorkbenchLane::MetadataError => counts.metadata_error += 1,
            ZhuigengWorkbenchLane::TargetError => counts.target_error += 1,
            ZhuigengWorkbenchLane::Unknown => counts.unknown += 1,
        }
    }
    counts
}

pub async fn zhuigeng_resource_plan_for_state(
    state: &AppState,
    req: ZhuigengResourcePlanRequest,
) -> AppResult<ZhuigengResourcePlanResponse> {
    let query = build_zhuigeng_resource_query(&req.item);
    if query.trim().is_empty() {
        return Err(AppError::BadRequest("剧名为空，无法找资源".to_string()));
    }
    let limit = req.limit.unwrap_or(24).clamp(1, 80);
    let mut fallback_queries = Vec::new();
    let mut search = run_zhuigeng_catalog_search(state, &query, limit, req.exact).await?;
    let name_query = req.item.name.trim().to_string();
    if search.items.is_empty() && !name_query.is_empty() && name_query != query {
        fallback_queries.push(name_query.clone());
        search = run_zhuigeng_catalog_search(state, &name_query, limit, req.exact).await?;
    }
    let recommended = choose_zhuigeng_resource_candidate(&search.items).cloned();
    Ok(ZhuigengResourcePlanResponse {
        ok: true,
        missing_hint: req
            .item
            .resource_hint
            .as_deref()
            .and_then(non_empty_trimmed)
            .map(ToString::to_string),
        item: req.item,
        query: search.query.clone(),
        fallback_queries,
        search,
        recommended,
    })
}

async fn run_zhuigeng_catalog_search(
    state: &AppState,
    q: &str,
    limit: i64,
    exact: Option<bool>,
) -> AppResult<CatalogRemoteSearchResponse> {
    catalog::catalog_remote_search_for_state(
        state,
        CatalogRemoteSearchQuery {
            q: q.to_string(),
            limit: Some(limit),
            offset: Some(0),
            disk_type: Some("115".to_string()),
            exact,
            sort: Some("resource".to_string()),
        },
    )
    .await
}

fn choose_zhuigeng_resource_candidate(items: &[CatalogItem]) -> Option<&CatalogItem> {
    items
        .iter()
        .find(|item| {
            item.transfer
                && item
                    .recommendation
                    .as_ref()
                    .is_some_and(|rec| matches!(rec.level.as_str(), "best" | "good"))
        })
        .or_else(|| {
            items.iter().find(|item| {
                item.transfer
                    && item
                        .recommendation
                        .as_ref()
                        .is_some_and(|rec| rec.level.as_str() == "warn" && !rec.already_have)
            })
        })
        .or_else(|| items.iter().find(|item| item.transfer))
}

pub async fn zhuigeng_update_execute_for_state(
    state: AppState,
    req: ZhuigengUpdateExecuteRequest,
) -> AppResult<TaskRun> {
    let target = zhuigeng_update_target(&req)?;
    let item = AddNewItem {
        url: candidate_transfer_url(&req.candidate)?,
        pwd: req.candidate.rc.clone(),
        label: req
            .candidate
            .name
            .clone()
            .or_else(|| non_empty_trimmed(&req.item.name).map(ToString::to_string)),
        file_ids: None,
        kind: req.candidate.link_type.clone(),
    };
    let add_req = AddNewRequest {
        items: vec![item],
        target: Some(target),
        lib: None,
        cid: None,
        delay_ms: req.delay_ms,
    };
    wizard::create_add_new_task(
        state,
        add_req,
        "zhuigeng",
        "zhuigeng_update",
        Some("追更一条龙更新"),
    )
    .await
}

fn zhuigeng_update_target(req: &ZhuigengUpdateExecuteRequest) -> AppResult<AddNewTarget> {
    let requested = req.target.clone().unwrap_or(AddNewTarget {
        lib: None,
        cid: None,
    });
    let lib = requested
        .lib
        .as_deref()
        .and_then(non_empty_trimmed)
        .map(ToString::to_string)
        .or_else(|| non_empty_trimmed(&req.item.lib).map(ToString::to_string));
    let cid = requested
        .cid
        .as_deref()
        .and_then(non_empty_trimmed)
        .map(ToString::to_string);
    if lib.is_none() && cid.is_none() {
        return Err(AppError::BadRequest(
            "缺少更新目标库或 115 cid，无法一条龙转存".to_string(),
        ));
    }
    Ok(AddNewTarget { lib, cid })
}

fn candidate_transfer_url(candidate: &CatalogTransferPlanItem) -> AppResult<String> {
    candidate
        .share
        .as_deref()
        .and_then(non_empty_trimmed)
        .or_else(|| non_empty_trimmed(&candidate.link))
        .map(ToString::to_string)
        .ok_or_else(|| AppError::BadRequest("候选资源缺少链接".to_string()))
}

pub async fn zhuigeng_archive_execute_for_state(
    state: AppState,
    req: ZhuigengArchiveExecuteRequest,
) -> AppResult<ZhuigengArchiveExecuteResponse> {
    let to_lib = non_empty_trimmed(&req.to_lib)
        .ok_or_else(|| AppError::BadRequest("归档目标库不能为空".to_string()))?
        .to_string();
    if req.items.is_empty() {
        return Err(AppError::BadRequest("items 不能为空".to_string()));
    }
    let mut by_lib = BTreeMap::<String, Vec<media_fs::ManageMoveBatchItem>>::new();
    for item in req.items {
        let from_lib = non_empty_trimmed(&item.lib)
            .ok_or_else(|| AppError::BadRequest(format!("{} 缺少来源库", item.name)))?
            .to_string();
        let folder = item
            .folder
            .as_deref()
            .and_then(non_empty_trimmed)
            .ok_or_else(|| AppError::BadRequest(format!("{} 缺少媒体文件夹", item.name)))?
            .to_string();
        by_lib
            .entry(from_lib)
            .or_default()
            .push(media_fs::ManageMoveBatchItem {
                folder,
                item_id: item
                    .id
                    .and_then(|id| non_empty_trimmed(&id).map(ToString::to_string)),
                to_folder: None,
            });
    }

    let total = by_lib.values().map(Vec::len).sum::<usize>();
    let mut tasks = Vec::new();
    for (from_lib, items) in by_lib {
        let move_req = media_fs::ManageMoveBatchRequest {
            from_lib,
            to_lib: to_lib.clone(),
            items,
            on_conflict: req
                .on_conflict
                .clone()
                .or_else(|| Some("smart".to_string())),
            reason: Some("追更完结剧一键归档".to_string()),
        };
        let task = media_fs::create_move_batch_task(
            state.clone(),
            move_req,
            "zhuigeng",
            "zhuigeng_archive",
            Some("追更完结归档"),
        )
        .await?;
        tasks.push(task);
    }

    Ok(ZhuigengArchiveExecuteResponse {
        ok: true,
        total,
        tasks,
    })
}

fn build_zhuigeng_resource_query(item: &ZhuigengItemRef) -> String {
    let name = item.name.trim();
    let hint = item
        .resource_hint
        .as_deref()
        .and_then(non_empty_trimmed)
        .unwrap_or_default();
    if name.is_empty() {
        return String::new();
    }
    if hint.is_empty() {
        name.to_string()
    } else {
        format!("{name} {hint}")
    }
}

impl ZhuigengConfig {
    pub fn new(
        emby_base_url: impl Into<String>,
        emby_api_key: impl Into<String>,
        tmdb_base_url: impl Into<String>,
        tmdb_api_key: impl Into<String>,
    ) -> Self {
        Self {
            emby_base_url: emby_base_url.into(),
            emby_api_key: emby_api_key.into(),
            tmdb_base_url: tmdb_base_url.into(),
            tmdb_api_key: tmdb_api_key.into(),
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
        }
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    fn validate(&self) -> AppResult<()> {
        if self.emby_api_key.trim().is_empty() {
            return Err(AppError::BadRequest(
                "api_key 未配置，无法读取 Emby 追更库".to_string(),
            ));
        }
        if !self.tmdb_api_key.trim().is_empty() && self.tmdb_base_url.trim().is_empty() {
            return Err(AppError::BadRequest(
                "tmdb_base_url/tmdb_url 未配置，无法请求 TMDb".to_string(),
            ));
        }
        if self.request_timeout.is_zero() {
            return Err(AppError::BadRequest(
                "tmdb_timeout_secs 必须是 1-60 的整数秒".to_string(),
            ));
        }
        Ok(())
    }

    fn has_tmdb_config(&self) -> bool {
        !self.tmdb_base_url.trim().is_empty() && !self.tmdb_api_key.trim().is_empty()
    }
}

async fn zhuigeng_config_from_state(state: &AppState) -> AppResult<ZhuigengConfig> {
    let emby_base_url =
        config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let emby_api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    let tmdb_base_url = get_first_string(&state.pool, &["tmdb_base_url", "tmdb_url"])
        .await?
        .unwrap_or_default();
    let tmdb_api_key = get_first_string(&state.pool, &["tmdb_api_key", "tmdb_key"])
        .await?
        .unwrap_or_default();
    let request_timeout = timeout_from_config(&state.pool).await?;
    let config = ZhuigengConfig {
        emby_base_url,
        emby_api_key,
        tmdb_base_url,
        tmdb_api_key,
        request_timeout,
    };
    Ok(config)
}

async fn get_first_string(pool: &PgPool, keys: &[&str]) -> Result<Option<String>, sqlx::Error> {
    for key in keys {
        if let Some(value) = config_store::get_string(pool, key).await?
            && !value.trim().is_empty()
        {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

async fn timeout_from_config(pool: &PgPool) -> AppResult<Duration> {
    let Some(value) = config_store::get_raw(pool, "tmdb_timeout_secs").await? else {
        return Ok(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS));
    };
    let secs = match value {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    };
    let Some(secs) = secs else {
        return Err(AppError::BadRequest(
            "tmdb_timeout_secs 必须是 1-60 的整数秒".to_string(),
        ));
    };
    if !(1..=MAX_REQUEST_TIMEOUT_SECS).contains(&secs) {
        return Err(AppError::BadRequest(
            "tmdb_timeout_secs 必须是 1-60 的整数秒".to_string(),
        ));
    }
    Ok(Duration::from_secs(secs))
}

#[derive(Clone)]
struct ZhuigengEmbyClient {
    base_url: String,
    api_key: String,
    timeout: Duration,
    http: Client,
}

impl ZhuigengEmbyClient {
    fn new(base_url: String, api_key: String, timeout: Duration, http: Client) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.trim().to_string(),
            timeout,
            http,
        }
    }

    async fn zhuigeng_libraries(&self) -> AppResult<Vec<ZhuigengLibrary>> {
        let url = format!("{}/Library/VirtualFolders", self.base_url);
        let folders: Vec<EmbyVirtualFolderLite> = get_json(
            &self.http,
            "Emby",
            "/Library/VirtualFolders",
            &url,
            &[("api_key", self.api_key.as_str())],
            self.timeout,
        )
        .await?;
        Ok(folders
            .into_iter()
            .filter_map(ZhuigengLibrary::from_virtual_folder)
            .filter(|library| {
                library.name.contains("追更")
                    && library.library_type.eq_ignore_ascii_case("tvshows")
            })
            .collect())
    }

    async fn series(&self, parent_id: &str) -> AppResult<Vec<EmbySeriesLite>> {
        let url = format!("{}/Items", self.base_url);
        let query = [
            ("api_key", self.api_key.as_str()),
            ("ParentId", parent_id),
            ("Recursive", "true"),
            ("IncludeItemTypes", "Series"),
            ("Fields", "Status,Path,ProviderIds"),
            ("SortBy", "SortName"),
            ("Limit", "30000"),
        ];
        Ok(
            get_json::<EmbyItemsPageLite>(&self.http, "Emby", "/Items", &url, &query, self.timeout)
                .await?
                .items,
        )
    }

    async fn episodes(&self, series_id: &str) -> AppResult<Vec<EmbyEpisodeLite>> {
        let path = format!("/Shows/{}/Episodes", urlencoding::encode(series_id.trim()));
        let url = format!("{}{}", self.base_url, path);
        let query = [
            ("api_key", self.api_key.as_str()),
            (
                "Fields",
                "PremiereDate,ParentIndexNumber,IndexNumber,LocationType",
            ),
            ("Limit", "6000"),
        ];
        Ok(
            get_json::<EmbyEpisodesPageLite>(&self.http, "Emby", &path, &url, &query, self.timeout)
                .await?
                .items,
        )
    }
}

impl ZhuigengLibrary {
    fn from_virtual_folder(folder: EmbyVirtualFolderLite) -> Option<Self> {
        let id = folder.item_id?.trim().to_string();
        if id.is_empty() {
            return None;
        }
        let name = if folder.name.trim().is_empty() {
            "(unnamed)".to_string()
        } else {
            folder.name
        };
        let mut paths = Vec::new();
        for path in folder.locations {
            push_path(&mut paths, path);
        }
        if let Some(options) = folder.library_options {
            for info in options.path_infos {
                if let Some(path) = info.path {
                    push_path(&mut paths, path);
                }
            }
        }
        Some(Self {
            id,
            name,
            library_type: folder
                .collection_type
                .unwrap_or_else(|| "mixed".to_string()),
            paths,
        })
    }
}

#[derive(Clone)]
struct TmdbClient {
    base_url: String,
    api_key: String,
    timeout: Duration,
    http: Client,
}

impl TmdbClient {
    fn new(base_url: String, api_key: String, timeout: Duration, http: Client) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.trim().to_string(),
            timeout,
            http,
        }
    }

    async fn tv(&self, tmdb_id: &str) -> AppResult<TmdbTvResponse> {
        let path = if self.base_url.ends_with("/3") {
            format!("/tv/{tmdb_id}")
        } else {
            format!("/3/tv/{tmdb_id}")
        };
        let url = format!("{}{}", self.base_url, path);
        get_json(
            &self.http,
            "TMDb",
            &path,
            &url,
            &[("api_key", self.api_key.as_str())],
            self.timeout,
        )
        .await
    }
}

async fn get_json<T>(
    http: &Client,
    service: &str,
    path: &str,
    url: &str,
    query: &[(&str, &str)],
    timeout: Duration,
) -> AppResult<T>
where
    T: DeserializeOwned,
{
    let response = http
        .get(url)
        .query(query)
        .timeout(timeout)
        .send()
        .await
        .map_err(|err| request_error(service, path, err))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::BadRequest(format!(
            "{service} {path} 返回 HTTP {status}"
        )));
    }
    response
        .json()
        .await
        .map_err(|err| AppError::BadRequest(format!("{service} {path} JSON 解析失败: {err}")))
}

fn request_error(service: &str, path: &str, err: reqwest::Error) -> AppError {
    if err.is_timeout() {
        AppError::BadRequest(format!("{service} {path} 请求超时"))
    } else {
        AppError::BadRequest(format!("{service} {path} 请求失败: {err}"))
    }
}

fn build_item_from_tmdb(
    library: &ZhuigengLibrary,
    item: &EmbySeriesLite,
    name: &str,
    folder: String,
    tmdb: String,
    local: LocalEpisodeSummary,
    meta: TmdbTvResponse,
) -> ZhuigengItem {
    let status = meta.status.unwrap_or_else(|| "?".to_string());
    let (state, continuing, ended) = tmdb_state(&status, meta.next_episode_to_air.as_ref());
    let behind = compute_behind(&local, meta.last_episode_to_air.as_ref());
    ZhuigengItem {
        lib: library.name.clone(),
        name: name.to_string(),
        id: item.id.clone(),
        folder,
        tmdb,
        tmdb_status: status,
        state,
        continuing,
        ended,
        local_count: local.count,
        local_latest: local.latest_date,
        local_latest_episode: local.latest_episode,
        last_episode_to_air: meta.last_episode_to_air,
        next_episode_to_air: meta.next_episode_to_air,
        behind: behind.count,
        behind_hint: behind.hint,
        resource_hint: behind.resource_hint,
        err: None,
    }
}

fn build_item_from_emby_status(
    library: &ZhuigengLibrary,
    item: &EmbySeriesLite,
    name: &str,
    folder: String,
    tmdb: String,
    local: LocalEpisodeSummary,
) -> ZhuigengItem {
    let status = item.status.clone().unwrap_or_else(|| "?".to_string());
    let (state, continuing, ended) = tmdb_state(&status, None);
    let behind = compute_virtual_behind(&local);
    ZhuigengItem {
        lib: library.name.clone(),
        name: name.to_string(),
        id: item.id.clone(),
        folder,
        tmdb,
        tmdb_status: status,
        state,
        continuing,
        ended,
        local_count: local.count,
        local_latest: local.latest_date,
        local_latest_episode: local.latest_episode,
        last_episode_to_air: None,
        next_episode_to_air: None,
        behind: behind.count,
        behind_hint: behind.hint,
        resource_hint: behind.resource_hint,
        err: None,
    }
}

fn build_error_item(
    library: &ZhuigengLibrary,
    item: &EmbySeriesLite,
    name: &str,
    folder: String,
    tmdb: String,
    local: LocalEpisodeSummary,
    err: String,
) -> ZhuigengItem {
    ZhuigengItem {
        lib: library.name.clone(),
        name: name.to_string(),
        id: item.id.clone(),
        folder,
        tmdb,
        tmdb_status: "?".to_string(),
        state: "unknown".to_string(),
        continuing: false,
        ended: false,
        local_count: local.count,
        local_latest: local.latest_date,
        local_latest_episode: local.latest_episode,
        last_episode_to_air: None,
        next_episode_to_air: None,
        behind: 0,
        behind_hint: None,
        resource_hint: None,
        err: Some(err),
    }
}

#[derive(Debug)]
struct BehindSummary {
    count: usize,
    hint: Option<String>,
    resource_hint: Option<String>,
}

fn compute_behind(
    local: &LocalEpisodeSummary,
    last_episode: Option<&TmdbEpisodeSummary>,
) -> BehindSummary {
    let Some(last_episode) = last_episode else {
        return BehindSummary {
            count: 0,
            hint: None,
            resource_hint: None,
        };
    };
    let Some(last_number) = last_episode.episode_number else {
        return BehindSummary {
            count: 0,
            hint: None,
            resource_hint: None,
        };
    };
    if last_number <= 0 {
        return BehindSummary {
            count: 0,
            hint: None,
            resource_hint: None,
        };
    }

    let season = last_episode.season_number;
    let have = local
        .have_by_season
        .get(&season)
        .cloned()
        .unwrap_or_default();
    let missing = (1..=last_number)
        .filter(|number| !have.contains(number))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return BehindSummary {
            count: 0,
            hint: None,
            resource_hint: None,
        };
    }

    let last_label =
        episode_label(season, Some(last_number)).unwrap_or_else(|| "最新集".to_string());
    let local_label = have
        .iter()
        .max()
        .and_then(|number| episode_label(season, Some(*number)))
        .or_else(|| local.latest_episode.clone())
        .unwrap_or_else(|| "本地无已入库集".to_string());
    let mut hint = format!(
        "落后 TMDb {} 集 · 最新 {} · 本地到 {}",
        missing.len(),
        last_label,
        local_label
    );
    if let Some(date) = last_episode
        .air_date
        .as_deref()
        .filter(|date| !date.is_empty())
    {
        hint.push_str(&format!(" · {date}"));
    }
    let resource_hint = match season {
        Some(season) => Some(format!("S{:02} E{}", season, compact_ints(&missing))),
        None => Some(format!("E{}", compact_ints(&missing))),
    };
    BehindSummary {
        count: missing.len(),
        hint: Some(hint),
        resource_hint,
    }
}

fn compute_virtual_behind(local: &LocalEpisodeSummary) -> BehindSummary {
    let mut missing = Vec::<(Option<i32>, i32)>::new();
    for (season, virtuals) in &local.virtual_by_season {
        let have = local
            .have_by_season
            .get(season)
            .cloned()
            .unwrap_or_default();
        for number in virtuals {
            if !have.contains(number) {
                missing.push((*season, *number));
            }
        }
    }
    if missing.is_empty() {
        return BehindSummary {
            count: 0,
            hint: None,
            resource_hint: None,
        };
    }
    missing.sort_by(compare_episode_pair);
    let resource_hint = format_episode_segments(&missing);
    let hint = Some(format!(
        "Emby 虚拟集提示缺 {} 集 · {}",
        missing.len(),
        resource_hint
    ));
    BehindSummary {
        count: missing.len(),
        hint,
        resource_hint: Some(resource_hint),
    }
}

fn format_episode_segments(missing: &[(Option<i32>, i32)]) -> String {
    let mut by_season: BTreeMap<Option<i32>, Vec<i32>> = BTreeMap::new();
    for (season, number) in missing {
        by_season.entry(*season).or_default().push(*number);
    }
    by_season
        .into_iter()
        .map(|(season, mut numbers)| {
            numbers.sort_unstable();
            numbers.dedup();
            let compact = compact_ints(&numbers);
            match season {
                Some(season) => format!("S{season:02} E{compact}"),
                None => format!("E{compact}"),
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn summarize_local_episodes(episodes: &[EmbyEpisodeLite]) -> LocalEpisodeSummary {
    let mut count = 0usize;
    let mut latest_date = None::<String>;
    let mut latest_pair = None::<(Option<i32>, i32)>;
    let mut have_by_season: BTreeMap<Option<i32>, Vec<i32>> = BTreeMap::new();
    let mut virtual_by_season: BTreeMap<Option<i32>, Vec<i32>> = BTreeMap::new();

    for episode in episodes {
        if episode
            .location_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("Virtual"))
        {
            if let Some(number) = episode.index_number {
                virtual_by_season
                    .entry(episode.parent_index_number)
                    .or_default()
                    .push(number);
            }
            continue;
        }
        count += 1;
        if let Some(date) = episode
            .premiere_date
            .as_deref()
            .map(|value| value.chars().take(10).collect::<String>())
            .filter(|value| !value.is_empty())
            && latest_date
                .as_deref()
                .is_none_or(|current| date.as_str() > current)
        {
            latest_date = Some(date);
        }
        if let Some(number) = episode.index_number {
            have_by_season
                .entry(episode.parent_index_number)
                .or_default()
                .push(number);
            let pair = (episode.parent_index_number, number);
            if latest_pair
                .as_ref()
                .is_none_or(|current| compare_episode_pair(&pair, current).is_gt())
            {
                latest_pair = Some(pair);
            }
        }
    }
    for values in have_by_season.values_mut() {
        values.sort_unstable();
        values.dedup();
    }
    for values in virtual_by_season.values_mut() {
        values.sort_unstable();
        values.dedup();
    }
    LocalEpisodeSummary {
        count,
        latest_date,
        latest_episode: latest_pair
            .and_then(|(season, number)| episode_label(season, Some(number))),
        have_by_season,
        virtual_by_season,
    }
}

fn compare_episode_pair(
    left: &(Option<i32>, i32),
    right: &(Option<i32>, i32),
) -> std::cmp::Ordering {
    left.0
        .unwrap_or(0)
        .cmp(&right.0.unwrap_or(0))
        .then_with(|| left.1.cmp(&right.1))
}

fn tmdb_state(status: &str, next_episode: Option<&TmdbEpisodeSummary>) -> (String, bool, bool) {
    let normalized = status.trim().to_ascii_lowercase();
    let ended = matches!(normalized.as_str(), "ended" | "canceled" | "cancelled");
    let continuing = !ended
        && (matches!(
            normalized.as_str(),
            "returning series" | "continuing" | "in production" | "pilot" | "planned"
        ) || next_episode.is_some());
    let state = if ended {
        "ended"
    } else if continuing {
        "continuing"
    } else {
        "unknown"
    };
    (state.to_string(), continuing, ended)
}

fn build_copy_text(items: &[ZhuigengItem], continuing_only: bool) -> String {
    items
        .iter()
        .filter(|item| item.behind > 0)
        .filter(|item| !continuing_only || item.continuing)
        .filter_map(|item| item.resource_hint.as_ref().map(|hint| (item, hint)))
        .map(|(item, hint)| {
            if item.tmdb.trim().is_empty() {
                format!("求 {} — {}", item.name, hint)
            } else {
                format!("求 {} [tmdb:{}] — {}", item.name, item.tmdb, hint)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn folder_from_series_path(path: Option<&str>, library: &ZhuigengLibrary) -> String {
    let Some(path) = path else {
        return String::new();
    };
    let library_folder = library
        .paths
        .iter()
        .filter_map(|path| top_folder_name(path))
        .next()
        .unwrap_or_else(|| library.name.clone());
    let sep = format!("/{library_folder}/");
    if let Some((_, rest)) = path.split_once(&sep) {
        return rest.split('/').next().unwrap_or_default().to_string();
    }
    for root in &library.paths {
        let root = root.trim_end_matches('/');
        if let Some(rest) = path.strip_prefix(root)
            && let Some(rest) = rest.strip_prefix('/')
        {
            return rest.split('/').next().unwrap_or_default().to_string();
        }
    }
    String::new()
}

fn top_folder_name(path: &str) -> Option<String> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn provider_id(provider_ids: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    provider_ids
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .and_then(|(_, value)| match value {
            Value::String(raw) => {
                let raw = raw.trim();
                (!raw.is_empty()).then(|| raw.to_string())
            }
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(value) => Some(value.to_string()),
            _ => None,
        })
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn validate_tmdb_id(tmdb: &str) -> AppResult<()> {
    let value = tmdb.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(
            "ProviderIds.Tmdb 为空，无法查询 TMDb".to_string(),
        ));
    }
    if !value.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(AppError::BadRequest(format!(
            "ProviderIds.Tmdb 必须是数字: {tmdb:?}"
        )));
    }
    Ok(())
}

fn episode_label(season: Option<i32>, episode: Option<i32>) -> Option<String> {
    let episode = episode?;
    Some(match season {
        Some(season) => format!("S{:02}E{:02}", season, episode),
        None => format!("E{episode:02}"),
    })
}

fn compact_ints(values: &[i32]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let mut values = values.to_vec();
    values.sort_unstable();
    values.dedup();
    let mut out = Vec::new();
    let mut start = values[0];
    let mut prev = values[0];
    for value in values.into_iter().skip(1) {
        if value == prev + 1 {
            prev = value;
            continue;
        }
        out.push(compact_range(start, prev));
        start = value;
        prev = value;
    }
    out.push(compact_range(start, prev));
    out.join(",")
}

fn compact_range(start: i32, end: i32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

fn push_path(paths: &mut Vec<String>, path: String) {
    let path = path.trim();
    if !path.is_empty() && !paths.iter().any(|existing| existing == path) {
        paths.push(path.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ZhuigengItem, ZhuigengWorkbenchLane, apply_resource_episode_inference,
        build_zhuigeng_workbench_row, compact_ints, tmdb_state,
    };
    use crate::catalog::CatalogItem;

    #[test]
    fn compacts_episode_ranges() {
        assert_eq!(compact_ints(&[1, 2, 3, 7, 9, 10]), "1-3,7,9-10");
    }

    #[test]
    fn maps_tmdb_states() {
        assert_eq!(
            tmdb_state("Ended", None),
            ("ended".to_string(), false, true)
        );
        assert_eq!(
            tmdb_state("Returning Series", None),
            ("continuing".to_string(), true, false)
        );
    }

    #[test]
    fn resource_complete_title_promotes_healthy_airing_to_archive_ready() {
        let mut item = test_item(25, "S01E25");
        apply_resource_episode_inference(&mut item, &[catalog_item("大唐迷雾 全25集 4K SDR")]);

        assert!(item.ended);
        assert!(!item.continuing);
        assert_eq!(item.behind, 0);
        assert!(
            item.behind_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("建议归档"))
        );
        let row = build_zhuigeng_workbench_row(item);
        assert_eq!(row.lane, ZhuigengWorkbenchLane::ArchiveReady);
        assert!(row.archiveable);
    }

    #[test]
    fn resource_new_episode_keeps_airing_update_needed() {
        let mut item = test_item(25, "S01E25");
        apply_resource_episode_inference(&mut item, &[catalog_item("大唐迷雾 (2026) S01E26")]);

        assert!(item.continuing);
        assert!(!item.ended);
        assert_eq!(item.behind, 1);
        assert_eq!(item.resource_hint.as_deref(), Some("S01 E26"));
        let row = build_zhuigeng_workbench_row(item);
        assert_eq!(row.lane, ZhuigengWorkbenchLane::UpdateNeeded);
        assert!(row.updateable);
    }

    #[test]
    fn resource_complete_package_with_missing_episodes_requires_update_then_archive() {
        let mut item = test_item(25, "S01E25");
        apply_resource_episode_inference(&mut item, &[catalog_item("大唐迷雾 完结 全40集")]);

        assert!(item.ended);
        assert!(!item.continuing);
        assert_eq!(item.behind, 15);
        assert_eq!(item.resource_hint.as_deref(), Some("S01 E26-40"));
        let row = build_zhuigeng_workbench_row(item);
        assert_eq!(row.lane, ZhuigengWorkbenchLane::CompleteAfterUpdate);
        assert!(row.updateable);
        assert!(!row.archiveable);
    }

    fn test_item(local_count: usize, latest: &str) -> ZhuigengItem {
        ZhuigengItem {
            lib: "电视剧追更".to_string(),
            name: "大唐迷雾".to_string(),
            id: Some("series-id".to_string()),
            folder: "大唐迷雾(2026)[tmdbid-289209]".to_string(),
            tmdb: "289209".to_string(),
            tmdb_status: "Continuing".to_string(),
            state: "continuing".to_string(),
            continuing: true,
            ended: false,
            local_count,
            local_latest: None,
            local_latest_episode: Some(latest.to_string()),
            last_episode_to_air: None,
            next_episode_to_air: None,
            behind: 0,
            behind_hint: None,
            resource_hint: None,
            err: None,
        }
    }

    fn catalog_item(name: &str) -> CatalogItem {
        CatalogItem {
            name: name.to_string(),
            sheet: "test".to_string(),
            link: "https://115cdn.com/s/example".to_string(),
            is_pkg: true,
            link_type: "share115".to_string(),
            transfer: true,
            share: Some("example".to_string()),
            rc: Some("abcd".to_string()),
            recommendation: None,
        }
    }
}
