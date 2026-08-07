//! Deterministic NDJSON replay pipeline.
//!
//! Reads a raw physical NDJSON transcript (as produced by the M2 recorder),
//! decodes it through the production M1 [`LineDecoder`] and
//! [`ChunkReassembler`], decodes each logical frame into an
//! [`IncomingFrame`], and drives it through [`SessionProjection`]. The
//! resulting [`ReplayReport`] captures the final projection plus counters
//! that a snapshot test can assert on.
//!
//! This module is deliberately free of I/O helpers beyond `std::fs`: replay
//! is a pure, deterministic pipeline. Any nondeterministic content (wall
//! clock timestamps, absolute session paths, session ids) is either
//! normalized upstream by the recorder or projected onto the dedicated
//! [`ReplaySummary`] shape used by snapshot tests, which strips ids and
//! keeps only reviewable content.

use serde::ser::{SerializeMap, SerializeSeq};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use omp_rpc_client::RpcError;
use omp_rpc_client::decoder::{ChunkReassembler, LineDecoder};
use omp_rpc_client::frames::decode_frame;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::projection::{RunPhase, SessionProjection, StateQuality};
use crate::transcript::{CompactionPhase, ToolStatus, TranscriptEntry};

/// Errors surfaced by the replay pipeline. Every variant carries the source
/// file path and, where meaningful, the 1-based physical line that failed so
/// callers can locate malformed fixtures without re-parsing.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Failed to open or read the NDJSON file.
    #[error("failed to read replay fixture `{path}`: {source}")]
    Io {
        /// Path of the fixture that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// M1 decoder rejected a physical frame or a chunk sequence.
    #[error("frame decoding failed at `{path}` (physical frame #{physical_frame}): {source}")]
    Decode {
        /// Path of the fixture that failed to decode.
        path: PathBuf,
        /// 1-based physical frame index at which decoding failed.
        physical_frame: u64,
        /// Underlying RPC decoder error.
        #[source]
        source: RpcError,
    },

    /// Reassembled logical frame is a JSON object but not a recognizable
    /// wire frame envelope.
    #[error("logical frame decoding failed at `{path}` (logical frame #{logical_frame}): {source}")]
    FrameDecode {
        /// Path of the fixture that failed to decode.
        path: PathBuf,
        /// 1-based logical frame index at which decoding failed.
        logical_frame: u64,
        /// Underlying serde error.
        #[source]
        source: serde_json::Error,
    },

    /// Stream ended with an unterminated line or an unfinished `rpc_chunk`
    /// sequence still buffered.
    #[error("replay fixture `{path}` ended mid-stream: {detail}")]
    UnexpectedEof {
        /// Path of the truncated fixture.
        path: PathBuf,
        /// Human-readable detail about what was left mid-stream.
        detail: String,
    },
}

/// Result of replaying one NDJSON fixture end-to-end.
#[derive(Debug, Clone, Serialize)]
pub struct ReplayReport {
    /// Final [`SessionProjection`] after every frame has been applied.
    pub projection: SessionProjection,
    /// Total physical newline-terminated frames observed (including
    /// individual `rpc_chunk` frames).
    pub physical_frames: u64,
    /// Logical frames emitted after chunk reassembly (i.e. what the
    /// projection actually saw).
    pub logical_frames: u64,
    /// Subset of `physical_frames` whose top-level `type` was `rpc_chunk`.
    pub chunk_frames: u64,
    /// Number of `rpc_chunk` sequences that reassembled into a complete
    /// logical frame during this replay.
    pub reassembled_sequences: u64,
}

impl ReplayReport {
    /// Project this report onto a dedicated, review-friendly summary shape
    /// used by `insta` snapshot tests.
    ///
    /// The summary is a lossy view of the full projection: nondeterministic
    /// ids (tool call ids, dialog ids, session ids, widget payloads) are
    /// dropped, environment-scale catalogs (`available_commands_raw`) are
    /// reduced to names, and side-channel raw blobs are represented by
    /// their cardinality. Every field that carries M2 contract semantics —
    /// run phase, state quality/promoted scalars, transcript ordering and
    /// content, tool status, bounded-text elision markers, dialog method
    /// and options, chunk counts — is preserved verbatim.
    #[must_use]
    pub fn summary(&self) -> ReplaySummary {
        ReplaySummary::from_report(self)
    }
}

/// Stable, reviewable snapshot shape derived from a [`ReplayReport`].
///
/// See [`ReplayReport::summary`] for what is preserved and what is
/// deliberately dropped.
#[derive(Debug, Clone, Serialize)]
pub struct ReplaySummary {
    /// Final [`RunPhase`] the projection settled on.
    pub run_phase: RunPhase,
    /// Projected view of [`crate::projection::RuntimeState`] restricted to
    /// operational scalars that are stable across recordings.
    pub state: StateSummary,
    /// Semantic transcript, with nondeterministic ids stripped and
    /// bounded-text elision markers preserved.
    pub transcript: Vec<TranscriptSummary>,
    /// Pending UI dialogs; wire id dropped, method/options/timeout kept.
    pub pending_dialogs: Vec<DialogSummary>,
    /// Non-dialog UI display state; widget payloads reduced to their keys.
    pub display: DisplaySummary,
    /// Number of raw todo payloads currently retained, or `null` if the
    /// stream has never delivered one.
    pub todos: TodosSummary,
    /// Number of raw subagent payloads currently retained.
    pub subagents_count: usize,
    /// Number of available commands last reported. The catalog varies
    /// heavily with locally-installed custom commands, skills, and
    /// extensions and is intentionally reduced to a scalar to keep the
    /// snapshot reviewable.
    pub available_commands_count: usize,
    /// Reason the child was declared dead, if any.
    pub dead_reason: Option<String>,
    /// Replay-pipeline counters.
    pub counts: ReplayCounts,
}

/// Stable, promoted view of [`crate::projection::RuntimeState`].
#[derive(Debug, Clone, Serialize)]
pub struct StateSummary {
    /// Freshness of the promoted state.
    pub quality: StateQuality,
    /// Whether any `get_state` snapshot was ever hydrated.
    pub has_snapshot: bool,
    /// Current model identifier, when reported.
    pub model: Option<String>,
    /// Whether OMP reports an active streaming turn.
    pub is_streaming: Option<bool>,
    /// Whether OMP reports an active compaction.
    pub is_compacting: Option<bool>,
    /// Whether fast mode is enabled.
    pub fast_mode_enabled: Option<bool>,
    /// Whether fast mode is currently active.
    pub fast_mode_active: Option<bool>,
}

/// Redacted view of a [`TranscriptEntry`] used for snapshotting.
///
/// Tool-call rows drop the anthropic-style `tool_call_id` (nondeterministic
/// per recording) but keep the tool name, arguments, status, duration and
/// rendered bounded output — including the `\n…[N bytes elided]\n` marker
/// that flags a truncated buffer.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptSummary {
    /// A user message.
    User {
        /// User-visible text.
        text: String,
    },
    /// An assistant text row.
    AssistantText {
        /// Raw markdown source.
        markdown: String,
        /// Whether the row is still streaming.
        streaming: bool,
    },
    /// A thinking row.
    Thinking {
        /// User-visible thinking text.
        text: String,
        /// Whether the row is still streaming.
        streaming: bool,
        /// Whether the row is currently collapsed.
        collapsed: bool,
    },
    /// A tool-call row.
    ToolCall {
        /// Tool name.
        name: String,
        /// Tool arguments (verbatim wire JSON).
        #[serde(serialize_with = "serialize_canonical_json")]
        args_json: Value,
        /// Current lifecycle status.
        status: ToolStatus,
        /// Rendered bounded output including any elision marker.
        output: String,
        /// Wall-clock duration reported by the wire, when available.
        duration_ms: Option<u64>,
    },
    /// A `notify`-type notice.
    Notice {
        /// Notice text.
        text: String,
    },
    /// An error row.
    Error {
        /// Human-readable error message.
        message: String,
        /// Machine-readable error code, when reported.
        code: Option<String>,
    },
    /// A `/command` command-output row.
    CommandOutput {
        /// Command output text.
        text: String,
    },
    /// A compaction lifecycle row.
    Compaction {
        /// Compaction phase.
        phase: CompactionPhase,
    },
    /// A retry lifecycle row.
    RetryInfo {
        /// Human-readable retry detail.
        detail: String,
    },
    /// A raw, unrecognized frame preserved for review.
    Unknown {
        /// Raw JSON.
        #[serde(serialize_with = "serialize_canonical_json")]
        raw: Value,
    },
}

/// Redacted view of a [`crate::projection::UiDialog`]. The wire request `id`
/// is dropped; the payload is otherwise preserved so options, titles, and
/// unknown extras remain reviewable.
#[derive(Debug, Clone, Serialize)]
pub struct DialogSummary {
    /// Stable wire method (`select` | `confirm` | `input` | `editor` | `open_url`).
    pub method: String,
    /// Full wire payload with the nondeterministic `id` field removed.
    #[serde(serialize_with = "serialize_canonical_json")]
    pub payload: Value,
    /// Wire timeout value in milliseconds.
    pub timeout_ms: Option<f64>,
}

/// Redacted view of [`crate::projection::DisplayState`]. Widget payloads are
/// reduced to their `widgetKey` — the raw request carries a
/// nondeterministic wire id.
#[derive(Debug, Clone, Serialize)]
pub struct DisplaySummary {
    /// Current window title, if set.
    pub title: Option<String>,
    /// `setStatus` map keyed by `statusKey`.
    pub statuses: BTreeMap<String, Option<String>>,
    /// Sorted list of widget keys currently mounted (payloads dropped).
    pub widget_keys: Vec<String>,
    /// Current editor text, if set.
    pub editor_text: Option<String>,
}

/// Summary of retained todo payloads.
#[derive(Debug, Clone, Serialize)]
pub struct TodosSummary {
    /// Whether the projection currently holds a todo payload.
    pub present: bool,
    /// Number of top-level entries in that payload, when it is an array
    /// or object; `None` for scalar payloads or when absent.
    pub entries: Option<usize>,
}

/// Reduce the last-seen available-commands catalog to its cardinality. The
/// catalog is environment-dependent (locally-installed custom commands,
/// skills, extensions), so its names would render every recording a fresh
/// review problem.
fn available_commands_count(value: Option<&Value>) -> usize {
    match value {
        Some(Value::Array(items)) => items.len(),
        Some(Value::Object(map)) => match map.get("commands") {
            Some(Value::Array(items)) => items.len(),
            _ => 0,
        },
        _ => 0,
    }
}

/// Serialize raw JSON for snapshots with every object map sorted by key.
///
/// Wire values stay insertion-ordered in the projection. This affects only
/// the review summary, preventing equivalent live recordings from producing
/// noisy snapshots because their JSON object keys arrived in a different
/// order.
fn serialize_canonical_json<S>(value: &Value, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    CanonicalJson(value).serialize(serializer)
}

struct CanonicalJson<'a>(&'a Value);

impl Serialize for CanonicalJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.0 {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(value) => serializer.serialize_bool(*value),
            Value::Number(value) => value.serialize(serializer),
            Value::String(value) => serializer.serialize_str(value),
            Value::Array(values) => {
                let mut seq = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    seq.serialize_element(&CanonicalJson(value))?;
                }
                seq.end()
            }
            Value::Object(values) => {
                let mut entries: Vec<_> = values.iter().collect();
                entries.sort_unstable_by_key(|(key, _)| *key);
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, &CanonicalJson(value))?;
                }
                map.end()
            }
        }
    }
}

/// Replay-pipeline counters preserved verbatim from [`ReplayReport`].
#[derive(Debug, Clone, Serialize)]
pub struct ReplayCounts {
    /// Total physical newline-terminated frames observed.
    pub physical_frames: u64,
    /// Logical frames emitted after chunk reassembly.
    pub logical_frames: u64,
    /// Subset of `physical_frames` whose top-level `type` was `rpc_chunk`.
    pub chunk_frames: u64,
    /// Number of `rpc_chunk` sequences that reassembled into a complete
    /// logical frame.
    pub reassembled_sequences: u64,
}

impl ReplaySummary {
    fn from_report(report: &ReplayReport) -> Self {
        let projection = &report.projection;
        let state = &projection.state;
        Self {
            run_phase: projection.run_phase.clone(),
            state: StateSummary {
                quality: state.quality.clone(),
                has_snapshot: state.state.is_some(),
                model: state.model.clone(),
                is_streaming: state.is_streaming,
                is_compacting: state.is_compacting,
                fast_mode_enabled: state.fast_mode_enabled,
                fast_mode_active: state.fast_mode_active,
            },
            transcript: projection
                .transcript
                .iter()
                .map(TranscriptSummary::from_entry)
                .collect(),
            pending_dialogs: projection
                .pending_dialogs
                .iter()
                .map(DialogSummary::from_dialog)
                .collect(),
            display: DisplaySummary {
                title: projection.display.title.clone(),
                statuses: projection.display.statuses.clone(),
                widget_keys: projection.display.widgets.keys().cloned().collect(),
                editor_text: projection.display.editor_text.clone(),
            },
            todos: TodosSummary::from_raw(projection.todos_raw.as_ref()),
            subagents_count: projection.subagents_raw.len(),
            available_commands_count: available_commands_count(
                projection.available_commands_raw.as_ref(),
            ),
            dead_reason: projection.dead_reason.clone(),
            counts: ReplayCounts {
                physical_frames: report.physical_frames,
                logical_frames: report.logical_frames,
                chunk_frames: report.chunk_frames,
                reassembled_sequences: report.reassembled_sequences,
            },
        }
    }
}

impl TranscriptSummary {
    fn from_entry(entry: &TranscriptEntry) -> Self {
        match entry {
            TranscriptEntry::User { text } => Self::User { text: text.clone() },
            TranscriptEntry::AssistantText {
                markdown,
                streaming,
            } => Self::AssistantText {
                markdown: markdown.as_str().to_owned(),
                streaming: *streaming,
            },
            TranscriptEntry::Thinking {
                text,
                streaming,
                collapsed,
            } => Self::Thinking {
                text: text.clone(),
                streaming: *streaming,
                collapsed: *collapsed,
            },
            TranscriptEntry::ToolCall(tc) => Self::ToolCall {
                name: tc.name.clone(),
                args_json: tc.args_json.clone(),
                status: tc.status,
                output: tc.output.to_string(),
                duration_ms: tc.duration_ms,
            },
            TranscriptEntry::Notice(text) => Self::Notice { text: text.clone() },
            TranscriptEntry::Error { message, code } => Self::Error {
                message: message.clone(),
                code: code.clone(),
            },
            TranscriptEntry::CommandOutput(text) => Self::CommandOutput { text: text.clone() },
            TranscriptEntry::Compaction { phase } => Self::Compaction { phase: *phase },
            TranscriptEntry::RetryInfo { detail } => Self::RetryInfo {
                detail: detail.clone(),
            },
            TranscriptEntry::Unknown { raw } => Self::Unknown { raw: raw.clone() },
        }
    }
}

impl DialogSummary {
    fn from_dialog(dialog: &crate::projection::UiDialog) -> Self {
        let mut payload = dialog.payload.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.remove("id");
        }
        Self {
            method: dialog.method.clone(),
            payload,
            timeout_ms: dialog.timeout_ms,
        }
    }
}

impl TodosSummary {
    fn from_raw(value: Option<&Value>) -> Self {
        let Some(value) = value else {
            return Self {
                present: false,
                entries: None,
            };
        };
        let entries = match value {
            Value::Array(items) => Some(items.len()),
            Value::Object(map) => Some(map.len()),
            _ => None,
        };
        Self {
            present: true,
            entries,
        }
    }
}

/// Replay a raw NDJSON fixture from disk.
///
/// # Errors
/// Returns [`CoreError`] on I/O failure, framing/decoding failure, or an
/// unterminated stream at EOF.
pub fn replay_ndjson_file(path: impl AsRef<Path>) -> Result<ReplayReport, CoreError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    replay_ndjson_bytes(path, &bytes)
}

/// Replay a raw NDJSON transcript from an in-memory buffer. Exposed for
/// callers (tests, tooling) that already have the bytes.
///
/// # Errors
/// See [`replay_ndjson_file`].
pub fn replay_ndjson_bytes(path: &Path, bytes: &[u8]) -> Result<ReplayReport, CoreError> {
    let mut projection = SessionProjection::new();
    let mut line_decoder = LineDecoder::new();
    let mut reassembler = ChunkReassembler::new();

    // Counters, tracked outside the sink so we can borrow them mutably
    // alongside the projection.
    let mut physical_frames: u64 = 0;
    let mut logical_frames: u64 = 0;
    let mut chunk_frames: u64 = 0;
    let mut reassembled_sequences: u64 = 0;
    // Whether the currently-forming logical frame came from an rpc_chunk
    // sequence; flipped on each chunk frame, reset when the reassembler
    // emits.
    let mut chunk_sequence_active = false;

    // We collect the first framing error out of the sink via this Option;
    // the LineDecoder's sink contract wants an RpcError-compatible return.
    // Any inner projection/decoding failure is stored here and reported with
    // rich path context after `feed` returns.
    let mut inner_error: Option<CoreError> = None;

    let feed_result = line_decoder.feed(bytes, |value| {
        physical_frames += 1;
        let is_chunk = value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|s| s == "rpc_chunk");
        if is_chunk {
            chunk_frames += 1;
            chunk_sequence_active = true;
        }
        match reassembler.push(value) {
            Ok(None) => Ok(()),
            Ok(Some(logical)) => {
                logical_frames += 1;
                if chunk_sequence_active {
                    reassembled_sequences += 1;
                    chunk_sequence_active = false;
                }
                match decode_frame(logical) {
                    Ok(frame) => {
                        projection.apply(&frame);
                        Ok(())
                    }
                    Err(source) => {
                        inner_error = Some(CoreError::FrameDecode {
                            path: path.to_path_buf(),
                            logical_frame: logical_frames,
                            source,
                        });
                        // Halt the LineDecoder via a synthetic protocol
                        // violation; the real error is in `inner_error`.
                        Err(RpcError::ProtocolViolation {
                            detail: "replay: logical frame decode failed".into(),
                        })
                    }
                }
            }
            Err(source) => {
                inner_error = Some(CoreError::Decode {
                    path: path.to_path_buf(),
                    physical_frame: physical_frames,
                    source,
                });
                Err(RpcError::ProtocolViolation {
                    detail: "replay: chunk reassembly failed".into(),
                })
            }
        }
    });

    if let Some(err) = inner_error {
        return Err(err);
    }
    if let Err(source) = feed_result {
        return Err(CoreError::Decode {
            path: path.to_path_buf(),
            physical_frame: physical_frames.saturating_add(1),
            source,
        });
    }

    if let Err(source) = line_decoder.eof() {
        return Err(CoreError::UnexpectedEof {
            path: path.to_path_buf(),
            detail: format!("line decoder: {source}"),
        });
    }
    if reassembler.has_pending() {
        return Err(CoreError::UnexpectedEof {
            path: path.to_path_buf(),
            detail: "rpc_chunk sequence incomplete at end of stream".into(),
        });
    }

    Ok(ReplayReport {
        projection,
        physical_frames,
        logical_frames,
        chunk_frames,
        reassembled_sequences,
    })
}

#[cfg(test)]
mod tests {
    use super::serialize_canonical_json;
    use serde::Serialize;
    use serde_json::json;

    #[derive(Serialize)]
    struct SnapshotValue<'a> {
        #[serde(serialize_with = "serialize_canonical_json")]
        raw: &'a serde_json::Value,
    }

    #[test]
    fn canonical_json_serialization_sorts_nested_object_keys() {
        let raw = json!({
            "z": {"b": true, "a": false},
            "a": [{"d": 4, "c": 3}],
        });

        let rendered = serde_json::to_string(&SnapshotValue { raw: &raw })
            .expect("canonical snapshot JSON serializes");

        assert_eq!(
            rendered,
            r#"{"raw":{"a":[{"c":3,"d":4}],"z":{"a":false,"b":true}}}"#
        );
    }
}
