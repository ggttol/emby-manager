use crate::{
    config_store,
    emby::{EmbyClient, EmbyItem, EmbyLibrary, EmbyRemoteSearchCandidate},
    error::{AppError, AppResult},
    state::AppState,
    tasks::{self, TaskRun},
};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const DEFAULT_SCAN_LIMIT: usize = 30_000;
const MAX_SCAN_LIMIT: usize = 100_000;

#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct PosterDetectRequest {
    pub lib: Option<String>,
    pub limit: Option<usize>,
    pub include_missing_primary: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PosterDetectResponse {
    pub ok: bool,
    pub scanned_libraries: usize,
    pub scanned_items: usize,
    pub total: usize,
    pub missing_primary_total: usize,
    pub mismatch_total: usize,
    pub truncated: bool,
    pub warnings: Vec<String>,
    pub items: Vec<PosterSignalItem>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PosterSignalItem {
    pub id: String,
    pub emby_name: String,
    pub name: String,
    pub lib: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub path: Option<String>,
    pub folder: String,
    pub folder_clean: String,
    pub tmdb: String,
    pub declared_tmdb: Option<String>,
    pub has_poster: bool,
    pub score: u16,
    pub reasons: Vec<String>,
    pub signals: Vec<PosterSignal>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PosterSignal {
    pub kind: &'static str,
    pub severity: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct PosterSearchRequest {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PosterSearchResponse {
    pub ok: bool,
    pub candidates: Vec<PosterSearchCandidate>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PosterSearchCandidate {
    pub name: String,
    pub year: Option<i32>,
    pub tmdb: String,
    pub img: String,
    pub overview: String,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct PosterApplyRequest {
    pub id: String,
    pub tmdb: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PosterApplyResponse {
    pub ok: bool,
    pub name: String,
    pub poster: bool,
    pub tmdb: String,
    pub apply_status: u16,
    pub refresh_status: u16,
    pub image_download_status: Option<u16>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct PosterFixBatchRequest {
    pub ids: Vec<String>,
    #[serde(rename = "type")]
    pub item_type: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PosterFixBatchResult {
    pub results: Vec<PosterFixOneResult>,
    pub ok_count: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PosterFixOneResult {
    pub id: String,
    pub name: String,
    pub ok: bool,
    pub tmdb: Option<String>,
    pub err: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/posters/detect-mismatch", post(detect_mismatch))
        .route("/api/v2/posters/search", post(search))
        .route("/api/v2/posters/apply", post(apply))
        .route("/api/v2/posters/fix-batch", post(fix_batch))
}

#[utoipa::path(
    post,
    path = "/api/v2/posters/detect-mismatch",
    tag = "posters",
    request_body = PosterDetectRequest,
    responses((status = 200, body = PosterDetectResponse))
)]
pub async fn detect_mismatch(
    State(state): State<AppState>,
    body: Option<Json<PosterDetectRequest>>,
) -> AppResult<Json<PosterDetectResponse>> {
    let req = body.map(|Json(req)| req).unwrap_or_default();
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; set it via /api/v2/config before detecting posters"
                .to_string(),
        ));
    }

    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    Ok(Json(detect_mismatched_posters(&client, req).await?))
}

#[utoipa::path(
    post,
    path = "/api/v2/posters/search",
    tag = "posters",
    request_body = PosterSearchRequest,
    responses((status = 200, body = PosterSearchResponse))
)]
pub async fn search(
    State(state): State<AppState>,
    Json(req): Json<PosterSearchRequest>,
) -> AppResult<Json<PosterSearchResponse>> {
    let client = poster_client(&state).await?;
    Ok(Json(search_posters(&client, req).await?))
}

#[utoipa::path(
    post,
    path = "/api/v2/posters/apply",
    tag = "posters",
    request_body = PosterApplyRequest,
    responses((status = 200, body = PosterApplyResponse))
)]
pub async fn apply(
    State(state): State<AppState>,
    Json(req): Json<PosterApplyRequest>,
) -> AppResult<Json<PosterApplyResponse>> {
    validate_tmdb(&req.tmdb)?;
    let client = poster_client(&state).await?;
    Ok(Json(apply_poster_match(&state.pool, &client, req).await?))
}

#[utoipa::path(
    post,
    path = "/api/v2/posters/fix-batch",
    tag = "posters",
    request_body = PosterFixBatchRequest,
    responses((status = 200, body = TaskRun))
)]
pub async fn fix_batch(
    State(state): State<AppState>,
    Json(req): Json<PosterFixBatchRequest>,
) -> AppResult<Json<TaskRun>> {
    if req.ids.is_empty() {
        return Err(AppError::BadRequest("ids must not be empty".to_string()));
    }
    validate_item_type(&req.item_type)?;
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;
    let ids: Vec<String> = req
        .ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .take(500)
        .collect();
    if ids.is_empty() {
        return Err(AppError::BadRequest("ids must not be empty".to_string()));
    }
    let item_type = req.item_type.trim().to_string();
    let params = serde_json::to_value(PosterFixBatchRequest {
        ids: ids.clone(),
        item_type: item_type.clone(),
    })
    .unwrap_or_else(|_| serde_json::json!({}));
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "poster_fix_batch",
        &format!("批量海报修复: {} x {}", item_type, ids.len()),
        ids.len() as i64,
        "api",
        params,
    )
    .await?;
    spawn_fix_batch(state, task.id, emby_url, api_key, ids, item_type);
    Ok(Json(task))
}

async fn poster_client(state: &AppState) -> AppResult<EmbyClient> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_api_key_configured(&api_key)?;
    Ok(EmbyClient::new(emby_url, api_key, state.http.clone()))
}

fn ensure_api_key_configured(api_key: &str) -> AppResult<()> {
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; set it via /api/v2/config before fixing posters"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_tmdb(tmdb: &str) -> AppResult<()> {
    let value = tmdb.trim();
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(AppError::BadRequest(format!(
            "tmdbid must contain digits only: {tmdb:?}"
        )));
    }
    Ok(())
}

fn validate_item_type(item_type: &str) -> AppResult<()> {
    match item_type.trim().to_ascii_lowercase().as_str() {
        "series" | "tvshow" | "tvshows" | "show" | "movie" | "movies" => Ok(()),
        _ => Err(AppError::BadRequest(format!(
            "unsupported poster item type: {item_type}"
        ))),
    }
}

fn validate_item_type_anyhow(item_type: &str) -> anyhow::Result<()> {
    match item_type.trim().to_ascii_lowercase().as_str() {
        "series" | "tvshow" | "tvshows" | "show" | "movie" | "movies" => Ok(()),
        _ => anyhow::bail!("unsupported poster item type: {item_type}"),
    }
}

fn validate_tmdb_anyhow(tmdb: &str) -> anyhow::Result<()> {
    let value = tmdb.trim();
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        anyhow::bail!("tmdbid must contain digits only: {tmdb:?}");
    }
    Ok(())
}

pub async fn search_posters(
    client: &EmbyClient,
    req: PosterSearchRequest,
) -> anyhow::Result<PosterSearchResponse> {
    validate_item_type_anyhow(&req.item_type)?;
    let candidates = match client
        .remote_search(&req.id, &req.name, &req.item_type, req.limit.unwrap_or(8))
        .await
    {
        Ok(candidates) => candidates.into_iter().map(candidate_from_emby).collect(),
        Err(_) => Vec::new(),
    };
    Ok(PosterSearchResponse {
        ok: true,
        candidates,
    })
}

pub async fn apply_poster_match(
    pool: &sqlx::PgPool,
    client: &EmbyClient,
    req: PosterApplyRequest,
) -> anyhow::Result<PosterApplyResponse> {
    validate_item_type_anyhow(&req.item_type)?;
    validate_tmdb_anyhow(&req.tmdb)?;
    let item_id = req.id.trim();
    let tmdb = req.tmdb.trim();
    let before = client.item(item_id, "ProviderIds,Name,ImageTags").await?;
    record_rebind_undo(pool, &req, before.as_ref(), tmdb).await?;

    let apply_status = client.apply_remote_search(item_id, tmdb).await?;
    let refresh_status = client.refresh_item(item_id, false, true).await?;
    let mut current = client.item(item_id, "ProviderIds,Name,ImageTags").await?;
    let mut image_download_status = None;

    if !current.as_ref().is_some_and(EmbyItem::has_primary_image)
        && let Some(image_url) = client
            .remote_search(
                item_id,
                req.name.as_deref().unwrap_or_default(),
                &req.item_type,
                8,
            )
            .await
            .unwrap_or_default()
            .into_iter()
            .find_map(|candidate| {
                let row = candidate_from_emby(candidate);
                (row.tmdb == tmdb && !row.img.is_empty()).then_some(row.img)
            })
    {
        image_download_status = Some(client.download_primary_image(item_id, &image_url).await?);
        current = client.item(item_id, "ProviderIds,Name,ImageTags").await?;
    }

    let name = current
        .as_ref()
        .and_then(|item| item.name.clone())
        .or_else(|| before.as_ref().and_then(|item| item.name.clone()))
        .or(req.name)
        .unwrap_or_default();
    let poster = current.as_ref().is_some_and(EmbyItem::has_primary_image);

    Ok(PosterApplyResponse {
        ok: true,
        name,
        poster,
        tmdb: tmdb.to_string(),
        apply_status,
        refresh_status,
        image_download_status,
    })
}

async fn record_rebind_undo(
    pool: &sqlx::PgPool,
    req: &PosterApplyRequest,
    before: Option<&EmbyItem>,
    new_tmdb: &str,
) -> anyhow::Result<()> {
    let old_tmdb = before
        .and_then(|item| item.provider_id("Tmdb"))
        .unwrap_or_default();
    if old_tmdb == new_tmdb {
        return Ok(());
    }
    let name = before
        .and_then(|item| item.name.clone())
        .or_else(|| req.name.clone())
        .unwrap_or_default();
    let payload = serde_json::json!({
        "id": req.id.trim(),
        "name": name,
        "old_tmdb": old_tmdb,
        "new_tmdb": new_tmdb,
        "type": req.item_type.trim(),
    });
    sqlx::query("INSERT INTO undo_entries(id, op, payload) VALUES ($1, 'rebind', $2)")
        .bind(Uuid::new_v4())
        .bind(payload)
        .execute(pool)
        .await?;
    Ok(())
}

fn candidate_from_emby(candidate: EmbyRemoteSearchCandidate) -> PosterSearchCandidate {
    PosterSearchCandidate {
        name: candidate
            .name
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string(),
        year: candidate.production_year,
        tmdb: provider_id_from_map(&candidate.provider_ids, "Tmdb").unwrap_or_default(),
        img: candidate
            .image_url
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string(),
        overview: truncate_chars(candidate.overview.as_deref().unwrap_or_default(), 160),
    }
}

fn provider_id_from_map(
    map: &std::collections::BTreeMap<String, Value>,
    key: &str,
) -> Option<String> {
    map.iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .and_then(|(_, value)| match value {
            Value::String(s) => {
                let s = s.trim();
                (!s.is_empty()).then(|| s.to_string())
            }
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn parent_folder_from_path(path: &str) -> Option<String> {
    let normalized = normalize_slashes(path);
    let mut parts = normalized.rsplit('/').filter(|part| !part.is_empty());
    let _file = parts.next()?;
    parts.next().map(trim_media_extension)
}

fn strip_bracket_suffix(value: &str) -> String {
    let mut end = value.len();
    for marker in ['(', '（', '[', '【'] {
        if let Some(index) = value.find(marker) {
            end = end.min(index);
        }
    }
    value[..end].trim().to_string()
}

fn short_id(item_id: &str) -> String {
    item_id.chars().take(8).collect()
}

fn spawn_fix_batch(
    state: AppState,
    task_id: Uuid,
    emby_url: String,
    api_key: String,
    ids: Vec<String>,
    item_type: String,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, task_id, "任务并发槽不可用", None).await;
            return;
        };
        let client = EmbyClient::new(emby_url, api_key, state.http.clone());
        let _ = tasks::mark_running(&state.pool, task_id, "批量海报修复启动").await;
        let mut results = Vec::new();

        for (index, item_id) in ids.iter().enumerate() {
            if tasks::cancel_requested(&state.pool, task_id).await {
                let _ = tasks::finish_cancelled(&state.pool, task_id).await;
                return;
            }
            let _ = tasks::set_progress(
                &state.pool,
                task_id,
                index as i64,
                &format!("修 {} {}", item_type, short_id(item_id)),
            )
            .await;
            results.push(fix_poster_one(&state.pool, &client, item_id, &item_type).await);
            let _ = tasks::set_progress(
                &state.pool,
                task_id,
                (index + 1) as i64,
                &format!("已处理 {}/{}", index + 1, ids.len()),
            )
            .await;
            sleep(Duration::from_millis(500)).await;
        }

        let ok_count = results.iter().filter(|row| row.ok).count();
        let total = results.len();
        let result = PosterFixBatchResult {
            results,
            ok_count,
            total,
        };
        let _ = tasks::finish_done_with_message(
            &state.pool,
            task_id,
            &format!("海报修复完成: {ok_count}/{total}"),
            serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
        )
        .await;
    });
}

pub async fn fix_poster_one(
    pool: &sqlx::PgPool,
    client: &EmbyClient,
    item_id: &str,
    item_type: &str,
) -> PosterFixOneResult {
    match fix_poster_one_inner(pool, client, item_id, item_type).await {
        Ok(row) => row,
        Err(err) => PosterFixOneResult {
            id: item_id.to_string(),
            name: "(?)".to_string(),
            ok: false,
            tmdb: None,
            err: err.to_string(),
        },
    }
}

async fn fix_poster_one_inner(
    pool: &sqlx::PgPool,
    client: &EmbyClient,
    item_id: &str,
    item_type: &str,
) -> anyhow::Result<PosterFixOneResult> {
    let item = client
        .item(item_id, "Name,Path,ProviderIds,ImageTags")
        .await?
        .ok_or_else(|| anyhow::anyhow!("Emby item not found"))?;
    let name = item.name.clone().unwrap_or_default();
    let folder = item
        .path
        .as_deref()
        .and_then(parent_folder_from_path)
        .unwrap_or_else(|| name.clone());
    let search_name = strip_bracket_suffix(&folder);
    let search_name = if search_name.trim().is_empty() {
        name.as_str()
    } else {
        search_name.as_str()
    };
    let picked = client
        .remote_search(item_id, search_name, item_type, 8)
        .await?
        .into_iter()
        .map(candidate_from_emby)
        .find(|candidate| !candidate.img.is_empty() && candidate.name.contains(search_name));

    let Some(candidate) = picked else {
        return Ok(PosterFixOneResult {
            id: item_id.to_string(),
            name,
            ok: false,
            tmdb: None,
            err: "无合适候选".to_string(),
        });
    };

    let applied = apply_poster_match(
        pool,
        client,
        PosterApplyRequest {
            id: item_id.to_string(),
            tmdb: candidate.tmdb.clone(),
            item_type: item_type.to_string(),
            name: Some(name.clone()),
        },
    )
    .await?;

    Ok(PosterFixOneResult {
        id: item_id.to_string(),
        name,
        ok: applied.poster,
        tmdb: Some(candidate.tmdb),
        err: if applied.poster {
            String::new()
        } else {
            "已绑定但海报未到".to_string()
        },
    })
}

pub async fn detect_mismatched_posters(
    client: &EmbyClient,
    req: PosterDetectRequest,
) -> anyhow::Result<PosterDetectResponse> {
    let requested_lib = req
        .lib
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let include_missing_primary = req.include_missing_primary.unwrap_or(true);
    let limit = req
        .limit
        .unwrap_or(DEFAULT_SCAN_LIMIT)
        .clamp(1, MAX_SCAN_LIMIT);
    let mut libraries = client.libraries().await?;

    if let Some(lib) = requested_lib.as_deref() {
        libraries.retain(|candidate| {
            candidate.name == lib || candidate.id.as_deref().is_some_and(|id| id == lib)
        });
    }

    let mut scanned_items = 0usize;
    let mut scanned_libraries = 0usize;
    let mut missing_primary_total = 0usize;
    let mut mismatch_total = 0usize;
    let mut truncated = false;
    let mut warnings = Vec::new();
    let mut items = Vec::new();

    for library in libraries {
        if scanned_items >= limit {
            truncated = true;
            break;
        }
        let Some(parent_id) = library
            .id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            warnings.push(format!("library {} has no ItemId; skipped", library.name));
            continue;
        };
        let item_types = poster_item_types(&library);
        let remaining = limit - scanned_items;
        let result = client
            .poster_items(parent_id, item_types, remaining)
            .await?;
        scanned_libraries += 1;
        if result.truncated {
            truncated = true;
            let total = result
                .total_record_count
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            warnings.push(format!(
                "library {} was truncated at {} of {} items",
                library.name,
                result.items.len(),
                total
            ));
        }
        for item in result.items {
            scanned_items += 1;
            let has_poster = item.has_primary_image();
            if !has_poster {
                missing_primary_total += 1;
            }
            if let Some(row) = analyze_item(&library, &item, include_missing_primary) {
                if row
                    .signals
                    .iter()
                    .any(|signal| signal.kind != "missing_primary")
                {
                    mismatch_total += 1;
                }
                items.push(row);
            }
        }
    }

    items.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.lib.cmp(&b.lib))
            .then_with(|| a.emby_name.cmp(&b.emby_name))
            .then_with(|| a.id.cmp(&b.id))
    });

    Ok(PosterDetectResponse {
        ok: true,
        scanned_libraries,
        scanned_items,
        total: items.len(),
        missing_primary_total,
        mismatch_total,
        truncated,
        warnings,
        items,
    })
}

fn analyze_item(
    library: &EmbyLibrary,
    item: &EmbyItem,
    include_missing_primary: bool,
) -> Option<PosterSignalItem> {
    let emby_name = item
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("(unnamed)")
        .to_string();
    let item_type = item
        .item_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Unknown")
        .to_string();
    let path = item
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let folder = path
        .as_deref()
        .and_then(|path| folder_from_library_path(path, &library.paths))
        .unwrap_or_else(|| fallback_folder(path.as_deref().unwrap_or(&emby_name)));
    let folder_clean = clean_folder_name(&folder);
    let tmdb = item.provider_id("Tmdb").unwrap_or_default();
    let declared_tmdb = declared_tmdb_id(&folder);
    let has_poster = item.has_primary_image();
    let mut score = 0u16;
    let mut reasons = Vec::new();
    let mut signals = Vec::new();

    if !has_poster && include_missing_primary {
        score = score.max(40);
        reasons.push("没有 Primary poster".to_string());
        signals.push(PosterSignal {
            kind: "missing_primary",
            severity: "warn",
            message: "条目没有 Primary poster".to_string(),
        });
    }

    if let Some(declared) = declared_tmdb.as_deref() {
        if tmdb.is_empty() {
            score = score.max(70);
            reasons.push(format!("folder 声明 tmdbid-{declared} 但 Emby 未绑定 Tmdb"));
            signals.push(PosterSignal {
                kind: "declared_tmdb_unbound",
                severity: "warn",
                message: format!("folder 声明 tmdbid-{declared} 但 ProviderIds.Tmdb 为空"),
            });
        } else if tmdb != declared {
            score = score.max(100);
            reasons.push(format!(
                "folder 声明 tmdbid-{declared} 但 Emby 绑了 {tmdb}(确定绑错)"
            ));
            signals.push(PosterSignal {
                kind: "declared_tmdb_mismatch",
                severity: "danger",
                message: format!("folder 声明 tmdbid-{declared} 与 ProviderIds.Tmdb={tmdb} 不一致"),
            });
        }
    }

    if signals.is_empty() {
        return None;
    }

    Some(PosterSignalItem {
        id: item.id.clone().unwrap_or_default(),
        emby_name: emby_name.clone(),
        name: emby_name,
        lib: library.name.clone(),
        item_type,
        path,
        folder,
        folder_clean,
        tmdb,
        declared_tmdb,
        has_poster,
        score,
        reasons,
        signals,
    })
}

fn poster_item_types(library: &EmbyLibrary) -> &'static str {
    match library.library_type.to_ascii_lowercase().as_str() {
        "movies" | "movie" => "Movie",
        "tvshows" | "series" | "shows" => "Series",
        _ => "Movie,Series",
    }
}

fn folder_from_library_path(path: &str, library_paths: &[String]) -> Option<String> {
    let path = normalize_slashes(path);
    for root in library_paths {
        let root = normalize_slashes(root);
        if root.is_empty() {
            continue;
        }
        let rest = if path == root {
            ""
        } else if let Some(rest) = path.strip_prefix(&(root.clone() + "/")) {
            rest
        } else {
            continue;
        };
        let first = rest.split('/').find(|part| !part.is_empty())?;
        return Some(trim_media_extension(first));
    }
    None
}

fn fallback_folder(path_or_name: &str) -> String {
    let value = normalize_slashes(path_or_name);
    let last = value
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(value.as_str());
    trim_media_extension(last)
}

fn normalize_slashes(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

fn trim_media_extension(value: &str) -> String {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    for extension in [
        ".strm", ".mkv", ".mp4", ".avi", ".mov", ".wmv", ".m4v", ".ts", ".iso",
    ] {
        if lower.ends_with(extension) {
            let end = trimmed.len() - extension.len();
            return trimmed[..end].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn declared_tmdb_id(folder: &str) -> Option<String> {
    let lower = folder.to_ascii_lowercase();
    for marker in ["tmdbid-", "tmdbid_"] {
        let Some(start) = lower.find(marker) else {
            continue;
        };
        let digit_start = start + marker.len();
        let digits: String = lower[digit_start..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return Some(digits);
        }
    }
    None
}

fn clean_folder_name(folder: &str) -> String {
    let mut cleaned = trim_media_extension(folder);
    if let Some(tmdb) = declared_tmdb_id(&cleaned) {
        for token in [
            format!("[tmdbid-{tmdb}]"),
            format!("[tmdbid_{tmdb}]"),
            format!("(tmdbid-{tmdb})"),
            format!("(tmdbid_{tmdb})"),
            format!("tmdbid-{tmdb}"),
            format!("tmdbid_{tmdb}"),
        ] {
            cleaned = replace_case_insensitive(&cleaned, &token, "");
        }
    }
    cleaned
        .trim_matches(|ch: char| ch.is_whitespace() || matches!(ch, '-' | '_' | '.' | '[' | ']'))
        .trim()
        .to_string()
}

fn replace_case_insensitive(source: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0usize;
    let lower_source = source.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    while let Some(pos) = lower_source[cursor..].find(&lower_needle) {
        let absolute = cursor + pos;
        out.push_str(&source[cursor..absolute]);
        out.push_str(replacement);
        cursor = absolute + needle.len();
    }
    out.push_str(&source[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::{clean_folder_name, declared_tmdb_id, folder_from_library_path};

    #[test]
    fn parses_declared_tmdb_id_and_clean_name() {
        assert_eq!(
            declared_tmdb_id("中文片 [tmdbid-12345]"),
            Some("12345".to_string())
        );
        assert_eq!(declared_tmdb_id("Show tmdbid_987"), Some("987".to_string()));
        assert_eq!(clean_folder_name("中文片 [tmdbid-12345].strm"), "中文片");
    }

    #[test]
    fn extracts_folder_under_library_root() {
        let roots = vec!["/strm/Movies".to_string()];
        assert_eq!(
            folder_from_library_path("/strm/Movies/沙丘 [tmdbid-438631]/movie.strm", &roots),
            Some("沙丘 [tmdbid-438631]".to_string())
        );
        assert_eq!(
            folder_from_library_path("/strm/Movies/沙丘.strm", &roots),
            Some("沙丘".to_string())
        );
    }
}
