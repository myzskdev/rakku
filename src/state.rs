use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    #[default]
    Github,
    Gitlab,
    Gitea,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::Github => write!(f, "github"),
            ProviderKind::Gitlab => write!(f, "gitlab"),
            ProviderKind::Gitea => write!(f, "gitea"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RunnerScope {
    Repo,
    Org,
    Enterprise,
    Instance,
}

impl std::fmt::Display for RunnerScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerScope::Repo => write!(f, "repo"),
            RunnerScope::Org => write!(f, "org"),
            RunnerScope::Enterprise => write!(f, "enterprise"),
            RunnerScope::Instance => write!(f, "instance"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContainerRuntime {
    #[default]
    None,
    Docker,
    Podman,
}

impl std::fmt::Display for ContainerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerRuntime::None => write!(f, "none"),
            ContainerRuntime::Docker => write!(f, "docker"),
            ContainerRuntime::Podman => write!(f, "podman"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerRecord {
    pub name: String,
    #[serde(default)]
    pub provider: ProviderKind,
    pub url: String,
    pub scope: RunnerScope,
    pub labels: Vec<String>,
    pub runner_group: Option<String>,
    pub linux_user: String,
    #[serde(default)]
    pub container_runtime: ContainerRuntime,
    #[serde(default)]
    pub docker_access: bool,
    pub sudo_access: bool,
    pub install_dir: PathBuf,
    pub service_name: String,
    pub runner_version: String,
    pub ephemeral: bool,
    #[serde(default)]
    pub memory_limit: Option<String>,
    #[serde(default)]
    pub cpu_quota: Option<String>,
    #[serde(default)]
    pub env_vars: Vec<String>,
    #[serde(default)]
    pub env_file: Option<PathBuf>,
    pub installed_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStore {
    pub schema_version: u32,
    pub runners: Vec<RunnerRecord>,
}

impl Default for StateStore {
    fn default() -> Self {
        Self {
            schema_version: 1,
            runners: Vec::new(),
        }
    }
}

impl StateStore {
    pub fn load_default() -> Result<Self> {
        Self::load_from(&config::state_file_path())
    }

    pub fn save_default(&self) -> Result<()> {
        self.save_to(&config::state_file_path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read state file {}", path.display()))?;
        let store: Self = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse state file {}", path.display()))?;
        Ok(store)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create state directory {}", parent.display())
            })?;
        }
        let payload = serde_json::to_string_pretty(self)?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, payload)
            .with_context(|| format!("failed to write temporary state {}", tmp_path.display()))?;
        fs::rename(&tmp_path, path)
            .with_context(|| format!("failed to persist state file {}", path.display()))?;
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&RunnerRecord> {
        self.runners.iter().find(|r| r.name == name)
    }

    pub fn find_mut(&mut self, name: &str) -> Option<&mut RunnerRecord> {
        self.runners.iter_mut().find(|r| r.name == name)
    }

    pub fn upsert(&mut self, record: RunnerRecord) {
        if let Some(existing) = self.find_mut(&record.name) {
            *existing = record;
        } else {
            self.runners.push(record);
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<RunnerRecord> {
        let idx = self.runners.iter().position(|r| r.name == name)?;
        Some(self.runners.remove(idx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_record(name: &str) -> RunnerRecord {
        RunnerRecord {
            name: name.to_string(),
            provider: ProviderKind::Github,
            url: "https://github.com/acme/demo".to_string(),
            scope: RunnerScope::Repo,
            labels: vec!["linux".into(), "vps".into()],
            runner_group: None,
            linux_user: "rakku".into(),
            container_runtime: ContainerRuntime::Docker,
            docker_access: true,
            sudo_access: false,
            install_dir: PathBuf::from(format!("/opt/rakku/runners/{name}")),
            service_name: format!("rakku-{name}.service"),
            runner_version: "2.323.0".into(),
            ephemeral: false,
            memory_limit: None,
            cpu_quota: None,
            env_vars: Vec::new(),
            env_file: None,
            installed_at: Utc::now(),
            updated_at: None,
        }
    }

    #[test]
    fn roundtrip_save_and_load() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("state.json");

        let mut store = StateStore::load_from(&state_path).unwrap();
        assert!(store.runners.is_empty());

        store.upsert(sample_record("runner-a"));
        store.upsert(sample_record("runner-b"));
        store.save_to(&state_path).unwrap();

        let reloaded = StateStore::load_from(&state_path).unwrap();
        assert_eq!(reloaded.runners.len(), 2);
        assert_eq!(reloaded.find("runner-a").unwrap().runner_version, "2.323.0");
    }

    #[test]
    fn upsert_updates_and_remove_deletes() {
        let mut store = StateStore::default();
        store.upsert(sample_record("r1"));
        let mut updated = sample_record("r1");
        updated.runner_version = "2.324.0".into();
        store.upsert(updated);

        assert_eq!(store.runners.len(), 1);
        assert_eq!(store.find("r1").unwrap().runner_version, "2.324.0");

        let removed = store.remove("r1");
        assert!(removed.is_some());
        assert!(store.runners.is_empty());
    }

    #[test]
    fn loads_legacy_v01_state_without_provider_or_runtime() {
        let legacy_json = r#"{
          "schema_version": 1,
          "runners": [
            {
              "name": "voucher-gen",
              "url": "https://github.com/myzskdev/voucher",
              "scope": "repo",
              "labels": ["vps"],
              "runner_group": null,
              "linux_user": "rakku",
              "docker_access": false,
              "sudo_access": false,
              "install_dir": "/opt/rakku/runners/voucher-gen",
              "service_name": "rakku-voucher-gen.service",
              "runner_version": "2.323.0",
              "ephemeral": false,
              "installed_at": "2026-11-26T15:00:00Z",
              "updated_at": null
            }
          ]
        }"#;

        let store: StateStore = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(store.runners.len(), 1);
        assert_eq!(store.runners[0].provider, ProviderKind::Github);
        assert_eq!(store.runners[0].container_runtime, ContainerRuntime::None);
    }
}
