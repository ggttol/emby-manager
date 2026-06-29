use emby_manager::emby::{EmbyClient, EmbyLibrary};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn emby_client_lists_libraries_from_virtual_folders() {
    let body = r#"[
        {
            "ItemId": "lib-movies",
            "Name": "Movies",
            "CollectionType": "movies",
            "Locations": ["/strm/Movies", "/strm/Movies"],
            "LibraryOptions": {"PathInfos": [{"Path": "/media/Movies"}]}
        },
        {
            "ItemId": "lib-shows",
            "Name": "Shows",
            "CollectionType": "tvshows",
            "Locations": ["/strm/Shows"]
        },
        {
            "Name": "",
            "Locations": [],
            "LibraryOptions": {"PathInfos": [{"Path": ""}, {"Path": "/mixed"}]}
        }
    ]"#;
    let (base_url, requests) = spawn_fake_emby_many(vec![body]).await;
    let client = EmbyClient::new(
        format!("{base_url}/emby/"),
        "secret-key",
        reqwest::Client::new(),
    );

    let libraries = client.libraries().await.unwrap();

    assert_eq!(
        libraries,
        vec![
            EmbyLibrary {
                id: Some("lib-movies".to_string()),
                name: "Movies".to_string(),
                library_type: "movies".to_string(),
                paths: vec!["/strm/Movies".to_string(), "/media/Movies".to_string()],
            },
            EmbyLibrary {
                id: Some("lib-shows".to_string()),
                name: "Shows".to_string(),
                library_type: "tvshows".to_string(),
                paths: vec!["/strm/Shows".to_string()],
            },
            EmbyLibrary {
                id: None,
                name: "(unnamed)".to_string(),
                library_type: "mixed".to_string(),
                paths: vec!["/mixed".to_string()],
            },
        ]
    );

    let request = requests.lock().unwrap()[0].clone();
    assert!(
        request.starts_with("GET /emby/Library/VirtualFolders?api_key=secret-key HTTP/1.1"),
        "{request}"
    );
}

#[tokio::test]
async fn emby_client_rejects_missing_api_key_without_http_call() {
    let client = EmbyClient::new("http://127.0.0.1:1/emby", " \t", reqwest::Client::new());

    let err = client.virtual_folders().await.unwrap_err().to_string();

    assert!(err.contains("api_key is not configured"), "{err}");
}

#[tokio::test]
async fn emby_client_refreshes_items_with_encoded_ids() {
    let (base_url, requests) = spawn_fake_emby_many(vec!["{}"]).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let code = client
        .refresh_item("id?evil=1&x=2#frag", true, false)
        .await
        .unwrap();

    assert_eq!(code, 204);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("POST /Items/id%3Fevil%3D1%26x%3D2%23frag/Refresh?"),
        "{}",
        requests[0]
    );
    assert!(
        requests[0].contains("api_key=secret-key"),
        "{}",
        requests[0]
    );
    assert!(requests[0].contains("Recursive=true"), "{}", requests[0]);
    assert!(
        requests[0].contains("MetadataRefreshMode=Default"),
        "{}",
        requests[0]
    );
}

#[tokio::test]
async fn emby_client_triggers_global_library_refresh() {
    let (base_url, requests) = spawn_fake_emby_many(vec!["{}"]).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let code = client.refresh_library().await.unwrap();

    assert_eq!(code, 204);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("POST /Library/Refresh?api_key=secret-key "),
        "{}",
        requests[0]
    );
}

#[tokio::test]
async fn emby_client_search_apply_and_download_poster_requests_are_typed_and_encoded() {
    let candidates = r#"[
        {
            "Name": "沙丘",
            "ProductionYear": 2021,
            "ProviderIds": {"Tmdb": "438631"},
            "ImageUrl": "https://image.tmdb.org/t/p/original/poster.jpg?sig=a&b=1",
            "Overview": "overview"
        }
    ]"#;
    let (base_url, requests) = spawn_fake_emby_many(vec![candidates, "{}", "{}"]).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let found = client
        .remote_search("id?evil=1&x=2#frag", "沙丘", "Movie", 8)
        .await
        .unwrap();
    let apply = client
        .apply_remote_search("id?evil=1&x=2#frag", "438631")
        .await
        .unwrap();
    let download = client
        .download_primary_image(
            "id?evil=1&x=2#frag",
            "https://image.tmdb.org/t/p/original/poster.jpg?sig=a&b=1",
        )
        .await
        .unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].provider_ids.get("Tmdb").unwrap(), "438631");
    assert_eq!(apply, 204);
    assert_eq!(download, 204);

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0].starts_with("POST /Items/RemoteSearch/Movie?api_key=secret-key "),
        "{}",
        requests[0]
    );
    assert!(
        request_body(&requests[0]).contains("\"ItemId\":\"id?evil=1&x=2#frag\""),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].starts_with("POST /Items/RemoteSearch/Apply/id%3Fevil%3D1%26x%3D2%23frag?"),
        "{}",
        requests[1]
    );
    assert!(
        request_body(&requests[1]).contains("\"Tmdb\":\"438631\""),
        "{}",
        requests[1]
    );
    assert!(
        requests[2].starts_with("POST /Items/id%3Fevil%3D1%26x%3D2%23frag/RemoteImages/Download?"),
        "{}",
        requests[2]
    );
    assert!(requests[2].contains("Type=Primary"), "{}", requests[2]);
    assert!(
        requests[2].contains("ImageUrl=https%3A%2F%2Fimage.tmdb.org"),
        "{}",
        requests[2]
    );
}

#[tokio::test]
async fn emby_client_lists_series_and_episodes_for_gap_scan() {
    let series = r#"{
        "Items": [
            {"Id": "series 1", "Name": "Show A", "ProviderIds": {"Tmdb": "123"}}
        ],
        "TotalRecordCount": 1
    }"#;
    let episodes = r#"{
        "Items": [
            {"Id": "e1", "ParentIndexNumber": 1, "IndexNumber": 1, "LocationType": "FileSystem"},
            {"Id": "e2", "ParentIndexNumber": 1, "IndexNumber": 2, "LocationType": "Virtual"}
        ]
    }"#;
    let (base_url, requests) = spawn_fake_emby_many(vec![series, episodes]).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let found = client.series("lib?id=1", 10).await.unwrap();
    let eps = client.episodes("series 1?x=2").await.unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id.as_deref(), Some("series 1"));
    assert_eq!(found[0].provider_id("Tmdb").as_deref(), Some("123"));
    assert_eq!(eps.len(), 2);
    assert_eq!(eps[1].location_type.as_deref(), Some("Virtual"));

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /Items?"), "{}", requests[0]);
    assert!(
        requests[0].contains("ParentId=lib%3Fid%3D1"),
        "{}",
        requests[0]
    );
    assert!(
        requests[0].contains("IncludeItemTypes=Series"),
        "{}",
        requests[0]
    );
    assert!(
        requests[0].contains("Fields=ProviderIds"),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].starts_with("GET /Shows/series%201%3Fx%3D2/Episodes?"),
        "{}",
        requests[1]
    );
    assert!(
        requests[1].contains("Fields=ParentIndexNumber%2CIndexNumber%2CLocationType"),
        "{}",
        requests[1]
    );
    assert!(requests[1].contains("Limit=6000"), "{}", requests[1]);
}

#[tokio::test]
async fn emby_client_lists_library_items_for_manage_picker() {
    let items = r#"{
        "Items": [
            {
                "Id": "movie-1",
                "Name": "Movie A",
                "Type": "Movie",
                "Path": "/strm/Movies/Movie A [tmdbid-123]/Movie A.strm",
                "ProductionYear": 2026,
                "ProviderIds": {"Tmdb": "123"}
            }
        ],
        "TotalRecordCount": 1
    }"#;
    let (base_url, requests) = spawn_fake_emby_many(vec![items]).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let found = client
        .library_items("lib movies", "Movie", 10)
        .await
        .unwrap();

    assert_eq!(found.items.len(), 1);
    assert_eq!(found.total_record_count, Some(1));
    assert!(!found.truncated);
    assert_eq!(found.items[0].id.as_deref(), Some("movie-1"));
    assert_eq!(found.items[0].item_type.as_deref(), Some("Movie"));
    assert_eq!(found.items[0].production_year, Some(2026));
    assert_eq!(found.items[0].provider_id("Tmdb").as_deref(), Some("123"));

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /Items?"), "{}", requests[0]);
    assert!(
        requests[0].contains("ParentId=lib%20movies")
            || requests[0].contains("ParentId=lib+movies"),
        "{}",
        requests[0]
    );
    assert!(
        requests[0].contains("IncludeItemTypes=Movie"),
        "{}",
        requests[0]
    );
    assert!(
        requests[0].contains("Fields=Path%2CProductionYear%2CProviderIds")
            || requests[0].contains("Fields=Path,ProductionYear,ProviderIds"),
        "{}",
        requests[0]
    );
}

async fn spawn_fake_emby_many(bodies: Vec<&'static str>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);

    tokio::spawn(async move {
        for body in bodies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            captured
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[..n]).to_string());

            let status = if body == "{}" {
                "204 No Content"
            } else {
                "200 OK"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });

    (format!("http://{addr}"), requests)
}

fn request_body(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap_or_default()
}
