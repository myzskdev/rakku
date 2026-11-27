use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Result};

use crate::cli::CleanArgs;
use crate::state::StateStore;
use crate::system::metrics::{self, JobState};
use crate::system::{self, user};

pub fn run(args: CleanArgs) -> Result<()> {
    system::require_linux_root()?;

    if !args.all && args.name.is_none() {
        bail!("specify a runner name or pass --all to clean every managed runner");
    }

    let store = StateStore::load_default()?;
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
        println!("No managed runners to clean.");
        return Ok(());
    }

    let mut total_reclaimed: u64 = 0;

    for name in target_names {
        let rec = store.find(&name).unwrap();
        if !args.force {
            let live = metrics::collect_runner_status(rec);
            if let JobState::Running(job) = live.job_state {
                println!(
                    "  - `{name}` is currently busy running `{job}` (skipping; pass --force to clean anyway)"
                );
                continue;
            }
        }

        let mut reclaimed: u64 = 0;
        let diag_dir = rec.install_dir.join("_diag");
        reclaimed += clear_dir_contents(&diag_dir);

        if args.work {
            // GitHub Actions uses `_work/`, GitLab Runner uses `builds/`
            for work_sub in ["_work", "builds", "cache"] {
                let work_dir = rec.install_dir.join(work_sub);
                reclaimed += clear_dir_contents(&work_dir);
            }
            let _ = user::chown_recursive(&rec.install_dir, &rec.linux_user);
        }

        total_reclaimed += reclaimed;
        println!(
            "  - `{name}`: reclaimed {}",
            metrics::format_bytes(Some(reclaimed))
        );
    }

    println!(
        "\nTotal disk space reclaimed: {}",
        metrics::format_bytes(Some(total_reclaimed))
    );
    if !args.work {
        println!("Tip: pass `--work` to also clear idle build workspaces (`_work/`, `builds/`).");
    }
    Ok(())
}

/// Remove all entries inside `dir` while keeping `dir` itself intact. Returns bytes reclaimed.
pub fn clear_dir_contents(dir: &Path) -> u64 {
    if !dir.exists() {
        return 0;
    }
    let before = metrics::dir_size_bytes(dir);
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let _ = fs::remove_dir_all(&path);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    let after = metrics::dir_size_bytes(dir);
    before.saturating_sub(after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn clears_directory_contents_and_reports_bytes() {
        let dir = tempdir().unwrap();
        let diag = dir.path().join("_diag");
        fs::create_dir_all(&diag).unwrap();
        fs::write(diag.join("runner.log"), vec![b'x'; 2048]).unwrap();

        let reclaimed = clear_dir_contents(&diag);
        assert_eq!(reclaimed, 2048);
        assert!(diag.exists());
        assert_eq!(fs::read_dir(&diag).unwrap().count(), 0);
    }
}
