# Pimiento — Native GPUI Desktop Client for Oh My Pi (OMP)
## v2 — Self-Hosting-First Edition

> **Handoff note for the implementing agent:** This document is self-contained; you need no other context. It supersedes any earlier draft. Read it fully before writing code. The plan is organized around one strategic objective: reach the **Self-Host Gate** — the point where Pimiento is good enough to be the primary interface for developing Pimiento itself — as early as possible, then dogfood every subsequent feature through the app.

**Goal:** A fast, native desktop app (macOS + Linux; Windows later) that drives the **user's existing `omp` installation** as a supervised child process over its NDJSON RPC protocol, rendering a semantic streaming agent workspace: transcript, tool cards, dialogs, abort/steer, sessions.

**Architecture in one sentence:** The OMP runtime is the sole authority over all agent/session state; the GPUI app is a *projection + command surface* that renders decoded RPC frames into entities and sends typed commands back — it never fabricates, infers, or persists runtime truth.

**Strategic objective:** Milestone SH ("Self-Host Gate") is the center of gravity. Everything before it is ruthlessly minimal; everything after it is implemented *using Pimiento + OMP as the development tool*, which makes the app its own permanent QA harness.

**Tech stack:** Rust (stable), GPUI + gpui_platform (pinned Zed git rev, Apache-2.0), gpui-component (Longbridge, Apache-2.0, pinned rev), smol (matches GPUI executors), serde/serde_json, base64, anyhow/thiserror. Child process: the user's `omp` in `--mode rpc-ui`.

**License posture:** gpui/gpui_platform: Apache-2.0. gpui-component: Apache-2.0. OMP: MIT. Do **NOT** copy code from Zed's `ui`, `editor`, `terminal_view`, `agent_ui` crates (GPL-3.0-or-later). Patterns yes, code no.

---

## Table of contents

0. Doctrine (non-negotiable rules)
1. Using the user's existing OMP install
2. Feature ranking — what is actually critical, and why
3. Repository layout
4. The OMP RPC protocol contract (authoritative summary)
5. Core architecture
6. Milestones (M0 → SH → D1–D4), with the Self-Host Gate criteria
7. The dogfood ritual (how the app builds itself)
8. UI/UX specification
9. Edge cases & failure matrix
10. Testing strategy
11. Risks & open questions
12. Appendix: sources

---

## 0. Doctrine — rules that override any local decision

Distilled from T4 Code's DESIGN.md (the shipped Electron OMP client), Zed's ACP design, and OMP's RPC docs. Violating these creates the bug class that kills agent frontends.

1. **The app never owns runtime state.** Everything durable (messages, session names, model, todos, locks, auth) lives in OMP. The client keeps only disposable display state: composer draft, scroll anchor, expanded/collapsed cards, layout. On doubt, re-fetch (`get_state`, `get_messages_page`) — never reconstruct by guessing.
2. **The app never owns the harness either.** Pimiento uses the user's existing `omp` binary, auth, config, model roles, extensions, and session store (§1). It never installs, updates, or reconfigures OMP, and never writes into `~/.omp`.
3. **Commands are explicit and correlated.** Every mutation is a typed RPC command with a generated `id`. Match responses on `id`, never on emission order (`bash` responses interleave).
4. **Command acceptance ≠ completion.** `prompt` ACKs immediately. A run completes only on `agent_end` with `isTerminal !== false`, or `data.agentInvoked:false` / `prompt_result` for local-only prompts.
5. **Four state qualities, rendered distinctly:** durable (snapshot), live (streaming), stale (child restarting — dimmed + banner), unknown (say "unknown"; never fabricate a default).
6. **Never smooth over failures.** Disabled controls state *why*. A crashed child gets a crash card with stderr tail and a Restart button, not a silent respawn.
7. **Child stdin/stdout are sacred protocol channels.** App logging goes elsewhere. Nothing is written to child stdin except valid frames.
8. **One session per child process.** Multi-session = multiple supervised `omp` processes. No multiplexing over `switch_session`; process isolation makes crash recovery tractable.
9. **Unknown wire data always renders.** Every enum has an `Unknown` fallback; unknown frames become visible raw rows. Never panic, never drop silently. This is what lets the app survive OMP upgrades.
10. **YAGNI ruthlessly.** No multi-host remoting, Tailnet pairing, browser workspace, terminal emulator pane, fleet dashboard, or settings mirror. T4 burned enormous complexity there; we ship one excellent session workspace that can build software — including itself.

---

## 1. Using the user's existing OMP install

Pimiento is a **frontend for the OMP the user already has**. This is a hard product requirement with concrete consequences:

**Discovery (in order):**
1. `PIMIENTO_OMP_BIN` env var (absolute path), else
2. app setting `omp_bin` if the user set one, else
3. `omp` resolved on the **user's login-shell PATH**. On macOS, GUI apps get a stripped environment — resolve PATH by running the user's shell as a login shell (`$SHELL -lc 'command -v omp && echo ---ENV--- && env'`) once at startup and caching the result. This is the standard GUI-app-on-macOS fix; without it, `~/.local/bin`, Homebrew, and mise shims are invisible.

**Environment inheritance:** spawn `omp` with the captured login-shell environment (HOME, PATH, provider API-key vars, etc.). This is what makes the user's existing auth (`~/.omp` OAuth credentials, keys), `~/.omp/agent/config.yml` model roles, `models.yml` custom providers, extensions, skills, and MCP servers all work with **zero configuration in Pimiento**.

**Model policy:** do not call `set_model` at startup. The session comes up on the user's configured `default` model role. Pimiento *displays* the active model from `get_state` and offers a picker (post-SH) that calls `set_model` only on explicit user action. Fallback chains, credential rotation, and role routing remain OMP's business.

**Sessions:** OMP owns the session store. Pimiento persists only lightweight pointers app-side: `{sessionFile, cwd, name, lastUsed}` for its "recent" list, and resumes via `--resume <sessionFile>`. If OMP refuses (lock, missing file), show OMP's error verbatim.

**Version gate, not version bundling:** on startup run `omp --version`; maintain a tested range (start: the version you develop against, e.g. ≥17.x). Outside range → yellow banner "Pimiento was tested with omp X–Y; you have Z — unknown events will still render" and proceed. `ready.supportedProtocolVersions` must include 2; if not, hard-fail with a clear card.

**Missing OMP:** show a card with the official install one-liner (`curl -fsSL https://omp.sh/install | sh`) as *text for the user to run* + a "re-detect" button. Never auto-install, never shell out to install.

**Never write to `~/.omp`.** Auth flows happen through the RPC `login` command + `open_url` dialog round-trip; OMP persists credentials itself.

---

## 2. Feature ranking — think-deep priority order

The ranking question: *what is the minimum closed loop that lets a developer sit in Pimiento, point OMP at the Pimiento repo, and have the agent write → build → run → read errors → fix → repeat, with the human steering?* Everything in that loop is Tier 0/1. Everything else is comfort.

Reasoning about the loop's actual dependencies:

- The agent does the editing, building, and debugging **through its own tools** (`edit`, `bash`, `read`, `lsp`). Pimiento doesn't need an editor, terminal emulator, or diff engine for self-hosting — it needs to make the agent's actions *legible* and the human's steering *frictionless*.
- The two things that silently kill a self-host loop: (a) an agent question the UI can't answer (run stalls forever → **dialogs are Tier 1, not polish**), and (b) invisible tool failures (agent claims success, human can't see the failing `cargo build` output → **tool output visibility is Tier 1**).
- The thing that makes iteration *fast* rather than merely possible: mid-run steering ("stop — the error is in the decoder, not the UI") without losing the turn → **abort/steer/follow-up are Tier 1**.
- The thing that makes it *safe*: the app under development crashes a lot; the OMP session must survive Pimiento crashing → **resume-by-pointer is Tier 1**. (The dogfood ritual §7 adds a tmux fallback so a broken build never strands the session.)
- Markdown beauty, model pickers, multi-session, diff review — none gate the loop. The agent's default model is already configured (§1); one session pointed at one repo is enough to start.

### Tier 0 — Bedrock (no UI value alone; everything depends on it)

| # | Feature | Why it's ranked here |
|---|---------|---------------------|
| 0.1 | RPC client: spawn user's omp, ready handshake, v2 negotiation + chunk reassembly, correlation map, event stream | The entire product is downstream of a correct wire implementation. Chunking matters immediately: `cargo build` output routinely exceeds 1 MiB. |
| 0.2 | Projection reducer (UI-free, replay-testable) + Unknown-frame tolerance | Deterministic state from events is what makes every later feature testable without a live agent; Unknown-tolerance is what survives OMP upgrades. |
| 0.3 | Process supervision: graceful shutdown, crash capture (stderr ring + exit code), fail-pending-on-death | A self-hosting app kills its own child processes constantly (rebuilds). Crash handling is a bedrock behavior, not an edge case. |

### Tier 1 — Self-Host Critical (the SH gate; nothing here is optional)

Ranked *within* the tier by how immediately their absence breaks the loop:

| # | Feature | Justification |
|---|---------|---------------|
| 1.1 | Streaming transcript with tail-follow (text deltas, per-turn structure) | The loop's output channel. Without live streaming you're flying blind between turns. |
| 1.2 | Tool cards with **full visible output** — especially `bash` (build/test runs) and `edit`/`write` results — summary line + expandable output, even if visually raw | The loop's *truth* channel. Debugging Pimiento-via-Pimiento means reading real `cargo build`/`cargo test` output inside the transcript. An agent frontend that hides tool output cannot be used to develop anything, least of all itself. |
| 1.3 | Composer: send; **streaming-aware steer/follow-up** (`streamingBehavior` is mandatory mid-stream — the wire fails without it); Esc→abort with confirm | The loop's steering wheel. Iteration speed lives here. |
| 1.4 | Extension-UI dialogs: `confirm`/`select`/`input` inline cards, keyboard-first (y/n, 1–9, Esc), timeout + `cancel` handling | OMP's `ask` tool and permission-style prompts arrive on this channel in `rpc-ui` mode. Unanswerable dialog = permanently stalled run = loop dead. |
| 1.5 | Truthful run-phase state machine: Idle / Streaming / AwaitingResume (`isTerminal:false`) / Compacting / Restarting / Dead — driving composer affordances and spinner | The human must always know whether the agent is working, waiting on them, or wedged. Misreporting this destroys trust in minutes. |
| 1.6 | Session start with **working-directory picker** + resume-by-pointer (`--resume`) + crash card with Restart | Points the agent at the Pimiento repo; survives the app being rebuilt/killed underneath the session. |
| 1.7 | Error/notice/`command_output`/Unknown rows rendered honestly | Failures during self-development are the *common* case; they must be first-class UI. |
| 1.8 | Read-only status strip: active model, thinking level, context meter (`contextUsage.percent`), tokens/sec | Zero-interaction awareness. Context % is genuinely operational: it tells you when to `/compact` before a long build-fix session. Display only — no pickers yet. |

### Tier 2 — Daily-driver comfort (post-SH, dogfooded)

2.1 Slash-command completion from `available_commands_update` (unlocks `/compact`, `/review`, `/model` via text without dedicated UI) · 2.2 Markdown/code-block polish + per-block copy buttons · 2.3 Model & thinking pickers (write path) · 2.4 Multi-session + session rail + Cmd/Ctrl+1..9 · 2.5 Todo panel (`todoPhases`, reminders) · 2.6 History hydration via `get_messages_page` for resumed sessions · 2.7 Fast-mode toggle (render `{enabled, active}` divergence honestly).

### Tier 3 — Power features

3.1 Diff review rows parsed from edit-tool results (read-only + confirmed-revert flow) · 3.2 Subagent strip + drawer (`set_subagent_subscription "progress"`, `get_subagent_messages` incremental tail) · 3.3 Compaction/retry UX (notices, fallback-model banners) · 3.4 `export_html` · 3.5 Theme polish + full keymap + command palette.

### Tier 4 — Later / explicitly deferred

Packaging & signing · Windows · host tools/URI registration (wire plumbing ships in Tier 0 types; registration UI later) · images in prompts · ACP mode (multi-harness) · cross-message text selection (GPUI lacks it; copy affordances instead) · TUI-session takeover/handoff (needs upstream authority surface).

---

## 3. Repository layout

Cargo workspace, strict dependency direction `omp-rpc-client` ← `pimiento-core` ← `pimiento-app`. Protocol types never import UI types.

```
pimiento/
├── Cargo.toml / Cargo.lock (COMMITTED) / rust-toolchain.toml
├── crates/
│   ├── omp-rpc-client/          # pure protocol lib, no GPUI
│   │   ├── src/{lib,frames,decoder,client,supervisor,discovery,error}.rs
│   │   └── tests/               # golden frames, chunk property tests, fake-server
│   ├── pimiento-core/           # projection + domain, UI-free
│   │   └── src/{lib,projection,transcript,markdown,settings}.rs
│   └── pimiento-app/            # GPUI
│       └── src/{main,app,workspace,transcript_view,composer,status_strip,
│                session_launcher,bridge,theme,keymap}.rs
│           └── rows/{assistant,user,tool_card,thinking,dialog,error,notice,unknown}.rs
├── fixtures/                    # recorded NDJSON transcripts
├── scripts/dogfood.sh           # §7 bootstrap script
└── docs/protocol-notes.md       # deltas discovered vs upstream docs
```

`discovery.rs` implements §1 (login-shell env capture, omp resolution, version probe).

---

## 4. OMP RPC protocol contract (authoritative summary)

Canonical upstream sources (keep open while implementing): `docs/rpc.md`, `packages/coding-agent/src/modes/rpc/rpc-types.ts` (canonical types), `rpc-frame.ts` (reference chunk decoder), Python reference client under `docs/python/omp-rpc/` — all in https://github.com/can1357/oh-my-pi.

### 4.1 Transport

- Spawn `omp --mode rpc-ui` (why rpc-ui: §4.7) with piped stdin/stdout/stderr, using the captured user environment (§1).
- One UTF-8 JSON object + `\n` per frame. Server→client physical frames ≤1 MiB. Client→server frames are **never chunked**: keep each command under `maxFrameBytes`.
- First server frame:
  ```json
  {"type":"ready","protocolVersion":1,"supportedProtocolVersions":[1,2],
   "maxFrameBytes":1048576,"maxReassembledFrameBytes":67108864}
  ```
- Immediately send `{"id":"protocol-1","type":"negotiate_protocol","protocolVersion":2}`; success carries `data.protocolVersion:2`.
- **v2 chunking (server→client only):** oversized logical objects arrive as an *uninterrupted* run of
  ```json
  {"type":"rpc_chunk","chunkId":"rpc-1","index":0,"count":7,"byteLength":1600042,"data":"<base64>"}
  ```
  Decoder MUST: contiguous `index` from 0; constant `chunkId`/`count`/`byteLength`; reject any interleaved non-chunk frame mid-sequence; total decoded bytes == `byteLength` ≤ advertised reassembly limit; strict UTF-8; parse exactly one JSON object. Chunk payloads are 256 KiB.
- Pre-negotiation oversize appears as a failure response (`error: "RPC response exceeded the transport limit"`) or `rpc_frame_error` — surface as an error row.

### 4.2 Correlation & responses

- Commands accept `id?: string` (generate `req_<n>`). Success `{"id?","type":"response","command","success":true,"data?"}`; failure adds `"error"` and optional machine `"code"`.
- Malformed client JSON → server replies `command:"parse"` with **no id** and continues. Unknown commands also return `id: undefined`. Tolerate id-less responses (log; don't corrupt the pending map).
- `prompt`/`abort_and_prompt` ACK immediately; `prompt` may emit a **same-id late async error** — keep it armed until a terminal signal.
- Response order across concurrent commands (esp. `bash`) is NOT guaranteed.

### 4.3 Commands (exact shapes)

Prompting: `prompt {message, images?, streamingBehavior?:"steer"|"followUp"}` · `steer {message, images?}` · `follow_up {message, images?}` · `abort {}` · `abort_and_prompt {message, images?}` · `new_session {parentSession?}`

State: `get_state` · `set_fast_mode {enabled}` · `get_available_commands` · `set_todos {phases}` · `set_host_tools {tools}` · `set_host_uri_schemes {schemes}` · `set_subagent_subscription {level:"off"|"progress"|"events"}` · `get_subagents` · `get_subagent_messages {subagentId?, sessionFile?, fromByte?}`

Model/thinking: `set_model {provider, modelId}` · `cycle_model` · `get_available_models` · `set_thinking_level {level}` · `cycle_thinking_level` (levels `off|minimal|low|medium|high|xhigh|max`)

Queue: `set_steering_mode {mode:"all"|"one-at-a-time"}` · `set_follow_up_mode {…}` · `set_interrupt_mode {mode:"immediate"|"wait"}`

Maintenance: `compact {customInstructions?}` · `set_auto_compaction {enabled}` · `set_auto_retry {enabled}` · `abort_retry` · `bash {command}` · `abort_bash`

Session/messages: `get_session_stats` · `export_html {outputPath?}` · `switch_session {sessionPath}` · `branch {entryId}` · `get_branch_messages` · `get_last_assistant_text` · `set_session_name {name}` · `handoff {customInstructions?}` · `get_messages` · `get_messages_page {cursor?, limit?}`

Auth: `get_login_providers` · `login {providerId}`

Key behaviors:
- **Mid-stream `prompt` without `streamingBehavior` FAILS.** The composer must track `isStreaming` and attach `"steer"` (default) or `"followUp"`.
- `get_messages_page`: ≤256 msgs/page, opaque `nextCursor`; errors `code:"session_busy"` (streaming/compacting) or `"stale_cursor"` (snapshot changed) → discard partial walk, retry when idle.
- `get_state` payload: `model {provider,id}`, `thinkingLevel`, `isStreaming`, `isCompacting`, `steeringMode`, `followUpMode`, `interruptMode`, `sessionFile`, `sessionId`, `sessionName`, `fastModeEnabled`, `fastModeActive`, `tokensPerSecond|null`, `autoCompactionEnabled`, `messageCount`, `queuedMessageCount`, `todoPhases`, `systemPrompt`, `dumpTools`, `contextUsage {tokens, contextWindow, percent}`.
- `set_fast_mode` returns `{enabled, active}` which can legitimately diverge; render both.

### 4.4 Event stream (unsolicited)

Session events: `agent_start`, `agent_end {messages, isTerminal?}`, `turn_start`, `turn_end`, `message_start`, `message_update {assistantMessageEvent, message}`, `message_end`, `tool_execution_start`, `tool_execution_update`, `tool_execution_end`, `auto_compaction_start/end`, `auto_retry_start/end`, `retry_fallback_applied/succeeded`, `model_changed`, `thinking_level_changed`, `ttsr_triggered`, `todo_reminder`, `todo_auto_clear`, `irc_message`, `notice`, `goal_updated`.

- `message_update.assistantMessageEvent` variants include `text_delta {delta}`, thinking deltas, tool-call deltas. **Extract the exact variant set from `rpc-types.ts` at the pinned OMP version** and model it as an exhaustive Rust enum with an `Unknown` catch-all.
- `agent_end.isTerminal === false` → session resumes shortly (maintenance/async); stay in AwaitingResume, don't declare idle.

Side channels: `available_commands_update {commands}`, `command_output {text}`, `session_info_update`, `config_update`, `extension_error {extensionPath, event, error}`, `prompt_result {id?, agentInvoked}`, and gated `subagent_lifecycle|subagent_progress|subagent_event`.

### 4.5 Extension UI sub-protocol (dialogs — Tier 1)

Server→client: `{"type":"extension_ui_request","id","method",...}`, methods `select | confirm | input | editor | cancel | notify | setStatus | setWidget | setTitle | set_editor_text | open_url`.

Client→server (dialogs only):
`{"type":"extension_ui_response","id","value":"..."}` · `{...,"confirmed":true|false}` · `{...,"cancelled":true,"timedOut?":true}`.

Rules: `method:"cancel"` carries `targetId` → dismiss that pending card. Requests may carry `timeout` ms (server auto-resolves on expiry — dismiss locally too). `open_url {url, launchUrl?, instructions?}` powers OAuth `/login`: open OS browser, show instructions + selectable URL. `setTitle` suppressed unless `PI_RPC_EMIT_TITLE=1` (don't set). `notify`→toast, `setStatus`→status strip text, `setWidget`→small widget slot (can be a stub initially, but render *something*).

Render dialogs as **inline blocking cards pinned above the composer**, keyboard-first (y/n, 1–9, Esc). Never OS modals.

### 4.6 Host tools & host URIs

Implement the frame types and dispatch in `omp-rpc-client` (they're part of the wire): `host_tool_call {id, toolCallId, toolName, arguments}` → `host_tool_update {id, partialResult}`* → `host_tool_result {id, result, isError?}`; `host_tool_cancel {id, targetId}`. Same pattern for `host_uri_request/result/cancel` (`operation:"read"|"write"`, lowercase schemes, `security://` reserved). **v1 registers nothing** (`set_host_tools`/`set_host_uri_schemes` never sent).

### 4.7 rpc vs rpc-ui; session flags

- Use **`--mode rpc-ui`**: identical wire, but the session gets real tool-UI context (`hasUI=true`) so `ask`-style tool dialogs/selectors surface as `extension_ui_request` frames (and `PI_NO_PTY=1` is forced). Plain `rpc` silently skips tool UI we depend on (Tier 1.4).
- Persistent session by default. `--no-session` (in-memory; incompatible with `--resume`/`--fork`) only for the app's own smoke tests. Resume: `--resume <path>`; `--continue` for latest.
- Startup order: spawn → `ready` → `negotiate_protocol 2` → `get_state` → `get_available_commands` → `set_subagent_subscription "progress"` → render. (`get_available_models` deferred to Tier 2.3.)

### 4.8 Shutdown & crash contract

- Graceful: close child stdin → OMP rejects pending UI/host calls, drains accepted commands, disposes session, exits 0. Wait 10 s → SIGTERM → SIGKILL.
- On malformed stdout, stdout EOF, or child exit: fail ALL pending ids with ChildDied; capture last 64 KiB stderr + exit code; fully reap before restart; **fresh decoder + correlation state** (never reuse an aborted reader); restart re-issues only idempotent setup (negotiate, get_state, subscription) and re-attaches via `--resume <sessionFile>` captured from the last `get_state`.

---

## 5. Core architecture

### 5.1 Dependency pinning (first task, do exactly)

```toml
# workspace Cargo.toml [workspace.dependencies]
gpui           = { git = "https://github.com/zed-industries/zed", rev = "<PIN>" }
gpui_platform  = { git = "https://github.com/zed-industries/zed", rev = "<PIN>", features = ["font-kit", "x11", "wayland", "runtime_shaders"] }
gpui-component = { git = "https://github.com/longbridge/gpui-component", rev = "<PIN2>" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
smol = "2"
base64 = "0.22"
anyhow = "1"
thiserror = "2"
```

Choose `<PIN>` = the Zed rev gpui-component's own lockfile builds against (inspect their workspace/`Cargo.lock`); verify their gallery example runs; freeze both. Commit `Cargo.lock`. Upgrades only on a dedicated branch with the full QA pass (§10.4). Do NOT use crates.io `gpui` (no matching `gpui_platform`; mismatches gpui-component).

### 5.2 GPUI registers mapped

- **Entities:** `SessionEntity` (SessionProjection + RPC handle + task handles), `WorkspaceEntity` (sessions, active id, layout), `ThemeEntity`. Mutate only via `entity.update(cx, ..)`; long-lived tasks hold `WeakEntity`.
- **Views:** workspace → launcher | (transcript + status strip + composer). Render is pure projection; zero I/O or heavy parsing in render paths.
- **Elements:** custom only for transcript rows and streaming text; everything else from gpui-component (buttons, inputs, markdown, scrollbars, theming).

### 5.3 Event pipeline

```
child stdout ──(background_spawn: line reader)──► LineDecoder ──► ChunkReassembler
      ──► serde_json → RpcFrame ──► smol::channel::unbounded<RpcFrame>
      ──► foreground pump (cx.spawn): drain ALL queued frames per wakeup
           ├─ responses → resolve correlation map (oneshot senders)
           └─ events    → SessionEntity.update: projection.apply(event); ONE cx.notify()
child stderr ──(background)──► 64 KiB ring buffer (crash cards)
child exit  ──(background)──► Supervisor → ChildDied frame into the same channel
commands: SessionHandle::send(cmd) -> oneshot<RpcResponse>; single background
          stdin-writer task fed by a channel (lines never interleave)
```

Critical: **batch deltas** — drain the whole channel per wakeup, apply as one entity update; coalesce to 30–60 Hz with a timer only if frame time exceeds ~8 ms. **Never** update entities from the background executor. Dropping a GPUI `Task` cancels it — store reader/pump tasks in the entity; drop on close.

### 5.4 Projection model (pimiento-core, UI-free, replay-testable)

```rust
struct SessionProjection {
    state: RuntimeState,              // mirror of get_state + staleness flag
    transcript: Vec<TranscriptEntry>, // append-mostly; streaming tail mutated in place
    pending_dialogs: Vec<UiDialog>,
    run_phase: RunPhase,              // Idle|Streaming|AwaitingResume|Compacting|Retrying|Restarting|Dead
    todos: Vec<TodoPhase>,
    subagents: Vec<SubagentSummary>,
}

enum TranscriptEntry {
    User { text },
    AssistantText { markdown_ir, streaming: bool },
    Thinking { text, streaming: bool, collapsed: bool },
    ToolCall { tool_name, args_json, status: Running|Ok|Err, output: BoundedText,
               duration, expanded: bool },
    Notice(String), Error { message, code: Option<String> },
    CommandOutput(String), Compaction { phase }, RetryInfo { detail },
    Unknown { raw: serde_json::Value },   // ALWAYS render
}
```

`BoundedText`: cap stored tool output (512 KiB head+tail with an elision marker) — build logs can be huge even before v2 chunk limits. The reducer is pure: recorded NDJSON fixtures replay deterministically in tests (§10.2).

---

## 6. Milestones

Estimated for one strong Rust developer. Everything through SH is hand-built; everything after SH is dogfooded (§7).

### M0 — Skeleton & pins (2–3 days)
1. Workspace per §3; pins per §5.1; toolchain pin.
2. `pimiento-app` opens a themed gpui-component window (button + dark/light toggle) on macOS AND Linux (X11 + Wayland).
3. Local gate script `scripts/gate.sh`: fmt `--check` + clippy `-D warnings` + nextest, run before every merge to main (no remote git / hosted CI until the user has a verified working version — see the kickoff prompt's local-only rule).
**Verify:** window on both OSes; gate script green.

### M1 — omp-rpc-client (1 week)
1. `discovery.rs`: §1 in full — login-shell env capture, omp resolution chain, `--version` probe, protocol-support check.
2. `frames.rs`: full serde model of §4 with `Unknown` fallbacks on every enum. Round-trip tests against golden JSON copied verbatim from upstream `docs/rpc.md`. **Extract exact `assistantMessageEvent` and `tool_execution_*` payload shapes from `rpc-types.ts` at the user's OMP version** (this gates M2 — do it first).
3. `decoder.rs`: line splitter + v2 chunk reassembler with the full validation matrix (§4.1). Property tests: random splits reassemble; interleaving rejects; over-limit rejects; bad UTF-8 rejects.
4. `client.rs`: spawn, handshake, negotiate v2, correlation map, event broadcast, single stdin writer.
5. `supervisor.rs`: graceful shutdown, crash capture, fail-pending-on-death, restart with fresh state + `--resume` (§4.8). Crash-loop breaker: 3 restarts/60 s → Dead.
6. Fake-server harness (Rust bin speaking the protocol from scripted fixtures: chunked frames, same-id late errors, id-less responses, malformed lines, mid-stream EOF). All automated tests run against it — no live OMP needed for the gate script.
7. `#[ignore]` live smoke vs the real user omp: `--mode rpc-ui --no-session`, handshake, `get_state`, `prompt "reply with exactly: pong"`, await terminal `agent_end`.
**Verify:** `cargo test -p omp-rpc-client` green; live smoke passes on the dev machine.

### M2 — Projection core (3–4 days)
1. `SessionProjection` reducer for the full Tier-1 event set: delta accumulation, tool lifecycle, run-phase machine, dialogs queue, notices/errors, `get_state` hydration, `available_commands_update` storage.
2. Record 4 real fixtures via a tee-recorder wrapper around the live client: (a) plain answer; (b) multi-tool run incl. a `cargo build` with errors (big bash output → exercises chunking + BoundedText); (c) aborted run; (d) run containing an `ask`-tool dialog.
3. Replay tests → `insta` snapshots of final projections.
**Verify:** snapshots deterministic; fixture (b) contains ≥1 reassembled v2 chunk sequence.

### SH — Self-Host Gate (2–2.5 weeks) ← the milestone that matters
Implements Tier 1 (§2) end-to-end:
1. `bridge.rs` pipeline per §5.3 with delta batching.
2. Transcript: GPUI variable-height `list` + `ListState`, `ListAlignment::Bottom`; tail-follow only while at bottom, "N new messages ↓" pill otherwise; `splice`/`reset` on any height-affecting change (card expand, streaming growth) — stale measurement is *the* classic GPUI list bug.
3. Rows (Tier 1.1/1.2/1.7): User; AssistantText (markdown via gpui-component; acceptable-not-beautiful); Thinking (collapsed); **ToolCall card**: summary line (icon · tool · ≤80-char arg digest · status chip · duration) + expandable output (mono, 24-line viewport, inner scroll, copy button, "elided" marker from BoundedText); Error; Notice; CommandOutput; Unknown (collapsed raw JSON).
4. Composer (1.3): multiline; Enter send / Shift+Enter newline; mid-stream the send control splits into **Steer** (default) / **Queue follow-up**; double-Esc abort; disabled-with-reason under Restarting/Dead/pending-blocking-dialog.
5. Dialog cards (1.4): queue above composer; y/n/1–9/Esc; timeouts; `cancel`; `notify`→toast; `setStatus`→status strip; `open_url`→OS browser + selectable URL card.
6. Run-phase machine surfaced (1.5): spinner + phase label; `isTerminal:false` → "finishing up…".
7. Launcher (1.6): directory picker → spawn → live; recent-session pointers; crash card (stderr tail, exit code, Restart button → `--resume`).
8. Status strip (1.8): model · thinking level · context meter · tokens/sec (read-only).
9. `scripts/dogfood.sh` (§7).

**SH exit criteria — all must pass, on record (screen capture or session export):**
- **SH-1 (the self-host proof):** Using Pimiento itself, drive OMP to implement a real, pre-selected Pimiento feature from the Tier-2 backlog (designated: *2.2 code-block copy buttons*): agent edits Pimiento source, runs `cargo build` (fails), reads errors from the tool card, fixes, builds green, runs `cargo test` — human steers at least once mid-run. Then rebuild and relaunch Pimiento with the new feature live.
- SH-2: A dialog (`ask` tool) answered keyboard-only mid-run.
- SH-3: `kill -9` the omp child mid-stream → crash card → Restart → session resumes with history intact (`--resume`).
- SH-4: Quit Pimiento mid-run, relaunch, resume the same session pointer, continue the conversation.
- SH-5: A `cargo build` producing >1 MiB output renders (elided) without UI stall — proving chunking + BoundedText + batching under load.
- SH-6: Full loop on both macOS and Linux (either X11 or Wayland at SH; both by D4).

### D1 — Dogfood wave 1: daily-driver comfort (1–1.5 weeks, built via Pimiento)
Tier 2 in order: 2.1 slash completion → 2.2 (done as SH-1) → 2.3 model/thinking pickers → 2.6 history hydration (`get_messages_page`, `session_busy`/`stale_cursor` handling) → 2.5 todo panel → 2.7 fast-mode toggle.
**Verify:** each feature's branch was authored in a Pimiento session and merged after the gate script passed; fixture added per feature.

### D2 — Dogfood wave 2: multi-session (1 week)
Tier 2.4: WorkspaceEntity with N sessions (one child each); rail (name, cwd, phase badge, unread/attention dot; inline rename → `set_session_name`); Cmd/Ctrl+1..9/T/W; per-session display state preserved across switches.
**Verify:** 3 concurrent streaming sessions, no jank; killing one child affects only its session. Dogfood pattern: one session implements, a second session reviews (`/review`).

### D3 — Dogfood wave 3: power (1.5 weeks)
Tier 3: diff rows (parse `edit`/`write`/`ast_edit` results at pinned version; raw fallback when parsing fails; read-only + "Revert file…" flow = show exact `git` command in a confirm card → execute via RPC `bash` → show output) · subagent strip + drawer (`fromByte`/`nextByte`, `reset:true`) · compaction/retry UX · `export_html`.

### D4 — Polish & hardening (1 week)
Tier 3.5: semantic theme tokens, light/dark follow + override; full keymap (Cmd/Ctrl+K palette, Cmd/Ctrl+B rail, PageUp/Down/Home/End); copy affordances audit; IME QA (CJK composition) on all OSes; Wayland + X11 popup audit; manual QA checklist (§10.4) green everywhere.

### D5 — Packaging (3–5 days, optional for personal use)
macOS .app/dmg; Linux AppImage/.deb; "Reveal logs" menu; first-run omp-detection card (§1). No auto-install, no bundled omp — ever.

**Total: ~6–7 weeks to SH being genuinely self-hosting; ~10 weeks to polished D4.**

---

## 7. The dogfood ritual (how the app builds itself)

From SH onward, Pimiento development happens *inside Pimiento*. The loop has one structural hazard: the agent rebuilds the very binary the human is sitting in. The ritual makes that safe:

1. **Two checkouts:** `~/dev/pimiento` (stable — the binary you run) and `~/dev/pimiento-work` (the worktree the agent edits: `git worktree add`). The Pimiento session's cwd is the worktree. The agent builds/tests there; your running app is never overwritten under you. This mirrors the industry-converged session-per-worktree pattern (Claude Code desktop, Conductor/Crystal).
2. **Promotion:** when the agent's branch builds green + tests pass, human reviews (in-transcript diffs at D3; `git diff` via a bash tool card before that), merges to stable, rebuilds, relaunches — and resumes the same OMP session via its pointer (SH-4 guarantees this survives).
3. **Session continuity net:** `scripts/dogfood.sh` starts the dev session and records `{sessionFile, cwd}` to `~/.pimiento/dogfood.json`. If Pimiento is ever too broken to launch, the same session is reachable from the terminal: `omp --resume <sessionFile>` — the OMP session store is the safety net *because we never owned that state* (Doctrine 1–2). A broken frontend can never strand the work.
4. **Self-QA convention:** every dogfooded feature branch must (a) be authored in a Pimiento session, (b) add or update one NDJSON fixture exercising the feature's projection path, (c) note any wire-shape surprises in `docs/protocol-notes.md`, (d) pass the gate script before merging to main.
5. **Escalation rule:** if a Pimiento bug blocks the loop itself (e.g., dialog rendering breaks), fix that bug from the terminal TUI (`omp` directly in the worktree), then return to the app. Never let dogfooding purity block the dogfood.

---

## 8. UI/UX specification (condensed)

Layout: `[rail — D2, collapsible] [transcript, hero, min 480px] [right pane — D1+, on demand: Todos | Subagents | Session info]`. No dashboard theater, no permanent side panes, no decorative motion (T4's hard-won doctrine).

Transcript rows are **semantic, not chat bubbles**: full-width rows; user rows get an accent left border; assistant plain; tool cards inset on elevated surface; thinking dimmed/italic/collapsed; errors danger-tinted. Streaming text renders progressively; parse completed markdown blocks eagerly, the trailing incomplete block leniently; re-parse throttled to frames.

Tool cards: summary always; expanded shows pretty-JSON args + bounded mono output + copy. Running cards: elapsed timer; the only cancel is turn-level `abort` (per-tool cancel doesn't exist on the wire — the tooltip says so).

Dialogs: inline cards pinned between transcript and composer; composer dims while blocking; keyboard-first (y/n, 1–9, Esc); never OS modals.

Disabled-with-reason everywhere (e.g. "Reconnecting to omp (attempt 2)…", exact server error for fast-mode rejection).

Keymap (final, D4): Cmd/Ctrl+K palette · Cmd/Ctrl+B rail · Cmd/Ctrl+1..9 sessions · Cmd/Ctrl+T/W new/close · Enter / Shift+Enter / Cmd/Ctrl+Enter send · Esc×2 abort · y/n, 1–9 dialogs · PageUp/Down, Home/End transcript.

---

## 9. Edge cases & failure matrix

| Situation | Required behavior |
|---|---|
| Unknown event type / unknown `assistantMessageEvent` variant | `Unknown` row (collapsed raw), log once — never panic/drop |
| Response with no `id` (parse error, unknown cmd) | Log; per-command 30 s client timeout surfaces an error card |
| `prompt` same-id late async error | Pending stays armed until terminal signal; late error → error row, composer re-enabled |
| `agent_end isTerminal:false` | AwaitingResume ("finishing up…"), composer not idle |
| Interrupted/interleaved `rpc_chunk` sequence | Protocol corruption → supervisor restart path |
| `session_busy` / `stale_cursor` on paging | Backoff; retry only when Idle; never merge partial walks |
| Close window while streaming | Confirm card ("abort run and quit?"); graceful shutdown on confirm |
| omp missing / no v2 support / untested version | Launcher card w/ remediation text · hard-fail card · yellow banner, respectively (§1) |
| Crash loop (3 restarts/60 s) | Dead card: stderr tail + exit code + manual Restart; no spawn loop |
| GUI-app stripped PATH (macOS) | Login-shell env capture (§1); if resolution still fails, show the resolved-PATH debug info in the card |
| OAuth `open_url`, browser unavailable | Selectable URL + copy button + instructions text |
| Multi-MB tool output | BoundedText 512 KiB head+tail + elision notice ("full output in session file") |
| Two instances resume one session | OMP owns locks; show its error verbatim; no read-only attach in v1 |
| Repaint pressure from delta storms | Full-drain batching; coalesce to 30–60 Hz if frame >8 ms |
| Wayland popup/coordinate quirks | gpui-component overlay primitives only; explicit Wayland QA |
| IME (CJK) in composer | Verify gpui-component editor InputHandler wiring on every OS — test, don't assume |
| Agent rebuilds the running binary | Prevented structurally by the worktree split (§7.1) |

---

## 10. Testing strategy

1. **Protocol unit tests** (M1): golden frames verbatim from upstream docs; chunk property tests; malformed-input corpus.
2. **Projection replay tests** (M2+): NDJSON fixtures → `insta` snapshots. Every later bug gets a reproducing fixture. Dogfooded features must ship a fixture (§7.4).
3. **Fake-server integration tests:** scripted conversations incl. crash injection, same-id late errors, dialog round-trips, restart-and-resume.
4. **Manual QA checklist** (per release AND per pin bump): streaming smoothness at high token rate; abort/steer/follow-up; keyboard-only dialogs; crash card + restart + resume; scroll behavior (tail follow, pill, paging into history); copy affordances; light/dark; macOS + X11 + Wayland; IME; window restore/multi-monitor.
5. **The dogfood loop is the deepest integration test:** SH-1 is re-run (with a fresh small task) as a release gate for every release after SH.
6. `#[gpui::test]` + `TestAppContext` for entity/pump logic; no pixel tests in v1.

---

## 11. Risks & open questions

1. **GPUI churn (top risk).** Pre-1.0, breaking changes routine. Mitigation: hard pins, upgrade-branch ritual, minimal custom Elements, prefer gpui-component wrappers.
2. **gpui-component ↔ Zed rev coupling.** Pins must be a compatible pair; budget a day per upgrade.
3. **OMP moves fast** and we track the *user's* install, not a bundled one — this makes Doctrine 9 (Unknown-tolerance) and the version-gate banner load-bearing, not defensive. Re-record fixtures per newly supported OMP release; track deltas in `docs/protocol-notes.md`.
4. **Exact payload shapes for `assistantMessageEvent`/`tool_execution_*`** live in `rpc-types.ts`, not fully enumerated in docs — M1 task 2 extracts them early; it gates M2.
5. **Streaming markdown rendering quality** is the biggest UX risk. Prototype gpui-component's Markdown under incremental updates in SH week 1; fallback: custom block renderer (pulldown-cmark IR in pimiento-core + syntect).
6. **No free text selection in GPUI.** Accepted; per-block copy affordances; revisit post-v1 (Zed's implementation is GPL — clean-room only).
7. **Windows deferred**; keep OS-specifics confined to `app.rs`/`discovery.rs`/packaging.
8. **Open:** upstream may grow a first-class revert/authority surface (T4's fork pressure suggests demand) — prefer it over our confirmed-`bash` revert when it lands.
9. **Open:** ACP mode later would let Pimiento host other harnesses; keep `pimiento-core` protocol-agnostic enough that a second client crate is conceivable, but build nothing speculative now (Doctrine 10).

---

## 12. Appendix: sources

- **OMP** (MIT): https://github.com/can1357/oh-my-pi — `docs/rpc.md` (wire contract), `packages/coding-agent/src/modes/rpc/rpc-types.ts` (canonical types), `rpc-frame.ts` (chunk decoder), tests `packages/coding-agent/test/rpc-*.test.ts`; Python reference client `docs/python/omp-rpc/`. Install: https://omp.sh
- **GPUI** (Apache-2.0): https://www.gpui.rs/ · https://docs.rs/gpui · `crates/gpui/src/elements/list.rs` (variable-height list) · `crates/gpui/examples/` (input, uniform_list, list, window). Zed's `ui`/`editor`/`terminal_view`/`agent_ui` are GPL-3.0-or-later: patterns only, never code.
- **gpui-component** (Apache-2.0): https://github.com/longbridge/gpui-component · https://longbridge.github.io/gpui-component/docs/getting-started — 60+ controls incl. Markdown, virtualized list/table, dock, theming, Rope/Tree-sitter editor.
- **T4 Code** (MIT; architecture reference for doctrine, not scope): https://github.com/LycaonLLC/t4-code — `DESIGN.md` (mirror-and-command), `docs/OMP_BRIDGE.md`, `FEATURE_MATRIX.md`.
- **Convergent UX:** Zed agent panel / ACP (https://agentclientprotocol.com/), Claude Code desktop worktree-per-session, opencode client/server split, Conductor/Crystal-style orchestrators.
