use crate::{
    config_store,
    emby::EmbyClient,
    error::{AppError, AppResult},
    media_fs::safe_under,
    state::AppState,
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
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DedupExecuteResponse {
    pub ok: bool,
    pub tmdb: Option<String>,
    pub removed: Vec<DedupDeleteResult>,
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
        .route("/api/v2/dedup/duplicates", get(duplicates))
        .route("/api/v2/dedup/analyze", get(duplicates))
        .route("/api/v2/dedup/replace", post(replace_execute))
        .route("/api/v2/dedup/auto_all", post(auto_all))
        .route("/api/v2/dedup/auto-all", post(auto_all))
}

#[utoipa::path(get, path = "/api/v2/dedup/duplicates", tag = "dedup", responses((status = 200, body = DedupAnalysisResponse)))]
pub async fn duplicates(State(state): State<AppState>) -> AppResult<Json<DedupAnalysisResponse>> {
    Ok(Json(analyze_duplicate_groups(&state.settings.strm_root)?))
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
    let mut removed = Vec::with_capacity(req.remove.len());
    for item in &req.remove {
        removed.push(delete_duplicate_folder(&state, item, &emby_client).await?);
    }

    Ok(Json(DedupExecuteResponse {
        ok: true,
        tmdb: req.tmdb,
        removed,
    }))
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

#[utoipa::path(post, path = "/api/v2/dedup/auto-all", tag = "dedup", request_body = DedupAutoAllRequest, responses((status = 200, body = DedupAutoAllResponse)))]
pub async fn auto_all(
    State(state): State<AppState>,
    body: Option<Json<DedupAutoAllRequest>>,
) -> AppResult<Json<DedupAutoAllResponse>> {
    let req = body.map(|Json(req)| req).unwrap_or_default();
    let analysis = analyze_duplicate_groups(&state.settings.strm_root)?;
    let review_count = analysis.review.len();
    let limit = req
        .limit
        .unwrap_or(MAX_AUTO_ALL_GROUPS)
        .min(MAX_AUTO_ALL_GROUPS);
    let mut results = Vec::new();

    let groups = analysis.dups.into_iter().take(limit).collect::<Vec<_>>();
    let emby_client = if groups.iter().any(|group| !group.remove.is_empty()) {
        Some(dedup_emby_client(&state).await?)
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
            };
            let client = emby_client
                .as_ref()
                .expect("dedup auto-all groups with removals require an Emby client");
            match delete_duplicate_folder(&state, &item, client).await {
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
) -> AppResult<DedupDeleteResult> {
    let lib = required_value(&item.lib, "lib")?;
    let folder = required_value(&item.folder, "folder")?;
    let cd_lib = safe_under(&state.settings.cd_root, lib)?;
    let strm_lib = safe_under(&state.settings.strm_root, lib)?;
    let cd_target = safe_under(&cd_lib, folder)?;
    let strm_target = safe_under(&strm_lib, folder)?;

    let mut deleted_from = Vec::new();
    if remove_path_if_exists(&cd_target).await? {
        deleted_from.push("115".to_string());
    }
    if remove_path_if_exists(&strm_target).await? {
        deleted_from.push("strm".to_string());
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

    let emby_updates = if deleted_from.is_empty() {
        Vec::new()
    } else {
        vec![EmbyUpdate {
            path: format!("/strm/{lib}/{folder}"),
            update_type: "Deleted".to_string(),
        }]
    };
    let notified = notify_emby_updates(emby_client, &emby_updates).await?;

    Ok(DedupDeleteResult {
        lib: lib.to_string(),
        folder: folder.to_string(),
        deleted_from,
        emby_updates,
        notified,
        undo_id,
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
