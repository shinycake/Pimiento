# Security Policy

## Supported versions

Security fixes target the latest `main` branch. Pre-1.0 releases (`0.x`) may receive fixes at maintainer discretion.

## Threat model (short)

Pimiento is a **local desktop frontend** for the user’s already-installed [Oh My Pi (`omp`)](https://omp.sh) binary. It:

- Spawns `omp --mode rpc-ui` as a supervised child
- Decodes NDJSON RPC frames and projects them into the UI
- Sends typed commands back to that child

It does **not** install or reconfigure OMP by default beyond explicit user actions (for example **Update OMP**, or assigning a model role via `omp config`). Auth credentials and session truth live in OMP (`~/.omp`), not in Pimiento.

Trust boundaries that matter:

| Boundary | Expectation |
|----------|-------------|
| Local `omp` child | Treated as a privileged peer: it inherits login-shell environment (including provider API keys) so existing auth works |
| NDJSON from the child | Parsed with size limits and unknown-frame fallbacks; never trusted for arbitrary OS opens without allowlists / user confirmation |
| Host bridge (`PIMIENTO_HOST_BRIDGE=1`) | Opt-in; file opens require per-request approval |

## Reporting a vulnerability

Please **do not** open a public issue for security-sensitive reports.

Prefer a **private GitHub security advisory** on this repository (Security → Advisories → New draft advisory). If that is unavailable, open a private channel with the maintainer via the contact listed on the GitHub profile.

Include:

- Affected commit / version
- Platform (macOS / Linux) and `omp --version`
- Steps to reproduce
- Impact assessment (e.g. local file write, URL scheme abuse, secret leakage into UI)

We will acknowledge reports as soon as practical and coordinate a fix and disclosure timeline.

## Safe disclosure checklist for contributors

Before filing or discussing a report publicly, avoid pasting:

- API keys, OAuth tokens, or full crash stderr that may contain secrets
- Raw session HTML exports
- Contents of `~/.omp` or `~/.pimiento`
