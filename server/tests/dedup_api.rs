use axum::{Json, extract::State};
use emby_manager::{db, dedup, error::AppError, settings::Settings, state::AppState};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use uuid::Uuid;

#[test]
fn replace_plan_rejects_target_conflict_same_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let cd_root = tmp.path().join("cd");
    let strm_root = tmp.path().join("strm");
    std::fs::create_dir_all(cd_root.join("Movies/Movie")).unwrap();
    std::fs::create_dir_all(&strm_root).unwrap();

    let err = dedup::plan_replace_for_roots(
        &cd_root,
        &strm_root,
        &dedup::ReplaceRequest {
            lib: "Movies".to_string(),
            win_folder: "Movie".to_string(),
            lose_folder: "Movie".to_string(),
            reason: None,
        },
    )
    .expect_err("same win/lose folder should be rejected");

    assert!(matches!(err, AppError::Conflict(_)));
}

#[test]
fn replace_plan_rejects_path_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let cd_root = tmp.path().join("cd");
    let strm_root = tmp.path().join("strm");
    std::fs::create_dir_all(cd_root.join("Movies/New")).unwrap();
    std::fs::create_dir_all(cd_root.join("Movies/Old")).unwrap();
    std::fs::create_dir_all(&strm_root).unwrap();

    let err = dedup::plan_replace_for_roots(
        &cd_root,
        &strm_root,
        &dedup::ReplaceRequest {
            lib: "Movies".to_string(),
            win_folder: "New".to_string(),
            lose_folder: "../Old".to_string(),
            reason: None,
        },
    )
    .expect_err("path traversal should be rejected");

    assert!(matches!(err, AppError::BadRequest(_)));
}

#[tokio::test]
async fn replace_execute_renames_strm_and_writes_replace_undo() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping dedup API DB test; set EMBY_MANAGER_DEDUP_TEST_DATABASE_URL");
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![FakeResponse::no_content()]).await;
    configure_emby(&state, &base_url).await;

    let win_cd = state.settings.cd_root.join("Movies/Show [tmdbid-123](1)");
    let lose_cd = state.settings.cd_root.join("Movies/Show [tmdbid-123]");
    let win_strm = state.settings.strm_root.join("Movies/Show [tmdbid-123](1)");
    let lose_strm = state.settings.strm_root.join("Movies/Show [tmdbid-123]");
    std::fs::create_dir_all(&win_cd).unwrap();
    std::fs::create_dir_all(&lose_cd).unwrap();
    std::fs::create_dir_all(&win_strm).unwrap();
    std::fs::create_dir_all(&lose_strm).unwrap();
    std::fs::write(win_cd.join("S01E01.mkv"), "new media").unwrap();
    std::fs::write(lose_cd.join("S01E01.mkv"), "old media").unwrap();
    std::fs::write(
        win_strm.join("S01E01.strm"),
        "/media/Movies/Show [tmdbid-123](1)/S01E01.mkv",
    )
    .unwrap();
    std::fs::write(lose_strm.join("old.strm"), "old strm").unwrap();

    let response = dedup::replace_execute(
        State(state.clone()),
        Json(dedup::ReplaceRequest {
            lib: "Movies".to_string(),
            win_folder: "Show [tmdbid-123](1)".to_string(),
            lose_folder: "Show [tmdbid-123]".to_string(),
            reason: Some(format!("replace-{}", Uuid::new_v4())),
        }),
    )
    .await
    .expect("replace should execute")
    .0;

    assert!(response.ok);
    assert_eq!(response.kept_as, "Show [tmdbid-123]");
    assert!(response.renamed);
    assert_eq!(response.deleted_from, vec!["115", "strm"]);
    assert!(response.notified);
    assert_eq!(
        serde_json::to_value(&response.emby_updates).unwrap(),
        json!([
            {"Path":"/strm/Movies/Show [tmdbid-123](1)","UpdateType":"Deleted"},
            {"Path":"/strm/Movies/Show [tmdbid-123]","UpdateType":"Modified"}
        ])
    );

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("POST /Library/Media/Updated?"),
        "{}",
        requests[0]
    );
    assert!(
        requests[0].contains("api_key=secret-key"),
        "{}",
        requests[0]
    );
    let notify_body: Value = serde_json::from_str(request_body(&requests[0])).unwrap();
    assert_eq!(
        notify_body,
        json!({
            "Updates": [
                {"Path":"/strm/Movies/Show [tmdbid-123](1)","UpdateType":"Deleted"},
                {"Path":"/strm/Movies/Show [tmdbid-123]","UpdateType":"Modified"}
            ]
        })
    );
    drop(requests);

    assert!(!win_cd.exists(), "win cd path should be renamed away");
    assert!(lose_cd.join("S01E01.mkv").exists());
    assert!(!win_strm.exists(), "win strm path should be renamed away");
    assert_eq!(
        std::fs::read_to_string(lose_strm.join("S01E01.strm")).unwrap(),
        "/media/Movies/Show [tmdbid-123]/S01E01.mkv"
    );

    let payload: Value =
        sqlx::query_scalar("SELECT payload FROM undo_entries WHERE id = $1 AND op = 'replace'")
            .bind(response.undo_id)
            .fetch_one(&state.pool)
            .await
            .expect("load replace undo payload");
    assert_eq!(payload["lib"], "Movies");
    assert_eq!(payload["win_was"], "Show [tmdbid-123](1)");
    assert_eq!(payload["lose_was"], "Show [tmdbid-123]");
    assert_eq!(payload["now_folder"], "Show [tmdbid-123]");
}

async fn configure_emby(state: &AppState, base_url: &str) {
    for (key, value) in [
        ("emby_url", json!(base_url)),
        ("api_key", json!("secret-key")),
    ] {
        sqlx::query(
            "INSERT INTO app_settings(key, value, updated_at) VALUES ($1, $2, now())
             ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(&state.pool)
        .await
        .expect("save Emby config");
    }
}

#[derive(Clone)]
struct FakeResponse {
    status: &'static str,
    body: &'static str,
}

impl FakeResponse {
    fn no_content() -> Self {
        Self {
            status: "204 No Content",
            body: "",
        }
    }
}

async fn spawn_fake_emby(responses: Vec<FakeResponse>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);

    tokio::spawn(async move {
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            captured
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[..n]).to_string());

            let raw = format!(
                "HTTP/1.1 {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response.status,
                response.body.len(),
                response.body
            );
            socket.write_all(raw.as_bytes()).await.unwrap();
        }
    });

    (format!("http://{addr}"), requests)
}

fn request_body(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap_or_default()
}

async fn test_state() -> Option<(TempDir, AppState)> {
    let database_url = dedup_test_database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect dedup test database");
    db::migrate(&pool).await.expect("run dedup test migrations");
    let tmp = tempfile::tempdir().unwrap();
    let cd_root = tmp.path().join("cd");
    let strm_root = tmp.path().join("strm");
    std::fs::create_dir_all(&cd_root).unwrap();
    std::fs::create_dir_all(&strm_root).unwrap();
    let settings = test_settings(database_url, cd_root, strm_root);
    Some((tmp, AppState::new(pool, settings)))
}

fn dedup_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_DEDUP_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn test_settings(database_url: String, cd_root: PathBuf, strm_root: PathBuf) -> Settings {
    Settings {
        host: "127.0.0.1".to_string(),
        port: 0,
        database_url,
        web_dist: PathBuf::from("/tmp"),
        legacy_dir: PathBuf::from("/tmp"),
        bootstrap_password: "admin".to_string(),
        cd_root,
        strm_root,
        docker_bin: PathBuf::from("/usr/bin/docker"),
        task_concurrency: 1,
    }
}
