use crate::{
    config_store,
    emby::{EmbyCleanupItem, EmbyClient, EmbyLibrary},
    error::{AppError, AppResult},
    state::AppState,
    tasks::{self, TaskRun},
};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, HashSet},
    io::ErrorKind,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use walkdir::WalkDir;

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const STRM_SUMMARY_MAX_DEPTH: usize = 8;
const STRM_SUMMARY_ENTRY_LIMIT: usize = 50_000;
const STRM_SUMMARY_SAMPLE_LIMIT: usize = 20;
const EMPTY_DIR_CLEANUP_LIMIT_DEFAULT: usize = 500;
const EMPTY_DIR_CLEANUP_LIMIT_MAX: usize = 10_000;
const EMPTY_DIR_CLEANUP_SAMPLE_LIMIT: usize = 20;
const CLEANUP_SUGGEST_TOP_DEFAULT: usize = 20;
const CLEANUP_SUGGEST_TOP_MAX: usize = 200;
const CLEANUP_SUGGEST_ITEM_LIMIT: usize = 3000;
const SUBTITLE_EXTENSIONS: &[&str] = &["ass", "idx", "smi", "srt", "ssa", "sub", "sup", "vtt"];
const CLEANUP_DIMENSIONS: &[&str] = &["rating", "age", "idle", "size", "meta"];

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct InsightMeta {
    pub generated_at: DateTime<Utc>,
    pub readonly: bool,
    pub source: Vec<String>,
    pub coverage: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct InsightTodo {
    pub severity: String,
    pub area: String,
    pub message: String,
    pub count: i64,
    pub source: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GapsSummaryResponse {
    pub ok: bool,
    pub complete_business_port: bool,
    pub meta: InsightMeta,
    pub task_history: TaskHistorySummary,
    pub catalog: CatalogInsight,
    pub strm: StrmReadOnlyOverview,
    pub autostrm: AutostrmSnapshot,
    pub todos: Vec<InsightTodo>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CleanupSummaryResponse {
    pub ok: bool,
    pub complete_business_port: bool,
    pub meta: InsightMeta,
    pub task_history: TaskHistorySummary,
    pub catalog: CatalogInsight,
    pub strm: StrmReadOnlyOverview,
    pub autostrm: AutostrmSnapshot,
    pub schedules: ScheduleInsight,
    pub logs: LogInsight,
    pub todos: Vec<InsightTodo>,
    pub warnings: Vec<String>,
    pub cleanup_candidates: Vec<CleanupCandidate>,
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct CleanupSuggestRequest {
    pub lib: Option<String>,
    pub top: Option<usize>,
    pub min_score: Option<f64>,
    pub dimensions: Option<Vec<String>>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CleanupCandidate {
    pub item_id: String,
    pub name: String,
    pub lib: String,
    pub path: Option<String>,
    pub score: f64,
    pub reasons: Vec<String>,
    pub dimensions: BTreeMap<String, CleanupDimensionScore>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CleanupDimensionScore {
    pub score: f64,
    pub reason: String,
    pub value: Option<String>,
    pub warning: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AutostrmStatusResponse {
    pub ok: bool,
    pub complete_business_port: bool,
    pub meta: InsightMeta,
    pub seen: AutostrmSeenStats,
    pub unmatched: AutostrmUnmatchedStats,
    pub libraries: Vec<AutostrmLibraryStat>,
    pub todos: Vec<InsightTodo>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TaskHistorySummary {
    pub total: i64,
    pub pending: i64,
    pub running: i64,
    pub stale_running: i64,
    pub done: i64,
    pub error: i64,
    pub cancelled: i64,
    pub interrupted: i64,
    pub last_updated_at: Option<DateTime<Utc>>,
    pub recent_issues: Vec<TaskIssue>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TaskIssue {
    pub id: Uuid,
    pub kind: String,
    pub label: String,
    pub status: String,
    pub message: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CatalogInsight {
    pub available: bool,
    pub total: i64,
    pub packages: i64,
    pub share115: i64,
    pub magnet: i64,
    pub ed2k: i64,
    pub other: i64,
    pub duplicate_links: i64,
    pub duplicate_names: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StrmReadOnlyOverview {
    pub root: String,
    pub exists: bool,
    pub is_dir: bool,
    pub max_depth: usize,
    pub entry_limit: usize,
    pub directories: i64,
    pub top_level_dirs: i64,
    pub empty_directories: i64,
    pub files: i64,
    pub strm_files: i64,
    pub subtitle_files: i64,
    pub other_files: i64,
    pub extension_counts: Vec<ExtensionCount>,
    pub samples: Vec<StrmSignalSample>,
    pub empty_directory_samples: Vec<String>,
    pub other_file_samples: Vec<String>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ExtensionCount {
    pub extension: String,
    pub count: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StrmSignalSample {
    pub kind: String,
    pub rel_path: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AutostrmSnapshot {
    pub seen: AutostrmSeenStats,
    pub unmatched: AutostrmUnmatchedStats,
    pub libraries: Vec<AutostrmLibraryStat>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AutostrmSeenStats {
    pub total: i64,
    pub libraries: i64,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AutostrmUnmatchedStats {
    pub total: i64,
    pub without_emby_id: i64,
    pub libraries: i64,
    pub first_created_at: Option<DateTime<Utc>>,
    pub last_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AutostrmLibraryStat {
    pub lib: String,
    pub seen: i64,
    pub unmatched: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ScheduleInsight {
    pub total: i64,
    pub enabled: i64,
    pub last_errors: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LogInsight {
    pub errors_7d: i64,
    pub warnings_7d: i64,
    pub last_error_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct EmptyDirCleanupRequest {
    pub execute: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct EmptyDirCleanupResponse {
    pub ok: bool,
    pub dry_run: bool,
    pub execute: bool,
    pub root: String,
    pub candidate_count: usize,
    pub samples: Vec<String>,
    pub truncated: bool,
    pub warnings: Vec<String>,
    pub task: Option<TaskRun>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct EmptyDirCleanupTaskResult {
    pub ok: bool,
    pub dry_run: bool,
    pub root: String,
    pub candidate_count: usize,
    pub deleted_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    pub deleted: Vec<String>,
    pub skipped: Vec<String>,
    pub failures: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/gaps/scan", post(gaps_summary))
        .route("/api/v2/cleanup/suggest", post(cleanup_summary))
        .route("/api/v2/cleanup/empty-dirs", post(cleanup_empty_dirs))
        .route("/api/v2/autostrm/status", get(autostrm_status))
}

#[utoipa::path(post, path = "/api/v2/gaps/scan", tag = "insights", responses((status = 200, body = GapsSummaryResponse)))]
pub async fn gaps_summary(State(state): State<AppState>) -> AppResult<Json<GapsSummaryResponse>> {
    let task_history = task_history_summary(&state.pool).await?;
    let catalog = catalog_insight(&state.pool).await?;
    let autostrm = autostrm_snapshot(&state.pool).await?;
    let strm = strm_readonly_overview(&state.settings.strm_root);

    let mut warnings = strm.warnings.clone();
    if !catalog.available {
        warnings.push("catalog_items 为空，缺集待办只能参考任务与 strm/autostrm 状态".to_string());
    }
    let todos = gaps_todos(&task_history, &catalog, &strm, &autostrm);

    Ok(Json(GapsSummaryResponse {
        ok: true,
        complete_business_port: false,
        meta: insight_meta(
            vec![
                "task_runs",
                "catalog_items",
                "autostrm_seen",
                "autostrm_unmatched",
                "strm_root filesystem metadata",
            ],
            vec![
                "只读预检摘要",
                "覆盖任务历史、目录规模、资源目录可用性、autostrm 未匹配状态",
            ],
            vec![
                "不连接 Emby，不读取剧集元数据，不推断真实缺集集号",
                "不读取 .strm 文件内容，不修改文件系统",
            ],
        ),
        task_history,
        catalog,
        strm,
        autostrm,
        todos,
        warnings,
    }))
}

#[utoipa::path(post, path = "/api/v2/cleanup/suggest", tag = "insights", request_body = CleanupSuggestRequest, responses((status = 200, body = CleanupSummaryResponse)))]
pub async fn cleanup_summary(
    State(state): State<AppState>,
    body: Option<Json<CleanupSuggestRequest>>,
) -> AppResult<Json<CleanupSummaryResponse>> {
    let req = body.map(|Json(req)| req).unwrap_or_default();
    let task_history = task_history_summary(&state.pool).await?;
    let catalog = catalog_insight(&state.pool).await?;
    let autostrm = autostrm_snapshot(&state.pool).await?;
    let schedules = schedule_insight(&state.pool).await?;
    let logs = log_insight(&state.pool).await?;
    let strm = strm_readonly_overview(&state.settings.strm_root);

    let mut warnings = strm.warnings.clone();
    if task_history.stale_running > 0 {
        warnings.push("存在长时间未更新的 running 任务，可能是旧进程残留".to_string());
    }
    let cleanup_candidates = cleanup_candidates(&state, &req, &mut warnings).await?;
    let todos = cleanup_todos(&task_history, &catalog, &strm, &autostrm, &schedules, &logs);

    Ok(Json(CleanupSummaryResponse {
        ok: true,
        complete_business_port: false,
        meta: insight_meta(
            vec![
                "task_runs",
                "schedule_jobs",
                "app_logs",
                "catalog_items",
                "autostrm_seen",
                "autostrm_unmatched",
                "strm_root filesystem metadata",
            ],
            vec![
                "只读清理候选摘要",
                "覆盖失败任务、日志异常、定时任务最近错误、catalog 重复项、strm 目录信号",
            ],
            vec![
                "不删除、不移动、不重命名任何文件",
                "评分建议只读读取 Emby 元数据，不连接 115，不会执行删除",
                "strm 统计只看文件名和元数据，不读取文件内容",
            ],
        ),
        task_history,
        catalog,
        strm,
        autostrm,
        schedules,
        logs,
        todos,
        warnings,
        cleanup_candidates,
    }))
}

#[utoipa::path(post, path = "/api/v2/cleanup/empty-dirs", tag = "insights", request_body = EmptyDirCleanupRequest, responses((status = 200, body = EmptyDirCleanupResponse)))]
pub async fn cleanup_empty_dirs(
    State(state): State<AppState>,
    Json(req): Json<EmptyDirCleanupRequest>,
) -> AppResult<Json<EmptyDirCleanupResponse>> {
    let limit = empty_dir_cleanup_limit(req.limit);
    let scan = scan_empty_strm_dirs(&state.settings.strm_root, limit);
    let execute = req.execute.unwrap_or(false);
    let candidate_count = scan.candidates.len();
    let samples = scan.samples();

    if !execute {
        return Ok(Json(EmptyDirCleanupResponse {
            ok: true,
            dry_run: true,
            execute: false,
            root: scan.root,
            candidate_count,
            samples,
            truncated: scan.truncated,
            warnings: scan.warnings,
            task: None,
        }));
    }

    let params = serde_json::json!({
        "execute": true,
        "limit": limit,
        "root": state.settings.strm_root.display().to_string(),
        "dry_run": false,
    });
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "cleanup_empty_strm_dirs",
        "清理空 STRM 目录",
        candidate_count.max(1) as i64,
        "manual",
        params,
    )
    .await?;
    spawn_empty_dir_cleanup(state, task.id, limit);

    Ok(Json(EmptyDirCleanupResponse {
        ok: true,
        dry_run: false,
        execute: true,
        root: scan.root,
        candidate_count,
        samples,
        truncated: scan.truncated,
        warnings: scan.warnings,
        task: Some(task),
    }))
}

#[utoipa::path(get, path = "/api/v2/autostrm/status", tag = "autostrm", responses((status = 200, body = AutostrmStatusResponse)))]
pub async fn autostrm_status(
    State(state): State<AppState>,
) -> AppResult<Json<AutostrmStatusResponse>> {
    let snapshot = autostrm_snapshot(&state.pool).await?;
    let mut warnings = Vec::new();
    if snapshot.seen.total == 0 && snapshot.unmatched.total == 0 {
        warnings.push("autostrm 状态表暂无数据，可能尚未导入旧数据或尚未收到 webhook".to_string());
    }
    let todos = autostrm_todos(&snapshot);

    Ok(Json(AutostrmStatusResponse {
        ok: true,
        complete_business_port: true,
        meta: insight_meta(
            vec!["autostrm_seen", "autostrm_unmatched"],
            vec![
                "只读状态统计",
                "覆盖 seen/unmatched 数量、库分布、最近更新时间",
            ],
            vec!["状态接口只读；写入由 /api/v2/autostrm/webhook 触发"],
        ),
        seen: snapshot.seen,
        unmatched: snapshot.unmatched,
        libraries: snapshot.libraries,
        todos,
        warnings,
    }))
}

fn insight_meta(
    source: Vec<&'static str>,
    coverage: Vec<&'static str>,
    limitations: Vec<&'static str>,
) -> InsightMeta {
    InsightMeta {
        generated_at: Utc::now(),
        readonly: true,
        source: source.into_iter().map(str::to_string).collect(),
        coverage: coverage.into_iter().map(str::to_string).collect(),
        limitations: limitations.into_iter().map(str::to_string).collect(),
    }
}

async fn cleanup_candidates(
    state: &AppState,
    req: &CleanupSuggestRequest,
    warnings: &mut Vec<String>,
) -> AppResult<Vec<CleanupCandidate>> {
    if !cleanup_request_has_scoring(req) {
        return Ok(Vec::new());
    }

    let dimensions = cleanup_dimensions(req)?;
    let top = req
        .top
        .unwrap_or(CLEANUP_SUGGEST_TOP_DEFAULT)
        .clamp(1, CLEANUP_SUGGEST_TOP_MAX);
    let min_score = req.min_score.unwrap_or(0.0).max(0.0);
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        warnings.push("api_key 未配置，无法读取 Emby 元数据生成清理评分建议".to_string());
        return Ok(Vec::new());
    }

    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let libraries = client.libraries().await?;
    let selected = select_cleanup_libraries(&libraries, req.lib.as_deref())?;
    let mut candidates = Vec::new();

    if dimensions.iter().any(|dimension| dimension == "size") {
        warnings.push(
            "size 维度未读取 CloudDrive/媒体文件内容；仅当未来有安全本地文件元数据时才参与评分"
                .to_string(),
        );
    }
    if dimensions.iter().any(|dimension| dimension == "idle") {
        warnings
            .push("idle 维度需要播放历史/最近访问数据；当前仅返回降级提示，不参与评分".to_string());
    }

    for library in selected {
        let Some(parent_id) = library.id.as_deref() else {
            warnings.push(format!("Emby 库「{}」缺少 ItemId，已跳过", library.name));
            continue;
        };
        let page = client
            .cleanup_items(
                parent_id,
                cleanup_item_types(library),
                CLEANUP_SUGGEST_ITEM_LIMIT,
            )
            .await?;
        if page.truncated {
            warnings.push(format!(
                "Emby 库「{}」条目超过 {}，评分结果已截断",
                library.name, CLEANUP_SUGGEST_ITEM_LIMIT
            ));
        }
        for item in page.items {
            if let Some(candidate) = score_cleanup_item(&library.name, item, &dimensions, min_score)
            {
                candidates.push(candidate);
            }
        }
    }

    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.name.cmp(&right.name))
    });
    candidates.truncate(top);
    Ok(candidates)
}

fn cleanup_request_has_scoring(req: &CleanupSuggestRequest) -> bool {
    req.lib
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || req.top.is_some()
        || req.min_score.is_some()
        || req
            .dimensions
            .as_ref()
            .is_some_and(|items| !items.is_empty())
}

fn cleanup_dimensions(req: &CleanupSuggestRequest) -> AppResult<Vec<String>> {
    let raw = req.dimensions.clone().unwrap_or_else(|| {
        CLEANUP_DIMENSIONS
            .iter()
            .map(|dimension| (*dimension).to_string())
            .collect()
    });
    let mut dimensions = Vec::new();
    for dimension in raw {
        let normalized = dimension.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        if !CLEANUP_DIMENSIONS.contains(&normalized.as_str()) {
            return Err(AppError::BadRequest(format!(
                "未知清理评分维度: {dimension}"
            )));
        }
        if !dimensions.iter().any(|existing| existing == &normalized) {
            dimensions.push(normalized);
        }
    }
    if dimensions.is_empty() {
        return Err(AppError::BadRequest("清理评分维度不能为空".to_string()));
    }
    Ok(dimensions)
}

fn select_cleanup_libraries<'a>(
    libraries: &'a [EmbyLibrary],
    requested: Option<&str>,
) -> AppResult<Vec<&'a EmbyLibrary>> {
    let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(libraries.iter().collect());
    };
    let selected: Vec<_> = libraries
        .iter()
        .filter(|library| {
            library.name.eq_ignore_ascii_case(requested)
                || library.id.as_deref().is_some_and(|id| id == requested)
        })
        .collect();
    if selected.is_empty() {
        return Err(AppError::NotFound(format!(
            "Emby library not found: {requested}"
        )));
    }
    Ok(selected)
}

fn cleanup_item_types(library: &EmbyLibrary) -> &'static str {
    match library.library_type.to_ascii_lowercase().as_str() {
        "movies" | "movie" => "Movie",
        "tvshows" | "series" | "shows" => "Series",
        _ => "Movie,Series",
    }
}

fn score_cleanup_item(
    lib: &str,
    item: EmbyCleanupItem,
    dimensions: &[String],
    min_score: f64,
) -> Option<CleanupCandidate> {
    let mut score = 0.0;
    let mut reasons = Vec::new();
    let mut details = BTreeMap::new();

    for dimension in dimensions {
        let detail = match dimension.as_str() {
            "rating" => score_rating(&item),
            "meta" => score_meta(&item),
            "age" => score_age(&item),
            "size" => CleanupDimensionScore {
                score: 0.0,
                reason: "size 维度已降级，未读取本地媒体文件大小".to_string(),
                value: item.path.clone(),
                warning: Some("no_safe_local_size_metadata".to_string()),
            },
            "idle" => CleanupDimensionScore {
                score: 0.0,
                reason: "idle 维度已降级，当前没有最近播放/访问数据".to_string(),
                value: None,
                warning: Some("idle_source_unavailable".to_string()),
            },
            _ => continue,
        };
        if detail.score > 0.0 {
            reasons.push(detail.reason.clone());
        }
        score += detail.score;
        details.insert(dimension.clone(), detail);
    }

    let score = round_score(score.min(100.0));
    if score < min_score {
        return None;
    }

    Some(CleanupCandidate {
        item_id: item.id?,
        name: item.name.unwrap_or_else(|| "(unnamed)".to_string()),
        lib: lib.to_string(),
        path: item.path,
        score,
        reasons,
        dimensions: details,
    })
}

fn score_rating(item: &EmbyCleanupItem) -> CleanupDimensionScore {
    match item.rating() {
        Some(rating) if rating <= 5.0 => CleanupDimensionScore {
            score: round_score((6.0 - rating).max(0.0) * 8.0),
            reason: format!("评分较低 ({rating:.1})"),
            value: Some(format!("{rating:.1}")),
            warning: None,
        },
        Some(rating) if rating <= 6.5 => CleanupDimensionScore {
            score: round_score((6.8 - rating).max(0.0) * 4.0),
            reason: format!("评分偏低 ({rating:.1})"),
            value: Some(format!("{rating:.1}")),
            warning: None,
        },
        Some(rating) => CleanupDimensionScore {
            score: 0.0,
            reason: "评分正常".to_string(),
            value: Some(format!("{rating:.1}")),
            warning: None,
        },
        None => CleanupDimensionScore {
            score: 0.0,
            reason: "缺少评分数据".to_string(),
            value: None,
            warning: Some("rating_unavailable".to_string()),
        },
    }
}

fn score_meta(item: &EmbyCleanupItem) -> CleanupDimensionScore {
    let missing_provider = !item.has_provider_id("Tmdb")
        && !item.has_provider_id("Imdb")
        && !item.has_provider_id("Tvdb");
    let missing_image = !item.has_primary_image();
    let mut score = 0.0;
    let mut parts = Vec::new();
    if missing_provider {
        score += 18.0;
        parts.push("缺少 TMDb/IMDb/TVDb 标识");
    }
    if missing_image {
        score += 8.0;
        parts.push("缺少主图");
    }

    CleanupDimensionScore {
        score,
        reason: if parts.is_empty() {
            "元数据较完整".to_string()
        } else {
            parts.join("；")
        },
        value: None,
        warning: None,
    }
}

fn score_age(item: &EmbyCleanupItem) -> CleanupDimensionScore {
    let Some(year) = item.production_year else {
        return CleanupDimensionScore {
            score: 0.0,
            reason: "缺少年份数据".to_string(),
            value: None,
            warning: Some("production_year_unavailable".to_string()),
        };
    };
    let current_year = Utc::now()
        .format("%Y")
        .to_string()
        .parse::<i32>()
        .unwrap_or(2026);
    let age = (current_year - year).max(0);
    let score = if age >= 25 {
        18.0
    } else if age >= 15 {
        12.0
    } else if age >= 8 {
        6.0
    } else {
        0.0
    };
    CleanupDimensionScore {
        score,
        reason: if score > 0.0 {
            format!("年份较早 ({year})")
        } else {
            "年份较新".to_string()
        },
        value: Some(year.to_string()),
        warning: None,
    }
}

fn round_score(score: f64) -> f64 {
    (score * 10.0).round() / 10.0
}

async fn task_history_summary(pool: &PgPool) -> AppResult<TaskHistorySummary> {
    let (
        total,
        pending,
        running,
        stale_running,
        done,
        error,
        cancelled,
        interrupted,
        last_updated_at,
    ): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<DateTime<Utc>>,
    ) = sqlx::query_as(
        "SELECT
                COUNT(*)::bigint,
                COUNT(*) FILTER (WHERE status = 'pending')::bigint,
                COUNT(*) FILTER (WHERE status = 'running')::bigint,
                COUNT(*) FILTER (
                    WHERE status = 'running' AND updated_at < now() - interval '30 minutes'
                )::bigint,
                COUNT(*) FILTER (WHERE status = 'done')::bigint,
                COUNT(*) FILTER (WHERE status = 'error')::bigint,
                COUNT(*) FILTER (WHERE status = 'cancelled')::bigint,
                COUNT(*) FILTER (WHERE status = 'interrupted')::bigint,
                MAX(updated_at)
             FROM task_runs",
    )
    .fetch_one(pool)
    .await?;

    let recent_issues = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            String,
            Option<String>,
            String,
            DateTime<Utc>,
        ),
    >(
        "SELECT id, kind, label, status, error, status_text, updated_at
         FROM task_runs
         WHERE status IN ('error', 'interrupted', 'cancelled')
         ORDER BY updated_at DESC
         LIMIT 8",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(
        |(id, kind, label, status, error, status_text, updated_at)| TaskIssue {
            id,
            kind,
            label,
            status,
            message: error
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(status_text),
            updated_at,
        },
    )
    .collect();

    Ok(TaskHistorySummary {
        total,
        pending,
        running,
        stale_running,
        done,
        error,
        cancelled,
        interrupted,
        last_updated_at,
        recent_issues,
    })
}

async fn catalog_insight(pool: &PgPool) -> AppResult<CatalogInsight> {
    let (total, packages, share115, magnet, ed2k, other): (i64, i64, i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT
                COUNT(*)::bigint,
                COUNT(*) FILTER (WHERE is_pkg)::bigint,
                COUNT(*) FILTER (WHERE link_type = 'share115')::bigint,
                COUNT(*) FILTER (WHERE link_type = 'magnet')::bigint,
                COUNT(*) FILTER (WHERE link_type = 'ed2k')::bigint,
                COUNT(*) FILTER (WHERE link_type NOT IN ('share115', 'magnet', 'ed2k'))::bigint
             FROM catalog_items",
        )
        .fetch_one(pool)
        .await?;
    let duplicate_links: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint
         FROM (
            SELECT link
            FROM catalog_items
            WHERE link <> ''
            GROUP BY link
            HAVING COUNT(*) > 1
         ) duplicated",
    )
    .fetch_one(pool)
    .await?;
    let duplicate_names: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint
         FROM (
            SELECT lower(name) AS normalized_name
            FROM catalog_items
            WHERE name <> ''
            GROUP BY lower(name)
            HAVING COUNT(*) > 1
         ) duplicated",
    )
    .fetch_one(pool)
    .await?;

    Ok(CatalogInsight {
        available: total > 0,
        total,
        packages,
        share115,
        magnet,
        ed2k,
        other,
        duplicate_links,
        duplicate_names,
    })
}

async fn autostrm_snapshot(pool: &PgPool) -> AppResult<AutostrmSnapshot> {
    let (seen_total, seen_libraries, last_seen_at): (i64, i64, Option<DateTime<Utc>>) =
        sqlx::query_as(
            "SELECT COUNT(*)::bigint, COUNT(DISTINCT lib)::bigint, MAX(updated_at)
             FROM autostrm_seen",
        )
        .fetch_one(pool)
        .await?;
    let (
        unmatched_total,
        without_emby_id,
        unmatched_libraries,
        first_created_at,
        last_updated_at,
    ): (i64, i64, i64, Option<DateTime<Utc>>, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT
            COUNT(*)::bigint,
            COUNT(*) FILTER (WHERE emby_id IS NULL OR emby_id = '')::bigint,
            COUNT(DISTINCT lib)::bigint,
            MIN(created_at),
            MAX(updated_at)
         FROM autostrm_unmatched",
    )
    .fetch_one(pool)
    .await?;
    let libraries = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT
            COALESCE(s.lib, u.lib) AS lib,
            COALESCE(s.seen, 0)::bigint AS seen,
            COALESCE(u.unmatched, 0)::bigint AS unmatched
         FROM (
            SELECT lib, COUNT(*)::bigint AS seen
            FROM autostrm_seen
            GROUP BY lib
         ) s
         FULL OUTER JOIN (
            SELECT lib, COUNT(*)::bigint AS unmatched
            FROM autostrm_unmatched
            GROUP BY lib
         ) u USING (lib)
         ORDER BY COALESCE(u.unmatched, 0) DESC, COALESCE(s.seen, 0) DESC, lib ASC
         LIMIT 20",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(lib, seen, unmatched)| AutostrmLibraryStat {
        lib,
        seen,
        unmatched,
    })
    .collect();

    Ok(AutostrmSnapshot {
        seen: AutostrmSeenStats {
            total: seen_total,
            libraries: seen_libraries,
            last_seen_at,
        },
        unmatched: AutostrmUnmatchedStats {
            total: unmatched_total,
            without_emby_id,
            libraries: unmatched_libraries,
            first_created_at,
            last_updated_at,
        },
        libraries,
    })
}

async fn schedule_insight(pool: &PgPool) -> AppResult<ScheduleInsight> {
    let (total, enabled, last_errors): (i64, i64, i64) = sqlx::query_as(
        "SELECT
            COUNT(*)::bigint,
            COUNT(*) FILTER (WHERE enabled)::bigint,
            COUNT(*) FILTER (WHERE last_status = 'error')::bigint
         FROM schedule_jobs",
    )
    .fetch_one(pool)
    .await?;
    Ok(ScheduleInsight {
        total,
        enabled,
        last_errors,
    })
}

async fn log_insight(pool: &PgPool) -> AppResult<LogInsight> {
    let (errors_7d, warnings_7d, last_error_at): (i64, i64, Option<DateTime<Utc>>) =
        sqlx::query_as(
            "SELECT
                COUNT(*) FILTER (
                    WHERE lower(level) = 'error'
                      AND created_at >= now() - interval '7 days'
                )::bigint,
                COUNT(*) FILTER (
                    WHERE lower(level) IN ('warn', 'warning')
                      AND created_at >= now() - interval '7 days'
                )::bigint,
                MAX(created_at) FILTER (WHERE lower(level) = 'error')
             FROM app_logs",
        )
        .fetch_one(pool)
        .await?;
    Ok(LogInsight {
        errors_7d,
        warnings_7d,
        last_error_at,
    })
}

pub fn strm_readonly_overview(root: &Path) -> StrmReadOnlyOverview {
    let mut overview = StrmReadOnlyOverview {
        root: root.display().to_string(),
        exists: root.exists(),
        is_dir: root.is_dir(),
        max_depth: STRM_SUMMARY_MAX_DEPTH,
        entry_limit: STRM_SUMMARY_ENTRY_LIMIT,
        directories: 0,
        top_level_dirs: 0,
        empty_directories: 0,
        files: 0,
        strm_files: 0,
        subtitle_files: 0,
        other_files: 0,
        extension_counts: Vec::new(),
        samples: Vec::new(),
        empty_directory_samples: Vec::new(),
        other_file_samples: Vec::new(),
        truncated: false,
        warnings: Vec::new(),
    };
    if !overview.exists {
        overview
            .warnings
            .push(format!("strm_root 不存在: {}", root.display()));
        return overview;
    }
    if !overview.is_dir {
        overview
            .warnings
            .push(format!("strm_root 不是目录: {}", root.display()));
        return overview;
    }

    let mut extension_counts = BTreeMap::<String, i64>::new();
    for entry in WalkDir::new(root)
        .min_depth(1)
        .max_depth(STRM_SUMMARY_MAX_DEPTH)
        .follow_links(false)
        .into_iter()
    {
        let Ok(entry) = entry else {
            overview
                .warnings
                .push("strm_root 遍历时有条目不可读".to_string());
            continue;
        };
        let depth = entry.depth();
        if (overview.directories + overview.files) as usize >= STRM_SUMMARY_ENTRY_LIMIT {
            overview.truncated = true;
            break;
        }

        let path = entry.path();
        let rel_path = rel_display(root, path);
        if entry.file_type().is_dir() {
            overview.directories += 1;
            if depth == 1 {
                overview.top_level_dirs += 1;
            }
            if is_empty_dir(path) {
                overview.empty_directories += 1;
                push_sample(&mut overview.samples, "empty_dir", &rel_path);
                push_text_sample(&mut overview.empty_directory_samples, &rel_path);
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        overview.files += 1;
        let ext = extension(path);
        if !ext.is_empty() {
            *extension_counts.entry(ext.clone()).or_default() += 1;
        }
        if ext == "strm" {
            overview.strm_files += 1;
            push_sample(&mut overview.samples, "strm", &rel_path);
        } else if SUBTITLE_EXTENSIONS.contains(&ext.as_str()) {
            overview.subtitle_files += 1;
            push_sample(&mut overview.samples, "subtitle", &rel_path);
        } else {
            overview.other_files += 1;
            push_sample(&mut overview.samples, "other_file", &rel_path);
            push_text_sample(&mut overview.other_file_samples, &rel_path);
        }
    }
    overview.extension_counts = extension_counts
        .into_iter()
        .map(|(extension, count)| ExtensionCount { extension, count })
        .collect();
    overview
}

fn is_empty_dir(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn rel_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn push_sample(samples: &mut Vec<StrmSignalSample>, kind: &str, rel_path: &str) {
    if samples.len() >= STRM_SUMMARY_SAMPLE_LIMIT {
        return;
    }
    samples.push(StrmSignalSample {
        kind: kind.to_string(),
        rel_path: rel_path.to_string(),
    });
}

fn push_text_sample(samples: &mut Vec<String>, rel_path: &str) {
    if samples.len() >= STRM_SUMMARY_SAMPLE_LIMIT {
        return;
    }
    samples.push(rel_path.to_string());
}

#[derive(Debug, Clone)]
struct EmptyDirCandidate {
    path: PathBuf,
    rel_path: String,
}

#[derive(Debug, Clone)]
struct EmptyDirScan {
    root: String,
    candidates: Vec<EmptyDirCandidate>,
    truncated: bool,
    warnings: Vec<String>,
}

impl EmptyDirScan {
    fn samples(&self) -> Vec<String> {
        self.candidates
            .iter()
            .take(EMPTY_DIR_CLEANUP_SAMPLE_LIMIT)
            .map(|candidate| candidate.rel_path.clone())
            .collect()
    }
}

fn empty_dir_cleanup_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(EMPTY_DIR_CLEANUP_LIMIT_DEFAULT)
        .clamp(1, EMPTY_DIR_CLEANUP_LIMIT_MAX)
}

fn scan_empty_strm_dirs(root: &Path, limit: usize) -> EmptyDirScan {
    let mut scan = EmptyDirScan {
        root: root.display().to_string(),
        candidates: Vec::new(),
        truncated: false,
        warnings: Vec::new(),
    };
    if !root.exists() {
        scan.warnings
            .push(format!("strm_root 不存在: {}", root.display()));
        return scan;
    }
    if !root.is_dir() {
        scan.warnings
            .push(format!("strm_root 不是目录: {}", root.display()));
        return scan;
    }
    let Ok(canon_root) = root.canonicalize() else {
        scan.warnings
            .push(format!("strm_root 无法解析真实路径: {}", root.display()));
        return scan;
    };

    let mut dirs = Vec::<(PathBuf, usize)>::new();
    let mut blocked_dirs = HashSet::<PathBuf>::new();
    let mut visited = 0usize;
    for entry in WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .contents_first(true)
        .into_iter()
    {
        if visited >= STRM_SUMMARY_ENTRY_LIMIT {
            scan.truncated = true;
            break;
        }
        visited += 1;

        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                scan.warnings
                    .push("strm_root 遍历时有条目不可读，已跳过相关父目录".to_string());
                if let Some(path) = err.path() {
                    mark_ancestors_blocked(root, path, &mut blocked_dirs);
                }
                continue;
            }
        };

        if entry.file_type().is_dir() {
            dirs.push((entry.path().to_path_buf(), entry.depth()));
        } else {
            mark_ancestors_blocked(root, entry.path(), &mut blocked_dirs);
        }
    }

    dirs.sort_by(|(left_path, left_depth), (right_path, right_depth)| {
        right_depth
            .cmp(left_depth)
            .then_with(|| rel_display(root, left_path).cmp(&rel_display(root, right_path)))
    });

    for (path, _depth) in dirs {
        if scan.candidates.len() >= limit {
            scan.truncated = true;
            break;
        }
        if blocked_dirs.contains(&path) {
            continue;
        }
        if let Err(err) = guard_existing_strm_child_dir(root, &canon_root, &path) {
            scan.warnings.push(err.to_string());
            continue;
        }
        scan.candidates.push(EmptyDirCandidate {
            rel_path: rel_display(root, &path),
            path,
        });
    }

    scan
}

fn mark_ancestors_blocked(root: &Path, path: &Path, blocked: &mut HashSet<PathBuf>) {
    if path != root && path.starts_with(root) {
        blocked.insert(path.to_path_buf());
    }
    let mut current = path.parent();
    while let Some(parent) = current {
        if parent == root {
            break;
        }
        if !parent.starts_with(root) {
            break;
        }
        blocked.insert(parent.to_path_buf());
        current = parent.parent();
    }
}

fn guard_existing_strm_child_dir(root: &Path, canon_root: &Path, path: &Path) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(|err| {
        AppError::BadRequest(format!(
            "空目录候选不可读，已拒绝: {} ({err})",
            path.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Err(AppError::BadRequest(format!(
            "空目录候选不是普通目录，已拒绝: {}",
            path.display()
        )));
    }
    let canon_path = path.canonicalize().map_err(|err| {
        AppError::BadRequest(format!(
            "空目录候选无法解析真实路径，已拒绝: {} ({err})",
            path.display()
        ))
    })?;
    if canon_path == canon_root || !canon_path.starts_with(canon_root) {
        return Err(AppError::BadRequest(format!(
            "空目录候选越过 strm_root，已拒绝: {}",
            path.display()
        )));
    }
    if !path.starts_with(root) {
        return Err(AppError::BadRequest(format!(
            "空目录候选不在 strm_root 下，已拒绝: {}",
            path.display()
        )));
    }
    Ok(())
}

fn spawn_empty_dir_cleanup(state: AppState, task_id: Uuid, limit: usize) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, task_id, "任务并发槽不可用", None).await;
            return;
        };
        if tasks::cancel_requested(&state.pool, task_id).await {
            let _ = tasks::finish_cancelled(&state.pool, task_id).await;
            return;
        }
        let _ = tasks::mark_running(&state.pool, task_id, "扫描空 STRM 目录").await;
        match run_empty_dir_cleanup(&state, task_id, limit).await {
            Ok(result) => {
                let _ = tasks::finish_done_with_message(
                    &state.pool,
                    task_id,
                    "空 STRM 目录清理完成",
                    serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
                )
                .await;
            }
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, task_id, &err.to_string(), None).await;
            }
        }
    });
}

async fn run_empty_dir_cleanup(
    state: &AppState,
    task_id: Uuid,
    limit: usize,
) -> AppResult<EmptyDirCleanupTaskResult> {
    let scan = scan_empty_strm_dirs(&state.settings.strm_root, limit);
    let total = scan.candidates.len().max(1) as i64;
    tasks::set_total(&state.pool, task_id, total).await?;
    if scan.candidates.is_empty() {
        tasks::set_progress(&state.pool, task_id, 1, "没有空 STRM 目录").await?;
        return Ok(EmptyDirCleanupTaskResult {
            ok: true,
            dry_run: false,
            root: scan.root,
            candidate_count: 0,
            deleted_count: 0,
            skipped_count: 0,
            failed_count: 0,
            deleted: Vec::new(),
            skipped: Vec::new(),
            failures: Vec::new(),
            warnings: scan.warnings,
        });
    }

    let canon_root = state
        .settings
        .strm_root
        .canonicalize()
        .map_err(|err| AppError::BadRequest(format!("strm_root 无法解析真实路径: {err}")))?;
    let mut deleted = Vec::<String>::new();
    let mut skipped = Vec::<String>::new();
    let mut failures = Vec::<String>::new();

    for (idx, candidate) in scan.candidates.iter().enumerate() {
        if tasks::cancel_requested(&state.pool, task_id).await {
            tasks::finish_cancelled(&state.pool, task_id).await?;
            return Err(AppError::Conflict("任务已取消".to_string()));
        }
        tasks::set_progress(
            &state.pool,
            task_id,
            idx as i64,
            &format!("清理 {}", candidate.rel_path),
        )
        .await?;

        if let Err(err) =
            guard_existing_strm_child_dir(&state.settings.strm_root, &canon_root, &candidate.path)
        {
            failures.push(format!("{}: {err}", candidate.rel_path));
            continue;
        }
        match std::fs::remove_dir(&candidate.path) {
            Ok(()) => deleted.push(candidate.rel_path.clone()),
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                ) =>
            {
                skipped.push(candidate.rel_path.clone());
            }
            Err(err) => failures.push(format!("{}: {err}", candidate.rel_path)),
        }
        tasks::set_progress(
            &state.pool,
            task_id,
            (idx + 1) as i64,
            &format!("已处理 {}/{}", idx + 1, total),
        )
        .await?;
    }

    Ok(EmptyDirCleanupTaskResult {
        ok: failures.is_empty(),
        dry_run: false,
        root: scan.root,
        candidate_count: scan.candidates.len(),
        deleted_count: deleted.len(),
        skipped_count: skipped.len(),
        failed_count: failures.len(),
        deleted,
        skipped,
        failures,
        warnings: scan.warnings,
    })
}

fn gaps_todos(
    task_history: &TaskHistorySummary,
    catalog: &CatalogInsight,
    strm: &StrmReadOnlyOverview,
    autostrm: &AutostrmSnapshot,
) -> Vec<InsightTodo> {
    let mut todos = Vec::new();
    if autostrm.unmatched.total > 0 {
        todos.push(todo(
            "high",
            "autostrm",
            "存在未匹配条目，缺集扫描前建议先处理 unmatched 队列",
            autostrm.unmatched.total,
            "autostrm_unmatched",
        ));
    }
    if task_history.error + task_history.interrupted > 0 {
        todos.push(todo(
            "medium",
            "tasks",
            "存在失败或中断任务，可能影响缺集判断的历史基线",
            task_history.error + task_history.interrupted,
            "task_runs",
        ));
    }
    if strm.exists && strm.strm_files == 0 {
        todos.push(todo(
            "medium",
            "strm",
            "strm_root 下没有发现 .strm 文件，缺集扫描暂无本地只读参照",
            1,
            "strm_root filesystem metadata",
        ));
    }
    if !catalog.available {
        todos.push(todo(
            "low",
            "catalog",
            "catalog_items 为空，无法给缺集待办提供资源目录参照",
            1,
            "catalog_items",
        ));
    }
    todos
}

fn cleanup_todos(
    task_history: &TaskHistorySummary,
    catalog: &CatalogInsight,
    strm: &StrmReadOnlyOverview,
    autostrm: &AutostrmSnapshot,
    schedules: &ScheduleInsight,
    logs: &LogInsight,
) -> Vec<InsightTodo> {
    let mut todos = Vec::new();
    if task_history.stale_running > 0 {
        todos.push(todo(
            "high",
            "tasks",
            "存在超过 30 分钟未更新的 running 任务，建议人工核对后清理状态",
            task_history.stale_running,
            "task_runs",
        ));
    }
    if task_history.error + task_history.interrupted > 0 {
        todos.push(todo(
            "medium",
            "tasks",
            "存在失败或中断任务，可作为清理/重试候选",
            task_history.error + task_history.interrupted,
            "task_runs",
        ));
    }
    if logs.errors_7d > 0 {
        todos.push(todo(
            "medium",
            "logs",
            "最近 7 天有 error 日志，建议先查看日志再执行危险清理",
            logs.errors_7d,
            "app_logs",
        ));
    }
    if schedules.last_errors > 0 {
        todos.push(todo(
            "medium",
            "schedules",
            "有定时任务最近一次执行失败",
            schedules.last_errors,
            "schedule_jobs",
        ));
    }
    if catalog.duplicate_links > 0 {
        todos.push(todo(
            "low",
            "catalog",
            "catalog_items 存在重复链接，可后续做只读去重报告",
            catalog.duplicate_links,
            "catalog_items",
        ));
    }
    if catalog.duplicate_names > 0 {
        todos.push(todo(
            "low",
            "catalog",
            "catalog_items 存在重复名称，可后续辅助资源目录整理",
            catalog.duplicate_names,
            "catalog_items",
        ));
    }
    if strm.empty_directories > 0 {
        todos.push(todo(
            "low",
            "strm",
            "strm_root 下存在空目录，仅作为候选信号，不会自动删除",
            strm.empty_directories,
            "strm_root filesystem metadata",
        ));
    }
    if strm.other_files > 0 {
        todos.push(todo(
            "low",
            "strm",
            "strm_root 下存在非 .strm/字幕文件，可后续生成清理候选列表",
            strm.other_files,
            "strm_root filesystem metadata",
        ));
    }
    if autostrm.unmatched.total > 0 {
        todos.push(todo(
            "low",
            "autostrm",
            "autostrm unmatched 仍有积压，清理前建议先确认是否为真实未匹配",
            autostrm.unmatched.total,
            "autostrm_unmatched",
        ));
    }
    todos
}

fn autostrm_todos(snapshot: &AutostrmSnapshot) -> Vec<InsightTodo> {
    let mut todos = Vec::new();
    if snapshot.unmatched.total > 0 {
        todos.push(todo(
            "medium",
            "autostrm",
            "存在 unmatched 条目，需要后续匹配 worker 或人工处理",
            snapshot.unmatched.total,
            "autostrm_unmatched",
        ));
    }
    if snapshot.seen.total == 0 {
        todos.push(todo(
            "low",
            "autostrm",
            "seen 表为空，当前只能说明状态库已就绪，不能证明 webhook 已运行",
            1,
            "autostrm_seen",
        ));
    }
    todos
}

fn todo(severity: &str, area: &str, message: &str, count: i64, source: &str) -> InsightTodo {
    InsightTodo {
        severity: severity.to_string(),
        area: area.to_string(),
        message: message.to_string(),
        count,
        source: source.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{guard_existing_strm_child_dir, scan_empty_strm_dirs, strm_readonly_overview};

    #[test]
    fn strm_overview_is_metadata_only_and_counts_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let season = tmp.path().join("Shows").join("Season 1");
        let empty = tmp.path().join("Empty");
        std::fs::create_dir_all(&season).unwrap();
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(season.join("E01.strm"), "http://example.invalid/E01.mkv").unwrap();
        std::fs::write(season.join("E01.srt"), "subtitle").unwrap();
        std::fs::write(season.join("poster.jpg"), "image").unwrap();

        let overview = strm_readonly_overview(tmp.path());

        assert!(overview.exists);
        assert_eq!(overview.top_level_dirs, 2);
        assert_eq!(overview.strm_files, 1);
        assert_eq!(overview.subtitle_files, 1);
        assert_eq!(overview.other_files, 1);
        assert_eq!(overview.empty_directories, 1);
        assert!(
            overview
                .samples
                .iter()
                .any(|sample| sample.kind == "empty_dir" && sample.rel_path == "Empty")
        );
    }

    #[test]
    fn empty_dir_scan_includes_parents_after_empty_children() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("Shows").join("Empty Season");
        let keep = tmp.path().join("Movies").join("Keep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("movie.strm"), "http://example.invalid/movie.mkv").unwrap();

        let scan = scan_empty_strm_dirs(tmp.path(), 20);

        assert_eq!(scan.candidates.len(), 2, "{scan:?}");
        assert_eq!(scan.candidates[0].rel_path, "Shows/Empty Season");
        assert_eq!(scan.candidates[1].rel_path, "Shows");
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| !candidate.rel_path.starts_with("Movies")),
            "{scan:?}"
        );
    }

    #[test]
    fn empty_dir_guard_rejects_outside_path() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_empty = outside.path().join("Empty");
        std::fs::create_dir_all(&outside_empty).unwrap();
        let canon_root = root.path().canonicalize().unwrap();

        let err = guard_existing_strm_child_dir(root.path(), &canon_root, &outside_empty)
            .expect_err("outside path should be rejected");

        assert!(err.to_string().contains("strm_root"), "{err}");
    }
}
