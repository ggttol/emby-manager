use axum::{Json, extract::State};
use emby_manager::{
    db,
    media_fs::{self, ManageDeleteRequest},
    settings::Settings,
    state::AppState,
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::sleep,
};
use uuid::Uuid;

#[tokio::test]
async fn delete_execute_deletes_emby_first_retries_then_disk_notify_and_undo() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping manage execute DB test; set EMBY_MANAGER_MANAGE_TEST_DATABASE_URL");
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![
        FakeResponse::no_content(),
        FakeResponse::json(r#"{"Items":[{"Id":"item-delete"}]}"#),
        FakeResponse::no_content(),
        FakeResponse::json(r#"{"Items":[]}"#),
        FakeResponse::no_content(),
    ])
    .await;
    configure_emby(&state, &base_url).await;
    let cd_target = state.settings.cd_root.join("Movies/A/Movie");
    let strm_target = state.settings.strm_root.join("Movies/A/Movie");
    std::fs::create_dir_all(&cd_target).unwrap();
    std::fs::create_dir_all(&strm_target).unwrap();
    std::fs::write(cd_target.join("movie.mkv"), "media").unwrap();
    std::fs::write(strm_target.join("movie.strm"), "strm").unwrap();

    let task = media_fs::execute_delete(
        State(state.clone()),
        Json(ManageDeleteRequest {
            lib: "Movies".to_string(),
            folder: "A/Movie".to_string(),
            item_id: Some("item-delete".to_string()),
            reason: Some(format!("delete-execute-{}", Uuid::new_v4())),
        }),
    )
    .await
    .expect("delete execute should create a task")
    .0;

    assert_eq!(task.kind, "manage_delete_execute");
    let task = wait_for_task_status(&state, task.id, "done").await;
    assert_eq!(task["result"]["ok"], true);
    assert_eq!(task["result"]["preview"], false);
    assert_eq!(task["result"]["dry_run"], false);
    assert_eq!(task["result"]["emby_gone"], true);
    assert_eq!(task["result"]["deleted_from"], json!(["115", "strm"]));
    assert_eq!(task["result"]["notified"], true);
    assert!(!cd_target.exists(), "cd path should be deleted after Emby");
    assert!(
        !strm_target.exists(),
        "strm path should be deleted after Emby"
    );

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    assert!(
        requests[0].starts_with("DELETE /Items/item-delete?"),
        "{}",
        requests[0]
    );
    assert!(requests[1].starts_with("GET /Items?"), "{}", requests[1]);
    assert!(requests[1].contains("Ids=item-delete"), "{}", requests[1]);
    assert!(requests[1].contains("Limit=1"), "{}", requests[1]);
    assert!(
        requests[2].starts_with("DELETE /Items/item-delete?"),
        "{}",
        requests[2]
    );
    assert!(requests[3].starts_with("GET /Items?"), "{}", requests[3]);
    assert!(
        requests[4].starts_with("POST /Library/Media/Updated?"),
        "{}",
        requests[4]
    );
    let notify_body = request_body(&requests[4]);
    assert!(notify_body.contains(r#""Path":"/strm/Movies/A/Movie""#));
    assert!(notify_body.contains(r#""UpdateType":"Deleted""#));
    drop(requests);

    let undo_payload = undo_payload_for(&state, task["result"]["undo_id"].as_str().unwrap()).await;
    assert_eq!(undo_payload["lib"], "Movies");
    assert_eq!(undo_payload["folder"], "A/Movie");
    assert_eq!(undo_payload["deleted_from"], json!(["115", "strm"]));
}

#[tokio::test]
async fn delete_execute_skips_deleted_notification_when_no_paths_were_removed() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping manage execute DB test; set EMBY_MANAGER_MANAGE_TEST_DATABASE_URL");
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![
        FakeResponse::no_content(),
        FakeResponse::json(r#"{"Items":[]}"#),
    ])
    .await;
    configure_emby(&state, &base_url).await;

    let task = media_fs::execute_delete(
        State(state.clone()),
        Json(ManageDeleteRequest {
            lib: "Movies".to_string(),
            folder: "Missing/Movie".to_string(),
            item_id: Some("item-gone".to_string()),
            reason: Some(format!("delete-empty-{}", Uuid::new_v4())),
        }),
    )
    .await
    .expect("delete execute should create a task")
    .0;

    let task = wait_for_task_status(&state, task.id, "done").await;
    assert_eq!(task["result"]["deleted_from"], json!([]));
    assert_eq!(task["result"]["notified"], false);

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "empty disk delete must not notify Emby");
    assert!(
        requests[0].starts_with("DELETE /Items/item-gone?"),
        "{}",
        requests[0]
    );
    assert!(requests[1].starts_with("GET /Items?"), "{}", requests[1]);
    drop(requests);

    let undo_payload = undo_payload_for(&state, task["result"]["undo_id"].as_str().unwrap()).await;
    assert_eq!(undo_payload["lib"], "Movies");
    assert_eq!(undo_payload["folder"], "Missing/Movie");
    assert_eq!(undo_payload["deleted_from"], json!([]));
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

async fn wait_for_task_status(state: &AppState, id: Uuid, status: &str) -> Value {
    for _ in 0..80 {
        let task: Value =
            sqlx::query_scalar("SELECT to_jsonb(task_runs) FROM task_runs WHERE id = $1")
                .bind(id)
                .fetch_one(&state.pool)
                .await
                .expect("load task");
        if task["status"] == status {
            return task;
        }
        if task["status"] == "error" {
            panic!("task {id} failed: {}", task["error"]);
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("task {id} did not reach {status}");
}

async fn undo_payload_for(state: &AppState, id: &str) -> Value {
    sqlx::query_scalar("SELECT payload FROM undo_entries WHERE id = $1 AND op = 'delete'")
        .bind(Uuid::parse_str(id).unwrap())
        .fetch_one(&state.pool)
        .await
        .expect("load undo payload")
}

async fn test_state() -> Option<(TempDir, AppState)> {
    let database_url = manage_test_database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect manage test database");
    db::migrate(&pool)
        .await
        .expect("run manage test migrations");
    let tmp = tempfile::tempdir().unwrap();
    let cd_root = tmp.path().join("cd");
    let strm_root = tmp.path().join("strm");
    std::fs::create_dir_all(&cd_root).unwrap();
    std::fs::create_dir_all(&strm_root).unwrap();
    let settings = test_settings(database_url, cd_root, strm_root);
    Some((tmp, AppState::new(pool, settings)))
}

fn manage_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_MANAGE_TEST_DATABASE_URL") {
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

#[derive(Clone)]
struct FakeResponse {
    status: &'static str,
    body: &'static str,
}

impl FakeResponse {
    fn json(body: &'static str) -> Self {
        Self {
            status: "200 OK",
            body,
        }
    }

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
