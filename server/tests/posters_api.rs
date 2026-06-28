use emby_manager::{
    db,
    emby::EmbyClient,
    openapi::ApiDoc,
    posters::{
        PosterApplyRequest, PosterDetectRequest, PosterSearchRequest, apply_poster_match,
        detect_mismatched_posters, search_posters,
    },
};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use std::env;
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::{Duration, timeout},
};
use utoipa::OpenApi;

#[tokio::test]
async fn detects_missing_primary_and_declared_tmdb_mismatch_from_fake_emby() {
    let libraries = r#"[
        {
            "ItemId": "lib-movies",
            "Name": "Movies",
            "CollectionType": "movies",
            "Locations": ["/strm/Movies"]
        },
        {
            "ItemId": "lib-shows",
            "Name": "Shows",
            "CollectionType": "tvshows",
            "Locations": ["/strm/Shows"]
        }
    ]"#;
    let movie_items = r#"{
        "Items": [
            {
                "Id": "movie-missing",
                "Name": "No Poster",
                "Type": "Movie",
                "Path": "/strm/Movies/No Poster [tmdbid-100]/No Poster.strm",
                "ProviderIds": {"Tmdb": "100"},
                "ImageTags": {}
            },
            {
                "Id": "movie-mismatch",
                "Name": "Wrong Movie",
                "Type": "Movie",
                "Path": "/strm/Movies/中文片 [tmdbid-200]/movie.strm",
                "ProviderIds": {"Tmdb": "201"},
                "ImageTags": {"Primary": "poster-tag"}
            }
        ],
        "TotalRecordCount": 2
    }"#;
    let show_items = r#"{
        "Items": [
            {
                "Id": "show-ok",
                "Name": "Normal Show",
                "Type": "Series",
                "Path": "/strm/Shows/Normal Show/show.strm",
                "ProviderIds": {"Tmdb": "300"},
                "ImageTags": {"Primary": "poster-tag"}
            }
        ],
        "TotalRecordCount": 1
    }"#;
    let (base_url, requests, handle) =
        spawn_fake_emby(vec![libraries, movie_items, show_items]).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let report = detect_mismatched_posters(&client, PosterDetectRequest::default())
        .await
        .unwrap();

    assert!(report.ok);
    assert_eq!(report.scanned_libraries, 2);
    assert_eq!(report.scanned_items, 3);
    assert_eq!(report.missing_primary_total, 1);
    assert_eq!(report.mismatch_total, 1);
    assert_eq!(report.total, 2);
    assert_eq!(report.items[0].id, "movie-mismatch");
    assert_eq!(report.items[0].score, 100);
    assert_eq!(report.items[0].declared_tmdb.as_deref(), Some("200"));
    assert_eq!(report.items[0].tmdb, "201");
    assert_eq!(report.items[0].folder_clean, "中文片");
    assert!(
        report.items[0]
            .signals
            .iter()
            .any(|signal| signal.kind == "declared_tmdb_mismatch")
    );
    assert_eq!(report.items[1].id, "movie-missing");
    assert_eq!(report.items[1].score, 40);
    assert!(!report.items[1].has_poster);
    assert!(
        report.items[1]
            .signals
            .iter()
            .any(|signal| signal.kind == "missing_primary")
    );

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0].starts_with("GET /Library/VirtualFolders?api_key=secret-key "),
        "{}",
        requests[0]
    );
    assert!(requests[1].starts_with("GET /Items?"), "{}", requests[1]);
    assert!(
        requests[1].contains("ParentId=lib-movies"),
        "{}",
        requests[1]
    );
    assert!(
        requests[1].contains("IncludeItemTypes=Movie"),
        "{}",
        requests[1]
    );
    assert!(
        requests[1].contains("Fields=ProviderIds%2CPath%2CImageTags")
            || requests[1].contains("Fields=ProviderIds,Path,ImageTags"),
        "{}",
        requests[1]
    );
    assert!(
        requests[2].contains("ParentId=lib-shows"),
        "{}",
        requests[2]
    );
    assert!(
        requests[2].contains("IncludeItemTypes=Series"),
        "{}",
        requests[2]
    );
}

#[tokio::test]
async fn searches_remote_posters_and_normalizes_legacy_candidate_shape() {
    let candidates = r#"[
        {
            "Name": "沙丘",
            "ProductionYear": 2021,
            "ProviderIds": {"Tmdb": 438631},
            "ImageUrl": "https://image.tmdb.org/poster.jpg",
            "Overview": "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz"
        },
        {
            "Name": "No Tmdb",
            "ProductionYear": 1999,
            "ProviderIds": {},
            "Overview": null
        }
    ]"#;
    let (base_url, requests, handle) = spawn_fake_emby(vec![candidates]).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let result = search_posters(
        &client,
        PosterSearchRequest {
            id: "item-1".to_string(),
            name: "沙丘".to_string(),
            item_type: "Movie".to_string(),
            limit: Some(8),
        },
    )
    .await
    .unwrap();

    assert!(result.ok);
    assert_eq!(result.candidates.len(), 2);
    assert_eq!(result.candidates[0].name, "沙丘");
    assert_eq!(result.candidates[0].year, Some(2021));
    assert_eq!(result.candidates[0].tmdb, "438631");
    assert_eq!(
        result.candidates[0].img,
        "https://image.tmdb.org/poster.jpg"
    );
    assert_eq!(result.candidates[0].overview.chars().count(), 160);
    assert_eq!(result.candidates[1].tmdb, "");
    assert_eq!(result.candidates[1].img, "");

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("POST /Items/RemoteSearch/Movie?api_key=secret-key "),
        "{}",
        requests[0]
    );
    assert!(requests[0].contains("\"Name\":\"沙丘\""), "{}", requests[0]);
    assert!(
        requests[0].contains("\"IncludeDisabledProviders\":true"),
        "{}",
        requests[0]
    );
}

#[tokio::test]
async fn remote_poster_search_errors_return_empty_candidates_like_legacy() {
    let client = EmbyClient::new("http://127.0.0.1:1", "secret-key", reqwest::Client::new());

    let result = search_posters(
        &client,
        PosterSearchRequest {
            id: "item-1".to_string(),
            name: "missing".to_string(),
            item_type: "Series".to_string(),
            limit: Some(8),
        },
    )
    .await
    .unwrap();

    assert!(result.ok);
    assert!(result.candidates.is_empty());
}

#[tokio::test]
async fn apply_poster_records_rebind_undo_and_downloads_primary_fallback() {
    let Some(database_url) = posters_test_database_url() else {
        eprintln!("skipping posters DB test; set EMBY_MANAGER_POSTERS_TEST_DATABASE_URL");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect posters test database");
    db::migrate(&pool)
        .await
        .expect("run posters test migrations");

    let item_id = format!("poster-item-{}", uuid::Uuid::new_v4());
    let old_item = format!(
        r#"{{
            "Items": [{{
                "Id": "{item_id}",
                "Name": "旧名",
                "Type": "Movie",
                "ProviderIds": {{"Tmdb": "111"}},
                "ImageTags": {{}}
            }}],
            "TotalRecordCount": 1
        }}"#
    );
    let item_without_poster = format!(
        r#"{{
            "Items": [{{
                "Id": "{item_id}",
                "Name": "新名",
                "Type": "Movie",
                "ProviderIds": {{"Tmdb": "222"}},
                "ImageTags": {{}}
            }}],
            "TotalRecordCount": 1
        }}"#
    );
    let search = r#"[
        {
            "Name": "新名",
            "ProductionYear": 2024,
            "ProviderIds": {"Tmdb": "222"},
            "ImageUrl": "https://image.tmdb.org/poster.jpg",
            "Overview": "ok"
        }
    ]"#;
    let item_with_poster = format!(
        r#"{{
            "Items": [{{
                "Id": "{item_id}",
                "Name": "新名",
                "Type": "Movie",
                "ProviderIds": {{"Tmdb": "222"}},
                "ImageTags": {{"Primary": "poster-tag"}}
            }}],
            "TotalRecordCount": 1
        }}"#
    );
    let responses = vec![
        fake_response(200, old_item),
        fake_response(204, String::new()),
        fake_response(204, String::new()),
        fake_response(200, item_without_poster),
        fake_response(200, search.to_string()),
        fake_response(204, String::new()),
        fake_response(200, item_with_poster),
    ];
    let (base_url, requests, handle) = spawn_fake_emby_responses(responses).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let applied = apply_poster_match(
        &pool,
        &client,
        PosterApplyRequest {
            id: item_id.clone(),
            tmdb: "222".to_string(),
            item_type: "Movie".to_string(),
            name: Some("新名".to_string()),
        },
    )
    .await
    .unwrap();

    assert!(applied.ok);
    assert!(applied.poster);
    assert_eq!(applied.name, "新名");
    assert_eq!(applied.tmdb, "222");
    assert_eq!(applied.apply_status, 204);
    assert_eq!(applied.refresh_status, 204);
    assert_eq!(applied.image_download_status, Some(204));

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 7);
    assert!(
        requests[1].starts_with(&format!("POST /Items/RemoteSearch/Apply/{item_id}?")),
        "{}",
        requests[1]
    );
    assert!(request_body(&requests[1]).contains("\"Tmdb\":\"222\""));
    assert!(
        requests[2].starts_with(&format!("POST /Items/{item_id}/Refresh?")),
        "{}",
        requests[2]
    );
    assert!(
        requests[5].starts_with(&format!("POST /Items/{item_id}/RemoteImages/Download?")),
        "{}",
        requests[5]
    );

    let undo_payload: Value =
        sqlx::query_scalar("SELECT payload FROM undo_entries WHERE op = 'rebind' AND payload->>'id' = $1 ORDER BY created_at DESC LIMIT 1")
            .bind(&item_id)
            .fetch_one(&pool)
            .await
            .expect("load rebind undo payload");
    assert_eq!(undo_payload["old_tmdb"], "111");
    assert_eq!(undo_payload["new_tmdb"], "222");
    assert_eq!(undo_payload["type"], "Movie");
}

#[test]
fn openapi_registers_poster_detection() {
    let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    assert!(paths.contains_key("/api/v2/posters/detect-mismatch"));
    assert!(paths.contains_key("/api/v2/posters/search"));
    assert!(paths.contains_key("/api/v2/posters/apply"));
    assert!(paths.contains_key("/api/v2/posters/fix-batch"));

    let schemas = doc["components"]["schemas"].as_object().unwrap();
    assert!(schemas.contains_key("PosterDetectRequest"));
    assert!(schemas.contains_key("PosterDetectResponse"));
    assert!(schemas.contains_key("PosterSignalItem"));
    assert!(schemas.contains_key("PosterSearchRequest"));
    assert!(schemas.contains_key("PosterSearchResponse"));
    assert!(schemas.contains_key("PosterSearchCandidate"));
    assert!(schemas.contains_key("PosterApplyRequest"));
    assert!(schemas.contains_key("PosterApplyResponse"));
    assert!(schemas.contains_key("PosterFixBatchRequest"));
    assert!(schemas.contains_key("PosterFixBatchResult"));
    assert!(schemas.contains_key("PosterFixOneResult"));
}

fn posters_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_POSTERS_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.contains("test").then_some(url)
}

async fn spawn_fake_emby(
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

fn fake_response(status: u16, body: String) -> FakeResponse {
    FakeResponse { status, body }
}

struct FakeResponse {
    status: u16,
    body: String,
}

async fn spawn_fake_emby_responses(
    responses: Vec<FakeResponse>,
) -> (String, Arc<Mutex<Vec<String>>>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);

    let handle = tokio::spawn(async move {
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            captured.lock().unwrap().push(request);

            let reason = match response.status {
                200 => "OK",
                204 => "No Content",
                _ => "OK",
            };
            let raw = format!(
                "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response.status,
                reason,
                response.body.len(),
                response.body
            );
            socket.write_all(raw.as_bytes()).await.unwrap();
        }
    });

    (format!("http://{addr}"), requests, handle)
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
        if let Some(header_end) = find_header_end(&buf) {
            let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .unwrap_or(0);
            if buf.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn request_body(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap_or_default()
}
