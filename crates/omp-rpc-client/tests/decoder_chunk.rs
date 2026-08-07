//! v2 chunk reassembler validation tests + property test for round-trips.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use omp_rpc_client::decoder::{
    ChunkReassembler, MAX_RPC_FRAME_BYTES, MAX_RPC_REASSEMBLED_BYTES, RPC_CHUNK_PAYLOAD_BYTES,
};
use omp_rpc_client::error::RpcError;
use proptest::prelude::*;
use serde_json::{Value, json};

const CHUNK_ID: &str = "abc123";

fn encode(value: &Value) -> String {
    serde_json::to_string(value).expect("serde_json::to_string on Value never fails")
}

fn chunk(chunk_id: &str, index: u32, count: u32, byte_length: usize, data: &[u8]) -> Value {
    json!({
        "type": "rpc_chunk",
        "chunkId": chunk_id,
        "index": index,
        "count": count,
        "byteLength": byte_length,
        "data": B64.encode(data),
    })
}

/// Build a valid JSON object whose serialized UTF-8 size lies in
/// `[target, target + slack]`.
fn logical_frame(target: usize) -> String {
    let base_len = encode(&json!({"k": ""})).len();
    let filler = "a".repeat(target.saturating_sub(base_len));
    encode(&json!({ "k": filler }))
}

/// Split `bytes` into a list of slices, each ≤ `RPC_CHUNK_PAYLOAD_BYTES`, and
/// feed them to `r` under `chunk_id`. Returns the assembled value.
fn feed_split(r: &mut ChunkReassembler, chunk_id: &str, bytes: &[u8]) -> Result<Value, RpcError> {
    let piece_size = RPC_CHUNK_PAYLOAD_BYTES;
    let count_usize = bytes.len().div_ceil(piece_size);
    assert!(count_usize >= 2, "need at least 2 chunks");
    let count = u32::try_from(count_usize).expect("chunk count fits in u32 by construction");
    let mut out = None;
    let mut cursor = 0usize;
    for i in 0..count {
        let end = (cursor + piece_size).min(bytes.len());
        let v = chunk(chunk_id, i, count, bytes.len(), &bytes[cursor..end]);
        out = r.push(v)?;
        cursor = end;
    }
    Ok(out.expect("final chunk must yield a value"))
}

#[test]
fn passthrough_non_chunk_object() {
    let mut r = ChunkReassembler::new();
    let out = r
        .push(json!({"type": "response", "id": 1}))
        .expect("plain object passes through unchanged");
    assert_eq!(out, Some(json!({"type": "response", "id": 1})));
}

#[test]
fn round_trip_multi_chunks() {
    let mut r = ChunkReassembler::new();
    // Force at least 5 chunks: 4 × 256 KiB < 1 MiB, so target > 1 MiB.
    let text = logical_frame(MAX_RPC_FRAME_BYTES + 4096);
    let bytes = text.as_bytes();
    let out = feed_split(&mut r, CHUNK_ID, bytes).expect("valid multi-chunk sequence assembles");
    assert!(out.is_object());
    assert_eq!(encode(&out), text);
}

#[test]
fn non_chunk_frame_mid_sequence_fails() {
    let mut r = ChunkReassembler::new();
    let byte_length = MAX_RPC_FRAME_BYTES;
    let piece = vec![b'x'; RPC_CHUNK_PAYLOAD_BYTES];
    r.push(chunk(CHUNK_ID, 0, 5, byte_length, &piece))
        .expect("first chunk starts a valid sequence");
    let err = r
        .push(json!({"type": "response"}))
        .expect_err("plain object mid-sequence must fail");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
    assert!(!r.has_pending(), "reassembler must clear pending on error");
}

#[test]
fn wrong_start_index_rejected() {
    let mut r = ChunkReassembler::new();
    let piece = vec![b'x'; RPC_CHUNK_PAYLOAD_BYTES];
    let err = r
        .push(chunk(CHUNK_ID, 1, 5, MAX_RPC_FRAME_BYTES, &piece))
        .expect_err("index != 0 at sequence start must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn count_below_two_rejected() {
    let mut r = ChunkReassembler::new();
    let err = r
        .push(chunk(CHUNK_ID, 0, 1, MAX_RPC_FRAME_BYTES, b"data"))
        .expect_err("count < 2 must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn byte_length_below_physical_max_rejected() {
    let mut r = ChunkReassembler::new();
    let err = r
        .push(chunk(CHUNK_ID, 0, 2, 100, b"AAAA"))
        .expect_err("byteLength below physical cap must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn byte_length_above_reassembled_max_rejected() {
    let mut r = ChunkReassembler::new();
    let too_big = MAX_RPC_REASSEMBLED_BYTES + 1;
    let v = json!({
        "type": "rpc_chunk",
        "chunkId": CHUNK_ID,
        "index": 0,
        "count": 2,
        "byteLength": too_big,
        "data": B64.encode(b"x"),
    });
    let err = r
        .push(v)
        .expect_err("byteLength above cap must be rejected");
    assert!(matches!(err, RpcError::FrameTooLarge { .. }));
}

#[test]
fn payload_above_256k_rejected() {
    let mut r = ChunkReassembler::new();
    let big = vec![b'x'; RPC_CHUNK_PAYLOAD_BYTES + 1];
    let err = r
        .push(chunk(CHUNK_ID, 0, 2, MAX_RPC_FRAME_BYTES, &big))
        .expect_err("decoded payload above 256 KiB must be rejected");
    assert!(matches!(err, RpcError::FrameTooLarge { .. }));
}

#[test]
fn non_canonical_base64_rejected() {
    let mut r = ChunkReassembler::new();
    // "AB" is not a valid base64 quantum (must be multiple of 4).
    let v = json!({
        "type": "rpc_chunk",
        "chunkId": CHUNK_ID,
        "index": 0,
        "count": 2,
        "byteLength": MAX_RPC_FRAME_BYTES,
        "data": "AB",
    });
    let err = r
        .push(v)
        .expect_err("non-multiple-of-4 base64 must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn non_canonical_padding_rejected() {
    // "AA==" decodes to 0x00 which re-encodes as "AA==" — canonical.
    // "AB==" decodes to 0x00 but re-encodes as "AA==" — non-canonical.
    let mut r = ChunkReassembler::new();
    let v = json!({
        "type": "rpc_chunk",
        "chunkId": CHUNK_ID,
        "index": 0,
        "count": 2,
        "byteLength": MAX_RPC_FRAME_BYTES,
        "data": "AB==",
    });
    let err = r
        .push(v)
        .expect_err("non-canonical padding must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn empty_chunk_id_rejected() {
    let mut r = ChunkReassembler::new();
    let err = r
        .push(chunk("", 0, 2, MAX_RPC_FRAME_BYTES, b"AAAA"))
        .expect_err("empty chunkId must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn oversize_chunk_id_rejected() {
    let mut r = ChunkReassembler::new();
    let long = "a".repeat(129);
    let err = r
        .push(chunk(&long, 0, 2, MAX_RPC_FRAME_BYTES, b"AAAA"))
        .expect_err("chunkId beyond 128 code units must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn metadata_mismatch_across_chunks_rejected() {
    let mut r = ChunkReassembler::new();
    let piece = vec![b'x'; RPC_CHUNK_PAYLOAD_BYTES];
    r.push(chunk(CHUNK_ID, 0, 5, MAX_RPC_FRAME_BYTES, &piece))
        .expect("first chunk starts a valid sequence");
    // Change chunkId on chunk 1.
    let err = r
        .push(chunk("different", 1, 5, MAX_RPC_FRAME_BYTES, &piece))
        .expect_err("chunkId change mid-sequence must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn received_overflow_rejected() {
    // 5 chunks × 256 KiB = 1 MiB declared, but chunk 5 sends extra bytes.
    let byte_length = MAX_RPC_FRAME_BYTES; // 1 MiB
    let piece = vec![b'x'; RPC_CHUNK_PAYLOAD_BYTES]; // 256 KiB
    let mut r = ChunkReassembler::new();
    for i in 0..4 {
        r.push(chunk(CHUNK_ID, i, 5, byte_length, &piece))
            .expect("first four chunks fit the declared length");
    }
    // Fifth chunk adds any nonzero bytes → overshoot.
    let extra = vec![b'x'; 8];
    let err = r
        .push(chunk(CHUNK_ID, 4, 5, byte_length, &extra))
        .expect_err("total > declared byteLength must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn length_mismatch_on_last_chunk_rejected() {
    // Declared 1 MiB + 100 but final total is short by 50.
    let byte_length = MAX_RPC_FRAME_BYTES + 100;
    let piece = vec![b'x'; RPC_CHUNK_PAYLOAD_BYTES]; // 256 KiB × 4 = 1 MiB
    let mut r = ChunkReassembler::new();
    for i in 0..4 {
        r.push(chunk(CHUNK_ID, i, 5, byte_length, &piece))
            .expect("first four chunks accepted");
    }
    let last = vec![b'x'; 50]; // need 100
    let err = r
        .push(chunk(CHUNK_ID, 4, 5, byte_length, &last))
        .expect_err("short final chunk must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn reassembled_non_utf8_rejected() {
    // 4 chunks × 256 KiB = 1 MiB exactly; last byte flipped to 0xFF.
    let byte_length = MAX_RPC_FRAME_BYTES;
    let piece = vec![b'x'; RPC_CHUNK_PAYLOAD_BYTES];
    let mut last_piece = piece.clone();
    *last_piece
        .last_mut()
        .expect("piece has non-zero length by construction") = 0xFF;
    let mut r = ChunkReassembler::new();
    for i in 0..3 {
        r.push(chunk(CHUNK_ID, i, 4, byte_length, &piece))
            .expect("first three chunks accepted");
    }
    let err = r
        .push(chunk(CHUNK_ID, 3, 4, byte_length, &last_piece))
        .expect_err("non-utf8 reassembly must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn reassembled_non_object_rejected() {
    // Assemble a valid JSON array — must be rejected.
    let inner = "1,".repeat((MAX_RPC_FRAME_BYTES / 2) + 4096);
    let payload_text = format!("[{inner}1]");
    let bytes = payload_text.as_bytes();
    assert!(bytes.len() > MAX_RPC_FRAME_BYTES);
    let mut r = ChunkReassembler::new();
    let err = feed_split(&mut r, CHUNK_ID, bytes)
        .expect_err("reassembled JSON array top-level must be rejected");
    assert!(matches!(err, RpcError::ProtocolViolation { .. }));
}

#[test]
fn reassembler_recovers_after_error() {
    let mut r = ChunkReassembler::new();
    // Cause a mid-sequence non-chunk error.
    let piece = vec![b'x'; RPC_CHUNK_PAYLOAD_BYTES];
    r.push(chunk(CHUNK_ID, 0, 5, MAX_RPC_FRAME_BYTES, &piece))
        .expect("first chunk starts a valid sequence");
    let _ = r
        .push(json!({"type": "response"}))
        .expect_err("non-chunk mid-sequence errors");
    // A fresh valid sequence works.
    let text = logical_frame(MAX_RPC_FRAME_BYTES + 4096);
    let bytes = text.as_bytes();
    let out = feed_split(&mut r, CHUNK_ID, bytes).expect("fresh sequence reassembles after error");
    assert!(out.is_object());
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

/// Build a valid logical UTF-8 JSON object whose serialized size falls in
/// `[MAX_RPC_FRAME_BYTES, MAX_RPC_FRAME_BYTES + slack]`.
fn build_logical_payload(slack: usize) -> String {
    let base = json!({"k": ""});
    let base_len = encode(&base).len();
    let filler = "a".repeat(MAX_RPC_FRAME_BYTES + slack - base_len);
    encode(&json!({ "k": filler }))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Reconstruct payloads > 1 MiB from randomly-sized chunks (bounded by
    /// [`RPC_CHUNK_PAYLOAD_BYTES`]).
    #[test]
    fn reconstructs_random_chunkings(
        slack in 8usize..2048,
        seed in any::<u64>(),
    ) {
        let payload = build_logical_payload(slack);
        let bytes = payload.as_bytes();
        // Deterministic PRNG from seed for repeatability.
        let mut state = seed | 1;
        let mut cuts = Vec::new();
        let mut pos = 0usize;
        while pos < bytes.len() {
            // Advance an xorshift step, get a chunk size in
            // [1, RPC_CHUNK_PAYLOAD_BYTES].
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let modulus = RPC_CHUNK_PAYLOAD_BYTES as u64;
            let step = usize::try_from(state % modulus)
                .expect("modulo of RPC_CHUNK_PAYLOAD_BYTES fits in usize")
                + 1;
            let end = (pos + step).min(bytes.len());
            cuts.push(end);
            pos = end;
        }
        // Must have at least two chunks (payload > 1 MiB, each ≤ 256 KiB).
        prop_assert!(cuts.len() >= 2);

        let mut r = ChunkReassembler::new();
        let count = u32::try_from(cuts.len())
            .expect("chunk count bounded by payload size / minimum step");
        let mut prev = 0usize;
        let mut last = None;
        for (i, &end) in cuts.iter().enumerate() {
            let index = u32::try_from(i).expect("index bounded by count");
            let v = chunk(CHUNK_ID, index, count, bytes.len(), &bytes[prev..end]);
            last = r.push(v)?;
            prev = end;
        }
        let out = last.expect("last chunk must yield frame");
        prop_assert!(out.is_object());
        // Round-trip the reassembled object to check payload integrity.
        prop_assert_eq!(encode(&out), payload);
    }
}
