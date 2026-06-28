use emby_manager::{
    emby::EmbyClient,
    openapi::ApiDoc,
    posters::{PosterDetectRequest, detect_mismatched_posters},
};
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

#[test]
fn openapi_registers_poster_detection() {
    let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    assert!(paths.contains_key("/api/v2/posters/detect-mismatch"));

    let schemas = doc["components"]["schemas"].as_object().unwrap();
    assert!(schemas.contains_key("PosterDetectRequest"));
    assert!(schemas.contains_key("PosterDetectResponse"));
    assert!(schemas.contains_key("PosterSignalItem"));
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
