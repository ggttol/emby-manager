use emby_manager::c115::{self, C115Client};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

#[test]
fn missing_cookie_reports_setting_key() {
    let err = c115::require_c115_cookie(None).unwrap_err();
    assert!(err.to_string().contains("未设置 115 cookie"));
    assert!(err.to_string().contains("c115_cookie"));

    let err = c115::require_c115_cookie(Some("   ".to_string())).unwrap_err();
    assert!(err.to_string().contains("c115_cookie"));
}

#[test]
fn parse_url_keeps_share_and_receive_code_behavior() {
    assert_eq!(
        c115::parse_115_url(" https://115.com/s/swABC?password=YYY#anchor ", None),
        (Some("swABC".to_string()), Some("YYY".to_string()))
    );
    assert_eq!(
        c115::parse_115_url("https://115cdn.com/s/swXYZ?pwd=ABC", Some(" OVERRIDE ")),
        (Some("swXYZ".to_string()), Some("OVERRIDE".to_string()))
    );
    assert_eq!(
        c115::parse_115_url("swABC YYY", None),
        (Some("swABC".to_string()), Some("YYY".to_string()))
    );
    assert_eq!(c115::parse_115_url("!!!@@@", None), (None, None));
}

#[tokio::test]
async fn client_tests_cookie_against_fake_server() {
    let body = r#"{"state":true,"data":{"space_info":{"all_total":{"size_format":"1.5 TB"}}}}"#;
    let (base_url, request_handle) = spawn_fake_c115(body).await;
    let cookie = "CID=cid-value; UID=123456_A1; SEID=seid-value";
    let client = C115Client::new(base_url, cookie, reqwest::Client::new());

    let result = client.test_cookie().await.unwrap();

    assert!(result.ok);
    assert_eq!(result.uid, "123456");
    assert_eq!(result.used, "1.5 TB");

    let request = request_handle.await.unwrap();
    assert!(request.starts_with("GET /files/index_info "));
    let lower = request.to_ascii_lowercase();
    assert!(lower.contains("cookie: cid=cid-value; uid=123456_a1; seid=seid-value"));
    assert!(lower.contains("referer: https://115.com/"));
}

#[tokio::test]
async fn client_deletes_child_dir_by_name_with_rb_delete() {
    let responses = vec![
        r#"{"state":true,"data":[{"cid":"456","n":"Movie Folder"}]}"#,
        r#"{"state":true}"#,
    ];
    let (base_url, request_handle) = spawn_fake_c115_sequence(responses).await;
    let cookie = "CID=cid-value; UID=123456_A1; SEID=seid-value";
    let client = C115Client::new(base_url, cookie, reqwest::Client::new());

    let deleted = client
        .delete_child_dir("123", "Movie Folder")
        .await
        .unwrap();

    assert!(deleted);
    let requests = request_handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /files?"), "{}", requests[0]);
    assert!(requests[0].contains("cid=123"), "{}", requests[0]);
    assert!(
        requests[1].starts_with("POST /rb/delete "),
        "{}",
        requests[1]
    );
    let body = request_body(&requests[1]);
    assert!(body.contains("pid=123"), "{body}");
    assert!(body.contains("fid%5B0%5D=456"), "{body}");
    assert!(body.contains("ignore_warn=1"), "{body}");
}

async fn spawn_fake_c115(body: &'static str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_string();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        request
    });
    (format!("http://{addr}"), handle)
}

async fn spawn_fake_c115_sequence(
    responses: Vec<&'static str>,
) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut requests = Vec::new();
        for body in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            requests.push(String::from_utf8_lossy(&buf[..n]).to_string());
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    (format!("http://{addr}"), handle)
}

fn request_body(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap_or_default()
}
