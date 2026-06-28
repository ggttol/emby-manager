use emby_manager::{
    c115::{C115Client, C115SnapFile, C115SnapRequest},
    openapi::ApiDoc,
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
async fn snap_paginates_and_normalizes_share_files() {
    let first = r#"{
        "state": true,
        "data": {
            "total": 3,
            "shareinfo": {"share_title": "Share Title", "file_size": 1050624},
            "list": [
                {"fid": "file-a", "n": "Movie.mkv", "s": "1048576"},
                {"cid": 9001, "n": "Season Folder", "size": 0}
            ]
        }
    }"#;
    let second = r#"{
        "state": true,
        "data": {
            "total": 3,
            "list": [
                {"file_id": 12345, "name": "Episode 02.mkv", "s": 2048}
            ]
        }
    }"#;
    let (base_url, requests, handle) = spawn_fake_c115(vec![first, second]).await;
    let client = C115Client::new(
        base_url,
        "CID=cid-value; UID=123456_A1; SEID=seid-value",
        reqwest::Client::new(),
    );

    let result = client
        .snap(C115SnapRequest {
            url: "https://115.com/s/swABC?password=YYY#anchor".to_string(),
            pwd: None,
            file_ids: None,
        })
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(result.share, "swABC");
    assert_eq!(result.rc.as_deref(), Some("YYY"));
    assert_eq!(result.share_title.as_deref(), Some("Share Title"));
    assert_eq!(
        result.files,
        vec![
            C115SnapFile {
                id: Some("file-a".to_string()),
                name: "Movie.mkv".to_string(),
                size: 1048576,
                is_dir: false,
            },
            C115SnapFile {
                id: Some("9001".to_string()),
                name: "Season Folder".to_string(),
                size: 0,
                is_dir: true,
            },
            C115SnapFile {
                id: Some("12345".to_string()),
                name: "Episode 02.mkv".to_string(),
                size: 2048,
                is_dir: false,
            },
        ]
    );

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0].starts_with("GET /share/snap?"),
        "{}",
        requests[0]
    );
    assert!(requests[0].contains("share_code=swABC"), "{}", requests[0]);
    assert!(requests[0].contains("receive_code=YYY"), "{}", requests[0]);
    assert!(requests[0].contains("cid=0"), "{}", requests[0]);
    assert!(requests[0].contains("offset=0"), "{}", requests[0]);
    assert!(requests[0].contains("limit=1000"), "{}", requests[0]);
    assert!(requests[1].contains("offset=2"), "{}", requests[1]);
    let lower = requests[0].to_ascii_lowercase();
    assert!(lower.contains("cookie: cid=cid-value; uid=123456_a1; seid=seid-value"));
    assert!(lower.contains("referer: https://115.com/"));
}

#[tokio::test]
async fn snap_filters_requested_file_ids_and_pwd_overrides_url() {
    let body = r#"{
        "state": true,
        "data": {
            "count": 2,
            "shareinfo": {"file_name": "Manual Codes"},
            "list": [
                {"fid": "keep", "n": "Keep.mkv", "s": 1},
                {"fid": "drop", "n": "Drop.mkv", "s": 2}
            ]
        }
    }"#;
    let (base_url, requests, handle) = spawn_fake_c115(vec![body]).await;
    let client = C115Client::new(base_url, "UID=123456_A1", reqwest::Client::new());

    let result = client
        .snap(C115SnapRequest {
            url: "swXYZ old_pwd".to_string(),
            pwd: Some(" NEWPWD ".to_string()),
            file_ids: Some(vec!["keep".to_string()]),
        })
        .await
        .unwrap();

    assert_eq!(result.share, "swXYZ");
    assert_eq!(result.rc.as_deref(), Some("NEWPWD"));
    assert_eq!(result.share_title.as_deref(), Some("Manual Codes"));
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].id.as_deref(), Some("keep"));
    assert_eq!(result.files[0].name, "Keep.mkv");

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("share_code=swXYZ"), "{}", requests[0]);
    assert!(
        requests[0].contains("receive_code=NEWPWD"),
        "{}",
        requests[0]
    );
}

#[test]
fn openapi_registers_c115_snap() {
    let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    assert!(paths.contains_key("/api/v2/c115/snap"));

    let schemas = doc["components"]["schemas"].as_object().unwrap();
    assert!(schemas.contains_key("C115SnapRequest"));
    assert!(schemas.contains_key("C115SnapResponse"));
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
            let mut buf = vec![0; 4096];
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
