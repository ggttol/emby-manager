use emby_manager::{
    c115::{C115CidMatch, C115Client, C115OfflineRequest, C115SaveRequest, validate_target_cid},
    openapi::ApiDoc,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::{Duration, timeout},
};
use utoipa::OpenApi;

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
