use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::CONTENT_TYPE},
};
use emby_manager::{catalog, settings::Settings, state::AppState};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::path::PathBuf;
use tower::ServiceExt;

#[tokio::test]
async fn transfer_plan_routes_share115_to_c115_save() {
    let link = "https://115cdn.com/s/swABC?password=YYY#frag";
    let (status, body) = post_transfer_plan(json!({
        "item": {
            "name": "沙丘 4K",
            "sheet": "电影",
            "link": link,
            "is_pkg": true,
            "link_type": "share115"
        },
        "lib": "电影"
    }))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["action"], "save_share");
    assert_eq!(body["link_type"], "share115");
    assert_eq!(body["transfer"], true);
    assert_eq!(body["is_pkg"], true);
    assert_eq!(body["target"]["lib"], "电影");
    assert_eq!(body["label"], "沙丘 4K");
    assert_eq!(body["save"]["endpoint"], "/api/v2/c115/save");
    assert_eq!(body["save"]["method"], "POST");
    assert_eq!(body["save"]["share"], "swABC");
    assert_eq!(body["save"]["receive_code"], "YYY");
    assert_eq!(body["save"]["payload"]["url"], link);
    assert_eq!(body["save"]["payload"]["pwd"], "YYY");
    assert_eq!(body["save"]["payload"]["lib"], "电影");
    assert!(body.get("offline").is_none(), "{body}");
    assert!(body.get("unsupported").is_none(), "{body}");
}

#[tokio::test]
async fn transfer_plan_routes_magnet_to_c115_offline() {
    let link = "magnet:?xt=urn:btih:0123456789abcdef";
    let (status, body) = post_transfer_plan(json!({
        "link": link,
        "label": "Ubuntu ISO",
        "cid": "123456"
    }))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["action"], "offline_download");
    assert_eq!(body["link_type"], "magnet");
    assert_eq!(body["transfer"], false);
    assert_eq!(body["target"]["cid"], "123456");
    assert_eq!(body["offline"]["endpoint"], "/api/v2/c115/offline");
    assert_eq!(body["offline"]["protocol"], "magnet");
    assert_eq!(body["offline"]["payload"]["url"], link);
    assert_eq!(body["offline"]["payload"]["cid"], "123456");
    assert_eq!(body["offline"]["payload"]["label"], "Ubuntu ISO");
    assert!(body.get("save").is_none(), "{body}");
}

#[tokio::test]
async fn transfer_plan_routes_ed2k_to_c115_offline() {
    let link = "ed2k://|file|movie.mkv|123|abcdef|/";
    let (status, body) = post_transfer_plan(json!({
        "item": {
            "name": "movie.mkv",
            "link": link,
            "link_type": "ed2k"
        },
        "lib": "电视剧"
    }))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["action"], "offline_download");
    assert_eq!(body["link_type"], "ed2k");
    assert_eq!(body["target"]["lib"], "电视剧");
    assert_eq!(body["offline"]["endpoint"], "/api/v2/c115/offline");
    assert_eq!(body["offline"]["protocol"], "ed2k");
    assert_eq!(body["offline"]["payload"]["url"], link);
    assert_eq!(body["offline"]["payload"]["lib"], "电视剧");
}

#[tokio::test]
async fn transfer_plan_marks_other_links_unsupported() {
    let link = "https://example.com/file.torrent";
    let (status, body) = post_transfer_plan(json!({
        "link": link,
        "link_type": "other",
        "cid": "123456"
    }))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], false);
    assert_eq!(body["action"], "unsupported");
    assert_eq!(body["link_type"], "other");
    assert_eq!(body["transfer"], false);
    assert_eq!(body["target"]["cid"], "123456");
    assert_eq!(body["unsupported"]["link"], link);
    assert!(
        body["unsupported"]["reason"]
            .as_str()
            .unwrap()
            .contains("not supported"),
        "{body}"
    );
    assert!(body.get("save").is_none(), "{body}");
    assert!(body.get("offline").is_none(), "{body}");
}

async fn post_transfer_plan(body: Value) -> (StatusCode, Value) {
    let app = catalog::router().with_state(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v2/catalog/transfer-plan")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, body)
}

fn test_state() -> AppState {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://emby_manager:emby_manager@127.0.0.1:1/emby_manager_test")
        .unwrap();
    AppState::new(
        pool,
        Settings {
            host: "127.0.0.1".to_string(),
            port: 0,
            database_url: "postgres://emby_manager:emby_manager@127.0.0.1:1/emby_manager_test"
                .to_string(),
            web_dist: PathBuf::from("/tmp"),
            legacy_dir: PathBuf::from("/tmp"),
            bootstrap_password: "admin".to_string(),
            cd_root: PathBuf::from("/tmp/cd"),
            strm_root: PathBuf::from("/tmp/strm"),
            docker_bin: PathBuf::from("/usr/bin/docker"),
            task_concurrency: 1,
        },
    )
}
