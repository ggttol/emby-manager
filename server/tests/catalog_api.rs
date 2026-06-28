use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::CONTENT_TYPE},
};
use emby_manager::{catalog, db, settings::Settings, state::AppState};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn transfer_plan_routes_share115_to_c115_save() {
    let link = "https://115cdn.com/s/swABC?password=YYY#frag";
    let (status, body) = post_transfer_plan(json!({
        "item": {
            "name": "沙丘 4K",
            "sheet": "电影",
            "link": link,
            "is_pkg": true,
            "link_type": "share115"
        },
        "lib": "电影"
    }))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["action"], "save_share");
    assert_eq!(body["link_type"], "share115");
    assert_eq!(body["transfer"], true);
    assert_eq!(body["is_pkg"], true);
    assert_eq!(body["target"]["lib"], "电影");
    assert_eq!(body["label"], "沙丘 4K");
    assert_eq!(body["save"]["endpoint"], "/api/v2/c115/save");
    assert_eq!(body["save"]["method"], "POST");
    assert_eq!(body["save"]["share"], "swABC");
    assert_eq!(body["save"]["receive_code"], "YYY");
    assert_eq!(body["save"]["payload"]["url"], link);
    assert_eq!(body["save"]["payload"]["pwd"], "YYY");
    assert_eq!(body["save"]["payload"]["lib"], "电影");
    assert!(body.get("offline").is_none(), "{body}");
    assert!(body.get("unsupported").is_none(), "{body}");
}

#[tokio::test]
async fn transfer_plan_routes_magnet_to_c115_offline() {
    let link = "magnet:?xt=urn:btih:0123456789abcdef";
    let (status, body) = post_transfer_plan(json!({
        "link": link,
        "label": "Ubuntu ISO",
        "cid": "123456"
    }))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["action"], "offline_download");
    assert_eq!(body["link_type"], "magnet");
    assert_eq!(body["transfer"], false);
    assert_eq!(body["target"]["cid"], "123456");
    assert_eq!(body["offline"]["endpoint"], "/api/v2/c115/offline");
    assert_eq!(body["offline"]["protocol"], "magnet");
    assert_eq!(body["offline"]["payload"]["url"], link);
    assert_eq!(body["offline"]["payload"]["cid"], "123456");
    assert_eq!(body["offline"]["payload"]["label"], "Ubuntu ISO");
    assert!(body.get("save").is_none(), "{body}");
}

#[tokio::test]
async fn transfer_plan_routes_ed2k_to_c115_offline() {
    let link = "ed2k://|file|movie.mkv|123|abcdef|/";
    let (status, body) = post_transfer_plan(json!({
        "item": {
            "name": "movie.mkv",
            "link": link,
            "link_type": "ed2k"
        },
        "lib": "电视剧"
    }))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["action"], "offline_download");
    assert_eq!(body["link_type"], "ed2k");
    assert_eq!(body["target"]["lib"], "电视剧");
    assert_eq!(body["offline"]["endpoint"], "/api/v2/c115/offline");
    assert_eq!(body["offline"]["protocol"], "ed2k");
    assert_eq!(body["offline"]["payload"]["url"], link);
    assert_eq!(body["offline"]["payload"]["lib"], "电视剧");
}

#[tokio::test]
async fn transfer_plan_marks_other_links_unsupported() {
    let link = "https://example.com/file.torrent";
    let (status, body) = post_transfer_plan(json!({
        "link": link,
        "link_type": "other",
        "cid": "123456"
    }))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], false);
    assert_eq!(body["action"], "unsupported");
    assert_eq!(body["link_type"], "other");
    assert_eq!(body["transfer"], false);
    assert_eq!(body["target"]["cid"], "123456");
    assert_eq!(body["unsupported"]["link"], link);
    assert!(
        body["unsupported"]["reason"]
            .as_str()
            .unwrap()
            .contains("not supported"),
        "{body}"
    );
    assert!(body.get("save").is_none(), "{body}");
    assert!(body.get("offline").is_none(), "{body}");
}

#[tokio::test]
async fn transfer_execute_runs_batch_and_records_item_errors() {
    let Some(database_url) = catalog_test_database_url() else {
        eprintln!(
            "skipping catalog transfer execute DB test; set EMBY_MANAGER_CATALOG_TEST_DATABASE_URL"
        );
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect catalog test database");
    db::migrate(&pool)
        .await
        .expect("run catalog test migrations");
    let state = test_state_with_pool(pool);

    let snap = r#"{
        "state": true,
        "data": {
            "total": 1,
            "shareinfo": {"share_title": "Share Movie"},
            "list": [{"fid": "file-share", "n": "Movie.mkv", "s": 1024}]
        }
    }"#;
    let receive = r#"{"state": true}"#;
    let space = r#"{"state": true, "sign": "SIGN", "time": 1710000000}"#;
    let offline_ok = r#"{"state": true, "info_hash": "HASH"}"#;
    let offline_fail = r#"{"state": false, "error_msg": "bad ed2k"}"#;
    let (base_url, requests, handle) =
        spawn_fake_c115(vec![snap, receive, space, offline_ok, space, offline_fail]).await;
    configure_c115(&state, &base_url).await;

    let app = catalog::router().with_state(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v2/catalog/transfer/execute")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "items": [
                            {
                                "name": "Share Movie",
                                "link": "https://115.com/s/swOK?password=RC",
                                "link_type": "share115"
                            },
                            {
                                "name": "Magnet Movie",
                                "link": "magnet:?xt=urn:btih:0123456789abcdef"
                            },
                            {
                                "name": "Bad Ed2k",
                                "link": "ed2k://|file|bad.mkv|123|abcdef|/"
                            },
                            {
                                "name": "Unsupported",
                                "link": "https://example.com/file.torrent",
                                "link_type": "other"
                            }
                        ],
                        "target": {"lib": "电影"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "catalog_transfer_execute");
    assert_eq!(body["total"], 4);
    let id = Uuid::parse_str(body["id"].as_str().unwrap()).unwrap();

    let task = wait_for_task_status(&state, id, "done").await;
    assert_eq!(task["status_text"], "完成，2 项失败");
    assert_eq!(task["result"]["ok"], false);
    assert_eq!(task["result"]["total"], 4);
    assert_eq!(task["result"]["succeeded"], 2);
    assert_eq!(task["result"]["failed"], 2);
    assert_eq!(task["result"]["target"]["cid"], "12345");
    assert_eq!(task["result"]["target"]["lib"], "电影");
    assert_eq!(task["result"]["items"][0]["ok"], true);
    assert_eq!(task["result"]["items"][0]["action"], "save_share");
    assert_eq!(task["result"]["items"][0]["response"]["count"], 1);
    assert_eq!(task["result"]["items"][1]["ok"], true);
    assert_eq!(task["result"]["items"][1]["action"], "offline_download");
    assert_eq!(task["result"]["items"][1]["response"]["info_hash"], "HASH");
    assert_eq!(task["result"]["items"][2]["ok"], false);
    assert!(
        task["result"]["items"][2]["error"]
            .as_str()
            .unwrap()
            .contains("bad ed2k"),
        "{task}"
    );
    assert_eq!(task["result"]["items"][3]["ok"], false);
    assert_eq!(task["result"]["items"][3]["action"], "unsupported");

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 6);
    assert!(requests[0].starts_with("GET /share/snap?"));
    assert!(requests[1].starts_with("POST /share/receive "));
    assert!(requests[1].contains("share_code=swOK"), "{}", requests[1]);
    assert!(requests[1].contains("receive_code=RC"), "{}", requests[1]);
    assert!(requests[1].contains("cid=12345"), "{}", requests[1]);
    assert!(requests[2].starts_with("GET /?"));
    assert!(requests[2].contains("ct=offline"), "{}", requests[2]);
    assert!(requests[3].starts_with("POST /web/lixian/?"));
    assert!(requests[3].contains("wp_path_id=12345"), "{}", requests[3]);
    assert!(requests[4].starts_with("GET /?"));
    assert!(requests[5].starts_with("POST /web/lixian/?"));
}

#[tokio::test]
async fn duplicates_endpoint_returns_limited_readonly_catalog_groups() {
    let Some(database_url) = catalog_test_database_url() else {
        eprintln!(
            "skipping catalog duplicates DB test; set EMBY_MANAGER_CATALOG_TEST_DATABASE_URL"
        );
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect catalog test database");
    db::migrate(&pool)
        .await
        .expect("run catalog test migrations");
    reset_catalog_items(&pool).await;

    let app = catalog::router().with_state(test_state_with_pool(pool));
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v2/catalog/duplicates?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["readonly"], true);
    assert_eq!(body["limit"], 1);
    assert_eq!(body["duplicate_link_groups"], 2);
    assert_eq!(body["duplicate_name_groups"], 2);
    assert_eq!(body["link_groups"].as_array().unwrap().len(), 1);
    assert_eq!(body["name_groups"].as_array().unwrap().len(), 1);
    assert_eq!(body["link_groups"][0]["key"], "https://115.com/s/shared");
    assert_eq!(body["link_groups"][0]["count"], 3);
    assert!(
        body["link_type_distribution"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["link_type"] == "share115" && row["count"] == 5),
        "{body}"
    );
}

async fn post_transfer_plan(body: Value) -> (StatusCode, Value) {
    let app = catalog::router().with_state(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v2/catalog/transfer-plan")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, body)
}

async fn reset_catalog_items(pool: &PgPool) {
    sqlx::query("TRUNCATE catalog_items RESTART IDENTITY")
        .execute(pool)
        .await
        .expect("reset catalog items");
    sqlx::query(
        "INSERT INTO catalog_items(name, sheet, link, is_pkg, link_type)
         VALUES
            ('Show A 1080p', '剧集', 'https://115.com/s/shared', false, 'share115'),
            ('Show A 4K', '剧集', 'https://115.com/s/shared', false, 'share115'),
            ('Show A Remux', '剧集', 'https://115.com/s/shared', true, 'share115'),
            ('Same Name', '电影', 'https://115.com/s/name-1', false, 'share115'),
            ('Same Name', '电影', 'magnet:?xt=urn:btih:name2', false, 'magnet'),
            ('Other Same', '电影', 'https://115.com/s/name-3', false, 'share115'),
            ('Other Same', '电影', 'ed2k://|file|other.mkv|1|hash|/', false, 'ed2k'),
            ('Unique', '电影', 'https://115.com/s/unique', false, 'share115'),
            ('Duplicate Link Two A', '电影', 'magnet:?xt=urn:btih:duplink2', false, 'magnet'),
            ('Duplicate Link Two B', '电影', 'magnet:?xt=urn:btih:duplink2', false, 'magnet')",
    )
    .execute(pool)
    .await
    .expect("seed catalog duplicates");
}

async fn configure_c115(state: &AppState, base_url: &str) {
    for (key, value) in [
        (
            "c115_cookie",
            json!("CID=cid-value; UID=123456_A1; SEID=seid-value"),
        ),
        ("c115_cid_map", json!({"电影": "12345"})),
        ("c115_api_base_url", json!(base_url)),
        ("c115_site_base_url", json!(base_url)),
    ] {
        sqlx::query(
            "INSERT INTO app_settings(key, value, updated_at) VALUES ($1, $2, now())
             ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(&state.pool)
        .await
        .expect("save 115 config");
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

fn test_state() -> AppState {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://emby_manager:emby_manager@127.0.0.1:1/emby_manager_test")
        .unwrap();
    test_state_with_pool(pool)
}

fn test_state_with_pool(pool: PgPool) -> AppState {
    AppState::new(
        pool,
        Settings {
            host: "127.0.0.1".to_string(),
            port: 0,
            database_url: "postgres://emby_manager:emby_manager@127.0.0.1:1/emby_manager_test"
                .to_string(),
            web_dist: PathBuf::from("/tmp"),
            legacy_dir: PathBuf::from("/tmp"),
            bootstrap_password: "admin".to_string(),
            cd_root: PathBuf::from("/tmp/cd"),
            strm_root: PathBuf::from("/tmp/strm"),
            docker_bin: PathBuf::from("/usr/bin/docker"),
            task_concurrency: 1,
        },
    )
}

fn catalog_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_CATALOG_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}
