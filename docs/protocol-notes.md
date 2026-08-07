# Pimiento protocol notes

Authoritative development target: locally installed `omp/17.2.10`.

Canonical sources inspected on 2026-08-06:

- `~/.bun/install/cache/@oh-my-pi/pi-coding-agent@17.2.10@@@1/src/modes/rpc/rpc-types.ts`
- `~/.bun/install/cache/@oh-my-pi/pi-coding-agent@17.2.10@@@1/src/modes/rpc/rpc-frame.ts`
- `~/.bun/install/cache/@oh-my-pi/pi-agent-core@17.2.10@@@1/src/types.ts`
- `~/.bun/install/cache/@oh-my-pi/pi-ai@17.2.10@@@1/src/types.ts`

## Transport v2

`ready` is protocol v1 and advertises `[1, 2]`, `maxFrameBytes`, and `maxReassembledFrameBytes`. Negotiate v2 before relying on chunking. Physical frames are newline-delimited UTF-8 JSON objects and are limited to 1 MiB including the newline. Logical reassembled frames are limited to 64 MiB. Server chunk payloads are 256 KiB before base64.

`rpc_chunk` fields are `{type, chunkId, index, count, byteLength, data}`. Installed OMP additionally validates: nonempty `chunkId` of at most 128 characters; `count >= 2`; `byteLength >= 1 MiB`; strict canonical base64; each decoded payload at most 256 KiB; uninterrupted, matching, contiguous sequence starting at index 0; exact declared decoded byte length; strict UTF-8; final JSON must be an object. A non-chunk frame during a pending sequence is protocol corruption.

Oversized responses become a failed `response` with error `RPC response exceeded the transport limit`. Other oversized frames may become `rpc_frame_error`; `agent_end` may be compacted before fallback.

## `message_update.assistantMessageEvent`

Exact discriminated variants in 17.2.10:

- `start { contentIndex?: undefined, partial: AssistantMessage }`
- `text_start { contentIndex: number, partial: AssistantMessage }`
- `text_delta { contentIndex: number, delta: string, partial: AssistantMessage }`
- `text_end { contentIndex: number, content: string, partial: AssistantMessage }`
- `thinking_start { contentIndex: number, partial: AssistantMessage }`
- `thinking_delta { contentIndex: number, delta: string, partial: AssistantMessage }`
- `thinking_end { contentIndex: number, content: string, partial: AssistantMessage }`
- `image_end { contentIndex: number, content: ImageContent, partial: AssistantMessage }`
- `toolcall_start { contentIndex: number, partial: AssistantMessage }`
- `toolcall_delta { contentIndex: number, delta: string, partial: AssistantMessage }`
- `toolcall_end { contentIndex: number, toolCall: ToolCall, partial: AssistantMessage }`
- `done { contentIndex?: undefined, reason: "stop" | "length" | "toolUse", message: AssistantMessage }`
- `error { contentIndex?: undefined, reason: "aborted" | "error", error: AssistantMessage }`

Every `message_update` also carries `message: AgentMessage`. `partial`, `message`, `toolCall`, image content, and completed/error messages must remain lossless JSON values at the RPC boundary: provider payloads and future content block fields are open-ended. Pimiento models the stable discriminant and scalar delta/index fields while retaining raw values and an Unknown fallback.

`ToolCall` stable fields are `{type:"toolCall", id, name, arguments}` with optional `thoughtSignature`, `intent`, `rawBlock`, `customWireName`, and provider metadata. Do not assume arguments arrive complete before `toolcall_end`.

## Tool execution lifecycle

Exact 17.2.10 shapes:

- `tool_execution_start { type, toolCallId: string, toolName: string, args: any, intent?: string }`
- `tool_execution_update { type, toolCallId: string, toolName: string, args: any, partialResult: any }`
- `tool_execution_end { type, toolCallId: string, toolName: string, result: any, isError?: boolean }`

The extension-facing type documents `isError: boolean`, but the core `AgentEvent` wire source marks it optional. Decode absence as unknown/false only in projection policy; preserve absence in the protocol model. `args`, `partialResult`, and `result` are arbitrary JSON and must never be narrowed or dropped.

## Other 17.2.10 deltas that affect M1

- `tool_execution_start.intent` is optional and should be preserved.
- `agent_end` may include optional `telemetry`, `coverage`, and session-added `isTerminal`.
- `turn_end` carries both `message` and `toolResults`.
- `thinking_level_changed` carries `thinkingLevel?`, optional configured selector, and optional resolved effort.
- `notice` is `{level:"info"|"warning"|"error", message, source?}`.
- Extension UI `editor` has optional `prefill` and `promptStyle`; `setStatus.statusText` and `setWidget.widgetLines` may be absent; `open_url` may include `launchUrl` and `instructions`.
- Host tool `partialResult`/`result` are structured `AgentToolResult`, not strings. Host URI result content type is restricted to markdown, JSON, or plain text.
- Unknown top-level frames and unknown discriminated variants must retain and render raw JSON.
