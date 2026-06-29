use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::CONTENT_TYPE},
};
use emby_manager::{
    c115::{
        self, C115CidMatch, C115Client, C115OfflineRequest, C115SaveRequest, validate_target_cid,
    },
    db,
    openapi::ApiDoc,
    settings::Settings,
    state::AppState,
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex as AsyncMutex,
    task::JoinHandle,
    time::{Duration, sleep, timeout},
};
use tower::ServiceExt;
use utoipa::OpenApi;
use uuid::Uuid;

static TEST_DB_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

#[tokio::test]
async fn save_to_cid_snaps_then_receives_selected_files() {
    let snap = r#"{
        "state": true,
        "data": {
            "total": 2,
            "shareinfo": {"share_title": "Share Title"},
            "list": [
                {"fid": "file-a", "n": "Movie.mkv", "s": 1024},
                {"cid": 9001, "n": "Season", "size": 0}
            ]
        }
    }"#;
    let receive = r#"{"state": true}"#;
    let (base_url, requests, handle) = spawn_fake_c115(vec![snap, receive]).await;
    let client = C115Client::new(
        base_url,
        "CID=cid-value; UID=123456_A1; SEID=seid-value",
        reqwest::Client::new(),
    );

    let result = client
        .save_to_cid(
            C115SaveRequest {
                url: "https://115.com/s/swABC?password=YYY#anchor".to_string(),
                pwd: None,
                lib: None,
                cid: None,
                label: Some("电影".to_string()),
                file_ids: None,
            },
            "12345".to_string(),
            Some("电影".to_string()),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(result.share, "swABC");
    assert_eq!(result.count, 2);
    assert_eq!(result.cid, "12345");
    assert_eq!(result.lib.as_deref(), Some("电影"));
    assert_eq!(result.title.as_deref(), Some("Share Title"));

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /share/snap?"));
    assert!(requests[1].starts_with("POST /share/receive "));
    assert!(requests[1].contains("share_code=swABC"), "{}", requests[1]);
    assert!(requests[1].contains("receive_code=YYY"), "{}", requests[1]);
    assert!(requests[1].contains("cid=12345"), "{}", requests[1]);
    assert!(requests[1].contains("user_id=123456"), "{}", requests[1]);
    assert!(
        requests[1].contains("file_id=file-a%2C9001")
            || requests[1].contains("file_id=file-a,9001"),
        "{}",
        requests[1]
    );
}

#[tokio::test]
async fn offline_add_gets_sign_before_posting_add_task() {
    let space = r#"{"state": true, "sign": "SIGN", "time": 1710000000}"#;
    let add = r#"{"state": true, "info_hash": "HASH"}"#;
    let (base_url, requests, handle) = spawn_fake_c115(vec![space, add]).await;
    let client = C115Client::new(
        base_url,
        "CID=cid-value; UID=123456_A1; SEID=seid-value",
        reqwest::Client::new(),
    );

    let result = client
        .offline_add(
            C115OfflineRequest {
                url: "magnet:?xt=urn:btih:abc".to_string(),
                lib: None,
                cid: None,
                label: Some("片名".to_string()),
            },
            "12345".to_string(),
            Some("电影".to_string()),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(result.info_hash.as_deref(), Some("HASH"));
    assert_eq!(result.cid, "12345");
    assert_eq!(result.lib.as_deref(), Some("电影"));

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /?"));
    assert!(requests[0].contains("ct=offline"), "{}", requests[0]);
    assert!(requests[0].contains("ac=space"), "{}", requests[0]);
    assert!(requests[1].starts_with("POST /web/lixian/?"));
    assert!(requests[1].contains("ct=lixian"), "{}", requests[1]);
    assert!(requests[1].contains("ac=add_task_url"), "{}", requests[1]);
    assert!(requests[1].contains("wp_path_id=12345"), "{}", requests[1]);
    assert!(requests[1].contains("sign=SIGN"), "{}", requests[1]);
    assert!(requests[1].contains("time=1710000000"), "{}", requests[1]);
}

#[tokio::test]
async fn auto_cid_matches_library_folder_names_without_writing_config() {
    let root = r#"{
        "state": true,
        "data": [
            {"cid": "10", "n": "电影"},
            {"cid": "11", "name": "Other"},
            {"fid": "file-a", "cid": "12", "n": "not-a-folder"}
        ]
    }"#;
    let (base_url, requests, handle) = spawn_fake_c115(vec![root]).await;
    let client = C115Client::new(base_url, "UID=123456_A1", reqwest::Client::new());
    let targets = BTreeMap::from([("电影".to_string(), "Movies".to_string())]);
    let current = BTreeMap::from([("Movies".to_string(), "999".to_string())]);

    let result = client.auto_cid(targets, current.clone(), 0).await.unwrap();

    assert!(result.ok);
    assert_eq!(result.scanned, 1);
    assert_eq!(result.current, current);
    assert_eq!(
        result.matches.get("Movies"),
        Some(&vec![C115CidMatch {
            cid: "10".to_string(),
            path: "/电影".to_string(),
        }])
    );

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /files?"));
    assert!(requests[0].contains("cid=0"), "{}", requests[0]);
    assert!(requests[0].contains("show_dir=1"), "{}", requests[0]);
}

#[tokio::test]
async fn save_active_task_reuse_includes_sorted_file_ids() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(state) = c115_test_state().await else {
        eprintln!("skipping c115 save DB test; set EMBY_MANAGER_C115_TEST_DATABASE_URL");
        return;
    };
    seed_c115_settings(&state).await;

    let permit = state
        .clouddrive_slot
        .clone()
        .acquire_owned()
        .await
        .expect("hold clouddrive slot");
    let app = c115::router().with_state(state.clone());

    let first_request = json!({
        "url": "https://115.com/s/swABC?password=YYY",
        "pwd": "YYY",
        "cid": "12345",
        "lib": "电影",
        "label": "片名",
        "file_ids": ["file-b", "file-a"]
    });
    let (status, first) = post_c115_save(&app, first_request).await;
    assert_eq!(status, StatusCode::OK, "{first}");

    let duplicate_request = json!({
        "url": "https://115.com/s/swABC?password=YYY",
        "pwd": "YYY",
        "cid": "12345",
        "lib": "电影",
        "label": "片名",
        "file_ids": ["file-a", "file-b"]
    });
    let (status, duplicate) = post_c115_save(&app, duplicate_request).await;
    assert_eq!(status, StatusCode::OK, "{duplicate}");
    assert_eq!(duplicate["id"], first["id"]);

    let different_files_request = json!({
        "url": "https://115.com/s/swABC?password=YYY",
        "pwd": "YYY",
        "cid": "12345",
        "lib": "电影",
        "label": "片名",
        "file_ids": ["file-c"]
    });
    let (status, different_files) = post_c115_save(&app, different_files_request).await;
    assert_eq!(status, StatusCode::OK, "{different_files}");
    assert_ne!(different_files["id"], first["id"]);

    let first_id = Uuid::parse_str(first["id"].as_str().unwrap()).unwrap();
    let params: Value = sqlx::query_scalar("SELECT params FROM task_runs WHERE id = $1")
        .bind(first_id)
        .fetch_one(&state.pool)
        .await
        .expect("load c115 save task params");
    assert_eq!(params["file_ids"], json!(["file-a", "file-b"]));

    let task_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_runs WHERE kind = $1")
        .bind("c115_save")
        .fetch_one(&state.pool)
        .await
        .expect("count c115 save tasks");
    assert_eq!(task_count, 2);

    sqlx::query("UPDATE task_runs SET cancel_requested = TRUE WHERE kind = $1")
        .bind("c115_save")
        .execute(&state.pool)
        .await
        .expect("cancel blocked c115 save tasks");
    drop(permit);
    sleep(Duration::from_millis(50)).await;
}

#[test]
fn target_cid_rejects_root_and_non_numeric_values() {
    assert_eq!(validate_target_cid("12345").unwrap(), "12345");
    for bad in ["", "0", "001", "abc", "12a", "-1"] {
        assert!(validate_target_cid(bad).is_err(), "{bad}");
    }
}

#[test]
fn openapi_registers_c115_live_ops() {
    let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    assert!(paths.contains_key("/api/v2/c115/save"));
    assert!(paths.contains_key("/api/v2/c115/offline"));
    assert!(paths.contains_key("/api/v2/c115/auto-cid"));

    let schemas = doc["components"]["schemas"].as_object().unwrap();
    assert!(schemas.contains_key("C115SaveRequest"));
    assert!(schemas.contains_key("C115OfflineRequest"));
    assert!(schemas.contains_key("C115AutoCidResponse"));
}

async fn post_c115_save(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v2/c115/save")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("build c115 save request"),
        )
        .await
        .expect("send c115 save request");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read c115 save response body");
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
    };
    (status, body)
}

async fn c115_test_state() -> Option<AppState> {
    let database_url = c115_test_database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect c115 test database");
    db::migrate(&pool).await.expect("run c115 test migrations");
    sqlx::query("TRUNCATE task_runs, app_settings RESTART IDENTITY CASCADE")
        .execute(&pool)
        .await
        .expect("reset c115 test tables");
    Some(AppState::new(pool, c115_test_settings(database_url)))
}

async fn seed_c115_settings(state: &AppState) {
    upsert_setting(
        &state.pool,
        "c115_cookie",
        json!("UID=123456_A1; CID=cid-value; SEID=seid-value"),
    )
    .await;
    upsert_setting(&state.pool, "c115_cid_map", json!({"电影": "12345"})).await;
}

async fn upsert_setting(pool: &PgPool, key: &str, value: Value) {
    sqlx::query(
        "INSERT INTO app_settings(key, value)
         VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    .expect("upsert c115 test setting");
}

fn c115_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_C115_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn c115_test_settings(database_url: String) -> Settings {
    Settings {
        host: "127.0.0.1".to_string(),
        port: 0,
        database_url,
        web_dist: PathBuf::from("/tmp"),
        legacy_dir: PathBuf::from("/tmp"),
        bootstrap_password: "admin".to_string(),
        cd_root: PathBuf::from("/tmp/cd"),
        strm_root: PathBuf::from("/tmp/strm"),
        docker_bin: PathBuf::from("/usr/bin/docker"),
        task_concurrency: 1,
    }
}

async fn spawn_fake_c115(
    bodies: Vec<&'static str>,
) -> (String, Arc<Mutex<Vec<String>>>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);

    let handle = tokio::spawn(async move {
        for body in bodies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 8192];
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
        }
    });

    (format!("http://{addr}"), requests, handle)
}
