use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::APP_NAME;
use crate::github::extract_sha256_from_release_body;
use crate::state::RunnerRecord;
use crate::system;
use crate::system::metrics::JobState;

#[derive(Debug, Deserialize)]
struct GiteaRelease {
    tag_name: String,
    assets: Vec<GiteaAsset>,
}

#[derive(Debug, Deserialize)]
struct GiteaAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone)]
pub struct GiteaReleaseInfo {
    pub version: String,
    pub asset_name: String,
    pub download_url: String,
    pub expected_sha256: Option<String>,
}

/// Map `std::env::consts::ARCH` to Gitea Runner's Linux architecture suffix.
pub fn current_arch() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        other => bail!("unsupported CPU architecture for Gitea runner: {other}"),
    }
}

/// Resolve the latest (or requested) Gitea runner release directly from Gitea's Releases API.
/// Supports both modern `gitea-runner-<ver>-linux-<arch>` and legacy `act_runner-<ver>-linux-<arch>`.
pub fn resolve_release(requested_version: Option<&str>, arch: &str) -> Result<GiteaReleaseInfo> {
    let api_url = match requested_version {
        Some(v) => {
            let tag = if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{v}")
            };
            format!("https://gitea.com/api/v1/repos/gitea/runner/releases/tags/{tag}")
        }
        None => "https://gitea.com/api/v1/repos/gitea/runner/releases/latest".to_string(),
    };

    let agent = ureq::AgentBuilder::new()
        .user_agent(&format!("{APP_NAME}/{}", env!("CARGO_PKG_VERSION")))
        .build();

    let rel: GiteaRelease = agent
        .get(&api_url)
        .call()
        .map_err(|e| anyhow!("failed to query Gitea runner release ({api_url}): {e}"))?
        .into_json()
        .context("failed to parse Gitea runner release JSON")?;

    let version = rel.tag_name.trim_start_matches('v').to_string();
    let suffix = format!("-linux-{arch}");
    let checksum_url = rel
        .assets
        .iter()
        .find(|a| a.name == "checksums.txt")
        .map(|a| a.browser_download_url.clone());

    let asset = rel
        .assets
        .into_iter()
        .find(|a| {
            a.name.ends_with(&suffix)
                && (a.name.starts_with("gitea-runner-") || a.name.starts_with("act_runner-"))
        })
        .ok_or_else(|| {
            anyhow!(
                "Gitea runner release {} has no uncompressed binary matching '*{suffix}'",
                rel.tag_name
            )
        })?;

    let expected_sha256 = checksum_url.and_then(|url| {
        let body = agent.get(&url).call().ok()?.into_string().ok()?;
        extract_sha256_from_release_body(&body, &asset.name)
    });

    Ok(GiteaReleaseInfo {
        version,
        asset_name: asset.name,
        download_url: asset.browser_download_url,
        expected_sha256,
    })
}

/// Download the Gitea runner static binary into `dest_dir/act_runner`, verify SHA-256, and chmod 0755.
pub fn download_binary(requested_version: Option<&str>, dest_dir: &Path) -> Result<String> {
    let arch = current_arch()?;
    let release = resolve_release(requested_version, arch)?;

    fs::create_dir_all(dest_dir)
        .with_context(|| format!("failed to create runner directory {}", dest_dir.display()))?;

    let bin_path = dest_dir.join("act_runner");
    let agent = ureq::AgentBuilder::new()
        .user_agent(&format!("{APP_NAME}/{}", env!("CARGO_PKG_VERSION")))
        .build();

    let resp = agent.get(&release.download_url).call().map_err(|e| {
        anyhow!(
            "failed to download Gitea runner from {}: {e}",
            release.download_url
        )
    })?;

    let total_bytes = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok());
    let mut reader = resp.into_reader();
    let mut out = File::create(&bin_path)
        .with_context(|| format!("failed to create {}", bin_path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_print = std::time::Instant::now();
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
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

    let actual_sha = format!("{:x}", hasher.finalize());
    if let Some(expected) = &release.expected_sha256 {
        if !actual_sha.eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&bin_path);
            bail!(
                "SHA-256 mismatch for {}: expected {}, got {}",
                release.asset_name,
                expected,
                actual_sha
            );
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755))?;
    }

    Ok(release.version)
}

/// Register a Gitea Actions runner as the dedicated Linux user.
pub fn register(
    linux_user: &str,
    install_dir: &Path,
    instance_url: &str,
    token: &str,
    name: &str,
    labels: &[String],
) -> Result<()> {
    let bin = install_dir.join("act_runner");
    let bin_str = bin
        .to_str()
        .ok_or_else(|| anyhow!("runner path contains invalid UTF-8"))?;

    let mut args = vec![
        "register".to_string(),
        "--no-interactive".to_string(),
        "--instance".to_string(),
        instance_url.to_string(),
        "--token".to_string(),
        token.to_string(),
        "--name".to_string(),
        name.to_string(),
    ];
    if !labels.is_empty() {
        args.push("--labels".to_string());
        args.push(labels.join(","));
    }

    system::run_as_user(linux_user, install_dir, bin_str, &args)?;
    Ok(())
}

pub fn exec_start(record: &RunnerRecord) -> String {
    let bin = record.install_dir.join("act_runner");
    format!("{} daemon", bin.to_string_lossy())
}

pub fn infer_job_state(lines: &[String]) -> JobState {
    for line in lines.iter().rev() {
        if line.contains("cannot ping the Gitea instance") || line.contains("level=error") {
            return JobState::Error("Connect error".into());
        }
        if line.contains("task ") && line.contains("finished") {
            return JobState::Idle;
        }
        if line.contains("running task") || line.contains("executing task") {
            return JobState::Running("gitea task".into());
        }
        if line.contains("declared successfully") || line.contains("Waiting for tasks") {
            return JobState::Idle;
        }
    }
    JobState::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_gitea_job_states() {
        let busy = vec![
            "level=info msg=\"runner: vps-1 declared successfully\"".into(),
            "level=info msg=\"running task 42\"".into(),
        ];
        assert_eq!(
            infer_job_state(&busy),
            JobState::Running("gitea task".into())
        );

        let idle = vec![
            "level=info msg=\"running task 42\"".into(),
            "level=info msg=\"task 42 finished\"".into(),
        ];
        assert_eq!(infer_job_state(&idle), JobState::Idle);
    }
}
