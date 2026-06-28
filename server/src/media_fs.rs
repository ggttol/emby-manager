use crate::{
    config_store,
    emby::{EmbyClient, EmbyLibrary},
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
use std::{
    collections::BTreeMap,
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
const TASK_CANCELLED_SENTINEL: &str = "__task_cancelled__";

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LibrariesResponse {
    pub libraries: Vec<EmbyLibrary>,
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
    pub subtitle_extensions: Vec<ExtensionCount>,
    pub samples: Vec<StrmSample>,
    pub truncated: bool,
    pub warnings: Vec<String>,
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

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
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
    pub from_lib: String,
    pub from_folder: String,
    pub to_lib: String,
    pub to_folder: Option<String>,
    pub item_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ManageDeleteBatchRequest {
    pub items: Vec<ManageDeleteRequest>,
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/libraries", get(libraries))
        .route("/api/v2/libraries/scan", post(scan_library))
        .route("/api/v2/libraries/strm", get(list_strm))
        .route("/api/v2/manage/delete", post(manage_delete))
        .route("/api/v2/manage/delete/execute", post(execute_delete))
        .route(
            "/api/v2/manage/delete/batch/execute",
            post(execute_delete_batch),
        )
        .route("/api/v2/manage/move", post(manage_move))
        .route("/api/v2/manage/move/execute", post(execute_move))
}

#[utoipa::path(get, path = "/api/v2/libraries", tag = "media", responses((status = 200, body = LibrariesResponse)))]
pub async fn libraries(State(state): State<AppState>) -> AppResult<Json<LibrariesResponse>> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;

    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let libraries = client.libraries().await?;
    Ok(Json(LibrariesResponse { libraries }))
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
        safe_under(&state.settings.strm_root, lib)?
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

#[utoipa::path(post, path = "/api/v2/manage/delete", tag = "media", request_body = ManageDeleteRequest, responses((status = 200, body = TaskRun)))]
pub async fn preview_delete(
    State(state): State<AppState>,
    Json(req): Json<ManageDeleteRequest>,
) -> AppResult<Json<TaskRun>> {
    let plan = plan_delete(&state, &req)?;
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
    let plan = plan_delete_execute(&state, &req)?;
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
    let items = req
        .items
        .into_iter()
        .take(500)
        .map(|mut item| {
            if item.reason.as_deref().and_then(non_empty_trimmed).is_none() {
                item.reason.clone_from(&req.reason);
            }
            plan_delete_execute(&state, &item).map(|plan| (item, plan))
        })
        .collect::<AppResult<Vec<_>>>()?;
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
    let plan = plan_move(&state, &req)?;
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
    let plan = plan_move_execute(&state, &req)?;
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

fn ensure_api_key_configured(api_key: &str) -> AppResult<()> {
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; set it via /api/v2/config before listing Emby libraries"
                .to_string(),
        ));
    }
    Ok(())
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
            ok: false,
            preview: true,
            dry_run: true,
            operation: operation.to_string(),
            planned_paths: planned_paths
                .into_iter()
                .map(|path| path.display().to_string())
                .collect(),
            warnings: vec![
                "Rust preview did not touch filesystem or Emby".to_string(),
                "真实删除/移动尚未在 Rust manage API 中启用".to_string(),
            ],
            next_steps: vec![
                "核对 planned_paths 是否符合预期".to_string(),
                "等待后续写入版实现接入 undo log 与 Emby 刷新".to_string(),
            ],
            message: "Rust preview did not touch filesystem or Emby".to_string(),
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
            Err(err) if err.to_string() == TASK_CANCELLED_SENTINEL => {}
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
        return Err(AppError::Conflict(TASK_CANCELLED_SENTINEL.to_string()));
    }

    let mut emby_gone = true;
    if let Some(item_id) = req.item_id.as_deref().and_then(non_empty_trimmed) {
        client.delete_item(item_id).await?;
        emby_gone = verify_emby_delete(client, item_id).await;
    }
    tasks::set_progress(&state.pool, id, 1, "Emby Item 删除请求已完成").await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(TASK_CANCELLED_SENTINEL.to_string()));
    }

    let deleted_from = delete_planned_paths(&plan).await?;
    tasks::set_progress(&state.pool, id, 2, "磁盘删除已完成").await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(TASK_CANCELLED_SENTINEL.to_string()));
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
    let mut emby_gone = true;
    if let Some(item_id) = req.item_id.as_deref().and_then(non_empty_trimmed) {
        client.delete_item(item_id).await?;
        emby_gone = verify_emby_delete(client, item_id).await;
    }
    let deleted_from = delete_planned_paths(&plan).await?;
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
            Err(err) if err.to_string() == TASK_CANCELLED_SENTINEL => {}
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, id, &err.to_string(), None).await;
            }
        }
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
        return Err(AppError::Conflict(TASK_CANCELLED_SENTINEL.to_string()));
    }

    let target_library_id = resolve_library_item_id(client, &plan.to_lib).await?;
    tasks::set_progress(&state.pool, id, 1, "目标库已确认").await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(TASK_CANCELLED_SENTINEL.to_string()));
    }

    move_cd_folder(&plan).await?;
    tasks::set_progress(&state.pool, id, 2, "媒体目录已移动").await?;

    let old_strm_removed = remove_path_if_exists(&plan.from_strm_target).await?;
    let strm_written = rebuild_strm_for_moved_folder(&plan)?;
    tasks::set_progress(&state.pool, id, 3, "STRM 已重建").await?;

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
    tasks::set_progress(&state.pool, id, 4, "旧路径通知已处理").await?;

    let refresh_code = client
        .refresh_item(&target_library_id, true, false)
        .await
        .ok();
    let undo_id = Uuid::new_v4();
    let to_folder = plan.to_folder.clone();
    let payload = serde_json::json!({
        "from": req.from_lib.trim(),
        "to": req.to_lib.trim(),
        "folder": req.from_folder.trim(),
        "to_folder": to_folder,
        "emby_id": req.item_id.as_deref().and_then(non_empty_trimmed),
        "strm_count": strm_written,
    });
    sqlx::query("INSERT INTO undo_entries(id, op, payload) VALUES ($1, 'move', $2)")
        .bind(undo_id)
        .bind(payload)
        .execute(&state.pool)
        .await?;
    tasks::set_progress(&state.pool, id, 5, "目标库刷新并写入 undo").await?;

    Ok(ManageMoveExecuteResult {
        ok: true,
        preview: false,
        dry_run: false,
        operation: "move".to_string(),
        from_lib: plan.from_lib,
        from_folder: req.from_folder.trim().to_string(),
        to_lib: plan.to_lib,
        to_folder,
        moved: true,
        old_strm_removed,
        strm_written,
        emby_gone,
        notified,
        refresh_code,
        undo_id,
    })
}

async fn delete_planned_paths(plan: &DeleteExecutePlan) -> AppResult<Vec<String>> {
    let mut deleted_from = Vec::new();
    for (path, label) in [(&plan.cd_target, "115"), (&plan.strm_target, "strm")] {
        if !path.exists() {
            continue;
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
        deleted_from.push(label.to_string());
    }
    Ok(deleted_from)
}

async fn resolve_library_item_id(client: &EmbyClient, name: &str) -> AppResult<String> {
    let library = client
        .libraries()
        .await?
        .into_iter()
        .find(|item| item.name == name || item.name.eq_ignore_ascii_case(name))
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

async fn remove_path_if_exists(path: &Path) -> AppResult<bool> {
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
        if write_strm(&plan.to_strm_lib, &plan.to_lib, root_rel, filename)? {
            written += 1;
        }
    }
    Ok(written)
}

fn plan_delete(state: &AppState, req: &ManageDeleteRequest) -> AppResult<Vec<PathBuf>> {
    let lib_root = safe_under(
        &state.settings.strm_root,
        required_segment(&req.lib, "lib")?,
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
) -> AppResult<DeleteExecutePlan> {
    let lib = required_segment(&req.lib, "lib")?;
    let folder = required_segment(&req.folder, "folder")?;
    let cd_lib = safe_under(&state.settings.cd_root, lib)?;
    let strm_lib = safe_under(&state.settings.strm_root, lib)?;
    Ok(DeleteExecutePlan {
        cd_target: safe_under(&cd_lib, folder)?,
        strm_target: safe_under(&strm_lib, folder)?,
        emby_deleted_path: format!("/strm/{lib}/{folder}"),
    })
}

fn plan_move(state: &AppState, req: &ManageMoveRequest) -> AppResult<Vec<PathBuf>> {
    let from_lib = safe_under(
        &state.settings.strm_root,
        required_segment(&req.from_lib, "from_lib")?,
    )?;
    let from = safe_under(
        &from_lib,
        required_segment(&req.from_folder, "from_folder")?,
    )?;
    let to_lib = safe_under(
        &state.settings.strm_root,
        required_segment(&req.to_lib, "to_lib")?,
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
    to_folder: String,
    from_cd_target: PathBuf,
    to_cd_lib: PathBuf,
    to_cd_target: PathBuf,
    from_strm_target: PathBuf,
    to_strm_lib: PathBuf,
    from_emby_path: String,
}

fn plan_move_execute(state: &AppState, req: &ManageMoveRequest) -> AppResult<MoveExecutePlan> {
    let from_lib = required_segment(&req.from_lib, "from_lib")?;
    let from_folder = required_segment(&req.from_folder, "from_folder")?;
    let to_lib = required_segment(&req.to_lib, "to_lib")?;
    let to_folder = move_target_folder(req)?;

    let from_cd_lib = safe_under(&state.settings.cd_root, from_lib)?;
    let to_cd_lib = safe_under(&state.settings.cd_root, to_lib)?;
    let from_strm_lib = safe_under(&state.settings.strm_root, from_lib)?;
    let to_strm_lib = safe_under(&state.settings.strm_root, to_lib)?;

    let from_cd_target = safe_under(&from_cd_lib, from_folder)?;
    let to_cd_target = safe_under(&to_cd_lib, &to_folder)?;
    let from_strm_target = safe_under(&from_strm_lib, from_folder)?;
    let _to_strm_target = safe_under(&to_strm_lib, &to_folder)?;

    Ok(MoveExecutePlan {
        from_lib: from_lib.to_string(),
        to_lib: to_lib.to_string(),
        to_folder,
        from_cd_target,
        to_cd_lib,
        to_cd_target,
        from_strm_target,
        to_strm_lib,
        from_emby_path: format!("/strm/{from_lib}/{from_folder}"),
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
            Err(err) if err.to_string() == TASK_CANCELLED_SENTINEL => {}
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, id, &err.to_string(), None).await;
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
                return Err(AppError::Conflict(TASK_CANCELLED_SENTINEL.to_string()));
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
                    return Err(AppError::Conflict(TASK_CANCELLED_SENTINEL.to_string()));
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
    tasks::set_total(&state.pool, id, 2).await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(TASK_CANCELLED_SENTINEL.to_string()));
    }
    tasks::set_progress(&state.pool, id, 0, "生成缺失 STRM").await?;
    let mut strm = generate_missing_strm_for_library(
        state,
        &lib,
        req.keyword.as_deref().and_then(non_empty_trimmed),
        req.fullauto.unwrap_or(false),
    )?;
    if req.cleanup_orphans.unwrap_or(false) {
        let cleaned = cleanup_orphan_strm_for_library(
            state,
            &lib,
            req.keyword.as_deref().and_then(non_empty_trimmed),
            &mut strm,
        )?;
        strm.orphans_cleaned = cleaned;
    }
    tasks::set_progress(&state.pool, id, 1, "STRM 生成完成").await?;
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Err(AppError::Conflict(TASK_CANCELLED_SENTINEL.to_string()));
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
        let library = libraries
            .into_iter()
            .find(|item| item.name == lib || item.name.eq_ignore_ascii_case(lib))
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

fn mount_alive(root: &Path, timeout: std::time::Duration) -> bool {
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
        subtitle_extensions: Vec::new(),
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
    let mut read_errors = 0usize;
    let mut seen_entries = 0usize;

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
    overview
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
        assert_eq!(result.matched, 2);
        assert_eq!(result.new_count, 1);
        assert_eq!(result.new_folders["Movie [tmdbid-123]"], 1);
        assert_eq!(result.attention.len(), 1);
        assert!(result.attention[0].contains("No Match"));
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
