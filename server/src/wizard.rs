use crate::{
    c115::{self, C115Client, C115OfflineRequest, C115SaveRequest},
    config_store,
    dedup::{self, DedupGroup, DedupReviewGroup, DedupRow},
    emby::{EmbyClient, EmbyItem, EmbyLibrary},
    error::{AppError, AppResult},
    media_fs::{self, ManageDeleteExecuteResult, ManageDeleteRequest, StrmGenerateResult},
    posters::{self, PosterApplyRequest, PosterDetectRequest},
    state::AppState,
    tasks::{self, TaskRun},
};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::time::sleep;
use uuid::Uuid;

const C115_COOKIE_KEY: &str = "c115_cookie";
const C115_CID_MAP_KEY: &str = "c115_cid_map";
const C115_API_BASE_URL_KEY: &str = "c115_api_base_url";
const C115_SITE_BASE_URL_KEY: &str = "c115_site_base_url";
const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const DEFAULT_STAGE_DELAY_MS: u64 = 500;
const MAX_STAGE_DELAY_MS: u64 = 30_000;
const DEFAULT_POSTER_SCAN_LIMIT: usize = 200;
const EMBY_DEDUP_SETTLE_TIMEOUT_MS: u64 = 15_000;
const EMBY_DEDUP_SETTLE_INTERVAL_MS: u64 = 1_500;

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct AddNewRequest {
    #[serde(default)]
    pub items: Vec<AddNewItem>,
    pub target: Option<AddNewTarget>,
    pub lib: Option<String>,
    pub cid: Option<String>,
    #[serde(alias = "stage_delay_ms", alias = "wait_delay_ms")]
    pub delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct AddNewItem {
    #[serde(alias = "link")]
    pub url: String,
    #[serde(alias = "receive_code")]
    pub pwd: Option<String>,
    #[serde(alias = "name")]
    pub label: Option<String>,
    pub file_ids: Option<Vec<String>>,
    #[serde(alias = "action", alias = "link_type")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct AddNewTarget {
    pub lib: Option<String>,
    pub cid: Option<String>,
}

#[derive(Debug, Clone)]
struct AddNewPlan {
    req: AddNewRequest,
    target_cid: String,
    target_lib: Option<String>,
    cookie: String,
    c115_api_base: String,
    c115_site_base: String,
    emby_url: String,
    emby_api_key: String,
    delay_ms: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewReport {
    pub ok: bool,
    pub target: AddNewTargetReport,
    pub transfer: AddNewTransferSummary,
    pub strm: AddNewStrmReport,
    pub dedup: AddNewDedupReport,
    pub auto_resolve: AddNewAutoResolveReport,
    pub scan: AddNewScanReport,
    pub poster: AddNewPosterReport,
    pub poster_auto_fix: AddNewPosterAutoFixReport,
    pub check: AddNewCheckReport,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewTargetReport {
    pub cid: String,
    pub lib: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewTransferSummary {
    pub ok: bool,
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub items: Vec<AddNewTransferItemReport>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewTransferItemReport {
    pub index: usize,
    pub ok: bool,
    pub action: AddNewTransferAction,
    pub label: Option<String>,
    pub url: String,
    pub response: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AddNewTransferAction {
    SaveShare,
    OfflineDownload,
    Unsupported,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewScanReport {
    pub ok: bool,
    pub triggered: bool,
    pub mode: String,
    pub lib: Option<String>,
    pub item_id: Option<String>,
    pub code: Option<u16>,
    pub delay_ms: u64,
    pub warning: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewStrmReport {
    pub ok: bool,
    pub triggered: bool,
    pub lib: Option<String>,
    pub matched: usize,
    pub new_count: usize,
    pub new_folders: BTreeMap<String, usize>,
    pub attention: Vec<String>,
    pub retried: bool,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewDedupReport {
    pub ok: bool,
    pub triggered: bool,
    pub lib: Option<String>,
    pub dups_count: usize,
    pub review_count: usize,
    pub dups: Vec<DedupGroup>,
    pub review: Vec<DedupReviewGroup>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewAutoResolveReport {
    pub ok: bool,
    pub triggered: bool,
    pub resolved_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub items: Vec<AddNewAutoResolveItemReport>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewAutoResolveItemReport {
    pub tmdb: String,
    pub action: String,
    pub status: String,
    pub kept_lib: String,
    pub kept_folder: String,
    pub removed_lib: Option<String>,
    pub removed_folder: Option<String>,
    pub removed_item_id: Option<String>,
    pub reason: String,
    pub result: Option<ManageDeleteExecuteResult>,
    pub error: Option<String>,
}

impl AddNewAutoResolveReport {
    fn not_triggered() -> Self {
        Self {
            ok: true,
            triggered: false,
            resolved_count: 0,
            skipped_count: 0,
            error_count: 0,
            items: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewPosterReport {
    pub ok: bool,
    pub triggered: bool,
    pub status: String,
    pub scanned_libraries: usize,
    pub scanned_items: usize,
    pub issue_count: usize,
    pub missing_primary_count: usize,
    pub mismatch_count: usize,
    pub truncated: bool,
    pub warnings: Vec<String>,
    pub items: Vec<AddNewPosterIssueReport>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AddNewPosterIssueReport {
    pub id: String,
    pub name: String,
    pub lib: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub has_poster: bool,
    pub score: u16,
    pub reasons: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewPosterAutoFixReport {
    pub ok: bool,
    pub triggered: bool,
    pub fixed_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub items: Vec<AddNewPosterAutoFixItemReport>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewPosterAutoFixItemReport {
    pub id: String,
    pub name: String,
    pub lib: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub tmdb: Option<String>,
    pub status: String,
    pub reason: String,
    pub poster: Option<bool>,
    pub error: Option<String>,
}

impl AddNewPosterAutoFixReport {
    fn not_triggered() -> Self {
        Self {
            ok: true,
            triggered: false,
            fixed_count: 0,
            skipped_count: 0,
            error_count: 0,
            items: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewCheckReport {
    pub ok: bool,
    pub status: String,
    pub item_success_count: usize,
    pub item_error_count: usize,
    pub stage_error_count: usize,
    pub suspicious_count: usize,
    pub items: Vec<AddNewCheckItemReport>,
    pub errors: Vec<AddNewCheckErrorReport>,
    pub suspicious: Vec<AddNewCheckSuspiciousReport>,
    pub message: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewCheckItemReport {
    pub index: usize,
    pub ok: bool,
    pub action: AddNewTransferAction,
    pub label: Option<String>,
    pub url: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewCheckErrorReport {
    pub stage: String,
    pub index: Option<usize>,
    pub label: Option<String>,
    pub message: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AddNewCheckSuspiciousReport {
    pub stage: String,
    pub severity: String,
    pub id: Option<String>,
    pub label: String,
    pub message: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v2/wizard/add-new", post(add_new))
}

#[utoipa::path(post, path = "/api/v2/wizard/add-new", tag = "wizard", request_body = AddNewRequest, responses((status = 200, body = TaskRun)))]
pub async fn add_new(
    State(state): State<AppState>,
    Json(req): Json<AddNewRequest>,
) -> AppResult<Json<TaskRun>> {
    Ok(Json(
        create_add_new_task(state, req, "manual", "add_new", None).await?,
    ))
}

pub async fn create_add_new_task(
    state: AppState,
    req: AddNewRequest,
    source: &str,
    kind: &str,
    label_prefix: Option<&str>,
) -> AppResult<TaskRun> {
    validate_add_new_request(&req)?;
    let (target_cid, target_lib) = resolve_target_cid(&state.pool, &req).await?;
    let cookie =
        c115::require_c115_cookie(config_store::get_string(&state.pool, C115_COOKIE_KEY).await?)?;
    let (c115_api_base, c115_site_base) = c115_base_urls(&state.pool).await?;
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let emby_api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    let delay_ms = req
        .delay_ms
        .unwrap_or(DEFAULT_STAGE_DELAY_MS)
        .min(MAX_STAGE_DELAY_MS);
    let total = (req.items.len() + 3).max(1) as i64;
    let label = match label_prefix.and_then(non_empty_trimmed) {
        Some(prefix) => format!(
            "{prefix}: {}",
            add_new_task_label(req.items.len(), &target_cid, target_lib.as_deref())
        ),
        None => add_new_task_label(req.items.len(), &target_cid, target_lib.as_deref()),
    };
    let params = serde_json::to_value(&req).unwrap_or_else(|_| json!({}));
    let task =
        tasks::insert_task_with_meta(&state.pool, kind, &label, total, source, params).await?;

    spawn_add_new(
        state,
        task.id,
        AddNewPlan {
            req,
            target_cid,
            target_lib,
            cookie,
            c115_api_base,
            c115_site_base,
            emby_url,
            emby_api_key,
            delay_ms,
        },
    );
    Ok(task)
}

fn spawn_add_new(state: AppState, id: Uuid, plan: AddNewPlan) {
    tokio::spawn(async move {
        let Ok(_permit) = state.clouddrive_slot.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, id, "115 任务串行锁不可用", None).await;
            return;
        };
        match run_add_new_pipeline(&state, id, plan).await {
            Ok(report) => {
                let failed = report.transfer.failed;
                let status_text = if failed > 0 {
                    format!("完成，{failed} 项转存/离线失败")
                } else if !report.strm.ok {
                    "完成，STRM 生成失败".to_string()
                } else if !report.scan.ok {
                    "完成，扫描触发失败".to_string()
                } else if !report.poster.ok {
                    "完成，海报检测失败".to_string()
                } else if report.poster_auto_fix.error_count > 0 {
                    format!(
                        "完成，海报自动修复失败 {} 项",
                        report.poster_auto_fix.error_count
                    )
                } else if report.poster_auto_fix.fixed_count > 0
                    && report.auto_resolve.resolved_count > 0
                {
                    format!(
                        "完成，自动修复 {} 个海报并处理 {} 个重复旧版本",
                        report.poster_auto_fix.fixed_count, report.auto_resolve.resolved_count
                    )
                } else if report.poster_auto_fix.fixed_count > 0 {
                    format!(
                        "完成，自动修复 {} 个海报",
                        report.poster_auto_fix.fixed_count
                    )
                } else if report.auto_resolve.error_count > 0 {
                    format!("完成，自动处理失败 {} 项", report.auto_resolve.error_count)
                } else if report.auto_resolve.resolved_count > 0 {
                    format!(
                        "完成，自动处理 {} 个重复旧版本",
                        report.auto_resolve.resolved_count
                    )
                } else if report.check.suspicious_count > 0 {
                    format!("完成，发现 {} 个可疑项", report.check.suspicious_count)
                } else {
                    "完成".to_string()
                };
                let _ = tasks::finish_done_with_message(
                    &state.pool,
                    id,
                    &status_text,
                    serde_json::to_value(report).unwrap_or_else(|_| json!({})),
                )
                .await;
            }
            Err(err) => {
                if err.to_string() != "__task_cancelled__" {
                    let _ = tasks::finish_error(&state.pool, id, &err.to_string(), None).await;
                }
            }
        }
    });
}

async fn run_add_new_pipeline(
    state: &AppState,
    id: Uuid,
    plan: AddNewPlan,
) -> AppResult<AddNewReport> {
    if cancel_if_requested(state, id).await? {
        return Err(AppError::Conflict("__task_cancelled__".to_string()));
    }
    tasks::mark_running(&state.pool, id, "准备一条龙加新资源...").await?;

    let c115_client = C115Client::new_with_site(
        plan.c115_api_base,
        plan.c115_site_base,
        plan.cookie,
        state.http.clone(),
    );
    let emby_client = EmbyClient::new(plan.emby_url, plan.emby_api_key, state.http.clone());

    let total_items = plan.req.items.len();
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut item_reports = Vec::with_capacity(total_items);

    for (index, item) in plan.req.items.iter().enumerate() {
        if cancel_if_requested(state, id).await? {
            return Err(AppError::Conflict("__task_cancelled__".to_string()));
        }
        let label = item_label(item);
        tasks::set_progress(
            &state.pool,
            id,
            index as i64,
            &format!(
                "转存/离线 {}/{}: {}",
                index + 1,
                total_items,
                truncate(&label, 48)
            ),
        )
        .await?;

        let report = execute_transfer_item(
            &c115_client,
            item,
            index,
            &plan.target_cid,
            plan.target_lib.as_deref(),
        )
        .await;
        if report.ok {
            succeeded += 1;
        } else {
            failed += 1;
        }
        item_reports.push(report);
        tasks::set_progress(
            &state.pool,
            id,
            (index + 1) as i64,
            &format!("已处理 {}/{}", index + 1, total_items),
        )
        .await?;
    }

    let transfer = AddNewTransferSummary {
        ok: failed == 0,
        total: total_items,
        succeeded,
        failed,
        items: item_reports,
    };

    if cancel_if_requested(state, id).await? {
        return Err(AppError::Conflict("__task_cancelled__".to_string()));
    }
    tasks::set_progress(&state.pool, id, total_items as i64, "生成目标库缺失 STRM").await?;
    let strm = generate_target_strm(state, &transfer, plan.target_lib.as_deref()).await;

    let mut dedup = inspect_target_dedup(state, &transfer, plan.target_lib.as_deref());

    if plan.delay_ms > 0 {
        tasks::set_progress(
            &state.pool,
            id,
            total_items as i64,
            &format!("等待媒体库可见 {}ms", plan.delay_ms),
        )
        .await?;
        sleep(Duration::from_millis(plan.delay_ms)).await;
    }

    if cancel_if_requested(state, id).await? {
        return Err(AppError::Conflict("__task_cancelled__".to_string()));
    }
    tasks::set_progress(&state.pool, id, total_items as i64, "触发 Emby 刷新").await?;
    let scan = trigger_scan(&emby_client, plan.target_lib.as_deref(), plan.delay_ms).await;
    tasks::set_progress(
        &state.pool,
        id,
        (total_items + 1) as i64,
        "Emby 刷新阶段完成",
    )
    .await?;

    let poster_before_fix = inspect_posters(&emby_client, plan.target_lib.as_deref()).await;
    let poster_auto_fix = auto_fix_new_posters(
        &state.pool,
        &emby_client,
        plan.target_lib.as_deref(),
        &strm,
        &poster_before_fix,
    )
    .await;
    let poster = if poster_auto_fix.fixed_count > 0 {
        inspect_posters(&emby_client, plan.target_lib.as_deref()).await
    } else {
        poster_before_fix
    };
    tasks::set_progress(
        &state.pool,
        id,
        (total_items + 2) as i64,
        "海报检测/自动修复阶段完成",
    )
    .await?;

    let emby_groups = collect_emby_tmdb_groups_for_add_new(
        state,
        &emby_client,
        &mut dedup,
        plan.target_lib.as_deref(),
        &strm,
        transfer.ok,
    )
    .await;
    append_emby_duplicate_review(&mut dedup, plan.target_lib.as_deref(), &emby_groups);
    let auto_resolve = auto_resolve_emby_duplicates(
        state,
        &emby_client,
        &mut dedup,
        plan.target_lib.as_deref(),
        &emby_groups,
        &strm,
    )
    .await;

    let check = build_post_add_check(
        &transfer,
        &strm,
        &dedup,
        &auto_resolve,
        &scan,
        &poster,
        &poster_auto_fix,
    );
    tasks::set_progress(
        &state.pool,
        id,
        (total_items + 3) as i64,
        "加新结果检查完成",
    )
    .await?;

    Ok(AddNewReport {
        ok: transfer.ok
            && strm.ok
            && auto_resolve.ok
            && poster_auto_fix.ok
            && scan.ok
            && poster.ok
            && check.item_error_count == 0
            && check.stage_error_count == 0,
        target: AddNewTargetReport {
            cid: plan.target_cid,
            lib: plan.target_lib,
        },
        transfer,
        strm,
        dedup,
        auto_resolve,
        scan,
        poster,
        poster_auto_fix,
        check,
    })
}

async fn execute_transfer_item(
    client: &C115Client,
    item: &AddNewItem,
    index: usize,
    target_cid: &str,
    target_lib: Option<&str>,
) -> AddNewTransferItemReport {
    let action = infer_action(item);
    let result = match action {
        AddNewTransferAction::SaveShare => {
            execute_save_share(client, item, target_cid, target_lib).await
        }
        AddNewTransferAction::OfflineDownload => {
            execute_offline_download(client, item, target_cid, target_lib).await
        }
        AddNewTransferAction::Unsupported => Err("unsupported item link/action".to_string()),
    };

    match result {
        Ok(response) => AddNewTransferItemReport {
            index,
            ok: true,
            action,
            label: item.label.clone(),
            url: item.url.clone(),
            response: Some(response),
            error: None,
        },
        Err(error) => AddNewTransferItemReport {
            index,
            ok: false,
            action,
            label: item.label.clone(),
            url: item.url.clone(),
            response: None,
            error: Some(error),
        },
    }
}

async fn execute_save_share(
    client: &C115Client,
    item: &AddNewItem,
    target_cid: &str,
    target_lib: Option<&str>,
) -> Result<Value, String> {
    let mut response = client
        .save_to_cid(
            C115SaveRequest {
                url: item.url.clone(),
                pwd: item.pwd.clone(),
                lib: target_lib.map(ToString::to_string),
                cid: Some(target_cid.to_string()),
                label: item.label.clone(),
                file_ids: item.file_ids.clone(),
            },
            target_cid.to_string(),
            target_lib.map(ToString::to_string),
        )
        .await
        .map_err(|err| err.to_string())?;
    if response.ok || is_already_received_message(&response.msg) {
        if !response.ok {
            response.ok = true;
            response.msg = format!("{}，继续执行 STRM/去重/Emby 刷新", response.msg);
        }
        Ok(serde_json::to_value(response).unwrap_or_else(|_| json!({})))
    } else {
        Err(response.msg)
    }
}

fn is_already_received_message(message: &str) -> bool {
    let normalized = message.trim();
    normalized.contains("文件已接收") || normalized.contains("无需重复接收")
}

async fn execute_offline_download(
    client: &C115Client,
    item: &AddNewItem,
    target_cid: &str,
    target_lib: Option<&str>,
) -> Result<Value, String> {
    let response = client
        .offline_add(
            C115OfflineRequest {
                url: item.url.clone(),
                lib: target_lib.map(ToString::to_string),
                cid: Some(target_cid.to_string()),
                label: item.label.clone(),
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

async fn trigger_scan(client: &EmbyClient, lib: Option<&str>, delay_ms: u64) -> AddNewScanReport {
    let lib = lib.and_then(non_empty_trimmed).map(ToString::to_string);
    if let Some(lib_name) = lib.clone() {
        match client.libraries().await {
            Ok(libraries) => {
                if let Some(library) = libraries
                    .into_iter()
                    .find(|library| library.name == lib_name)
                    && let Some(item_id) = library.id
                {
                    return match client.refresh_item(&item_id, true, false).await {
                        Ok(code) => AddNewScanReport {
                            ok: (200..300).contains(&code),
                            triggered: true,
                            mode: "library".to_string(),
                            lib,
                            item_id: Some(item_id),
                            code: Some(code),
                            delay_ms,
                            warning: None,
                            error: None,
                        },
                        Err(err) => scan_error("library", lib, Some(item_id), delay_ms, err),
                    };
                }

                match client.refresh_library().await {
                    Ok(code) => AddNewScanReport {
                        ok: (200..300).contains(&code),
                        triggered: true,
                        mode: "global".to_string(),
                        lib,
                        item_id: None,
                        code: Some(code),
                        delay_ms,
                        warning: Some(format!("未找到 Emby 库「{lib_name}」，已触发全局刷新")),
                        error: None,
                    },
                    Err(err) => scan_error("global", lib, None, delay_ms, err),
                }
            }
            Err(err) => scan_error("library", lib, None, delay_ms, err),
        }
    } else {
        match client.refresh_library().await {
            Ok(code) => AddNewScanReport {
                ok: (200..300).contains(&code),
                triggered: true,
                mode: "global".to_string(),
                lib: None,
                item_id: None,
                code: Some(code),
                delay_ms,
                warning: None,
                error: None,
            },
            Err(err) => scan_error("global", None, None, delay_ms, err),
        }
    }
}

fn scan_error(
    mode: &str,
    lib: Option<String>,
    item_id: Option<String>,
    delay_ms: u64,
    err: anyhow::Error,
) -> AddNewScanReport {
    AddNewScanReport {
        ok: false,
        triggered: false,
        mode: mode.to_string(),
        lib,
        item_id,
        code: None,
        delay_ms,
        warning: None,
        error: Some(err.to_string()),
    }
}

async fn inspect_posters(client: &EmbyClient, lib: Option<&str>) -> AddNewPosterReport {
    let lib = lib.and_then(non_empty_trimmed).map(ToString::to_string);
    match posters::detect_mismatched_posters(
        client,
        PosterDetectRequest {
            lib,
            limit: Some(DEFAULT_POSTER_SCAN_LIMIT),
            include_missing_primary: Some(true),
        },
    )
    .await
    {
        Ok(report) => {
            let issue_count = report.total;
            let status = if issue_count > 0 {
                "issues"
            } else if report.truncated {
                "ok_truncated"
            } else {
                "ok"
            };
            AddNewPosterReport {
                ok: true,
                triggered: true,
                status: status.to_string(),
                scanned_libraries: report.scanned_libraries,
                scanned_items: report.scanned_items,
                issue_count,
                missing_primary_count: report.missing_primary_total,
                mismatch_count: report.mismatch_total,
                truncated: report.truncated,
                warnings: report.warnings,
                items: report
                    .items
                    .into_iter()
                    .map(poster_issue_from_signal)
                    .collect(),
                error: None,
            }
        }
        Err(err) => AddNewPosterReport {
            ok: false,
            triggered: false,
            status: "error".to_string(),
            scanned_libraries: 0,
            scanned_items: 0,
            issue_count: 0,
            missing_primary_count: 0,
            mismatch_count: 0,
            truncated: false,
            warnings: Vec::new(),
            items: Vec::new(),
            error: Some(err.to_string()),
        },
    }
}

#[derive(Debug, Clone)]
struct PosterTmdbAlias {
    key: String,
    tmdb: String,
    folder: String,
}

async fn auto_fix_new_posters(
    pool: &sqlx::PgPool,
    client: &EmbyClient,
    target_lib: Option<&str>,
    strm: &AddNewStrmReport,
    poster: &AddNewPosterReport,
) -> AddNewPosterAutoFixReport {
    let Some(target_lib) = target_lib.and_then(non_empty_trimmed) else {
        return AddNewPosterAutoFixReport::not_triggered();
    };
    if poster.items.is_empty() {
        return AddNewPosterAutoFixReport::not_triggered();
    }

    let new_folders = strm
        .new_folders
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let libraries = match client.libraries().await {
        Ok(libraries) => libraries,
        Err(err) => {
            return AddNewPosterAutoFixReport {
                ok: false,
                triggered: true,
                fixed_count: 0,
                skipped_count: 0,
                error_count: 1,
                items: Vec::new(),
                warnings: vec![format!("海报自动修复读取 Emby 库失败: {err}")],
            };
        }
    };
    let Some(library) = libraries.iter().find(|library| library.name == target_lib) else {
        return AddNewPosterAutoFixReport {
            ok: false,
            triggered: true,
            fixed_count: 0,
            skipped_count: 0,
            error_count: 1,
            items: Vec::new(),
            warnings: vec![format!("海报自动修复未找到目标 Emby 库: {target_lib}")],
        };
    };
    let aliases = collect_poster_tmdb_aliases(client, library).await;
    let restrict_to_new_folders = !new_folders.is_empty();

    let mut items = Vec::new();
    for issue in poster
        .items
        .iter()
        .filter(|issue| issue.lib == target_lib && !issue.has_poster)
    {
        let current = match client
            .item(&issue.id, "Path,ProviderIds,Name,ImageTags")
            .await
        {
            Ok(Some(item)) => item,
            Ok(None) => {
                items.push(poster_auto_fix_item(
                    issue,
                    None,
                    "skipped",
                    "Emby 条目已不存在，跳过海报自动修复",
                    None,
                    None,
                ));
                continue;
            }
            Err(err) => {
                items.push(poster_auto_fix_item(
                    issue,
                    None,
                    "error",
                    "读取 Emby 条目失败",
                    None,
                    Some(err.to_string()),
                ));
                continue;
            }
        };
        let folder = current
            .path
            .as_deref()
            .and_then(|path| folder_from_emby_path(path, library))
            .or_else(|| current.name.clone())
            .unwrap_or_else(|| issue.name.clone());
        if restrict_to_new_folders && !new_folders.contains(folder.as_str()) {
            continue;
        }
        let inferred = match infer_poster_tmdb(&current, issue, &folder, &aliases) {
            Some(inferred) => Ok(Some(inferred)),
            None => infer_poster_tmdb_from_remote(client, &current, issue, &folder).await,
        };
        let (tmdb, reason) = match inferred {
            Ok(Some(inferred)) => inferred,
            Ok(None) => {
                items.push(poster_auto_fix_item(
                    issue,
                    None,
                    "skipped",
                    "条目缺少 tmdbid，RemoteSearch 未返回明确且带图的唯一候选",
                    None,
                    None,
                ));
                continue;
            }
            Err(err) => {
                items.push(poster_auto_fix_item(
                    issue,
                    None,
                    "error",
                    "条目缺少 tmdbid，RemoteSearch 查询失败",
                    None,
                    Some(err.to_string()),
                ));
                continue;
            }
        };

        match posters::apply_poster_match(
            pool,
            client,
            PosterApplyRequest {
                id: issue.id.clone(),
                tmdb: tmdb.clone(),
                item_type: issue.item_type.clone(),
                name: Some(issue.name.clone()),
            },
        )
        .await
        {
            Ok(result) => items.push(poster_auto_fix_item(
                issue,
                Some(tmdb),
                "fixed",
                &reason,
                Some(result.poster),
                None,
            )),
            Err(err) => items.push(poster_auto_fix_item(
                issue,
                Some(tmdb),
                "error",
                &reason,
                None,
                Some(err.to_string()),
            )),
        }
    }

    if items.is_empty() {
        return AddNewPosterAutoFixReport::not_triggered();
    }
    let fixed_count = items.iter().filter(|item| item.status == "fixed").count();
    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let error_count = items.iter().filter(|item| item.status == "error").count();
    AddNewPosterAutoFixReport {
        ok: error_count == 0,
        triggered: true,
        fixed_count,
        skipped_count,
        error_count,
        items,
        warnings: Vec::new(),
    }
}

async fn collect_poster_tmdb_aliases(
    client: &EmbyClient,
    library: &EmbyLibrary,
) -> Vec<PosterTmdbAlias> {
    let Some(parent_id) = library.id.as_deref().and_then(non_empty_trimmed) else {
        return Vec::new();
    };
    let Ok(result) = client
        .library_items(parent_id, emby_item_types(library), 30_000)
        .await
    else {
        return Vec::new();
    };
    let mut aliases = Vec::new();
    for item in result.items {
        let Some(tmdb) = item.provider_id("Tmdb") else {
            continue;
        };
        let folder = item
            .path
            .as_deref()
            .and_then(|path| folder_from_emby_path(path, library))
            .or(item.name.clone())
            .unwrap_or_else(|| tmdb.clone());
        for key in poster_match_keys([Some(folder.as_str()), item.name.as_deref()]) {
            aliases.push(PosterTmdbAlias {
                key,
                tmdb: tmdb.clone(),
                folder: folder.clone(),
            });
        }
    }
    aliases
}

fn infer_poster_tmdb(
    item: &EmbyItem,
    issue: &AddNewPosterIssueReport,
    folder: &str,
    aliases: &[PosterTmdbAlias],
) -> Option<(String, String)> {
    if let Some(tmdb) = item.provider_id("Tmdb") {
        return Some((tmdb, "条目已有 TMDb，自动刷新海报".to_string()));
    }
    if let Some(tmdb) = declared_tmdb_id(folder).or_else(|| declared_tmdb_id(&issue.name)) {
        return Some((tmdb, "新目录声明 tmdbid，自动绑定并刷新海报".to_string()));
    }

    let keys = poster_match_keys([Some(folder), Some(issue.name.as_str())]);
    let mut matched = BTreeMap::<String, BTreeSet<String>>::new();
    for alias in aliases {
        if keys.contains(&alias.key) {
            matched
                .entry(alias.tmdb.clone())
                .or_default()
                .insert(alias.folder.clone());
        }
    }
    if matched.len() != 1 {
        return None;
    }
    let (tmdb, folders) = matched.into_iter().next()?;
    let source = folders.into_iter().next().unwrap_or_else(|| tmdb.clone());
    Some((
        tmdb,
        format!("匹配到同库已绑定条目「{source}」，复用其 TMDb 自动刷新海报"),
    ))
}

async fn infer_poster_tmdb_from_remote(
    client: &EmbyClient,
    item: &EmbyItem,
    issue: &AddNewPosterIssueReport,
    folder: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let names = poster_remote_search_names(folder, item.name.as_deref(), &issue.name);
    for search_name in names {
        let candidates = client
            .remote_search(&issue.id, &search_name, &issue.item_type, 8)
            .await?;
        if let Some((tmdb, candidate_name)) =
            pick_remote_poster_candidate(&candidates, &search_name, folder, issue)
        {
            return Ok(Some((
                tmdb,
                format!(
                    "RemoteSearch「{search_name}」匹配到「{candidate_name}」，自动绑定并刷新海报"
                ),
            )));
        }
    }
    Ok(None)
}

fn poster_remote_search_names(
    folder: &str,
    item_name: Option<&str>,
    issue_name: &str,
) -> Vec<String> {
    let mut names = Vec::<String>::new();
    for value in [Some(folder), item_name, Some(issue_name)]
        .into_iter()
        .flatten()
    {
        for candidate in [
            poster_search_title(value),
            poster_search_title_without_year(value),
        ] {
            let candidate = candidate.trim();
            if candidate.chars().count() >= 2
                && !names
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(candidate))
            {
                names.push(candidate.to_string());
            }
        }
    }
    names.truncate(4);
    names
}

fn poster_search_title(value: &str) -> String {
    let value = trim_media_extension(duplicate_suffix_base(value).unwrap_or(value));
    let mut parts = Vec::new();
    for token in value.split(|ch: char| {
        ch.is_whitespace() || matches!(ch, '.' | '_' | '-' | '[' | ']' | '(' | ')' | '{' | '}')
    }) {
        let token = token.trim();
        if token.is_empty() || token.eq_ignore_ascii_case("tmdbid") {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("tmdbid")
            || is_season_release_token(&lower)
            || is_quality_release_token(&lower)
        {
            break;
        }
        parts.push(token);
    }
    parts.join(" ").trim().to_string()
}

fn poster_search_title_without_year(value: &str) -> String {
    poster_search_title(value)
        .split_whitespace()
        .filter(|part| !is_year_token(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_year_token(value: &str) -> bool {
    value.len() == 4
        && value.chars().all(|ch| ch.is_ascii_digit())
        && value
            .parse::<i32>()
            .is_ok_and(|year| (1900..=2100).contains(&year))
}

fn pick_remote_poster_candidate(
    candidates: &[crate::emby::EmbyRemoteSearchCandidate],
    search_name: &str,
    folder: &str,
    issue: &AddNewPosterIssueReport,
) -> Option<(String, String)> {
    let valid = candidates
        .iter()
        .filter_map(|candidate| {
            let tmdb = remote_candidate_tmdb(candidate)?;
            let image = candidate.image_url.as_deref().unwrap_or_default().trim();
            if image.is_empty() {
                return None;
            }
            let name = candidate.name.clone().unwrap_or_else(|| tmdb.clone());
            Some((candidate, tmdb, name))
        })
        .collect::<Vec<_>>();
    if valid.len() == 1 {
        let (_, tmdb, name) = valid.into_iter().next()?;
        return Some((tmdb, name));
    }

    let keys = poster_match_keys([Some(search_name), Some(folder), Some(issue.name.as_str())]);
    let year = declared_year(folder).or_else(|| declared_year(&issue.name));
    let matched = valid
        .into_iter()
        .filter(|(candidate, _tmdb, name)| {
            let key_match = poster_match_key(name)
                .as_ref()
                .is_some_and(|key| keys.contains(key));
            let year_match = year.is_some_and(|year| candidate.production_year == Some(year));
            key_match || year_match
        })
        .collect::<Vec<_>>();
    if matched.len() == 1 {
        let (_, tmdb, name) = matched.into_iter().next()?;
        return Some((tmdb, name));
    }
    None
}

fn remote_candidate_tmdb(candidate: &crate::emby::EmbyRemoteSearchCandidate) -> Option<String> {
    candidate
        .provider_ids
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Tmdb"))
        .and_then(|(_, value)| match value {
            Value::String(value) => {
                let value = value.trim();
                (!value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
                    .then(|| value.to_string())
            }
            Value::Number(value) => Some(value.to_string())
                .filter(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())),
            _ => None,
        })
}

fn declared_year(value: &str) -> Option<i32> {
    let mut digits = String::new();
    for ch in value.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        if is_year_token(&digits) {
            return digits.parse().ok();
        }
        digits.clear();
    }
    None
}

fn poster_auto_fix_item(
    issue: &AddNewPosterIssueReport,
    tmdb: Option<String>,
    status: &str,
    reason: &str,
    poster: Option<bool>,
    error: Option<String>,
) -> AddNewPosterAutoFixItemReport {
    AddNewPosterAutoFixItemReport {
        id: issue.id.clone(),
        name: issue.name.clone(),
        lib: issue.lib.clone(),
        item_type: issue.item_type.clone(),
        tmdb,
        status: status.to_string(),
        reason: reason.to_string(),
        poster,
        error,
    }
}

fn poster_match_keys<const N: usize>(values: [Option<&str>; N]) -> BTreeSet<String> {
    values
        .into_iter()
        .flatten()
        .filter_map(poster_match_key)
        .collect()
}

fn poster_match_key(value: &str) -> Option<String> {
    let mut value = trim_media_extension(value);
    if let Some(base) = duplicate_suffix_base(&value) {
        value = base.to_string();
    }
    let lower = value.to_ascii_lowercase();
    let mut out = String::new();
    let mut last_space = false;
    for token in lower.split(|ch: char| {
        ch.is_whitespace() || matches!(ch, '.' | '_' | '-' | '[' | ']' | '(' | ')' | '{' | '}')
    }) {
        if token.is_empty() {
            continue;
        }
        if is_season_release_token(token) || is_quality_release_token(token) {
            break;
        }
        if token.starts_with("tmdbid") {
            continue;
        }
        if !last_space && !out.is_empty() {
            out.push(' ');
        }
        out.push_str(token);
        last_space = false;
    }
    let compact = out
        .chars()
        .filter(|ch| ch.is_alphanumeric() || is_cjk(*ch))
        .collect::<String>();
    (compact.chars().count() >= 4).then_some(compact)
}

fn is_season_release_token(token: &str) -> bool {
    let Some(rest) = token.strip_prefix('s') else {
        return token.starts_with("season");
    };
    if !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }
    rest.split_once('e').is_some_and(|(season, episode)| {
        !season.is_empty()
            && !episode.is_empty()
            && season.chars().all(|ch| ch.is_ascii_digit())
            && episode.chars().all(|ch| ch.is_ascii_digit())
    })
}

fn is_quality_release_token(token: &str) -> bool {
    matches!(
        token,
        "2160p"
            | "1080p"
            | "720p"
            | "480p"
            | "web"
            | "webdl"
            | "web-dl"
            | "webrip"
            | "bluray"
            | "hdtv"
            | "remux"
            | "h264"
            | "h265"
            | "x264"
            | "x265"
            | "hevc"
            | "dv"
            | "hdr"
            | "ddp"
            | "ddp5"
            | "ddp5.1"
    )
}

fn is_cjk(ch: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
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

fn declared_tmdb_id(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    for marker in ["tmdbid-", "tmdbid_"] {
        let Some(start) = lower.find(marker) else {
            continue;
        };
        let digit_start = start + marker.len();
        let digits = lower[digit_start..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if !digits.is_empty() {
            return Some(digits);
        }
    }
    None
}

fn poster_issue_from_signal(item: posters::PosterSignalItem) -> AddNewPosterIssueReport {
    AddNewPosterIssueReport {
        id: item.id,
        name: item.name,
        lib: item.lib,
        item_type: item.item_type,
        has_poster: item.has_poster,
        score: item.score,
        reasons: item.reasons,
    }
}

async fn generate_target_strm(
    state: &AppState,
    transfer: &AddNewTransferSummary,
    lib: Option<&str>,
) -> AddNewStrmReport {
    let Some(lib) = lib.and_then(non_empty_trimmed).map(ToString::to_string) else {
        return AddNewStrmReport {
            ok: true,
            triggered: false,
            lib: None,
            matched: 0,
            new_count: 0,
            new_folders: BTreeMap::new(),
            attention: Vec::new(),
            retried: false,
            warnings: vec!["未指定目标库，跳过 STRM 生成".to_string()],
            error: None,
        };
    };
    if transfer.succeeded == 0 {
        return AddNewStrmReport {
            ok: true,
            triggered: false,
            lib: Some(lib),
            matched: 0,
            new_count: 0,
            new_folders: BTreeMap::new(),
            attention: Vec::new(),
            retried: false,
            warnings: vec!["没有成功转存/离线项目，跳过 STRM 生成".to_string()],
            error: None,
        };
    }

    let first = media_fs::generate_missing_strm_for_library(state, &lib, None, true);
    let (result, retried) = match first {
        Ok(result) if result.new_count == 0 => {
            sleep(Duration::from_millis(500)).await;
            (
                media_fs::generate_missing_strm_for_library(state, &lib, None, true),
                true,
            )
        }
        other => (other, false),
    };

    match result {
        Ok(result) => strm_report_from_result(result, retried),
        Err(err) => AddNewStrmReport {
            ok: false,
            triggered: true,
            lib: Some(lib),
            matched: 0,
            new_count: 0,
            new_folders: BTreeMap::new(),
            attention: Vec::new(),
            retried,
            warnings: vec!["STRM 生成失败，已保留转存/离线结果".to_string()],
            error: Some(err.to_string()),
        },
    }
}

fn strm_report_from_result(result: StrmGenerateResult, retried: bool) -> AddNewStrmReport {
    let mut warnings = result.attention.clone();
    if result.new_count == 0 {
        warnings.push(if retried {
            "STRM 生成重试后仍为 0 new".to_string()
        } else {
            "STRM 生成结果为 0 new".to_string()
        });
    }
    AddNewStrmReport {
        ok: true,
        triggered: true,
        lib: Some(result.lib),
        matched: result.matched,
        new_count: result.new_count,
        new_folders: result.new_folders,
        attention: result.attention,
        retried,
        warnings,
        error: None,
    }
}

fn inspect_target_dedup(
    state: &AppState,
    transfer: &AddNewTransferSummary,
    lib: Option<&str>,
) -> AddNewDedupReport {
    let lib = lib.and_then(non_empty_trimmed).map(ToString::to_string);
    if transfer.succeeded == 0 {
        return AddNewDedupReport {
            ok: true,
            triggered: false,
            lib,
            dups_count: 0,
            review_count: 0,
            dups: Vec::new(),
            review: Vec::new(),
            warnings: vec!["没有成功转存/离线项目，跳过去重报告".to_string()],
            error: None,
        };
    }

    match dedup::analyze_duplicate_groups(&state.settings.strm_root) {
        Ok(report) => {
            let dups = filter_dedup_groups(report.dups, lib.as_deref());
            let review = filter_review_groups(report.review, lib.as_deref());
            AddNewDedupReport {
                ok: true,
                triggered: true,
                lib,
                dups_count: dups.len(),
                review_count: review.len(),
                dups,
                review,
                warnings: Vec::new(),
                error: None,
            }
        }
        Err(err) => AddNewDedupReport {
            ok: false,
            triggered: true,
            lib,
            dups_count: 0,
            review_count: 0,
            dups: Vec::new(),
            review: Vec::new(),
            warnings: vec!["去重报告生成失败，已保留转存/离线结果".to_string()],
            error: Some(err.to_string()),
        },
    }
}

fn filter_dedup_groups(groups: Vec<DedupGroup>, lib: Option<&str>) -> Vec<DedupGroup> {
    groups
        .into_iter()
        .filter(|group| {
            lib.is_none_or(|lib| {
                group.keep.lib == lib || group.remove.iter().any(|row| row.lib == lib)
            })
        })
        .collect()
}

fn filter_review_groups(groups: Vec<DedupReviewGroup>, lib: Option<&str>) -> Vec<DedupReviewGroup> {
    groups
        .into_iter()
        .filter(|group| lib.is_none_or(|lib| group.rows.iter().any(|row| row.lib == lib)))
        .collect()
}

async fn collect_emby_tmdb_groups_for_add_new(
    state: &AppState,
    client: &EmbyClient,
    dedup: &mut AddNewDedupReport,
    target_lib: Option<&str>,
    strm: &AddNewStrmReport,
    transfer_ok: bool,
) -> BTreeMap<String, Vec<EmbyDuplicateRow>> {
    let mut groups = collect_emby_tmdb_groups(state, client, dedup).await;
    let Some(target_lib) = target_lib.and_then(non_empty_trimmed) else {
        return groups;
    };
    if !should_wait_for_emby_dedup(target_lib, strm, transfer_ok)
        || emby_groups_have_target_new_folder(target_lib, &strm.new_folders, &groups)
        || emby_groups_have_auto_resolve_candidate(target_lib, &groups)
    {
        return groups;
    }

    let mut waited = 0u64;
    while waited < EMBY_DEDUP_SETTLE_TIMEOUT_MS {
        sleep(Duration::from_millis(EMBY_DEDUP_SETTLE_INTERVAL_MS)).await;
        waited += EMBY_DEDUP_SETTLE_INTERVAL_MS;
        groups = collect_emby_tmdb_groups(state, client, dedup).await;
        if emby_groups_have_target_new_folder(target_lib, &strm.new_folders, &groups)
            || emby_groups_have_auto_resolve_candidate(target_lib, &groups)
        {
            break;
        }
    }
    groups
}

fn append_emby_duplicate_review(
    dedup: &mut AddNewDedupReport,
    target_lib: Option<&str>,
    by_tmdb: &BTreeMap<String, Vec<EmbyDuplicateRow>>,
) {
    let Some(target_lib) = target_lib.and_then(non_empty_trimmed) else {
        return;
    };

    for (tmdb, rows) in by_tmdb {
        if rows.len() < 2 || !rows.iter().any(|row| row.lib == target_lib) {
            continue;
        }
        if dedup.dups.iter().any(|group| group.tmdb == *tmdb)
            || dedup.review.iter().any(|group| group.tmdb == *tmdb)
        {
            continue;
        }
        dedup.review.push(DedupReviewGroup {
            tmdb: tmdb.clone(),
            why: "Emby ProviderIds.Tmdb 相同，跨库疑似重复；请人工确认后处理".to_string(),
            rows: rows.iter().map(|row| row.public_row.clone()).collect(),
        });
    }
    dedup.review_count = dedup.review.len();
}

async fn auto_resolve_emby_duplicates(
    state: &AppState,
    client: &EmbyClient,
    dedup: &mut AddNewDedupReport,
    target_lib: Option<&str>,
    groups: &BTreeMap<String, Vec<EmbyDuplicateRow>>,
    strm: &AddNewStrmReport,
) -> AddNewAutoResolveReport {
    let Some(target_lib) = target_lib.and_then(non_empty_trimmed) else {
        return AddNewAutoResolveReport::not_triggered();
    };

    let mut items = Vec::new();
    let mut resolved_tmdb = Vec::<String>::new();
    let mut triggered = false;

    for (tmdb, rows) in groups {
        if rows.len() < 2 {
            continue;
        }
        let target_rows = rows
            .iter()
            .filter(|row| row.lib == target_lib)
            .collect::<Vec<_>>();
        if target_rows.is_empty() {
            continue;
        }
        let Some(kept) = choose_auto_resolve_keep_row(&target_rows, &strm.new_folders) else {
            continue;
        };
        let keep_is_new = strm.new_folders.contains_key(&kept.folder);
        let old_rows = auto_resolve_old_rows(target_lib, kept, rows, keep_is_new);
        if old_rows.is_empty() {
            continue;
        }
        triggered = true;
        let mut unresolved = false;

        for old in old_rows {
            let same_lib = old.lib == kept.lib;
            let reason = auto_resolve_reason(tmdb, kept, old, same_lib);
            if kept.episode_count == 0 {
                unresolved = true;
                items.push(auto_resolve_skipped(
                    tmdb,
                    kept,
                    old,
                    "目标库本地 STRM 数量为 0，跳过自动删除".to_string(),
                ));
                continue;
            }
            if kept.episode_count < old.episode_count {
                unresolved = true;
                items.push(auto_resolve_skipped(
                    tmdb,
                    kept,
                    old,
                    format!(
                        "目标库集数 {} 少于旧追更库集数 {}，跳过自动删除",
                        kept.episode_count, old.episode_count
                    ),
                ));
                continue;
            }
            if same_lib && !keep_is_new && kept.episode_count == old.episode_count {
                unresolved = true;
                items.push(auto_resolve_skipped(
                    tmdb,
                    kept,
                    old,
                    "同库版本集数相同，但本次新目录未明确命中，跳过自动删除".to_string(),
                ));
                continue;
            }
            let Some(item_id) = old.item_id.as_deref().and_then(non_empty_trimmed) else {
                unresolved = true;
                items.push(auto_resolve_skipped(
                    tmdb,
                    kept,
                    old,
                    "旧追更条目缺少 Emby item id，跳过自动删除".to_string(),
                ));
                continue;
            };
            let req = ManageDeleteRequest {
                lib: old.lib.clone(),
                folder: old.folder.clone(),
                item_id: Some(item_id.to_string()),
                reason: Some(reason.clone()),
            };
            match media_fs::execute_delete_direct(state, client, req).await {
                Ok(result) => items.push(AddNewAutoResolveItemReport {
                    tmdb: tmdb.clone(),
                    action: "delete_old_followup".to_string(),
                    status: "resolved".to_string(),
                    kept_lib: kept.lib.clone(),
                    kept_folder: kept.folder.clone(),
                    removed_lib: Some(old.lib.clone()),
                    removed_folder: Some(old.folder.clone()),
                    removed_item_id: old.item_id.clone(),
                    reason,
                    result: Some(result),
                    error: None,
                }),
                Err(err) => {
                    unresolved = true;
                    items.push(AddNewAutoResolveItemReport {
                        tmdb: tmdb.clone(),
                        action: "delete_old_followup".to_string(),
                        status: "error".to_string(),
                        kept_lib: kept.lib.clone(),
                        kept_folder: kept.folder.clone(),
                        removed_lib: Some(old.lib.clone()),
                        removed_folder: Some(old.folder.clone()),
                        removed_item_id: old.item_id.clone(),
                        reason,
                        result: None,
                        error: Some(err.to_string()),
                    });
                }
            }
        }

        if !unresolved
            && items
                .iter()
                .any(|item| item.tmdb == *tmdb && item.status == "resolved")
        {
            resolved_tmdb.push(tmdb.clone());
        }
    }

    if !resolved_tmdb.is_empty() {
        dedup
            .review
            .retain(|group| !resolved_tmdb.iter().any(|tmdb| tmdb == &group.tmdb));
        dedup.review_count = dedup.review.len();
    }

    let resolved_count = items
        .iter()
        .filter(|item| item.status == "resolved")
        .count();
    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let error_count = items.iter().filter(|item| item.status == "error").count();
    AddNewAutoResolveReport {
        ok: error_count == 0,
        triggered,
        resolved_count,
        skipped_count,
        error_count,
        items,
        warnings: Vec::new(),
    }
}

fn should_wait_for_emby_dedup(
    _target_lib: &str,
    strm: &AddNewStrmReport,
    transfer_ok: bool,
) -> bool {
    transfer_ok && strm.ok && strm.new_count > 0 && !strm.new_folders.is_empty()
}

fn emby_groups_have_target_new_folder(
    target_lib: &str,
    new_folders: &BTreeMap<String, usize>,
    groups: &BTreeMap<String, Vec<EmbyDuplicateRow>>,
) -> bool {
    let folders = new_folders
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    !folders.is_empty()
        && groups.values().any(|rows| {
            rows.iter()
                .any(|row| row.lib == target_lib && folders.contains(row.folder.as_str()))
        })
}

fn emby_groups_have_auto_resolve_candidate(
    target_lib: &str,
    groups: &BTreeMap<String, Vec<EmbyDuplicateRow>>,
) -> bool {
    groups.values().any(|rows| {
        let target_count = rows.iter().filter(|row| row.lib == target_lib).count();
        if target_count == 0 {
            return false;
        }
        if is_followup_library(target_lib) && target_count >= 2 {
            return true;
        }
        rows.iter()
            .any(|row| row.lib != target_lib && is_followup_library(&row.lib))
    })
}

fn choose_auto_resolve_keep_row<'a>(
    target_rows: &[&'a EmbyDuplicateRow],
    new_folders: &BTreeMap<String, usize>,
) -> Option<&'a EmbyDuplicateRow> {
    let mut candidates = target_rows
        .iter()
        .copied()
        .filter(|row| new_folders.contains_key(&row.folder))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        candidates = target_rows.to_vec();
    }
    candidates.sort_by(|left, right| compare_auto_resolve_keep_row(left, right, new_folders));
    candidates.into_iter().next()
}

fn compare_auto_resolve_keep_row(
    left: &EmbyDuplicateRow,
    right: &EmbyDuplicateRow,
    new_folders: &BTreeMap<String, usize>,
) -> std::cmp::Ordering {
    let left_new = new_folders.contains_key(&left.folder);
    let right_new = new_folders.contains_key(&right.folder);
    right_new
        .cmp(&left_new)
        .then_with(|| right.episode_count.cmp(&left.episode_count))
        .then_with(|| left.folder.cmp(&right.folder))
}

fn auto_resolve_old_rows<'a>(
    target_lib: &str,
    kept: &EmbyDuplicateRow,
    rows: &'a [EmbyDuplicateRow],
    keep_is_new: bool,
) -> Vec<&'a EmbyDuplicateRow> {
    if is_followup_library(target_lib) {
        return rows
            .iter()
            .filter(|row| row.lib == target_lib)
            .filter(|row| row.folder != kept.folder)
            .filter(|row| keep_is_new || row.episode_count < kept.episode_count)
            .collect();
    }
    rows.iter()
        .filter(|row| row.lib != target_lib && is_followup_library(&row.lib))
        .collect()
}

fn auto_resolve_reason(
    tmdb: &str,
    kept: &EmbyDuplicateRow,
    old: &EmbyDuplicateRow,
    same_lib: bool,
) -> String {
    if same_lib {
        return format!(
            "同 TMDb {tmdb} 在「{}」已更新到「{}」({} 集)，旧目录「{}」({} 集) 自动清理",
            kept.lib, kept.folder, kept.episode_count, old.folder, old.episode_count
        );
    }
    format!(
        "同 TMDb {tmdb} 已入库到「{}」，且目标库集数 {} >= 旧追更库集数 {}，自动清理旧追更版本",
        kept.lib, kept.episode_count, old.episode_count
    )
}

#[derive(Debug, Clone)]
struct EmbyDuplicateRow {
    lib: String,
    folder: String,
    item_id: Option<String>,
    episode_count: usize,
    public_row: DedupRow,
}

async fn collect_emby_tmdb_groups(
    state: &AppState,
    client: &EmbyClient,
    dedup: &mut AddNewDedupReport,
) -> BTreeMap<String, Vec<EmbyDuplicateRow>> {
    let libraries = match client.libraries().await {
        Ok(libraries) => libraries,
        Err(err) => {
            dedup
                .warnings
                .push(format!("Emby ProviderIds 去重扫描失败: {err}"));
            return BTreeMap::new();
        }
    };
    let mut by_tmdb: BTreeMap<String, Vec<EmbyDuplicateRow>> = BTreeMap::new();
    for library in libraries {
        let Some(parent_id) = library.id.as_deref() else {
            continue;
        };
        let item_types = emby_item_types(&library);
        let result = match client.library_items(parent_id, item_types, 30_000).await {
            Ok(result) => result,
            Err(err) => {
                dedup.warnings.push(format!(
                    "Emby ProviderIds 去重扫描库 {} 失败: {err}",
                    library.name
                ));
                continue;
            }
        };
        if result.truncated {
            dedup.warnings.push(format!(
                "Emby ProviderIds 去重扫描库 {} 超过 30000 项，结果可能截断",
                library.name
            ));
        }
        for item in result.items {
            let Some(tmdb) = item.provider_id("Tmdb") else {
                continue;
            };
            let folder = item
                .path
                .as_deref()
                .and_then(|path| folder_from_emby_path(path, &library))
                .or(item.name.clone())
                .unwrap_or_else(|| item.id.clone().unwrap_or_else(|| tmdb.clone()));
            let item_id = item.id.clone();
            let episode_count = count_strm_files(state, &library.name, &folder);
            let public_row = DedupRow {
                lib: library.name.clone(),
                folder: folder.clone(),
                score: 0,
                n: episode_count,
                item_id: item_id.clone(),
            };
            by_tmdb
                .entry(tmdb.clone())
                .or_default()
                .push(EmbyDuplicateRow {
                    lib: library.name.clone(),
                    folder,
                    item_id,
                    episode_count,
                    public_row,
                });
        }
    }
    by_tmdb
}

fn auto_resolve_skipped(
    tmdb: &str,
    kept: &EmbyDuplicateRow,
    old: &EmbyDuplicateRow,
    reason: String,
) -> AddNewAutoResolveItemReport {
    AddNewAutoResolveItemReport {
        tmdb: tmdb.to_string(),
        action: "delete_old_followup".to_string(),
        status: "skipped".to_string(),
        kept_lib: kept.lib.clone(),
        kept_folder: kept.folder.clone(),
        removed_lib: Some(old.lib.clone()),
        removed_folder: Some(old.folder.clone()),
        removed_item_id: old.item_id.clone(),
        reason,
        result: None,
        error: None,
    }
}

fn is_followup_library(lib: &str) -> bool {
    lib.contains("追更")
}

fn count_strm_files(state: &AppState, lib: &str, folder: &str) -> usize {
    let Some(path) = safe_count_path(&state.settings.strm_root, lib, folder) else {
        return 0;
    };
    if !path.exists() {
        return 0;
    }
    walkdir::WalkDir::new(path)
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

fn safe_count_path(root: &std::path::Path, lib: &str, folder: &str) -> Option<std::path::PathBuf> {
    let lib = non_empty_trimmed(lib)?;
    let folder = non_empty_trimmed(folder)?;
    if lib.contains('/') || lib.contains('\\') || folder.contains('/') || folder.contains('\\') {
        return None;
    }
    if lib.contains("..") || folder.contains("..") {
        return None;
    }
    Some(root.join(lib).join(folder))
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

fn build_post_add_check(
    transfer: &AddNewTransferSummary,
    strm: &AddNewStrmReport,
    dedup: &AddNewDedupReport,
    auto_resolve: &AddNewAutoResolveReport,
    scan: &AddNewScanReport,
    poster: &AddNewPosterReport,
    poster_auto_fix: &AddNewPosterAutoFixReport,
) -> AddNewCheckReport {
    let mut items = Vec::with_capacity(transfer.items.len());
    let mut errors = Vec::new();
    let mut suspicious = Vec::new();

    for item in &transfer.items {
        let message = if item.ok {
            "转存/离线请求成功".to_string()
        } else {
            item.error
                .clone()
                .unwrap_or_else(|| "转存/离线请求失败".to_string())
        };
        if !item.ok {
            errors.push(AddNewCheckErrorReport {
                stage: "transfer".to_string(),
                index: Some(item.index),
                label: item.label.clone(),
                message: message.clone(),
            });
        }
        items.push(AddNewCheckItemReport {
            index: item.index,
            ok: item.ok,
            action: item.action,
            label: item.label.clone(),
            url: item.url.clone(),
            status: if item.ok { "ok" } else { "error" }.to_string(),
            message,
        });
    }

    let mut stage_error_count = 0usize;
    if !scan.ok {
        stage_error_count += 1;
        errors.push(AddNewCheckErrorReport {
            stage: "scan".to_string(),
            index: None,
            label: scan.lib.clone(),
            message: scan
                .error
                .clone()
                .unwrap_or_else(|| "Emby 刷新未成功触发".to_string()),
        });
    }
    if !poster.ok {
        stage_error_count += 1;
        errors.push(AddNewCheckErrorReport {
            stage: "poster".to_string(),
            index: None,
            label: None,
            message: poster
                .error
                .clone()
                .unwrap_or_else(|| "海报检测失败".to_string()),
        });
    }
    if !poster_auto_fix.ok && poster_auto_fix.items.is_empty() {
        stage_error_count += 1;
        errors.push(AddNewCheckErrorReport {
            stage: "poster_auto_fix".to_string(),
            index: None,
            label: None,
            message: poster_auto_fix
                .warnings
                .first()
                .cloned()
                .unwrap_or_else(|| "海报自动修复失败".to_string()),
        });
    }

    if !strm.ok {
        stage_error_count += 1;
        errors.push(AddNewCheckErrorReport {
            stage: "strm".to_string(),
            index: None,
            label: strm.lib.clone(),
            message: strm
                .error
                .clone()
                .unwrap_or_else(|| "STRM 生成失败".to_string()),
        });
    }
    for warning in &strm.warnings {
        suspicious.push(AddNewCheckSuspiciousReport {
            stage: "strm".to_string(),
            severity: "warn".to_string(),
            id: None,
            label: strm.lib.clone().unwrap_or_else(|| "strm".to_string()),
            message: warning.clone(),
        });
    }
    if !dedup.ok {
        suspicious.push(AddNewCheckSuspiciousReport {
            stage: "dedup".to_string(),
            severity: "warn".to_string(),
            id: None,
            label: dedup.lib.clone().unwrap_or_else(|| "dedup".to_string()),
            message: dedup
                .error
                .clone()
                .unwrap_or_else(|| "去重报告生成失败".to_string()),
        });
    }
    for warning in &dedup.warnings {
        suspicious.push(AddNewCheckSuspiciousReport {
            stage: "dedup".to_string(),
            severity: "warn".to_string(),
            id: None,
            label: dedup.lib.clone().unwrap_or_else(|| "dedup".to_string()),
            message: warning.clone(),
        });
    }
    for group in &dedup.dups {
        suspicious.push(AddNewCheckSuspiciousReport {
            stage: "dedup".to_string(),
            severity: "warn".to_string(),
            id: Some(group.tmdb.clone()),
            label: group.keep.folder.clone(),
            message: format!("发现可自动处理重复项 {} 组", group.remove.len()),
        });
    }
    for group in &dedup.review {
        suspicious.push(AddNewCheckSuspiciousReport {
            stage: "dedup".to_string(),
            severity: "warn".to_string(),
            id: Some(group.tmdb.clone()),
            label: group
                .rows
                .first()
                .map(|row| row.folder.clone())
                .unwrap_or_else(|| group.tmdb.clone()),
            message: group.why.clone(),
        });
    }
    for item in &auto_resolve.items {
        match item.status.as_str() {
            "resolved" => {
                items.push(AddNewCheckItemReport {
                    index: items.len(),
                    ok: true,
                    action: AddNewTransferAction::Unsupported,
                    label: Some(format!(
                        "自动清理 {} / {}",
                        item.removed_lib
                            .clone()
                            .unwrap_or_else(|| "旧库".to_string()),
                        item.removed_folder
                            .clone()
                            .unwrap_or_else(|| item.tmdb.clone())
                    )),
                    url: format!("tmdb:{}", item.tmdb),
                    status: "ok".to_string(),
                    message: item.reason.clone(),
                });
            }
            "skipped" => {
                suspicious.push(AddNewCheckSuspiciousReport {
                    stage: "auto_resolve".to_string(),
                    severity: "warn".to_string(),
                    id: Some(item.tmdb.clone()),
                    label: item
                        .removed_folder
                        .clone()
                        .unwrap_or_else(|| item.kept_folder.clone()),
                    message: item.reason.clone(),
                });
            }
            "error" => {
                stage_error_count += 1;
                errors.push(AddNewCheckErrorReport {
                    stage: "auto_resolve".to_string(),
                    index: None,
                    label: item.removed_folder.clone(),
                    message: item.error.clone().unwrap_or_else(|| item.reason.clone()),
                });
            }
            _ => {}
        }
    }
    for warning in &auto_resolve.warnings {
        suspicious.push(AddNewCheckSuspiciousReport {
            stage: "auto_resolve".to_string(),
            severity: "warn".to_string(),
            id: None,
            label: "auto-resolve".to_string(),
            message: warning.clone(),
        });
    }

    if let Some(warning) = scan.warning.as_deref() {
        suspicious.push(AddNewCheckSuspiciousReport {
            stage: "scan".to_string(),
            severity: "warn".to_string(),
            id: scan.item_id.clone(),
            label: scan.lib.clone().unwrap_or_else(|| scan.mode.clone()),
            message: warning.to_string(),
        });
    }
    for warning in &poster.warnings {
        suspicious.push(AddNewCheckSuspiciousReport {
            stage: "poster".to_string(),
            severity: "warn".to_string(),
            id: None,
            label: "poster-detect".to_string(),
            message: warning.clone(),
        });
    }
    for item in &poster_auto_fix.items {
        match item.status.as_str() {
            "fixed" => {
                items.push(AddNewCheckItemReport {
                    index: items.len(),
                    ok: true,
                    action: AddNewTransferAction::Unsupported,
                    label: Some(format!("自动修复海报 {}", item.name)),
                    url: item
                        .tmdb
                        .as_deref()
                        .map(|tmdb| format!("tmdb:{tmdb}"))
                        .unwrap_or_else(|| item.id.clone()),
                    status: "ok".to_string(),
                    message: item.reason.clone(),
                });
            }
            "skipped" => {
                suspicious.push(AddNewCheckSuspiciousReport {
                    stage: "poster_auto_fix".to_string(),
                    severity: "warn".to_string(),
                    id: Some(item.id.clone()),
                    label: item.name.clone(),
                    message: item.reason.clone(),
                });
            }
            "error" => {
                errors.push(AddNewCheckErrorReport {
                    stage: "poster_auto_fix".to_string(),
                    index: None,
                    label: Some(item.name.clone()),
                    message: item.error.clone().unwrap_or_else(|| item.reason.clone()),
                });
                stage_error_count += 1;
            }
            _ => {}
        }
    }
    for item in &poster.items {
        suspicious.push(AddNewCheckSuspiciousReport {
            stage: "poster".to_string(),
            severity: if item.score >= 100 { "danger" } else { "warn" }.to_string(),
            id: Some(item.id.clone()),
            label: if item.name.trim().is_empty() {
                item.id.clone()
            } else {
                item.name.clone()
            },
            message: if item.reasons.is_empty() {
                "海报检测发现可疑项".to_string()
            } else {
                item.reasons.join("; ")
            },
        });
    }

    let item_success_count = transfer.succeeded;
    let item_error_count = transfer.failed;
    let error_count = item_error_count + stage_error_count;
    let suspicious_count = suspicious.len();
    let status = if error_count > 0 {
        "errors"
    } else if suspicious_count > 0 {
        "suspicious"
    } else {
        "ok"
    };
    let message = format!(
        "检查完成: {item_success_count} 项成功, {item_error_count} 项失败, {stage_error_count} 个阶段错误, {suspicious_count} 个可疑项"
    );

    AddNewCheckReport {
        ok: error_count == 0,
        status: status.to_string(),
        item_success_count,
        item_error_count,
        stage_error_count,
        suspicious_count,
        items,
        errors,
        suspicious,
        message,
    }
}

fn validate_add_new_request(req: &AddNewRequest) -> AppResult<()> {
    if req.items.is_empty() {
        return Err(AppError::BadRequest(
            "add-new requires at least one item".to_string(),
        ));
    }
    Ok(())
}

async fn resolve_target_cid(
    pool: &sqlx::PgPool,
    req: &AddNewRequest,
) -> AppResult<(String, Option<String>)> {
    let target = merged_target(req);
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
    let map = cid_map(pool).await?;
    let cid = map
        .get(lib)
        .ok_or_else(|| AppError::BadRequest(format!("库「{lib}」没配 115 cid,去设置页填")))?;
    Ok((c115::validate_target_cid(cid)?, Some(lib.to_string())))
}

fn merged_target(req: &AddNewRequest) -> AddNewTarget {
    AddNewTarget {
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

async fn cid_map(pool: &sqlx::PgPool) -> AppResult<BTreeMap<String, String>> {
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

fn infer_action(item: &AddNewItem) -> AddNewTransferAction {
    let kind = item
        .kind
        .as_deref()
        .and_then(non_empty_trimmed)
        .map(|value| value.to_ascii_lowercase().replace(['-', ' '], "_"));
    match kind.as_deref() {
        Some("save") | Some("share") | Some("share115") | Some("save_share")
        | Some("c115_save") => return AddNewTransferAction::SaveShare,
        Some("offline")
        | Some("offline_download")
        | Some("c115_offline")
        | Some("magnet")
        | Some("ed2k") => return AddNewTransferAction::OfflineDownload,
        _ => {}
    }

    let url = item.url.trim();
    if url.starts_with("magnet:") || url.starts_with("ed2k://") {
        return AddNewTransferAction::OfflineDownload;
    }
    if c115::parse_115_url(url, item.pwd.as_deref()).0.is_some() {
        return AddNewTransferAction::SaveShare;
    }
    AddNewTransferAction::Unsupported
}

async fn cancel_if_requested(state: &AppState, id: Uuid) -> AppResult<bool> {
    if tasks::cancel_requested(&state.pool, id).await {
        tasks::finish_cancelled(&state.pool, id).await?;
        return Ok(true);
    }
    Ok(false)
}

fn add_new_task_label(count: usize, cid: &str, lib: Option<&str>) -> String {
    let target = lib
        .and_then(non_empty_trimmed)
        .map(|lib| format!("库「{lib}」"))
        .unwrap_or_else(|| format!("cid={cid}"));
    format!("一条龙加新资源: {count} 项 -> {target}")
}

fn item_label(item: &AddNewItem) -> String {
    first_clean([item.label.clone(), Some(item.url.clone())]).unwrap_or_else(|| "item".to_string())
}

fn first_clean<const N: usize>(values: [Option<String>; N]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find_map(|value| non_empty_trimmed(&value).map(ToString::to_string))
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn truncate(value: &str, limit: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::{
        AddNewAutoResolveReport, AddNewDedupReport, AddNewPosterAutoFixReport,
        AddNewPosterIssueReport, AddNewPosterReport, AddNewScanReport, AddNewStrmReport,
        AddNewTransferAction, AddNewTransferItemReport, AddNewTransferSummary, EmbyDuplicateRow,
        PosterTmdbAlias, auto_resolve_old_rows, build_post_add_check, choose_auto_resolve_keep_row,
        infer_poster_tmdb, is_already_received_message, poster_match_key,
    };
    use crate::dedup::DedupRow;
    use crate::emby::EmbyItem;
    use std::collections::BTreeMap;

    #[test]
    fn treats_115_already_received_as_idempotent_success() {
        assert!(is_already_received_message("文件已接收，无需重复接收！"));
        assert!(is_already_received_message("无需重复接收"));
        assert!(!is_already_received_message("转存失败"));
    }

    #[test]
    fn poster_match_key_groups_release_variants() {
        assert_eq!(
            poster_match_key("The.First.Jasmine.2026.S01.2160p.WEB-DL.DV.H.265.DDP5.1-HiveWeb"),
            Some("thefirstjasmine2026".to_string())
        );
        assert_eq!(
            poster_match_key("The.First.Jasmine.2026.S01.2160p.WEB-DL.H.265.DDP-Pure@HiveWeb(1)"),
            Some("thefirstjasmine2026".to_string())
        );
    }

    #[test]
    fn infers_poster_tmdb_from_same_library_alias() {
        let issue = AddNewPosterIssueReport {
            id: "new-series".to_string(),
            name: "The.First.Jasmine.2026".to_string(),
            lib: "2026完结剧集".to_string(),
            item_type: "Series".to_string(),
            has_poster: false,
            score: 40,
            reasons: vec!["没有 Primary poster".to_string()],
        };
        let item = EmbyItem {
            id: Some("new-series".to_string()),
            name: Some("The.First.Jasmine.2026".to_string()),
            item_type: Some("Series".to_string()),
            path: Some("/strm/2026完结剧集/The.First.Jasmine.2026.S01.2160p.WEB-DL.DV.H.265.DDP5.1-HiveWeb".to_string()),
            production_year: Some(2026),
            image_tags: BTreeMap::new(),
            provider_ids: BTreeMap::new(),
        };
        let aliases = vec![PosterTmdbAlias {
            key: "thefirstjasmine2026".to_string(),
            tmdb: "292696".to_string(),
            folder: "The.First.Jasmine.2026.S01.2160p.WEB-DL.H.265.DDP-Pure@HiveWeb".to_string(),
        }];

        let inferred = infer_poster_tmdb(
            &item,
            &issue,
            "The.First.Jasmine.2026.S01.2160p.WEB-DL.DV.H.265.DDP5.1-HiveWeb",
            &aliases,
        );

        assert_eq!(inferred.map(|(tmdb, _)| tmdb), Some("292696".to_string()));
    }

    #[test]
    fn post_add_check_keeps_suspicious_only_as_success_with_warning() {
        let transfer = ok_transfer();
        let mut strm = ok_strm();
        strm.warnings
            .push("STRM 生成结果为 0 new，请确认是否已有文件".to_string());
        let dedup = ok_dedup();
        let auto_resolve = AddNewAutoResolveReport::not_triggered();
        let scan = ok_scan();
        let poster = ok_poster();
        let poster_auto_fix = AddNewPosterAutoFixReport::not_triggered();

        let check = build_post_add_check(
            &transfer,
            &strm,
            &dedup,
            &auto_resolve,
            &scan,
            &poster,
            &poster_auto_fix,
        );

        assert!(check.ok);
        assert_eq!(check.status, "suspicious");
        assert_eq!(check.item_error_count, 0);
        assert_eq!(check.stage_error_count, 0);
        assert_eq!(check.suspicious_count, 1);
    }

    #[test]
    fn post_add_check_treats_strm_failure_as_stage_error() {
        let transfer = ok_transfer();
        let mut strm = ok_strm();
        strm.ok = false;
        strm.error = Some("permission denied".to_string());
        let dedup = ok_dedup();
        let auto_resolve = AddNewAutoResolveReport::not_triggered();
        let scan = ok_scan();
        let poster = ok_poster();
        let poster_auto_fix = AddNewPosterAutoFixReport::not_triggered();

        let check = build_post_add_check(
            &transfer,
            &strm,
            &dedup,
            &auto_resolve,
            &scan,
            &poster,
            &poster_auto_fix,
        );

        assert!(!check.ok);
        assert_eq!(check.status, "errors");
        assert_eq!(check.item_error_count, 0);
        assert_eq!(check.stage_error_count, 1);
        assert_eq!(check.errors[0].stage, "strm");
    }

    #[test]
    fn followup_update_prefers_new_folder_and_selects_same_library_old_versions() {
        let rows = vec![
            duplicate_row("电视剧追更", "旧版 [tmdbid-100]", "old-id", 24),
            duplicate_row("电视剧追更", "新版 {tmdb-100}", "new-id", 30),
            duplicate_row("2026完结剧集", "完结版 [tmdbid-100]", "archive-id", 30),
        ];
        let target_rows = rows
            .iter()
            .filter(|row| row.lib == "电视剧追更")
            .collect::<Vec<_>>();
        let mut new_folders = BTreeMap::new();
        new_folders.insert("新版 {tmdb-100}".to_string(), 30usize);

        let kept = choose_auto_resolve_keep_row(&target_rows, &new_folders).unwrap();
        assert_eq!(kept.folder, "新版 {tmdb-100}");
        let old = auto_resolve_old_rows("电视剧追更", kept, &rows, true);
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].folder, "旧版 [tmdbid-100]");
    }

    #[test]
    fn followup_update_does_not_delete_equal_count_without_new_folder_signal() {
        let rows = vec![
            duplicate_row("电视剧追更", "A [tmdbid-100]", "a-id", 30),
            duplicate_row("电视剧追更", "B [tmdbid-100]", "b-id", 30),
        ];
        let target_rows = rows.iter().collect::<Vec<_>>();
        let new_folders = BTreeMap::new();

        let kept = choose_auto_resolve_keep_row(&target_rows, &new_folders).unwrap();
        let old = auto_resolve_old_rows("电视剧追更", kept, &rows, false);
        assert!(old.is_empty());
    }

    fn duplicate_row(
        lib: &str,
        folder: &str,
        item_id: &str,
        episode_count: usize,
    ) -> EmbyDuplicateRow {
        EmbyDuplicateRow {
            lib: lib.to_string(),
            folder: folder.to_string(),
            item_id: Some(item_id.to_string()),
            episode_count,
            public_row: DedupRow {
                lib: lib.to_string(),
                folder: folder.to_string(),
                score: 0,
                n: episode_count,
                item_id: Some(item_id.to_string()),
            },
        }
    }

    fn ok_transfer() -> AddNewTransferSummary {
        AddNewTransferSummary {
            ok: true,
            total: 1,
            succeeded: 1,
            failed: 0,
            items: vec![AddNewTransferItemReport {
                index: 0,
                ok: true,
                action: AddNewTransferAction::SaveShare,
                label: Some("示例剧 S01E01".to_string()),
                url: "https://115.com/s/example".to_string(),
                response: None,
                error: None,
            }],
        }
    }

    fn ok_strm() -> AddNewStrmReport {
        AddNewStrmReport {
            ok: true,
            triggered: true,
            lib: Some("电视剧追更".to_string()),
            matched: 1,
            new_count: 1,
            new_folders: BTreeMap::new(),
            attention: Vec::new(),
            retried: false,
            warnings: Vec::new(),
            error: None,
        }
    }

    fn ok_dedup() -> AddNewDedupReport {
        AddNewDedupReport {
            ok: true,
            triggered: true,
            lib: Some("电视剧追更".to_string()),
            dups_count: 0,
            review_count: 0,
            dups: Vec::new(),
            review: Vec::new(),
            warnings: Vec::new(),
            error: None,
        }
    }

    fn ok_scan() -> AddNewScanReport {
        AddNewScanReport {
            ok: true,
            triggered: true,
            mode: "library".to_string(),
            lib: Some("电视剧追更".to_string()),
            item_id: Some("lib-tv".to_string()),
            code: Some(204),
            delay_ms: 0,
            warning: None,
            error: None,
        }
    }

    fn ok_poster() -> AddNewPosterReport {
        AddNewPosterReport {
            ok: true,
            triggered: true,
            status: "ok".to_string(),
            scanned_libraries: 1,
            scanned_items: 1,
            issue_count: 0,
            missing_primary_count: 0,
            mismatch_count: 0,
            truncated: false,
            warnings: Vec::new(),
            items: Vec::new(),
            error: None,
        }
    }
}
