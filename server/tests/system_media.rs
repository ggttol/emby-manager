use axum::extract::{Query, State};
use emby_manager::{
    media_fs::{self, StrmListQuery},
    settings::Settings,
    state::AppState,
    system,
};
use std::path::{Path, PathBuf};

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
    assert!(
        summary
            .warnings
            .iter()
            .any(|warning| warning.contains("数据库检查失败"))
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
    assert_eq!(overview.files, 4);
    assert_eq!(overview.strm_files, 1);
    assert_eq!(overview.subtitle_files, 2);
    assert_eq!(overview.other_files, 1);
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
