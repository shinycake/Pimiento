//! Reducer-focused tests for `pimiento_core::projection::SessionProjection`.
//!
//! Every Tier-1 wire event has at least one direct transition test here.
//! Fixture-driven replay lives in `M2Replay`'s tests; these exercise pure
//! reducer semantics: correctness, concurrent-tool safety, duplicate /
//! reordered lifecycle tolerance, and the Unknown-visibility rule.

use omp_rpc_client::frames::{IncomingFrame, decode_frame};
use pimiento_core::projection::{RunPhase, SessionProjection, StateQuality};
use pimiento_core::transcript::{ToolStatus, TranscriptEntry};
use serde_json::{Value, json};

fn frame(v: Value) -> IncomingFrame {
    match decode_frame(v) {
        Ok(frame) => frame,
        Err(err) => panic!("test frame must decode: {err}"),
    }
}

fn apply(p: &mut SessionProjection, v: Value) {
    p.apply(&frame(v));
}

// ---------------------------------------------------------------------------
// Agent lifecycle: agent_start -> Streaming, agent_end -> Idle / AwaitingResume
// ---------------------------------------------------------------------------

#[test]
fn agent_start_transitions_to_streaming() {
    let mut p = SessionProjection::new();
    assert_eq!(p.run_phase, RunPhase::Idle);
    apply(&mut p, json!({ "type": "agent_start" }));
    assert_eq!(p.run_phase, RunPhase::Streaming);
    assert_eq!(p.state.is_streaming, Some(true));
}

#[test]
fn agent_end_terminal_goes_idle() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "agent_start" }));
    apply(&mut p, json!({ "type": "agent_end", "messages": [] }));
    assert_eq!(p.run_phase, RunPhase::Idle);
    assert_eq!(p.state.is_streaming, Some(false));
}

#[test]
fn agent_end_non_terminal_awaits_resume() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "agent_start" }));
    apply(
        &mut p,
        json!({ "type": "agent_end", "messages": [], "isTerminal": false }),
    );
    assert_eq!(p.run_phase, RunPhase::AwaitingResume);
}

#[test]
fn agent_end_aborted_emits_visible_error() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "agent_start" }));
    apply(
        &mut p,
        json!({
            "type": "agent_end",
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                {
                    "role": "assistant",
                    "content": [],
                    "stopReason": "aborted",
                    "errorMessage": "Interrupted by user"
                }
            ]
        }),
    );
    assert_eq!(p.run_phase, RunPhase::Idle);
    let errors: Vec<_> = p
        .transcript
        .iter()
        .filter_map(|e| match e {
            TranscriptEntry::Error { message, code } => Some((message.clone(), code.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        errors,
        vec![("Interrupted by user".to_owned(), Some("aborted".to_owned()))]
    );
}

#[test]
fn agent_end_aborted_is_idempotent_on_duplicate() {
    let mut p = SessionProjection::new();
    let end = json!({
        "type": "agent_end",
        "messages": [
            {
                "role": "assistant",
                "content": [],
                "stopReason": "aborted",
                "errorMessage": "Interrupted by user"
            }
        ]
    });
    apply(&mut p, end.clone());
    apply(&mut p, end);
    let errors = p
        .transcript
        .iter()
        .filter(|e| matches!(e, TranscriptEntry::Error { .. }))
        .count();
    assert_eq!(errors, 1);
}

#[test]
fn agent_end_non_terminal_does_not_emit_error() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({
            "type": "agent_end",
            "isTerminal": false,
            "messages": [
                {
                    "role": "assistant",
                    "content": [],
                    "stopReason": "aborted",
                    "errorMessage": "Interrupted by user"
                }
            ]
        }),
    );
    assert!(
        p.transcript
            .iter()
            .all(|e| !matches!(e, TranscriptEntry::Error { .. }))
    );
}

// ---------------------------------------------------------------------------
// Assistant text lifecycle: deltas accumulate, text_end reconciles losslessly
// ---------------------------------------------------------------------------

fn message_update(evt: &Value) -> Value {
    json!({
        "type": "message_update",
        "assistantMessageEvent": evt,
        "message": {}
    })
}

#[test]
fn text_deltas_accumulate_and_end_reconciles() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "message_start" }));
    apply(
        &mut p,
        message_update(&json!({ "type": "start", "partial": {} })),
    );
    apply(
        &mut p,
        message_update(&json!({
            "type": "text_start", "contentIndex": 0, "partial": {}
        })),
    );
    apply(
        &mut p,
        message_update(&json!({
            "type": "text_delta", "contentIndex": 0, "delta": "Hel", "partial": {}
        })),
    );
    apply(
        &mut p,
        message_update(&json!({
            "type": "text_delta", "contentIndex": 0, "delta": "lo", "partial": {}
        })),
    );
    // Before text_end, the row is streaming and reflects deltas.
    match &p.transcript[0] {
        TranscriptEntry::AssistantText {
            markdown,
            streaming,
        } => {
            assert!(streaming);
            assert_eq!(markdown.0, "Hello");
        }
        other => panic!("expected AssistantText, got {other:?}"),
    }
    apply(
        &mut p,
        message_update(&json!({
            "type": "text_end", "contentIndex": 0, "content": "Hello, world!", "partial": {}
        })),
    );
    match &p.transcript[0] {
        TranscriptEntry::AssistantText {
            markdown,
            streaming,
        } => {
            // text_end reconciles final content — does NOT concatenate.
            assert!(!streaming);
            assert_eq!(markdown.0, "Hello, world!");
        }
        other => panic!("expected AssistantText, got {other:?}"),
    }
    apply(&mut p, json!({ "type": "message_end" }));
    // No new rows; still one AssistantText.
    assert_eq!(p.transcript.len(), 1);
}

#[test]
fn thinking_deltas_accumulate_and_end_reconciles() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "message_start" }));
    apply(
        &mut p,
        message_update(&json!({
            "type": "thinking_start", "contentIndex": 0, "partial": {}
        })),
    );
    apply(
        &mut p,
        message_update(&json!({
            "type": "thinking_delta", "contentIndex": 0, "delta": "ponder ", "partial": {}
        })),
    );
    apply(
        &mut p,
        message_update(&json!({
            "type": "thinking_end", "contentIndex": 0, "content": "pondered", "partial": {}
        })),
    );
    match &p.transcript[0] {
        TranscriptEntry::Thinking {
            text, streaming, ..
        } => {
            assert!(!streaming);
            assert_eq!(text, "pondered");
        }
        other => panic!("expected Thinking, got {other:?}"),
    }
}

#[test]
fn done_closes_streaming_rows_without_duplicating_content() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "message_start" }));
    apply(
        &mut p,
        message_update(&json!({
            "type": "text_delta", "contentIndex": 0, "delta": "hi", "partial": {}
        })),
    );
    apply(
        &mut p,
        message_update(&json!({
            "type": "done", "reason": "stop", "message": {}
        })),
    );
    match &p.transcript[0] {
        TranscriptEntry::AssistantText {
            markdown,
            streaming,
        } => {
            assert!(!streaming);
            assert_eq!(markdown.0, "hi");
        }
        other => panic!("expected AssistantText, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// User message
// ---------------------------------------------------------------------------

#[test]
fn user_message_is_visible_once() {
    let mut p = SessionProjection::new();
    p.push_user_message("hi there".to_owned());
    assert_eq!(p.transcript.len(), 1);
    match &p.transcript[0] {
        TranscriptEntry::User { text } => assert_eq!(text, "hi there"),
        other => panic!("expected User, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tool execution — concurrent lifecycles keyed by toolCallId
// ---------------------------------------------------------------------------

fn tool_start(id: &str, name: &str, args: &Value) -> Value {
    json!({
        "type": "tool_execution_start",
        "toolCallId": id, "toolName": name, "args": args
    })
}
fn tool_update(id: &str, partial: &Value) -> Value {
    json!({
        "type": "tool_execution_update",
        "toolCallId": id, "toolName": "t", "args": {}, "partialResult": partial
    })
}
fn tool_end(id: &str, result: Value, is_error: Option<bool>) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("type".into(), json!("tool_execution_end"));
    m.insert("toolCallId".into(), json!(id));
    m.insert("toolName".into(), json!("t"));
    m.insert("result".into(), result);
    if let Some(e) = is_error {
        m.insert("isError".into(), json!(e));
    }
    Value::Object(m)
}

#[test]
fn concurrent_tools_route_to_correct_row() {
    let mut p = SessionProjection::new();
    apply(&mut p, tool_start("a", "bash", &json!({"cmd":"ls"})));
    apply(&mut p, tool_start("b", "read", &json!({"path":"x"})));
    // Interleaved updates.
    apply(&mut p, tool_update("b", &json!("readB1")));
    apply(&mut p, tool_update("a", &json!("outA1")));
    apply(&mut p, tool_end("a", json!({"text":"final-A"}), None));
    apply(&mut p, tool_update("b", &json!("readB2")));
    apply(&mut p, tool_end("b", json!({"text":"final-B"}), Some(true)));

    // Two ToolCall rows, in the order they started.
    let calls: Vec<_> = p
        .transcript
        .iter()
        .filter_map(|e| match e {
            TranscriptEntry::ToolCall(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].tool_call_id, "a");
    assert_eq!(calls[1].tool_call_id, "b");
    assert_eq!(calls[0].status, ToolStatus::Ok);
    assert_eq!(calls[1].status, ToolStatus::Err);
    assert_eq!(calls[0].output.render(), "final-A");
    assert_eq!(calls[1].output.render(), "final-B");
}

#[test]
fn tool_end_without_start_creates_row() {
    let mut p = SessionProjection::new();
    apply(&mut p, tool_end("x", json!("done"), None));
    match &p.transcript[0] {
        TranscriptEntry::ToolCall(c) => {
            assert_eq!(c.tool_call_id, "x");
            assert_eq!(c.status, ToolStatus::Ok);
            assert_eq!(c.output.render(), "done");
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn tool_update_without_start_creates_row() {
    let mut p = SessionProjection::new();
    apply(&mut p, tool_update("q", &json!("partial")));
    match &p.transcript[0] {
        TranscriptEntry::ToolCall(c) => {
            assert_eq!(c.tool_call_id, "q");
            assert_eq!(c.status, ToolStatus::Running);
            assert_eq!(c.output.render(), "partial");
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn duplicate_tool_start_does_not_reset_row() {
    let mut p = SessionProjection::new();
    apply(&mut p, tool_start("a", "bash", &json!({"cmd":"ls"})));
    apply(&mut p, tool_end("a", json!("ok"), None));
    // A second start (protocol tolerance) must not duplicate the row or
    // clobber the Ok status.
    apply(&mut p, tool_start("a", "bash", &json!({"cmd":"ls"})));
    let calls: Vec<_> = p
        .transcript
        .iter()
        .filter_map(|e| match e {
            TranscriptEntry::ToolCall(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].status, ToolStatus::Ok);
}

#[test]
fn is_error_absence_and_false_map_to_ok() {
    let mut p = SessionProjection::new();
    apply(&mut p, tool_start("a", "t", &json!({})));
    apply(&mut p, tool_end("a", json!("x"), None));
    apply(&mut p, tool_start("b", "t", &json!({})));
    apply(&mut p, tool_end("b", json!("y"), Some(false)));
    let ok_a =
        matches!(&p.transcript[0], TranscriptEntry::ToolCall(c) if c.status == ToolStatus::Ok);
    let ok_b =
        matches!(&p.transcript[1], TranscriptEntry::ToolCall(c) if c.status == ToolStatus::Ok);
    assert!(ok_a && ok_b);
}

#[test]
fn tool_partial_result_recursive_extraction() {
    let mut p = SessionProjection::new();
    apply(&mut p, tool_start("a", "t", &json!({})));
    apply(
        &mut p,
        tool_update(
            "a",
            &json!({ "content": [ { "text": "one" }, { "text": "two" } ] }),
        ),
    );
    match &p.transcript[0] {
        TranscriptEntry::ToolCall(c) => assert_eq!(c.output.render(), "one\ntwo"),
        other => panic!("expected ToolCall, got {other:?}"),
    }
    // Non-text object — falls back to pretty JSON, still visible.
    apply(&mut p, tool_end("a", json!({ "code": 42 }), None));
    match &p.transcript[0] {
        TranscriptEntry::ToolCall(c) => {
            assert!(c.output.render().contains("42"));
            assert_eq!(c.status, ToolStatus::Ok);
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Compaction / retry phases
// ---------------------------------------------------------------------------

#[test]
fn compaction_lifecycle_transitions_phase() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "agent_start" }));
    apply(&mut p, json!({ "type": "auto_compaction_start" }));
    assert_eq!(p.run_phase, RunPhase::Compacting);
    assert_eq!(p.state.is_compacting, Some(true));
    apply(&mut p, json!({ "type": "auto_compaction_end" }));
    assert_eq!(p.state.is_compacting, Some(false));
    // is_streaming still true → return to Streaming.
    assert_eq!(p.run_phase, RunPhase::Streaming);
    // Two compaction rows emitted (Started, Completed).
    let compactions = p
        .transcript
        .iter()
        .filter(|e| matches!(e, TranscriptEntry::Compaction { .. }))
        .count();
    assert_eq!(compactions, 2);
}

#[test]
fn retry_lifecycle_transitions_phase() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "agent_start" }));
    apply(&mut p, json!({ "type": "auto_retry_start" }));
    assert_eq!(p.run_phase, RunPhase::Retrying);
    apply(&mut p, json!({ "type": "auto_retry_end" }));
    assert_eq!(p.run_phase, RunPhase::Streaming);
    apply(&mut p, json!({ "type": "retry_fallback_applied" }));
    apply(&mut p, json!({ "type": "retry_fallback_succeeded" }));
    let retries = p
        .transcript
        .iter()
        .filter(|e| matches!(e, TranscriptEntry::RetryInfo { .. }))
        .count();
    assert_eq!(retries, 4);
}

// ---------------------------------------------------------------------------
// Notice / errors / prompt_result / extension_error / rpc_frame_error
// ---------------------------------------------------------------------------

#[test]
fn notice_info_becomes_notice_entry() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "notice", "level": "info", "message": "hi" }),
    );
    assert!(matches!(&p.transcript[0], TranscriptEntry::Notice(s) if s == "hi"));
}

#[test]
fn notice_error_becomes_error_entry() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "notice", "level": "error", "message": "boom", "source": "s" }),
    );
    match &p.transcript[0] {
        TranscriptEntry::Error { message, code } => {
            assert_eq!(message, "boom");
            assert_eq!(code.as_deref(), Some("s"));
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn extension_error_becomes_error_entry() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "extension_error", "message": "ext failed", "code": "E1" }),
    );
    match &p.transcript[0] {
        TranscriptEntry::Error { message, code } => {
            assert_eq!(message, "ext failed");
            assert_eq!(code.as_deref(), Some("E1"));
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn rpc_frame_error_leaves_visible_row() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "rpc_frame_error", "error": "too big" }),
    );
    // Both an Error and a raw Unknown are emitted (visibility over
    // duplication concerns — the raw frame is preserved).
    assert!(
        p.transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::Error { .. }))
    );
    assert!(
        p.transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::Unknown { .. }))
    );
}

#[test]
fn prompt_result_false_emits_notice() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "prompt_result", "id": "req_1", "agentInvoked": false }),
    );
    assert!(matches!(&p.transcript[0], TranscriptEntry::Notice(_)));
}

#[test]
fn prompt_result_true_is_silent() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "prompt_result", "id": "req_1", "agentInvoked": true }),
    );
    assert!(p.transcript.is_empty());
}

#[test]
fn command_output_becomes_row() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "command_output", "output": "stdout tail" }),
    );
    match &p.transcript[0] {
        TranscriptEntry::CommandOutput(s) => assert_eq!(s, "stdout tail"),
        other => panic!("expected CommandOutput, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Extension UI dialogs and display state
// ---------------------------------------------------------------------------

#[test]
fn select_confirm_input_editor_open_url_enqueue_dialogs() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id": "d1", "method": "select",
                "title": "pick", "options": ["a","b"], "timeout": 5000.0 }),
    );
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id": "d2", "method": "confirm",
                "title": "ok?", "message": "sure?" }),
    );
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id": "d3", "method": "input",
                "title": "?", "placeholder": "type" }),
    );
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id": "d4", "method": "editor",
                "title": "compose" }),
    );
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id": "d5", "method": "open_url",
                "url": "https://x/" }),
    );
    let methods: Vec<_> = p
        .pending_dialogs
        .iter()
        .map(|d| d.method.as_str())
        .collect();
    assert_eq!(
        methods,
        ["select", "confirm", "input", "editor", "open_url"]
    );
    assert_eq!(p.pending_dialogs[0].timeout_ms, Some(5000.0));
}

#[test]
fn cancel_removes_dialog_by_target_id() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id": "d1", "method": "confirm",
                "title": "a", "message": "b" }),
    );
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id": "d2", "method": "confirm",
                "title": "c", "message": "d" }),
    );
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id": "cancel-1", "method": "cancel",
                "targetId": "d1" }),
    );
    assert_eq!(p.pending_dialogs.len(), 1);
    assert_eq!(p.pending_dialogs[0].id, "d2");
}

#[test]
fn display_state_captures_notify_status_widget_title_editor() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id":"u1", "method": "setTitle", "title":"T" }),
    );
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id":"u2", "method": "setStatus",
                "statusKey":"k", "statusText":"v" }),
    );
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id":"u3", "method": "setWidget",
                "widgetKey":"w", "widgetLines":["a","b"] }),
    );
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id":"u4", "method": "set_editor_text",
                "text":"draft" }),
    );
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id":"u5", "method": "notify",
                "message":"heads up", "notifyType":"info" }),
    );
    assert_eq!(p.display.title.as_deref(), Some("T"));
    assert_eq!(p.display.statuses.get("k"), Some(&Some("v".to_owned())));
    assert!(p.display.widgets.contains_key("w"));
    assert_eq!(p.display.editor_text.as_deref(), Some("draft"));
    assert!(matches!(
        p.transcript.last(),
        Some(TranscriptEntry::Notice(_))
    ));
    // No dialog entries were enqueued.
    assert!(p.pending_dialogs.is_empty());
}

#[test]
fn unknown_ui_method_stays_visible_as_unknown_row() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id":"u1", "method": "future_method",
                "whatever": true }),
    );
    assert!(matches!(
        p.transcript.last(),
        Some(TranscriptEntry::Unknown { .. })
    ));
}

// ---------------------------------------------------------------------------
// Hydration: get_state + available_commands
// ---------------------------------------------------------------------------

#[test]
fn hydrate_get_state_promotes_scalars_and_marks_durable() {
    let mut p = SessionProjection::new();
    p.hydrate_get_state(&json!({
        "state": {
            "model": "gpt-x",
            "sessionId": "s1",
            "sessionFile": "/tmp/s1.jsonl",
            "isStreaming": false,
            "isCompacting": false,
            "fastMode": { "enabled": true, "active": false },
            "tokens": { "in": 10, "out": 20 },
            "context": { "used": 12345 },
            "thinkingLevel": "high",
            "todoPhases": [{"phase":"p1"}],
            "extraKey": "kept-in-raw"
        }
    }));
    assert_eq!(p.state.quality, StateQuality::Durable);
    assert_eq!(p.state.model.as_deref(), Some("gpt-x"));
    assert_eq!(p.state.session_id.as_deref(), Some("s1"));
    assert_eq!(p.state.session_file.as_deref(), Some("/tmp/s1.jsonl"));
    assert_eq!(p.state.is_streaming, Some(false));
    assert_eq!(p.state.fast_mode_enabled, Some(true));
    assert_eq!(p.state.fast_mode_active, Some(false));
    assert_eq!(p.state.tokens, Some(json!({"in":10,"out":20})));
    assert_eq!(p.state.context, Some(json!({"used":12345})));
    assert_eq!(p.state.thinking, Some(json!("high")));
    assert!(p.todos_raw.is_some());
    // Raw state preserved losslessly.
    assert_eq!(
        p.state.state.as_ref().and_then(|raw| raw.get("extraKey")),
        Some(&json!("kept-in-raw"))
    );
}

#[test]
fn hydrate_available_commands_stores_raw() {
    let mut p = SessionProjection::new();
    p.hydrate_available_commands(&json!({ "commands": [ {"name":"/compact"} ] }));
    assert!(p.available_commands_raw.is_some());
}

#[test]
fn available_commands_update_frame_stores_raw() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "available_commands_update", "commands": [{"name":"/help"}] }),
    );
    assert!(p.available_commands_raw.is_some());
}

// ---------------------------------------------------------------------------
// Explicit supervisor transitions
// ---------------------------------------------------------------------------

#[test]
fn mark_restarting_clears_dialogs_and_marks_stale() {
    let mut p = SessionProjection::new();
    p.hydrate_get_state(&json!({"state":{"model":"m"}}));
    apply(
        &mut p,
        json!({ "type": "extension_ui_request", "id":"d1", "method": "confirm",
                "title":"a", "message":"b" }),
    );
    p.mark_restarting();
    assert_eq!(p.run_phase, RunPhase::Restarting);
    assert_eq!(p.state.quality, StateQuality::Stale);
    assert!(p.pending_dialogs.is_empty());
}

#[test]
fn mark_dead_records_reason() {
    let mut p = SessionProjection::new();
    p.mark_dead("segfault".to_owned());
    assert_eq!(p.run_phase, RunPhase::Dead);
    assert_eq!(p.dead_reason.as_deref(), Some("segfault"));
}

// ---------------------------------------------------------------------------
// Model / thinking-level / notice-unknown / subagent / todo-clear / config /
// unknown frame all remain visible
// ---------------------------------------------------------------------------

#[test]
fn model_changed_updates_state_and_keeps_raw_visible() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "model_changed", "model": "gpt-y" }));
    assert_eq!(p.state.model.as_deref(), Some("gpt-y"));
    assert!(matches!(
        p.transcript.last(),
        Some(TranscriptEntry::Unknown { .. })
    ));
}

#[test]
fn thinking_level_changed_updates_state() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "thinking_level_changed", "resolved": "low" }),
    );
    assert_eq!(p.state.thinking, Some(json!("low")));
}

#[test]
fn subagent_frames_stored_raw() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "subagent_progress", "payload": {"n":1} }),
    );
    apply(
        &mut p,
        json!({ "type": "subagent_event", "payload": {"n":2} }),
    );
    apply(
        &mut p,
        json!({ "type": "subagent_lifecycle", "payload": {"n":3} }),
    );
    assert_eq!(p.subagents_raw.len(), 3);
}

#[test]
fn todo_auto_clear_wipes_todos() {
    let mut p = SessionProjection::new();
    p.hydrate_get_state(&json!({"state":{"todoPhases":[{"x":1}]}}));
    assert!(p.todos_raw.is_some());
    apply(&mut p, json!({ "type": "todo_auto_clear" }));
    assert!(p.todos_raw.is_none());
}

#[test]
fn unknown_frame_type_is_visible() {
    let mut p = SessionProjection::new();
    apply(
        &mut p,
        json!({ "type": "future_frame_from_omp_18", "payload": 1 }),
    );
    assert!(matches!(
        p.transcript.last(),
        Some(TranscriptEntry::Unknown { raw }) if raw.get("type").and_then(Value::as_str) == Some("future_frame_from_omp_18")
    ));
}

#[test]
fn unhandled_known_frames_stay_visible_as_unknown() {
    let mut p = SessionProjection::new();
    for ty in [
        "ttsr_triggered",
        "todo_reminder",
        "irc_message",
        "goal_updated",
        "config_update",
    ] {
        apply(&mut p, json!({ "type": ty }));
    }
    let unknowns = p
        .transcript
        .iter()
        .filter(|e| matches!(e, TranscriptEntry::Unknown { .. }))
        .count();
    assert_eq!(unknowns, 5);
}

// ---------------------------------------------------------------------------
// Reorder / duplicate lifecycle tolerance
// ---------------------------------------------------------------------------

#[test]
fn duplicate_message_end_is_idempotent() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "message_start" }));
    apply(
        &mut p,
        message_update(&json!({
            "type": "text_delta", "contentIndex": 0, "delta": "x", "partial": {}
        })),
    );
    apply(&mut p, json!({ "type": "message_end" }));
    apply(&mut p, json!({ "type": "message_end" }));
    match &p.transcript[0] {
        TranscriptEntry::AssistantText {
            streaming,
            markdown,
        } => {
            assert!(!streaming);
            assert_eq!(markdown.0, "x");
        }
        other => panic!("expected AssistantText, got {other:?}"),
    }
}

#[test]
fn double_agent_start_stays_streaming() {
    let mut p = SessionProjection::new();
    apply(&mut p, json!({ "type": "agent_start" }));
    apply(&mut p, json!({ "type": "agent_start" }));
    assert_eq!(p.run_phase, RunPhase::Streaming);
}
