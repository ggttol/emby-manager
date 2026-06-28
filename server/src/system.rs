use crate::{error::AppResult, state::AppState};
use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use sqlx::{PgPool, Row};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tokio::time::timeout;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SystemSummary {
    pub ok: bool,
    pub version: &'static str,
    pub cd_root: String,
    pub strm_root: String,
    pub docker_bin: String,
    pub cd_root_exists: bool,
    pub strm_root_exists: bool,
    pub docker_bin_exists: bool,
    pub database: DatabaseSummary,
    pub configured_roots: Vec<PathStatus>,
    pub host: HostMetrics,
    pub warnings: Vec<String>,
    pub rust_version: &'static str,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DatabaseSummary {
    pub configured: bool,
    pub url: String,
    pub status: String,
    pub current_database: Option<String>,
    pub server_version: Option<String>,
    pub pool_size: u32,
    pub idle_connections: usize,
    pub warning: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PathStatus {
    pub key: String,
    pub label: String,
    pub path: String,
    pub expected_kind: String,
    pub exists: bool,
    pub is_dir: bool,
    pub is_file: bool,
    pub readable: Option<bool>,
    pub writable_hint: Option<bool>,
    pub disk: Option<DiskSummary>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DiskSummary {
    pub filesystem: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: Option<f64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HostMetrics {
    pub os: String,
    pub arch: String,
    pub process_id: u32,
    pub memory: Option<MemorySummary>,
    pub load_average: Option<LoadAverage>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MemorySummary {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: f64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v2/system/summary", get(system_summary))
}

#[utoipa::path(get, path = "/api/v2/system/summary", tag = "system", responses((status = 200, body = SystemSummary)))]
pub async fn system_summary(State(state): State<AppState>) -> AppResult<Json<SystemSummary>> {
    let mut warnings = Vec::new();

    let database = database_summary(&state.pool, &state.settings.database_url).await;
    if let Some(warning) = database.warning.as_ref() {
        warnings.push(format!("数据库检查失败: {warning}"));
    }

    let configured_roots = vec![
        inspect_path(PathSpec::directory(
            "cd_root",
            "CloudDrive 媒体根",
            &state.settings.cd_root,
            true,
            false,
        ))
        .await,
        inspect_path(PathSpec::directory(
            "strm_root",
            "strm 根目录",
            &state.settings.strm_root,
            true,
            true,
        ))
        .await,
        inspect_path(PathSpec::directory(
            "legacy_dir",
            "旧版数据目录",
            &state.settings.legacy_dir,
            false,
            false,
        ))
        .await,
        inspect_path(PathSpec::directory(
            "web_dist",
            "前端静态资源目录",
            &state.settings.web_dist,
            true,
            false,
        ))
        .await,
        inspect_path(PathSpec::file(
            "docker_bin",
            "Docker CLI",
            &state.settings.docker_bin,
            true,
        ))
        .await,
    ];
    warnings.extend(
        configured_roots
            .iter()
            .flat_map(|status| status.warnings.iter().cloned()),
    );

    let host = host_metrics(&mut warnings);
    let cd_root_exists = path_exists(&configured_roots, "cd_root");
    let strm_root_exists = path_exists(&configured_roots, "strm_root");
    let docker_bin_exists = path_exists(&configured_roots, "docker_bin");

    Ok(Json(SystemSummary {
        ok: warnings.is_empty(),
        version: env!("CARGO_PKG_VERSION"),
        cd_root: state.settings.cd_root.display().to_string(),
        strm_root: state.settings.strm_root.display().to_string(),
        docker_bin: state.settings.docker_bin.display().to_string(),
        cd_root_exists,
        strm_root_exists,
        docker_bin_exists,
        database,
        configured_roots,
        host,
        warnings,
        rust_version: env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Clone, Copy)]
enum ExpectedKind {
    Directory,
    File,
}

struct PathSpec<'a> {
    key: &'static str,
    label: &'static str,
    path: &'a Path,
    expected_kind: ExpectedKind,
    warn_missing: bool,
    warn_readonly: bool,
}

impl<'a> PathSpec<'a> {
    fn directory(
        key: &'static str,
        label: &'static str,
        path: &'a Path,
        warn_missing: bool,
        warn_readonly: bool,
    ) -> Self {
        Self {
            key,
            label,
            path,
            expected_kind: ExpectedKind::Directory,
            warn_missing,
            warn_readonly,
        }
    }

    fn file(key: &'static str, label: &'static str, path: &'a Path, warn_missing: bool) -> Self {
        Self {
            key,
            label,
            path,
            expected_kind: ExpectedKind::File,
            warn_missing,
            warn_readonly: false,
        }
    }
}

async fn database_summary(pool: &PgPool, database_url: &str) -> DatabaseSummary {
    let configured = !database_url.trim().is_empty();
    let mut summary = DatabaseSummary {
        configured,
        url: redact_database_url(database_url),
        status: if configured {
            "checking".to_string()
        } else {
            "not_configured".to_string()
        },
        current_database: None,
        server_version: None,
        pool_size: pool.size(),
        idle_connections: pool.num_idle(),
        warning: None,
    };
    if !configured {
        summary.warning = Some("DATABASE_URL 未配置".to_string());
        return summary;
    }

    let result = timeout(
        Duration::from_millis(500),
        sqlx::query(
            "SELECT current_database()::text AS current_database, version()::text AS server_version",
        )
        .fetch_one(pool),
    )
    .await;
    match result {
        Ok(Ok(row)) => {
            summary.status = "ok".to_string();
            summary.current_database = row.try_get::<String, _>("current_database").ok();
            summary.server_version = row
                .try_get::<String, _>("server_version")
                .ok()
                .map(|v| shorten(&v, 180));
        }
        Ok(Err(err)) => {
            summary.status = "unavailable".to_string();
            summary.warning = Some(shorten(&err.to_string(), 220));
        }
        Err(_) => {
            summary.status = "timeout".to_string();
            summary.warning = Some("500ms 内未完成数据库探测".to_string());
        }
    }
    summary
}

async fn inspect_path(spec: PathSpec<'_>) -> PathStatus {
    let metadata = fs::metadata(spec.path).ok();
    let exists = metadata.is_some();
    let is_dir = metadata.as_ref().is_some_and(|m| m.is_dir());
    let is_file = metadata.as_ref().is_some_and(|m| m.is_file());
    let readable = readable_hint(spec.path, metadata.as_ref());
    let writable_hint = metadata.as_ref().map(|_| looks_writable(spec.path));
    let expected_kind = match spec.expected_kind {
        ExpectedKind::Directory => "directory",
        ExpectedKind::File => "file",
    }
    .to_string();
    let mut warnings = Vec::new();

    if !exists && spec.warn_missing {
        warnings.push(format!("{} 不存在: {}", spec.label, spec.path.display()));
    }
    if exists {
        match spec.expected_kind {
            ExpectedKind::Directory if !is_dir => {
                warnings.push(format!("{} 不是目录: {}", spec.label, spec.path.display()))
            }
            ExpectedKind::File if !is_file => {
                warnings.push(format!("{} 不是文件: {}", spec.label, spec.path.display()))
            }
            _ => {}
        }
    }
    if spec.warn_readonly && matches!(writable_hint, Some(false)) {
        warnings.push(format!(
            "{} 权限位显示不可写: {}",
            spec.label,
            spec.path.display()
        ));
    }

    let disk = match disk_summary(spec.path).await {
        Ok(disk) => disk,
        Err(err) => {
            warnings.push(format!("{} 磁盘信息不可用: {err}", spec.label));
            None
        }
    };

    PathStatus {
        key: spec.key.to_string(),
        label: spec.label.to_string(),
        path: spec.path.display().to_string(),
        expected_kind,
        exists,
        is_dir,
        is_file,
        readable,
        writable_hint,
        disk,
        warnings,
    }
}

fn path_exists(paths: &[PathStatus], key: &str) -> bool {
    paths
        .iter()
        .find(|status| status.key == key)
        .is_some_and(|status| status.exists)
}

fn readable_hint(path: &Path, metadata: Option<&fs::Metadata>) -> Option<bool> {
    let metadata = metadata?;
    if metadata.is_dir() {
        Some(fs::read_dir(path).is_ok())
    } else if metadata.is_file() {
        Some(fs::File::open(path).is_ok())
    } else {
        None
    }
}

fn looks_writable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| !m.permissions().readonly())
        .unwrap_or(false)
}

async fn disk_summary(path: &Path) -> Result<Option<DiskSummary>, String> {
    let Some(target) = disk_probe_path(path) else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || run_df(&target))
        .await
        .map_err(|err| err.to_string())?
}

fn disk_probe_path(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|candidate| candidate.exists())
        .map(Path::to_path_buf)
}

fn run_df(path: &Path) -> Result<Option<DiskSummary>, String> {
    let output = Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(line) = text.lines().filter(|line| !line.trim().is_empty()).last() else {
        return Ok(None);
    };
    let cols = line.split_whitespace().collect::<Vec<_>>();
    if cols.len() < 6 {
        return Err("df 输出无法解析".to_string());
    }
    let total_kib = parse_u64(cols[1], "total")?;
    let used_kib = parse_u64(cols[2], "used")?;
    let available_kib = parse_u64(cols[3], "available")?;
    let used_percent = cols[4].trim_end_matches('%').parse::<f64>().ok();
    Ok(Some(DiskSummary {
        filesystem: cols[0].to_string(),
        total_bytes: total_kib.saturating_mul(1024),
        used_bytes: used_kib.saturating_mul(1024),
        available_bytes: available_kib.saturating_mul(1024),
        used_percent,
        mount_point: cols[5..].join(" "),
    }))
}

fn host_metrics(warnings: &mut Vec<String>) -> HostMetrics {
    let memory = if Path::new("/proc/meminfo").exists() {
        match read_proc_meminfo() {
            Ok(memory) => Some(memory),
            Err(err) => {
                warnings.push(format!("内存信息不可用: {err}"));
                None
            }
        }
    } else {
        None
    };
    let load_average = if Path::new("/proc/loadavg").exists() {
        match read_proc_loadavg() {
            Ok(load) => Some(load),
            Err(err) => {
                warnings.push(format!("load average 不可用: {err}"));
                None
            }
        }
    } else {
        None
    };

    HostMetrics {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        process_id: std::process::id(),
        memory,
        load_average,
    }
}

fn read_proc_meminfo() -> Result<MemorySummary, String> {
    let text = fs::read_to_string("/proc/meminfo").map_err(|err| err.to_string())?;
    let mut total_kib = None;
    let mut available_kib = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next().map(|v| v.trim_end_matches(':')) else {
            continue;
        };
        let Some(raw_value) = parts.next() else {
            continue;
        };
        let Ok(value) = raw_value.parse::<u64>() else {
            continue;
        };
        match key {
            "MemTotal" => total_kib = Some(value),
            "MemAvailable" => available_kib = Some(value),
            "MemFree" if available_kib.is_none() => available_kib = Some(value),
            _ => {}
        }
    }
    let total_kib = total_kib.ok_or_else(|| "缺少 MemTotal".to_string())?;
    let available_kib = available_kib.ok_or_else(|| "缺少 MemAvailable/MemFree".to_string())?;
    let total_bytes = total_kib.saturating_mul(1024);
    let available_bytes = available_kib.saturating_mul(1024);
    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let used_percent = if total_bytes == 0 {
        0.0
    } else {
        used_bytes as f64 * 100.0 / total_bytes as f64
    };
    Ok(MemorySummary {
        total_bytes,
        available_bytes,
        used_percent,
    })
}

fn read_proc_loadavg() -> Result<LoadAverage, String> {
    let text = fs::read_to_string("/proc/loadavg").map_err(|err| err.to_string())?;
    let mut parts = text.split_whitespace();
    let one = parse_f64(parts.next(), "1m")?;
    let five = parse_f64(parts.next(), "5m")?;
    let fifteen = parse_f64(parts.next(), "15m")?;
    Ok(LoadAverage { one, five, fifteen })
}

fn parse_u64(value: &str, label: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{label} 不是数字: {value}"))
}

fn parse_f64(value: Option<&str>, label: &str) -> Result<f64, String> {
    value
        .ok_or_else(|| format!("缺少 {label} 字段"))?
        .parse::<f64>()
        .map_err(|_| format!("{label} 不是数字"))
}

fn redact_database_url(raw: &str) -> String {
    let raw = raw.trim();
    let Some(scheme_end) = raw.find("://") else {
        return raw.to_string();
    };
    let prefix = &raw[..scheme_end + 3];
    let rest = &raw[scheme_end + 3..];
    if let Some(at) = rest.find('@') {
        format!("{prefix}***@{}", &rest[at + 1..])
    } else {
        raw.to_string()
    }
}

fn shorten(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}
