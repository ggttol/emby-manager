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
    time::{Duration, sleep},
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

#[tokio::test]
async fn duplicates_include_emby_provider_id_groups_without_folder_tmdb() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping dedup API DB test; set EMBY_MANAGER_DEDUP_TEST_DATABASE_URL");
        return;
    };
    let libraries = r#"[{
        "ItemId": "lib-tv",
        "Name": "TV",
        "CollectionType": "tvshows",
        "Locations": ["/strm/TV"]
    }]"#;
    let items = r#"{
        "Items": [
            {
                "Id": "item-old",
                "Name": "Show",
                "Type": "Series",
                "Path": "/strm/TV/Show",
                "ProviderIds": {"Tmdb": "123"}
            },
            {
                "Id": "item-copy",
                "Name": "Show Copy",
                "Type": "Series",
                "Path": "/strm/TV/Show(1)",
                "ProviderIds": {"Tmdb": "123"}
            }
        ],
        "TotalRecordCount": 2
    }"#;
    let (base_url, _requests) = spawn_fake_emby(vec![
        FakeResponse::json(libraries),
        FakeResponse::json(items),
    ])
    .await;
    configure_emby(&state, &base_url).await;
    std::fs::create_dir_all(state.settings.strm_root.join("TV/Show")).unwrap();
    std::fs::create_dir_all(state.settings.strm_root.join("TV/Show(1)")).unwrap();
    std::fs::write(state.settings.strm_root.join("TV/Show/S01E01.strm"), "strm").unwrap();
    std::fs::write(
        state.settings.strm_root.join("TV/Show(1)/S01E01.strm"),
        "strm",
    )
    .unwrap();

    let response = dedup::duplicates(State(state.clone()))
        .await
        .expect("duplicates should include Emby provider id groups")
        .0;

    assert_eq!(response.dups.len(), 0);
    assert_eq!(response.review.len(), 1);
    assert_eq!(response.review[0].tmdb, "123");
    assert_eq!(response.review[0].rows.len(), 2);
    assert!(
        response.review[0]
            .rows
            .iter()
            .any(|row| row.folder == "Show(1)" && row.item_id.as_deref() == Some("item-copy")),
        "{:?}",
        response.review[0].rows
    );
}

#[tokio::test]
async fn execute_dedup_allows_emby_item_only_duplicate_delete() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping dedup API DB test; set EMBY_MANAGER_DEDUP_TEST_DATABASE_URL");
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![FakeResponse::no_content()]).await;
    configure_emby(&state, &base_url).await;

    let response = dedup::execute_dedup(
        State(state.clone()),
        Json(dedup::DedupExecuteRequest {
            tmdb: Some("661029".to_string()),
            remove: vec![dedup::DedupFolderRef {
                lib: "TV".to_string(),
                folder: "../Pokemon XY".to_string(),
                item_id: Some("53148".to_string()),
            }],
            reason: Some("provider duplicate item cleanup".to_string()),
        }),
    )
    .await
    .expect("Emby item-only duplicate delete should execute")
    .0;

    assert!(response.ok);
    assert_eq!(response.tmdb.as_deref(), Some("661029"));
    assert_eq!(response.removed.len(), 1);
    assert_eq!(response.removed[0].folder, "../Pokemon XY");
    assert_eq!(response.removed[0].deleted_from, vec!["emby"]);
    assert!(response.removed[0].emby_updates.is_empty());
    assert!(!response.removed[0].notified);

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("DELETE /Items/53148?"),
        "{}",
        requests[0]
    );
    assert!(
        requests[0].contains("api_key=secret-key"),
        "{}",
        requests[0]
    );
}

#[tokio::test]
async fn execute_dedup_continues_path_cleanup_when_emby_delete_fails() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping dedup API DB test; set EMBY_MANAGER_DEDUP_TEST_DATABASE_URL");
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![
        FakeResponse::server_error(),
        FakeResponse::no_content(),
        FakeResponse::no_content(),
    ])
    .await;
    configure_emby(&state, &base_url).await;

    let cd_folder = state.settings.cd_root.join("TV/Show [tmdbid-100]");
    let strm_folder = state.settings.strm_root.join("TV/Show [tmdbid-100]");
    std::fs::create_dir_all(&cd_folder).unwrap();
    std::fs::create_dir_all(&strm_folder).unwrap();
    std::fs::write(strm_folder.join("S01E01.strm"), "strm").unwrap();

    let response = dedup::execute_dedup(
        State(state.clone()),
        Json(dedup::DedupExecuteRequest {
            tmdb: Some("100".to_string()),
            remove: vec![dedup::DedupFolderRef {
                lib: "TV".to_string(),
                folder: "Show [tmdbid-100]".to_string(),
                item_id: Some("item-fails".to_string()),
            }],
            reason: Some("emby delete failure should fall back to path cleanup".to_string()),
        }),
    )
    .await
    .expect("Emby delete failure should not block path cleanup")
    .0;

    assert!(response.ok);
    assert_eq!(response.removed.len(), 1);
    assert_eq!(response.removed[0].deleted_from, vec!["strm", "emby"]);
    assert_eq!(response.removed[0].warnings.len(), 2);
    assert!(
        response.removed[0].warnings[0].contains("HTTP 500"),
        "{:?}",
        response.removed[0].warnings
    );
    assert!(
        cd_folder.exists(),
        "CloudDrive folder remains without 115 delete context"
    );
    assert!(
        !strm_folder.exists(),
        "STRM folder should be cleaned by fallback"
    );

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0].starts_with("DELETE /Items/item-fails?"),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].starts_with("DELETE /Items/item-fails?"),
        "{}",
        requests[1]
    );
    assert!(
        requests[2].starts_with("POST /Library/Media/Updated?"),
        "{}",
        requests[2]
    );
}

#[tokio::test]
async fn execute_dedup_batch_records_warnings_when_emby_delete_fails() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping dedup API DB test; set EMBY_MANAGER_DEDUP_TEST_DATABASE_URL");
        return;
    };
    let (base_url, _requests) = spawn_fake_emby(vec![
        FakeResponse::server_error(),
        FakeResponse::no_content(),
        FakeResponse::no_content(),
    ])
    .await;
    configure_emby(&state, &base_url).await;

    let strm_folder = state.settings.strm_root.join("TV/Show [tmdbid-200]");
    std::fs::create_dir_all(&strm_folder).unwrap();
    std::fs::write(strm_folder.join("S01E01.strm"), "strm").unwrap();

    let task = dedup::execute_dedup_batch(
        State(state.clone()),
        Json(dedup::DedupExecuteBatchRequest {
            groups: vec![dedup::DedupExecuteBatchGroup {
                tmdb: Some("200".to_string()),
                remove: vec![dedup::DedupFolderRef {
                    lib: "TV".to_string(),
                    folder: "Show [tmdbid-200]".to_string(),
                    item_id: Some("item-fails".to_string()),
                }],
            }],
        }),
    )
    .await
    .expect("batch task should be created")
    .0;

    let mut row: Option<(String, Option<Value>, Option<String>)> = None;
    for _ in 0..40 {
        let current = sqlx::query_as::<_, (String, Option<Value>, Option<String>)>(
            "SELECT status, result, error FROM task_runs WHERE id = $1",
        )
        .bind(task.id)
        .fetch_one(&state.pool)
        .await
        .expect("load task");
        if !matches!(current.0.as_str(), "pending" | "running") {
            row = Some(current);
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    let (status, result, error) = row.expect("task should finish");
    let result = result.expect("batch should store a structured result");
    assert_eq!(status, "done");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(result["ok"], json!(true));
    assert_eq!(result["ok_count"], json!(1));
    assert_eq!(result["error_count"], json!(0));
    assert_eq!(result["results"][0]["ok"], json!(true));
    assert_eq!(
        result["results"][0]["warnings"].as_array().unwrap().len(),
        2
    );
    assert!(!strm_folder.exists(), "fallback should remove STRM");
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

    fn server_error() -> Self {
        Self {
            status: "500 Internal Server Error",
            body: r#"{"error":"locked"}"#,
        }
    }

    fn json(body: &'static str) -> Self {
        Self {
            status: "200 OK",
            body,
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
