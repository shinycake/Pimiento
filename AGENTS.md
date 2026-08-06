# AGENTS.md — Pimiento

Pimiento is a native GPUI desktop client for the Oh My Pi (`omp`) coding-agent harness on macOS and Linux. It spawns the user's existing `omp` binary in `--mode rpc-ui` and renders decoded NDJSON RPC frames into a semantic streaming workspace — transcript, tool cards, dialogs, abort/steer, sessions — while OMP remains the sole authority over runtime state. The client is a projection + command surface: it never fabricates, infers, or persists agent/session truth. Target milestone sequence is **M0 → M1 → M2 → SH (Self-Host Gate)**, then dogfooded waves D1–D4.

## Sources of truth

- **`PLAN.md`** — the complete implementation plan (v2, self-hosting-first). Read it fully before writing code; re-read the **Doctrine (§0)** whenever in doubt. Doctrine overrides any local decision.
- **`KICKOFF-PROMPT.md`** — environment bootstrap and inner-loop discipline.
- **`docs/protocol-notes.md`** (created during M1) — every wire-shape discovery, OMP quirk, and GPUI API surprise.

## Local-only workflow (hard rule)

This project has **NO remote git, NO GitHub, NO hosted CI, and NO pull requests** until the user has a verified working version.

- `git init` locally; commit early and often on local feature branches.
- "PR" anywhere in `PLAN.md` means "local feature branch merged after `scripts/gate.sh` passes."
- **NEVER** `git push`, add a remote, create a repo on any forge, or reference GitHub Actions / other hosted CI.
- **Gate before merge:** every merge to `main` requires a green `scripts/gate.sh` run on the branch tip.

## Orchestration model — you are the director, not the typist

Your primary-session tokens are the scarcest resource. Preserve them by running as a director:

- Configure the OMP **`task` role to `gpt-5.6-luna` at max thinking effort** in `~/.omp/agent/config.yml` under `modelRoles` (verify the exact provider-qualified id via `omp models` / `/model`). Verify with a trivial `task` dispatch before relying on it.
- **Delegate all substantial code writing to `task` subagents** on that model: implementing a module, writing a test suite, mechanical refactors, fixture recording. Each brief is self-contained — exact file paths, the relevant PLAN.md excerpt or rule, wire shapes involved, expected commands + output, and an explicit output schema (files changed, gate-script result, notes). Subagents inherit zero conversation context.
- **Parallelize where the plan's structure allows it.** The three-crate split exists so `frames.rs` + `decoder.rs`, `supervisor.rs`, and fixture tooling can be built by parallel workers without collision. Fan out, integrate serially.
- **Keep for yourself (do NOT delegate):** reading PLAN.md and making architectural decisions; extracting wire-shape truth from `rpc-types.ts` at the pinned OMP rev (correctness-critical, gates M2); reviewing every subagent diff before merge; running the gate script and the live smoke; writing `docs/protocol-notes.md`; commit/merge decisions; anything touching the doctrine.
- **Review discipline:** treat subagent output as untrusted until the gate script passes and you have read the diff. "Tests pass" is not evidence — run them. Reject and re-dispatch with a sharper brief rather than hand-fix sloppy output.
- **Budget guide:** your turns should be mostly briefs, diffs, decisions, and verification output. If you are streaming whole source files, stop and delegate.

## Cost-ordered feedback loop

Always run the cheapest sufficient check first:

1. `cargo check -p <crate>` after every edit burst (or keep `bacon` running).
2. `cargo clippy --workspace --all-targets -- -D warnings` before every commit — treat every lint as a bug until consciously waived with `#[allow(...)]` + reason comment.
3. `cargo nextest run -p <crate>` for the touched crate; full `cargo nextest run --workspace` before commit.
4. `cargo fmt --all` before commit (the gate script enforces `--check`).
5. Full `cargo build` only when you actually need a binary — the workspace split exists so most work never links GPUI.

`scripts/gate.sh` runs the blocking subset in this exact order and then the nightly canary; it is the sole quality gate for merges to `main`.

## Stable builds; nightly Polonius canary (non-blocking)

- **Edition 2024**, stable toolchain for all builds and gate checks. `#![forbid(unsafe_code)]` in every crate.
- Polonius Alpha is enabled by default on **nightly**, targeting stabilization later in 2026. Policy:
  - **Build on stable.** Never write code that only compiles under Polonius — it would break the stable build. If NLL rejects a sound pattern, restructure (split borrows, `Option::take` take-and-put-back, index instead of reference, scoped `Cell`/`RefCell` in single-threaded UI state) and add a comment `// NLL-workaround: revisit post-Polonius-stabilization` so we can grep and simplify when Polonius stabilizes.
  - The gate script runs `cargo +nightly check --workspace` as a **non-blocking** Polonius canary — its result is reported (`PASS` / `WARN`) but never fails the gate. Opt-out for bisects: `-Zpolonius=off`.
  - When Polonius stabilizes, bump the pinned stable toolchain, delete the workaround comments where the simpler form now compiles, and remove the canary from `gate.sh`.

## The threading law (PLAN §5.3)

- **Background tasks parse and read; only the foreground pump mutates entities.** No `entity.update` from a background executor — ever.
- The event pipeline drains the whole `smol::channel` per foreground wakeup, applies **batched deltas** as one entity update, then emits **one** `cx.notify()`. Coalesce to 30–60 Hz with a timer only if frame time exceeds ~8 ms.
- Single stdin-writer task owns child stdin; command lines never interleave. Dropping a GPUI `Task` cancels it — store reader/pump tasks on the entity and drop on close.
- Async runtime is **smol only** (matches GPUI's executors). No `tokio`. Channels: `smol::channel`; oneshots: `futures::channel::oneshot` or equivalent.

## Unknown wire data always renders (Doctrine §9)

Every enum on the wire has an `Unknown` fallback; unknown frames become visible raw rows (`TranscriptEntry::Unknown { raw: serde_json::Value }`). **Never panic, never drop silently.** This is what lets the app survive OMP upgrades and is load-bearing for the version-gate posture (§1 / §11.3).

## GPL boundary

- `gpui` and `gpui_platform` are Apache-2.0 — fair game to read closely and depend on at the pinned Zed rev.
- `gpui-component` is Apache-2.0 — the pinned hello example is a legitimate reference for correct API use.
- Zed's `ui`, `editor`, `terminal_view`, and `agent_ui` crates are **GPL-3.0-or-later**. **Patterns only, never code.** Do not copy source from those crates under any circumstance. When you need GPUI API truth, read the pinned checkout under `~/.cargo/git/checkouts/...` from the Apache-2.0 crates only.
- OMP itself is MIT; `rpc-types.ts` at the pinned OMP rev is the authoritative source for wire shapes.

## Dogfood ritual — pointer

From SH onward, Pimiento development happens **inside Pimiento** via the worktree split, session-continuity net (`scripts/dogfood.sh` writing `~/.pimiento/dogfood.json`), self-QA convention, and terminal-TUI escalation rule. See **`PLAN.md` §7** for the full ritual; do not paraphrase it here — always follow the plan verbatim.
