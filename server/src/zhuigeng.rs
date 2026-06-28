use crate::{
    config_store,
    error::{AppError, AppResult},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use reqwest::Client;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::PgPool;
use std::{collections::BTreeMap, time::Duration};

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 10;
const MAX_REQUEST_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone)]
pub struct ZhuigengConfig {
    pub emby_base_url: String,
    pub emby_api_key: String,
    pub tmdb_base_url: String,
    pub tmdb_api_key: String,
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengStatusResponse {
    pub ok: bool,
    pub items: Vec<ZhuigengItem>,
    pub continuing: usize,
    pub ended: usize,
    pub total: usize,
    pub copy_text: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengItem {
    pub lib: String,
    pub name: String,
    pub id: Option<String>,
    pub folder: String,
    pub tmdb: String,
    pub tmdb_status: String,
    pub state: String,
    pub continuing: bool,
    pub ended: bool,
    pub local_count: usize,
    pub local_latest: Option<String>,
    pub local_latest_episode: Option<String>,
    pub last_episode_to_air: Option<TmdbEpisodeSummary>,
    pub next_episode_to_air: Option<TmdbEpisodeSummary>,
    pub behind: usize,
    pub behind_hint: Option<String>,
    pub resource_hint: Option<String>,
    pub err: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct TmdbEpisodeSummary {
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub air_date: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengScanAiringResponse {
    pub ok: bool,
    pub total: usize,
    pub results: Vec<ZhuigengScanAiringRow>,
    pub copy_text: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengScanAiringRow {
    pub lib: String,
    pub name: String,
    pub id: Option<String>,
    pub tmdb: String,
    pub status: String,
    pub behind: usize,
    pub hint: Option<String>,
    pub ok: bool,
    pub err: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengGapsSummaryResponse {
    pub ok: bool,
    pub items: Vec<ZhuigengGapRow>,
    pub total: usize,
    pub copy_text: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZhuigengGapRow {
    pub lib: String,
    pub name: String,
    pub id: Option<String>,
    pub tmdb: String,
    pub fmt: String,
    pub behind: usize,
}

#[derive(Debug, Deserialize)]
struct EmbyVirtualFolderLite {
    #[serde(rename = "ItemId")]
    item_id: Option<String>,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "CollectionType")]
    collection_type: Option<String>,
    #[serde(rename = "Locations", default)]
    locations: Vec<String>,
    #[serde(rename = "LibraryOptions")]
    library_options: Option<EmbyLibraryOptionsLite>,
}

#[derive(Debug, Deserialize)]
struct EmbyLibraryOptionsLite {
    #[serde(rename = "PathInfos", default)]
    path_infos: Vec<EmbyPathInfoLite>,
}

#[derive(Debug, Deserialize)]
struct EmbyPathInfoLite {
    #[serde(rename = "Path")]
    path: Option<String>,
}

#[derive(Debug, Clone)]
struct ZhuigengLibrary {
    id: String,
    name: String,
    library_type: String,
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EmbyItemsPageLite {
    #[serde(rename = "Items", default)]
    items: Vec<EmbySeriesLite>,
}

#[derive(Debug, Deserialize)]
struct EmbySeriesLite {
    #[serde(rename = "Id")]
    id: Option<String>,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Path")]
    path: Option<String>,
    #[serde(rename = "ProviderIds", default)]
    provider_ids: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct EmbyEpisodesPageLite {
    #[serde(rename = "Items", default)]
    items: Vec<EmbyEpisodeLite>,
}

#[derive(Debug, Deserialize)]
struct EmbyEpisodeLite {
    #[serde(rename = "ParentIndexNumber")]
    parent_index_number: Option<i32>,
    #[serde(rename = "IndexNumber")]
    index_number: Option<i32>,
    #[serde(rename = "LocationType")]
    location_type: Option<String>,
    #[serde(rename = "PremiereDate")]
    premiere_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TmdbTvResponse {
    status: Option<String>,
    last_episode_to_air: Option<TmdbEpisodeSummary>,
    next_episode_to_air: Option<TmdbEpisodeSummary>,
}

#[derive(Debug)]
struct LocalEpisodeSummary {
    count: usize,
    latest_date: Option<String>,
    latest_episode: Option<String>,
    have_by_season: BTreeMap<Option<i32>, Vec<i32>>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/zhuigeng", get(status))
        .route("/api/v2/zhuigeng/scan-airing", post(scan_airing))
        .route("/api/v2/zhuigeng/scan_airing", post(scan_airing))
        .route("/api/v2/zhuigeng/gaps-summary", post(gaps_summary))
        .route("/api/v2/zhuigeng/gaps_summary", post(gaps_summary))
}

#[utoipa::path(get, path = "/api/v2/zhuigeng", tag = "zhuigeng", responses((status = 200, body = ZhuigengStatusResponse)))]
pub async fn status(State(state): State<AppState>) -> AppResult<Json<ZhuigengStatusResponse>> {
    let config = zhuigeng_config_from_state(&state).await?;
    Ok(Json(
        zhuigeng_status_with_config(config, state.http.clone()).await?,
    ))
}

#[utoipa::path(post, path = "/api/v2/zhuigeng/scan-airing", tag = "zhuigeng", responses((status = 200, body = ZhuigengScanAiringResponse)))]
pub async fn scan_airing(
    State(state): State<AppState>,
) -> AppResult<Json<ZhuigengScanAiringResponse>> {
    let config = zhuigeng_config_from_state(&state).await?;
    Ok(Json(
        zhuigeng_scan_airing_with_config(config, state.http.clone()).await?,
    ))
}

#[utoipa::path(post, path = "/api/v2/zhuigeng/gaps-summary", tag = "zhuigeng", responses((status = 200, body = ZhuigengGapsSummaryResponse)))]
pub async fn gaps_summary(
    State(state): State<AppState>,
) -> AppResult<Json<ZhuigengGapsSummaryResponse>> {
    let config = zhuigeng_config_from_state(&state).await?;
    Ok(Json(
        zhuigeng_gaps_summary_with_config(config, state.http.clone()).await?,
    ))
}

pub async fn zhuigeng_status_with_config(
    config: ZhuigengConfig,
    http: Client,
) -> AppResult<ZhuigengStatusResponse> {
    config.validate()?;
    let emby = ZhuigengEmbyClient::new(
        config.emby_base_url.clone(),
        config.emby_api_key.clone(),
        config.request_timeout,
        http.clone(),
    );
    let tmdb = TmdbClient::new(
        config.tmdb_base_url.clone(),
        config.tmdb_api_key.clone(),
        config.request_timeout,
        http,
    );

    let libraries = emby.zhuigeng_libraries().await?;
    let mut items = Vec::new();
    for library in libraries {
        let series = emby.series(&library.id).await?;
        for item in series {
            let name = item.name.clone().unwrap_or_else(|| "(无名)".to_string());
            let folder = folder_from_series_path(item.path.as_deref(), &library);
            let tmdb_id = provider_id(&item.provider_ids, "Tmdb").unwrap_or_default();
            let (local, episode_err) = match item.id.as_deref() {
                Some(series_id) => match emby.episodes(series_id).await {
                    Ok(episodes) => (summarize_local_episodes(&episodes), None),
                    Err(err) => (summarize_local_episodes(&[]), Some(err.to_string())),
                },
                None => (
                    summarize_local_episodes(&[]),
                    Some("Emby Series 缺少 Id，无法读取本地集数".to_string()),
                ),
            };

            let mut row = match validate_tmdb_id(&tmdb_id) {
                Ok(()) => match tmdb.tv(&tmdb_id).await {
                    Ok(meta) => {
                        build_item_from_tmdb(&library, &item, &name, folder, tmdb_id, local, meta)
                    }
                    Err(err) => build_error_item(
                        &library,
                        &item,
                        &name,
                        folder,
                        tmdb_id,
                        local,
                        err.to_string(),
                    ),
                },
                Err(err) => build_error_item(
                    &library,
                    &item,
                    &name,
                    folder,
                    tmdb_id,
                    local,
                    err.to_string(),
                ),
            };
            if let Some(err) = episode_err {
                row.err = match row.err {
                    Some(existing) => Some(format!("{existing}; {err}")),
                    None => Some(err),
                };
            }
            items.push(row);
        }
    }

    items.sort_by(|left, right| {
        right
            .continuing
            .cmp(&left.continuing)
            .then_with(|| right.behind.cmp(&left.behind))
            .then_with(|| left.name.cmp(&right.name))
    });
    let continuing = items.iter().filter(|item| item.continuing).count();
    let ended = items.iter().filter(|item| item.ended).count();
    let copy_text = build_copy_text(&items, true);
    Ok(ZhuigengStatusResponse {
        ok: true,
        total: items.len(),
        items,
        continuing,
        ended,
        copy_text,
    })
}

pub async fn zhuigeng_scan_airing_with_config(
    config: ZhuigengConfig,
    http: Client,
) -> AppResult<ZhuigengScanAiringResponse> {
    let status = zhuigeng_status_with_config(config, http).await?;
    let results = status
        .items
        .iter()
        .filter(|item| item.continuing)
        .map(|item| ZhuigengScanAiringRow {
            lib: item.lib.clone(),
            name: item.name.clone(),
            id: item.id.clone(),
            tmdb: item.tmdb.clone(),
            status: item.tmdb_status.clone(),
            behind: item.behind,
            hint: item.behind_hint.clone(),
            ok: item.err.is_none(),
            err: item.err.clone(),
        })
        .collect::<Vec<_>>();
    Ok(ZhuigengScanAiringResponse {
        ok: true,
        total: results.len(),
        results,
        copy_text: status.copy_text,
        note: "最小 TMDb 语义版：仅汇总在更剧状态和落后提示，不触发文件扫描".to_string(),
    })
}

pub async fn zhuigeng_gaps_summary_with_config(
    config: ZhuigengConfig,
    http: Client,
) -> AppResult<ZhuigengGapsSummaryResponse> {
    let status = zhuigeng_status_with_config(config, http).await?;
    let items = status
        .items
        .iter()
        .filter(|item| item.continuing && item.behind > 0)
        .filter_map(|item| {
            Some(ZhuigengGapRow {
                lib: item.lib.clone(),
                name: item.name.clone(),
                id: item.id.clone(),
                tmdb: item.tmdb.clone(),
                fmt: item.resource_hint.clone()?,
                behind: item.behind,
            })
        })
        .collect::<Vec<_>>();
    let copy_text = items
        .iter()
        .map(|item| {
            if item.tmdb.trim().is_empty() {
                format!("求 {} — {}", item.name, item.fmt)
            } else {
                format!("求 {} [tmdb:{}] — {}", item.name, item.tmdb, item.fmt)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ZhuigengGapsSummaryResponse {
        ok: true,
        total: items.len(),
        items,
        copy_text,
    })
}

impl ZhuigengConfig {
    pub fn new(
        emby_base_url: impl Into<String>,
        emby_api_key: impl Into<String>,
        tmdb_base_url: impl Into<String>,
        tmdb_api_key: impl Into<String>,
    ) -> Self {
        Self {
            emby_base_url: emby_base_url.into(),
            emby_api_key: emby_api_key.into(),
            tmdb_base_url: tmdb_base_url.into(),
            tmdb_api_key: tmdb_api_key.into(),
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
        }
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    fn validate(&self) -> AppResult<()> {
        if self.emby_api_key.trim().is_empty() {
            return Err(AppError::BadRequest(
                "api_key 未配置，无法读取 Emby 追更库".to_string(),
            ));
        }
        if self.tmdb_base_url.trim().is_empty() {
            return Err(AppError::BadRequest(
                "tmdb_base_url/tmdb_url 未配置，无法请求 TMDb".to_string(),
            ));
        }
        if self.tmdb_api_key.trim().is_empty() {
            return Err(AppError::BadRequest(
                "tmdb_api_key/tmdb_key 未配置，无法请求 TMDb".to_string(),
            ));
        }
        if self.request_timeout.is_zero() {
            return Err(AppError::BadRequest(
                "tmdb_timeout_secs 必须是 1-60 的整数秒".to_string(),
            ));
        }
        Ok(())
    }
}

async fn zhuigeng_config_from_state(state: &AppState) -> AppResult<ZhuigengConfig> {
    let emby_base_url =
        config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let emby_api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    let tmdb_base_url = get_first_string(&state.pool, &["tmdb_base_url", "tmdb_url"])
        .await?
        .unwrap_or_default();
    let tmdb_api_key = get_first_string(&state.pool, &["tmdb_api_key", "tmdb_key"])
        .await?
        .unwrap_or_default();
    let request_timeout = timeout_from_config(&state.pool).await?;
    let config = ZhuigengConfig {
        emby_base_url,
        emby_api_key,
        tmdb_base_url,
        tmdb_api_key,
        request_timeout,
    };
    config.validate()?;
    Ok(config)
}

async fn get_first_string(pool: &PgPool, keys: &[&str]) -> Result<Option<String>, sqlx::Error> {
    for key in keys {
        if let Some(value) = config_store::get_string(pool, key).await?
            && !value.trim().is_empty()
        {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

async fn timeout_from_config(pool: &PgPool) -> AppResult<Duration> {
    let Some(value) = config_store::get_raw(pool, "tmdb_timeout_secs").await? else {
        return Ok(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS));
    };
    let secs = match value {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    };
    let Some(secs) = secs else {
        return Err(AppError::BadRequest(
            "tmdb_timeout_secs 必须是 1-60 的整数秒".to_string(),
        ));
    };
    if !(1..=MAX_REQUEST_TIMEOUT_SECS).contains(&secs) {
        return Err(AppError::BadRequest(
            "tmdb_timeout_secs 必须是 1-60 的整数秒".to_string(),
        ));
    }
    Ok(Duration::from_secs(secs))
}

#[derive(Clone)]
struct ZhuigengEmbyClient {
    base_url: String,
    api_key: String,
    timeout: Duration,
    http: Client,
}

impl ZhuigengEmbyClient {
    fn new(base_url: String, api_key: String, timeout: Duration, http: Client) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.trim().to_string(),
            timeout,
            http,
        }
    }

    async fn zhuigeng_libraries(&self) -> AppResult<Vec<ZhuigengLibrary>> {
        let url = format!("{}/Library/VirtualFolders", self.base_url);
        let folders: Vec<EmbyVirtualFolderLite> = get_json(
            &self.http,
            "Emby",
            "/Library/VirtualFolders",
            &url,
            &[("api_key", self.api_key.as_str())],
            self.timeout,
        )
        .await?;
        Ok(folders
            .into_iter()
            .filter_map(ZhuigengLibrary::from_virtual_folder)
            .filter(|library| {
                library.name.contains("追更")
                    && library.library_type.eq_ignore_ascii_case("tvshows")
            })
            .collect())
    }

    async fn series(&self, parent_id: &str) -> AppResult<Vec<EmbySeriesLite>> {
        let url = format!("{}/Items", self.base_url);
        let query = [
            ("api_key", self.api_key.as_str()),
            ("ParentId", parent_id),
            ("Recursive", "true"),
            ("IncludeItemTypes", "Series"),
            ("Fields", "Status,Path,ProviderIds"),
            ("SortBy", "SortName"),
            ("Limit", "30000"),
        ];
        Ok(
            get_json::<EmbyItemsPageLite>(&self.http, "Emby", "/Items", &url, &query, self.timeout)
                .await?
                .items,
        )
    }

    async fn episodes(&self, series_id: &str) -> AppResult<Vec<EmbyEpisodeLite>> {
        let path = format!("/Shows/{}/Episodes", urlencoding::encode(series_id.trim()));
        let url = format!("{}{}", self.base_url, path);
        let query = [
            ("api_key", self.api_key.as_str()),
            (
                "Fields",
                "PremiereDate,ParentIndexNumber,IndexNumber,LocationType",
            ),
            ("Limit", "6000"),
        ];
        Ok(
            get_json::<EmbyEpisodesPageLite>(&self.http, "Emby", &path, &url, &query, self.timeout)
                .await?
                .items,
        )
    }
}

impl ZhuigengLibrary {
    fn from_virtual_folder(folder: EmbyVirtualFolderLite) -> Option<Self> {
        let id = folder.item_id?.trim().to_string();
        if id.is_empty() {
            return None;
        }
        let name = if folder.name.trim().is_empty() {
            "(unnamed)".to_string()
        } else {
            folder.name
        };
        let mut paths = Vec::new();
        for path in folder.locations {
            push_path(&mut paths, path);
        }
        if let Some(options) = folder.library_options {
            for info in options.path_infos {
                if let Some(path) = info.path {
                    push_path(&mut paths, path);
                }
            }
        }
        Some(Self {
            id,
            name,
            library_type: folder
                .collection_type
                .unwrap_or_else(|| "mixed".to_string()),
            paths,
        })
    }
}

#[derive(Clone)]
struct TmdbClient {
    base_url: String,
    api_key: String,
    timeout: Duration,
    http: Client,
}

impl TmdbClient {
    fn new(base_url: String, api_key: String, timeout: Duration, http: Client) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.trim().to_string(),
            timeout,
            http,
        }
    }

    async fn tv(&self, tmdb_id: &str) -> AppResult<TmdbTvResponse> {
        let path = if self.base_url.ends_with("/3") {
            format!("/tv/{tmdb_id}")
        } else {
            format!("/3/tv/{tmdb_id}")
        };
        let url = format!("{}{}", self.base_url, path);
        get_json(
            &self.http,
            "TMDb",
            &path,
            &url,
            &[("api_key", self.api_key.as_str())],
            self.timeout,
        )
        .await
    }
}

async fn get_json<T>(
    http: &Client,
    service: &str,
    path: &str,
    url: &str,
    query: &[(&str, &str)],
    timeout: Duration,
) -> AppResult<T>
where
    T: DeserializeOwned,
{
    let response = http
        .get(url)
        .query(query)
        .timeout(timeout)
        .send()
        .await
        .map_err(|err| request_error(service, path, err))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::BadRequest(format!(
            "{service} {path} 返回 HTTP {status}"
        )));
    }
    response
        .json()
        .await
        .map_err(|err| AppError::BadRequest(format!("{service} {path} JSON 解析失败: {err}")))
}

fn request_error(service: &str, path: &str, err: reqwest::Error) -> AppError {
    if err.is_timeout() {
        AppError::BadRequest(format!("{service} {path} 请求超时"))
    } else {
        AppError::BadRequest(format!("{service} {path} 请求失败: {err}"))
    }
}

fn build_item_from_tmdb(
    library: &ZhuigengLibrary,
    item: &EmbySeriesLite,
    name: &str,
    folder: String,
    tmdb: String,
    local: LocalEpisodeSummary,
    meta: TmdbTvResponse,
) -> ZhuigengItem {
    let status = meta.status.unwrap_or_else(|| "?".to_string());
    let (state, continuing, ended) = tmdb_state(&status, meta.next_episode_to_air.as_ref());
    let behind = compute_behind(&local, meta.last_episode_to_air.as_ref());
    ZhuigengItem {
        lib: library.name.clone(),
        name: name.to_string(),
        id: item.id.clone(),
        folder,
        tmdb,
        tmdb_status: status,
        state,
        continuing,
        ended,
        local_count: local.count,
        local_latest: local.latest_date,
        local_latest_episode: local.latest_episode,
        last_episode_to_air: meta.last_episode_to_air,
        next_episode_to_air: meta.next_episode_to_air,
        behind: behind.count,
        behind_hint: behind.hint,
        resource_hint: behind.resource_hint,
        err: None,
    }
}

fn build_error_item(
    library: &ZhuigengLibrary,
    item: &EmbySeriesLite,
    name: &str,
    folder: String,
    tmdb: String,
    local: LocalEpisodeSummary,
    err: String,
) -> ZhuigengItem {
    ZhuigengItem {
        lib: library.name.clone(),
        name: name.to_string(),
        id: item.id.clone(),
        folder,
        tmdb,
        tmdb_status: "?".to_string(),
        state: "unknown".to_string(),
        continuing: false,
        ended: false,
        local_count: local.count,
        local_latest: local.latest_date,
        local_latest_episode: local.latest_episode,
        last_episode_to_air: None,
        next_episode_to_air: None,
        behind: 0,
        behind_hint: None,
        resource_hint: None,
        err: Some(err),
    }
}

#[derive(Debug)]
struct BehindSummary {
    count: usize,
    hint: Option<String>,
    resource_hint: Option<String>,
}

fn compute_behind(
    local: &LocalEpisodeSummary,
    last_episode: Option<&TmdbEpisodeSummary>,
) -> BehindSummary {
    let Some(last_episode) = last_episode else {
        return BehindSummary {
            count: 0,
            hint: None,
            resource_hint: None,
        };
    };
    let Some(last_number) = last_episode.episode_number else {
        return BehindSummary {
            count: 0,
            hint: None,
            resource_hint: None,
        };
    };
    if last_number <= 0 {
        return BehindSummary {
            count: 0,
            hint: None,
            resource_hint: None,
        };
    }

    let season = last_episode.season_number;
    let have = local
        .have_by_season
        .get(&season)
        .cloned()
        .unwrap_or_default();
    let missing = (1..=last_number)
        .filter(|number| !have.contains(number))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return BehindSummary {
            count: 0,
            hint: None,
            resource_hint: None,
        };
    }

    let last_label =
        episode_label(season, Some(last_number)).unwrap_or_else(|| "最新集".to_string());
    let local_label = have
        .iter()
        .max()
        .and_then(|number| episode_label(season, Some(*number)))
        .or_else(|| local.latest_episode.clone())
        .unwrap_or_else(|| "本地无已入库集".to_string());
    let mut hint = format!(
        "落后 TMDb {} 集 · 最新 {} · 本地到 {}",
        missing.len(),
        last_label,
        local_label
    );
    if let Some(date) = last_episode
        .air_date
        .as_deref()
        .filter(|date| !date.is_empty())
    {
        hint.push_str(&format!(" · {date}"));
    }
    let resource_hint = match season {
        Some(season) => Some(format!("S{:02} E{}", season, compact_ints(&missing))),
        None => Some(format!("E{}", compact_ints(&missing))),
    };
    BehindSummary {
        count: missing.len(),
        hint: Some(hint),
        resource_hint,
    }
}

fn summarize_local_episodes(episodes: &[EmbyEpisodeLite]) -> LocalEpisodeSummary {
    let mut count = 0usize;
    let mut latest_date = None::<String>;
    let mut latest_pair = None::<(Option<i32>, i32)>;
    let mut have_by_season: BTreeMap<Option<i32>, Vec<i32>> = BTreeMap::new();

    for episode in episodes {
        if episode
            .location_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("Virtual"))
        {
            continue;
        }
        count += 1;
        if let Some(date) = episode
            .premiere_date
            .as_deref()
            .map(|value| value.chars().take(10).collect::<String>())
            .filter(|value| !value.is_empty())
            && latest_date
                .as_deref()
                .is_none_or(|current| date.as_str() > current)
        {
            latest_date = Some(date);
        }
        if let Some(number) = episode.index_number {
            have_by_season
                .entry(episode.parent_index_number)
                .or_default()
                .push(number);
            let pair = (episode.parent_index_number, number);
            if latest_pair
                .as_ref()
                .is_none_or(|current| compare_episode_pair(&pair, current).is_gt())
            {
                latest_pair = Some(pair);
            }
        }
    }
    for values in have_by_season.values_mut() {
        values.sort_unstable();
        values.dedup();
    }
    LocalEpisodeSummary {
        count,
        latest_date,
        latest_episode: latest_pair
            .and_then(|(season, number)| episode_label(season, Some(number))),
        have_by_season,
    }
}

fn compare_episode_pair(
    left: &(Option<i32>, i32),
    right: &(Option<i32>, i32),
) -> std::cmp::Ordering {
    left.0
        .unwrap_or(0)
        .cmp(&right.0.unwrap_or(0))
        .then_with(|| left.1.cmp(&right.1))
}

fn tmdb_state(status: &str, next_episode: Option<&TmdbEpisodeSummary>) -> (String, bool, bool) {
    let normalized = status.trim().to_ascii_lowercase();
    let ended = matches!(normalized.as_str(), "ended" | "canceled" | "cancelled");
    let continuing = !ended
        && (matches!(
            normalized.as_str(),
            "returning series" | "continuing" | "in production" | "pilot" | "planned"
        ) || next_episode.is_some());
    let state = if ended {
        "ended"
    } else if continuing {
        "continuing"
    } else {
        "unknown"
    };
    (state.to_string(), continuing, ended)
}

fn build_copy_text(items: &[ZhuigengItem], continuing_only: bool) -> String {
    items
        .iter()
        .filter(|item| item.behind > 0)
        .filter(|item| !continuing_only || item.continuing)
        .filter_map(|item| item.resource_hint.as_ref().map(|hint| (item, hint)))
        .map(|(item, hint)| {
            if item.tmdb.trim().is_empty() {
                format!("求 {} — {}", item.name, hint)
            } else {
                format!("求 {} [tmdb:{}] — {}", item.name, item.tmdb, hint)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn folder_from_series_path(path: Option<&str>, library: &ZhuigengLibrary) -> String {
    let Some(path) = path else {
        return String::new();
    };
    let library_folder = library
        .paths
        .iter()
        .filter_map(|path| top_folder_name(path))
        .next()
        .unwrap_or_else(|| library.name.clone());
    let sep = format!("/{library_folder}/");
    if let Some((_, rest)) = path.split_once(&sep) {
        return rest.split('/').next().unwrap_or_default().to_string();
    }
    for root in &library.paths {
        let root = root.trim_end_matches('/');
        if let Some(rest) = path.strip_prefix(root)
            && let Some(rest) = rest.strip_prefix('/')
        {
            return rest.split('/').next().unwrap_or_default().to_string();
        }
    }
    String::new()
}

fn top_folder_name(path: &str) -> Option<String> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn provider_id(provider_ids: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    provider_ids
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .and_then(|(_, value)| match value {
            Value::String(raw) => {
                let raw = raw.trim();
                (!raw.is_empty()).then(|| raw.to_string())
            }
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(value) => Some(value.to_string()),
            _ => None,
        })
}

fn validate_tmdb_id(tmdb: &str) -> AppResult<()> {
    let value = tmdb.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(
            "ProviderIds.Tmdb 为空，无法查询 TMDb".to_string(),
        ));
    }
    if !value.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(AppError::BadRequest(format!(
            "ProviderIds.Tmdb 必须是数字: {tmdb:?}"
        )));
    }
    Ok(())
}

fn episode_label(season: Option<i32>, episode: Option<i32>) -> Option<String> {
    let episode = episode?;
    Some(match season {
        Some(season) => format!("S{:02}E{:02}", season, episode),
        None => format!("E{episode:02}"),
    })
}

fn compact_ints(values: &[i32]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let mut values = values.to_vec();
    values.sort_unstable();
    values.dedup();
    let mut out = Vec::new();
    let mut start = values[0];
    let mut prev = values[0];
    for value in values.into_iter().skip(1) {
        if value == prev + 1 {
            prev = value;
            continue;
        }
        out.push(compact_range(start, prev));
        start = value;
        prev = value;
    }
    out.push(compact_range(start, prev));
    out.join(",")
}

fn compact_range(start: i32, end: i32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

fn push_path(paths: &mut Vec<String>, path: String) {
    let path = path.trim();
    if !path.is_empty() && !paths.iter().any(|existing| existing == path) {
        paths.push(path.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::{compact_ints, tmdb_state};

    #[test]
    fn compacts_episode_ranges() {
        assert_eq!(compact_ints(&[1, 2, 3, 7, 9, 10]), "1-3,7,9-10");
    }

    #[test]
    fn maps_tmdb_states() {
        assert_eq!(
            tmdb_state("Ended", None),
            ("ended".to_string(), false, true)
        );
        assert_eq!(
            tmdb_state("Returning Series", None),
            ("continuing".to_string(), true, false)
        );
    }
}
