use crate::{
    config_store,
    emby::EmbyClient,
    error::{AppError, AppResult},
    media_fs::safe_under,
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";

type UndoValidationResult<T> = Result<T, Box<UndoExecuteResponse>>;

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct UndoListQuery {
    pub limit: Option<i64>,
}

impl UndoListQuery {
    pub fn limit(&self) -> i64 {
        self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
    }
}

#[derive(Debug, Serialize, FromRow, utoipa::ToSchema)]
pub struct UndoEntry {
    pub id: Uuid,
    pub legacy_id: Option<String>,
    pub op: String,
    pub payload: Value,
    pub undone: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UndoListResponse {
    pub items: Vec<UndoEntry>,
    pub total: usize,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UndoExecuteRequest {
    pub id: Uuid,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UndoExecuteAction {
    ManualRestore,
    Executed,
    PendingPort,
    AlreadyUndone,
    Unsupported,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UndoExecuteResponse {
    pub ok: bool,
    pub id: Uuid,
    pub op: String,
    pub action: UndoExecuteAction,
    pub msg: String,
    pub lib: Option<String>,
    pub folder: Option<String>,
    pub hint: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/manage/undo", get(list_undo))
        .route("/api/v2/manage/undo/execute", post(exec_undo))
}

#[utoipa::path(get, path = "/api/v2/manage/undo", tag = "undo", params(UndoListQuery), responses((status = 200, body = UndoListResponse)))]
pub async fn list_undo(
    State(state): State<AppState>,
    Query(query): Query<UndoListQuery>,
) -> AppResult<Json<UndoListResponse>> {
    let limit = query.limit();
    let items = sqlx::query_as::<_, UndoEntry>(
        "SELECT id, legacy_id, op, payload, undone, created_at
         FROM undo_entries
         ORDER BY created_at DESC
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    let total = items.len();
    Ok(Json(UndoListResponse { items, total }))
}

#[utoipa::path(post, path = "/api/v2/manage/undo/execute", tag = "undo", request_body = UndoExecuteRequest, responses((status = 200, body = UndoExecuteResponse)))]
pub async fn exec_undo(
    State(state): State<AppState>,
    Json(req): Json<UndoExecuteRequest>,
) -> AppResult<Json<UndoExecuteResponse>> {
    let entry = sqlx::query_as::<_, UndoEntry>(
        "SELECT id, legacy_id, op, payload, undone, created_at
         FROM undo_entries
         WHERE id = $1",
    )
    .bind(req.id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("未知 undo id".to_string()))?;
    Ok(Json(execute_entry(&state, &entry).await?))
}

pub fn build_execute_response(entry: &UndoEntry) -> UndoExecuteResponse {
    if entry.undone {
        return already_undone_response(entry);
    }

    match entry.op.as_str() {
        "delete" | "smart_archive" | "replace" => manual_restore_response(entry),
        "move" => move_requires_execution_response(entry),
        "rebind" => rebind_response(entry),
        _ => UndoExecuteResponse {
            ok: false,
            id: entry.id,
            op: entry.op.clone(),
            action: UndoExecuteAction::Unsupported,
            msg: format!("不支持撤销此操作: {}", entry.op),
            lib: payload_string(&entry.payload, &["lib", "from", "to"]),
            folder: payload_string(&entry.payload, &["folder"]),
            hint: None,
        },
    }
}

async fn execute_entry(state: &AppState, entry: &UndoEntry) -> AppResult<UndoExecuteResponse> {
    if entry.undone {
        return Ok(already_undone_response(entry));
    }

    match entry.op.as_str() {
        "move" => execute_move_undo(state, entry).await,
        "delete" | "smart_archive" | "replace" => Ok(manual_restore_response(entry)),
        "rebind" => execute_rebind_undo(state, entry).await,
        _ => Ok(UndoExecuteResponse {
            ok: false,
            id: entry.id,
            op: entry.op.clone(),
            action: UndoExecuteAction::Unsupported,
            msg: format!("不支持撤销此操作: {}", entry.op),
            lib: payload_string(&entry.payload, &["lib", "from", "to"]),
            folder: payload_string(&entry.payload, &["folder"]),
            hint: None,
        }),
    }
}

fn already_undone_response(entry: &UndoEntry) -> UndoExecuteResponse {
    UndoExecuteResponse {
        ok: false,
        id: entry.id,
        op: entry.op.clone(),
        action: UndoExecuteAction::AlreadyUndone,
        msg: "这条操作已经撤销过了,不会重复执行".to_string(),
        lib: payload_string(&entry.payload, &["lib", "from", "to"]),
        folder: payload_string(&entry.payload, &["folder", "lose_was", "lose_folder"]),
        hint: None,
    }
}

fn manual_restore_response(entry: &UndoEntry) -> UndoExecuteResponse {
    let folder = payload_string(&entry.payload, &["folder", "lose_was", "lose_folder"]);
    let lib = payload_string(&entry.payload, &["lib", "from", "to"]);
    let label = match entry.op.as_str() {
        "delete" => "删除",
        "smart_archive" => "智能归档删源",
        "replace" => "全替换删旧版",
        _ => entry.op.as_str(),
    };
    let folder_text = folder.as_deref().unwrap_or("");
    let hint = format!("115 web -> 回收站 -> 找「{folder_text}」-> 还原 -> 回到工具扫描对应库");
    UndoExecuteResponse {
        ok: false,
        id: entry.id,
        op: entry.op.clone(),
        action: UndoExecuteAction::ManualRestore,
        msg: format!("「{label}」已把 115 文件夹送入回收站,请先去 115 web 还原它,再用扫描补 strm"),
        lib,
        folder,
        hint: Some(hint),
    }
}

fn move_requires_execution_response(entry: &UndoEntry) -> UndoExecuteResponse {
    let payload = match parse_move_payload(entry) {
        Ok(payload) => payload,
        Err(response) => return *response,
    };
    UndoExecuteResponse {
        ok: false,
        id: entry.id,
        op: entry.op.clone(),
        action: UndoExecuteAction::ManualRestore,
        msg: "move undo 需要在执行接口中检查当前 STRM/CD 路径后才能反向移动".to_string(),
        lib: Some(format!("{} -> {}", payload.to, payload.from)),
        folder: Some(payload.folder),
        hint: Some("调用 /api/v2/manage/undo/execute 会尝试安全反向移动; 若路径已变化,请手动恢复后扫描原库".to_string()),
    }
}

fn rebind_response(entry: &UndoEntry) -> UndoExecuteResponse {
    let payload = match parse_rebind_payload(entry) {
        Ok(payload) => payload,
        Err(response) => return *response,
    };
    UndoExecuteResponse {
        ok: false,
        id: entry.id,
        op: entry.op.clone(),
        action: UndoExecuteAction::ManualRestore,
        msg: format!(
            "rebind undo 可恢复旧 TMDb {}; 调用执行接口会写回 Emby 远端搜索绑定",
            payload.old_tmdb
        ),
        lib: None,
        folder: payload.name,
        hint: Some(format!(
            "调用 /api/v2/manage/undo/execute 会对 Emby item {} 应用旧 TMDb {}",
            payload.item_id, payload.old_tmdb
        )),
    }
}

async fn execute_rebind_undo(
    state: &AppState,
    entry: &UndoEntry,
) -> AppResult<UndoExecuteResponse> {
    let payload = match parse_rebind_payload(entry) {
        Ok(payload) => payload,
        Err(response) => return Ok(*response),
    };
    let client = rebind_emby_client(state).await?;
    let status = match client
        .apply_remote_search(&payload.item_id, &payload.old_tmdb)
        .await
    {
        Ok(status) => status,
        Err(err) => {
            return Ok(unsupported_response(
                entry,
                &format!("rebind undo 恢复旧 TMDb 失败,未标记 undone: {err}"),
                Some("请确认 Emby 可访问、api_key 有效,再重试或手动恢复 TMDb 绑定".to_string()),
            ));
        }
    };

    let updated =
        sqlx::query("UPDATE undo_entries SET undone = TRUE WHERE id = $1 AND undone = FALSE")
            .bind(entry.id)
            .execute(&state.pool)
            .await?;
    if updated.rows_affected() == 0 {
        return Ok(already_undone_response(entry));
    }

    Ok(UndoExecuteResponse {
        ok: true,
        id: entry.id,
        op: entry.op.clone(),
        action: UndoExecuteAction::Executed,
        msg: format!(
            "已恢复 Emby item {} 的旧 TMDb {} (apply status {})",
            payload.item_id, payload.old_tmdb, status
        ),
        lib: None,
        folder: payload.name,
        hint: Some("如海报未立即刷新,请在 Emby 中刷新该条目的元数据/图片".to_string()),
    })
}

#[derive(Debug, Clone)]
struct RebindUndoPayload {
    item_id: String,
    old_tmdb: String,
    name: Option<String>,
}

fn parse_rebind_payload(entry: &UndoEntry) -> UndoValidationResult<RebindUndoPayload> {
    let item_id = payload_string(&entry.payload, &["item_id", "id"]);
    let old_tmdb = payload_string(&entry.payload, &["old_tmdb"]);
    let missing: Vec<&str> = [
        ("item_id/id", item_id.as_ref()),
        ("old_tmdb", old_tmdb.as_ref()),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.is_none().then_some(key))
    .collect();
    if !missing.is_empty() {
        return Err(Box::new(unsupported_response(
            entry,
            &format!(
                "rebind undo payload 缺少 {},无法安全恢复海报绑定",
                missing.join("/")
            ),
            Some("请手动检查当前海报/TMDB 绑定状态".to_string()),
        )));
    }
    Ok(RebindUndoPayload {
        item_id: item_id.expect("validated item_id"),
        old_tmdb: old_tmdb.expect("validated old_tmdb"),
        name: payload_string(&entry.payload, &["name", "folder"]),
    })
}

async fn rebind_emby_client(state: &AppState) -> AppResult<EmbyClient> {
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    ensure_rebind_api_key_configured(&api_key)?;
    Ok(EmbyClient::new(emby_url, api_key, state.http.clone()))
}

fn ensure_rebind_api_key_configured(api_key: &str) -> AppResult<()> {
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; set it via /api/v2/config before undoing poster rebind"
                .to_string(),
        ));
    }
    Ok(())
}

async fn execute_move_undo(state: &AppState, entry: &UndoEntry) -> AppResult<UndoExecuteResponse> {
    let (payload, plan) = match build_move_undo_plan(state, entry) {
        Ok(plan) => plan,
        Err(response) => return Ok(*response),
    };
    let preflight = match preflight_move_undo(entry, &payload, &plan) {
        Ok(preflight) => preflight,
        Err(response) => return Ok(*response),
    };

    create_parent(&plan.restore_cd).await?;
    if preflight.move_strm {
        create_parent(&plan.restore_strm).await?;
    }

    tokio::fs::rename(&plan.current_cd, &plan.restore_cd)
        .await
        .map_err(anyhow::Error::from)?;
    if preflight.move_strm
        && let Err(err) = tokio::fs::rename(&plan.current_strm, &plan.restore_strm).await
    {
        let _ = tokio::fs::rename(&plan.restore_cd, &plan.current_cd).await;
        return Err(AppError::Anyhow(err.into()));
    }

    let updated =
        sqlx::query("UPDATE undo_entries SET undone = TRUE WHERE id = $1 AND undone = FALSE")
            .bind(entry.id)
            .execute(&state.pool)
            .await?;
    if updated.rows_affected() == 0 {
        return Ok(already_undone_response(entry));
    }

    let strm_msg = if preflight.move_strm {
        "STRM 已一起移回"
    } else {
        "STRM 源目录不存在,请按需扫描原库补齐 STRM"
    };
    Ok(UndoExecuteResponse {
        ok: true,
        id: entry.id,
        op: entry.op.clone(),
        action: UndoExecuteAction::Executed,
        msg: format!(
            "已反向移动 115 文件夹: {}/{} -> {}/{}; {strm_msg}",
            payload.to, payload.to_folder, payload.from, payload.folder
        ),
        lib: Some(format!("{} -> {}", payload.to, payload.from)),
        folder: Some(payload.folder),
        hint: Some("未调用 Emby 刷新; 如媒体库仍显示旧路径,请扫描/刷新对应库".to_string()),
    })
}

#[derive(Debug, Clone)]
struct MoveUndoPayload {
    from: String,
    to: String,
    folder: String,
    to_folder: String,
}

#[derive(Debug)]
struct MoveUndoPlan {
    current_cd: PathBuf,
    restore_cd: PathBuf,
    current_strm: PathBuf,
    restore_strm: PathBuf,
}

#[derive(Debug)]
struct MoveUndoPreflight {
    move_strm: bool,
}

fn build_move_undo_plan(
    state: &AppState,
    entry: &UndoEntry,
) -> UndoValidationResult<(MoveUndoPayload, MoveUndoPlan)> {
    let payload = parse_move_payload(entry)?;
    let plan = MoveUndoPlan {
        current_cd: guarded_media_path(
            entry,
            &state.settings.cd_root,
            &payload.to,
            &payload.to_folder,
        )?,
        restore_cd: guarded_media_path(
            entry,
            &state.settings.cd_root,
            &payload.from,
            &payload.folder,
        )?,
        current_strm: guarded_media_path(
            entry,
            &state.settings.strm_root,
            &payload.to,
            &payload.to_folder,
        )?,
        restore_strm: guarded_media_path(
            entry,
            &state.settings.strm_root,
            &payload.from,
            &payload.folder,
        )?,
    };
    Ok((payload, plan))
}

fn parse_move_payload(entry: &UndoEntry) -> UndoValidationResult<MoveUndoPayload> {
    let missing: Vec<&str> = ["from", "to", "folder", "to_folder"]
        .into_iter()
        .filter(|key| required_payload_string(&entry.payload, key).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(Box::new(unsupported_response(
            entry,
            &format!(
                "move undo payload 缺少 {},无法安全判断反向移动路径",
                missing.join("/")
            ),
            Some("请手动确认 115 与 STRM 当前路径后恢复,再扫描对应库".to_string()),
        )));
    }
    Ok(MoveUndoPayload {
        from: required_payload_string(&entry.payload, "from")
            .unwrap()
            .to_string(),
        to: required_payload_string(&entry.payload, "to")
            .unwrap()
            .to_string(),
        folder: required_payload_string(&entry.payload, "folder")
            .unwrap()
            .to_string(),
        to_folder: required_payload_string(&entry.payload, "to_folder")
            .unwrap()
            .to_string(),
    })
}

fn guarded_path(
    entry: &UndoEntry,
    base: impl AsRef<Path>,
    segment: &str,
) -> UndoValidationResult<PathBuf> {
    safe_under(base, segment).map_err(|err| {
        Box::new(unsupported_response(
            entry,
            &format!("move undo payload 含非法路径段,已拒绝执行: {err}"),
            Some("请手动确认路径没有绝对路径、.. 或符号链接越界".to_string()),
        ))
    })
}

fn guarded_media_path(
    entry: &UndoEntry,
    root: &Path,
    lib: &str,
    folder: &str,
) -> UndoValidationResult<PathBuf> {
    guarded_path(entry, root, lib)?;
    guarded_path(entry, root, folder)?;
    guarded_path(entry, root, &format!("{lib}/{folder}"))
}

fn preflight_move_undo(
    entry: &UndoEntry,
    payload: &MoveUndoPayload,
    plan: &MoveUndoPlan,
) -> UndoValidationResult<MoveUndoPreflight> {
    if !plan.current_cd.is_dir() {
        return Err(Box::new(move_manual_restore_response(
            entry,
            payload,
            &format!(
                "当前 115 源目录不存在,未执行: {}",
                plan.current_cd.display()
            ),
            "请确认文件夹是否已被移动/删除,必要时从 115 web 手动恢复后扫描原库",
        )));
    }
    if plan.restore_cd.exists() {
        return Err(Box::new(move_manual_restore_response(
            entry,
            payload,
            &format!(
                "115 还原目标已存在,为避免覆盖未执行: {}",
                plan.restore_cd.display()
            ),
            "请手动合并或移开目标目录后再重试",
        )));
    }
    if plan.restore_strm.exists() {
        return Err(Box::new(move_manual_restore_response(
            entry,
            payload,
            &format!(
                "STRM 还原目标已存在,为避免覆盖未执行: {}",
                plan.restore_strm.display()
            ),
            "请手动确认 STRM 目录后再重试,或移开冲突目录并重新执行",
        )));
    }
    if plan.current_strm.exists() && !plan.current_strm.is_dir() {
        return Err(Box::new(move_manual_restore_response(
            entry,
            payload,
            &format!(
                "当前 STRM 源路径不是目录,未执行: {}",
                plan.current_strm.display()
            ),
            "请手动检查 STRM 文件/目录状态后扫描对应库",
        )));
    }
    Ok(MoveUndoPreflight {
        move_strm: plan.current_strm.is_dir(),
    })
}

async fn create_parent(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(anyhow::Error::from)?;
    }
    Ok(())
}

fn move_manual_restore_response(
    entry: &UndoEntry,
    payload: &MoveUndoPayload,
    msg: &str,
    hint: &str,
) -> UndoExecuteResponse {
    UndoExecuteResponse {
        ok: false,
        id: entry.id,
        op: entry.op.clone(),
        action: UndoExecuteAction::ManualRestore,
        msg: msg.to_string(),
        lib: Some(format!("{} -> {}", payload.to, payload.from)),
        folder: Some(payload.folder.clone()),
        hint: Some(hint.to_string()),
    }
}

fn unsupported_response(entry: &UndoEntry, msg: &str, hint: Option<String>) -> UndoExecuteResponse {
    UndoExecuteResponse {
        ok: false,
        id: entry.id,
        op: entry.op.clone(),
        action: UndoExecuteAction::Unsupported,
        msg: msg.to_string(),
        lib: payload_string(&entry.payload, &["lib", "from", "to"]),
        folder: payload_string(&entry.payload, &["folder"]),
        hint,
    }
}

fn required_payload_string<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn payload_string(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        payload
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}
