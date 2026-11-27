use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use chrono::Utc;
use dialoguer::{Confirm, Input, Password, Select};

use crate::cli::InstallArgs;
use crate::config::{self, APP_NAME};
use crate::github::{current_runner_arch, GitHubClient, GitHubTarget};
use crate::state::{ContainerRuntime, RunnerRecord, StateStore};
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

    let raw_url = match args.url {
        Some(u) => u,
        None if args.non_interactive => {
            bail!("--url is required when running with --non-interactive")
        }
        None => Input::<String>::new()
            .with_prompt("GitHub repo or org (e.g. owner/repo or https://github.com/my-org)")
            .interact_text()?,
    };
    let target = GitHubTarget::parse(&raw_url)?;
    let url = target.canonical_url();

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

    let (pat, manual_token) = resolve_auth(args.pat, args.token, args.non_interactive)?;

    let labels_raw = match args.labels {
        Some(l) => l,
        None if interactive => Input::<String>::new()
            .with_prompt("Custom labels (comma-separated, optional)")
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

    let sudo_access = args.sudo;
    let docker_access = runtime == ContainerRuntime::Docker;
    let install_dir = config::runner_install_dir(Some(&args.install_root), &name);
    let service_name = config::systemd_service_name(&name);
    let dir_existed_before = install_dir.exists();

    let gh = GitHubClient::new();
    let reg_token = match (pat.as_deref(), manual_token) {
        (_, Some(tok)) => tok,
        (Some(p), None) => {
            println!("[1/7] Requesting GitHub runner registration token via API...");
            gh.create_registration_token(&target, p)?
        }
        (None, None) => unreachable!("auth resolution guarantees either PAT or manual token"),
    };

    println!("[2/7] Ensuring Linux user `{linux_user}` and permissions (runtime: {runtime})...");
    user::ensure_linux_user(&linux_user, runtime, sudo_access)?;

    let mut guard = RollbackGuard::new();
    if !dir_existed_before {
        guard.track_dir(install_dir.clone());
    }

    let arch = current_runner_arch()?;
    println!("[3/7] Resolving actions/runner release for linux-{arch}...");
    let release = gh.fetch_runner_release(args.runner_version.as_deref(), arch, pat.as_deref())?;
    println!(
        "      Downloading {} (v{})...",
        release.asset_name, release.version
    );
    gh.download_and_extract(&release, &install_dir)?;

    if !args.skip_deps {
        let deps_script = install_dir.join("bin").join("installdependencies.sh");
        if deps_script.exists() {
            println!("[4/7] Installing runner OS dependencies via bin/installdependencies.sh...");
            let script_str = deps_script
                .to_str()
                .ok_or_else(|| anyhow!("dependency script path contains invalid UTF-8"))?;
            system::run_checked("bash", &[script_str])?;
        } else {
            println!("[4/7] Skipping OS dependencies (bin/installdependencies.sh not present).");
        }
    } else {
        println!("[4/7] Skipping OS dependencies (--skip-deps).");
    }

    user::chown_recursive(&install_dir, &linux_user)?;

    println!("[5/7] Registering runner `{name}` with GitHub...");
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

    let record = RunnerRecord {
        name: name.clone(),
        url: url.clone(),
        scope: target.scope(),
        labels,
        runner_group: args.runner_group,
        linux_user: linux_user.clone(),
        container_runtime: runtime,
        docker_access,
        sudo_access,
        install_dir: install_dir.clone(),
        service_name: service_name.clone(),
        runner_version: release.version.clone(),
        ephemeral: args.ephemeral,
        installed_at: Utc::now(),
        updated_at: None,
    };

    println!("[6/7] Installing and starting systemd service `{service_name}`...");
    guard.track_unit(name.clone(), service_name.clone());
    systemd::install_and_enable_unit(&record)?;

    println!("[7/7] Saving runner metadata to local state...");
    store.upsert(record);
    store.save_default()?;

    guard.disarm();

    println!("\nRunner `{name}` is installed and running!");
    println!("  Target:    {url}");
    println!("  User:      {linux_user}");
    println!("  Directory: {}", install_dir.display());
    println!("  Service:   {service_name}");
    println!("\nUseful commands:");
    println!("  {APP_NAME} monitor          # Live dashboard for all local runners");
    println!("  {APP_NAME} status {name}");
    println!("  {APP_NAME} logs {name} -f");
    Ok(())
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
                .with_prompt("`git` is not installed (needed by actions/checkout). Install it now?")
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
    pat: Option<String>,
    token: Option<String>,
    non_interactive: bool,
) -> Result<(Option<String>, Option<String>)> {
    if pat.is_some() || token.is_some() {
        return Ok((pat, token));
    }
    if non_interactive {
        bail!("either --pat (or GITHUB_PAT) or --token (or GITHUB_RUNNER_TOKEN) is required in --non-interactive mode");
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
