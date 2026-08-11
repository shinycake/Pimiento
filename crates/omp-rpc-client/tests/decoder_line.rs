//! Line-splitter validation tests. Depend only on public exports the
//! integration wiring must provide: `decoder::LineDecoder`,
//! `decoder::MAX_RPC_FRAME_BYTES`, `error::RpcError`.

use omp_rpc_client::decoder::{LineDecoder, MAX_RPC_FRAME_BYTES};
use omp_rpc_client::error::RpcError;
use serde_json::{Value, json};

fn drain(dec: &mut LineDecoder, bytes: &[u8]) -> Result<Vec<Value>, RpcError> {
    let mut out = Vec::new();
    dec.feed(bytes, |v| {
        out.push(v);
        Ok(())
    })?;
    Ok(out)
}

fn encode(value: &Value) -> String {
    serde_json::to_string(value).expect("serde_json::to_string on Value never fails")
}

#[test]
fn splits_multiple_frames_in_one_feed() {
    let mut d = LineDecoder::new();
    let mut input = String::new();
    input.push_str(&encode(&json!({"a": 1})));
    input.push('\n');
    input.push_str(&encode(&json!({"b": 2})));
    input.push('\n');
    let out = drain(&mut d, input.as_bytes()).expect("valid two-frame stream");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0], json!({"a": 1}));
    assert_eq!(out[1], json!({"b": 2}));
    d.eof().expect("clean eof after both frames delivered");
}

#[test]
fn joins_partial_writes_across_feeds() {
    let mut d = LineDecoder::new();
    let text = r#"{"hello":"world"}"#;
    let (a, b) = text.split_at(5);
    let out_a = drain(&mut d, a.as_bytes()).expect("partial input is valid");
    assert!(out_a.is_empty());
    assert!(d.has_pending());
    let mut second = String::from(b);
    second.push('\n');
    let out_b = drain(&mut d, second.as_bytes()).expect("completed frame parses");
    assert_eq!(out_b, vec![json!({"hello": "world"})]);
    d.eof().expect("clean eof after completion");
}

#[test]
fn accepts_crlf_terminators() {
    let mut d = LineDecoder::new();
    let input = "{\"x\":1}\r\n{\"y\":2}\r\n";
    let out = drain(&mut d, input.as_bytes()).expect("CRLF terminators accepted");
    assert_eq!(out, vec![json!({"x": 1}), json!({"y": 2})]);
}

#[test]
fn rejects_non_object_top_level() {
    let mut d = LineDecoder::new();
    let err = drain(&mut d, b"[1,2,3]\n").expect_err("array top-level must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn rejects_invalid_utf8() {
    let mut d = LineDecoder::new();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"{\"k\":\"");
    bytes.extend_from_slice(&[0xFFu8, 0xFEu8]);
    bytes.extend_from_slice(b"\"}\n");
    let err = drain(&mut d, &bytes).expect_err("invalid utf-8 must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn rejects_invalid_json() {
    let mut d = LineDecoder::new();
    let err = drain(&mut d, b"{not-json}\n").expect_err("invalid JSON must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn rejects_oversize_physical_frame() {
    let mut d = LineDecoder::new();
    // Build a valid JSON object whose serialized length + '\n' exceeds the cap.
    let filler = "x".repeat(MAX_RPC_FRAME_BYTES);
    let obj = encode(&json!({ "k": filler }));
    let mut buf = obj.into_bytes();
    buf.push(b'\n');
    let err = drain(&mut d, &buf).expect_err("frame beyond limit must be rejected");
    match err {
        RpcError::FrameTooLarge { limit, .. } => assert_eq!(limit, MAX_RPC_FRAME_BYTES),
        other => panic!("expected FrameTooLarge, got {other:?}"),
    }
}

#[test]
fn rejects_oversize_partial_no_newline() {
    let mut d = LineDecoder::new();
    let big = vec![b'x'; MAX_RPC_FRAME_BYTES]; // no newline anywhere
    let err = drain(&mut d, &big).expect_err("partial past the cap must be rejected");
    assert!(matches!(err, RpcError::FrameTooLarge { .. }));
}

#[test]
fn eof_with_pending_bytes_errors() {
    let mut d = LineDecoder::new();
    drain(&mut d, b"{\"partial\":").expect("partial feed accepted before eof");
    let err = d.eof().expect_err("eof with pending bytes must error");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn use_after_error_is_poisoned() {
    let mut d = LineDecoder::new();
    let _ = drain(&mut d, b"[bad]\n").expect_err("first bad feed errors");
    let err = drain(&mut d, b"{\"ok\":true}\n").expect_err("decoder stays poisoned");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn empty_feed_is_noop() {
    let mut d = LineDecoder::new();
    let out = drain(&mut d, b"").expect("empty feed is a no-op");
    assert!(out.is_empty());
    d.eof().expect("clean eof after empty feed");
}

#[test]
fn byte_by_byte_delivery() {
    let mut d = LineDecoder::new();
    let msg = r#"{"a":42,"b":"hi"}"#.to_string() + "\n";
    let mut collected = Vec::new();
    for byte in msg.bytes() {
        d.feed(&[byte], |v| {
            collected.push(v);
            Ok(())
        })
        .expect("each single-byte feed remains valid until the terminator");
    }
    assert_eq!(collected, vec![json!({"a": 42, "b": "hi"})]);
}
