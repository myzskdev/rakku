use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::SERVICE_PREFIX;
use crate::state::ContainerRuntime;
use crate::system::{command_exists, run_checked};

/// Ensure the dedicated Linux runner user exists and has the requested runtime/sudo permissions.
pub fn ensure_linux_user(
    username: &str,
    runtime: ContainerRuntime,
    sudo_access: bool,
) -> Result<()> {
    validate_username(username)?;

    if !user_exists(username) {
        run_checked("useradd", &["-m", "-s", "/bin/bash", username])
            .with_context(|| format!("failed to create Linux user `{username}`"))?;
    }

    match runtime {
        ContainerRuntime::None => {}
        ContainerRuntime::Docker => {
            if group_exists("docker") {
                run_checked("usermod", &["-aG", "docker", username])
                    .with_context(|| format!("failed to add `{username}` to `docker` group"))?;
            } else {
                eprintln!(
                    "warning: Docker runtime selected, but `docker` group does not exist yet on this host"
                );
            }
        }
        ContainerRuntime::Podman => {
            // Enable systemd user lingering so rootless Podman works cleanly inside the runner service.
            if command_exists("loginctl") {
                if let Err(err) = run_checked("loginctl", &["enable-linger", username]) {
                    eprintln!(
                        "warning: failed to enable systemd linger for `{username}` ({err}); rootless Podman may require manual `loginctl enable-linger {username}`"
                    );
                }
            }
        }
    }

    if sudo_access {
        grant_passwordless_sudo(username)?;
    }

    Ok(())
}

/// Recursively set ownership of `path` to `<username>:<username>`.
pub fn chown_recursive(path: &Path, username: &str) -> Result<()> {
    let owner_spec = format!("{username}:{username}");
    let path_str = path.to_string_lossy();
    run_checked("chown", &["-R", &owner_spec, &path_str])
        .with_context(|| format!("failed to chown {} to {owner_spec}", path.display()))?;
    Ok(())
}

fn user_exists(username: &str) -> bool {
    Command::new("id")
        .arg("-u")
        .arg(username)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn group_exists(group: &str) -> bool {
    Command::new("getent")
        .args(["group", group])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn grant_passwordless_sudo(username: &str) -> Result<()> {
    let sudoers_file = format!("/etc/sudoers.d/{SERVICE_PREFIX}-{username}");
    let content = format!("# Managed by {SERVICE_PREFIX}\n{username} ALL=(ALL) NOPASSWD:ALL\n");
    fs::write(&sudoers_file, content)
        .with_context(|| format!("failed to write sudoers file {sudoers_file}"))?;
    run_checked("chmod", &["0440", &sudoers_file])?;
    Ok(())
}

pub fn validate_username(username: &str) -> Result<()> {
    if username.is_empty() || username.len() > 32 {
        bail!("Linux username `{username}` must be 1..=32 characters");
    }
    if username == "root" {
        bail!("running a GitHub Actions runner as `root` is unsafe and not allowed; use a dedicated user");
    }
    let valid = username
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !valid {
        bail!(
            "Linux username `{username}` contains invalid characters (allowed: lowercase letters, digits, '-', '_')"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_linux_usernames() {
        assert!(validate_username("rakku").is_ok());
        assert!(validate_username("gh-runner_1").is_ok());
        assert!(validate_username("root").is_err());
        assert!(validate_username("Bad User").is_err());
        assert!(validate_username("").is_err());
    }
}
