use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::CONTENT_TYPE},
};
use emby_manager::{db, settings::Settings, state::AppState, wizard};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
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

static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn add_new_runs_batch_transfer_and_library_scan_with_item_errors() {
    let _guard = DB_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let cd_root = tmp.path().join("cd");
    let strm_root = tmp.path().join("strm");
    prepare_wizard_media_fixture(&cd_root, &strm_root);
    let Some(state) = test_state_with_roots(cd_root.clone(), strm_root.clone()).await else {
        eprintln!("skipping wizard API DB test; set EMBY_MANAGER_WIZARD_TEST_DATABASE_URL");
        return;
    };

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
    let offline_fail = r#"{"state": false, "error_msg": "bad magnet"}"#;
    let (c115_base, c115_requests, c115_handle) = spawn_fake_json_server(vec![
        fake_json(snap),
        fake_json(receive),
        fake_json(space),
        fake_json(offline_fail),
    ])
    .await;

    let libraries = r#"[{
        "ItemId": "lib-movie",
        "Name": "电影",
        "CollectionType": "movies",
        "Locations": ["/strm/电影"]
    }]"#;
    let poster_items = r#"{
        "Items": [
            {
                "Id": "movie-missing-poster",
                "Name": "Share Movie",
                "Type": "Movie",
                "Path": "/strm/电影/Share Movie [tmdbid-100]/Share Movie.strm",
                "ProviderIds": {"Tmdb": "100"},
                "ImageTags": {}
            },
            {
                "Id": "movie-wrong-tmdb",
                "Name": "Wrong Movie",
                "Type": "Movie",
                "Path": "/strm/电影/Wrong Movie [tmdbid-200]/movie.strm",
                "ProviderIds": {"Tmdb": "201"},
                "ImageTags": {"Primary": "poster-tag"}
            }
        ],
        "TotalRecordCount": 2
    }"#;
    let (emby_base, emby_requests, emby_handle) = spawn_fake_json_server(vec![
        fake_json(libraries),
        fake_response("204 No Content", ""),
        fake_json(libraries),
        fake_json(poster_items),
    ])
    .await;

    configure_settings(&state, &c115_base, &emby_base).await;
    let app = wizard::router().with_state(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v2/wizard/add-new")
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
                                "name": "Bad Magnet",
                                "link": "magnet:?xt=urn:btih:0123456789abcdef"
                            },
                            {
                                "name": "Unsupported",
                                "link": "https://example.com/file.torrent",
                                "link_type": "other"
                            }
                        ],
                        "target": {"lib": "电影"},
                        "delay_ms": 0
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
    assert_eq!(body["kind"], "add_new");
    assert_eq!(body["total"], 6);
    let id = Uuid::parse_str(body["id"].as_str().unwrap()).unwrap();

    let task = wait_for_task_status(&state, id, "done").await;
    assert_eq!(task["status_text"], "完成，2 项转存/离线失败");
    assert_eq!(task["result"]["ok"], false);
    assert_eq!(task["result"]["target"]["cid"], "12345");
    assert_eq!(task["result"]["target"]["lib"], "电影");
    assert_eq!(task["result"]["transfer"]["total"], 3);
    assert_eq!(task["result"]["transfer"]["succeeded"], 1);
    assert_eq!(task["result"]["transfer"]["failed"], 2);
    assert_eq!(task["result"]["transfer"]["items"][0]["ok"], true);
    assert_eq!(
        task["result"]["transfer"]["items"][0]["action"],
        "save_share"
    );
    assert_eq!(
        task["result"]["transfer"]["items"][0]["response"]["count"],
        1
    );
    assert_eq!(task["result"]["transfer"]["items"][1]["ok"], false);
    assert_eq!(
        task["result"]["transfer"]["items"][1]["action"],
        "offline_download"
    );
    assert!(
        task["result"]["transfer"]["items"][1]["error"]
            .as_str()
            .unwrap()
            .contains("bad magnet"),
        "{task}"
    );
    assert_eq!(task["result"]["transfer"]["items"][2]["ok"], false);
    assert_eq!(
        task["result"]["transfer"]["items"][2]["action"],
        "unsupported"
    );
    assert_eq!(task["result"]["strm"]["triggered"], true);
    assert_eq!(task["result"]["strm"]["lib"], "电影");
    assert_eq!(task["result"]["strm"]["new_count"], 1);
    assert_eq!(
        task["result"]["strm"]["new_folders"]["Share Movie [tmdbid-100]"],
        1
    );
    assert_eq!(task["result"]["dedup"]["triggered"], true);
    assert_eq!(task["result"]["dedup"]["lib"], "电影");
    assert_eq!(task["result"]["dedup"]["dups_count"], 0);
    assert_eq!(task["result"]["dedup"]["review_count"], 0);
    assert!(
        task["result"]["dedup"]["dups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| group["keep"]["lib"] == "电影"
                || group["remove"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["lib"] == "电影")),
        "{task}"
    );
    assert!(
        strm_root
            .join("电影/Share Movie [tmdbid-100]/Share Movie.strm")
            .is_file()
    );
    assert!(
        !strm_root
            .join("剧集/Other Show [tmdbid-999]/Other Show.strm")
            .is_file()
    );
    assert_eq!(task["result"]["scan"]["triggered"], true);
    assert_eq!(task["result"]["scan"]["mode"], "library");
    assert_eq!(task["result"]["scan"]["lib"], "电影");
    assert_eq!(task["result"]["scan"]["item_id"], "lib-movie");
    assert_eq!(task["result"]["scan"]["code"], 204);
    assert_ne!(task["result"]["poster"]["status"], "placeholder");
    assert_eq!(task["result"]["poster"]["status"], "issues");
    assert_eq!(task["result"]["poster"]["triggered"], true);
    assert_eq!(task["result"]["poster"]["scanned_libraries"], 1);
    assert_eq!(task["result"]["poster"]["scanned_items"], 2);
    assert_eq!(task["result"]["poster"]["issue_count"], 2);
    assert_eq!(task["result"]["poster"]["missing_primary_count"], 1);
    assert_eq!(task["result"]["poster"]["mismatch_count"], 1);
    assert_eq!(
        task["result"]["poster"]["items"].as_array().unwrap().len(),
        2
    );
    assert_ne!(task["result"]["check"]["status"], "placeholder");
    assert_eq!(task["result"]["check"]["status"], "errors");
    assert_eq!(task["result"]["check"]["item_success_count"], 1);
    assert_eq!(task["result"]["check"]["item_error_count"], 2);
    assert_eq!(task["result"]["check"]["stage_error_count"], 0);
    assert_eq!(task["result"]["check"]["suspicious_count"], 2);
    assert_eq!(
        task["result"]["check"]["errors"].as_array().unwrap().len(),
        2
    );
    assert_eq!(task["result"]["check"]["items"][1]["status"], "error");
    assert!(
        task["result"]["check"]["suspicious"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "movie-wrong-tmdb"),
        "{task}"
    );

    timeout(Duration::from_secs(1), c115_handle)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(1), emby_handle)
        .await
        .unwrap()
        .unwrap();

    let c115_requests = c115_requests.lock().unwrap();
    assert_eq!(c115_requests.len(), 4);
    assert!(c115_requests[0].starts_with("GET /share/snap?"));
    assert!(c115_requests[1].starts_with("POST /share/receive "));
    assert!(
        c115_requests[1].contains("share_code=swOK"),
        "{}",
        c115_requests[1]
    );
    assert!(
        c115_requests[1].contains("receive_code=RC"),
        "{}",
        c115_requests[1]
    );
    assert!(
        c115_requests[1].contains("cid=12345"),
        "{}",
        c115_requests[1]
    );
    assert!(c115_requests[2].starts_with("GET /?"));
    assert!(
        c115_requests[2].contains("ct=offline"),
        "{}",
        c115_requests[2]
    );
    assert!(c115_requests[3].starts_with("POST /web/lixian/?"));
    assert!(
        c115_requests[3].contains("wp_path_id=12345"),
        "{}",
        c115_requests[3]
    );

    let emby_requests = emby_requests.lock().unwrap();
    assert_eq!(emby_requests.len(), 4);
    assert!(
        emby_requests[0].starts_with("GET /Library/VirtualFolders?api_key=secret-key"),
        "{}",
        emby_requests[0]
    );
    assert!(
        emby_requests[1].starts_with("POST /Items/lib-movie/Refresh?"),
        "{}",
        emby_requests[1]
    );
    assert!(
        emby_requests[1].contains("api_key=secret-key"),
        "{}",
        emby_requests[1]
    );
    assert!(
        emby_requests[2].starts_with("GET /Library/VirtualFolders?api_key=secret-key"),
        "{}",
        emby_requests[2]
    );
    assert!(
        emby_requests[3].starts_with("GET /Items?"),
        "{}",
        emby_requests[3]
    );
    assert!(
        emby_requests[3].contains("ParentId=lib-movie"),
        "{}",
        emby_requests[3]
    );
    assert!(
        emby_requests[3].contains("IncludeItemTypes=Movie"),
        "{}",
        emby_requests[3]
    );
}

async fn configure_settings(state: &AppState, c115_base: &str, emby_base: &str) {
    for (key, value) in [
        (
            "c115_cookie",
            json!("CID=cid-value; UID=123456_A1; SEID=seid-value"),
        ),
        ("c115_cid_map", json!({"电影": "12345"})),
        ("c115_api_base_url", json!(c115_base)),
        ("c115_site_base_url", json!(c115_base)),
        ("emby_url", json!(emby_base)),
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
        .expect("save test setting");
    }
}

fn prepare_wizard_media_fixture(cd_root: &std::path::Path, strm_root: &std::path::Path) {
    let movie_dir = cd_root.join("电影/Share Movie [tmdbid-100]");
    std::fs::create_dir_all(&movie_dir).unwrap();
    std::fs::write(movie_dir.join("Share Movie.mkv"), b"movie").unwrap();

    let other_a = strm_root.join("剧集/Other Show [tmdbid-999]");
    let other_b = strm_root.join("剧集/Other Show Copy [tmdbid-999]");
    std::fs::create_dir_all(&other_a).unwrap();
    std::fs::create_dir_all(&other_b).unwrap();
    std::fs::write(
        other_a.join("E01.strm"),
        "/media/剧集/Other Show [tmdbid-999]/E01.mkv",
    )
    .unwrap();
    std::fs::write(
        other_b.join("E02.strm"),
        "/media/剧集/Other Show Copy [tmdbid-999]/E02.mkv",
    )
    .unwrap();
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

#[derive(Clone)]
struct FakeResponse {
    status: &'static str,
    body: &'static str,
}

fn fake_json(body: &'static str) -> FakeResponse {
    fake_response("200 OK", body)
}

fn fake_response(status: &'static str, body: &'static str) -> FakeResponse {
    FakeResponse { status, body }
}

async fn spawn_fake_json_server(
    responses: Vec<FakeResponse>,
) -> (String, Arc<Mutex<Vec<String>>>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);

    let handle = tokio::spawn(async move {
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 8192];
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

    (format!("http://{addr}"), requests, handle)
}

async fn test_state_with_roots(cd_root: PathBuf, strm_root: PathBuf) -> Option<AppState> {
    let database_url = wizard_test_database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect wizard test database");
    db::migrate(&pool)
        .await
        .expect("run wizard test migrations");
    sqlx::query("TRUNCATE task_runs, app_settings RESTART IDENTITY CASCADE")
        .execute(&pool)
        .await
        .expect("reset wizard test tables");
    Some(AppState::new(
        pool,
        test_settings_with_roots(database_url, cd_root, strm_root),
    ))
}

fn wizard_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_WIZARD_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn test_settings_with_roots(
    database_url: String,
    cd_root: PathBuf,
    strm_root: PathBuf,
) -> Settings {
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
