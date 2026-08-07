//! Live smoke test against the user's real installed OMP.
//!
//! This test is `#[ignore]` because it spawns the *real* `omp` binary
//! discovered on the developer's machine and issues one prompt, which
//! consumes an actual model request against whatever provider that OMP
//! is logged into. Run it explicitly with:
//!
//! ```text
//! cargo nextest run -p omp-rpc-client --run-ignored ignored-only
//! ```
//!
//! The regular test gate never touches this file.
//!
//! Flow (`PLAN` §§4.2, 4.7, 4.8, 5.3):
//!
//! 1. Discover `omp` via the same login-shell path the real app uses,
//!    inheriting the current process env for `PIMIENTO_OMP_BIN` /
//!    `SHELL`.
//! 2. Connect via [`RpcClient::connect`] — this spawns `omp
//!    --mode rpc-ui --no-session`, reads the `ready` frame, and
//!    negotiates protocol v2.
//! 3. Issue `get_state`.
//! 4. Issue a `prompt` asking the model to reply with exactly "pong"
//!    and assert its ACK response is `success: true`.
//! 5. Consume streamed events until a terminal `agent_end` frame,
//!    accumulating every assistant text delta and full-message content
//!    block. Assert the combined transcript contains "pong" — no
//!    assumption is made about how many deltas the provider emits or
//!    which envelope carries the final text.
//!
//! A generous bounded timeout wraps the whole session so a hung child
//! can never wedge CI. Cleanup (`close_stdin` + `wait`) is always
//! attempted, even on failure, so the child is reaped.
//!
//! ### API assumptions (coordinated with `M1Client`)
//!
//! Everything below is the `M1Client` / discovery surface this test
//! depends on. If any of it drifts, this file breaks compile — by
//! design, since it's the runtime canary.
//!
//! * `discovery::discover(&DiscoveryInputs, &SystemRunner) ->
//!   Result<DiscoveredOmp, RpcError>` with `override_bin`,
//!   `setting_bin`, `login_shell`, `current_env` inputs; returns
//!   `path`, `env`, `version`, `version_text`. Missing/unsupported
//!   binaries surface as `RpcError::Discovery { detail }`.
//! * `client::ClientConfig { program, env, cwd, extra_args, no_session,
//!   resume, command_timeout }` — plain-struct config.
//! * `client::RpcClient::connect(cfg).await -> Result<RpcClient,
//!   RpcError>` spawns `program --mode rpc-ui [--no-session] [--resume
//!   <path>] <extra_args...>`, awaits `ready`, sends the `protocol-1`
//!   negotiate, verifies v2.
//! * `RpcClient::send(RpcCommandBody).await -> Result<RpcResponse,
//!   RpcError>` auto-assigns `req_<N>` ids and enforces
//!   `command_timeout`.
//! * `RpcClient::events() -> async_channel::Receiver<ClientEvent>`.
//!   `ClientEvent::Frame(IncomingFrame)` surfaces every non-routed
//!   inbound frame; `ClientEvent::Closed(ClosedInfo)` (tuple variant
//!   with `exit_code`, `stderr_tail`, `error_msg`) is the single
//!   terminal signal.
//! * `RpcClient::close_stdin()` half-closes stdin for graceful
//!   shutdown; `RpcClient::wait(self).await -> ClosedInfo` reaps the
//!   child.
//! * Prompt ACK (`success: true`) means the run started, NOT that it
//!   finished — the test drives on to `agent_end` explicitly.
//! * `VersionSupport::BelowMinimum` is treated as an actionable failure
//!   (not a silent skip) per the assignment.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use smol::future;

use omp_rpc_client::client::{ClientConfig, ClientEvent, RpcClient};
use omp_rpc_client::discovery::{
    DiscoveredOmp, DiscoveryInputs, MIN_SUPPORTED, SystemRunner, VersionSupport, discover,
};
use omp_rpc_client::frames::{
    AssistantMessageEvent, AssistantMessageEventKind, IncomingFrame, IncomingFrameKind,
    MessageUpdateFrame, RpcCommandBody,
};

/// Outer bound for the whole handshake+prompt+drain flow. Generous
/// enough to survive a slow model; tight enough to fail CI rather than
/// wedge it.
const LIVE_TIMEOUT: Duration = Duration::from_mins(3);

/// The exact string the prompt asks the model to reply with. The test
/// only asserts *substring* containment, so provider quirks (quoting,
/// trailing punctuation, wrapping the reply in prose) are tolerated.
const EXPECTED_REPLY: &str = "pong";

#[test]
#[ignore = "spawns the user's real omp and consumes a real model request; run with --run-ignored"]
fn live_omp_pong() {
    smol::block_on(async {
        let outcome = future::or(run_live_smoke(), timeout_error()).await;
        outcome.expect("live omp smoke test failed");
    });
}

async fn timeout_error() -> Result<(), String> {
    smol::Timer::after(LIVE_TIMEOUT).await;
    Err(format!(
        "live omp smoke test exceeded {}s bounded timeout",
        LIVE_TIMEOUT.as_secs()
    ))
}

async fn run_live_smoke() -> Result<(), String> {
    // -- 1. Discover the user's real omp using their real environment.
    let discovered = discover_real_omp()?;

    match discovered.version.support() {
        VersionSupport::Supported | VersionSupport::Newer => {}
        VersionSupport::BelowMinimum => {
            return Err(format!(
                "discovered omp at {} reports version {} which is below the minimum supported \
                 (>= {MIN_SUPPORTED}) by this build; upgrade omp or point PIMIENTO_OMP_BIN at a \
                 newer install",
                discovered.path.display(),
                discovered.version,
            ));
        }
    }

    // -- 2. Connect: spawn + ready + negotiate v2 (owned by RpcClient).
    let cfg = ClientConfig {
        program: discovered.path.clone(),
        env: discovered.env.clone(),
        cwd: None,
        extra_args: Vec::new(),
        no_session: true,
        resume: None,
        command_timeout: Duration::from_mins(1),
    };

    let client = RpcClient::connect(cfg).await.map_err(|e| {
        format!(
            "RpcClient::connect failed against {}: {e}",
            discovered.path.display()
        )
    })?;

    // From here on out, we MUST reap the child on every exit path.
    drive_session(client).await
}

async fn drive_session(client: RpcClient) -> Result<(), String> {
    let result = run_prompt(&client).await;

    // Best-effort cleanup on EVERY path. Half-close stdin first (the
    // graceful path); supervisor-style SIGTERM/kill is out of scope
    // for this smoke — the outer LIVE_TIMEOUT is our backstop.
    client.close_stdin().await;
    let closed = client.wait().await;
    match result {
        Ok(()) => Ok(()),
        Err(primary) => Err(format!(
            "{primary}\n(child exit_code={:?}, stderr tail:\n{})",
            closed.exit_code, closed.stderr_tail
        )),
    }
}

async fn run_prompt(client: &RpcClient) -> Result<(), String> {
    let events = client.events();

    // -- 3. get_state — sanity-check that commands round-trip.
    let state = client
        .send(RpcCommandBody::GetState)
        .await
        .map_err(|e| format!("get_state send failed: {e}"))?;
    if !state.success {
        return Err(format!(
            "get_state returned success=false; command={:?} error={:?} code={:?}",
            state.command, state.error, state.code
        ));
    }

    // -- 4. prompt — ACK must be success=true. ACK does NOT mean the
    //    run finished; we still wait on agent_end below.
    let prompt_ack = client
        .send(RpcCommandBody::Prompt {
            message: format!("reply with exactly: {EXPECTED_REPLY}"),
            images: None,
            streaming_behavior: None,
        })
        .await
        .map_err(|e| format!("prompt send failed: {e}"))?;
    if !prompt_ack.success {
        return Err(format!(
            "prompt ACK returned success=false; error={:?} code={:?}",
            prompt_ack.error, prompt_ack.code
        ));
    }

    // -- 5. Drain events until agent_end. Accumulate every scrap of
    //    assistant text we see: streaming text deltas, terminal
    //    text_end.content payloads, and full `message.content[*].text`
    //    on the enveloping message. Any of them containing `pong`
    //    counts — we do NOT assume the model streams even a single
    //    delta.
    let mut transcript = String::new();

    let agent_end_raw = loop {
        let ev = events
            .recv()
            .await
            .map_err(|_| "event channel closed before agent_end".to_string())?;

        match ev {
            ClientEvent::Frame(frame) => {
                collect_text(&frame, &mut transcript);
                if let IncomingFrameKind::AgentEnd(_) = &frame.kind {
                    break frame.raw;
                }
            }
            ClientEvent::Closed(info) => {
                return Err(format!(
                    "child closed before agent_end (exit_code={:?}, error={:?}); stderr tail:\n{}",
                    info.exit_code, info.error_msg, info.stderr_tail
                ));
            }
        }
    };

    // AgentEndFrame's optional error payload lives on the raw JSON —
    // surface it verbatim so a provider-side failure isn't masked by
    // the substring assertion below.
    if let Some(err) = agent_end_raw.get("error")
        && !err.is_null()
    {
        return Err(format!("agent_end carried error payload: {err}"));
    }

    if !transcript.contains(EXPECTED_REPLY) {
        return Err(format!(
            "assistant transcript did not contain {EXPECTED_REPLY:?}; transcript was: \
             {transcript:?}"
        ));
    }

    Ok(())
}

/// Extract every human-visible text fragment from an inbound frame and
/// append it to `sink`. Robust to the fact that a real provider can
/// stream deltas, emit a single terminal `text_end`, or only publish
/// text on the terminal `message` envelope — we accept any of them.
fn collect_text(frame: &IncomingFrame, sink: &mut String) {
    if let IncomingFrameKind::MessageUpdate(MessageUpdateFrame {
        assistant_message_event,
        message,
        ..
    }) = &frame.kind
    {
        collect_from_event(assistant_message_event, sink);
        collect_from_message(message, sink);
    }
}

fn collect_from_event(evt: &AssistantMessageEvent, sink: &mut String) {
    match &evt.kind {
        AssistantMessageEventKind::TextDelta { delta, .. } => push(sink, delta),
        AssistantMessageEventKind::TextEnd { content, .. } => push(sink, content),
        _ => {}
    }
    // The event's `partial` and `message` payloads may also contain
    // fully-materialized assistant text — walk them too.
    if let Some(partial) = evt.partial() {
        collect_from_message(partial, sink);
    }
    if let Some(msg) = evt.done_message() {
        collect_from_message(msg, sink);
    }
}

/// Walk an assistant-message `Value` and append every `text` field of
/// every content block into `sink`. Provider content shapes vary; we
/// look at anything shaped `{ content: [ { text: "..." }, ... ] }` and
/// also accept a top-level `text` string for defensiveness.
fn collect_from_message(message: &Value, sink: &mut String) {
    if let Some(text) = message.get("text").and_then(Value::as_str) {
        push(sink, text);
    }
    if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        for block in blocks {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                push(sink, text);
            }
        }
    }
}

fn push(sink: &mut String, s: &str) {
    if !s.is_empty() {
        sink.push_str(s);
        sink.push('\n');
    }
}

/// Build discovery inputs from the *real* current process env and
/// login shell — this is deliberately not the sandboxed test path.
fn discover_real_omp() -> Result<DiscoveredOmp, String> {
    let current_env: BTreeMap<OsString, OsString> = env::vars_os().collect();

    let override_bin = current_env
        .get(OsString::from("PIMIENTO_OMP_BIN").as_os_str())
        .map(PathBuf::from);

    let login_shell = current_env
        .get(OsString::from("SHELL").as_os_str())
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from("/bin/sh")));

    let inputs = DiscoveryInputs {
        override_bin,
        setting_bin: None,
        login_shell,
        current_env,
    };

    discover(&inputs, &SystemRunner).map_err(|e| {
        format!(
            "omp discovery failed: {e}. Set PIMIENTO_OMP_BIN to an absolute path of a working \
             `omp` binary (>= {MIN_SUPPORTED}) or install `omp` on the login-shell PATH."
        )
    })
}
