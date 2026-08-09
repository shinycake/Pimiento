# Pimiento ↔ OMP Parity Plan

**Status:** living plan (v2 — adds T4-inspired visual/UX system)  
**Pinned against:** OMP **17.2.11** (live binary + upstream `v17.2.11` RPC sources)  
**UI reference:** [T4 Code](https://github.com/LycaonLLC/t4-code) `DESIGN.md` + lane-b screenshots (patterns only; **not** their brand pink / Electron stack)  
**Pimiento tested baseline:** `omp ≥ 17.2.10` (version-gate banner for drift)  
**Date:** 2026-08-09  
**Companion docs:** `PLAN.md` (doctrine + original milestones), `docs/protocol-notes.md` (wire truth), `.cursor/skills/pimiento-ui-craft/SKILL.md`

---

## 0. Executive summary

Pimiento has already cleared the original **Self-Host Gate feature surface** and most of **Tier 2–3** from `PLAN.md`: supervised `rpc-ui`, v2 chunking, projection with Unknown tolerance, streaming transcript, tool cards, dialogs, steer/abort/follow-up, multi-session, model/thinking/fast, slash completion, attachments, diffs/revert, subagent inspector, compaction/retry UX, export, login, palette.

What remains has two equal tracks:

1. **Protocol / control parity** — honest UI for wire capabilities that already exist (queue modes, auto-compact/retry, rich ask, hub/jobs, host bridge, …) plus SH proof recordings.
2. **Quiet-console visual parity** — take **heavy inspiration** from T4 Code’s layout, depth, status taxonomy, and composer UX, reinterpreted as a **GPUI-native** system with a **distinct Pimiento palette** (never clone Pi Pink / their Electron chrome).

This plan ranks work by self-host-loop friction first, then by how much a calm control-room visual system reduces scan cost.

### Verdict in one line

| Layer | State |
|-------|--------|
| Bedrock (RPC + projection + supervision) | **Done** |
| Self-host critical UX (Tier 1) | **Done in code**; SH-1…SH-6 still need recorded proofs |
| Daily-driver (Tier 2) + power (Tier 3) | **Mostly done** |
| True TUI / RPC control parity | **Gaps — Waves A–E** |
| Visual system (T4-inspired, GPUI-native) | **Gaps — Wave U** (below) |
| Explicit non-goals | ACP multi-harness, terminal emulator pane, settings mirror of `~/.omp`, bundling omp, cloning T4 brand/colors/Electron |

---

## 1. Method & sources

Inventories were cross-checked from:

1. **Live omp 17.2.11** — `omp --mode rpc-ui` probe of `ready`, `get_state`, `get_available_commands` (38 slash cmds), `dumpTools`, `get_available_models`, `get_login_providers`.
2. **Upstream @ v17.2.11** — `docs/rpc.md`, `packages/coding-agent/src/modes/rpc/rpc-types.ts`, `agent-session-events.ts`, slash builtin registry, release notes.
3. **Pimiento tree** — `crates/omp-rpc-client` (typed commands/events), `pimiento-core` (reducer), `pimiento-app` (UI send paths), `docs/protocol-notes.md`, `PLAN.md`.
4. **T4 Code (UI reference only)** — `DESIGN.md` (“Quiet Control Room”), lane-b screenshots (`01-light-stream` … `09-light-tool-expanded`), `FEATURE_MATRIX.md` surface map. Steal **structure and discipline**, not brand pink, Electron, remoting, or dashboard theater.

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

## 3. Visual & interaction system — T4-inspired, GPUI-native

> **North star:** *Quiet Pepper Console* — a calm observability + control surface for a running OMP session. Dense when the work demands it; quiet when it does not. Transcript is the hero. Chrome reports state; it never decorates.

T4 Code ([LycaonLLC/t4-code](https://github.com/LycaonLLC/t4-code)) is the shipped Electron OMP client and the best existing **interaction reference** (`DESIGN.md`, lane-b screenshots, `FEATURE_MATRIX.md`). Pimiento must feel as intentional, but remain:

- **GPUI + gpui-component** (Apache-2.0 pins only — patterns from Zed GPL crates, never code)
- **OMP-authoritative** (no invented session truth)
- **Visually distinct** from T4 (different brand hue; no Pi Pink identity lockup)

Canonical craft rules also live in `.cursor/skills/pimiento-ui-craft/SKILL.md`; this section is the product-level expansion for the parity program.

### 3.1 Steal these T4 rules (behavior, not paint)

| T4 rule | Pimiento adaptation |
|---------|---------------------|
| Zero-chroma neutral chassis; hue only with meaning | Same — map to gpui-component theme tokens + a small Pimiento token overlay |
| Two-tier brand: identity vs AA-safe action | Same *structure*, **different hue family** (see §3.2) |
| Brand accent never doubles as warning | Amber = warning; brand reserved for identity + primary actions |
| Fixed status taxonomy (working / approval / input / plan / done / error) with **dot + label** | Map onto `RunPhase` + dialog pending + rail attention; never color-only |
| Hairline depth at rest; real shadows only for floating tier | Docked = tone + 1px border (+ optional 1px bevel); floating = `popover` + `shadow_lg`/`xl` |
| Right pane closed by default; earn open with badges | Keep inspector collapsed; badge on Checklist/Agents only when OMP data nonempty |
| Composer owns model / thinking / fast / steer\|queue | Align with craft skill: composer band, not status-strip duplication |
| 4px grid, soft radius ceiling (~10–16px), no decorative left stripes | 8pt grid already; **ban** `border_l_2` accent bars on selected rows |
| Motion is vocabulary; reduced-motion path | Prefer gpui-component transitions; no continuous glow — at most **one** live status pulse |
| Transcript measure capped (~48rem) | Constrain prose column; tool/code full width of transcript pane |
| Reject AI-dashboard theater | Already doctrine — reinforce in Wave U QA |

### 3.2 Pimiento color system (inspired *usage*, not T4 colors)

Do **not** reuse T4’s `#e83174` Pi Pink or their raspberry action tier. Avoid purple/glow stacks and the common cream+terracotta AI landing-page look.

**Chassis (zero chroma)**

| Token role | Light | Dark | Usage |
|------------|-------|------|-------|
| Background | near-white `#FAFAFA` / white cards | near-black mixed slightly toward white | App, transcript |
| Foreground | graphite ~oklch(0.27 0 0) | ~oklch(0.97 0 0) | Primary text |
| Muted | ~oklch(0.55 0 0) | ~oklch(0.71 0 0) | Meta, timestamps |
| Wash / secondary | black @ 4% | white @ 4% | Hover, selected rail wash, muted panels |
| Hairline border | black @ 8% | white @ 6% | All docked dividers |
| Field stroke | black @ 10% | white @ 8% | Inputs, outline buttons |

**Brand (two-tier — paprika/ember family)**

| Tier | Role | Allowed | Banned |
|------|------|---------|--------|
| **Identity paprika** ≈ `#C45C26` | Mark, selected ticks, non-text brand | Logo moments | Body copy in light; washes; warnings |
| **Action ember** (darker AA-safe coral-ember) | Filled Send/Steer/Approve, focus ring | Primary CTAs | Status taxonomy; diffs; destructive |

Implement via one module (`PimientoTokens` / theme extension) that owns raw colors; everywhere else consumes tokens (T4’s “One Token File” rule).

**Semantic status (same jobs as T4, independent hex)**

| Status | Maps from | Family |
|--------|-----------|--------|
| Working / Streaming / Connecting | `RunPhase::Streaming`, reconnecting | Sky / info |
| Approval | blocking confirm/select | Amber |
| Awaiting input | input/editor/ask | Indigo (not paprika) |
| Plan ready | plan artifact if exposed | Violet **as status only** |
| Done / idle-success | terminal success → idle | Emerald |
| Error / Dead | errors, crash card | Crimson |

**Diff:** added = success @ ~12% alpha; removed = destructive @ ~12%. **Mono:** one stack for tool output + fences.

### 3.3 Layout anatomy (target)

```
┌─ topbar ~52px ─────────────────────────────────────────────────────────┐
│ brand · workspace   │  phase pill · omp ver   │  palette · theme · pane │
├──────────┬──────────────────────────────────┬──────────────────────────┤
│ session  │  transcript (hero)               │  context inspector       │
│ rail     │  prose measure ≤ ~48rem          │  closed by default       │
│ 208–256  │  tools/code use full pane width  │  240–280 when open       │
│          ├──────────────────────────────────┤                          │
│          │  composer band (elevated island) │                          │
│          │  model · think · fast · attach   │                          │
│          │  ctx ring   [Queue]  [Steer/Send]│                          │
└──────────┴──────────────────────────────────┴──────────────────────────┘
```

**Today → target**

| Today (post-D4 screenshot) | Target |
|----------------------------|--------|
| Busy chrome duplicating model/think/ctx | Thin topbar; model/think/fast **only** on composer |
| Flat composer, equal-weight chips | Elevated control island; primary Steer/Send in **action-ember** |
| Weak phase tag | Status **pill = 6px dot + sentence-case label** |
| Always-on full tool cards | **Action groups**: collapsed step rows + chevron |
| Plain dialogs | Numbered options, recommended default, “Press 1–9”, free-text fallback |
| Context % as text | Compact **ring meter** from `contextUsage.percent` |
| Selected rail ambiguity | Wash + `rounded_sm` — **no** colored left stripe |

### 3.4 Depth recipe (GPUI mapping)

| Surface | Recipe |
|---------|--------|
| App / transcript | `background` |
| Rail / inspector | `sidebar` + `sidebar_border`; **no shadow** |
| Tool inset / action group | `secondary` + 1px `border` + optional top-edge 1px bevel wash |
| Composer band | Docked for list stability, but elevated fill + top hairline + **`shadow_md`/`lg` only here** |
| Palette / model picker / About | `overlay` + `popover` + `shadow_xl` + `rounded_lg` |
| Primary button | Action-ember fill + light inset bevel if cheap; hover dims ~90% |
| Status pill | Dot + label; ping **only** on Working |

**Hard bans:** purple glow, neon rings, multi-layer shadows on docked chrome, glass monoculture, gradient text, grain, radius >16px, decorative `border_l` stripes, metric dashboards.

### 3.5 Transcript, composer, rail (detail)

**Transcript**

- Keep semantic rows (PLAN §8).
- Consecutive tools in one turn → rounded **action group**; expand in place with careful `ListState` splice.
- Summary line: icon · tool · ≤80-char digest · status chip · duration.
- Bash: command + exit + duration; ~24-line output viewport; BoundedText elision.
- Unknown: collapsed raw JSON.
- Streaming: muted ellipsis only — no fake skeleton copy.

**Composer**

- Streaming placeholder: “Steer the running turn, or queue a follow-up.”
- Mid-stream: primary **Steer** (ember), outline **Queue**; idle **Send**.
- Controls: model popover, filtered thinking, Fast switch, Attach, ctx ring, Abort (danger ghost when abortable).
- Blocking dialog between transcript and composer; dims composer; keyboard-first.

**Rail / inspector**

- Group by cwd; row = title · muted model · relative time · status pill.
- Roll up highest-priority child status to workspace header (error > approval > input > working > idle).
- Inspector: Session · Checklist · Agents · Tools · Git · **Queue modes** (Wave A).
- Auto-open inspector only for approval/plan attention — never every tool.

### 3.6 Motion & a11y

- Hover ≤120ms; expand/popover ≤190ms ease-out; dialog ≤240ms.
- Suppress transitions on theme flip / rail drag.
- Reduced motion: no ping, no animated smooth scroll.
- Status never color-only; focus ring = action-ember ≥3:1 non-text.
- Light + dark both first-class.

### 3.7 Explicitly **not** taken from T4

Multi-host / Tailnet / daemon · offline JSONL-as-truth · terminal/PTY pane · browser workspace pane · Electron titlebar · copying Pi/plugin mark geometry · fleet attention inbox. (Doctrine 10 + local-child model.)

### 3.8 UI-specific gap table

| Area | Severity | Wave |
|------|----------|------|
| Tokenized paprika/ember theme + status pills | High | **U1** |
| Composer elevation + Steer/Queue hierarchy | High | **U2** |
| Dedupe strip/composer/inspector labels | High | **U2** |
| Context ring meter | Medium | **U2** |
| Action-group tool collapsing | High | **U3** |
| Rich ask/approval card chrome | High | **U3** (+ B1) |
| Rail status pills + priority rollup | Medium | **U4** |
| Transcript measure + denser 8pt rhythm | Medium | **U4** |
| Motion / reduced-motion + L/D QA | Required | **U5** |

---

## 4. Gap catalog (OMP feature → Pimiento)

Legend: **A** = absent · **P** = partial · **U** = Unknown/silent only · **N** = non-goal / out of scope

### 4.1 Session & queue control (high leverage)

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

### 4.2 Compaction, retry, context

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| Auto-compaction events | Banner UX | **P** — no `set_auto_compaction` toggle |
| `/compact`, `/shake` | Slash + palette compact | **P** — custom instructions field for `compact` |
| Auto-retry + fallback events | Banners | **P** — `set_auto_retry` + clearer fallback actions |
| `contextUsage` | Shown | **P** — proactive “context high → compact” CTA |
| `get_session_stats` / `/usage` `/stats` | Thin | **A** — stats sheet from RPC |

### 4.3 Models, roles, thinking, fast, service tier

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| Model picker / cycle | Done | — |
| Thinking picker | Done | — |
| Fast mode | Done | — |
| Model roles (`default`/`smol`/`slow`/`task`/`plan`) | Chips; may call `omp config` | **P** — doctrine-safe: prefer documenting `omp config` / slash; avoid silent `~/.omp` writes |
| `--service-tier` / provider tiers | Config keys exist | **A** — show/edit only if exposed on RPC state (else slash/`omp config`) |
| Prewalk / plan-yolo | CLI flags + `/prewalk` | **P** — slash-only OK; optional status chip when active |

### 4.4 Dialogs & approvals (parity depth)

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| Basic confirm/select/input/editor/open_url | Done | — |
| Rich `ask` (`multi`, `recommended`, `preview`, multi-question) | Basic select/confirm | **P** — multi-question cards, recommended default, option previews |
| Approval modes (`always-ask` / `write` / `yolo`) | Via omp config / CLI | **A** — surface current mode if discoverable; else document slash/`omp` only |
| Dialog while tool-expanded / nested UI | Fragile in TUI historically | **P** — QA matrix (SH-2 style) |

### 4.5 Tools, jobs, hub, computer, browser

| OMP surface | Live dumpTools (sample) | Pimiento | Gap |
|-------------|-------------------------|----------|-----|
| Tool cards for any tool name | read/bash/ask/eval/glob/grep/task/hub/todo/web_search/write… | Generic cards | **P** — specialized renderers: `hub` jobs, `eval` cells, `ask` rich, `task` linkage |
| `hub` (background jobs / peers / PTY send) | First-class tool | Opaque tool output | **A/P** — Jobs panel fed by tool args/results + `/jobs` |
| `computer` / `browser` / `vision` | Slash toggles; computer off by default | Slash-only | **P** — status indicators when enabled |
| Per-bash abort | `abort_bash` | Missing | **A** — when bash toolCallId correlation exists |
| MCP / plugins / marketplace | Slash (`/mcp`, `/plugins`, …) | Slash completion only | **P** — inspector “Extensions” section listing mounted tools from `dumpTools` + notices |
| LSP status | Not on rpc-ui state | Explicit “not published” copy | **N** until OMP publishes |

### 4.6 Subagents & collaboration

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| `task` / Agent Hub | Inspector list + message tail | **P** — progress strip in transcript, jump-to-subagent, better lifecycle badges |
| `set_subagent_subscription` levels | Connect sets `events` | **P** — user-selectable off/progress/events |
| Collab / share / join | `/share`, CLI `omp share` | Slash/CLI only | **P** — “Share session” palette → reveal URL/path |
| IRC / collab messages | `irc_message` event | **U** → dedicated row |

### 4.7 Todos, goals, memory, TTSR

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| `todoPhases` + `set_todos` | Checklist inspector | **P** — blocked state, reasons, phase UX matching todo tool ops |
| `todo_reminder` / `todo_auto_clear` | auto_clear handled; reminder **U** | **A** — reminder toast/row |
| `goal_updated` | **U** | **A** — goal chip / row if users use `/goal` |
| `ttsr_triggered` | **U** | **P** — quiet notice (TTSR is OMP-side) |
| Memory / mnemopi / hindsight | Config + slash | Slash-only | **N** for v1 UI (document) |

### 4.8 Auth & providers

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| `get_login_providers` / `login` | Palette flow | **P** — settings “Accounts” list with auth status |
| Auth-broker / gateway | CLI | **N** — power-user docs only |
| Cursor OAuth / tokens | Works via omp | Document dogfood auth path | Doc |

### 4.9 Attachments & media

| OMP surface | Pimiento | Gap |
|-------------|----------|-----|
| Images on prompt/steer | Done | — |
| Vision / inspect_image | Tool + slash | **P** — ensure image tool results render usefully |
| Audio / live / STT | TUI-native | **N** |

### 4.10 Packaging, platforms, polish

| Item | Gap |
|------|-----|
| Linux AppImage/`.deb` | **A** (macOS unsigned `.app` exists) |
| Windows | **N** (PLAN) |
| IME Enter-while-composing guard | **Blocked** on gpui-component API |
| Cross-message text selection | **N** (copy affordances) |
| Wayland popup QA | Manual QA remaining |
| SH-1…SH-6 recorded proofs | **Operational debt** |

### 4.11 Events still Unknown / silent (projection debt)

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

## 5. Prioritized delivery waves

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

### Wave U — Quiet Pepper Console (T4-inspired visual system)

**Why parallel-early:** Control parity without scan hierarchy still feels like a raw RPC shell. T4’s calm density is the bar; Pimiento must hit it in GPUI without cloning Pi Pink.

Ship **U1–U2 before or alongside Wave A** so new toggles land inside the elevated composer/inspector chrome rather than retrofitting twice.

| # | Work item | Detail |
|---|-----------|--------|
| U1 | Token module + status pills | `PimientoTokens` (paprika identity / ember action / zero-chroma neutrals / semantic status). Map `RunPhase` + dialog pending → dot+label pills. Light+dark. |
| U2 | Composer island + chrome dedupe | Elevate composer (hairline + limited shadow); Steer primary / Queue outline; ctx **ring**; strip only omp/phase; model·think·fast only on composer |
| U3 | Action-group tools + ask chrome | Collapse consecutive tool steps; numbered ask/approval cards with recommended + 1–9 hint (pairs with B1) |
| U4 | Rail densify | Status pills per row; priority rollup on workspace headers; 8pt rhythm; no left accent stripe |
| U5 | Motion + visual QA | Duration tokens; reduced-motion; screenshot diff vs T4 *structure* (not colors) on light/dark streaming, ask, tool-expanded |

**Exit:** Side-by-side with T4 lane-b shots: same information hierarchy and calm density; **obviously different** brand hue; gate + light/dark manual QA green.

**Skill sync:** After U1–U2 land, update `.cursor/skills/pimiento-ui-craft/SKILL.md` to point at the shipped tokens (keep Adobe/GPUI hard rules).

---

## 6. Explicit non-goals (remain Doctrine 10)

Do **not** schedule unless doctrine changes:

1. Reimplementing OMP settings UI as a full mirror of `~/.omp/agent/config.yml`.
2. Multiplexing multiple OMP sessions over one child via `switch_session`.
3. Bundling / auto-installing omp.
4. Terminal emulator pane / full PTY UI (rpc-ui forces `PI_NO_PTY=1`).
5. ACP multi-harness hosting (keep `pimiento-core` clean for a future crate; build nothing speculative).
6. Speech / live STT / fleet dashboards / remoting / Tailnet multi-host (T4 FEATURE_MATRIX Launch items we reject).
7. Copying GPL Zed agent UI code.
8. **Cloning T4’s Pi Pink brand, OMP mark geometry, Electron chrome, or cream/terracotta aesthetic.** Steal layout discipline only.

---

## 7. Version & wire hygiene

| Practice | Detail |
|----------|--------|
| Pin for extraction | Re-extract `rpc-types.ts` + `agent-session-events.ts` at each newly supported omp minor |
| Fixtures | One NDJSON fixture per wave feature path |
| protocol-notes | Required for every newly projected event or newly sent command |
| Version gate | Keep banner; widen tested range only after Wave 0 QA on that version |
| 17.2.11 watchouts | Agent Plugins 1.0; `omp share`; Cursor quota/auth fixes; no RPC schema break vs 17.2.10 |

Live slash set (38) is **dynamic** — never hardcode; always prefer `available_commands_update`.

---

## 8. Suggested sequencing vs original PLAN milestones

```
Wave 0  →  close SH proofs + doc drift                 (ops)
Wave U1–U2 →  tokens, status pills, composer island     (visual foundation)
Wave A  →  queue/compact/stats controls                 (lands in new chrome)
Wave U3 + B →  action groups + rich ask                 (feel / self-host)
Wave U4 →  rail densify                                 (scan cost)
Wave D  →  subagents/share/extensions                   (parallel after A)
Wave C  →  host bridge                                  (largest; after A+B)
Wave U5 →  motion + L/D QA                              (before calling “done”)
Wave E  →  packaging
```

Parallelism allowed by crate split:

- `omp-rpc-client` + fixtures: new command send helpers / event kinds
- `pimiento-core`: projection for reminders/goals/hub summaries
- `pimiento-app`: tokens, composer/rail/transcript chrome, inspector toggles

Integrate serially; gate with `scripts/gate.sh`.

---

## 9. Acceptance criteria (plan-level)

The parity program is “done enough” when:

1. **Wave 0** SH proofs exist for macOS + Linux.
2. **Wave U1–U2** ship a recognizable Quiet Pepper Console: status pills, ember primary CTA, deduped chrome, ctx ring — **not** a T4 pink clone.
3. **Wave A** users can change queue + auto-compact/retry without leaving Pimiento.
4. **Wave B + U3** multi-question `ask` never stalls a run; tool action groups are scannable; todo reminders visible.
5. Unknown rate for common sessions drops: `todo_reminder`, `goal_updated`, `ttsr_triggered`, `irc_message` projected or consciously quieted.
6. Host bridge (**Wave C**) is either shipped behind a flag or explicitly deferred here.
7. **Wave U5** light/dark visual QA recorded against the structural checklist in §3.
8. Linux packaging (**Wave E1**) exists or is explicitly waived for personal dogfood.

---

## 10. Risks

| Risk | Mitigation |
|------|------------|
| OMP weekly churn | Unknown rows + version gate + fixture re-record |
| Doctrine violation via `omp config` / role chips | Prefer RPC; if config CLI used, only on explicit user action; never silent |
| Rich ask shapes drift | Extract from live `ask` tool schema + rpc-ui captures; don’t guess |
| Hub/jobs UI invents state | Parse tool results only; re-fetch via slash/`hub` ops when unsure |
| Host tools security | Default deny; confirm cards; no yolo registration |
| Scope explosion into TUI clone | Enforce non-goals; slash passthrough first |
| Accidental T4 visual clone (pink/cream/glow) | Token review in U1; screenshot side-by-side must show distinct brand |
| Composer float fighting GPUI list layout | Keep composer **docked**; fake float with elevation/shadow only |
| Action-group reflow jank | Strict `ListState` splice/reset discipline; fixture + manual scroll QA |

---

## 11. Appendix — live probe snapshot (2026-08-09)

**ready:** `protocolVersion:1`, supports `[1,2]`, maxFrame 1 MiB, reassembly 64 MiB  

**get_state keys observed:**  
`autoCompactionEnabled`, `contextUsage`, `dumpTools`, `fastModeActive`, `fastModeEnabled`, `followUpMode`, `interruptMode`, `isCompacting`, `isStreaming`, `messageCount`, `model`, `queuedMessageCount`, `sessionId`, `steeringMode`, `systemPrompt`, `todoPhases`, `tokensPerSecond`  

**Queue defaults observed:** steering/follow-up `one-at-a-time`, interrupt `immediate`, autoCompaction `true`  

**Slash (38):**  
`add-dir`, `advisor`, `autoresearch`, `browser`, `changelog`, `compact`, `computer`, `context`, `dirs`, `dump`, `export`, `fast`, `force`, `fresh`, `green`, `init`, `jobs`, `marketplace`, `mcp`, `memory`, `model`, `move`, `plugins`, `prewalk`, `reload-plugins`, `remove-dir`, `rename`, `review`, `security`, `session`, `shake`, `share`, `ssh`, `stats`, `todo`, `tools`, `usage`, `vision`

**dumpTools (this env):**  
`read`, `bash`, `ask`, `eval`, `glob`, `grep`, `task`, `hub`, `todo`, `web_search`, `write`

---

## 12. Next concrete action

1. **Wave 0** — SH proof recording + PLAN/protocol-notes drift fix.  
2. **Wave U1** — land `PimientoTokens` + status pills (foundation for everything visual).  
3. **Wave U2 + A1–A3** — composer island + queue/auto toggles together so new controls inherit the Quiet Pepper chrome.

Reference while implementing: T4 `DESIGN.md` + lane-b screenshots for *structure*; this doc §3 for *Pimiento* color/depth; craft skill for GPUI hard rules.
