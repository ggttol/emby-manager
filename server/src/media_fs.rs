use crate::{
    config_store,
    emby::{EmbyClient, EmbyLibrary, EmbyLibraryOptions, EmbyPathInfo},
    error::{AppError, AppResult},
    state::AppState,
    tasks::{self, TaskRun},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};
use tokio::time::{Duration, sleep};
use uuid::Uuid;
use walkdir::WalkDir;

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const DEFAULT_STRM_OVERVIEW_DEPTH: usize = 8;
const MAX_STRM_OVERVIEW_DEPTH: usize = 20;
const DEFAULT_STRM_SAMPLE_LIMIT: usize = 20;
const MAX_STRM_SAMPLE_LIMIT: usize = 100;
const STRM_OVERVIEW_ENTRY_LIMIT: usize = 100_000;
const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "ts", "m2ts", "avi", "iso", "mov", "flv", "wmv", "rmvb",
];
const SUBTITLE_EXTENSIONS: &[&str] = &["ass", "idx", "smi", "srt", "ssa", "sub", "sup", "vtt"];
pub const SCAN_TASK_CANCELLED_SENTINEL: &str = "__task_cancelled__";

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LibrariesResponse {
    pub libraries: Vec<EmbyLibrary>,
    pub excluded: Vec<ExcludedLibrary>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ExcludedLibrary {
    pub name: String,
    pub reason: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateLibraryRequest {
    pub name: String,
    #[serde(alias = "ctype")]
    pub collection_type: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CreateLibraryResponse {
    pub ok: bool,
    pub name: String,
    pub id: Option<String>,
    pub library: Option<EmbyLibrary>,
    pub created_dirs: Vec<String>,
    pub emby_status: u16,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct LibraryItemsQuery {
    pub lib: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LibraryItemEntry {
    pub id: Option<String>,
    pub name: String,
    pub item_type: Option<String>,
    pub year: Option<i32>,
    pub tmdb: String,
    pub folder: String,
    pub path: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LibraryItemsResponse {
    pub lib: String,
    pub item_types: String,
    pub total_record_count: Option<usize>,
    pub truncated: bool,
    pub items: Vec<LibraryItemEntry>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct StrmListQuery {
    pub lib: Option<String>,
    pub folder: Option<String>,
    pub limit: Option<usize>,
    pub overview: Option<bool>,
    pub overview_depth: Option<usize>,
    pub sample_limit: Option<usize>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StrmEntry {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StrmListResponse {
    pub base: String,
    pub items: Vec<StrmEntry>,
    pub truncated: bool,
    pub overview: Option<StrmOverview>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StrmOverview {
    pub base: String,
    pub max_depth: usize,
    pub entry_limit: usize,
    pub directories: usize,
    pub files: usize,
    pub strm_files: usize,
    pub subtitle_files: usize,
    pub other_files: usize,
    pub strm_bytes: u64,
    pub subtitle_bytes: u64,
    pub strm_with_subtitles: usize,
    pub strm_without_subtitles: usize,
    pub subtitle_coverage_percent: f64,
    pub subtitle_extensions: Vec<ExtensionCount>,
    pub subtitle_languages: Vec<SubtitleLanguageCount>,
    pub library_coverage: Vec<SubtitleLibraryCoverage>,
    pub missing_subtitle_samples: Vec<String>,
    pub samples: Vec<StrmSample>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SubtitleLanguageCount {
    pub language: String,
    pub count: usize,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SubtitleLibraryCoverage {
    pub library: String,
    pub strm_files: usize,
    pub with_subtitles: usize,
    pub missing_subtitles: usize,
    pub coverage_percent: f64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ExtensionCount {
    pub extension: String,
    pub count: usize,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StrmSample {
    pub rel_path: String,
    pub kind: String,
    pub extension: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ScanLibraryRequest {
    pub lib: Option<String>,
    pub item_id: Option<String>,
    pub recursive: Option<bool>,
    pub full: Option<bool>,
    pub generate_strm: Option<bool>,
    pub keyword: Option<String>,
    pub fullauto: Option<bool>,
    pub cleanup_orphans: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ScanLibraryResult {
    pub ok: bool,
    pub mode: String,
    pub requested: Option<String>,
    pub triggered: usize,
    pub global_refresh: bool,
    pub items: Vec<ScanLibraryItemResult>,
    pub strm: Option<StrmGenerateResult>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ScanLibraryItemResult {
    pub id: Option<String>,
    pub name: String,
    pub code: u16,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct StrmGenerateResult {
    pub lib: String,
    pub keyword: String,
    pub matched: usize,
    pub new_count: usize,
    pub new_folders: BTreeMap<String, usize>,
    pub attention: Vec<String>,
    pub orphans_cleaned: usize,
    pub orphan_cleanup_skipped: bool,
    pub permissions_fixed: usize,
    pub refreshed: bool,
    pub refresh_code: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ManageDeleteRequest {
    pub lib: String,
    pub folder: String,
    pub item_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ManageMoveRequest {
    #[serde(alias = "from")]
    pub from_lib: String,
    #[serde(alias = "folder")]
    pub from_folder: String,
    #[serde(alias = "to")]
    pub to_lib: String,
    pub to_folder: Option<String>,
    #[serde(alias = "id")]
    pub item_id: Option<String>,
    pub reason: Option<String>,
    pub on_conflict: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ManageDeleteBatchRequest {
    pub items: Vec<ManageDeleteRequest>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ManageMoveBatchItem {
    pub folder: String,
    #[serde(alias = "id")]
    pub item_id: Option<String>,
    pub to_folder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ManageMoveBatchRequest {
    #[serde(alias = "from")]
    pub from_lib: String,
    #[serde(alias = "to")]
    pub to_lib: String,
    pub items: Vec<ManageMoveBatchItem>,
    pub on_conflict: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ManagePreviewResult {
    pub ok: bool,
    pub preview: bool,
    pub dry_run: bool,
    pub operation: String,
    pub planned_paths: Vec<String>,
    pub warnings: Vec<String>,
    pub next_steps: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ManageExecuteQuery {
    pub execute: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ManageDeleteExecuteResult {
    pub ok: bool,
    pub preview: bool,
    pub dry_run: bool,
    pub operation: String,
    pub folder: String,
    pub emby_gone: bool,
    pub deleted_from: Vec<String>,
    pub notified: bool,
    pub undo_id: Uuid,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ManageMoveExecuteResult {
    pub ok: bool,
    pub preview: bool,
    pub dry_run: bool,
    pub operation: String,
    pub from_lib: String,
    pub from_folder: String,
    pub to_lib: String,
    pub to_folder: String,
    pub moved: bool,
    pub skipped: bool,
    pub smart_action: Option<String>,
    pub msg: Option<String>,
    pub src_count: Option<usize>,
    pub dst_count: Option<usize>,
    pub old_strm_removed: bool,
    pub strm_written: usize,
    pub emby_gone: bool,
    pub notified: bool,
    pub refresh_code: Option<u16>,
    pub undo_id: Uuid,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ManageDeleteBatchResult {
    pub ok: bool,
    pub total: usize,
    pub ok_count: usize,
    pub error_count: usize,
    pub results: Vec<ManageDeleteBatchItemResult>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ManageDeleteBatchItemResult {
    pub lib: String,
    pub folder: String,
    pub ok: bool,
    pub result: Option<ManageDeleteExecuteResult>,
    pub err: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ManageMoveBatchResult {
    pub ok: bool,
    pub from_lib: String,
    pub to_lib: String,
    pub total: usize,
    pub ok_count: usize,
    pub error_count: usize,
    pub smart_count: usize,
    pub results: Vec<ManageMoveBatchItemResult>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ManageMoveBatchItemResult {
    pub folder: String,
    pub ok: bool,
    pub skipped: bool,
    pub smart_action: Option<String>,
    pub result: Option<ManageMoveExecuteResult>,
    pub err: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/libraries", get(libraries).post(create_library))
        .route("/api/v2/libraries/items", get(library_items))
        .route("/api/v2/libraries/scan", post(scan_library))
        .route("/api/v2/libraries/scan-all", post(scan_all_libraries))
        .route("/api/v2/libraries/strm", get(list_strm))
        .route("/api/v2/manage/delete", post(manage_delete))
        .route("/api/v2/manage/delete/execute", post(execute_delete))
        .route(
            "/api/v2/manage/delete/batch/execute",
            post(execute_delete_batch),
        )
        .route("/api/v2/manage/move", post(manage_move))
        .route("/api/v2/manage/move/execute", post(execute_move))
        .route(
            "/api/v2/manage/move/batch/execute",
            post(execute_move_batch),
        )
}

#[utoipa::path(get, path = "/api/v2/libraries", tag = "media", responses((status = 200, body = LibrariesResponse)))]
pub async fn libraries(State(state): State<AppState>) -> AppResult<Json<LibrariesResponse>> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;

    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let mut libraries = client.libraries().await?;
    enrich_library_card_stats(&client, &mut libraries).await;
    let excluded = excluded_libraries(&libraries);
    Ok(Json(LibrariesResponse {
        libraries,
        excluded,
    }))
}

async fn enrich_library_card_stats(client: &EmbyClient, libraries: &mut [EmbyLibrary]) {
    for library in libraries {
        let Some(id) = library.id.clone() else {
            library.refresh_summary();
            continue;
        };
        if library.library_type.eq_ignore_ascii_case("tvshows") {
            let mut counted = false;
            if let Ok(series) = client.item_count(&id, "Series").await {
                library.counts.series = series;
                counted = true;
            }
            if let Ok(episodes) = client.item_count(&id, "Episode").await {
                library.counts.episodes = episodes;
                counted = true;
            }
            if counted {
                library.counts.items = library.counts.series + library.counts.episodes;
            }
        } else if let Ok(movies) = client
            .item_count(&id, library_item_types(&library.library_type))
            .await
        {
            library.counts.movies = movies;
            library.counts.items = movies;
        }
        library.refresh_summary();
    }
}

fn excluded_libraries(libraries: &[EmbyLibrary]) -> Vec<ExcludedLibrary> {
    libraries
        .iter()
        .filter_map(|library| {
            if library.paths.is_empty() {
                return Some(ExcludedLibrary {
                    name: library.name.clone(),
                    reason: "无路径".to_string(),
                    paths: Vec::new(),
                });
            }
            if library
                .paths
                .iter()
                .any(|path| folder_from_strm_path(path).is_some())
            {
                return None;
            }
            Some(ExcludedLibrary {
                name: library.name.clone(),
                reason: "无 /strm/ 路径(boxset 或别的库类型)".to_string(),
                paths: library.paths.clone(),
            })
        })
        .collect()
}

#[utoipa::path(post, path = "/api/v2/libraries", tag = "media", request_body = CreateLibraryRequest, responses((status = 200, body = CreateLibraryResponse)))]
pub async fn create_library(
    State(state): State<AppState>,
    Json(req): Json<CreateLibraryRequest>,
) -> AppResult<Json<CreateLibraryResponse>> {
    let name = required_segment(&req.name, "name")?.to_string();
    let collection_type = normalize_library_collection_type(&req.collection_type)?;
    let emby_path = format!("/strm/{name}");
    let strm_dir = safe_under(&state.settings.strm_root, &name)?;
    let cd_dir = safe_under(&state.settings.cd_root, &name)?;

    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;

    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let virtual_folders = client.virtual_folders().await?;
    if virtual_folders
        .iter()
        .any(|folder| folder.name.trim() == name)
    {
        return Err(AppError::Conflict("已存在同名库".to_string()));
    }

    tokio::fs::create_dir_all(&strm_dir)
        .await
        .map_err(anyhow::Error::from)?;
    tokio::fs::create_dir_all(&cd_dir)
        .await
        .map_err(anyhow::Error::from)?;
    chmod_public_dir(&strm_dir);
    chmod_public_dir(&cd_dir);

    let mut warnings = Vec::new();
    let mut library_options = virtual_folders
        .iter()
        .find_map(|folder| {
            let same_type = folder
                .collection_type
                .as_deref()
                .is_some_and(|value| value == collection_type);
            let has_strm_location = folder
                .locations
                .iter()
                .any(|path| path.trim_start().starts_with("/strm/"))
                || folder.library_options.as_ref().is_some_and(|options| {
                    options.path_infos.iter().any(|info| {
                        info.path
                            .as_deref()
                            .is_some_and(|path| path.trim_start().starts_with("/strm/"))
                    })
                });
            if same_type && has_strm_location {
                folder.library_options.clone()
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            warnings.push(format!(
                "未找到可复用的 {collection_type} /strm 库设置，使用最小 PathInfos"
            ));
            EmbyLibraryOptions {
                path_infos: Vec::new(),
                extra: Default::default(),
            }
        });
    library_options.path_infos = vec![EmbyPathInfo {
        path: Some(emby_path.clone()),
    }];

    let emby_status = client
        .create_virtual_folder(&name, collection_type, &emby_path, library_options)
        .await?;
    sleep(Duration::from_secs(1)).await;

    let library = client
        .libraries()
        .await?
        .into_iter()
        .find(|library| library.name == name);
    let id = library.as_ref().and_then(|library| library.id.clone());
    if library.is_none() {
        warnings.push(format!("创建后未在库列表找到 (HTTP {emby_status})"));
    }
    Ok(Json(CreateLibraryResponse {
        ok: library.is_some(),
        name,
        id,
        library,
        created_dirs: vec![strm_dir.display().to_string(), cd_dir.display().to_string()],
        emby_status,
        warnings,
    }))
}

#[utoipa::path(get, path = "/api/v2/libraries/items", tag = "media", params(LibraryItemsQuery), responses((status = 200, body = LibraryItemsResponse)))]
pub async fn library_items(
    State(state): State<AppState>,
    Query(query): Query<LibraryItemsQuery>,
) -> AppResult<Json<LibraryItemsResponse>> {
    let requested = query.lib.trim();
    if requested.is_empty() {
        return Err(AppError::BadRequest("lib is required".to_string()));
    }

    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;

    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let libraries = client.libraries().await?;
    let library = libraries
        .into_iter()
        .find(|library| {
            library.name == requested || library.id.as_deref().is_some_and(|id| id == requested)
        })
        .ok_or_else(|| AppError::BadRequest(format!("未知库 {requested}")))?;
    let parent_id = library
        .id
        .as_deref()
        .and_then(non_empty_trimmed)
        .ok_or_else(|| AppError::BadRequest(format!("库 {} 缺少 Emby ItemId", library.name)))?;
    let item_types = library_item_types(&library.library_type);
    let result = client
        .library_items(parent_id, item_types, query.limit.unwrap_or(30_000))
        .await?;
    let items = result
        .items
        .into_iter()
        .map(|item| LibraryItemEntry {
            tmdb: item.provider_id("Tmdb").unwrap_or_default(),
            folder: folder_from_library_path(item.path.as_deref(), &library),
            id: item.id,
            name: item.name.unwrap_or_else(|| "(无名)".to_string()),
            item_type: item.item_type,
            year: item.production_year,
            path: item.path,
        })
        .collect();

    Ok(Json(LibraryItemsResponse {
        lib: library.name,
        item_types: item_types.to_string(),
        total_record_count: result.total_record_count,
        truncated: result.truncated,
        items,
    }))
}

#[utoipa::path(get, path = "/api/v2/libraries/strm", tag = "media", params(StrmListQuery), responses((status = 200, body = StrmListResponse)))]
pub async fn list_strm(
    State(state): State<AppState>,
    Query(query): Query<StrmListQuery>,
) -> AppResult<Json<StrmListResponse>> {
    let lib_root = if let Some(lib) = query
        .lib
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let folder =
            resolve_library_folder_for_roots(&state, lib, &[&state.settings.strm_root]).await?;
        safe_under(&state.settings.strm_root, folder)?
    } else {
        state.settings.strm_root.clone()
    };
    let base = if let Some(folder) = query.folder.as_deref().filter(|s| !s.trim().is_empty()) {
        safe_under(&lib_root, folder.trim())?
    } else {
        lib_root
    };
    let limit = query.limit.unwrap_or(500).clamp(1, 5000);
    let mut items = Vec::new();
    let mut truncated = false;
    if !base.exists() {
        return Ok(Json(StrmListResponse {
            base: base.display().to_string(),
            items,
            truncated,
            overview: maybe_strm_overview(&base, &query),
        }));
    }
    for entry in WalkDir::new(&base)
        .min_depth(1)
        .max_depth(2)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        let file_type = entry.file_type();
        let is_dir = file_type.is_dir();
        let is_strm = file_type.is_file() && has_extension(path, "strm");
        if !is_dir && !is_strm {
            continue;
        }
        if items.len() >= limit {
            truncated = true;
            break;
        }
        let Ok(rel) = path.strip_prefix(&base) else {
            continue;
        };
        items.push(StrmEntry {
            name: path
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or_default()
                .to_string(),
            rel_path: rel.to_string_lossy().replace('\\', "/"),
            is_dir,
            size: if is_strm {
                entry.metadata().map(|m| m.len()).unwrap_or(0)
            } else {
                0
            },
        });
    }
    Ok(Json(StrmListResponse {
        base: base.display().to_string(),
        items,
        truncated,
        overview: maybe_strm_overview(&base, &query),
    }))
}

#[utoipa::path(post, path = "/api/v2/libraries/scan", tag = "media", request_body = ScanLibraryRequest, responses((status = 200, body = TaskRun)))]
pub async fn scan_library(
    State(state): State<AppState>,
    Json(req): Json<ScanLibraryRequest>,
) -> AppResult<Json<TaskRun>> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;

    let label = scan_task_label(&req);
    let task = tasks::insert_task(&state.pool, "scan_library", &label, 1).await?;
    spawn_scan_task(state, task.id, emby_url, api_key, req);
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/libraries/scan-all", tag = "media", request_body = ScanLibraryRequest, responses((status = 200, body = TaskRun)))]
pub async fn scan_all_libraries(
    State(state): State<AppState>,
    Json(req): Json<ScanLibraryRequest>,
) -> AppResult<Json<TaskRun>> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;

    let params = serde_json::to_value(&req).unwrap_or_else(|_| serde_json::json!({}));
    let label = if let Some(lib) = req.lib.as_deref().and_then(non_empty_trimmed) {
        format!("扫全库: {lib}")
    } else {
        "扫全库".to_string()
    };
    let task =
        tasks::insert_task_with_meta(&state.pool, "scan_all", &label, 1, "manual", params.clone())
            .await?;
    spawn_scan_all_task(state, task.id, label, emby_url, api_key, params);
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/manage/delete", tag = "media", request_body = ManageDeleteRequest, responses((status = 200, body = TaskRun)))]
pub async fn preview_delete(
    State(state): State<AppState>,
    Json(req): Json<ManageDeleteRequest>,
) -> AppResult<Json<TaskRun>> {
    let lib_folder = resolve_library_folder_for_roots(
        &state,
        &req.lib,
        &[&state.settings.strm_root, &state.settings.cd_root],
    )
    .await?;
    let plan = plan_delete(&state, &req, &lib_folder)?;
    let params = serde_json::to_value(&req).unwrap_or_else(|_| serde_json::json!({}));
    let label = format!("预览删除: {}/{}", req.lib.trim(), req.folder.trim());
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "manage_delete_preview",
        &label,
        1,
        "manual",
        params,
    )
    .await?;
    spawn_manage_preview(state, task.id, "delete", plan);
    Ok(Json(task))
}

pub async fn manage_delete(
    State(state): State<AppState>,
    Query(query): Query<ManageExecuteQuery>,
    Json(req): Json<ManageDeleteRequest>,
) -> AppResult<Json<TaskRun>> {
    if query.execute.unwrap_or(false) {
        execute_delete(State(state), Json(req)).await
    } else {
        preview_delete(State(state), Json(req)).await
    }
}

#[utoipa::path(post, path = "/api/v2/manage/delete/execute", tag = "media", request_body = ManageDeleteRequest, responses((status = 200, body = TaskRun)))]
pub async fn execute_delete(
    State(state): State<AppState>,
    Json(req): Json<ManageDeleteRequest>,
) -> AppResult<Json<TaskRun>> {
    let lib_folder = resolve_library_folder_for_roots(
        &state,
        &req.lib,
        &[&state.settings.strm_root, &state.settings.cd_root],
    )
    .await?;
    let plan = plan_delete_execute(&state, &req, &lib_folder)?;
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;

    let params = serde_json::to_value(&req).unwrap_or_else(|_| serde_json::json!({}));
    let label = format!("删除: {}/{}", req.lib.trim(), req.folder.trim());
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "manage_delete_execute",
        &label,
        4,
        "manual",
        params,
    )
    .await?;
    spawn_delete_execute(state, task.id, emby_url, api_key, req, plan);
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/manage/delete/batch/execute", tag = "media", request_body = ManageDeleteBatchRequest, responses((status = 200, body = TaskRun)))]
pub async fn execute_delete_batch(
    State(state): State<AppState>,
    Json(req): Json<ManageDeleteBatchRequest>,
) -> AppResult<Json<TaskRun>> {
    let mut items = Vec::new();
    for mut item in req.items.into_iter().take(500) {
        if item.reason.as_deref().and_then(non_empty_trimmed).is_none() {
            item.reason.clone_from(&req.reason);
        }
        let lib_folder = resolve_library_folder_for_roots(
            &state,
            &item.lib,
            &[&state.settings.strm_root, &state.settings.cd_root],
        )
        .await?;
        let plan = plan_delete_execute(&state, &item, &lib_folder)?;
        items.push((item, plan));
    }
    if items.is_empty() {
        return Err(AppError::BadRequest("items must not be empty".to_string()));
    }

    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;

    let params = serde_json::json!({
        "items": items.iter().map(|(item, _)| item).collect::<Vec<_>>(),
        "reason": req.reason,
    });
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "manage_delete_batch_execute",
        &format!("批量删除: {} 项", items.len()),
        items.len() as i64,
        "manual",
        params,
    )
    .await?;
    spawn_delete_batch_execute(state, task.id, emby_url, api_key, items);
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/manage/move", tag = "media", request_body = ManageMoveRequest, responses((status = 200, body = TaskRun)))]
pub async fn preview_move(
    State(state): State<AppState>,
    Json(req): Json<ManageMoveRequest>,
) -> AppResult<Json<TaskRun>> {
    let from_lib_folder = resolve_library_folder_for_roots(
        &state,
        &req.from_lib,
        &[&state.settings.strm_root, &state.settings.cd_root],
    )
    .await?;
    let to_lib_folder = resolve_library_folder_for_roots(
        &state,
        &req.to_lib,
        &[&state.settings.strm_root, &state.settings.cd_root],
    )
    .await?;
    let plan = plan_move(&state, &req, &from_lib_folder, &to_lib_folder)?;
    let params = serde_json::to_value(&req).unwrap_or_else(|_| serde_json::json!({}));
    let label = format!(
        "预览移动: {}/{} -> {}",
        req.from_lib.trim(),
        req.from_folder.trim(),
        req.to_lib.trim()
    );
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "manage_move_preview",
        &label,
        1,
        "manual",
        params,
    )
    .await?;
    spawn_manage_preview(state, task.id, "move", plan);
    Ok(Json(task))
}

pub async fn manage_move(
    State(state): State<AppState>,
    Query(query): Query<ManageExecuteQuery>,
    Json(req): Json<ManageMoveRequest>,
) -> AppResult<Json<TaskRun>> {
    if query.execute.unwrap_or(false) {
        execute_move(State(state), Json(req)).await
    } else {
        preview_move(State(state), Json(req)).await
    }
}

#[utoipa::path(post, path = "/api/v2/manage/move/execute", tag = "media", request_body = ManageMoveRequest, responses((status = 200, body = TaskRun)))]
pub async fn execute_move(
    State(state): State<AppState>,
    Json(req): Json<ManageMoveRequest>,
) -> AppResult<Json<TaskRun>> {
    let from_lib_folder = resolve_library_folder_for_roots(
        &state,
        &req.from_lib,
        &[&state.settings.strm_root, &state.settings.cd_root],
    )
    .await?;
    let to_lib_folder = resolve_library_folder_for_roots(
        &state,
        &req.to_lib,
        &[&state.settings.strm_root, &state.settings.cd_root],
    )
    .await?;
    let plan = plan_move_execute(&state, &req, &from_lib_folder, &to_lib_folder)?;
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;

    let params = serde_json::to_value(&req).unwrap_or_else(|_| serde_json::json!({}));
    let label = format!(
        "移动: {}/{} -> {}/{}",
        req.from_lib.trim(),
        req.from_folder.trim(),
        req.to_lib.trim(),
        plan.to_folder
    );
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "manage_move_execute",
        &label,
        5,
        "manual",
        params,
    )
    .await?;
    spawn_move_execute(state, task.id, emby_url, api_key, req, plan);
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/manage/move/batch/execute", tag = "media", request_body = ManageMoveBatchRequest, responses((status = 200, body = TaskRun)))]
pub async fn execute_move_batch(
    State(state): State<AppState>,
    Json(req): Json<ManageMoveBatchRequest>,
) -> AppResult<Json<TaskRun>> {
    if req.items.is_empty() {
        return Err(AppError::BadRequest("items 不能为空".to_string()));
    }
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;
    let from_lib_folder = resolve_library_folder_for_roots(
        &state,
        &req.from_lib,
        &[&state.settings.strm_root, &state.settings.cd_root],
    )
    .await?;
    let to_lib_folder = resolve_library_folder_for_roots(
        &state,
        &req.to_lib,
        &[&state.settings.strm_root, &state.settings.cd_root],
    )
    .await?;
    let items = req
        .items
        .iter()
        .map(|item| ManageMoveRequest {
            from_lib: req.from_lib.clone(),
            from_folder: item.folder.clone(),
            to_lib: req.to_lib.clone(),
            to_folder: item.to_folder.clone(),
            item_id: item.item_id.clone(),
            reason: req.reason.clone(),
            on_conflict: req.on_conflict.clone(),
        })
        .collect::<Vec<_>>();
    let params = serde_json::to_value(&req).unwrap_or_else(|_| serde_json::json!({}));
    let label = format!(
        "批量移动: {} -> {} ({} 项)",
        req.from_lib.trim(),
        req.to_lib.trim(),
        items.len()
    );
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "manage_move_batch_execute",
        &label,
        items.len() as i64,
        "manual",
        params,
    )
    .await?;
    spawn_move_batch_execute(
        state,
        task.id,
        emby_url,
        api_key,
        req.from_lib,
        req.to_lib,
        from_lib_folder,
        to_lib_folder,
        items,
    );
    Ok(Json(task))
}

fn ensure_api_key_configured(api_key: &str) -> AppResult<()> {
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; set it via /api/v2/config before listing Emby libraries"
                .to_string(),
        ));
    }
    Ok(())
}

fn normalize_library_collection_type(value: &str) -> AppResult<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "movies" => Ok("movies"),
        "tvshows" => Ok("tvshows"),
        _ => Err(AppError::BadRequest("类型只能 tvshows/movies".to_string())),
    }
}

fn library_item_types(library_type: &str) -> &'static str {
    if library_type.eq_ignore_ascii_case("tvshows") {
        "Series"
    } else {
        "Movie"
    }
}

fn folder_from_library_path(path: Option<&str>, library: &EmbyLibrary) -> String {
    let Some(path) = path else {
        return String::new();
    };
    for root in &library.paths {
        let root = root.trim_end_matches('/');
        if root.is_empty() {
            continue;
        }
        if let Some(rest) = path.strip_prefix(root)
            && let Some(rest) = rest.strip_prefix('/')
        {
            return rest.split('/').next().unwrap_or_default().to_string();
        }
    }
    let library_folder = library
        .paths
        .iter()
        .filter_map(|root| {
            root.trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
        })
        .next()
        .unwrap_or(&library.name);
    let sep = format!("/{library_folder}/");
    path.split_once(&sep)
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default().to_string())
        .unwrap_or_default()
}

pub fn library_folder_name(library: &EmbyLibrary) -> String {
    library
        .paths
        .iter()
        .find_map(|path| folder_from_strm_path(path))
        .or_else(|| {
            library.paths.iter().find_map(|path| {
                let trimmed = path.trim().trim_end_matches('/');
                (!trimmed.is_empty()).then(|| {
                    trimmed
                        .rsplit('/')
                        .next()
                        .unwrap_or(&library.name)
                        .to_string()
                })
            })
        })
        .unwrap_or_else(|| library.name.clone())
}

fn folder_from_strm_path(path: &str) -> Option<String> {
    let normalized = path.trim().trim_end_matches('/');
    let (_, rest) = normalized.split_once("/strm/")?;
    rest.split('/')
        .next()
        .filter(|folder| !folder.trim().is_empty())
        .map(ToString::to_string)
}

async fn resolve_library_folder_for_roots(
    state: &AppState,
    requested: &str,
    roots: &[&Path],
) -> AppResult<String> {
    let requested = required_segment(requested, "lib")?;
    for root in roots {
        if safe_under(root, requested)?.exists() {
            return Ok(requested.to_string());
        }
    }

    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        return Ok(requested.to_string());
    }
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let requested_lower = requested.to_ascii_lowercase();
    match client.libraries().await {
        Ok(libraries) => Ok(libraries
            .iter()
            .find(|library| {
                library.name.eq_ignore_ascii_case(requested)
                    || library.id.as_deref().is_some_and(|id| id == requested)
                    || library_folder_name(library).to_ascii_lowercase() == requested_lower
            })
            .map(library_folder_name)
            .unwrap_or_else(|| requested.to_string())),
        Err(_) => Ok(requested.to_string()),
    }
}

fn spawn_manage_preview(
    state: AppState,
    id: Uuid,
    operation: &'static str,
    planned_paths: Vec<PathBuf>,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, id, "任务并发槽不可用", None).await;
            return;
        };
        if tasks::cancel_requested(&state.pool, id).await {
            let _ = tasks::finish_cancelled(&state.pool, id).await;
            return;
        }
        let _ = tasks::mark_running(&state.pool, id, "生成安全预览").await;
        if tasks::cancel_requested(&state.pool, id).await {
            let _ = tasks::finish_cancelled(&state.pool, id).await;
            return;
        }
        let _ = tasks::set_progress(&state.pool, id, 1, "preview worker dry run").await;
        if tasks::cancel_requested(&state.pool, id).await {
            let _ = tasks::finish_cancelled(&state.pool, id).await;
            return;
        }
        let result = ManagePreviewResult {
            ok: true,
            preview: true,
            dry_run: true,
            operation: operation.to_string(),
            planned_paths: planned_paths
                .into_iter()
                .map(|path| path.display().to_string())
                .collect(),
            warnings: vec![
                "dry-run preview only; did not touch filesystem or Emby".to_string(),
                "真实执行已接入，请确认后调用对应 execute 端点".to_string(),
            ],
            next_steps: vec![
                "核对 planned_paths 是否符合预期".to_string(),
                "确认后使用 /api/v2/manage/delete/execute 或 /api/v2/manage/move/execute"
                    .to_string(),
            ],
            message: "dry-run preview only; did not touch filesystem or Emby".to_string(),
        };
        let _ = tasks::finish_done_with_message(
            &state.pool,
            id,
            "preview worker dry run done",
            serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
        )
        .await;
    });
}

fn spawn_delete_execute(
    state: AppState,
    id: Uuid,
    emby_url: String,
    api_key: String,
    req: ManageDeleteRequest,
    plan: DeleteExecutePlan,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, id, "任务并发槽不可用", None).await;
            return;
        };
        if tasks::cancel_requested(&state.pool, id).await {
            let _ = tasks::finish_cancelled(&state.pool, id).await;
            return;
        }
        let _ = tasks::mark_running(&state.pool, id, "先删除 Emby Item").await;
        let client = EmbyClient::new(emby_url, api_key, state.http.clone());
        match run_delete_execute(&state, id, &client, req, plan).await {
            Ok(result) => {
                let _ = tasks::finish_done(
                    &state.pool,
                    id,
                    serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
                )
                .await;
            }
            Err(err) if err.to_string() == SCAN_TASK_CANCELLED_SENTINEL => {}
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, id, &err.to_string(), None).await;
            }
        }
    });
}

fn spawn_delete_batch_execute(
    state: AppState,
    id: Uuid,
    emby_url: String,
    api_key: String,
    items: Vec<(ManageDeleteRequest, DeleteExecutePlan)>,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, id, "任务并发槽不可用", None).await;
            return;
        };
        let _ = tasks::mark_running(&state.pool, id, "批量删除启动").await;
        let client = EmbyClient::new(emby_url, api_key, state.http.clone());
        let total = items.len();
        let mut results = Vec::new();

        for (index, (req, plan)) in items.into_iter().enumerate() {
            if tasks::cancel_requested(&state.pool, id).await {
                let _ = tasks::finish_cancelled(&state.pool, id).await;
                return;
            }
            let _ = tasks::set_progress(
                &state.pool,
                id,
                index as i64,
                &format!("删除 {}/{}", req.lib.trim(), req.folder.trim()),
            )
            .await;
            let lib = req.lib.trim().to_string();
            let folder = req.folder.trim().to_string();
            match run_delete_execute_core(&state, &client, req, plan).await {
                Ok(result) => results.push(ManageDeleteBatchItemResult {
                    lib,
                    folder,
                    ok: true,
                    result: Some(result),
                    err: None,
                }),
                Err(err) => results.push(ManageDeleteBatchItemResult {
                    lib,
                    folder,
                    ok: false,
                    result: None,
                    err: Some(err.to_string()),
                }),
            }
            let _ = tasks::set_progress(
                &state.pool,
                id,
                (index + 1) as i64,
                &format!("批量删除 {}/{}", index + 1, total),
            )
            .await;
        }

        let ok_count = results.iter().filter(|item| item.ok).count();
        let result = ManageDeleteBatchResult {
            ok: ok_count == total,
            total,
            ok_count,
            error_count: total.saturating_sub(ok_count),
            results,
        };
        let _ = tasks::finish_done_with_message(
            &state.pool,
            id,
            &format!("批量删除完成: {ok_count}/{total}"),
            serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
        )
        .await;
    });
}

async fn run_delete_execute(
    state: &AppState,
    id: Uuid,
    client: &EmbyClient,
    req: ManageDeleteRequest,
    plan: DeleteExecutePlan,
) -> AppResult<ManageDeleteExecuteResult> {
    tasks::set_total(&state.pool, id, 4).await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(SCAN_TASK_CANCELLED_SENTINEL.to_string()));
    }

    let mut emby_gone = true;
    if let Some(item_id) = req.item_id.as_deref().and_then(non_empty_trimmed) {
        client.delete_item(item_id).await?;
        emby_gone = verify_emby_delete(client, item_id).await;
    }
    tasks::set_progress(&state.pool, id, 1, "Emby Item 删除请求已完成").await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(SCAN_TASK_CANCELLED_SENTINEL.to_string()));
    }

    let deleted_from = delete_planned_paths(&plan).await?;
    tasks::set_progress(&state.pool, id, 2, "磁盘删除已完成").await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(SCAN_TASK_CANCELLED_SENTINEL.to_string()));
    }

    let notified = if deleted_from.is_empty() {
        false
    } else {
        client.notify_media_deleted(&plan.emby_deleted_path).await?;
        true
    };
    tasks::set_progress(&state.pool, id, 3, "Emby Deleted 通知已处理").await?;

    let undo_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "lib": req.lib.trim(),
        "folder": req.folder.trim(),
        "deleted_from": deleted_from,
    });
    sqlx::query("INSERT INTO undo_entries(id, op, payload) VALUES ($1, 'delete', $2)")
        .bind(undo_id)
        .bind(payload)
        .execute(&state.pool)
        .await?;
    tasks::set_progress(&state.pool, id, 4, "undo log 已写入").await?;

    Ok(ManageDeleteExecuteResult {
        ok: true,
        preview: false,
        dry_run: false,
        operation: "delete".to_string(),
        folder: req.folder.trim().to_string(),
        emby_gone,
        deleted_from,
        notified,
        undo_id,
    })
}

async fn run_delete_execute_core(
    state: &AppState,
    client: &EmbyClient,
    req: ManageDeleteRequest,
    plan: DeleteExecutePlan,
) -> AppResult<ManageDeleteExecuteResult> {
    run_delete_execute_core_with_options(state, client, req, plan, true).await
}

async fn run_delete_execute_core_with_options(
    state: &AppState,
    client: &EmbyClient,
    req: ManageDeleteRequest,
    plan: DeleteExecutePlan,
    strict_emby_delete: bool,
) -> AppResult<ManageDeleteExecuteResult> {
    let mut emby_gone = true;
    if let Some(item_id) = req.item_id.as_deref().and_then(non_empty_trimmed) {
        match client.delete_item(item_id).await {
            Ok(_) => {
                emby_gone = verify_emby_delete(client, item_id).await;
            }
            Err(err) if strict_emby_delete => return Err(err.into()),
            Err(err) => {
                tracing::warn!(
                    item_id,
                    error = %err,
                    "Emby item delete failed; continuing direct disk cleanup"
                );
                emby_gone = verify_emby_delete(client, item_id).await;
            }
        }
    }
    let deleted_from = delete_planned_paths_with_options(&plan, !strict_emby_delete).await?;
    let notified = if deleted_from.is_empty() {
        false
    } else {
        client.notify_media_deleted(&plan.emby_deleted_path).await?;
        true
    };

    let undo_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "lib": req.lib.trim(),
        "folder": req.folder.trim(),
        "deleted_from": deleted_from,
    });
    sqlx::query("INSERT INTO undo_entries(id, op, payload) VALUES ($1, 'delete', $2)")
        .bind(undo_id)
        .bind(payload)
        .execute(&state.pool)
        .await?;

    Ok(ManageDeleteExecuteResult {
        ok: true,
        preview: false,
        dry_run: false,
        operation: "delete".to_string(),
        folder: req.folder.trim().to_string(),
        emby_gone,
        deleted_from,
        notified,
        undo_id,
    })
}

pub async fn execute_delete_direct(
    state: &AppState,
    client: &EmbyClient,
    req: ManageDeleteRequest,
) -> AppResult<ManageDeleteExecuteResult> {
    let lib_folder = resolve_library_folder_for_roots(
        state,
        &req.lib,
        &[&state.settings.strm_root, &state.settings.cd_root],
    )
    .await?;
    let plan = plan_delete_execute(state, &req, &lib_folder)?;
    run_delete_execute_core_with_options(state, client, req, plan, false).await
}

fn spawn_move_execute(
    state: AppState,
    id: Uuid,
    emby_url: String,
    api_key: String,
    req: ManageMoveRequest,
    plan: MoveExecutePlan,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, id, "任务并发槽不可用", None).await;
            return;
        };
        if tasks::cancel_requested(&state.pool, id).await {
            let _ = tasks::finish_cancelled(&state.pool, id).await;
            return;
        }
        let _ = tasks::mark_running(&state.pool, id, "准备移动").await;
        let client = EmbyClient::new(emby_url, api_key, state.http.clone());
        match run_move_execute(&state, id, &client, req, plan).await {
            Ok(result) => {
                let _ = tasks::finish_done(
                    &state.pool,
                    id,
                    serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
                )
                .await;
            }
            Err(err) if err.to_string() == SCAN_TASK_CANCELLED_SENTINEL => {}
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, id, &err.to_string(), None).await;
            }
        }
    });
}

fn spawn_move_batch_execute(
    state: AppState,
    id: Uuid,
    emby_url: String,
    api_key: String,
    from_lib: String,
    to_lib: String,
    from_lib_folder: String,
    to_lib_folder: String,
    items: Vec<ManageMoveRequest>,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, id, "任务并发槽不可用", None).await;
            return;
        };
        if tasks::cancel_requested(&state.pool, id).await {
            let _ = tasks::finish_cancelled(&state.pool, id).await;
            return;
        }
        let _ = tasks::mark_running(&state.pool, id, "批量移动启动").await;
        let client = EmbyClient::new(emby_url, api_key, state.http.clone());
        let total = items.len();
        let mut results = Vec::with_capacity(total);

        for (index, req) in items.into_iter().enumerate() {
            if tasks::cancel_requested(&state.pool, id).await {
                let _ = tasks::finish_cancelled(&state.pool, id).await;
                return;
            }
            let folder = req.from_folder.trim().to_string();
            let _ = tasks::set_progress(
                &state.pool,
                id,
                index as i64,
                &format!("移动 {}", truncate_task_label(&folder, 40)),
            )
            .await;
            let result = match plan_move_execute(&state, &req, &from_lib_folder, &to_lib_folder) {
                Ok(plan) => match run_move_execute_core(&state, &client, req, plan).await {
                    Ok(result) => ManageMoveBatchItemResult {
                        folder,
                        ok: result.ok,
                        skipped: result.skipped,
                        smart_action: result.smart_action.clone(),
                        result: Some(result),
                        err: None,
                    },
                    Err(err) => ManageMoveBatchItemResult {
                        folder,
                        ok: false,
                        skipped: false,
                        smart_action: None,
                        result: None,
                        err: Some(err.to_string()),
                    },
                },
                Err(err) => ManageMoveBatchItemResult {
                    folder,
                    ok: false,
                    skipped: false,
                    smart_action: None,
                    result: None,
                    err: Some(err.to_string()),
                },
            };
            results.push(result);
            let _ = tasks::set_progress(
                &state.pool,
                id,
                (index + 1) as i64,
                &format!("批量移动 {}/{}", index + 1, total),
            )
            .await;
        }

        let ok_count = results.iter().filter(|item| item.ok).count();
        let smart_count = results
            .iter()
            .filter(|item| item.smart_action.is_some())
            .count();
        let result = ManageMoveBatchResult {
            ok: ok_count == total,
            from_lib,
            to_lib,
            total,
            ok_count,
            error_count: total.saturating_sub(ok_count),
            smart_count,
            results,
        };
        let _ = tasks::finish_done(
            &state.pool,
            id,
            serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
        )
        .await;
    });
}

async fn verify_emby_delete(client: &EmbyClient, item_id: &str) -> bool {
    match client.item_exists(item_id).await {
        Ok(false) => true,
        Ok(true) => {
            sleep(Duration::from_millis(500)).await;
            let _ = client.delete_item(item_id).await;
            match client.item_exists(item_id).await {
                Ok(exists) => !exists,
                Err(_) => true,
            }
        }
        Err(_) => true,
    }
}

async fn run_move_execute(
    state: &AppState,
    id: Uuid,
    client: &EmbyClient,
    req: ManageMoveRequest,
    plan: MoveExecutePlan,
) -> AppResult<ManageMoveExecuteResult> {
    tasks::set_total(&state.pool, id, 5).await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(SCAN_TASK_CANCELLED_SENTINEL.to_string()));
    }
    tasks::set_progress(&state.pool, id, 1, "目标库已确认").await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(SCAN_TASK_CANCELLED_SENTINEL.to_string()));
    }
    let result = run_move_execute_core(state, client, req, plan).await?;
    tasks::set_progress(&state.pool, id, 2, "媒体目录已移动").await?;
    tasks::set_progress(&state.pool, id, 3, "STRM 已重建").await?;
    tasks::set_progress(&state.pool, id, 4, "旧路径通知已处理").await?;
    tasks::set_progress(&state.pool, id, 5, "目标库刷新并写入 undo").await?;

    Ok(result)
}

async fn run_move_execute_core(
    state: &AppState,
    client: &EmbyClient,
    req: ManageMoveRequest,
    plan: MoveExecutePlan,
) -> AppResult<ManageMoveExecuteResult> {
    let target_library_id = resolve_library_item_id(client, &plan.to_lib).await?;
    let conflict_mode = move_conflict_mode(req.on_conflict.as_deref())?;
    if plan.to_cd_target.exists() {
        match conflict_mode {
            MoveConflictMode::Error => {
                return Err(AppError::Conflict(format!(
                    "目标已存在同名文件夹: {}",
                    plan.to_cd_target.display()
                )));
            }
            MoveConflictMode::Skip => {
                let undo_id = Uuid::new_v4();
                return Ok(ManageMoveExecuteResult {
                    ok: false,
                    preview: false,
                    dry_run: false,
                    operation: "move".to_string(),
                    from_lib: plan.from_lib,
                    from_folder: req.from_folder.trim().to_string(),
                    to_lib: plan.to_lib,
                    to_folder: plan.to_folder,
                    moved: false,
                    skipped: true,
                    smart_action: None,
                    msg: Some("目标已存在同名文件夹(skip)".to_string()),
                    src_count: None,
                    dst_count: None,
                    old_strm_removed: false,
                    strm_written: 0,
                    emby_gone: true,
                    notified: false,
                    refresh_code: None,
                    undo_id,
                });
            }
            MoveConflictMode::Smart => {
                if let Some(result) = handle_smart_move_conflict(state, client, &req, &plan).await?
                {
                    return Ok(result);
                }
            }
        }
    }

    move_cd_folder(&plan).await?;
    let old_strm_removed = remove_path_if_exists(&plan.from_strm_target).await?;
    let strm_written = rebuild_strm_for_moved_folder(&plan)?;

    let mut emby_gone = true;
    if let Some(item_id) = req.item_id.as_deref().and_then(non_empty_trimmed) {
        match client.delete_item(item_id).await {
            Ok(_) => emby_gone = verify_emby_delete(client, item_id).await,
            Err(_) => emby_gone = false,
        }
    }
    let notified = client
        .notify_media_deleted(&plan.from_emby_path)
        .await
        .is_ok();

    let refresh_code = client
        .refresh_item(&target_library_id, true, false)
        .await
        .ok();
    let undo_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "from": req.from_lib.trim(),
        "to": req.to_lib.trim(),
        "folder": req.from_folder.trim(),
        "to_folder": plan.to_folder,
        "emby_id": req.item_id.as_deref().and_then(non_empty_trimmed),
        "strm_count": strm_written,
        "on_conflict": req.on_conflict.as_deref(),
    });
    sqlx::query("INSERT INTO undo_entries(id, op, payload) VALUES ($1, 'move', $2)")
        .bind(undo_id)
        .bind(payload)
        .execute(&state.pool)
        .await?;

    Ok(ManageMoveExecuteResult {
        ok: true,
        preview: false,
        dry_run: false,
        operation: "move".to_string(),
        from_lib: plan.from_lib,
        from_folder: req.from_folder.trim().to_string(),
        to_lib: plan.to_lib,
        to_folder: plan.to_folder,
        moved: true,
        skipped: false,
        smart_action: None,
        msg: None,
        src_count: None,
        dst_count: None,
        old_strm_removed,
        strm_written,
        emby_gone,
        notified,
        refresh_code,
        undo_id,
    })
}

async fn handle_smart_move_conflict(
    state: &AppState,
    client: &EmbyClient,
    req: &ManageMoveRequest,
    plan: &MoveExecutePlan,
) -> AppResult<Option<ManageMoveExecuteResult>> {
    let mut src_count = count_strm_files(&plan.from_strm_target);
    let dst_count = count_strm_files(&plan.to_strm_target);
    if src_count == 0 || dst_count == 0 {
        return Err(AppError::Conflict(
            "源或目标的 strm 未生成，拒绝智能判定；请先扫描相关库再归档".to_string(),
        ));
    }
    if src_count == dst_count {
        let src_quality = folder_max_quality_score(&plan.from_strm_target);
        let dst_quality = folder_max_quality_score(&plan.to_strm_target);
        if src_quality > dst_quality {
            src_count = dst_count + 1;
        }
    }
    if src_count > dst_count {
        delete_path_if_exists(&plan.to_cd_target).await?;
        let _ = remove_path_if_exists(&plan.to_strm_target).await?;
        let _ = client.notify_media_deleted(&plan.to_emby_path).await;
        return Ok(None);
    }

    delete_path_if_exists(&plan.from_cd_target).await?;
    let old_strm_removed = remove_path_if_exists(&plan.from_strm_target).await?;
    let mut emby_gone = true;
    if let Some(item_id) = req.item_id.as_deref().and_then(non_empty_trimmed) {
        match client.delete_item(item_id).await {
            Ok(_) => emby_gone = verify_emby_delete(client, item_id).await,
            Err(_) => emby_gone = false,
        }
    }
    let notified = client
        .notify_media_deleted(&plan.from_emby_path)
        .await
        .is_ok();
    let undo_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "from": req.from_lib.trim(),
        "to": req.to_lib.trim(),
        "folder": req.from_folder.trim(),
        "to_folder": plan.to_folder,
        "emby_id": req.item_id.as_deref().and_then(non_empty_trimmed),
        "action": "deleted_source",
        "src_n": src_count,
        "dst_n": dst_count,
    });
    sqlx::query("INSERT INTO undo_entries(id, op, payload) VALUES ($1, 'smart_archive', $2)")
        .bind(undo_id)
        .bind(payload)
        .execute(&state.pool)
        .await?;

    Ok(Some(ManageMoveExecuteResult {
        ok: true,
        preview: false,
        dry_run: false,
        operation: "move".to_string(),
        from_lib: plan.from_lib.clone(),
        from_folder: req.from_folder.trim().to_string(),
        to_lib: plan.to_lib.clone(),
        to_folder: plan.to_folder.clone(),
        moved: false,
        skipped: false,
        smart_action: Some("deleted_source".to_string()),
        msg: Some(format!(
            "源 {src_count} 集 ≤ 目标 {dst_count} 集，删源保留目标"
        )),
        src_count: Some(src_count),
        dst_count: Some(dst_count),
        old_strm_removed,
        strm_written: 0,
        emby_gone,
        notified,
        refresh_code: None,
        undo_id,
    }))
}

async fn delete_planned_paths(plan: &DeleteExecutePlan) -> AppResult<Vec<String>> {
    delete_planned_paths_with_options(plan, false).await
}

async fn delete_planned_paths_with_options(
    plan: &DeleteExecutePlan,
    ignore_media_errors: bool,
) -> AppResult<Vec<String>> {
    let mut deleted_from = Vec::new();
    for (path, label) in [(&plan.cd_target, "115"), (&plan.strm_target, "strm")] {
        if !path.exists() {
            continue;
        }
        let removed = async {
            let metadata = tokio::fs::symlink_metadata(path).await?;
            if metadata.is_dir() {
                tokio::fs::remove_dir_all(path).await
            } else {
                tokio::fs::remove_file(path).await
            }
        }
        .await;
        match removed {
            Ok(()) => deleted_from.push(label.to_string()),
            Err(err) if ignore_media_errors && label == "115" => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "media delete failed; continuing STRM cleanup"
                );
            }
            Err(err) => return Err(anyhow::Error::from(err).into()),
        }
    }
    Ok(deleted_from)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoveConflictMode {
    Error,
    Skip,
    Smart,
}

fn move_conflict_mode(value: Option<&str>) -> AppResult<MoveConflictMode> {
    match value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("error")
        .to_ascii_lowercase()
        .as_str()
    {
        "error" => Ok(MoveConflictMode::Error),
        "skip" => Ok(MoveConflictMode::Skip),
        "smart" => Ok(MoveConflictMode::Smart),
        other => Err(AppError::BadRequest(format!(
            "未知 on_conflict 模式: {other}"
        ))),
    }
}

async fn resolve_library_item_id(client: &EmbyClient, name: &str) -> AppResult<String> {
    let requested = name.to_ascii_lowercase();
    let library = client
        .libraries()
        .await?
        .into_iter()
        .find(|item| {
            item.name.eq_ignore_ascii_case(name)
                || item.id.as_deref().is_some_and(|id| id == name)
                || library_folder_name(item).to_ascii_lowercase() == requested
        })
        .ok_or_else(|| AppError::NotFound(format!("Emby library not found: {name}")))?;
    library
        .id
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "Emby library {name} has no ItemId; cannot refresh target after move"
            ))
        })
}

async fn move_cd_folder(plan: &MoveExecutePlan) -> AppResult<()> {
    if !plan.from_cd_target.is_dir() {
        return Err(AppError::NotFound(format!(
            "源 115 文件夹不存在: {}",
            plan.from_cd_target.display()
        )));
    }
    if plan.to_cd_target.exists() {
        return Err(AppError::Conflict(format!(
            "目标已存在同名文件夹: {}",
            plan.to_cd_target.display()
        )));
    }
    if let Some(parent) = plan.to_cd_target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(anyhow::Error::from)?;
    }
    tokio::fs::rename(&plan.from_cd_target, &plan.to_cd_target)
        .await
        .map_err(anyhow::Error::from)?;
    Ok(())
}

async fn delete_path_if_exists(path: &Path) -> AppResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(anyhow::Error::from)?;
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(path)
            .await
            .map_err(anyhow::Error::from)?;
    } else {
        tokio::fs::remove_file(path)
            .await
            .map_err(anyhow::Error::from)?;
    }
    Ok(true)
}

async fn remove_path_if_exists(path: &Path) -> AppResult<bool> {
    delete_path_if_exists(path).await
}

fn rebuild_strm_for_moved_folder(plan: &MoveExecutePlan) -> AppResult<usize> {
    std::fs::create_dir_all(&plan.to_strm_lib).map_err(|err| AppError::Anyhow(err.into()))?;
    chmod_public_dir(&plan.to_strm_lib);
    let mut written = 0usize;
    for entry in WalkDir::new(&plan.to_cd_target)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() || !is_video_file(entry.path()) {
            continue;
        }
        let Ok(root_rel) = entry
            .path()
            .parent()
            .unwrap_or(&plan.to_cd_target)
            .strip_prefix(&plan.to_cd_lib)
        else {
            continue;
        };
        let Some(filename) = entry.path().file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if write_strm(&plan.to_strm_lib, &plan.to_lib_folder, root_rel, filename)? {
            written += 1;
        }
    }
    Ok(written)
}

fn count_strm_files(folder: &Path) -> usize {
    if !folder.is_dir() {
        return 0;
    }
    WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && has_extension(entry.path(), "strm"))
        .count()
}

fn folder_max_quality_score(folder: &Path) -> i64 {
    if !folder.is_dir() {
        return 0;
    }
    WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && has_extension(entry.path(), "strm"))
        .map(|entry| {
            std::fs::read_to_string(entry.path())
                .map(|content| quality_score(&content))
                .unwrap_or_else(|_| quality_score(&entry.file_name().to_string_lossy()))
        })
        .max()
        .unwrap_or(0)
}

fn quality_score(value: &str) -> i64 {
    let lower = value.to_ascii_lowercase();
    let mut score = 0;
    if lower.contains("2160p") || lower.contains("4k") {
        score += 4000;
    } else if lower.contains("1080p") {
        score += 1080;
    } else if lower.contains("720p") {
        score += 720;
    }
    if lower.contains("remux") {
        score += 200;
    }
    if lower.contains("bluray") || lower.contains("blu-ray") {
        score += 120;
    }
    if lower.contains("web-dl") || lower.contains("webrip") {
        score += 80;
    }
    if lower.contains("x265") || lower.contains("h265") || lower.contains("hevc") {
        score += 30;
    }
    if lower.contains("hdr") || lower.contains("dolby vision") || lower.contains("dv.") {
        score += 20;
    }
    score
}

fn truncate_task_label(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push('…');
    }
    out
}

fn plan_delete(
    state: &AppState,
    req: &ManageDeleteRequest,
    lib_folder: &str,
) -> AppResult<Vec<PathBuf>> {
    let lib_root = safe_under(
        &state.settings.strm_root,
        required_segment(lib_folder, "lib")?,
    )?;
    let target = safe_under(&lib_root, required_segment(&req.folder, "folder")?)?;
    Ok(vec![target])
}

#[derive(Debug, Clone)]
struct DeleteExecutePlan {
    cd_target: PathBuf,
    strm_target: PathBuf,
    emby_deleted_path: String,
}

fn plan_delete_execute(
    state: &AppState,
    req: &ManageDeleteRequest,
    lib_folder: &str,
) -> AppResult<DeleteExecutePlan> {
    let lib = required_segment(lib_folder, "lib")?;
    let folder = required_segment(&req.folder, "folder")?;
    let cd_lib = safe_under(&state.settings.cd_root, lib)?;
    let strm_lib = safe_under(&state.settings.strm_root, lib)?;
    Ok(DeleteExecutePlan {
        cd_target: safe_under(&cd_lib, folder)?,
        strm_target: safe_under(&strm_lib, folder)?,
        emby_deleted_path: format!("/strm/{lib}/{folder}"),
    })
}

fn plan_move(
    state: &AppState,
    req: &ManageMoveRequest,
    from_lib_folder: &str,
    to_lib_folder: &str,
) -> AppResult<Vec<PathBuf>> {
    let from_lib = safe_under(
        &state.settings.strm_root,
        required_segment(from_lib_folder, "from_lib")?,
    )?;
    let from = safe_under(
        &from_lib,
        required_segment(&req.from_folder, "from_folder")?,
    )?;
    let to_lib = safe_under(
        &state.settings.strm_root,
        required_segment(to_lib_folder, "to_lib")?,
    )?;
    let to = if let Some(folder) = req.to_folder.as_deref().and_then(non_empty_trimmed) {
        safe_under(&to_lib, folder)?
    } else {
        let name = Path::new(required_segment(&req.from_folder, "from_folder")?)
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| AppError::BadRequest("from_folder 缺少可用末级目录名".to_string()))?;
        safe_under(&to_lib, name)?
    };
    Ok(vec![from, to])
}

#[derive(Debug, Clone)]
struct MoveExecutePlan {
    from_lib: String,
    to_lib: String,
    to_lib_folder: String,
    to_folder: String,
    from_cd_target: PathBuf,
    to_cd_lib: PathBuf,
    to_cd_target: PathBuf,
    from_strm_target: PathBuf,
    to_strm_lib: PathBuf,
    to_strm_target: PathBuf,
    from_emby_path: String,
    to_emby_path: String,
}

fn plan_move_execute(
    state: &AppState,
    req: &ManageMoveRequest,
    from_lib_folder: &str,
    to_lib_folder: &str,
) -> AppResult<MoveExecutePlan> {
    let from_lib = required_segment(from_lib_folder, "from_lib")?;
    let from_folder = required_segment(&req.from_folder, "from_folder")?;
    let to_lib = required_segment(to_lib_folder, "to_lib")?;
    let to_folder = move_target_folder(req)?;

    let from_cd_lib = safe_under(&state.settings.cd_root, from_lib)?;
    let to_cd_lib = safe_under(&state.settings.cd_root, to_lib)?;
    let from_strm_lib = safe_under(&state.settings.strm_root, from_lib)?;
    let to_strm_lib = safe_under(&state.settings.strm_root, to_lib)?;

    let from_cd_target = safe_under(&from_cd_lib, from_folder)?;
    let to_cd_target = safe_under(&to_cd_lib, &to_folder)?;
    let from_strm_target = safe_under(&from_strm_lib, from_folder)?;
    let to_strm_target = safe_under(&to_strm_lib, &to_folder)?;

    Ok(MoveExecutePlan {
        from_lib: req.from_lib.trim().to_string(),
        to_lib: req.to_lib.trim().to_string(),
        to_lib_folder: to_lib.to_string(),
        to_folder: to_folder.clone(),
        from_cd_target,
        to_cd_lib,
        to_cd_target,
        from_strm_target,
        to_strm_lib,
        to_strm_target,
        from_emby_path: format!("/strm/{from_lib}/{from_folder}"),
        to_emby_path: format!("/strm/{to_lib}/{to_folder}"),
    })
}

fn move_target_folder(req: &ManageMoveRequest) -> AppResult<String> {
    if let Some(folder) = req.to_folder.as_deref().and_then(non_empty_trimmed) {
        required_segment(folder, "to_folder")?;
        return Ok(folder.to_string());
    }
    let name = Path::new(required_segment(&req.from_folder, "from_folder")?)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::BadRequest("from_folder 缺少可用末级目录名".to_string()))?;
    Ok(name.to_string())
}

fn required_segment<'a>(value: &'a str, field: &str) -> AppResult<&'a str> {
    non_empty_trimmed(value).ok_or_else(|| AppError::BadRequest(format!("{field} 不能为空")))
}

fn spawn_scan_task(
    state: AppState,
    id: Uuid,
    emby_url: String,
    api_key: String,
    req: ScanLibraryRequest,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, id, "任务并发槽不可用", None).await;
            return;
        };
        if tasks::cancel_requested(&state.pool, id).await {
            let _ = tasks::finish_cancelled(&state.pool, id).await;
            return;
        }
        let _ = tasks::mark_running(&state.pool, id, "准备扫描").await;
        let client = EmbyClient::new(emby_url, api_key, state.http.clone());
        match run_scan_task(&state, id, &client, req).await {
            Ok(result) => {
                let _ = tasks::finish_done(
                    &state.pool,
                    id,
                    serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
                )
                .await;
            }
            Err(err) if err.to_string() == SCAN_TASK_CANCELLED_SENTINEL => {}
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, id, &err.to_string(), None).await;
            }
        }
    });
}

fn spawn_scan_all_task(
    state: AppState,
    id: Uuid,
    label: String,
    emby_url: String,
    api_key: String,
    params: Value,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, id, "任务并发槽不可用", None).await;
            return;
        };
        if tasks::cancel_requested(&state.pool, id).await {
            let _ = tasks::finish_cancelled(&state.pool, id).await;
            return;
        }
        let _ = tasks::mark_running(&state.pool, id, "手动扫全库启动").await;
        let client = EmbyClient::new(emby_url, api_key, state.http.clone());
        match run_scheduled_scan_all_libraries(&state, id, &client, &params).await {
            Ok(detail) => {
                let message = "手动扫全库完成";
                let _ = tasks::finish_done_with_message(
                    &state.pool,
                    id,
                    message,
                    serde_json::json!({
                        "ok": true,
                        "preview": false,
                        "dry_run": false,
                        "source": "manual",
                        "kind": "scan_all",
                        "label": label,
                        "message": message,
                        "detail": detail,
                    }),
                )
                .await;
            }
            Err(AppError::Conflict(message)) if message == SCAN_TASK_CANCELLED_SENTINEL => {
                let _ = tasks::finish_cancelled(&state.pool, id).await;
            }
            Err(err) => {
                let message = err.to_string();
                let _ = tasks::finish_error(
                    &state.pool,
                    id,
                    &message,
                    Some(serde_json::json!({
                        "ok": false,
                        "preview": false,
                        "dry_run": false,
                        "source": "manual",
                        "kind": "scan_all",
                        "label": label,
                        "message": message,
                    })),
                )
                .await;
            }
        }
    });
}

async fn run_scan_task(
    state: &AppState,
    id: Uuid,
    client: &EmbyClient,
    req: ScanLibraryRequest,
) -> AppResult<ScanLibraryResult> {
    let recursive = req.recursive.unwrap_or(true);
    let full = req.full.unwrap_or(false);
    let requested = first_non_empty([req.item_id.clone(), req.lib.clone()]);
    if req.generate_strm.unwrap_or(false) {
        return run_strm_generate_task(state, id, client, req, requested, recursive, full).await;
    }
    let plan = resolve_scan_plan(client, &req).await?;

    match plan {
        ScanPlan::Global { reason } => {
            tasks::set_total(&state.pool, id, 1).await?;
            if tasks::cancel_requested(&state.pool, id).await {
                tasks::finish_cancelled(&state.pool, id).await?;
                return Err(AppError::Conflict(SCAN_TASK_CANCELLED_SENTINEL.to_string()));
            }
            tasks::set_progress(&state.pool, id, 0, &format!("全局刷新: {reason}")).await?;
            let code = client.refresh_library().await?;
            tasks::set_progress(&state.pool, id, 1, "全局刷新已触发").await?;
            Ok(ScanLibraryResult {
                ok: (200..300).contains(&code),
                mode: "global".to_string(),
                requested,
                triggered: 1,
                global_refresh: true,
                items: vec![ScanLibraryItemResult {
                    id: None,
                    name: reason,
                    code,
                }],
                strm: None,
            })
        }
        ScanPlan::Items(targets) => {
            tasks::set_total(&state.pool, id, targets.len() as i64).await?;
            let mut items = Vec::new();
            let total = targets.len();
            for (idx, target) in targets.into_iter().enumerate() {
                if tasks::cancel_requested(&state.pool, id).await {
                    tasks::finish_cancelled(&state.pool, id).await?;
                    return Err(AppError::Conflict(SCAN_TASK_CANCELLED_SENTINEL.to_string()));
                }
                tasks::set_progress(
                    &state.pool,
                    id,
                    idx as i64,
                    &format!("刷新 {}", target.name),
                )
                .await?;
                let code = client.refresh_item(&target.id, recursive, full).await?;
                items.push(ScanLibraryItemResult {
                    id: Some(target.id),
                    name: target.name,
                    code,
                });
                tasks::set_progress(
                    &state.pool,
                    id,
                    (idx + 1) as i64,
                    &format!("已刷新 {}/{}", idx + 1, total),
                )
                .await?;
            }
            Ok(ScanLibraryResult {
                ok: items.iter().all(|item| (200..300).contains(&item.code)),
                mode: "items".to_string(),
                requested,
                triggered: items.len(),
                global_refresh: false,
                items,
                strm: None,
            })
        }
    }
}

pub async fn run_scheduled_scan_all_libraries(
    state: &AppState,
    task_id: Uuid,
    client: &EmbyClient,
    params: &Value,
) -> AppResult<Value> {
    let Ok(_cloud_permit) = state.clouddrive_slot.clone().acquire_owned().await else {
        return Err(AppError::Conflict(
            "CloudDrive 任务并发槽不可用".to_string(),
        ));
    };
    let recursive = params_bool(params, "recursive", true);
    let full = params_bool(params, "full", false);
    let generate_strm = params_bool(params, "generate_strm", true);
    let cleanup_orphans = params_bool(params, "cleanup_orphans", true);
    let default_fullauto = config_store::get_raw(&state.pool, "auto_strm_fullauto")
        .await?
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let fullauto = params_bool(params, "fullauto", default_fullauto);
    let requested_libs = requested_scan_all_libraries(params);

    tasks::set_progress(&state.pool, task_id, 0, "读取 Emby 库").await?;
    let libraries = client
        .libraries()
        .await?
        .into_iter()
        .filter(|library| {
            let folder = library_folder_name(library).to_ascii_lowercase();
            requested_libs.is_empty()
                || requested_libs.contains(&library.name.to_ascii_lowercase())
                || requested_libs.contains(&folder)
                || library
                    .id
                    .as_deref()
                    .is_some_and(|id| requested_libs.contains(&id.to_ascii_lowercase()))
        })
        .collect::<Vec<_>>();
    tasks::set_total(&state.pool, task_id, libraries.len().max(1) as i64).await?;

    if libraries.is_empty() {
        tasks::set_progress(&state.pool, task_id, 1, "没有可扫描的 Emby 库").await?;
        return Ok(serde_json::json!({
            "action": "scan_all",
            "libs_scanned": 0,
            "new_count": 0,
            "orphans_cleaned": 0,
            "permissions_fixed": 0,
            "attention": [],
            "results": [],
            "generate_strm": generate_strm,
            "cleanup_orphans": cleanup_orphans,
            "fullauto": fullauto,
        }));
    }

    let mut results = Vec::new();
    let mut total_new = 0usize;
    let mut total_orphans = 0usize;
    let mut total_permissions = 0usize;
    let mut attention = Vec::<String>::new();
    let mut ok_count = 0usize;

    for (index, library) in libraries.iter().enumerate() {
        if tasks::cancel_requested(&state.pool, task_id).await {
            tasks::finish_cancelled(&state.pool, task_id).await?;
            return Err(AppError::Conflict(SCAN_TASK_CANCELLED_SENTINEL.to_string()));
        }
        tasks::set_progress(
            &state.pool,
            task_id,
            index as i64,
            &format!("扫 {}", library.name),
        )
        .await?;

        match run_scheduled_scan_one_library(
            state,
            client,
            library,
            recursive,
            full,
            generate_strm,
            cleanup_orphans,
            fullauto,
        )
        .await
        {
            Ok(result) => {
                ok_count += usize::from(result.ok);
                if let Some(strm) = &result.strm {
                    total_new += strm.new_count;
                    total_orphans += strm.orphans_cleaned;
                    total_permissions += strm.permissions_fixed;
                    attention.extend(
                        strm.attention
                            .iter()
                            .map(|item| format!("{}: {item}", library.name)),
                    );
                }
                results.push(serde_json::json!({
                    "lib": library.name,
                    "ok": result.ok,
                    "result": result,
                }));
            }
            Err(err) => {
                results.push(serde_json::json!({
                    "lib": library.name,
                    "ok": false,
                    "err": err.to_string(),
                }));
            }
        }

        tasks::set_progress(
            &state.pool,
            task_id,
            (index + 1) as i64,
            &format!("扫全库 {}/{}", index + 1, libraries.len()),
        )
        .await?;
        sleep(Duration::from_millis(500)).await;
    }

    Ok(serde_json::json!({
        "action": "scan_all",
        "libs_scanned": results.len(),
        "ok_count": ok_count,
        "error_count": results.len().saturating_sub(ok_count),
        "new_count": total_new,
        "orphans_cleaned": total_orphans,
        "permissions_fixed": total_permissions,
        "attention": attention,
        "results": results,
        "generate_strm": generate_strm,
        "cleanup_orphans": cleanup_orphans,
        "fullauto": fullauto,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn run_scheduled_scan_one_library(
    state: &AppState,
    client: &EmbyClient,
    library: &EmbyLibrary,
    recursive: bool,
    full: bool,
    generate_strm: bool,
    cleanup_orphans: bool,
    fullauto: bool,
) -> AppResult<ScanLibraryResult> {
    let folder = library_folder_name(library);
    let mut strm = generate_strm
        .then(|| generate_missing_strm_for_library(state, &folder, None, fullauto))
        .transpose()?;
    if let Some(strm_result) = &mut strm {
        strm_result.lib.clone_from(&library.name);
    }
    if cleanup_orphans && let Some(strm_result) = &mut strm {
        let cleaned = cleanup_orphan_strm_for_library(state, &folder, None, strm_result)?;
        strm_result.orphans_cleaned = cleaned;
    }

    let should_refresh = if let Some(strm_result) = &strm {
        strm_result.new_count > 0
            || strm_result.orphans_cleaned > 0
            || strm_result.permissions_fixed > 0
    } else {
        true
    };
    let mut items = Vec::new();
    if should_refresh {
        let id = library.id.as_deref().filter(|id| !id.trim().is_empty());
        if let Some(id) = id {
            let code = client.refresh_item(id, recursive, full).await?;
            if let Some(strm_result) = &mut strm {
                strm_result.refreshed = (200..300).contains(&code);
                strm_result.refresh_code = Some(code);
            }
            items.push(ScanLibraryItemResult {
                id: Some(id.to_string()),
                name: library.name.clone(),
                code,
            });
        }
    }

    let ok = items.iter().all(|item| (200..300).contains(&item.code))
        && strm
            .as_ref()
            .is_none_or(|strm_result| !strm_result.orphan_cleanup_skipped);
    Ok(ScanLibraryResult {
        ok,
        mode: if generate_strm {
            "scan_all_strm_generate".to_string()
        } else {
            "scan_all_refresh".to_string()
        },
        requested: Some(library.name.clone()),
        triggered: items.len(),
        global_refresh: false,
        items,
        strm,
    })
}

fn params_bool(params: &Value, key: &str, default: bool) -> bool {
    params.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn requested_scan_all_libraries(params: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(lib) = params.get("lib").and_then(Value::as_str) {
        let lib = lib.trim();
        if !lib.is_empty() {
            out.insert(lib.to_ascii_lowercase());
        }
    }
    if let Some(libs) = params.get("libs").and_then(Value::as_array) {
        out.extend(libs.iter().filter_map(Value::as_str).filter_map(|lib| {
            let lib = lib.trim();
            (!lib.is_empty()).then(|| lib.to_ascii_lowercase())
        }));
    }
    out
}

async fn run_strm_generate_task(
    state: &AppState,
    id: Uuid,
    client: &EmbyClient,
    req: ScanLibraryRequest,
    requested: Option<String>,
    recursive: bool,
    full: bool,
) -> AppResult<ScanLibraryResult> {
    let lib = req
        .lib
        .as_deref()
        .and_then(non_empty_trimmed)
        .ok_or_else(|| AppError::BadRequest("生成 STRM 需要指定 lib".to_string()))?
        .to_string();
    let libraries = client.libraries().await?;
    let requested_lib = lib.to_ascii_lowercase();
    let library = libraries
        .iter()
        .find(|item| {
            item.name.eq_ignore_ascii_case(&lib)
                || item.id.as_deref().is_some_and(|id| id == lib)
                || library_folder_name(item).to_ascii_lowercase() == requested_lib
        })
        .ok_or_else(|| AppError::NotFound(format!("Emby library not found: {lib}")))?;
    let folder = library_folder_name(library);
    tasks::set_total(&state.pool, id, 2).await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(SCAN_TASK_CANCELLED_SENTINEL.to_string()));
    }
    tasks::set_progress(&state.pool, id, 0, "生成缺失 STRM").await?;
    let mut strm = generate_missing_strm_for_library(
        state,
        &folder,
        req.keyword.as_deref().and_then(non_empty_trimmed),
        req.fullauto.unwrap_or(false),
    )?;
    strm.lib.clone_from(&library.name);
    if req.cleanup_orphans.unwrap_or(false) {
        let cleaned = cleanup_orphan_strm_for_library(
            state,
            &folder,
            req.keyword.as_deref().and_then(non_empty_trimmed),
            &mut strm,
        )?;
        strm.orphans_cleaned = cleaned;
    }
    tasks::set_progress(&state.pool, id, 1, "STRM 生成完成").await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(SCAN_TASK_CANCELLED_SENTINEL.to_string()));
    }

    let mut items = Vec::new();
    if strm.new_count > 0 || strm.orphans_cleaned > 0 || strm.permissions_fixed > 0 {
        let plan = resolve_scan_plan(client, &req).await?;
        match plan {
            ScanPlan::Items(targets) => {
                if let Some(target) = targets.into_iter().next() {
                    tasks::set_progress(&state.pool, id, 1, &format!("刷新 {}", target.name))
                        .await?;
                    let code = client.refresh_item(&target.id, recursive, full).await?;
                    strm.refreshed = (200..300).contains(&code);
                    strm.refresh_code = Some(code);
                    items.push(ScanLibraryItemResult {
                        id: Some(target.id),
                        name: target.name,
                        code,
                    });
                }
            }
            ScanPlan::Global { .. } => {
                let code = client.refresh_library().await?;
                strm.refreshed = (200..300).contains(&code);
                strm.refresh_code = Some(code);
                items.push(ScanLibraryItemResult {
                    id: None,
                    name: "全局刷新".to_string(),
                    code,
                });
            }
        }
    }
    tasks::set_progress(&state.pool, id, 2, "STRM 任务完成").await?;
    Ok(ScanLibraryResult {
        ok: strm
            .refresh_code
            .is_none_or(|code| (200..300).contains(&code)),
        mode: "strm_generate".to_string(),
        requested,
        triggered: items.len(),
        global_refresh: items.iter().any(|item| item.id.is_none()),
        items,
        strm: Some(strm),
    })
}

#[derive(Debug)]
enum ScanPlan {
    Global { reason: String },
    Items(Vec<ScanTarget>),
}

#[derive(Debug)]
struct ScanTarget {
    id: String,
    name: String,
}

async fn resolve_scan_plan(client: &EmbyClient, req: &ScanLibraryRequest) -> AppResult<ScanPlan> {
    if let Some(item_id) = req.item_id.as_deref().and_then(non_empty_trimmed) {
        return Ok(ScanPlan::Items(vec![ScanTarget {
            id: item_id.to_string(),
            name: req
                .lib
                .as_deref()
                .and_then(non_empty_trimmed)
                .unwrap_or(item_id)
                .to_string(),
        }]));
    }

    let libraries = client.libraries().await?;
    if let Some(lib) = req.lib.as_deref().and_then(non_empty_trimmed) {
        let requested = lib.to_ascii_lowercase();
        let library = libraries
            .into_iter()
            .find(|item| {
                item.name.eq_ignore_ascii_case(lib)
                    || item.id.as_deref().is_some_and(|id| id == lib)
                    || library_folder_name(item).to_ascii_lowercase() == requested
            })
            .ok_or_else(|| AppError::NotFound(format!("Emby library not found: {lib}")))?;
        return library
            .id
            .filter(|id| !id.trim().is_empty())
            .map(|id| {
                ScanPlan::Items(vec![ScanTarget {
                    id,
                    name: library.name,
                }])
            })
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "Emby library {lib} has no ItemId; cannot target-refresh it"
                ))
            });
    }

    let targets = libraries
        .into_iter()
        .filter_map(|library| {
            let id = library.id?;
            (!id.trim().is_empty()).then_some(ScanTarget {
                id,
                name: library.name,
            })
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        Ok(ScanPlan::Global {
            reason: "Emby library list did not include ItemId".to_string(),
        })
    } else {
        Ok(ScanPlan::Items(targets))
    }
}

fn scan_task_label(req: &ScanLibraryRequest) -> String {
    if req.generate_strm.unwrap_or(false)
        && let Some(lib) = req.lib.as_deref().and_then(non_empty_trimmed)
    {
        format!("生成 STRM: {lib}")
    } else if let Some(lib) = req.lib.as_deref().and_then(non_empty_trimmed) {
        format!("扫描库: {lib}")
    } else if let Some(item_id) = req.item_id.as_deref().and_then(non_empty_trimmed) {
        format!("刷新 Item: {item_id}")
    } else {
        "扫描全部 Emby 库".to_string()
    }
}

fn first_non_empty(values: [Option<String>; 2]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find_map(|value| non_empty_trimmed(&value).map(ToString::to_string))
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

pub fn generate_missing_strm_for_library(
    state: &AppState,
    lib: &str,
    keyword: Option<&str>,
    fullauto: bool,
) -> AppResult<StrmGenerateResult> {
    let lib = required_segment(lib, "lib")?;
    let src_base = safe_under(&state.settings.cd_root, lib)?;
    let strm_base = safe_under(&state.settings.strm_root, lib)?;
    if !src_base.is_dir() {
        return Err(AppError::NotFound(format!(
            "115 文件夹不存在: {}",
            src_base.display()
        )));
    }
    std::fs::create_dir_all(&strm_base).map_err(|err| AppError::Anyhow(err.into()))?;
    chmod_public_dir(&strm_base);

    let keyword = keyword.unwrap_or_default().trim().to_string();
    let mut tops = std::fs::read_dir(&src_base)
        .map_err(|err| AppError::Anyhow(err.into()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .collect::<Vec<_>>();
    tops.sort_by_key(|entry| entry.file_name());

    let mut matched = 0usize;
    let mut new_count = 0usize;
    let mut new_folders = BTreeMap::new();
    let mut attention = Vec::new();
    for entry in tops {
        let top = entry.file_name().to_string_lossy().to_string();
        if !keyword.is_empty() && !top.contains(&keyword) {
            continue;
        }
        matched += 1;
        if is_duplicate_suffix_shadow(&top, &src_base, &strm_base) {
            attention.push(format!("{top} (检测到同名原目录，跳过重复副本 STRM 生成)"));
            continue;
        }
        let missing = missing_strm_in_top(&src_base, &strm_base, &entry.path());
        if missing.is_empty() {
            continue;
        }
        if should_generate_missing_top(&top, &strm_base, fullauto) {
            let mut written = 0usize;
            for (rel, filename) in missing {
                if write_strm(&strm_base, lib, &rel, &filename)? {
                    written += 1;
                }
            }
            if written > 0 {
                new_count += written;
                new_folders.insert(top, written);
            }
        } else {
            attention.push(format!(
                "{top} (+{}个视频,无tmdbid且首次出现,需看一眼)",
                missing.len()
            ));
        }
    }

    Ok(StrmGenerateResult {
        lib: lib.to_string(),
        keyword,
        matched,
        new_count,
        new_folders,
        attention,
        orphans_cleaned: 0,
        orphan_cleanup_skipped: false,
        permissions_fixed: 0,
        refreshed: false,
        refresh_code: None,
    })
}

fn cleanup_orphan_strm_for_library(
    state: &AppState,
    lib: &str,
    keyword: Option<&str>,
    result: &mut StrmGenerateResult,
) -> AppResult<usize> {
    let lib = required_segment(lib, "lib")?;
    let strm_base = safe_under(&state.settings.strm_root, lib)?;
    if !strm_base.is_dir() {
        return Ok(0);
    }
    if !mount_alive(&state.settings.cd_root, std::time::Duration::from_secs(5)) {
        result.orphan_cleanup_skipped = true;
        result
            .attention
            .push("跳过清孤儿: 媒体根探测失败，防止误删整库 STRM".to_string());
        return Ok(0);
    }
    let keyword = keyword.unwrap_or_default().trim();
    let mut cleaned = 0usize;
    for entry in WalkDir::new(&strm_base)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() || !has_extension(entry.path(), "strm") {
            continue;
        }
        if !keyword.is_empty() && !strm_path_matches_keyword(&strm_base, entry.path(), keyword) {
            continue;
        }
        let content = match std::fs::read_to_string(entry.path()) {
            Ok(content) => content.trim().to_string(),
            Err(_) => continue,
        };
        let Some(rel_media) = content.strip_prefix("/media/") else {
            continue;
        };
        let target = match safe_under(&state.settings.cd_root, rel_media) {
            Ok(target) => target,
            Err(_) => continue,
        };
        if target.exists() {
            continue;
        }
        if cleaned >= 30
            && (cleaned - 30).is_multiple_of(25)
            && !mount_alive(&state.settings.cd_root, std::time::Duration::from_secs(5))
        {
            result.orphan_cleanup_skipped = true;
            result.attention.push(format!(
                "清孤儿中止: 已删 {cleaned} 个且媒体根复探失败，防止误删整库 STRM"
            ));
            return Ok(cleaned);
        }
        std::fs::remove_file(entry.path()).map_err(|err| AppError::Anyhow(err.into()))?;
        cleaned += 1;
    }
    Ok(cleaned)
}

fn strm_path_matches_keyword(strm_base: &Path, path: &Path, keyword: &str) -> bool {
    path.strip_prefix(strm_base)
        .ok()
        .and_then(|rel| rel.components().next())
        .map(|component| component.as_os_str().to_string_lossy().contains(keyword))
        .unwrap_or(false)
}

pub fn mount_alive(root: &Path, timeout: std::time::Duration) -> bool {
    let root = root.to_path_buf();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let ok = root
            .is_dir()
            .then(|| std::fs::read_dir(root).map(|mut entries| entries.next().is_some()))
            .transpose()
            .ok()
            .flatten()
            .unwrap_or(false);
        let _ = tx.send(ok);
    });
    rx.recv_timeout(timeout).unwrap_or(false)
}

fn missing_strm_in_top(
    src_base: &Path,
    strm_base: &Path,
    top_path: &Path,
) -> Vec<(PathBuf, String)> {
    let mut missing = Vec::new();
    for entry in WalkDir::new(top_path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() || !is_video_file(entry.path()) {
            continue;
        }
        let Ok(root_rel) = entry
            .path()
            .parent()
            .unwrap_or(top_path)
            .strip_prefix(src_base)
        else {
            continue;
        };
        let Some(stem) = entry.path().file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let strm_path = strm_base.join(root_rel).join(format!("{stem}.strm"));
        if !strm_path.exists() {
            let Some(filename) = entry
                .path()
                .file_name()
                .and_then(|value| value.to_str())
                .map(ToString::to_string)
            else {
                continue;
            };
            missing.push((root_rel.to_path_buf(), filename));
        }
    }
    missing.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    missing
}

fn should_generate_missing_top(top: &str, strm_base: &Path, fullauto: bool) -> bool {
    has_declared_tmdb(top) || strm_base.join(top).is_dir() || fullauto
}

fn is_duplicate_suffix_shadow(top: &str, src_base: &Path, strm_base: &Path) -> bool {
    let Some(base) = duplicate_suffix_base(top) else {
        return false;
    };
    src_base.join(base).is_dir() || strm_base.join(base).is_dir()
}

fn duplicate_suffix_base(folder: &str) -> Option<&str> {
    let close = folder.strip_suffix(')')?;
    let open = close.rfind('(')?;
    let suffix = &close[open + 1..];
    if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(close[..open].trim_end())
}

fn has_declared_tmdb(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower
        .match_indices("tmdbid")
        .any(|(idx, _)| lower[idx + 6..].chars().any(|ch| ch.is_ascii_digit()))
}

fn write_strm(strm_base: &Path, lib: &str, rel: &Path, filename: &str) -> AppResult<bool> {
    let target_dir = strm_base.join(rel);
    std::fs::create_dir_all(&target_dir).map_err(|err| AppError::Anyhow(err.into()))?;
    chmod_public_tree(strm_base, rel);
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::BadRequest(format!("文件名缺少 stem: {filename}")))?;
    let strm_path = target_dir.join(format!("{stem}.strm"));
    if strm_path.exists() {
        chmod_public_file(&strm_path);
        return Ok(false);
    }
    let rel_text = rel.to_string_lossy().replace('\\', "/");
    let media_path = format!("/media/{lib}/{rel_text}/{filename}");
    std::fs::write(&strm_path, media_path).map_err(|err| AppError::Anyhow(err.into()))?;
    chmod_public_file(&strm_path);
    Ok(true)
}

fn is_video_file(path: &Path) -> bool {
    lower_extension(path)
        .as_deref()
        .is_some_and(|ext| VIDEO_EXTENSIONS.contains(&ext))
}

fn chmod_public_tree(base: &Path, rel: &Path) {
    chmod_public_dir(base);
    let mut current = base.to_path_buf();
    for part in rel.components() {
        current.push(part);
        chmod_public_dir(&current);
    }
}

#[cfg(unix)]
fn chmod_public_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.mode() & 0o777 != 0o755 {
            permissions.set_mode(0o755);
            let _ = std::fs::set_permissions(path, permissions);
        }
    }
}

#[cfg(not(unix))]
fn chmod_public_dir(_path: &Path) {}

#[cfg(unix)]
fn chmod_public_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.mode() & 0o777 != 0o644 {
            permissions.set_mode(0o644);
            let _ = std::fs::set_permissions(path, permissions);
        }
    }
}

#[cfg(not(unix))]
fn chmod_public_file(_path: &Path) {}

pub fn safe_under(base: impl AsRef<Path>, name: impl AsRef<Path>) -> AppResult<PathBuf> {
    let base = base.as_ref();
    let name = name.as_ref();
    let raw = name.as_os_str().to_string_lossy();
    let normalized = raw.replace('\\', "/");
    if raw.is_empty()
        || raw == "."
        || raw == ".."
        || raw.contains('\0')
        || normalized.starts_with('/')
        || normalized.split('/').any(|seg| seg == "..")
        || name.is_absolute()
        || name
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AppError::BadRequest(
            "非法路径段(含 .. 或绝对路径)".to_string(),
        ));
    }
    let joined = base.join(name);
    let canon_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let mut probe = joined.as_path();
    while !probe.exists() {
        let Some(parent) = probe.parent() else {
            break;
        };
        if parent == probe {
            break;
        }
        probe = parent;
    }
    let probe = probe.canonicalize().unwrap_or_else(|_| canon_base.clone());
    if !probe.starts_with(&canon_base) {
        return Err(AppError::BadRequest("路径越界".to_string()));
    }
    Ok(joined)
}

fn maybe_strm_overview(base: &Path, query: &StrmListQuery) -> Option<StrmOverview> {
    query.overview.unwrap_or(false).then(|| {
        let depth = query
            .overview_depth
            .unwrap_or(DEFAULT_STRM_OVERVIEW_DEPTH)
            .clamp(1, MAX_STRM_OVERVIEW_DEPTH);
        let sample_limit = query
            .sample_limit
            .unwrap_or(DEFAULT_STRM_SAMPLE_LIMIT)
            .clamp(0, MAX_STRM_SAMPLE_LIMIT);
        build_strm_overview(base, depth, sample_limit)
    })
}

fn build_strm_overview(base: &Path, max_depth: usize, sample_limit: usize) -> StrmOverview {
    let mut overview = StrmOverview {
        base: base.display().to_string(),
        max_depth,
        entry_limit: STRM_OVERVIEW_ENTRY_LIMIT,
        directories: 0,
        files: 0,
        strm_files: 0,
        subtitle_files: 0,
        other_files: 0,
        strm_bytes: 0,
        subtitle_bytes: 0,
        strm_with_subtitles: 0,
        strm_without_subtitles: 0,
        subtitle_coverage_percent: 0.0,
        subtitle_extensions: Vec::new(),
        subtitle_languages: Vec::new(),
        library_coverage: Vec::new(),
        missing_subtitle_samples: Vec::new(),
        samples: Vec::new(),
        truncated: false,
        warnings: Vec::new(),
    };
    if !base.exists() {
        overview
            .warnings
            .push(format!("strm 概览目录不存在: {}", base.display()));
        return overview;
    }
    if !base.is_dir() {
        overview
            .warnings
            .push(format!("strm 概览目标不是目录: {}", base.display()));
        return overview;
    }

    let mut subtitle_counts = BTreeMap::<String, usize>::new();
    let mut subtitle_language_counts = BTreeMap::<String, usize>::new();
    let mut subtitle_keys = BTreeSet::<String>::new();
    let mut strm_entries = Vec::<StrmCoverageEntry>::new();
    let mut read_errors = 0usize;
    let mut seen_entries = 0usize;
    let single_library = base.file_name().and_then(|name| name.to_str());

    for entry in WalkDir::new(base)
        .min_depth(1)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
    {
        if seen_entries >= STRM_OVERVIEW_ENTRY_LIMIT {
            overview.truncated = true;
            overview.warnings.push(format!(
                "strm 概览超过 {} 个条目，结果已截断",
                STRM_OVERVIEW_ENTRY_LIMIT
            ));
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                read_errors += 1;
                continue;
            }
        };
        seen_entries += 1;
        let path = entry.path();
        let file_type = entry.file_type();
        if file_type.is_dir() {
            overview.directories += 1;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        overview.files += 1;
        let extension = lower_extension(path);
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let Ok(rel) = path.strip_prefix(base) else {
            continue;
        };
        let rel_path = rel.to_string_lossy().replace('\\', "/");

        if extension.as_deref() == Some("strm") {
            overview.strm_files += 1;
            overview.strm_bytes = overview.strm_bytes.saturating_add(size);
            strm_entries.push(StrmCoverageEntry {
                key: rel_key_without_extension(rel),
                library: library_for_rel(rel, single_library),
                rel_path: rel_path.clone(),
            });
            push_sample(
                &mut overview.samples,
                sample_limit,
                rel_path,
                "strm",
                "strm",
                size,
            );
        } else if extension
            .as_deref()
            .is_some_and(|ext| SUBTITLE_EXTENSIONS.contains(&ext))
        {
            let extension = extension.unwrap_or_default();
            overview.subtitle_files += 1;
            overview.subtitle_bytes = overview.subtitle_bytes.saturating_add(size);
            *subtitle_counts.entry(extension.clone()).or_default() += 1;
            subtitle_keys.insert(subtitle_video_key(rel));
            *subtitle_language_counts
                .entry(infer_subtitle_language(rel))
                .or_default() += 1;
            push_sample(
                &mut overview.samples,
                sample_limit,
                rel_path,
                "subtitle",
                &extension,
                size,
            );
        } else {
            overview.other_files += 1;
        }
    }

    if read_errors > 0 {
        overview
            .warnings
            .push(format!("strm 概览有 {read_errors} 个条目无法读取，已跳过"));
    }
    overview.subtitle_extensions = subtitle_counts
        .into_iter()
        .map(|(extension, count)| ExtensionCount { extension, count })
        .collect();
    overview.subtitle_languages = subtitle_language_counts
        .into_iter()
        .map(|(language, count)| SubtitleLanguageCount { language, count })
        .collect();
    finish_subtitle_coverage(&mut overview, strm_entries, &subtitle_keys, sample_limit);
    overview
}

struct StrmCoverageEntry {
    key: String,
    library: String,
    rel_path: String,
}

#[derive(Default)]
struct LibraryCoverageAccumulator {
    strm_files: usize,
    with_subtitles: usize,
}

fn finish_subtitle_coverage(
    overview: &mut StrmOverview,
    strm_entries: Vec<StrmCoverageEntry>,
    subtitle_keys: &BTreeSet<String>,
    sample_limit: usize,
) {
    let mut libraries = BTreeMap::<String, LibraryCoverageAccumulator>::new();
    for entry in strm_entries {
        let has_subtitle = subtitle_keys.contains(&entry.key);
        if has_subtitle {
            overview.strm_with_subtitles += 1;
        } else {
            overview.strm_without_subtitles += 1;
            if overview.missing_subtitle_samples.len() < sample_limit {
                overview
                    .missing_subtitle_samples
                    .push(entry.rel_path.clone());
            }
        }
        let library = libraries.entry(entry.library).or_default();
        library.strm_files += 1;
        if has_subtitle {
            library.with_subtitles += 1;
        }
    }
    overview.subtitle_coverage_percent =
        coverage_percent(overview.strm_with_subtitles, overview.strm_files);
    overview.library_coverage = libraries
        .into_iter()
        .map(|(library, stats)| {
            let missing_subtitles = stats.strm_files.saturating_sub(stats.with_subtitles);
            SubtitleLibraryCoverage {
                library,
                strm_files: stats.strm_files,
                with_subtitles: stats.with_subtitles,
                missing_subtitles,
                coverage_percent: coverage_percent(stats.with_subtitles, stats.strm_files),
            }
        })
        .collect();
}

fn coverage_percent(with_subtitles: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    ((with_subtitles as f64 / total as f64) * 1000.0).round() / 10.0
}

fn rel_key_without_extension(rel: &Path) -> String {
    rel.with_extension("").to_string_lossy().replace('\\', "/")
}

fn subtitle_video_key(rel: &Path) -> String {
    let mut without_extension = rel.with_extension("");
    let Some(stem) = without_extension.file_name().and_then(|name| name.to_str()) else {
        return rel_key_without_extension(rel);
    };
    let mut parts: Vec<&str> = stem.split('.').collect();
    while parts.len() > 1
        && parts
            .last()
            .is_some_and(|part| subtitle_tag_language(part).is_some() || is_subtitle_modifier(part))
    {
        parts.pop();
    }
    without_extension.set_file_name(parts.join("."));
    without_extension.to_string_lossy().replace('\\', "/")
}

fn library_for_rel(rel: &Path, single_library: Option<&str>) -> String {
    if let Some(library) = single_library.filter(|value| !value.is_empty()) {
        return library.to_string();
    }
    rel.components()
        .next()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "(root)".to_string())
}

fn infer_subtitle_language(rel: &Path) -> String {
    let Some(stem) = rel.file_stem().and_then(|name| name.to_str()) else {
        return "unknown".to_string();
    };
    for part in stem.split('.').rev() {
        if let Some(language) = subtitle_tag_language(part) {
            return language.to_string();
        }
    }
    "unknown".to_string()
}

fn subtitle_tag_language(part: &str) -> Option<&'static str> {
    let normalized = part.trim().to_ascii_lowercase().replace(['_', '-'], "");
    match normalized.as_str() {
        "zh" | "zho" | "chi" | "chs" | "cht" | "sc" | "tc" | "cn" | "zhcn" | "zhtw" | "big5"
        | "gb" => Some("zh"),
        "en" | "eng" | "english" => Some("en"),
        "ja" | "jp" | "jpn" | "japanese" => Some("ja"),
        "ko" | "kor" | "kr" | "korean" => Some("ko"),
        "fr" | "fre" | "fra" | "french" => Some("fr"),
        "de" | "ger" | "deu" | "german" => Some("de"),
        "es" | "spa" | "spanish" => Some("es"),
        _ => None,
    }
}

fn is_subtitle_modifier(part: &str) -> bool {
    matches!(
        part.trim().to_ascii_lowercase().as_str(),
        "default" | "forced" | "sdh" | "hi" | "cc"
    )
}

fn push_sample(
    samples: &mut Vec<StrmSample>,
    sample_limit: usize,
    rel_path: String,
    kind: &str,
    extension: &str,
    size: u64,
) {
    if samples.len() < sample_limit {
        samples.push(StrmSample {
            rel_path,
            kind: kind.to_string(),
            extension: extension.to_string(),
            size,
        });
    }
}

fn lower_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|v| v.to_str())
        .map(str::to_ascii_lowercase)
}

fn has_extension(path: &Path, wanted: &str) -> bool {
    lower_extension(path)
        .as_deref()
        .is_some_and(|ext| ext == wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(safe_under(tmp.path(), "电影/A").is_ok());
        assert!(safe_under(tmp.path(), "").is_err());
        assert!(safe_under(tmp.path(), ".").is_err());
        assert!(safe_under(tmp.path(), "../etc").is_err());
        assert!(safe_under(tmp.path(), "sub/../../etc").is_err());
        assert!(safe_under(tmp.path(), "..\\etc").is_err());
        assert!(safe_under(tmp.path(), "/etc/passwd").is_err());
        assert!(safe_under(tmp.path(), "evil\0.txt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_missing_child_below_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("link")).unwrap();

        assert!(safe_under(tmp.path(), "link/new/file.strm").is_err());
    }

    #[test]
    fn missing_api_key_error_is_clear() {
        let err = ensure_api_key_configured(" \t").unwrap_err();
        assert!(err.to_string().contains("api_key is not configured"));
        assert!(ensure_api_key_configured("secret").is_ok());
    }

    #[tokio::test]
    async fn lists_strm_files_without_non_strm_noise() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("电影");
        std::fs::create_dir_all(lib.join("A")).unwrap();
        std::fs::write(lib.join("A").join("a.strm"), "http://example").unwrap();
        std::fs::write(lib.join("A").join("a.nfo"), "noise").unwrap();

        let settings = crate::settings::Settings {
            host: "127.0.0.1".to_string(),
            port: 8098,
            database_url: "postgres://unused".to_string(),
            web_dist: tmp.path().to_path_buf(),
            legacy_dir: tmp.path().to_path_buf(),
            bootstrap_password: "admin".to_string(),
            cd_root: tmp.path().to_path_buf(),
            strm_root: tmp.path().to_path_buf(),
            docker_bin: tmp.path().join("docker"),
            task_concurrency: 1,
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused")
            .unwrap();
        let state = AppState::new(pool, settings);
        let res = list_strm(
            State(state),
            Query(StrmListQuery {
                lib: Some("电影".to_string()),
                folder: None,
                limit: Some(10),
                overview: None,
                overview_depth: None,
                sample_limit: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(res.items.iter().any(|i| i.rel_path == "A"));
        assert!(res.items.iter().any(|i| i.rel_path == "A/a.strm"));
        assert!(!res.items.iter().any(|i| i.rel_path.ends_with(".nfo")));
    }

    #[tokio::test]
    async fn generates_missing_strm_without_overwriting_or_unknown_first_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let cd_root = tmp.path().join("cd");
        let strm_root = tmp.path().join("strm");
        std::fs::create_dir_all(cd_root.join("电影/Movie [tmdbid-123]/Season 1")).unwrap();
        std::fs::create_dir_all(cd_root.join("电影/Movie [tmdbid-123](1)/Season 1")).unwrap();
        std::fs::create_dir_all(cd_root.join("电影/No Match")).unwrap();
        std::fs::create_dir_all(strm_root.join("电影/Movie [tmdbid-123]/Season 1")).unwrap();
        std::fs::write(
            cd_root.join("电影/Movie [tmdbid-123]/Season 1/E01.mkv"),
            "video",
        )
        .unwrap();
        std::fs::write(cd_root.join("电影/No Match/Loose.mp4"), "video").unwrap();
        std::fs::write(
            strm_root.join("电影/Movie [tmdbid-123]/Season 1/E01.strm"),
            "keep-me",
        )
        .unwrap();
        std::fs::write(
            cd_root.join("电影/Movie [tmdbid-123]/Season 1/E02.mp4"),
            "video",
        )
        .unwrap();
        std::fs::write(
            cd_root.join("电影/Movie [tmdbid-123](1)/Season 1/E03.mp4"),
            "video",
        )
        .unwrap();

        let state = AppState::new(
            sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://unused")
                .unwrap(),
            crate::settings::Settings {
                host: "127.0.0.1".to_string(),
                port: 8098,
                database_url: "postgres://unused".to_string(),
                web_dist: tmp.path().to_path_buf(),
                legacy_dir: tmp.path().to_path_buf(),
                bootstrap_password: "admin".to_string(),
                cd_root: cd_root.clone(),
                strm_root: strm_root.clone(),
                docker_bin: tmp.path().join("docker"),
                task_concurrency: 1,
            },
        );

        let result = generate_missing_strm_for_library(&state, "电影", None, false).unwrap();
        assert_eq!(result.matched, 3);
        assert_eq!(result.new_count, 1);
        assert_eq!(result.new_folders["Movie [tmdbid-123]"], 1);
        assert_eq!(result.attention.len(), 2);
        assert!(
            result
                .attention
                .iter()
                .any(|item| item.contains("No Match"))
        );
        assert!(
            result
                .attention
                .iter()
                .any(|item| item.contains("Movie [tmdbid-123](1)"))
        );
        assert_eq!(
            std::fs::read_to_string(strm_root.join("电影/Movie [tmdbid-123]/Season 1/E01.strm"))
                .unwrap(),
            "keep-me"
        );
        assert_eq!(
            std::fs::read_to_string(strm_root.join("电影/Movie [tmdbid-123]/Season 1/E02.strm"))
                .unwrap(),
            "/media/电影/Movie [tmdbid-123]/Season 1/E02.mp4"
        );
        assert!(
            !strm_root
                .join("电影/Movie [tmdbid-123](1)/Season 1/E03.strm")
                .exists()
        );
        assert!(!strm_root.join("电影/No Match/Loose.strm").exists());

        let mut result =
            generate_missing_strm_for_library(&state, "电影", Some("No"), true).unwrap();
        assert_eq!(result.matched, 1);
        assert_eq!(result.new_count, 1);
        assert_eq!(
            std::fs::read_to_string(strm_root.join("电影/No Match/Loose.strm")).unwrap(),
            "/media/电影/No Match/Loose.mp4"
        );

        std::fs::create_dir_all(strm_root.join("电影/Orphan")).unwrap();
        std::fs::write(
            strm_root.join("电影/Orphan/Gone.strm"),
            "/media/电影/Orphan/Gone.mkv",
        )
        .unwrap();
        std::fs::write(
            strm_root.join("电影/Orphan/External.strm"),
            "http://example",
        )
        .unwrap();
        let cleaned = cleanup_orphan_strm_for_library(&state, "电影", None, &mut result).unwrap();
        assert_eq!(cleaned, 1);
        assert!(!strm_root.join("电影/Orphan/Gone.strm").exists());
        assert!(strm_root.join("电影/Orphan/External.strm").exists());
    }

    #[tokio::test]
    async fn orphan_cleanup_skips_when_media_mount_probe_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let cd_root = tmp.path().join("empty-cd");
        let strm_root = tmp.path().join("strm");
        std::fs::create_dir_all(&cd_root).unwrap();
        std::fs::create_dir_all(strm_root.join("电影/Orphan")).unwrap();
        std::fs::write(
            strm_root.join("电影/Orphan/Gone.strm"),
            "/media/电影/Orphan/Gone.mkv",
        )
        .unwrap();

        let state = AppState::new(
            sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://unused")
                .unwrap(),
            crate::settings::Settings {
                host: "127.0.0.1".to_string(),
                port: 8098,
                database_url: "postgres://unused".to_string(),
                web_dist: tmp.path().to_path_buf(),
                legacy_dir: tmp.path().to_path_buf(),
                bootstrap_password: "admin".to_string(),
                cd_root,
                strm_root: strm_root.clone(),
                docker_bin: tmp.path().join("docker"),
                task_concurrency: 1,
            },
        );
        let mut result = StrmGenerateResult {
            lib: "电影".to_string(),
            keyword: String::new(),
            matched: 0,
            new_count: 0,
            new_folders: BTreeMap::new(),
            attention: Vec::new(),
            orphans_cleaned: 0,
            orphan_cleanup_skipped: false,
            permissions_fixed: 0,
            refreshed: false,
            refresh_code: None,
        };

        let cleaned = cleanup_orphan_strm_for_library(&state, "电影", None, &mut result).unwrap();
        assert_eq!(cleaned, 0);
        assert!(result.orphan_cleanup_skipped);
        assert!(strm_root.join("电影/Orphan/Gone.strm").exists());
    }
}
