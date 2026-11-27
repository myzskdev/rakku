use std::fs;

use anyhow::{anyhow, bail, Result};
use dialoguer::{Confirm, Password};

use crate::cli::RemoveArgs;
use crate::github::{GitHubClient, GitHubTarget};
use crate::providers::gitlab;
use crate::state::{ProviderKind, RunnerRecord, StateStore};
use crate::system::{self, systemd};

pub fn run(args: RemoveArgs) -> Result<()> {
    system::require_linux_root()?;

    let mut store = StateStore::load_default()?;
    let rec = store
        .find(&args.name)
        .cloned()
        .ok_or_else(|| anyhow!("runner `{}` is not managed locally", args.name))?;

    if !args.yes {
        let confirmed = Confirm::new()
            .with_prompt(format!(
                "Stop, deregister, and delete {} runner `{}` ({})?",
                rec.provider, rec.name, rec.url
            ))
            .default(false)
            .interact()?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    println!(
        "[1/4] Stopping and removing systemd service `{}`...",
        rec.service_name
    );
    systemd::remove_unit(&rec.name, &rec.service_name)?;

    println!(
        "[2/4] Deregistering {} runner `{}`...",
        rec.provider, rec.name
    );
    let deregister_res =
        deregister_runner(&rec, args.pat.as_deref(), args.token.as_deref(), args.force);
    if let Err(err) = deregister_res {
        if args.force {
            eprintln!(
                "warning: remote deregistration skipped/failed ({err}); continuing due to --force"
            );
        } else {
            return Err(err);
        }
    }

    println!(
        "[3/4] Removing runner directory {}...",
        rec.install_dir.display()
    );
    if rec.install_dir.exists() {
        fs::remove_dir_all(&rec.install_dir)?;
    }

    println!("[4/4] Updating local state...");
    store.remove(&rec.name);
    store.save_default()?;

    println!("Runner `{}` removed cleanly.", rec.name);
    Ok(())
}

fn deregister_runner(
    rec: &RunnerRecord,
    pat: Option<&str>,
    manual_token: Option<&str>,
    force: bool,
) -> Result<()> {
    match rec.provider {
        ProviderKind::Github => deregister_github(rec, pat, manual_token, force),
        ProviderKind::Gitlab => gitlab::deregister(rec),
        ProviderKind::Gitea => {
            println!(
                "      Note: Gitea act_runner has no CLI unregister command; remove `{}` from Gitea Settings -> Actions -> Runners if still listed.",
                rec.name
            );
            Ok(())
        }
    }
}

fn deregister_github(
    rec: &RunnerRecord,
    pat: Option<&str>,
    manual_token: Option<&str>,
    force: bool,
) -> Result<()> {
    let remove_token = if let Some(tok) = manual_token {
        tok.to_string()
    } else if let Some(p) = pat {
        let target = GitHubTarget::parse(&rec.url)?;
        GitHubClient::new().create_remove_token(&target, p)?
    } else if force {
        bail!("no --pat or --token supplied");
    } else {
        let tok = Password::new()
            .with_prompt("GitHub PAT or manual removal token (or re-run with --force to skip)")
            .interact()?;
        if tok.trim().is_empty() {
            bail!("empty token provided");
        }
        if tok.starts_with("ghp_") || tok.starts_with("github_pat_") {
            let target = GitHubTarget::parse(&rec.url)?;
            GitHubClient::new().create_remove_token(&target, &tok)?
        } else {
            tok
        }
    };

    let config_script = rec.install_dir.join("config.sh");
    if !config_script.exists() {
        bail!("{} not found", config_script.display());
    }
    let script_str = config_script
        .to_str()
        .ok_or_else(|| anyhow!("runner path contains invalid UTF-8"))?;

    let args = vec!["remove".to_string(), "--token".to_string(), remove_token];
    system::run_as_user(&rec.linux_user, &rec.install_dir, script_str, &args)?;
    Ok(())
}
