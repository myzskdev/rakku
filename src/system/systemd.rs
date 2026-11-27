use std::collections::HashMap;
use std::fs;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::config;
use crate::state::RunnerRecord;
use crate::system::run_checked;

#[derive(Debug, Clone, Default)]
pub struct ServiceStatus {
    pub active_state: String,
    pub sub_state: String,
    pub main_pid: Option<u32>,
    pub restarts: u32,
    pub memory_bytes: Option<u64>,
    pub uptime_secs: Option<u64>,
}

/// Generate a systemd unit file for a managed CI runner (GitHub, GitLab, or Gitea).
pub fn render_unit_file(record: &RunnerRecord) -> String {
    let workdir = record.install_dir.to_string_lossy();
    let exec_start = crate::providers::exec_start_command(record);

    format!(
        "[Unit]\n\
Description=CI Runner ({name} / {provider}) [managed by {app}]\n\
After=network-online.target\n\
Wants=network-online.target\n\
\n\
[Service]\n\
Type=simple\n\
User={user}\n\
WorkingDirectory={workdir}\n\
ExecStart={exec_start}\n\
Restart=always\n\
RestartSec=5\n\
KillMode=control-group\n\
KillSignal=SIGTERM\n\
TimeoutStopSec=5min\n\
Environment=HOME=/home/{user}\n\
\n\
[Install]\n\
WantedBy=multi-user.target\n",
        name = record.name,
        provider = record.provider,
        app = config::APP_NAME,
        user = record.linux_user,
    )
}

/// Write the unit file, reload systemd, and enable + start the service.
pub fn install_and_enable_unit(record: &RunnerRecord) -> Result<()> {
    let unit_path = config::systemd_unit_path(&record.name);
    let unit_body = render_unit_file(record);
    fs::write(&unit_path, unit_body)
        .with_context(|| format!("failed to write systemd unit {}", unit_path.display()))?;

    run_checked("systemctl", &["daemon-reload"])?;
    run_checked("systemctl", &["enable", "--now", &record.service_name])?;
    Ok(())
}

/// Stop, disable, and delete a runner's systemd unit.
pub fn remove_unit(runner_name: &str, service_name: &str) -> Result<()> {
    let _ = Command::new("systemctl")
        .args(["stop", service_name])
        .output();
    let _ = Command::new("systemctl")
        .args(["disable", service_name])
        .output();

    let unit_path = config::systemd_unit_path(runner_name);
    if unit_path.exists() {
        fs::remove_file(&unit_path)
            .with_context(|| format!("failed to remove systemd unit {}", unit_path.display()))?;
    }
    let _ = Command::new("systemctl").arg("daemon-reload").output();
    Ok(())
}

/// Execute `systemctl start|stop|restart <service>`.
pub fn control_service(service_name: &str, action: &str) -> Result<()> {
    run_checked("systemctl", &[action, service_name])?;
    Ok(())
}

/// Query systemd for live properties of a runner service.
pub fn inspect_service(service_name: &str) -> ServiceStatus {
    let output = Command::new("systemctl")
        .args([
            "show",
            service_name,
            "--property=ActiveState,SubState,MainPID,NRestarts,MemoryCurrent,ActiveEnterTimestamp",
            "--no-pager",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            parse_systemctl_show(&text)
        }
        _ => ServiceStatus {
            active_state: "unavailable".into(),
            sub_state: "unknown".into(),
            ..Default::default()
        },
    }
}

pub fn parse_systemctl_show(raw: &str) -> ServiceStatus {
    let mut map = HashMap::new();
    for line in raw.lines() {
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim(), v.trim());
        }
    }

    let active_state = map
        .get("ActiveState")
        .copied()
        .unwrap_or("unknown")
        .to_string();
    let sub_state = map
        .get("SubState")
        .copied()
        .unwrap_or("unknown")
        .to_string();
    let main_pid = map
        .get("MainPID")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&pid| pid > 0);
    let restarts = map
        .get("NRestarts")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let memory_bytes = map
        .get("MemoryCurrent")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&m| m != u64::MAX);

    let uptime_secs = if active_state == "active" {
        map.get("ActiveEnterTimestamp")
            .and_then(|ts| parse_systemd_uptime_secs(ts))
    } else {
        None
    };

    ServiceStatus {
        active_state,
        sub_state,
        main_pid,
        restarts,
        memory_bytes,
        uptime_secs,
    }
}

fn parse_systemd_uptime_secs(ts: &str) -> Option<u64> {
    if ts.is_empty() || ts == "n/a" {
        return None;
    }
    // Typical systemd format: "Thu 2026-11-26 14:20:05 UTC"
    let parts: Vec<&str> = ts.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    let datetime_str = format!("{} {}", parts[1], parts[2]);
    let naive = chrono::NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S").ok()?;
    let started = naive.and_utc();
    let diff = chrono::Utc::now().signed_duration_since(started);
    if diff.num_seconds() >= 0 {
        Some(diff.num_seconds() as u64)
    } else {
        Some(0)
    }
}

/// Read the last `lines` journal entries for a service.
pub fn read_recent_logs(service_name: &str, lines: usize) -> Vec<String> {
    let lines_arg = lines.to_string();
    let output = Command::new("journalctl")
        .args([
            "-u",
            service_name,
            "-n",
            &lines_arg,
            "--no-pager",
            "-o",
            "short-iso",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Stream journalctl logs directly to the terminal.
pub fn stream_logs(service_name: &str, lines: usize, follow: bool) -> Result<()> {
    let mut cmd = Command::new("journalctl");
    cmd.args(["-u", service_name, "-n", &lines.to_string(), "--no-pager"]);
    if follow {
        cmd.arg("-f");
    }
    let status = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to launch `journalctl`")?;

    if !status.success() {
        anyhow::bail!("journalctl exited with status {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RunnerScope;
    use chrono::Utc;
    use std::path::PathBuf;

    #[test]
    fn renders_valid_systemd_unit() {
        let rec = RunnerRecord {
            name: "vps-01".into(),
            provider: crate::state::ProviderKind::Github,
            url: "https://github.com/acme/api".into(),
            scope: RunnerScope::Repo,
            labels: vec!["vps".into()],
            runner_group: None,
            linux_user: "rakku".into(),
            container_runtime: crate::state::ContainerRuntime::Docker,
            docker_access: true,
            sudo_access: false,
            install_dir: PathBuf::from("/opt/rakku/runners/vps-01"),
            service_name: "rakku-vps-01.service".into(),
            runner_version: "2.323.0".into(),
            ephemeral: false,
            installed_at: Utc::now(),
            updated_at: None,
        };
        let unit = render_unit_file(&rec);
        assert!(unit.contains("User=rakku"));
        assert!(unit.contains("WorkingDirectory=/opt/rakku/runners/vps-01"));
        assert!(unit.contains("Runner.Listener run --startuptype service"));
        assert!(unit.contains("KillMode=control-group"));
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn parses_systemctl_show_output() {
        let raw = "\
ActiveState=active
SubState=running
MainPID=4219
NRestarts=2
MemoryCurrent=134217728
ActiveEnterTimestamp=n/a
";
        let st = parse_systemctl_show(raw);
        assert_eq!(st.active_state, "active");
        assert_eq!(st.sub_state, "running");
        assert_eq!(st.main_pid, Some(4219));
        assert_eq!(st.restarts, 2);
        assert_eq!(st.memory_bytes, Some(134_217_728));
    }
}
