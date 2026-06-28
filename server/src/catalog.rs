use crate::{
    c115::{self, C115Client, C115OfflineRequest, C115SaveRequest},
    config_store,
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
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

const C115_COOKIE_KEY: &str = "c115_cookie";
const C115_CID_MAP_KEY: &str = "c115_cid_map";
const C115_API_BASE_URL_KEY: &str = "c115_api_base_url";
const C115_SITE_BASE_URL_KEY: &str = "c115_site_base_url";

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
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CatalogSearchResponse {
    pub items: Vec<CatalogItem>,
    pub total: usize,
    pub truncated: bool,
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
            }
        })
        .collect::<Vec<_>>();
    Ok(Json(CatalogSearchResponse {
        total: items.len(),
        items,
        truncated,
    }))
}

#[utoipa::path(post, path = "/api/v2/catalog/transfer-plan", tag = "catalog", request_body = CatalogTransferPlanRequest, responses((status = 200, body = CatalogTransferPlanResponse)))]
pub async fn catalog_transfer_plan(
    Json(req): Json<CatalogTransferPlanRequest>,
) -> AppResult<Json<CatalogTransferPlanResponse>> {
    Ok(Json(build_transfer_plan(req)?))
}

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
        state, task.id, cookie, api_base, site_base, plans, target_cid, target_lib,
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

fn spawn_catalog_transfer_execute(
    state: AppState,
    id: Uuid,
    cookie: String,
    api_base: String,
    site_base: String,
    plans: Vec<CatalogTransferPlanResponse>,
    target_cid: String,
    target_lib: Option<String>,
) {
    tokio::spawn(async move {
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

    #[test]
    fn parses_115_share_and_receive_code() {
        let (share, rc) = parse_share("https://115.com/s/abc123?password=xy9z#anchor");
        assert_eq!(share.as_deref(), Some("abc123"));
        assert_eq!(rc.as_deref(), Some("xy9z"));
        assert_eq!(infer_type(" magnet:?xt=urn:btih:abc"), "magnet");
        assert_eq!(infer_type("https://anxia.com/s/swabc"), "share115");
    }
}
