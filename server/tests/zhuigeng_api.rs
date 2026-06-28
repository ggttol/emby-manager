use emby_manager::{
    scheduler,
    settings::Settings,
    state::AppState,
    zhuigeng::{
        ZhuigengConfig, ZhuigengScanAiringResponse, ZhuigengScanAiringRow, execute_scan_airing_row,
        zhuigeng_gaps_summary_with_config, zhuigeng_scan_airing_with_config,
        zhuigeng_status_with_config,
    },
};
use sqlx::postgres::PgPoolOptions;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tempfile::tempdir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::{Duration, sleep, timeout},
};

#[tokio::test]
async fn status_reads_emby_series_and_tmdb_airing_semantics() {
    let libraries = r#"[
        {
            "ItemId": "lib-airing",
            "Name": "电视剧追更",
            "CollectionType": "tvshows",
            "Locations": ["/strm/电视剧追更"]
        },
        {
            "ItemId": "lib-archive",
            "Name": "电视剧完结",
            "CollectionType": "tvshows",
            "Locations": ["/strm/电视剧完结"]
        }
    ]"#;
    let series = r#"{
        "Items": [
            {
                "Id": "series-a",
                "Name": "示例剧",
                "Path": "/strm/电视剧追更/示例剧 [tmdbid-100]/show.strm",
                "ProviderIds": {"Tmdb": "100"}
            },
            {
                "Id": "series-b",
                "Name": "完结剧",
                "Path": "/strm/电视剧追更/完结剧 [tmdbid-200]/show.strm",
                "ProviderIds": {"Tmdb": 200}
            }
        ]
    }"#;
    let episodes_a = r#"{
        "Items": [
            {"ParentIndexNumber": 1, "IndexNumber": 1, "PremiereDate": "2026-06-01T00:00:00Z", "LocationType": "FileSystem"},
            {"ParentIndexNumber": 1, "IndexNumber": 2, "PremiereDate": "2026-06-08T00:00:00Z", "LocationType": "FileSystem"},
            {"ParentIndexNumber": 1, "IndexNumber": 3, "PremiereDate": "2026-06-15T00:00:00Z", "LocationType": "Virtual"}
        ]
    }"#;
    let episodes_b = r#"{
        "Items": [
            {"ParentIndexNumber": 1, "IndexNumber": 1, "PremiereDate": "2025-01-01T00:00:00Z", "LocationType": "FileSystem"},
            {"ParentIndexNumber": 1, "IndexNumber": 2, "PremiereDate": "2025-01-08T00:00:00Z", "LocationType": "FileSystem"}
        ]
    }"#;
    let tmdb_a = r#"{
        "status": "Returning Series",
        "last_episode_to_air": {"season_number": 1, "episode_number": 3, "air_date": "2026-06-15", "name": "第三集"},
        "next_episode_to_air": {"season_number": 1, "episode_number": 4, "air_date": "2026-06-22", "name": "第四集"}
    }"#;
    let tmdb_b = r#"{
        "status": "Ended",
        "last_episode_to_air": {"season_number": 1, "episode_number": 2, "air_date": "2025-01-08", "name": "终章"},
        "next_episode_to_air": null
    }"#;
    let (emby_base, emby_requests, emby_handle) =
        spawn_fake_sequence(vec![libraries, series, episodes_a, episodes_b]).await;
    let (tmdb_base, tmdb_requests, tmdb_handle) = spawn_fake_sequence(vec![tmdb_a, tmdb_b]).await;

    let response =
        zhuigeng_status_with_config(fake_config(&emby_base, &tmdb_base), reqwest::Client::new())
            .await
            .unwrap();

    assert!(response.ok);
    assert_eq!(response.total, 2);
    assert_eq!(response.continuing, 1);
    assert_eq!(response.ended, 1);
    assert_eq!(response.copy_text, "求 示例剧 [tmdb:100] — S01 E3");
    let airing = response
        .items
        .iter()
        .find(|item| item.name == "示例剧")
        .unwrap();
    assert!(airing.continuing);
    assert!(!airing.ended);
    assert_eq!(airing.state, "continuing");
    assert_eq!(airing.folder, "示例剧 [tmdbid-100]");
    assert_eq!(airing.local_count, 2);
    assert_eq!(airing.local_latest.as_deref(), Some("2026-06-08"));
    assert_eq!(airing.local_latest_episode.as_deref(), Some("S01E02"));
    assert_eq!(airing.behind, 1);
    assert_eq!(airing.resource_hint.as_deref(), Some("S01 E3"));
    assert!(
        airing
            .behind_hint
            .as_deref()
            .is_some_and(|hint| hint.contains("落后 TMDb 1 集")),
        "{airing:?}"
    );
    assert_eq!(
        airing
            .last_episode_to_air
            .as_ref()
            .and_then(|episode| episode.episode_number),
        Some(3)
    );
    assert_eq!(
        airing
            .next_episode_to_air
            .as_ref()
            .and_then(|episode| episode.episode_number),
        Some(4)
    );
    let ended = response
        .items
        .iter()
        .find(|item| item.name == "完结剧")
        .unwrap();
    assert!(ended.ended);
    assert_eq!(ended.state, "ended");
    assert_eq!(ended.behind, 0);

    timeout(Duration::from_secs(1), emby_handle)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(1), tmdb_handle)
        .await
        .unwrap()
        .unwrap();
    let emby_requests = emby_requests.lock().unwrap();
    assert_eq!(emby_requests.len(), 4);
    assert!(
        emby_requests[0].starts_with("GET /Library/VirtualFolders?api_key=emby-key "),
        "{}",
        emby_requests[0]
    );
    assert!(
        emby_requests[1].contains("ParentId=lib-airing"),
        "{}",
        emby_requests[1]
    );
    assert!(
        emby_requests[1].contains("Fields=Status%2CPath%2CProviderIds")
            || emby_requests[1].contains("Fields=Status,Path,ProviderIds"),
        "{}",
        emby_requests[1]
    );
    assert!(
        emby_requests[2].starts_with("GET /Shows/series-a/Episodes?"),
        "{}",
        emby_requests[2]
    );
    let tmdb_requests = tmdb_requests.lock().unwrap();
    assert_eq!(tmdb_requests.len(), 2);
    assert!(
        tmdb_requests[0].starts_with("GET /3/tv/100?api_key=tmdb-key "),
        "{}",
        tmdb_requests[0]
    );
    assert!(
        tmdb_requests[1].starts_with("GET /3/tv/200?api_key=tmdb-key "),
        "{}",
        tmdb_requests[1]
    );
}

#[tokio::test]
async fn scan_airing_and_gaps_summary_use_tmdb_behind_rows() {
    let libraries = r#"[{"ItemId":"lib-airing","Name":"追更库","CollectionType":"tvshows","Locations":["/strm/追更库"]}]"#;
    let series = r#"{"Items":[{"Id":"series-a","Name":"缺一集","Path":"/strm/追更库/缺一集/show.strm","ProviderIds":{"Tmdb":"300"}}]}"#;
    let episodes =
        r#"{"Items":[{"ParentIndexNumber":1,"IndexNumber":1,"LocationType":"FileSystem"}]}"#;
    let tmdb = r#"{
        "status": "Returning Series",
        "last_episode_to_air": {"season_number": 1, "episode_number": 2, "air_date": "2026-06-20"},
        "next_episode_to_air": null
    }"#;

    let (emby_base, _, emby_handle) = spawn_fake_sequence(vec![libraries, series, episodes]).await;
    let (tmdb_base, _, tmdb_handle) = spawn_fake_sequence(vec![tmdb]).await;
    let scan = zhuigeng_scan_airing_with_config(
        fake_config(&emby_base, &tmdb_base),
        reqwest::Client::new(),
    )
    .await
    .unwrap();
    assert!(scan.ok);
    assert_eq!(scan.total, 1);
    assert_eq!(scan.results[0].name, "缺一集");
    assert_eq!(scan.results[0].lib, "追更库");
    assert_eq!(scan.results[0].keyword, "缺一集");
    assert_eq!(scan.results[0].status, "planned");
    assert_eq!(scan.results[0].new_count, 0);
    assert_eq!(scan.results[0].behind, 1);
    assert_eq!(scan.copy_text, "求 缺一集 [tmdb:300] — S01 E2");
    assert!(scan.note.contains("执行计划"));
    timeout(Duration::from_secs(1), emby_handle)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(1), tmdb_handle)
        .await
        .unwrap()
        .unwrap();

    let (emby_base, _, emby_handle) = spawn_fake_sequence(vec![libraries, series, episodes]).await;
    let (tmdb_base, _, tmdb_handle) = spawn_fake_sequence(vec![tmdb]).await;
    let gaps = zhuigeng_gaps_summary_with_config(
        fake_config(&emby_base, &tmdb_base),
        reqwest::Client::new(),
    )
    .await
    .unwrap();
    assert!(gaps.ok);
    assert_eq!(gaps.total, 1);
    assert_eq!(gaps.items[0].fmt, "S01 E2");
    assert_eq!(gaps.copy_text, "求 缺一集 [tmdb:300] — S01 E2");
    timeout(Duration::from_secs(1), emby_handle)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(1), tmdb_handle)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn scan_airing_filters_ended_series_from_execution_plan() {
    let libraries = r#"[{"ItemId":"lib-airing","Name":"追更库","CollectionType":"tvshows","Locations":["/strm/追更库"]}]"#;
    let series = r#"{"Items":[
        {"Id":"series-a","Name":"在更剧","Path":"/strm/追更库/在更剧/show.strm","ProviderIds":{"Tmdb":"301"}},
        {"Id":"series-b","Name":"完结剧","Path":"/strm/追更库/完结剧/show.strm","ProviderIds":{"Tmdb":"302"}}
    ]}"#;
    let episodes = r#"{"Items":[]}"#;
    let tmdb_airing = r#"{"status":"Returning Series","last_episode_to_air":null,"next_episode_to_air":{"season_number":1,"episode_number":2}}"#;
    let tmdb_ended = r#"{"status":"Ended","last_episode_to_air":{"season_number":1,"episode_number":1},"next_episode_to_air":null}"#;
    let (emby_base, _, emby_handle) =
        spawn_fake_sequence(vec![libraries, series, episodes, episodes]).await;
    let (tmdb_base, _, tmdb_handle) = spawn_fake_sequence(vec![tmdb_airing, tmdb_ended]).await;

    let scan = zhuigeng_scan_airing_with_config(
        fake_config(&emby_base, &tmdb_base),
        reqwest::Client::new(),
    )
    .await
    .unwrap();

    assert_eq!(scan.total, 1);
    assert_eq!(scan.results[0].name, "在更剧");
    assert!(scan.results.iter().all(|row| row.name != "完结剧"));
    timeout(Duration::from_secs(1), emby_handle)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(1), tmdb_handle)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn scan_airing_row_executes_strm_generation_with_series_keyword_and_degrades_errors() {
    let tmp = tempdir().unwrap();
    let cd_root = tmp.path().join("cd");
    let strm_root = tmp.path().join("strm");
    std::fs::create_dir_all(cd_root.join("追更库/缺一集/Season 1")).unwrap();
    std::fs::create_dir_all(strm_root.join("追更库")).unwrap();
    std::fs::write(cd_root.join("追更库/缺一集/Season 1/E02.mkv"), "video").unwrap();

    let state = test_state(cd_root, strm_root.clone());
    let mut row = scan_row("追更库", "缺一集");
    execute_scan_airing_row(&state, &mut row);

    assert!(row.ok, "{row:?}");
    assert_eq!(row.status, "generated");
    assert_eq!(row.keyword, "缺一集");
    assert_eq!(row.matched, 1);
    assert_eq!(row.new_count, 1);
    assert!(strm_root.join("追更库/缺一集/Season 1/E02.strm").is_file());

    let mut missing = scan_row("不存在", "缺一集");
    execute_scan_airing_row(&state, &mut missing);
    assert!(!missing.ok);
    assert_eq!(missing.status, "error");
    assert!(
        missing
            .error
            .as_deref()
            .is_some_and(|err| err.contains("115 文件夹不存在")),
        "{missing:?}"
    );
}

#[test]
fn scheduler_scan_airing_detail_reports_real_execution_not_dry_run_preview() {
    let detail = scheduler::zhuigeng_scan_airing_schedule_detail(ZhuigengScanAiringResponse {
        ok: true,
        total: 1,
        ok_count: 1,
        error_count: 0,
        new_count: 2,
        results: vec![scan_row("追更库", "缺一集")],
        copy_text: "求 缺一集".to_string(),
        note: "已按 TMDb continuing 剧集串行用剧名扫描对应库并生成缺失 STRM".to_string(),
    });

    assert_eq!(detail["action"], "zhuigeng_scan_airing");
    assert_eq!(detail["new_count"], 2);
    assert!(
        detail["message"]
            .as_str()
            .is_some_and(|message| message.contains("剧名串行扫描")),
        "{detail}"
    );
    assert_ne!(detail["action"], "refresh_airing_libraries");
    assert!(detail.get("dry_run").is_none(), "{detail}");
}

#[tokio::test]
async fn missing_tmdb_config_reports_clear_bad_request() {
    let err = zhuigeng_status_with_config(
        ZhuigengConfig::new("http://127.0.0.1:1", "emby-key", "", "tmdb-key"),
        reqwest::Client::new(),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(err.contains("tmdb_base_url/tmdb_url 未配置"), "{err}");
}

#[tokio::test]
async fn tmdb_timeout_is_reported_on_the_series_row() {
    let libraries = r#"[{"ItemId":"lib-airing","Name":"追更库","CollectionType":"tvshows","Locations":["/strm/追更库"]}]"#;
    let series = r#"{"Items":[{"Id":"series-a","Name":"慢剧","Path":"/strm/追更库/慢剧/show.strm","ProviderIds":{"Tmdb":"400"}}]}"#;
    let episodes =
        r#"{"Items":[{"ParentIndexNumber":1,"IndexNumber":1,"LocationType":"FileSystem"}]}"#;
    let (emby_base, _, emby_handle) = spawn_fake_sequence(vec![libraries, series, episodes]).await;
    let (tmdb_base, tmdb_handle) = spawn_hanging_server().await;

    let response = zhuigeng_status_with_config(
        fake_config(&emby_base, &tmdb_base).with_request_timeout(Duration::from_millis(50)),
        reqwest::Client::new(),
    )
    .await
    .unwrap();

    assert_eq!(response.total, 1);
    let err = response.items[0].err.as_deref().unwrap_or_default();
    assert!(err.contains("TMDb /3/tv/400 请求超时"), "{err}");
    timeout(Duration::from_secs(1), emby_handle)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(1), tmdb_handle)
        .await
        .unwrap()
        .unwrap();
}

fn fake_config(emby_base: &str, tmdb_base: &str) -> ZhuigengConfig {
    ZhuigengConfig::new(emby_base, "emby-key", tmdb_base, "tmdb-key")
}

fn scan_row(lib: &str, name: &str) -> ZhuigengScanAiringRow {
    ZhuigengScanAiringRow {
        lib: lib.to_string(),
        series: Some("series-a".to_string()),
        name: name.to_string(),
        id: Some("series-a".to_string()),
        tmdb: "300".to_string(),
        tmdb_status: "Returning Series".to_string(),
        keyword: name.to_string(),
        status: "planned".to_string(),
        behind: 1,
        hint: Some("落后 TMDb 1 集".to_string()),
        ok: true,
        matched: 0,
        new_count: 0,
        attention: Vec::new(),
        error: None,
        err: None,
    }
}

fn test_state(cd_root: PathBuf, strm_root: PathBuf) -> AppState {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://emby_manager:emby_manager@127.0.0.1/emby_manager_test")
        .unwrap();
    AppState::new(
        pool,
        Settings {
            host: "127.0.0.1".to_string(),
            port: 0,
            database_url: "postgres://emby_manager:emby_manager@127.0.0.1/emby_manager_test"
                .to_string(),
            web_dist: PathBuf::from("/tmp"),
            legacy_dir: PathBuf::from("/tmp"),
            bootstrap_password: "admin".to_string(),
            cd_root,
            strm_root,
            docker_bin: PathBuf::from("/usr/bin/docker"),
            task_concurrency: 1,
        },
    )
}

async fn spawn_fake_sequence(
    bodies: Vec<&'static str>,
) -> (String, Arc<Mutex<Vec<String>>>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);

    let handle = tokio::spawn(async move {
        for body in bodies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            captured.lock().unwrap().push(request);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });

    (format!("http://{addr}"), requests, handle)
}

async fn spawn_hanging_server() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut socket).await;
        sleep(Duration::from_millis(200)).await;
    });
    (format!("http://{addr}"), handle)
}

async fn read_http_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0; 1024];
    loop {
        let n = socket.read(&mut tmp).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if find_header_end(&buf).is_some() {
            break;
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}
