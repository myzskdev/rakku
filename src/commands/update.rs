use std::fs;

use anyhow::{anyhow, bail, Result};
use chrono::Utc;

use crate::cli::UpdateArgs;
use crate::config::APP_NAME;
use crate::github::{current_runner_arch, GitHubClient};
use crate::providers::{gitea, gitlab};
use crate::state::{ProviderKind, StateStore};
use crate::system::metrics::{self, JobState};
use crate::system::{self, systemd, user};

pub fn run(args: UpdateArgs) -> Result<()> {
    system::require_linux_root()?;

    if !args.all && args.name.is_none() {
        bail!("specify a runner name or pass --all to upgrade every managed runner");
    }

    let mut store = StateStore::load_default()?;
    let target_names: Vec<String> = if args.all {
        store.runners.iter().map(|r| r.name.clone()).collect()
    } else {
        let name = args.name.unwrap();
        if store.find(&name).is_none() {
            return Err(anyhow!("runner `{name}` is not managed locally"));
        }
        vec![name]
    };

    if target_names.is_empty() {
        println!("No managed runners to upgrade.");
        return Ok(());
    }

    let mut gh_targets = Vec::new();
    let mut gl_targets = Vec::new();
    let mut gt_targets = Vec::new();
    for name in target_names {
        let rec = store.find(&name).unwrap();
        if !args.force {
            let live = metrics::collect_runner_status(rec);
            if let JobState::Running(job) = live.job_state {
                println!(
                    "  - `{name}` is currently busy running `{job}` (skipping; pass --force to interrupt)"
                );
                continue;
            }
        }
        match rec.provider {
            ProviderKind::Github => gh_targets.push(name),
            ProviderKind::Gitlab => gl_targets.push(name),
            ProviderKind::Gitea => gt_targets.push(name),
        }
    }

    let mut failed = Vec::new();

    // 1. Upgrade GitHub Actions runners (download tarball once before stopping any runner)
    if !gh_targets.is_empty() {
        let arch = current_runner_arch()?;
        let gh = GitHubClient::new();
        let release = gh.fetch_runner_release(args.runner_version.as_deref(), arch, None)?;
        println!("Target GitHub actions/runner version: v{}", release.version);

        let gh_to_update: Vec<String> = gh_targets
            .into_iter()
            .filter(|name| {
                let rec = store.find(name).unwrap();
                if rec.runner_version == release.version {
                    println!("  - `{name}` is already on v{} (skipping)", release.version);
                    false
                } else {
                    true
                }
            })
            .collect();

        if !gh_to_update.is_empty() {
            let tmp_archive =
                std::env::temp_dir().join(format!("{APP_NAME}-{}", release.asset_name));
            println!(
                "Downloading and verifying {} once before stopping runners...",
                release.asset_name
            );
            gh.download_tarball(&release, &tmp_archive)?;

            for name in gh_to_update {
                let rec = store.find(&name).unwrap().clone();
                println!(
                    "  - Upgrading `{name}` (github) v{} -> v{}...",
                    rec.runner_version, release.version
                );
                if let Err(err) = systemd::control_service(&rec.service_name, "stop") {
                    eprintln!("    error stopping `{name}`: {err}");
                    failed.push(name);
                    continue;
                }
                let apply_res = GitHubClient::extract_tarball(&tmp_archive, &rec.install_dir)
                    .and_then(|()| user::chown_recursive(&rec.install_dir, &rec.linux_user));
                if let Err(err) = apply_res {
                    eprintln!("    error upgrading `{name}`: {err}");
                    let _ = systemd::control_service(&rec.service_name, "start");
                    failed.push(name);
                    continue;
                }
                if let Err(err) = systemd::control_service(&rec.service_name, "start") {
                    eprintln!("    error starting `{name}`: {err}");
                    failed.push(name);
                    continue;
                }
                if let Some(entry) = store.find_mut(&name) {
                    entry.runner_version = release.version.clone();
                    entry.updated_at = Some(Utc::now());
                }
                store.save_default()?;
                println!("    `{name}` upgraded and restarted.");
            }
            let _ = fs::remove_file(&tmp_archive);
        }
    }

    // 2. Upgrade GitLab Runners
    for name in gl_targets {
        let rec = store.find(&name).unwrap().clone();
        println!("  - Upgrading `{name}` (gitlab)...");
        if let Err(err) = systemd::control_service(&rec.service_name, "stop") {
            eprintln!("    error stopping `{name}`: {err}");
            failed.push(name);
            continue;
        }
        let res = gitlab::download_binary(args.runner_version.as_deref(), &rec.install_dir)
            .and_then(|ver| {
                user::chown_recursive(&rec.install_dir, &rec.linux_user)?;
                Ok(ver)
            });
        match res {
            Ok(ver) => {
                if let Err(err) = systemd::control_service(&rec.service_name, "start") {
                    eprintln!("    error starting `{name}`: {err}");
                    failed.push(name);
                    continue;
                }
                if let Some(entry) = store.find_mut(&name) {
                    entry.runner_version = ver;
                    entry.updated_at = Some(Utc::now());
                }
                store.save_default()?;
                println!("    `{name}` upgraded and restarted.");
            }
            Err(err) => {
                eprintln!("    error upgrading `{name}`: {err}");
                let _ = systemd::control_service(&rec.service_name, "start");
                failed.push(name);
            }
        }
    }

    // 3. Upgrade Gitea act_runners
    for name in gt_targets {
        let rec = store.find(&name).unwrap().clone();
        println!("  - Upgrading `{name}` (gitea)...");
        if let Err(err) = systemd::control_service(&rec.service_name, "stop") {
            eprintln!("    error stopping `{name}`: {err}");
            failed.push(name);
            continue;
        }
        let res = gitea::download_binary(args.runner_version.as_deref(), &rec.install_dir)
            .and_then(|ver| {
                user::chown_recursive(&rec.install_dir, &rec.linux_user)?;
                Ok(ver)
            });
        match res {
            Ok(ver) => {
                if let Err(err) = systemd::control_service(&rec.service_name, "start") {
                    eprintln!("    error starting `{name}`: {err}");
                    failed.push(name);
                    continue;
                }
                if let Some(entry) = store.find_mut(&name) {
                    entry.runner_version = ver;
                    entry.updated_at = Some(Utc::now());
                }
                store.save_default()?;
                println!("    `{name}` upgraded and restarted.");
            }
            Err(err) => {
                eprintln!("    error upgrading `{name}`: {err}");
                let _ = systemd::control_service(&rec.service_name, "start");
                failed.push(name);
            }
        }
    }

    if !failed.is_empty() {
        bail!("failed to upgrade runner(s): {}", failed.join(", "));
    }
    Ok(())
}
