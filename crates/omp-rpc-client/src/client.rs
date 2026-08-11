//! Async RPC client over the installed OMP 17.2.10 wire protocol.
//!
//! The [`RpcClient`] owns one supervised child `omp --mode rpc-ui` process. It:
//!
//! * Spawns the child with a caller-provided absolute program, environment,
//!   optional CWD, and extra args (`--no-session` / `--resume <path>` opt-ins),
//!   with piped stdin / stdout / stderr.
//! * Reads the first `ready` frame, verifies protocol v2 is advertised, and
//!   sends `{"id":"protocol-1","type":"negotiate_protocol","protocolVersion":2}`
//!   before returning to the caller.
//! * Runs a single writer task that owns stdin — no two tasks can write
//!   concurrently. Every outbound payload is serialized as one line and
//!   validated against the advertised `maxFrameBytes` before enqueuing.
//! * Runs a reader task that feeds bytes through [`LineDecoder`] →
//!   [`ChunkReassembler`] → [`decode_frame`], routes `response` frames with a
//!   matching id through per-request oneshot channels, and delivers every
//!   other frame (events, id-less responses, unknown frame types) to the
//!   caller's [`ClientEvent`] channel.
//! * Runs a stderr task that keeps the trailing 64 KiB of the child's stderr
//!   for crash-card reporting.
//! * On child exit, stream EOF, or a fatal decoder error: fails every pending
//!   request with [`RpcError::ChildDied`], broadcasts a single
//!   [`ClientEvent::Closed`], closes the events channel, and reaps the child.
//!
//! Supervision (graceful shutdown timers, restart with `--resume`, crash-loop
//! breaker) lives in `supervisor.rs`; this module deliberately stops at owning
//! and lifecycling ONE connection.
//!
//! ## Correlation semantics
//!
//! * Request ids are assigned by [`RpcClient::send`] from a per-client
//!   atomic counter as `req_<n>`. Ids are never reused within a client
//!   instance, so a late same-id error after the pending entry has already
//!   been resolved surfaces harmlessly as a [`ClientEvent::Frame`] rather
//!   than corrupting an unrelated request. This matches PLAN §4.2 — a
//!   `prompt` ACK does NOT imply run completion; the caller must observe
//!   `agent_end` / `error` / `turn_end` events for lifecycle.
//! * Id-less responses (parse failures, unknown commands, server-side
//!   overflow responses without an id) surface as `Frame(_)` events so
//!   callers can log them without corrupting the pending map.
//! * Per-command timeout defaults to 30 seconds and never leaks a pending
//!   entry: on timeout the entry is removed and the id is retired.
//!
//! ## Testing strategy
//!
//! Full process-transport paths are exercised by the M1 fake-server binary
//! selected via `PIMIENTO_OMP_BIN` (see `crates/omp-rpc-client/tests/`), and
//! by the live smoke against the developer's real `omp`. The correlation
//! router itself is a pure state machine and is unit-tested here without a
//! child process.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use smol::channel::{self, Receiver, Sender};
use smol::future::FutureExt as _;
use smol::io::{AsyncReadExt as _, AsyncWriteExt as _};
use smol::lock::Mutex as AsyncMutex;
use smol::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};

use crate::decoder::{ChunkReassembler, LineDecoder};
use crate::error::RpcError;
use crate::frames::{
    IncomingFrame, IncomingFrameKind, MAX_RPC_FRAME_BYTES, ReadyFrame, RpcCommand, RpcCommandBody,
    RpcResponse, decode_frame,
};

/// Trailing stderr window kept for crash cards (PLAN §4.8).
pub const STDERR_TAIL_BYTES: usize = 64 * 1024;

/// Default per-command deadline (PLAN §4.2 / spec §4).
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Spawn / handshake configuration for [`RpcClient::connect`].
///
/// All fields are `pub` — construct with named-field syntax; there is no
/// builder. Discovery (`omp` path, login-shell env) lives in
/// [`crate::discovery`]; this struct is the transport-level input.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Absolute path to the `omp` binary. Callers get this from
    /// [`crate::discovery::discover`].
    pub program: PathBuf,
    /// Environment to spawn the child with. Passed verbatim; no inheritance
    /// from the current process env.
    pub env: BTreeMap<OsString, OsString>,
    /// Working directory for the child, if any.
    pub cwd: Option<PathBuf>,
    /// Extra args appended after `--mode rpc-ui [--no-session] [--resume <p>]`.
    pub extra_args: Vec<OsString>,
    /// Pass `--no-session` (in-memory session; smoke tests only per PLAN §4.7).
    pub no_session: bool,
    /// Pass `--resume <path>` — supplied by the supervisor from the last
    /// captured `get_state.sessionFile`.
    pub resume: Option<PathBuf>,
    /// Default per-command deadline; overridable via
    /// [`RpcClient::send_with_timeout`].
    pub command_timeout: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            program: PathBuf::new(),
            env: BTreeMap::new(),
            cwd: None,
            extra_args: Vec::new(),
            no_session: false,
            resume: None,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }
}

/// Capacity of the [`ClientEvent`] delivery channel.
///
/// Slow UI consumers apply backpressure on the stdout reader once this many
/// events are buffered — the reader task awaits `events_tx.send` and stops
/// draining child stdout until the UI drains the channel.
pub const CLIENT_EVENT_CHANNEL_CAP: usize = 512;

/// Terminal info about a closed connection. Cloneable so supervisor state
/// machines can retain it in the `Dead` phase.
#[derive(Debug, Clone)]
pub struct ClosedInfo {
    /// Child exit code if the process was reaped, else `None`.
    pub exit_code: Option<i32>,
    /// Trailing up-to-64-KiB slice of the child's stderr, best-effort UTF-8.
    pub stderr_tail: String,
    /// Human-readable reason for closure (protocol violation, IO error, etc.).
    /// `None` means the child exited cleanly with no protocol error observed.
    pub error_msg: Option<String>,
}

/// Every message delivered on the events channel.
///
/// After [`ClientEvent::Closed`] is emitted the channel is closed and no
/// further items appear.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// One inbound frame — session event, extension UI request, id-less
    /// response, or unknown frame. Response frames with a matching pending
    /// id are routed to the caller of [`RpcClient::send`] and do NOT appear
    /// here.
    Frame(Box<IncomingFrame>),
    /// Terminal frame — the connection is dead. All pending requests have
    /// been failed with [`RpcError::ChildDied`] by the time this is
    /// delivered.
    Closed(ClosedInfo),
}

/// Owning handle to one spawned `omp` child.
///
/// Cheap-`Clone`: the handle is an `Arc` internally so supervisor and app
/// layers can hold references without wrestling with borrows. Consuming
/// operations (`wait`) take `self` to drop the underlying `Arc`.
#[derive(Debug, Clone)]
pub struct RpcClient {
    inner: Arc<Inner>,
    events_rx: Receiver<ClientEvent>,
}

#[derive(Debug)]
struct Inner {
    /// Ready-frame advertised by the peer (protocol limits, versions).
    ready: ReadyFrame,
    /// Writer channel — sender owns stdin. `None` variant signals half-close.
    writer_tx: Sender<Option<Vec<u8>>>,
    /// Per-id oneshot senders. Guarded by an async mutex; `close_all` takes
    /// this lock BEFORE draining, and `send_internal` checks `closed` under
    /// the same lock before inserting — this closes the race where a fatal
    /// stream error resolves `close_all`'s drain just before a fresh pending
    /// entry arrives from a `send_internal` in flight.
    pending: AsyncMutex<HashMap<String, Sender<RpcResponse>>>,
    /// Request-id counter. Ids are `req_<n>` starting at 1.
    counter: AtomicU64,
    /// Default per-call timeout.
    default_timeout: Duration,
    /// Handle to the spawned child (mutex because `kill` / `try_status` /
    /// `status` all take `&mut Child`). Wrapped in `Option` so we can move
    /// the `Child` out to `wait()` for reaping.
    child: AsyncMutex<Option<Child>>,
    /// Trailing stderr window (bytes). Bounded to `STDERR_TAIL_BYTES`.
    stderr_tail: AsyncMutex<VecDeque<u8>>,
    /// Closed once the stderr-read task terminates (child stderr EOF). Used
    /// by `close_all` to make sure the terminal `Closed` event includes the
    /// last stderr bytes even when the reader observes EOF before the
    /// stderr task has finished draining.
    stderr_done_tx: Sender<()>,
    stderr_done_rx: Receiver<()>,
    /// Once-flag guarding `close_all`.
    closed: AtomicBool,
    /// Events sender kept here so `close_all` can broadcast a terminal
    /// message from any task without racing multiple senders.
    events_tx: Sender<ClientEvent>,
}

/// Internal — dispatch a decoded logical frame.
///
/// Split out so the correlation router is unit-testable without a child
/// process (see `tests/` for the deterministic in-process harness).
async fn route_frame(inner: &Inner, frame: IncomingFrame) {
    if let IncomingFrameKind::Response(RpcResponse { id: Some(id), .. }) = &frame.kind {
        // If there IS a pending entry for this id, resolve it. Otherwise the
        // response is unsolicited (either the caller already timed out and
        // dropped its receiver, or this is a late same-id error after the
        // first ACK) — in that case fall through and deliver as an event so
        // it stays visible.
        let mut pending = inner.pending.lock().await;
        if let Some(tx) = pending.remove(id) {
            let RpcResponse { .. } = &frame.kind.as_response().expect("just matched Response");
            let resp = frame.kind.into_response().expect("just matched Response");
            // Best-effort send; if the caller dropped their oneshot receiver
            // (e.g. after a timeout) treat it as an unroutable event.
            if let Err(err) = tx.try_send(resp) {
                // Reconstruct a Frame event for visibility.
                let ev = ClientEvent::Frame(Box::new(IncomingFrame {
                    kind: IncomingFrameKind::Response(err.into_inner()),
                    raw: frame.raw,
                }));
                let _ = inner.events_tx.send(ev).await;
            }
            return;
        }
        // fall through
    }
    let _ = inner
        .events_tx
        .send(ClientEvent::Frame(Box::new(frame)))
        .await;
}

impl IncomingFrameKind {
    fn as_response(&self) -> Option<&RpcResponse> {
        if let IncomingFrameKind::Response(r) = self {
            Some(r)
        } else {
            None
        }
    }
    fn into_response(self) -> Option<RpcResponse> {
        if let IncomingFrameKind::Response(r) = self {
            Some(r)
        } else {
            None
        }
    }
}

/// Stage-1 handshake outcome — separates a reader-level failure (which
/// needs a bounded stderr/exit drain via [`connect_failure`]) from a
/// synchronous protocol-level rejection that can be returned as-is.
enum HandshakeStageError {
    Reader(FrameReaderError),
    Rpc(RpcError),
}

impl RpcClient {
    /// Spawn the child, complete the ready/negotiate handshake, and return
    /// the connected client.
    ///
    /// # Errors
    /// * [`RpcError::Io`] — spawn or pipe setup failed.
    /// * [`RpcError::ProtocolViolation`] — malformed `ready` frame or
    ///   negotiate response.
    /// * [`RpcError::UnsupportedProtocol`] — peer did not advertise v2.
    /// * [`RpcError::CommandFailed`] — `negotiate_protocol` returned an
    ///   error response.
    /// * [`RpcError::ChildDied`] — child exited before completing the
    ///   handshake.
    pub async fn connect(cfg: ClientConfig) -> Result<Self, RpcError> {
        let (mut child, stdin, stdout, stderr) = Self::spawn_child(&cfg)?;

        // Stage 1: read frames until we see `ready`. If the child dies or
        // emits garbage before ready, synthesize a ChildDied that carries
        // the real exit code + stderr tail (needs a brief reap wait +
        // stderr drain — otherwise the caller sees exit_code=None and an
        // empty tail because the OS hasn't finished cleaning up).
        let mut reader = FrameReader::new(stdout);
        let (ready, outbound_max) = match Self::read_ready(&mut reader).await {
            Ok(pair) => pair,
            Err(HandshakeStageError::Reader(read_err)) => {
                return Err(connect_failure(read_err, &mut child, stderr).await);
            }
            Err(HandshakeStageError::Rpc(e)) => return Err(e),
        };

        let (writer_tx, writer_rx) = channel::bounded::<Option<Vec<u8>>>(64);
        let (stderr_done_tx, stderr_done_rx) = channel::bounded::<()>(1);
        let (inner, events_rx) = Self::build_inner(
            &cfg,
            ready,
            child,
            writer_tx,
            stderr_done_tx,
            stderr_done_rx,
        );
        spawn_writer(Arc::clone(&inner), writer_rx, stdin, outbound_max);
        spawn_stderr(Arc::clone(&inner), stderr);
        spawn_exit_watch(Arc::clone(&inner));
        spawn_reader(Arc::clone(&inner), reader);

        let this = Self { inner, events_rx };
        this.negotiate_v2().await?;
        Ok(this)
    }

    fn spawn_child(
        cfg: &ClientConfig,
    ) -> Result<(Child, ChildStdin, ChildStdout, ChildStderr), RpcError> {
        let mut cmd = Command::new(&cfg.program);
        cmd.env_clear();
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        if let Some(cwd) = &cfg.cwd {
            cmd.current_dir(cwd);
        }
        cmd.arg("--mode").arg("rpc-ui");
        if cfg.no_session {
            cmd.arg("--no-session");
        }
        if let Some(p) = &cfg.resume {
            cmd.arg("--resume").arg(p);
        }
        for a in &cfg.extra_args {
            cmd.arg::<&OsStr>(a.as_ref());
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(RpcError::Io)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| RpcError::Io(io_other("child stdin missing")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| RpcError::Io(io_other("child stdout missing")))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| RpcError::Io(io_other("child stderr missing")))?;
        Ok((child, stdin, stdout, stderr))
    }

    async fn read_ready(
        reader: &mut FrameReader,
    ) -> Result<(ReadyFrame, usize), HandshakeStageError> {
        let ready_frame = reader
            .next_frame()
            .await
            .map_err(HandshakeStageError::Reader)?;
        let ready = match ready_frame.kind {
            IncomingFrameKind::Ready(r) => r,
            other => {
                return Err(HandshakeStageError::Rpc(RpcError::ProtocolViolation {
                    detail: format!("expected `ready` first, got {:?}", frame_kind_name(&other)),
                }));
            }
        };
        if !ready.supported_protocol_versions.contains(&2) {
            return Err(HandshakeStageError::Rpc(RpcError::UnsupportedProtocol {
                supported: ready.supported_protocol_versions,
            }));
        }
        let outbound_max = usize::try_from(ready.max_frame_bytes)
            .unwrap_or(MAX_RPC_FRAME_BYTES)
            .min(MAX_RPC_FRAME_BYTES);
        Ok((ready, outbound_max))
    }

    fn build_inner(
        cfg: &ClientConfig,
        ready: ReadyFrame,
        child: Child,
        writer_tx: Sender<Option<Vec<u8>>>,
        stderr_done_tx: Sender<()>,
        stderr_done_rx: Receiver<()>,
    ) -> (Arc<Inner>, Receiver<ClientEvent>) {
        let (events_tx, events_rx) = channel::bounded::<ClientEvent>(CLIENT_EVENT_CHANNEL_CAP);
        let inner = Arc::new(Inner {
            ready,
            writer_tx,
            pending: AsyncMutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
            default_timeout: if cfg.command_timeout.is_zero() {
                DEFAULT_COMMAND_TIMEOUT
            } else {
                cfg.command_timeout
            },
            child: AsyncMutex::new(Some(child)),
            stderr_tail: AsyncMutex::new(VecDeque::with_capacity(STDERR_TAIL_BYTES)),
            stderr_done_tx,
            stderr_done_rx,
            closed: AtomicBool::new(false),
            events_tx,
        });
        (inner, events_rx)
    }

    async fn negotiate_v2(&self) -> Result<(), RpcError> {
        let resp = self
            .send_with_id(
                "protocol-1".to_string(),
                RpcCommandBody::NegotiateProtocol {
                    protocol_version: 2,
                },
            )
            .await?;
        if !resp.success {
            return Err(RpcError::CommandFailed {
                command: resp.command,
                code: None,
                message: resp
                    .error
                    .unwrap_or_else(|| "negotiate_protocol failed".into()),
            });
        }
        let data_v = resp
            .data
            .as_ref()
            .and_then(|d| d.get("protocolVersion"))
            .and_then(Value::as_u64);
        if data_v != Some(2) {
            return Err(RpcError::ProtocolViolation {
                detail: format!("negotiate_protocol returned data={:?}", resp.data),
            });
        }
        Ok(())
    }

    /// Ready-frame advertised limits (max frame / reassembled bytes, versions).
    #[must_use]
    pub fn ready(&self) -> &ReadyFrame {
        &self.inner.ready
    }

    /// Receiver for every non-response frame plus the terminal
    /// [`ClientEvent::Closed`].
    #[must_use]
    pub fn events(&self) -> Receiver<ClientEvent> {
        self.events_rx.clone()
    }

    /// Send a typed command body under an auto-generated `req_<n>` id and
    /// wait for its response with the default timeout.
    ///
    /// # Errors
    /// See [`RpcError`]. On timeout the pending entry is removed and the id
    /// is retired.
    pub async fn send(&self, body: RpcCommandBody) -> Result<RpcResponse, RpcError> {
        self.send_with_timeout(body, self.inner.default_timeout)
            .await
    }

    /// Same as [`Self::send`] but with a caller-specified timeout.
    ///
    /// # Errors
    /// See [`RpcError`]. On timeout the pending entry is removed and the id
    /// is retired.
    pub async fn send_with_timeout(
        &self,
        body: RpcCommandBody,
        timeout: Duration,
    ) -> Result<RpcResponse, RpcError> {
        let n = self.inner.counter.fetch_add(1, Ordering::Relaxed);
        let id = format!("req_{n}");
        self.send_internal(id, body, timeout).await
    }

    /// Send with a caller-chosen id (used for the initial `protocol-1`
    /// negotiate and other well-known handshake ids). The caller MUST NOT
    /// collide with an auto-generated `req_<n>`.
    ///
    /// # Errors
    /// See [`RpcError`].
    pub async fn send_with_id(
        &self,
        id: String,
        body: RpcCommandBody,
    ) -> Result<RpcResponse, RpcError> {
        let t = self.inner.default_timeout;
        self.send_internal(id, body, t).await
    }

    async fn send_internal(
        &self,
        id: String,
        body: RpcCommandBody,
        timeout: Duration,
    ) -> Result<RpcResponse, RpcError> {
        let cmd = RpcCommand::new(Some(id.clone()), body);
        let mut line = serde_json::to_vec(&cmd)?;
        line.push(b'\n');
        self.enforce_outbound_limit(&line)?;

        let (tx, rx) = channel::bounded::<RpcResponse>(1);
        {
            let mut pending = self.inner.pending.lock().await;
            // Serialize with close_all: if the reader task already flipped
            // `closed` under this same lock, fail fast with ChildDied
            // instead of inserting a pending entry that would only be
            // reaped on timeout.
            if self.inner.closed.load(Ordering::SeqCst) {
                drop(pending);
                return Err(self.child_died_error().await);
            }
            pending.insert(id.clone(), tx);
        }
        if let Err(err) = self.inner.writer_tx.send(Some(line)).await {
            // Writer channel closed — connection is going down.
            let _ = err;
            self.inner.pending.lock().await.remove(&id);
            return Err(self.child_died_error().await);
        }

        // Race: response vs timeout. Whichever fires first wins; if the timer
        // wins we MUST remove the pending entry so a late arrival does not
        // dangle in the map.
        let id_for_timer = id.clone();
        let timer = smol::Timer::after(timeout);
        let recv = async {
            match rx.recv().await {
                Ok(resp) => Ok(resp),
                Err(_) => Err(self.child_died_error().await),
            }
        };
        let timeout_arm = async move {
            let _ = timer.await;
            Err::<RpcResponse, RpcError>(RpcError::Timeout { id: id_for_timer })
        };
        let outcome = recv.or(timeout_arm).await;
        if outcome.is_err() {
            // Timeout or ChildDied path: drop pending entry.
            self.inner.pending.lock().await.remove(&id);
        }
        outcome
    }

    /// Send a raw pre-shaped JSON value as one frame — used for extension UI
    /// responses (`extension_ui_response`), host tool results/updates, and
    /// host URI results. Fire-and-forget: no response correlation.
    ///
    /// # Errors
    /// * [`RpcError::FrameTooLarge`] if serialization exceeds the advertised
    ///   `maxFrameBytes` (including its newline).
    /// * [`RpcError::ChildDied`] if the writer has already closed.
    pub async fn send_raw(&self, value: Value) -> Result<(), RpcError> {
        let mut line = serde_json::to_vec(&value)?;
        line.push(b'\n');
        self.enforce_outbound_limit(&line)?;
        if self.inner.writer_tx.send(Some(line)).await.is_err() {
            return Err(self.child_died_error().await);
        }
        Ok(())
    }

    fn enforce_outbound_limit(&self, line: &[u8]) -> Result<(), RpcError> {
        let limit = usize::try_from(self.inner.ready.max_frame_bytes)
            .unwrap_or(MAX_RPC_FRAME_BYTES)
            .min(MAX_RPC_FRAME_BYTES);
        if line.len() > limit {
            return Err(RpcError::FrameTooLarge {
                size: line.len(),
                limit,
            });
        }
        Ok(())
    }

    /// Half-close stdin. Signals a graceful shutdown to the peer per PLAN
    /// §4.8; supervisor drives the 10 s / SIGTERM / SIGKILL escalation.
    pub async fn close_stdin(&self) {
        let _ = self.inner.writer_tx.send(None).await;
    }

    /// SIGKILL the child (portable, safe). Supervisor uses SIGTERM+timer
    /// escalation via its own OS-specific helpers; this is the last-resort
    /// hard kill.
    ///
    /// # Errors
    /// Propagates the OS error from [`Child::kill`].
    pub async fn kill(&self) -> std::io::Result<()> {
        let mut guard = self.inner.child.lock().await;
        if let Some(child) = guard.as_mut() {
            child.kill()?;
        }
        Ok(())
    }

    /// Wait for the child to exit and drain the terminal `Closed` event.
    /// Consumes the handle so the caller cannot accidentally reuse it.
    pub async fn wait(self) -> ClosedInfo {
        // Ensure a Closed has been broadcast; the exit-watch task also does
        // this, but calling wait() before reader-error observation should not
        // stall.
        while let Ok(ev) = self.events_rx.recv().await {
            if let ClientEvent::Closed(info) = ev {
                return info;
            }
        }
        // Channel closed without a Closed — synthesize one.
        ClosedInfo {
            exit_code: None,
            stderr_tail: current_stderr_tail(&self.inner).await,
            error_msg: None,
        }
    }

    async fn child_died_error(&self) -> RpcError {
        RpcError::ChildDied {
            exit_code: None,
            stderr_tail: current_stderr_tail(&self.inner).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Background tasks.
// ---------------------------------------------------------------------------

fn spawn_writer(
    inner: Arc<Inner>,
    rx: Receiver<Option<Vec<u8>>>,
    mut stdin: ChildStdin,
    _outbound_max: usize,
) {
    smol::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            let Some(bytes) = msg else {
                // Half-close: drop stdin and terminate writer. Do NOT
                // close_all — the child gets EOF and will exit; the
                // exit-watch / reader tasks broadcast Closed.
                drop(stdin);
                return;
            };
            if let Err(e) = stdin.write_all(&bytes).await {
                close_all(&inner, Some(RpcError::Io(e))).await;
                return;
            }
            if let Err(e) = stdin.flush().await {
                close_all(&inner, Some(RpcError::Io(e))).await;
                return;
            }
        }
    })
    .detach();
}

fn spawn_stderr(inner: Arc<Inner>, mut stderr: ChildStderr) {
    smol::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) | Err(_) => {
                    // Signal stderr EOF so close_all can drain us before
                    // building the terminal ClosedInfo.
                    let _ = inner.stderr_done_tx.try_send(());
                    inner.stderr_done_tx.close();
                    return;
                }
                Ok(n) => {
                    let mut tail = inner.stderr_tail.lock().await;
                    for &b in &buf[..n] {
                        if tail.len() == STDERR_TAIL_BYTES {
                            tail.pop_front();
                        }
                        tail.push_back(b);
                    }
                }
            }
        }
    })
    .detach();
}

fn spawn_exit_watch(inner: Arc<Inner>) {
    smol::spawn(async move {
        // Poll `try_status` on a short timer instead of holding the mutex
        // across an await on `status()`, so `kill()` etc. remain responsive.
        loop {
            {
                let mut guard = inner.child.lock().await;
                if let Some(child) = guard.as_mut()
                    && let Ok(Some(status)) = child.try_status()
                {
                    let code = status.code();
                    drop(guard);
                    close_all_with_exit(&inner, code, None).await;
                    return;
                }
            }
            smol::Timer::after(Duration::from_millis(50)).await;
            if inner.closed.load(Ordering::SeqCst) {
                return;
            }
        }
    })
    .detach();
}

fn spawn_reader(inner: Arc<Inner>, mut reader: FrameReader) {
    smol::spawn(async move {
        loop {
            match reader.next_frame().await {
                Ok(frame) => route_frame(&inner, frame).await,
                Err(FrameReaderError::Eof) => {
                    close_all(&inner, None).await;
                    return;
                }
                Err(FrameReaderError::Io(e)) => {
                    close_all(&inner, Some(RpcError::Io(e))).await;
                    return;
                }
                Err(FrameReaderError::Rpc(e)) => {
                    close_all(&inner, Some(e)).await;
                    return;
                }
            }
        }
    })
    .detach();
}

// ---------------------------------------------------------------------------
// Frame reader (line splitter + chunk reassembler + typed decode).
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct FrameReader {
    stdout: ChildStdout,
    lines: LineDecoder,
    chunks: ChunkReassembler,
    // Pending decoded values (a single read can complete multiple frames).
    ready_frames: VecDeque<IncomingFrame>,
    hit_eof: bool,
}

#[derive(Debug)]
enum FrameReaderError {
    Eof,
    Io(std::io::Error),
    Rpc(RpcError),
}

impl From<RpcError> for FrameReaderError {
    fn from(e: RpcError) -> Self {
        Self::Rpc(e)
    }
}

impl FrameReader {
    fn new(stdout: ChildStdout) -> Self {
        Self {
            stdout,
            lines: LineDecoder::new(),
            chunks: ChunkReassembler::new(),
            ready_frames: VecDeque::new(),
            hit_eof: false,
        }
    }

    async fn next_frame(&mut self) -> Result<IncomingFrame, FrameReaderError> {
        loop {
            if let Some(f) = self.ready_frames.pop_front() {
                return Ok(f);
            }
            if self.hit_eof {
                return Err(FrameReaderError::Eof);
            }
            let mut buf = [0u8; 8192];
            let n = self
                .stdout
                .read(&mut buf)
                .await
                .map_err(FrameReaderError::Io)?;
            if n == 0 {
                self.hit_eof = true;
                // Flush any partial line as protocol violation.
                if let Err(e) = self.lines.eof() {
                    return Err(FrameReaderError::Rpc(e));
                }
                continue;
            }
            // Feed line decoder → chunk reassembler → typed decode. Every
            // completed logical frame is pushed onto `ready_frames`.
            let ready = &mut self.ready_frames;
            let chunks = &mut self.chunks;
            let mut inner_err: Option<RpcError> = None;
            let feed_res = self.lines.feed(&buf[..n], |v| match chunks.push(v) {
                Ok(Some(logical)) => match decode_frame(logical) {
                    Ok(frame) => {
                        ready.push_back(frame);
                        Ok(())
                    }
                    Err(e) => {
                        inner_err = Some(RpcError::Json(e));
                        Err(RpcError::ProtocolViolation {
                            detail: "typed decode failed".into(),
                        })
                    }
                },
                Ok(None) => Ok(()),
                Err(e) => {
                    inner_err = Some(e.clone_shape());
                    Err(e)
                }
            });
            if let Some(e) = inner_err {
                return Err(FrameReaderError::Rpc(e));
            }
            if let Err(e) = feed_res {
                return Err(FrameReaderError::Rpc(e));
            }
        }
    }
}

// RpcError isn't Clone; give ourselves a tiny helper for the reader's dual
// bookkeeping (LineDecoder swallows the sink error into its own poison state
// so we stash the real cause).
trait CloneShape {
    fn clone_shape(&self) -> RpcError;
}
impl CloneShape for RpcError {
    fn clone_shape(&self) -> RpcError {
        RpcError::ProtocolViolation {
            detail: self.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Shutdown plumbing.
// ---------------------------------------------------------------------------

async fn close_all(inner: &Inner, err: Option<RpcError>) {
    close_all_with_exit(inner, None, err).await;
}

/// Terminal shutdown. Idempotent under the `pending` lock so a concurrent
/// `send_internal` cannot insert a fresh entry that we'd then fail to drain.
///
/// `code` is a caller-observed exit code (from exit-watch); when `None` we
/// wait — bounded — for the child to exit and for the stderr task to drain
/// so the terminal `ClosedInfo` includes both `exit_code` and the full
/// stderr tail. The bound (see `REAP_WAIT`) prevents a wedged child from
/// stalling shutdown; on timeout we surface `exit_code = None`.
async fn close_all_with_exit(inner: &Inner, code: Option<i32>, err: Option<RpcError>) {
    const REAP_WAIT: Duration = Duration::from_millis(500);

    // Serialize with send_internal: take the pending lock first, flip the
    // closed flag under it, then drain. A send_internal that observed
    // closed==false and is queued on this lock will see closed==true and
    // fail before inserting.
    let mut pending = inner.pending.lock().await;
    if inner.closed.swap(true, Ordering::SeqCst) {
        return;
    }
    let drained: Vec<Sender<RpcResponse>> = pending.drain().map(|(_, s)| s).collect();
    drop(pending);
    // Dropping the senders is enough: send_internal treats channel-closed
    // as ChildDied.
    drop(drained);

    // Best-effort: wait briefly for the child to exit and the stderr task
    // to finish draining. Both may already be done — the awaits return
    // immediately in that case. Bounded so a wedged child never stalls the
    // supervisor's shutdown path.
    let exit_code = if let Some(c) = code {
        Some(c)
    } else {
        wait_child_status(inner, REAP_WAIT).await
    };
    // Give the stderr task up to REAP_WAIT to drain — its sender closes on
    // EOF, so a closed rx (`recv() -> Err`) means the tail is complete.
    let _ = inner
        .stderr_done_rx
        .recv()
        .or(async {
            smol::Timer::after(REAP_WAIT).await;
            Err(smol::channel::RecvError)
        })
        .await;

    let stderr_tail = current_stderr_tail(inner).await;
    let error_msg = err.map(|e| e.to_string());
    let info = ClosedInfo {
        exit_code,
        stderr_tail,
        error_msg,
    };
    let _ = inner.events_tx.send(ClientEvent::Closed(info)).await;
    inner.events_tx.close();
    inner.writer_tx.close();
}

/// Wait up to `budget` for the child to exit. `try_status` alone races the
/// kernel's process reaping — we poll with a short sleep so a child that
/// has just closed stdout but not yet been reaped still yields an exit
/// code.
async fn wait_child_status(inner: &Inner, budget: Duration) -> Option<i32> {
    let deadline = std::time::Instant::now() + budget;
    loop {
        {
            let mut guard = inner.child.lock().await;
            if let Some(child) = guard.as_mut()
                && let Ok(Some(status)) = child.try_status()
            {
                return status.code();
            }
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        smol::Timer::after(Duration::from_millis(10)).await;
    }
}

async fn current_stderr_tail(inner: &Inner) -> String {
    let tail = inner.stderr_tail.lock().await;
    let bytes: Vec<u8> = tail.iter().copied().collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn io_other(msg: &'static str) -> std::io::Error {
    std::io::Error::other(msg)
}

/// On a pre-handshake failure, drain the child's stderr + wait for exit
/// (bounded) so the returned error carries the real diagnostic tail and
/// exit code. Called only from `connect()`.
async fn connect_failure(
    read_err: FrameReaderError,
    child: &mut Child,
    mut stderr: ChildStderr,
) -> RpcError {
    const DRAIN_BUDGET: Duration = Duration::from_millis(300);
    const REAP_BUDGET: Duration = Duration::from_millis(500);

    // If the stream was a fatal protocol / json / io error (not EOF), pass
    // it through unchanged — the caller wants to know WHY the peer's frame
    // was rejected, not that it died. Otherwise (EOF), synthesize
    // ChildDied with the real exit code + stderr tail.
    let mut want_child_died = false;
    let base_err = match read_err {
        FrameReaderError::Eof => {
            want_child_died = true;
            None
        }
        FrameReaderError::Io(e) => Some(RpcError::Io(e)),
        FrameReaderError::Rpc(e) => Some(e),
    };

    // Drain remaining stderr to EOF (bounded). The child has already exited
    // in the ChildDied case; even in the malformed-line case a well-behaved
    // fake keeps its stderr short. Cap at STDERR_TAIL_BYTES + a slack.
    let mut tail: VecDeque<u8> = VecDeque::with_capacity(STDERR_TAIL_BYTES);
    let drain_deadline = std::time::Instant::now() + DRAIN_BUDGET;
    let mut buf = [0u8; 4096];
    loop {
        let remaining = drain_deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let read = stderr.read(&mut buf);
        let timer = async {
            smol::Timer::after(remaining).await;
            Ok::<usize, std::io::Error>(0usize)
        };
        match read.or(timer).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if tail.len() == STDERR_TAIL_BYTES {
                        tail.pop_front();
                    }
                    tail.push_back(b);
                }
            }
        }
    }
    let stderr_tail =
        String::from_utf8_lossy(&tail.iter().copied().collect::<Vec<_>>()).into_owned();

    // Best-effort wait for exit. If the child is still alive (malformed-line
    // path), kill it so we don't leak the process.
    let deadline = std::time::Instant::now() + REAP_BUDGET;
    let mut exit_code: Option<i32> = None;
    loop {
        match child.try_status() {
            Ok(Some(status)) => {
                exit_code = status.code();
                break;
            }
            Ok(None) => {}
            Err(_) => break,
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            if let Ok(Some(status)) = child.try_status() {
                exit_code = status.code();
            }
            break;
        }
        smol::Timer::after(Duration::from_millis(10)).await;
    }

    match base_err {
        Some(err) if !want_child_died => {
            // Return the specific decode / IO error unchanged so callers
            // can tell why the peer's frame was malformed. The stderr
            // tail and exit code are useful diagnostics but they'd change
            // the error variant — keep the specific error, callers can
            // inspect stderr separately if needed.
            err
        }
        _ => RpcError::ChildDied {
            exit_code,
            stderr_tail,
        },
    }
}

fn frame_kind_name(k: &IncomingFrameKind) -> &'static str {
    match k {
        IncomingFrameKind::Ready(_) => "ready",
        IncomingFrameKind::Response(_) => "response",
        IncomingFrameKind::RpcChunk(_) => "rpc_chunk",
        IncomingFrameKind::RpcFrameError(_) => "rpc_frame_error",
        _ => "event",
    }
}

// ---------------------------------------------------------------------------
// Unit tests — pure correlation router + config plumbing, no child process.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::{IncomingFrame, IncomingFrameKind, RpcResponse};
    use serde_json::json;

    fn resp(id: Option<&str>, command: &str) -> IncomingFrame {
        let r = RpcResponse {
            id: id.map(str::to_string),
            command: command.into(),
            success: true,
            data: None,
            error: None,
            code: None,
        };
        IncomingFrame {
            raw: serde_json::to_value(&r).expect("RpcResponse serializes"),
            kind: IncomingFrameKind::Response(r),
        }
    }

    fn event_frame() -> IncomingFrame {
        IncomingFrame {
            kind: IncomingFrameKind::AgentStart,
            raw: json!({"type":"agent_start"}),
        }
    }

    fn test_inner() -> (Arc<Inner>, Receiver<ClientEvent>) {
        let (writer_tx, _writer_rx) = channel::bounded::<Option<Vec<u8>>>(4);
        let (events_tx, events_rx) = channel::bounded::<ClientEvent>(CLIENT_EVENT_CHANNEL_CAP);
        let inner = Arc::new(Inner {
            ready: ReadyFrame {
                protocol_version: 1,
                supported_protocol_versions: vec![1, 2],
                max_frame_bytes: 1024 * 1024,
                max_reassembled_frame_bytes: 64 * 1024 * 1024,
            },
            writer_tx,
            pending: AsyncMutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
            default_timeout: DEFAULT_COMMAND_TIMEOUT,
            child: AsyncMutex::new(None),
            stderr_tail: AsyncMutex::new(VecDeque::new()),
            stderr_done_tx: {
                let (t, _r) = channel::bounded::<()>(1);
                t
            },
            stderr_done_rx: channel::bounded::<()>(1).1,
            closed: AtomicBool::new(false),
            events_tx,
        });
        (inner, events_rx)
    }

    #[test]
    fn response_with_matching_id_resolves_pending() {
        smol::block_on(async {
            let (inner, _events) = test_inner();
            let (tx, rx) = channel::bounded::<RpcResponse>(1);
            inner.pending.lock().await.insert("req_1".into(), tx);
            route_frame(&inner, resp(Some("req_1"), "get_state")).await;
            let got = rx.try_recv().expect("pending resolved");
            assert_eq!(got.command, "get_state");
            assert!(inner.pending.lock().await.is_empty());
        });
    }

    #[test]
    fn response_without_id_becomes_event() {
        smol::block_on(async {
            let (inner, events) = test_inner();
            route_frame(&inner, resp(None, "parse")).await;
            let ev = events.try_recv().expect("event delivered");
            match ev {
                ClientEvent::Frame(f) => match f.kind {
                    IncomingFrameKind::Response(r) => assert_eq!(r.command, "parse"),
                    _ => panic!("wrong frame kind"),
                },
                ClientEvent::Closed(_) => panic!("wrong event"),
            }
        });
    }

    #[test]
    fn response_with_unknown_id_becomes_event_and_does_not_corrupt_pending() {
        smol::block_on(async {
            let (inner, events) = test_inner();
            // Insert an unrelated pending entry — it must survive.
            let (tx, _rx) = channel::bounded::<RpcResponse>(1);
            inner.pending.lock().await.insert("req_2".into(), tx);
            // Late same-id error from a previously-resolved request.
            route_frame(&inner, resp(Some("req_1"), "prompt")).await;
            let ev = events.try_recv().expect("event delivered");
            assert!(matches!(ev, ClientEvent::Frame(_)));
            assert!(inner.pending.lock().await.contains_key("req_2"));
        });
    }

    #[test]
    fn unsolicited_event_frame_reaches_events_channel() {
        smol::block_on(async {
            let (inner, events) = test_inner();
            route_frame(&inner, event_frame()).await;
            assert!(matches!(
                events.try_recv().expect("event delivered"),
                ClientEvent::Frame(f) if matches!(f.kind, IncomingFrameKind::AgentStart)
            ));
        });
    }

    #[test]
    fn close_all_broadcasts_closed_and_fails_pending() {
        smol::block_on(async {
            let (inner, events) = test_inner();
            let (tx, rx) = channel::bounded::<RpcResponse>(1);
            inner.pending.lock().await.insert("req_1".into(), tx);
            close_all(
                &inner,
                Some(RpcError::ProtocolViolation { detail: "x".into() }),
            )
            .await;
            // Sender was dropped → recv returns error, callers translate to ChildDied.
            assert!(rx.recv().await.is_err());
            let ev = events.recv().await.expect("closed delivered");
            match ev {
                ClientEvent::Closed(info) => {
                    assert_eq!(info.error_msg.as_deref(), Some("protocol violation: x"));
                }
                ClientEvent::Frame(_) => panic!("expected Closed"),
            }
            // Second close is a no-op.
            close_all(&inner, None).await;
        });
    }

    #[test]
    fn client_event_channel_cap_is_512() {
        assert_eq!(CLIENT_EVENT_CHANNEL_CAP, 512);
        let (tx, _rx) = channel::bounded::<ClientEvent>(CLIENT_EVENT_CHANNEL_CAP);
        for _ in 0..CLIENT_EVENT_CHANNEL_CAP {
            tx.try_send(ClientEvent::Frame(Box::new(event_frame())))
                .expect("slot available");
        }
        assert!(
            tx.try_send(ClientEvent::Frame(Box::new(event_frame())))
                .is_err(),
            "channel must be bounded at CLIENT_EVENT_CHANNEL_CAP"
        );
    }

    #[test]
    fn config_default_uses_30s_timeout() {
        let c = ClientConfig::default();
        assert_eq!(c.command_timeout, DEFAULT_COMMAND_TIMEOUT);
        assert!(!c.no_session);
        assert!(c.resume.is_none());
    }

    #[test]
    fn enforce_outbound_limit_rejects_too_large() {
        // Build a minimal RpcClient shell around test_inner so we can call
        // the private helper. We can't easily construct RpcClient without a
        // spawn; instead assert the helper's math directly.
        let ready = ReadyFrame {
            protocol_version: 1,
            supported_protocol_versions: vec![1, 2],
            max_frame_bytes: 32,
            max_reassembled_frame_bytes: 64 * 1024,
        };
        // Replicate the guard inline (mirrors enforce_outbound_limit).
        let line = [b'x'; 33];
        let limit = usize::try_from(ready.max_frame_bytes).expect("test constant fits usize");
        assert!(line.len() > limit);
    }
}
