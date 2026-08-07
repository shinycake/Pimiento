//! Transcript domain types for Pimiento.
//!
//! UI-free by construction: this module never depends on GPUI or any renderer.
//! See `PLAN.md` §5.4 and §6 M2.
//!
//! [`BoundedText`] caps stored tool output at [`BOUNDED_TEXT_CAP`] rendered
//! bytes — retaining a deterministic head and tail joined by a stable elision
//! marker of the form `"\n…[N bytes elided]\n"`. All append operations
//! preserve UTF-8 boundaries and never require more than a bounded amount of
//! transient allocation on top of the caller-owned chunk (§5.4, SH-5).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum rendered size of a [`BoundedText`], in bytes.
///
/// 512 KiB — the storage budget from `PLAN.md` §5.4. This bounds the
/// concatenation of `head`, the elision marker, and `tail`.
pub const BOUNDED_TEXT_CAP: usize = 512 * 1024;

/// Bytes reserved for the elision marker. `\n` + `…` (3-byte UTF-8) + `[` +
/// up to 20 decimal digits (`u64::MAX`) + ` bytes elided]` + `\n` fits in ~40;
/// 64 keeps a safe margin.
const MARKER_RESERVE: usize = 64;

/// Bytes retained at the start of an elided text.
const HEAD_CAP: usize = (BOUNDED_TEXT_CAP - MARKER_RESERVE) / 2;

/// Bytes retained at the end of an elided text. Chosen so that
/// `HEAD_CAP + TAIL_CAP + MARKER_RESERVE == BOUNDED_TEXT_CAP`, which is what
/// makes `head + marker + tail` fit the cap for any marker up to
/// `MARKER_RESERVE` bytes. The `+3` slack the `push_str` post-condition
/// admits is a UTF-8 rounding artifact of `ceil_char_boundary` (at most one
/// 4-byte codepoint minus one byte) and is absorbed by `MARKER_RESERVE`.
const TAIL_CAP: usize = BOUNDED_TEXT_CAP - MARKER_RESERVE - HEAD_CAP;

// ---------------------------------------------------------------------------
// BoundedText
// ---------------------------------------------------------------------------

/// A UTF-8 text buffer whose rendered form is bounded to [`BOUNDED_TEXT_CAP`]
/// bytes.
///
/// Memory invariant:
/// - `head.len() <= HEAD_CAP`; head fills first, then freezes on the first
///   byte routed to the tail so stream order is preserved.
/// - The tail is stored in a contiguous buffer that is compacted whenever it
///   exceeds `2 * TAIL_CAP`; the buffer is therefore always `<= 2 * TAIL_CAP`
///   bytes at rest. Compaction drains only bytes that will never be shown
///   again, so every byte is moved at most twice — amortized O(1) per byte.
/// - The *rendered* tail is the trailing `<= TAIL_CAP` bytes of the buffer,
///   sliced at a char boundary (via `ceil_char_boundary`), so the rendered
///   form (`Display` / [`render`](Self::render)) is always
///   `<= HEAD_CAP + MARKER_RESERVE + TAIL_CAP == BOUNDED_TEXT_CAP` bytes.
/// - `head` and the rendered `tail` are always valid UTF-8; no split ever
///   lands inside a codepoint.
/// - `elided_bytes = total_bytes - head.len() - tail.len()` and grows
///   monotonically across appends (both hysteresis "hidden" bytes and truly
///   dropped bytes count as elided — they are not part of the rendered
///   output).
///
/// The serialized shape captures head, rendered tail, and `total_bytes` —
/// enough to reconstruct the projection deterministically and to produce
/// stable `insta` snapshots.
#[derive(Clone, Default)]
pub struct BoundedText {
    head: String,
    /// Hysteresis storage. Only its trailing rendered-tail slice is ever
    /// exposed; the leading bytes are already elided but held to keep append
    /// amortized O(1).
    tail_buf: String,
    total_bytes: u64,
}

impl BoundedText {
    /// Total storage budget (rendered bytes), including the elision marker.
    #[must_use]
    pub const fn cap() -> usize {
        BOUNDED_TEXT_CAP
    }

    /// Maximum retained head length in bytes.
    #[must_use]
    pub const fn head_cap() -> usize {
        HEAD_CAP
    }

    /// Maximum rendered tail length in bytes.
    #[must_use]
    pub const fn tail_cap() -> usize {
        TAIL_CAP
    }

    /// Creates an empty bounded text.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty bounded text with a pre-allocated head, clamped to
    /// [`HEAD_CAP`]. Useful when the caller has an upper bound on the payload.
    #[must_use]
    pub fn with_capacity_hint(hint: usize) -> Self {
        Self {
            head: String::with_capacity(hint.min(HEAD_CAP)),
            tail_buf: String::new(),
            total_bytes: 0,
        }
    }

    /// Builds a bounded text from a single string, truncating as needed.
    #[must_use]
    pub fn from_text(s: &str) -> Self {
        let mut b = Self::new();
        b.push_str(s);
        b
    }

    /// Resets to empty. Preserves any pre-allocated capacity.
    pub fn clear(&mut self) {
        self.head.clear();
        self.tail_buf.clear();
        self.total_bytes = 0;
    }

    /// The retained prefix. Always valid UTF-8.
    #[must_use]
    pub fn head(&self) -> &str {
        &self.head
    }

    /// The retained suffix (empty until the first elision). Always valid
    /// UTF-8, always `<= TAIL_CAP` bytes long, always aligned on a codepoint
    /// boundary.
    #[must_use]
    pub fn tail(&self) -> &str {
        let n = self.tail_buf.len();
        if n <= TAIL_CAP {
            return &self.tail_buf;
        }
        let want_start = n - TAIL_CAP;
        let start = ceil_char_boundary(&self.tail_buf, want_start);
        &self.tail_buf[start..]
    }

    /// Total bytes ever appended.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Bytes dropped from the rendered output. `total_bytes - head - tail`.
    #[must_use]
    pub fn elided_bytes(&self) -> u64 {
        // Invariant guarantees head + rendered tail <= total_bytes.
        self.total_bytes - self.head.len() as u64 - self.tail().len() as u64
    }

    /// Whether any bytes have been elided from the rendered output.
    #[must_use]
    pub fn is_elided(&self) -> bool {
        self.elided_bytes() > 0
    }

    /// Whether no bytes have ever been appended.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total_bytes == 0
    }

    /// The elision marker, if any bytes were elided. Deterministic format:
    /// `"\n…[N bytes elided]\n"`.
    #[must_use]
    pub fn marker(&self) -> Option<String> {
        if self.is_elided() {
            Some(format!("\n…[{} bytes elided]\n", self.elided_bytes()))
        } else {
            None
        }
    }

    /// Rendered length in bytes (head + marker + tail). Always `<= cap()`.
    #[must_use]
    #[allow(clippy::len_without_is_empty)] // `is_empty` reports the logical stream, not the rendered form.
    pub fn len(&self) -> usize {
        match self.marker() {
            Some(m) => self.head.len() + m.len() + self.tail().len(),
            None => self.head.len(),
        }
    }

    /// Renders `head + marker + tail` into a new `String`. O(head + tail).
    #[must_use]
    pub fn render(&self) -> String {
        let tail = self.tail();
        match self.marker() {
            Some(m) => {
                let mut s = String::with_capacity(self.head.len() + m.len() + tail.len());
                s.push_str(&self.head);
                s.push_str(&m);
                s.push_str(tail);
                s
            }
            None => self.head.clone(),
        }
    }

    /// Appends a UTF-8 chunk, preserving char boundaries and the rendered
    /// cap.
    ///
    /// Transient allocation on top of the caller-owned chunk is bounded by
    /// the small hysteresis constant (`TAIL_CAP` in the worst case), and
    /// each byte is copied into the tail buffer at most twice across its
    /// entire lifetime — so streaming append is amortized O(1) per byte.
    pub fn push_str(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        self.total_bytes = self.total_bytes.saturating_add(chunk.len() as u64);

        let mut rest = chunk;

        // Fill head while the tail is still empty. Once we've started routing
        // bytes to the tail, head stays frozen so ordering is preserved.
        if self.tail_buf.is_empty() && self.head.len() < HEAD_CAP {
            let room = HEAD_CAP - self.head.len();
            if rest.len() <= room {
                self.head.push_str(rest);
                return;
            }
            let split = floor_char_boundary(rest, room);
            self.head.push_str(&rest[..split]);
            rest = &rest[split..];
            if rest.is_empty() {
                return;
            }
        }

        self.append_tail(rest);
    }

    /// Routes `rest` into the tail buffer, compacting once it grows past the
    /// hysteresis limit (`2 * TAIL_CAP`).
    fn append_tail(&mut self, rest: &str) {
        // If the incoming chunk alone eclipses the entire rendered tail, we
        // can skip appending the parts that would be dropped anyway.
        if rest.len() >= TAIL_CAP {
            self.tail_buf.clear();
            let start_raw = rest.len() - TAIL_CAP;
            let start = ceil_char_boundary(rest, start_raw);
            self.tail_buf.push_str(&rest[start..]);
            return;
        }

        self.tail_buf.push_str(rest);

        // Compact only when the hysteresis buffer overflows. This bounds
        // the amortized cost of `push_str` to O(chunk.len()).
        if self.tail_buf.len() > 2 * TAIL_CAP {
            let n = self.tail_buf.len();
            let want_start = n - TAIL_CAP;
            let start = ceil_char_boundary(&self.tail_buf, want_start);
            self.tail_buf.drain(..start);
        }
    }
}

impl PartialEq for BoundedText {
    /// Equality is defined on the *observable* state: head, rendered tail,
    /// and the running byte counter. Two `BoundedText` values that render identically
    /// and have seen the same total volume compare equal, regardless of any
    /// hysteresis bytes the tail buffer happens to be holding.
    fn eq(&self, other: &Self) -> bool {
        self.total_bytes == other.total_bytes
            && self.head == other.head
            && self.tail() == other.tail()
    }
}

impl Eq for BoundedText {}

impl fmt::Debug for BoundedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundedText")
            .field("head_bytes", &self.head.len())
            .field("tail_bytes", &self.tail().len())
            .field("total_bytes", &self.total_bytes)
            .field("elided_bytes", &self.elided_bytes())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for BoundedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.head)?;
        if let Some(m) = self.marker() {
            f.write_str(&m)?;
        }
        f.write_str(self.tail())
    }
}

// Serialize/Deserialize via a stable public shape: {head, tail, total_bytes}.
// The rendered tail is what serializes, so replay reconstructs the exact
// projection state seen at snapshot time.
#[derive(Serialize, Deserialize)]
struct BoundedTextRepr<'a> {
    head: std::borrow::Cow<'a, str>,
    tail: std::borrow::Cow<'a, str>,
    total_bytes: u64,
}

impl Serialize for BoundedText {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        BoundedTextRepr {
            head: std::borrow::Cow::Borrowed(&self.head),
            tail: std::borrow::Cow::Borrowed(self.tail()),
            total_bytes: self.total_bytes,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for BoundedText {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = BoundedTextRepr::deserialize(d)?;
        let head = r.head.into_owned();
        let tail = r.tail.into_owned();
        let retained = head.len().saturating_add(tail.len()) as u64;
        if head.len() > HEAD_CAP || tail.len() > TAIL_CAP || r.total_bytes < retained {
            return Err(serde::de::Error::custom(
                "invalid BoundedText representation",
            ));
        }
        Ok(BoundedText {
            head,
            tail_buf: tail,
            total_bytes: r.total_bytes,
        })
    }
}

// ---------------------------------------------------------------------------
// UTF-8 boundary helpers (stable equivalents of `str::{floor,ceil}_char_boundary`)
// ---------------------------------------------------------------------------

fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

// ---------------------------------------------------------------------------
// Tool lifecycle, compaction, markdown
// ---------------------------------------------------------------------------

/// Status of a tool invocation on the transcript. `PLAN.md` §5.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStatus {
    Running,
    Ok,
    Err,
}

/// Phase of a compaction event. `PLAN.md` §5.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionPhase {
    Started,
    Progress,
    Completed,
    Failed,
}

/// Minimal markdown representation for M2 — the raw source string.
///
/// SH-era rendering will replace this with a parsed IR; the wire text is
/// preserved here so replay is faithful and later parsers are drop-in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Markdown(pub String);

impl Markdown {
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Markdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Transcript entries
// ---------------------------------------------------------------------------

/// A single tool call row. Keyed inside the projection by `tool_call_id` so
/// concurrent lifecycles resolve to the correct entry; the transcript itself
/// stays append-mostly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_call_id: String,
    pub name: String,
    pub args_json: serde_json::Value,
    pub status: ToolStatus,
    pub output: BoundedText,
    pub duration_ms: Option<u64>,
    pub expanded: bool,
}

impl ToolCall {
    /// Constructs a freshly-started tool call with an empty output buffer and
    /// `expanded = false`.
    #[must_use]
    pub fn new_running(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        args_json: serde_json::Value,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            name: name.into(),
            args_json,
            status: ToolStatus::Running,
            output: BoundedText::new(),
            duration_ms: None,
            expanded: false,
        }
    }
}

/// One row in the transcript. `PLAN.md` §5.4; `Unknown` MUST always render.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TranscriptEntry {
    User {
        text: String,
    },
    AssistantText {
        markdown: Markdown,
        streaming: bool,
    },
    Thinking {
        text: String,
        streaming: bool,
        collapsed: bool,
    },
    ToolCall(ToolCall),
    Notice(String),
    Error {
        message: String,
        code: Option<String>,
    },
    CommandOutput(String),
    Compaction {
        phase: CompactionPhase,
    },
    RetryInfo {
        detail: String,
    },
    Unknown {
        raw: serde_json::Value,
    },
}
