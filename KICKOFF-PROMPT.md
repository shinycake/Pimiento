# Pimiento — Agent Kickoff Prompt (M0/M1 start)

> Paste everything below the line into the coding agent's first message, alongside the plan file
> `2026-08-05_061500-omp-gpui-client-v2-selfhost.md`. Both files travel together.

---

You are starting work on **Pimiento**, a native GPUI desktop client for the Oh My Pi (`omp`) coding-agent harness. The complete implementation plan is in the accompanying file `PLAN.md` (a.k.a. `2026-08-05_061500-omp-gpui-client-v2-selfhost.md`). Read it fully before writing any code. This message sets up your environment, tooling, working conventions, and first deliverables.

## 0. Prime directives

1. The plan's **Doctrine section (§0)** overrides any decision you'd otherwise make. Re-read it when in doubt.
2. Your target milestone sequence is **M0 → M1 → M2 → SH** (the Self-Host Gate). Do not build Tier-2+ features early, no matter how tempting — SH is the center of gravity.
3. Never fabricate protocol behavior. When a wire shape is ambiguous, extract truth from the pinned OMP source (`rpc-types.ts`) or record it from a live `omp --mode rpc-ui` session, and log the finding in `docs/protocol-notes.md`.
4. Verify by running. "It should work" is not a status; paste real command output.
5. **You are the orchestrator, not the typist.** Delegate the actual code writing to subagents per §0.1 to preserve your own context/tokens for planning, review, and integration.

## 0.1 Orchestration model — delegate code writing to GPT 5.6 Luna (max thinking)

Your primary-session tokens are the scarcest resource on this project. Preserve them by running as a director:

- **Configure the `task` role to use `gpt-5.6-luna` at max thinking effort** before starting work. Set it in `~/.omp/agent/config.yml` under `modelRoles` (e.g. `task: <provider>/gpt-5.6-luna` with thinking level `max` — check `omp models` / `/model` for the exact provider-qualified id available in this install), or assign it via the `/model` role picker. Verify with a trivial `task` dispatch before relying on it.
- **All substantial code writing goes through `task` subagents** running that model: implementing a module, writing a test suite, mechanical refactors, fixture recording harnesses. Each dispatch must be self-contained: exact file paths, the relevant PLAN.md excerpt or rule, the wire shapes involved, expected commands + expected output, and an explicit output schema (files changed, gate-script result, notes). Subagents inherit no conversation context — write briefs accordingly.
- **Parallelize where the plan's structure allows it:** the three-crate split exists precisely so `frames.rs`+`decoder.rs`, `supervisor.rs`, and fixture tooling can be built by parallel workers in isolated worktrees without collisions. Fan out with workspace isolation; integrate serially yourself.
- **You keep for yourself (do NOT delegate):** reading PLAN.md and making architectural decisions; extracting wire-shape truth from `rpc-types.ts` (correctness-critical, gates M2); reviewing every subagent diff before merge; running the gate script and the live smoke test; writing `docs/protocol-notes.md`; commit/merge decisions; anything touching the doctrine.
- **Review discipline:** treat subagent output as untrusted until the gate script passes and you have read the diff. A subagent claiming "tests pass" is not evidence — run them. Reject and re-dispatch with a sharper brief rather than hand-fixing large sloppy output (hand-fixing burns exactly the tokens delegation was meant to save; small touch-ups are fine).
- **Budget guide:** your session should be mostly briefs, diffs, decisions, and verification output — if you find yourself streaming whole source files, stop and dispatch instead.

## 1. Environment bootstrap (do this first, in order)

```bash
# 1) Toolchain — STABLE is the build toolchain; pin it.
rustup toolchain install stable
rustup component add clippy rustfmt rust-analyzer

# Also install nightly — NOT for building the product; it is the Polonius canary + rustfmt niceties (see §3).
rustup toolchain install nightly

# 2) Fast iteration tooling
cargo install cargo-nextest --locked   # test runner (3x faster, per-test isolation, better output)
cargo install cargo-insta --locked     # snapshot review workflow for projection tests
cargo install bacon --locked           # background cargo-check watcher (use between edits)

# 3) Fast linker — biggest single win for GPUI's huge dep graph
#   Linux: install mold (apt install mold / dnf install mold), then in .cargo/config.toml:
#     [target.x86_64-unknown-linux-gnu]
#     linker = "clang"
#     rustflags = ["-C", "link-arg=-fuse-ld=mold"]
#   macOS: the new default ld-prime is already fast; optionally `-C link-arg=-fuse-ld=lld`.

# 4) Optional but recommended for the GPUI/Zed git-dep graph (300+ crates):
cargo install sccache --locked         # then: export RUSTC_WRAPPER=sccache

# 5) Verify omp availability (Pimiento drives the user's existing install):
command -v omp && omp --version
```

Create `rust-toolchain.toml` pinning stable, and commit `.cargo/config.toml` with the linker config guarded per-platform.

## 2. Inner-loop discipline (how you work, every task)

Cost-ordered feedback loop — always run the cheapest sufficient check first:

1. **`cargo check -p <crate>`** after every edit burst (or keep `bacon` running). Seconds, catches most errors.
2. **`cargo clippy --workspace --all-targets -- -D warnings`** before every commit. Clippy is your senior reviewer: treat every lint as a bug until consciously waived with a written `#[allow(...)]` + reason comment.
3. **`cargo nextest run -p <crate>`** for the crate you touched; full `cargo nextest run --workspace` before commit.
4. **`cargo fmt --all`** before commit (the local gate script enforces `--check`).
5. Full `cargo build` only when you actually need a binary (GPUI links are expensive; don't rebuild the app to validate a protocol-crate change — the workspace split in the plan exists precisely so most work never links GPUI).

Additional inner-loop rules:

- **Snapshot tests:** projection tests use `insta`. When a snapshot changes, run `cargo insta review` and *read the diff*; accept only diffs you can explain. Never blind-accept with `--accept-all`.
- **TDD where it pays:** the protocol decoder, chunk reassembler, and projection reducer are pure functions — write the failing test first there. UI code is exempt from strict TDD but not from the manual verification script in each milestone.
- **Commits:** small, one logical change, imperative message (`feat: v2 chunk reassembler with property tests`). Commit after each green task, not at end of day.
- **When stuck on the borrow checker >20 min:** prefer restructuring (split borrows, take-and-put-back with `Option::take`, index instead of reference, small `Cell`/`RefCell` in single-threaded UI state) over `clone()` sprinkling or `unsafe`. `unsafe` is forbidden in this codebase (`#![forbid(unsafe_code)]` in all three crates) — the FFI-heavy parts live inside GPUI, not our code.

## 3. Rust edition, lints, and the new borrow checker

- **Edition 2024**, stable toolchain for all builds and gate checks.
- Workspace lints in root `Cargo.toml`:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
missing_debug_implementations = "warn"

[workspace.lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"        # use expect("why") or proper error paths; unwrap only in tests
dbg_macro = "deny"
todo = "warn"
```

  (`expect_used` stays allowed — a crash with a reason string beats a silent lie, per Doctrine 6.)

- **Polonius Alpha (the new borrow checker):** as of the 2026-08-04 Rust blog post, Polonius Alpha is enabled **by default on nightly**, targeting stabilization later this year. It is flow-sensitive and accepts strictly more sound code (e.g. the classic `get_mut_or_default` HashMap pattern). Policy for this project:
  - **Build on stable. Do not write code that only compiles under Polonius** — it would break the stable build. If NLL rejects a sound pattern, restructure (see §2) and add a `// NLL-workaround: revisit post-Polonius-stabilization` comment so we can simplify later; grep for these when Polonius lands in stable.
  - **Add a nightly Polonius canary to the local gate script** (`cargo +nightly check --workspace`, non-blocking: report its result but never fail the gate on it). This gets us Polonius coverage for free and surfaces future diagnostics/perf differences early. Opt-out flag, if ever needed for a bisect: `-Zpolonius=off`.
  - When Polonius stabilizes (expected end of 2026), bump the pinned stable toolchain, remove the workaround comments where the simpler form now compiles, and delete the canary job.

## 4. Error-handling and async conventions

- Libraries (`omp-rpc-client`, `pimiento-core`): `thiserror` enums with meaningful variants (`ChildDied { exit_code, stderr_tail }`, `ProtocolViolation { detail }`, `FrameTooLarge`…). Never `anyhow` in library public APIs.
- Binary (`pimiento-app`): `anyhow` at the edges is fine.
- Async: **smol only** (matches GPUI's executors). No tokio — one runtime in the process. Channels: `smol::channel`; oneshots: `futures::channel::oneshot` or equivalent. Respect the plan's §5.3 threading law: background tasks parse/read, only the foreground pump mutates entities.
- Process I/O: `smol::process::Command` (or `async-process`) with piped stdio; single writer task owns child stdin.

## 5. Using your own toolbox well (you are running inside omp)

- Use your **LSP tool** (rust-analyzer) aggressively: `lsp` diagnostics after edits are cheaper than `cargo check`; use references/rename for refactors instead of regex edits; hover to confirm inferred types at tricky borrow sites.
- Use **hashline edits** as normal; for wide mechanical renames prefer LSP rename.
- Long builds: run `cargo build`/`nextest` via bash tool and **read the full output** — do not truncate-and-guess. First GPUI build will take many minutes; that's expected, not a hang (don't kill it before ~20 min).
- Keep a **scratch session log** in `docs/protocol-notes.md` of every wire-shape discovery, OMP quirk, and GPUI API surprise. Future-you (and the SH-gate dogfood sessions) depend on it.
- When you need GPUI API truth, read the **pinned** Zed source under `~/.cargo/git/checkouts/...` (or fetch the specific file at the pinned rev) — never main-branch docs, they drift. Same for gpui-component. Reminder: Zed's `ui`/`editor`/`agent_ui` crates are **GPL — patterns only, never copy code**; `gpui`/`gpui_platform` are Apache-2.0 and fair game to read closely.

## 6. Project scaffolding deliverables (your first milestone — plan M0)

**Local-only workflow (hard requirement until further notice):** this project has NO remote git, NO GitHub, NO hosted CI, and NO pull requests until the user has a verified working version. `git init` locally, commit early and often, and use local branches + merges where the plan says "PR" (read "PR" anywhere in PLAN.md as "local feature branch merged after the gate script passes"). Never run `git push`, never add a remote, never create a repo on any forge, never reference GitHub Actions. All quality gates run locally via the gate script below.

1. Workspace per plan §3 (three crates, strict dependency direction), `rust-toolchain.toml`, `.cargo/config.toml` (fast linker), workspace lints (§3 above), `Cargo.lock` committed.
2. Dependency pins per plan §5.1 — resolve `<PIN>`/`<PIN2>` by inspecting gpui-component's current lockfile for its Zed rev, verify their gallery example builds and runs, then freeze.
3. **`AGENTS.md` at repo root** (you are writing the brief for your future selves — omp reads it automatically). Contents: one-paragraph project summary; pointer to PLAN.md and doctrine; the local-only rule (no remotes, no push, gate script before merge); the orchestration model from §0.1 (task role = gpt-5.6-luna max thinking; delegate code writing, keep review/gating); build/test/lint commands from §2; the stable-vs-Polonius policy from §3; the threading law (background parses, foreground mutates, batch deltas); the "unknown wire data always renders" rule; the GPL boundary; the dogfood ritual pointer (plan §7).
4. **Local gate script instead of CI:** `scripts/gate.sh` — runs, in order: `cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo nextest run --workspace` · nightly Polonius canary (`cargo +nightly check --workspace`, non-blocking, result reported). The script exits non-zero if any blocking step fails. Run it before every merge to the main branch; optionally wire it as a `.git/hooks/pre-commit` or pre-merge convention. Cache-friendly: respect sccache if `RUSTC_WRAPPER` is set.
5. Hello-window smoke: themed gpui-component window with a button and dark/light toggle, launching on macOS and Linux.

Then proceed directly into M1 (`omp-rpc-client`) per the plan, starting with `discovery.rs` and extracting the exact `assistantMessageEvent` / `tool_execution_*` payload shapes from the installed OMP version's `rpc-types.ts` — that extraction gates M2, so front-load it.

## 7. Definition of done (every task, no exceptions)

- `cargo fmt --check` ✅ · `clippy -D warnings` ✅ · `nextest run --workspace` ✅
- New behavior has a test (protocol/projection) or a written manual-verification note (UI)
- Any wire discovery logged in `docs/protocol-notes.md`
- Commit message explains *why*, not just what
- You ran it. Paste the evidence.
