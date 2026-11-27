use anyhow::{anyhow, Result};

use crate::cli::RunnerTargetArgs;
use crate::state::StateStore;
use crate::system::{metrics, systemd};

pub fn run(args: RunnerTargetArgs) -> Result<()> {
    let store = StateStore::load_default()?;
    let rec = store
        .find(&args.name)
        .ok_or_else(|| anyhow!("runner `{}` is not managed locally", args.name))?;

    let live = metrics::collect_runner_status(rec);
    let logs = systemd::read_recent_logs(&rec.service_name, 15);

    println!("Runner:        {} ({})", rec.name, rec.provider);
    println!("Target URL:    {} ({})", rec.url, rec.scope);
    println!(
        "Service:       {} ({} / {})",
        rec.service_name, live.active_state, live.sub_state
    );
    println!("Job State:     {}", live.job_state);
    println!(
        "PID / Uptime:  {} / {}",
        live.pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".into()),
        metrics::format_uptime(live.uptime_secs)
    );
    println!(
        "CPU / Memory:  {} / {}",
        live.cpu_percent
            .map(|c| format!("{c:.1}%"))
            .unwrap_or_else(|| "-".into()),
        metrics::format_bytes(live.memory_bytes)
    );
    println!("Disk Usage:    {}", metrics::format_bytes(live.disk_bytes));
    if rec.memory_limit.is_some() || rec.cpu_quota.is_some() {
        println!(
            "Limits:        MemoryMax={}, CPUQuota={}",
            rec.memory_limit.as_deref().unwrap_or("unlimited"),
            rec.cpu_quota.as_deref().unwrap_or("unlimited")
        );
    }
    if !rec.env_vars.is_empty() || rec.env_file.is_some() {
        println!(
            "Env Config:    {} var(s), env_file={}",
            rec.env_vars.len(),
            rec.env_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "-".into())
        );
    }
    println!("Restarts:      {}", live.restarts);
    println!(
        "Linux User:    {} (runtime={}, sudo={})",
        rec.linux_user, rec.container_runtime, rec.sudo_access
    );
    println!(
        "Labels:        {}",
        if rec.labels.is_empty() {
            "-".into()
        } else {
            rec.labels.join(", ")
        }
    );
    println!("Version:       {}", rec.runner_version);
    println!("Directory:     {}", rec.install_dir.display());
    println!("Installed At:  {}", rec.installed_at.to_rfc3339());
    if let Some(updated) = rec.updated_at {
        println!("Updated At:    {}", updated.to_rfc3339());
    }

    println!("\nRecent Logs:");
    if logs.is_empty() {
        println!("  (no journal entries available)");
    } else {
        for line in logs {
            println!("  {line}");
        }
    }

    Ok(())
}
