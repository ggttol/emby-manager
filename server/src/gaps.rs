use crate::{
    config_store,
    emby::{EmbyClient, EmbyEpisode},
    error::{AppError, AppResult},
    state::AppState,
    tasks::{self, TaskRun},
};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";
const TASK_CANCELLED_SENTINEL: &str = "__task_cancelled__";

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct GapsScanLibRequest {
    pub lib: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GapsScanLibResult {
    pub ok: bool,
    pub lib: String,
    pub items: Vec<GapsScanRow>,
    pub total: usize,
    pub analyzed: usize,
    pub copy_text: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GapsScanRow {
    pub name: String,
    pub id: Option<String>,
    pub tmdb: String,
    pub fmt: Option<String>,
    pub gap_count: usize,
    pub behind: i32,
    pub score: i32,
    pub err: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SeriesGaps {
    pub mode: String,
    pub have: usize,
    pub gaps: usize,
    pub max_ep: i32,
    pub tmdb_max: i32,
    pub noidx: usize,
    pub gap_list: Vec<String>,
    pub seasons: Vec<SeasonGaps>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SeasonGaps {
    pub season: Option<i32>,
    pub count: usize,
    pub lo: i32,
    pub hi: i32,
    pub gaps: Vec<String>,
    pub gapcount: usize,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v2/gaps/scan-lib", post(scan_library_gaps))
}

#[utoipa::path(post, path = "/api/v2/gaps/scan-lib", tag = "gaps", request_body = GapsScanLibRequest, responses((status = 200, body = TaskRun)))]
pub async fn scan_library_gaps(
    State(state): State<AppState>,
    Json(req): Json<GapsScanLibRequest>,
) -> AppResult<Json<TaskRun>> {
    let requested_lib = req.lib.trim();
    if requested_lib.is_empty() {
        return Err(AppError::BadRequest("lib is required".to_string()));
    }

    let emby_url = config_store::get_string_or(&state.pool, "emby_url", DEFAULT_EMBY_URL).await?;
    let api_key = config_store::get_string_or(&state.pool, "api_key", "").await?;
    if api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "api_key 未配置，无法连接 Emby".to_string(),
        ));
    }

    let client = EmbyClient::new(emby_url, api_key, state.http.clone());
    let libraries = client.libraries().await?;
    let library = libraries
        .into_iter()
        .find(|library| {
            library.name == requested_lib
                || library.id.as_deref().is_some_and(|id| id == requested_lib)
        })
        .ok_or_else(|| AppError::BadRequest(format!("未知库 {requested_lib}")))?;
    if library.library_type != "tvshows" {
        return Err(AppError::BadRequest(format!(
            "只能扫剧集库(tvshows)，当前: {}",
            library.library_type
        )));
    }
    let Some(library_id) = library.id.clone() else {
        return Err(AppError::BadRequest(format!(
            "剧集库 {} 缺少 Emby ItemId",
            library.name
        )));
    };

    let task = tasks::insert_task_with_meta(
        &state.pool,
        "gaps_scan_lib",
        &format!("全库缺集扫描 {}", library.name),
        1,
        "manual",
        serde_json::json!({
            "lib": library.name,
            "library_id": library_id,
        }),
    )
    .await?;
    spawn_gaps_scan(state, task.id, client, library.name, library_id);
    Ok(Json(task))
}

fn spawn_gaps_scan(
    state: AppState,
    task_id: Uuid,
    client: EmbyClient,
    lib: String,
    library_id: String,
) {
    tokio::spawn(async move {
        let Ok(_permit) = state.task_slots.clone().acquire_owned().await else {
            let _ = tasks::finish_error(&state.pool, task_id, "任务并发槽不可用", None).await;
            return;
        };

        let _ = tasks::mark_running(&state.pool, task_id, "准备缺集扫描").await;
        match run_gaps_scan(&state, task_id, &client, &lib, &library_id).await {
            Ok(result) => {
                let _ = tasks::finish_done_with_message(
                    &state.pool,
                    task_id,
                    "全库缺集扫描完成",
                    serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
                )
                .await;
            }
            Err(err) if err.to_string() == TASK_CANCELLED_SENTINEL => {}
            Err(err) => {
                let _ = tasks::finish_error(&state.pool, task_id, &err.to_string(), None).await;
            }
        }
    });
}

async fn run_gaps_scan(
    state: &AppState,
    task_id: Uuid,
    client: &EmbyClient,
    lib: &str,
    library_id: &str,
) -> AppResult<GapsScanLibResult> {
    let series = client.series(library_id, 10_000).await?;
    let total = series.len();
    tasks::set_total(&state.pool, task_id, total.max(1) as i64).await?;
    tasks::set_progress(&state.pool, task_id, 0, &format!("扫 {lib}")).await?;

    let mut rows = Vec::new();
    for (idx, item) in series.iter().enumerate() {
        if tasks::cancel_requested(&state.pool, task_id).await {
            tasks::finish_cancelled(&state.pool, task_id).await?;
            return Err(AppError::Conflict(TASK_CANCELLED_SENTINEL.to_string()));
        }
        let name = item.name.clone().unwrap_or_else(|| "?".to_string());
        if idx % 5 == 0 {
            tasks::set_progress(
                &state.pool,
                task_id,
                idx as i64,
                &format!("查 {lib} ({idx}/{total})"),
            )
            .await?;
        }

        let row = match item.id.as_deref() {
            Some(series_id) => match client.episodes(series_id).await {
                Ok(episodes) => build_scan_row(
                    &name,
                    Some(series_id.to_string()),
                    item.provider_id("Tmdb"),
                    &episodes,
                ),
                Err(err) => GapsScanRow {
                    name,
                    id: Some(series_id.to_string()),
                    tmdb: item.provider_id("Tmdb").unwrap_or_default(),
                    fmt: None,
                    gap_count: 0,
                    behind: 0,
                    score: 0,
                    err: Some(err.to_string()),
                },
            },
            None => GapsScanRow {
                name,
                id: None,
                tmdb: item.provider_id("Tmdb").unwrap_or_default(),
                fmt: None,
                gap_count: 0,
                behind: 0,
                score: 0,
                err: Some("series item missing Id".to_string()),
            },
        };
        if row.fmt.is_some() || row.err.is_some() {
            rows.push(row);
        }
    }

    rows.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.name.cmp(&right.name))
    });
    let copy_text = rows
        .iter()
        .filter_map(|row| row.fmt.as_ref().map(|fmt| (row, fmt)))
        .map(|(row, fmt)| {
            if row.tmdb.trim().is_empty() {
                format!("求 {} — {}", row.name, fmt)
            } else {
                format!("求 {} [tmdb:{}] — {}", row.name, row.tmdb, fmt)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    Ok(GapsScanLibResult {
        ok: true,
        lib: lib.to_string(),
        total: rows.len(),
        analyzed: total,
        items: rows,
        copy_text,
    })
}

fn build_scan_row(
    name: &str,
    id: Option<String>,
    tmdb: Option<String>,
    episodes: &[EmbyEpisode],
) -> GapsScanRow {
    let gaps = series_gaps(episodes);
    let mut gap_count = 0usize;
    let behind: i32;
    let mut fmt = String::new();

    if gaps.mode == "absolute" {
        gap_count = gaps.gap_list.len();
        behind = (gaps.tmdb_max - gaps.max_ep).max(0);
        if !gaps.gap_list.is_empty() {
            fmt = format!("缺 E{}", gaps.gap_list.join(","));
        }
        if behind > 0 {
            if !fmt.is_empty() {
                fmt.push_str(" · ");
            }
            fmt.push_str(&format!("落后到 E{} (本地 {})", gaps.tmdb_max, gaps.max_ep));
        }
    } else {
        let mut segments = Vec::new();
        for season in &gaps.seasons {
            if season.gapcount > 0 {
                segments.push(format!(
                    "S{:02} E{}",
                    season.season.unwrap_or(0),
                    season.gaps.join(",")
                ));
                gap_count += season.gapcount;
            }
        }
        behind = (gaps.tmdb_max - gaps.max_ep).max(0);
        fmt = segments.join(" · ");
        if behind > 0 {
            if !fmt.is_empty() {
                fmt.push_str(" · ");
            }
            fmt.push_str(&format!("落后 TMDb {behind} 集"));
        }
    }

    let fmt = (!fmt.is_empty()).then_some(fmt);
    GapsScanRow {
        name: name.to_string(),
        id,
        tmdb: tmdb.unwrap_or_default(),
        fmt,
        gap_count,
        behind,
        score: gap_count as i32 + behind * 2,
        err: None,
    }
}

pub fn series_gaps(episodes: &[EmbyEpisode]) -> SeriesGaps {
    let mut have: BTreeMap<Option<i32>, BTreeSet<i32>> = BTreeMap::new();
    let mut virt: BTreeMap<Option<i32>, BTreeSet<i32>> = BTreeMap::new();
    let mut noidx = 0usize;

    for episode in episodes {
        let Some(number) = episode.index_number else {
            noidx += 1;
            continue;
        };
        let target = if episode
            .location_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("Virtual"))
        {
            &mut virt
        } else {
            &mut have
        };
        target
            .entry(episode.parent_index_number)
            .or_default()
            .insert(number);
    }

    let virt_all = virt
        .values()
        .flat_map(|numbers| numbers.iter().copied())
        .collect::<BTreeSet<_>>();
    let positive_have = have
        .iter()
        .filter_map(|(season, numbers)| season.filter(|season| *season > 0).map(|_| numbers))
        .collect::<Vec<_>>();
    let total_positive = positive_have
        .iter()
        .map(|numbers| numbers.len())
        .sum::<usize>();
    let union_positive = positive_have
        .iter()
        .flat_map(|numbers| numbers.iter().copied())
        .collect::<BTreeSet<_>>();
    let absolute = union_positive.iter().next_back().is_some_and(|max| {
        !union_positive.is_empty() && total_positive == union_positive.len() && *max > 50
    });

    if absolute {
        let hi = union_positive
            .union(&virt_all)
            .copied()
            .max()
            .unwrap_or_default();
        let lo = union_positive.iter().next().copied().unwrap_or_default();
        let missing = (lo..=hi)
            .filter(|number| !union_positive.contains(number))
            .collect::<Vec<_>>();
        return SeriesGaps {
            mode: "absolute".to_string(),
            have: union_positive.len(),
            max_ep: union_positive
                .iter()
                .next_back()
                .copied()
                .unwrap_or_default(),
            tmdb_max: hi,
            gaps: missing.len(),
            gap_list: compact_ints(&missing),
            noidx,
            seasons: Vec::new(),
        };
    }

    let mut keys = have.keys().chain(virt.keys()).copied().collect::<Vec<_>>();
    keys.sort_by(|left, right| match (left, right) {
        (Some(left), Some(right)) => left.cmp(right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    keys.dedup();

    let mut seasons = Vec::new();
    let mut total_have = 0usize;
    let mut total_gaps = 0usize;
    let mut max_ep = 0i32;
    let mut tmdb_max = 0i32;

    for season in keys {
        let have_numbers = have.get(&season).cloned().unwrap_or_default();
        let virt_numbers = virt.get(&season).cloned().unwrap_or_default();
        let full = have_numbers
            .union(&virt_numbers)
            .copied()
            .collect::<BTreeSet<_>>();
        let Some(lo) = full.iter().next().copied() else {
            continue;
        };
        let hi = full.iter().next_back().copied().unwrap_or(lo);
        let missing = (lo..=hi)
            .filter(|number| !have_numbers.contains(number))
            .collect::<Vec<_>>();
        let count = have_numbers.len();
        let gapcount = missing.len();
        total_have += count;
        total_gaps += gapcount;
        if let Some(season_max) = have_numbers.iter().next_back() {
            max_ep = max_ep.max(*season_max);
        }
        tmdb_max = tmdb_max.max(hi);
        seasons.push(SeasonGaps {
            season,
            count,
            lo,
            hi,
            gaps: compact_ints(&missing),
            gapcount,
        });
    }

    SeriesGaps {
        mode: "season".to_string(),
        have: total_have,
        gaps: total_gaps,
        max_ep,
        tmdb_max,
        noidx,
        gap_list: Vec::new(),
        seasons,
    }
}

fn compact_ints(values: &[i32]) -> Vec<String> {
    if values.is_empty() {
        return Vec::new();
    }
    let mut xs = values.to_vec();
    xs.sort_unstable();
    xs.dedup();
    let mut out = Vec::new();
    let mut start = xs[0];
    let mut prev = xs[0];
    for &value in &xs[1..] {
        if value == prev + 1 {
            prev = value;
            continue;
        }
        out.push(format_range(start, prev));
        start = value;
        prev = value;
    }
    out.push(format_range(start, prev));
    out
}

fn format_range(start: i32, end: i32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(season: Option<i32>, number: Option<i32>, virtual_item: bool) -> EmbyEpisode {
        EmbyEpisode {
            id: None,
            name: None,
            parent_index_number: season,
            index_number: number,
            location_type: virtual_item.then(|| "Virtual".to_string()),
        }
    }

    #[test]
    fn detects_season_gaps_and_virtual_tail() {
        let gaps = series_gaps(&[
            episode(Some(1), Some(1), false),
            episode(Some(1), Some(3), false),
            episode(Some(1), Some(4), true),
            episode(Some(2), Some(1), false),
            episode(Some(2), Some(2), true),
            episode(Some(2), None, false),
        ]);

        assert_eq!(gaps.mode, "season");
        assert_eq!(gaps.have, 3);
        assert_eq!(gaps.gaps, 3);
        assert_eq!(gaps.max_ep, 3);
        assert_eq!(gaps.tmdb_max, 4);
        assert_eq!(gaps.noidx, 1);
        assert_eq!(gaps.seasons[0].gaps, vec!["2", "4"]);
        assert_eq!(gaps.seasons[1].gaps, vec!["2"]);
    }

    #[test]
    fn detects_absolute_numbering() {
        let mut episodes = (1..=60)
            .filter(|number| !matches!(number, 10 | 11 | 55))
            .map(|number| episode(Some(1), Some(number), false))
            .collect::<Vec<_>>();
        episodes.push(episode(Some(1), Some(61), true));

        let gaps = series_gaps(&episodes);

        assert_eq!(gaps.mode, "absolute");
        assert_eq!(gaps.have, 57);
        assert_eq!(gaps.max_ep, 60);
        assert_eq!(gaps.tmdb_max, 61);
        assert_eq!(gaps.gap_list, vec!["10-11", "55", "61"]);
    }
}
