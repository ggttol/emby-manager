use chrono::{TimeZone, Utc};
use emby_manager::{logs, openapi::ApiDoc, undo};
use serde_json::json;
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
fn undo_execute_move_is_explicitly_pending_port() {
    let entry = undo_entry(
        "move",
        json!({"folder": "Show S01", "from": "Old", "to": "New"}),
        false,
    );

    let response = undo::build_execute_response(&entry);

    assert!(!response.ok);
    assert_eq!(response.action, undo::UndoExecuteAction::PendingPort);
    assert_eq!(response.lib.as_deref(), Some("Old"));
    assert_eq!(response.folder.as_deref(), Some("Show S01"));
    assert!(response.msg.contains("move undo"));
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
