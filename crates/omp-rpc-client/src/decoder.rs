//! Byte-level newline splitter and protocol-v2 chunk reassembler.
//!
//! Mirrors the installed OMP 17.2.10 `RpcFrameDecoder` semantics documented in
//! `docs/protocol-notes.md`. All validation errors are reported as
//! [`RpcError::ProtocolViolation`] or [`RpcError::FrameTooLarge`]; the caller is
//! expected to treat any error as fatal for the current process' stream.
//!
//! Both decoders are single-consumer state machines. `LineDecoder` accumulates
//! bytes into complete newline-terminated JSON objects; `ChunkReassembler`
//! coalesces `rpc_chunk` frames back into a single logical JSON object.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::Value;

use crate::error::RpcError;

/// Maximum UTF-8 size of a single physical newline-delimited frame, including
/// its terminating newline byte.
pub const MAX_RPC_FRAME_BYTES: usize = 1024 * 1024;
/// Maximum UTF-8 size of one logical (reassembled) frame.
pub const MAX_RPC_REASSEMBLED_BYTES: usize = 64 * 1024 * 1024;
/// Maximum decoded payload size of a single `rpc_chunk` frame.
pub const RPC_CHUNK_PAYLOAD_BYTES: usize = 256 * 1024;
/// Maximum permitted `count` metadata value — `ceil(MAX_REASSEMBLED / PAYLOAD)`.
///
/// Expressed as `u64` because chunk metadata is decoded as a JS-safe unsigned
/// integer; the numeric value (`256`) trivially fits.
pub const RPC_CHUNK_MAX_COUNT: u64 =
    MAX_RPC_REASSEMBLED_BYTES.div_ceil(RPC_CHUNK_PAYLOAD_BYTES) as u64;
/// Maximum permitted `chunkId` length in UTF-16 code units (matches TS
/// `string.length`).
pub const RPC_CHUNK_ID_MAX_UTF16: usize = 128;

/// Largest JS `Number.isSafeInteger`, i.e. `2^53 − 1`.
const JS_SAFE_INTEGER_MAX: u64 = (1_u64 << 53) - 1;

/// Splits an incoming byte stream into complete newline-terminated JSON object
/// values. CRLF terminators are accepted (the trailing `\r` is stripped before
/// parsing). Any partial line exceeding [`MAX_RPC_FRAME_BYTES`] is rejected as
/// [`RpcError::FrameTooLarge`]; any non-UTF-8, non-JSON, or non-object payload
/// is rejected as [`RpcError::ProtocolViolation`].
///
/// After an error the decoder is poisoned — callers MUST drop the instance and
/// create a fresh one to continue.
#[derive(Debug, Default)]
pub struct LineDecoder {
    /// Bytes of the in-flight (not-yet-newline-terminated) physical frame.
    buffer: Vec<u8>,
    poisoned: bool,
}

impl LineDecoder {
    /// Create a new empty decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of bytes. Emits every physical frame completed by the input
    /// via `sink`, in order. The sink is called with the parsed JSON object
    /// value; the caller can hand it to [`ChunkReassembler::push`].
    ///
    /// Zero-length input is a no-op.
    ///
    /// # Errors
    /// Returns [`RpcError::FrameTooLarge`] if a physical line exceeds
    /// [`MAX_RPC_FRAME_BYTES`] (including its newline), or
    /// [`RpcError::ProtocolViolation`] on invalid UTF-8, invalid JSON, or a
    /// non-object top-level JSON value.
    pub fn feed<F>(&mut self, mut bytes: &[u8], mut sink: F) -> Result<(), RpcError>
    where
        F: FnMut(Value) -> Result<(), RpcError>,
    {
        if self.poisoned {
            return Err(RpcError::ProtocolViolation {
                detail: "line decoder used after error".into(),
            });
        }
        while let Some(nl) = memchr_newline(bytes) {
            // `nl` is the index of the '\n' in `bytes`. The physical frame
            // includes bytes up to and including that newline, prefixed by any
            // previously buffered partial line.
            let piece = &bytes[..=nl];
            let total = self.buffer.len().saturating_add(piece.len());
            if total > MAX_RPC_FRAME_BYTES {
                self.poison();
                return Err(RpcError::FrameTooLarge {
                    size: total,
                    limit: MAX_RPC_FRAME_BYTES,
                });
            }
            let value = if self.buffer.is_empty() {
                // Single-copy fast path: parse straight from the input slice.
                parse_physical_line(&piece[..nl]).map_err(|e| self.poison_return(e))?
            } else {
                self.buffer.extend_from_slice(piece);
                let end = self.buffer.len() - 1; // strip trailing '\n'
                let v =
                    parse_physical_line(&self.buffer[..end]).map_err(|e| self.poison_return(e))?;
                self.buffer.clear();
                v
            };
            sink(value).map_err(|e| self.poison_return(e))?;
            bytes = &bytes[nl + 1..];
        }
        // No newline in remaining bytes; append to buffer.
        if !bytes.is_empty() {
            let projected = self.buffer.len().saturating_add(bytes.len());
            // Even without a newline yet, a partial >= MAX_RPC_FRAME_BYTES can
            // never be completed within the limit (the newline alone would push
            // it over). Reject preemptively.
            if projected >= MAX_RPC_FRAME_BYTES {
                self.poison();
                return Err(RpcError::FrameTooLarge {
                    size: projected + 1,
                    limit: MAX_RPC_FRAME_BYTES,
                });
            }
            self.buffer.extend_from_slice(bytes);
        }
        Ok(())
    }

    /// Signal end-of-stream. Errors if a partial (unterminated) physical frame
    /// is still buffered.
    ///
    /// # Errors
    /// Returns [`RpcError::ProtocolViolation`] if the stream ended with an
    /// unterminated line.
    pub fn eof(&mut self) -> Result<(), RpcError> {
        if self.poisoned {
            return Err(RpcError::ProtocolViolation {
                detail: "line decoder used after error".into(),
            });
        }
        if !self.buffer.is_empty() {
            let leftover = self.buffer.len();
            self.poison();
            return Err(RpcError::ProtocolViolation {
                detail: format!("stream ended with {leftover} buffered bytes and no newline"),
            });
        }
        Ok(())
    }

    /// Whether any bytes are buffered awaiting a newline. Diagnostic only.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.buffer.is_empty()
    }

    fn poison(&mut self) {
        self.poisoned = true;
        self.buffer.clear();
        self.buffer.shrink_to_fit();
    }

    fn poison_return(&mut self, err: RpcError) -> RpcError {
        self.poison();
        err
    }
}

fn memchr_newline(bytes: &[u8]) -> Option<usize> {
    bytes.iter().position(|&b| b == b'\n')
}

fn parse_physical_line(line_bytes: &[u8]) -> Result<Value, RpcError> {
    // Strip a single trailing CR to accept CRLF.
    let payload = if line_bytes.last() == Some(&b'\r') {
        &line_bytes[..line_bytes.len() - 1]
    } else {
        line_bytes
    };
    // Validate UTF-8 up front so we can report a precise error.
    let text = std::str::from_utf8(payload).map_err(|e| RpcError::ProtocolViolation {
        detail: format!("invalid utf-8 in physical frame: {e}"),
    })?;
    let value: Value = serde_json::from_str(text).map_err(|e| RpcError::ProtocolViolation {
        detail: format!("invalid json in physical frame: {e}"),
    })?;
    if !value.is_object() {
        return Err(RpcError::ProtocolViolation {
            detail: "physical frame must be a JSON object".into(),
        });
    }
    Ok(value)
}

/// Reassembles v2 chunked logical frames. Feed each parsed physical
/// [`Value`] from a [`LineDecoder`]; the reassembler either passes through a
/// non-chunk object frame or, once a complete `rpc_chunk` sequence has arrived,
/// returns the reconstructed logical object.
///
/// After a validation error the reassembler drops its pending state and
/// becomes usable again for a fresh sequence; the caller should nonetheless
/// treat the transport as compromised.
#[derive(Debug, Default)]
pub struct ChunkReassembler {
    pending: Option<Pending>,
}

#[derive(Debug)]
struct Pending {
    chunk_id: String,
    count: u32,
    byte_length: usize,
    next_index: u32,
    received: usize,
    /// Accumulated decoded payload. Reserved once with the declared byte length
    /// so we only pay the copy per chunk-decode.
    payload: Vec<u8>,
}

/// Fully-validated metadata extracted from a single `rpc_chunk` frame.
struct ChunkMeta<'a> {
    chunk_id: &'a str,
    index: u32,
    count: u32,
    byte_length: usize,
}

impl ChunkReassembler {
    /// Create a new empty reassembler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a chunk sequence is currently in-flight.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Feed one parsed physical JSON value.
    ///
    /// Returns `Ok(Some(value))` when a complete logical frame is available
    /// (either the value itself if it is not an `rpc_chunk`, or the assembled
    /// object after the last chunk); returns `Ok(None)` while awaiting further
    /// chunks.
    ///
    /// # Errors
    /// [`RpcError::ProtocolViolation`] on any of the validation failures listed
    /// in the module docs; [`RpcError::FrameTooLarge`] when a chunk payload or
    /// declared reassembled length exceeds the configured limits.
    pub fn push(&mut self, value: Value) -> Result<Option<Value>, RpcError> {
        if !is_rpc_chunk(&value) {
            if self.pending.is_some() {
                self.pending = None;
                return Err(RpcError::ProtocolViolation {
                    detail: "non-chunk frame during pending rpc_chunk sequence".into(),
                });
            }
            if !value.is_object() {
                return Err(RpcError::ProtocolViolation {
                    detail: "physical frame must be a JSON object".into(),
                });
            }
            return Ok(Some(value));
        }
        self.push_chunk(&value).inspect_err(|_| {
            // Poisoned sequence state is cleared so the reassembler can be
            // reused after the caller decides how to handle the error.
            self.pending = None;
        })
    }

    fn push_chunk(&mut self, value: &Value) -> Result<Option<Value>, RpcError> {
        let obj = value
            .as_object()
            .ok_or_else(|| protocol("rpc_chunk must be a JSON object"))?;
        let meta = extract_chunk_meta(obj)?;
        let bytes = decode_chunk_payload(obj)?;

        // --- sequence bookkeeping -------------------------------------------
        if self.pending.is_none() {
            if meta.index != 0 {
                return Err(protocol("rpc_chunk sequence must start at index 0"));
            }
            self.pending = Some(Pending {
                chunk_id: meta.chunk_id.to_owned(),
                count: meta.count,
                byte_length: meta.byte_length,
                next_index: 0,
                received: 0,
                payload: Vec::with_capacity(meta.byte_length),
            });
        }
        let pending = self.pending.as_mut().expect("just initialized");
        if pending.chunk_id != meta.chunk_id
            || pending.count != meta.count
            || pending.byte_length != meta.byte_length
            || pending.next_index != meta.index
        {
            return Err(protocol("rpc_chunk sequence mismatch"));
        }

        pending.received = pending.received.saturating_add(bytes.len());
        if pending.received > pending.byte_length {
            return Err(protocol("rpc_chunk sequence exceeds declared length"));
        }
        pending.payload.extend_from_slice(&bytes);
        pending.next_index += 1;

        if pending.next_index < pending.count {
            return Ok(None);
        }
        if pending.received != pending.byte_length {
            return Err(protocol("rpc_chunk sequence length mismatch"));
        }

        // Finalize.
        let pending = self.pending.take().expect("finalization");
        let text =
            std::str::from_utf8(&pending.payload).map_err(|e| RpcError::ProtocolViolation {
                detail: format!("rpc_chunk reassembled payload is not valid utf-8: {e}"),
            })?;
        let frame: Value = serde_json::from_str(text).map_err(|e| RpcError::ProtocolViolation {
            detail: format!("rpc_chunk reassembled payload is not valid json: {e}"),
        })?;
        if !frame.is_object() {
            return Err(protocol(
                "rpc_chunk reassembled frame must be a JSON object",
            ));
        }
        Ok(Some(frame))
    }
}

/// Validate every non-payload field of an `rpc_chunk` object and return the
/// resulting numeric metadata narrowed to native widths.
fn extract_chunk_meta(obj: &serde_json::Map<String, Value>) -> Result<ChunkMeta<'_>, RpcError> {
    let chunk_id = obj
        .get("chunkId")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol("rpc_chunk.chunkId must be a string"))?;
    if chunk_id.is_empty() {
        return Err(protocol("rpc_chunk.chunkId must be non-empty"));
    }
    // Match TS `string.length` (UTF-16 code units).
    if chunk_id.encode_utf16().count() > RPC_CHUNK_ID_MAX_UTF16 {
        return Err(protocol("rpc_chunk.chunkId exceeds 128 code units"));
    }

    let index = extract_safe_u64(obj.get("index"), "index")?;
    let count = extract_safe_u64(obj.get("count"), "count")?;
    let byte_length = extract_safe_u64(obj.get("byteLength"), "byteLength")?;

    if count < 2 {
        return Err(protocol("rpc_chunk.count must be >= 2"));
    }
    if count > RPC_CHUNK_MAX_COUNT {
        return Err(protocol("rpc_chunk.count exceeds reassembly bound"));
    }
    if index >= count {
        return Err(protocol("rpc_chunk.index must be < count"));
    }
    // MAX_RPC_FRAME_BYTES fits in u32 (1 MiB); the widening cast is exact.
    if byte_length < MAX_RPC_FRAME_BYTES as u64 {
        return Err(protocol(
            "rpc_chunk.byteLength must be >= physical frame max",
        ));
    }
    if byte_length > MAX_RPC_REASSEMBLED_BYTES as u64 {
        // 64 MiB comfortably fits in usize on every supported target.
        return Err(RpcError::FrameTooLarge {
            size: usize::try_from(byte_length).unwrap_or(usize::MAX),
            limit: MAX_RPC_REASSEMBLED_BYTES,
        });
    }

    // All three values are bounded above by RPC_CHUNK_MAX_COUNT (256) or
    // MAX_RPC_REASSEMBLED_BYTES (64 MiB), which both fit in u32.
    Ok(ChunkMeta {
        chunk_id,
        index: u32::try_from(index).expect("index bounded by count"),
        count: u32::try_from(count).expect("count bounded by RPC_CHUNK_MAX_COUNT"),
        byte_length: usize::try_from(byte_length)
            .expect("byte_length bounded by MAX_RPC_REASSEMBLED_BYTES"),
    })
}

/// Decode the base64 `data` field of an `rpc_chunk` object into its raw
/// payload bytes, applying the canonical-encoding and payload-size checks.
fn decode_chunk_payload(obj: &serde_json::Map<String, Value>) -> Result<Vec<u8>, RpcError> {
    let data_str = obj
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol("rpc_chunk.data must be a string"))?;
    if data_str.is_empty() {
        return Err(protocol("rpc_chunk.data must be non-empty"));
    }
    if !is_canonical_base64(data_str) {
        return Err(protocol("rpc_chunk.data is not canonical base64"));
    }
    let bytes = BASE64_STANDARD
        .decode(data_str)
        .map_err(|e| protocol(&format!("rpc_chunk.data base64 decode failed: {e}")))?;
    // Canonical round-trip check (matches TS re-encode comparison).
    if BASE64_STANDARD.encode(&bytes) != data_str {
        return Err(protocol("rpc_chunk.data is not canonical base64"));
    }
    if bytes.len() > RPC_CHUNK_PAYLOAD_BYTES {
        return Err(RpcError::FrameTooLarge {
            size: bytes.len(),
            limit: RPC_CHUNK_PAYLOAD_BYTES,
        });
    }
    Ok(bytes)
}

fn is_rpc_chunk(value: &Value) -> bool {
    value
        .as_object()
        .and_then(|o| o.get("type"))
        .and_then(Value::as_str)
        == Some("rpc_chunk")
}

/// Extract a non-negative JSON integer that fits `Number.isSafeInteger`.
fn extract_safe_u64(v: Option<&Value>, field: &str) -> Result<u64, RpcError> {
    let value = v.ok_or_else(|| protocol(&format!("rpc_chunk.{field} is required")))?;
    // `as_u64` returns `Some` iff the value is an integer literal within
    // `[0, u64::MAX]`; that rules out negatives, fractions, and NaN/Inf up
    // front and matches the TS `Number.isSafeInteger` predicate once the
    // upper bound is applied.
    match value.as_u64() {
        Some(u) if u <= JS_SAFE_INTEGER_MAX => Ok(u),
        _ => Err(protocol(&format!(
            "rpc_chunk.{field} must be a safe integer"
        ))),
    }
}

/// Enforce the exact TS regex
/// `(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?`.
fn is_canonical_base64(input: &str) -> bool {
    let bytes = input.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    // Length must be a positive multiple of 4.
    if !bytes.len().is_multiple_of(4) {
        return false;
    }
    // Padding is either "==", "=", or absent — all three forms carve a
    // 4-byte tail off the trailing quantum when present.
    let (body, tail) = if bytes.last() == Some(&b'=') {
        (&bytes[..bytes.len() - 4], &bytes[bytes.len() - 4..])
    } else {
        (bytes, &b""[..])
    };
    if !body.iter().copied().all(is_b64_char) {
        return false;
    }
    match tail {
        b"" => true,
        [t0, t1, b'=', b'='] => is_b64_char(*t0) && is_b64_char(*t1),
        [t0, t1, t2, b'='] => is_b64_char(*t0) && is_b64_char(*t1) && is_b64_char(*t2),
        [t0, t1, t2, t3] => {
            is_b64_char(*t0) && is_b64_char(*t1) && is_b64_char(*t2) && is_b64_char(*t3)
        }
        _ => false,
    }
}

fn is_b64_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/'
}

fn protocol(detail: &str) -> RpcError {
    RpcError::ProtocolViolation {
        detail: detail.to_owned(),
    }
}
