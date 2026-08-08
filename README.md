# Pimiento

A native GPUI desktop client for the Oh My Pi (`omp`) coding-agent harness.

## Requirements

- Rust stable
- `omp` installed and available on your login-shell `PATH`
- macOS or Linux

Pimiento uses your existing `omp` installation, configuration, and session store.

## Quick start

```sh
cargo build -p pimiento-app
./scripts/run_app.sh
```

Set `PIMIENTO_AUTO_CONNECT=1` when launching to automatically connect (also the default in `scripts/run_app.sh`):

```sh
PIMIENTO_AUTO_CONNECT=1 ./scripts/run_app.sh
```

Palette **Toggle theme** choices persist in `ui.json`. For a process-only dogfood/QA override that leaves the stored preference unchanged:

```sh
PIMIENTO_THEME=light ./scripts/run_app.sh
```

## Daily loop

Run the local quality gate before promoting work:

```sh
scripts/gate.sh
```

For the self-hosting workflow, follow the dogfood ritual in [PLAN.md §7](PLAN.md#7-the-dogfood-ritual-how-the-app-builds-itself). `scripts/dogfood.sh` starts and records a development session; `scripts/reveal_logs.sh` opens Pimiento's local logs.

## macOS packaging

See [docs/packaging.md](docs/packaging.md) for the unsigned local `.app` bundle. Use `scripts/package_macos_app.sh` to build it, or `scripts/install_macos_app.sh` to install it for the current user (`~/Applications/Pimiento.app`). Linux AppImage/`.deb` packaging is deferred — run via `cargo build` / `scripts/run_app.sh` for now (same doc).

## Architecture and doctrine

The workspace is split into the protocol client, a UI-free projection core, and the GPUI app. OMP is the sole authority for agent and session state. Pimiento is a projection and command surface: it renders decoded RPC frames and sends explicit commands, without fabricating or persisting runtime truth.

## Agent handoff

For implementation guidance, start with [KICKOFF-PROMPT.md](KICKOFF-PROMPT.md) and then read [PLAN.md](PLAN.md) in full.
