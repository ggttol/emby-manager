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
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ScanLibraryResult {
    pub ok: bool,
    pub mode: String,
    pub requested: Option<String>,
    pub triggered: usize,
    pub global_refresh: bool,
    pub items: Vec<ScanLibraryItemResult>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ScanLibraryItemResult {
    pub id: Option<String>,
    pub name: String,
    pub code: u16,
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/libraries", get(libraries))
        .route("/api/v2/libraries/scan", post(scan_library))
        .route("/api/v2/libraries/strm", get(list_strm))
        .route("/api/v2/manage/delete", post(manage_delete))
        .route("/api/v2/manage/delete/execute", post(execute_delete))
        .route("/api/v2/manage/move", post(preview_move))
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
            })
        }
    }
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
    if let Some(lib) = req.lib.as_deref().and_then(non_empty_trimmed) {
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
}
