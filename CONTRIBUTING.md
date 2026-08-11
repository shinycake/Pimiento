# Contributing to Pimiento

Thanks for helping improve Pimiento. By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Prerequisites

- Rust stable (see `rust-toolchain.toml` and workspace `rust-version`)
- [cargo-nextest](https://nexte.st/)
- [cargo-audit](https://github.com/RustSec/rustsec/tree/main/cargo-audit) (`cargo install cargo-audit --locked`)
- [omp](https://omp.sh) on your login-shell `PATH` for live runs

## Run the app

```sh
./scripts/run_app.sh
```

See the [README](README.md) for packaging and environment overrides.

## Quality gate

Every change that lands on `main` must pass:

```sh
scripts/gate.sh
```

That runs, in order:

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo nextest run --workspace`
4. `cargo audit`
5. Non-blocking `cargo +nightly check --workspace` (Polonius canary)

Cheaper loops while iterating:

```sh
cargo check -p <crate>
cargo nextest run -p <crate>
```

## Architecture

Read [docs/architecture.md](docs/architecture.md) before changing session, RPC, or projection code. Wire discoveries belong in [docs/protocol-notes.md](docs/protocol-notes.md).

Hard rules:

- OMP owns runtime truth; the UI projects and commands only
- Unknown wire data always renders
- No `unsafe` code
- Do **not** copy source from Zed’s GPL crates (`ui`, `editor`, `terminal_view`, `agent_ui`)

## Session continuity

[`scripts/dogfood.sh`](scripts/dogfood.sh) records `{sessionFile, cwd}` under `~/.pimiento/dogfood.json` so you can resume with `omp --resume` if the GUI cannot launch.

## Security

Report vulnerabilities privately per [SECURITY.md](SECURITY.md). Do not open public issues for security-sensitive reports.
