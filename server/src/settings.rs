use anyhow::Context;
use std::{env, path::PathBuf};

#[derive(Clone, Debug)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub web_dist: PathBuf,
    pub legacy_dir: PathBuf,
    pub bootstrap_password: String,
    pub cd_root: PathBuf,
    pub strm_root: PathBuf,
    pub docker_bin: PathBuf,
    pub task_concurrency: usize,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            host: env::var("EMBY_MANAGER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: env::var("EMBY_MANAGER_PORT")
                .unwrap_or_else(|_| "8098".to_string())
                .parse()
                .context("EMBY_MANAGER_PORT must be a u16")?,
            database_url: env::var("DATABASE_URL").context("DATABASE_URL is required")?,
            web_dist: env::var("EMBY_MANAGER_WEB_DIST")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/app/web")),
            legacy_dir: env::var("EMBY_MANAGER_LEGACY_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/legacy")),
            bootstrap_password: validate_bootstrap_password(
                env::var("EMBY_MANAGER_BOOTSTRAP_PASSWORD").context(
                    "EMBY_MANAGER_BOOTSTRAP_PASSWORD is required for first-run admin setup",
                )?,
            )?,
            cd_root: env::var("EMBY_MANAGER_CD_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    PathBuf::from("/volume1/docker/clouddrive2/CloudNAS/CloudDrive")
                }),
            strm_root: env::var("EMBY_MANAGER_STRM_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/volume1/strm")),
            docker_bin: env::var("EMBY_MANAGER_DOCKER_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    PathBuf::from("/var/packages/ContainerManager/target/usr/bin/docker")
                }),
            task_concurrency: validate_task_concurrency(
                env::var("EMBY_MANAGER_TASK_CONCURRENCY")
                    .unwrap_or_else(|_| "3".to_string())
                    .parse()
                    .context("EMBY_MANAGER_TASK_CONCURRENCY must be a number")?,
            )?,
        })
    }
}

fn validate_bootstrap_password(password: String) -> anyhow::Result<String> {
    let trimmed = password.trim();
    if trimmed.len() < 12 {
        anyhow::bail!("EMBY_MANAGER_BOOTSTRAP_PASSWORD must be at least 12 characters");
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "admin" | "password" | "emby_manager" | "changeme"
    ) {
        anyhow::bail!("EMBY_MANAGER_BOOTSTRAP_PASSWORD is too weak");
    }
    if lower.contains("change-me") || lower.contains("changeme") || lower.contains("example") {
        anyhow::bail!("EMBY_MANAGER_BOOTSTRAP_PASSWORD must not be a placeholder value");
    }
    Ok(trimmed.to_string())
}

fn validate_task_concurrency(value: usize) -> anyhow::Result<usize> {
    if value == 0 {
        anyhow::bail!("EMBY_MANAGER_TASK_CONCURRENCY must be at least 1");
    }
    if value > 64 {
        anyhow::bail!("EMBY_MANAGER_TASK_CONCURRENCY must be 64 or lower");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{validate_bootstrap_password, validate_task_concurrency};

    #[test]
    fn rejects_weak_bootstrap_passwords() {
        assert!(validate_bootstrap_password("admin".to_string()).is_err());
        assert!(validate_bootstrap_password("short".to_string()).is_err());
        assert!(
            validate_bootstrap_password("change-me-to-a-long-random-password".to_string()).is_err()
        );
        assert!(validate_bootstrap_password("example-bootstrap-password".to_string()).is_err());
        assert!(validate_bootstrap_password("a-strong-preview-password".to_string()).is_ok());
    }

    #[test]
    fn validates_task_concurrency_bounds() {
        assert!(validate_task_concurrency(0).is_err());
        assert_eq!(validate_task_concurrency(1).unwrap(), 1);
        assert_eq!(validate_task_concurrency(64).unwrap(), 64);
        assert!(validate_task_concurrency(65).is_err());
    }
}
