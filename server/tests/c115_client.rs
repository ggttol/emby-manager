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
