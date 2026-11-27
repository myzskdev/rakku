use std::path::PathBuf;

/// Application name (`rakku` — 楽 raku + 駆 ku, by fukulabs.dev).
pub const APP_NAME: &str = "rakku";

/// GitHub repository (`owner/repo`) used by `self-update`.
pub const APP_REPO: &str = "myzskdev/rakku";

/// Default Linux user created to run self-hosted runners.
pub const DEFAULT_RUNNER_USER: &str = "rakku";

/// Default directory for state storage (`/var/lib/rakku`).
pub const DEFAULT_STATE_DIR: &str = "/var/lib/rakku";

/// Default root directory where individual runners are installed (`/opt/rakku/runners`).
pub const DEFAULT_INSTALL_ROOT: &str = "/opt/rakku/runners";

/// Directory where systemd unit files are placed.
pub const SYSTEMD_UNIT_DIR: &str = "/etc/systemd/system";

/// Prefix for systemd service names (`rakku-<runner_name>.service`).
pub const SERVICE_PREFIX: &str = "rakku";

/// GitHub API base URL.
pub const GITHUB_API_BASE: &str = "https://api.github.com";

/// GitHub runner repository (`owner/repo`).
pub const ACTIONS_RUNNER_REPO: &str = "actions/runner";

/// Resolve the path to the local state file.
/// Honors `RAKKU_STATE_FILE` for testing or custom deployments.
pub fn state_file_path() -> PathBuf {
    if let Ok(custom) = std::env::var("RAKKU_STATE_FILE") {
        return PathBuf::from(custom);
    }
    PathBuf::from(DEFAULT_STATE_DIR).join("state.json")
}

/// Resolve the installation directory for a named runner.
pub fn runner_install_dir(install_root: Option<&str>, name: &str) -> PathBuf {
    let root = install_root.unwrap_or(DEFAULT_INSTALL_ROOT);
    PathBuf::from(root).join(name)
}

/// Build the systemd unit name for a runner (e.g., `rakku-my-runner.service`).
pub fn systemd_service_name(runner_name: &str) -> String {
    format!(
        "{}-{}.service",
        SERVICE_PREFIX,
        sanitize_unit_part(runner_name)
    )
}

/// Build the full path to the systemd unit file.
pub fn systemd_unit_path(runner_name: &str) -> PathBuf {
    PathBuf::from(SYSTEMD_UNIT_DIR).join(systemd_service_name(runner_name))
}

/// Sanitize a runner name into a systemd-safe unit suffix.
pub fn sanitize_unit_part(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_service_names() {
        assert_eq!(
            systemd_service_name("prod/web 01"),
            "rakku-prod-web-01.service"
        );
        assert_eq!(
            systemd_service_name("runner_1.east"),
            "rakku-runner_1.east.service"
        );
    }
}
