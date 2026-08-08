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
