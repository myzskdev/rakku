pub mod metrics;
pub mod preflight;
pub mod systemd;
pub mod user;

use std::path::Path;
use std::process::{Command, Output};

use anyhow::{bail, Context, Result};

/// Ensure the command is running on Linux with root privileges (required for useradd, /opt, systemd).
pub fn require_linux_root() -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!(
            "this command modifies Linux users and systemd services and must be run on a Linux host (current OS: {})",
            std::env::consts::OS
        );
    }

    let out = Command::new("id")
        .arg("-u")
        .output()
        .context("failed to execute `id -u`")?;
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if uid != "0" {
        bail!("root privileges are required; please re-run with `sudo`");
    }
    Ok(())
}

/// Best-effort local hostname lookup.
pub fn hostname() -> String {
    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let trimmed = h.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(out) = Command::new("hostname").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    "linux-runner".to_string()
}

/// Run a command and fail with stderr/stdout details if it exits non-zero.
pub fn run_checked(program: &str, args: &[&str]) -> Result<Output> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute `{program} {}`", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "command `{program} {}` failed (exit {:?}):\nstdout: {}\nstderr: {}",
            args.join(" "),
            output.status.code(),
            stdout.trim(),
            stderr.trim()
        );
    }
    Ok(output)
}

/// Execute a command as a non-root Linux user inside `cwd`.
/// Uses `runuser` (available on virtually all modern Linux distros) with a `su` fallback.
pub fn run_as_user(user: &str, cwd: &Path, program: &str, args: &[String]) -> Result<Output> {
    let mut cmd = if command_exists("runuser") {
        let mut c = Command::new("runuser");
        c.arg("-u").arg(user).arg("--").arg(program).args(args);
        c
    } else {
        let escaped = std::iter::once(program.to_string())
            .chain(args.iter().cloned())
            .map(|a| shell_escape(&a))
            .collect::<Vec<_>>()
            .join(" ");
        let mut c = Command::new("su");
        c.arg("-s")
            .arg("/bin/bash")
            .arg(user)
            .arg("-c")
            .arg(escaped);
        c
    };

    let output = cmd
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to execute `{program}` as user `{user}`"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "`{program}` as user `{user}` failed (exit {:?}):\nstdout: {}\nstderr: {}",
            output.status.code(),
            stdout.trim(),
            stderr.trim()
        );
    }

    Ok(output)
}

pub fn command_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
