use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tar::Archive;

use crate::config::{ACTIONS_RUNNER_REPO, APP_NAME, GITHUB_API_BASE};
use crate::state::RunnerScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubTarget {
    Repo { owner: String, repo: String },
    Org { org: String },
    Enterprise { enterprise: String },
}

impl GitHubTarget {
    /// Parse a GitHub URL or `owner/repo` shorthand into a repo, org, or enterprise target.
    ///
    /// Examples:
    /// - `owner/repo` -> Repo (`https://github.com/owner/repo`)
    /// - `https://github.com/owner/repo` -> Repo
    /// - `https://github.com/my-org` -> Org
    /// - `https://github.com/enterprises/my-ent` -> Enterprise
    pub fn parse(raw_url: &str) -> Result<Self> {
        let trimmed = raw_url
            .trim()
            .trim_end_matches('/')
            .trim_end_matches(".git");
        if trimmed.is_empty() {
            bail!("GitHub URL or owner/repo cannot be empty");
        }

        // Allow convenient `owner/repo` shorthand without requiring `https://github.com/`.
        if !trimmed.contains("://") {
            let parts: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
            if parts.len() == 2 && !parts[0].contains('.') {
                return Ok(Self::Repo {
                    owner: parts[0].to_string(),
                    repo: parts[1].to_string(),
                });
            }
            bail!(
                "invalid GitHub target '{raw_url}': use `owner/repo` or a full URL like `https://github.com/owner/repo`"
            );
        }

        let after_scheme = trimmed
            .strip_prefix("https://")
            .or_else(|| trimmed.strip_prefix("http://"))
            .ok_or_else(|| anyhow!("URL must start with https:// or http:// (got: {raw_url})"))?;

        let parts: Vec<&str> = after_scheme.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() < 2 {
            bail!("invalid GitHub URL '{raw_url}': expected host plus org or owner/repo");
        }

        // parts[0] is host (e.g. github.com)
        match parts.as_slice() {
            [_host, "enterprises", enterprise] => Ok(Self::Enterprise {
                enterprise: (*enterprise).to_string(),
            }),
            [_host, org] => Ok(Self::Org {
                org: (*org).to_string(),
            }),
            [_host, owner, repo] => Ok(Self::Repo {
                owner: (*owner).to_string(),
                repo: (*repo).to_string(),
            }),
            _ => bail!(
                "unsupported GitHub URL format '{raw_url}'. Expected owner/repo, https://github.com/<org>, https://github.com/<owner>/<repo>, or https://github.com/enterprises/<slug>"
            ),
        }
    }

    /// Return the canonical `https://github.com/...` URL for this target.
    pub fn canonical_url(&self) -> String {
        match self {
            Self::Repo { owner, repo } => format!("https://github.com/{owner}/{repo}"),
            Self::Org { org } => format!("https://github.com/{org}"),
            Self::Enterprise { enterprise } => {
                format!("https://github.com/enterprises/{enterprise}")
            }
        }
    }

    pub fn scope(&self) -> RunnerScope {
        match self {
            Self::Repo { .. } => RunnerScope::Repo,
            Self::Org { .. } => RunnerScope::Org,
            Self::Enterprise { .. } => RunnerScope::Enterprise,
        }
    }

    pub fn registration_token_endpoint(&self) -> String {
        match self {
            Self::Repo { owner, repo } => {
                format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/actions/runners/registration-token")
            }
            Self::Org { org } => {
                format!("{GITHUB_API_BASE}/orgs/{org}/actions/runners/registration-token")
            }
            Self::Enterprise { enterprise } => {
                format!(
                    "{GITHUB_API_BASE}/enterprises/{enterprise}/actions/runners/registration-token"
                )
            }
        }
    }

    pub fn remove_token_endpoint(&self) -> String {
        match self {
            Self::Repo { owner, repo } => {
                format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/actions/runners/remove-token")
            }
            Self::Org { org } => {
                format!("{GITHUB_API_BASE}/orgs/{org}/actions/runners/remove-token")
            }
            Self::Enterprise { enterprise } => {
                format!("{GITHUB_API_BASE}/enterprises/{enterprise}/actions/runners/remove-token")
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseResponse {
    tag_name: String,
    body: Option<String>,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct RunnerTokenResponse {
    token: String,
}

#[derive(Debug, Clone)]
pub struct RunnerReleaseInfo {
    pub version: String,
    pub download_url: String,
    pub asset_name: String,
    pub expected_sha256: Option<String>,
}

pub struct GitHubClient {
    agent: ureq::Agent,
}

impl GitHubClient {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .user_agent(&format!("{APP_NAME}/{}", env!("CARGO_PKG_VERSION")))
            .build();
        Self { agent }
    }

    /// Mint a short-lived registration token using a GitHub PAT.
    pub fn create_registration_token(&self, target: &GitHubTarget, pat: &str) -> Result<String> {
        self.post_token(&target.registration_token_endpoint(), pat)
            .context(
            "failed to create GitHub runner registration token (check PAT scopes: repo/admin:org)",
        )
    }

    /// Mint a short-lived removal token using a GitHub PAT.
    pub fn create_remove_token(&self, target: &GitHubTarget, pat: &str) -> Result<String> {
        self.post_token(&target.remove_token_endpoint(), pat)
            .context(
                "failed to create GitHub runner removal token (check PAT scopes: repo/admin:org)",
            )
    }

    fn post_token(&self, url: &str, pat: &str) -> Result<String> {
        let resp = self
            .agent
            .post(url)
            .set("Accept", "application/vnd.github+json")
            .set("Authorization", &format!("Bearer {pat}"))
            .set("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(|e| map_ureq_error(e, url))?;

        let parsed: RunnerTokenResponse = resp
            .into_json()
            .context("failed to parse GitHub token JSON response")?;
        Ok(parsed.token)
    }

    /// Lightweight reachability check against GitHub's API (used by `doctor` / preflight).
    pub fn check_api_reachable(&self) -> Result<()> {
        let url = format!("{GITHUB_API_BASE}/zen");
        self.agent
            .get(&url)
            .call()
            .map_err(|e| map_ureq_error(e, &url))?;
        Ok(())
    }

    /// Fetch metadata for the latest (or a specific) `actions/runner` release for the given Linux arch.
    pub fn fetch_runner_release(
        &self,
        requested_version: Option<&str>,
        runner_arch: &str,
        pat: Option<&str>,
    ) -> Result<RunnerReleaseInfo> {
        let api_url = match requested_version {
            Some(v) => {
                let tag = if v.starts_with('v') {
                    v.to_string()
                } else {
                    format!("v{v}")
                };
                format!("{GITHUB_API_BASE}/repos/{ACTIONS_RUNNER_REPO}/releases/tags/{tag}")
            }
            None => format!("{GITHUB_API_BASE}/repos/{ACTIONS_RUNNER_REPO}/releases/latest"),
        };

        let mut req = self
            .agent
            .get(&api_url)
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = pat {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }

        let release: ReleaseResponse = req
            .call()
            .map_err(|e| map_ureq_error(e, &api_url))?
            .into_json()
            .context("failed to parse actions/runner release metadata")?;

        let version = release.tag_name.trim_start_matches('v').to_string();
        let expected_asset = format!("actions-runner-linux-{runner_arch}-{version}.tar.gz");

        let asset = release
            .assets
            .into_iter()
            .find(|a| a.name == expected_asset)
            .ok_or_else(|| {
                anyhow!(
                    "could not find release asset '{expected_asset}' in release {}",
                    release.tag_name
                )
            })?;

        let expected_sha256 = release
            .body
            .as_deref()
            .and_then(|body| extract_sha256_from_release_body(body, &asset.name));

        Ok(RunnerReleaseInfo {
            version,
            download_url: asset.browser_download_url,
            asset_name: asset.name,
            expected_sha256,
        })
    }

    /// Download the runner tarball to `archive_path` and verify its SHA-256 checksum (when available).
    pub fn download_tarball(&self, release: &RunnerReleaseInfo, archive_path: &Path) -> Result<()> {
        if let Some(parent) = archive_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }

        let resp = self
            .agent
            .get(&release.download_url)
            .call()
            .map_err(|e| map_ureq_error(e, &release.download_url))?;

        let total_bytes = resp
            .header("Content-Length")
            .and_then(|v| v.parse::<u64>().ok());
        let mut reader = resp.into_reader();
        let mut out = File::create(archive_path)
            .with_context(|| format!("failed to create {}", archive_path.display()))?;

        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        let mut downloaded: u64 = 0;
        let mut last_print = std::time::Instant::now();
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            downloaded += n as u64;
            if last_print.elapsed() >= std::time::Duration::from_millis(200) {
                print_download_progress(downloaded, total_bytes);
                last_print = std::time::Instant::now();
            }
        }
        print_download_progress(downloaded, total_bytes);
        println!();
        out.flush()?;
        drop(out);

        let actual_sha = format!("{:x}", hasher.finalize());
        if let Some(expected) = &release.expected_sha256 {
            if !actual_sha.eq_ignore_ascii_case(expected) {
                let _ = fs::remove_file(archive_path);
                bail!(
                    "SHA-256 mismatch for {}: expected {}, got {}",
                    release.asset_name,
                    expected,
                    actual_sha
                );
            }
        }
        Ok(())
    }

    /// Extract a previously downloaded `.tar.gz` runner archive into `dest_dir`.
    pub fn extract_tarball(archive_path: &Path, dest_dir: &Path) -> Result<()> {
        fs::create_dir_all(dest_dir)
            .with_context(|| format!("failed to create runner directory {}", dest_dir.display()))?;
        let tar_gz = File::open(archive_path)
            .with_context(|| format!("failed to open archive {}", archive_path.display()))?;
        let dec = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(dec);
        archive.unpack(dest_dir).with_context(|| {
            format!(
                "failed to extract {} into {}",
                archive_path.display(),
                dest_dir.display()
            )
        })?;
        Ok(())
    }

    /// Download the runner tarball, verify its SHA-256 checksum (when available), and extract it.
    pub fn download_and_extract(&self, release: &RunnerReleaseInfo, dest_dir: &Path) -> Result<()> {
        fs::create_dir_all(dest_dir)
            .with_context(|| format!("failed to create runner directory {}", dest_dir.display()))?;
        let archive_path = dest_dir.join(&release.asset_name);
        self.download_tarball(release, &archive_path)?;
        let res = Self::extract_tarball(&archive_path, dest_dir);
        let _ = fs::remove_file(&archive_path);
        res
    }

    /// Fetch a GitHub release for this CLI itself (`self-update`).
    pub fn fetch_cli_release(
        &self,
        repo: &str,
        requested_tag: Option<&str>,
        asset_name: &str,
    ) -> Result<RunnerReleaseInfo> {
        let api_url = match requested_tag {
            Some(tag) => {
                let normalized = if tag.starts_with('v') {
                    tag.to_string()
                } else {
                    format!("v{tag}")
                };
                format!("{GITHUB_API_BASE}/repos/{repo}/releases/tags/{normalized}")
            }
            None => format!("{GITHUB_API_BASE}/repos/{repo}/releases/latest"),
        };

        let release: ReleaseResponse = self
            .agent
            .get(&api_url)
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(|e| map_ureq_error(e, &api_url))?
            .into_json()
            .context("failed to parse CLI release metadata")?;

        let version = release.tag_name.trim_start_matches('v').to_string();
        let checksum_asset = release
            .assets
            .iter()
            .find(|a| a.name == "SHA256SUMS.txt")
            .map(|a| a.browser_download_url.clone());

        let asset = release
            .assets
            .into_iter()
            .find(|a| a.name == asset_name)
            .ok_or_else(|| {
                anyhow!(
                    "release {} in {repo} does not contain asset '{asset_name}'",
                    release.tag_name
                )
            })?;

        let expected_sha256 = checksum_asset.and_then(|url| {
            let body = self.agent.get(&url).call().ok()?.into_string().ok()?;
            extract_sha256_from_release_body(&body, &asset.name)
        });

        Ok(RunnerReleaseInfo {
            version,
            download_url: asset.browser_download_url,
            asset_name: asset.name,
            expected_sha256,
        })
    }

    /// Download a binary file and verify its SHA-256 checksum if provided.
    pub fn download_binary(&self, release: &RunnerReleaseInfo, dest: &Path) -> Result<()> {
        self.download_tarball(release, dest)
    }
}

/// Map `std::env::consts::ARCH` to GitHub's `actions/runner` Linux architecture suffix.
pub fn current_runner_arch() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("x64"),
        "aarch64" => Ok("arm64"),
        "arm" => Ok("arm"),
        other => bail!("unsupported CPU architecture for GitHub Actions runner: {other}"),
    }
}

/// Parse SHA-256 checksum for a given asset filename from GitHub release notes.
/// GitHub release bodies typically contain lines like:
/// `<!-- BEGIN SHA linux-x64 -->abc123...<!-- END SHA linux-x64 -->`
/// or `<64-hex-chars>  actions-runner-linux-x64-2.323.0.tar.gz`.
pub fn extract_sha256_from_release_body(body: &str, asset_name: &str) -> Option<String> {
    for line in body.lines() {
        if line.contains(asset_name) {
            for token in line.split_whitespace() {
                let cleaned = token.trim_matches(|c: char| !c.is_ascii_hexdigit());
                if cleaned.len() == 64 && cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Some(cleaned.to_lowercase());
                }
            }
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct GitHubErrorBody {
    message: Option<String>,
}

/// Convert `ureq::Error` into an actionable, human-friendly error message.
pub fn map_ureq_error(err: ureq::Error, url: &str) -> anyhow::Error {
    match err {
        ureq::Error::Status(code, resp) => {
            let api_msg = resp
                .into_json::<GitHubErrorBody>()
                .ok()
                .and_then(|b| b.message)
                .unwrap_or_default();
            let detail = if api_msg.is_empty() {
                String::new()
            } else {
                format!(" ({api_msg})")
            };

            match code {
                401 => anyhow!(
                    "GitHub API 401 Unauthorized{detail}: your PAT is invalid or expired. Generate a fresh token and retry."
                ),
                403 => anyhow!(
                    "GitHub API 403 Forbidden{detail}: check that your PAT has `repo` (or `admin:org` / fine-grained `Administration: Read & Write` + `Self-hosted runners: Read & Write`) scope, or that you haven't hit the API rate limit."
                ),
                404 => anyhow!(
                    "GitHub API 404 Not Found{detail}: verify the URL ({url}) and ensure your PAT has admin access to this repository/organization."
                ),
                422 => anyhow!("GitHub API 422 Validation Failed{detail} for {url}"),
                other => anyhow!("GitHub API returned HTTP {other}{detail} for {url}"),
            }
        }
        ureq::Error::Transport(t) => anyhow!(
            "Network error reaching {url}: {t}. Check internet connectivity, DNS, and firewall rules."
        ),
    }
}

/// Print a lightweight inline download progress line (`\r`) without extra dependencies.
pub fn print_download_progress(downloaded: u64, total: Option<u64>) {
    let dl_mb = downloaded as f64 / (1024.0 * 1024.0);
    match total {
        Some(t) if t > 0 => {
            let total_mb = t as f64 / (1024.0 * 1024.0);
            let pct = ((downloaded as f64 / t as f64) * 100.0).min(100.0);
            print!("\r      Progress: {dl_mb:.1} / {total_mb:.1} MiB ({pct:.0}%)   ");
        }
        _ => {
            print!("\r      Progress: {dl_mb:.1} MiB   ");
        }
    }
    let _ = io::stdout().flush();
}

#[allow(dead_code)]
fn _assert_io_import(_: io::Error) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repo_org_and_enterprise_urls() {
        let shorthand = GitHubTarget::parse("octocat/hello-world").unwrap();
        assert_eq!(
            shorthand,
            GitHubTarget::Repo {
                owner: "octocat".into(),
                repo: "hello-world".into()
            }
        );
        assert_eq!(
            shorthand.canonical_url(),
            "https://github.com/octocat/hello-world"
        );
        assert_eq!(
            GitHubTarget::parse("https://github.com/octocat/hello-world.git").unwrap(),
            GitHubTarget::Repo {
                owner: "octocat".into(),
                repo: "hello-world".into()
            }
        );
        assert_eq!(
            GitHubTarget::parse("https://github.com/my-org/").unwrap(),
            GitHubTarget::Org {
                org: "my-org".into()
            }
        );
        assert_eq!(
            GitHubTarget::parse("https://github.com/enterprises/my-corp").unwrap(),
            GitHubTarget::Enterprise {
                enterprise: "my-corp".into()
            }
        );
    }

    #[test]
    fn rejects_invalid_urls() {
        assert!(GitHubTarget::parse("github.com/foo/bar").is_err());
        assert!(GitHubTarget::parse("https://github.com/").is_err());
        assert!(GitHubTarget::parse("https://github.com/a/b/c").is_err());
    }

    #[test]
    fn extracts_sha256_from_release_notes() {
        let sha = "a".repeat(64);
        let body = format!("## Checksums\n`{sha}`  actions-runner-linux-x64-2.323.0.tar.gz\n");
        assert_eq!(
            extract_sha256_from_release_body(&body, "actions-runner-linux-x64-2.323.0.tar.gz"),
            Some(sha)
        );
    }
}
