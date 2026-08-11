//! Integration tests that drive the real `RpcClient` against the `fake_omp`
//! binary shipped in `src/bin/fake_omp.rs`.
//!
//! Every scenario is deterministic and bounded — no clocks, no RNG, no
//! dependency on a real installed OMP. Cargo compiles `fake_omp` for us and
//! hands us its absolute path via `CARGO_BIN_EXE_fake_omp`; we pass that path
//! into the `RpcClient` via `ClientConfig::program`.
//!
//! What each `#[test]` proves (see `PLAN` §§4.2, 4.7, 4.8):
//!
//! * `handshake_v2_and_get_state`     — spawn → ready → negotiate v2 → command
//! * `concurrent_requests_reorder`    — reversed response order still correlates
//! * `chunked_response_reassembles`   — >1 MiB logical payload arrives whole
//! * `late_same_id_error_visible`     — post-ACK same-id failure surfaces as event
//! * `idless_response_visible`        — id-less error responses reach events
//! * `malformed_line_fatal`           — bad stdout kills connect/pending
//! * `interleave_during_chunk_fatal`  — non-chunk mid-chunk kills pending + Closed
//! * `mid_stream_eof_fatal`           — partial trailing frame + EOF kills pending
//! * `stderr_and_nonzero_exit_propagate`
//!   — captures 64 KiB stderr tail + exit code
//! * `resume_argv_forwarded`          — `--resume <path>` reaches the child argv

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use omp_rpc_client::RpcError;
use omp_rpc_client::client::{ClientConfig, ClientEvent, RpcClient};
use omp_rpc_client::frames::{IncomingFrameKind, RpcCommandBody};
use serde_json::Value;
use smol::future;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn fake_omp_path() -> PathBuf {
    // Cargo populates this for `[[bin]]` targets in the same package as an
    // integration test. Failing here means the fake_omp binary was not built,
    // which is a workspace/manifest bug, not a test failure.
    PathBuf::from(env!("CARGO_BIN_EXE_fake_omp"))
}

fn base_config(scenario: &str) -> ClientConfig {
    let mut env = BTreeMap::new();
    env.insert(
        OsString::from("PIMIENTO_FAKE_SCENARIO"),
        OsString::from(scenario),
    );
    if let Ok(p) = std::env::var("PATH") {
        env.insert(OsString::from("PATH"), OsString::from(p));
    }
    ClientConfig {
        program: fake_omp_path(),
        env,
        cwd: None,
        extra_args: Vec::new(),
        no_session: true,
        resume: None,
        command_timeout: Duration::from_secs(5),
    }
}

/// Await a future with a hard 10-second wall-clock cap so a scenario bug can
/// never wedge the suite.
fn block<F: Future>(fut: F) -> F::Output {
    smol::block_on(async move {
        future::or(fut, async {
            smol::Timer::after(Duration::from_secs(10)).await;
            panic!("test future exceeded 10s");
        })
        .await
    })
}

/// Poll the event channel until predicate returns `Some(value)` or `deadline`.
async fn wait_for<F, T>(
    events: &smol::channel::Receiver<ClientEvent>,
    deadline: Duration,
    mut pred: F,
) -> Option<T>
where
    F: FnMut(&ClientEvent) -> Option<T>,
{
    future::or(
        async {
            loop {
                match events.recv().await {
                    Ok(ev) => {
                        if let Some(t) = pred(&ev) {
                            return Some(t);
                        }
                        if matches!(ev, ClientEvent::Closed(_)) {
                            return None;
                        }
                    }
                    Err(_) => return None,
                }
            }
        },
        async {
            smol::Timer::after(deadline).await;
            None
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// 1. Baseline handshake — v2 advertised, negotiate succeeded, get_state works.
// ---------------------------------------------------------------------------

#[test]
fn handshake_v2_and_get_state() {
    block(async {
        let client = RpcClient::connect(base_config("handshake_basic"))
            .await
            .expect("test fixture operation must succeed");
        assert!(
            client.ready().supported_protocol_versions.contains(&2),
            "ready must advertise v2"
        );
        let resp = client
            .send(RpcCommandBody::GetState)
            .await
            .expect("test fixture operation must succeed");
        assert!(resp.success, "get_state should succeed");
        assert_eq!(resp.command, "get_state");
        assert!(resp.data.is_some(), "get_state carries data");
    });
}

// ---------------------------------------------------------------------------
// 2. Concurrent requests, reversed response order → correlation by id, not order.
// ---------------------------------------------------------------------------

#[test]
fn concurrent_requests_reorder() {
    block(async {
        let client = RpcClient::connect(base_config("concurrent_reorder"))
            .await
            .expect("test fixture operation must succeed");
        let a = client.send(RpcCommandBody::GetState);
        let b = client.send(RpcCommandBody::GetAvailableCommands);
        let (ra, rb) = future::zip(a, b).await;
        let ra = ra.expect("test fixture operation must succeed");
        let rb = rb.expect("test fixture operation must succeed");
        assert_eq!(ra.command, "get_state");
        assert_eq!(rb.command, "get_available_commands");
        assert_eq!(
            ra.data
                .as_ref()
                .and_then(|v| v.get("which"))
                .and_then(Value::as_str),
            Some("A"),
            "caller A's future must resolve with response A regardless of arrival order"
        );
        assert_eq!(
            rb.data
                .as_ref()
                .and_then(|v| v.get("which"))
                .and_then(Value::as_str),
            Some("B")
        );
    });
}

// ---------------------------------------------------------------------------
// 3. Chunked >1 MiB response reassembly.
// ---------------------------------------------------------------------------

#[test]
fn chunked_response_reassembles() {
    block(async {
        let client = RpcClient::connect(base_config("chunked_large"))
            .await
            .expect("test fixture operation must succeed");
        let resp = client
            .send(RpcCommandBody::GetState)
            .await
            .expect("test fixture operation must succeed");
        assert!(resp.success);
        let blob = resp
            .data
            .as_ref()
            .and_then(|v| v.get("blob"))
            .and_then(Value::as_str)
            .expect("chunked payload carries `blob` string");
        // Fake emits 5*256KiB+1234 = 1_312_002 bytes of 'x' — proves the
        // decoder → reassembler → client path for a genuine >1 MiB logical
        // frame using canonical 256 KiB base64 chunks.
        assert_eq!(blob.len(), 5 * 256 * 1024 + 1234);
        assert!(blob.bytes().all(|b| b == b'x'));
    });
}

// ---------------------------------------------------------------------------
// 4. Same-id late error after ACK — visible as an event, not a resolved future.
// ---------------------------------------------------------------------------

#[test]
fn late_same_id_error_visible() {
    block(async {
        let client = RpcClient::connect(base_config("late_prompt_error"))
            .await
            .expect("test fixture operation must succeed");
        let events = client.events();
        let resp = client
            .send(RpcCommandBody::Prompt {
                message: "hi".into(),
                images: None,
                streaming_behavior: None,
            })
            .await
            .expect("test fixture operation must succeed");
        assert!(resp.success, "first response is the ACK");

        let found = wait_for(&events, Duration::from_secs(3), |ev| {
            if let ClientEvent::Frame(frame) = ev
                && let IncomingFrameKind::Response(r) = &frame.kind
                && !r.success
                && r.command == "prompt"
            {
                return Some(());
            }
            None
        })
        .await;
        assert!(
            found.is_some(),
            "late same-id error must appear as ClientEvent::Frame(Response{{success:false}})"
        );
    });
}

// ---------------------------------------------------------------------------
// 5. Id-less responses (parse-failure / unknown-command) surface as events.
// ---------------------------------------------------------------------------

#[test]
fn idless_response_visible() {
    block(async {
        let client = RpcClient::connect(base_config("idless_response"))
            .await
            .expect("test fixture operation must succeed");
        let events = client.events();
        // Fake emits both id-less error responses immediately after negotiate.
        let counter = async {
            let mut seen = 0u32;
            while seen < 2 {
                match events.recv().await {
                    Ok(ClientEvent::Frame(f)) => {
                        if let IncomingFrameKind::Response(r) = &f.kind
                            && r.id.is_none()
                            && !r.success
                        {
                            seen += 1;
                        }
                    }
                    Ok(ClientEvent::Closed(_)) | Err(_) => break,
                }
            }
            seen
        };
        let timeout = async {
            smol::Timer::after(Duration::from_secs(3)).await;
            0u32
        };
        let got = future::or(counter, timeout).await;
        assert_eq!(
            got, 2,
            "both id-less error responses must reach the event stream"
        );
    });
}

// ---------------------------------------------------------------------------
// 6. Malformed physical line is fatal — connect fails outright because the
//    fake emits garbage before waiting for negotiate.
// ---------------------------------------------------------------------------

#[test]
fn malformed_line_fatal() {
    block(async {
        let err = RpcClient::connect(base_config("malformed_line"))
            .await
            .expect_err("connect must fail on malformed stdout");
        match err {
            RpcError::ProtocolViolation { .. } | RpcError::Json(_) | RpcError::ChildDied { .. } => {
            }
            other => panic!("unexpected error: {other:?}"),
        }
    });
}

// ---------------------------------------------------------------------------
// 7. Non-chunk frame during a pending chunk sequence is fatal — pending
//    request fails, Closed follows.
// ---------------------------------------------------------------------------

#[test]
fn interleave_during_chunk_fatal() {
    block(async {
        let client = RpcClient::connect(base_config("interleave_chunk"))
            .await
            .expect("test fixture operation must succeed");
        let events = client.events();
        let send_err = client.send(RpcCommandBody::GetState).await.err();
        assert!(
            send_err.is_some(),
            "pending request must fail after protocol violation"
        );
        let closed = wait_for(&events, Duration::from_secs(3), |ev| {
            matches!(ev, ClientEvent::Closed(_)).then_some(())
        })
        .await;
        assert!(closed.is_some(), "Closed event must follow interleave");
    });
}

// ---------------------------------------------------------------------------
// 8. Mid-stream EOF — partial trailing frame followed by stdout close.
// ---------------------------------------------------------------------------

#[test]
fn mid_stream_eof_fatal() {
    block(async {
        let client = RpcClient::connect(base_config("mid_stream_eof"))
            .await
            .expect("test fixture operation must succeed");
        let events = client.events();
        let send_err = client.send(RpcCommandBody::GetState).await.err();
        assert!(
            send_err.is_some(),
            "pending request must fail on mid-stream EOF"
        );
        let closed = wait_for(&events, Duration::from_secs(3), |ev| {
            matches!(ev, ClientEvent::Closed(_)).then_some(())
        })
        .await;
        assert!(closed.is_some());
    });
}

// ---------------------------------------------------------------------------
// 9. stderr payload + nonzero exit propagate through the connect error.
// ---------------------------------------------------------------------------

#[test]
fn stderr_and_nonzero_exit_propagate() {
    block(async {
        let err = RpcClient::connect(base_config("stderr_then_exit"))
            .await
            .expect_err("connect must fail when child exits before ready");
        match err {
            RpcError::ChildDied {
                exit_code,
                stderr_tail,
            } => {
                assert_eq!(exit_code, Some(42), "fake exits 42");
                assert!(
                    stderr_tail.contains("boom: simulated startup failure"),
                    "stderr tail must include the fake's diagnostic; got: {stderr_tail:?}"
                );
            }
            other => panic!("expected ChildDied, got {other:?}"),
        }
    });
}

// ---------------------------------------------------------------------------
// 10. Resume pointer reaches the child argv verbatim.
// ---------------------------------------------------------------------------

#[test]
fn resume_argv_forwarded() {
    block(async {
        let mut cfg = base_config("resume_capture");
        cfg.resume = Some(PathBuf::from("/tmp/pimiento-session-fake.json"));
        // --resume is incompatible with --no-session per PLAN §4.7.
        cfg.no_session = false;
        let client = RpcClient::connect(cfg)
            .await
            .expect("test fixture operation must succeed");
        // Drive one command so the fake finishes its startup and stderr has
        // flushed all of its ARGV lines.
        let _ = client
            .send(RpcCommandBody::GetState)
            .await
            .expect("test fixture operation must succeed");
        client.close_stdin().await;
        let events = client.events();
        let tail = wait_for(&events, Duration::from_secs(3), |ev| match ev {
            ClientEvent::Closed(info) => Some(info.stderr_tail.clone()),
            ClientEvent::Frame(_) => None,
        })
        .await
        .unwrap_or_default();

        assert!(
            tail.contains("--mode") && tail.contains("rpc-ui"),
            "child must be invoked with --mode rpc-ui; stderr: {tail:?}"
        );
        assert!(
            tail.contains("--resume") && tail.contains("pimiento-session-fake.json"),
            "child must receive --resume <path>; stderr: {tail:?}"
        );
        assert!(
            !tail.contains("--no-session"),
            "--no-session must be omitted when resume is set; stderr: {tail:?}"
        );
    });
}
