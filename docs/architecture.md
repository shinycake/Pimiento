# Pimiento architecture

Pimiento is a native GPUI desktop client for the user’s existing [Oh My Pi](https://omp.sh) (`omp`) install. OMP remains the sole authority over agent and session state. Pimiento is a **projection and command surface**: it renders decoded NDJSON RPC frames and sends typed commands back — it never fabricates, infers, or persists runtime truth.

## Doctrine

1. **The app never owns runtime state.** Durable data (messages, session names, model, todos, locks, auth) lives in OMP. The client keeps only disposable display state (composer draft, scroll, expanded cards, layout). On doubt, re-fetch (`get_state`, `get_messages_page`).
2. **The app never owns the harness.** Pimiento uses the user’s existing `omp` binary, auth, config, and session store. It does not install OMP by default and does not hand-edit `~/.omp` (explicit user actions such as Update OMP or assigning a model role via `omp config` are the exceptions).
3. **Commands are explicit and correlated.** Match responses on command `id`, never emission order.
4. **Command acceptance ≠ completion.** A run completes on terminal `agent_end` (or local-only prompt results), not on the prompt ACK.
5. **Render state qualities honestly:** durable, live, stale (restarting), unknown — never invent a default.
6. **Never smooth over failures.** Crash cards show why; disabled controls state why.
7. **Child stdin/stdout are protocol-only.** App logs go elsewhere.
8. **One session per child process.** Multi-session means multiple supervised `omp` processes.
9. **Unknown wire data always renders.** Every wire enum has an `Unknown` fallback; unknown frames become visible raw rows. Never panic, never drop silently.
10. **YAGNI.** One excellent session workspace — not a remoting fleet, IDE, or settings mirror.

## Crate layout

```text
omp-rpc-client  →  pimiento-core  →  pimiento-app
   (wire)            (projection)        (GPUI)
```

Protocol types never import UI types. `#![forbid(unsafe_code)]` in every crate.

## Event pipeline

- Background tasks parse and read; **only the foreground pump mutates GPUI entities.**
- `ClientEvent` uses a **bounded(512)** channel. A slow UI applies backpressure on the stdout reader (intentional).
- The pump drains the channel per wakeup, applies batched deltas in one entity update, then emits one `cx.notify()`.
- Async runtime is **smol** (matches GPUI). No tokio.
- Child supervision uses a crash-loop breaker (3 restarts / 60s); beyond that the UI shows Dead + Restart.

## Transcript memory

Tool output, assistant markdown, thinking text, and command-output rows use **`BoundedText`** with a **512 KiB** head+tail budget and a visible elision marker. Full content remains in the OMP session file.

## GPL boundary

- `gpui`, `gpui_platform`, and `gpui-component` are Apache-2.0 — fine to read and depend on at the pinned revs.
- Zed’s `ui`, `editor`, `terminal_view`, and `agent_ui` crates are **GPL-3.0-or-later**. Patterns only; never copy source.
- OMP is MIT; `rpc-types.ts` at the pinned OMP revision is the authoritative wire reference. See also [protocol-notes.md](protocol-notes.md).
