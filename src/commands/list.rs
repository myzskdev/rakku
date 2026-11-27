use anyhow::Result;

use crate::cli::ListArgs;
use crate::config::APP_NAME;
use crate::state::StateStore;
use crate::system::metrics::{self, RunnerLiveStatus};

pub fn run(args: ListArgs) -> Result<()> {
    let store = StateStore::load_default()?;
    let statuses: Vec<RunnerLiveStatus> = store
        .runners
        .iter()
        .map(metrics::collect_runner_status)
        .collect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&statuses)?);
        return Ok(());
    }

    if statuses.is_empty() {
        println!(
            "No runners managed by {APP_NAME} yet. Run `sudo {APP_NAME} install` to set one up."
        );
        return Ok(());
    }

    println!(
        "{:<18} {:<10} {:<22} {:<12} {:<8} {:<7} {:<10} {:<10} TARGET",
        "NAME", "SERVICE", "JOB", "USER", "VERSION", "CPU%", "MEM", "UPTIME"
    );
    println!("{}", "-".repeat(120));

    for st in statuses {
        let cpu = st
            .cpu_percent
            .map(|c| format!("{c:.1}%"))
            .unwrap_or_else(|| "-".into());
        let mem = metrics::format_bytes(st.memory_bytes);
        let uptime = metrics::format_uptime(st.uptime_secs);
        let job = truncate(&st.job_state.to_string(), 22);
        println!(
            "{:<18} {:<10} {:<22} {:<12} {:<8} {:<7} {:<10} {:<10} {}",
            truncate(&st.name, 18),
            truncate(&st.active_state, 10),
            job,
            truncate(&st.linux_user, 12),
            truncate(&st.runner_version, 8),
            cpu,
            mem,
            uptime,
            st.url
        );
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max > 1 {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    } else {
        s.chars().take(max).collect()
    }
}
