use emby_manager::{db, migrate};
use rusqlite::Connection;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, fs, path::Path, sync::Mutex};
use uuid::Uuid;

static MIGRATE_ADMIN_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn dry_run_reports_legacy_counts_without_writing_postgres() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping legacy migration DB test; set EMBY_MANAGER_MIGRATE_TEST_DATABASE_URL");
        return;
    };
    let marker = format!("dry_{}", Uuid::new_v4().simple());
    let legacy = tempfile::tempdir().expect("create legacy dir");
    seed_legacy_dir(legacy.path(), &marker, CatalogSchema::WithoutLinkType);

    let before = marker_counts(&pool, &marker).await;
    let report = migrate::run(&pool, legacy.path().to_path_buf(), false)
        .await
        .expect("dry-run legacy migration");
    let after = marker_counts(&pool, &marker).await;

    assert!(!report.applied);
    assert_eq!(report.config_keys, 3);
    assert_eq!(report.schedules, 2);
    assert_eq!(report.undo_entries, 2);
    assert_eq!(report.catalog_items, 2);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("skipped malformed undo line 2")),
        "{:?}",
        report.warnings
    );
    assert_eq!(after, before, "dry-run must not write marker rows");
}

#[tokio::test(flavor = "current_thread")]
async fn apply_imports_legacy_rows_infers_old_catalog_types_and_skips_repeats() {
    let _guard = MIGRATE_ADMIN_LOCK.lock().unwrap();
    let Some(pool) = test_pool().await else {
        eprintln!("skipping legacy migration DB test; set EMBY_MANAGER_MIGRATE_TEST_DATABASE_URL");
        return;
    };
    delete_admin(&pool).await;
    let marker = format!("apply_{}", Uuid::new_v4().simple());
    let legacy = tempfile::tempdir().expect("create legacy dir");
    seed_legacy_dir(legacy.path(), &marker, CatalogSchema::WithoutLinkType);

    let report = migrate::run(&pool, legacy.path().to_path_buf(), true)
        .await
        .expect("apply legacy migration");
    assert!(report.applied);
    assert_eq!(report.config_keys, 3);
    assert_eq!(report.schedules, 2);
    assert_eq!(report.undo_entries, 2);
    assert_eq!(report.catalog_items, 2);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("skipped malformed undo line 2")),
        "{:?}",
        report.warnings
    );

    assert_eq!(
        setting_value(&pool, "emby_url").await,
        json!(format!("http://{marker}.emby"))
    );
    let admin: (String, bool) = sqlx::query_as(
        "SELECT password_hash, legacy_hash FROM auth_users WHERE username = 'admin'",
    )
    .fetch_one(&pool)
    .await
    .expect("load migrated admin user");
    assert_eq!(admin.0, format!("legacy-hash-{marker}"));
    assert!(admin.1);

    assert_eq!(
        marker_counts(&pool, &marker).await,
        MarkerCounts {
            app_settings: 1,
            auth_users: 1,
            schedules: 2,
            undo_entries: 2,
            catalog_items: 2,
        }
    );
    assert_eq!(
        catalog_type(&pool, &format!("magnet:?xt=urn:btih:{marker}")).await,
        "magnet"
    );
    assert_eq!(
        catalog_type(&pool, &format!("https://anxia.com/s/{marker}")).await,
        "share115"
    );

    migrate::run(&pool, legacy.path().to_path_buf(), true)
        .await
        .expect("repeat apply legacy migration");
    assert_eq!(
        marker_counts(&pool, &marker).await,
        MarkerCounts {
            app_settings: 1,
            auth_users: 1,
            schedules: 2,
            undo_entries: 2,
            catalog_items: 2,
        },
        "repeat apply should not duplicate exact same legacy schedules, undo entries, or catalog links"
    );
}

#[tokio::test]
async fn missing_legacy_files_report_clear_warnings() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping legacy migration DB test; set EMBY_MANAGER_MIGRATE_TEST_DATABASE_URL");
        return;
    };
    let legacy = tempfile::tempdir().expect("create empty legacy dir");

    let report = migrate::run(&pool, legacy.path().to_path_buf(), false)
        .await
        .expect("dry-run empty legacy migration");

    assert_eq!(report.config_keys, 0);
    assert_eq!(report.schedules, 0);
    assert_eq!(report.undo_entries, 0);
    assert_eq!(report.catalog_items, 0);
    assert_warning_contains(&report.warnings, "config.json not found at");
    assert_warning_contains(&report.warnings, "undo_log.jsonl not found at");
    assert_warning_contains(&report.warnings, "catalog_115.db not found at");
}

#[tokio::test(flavor = "current_thread")]
async fn apply_does_not_overwrite_existing_admin_password() {
    let _guard = MIGRATE_ADMIN_LOCK.lock().unwrap();
    let Some(pool) = test_pool().await else {
        eprintln!("skipping legacy migration DB test; set EMBY_MANAGER_MIGRATE_TEST_DATABASE_URL");
        return;
    };
    delete_admin(&pool).await;
    let marker = format!("admin_{}", Uuid::new_v4().simple());
    let legacy = tempfile::tempdir().expect("create legacy dir");
    seed_legacy_dir(legacy.path(), &marker, CatalogSchema::WithoutLinkType);
    let existing_hash = format!("existing-admin-hash-{marker}");
    sqlx::query(
        "INSERT INTO auth_users(id, username, password_hash, legacy_hash)
         VALUES ($1, 'admin', $2, FALSE)
         ON CONFLICT(username) DO UPDATE
         SET password_hash = EXCLUDED.password_hash, legacy_hash = FALSE, updated_at = now()",
    )
    .bind(Uuid::new_v4())
    .bind(&existing_hash)
    .execute(&pool)
    .await
    .expect("seed existing admin");

    migrate::run(&pool, legacy.path().to_path_buf(), true)
        .await
        .expect("apply legacy migration");

    let admin: (String, bool) = sqlx::query_as(
        "SELECT password_hash, legacy_hash FROM auth_users WHERE username = 'admin'",
    )
    .fetch_one(&pool)
    .await
    .expect("load admin user");
    assert_eq!(admin.0, existing_hash);
    assert!(!admin.1);
}

#[derive(Clone, Copy)]
enum CatalogSchema {
    WithoutLinkType,
}

#[derive(Debug, PartialEq, Eq)]
struct MarkerCounts {
    app_settings: i64,
    auth_users: i64,
    schedules: i64,
    undo_entries: i64,
    catalog_items: i64,
}

async fn test_pool() -> Option<PgPool> {
    let database_url = migrate_test_database_url()?;
    let pool = match PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
    {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!(
                "skipping legacy migration DB test; could not connect to test database: {err}"
            );
            return None;
        }
    };
    if let Err(err) = db::migrate(&pool).await {
        eprintln!("skipping legacy migration DB test; could not run migrations: {err}");
        return None;
    }
    Some(pool)
}

async fn delete_admin(pool: &PgPool) {
    sqlx::query("DELETE FROM sessions WHERE user_id IN (SELECT id FROM auth_users WHERE username = 'admin')")
        .execute(pool)
        .await
        .expect("delete admin sessions");
    sqlx::query("DELETE FROM auth_users WHERE username = 'admin'")
        .execute(pool)
        .await
        .expect("delete admin user");
}

fn migrate_test_database_url() -> Option<String> {
    if let Ok(url) = env::var("EMBY_MANAGER_MIGRATE_TEST_DATABASE_URL") {
        return Some(url);
    }
    let url = env::var("DATABASE_URL").ok()?;
    url.to_ascii_lowercase().contains("test").then_some(url)
}

fn seed_legacy_dir(path: &Path, marker: &str, schema: CatalogSchema) {
    fs::write(
        path.join("config.json"),
        serde_json::to_vec_pretty(&json!({
            "emby_url": format!("http://{marker}.emby"),
            "password_hash": format!("legacy-hash-{marker}"),
            "schedules": [
                {
                    "name": format!("{marker}-scan"),
                    "kind": "scan_all",
                    "params": {"marker": marker},
                    "schedule": {"mode": "daily", "hour": 3, "minute": 15},
                    "enabled": true,
                    "last_status": "done"
                },
                {
                    "name": format!("{marker}-clean"),
                    "kind": "cleanup",
                    "params": {"marker": marker, "dry": false},
                    "schedule": {"mode": "interval", "minutes": 120},
                    "enabled": false,
                    "last_err": "legacy error"
                }
            ]
        }))
        .expect("serialize config"),
    )
    .expect("write config.json");

    fs::write(
        path.join("undo_log.jsonl"),
        format!(
            "{}\nnot-json\n{}\n\n",
            json!({
                "id": format!("{marker}-undo-1"),
                "op": "move",
                "payload": {"marker": marker, "from": "/a", "to": "/b"},
                "undone": false,
                "ts": 1_700_000_001_i64
            }),
            json!({
                "id": format!("{marker}-undo-2"),
                "op": "delete",
                "payload": {"marker": marker, "path": "/gone"},
                "undone": true,
                "ts": 1_700_000_002_i64
            })
        ),
    )
    .expect("write undo_log.jsonl");

    let con = Connection::open(path.join("catalog_115.db")).expect("open catalog sqlite");
    match schema {
        CatalogSchema::WithoutLinkType => {
            con.execute(
                "CREATE TABLE catalog(name TEXT, sheet TEXT, link TEXT, is_pkg INTEGER)",
                [],
            )
            .expect("create old catalog table");
            con.execute(
                "INSERT INTO catalog(name, sheet, link, is_pkg) VALUES (?1, ?2, ?3, ?4)",
                (
                    format!("{marker} magnet"),
                    "Movies",
                    format!("magnet:?xt=urn:btih:{marker}"),
                    0_i64,
                ),
            )
            .expect("insert magnet catalog");
            con.execute(
                "INSERT INTO catalog(name, sheet, link, is_pkg) VALUES (?1, ?2, ?3, ?4)",
                (
                    format!("{marker} share"),
                    "Shows",
                    format!("https://anxia.com/s/{marker}"),
                    1_i64,
                ),
            )
            .expect("insert share catalog");
        }
    }
}

async fn marker_counts(pool: &PgPool, marker: &str) -> MarkerCounts {
    let app_settings = sqlx::query_scalar("SELECT COUNT(*) FROM app_settings WHERE value = $1")
        .bind(json!(format!("http://{marker}.emby")))
        .fetch_one(pool)
        .await
        .expect("count marker app settings");
    let auth_users = sqlx::query_scalar("SELECT COUNT(*) FROM auth_users WHERE password_hash = $1")
        .bind(format!("legacy-hash-{marker}"))
        .fetch_one(pool)
        .await
        .expect("count marker auth users");
    let schedules = sqlx::query_scalar("SELECT COUNT(*) FROM schedule_jobs WHERE name IN ($1, $2)")
        .bind(format!("{marker}-scan"))
        .bind(format!("{marker}-clean"))
        .fetch_one(pool)
        .await
        .expect("count marker schedules");
    let undo_entries =
        sqlx::query_scalar("SELECT COUNT(*) FROM undo_entries WHERE legacy_id IN ($1, $2)")
            .bind(format!("{marker}-undo-1"))
            .bind(format!("{marker}-undo-2"))
            .fetch_one(pool)
            .await
            .expect("count marker undo entries");
    let catalog_items =
        sqlx::query_scalar("SELECT COUNT(*) FROM catalog_items WHERE link IN ($1, $2)")
            .bind(format!("magnet:?xt=urn:btih:{marker}"))
            .bind(format!("https://anxia.com/s/{marker}"))
            .fetch_one(pool)
            .await
            .expect("count marker catalog items");
    MarkerCounts {
        app_settings,
        auth_users,
        schedules,
        undo_entries,
        catalog_items,
    }
}

async fn setting_value(pool: &PgPool, key: &str) -> Value {
    sqlx::query_scalar("SELECT value FROM app_settings WHERE key = $1")
        .bind(key)
        .fetch_one(pool)
        .await
        .expect("load app setting")
}

async fn catalog_type(pool: &PgPool, link: &str) -> String {
    sqlx::query_scalar("SELECT link_type FROM catalog_items WHERE link = $1")
        .bind(link)
        .fetch_one(pool)
        .await
        .expect("load catalog type")
}

fn assert_warning_contains(warnings: &[String], needle: &str) {
    assert!(
        warnings.iter().any(|warning| warning.contains(needle)),
        "expected warning containing {needle:?}, got {warnings:?}"
    );
}
