# rakku *(楽駆)*

> Effortless Linux setup, lifecycle management, and local monitoring for **GitHub Actions self-hosted runners** — built in Rust by [fukulabs.dev](https://fukulabs.dev).

`rakku` (*raku* 楽 = effortless + *ku* 駆 = run) eliminates the repetitive manual work of provisioning self-hosted runners on Linux VPSs, servers, and VMs: dedicated user creation, permissions/Docker group setup, OS dependencies, runner registration, background `systemd` services, safe updates, clean deregistration, and live local monitoring.

## Features

- **One-command Linux runner setup (`install`):**
  - Preflight host checks + optional auto-install of missing prerequisites (`git`, `docker`, `podman`)
  - Creates a dedicated non-root Linux user (default: `rakku`)
  - Container runtime selector (`none`, `docker`, or `podman` with rootless `systemd` linger) + optional `sudo` access
  - Auto-detects CPU architecture (`x64` / `arm64`), downloads the latest `actions/runner` tarball, verifies SHA-256, and installs OS dependencies (`./bin/installdependencies.sh`)
  - Supports **GitHub PAT** (auto-mints registration & removal tokens via API) or **manual registration tokens**
  - Supports **repository**, **organization**, and **enterprise** runners
  - Generates, enables, and starts a managed `systemd` service with automatic rollback on partial failure
- **Multi-runner support:** Run and track multiple independent runners on the same VPS
- **Live local monitoring (`monitor`):**
  - Real-time terminal dashboard (TUI) for every runner installed by `rakku`
  - Displays `systemd` state, live job activity (`Idle` / `Running: <job>`), PID, CPU %, RAM, uptime, restart count, and live `journalctl` logs
  - Built-in hotkeys to `restart` (`r`), `start/stop` (`s`), toggle logs (`l`), or refresh (`Space`)
- **Full lifecycle management:** `list` (with `--json`), `status`, `logs`, `start`, `stop`, `restart`, `upgrade` (upgrades `actions/runner`, skips busy runners, downloads once before stopping), `update` (updates `rakku` itself), `doctor`, and clean `remove` (deregisters from GitHub so no ghost offline runners remain)

## Quick Install (Linux VPS)

```bash
curl -fsSL https://raw.githubusercontent.com/myzskdev/rakku/main/install.sh | sudo sh
```

## Quick Usage (on Linux VPS)

### Check host readiness
```bash
rakku doctor
```

### Interactive setup wizard *(aliases: `install`, `setup`, `init`, `add`)*
```bash
sudo rakku setup
```
Enter your repo (`owner/repo` or full GitHub URL) and a PAT or manual token, then press **Enter** to accept sensible defaults for everything else.

### Non-interactive install (scripts / cloud-init / Terraform)
```bash
sudo rakku install \
  --url owner/repo \
  --pat "$GITHUB_PAT" \
  --name vps-prod-01 \
  --labels linux,vps,docker \
  --docker \
  --install-missing \
  --non-interactive
```

### Monitor all local runners live
```bash
rakku monitor
```

### Other commands
```bash
rakku list                 # Status table of all local runners
rakku list --json          # Machine-readable JSON
rakku status vps-prod-01   # Detailed status + recent logs
rakku logs vps-prod-01 -f  # Follow journalctl logs
sudo rakku restart vps-prod-01
sudo rakku update          # Update rakku itself from GitHub Releases
sudo rakku upgrade --all   # Upgrade all idle local runners to latest actions/runner
sudo rakku remove vps-prod-01 --pat "$GITHUB_PAT"
```

## Linux Filesystem Layout

| Path | Purpose |
|---|---|
| `/var/lib/rakku/state.json` | Local registry of runners installed by `rakku` |
| `/opt/rakku/runners/<name>/` | Individual runner installation directories |
| `/etc/systemd/system/rakku-<name>.service` | Managed `systemd` unit files |

## Building from Source

```bash
cargo test
cargo build --release
```

For a static Linux `amd64` binary:
```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

## License

MIT — [fukulabs.dev](https://fukulabs.dev)
