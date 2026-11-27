use std::process::Command;

use serde::Serialize;

use crate::providers;
use crate::state::{ProviderKind, RunnerRecord, RunnerScope};
use crate::system::systemd;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", content = "detail", rename_all = "lowercase")]
pub enum JobState {
    Idle,
    Running(String),
    Starting,
    Error(String),
    Offline,
    Unknown,
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobState::Idle => write!(f, "Idle"),
            JobState::Running(job) => write!(f, "Running: {job}"),
            JobState::Starting => write!(f, "Starting"),
            JobState::Error(msg) => write!(f, "Error: {msg}"),
            JobState::Offline => write!(f, "Offline"),
            JobState::Unknown => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerLiveStatus {
    pub name: String,
    pub provider: ProviderKind,
    pub url: String,
    pub scope: RunnerScope,
    pub linux_user: String,
    pub runner_version: String,
    pub service_name: String,
    pub labels: Vec<String>,
    pub active_state: String,
    pub sub_state: String,
    pub pid: Option<u32>,
    pub restarts: u32,
    pub memory_bytes: Option<u64>,
    pub cpu_percent: Option<f32>,
    pub uptime_secs: Option<u64>,
    pub job_state: JobState,
}

/// Collect live status, resource usage, and current GitHub job state for a runner.
pub fn collect_runner_status(record: &RunnerRecord) -> RunnerLiveStatus {
    let svc = systemd::inspect_service(&record.service_name);
    let cpu_percent = svc.main_pid.and_then(read_pid_cpu_percent);
    let memory_bytes = svc
        .memory_bytes
        .or_else(|| svc.main_pid.and_then(read_pid_rss_bytes));

    let job_state = if svc.active_state != "active" {
        JobState::Offline
    } else {
        let recent_logs = systemd::read_recent_logs(&record.service_name, 30);
        providers::infer_job_state(record.provider, &recent_logs)
    };

    RunnerLiveStatus {
        name: record.name.clone(),
        provider: record.provider,
        url: record.url.clone(),
        scope: record.scope.clone(),
        linux_user: record.linux_user.clone(),
        runner_version: record.runner_version.clone(),
        service_name: record.service_name.clone(),
        labels: record.labels.clone(),
        active_state: svc.active_state,
        sub_state: svc.sub_state,
        pid: svc.main_pid,
        restarts: svc.restarts,
        memory_bytes,
        cpu_percent,
        uptime_secs: svc.uptime_secs,
        job_state,
    }
}

/// Infer whether a GitHub Actions runner is idle, busy, or encountering connect errors.
pub fn infer_github_job_state(lines: &[String]) -> JobState {
    for line in lines.iter().rev() {
        if line.contains("A session for this runner already exists") {
            return JobState::Error("Session conflict".into());
        }
        if line.contains("Runner connect error:") {
            return JobState::Error("Connect error".into());
        }
        if let Some(pos) = line.find("Running job:") {
            let job = line[pos + "Running job:".len()..].trim();
            return JobState::Running(if job.is_empty() {
                "job".to_string()
            } else {
                job.to_string()
            });
        }
        if line.contains("completed with result:") || line.contains("Listening for Jobs") {
            return JobState::Idle;
        }
        if line.contains("Connected to GitHub") || line.contains("Current runner version:") {
            return JobState::Starting;
        }
    }
    JobState::Unknown
}

fn read_pid_cpu_percent(pid: u32) -> Option<f32> {
    let out = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "%cpu="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f32>()
        .ok()
}

fn read_pid_rss_bytes(pid: u32) -> Option<u64> {
    let out = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "rss="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let kb = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(kb * 1024)
}

pub fn format_bytes(bytes: Option<u64>) -> String {
    match bytes {
        None => "-".into(),
        Some(b) if b >= 1024 * 1024 * 1024 => {
            format!("{:.1} GiB", b as f64 / (1024.0 * 1024.0 * 1024.0))
        }
        Some(b) if b >= 1024 * 1024 => format!("{:.1} MiB", b as f64 / (1024.0 * 1024.0)),
        Some(b) if b >= 1024 => format!("{:.0} KiB", b as f64 / 1024.0),
        Some(b) => format!("{b} B"),
    }
}

pub fn format_uptime(secs: Option<u64>) -> String {
    match secs {
        None => "-".into(),
        Some(s) => {
            let days = s / 86400;
            let hours = (s % 86400) / 3600;
            let mins = (s % 3600) / 60;
            let rem = s % 60;
            if days > 0 {
                format!("{days}d {hours}h")
            } else if hours > 0 {
                format!("{hours}h {mins}m")
            } else if mins > 0 {
                format!("{mins}m {rem}s")
            } else {
                format!("{rem}s")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_running_and_idle_job_states() {
        let running = vec![
            "2026-11-26T10:00:00Z host run.sh: Listening for Jobs".to_string(),
            "2026-11-26T10:01:00Z host run.sh: Running job: build-and-test".to_string(),
        ];
        assert_eq!(
            infer_github_job_state(&running),
            JobState::Running("build-and-test".into())
        );

        let idle = vec![
            "2026-11-26T10:01:00Z host run.sh: Running job: build-and-test".to_string(),
            "2026-11-26T10:03:00Z host run.sh: Job build-and-test completed with result: Succeeded"
                .to_string(),
        ];
        assert_eq!(infer_github_job_state(&idle), JobState::Idle);
    }

    #[test]
    fn formats_bytes_and_uptime() {
        assert_eq!(format_bytes(Some(134_217_728)), "128.0 MiB");
        assert_eq!(format_uptime(Some(90_000)), "1d 1h");
        assert_eq!(format_uptime(Some(125)), "2m 5s");
    }
}
