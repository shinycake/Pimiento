# Pimiento ↔ OMP Parity Plan

**Status:** living plan (v1)  
**Pinned against:** OMP **17.2.11** (live binary + upstream `v17.2.11` RPC sources)  
**Pimiento tested baseline:** `omp ≥ 17.2.10` (version-gate banner for drift)  
**Date:** 2026-08-09  
**Companion docs:** `PLAN.md` (doctrine + original milestones), `docs/protocol-notes.md` (wire truth)

---

## 0. Executive summary

Pimiento has already cleared the original **Self-Host Gate feature surface** and most of **Tier 2–3** from `PLAN.md`: supervised `rpc-ui`, v2 chunking, projection with Unknown tolerance, streaming transcript, tool cards, dialogs, steer/abort/follow-up, multi-session, model/thinking/fast, slash completion, attachments, diffs/revert, subagent inspector, compaction/retry UX, export, login, palette.

What remains for **true daily-driver parity with the OMP TUI** is not another “big rewrite.” It is a set of **honest control surfaces for wire capabilities that already exist**, plus **projection depth for events that currently fall through to Unknown/silent**, plus **operational QA that SH exit criteria still require on the record**.

This plan ranks remaining work by how much it reduces friction in the self-host loop (Doctrine §0 / PLAN Tier ranking), not by feature glitter.

### Verdict in one line

| Layer | State |
|-------|--------|
| Bedrock (RPC + projection + supervision) | **Done** |
| Self-host critical UX (Tier 1) | **Done in code**; SH-1…SH-6 still need recorded proofs |
| Daily-driver (Tier 2) + power (Tier 3) | **Mostly done** |
| True TUI parity (queue modes, auto-compact/retry controls, session ops, host-tool bridge, rich ask, jobs/hub, goals/TTSR, packaging) | **Gaps — this plan** |
| Explicit non-goals | ACP multi-harness, terminal emulator pane, settings mirror of `~/.omp`, bundling omp |

---

## 1. Method & sources

Inventories were cross-checked from:

1. **Live omp 17.2.11** — `omp --mode rpc-ui` probe of `ready`, `get_state`, `get_available_commands` (38 slash cmds), `dumpTools`, `get_available_models`, `get_login_providers`.
2. **Upstream @ v17.2.11** — `docs/rpc.md`, `packages/coding-agent/src/modes/rpc/rpc-types.ts`, `agent-session-events.ts`, slash builtin registry, release notes.
3. **Pimiento tree** — `crates/omp-rpc-client` (typed commands/events), `pimiento-core` (reducer), `pimiento-app` (UI send paths), `docs/protocol-notes.md`, `PLAN.md`.

Doctrine constraints that bound every recommendation:

- OMP remains sole authority; Pimiento is projection + command surface (`PLAN.md` §0.1–0.2).
- Prefer **RPC commands** over reimplementing CLI/TUI features.
- Prefer **slash passthrough** (`prompt` with `/…`) over hardcoding command lists.
- **Never drop Unknown**; never invent session truth.
- **Do not write into `~/.omp` from Pimiento** — use RPC (`login`, `set_model`, `set_todos`, …) or document an intentional exception if `omp config` is invoked as a user-driven escape hatch.

---

## 2. Current parity map (what we already cover)

### 2.1 Transport & bedrock

| Capability | Status |
|------------|--------|
| Discover user `omp` + login-shell env | Done |
| `--mode rpc-ui`, ready + negotiate v2 | Done |
| Chunk reassembly (1 MiB / 64 MiB / 256 KiB) | Done |
| Correlation map, fail-pending-on-death | Done |
| Supervisor: graceful stop, stderr ring, crash-loop breaker, restart/`--resume` | Done |
| Unknown enums + raw transcript rows | Done |

### 2.2 Self-host UX

| Capability | Status |
|------------|--------|
| Streaming transcript + tail-follow + “N new ↓” | Done |
| Tool cards (expand, copy, elapsed, BoundedText) | Done |
| Composer: prompt / steer / follow-up / abort / abort_and_prompt | Done |
| Extension UI: select/confirm/input/editor/open_url | Done (basic) |
| Run-phase machine driving composer affordances | Done |
| Launcher, recent pointers, crash card + Restart | Done |
| Status: model, thinking, ctx%, tok/s, version gate | Done |
| Attachments (images + `@path` mentions + DnD) | Done (**PLAN Tier 4 outdated**) |

### 2.3 Daily-driver + power (post-SH waves)

| Capability | Status |
|------------|--------|
| Slash completion from `available_commands_*` | Done |
| Markdown / code-block copy | Done |
| Model + thinking pickers; cycle | Done |
| Multi-session rail + shortcuts | Done |
| Todo checklist inspector + `set_todos` | Done |
| History hydration `get_messages_page` (+ busy/stale) | Done |
| Fast mode `{enabled, active}` | Done |
| Diff rows + confirmed Revert via `bash` | Done |
| Subagent subscription + inspector tail | Done |
| Compaction / retry banners | Done |
| `export_html` + reveal in file manager | Done |
| Login providers + `open_url` flow | Done |
| Branch → new tab | Done |
| Command palette / theme / keymap polish | Mostly done |
| Host-side git inspector (read-only) | Done (app-side, not OMP) |

### 2.4 Typed on the wire but unused by the app

These exist in `omp-rpc-client::frames::RpcCommandBody` and/or live OMP, but **Pimiento never sends them** (or only renders as Unknown):

| Command / surface | Gap class |
|-------------------|-----------|
| `set_steering_mode` / `set_follow_up_mode` / `set_interrupt_mode` | Control missing (state already in `get_state`) |
| `set_auto_compaction` / `set_auto_retry` / `abort_retry`* | Controls missing (`abort_retry` partially via palette) |
| `get_session_stats` | Stats UI thin vs `/stats`/`/usage` |
| `get_last_assistant_text` | Unused helper |
| `new_session` (wire) | Workspace “New” ≠ wire `new_session` |
| `switch_session` | Intentionally avoided (Doctrine: one child per session) |
| `handoff` | Typed, unused |
| `abort_bash` | No per-bash cancel UI |
| `set_host_tools` / `set_host_uri_schemes` + host_tool/uri reply frames | Bridge UI missing |
| Host tool/URI inbound events | Unknown rows only |

\*Confirm palette coverage for `abort_retry` / `compact` before treating as fully done.

---

## 3. Gap catalog (OMP feature → Pimiento)

Legend: **A** = absent · **P** = partial · **U** = Unknown/silent only · **N** = non-goal / out of scope

### 3.1 Session & queue control (high leverage)

| OMP surface | Live truth | Pimiento | Gap |
|-------------|------------|----------|-----|
| Steer / follow-up / abort | RPC | Done | — |
| `steeringMode` / `followUpMode` / `interruptMode` | In `get_state` (`one-at-a-time` / `immediate` observed) | Display sparse / no writers | **P** — need inspector toggles → `set_*_mode` |
| Queued message count | `queuedMessageCount` | Partial | **P** — show queue depth + clear/steer affordances |
| Wire `new_session` / parentSession | Available | Not used | **P** — optional “fork child session” vs tab-new |
| `handoff` | Available | Unused | **A** — escape hatch to TUI (PLAN Tier 4 adjacent) |
| Session tree / fork / branch UX | TUI rich; RPC has `branch` / `get_branch_messages` | Branch→new tab only | **P** — tree browser, pick entry to branch |
| `switch_session` multiplexing | Available | Deliberately **N** | Keep multi-process model |
| Rename / move / fresh | Slash + `set_session_name` | Rename yes; `/move`/`/fresh` via slash | **P** — first-class “Fresh session” button |

### 3.2 Compaction, retry, context

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| Auto-compaction events | Banner UX | **P** — no `set_auto_compaction` toggle |
| `/compact`, `/shake` | Slash + palette compact | **P** — custom instructions field for `compact` |
| Auto-retry + fallback events | Banners | **P** — `set_auto_retry` + clearer fallback actions |
| `contextUsage` | Shown | **P** — proactive “context high → compact” CTA |
| `get_session_stats` / `/usage` `/stats` | Thin | **A** — stats sheet from RPC |

### 3.3 Models, roles, thinking, fast, service tier

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| Model picker / cycle | Done | — |
| Thinking picker | Done | — |
| Fast mode | Done | — |
| Model roles (`default`/`smol`/`slow`/`task`/`plan`) | Chips; may call `omp config` | **P** — doctrine-safe: prefer documenting `omp config` / slash; avoid silent `~/.omp` writes |
| `--service-tier` / provider tiers | Config keys exist | **A** — show/edit only if exposed on RPC state (else slash/`omp config`) |
| Prewalk / plan-yolo | CLI flags + `/prewalk` | **P** — slash-only OK; optional status chip when active |

### 3.4 Dialogs & approvals (parity depth)

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| Basic confirm/select/input/editor/open_url | Done | — |
| Rich `ask` (`multi`, `recommended`, `preview`, multi-question) | Basic select/confirm | **P** — multi-question cards, recommended default, option previews |
| Approval modes (`always-ask` / `write` / `yolo`) | Via omp config / CLI | **A** — surface current mode if discoverable; else document slash/`omp` only |
| Dialog while tool-expanded / nested UI | Fragile in TUI historically | **P** — QA matrix (SH-2 style) |

### 3.5 Tools, jobs, hub, computer, browser

| OMP surface | Live dumpTools (sample) | Pimiento | Gap |
|-------------|-------------------------|----------|-----|
| Tool cards for any tool name | read/bash/ask/eval/glob/grep/task/hub/todo/web_search/write… | Generic cards | **P** — specialized renderers: `hub` jobs, `eval` cells, `ask` rich, `task` linkage |
| `hub` (background jobs / peers / PTY send) | First-class tool | Opaque tool output | **A/P** — Jobs panel fed by tool args/results + `/jobs` |
| `computer` / `browser` / `vision` | Slash toggles; computer off by default | Slash-only | **P** — status indicators when enabled |
| Per-bash abort | `abort_bash` | Missing | **A** — when bash toolCallId correlation exists |
| MCP / plugins / marketplace | Slash (`/mcp`, `/plugins`, …) | Slash completion only | **P** — inspector “Extensions” section listing mounted tools from `dumpTools` + notices |
| LSP status | Not on rpc-ui state | Explicit “not published” copy | **N** until OMP publishes |

### 3.6 Subagents & collaboration

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| `task` / Agent Hub | Inspector list + message tail | **P** — progress strip in transcript, jump-to-subagent, better lifecycle badges |
| `set_subagent_subscription` levels | Connect sets `events` | **P** — user-selectable off/progress/events |
| Collab / share / join | `/share`, CLI `omp share` | Slash/CLI only | **P** — “Share session” palette → reveal URL/path |
| IRC / collab messages | `irc_message` event | **U** → dedicated row |

### 3.7 Todos, goals, memory, TTSR

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| `todoPhases` + `set_todos` | Checklist inspector | **P** — blocked state, reasons, phase UX matching todo tool ops |
| `todo_reminder` / `todo_auto_clear` | auto_clear handled; reminder **U** | **A** — reminder toast/row |
| `goal_updated` | **U** | **A** — goal chip / row if users use `/goal` |
| `ttsr_triggered` | **U** | **P** — quiet notice (TTSR is OMP-side) |
| Memory / mnemopi / hindsight | Config + slash | Slash-only | **N** for v1 UI (document) |

### 3.8 Auth & providers

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| `get_login_providers` / `login` | Palette flow | **P** — settings “Accounts” list with auth status |
| Auth-broker / gateway | CLI | **N** — power-user docs only |
| Cursor OAuth / tokens | Works via omp | Document dogfood auth path | Doc |

### 3.9 Attachments & media

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| Images on prompt/steer | Done | — |
| Vision / inspect_image | Tool + slash | **P** — ensure image tool results render usefully |
| Audio / live / STT | TUI-native | **N** |

### 3.10 Packaging, platforms, polish

| Item | Gap |
|------|-----|
| Linux AppImage/`.deb` | **A** (macOS unsigned `.app` exists) |
| Windows | **N** (PLAN) |
| IME Enter-while-composing guard | **Blocked** on gpui-component API |
| Cross-message text selection | **N** (copy affordances) |
| Wayland popup QA | Manual QA remaining |
| SH-1…SH-6 recorded proofs | **Operational debt** |

### 3.11 Events still Unknown / silent (projection debt)

Promote from Unknown (or silent) to first-class rows/state when they matter to humans:

| Event | Proposed treatment |
|-------|--------------------|
| `todo_reminder` | Toast + checklist pulse |
| `goal_updated` | Goal strip / notice |
| `ttsr_triggered` | Quiet notice |
| `irc_message` | Collab row |
| `config_update` | Refresh model/thinking display (may already partially) |
| `host_tool_*` / `host_uri_*` | Host-bridge UI (Wave C) |
| Tree/switch internal names if observed | Unknown until shapes confirmed in protocol-notes |

---

## 4. Prioritized delivery waves

Waves are sized for dogfood-friendly branches. Each wave: fixture(s) + `protocol-notes` delta + `scripts/gate.sh` green + manual QA notes.

### Wave 0 — Close the books (ops, not features)

**Goal:** Prove SH, refresh the plan canon, stop lying to ourselves in docs.

1. Record SH-1…SH-6 artifacts (screen/session export) on macOS + Linux.
2. Update `PLAN.md` Tier 4: **images are implemented**; mark D1–D4 code-complete vs QA-open.
3. Bump / document tested omp range for **17.2.11** (plugins, share CLI, catalog fixes) in version-gate copy + protocol-notes.
4. Run `docs/manual-qa-checklist.md` and file IME/Wayland results against the pin.

**Exit:** Checked boxes for SH proofs + PLAN/protocol-notes drift resolved.

---

### Wave A — Queue, compaction, and session controls (highest daily friction)

**Why first:** State is already on the wire (`get_state`); humans currently cannot steer policies without leaving for TUI/`omp config`.

| # | Work item | Wire | UI |
|---|-----------|------|-----|
| A1 | Queue mode toggles | `set_steering_mode`, `set_follow_up_mode`, `set_interrupt_mode` | Inspector “Queue” section; disabled-with-reason on RPC error |
| A2 | Show `queuedMessageCount` | state | Composer/status badge |
| A3 | Auto-compaction / auto-retry toggles | `set_auto_compaction`, `set_auto_retry` | Inspector; keep event banners |
| A4 | Compact with custom instructions | `compact {customInstructions?}` | Palette/dialog |
| A5 | Session stats sheet | `get_session_stats` | Palette “Session stats” |
| A6 | Fresh / handoff affordances | slash `fresh` and/or `handoff` | Palette; handoff confirms “continue in TUI” copy |
| A7 | Context-high CTA | `contextUsage` | Soft banner → opens compact |

**Exit:** All three queue modes + both auto toggles round-trip via RPC; fixture covers mode changes; no `~/.omp` writes.

---

### Wave B — Rich ask, todos/goals, specialized tool cards

**Why:** Self-host stalls and illegibility still hide here.

| # | Work item | Notes |
|---|-----------|-------|
| B1 | Multi-question / multi-select / recommended ask UI | Map `ask` tool → extension UI or richer select cards; keyboard-first preserved |
| B2 | `todo_reminder` + blocked-task UX | Project reminder; surface `block` reasons if present in phases |
| B3 | `goal_updated` strip | Optional; hide when empty |
| B4 | `hub` / jobs card renderer | Parse hub op results into a Jobs list (still OMP-authoritative) |
| B5 | `eval` / `task` card polish | Link task toolCallId → subagent drawer |
| B6 | `abort_bash` when correlatable | Only if toolCallId ↔ bash id is knowable without invention |

**Exit:** Ask with ≥2 questions answerable mid-run (SH-2⁺); reminder visible; one hub job readable without raw JSON.

---

### Wave C — Host tools / URI bridge (true client extensibility)

**Why:** Types exist; without UI Pimiento cannot be a host the way Zed ACP clients can.

| # | Work item |
|---|-----------|
| C1 | Settings: register host tools / URI schemes → `set_host_tools` / `set_host_uri_schemes` |
| C2 | Foreground handlers for `host_tool_call` / `host_uri_request` with progress (`host_tool_update`) and results |
| C3 | Cancel paths (`host_tool_cancel` / `host_uri_cancel`) |
| C4 | Security UX: per-tool permission, never auto-approve dangerous host ops |

**Exit:** One sample host tool (e.g. `pimiento.open_file`) callable from an agent turn end-to-end.

**Note:** This is the largest architectural wave; keep behind a feature flag until SH proofs (Wave 0) are green.

---

### Wave D — Subagents, share, slash-power surfaces

| # | Work item |
|---|-----------|
| D1 | Transcript subagent strip (progress) + subscription level control |
| D2 | Share session (palette → `/share` or CLI reveal) |
| D3 | Extensions inspector: `dumpTools` grouping (builtin / MCP / plugin) |
| D4 | Computer/browser/vision enabled indicators (from state or slash feedback) |
| D5 | Branch/tree browser (list branchable entries → `branch`) |

**Exit:** Two concurrent subagents visible without opening inspector; share path copied once.

---

### Wave E — Packaging & platform finish

| # | Work item |
|---|-----------|
| E1 | Linux AppImage or `.deb` (pick one first) |
| E2 | First-run omp detection polish (already partially present) |
| E3 | Upstream ask: gpui-component `is_composing` API → IME guard |
| E4 | Windows spike (optional; PLAN deferred) |

---

## 5. Explicit non-goals (remain Doctrine 10)

Do **not** schedule unless doctrine changes:

1. Reimplementing OMP settings UI as a full mirror of `~/.omp/agent/config.yml`.
2. Multiplexing multiple OMP sessions over one child via `switch_session`.
3. Bundling / auto-installing omp.
4. Terminal emulator pane / full PTY UI (rpc-ui forces `PI_NO_PTY=1`).
5. ACP multi-harness hosting (keep `pimiento-core` clean for a future crate; build nothing speculative).
6. Speech / live STT / fleet dashboards / remoting.
7. Copying GPL Zed agent UI code.

---

## 6. Version & wire hygiene

| Practice | Detail |
|----------|--------|
| Pin for extraction | Re-extract `rpc-types.ts` + `agent-session-events.ts` at each newly supported omp minor |
| Fixtures | One NDJSON fixture per wave feature path |
| protocol-notes | Required for every newly projected event or newly sent command |
| Version gate | Keep banner; widen tested range only after Wave 0 QA on that version |
| 17.2.11 watchouts | Agent Plugins 1.0; `omp share`; Cursor quota/auth fixes; no RPC schema break vs 17.2.10 |

Live slash set (38) is **dynamic** — never hardcode; always prefer `available_commands_update`.

---

## 7. Suggested sequencing vs original PLAN milestones

```
Wave 0  →  close SH proofs + doc drift          (ops)
Wave A  →  queue/compact/stats controls         (maps to “Tier 2 leftovers”)
Wave B  →  ask/todos/tool render depth          (feel / self-host reliability)
Wave D  →  subagents/share/extensions           (can parallelize after A)
Wave C  →  host bridge                          (largest; after A+B stable)
Wave E  →  packaging                            (D5 / PLAN packaging)
```

Parallelism allowed by crate split:

- `omp-rpc-client` + fixtures: new command send helpers / event kinds
- `pimiento-core`: projection for reminders/goals/hub summaries
- `pimiento-app`: inspector toggles / specialized cards

Integrate serially; gate with `scripts/gate.sh`.

---

## 8. Acceptance criteria (plan-level)

The parity program is “done enough” when:

1. **Wave 0** SH proofs exist for macOS + Linux.
2. **Wave A** users can change queue + auto-compact/retry without leaving Pimiento.
3. **Wave B** multi-question `ask` never stalls a run; todo reminders are visible.
4. Unknown rate for common sessions drops: `todo_reminder`, `goal_updated`, `ttsr_triggered`, `irc_message` no longer surprise humans as raw dumps (either projected or consciously quieted with a notice).
5. Host bridge (**Wave C**) is either shipped behind a flag or explicitly deferred with rationale in this doc.
6. Linux packaging (**Wave E1**) exists or is explicitly waived for personal dogfood.

---

## 9. Risks

| Risk | Mitigation |
|------|------------|
| OMP weekly churn | Unknown rows + version gate + fixture re-record |
| Doctrine violation via `omp config` / role chips | Prefer RPC; if config CLI used, only on explicit user action; never silent |
| Rich ask shapes drift | Extract from live `ask` tool schema + rpc-ui captures; don’t guess |
| Hub/jobs UI invents state | Parse tool results only; re-fetch via slash/`hub` ops when unsure |
| Host tools security | Default deny; confirm cards; no yolo registration |
| Scope explosion into TUI clone | Enforce non-goals; slash passthrough first |

---

## 10. Appendix — live probe snapshot (2026-08-09)

**ready:** `protocolVersion:1`, supports `[1,2]`, maxFrame 1 MiB, reassembly 64 MiB  

**get_state keys observed:**  
`autoCompactionEnabled`, `contextUsage`, `dumpTools`, `fastModeActive`, `fastModeEnabled`, `followUpMode`, `interruptMode`, `isCompacting`, `isStreaming`, `messageCount`, `model`, `queuedMessageCount`, `sessionId`, `steeringMode`, `systemPrompt`, `todoPhases`, `tokensPerSecond`  

**Queue defaults observed:** steering/follow-up `one-at-a-time`, interrupt `immediate`, autoCompaction `true`  

**Slash (38):**  
`add-dir`, `advisor`, `autoresearch`, `browser`, `changelog`, `compact`, `computer`, `context`, `dirs`, `dump`, `export`, `fast`, `force`, `fresh`, `green`, `init`, `jobs`, `marketplace`, `mcp`, `memory`, `model`, `move`, `plugins`, `prewalk`, `reload-plugins`, `remove-dir`, `rename`, `review`, `security`, `session`, `shake`, `share`, `ssh`, `stats`, `todo`, `tools`, `usage`, `vision`

**dumpTools (this env):**  
`read`, `bash`, `ask`, `eval`, `glob`, `grep`, `task`, `hub`, `todo`, `web_search`, `write`

---

## 11. Next concrete action

Start **Wave 0** (SH proof recording + PLAN/protocol-notes drift fix), then open the first implementation branch for **Wave A1–A3** (queue + auto toggles) — highest parity per line of code because the RPC and state fields already exist.
