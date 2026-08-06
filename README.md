# Pimiento — Handoff Package

Native GPUI desktop client for the Oh My Pi (`omp`) coding-agent harness.
Prepared 2026-08-06. Hand both files to the coding agent together.

## Contents

| File | What it is | How to use it |
|---|---|---|
| `KICKOFF-PROMPT.md` | The agent's first message: environment bootstrap (toolchains, cargo-nextest, insta, bacon, mold/sccache), inner-loop discipline, lint policy, Polonius Alpha borrow-checker policy (stable builds + nightly canary CI), async/error conventions, first-PR deliverables (M0), definition of done. | Paste its contents (below the `---`) as the opening prompt of the coding session, with `PLAN.md` attached or placed in the repo root. |
| `PLAN.md` | The full implementation plan (v2, self-hosting-first): doctrine, using the user's existing omp install, deep feature ranking (Tier 0–4), complete OMP RPC wire contract, core architecture, milestones M0 → M1 → M2 → **SH (Self-Host Gate)** → D1–D5, dogfood ritual, UX spec, edge-case matrix, testing strategy, risks, sources. | The agent's reference document for the entire project. It is self-contained — no other context needed. |

## Suggested opening message to the agent

> Read KICKOFF-PROMPT.md and follow it exactly. The full plan is in PLAN.md — read it
> completely before writing code. This project is LOCAL-ONLY: no remote git, no PRs,
> no hosted CI — local branches + scripts/gate.sh. You orchestrate: dispatch gpt-5.6-luna
> (max thinking) task subagents for code writing per kickoff §0.1. Start with the
> environment bootstrap, then deliver M0. Target sequence: M0 → M1 → M2 → SH.
> Report real command output at every verification step.

## Key decisions already made (don't re-litigate)

- **Local-only workflow:** no remote git, no GitHub, no hosted CI, no PRs until the user has a verified working version. Local `git init`, feature branches merged after `scripts/gate.sh` passes. "PR" anywhere in the docs means "local branch merged after the gate passes."
- **Orchestrated code writing:** the primary agent acts as director and dispatches OMP `task` subagents running **gpt-5.6-luna at max thinking** for all substantial code writing (kickoff §0.1) to preserve primary-session tokens; it keeps architecture, wire-shape extraction, review, gating, and merges for itself.
- Uses the **user's existing omp install** (login-shell PATH discovery, env inheritance, never touches ~/.omp state, no set_model at startup, version gate not version bundling).
- **Self-Host Gate (SH)** is the center of gravity: the app must be able to build itself (agent edits → cargo build → read errors in tool cards → fix → relaunch) before any comfort features. SH-1..SH-6 exit criteria are in PLAN.md §6.
- **Stable Rust builds; Polonius Alpha via non-blocking nightly canary in the local gate script** (enabled on nightly 2026-08-04, stabilizing later this year — NLL-workarounds get greppable comments for post-stabilization cleanup).
- GPUI + gpui_platform pinned to one Zed git rev; gpui-component pinned to a compatible rev; crates.io gpui is NOT used. Zed's ui/editor/agent_ui crates are GPL — patterns only, never code.
- Three-crate workspace, strict dependency direction: omp-rpc-client ← pimiento-core ← pimiento-app. `#![forbid(unsafe_code)]` everywhere. smol only (no tokio).
- The app never owns runtime state — OMP is authoritative; the app is a projection + command surface (PLAN.md Doctrine §0).
