use crate::{
    c115::{self, C115_API, C115_SITE, C115Client},
    config_store,
    emby::{EmbyClient, EmbyLibrary},
    error::{AppError, AppResult},
    media_fs::{library_folder_name, safe_under},
    state::AppState,
    tasks::{self, TaskRun},
};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use walkdir::WalkDir;

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const MAX_AUTO_ALL_GROUPS: usize = 100;

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupRow {
    pub lib: String,
    pub folder: String,
    pub score: i64,
    pub n: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupGroup {
    pub tmdb: String,
    pub keep: DedupRow,
    pub remove: Vec<DedupRow>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupReviewGroup {
    pub tmdb: String,
    pub why: String,
    pub rows: Vec<DedupRow>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupAnalysisResponse {
    pub dups: Vec<DedupGroup>,
    pub review: Vec<DedupReviewGroup>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct DedupFolderRef {
    pub lib: String,
    pub folder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct DedupExecuteRequest {
    pub tmdb: Option<String>,
    pub remove: Vec<DedupFolderRef>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupDeleteResult {
    pub lib: String,
    pub folder: String,
    pub deleted_from: Vec<String>,
    pub emby_updates: Vec<EmbyUpdate>,
    pub notified: bool,
    pub undo_id: Uuid,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupExecuteResponse {
    pub ok: bool,
    pub tmdb: Option<String>,
    pub removed: Vec<DedupDeleteResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct DedupExecuteBatchGroup {
    pub tmdb: Option<String>,
    pub remove: Vec<DedupFolderRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct DedupExecuteBatchRequest {
    pub groups: Vec<DedupExecuteBatchGroup>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupExecuteBatchItemResult {
    pub tmdb: Option<String>,
    pub ok: bool,
    pub removed: usize,
    pub err: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupExecuteBatchResult {
    pub ok: bool,
    pub results: Vec<DedupExecuteBatchItemResult>,
    pub ok_count: usize,
    pub error_count: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ReplaceRequest {
    pub lib: String,
    pub win_folder: String,
    pub lose_folder: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct EmbyUpdate {
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "UpdateType")]
    pub update_type: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ReplaceExecuteResponse {
    pub ok: bool,
    pub lib: String,
    pub kept_as: String,
    pub dropped: String,
    pub renamed: bool,
    pub deleted_from: Vec<String>,
    pub emby_updates: Vec<EmbyUpdate>,
    pub notified: bool,
    pub undo_id: Uuid,
    pub msg: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ReplaceBatchRequest {
    pub items: Vec<ReplaceRequest>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ReplaceBatchItemResult {
    pub lib: String,
    pub win: String,
    pub lose: String,
    pub ok: bool,
    pub kept_as: Option<String>,
    pub msg: String,
    pub err: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ReplaceBatchResult {
    pub results: Vec<ReplaceBatchItemResult>,
    pub ok_count: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Deserialize, Default, utoipa::ToSchema)]
pub struct DedupAutoAllRequest {
    pub limit: Option<usize>,
    #[serde(rename = "async")]
    pub async_requested: Option<bool>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupAutoAllItemResult {
    pub tmdb: String,
    pub ok: bool,
    pub kept: String,
    pub removed: usize,
    pub err: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupAutoAllResponse {
    pub results: Vec<DedupAutoAllItemResult>,
    pub ok_count: usize,
    pub total: usize,
    pub total_removed_folders: usize,
    pub review_count: usize,
    pub async_requested: bool,
}

#[derive(Clone)]
struct C115DeleteContext {
    client: C115Client,
    cid_map: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ReplacePlan {
    pub lib: String,
    pub win_folder: String,
    pub lose_folder: String,
    pub kept_as: String,
    pub rename_win: bool,
    pub win_cd: PathBuf,
    pub lose_cd: PathBuf,
    pub target_cd: PathBuf,
    pub win_strm: PathBuf,
    pub lose_strm: PathBuf,
    pub target_strm: PathBuf,
    pub emby_updates: Vec<EmbyUpdate>,
}

#[derive(Debug)]
struct ScannedFolder {
    row: DedupRow,
    medias: Vec<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/dedup", post(execute_dedup))
        .route("/api/v2/dedup/execute", post(execute_dedup))
        .route("/api/v2/dedup/execute-batch", post(execute_dedup_batch))
        .route("/api/v2/dedup/exec-batch", post(execute_dedup_batch))
        .route("/api/v2/dedup/exec_batch", post(execute_dedup_batch))
        .route("/api/v2/dedup/duplicates", get(duplicates))
        .route("/api/v2/dedup/analyze", get(duplicates))
        .route("/api/v2/dedup/replace", post(replace_execute))
        .route("/api/v2/dedup/replace-batch", post(replace_batch))
        .route("/api/v2/dedup/replace_batch", post(replace_batch))
        .route("/api/v2/dedup/auto_all", post(auto_all))
        .route("/api/v2/dedup/auto-all", post(auto_all))
}

#[utoipa::path(get, path = "/api/v2/dedup/duplicates", tag = "dedup", responses((status = 200, body = DedupAnalysisResponse)))]
pub async fn duplicates(State(state): State<AppState>) -> AppResult<Json<DedupAnalysisResponse>> {
    Ok(Json(analyze_duplicate_groups_for_state(&state).await?))
}

#[utoipa::path(post, path = "/api/v2/dedup/execute", tag = "dedup", request_body = DedupExecuteRequest, responses((status = 200, body = DedupExecuteResponse)))]
pub async fn execute_dedup(
    State(state): State<AppState>,
    Json(req): Json<DedupExecuteRequest>,
) -> AppResult<Json<DedupExecuteResponse>> {
    if req.remove.is_empty() {
        return Err(AppError::BadRequest("remove must not be empty".to_string()));
    }

    let emby_client = dedup_emby_client(&state).await?;
    let c115_delete = dedup_c115_delete_context(&state).await?;
    let mut removed = Vec::with_capacity(req.remove.len());
    for item in &req.remove {
        removed
            .push(delete_duplicate_folder(&state, item, &emby_client, c115_delete.as_ref()).await?);
    }

    Ok(Json(DedupExecuteResponse {
        ok: true,
        tmdb: req.tmdb,
        removed,
    }))
}

#[utoipa::path(post, path = "/api/v2/dedup/execute-batch", tag = "dedup", request_body = DedupExecuteBatchRequest, responses((status = 200, body = TaskRun)))]
pub async fn execute_dedup_batch(
    State(state): State<AppState>,
    Json(req): Json<DedupExecuteBatchRequest>,
) -> AppResult<Json<TaskRun>> {
    Ok(Json(
        dedup_execute_batch_for_state(state, req, "api", None).await?,
    ))
}

pub async fn dedup_execute_batch_for_state(
    state: AppState,
    req: DedupExecuteBatchRequest,
    source: &str,
    label: Option<&str>,
) -> AppResult<TaskRun> {
    if req.groups.is_empty() {
        return Err(AppError::BadRequest("groups must not be empty".to_string()));
    }
    if !req.groups.iter().any(|group| !group.remove.is_empty()) {
        return Err(AppError::BadRequest(
            "groups must include at least one folder to remove".to_string(),
        ));
    }
    let groups = req.groups;
    let params = serde_json::to_value(DedupExecuteBatchRequest {
        groups: groups.clone(),
    })
    .unwrap_or_else(|_| serde_json::json!({}));
    let task_label = label
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("批量去重: {} 组", groups.len()));
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "dedup_exec_batch",
        &task_label,
        groups.len() as i64,
        source,
        params,
    )
    .await?;
    spawn_dedup_batch(state, task.id, groups);
    Ok(task)
}

#[utoipa::path(post, path = "/api/v2/dedup/replace", tag = "dedup", request_body = ReplaceRequest, responses((status = 200, body = ReplaceExecuteResponse)))]
pub async fn replace_execute(
    State(state): State<AppState>,
    Json(req): Json<ReplaceRequest>,
) -> AppResult<Json<ReplaceExecuteResponse>> {
    let plan = plan_replace_for_roots(&state.settings.cd_root, &state.settings.strm_root, &req)?;
    let emby_client = dedup_emby_client(&state).await?;
    Ok(Json(
        run_replace_execute(&state, &req, plan, &emby_client).await?,
    ))
}

#[utoipa::path(post, path = "/api/v2/dedup/replace-batch", tag = "dedup", request_body = ReplaceBatchRequest, responses((status = 200, body = TaskRun)))]
pub async fn replace_batch(
    State(state): State<AppState>,
    Json(req): Json<ReplaceBatchRequest>,
) -> AppResult<Json<TaskRun>> {
    if req.items.is_empty() {
        return Err(AppError::BadRequest("items must not be empty".to_string()));
    }
    let items = req.items;
    let params = serde_json::to_value(ReplaceBatchRequest {
        items: items.clone(),
    })
    .unwrap_or_else(|_| serde_json::json!({}));
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "replace_batch",
        &format!("批量替换: {} 项", items.len()),
        items.len() as i64,
        "api",
        params,
    )
    .await?;
    spawn_replace_batch(state, task.id, items);
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/dedup/auto-all", tag = "dedup", request_body = DedupAutoAllRequest, responses((status = 200, body = DedupAutoAllResponse)))]
pub async fn auto_all(
    State(state): State<AppState>,
    body: Option<Json<DedupAutoAllRequest>>,
) -> AppResult<Json<DedupAutoAllResponse>> {
    let req = body.map(|Json(req)| req).unwrap_or_default();
    let analysis = analyze_duplicate_groups_for_state(&state).await?;
    let review_count = analysis.review.len();
    let limit = req
        .limit
        .unwrap_or(MAX_AUTO_ALL_GROUPS)
        .min(MAX_AUTO_ALL_GROUPS);
    let mut results = Vec::new();

    let groups = analysis.dups.into_iter().take(limit).collect::<Vec<_>>();
    let has_removals = groups.iter().any(|group| !group.remove.is_empty());
    let emby_client = if has_removals {
        Some(dedup_emby_client(&state).await?)
    } else {
        None
    };
    let c115_delete = if has_removals {
        dedup_c115_delete_context(&state).await?
    } else {
        None
    };

    for group in groups {
        let mut removed = 0usize;
        let mut err = None;
        for row in &group.remove {
            let item = DedupFolderRef {
                lib: row.lib.clone(),
                folder: row.folder.clone(),
                item_id: row.item_id.clone(),
            };
            let client = emby_client
                .as_ref()
                .expect("dedup auto-all groups with removals require an Emby client");
            match delete_duplicate_folder(&state, &item, client, c115_delete.as_ref()).await {
                Ok(_) => removed += 1,
                Err(e) => {
                    err = Some(e.to_string());
                    break;
                }
            }
        }
        results.push(DedupAutoAllItemResult {
            tmdb: group.tmdb,
            ok: err.is_none(),
            kept: group.keep.folder,
            removed,
            err,
        });
    }

    let ok_count = results.iter().filter(|item| item.ok).count();
    let total_removed_folders = results.iter().map(|item| item.removed).sum();
    Ok(Json(DedupAutoAllResponse {
        total: results.len(),
        results,
        ok_count,
        total_removed_folders,
        review_count,
        async_requested: req.async_requested.unwrap_or(false),
    }))
}

fn spawn_dedup_batch(state: AppState, task_id: Uuid, groups: Vec<DedupExecuteBatchGroup>) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, task_id, "任务并发槽不可用", None).await;
            return;
        };
        let Ok(_cloud_permit) = state.clouddrive_slot.clone().acquire_owned().await else {
            let _ =
                tasks::finish_error(&state.pool, task_id, "CloudDrive 串行锁不可用", None).await;
            return;
        };
        let _ = tasks::mark_running(&state.pool, task_id, "批量去重启动").await;
        let emby_client = match dedup_emby_client(&state).await {
            Ok(client) => client,
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, task_id, &err.to_string(), None).await;
                return;
            }
        };
        let c115_delete = match dedup_c115_delete_context(&state).await {
            Ok(context) => context,
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, task_id, &err.to_string(), None).await;
                return;
            }
        };

        let mut results = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            if tasks::cancel_requested(&state.pool, task_id).await {
                let _ = tasks::finish_cancelled(&state.pool, task_id).await;
                return;
            }
            let label = group.tmdb.as_deref().unwrap_or("(no tmdb)");
            let _ = tasks::set_progress(
                &state.pool,
                task_id,
                index as i64,
                &format!("去重 tmdb {label}"),
            )
            .await;

            let mut removed = 0usize;
            let mut err = None;
            let mut errors = Vec::new();
            let mut warnings = Vec::new();
            for item in &group.remove {
                match delete_duplicate_folder(&state, item, &emby_client, c115_delete.as_ref())
                    .await
                {
                    Ok(result) => {
                        removed += 1;
                        warnings.extend(
                            result.warnings.into_iter().map(|warning| {
                                format!("{} / {}: {warning}", item.lib, item.folder)
                            }),
                        );
                    }
                    Err(current) => {
                        let detail = format!(
                            "{} / {} item_id={}: {current}",
                            item.lib,
                            item.folder,
                            item.item_id.as_deref().unwrap_or("-")
                        );
                        errors.push(detail.clone());
                        err = Some(detail);
                        break;
                    }
                }
            }
            results.push(DedupExecuteBatchItemResult {
                tmdb: group.tmdb.clone(),
                ok: err.is_none(),
                removed,
                err,
                errors,
                warnings,
            });
            let _ = tasks::set_progress(
                &state.pool,
                task_id,
                (index + 1) as i64,
                &format!("已处理 {}/{}", index + 1, groups.len()),
            )
            .await;
        }

        let ok_count = results.iter().filter(|item| item.ok).count();
        let total = results.len();
        let error_count = total.saturating_sub(ok_count);
        let result = DedupExecuteBatchResult {
            results,
            ok: error_count == 0,
            ok_count,
            error_count,
            total,
        };
        let result_value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
        if error_count > 0 {
            let _ = tasks::finish_error(
                &state.pool,
                task_id,
                &format!("批量去重失败: {error_count}/{total} 组失败"),
                Some(result_value),
            )
            .await;
        } else {
            let _ = tasks::finish_done_with_message(
                &state.pool,
                task_id,
                &format!("批量去重完成: {ok_count}/{total}"),
                result_value,
            )
            .await;
        }
    });
}

fn spawn_replace_batch(state: AppState, task_id: Uuid, items: Vec<ReplaceRequest>) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, task_id, "任务并发槽不可用", None).await;
            return;
        };
        let Ok(_cloud_permit) = state.clouddrive_slot.clone().acquire_owned().await else {
            let _ =
                tasks::finish_error(&state.pool, task_id, "CloudDrive 串行锁不可用", None).await;
            return;
        };
        let _ = tasks::mark_running(&state.pool, task_id, "批量替换启动").await;
        let emby_client = match dedup_emby_client(&state).await {
            Ok(client) => client,
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, task_id, &err.to_string(), None).await;
                return;
            }
        };

        let mut results = Vec::new();
        for (index, req) in items.iter().enumerate() {
            if tasks::cancel_requested(&state.pool, task_id).await {
                let _ = tasks::finish_cancelled(&state.pool, task_id).await;
                return;
            }
            let _ = tasks::set_progress(
                &state.pool,
                task_id,
                index as i64,
                &format!("替换 {}", req.lose_folder.trim()),
            )
            .await;

            let result = match plan_replace_for_roots(
                &state.settings.cd_root,
                &state.settings.strm_root,
                req,
            ) {
                Ok(plan) => match run_replace_execute(&state, req, plan, &emby_client).await {
                    Ok(response) => ReplaceBatchItemResult {
                        lib: req.lib.clone(),
                        win: req.win_folder.clone(),
                        lose: req.lose_folder.clone(),
                        ok: true,
                        kept_as: Some(response.kept_as),
                        msg: response.msg,
                        err: None,
                    },
                    Err(err) => ReplaceBatchItemResult {
                        lib: req.lib.clone(),
                        win: req.win_folder.clone(),
                        lose: req.lose_folder.clone(),
                        ok: false,
                        kept_as: None,
                        msg: String::new(),
                        err: Some(err.to_string()),
                    },
                },
                Err(err) => ReplaceBatchItemResult {
                    lib: req.lib.clone(),
                    win: req.win_folder.clone(),
                    lose: req.lose_folder.clone(),
                    ok: false,
                    kept_as: None,
                    msg: String::new(),
                    err: Some(err.to_string()),
                },
            };
            results.push(result);
            let _ = tasks::set_progress(
                &state.pool,
                task_id,
                (index + 1) as i64,
                &format!("已处理 {}/{}", index + 1, items.len()),
            )
            .await;
        }

        let ok_count = results.iter().filter(|item| item.ok).count();
        let total = results.len();
        let result = ReplaceBatchResult {
            results,
            ok_count,
            total,
        };
        let _ = tasks::finish_done_with_message(
            &state.pool,
            task_id,
            &format!("批量替换完成: {ok_count}/{total}"),
            serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
        )
        .await;
    });
}

pub fn analyze_duplicate_groups(strm_root: &Path) -> AppResult<DedupAnalysisResponse> {
    let mut groups: BTreeMap<String, Vec<ScannedFolder>> = BTreeMap::new();
    let Ok(libs) = fs::read_dir(strm_root) else {
        return Ok(DedupAnalysisResponse {
            dups: Vec::new(),
            review: Vec::new(),
        });
    };

    for lib_entry in libs.filter_map(Result::ok) {
        let Ok(file_type) = lib_entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let lib = lib_entry.file_name().to_string_lossy().to_string();
        let Ok(folders) = fs::read_dir(lib_entry.path()) else {
            continue;
        };
        for folder_entry in folders.filter_map(Result::ok) {
            let Ok(file_type) = folder_entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let folder = folder_entry.file_name().to_string_lossy().to_string();
            let Some(tmdb) = extract_tmdb_id(&folder) else {
                continue;
            };
            let (medias, score) = scan_strm_folder(&folder_entry.path());
            groups.entry(tmdb).or_default().push(ScannedFolder {
                row: DedupRow {
                    lib: lib.clone(),
                    folder,
                    score,
                    n: medias.len(),
                    item_id: None,
                },
                medias,
            });
        }
    }

    let mut dups = Vec::new();
    let mut review = Vec::new();
    for (tmdb, mut rows) in groups {
        if rows.len() < 2 {
            continue;
        }
        rows.sort_by(compare_scanned_folder);
        let shared = has_shared_media(&rows);
        let looks_series = rows
            .iter()
            .any(|item| item.row.n > 1 || item.row.lib.contains("追更"));
        let public_rows = rows.iter().map(|item| item.row.clone()).collect::<Vec<_>>();
        if shared {
            review.push(DedupReviewGroup {
                tmdb,
                why: "多库共享同一文件(删文件会双双坏)".to_string(),
                rows: public_rows,
            });
            continue;
        }
        if looks_series {
            review.push(DedupReviewGroup {
                tmdb,
                why: "疑似剧集或追更库,请人工确认是否真重复".to_string(),
                rows: public_rows,
            });
            continue;
        }
        let keep = rows[0].row.clone();
        let remove = rows
            .iter()
            .skip(1)
            .map(|item| item.row.clone())
            .collect::<Vec<_>>();
        dups.push(DedupGroup { tmdb, keep, remove });
    }

    Ok(DedupAnalysisResponse { dups, review })
}

pub(crate) async fn analyze_duplicate_groups_for_state(
    state: &AppState,
) -> AppResult<DedupAnalysisResponse> {
    let mut analysis = analyze_duplicate_groups(&state.settings.strm_root)?;
    let Ok(client) = optional_dedup_emby_client(state).await else {
        return Ok(analysis);
    };
    let Ok(emby_groups) = collect_emby_duplicate_groups(state, &client).await else {
        return Ok(analysis);
    };
    append_emby_review_groups(&mut analysis, emby_groups);
    Ok(analysis)
}

async fn optional_dedup_emby_client(state: &AppState) -> AppResult<EmbyClient> {
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured".to_string(),
        ));
    }
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    Ok(EmbyClient::new(emby_url, api_key, state.http.clone()))
}

async fn collect_emby_duplicate_groups(
    state: &AppState,
    client: &EmbyClient,
) -> anyhow::Result<BTreeMap<String, Vec<DedupRow>>> {
    let mut by_tmdb: BTreeMap<String, Vec<DedupRow>> = BTreeMap::new();
    for library in client.libraries().await? {
        let Some(parent_id) = library.id.as_deref().filter(|id| !id.trim().is_empty()) else {
            continue;
        };
        let item_types = emby_item_types(&library);
        let lib_folder = library_folder_name(&library);
        let items = client.library_items(parent_id, item_types, 100_000).await?;
        for item in items.items {
            let Some(tmdb) = item.provider_id("Tmdb") else {
                continue;
            };
            let Some(folder) = item
                .path
                .as_deref()
                .and_then(|path| folder_from_emby_path(path, &library))
                .or(item.name)
                .filter(|folder| !folder.trim().is_empty())
            else {
                continue;
            };
            let strm_folder = state.settings.strm_root.join(&lib_folder).join(&folder);
            by_tmdb.entry(tmdb).or_default().push(DedupRow {
                lib: lib_folder.clone(),
                folder,
                score: 0,
                n: count_strm_files(&strm_folder),
                item_id: item.id,
            });
        }
    }
    Ok(by_tmdb)
}

fn append_emby_review_groups(
    analysis: &mut DedupAnalysisResponse,
    emby_groups: BTreeMap<String, Vec<DedupRow>>,
) {
    for (tmdb, mut rows) in emby_groups {
        dedup_rows_by_lib_folder(&mut rows);
        if rows.len() < 2 || analysis_has_tmdb(analysis, &tmdb) {
            continue;
        }
        rows.sort_by(compare_dedup_row);
        analysis.review.push(DedupReviewGroup {
            tmdb,
            why: "Emby ProviderIds.Tmdb 相同，媒体库内仍有重复 Item；可勾选旧目录/副本删除"
                .to_string(),
            rows,
        });
    }
}

fn dedup_rows_by_lib_folder(rows: &mut Vec<DedupRow>) {
    let mut seen = BTreeSet::new();
    rows.retain(|row| seen.insert((row.lib.clone(), row.folder.clone())));
}

fn analysis_has_tmdb(analysis: &DedupAnalysisResponse, tmdb: &str) -> bool {
    analysis.dups.iter().any(|group| group.tmdb == tmdb)
        || analysis.review.iter().any(|group| group.tmdb == tmdb)
}

fn compare_dedup_row(a: &DedupRow, b: &DedupRow) -> Ordering {
    b.score
        .cmp(&a.score)
        .then_with(|| b.n.cmp(&a.n))
        .then_with(|| {
            duplicate_suffix_base(&a.folder)
                .is_some()
                .cmp(&duplicate_suffix_base(&b.folder).is_some())
        })
        .then_with(|| a.lib.cmp(&b.lib))
        .then_with(|| a.folder.cmp(&b.folder))
}

fn emby_item_types(library: &EmbyLibrary) -> &'static str {
    match library.library_type.to_ascii_lowercase().as_str() {
        "movies" | "movie" => "Movie",
        "tvshows" | "series" | "shows" => "Series",
        _ => "Movie,Series",
    }
}

fn folder_from_emby_path(path: &str, library: &EmbyLibrary) -> Option<String> {
    let path = normalize_slashes(path);
    for root in &library.paths {
        let root = normalize_slashes(root);
        if root.is_empty() {
            continue;
        }
        let rest = if path == root {
            ""
        } else if let Some(rest) = path.strip_prefix(&(root + "/")) {
            rest
        } else {
            continue;
        };
        return rest
            .split('/')
            .find(|part| !part.trim().is_empty())
            .map(trim_media_extension);
    }
    path.rsplit('/').next().map(trim_media_extension)
}

fn normalize_slashes(value: &str) -> String {
    value.replace('\\', "/").trim_end_matches('/').to_string()
}

fn trim_media_extension(value: &str) -> String {
    for ext in [".strm", ".mkv", ".mp4", ".avi", ".mov", ".ts"] {
        if value.len() > ext.len() && value.to_ascii_lowercase().ends_with(ext) {
            return value[..value.len() - ext.len()].to_string();
        }
    }
    value.to_string()
}

fn count_strm_files(folder: &Path) -> usize {
    if !folder.is_dir() {
        return 0;
    }
    WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("strm"))
        })
        .count()
}

pub fn plan_replace_for_roots(
    cd_root: impl AsRef<Path>,
    strm_root: impl AsRef<Path>,
    req: &ReplaceRequest,
) -> AppResult<ReplacePlan> {
    let lib = required_value(&req.lib, "lib")?;
    let win_folder = required_value(&req.win_folder, "win_folder")?;
    let lose_folder = required_value(&req.lose_folder, "lose_folder")?;
    if win_folder == lose_folder {
        return Err(AppError::Conflict(
            "win_folder 和 lose_folder 不能相同".to_string(),
        ));
    }

    let cd_lib = safe_under(cd_root, lib)?;
    let strm_lib = safe_under(strm_root, lib)?;
    let win_cd = safe_under(&cd_lib, win_folder)?;
    let lose_cd = safe_under(&cd_lib, lose_folder)?;
    let win_strm = safe_under(&strm_lib, win_folder)?;
    let lose_strm = safe_under(&strm_lib, lose_folder)?;

    ensure_real_dir(&lose_cd, &format!("源 folder 不存在: {lose_folder}"))?;
    ensure_real_dir(&win_cd, &format!("新 folder 不存在: {win_folder}"))?;
    reject_nested_replace_paths(&win_cd, &lose_cd)?;

    let rename_win = duplicate_suffix_base(win_folder).is_some_and(|base| base == lose_folder);
    let kept_as = if rename_win { lose_folder } else { win_folder }.to_string();
    let target_cd = safe_under(&cd_lib, &kept_as)?;
    let target_strm = safe_under(&strm_lib, &kept_as)?;
    if rename_win && target_cd.exists() && !same_existing_path(&target_cd, &lose_cd) {
        return Err(AppError::Conflict(format!(
            "目标已存在同名文件夹: {}",
            target_cd.display()
        )));
    }

    Ok(ReplacePlan {
        lib: lib.to_string(),
        win_folder: win_folder.to_string(),
        lose_folder: lose_folder.to_string(),
        kept_as,
        rename_win,
        win_cd,
        lose_cd,
        target_cd,
        win_strm,
        lose_strm,
        target_strm,
        emby_updates: replace_updates(lib, win_folder, lose_folder, rename_win),
    })
}

async fn run_replace_execute(
    state: &AppState,
    req: &ReplaceRequest,
    plan: ReplacePlan,
    emby_client: &EmbyClient,
) -> AppResult<ReplaceExecuteResponse> {
    let mut deleted_from = Vec::new();
    if remove_path_if_exists(&plan.lose_cd).await? {
        deleted_from.push("115".to_string());
    }
    if remove_path_if_exists(&plan.lose_strm).await? {
        deleted_from.push("strm".to_string());
    }

    if plan.rename_win {
        if plan.target_cd.exists() {
            return Err(AppError::Conflict(format!(
                "目标已存在同名文件夹: {}",
                plan.target_cd.display()
            )));
        }
        create_parent(&plan.target_cd).await?;
        tokio::fs::rename(&plan.win_cd, &plan.target_cd)
            .await
            .map_err(anyhow::Error::from)?;

        if plan.win_strm.exists() {
            if plan.target_strm.exists() {
                return Err(AppError::Conflict(format!(
                    "STRM 目标已存在,为避免覆盖未执行: {}",
                    plan.target_strm.display()
                )));
            }
            create_parent(&plan.target_strm).await?;
            tokio::fs::rename(&plan.win_strm, &plan.target_strm)
                .await
                .map_err(anyhow::Error::from)?;
            rewrite_strm_folder(&plan.target_strm, &plan.win_folder, &plan.lose_folder)?;
            chmod_public_subtree(&plan.target_strm);
        }
    }

    let kept_strm = if plan.rename_win {
        &plan.target_strm
    } else {
        &plan.win_strm
    };
    if kept_strm.is_dir() {
        chmod_public_subtree(kept_strm);
    }

    let undo_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "lib": req.lib.trim(),
        "win_was": req.win_folder.trim(),
        "lose_was": req.lose_folder.trim(),
        "now_folder": plan.kept_as.clone(),
    });
    sqlx::query("INSERT INTO undo_entries(id, op, payload) VALUES ($1, 'replace', $2)")
        .bind(undo_id)
        .bind(payload)
        .execute(&state.pool)
        .await?;

    let msg = if plan.rename_win {
        format!(
            "已替换:删了「{}」新 folder 改名回「{}」",
            plan.lose_folder, plan.kept_as
        )
    } else {
        format!("已替换:删了「{}」", plan.lose_folder)
    };

    let notified = if deleted_from.is_empty() {
        false
    } else {
        notify_emby_updates(emby_client, &plan.emby_updates).await?
    };

    Ok(ReplaceExecuteResponse {
        ok: true,
        lib: plan.lib,
        kept_as: plan.kept_as,
        dropped: plan.lose_folder,
        renamed: plan.rename_win,
        deleted_from,
        emby_updates: plan.emby_updates,
        notified,
        undo_id,
        msg,
    })
}

async fn delete_duplicate_folder(
    state: &AppState,
    item: &DedupFolderRef,
    emby_client: &EmbyClient,
    c115_delete: Option<&C115DeleteContext>,
) -> AppResult<DedupDeleteResult> {
    let lib = required_value(&item.lib, "lib")?;
    let folder = required_value(&item.folder, "folder")?;
    let mut deleted_from = Vec::new();
    let mut warnings = Vec::new();
    let emby_item_id = item
        .item_id
        .as_deref()
        .and_then(non_empty_trimmed)
        .map(ToString::to_string);
    if let Some(item_id) = emby_item_id.as_deref() {
        match emby_client.delete_item(item_id).await {
            Ok(_) => deleted_from.push("emby".to_string()),
            Err(err) => {
                let warning = format!(
                    "Emby 删除 item_id={item_id} 失败，已继续清理本地/115 并通知媒体库刷新: {err}"
                );
                tracing::warn!(
                    item_id,
                    lib,
                    folder,
                    error = %err,
                    "Emby item delete failed during dedup; continuing path cleanup"
                );
                warnings.push(warning);
            }
        }
    }

    let cd_lib = match safe_under(&state.settings.cd_root, lib) {
        Ok(path) => path,
        Err(_err) if !deleted_from.is_empty() => {
            return finish_duplicate_delete(
                state,
                emby_client,
                lib,
                folder,
                deleted_from,
                warnings,
            )
            .await;
        }
        Err(err) => return Err(err),
    };
    let strm_lib = match safe_under(&state.settings.strm_root, lib) {
        Ok(path) => path,
        Err(_err) if !deleted_from.is_empty() => {
            return finish_duplicate_delete(
                state,
                emby_client,
                lib,
                folder,
                deleted_from,
                warnings,
            )
            .await;
        }
        Err(err) => return Err(err),
    };
    let cd_target = match safe_under(&cd_lib, folder) {
        Ok(path) => path,
        Err(_err) if !deleted_from.is_empty() => {
            return finish_duplicate_delete(
                state,
                emby_client,
                lib,
                folder,
                deleted_from,
                warnings,
            )
            .await;
        }
        Err(err) => return Err(err),
    };
    let strm_target = match safe_under(&strm_lib, folder) {
        Ok(path) => path,
        Err(_err) if !deleted_from.is_empty() => {
            return finish_duplicate_delete(
                state,
                emby_client,
                lib,
                folder,
                deleted_from,
                warnings,
            )
            .await;
        }
        Err(err) => return Err(err),
    };

    if delete_c115_folder(c115_delete, lib, folder, &cd_target).await? {
        deleted_from.push("115".to_string());
    }
    if remove_path_if_exists(&strm_target).await? {
        deleted_from.push("strm".to_string());
    }
    reconcile_emby_after_path_cleanup(
        emby_client,
        emby_item_id.as_deref(),
        &mut deleted_from,
        &mut warnings,
    )
    .await?;

    finish_duplicate_delete(state, emby_client, lib, folder, deleted_from, warnings).await
}

async fn reconcile_emby_after_path_cleanup(
    emby_client: &EmbyClient,
    item_id: Option<&str>,
    deleted_from: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> AppResult<()> {
    let Some(item_id) = item_id else {
        return Ok(());
    };
    if deleted_from.iter().any(|source| source == "emby")
        || !deleted_from
            .iter()
            .any(|source| source == "115" || source == "strm")
    {
        return Ok(());
    }

    match emby_client.delete_item(item_id).await {
        Ok(_) => {
            deleted_from.push("emby".to_string());
            warnings.push(format!("路径清理后已二次删除 Emby item_id={item_id}"));
            Ok(())
        }
        Err(delete_err) => match emby_client.item_exists(item_id).await {
            Ok(false) => {
                warnings.push(format!(
                    "Emby 二次删除 item_id={item_id} 返回错误，但回查条目已消失: {delete_err}"
                ));
                Ok(())
            }
            Ok(true) => Err(AppError::Conflict(format!(
                "路径已清理，但 Emby item_id={item_id} 仍存在；二次删除失败: {delete_err}"
            ))),
            Err(check_err) => Err(AppError::Conflict(format!(
                "路径已清理，但无法确认 Emby item_id={item_id} 是否消失；二次删除失败: {delete_err}; 回查失败: {check_err}"
            ))),
        },
    }
}

async fn finish_duplicate_delete(
    state: &AppState,
    emby_client: &EmbyClient,
    lib: &str,
    folder: &str,
    deleted_from: Vec<String>,
    warnings: Vec<String>,
) -> AppResult<DedupDeleteResult> {
    if deleted_from.is_empty() {
        return Err(AppError::NotFound(format!(
            "未找到可删除的 Emby/115/STRM 资源: {lib}/{folder}"
        )));
    }

    let undo_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "lib": lib,
        "folder": folder,
        "deleted_from": deleted_from.clone(),
    });
    sqlx::query("INSERT INTO undo_entries(id, op, payload) VALUES ($1, 'delete', $2)")
        .bind(undo_id)
        .bind(payload)
        .execute(&state.pool)
        .await?;

    let removed_media_folder = deleted_from
        .iter()
        .any(|source| source == "115" || source == "strm");
    let emby_updates = if removed_media_folder {
        vec![EmbyUpdate {
            path: format!("/strm/{lib}/{folder}"),
            update_type: "Deleted".to_string(),
        }]
    } else {
        Vec::new()
    };
    let notified = notify_emby_updates(emby_client, &emby_updates).await?;

    Ok(DedupDeleteResult {
        lib: lib.to_string(),
        folder: folder.to_string(),
        deleted_from,
        emby_updates,
        notified,
        undo_id,
        warnings,
    })
}

async fn dedup_c115_delete_context(state: &AppState) -> AppResult<Option<C115DeleteContext>> {
    let cookie = match c115::require_c115_cookie(
        config_store::get_string(&state.pool, "c115_cookie").await?,
    ) {
        Ok(cookie) => cookie,
        Err(_) => return Ok(None),
    };
    let cid_map = c115::cid_map(&state.pool).await?;
    if cid_map.is_empty() {
        return Ok(None);
    }
    Ok(Some(C115DeleteContext {
        client: C115Client::new_with_site(C115_API, C115_SITE, cookie, state.http.clone()),
        cid_map,
    }))
}

async fn delete_c115_folder(
    context: Option<&C115DeleteContext>,
    lib: &str,
    folder: &str,
    cd_target: &Path,
) -> AppResult<bool> {
    if let Some(context) = context
        && let Some(parent_cid) = context.cid_map.get(lib)
    {
        match context.client.delete_child_dir(parent_cid, folder).await {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(err) => {
                return Err(AppError::BadRequest(format!(
                    "115 网盘目录删除失败: {lib}/{folder}: {err}"
                )));
            }
        }
    }

    remove_path_if_exists(cd_target).await.map_err(|err| {
        AppError::BadRequest(format!(
            "115 挂载目录删除失败: {}: {err}",
            cd_target.display()
        ))
    })
}

async fn dedup_emby_client(state: &AppState) -> AppResult<EmbyClient> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;
    Ok(EmbyClient::new(emby_url, api_key, state.http.clone()))
}

fn ensure_api_key_configured(api_key: &str) -> AppResult<()> {
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; set it via /api/v2/config before executing dedup writes"
                .to_string(),
        ));
    }
    Ok(())
}

async fn notify_emby_updates(client: &EmbyClient, updates: &[EmbyUpdate]) -> AppResult<bool> {
    if updates.is_empty() {
        return Ok(false);
    }
    client
        .notify_media_updated(
            updates
                .iter()
                .map(|update| (update.path.as_str(), update.update_type.as_str())),
        )
        .await?;
    Ok(true)
}

fn scan_strm_folder(path: &Path) -> (Vec<String>, i64) {
    let mut medias = Vec::new();
    let mut score = 0i64;
    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let is_strm = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("strm"));
        if !is_strm {
            continue;
        }
        let media = fs::read_to_string(entry.path())
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|_| entry.file_name().to_string_lossy().to_string());
        score = score.max(quality_score(&media));
        medias.push(media);
    }
    (medias, score)
}

fn compare_scanned_folder(a: &ScannedFolder, b: &ScannedFolder) -> Ordering {
    b.row
        .score
        .cmp(&a.row.score)
        .then_with(|| b.row.n.cmp(&a.row.n))
        .then_with(|| {
            duplicate_suffix_base(&a.row.folder)
                .is_some()
                .cmp(&duplicate_suffix_base(&b.row.folder).is_some())
        })
        .then_with(|| a.row.folder.len().cmp(&b.row.folder.len()))
        .then_with(|| a.row.folder.cmp(&b.row.folder))
}

fn has_shared_media(rows: &[ScannedFolder]) -> bool {
    let mut seen = BTreeSet::new();
    for media in rows.iter().flat_map(|item| &item.medias) {
        if !seen.insert(media) {
            return true;
        }
    }
    false
}

fn extract_tmdb_id(folder: &str) -> Option<String> {
    let lower = folder.to_ascii_lowercase();
    let marker = lower
        .find("tmdbid-")
        .map(|idx| idx + "tmdbid-".len())
        .or_else(|| lower.find("tmdbid_").map(|idx| idx + "tmdbid_".len()))?;
    let digits = lower[marker..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then_some(digits)
}

fn quality_score(path: &str) -> i64 {
    let lower = path.to_ascii_lowercase();
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
    if lower.contains("cam") || lower.contains("ts") {
        score -= 500;
    }
    score
}

fn required_value<'a>(value: &'a str, field: &str) -> AppResult<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} 不能为空")));
    }
    Ok(value)
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn ensure_real_dir(path: &Path, msg: &str) -> AppResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            AppError::NotFound(msg.to_string())
        } else {
            AppError::Anyhow(err.into())
        }
    })?;
    if !metadata.file_type().is_dir() {
        return Err(AppError::NotFound(msg.to_string()));
    }
    Ok(())
}

fn reject_nested_replace_paths(win_cd: &Path, lose_cd: &Path) -> AppResult<()> {
    let win = win_cd.canonicalize().map_err(anyhow::Error::from)?;
    let lose = lose_cd.canonicalize().map_err(anyhow::Error::from)?;
    if win.starts_with(&lose) || lose.starts_with(&win) {
        return Err(AppError::Conflict(
            "win_folder 和 lose_folder 不能互为父子目录".to_string(),
        ));
    }
    Ok(())
}

fn duplicate_suffix_base(folder: &str) -> Option<&str> {
    let folder = folder.trim_end();
    let (close_idx, close) = folder.char_indices().last()?;
    if close != ')' && close != '）' {
        return None;
    }
    for (open_idx, open) in folder[..close_idx].char_indices().rev() {
        if open != '(' && open != '（' {
            continue;
        }
        let digits = &folder[open_idx + open.len_utf8()..close_idx];
        if !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()) && open_idx > 0 {
            return Some(folder[..open_idx].trim_end());
        }
        return None;
    }
    None
}

fn replace_updates(
    lib: &str,
    win_folder: &str,
    lose_folder: &str,
    rename_win: bool,
) -> Vec<EmbyUpdate> {
    let path = |folder: &str| format!("/strm/{lib}/{folder}");
    if rename_win {
        vec![
            EmbyUpdate {
                path: path(win_folder),
                update_type: "Deleted".to_string(),
            },
            EmbyUpdate {
                path: path(lose_folder),
                update_type: "Modified".to_string(),
            },
        ]
    } else {
        vec![
            EmbyUpdate {
                path: path(lose_folder),
                update_type: "Deleted".to_string(),
            },
            EmbyUpdate {
                path: path(win_folder),
                update_type: "Created".to_string(),
            },
        ]
    }
}

fn same_existing_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

async fn remove_path_if_exists(path: &Path) -> AppResult<bool> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(AppError::Anyhow(err.into())),
    };
    if metadata.file_type().is_dir() {
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

async fn create_parent(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(anyhow::Error::from)?;
    }
    Ok(())
}

fn rewrite_strm_folder(path: &Path, old_folder: &str, new_folder: &str) -> AppResult<usize> {
    let needle = format!("/{old_folder}/");
    let replacement = format!("/{new_folder}/");
    let mut changed = 0usize;
    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let is_strm = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("strm"));
        if !is_strm {
            continue;
        }
        let content = fs::read_to_string(entry.path()).map_err(anyhow::Error::from)?;
        let new_content = content.replacen(&needle, &replacement, 1);
        if new_content != content {
            fs::write(entry.path(), new_content).map_err(anyhow::Error::from)?;
            chmod_public_file(entry.path());
            changed += 1;
        }
    }
    Ok(changed)
}

fn chmod_public_subtree(path: &Path) {
    chmod_public_dir(path);
    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_dir() {
            chmod_public_dir(entry.path());
        } else if entry.file_type().is_file() {
            chmod_public_file(entry.path());
        }
    }
}

#[cfg(unix)]
fn chmod_public_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.mode() & 0o777 != 0o755 {
            permissions.set_mode(0o755);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

#[cfg(not(unix))]
fn chmod_public_dir(_path: &Path) {}

#[cfg(unix)]
fn chmod_public_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.mode() & 0o777 != 0o644 {
            permissions.set_mode(0o644);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

#[cfg(not(unix))]
fn chmod_public_file(_path: &Path) {}
