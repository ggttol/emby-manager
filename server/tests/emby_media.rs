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
