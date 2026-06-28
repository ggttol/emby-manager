use crate::{
    config_store,
    emby::{EmbyClient, EmbyItem, EmbyLibrary},
    error::{AppError, AppResult},
    state::AppState,
};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};

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

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v2/posters/detect-mismatch", post(detect_mismatch))
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
