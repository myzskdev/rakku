use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::config::APP_NAME;
use crate::state::{ContainerRuntime, RunnerRecord};
use crate::system;
use crate::system::metrics::JobState;

/// Map `std::env::consts::ARCH` to GitLab Runner's Linux binary architecture suffix.
pub fn current_arch() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        "arm" => Ok("arm"),
        other => bail!("unsupported CPU architecture for GitLab Runner: {other}"),
    }
}

/// Build the official GitLab Runner Linux binary download URL.
pub fn download_url(requested_version: Option<&str>, arch: &str) -> (String, String) {
    match requested_version {
        Some(v) => {
            let clean = v.trim_start_matches('v');
            (
                clean.to_string(),
                format!(
                    "https://gitlab-runner-downloads.s3.amazonaws.com/v{clean}/binaries/gitlab-runner-linux-{arch}"
                ),
            )
        }
        None => (
            "latest".to_string(),
            format!(
                "https://gitlab-runner-downloads.s3.amazonaws.com/latest/binaries/gitlab-runner-linux-{arch}"
            ),
        ),
    }
}

/// Download the `gitlab-runner` static binary into `dest_dir/gitlab-runner` and mark it executable.
pub fn download_binary(requested_version: Option<&str>, dest_dir: &Path) -> Result<String> {
    let arch = current_arch()?;
    let (ver_label, url) = download_url(requested_version, arch);

    fs::create_dir_all(dest_dir)
        .with_context(|| format!("failed to create runner directory {}", dest_dir.display()))?;

    let bin_path = dest_dir.join("gitlab-runner");
    let agent = ureq::AgentBuilder::new()
        .user_agent(&format!("{APP_NAME}/{}", env!("CARGO_PKG_VERSION")))
        .build();

    let resp = agent
        .get(&url)
        .call()
        .map_err(|e| anyhow!("failed to download GitLab Runner from {url}: {e}"))?;

    let total_bytes = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok());
    let mut reader = resp.into_reader();
    let mut out = File::create(&bin_path)
        .with_context(|| format!("failed to create {}", bin_path.display()))?;
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_print = std::time::Instant::now();
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        downloaded += n as u64;
        if last_print.elapsed() >= std::time::Duration::from_millis(200) {
            crate::github::print_download_progress(downloaded, total_bytes);
            last_print = std::time::Instant::now();
        }
    }
    crate::github::print_download_progress(downloaded, total_bytes);
    println!();
    out.flush()?;
    drop(out);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755))?;
    }

    Ok(ver_label)
}

/// Register a GitLab Runner into `<install_dir>/config.toml` as the dedicated Linux user.
#[allow(clippy::too_many_arguments)]
pub fn register(
    linux_user: &str,
    install_dir: &Path,
    url: &str,
    token: &str,
    name: &str,
    labels: &[String],
    runtime: ContainerRuntime,
    executor_override: Option<&str>,
    docker_image: &str,
) -> Result<()> {
    let bin = install_dir.join("gitlab-runner");
    let bin_str = bin
        .to_str()
        .ok_or_else(|| anyhow!("runner path contains invalid UTF-8"))?;
    let config_path = install_dir.join("config.toml");
    let config_str = config_path
        .to_str()
        .ok_or_else(|| anyhow!("config path contains invalid UTF-8"))?;

    let executor = executor_override.unwrap_or(match runtime {
        ContainerRuntime::Docker => "docker",
        ContainerRuntime::Podman | ContainerRuntime::None => "shell",
    });

    let mut args = vec![
        "register".to_string(),
        "--non-interactive".to_string(),
        "--config".to_string(),
        config_str.to_string(),
        "--url".to_string(),
        url.to_string(),
        "--name".to_string(),
        name.to_string(),
        "--executor".to_string(),
        executor.to_string(),
    ];

    // GitLab's modern runner authentication tokens start with `glrt-` and use `--token`.
    // Legacy project/group registration tokens use `--registration-token` (and support `--tag-list`).
    if token.starts_with("glrt-") {
        args.push("--token".to_string());
        args.push(token.to_string());
    } else {
        args.push("--registration-token".to_string());
        args.push(token.to_string());
        if !labels.is_empty() {
            args.push("--tag-list".to_string());
            args.push(labels.join(","));
        }
    }

    if executor == "docker" {
        args.push("--docker-image".to_string());
        args.push(docker_image.to_string());
    }

    system::run_as_user(linux_user, install_dir, bin_str, &args)?;
    Ok(())
}

/// Unregister a GitLab Runner cleanly from its GitLab instance.
pub fn deregister(record: &RunnerRecord) -> Result<()> {
    let bin = record.install_dir.join("gitlab-runner");
    let config_path = record.install_dir.join("config.toml");
    if !bin.exists() || !config_path.exists() {
        bail!(
            "gitlab-runner binary or config.toml not found in {}",
            record.install_dir.display()
        );
    }
    let bin_str = bin
        .to_str()
        .ok_or_else(|| anyhow!("runner path contains invalid UTF-8"))?;
    let config_str = config_path
        .to_str()
        .ok_or_else(|| anyhow!("config path contains invalid UTF-8"))?;

    let args = vec![
        "unregister".to_string(),
        "--config".to_string(),
        config_str.to_string(),
        "--all-runners".to_string(),
    ];
    system::run_as_user(&record.linux_user, &record.install_dir, bin_str, &args)?;
    Ok(())
}

pub fn exec_start(record: &RunnerRecord) -> String {
    let bin = record.install_dir.join("gitlab-runner");
    let config = record.install_dir.join("config.toml");
    format!(
        "{} run --config {} --working-directory {}",
        bin.to_string_lossy(),
        config.to_string_lossy(),
        record.install_dir.to_string_lossy()
    )
}

pub fn infer_job_state(lines: &[String]) -> JobState {
    for line in lines.iter().rev() {
        if line.contains("FATAL:") || line.contains("ERROR: Checking for jobs... forbidden") {
            return JobState::Error("Auth/connect error".into());
        }
        if line.contains("Job succeeded")
            || line.contains("Job failed")
            || line.contains("Configuration loaded")
            || line.contains("Starting multi-runner")
        {
            return JobState::Idle;
        }
        if line.contains("Processing runner") || line.contains("Job received") {
            return JobState::Running("gitlab job".into());
        }
    }
    JobState::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_gitlab_download_urls() {
        let (ver, url) = download_url(Some("v17.9.0"), "amd64");
        assert_eq!(ver, "17.9.0");
        assert_eq!(
            url,
            "https://gitlab-runner-downloads.s3.amazonaws.com/v17.9.0/binaries/gitlab-runner-linux-amd64"
        );
    }

    #[test]
    fn infers_gitlab_job_states() {
        let busy = vec![
            "Starting multi-runner".into(),
            "Job received job=12345".into(),
        ];
        assert_eq!(
            infer_job_state(&busy),
            JobState::Running("gitlab job".into())
        );

        let idle = vec![
            "Job received job=12345".into(),
            "Job succeeded duration_s=12.4".into(),
        ];
        assert_eq!(infer_job_state(&idle), JobState::Idle);
    }
}
