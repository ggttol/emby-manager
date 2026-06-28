use axum::extract::{Query, State};
use emby_manager::{
    media_fs::{self, StrmListQuery},
    settings::Settings,
    state::AppState,
    system,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn system_summary_reports_database_and_path_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    let strm_root = tmp.path().join("strm");
    std::fs::create_dir_all(&strm_root).unwrap();

    let state = state_with_roots(
        tmp.path(),
        tmp.path().join("missing-cd-root"),
        strm_root,
        tmp.path().join("missing-docker"),
    );

    let summary = system::system_summary(State(state)).await.unwrap().0;

    assert!(!summary.ok);
    assert_eq!(summary.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(summary.rust_version, env!("CARGO_PKG_VERSION"));
    assert!(
        matches!(summary.database.status.as_str(), "timeout" | "unavailable"),
        "{}",
        summary.database.status
    );
    assert!(!summary.database.url.contains("secret"));
    assert!(
        summary
            .configured_roots
            .iter()
            .any(|root| root.key == "strm_root" && root.exists && root.is_dir)
    );
    assert!(
        summary
            .warnings
            .iter()
            .any(|warning| warning.contains("CloudDrive 媒体根") && warning.contains("不存在"))
    );
    assert!(
        summary
            .warnings
            .iter()
            .any(|warning| warning.contains("Docker CLI") && warning.contains("不存在"))
    );
    assert_eq!(summary.docker.status, "unavailable");
    assert!(summary.docker.containers.is_empty());
    assert!(
        summary
            .docker
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("Docker 容器列表不可用"))
    );
    assert_eq!(summary.emby.status, "config_unavailable");
    assert!(
        summary
            .emby
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("Emby 配置读取失败"))
    );
    assert!(
        summary
            .warnings
            .iter()
            .any(|warning| warning.contains("数据库检查失败"))
    );
}

#[test]
fn docker_ps_parser_reads_json_lines() {
    let output = r#"
{"ID":"abc123def456","Image":"emby/embyserver:latest","Names":"emby","State":"running","Status":"Up 2 hours","Ports":"0.0.0.0:8096->8096/tcp"}
{"ID":"def456abc123","Image":"postgres:16","Names":"postgres","State":"exited","Status":"Exited (0) 3 minutes ago","Ports":""}
"#;

    let containers = system::parse_docker_ps_output(output).unwrap();

    assert_eq!(containers.len(), 2);
    assert_eq!(containers[0].id, "abc123def456");
    assert_eq!(containers[0].name, "emby");
    assert_eq!(containers[0].image, "emby/embyserver:latest");
    assert_eq!(containers[0].state, "running");
    assert!(containers[0].ports.contains("8096"));
    assert_eq!(containers[1].name, "postgres");
    assert_eq!(containers[1].state, "exited");
}

#[test]
fn docker_ps_parser_reports_bad_json_line() {
    let err = system::parse_docker_ps_output("{not-json}").unwrap_err();

    assert!(err.contains("第 1 行 JSON 无法解析"), "{err}");
}

#[tokio::test]
async fn emby_health_reads_system_info_with_readonly_request() {
    let body = r#"{
        "Version": "4.9.5.0",
        "ServerName": "NAS Emby",
        "Id": "server-1",
        "OperatingSystemDisplayName": "Linux"
    }"#;
    let (base_url, requests) = spawn_fake_emby_once(body).await;

    let health = system::probe_emby_health(
        &format!("{base_url}/emby/"),
        "secret-key",
        &reqwest::Client::new(),
    )
    .await;

    assert!(health.configured);
    assert!(health.online);
    assert_eq!(health.status, "ok");
    assert_eq!(health.http_status, Some(200));
    assert_eq!(health.version.as_deref(), Some("4.9.5.0"));
    assert_eq!(health.server_name.as_deref(), Some("NAS Emby"));
    assert_eq!(health.server_id.as_deref(), Some("server-1"));
    assert_eq!(health.operating_system.as_deref(), Some("Linux"));

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("GET /emby/System/Info?api_key=secret-key HTTP/1.1"),
        "{}",
        requests[0]
    );
    assert!(!requests[0].starts_with("POST "), "{}", requests[0]);
    assert!(!requests[0].starts_with("DELETE "), "{}", requests[0]);
}

#[tokio::test]
async fn emby_health_warns_without_api_key() {
    let health =
        system::probe_emby_health("http://127.0.0.1:1/emby", " ", &reqwest::Client::new()).await;

    assert!(!health.configured);
    assert!(!health.online);
    assert_eq!(health.status, "missing_api_key");
    assert!(
        health
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("api_key 未配置"))
    );
}

#[tokio::test]
async fn strm_overview_counts_subtitles_and_samples_without_reading_payloads() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = tmp.path().join("Shows");
    let season = lib.join("Season 1");
    std::fs::create_dir_all(&season).unwrap();
    std::fs::write(season.join("E01.strm"), "http://example/E01.mkv").unwrap();
    std::fs::write(season.join("E01.zh.srt"), "字幕").unwrap();
    std::fs::write(season.join("E01.ass"), "[Script Info]").unwrap();
    std::fs::write(season.join("E02.strm"), "http://example/E02.mkv").unwrap();
    std::fs::write(season.join("poster.jpg"), "noise").unwrap();

    let state = state_with_roots(
        tmp.path(),
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
        tmp.path().join("docker"),
    );
    let response = media_fs::list_strm(
        State(state),
        Query(StrmListQuery {
            lib: Some("Shows".to_string()),
            folder: None,
            limit: Some(20),
            overview: Some(true),
            overview_depth: Some(5),
            sample_limit: Some(10),
        }),
    )
    .await
    .unwrap()
    .0;

    assert!(
        response
            .items
            .iter()
            .any(|item| item.rel_path == "Season 1")
    );
    assert!(
        response
            .items
            .iter()
            .any(|item| item.rel_path == "Season 1/E01.strm")
    );

    let overview = response
        .overview
        .expect("overview=true should produce stats");
    assert_eq!(overview.directories, 1);
    assert_eq!(overview.files, 5);
    assert_eq!(overview.strm_files, 2);
    assert_eq!(overview.subtitle_files, 2);
    assert_eq!(overview.other_files, 1);
    assert_eq!(overview.strm_with_subtitles, 1);
    assert_eq!(overview.strm_without_subtitles, 1);
    assert_eq!(overview.subtitle_coverage_percent, 50.0);
    assert_eq!(
        overview.missing_subtitle_samples,
        vec!["Season 1/E02.strm".to_string()]
    );
    assert_eq!(overview.library_coverage.len(), 1);
    assert_eq!(overview.library_coverage[0].library, "Shows");
    assert_eq!(overview.library_coverage[0].strm_files, 2);
    assert_eq!(overview.library_coverage[0].with_subtitles, 1);
    assert_eq!(overview.library_coverage[0].missing_subtitles, 1);
    assert_eq!(overview.library_coverage[0].coverage_percent, 50.0);
    assert!(!overview.truncated);
    assert!(overview.warnings.is_empty(), "{:?}", overview.warnings);
    assert!(
        overview
            .subtitle_extensions
            .iter()
            .any(|ext| ext.extension == "ass" && ext.count == 1)
    );
    assert!(
        overview
            .subtitle_extensions
            .iter()
            .any(|ext| ext.extension == "srt" && ext.count == 1)
    );
    assert!(
        overview
            .subtitle_languages
            .iter()
            .any(|lang| lang.language == "zh" && lang.count == 1)
    );
    assert!(
        overview
            .subtitle_languages
            .iter()
            .any(|lang| lang.language == "unknown" && lang.count == 1)
    );
    assert!(
        overview
            .samples
            .iter()
            .any(|sample| sample.kind == "strm" && sample.rel_path == "Season 1/E01.strm")
    );
    assert!(
        overview
            .samples
            .iter()
            .any(|sample| sample.kind == "subtitle" && sample.rel_path == "Season 1/E01.zh.srt")
    );
}

#[tokio::test]
async fn list_strm_rejects_traversal_before_any_walk() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_with_roots(
        tmp.path(),
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
        tmp.path().join("docker"),
    );

    let err = media_fs::list_strm(
        State(state),
        Query(StrmListQuery {
            lib: Some("../outside".to_string()),
            folder: None,
            limit: None,
            overview: Some(true),
            overview_depth: None,
            sample_limit: None,
        }),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("非法路径段"), "{err}");
}

fn state_with_roots(
    base: &Path,
    cd_root: PathBuf,
    strm_root: PathBuf,
    docker_bin: PathBuf,
) -> AppState {
    let settings = Settings {
        host: "127.0.0.1".to_string(),
        port: 8098,
        database_url: "postgres://user:secret@127.0.0.1:1/emby".to_string(),
        web_dist: base.to_path_buf(),
        legacy_dir: base.to_path_buf(),
        bootstrap_password: "admin".to_string(),
        cd_root,
        strm_root,
        docker_bin,
        task_concurrency: 1,
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy(&settings.database_url)
        .unwrap();
    AppState::new(pool, settings)
}

async fn spawn_fake_emby_once(body: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        captured
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(&buf[..n]).to_string());

        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });

    (format!("http://{addr}"), requests)
}
