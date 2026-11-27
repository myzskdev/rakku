use std::fs;

use anyhow::{anyhow, bail, Result};
use chrono::Utc;

use crate::cli::UpdateArgs;
use crate::config::APP_NAME;
use crate::github::{current_runner_arch, GitHubClient};
use crate::state::StateStore;
use crate::system::metrics::{self, JobState};
use crate::system::{self, systemd, user};

pub fn run(args: UpdateArgs) -> Result<()> {
    system::require_linux_root()?;

    if !args.all && args.name.is_none() {
        bail!("specify a runner name or pass --all to update every managed runner");
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
        println!("No managed runners to update.");
        return Ok(());
    }

    let arch = current_runner_arch()?;
    let gh = GitHubClient::new();
    let release = gh.fetch_runner_release(args.runner_version.as_deref(), arch, None)?;
    println!("Target actions/runner version: v{}", release.version);

    // Filter out runners already up-to-date or currently busy (unless --force).
    let mut to_update = Vec::new();
    for name in target_names {
        let rec = store.find(&name).unwrap();
        if rec.runner_version == release.version {
            println!("  - `{name}` is already on v{} (skipping)", release.version);
            continue;
        }
        if !args.force {
            let live = metrics::collect_runner_status(rec);
            if let JobState::Running(job) = live.job_state {
                println!(
                    "  - `{name}` is currently busy running job `{job}` (skipping; pass --force to interrupt)"
                );
                continue;
            }
        }
        to_update.push(name);
    }

    if to_update.is_empty() {
        println!("Nothing to update.");
        return Ok(());
    }

    // Download and verify the tarball ONCE before stopping any runner service.
    let tmp_archive = std::env::temp_dir().join(format!("{APP_NAME}-{}", release.asset_name));
    println!(
        "Downloading and verifying {} once before stopping runners...",
        release.asset_name
    );
    gh.download_tarball(&release, &tmp_archive)?;

    let mut failed = Vec::new();
    for name in to_update {
        let rec = store.find(&name).unwrap().clone();
        println!(
            "  - Updating `{name}` from v{} -> v{}...",
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
            eprintln!("    error updating `{name}`: {err}");
            eprintln!("    attempting to restart `{name}` so it does not stay offline...");
            let _ = systemd::control_service(&rec.service_name, "start");
            failed.push(name);
            continue;
        }

        if let Err(err) = systemd::control_service(&rec.service_name, "start") {
            eprintln!("    error starting `{name}` after update: {err}");
            failed.push(name);
            continue;
        }

        if let Some(entry) = store.find_mut(&name) {
            entry.runner_version = release.version.clone();
            entry.updated_at = Some(Utc::now());
        }
        store.save_default()?;
        println!("    `{name}` updated and restarted.");
    }

    let _ = fs::remove_file(&tmp_archive);

    if !failed.is_empty() {
        bail!("failed to update runner(s): {}", failed.join(", "));
    }
    Ok(())
}
