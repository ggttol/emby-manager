use crate::{config_store, error::AppResult, state::AppState};
use axum::{Json, Router, extract::State, routing::get};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command as StdCommand, Stdio},
    time::Duration,
};
use tokio::{process::Command as TokioCommand, time::timeout};

const DEFAULT_EMBY_URL: &str = "http://127.0.0.1:8096/emby";

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
    pub docker: DockerSummary,
    pub emby: EmbyHealthSummary,
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
pub struct DockerSummary {
    pub configured: bool,
    pub available: bool,
    pub docker_bin: String,
    pub status: String,
    pub total: usize,
    pub running: usize,
    pub containers: Vec<DockerContainerSummary>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct DockerContainerSummary {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct EmbyHealthSummary {
    pub configured: bool,
    pub online: bool,
    pub status: String,
    pub base_url: String,
    pub http_status: Option<u16>,
    pub version: Option<String>,
    pub server_name: Option<String>,
    pub server_id: Option<String>,
    pub operating_system: Option<String>,
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

    let docker = docker_summary(&state.settings.docker_bin).await;
    if let Some(warning) = docker.warning.as_ref() {
        warnings.push(warning.clone());
    }

    let emby = emby_summary(&state).await;
    if let Some(warning) = emby.warning.as_ref() {
        warnings.push(warning.clone());
    }

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
        docker,
        emby,
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

async fn docker_summary(docker_bin: &Path) -> DockerSummary {
    let configured = !docker_bin.as_os_str().is_empty();
    let mut summary = DockerSummary {
        configured,
        available: false,
        docker_bin: docker_bin.display().to_string(),
        status: if configured {
            "checking".to_string()
        } else {
            "not_configured".to_string()
        },
        total: 0,
        running: 0,
        containers: Vec::new(),
        warning: None,
    };

    if !configured {
        summary.warning = Some("Docker CLI 未配置，跳过容器列表".to_string());
        return summary;
    }

    match run_docker_ps(docker_bin).await {
        Ok(stdout) => {
            summary.available = true;
            match parse_docker_ps_output(&stdout) {
                Ok(containers) => {
                    summary.running = containers
                        .iter()
                        .filter(|container| docker_container_running(container))
                        .count();
                    summary.total = containers.len();
                    summary.containers = containers;
                    summary.status = "ok".to_string();
                }
                Err(err) => {
                    summary.status = "parse_error".to_string();
                    summary.warning = Some(format!("Docker 容器列表解析失败: {err}"));
                }
            }
        }
        Err(err) => {
            summary.status = "unavailable".to_string();
            summary.warning = Some(format!("Docker 容器列表不可用: {err}"));
        }
    }

    summary
}

async fn run_docker_ps(docker_bin: &Path) -> Result<String, String> {
    let mut command = TokioCommand::new(docker_bin);
    command
        .arg("ps")
        .arg("--all")
        .arg("--format")
        .arg("{{json .}}")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output = timeout(Duration::from_millis(1500), command.output())
        .await
        .map_err(|_| "1500ms 内未完成 Docker 容器探测".to_string())?
        .map_err(|err| err.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = shorten(stderr.trim(), 220);
        return Err(if message.is_empty() {
            format!("docker ps 退出状态 {}", output.status)
        } else {
            format!("docker ps 退出状态 {}: {message}", output.status)
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn parse_docker_ps_output(text: &str) -> Result<Vec<DockerContainerSummary>, String> {
    let mut containers = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line)
            .map_err(|err| format!("第 {} 行 JSON 无法解析: {err}", index + 1))?;
        containers.push(DockerContainerSummary {
            id: json_string_field(&value, &["ID", "Id", "ContainerID"]),
            name: json_string_field(&value, &["Names", "Name"]),
            image: json_string_field(&value, &["Image"]),
            state: json_string_field(&value, &["State"]),
            status: json_string_field(&value, &["Status"]),
            ports: json_string_field(&value, &["Ports"]),
        });
    }
    Ok(containers)
}

fn docker_container_running(container: &DockerContainerSummary) -> bool {
    let state = container.state.trim().to_ascii_lowercase();
    state == "running"
        || (state.is_empty() && container.status.to_ascii_lowercase().starts_with("up "))
}

async fn emby_summary(state: &AppState) -> EmbyHealthSummary {
    match read_emby_probe_config(&state.pool).await {
        Ok(config) => probe_emby_health(&config.base_url, &config.api_key, &state.http).await,
        Err(err) => EmbyHealthSummary {
            configured: false,
            online: false,
            status: "config_unavailable".to_string(),
            base_url: DEFAULT_EMBY_URL.to_string(),
            http_status: None,
            version: None,
            server_name: None,
            server_id: None,
            operating_system: None,
            warning: Some(format!("Emby 配置读取失败: {err}")),
        },
    }
}

struct EmbyProbeConfig {
    base_url: String,
    api_key: String,
}

async fn read_emby_probe_config(pool: &PgPool) -> Result<EmbyProbeConfig, String> {
    timeout(Duration::from_millis(500), async {
        let base_url = config_store::get_string_or(pool, "emby_url", DEFAULT_EMBY_URL)
            .await
            .map_err(|err| err.to_string())?;
        let api_key = config_store::get_string_or(pool, "api_key", "")
            .await
            .map_err(|err| err.to_string())?;
        Ok(EmbyProbeConfig { base_url, api_key })
    })
    .await
    .map_err(|_| "500ms 内未完成 Emby 配置读取".to_string())?
}

pub async fn probe_emby_health(
    base_url: &str,
    api_key: &str,
    http: &reqwest::Client,
) -> EmbyHealthSummary {
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    let api_key = api_key.trim();
    let configured = !base_url.is_empty() && !api_key.is_empty();
    let mut summary = EmbyHealthSummary {
        configured,
        online: false,
        status: if configured {
            "checking".to_string()
        } else if api_key.is_empty() {
            "missing_api_key".to_string()
        } else {
            "bad_url".to_string()
        },
        base_url: base_url.clone(),
        http_status: None,
        version: None,
        server_name: None,
        server_id: None,
        operating_system: None,
        warning: None,
    };

    if base_url.is_empty() {
        summary.warning = Some("Emby URL 未配置，跳过在线/版本探测".to_string());
        return summary;
    }
    if api_key.is_empty() {
        summary.warning = Some("Emby api_key 未配置，跳过在线/版本探测".to_string());
        return summary;
    }

    let endpoint = format!("{base_url}/System/Info");
    let Ok(url) = reqwest::Url::parse(&endpoint) else {
        summary.status = "bad_url".to_string();
        summary.warning = Some(format!("Emby URL 无法解析: {base_url}"));
        return summary;
    };

    let response = match timeout(
        Duration::from_secs(2),
        http.get(url).query(&[("api_key", api_key)]).send(),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            summary.status = "unavailable".to_string();
            summary.warning = Some(format!(
                "Emby /System/Info 不可用: {}",
                shorten(&err.to_string(), 220)
            ));
            return summary;
        }
        Err(_) => {
            summary.status = "timeout".to_string();
            summary.warning = Some("2000ms 内未完成 Emby /System/Info 探测".to_string());
            return summary;
        }
    };

    let status = response.status();
    summary.http_status = Some(status.as_u16());
    summary.online = true;
    if !status.is_success() {
        summary.status = "http_error".to_string();
        summary.warning = Some(format!("Emby /System/Info 返回 HTTP {}", status.as_u16()));
        return summary;
    }

    let info = match timeout(
        Duration::from_millis(700),
        response.json::<EmbySystemInfo>(),
    )
    .await
    {
        Ok(Ok(info)) => info,
        Ok(Err(err)) => {
            summary.status = "parse_error".to_string();
            summary.warning = Some(format!(
                "Emby /System/Info 响应解析失败: {}",
                shorten(&err.to_string(), 220)
            ));
            return summary;
        }
        Err(_) => {
            summary.status = "timeout".to_string();
            summary.warning = Some("700ms 内未完成 Emby /System/Info 响应解析".to_string());
            return summary;
        }
    };

    summary.version = non_empty_string(info.version);
    summary.server_name = non_empty_string(info.server_name);
    summary.server_id = non_empty_string(info.server_id);
    summary.operating_system = non_empty_string(info.operating_system_display_name)
        .or_else(|| non_empty_string(info.operating_system));

    if summary.version.is_some() {
        summary.status = "ok".to_string();
    } else {
        summary.status = "partial".to_string();
        summary.warning = Some("Emby 在线，但 /System/Info 未返回 Version".to_string());
    }

    summary
}

#[derive(Debug, Deserialize)]
struct EmbySystemInfo {
    #[serde(rename = "Version")]
    version: Option<String>,
    #[serde(rename = "ServerName")]
    server_name: Option<String>,
    #[serde(rename = "Id")]
    server_id: Option<String>,
    #[serde(rename = "OperatingSystem")]
    operating_system: Option<String>,
    #[serde(rename = "OperatingSystemDisplayName")]
    operating_system_display_name: Option<String>,
}

fn non_empty_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn json_string_field(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|value| match value {
            Value::String(text) => Some(text.trim().to_string()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .unwrap_or_default()
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
    let output = StdCommand::new("df")
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
