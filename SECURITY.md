# Security Policy

## Reporting a Vulnerability

Because `rakku` manages Linux users, `systemd` services, and CI runner tokens, we take security seriously.

If you discover a security vulnerability, **please do not open a public issue**. Instead, report it privately via GitHub Security Advisories:

- https://github.com/myzskdev/rakku/security/advisories/new

We will acknowledge your report and work on a fix promptly.

## Security Design Principles

- Runners are never executed as `root` — `rakku` creates a dedicated non-root Linux user (`rakku` by default).
- Runner binaries/tarballs are verified with SHA-256 checksums when published by upstream releases.
- PATs and registration tokens are never stored in `state.json` or `systemd` unit files.
