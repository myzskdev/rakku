# rakku *(楽駆)*

[![CI](https://github.com/myzskdev/rakku/actions/workflows/ci.yml/badge.svg)](https://github.com/myzskdev/rakku/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/myzskdev/rakku)](https://github.com/myzskdev/rakku/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Built by fukulabs.dev](https://img.shields.io/badge/by-fukulabs.dev-black)](https://fukulabs.dev)

> **Open-source** Rust CLI for effortless Linux setup, lifecycle management, and live local monitoring of self-hosted CI runners — **GitHub Actions**, **GitLab Runner**, and **Gitea Actions** — by [fukulabs.dev](https://fukulabs.dev).

```text
┌ rakku monitor | Total: 3  Active: 3  Busy: 1  Offline: 0 | Updated: 15:20:04 ─────────────────────┐
│ NAME            PROVIDER SERVICE  JOB                  USER     CPU    MEM      DISK     UPTIME   │
│ ▶ prod-api-01   github   active   Running: build-test  rakku    42.3%  148 MiB  1.2 GiB  4d 11h   │
│   gl-worker-01  gitlab   active   Idle                 rakku     0.4%   52 MiB  412 MiB  4d 11h   │
│   gt-edge-01    gitea    active   Idle                 rakku     0.2%   38 MiB  280 MiB  2d 03h   │
└───────────────────────────────────────────────────────────────────────────────────────────────────┘
 [↑/↓ or j/k] Select  [r] Restart  [s] Start/Stop  [l] Toggle Logs  [Space] Refresh  [q] Quit
```

`rakku` (*raku* 楽 = effortless + *ku* 駆 = run) is a free, open-source, zero-dependency static binary that automates the painful manual steps of running self-hosted CI runners on Linux VPSs, servers, and VMs:

- Dedicated non-root Linux user & permission setup
- Container runtime configuration (**None**, **Docker**, or rootless **Podman**)
- Automatic OS dependency installation (`git`, `docker`/`podman`, `.NET` libs)
- Runner download, SHA-256 checksum verification, and registration
- Managed `systemd` background services (`KillMode=control-group`, auto-start on boot, automatic rollback on failure)
- One unified live terminal dashboard (`rakku monitor`) for every runner on the machine

---

## Supported Providers

| Provider | Runner Engine | Authentication | Notes |
|---|---|---|---|
| **GitHub Actions** (`github`) | `actions/runner` | GitHub PAT *(recommended)* or 1-hour registration token | Supports repo (`owner/repo`), org, and enterprise |
| **GitLab Runner** (`gitlab`) | `gitlab-runner` | `glrt-*` auth token or legacy registration token | Supports `shell` and `docker` executors (`gitlab.com` & self-hosted) |
| **Gitea Actions** (`gitea`) | `gitea-runner` / `act_runner` | Gitea runner registration token | Supports self-hosted Gitea instances with SHA-256 verification |

---

## Quick Start Tutorial (Linux VPS)

### Step 1 — Install `rakku`
```bash
curl -fsSL https://raw.githubusercontent.com/myzskdev/rakku/main/install.sh | sudo sh
```
Works on Ubuntu, Debian, RHEL, Rocky, AlmaLinux, Fedora, and Arch (`amd64` & `arm64`).

### Step 2 — Check server readiness
```bash
rakku doctor
```
Verifies Linux, `systemd`, core utilities, `git`, `docker`/`podman`, and network reachability.

### Step 3 — Set up your first runner
```bash
sudo rakku setup
```
*(Aliases: `install`, `setup`, `init`, `add`)*

The interactive wizard will ask you:
1. **CI Runner provider** — GitHub Actions, GitLab Runner, or Gitea Actions
2. **Target** — e.g. `owner/repo` (GitHub), `https://gitlab.com`, or `https://gitea.example.com`
3. **Token** — PAT or runner registration token
4. **Defaults** — press **Enter** to accept sensible defaults for runner name, labels, `rakku` user, and container runtime (`none` / `docker` / `podman`)

### Step 4 — Monitor all local runners live
```bash
rakku monitor
```
Displays every runner installed by `rakku` on this server in real time: provider, `systemd` state, live job activity (`Idle`, `Running`, `Error`), CPU %, RAM, uptime, restarts, and live `journalctl` logs.

**Monitor TUI Hotkeys:**
| Key | Action |
|---|---|
| `↑` / `↓` or `j` / `k` | Select runner |
| `r` | Restart selected runner *(requires `sudo rakku monitor`)* |
| `s` | Start / Stop selected runner *(requires `sudo rakku monitor`)* |
| `l` | Toggle live logs pane |
| `Space` | Force immediate refresh |
| `q` / `Esc` | Quit |

---

## Non-Interactive Automation Examples

Ideal for cloud-init, Terraform `user_data`, or shell provisioning scripts:

### GitHub Actions
> Create a PAT with `repo` (or fine-grained `Administration: Read & Write` + `Self-hosted runners: Read & Write`), or copy a 1-hour token from **Repo → Settings → Actions → Runners → New self-hosted runner**.
```bash
sudo rakku install \
  --provider github \
  --url owner/repo \
  --pat "$GITHUB_PAT" \
  --name vps-prod-01 \
  --labels linux,vps,docker \
  --runtime docker \
  --memory-limit 4G \
  --cpu-quota 200% \
  --env HTTP_PROXY=http://proxy.internal:8080 \
  --install-missing \
  --non-interactive
```

### GitLab Runner
> Create a runner in **Project/Group → Settings → CI/CD → Runners → New runner** and copy the `glrt-...` token.
```bash
sudo rakku install \
  --provider gitlab \
  --url https://gitlab.com \
  --token "$GITLAB_RUNNER_TOKEN" \
  --name gitlab-vps-01 \
  --runtime docker \
  --docker-image alpine:latest \
  --install-missing \
  --non-interactive
```

### Gitea Actions
> Copy a registration token from **Site/Org/Repo Settings → Actions → Runners → Create new runner**.
```bash
sudo rakku install \
  --provider gitea \
  --url https://gitea.example.com \
  --token "$GITEA_RUNNER_TOKEN" \
  --name gitea-vps-01 \
  --runtime podman \
  --install-missing \
  --non-interactive
```

---

## Command Reference

| Command | Root (`sudo`)? | Description |
|---|---|---|
| `rakku setup` *(or `install`, `init`, `add`)* | Yes | Install, register, and start a new CI runner |
| `rakku monitor` | Optional* | Live TUI dashboard for all local runners |
| `rakku list` | No | One-shot table of all local runners (`--json` for scripts) |
| `rakku status <name>` | No | Detailed runner config, metrics, and recent logs |
| `rakku logs <name> [-f] [-n 50]` | No | View or follow `journalctl` logs for a runner |
| `rakku start <name>` | Yes | Start a stopped runner service |
| `rakku stop <name>` | Yes | Stop a running runner service |
| `rakku restart <name>` | Yes | Restart a runner service |
| `rakku upgrade <name\|--all>` | Yes | Safely upgrade idle runner binaries (`--force` to interrupt busy jobs) |
| `rakku clean <name\|--all>` | Yes | Reclaim disk space from `_diag/` logs (`--work` also clears idle `_work/`/`builds/` caches) |
| `rakku remove <name>` | Yes | Stop service, deregister from provider, and clean up files |
| `rakku update` *(alias `self-update`)* | Yes | Update `rakku` itself to the latest GitHub Release (`--check` to inspect) |
| `rakku doctor` | No | Run host diagnostics (`systemd`, tools, `git`, `docker`/`podman`, network) |

*\*Running `rakku monitor` without `sudo` works for read-only monitoring; run with `sudo` to use `r` (restart) and `s` (start/stop) hotkeys.*

---

## Linux Filesystem Layout

| Path | Purpose |
|---|---|
| `/usr/local/bin/rakku` *(symlinked at `/usr/bin/rakku`)* | `rakku` CLI binary |
| `/var/lib/rakku/state.json` | Local registry of all runners managed by `rakku` |
| `/opt/rakku/runners/<name>/` | Individual runner installation directories |
| `/etc/systemd/system/rakku-<name>.service` | Managed `systemd` unit files |

---

## Building from Source

```bash
git clone https://github.com/myzskdev/rakku.git
cd rakku
cargo test
cargo build --release
```

Static Linux `amd64` build:
```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

---

## Open Source & Contributing

`rakku` is **open-source** software released under the [MIT License](LICENSE), maintained under [fukulabs.dev](https://fukulabs.dev).

Issues, feature ideas (like additional CI providers), and pull requests are welcome at https://github.com/myzskdev/rakku.
