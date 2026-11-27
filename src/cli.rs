use clap::{Args, Parser, Subcommand};

use crate::config::{APP_NAME, DEFAULT_INSTALL_ROOT, DEFAULT_RUNNER_USER};

#[derive(Debug, Parser)]
#[command(
    name = APP_NAME,
    version,
    about = "Automated Linux setup, lifecycle management, and local monitoring for GitHub Actions self-hosted runners"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Install, register, and start a new GitHub Actions self-hosted runner
    #[command(aliases = ["init", "setup", "add"])]
    Install(InstallArgs),

    /// List all locally managed runners and their live status
    List(ListArgs),

    /// Launch a live terminal dashboard monitoring all locally installed runners
    Monitor(MonitorArgs),

    /// Show detailed status, metrics, and recent logs for a runner
    Status(RunnerTargetArgs),

    /// View or follow systemd journal logs for a runner
    Logs(LogsArgs),

    /// Start a stopped runner service
    Start(RunnerTargetArgs),

    /// Stop a running runner service
    Stop(RunnerTargetArgs),

    /// Restart a runner service
    Restart(RunnerTargetArgs),

    /// Stop, deregister from GitHub, and remove a runner
    Remove(RemoveArgs),

    /// Update rakku itself to the latest GitHub Release
    #[command(alias = "self-update")]
    Update(SelfUpdateArgs),

    /// Upgrade one or all managed runners to the latest actions/runner release
    Upgrade(UpdateArgs),

    /// Check host readiness (systemd, Linux tools, git, docker, GitHub API)
    Doctor,
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// GitHub repository, organization, or enterprise URL
    /// (e.g. https://github.com/owner/repo or https://github.com/my-org)
    #[arg(long)]
    pub url: Option<String>,

    /// GitHub Personal Access Token (recommended: auto-mints registration & removal tokens)
    #[arg(long, env = "GITHUB_PAT")]
    pub pat: Option<String>,

    /// Manual 1-hour runner registration token from GitHub Settings
    #[arg(long, env = "GITHUB_RUNNER_TOKEN")]
    pub token: Option<String>,

    /// Name for this runner (defaults to hostname or interactive prompt)
    #[arg(long)]
    pub name: Option<String>,

    /// Comma-separated custom labels (e.g. linux,vps,docker)
    #[arg(long)]
    pub labels: Option<String>,

    /// Runner group name (for org/enterprise runners)
    #[arg(long)]
    pub runner_group: Option<String>,

    /// Dedicated Linux user to run the runner under
    #[arg(long, default_value = DEFAULT_RUNNER_USER)]
    pub user: String,

    /// Root directory where runners are installed
    #[arg(long, default_value = DEFAULT_INSTALL_ROOT)]
    pub install_root: String,

    /// Add the runner Linux user to the `docker` group
    #[arg(long)]
    pub docker: bool,

    /// Grant passwordless sudo to the runner user (use with caution)
    #[arg(long)]
    pub sudo: bool,

    /// Specific actions/runner version to install (e.g. 2.323.0). Defaults to latest.
    #[arg(long)]
    pub runner_version: Option<String>,

    /// Replace an existing runner with the same name on GitHub
    #[arg(long)]
    pub replace: bool,

    /// Register as an ephemeral runner (exits after one job)
    #[arg(long)]
    pub ephemeral: bool,

    /// Skip running GitHub runner's `./bin/installdependencies.sh` system library check
    #[arg(long)]
    pub skip_deps: bool,

    /// Automatically install missing prerequisites (e.g. git, docker when --docker is set)
    #[arg(long)]
    pub install_missing: bool,

    /// Skip interactive prompts and fail if required arguments are missing
    #[arg(long)]
    pub non_interactive: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Output machine-readable JSON instead of a table
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct MonitorArgs {
    /// Refresh interval in milliseconds
    #[arg(long, default_value_t = 2000)]
    pub interval_ms: u64,
}

#[derive(Debug, Args)]
pub struct RunnerTargetArgs {
    /// Name of the managed runner
    pub name: String,
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Name of the managed runner
    pub name: String,

    /// Follow log output in real time
    #[arg(short, long)]
    pub follow: bool,

    /// Number of recent log lines to show
    #[arg(short = 'n', long, default_value_t = 50)]
    pub lines: usize,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Name of the managed runner to remove
    pub name: String,

    /// GitHub PAT to auto-mint a removal token
    #[arg(long, env = "GITHUB_PAT")]
    pub pat: Option<String>,

    /// Manual removal token from GitHub Settings
    #[arg(long)]
    pub token: Option<String>,

    /// Force local cleanup even if GitHub deregistration fails
    #[arg(long)]
    pub force: bool,

    /// Skip confirmation prompt
    #[arg(short = 'y', long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Name of a specific runner to update
    pub name: Option<String>,

    /// Update all locally installed runners
    #[arg(long)]
    pub all: bool,

    /// Specific actions/runner version to update to (defaults to latest)
    #[arg(long)]
    pub runner_version: Option<String>,

    /// Update even if a runner is currently executing a job
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct SelfUpdateArgs {
    /// Specific release tag to install (e.g. v0.2.0). Defaults to latest.
    #[arg(long)]
    pub version: Option<String>,

    /// GitHub repository (`owner/repo`) to download releases from
    #[arg(long, env = "RAKKU_REPO")]
    pub repo: Option<String>,

    /// Check for a newer version without replacing the binary
    #[arg(long)]
    pub check: bool,
}
