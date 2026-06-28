use axum::{Json, extract::State};
use emby_manager::{
    db,
    error::AppError,
    media_fs::{self, ManageDeleteRequest, ManageMoveRequest},
    openapi::ApiDoc,
    settings::Settings,
    state::AppState,
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::{env, path::PathBuf, time::Duration};
use tempfile::TempDir;
use tokio::time::sleep;
use utoipa::OpenApi;
use uuid::Uuid;

#[tokio::test]
async fn delete_preview_creates_task_and_finishes_done() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping manage preview DB test; set EMBY_MANAGER_MANAGE_TEST_DATABASE_URL");
        return;
    };
    let req = ManageDeleteRequest {
        lib: "Movies".to_string(),
        folder: "A/Movie".to_string(),
        item_id: Some("item-delete".to_string()),
        reason: Some(format!("delete-preview-{}", Uuid::new_v4())),
    };

    let task = media_fs::preview_delete(State(state.clone()), Json(req.clone()))
        .await
        .expect("delete preview should create a task")
        .0;

    assert_eq!(task.kind, "manage_delete_preview");
    assert_eq!(task.source, "manual");
    assert_eq!(task.params["lib"], json!(req.lib));
    assert_eq!(task.params["folder"], json!(req.folder));

    let task = wait_for_task_status(&state, task.id, "done").await;
    assert_eq!(task["source"], "manual");
    assert_eq!(task["result"]["ok"], false);
    assert_eq!(task["result"]["preview"], true);
    assert_eq!(task["result"]["dry_run"], true);
    assert_eq!(task["result"]["operation"], "delete");
    assert!(
        task["result"]["planned_paths"][0]
            .as_str()
            .unwrap()
            .ends_with("Movies/A/Movie")
    );
    assert!(
        task["result"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("did not touch filesystem or Emby")
    );
}

#[tokio::test]
async fn move_preview_creates_task_and_finishes_done() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping manage preview DB test; set EMBY_MANAGER_MANAGE_TEST_DATABASE_URL");
        return;
    };
    let req = ManageMoveRequest {
        from_lib: "Movies".to_string(),
        from_folder: "A/Movie".to_string(),
        to_lib: "Archive".to_string(),
        to_folder: Some("Done/Movie".to_string()),
        item_id: Some("item-move".to_string()),
        reason: Some(format!("move-preview-{}", Uuid::new_v4())),
    };

    let task = media_fs::preview_move(State(state.clone()), Json(req.clone()))
        .await
        .expect("move preview should create a task")
        .0;

    assert_eq!(task.kind, "manage_move_preview");
    assert_eq!(task.source, "manual");
    assert_eq!(task.params["from_lib"], json!(req.from_lib));
    assert_eq!(task.params["to_folder"], json!(req.to_folder));

    let task = wait_for_task_status(&state, task.id, "done").await;
    assert_eq!(task["result"]["ok"], false);
    assert_eq!(task["result"]["preview"], true);
    assert_eq!(task["result"]["dry_run"], true);
    assert_eq!(task["result"]["operation"], "move");
    let planned = task["result"]["planned_paths"].as_array().unwrap();
    assert_eq!(planned.len(), 2);
    assert!(planned[0].as_str().unwrap().ends_with("Movies/A/Movie"));
    assert!(planned[1].as_str().unwrap().ends_with("Archive/Done/Movie"));
}

#[tokio::test]
async fn traversal_is_rejected_without_creating_task() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping manage preview DB test; set EMBY_MANAGER_MANAGE_TEST_DATABASE_URL");
        return;
    };
    let marker = format!("traversal-preview-{}", Uuid::new_v4());
    let req = ManageDeleteRequest {
        lib: "Movies".to_string(),
        folder: "../outside".to_string(),
        item_id: None,
        reason: Some(marker.clone()),
    };

    let err = media_fs::preview_delete(State(state.clone()), Json(req))
        .await
        .expect_err("path traversal should be rejected");

    assert!(matches!(err, AppError::BadRequest(_)));
    assert_eq!(task_count_for_reason(&state, &marker).await, 0);

    let marker = format!("move-traversal-preview-{}", Uuid::new_v4());
    let req = ManageMoveRequest {
        from_lib: "Movies".to_string(),
        from_folder: "A/Movie".to_string(),
        to_lib: "../Archive".to_string(),
        to_folder: Some("Done/Movie".to_string()),
        item_id: None,
        reason: Some(marker.clone()),
    };

    let err = media_fs::preview_move(State(state.clone()), Json(req))
        .await
        .expect_err("move path traversal should be rejected");

    assert!(matches!(err, AppError::BadRequest(_)));
    assert_eq!(task_count_for_reason(&state, &marker).await, 0);
}

#[test]
fn openapi_registers_manage_preview_paths_and_schemas() {
    let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    assert!(paths.contains_key("/api/v2/manage/delete"));
    assert!(paths.contains_key("/api/v2/manage/delete/execute"));
    assert!(paths.contains_key("/api/v2/manage/move"));

    let schemas = doc["components"]["schemas"].as_object().unwrap();
    assert!(schemas.contains_key("ManageDeleteRequest"));
    assert!(schemas.contains_key("ManageDeleteExecuteResult"));
    assert!(schemas.contains_key("ManageMoveRequest"));
    assert!(schemas.contains_key("ManagePreviewResult"));
}

async fn wait_for_task_status(state: &AppState, id: Uuid, status: &str) -> Value {
    for _ in 0..50 {
        let task: Value =
            sqlx::query_scalar("SELECT to_jsonb(task_runs) FROM task_runs WHERE id = $1")
                .bind(id)
                .fetch_one(&state.pool)
                .await
                .expect("load task");
        if task["status"] == status {
            return task;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("task {id} did not reach {status}");
}

async fn task_count_for_reason(state: &AppState, reason: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM task_runs WHERE params->>'reason' = $1")
        .bind(reason)
        .fetch_one(&state.pool)
        .await
        .expect("count tasks")
}

async fn test_state() -> Option<(TempDir, AppState)> {
    let database_url = manage_test_database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect manage test database");
    db::migrate(&pool)
        .await
        .expect("run manage test migrations");
    let tmp = tempfile::tempdir().unwrap();
    let strm_root = tmp.path().join("strm");
    std::fs::create_dir_all(&strm_root).unwrap();
    let settings = test_settings(database_url, strm_root);
    Some((tmp, AppState::new(pool, settings)))
}

fn manage_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_MANAGE_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn test_settings(database_url: String, strm_root: PathBuf) -> Settings {
    Settings {
        host: "127.0.0.1".to_string(),
        port: 0,
        database_url,
        web_dist: PathBuf::from("/tmp"),
        legacy_dir: PathBuf::from("/tmp"),
        bootstrap_password: "admin".to_string(),
        cd_root: PathBuf::from("/tmp/cd"),
        strm_root,
        docker_bin: PathBuf::from("/usr/bin/docker"),
        task_concurrency: 1,
    }
}
