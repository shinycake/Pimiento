//! §4.8 / §5.3 process supervision policy for the OMP RPC child.
//!
//! This module is intentionally decoupled from wire decoding and from
//! `client.rs`: it operates over a [`ProcessHandle`] trait so supervision
//! policy is unit-testable without spawning a real `omp`, and so the
//! upper layer can plug in the concrete `RpcClient` (from `M1Client`)
//! via a thin adapter without any changes here.
//!
//! Policy owned here:
//! * Argv construction (`--mode rpc-ui` always; `--no-session` XOR
//!   `--resume <path>`).
//! * Stderr 64 KiB tail ring.
//! * Graceful shutdown: close stdin → wait 10 s → request termination →
//!   force kill → reap. All portable, safe APIs — no `libc` signal
//!   calls, no `unsafe`.
//! * Crash-loop breaker: three restarts within a rolling 60 s window
//!   transitions to [`SupervisorState::Dead`] and requires
//!   [`Supervisor::reset`].
//! * Truthful state transitions surfaced as [`SupervisorEvent`]s.
//! * Pending-request failure on death is delegated to the
//!   [`ProcessHandle`] impl (the concrete client fails its pending map
//!   with [`RpcError::ChildDied`] when its own reader observes EOF).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use parking_lot::Mutex;
use smol::channel;

use crate::error::RpcError;

// ---------------------------------------------------------------------
// Public policy inputs
// ---------------------------------------------------------------------

/// Configuration handed to the supervisor. Everything the spawn side
/// needs is captured up front so restarts are pure functions of the
/// current [`SupervisorConfig`] (with `resume` possibly updated by the
/// upper layer between attempts).
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// Caller-controlled extra CLI args appended *after* the mode/session
    /// switches. Reserved for tests and future flags.
    pub extra_args: Vec<OsString>,
    /// `--no-session` (mutually exclusive with `resume`).
    pub no_session: bool,
    /// Last observed `sessionFile` from `get_state`. When set (and
    /// `no_session` is false), the next spawn appends `--resume <path>`.
    pub resume: Option<PathBuf>,
}

impl SupervisorConfig {
    /// Build the argv vector the child will be spawned with, in the
    /// order specified by PLAN §4.7. `--mode rpc-ui` is always first.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError::ProtocolViolation`] if `no_session` and
    /// `resume` are both set — PLAN §4.7 makes them mutually
    /// exclusive.
    pub fn build_argv(&self) -> Result<Vec<OsString>, RpcError> {
        if self.no_session && self.resume.is_some() {
            return Err(RpcError::ProtocolViolation {
                detail: "--no-session is incompatible with --resume".into(),
            });
        }
        let mut argv: Vec<OsString> = Vec::with_capacity(4 + self.extra_args.len());
        argv.push(OsString::from("--mode"));
        argv.push(OsString::from("rpc-ui"));
        if self.no_session {
            argv.push(OsString::from("--no-session"));
        } else if let Some(path) = &self.resume {
            argv.push(OsString::from("--resume"));
            argv.push(path.clone().into_os_string());
        }
        argv.extend(self.extra_args.iter().cloned());
        Ok(argv)
    }
}

// ---------------------------------------------------------------------
// State / events
// ---------------------------------------------------------------------

/// Terminal reason for a supervisor entering [`SupervisorState::Dead`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeathReason {
    /// Three restarts within a rolling 60 s window.
    CrashLoop,
    /// The caller invoked [`Supervisor::shutdown`].
    Explicit,
    /// A non-retryable error surfaced before the child ever became
    /// healthy (e.g. discovery failure, handshake mismatch).
    Fatal(String),
}

/// Truthful lifecycle of the supervised child. Never silently
/// respawns: every transition is emitted as a [`SupervisorEvent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorState {
    /// Child is up and events flow.
    Running,
    /// A restart is in progress. `attempt` is 1-based and monotonic
    /// within the current 60 s window.
    Restarting { attempt: u32 },
    /// Terminal. Requires [`Supervisor::reset`] before another start.
    Dead {
        reason: DeathReason,
        exit_code: Option<i32>,
        stderr_tail: String,
    },
}

/// Coarse-grained supervisor event stream.
///
/// This is deliberately narrow: frame-level events belong to the
/// [`ProcessHandle`]'s own receiver (which the upper layer wires
/// straight into the GPUI session pump). The supervisor stream only
/// surfaces its own state changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorEvent {
    StateChanged(SupervisorState),
}

// ---------------------------------------------------------------------
// Injection surfaces (clock + process handle)
// ---------------------------------------------------------------------

/// Monotonic time source. `SystemClock` uses [`Instant::now`]; tests
/// inject [`FakeClock`] for deterministic breaker boundary coverage.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

/// Default clock backed by [`Instant::now`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Terminal information reported after the child has been fully reaped.
///
/// Wire-compatible with the shape `M1Client` emits on `ClientEvent::Closed`
/// (the adapter maps that variant into this struct).
#[derive(Debug, Clone)]
pub struct ClosedInfo {
    pub exit_code: Option<i32>,
    pub stderr_tail: String,
    pub error_msg: Option<String>,
}

/// Abstract handle to a running child. The concrete implementation
/// lives in the client layer (adapter around `RpcClient` from
/// `M1Client`); tests provide a lightweight fake.
///
/// Semantics required of implementors:
/// * `close_stdin` half-closes the child's stdin (graceful shutdown).
/// * `kill` requests hard termination via
///   `smol::process::Child::kill` — a safe, portable, non-`libc` API.
/// * `wait` blocks until the child is fully reaped and returns the
///   terminal `ClosedInfo`. Implementors must fail all pending
///   requests with [`RpcError::ChildDied`] before or during `wait`.
/// * `terminated` yields once when the child exits (any reason). Used
///   by the supervisor's shutdown timer.
pub trait ProcessHandle: Send + 'static {
    fn close_stdin(&self);
    fn kill(&self);
    fn terminated(&self) -> channel::Receiver<ClosedInfo>;
    fn wait(self: Box<Self>) -> channel::Receiver<ClosedInfo>;
}

/// Factory for [`ProcessHandle`]s. `spawn` is invoked once per
/// supervisor start and once per restart; each call must yield a
/// *fresh* handle (fresh decoder, correlation map, stdin writer — see
/// PLAN §4.8).
pub trait Spawner: Send + Sync + 'static {
    fn spawn(
        &self,
        argv: &[OsString],
    ) -> smol::channel::Receiver<Result<Box<dyn ProcessHandle>, RpcError>>;
}

// ---------------------------------------------------------------------
// Stderr tail ring
// ---------------------------------------------------------------------

/// Fixed-capacity ring buffer for the last N bytes of the child's
/// stderr. When the child dies, the tail is embedded in the
/// `RpcError::ChildDied` handed to pending callers and in the
/// [`SupervisorState::Dead`] payload.
///
/// PLAN §4.8 pins the capacity at 64 KiB.
#[derive(Debug)]
pub struct StderrRing {
    buf: VecDeque<u8>,
    cap: usize,
}

/// The 64 KiB tail size mandated by PLAN §4.8.
pub const STDERR_TAIL_CAPACITY: usize = 64 * 1024;

impl StderrRing {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(capacity.min(1 << 20)),
            cap: capacity,
        }
    }

    /// Fresh ring at the 64 KiB PLAN default.
    #[must_use]
    pub fn with_default_capacity() -> Self {
        Self::new(STDERR_TAIL_CAPACITY)
    }

    /// Append `chunk`, dropping oldest bytes as needed to stay within
    /// capacity. A chunk larger than the ring capacity keeps only its
    /// last `capacity` bytes (tail-preserving).
    pub fn push(&mut self, chunk: &[u8]) {
        if self.cap == 0 {
            return;
        }
        if chunk.len() >= self.cap {
            self.buf.clear();
            self.buf.extend(&chunk[chunk.len() - self.cap..]);
            return;
        }
        let overflow = (self.buf.len() + chunk.len()).saturating_sub(self.cap);
        for _ in 0..overflow {
            self.buf.pop_front();
        }
        self.buf.extend(chunk);
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Snapshot as a lossy UTF-8 string. Callers embed this in
    /// diagnostics; we never surface stderr bytes as strict UTF-8
    /// because tools frequently emit ANSI/CRLF/binary spinners.
    #[must_use]
    pub fn to_string_lossy(&self) -> String {
        let (a, b) = self.buf.as_slices();
        let mut bytes = Vec::with_capacity(a.len() + b.len());
        bytes.extend_from_slice(a);
        bytes.extend_from_slice(b);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

// ---------------------------------------------------------------------
// Crash-loop breaker
// ---------------------------------------------------------------------

/// Rolling-window restart counter. PLAN §4.8: the third restart within
/// 60 s trips the breaker and demands manual reset.
#[derive(Debug, Clone)]
pub struct RestartBreaker {
    window: Duration,
    limit: u32,
    events: VecDeque<Instant>,
}

impl RestartBreaker {
    /// 3 restarts / 60 s per PLAN §4.8.
    #[must_use]
    pub fn plan_default() -> Self {
        Self::new(3, Duration::from_mins(1))
    }

    #[must_use]
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            window,
            limit,
            events: VecDeque::new(),
        }
    }

    /// Record a restart at `now`; return `true` if the breaker is
    /// tripped (i.e. this restart pushed the window over the limit).
    ///
    /// Semantics: with `limit=3` and `window=60s`, a third restart
    /// whose timestamp falls within 60 s of the earliest retained
    /// event trips. Exactly-at-60s is *outside* the window (open on
    /// the trailing edge) so a slow-drip of one restart every 60 s
    /// does not trip.
    pub fn record(&mut self, now: Instant) -> bool {
        self.events.push_back(now);
        // Evict anything older than `window` relative to `now`.
        while let Some(&front) = self.events.front() {
            if now.duration_since(front) >= self.window {
                self.events.pop_front();
            } else {
                break;
            }
        }
        u32::try_from(self.events.len()).unwrap_or(u32::MAX) >= self.limit
    }

    /// Attempts observed in the current window.
    #[must_use]
    pub fn attempts_in_window(&self) -> u32 {
        u32::try_from(self.events.len()).unwrap_or(u32::MAX)
    }

    /// Clear all recorded events. Called from [`Supervisor::reset`].
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

// ---------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------

/// Cross-thread-shared supervisor state. Kept behind a `Mutex` because
/// the mutation surface is tiny (state transitions + config
/// mutations) and lock hold times are microseconds — a plain `Mutex`
/// is the boring correct choice over async cell juggling.
#[derive(Debug)]
struct Inner {
    config: SupervisorConfig,
    state: SupervisorState,
    breaker: RestartBreaker,
    stderr: StderrRing,
}

/// Public supervisor handle. Cheaply cloneable via `Arc`.
#[derive(Clone)]
pub struct Supervisor {
    inner: Arc<Mutex<Inner>>,
    events_tx: channel::Sender<SupervisorEvent>,
    events_rx: channel::Receiver<SupervisorEvent>,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor").finish_non_exhaustive()
    }
}

impl Supervisor {
    /// Construct a supervisor without starting the child. Callers then
    /// drive lifecycle explicitly; higher layers (or a future
    /// `Supervisor::run`) can wire this to a `Spawner` + `Clock` and
    /// an event pump.
    #[must_use]
    pub fn new(config: SupervisorConfig) -> Self {
        let (tx, rx) = channel::unbounded();
        Self {
            inner: Arc::new(Mutex::new(Inner {
                config,
                state: SupervisorState::Running,
                breaker: RestartBreaker::plan_default(),
                stderr: StderrRing::with_default_capacity(),
            })),
            events_tx: tx,
            events_rx: rx,
        }
    }

    /// Receiver for state-change events.
    #[must_use]
    pub fn events(&self) -> channel::Receiver<SupervisorEvent> {
        self.events_rx.clone()
    }

    /// Snapshot of the current state.
    #[must_use]
    pub fn state(&self) -> SupervisorState {
        self.inner.lock().state.clone()
    }

    /// Current resume pointer, if any.
    #[must_use]
    pub fn resume(&self) -> Option<PathBuf> {
        self.inner.lock().config.resume.clone()
    }

    /// Update the resume pointer captured from the last `get_state`.
    /// The next restart's argv will include it.
    pub fn set_resume(&self, path: Option<PathBuf>) {
        self.inner.lock().config.resume = path;
    }

    /// Feed stderr bytes into the tail ring.
    pub fn record_stderr(&self, bytes: &[u8]) {
        self.inner.lock().stderr.push(bytes);
    }

    /// Snapshot of the stderr tail.
    #[must_use]
    pub fn stderr_tail(&self) -> String {
        self.inner.lock().stderr.to_string_lossy()
    }

    /// Record a restart attempt at `now`. Returns the new state:
    /// * `Restarting { attempt }` on the first/second attempt within
    ///   the window.
    /// * `Dead { reason: CrashLoop, .. }` when the breaker trips.
    ///
    /// The exit code and stderr tail associated with the death that
    /// triggered the restart are threaded in by the caller so the
    /// terminal `Dead` payload is truthful — supervisor never
    /// silently drops them.
    #[must_use]
    pub fn on_child_died(
        &self,
        now: Instant,
        exit_code: Option<i32>,
        stderr_tail: String,
    ) -> SupervisorState {
        let mut inner = self.inner.lock();
        // Merge tails: prefer the ProcessHandle-provided one (client
        // already draining stderr) but retain anything the supervisor
        // itself observed via `record_stderr`.
        let combined = if stderr_tail.is_empty() {
            inner.stderr.to_string_lossy()
        } else {
            stderr_tail
        };
        let tripped = inner.breaker.record(now);
        let next = if tripped {
            SupervisorState::Dead {
                reason: DeathReason::CrashLoop,
                exit_code,
                stderr_tail: combined,
            }
        } else {
            SupervisorState::Restarting {
                attempt: inner.breaker.attempts_in_window(),
            }
        };
        inner.state = next.clone();
        drop(inner);
        let _ = self
            .events_tx
            .try_send(SupervisorEvent::StateChanged(next.clone()));
        next
    }

    /// Mark the child healthy (ready → negotiate → `get_state`
    /// succeeded). Emits `StateChanged(Running)`.
    pub fn on_child_running(&self) {
        let mut inner = self.inner.lock();
        inner.state = SupervisorState::Running;
        drop(inner);
        let _ = self
            .events_tx
            .try_send(SupervisorEvent::StateChanged(SupervisorState::Running));
    }

    /// Caller-driven explicit shutdown. Emits terminal
    /// `Dead { reason: Explicit, .. }`.
    pub fn on_explicit_shutdown(&self, exit_code: Option<i32>, stderr_tail: String) {
        let mut inner = self.inner.lock();
        let next = SupervisorState::Dead {
            reason: DeathReason::Explicit,
            exit_code,
            stderr_tail,
        };
        inner.state = next.clone();
        drop(inner);
        let _ = self.events_tx.try_send(SupervisorEvent::StateChanged(next));
    }

    /// Non-retryable fatal error before or after the child is up.
    pub fn on_fatal(&self, err: &RpcError, exit_code: Option<i32>, stderr_tail: String) {
        let mut inner = self.inner.lock();
        let next = SupervisorState::Dead {
            reason: DeathReason::Fatal(err.to_string()),
            exit_code,
            stderr_tail,
        };
        inner.state = next.clone();
        drop(inner);
        let _ = self.events_tx.try_send(SupervisorEvent::StateChanged(next));
    }

    /// Manual recovery from a `Dead` state. Clears the breaker window
    /// and returns to `Running`-eligible (does not itself respawn — the
    /// upper layer decides when to `Spawner::spawn` again).
    ///
    /// # Errors
    ///
    /// Returns [`RpcError::ProtocolViolation`] if the current state is
    /// not [`SupervisorState::Dead`].
    pub fn reset(&self) -> Result<(), RpcError> {
        let mut inner = self.inner.lock();
        if !matches!(inner.state, SupervisorState::Dead { .. }) {
            return Err(RpcError::ProtocolViolation {
                detail: "supervisor reset requested outside Dead state".into(),
            });
        }
        inner.breaker.clear();
        inner.stderr = StderrRing::with_default_capacity();
        inner.state = SupervisorState::Restarting { attempt: 0 };
        drop(inner);
        let _ =
            self.events_tx
                .try_send(SupervisorEvent::StateChanged(SupervisorState::Restarting {
                    attempt: 0,
                }));
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Graceful shutdown driver
// ---------------------------------------------------------------------

/// PLAN §4.8 graceful shutdown deadline before termination is requested.
pub const GRACEFUL_STDIN_CLOSE_DEADLINE: Duration = Duration::from_secs(10);

/// Outcome of [`shutdown_child`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShutdownOutcome {
    /// Child exited on its own after `close_stdin` (0 ≤ elapsed < 10 s).
    GracefulExit,
    /// Deadline elapsed → we called `kill()` and then reaped.
    Killed,
}

/// Drive the two-phase shutdown sequence:
/// 1. `close_stdin`
/// 2. wait up to [`GRACEFUL_STDIN_CLOSE_DEADLINE`] for `terminated`
/// 3. if still alive, `kill`, then reap via `wait`
///
/// All steps use safe, portable APIs (`smol::process::Child::kill`);
/// no `libc` signal calls, no `unsafe`.
///
/// Returns the `ClosedInfo` alongside the outcome.
pub async fn shutdown_child(
    handle: Box<dyn ProcessHandle>,
    deadline: Duration,
) -> (ShutdownOutcome, ClosedInfo) {
    handle.close_stdin();
    let terminated = handle.terminated();

    // Race graceful exit against the deadline. Both branches yield
    // `Option<ClosedInfo>` so the outer match stays flat.
    let race = smol::future::or(
        async {
            let info = terminated.recv().await.ok();
            (ShutdownOutcome::GracefulExit, info)
        },
        async {
            smol::Timer::after(deadline).await;
            (ShutdownOutcome::Killed, None)
        },
    )
    .await;

    if let (ShutdownOutcome::GracefulExit, Some(info)) = race {
        (ShutdownOutcome::GracefulExit, info)
    } else {
        // Graceful window expired (or the terminated channel closed
        // before delivering info): force termination and reap.
        handle.kill();
        drop(terminated);
        let info = handle.wait().recv().await.unwrap_or(ClosedInfo {
            exit_code: None,
            stderr_tail: String::new(),
            error_msg: None,
        });
        (ShutdownOutcome::Killed, info)
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    // ---------- argv construction ---------------------------------

    #[test]
    fn argv_default_is_rpc_ui_only() {
        let cfg = SupervisorConfig {
            extra_args: vec![],
            no_session: false,
            resume: None,
        };
        let argv = cfg.build_argv().expect("valid argv config");
        assert_eq!(
            argv,
            vec![OsString::from("--mode"), OsString::from("rpc-ui")]
        );
    }

    #[test]
    fn argv_no_session_flag() {
        let cfg = SupervisorConfig {
            extra_args: vec![],
            no_session: true,
            resume: None,
        };
        let argv = cfg.build_argv().expect("valid argv config");
        assert_eq!(
            argv,
            vec![
                OsString::from("--mode"),
                OsString::from("rpc-ui"),
                OsString::from("--no-session"),
            ]
        );
    }

    #[test]
    fn argv_resume_appends_pair() {
        let cfg = SupervisorConfig {
            extra_args: vec![],
            no_session: false,
            resume: Some(PathBuf::from("/tmp/sess.json")),
        };
        let argv = cfg.build_argv().expect("valid argv config");
        assert_eq!(
            argv,
            vec![
                OsString::from("--mode"),
                OsString::from("rpc-ui"),
                OsString::from("--resume"),
                OsString::from("/tmp/sess.json"),
            ]
        );
    }

    #[test]
    fn argv_no_session_and_resume_are_mutually_exclusive() {
        let cfg = SupervisorConfig {
            extra_args: vec![],
            no_session: true,
            resume: Some(PathBuf::from("/tmp/sess.json")),
        };
        assert!(matches!(
            cfg.build_argv(),
            Err(RpcError::ProtocolViolation { .. })
        ));
    }

    #[test]
    fn argv_extra_args_come_last() {
        let cfg = SupervisorConfig {
            extra_args: vec![OsString::from("--verbose")],
            no_session: false,
            resume: Some(PathBuf::from("/s")),
        };
        let argv = cfg.build_argv().expect("valid argv config");
        assert_eq!(argv.last(), Some(&OsString::from("--verbose")));
    }

    // ---------- stderr ring truncation ----------------------------

    #[test]
    fn stderr_ring_keeps_last_bytes_within_cap() {
        let mut ring = StderrRing::new(8);
        ring.push(b"abcdefghij"); // 10 bytes, cap 8 -> keep last 8
        assert_eq!(ring.len(), 8);
        assert_eq!(ring.to_string_lossy(), "cdefghij");
    }

    #[test]
    fn stderr_ring_streaming_pushes_evict_from_front() {
        let mut ring = StderrRing::new(4);
        ring.push(b"AB");
        ring.push(b"CD");
        ring.push(b"EF");
        assert_eq!(ring.to_string_lossy(), "CDEF");
    }

    #[test]
    fn stderr_ring_at_default_capacity_is_64kib() {
        let mut ring = StderrRing::with_default_capacity();
        // Feed 3 * 64 KiB, verify tail is exactly the last 64 KiB.
        let mut counter: u8 = 0;
        for _ in 0..(3 * STDERR_TAIL_CAPACITY) {
            ring.push(&[counter]);
            counter = counter.wrapping_add(1);
        }
        assert_eq!(ring.len(), STDERR_TAIL_CAPACITY);
    }

    #[test]
    fn stderr_ring_zero_cap_is_no_op() {
        let mut ring = StderrRing::new(0);
        ring.push(b"never stored");
        assert!(ring.is_empty());
        assert_eq!(ring.to_string_lossy(), "");
    }

    #[test]
    fn stderr_ring_lossy_utf8_survives_binary_bytes() {
        let mut ring = StderrRing::new(16);
        ring.push(&[0xC3, 0x28, b'X']); // invalid UTF-8 followed by ASCII
        let s = ring.to_string_lossy();
        assert!(s.ends_with('X'));
    }

    // ---------- restart breaker boundaries ------------------------

    fn t(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn breaker_first_two_restarts_within_window_do_not_trip() {
        let base = Instant::now();
        let mut b = RestartBreaker::new(3, Duration::from_mins(1));
        assert!(!b.record(t(base, 0)));
        assert!(!b.record(t(base, 30)));
        assert_eq!(b.attempts_in_window(), 2);
    }

    #[test]
    fn breaker_third_restart_within_window_trips() {
        let base = Instant::now();
        let mut b = RestartBreaker::new(3, Duration::from_mins(1));
        b.record(t(base, 0));
        b.record(t(base, 30));
        let tripped = b.record(t(base, 59));
        assert!(
            tripped,
            "third restart at t=59s within 60s window must trip"
        );
    }

    #[test]
    fn breaker_third_restart_exactly_at_window_edge_does_not_trip() {
        let base = Instant::now();
        let mut b = RestartBreaker::new(3, Duration::from_mins(1));
        b.record(t(base, 0));
        b.record(t(base, 30));
        // t=60 evicts the t=0 entry (>= window), leaving 2 events → not tripped.
        let tripped = b.record(t(base, 60));
        assert!(!tripped, "restart at exactly window edge must not trip");
        assert_eq!(b.attempts_in_window(), 2);
    }

    #[test]
    fn breaker_slow_drip_never_trips() {
        let base = Instant::now();
        let mut b = RestartBreaker::new(3, Duration::from_mins(1));
        for i in 0..10 {
            let tripped = b.record(t(base, i * 60));
            assert!(!tripped, "one restart per window must never trip (i={i})");
        }
    }

    #[test]
    fn breaker_clears_on_reset() {
        let base = Instant::now();
        let mut b = RestartBreaker::plan_default();
        b.record(t(base, 0));
        b.record(t(base, 10));
        b.record(t(base, 20));
        assert_eq!(b.attempts_in_window(), 3);
        b.clear();
        assert_eq!(b.attempts_in_window(), 0);
        assert!(!b.record(t(base, 21)));
    }

    // ---------- Supervisor state transitions ----------------------

    #[test]
    fn supervisor_starts_running_and_emits_no_events_yet() {
        let sup = Supervisor::new(SupervisorConfig {
            extra_args: vec![],
            no_session: false,
            resume: None,
        });
        assert_eq!(sup.state(), SupervisorState::Running);
        assert!(
            sup.events().try_recv().is_err(),
            "no events until a transition"
        );
    }

    #[test]
    fn supervisor_transitions_running_restarting_dead_on_repeated_deaths() {
        let sup = Supervisor::new(SupervisorConfig {
            extra_args: vec![],
            no_session: false,
            resume: None,
        });
        let base = Instant::now();

        let s1 = sup.on_child_died(t(base, 0), Some(1), "oops-1".into());
        assert_eq!(s1, SupervisorState::Restarting { attempt: 1 });

        sup.on_child_running();
        assert_eq!(sup.state(), SupervisorState::Running);

        let s2 = sup.on_child_died(t(base, 10), Some(2), "oops-2".into());
        assert_eq!(s2, SupervisorState::Restarting { attempt: 2 });

        let s3 = sup.on_child_died(t(base, 20), Some(3), "oops-3".into());
        match s3 {
            SupervisorState::Dead {
                reason,
                exit_code,
                stderr_tail,
            } => {
                assert_eq!(reason, DeathReason::CrashLoop);
                assert_eq!(exit_code, Some(3));
                assert_eq!(stderr_tail, "oops-3");
            }
            other => panic!("expected Dead(CrashLoop), got {other:?}"),
        }

        // Reset returns supervisor to Restarting{0} and clears the breaker.
        sup.reset().expect("reset succeeds from Dead state");
        assert_eq!(sup.state(), SupervisorState::Restarting { attempt: 0 });
        // After reset, a single death should be Restarting{1} again, not Dead.
        let s = sup.on_child_died(t(base, 21), Some(4), "oops-4".into());
        assert_eq!(s, SupervisorState::Restarting { attempt: 1 });
    }

    #[test]
    fn supervisor_reset_rejected_outside_dead() {
        let sup = Supervisor::new(SupervisorConfig {
            extra_args: vec![],
            no_session: false,
            resume: None,
        });
        assert!(matches!(
            sup.reset(),
            Err(RpcError::ProtocolViolation { .. })
        ));
    }

    #[test]
    fn supervisor_explicit_shutdown_marks_dead_with_explicit_reason() {
        let sup = Supervisor::new(SupervisorConfig {
            extra_args: vec![],
            no_session: false,
            resume: None,
        });
        sup.on_explicit_shutdown(Some(0), "bye".into());
        match sup.state() {
            SupervisorState::Dead {
                reason,
                exit_code,
                stderr_tail,
            } => {
                assert_eq!(reason, DeathReason::Explicit);
                assert_eq!(exit_code, Some(0));
                assert_eq!(stderr_tail, "bye");
            }
            other => panic!("expected Dead(Explicit), got {other:?}"),
        }
    }

    #[test]
    fn supervisor_fatal_captures_error_message() {
        let sup = Supervisor::new(SupervisorConfig {
            extra_args: vec![],
            no_session: false,
            resume: None,
        });
        let err = RpcError::UnsupportedProtocol { supported: vec![1] };
        sup.on_fatal(&err, None, String::new());
        match sup.state() {
            SupervisorState::Dead {
                reason: DeathReason::Fatal(msg),
                ..
            } => {
                assert!(msg.contains("unsupported OMP protocol"), "got {msg:?}");
            }
            other => panic!("expected Dead(Fatal), got {other:?}"),
        }
    }

    #[test]
    fn supervisor_events_stream_is_ordered_and_complete() {
        let sup = Supervisor::new(SupervisorConfig {
            extra_args: vec![],
            no_session: false,
            resume: None,
        });
        let rx = sup.events();
        let base = Instant::now();
        let _ = sup.on_child_died(t(base, 0), Some(1), "a".into());
        sup.on_child_running();
        let _ = sup.on_child_died(t(base, 5), Some(2), "b".into());
        let _ = sup.on_child_died(t(base, 10), Some(3), "c".into());

        let mut collected = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            collected.push(ev);
        }
        assert!(matches!(
            collected[0],
            SupervisorEvent::StateChanged(SupervisorState::Restarting { attempt: 1 })
        ));
        assert!(matches!(
            collected[1],
            SupervisorEvent::StateChanged(SupervisorState::Running)
        ));
        assert!(matches!(
            collected[2],
            SupervisorEvent::StateChanged(SupervisorState::Restarting { attempt: 2 })
        ));
        assert!(matches!(
            collected[3],
            SupervisorEvent::StateChanged(SupervisorState::Dead {
                reason: DeathReason::CrashLoop,
                ..
            })
        ));
    }

    #[test]
    fn supervisor_resume_pointer_flows_into_next_argv() {
        let sup = Supervisor::new(SupervisorConfig {
            extra_args: vec![],
            no_session: false,
            resume: None,
        });
        sup.set_resume(Some(PathBuf::from("/tmp/session-42.json")));
        let cfg = sup.inner.lock().config.clone();
        let argv = cfg.build_argv().expect("valid argv config");
        assert!(argv.windows(2).any(|w| w
            == [
                OsString::from("--resume"),
                OsString::from("/tmp/session-42.json")
            ]));
        assert_eq!(sup.resume(), Some(PathBuf::from("/tmp/session-42.json")));
    }

    // ---------- graceful shutdown driver --------------------------

    /// Minimal fake `ProcessHandle` so we can exercise `shutdown_child`
    /// deterministically without spawning a real child.
    struct FakeChild {
        closed_stdin: Arc<AtomicBool>,
        killed: Arc<AtomicBool>,
        terminated_tx: channel::Sender<ClosedInfo>,
        terminated_rx: channel::Receiver<ClosedInfo>,
        /// If set, terminate immediately once `close_stdin` is called.
        graceful: bool,
    }

    impl FakeChild {
        fn new(graceful: bool) -> Self {
            let (tx, rx) = channel::unbounded();
            Self {
                closed_stdin: Arc::new(AtomicBool::new(false)),
                killed: Arc::new(AtomicBool::new(false)),
                terminated_tx: tx,
                terminated_rx: rx,
                graceful,
            }
        }
    }

    impl ProcessHandle for FakeChild {
        fn close_stdin(&self) {
            self.closed_stdin.store(true, Ordering::SeqCst);
            if self.graceful {
                let _ = self.terminated_tx.try_send(ClosedInfo {
                    exit_code: Some(0),
                    stderr_tail: "graceful".into(),
                    error_msg: None,
                });
            }
        }
        fn kill(&self) {
            self.killed.store(true, Ordering::SeqCst);
            let _ = self.terminated_tx.try_send(ClosedInfo {
                exit_code: Some(137),
                stderr_tail: "killed".into(),
                error_msg: None,
            });
        }
        fn terminated(&self) -> channel::Receiver<ClosedInfo> {
            self.terminated_rx.clone()
        }
        fn wait(self: Box<Self>) -> channel::Receiver<ClosedInfo> {
            self.terminated_rx.clone()
        }
    }

    #[test]
    fn shutdown_graceful_when_child_exits_before_deadline() {
        let child = Box::new(FakeChild::new(true));
        let killed_flag = child.killed.clone();
        let closed_flag = child.closed_stdin.clone();
        let (outcome, info) = smol::block_on(shutdown_child(child, Duration::from_secs(5)));
        assert_eq!(outcome, ShutdownOutcome::GracefulExit);
        assert!(closed_flag.load(Ordering::SeqCst));
        assert!(
            !killed_flag.load(Ordering::SeqCst),
            "kill must not fire on graceful exit"
        );
        assert_eq!(info.exit_code, Some(0));
    }

    #[test]
    fn shutdown_kills_after_deadline_when_child_hangs() {
        let child = Box::new(FakeChild::new(false));
        let killed_flag = child.killed.clone();
        // Deadline of 50 ms — deterministic and fast.
        let (outcome, info) = smol::block_on(shutdown_child(child, Duration::from_millis(50)));
        assert_eq!(outcome, ShutdownOutcome::Killed);
        assert!(
            killed_flag.load(Ordering::SeqCst),
            "kill must fire past deadline"
        );
        assert_eq!(info.exit_code, Some(137));
    }
}
