use crate::{
    config_store,
    emby::{EmbyClient, EmbyLibrary},
    error::{AppError, AppResult},
    media_fs,
    state::AppState,
    tasks,
};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const DEFAULT_CD2_PREFIX: &str = "/CloudNAS/CloudDrive";
const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "ts", "m2ts", "avi", "iso", "mov", "flv", "wmv", "rmvb",
];

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct AutostrmWebhookQuery {
    pub key: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AutostrmWebhookResponse {
    pub ok: bool,
    pub queued: usize,
    pub ignored: usize,
    pub unmapped: usize,
    pub tid: Option<Uuid>,
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AutostrmWebhookEvent {
    pub lib: String,
    pub top: String,
    pub source_file: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AutostrmTaskResult {
    pub ok: bool,
    pub processed: usize,
    pub skipped_seen: usize,
    pub new_strm: usize,
    pub refreshed: usize,
    pub unmatched: usize,
    pub errors: Vec<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v2/autostrm/webhook", post(webhook))
}

#[utoipa::path(
    post,
    path = "/api/v2/autostrm/webhook",
    tag = "autostrm",
    params(AutostrmWebhookQuery),
    request_body = Value,
    responses((status = 200, body = AutostrmWebhookResponse))
)]
pub async fn webhook(
    State(state): State<AppState>,
    Query(query): Query<AutostrmWebhookQuery>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Json<AutostrmWebhookResponse>> {
    let secret = config_store::get_string_or(&state.pool, "cd2_webhook_secret", "").await?;
    let sent = headers
        .get("x-webhook-secret")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or(query.key)
        .unwrap_or_default();
    if secret.trim().is_empty() || !constant_time_eq(secret.trim().as_bytes(), sent.as_bytes()) {
        return Err(AppError::Unauthorized("forbidden".to_string()));
    }

    if !config_bool(&state, "auto_strm_enabled", false).await? {
        return Ok(Json(AutostrmWebhookResponse {
            ok: true,
            queued: 0,
            ignored: 0,
            unmapped: 0,
            tid: None,
            disabled: true,
        }));
    }

    let prefix =
        config_store::get_string_or(&state.pool, "cd2_mount_prefix", DEFAULT_CD2_PREFIX).await?;
    let folder_map = folder_to_lib_map(&state).await;
    let parsed = parse_webhook_events(&payload, &prefix, &state, &folder_map);
    let queued = parsed.events.len();
    if queued == 0 {
        return Ok(Json(AutostrmWebhookResponse {
            ok: true,
            queued,
            ignored: parsed.ignored,
            unmapped: parsed.unmapped,
            tid: None,
            disabled: false,
        }));
    }

    let params = serde_json::json!({
        "events": parsed.events,
        "ignored": parsed.ignored,
        "unmapped": parsed.unmapped,
    });
    let task = tasks::insert_task_with_meta(
        &state.pool,
        "autostrm",
        &format!("autostrm webhook: {} top", queued),
        queued as i64,
        "webhook",
        params,
    )
    .await?;
    spawn_autostrm_task(state, task.id, parsed.events);
    Ok(Json(AutostrmWebhookResponse {
        ok: true,
        queued,
        ignored: parsed.ignored,
        unmapped: parsed.unmapped,
        tid: Some(task.id),
        disabled: false,
    }))
}

fn spawn_autostrm_task(state: AppState, task_id: Uuid, events: Vec<AutostrmWebhookEvent>) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, task_id, "任务并发槽不可用", None).await;
            return;
        };
        let _ = tasks::mark_running(&state.pool, task_id, "autostrm 生成 STRM").await;
        match run_autostrm_task(&state, task_id, events).await {
            Ok(result) => {
                let _ = tasks::finish_done_with_message(
                    &state.pool,
                    task_id,
                    &format!("autostrm 完成: 新增 {} 个 STRM", result.new_strm),
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

async fn run_autostrm_task(
    state: &AppState,
    task_id: Uuid,
    events: Vec<AutostrmWebhookEvent>,
) -> AppResult<AutostrmTaskResult> {
    let mut processed = 0usize;
    let mut skipped_seen = 0usize;
    let mut new_strm = 0usize;
    let mut refreshed_libs = BTreeSet::new();
    let mut unmatched = 0usize;
    let mut errors = Vec::new();

    for (index, event) in events.iter().enumerate() {
        if tasks::cancel_requested(&state.pool, task_id).await {
            tasks::finish_cancelled(&state.pool, task_id).await?;
            return Ok(AutostrmTaskResult {
                ok: true,
                processed,
                skipped_seen,
                new_strm,
                refreshed: refreshed_libs.len(),
                unmatched,
                errors,
            });
        }
        tasks::set_progress(
            &state.pool,
            task_id,
            index as i64,
            &format!("处理 {}/{}", event.lib, event.top),
        )
        .await?;

        let mtime = top_mtime(state, &event.lib, &event.top);
        let seen: Option<i64> =
            sqlx::query_scalar("SELECT mtime FROM autostrm_seen WHERE lib = $1 AND top = $2")
                .bind(&event.lib)
                .bind(&event.top)
                .fetch_optional(&state.pool)
                .await?;
        if seen.is_some_and(|stored| stored >= mtime) {
            skipped_seen += 1;
            continue;
        }

        match media_fs::generate_missing_strm_for_library(state, &event.lib, Some(&event.top), true)
        {
            Ok(result) => {
                processed += 1;
                new_strm += result.new_count;
                record_seen(state, &event.lib, &event.top, mtime).await?;
                if result.new_count > 0 {
                    refreshed_libs.insert(event.lib.clone());
                    if !has_declared_tmdb(&event.top) {
                        record_unmatched(state, &event.lib, &event.top, None, None).await?;
                        unmatched += 1;
                    }
                }
            }
            Err(err) => {
                errors.push(format!("{}/{}: {}", event.lib, event.top, err));
            }
        }
        tasks::set_progress(
            &state.pool,
            task_id,
            (index + 1) as i64,
            &format!("autostrm {}/{}", index + 1, events.len()),
        )
        .await?;
    }

    let refresh_codes = refresh_libraries(state, &refreshed_libs).await;
    Ok(AutostrmTaskResult {
        ok: errors.is_empty(),
        processed,
        skipped_seen,
        new_strm,
        refreshed: refresh_codes.len(),
        unmatched,
        errors,
    })
}

async fn refresh_libraries(state: &AppState, libs: &BTreeSet<String>) -> BTreeMap<String, u16> {
    let mut refreshed = BTreeMap::new();
    if libs.is_empty() {
        return refreshed;
    }
    let Ok(emby_url) = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await
    else {
        return refreshed;
    };
    let Ok(api_key) = config_store::get_string_or(&state.pool, "api_key", "").await else {
        return refreshed;
    };
    if api_key.trim().is_empty() {
        return refreshed;
    }
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let Ok(libraries) = client.libraries().await else {
        return refreshed;
    };
    for lib in libs {
        let Some(id) = libraries
            .iter()
            .find(|candidate| candidate.name == *lib || candidate.name.eq_ignore_ascii_case(lib))
            .and_then(|library| library.id.as_deref())
            .filter(|id| !id.trim().is_empty())
        else {
            continue;
        };
        if let Ok(code) = client.refresh_item(id, true, false).await {
            refreshed.insert(lib.clone(), code);
        }
    }
    refreshed
}

async fn record_seen(state: &AppState, lib: &str, top: &str, mtime: i64) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO autostrm_seen(lib, top, mtime, updated_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT(lib, top) DO UPDATE
         SET mtime = EXCLUDED.mtime, updated_at = now()",
    )
    .bind(lib)
    .bind(top)
    .bind(mtime)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn record_unmatched(
    state: &AppState,
    lib: &str,
    top: &str,
    emby_id: Option<&str>,
    name: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO autostrm_unmatched(lib, top, emby_id, name, updated_at)
         VALUES ($1, $2, $3, $4, now())
         ON CONFLICT(lib, top) DO UPDATE
         SET emby_id = EXCLUDED.emby_id, name = EXCLUDED.name, updated_at = now()",
    )
    .bind(lib)
    .bind(top)
    .bind(emby_id)
    .bind(name)
    .execute(&state.pool)
    .await?;
    Ok(())
}

#[derive(Debug)]
struct ParsedWebhookEvents {
    events: Vec<AutostrmWebhookEvent>,
    ignored: usize,
    unmapped: usize,
}

fn parse_webhook_events(
    payload: &Value,
    prefix: &str,
    state: &AppState,
    folder_map: &BTreeMap<String, String>,
) -> ParsedWebhookEvents {
    let mut events = Vec::new();
    let mut ignored = 0usize;
    let mut unmapped = 0usize;
    let mut seen = BTreeSet::new();
    let Some(data) = payload.get("data").and_then(Value::as_array) else {
        return ParsedWebhookEvents {
            events,
            ignored: 1,
            unmapped,
        };
    };
    for event in data {
        if is_dir_event(event) || is_delete_event(event) {
            ignored += 1;
            continue;
        }
        let Some(source_file) = event_path(event) else {
            ignored += 1;
            continue;
        };
        if !is_video_path(source_file) {
            ignored += 1;
            continue;
        }
        let Some((lib, top)) = reverse_map(source_file, prefix, state, folder_map) else {
            unmapped += 1;
            continue;
        };
        if seen.insert((lib.clone(), top.clone())) {
            events.push(AutostrmWebhookEvent {
                lib,
                top,
                source_file: source_file.to_string(),
            });
        }
    }
    ParsedWebhookEvents {
        events,
        ignored,
        unmapped,
    }
}

fn event_path(event: &Value) -> Option<&str> {
    event
        .get("destination_file")
        .and_then(Value::as_str)
        .or_else(|| event.get("source_file").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_dir_event(event: &Value) -> bool {
    match event.get("is_dir") {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes"
            )
        }
        Some(Value::Number(value)) => value.as_i64().unwrap_or_default() != 0,
        _ => false,
    }
}

fn is_delete_event(event: &Value) -> bool {
    let action = event
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        action.as_str(),
        "delete" | "remove" | "removed" | "deleted" | "unlink" | "rmdir"
    )
}

fn reverse_map(
    source_file: &str,
    prefix: &str,
    state: &AppState,
    folder_map: &BTreeMap<String, String>,
) -> Option<(String, String)> {
    let normalized = source_file.replace('\\', "/");
    let rel = strip_prefix_path(&normalized, prefix)
        .or_else(|| strip_prefix_path(&normalized, &state.settings.cd_root.display().to_string()))
        .or_else(|| strip_prefix_path(&normalized, "/media"))?;
    let mut parts = rel.split('/').filter(|part| !part.trim().is_empty());
    let folder = parts.next()?.trim();
    let top = parts.next()?.trim();
    if folder == "." || folder == ".." || top == "." || top == ".." {
        return None;
    }
    let lib = folder_map
        .get(folder)
        .cloned()
        .unwrap_or_else(|| folder.to_string());
    Some((lib, top.to_string()))
}

fn strip_prefix_path<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return None;
    }
    value
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('/'))
}

async fn folder_to_lib_map(state: &AppState) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Ok(emby_url) = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await
    else {
        return map;
    };
    let Ok(api_key) = config_store::get_string_or(&state.pool, "api_key", "").await else {
        return map;
    };
    if api_key.trim().is_empty() {
        return map;
    }
    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    if let Ok(libraries) = client.libraries().await {
        for library in libraries {
            insert_library_folder_aliases(&mut map, &library);
        }
    }
    map
}

fn insert_library_folder_aliases(map: &mut BTreeMap<String, String>, library: &EmbyLibrary) {
    map.insert(library.name.clone(), library.name.clone());
    for path in &library.paths {
        let normalized = path.replace('\\', "/");
        if let Some(name) = normalized
            .split('/')
            .filter(|part| !part.trim().is_empty())
            .next_back()
            .filter(|part| !part.is_empty())
        {
            map.insert(name.to_string(), library.name.clone());
        }
    }
}

async fn config_bool(state: &AppState, key: &str, default: bool) -> Result<bool, sqlx::Error> {
    Ok(config_store::get_raw(&state.pool, key)
        .await?
        .and_then(|value| match value {
            Value::Bool(value) => Some(value),
            Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            },
            Value::Number(value) => Some(value.as_i64().unwrap_or_default() != 0),
            _ => None,
        })
        .unwrap_or(default))
}

fn top_mtime(state: &AppState, lib: &str, top: &str) -> i64 {
    let path = state.settings.cd_root.join(lib).join(top);
    path_mtime_seconds(&path)
}

fn path_mtime_seconds(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or_else(|_| SystemTime::now())
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn is_video_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
        .is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension))
}

fn has_declared_tmdb(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower
        .match_indices("tmdbid")
        .any(|(idx, _)| lower[idx + 6..].chars().any(|ch| ch.is_ascii_digit()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    for idx in 0..left.len().max(right.len()) {
        let a = left.get(idx).copied().unwrap_or_default();
        let b = right.get(idx).copied().unwrap_or_default();
        diff |= (a ^ b) as usize;
    }
    diff == 0
}
