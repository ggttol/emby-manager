use crate::{
    error::{AppError, AppResult},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CatalogTransferPlanItem {
    pub name: Option<String>,
    pub sheet: Option<String>,
    pub link: String,
    pub is_pkg: Option<bool>,
    pub link_type: Option<String>,
    pub share: Option<String>,
    pub rc: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
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

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CatalogTransferAction {
    SaveShare,
    OfflineDownload,
    Unsupported,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogTransferTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lib: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
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

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogC115SavePlan {
    pub endpoint: String,
    pub method: String,
    pub share: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receive_code: Option<String>,
    pub payload: CatalogC115SavePayload,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogC115OfflinePayload {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lib: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogC115OfflinePlan {
    pub endpoint: String,
    pub method: String,
    pub protocol: String,
    pub payload: CatalogC115OfflinePayload,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct CatalogUnsupportedPlan {
    pub reason: String,
    pub link: String,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/catalog/stats", get(catalog_stats))
        .route("/api/v2/catalog/search", get(catalog_search))
        .route("/api/v2/catalog/duplicates", get(catalog_duplicates))
        .route("/api/v2/catalog/transfer-plan", post(catalog_transfer_plan))
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
