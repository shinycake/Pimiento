//! Live fixture recorder for `pimiento-core` projection replay tests
//! (PLAN §§6 M2, 10.2).
//!
//! Spawns the user's real `omp` in `--mode rpc-ui --no-session`, drives one
//! of four scripted scenarios, and writes the child's stdout — the raw
//! physical NDJSON server frame stream, including `rpc_chunk` frames
//! byte-for-byte — to a caller-supplied fixture file. The recorder is
//! deliberately not built on top of [`RpcClient`] because that type only
//! exposes reassembled logical events; the fixture contract in
//! `docs/protocol-notes.md` requires the raw pre-reassembly bytes so the
//! replay tests exercise `LineDecoder` → `ChunkReassembler` → `decode_frame`
//! end-to-end.
//!
//! Design invariants:
//!
//! * **Raw stdout is tee'd verbatim.** The recorder writes every byte
//!   received on the child's stdout to the temp fixture. There is no
//!   normalization, no filtering, no comment lines — just NDJSON.
//! * **Logical parsing is a side channel.** The same bytes go through
//!   the M1 decoding stack so the driver knows when `ready`/responses/
//!   `agent_end` arrive, and so we can scan decoded logical frames for
//!   obvious secret key names before committing the fixture to disk.
//! * **Stdin is written only by us**, only with valid JSON frames. Every
//!   scenario auto-negotiates protocol v2 first (matching `RpcClient`) so
//!   chunked responses are exercisable.
//! * **Child stderr is drained to /dev/null.** It is never written to the
//!   fixture and never inspected for secrets.
//! * **Overwrite protection.** The output path is refused if it already
//!   exists, unless `--force` is passed.
//! * **Secret scan.** Once the stream terminates, every decoded logical
//!   frame is walked for keys whose lowercase form contains any of a
//!   fixed list of obviously sensitive names (`apikey`, `api_key`,
//!   `authorization`, `bearer`, `password`, `secret`, `token`, and a few
//!   variants). On any hit the temp file is deleted and the recorder
//!   exits non-zero. This is intentionally conservative — a real user
//!   MUST review the fixture before committing.
//! * **Stdout is for the operator.** Progress lines go to the recorder's
//!   own stdout as short human strings; the child's protocol bytes never
//!   touch it. Anything unrecoverable is reported to stderr.
//! * **Bounded runtime.** Each scenario has an outer timeout; on
//!   timeout, the child is killed and no fixture is written.

#![forbid(unsafe_code)]
#![allow(clippy::print_stderr, clippy::print_stdout)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use smol::channel::{self, Receiver, Sender};
use smol::io::{AsyncReadExt as _, AsyncWriteExt as _};
use smol::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use omp_rpc_client::decoder::{ChunkReassembler, LineDecoder};
use omp_rpc_client::discovery::{
    DiscoveredOmp, DiscoveryInputs, MIN_SUPPORTED, SystemRunner, VersionSupport, discover,
};
use omp_rpc_client::frames::{
    AssistantMessageEventKind, ExtensionUiMethod, IncomingFrame, IncomingFrameKind, decode_frame,
};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// Scenario tag driven at the CLI. Ordering matches the M2 fixture list in
/// PLAN §6 M2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Plain,
    MultiToolLarge,
    Aborted,
    AskDialog,
}

impl Scenario {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "plain" => Ok(Self::Plain),
            "multi-tool-large" => Ok(Self::MultiToolLarge),
            "aborted" => Ok(Self::Aborted),
            "ask-dialog" => Ok(Self::AskDialog),
            other => Err(format!(
                "unknown scenario {other:?}; expected one of \
                 plain | multi-tool-large | aborted | ask-dialog"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::MultiToolLarge => "multi-tool-large",
            Self::Aborted => "aborted",
            Self::AskDialog => "ask-dialog",
        }
    }

    /// Outer time budget for the whole scenario. Generous enough to survive
    /// a slow model / large `bash` run; tight enough to fail loudly.
    fn timeout(self) -> Duration {
        match self {
            Self::MultiToolLarge => Duration::from_mins(4),
            Self::Plain | Self::Aborted | Self::AskDialog => Duration::from_secs(90),
        }
    }
}

/// Threshold below which a `multi-tool-large` recording is refused. The
/// installed OMP transport ceiling is `dW = 1 MiB` per physical frame; a
/// fixture that fits in fewer bytes cannot possibly contain a real
/// `rpc_chunk` sequence, so this is exactly the same constant OMP itself
/// uses to decide whether to chunk (see `dist/cli.js` `dW=1048576`).
const MIN_LARGE_FIXTURE_BYTES: u64 = 1_048_576;

/// Size of the single text block emitted through the recorder's private
/// `--trusted-extension` for the `multi-tool-large` scenario. Chosen well
/// above `dW` so that even after JSON escaping and envelope overhead the
/// server-side encoder is forced through `X7n.encodeFrames` -> `ce1(...)`,
/// which emits the `rpc_chunk` sequence we are here to record.
const LARGE_BLOB_BYTES: usize = 1_400_000;

/// Slash command name the recorder's private extension registers.
/// Must be a plain, unique identifier (matched by installed OMP against
/// `line.startsWith("/") && extensionRunner.getCommand(name)`).
const EXTENSION_COMMAND: &str = "record-fixture-large";

/// Deterministic, non-existent Cargo manifest path used by the
/// `multi-tool-large` phase-B prompt. The recorder REQUIRES this path
/// to not exist at run start (see `ensure_missing_manifest_absent`); the
/// recorder NEVER creates or removes it. `cargo build --manifest-path`
/// on a missing manifest produces a real, deterministic build error
/// (something like `error: could not find Cargo.toml`) without any side effect on the
/// working tree, satisfying the PLAN §6 M2 requirement that
/// `multi-tool-large` include a failing `cargo build`.
///
/// Path is relative to the recorder's cwd; the `__pimiento_fixture_...`
/// prefix makes accidental collision with real project content
/// vanishingly unlikely.
const M2_MISSING_MANIFEST: &str = "./__pimiento_fixture_missing__/Cargo.toml";

/// Bun/Node source of the private extension the recorder writes to a
/// tempfile and loads via `--trusted-extension`. Loading is deterministic:
/// `--trusted-extension` bypasses the interactive trust dialog, disables
/// ambient extension discovery, and asserts the file is an existing
/// absolute-path module (installed OMP `dist/cli.js` at offsets ~11537567
/// and ~11543650).
///
/// Handler:
///  1. `pi.registerCommand("record-fixture-large", ...)` — identical
///     shape to `examples/extensions/reload-runtime.ts` and `plan-mode.ts`
///     in the installed package.
///  2. On invocation, calls `ctx.ui.setWidget("record-fixture-large",
///     [long-line])`, which in RPC mode emits
///     `{type:"extension_ui_request", method:"setWidget", widgetKey,
///      widgetLines:[long-line]}` (installed cli.js ~offset 11510386).
///     The stringified frame exceeds `dW=1MiB`, so encodeFrames' `ce1(...)`
///     splits it into an `rpc_chunk` sequence — the recorder's whole
///     reason to exist.
///  3. Handler returns; server responds to the `prompt` with
///     `agentInvoked:false` because the message was a local slash command.
///
/// The extension is model-independent and free of anything that could look
/// like a secret (no arguments, no env access, no I/O).
fn extension_source() -> String {
    let command = EXTENSION_COMMAND;
    let bytes = LARGE_BLOB_BYTES;
    format!(
        r#"export default function (pi) {{
  pi.registerCommand({command:?}, {{
    description: "record_fixture: force one large extension_ui_request",
    handler: async (_args, ctx) => {{
      const blob = "x".repeat({bytes});
      ctx.ui.setWidget({command:?}, [blob]);
    }},
  }});
}}
"#,
    )
}

/// RAII handle to the private extension tempfile the recorder writes for
/// the `multi-tool-large` scenario. Dropping the handle unlinks the file
/// (best-effort; unlink failure is logged to stderr but does not
/// fail the recording). The file is placed under the OS temp directory,
/// never under the repo, and has a unique per-run name.
struct ExtensionTempfile {
    path: PathBuf,
}

impl ExtensionTempfile {
    fn new(source: &str) -> Result<Self, String> {
        // Deterministic-enough filename: pid + monotonic-ish nanos. The
        // `.ts` suffix is what Bun/the installed OMP loader expects for
        // TypeScript-flavoured extensions, but installed dist/cli.js also
        // accepts `.js`; we stick to `.mjs` so no TS toolchain is required
        // (the extension source is plain ES modules).
        let pid = std::process::id();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let mut path = std::env::temp_dir();
        path.push(format!("record_fixture_ext_{pid}_{ts}.mjs"));
        std::fs::write(&path, source)
            .map_err(|e| format!("write extension tempfile {}: {e}", path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ExtensionTempfile {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.path) {
            // Non-fatal: leaving a small no-secret .mjs behind in /tmp is
            // preferable to panicking during teardown.
            eprintln!(
                "record_fixture: failed to remove extension tempfile {}: {e}",
                self.path.display(),
            );
        }
    }
}

#[derive(Debug, Clone)]
struct Args {
    scenario: Scenario,
    out: PathBuf,
    force: bool,
}

impl Args {
    fn parse<I: IntoIterator<Item = String>>(iter: I) -> Result<Self, String> {
        let mut scenario: Option<Scenario> = None;
        let mut out: Option<PathBuf> = None;
        let mut force = false;
        let mut it = iter.into_iter().peekable();
        // Skip argv[0].
        it.next();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--scenario" | "-s" => {
                    let v = it
                        .next()
                        .ok_or_else(|| "--scenario needs a value".to_string())?;
                    scenario = Some(Scenario::parse(&v)?);
                }
                "--out" | "-o" => {
                    let v = it.next().ok_or_else(|| "--out needs a value".to_string())?;
                    out = Some(PathBuf::from(v));
                }
                "--force" | "-f" => force = true,
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unexpected argument {other:?}\n{}", usage())),
            }
        }
        let scenario = scenario.ok_or_else(|| format!("missing --scenario\n{}", usage()))?;
        let out = out.ok_or_else(|| format!("missing --out\n{}", usage()))?;
        Ok(Self {
            scenario,
            out,
            force,
        })
    }
}

fn usage() -> String {
    "\
usage: record_fixture --scenario <plain|multi-tool-large|aborted|ask-dialog> \
                      --out <path> [--force]

Records one live OMP session to a raw NDJSON fixture file for M2 replay
tests. The user's real `omp` binary is discovered via the login shell and
consumes a real model request; run against a benign session only.

Options:
  -s, --scenario NAME   which scripted scenario to drive
  -o, --out PATH        output NDJSON file (comment-free, server frames only)
  -f, --force           overwrite PATH if it already exists
  -h, --help            print this message
"
    .to_string()
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    let args = match Args::parse(std::env::args()) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => {
            println!(
                "record_fixture: wrote {} for scenario {}",
                args.out.display(),
                args.scenario.as_str()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("record_fixture: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    if args.out.exists() && !args.force {
        return Err(format!(
            "refusing to overwrite existing file {} (pass --force to replace)",
            args.out.display()
        ));
    }
    if let Some(parent) = args.out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create parent directory {}: {e}", parent.display()))?;
    }
    if args.scenario == Scenario::MultiToolLarge {
        ensure_missing_manifest_absent()?;
    }
    let discovered = discover_real_omp()?;
    match discovered.version.support() {
        VersionSupport::Supported | VersionSupport::Newer => {}
        VersionSupport::BelowMinimum => {
            return Err(format!(
                "discovered omp {} at {} is below minimum {MIN_SUPPORTED}; \
                 point PIMIENTO_OMP_BIN at a newer install",
                discovered.version,
                discovered.path.display(),
            ));
        }
    }
    println!(
        "record_fixture: using omp {} at {}",
        discovered.version,
        discovered.path.display()
    );

    let tmp = tmp_path_for(&args.out);
    // If a prior aborted run left one behind, don't be confused by it.
    let _ = fs::remove_file(&tmp);

    let scenario = args.scenario;
    let outcome: Result<(Vec<IncomingFrame>, u64), String> = smol::block_on(async {
        smol::future::race(drive_scenario(scenario, &discovered, &tmp), async move {
            smol::Timer::after(scenario.timeout()).await;
            Err(format!(
                "scenario {} exceeded timeout {:?}",
                scenario.as_str(),
                scenario.timeout(),
            ))
        })
        .await
    });

    match outcome {
        Ok((frames, raw_chunk_count)) => {
            if let Some(hit) = scan_frames_for_secrets(&frames) {
                let _ = fs::remove_file(&tmp);
                return Err(format!(
                    "refusing to write fixture: decoded frame contained \
                     suspicious key {hit:?} — review the transcript, redact, \
                     and retry manually",
                ));
            }
            if scenario == Scenario::MultiToolLarge {
                let size = fs::metadata(&tmp).map_or(0, |m| m.len());
                if let Err(msg) = enforce_large_invariant(raw_chunk_count, size) {
                    // Preserve the partial for diagnosis instead of
                    // deleting it: on MultiToolLarge invariant failure the
                    // most likely cause is that the model never invoked
                    // the host tool, and the raw frame stream is the only
                    // evidence of what actually happened.
                    let rejected = rejected_path_for(&args.out);
                    let _ = fs::remove_file(&rejected);
                    let preserved_at = match fs::rename(&tmp, &rejected) {
                        Ok(()) => format!(" (partial preserved at {})", rejected.display()),
                        Err(e) => format!(
                            " (partial preserved at {}; rename failed: {e})",
                            tmp.display(),
                        ),
                    };
                    return Err(format!("{msg}{preserved_at}"));
                }
            }
            fs::rename(&tmp, &args.out)
                .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), args.out.display()))?;
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// M2 large-fixture proof-of-recording gate.
///
/// Only `Scenario::MultiToolLarge` is expected to force server-side chunking;
/// with `dW = 1 MiB` per physical frame (installed OMP `dist/cli.js`), a
/// fixture that either fails to observe an `rpc_chunk` sequence, or whose raw
/// byte stream stays at or below the transport ceiling, cannot possibly
/// exercise `ChunkReassembler` on replay. Refuse loudly rather than commit a
/// fixture that silently understates M2 coverage.
fn enforce_large_invariant(raw_chunk_count: u64, file_bytes: u64) -> Result<(), String> {
    if raw_chunk_count == 0 {
        return Err(format!(
            "refusing to write multi-tool-large fixture: no rpc_chunk frames \
             observed on the wire (server never crossed the \
             {MIN_LARGE_FIXTURE_BYTES} B transport ceiling). Replay coverage \
             would be identical to a non-chunked fixture; re-run and \
             confirm the host tool actually executed."
        ));
    }
    if file_bytes <= MIN_LARGE_FIXTURE_BYTES {
        return Err(format!(
            "refusing to write multi-tool-large fixture: raw file is \
             {file_bytes} B (must exceed {MIN_LARGE_FIXTURE_BYTES} B to \
             guarantee a real rpc_chunk sequence)."
        ));
    }
    Ok(())
}

/// Preflight for the `multi-tool-large` scenario's phase B: the
/// deterministic manifest path must NOT exist. The recorder never
/// creates or removes anything on this path — a real user putting
/// content there is a signal that the recorder would run destructively
/// against their file, so we refuse. This is scoped to the recorder's
/// cwd; the `__pimiento_fixture_missing__` prefix makes a real user
/// collision vanishingly unlikely, but the check is still cheap and
/// unambiguous when it triggers.
fn ensure_missing_manifest_absent() -> Result<(), String> {
    let p = Path::new(M2_MISSING_MANIFEST);
    if p.exists() {
        return Err(format!(
            "refusing to run multi-tool-large scenario: expected {} to \
             be absent so `cargo build --manifest-path` produces a \
             deterministic 'could not find Cargo.toml' failure. Remove \
             (or rename) that path yourself and retry.",
            p.display()
        ));
    }
    // Also verify the parent directory is absent — cargo error text
    // differs subtly (missing directory vs missing file). The recorder's
    // fixture snapshot pins the argv, not the stderr text, but we keep
    // the surface as clean as possible.
    if let Some(parent) = p.parent()
        && parent.exists()
    {
        return Err(format!(
            "refusing to run multi-tool-large scenario: {} exists. Remove \
             (or rename) it yourself and retry — the recorder never \
             deletes paths outside its own tempfile.",
            parent.display()
        ));
    }
    Ok(())
}

fn tmp_path_for(out: &Path) -> PathBuf {
    let mut name = out
        .file_name()
        .map_or_else(|| OsString::from("fixture"), std::ffi::OsString::from);
    name.push(".partial");
    out.with_file_name(name)
}

/// Diagnostic path used when the multi-tool-large invariant refuses a
/// recording: the raw partial NDJSON is renamed to `<out>.rejected` so an
/// operator can inspect exactly what the server sent (specifically whether
/// a `host_tool_call` frame ever arrived, and whether the model instead
/// called a different tool).
fn rejected_path_for(out: &Path) -> PathBuf {
    let mut name = out
        .file_name()
        .map_or_else(|| OsString::from("fixture"), std::ffi::OsString::from);
    name.push(".rejected");
    out.with_file_name(name)
}

fn discover_real_omp() -> Result<DiscoveredOmp, String> {
    let current_env: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    let override_bin = current_env
        .get(OsString::from("PIMIENTO_OMP_BIN").as_os_str())
        .map(PathBuf::from);
    let login_shell = current_env
        .get(OsString::from("SHELL").as_os_str())
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from("/bin/sh")));
    let inputs = DiscoveryInputs {
        override_bin,
        setting_bin: None,
        login_shell,
        current_env,
    };
    discover(&inputs, &SystemRunner).map_err(|e| format!("omp discovery failed: {e}"))
}

// ---------------------------------------------------------------------------
// Live driver
// ---------------------------------------------------------------------------

/// Returns every logical frame we saw (for post-hoc secret scanning) and the
/// number of raw `rpc_chunk` physical frames observed on the wire.
async fn drive_scenario(
    scenario: Scenario,
    discovered: &DiscoveredOmp,
    tmp_path: &Path,
) -> Result<(Vec<IncomingFrame>, u64), String> {
    let mut cmd = Command::new(&discovered.path);
    cmd.env_clear();
    for (k, v) in &discovered.env {
        cmd.env(k, v);
    }
    cmd.arg("--mode").arg("rpc-ui").arg("--no-session");

    // MultiToolLarge deterministically forces server-side v2 chunking by
    // loading a private extension whose `/record-fixture-large` command
    // calls `ctx.ui.setWidget(...)` with a 1.4 MiB widgetLine. The
    // extension is written to a tempfile OUTSIDE the workspace and loaded
    // via `--trusted-extension`, which:
    //   * requires an absolute-path module file (installed dist/cli.js
    //     ~11537567: `trustedExtension must be an existing module file`),
    //   * bypasses interactive trust confirmation,
    //   * disables ambient extension discovery, so the user's local
    //     ~/.omp extensions do not run.
    let _ext_guard = if scenario == Scenario::MultiToolLarge {
        let guard = ExtensionTempfile::new(&extension_source())?;
        cmd.arg("--trusted-extension").arg(guard.path());
        Some(guard)
    } else {
        None
    };

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("spawn omp: {e}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "child stdin missing".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "child stdout missing".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "child stderr missing".to_string())?;

    let file = std::fs::File::create(tmp_path)
        .map_err(|e| format!("create {}: {e}", tmp_path.display()))?;
    let sink = Arc::new(Mutex::new(FixtureSink::new(file)));

    let (frames_tx, frames_rx) = channel::unbounded::<IncomingFrame>();
    let (writer_tx, writer_rx) = channel::unbounded::<Option<Vec<u8>>>();

    // Discard stderr — it must never be teed into the fixture or read for
    // control-flow, but we still need to drain it so the child doesn't
    // block on a full pipe.
    let stderr_task = smol::spawn(drain_stderr(stderr));
    let writer_task = smol::spawn(write_stdin(stdin, writer_rx));
    let reader_task = smol::spawn(pump_stdout(stdout, Arc::clone(&sink), frames_tx.clone()));
    drop(frames_tx);

    let result = run_driver(scenario, frames_rx, writer_tx.clone()).await;

    // Always try to close stdin then reap the child.
    let _ = writer_tx.send(None).await;
    drop(writer_tx);
    writer_task.await;
    reader_task.await;
    stderr_task.await;
    reap_child(&mut child, Duration::from_secs(5)).await;

    // Flush and finalise the fixture file — even on error, so a partial
    // stream can be inspected during driver debugging.
    let (frames, raw_chunk_count, write_err) = {
        let mut s = sink.lock();
        let flush = s.file.flush();
        (
            std::mem::take(&mut s.frames),
            s.raw_chunk_count,
            flush.err().map(|e| e.to_string()),
        )
    };
    result?;
    if let Some(e) = write_err {
        return Err(format!("flush fixture: {e}"));
    }
    Ok((frames, raw_chunk_count))
}

struct FixtureSink {
    file: std::fs::File,
    lines: LineDecoder,
    chunks: ChunkReassembler,
    frames: Vec<IncomingFrame>,
    /// Count of *physical* `type:"rpc_chunk"` frames observed on the wire,
    /// pre-reassembly. Mirrors `X7n.encodeFrames` chunk emission
    /// (`dist/cli.js` `ce1(...)`), independent of whether reassembly
    /// succeeds. This is the recorder's proof that v2 chunking actually
    /// engaged.
    raw_chunk_count: u64,
}

impl FixtureSink {
    fn new(file: std::fs::File) -> Self {
        Self {
            file,
            lines: LineDecoder::new(),
            chunks: ChunkReassembler::new(),
            frames: Vec::new(),
            raw_chunk_count: 0,
        }
    }

    /// Tee raw bytes verbatim to disk, and feed the same bytes through the
    /// M1 decode stack. Returns any decoded logical frames.
    fn ingest(&mut self, bytes: &[u8]) -> Result<Vec<IncomingFrame>, String> {
        self.file
            .write_all(bytes)
            .map_err(|e| format!("write fixture: {e}"))?;
        let mut out = Vec::new();
        let chunks = &mut self.chunks;
        let raw_chunk_count = &mut self.raw_chunk_count;
        let sink_frames = &mut out;
        let mut inner_err: Option<String> = None;
        let feed = self.lines.feed(bytes, |v| {
            // Count physical `rpc_chunk` frames BEFORE reassembly so the
            // metric reflects what actually crossed the wire, matching
            // the server's `X7n.encodeFrames` chunk emission path.
            if v.get("type").and_then(Value::as_str) == Some("rpc_chunk") {
                *raw_chunk_count += 1;
            }
            match chunks.push(v) {
                Ok(Some(logical)) => match decode_frame(logical) {
                    Ok(frame) => {
                        sink_frames.push(frame);
                        Ok(())
                    }
                    Err(e) => {
                        inner_err = Some(format!("decode_frame: {e}"));
                        Err(omp_rpc_client::RpcError::ProtocolViolation {
                            detail: "typed decode failed".into(),
                        })
                    }
                },
                Ok(None) => Ok(()),
                Err(e) => Err(e),
            }
        });
        if let Some(msg) = inner_err {
            return Err(msg);
        }
        feed.map_err(|e| format!("decoder: {e}"))?;
        self.frames.extend(out.iter().cloned());
        Ok(out)
    }
}

async fn reap_child(child: &mut Child, budget: Duration) {
    let deadline = std::time::Instant::now() + budget;
    loop {
        match child.try_status() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => {}
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.status().await;
            return;
        }
        smol::Timer::after(Duration::from_millis(50)).await;
    }
}

async fn drain_stderr(mut stderr: smol::process::ChildStderr) {
    let mut buf = [0u8; 4096];
    while let Ok(n) = stderr.read(&mut buf).await
        && n > 0
    {}
}

async fn write_stdin(mut stdin: ChildStdin, rx: Receiver<Option<Vec<u8>>>) {
    while let Ok(msg) = rx.recv().await {
        let Some(bytes) = msg else {
            break;
        };
        if stdin.write_all(&bytes).await.is_err() {
            break;
        }
        if stdin.flush().await.is_err() {
            break;
        }
    }
    // Drop closes stdin and signals half-close to the child.
}

async fn pump_stdout(
    mut stdout: ChildStdout,
    sink: Arc<Mutex<FixtureSink>>,
    frames_tx: Sender<IncomingFrame>,
) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = match stdout.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let decoded = {
            let mut s = sink.lock();
            s.ingest(&buf[..n])
        };
        match decoded {
            Ok(frames) => {
                for f in frames {
                    if frames_tx.send(f).await.is_err() {
                        return;
                    }
                }
            }
            Err(_) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario state machines
// ---------------------------------------------------------------------------

async fn run_driver(
    scenario: Scenario,
    frames: Receiver<IncomingFrame>,
    writer: Sender<Option<Vec<u8>>>,
) -> Result<(), String> {
    // Every scenario: wait for `ready`, negotiate v2, then send its prompt
    // and drive to a terminal `agent_end` (or `prompt_result` for local-only
    // prompts). The correlation surface is trivial because we only ever
    // have one command in flight at a time.
    wait_for_ready(&frames).await?;
    send_json(
        &writer,
        &json!({
            "id": "protocol-1",
            "type": "negotiate_protocol",
            "protocolVersion": 2,
        }),
    )
    .await?;
    await_response_id(&frames, "protocol-1").await?;

    // MultiToolLarge splits into two phases so v2 chunking is deterministic
    // and model-independent:
    //   Phase A: drive `/record-fixture-large`, a local slash command
    //   registered by the recorder's private extension. Installed OMP
    //   dispatches it via `vF0(...)` (dist/cli.js ~offset 11512006); the
    //   handler calls `ctx.ui.setWidget("record-fixture-large", [blob])`,
    //   which emits an `extension_ui_request` whose stringified JSON
    //   exceeds `dW=1 MiB` and is therefore chunked by X7n.encodeFrames
    //   (`ce1(...)`). The command returns; the server sends
    //   `prompt_result { agentInvoked:false }`.
    //   Phase B: a normal prompt asking the model to run a single failing
    //   `bash` command, so the fixture also covers a `tool_execution_end`
    //   with `isError:true`.
    if scenario == Scenario::MultiToolLarge {
        println!(
            "record_fixture: driving scenario {} — phase A: /{EXTENSION_COMMAND} \
             (extension emits large widget → rpc_chunk)",
            scenario.as_str(),
        );
        send_json(
            &writer,
            &json!({
                "id": "prompt-large",
                "type": "prompt",
                "message": format!("/{EXTENSION_COMMAND}"),
            }),
        )
        .await?;
        await_response_id(&frames, "prompt-large").await?;
        // The command is local; the server replies with prompt_result
        // agentInvoked:false. Drain frames until that terminal signal.
        await_prompt_result(&frames).await?;

        println!(
            "record_fixture: driving scenario {} — phase B: failing bash",
            scenario.as_str(),
        );
    }

    let (prompt_msg, phase) = scenario_script(scenario);
    if scenario != Scenario::MultiToolLarge {
        println!(
            "record_fixture: driving scenario {} — {}",
            scenario.as_str(),
            phase,
        );
    }
    send_json(
        &writer,
        &json!({
            "id": "prompt-1",
            "type": "prompt",
            "message": prompt_msg,
        }),
    )
    .await?;
    await_response_id(&frames, "prompt-1").await?;

    match scenario {
        Scenario::Plain | Scenario::MultiToolLarge => drive_until_agent_end(&frames).await,
        Scenario::Aborted => drive_abort(&frames, &writer).await,
        Scenario::AskDialog => drive_ask_dialog(&frames, &writer).await,
    }
}

fn scenario_script(s: Scenario) -> (String, &'static str) {
    match s {
        Scenario::Plain => (
            "reply with exactly: pong".to_string(),
            "plain: expect a short assistant reply then agent_end",
        ),
        Scenario::MultiToolLarge => (
            // Phase B only — phase A is driven by run_driver via the
            // /record-fixture-large slash command handler. This second
            // prompt exercises a failing `cargo build` (PLAN §6 M2
            // requires a multi-tool run that includes cargo build with
            // errors). The manifest path is deterministic, never
            // touched by the recorder, and its absence is enforced by
            // `ensure_missing_manifest_absent` at run start — so cargo
            // emits a real, model-independent "could not find
            // Cargo.toml" error and the fixture captures a
            // `tool_execution_end` with `isError:true`.
            format!(
                "call the bash tool exactly once with the command \
                 `cargo build --manifest-path {M2_MISSING_MANIFEST}`. \
                 That path does not exist; cargo MUST report an error \
                 and the tool call MUST fail — that is expected. Then \
                 reply with the single word: done. Do not invoke any \
                 other tools. Do not call report_issue. Do not repeat \
                 the command. Finish immediately after the one bash \
                 call."
            ),
            "multi-tool-large: phase B — one failing `cargo build` on a missing manifest, then done",
        ),
        Scenario::Aborted => (
            "count slowly from 1 to 1000, one number per line, no other text".to_string(),
            "aborted: send abort as soon as the first assistant text arrives",
        ),
        Scenario::AskDialog => (
            "use the ask tool with method=confirm, title='proceed?', \
             message='ok to proceed?' — after I confirm, reply with the \
             single word: acknowledged"
                .to_string(),
            "ask-dialog: auto-respond to the extension_ui_request",
        ),
    }
}

async fn wait_for_ready(frames: &Receiver<IncomingFrame>) -> Result<(), String> {
    loop {
        let f = frames
            .recv()
            .await
            .map_err(|_| "child closed before ready".to_string())?;
        if matches!(f.kind, IncomingFrameKind::Ready(_)) {
            return Ok(());
        }
    }
}

async fn await_response_id(frames: &Receiver<IncomingFrame>, id: &str) -> Result<(), String> {
    loop {
        let f = frames
            .recv()
            .await
            .map_err(|_| "child closed before response".to_string())?;
        if let IncomingFrameKind::Response(r) = &f.kind
            && r.id.as_deref() == Some(id)
        {
            if !r.success {
                return Err(format!(
                    "response {id} returned success=false: {:?}",
                    r.error
                ));
            }
            return Ok(());
        }
    }
}

async fn drive_until_agent_end(frames: &Receiver<IncomingFrame>) -> Result<(), String> {
    loop {
        let f = frames
            .recv()
            .await
            .map_err(|_| "child closed before agent_end".to_string())?;
        match &f.kind {
            IncomingFrameKind::AgentEnd(_) => return Ok(()),
            IncomingFrameKind::PromptResult(_) => {
                // agentInvoked:false path — the run ended without a real
                // agent turn. Terminal for our purposes.
                return Ok(());
            }
            _ => {}
        }
    }
}

/// Drain frames until the server emits a `prompt_result`. Used by the
/// `MultiToolLarge` scenario after issuing a local slash command: those
/// commands are handled entirely on the server side (installed OMP
/// `vF0(...)` dispatch, `dist/cli.js` ~11512006) and never produce an
/// `agent_start`/`agent_end` pair.
async fn await_prompt_result(frames: &Receiver<IncomingFrame>) -> Result<(), String> {
    loop {
        let f = frames.recv().await.map_err(|_| {
            "child closed before prompt_result (extension command may have panicked)".to_string()
        })?;
        if let IncomingFrameKind::PromptResult(_) = &f.kind {
            return Ok(());
        }
        if let IncomingFrameKind::AgentEnd(_) = &f.kind {
            // Unexpected — a slash command shouldn't invoke the agent —
            // but treat it as a terminal signal so we don't hang.
            return Ok(());
        }
    }
}

async fn drive_abort(
    frames: &Receiver<IncomingFrame>,
    writer: &Sender<Option<Vec<u8>>>,
) -> Result<(), String> {
    let mut aborted = false;
    loop {
        let f = frames
            .recv()
            .await
            .map_err(|_| "child closed before agent_end".to_string())?;
        if !aborted {
            let trigger = matches!(&f.kind, IncomingFrameKind::AgentStart)
                || matches!(&f.kind, IncomingFrameKind::MessageStart)
                || matches!(&f.kind, IncomingFrameKind::MessageUpdate(mu)
                    if matches!(mu.assistant_message_event.kind,
                        AssistantMessageEventKind::TextDelta { .. }
                        | AssistantMessageEventKind::TextStart { .. }
                        | AssistantMessageEventKind::Start));
            if trigger {
                send_json(
                    writer,
                    &json!({
                        "id": "abort-1",
                        "type": "abort",
                    }),
                )
                .await?;
                aborted = true;
            }
        }
        if matches!(&f.kind, IncomingFrameKind::AgentEnd(_))
            || matches!(&f.kind, IncomingFrameKind::PromptResult(_))
        {
            return Ok(());
        }
    }
}

async fn drive_ask_dialog(
    frames: &Receiver<IncomingFrame>,
    writer: &Sender<Option<Vec<u8>>>,
) -> Result<(), String> {
    loop {
        let f = frames
            .recv()
            .await
            .map_err(|_| "child closed before agent_end".to_string())?;
        match &f.kind {
            IncomingFrameKind::ExtensionUiRequest(req) => {
                let reply = match &req.method {
                    ExtensionUiMethod::Confirm { .. } => json!({
                        "type": "extension_ui_response",
                        "id": req.id,
                        "confirmed": true,
                    }),
                    ExtensionUiMethod::Select { options, .. } => {
                        let value = options.first().cloned().unwrap_or_default();
                        json!({
                            "type": "extension_ui_response",
                            "id": req.id,
                            "value": value,
                        })
                    }
                    ExtensionUiMethod::Input { .. } => json!({
                        "type": "extension_ui_response",
                        "id": req.id,
                        "value": "yes",
                    }),
                    _ => continue,
                };
                send_json(writer, &reply).await?;
            }
            IncomingFrameKind::AgentEnd(_) | IncomingFrameKind::PromptResult(_) => {
                return Ok(());
            }
            _ => {}
        }
    }
}

async fn send_json(writer: &Sender<Option<Vec<u8>>>, v: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(v).map_err(|e| format!("serialize: {e}"))?;
    bytes.push(b'\n');
    writer
        .send(Some(bytes))
        .await
        .map_err(|_| "writer channel closed".to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Secret scan (post-run, on decoded logical frames)
// ---------------------------------------------------------------------------

/// Substrings, matched against a *normalized* key name (lowercased with
/// `-` and `_` removed), that flag a key as obviously credential-bearing.
/// The normalization means one entry covers many wire spellings, e.g.
/// `apikey` also matches `apiKey`, `api_key`, `api-key`, `X-API-Key` (which
/// normalizes to `xapikey`).
///
/// Notably **not** in this list: a bare `token`. OMP's session-state and
/// telemetry frames legitimately expose `totalTokens`, `inputTokens`,
/// `outputTokens`, `contextUsage.tokens`, etc.; matching on `token` alone
/// would refuse every normal recording. We only match *credential-shaped*
/// token names (`accessToken`, `refreshToken`, `idToken`, …).
const SECRET_KEY_MARKERS: &[&str] = &[
    "apikey",
    "authorization",
    "bearer",
    "password",
    "passphrase",
    "privatekey",
    "secret",
    "accesstoken",
    "refreshtoken",
    "idtoken",
    "oauthtoken",
    "sessiontoken",
    "authtoken",
];

fn normalize_key(k: &str) -> String {
    let mut out = String::with_capacity(k.len());
    for ch in k.chars() {
        if ch == '-' || ch == '_' {
            continue;
        }
        for c in ch.to_lowercase() {
            out.push(c);
        }
    }
    out
}

/// Walk `frames` and return the first offending key name, if any.
fn scan_frames_for_secrets(frames: &[IncomingFrame]) -> Option<String> {
    for f in frames {
        if let Some(hit) = scan_value_for_secret_key(&f.raw) {
            return Some(hit);
        }
    }
    None
}

fn scan_value_for_secret_key(v: &Value) -> Option<String> {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                let norm = normalize_key(k);
                if SECRET_KEY_MARKERS.iter().any(|m| norm.contains(m)) {
                    return Some(k.clone());
                }
                if let Some(hit) = scan_value_for_secret_key(child) {
                    return Some(hit);
                }
            }
            None
        }
        Value::Array(a) => a.iter().find_map(scan_value_for_secret_key),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Unit tests — args parsing, overwrite protection, secret scan.
// No live model or child process.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use omp_rpc_client::frames::decode_frame;

    fn argv<const N: usize>(items: [&str; N]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn parses_valid_arguments() {
        let a = Args::parse(argv([
            "record_fixture",
            "--scenario",
            "plain",
            "--out",
            "/tmp/pl.ndjson",
        ]))
        .expect("parse ok");
        assert_eq!(a.scenario, Scenario::Plain);
        assert_eq!(a.out, PathBuf::from("/tmp/pl.ndjson"));
        assert!(!a.force);
    }

    #[test]
    fn parses_all_scenarios_and_force() {
        for (raw, want) in [
            ("plain", Scenario::Plain),
            ("multi-tool-large", Scenario::MultiToolLarge),
            ("aborted", Scenario::Aborted),
            ("ask-dialog", Scenario::AskDialog),
        ] {
            let a = Args::parse(argv([
                "record_fixture",
                "--scenario",
                raw,
                "--out",
                "/tmp/x.ndjson",
                "--force",
            ]))
            .unwrap_or_else(|e| panic!("parse {raw}: {e}"));
            assert_eq!(a.scenario, want);
            assert!(a.force);
        }
    }

    #[test]
    fn rejects_unknown_scenario() {
        let err = Args::parse(argv([
            "record_fixture",
            "--scenario",
            "bogus",
            "--out",
            "/tmp/x.ndjson",
        ]))
        .expect_err("must reject unknown scenario");
        assert!(err.contains("bogus"), "err was: {err}");
    }

    #[test]
    fn rejects_missing_required_flags() {
        assert!(Args::parse(argv(["record_fixture", "--scenario", "plain"])).is_err());
        assert!(Args::parse(argv(["record_fixture", "--out", "/tmp/x.ndjson"])).is_err());
    }

    #[test]
    fn overwrite_refused_without_force() {
        let dir = tempfile::tempdir().expect("mktemp");
        let path = dir.path().join("existing.ndjson");
        std::fs::write(&path, b"prior content").expect("seed");
        let args = Args {
            scenario: Scenario::Plain,
            out: path.clone(),
            force: false,
        };
        let err = run(&args).expect_err("must refuse to overwrite");
        assert!(err.contains("refusing to overwrite"), "err was: {err}");
        // Original file untouched.
        let after = std::fs::read(&path).expect("read");
        assert_eq!(after, b"prior content");
    }

    #[test]
    fn secret_scan_flags_obvious_keys() {
        let frame_with_secret = decode_frame(json!({
            "type": "notice",
            "level": "info",
            "message": "hello",
            "meta": {
                "provider": {
                    "apiKey": "sk-live-xxxx"
                }
            }
        }))
        .expect("decode");
        let frames = vec![frame_with_secret];
        let hit = scan_frames_for_secrets(&frames).expect("must flag");
        assert_eq!(hit, "apiKey");
    }

    #[test]
    fn secret_scan_flags_top_level_authorization_key() {
        let frame = decode_frame(json!({
            "type": "notice",
            "level": "warning",
            "message": "leaked",
            "Authorization": "Bearer zzz",
        }))
        .expect("decode");
        assert_eq!(
            scan_frames_for_secrets(&[frame]).as_deref(),
            Some("Authorization"),
        );
    }

    #[test]
    fn secret_scan_flags_nested_array_secret() {
        let frame = decode_frame(json!({
            "type": "notice",
            "level": "info",
            "message": "arrays too",
            "items": [
                { "harmless": 1 },
                { "password": "hunter2" }
            ]
        }))
        .expect("decode");
        assert_eq!(
            scan_frames_for_secrets(&[frame]).as_deref(),
            Some("password"),
        );
    }

    #[test]
    fn secret_scan_passes_clean_frames() {
        let frame = decode_frame(json!({
            "type": "notice",
            "level": "info",
            "message": "no secrets here",
            "meta": { "count": 3, "labels": ["a", "b"] }
        }))
        .expect("decode");
        assert_eq!(scan_frames_for_secrets(&[frame]), None);
    }

    #[test]
    fn secret_scan_is_case_insensitive() {
        let frame = decode_frame(json!({
            "type": "notice",
            "level": "info",
            "message": "yep",
            "X-API-KEY": "leak"
        }))
        .expect("decode");
        assert!(scan_frames_for_secrets(&[frame]).is_some());
    }

    #[test]
    fn secret_scan_allows_usage_token_fields() {
        // Real OMP state / telemetry frames expose token counts and
        // context usage widely; none of these keys are credentials.
        let frame = decode_frame(json!({
            "type": "notice",
            "level": "info",
            "message": "usage snapshot",
            "usage": {
                "totalTokens": 1234,
                "inputTokens": 900,
                "outputTokens": 334,
                "cachedTokens": 100,
                "reasoningTokens": 42
            },
            "contextUsage": {
                "tokens": 1234,
                "percent": 12.5,
                "maxTokens": 200_000
            }
        }))
        .expect("decode");
        assert_eq!(scan_frames_for_secrets(&[frame]), None);
    }

    #[test]
    fn secret_scan_rejects_credential_token_shapes() {
        for key in [
            "accessToken",
            "access_token",
            "refreshToken",
            "refresh_token",
            "idToken",
            "id_token",
            "oauthToken",
            "sessionToken",
            "authToken",
            "clientSecret",
            "client_secret",
            "privateKey",
        ] {
            let frame = decode_frame(json!({
                "type": "notice",
                "level": "info",
                "message": "leaked",
                key: "xxx"
            }))
            .expect("decode");
            assert_eq!(
                scan_frames_for_secrets(&[frame]).as_deref(),
                Some(key),
                "expected {key} to be flagged"
            );
        }
    }

    #[test]
    fn normalize_key_strips_separators_and_case() {
        assert_eq!(normalize_key("access_token"), "accesstoken");
        assert_eq!(normalize_key("Access-Token"), "accesstoken");
        assert_eq!(normalize_key("X-API-Key"), "xapikey");
        assert_eq!(normalize_key("totalTokens"), "totaltokens");
    }

    #[test]
    fn tmp_path_appends_partial_suffix() {
        let p = tmp_path_for(Path::new("/tmp/foo/bar.ndjson"));
        assert_eq!(p, PathBuf::from("/tmp/foo/bar.ndjson.partial"));
    }

    #[test]
    fn rejected_path_appends_rejected_suffix() {
        let p = rejected_path_for(Path::new("/tmp/foo/bar.ndjson"));
        assert_eq!(p, PathBuf::from("/tmp/foo/bar.ndjson.rejected"));
    }

    #[test]
    fn scenario_timeouts_are_positive() {
        for s in [
            Scenario::Plain,
            Scenario::MultiToolLarge,
            Scenario::Aborted,
            Scenario::AskDialog,
        ] {
            assert!(s.timeout() > Duration::ZERO);
        }
    }

    #[test]
    fn multi_tool_large_phase_b_prompt_asks_for_failing_cargo_build() {
        // Phase B (the model-driven portion) exercises a failing
        // `cargo build` on a deterministic non-existent manifest, so
        // the fixture covers `tool_execution_end` with isError:true
        // AND the PLAN §6 M2 "multi-tool run including cargo build with
        // errors" requirement. Phase A (the local slash command that
        // forces rpc_chunk) is driven by run_driver and is NOT in the
        // prompt.
        let (prompt, _label) = scenario_script(Scenario::MultiToolLarge);
        assert!(
            prompt.contains("cargo build --manifest-path"),
            "prompt must include `cargo build --manifest-path`: {prompt}"
        );
        assert!(
            prompt.contains(M2_MISSING_MANIFEST),
            "prompt must name the deterministic missing manifest path \
             {M2_MISSING_MANIFEST}: {prompt}"
        );
        assert!(
            prompt.contains("Do not invoke any other tools"),
            "prompt must forbid other tool calls: {prompt}"
        );
        assert!(
            prompt.contains("report_issue"),
            "prompt must forbid report_issue: {prompt}"
        );
        // Phase A machinery must NOT leak into the phase-B prompt.
        assert!(
            !prompt.contains(EXTENSION_COMMAND),
            "phase-B prompt must not mention the local slash command: {prompt}"
        );
        assert!(
            !prompt.contains("host tool"),
            "phase-B prompt must not reference host tools (unused): {prompt}"
        );
        // And no dangling stubs from earlier revisions.
        for stale in ["head -c 1400000 /dev/zero", "sh -c 'exit 7'"] {
            assert!(
                !prompt.contains(stale),
                "phase-B prompt must not include stale marker {stale:?}: {prompt}"
            );
        }
    }

    #[test]
    fn missing_manifest_path_is_scoped_and_relative() {
        // The deterministic manifest path MUST be scoped to the
        // recorder's cwd (relative, `./` prefix) so preflight can never
        // reach a system path, and MUST carry the marker prefix that
        // guarantees no realistic project collision.
        assert!(
            M2_MISSING_MANIFEST.starts_with("./"),
            "M2_MISSING_MANIFEST must be a relative `./`-prefixed path: {M2_MISSING_MANIFEST}"
        );
        assert!(
            M2_MISSING_MANIFEST.contains("__pimiento_fixture_missing__"),
            "M2_MISSING_MANIFEST must carry the unique marker prefix: {M2_MISSING_MANIFEST}"
        );
        assert!(
            M2_MISSING_MANIFEST.ends_with("Cargo.toml"),
            "M2_MISSING_MANIFEST must name a Cargo.toml: {M2_MISSING_MANIFEST}"
        );
    }

    /// Serializes tests that must mutate the process-wide cwd.
    static CWD_LOCK: std::sync::LazyLock<parking_lot::Mutex<()>> =
        std::sync::LazyLock::new(|| parking_lot::Mutex::new(()));

    #[test]
    fn ensure_missing_manifest_absent_passes_on_clean_cwd() {
        // Run inside a fresh tempdir so we know the marker path can't
        // exist. The recorder's real cwd is workspace root; unit tests
        // are not allowed to mutate it, so we `set_current_dir` to a
        // scratch dir only for the scope of this test.
        let _guard = CWD_LOCK.lock();
        let dir = tempfile::tempdir().expect("mktemp");
        let prev = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(dir.path()).expect("set_current_dir");
        let res = ensure_missing_manifest_absent();
        std::env::set_current_dir(&prev).expect("restore cwd");
        res.expect("clean cwd must pass preflight");
    }

    #[test]
    fn ensure_missing_manifest_absent_refuses_when_file_present() {
        let _guard = CWD_LOCK.lock();
        let dir = tempfile::tempdir().expect("mktemp");
        let prev = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(dir.path()).expect("set_current_dir");
        let target = Path::new(M2_MISSING_MANIFEST);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("mkdir marker parent");
        }
        std::fs::write(target, b"[package]\nname = \"x\"\n").expect("seed manifest");
        let err = ensure_missing_manifest_absent();
        std::env::set_current_dir(&prev).expect("restore cwd");
        let err = err.expect_err("must refuse when the marker path exists");
        assert!(err.contains("refusing"), "err was: {err}");
    }

    #[test]
    fn large_blob_bytes_exceeds_transport_ceiling() {
        // The widget blob size the extension emits must be strictly
        // larger than the installed OMP `dW=1 MiB` frame ceiling,
        // otherwise the server would ship the extension_ui_request as
        // one physical frame and no rpc_chunk sequence would ever be
        // produced.
        assert!(
            (LARGE_BLOB_BYTES as u64) > MIN_LARGE_FIXTURE_BYTES,
            "LARGE_BLOB_BYTES must exceed the 1 MiB transport ceiling"
        );
    }

    #[test]
    fn enforce_large_invariant_rejects_zero_chunks() {
        // Recording yielded no physical rpc_chunk frames on the wire -> not
        // an M2 fixture regardless of file size.
        let err = enforce_large_invariant(0, 10 * 1024 * 1024)
            .expect_err("must reject a chunk-less recording");
        assert!(err.contains("no rpc_chunk frames"), "err was: {err}");
    }

    #[test]
    fn enforce_large_invariant_rejects_undersized_file() {
        // Non-zero chunk count but total file size at-or-below the 1 MiB
        // transport ceiling is nonsensical (chunking implies >1 MiB).
        let err = enforce_large_invariant(3, MIN_LARGE_FIXTURE_BYTES)
            .expect_err("must reject a fixture at or below the transport ceiling");
        assert!(err.contains("raw file is"), "err was: {err}");
    }

    #[test]
    fn enforce_large_invariant_accepts_real_recording() {
        // A single valid recording: at least one physical rpc_chunk and a
        // file strictly larger than the transport ceiling.
        enforce_large_invariant(2, MIN_LARGE_FIXTURE_BYTES + 1)
            .expect("must accept a valid multi-tool-large recording");
    }

    #[test]
    fn extension_source_registers_command_with_setwidget_call() {
        // The generated extension MUST:
        //   1. call `pi.registerCommand(<EXTENSION_COMMAND>, ...)` so
        //      installed OMP routes `/<EXTENSION_COMMAND>` to it,
        //   2. call `ctx.ui.setWidget(<EXTENSION_COMMAND>, [blob])` so
        //      the server emits an `extension_ui_request` with
        //      widgetLines large enough to force v2 chunking,
        //   3. embed the exact LARGE_BLOB_BYTES payload size,
        //   4. be a plain ES module (export default function) so no TS
        //      toolchain is required.
        let src = extension_source();
        assert!(
            src.contains(&format!("pi.registerCommand(\"{EXTENSION_COMMAND}\"")),
            "extension must registerCommand({EXTENSION_COMMAND:?}): {src}"
        );
        assert!(
            src.contains("ctx.ui.setWidget"),
            "extension must call ctx.ui.setWidget: {src}"
        );
        assert!(
            src.contains(&LARGE_BLOB_BYTES.to_string()),
            "extension must reference LARGE_BLOB_BYTES: {src}"
        );
        assert!(
            src.starts_with("export default function"),
            "extension must be a plain ES module: {src}"
        );
        // No file I/O, no environment access, no arguments \u2014 the source
        // is model- and env-independent by construction.
        for forbidden in ["process.env", "readFile", "writeFile", "child_process"] {
            assert!(
                !src.contains(forbidden),
                "extension must not reference {forbidden}: {src}"
            );
        }
    }

    #[test]
    fn extension_tempfile_lifecycle() {
        // The RAII tempfile writes exactly the extension source, exposes
        // an absolute path (installed OMP's --trusted-extension validator
        // requires this), and unlinks on drop.
        let guard = ExtensionTempfile::new("export default function(pi){}\n")
            .expect("must write extension tempfile");
        let path = guard.path().to_path_buf();
        assert!(
            path.is_absolute(),
            "path must be absolute: {}",
            path.display()
        );
        assert!(path.exists(), "tempfile must exist while guard is alive");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read tempfile"),
            "export default function(pi){}\n"
        );
        drop(guard);
        assert!(
            !path.exists(),
            "tempfile must be unlinked when guard is dropped: {}",
            path.display()
        );
    }

    #[test]
    fn drive_scenario_spawn_uses_trusted_extension_for_multi_tool_large() {
        // Sanity: EXTENSION_COMMAND names a bare slash-command-friendly
        // identifier (no whitespace, no leading `/`, no slashes).
        assert!(
            !EXTENSION_COMMAND.is_empty(),
            "EXTENSION_COMMAND must be non-empty"
        );
        assert!(
            !EXTENSION_COMMAND.starts_with('/'),
            "EXTENSION_COMMAND must not include the leading `/`"
        );
        assert!(
            EXTENSION_COMMAND
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "EXTENSION_COMMAND must be a plain slash-command identifier"
        );
    }
}
