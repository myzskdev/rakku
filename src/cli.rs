use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::config::{APP_NAME, DEFAULT_INSTALL_ROOT, DEFAULT_RUNNER_USER};
use crate::state::{ContainerRuntime, ProviderKind};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProviderArg {
    Github,
    Gitlab,
    Gitea,
}

impl From<ProviderArg> for ProviderKind {
    fn from(value: ProviderArg) -> Self {
        match value {
            ProviderArg::Github => ProviderKind::Github,
            ProviderArg::Gitlab => ProviderKind::Gitlab,
            ProviderArg::Gitea => ProviderKind::Gitea,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RuntimeArg {
    None,
    Docker,
    Podman,
}

impl From<RuntimeArg> for ContainerRuntime {
    fn from(value: RuntimeArg) -> Self {
        match value {
            RuntimeArg::None => ContainerRuntime::None,
            RuntimeArg::Docker => ContainerRuntime::Docker,
            RuntimeArg::Podman => ContainerRuntime::Podman,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = APP_NAME,
    version,
    about = "Effortless Linux setup, lifecycle management, and local monitoring for self-hosted CI runners (GitHub Actions, GitLab Runner, Gitea Actions)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Install, register, and start a new self-hosted CI runner
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

    /// Upgrade one or all managed runners to the latest upstream runner release
    Upgrade(UpdateArgs),

    /// Check host readiness (systemd, Linux tools, git, docker, GitHub API)
    Doctor,
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// CI runner provider (github, gitlab, or gitea)
    #[arg(long, value_enum)]
    pub provider: Option<ProviderArg>,

    /// Target URL (GitHub: owner/repo or URL; GitLab: https://gitlab.com; Gitea: https://gitea.example.com)
    #[arg(long)]
    pub url: Option<String>,

    /// GitHub Personal Access Token (GitHub only: auto-mints registration & removal tokens)
    #[arg(long, env = "GITHUB_PAT")]
    pub pat: Option<String>,

    /// Runner registration/authentication token (GitHub 1h token, GitLab glrt-*/registration token, or Gitea runner token)
    #[arg(long, env = "RUNNER_TOKEN")]
    pub token: Option<String>,

    /// GitLab Runner executor (shell or docker; defaults to docker when --runtime docker, else shell)
    #[arg(long)]
    pub executor: Option<String>,

    /// Default Docker image for GitLab docker executor (default: alpine:latest)
    #[arg(long, default_value = "alpine:latest")]
    pub docker_image: String,

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

    /// Container runtime for CI jobs (none, docker, or podman)
    #[arg(long, value_enum)]
    pub runtime: Option<RuntimeArg>,

    /// Shorthand for `--runtime docker` (add the runner Linux user to the `docker` group)
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
