//! Deterministic replay snapshot tests.
//!
//! Each test drives one recorded raw NDJSON fixture through the production
//! M1 line/chunk decoder + M2 projection reducer via
//! [`pimiento_core::replay::replay_ndjson_file`] and asserts on the
//! dedicated [`pimiento_core::replay::ReplaySummary`] view via `insta`.
//!
//! Snapshots use YAML for a reviewable, line-oriented diff. Redactions are
//! limited to fields the recorder cannot deterministically pin (nested
//! session ids, absolute session paths, wall-clock timestamps carried on
//! `Unknown` rows). Every other field is asserted verbatim.
//!
//! Fixtures live in `crates/pimiento-core/tests/fixtures/*.ndjson` and are
//! produced by the M2 recorder (`omp-recorder`). Missing fixtures are hard
//! failures — the recorder ships ahead of these tests.

use std::path::{Path, PathBuf};

use pimiento_core::replay::{ReplayReport, ReplaySummary, replay_ndjson_file};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn fixture_path(name: &str) -> PathBuf {
    let path = fixture_dir().join(name);
    assert!(
        path.exists(),
        "replay fixture `{name}` missing at {} — regenerate with `omp-recorder`",
        path.display()
    );
    path
}

/// Redactions applied to every snapshot. The [`ReplaySummary`] shape
/// already drops the most volatile fields (tool call ids, dialog wire ids,
/// widget payloads, the full command catalog); these redactions cover the
/// long tail of nondeterministic bytes that may survive on raw `Unknown`
/// transcript rows or nested `state` blobs.
macro_rules! snapshot_summary {
    ($name:expr, $summary:expr) => {{
        insta::with_settings!({
            snapshot_path => "snapshots",
            prepend_module_to_snapshot => false,
        }, {
            insta::assert_yaml_snapshot!($name, &$summary, {
                ".**.session_id" => "[session-id]",
                ".**.sessionId" => "[session-id]",
                ".**.session_path" => "[session-path]",
                ".**.sessionPath" => "[session-path]",
                ".**.cwd" => "[cwd]",
                ".**.timestamp" => "[timestamp]",
                ".**.startedAt" => "[timestamp]",
                ".**.endedAt" => "[timestamp]",
                ".**.updatedAt" => "[timestamp]",
                ".**.responseId" => "[response-id]",
            });
        });
    }};
}

fn replay_summary(name: &str) -> ReplaySummary {
    let path = fixture_path(name);
    let report: ReplayReport =
        replay_ndjson_file(&path).unwrap_or_else(|e| panic!("replay `{name}` failed: {e}"));
    report.summary()
}

#[test]
fn replay_plain_answer() {
    let summary = replay_summary("plain-answer.ndjson");
    snapshot_summary!("plain-answer", summary);
}

#[test]
fn replay_multi_tool_large() {
    let path = fixture_path("multi-tool-large.ndjson");
    let raw_size = std::fs::metadata(&path)
        .expect("stat multi-tool-large fixture")
        .len();
    assert!(
        raw_size > 1024 * 1024,
        "multi-tool-large fixture must exceed 1 MiB of raw NDJSON bytes; got {raw_size}"
    );

    let report: ReplayReport = replay_ndjson_file(&path).expect("multi-tool-large replay");
    assert!(
        report.reassembled_sequences >= 1,
        "multi-tool-large must contain at least one rpc_chunk sequence; \
         got reassembled_sequences = {}",
        report.reassembled_sequences
    );
    assert!(
        report.chunk_frames >= 2,
        "a completed rpc_chunk sequence needs >=2 physical chunk frames; \
         got chunk_frames = {}",
        report.chunk_frames
    );

    snapshot_summary!("multi-tool-large", report.summary());
}

#[test]
fn replay_aborted_run() {
    let summary = replay_summary("aborted-run.ndjson");
    snapshot_summary!("aborted-run", summary);
}

#[test]
fn replay_ask_dialog() {
    let summary = replay_summary("ask-dialog.ndjson");
    snapshot_summary!("ask-dialog", summary);
}
