use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::github::GitHubClient;
use crate::system::{self, command_exists, run_checked};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Apt,
    Dnf,
    Yum,
    Pacman,
    Unknown,
}

impl PackageManager {
    pub fn detect() -> Self {
        if command_exists("apt-get") {
            Self::Apt
        } else if command_exists("dnf") {
            Self::Dnf
        } else if command_exists("yum") {
            Self::Yum
        } else if command_exists("pacman") {
            Self::Pacman
        } else {
            Self::Unknown
        }
    }

    pub fn install_hint(&self, pkg: &str) -> String {
        match self {
            Self::Apt => format!("sudo apt-get update && sudo apt-get install -y {pkg}"),
            Self::Dnf => format!("sudo dnf install -y {pkg}"),
            Self::Yum => format!("sudo yum install -y {pkg}"),
            Self::Pacman => format!("sudo pacman -Sy --noconfirm {pkg}"),
            Self::Unknown => format!("install `{pkg}` using your distribution's package manager"),
        }
    }

    pub fn docker_package_name(&self) -> &'static str {
        match self {
            Self::Apt => "docker.io",
            Self::Dnf | Self::Yum | Self::Pacman | Self::Unknown => "docker",
        }
    }
}

/// Install one or more OS packages using the detected Linux package manager.
pub fn install_packages(pm: PackageManager, packages: &[&str]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    match pm {
        PackageManager::Apt => {
            run_checked("apt-get", &["update"])?;
            let mut args = vec!["install", "-y"];
            args.extend_from_slice(packages);
            run_checked("apt-get", &args)
                .with_context(|| format!("failed to install packages via apt-get: {packages:?}"))?;
        }
        PackageManager::Dnf => {
            let mut args = vec!["install", "-y"];
            args.extend_from_slice(packages);
            run_checked("dnf", &args)
                .with_context(|| format!("failed to install packages via dnf: {packages:?}"))?;
        }
        PackageManager::Yum => {
            let mut args = vec!["install", "-y"];
            args.extend_from_slice(packages);
            run_checked("yum", &args)
                .with_context(|| format!("failed to install packages via yum: {packages:?}"))?;
        }
        PackageManager::Pacman => {
            let mut args = vec!["-Sy", "--noconfirm"];
            args.extend_from_slice(packages);
            run_checked("pacman", &args)
                .with_context(|| format!("failed to install packages via pacman: {packages:?}"))?;
        }
        PackageManager::Unknown => {
            bail!(
                "could not detect a supported package manager (apt-get, dnf, yum, pacman); please install {} manually",
                packages.join(", ")
            );
        }
    }
    Ok(())
}

/// Install Docker and enable/start `docker.service`.
pub fn install_docker(pm: PackageManager) -> Result<()> {
    match pm {
        PackageManager::Dnf | PackageManager::Yum => {
            // On RHEL/Rocky/Alma/Fedora, try Docker CE repo first if `docker` isn't in base repos.
            if install_packages(pm, &["docker"]).is_err() {
                let _ = install_packages(pm, &["dnf-plugins-core"]);
                let _ = run_checked(
                    "dnf",
                    &[
                        "config-manager",
                        "--add-repo",
                        "https://download.docker.com/linux/centos/docker-ce.repo",
                    ],
                );
                install_packages(pm, &["docker-ce", "docker-ce-cli", "containerd.io"])?;
            }
        }
        _ => {
            let pkg = pm.docker_package_name();
            install_packages(pm, &[pkg])?;
        }
    }
    let _ = run_checked("systemctl", &["enable", "--now", "docker"]);
    Ok(())
}

/// Install Podman (plus `podman-docker` CLI compatibility when available).
pub fn install_podman(pm: PackageManager) -> Result<()> {
    install_packages(pm, &["podman"])?;
    // Best-effort install of `podman-docker` so `docker ...` commands in workflows work seamlessly.
    let _ = install_packages(pm, &["podman-docker"]);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckSeverity {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct CheckItem {
    pub name: String,
    pub severity: CheckSeverity,
    pub message: String,
    pub hint: Option<String>,
}

pub struct PreflightReport {
    pub pm: PackageManager,
    pub items: Vec<CheckItem>,
}

impl PreflightReport {
    pub fn has_failures(&self) -> bool {
        self.items.iter().any(|i| i.severity == CheckSeverity::Fail)
    }

    pub fn print(&self) {
        for item in &self.items {
            let tag = match item.severity {
                CheckSeverity::Ok => "[OK]  ",
                CheckSeverity::Warn => "[WARN]",
                CheckSeverity::Fail => "[FAIL]",
            };
            println!("{tag} {:<16} {}", item.name, item.message);
            if let Some(hint) = &item.hint {
                println!("       -> {hint}");
            }
        }
    }
}

/// Run host environment diagnostics for Linux runner readiness.
pub fn run_diagnostics(require_docker: bool, check_network: bool) -> PreflightReport {
    let pm = PackageManager::detect();
    let mut items = Vec::new();

    // 1. OS check
    if cfg!(target_os = "linux") {
        items.push(CheckItem {
            name: "OS".into(),
            severity: CheckSeverity::Ok,
            message: format!("Linux ({})", std::env::consts::ARCH),
            hint: None,
        });
    } else {
        items.push(CheckItem {
            name: "OS".into(),
            severity: CheckSeverity::Fail,
            message: format!(
                "{} (Linux is required to host runners)",
                std::env::consts::OS
            ),
            hint: Some("Deploy the compiled Linux binary on your Linux VPS/VM.".into()),
        });
    }

    // 2. systemd check
    let has_systemd = cfg!(target_os = "linux")
        && Path::new("/run/systemd/system").exists()
        && command_exists("systemctl");
    items.push(CheckItem {
        name: "systemd".into(),
        severity: if has_systemd {
            CheckSeverity::Ok
        } else {
            CheckSeverity::Fail
        },
        message: if has_systemd {
            "active and available".into()
        } else {
            "not detected (systemctl or /run/systemd/system missing)".into()
        },
        hint: if has_systemd {
            None
        } else {
            Some("Run on a systemd-enabled Linux host (most Ubuntu, Debian, Rocky, Alma, Fedora VMs).".into())
        },
    });

    // 3. Core system utilities
    let required_bins = ["useradd", "usermod", "chown", "bash"];
    let missing_bins: Vec<&str> = required_bins
        .iter()
        .copied()
        .filter(|b| !command_exists(b))
        .collect();
    items.push(CheckItem {
        name: "Linux tools".into(),
        severity: if missing_bins.is_empty() {
            CheckSeverity::Ok
        } else {
            CheckSeverity::Fail
        },
        message: if missing_bins.is_empty() {
            "useradd, usermod, chown, bash present".into()
        } else {
            format!("missing: {}", missing_bins.join(", "))
        },
        hint: if missing_bins.is_empty() {
            None
        } else {
            Some("Install core shadow-utils / passwd and bash packages.".into())
        },
    });

    // 4. git (needed by almost every GitHub Actions workflow via actions/checkout)
    let has_git = command_exists("git");
    items.push(CheckItem {
        name: "git".into(),
        severity: if has_git {
            CheckSeverity::Ok
        } else {
            CheckSeverity::Warn
        },
        message: if has_git {
            "installed".into()
        } else {
            "not installed (needed by actions/checkout in workflows)".into()
        },
        hint: if has_git {
            None
        } else {
            Some(pm.install_hint("git"))
        },
    });

    // 5. Container runtime (docker / podman)
    let has_docker = command_exists("docker");
    let has_podman = command_exists("podman");
    let runtime_msg = match (has_docker, has_podman) {
        (true, true) => "docker and podman installed".to_string(),
        (true, false) => "docker installed".to_string(),
        (false, true) => "podman installed".to_string(),
        (false, false) => "none installed (optional unless workflows use containers)".to_string(),
    };
    items.push(CheckItem {
        name: "Containers".into(),
        severity: if has_docker || has_podman {
            CheckSeverity::Ok
        } else if require_docker {
            CheckSeverity::Fail
        } else {
            CheckSeverity::Warn
        },
        message: runtime_msg,
        hint: if has_docker || has_podman {
            None
        } else {
            Some(format!(
                "Docker: `{}` | Podman: `{}`",
                pm.install_hint(pm.docker_package_name()),
                pm.install_hint("podman")
            ))
        },
    });

    // 6. GitHub API reachability
    if check_network {
        match GitHubClient::new().check_api_reachable() {
            Ok(()) => items.push(CheckItem {
                name: "GitHub API".into(),
                severity: CheckSeverity::Ok,
                message: "api.github.com reachable".into(),
                hint: None,
            }),
            Err(err) => items.push(CheckItem {
                name: "GitHub API".into(),
                severity: CheckSeverity::Fail,
                message: format!("unreachable ({err})"),
                hint: Some("Verify outbound HTTPS (port 443) and DNS on this server.".into()),
            }),
        }
    }

    // 7. Root privilege info (informational for doctor)
    let _ = system::command_exists("journalctl");

    PreflightReport { pm, items }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_distro_install_hints() {
        assert!(PackageManager::Apt
            .install_hint("git")
            .contains("apt-get install -y git"));
        assert!(PackageManager::Dnf
            .install_hint("git")
            .contains("dnf install -y git"));
        assert_eq!(PackageManager::Apt.docker_package_name(), "docker.io");
        assert_eq!(PackageManager::Dnf.docker_package_name(), "docker");
    }
}
