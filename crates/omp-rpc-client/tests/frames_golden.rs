//! Golden-JSON tests for the PLAN §4 wire model in `omp-rpc-client::frames`.
//!
//! Every literal here comes from the canonical shapes documented in
//! `docs/protocol-notes.md` (OMP 17.2.10). The tests prove:
//!
//! * Every discriminated variant of `assistantMessageEvent` decodes into the
//!   typed kind, retains raw JSON, and round-trips byte-identically.
//! * All tool lifecycle frames decode into typed structs; `isError` remains
//!   `Option<bool>` and its absence is preserved.
//! * `ready`, `negotiate_protocol`, `get_state`, and `prompt` commands
//!   serialize to the exact camelCase wire keys OMP expects and omit absent
//!   optionals.
//! * Unknown top-level frames and unknown assistant variants decode into the
//!   `Unknown` arm and preserve their raw JSON verbatim.

use omp_rpc_client::frames::{
    AssistantMessageEvent, AssistantMessageEventKind, DoneReason, ErrorReason, IncomingFrame,
    IncomingFrameKind, MessageUpdateFrame, RpcCommand, RpcCommandBody, StreamingBehavior,
    ToolExecutionEndFrame, ToolExecutionStartFrame, ToolExecutionUpdateFrame, decode_frame,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_frame(v: Value) -> IncomingFrame {
    decode_frame(v).expect("frame decodes")
}

fn roundtrip_value(v: &Value) {
    let s = serde_json::to_string(v).expect("serialization/parse succeeds in test fixture");
    let back: Value =
        serde_json::from_str(&s).expect("serialization/parse succeeds in test fixture");
    assert_eq!(&back, v);
}

fn assistant_event(v: Value) -> AssistantMessageEvent {
    serde_json::from_value(v).expect("assistantMessageEvent decodes")
}

// ---------------------------------------------------------------------------
// Ready / response / negotiate / get_state / prompt
// ---------------------------------------------------------------------------

#[test]
fn ready_frame_decodes_with_supported_versions_and_limits() {
    let raw = json!({
        "type": "ready",
        "protocolVersion": 1,
        "supportedProtocolVersions": [1, 2],
        "maxFrameBytes": 1_048_576,
        "maxReassembledFrameBytes": 67_108_864,
    });
    let f = parse_frame(raw.clone());
    match &f.kind {
        IncomingFrameKind::Ready(r) => {
            assert_eq!(r.protocol_version, 1);
            assert_eq!(r.supported_protocol_versions, vec![1, 2]);
            assert_eq!(r.max_frame_bytes, 1_048_576);
            assert_eq!(r.max_reassembled_frame_bytes, 67_108_864);
        }
        other => panic!("expected Ready, got {other:?}"),
    }
    assert_eq!(f.raw, raw);
}

#[test]
fn response_tolerates_missing_id_and_success_false() {
    // Malformed client JSON: server returns id-less parse failure.
    let raw = json!({
        "type": "response",
        "command": "parse",
        "success": false,
        "error": "bad JSON",
    });
    let f = parse_frame(raw.clone());
    match &f.kind {
        IncomingFrameKind::Response(r) => {
            assert!(r.id.is_none());
            assert_eq!(r.command, "parse");
            assert!(!r.success);
            assert_eq!(r.error.as_deref(), Some("bad JSON"));
            assert!(r.code.is_none());
            assert!(r.data.is_none());
        }
        other => panic!("expected Response, got {other:?}"),
    }
}

#[test]
fn response_get_state_data_stays_lossless() {
    // Only a subset of RpcSessionState fields; unknown-shape data must survive.
    let raw = json!({
        "id": "req_1",
        "type": "response",
        "command": "get_state",
        "success": true,
        "data": {
            "sessionId": "s-1",
            "isStreaming": false,
            "isCompacting": false,
            "steeringMode": "all",
            "followUpMode": "all",
            "interruptMode": "wait",
            "autoCompactionEnabled": true,
            "fastModeEnabled": false,
            "fastModeActive": false,
            "tokensPerSecond": null,
            "messageCount": 0,
            "queuedMessageCount": 0,
            "todoPhases": [],
            "someFutureField": {"nested": [1, 2, 3]},
        },
    });
    let f = parse_frame(raw.clone());
    if let IncomingFrameKind::Response(r) = &f.kind {
        assert_eq!(r.id.as_deref(), Some("req_1"));
        assert!(r.success);
        assert_eq!(
            r.data
                .as_ref()
                .expect("serialization/parse succeeds in test fixture")["someFutureField"]["nested"]
                [2],
            json!(3)
        );
    } else {
        panic!("expected Response");
    }
    // The whole frame round-trips verbatim.
    let re = serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture");
    assert_eq!(re, raw);
}

#[test]
fn command_negotiate_protocol_serializes_exact_camelcase() {
    let cmd = RpcCommand::new(
        Some("req_1".to_owned()),
        RpcCommandBody::NegotiateProtocol {
            protocol_version: 2,
        },
    );
    let s = serde_json::to_value(&cmd).expect("serialization/parse succeeds in test fixture");
    assert_eq!(
        s,
        json!({
            "id": "req_1",
            "type": "negotiate_protocol",
            "protocolVersion": 2,
        })
    );
}

#[test]
fn command_get_state_omits_id_when_absent() {
    let cmd = RpcCommand::new(None, RpcCommandBody::GetState);
    let s = serde_json::to_value(&cmd).expect("serialization/parse succeeds in test fixture");
    assert_eq!(s, json!({"type": "get_state"}));
    // No stray fields.
    assert_eq!(
        s.as_object()
            .expect("serialization/parse succeeds in test fixture")
            .len(),
        1
    );
}

#[test]
fn command_prompt_omits_absent_optionals_and_encodes_streaming_behavior() {
    let bare = RpcCommand::new(
        Some("req_2".to_owned()),
        RpcCommandBody::Prompt {
            message: "hi".to_owned(),
            images: None,
            streaming_behavior: None,
        },
    );
    assert_eq!(
        serde_json::to_value(&bare).expect("serialization/parse succeeds in test fixture"),
        json!({"id": "req_2", "type": "prompt", "message": "hi"})
    );

    let full = RpcCommand::new(
        Some("req_3".to_owned()),
        RpcCommandBody::Prompt {
            message: "hi".to_owned(),
            images: Some(vec![json!({"type": "image", "url": "x"})]),
            streaming_behavior: Some(StreamingBehavior::FollowUp),
        },
    );
    assert_eq!(
        serde_json::to_value(&full).expect("serialization/parse succeeds in test fixture"),
        json!({
            "id": "req_3",
            "type": "prompt",
            "message": "hi",
            "images": [{"type": "image", "url": "x"}],
            "streamingBehavior": "followUp",
        })
    );
}

#[test]
fn command_prompt_round_trips_through_deserialize() {
    let wire = json!({
        "id": "req_4",
        "type": "prompt",
        "message": "hello",
        "streamingBehavior": "steer",
    });
    let back: RpcCommand =
        serde_json::from_value(wire.clone()).expect("serialization/parse succeeds in test fixture");
    assert_eq!(
        serde_json::to_value(&back).expect("serialization/parse succeeds in test fixture"),
        wire
    );
    match &back.body {
        RpcCommandBody::Prompt {
            streaming_behavior, ..
        } => {
            assert_eq!(streaming_behavior, &Some(StreamingBehavior::Steer));
        }
        _ => panic!("wrong body"),
    }
}

// ---------------------------------------------------------------------------
// Assistant message event variants — every family from protocol-notes.md.
// ---------------------------------------------------------------------------

fn partial_stub() -> Value {
    json!({"role": "assistant", "content": []})
}

#[test]
fn assistant_start_variant() {
    let raw = json!({"type": "start", "partial": partial_stub()});
    let e = assistant_event(raw.clone());
    assert!(matches!(e.kind, AssistantMessageEventKind::Start));
    assert_eq!(e.partial(), Some(&partial_stub()));
    roundtrip_value(&raw);
}

#[test]
fn assistant_text_family() {
    let start = json!({"type": "text_start", "contentIndex": 0, "partial": partial_stub()});
    let delta =
        json!({"type": "text_delta", "contentIndex": 0, "delta": "hi", "partial": partial_stub()});
    let end = json!({"type": "text_end", "contentIndex": 0, "content": "hi there", "partial": partial_stub()});

    match assistant_event(start.clone()).kind {
        AssistantMessageEventKind::TextStart { content_index: 0 } => (),
        other => panic!("{other:?}"),
    }
    match assistant_event(delta.clone()).kind {
        AssistantMessageEventKind::TextDelta {
            content_index: 0,
            delta: d,
        } if d == "hi" => (),
        other => panic!("{other:?}"),
    }
    match assistant_event(end.clone()).kind {
        AssistantMessageEventKind::TextEnd {
            content_index: 0,
            content: c,
        } if c == "hi there" => (),
        other => panic!("{other:?}"),
    }
}

#[test]
fn assistant_thinking_family() {
    let delta = json!({"type": "thinking_delta", "contentIndex": 1, "delta": "hmm", "partial": partial_stub()});
    let end = json!({"type": "thinking_end", "contentIndex": 1, "content": "hmm ok", "partial": partial_stub()});
    match assistant_event(delta).kind {
        AssistantMessageEventKind::ThinkingDelta {
            content_index: 1,
            delta: d,
        } if d == "hmm" => (),
        other => panic!("{other:?}"),
    }
    match assistant_event(end).kind {
        AssistantMessageEventKind::ThinkingEnd {
            content_index: 1,
            content: c,
        } if c == "hmm ok" => (),
        other => panic!("{other:?}"),
    }
}

#[test]
fn assistant_image_end_preserves_content_value() {
    let raw = json!({
        "type": "image_end",
        "contentIndex": 2,
        "content": {"type": "image", "source": {"type": "base64", "data": "AAAA"}},
        "partial": partial_stub(),
    });
    let e = assistant_event(raw.clone());
    match e.kind {
        AssistantMessageEventKind::ImageEnd { content_index: 2 } => (),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        e.image_content()
            .expect("serialization/parse succeeds in test fixture")["source"]["data"],
        json!("AAAA")
    );
    roundtrip_value(&raw);
}

#[test]
fn assistant_toolcall_family_preserves_toolcall_value() {
    let start = json!({"type": "toolcall_start", "contentIndex": 3, "partial": partial_stub()});
    let delta = json!({"type": "toolcall_delta", "contentIndex": 3, "delta": "{\"x\":", "partial": partial_stub()});
    let end = json!({
        "type": "toolcall_end",
        "contentIndex": 3,
        "toolCall": {
            "type": "toolCall",
            "id": "tc_1",
            "name": "read",
            "arguments": {"path": "foo"},
            "thoughtSignature": "sig",
            "rawBlock": {"provider": "x"},
        },
        "partial": partial_stub(),
    });
    assert!(matches!(
        assistant_event(start).kind,
        AssistantMessageEventKind::ToolcallStart { content_index: 3 }
    ));
    match assistant_event(delta).kind {
        AssistantMessageEventKind::ToolcallDelta {
            content_index: 3,
            delta: d,
        } if d == "{\"x\":" => (),
        other => panic!("{other:?}"),
    }
    let e = assistant_event(end.clone());
    assert!(matches!(
        e.kind,
        AssistantMessageEventKind::ToolcallEnd { content_index: 3 }
    ));
    assert_eq!(
        e.tool_call()
            .expect("serialization/parse succeeds in test fixture")["id"],
        json!("tc_1")
    );
    assert_eq!(
        e.tool_call()
            .expect("serialization/parse succeeds in test fixture")["rawBlock"]["provider"],
        json!("x")
    );
    roundtrip_value(&end);
}

#[test]
fn assistant_done_reasons() {
    for reason in ["stop", "length", "toolUse"] {
        let raw = json!({"type": "done", "reason": reason, "message": partial_stub()});
        let e = assistant_event(raw.clone());
        match (&e.kind, reason) {
            (
                AssistantMessageEventKind::Done {
                    reason: DoneReason::Stop,
                },
                "stop",
            )
            | (
                AssistantMessageEventKind::Done {
                    reason: DoneReason::Length,
                },
                "length",
            )
            | (
                AssistantMessageEventKind::Done {
                    reason: DoneReason::ToolUse,
                },
                "toolUse",
            ) => (),
            _ => panic!("mismatch: {e:?}"),
        }
        assert!(e.done_message().is_some());
    }
}

#[test]
fn assistant_error_reasons() {
    for reason in ["aborted", "error"] {
        let raw = json!({"type": "error", "reason": reason, "error": partial_stub()});
        let e = assistant_event(raw);
        match (&e.kind, reason) {
            (
                AssistantMessageEventKind::Error {
                    reason: ErrorReason::Aborted,
                },
                "aborted",
            )
            | (
                AssistantMessageEventKind::Error {
                    reason: ErrorReason::Error,
                },
                "error",
            ) => (),
            _ => panic!("mismatch: {e:?}"),
        }
        assert!(e.error_message().is_some());
    }
}

#[test]
fn assistant_unknown_variant_preserves_raw() {
    let raw = json!({
        "type": "brand_new_2027_variant",
        "contentIndex": 9,
        "custom": {"provider": "future"},
        "partial": partial_stub(),
    });
    let e = assistant_event(raw.clone());
    match &e.kind {
        AssistantMessageEventKind::Unknown { type_field } => {
            assert_eq!(type_field.as_deref(), Some("brand_new_2027_variant"));
        }
        other => panic!("expected Unknown, got {other:?}"),
    }
    assert_eq!(e.raw, raw);
    // Round-trip through serialize preserves everything.
    let re = serde_json::to_value(&e).expect("serialization/parse succeeds in test fixture");
    assert_eq!(re, raw);
}

// ---------------------------------------------------------------------------
// message_update envelope carries typed event + full raw.
// ---------------------------------------------------------------------------

#[test]
fn message_update_envelope_exposes_typed_event_and_message() {
    let raw = json!({
        "type": "message_update",
        "message": {"role": "assistant", "content": [{"type": "text", "text": "hi"}]},
        "assistantMessageEvent": {
            "type": "text_delta",
            "contentIndex": 0,
            "delta": "hi",
            "partial": {"role": "assistant", "content": [{"type": "text", "text": "hi"}]},
        },
    });
    let f = parse_frame(raw.clone());
    match &f.kind {
        IncomingFrameKind::MessageUpdate(MessageUpdateFrame {
            assistant_message_event,
            message,
            ..
        }) => {
            assert!(matches!(
                assistant_message_event.kind,
                AssistantMessageEventKind::TextDelta {
                    content_index: 0,
                    ..
                }
            ));
            assert_eq!(message["content"][0]["text"], json!("hi"));
        }
        other => panic!("expected MessageUpdate, got {other:?}"),
    }
    assert_eq!(f.raw, raw);
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

// ---------------------------------------------------------------------------
// Tool lifecycle frames.
// ---------------------------------------------------------------------------

#[test]
fn tool_execution_start_preserves_intent_and_open_args() {
    let raw = json!({
        "type": "tool_execution_start",
        "toolCallId": "tc_1",
        "toolName": "read",
        "args": {"path": "foo", "future": {"x": 1}},
        "intent": "peek at foo",
    });
    let f = parse_frame(raw.clone());
    match &f.kind {
        IncomingFrameKind::ToolExecutionStart(ToolExecutionStartFrame {
            tool_call_id,
            tool_name,
            args,
            intent,
        }) => {
            assert_eq!(tool_call_id, "tc_1");
            assert_eq!(tool_name, "read");
            assert_eq!(args["future"]["x"], json!(1));
            assert_eq!(intent.as_deref(), Some("peek at foo"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn tool_execution_start_intent_optional_absence_preserved() {
    let raw = json!({
        "type": "tool_execution_start",
        "toolCallId": "tc_2",
        "toolName": "bash",
        "args": {"cmd": "ls"},
    });
    let f = parse_frame(raw.clone());
    match &f.kind {
        IncomingFrameKind::ToolExecutionStart(s) => assert!(s.intent.is_none()),
        other => panic!("{other:?}"),
    }
    // No fabricated intent field on re-serialize.
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn tool_execution_update_frame() {
    let raw = json!({
        "type": "tool_execution_update",
        "toolCallId": "tc_1",
        "toolName": "read",
        "args": {"path": "foo"},
        "partialResult": {"bytes": 42},
    });
    let f = parse_frame(raw.clone());
    match &f.kind {
        IncomingFrameKind::ToolExecutionUpdate(ToolExecutionUpdateFrame {
            partial_result, ..
        }) => {
            assert_eq!(partial_result["bytes"], json!(42));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn tool_execution_end_is_error_optional_absence_preserved() {
    let raw = json!({
        "type": "tool_execution_end",
        "toolCallId": "tc_1",
        "toolName": "read",
        "result": {"content": [{"type": "text", "text": "hi"}]},
    });
    let f = parse_frame(raw.clone());
    match &f.kind {
        IncomingFrameKind::ToolExecutionEnd(ToolExecutionEndFrame { is_error, .. }) => {
            assert!(
                is_error.is_none(),
                "isError absence must not default to false"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn tool_execution_end_is_error_true_survives_round_trip() {
    let raw = json!({
        "type": "tool_execution_end",
        "toolCallId": "tc_1",
        "toolName": "bash",
        "result": {"exitCode": 1},
        "isError": true,
    });
    let f = parse_frame(raw.clone());
    match &f.kind {
        IncomingFrameKind::ToolExecutionEnd(e) => assert_eq!(e.is_error, Some(true)),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

// ---------------------------------------------------------------------------
// Extension UI, host tool/URI, subagents, and unknown top-level fallback.
// ---------------------------------------------------------------------------

#[test]
fn extension_ui_open_url_preserves_launch_url_and_instructions() {
    let raw = json!({
        "type": "extension_ui_request",
        "id": "u1",
        "method": "open_url",
        "url": "https://example.com/oauth?token=x",
        "launchUrl": "http://127.0.0.1:5555/",
        "instructions": "Open in browser",
    });
    let f = parse_frame(raw.clone());
    if let IncomingFrameKind::ExtensionUiRequest(r) = &f.kind {
        assert_eq!(r.id, "u1");
        // Method typed decoding covered structurally.
        assert!(matches!(
            r.method,
            omp_rpc_client::frames::ExtensionUiMethod::OpenUrl { .. }
        ));
    } else {
        panic!("expected ExtensionUiRequest");
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn host_tool_call_decodes() {
    let raw = json!({
        "type": "host_tool_call",
        "id": "h1",
        "toolCallId": "tc_9",
        "toolName": "custom",
        "arguments": {"a": 1},
    });
    let f = parse_frame(raw.clone());
    if let IncomingFrameKind::HostToolCall(r) = &f.kind {
        assert_eq!(r.id, "h1");
        assert_eq!(r.tool_call_id, "tc_9");
        assert_eq!(r.tool_name, "custom");
        assert_eq!(r.arguments["a"], json!(1));
    } else {
        panic!("expected HostToolCall");
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn host_uri_request_decodes_with_optional_content() {
    let raw = json!({
        "type": "host_uri_request",
        "id": "u2",
        "operation": "write",
        "url": "db://users/42",
        "content": "hello",
    });
    let f = parse_frame(raw.clone());
    if let IncomingFrameKind::HostUriRequest(r) = &f.kind {
        assert_eq!(r.operation, omp_rpc_client::frames::HostUriOperation::Write);
        assert_eq!(r.content.as_deref(), Some("hello"));
    } else {
        panic!("expected HostUriRequest");
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn subagent_lifecycle_payload_stays_lossless() {
    let raw = json!({
        "type": "subagent_lifecycle",
        "payload": {"id": "s1", "kind": "started", "future": [1, 2]},
    });
    let f = parse_frame(raw.clone());
    if let IncomingFrameKind::SubagentLifecycle(p) = &f.kind {
        assert_eq!(p.payload["future"][1], json!(2));
    } else {
        panic!("expected SubagentLifecycle");
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn prompt_result_side_channel_decodes() {
    let raw = json!({"type": "prompt_result", "id": "req_2", "agentInvoked": false});
    let f = parse_frame(raw.clone());
    if let IncomingFrameKind::PromptResult(p) = &f.kind {
        assert_eq!(p.id.as_deref(), Some("req_2"));
        assert!(!p.agent_invoked);
    } else {
        panic!("expected PromptResult");
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn rpc_chunk_and_frame_error_decode() {
    let chunk = json!({
        "type": "rpc_chunk",
        "chunkId": "c1",
        "index": 0,
        "count": 2,
        "byteLength": 512,
        "data": "aGVsbG8=",
    });
    match parse_frame(chunk.clone()).kind {
        IncomingFrameKind::RpcChunk(c) => {
            assert_eq!(c.chunk_id, "c1");
            assert_eq!(c.count, 2);
            assert_eq!(c.byte_length, 512);
        }
        other => panic!("{other:?}"),
    }

    let err = json!({
        "type": "rpc_frame_error",
        "originalType": "message_update",
        "error": "RPC frame exceeded the transport limit",
    });
    let f = parse_frame(err.clone());
    if let IncomingFrameKind::RpcFrameError(e) = &f.kind {
        assert_eq!(e.original_type.as_deref(), Some("message_update"));
        assert!(e.error.contains("exceeded"));
    } else {
        panic!("expected RpcFrameError");
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        err
    );
}

#[test]
fn unknown_top_level_frame_preserves_full_raw() {
    let raw = json!({
        "type": "brand_new_2028_frame",
        "payload": {"deep": {"arr": [1, 2, 3]}},
        "flag": true,
    });
    let f = parse_frame(raw.clone());
    match &f.kind {
        IncomingFrameKind::Unknown { type_field } => {
            assert_eq!(type_field.as_deref(), Some("brand_new_2028_frame"));
        }
        other => panic!("expected Unknown, got {other:?}"),
    }
    assert_eq!(f.raw, raw);
    let re = serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture");
    assert_eq!(re, raw);
}

#[test]
fn frame_without_type_is_unknown_with_none_field() {
    let raw = json!({"weird": true});
    let f = parse_frame(raw.clone());
    match &f.kind {
        IncomingFrameKind::Unknown { type_field } => assert!(type_field.is_none()),
        other => panic!("{other:?}"),
    }
    assert_eq!(f.raw, raw);
}

// ---------------------------------------------------------------------------
// Notice + thinking_level_changed carry their optional fields losslessly.
// ---------------------------------------------------------------------------

#[test]
fn notice_frame_source_optional() {
    let raw = json!({"type": "notice", "level": "warning", "message": "check"});
    let f = parse_frame(raw.clone());
    if let IncomingFrameKind::Notice(n) = &f.kind {
        assert!(n.source.is_none());
        assert_eq!(n.message, "check");
    } else {
        panic!();
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn thinking_level_changed_all_optional() {
    // All three fields optional per 17.2.10 — absent object still decodes.
    let raw = json!({"type": "thinking_level_changed"});
    match parse_frame(raw.clone()).kind {
        IncomingFrameKind::ThinkingLevelChanged(t) => {
            assert!(t.thinking_level.is_none());
            assert!(t.configured.is_none());
            assert!(t.resolved.is_none());
        }
        other => panic!("{other:?}"),
    }

    let raw = json!({
        "type": "thinking_level_changed",
        "thinkingLevel": "auto",
        "configured": {"selector": "auto"},
        "resolved": "high",
    });
    let f = parse_frame(raw.clone());
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

// ---------------------------------------------------------------------------
// agent_end / turn_end retain open payloads.
// ---------------------------------------------------------------------------

#[test]
fn agent_end_optional_fields_preserved() {
    let raw = json!({
        "type": "agent_end",
        "messages": [{"role": "assistant", "content": []}],
        "telemetry": {"tokens": 100},
        "coverage": {"tools": 3},
        "isTerminal": false,
    });
    let f = parse_frame(raw.clone());
    if let IncomingFrameKind::AgentEnd(a) = &f.kind {
        assert_eq!(
            a.telemetry
                .as_ref()
                .expect("serialization/parse succeeds in test fixture")["tokens"],
            json!(100)
        );
        assert_eq!(a.is_terminal, Some(false));
    } else {
        panic!();
    }
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}

#[test]
fn turn_end_message_and_tool_results_lossless() {
    let raw = json!({
        "type": "turn_end",
        "message": {"role": "assistant", "content": []},
        "toolResults": [{"toolCallId": "tc_1", "result": {"ok": true}}],
    });
    let f = parse_frame(raw.clone());
    assert!(matches!(f.kind, IncomingFrameKind::TurnEnd(_)));
    assert_eq!(
        serde_json::to_value(&f).expect("serialization/parse succeeds in test fixture"),
        raw
    );
}
