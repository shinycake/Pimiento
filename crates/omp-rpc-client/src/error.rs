//! Public error type for the OMP RPC client.
//!
//! Library errors are `thiserror`-based (workspace policy);
//! app-level callers may wrap these with `anyhow::Context` at the edges.

use std::io;

use thiserror::Error;

/// Errors surfaced by the RPC client, discovery, and framing layers.
#[derive(Debug, Error)]
pub enum RpcError {
    /// Underlying I/O error (child pipes, discovery FS checks, etc.).
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    /// Malformed JSON on the wire.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Wire contract broken by the peer (unexpected frame, invalid chunk sequence, etc.).
    #[error("protocol violation: {detail}")]
    ProtocolViolation { detail: String },

    /// A single physical or reassembled frame exceeded its declared byte limit.
    #[error("frame too large: {size} bytes exceeds limit of {limit}")]
    FrameTooLarge { size: usize, limit: usize },

    /// `ready.supportedProtocolVersions` did not include a version we can speak.
    #[error("unsupported OMP protocol version; peer advertised {supported:?}")]
    UnsupportedProtocol { supported: Vec<u32> },

    /// An RPC command completed with an error `response`.
    #[error("rpc command `{command}` failed (code {code:?}): {message}")]
    CommandFailed {
        command: String,
        code: Option<i32>,
        message: String,
    },

    /// An in-flight request exceeded the caller-configured deadline.
    #[error("rpc request `{id}` timed out")]
    Timeout { id: String },

    /// The `omp` child exited unexpectedly.
    #[error("omp child died (exit code {exit_code:?}); stderr tail: {stderr_tail}")]
    ChildDied {
        exit_code: Option<i32>,
        stderr_tail: String,
    },

    /// Discovery failed — no usable `omp` binary or version could not be probed.
    #[error("omp discovery failed: {detail}")]
    Discovery { detail: String },
}
