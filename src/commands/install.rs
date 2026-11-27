use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use chrono::Utc;
use dialoguer::{Confirm, Input, Password, Select};

use crate::cli::InstallArgs;
use crate::config::{self, APP_NAME};
use crate::github::{current_runner_arch, GitHubClient, GitHubTarget};
use crate::providers::{gitea, gitlab};
use crate::state::{ContainerRuntime, ProviderKind, RunnerRecord, RunnerScope, StateStore};
use crate::system::{self, preflight, systemd, user};

/// Automatically cleans up partial runner files and systemd units if installation fails mid-way.
struct RollbackGuard {
    created_dir: Option<PathBuf>,
    created_unit: Option<(String, String)>,
    armed: bool,
}

impl RollbackGuard {
    fn new() -> Self {
        Self {
            created_dir: None,
            created_unit: None,
            armed: true,
        }
    }

    fn track_dir(&mut self, dir: PathBuf) {
        self.created_dir = Some(dir);
    }

    fn track_unit(&mut self, runner_name: String, service_name: String) {
        self.created_unit = Some((runner_name, service_name));
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn rollback(&mut self) {
        if !self.armed {
            return;
        }
        self.armed = false;
        eprintln!("\nInstallation failed — rolling back partial changes...");

        if let Some((runner_name, service_name)) = self.created_unit.take() {
            let _ = systemd::remove_unit(&runner_name, &service_name);
            eprintln!("  - Removed systemd unit `{service_name}`");
        }
        if let Some(dir) = self.created_dir.take() {
            if dir.exists() {
                let _ = fs::remove_dir_all(&dir);
                eprintln!("  - Removed partial runner directory {}", dir.display());
            }
        }
    }
}

impl Drop for RollbackGuard {
    fn drop(&mut self) {
        self.rollback();
    }
}

pub fn run(args: InstallArgs) -> Result<()> {
    system::require_linux_root()?;

    // Fail fast on missing systemd or core Linux utilities before prompting or modifying anything.
    let preflight_report = preflight::run_diagnostics(false, false);
    if preflight_report.has_failures() {
        preflight_report.print();
        bail!("host preflight checks failed; fix the [FAIL] items above and retry");
    }

    let mut store = StateStore::load_default()?;
    let interactive = !args.non_interactive && args.url.is_none();

    if interactive {
        println!("{APP_NAME} setup wizard — press Enter to accept defaults.\n");
    }

    let provider: ProviderKind = if let Some(p) = args.provider {
        p.into()
    } else if interactive {
        let choices = &[
            "GitHub Actions (github.com)",
            "GitLab Runner  (gitlab.com or self-hosted GitLab)",
            "Gitea Actions  (self-hosted Gitea / act_runner)",
        ];
        match Select::new()
            .with_prompt("CI Runner provider")
            .items(choices)
            .default(0)
            .interact()?
        {
            1 => ProviderKind::Gitlab,
            2 => ProviderKind::Gitea,
            _ => ProviderKind::Github,
        }
    } else {
        ProviderKind::Github
    };

    let (url, scope, gh_target) = resolve_target(provider, args.url, args.non_interactive)?;

    let default_name = suggest_runner_name(&store);
    let name = match args.name {
        Some(n) => n,
        None if args.non_interactive => default_name,
        None => Input::<String>::new()
            .with_prompt("Runner name")
            .default(default_name)
            .interact_text()?,
    };

    if store.find(&name).is_some() && !args.replace {
        bail!("a runner named `{name}` is already managed locally; pass --replace to overwrite it");
    }

    let (pat, token) = resolve_auth(provider, args.pat, args.token, args.non_interactive)?;

    let labels_raw = match args.labels {
        Some(l) => l,
        None if interactive => Input::<String>::new()
            .with_prompt("Custom labels/tags (comma-separated, optional)")
            .allow_empty(true)
            .interact_text()?,
        None => String::new(),
    };
    let labels: Vec<String> = labels_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let linux_user = if interactive {
        Input::<String>::new()
            .with_prompt("Dedicated Linux user to run the runner")
            .default(args.user)
            .interact_text()?
    } else {
        args.user
    };

    let runtime = if let Some(r) = args.runtime {
        r.into()
    } else if args.docker {
        ContainerRuntime::Docker
    } else if interactive {
        let choices = &[
            "None   (bare-metal / host deployments only)",
            "Docker (adds user to `docker` group; installs Docker if missing)",
            "Podman (rootless containers, native on RHEL/Rocky/Fedora; enables user linger)",
        ];
        let default_idx = if system::command_exists("podman") && !system::command_exists("docker") {
            2
        } else if system::command_exists("docker") {
            1
        } else {
            0
        };
        match Select::new()
            .with_prompt("Container runtime for CI jobs")
            .items(choices)
            .default(default_idx)
            .interact()?
        {
            1 => ContainerRuntime::Docker,
            2 => ContainerRuntime::Podman,
            _ => ContainerRuntime::None,
        }
    } else {
        ContainerRuntime::None
    };

    // Ensure optional/recommended server prerequisites (git, docker/podman) are present or installed.
    ensure_prerequisites(
        preflight_report.pm,
        runtime,
        args.install_missing,
        !args.non_interactive,
    )?;

    for kv in &args.env_vars {
        if !kv.contains('=') || kv.starts_with('=') {
            bail!("invalid --env `{kv}`: expected `KEY=VALUE` format");
        }
    }

    let sudo_access = args.sudo;
    let docker_access = runtime == ContainerRuntime::Docker;
    let install_dir = config::runner_install_dir(Some(&args.install_root), &name);
    let service_name = config::systemd_service_name(&name);
    let dir_existed_before = install_dir.exists();

    println!("[1/6] Ensuring Linux user `{linux_user}` and permissions (runtime: {runtime})...");
    user::ensure_linux_user(&linux_user, runtime, sudo_access)?;

    let mut guard = RollbackGuard::new();
    if !dir_existed_before {
        guard.track_dir(install_dir.clone());
    }

    let installed_version = match provider {
        ProviderKind::Github => {
            let target = gh_target.expect("GitHub target must be parsed");
            let gh = GitHubClient::new();
            let reg_token = match (pat.as_deref(), token) {
                (_, Some(tok)) => tok,
                (Some(p), None) => {
                    println!("[2/6] Requesting GitHub runner registration token via API...");
                    gh.create_registration_token(&target, p)?
                }
                (None, None) => unreachable!("auth resolution guarantees token or PAT"),
            };

            let arch = current_runner_arch()?;
            println!("[3/6] Downloading actions/runner for linux-{arch}...");
            let release =
                gh.fetch_runner_release(args.runner_version.as_deref(), arch, pat.as_deref())?;
            println!("      {} (v{})", release.asset_name, release.version);
            gh.download_and_extract(&release, &install_dir)?;

            if !args.skip_deps {
                let deps_script = install_dir.join("bin").join("installdependencies.sh");
                if deps_script.exists() {
                    println!("      Installing OS dependencies via bin/installdependencies.sh...");
                    let script_str = deps_script
                        .to_str()
                        .ok_or_else(|| anyhow!("dependency script path contains invalid UTF-8"))?;
                    system::run_checked("bash", &[script_str])?;
                }
            }

            user::chown_recursive(&install_dir, &linux_user)?;

            println!("[4/6] Registering GitHub Actions runner `{name}`...");
            let mut cfg_args = vec![
                "--unattended".to_string(),
                "--url".to_string(),
                url.clone(),
                "--token".to_string(),
                reg_token,
                "--name".to_string(),
                name.clone(),
            ];
            if !labels.is_empty() {
                cfg_args.push("--labels".to_string());
                cfg_args.push(labels.join(","));
            }
            if let Some(group) = &args.runner_group {
                cfg_args.push("--runnergroup".to_string());
                cfg_args.push(group.clone());
            }
            if args.replace {
                cfg_args.push("--replace".to_string());
            }
            if args.ephemeral {
                cfg_args.push("--ephemeral".to_string());
            }

            let config_script = install_dir.join("config.sh");
            let config_script_str = config_script
                .to_str()
                .ok_or_else(|| anyhow!("install path contains invalid UTF-8"))?;
            system::run_as_user(&linux_user, &install_dir, config_script_str, &cfg_args)?;
            release.version
        }
        ProviderKind::Gitlab => {
            let gl_token = token.ok_or_else(|| anyhow!("GitLab runner token is required"))?;
            println!("[2/6] Skipping API token exchange (using GitLab runner token directly)...");
            println!("[3/6] Downloading GitLab Runner binary...");
            let version = gitlab::download_binary(args.runner_version.as_deref(), &install_dir)?;
            user::chown_recursive(&install_dir, &linux_user)?;

            println!("[4/6] Registering GitLab Runner `{name}` with {url}...");
            gitlab::register(
                &linux_user,
                &install_dir,
                &url,
                &gl_token,
                &name,
                &labels,
                runtime,
                args.executor.as_deref(),
                &args.docker_image,
            )?;
            version
        }
        ProviderKind::Gitea => {
            let gt_token = token.ok_or_else(|| anyhow!("Gitea runner token is required"))?;
            println!("[2/6] Skipping API token exchange (using Gitea runner token directly)...");
            println!("[3/6] Downloading Gitea act_runner binary...");
            let version = gitea::download_binary(args.runner_version.as_deref(), &install_dir)?;
            user::chown_recursive(&install_dir, &linux_user)?;

            println!("[4/6] Registering Gitea act_runner `{name}` with {url}...");
            gitea::register(&linux_user, &install_dir, &url, &gt_token, &name, &labels)?;
            version
        }
    };

    let record = RunnerRecord {
        name: name.clone(),
        provider,
        url: url.clone(),
        scope,
        labels,
        runner_group: args.runner_group,
        linux_user: linux_user.clone(),
        container_runtime: runtime,
        docker_access,
        sudo_access,
        install_dir: install_dir.clone(),
        service_name: service_name.clone(),
        runner_version: installed_version,
        ephemeral: args.ephemeral,
        memory_limit: args.memory_limit,
        cpu_quota: args.cpu_quota,
        env_vars: args.env_vars,
        env_file: args.env_file.map(PathBuf::from),
        installed_at: Utc::now(),
        updated_at: None,
    };

    println!("[5/6] Installing and starting systemd service `{service_name}`...");
    guard.track_unit(name.clone(), service_name.clone());
    systemd::install_and_enable_unit(&record)?;

    println!("[6/6] Saving runner metadata to local state...");
    store.upsert(record);
    store.save_default()?;

    guard.disarm();

    println!("\nRunner `{name}` ({provider}) is installed and running!");
    println!("  Target:    {url}");
    println!("  User:      {linux_user} (runtime: {runtime})");
    println!("  Directory: {}", install_dir.display());
    println!("  Service:   {service_name}");
    println!("\nUseful commands:");
    println!("  {APP_NAME} monitor          # Live dashboard for all local runners");
    println!("  {APP_NAME} status {name}");
    println!("  {APP_NAME} logs {name} -f");
    Ok(())
}

fn resolve_target(
    provider: ProviderKind,
    raw_url: Option<String>,
    non_interactive: bool,
) -> Result<(String, RunnerScope, Option<GitHubTarget>)> {
    match provider {
        ProviderKind::Github => {
            let input = match raw_url {
                Some(u) => u,
                None if non_interactive => {
                    bail!("--url is required when running with --non-interactive")
                }
                None => Input::<String>::new()
                    .with_prompt(
                        "GitHub repo or org (e.g. owner/repo or https://github.com/my-org)",
                    )
                    .interact_text()?,
            };
            let target = GitHubTarget::parse(&input)?;
            let canonical = target.canonical_url();
            let scope = target.scope();
            Ok((canonical, scope, Some(target)))
        }
        ProviderKind::Gitlab => {
            let input = match raw_url {
                Some(u) => u,
                None if non_interactive => "https://gitlab.com".to_string(),
                None => Input::<String>::new()
                    .with_prompt("GitLab instance URL")
                    .default("https://gitlab.com".to_string())
                    .interact_text()?,
            };
            let trimmed = input.trim().trim_end_matches('/').to_string();
            if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                bail!("GitLab URL must start with https:// or http:// (got: {trimmed})");
            }
            Ok((trimmed, RunnerScope::Instance, None))
        }
        ProviderKind::Gitea => {
            let input = match raw_url {
                Some(u) => u,
                None if non_interactive => {
                    bail!("--url (Gitea instance URL) is required when running with --non-interactive")
                }
                None => Input::<String>::new()
                    .with_prompt("Gitea instance URL (e.g. https://gitea.example.com)")
                    .interact_text()?,
            };
            let trimmed = input.trim().trim_end_matches('/').to_string();
            if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                bail!("Gitea URL must start with https:// or http:// (got: {trimmed})");
            }
            Ok((trimmed, RunnerScope::Instance, None))
        }
    }
}

fn ensure_prerequisites(
    pm: preflight::PackageManager,
    runtime: ContainerRuntime,
    auto_install: bool,
    allow_prompt: bool,
) -> Result<()> {
    if !system::command_exists("git") {
        let should_install = if auto_install {
            true
        } else if allow_prompt {
            Confirm::new()
                .with_prompt("`git` is not installed (needed by CI checkouts). Install it now?")
                .default(true)
                .interact()?
        } else {
            eprintln!(
                "warning: `git` is not installed; install it with `{}` or pass --install-missing",
                pm.install_hint("git")
            );
            false
        };

        if should_install {
            println!("Installing `git`...");
            preflight::install_packages(pm, &["git"])?;
        }
    }

    match runtime {
        ContainerRuntime::None => {}
        ContainerRuntime::Docker => {
            if !system::command_exists("docker") {
                let should_install = if auto_install {
                    true
                } else if allow_prompt {
                    Confirm::new()
                        .with_prompt(
                            "Docker runtime selected, but Docker is not installed. Install Docker now?",
                        )
                        .default(true)
                        .interact()?
                } else {
                    bail!(
                        "Docker runtime was requested, but Docker is not installed. Re-run with `--install-missing` or run `{}` first.",
                        pm.install_hint(pm.docker_package_name())
                    );
                };

                if should_install {
                    println!("Installing Docker...");
                    preflight::install_docker(pm)?;
                }
            }
        }
        ContainerRuntime::Podman => {
            if !system::command_exists("podman") {
                let should_install = if auto_install {
                    true
                } else if allow_prompt {
                    Confirm::new()
                        .with_prompt(
                            "Podman runtime selected, but Podman is not installed. Install Podman now?",
                        )
                        .default(true)
                        .interact()?
                } else {
                    bail!(
                        "Podman runtime was requested, but Podman is not installed. Re-run with `--install-missing` or run `{}` first.",
                        pm.install_hint("podman")
                    );
                };

                if should_install {
                    println!("Installing Podman...");
                    preflight::install_podman(pm)?;
                }
            }
        }
    }

    Ok(())
}

fn resolve_auth(
    provider: ProviderKind,
    pat: Option<String>,
    token: Option<String>,
    non_interactive: bool,
) -> Result<(Option<String>, Option<String>)> {
    match provider {
        ProviderKind::Github => {
            if pat.is_some() || token.is_some() {
                return Ok((pat, token));
            }
            if non_interactive {
                bail!("either --pat (or GITHUB_PAT) or --token (or RUNNER_TOKEN) is required in --non-interactive mode");
            }

            let choices = &[
                "GitHub PAT (recommended: auto-mints registration & removal tokens)",
                "Manual 1-hour registration token (from GitHub Settings -> Actions -> Runners)",
            ];
            let selection = Select::new()
                .with_prompt("Authentication method")
                .items(choices)
                .default(0)
                .interact()?;

            if selection == 0 {
                let p = Password::new()
                    .with_prompt("GitHub Personal Access Token (PAT)")
                    .interact()?;
                if p.trim().is_empty() {
                    bail!("PAT cannot be empty");
                }
                Ok((Some(p), None))
            } else {
                let t = Password::new()
                    .with_prompt("GitHub Runner Registration Token")
                    .interact()?;
                if t.trim().is_empty() {
                    bail!("registration token cannot be empty");
                }
                Ok((None, Some(t)))
            }
        }
        ProviderKind::Gitlab => {
            if let Some(t) = token {
                return Ok((None, Some(t)));
            }
            if non_interactive {
                bail!("--token (or RUNNER_TOKEN) is required for GitLab Runner in --non-interactive mode");
            }
            let t = Password::new()
                .with_prompt("GitLab Runner Token (glrt-... or registration token from CI/CD Settings -> Runners)")
                .interact()?;
            if t.trim().is_empty() {
                bail!("GitLab runner token cannot be empty");
            }
            Ok((None, Some(t)))
        }
        ProviderKind::Gitea => {
            if let Some(t) = token {
                return Ok((None, Some(t)));
            }
            if non_interactive {
                bail!("--token (or RUNNER_TOKEN) is required for Gitea act_runner in --non-interactive mode");
            }
            let t = Password::new()
                .with_prompt(
                    "Gitea Runner Registration Token (from Settings -> Actions -> Runners)",
                )
                .interact()?;
            if t.trim().is_empty() {
                bail!("Gitea runner token cannot be empty");
            }
            Ok((None, Some(t)))
        }
    }
}

fn suggest_runner_name(store: &StateStore) -> String {
    let base = system::hostname();
    if store.find(&base).is_none() {
        return base;
    }
    for i in 2..=999 {
        let candidate = format!("{base}-{i}");
        if store.find(&candidate).is_none() {
            return candidate;
        }
    }
    format!("{base}-runner")
}
