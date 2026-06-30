use crate::{
    config_store, dedup,
    emby::{EmbyClient, EmbyLibrary},
    error::{AppResult, redact_sensitive_text},
    posters::{self, PosterDetectRequest},
    state::AppState,
    zhuigeng,
};
use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use std::collections::BTreeMap;

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DashboardTodoResponse {
    pub noposter: usize,
    pub no_rating: usize,
    pub dups_auto: usize,
    pub dups_review: usize,
    pub airing_count: usize,
    pub airing_low_count: usize,
    pub noposter_by_lib: BTreeMap<String, usize>,
    pub no_rating_by_lib: BTreeMap<String, usize>,
    pub noposter_err: Option<String>,
    pub no_rating_err: Option<String>,
    pub dups_err: Option<String>,
    pub airing_err: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DashboardSmartActionsResponse {
    pub ok: bool,
    pub total: usize,
    pub actions: Vec<DashboardSmartAction>,
    pub warnings: Vec<String>,
    pub todo: DashboardTodoResponse,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DashboardSmartAction {
    pub severity: String,
    pub area: String,
    pub title: String,
    pub message: String,
    pub count: i64,
    pub tab: String,
    pub action: String,
    pub source: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/dashboard/todo", get(dashboard_todo))
        .route(
            "/api/v2/dashboard/smart-actions",
            get(dashboard_smart_actions),
        )
}

#[utoipa::path(get, path = "/api/v2/dashboard/todo", tag = "dashboard", responses((status = 200, body = DashboardTodoResponse)))]
pub async fn dashboard_todo(
    State(state): State<AppState>,
) -> AppResult<Json<DashboardTodoResponse>> {
    Ok(Json(dashboard_todo_for_state(&state, true).await))
}

async fn dashboard_todo_for_state(state: &AppState, include_airing: bool) -> DashboardTodoResponse {
    let mut response = DashboardTodoResponse {
        noposter: 0,
        no_rating: 0,
        dups_auto: 0,
        dups_review: 0,
        airing_count: 0,
        airing_low_count: 0,
        noposter_by_lib: BTreeMap::new(),
        no_rating_by_lib: BTreeMap::new(),
        noposter_err: None,
        no_rating_err: None,
        dups_err: None,
        airing_err: None,
    };

    match no_poster_todo(state).await {
        Ok((total, by_lib)) => {
            response.noposter = total;
            response.noposter_by_lib = by_lib;
        }
        Err(err) => response.noposter_err = Some(redact_sensitive_text(&err.to_string())),
    }

    match no_rating_todo(state).await {
        Ok((total, by_lib)) => {
            response.no_rating = total;
            response.no_rating_by_lib = by_lib;
        }
        Err(err) => response.no_rating_err = Some(redact_sensitive_text(&err.to_string())),
    }

    match dedup::analyze_duplicate_groups(&state.settings.strm_root) {
        Ok(dups) => {
            response.dups_auto = dups.dups.len();
            response.dups_review = dups.review.len();
        }
        Err(err) => response.dups_err = Some(redact_sensitive_text(&err.to_string())),
    }

    if include_airing {
        match zhuigeng::status(State(state.clone())).await {
            Ok(Json(status)) => {
                response.airing_count = status.continuing;
                response.airing_low_count = status.continuing;
            }
            Err(err) => {
                let message = err.to_string();
                if !is_optional_tmdb_config_error(&message) {
                    response.airing_err = Some(redact_sensitive_text(&message));
                }
            }
        }
    }

    response
}

#[utoipa::path(get, path = "/api/v2/dashboard/smart-actions", tag = "dashboard", responses((status = 200, body = DashboardSmartActionsResponse)))]
pub async fn dashboard_smart_actions(
    State(state): State<AppState>,
) -> AppResult<Json<DashboardSmartActionsResponse>> {
    let mut todo = dashboard_todo_for_state(&state, false).await;
    let mut actions = Vec::new();
    let mut warnings = [
        todo.noposter_err.clone(),
        todo.no_rating_err.clone(),
        todo.dups_err.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    push_dashboard_action(
        &mut actions,
        todo.noposter as i64,
        "high",
        "posters",
        "修复无海报条目",
        &format!(
            "{} 个条目缺少主海报，优先进入海报修复；一条龙转存后的新资源也会复用这里的自动修复能力。",
            todo.noposter
        ),
        "posters",
        "打开海报修复",
        "dashboard_todo.noposter",
    );
    push_dashboard_action(
        &mut actions,
        todo.dups_auto as i64,
        "high",
        "dedup",
        "处理可自动去重组",
        &format!(
            "{} 个重复组已有明确保留/删除建议，可进入去重页先智能选择再确认执行。",
            todo.dups_auto
        ),
        "dedup",
        "打开去重闭环",
        "dedup.auto_groups",
    );
    push_dashboard_action(
        &mut actions,
        todo.dups_review as i64,
        "medium",
        "dedup",
        "复核人工去重组",
        &format!(
            "{} 个重复组需要人工确认，系统会标出推荐保留项和建议删除项。",
            todo.dups_review
        ),
        "dedup",
        "复核重复资源",
        "dedup.review_groups",
    );
    push_dashboard_action(
        &mut actions,
        todo.no_rating as i64,
        "medium",
        "cleanup",
        "刷新无评分元数据",
        &format!(
            "{} 个条目没有评分，建议在智能清理页先刷新元数据，避免误判清理候选。",
            todo.no_rating
        ),
        "cleanup",
        "打开智能清理",
        "dashboard_todo.no_rating",
    );

    match zhuigeng::workbench(State(state.clone())).await {
        Ok(Json(workbench)) => {
            todo.airing_count = workbench.status.continuing;
            todo.airing_low_count = workbench.status.continuing;
            let counts = workbench.counts;
            push_dashboard_action(
                &mut actions,
                counts.update_needed as i64,
                "high",
                "zhuigeng",
                "追更剧有新集可更新",
                &format!(
                    "{} 部在更剧落后，共缺 {} 集；可在追更工作台智能找资源并一条龙更新。",
                    counts.update_needed, counts.behind_total
                ),
                "zhuigeng",
                "智能找资源",
                "zhuigeng.update_needed",
            );
            push_dashboard_action(
                &mut actions,
                counts.complete_after_update as i64,
                "high",
                "zhuigeng",
                "完结剧需要先补齐再归档",
                &format!(
                    "{} 部剧已判定完结但本地仍缺集，先一条龙补齐，完成后再归档。",
                    counts.complete_after_update
                ),
                "zhuigeng",
                "补齐后归档",
                "zhuigeng.complete_after_update",
            );
            push_dashboard_action(
                &mut actions,
                counts.archive_ready as i64,
                "medium",
                "zhuigeng",
                "完结剧可一键归档",
                &format!(
                    "{} 部剧已完结且本地齐全，可批量移动到完结库。",
                    counts.archive_ready
                ),
                "zhuigeng",
                "打开归档队列",
                "zhuigeng.archive_ready",
            );
            push_dashboard_action(
                &mut actions,
                (counts.metadata_error + counts.target_error) as i64,
                "medium",
                "zhuigeng",
                "追更元数据需要修复",
                &format!(
                    "{} 个追更条目缺少 TMDb、路径或库信息，会影响更新和归档判断。",
                    counts.metadata_error + counts.target_error
                ),
                "zhuigeng",
                "查看异常追更",
                "zhuigeng.errors",
            );
        }
        Err(err) => {
            let message = err.to_string();
            if !is_optional_tmdb_config_error(&message) {
                let redacted = redact_sensitive_text(&message);
                todo.airing_err = Some(redacted.clone());
                warnings.push(redacted);
            }
        }
    }

    match recent_task_issue_count(&state.pool).await {
        Ok(count) => push_dashboard_action(
            &mut actions,
            count,
            "medium",
            "tasks",
            "检查失败或中断任务",
            &format!("{count} 个任务最近处于失败或中断状态，建议先打开任务中心确认是否需要重试。"),
            "dashboard",
            "打开任务中心",
            "task_runs",
        ),
        Err(err) => warnings.push(redact_sensitive_text(&err.to_string())),
    }

    actions.sort_by(|left, right| {
        severity_rank(&right.severity)
            .cmp(&severity_rank(&left.severity))
            .then_with(|| right.count.cmp(&left.count))
            .then_with(|| left.area.cmp(&right.area))
    });
    Ok(Json(DashboardSmartActionsResponse {
        ok: warnings.is_empty(),
        total: actions.len(),
        actions,
        warnings,
        todo,
    }))
}

async fn recent_task_issue_count(pool: &sqlx::PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM task_runs
         WHERE status IN ('error', 'interrupted', 'cancelled')
            OR (status = 'running' AND updated_at < now() - interval '30 minutes')",
    )
    .fetch_one(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
fn push_dashboard_action(
    actions: &mut Vec<DashboardSmartAction>,
    count: i64,
    severity: &str,
    area: &str,
    title: &str,
    message: &str,
    tab: &str,
    action: &str,
    source: &str,
) {
    if count <= 0 {
        return;
    }
    actions.push(DashboardSmartAction {
        severity: severity.to_string(),
        area: area.to_string(),
        title: title.to_string(),
        message: message.to_string(),
        count,
        tab: tab.to_string(),
        action: action.to_string(),
        source: source.to_string(),
    });
}

fn severity_rank(severity: &str) -> i32 {
    match severity {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn is_optional_tmdb_config_error(message: &str) -> bool {
    message.contains("tmdb_api_key/tmdb_key 未配置")
        || message.contains("tmdb_base_url/tmdb_url 未配置")
}

async fn no_poster_todo(state: &AppState) -> AppResult<(usize, BTreeMap<String, usize>)> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let report = posters::detect_mismatched_posters(
        &client,
        PosterDetectRequest {
            lib: None,
            limit: Some(100_000),
            include_missing_primary: Some(true),
        },
    )
    .await?;

    let mut by_lib = BTreeMap::new();
    for item in report.items.iter().filter(|item| !item.has_poster) {
        *by_lib.entry(item.lib.clone()).or_insert(0) += 1;
    }
    Ok((report.missing_primary_total, by_lib))
}

async fn no_rating_todo(state: &AppState) -> AppResult<(usize, BTreeMap<String, usize>)> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let libraries = client.libraries().await?;
    let mut total = 0usize;
    let mut by_lib = BTreeMap::new();

    for library in libraries {
        let Some(parent_id) = library.id.as_deref().filter(|id| !id.trim().is_empty()) else {
            continue;
        };
        let page = client
            .cleanup_items(parent_id, dashboard_item_types(&library), 30_000)
            .await?;
        let count = page
            .items
            .iter()
            .filter(|item| item.community_rating.unwrap_or(0.0) <= 0.0)
            .count();
        if count > 0 {
            total += count;
            by_lib.insert(library.name.clone(), count);
        }
    }

    Ok((total, by_lib))
}

fn dashboard_item_types(library: &EmbyLibrary) -> &'static str {
    match library.library_type.to_ascii_lowercase().as_str() {
        "movies" | "movie" => "Movie",
        "tvshows" | "series" | "shows" => "Series",
        _ => "Movie,Series",
    }
}
