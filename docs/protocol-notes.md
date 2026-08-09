[protocol-notes.md#E3A8]
1:# Pimiento protocol notes
2:
3:Wire extraction baseline: locally installed `omp/17.2.10`. Compatibility has also been exercised against **omp 17.2.11**; the documented tested range is **17.2.10–17.2.11**.
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

Pimiento loads the full `models` array into the composer-band model picker (searchable; no provider shortlist gate). Roles chips peek `~/.omp/agent/config.yml` `modelRoles` (read-only).
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
- `get_state` on OMP 17.2.10 publishes top-level `fastModeEnabled` / `fastModeActive` booleans; Pimiento also still accepts the older nested `fastMode.{enabled,active}` fixture shape.

## Todo panel — 2026-08-08

- `get_state.todoPhases` (also accepted as `todos` / `{phases:[…]}`) is parsed into phase/task rows.
- Status strip **Todos (actionable/total)** toggles a read-only panel above the composer with glyphs for pending/in_progress/completed/blocked/abandoned.

## Multi-session workspace — 2026-08-08

- Root UI is a workspace with a left session rail; each rail entry is its own `SessionView` + supervised OMP child.
- **New** / **Close** manage slots; Cmd/Ctrl+1..9 switches the active session without pausing inactive pumps.
- The rail groups live sessions by their OMP launch cwd (directory basename as the visible workspace heading); ordering is presentation-only and does not alter session authority.

## Workspace context inspector — 2026-08-08

- Cmd/Ctrl+J toggles the right inspector; Cmd/Ctrl+B remains the left session rail shortcut. Toolbar Checklist/Agents actions open and focus their inspector sections.
- Session, model, thinking, context, token speed, fast-mode `{enabled, active}`, todos, subagents, and `dumpTools` names render only from the active session's retained `get_state`/RPC projection.
- `dumpTools` is optional and open-ended; Pimiento tolerantly accepts string entries or object `name`/`id` fields and caps the visible list.
- OMP rpc-ui 17.2.10 does not publish authoritative LSP or MCP connection status, so the inspector explicitly says that status is unavailable rather than inferring it.
- The previous transcript-column todo panel and large agent drawer are no longer rendered; their content now lives in the right inspector.

## Diff review — 2026-08-08

- `edit` / `write` tool results prefer `details.diff` for visible output.
- Expanded tool cards color add/remove/meta lines for quick review (read-only).

## Export HTML + subagents — 2026-08-08

- Status strip **Export** calls `export_html` with a timestamped path under the session cwd and posts a notice with the result path.
- **Agents** opens the right Context inspector's authoritative `get_subagents` snapshot list.
- Compacting / retrying phases show a warning banner above the transcript.

## Thinking collapse + session rename — 2026-08-08

- Collapsed thinking rows expand on click; expanded rows offer collapse.
- Palette **Rename session** opens an About-style floating popover (`popover` + `shadow_xl`) with an Input prefilled from the current projected name; Confirm sends `set_session_name` (Cancel / Esc dismisses).

## Branch new tab + git chrome — 2026-08-08

- Palette **Branch into new tab** calls `get_branch_messages` → floating picker (`entryId` + text preview) → `branch { entryId }`. On success (not cancelled), the current RPC session is the branch; Pimiento sets `pending_new_tab_cwd` so Workspace opens a **fresh** tab via `add_session_for_cwd` (never `switch_session`).
- Palette **Login providers** → `get_login_providers` → floating list → `login { providerId }` (existing `open_url` dialogs handle OAuth).
- Context inspector **Git** section: host-side `probe_git_inspector(cwd)` shows `summary_line` + optional worktree label; omitted when cwd is not a git work tree. Read-only chrome — not OMP authority.
- Inspector Session extras surface `queuedMessageCount` / `messageCount` / token in·out / `cost` from `get_state` raw only when present.
- Checklist tasks are clickable: toggle status via `set_todos { phases }` after carefully mutating `todos_raw` (pending↔completed).
- Connect-time subagent subscription uses `events` (was `progress`) so the Agents hub stays fresher; Refresh still calls `get_subagents`.
- `abort_and_prompt` palette action sends composer draft + pending attachments (same compose path as Prompt), then clears the composer — not bare `abort`.
- `cycle_model` / `cycle_thinking_level` prefer the RPC commands; local catalog walk remains the fallback.

## Confirmed revert + command palette — 2026-08-08

- Edit/write tool cards expose **Revert file…** → confirm card showing `git restore --worktree -- '<path>'` → `bash` RPC.
- Cmd/Ctrl+K opens an in-app command palette (type to filter, Enter runs). Cmd/Ctrl+T/W/B = new/close/toggle rail.
- Tool output JSON is parsed when present so `details.diff` coloring still works for structured results.

## Subagent work modal + transcript paging — 2026-08-08

- `get_subagents` returns `{ subagents: RpcSubagentSnapshot[] }`; the Context inspector renders each authoritative snapshot tolerantly, preserving raw values rather than assuming every optional field exists.
- An unseen `subagent_lifecycle`, `subagent_progress`, or `subagent_event` id triggers one guarded `get_subagents` refresh, so an already-open inspector discovers newly spawned agents without manual refresh or duplicate in-flight requests.
- Subagents have one workspace representation: summary rows in the inspector's **Agents** section. Clicking a row opens a centered work modal backed by `get_subagent_messages`; message tails are not duplicated inline in the narrow inspector or in a strip above the transcript.
- `get_subagent_messages { subagentId?, sessionFile?, fromByte? }` returns `nextByte`, which is passed back when the same agent is opened again for incremental tailing. A `reset: true` response replaces the locally displayed tail before its page is applied. The modal remains visible while loading and is dismissed by backdrop click, **Close**, or Esc.
- PageUp/PageDown/Home/End navigate the transcript `ListState`; PageUp/PageDown/Home leave tail-follow mode and End re-enables it at the transcript tail. Keys are ignored while the composer or model-search input is focused so Home/End still move the caret.

## Rich slash-command discovery — 2026-08-09

- OMP 17.2.11 publishes slash commands dynamically through both `available_commands_update.commands` and `get_available_commands.data.commands`. Entries may include `name`, `aliases`, `description`, `input.hint`, `source`, and nested `subcommands[] { name, description, usage? }`.
- Pimiento treats this metadata as an open catalog rather than a fixed enum. Completion supports top-level aliases and a second subcommand stage while preserving unknown future `source` values. Selecting a suggestion only fills the composer; execution still requires an explicit send so destructive extension/custom commands are never triggered by menu navigation.

## Named themes and live registry — 2026-08-09

- The pinned Apache-2.0 `gpui-component` exposes `ThemeSet`, `ThemeConfig`, `ThemeRegistry::load_themes_from_str`, `ThemeRegistry::watch_dir`, and `Theme::apply_config`. Pimiento uses that public registry directly for bundled and user-authored JSON themes; no Zed GPL UI code is copied.
- Theme persistence mirrors Zed's useful behavior at a smaller scope: one appearance mode (`system`, `light`, or `dark`) plus separately selected light/dark theme names. Existing `ui.json` files containing only the legacy `theme` field migrate through serde defaults.
- Compatible custom theme files under `PIMIENTO_HOME/themes` (normally `~/.pimiento/themes`) reload without restarting. Invalid files are ignored by the registry and cannot replace the bundled Quiet Pepper fallback pair.
- The bundled picker includes all 11 theme variants shipped in the pinned Zed asset catalog (One, Ayu, and Gruvbox families), all four Catppuccin flavors, and Dracula. Zed theme assets are MIT-licensed and converted into the Apache-2.0 `gpui-component` schema; attribution is recorded in `docs/third-party-theme-notices.md`.
- `IconName` only resolves SVG paths; GPUI still needs an `AssetSource`. Applications using the bundled icon set must start with `gpui_platform::application().with_assets(gpui_component_assets::Assets)`. Without that registration, icon-only buttons keep their click targets and tooltips but paint blank.

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

## Per-model thinking controls — 2026-08-08

- `get_available_models` returns `ModelInfo` entries whose optional `thinking.efforts` array is the authority for each model's controllable effort levels. Pimiento preserves catalog order and offers only `off` + advertised efforts + `auto`, with duplicates removed.
- Missing/null `thinking` or an empty `efforts` array means the model has no controllable thinking surface, so Pimiento hides the thinking picker. OMP still exposes `set_thinking_level` / `cycle_thinking_level` and remains responsible for clamping wire commands.
- The status strip keeps `status_message` for the OMP version, connection state, errors, and transient notices only. Model/thinking controls and the quiet context/tokens readouts render separately, so runtime facts are not duplicated.

## Pristine UI polish — 2026-08-08

- Transcript-level `Copy` actions are hidden until their semantic row is hovered; Markdown code-block copy remains visible because it is contextual to the block.
- The toolbar and inspector shorten `cursor/<model-id>` to `<model-id>` for display; the OMP-published value remains authoritative in projection state.
- Running tool elapsed time is disposable display state keyed by the OMP tool-call ID. A foreground GPUI task repaints once per second while any projected tool is running; completion duration remains OMP-authoritative when published.
- Inspector visibility is stored as `{ "inspector_open": bool }` in `ui.json` under the existing Pimiento persistence root. Missing or malformed state defaults to open.

## Reveal logs + inspector density — 2026-08-08

- Command-palette **Reveal logs** opens the existing Pimiento persistence root (`~/.pimiento` or `PIMIENTO_HOME`) with `open` on macOS or `xdg-open` on Linux; `scripts/reveal_logs.sh` provides the same terminal dogfood action without reading or printing stored data.
- The inspector keeps the authoritative `ctx:N%` text and adds a thin progress bar for the same clamped `contextUsage.percent` value.
- Inspector tool chips default collapsed when OMP reports more than eight tools; **Tools (N)** expands the existing capped chip list.

## Rail attention + window title — 2026-08-08

- Rail dots use only projected state: info for Streaming/AwaitingResume/Compacting/Retrying, warning for unread transcript rows while inactive, and no dot otherwise.
- The primary window title follows the active session's OMP-published name (cwd fallback) and projected run phase. GPUI's `Window::set_window_title` is called only when the computed title changes.
- The window title omits the session-name segment when the authoritative name is empty or exactly matches the product name, avoiding `Pimiento — Pimiento`.
- The composer Abort action was already always visible for abortable phases and danger-styled; the unread-tail pill was already primary-styled as `N new ↓`.

## D4 launcher and dialog polish — 2026-08-08

- The command palette exposes an in-app **About Pimiento** notice with the discovered `omp --version` output when connected; this is disposable display state only.
- Launcher and inline-dialog changes are presentation-only. Dialog responses, Esc cancellation, and `open_url` copy/open behavior retain their existing RPC shapes.
- App sources were modularized for maintainability without changing protocol or runtime behavior.
- Visual-hierarchy polish (rail affordance, transcript spacing, inspector emphasis, and empty-state copy) changes presentation only; all displayed session facts remain OMP-authoritative.
- Versions outside the documented tested range `omp 17.2.10–17.2.11` should proceed with a warning banner; the exact discovered semantic version comes from startup discovery.

## D4 light-theme transcript audit — 2026-08-08

- Transcript and workspace surfaces now use gpui-component semantic theme tokens for overlays, sidebars, elevated rows, and paired warning/danger foregrounds. This was a source audit; light and dark visual QA remain manual.

## D4 IME composition — 2026-08-08

- Pinned `gpui-component` Input tracks composition privately (`ime_marked_range`) and `submit_on_enter` still emits `InputEvent::PressEnter` without a public `is_composing` / marked-range API.
- Pimiento therefore cannot add an app-level Enter-while-composing guard without forking or waiting for an upstream Input API. Manual CJK IME QA remains required on each OS; document failures against the pin rev rather than inventing a client-side composition state.

## Theme preference env — 2026-08-08

- `PIMIENTO_THEME=system|light|dark` selects the initial theme preference at process start (default `system`). Palette **Toggle theme** still cycles at runtime. This is disposable display state only.
- Theme choices from the palette persist in `ui.json`; `PIMIENTO_THEME` overrides for the process without rewriting that file.
- Without that environment override, startup reads `theme` from `ui.json`; palette changes write it immediately, while an environment override never overwrites the stored preference.
- The collapsed session rail now keeps a full-height 64 px restore target labeled **Show Sessions** with a muted `⌘B` hint.

## About keymap card — 2026-08-08

- The About overlay includes a compact keymap cheat-sheet (palette, sessions rail, inspector, new/close, send, abort). Presentation-only.

## Quiet Pepper Console — 2026-08-09

- Brand tokens live in `crates/pimiento-app/src/tokens.rs` (paprika identity / ember action). `apply_pimiento_brand` remaps gpui-component `primary`/`ring`/`accent` after every theme preference change.
- Status pills use a fixed taxonomy (working / awaiting / busy / idle / error) via `Tag::custom`, not brand paprika.
- Composer band uses `shadow_lg` as the sole docked elevation exception (parity-plan Wave U2).
- Inspector **Queue** section writes `set_steering_mode` / `set_follow_up_mode` / `set_interrupt_mode` / `set_auto_compaction` / `set_auto_retry`. Promoted `RuntimeState` fields hydrate from `get_state` camelCase keys; optimistic UI reverts on RPC failure.
- `queuedMessageCount` surfaces as a composer `queue:N` chip when > 0 and in inspector extras.
- Wave A session controls use existing typed RPC shapes: `compact { customInstructions? }`, `get_session_stats`, and `handoff { customInstructions? }`. Empty compact input omits `customInstructions`; stats and handoff response data render losslessly as pretty JSON notices.
- **Fresh session** deliberately sends the dynamic slash command `/fresh` through `prompt` while idle; it does not invent a client-owned session reset.
- An idle `contextUsage.percent >= 80` shows a soft compact CTA. The threshold is display policy only; the percentage remains OMP-authoritative.



## Rail persistence + streaming polish — 2026-08-08

- Session-rail visibility is disposable layout state stored as `{ "rail_collapsed": bool }` in `ui.json`; missing state defaults to expanded.
- An empty projected assistant row renders a quiet muted ellipsis only while its wire-derived `streaming` flag is true. The cue contains no inferred assistant content and does not duplicate the toolbar phase.
- The empty transcript presents one short sentence without key-command tips.
- Completed thinking rows with empty text render as zero-height placeholders so list indices stay stable without showing a hollow "Thinking · expand" control.
- Info notices that look like tool/MCP mount chatter stay fully visible but drop the `Notice` label and render slightly quieter — text remains OMP-authoritative.
- The context inspector header includes a ghost **Hide** control (alongside the existing `⌘J` hint) that toggles the same disposable `inspector_open` preference.
- When the inspector is closed, a full-height 64 px **Show Context** strip mirrors the collapsed session-rail restore affordance.
- While a run is abortable/streaming, the primary composer button reads **Steer** (same `steer` RPC path as Enter); **Follow-up** remains a separate control.
- Empty transcript copy is a short brand-ready orientation pair (no keymap wall).

## Export reveal — 2026-08-08

- After a successful `export_html`, Pimiento reveals the written file in the platform file manager (`open -R` on macOS; parent folder via `xdg-open` on Linux). The transcript notice still carries the authoritative path from OMP.

## Toolbar deference when inspector open — 2026-08-08

- While the Context inspector is visible, the session toolbar hides the duplicated Checklist/Agents shortcuts and ctx%/tps readouts (those facts already live in the inspector). Model/thinking/Fast live on the composer band (not the status strip). Presentation-only.

## Thinking collapse preview — 2026-08-08

- Collapsed thinking rows show a truncated first non-empty line from the wire thinking text (not a fabricated summary). Empty completed thinking remains a zero-height placeholder.

## Dialog / dead chrome polish — 2026-08-08

- Pending extension-UI dialogs dim the composer row and disable Send/Steer with reason "Answer the dialog above first".
- Dead sessions keep Restart only on the crash card (not duplicated on the composer). Crash Copy copies the full displayed detail (dead_reason + status).
- Running tool cards note that cancel is turn-level Abort only.
- Empty Checklist inspector shows "No checklist items yet". About keymap lists session digits and transcript paging.

## Palette theme label — 2026-08-08

- Command palette Theme entry shows the current preference (`Theme: Light` etc.) and documents the cycle order. Presentation-only.

## Toolbar phase Tag — 2026-08-08

- The status strip renders the projected run phase as a small gpui-component `Tag` (info/warning/danger/secondary) beside the OMP version, matching rail phase badges.

## UI depth + composer band — 2026-08-08

- **Depth recipe (Zed / gpui-component Apache crates):** docked chrome (rail, composer band, inspector) = tone step (`sidebar` / `secondary`) + hairline border, no shadow. Floating overlays (model picker, command palette, About, slash menu) = `popover` fill + border + radius + `shadow_lg` / `shadow_xl` over `overlay` scrim. Prefer pinned `gpui` / `gpui_platform` / `gpui-component` checkouts under `~/.cargo/git/checkouts/` for API truth; Zed `ui` / `editor` / `agent_ui` are GPL — patterns only, never copy source.
- **Session rail:** selected row is `sidebar_accent` wash + rounded — no primary accent bar. Hover × closes the in-app tab and `forget_session` from Pimiento `recent.json` — does **not** delete OMP files under `~/.omp`. Per-workspace `+` reconnects that cwd; top **Workspace…** is a directory picker → new tab without full-screen launcher when possible.
- **Composer band:** model + thinking + Fast `Switch` live immediately above the input (removed from the status-strip trailing cluster). Model opens a floating picker with search, provider-grouped list, and **Roles (from omp config)** chips.
- **`modelRoles` / `modelTags`:** not on rpc-ui. Pimiento peeks `~/.omp/agent/config.yml` for roles + tag colors (built-in OMP colors for default/smol/slow/…). Click a role to `set_model`. **Set** assigns the current session model to that role via `omp config set modelRoles` (full-record merge — never hand-edit YAML).
- **Fast mode:** OMP `/fast` only works for models with a service-tier family (OpenAI / Google / Anthropic-messages / matching OpenRouter ids). Cursor/Grok has none — Switch is disabled with `n/a · no service tier`. Failed `set_fast_mode` reverts optimistic UI and shows OMP's error as a notice.
- **Composer attachments (OMP parity):** Attach / DnD / clipboard accept **any file**. Images → `PendingAttachment::Image` with OMP resize (max edge 1568, min edge 200, ~500 KiB raw target; keep original png/jpeg/gif/webp when already under budget; honor `images.autoResize`, default true; else wire-budget clamp ~700KB b64). Non-images → `PathMention` inserting `@path` into the message (rpc-ui has no document content type). Message text carries `[Image #N, WxH]` markers via `compose_message_with_image_markers`. Wire images only: `{type:"image", mimeType, data}` on `prompt` / `steer` / `follow_up`. Large paste (≥ `paste.largeMenuThreshold`, default 100) offers Wrap `<attachment>` / `local://paste-N.md` under `{session.jsonl−suffix}/local/` / Inline. `@` token opens a capped cwd file picker. Hydrated `role: fileMention` messages stay `TranscriptEntry::Unknown` and render as a quiet “File mention: …” row. Send errors (including `FrameTooLarge`) surface in the transcript.
- **Command palette:** focused search `Input` (typing works); Esc / arrows handled on the outer capture layer (overlay is a sibling of the old capture tree — that was why Esc/typing felt dead). Panel uses `popover` + `shadow_xl`.
- **Inspector:** omit empty Checklist/Tools; Fast lives on the composer band only; no LSP/MCP footer (rpc-ui does not publish those statuses).

## Extension UI confirm / cancel / timeout — 2026-08-08

- Confirm responses must use `confirmed: true|false` (not `accepted`). OMP rpc-types (`RpcExtensionUIResponse`) and record-fixture ask-dialog replies both use `confirmed`.
- Cancel / Esc / dialog expiry use `cancelled: true`, with `timedOut: true` when the client fires the wire `timeout` ms locally.
- `input` / `editor` responses carry `value: string`. Pimiento renders a text field + Submit (and Cancel) for those methods; Cancel alone is not enough.
- Non-dialog display methods (`setTitle` / `setStatus` / `setWidget` / `set_editor_text`) project into `DisplayState` and surface in the status strip / inspector only when present. OS window title follows `setTitle` only when `PI_RPC_EMIT_TITLE` is truthy (OMP's emit gate).
- `image_end` projects to a visible `TranscriptEntry::Notice` with mime/type summary so image blocks are never dropped silently.

## Wave E3 upstream IME API ask — 2026-08-09

- **Blocked upstream:** Pimiento needs gpui-component `InputState::is_composing()` (or an equivalent composition flag on the submit event) so Enter can commit an active IME candidate without also sending the composer.
- The pinned component keeps `ime_marked_range` private, so Pimiento cannot implement a reliable app-level guard without a fork. Do not infer composition state from key timing or text changes.
- No upstream completion is claimed here. Until the public API lands and the pin is updated, CJK IME remains required manual QA on macOS, Linux X11, and Linux Wayland.

## Quiet Pepper execute waves — 2026-08-09

- Wave 0 documentation now reflects image attachments as shipped, records the **omp 17.2.10–17.2.11** tested range, and separates D1–D4 code completion from still-open dogfood and platform QA.
- Wave E adds a minimal Linux personal-dogfood release tarball path. AppImage, `.deb`, signing, and Windows remain deferred; Pimiento still neither bundles nor installs `omp`.
- Wave U5 has a dedicated motion and screenshot checklist in `docs/quiet-pepper-qa.md`; durations and reduced-motion behavior are recommendations until implemented and manually verified.
- SH proof templates live in `docs/sh-proofs.md`. No SH proof is complete merely because its slot exists; Linux is environment-ready pending live dogfood, and macOS requires the user's machine.
- This entry summarizes the Quiet Pepper execute integration change set; it does not assert a hosted PR or completed QA in this local-only repository.

## Experimental host tool bridge — 2026-08-09

- `PIMIENTO_HOST_BRIDGE=1` registers `pimiento.open_file` with `set_host_tools`; every other value leaves the bridge off and sends no registration.
- OMP 17.2.11 expects `{name,label?,description,parameters,hidden?,loadMode?}` definitions. Results are fire-and-forget `host_tool_result` frames whose `result` is an `AgentToolResult` such as `{content:[{type:"text",text:"…"}]}`; failures also set top-level `isError:true`.
- Host calls remain visible as Unknown transcript rows and additionally become foreground-owned approval cards. Denial is the default; approval requires an existing absolute file path. `host_tool_cancel.targetId` dismisses pending work and suppresses late results.
- No host URI scheme is registered yet. Unexpected URI requests remain visible Unknown rows and additionally get a deny-only card; `host_uri_cancel` dismisses that pending card and suppresses late results.

## Rail density + Wave D surfaces — 2026-08-09

- Rail rows and workspace headers derive status pills only from each session's projected `RunPhase`; workspace rollup priority is dead/error, awaiting input, active/busy, then idle.
- The inspector Agents section renders retained `get_subagents` snapshots and cycles the existing `set_subagent_subscription` wire levels `off → progress → events`; subscription events do not create a second list or transcript strip.
- Palette **Share session** sends a regular `prompt` carrying `/share`; Pimiento does not infer or persist a share URL and waits for OMP output.
- Inspector tool grouping is display-only: a fixed known-builtin allowlist is labeled **Builtin**, and all unknown names remain visible under **Extensions / MCP**. Computer/browser/vision tags are best-effort presence indicators from `dumpTools` names or display-widget keys, not connection-health claims.

## Hub / task / bash tool polish — 2026-08-09

- `hub` tool cards render a Jobs summary only from fields present on args/results (`op`, job id, status, command, or a `jobs[]` array). Unparseable payloads keep the generic card.
- `task` cards show **Open agents** only when a named `subagentId`/`toolCallId` linkage field is present; the button opens the existing Agents inspector focus and does not invent linkage. `eval` titles/digests use only supplied title, language, and code.
- `abort_bash` is targetless (`{type:"abort_bash"}`), while transcript bash rows are identified by `tool_execution_* .toolCallId`; no safe correlation exists, so no per-card Abort button renders.

## Discoverable slash commands + theme preview — 2026-08-09

- The main command palette combines its static actions with every top-level command from OMP's open `available_commands` catalog. Search also flattens matching nested subcommands and considers names, descriptions, input/usage hints, source strings, and aliases.
- Choosing a slash entry reuses the composer's completion text (including safe trailing spaces for required input), focuses the composer, and never sends. Slash entries use variable-height rows so source, aliases, descriptions, and usage metadata cannot overlap.
- The named theme picker snapshots the exact appearance/light/dark selection on open. Highlight changes preview a selection in memory only; Enter/click persists, while Esc, backdrop, and Close restore the opening snapshot. Each preview is derived from that snapshot so moving from a named preview back to an appearance row also restores the original paired theme names.

## Native slash parity + palette focus — 2026-08-09

- OMP 17.2.11 keeps `/fork` in the TUI-only command surface; it is not published by `rpc-ui` command discovery. Pimiento lists `/fork` as a native palette alias for its authoritative `get_branch_messages` → `branch { entryId }` picker, rather than fabricating slash-command RPC support.
- A small static native-slash catalog covers only TUI names that map honestly to existing Pimiento actions. It is additive to the dynamic `available_commands` catalog, and a same-name dynamic command is deduplicated. Native entries run their mapped GUI action immediately; dynamic OMP entries remain safe composer insertions whose metadata says Enter is still required to send.
- Command and theme rows use one shared mouse/keyboard selected index, so hover moves the keyboard focus instead of painting a competing highlight. Theme hover previews immediately; click/Enter commits, while Esc or backdrop still restores the opening theme snapshot.
- The command palette uses a responsive 80%-height panel capped at 680 px with a flexing scroll region and non-shrinking variable-height rows. Theme selection remains available through the dedicated `Theme…` palette action; the duplicate toolbar Theme button and duplicate top-strip widget rendering were removed. Projected widgets remain visible in the Context inspector Display section.
