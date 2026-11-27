use anyhow::{anyhow, Result};

use crate::cli::RunnerTargetArgs;
use crate::state::StateStore;
use crate::system::{self, systemd};

pub fn run(args: RunnerTargetArgs, action: &str) -> Result<()> {
    system::require_linux_root()?;

    let store = StateStore::load_default()?;
    let rec = store
        .find(&args.name)
        .ok_or_else(|| anyhow!("runner `{}` is not managed locally", args.name))?;

    systemd::control_service(&rec.service_name, action)?;
    println!(
        "Runner `{}` ({}) -> {action} OK",
        rec.name, rec.service_name
    );
    Ok(())
}
