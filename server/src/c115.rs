use crate::{
    config_store,
    emby::EmbyClient,
    error::{AppError, AppResult},
    state::AppState,
    tasks::{self, TaskRun},
};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use reqwest::{
    Client,
    header::{ACCEPT, COOKIE, REFERER, USER_AGENT},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use tokio::time::{Duration, sleep};
use uuid::Uuid;

pub const C115_API: &str = "https://webapi.115.com";
pub const C115_SITE: &str = "https://115.com";
pub const C115_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
const C115_COOKIE_KEY: &str = "c115_cookie";
pub const C115_CID_MAP_KEY: &str = "c115_cid_map";
const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const C115_DELETE_RETRY_COUNT: usize = 5;
const C115_DELETE_RETRY_DELAY: Duration = Duration::from_millis(1800);

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ShareUrl {
    pub url: String,
    pub pwd: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct C115ParseResponse {
    pub share: Option<String>,
    pub receive_code: Option<String>,
}

#[derive(Clone)]
pub struct C115Client {
    base_url: String,
    site_url: String,
    cookie: String,
    http: Client,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct C115TestResponse {
    pub ok: bool,
    pub uid: String,
    pub used: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct C115TestCandidateRequest {
    pub cookie: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct C115SnapRequest {
    pub url: String,
    pub pwd: Option<String>,
    pub file_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct C115SnapFile {
    pub id: Option<String>,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct C115SnapResponse {
    pub ok: bool,
    pub share: String,
    pub rc: Option<String>,
    pub share_title: Option<String>,
    pub file_size: Option<Value>,
    pub files: Vec<C115SnapFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct C115SaveRequest {
    pub url: String,
    pub pwd: Option<String>,
    pub lib: Option<String>,
    pub cid: Option<String>,
    pub label: Option<String>,
    pub file_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct C115SaveBatchRequest {
    pub items: Vec<C115SaveRequest>,
    pub lib: Option<String>,
    pub cid: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct C115SaveResponse {
    pub ok: bool,
    pub share: String,
    pub count: usize,
    pub cid: String,
    pub lib: Option<String>,
    pub title: Option<String>,
    pub msg: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct C115OfflineRequest {
    pub url: String,
    pub lib: Option<String>,
    pub cid: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct C115OfflineBatchRequest {
    pub items: Vec<C115OfflineRequest>,
    pub lib: Option<String>,
    pub cid: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct C115OfflineResponse {
    pub ok: bool,
    pub info_hash: Option<String>,
    pub cid: String,
    pub lib: Option<String>,
    pub msg: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct C115AutoCidRequest {
    pub max_depth: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct C115DirEntry {
    pub cid: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct C115CidMatch {
    pub cid: String,
    pub path: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct C115AutoCidResponse {
    pub ok: bool,
    pub matches: BTreeMap<String, Vec<C115CidMatch>>,
    pub current: BTreeMap<String, String>,
    pub scanned: usize,
}

struct C115AutoCidPlan {
    cookie: String,
    targets: BTreeMap<String, String>,
    current: BTreeMap<String, String>,
    max_depth: usize,
}

#[derive(Debug, Deserialize)]
struct C115IndexInfo {
    #[serde(default)]
    state: bool,
    data: Option<C115IndexData>,
    error: Option<String>,
    msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct C115IndexData {
    space_info: Option<C115SpaceInfo>,
}

#[derive(Debug, Deserialize)]
struct C115SpaceInfo {
    all_total: Option<C115SpaceSize>,
    all_use: Option<C115SpaceSize>,
}

#[derive(Debug, Deserialize)]
struct C115SpaceSize {
    size_format: Option<String>,
}

#[derive(Debug, Deserialize)]
struct C115SnapApiResponse {
    #[serde(default)]
    state: bool,
    data: Option<C115SnapData>,
    error: Option<String>,
    msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct C115GenericApiResponse {
    #[serde(default)]
    state: bool,
    error: Option<String>,
    msg: Option<String>,
    errno: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct C115FilesApiResponse {
    #[serde(default)]
    state: bool,
    #[serde(default)]
    data: Vec<C115FileItem>,
    error: Option<String>,
    msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct C115FileItem {
    fid: Option<Value>,
    cid: Option<Value>,
    n: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct C115OfflineSpaceResponse {
    #[serde(default)]
    state: bool,
    sign: Option<Value>,
    time: Option<Value>,
    error: Option<String>,
    #[serde(rename = "_err")]
    private_err: Option<String>,
    msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct C115OfflineAddResponse {
    #[serde(default)]
    state: bool,
    info_hash: Option<Value>,
    error_msg: Option<String>,
    error: Option<String>,
    errtype: Option<String>,
    errcode: Option<Value>,
    msg: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct C115SnapData {
    #[serde(default)]
    list: Vec<C115SnapItem>,
    shareinfo: Option<C115ShareInfo>,
    total: Option<Value>,
    count: Option<Value>,
    file_count: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct C115SnapItem {
    fid: Option<Value>,
    file_id: Option<Value>,
    cid: Option<Value>,
    n: Option<String>,
    name: Option<String>,
    s: Option<Value>,
    size: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct C115ShareInfo {
    share_title: Option<String>,
    file_name: Option<String>,
    file_size: Option<Value>,
    total_size: Option<Value>,
    file_count: Option<Value>,
    total: Option<Value>,
    count: Option<Value>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/c115/test", get(test_cookie))
        .route("/api/v2/c115/test-candidate", post(test_candidate_cookie))
        .route("/api/v2/c115/test_candidate", post(test_candidate_cookie))
        .route("/api/v2/c115/parse", post(parse_url))
        .route("/api/v2/c115/snap", post(snap))
        .route("/api/v2/c115/save", post(save))
        .route("/api/v2/c115/save/batch", post(save_batch))
        .route("/api/v2/c115/offline", post(offline))
        .route("/api/v2/c115/offline/batch", post(offline_batch))
        .route("/api/v2/c115/auto-cid", post(auto_cid))
        .route("/api/v2/c115/auto-cid/task", post(auto_cid_task))
}

impl C115Client {
    pub fn new(base_url: impl Into<String>, cookie: impl Into<String>, http: Client) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            site_url: base_url.clone(),
            base_url,
            cookie: cookie.into(),
            http,
        }
    }

    pub fn new_with_site(
        base_url: impl Into<String>,
        site_url: impl Into<String>,
        cookie: impl Into<String>,
        http: Client,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            site_url: site_url.into().trim_end_matches('/').to_string(),
            cookie: cookie.into(),
            http,
        }
    }

    pub async fn test_cookie(&self) -> AppResult<C115TestResponse> {
        let cookie = require_c115_cookie(Some(self.cookie.clone()))?;
        let url = format!("{}/files/index_info", self.base_url);
        let resp = self
            .http
            .get(url)
            .header(USER_AGENT, C115_UA)
            .header(COOKIE, cookie.as_str())
            .header(REFERER, "https://115.com/")
            .header(ACCEPT, "application/json, text/plain, */*")
            .send()
            .await
            .map_err(|e| AppError::BadRequest(format!("115 cookie 测试请求失败: {e}")))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AppError::BadRequest(format!("读取 115 响应失败: {e}")))?;
        if !status.is_success() {
            return Err(AppError::BadRequest(format!(
                "115 cookie 验证失败: HTTP {status}: {}",
                body_snippet(&body)
            )));
        }

        let info: C115IndexInfo = serde_json::from_str(&body).map_err(|e| {
            AppError::BadRequest(format!(
                "115 cookie 验证失败: 响应不是预期 JSON: {e}: {}",
                body_snippet(&body)
            ))
        })?;
        if !info.state {
            return Err(AppError::BadRequest(format!(
                "115 cookie 验证失败: {}",
                info.error
                    .or(info.msg)
                    .filter(|v| !v.trim().is_empty())
                    .unwrap_or_else(|| "115 返回 state=false".to_string())
            )));
        }

        Ok(C115TestResponse {
            ok: true,
            uid: c115_uid_from_cookie(&cookie).unwrap_or_default(),
            used: used_space(&info.data),
        })
    }

    pub async fn snap(&self, req: C115SnapRequest) -> AppResult<C115SnapResponse> {
        const LIMIT: usize = 1000;
        const MAX_PAGES: usize = 20;

        let cookie = require_c115_cookie(Some(self.cookie.clone()))?;
        let (share, rc) = parse_115_url(&req.url, req.pwd.as_deref());
        let share = share.ok_or_else(|| {
            AppError::BadRequest(
                "解析不到 share_code(贴完整 115 分享链接,或 share_code+空格+提取码)".to_string(),
            )
        })?;
        let receive_code = rc.as_deref().unwrap_or("");

        let mut items = Vec::new();
        let mut shareinfo = None;
        let mut total_hint = None;
        let mut offset = 0usize;

        for _ in 0..MAX_PAGES {
            let page = self
                .snap_page(&cookie, &share, receive_code, offset, LIMIT)
                .await?;
            let page_total = page.total_hint();
            if total_hint.is_none() {
                total_hint = page_total;
            }
            if shareinfo.is_none() {
                shareinfo = page.shareinfo.clone();
            }

            let chunk_len = page.list.len();
            items.extend(page.list);
            offset = items.len();

            if chunk_len == 0 {
                break;
            }
            if let Some(total) = page_total.or(total_hint) {
                if items.len() >= total {
                    break;
                }
            } else if chunk_len < LIMIT {
                break;
            }
        }

        let wanted = req.file_ids.as_ref().map(|ids| {
            ids.iter()
                .filter_map(|id| non_empty_trimmed(id).map(ToString::to_string))
                .collect::<HashSet<_>>()
        });
        let files = items
            .into_iter()
            .filter_map(|item| snap_file_from_item(item, wanted.as_ref()))
            .collect();

        Ok(C115SnapResponse {
            ok: true,
            share,
            rc,
            share_title: shareinfo.as_ref().and_then(C115ShareInfo::title),
            file_size: shareinfo.as_ref().and_then(C115ShareInfo::size),
            files,
        })
    }

    pub async fn save_to_cid(
        &self,
        req: C115SaveRequest,
        target_cid: String,
        target_lib: Option<String>,
    ) -> AppResult<C115SaveResponse> {
        let target_cid = validate_target_cid(&target_cid)?;
        let snap = self
            .snap(C115SnapRequest {
                url: req.url,
                pwd: req.pwd,
                file_ids: req.file_ids.clone(),
            })
            .await?;
        let file_ids = req.file_ids.unwrap_or_else(|| {
            snap.files
                .iter()
                .filter_map(|file| file.id.clone())
                .collect()
        });
        let file_ids = file_ids
            .into_iter()
            .filter_map(|id| non_empty_trimmed(&id).map(ToString::to_string))
            .collect::<Vec<_>>();
        if file_ids.is_empty() {
            return Err(AppError::BadRequest(
                "分享内无可转存文件(snap 返回里没识别出 id)".to_string(),
            ));
        }

        let receive = self
            .receive(&snap.share, snap.rc.as_deref(), &file_ids, &target_cid)
            .await?;
        let where_label = target_lib
            .as_deref()
            .or(req.label.as_deref())
            .map(|label| format!("库「{label}」"))
            .unwrap_or_else(|| format!("目录 cid={target_cid}"));
        let msg = if receive.state {
            format!("已转存 {} 项到{}", file_ids.len(), where_label)
        } else {
            receive
                .error
                .or(receive.msg)
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "115 receive 返回 state=false".to_string())
        };

        Ok(C115SaveResponse {
            ok: receive.state,
            share: snap.share,
            count: file_ids.len(),
            cid: target_cid,
            lib: target_lib,
            title: snap.share_title,
            msg,
        })
    }

    pub async fn offline_add(
        &self,
        req: C115OfflineRequest,
        target_cid: String,
        target_lib: Option<String>,
    ) -> AppResult<C115OfflineResponse> {
        let target_cid = validate_target_cid(&target_cid)?;
        let url = non_empty_trimmed(&req.url)
            .ok_or_else(|| AppError::BadRequest("空链接".to_string()))?
            .to_string();
        let space = self.offline_space().await?;
        if !space.state {
            let err = space
                .error
                .or(space.private_err)
                .or(space.msg)
                .unwrap_or_else(|| "115 离线 sign 获取失败".to_string());
            return Err(AppError::BadRequest(format!(
                "拿离线 sign 失败(cookie 失效或无离线权限): {}",
                err
            )));
        }
        let sign = space.sign.as_ref().and_then(value_to_string);
        let time = space.time.as_ref().and_then(value_to_string);
        let (Some(sign), Some(time)) = (sign, time) else {
            return Err(AppError::BadRequest(
                "离线 sign/time 缺失(115 响应结构异常,可能接口变更)".to_string(),
            ));
        };

        let add = self
            .offline_add_task(&url, &target_cid, &sign, &time)
            .await?;
        let where_label = target_lib
            .as_deref()
            .or(req.label.as_deref())
            .map(|label| format!("库「{label}」"))
            .unwrap_or_else(|| format!("目录 cid={target_cid}"));
        if add.state {
            Ok(C115OfflineResponse {
                ok: true,
                info_hash: add.info_hash.as_ref().and_then(value_to_string),
                cid: target_cid,
                lib: target_lib,
                msg: format!("已加入 115 离线下载队列(到 115 看进度)→ {where_label}"),
            })
        } else {
            let err = add
                .error_msg
                .or(add.error)
                .or(add.errtype)
                .or(add.msg)
                .or_else(|| {
                    add.errcode
                        .as_ref()
                        .map(|v| format!("离线添加失败(errcode={v})"))
                })
                .unwrap_or_else(|| "离线添加失败".to_string());
            Ok(C115OfflineResponse {
                ok: false,
                info_hash: add.info_hash.as_ref().and_then(value_to_string),
                cid: target_cid,
                lib: target_lib,
                msg: err,
            })
        }
    }

    pub async fn list_dirs(&self, cid: &str) -> AppResult<Vec<C115DirEntry>> {
        const LIMIT: usize = 1000;
        const MAX_PAGES: usize = 50;
        let cookie = require_c115_cookie(Some(self.cookie.clone()))?;
        let url = format!("{}/files", self.base_url);
        let mut offset = 0usize;
        let mut dirs = Vec::new();
        for _ in 0..MAX_PAGES {
            let params = [
                ("aid", "1".to_string()),
                ("cid", cid.to_string()),
                ("o", "user_ptime".to_string()),
                ("asc", "0".to_string()),
                ("offset", offset.to_string()),
                ("limit", LIMIT.to_string()),
                ("show_dir", "1".to_string()),
                ("format", "json".to_string()),
            ];
            let resp = self
                .http
                .get(&url)
                .query(&params)
                .header(USER_AGENT, C115_UA)
                .header(COOKIE, cookie.as_str())
                .header(REFERER, "https://115.com/")
                .header(ACCEPT, "application/json, text/plain, */*")
                .send()
                .await
                .map_err(|e| AppError::BadRequest(format!("115 列目录请求失败: {e}")))?;
            let status = resp.status();
            let body = resp
                .text()
                .await
                .map_err(|e| AppError::BadRequest(format!("读取 115 列目录响应失败: {e}")))?;
            if !status.is_success() {
                return Err(AppError::BadRequest(format!(
                    "115 列目录失败: HTTP {status}: {}",
                    body_snippet(&body)
                )));
            }
            let files: C115FilesApiResponse = serde_json::from_str(&body).map_err(|e| {
                AppError::BadRequest(format!(
                    "115 列目录失败: 响应不是预期 JSON: {e}: {}",
                    body_snippet(&body)
                ))
            })?;
            if !files.state {
                return Err(AppError::BadRequest(format!(
                    "115 列目录失败: {}",
                    files
                        .error
                        .or(files.msg)
                        .filter(|v| !v.trim().is_empty())
                        .unwrap_or_else(|| "115 返回 state=false".to_string())
                )));
            }
            let chunk_len = files.data.len();
            dirs.extend(
                files
                    .data
                    .into_iter()
                    .filter(|item| item.fid.is_none())
                    .filter_map(|item| {
                        let cid = item.cid.as_ref().and_then(value_to_string)?;
                        let name = item
                            .n
                            .or(item.name)
                            .and_then(|v| non_empty_trimmed(&v).map(ToString::to_string))?;
                        Some(C115DirEntry { cid, name })
                    }),
            );
            if chunk_len < LIMIT {
                break;
            }
            offset += chunk_len;
        }
        Ok(dirs)
    }

    pub async fn delete_child_dir(&self, parent_cid: &str, folder: &str) -> AppResult<bool> {
        let folder = non_empty_trimmed(folder)
            .ok_or_else(|| AppError::BadRequest("115 删除目录名不能为空".to_string()))?;
        let parent_cid = validate_target_cid(parent_cid)?;
        let matches = self
            .list_dirs(&parent_cid)
            .await?
            .into_iter()
            .filter(|dir| dir.name == folder)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Ok(false);
        }
        if matches.len() > 1 {
            return Err(AppError::Conflict(format!(
                "115 目录 cid={parent_cid} 下存在多个同名文件夹「{folder}」，为避免误删已中止"
            )));
        }
        self.delete_ids(&parent_cid, &[matches[0].cid.clone()])
            .await?;
        Ok(true)
    }

    pub async fn delete_ids(&self, parent_cid: &str, ids: &[String]) -> AppResult<()> {
        let cookie = require_c115_cookie(Some(self.cookie.clone()))?;
        let parent_cid = validate_target_cid(parent_cid)?;
        let ids = ids
            .iter()
            .filter_map(|id| non_empty_trimmed(id).map(ToString::to_string))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Err(AppError::BadRequest("115 删除 ids 不能为空".to_string()));
        }
        let url = format!("{}/rb/delete", self.base_url);
        let mut form = vec![
            ("pid".to_string(), parent_cid),
            ("ignore_warn".to_string(), "1".to_string()),
        ];
        for (idx, id) in ids.iter().enumerate() {
            form.push((format!("fid[{idx}]"), id.clone()));
        }
        let mut last_error = None;
        for attempt in 0..=C115_DELETE_RETRY_COUNT {
            let resp = self
                .http
                .post(&url)
                .form(&form)
                .header(USER_AGENT, C115_UA)
                .header(COOKIE, cookie.as_str())
                .header(REFERER, "https://115.com/")
                .header(ACCEPT, "application/json, text/plain, */*")
                .send()
                .await
                .map_err(|e| AppError::BadRequest(format!("115 删除请求失败: {e}")))?;
            let result: C115GenericApiResponse = self.parse_json_response(resp, "115 删除").await?;
            if result.state {
                return Ok(());
            }
            let retryable = c115_delete_is_busy(&result);
            let err = c115_generic_error(result, "115 返回 state=false");
            if retryable && attempt < C115_DELETE_RETRY_COUNT {
                last_error = Some(err);
                sleep(C115_DELETE_RETRY_DELAY).await;
                continue;
            }
            return Err(AppError::BadRequest(format!("115 删除失败: {err}")));
        }
        Err(AppError::BadRequest(format!(
            "115 删除失败: {}",
            last_error.unwrap_or_else(|| "重试耗尽".to_string())
        )))
    }

    pub async fn auto_cid(
        &self,
        targets: BTreeMap<String, String>,
        current: BTreeMap<String, String>,
        max_depth: usize,
    ) -> AppResult<C115AutoCidResponse> {
        let mut matches: BTreeMap<String, Vec<C115CidMatch>> = BTreeMap::new();
        let mut visited = HashSet::new();
        let mut scanned = 0usize;
        let max_depth = max_depth.min(5);
        self.walk_cids(
            "0".to_string(),
            String::new(),
            max_depth,
            &targets,
            &mut matches,
            &mut visited,
            &mut scanned,
        )
        .await?;
        Ok(C115AutoCidResponse {
            ok: true,
            matches,
            current,
            scanned,
        })
    }

    async fn receive(
        &self,
        share_code: &str,
        receive_code: Option<&str>,
        file_ids: &[String],
        target_cid: &str,
    ) -> AppResult<C115GenericApiResponse> {
        let cookie = require_c115_cookie(Some(self.cookie.clone()))?;
        let url = format!("{}/share/receive", self.base_url);
        let user_id = c115_uid_from_cookie(&cookie).unwrap_or_default();
        let form = [
            ("share_code", share_code.to_string()),
            ("receive_code", receive_code.unwrap_or("").to_string()),
            ("file_id", file_ids.join(",")),
            ("cid", target_cid.to_string()),
            ("user_id", user_id),
        ];
        let resp = self
            .http
            .post(url)
            .form(&form)
            .header(USER_AGENT, C115_UA)
            .header(COOKIE, cookie)
            .header(REFERER, "https://115.com/")
            .header(ACCEPT, "application/json, text/plain, */*")
            .send()
            .await
            .map_err(|e| AppError::BadRequest(format!("115 receive 请求失败: {e}")))?;
        self.parse_json_response(resp, "115 receive").await
    }

    async fn offline_space(&self) -> AppResult<C115OfflineSpaceResponse> {
        let cookie = require_c115_cookie(Some(self.cookie.clone()))?;
        let url = format!("{}/", self.site_url);
        let resp = self
            .http
            .get(url)
            .query(&[("ct", "offline"), ("ac", "space")])
            .header(USER_AGENT, C115_UA)
            .header(COOKIE, cookie)
            .header(REFERER, "https://115.com/")
            .header(ACCEPT, "application/json, text/plain, */*")
            .send()
            .await
            .map_err(|e| AppError::BadRequest(format!("115 离线 sign 请求失败: {e}")))?;
        self.parse_json_response(resp, "115 离线 sign").await
    }

    async fn offline_add_task(
        &self,
        url_value: &str,
        target_cid: &str,
        sign: &str,
        time: &str,
    ) -> AppResult<C115OfflineAddResponse> {
        let cookie = require_c115_cookie(Some(self.cookie.clone()))?;
        let url = format!("{}/web/lixian/", self.site_url);
        let params = [("ct", "lixian"), ("ac", "add_task_url")];
        let form = [
            ("url", url_value.to_string()),
            ("wp_path_id", target_cid.to_string()),
            ("sign", sign.to_string()),
            ("time", time.to_string()),
        ];
        let resp = self
            .http
            .post(url)
            .query(&params)
            .form(&form)
            .header(USER_AGENT, C115_UA)
            .header(COOKIE, cookie)
            .header(REFERER, "https://115.com/")
            .header(ACCEPT, "application/json, text/plain, */*")
            .send()
            .await
            .map_err(|e| AppError::BadRequest(format!("115 离线添加请求失败: {e}")))?;
        self.parse_json_response(resp, "115 离线添加").await
    }

    async fn parse_json_response<T: for<'de> Deserialize<'de>>(
        &self,
        resp: reqwest::Response,
        label: &str,
    ) -> AppResult<T> {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AppError::BadRequest(format!("读取 {label} 响应失败: {e}")))?;
        if !status.is_success() {
            return Err(AppError::BadRequest(format!(
                "{label} 失败: HTTP {status}: {}",
                body_snippet(&body)
            )));
        }
        serde_json::from_str(&body).map_err(|e| {
            AppError::BadRequest(format!(
                "{label} 失败: 响应不是预期 JSON: {e}: {}",
                body_snippet(&body)
            ))
        })
    }

    async fn walk_cids(
        &self,
        cid: String,
        prefix: String,
        depth: usize,
        targets: &BTreeMap<String, String>,
        matches: &mut BTreeMap<String, Vec<C115CidMatch>>,
        visited: &mut HashSet<String>,
        scanned: &mut usize,
    ) -> AppResult<()> {
        let mut stack = vec![(cid, prefix, depth)];
        while let Some((cid, prefix, depth)) = stack.pop() {
            if *scanned >= 80 || !visited.insert(cid.clone()) {
                continue;
            }
            *scanned += 1;
            let dirs = self.list_dirs(&cid).await?;
            for dir in dirs {
                let path = format!("{}/{}", prefix, dir.name);
                if let Some(lib) = targets.get(&dir.name) {
                    matches.entry(lib.clone()).or_default().push(C115CidMatch {
                        cid: dir.cid.clone(),
                        path: path.clone(),
                    });
                }
                if depth > 0 {
                    stack.push((dir.cid, path, depth - 1));
                }
            }
        }
        Ok(())
    }

    async fn snap_page(
        &self,
        cookie: &str,
        share_code: &str,
        receive_code: &str,
        offset: usize,
        limit: usize,
    ) -> AppResult<C115SnapData> {
        let url = format!("{}/share/snap", self.base_url);
        let params = [
            ("share_code", share_code.to_string()),
            ("receive_code", receive_code.to_string()),
            ("cid", "0".to_string()),
            ("offset", offset.to_string()),
            ("limit", limit.to_string()),
        ];
        let resp = self
            .http
            .get(url)
            .query(&params)
            .header(USER_AGENT, C115_UA)
            .header(COOKIE, cookie)
            .header(REFERER, "https://115.com/")
            .header(ACCEPT, "application/json, text/plain, */*")
            .send()
            .await
            .map_err(|e| AppError::BadRequest(format!("115 snap 请求失败: {e}")))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AppError::BadRequest(format!("读取 115 snap 响应失败: {e}")))?;
        if !status.is_success() {
            return Err(AppError::BadRequest(format!(
                "115 snap 失败: HTTP {status}: {}",
                body_snippet(&body)
            )));
        }

        let snap: C115SnapApiResponse = serde_json::from_str(&body).map_err(|e| {
            AppError::BadRequest(format!(
                "115 snap 失败: 响应不是预期 JSON: {e}: {}",
                body_snippet(&body)
            ))
        })?;
        if !snap.state {
            return Err(AppError::BadRequest(format!(
                "115 snap 失败: {}",
                snap.error
                    .or(snap.msg)
                    .filter(|v| !v.trim().is_empty())
                    .unwrap_or_else(|| "115 返回 state=false".to_string())
            )));
        }

        Ok(snap.data.unwrap_or_default())
    }
}

#[utoipa::path(get, path = "/api/v2/c115/test", tag = "c115", responses((status = 200, body = C115TestResponse)))]
pub async fn test_cookie(State(state): State<AppState>) -> AppResult<Json<C115TestResponse>> {
    let cookie =
        require_c115_cookie(config_store::get_string(&state.pool, C115_COOKIE_KEY).await?)?;
    let client = C115Client::new_with_site(C115_API, C115_SITE, cookie, state.http.clone());
    Ok(Json(client.test_cookie().await?))
}

#[utoipa::path(post, path = "/api/v2/c115/test-candidate", tag = "c115", request_body = C115TestCandidateRequest, responses((status = 200, body = C115TestResponse)))]
pub async fn test_candidate_cookie(
    State(state): State<AppState>,
    Json(req): Json<C115TestCandidateRequest>,
) -> AppResult<Json<C115TestResponse>> {
    let cookie = require_c115_cookie(Some(req.cookie))?;
    let client = C115Client::new_with_site(C115_API, C115_SITE, cookie, state.http.clone());
    Ok(Json(client.test_cookie().await?))
}

#[utoipa::path(post, path = "/api/v2/c115/snap", tag = "c115", request_body = C115SnapRequest, responses((status = 200, body = C115SnapResponse)))]
pub async fn snap(
    State(state): State<AppState>,
    Json(req): Json<C115SnapRequest>,
) -> AppResult<Json<C115SnapResponse>> {
    let cookie =
        require_c115_cookie(config_store::get_string(&state.pool, C115_COOKIE_KEY).await?)?;
    let client = C115Client::new_with_site(C115_API, C115_SITE, cookie, state.http.clone());
    Ok(Json(client.snap(req).await?))
}

#[utoipa::path(post, path = "/api/v2/c115/save", tag = "c115", request_body = C115SaveRequest, responses((status = 200, body = TaskRun)))]
pub async fn save(
    State(state): State<AppState>,
    Json(req): Json<C115SaveRequest>,
) -> AppResult<Json<TaskRun>> {
    let cookie =
        require_c115_cookie(config_store::get_string(&state.pool, C115_COOKIE_KEY).await?)?;
    let (cid, lib) =
        resolve_target_cid(&state.pool, req.cid.as_deref(), req.lib.as_deref()).await?;
    let label = c115_task_label("115 转存", &req.url, &cid);
    let params = c115_save_task_params(&req, &cid, lib.as_deref());
    let (task, created) =
        insert_or_reuse_c115_task_with_params(&state, "c115_save", &label, params).await?;
    if created {
        spawn_c115_save(state, task.id, cookie, req, cid, lib);
    }
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/c115/save/batch", tag = "c115", request_body = C115SaveBatchRequest, responses((status = 200, body = TaskRun)))]
pub async fn save_batch(
    State(state): State<AppState>,
    Json(req): Json<C115SaveBatchRequest>,
) -> AppResult<Json<TaskRun>> {
    validate_batch_len(req.items.len(), "转存")?;
    let cookie =
        require_c115_cookie(config_store::get_string(&state.pool, C115_COOKIE_KEY).await?)?;
    let (cid, lib) =
        match resolve_target_cid(&state.pool, req.cid.as_deref(), req.lib.as_deref()).await {
            Ok(target) => target,
            Err(_) => resolve_first_save_target(&state.pool, &req).await?,
        };
    let label = req
        .label
        .as_deref()
        .and_then(non_empty_trimmed)
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("115 批量转存: {} 项 -> cid={cid}", req.items.len()));
    let params = serde_json::json!({
        "items": &req.items,
        "cid": cid,
        "lib": lib,
        "label": &req.label,
    });
    let (task, created) =
        insert_or_reuse_c115_task_with_params(&state, "c115_save_batch", &label, params).await?;
    if created {
        spawn_c115_save_batch(state, task.id, cookie, req.items, cid, lib);
    }
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/c115/offline", tag = "c115", request_body = C115OfflineRequest, responses((status = 200, body = TaskRun)))]
pub async fn offline(
    State(state): State<AppState>,
    Json(req): Json<C115OfflineRequest>,
) -> AppResult<Json<TaskRun>> {
    let cookie =
        require_c115_cookie(config_store::get_string(&state.pool, C115_COOKIE_KEY).await?)?;
    let (cid, lib) =
        resolve_target_cid(&state.pool, req.cid.as_deref(), req.lib.as_deref()).await?;
    let label = c115_task_label("115 离线", &req.url, &cid);
    let (task, created) = insert_or_reuse_c115_task(&state, "c115_offline", &label).await?;
    if created {
        spawn_c115_offline(state, task.id, cookie, req, cid, lib);
    }
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/c115/offline/batch", tag = "c115", request_body = C115OfflineBatchRequest, responses((status = 200, body = TaskRun)))]
pub async fn offline_batch(
    State(state): State<AppState>,
    Json(req): Json<C115OfflineBatchRequest>,
) -> AppResult<Json<TaskRun>> {
    validate_batch_len(req.items.len(), "离线")?;
    let cookie =
        require_c115_cookie(config_store::get_string(&state.pool, C115_COOKIE_KEY).await?)?;
    let (cid, lib) =
        match resolve_target_cid(&state.pool, req.cid.as_deref(), req.lib.as_deref()).await {
            Ok(target) => target,
            Err(_) => resolve_first_offline_target(&state.pool, &req).await?,
        };
    let label = req
        .label
        .as_deref()
        .and_then(non_empty_trimmed)
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("115 批量离线: {} 项 -> cid={cid}", req.items.len()));
    let params = serde_json::json!({
        "items": &req.items,
        "cid": cid,
        "lib": lib,
        "label": &req.label,
    });
    let (task, created) =
        insert_or_reuse_c115_task_with_params(&state, "c115_offline_batch", &label, params).await?;
    if created {
        spawn_c115_offline_batch(state, task.id, cookie, req.items, cid, lib);
    }
    Ok(Json(task))
}

#[utoipa::path(post, path = "/api/v2/c115/auto-cid", tag = "c115", request_body = C115AutoCidRequest, responses((status = 200, body = C115AutoCidResponse)))]
pub async fn auto_cid(
    State(state): State<AppState>,
    Json(req): Json<C115AutoCidRequest>,
) -> AppResult<Json<C115AutoCidResponse>> {
    let plan = prepare_auto_cid(&state, &req).await?;
    Ok(Json(run_auto_cid_plan(&state, plan).await?))
}

#[utoipa::path(post, path = "/api/v2/c115/auto-cid/task", tag = "c115", request_body = C115AutoCidRequest, responses((status = 200, body = TaskRun)))]
pub async fn auto_cid_task(
    State(state): State<AppState>,
    Json(req): Json<C115AutoCidRequest>,
) -> AppResult<Json<TaskRun>> {
    let plan = prepare_auto_cid(&state, &req).await?;
    let label = format!("115 自动匹配 cid(depth {})", plan.max_depth);
    let params = serde_json::json!({
        "max_depth": plan.max_depth,
        "targets": &plan.targets,
        "current": &plan.current,
    });
    let (task, created) =
        insert_or_reuse_c115_task_with_params(&state, "c115_auto_cid", &label, params).await?;
    if created {
        spawn_c115_auto_cid(state, task.id, plan);
    }
    Ok(Json(task))
}

async fn prepare_auto_cid(
    state: &AppState,
    req: &C115AutoCidRequest,
) -> AppResult<C115AutoCidPlan> {
    let cookie =
        require_c115_cookie(config_store::get_string(&state.pool, C115_COOKIE_KEY).await?)?;
    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key is not configured; set it before auto cid".to_string(),
        ));
    }
    let libraries = EmbyClient::new(emby_url, api_key, state.http.clone())
        .libraries()
        .await?;
    let targets = libraries
        .into_iter()
        .flat_map(|lib| library_target_names(&lib.name, &lib.paths))
        .collect::<BTreeMap<_, _>>();
    let current = cid_map(&state.pool).await?;
    Ok(C115AutoCidPlan {
        cookie,
        targets,
        current,
        max_depth: req.max_depth.unwrap_or(2),
    })
}

async fn run_auto_cid_plan(
    state: &AppState,
    plan: C115AutoCidPlan,
) -> AppResult<C115AutoCidResponse> {
    let client = C115Client::new_with_site(C115_API, C115_SITE, plan.cookie, state.http.clone());
    client
        .auto_cid(plan.targets, plan.current, plan.max_depth)
        .await
}

fn c115_save_task_params(req: &C115SaveRequest, cid: &str, lib: Option<&str>) -> Value {
    let file_ids = req.file_ids.as_ref().map(|ids| {
        let mut ids = ids.clone();
        ids.sort();
        ids
    });
    serde_json::json!({
        "url": &req.url,
        "pwd": &req.pwd,
        "cid": cid,
        "lib": lib,
        "label": &req.label,
        "file_ids": file_ids,
    })
}

fn validate_batch_len(len: usize, label: &str) -> AppResult<()> {
    if len == 0 {
        return Err(AppError::BadRequest(format!(
            "{label}批量任务至少需要 1 项"
        )));
    }
    if len > 100 {
        return Err(AppError::BadRequest(format!(
            "{label}批量任务一次最多 100 项"
        )));
    }
    Ok(())
}

async fn resolve_first_save_target(
    pool: &sqlx::PgPool,
    req: &C115SaveBatchRequest,
) -> AppResult<(String, Option<String>)> {
    let first = req
        .items
        .first()
        .ok_or_else(|| AppError::BadRequest("转存批量任务至少需要 1 项".to_string()))?;
    resolve_target_cid(pool, first.cid.as_deref(), first.lib.as_deref()).await
}

async fn resolve_first_offline_target(
    pool: &sqlx::PgPool,
    req: &C115OfflineBatchRequest,
) -> AppResult<(String, Option<String>)> {
    let first = req
        .items
        .first()
        .ok_or_else(|| AppError::BadRequest("离线批量任务至少需要 1 项".to_string()))?;
    resolve_target_cid(pool, first.cid.as_deref(), first.lib.as_deref()).await
}

async fn insert_or_reuse_c115_task_with_params(
    state: &AppState,
    kind: &str,
    label: &str,
    params: Value,
) -> AppResult<(TaskRun, bool)> {
    if let Some(existing) = sqlx::query_as::<_, TaskRun>(
        "SELECT * FROM task_runs
         WHERE kind = $1 AND label = $2 AND params = $3 AND status = ANY($4)
         ORDER BY queued_at DESC
         LIMIT 1",
    )
    .bind(kind)
    .bind(label)
    .bind(params.clone())
    .bind(tasks::ACTIVE_STATUSES)
    .fetch_optional(&state.pool)
    .await?
    {
        return Ok((existing, false));
    }
    Ok((
        tasks::insert_task_with_meta(&state.pool, kind, label, 1, "manual", params).await?,
        true,
    ))
}

async fn insert_or_reuse_c115_task(
    state: &AppState,
    kind: &str,
    label: &str,
) -> AppResult<(TaskRun, bool)> {
    if let Some(existing) = sqlx::query_as::<_, TaskRun>(
        "SELECT * FROM task_runs
         WHERE kind = $1 AND label = $2 AND status = ANY($3)
         ORDER BY queued_at DESC
         LIMIT 1",
    )
    .bind(kind)
    .bind(label)
    .bind(tasks::ACTIVE_STATUSES)
    .fetch_optional(&state.pool)
    .await?
    {
        return Ok((existing, false));
    }
    Ok((tasks::insert_task(&state.pool, kind, label, 1).await?, true))
}

fn spawn_c115_save(
    state: AppState,
    id: Uuid,
    cookie: String,
    req: C115SaveRequest,
    cid: String,
    lib: Option<String>,
) {
    tokio::spawn(async move {
        run_c115_task(state, id, |state| async move {
            let client = C115Client::new_with_site(C115_API, C115_SITE, cookie, state.http.clone());
            let response = client.save_to_cid(req, cid, lib).await?;
            if response.ok {
                Ok(serde_json::to_value(response).unwrap_or_else(|_| serde_json::json!({})))
            } else {
                Err(AppError::BadRequest(response.msg))
            }
        })
        .await;
    });
}

fn spawn_c115_offline(
    state: AppState,
    id: Uuid,
    cookie: String,
    req: C115OfflineRequest,
    cid: String,
    lib: Option<String>,
) {
    tokio::spawn(async move {
        run_c115_task(state, id, |state| async move {
            let client = C115Client::new_with_site(C115_API, C115_SITE, cookie, state.http.clone());
            let response = client.offline_add(req, cid, lib).await?;
            if response.ok {
                Ok(serde_json::to_value(response).unwrap_or_else(|_| serde_json::json!({})))
            } else {
                Err(AppError::BadRequest(response.msg))
            }
        })
        .await;
    });
}

fn spawn_c115_save_batch(
    state: AppState,
    id: Uuid,
    cookie: String,
    items: Vec<C115SaveRequest>,
    cid: String,
    lib: Option<String>,
) {
    tokio::spawn(async move {
        run_c115_batch_task(state, id, items.len(), |state, id| async move {
            let client = C115Client::new_with_site(C115_API, C115_SITE, cookie, state.http.clone());
            let total_items = items.len();
            let mut results = Vec::with_capacity(items.len());
            let mut succeeded = 0usize;
            for (index, item) in items.into_iter().enumerate() {
                if tasks::cancel_requested(&state.pool, id).await {
                    tasks::finish_cancelled(&state.pool, id).await?;
                    return Err(AppError::Conflict("cancelled".to_string()));
                }
                tasks::set_progress(
                    &state.pool,
                    id,
                    index as i64,
                    &format!("转存 {}/{}", index + 1, total_items),
                )
                .await?;
                let url = item.url.clone();
                let label = item.label.clone();
                match client
                    .save_to_cid(item, cid.clone(), lib.clone())
                    .await
                    .and_then(|response| {
                        if response.ok {
                            Ok(response)
                        } else {
                            Err(AppError::BadRequest(response.msg))
                        }
                    }) {
                    Ok(response) => {
                        succeeded += 1;
                        results.push(serde_json::json!({
                            "ok": true,
                            "url": url,
                            "label": label,
                            "result": response,
                        }));
                    }
                    Err(err) => {
                        results.push(serde_json::json!({
                            "ok": false,
                            "url": url,
                            "label": label,
                            "err": err.to_string(),
                        }));
                    }
                }
                tasks::set_progress(
                    &state.pool,
                    id,
                    (index + 1) as i64,
                    &format!("已转存 {}/{}", index + 1, total_items),
                )
                .await?;
            }
            let total = results.len();
            let failed = total.saturating_sub(succeeded);
            Ok(serde_json::json!({
                "ok": failed == 0,
                "action": "c115_save_batch",
                "total": total,
                "succeeded": succeeded,
                "failed": failed,
                "cid": cid,
                "lib": lib,
                "items": results,
            }))
        })
        .await;
    });
}

fn spawn_c115_offline_batch(
    state: AppState,
    id: Uuid,
    cookie: String,
    items: Vec<C115OfflineRequest>,
    cid: String,
    lib: Option<String>,
) {
    tokio::spawn(async move {
        run_c115_batch_task(state, id, items.len(), |state, id| async move {
            let client = C115Client::new_with_site(C115_API, C115_SITE, cookie, state.http.clone());
            let total_items = items.len();
            let mut results = Vec::with_capacity(items.len());
            let mut succeeded = 0usize;
            for (index, item) in items.into_iter().enumerate() {
                if tasks::cancel_requested(&state.pool, id).await {
                    tasks::finish_cancelled(&state.pool, id).await?;
                    return Err(AppError::Conflict("cancelled".to_string()));
                }
                tasks::set_progress(
                    &state.pool,
                    id,
                    index as i64,
                    &format!("离线 {}/{}", index + 1, total_items),
                )
                .await?;
                let url = item.url.clone();
                let label = item.label.clone();
                match client
                    .offline_add(item, cid.clone(), lib.clone())
                    .await
                    .and_then(|response| {
                        if response.ok {
                            Ok(response)
                        } else {
                            Err(AppError::BadRequest(response.msg))
                        }
                    }) {
                    Ok(response) => {
                        succeeded += 1;
                        results.push(serde_json::json!({
                            "ok": true,
                            "url": url,
                            "label": label,
                            "result": response,
                        }));
                    }
                    Err(err) => {
                        results.push(serde_json::json!({
                            "ok": false,
                            "url": url,
                            "label": label,
                            "err": err.to_string(),
                        }));
                    }
                }
                tasks::set_progress(
                    &state.pool,
                    id,
                    (index + 1) as i64,
                    &format!("已离线 {}/{}", index + 1, total_items),
                )
                .await?;
            }
            let total = results.len();
            let failed = total.saturating_sub(succeeded);
            Ok(serde_json::json!({
                "ok": failed == 0,
                "action": "c115_offline_batch",
                "total": total,
                "succeeded": succeeded,
                "failed": failed,
                "cid": cid,
                "lib": lib,
                "items": results,
            }))
        })
        .await;
    });
}

fn spawn_c115_auto_cid(state: AppState, id: Uuid, plan: C115AutoCidPlan) {
    tokio::spawn(async move {
        run_c115_task(state, id, |state| async move {
            let response = run_auto_cid_plan(&state, plan).await?;
            Ok(serde_json::to_value(response).unwrap_or_else(|_| serde_json::json!({})))
        })
        .await;
    });
}

async fn run_c115_task<F, Fut>(state: AppState, id: Uuid, op: F)
where
    F: FnOnce(AppState) -> Fut,
    Fut: std::future::Future<Output = AppResult<Value>>,
{
    let Ok(_permit) = state.clouddrive_slot.clone().acquire_owned().await else {
        let _ = mark_task_error(&state, id, "115 任务串行锁不可用", None).await;
        return;
    };
    if task_cancel_requested(&state, id).await {
        let _ = mark_task_cancelled(&state, id).await;
        return;
    }
    let _ = sqlx::query(
        "UPDATE task_runs
         SET status = 'running', started_at = COALESCE(started_at, now()), status_text = '等待 115 响应...', updated_at = now()
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(id)
    .execute(&state.pool)
    .await;
    match op(state.clone()).await {
        Ok(result) => {
            let _ = sqlx::query(
                "UPDATE task_runs
                 SET status = 'done', progress = total, status_text = '完成', result = $2, ended_at = now(), updated_at = now()
                 WHERE id = $1",
            )
            .bind(id)
            .bind(result)
            .execute(&state.pool)
            .await;
        }
        Err(err) => {
            let _ = mark_task_error(&state, id, &err.to_string(), None).await;
        }
    }
}

async fn run_c115_batch_task<F, Fut>(state: AppState, id: Uuid, total: usize, op: F)
where
    F: FnOnce(AppState, Uuid) -> Fut,
    Fut: std::future::Future<Output = AppResult<Value>>,
{
    let Ok(_permit) = state.clouddrive_slot.clone().acquire_owned().await else {
        let _ = mark_task_error(&state, id, "115 任务串行锁不可用", None).await;
        return;
    };
    if task_cancel_requested(&state, id).await {
        let _ = mark_task_cancelled(&state, id).await;
        return;
    }
    let _ = tasks::set_total(&state.pool, id, total.max(1) as i64).await;
    let _ = tasks::mark_running(&state.pool, id, "批量 115 任务启动").await;
    match op(state.clone(), id).await {
        Ok(result) => {
            let failed = result.get("failed").and_then(Value::as_u64).unwrap_or(0);
            let status_text = if failed == 0 {
                "完成".to_string()
            } else {
                format!("完成，{failed} 项失败")
            };
            let _ = tasks::finish_done_with_message(&state.pool, id, &status_text, result).await;
        }
        Err(AppError::Conflict(message)) if message == "cancelled" => {}
        Err(err) => {
            let _ = mark_task_error(&state, id, &err.to_string(), None).await;
        }
    }
}

async fn task_cancel_requested(state: &AppState, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT cancel_requested FROM task_runs WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}

async fn mark_task_cancelled(state: &AppState, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE task_runs
         SET status = 'cancelled', status_text = '已取消', ended_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn mark_task_error(
    state: &AppState,
    id: Uuid,
    error: &str,
    result: Option<Value>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE task_runs
         SET status = 'error', status_text = '失败', error = $2, result = $3, ended_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(error)
    .bind(result)
    .execute(&state.pool)
    .await?;
    Ok(())
}

fn c115_task_label(prefix: &str, url: &str, cid: &str) -> String {
    let mut clean_url = url.trim().replace(['\n', '\r', '\t'], " ");
    if clean_url.chars().count() > 96 {
        clean_url = clean_url.chars().take(96).collect::<String>();
        clean_url.push_str("...");
    }
    format!("{prefix}: {clean_url} -> cid={cid}")
}

pub fn require_c115_cookie(raw: Option<String>) -> AppResult<String> {
    let cookie = raw.unwrap_or_default();
    let cookie = cookie.trim();
    if cookie.is_empty() {
        Err(AppError::BadRequest(
            "未设置 115 cookie(c115_cookie)，请先在设置页填写".to_string(),
        ))
    } else {
        Ok(cookie.to_string())
    }
}

#[utoipa::path(post, path = "/api/v2/c115/parse", tag = "c115", request_body = ShareUrl, responses((status = 200, body = C115ParseResponse)))]
pub async fn parse_url(Json(req): Json<ShareUrl>) -> Json<C115ParseResponse> {
    let (share, rc) = parse_115_url(&req.url, req.pwd.as_deref());
    Json(C115ParseResponse {
        share,
        receive_code: rc,
    })
}

pub fn parse_115_url(url: &str, pwd: Option<&str>) -> (Option<String>, Option<String>) {
    let url = url.trim();
    let (share, manual_rc) = parse_share_parts(url);
    let rc = pwd
        .and_then(non_empty_trimmed)
        .map(ToString::to_string)
        .or_else(|| parse_receive_code(url))
        .or(manual_rc);
    (share, rc)
}

fn parse_share_parts(url: &str) -> (Option<String>, Option<String>) {
    if let Some((_, rest)) = url.split_once("/s/") {
        return (take_ascii_alnum(rest), None);
    }

    let mut tokens = url
        .split(|ch: char| ch.is_whitespace() || ch == ',')
        .filter_map(non_empty_trimmed);
    let share = tokens
        .next()
        .filter(|token| token.chars().all(|ch| ch.is_ascii_alphanumeric()))
        .map(ToString::to_string);
    let rc = tokens
        .next()
        .filter(|token| token.chars().all(|ch| ch.is_ascii_alphanumeric()))
        .map(ToString::to_string);
    (share, rc)
}

fn parse_receive_code(url: &str) -> Option<String> {
    url.split(['?', '&']).find_map(|part| {
        let (key, value) = part.split_once('=')?;
        if matches!(key, "password" | "pwd" | "pickcode") {
            take_query_value(value)
        } else {
            None
        }
    })
}

fn take_ascii_alnum(input: &str) -> Option<String> {
    let token: String = input
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric())
        .collect();
    (!token.is_empty()).then_some(token)
}

fn take_query_value(input: &str) -> Option<String> {
    let token = input
        .split(|ch: char| ch == '#' || ch == '&' || ch.is_whitespace())
        .next()?;
    non_empty_trimmed(token).map(ToString::to_string)
}

fn non_empty_trimmed(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

impl C115SnapData {
    fn total_hint(&self) -> Option<usize> {
        value_as_usize(self.total.as_ref())
            .or_else(|| value_as_usize(self.count.as_ref()))
            .or_else(|| value_as_usize(self.file_count.as_ref()))
            .or_else(|| {
                self.shareinfo.as_ref().and_then(|info| {
                    value_as_usize(info.total.as_ref())
                        .or_else(|| value_as_usize(info.count.as_ref()))
                        .or_else(|| value_as_usize(info.file_count.as_ref()))
                })
            })
    }
}

impl C115ShareInfo {
    fn title(&self) -> Option<String> {
        self.share_title
            .as_deref()
            .or(self.file_name.as_deref())
            .and_then(non_empty_trimmed)
            .map(ToString::to_string)
    }

    fn size(&self) -> Option<Value> {
        self.file_size.clone().or_else(|| self.total_size.clone())
    }
}

fn snap_file_from_item(
    item: C115SnapItem,
    wanted: Option<&HashSet<String>>,
) -> Option<C115SnapFile> {
    let file_id = item
        .fid
        .as_ref()
        .and_then(value_to_string)
        .or_else(|| item.file_id.as_ref().and_then(value_to_string));
    let id = file_id
        .clone()
        .or_else(|| item.cid.as_ref().and_then(value_to_string));
    if let Some(wanted) = wanted {
        if !id.as_ref().is_some_and(|id| wanted.contains(id)) {
            return None;
        }
    }

    Some(C115SnapFile {
        id,
        name: item
            .n
            .or(item.name)
            .and_then(|name| non_empty_trimmed(&name).map(ToString::to_string))
            .unwrap_or_default(),
        size: item
            .s
            .as_ref()
            .and_then(value_as_u64)
            .or_else(|| item.size.as_ref().and_then(value_as_u64))
            .unwrap_or(0),
        is_dir: file_id.is_none(),
    })
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => non_empty_trimmed(s).map(ToString::to_string),
        Value::Number(n) => Some(n.to_string()).filter(|s| !s.is_empty()),
        _ => None,
    }
}

fn c115_delete_is_busy(result: &C115GenericApiResponse) -> bool {
    result
        .errno
        .as_ref()
        .and_then(value_to_string)
        .is_some_and(|errno| errno == "990009")
        || result
            .error
            .as_deref()
            .or(result.msg.as_deref())
            .is_some_and(|message| message.contains("尚未执行完成") || message.contains("稍后再试"))
}

fn c115_generic_error(result: C115GenericApiResponse, fallback: &str) -> String {
    result
        .error
        .or(result.msg)
        .or_else(|| result.errno.map(|errno| format!("errno={errno}")))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|v| u64::try_from(v).ok())),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn value_as_usize(value: Option<&Value>) -> Option<usize> {
    value
        .and_then(value_as_u64)
        .and_then(|v| usize::try_from(v).ok())
}

pub fn c115_uid_from_cookie(cookie: &str) -> Option<String> {
    cookie.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == "UID").then(|| value.split('_').next().unwrap_or("").to_string())
    })
}

pub fn validate_target_cid(cid: &str) -> AppResult<String> {
    let cid = cid.trim();
    let mut chars = cid.chars();
    let Some(first) = chars.next() else {
        return Err(AppError::BadRequest("未指定目标 115 目录 cid".to_string()));
    };
    if !matches!(first, '1'..='9') || !chars.all(|ch| ch.is_ascii_digit()) {
        return Err(AppError::BadRequest(
            "目标 cid 非法(必须正整数,0=根目录不允许;检查库的 cid 配置)".to_string(),
        ));
    }
    Ok(cid.to_string())
}

async fn resolve_target_cid(
    pool: &sqlx::PgPool,
    explicit_cid: Option<&str>,
    lib: Option<&str>,
) -> AppResult<(String, Option<String>)> {
    if let Some(cid) = explicit_cid.and_then(non_empty_trimmed) {
        return Ok((
            validate_target_cid(cid)?,
            lib.and_then(non_empty_trimmed).map(ToString::to_string),
        ));
    }
    let lib = lib
        .and_then(non_empty_trimmed)
        .ok_or_else(|| AppError::BadRequest("未指定目标库或 cid".to_string()))?;
    let map = cid_map(pool).await?;
    let cid = map
        .get(lib)
        .ok_or_else(|| AppError::BadRequest(format!("库「{lib}」没配 115 cid,去设置页填")))?;
    Ok((validate_target_cid(cid)?, Some(lib.to_string())))
}

pub async fn cid_map(pool: &sqlx::PgPool) -> AppResult<BTreeMap<String, String>> {
    let Some(value) = config_store::get_raw(pool, C115_CID_MAP_KEY).await? else {
        return Ok(BTreeMap::new());
    };
    let map = value
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
        .unwrap_or_default();
    Ok(map)
}

fn library_target_names(lib_name: &str, paths: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in paths {
        let normalized = path.trim().replace('\\', "/");
        if let Some(name) = normalized
            .rsplit('/')
            .find(|seg| !seg.trim().is_empty())
            .and_then(non_empty_trimmed)
        {
            out.push((name.to_string(), lib_name.to_string()));
        }
    }
    if out.is_empty() {
        if let Some(name) = non_empty_trimmed(lib_name) {
            out.push((name.to_string(), lib_name.to_string()));
        }
    }
    out
}

fn used_space(data: &Option<C115IndexData>) -> String {
    data.as_ref()
        .and_then(|d| d.space_info.as_ref())
        .and_then(|space| space.all_total.as_ref().or(space.all_use.as_ref()))
        .and_then(|size| size.size_format.as_deref())
        .unwrap_or("")
        .to_string()
}

fn body_snippet(body: &str) -> String {
    body.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url_and_manual_forms() {
        assert_eq!(
            parse_115_url(" https://115.com/s/swABC?password=YYY#anchor ", None),
            (Some("swABC".to_string()), Some("YYY".to_string()))
        );
        assert_eq!(
            parse_115_url("swABC,YYY", None),
            (Some("swABC".to_string()), Some("YYY".to_string()))
        );
        assert_eq!(
            parse_115_url("swABC YYY", Some(" OVERRIDE ")),
            (Some("swABC".to_string()), Some("OVERRIDE".to_string()))
        );
    }
}
