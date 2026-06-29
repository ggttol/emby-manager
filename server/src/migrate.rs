use crate::catalog::infer_type;
use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, Postgres, QueryBuilder};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const CATALOG_BATCH_SIZE: usize = 1000;

#[derive(Debug, Serialize)]
pub struct MigrationReport {
    pub legacy_dir: PathBuf,
    pub applied: bool,
    pub config_keys: usize,
    pub schedules: usize,
    pub undo_entries: usize,
    pub catalog_items: usize,
    pub autostrm_seen: usize,
    pub warnings: Vec<String>,
}

pub async fn run(
    pool: &PgPool,
    legacy_dir: PathBuf,
    apply: bool,
) -> anyhow::Result<MigrationReport> {
    let mut report = MigrationReport {
        legacy_dir: legacy_dir.clone(),
        applied: apply,
        config_keys: 0,
        schedules: 0,
        undo_entries: 0,
        catalog_items: 0,
        autostrm_seen: 0,
        warnings: vec![],
    };
    migrate_config(pool, &legacy_dir, apply, &mut report).await?;
    migrate_undo(pool, &legacy_dir, apply, &mut report).await?;
    migrate_catalog(pool, &legacy_dir, apply, &mut report).await?;
    migrate_autostrm_seen(pool, &legacy_dir, apply, &mut report).await?;
    Ok(report)
}

async fn migrate_config(
    pool: &PgPool,
    legacy_dir: &Path,
    apply: bool,
    report: &mut MigrationReport,
) -> anyhow::Result<()> {
    let path = legacy_dir.join("config.json");
    if !path.exists() {
        report
            .warnings
            .push(format!("config.json not found at {}", path.display()));
        return Ok(());
    }
    let value: Value = serde_json::from_reader(File::open(path)?)?;
    let Some(obj) = value.as_object() else {
        report
            .warnings
            .push("config.json is not an object".to_string());
        return Ok(());
    };
    report.config_keys = obj.len();
    if apply {
        for (key, value) in obj {
            if key == "schedules" {
                if let Some(items) = value.as_array() {
                    for item in items {
                        import_schedule(pool, item).await?;
                        report.schedules += 1;
                    }
                }
            } else {
                sqlx::query(
                    "INSERT INTO app_settings(key, value, updated_at) VALUES ($1, $2, now())
                     ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
                )
                .bind(key)
                .bind(value)
                .execute(pool)
                .await?;
            }
        }
        if let Some(hash) = obj
            .get("password_hash")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(
                "INSERT INTO auth_users(id, username, password_hash, legacy_hash)
                 VALUES ($1, 'admin', $2, TRUE)
                 ON CONFLICT(username) DO NOTHING",
            )
            .bind(Uuid::new_v4())
            .bind(hash)
            .execute(pool)
            .await?;
        }
    } else if let Some(items) = obj.get("schedules").and_then(Value::as_array) {
        report.schedules = items.len();
    }
    Ok(())
}

async fn import_schedule(pool: &PgPool, item: &Value) -> anyhow::Result<()> {
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("legacy schedule");
    let kind = item
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let params = item
        .get("params")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let schedule = item
        .get("schedule")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"mode":"daily","hour":3,"minute":0}));
    let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    sqlx::query(
        "INSERT INTO schedule_jobs(id, name, kind, params, schedule, enabled, last_status, last_error)
         SELECT $1, $2, $3, $4, $5, $6, $7, $8
         WHERE NOT EXISTS (
             SELECT 1 FROM schedule_jobs
             WHERE name = $2 AND kind = $3 AND params = $4 AND schedule = $5
         )",
    )
    .bind(Uuid::new_v4())
    .bind(name)
    .bind(kind)
    .bind(params)
    .bind(schedule)
    .bind(enabled)
    .bind(item.get("last_status").and_then(Value::as_str))
    .bind(item.get("last_err").and_then(Value::as_str))
    .execute(pool)
    .await?;
    Ok(())
}

async fn migrate_undo(
    pool: &PgPool,
    legacy_dir: &Path,
    apply: bool,
    report: &mut MigrationReport,
) -> anyhow::Result<()> {
    let path = legacy_dir.join("undo_log.jsonl");
    if !path.exists() {
        report
            .warnings
            .push(format!("undo_log.jsonl not found at {}", path.display()));
        return Ok(());
    }
    let file = File::open(path)?;
    for (idx, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                report.warnings.push(format!(
                    "skipped malformed undo line {} in undo_log.jsonl",
                    idx + 1
                ));
                continue;
            }
        };
        report.undo_entries += 1;
        if apply {
            let ts = value
                .get("ts")
                .and_then(Value::as_i64)
                .unwrap_or_else(|| Utc::now().timestamp());
            let created_at = Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now);
            let legacy_id = value.get("id").and_then(Value::as_str);
            let op = value.get("op").and_then(Value::as_str).unwrap_or("unknown");
            let payload = value
                .get("payload")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let undone = value
                .get("undone")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            sqlx::query(
                "INSERT INTO undo_entries(id, legacy_id, op, payload, undone, created_at)
                 SELECT $1, $2, $3, $4, $5, $6
                 WHERE NOT EXISTS (
                     SELECT 1 FROM undo_entries
                     WHERE legacy_id IS NOT DISTINCT FROM $2
                       AND op = $3
                       AND payload = $4
                       AND created_at = $6
                 )",
            )
            .bind(Uuid::new_v4())
            .bind(legacy_id)
            .bind(op)
            .bind(payload)
            .bind(undone)
            .bind(created_at)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

async fn migrate_catalog(
    pool: &PgPool,
    legacy_dir: &Path,
    apply: bool,
    report: &mut MigrationReport,
) -> anyhow::Result<()> {
    let path = legacy_dir.join("catalog_115.db");
    if !path.exists() {
        report
            .warnings
            .push(format!("catalog_115.db not found at {}", path.display()));
        return Ok(());
    }
    let con = Connection::open(path)?;
    let has_type_col = con
        .prepare("PRAGMA table_info(catalog)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == "link_type");
    let sql = if has_type_col {
        "SELECT name, sheet, link, is_pkg, link_type FROM catalog"
    } else {
        "SELECT name, sheet, link, is_pkg, '' as link_type FROM catalog"
    };
    let mut stmt = con.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?.unwrap_or_default(),
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, i64>(3).unwrap_or(0) != 0,
            row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        ))
    })?;
    let mut batch = Vec::with_capacity(CATALOG_BATCH_SIZE);
    for row in rows {
        let (name, sheet, link, is_pkg, link_type) = row?;
        if name.is_empty() && link.is_empty() {
            continue;
        }
        report.catalog_items += 1;
        if apply {
            let link_type = if link_type.is_empty() {
                infer_type(&link).to_string()
            } else {
                link_type
            };
            batch.push(CatalogImportRow {
                name,
                sheet,
                link,
                is_pkg,
                link_type,
            });
            if batch.len() >= CATALOG_BATCH_SIZE {
                flush_catalog_batch(pool, &mut batch).await?;
            }
        }
    }
    if apply {
        flush_catalog_batch(pool, &mut batch).await?;
    }
    Ok(())
}

async fn migrate_autostrm_seen(
    pool: &PgPool,
    legacy_dir: &Path,
    apply: bool,
    report: &mut MigrationReport,
) -> anyhow::Result<()> {
    let Some(path) = find_autostrm_seen_path(legacy_dir) else {
        report.warnings.push(format!(
            "autostrm_seen.json not found at {} or {}",
            legacy_dir.join("autostrm_seen.json").display(),
            legacy_dir.join("strm").join("autostrm_seen.json").display()
        ));
        return Ok(());
    };
    let value: Value = match serde_json::from_reader(File::open(&path)?) {
        Ok(value) => value,
        Err(err) => {
            report.warnings.push(format!(
                "skipped malformed autostrm_seen.json at {}: {err}",
                path.display()
            ));
            return Ok(());
        }
    };
    let Some(libs) = value.get("libs").and_then(Value::as_object) else {
        report
            .warnings
            .push("autostrm_seen.json missing object field libs".to_string());
        return Ok(());
    };
    for (lib, tops) in libs {
        let Some(tops) = tops.as_object() else {
            report.warnings.push(format!(
                "skipped autostrm seen lib {lib}: value is not an object"
            ));
            continue;
        };
        for (top, mtime_value) in tops {
            let Some(mtime) = legacy_mtime(mtime_value) else {
                report
                    .warnings
                    .push(format!("skipped autostrm seen {lib}/{top}: invalid mtime"));
                continue;
            };
            report.autostrm_seen += 1;
            if apply {
                sqlx::query(
                    "INSERT INTO autostrm_seen(lib, top, mtime, updated_at)
                     VALUES ($1, $2, $3, now())
                     ON CONFLICT(lib, top) DO UPDATE
                     SET mtime = EXCLUDED.mtime, updated_at = now()
                     WHERE autostrm_seen.mtime IS DISTINCT FROM EXCLUDED.mtime",
                )
                .bind(lib)
                .bind(top)
                .bind(mtime)
                .execute(pool)
                .await?;
            }
        }
    }
    Ok(())
}

fn find_autostrm_seen_path(legacy_dir: &Path) -> Option<PathBuf> {
    [
        legacy_dir.join("autostrm_seen.json"),
        legacy_dir.join("strm").join("autostrm_seen.json"),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn legacy_mtime(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|value| value.trunc() as i64))
}

struct CatalogImportRow {
    name: String,
    sheet: String,
    link: String,
    is_pkg: bool,
    link_type: String,
}

async fn flush_catalog_batch(
    pool: &PgPool,
    batch: &mut Vec<CatalogImportRow>,
) -> anyhow::Result<()> {
    if batch.is_empty() {
        return Ok(());
    }

    let mut builder =
        QueryBuilder::<Postgres>::new("WITH incoming(name, sheet, link, is_pkg, link_type) AS (");
    builder.push_values(batch.iter(), |mut row, item| {
        row.push_bind(&item.name)
            .push_bind(&item.sheet)
            .push_bind(&item.link)
            .push_bind(item.is_pkg)
            .push_bind(&item.link_type);
    });
    builder.push(
        ")
         INSERT INTO catalog_items(name, sheet, link, is_pkg, link_type)
         SELECT i.name, i.sheet, i.link, i.is_pkg, i.link_type
         FROM incoming i
         WHERE NOT EXISTS (
             SELECT 1 FROM catalog_items c WHERE c.link = i.link
         )",
    );
    builder.build().execute(pool).await?;
    batch.clear();
    Ok(())
}
