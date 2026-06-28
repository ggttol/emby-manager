use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::CONTENT_TYPE},
};
use emby_manager::{catalog, db, settings::Settings, state::AppState};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, path::PathBuf};
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

#[tokio::test]
async fn duplicates_endpoint_returns_limited_readonly_catalog_groups() {
    let Some(database_url) = catalog_test_database_url() else {
        eprintln!(
            "skipping catalog duplicates DB test; set EMBY_MANAGER_CATALOG_TEST_DATABASE_URL"
        );
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect catalog test database");
    db::migrate(&pool)
        .await
        .expect("run catalog test migrations");
    reset_catalog_items(&pool).await;

    let app = catalog::router().with_state(test_state_with_pool(pool));
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v2/catalog/duplicates?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["readonly"], true);
    assert_eq!(body["limit"], 1);
    assert_eq!(body["duplicate_link_groups"], 2);
    assert_eq!(body["duplicate_name_groups"], 2);
    assert_eq!(body["link_groups"].as_array().unwrap().len(), 1);
    assert_eq!(body["name_groups"].as_array().unwrap().len(), 1);
    assert_eq!(body["link_groups"][0]["key"], "https://115.com/s/shared");
    assert_eq!(body["link_groups"][0]["count"], 3);
    assert!(
        body["link_type_distribution"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["link_type"] == "share115" && row["count"] == 5),
        "{body}"
    );
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

async fn reset_catalog_items(pool: &PgPool) {
    sqlx::query("TRUNCATE catalog_items RESTART IDENTITY")
        .execute(pool)
        .await
        .expect("reset catalog items");
    sqlx::query(
        "INSERT INTO catalog_items(name, sheet, link, is_pkg, link_type)
         VALUES
            ('Show A 1080p', '剧集', 'https://115.com/s/shared', false, 'share115'),
            ('Show A 4K', '剧集', 'https://115.com/s/shared', false, 'share115'),
            ('Show A Remux', '剧集', 'https://115.com/s/shared', true, 'share115'),
            ('Same Name', '电影', 'https://115.com/s/name-1', false, 'share115'),
            ('Same Name', '电影', 'magnet:?xt=urn:btih:name2', false, 'magnet'),
            ('Other Same', '电影', 'https://115.com/s/name-3', false, 'share115'),
            ('Other Same', '电影', 'ed2k://|file|other.mkv|1|hash|/', false, 'ed2k'),
            ('Unique', '电影', 'https://115.com/s/unique', false, 'share115'),
            ('Duplicate Link Two A', '电影', 'magnet:?xt=urn:btih:duplink2', false, 'magnet'),
            ('Duplicate Link Two B', '电影', 'magnet:?xt=urn:btih:duplink2', false, 'magnet')",
    )
    .execute(pool)
    .await
    .expect("seed catalog duplicates");
}

fn test_state() -> AppState {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://emby_manager:emby_manager@127.0.0.1:1/emby_manager_test")
        .unwrap();
    test_state_with_pool(pool)
}

fn test_state_with_pool(pool: PgPool) -> AppState {
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

fn catalog_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_CATALOG_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}
