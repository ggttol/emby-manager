use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, StatusCode, header::CONTENT_TYPE},
};
use emby_manager::{db, media_fs, settings::Settings, state::AppState};
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
use tower::ServiceExt;

static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn create_library_rejects_empty_name_without_emby_call() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some((_tmp, state)) = test_state().await else {
        eprintln!(
            "skipping library create DB test; set EMBY_MANAGER_LIBRARY_CREATE_TEST_DATABASE_URL"
        );
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![]).await;
    configure_emby(&state, &base_url).await;
    let app = test_app(state.clone());

    let (status, body) = send(
        &app,
        Method::POST,
        "/api/v2/libraries",
        Some(json!({"name": "   ", "collection_type": "movies"})),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "bad_request");
    assert!(
        body["err"]
            .as_str()
            .is_some_and(|err| err.contains("库名") || err.contains("name")),
        "{body}"
    );
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn create_library_rejects_invalid_collection_type_without_emby_call() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some((_tmp, state)) = test_state().await else {
        eprintln!(
            "skipping library create DB test; set EMBY_MANAGER_LIBRARY_CREATE_TEST_DATABASE_URL"
        );
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![]).await;
    configure_emby(&state, &base_url).await;
    let app = test_app(state.clone());

    let (status, body) = send(
        &app,
        Method::POST,
        "/api/v2/libraries",
        Some(json!({"name": "Music", "collection_type": "music"})),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "bad_request");
    assert!(
        body["err"].as_str().is_some_and(|err| {
            err.contains("tvshows") || err.contains("movies") || err.contains("collection_type")
        }),
        "{body}"
    );
    assert!(requests.lock().unwrap().is_empty());
    assert!(!state.settings.strm_root.join("Music").exists());
    assert!(!state.settings.cd_root.join("Music").exists());
}

#[tokio::test]
async fn create_library_rejects_path_traversal_without_creating_dirs_or_posting_to_emby() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some((tmp, state)) = test_state().await else {
        eprintln!(
            "skipping library create DB test; set EMBY_MANAGER_LIBRARY_CREATE_TEST_DATABASE_URL"
        );
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![FakeResponse::json("[]")]).await;
    configure_emby(&state, &base_url).await;
    let app = test_app(state.clone());
    let outside = tmp.path().join("outside");

    let (status, body) = send(
        &app,
        Method::POST,
        "/api/v2/libraries",
        Some(json!({"name": "../outside", "collection_type": "movies"})),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "bad_request");
    assert!(
        body["err"]
            .as_str()
            .is_some_and(|err| err.contains("非法") || err.contains("路径")),
        "{body}"
    );
    assert!(
        !outside.exists(),
        "path traversal must not create {}",
        outside.display()
    );
    assert_no_virtual_folder_post(&requests.lock().unwrap());
}

#[tokio::test]
async fn create_library_rejects_duplicate_name_without_creating_dirs_or_posting_to_emby() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some((_tmp, state)) = test_state().await else {
        eprintln!(
            "skipping library create DB test; set EMBY_MANAGER_LIBRARY_CREATE_TEST_DATABASE_URL"
        );
        return;
    };
    let (base_url, requests) = spawn_fake_emby(vec![FakeResponse::json(
        r#"[
            {
                "ItemId": "movies-lib",
                "Name": "Movies",
                "CollectionType": "movies",
                "Locations": ["/strm/Movies"]
            }
        ]"#,
    )])
    .await;
    configure_emby(&state, &base_url).await;
    let app = test_app(state.clone());

    let (status, body) = send(
        &app,
        Method::POST,
        "/api/v2/libraries",
        Some(json!({"name": "Movies", "collection_type": "movies"})),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "conflict");
    assert!(
        body["err"]
            .as_str()
            .is_some_and(|err| err.contains("已存在") || err.contains("duplicate")),
        "{body}"
    );
    assert!(!state.settings.strm_root.join("Movies").exists());
    assert!(!state.settings.cd_root.join("Movies").exists());

    let requests = requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request_line(request).starts_with("GET /Library/VirtualFolders?")),
        "{requests:#?}"
    );
    assert_no_virtual_folder_post(&requests);
}

#[tokio::test]
async fn create_library_posts_virtual_folder_with_template_options_and_creates_roots() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some((_tmp, state)) = test_state().await else {
        eprintln!(
            "skipping library create DB test; set EMBY_MANAGER_LIBRARY_CREATE_TEST_DATABASE_URL"
        );
        return;
    };
    let existing_libraries = r#"[
        {
            "ItemId": "movies-lib",
            "Name": "Movies",
            "CollectionType": "movies",
            "Locations": ["/strm/Movies"],
            "LibraryOptions": {
                "PathInfos": [{"Path": "/strm/Movies"}],
                "EnableRealtimeMonitor": false,
                "PreferredMetadataLanguage": "zh-CN"
            }
        }
    ]"#;
    let libraries_after_create = r#"[
        {
            "ItemId": "movies-lib",
            "Name": "Movies",
            "CollectionType": "movies",
            "Locations": ["/strm/Movies"]
        },
        {
            "ItemId": "anime-lib",
            "Name": "Anime",
            "CollectionType": "movies",
            "Locations": ["/strm/Anime"]
        }
    ]"#;
    let (base_url, requests) = spawn_fake_emby(vec![
        FakeResponse::json(existing_libraries),
        FakeResponse::no_content(),
        FakeResponse::json(libraries_after_create),
    ])
    .await;
    configure_emby(&state, &base_url).await;
    let app = test_app(state.clone());

    let (status, body) = send(
        &app,
        Method::POST,
        "/api/v2/libraries",
        Some(json!({"name": "Anime", "collection_type": "movies"})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["name"], "Anime");
    assert_eq!(body["id"], "anime-lib");
    assert!(state.settings.strm_root.join("Anime").is_dir());
    assert!(state.settings.cd_root.join("Anime").is_dir());

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3, "{requests:#?}");
    assert_virtual_folder_get(&requests[0]);
    assert_virtual_folder_get(&requests[2]);

    let post_line = request_line(&requests[1]);
    assert!(
        post_line.starts_with("POST /Library/VirtualFolders?"),
        "{}",
        requests[1]
    );
    assert!(post_line.contains("api_key=secret-key"), "{post_line}");
    assert!(post_line.contains("name=Anime"), "{post_line}");
    assert!(post_line.contains("collectionType=movies"), "{post_line}");
    assert!(
        post_line.contains("paths=%2Fstrm%2FAnime") || post_line.contains("paths=/strm/Anime"),
        "{post_line}"
    );
    assert!(post_line.contains("refreshLibrary=false"), "{post_line}");

    let post_body: Value =
        serde_json::from_str(request_body(&requests[1])).expect("valid Emby create JSON body");
    assert_eq!(
        post_body["LibraryOptions"]["PathInfos"],
        json!([{"Path": "/strm/Anime"}])
    );
    assert_eq!(
        post_body["LibraryOptions"]["EnableRealtimeMonitor"],
        json!(false)
    );
    assert_eq!(
        post_body["LibraryOptions"]["PreferredMetadataLanguage"],
        json!("zh-CN")
    );
}

fn test_app(state: AppState) -> Router {
    media_fs::router().with_state(state)
}

async fn test_state() -> Option<(TempDir, AppState)> {
    let database_url = library_create_test_database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect library create test database");
    db::migrate(&pool)
        .await
        .expect("run library create test migrations");
    let tmp = tempfile::tempdir().unwrap();
    let cd_root = tmp.path().join("cd");
    let strm_root = tmp.path().join("strm");
    std::fs::create_dir_all(&cd_root).unwrap();
    std::fs::create_dir_all(&strm_root).unwrap();
    let settings = test_settings(database_url, cd_root, strm_root);
    Some((tmp, AppState::new(pool, settings)))
}

fn library_create_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_LIBRARY_CREATE_TEST_DATABASE_URL") {
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

async fn send(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let body = body.map(|value| value.to_string()).unwrap_or_default();
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if !body.is_empty() {
        builder = builder.header(CONTENT_TYPE, "application/json");
    }

    let response = app
        .clone()
        .oneshot(
            builder
                .body(Body::from(body))
                .expect("build library create request"),
        )
        .await
        .expect("send library create request");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read library create response body");
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
    };
    (status, body)
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
            let request = read_http_request(&mut socket).await;
            captured.lock().unwrap().push(request);

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

async fn read_http_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        let n = socket.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if http_request_complete(&buf) {
            break;
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn http_request_complete(buf: &[u8]) -> bool {
    let Some(header_end) = buf.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    buf.len() >= header_end + 4 + content_length
}

fn assert_virtual_folder_get(request: &str) {
    let line = request_line(request);
    assert!(
        line.starts_with("GET /Library/VirtualFolders?"),
        "{request}"
    );
    assert!(line.contains("api_key=secret-key"), "{line}");
}

fn assert_no_virtual_folder_post(requests: &[String]) {
    assert!(
        !requests
            .iter()
            .any(|request| request_line(request).starts_with("POST /Library/VirtualFolders?")),
        "{requests:#?}"
    );
}

fn request_line(request: &str) -> &str {
    request.lines().next().unwrap_or_default()
}

fn request_body(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap_or_default()
}
