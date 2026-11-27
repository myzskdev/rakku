mod cli;
mod commands;
mod config;
mod github;
mod providers;
mod state;
mod system;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Install(args) => commands::install::run(args),
        Commands::List(args) => commands::list::run(args),
        Commands::Monitor(args) => commands::monitor::run(args),
        Commands::Status(args) => commands::status::run(args),
        Commands::Logs(args) => commands::logs::run(args),
        Commands::Start(args) => commands::control::run(args, "start"),
        Commands::Stop(args) => commands::control::run(args, "stop"),
        Commands::Restart(args) => commands::control::run(args, "restart"),
        Commands::Remove(args) => commands::remove::run(args),
        Commands::Update(args) => commands::self_update::run(args),
        Commands::Upgrade(args) => commands::update::run(args),
        Commands::Doctor => commands::doctor::run(),
    }
}
