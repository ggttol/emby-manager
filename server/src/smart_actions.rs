use crate::{
    catalog::{
        self, CatalogItem, CatalogLibraryContextResponse, CatalogResourceRecommendation,
        CatalogTransferExecuteRequest,
    },
    config_store,
    dashboard::{self, DashboardSmartAction},
    dedup::{
        self, DedupExecuteBatchRequest, DedupFolderRef, DedupGroup, DedupReviewGroup, DedupRow,
    },
    emby::EmbyClient,
    error::{AppError, AppResult},
    posters::{self, PosterDetectRequest, PosterSignalItem},
    state::AppState,
    tasks::{self, TaskRun},
    zhuigeng::{
        self, ZhuigengArchiveExecuteRequest, ZhuigengItem, ZhuigengItemRef,
        ZhuigengUpdateExecuteRequest, ZhuigengWorkbenchLane, ZhuigengWorkbenchRow,
    },
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct SmartActionsQuery {
    pub status: Option<String>,
    pub action_type: Option<String>,
    pub risk: Option<String>,
    pub subject_kind: Option<String>,
    pub lib: Option<String>,
    pub q: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const POSTER_ACTION_SCAN_LIMIT: usize = 120;
const POSTER_ACTION_MAX_ITEMS: usize = 30;
const DEDUP_ACTION_MAX_GROUPS: usize = 30;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionsListResponse {
    pub ok: bool,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
    pub actions: Vec<SmartAction>,
    pub warnings: Vec<String>,
    pub summary: SmartActionsSummary,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionsSummaryResponse {
    pub ok: bool,
    pub summary: SmartActionsSummary,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct SmartActionInspectRequest {
    pub subject: Option<SmartSubject>,
    pub q: Option<String>,
    pub limit: Option<usize>,
    pub catalog_items: Option<Vec<CatalogItem>>,
    pub catalog_context: Option<CatalogLibraryContextResponse>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionInspectResponse {
    pub ok: bool,
    pub actions: Vec<SmartAction>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct SmartActionsSummary {
    pub total: usize,
    pub suggested: usize,
    pub auto_ready: usize,
    pub confirm_required: usize,
    pub running: usize,
    pub failed: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub critical: usize,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionDetailResponse {
    pub ok: bool,
    pub action: SmartAction,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionFromTaskResponse {
    pub ok: bool,
    pub action: SmartAction,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SmartActionFromNextActionRequest {
    pub next_action: SmartNextAction,
    pub source_action_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub persist: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionFromNextActionResponse {
    pub ok: bool,
    pub action: SmartAction,
    pub persisted: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct SmartActionExecuteRequest {
    pub confirm_text: Option<String>,
    pub dry_run: Option<bool>,
    pub payload: Option<Value>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionExecuteResponse {
    pub ok: bool,
    pub id: Uuid,
    pub status: SmartActionStatus,
    pub task: TaskRun,
    pub message: String,
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct SmartActionExecuteBatchRequest {
    pub ids: Vec<Uuid>,
    pub dry_run: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionExecuteBatchItem {
    pub id: Uuid,
    pub ok: bool,
    pub status: Option<SmartActionStatus>,
    pub task: Option<TaskRun>,
    pub err: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionExecuteBatchResponse {
    pub ok: bool,
    pub total: usize,
    pub submitted: usize,
    pub failed: usize,
    pub results: Vec<SmartActionExecuteBatchItem>,
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct SmartActionDismissRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionDismissResponse {
    pub ok: bool,
    pub id: Uuid,
    pub status: SmartActionStatus,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionVerifyResponse {
    pub ok: bool,
    pub id: Uuid,
    pub status: SmartActionStatus,
    pub result: Value,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartNextActionSubject {
    pub name: Option<String>,
    pub lib: Option<String>,
    pub tmdb: Option<String>,
    pub emby_id: Option<String>,
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartNextAction {
    pub action_type: String,
    pub label: String,
    pub tab: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<SmartNextActionSubject>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartActionTaskResult {
    pub action_id: Uuid,
    pub action_type: String,
    pub subject: SmartSubject,
    pub dry_run: bool,
    pub steps: Vec<Value>,
    pub outputs: Vec<Value>,
    pub verification: Value,
    pub next_actions: Vec<SmartNextAction>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SmartActionPolicy {
    pub key: String,
    pub enabled: bool,
    pub mode: SmartPolicyMode,
    pub max_risk: SmartRiskLevel,
    pub params: Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionPoliciesResponse {
    pub ok: bool,
    pub policies: Vec<SmartActionPolicy>,
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct SmartActionPolicyUpdateRequest {
    pub enabled: Option<bool>,
    pub mode: Option<SmartPolicyMode>,
    pub max_risk: Option<SmartRiskLevel>,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SmartActionPolicyUpdateResponse {
    pub ok: bool,
    pub policy: SmartActionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartAction {
    pub id: Uuid,
    pub action_type: SmartActionType,
    pub status: SmartActionStatus,
    pub subject: SmartSubject,
    pub title: String,
    pub summary: String,
    pub recommendation: SmartRecommendation,
    pub evidence: Vec<SmartEvidence>,
    pub plan: SmartExecutionPlan,
    pub risk: SmartRisk,
    pub policy: SmartPolicyDecision,
    pub verification: SmartVerificationPlan,
    pub source: String,
    pub tab: String,
    pub action_label: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmartActionType {
    TransferAddNew,
    TransferUpdateSeries,
    DedupRemoveOld,
    DedupReview,
    PosterFix,
    MetadataRefresh,
    LibraryScan,
    ArchiveSeries,
    CleanupEmptyFolder,
    TaskRetryOrDiagnose,
}

impl SmartActionType {
    fn all() -> &'static [Self] {
        &[
            Self::TransferAddNew,
            Self::TransferUpdateSeries,
            Self::DedupRemoveOld,
            Self::DedupReview,
            Self::PosterFix,
            Self::MetadataRefresh,
            Self::LibraryScan,
            Self::ArchiveSeries,
            Self::CleanupEmptyFolder,
            Self::TaskRetryOrDiagnose,
        ]
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::TransferAddNew => "transfer_add_new",
            Self::TransferUpdateSeries => "transfer_update_series",
            Self::DedupRemoveOld => "dedup_remove_old",
            Self::DedupReview => "dedup_review",
            Self::PosterFix => "poster_fix",
            Self::MetadataRefresh => "metadata_refresh",
            Self::LibraryScan => "library_scan",
            Self::ArchiveSeries => "archive_series",
            Self::CleanupEmptyFolder => "cleanup_empty_folder",
            Self::TaskRetryOrDiagnose => "task_retry_or_diagnose",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Self::all()
            .iter()
            .copied()
            .find(|action_type| action_type.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmartActionStatus {
    Suggested,
    Confirmed,
    Queued,
    Running,
    Verifying,
    Done,
    Partial,
    Failed,
    Cancelled,
    Dismissed,
}

impl SmartActionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Suggested => "suggested",
            Self::Confirmed => "confirmed",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Verifying => "verifying",
            Self::Done => "done",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Dismissed => "dismissed",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "suggested" => Some(Self::Suggested),
            "confirmed" => Some(Self::Confirmed),
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "verifying" => Some(Self::Verifying),
            "done" => Some(Self::Done),
            "partial" => Some(Self::Partial),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmartSubjectKind {
    Movie,
    Series,
    Season,
    Episode,
    Library,
    Task,
    System,
    Unknown,
}

impl SmartSubjectKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Season => "season",
            Self::Episode => "episode",
            Self::Library => "library",
            Self::Task => "task",
            Self::System => "system",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartSubject {
    pub kind: SmartSubjectKind,
    pub name: String,
    pub year: Option<i32>,
    pub tmdb: Option<String>,
    pub emby_id: Option<String>,
    pub lib: Option<String>,
    pub folder: Option<String>,
    pub strm_path: Option<String>,
    pub cd_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartRecommendation {
    pub score: i32,
    pub confidence: SmartConfidence,
    pub primary_action: String,
    pub reasons: Vec<String>,
    pub alternatives: Vec<SmartAlternative>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmartConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartAlternative {
    pub action: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartEvidence {
    pub source: SmartEvidenceSource,
    pub label: String,
    pub value: Value,
    pub weight: i32,
    pub collected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmartEvidenceSource {
    EmbyItem,
    EmbyEpisodes,
    StrmScan,
    CloudDrivePath,
    C115Resource,
    CatalogCandidate,
    TmdbMetadata,
    PosterDetection,
    DedupAnalysis,
    TaskHistory,
    UndoLog,
    SystemHealth,
    DashboardTodo,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartRisk {
    pub level: SmartRiskLevel,
    pub destructive: bool,
    pub touches_emby: bool,
    pub touches_disk: bool,
    pub touches_c115: bool,
    pub requires_confirm_text: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SmartRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl SmartRiskLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartExecutionPlan {
    pub steps: Vec<SmartExecutionStep>,
    pub estimated_seconds: Option<i64>,
    pub concurrency_key: Option<String>,
    pub can_cancel: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartExecutionStep {
    pub key: String,
    pub title: String,
    pub executor: SmartExecutorKind,
    pub params: Value,
    pub rollback: Option<SmartRollbackStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmartExecutorKind {
    OpenTab,
    ExistingEndpoint,
    TaskPipeline,
    ManualConfirm,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartRollbackStep {
    pub title: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartPolicyDecision {
    pub enabled: bool,
    pub mode: SmartPolicyMode,
    pub max_risk: SmartRiskLevel,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmartPolicyMode {
    Auto,
    Confirm,
    Disabled,
}

impl SmartPolicyMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Confirm => "confirm",
            Self::Disabled => "disabled",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "confirm" => Some(Self::Confirm),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartVerificationPlan {
    pub checks: Vec<SmartVerificationCheck>,
    pub success_message: String,
    pub partial_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SmartVerificationCheck {
    pub key: String,
    pub title: String,
    pub source: SmartEvidenceSource,
    pub expected: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/smart-actions", get(list_smart_actions))
        .route("/api/v2/smart-actions/summary", get(smart_actions_summary))
        .route(
            "/api/v2/smart-actions/policies",
            get(list_smart_action_policies),
        )
        .route(
            "/api/v2/smart-actions/policies/{key}",
            put(update_smart_action_policy),
        )
        .route(
            "/api/v2/smart-actions/execute-batch",
            post(execute_smart_actions_batch),
        )
        .route("/api/v2/smart-actions/inspect", post(inspect_smart_actions))
        .route("/api/v2/smart-actions/refresh", post(refresh_smart_actions))
        .route(
            "/api/v2/smart-actions/from-task/{task_id}",
            post(smart_action_from_task),
        )
        .route(
            "/api/v2/smart-actions/from-next-action",
            post(smart_action_from_next_action),
        )
        .route("/api/v2/smart-actions/{id}", get(get_smart_action))
        .route(
            "/api/v2/smart-actions/{id}/execute",
            post(execute_smart_action),
        )
        .route(
            "/api/v2/smart-actions/{id}/dismiss",
            post(dismiss_smart_action),
        )
        .route(
            "/api/v2/smart-actions/{id}/verify",
            post(verify_smart_action),
        )
}

pub async fn reconcile_interrupted(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE smart_action_runs
         SET status = 'failed',
             error = COALESCE(error, '服务重启后中断'),
             updated_at = now()
         WHERE status IN ('queued', 'running', 'verifying')",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[utoipa::path(get, path = "/api/v2/smart-actions", tag = "smart_actions", params(SmartActionsQuery), responses((status = 200, body = SmartActionsListResponse)))]
pub async fn list_smart_actions(
    State(state): State<AppState>,
    Query(query): Query<SmartActionsQuery>,
) -> AppResult<Json<SmartActionsListResponse>> {
    let generated = generate_smart_actions(&state).await?;
    let persisted = load_persisted_smart_actions(&state).await?;
    Ok(Json(filter_smart_actions(
        merge_persisted_smart_actions(generated, persisted),
        query,
    )))
}

#[utoipa::path(get, path = "/api/v2/smart-actions/summary", tag = "smart_actions", responses((status = 200, body = SmartActionsSummaryResponse)))]
pub async fn smart_actions_summary(
    State(state): State<AppState>,
) -> AppResult<Json<SmartActionsSummaryResponse>> {
    let generated = generate_smart_actions(&state).await?;
    let persisted = load_persisted_smart_actions(&state).await?;
    let merged = merge_persisted_smart_actions(generated, persisted);
    Ok(Json(SmartActionsSummaryResponse {
        ok: merged.warnings.is_empty(),
        summary: summarize_actions(&merged.actions),
        warnings: merged.warnings,
    }))
}

#[utoipa::path(get, path = "/api/v2/smart-actions/policies", tag = "smart_actions", responses((status = 200, body = SmartActionPoliciesResponse)))]
pub async fn list_smart_action_policies(
    State(state): State<AppState>,
) -> AppResult<Json<SmartActionPoliciesResponse>> {
    Ok(Json(SmartActionPoliciesResponse {
        ok: true,
        policies: load_smart_action_policies(&state).await?,
    }))
}

#[utoipa::path(post, path = "/api/v2/smart-actions/inspect", tag = "smart_actions", request_body = SmartActionInspectRequest, responses((status = 200, body = SmartActionInspectResponse)))]
pub async fn inspect_smart_actions(
    State(state): State<AppState>,
    Json(req): Json<SmartActionInspectRequest>,
) -> AppResult<Json<SmartActionInspectResponse>> {
    let generated = generate_smart_actions(&state).await?;
    let mut actions = generated.actions;
    if let Some(items) = req.catalog_items.as_deref() {
        let now = Utc::now();
        let catalog_limit = req.limit.unwrap_or(4).clamp(1, 24);
        actions.extend(smart_actions_from_catalog_candidates(
            items,
            req.catalog_context.as_ref(),
            now,
            catalog_limit,
        ));
    }
    if let Some(subject) = req.subject.as_ref() {
        actions.retain(|action| smart_action_matches_subject(action, subject));
    }
    if let Some(q) = req.q.as_deref().and_then(non_empty_trimmed) {
        let q = q.to_lowercase();
        actions.retain(|action| smart_action_haystack(action).contains(&q));
    }
    actions.sort_by(|left, right| {
        right
            .recommendation
            .score
            .cmp(&left.recommendation.score)
            .then_with(|| right.risk.level.cmp(&left.risk.level))
            .then_with(|| left.title.cmp(&right.title))
    });
    let limit = req.limit.unwrap_or(20).clamp(1, 100);
    actions.truncate(limit);
    Ok(Json(SmartActionInspectResponse {
        ok: generated.warnings.is_empty(),
        actions,
        warnings: generated.warnings,
    }))
}

#[utoipa::path(post, path = "/api/v2/smart-actions/refresh", tag = "smart_actions", responses((status = 200, body = TaskRun)))]
pub async fn refresh_smart_actions(State(state): State<AppState>) -> AppResult<Json<TaskRun>> {
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "smart_actions_refresh",
        "刷新智能动作建议",
        4,
        "smart_actions",
        json!({}),
    )
    .await?;
    spawn_smart_actions_refresh(state, task.id);
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/smart-actions/from-task/{task_id}", tag = "smart_actions", params(("task_id" = Uuid, Path, description = "Task id to diagnose")), responses((status = 200, body = SmartActionFromTaskResponse)))]
pub async fn smart_action_from_task(
    State(state): State<AppState>,
    Path(task_id): Path<Uuid>,
) -> AppResult<Json<SmartActionFromTaskResponse>> {
    let task = sqlx::query_as::<_, TaskRun>("SELECT * FROM task_runs WHERE id = $1")
        .bind(task_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("未知任务".to_string()))?;
    let action = smart_action_from_task_run(&task, Utc::now());
    upsert_smart_action_run(
        &state,
        &action,
        SmartActionStatus::Suggested,
        Some(task.id),
        None,
        None,
    )
    .await?;
    Ok(Json(SmartActionFromTaskResponse { ok: true, action }))
}

#[utoipa::path(post, path = "/api/v2/smart-actions/from-next-action", tag = "smart_actions", request_body = SmartActionFromNextActionRequest, responses((status = 200, body = SmartActionFromNextActionResponse)))]
pub async fn smart_action_from_next_action(
    State(state): State<AppState>,
    Json(req): Json<SmartActionFromNextActionRequest>,
) -> AppResult<Json<SmartActionFromNextActionResponse>> {
    let (action, warnings) = smart_action_from_next_action_request(&req, Utc::now())?;
    let persist = req.persist.unwrap_or(true);
    if persist {
        upsert_smart_action_run(
            &state,
            &action,
            SmartActionStatus::Suggested,
            req.task_id,
            Some(json!({
                "source": "next_action",
                "source_action_id": req.source_action_id,
                "task_id": req.task_id,
                "next_action": req.next_action,
                "warnings": warnings,
            })),
            None,
        )
        .await?;
    }
    Ok(Json(SmartActionFromNextActionResponse {
        ok: true,
        action,
        persisted: persist,
        warnings,
    }))
}

#[utoipa::path(put, path = "/api/v2/smart-actions/policies/{key}", tag = "smart_actions", params(("key" = String, Path, description = "Smart action type key")), request_body = SmartActionPolicyUpdateRequest, responses((status = 200, body = SmartActionPolicyUpdateResponse)))]
pub async fn update_smart_action_policy(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(req): Json<SmartActionPolicyUpdateRequest>,
) -> AppResult<Json<SmartActionPolicyUpdateResponse>> {
    let key = normalized_policy_key(&key)?;
    let default = default_policy_record(key.as_str(), Utc::now())?;
    let enabled = req.enabled.unwrap_or(default.enabled);
    let mode = req.mode.unwrap_or(default.mode);
    let max_risk = req.max_risk.unwrap_or(default.max_risk);
    let params = req.params.unwrap_or(default.params);

    let row = sqlx::query_as::<_, (String, bool, String, String, Value, DateTime<Utc>)>(
        "INSERT INTO smart_action_policies(key, enabled, mode, max_risk, params, updated_at)
         VALUES($1, $2, $3, $4, $5, now())
         ON CONFLICT(key) DO UPDATE SET
            enabled = EXCLUDED.enabled,
            mode = EXCLUDED.mode,
            max_risk = EXCLUDED.max_risk,
            params = EXCLUDED.params,
            updated_at = now()
         RETURNING key, enabled, mode, max_risk, params, updated_at",
    )
    .bind(&key)
    .bind(enabled)
    .bind(mode.as_str())
    .bind(max_risk.as_str())
    .bind(params)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(SmartActionPolicyUpdateResponse {
        ok: true,
        policy: policy_from_row(row)?,
    }))
}

#[utoipa::path(get, path = "/api/v2/smart-actions/{id}", tag = "smart_actions", params(("id" = Uuid, Path, description = "Smart action id")), responses((status = 200, body = SmartActionDetailResponse)))]
pub async fn get_smart_action(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<SmartActionDetailResponse>> {
    let action = find_smart_action(&state, id).await?;
    Ok(Json(SmartActionDetailResponse { ok: true, action }))
}

#[utoipa::path(post, path = "/api/v2/smart-actions/{id}/execute", tag = "smart_actions", params(("id" = Uuid, Path, description = "Smart action id")), request_body = SmartActionExecuteRequest, responses((status = 200, body = SmartActionExecuteResponse)))]
pub async fn execute_smart_action(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<SmartActionExecuteRequest>,
) -> AppResult<Json<SmartActionExecuteResponse>> {
    let action = find_smart_action(&state, id).await?;
    Ok(Json(
        submit_smart_action_execution(&state, action, req).await?,
    ))
}

#[utoipa::path(post, path = "/api/v2/smart-actions/execute-batch", tag = "smart_actions", request_body = SmartActionExecuteBatchRequest, responses((status = 200, body = SmartActionExecuteBatchResponse)))]
pub async fn execute_smart_actions_batch(
    State(state): State<AppState>,
    Json(req): Json<SmartActionExecuteBatchRequest>,
) -> AppResult<Json<SmartActionExecuteBatchResponse>> {
    if req.ids.is_empty() {
        return Err(AppError::BadRequest("ids must not be empty".to_string()));
    }
    if req.ids.len() > 50 {
        return Err(AppError::BadRequest(
            "最多一次批量执行 50 个智能动作".to_string(),
        ));
    }

    let generated = generate_smart_actions(&state).await?;
    let actions_by_id = generated
        .actions
        .into_iter()
        .map(|action| (action.id, action))
        .collect::<HashMap<_, _>>();
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    for id in req.ids {
        if !seen.insert(id) {
            continue;
        }
        let Some(action) = actions_by_id.get(&id).cloned() else {
            results.push(SmartActionExecuteBatchItem {
                id,
                ok: false,
                status: None,
                task: None,
                err: Some(format!("smart action not found: {id}")),
            });
            continue;
        };
        let exec_req = SmartActionExecuteRequest {
            confirm_text: None,
            dry_run: req.dry_run,
            payload: None,
        };
        if !smart_action_is_batch_auto_ready(&action) {
            results.push(SmartActionExecuteBatchItem {
                id,
                ok: false,
                status: None,
                task: None,
                err: Some("批量执行只允许低风险 auto_ready 动作，请打开详情单独审阅".to_string()),
            });
            continue;
        }
        match submit_smart_action_execution(&state, action, exec_req).await {
            Ok(response) => results.push(SmartActionExecuteBatchItem {
                id,
                ok: true,
                status: Some(response.status),
                task: Some(response.task),
                err: None,
            }),
            Err(err) => results.push(SmartActionExecuteBatchItem {
                id,
                ok: false,
                status: None,
                task: None,
                err: Some(err.to_string()),
            }),
        }
    }

    let submitted = results.iter().filter(|item| item.ok).count();
    let failed = results.len().saturating_sub(submitted);
    Ok(Json(SmartActionExecuteBatchResponse {
        ok: failed == 0,
        total: results.len(),
        submitted,
        failed,
        results,
    }))
}

async fn submit_smart_action_execution(
    state: &AppState,
    action: SmartAction,
    req: SmartActionExecuteRequest,
) -> AppResult<SmartActionExecuteResponse> {
    let id = action.id;
    ensure_action_executable(&action, &req)?;
    let dry_run = req.dry_run.unwrap_or(false);
    let payload = req.payload.clone();

    let task = tasks::insert_task_with_meta(
        &state.pool,
        "smart_action_execute",
        &format!("智能动作: {}", action.title),
        (action.plan.steps.len() as i64 + 1).max(1),
        "smart_actions",
        json!({
            "action_id": action.id,
            "action_type": action.action_type.as_str(),
            "subject": action.subject,
            "dry_run": dry_run,
            "payload": payload,
        }),
    )
    .await?;
    upsert_smart_action_run(
        state,
        &action,
        SmartActionStatus::Queued,
        Some(task.id),
        None,
        None,
    )
    .await?;
    spawn_smart_action_execute(state.clone(), task.id, action, dry_run, req.payload);

    Ok(SmartActionExecuteResponse {
        ok: true,
        id,
        status: SmartActionStatus::Queued,
        task,
        message: "已提交智能动作任务".to_string(),
    })
}

#[utoipa::path(post, path = "/api/v2/smart-actions/{id}/dismiss", tag = "smart_actions", params(("id" = Uuid, Path, description = "Smart action id")), request_body = SmartActionDismissRequest, responses((status = 200, body = SmartActionDismissResponse)))]
pub async fn dismiss_smart_action(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<SmartActionDismissRequest>,
) -> AppResult<Json<SmartActionDismissResponse>> {
    let action = find_smart_action(&state, id).await?;
    upsert_smart_action_run(
        &state,
        &action,
        SmartActionStatus::Dismissed,
        None,
        Some(json!({ "reason": req.reason.unwrap_or_default() })),
        None,
    )
    .await?;
    Ok(Json(SmartActionDismissResponse {
        ok: true,
        id,
        status: SmartActionStatus::Dismissed,
    }))
}

#[utoipa::path(post, path = "/api/v2/smart-actions/{id}/verify", tag = "smart_actions", params(("id" = Uuid, Path, description = "Smart action id")), responses((status = 200, body = SmartActionVerifyResponse)))]
pub async fn verify_smart_action(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<SmartActionVerifyResponse>> {
    let generated = generate_smart_actions(&state).await?;
    let action = generated.actions.into_iter().find(|action| action.id == id);
    let live_signal_present = action.is_some();
    let action = match action {
        Some(action) => Some(action),
        None => load_persisted_smart_action(&state, id).await?,
    };
    let (status, result, warnings) = if let Some(action) = action {
        let evidence = load_smart_action_verify_evidence(&state, &action).await?;
        let status =
            smart_action_status_from_verify_evidence(&action, live_signal_present, &evidence);
        let result =
            smart_action_verification_result_with_evidence(&action, status, false, &evidence);
        let warnings = smart_action_verify_warnings_with_evidence(&action, status, &evidence);
        upsert_smart_action_run(&state, &action, status, None, Some(result.clone()), None).await?;
        (status, result, warnings)
    } else {
        let result = json!({
            "action_id": id,
            "verification": {
                "status": "done",
                "message": "当前刷新结果中已找不到该建议，视为通过验收。"
            },
        });
        update_smart_action_run_status(
            &state,
            id,
            SmartActionStatus::Done,
            Some(result.clone()),
            None,
        )
        .await?;
        (SmartActionStatus::Done, result, Vec::new())
    };
    Ok(Json(SmartActionVerifyResponse {
        ok: warnings.is_empty(),
        id,
        status,
        result,
        warnings,
    }))
}

#[derive(Debug)]
struct GeneratedSmartActions {
    actions: Vec<SmartAction>,
    warnings: Vec<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct PersistedSmartActionRow {
    id: Uuid,
    action_type: String,
    status: String,
    subject: Value,
    title: String,
    summary: String,
    recommendation: Value,
    evidence: Value,
    plan: Value,
    risk: Value,
    policy: Value,
    verification: Value,
    source: String,
    tab: String,
    action_label: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct SmartActionRunEvidenceRow {
    status: String,
    task_id: Option<Uuid>,
    result: Option<Value>,
    error: Option<String>,
    source: String,
    tab: String,
    action_label: String,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
struct SmartActionVerifyEvidence {
    collected: bool,
    persisted_status: Option<SmartActionStatus>,
    persisted_task_id: Option<Uuid>,
    persisted_result: Option<Value>,
    persisted_error: Option<String>,
    persisted_source: Option<String>,
    persisted_tab: Option<String>,
    persisted_action_label: Option<String>,
    persisted_updated_at: Option<DateTime<Utc>>,
    task_runs: Vec<VerifyTaskRunEvidence>,
    audit_logs: Vec<VerifyAuditLogEvidence>,
    app_logs: Vec<VerifyAppLogEvidence>,
}

#[derive(Debug, Clone, Serialize)]
struct VerifyTaskRunEvidence {
    id: Uuid,
    kind: String,
    label: String,
    source: String,
    params: Value,
    status: String,
    progress: i64,
    total: i64,
    status_text: String,
    result: Option<Value>,
    error: Option<String>,
    queued_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
struct VerifyAuditLogEvidence {
    id: i64,
    actor: String,
    action: String,
    detail: Value,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
struct VerifyAppLogEvidence {
    id: i64,
    level: String,
    message: String,
    detail: Value,
    created_at: DateTime<Utc>,
}

impl From<TaskRun> for VerifyTaskRunEvidence {
    fn from(task: TaskRun) -> Self {
        Self {
            id: task.id,
            kind: task.kind,
            label: task.label,
            source: task.source,
            params: task.params,
            status: task.status,
            progress: task.progress,
            total: task.total,
            status_text: task.status_text,
            result: task.result,
            error: task.error,
            queued_at: task.queued_at,
            started_at: task.started_at,
            ended_at: task.ended_at,
            updated_at: task.updated_at,
        }
    }
}

async fn generate_smart_actions(state: &AppState) -> AppResult<GeneratedSmartActions> {
    let Json(dashboard) = dashboard::dashboard_smart_actions(State(state.clone())).await?;
    let now = Utc::now();
    let mut actions = dashboard
        .actions
        .iter()
        .map(|action| smart_action_from_dashboard(action, now))
        .collect::<Vec<_>>();
    let mut warnings = dashboard.warnings;
    match collect_poster_smart_actions(state, now).await {
        Ok(mut poster_actions) => actions.append(&mut poster_actions),
        Err(err) => warnings.push(format!("poster collector skipped: {err}")),
    }
    match collect_zhuigeng_smart_actions(state, now).await {
        Ok(mut zhuigeng_actions) => actions.append(&mut zhuigeng_actions),
        Err(err) => warnings.push(format!("zhuigeng collector skipped: {err}")),
    }
    match collect_dedup_smart_actions(state, now).await {
        Ok(mut dedup_actions) => actions.append(&mut dedup_actions),
        Err(err) => warnings.push(format!("dedup collector skipped: {err}")),
    }
    apply_policy_overrides(state, &mut actions).await?;
    apply_persisted_action_state(state, &mut actions).await?;
    Ok(GeneratedSmartActions { actions, warnings })
}

async fn collect_poster_smart_actions(
    state: &AppState,
    now: DateTime<Utc>,
) -> AppResult<Vec<SmartAction>> {
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        return Ok(Vec::new());
    }
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let response = posters::detect_mismatched_posters(
        &client,
        PosterDetectRequest {
            lib: None,
            limit: Some(POSTER_ACTION_SCAN_LIMIT),
            include_missing_primary: Some(true),
        },
    )
    .await
    .map_err(AppError::Anyhow)?;
    Ok(response
        .items
        .iter()
        .filter(|item| !item.id.trim().is_empty())
        .take(POSTER_ACTION_MAX_ITEMS)
        .map(|item| smart_action_from_poster_signal(item, now))
        .collect())
}

async fn collect_zhuigeng_smart_actions(
    state: &AppState,
    now: DateTime<Utc>,
) -> AppResult<Vec<SmartAction>> {
    let workbench = zhuigeng::zhuigeng_workbench_for_state(state).await?;
    Ok(workbench
        .rows
        .iter()
        .filter(|row| {
            matches!(
                row.lane,
                ZhuigengWorkbenchLane::UpdateNeeded
                    | ZhuigengWorkbenchLane::CompleteAfterUpdate
                    | ZhuigengWorkbenchLane::ArchiveReady
            )
        })
        .map(|row| smart_action_from_zhuigeng_row(row, now))
        .collect())
}

async fn collect_dedup_smart_actions(
    state: &AppState,
    now: DateTime<Utc>,
) -> AppResult<Vec<SmartAction>> {
    let analysis = dedup::analyze_duplicate_groups_for_state(state).await?;
    let mut actions = analysis
        .dups
        .iter()
        .take(DEDUP_ACTION_MAX_GROUPS)
        .map(|group| smart_action_from_dedup_group(group, now))
        .collect::<Vec<_>>();
    actions.extend(
        analysis
            .review
            .iter()
            .take(DEDUP_ACTION_MAX_GROUPS)
            .map(|group| smart_action_from_dedup_review_group(group, now)),
    );
    Ok(actions)
}

async fn find_generated_action(state: &AppState, id: Uuid) -> AppResult<SmartAction> {
    let generated = generate_smart_actions(state).await?;
    generated
        .actions
        .into_iter()
        .find(|action| action.id == id)
        .ok_or_else(|| AppError::NotFound(format!("smart action not found: {id}")))
}

async fn find_smart_action(state: &AppState, id: Uuid) -> AppResult<SmartAction> {
    match find_generated_action(state, id).await {
        Ok(action) => Ok(action),
        Err(AppError::NotFound(_)) => load_persisted_smart_action(state, id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("smart action not found: {id}"))),
        Err(err) => Err(err),
    }
}

async fn load_persisted_smart_action(state: &AppState, id: Uuid) -> AppResult<Option<SmartAction>> {
    let row = sqlx::query_as::<_, PersistedSmartActionRow>(
        "SELECT id, action_type, status, subject, title, summary, recommendation, evidence,
                plan, risk, policy, verification, source, tab, action_label, created_at, updated_at
         FROM smart_action_runs
         WHERE id = $1
           AND (expires_at IS NULL OR expires_at > now())",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    row.map(smart_action_from_persisted_row).transpose()
}

async fn load_persisted_smart_actions(state: &AppState) -> AppResult<Vec<SmartAction>> {
    let rows = sqlx::query_as::<_, PersistedSmartActionRow>(
        "SELECT id, action_type, status, subject, title, summary, recommendation, evidence,
                plan, risk, policy, verification, source, tab, action_label, created_at, updated_at
         FROM smart_action_runs
         WHERE expires_at IS NULL OR expires_at > now()
         ORDER BY updated_at DESC",
    )
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter()
        .map(smart_action_from_persisted_row)
        .collect()
}

async fn load_smart_action_verify_evidence(
    state: &AppState,
    action: &SmartAction,
) -> AppResult<SmartActionVerifyEvidence> {
    let mut evidence = SmartActionVerifyEvidence {
        collected: true,
        ..SmartActionVerifyEvidence::default()
    };
    if let Some(row) = sqlx::query_as::<_, SmartActionRunEvidenceRow>(
        "SELECT status, task_id, result, error, source, tab, action_label, updated_at
         FROM smart_action_runs
         WHERE id = $1",
    )
    .bind(action.id)
    .fetch_optional(&state.pool)
    .await?
    {
        evidence.persisted_status = SmartActionStatus::from_str(&row.status);
        evidence.persisted_task_id = row.task_id;
        evidence.persisted_result = row.result;
        evidence.persisted_error = row.error;
        evidence.persisted_source = Some(row.source);
        evidence.persisted_tab = Some(row.tab);
        evidence.persisted_action_label = Some(row.action_label);
        evidence.persisted_updated_at = Some(row.updated_at);
    }

    let action_id = action.id.to_string();
    let task_kinds = related_verify_task_kinds(action.action_type);
    let subject_token = verify_subject_token(action);
    let subject_pattern = subject_token
        .as_deref()
        .map(|token| format!("%{token}%"))
        .unwrap_or_default();
    let tasks = sqlx::query_as::<_, TaskRun>(
        "SELECT *
         FROM task_runs
         WHERE ($1::uuid IS NOT NULL AND id = $1)
            OR params->>'action_id' = $2
            OR (
                updated_at > now() - interval '24 hours'
                AND (
                    (source = 'smart_actions' AND kind = ANY($3) AND params->>'action_type' = $4)
                    OR (
                        kind = ANY($3)
                        AND $5::text <> ''
                        AND (label ILIKE $6 OR params::text ILIKE $6)
                    )
                )
            )
         ORDER BY updated_at DESC
         LIMIT 5",
    )
    .bind(evidence.persisted_task_id)
    .bind(&action_id)
    .bind(&task_kinds)
    .bind(action.action_type.as_str())
    .bind(subject_token.as_deref().unwrap_or(""))
    .bind(&subject_pattern)
    .fetch_all(&state.pool)
    .await?;
    evidence.task_runs = tasks.into_iter().map(VerifyTaskRunEvidence::from).collect();

    let action_pattern = format!("%{action_id}%");
    let type_pattern = format!("%{}%", action.action_type.as_str());
    let title_pattern = format!("%{}%", action.title);
    let audit_logs = sqlx::query_as::<_, VerifyAuditLogEvidence>(
        "SELECT id, actor, action, detail, created_at
         FROM audit_logs
         WHERE created_at > now() - interval '24 hours'
           AND (
                detail::text ILIKE $1
                OR action ILIKE $2
                OR detail::text ILIKE $2
                OR action ILIKE $3
                OR detail::text ILIKE $3
           )
         ORDER BY created_at DESC
         LIMIT 5",
    )
    .bind(&action_pattern)
    .bind(&type_pattern)
    .bind(&title_pattern)
    .fetch_all(&state.pool)
    .await?;
    evidence.audit_logs = audit_logs;

    let app_logs = sqlx::query_as::<_, VerifyAppLogEvidence>(
        "SELECT id, level, message, detail, created_at
         FROM app_logs
         WHERE created_at > now() - interval '24 hours'
           AND (
                detail::text ILIKE $1
                OR message ILIKE $2
                OR detail::text ILIKE $2
                OR message ILIKE $3
                OR detail::text ILIKE $3
           )
         ORDER BY created_at DESC
         LIMIT 5",
    )
    .bind(&action_pattern)
    .bind(&type_pattern)
    .bind(&title_pattern)
    .fetch_all(&state.pool)
    .await?;
    evidence.app_logs = app_logs;

    Ok(evidence)
}

fn related_verify_task_kinds(action_type: SmartActionType) -> Vec<String> {
    let kinds = match action_type {
        SmartActionType::TransferAddNew => [
            "smart_action_execute",
            "catalog_transfer_execute",
            "wizard_add_new",
        ]
        .as_slice(),
        SmartActionType::TransferUpdateSeries => {
            ["smart_action_execute", "zhuigeng_update", "wizard_add_new"].as_slice()
        }
        SmartActionType::DedupRemoveOld | SmartActionType::DedupReview => {
            ["smart_action_execute", "dedup_exec_batch", "replace_batch"].as_slice()
        }
        SmartActionType::ArchiveSeries => {
            ["smart_action_execute", "zhuigeng_archive", "move_batch"].as_slice()
        }
        SmartActionType::PosterFix => {
            ["smart_action_execute", "poster_fix_batch", "poster_detect"].as_slice()
        }
        SmartActionType::MetadataRefresh | SmartActionType::LibraryScan => {
            ["smart_action_execute", "scan_library", "scan_all"].as_slice()
        }
        SmartActionType::CleanupEmptyFolder => {
            ["smart_action_execute", "cleanup_empty", "cleanup"].as_slice()
        }
        SmartActionType::TaskRetryOrDiagnose => ["smart_action_execute"].as_slice(),
    };
    kinds.iter().map(|kind| (*kind).to_string()).collect()
}

fn verify_subject_token(action: &SmartAction) -> Option<String> {
    [
        action.subject.folder.as_deref(),
        Some(action.subject.name.as_str()),
        action.subject.tmdb.as_deref(),
        action.subject.emby_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter_map(non_empty_trimmed)
    .max_by_key(|value| value.chars().count())
    .map(ToString::to_string)
}

fn merge_persisted_smart_actions(
    mut generated: GeneratedSmartActions,
    persisted: Vec<SmartAction>,
) -> GeneratedSmartActions {
    let mut persisted_by_id = persisted
        .into_iter()
        .map(|action| (action.id, action))
        .collect::<HashMap<_, _>>();
    for action in &mut generated.actions {
        if let Some(persisted) = persisted_by_id.remove(&action.id) {
            action.status = persisted.status;
            action.updated_at = persisted.updated_at;
        }
    }
    generated.actions.extend(persisted_by_id.into_values());
    generated
}

fn smart_action_from_persisted_row(row: PersistedSmartActionRow) -> AppResult<SmartAction> {
    let action_type = SmartActionType::from_str(&row.action_type).ok_or_else(|| {
        AppError::BadRequest(format!(
            "invalid persisted smart action type: {}",
            row.action_type
        ))
    })?;
    let status = SmartActionStatus::from_str(&row.status).ok_or_else(|| {
        AppError::BadRequest(format!(
            "invalid persisted smart action status: {}",
            row.status
        ))
    })?;
    Ok(SmartAction {
        id: row.id,
        action_type,
        status,
        subject: from_persisted_json(row.subject, "subject")?,
        title: row.title,
        summary: row.summary,
        recommendation: from_persisted_json(row.recommendation, "recommendation")?,
        evidence: from_persisted_json(row.evidence, "evidence")?,
        plan: from_persisted_json(row.plan, "plan")?,
        risk: from_persisted_json(row.risk, "risk")?,
        policy: from_persisted_json(row.policy, "policy")?,
        verification: from_persisted_json(row.verification, "verification")?,
        source: non_empty_trimmed(&row.source)
            .map(ToString::to_string)
            .unwrap_or_else(|| "smart_action_runs".to_string()),
        tab: non_empty_trimmed(&row.tab)
            .map(ToString::to_string)
            .unwrap_or_else(|| persisted_tab_from_action_type(action_type)),
        action_label: non_empty_trimmed(&row.action_label)
            .map(ToString::to_string)
            .unwrap_or_else(|| persisted_action_label(action_type)),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn from_persisted_json<T: DeserializeOwned>(value: Value, field: &str) -> AppResult<T> {
    serde_json::from_value(value)
        .map_err(|err| AppError::BadRequest(format!("smart action {field} 快照无效：{err}")))
}

fn persisted_tab_from_action_type(action_type: SmartActionType) -> String {
    match action_type {
        SmartActionType::TransferAddNew => "catalog",
        SmartActionType::TransferUpdateSeries | SmartActionType::ArchiveSeries => "zhuigeng",
        SmartActionType::DedupRemoveOld | SmartActionType::DedupReview => "dedup",
        SmartActionType::PosterFix | SmartActionType::MetadataRefresh => "posters",
        SmartActionType::TaskRetryOrDiagnose => "tasks",
        SmartActionType::CleanupEmptyFolder => "cleanup",
        SmartActionType::LibraryScan => "scan",
    }
    .to_string()
}

fn persisted_action_label(action_type: SmartActionType) -> String {
    match action_type {
        SmartActionType::TransferAddNew => "打开找资源".to_string(),
        SmartActionType::TransferUpdateSeries => "追更更新".to_string(),
        SmartActionType::DedupRemoveOld => "删除旧资源".to_string(),
        SmartActionType::DedupReview => "复核重复资源".to_string(),
        SmartActionType::PosterFix => "修复海报".to_string(),
        SmartActionType::MetadataRefresh => "刷新元数据".to_string(),
        SmartActionType::LibraryScan => "刷新媒体库".to_string(),
        SmartActionType::ArchiveSeries => "归档".to_string(),
        SmartActionType::CleanupEmptyFolder => "清理空目录".to_string(),
        SmartActionType::TaskRetryOrDiagnose => "查看诊断".to_string(),
    }
}

async fn apply_persisted_action_state(
    state: &AppState,
    actions: &mut [SmartAction],
) -> AppResult<()> {
    if actions.is_empty() {
        return Ok(());
    }
    let ids = actions.iter().map(|action| action.id).collect::<Vec<_>>();
    let rows = sqlx::query_as::<_, (Uuid, String, DateTime<Utc>)>(
        "SELECT id, status, updated_at
         FROM smart_action_runs
         WHERE id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(&state.pool)
    .await?;

    for (id, status, updated_at) in rows {
        let Some(status) = SmartActionStatus::from_str(&status) else {
            continue;
        };
        if let Some(action) = actions.iter_mut().find(|action| action.id == id) {
            action.status = status;
            action.updated_at = updated_at;
        }
    }
    Ok(())
}

async fn apply_policy_overrides(state: &AppState, actions: &mut [SmartAction]) -> AppResult<()> {
    if actions.is_empty() {
        return Ok(());
    }
    let policies = load_smart_action_policies(state)
        .await?
        .into_iter()
        .map(|policy| (policy.key.clone(), policy))
        .collect::<HashMap<_, _>>();
    for action in actions {
        if let Some(policy) = policies.get(action.action_type.as_str()) {
            action.policy = SmartPolicyDecision {
                enabled: policy.enabled,
                mode: policy.mode,
                max_risk: policy.max_risk,
                reason: format!("策略来自 smart_action_policies: {}", policy.key),
            };
        }
    }
    Ok(())
}

async fn load_smart_action_policies(state: &AppState) -> AppResult<Vec<SmartActionPolicy>> {
    let rows = sqlx::query_as::<_, (String, bool, String, String, Value, DateTime<Utc>)>(
        "SELECT key, enabled, mode, max_risk, params, updated_at FROM smart_action_policies",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut policies = SmartActionType::all()
        .iter()
        .map(|action_type| default_policy_record(action_type.as_str(), Utc::now()))
        .collect::<AppResult<Vec<_>>>()?;
    let mut by_key = rows
        .into_iter()
        .map(policy_from_row)
        .collect::<AppResult<Vec<_>>>()?
        .into_iter()
        .map(|policy| (policy.key.clone(), policy))
        .collect::<HashMap<_, _>>();
    for policy in &mut policies {
        if let Some(override_policy) = by_key.remove(&policy.key) {
            *policy = override_policy;
        }
    }
    policies.extend(by_key.into_values());
    policies.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(policies)
}

fn default_policy_record(key: &str, updated_at: DateTime<Utc>) -> AppResult<SmartActionPolicy> {
    let action_type = SmartActionType::from_str(key)
        .ok_or_else(|| AppError::BadRequest(format!("unknown smart action policy key: {key}")))?;
    let default = smart_policy(action_type, smart_risk(action_type).level);
    Ok(SmartActionPolicy {
        key: key.to_string(),
        enabled: default.enabled,
        mode: default.mode,
        max_risk: default.max_risk,
        params: json!({}),
        updated_at,
    })
}

fn policy_from_row(
    row: (String, bool, String, String, Value, DateTime<Utc>),
) -> AppResult<SmartActionPolicy> {
    let (key, enabled, mode, max_risk, params, updated_at) = row;
    Ok(SmartActionPolicy {
        key,
        enabled,
        mode: SmartPolicyMode::from_str(&mode)
            .ok_or_else(|| AppError::BadRequest(format!("invalid smart policy mode: {mode}")))?,
        max_risk: SmartRiskLevel::from_str(&max_risk).ok_or_else(|| {
            AppError::BadRequest(format!("invalid smart policy risk: {max_risk}"))
        })?,
        params,
        updated_at,
    })
}

fn normalized_policy_key(key: &str) -> AppResult<String> {
    let key = key.trim().to_ascii_lowercase();
    if SmartActionType::from_str(&key).is_none() {
        return Err(AppError::BadRequest(format!(
            "unknown smart action policy key: {key}"
        )));
    }
    Ok(key)
}

fn ensure_action_executable(
    action: &SmartAction,
    req: &SmartActionExecuteRequest,
) -> AppResult<()> {
    if !action.policy.enabled {
        return Err(AppError::Conflict("该智能动作策略已禁用".to_string()));
    }
    if action.policy.mode == SmartPolicyMode::Disabled {
        return Err(AppError::Conflict("该智能动作策略已禁用".to_string()));
    }
    if action.risk.level > action.policy.max_risk {
        return Err(AppError::Conflict(format!(
            "动作风险 {} 超过策略允许的 {}",
            action.risk.level.as_str(),
            action.policy.max_risk.as_str()
        )));
    }
    if action.action_type == SmartActionType::DedupReview {
        return Err(AppError::Conflict(
            "该智能动作只用于人工复核，不接入删除执行器".to_string(),
        ));
    }
    if let Some(expected) = action.risk.requires_confirm_text.as_deref() {
        let actual = req.confirm_text.as_deref().unwrap_or("").trim();
        if actual != expected {
            return Err(AppError::BadRequest(format!(
                "需要输入确认文本：{expected}"
            )));
        }
        validate_confirm_payload(action, req.payload.as_ref())?;
    }
    if action.policy.mode == SmartPolicyMode::Confirm {
        if !supports_confirm_execution(action.action_type) {
            return Err(AppError::Conflict(
                "该智能动作目前只支持生成复核建议，不支持直接执行".to_string(),
            ));
        }
        return Ok(());
    }
    if action.policy.mode != SmartPolicyMode::Auto {
        return Err(AppError::Conflict("该智能动作策略不允许执行".to_string()));
    }
    if action.risk.requires_confirm_text.is_some() {
        return Err(AppError::Conflict(
            "高风险智能动作必须走人工确认，不能自动执行".to_string(),
        ));
    }
    if !supports_auto_execution(action.action_type) {
        return Err(AppError::Conflict("该智能动作尚未接入执行器".to_string()));
    }
    Ok(())
}

fn supports_auto_execution(action_type: SmartActionType) -> bool {
    matches!(
        action_type,
        SmartActionType::PosterFix
            | SmartActionType::MetadataRefresh
            | SmartActionType::LibraryScan
    )
}

fn smart_action_is_batch_auto_ready(action: &SmartAction) -> bool {
    action.policy.enabled
        && action.policy.mode == SmartPolicyMode::Auto
        && action.risk.level == SmartRiskLevel::Low
        && action.risk.level <= action.policy.max_risk
        && action.risk.requires_confirm_text.is_none()
        && supports_auto_execution(action.action_type)
}

fn supports_confirm_execution(action_type: SmartActionType) -> bool {
    matches!(
        action_type,
        SmartActionType::TransferAddNew
            | SmartActionType::TransferUpdateSeries
            | SmartActionType::ArchiveSeries
            | SmartActionType::DedupRemoveOld
    )
}

fn validate_confirm_payload(action: &SmartAction, payload: Option<&Value>) -> AppResult<()> {
    match action.action_type {
        SmartActionType::TransferAddNew => {
            let step = action_step(action, "catalog_transfer_execute")?;
            let _: CatalogTransferExecuteRequest = catalog_transfer_request(step, payload)?;
        }
        SmartActionType::TransferUpdateSeries => {
            let step = action_step(action, "zhuigeng_update_execute")?;
            let _: ZhuigengUpdateExecuteRequest = zhuigeng_update_request(step, payload)?;
        }
        SmartActionType::ArchiveSeries => {
            let step = action_step(action, "zhuigeng_archive_execute")?;
            let _: ZhuigengArchiveExecuteRequest = zhuigeng_archive_request(step, payload)?;
        }
        SmartActionType::DedupRemoveOld => {
            let step = action_step(action, "dedup_execute_batch")?;
            let _: DedupExecuteBatchRequest = dedup_batch_request(step, payload)?;
        }
        _ => {}
    }
    Ok(())
}

async fn upsert_smart_action_run(
    state: &AppState,
    action: &SmartAction,
    status: SmartActionStatus,
    task_id: Option<Uuid>,
    result: Option<Value>,
    error: Option<String>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO smart_action_runs(
            id, action_type, status, subject, title, summary, recommendation, evidence,
            plan, risk, policy, verification, source, tab, action_label, task_id, result, error,
            updated_at
         )
         VALUES(
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18,
            now()
         )
         ON CONFLICT(id) DO UPDATE SET
            action_type = EXCLUDED.action_type,
            status = EXCLUDED.status,
            subject = EXCLUDED.subject,
            title = EXCLUDED.title,
            summary = EXCLUDED.summary,
            recommendation = EXCLUDED.recommendation,
            evidence = EXCLUDED.evidence,
            plan = EXCLUDED.plan,
            risk = EXCLUDED.risk,
            policy = EXCLUDED.policy,
            verification = EXCLUDED.verification,
            source = EXCLUDED.source,
            tab = EXCLUDED.tab,
            action_label = EXCLUDED.action_label,
            task_id = COALESCE(EXCLUDED.task_id, smart_action_runs.task_id),
            result = EXCLUDED.result,
            error = EXCLUDED.error,
            updated_at = now()",
    )
    .bind(action.id)
    .bind(action.action_type.as_str())
    .bind(status.as_str())
    .bind(to_json(&action.subject)?)
    .bind(&action.title)
    .bind(&action.summary)
    .bind(to_json(&action.recommendation)?)
    .bind(to_json(&action.evidence)?)
    .bind(to_json(&action.plan)?)
    .bind(to_json(&action.risk)?)
    .bind(to_json(&action.policy)?)
    .bind(to_json(&action.verification)?)
    .bind(&action.source)
    .bind(&action.tab)
    .bind(&action.action_label)
    .bind(task_id)
    .bind(result)
    .bind(error)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn update_smart_action_run_status(
    state: &AppState,
    id: Uuid,
    status: SmartActionStatus,
    result: Option<Value>,
    error: Option<String>,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE smart_action_runs
         SET status = $2, result = COALESCE($3, result), error = $4, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(status.as_str())
    .bind(result)
    .bind(error)
    .execute(&state.pool)
    .await?;
    Ok(())
}

fn spawn_smart_actions_refresh(state: AppState, task_id: Uuid) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, task_id, "任务并发槽不可用", None).await;
            return;
        };
        match run_smart_actions_refresh_for_task(&state, task_id).await {
            Ok(result) => {
                let total = result.get("total").and_then(Value::as_u64).unwrap_or(0);
                let persisted = result.get("persisted").and_then(Value::as_u64).unwrap_or(0);
                let message = format!(
                    "智能动作刷新完成: {} 条建议，持久化 {} 条",
                    total, persisted
                );
                let _ =
                    tasks::finish_done_with_message(&state.pool, task_id, &message, result).await;
            }
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, task_id, &err.to_string(), None).await;
            }
        }
    });
}

pub async fn run_smart_actions_refresh_for_task(
    state: &AppState,
    task_id: Uuid,
) -> AppResult<Value> {
    tasks::mark_running(&state.pool, task_id, "采集智能动作信号").await?;
    tasks::set_progress(&state.pool, task_id, 1, "读取仪表盘和对象级信号").await?;
    let generated = generate_smart_actions(state).await?;
    tasks::set_progress(&state.pool, task_id, 2, "持久化建议状态").await?;
    let mut persisted = 0usize;
    for action in &generated.actions {
        if action.status != SmartActionStatus::Suggested {
            continue;
        }
        if upsert_smart_action_run(
            state,
            action,
            SmartActionStatus::Suggested,
            None,
            None,
            None,
        )
        .await
        .is_ok()
        {
            persisted += 1;
        }
    }
    tasks::set_progress(&state.pool, task_id, 3, "汇总刷新结果").await?;
    let summary = summarize_actions(&generated.actions);
    Ok(json!({
        "ok": generated.warnings.is_empty(),
        "total": generated.actions.len(),
        "persisted": persisted,
        "warnings": generated.warnings,
        "summary": summary,
    }))
}

fn spawn_smart_action_execute(
    state: AppState,
    task_id: Uuid,
    action: SmartAction,
    dry_run: bool,
    payload: Option<Value>,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let message = "任务并发槽不可用";
            let _ = update_smart_action_run_status(
                &state,
                action.id,
                SmartActionStatus::Failed,
                None,
                Some(message.to_string()),
            )
            .await;
            let _ = tasks::finish_error(&state.pool, task_id, message, None).await;
            return;
        };
        let _ = tasks::mark_running(&state.pool, task_id, "智能动作执行中").await;
        let _ = update_smart_action_run_status(
            &state,
            action.id,
            SmartActionStatus::Running,
            None,
            None,
        )
        .await;

        let mut step_results = Vec::new();
        let mut execution_outputs = Vec::new();
        let mut fatal_error: Option<String> = None;
        for (index, step) in action.plan.steps.iter().enumerate() {
            if tasks::cancel_requested(&state.pool, task_id).await {
                let _ = update_smart_action_run_status(
                    &state,
                    action.id,
                    SmartActionStatus::Cancelled,
                    Some(json!({ "cancelled_at_step": step.key })),
                    None,
                )
                .await;
                let _ = tasks::finish_cancelled(&state.pool, task_id).await;
                return;
            }
            let _ = tasks::set_progress(
                &state.pool,
                task_id,
                index as i64,
                &format!("执行: {}", step.title),
            )
            .await;
            let output =
                execute_smart_action_step(&state, &action, step, dry_run, payload.as_ref()).await;
            let (status, message, result) = match output {
                Ok(result) => {
                    execution_outputs.push(result.clone());
                    (
                        "done",
                        smart_action_step_message(&action, step, dry_run),
                        result,
                    )
                }
                Err(err) => {
                    let message = format!("{}：{err}", step.title);
                    let result = json!({ "ok": false, "err": err.to_string() });
                    execution_outputs.push(result.clone());
                    fatal_error.get_or_insert_with(|| message.clone());
                    ("error", message, result)
                }
            };
            step_results.push(json!({
                "key": step.key,
                "title": step.title,
                "executor": step.executor,
                "status": status,
                "message": message,
                "result": result
            }));
            if fatal_error.is_some() {
                break;
            }
        }

        let final_status = smart_action_execution_status(
            &action,
            dry_run,
            fatal_error.is_some(),
            &execution_outputs,
        );
        let result = smart_action_task_result(
            &action,
            final_status,
            dry_run,
            step_results,
            execution_outputs,
        );
        let message = match final_status {
            SmartActionStatus::Done => "智能动作已完成并通过验收",
            SmartActionStatus::Partial => "智能动作已执行，仍需人工复查",
            _ => "智能动作已结束",
        };
        let _ = update_smart_action_run_status(
            &state,
            action.id,
            final_status,
            Some(result.clone()),
            fatal_error.clone(),
        )
        .await;
        if let Some(error) = fatal_error {
            let _ = tasks::finish_error(&state.pool, task_id, &error, Some(result)).await;
        } else {
            let _ = tasks::finish_done_with_message(&state.pool, task_id, message, result).await;
        }
    });
}

async fn execute_smart_action_step(
    state: &AppState,
    action: &SmartAction,
    step: &SmartExecutionStep,
    dry_run: bool,
    payload: Option<&Value>,
) -> AppResult<Value> {
    if dry_run {
        return Ok(json!({
            "ok": true,
            "dry_run": true,
            "params": step.params,
        }));
    }
    match step.key.as_str() {
        "execute_low_risk" => match action.action_type {
            SmartActionType::PosterFix => execute_poster_fix_action(state, action).await,
            SmartActionType::MetadataRefresh | SmartActionType::LibraryScan => {
                execute_emby_refresh_action(state, action).await
            }
            _ => Ok(json!({
                "ok": false,
                "skipped": true,
                "reason": "该动作类型尚未接入真实低风险执行器",
            })),
        },
        "zhuigeng_update_execute" => {
            execute_zhuigeng_update_action(state, action, step, payload).await
        }
        "zhuigeng_archive_execute" => {
            execute_zhuigeng_archive_action(state, action, step, payload).await
        }
        "dedup_execute_batch" => execute_dedup_batch_action(state, step, payload).await,
        "catalog_transfer_execute" if action.action_type == SmartActionType::TransferAddNew => {
            execute_catalog_transfer_action(state, step, payload).await
        }
        _ => Ok(json!({
            "ok": true,
            "dry_run": false,
            "params": step.params,
        })),
    }
}

async fn execute_zhuigeng_update_action(
    state: &AppState,
    _action: &SmartAction,
    step: &SmartExecutionStep,
    payload: Option<&Value>,
) -> AppResult<Value> {
    let req = zhuigeng_update_request(step, payload)?;
    let task = zhuigeng::zhuigeng_update_execute_for_state(state.clone(), req).await?;
    Ok(json!({
        "ok": true,
        "module": "zhuigeng",
        "function": "zhuigeng_update_execute_for_state",
        "task": task,
    }))
}

async fn execute_zhuigeng_archive_action(
    state: &AppState,
    _action: &SmartAction,
    step: &SmartExecutionStep,
    payload: Option<&Value>,
) -> AppResult<Value> {
    let req = zhuigeng_archive_request(step, payload)?;
    let response = zhuigeng::zhuigeng_archive_execute_for_state(state.clone(), req).await?;
    Ok(json!({
        "ok": response.ok,
        "module": "zhuigeng",
        "function": "zhuigeng_archive_execute_for_state",
        "result": response,
    }))
}

async fn execute_dedup_batch_action(
    state: &AppState,
    step: &SmartExecutionStep,
    payload: Option<&Value>,
) -> AppResult<Value> {
    let req = dedup_batch_request(step, payload)?;
    let task = dedup::dedup_execute_batch_for_state(
        state.clone(),
        req,
        "smart_actions",
        Some("智能动作批量去重"),
    )
    .await?;
    Ok(json!({
        "ok": true,
        "module": "dedup",
        "function": "dedup_execute_batch_for_state",
        "task": task,
    }))
}

async fn execute_catalog_transfer_action(
    state: &AppState,
    step: &SmartExecutionStep,
    payload: Option<&Value>,
) -> AppResult<Value> {
    let req = catalog_transfer_request(step, payload)?;
    let task =
        catalog::catalog_transfer_execute_for_state(state.clone(), req, "smart_actions").await?;
    Ok(json!({
        "ok": true,
        "module": "catalog",
        "function": "catalog_transfer_execute_for_state",
        "task": task,
    }))
}

fn action_step<'a>(action: &'a SmartAction, key: &str) -> AppResult<&'a SmartExecutionStep> {
    action
        .plan
        .steps
        .iter()
        .find(|step| step.key == key)
        .ok_or_else(|| AppError::BadRequest(format!("智能动作缺少执行步骤：{key}")))
}

fn payload_request_value(payload: Option<&Value>) -> Option<Value> {
    payload.map(|value| {
        value
            .get("request")
            .cloned()
            .unwrap_or_else(|| value.clone())
    })
}

fn zhuigeng_update_request(
    step: &SmartExecutionStep,
    payload: Option<&Value>,
) -> AppResult<ZhuigengUpdateExecuteRequest> {
    let value = payload_request_value(payload)
        .ok_or_else(|| AppError::BadRequest("追更更新需要传入候选资源 candidate".to_string()))?;
    let value = with_default_field(value, "item", step.params.get("item").cloned());
    let req: ZhuigengUpdateExecuteRequest = serde_json::from_value(value)
        .map_err(|err| AppError::BadRequest(format!("追更更新 payload 无效：{err}")))?;
    if !zhuigeng_update_has_explicit_target(&req) {
        return Err(AppError::BadRequest(
            "追更更新需要明确目标库 lib 或 115 cid".to_string(),
        ));
    }
    Ok(req)
}

fn zhuigeng_archive_request(
    step: &SmartExecutionStep,
    payload: Option<&Value>,
) -> AppResult<ZhuigengArchiveExecuteRequest> {
    let value = payload_request_value(payload)
        .ok_or_else(|| AppError::BadRequest("追更归档需要传入目标库 to_lib".to_string()))?;
    let value = with_default_field(
        value,
        "on_conflict",
        step.params.get("on_conflict").cloned(),
    );
    let value = with_default_field(
        value,
        "items",
        step.params.get("item").cloned().map(|item| json!([item])),
    );
    serde_json::from_value(value)
        .map_err(|err| AppError::BadRequest(format!("追更归档 payload 无效：{err}")))
}

fn dedup_batch_request(
    step: &SmartExecutionStep,
    payload: Option<&Value>,
) -> AppResult<DedupExecuteBatchRequest> {
    let value = payload_request_value(payload)
        .or_else(|| step.params.get("request").cloned())
        .ok_or_else(|| AppError::BadRequest("去重删除缺少批量删除 request".to_string()))?;
    serde_json::from_value(value)
        .map_err(|err| AppError::BadRequest(format!("去重删除 payload 无效：{err}")))
}

fn catalog_transfer_request(
    step: &SmartExecutionStep,
    payload: Option<&Value>,
) -> AppResult<CatalogTransferExecuteRequest> {
    let raw = payload_request_value(payload)
        .ok_or_else(|| AppError::BadRequest("新增转存需要传入目标库或 cid".to_string()))?;
    let value = with_default_field(raw.clone(), "item", step.params.get("item").cloned());
    let req: CatalogTransferExecuteRequest = serde_json::from_value(value.clone())
        .map_err(|err| AppError::BadRequest(format!("新增转存 payload 无效：{err}")))?;
    if !catalog_transfer_has_target(&req) {
        return Err(AppError::BadRequest(
            "新增转存需要明确目标库 lib 或 115 cid".to_string(),
        ));
    }
    if catalog_transfer_contains_package(&req) {
        let package_ack = raw
            .get("package_ack")
            .or_else(|| value.get("package_ack"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if package_ack != "整包" {
            return Err(AppError::BadRequest(
                "整包资源需要输入确认文本：整包".to_string(),
            ));
        }
    }
    Ok(req)
}

fn catalog_transfer_has_target(req: &CatalogTransferExecuteRequest) -> bool {
    req.cid.as_deref().and_then(non_empty_trimmed).is_some()
        || req.lib.as_deref().and_then(non_empty_trimmed).is_some()
        || req
            .target
            .as_ref()
            .and_then(|target| target.cid.as_deref())
            .and_then(non_empty_trimmed)
            .is_some()
        || req
            .target
            .as_ref()
            .and_then(|target| target.lib.as_deref())
            .and_then(non_empty_trimmed)
            .is_some()
}

fn catalog_transfer_contains_package(req: &CatalogTransferExecuteRequest) -> bool {
    req.is_pkg == Some(true)
        || req.item.as_ref().and_then(|item| item.is_pkg) == Some(true)
        || req.items.iter().any(|item| item.is_pkg == Some(true))
}

fn zhuigeng_update_has_explicit_target(req: &ZhuigengUpdateExecuteRequest) -> bool {
    req.target
        .as_ref()
        .and_then(|target| target.cid.as_deref())
        .and_then(non_empty_trimmed)
        .is_some()
        || req
            .target
            .as_ref()
            .and_then(|target| target.lib.as_deref())
            .and_then(non_empty_trimmed)
            .is_some()
}

fn with_default_field(mut value: Value, key: &str, default: Option<Value>) -> Value {
    if let (Value::Object(map), Some(default)) = (&mut value, default) {
        map.entry(key.to_string()).or_insert(default);
    }
    value
}

async fn execute_poster_fix_action(state: &AppState, action: &SmartAction) -> AppResult<Value> {
    let item_id = action
        .subject
        .emby_id
        .as_deref()
        .and_then(non_empty_trimmed)
        .ok_or_else(|| AppError::BadRequest("poster action missing emby_id".to_string()))?;
    let item_type = smart_action_item_type(action);
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; cannot execute poster action".to_string(),
        ));
    }
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let result = posters::fix_poster_one(&state.pool, &client, item_id, &item_type).await;
    Ok(json!({
        "ok": result.ok,
        "module": "posters",
        "function": "fix_poster_one",
        "result": result,
    }))
}

async fn execute_emby_refresh_action(state: &AppState, action: &SmartAction) -> AppResult<Value> {
    let item_id = action
        .subject
        .emby_id
        .as_deref()
        .and_then(non_empty_trimmed)
        .ok_or_else(|| AppError::BadRequest("refresh action missing emby_id".to_string()))?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; cannot execute refresh action".to_string(),
        ));
    }
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let code = client
        .refresh_item_with_options(item_id, true, false, false)
        .await?;
    Ok(json!({
        "ok": (200..300).contains(&code),
        "module": "emby",
        "function": "refresh_item_with_options",
        "code": code,
        "item_id": item_id,
    }))
}

fn smart_action_item_type(action: &SmartAction) -> String {
    match action.subject.kind {
        SmartSubjectKind::Movie => "Movie",
        SmartSubjectKind::Series => "Series",
        SmartSubjectKind::Season => "Season",
        SmartSubjectKind::Episode => "Episode",
        _ => "Series",
    }
    .to_string()
}

fn smart_action_execution_status(
    action: &SmartAction,
    dry_run: bool,
    step_failed: bool,
    execution_outputs: &[Value],
) -> SmartActionStatus {
    if dry_run {
        return SmartActionStatus::Done;
    }
    if step_failed {
        return SmartActionStatus::Failed;
    }
    if execution_outputs
        .iter()
        .any(|value| value.get("ok").and_then(Value::as_bool) == Some(false))
    {
        return SmartActionStatus::Partial;
    }
    if matches!(
        action.action_type,
        SmartActionType::PosterFix
            | SmartActionType::MetadataRefresh
            | SmartActionType::LibraryScan
    ) && action.subject.emby_id.is_some()
    {
        SmartActionStatus::Done
    } else {
        SmartActionStatus::Partial
    }
}

fn smart_action_step_message(
    action: &SmartAction,
    step: &SmartExecutionStep,
    dry_run: bool,
) -> String {
    if dry_run {
        return "dry-run 预演通过，未执行写操作。".to_string();
    }
    match step.key.as_str() {
        "execute_low_risk" if action.subject.emby_id.is_none() => {
            "当前建议来自聚合信号，缺少具体 Emby item id，已转为人工复查动作。".to_string()
        }
        "execute_low_risk" => "低/中风险动作执行器已处理该对象。".to_string(),
        "open_context" => "已记录建议对应的功能入口。".to_string(),
        "review_evidence" => "证据和风险已写入智能动作审计记录。".to_string(),
        _ => "步骤完成。".to_string(),
    }
}

fn smart_action_task_result(
    action: &SmartAction,
    status: SmartActionStatus,
    dry_run: bool,
    steps: Vec<Value>,
    execution_outputs: Vec<Value>,
) -> Value {
    serde_json::to_value(SmartActionTaskResult {
        action_id: action.id,
        action_type: action.action_type.as_str().to_string(),
        subject: action.subject.clone(),
        dry_run,
        steps,
        outputs: execution_outputs,
        verification: smart_action_verification_result(action, status, dry_run),
        next_actions: smart_action_next_actions(action, status),
    })
    .unwrap_or_else(|err| {
        json!({
            "action_id": action.id,
            "action_type": action.action_type.as_str(),
            "subject": action.subject,
            "dry_run": dry_run,
            "steps": [],
            "outputs": [],
            "verification": {
                "status": SmartActionStatus::Failed.as_str(),
                "message": format!("智能动作结果序列化失败：{err}"),
                "checks": [],
                "check_summaries": [],
            },
            "next_actions": [],
        })
    })
}

fn smart_action_verification_result(
    action: &SmartAction,
    status: SmartActionStatus,
    dry_run: bool,
) -> Value {
    smart_action_verification_result_with_evidence(
        action,
        status,
        dry_run,
        &SmartActionVerifyEvidence::default(),
    )
}

fn smart_action_verification_result_with_evidence(
    action: &SmartAction,
    status: SmartActionStatus,
    dry_run: bool,
    evidence: &SmartActionVerifyEvidence,
) -> Value {
    let message = match status {
        SmartActionStatus::Done if dry_run => "dry-run 已验证执行计划，不代表业务状态已变化。",
        SmartActionStatus::Done => action.verification.success_message.as_str(),
        SmartActionStatus::Partial => action.verification.partial_message.as_str(),
        SmartActionStatus::Failed => "执行失败，需要查看任务错误并生成诊断动作。",
        SmartActionStatus::Verifying | SmartActionStatus::Running | SmartActionStatus::Queued => {
            "动作执行证据仍在进行中，等待任务结束后再次验证。"
        }
        _ => "动作尚未完成验收。",
    };
    json!({
        "status": status.as_str(),
        "message": message,
        "checks": action.verification.checks,
        "check_summaries": smart_action_check_summaries_with_evidence(
            action,
            status,
            dry_run,
            evidence,
        ),
    })
}

fn smart_action_verify_warnings_with_evidence(
    action: &SmartAction,
    status: SmartActionStatus,
    evidence: &SmartActionVerifyEvidence,
) -> Vec<String> {
    if status == SmartActionStatus::Done {
        return Vec::new();
    }
    let mut warnings = vec![format!(
        "{}：源信号仍然存在，动作尚未通过验收。",
        action.title
    )];
    if let Some(task) = latest_failed_verify_task(evidence) {
        warnings.push(format!(
            "最近执行任务失败：{} / {}：{}",
            task.kind,
            task.label,
            verify_task_message(task)
        ));
    } else if let Some(task) = latest_active_verify_task(evidence) {
        warnings.push(format!(
            "最近执行任务仍在 {}：{} / {} ({}/{})",
            task.status, task.kind, task.label, task.progress, task.total
        ));
    } else if evidence.collected
        && action_requires_execution_evidence(action.action_type)
        && evidence.task_runs.is_empty()
        && evidence.audit_logs.is_empty()
        && evidence.app_logs.is_empty()
    {
        warnings.push(format!(
            "没有找到 {} 的最近执行任务、audit 或日志证据，请先确认是否已提交执行。",
            action_subject_label(action)
        ));
    }
    warnings.extend(
        smart_action_check_summaries_with_evidence(action, status, false, evidence)
            .into_iter()
            .filter_map(|summary| {
                summary
                    .get("warning")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .take(3),
    );
    warnings
}

fn smart_action_check_summaries_with_evidence(
    action: &SmartAction,
    status: SmartActionStatus,
    dry_run: bool,
    evidence: &SmartActionVerifyEvidence,
) -> Vec<Value> {
    action
        .verification
        .checks
        .iter()
        .map(|check| {
            let detail = smart_action_check_detail_with_evidence(action, evidence);
            json!({
                "key": check.key,
                "title": check.title,
                "source": check.source,
                "status": verification_check_status(status, dry_run),
                "expected": check.expected,
                "summary": detail.0,
                "warning": detail.1,
                "next_check": detail.2,
                "action": smart_action_verify_action_snapshot(action, evidence),
                "evidence": smart_action_verify_evidence_snapshot(evidence),
            })
        })
        .collect()
}

fn verification_check_status(status: SmartActionStatus, dry_run: bool) -> &'static str {
    match status {
        SmartActionStatus::Done if dry_run => "planned",
        SmartActionStatus::Done => "passed",
        SmartActionStatus::Partial => "still_present",
        SmartActionStatus::Failed => "blocked",
        SmartActionStatus::Queued | SmartActionStatus::Running | SmartActionStatus::Verifying => {
            "verifying"
        }
        _ => "pending",
    }
}

fn smart_action_check_detail_with_evidence(
    action: &SmartAction,
    evidence: &SmartActionVerifyEvidence,
) -> (String, String, String) {
    if let Some(task) = latest_failed_verify_task(evidence) {
        let subject = action_subject_label(action);
        let message = verify_task_message(task);
        return (
            format!(
                "{subject} 的最近执行任务失败：{} / {}，{}。",
                task.kind, task.label, message
            ),
            format!("执行失败，需要查看任务中心错误和阶段 result：{message}。"),
            "打开任务中心定位该任务，按 error/result 生成下一步修复或重试。".to_string(),
        );
    }
    if let Some(task) = latest_active_verify_task(evidence) {
        return (
            format!(
                "最近执行任务仍在 {}：{} / {}，进度 {}/{}。",
                task.status, task.kind, task.label, task.progress, task.total
            ),
            "任务尚未结束，当前只能等待验证，避免重复提交同一动作。".to_string(),
            "等任务状态变为 done/error 后重新点击验证；如长时间不动，先检查任务中心。".to_string(),
        );
    }
    if evidence.collected
        && action_requires_execution_evidence(action.action_type)
        && evidence.task_runs.is_empty()
        && evidence.audit_logs.is_empty()
        && evidence.app_logs.is_empty()
    {
        let subject = action_subject_label(action);
        return (
            format!("{subject} 没有最近执行证据，尚不能判断动作是否真的提交。"),
            "未找到 task_runs、audit_logs 或 app_logs 证据；可能还未执行，或执行记录未带 action_id。"
                .to_string(),
            format!("先回到{}页确认是否提交过「{}」，再执行或重新验证。", action.tab, action.action_label),
        );
    }
    match action.action_type {
        SmartActionType::TaskRetryOrDiagnose => task_verify_detail(action),
        SmartActionType::TransferAddNew => transfer_add_new_verify_detail(action),
        SmartActionType::TransferUpdateSeries => transfer_update_verify_detail(action),
        SmartActionType::ArchiveSeries => archive_verify_detail(action),
        SmartActionType::DedupRemoveOld => dedup_remove_verify_detail(action),
        _ => (
            format!("仍需复查：{}", action.verification.partial_message),
            "源待办仍在当前刷新结果中。".to_string(),
            "回到对应页面确认源信号是否下降或消失。".to_string(),
        ),
    }
}

fn smart_action_status_from_verify_evidence(
    action: &SmartAction,
    live_signal_present: bool,
    evidence: &SmartActionVerifyEvidence,
) -> SmartActionStatus {
    if latest_failed_verify_task(evidence).is_some() {
        return SmartActionStatus::Failed;
    }
    if latest_active_verify_task(evidence).is_some() {
        return SmartActionStatus::Verifying;
    }
    if evidence
        .persisted_error
        .as_deref()
        .and_then(non_empty_trimmed)
        .is_some()
        || evidence.persisted_status == Some(SmartActionStatus::Failed)
    {
        return SmartActionStatus::Failed;
    }
    if matches!(
        evidence.persisted_status,
        Some(SmartActionStatus::Queued | SmartActionStatus::Running | SmartActionStatus::Verifying)
    ) {
        return SmartActionStatus::Verifying;
    }
    if live_signal_present {
        return SmartActionStatus::Partial;
    }
    if evidence.collected
        && action_requires_execution_evidence(action.action_type)
        && evidence.task_runs.is_empty()
        && evidence.audit_logs.is_empty()
        && evidence.app_logs.is_empty()
        && evidence.persisted_result.is_none()
    {
        return SmartActionStatus::Partial;
    }
    SmartActionStatus::Done
}

fn latest_failed_verify_task(
    evidence: &SmartActionVerifyEvidence,
) -> Option<&VerifyTaskRunEvidence> {
    evidence
        .task_runs
        .iter()
        .find(|task| verify_task_failed(task))
}

fn latest_active_verify_task(
    evidence: &SmartActionVerifyEvidence,
) -> Option<&VerifyTaskRunEvidence> {
    evidence
        .task_runs
        .iter()
        .find(|task| verify_task_active(task))
}

fn verify_task_failed(task: &VerifyTaskRunEvidence) -> bool {
    matches!(
        task.status.as_str(),
        "error" | "failed" | "interrupted" | "cancelled"
    ) || task.error.as_deref().and_then(non_empty_trimmed).is_some()
        || task_result_has_partial_signal(task.result.as_ref())
}

fn verify_task_active(task: &VerifyTaskRunEvidence) -> bool {
    matches!(
        task.status.as_str(),
        "pending" | "queued" | "running" | "verifying"
    )
}

fn verify_task_message(task: &VerifyTaskRunEvidence) -> String {
    task.error
        .as_deref()
        .and_then(non_empty_trimmed)
        .map(ToString::to_string)
        .or_else(|| {
            task.result
                .as_ref()
                .and_then(|result| json_string_at(result, &["err"]))
        })
        .or_else(|| {
            task.result
                .as_ref()
                .and_then(|result| json_string_at(result, &["error"]))
        })
        .or_else(|| non_empty_trimmed(&task.status_text).map(ToString::to_string))
        .unwrap_or_else(|| format!("状态为 {}", task.status))
}

fn action_requires_execution_evidence(action_type: SmartActionType) -> bool {
    matches!(
        action_type,
        SmartActionType::TransferAddNew
            | SmartActionType::TransferUpdateSeries
            | SmartActionType::DedupRemoveOld
            | SmartActionType::ArchiveSeries
    )
}

fn smart_action_verify_action_snapshot(
    action: &SmartAction,
    evidence: &SmartActionVerifyEvidence,
) -> Value {
    json!({
        "source": evidence
            .persisted_source
            .as_deref()
            .unwrap_or(action.source.as_str()),
        "tab": evidence
            .persisted_tab
            .as_deref()
            .unwrap_or(action.tab.as_str()),
        "action_label": evidence
            .persisted_action_label
            .as_deref()
            .unwrap_or(action.action_label.as_str()),
        "persisted_status": evidence.persisted_status.map(|status| status.as_str()),
        "persisted_task_id": evidence.persisted_task_id,
        "persisted_updated_at": evidence.persisted_updated_at,
        "snapshot": {
            "id": action.id,
            "action_type": action.action_type.as_str(),
            "status": action.status.as_str(),
            "title": &action.title,
            "summary": &action.summary,
            "subject": &action.subject,
            "risk": &action.risk,
            "policy": &action.policy,
            "created_at": action.created_at,
            "updated_at": action.updated_at,
        },
    })
}

fn smart_action_verify_evidence_snapshot(evidence: &SmartActionVerifyEvidence) -> Value {
    json!({
        "task_runs": &evidence.task_runs,
        "audit_logs": &evidence.audit_logs,
        "app_logs": &evidence.app_logs,
        "counts": {
            "task_runs": evidence.task_runs.len(),
            "audit_logs": evidence.audit_logs.len(),
            "app_logs": evidence.app_logs.len(),
        },
        "latest_task": evidence.task_runs.first().map(|task| {
            json!({
                "id": task.id,
                "kind": task.kind,
                "label": task.label,
                "status": task.status,
                "message": verify_task_message(task),
                "progress": task.progress,
                "total": task.total,
                "updated_at": task.updated_at,
            })
        }),
        "persisted_result": &evidence.persisted_result,
        "persisted_error": &evidence.persisted_error,
    })
}

fn task_verify_detail(action: &SmartAction) -> (String, String, String) {
    let task = first_evidence_value(action, SmartEvidenceSource::TaskHistory);
    let kind = task
        .and_then(|value| json_string_at(value, &["kind"]))
        .unwrap_or_else(|| action.source.clone());
    let status = task
        .and_then(|value| json_string_at(value, &["status"]))
        .unwrap_or_else(|| "unknown".to_string());
    let error = task
        .and_then(|value| json_string_at(value, &["error"]))
        .or_else(|| task.and_then(|value| json_string_at(value, &["status_text"])))
        .unwrap_or_else(|| "没有明确错误文本".to_string());
    let progress = task
        .and_then(|value| json_i64_at(value, &["progress"]))
        .zip(task.and_then(|value| json_i64_at(value, &["total"])))
        .map(|(progress, total)| format!("，进度 {progress}/{total}"))
        .unwrap_or_default();
    (
        format!("任务 {kind} 仍处于 {status} 信号{progress}：{error}"),
        format!("失败或半成功任务仍存在：{error}"),
        "打开任务中心查看阶段 result/error，优先生成更具体的修复动作或人工结论。".to_string(),
    )
}

fn transfer_add_new_verify_detail(action: &SmartAction) -> (String, String, String) {
    let catalog = first_evidence_value(action, SmartEvidenceSource::CatalogCandidate);
    let ranges = catalog
        .and_then(|value| json_string_array_at(value, &["recommendation", "episode_ranges"]))
        .filter(|ranges| !ranges.is_empty())
        .map(|ranges| ranges.join("、"))
        .unwrap_or_else(|| "候选资源集数未解析".to_string());
    let context = first_evidence_value(action, SmartEvidenceSource::EmbyEpisodes)
        .or_else(|| first_evidence_value(action, SmartEvidenceSource::EmbyItem));
    let missing = context
        .and_then(|value| json_string_array_at(value, &["summary", "missing_ranges"]))
        .filter(|ranges| !ranges.is_empty())
        .map(|ranges| ranges.join("、"))
        .unwrap_or_else(|| "缺口未知".to_string());
    let subject = action_subject_label(action);
    (
        format!("{subject} 的转存建议仍在：候选覆盖 {ranges}，本地缺口 {missing}。"),
        format!("仍需确认 115 转存、STRM 生成和 Emby 可见性：{subject}。"),
        "在找资源/任务中心确认转存任务成功，再检查扫库后是否出现对应 Emby 条目。".to_string(),
    )
}

fn transfer_update_verify_detail(action: &SmartAction) -> (String, String, String) {
    let tmdb = first_evidence_value(action, SmartEvidenceSource::TmdbMetadata);
    let behind = tmdb.and_then(|value| json_i64_at(value, &["behind"]));
    let hint = tmdb
        .and_then(|value| json_string_at(value, &["behind_hint"]))
        .unwrap_or_else(|| "追更缺口仍未确认消失".to_string());
    let latest = first_evidence_value(action, SmartEvidenceSource::EmbyEpisodes)
        .and_then(|value| json_string_at(value, &["local_latest_episode"]))
        .unwrap_or_else(|| "本地最新集未知".to_string());
    let subject = action_subject_label(action);
    let behind_text = behind
        .map(|value| format!("落后 {value} 集"))
        .unwrap_or_else(|| "落后集数未知".to_string());
    (
        format!("{subject} 仍有追更更新信号：{behind_text}，{hint}，当前 {latest}。"),
        format!("更新后仍需复查新集 STRM、Emby 可见性和旧版本冲突：{subject}。"),
        "回到追更检查确认行是否移出更新队列，并核对任务中心阶段结果。".to_string(),
    )
}

fn archive_verify_detail(action: &SmartAction) -> (String, String, String) {
    let tmdb = first_evidence_value(action, SmartEvidenceSource::TmdbMetadata);
    let state = tmdb
        .and_then(|value| json_string_at(value, &["state"]))
        .unwrap_or_else(|| "状态未知".to_string());
    let latest = first_evidence_value(action, SmartEvidenceSource::EmbyEpisodes)
        .and_then(|value| json_string_at(value, &["local_latest_episode"]))
        .unwrap_or_else(|| "本地最新集未知".to_string());
    let subject = action_subject_label(action);
    (
        format!("{subject} 仍在归档候选中：追更状态 {state}，当前 {latest}。"),
        format!("归档信号仍存在，需确认追更库已移出且目标库可见：{subject}。"),
        "检查归档任务结果、目标库条目和原追更目录是否按 undo/audit 记录移动。".to_string(),
    )
}

fn dedup_remove_verify_detail(action: &SmartAction) -> (String, String, String) {
    let dedup = first_evidence_value(action, SmartEvidenceSource::DedupAnalysis);
    let remove_count = dedup
        .and_then(|value| value.get("remove"))
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    let tmdb = dedup
        .and_then(|value| json_string_at(value, &["tmdb"]))
        .or_else(|| action.subject.tmdb.clone())
        .unwrap_or_else(|| "未知 TMDb".to_string());
    let subject = action_subject_label(action);
    (
        format!("{subject} 的重复组仍存在：TMDb {tmdb}，待删除候选 {remove_count} 个。"),
        format!("去重后仍命中重复信号，需复核删除失败项和 undo/audit 记录：{subject}。"),
        "打开去重页确认重复组是否减少，并检查任务中心是否有路径锁定或删除失败。".to_string(),
    )
}

fn action_subject_label(action: &SmartAction) -> String {
    let mut label = action.subject.name.clone();
    if let Some(folder) = action.subject.folder.as_deref().and_then(non_empty_trimmed)
        && !label.contains(folder)
    {
        label = format!("{label} / {folder}");
    }
    if let Some(lib) = action.subject.lib.as_deref().and_then(non_empty_trimmed) {
        label = format!("{lib} / {label}");
    }
    label
}

fn first_evidence_value(action: &SmartAction, source: SmartEvidenceSource) -> Option<&Value> {
    action
        .evidence
        .iter()
        .find(|evidence| evidence.source == source)
        .map(|evidence| &evidence.value)
}

fn json_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn json_string_at(value: &Value, path: &[&str]) -> Option<String> {
    json_at(value, path)
        .and_then(Value::as_str)
        .and_then(non_empty_trimmed)
        .map(ToString::to_string)
}

fn json_i64_at(value: &Value, path: &[&str]) -> Option<i64> {
    json_at(value, path).and_then(Value::as_i64)
}

fn json_string_array_at(value: &Value, path: &[&str]) -> Option<Vec<String>> {
    Some(
        json_at(value, path)?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .filter_map(non_empty_trimmed)
            .map(ToString::to_string)
            .collect(),
    )
}

fn smart_action_next_actions(
    action: &SmartAction,
    status: SmartActionStatus,
) -> Vec<SmartNextAction> {
    if status != SmartActionStatus::Partial {
        return Vec::new();
    }
    let subject = SmartNextActionSubject {
        name: Some(action.subject.name.clone()),
        lib: action.subject.lib.clone(),
        tmdb: action.subject.tmdb.clone(),
        emby_id: action.subject.emby_id.clone(),
        folder: action.subject.folder.clone(),
    };
    let mut actions = match action.action_type {
        SmartActionType::TransferAddNew => vec![
            smart_next_action(
                "library_scan",
                "刷新媒体库",
                "scan",
                "转存或 STRM 生成后仍未通过验收，先让 Emby 重新发现新条目。",
                &subject,
            ),
            smart_next_action(
                "poster_fix",
                "修复海报",
                "posters",
                "新资源已进入库但海报/元数据仍可能缺失，继续做海报检测与自动修复。",
                &subject,
            ),
            smart_next_action(
                "dedup_review",
                "复核重复旧版本",
                "dedup",
                "新增资源后仍需确认同剧旧目录是否被清掉，避免同一部剧并存。",
                &subject,
            ),
        ],
        SmartActionType::TransferUpdateSeries => vec![
            smart_next_action(
                "library_scan",
                "刷新追更库",
                "scan",
                "新集转存后仍有追更缺口信号，先刷新对应媒体库。",
                &subject,
            ),
            smart_next_action(
                "dedup_review",
                "清理同剧旧目录",
                "dedup",
                "追更更新后旧版本仍可能并存，需要复核并删除旧目录。",
                &subject,
            ),
            smart_next_action(
                "poster_fix",
                "检查海报",
                "posters",
                "更新后如果 TMDb/海报未绑定，需要继续修复海报和元数据。",
                &subject,
            ),
        ],
        SmartActionType::DedupRemoveOld => vec![
            smart_next_action(
                "dedup_review",
                "复查重复组",
                "dedup",
                "删除后重复信号仍在，回到去重页确认失败项和保留项。",
                &subject,
            ),
            smart_next_action(
                "undo_review",
                "查看 Undo 记录",
                "manage",
                "删除类动作需要保留可追溯记录；如果误删，优先从 Undo 入口处理。",
                &subject,
            ),
        ],
        SmartActionType::ArchiveSeries => vec![
            smart_next_action(
                "archive_review",
                "复查归档结果",
                "zhuigeng",
                "归档后仍在追更候选中，确认原库是否移出、目标库是否可见。",
                &subject,
            ),
            smart_next_action(
                "library_scan",
                "刷新源库和目标库",
                "scan",
                "归档移动后需要让 Emby 重新识别源库删除和目标库新增。",
                &subject,
            ),
        ],
        SmartActionType::PosterFix => vec![
            smart_next_action(
                "poster_fix",
                "重新检测海报",
                "posters",
                "海报修复未完全通过，继续在海报修复页查看候选和失败原因。",
                &subject,
            ),
            smart_next_action(
                "metadata_refresh",
                "刷新元数据",
                "scan",
                "如果图片已写入但 Emby 仍显示旧海报，刷新该条目元数据。",
                &subject,
            ),
        ],
        SmartActionType::TaskRetryOrDiagnose => vec![smart_next_action(
            "task_retry_or_diagnose",
            "生成更具体修复动作",
            "smart-actions",
            "任务仍未成功，需要根据 error/result 继续生成下一步修复。",
            &subject,
        )],
        _ => Vec::new(),
    };
    actions.push(smart_next_action(
        action.action_type.as_str(),
        &action.action_label,
        &action.tab,
        "该建议仍需在具体功能页完成对象级处理。",
        &subject,
    ));
    actions
}

fn smart_next_action(
    action_type: &str,
    label: &str,
    tab: &str,
    reason: &str,
    subject: &SmartNextActionSubject,
) -> SmartNextAction {
    SmartNextAction {
        action_type: action_type.to_string(),
        label: label.to_string(),
        tab: tab.to_string(),
        reason: reason.to_string(),
        subject: Some(subject.clone()),
    }
}

fn to_json<T: Serialize>(value: &T) -> AppResult<Value> {
    serde_json::to_value(value).map_err(|err| AppError::Anyhow(err.into()))
}

fn filter_smart_actions(
    generated: GeneratedSmartActions,
    query: SmartActionsQuery,
) -> SmartActionsListResponse {
    let mut actions = generated.actions;
    let status_filter = normalized_filter(query.status.as_deref());
    if let Some(status) = status_filter.as_deref() {
        actions.retain(|action| action.status.as_str() == status);
    } else {
        actions.retain(|action| action.status != SmartActionStatus::Dismissed);
    }
    if let Some(action_type) = normalized_filter(query.action_type.as_deref()) {
        actions.retain(|action| action.action_type.as_str() == action_type);
    }
    if let Some(risk) = normalized_filter(query.risk.as_deref()) {
        actions.retain(|action| action.risk.level.as_str() == risk);
    }
    if let Some(subject_kind) = normalized_filter(query.subject_kind.as_deref()) {
        actions.retain(|action| action.subject.kind.as_str() == subject_kind);
    }
    if let Some(lib) = query.lib.as_deref().and_then(non_empty_trimmed) {
        actions.retain(|action| {
            action
                .subject
                .lib
                .as_deref()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(lib))
        });
    }
    if let Some(q) = query.q.as_deref().and_then(non_empty_trimmed) {
        let q = q.to_lowercase();
        actions.retain(|action| smart_action_haystack(action).contains(&q));
    }

    actions.sort_by(|left, right| {
        right
            .recommendation
            .score
            .cmp(&left.recommendation.score)
            .then_with(|| right.risk.level.cmp(&left.risk.level))
            .then_with(|| left.title.cmp(&right.title))
    });

    let summary = summarize_actions(&actions);
    let total = actions.len();
    let limit = query.limit.unwrap_or(80).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).min(total);
    let actions = actions.into_iter().skip(offset).take(limit).collect();

    SmartActionsListResponse {
        ok: generated.warnings.is_empty(),
        total,
        limit,
        offset,
        actions,
        warnings: generated.warnings,
        summary,
    }
}

fn smart_action_from_dashboard(item: &DashboardSmartAction, now: DateTime<Utc>) -> SmartAction {
    let action_type = action_type_from_dashboard(item);
    let risk = smart_risk(action_type);
    let policy = smart_policy(action_type, risk.level);
    let status = SmartActionStatus::Suggested;
    let fingerprint = format!("{}:{}:{}:{}", item.source, item.area, item.tab, item.title);
    let id = Uuid::new_v5(&Uuid::NAMESPACE_URL, fingerprint.as_bytes());
    SmartAction {
        id,
        action_type,
        status,
        subject: smart_subject_from_dashboard(item),
        title: item.title.clone(),
        summary: item.message.clone(),
        recommendation: smart_recommendation(item, action_type),
        evidence: vec![SmartEvidence {
            source: evidence_source_from_dashboard(item),
            label: "Dashboard 聚合信号".to_string(),
            value: json!({
                "area": item.area,
                "source": item.source,
                "count": item.count,
                "tab": item.tab,
                "action": item.action,
            }),
            weight: score_from_severity(&item.severity),
            collected_at: now,
        }],
        plan: smart_execution_plan(item, action_type),
        risk,
        policy,
        verification: smart_verification_plan(item, action_type),
        source: item.source.clone(),
        tab: item.tab.clone(),
        action_label: item.action.clone(),
        created_at: now,
        updated_at: now,
    }
}

fn smart_action_from_task_run(task: &TaskRun, now: DateTime<Utc>) -> SmartAction {
    let action_type = SmartActionType::TaskRetryOrDiagnose;
    let risk = smart_risk(action_type);
    let policy = smart_policy(action_type, risk.level);
    let repair_suggestions = task_repair_suggestions(task);
    let status_label = if task
        .error
        .as_deref()
        .is_some_and(|err| !err.trim().is_empty())
    {
        "失败"
    } else if task.status == "done" && task_result_has_partial_signal(task.result.as_ref()) {
        "半成功"
    } else {
        "需复查"
    };
    let title = format!("诊断{}任务：{}", status_label, task.label);
    let summary = task
        .error
        .as_deref()
        .and_then(non_empty_trimmed)
        .map(ToString::to_string)
        .or_else(|| non_empty_trimmed(&task.status_text).map(ToString::to_string))
        .unwrap_or_else(|| format!("任务 {} 状态为 {}", task.kind, task.status));
    let score = if task.status == "error" {
        88
    } else if task_result_has_partial_signal(task.result.as_ref()) {
        82
    } else {
        64
    };
    let primary_action = repair_suggestions
        .first()
        .map(|suggestion| suggestion.label.clone())
        .unwrap_or_else(|| "查看任务阶段结果并生成下一步修复".to_string());
    let alternatives = if repair_suggestions.is_empty() {
        vec![SmartAlternative {
            action: "打开任务中心查看技术详情".to_string(),
            reason: "如果阶段结果不足以判断，可以查看原始 result/error。".to_string(),
        }]
    } else {
        repair_suggestions
            .iter()
            .map(|suggestion| SmartAlternative {
                action: suggestion.label.clone(),
                reason: suggestion.reason.clone(),
            })
            .collect()
    };
    let mut steps = vec![
        SmartExecutionStep {
            key: "open_task_center".to_string(),
            title: "打开任务中心定位该任务".to_string(),
            executor: SmartExecutorKind::OpenTab,
            params: json!({ "tab": "tasks", "task_id": task.id }),
            rollback: None,
        },
        SmartExecutionStep {
            key: "review_task_result".to_string(),
            title: "复核阶段结果和错误信息".to_string(),
            executor: SmartExecutorKind::ManualConfirm,
            params: json!({
                "kind": task.kind,
                "status": task.status,
                "status_text": task.status_text,
                "error": task.error,
            }),
            rollback: None,
        },
    ];
    steps.extend(
        repair_suggestions
            .iter()
            .enumerate()
            .map(|(index, suggestion)| SmartExecutionStep {
                key: format!("open_repair_{}_{}", index + 1, suggestion.action_type),
                title: suggestion.label.clone(),
                executor: SmartExecutorKind::OpenTab,
                params: json!({
                    "tab": suggestion.tab,
                    "action_type": suggestion.action_type,
                    "reason": suggestion.reason,
                    "subject": {
                        "lib": task_param_string(&task.params, "lib")
                            .or_else(|| task_param_string(&task.params, "target_lib")),
                        "folder": task_param_string(&task.params, "folder"),
                        "tmdb": task_param_string(&task.params, "tmdb"),
                        "emby_id": task_param_string(&task.params, "item_id")
                            .or_else(|| task_param_string(&task.params, "emby_id")),
                    }
                }),
                rollback: None,
            }),
    );
    SmartAction {
        id: Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("task_diagnose:{}", task.id).as_bytes(),
        ),
        action_type,
        status: SmartActionStatus::Suggested,
        subject: SmartSubject {
            kind: SmartSubjectKind::Task,
            name: task.label.clone(),
            year: None,
            tmdb: task_param_string(&task.params, "tmdb"),
            emby_id: task_param_string(&task.params, "item_id")
                .or_else(|| task_param_string(&task.params, "emby_id")),
            lib: task_param_string(&task.params, "lib")
                .or_else(|| task_param_string(&task.params, "target_lib")),
            folder: task_param_string(&task.params, "folder"),
            strm_path: None,
            cd_path: None,
        },
        title,
        summary: summary.clone(),
        recommendation: SmartRecommendation {
            score,
            confidence: confidence_from_score(score),
            primary_action,
            reasons: task_diagnose_reasons(task, &summary, &repair_suggestions),
            alternatives,
        },
        evidence: vec![SmartEvidence {
            source: SmartEvidenceSource::TaskHistory,
            label: "任务历史记录".to_string(),
            value: json!({
                "id": task.id,
                "kind": task.kind,
                "label": task.label,
                "source": task.source,
                "status": task.status,
                "progress": task.progress,
                "total": task.total,
                "status_text": task.status_text,
                "params": task.params,
                "result": task.result,
                "error": task.error,
                "repair_suggestions": repair_suggestions,
                "updated_at": task.updated_at,
            }),
            weight: score,
            collected_at: now,
        }],
        plan: SmartExecutionPlan {
            steps,
            estimated_seconds: None,
            concurrency_key: None,
            can_cancel: false,
        },
        risk,
        policy,
        verification: SmartVerificationPlan {
            checks: vec![SmartVerificationCheck {
                key: "task_diagnosed".to_string(),
                title: "失败原因已定位".to_string(),
                source: SmartEvidenceSource::TaskHistory,
                expected: "任务错误被归类，并生成具体修复动作或人工处理结论".to_string(),
            }],
            success_message: "任务已诊断并有明确下一步。".to_string(),
            partial_message: "仍需查看任务结果或相关业务页面继续定位。".to_string(),
        },
        source: format!("task_runs.{}", task.kind),
        tab: "tasks".to_string(),
        action_label: "诊断任务".to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn smart_action_from_next_action_request(
    req: &SmartActionFromNextActionRequest,
    now: DateTime<Utc>,
) -> AppResult<(SmartAction, Vec<String>)> {
    let next = &req.next_action;
    let action_type = next_action_type(&next.action_type);
    let risk = smart_risk(action_type);
    let policy = smart_policy(action_type, risk.level);
    let subject = smart_subject_from_next_action(next);
    let mut warnings = Vec::new();
    if matches!(
        action_type,
        SmartActionType::TransferAddNew
            | SmartActionType::TransferUpdateSeries
            | SmartActionType::ArchiveSeries
            | SmartActionType::DedupRemoveOld
    ) {
        warnings.push("该后续动作已生成草案，但执行前仍需要在对应功能页补齐候选资源、目标库或删除确认 payload。".to_string());
    }
    if matches!(
        action_type,
        SmartActionType::PosterFix
            | SmartActionType::MetadataRefresh
            | SmartActionType::LibraryScan
    ) && subject
        .emby_id
        .as_deref()
        .and_then(non_empty_trimmed)
        .is_none()
    {
        warnings.push(
            "缺少 Emby item id，当前草案只能打开上下文，不能直接执行低风险修复。".to_string(),
        );
    }
    let source_action = req
        .source_action_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "manual".to_string());
    let task = req
        .task_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "none".to_string());
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "next_action:{source_action}:{task}:{}:{}:{}:{}",
            next.action_type, next.tab, next.label, subject.name
        )
        .as_bytes(),
    );
    let title = format!("下一步：{}", next.label);
    let summary = if next.reason.trim().is_empty() {
        format!("从任务中心后续动作生成，建议进入{}继续处理。", next.tab)
    } else {
        next.reason.clone()
    };
    let score = match risk.level {
        SmartRiskLevel::Low => 58,
        SmartRiskLevel::Medium => 72,
        SmartRiskLevel::High => 84,
        SmartRiskLevel::Critical => 92,
    };
    let plan = next_action_execution_plan(next, action_type, &subject);
    let verification = next_action_verification_plan(next, action_type);
    Ok((
        SmartAction {
            id,
            action_type,
            status: SmartActionStatus::Suggested,
            subject,
            title,
            summary: summary.clone(),
            recommendation: SmartRecommendation {
                score,
                confidence: confidence_from_score(score),
                primary_action: next.label.clone(),
                reasons: vec![
                    summary,
                    "该动作来自任务结果验收后的后续建议，已纳入智能动作追踪。".to_string(),
                ],
                alternatives: vec![SmartAlternative {
                    action: "只打开原功能页人工处理".to_string(),
                    reason: "如果候选资源、目标库或删除范围不明确，不应直接执行。".to_string(),
                }],
            },
            evidence: vec![SmartEvidence {
                source: SmartEvidenceSource::TaskHistory,
                label: "任务中心后续动作".to_string(),
                value: json!({
                    "next_action": next,
                    "source_action_id": req.source_action_id,
                    "task_id": req.task_id,
                }),
                weight: score,
                collected_at: now,
            }],
            plan,
            risk,
            policy,
            verification,
            source: "task_next_actions".to_string(),
            tab: next.tab.clone(),
            action_label: next.label.clone(),
            created_at: now,
            updated_at: now,
        },
        warnings,
    ))
}

fn next_action_type(value: &str) -> SmartActionType {
    SmartActionType::from_str(value).unwrap_or(match value {
        "dedup_review" | "undo_review" => SmartActionType::DedupReview,
        "archive_review" => SmartActionType::ArchiveSeries,
        "config_tmdb" | "config_fix" | "task_retry_or_diagnose" => {
            SmartActionType::TaskRetryOrDiagnose
        }
        _ => SmartActionType::TaskRetryOrDiagnose,
    })
}

fn smart_subject_from_next_action(next: &SmartNextAction) -> SmartSubject {
    let subject = next.subject.as_ref();
    SmartSubject {
        kind: SmartSubjectKind::Unknown,
        name: subject
            .and_then(|subject| subject.name.clone())
            .and_then(|name| non_empty_trimmed(&name).map(ToString::to_string))
            .unwrap_or_else(|| next.label.clone()),
        year: None,
        tmdb: subject.and_then(|subject| subject.tmdb.clone()),
        emby_id: subject.and_then(|subject| subject.emby_id.clone()),
        lib: subject.and_then(|subject| subject.lib.clone()),
        folder: subject.and_then(|subject| subject.folder.clone()),
        strm_path: None,
        cd_path: None,
    }
}

fn next_action_execution_plan(
    next: &SmartNextAction,
    action_type: SmartActionType,
    subject: &SmartSubject,
) -> SmartExecutionPlan {
    let mut steps = vec![
        SmartExecutionStep {
            key: "open_context".to_string(),
            title: format!("打开{}入口", next.label),
            executor: SmartExecutorKind::OpenTab,
            params: json!({
                "tab": next.tab,
                "action_type": next.action_type,
                "subject": subject,
            }),
            rollback: None,
        },
        SmartExecutionStep {
            key: "review_evidence".to_string(),
            title: "复核后续动作上下文".to_string(),
            executor: SmartExecutorKind::ManualConfirm,
            params: json!({
                "reason": next.reason,
                "subject": subject,
            }),
            rollback: None,
        },
    ];
    if matches!(
        action_type,
        SmartActionType::PosterFix
            | SmartActionType::MetadataRefresh
            | SmartActionType::LibraryScan
    ) && subject
        .emby_id
        .as_deref()
        .and_then(non_empty_trimmed)
        .is_some()
    {
        steps.push(SmartExecutionStep {
            key: "execute_low_risk".to_string(),
            title: "执行低/中风险后续修复".to_string(),
            executor: SmartExecutorKind::ExistingEndpoint,
            params: json!({
                "from_next_action": true,
                "action_type": next.action_type,
            }),
            rollback: None,
        });
    } else {
        steps.push(SmartExecutionStep {
            key: "manual_followup".to_string(),
            title: "在对应功能页完成人工处理".to_string(),
            executor: SmartExecutorKind::ManualConfirm,
            params: json!({
                "tab": next.tab,
                "reason": next.reason,
            }),
            rollback: None,
        });
    }
    SmartExecutionPlan {
        steps,
        estimated_seconds: None,
        concurrency_key: concurrency_key(action_type),
        can_cancel: true,
    }
}

fn next_action_verification_plan(
    next: &SmartNextAction,
    action_type: SmartActionType,
) -> SmartVerificationPlan {
    let expected = match action_type {
        SmartActionType::LibraryScan => "刷新后 Emby 条目可见，任务结果不再提示扫库。",
        SmartActionType::PosterFix | SmartActionType::MetadataRefresh => {
            "海报或元数据缺失信号消失。"
        }
        SmartActionType::DedupReview | SmartActionType::DedupRemoveOld => {
            "重复组减少，保留/删除对象被明确记录。"
        }
        SmartActionType::ArchiveSeries => "追更归档信号消失，源库和目标库状态一致。",
        SmartActionType::TransferAddNew | SmartActionType::TransferUpdateSeries => {
            "转存、STRM、扫库和旧版本清理均通过任务中心验收。"
        }
        _ => "任务中心后续动作被处理，相关失败或半成功信号消失。",
    };
    SmartVerificationPlan {
        checks: vec![SmartVerificationCheck {
            key: "next_action_resolved".to_string(),
            title: format!("复查{}", next.label),
            source: SmartEvidenceSource::TaskHistory,
            expected: expected.to_string(),
        }],
        success_message: "后续动作已处理并通过复查。".to_string(),
        partial_message: "后续动作仍需继续在对应功能页处理。".to_string(),
    }
}

fn smart_action_from_poster_signal(item: &PosterSignalItem, now: DateTime<Utc>) -> SmartAction {
    let action_type = SmartActionType::PosterFix;
    let mut risk = smart_risk(action_type);
    if item
        .signals
        .iter()
        .any(|signal| signal.kind == "declared_tmdb_mismatch")
    {
        risk.warnings
            .push("检测到声明 TMDb 与 Emby 绑定不一致，执行会重新绑定并刷新元数据。".to_string());
    }
    let policy = smart_policy(action_type, risk.level);
    let score = item.score.clamp(1, 100) as i32;
    let title = format!("修复海报：{}", item.name);
    let summary = if item.reasons.is_empty() {
        format!("{} / {} 命中海报或 TMDb 修复信号。", item.lib, item.folder)
    } else {
        item.reasons.join("；")
    };
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("poster:{}:{}:{:?}", item.id, item.tmdb, item.declared_tmdb).as_bytes(),
    );

    SmartAction {
        id,
        action_type,
        status: SmartActionStatus::Suggested,
        subject: SmartSubject {
            kind: smart_subject_kind_from_item_type(&item.item_type),
            name: item.name.clone(),
            year: None,
            tmdb: first_non_empty([Some(item.tmdb.clone()), item.declared_tmdb.clone()]),
            emby_id: Some(item.id.clone()),
            lib: Some(item.lib.clone()),
            folder: Some(item.folder.clone()),
            strm_path: item.path.clone(),
            cd_path: None,
        },
        title,
        summary,
        recommendation: SmartRecommendation {
            score,
            confidence: confidence_from_score(score),
            primary_action: "自动修复海报/绑定并刷新 Emby".to_string(),
            reasons: item.reasons.clone(),
            alternatives: vec![SmartAlternative {
                action: "打开海报修复页人工确认".to_string(),
                reason: "如果名称或 TMDb 信号不确定，可以先查看候选海报。".to_string(),
            }],
        },
        evidence: vec![SmartEvidence {
            source: SmartEvidenceSource::PosterDetection,
            label: "对象级海报检测".to_string(),
            value: serde_json::to_value(item).unwrap_or_else(|_| json!({})),
            weight: score,
            collected_at: now,
        }],
        plan: smart_poster_execution_plan(item),
        risk,
        policy,
        verification: SmartVerificationPlan {
            checks: vec![SmartVerificationCheck {
                key: "primary_poster_present".to_string(),
                title: "Primary 海报存在".to_string(),
                source: SmartEvidenceSource::PosterDetection,
                expected: "重新检测时该 Emby 条目不再出现在海报问题列表".to_string(),
            }],
            success_message: "海报已绑定，Emby 条目出现 Primary 图片。".to_string(),
            partial_message: "执行已完成，但海报检测信号仍存在，需要人工复查候选。".to_string(),
        },
        source: "poster_detection.item".to_string(),
        tab: "posters".to_string(),
        action_label: "修复海报".to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn smart_action_from_zhuigeng_row(row: &ZhuigengWorkbenchRow, now: DateTime<Utc>) -> SmartAction {
    let item = &row.item;
    let item_ref = zhuigeng_item_ref(item);
    let action_type = match row.lane {
        ZhuigengWorkbenchLane::ArchiveReady => SmartActionType::ArchiveSeries,
        _ => SmartActionType::TransferUpdateSeries,
    };
    let risk = smart_risk(action_type);
    let policy = smart_policy(action_type, risk.level);
    let title = match row.lane {
        ZhuigengWorkbenchLane::ArchiveReady => format!("归档完结剧：{}", item.name),
        ZhuigengWorkbenchLane::CompleteAfterUpdate => format!("补齐后归档：{}", item.name),
        _ => format!("更新追更剧：{}", item.name),
    };
    let summary = row
        .blockers
        .first()
        .cloned()
        .or_else(|| item.behind_hint.clone())
        .unwrap_or_else(|| row.action.clone());
    let fingerprint = format!(
        "zhuigeng:{}:{}:{}:{:?}",
        action_type.as_str(),
        item.lib,
        item.id.as_deref().unwrap_or(&item.name),
        row.lane
    );

    SmartAction {
        id: Uuid::new_v5(&Uuid::NAMESPACE_URL, fingerprint.as_bytes()),
        action_type,
        status: SmartActionStatus::Suggested,
        subject: SmartSubject {
            kind: SmartSubjectKind::Series,
            name: item.name.clone(),
            year: None,
            tmdb: non_empty_trimmed(&item.tmdb).map(ToString::to_string),
            emby_id: item.id.clone(),
            lib: Some(item.lib.clone()),
            folder: non_empty_trimmed(&item.folder).map(ToString::to_string),
            strm_path: None,
            cd_path: None,
        },
        title,
        summary,
        recommendation: SmartRecommendation {
            score: zhuigeng_score(row),
            confidence: confidence_from_score(zhuigeng_score(row)),
            primary_action: row.action.clone(),
            reasons: zhuigeng_reasons(row),
            alternatives: vec![SmartAlternative {
                action: "打开追更检查人工处理".to_string(),
                reason: "追更更新和归档会触发转存、移动或库刷新，执行前适合复核资源候选。"
                    .to_string(),
            }],
        },
        evidence: zhuigeng_evidence(row, now),
        plan: zhuigeng_execution_plan(row, action_type, &item_ref),
        risk,
        policy,
        verification: zhuigeng_verification_plan(row, action_type),
        source: format!("zhuigeng.{:?}", row.lane).to_ascii_lowercase(),
        tab: "zhuigeng".to_string(),
        action_label: row.action.clone(),
        created_at: now,
        updated_at: now,
    }
}

fn zhuigeng_item_ref(item: &ZhuigengItem) -> ZhuigengItemRef {
    ZhuigengItemRef {
        lib: item.lib.clone(),
        name: item.name.clone(),
        id: item.id.clone(),
        folder: non_empty_trimmed(&item.folder).map(ToString::to_string),
        tmdb: non_empty_trimmed(&item.tmdb).map(ToString::to_string),
        behind: Some(item.behind),
        resource_hint: item.resource_hint.clone(),
    }
}

fn zhuigeng_score(row: &ZhuigengWorkbenchRow) -> i32 {
    match row.lane {
        ZhuigengWorkbenchLane::CompleteAfterUpdate => 92,
        ZhuigengWorkbenchLane::UpdateNeeded => 88,
        ZhuigengWorkbenchLane::ArchiveReady => 82,
        _ => 55,
    }
}

fn zhuigeng_reasons(row: &ZhuigengWorkbenchRow) -> Vec<String> {
    let item = &row.item;
    let mut reasons = Vec::new();
    if let Some(hint) = item.behind_hint.as_deref().and_then(non_empty_trimmed) {
        reasons.push(hint.to_string());
    }
    if item.behind > 0 {
        reasons.push(format!("本地落后 {} 集，需要找资源补齐。", item.behind));
    }
    if item.ended {
        reasons.push("剧集已完结，补齐后适合归档到正式库。".to_string());
    }
    if item.continuing {
        reasons.push("剧集仍在更新，适合保持在追更流程中。".to_string());
    }
    reasons.extend(row.blockers.iter().cloned());
    if reasons.is_empty() {
        reasons.push(row.action.clone());
    }
    reasons
}

fn zhuigeng_evidence(row: &ZhuigengWorkbenchRow, now: DateTime<Utc>) -> Vec<SmartEvidence> {
    let item = &row.item;
    vec![
        SmartEvidence {
            source: SmartEvidenceSource::TmdbMetadata,
            label: "追更/TMDb 状态".to_string(),
            value: json!({
                "tmdb": item.tmdb,
                "tmdb_status": item.tmdb_status,
                "state": item.state,
                "continuing": item.continuing,
                "ended": item.ended,
                "behind": item.behind,
                "behind_hint": item.behind_hint,
                "last_episode_to_air": item.last_episode_to_air,
                "next_episode_to_air": item.next_episode_to_air,
                "lane": row.lane,
            }),
            weight: zhuigeng_score(row),
            collected_at: now,
        },
        SmartEvidence {
            source: SmartEvidenceSource::EmbyEpisodes,
            label: "Emby 本地剧集".to_string(),
            value: json!({
                "lib": item.lib,
                "id": item.id,
                "folder": item.folder,
                "local_count": item.local_count,
                "local_latest": item.local_latest,
                "local_latest_episode": item.local_latest_episode,
                "resource_hint": item.resource_hint,
            }),
            weight: 70,
            collected_at: now,
        },
    ]
}

fn zhuigeng_execution_plan(
    row: &ZhuigengWorkbenchRow,
    action_type: SmartActionType,
    item_ref: &ZhuigengItemRef,
) -> SmartExecutionPlan {
    let mut steps = vec![
        SmartExecutionStep {
            key: "open_context".to_string(),
            title: "打开追更检查上下文".to_string(),
            executor: SmartExecutorKind::OpenTab,
            params: json!({ "tab": "zhuigeng", "item": item_ref }),
            rollback: None,
        },
        SmartExecutionStep {
            key: "review_evidence".to_string(),
            title: "复核追更状态和资源线索".to_string(),
            executor: SmartExecutorKind::ManualConfirm,
            params: json!({ "lane": row.lane, "blockers": row.blockers }),
            rollback: None,
        },
    ];
    if action_type == SmartActionType::TransferUpdateSeries {
        steps.push(SmartExecutionStep {
            key: "resource_plan".to_string(),
            title: "生成找资源候选".to_string(),
            executor: SmartExecutorKind::ExistingEndpoint,
            params: json!({
                "endpoint": "/api/v2/zhuigeng/resource-plan",
                "request": {
                    "item": item_ref,
                    "limit": 24,
                    "exact": false,
                }
            }),
            rollback: None,
        });
        steps.push(SmartExecutionStep {
            key: "zhuigeng_update_execute".to_string(),
            title: "确认候选后一条龙更新".to_string(),
            executor: SmartExecutorKind::TaskPipeline,
            params: json!({
                "endpoint": "/api/v2/zhuigeng/update/execute",
                "requires_candidate": true,
                "item": item_ref,
            }),
            rollback: Some(SmartRollbackStep {
                title: "按一条龙 undo/audit 回滚新增 STRM 和旧版本处理".to_string(),
                params: json!({ "source": "wizard.add_new" }),
            }),
        });
    } else {
        steps.push(SmartExecutionStep {
            key: "zhuigeng_archive_execute".to_string(),
            title: "确认目标库后归档".to_string(),
            executor: SmartExecutorKind::TaskPipeline,
            params: json!({
                "endpoint": "/api/v2/zhuigeng/archive/execute",
                "requires_policy_param": "archive_to_lib",
                "item": item_ref,
                "on_conflict": "smart",
            }),
            rollback: Some(SmartRollbackStep {
                title: "按 move undo 反向移动回追更库".to_string(),
                params: json!({ "source": "undo_entries" }),
            }),
        });
    }
    SmartExecutionPlan {
        steps,
        estimated_seconds: Some(if action_type == SmartActionType::TransferUpdateSeries {
            240
        } else {
            90
        }),
        concurrency_key: Some("clouddrive".to_string()),
        can_cancel: true,
    }
}

fn zhuigeng_verification_plan(
    row: &ZhuigengWorkbenchRow,
    action_type: SmartActionType,
) -> SmartVerificationPlan {
    let expected = if action_type == SmartActionType::TransferUpdateSeries {
        "新集 STRM 生成，Emby 条目可见，旧版本冲突被处理"
    } else {
        "条目从追更库移出，并在归档目标库可见"
    };
    SmartVerificationPlan {
        checks: vec![SmartVerificationCheck {
            key: "zhuigeng_row_resolved".to_string(),
            title: "追更状态已解决".to_string(),
            source: SmartEvidenceSource::TmdbMetadata,
            expected: expected.to_string(),
        }],
        success_message: format!("{} 已完成：{}", row.item.name, expected),
        partial_message: "追更动作已提交，但仍需复查 STRM、Emby 可见性和旧目录清理。".to_string(),
    }
}

fn smart_actions_from_catalog_candidates(
    items: &[CatalogItem],
    context: Option<&CatalogLibraryContextResponse>,
    now: DateTime<Utc>,
    limit: usize,
) -> Vec<SmartAction> {
    let mut candidates = items
        .iter()
        .filter_map(|item| {
            let recommendation = catalog_candidate_recommendation(item);
            catalog_candidate_is_actionable(item, &recommendation).then_some((item, recommendation))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|(left_item, left_rec), (right_item, right_rec)| {
        right_rec
            .score
            .cmp(&left_rec.score)
            .then_with(|| left_item.name.len().cmp(&right_item.name.len()))
            .then_with(|| left_item.name.cmp(&right_item.name))
    });
    candidates
        .into_iter()
        .take(limit.clamp(1, 24))
        .map(|(item, recommendation)| {
            smart_action_from_catalog_candidate(item, &recommendation, context, now)
        })
        .collect()
}

fn catalog_candidate_recommendation(item: &CatalogItem) -> CatalogResourceRecommendation {
    item.recommendation
        .clone()
        .unwrap_or_else(|| CatalogResourceRecommendation {
            score: if item.transfer { 72 } else { 0 },
            level: if item.transfer { "warn" } else { "skip" }.to_string(),
            action: if item.transfer {
                "谨慎确认"
            } else {
                "暂不推荐"
            }
            .to_string(),
            reasons: vec![
                if item.transfer {
                    "候选资源可转存，但缺少 Emby 上下文评分，需要人工确认。"
                } else {
                    "资源类型暂不支持一条龙转存。"
                }
                .to_string(),
            ],
            episode_ranges: Vec::new(),
            covers_missing: false,
            duplicate_risk: false,
            already_have: false,
        })
}

fn catalog_candidate_is_actionable(
    item: &CatalogItem,
    recommendation: &CatalogResourceRecommendation,
) -> bool {
    item.transfer
        && recommendation.level != "skip"
        && !recommendation.already_have
        && non_empty_trimmed(&item.link).is_some()
}

fn smart_action_from_catalog_candidate(
    item: &CatalogItem,
    recommendation: &CatalogResourceRecommendation,
    context: Option<&CatalogLibraryContextResponse>,
    now: DateTime<Utc>,
) -> SmartAction {
    let action_type = SmartActionType::TransferAddNew;
    let mut risk = smart_risk(action_type);
    if recommendation.duplicate_risk {
        risk.warnings
            .push("Emby 库内已有重复迹象，转存后建议立刻进入去重确认。".to_string());
    }
    let policy = smart_policy(action_type, risk.level);
    let subject = catalog_candidate_subject(item, context);
    let title = format!("候选转存：{}", item.name);
    let summary = catalog_candidate_summary(item, recommendation, context);
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "catalog_transfer_add_new:{}:{}:{}",
            item.link,
            item.name,
            context.map(|value| value.query.as_str()).unwrap_or("")
        )
        .as_bytes(),
    );

    SmartAction {
        id,
        action_type,
        status: SmartActionStatus::Suggested,
        subject,
        title,
        summary,
        recommendation: SmartRecommendation {
            score: recommendation.score,
            confidence: confidence_from_score(recommendation.score),
            primary_action: recommendation.action.clone(),
            reasons: catalog_candidate_reasons(recommendation, context),
            alternatives: catalog_candidate_alternatives(recommendation),
        },
        evidence: catalog_candidate_evidence(item, recommendation, context, now),
        plan: catalog_candidate_execution_plan(item, recommendation, context),
        risk,
        policy,
        verification: catalog_candidate_verification_plan(item, recommendation),
        source: "catalog.candidate".to_string(),
        tab: "catalog".to_string(),
        action_label: recommendation.action.clone(),
        created_at: now,
        updated_at: now,
    }
}

fn catalog_candidate_subject(
    item: &CatalogItem,
    context: Option<&CatalogLibraryContextResponse>,
) -> SmartSubject {
    let first_context_item = context.and_then(|value| value.items.first());
    let context_query = context
        .and_then(|value| non_empty_trimmed(&value.query))
        .map(ToString::to_string);
    let tmdb = context
        .and_then(|value| {
            (value.summary.tmdb_ids.len() == 1).then(|| value.summary.tmdb_ids[0].clone())
        })
        .or_else(|| first_context_item.and_then(|value| value.tmdb.clone()));
    let lib = context
        .and_then(|value| {
            (value.summary.libraries.len() == 1).then(|| value.summary.libraries[0].clone())
        })
        .or_else(|| first_context_item.and_then(|value| value.library.clone()));
    let year = context
        .and_then(|value| (value.summary.years.len() == 1).then_some(value.summary.years[0]))
        .or_else(|| first_context_item.and_then(|value| value.year));

    SmartSubject {
        kind: catalog_candidate_subject_kind(
            item,
            first_context_item.map(|value| &value.item_type),
        ),
        name: context_query.unwrap_or_else(|| item.name.clone()),
        year,
        tmdb,
        emby_id: first_context_item.and_then(|value| value.id.clone()),
        lib,
        folder: first_context_item.and_then(|value| value.folder.clone()),
        strm_path: first_context_item.and_then(|value| value.path.clone()),
        cd_path: None,
    }
}

fn catalog_candidate_subject_kind(
    item: &CatalogItem,
    item_type: Option<&String>,
) -> SmartSubjectKind {
    if let Some(kind) = item_type {
        let mapped = smart_subject_kind_from_item_type(kind);
        if mapped != SmartSubjectKind::Unknown {
            return mapped;
        }
    }
    let lower_name = item.name.to_ascii_lowercase();
    if item.is_pkg
        || lower_name.contains("s0")
        || lower_name.contains("season")
        || item.name.contains('第')
    {
        SmartSubjectKind::Series
    } else if lower_name.contains("movie") || item.sheet.contains("电影") {
        SmartSubjectKind::Movie
    } else {
        SmartSubjectKind::Unknown
    }
}

fn catalog_candidate_summary(
    item: &CatalogItem,
    recommendation: &CatalogResourceRecommendation,
    context: Option<&CatalogLibraryContextResponse>,
) -> String {
    if recommendation.covers_missing {
        let ranges = if recommendation.episode_ranges.is_empty() {
            "候选资源覆盖当前缺口".to_string()
        } else {
            format!("资源集数 {}", recommendation.episode_ranges.join("、"))
        };
        return format!("{ranges}，适合从找资源页确认后一条龙转存。");
    }
    if context.is_some_and(|value| value.summary.matched) {
        return format!(
            "Emby 已找到同名条目，候选资源评分 {}，转存前需要确认是否会形成重复版本。",
            recommendation.score
        );
    }
    format!(
        "{} 可作为新资源候选，建议在找资源页选择目标库/cid 后执行。",
        item.name
    )
}

fn catalog_candidate_reasons(
    recommendation: &CatalogResourceRecommendation,
    context: Option<&CatalogLibraryContextResponse>,
) -> Vec<String> {
    let mut reasons = recommendation.reasons.clone();
    if !recommendation.episode_ranges.is_empty() {
        reasons.push(format!(
            "候选标题解析到集数：{}。",
            recommendation.episode_ranges.join("、")
        ));
    }
    if let Some(context) = context {
        if let Some(note) = non_empty_trimmed(&context.summary.note) {
            reasons.push(format!("Emby 现状：{note}"));
        }
        if !context.summary.missing_ranges.is_empty() {
            reasons.push(format!(
                "本地缺口：{}。",
                context.summary.missing_ranges.join("、")
            ));
        }
        if context.summary.duplicate {
            reasons.push(format!(
                "本库已有 {} 组重复，转存后需要联动去重。",
                context.summary.duplicate_groups
            ));
        }
    }
    if reasons.is_empty() {
        reasons.push("候选资源可转存，但需要人工确认目标库和资源匹配度。".to_string());
    }
    reasons
}

fn catalog_candidate_alternatives(
    recommendation: &CatalogResourceRecommendation,
) -> Vec<SmartAlternative> {
    let mut alternatives = vec![SmartAlternative {
        action: "返回找资源页手动选择其它候选".to_string(),
        reason: "资源标题、集数或清晰度不确定时，先看完整候选列表更稳。".to_string(),
    }];
    if recommendation.duplicate_risk {
        alternatives.push(SmartAlternative {
            action: "先打开去重页复核现有重复".to_string(),
            reason: "库内已有重复时继续转存可能扩大重复面。".to_string(),
        });
    }
    alternatives
}

fn catalog_candidate_evidence(
    item: &CatalogItem,
    recommendation: &CatalogResourceRecommendation,
    context: Option<&CatalogLibraryContextResponse>,
    now: DateTime<Utc>,
) -> Vec<SmartEvidence> {
    let mut evidence = vec![SmartEvidence {
        source: SmartEvidenceSource::CatalogCandidate,
        label: "找资源候选".to_string(),
        value: json!({
            "item": item,
            "recommendation": recommendation,
        }),
        weight: recommendation.score,
        collected_at: now,
    }];
    if let Some(context) = context {
        evidence.push(SmartEvidence {
            source: if context.summary.matched {
                SmartEvidenceSource::EmbyEpisodes
            } else {
                SmartEvidenceSource::EmbyItem
            },
            label: "Emby 库内现状".to_string(),
            value: serde_json::to_value(context).unwrap_or_else(|_| json!({})),
            weight: if context.summary.matched { 80 } else { 45 },
            collected_at: now,
        });
    }
    evidence
}

fn catalog_candidate_execution_plan(
    item: &CatalogItem,
    recommendation: &CatalogResourceRecommendation,
    context: Option<&CatalogLibraryContextResponse>,
) -> SmartExecutionPlan {
    let transfer_item = catalog_candidate_transfer_item(item);
    SmartExecutionPlan {
        steps: vec![
            SmartExecutionStep {
                key: "open_context".to_string(),
                title: "打开找资源上下文".to_string(),
                executor: SmartExecutorKind::OpenTab,
                params: json!({
                    "tab": "catalog",
                    "q": context.map(|value| value.query.as_str()).unwrap_or(item.name.as_str()),
                    "link": item.link,
                }),
                rollback: None,
            },
            SmartExecutionStep {
                key: "review_candidate".to_string(),
                title: "确认候选资源、目标库和 cid".to_string(),
                executor: SmartExecutorKind::ManualConfirm,
                params: json!({
                    "candidate": transfer_item,
                    "recommendation": recommendation,
                    "requires_target": true,
                    "requires_existing_transfer_dialog": true,
                }),
                rollback: None,
            },
            SmartExecutionStep {
                key: "catalog_transfer_plan".to_string(),
                title: "生成转存计划".to_string(),
                executor: SmartExecutorKind::ExistingEndpoint,
                params: json!({
                    "endpoint": "/api/v2/catalog/transfer-plan",
                    "request": {
                        "item": transfer_item,
                        "target": { "requires_lib_or_cid": true }
                    }
                }),
                rollback: None,
            },
            SmartExecutionStep {
                key: "catalog_transfer_execute".to_string(),
                title: "通过找资源页一条龙转存".to_string(),
                executor: SmartExecutorKind::TaskPipeline,
                params: json!({
                    "endpoint": "/api/v2/catalog/transfer/execute",
                    "item": transfer_item,
                    "requires_target": true,
                    "requires_confirm_dialog": true,
                    "auto_scan_after_transfer": true,
                }),
                rollback: Some(SmartRollbackStep {
                    title: "按一条龙 undo/audit 回滚新增 STRM 和 115 转存记录".to_string(),
                    params: json!({ "source": "catalog_transfer_execute" }),
                }),
            },
        ],
        estimated_seconds: Some(240),
        concurrency_key: Some("clouddrive".to_string()),
        can_cancel: true,
    }
}

fn catalog_candidate_transfer_item(item: &CatalogItem) -> Value {
    json!({
        "name": item.name,
        "sheet": item.sheet,
        "link": item.link,
        "is_pkg": item.is_pkg,
        "link_type": item.link_type,
        "share": item.share,
        "rc": item.rc,
    })
}

fn catalog_candidate_verification_plan(
    item: &CatalogItem,
    recommendation: &CatalogResourceRecommendation,
) -> SmartVerificationPlan {
    let mut checks = vec![
        SmartVerificationCheck {
            key: "catalog_transfer_done".to_string(),
            title: "115 转存或离线任务成功".to_string(),
            source: SmartEvidenceSource::C115Resource,
            expected: "目标 cid 下出现候选资源，任务中心没有失败阶段".to_string(),
        },
        SmartVerificationCheck {
            key: "emby_visible".to_string(),
            title: "Emby 媒体库可见".to_string(),
            source: SmartEvidenceSource::EmbyItem,
            expected: "STRM 生成并完成扫库，Emby 可搜索到对应条目".to_string(),
        },
        SmartVerificationCheck {
            key: "poster_after_transfer".to_string(),
            title: "海报/TMDb 状态已修复".to_string(),
            source: SmartEvidenceSource::PosterDetection,
            expected: "缺 tmdbid 或缺海报时自动进入海报修复链路，不留下裸条目".to_string(),
        },
    ];
    if recommendation.duplicate_risk {
        checks.push(SmartVerificationCheck {
            key: "dedup_after_transfer".to_string(),
            title: "重复资源已复核".to_string(),
            source: SmartEvidenceSource::DedupAnalysis,
            expected: "同剧旧目录不并存，必要时已生成去重建议。".to_string(),
        });
    }
    SmartVerificationPlan {
        checks,
        success_message: format!("{} 已完成转存、扫库和海报复查。", item.name),
        partial_message: "转存任务已提交，但仍需复查 Emby 可见性、海报和重复旧目录。".to_string(),
    }
}

fn smart_action_from_dedup_group(group: &DedupGroup, now: DateTime<Utc>) -> SmartAction {
    let action_type = SmartActionType::DedupRemoveOld;
    let risk = smart_risk(action_type);
    let policy = smart_policy(action_type, risk.level);
    let remove_refs = dedup_remove_refs(&group.remove);
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        dedup_group_fingerprint(group).as_bytes(),
    );
    SmartAction {
        id,
        action_type,
        status: SmartActionStatus::Suggested,
        subject: SmartSubject {
            kind: dedup_subject_kind(&group.keep),
            name: group.keep.folder.clone(),
            year: None,
            tmdb: non_empty_trimmed(&group.tmdb).map(ToString::to_string),
            emby_id: group.keep.item_id.clone(),
            lib: Some(group.keep.lib.clone()),
            folder: Some(group.keep.folder.clone()),
            strm_path: None,
            cd_path: None,
        },
        title: format!("删除重复旧版本：{}", group.keep.folder),
        summary: format!(
            "保留 {}/{}，建议删除 {} 个重复目录。",
            group.keep.lib,
            group.keep.folder,
            group.remove.len()
        ),
        recommendation: SmartRecommendation {
            score: 94,
            confidence: SmartConfidence::High,
            primary_action: "确认后批量去重删除旧版本".to_string(),
            reasons: vec![
                format!("同一 TMDb {} 命中多个目录。", group.tmdb),
                format!(
                    "保留项得分 {}，删除候选 {} 个。",
                    group.keep.score,
                    group.remove.len()
                ),
                "删除类动作涉及 Emby、磁盘、CloudDrive/115，必须人工确认。".to_string(),
            ],
            alternatives: vec![SmartAlternative {
                action: "打开去重页逐项复核".to_string(),
                reason: "路径、画质或字幕信息不确定时，先人工选择保留项。".to_string(),
            }],
        },
        evidence: vec![SmartEvidence {
            source: SmartEvidenceSource::DedupAnalysis,
            label: "去重自动组".to_string(),
            value: serde_json::to_value(group).unwrap_or_else(|_| json!({})),
            weight: 94,
            collected_at: now,
        }],
        plan: SmartExecutionPlan {
            steps: vec![
                SmartExecutionStep {
                    key: "open_context".to_string(),
                    title: "打开去重上下文".to_string(),
                    executor: SmartExecutorKind::OpenTab,
                    params: json!({ "tab": "dedup", "tmdb": group.tmdb }),
                    rollback: None,
                },
                SmartExecutionStep {
                    key: "review_keep_remove".to_string(),
                    title: "复核保留项和删除项".to_string(),
                    executor: SmartExecutorKind::ManualConfirm,
                    params: json!({ "keep": group.keep, "remove": group.remove }),
                    rollback: None,
                },
                SmartExecutionStep {
                    key: "dedup_execute_batch".to_string(),
                    title: "确认后提交批量去重".to_string(),
                    executor: SmartExecutorKind::TaskPipeline,
                    params: json!({
                        "endpoint": "/api/v2/dedup/execute-batch",
                        "request": {
                            "groups": [{
                                "tmdb": group.tmdb,
                                "remove": remove_refs,
                            }]
                        }
                    }),
                    rollback: Some(SmartRollbackStep {
                        title: "按 undo_entries 恢复删除或移动影响".to_string(),
                        params: json!({ "source": "undo_entries" }),
                    }),
                },
            ],
            estimated_seconds: Some(120),
            concurrency_key: Some("clouddrive".to_string()),
            can_cancel: true,
        },
        risk,
        policy,
        verification: SmartVerificationPlan {
            checks: vec![SmartVerificationCheck {
                key: "duplicate_group_removed".to_string(),
                title: "重复组消失或减少".to_string(),
                source: SmartEvidenceSource::DedupAnalysis,
                expected: "重复组数量下降，删除/移动记录写入 undo/audit".to_string(),
            }],
            success_message: "重复旧版本已删除，并写入 undo/audit。".to_string(),
            partial_message: "去重任务结束但重复信号仍存在，需要人工复核失败项。".to_string(),
        },
        source: "dedup.auto_group".to_string(),
        tab: "dedup".to_string(),
        action_label: "确认去重删除".to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn smart_action_from_dedup_review_group(
    group: &DedupReviewGroup,
    now: DateTime<Utc>,
) -> SmartAction {
    let action_type = SmartActionType::DedupReview;
    let risk = smart_risk(action_type);
    let policy = smart_policy(action_type, risk.level);
    let first = group.rows.first();
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "dedup_review:{}:{}",
            group.tmdb,
            group
                .rows
                .iter()
                .map(|row| format!("{}/{}", row.lib, row.folder))
                .collect::<Vec<_>>()
                .join("|")
        )
        .as_bytes(),
    );
    SmartAction {
        id,
        action_type,
        status: SmartActionStatus::Suggested,
        subject: SmartSubject {
            kind: first
                .map(dedup_subject_kind)
                .unwrap_or(SmartSubjectKind::Unknown),
            name: first
                .map(|row| row.folder.clone())
                .unwrap_or_else(|| format!("TMDb {}", group.tmdb)),
            year: None,
            tmdb: non_empty_trimmed(&group.tmdb).map(ToString::to_string),
            emby_id: first.and_then(|row| row.item_id.clone()),
            lib: first.map(|row| row.lib.clone()),
            folder: first.map(|row| row.folder.clone()),
            strm_path: None,
            cd_path: None,
        },
        title: format!("复核重复资源：{}", group.tmdb),
        summary: group.why.clone(),
        recommendation: SmartRecommendation {
            score: 74,
            confidence: SmartConfidence::Medium,
            primary_action: "人工复核重复资源".to_string(),
            reasons: vec![
                group.why.clone(),
                format!("{} 个候选目录需要确认保留项。", group.rows.len()),
                "证据不足以自动删除，先不要提交删除任务。".to_string(),
            ],
            alternatives: Vec::new(),
        },
        evidence: vec![SmartEvidence {
            source: SmartEvidenceSource::DedupAnalysis,
            label: "去重人工复核组".to_string(),
            value: serde_json::to_value(group).unwrap_or_else(|_| json!({})),
            weight: 74,
            collected_at: now,
        }],
        plan: SmartExecutionPlan {
            steps: vec![
                SmartExecutionStep {
                    key: "open_context".to_string(),
                    title: "打开去重页".to_string(),
                    executor: SmartExecutorKind::OpenTab,
                    params: json!({ "tab": "dedup", "tmdb": group.tmdb }),
                    rollback: None,
                },
                SmartExecutionStep {
                    key: "review_evidence".to_string(),
                    title: "人工选择保留项和删除项".to_string(),
                    executor: SmartExecutorKind::ManualConfirm,
                    params: json!({ "why": group.why, "rows": group.rows }),
                    rollback: None,
                },
            ],
            estimated_seconds: None,
            concurrency_key: Some("clouddrive".to_string()),
            can_cancel: false,
        },
        risk,
        policy,
        verification: SmartVerificationPlan {
            checks: vec![SmartVerificationCheck {
                key: "manual_review_done".to_string(),
                title: "人工复核完成".to_string(),
                source: SmartEvidenceSource::DedupAnalysis,
                expected: "用户在去重页完成保留/删除选择，或明确忽略该组".to_string(),
            }],
            success_message: "重复资源已人工复核。".to_string(),
            partial_message: "仍需在去重页确认保留项。".to_string(),
        },
        source: "dedup.review_group".to_string(),
        tab: "dedup".to_string(),
        action_label: "复核重复资源".to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn dedup_remove_refs(rows: &[DedupRow]) -> Vec<DedupFolderRef> {
    rows.iter()
        .map(|row| DedupFolderRef {
            lib: row.lib.clone(),
            folder: row.folder.clone(),
            item_id: row.item_id.clone(),
        })
        .collect()
}

fn dedup_group_fingerprint(group: &DedupGroup) -> String {
    let mut remove = group
        .remove
        .iter()
        .map(|row| {
            format!(
                "{}/{}/{}",
                row.lib,
                row.folder,
                row.item_id.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>();
    remove.sort();
    format!(
        "dedup_remove_old:{}:{}/{}:{}",
        group.tmdb,
        group.keep.lib,
        group.keep.folder,
        remove.join("|")
    )
}

fn dedup_subject_kind(row: &DedupRow) -> SmartSubjectKind {
    let text = format!("{} {}", row.lib, row.folder).to_ascii_lowercase();
    if text.contains("剧") || text.contains("season") || text.contains("s0") {
        SmartSubjectKind::Series
    } else if text.contains("电影") || text.contains("movie") {
        SmartSubjectKind::Movie
    } else {
        SmartSubjectKind::Unknown
    }
}

fn smart_poster_execution_plan(item: &PosterSignalItem) -> SmartExecutionPlan {
    SmartExecutionPlan {
        steps: vec![
            SmartExecutionStep {
                key: "open_context".to_string(),
                title: "打开海报修复上下文".to_string(),
                executor: SmartExecutorKind::OpenTab,
                params: json!({ "tab": "posters", "id": item.id, "lib": item.lib }),
                rollback: None,
            },
            SmartExecutionStep {
                key: "review_evidence".to_string(),
                title: "复核海报/TMDb 证据".to_string(),
                executor: SmartExecutorKind::ManualConfirm,
                params: json!({
                    "signals": item.signals,
                    "reasons": item.reasons,
                    "tmdb": item.tmdb,
                    "declared_tmdb": item.declared_tmdb,
                }),
                rollback: None,
            },
            SmartExecutionStep {
                key: "execute_low_risk".to_string(),
                title: "执行单项海报修复".to_string(),
                executor: SmartExecutorKind::ExistingEndpoint,
                params: json!({
                    "module": "posters",
                    "function": "fix_poster_one",
                    "id": item.id,
                    "type": item.item_type,
                }),
                rollback: Some(SmartRollbackStep {
                    title: "通过海报 rebind undo 恢复旧 ProviderId".to_string(),
                    params: json!({ "source": "undo_entries" }),
                }),
            },
        ],
        estimated_seconds: Some(30),
        concurrency_key: None,
        can_cancel: true,
    }
}

fn smart_subject_kind_from_item_type(item_type: &str) -> SmartSubjectKind {
    match item_type.to_ascii_lowercase().as_str() {
        "movie" | "movies" => SmartSubjectKind::Movie,
        "series" | "tvshow" | "tvshows" | "show" => SmartSubjectKind::Series,
        "season" => SmartSubjectKind::Season,
        "episode" => SmartSubjectKind::Episode,
        _ => SmartSubjectKind::Unknown,
    }
}

fn action_type_from_dashboard(item: &DashboardSmartAction) -> SmartActionType {
    match item.source.as_str() {
        "dashboard_todo.noposter" => SmartActionType::PosterFix,
        "dashboard_todo.no_rating" => SmartActionType::MetadataRefresh,
        "dedup.auto_groups" => SmartActionType::DedupRemoveOld,
        "dedup.review_groups" => SmartActionType::DedupReview,
        "zhuigeng.update_needed" | "zhuigeng.complete_after_update" => {
            SmartActionType::TransferUpdateSeries
        }
        "zhuigeng.archive_ready" => SmartActionType::ArchiveSeries,
        "zhuigeng.errors" => SmartActionType::MetadataRefresh,
        "task_runs" => SmartActionType::TaskRetryOrDiagnose,
        _ if item.area == "posters" => SmartActionType::PosterFix,
        _ if item.area == "dedup" => SmartActionType::DedupReview,
        _ if item.area == "zhuigeng" => SmartActionType::TransferUpdateSeries,
        _ => SmartActionType::LibraryScan,
    }
}

fn smart_subject_from_dashboard(item: &DashboardSmartAction) -> SmartSubject {
    let kind = match item.area.as_str() {
        "tasks" => SmartSubjectKind::Task,
        "system" => SmartSubjectKind::System,
        _ => SmartSubjectKind::Library,
    };
    SmartSubject {
        kind,
        name: item.title.clone(),
        year: None,
        tmdb: None,
        emby_id: None,
        lib: None,
        folder: None,
        strm_path: None,
        cd_path: None,
    }
}

fn smart_recommendation(
    item: &DashboardSmartAction,
    action_type: SmartActionType,
) -> SmartRecommendation {
    SmartRecommendation {
        score: score_from_severity(&item.severity),
        confidence: confidence_from_severity(&item.severity),
        primary_action: item.action.clone(),
        reasons: recommendation_reasons(item, action_type),
        alternatives: recommendation_alternatives(action_type),
    }
}

fn recommendation_reasons(
    item: &DashboardSmartAction,
    action_type: SmartActionType,
) -> Vec<String> {
    let mut reasons = vec![format!("{} 个对象命中 {}", item.count, item.source)];
    reasons.push(item.message.clone());
    match action_type {
        SmartActionType::PosterFix => {
            reasons.push("海报修复属于中风险动作，通常只影响 Emby 元数据。".to_string());
        }
        SmartActionType::DedupRemoveOld => {
            reasons.push("已有明确保留/删除候选，但删除类动作仍需要确认。".to_string());
        }
        SmartActionType::TransferUpdateSeries => {
            reasons
                .push("追更更新会复用找资源和一条龙转存，执行后必须验证 Emby 可见性。".to_string());
        }
        SmartActionType::ArchiveSeries => {
            reasons.push("归档会移动目录并刷新库，必须先确认目标库和路径。".to_string());
        }
        SmartActionType::TaskRetryOrDiagnose => {
            reasons.push("失败任务需要先诊断阶段结果，再决定重试或跳转修复。".to_string());
        }
        _ => {}
    }
    reasons
}

fn recommendation_alternatives(action_type: SmartActionType) -> Vec<SmartAlternative> {
    match action_type {
        SmartActionType::DedupRemoveOld | SmartActionType::DedupReview => vec![SmartAlternative {
            action: "只打开去重页复核，不立即删除".to_string(),
            reason: "重复资源可能涉及跨库和 115 删除，人工复核更稳。".to_string(),
        }],
        SmartActionType::TransferUpdateSeries => vec![SmartAlternative {
            action: "只生成找资源候选，不执行转存".to_string(),
            reason: "资源标题或集数不确定时先看候选列表。".to_string(),
        }],
        SmartActionType::ArchiveSeries => vec![SmartAlternative {
            action: "暂时忽略归档，仅保留在追更库".to_string(),
            reason: "目标库或路径映射未确认前不移动目录。".to_string(),
        }],
        _ => Vec::new(),
    }
}

fn evidence_source_from_dashboard(item: &DashboardSmartAction) -> SmartEvidenceSource {
    match item.area.as_str() {
        "posters" => SmartEvidenceSource::PosterDetection,
        "dedup" => SmartEvidenceSource::DedupAnalysis,
        "zhuigeng" => SmartEvidenceSource::TmdbMetadata,
        "tasks" => SmartEvidenceSource::TaskHistory,
        "cleanup" => SmartEvidenceSource::EmbyItem,
        _ => SmartEvidenceSource::DashboardTodo,
    }
}

fn smart_risk(action_type: SmartActionType) -> SmartRisk {
    let (level, destructive, touches_emby, touches_disk, touches_c115, confirm, warnings) =
        match action_type {
            SmartActionType::PosterFix | SmartActionType::MetadataRefresh => (
                SmartRiskLevel::Medium,
                false,
                true,
                false,
                false,
                None,
                vec!["会修改或刷新 Emby 元数据，执行后需要复查条目状态。".to_string()],
            ),
            SmartActionType::LibraryScan => (
                SmartRiskLevel::Low,
                false,
                true,
                false,
                false,
                None,
                Vec::new(),
            ),
            SmartActionType::TaskRetryOrDiagnose => (
                SmartRiskLevel::Medium,
                false,
                true,
                false,
                false,
                None,
                Vec::new(),
            ),
            SmartActionType::TransferAddNew | SmartActionType::TransferUpdateSeries => (
                SmartRiskLevel::High,
                false,
                true,
                true,
                true,
                Some("执行".to_string()),
                vec![
                    "会触发 115/CloudDrive、STRM 和 Emby 扫描，CloudDrive 步骤必须串行。"
                        .to_string(),
                ],
            ),
            SmartActionType::ArchiveSeries => (
                SmartRiskLevel::High,
                true,
                true,
                true,
                false,
                Some("归档".to_string()),
                vec!["会移动媒体目录并刷新 Emby，执行前必须确认目标库。".to_string()],
            ),
            SmartActionType::DedupRemoveOld | SmartActionType::CleanupEmptyFolder => (
                SmartRiskLevel::Critical,
                true,
                true,
                true,
                true,
                Some("删除".to_string()),
                vec![
                    "可能删除 Emby 条目、STRM、CloudDrive 或 115 资源，必须写入 undo/audit。"
                        .to_string(),
                ],
            ),
            SmartActionType::DedupReview => (
                SmartRiskLevel::High,
                true,
                true,
                true,
                true,
                Some("确认".to_string()),
                vec!["证据不足以自动删除，需要人工确认保留项和删除项。".to_string()],
            ),
        };
    SmartRisk {
        level,
        destructive,
        touches_emby,
        touches_disk,
        touches_c115,
        requires_confirm_text: confirm,
        warnings,
    }
}

fn smart_policy(action_type: SmartActionType, risk: SmartRiskLevel) -> SmartPolicyDecision {
    let mode = match action_type {
        SmartActionType::PosterFix
        | SmartActionType::MetadataRefresh
        | SmartActionType::LibraryScan => SmartPolicyMode::Auto,
        _ => SmartPolicyMode::Confirm,
    };
    SmartPolicyDecision {
        enabled: true,
        mode,
        max_risk: if mode == SmartPolicyMode::Auto {
            SmartRiskLevel::Medium
        } else {
            risk
        },
        reason: match mode {
            SmartPolicyMode::Auto => "低/中风险动作可在后续执行器阶段自动处理。".to_string(),
            SmartPolicyMode::Confirm => {
                "当前动作涉及高风险或跨系统修改，需要用户确认。".to_string()
            }
            SmartPolicyMode::Disabled => "策略禁用。".to_string(),
        },
    }
}

fn smart_execution_plan(
    item: &DashboardSmartAction,
    action_type: SmartActionType,
) -> SmartExecutionPlan {
    let mut steps = vec![SmartExecutionStep {
        key: "open_context".to_string(),
        title: format!("打开{}上下文", item.action),
        executor: SmartExecutorKind::OpenTab,
        params: json!({ "tab": item.tab }),
        rollback: None,
    }];
    steps.push(SmartExecutionStep {
        key: "review_evidence".to_string(),
        title: "复核证据和风险".to_string(),
        executor: SmartExecutorKind::ManualConfirm,
        params: json!({ "source": item.source, "count": item.count }),
        rollback: None,
    });
    if matches!(
        action_type,
        SmartActionType::PosterFix
            | SmartActionType::MetadataRefresh
            | SmartActionType::LibraryScan
    ) {
        steps.push(SmartExecutionStep {
            key: "execute_low_risk".to_string(),
            title: "执行低/中风险修复".to_string(),
            executor: SmartExecutorKind::ExistingEndpoint,
            params: json!({ "phase": "phase_3_executor_pending" }),
            rollback: None,
        });
    } else {
        steps.push(SmartExecutionStep {
            key: "queue_pipeline".to_string(),
            title: "提交一条龙或高风险任务".to_string(),
            executor: SmartExecutorKind::TaskPipeline,
            params: json!({ "phase": "future_executor", "action_type": action_type.as_str() }),
            rollback: Some(SmartRollbackStep {
                title: "按 undo/audit 记录回滚".to_string(),
                params: json!({ "requires_audit": true }),
            }),
        });
    }
    SmartExecutionPlan {
        steps,
        estimated_seconds: None,
        concurrency_key: concurrency_key(action_type),
        can_cancel: true,
    }
}

fn concurrency_key(action_type: SmartActionType) -> Option<String> {
    match action_type {
        SmartActionType::TransferAddNew
        | SmartActionType::TransferUpdateSeries
        | SmartActionType::ArchiveSeries
        | SmartActionType::DedupRemoveOld
        | SmartActionType::DedupReview
        | SmartActionType::CleanupEmptyFolder => Some("clouddrive".to_string()),
        _ => None,
    }
}

fn smart_verification_plan(
    item: &DashboardSmartAction,
    action_type: SmartActionType,
) -> SmartVerificationPlan {
    let source = evidence_source_from_dashboard(item);
    let expected = match action_type {
        SmartActionType::PosterFix => "无海报数量下降，相关 Emby 条目出现 Primary 图片",
        SmartActionType::MetadataRefresh => "无评分或元数据异常数量下降",
        SmartActionType::TransferUpdateSeries => "新集 STRM 生成，Emby 条目可见，旧版本冲突被处理",
        SmartActionType::ArchiveSeries => "条目从追更库移出，并在目标库可见",
        SmartActionType::DedupRemoveOld | SmartActionType::DedupReview => {
            "重复组数量下降，删除/移动记录写入 undo/audit"
        }
        SmartActionType::TaskRetryOrDiagnose => "失败任务生成明确下一步或重试成功",
        _ => "对应待办数量下降",
    };
    SmartVerificationPlan {
        checks: vec![SmartVerificationCheck {
            key: "source_count_decreases".to_string(),
            title: "复查待办数量".to_string(),
            source,
            expected: expected.to_string(),
        }],
        success_message: "动作已执行并通过业务验收。".to_string(),
        partial_message: "执行完成但业务验收未通过，需要生成后续诊断动作。".to_string(),
    }
}

fn summarize_actions(actions: &[SmartAction]) -> SmartActionsSummary {
    let mut summary = SmartActionsSummary {
        total: actions.len(),
        ..SmartActionsSummary::default()
    };
    for action in actions {
        match action.status {
            SmartActionStatus::Suggested => summary.suggested += 1,
            SmartActionStatus::Running
            | SmartActionStatus::Queued
            | SmartActionStatus::Verifying => summary.running += 1,
            SmartActionStatus::Failed | SmartActionStatus::Partial => summary.failed += 1,
            _ => {}
        }
        match action.risk.level {
            SmartRiskLevel::Low => summary.low += 1,
            SmartRiskLevel::Medium => summary.medium += 1,
            SmartRiskLevel::High => summary.high += 1,
            SmartRiskLevel::Critical => summary.critical += 1,
        }
        match action.policy.mode {
            SmartPolicyMode::Auto if smart_action_is_batch_auto_ready(action) => {
                summary.auto_ready += 1;
            }
            SmartPolicyMode::Confirm => summary.confirm_required += 1,
            _ => {}
        }
    }
    summary
}

fn score_from_severity(severity: &str) -> i32 {
    match severity {
        "high" => 90,
        "medium" => 70,
        "low" => 45,
        _ => 30,
    }
}

fn confidence_from_severity(severity: &str) -> SmartConfidence {
    match severity {
        "high" => SmartConfidence::High,
        "medium" => SmartConfidence::Medium,
        _ => SmartConfidence::Low,
    }
}

fn confidence_from_score(score: i32) -> SmartConfidence {
    if score >= 85 {
        SmartConfidence::High
    } else if score >= 60 {
        SmartConfidence::Medium
    } else {
        SmartConfidence::Low
    }
}

fn normalized_filter(value: Option<&str>) -> Option<String> {
    value
        .and_then(non_empty_trimmed)
        .map(|value| value.to_ascii_lowercase())
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn first_non_empty(values: [Option<String>; 2]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

fn task_param_string(params: &Value, key: &str) -> Option<String> {
    let direct = json_string_at(params, &[key])
        .or_else(|| json_string_at(params, &["params", key]))
        .or_else(|| json_string_at(params, &["request", key]))
        .or_else(|| json_string_at(params, &["payload", key]));
    if direct.is_some() {
        return direct;
    }
    let target_key = match key {
        "lib" | "target_lib" => Some("lib"),
        "cid" | "target_cid" => Some("cid"),
        _ => None,
    }?;
    json_string_at(params, &["target", target_key])
        .or_else(|| json_string_at(params, &["params", "target", target_key]))
        .or_else(|| json_string_at(params, &["request", "target", target_key]))
        .or_else(|| json_string_at(params, &["payload", "target", target_key]))
}

fn task_result_has_partial_signal(result: Option<&Value>) -> bool {
    let Some(Value::Object(map)) = result else {
        return false;
    };
    if map.get("ok").and_then(Value::as_bool) == Some(false) {
        return true;
    }
    if let (Some(total), Some(ok_count)) = (
        map.get("total").and_then(Value::as_i64),
        map.get("ok_count").and_then(Value::as_i64),
    ) && ok_count < total
    {
        return true;
    }
    for key in ["error_count", "failed", "failed_count", "stage_error_count"] {
        if map.get(key).and_then(Value::as_i64).unwrap_or(0) > 0 {
            return true;
        }
    }
    if map
        .get("items")
        .or_else(|| map.get("results"))
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("ok").and_then(Value::as_bool) == Some(false)
                    || item
                        .get("status")
                        .and_then(Value::as_str)
                        .is_some_and(|status| matches!(status, "error" | "failed"))
            })
        })
    {
        return true;
    }
    if map
        .get("check")
        .and_then(Value::as_object)
        .and_then(|check| check.get("stage_error_count"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
        > 0
    {
        return true;
    }
    if map
        .get("check")
        .and_then(Value::as_object)
        .is_some_and(|check| {
            check
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| matches!(status, "suspicious" | "errors" | "partial"))
                || check
                    .get("suspicious_count")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    > 0
                || check
                    .get("suspicious")
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty())
        })
    {
        return true;
    }
    for section in [
        "poster",
        "poster_auto_fix",
        "dedup",
        "auto_resolve",
        "transfer",
        "strm",
        "scan",
    ] {
        let Some(section) = map.get(section) else {
            continue;
        };
        if section.get("ok").and_then(Value::as_bool) == Some(false)
            || section
                .get("error_count")
                .or_else(|| section.get("failed"))
                .or_else(|| section.get("failed_count"))
                .or_else(|| section.get("issue_count"))
                .or_else(|| section.get("suspicious_count"))
                .or_else(|| section.get("review_count"))
                .and_then(Value::as_i64)
                .unwrap_or(0)
                > 0
            || section
                .get("items")
                .or_else(|| section.get("results"))
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        item.get("ok").and_then(Value::as_bool) == Some(false)
                            || item
                                .get("status")
                                .and_then(Value::as_str)
                                .is_some_and(|status| matches!(status, "error" | "failed"))
                    })
                })
        {
            return true;
        }
    }
    false
}

fn task_repair_suggestions(task: &TaskRun) -> Vec<SmartNextAction> {
    let mut suggestions = Vec::new();
    let result = task.result.as_ref();
    let text = task_diagnose_text(task);
    if text.contains("tmdb_api_key")
        || text.contains("tmdb_key")
        || text.contains("tmdb api key")
        || text.contains("tmdb") && text.contains("key")
    {
        push_task_repair_suggestion(
            &mut suggestions,
            "config_tmdb",
            "配置 TMDb Key",
            "settings",
            "任务错误指向 TMDb key 缺失或不可用，先补齐配置再重试元数据/海报相关步骤。",
        );
    }

    for (stage, message) in task_result_stages(result) {
        match stage.as_str() {
            "transfer" | "offline" | "c115" | "save" | "receive" => {
                let tab = if task.kind.contains("zhuigeng") {
                    "zhuigeng"
                } else {
                    "catalog"
                };
                let action_type = if task.kind.contains("zhuigeng") {
                    "transfer_update_series"
                } else {
                    "transfer_add_new"
                };
                push_task_repair_suggestion(
                    &mut suggestions,
                    action_type,
                    "重试 115 转存",
                    tab,
                    &format!(
                        "转存阶段失败：{message}。检查 115 cookie、cid、分享链接/提取码后重试。"
                    ),
                );
            }
            "strm" | "media_fs" => push_task_repair_suggestion(
                &mut suggestions,
                "library_scan",
                "重新生成 STRM 并刷新库",
                "scan",
                &format!("STRM 阶段异常：{message}。需要重新生成 STRM，再触发 Emby 刷新。"),
            ),
            "scan" | "emby_scan" | "library_scan" => push_task_repair_suggestion(
                &mut suggestions,
                "library_scan",
                "刷新媒体库",
                "scan",
                &format!("Emby 扫库阶段异常：{message}。先刷新对应库并确认条目可见。"),
            ),
            "poster" | "poster_auto_fix" | "tmdb" | "metadata" => push_task_repair_suggestion(
                &mut suggestions,
                "poster_fix",
                "修复海报/元数据",
                "posters",
                &format!("海报或元数据阶段异常：{message}。进入海报修复页重新匹配 TMDb 和图片。"),
            ),
            "dedup" | "auto_resolve" | "replace" => push_task_repair_suggestion(
                &mut suggestions,
                "dedup_review",
                "复核重复旧版本",
                "dedup",
                &format!("去重阶段仍有风险：{message}。复核保留项、删除项和 undo 记录。"),
            ),
            "archive" | "move" => push_task_repair_suggestion(
                &mut suggestions,
                "archive_review",
                "复查归档/移动",
                "zhuigeng",
                &format!("归档或移动阶段异常：{message}。确认源库移出、目标库可见和路径权限。"),
            ),
            "config" | "settings" => push_task_repair_suggestion(
                &mut suggestions,
                "config_fix",
                "检查配置",
                "settings",
                &format!("配置阶段异常：{message}。先检查相关 key、路径映射和密钥是否完整。"),
            ),
            _ => {}
        }
    }

    suggestions
}

fn push_task_repair_suggestion(
    suggestions: &mut Vec<SmartNextAction>,
    action_type: &str,
    label: &str,
    tab: &str,
    reason: &str,
) {
    if suggestions.iter().any(|suggestion| {
        suggestion.action_type == action_type && suggestion.tab == tab && suggestion.label == label
    }) {
        return;
    }
    suggestions.push(SmartNextAction {
        action_type: action_type.to_string(),
        label: label.to_string(),
        tab: tab.to_string(),
        reason: reason.to_string(),
        subject: None,
    });
}

fn task_diagnose_text(task: &TaskRun) -> String {
    [
        task.kind.clone(),
        task.label.clone(),
        task.status.clone(),
        task.status_text.clone(),
        task.error.clone().unwrap_or_default(),
        task.result
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_default(),
    ]
    .join("\n")
    .to_lowercase()
}

fn task_result_stages(result: Option<&Value>) -> Vec<(String, String)> {
    let Some(result) = result else {
        return Vec::new();
    };
    let mut stages = Vec::new();
    collect_stage_messages(json_at(result, &["check", "errors"]), &mut stages);
    collect_stage_messages(json_at(result, &["check", "suspicious"]), &mut stages);
    collect_stage_messages(json_at(result, &["errors"]), &mut stages);
    collect_stage_messages(json_at(result, &["warnings"]), &mut stages);

    for key in [
        "transfer",
        "strm",
        "scan",
        "poster",
        "poster_auto_fix",
        "dedup",
        "auto_resolve",
    ] {
        let Some(section) = result.get(key) else {
            continue;
        };
        let ok = section.get("ok").and_then(Value::as_bool);
        let error_count = section
            .get("error_count")
            .or_else(|| section.get("failed"))
            .or_else(|| section.get("failed_count"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let review_count = if key == "dedup" {
            section
                .get("review_count")
                .and_then(Value::as_i64)
                .unwrap_or(0)
        } else {
            0
        };
        if ok == Some(false) || error_count > 0 || review_count > 0 {
            let message =
                section_message(section).unwrap_or_else(|| format!("{key} 阶段返回异常信号"));
            stages.push((key.to_string(), message));
        }
    }
    stages
}

fn collect_stage_messages(value: Option<&Value>, stages: &mut Vec<(String, String)>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    for item in items {
        if let Some(stage) = json_string_at(item, &["stage"]) {
            let message = json_string_at(item, &["message"])
                .or_else(|| json_string_at(item, &["error"]))
                .or_else(|| json_string_at(item, &["reason"]))
                .or_else(|| json_string_at(item, &["label"]))
                .unwrap_or_else(|| "阶段异常".to_string());
            stages.push((stage.to_ascii_lowercase(), message));
        }
    }
}

fn section_message(section: &Value) -> Option<String> {
    json_string_at(section, &["error"])
        .or_else(|| json_string_at(section, &["warning"]))
        .or_else(|| json_string_at(section, &["message"]))
        .or_else(|| {
            json_string_array_at(section, &["warnings"]).and_then(|warnings| {
                (!warnings.is_empty())
                    .then(|| warnings.into_iter().take(2).collect::<Vec<_>>().join("；"))
            })
        })
}

fn task_diagnose_reasons(
    task: &TaskRun,
    summary: &str,
    repair_suggestions: &[SmartNextAction],
) -> Vec<String> {
    let mut reasons = Vec::new();
    reasons.push(format!(
        "任务类型 {} 当前状态为 {}。",
        task.kind, task.status
    ));
    if let Some(error) = task.error.as_deref().and_then(non_empty_trimmed) {
        reasons.push(format!("错误信息：{error}"));
    } else if task_result_has_partial_signal(task.result.as_ref()) {
        reasons.push("任务结果包含失败/半成功信号，需要继续拆解阶段结果。".to_string());
    } else if let Some(status_text) = non_empty_trimmed(&task.status_text) {
        reasons.push(format!("状态说明：{status_text}"));
    }
    if !summary.trim().is_empty() && !reasons.iter().any(|reason| reason.contains(summary)) {
        reasons.push(summary.to_string());
    }
    reasons.extend(
        repair_suggestions
            .iter()
            .map(|suggestion| format!("建议动作：{}。{}", suggestion.label, suggestion.reason)),
    );
    reasons
}

fn smart_action_haystack(action: &SmartAction) -> String {
    format!(
        "{} {} {} {} {} {}",
        action.title,
        action.summary,
        action.subject.name,
        action.source,
        action.tab,
        action.action_label
    )
    .to_lowercase()
}

fn smart_action_matches_subject(action: &SmartAction, subject: &SmartSubject) -> bool {
    if subject
        .emby_id
        .as_deref()
        .and_then(non_empty_trimmed)
        .is_some_and(|emby_id| {
            action
                .subject
                .emby_id
                .as_deref()
                .is_some_and(|candidate| candidate == emby_id)
        })
    {
        return true;
    }
    if subject
        .tmdb
        .as_deref()
        .and_then(non_empty_trimmed)
        .is_some_and(|tmdb| {
            action
                .subject
                .tmdb
                .as_deref()
                .is_some_and(|candidate| candidate == tmdb)
        })
    {
        return true;
    }
    if subject
        .folder
        .as_deref()
        .and_then(non_empty_trimmed)
        .is_some_and(|folder| {
            action
                .subject
                .folder
                .as_deref()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(folder))
        })
    {
        return true;
    }
    let name = subject.name.trim();
    !name.is_empty() && action.subject.name.eq_ignore_ascii_case(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dashboard_action(source: &str, area: &str, severity: &str) -> DashboardSmartAction {
        DashboardSmartAction {
            severity: severity.to_string(),
            area: area.to_string(),
            title: "修复无海报条目".to_string(),
            message: "2 个条目缺少主海报".to_string(),
            count: 2,
            tab: area.to_string(),
            action: "打开海报修复".to_string(),
            source: source.to_string(),
        }
    }

    fn dashboard_action_with_title(
        source: &str,
        area: &str,
        severity: &str,
        title: &str,
    ) -> DashboardSmartAction {
        DashboardSmartAction {
            title: title.to_string(),
            message: format!("{title} 待处理"),
            ..dashboard_action(source, area, severity)
        }
    }

    fn empty_query() -> SmartActionsQuery {
        SmartActionsQuery {
            status: None,
            action_type: None,
            risk: None,
            subject_kind: None,
            lib: None,
            q: None,
            limit: None,
            offset: None,
        }
    }

    fn persisted_row_from_action(
        action: &SmartAction,
        status: SmartActionStatus,
    ) -> PersistedSmartActionRow {
        PersistedSmartActionRow {
            id: action.id,
            action_type: action.action_type.as_str().to_string(),
            status: status.as_str().to_string(),
            subject: to_json(&action.subject).unwrap(),
            title: action.title.clone(),
            summary: action.summary.clone(),
            recommendation: to_json(&action.recommendation).unwrap(),
            evidence: to_json(&action.evidence).unwrap(),
            plan: to_json(&action.plan).unwrap(),
            risk: to_json(&action.risk).unwrap(),
            policy: to_json(&action.policy).unwrap(),
            verification: to_json(&action.verification).unwrap(),
            source: action.source.clone(),
            tab: action.tab.clone(),
            action_label: action.action_label.clone(),
            created_at: action.created_at,
            updated_at: action.updated_at,
        }
    }

    #[test]
    fn dashboard_mapping_has_stable_id_and_evidence() {
        let now = Utc::now();
        let action = dashboard_action("dashboard_todo.noposter", "posters", "high");
        let first = smart_action_from_dashboard(&action, now);
        let second = smart_action_from_dashboard(&action, now);
        assert_eq!(first.id, second.id);
        assert_eq!(first.action_type, SmartActionType::PosterFix);
        assert_eq!(
            first.evidence[0].source,
            SmartEvidenceSource::PosterDetection
        );
        assert_eq!(first.recommendation.confidence, SmartConfidence::High);
    }

    #[test]
    fn summary_counts_auto_and_confirm_actions() {
        let now = Utc::now();
        let poster = smart_action_from_dashboard(
            &dashboard_action("dashboard_todo.noposter", "posters", "high"),
            now,
        );
        let dedup = smart_action_from_dashboard(
            &dashboard_action("dedup.auto_groups", "dedup", "high"),
            now,
        );
        let scan =
            smart_action_from_dashboard(&dashboard_action("scan.library", "scan", "low"), now);
        let summary = summarize_actions(&[poster.clone(), dedup, scan.clone()]);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.auto_ready, 1);
        assert_eq!(summary.confirm_required, 1);
        assert_eq!(summary.critical, 1);
        assert!(
            !smart_action_is_batch_auto_ready(&poster),
            "medium-risk auto actions stay single-action only"
        );
        assert!(smart_action_is_batch_auto_ready(&scan));
    }

    #[test]
    fn dashboard_mapping_maps_phase1_sources() {
        let now = Utc::now();
        let cases = [
            (
                dashboard_action("dashboard_todo.noposter", "posters", "high"),
                SmartActionType::PosterFix,
                SmartRiskLevel::Medium,
                SmartPolicyMode::Auto,
                SmartEvidenceSource::PosterDetection,
                None,
            ),
            (
                dashboard_action("dashboard_todo.no_rating", "cleanup", "medium"),
                SmartActionType::MetadataRefresh,
                SmartRiskLevel::Medium,
                SmartPolicyMode::Auto,
                SmartEvidenceSource::EmbyItem,
                None,
            ),
            (
                dashboard_action("dedup.auto_groups", "dedup", "high"),
                SmartActionType::DedupRemoveOld,
                SmartRiskLevel::Critical,
                SmartPolicyMode::Confirm,
                SmartEvidenceSource::DedupAnalysis,
                Some("clouddrive"),
            ),
            (
                dashboard_action("dedup.review_groups", "dedup", "medium"),
                SmartActionType::DedupReview,
                SmartRiskLevel::High,
                SmartPolicyMode::Confirm,
                SmartEvidenceSource::DedupAnalysis,
                Some("clouddrive"),
            ),
            (
                dashboard_action("zhuigeng.update_needed", "zhuigeng", "high"),
                SmartActionType::TransferUpdateSeries,
                SmartRiskLevel::High,
                SmartPolicyMode::Confirm,
                SmartEvidenceSource::TmdbMetadata,
                Some("clouddrive"),
            ),
            (
                dashboard_action("zhuigeng.archive_ready", "zhuigeng", "medium"),
                SmartActionType::ArchiveSeries,
                SmartRiskLevel::High,
                SmartPolicyMode::Confirm,
                SmartEvidenceSource::TmdbMetadata,
                Some("clouddrive"),
            ),
            (
                dashboard_action("task_runs", "tasks", "medium"),
                SmartActionType::TaskRetryOrDiagnose,
                SmartRiskLevel::Medium,
                SmartPolicyMode::Confirm,
                SmartEvidenceSource::TaskHistory,
                None,
            ),
        ];

        for (input, action_type, risk, policy, evidence, concurrency) in cases {
            let action = smart_action_from_dashboard(&input, now);
            assert_eq!(action.action_type, action_type, "{}", input.source);
            assert_eq!(action.risk.level, risk, "{}", input.source);
            assert_eq!(action.policy.mode, policy, "{}", input.source);
            assert_eq!(action.evidence[0].source, evidence, "{}", input.source);
            assert_eq!(
                action.plan.concurrency_key.as_deref(),
                concurrency,
                "{}",
                input.source
            );
        }
    }

    #[test]
    fn filter_smart_actions_filters_type_risk_q_and_paginates() {
        let now = Utc::now();
        let generated = GeneratedSmartActions {
            actions: vec![
                smart_action_from_dashboard(
                    &dashboard_action("dashboard_todo.noposter", "posters", "high"),
                    now,
                ),
                smart_action_from_dashboard(
                    &dashboard_action_with_title(
                        "dedup.auto_groups",
                        "dedup",
                        "high",
                        "清理重复旧版本",
                    ),
                    now,
                ),
                smart_action_from_dashboard(&dashboard_action("task_runs", "tasks", "medium"), now),
            ],
            warnings: vec![],
        };

        let response = filter_smart_actions(
            generated,
            SmartActionsQuery {
                status: Some("suggested".to_string()),
                action_type: None,
                risk: Some("critical".to_string()),
                subject_kind: None,
                lib: None,
                q: Some("重复".to_string()),
                limit: Some(1),
                offset: Some(0),
            },
        );
        assert_eq!(response.total, 1);
        assert_eq!(response.actions.len(), 1);
        assert_eq!(
            response.actions[0].action_type,
            SmartActionType::DedupRemoveOld
        );
        assert_eq!(response.summary.critical, 1);
    }

    #[test]
    fn execute_policy_allows_only_auto_actions_within_risk() {
        let now = Utc::now();
        let poster = smart_action_from_dashboard(
            &dashboard_action("dashboard_todo.noposter", "posters", "high"),
            now,
        );
        ensure_action_executable(&poster, &SmartActionExecuteRequest::default()).unwrap();

        let dedup = smart_action_from_dashboard(
            &dashboard_action("dedup.auto_groups", "dedup", "high"),
            now,
        );
        let err = ensure_action_executable(&dedup, &SmartActionExecuteRequest::default())
            .expect_err("high-risk confirm action must not auto execute");
        assert!(matches!(err, AppError::BadRequest(_)));

        let mut too_risky = poster;
        too_risky.risk.level = SmartRiskLevel::High;
        let err = ensure_action_executable(&too_risky, &SmartActionExecuteRequest::default())
            .expect_err("risk above policy max must be rejected");
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[test]
    fn default_filter_hides_dismissed_actions_but_status_filter_can_show_them() {
        let now = Utc::now();
        let mut dismissed = smart_action_from_dashboard(
            &dashboard_action("dashboard_todo.noposter", "posters", "high"),
            now,
        );
        dismissed.status = SmartActionStatus::Dismissed;
        let suggested =
            smart_action_from_dashboard(&dashboard_action("task_runs", "tasks", "medium"), now);

        let default_response = filter_smart_actions(
            GeneratedSmartActions {
                actions: vec![dismissed.clone(), suggested.clone()],
                warnings: vec![],
            },
            empty_query(),
        );
        assert_eq!(default_response.total, 1);
        assert_eq!(default_response.actions[0].id, suggested.id);

        let dismissed_response = filter_smart_actions(
            GeneratedSmartActions {
                actions: vec![dismissed, suggested],
                warnings: vec![],
            },
            SmartActionsQuery {
                status: Some("dismissed".to_string()),
                ..empty_query()
            },
        );
        assert_eq!(dismissed_response.total, 1);
        assert_eq!(
            dismissed_response.actions[0].status,
            SmartActionStatus::Dismissed
        );
    }

    #[test]
    fn persisted_standalone_action_is_restored_and_listed() {
        let now = Utc::now();
        let mut action =
            smart_action_from_dashboard(&dashboard_action("task_runs", "tasks", "medium"), now);
        action.status = SmartActionStatus::Failed;
        action.updated_at = now + chrono::TimeDelta::seconds(30);
        let restored = smart_action_from_persisted_row(persisted_row_from_action(
            &action,
            SmartActionStatus::Failed,
        ))
        .unwrap();

        assert_eq!(restored.id, action.id);
        assert_eq!(restored.status, SmartActionStatus::Failed);
        assert_eq!(restored.source, action.source);
        assert_eq!(restored.tab, action.tab);
        assert_eq!(restored.action_label, action.action_label);
        assert_eq!(restored.updated_at, action.updated_at);

        let response = filter_smart_actions(
            merge_persisted_smart_actions(
                GeneratedSmartActions {
                    actions: Vec::new(),
                    warnings: Vec::new(),
                },
                vec![restored],
            ),
            empty_query(),
        );
        assert_eq!(response.total, 1);
        assert_eq!(response.actions[0].status, SmartActionStatus::Failed);
        assert_eq!(response.summary.failed, 1);
    }

    #[test]
    fn persisted_state_overrides_live_generated_status_without_replacing_live_content() {
        let now = Utc::now();
        let mut live = smart_action_from_dashboard(
            &dashboard_action("dashboard_todo.noposter", "posters", "high"),
            now,
        );
        let mut stale = live.clone();
        stale.title = "旧快照标题".to_string();
        stale.status = SmartActionStatus::Dismissed;
        stale.updated_at = now + chrono::TimeDelta::minutes(5);

        let merged = merge_persisted_smart_actions(
            GeneratedSmartActions {
                actions: vec![live.clone()],
                warnings: Vec::new(),
            },
            vec![stale],
        );

        assert_eq!(merged.actions.len(), 1);
        assert_eq!(merged.actions[0].title, live.title);
        assert_eq!(merged.actions[0].status, SmartActionStatus::Dismissed);
        assert_eq!(
            merged.actions[0].updated_at,
            now + chrono::TimeDelta::minutes(5)
        );

        live.status = SmartActionStatus::Suggested;
        assert_ne!(merged.actions[0].status, live.status);
    }

    #[test]
    fn smart_action_task_result_marks_missing_subject_as_partial() {
        let now = Utc::now();
        let action = smart_action_from_dashboard(
            &dashboard_action("dashboard_todo.noposter", "posters", "high"),
            now,
        );
        let status = smart_action_execution_status(&action, false, false, &[]);
        assert_eq!(status, SmartActionStatus::Partial);
        let result = smart_action_task_result(&action, status, false, vec![], vec![]);
        assert_eq!(result["verification"]["status"], json!("partial"));
        assert_eq!(
            result["next_actions"][0]["action_type"],
            json!("poster_fix")
        );
        assert_eq!(result["next_actions"][0]["tab"], json!("posters"));
        assert_eq!(
            result["next_actions"][0]["subject"]["name"],
            json!("修复无海报条目")
        );

        let mut concrete_action = action.clone();
        concrete_action.subject.emby_id = Some("emby-item-1".to_string());
        let status = smart_action_execution_status(&concrete_action, false, false, &[]);
        assert_eq!(status, SmartActionStatus::Done);
        let result = smart_action_task_result(&concrete_action, status, false, vec![], vec![]);
        assert_eq!(result["verification"]["status"], json!("done"));
        assert!(result["next_actions"].as_array().unwrap().is_empty());

        let status = smart_action_execution_status(
            &concrete_action,
            false,
            false,
            &[json!({ "ok": false, "err": "poster still missing" })],
        );
        assert_eq!(status, SmartActionStatus::Partial);

        let status = smart_action_execution_status(
            &concrete_action,
            false,
            true,
            &[json!({ "ok": false, "err": "request failed" })],
        );
        assert_eq!(status, SmartActionStatus::Failed);
        let result = smart_action_task_result(
            &concrete_action,
            status,
            false,
            vec![json!({ "status": "error", "message": "请求失败" })],
            vec![json!({ "ok": false, "err": "request failed" })],
        );
        assert_eq!(result["verification"]["status"], json!("failed"));
        assert_eq!(result["steps"][0]["status"], json!("error"));
    }

    #[test]
    fn poster_signal_maps_to_object_level_action() {
        let now = Utc::now();
        let item = PosterSignalItem {
            id: "emby-series-1".to_string(),
            emby_name: "莫离".to_string(),
            name: "莫离".to_string(),
            lib: "电视剧".to_string(),
            item_type: "Series".to_string(),
            path: Some("/strm/电视剧/莫离/S01E01.strm".to_string()),
            folder: "莫离 (2026) tmdbid-12345".to_string(),
            folder_clean: "莫离".to_string(),
            tmdb: String::new(),
            declared_tmdb: Some("12345".to_string()),
            has_poster: false,
            score: 70,
            reasons: vec!["folder 声明 tmdbid-12345 但 Emby 未绑定 Tmdb".to_string()],
            signals: vec![posters::PosterSignal {
                kind: "declared_tmdb_unbound",
                severity: "warn",
                message: "folder 声明 tmdbid-12345 但 ProviderIds.Tmdb 为空".to_string(),
            }],
        };
        let action = smart_action_from_poster_signal(&item, now);
        assert_eq!(action.action_type, SmartActionType::PosterFix);
        assert_eq!(action.subject.kind, SmartSubjectKind::Series);
        assert_eq!(action.subject.emby_id.as_deref(), Some("emby-series-1"));
        assert_eq!(action.subject.tmdb.as_deref(), Some("12345"));
        assert_eq!(action.subject.lib.as_deref(), Some("电视剧"));
        assert_eq!(
            action.evidence[0].source,
            SmartEvidenceSource::PosterDetection
        );
        assert!(
            action
                .plan
                .steps
                .iter()
                .any(|step| step.key == "execute_low_risk"
                    && step.params["function"] == json!("fix_poster_one"))
        );
        assert_eq!(smart_action_item_type(&action), "Series");
    }

    fn zhuigeng_row(lane: ZhuigengWorkbenchLane) -> ZhuigengWorkbenchRow {
        let ended = matches!(lane, ZhuigengWorkbenchLane::ArchiveReady);
        let behind = if ended { 0 } else { 2 };
        ZhuigengWorkbenchRow {
            item: ZhuigengItem {
                lib: "追更剧".to_string(),
                name: "大唐迷雾".to_string(),
                id: Some("series-25".to_string()),
                folder: "大唐迷雾 (2026)".to_string(),
                tmdb: "98765".to_string(),
                tmdb_status: if ended { "Ended" } else { "Returning Series" }.to_string(),
                state: if ended { "ended" } else { "continuing" }.to_string(),
                continuing: !ended,
                ended,
                local_count: 25,
                local_latest: Some("2026-06-30".to_string()),
                local_latest_episode: Some("S01E25".to_string()),
                last_episode_to_air: Some(zhuigeng::TmdbEpisodeSummary {
                    season_number: Some(1),
                    episode_number: Some(if ended { 25 } else { 27 }),
                    air_date: Some("2026-06-30".to_string()),
                    name: Some("终章".to_string()),
                }),
                next_episode_to_air: None,
                behind,
                behind_hint: Some(if ended {
                    "已完结且本地齐全".to_string()
                } else {
                    "本地到 E25，TMDb 到 E27".to_string()
                }),
                resource_hint: (!ended).then(|| "S01E26-S01E27".to_string()),
                err: None,
            },
            priority: if ended { 640 } else { 720 },
            action: if ended {
                "一键归档到完结库".to_string()
            } else {
                "找资源并一条龙更新".to_string()
            },
            resource_query: (!ended).then(|| "大唐迷雾 S01E26-S01E27".to_string()),
            archiveable: ended,
            updateable: !ended,
            blockers: Vec::new(),
            lane,
        }
    }

    #[test]
    fn zhuigeng_row_maps_to_transfer_update_series_action() {
        let now = Utc::now();
        let row = zhuigeng_row(ZhuigengWorkbenchLane::UpdateNeeded);
        let action = smart_action_from_zhuigeng_row(&row, now);
        assert_eq!(action.action_type, SmartActionType::TransferUpdateSeries);
        assert_eq!(action.subject.kind, SmartSubjectKind::Series);
        assert_eq!(action.subject.name, "大唐迷雾");
        assert_eq!(action.subject.tmdb.as_deref(), Some("98765"));
        assert_eq!(action.subject.emby_id.as_deref(), Some("series-25"));
        assert_eq!(action.plan.concurrency_key.as_deref(), Some("clouddrive"));
        assert!(
            action
                .plan
                .steps
                .iter()
                .any(|step| step.key == "resource_plan")
        );
        assert!(
            action
                .evidence
                .iter()
                .any(|evidence| evidence.source == SmartEvidenceSource::TmdbMetadata)
        );
    }

    #[test]
    fn transfer_update_verify_result_includes_domain_summary() {
        let action = smart_action_from_zhuigeng_row(
            &zhuigeng_row(ZhuigengWorkbenchLane::UpdateNeeded),
            Utc::now(),
        );
        let result = smart_action_verification_result(&action, SmartActionStatus::Partial, false);
        let summary = result["check_summaries"][0]["summary"].as_str().unwrap();
        assert!(summary.contains("落后 2 集"));
        assert!(summary.contains("S01E25"));
        assert!(
            result["check_summaries"][0]["warning"]
                .as_str()
                .unwrap()
                .contains("新集 STRM")
        );
    }

    #[test]
    fn zhuigeng_archive_ready_maps_to_archive_action() {
        let now = Utc::now();
        let row = zhuigeng_row(ZhuigengWorkbenchLane::ArchiveReady);
        let action = smart_action_from_zhuigeng_row(&row, now);
        assert_eq!(action.action_type, SmartActionType::ArchiveSeries);
        assert_eq!(action.risk.level, SmartRiskLevel::High);
        assert_eq!(action.risk.requires_confirm_text.as_deref(), Some("归档"));
        assert_eq!(action.plan.concurrency_key.as_deref(), Some("clouddrive"));
        assert!(
            action
                .plan
                .steps
                .iter()
                .any(|step| step.key == "zhuigeng_archive_execute"
                    && step.params["requires_policy_param"] == json!("archive_to_lib"))
        );
    }

    #[test]
    fn archive_verify_result_includes_domain_summary() {
        let action = smart_action_from_zhuigeng_row(
            &zhuigeng_row(ZhuigengWorkbenchLane::ArchiveReady),
            Utc::now(),
        );
        let result = smart_action_verification_result(&action, SmartActionStatus::Partial, false);
        let summary = result["check_summaries"][0]["summary"].as_str().unwrap();
        assert!(summary.contains("归档候选"));
        assert!(summary.contains("ended"));
        assert!(
            result["check_summaries"][0]["next_check"]
                .as_str()
                .unwrap()
                .contains("目标库")
        );
    }

    fn dedup_group() -> DedupGroup {
        DedupGroup {
            tmdb: "12345".to_string(),
            keep: DedupRow {
                lib: "电视剧".to_string(),
                folder: "莫离 (2026) 2160p".to_string(),
                score: 98,
                n: 12,
                item_id: Some("keep-series".to_string()),
            },
            remove: vec![DedupRow {
                lib: "电视剧".to_string(),
                folder: "莫离 (2026) 1080p".to_string(),
                score: 42,
                n: 12,
                item_id: Some("remove-series".to_string()),
            }],
        }
    }

    #[test]
    fn dedup_group_maps_to_critical_confirm_delete_action() {
        let now = Utc::now();
        let group = dedup_group();
        let action = smart_action_from_dedup_group(&group, now);
        assert_eq!(action.action_type, SmartActionType::DedupRemoveOld);
        assert_eq!(action.risk.level, SmartRiskLevel::Critical);
        assert_eq!(action.risk.requires_confirm_text.as_deref(), Some("删除"));
        assert_eq!(action.policy.mode, SmartPolicyMode::Confirm);
        assert_eq!(action.plan.concurrency_key.as_deref(), Some("clouddrive"));
        let step = action_step(&action, "dedup_execute_batch").unwrap();
        assert_eq!(
            step.params["endpoint"],
            json!("/api/v2/dedup/execute-batch")
        );
        assert_eq!(step.params["request"]["groups"][0]["tmdb"], json!("12345"));
        assert_eq!(
            step.params["request"]["groups"][0]["remove"][0]["folder"],
            json!("莫离 (2026) 1080p")
        );

        ensure_action_executable(
            &action,
            &SmartActionExecuteRequest {
                confirm_text: Some("删除".to_string()),
                ..SmartActionExecuteRequest::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn dedup_verify_result_includes_remove_count_summary() {
        let action = smart_action_from_dedup_group(&dedup_group(), Utc::now());
        let result = smart_action_verification_result(&action, SmartActionStatus::Partial, false);
        let summary = result["check_summaries"][0]["summary"].as_str().unwrap();
        assert!(summary.contains("TMDb 12345"));
        assert!(summary.contains("待删除候选 1 个"));
        assert!(
            smart_action_verify_warnings_with_evidence(
                &action,
                SmartActionStatus::Partial,
                &SmartActionVerifyEvidence::default(),
            )
            .iter()
            .any(|warning| warning.contains("undo/audit"))
        );
    }

    #[test]
    fn dedup_review_group_never_builds_delete_pipeline() {
        let now = Utc::now();
        let group = DedupReviewGroup {
            tmdb: "67890".to_string(),
            why: "候选项得分接近，需要人工判断".to_string(),
            rows: vec![
                DedupRow {
                    lib: "电视剧".to_string(),
                    folder: "莫离 A".to_string(),
                    score: 80,
                    n: 12,
                    item_id: Some("a".to_string()),
                },
                DedupRow {
                    lib: "电视剧".to_string(),
                    folder: "莫离 B".to_string(),
                    score: 79,
                    n: 12,
                    item_id: Some("b".to_string()),
                },
            ],
        };
        let action = smart_action_from_dedup_review_group(&group, now);
        assert_eq!(action.action_type, SmartActionType::DedupReview);
        assert_eq!(action.risk.level, SmartRiskLevel::High);
        assert!(
            !action
                .plan
                .steps
                .iter()
                .any(|step| step.key == "dedup_execute_batch")
        );
        let err = ensure_action_executable(
            &action,
            &SmartActionExecuteRequest {
                confirm_text: Some("确认".to_string()),
                payload: Some(json!({ "groups": [] })),
                ..SmartActionExecuteRequest::default()
            },
        )
        .expect_err("review-only actions must not execute destructive pipelines");
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[test]
    fn confirm_execution_validates_required_payloads() {
        let now = Utc::now();

        let update =
            smart_action_from_zhuigeng_row(&zhuigeng_row(ZhuigengWorkbenchLane::UpdateNeeded), now);
        let err = ensure_action_executable(
            &update,
            &SmartActionExecuteRequest {
                confirm_text: Some("执行".to_string()),
                ..SmartActionExecuteRequest::default()
            },
        )
        .expect_err("update needs a resource candidate");
        assert!(matches!(err, AppError::BadRequest(_)));
        ensure_action_executable(
            &update,
            &SmartActionExecuteRequest {
                confirm_text: Some("执行".to_string()),
                payload: Some(json!({
                    "candidate": {
                        "name": "大唐迷雾 S01E26-E27",
                        "link": "https://115cdn.com/s/example",
                        "link_type": "share",
                        "share": "https://115cdn.com/s/example",
                        "rc": "abcd"
                    },
                    "target": { "lib": "追更剧" }
                })),
                ..SmartActionExecuteRequest::default()
            },
        )
        .unwrap();
        let err = ensure_action_executable(
            &update,
            &SmartActionExecuteRequest {
                confirm_text: Some("执行".to_string()),
                payload: Some(json!({
                    "candidate": {
                        "name": "大唐迷雾 S01E26-E27",
                        "link": "https://115cdn.com/s/example",
                        "link_type": "share",
                        "share": "https://115cdn.com/s/example",
                        "rc": "abcd"
                    }
                })),
                ..SmartActionExecuteRequest::default()
            },
        )
        .expect_err("smart zhuigeng update must confirm target lib or cid explicitly");
        assert!(matches!(err, AppError::BadRequest(_)));

        let add_new = smart_actions_from_catalog_candidates(
            &[catalog_item(
                "莫离 S01E01-E40 2160p",
                "best",
                230,
                true,
                false,
            )],
            Some(&catalog_context()),
            now,
            1,
        )
        .remove(0);
        let err = ensure_action_executable(
            &add_new,
            &SmartActionExecuteRequest {
                confirm_text: Some("执行".to_string()),
                payload: Some(json!({ "request": { "package_ack": "整包" } })),
                ..SmartActionExecuteRequest::default()
            },
        )
        .expect_err("add-new transfer needs a target lib or cid");
        assert!(matches!(err, AppError::BadRequest(_)));
        let err = ensure_action_executable(
            &add_new,
            &SmartActionExecuteRequest {
                confirm_text: Some("执行".to_string()),
                payload: Some(json!({ "request": { "target": { "lib": "电视剧" } } })),
                ..SmartActionExecuteRequest::default()
            },
        )
        .expect_err("package transfers need explicit package acknowledgement");
        assert!(matches!(err, AppError::BadRequest(_)));
        ensure_action_executable(
            &add_new,
            &SmartActionExecuteRequest {
                confirm_text: Some("执行".to_string()),
                payload: Some(json!({
                    "request": {
                        "target": { "lib": "电视剧" },
                        "package_ack": "整包"
                    }
                })),
                ..SmartActionExecuteRequest::default()
            },
        )
        .unwrap();

        let archive =
            smart_action_from_zhuigeng_row(&zhuigeng_row(ZhuigengWorkbenchLane::ArchiveReady), now);
        let err = ensure_action_executable(
            &archive,
            &SmartActionExecuteRequest {
                confirm_text: Some("执行".to_string()),
                payload: Some(json!({ "to_lib": "电视剧" })),
                ..SmartActionExecuteRequest::default()
            },
        )
        .expect_err("archive has a distinct confirmation word");
        assert!(matches!(err, AppError::BadRequest(_)));
        ensure_action_executable(
            &archive,
            &SmartActionExecuteRequest {
                confirm_text: Some("归档".to_string()),
                payload: Some(json!({ "to_lib": "电视剧" })),
                ..SmartActionExecuteRequest::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn high_risk_policy_cannot_be_forced_into_auto_execution() {
        let now = Utc::now();
        let mut action = smart_action_from_dedup_group(&dedup_group(), now);
        action.policy.mode = SmartPolicyMode::Auto;
        action.policy.max_risk = SmartRiskLevel::Critical;
        let err = ensure_action_executable(
            &action,
            &SmartActionExecuteRequest {
                confirm_text: Some("删除".to_string()),
                ..SmartActionExecuteRequest::default()
            },
        )
        .expect_err("destructive action should stay confirm-only even if policy is loosened");
        assert!(matches!(err, AppError::Conflict(_)));
    }

    fn task_run(status: &str, result: Option<Value>, error: Option<&str>) -> TaskRun {
        let now = Utc::now();
        TaskRun {
            id: Uuid::new_v4(),
            kind: "wizard_add_new".to_string(),
            label: "一条龙转存: 莫离".to_string(),
            source: "api".to_string(),
            params: json!({
                "lib": "电视剧",
                "folder": "莫离 (2026)",
                "tmdb": "12345",
                "item_id": "emby-1",
            }),
            status: status.to_string(),
            progress: 3,
            total: 5,
            status_text: "海报修复失败".to_string(),
            result,
            error: error.map(ToString::to_string),
            cancel_requested: false,
            queued_at: now,
            started_at: Some(now),
            ended_at: Some(now),
            updated_at: now,
        }
    }

    #[test]
    fn verify_failed_task_evidence_marks_transfer_add_new_failed() {
        let action = smart_actions_from_catalog_candidates(
            &[catalog_item(
                "莫离 S01E01-E40 2160p",
                "best",
                230,
                true,
                false,
            )],
            Some(&catalog_context()),
            Utc::now(),
            1,
        )
        .remove(0);
        let mut task = VerifyTaskRunEvidence::from(task_run(
            "error",
            Some(json!({ "ok": false, "failed": 1 })),
            Some("115 receive failed"),
        ));
        task.kind = "catalog_transfer_execute".to_string();
        task.label = "115 转存: 莫离 S01E01-E40".to_string();
        task.source = "smart_actions".to_string();
        task.params = json!({
            "action_id": action.id,
            "action_type": action.action_type.as_str(),
        });
        let evidence = SmartActionVerifyEvidence {
            persisted_source: Some("catalog.candidate".to_string()),
            persisted_tab: Some("catalog".to_string()),
            persisted_action_label: Some("确认转存".to_string()),
            task_runs: vec![task],
            ..SmartActionVerifyEvidence::default()
        };

        let status = smart_action_status_from_verify_evidence(&action, true, &evidence);
        assert_eq!(status, SmartActionStatus::Failed);
        let result =
            smart_action_verification_result_with_evidence(&action, status, false, &evidence);
        assert_eq!(result["status"], json!("failed"));
        assert_eq!(
            result["check_summaries"][0]["action"]["source"],
            json!("catalog.candidate")
        );
        assert_eq!(
            result["check_summaries"][0]["action"]["tab"],
            json!("catalog")
        );
        assert_eq!(
            result["check_summaries"][0]["action"]["action_label"],
            json!("确认转存")
        );
        assert!(
            result["check_summaries"][0]["summary"]
                .as_str()
                .unwrap()
                .contains("115 receive failed")
        );
        assert_eq!(
            result["check_summaries"][0]["evidence"]["task_runs"][0]["status"],
            json!("error")
        );
        assert!(
            smart_action_verify_warnings_with_evidence(&action, status, &evidence)
                .iter()
                .any(|warning| warning.contains("最近执行任务失败"))
        );
    }

    #[test]
    fn verify_running_task_evidence_marks_transfer_update_verifying() {
        let action = smart_action_from_zhuigeng_row(
            &zhuigeng_row(ZhuigengWorkbenchLane::UpdateNeeded),
            Utc::now(),
        );
        let mut task = VerifyTaskRunEvidence::from(task_run("running", None, None));
        task.kind = "zhuigeng_update".to_string();
        task.label = "追更一条龙更新: 大唐迷雾".to_string();
        task.source = "zhuigeng".to_string();
        task.progress = 1;
        task.total = 4;
        task.status_text = "等待 115 转存".to_string();
        task.params = json!({
            "action_id": action.id,
            "action_type": action.action_type.as_str(),
        });
        let evidence = SmartActionVerifyEvidence {
            task_runs: vec![task],
            ..SmartActionVerifyEvidence::default()
        };

        let status = smart_action_status_from_verify_evidence(&action, true, &evidence);
        assert_eq!(status, SmartActionStatus::Verifying);
        let result =
            smart_action_verification_result_with_evidence(&action, status, false, &evidence);
        assert_eq!(result["status"], json!("verifying"));
        assert_eq!(result["check_summaries"][0]["status"], json!("verifying"));
        assert!(
            result["check_summaries"][0]["summary"]
                .as_str()
                .unwrap()
                .contains("进度 1/4")
        );
        assert!(
            smart_action_verify_warnings_with_evidence(&action, status, &evidence)
                .iter()
                .any(|warning| warning.contains("仍在 running"))
        );
    }

    fn catalog_context() -> CatalogLibraryContextResponse {
        CatalogLibraryContextResponse {
            ok: true,
            query: "莫离".to_string(),
            total_matches: 1,
            truncated: false,
            summary: crate::catalog::CatalogLibraryContextSummary {
                matched: true,
                duplicate: false,
                duplicate_groups: 0,
                libraries: vec!["电视剧".to_string()],
                tmdb_ids: vec!["12345".to_string()],
                years: vec![2026],
                episode_ranges: vec!["S01E01-E08".to_string()],
                missing_ranges: vec!["S01E09-E40".to_string()],
                max_episode: 8,
                total_episodes: 8,
                note: "本地到 E08，缺 S01E09-E40".to_string(),
            },
            items: vec![crate::catalog::CatalogLibraryContextItem {
                id: Some("emby-series-1".to_string()),
                name: "莫离".to_string(),
                item_type: "Series".to_string(),
                library: Some("电视剧".to_string()),
                folder: Some("莫离 (2026)".to_string()),
                path: Some("/strm/电视剧/莫离".to_string()),
                year: Some(2026),
                tmdb: Some("12345".to_string()),
                has_primary_image: false,
                duplicate: false,
                episode_count: 8,
                episode_ranges: vec!["S01E01-E08".to_string()],
                missing_ranges: vec!["S01E09-E40".to_string()],
                max_episode: 8,
                error: None,
            }],
            warnings: Vec::new(),
        }
    }

    fn catalog_item(
        name: &str,
        level: &str,
        score: i32,
        covers_missing: bool,
        already_have: bool,
    ) -> CatalogItem {
        CatalogItem {
            name: name.to_string(),
            sheet: "tg-resource".to_string(),
            link: "https://115cdn.com/s/example?password=abcd".to_string(),
            is_pkg: true,
            link_type: "share115".to_string(),
            transfer: true,
            share: Some("https://115cdn.com/s/example".to_string()),
            rc: Some("abcd".to_string()),
            recommendation: Some(CatalogResourceRecommendation {
                score,
                level: level.to_string(),
                action: if level == "skip" {
                    "可能已存在"
                } else {
                    "推荐转存"
                }
                .to_string(),
                reasons: vec!["115 可直接转存".to_string()],
                episode_ranges: vec!["S01E01-E40".to_string()],
                covers_missing,
                duplicate_risk: false,
                already_have,
            }),
        }
    }

    #[test]
    fn catalog_candidate_maps_to_transfer_add_new_action() {
        let now = Utc::now();
        let context = catalog_context();
        let actions = smart_actions_from_catalog_candidates(
            &[catalog_item(
                "莫离 S01E01-E40 2160p",
                "best",
                230,
                true,
                false,
            )],
            Some(&context),
            now,
            4,
        );

        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        assert_eq!(action.action_type, SmartActionType::TransferAddNew);
        assert_eq!(action.risk.level, SmartRiskLevel::High);
        assert_eq!(action.risk.requires_confirm_text.as_deref(), Some("执行"));
        assert_eq!(action.policy.mode, SmartPolicyMode::Confirm);
        assert_eq!(action.subject.kind, SmartSubjectKind::Series);
        assert_eq!(action.subject.name, "莫离");
        assert_eq!(action.subject.tmdb.as_deref(), Some("12345"));
        assert_eq!(action.subject.emby_id.as_deref(), Some("emby-series-1"));
        assert_eq!(action.plan.concurrency_key.as_deref(), Some("clouddrive"));
        assert!(
            action
                .evidence
                .iter()
                .any(|evidence| evidence.source == SmartEvidenceSource::CatalogCandidate)
        );
        assert!(
            action
                .evidence
                .iter()
                .any(|evidence| evidence.source == SmartEvidenceSource::EmbyEpisodes)
        );
        assert!(
            action
                .plan
                .steps
                .iter()
                .any(|step| step.key == "catalog_transfer_execute"
                    && step.params["requires_target"] == json!(true))
        );
        let err = ensure_action_executable(
            action,
            &SmartActionExecuteRequest {
                confirm_text: Some("执行".to_string()),
                ..SmartActionExecuteRequest::default()
            },
        )
        .expect_err(
            "catalog candidate execution needs explicit target and package acknowledgement",
        );
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn transfer_add_new_verify_result_includes_candidate_and_gap_summary() {
        let actions = smart_actions_from_catalog_candidates(
            &[catalog_item(
                "莫离 S01E01-E40 2160p",
                "best",
                230,
                true,
                false,
            )],
            Some(&catalog_context()),
            Utc::now(),
            4,
        );
        let result =
            smart_action_verification_result(&actions[0], SmartActionStatus::Partial, false);
        let summary = result["check_summaries"][0]["summary"].as_str().unwrap();
        assert!(summary.contains("S01E01-E40"));
        assert!(summary.contains("S01E09-E40"));
        assert!(
            result["check_summaries"][0]["warning"]
                .as_str()
                .unwrap()
                .contains("115 转存")
        );
        let task_result = smart_action_task_result(
            &actions[0],
            SmartActionStatus::Partial,
            false,
            Vec::new(),
            Vec::new(),
        );
        let next_actions = task_result["next_actions"].as_array().unwrap();
        assert!(next_actions.iter().any(|action| {
            action["action_type"] == json!("library_scan")
                && action["label"] == json!("刷新媒体库")
                && action["tab"] == json!("scan")
                && action["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("Emby"))
                && action["subject"]["name"] == json!("莫离")
        }));
        assert!(next_actions.iter().any(|action| {
            action["action_type"] == json!("poster_fix")
                && action["tab"] == json!("posters")
                && action["subject"]["name"] == json!("莫离")
        }));
        assert!(next_actions.iter().any(|action| {
            action["action_type"] == json!("dedup_review")
                && action["tab"] == json!("dedup")
                && action["subject"]["name"] == json!("莫离")
        }));
    }

    #[test]
    fn catalog_candidate_skips_existing_or_non_transferable_items() {
        let now = Utc::now();
        let context = catalog_context();
        let mut unsupported = catalog_item("莫离 百度云", "warn", 80, false, false);
        unsupported.transfer = false;
        unsupported.link_type = "baidu".to_string();
        let actions = smart_actions_from_catalog_candidates(
            &[
                catalog_item("莫离 S01E05 2160p", "skip", 20, false, true),
                unsupported,
            ],
            Some(&context),
            now,
            4,
        );

        assert!(
            actions.is_empty(),
            "already-have and non-transferable resources must not become transfer actions"
        );
    }

    #[test]
    fn task_run_maps_to_diagnostic_action() {
        let task = task_run("error", None, Some("TMDb API key missing"));
        let action = smart_action_from_task_run(&task, Utc::now());
        assert_eq!(action.action_type, SmartActionType::TaskRetryOrDiagnose);
        assert_eq!(action.subject.kind, SmartSubjectKind::Task);
        assert_eq!(action.subject.lib.as_deref(), Some("电视剧"));
        assert_eq!(action.subject.folder.as_deref(), Some("莫离 (2026)"));
        assert_eq!(action.subject.tmdb.as_deref(), Some("12345"));
        assert_eq!(action.subject.emby_id.as_deref(), Some("emby-1"));
        assert_eq!(action.policy.mode, SmartPolicyMode::Confirm);
        assert!(
            action
                .recommendation
                .reasons
                .iter()
                .any(|reason| reason.contains("TMDb API key missing"))
        );
        assert!(
            action
                .plan
                .steps
                .iter()
                .any(|step| step.key == "open_task_center")
        );
        assert!(
            action
                .plan
                .steps
                .iter()
                .any(|step| step.key.starts_with("open_repair_")
                    && step.params["tab"] == json!("settings")
                    && step.params["action_type"] == json!("config_tmdb"))
        );
    }

    #[test]
    fn task_verify_result_includes_error_and_progress_summary() {
        let task = task_run("error", None, Some("TMDb API key missing"));
        let action = smart_action_from_task_run(&task, Utc::now());
        let result = smart_action_verification_result(&action, SmartActionStatus::Partial, false);
        let summary = result["check_summaries"][0]["summary"].as_str().unwrap();
        assert!(summary.contains("wizard_add_new"));
        assert!(summary.contains("3/5"));
        assert!(summary.contains("TMDb API key missing"));
        assert!(
            smart_action_verify_warnings_with_evidence(
                &action,
                SmartActionStatus::Partial,
                &SmartActionVerifyEvidence::default(),
            )
            .iter()
            .any(|warning| warning.contains("TMDb API key missing"))
        );
    }

    #[test]
    fn task_partial_result_is_diagnostic_signal() {
        let task = task_run(
            "done",
            Some(json!({
                "ok": true,
                "check": { "stage_error_count": 1 },
                "poster_auto_fix": { "error_count": 1 }
            })),
            None,
        );
        assert!(task_result_has_partial_signal(task.result.as_ref()));
        let action = smart_action_from_task_run(&task, Utc::now());
        assert!(action.title.contains("半成功"));
        assert!(
            action
                .recommendation
                .reasons
                .iter()
                .any(|reason| reason.contains("半成功"))
        );

        assert!(task_result_has_partial_signal(Some(&json!({
            "ok": true,
            "check": {
                "status": "suspicious",
                "suspicious_count": 1,
                "suspicious": [{ "stage": "dedup", "message": "旧目录仍在" }]
            }
        }))));
        assert!(task_result_has_partial_signal(Some(&json!({
            "ok": true,
            "dedup": { "review_count": 1 }
        }))));
        assert!(task_result_has_partial_signal(Some(&json!({
            "total": 3,
            "ok_count": 2
        }))));
        assert!(task_result_has_partial_signal(Some(&json!({
            "items": [{ "ok": true }, { "ok": false, "error": "failed" }]
        }))));
    }

    #[test]
    fn task_diagnosis_extracts_stage_specific_repair_actions() {
        let task = task_run(
            "done",
            Some(json!({
                "ok": true,
                "scan": { "ok": false, "error": "Emby refresh timeout" },
                "poster_auto_fix": { "ok": true, "error_count": 1 },
                "dedup": { "ok": true, "review_count": 2 },
                "check": {
                    "stage_error_count": 1,
                    "errors": [
                        { "stage": "scan", "message": "Emby refresh timeout" }
                    ],
                    "suspicious": [
                        { "stage": "poster", "message": "缺少 Primary poster" },
                        { "stage": "dedup", "message": "同剧旧目录仍需复核" }
                    ]
                }
            })),
            None,
        );
        let suggestions = task_repair_suggestions(&task);
        assert!(suggestions.iter().any(|item| {
            item.action_type == "library_scan" && item.tab == "scan" && item.reason.contains("Emby")
        }));
        assert!(suggestions.iter().any(|item| {
            item.action_type == "poster_fix"
                && item.tab == "posters"
                && item.reason.contains("海报")
        }));
        assert!(suggestions.iter().any(|item| {
            item.action_type == "dedup_review"
                && item.tab == "dedup"
                && item.reason.contains("旧目录")
        }));

        let action = smart_action_from_task_run(&task, Utc::now());
        assert_eq!(action.recommendation.primary_action, "刷新媒体库");
        assert!(
            action
                .recommendation
                .alternatives
                .iter()
                .any(|item| item.action == "修复海报/元数据")
        );
        assert!(action.plan.steps.iter().any(
            |step| step.key.starts_with("open_repair_") && step.params["tab"] == json!("dedup")
        ));
    }

    #[test]
    fn task_diagnosis_reads_nested_target_context() {
        let mut task = task_run(
            "done",
            Some(json!({
                "ok": true,
                "check": {
                    "stage_error_count": 1,
                    "errors": [{ "stage": "scan", "message": "scan failed" }]
                }
            })),
            None,
        );
        task.params = json!({
            "target": { "lib": "追更剧", "cid": "12345" },
            "folder": "大唐迷雾 (2026)",
            "tmdb": "98765"
        });
        let action = smart_action_from_task_run(&task, Utc::now());
        assert_eq!(action.subject.lib.as_deref(), Some("追更剧"));
        assert!(
            action
                .plan
                .steps
                .iter()
                .any(|step| step.key.starts_with("open_repair_")
                    && step.params["subject"]["lib"] == json!("追更剧"))
        );
    }

    #[test]
    fn inspect_subject_matching_uses_object_identifiers() {
        let action = smart_action_from_dedup_group(&dedup_group(), Utc::now());
        assert!(smart_action_matches_subject(
            &action,
            &SmartSubject {
                kind: SmartSubjectKind::Series,
                name: String::new(),
                year: None,
                tmdb: Some("12345".to_string()),
                emby_id: None,
                lib: None,
                folder: None,
                strm_path: None,
                cd_path: None,
            }
        ));
        assert!(smart_action_matches_subject(
            &action,
            &SmartSubject {
                kind: SmartSubjectKind::Unknown,
                name: "莫离 (2026) 2160p".to_string(),
                year: None,
                tmdb: None,
                emby_id: None,
                lib: None,
                folder: None,
                strm_path: None,
                cd_path: None,
            }
        ));
    }

    #[test]
    fn next_action_request_builds_persistable_low_risk_action() {
        let req = SmartActionFromNextActionRequest {
            next_action: SmartNextAction {
                action_type: "poster_fix".to_string(),
                label: "修复海报".to_string(),
                tab: "posters".to_string(),
                reason: "Primary 图片仍缺失".to_string(),
                subject: Some(SmartNextActionSubject {
                    name: Some("莫离".to_string()),
                    lib: Some("电视剧".to_string()),
                    tmdb: Some("12345".to_string()),
                    emby_id: Some("emby-1".to_string()),
                    folder: Some("/strm/电视剧/莫离".to_string()),
                }),
            },
            source_action_id: Some(Uuid::new_v4()),
            task_id: Some(Uuid::new_v4()),
            persist: Some(true),
        };

        let (action, warnings) = smart_action_from_next_action_request(&req, Utc::now()).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(action.action_type, SmartActionType::PosterFix);
        assert_eq!(action.subject.name, "莫离");
        assert_eq!(action.subject.emby_id.as_deref(), Some("emby-1"));
        assert_eq!(action.source, "task_next_actions");
        assert!(
            action
                .plan
                .steps
                .iter()
                .any(|step| step.key == "execute_low_risk")
        );
    }

    #[test]
    fn next_action_request_downgrades_unknown_actions_to_manual_diagnosis() {
        let req = SmartActionFromNextActionRequest {
            next_action: SmartNextAction {
                action_type: "config_tmdb".to_string(),
                label: "配置 TMDb Key".to_string(),
                tab: "settings".to_string(),
                reason: "TMDb key 缺失".to_string(),
                subject: None,
            },
            source_action_id: None,
            task_id: None,
            persist: None,
        };

        let (action, warnings) = smart_action_from_next_action_request(&req, Utc::now()).unwrap();
        assert_eq!(action.action_type, SmartActionType::TaskRetryOrDiagnose);
        assert_eq!(action.subject.name, "配置 TMDb Key");
        assert_eq!(action.tab, "settings");
        assert!(
            action
                .plan
                .steps
                .iter()
                .any(|step| step.key == "manual_followup")
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn policy_records_parse_validate_and_override_defaults() {
        let now = Utc::now();
        let poster = default_policy_record("poster_fix", now).unwrap();
        assert!(poster.enabled);
        assert_eq!(poster.mode, SmartPolicyMode::Auto);
        assert_eq!(poster.max_risk, SmartRiskLevel::Medium);

        let dedup = default_policy_record("dedup_remove_old", now).unwrap();
        assert_eq!(dedup.mode, SmartPolicyMode::Confirm);
        assert_eq!(dedup.max_risk, SmartRiskLevel::Critical);

        let row = (
            "poster_fix".to_string(),
            false,
            "disabled".to_string(),
            "low".to_string(),
            json!({ "max_per_round": 0 }),
            now,
        );
        let policy = policy_from_row(row).unwrap();
        assert!(!policy.enabled);
        assert_eq!(policy.mode, SmartPolicyMode::Disabled);
        assert_eq!(policy.max_risk, SmartRiskLevel::Low);
        assert_eq!(policy.params["max_per_round"], json!(0));

        assert!(normalized_policy_key(" POSTER_FIX ").is_ok());
        assert!(normalized_policy_key("unknown").is_err());
        assert!(
            policy_from_row((
                "poster_fix".to_string(),
                true,
                "surprise".to_string(),
                "medium".to_string(),
                json!({}),
                now,
            ))
            .is_err()
        );
    }
}
