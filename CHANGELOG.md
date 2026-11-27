# Changelog

All notable changes to **`rakku`** are documented here.

## [0.3.0] - 2026-11-27

### Added
- `rakku clean <name|--all>` to reclaim disk space from `_diag/` logs and (with `--work`) idle `_work/` / `builds/` / `cache/` workspaces.
- Live per-runner **`DISK`** usage column in `rakku list`, `rakku status`, and `rakku monitor`.
- Per-runner `systemd` resource limits: `--memory-limit` (`MemoryMax=`) and `--cpu-quota` (`CPUQuota=`).
- Custom environment variables via repeatable `--env KEY=VAL` and `--env-file <path>`.
- Inline download progress (MiB / %) across GitHub, GitLab, and Gitea runner downloads.
- Open-source community docs: `CONTRIBUTING.md`, `SECURITY.md`, `CHANGELOG.md`, and GitHub issue/PR templates.

## [0.2.0] - 2026-11-27

### Added
- Horizontal multi-provider architecture (`src/providers/`) supporting **GitHub Actions**, **GitLab Runner**, and **Gitea Actions** (`gitea-runner` / `act_runner`).
- Unified **`PROVIDER`** column in `rakku monitor` and `rakku list`.
- Backwards-compatible `state.json` migration for existing `v0.1.x` GitHub runners.

## [0.1.0] - 2026-11-26

### Added
- Initial Rust CLI (`ureq` + `rustls`, zero runtime dependencies) for automated Linux runner setup, `systemd` service management, `doctor` diagnostics, `update`, `upgrade`, `remove`, and the live `rakku monitor` TUI dashboard.
- Container runtime selector (`none`, `docker`, `podman`) with rootless Podman linger and RHEL/Rocky/Alma support.
- One-line Linux installer (`install.sh`) and static `amd64`/`arm64` musl release builds.
