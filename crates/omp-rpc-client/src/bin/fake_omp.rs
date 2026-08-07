//! Deterministic fake OMP server for `omp-rpc-client` integration tests.
//!
//! This binary speaks the OMP JSONL protocol over stdin/stdout well enough for
//! the real client/supervisor to be exercised end-to-end without any dependency
//! on a real installed `omp`. It is scenario-driven: the test picks one via
//! the `PIMIENTO_FAKE_SCENARIO` environment variable, and every scenario is
//! deterministic (no clocks, no RNG, bounded I/O).
//!
//! Stdout is **protocol frames only**. All diagnostics — including argv
//! capture for the resume scenario — go to stderr with a fixed marker prefix
//! so tests can grep for them.
//!
//! The fake tolerates the argv the real client passes (`--mode rpc-ui`,
//! optional `--no-session`, optional `--resume <ptr>`). It also supports
//! `--version` so that `discovery::probe_version` can classify it as a
//! supported peer without special-casing the tests.
//!
//! Scenarios:
//!
//! | env value              | behavior                                                    |
//! |------------------------|-------------------------------------------------------------|
//! | `handshake_basic`      | ready v2, ack negotiate, ack `get_state` with a small state |
//! | `chunked_large`        | ready v2, ack negotiate, then a >1 MiB chunked response     |
//! | `concurrent_reorder`   | ready v2, ack negotiate, buffers two commands and replies   |
//! |                        | in REVERSE receive order to prove id-based correlation      |
//! | `late_prompt_error`    | ack `prompt`; later emits a second response with same id    |
//! | `idless_response`      | after negotiate emits a response with no `id`               |
//! | `malformed_line`       | after negotiate emits `not-json\n` — must be fatal          |
//! | `interleave_chunk`     | mid-chunk emits a plain frame — must be fatal               |
//! | `mid_stream_eof`       | after negotiate closes stdout mid-frame                     |
//! | `stderr_then_exit`     | writes to stderr and exits with code 42 (no ready)          |
//! | `resume_capture`       | logs argv+env to stderr with marker, then handshake_basic   |

#![forbid(unsafe_code)]
#![allow(clippy::print_stderr)]

use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::ExitCode;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

const MAX_FRAME_BYTES: u64 = 1024 * 1024;
const MAX_REASSEMBLED_BYTES: u64 = 64 * 1024 * 1024;
const CHUNK_PAYLOAD_BYTES: usize = 256 * 1024;

/// Marker prefix on every stderr diagnostic so tests can grep unambiguously.
const DIAG_TAG: &str = "FAKE_OMP:";

/// Marker line preceding argv capture in the `resume_capture` scenario.
const ARGV_MARKER: &str = "FAKE_OMP_ARGV:";

fn ready() -> Value {
    json!({
        "type": "ready",
        "protocolVersion": 1,
        "supportedProtocolVersions": [1, 2],
        "maxFrameBytes": MAX_FRAME_BYTES,
        "maxReassembledFrameBytes": MAX_REASSEMBLED_BYTES,
    })
}

fn diag(msg: &str) {
    eprintln!("{DIAG_TAG} {msg}");
}

/// Write one JSON object as a newline-terminated frame. Flushes so the client
/// can observe the frame before we do anything else. Returns Err on broken
/// pipe so callers can propagate mid-stream EOF cleanly.
fn write_frame(out: &mut impl Write, v: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(v).expect("frame serializes");
    bytes.push(b'\n');
    out.write_all(&bytes)?;
    out.flush()
}

/// Extract `id` field of an inbound command line (may be absent).
fn cmd_id(v: &Value) -> Option<String> {
    v.get("id").and_then(|x| x.as_str()).map(str::to_owned)
}

fn cmd_type(v: &Value) -> Option<&str> {
    v.get("type").and_then(Value::as_str)
}

/// Success response for a given command name/id, optional data.
fn ok_response(id: Option<&str>, command: &str, data: Value) -> Value {
    let mut o = serde_json::Map::new();
    if let Some(id) = id {
        o.insert("id".into(), Value::String(id.into()));
    }
    o.insert("type".into(), Value::String("response".into()));
    o.insert("command".into(), Value::String(command.into()));
    o.insert("success".into(), Value::Bool(true));
    if !data.is_null() {
        o.insert("data".into(), data);
    }
    Value::Object(o)
}

fn err_response(id: Option<&str>, command: &str, msg: &str) -> Value {
    let mut o = serde_json::Map::new();
    if let Some(id) = id {
        o.insert("id".into(), Value::String(id.into()));
    }
    o.insert("type".into(), Value::String("response".into()));
    o.insert("command".into(), Value::String(command.into()));
    o.insert("success".into(), Value::Bool(false));
    o.insert("error".into(), Value::String(msg.into()));
    Value::Object(o)
}

/// Split a large logical JSON object into canonical v2 chunk frames.
///
/// Produces `count >= 2` chunks, each with a 256 KiB decoded payload except
/// the last which may be smaller, canonical base64, stable `chunkId`.
fn chunkify(chunk_id: &str, logical: &Value) -> Vec<Value> {
    let body = serde_json::to_vec(logical).expect("logical serializes");
    let byte_length = body.len();
    assert!(
        byte_length >= usize::try_from(MAX_FRAME_BYTES).expect("1 MiB fits in usize"),
        "chunkify requires >=1 MiB logical payload; got {byte_length}"
    );
    let count = byte_length.div_ceil(CHUNK_PAYLOAD_BYTES);
    assert!(count >= 2, "chunk count must be >=2");
    let mut out = Vec::with_capacity(count);
    for (index, slice) in body.chunks(CHUNK_PAYLOAD_BYTES).enumerate() {
        out.push(json!({
            "type": "rpc_chunk",
            "chunkId": chunk_id,
            "index": index,
            "count": count,
            "byteLength": byte_length,
            "data": BASE64.encode(slice),
        }));
    }
    out
}

/// Read one line from stdin. Returns Ok(None) on EOF.
///
/// The caller-controlled scenarios only ever send small commands so we cap
/// individual lines at `MAX_FRAME_BYTES` to mirror the real transport.
fn read_line(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut buf = String::new();
    let n = reader.take(MAX_FRAME_BYTES + 1).read_line(&mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(buf))
}

/// Wait for a JSON command from stdin. Returns None on EOF. Non-JSON lines are
/// treated as fatal (the real client would never send them).
fn recv(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let Some(line) = read_line(reader)? else {
        return Ok(None);
    };
    let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
    if trimmed.is_empty() {
        // Skip empty keepalive-style blanks.
        return recv(reader);
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(v) => Ok(Some(v)),
        Err(e) => {
            diag(&format!("bad stdin line: {e}"));
            Err(io::Error::other(format!("bad stdin: {e}")))
        }
    }
}

/// Read a command whose `type` is `negotiate_transport_version` and reply
/// success. This is a common prefix to almost every scenario after `ready`.
fn expect_negotiate(reader: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    let Some(cmd) = recv(reader)? else {
        diag("EOF before negotiate");
        return Err(io::Error::other("EOF before negotiate"));
    };
    let ty = cmd_type(&cmd).unwrap_or("").to_owned();
    if ty != "negotiate_protocol" {
        diag(&format!("expected negotiate, got {ty}"));
    }
    let id = cmd_id(&cmd);
    // v2 in ack matches what the client advertised.
    write_frame(
        out,
        &ok_response(
            id.as_deref(),
            "negotiate_protocol",
            json!({ "protocolVersion": 2 }),
        ),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

fn scenario_handshake_basic(reader: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    write_frame(out, &ready())?;
    expect_negotiate(reader, out)?;
    // Then answer any get_state / setup / no-op commands until EOF.
    while let Some(cmd) = recv(reader)? {
        let id = cmd_id(&cmd);
        let ty = cmd_type(&cmd).unwrap_or("unknown").to_owned();
        let data = match ty.as_str() {
            "get_state" => json!({ "sessionId": "fake-session", "messages": [] }),
            _ => Value::Null,
        };
        write_frame(out, &ok_response(id.as_deref(), &ty, data))?;
    }
    Ok(())
}

fn scenario_chunked_large(reader: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    write_frame(out, &ready())?;
    expect_negotiate(reader, out)?;
    // Wait for any command; reply with a chunked >1 MiB `response`.
    let Some(cmd) = recv(reader)? else {
        diag("EOF before chunk-trigger command");
        return Ok(());
    };
    let id = cmd_id(&cmd);
    let ty = cmd_type(&cmd).unwrap_or("get_state").to_owned();
    // Build ~1.25 MiB payload so count>=5 and last chunk < 256KiB — proves
    // both the multi-chunk path and the tail-boundary case.
    let payload_bytes = 5 * CHUNK_PAYLOAD_BYTES + 1234;
    let big_string: String = "x".repeat(payload_bytes);
    let logical = ok_response(id.as_deref(), &ty, json!({ "blob": big_string }));
    for frame in chunkify("chunk-1", &logical) {
        write_frame(out, &frame)?;
    }
    // Keep going for further commands.
    while let Some(cmd) = recv(reader)? {
        let id = cmd_id(&cmd);
        let ty = cmd_type(&cmd).unwrap_or("unknown").to_owned();
        write_frame(out, &ok_response(id.as_deref(), &ty, Value::Null))?;
    }
    Ok(())
}

fn scenario_concurrent_reorder(reader: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    write_frame(out, &ready())?;
    expect_negotiate(reader, out)?;
    // Buffer two commands, respond in REVERSE receive order.
    let a = recv(reader)?.ok_or_else(|| io::Error::other("EOF awaiting cmd A"))?;
    let b = recv(reader)?.ok_or_else(|| io::Error::other("EOF awaiting cmd B"))?;
    let a_id = cmd_id(&a);
    let a_ty = cmd_type(&a).unwrap_or("").to_owned();
    let b_id = cmd_id(&b);
    let b_ty = cmd_type(&b).unwrap_or("").to_owned();
    write_frame(
        out,
        &ok_response(b_id.as_deref(), &b_ty, json!({ "which": "B" })),
    )?;
    write_frame(
        out,
        &ok_response(a_id.as_deref(), &a_ty, json!({ "which": "A" })),
    )?;
    while let Some(cmd) = recv(reader)? {
        let id = cmd_id(&cmd);
        let ty = cmd_type(&cmd).unwrap_or("unknown").to_owned();
        write_frame(out, &ok_response(id.as_deref(), &ty, Value::Null))?;
    }
    Ok(())
}

fn scenario_late_prompt_error(reader: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    write_frame(out, &ready())?;
    expect_negotiate(reader, out)?;
    // Wait for a prompt command; ACK success (so the client's pending request
    // resolves), then emit a second response with the SAME id that reports a
    // failure. The client cannot correlate this to a live future and must
    // surface it as a visible event.
    let Some(cmd) = recv(reader)? else {
        return Ok(());
    };
    let id = cmd_id(&cmd);
    let ty = cmd_type(&cmd).unwrap_or("prompt").to_owned();
    write_frame(
        out,
        &ok_response(id.as_deref(), &ty, json!({ "accepted": true })),
    )?;
    write_frame(
        out,
        &err_response(id.as_deref(), &ty, "prompt aborted after ack"),
    )?;
    while let Some(cmd) = recv(reader)? {
        let id = cmd_id(&cmd);
        let ty = cmd_type(&cmd).unwrap_or("unknown").to_owned();
        write_frame(out, &ok_response(id.as_deref(), &ty, Value::Null))?;
    }
    Ok(())
}

fn scenario_idless_response(reader: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    write_frame(out, &ready())?;
    expect_negotiate(reader, out)?;
    // Emit both an id-less parse-failure and an id-less unknown-command response.
    // Neither can be correlated to a caller future and must become an event.
    write_frame(
        out,
        &err_response(None, "parse", "invalid JSON on client stdin"),
    )?;
    write_frame(out, &err_response(None, "unknown", "unrecognized command"))?;
    while let Some(cmd) = recv(reader)? {
        let id = cmd_id(&cmd);
        let ty = cmd_type(&cmd).unwrap_or("unknown").to_owned();
        write_frame(out, &ok_response(id.as_deref(), &ty, Value::Null))?;
    }
    Ok(())
}

fn scenario_malformed_line(_reader: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    write_frame(out, &ready())?;
    // Immediately emit a malformed physical line. Do NOT wait for negotiate
    // because we don't care about the client's response; we want the decoder
    // to hit fatal ProtocolViolation as soon as it sees it.
    out.write_all(b"not-json\n")?;
    out.flush()?;
    // Keep stdout open so the client observes the error rather than EOF.
    // Read stdin to a natural EOF triggered by the client dropping us.
    let mut sink = Vec::new();
    let _ = io::stdin().read_to_end(&mut sink);
    Ok(())
}

fn scenario_interleave_chunk(reader: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    write_frame(out, &ready())?;
    expect_negotiate(reader, out)?;
    // Start a valid chunk sequence, then interrupt with a non-chunk frame.
    let payload = ok_response(Some("interleave"), "get_state", json!({ "x": "y" }));
    let all = chunkify(
        "interleave-1",
        &ok_response(
            Some("interleave"),
            "get_state",
            json!({ "blob": "x".repeat(usize::try_from(MAX_FRAME_BYTES).expect("1 MiB fits in usize") + 32) }),
        ),
    );
    // Write the first chunk, then a non-chunk frame — protocol violation.
    write_frame(out, &all[0])?;
    write_frame(out, &payload)?;
    // Read stdin to natural EOF.
    let mut sink = Vec::new();
    let _ = reader.read_to_end(&mut sink);
    Ok(())
}

fn scenario_mid_stream_eof(reader: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    write_frame(out, &ready())?;
    expect_negotiate(reader, out)?;
    // Emit a partial physical frame (no terminating newline), then EOF stdout.
    out.write_all(b"{\"type\":\"response\",\"id\":\"orphan\",\"command\":\"get_state\",\"success\":true,\"data\":{\"partial\":true")?;
    out.flush()?;
    // Dropping stdout via process exit closes it. Return so main() exits 0.
    Ok(())
}

fn scenario_stderr_then_exit() -> ExitCode {
    eprintln!("{DIAG_TAG} boom: simulated startup failure");
    eprintln!("{DIAG_TAG} additional diagnostic context on line two");
    ExitCode::from(42)
}

fn scenario_resume_capture(reader: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    // Emit argv on stderr under a stable marker so tests can assert on the
    // presence and ordering of `--mode rpc-ui`, `--no-session`, `--resume`.
    for arg in env::args() {
        eprintln!("{ARGV_MARKER} {arg}");
    }
    scenario_handshake_basic(reader, out)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn dispatch() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    // Discovery / version probe short-circuit.
    if args.iter().any(|a| a == "--version") {
        println!("omp 17.2.10");
        return ExitCode::SUCCESS;
    }

    let scenario = env::var("PIMIENTO_FAKE_SCENARIO").unwrap_or_else(|_| "handshake_basic".into());

    if scenario == "stderr_then_exit" {
        return scenario_stderr_then_exit();
    }

    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let result = match scenario.as_str() {
        "handshake_basic" => scenario_handshake_basic(&mut reader, &mut out),
        "chunked_large" => scenario_chunked_large(&mut reader, &mut out),
        "concurrent_reorder" => scenario_concurrent_reorder(&mut reader, &mut out),
        "late_prompt_error" => scenario_late_prompt_error(&mut reader, &mut out),
        "idless_response" => scenario_idless_response(&mut reader, &mut out),
        "malformed_line" => scenario_malformed_line(&mut reader, &mut out),
        "interleave_chunk" => scenario_interleave_chunk(&mut reader, &mut out),
        "mid_stream_eof" => scenario_mid_stream_eof(&mut reader, &mut out),
        "resume_capture" => scenario_resume_capture(&mut reader, &mut out),
        other => {
            diag(&format!("unknown scenario: {other}"));
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
            diag("stdout closed by peer");
            ExitCode::SUCCESS
        }
        Err(e) => {
            diag(&format!("scenario failed: {e}"));
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    dispatch()
}
