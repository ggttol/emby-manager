use emby_manager::{
    emby::EmbyClient,
    error::AppError,
    openapi::ApiDoc,
    users::{
        CreateUserRequest, UpdateUserPolicyRequest, create_user_with_client,
        delete_user_with_client, list_users_with_client, update_user_policy_with_client,
    },
};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::{Duration, timeout},
};
use utoipa::OpenApi;

#[tokio::test]
async fn lists_users_from_fake_emby() {
    let users = r#"[
        {
            "Id": "u1",
            "Name": "Alice",
            "LastActivityDate": "2026-06-28T10:00:00Z",
            "Policy": {
                "IsDisabled": false,
                "RemoteClientBitrateLimit": 25000000,
                "SimultaneousStreamLimit": 2
            }
        },
        {
            "Id": "u2",
            "Name": "Bob",
            "Policy": {"IsDisabled": true}
        }
    ]"#;
    let (base_url, requests, handle) = spawn_fake_emby(vec![fake_response(200, users)]).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let response = list_users_with_client(&client).await.unwrap();

    assert_eq!(response.users.len(), 2);
    assert_eq!(response.users[0].id, "u1");
    assert_eq!(response.users[0].name, "Alice");
    assert!(!response.users[0].admin);
    assert!(!response.users[0].disabled);
    assert_eq!(
        response.users[0].last_activity_date.as_deref(),
        Some("2026-06-28T10:00:00Z")
    );
    assert_eq!(
        response.users[0].policy.remote_client_bitrate_limit,
        Some(25_000_000)
    );
    assert_eq!(response.users[0].policy.simultaneous_stream_limit, Some(2));
    assert_eq!(response.users[0].remote_bitrate_mbps, Some(25.0));
    assert!(response.users[1].disabled);

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("GET /Users?api_key=secret-key HTTP/1.1"),
        "{}",
        requests[0]
    );
}

#[tokio::test]
async fn creates_user_and_sets_password_after_readback() {
    let before = r#"[
        {"Id": "u1", "Name": "Alice", "Policy": {"IsAdministrator": false}}
    ]"#;
    let after = r#"[
        {"Id": "u1", "Name": "Alice", "Policy": {"IsAdministrator": false}},
        {"Id": "user/3", "Name": "Carol", "Policy": {"IsAdministrator": false, "IsDisabled": false}}
    ]"#;
    let (base_url, requests, handle) = spawn_fake_emby(vec![
        fake_response(200, before),
        fake_response(204, ""),
        fake_response(200, after),
        fake_response(204, ""),
    ])
    .await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let response = create_user_with_client(
        &client,
        CreateUserRequest {
            name: "  Carol  ".to_string(),
            password: Some("pw123".to_string()),
        },
    )
    .await
    .unwrap();

    assert!(response.ok);
    assert_eq!(response.user.id, "user/3");
    assert_eq!(response.user.name, "Carol");
    assert!(!response.user.admin);

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(
        requests[0].starts_with("GET /Users?api_key=secret-key "),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].starts_with("POST /Users/New?api_key=secret-key "),
        "{}",
        requests[1]
    );
    assert!(
        requests[2].starts_with("GET /Users?api_key=secret-key "),
        "{}",
        requests[2]
    );
    assert!(
        requests[3].starts_with("POST /Users/user%2F3/Password?api_key=secret-key "),
        "{}",
        requests[3]
    );
    let create_payload: Value = serde_json::from_str(request_body(&requests[1])).unwrap();
    assert_eq!(create_payload["Name"], "Carol");
    let password_payload: Value = serde_json::from_str(request_body(&requests[3])).unwrap();
    assert_eq!(password_payload["Id"], "user/3");
    assert_eq!(password_payload["CurrentPw"], "");
    assert_eq!(password_payload["NewPw"], "pw123");
}

#[tokio::test]
async fn create_user_rejects_empty_or_duplicate_name() {
    let (base_url, requests, handle) = spawn_fake_emby(vec![fake_response(
        200,
        r#"[{"Id":"u1","Name":"Alice","Policy":{}}]"#,
    )])
    .await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let err = create_user_with_client(
        &client,
        CreateUserRequest {
            name: " ".to_string(),
            password: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::BadRequest(_)));

    let err = create_user_with_client(
        &client,
        CreateUserRequest {
            name: "Alice".to_string(),
            password: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::Conflict(_)));

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn deletes_non_admin_user_with_encoded_id() {
    let user = r#"{
        "Id": "user/3",
        "Name": "Carol",
        "Policy": {"IsAdministrator": false}
    }"#;
    let (base_url, requests, handle) =
        spawn_fake_emby(vec![fake_response(200, user), fake_response(204, "")]).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let response = delete_user_with_client(&client, "user/3").await.unwrap();

    assert!(response.ok);
    assert_eq!(response.code, 204);

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0].starts_with("GET /Users/user%2F3?api_key=secret-key "),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].starts_with("DELETE /Users/user%2F3?api_key=secret-key "),
        "{}",
        requests[1]
    );
}

#[tokio::test]
async fn delete_user_rejects_admin_before_delete() {
    let user = r#"{
        "Id": "admin/1",
        "Name": "Admin",
        "Policy": {"IsAdministrator": true}
    }"#;
    let (base_url, requests, handle) = spawn_fake_emby(vec![fake_response(200, user)]).await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let err = delete_user_with_client(&client, "admin/1")
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::BadRequest(_)));
    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("GET /Users/admin%2F1?api_key=secret-key "),
        "{}",
        requests[0]
    );
}

#[tokio::test]
async fn updates_user_policy_after_reading_current_policy() {
    let before = r#"{
        "Id": "user/1",
        "Name": "Alice",
        "Policy": {
            "IsDisabled": false,
            "RemoteClientBitrateLimit": 5000000,
            "SimultaneousStreamLimit": 1,
            "EnableAllDevices": true
        }
    }"#;
    let after = r#"{
        "Id": "user/1",
        "Name": "Alice",
        "Policy": {
            "IsDisabled": true,
            "RemoteClientBitrateLimit": 12500000,
            "SimultaneousStreamLimit": 3,
            "EnableAllDevices": true
        }
    }"#;
    let (base_url, requests, handle) = spawn_fake_emby(vec![
        fake_response(200, before),
        fake_response(204, ""),
        fake_response(200, after),
    ])
    .await;
    let client = EmbyClient::new(base_url, "secret-key", reqwest::Client::new());

    let response = update_user_policy_with_client(
        &client,
        "user/1",
        UpdateUserPolicyRequest {
            remote_bitrate_mbps: Some(12.5),
            simultaneous_stream_limit: Some(3),
            disabled: Some(true),
        },
    )
    .await
    .unwrap();

    assert!(response.ok);
    assert!(response.user.disabled);
    assert_eq!(
        response.user.policy.remote_client_bitrate_limit,
        Some(12_500_000)
    );
    assert_eq!(response.user.policy.simultaneous_stream_limit, Some(3));
    assert_eq!(response.user.remote_bitrate_mbps, Some(12.5));

    timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0].starts_with("GET /Users/user%2F1?api_key=secret-key "),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].starts_with("POST /Users/user%2F1/Policy?api_key=secret-key "),
        "{}",
        requests[1]
    );
    assert!(
        requests[2].starts_with("GET /Users/user%2F1?api_key=secret-key "),
        "{}",
        requests[2]
    );

    let payload: Value = serde_json::from_str(request_body(&requests[1])).unwrap();
    assert_eq!(payload["RemoteClientBitrateLimit"], 12_500_000);
    assert_eq!(payload["SimultaneousStreamLimit"], 3);
    assert_eq!(payload["IsDisabled"], true);
    assert_eq!(payload["EnableAllDevices"], true);
    assert!(payload.get("MaxActiveSessions").is_none(), "{payload}");
}

#[test]
fn openapi_registers_user_policy_routes() {
    let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    assert!(paths.contains_key("/api/v2/users"));
    assert!(paths.contains_key("/api/v2/users/{id}"));
    assert!(paths.contains_key("/api/v2/users/{id}/policy"));

    let schemas = doc["components"]["schemas"].as_object().unwrap();
    assert!(schemas.contains_key("CreateUserRequest"));
    assert!(schemas.contains_key("CreateUserResponse"));
    assert!(schemas.contains_key("DeleteUserResponse"));
    assert!(schemas.contains_key("UsersResponse"));
    assert!(schemas.contains_key("UserSummary"));
    assert!(schemas.contains_key("UserPolicySummary"));
    assert!(schemas.contains_key("UpdateUserPolicyRequest"));
    assert!(schemas.contains_key("UpdateUserPolicyResponse"));
}

fn fake_response(status: u16, body: &'static str) -> FakeResponse {
    FakeResponse { status, body }
}

struct FakeResponse {
    status: u16,
    body: &'static str,
}

async fn spawn_fake_emby(
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
