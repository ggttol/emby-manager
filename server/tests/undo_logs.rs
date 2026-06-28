use axum::{Json, extract::State};
use chrono::{TimeZone, Utc};
use emby_manager::{db, logs, openapi::ApiDoc, settings::Settings, state::AppState, undo};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::{env, path::PathBuf};
use tempfile::TempDir;
use utoipa::OpenApi;
use uuid::Uuid;

#[test]
fn undo_limit_defaults_and_clamps() {
    assert_eq!(undo::UndoListQuery { limit: None }.limit(), 50);
    assert_eq!(undo::UndoListQuery { limit: Some(0) }.limit(), 1);
    assert_eq!(undo::UndoListQuery { limit: Some(500) }.limit(), 200);
}

#[test]
fn log_query_normalizes_level_and_clamps_limit() {
    let query = logs::LogListQuery {
        limit: Some(0),
        level: Some(" WARN ".to_string()),
    };
    assert_eq!(query.limit(), 1);
    assert_eq!(query.normalized_level().as_deref(), Some("warn"));

    let blank = logs::LogListQuery {
        limit: Some(999),
        level: Some("   ".to_string()),
    };
    assert_eq!(blank.limit(), 500);
    assert_eq!(blank.normalized_level(), None);
}

#[test]
fn undo_and_log_entries_serialize_expected_fields() {
    let created_at = Utc.with_ymd_and_hms(2026, 6, 28, 1, 2, 3).unwrap();
    let undo_entry = undo::UndoEntry {
        id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
        legacy_id: Some("deadbeef".to_string()),
        op: "move".to_string(),
        payload: json!({"folder": "Movie", "from": "A", "to": "B"}),
        undone: false,
        created_at,
    };
    let log_entry = logs::AppLogEntry {
        id: 7,
        level: "info".to_string(),
        message: "scan done".to_string(),
        detail: json!({"count": 3}),
        created_at,
    };

    let undo_json = serde_json::to_value(undo_entry).unwrap();
    assert_eq!(undo_json["legacy_id"], "deadbeef");
    assert_eq!(undo_json["payload"]["folder"], "Movie");
    assert_eq!(undo_json["undone"], false);

    let log_json = serde_json::to_value(log_entry).unwrap();
    assert_eq!(log_json["id"], 7);
    assert_eq!(log_json["level"], "info");
    assert_eq!(log_json["detail"]["count"], 3);
}

#[test]
fn undo_execute_delete_returns_manual_restore_guidance() {
    let entry = undo_entry(
        "delete",
        json!({"lib": "Movies", "folder": "Movie A"}),
        false,
    );

    let response = undo::build_execute_response(&entry);

    assert!(!response.ok);
    assert_eq!(response.action, undo::UndoExecuteAction::ManualRestore);
    assert_eq!(response.lib.as_deref(), Some("Movies"));
    assert_eq!(response.folder.as_deref(), Some("Movie A"));
    assert!(response.msg.contains("115"));
    assert!(response.hint.unwrap().contains("Movie A"));
}

#[test]
fn undo_execute_move_with_payload_guides_execution() {
    let entry = undo_entry(
        "move",
        json!({"folder": "Show S01", "from": "Old", "to": "New", "to_folder": "Done/Show S01"}),
        false,
    );

    let response = undo::build_execute_response(&entry);

    assert!(!response.ok);
    assert_eq!(response.action, undo::UndoExecuteAction::ManualRestore);
    assert_eq!(response.lib.as_deref(), Some("New -> Old"));
    assert_eq!(response.folder.as_deref(), Some("Show S01"));
    assert!(response.msg.contains("执行接口"));
}

#[test]
fn undo_execute_move_missing_paths_is_unsupported() {
    let entry = undo_entry(
        "move",
        json!({"folder": "Show S01", "from": "Old", "to": "New"}),
        false,
    );

    let response = undo::build_execute_response(&entry);

    assert!(!response.ok);
    assert_eq!(response.action, undo::UndoExecuteAction::Unsupported);
    assert!(response.msg.contains("to_folder"));
}

#[test]
fn undo_execute_respects_already_undone_entries() {
    let entry = undo_entry("delete", json!({"folder": "Movie B"}), true);

    let response = undo::build_execute_response(&entry);

    assert!(!response.ok);
    assert_eq!(response.action, undo::UndoExecuteAction::AlreadyUndone);
    assert!(response.msg.contains("已经撤销"));
}

#[test]
fn routers_and_openapi_include_logs_and_undo() {
    let _undo_router = undo::router();
    let _logs_router = logs::router();

    let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    assert!(paths.contains_key("/api/v2/manage/undo"));
    assert!(paths.contains_key("/api/v2/manage/undo/execute"));
    assert!(paths.contains_key("/api/v2/logs"));

    let schemas = doc["components"]["schemas"].as_object().unwrap();
    assert!(schemas.contains_key("UndoExecuteRequest"));
    assert!(schemas.contains_key("UndoExecuteResponse"));
    assert!(schemas.contains_key("UndoExecuteAction"));
}

#[tokio::test]
async fn undo_execute_move_reverses_cd_and_strm_and_marks_undone() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping undo DB test; set EMBY_MANAGER_UNDO_TEST_DATABASE_URL");
        return;
    };
    let id = Uuid::new_v4();
    let current_cd = state.settings.cd_root.join("Archive/Done/Movie");
    let current_strm = state.settings.strm_root.join("Archive/Done/Movie");
    let restore_cd = state.settings.cd_root.join("Movies/A/Movie");
    let restore_strm = state.settings.strm_root.join("Movies/A/Movie");
    std::fs::create_dir_all(&current_cd).unwrap();
    std::fs::create_dir_all(&current_strm).unwrap();
    std::fs::write(current_cd.join("movie.mkv"), "media").unwrap();
    std::fs::write(current_strm.join("movie.strm"), "strm").unwrap();
    insert_move_undo(
        &state,
        id,
        json!({
            "from": "Movies",
            "to": "Archive",
            "folder": "A/Movie",
            "to_folder": "Done/Movie"
        }),
    )
    .await;

    let response = undo::exec_undo(State(state.clone()), Json(undo::UndoExecuteRequest { id }))
        .await
        .expect("move undo should execute")
        .0;

    assert!(response.ok);
    assert_eq!(response.action, undo::UndoExecuteAction::Executed);
    assert!(restore_cd.join("movie.mkv").is_file());
    assert!(restore_strm.join("movie.strm").is_file());
    assert!(!current_cd.exists());
    assert!(!current_strm.exists());
    assert!(undo_marked(&state, id).await);

    let response = undo::exec_undo(State(state.clone()), Json(undo::UndoExecuteRequest { id }))
        .await
        .expect("second move undo should be handled")
        .0;
    assert!(!response.ok);
    assert_eq!(response.action, undo::UndoExecuteAction::AlreadyUndone);
}

#[tokio::test]
async fn undo_execute_move_target_conflict_returns_manual_without_marking_undone() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping undo DB test; set EMBY_MANAGER_UNDO_TEST_DATABASE_URL");
        return;
    };
    let id = Uuid::new_v4();
    let current_cd = state.settings.cd_root.join("Archive/Done/Movie");
    let restore_cd = state.settings.cd_root.join("Movies/A/Movie");
    std::fs::create_dir_all(&current_cd).unwrap();
    std::fs::create_dir_all(&restore_cd).unwrap();
    insert_move_undo(
        &state,
        id,
        json!({
            "from": "Movies",
            "to": "Archive",
            "folder": "A/Movie",
            "to_folder": "Done/Movie"
        }),
    )
    .await;

    let response = undo::exec_undo(State(state.clone()), Json(undo::UndoExecuteRequest { id }))
        .await
        .expect("move undo conflict should return guidance")
        .0;

    assert!(!response.ok);
    assert_eq!(response.action, undo::UndoExecuteAction::ManualRestore);
    assert!(response.msg.contains("已存在"));
    assert!(current_cd.exists());
    assert!(restore_cd.exists());
    assert!(!undo_marked(&state, id).await);
}

#[tokio::test]
async fn undo_execute_move_rejects_path_escape_without_marking_undone() {
    let Some((_tmp, state)) = test_state().await else {
        eprintln!("skipping undo DB test; set EMBY_MANAGER_UNDO_TEST_DATABASE_URL");
        return;
    };
    let id = Uuid::new_v4();
    insert_move_undo(
        &state,
        id,
        json!({
            "from": "Movies",
            "to": "Archive",
            "folder": "../A/Movie",
            "to_folder": "Done/Movie"
        }),
    )
    .await;

    let response = undo::exec_undo(State(state.clone()), Json(undo::UndoExecuteRequest { id }))
        .await
        .expect("unsafe move undo should return unsupported")
        .0;

    assert!(!response.ok);
    assert_eq!(response.action, undo::UndoExecuteAction::Unsupported);
    assert!(response.msg.contains("非法路径"));
    assert!(!undo_marked(&state, id).await);
}

fn undo_entry(op: &str, payload: serde_json::Value, undone: bool) -> undo::UndoEntry {
    undo::UndoEntry {
        id: Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
        legacy_id: None,
        op: op.to_string(),
        payload,
        undone,
        created_at: Utc.with_ymd_and_hms(2026, 6, 28, 4, 5, 6).unwrap(),
    }
}

async fn insert_move_undo(state: &AppState, id: Uuid, payload: serde_json::Value) {
    sqlx::query("INSERT INTO undo_entries(id, op, payload) VALUES ($1, 'move', $2)")
        .bind(id)
        .bind(payload)
        .execute(&state.pool)
        .await
        .expect("insert move undo");
}

async fn undo_marked(state: &AppState, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT undone FROM undo_entries WHERE id = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .expect("load undo flag")
}

async fn test_state() -> Option<(TempDir, AppState)> {
    let database_url = undo_test_database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect undo test database");
    db::migrate(&pool).await.expect("run undo test migrations");
    let tmp = tempfile::tempdir().unwrap();
    let cd_root = tmp.path().join("cd");
    let strm_root = tmp.path().join("strm");
    std::fs::create_dir_all(&cd_root).unwrap();
    std::fs::create_dir_all(&strm_root).unwrap();
    let settings = test_settings(database_url, cd_root, strm_root);
    Some((tmp, AppState::new(pool, settings)))
}

fn undo_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_UNDO_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn test_settings(database_url: String, cd_root: PathBuf, strm_root: PathBuf) -> Settings {
    Settings {
        host: "127.0.0.1".to_string(),
        port: 0,
        database_url,
        web_dist: PathBuf::from("/tmp"),
        legacy_dir: PathBuf::from("/tmp"),
        bootstrap_password: "admin".to_string(),
        cd_root,
        strm_root,
        docker_bin: PathBuf::from("/usr/bin/docker"),
        task_concurrency: 1,
    }
}
