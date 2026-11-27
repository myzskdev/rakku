pub mod gitea;
pub mod gitlab;

use crate::state::{ProviderKind, RunnerRecord};
use crate::system::metrics::JobState;

/// Return the `systemd` `ExecStart=` command for a managed runner based on its provider.
pub fn exec_start_command(record: &RunnerRecord) -> String {
    match record.provider {
        ProviderKind::Github => {
            let listener = record.install_dir.join("bin").join("Runner.Listener");
            format!("{} run --startuptype service", listener.to_string_lossy())
        }
        ProviderKind::Gitlab => gitlab::exec_start(record),
        ProviderKind::Gitea => gitea::exec_start(record),
    }
}

/// Infer live `JobState` from recent journalctl log lines using provider-specific log patterns.
pub fn infer_job_state(provider: ProviderKind, lines: &[String]) -> JobState {
    match provider {
        ProviderKind::Github => crate::system::metrics::infer_github_job_state(lines),
        ProviderKind::Gitlab => gitlab::infer_job_state(lines),
        ProviderKind::Gitea => gitea::infer_job_state(lines),
    }
}
