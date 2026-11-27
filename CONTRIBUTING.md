# Contributing to `rakku`

Thank you for your interest in contributing to **`rakku`**!

## Development Setup

You need a recent Rust toolchain (`rustup`, `cargo`, `clippy`, `rustfmt`).

```bash
git clone https://github.com/myzskdev/rakku.git
cd rakku
cargo test
```

## Before Opening a Pull Request

Please make sure all checks pass locally:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Adding a New CI Runner Provider

`rakku` is designed to be horizontal across self-hosted CI runners:

1. Add a new variant to `ProviderKind` in `src/state.rs` and `ProviderArg` in `src/cli.rs`.
2. Create `src/providers/<provider>.rs` implementing:
   - Binary download & SHA-256 verification (using `ureq`)
   - Unattended registration (run as the dedicated non-root Linux user)
   - Clean deregistration (if supported by the upstream runner)
   - `exec_start(record)` for `systemd`
   - `infer_job_state(lines)` for `rakku monitor`
3. Wire it into `src/providers/mod.rs`, `src/commands/install.rs`, `src/commands/remove.rs`, and `src/commands/update.rs`.
4. Add unit tests for URL/release resolution and log-state inference.

## Commit Style

We use [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `chore:`).
