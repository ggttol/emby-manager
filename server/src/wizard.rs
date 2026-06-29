use crate::{
    c115::{self, C115Client, C115OfflineRequest, C115SaveRequest},
    config_store,
    dedup::{self, DedupGroup, DedupReviewGroup},
    emby::EmbyClient,
    error::{AppError, AppResult},
    media_fs::{self, StrmGenerateResult},
    posters::{self, PosterDetectRequest},
    state::AppState,
    tasks::{self, TaskRun},
};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};
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
    pub scan: AddNewScanReport,
    pub poster: AddNewPosterReport,
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
    let label = add_new_task_label(req.items.len(), &target_cid, target_lib.as_deref());
    let params = serde_json::to_value(&req).unwrap_or_else(|_| json!({}));
    let task =
        tasks::insert_task_with_meta(&state.pool, "add_new", &label, total, "manual", params)
            .await?;

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
    Ok(Json(task))
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
                } else if !report.scan.ok {
                    "完成，扫描触发失败".to_string()
                } else if !report.poster.ok {
                    "完成，海报检测失败".to_string()
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

    let dedup = inspect_target_dedup(state, &transfer, plan.target_lib.as_deref());

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

    let poster = inspect_posters(&emby_client, plan.target_lib.as_deref()).await;
    tasks::set_progress(
        &state.pool,
        id,
        (total_items + 2) as i64,
        "海报检测阶段完成",
    )
    .await?;

    let check = build_post_add_check(&transfer, &strm, &dedup, &scan, &poster);
    tasks::set_progress(
        &state.pool,
        id,
        (total_items + 3) as i64,
        "加新结果检查完成",
    )
    .await?;

    Ok(AddNewReport {
        ok: transfer.ok && scan.ok && poster.ok && check.ok,
        target: AddNewTargetReport {
            cid: plan.target_cid,
            lib: plan.target_lib,
        },
        transfer,
        strm,
        dedup,
        scan,
        poster,
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
    let response = client
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
    if response.ok {
        Ok(serde_json::to_value(response).unwrap_or_else(|_| json!({})))
    } else {
        Err(response.msg)
    }
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

fn build_post_add_check(
    transfer: &AddNewTransferSummary,
    strm: &AddNewStrmReport,
    dedup: &AddNewDedupReport,
    scan: &AddNewScanReport,
    poster: &AddNewPosterReport,
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

    if !strm.ok {
        suspicious.push(AddNewCheckSuspiciousReport {
            stage: "strm".to_string(),
            severity: "warn".to_string(),
            id: None,
            label: strm.lib.clone().unwrap_or_else(|| "strm".to_string()),
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
        ok: error_count == 0 && suspicious_count == 0,
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
