use anyhow::{anyhow, Result};

use crate::cli::LogsArgs;
use crate::state::StateStore;
use crate::system::systemd;

pub fn run(args: LogsArgs) -> Result<()> {
    let store = StateStore::load_default()?;
    let rec = store
        .find(&args.name)
        .ok_or_else(|| anyhow!("runner `{}` is not managed locally", args.name))?;

    systemd::stream_logs(&rec.service_name, args.lines, args.follow)
}
