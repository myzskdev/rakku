use std::fs;

use anyhow::{bail, Context, Result};

use crate::cli::SelfUpdateArgs;
use crate::config::{APP_NAME, APP_REPO};
use crate::github::GitHubClient;

pub fn run(args: SelfUpdateArgs) -> Result<()> {
    let repo = args.repo.unwrap_or_else(|| APP_REPO.to_string());
    if repo.starts_with("YOUR_GITHUB_USER/") || repo.starts_with("YOUR_ORG/") {
        bail!(
            "APP_REPO is still set to placeholder `{repo}`. Pass `--repo <owner/repo>` (or update `APP_REPO` in src/config.rs once published)."
        );
    }

    let arch = current_cli_arch()?;
    let asset_name = format!("{APP_NAME}-linux-{arch}");
    let current_ver = env!("CARGO_PKG_VERSION");

    println!("Checking {repo} for latest {APP_NAME} release (current: v{current_ver})...");
    let gh = GitHubClient::new();
    let release = gh.fetch_cli_release(&repo, args.version.as_deref(), &asset_name)?;

    if release.version == current_ver && args.version.is_none() {
        println!("{APP_NAME} is already up to date (v{current_ver}).");
        return Ok(());
    }

    println!("Found v{} ({}).", release.version, release.asset_name);
    if args.check {
        println!("Run `{APP_NAME} self-update` to install it.");
        return Ok(());
    }

    if !cfg!(target_os = "linux") {
        bail!("self-update downloads Linux release binaries and is supported on Linux hosts");
    }

    let exe_path = std::env::current_exe().context("failed to locate current executable path")?;
    let parent = exe_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine parent directory of executable"))?;
    let tmp_path = parent.join(format!(".{APP_NAME}.new"));

    println!("Downloading {}...", release.download_url);
    gh.download_binary(&release, &tmp_path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("failed to chmod {}", tmp_path.display()))?;
    }

    fs::rename(&tmp_path, &exe_path).with_context(|| {
        format!(
            "failed to replace {} (try running with `sudo`)",
            exe_path.display()
        )
    })?;

    println!(
        "Updated {APP_NAME} v{current_ver} -> v{} at {}",
        release.version,
        exe_path.display()
    );
    Ok(())
}

fn current_cli_arch() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        other => bail!("unsupported architecture for self-update: {other}"),
    }
}
