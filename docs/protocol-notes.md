[protocol-notes.md#E3A8]
1:# Pimiento protocol notes
2:
3:Authoritative development target: locally installed `omp/17.2.10`.
4:
5:Canonical sources inspected on 2026-08-06:
6:
7:- `~/.bun/install/cache/@oh-my-pi/pi-coding-agent@17.2.10@@@1/src/modes/rpc/rpc-types.ts`
8:- `~/.bun/install/cache/@oh-my-pi/pi-coding-agent@17.2.10@@@1/src/modes/rpc/rpc-frame.ts`
9:- `~/.bun/install/cache/@oh-my-pi/pi-agent-core@17.2.10@@@1/src/types.ts`
10:- `~/.bun/install/cache/@oh-my-pi/pi-ai@17.2.10@@@1/src/types.ts`
11:
12:## Transport v2
13:
14:`ready` is protocol v1 and advertises `[1, 2]`, `maxFrameBytes`, and `maxReassembledFrameBytes`. Negotiate v2 before relying on chunking. Physical frames are newline-delimited UTF-8 JSON objects and are limited to 1 MiB including the newline. Logical reassembled frames are limited to 64 MiB. Server chunk payloads are 256 KiB before base64.
15:
16:`rpc_chunk` fields are `{type, chunkId, index, count, byteLength, data}`. Installed OMP additionally validates: nonempty `chunkId` of at most 128 characters; `count >= 2`; `byteLength >= 1 MiB`; strict canonical base64; each decoded payload at most 256 KiB; uninterrupted, matching, contiguous sequence starting at index 0; exact declared decoded byte length; strict UTF-8; final JSON must be an object. A non-chunk frame during a pending sequence is protocol corruption.
17:
18:Oversized responses become a failed `response` with error `RPC response exceeded the transport limit`. Other oversized frames may become `rpc_frame_error`; `agent_end` may be compacted before fallback.
19:
20:## `message_update.assistantMessageEvent`
21:
22:Exact discriminated variants in 17.2.10:
23:
24:- `start { contentIndex?: undefined, partial: AssistantMessage }`
25:- `text_start { contentIndex: number, partial: AssistantMessage }`
26:- `text_delta { contentIndex: number, delta: string, partial: AssistantMessage }`
27:- `text_end { contentIndex: number, content: string, partial: AssistantMessage }`
28:- `thinking_start { contentIndex: number, partial: AssistantMessage }`
29:- `thinking_delta { contentIndex: number, delta: string, partial: AssistantMessage }`
30:- `thinking_end { contentIndex: number, content: string, partial: AssistantMessage }`
31:- `image_end { contentIndex: number, content: ImageContent, partial: AssistantMessage }`
32:- `toolcall_start { contentIndex: number, partial: AssistantMessage }`
33:- `toolcall_delta { contentIndex: number, delta: string, partial: AssistantMessage }`
34:- `toolcall_end { contentIndex: number, toolCall: ToolCall, partial: AssistantMessage }`
35:- `done { contentIndex?: undefined, reason: "stop" | "length" | "toolUse", message: AssistantMessage }`
36:- `error { contentIndex?: undefined, reason: "aborted" | "error", error: AssistantMessage }`
37:
38:Every `message_update` also carries `message: AgentMessage`. `partial`, `message`, `toolCall`, image content, and completed/error messages must remain lossless JSON values at the RPC boundary: provider payloads and future content block fields are open-ended. Pimiento models the stable discriminant and scalar delta/index fields while retaining raw values and an Unknown fallback.
39:
40:`ToolCall` stable fields are `{type:"toolCall", id, name, arguments}` with optional `thoughtSignature`, `intent`, `rawBlock`, `customWireName`, and provider metadata. Do not assume arguments arrive complete before `toolcall_end`.
41:
42:## Tool execution lifecycle
43:
44:Exact 17.2.10 shapes:
45:
46:- `tool_execution_start { type, toolCallId: string, toolName: string, args: any, intent?: string }`
47:- `tool_execution_update { type, toolCallId: string, toolName: string, args: any, partialResult: any }`
48:- `tool_execution_end { type, toolCallId: string, toolName: string, result: any, isError?: boolean }`
49:
50:The extension-facing type documents `isError: boolean`, but the core `AgentEvent` wire source marks it optional. Decode absence as unknown/false only in projection policy; preserve absence in the protocol model. `args`, `partialResult`, and `result` are arbitrary JSON and must never be narrowed or dropped.
51:
52:## Other 17.2.10 deltas that affect M1
53:
54:- `tool_execution_start.intent` is optional and should be preserved.
55:- `agent_end` may include optional `telemetry`, `coverage`, and session-added `isTerminal`.
56:- `turn_end` carries both `message` and `toolResults`.
57:- `thinking_level_changed` carries `thinkingLevel?`, optional configured selector, and optional resolved effort.
58:- `notice` is `{level:"info"|"warning"|"error", message, source?}`.
59:- Extension UI `editor` has optional `prefill` and `promptStyle`; `setStatus.statusText` and `setWidget.widgetLines` may be absent; `open_url` may include `launchUrl` and `instructions`.
60:- Host tool `partialResult`/`result` are structured `AgentToolResult`, not strings. Host URI result content type is restricted to markdown, JSON, or plain text.
61:- Unknown top-level frames and unknown discriminated variants must retain and render raw JSON.
62:
63:## Model wire shapes (OMP 17.2.10) — 2026-08-07
64:
65:- `get_state` returns the session state object directly as `data` (not wrapped in `{state:…}`).
66:- `state.model` is a **Model object** `{ provider, id, … }`, not a string. Pimiento formats it as `provider/id` for display and `set_model`.
67:- `get_available_models` → `{ models: Model[] }`.
68:- `set_model { provider, modelId }` success `data` is the Model object.
69:- `model_changed` is a **bare** event `{ type: "model_changed" }` with **no** model payload. Clients must refresh via `get_state` or trust the preceding `set_model` response.

Pimiento loads the full `models` array into the status-strip picker (searchable; no provider shortlist gate).
70:

## Model catalog loading — 2026-08-08

- `get_available_models` can exceed the default 30s RPC timeout; Pimiento loads it asynchronously after window open with a 180s timeout.
- Outbound commands are flat NDJSON objects (`{id,type,provider,modelId}`), never wrapped in `params`.
- Session model picker: searchable full catalog + freeform `provider/id` Enter fallback.

## Session durability (SH) — 2026-08-08

- Live app now spawns durable OMP sessions (`no_session: false`) with `cwd` from `PIMIENTO_CWD` or process cwd.
- `get_state.sessionFile` is remembered at `~/.pimiento/last-session` and passed as `--resume` on relaunch/Restart.
- Stale resume pointers fall back to a fresh durable session.
- Composer Enter sends `steer` while `RunPhase::Streaming`, otherwise `prompt`.

## contextUsage / tokensPerSecond — 2026-08-08

- `get_state` reports `contextUsage: { tokens, contextWindow, percent }` (not `context`).
- `tokensPerSecond` is a top-level scalar (often null until a turn is streaming).
- Pimiento promotes these into `RuntimeState.context` / `.tokens` for the status strip (`ctx:N%`, `N/s`).

## Session launcher — 2026-08-08

- The launcher opens before spawning OMP; its working directory comes from the picker or an existing recent pointer, with `PIMIENTO_CWD` as the initial override.
- Recent pointers live in `recent.json` as `{sessionFile, cwd, name, lastUsed}`; `PIMIENTO_AUTO_CONNECT=1` preserves the one-shot eager-connect escape hatch.
- `PIMIENTO_HOME` overrides the `~/.pimiento` directory used for `last-session` and `recent.json`.
- Status strip **Sessions** returns to the launcher (drops the current child) so you can pick another cwd/session.
- **Choose directory…** only updates the launcher cwd; it does not auto-start. Use **Start here** or a session row.
- Session list for the selected cwd merges `recent.json` with on-disk OMP sessions under `~/.omp/agent/sessions/<encoded-cwd>/*.jsonl` (or `$PI_CODING_AGENT_DIR/sessions/...`), using session title / first user prompt for labels.

## Message history hydration — 2026-08-08

- Connect/resume hydrates history via paginated `get_messages_page` (`limit` 100, follows `nextCursor`, capped pages).
- `session_busy` retries briefly; `stale_cursor` clears partial history and restarts paging once; hard failures fall back to legacy `get_messages`.
- Page payloads are `{ messages, nextCursor?, totalMessages }`; message content may be a string or typed parts. `toolResult` rows pair to `toolCall` by `toolCallId`.

## Assistant code block copy — 2026-08-08

- Assistant Markdown code blocks expose a small `Copy` action that writes the rendered code to the host clipboard.
- Copy button IDs hash the transcript row index, language, and code so repeated blocks remain distinct.

## Slash command completion — 2026-08-08

- Pimiento parses `available_commands_raw` as either a command array or `{commands: []}`. Slash names and aliases are normalized with a leading `/`; aliases filter to and complete the primary command name.
- The composer opens a capped, prefix-filtered slash menu for slash-only drafts; Enter or a click completes locally, while Enter sends only when there are no menu matches.

## Interactive model controls — 2026-08-08

- The status strip exposes `set_thinking_level {level}` for the configured levels `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `auto`; Pimiento refreshes `get_state` after the command so the rendered selector remains OMP-authoritative.
- `set_fast_mode {enabled}` returns `{enabled, active}`. These values describe configured and currently active fast mode and may diverge, so the status strip labels `fast:off`, `fast:on`, or `fast:active` rather than collapsing them into one boolean.

## Todo panel — 2026-08-08

- `get_state.todoPhases` (also accepted as `todos` / `{phases:[…]}`) is parsed into phase/task rows.
- Status strip **Todos (actionable/total)** toggles a read-only panel above the composer with glyphs for pending/in_progress/completed/blocked/abandoned.

## Multi-session workspace — 2026-08-08

- Root UI is a workspace with a left session rail; each rail entry is its own `SessionView` + supervised OMP child.
- **New** / **Close** manage slots; Cmd/Ctrl+1..9 switches the active session without pausing inactive pumps.

## Diff review — 2026-08-08

- `edit` / `write` tool results prefer `details.diff` for visible output.
- Expanded tool cards color add/remove/meta lines for quick review (read-only).

## Export HTML + subagent strip — 2026-08-08

- Status strip **Export** calls `export_html` with a timestamped path under the session cwd and posts a notice with the result path.
- **Agents (n)** toggles a read-only drawer of recent `subagent_*` payloads already retained on the projection.
- Compacting / retrying phases show a warning banner above the transcript.

## Thinking collapse + session rename — 2026-08-08

- Collapsed thinking rows expand on click; expanded rows offer collapse.
- Status strip **Rename** calls `set_session_name` with a stamped label (modal rename can come later).

## Confirmed revert + command palette — 2026-08-08

- Edit/write tool cards expose **Revert file…** → confirm card showing `git restore --worktree -- '<path>'` → `bash` RPC.
- Cmd/Ctrl+K opens an in-app command palette (type to filter, Enter runs). Cmd/Ctrl+T/W/B = new/close/toggle rail.
- Tool output JSON is parsed when present so `details.diff` coloring still works for structured results.

## Subagent drawer tail + transcript paging — 2026-08-08

- `get_subagents` returns `{ subagents: RpcSubagentSnapshot[] }`; the Agents drawer renders each snapshot tolerantly, preserving raw values rather than assuming every optional field exists.
- `get_subagent_messages { subagentId?, sessionFile?, fromByte? }` returns `nextByte`, which is passed back on refresh for incremental tailing. A `reset: true` response replaces the locally displayed tail before its page is applied.
- PageUp/PageDown/Home/End navigate the transcript `ListState`; PageUp/PageDown/Home leave tail-follow mode and End re-enables it at the transcript tail. Keys are ignored while the composer or model-search input is focused so Home/End still move the caret.

## Compaction and retry fallback UX — 2026-08-08

- `retry_fallback_applied` carries `{ from, to, role }`; `retry_fallback_succeeded` carries `{ model, role }`. Pimiento reads these only from the retained raw event JSON, emits human-readable retry rows, and omits missing values without guessing.
- An applied fallback sets a wire-derived sticky banner (`Using fallback model …`); OMP's fallback-success or retry-end event clears it. `auto_retry_*` attempt counts are rendered only when supplied by the raw event.
- `auto_compaction_start/end` render as `Compacting…` and `Compaction complete`, rather than debug enum names.

## Transcript copy affordances — 2026-08-08

- Error, notice, command-output, unknown-frame, and expanded tool rows expose small copy actions using only their projected text or JSON fields.
- User rows expose the same ghost `Copy` action and copy only the submitted user text.
- Assistant rows expose a ghost `Copy` action that copies the projected Markdown source; per-code-block copy remains available.
- Thinking rows (collapsed and expanded) expose a ghost `Copy` action that copies only the projected thinking `text`.

## Esc abort arming — 2026-08-08

- When OMP's run phase allows abort, the first unmodified Esc shows `Press Esc again to abort`; a second Esc within 1.2 seconds sends the existing `abort` RPC. Palette, dialog, and slash-menu Esc handling runs first, so a lone Esc continues to dismiss those surfaces.

## D4 composer keymap and theme override — 2026-08-08

- `InputEvent::PressEnter.secondary` is gpui-component's `secondary-enter` binding (Cmd+Enter on macOS, Ctrl+Enter elsewhere). It bypasses slash completion and follows the existing prompt/steer send path. Shift+Enter remains the component-provided multiline newline path.
- Pimiento defaults to `Theme: System` and observes subsequent window appearance changes. The status button and command-palette theme action share a three-state `System → Light → Dark → System` cycle; Light/Dark are manual overrides, while returning to System synchronizes immediately and resumes following the OS.

## D4 close-while-streaming confirmation — 2026-08-08

- A primary-window close request is blocked while any workspace session is Streaming, AwaitingResume, Compacting, or Retrying. A workspace-level confirm card accepts Yes/No, y/n, and Esc=No; confirmation sends `abort` to every abortable session, half-closes all child stdin streams for graceful shutdown, then quits the app.

## D4 semantic rows and platform QA — 2026-08-08

- Transcript presentation is semantic rather than chat-like: user entries are full-width plain rows with a theme-accent left border; assistant entries remain plain full-width rows. User text is no longer rendered in a filled bubble.
- The composer uses gpui-component `Input`. For macOS CJK IME QA, enable a CJK input source, start composing in the composer, and press Enter while the candidate/composition UI is active: Enter must confirm composition without sending. After composition is committed, a subsequent Enter must send exactly once. This remains a manual verification; Linux/Wayland IME is still an outstanding checklist item.
- Popups and overlays must use gpui-component primitives so placement remains backend-owned. Wayland and X11 popup behavior both remain outstanding Linux QA on this macOS dogfood machine.

## D4 window bounds — 2026-08-08

- The primary window's last windowed bounds are stored at `window.json` under `~/.pimiento` (or `PIMIENTO_HOME`) and restored on the next launch. Maximized and fullscreen bounds are not persisted.
- Persisted width and height are clamped to 480×320 logical pixels; malformed or non-positive bounds are ignored.
- Multi-monitor caveat: when a saved origin is no longer on-screen, GPUI or the host OS may adjust placement. Pimiento v1 deliberately does not invent display-coordinate correction logic.
