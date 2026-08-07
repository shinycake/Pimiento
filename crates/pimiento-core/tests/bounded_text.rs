//! Focused tests for `BoundedText` and adjacent transcript types.
//!
//! These verify the memory invariant advertised in `transcript.rs`:
//! * head + tail is bounded regardless of stream size,
//! * UTF-8 char boundaries are never split,
//! * the elided-byte counter is exact,
//! * the marker format is deterministic.

use pimiento_core::transcript::{
    BOUNDED_TEXT_CAP, BoundedText, CompactionPhase, Markdown, ToolCall, ToolStatus, TranscriptEntry,
};

const HEAD_CAP: usize = BoundedText::head_cap();
const TAIL_CAP: usize = BoundedText::tail_cap();

#[test]
fn under_cap_is_verbatim() {
    let mut b = BoundedText::new();
    b.push_str("hello ");
    b.push_str("world");
    assert!(!b.is_elided());
    assert_eq!(b.head(), "hello world");
    assert_eq!(b.tail(), "");
    assert_eq!(b.elided_bytes(), 0);
    assert_eq!(b.total_bytes(), "hello world".len() as u64);
    assert_eq!(b.render(), "hello world");
    assert_eq!(b.to_string(), "hello world");
    assert_eq!(b.marker(), None);
}

#[test]
fn exactly_head_cap_does_not_elide() {
    // A stream of exactly HEAD_CAP ASCII bytes fits entirely in head; no marker.
    let s: String = "a".repeat(HEAD_CAP);
    let b = BoundedText::from_text(&s);
    assert!(!b.is_elided());
    assert_eq!(b.head().len(), HEAD_CAP);
    assert_eq!(b.tail(), "");
    assert_eq!(b.elided_bytes(), 0);
    assert_eq!(b.total_bytes(), HEAD_CAP as u64);
}

#[test]
fn one_byte_over_head_cap_starts_tail_without_loss() {
    // HEAD_CAP + 1 bytes: head fills to HEAD_CAP, the last byte lives in tail,
    // no bytes are elided yet (head + tail == total).
    let s: String = "b".repeat(HEAD_CAP + 1);
    let b = BoundedText::from_text(&s);
    assert!(!b.is_elided(), "elided_bytes = {}", b.elided_bytes());
    assert_eq!(b.head().len(), HEAD_CAP);
    assert_eq!(b.tail(), "b");
    assert_eq!(b.total_bytes(), (HEAD_CAP + 1) as u64);
}

#[test]
fn over_cap_elides_middle_and_stays_bounded() {
    // Push well past BOUNDED_TEXT_CAP; head must retain the opening bytes and
    // tail the closing bytes, with a deterministic marker in between.
    let n: usize = BOUNDED_TEXT_CAP * 3;
    let mut b = BoundedText::new();
    for i in 0..n {
        // Cycling ASCII so we can identify head/tail contents.
        let c = char::from(b'a' + u8::try_from(i % 26).expect("mod 26 fits in u8"));
        let mut buf = [0u8; 1];
        b.push_str(c.encode_utf8(&mut buf));
    }
    assert!(b.is_elided());
    assert_eq!(b.head().len(), HEAD_CAP);
    // Head starts with the very first bytes of the stream.
    assert!(b.head().starts_with("abcdefghij"));
    // Tail ends with the terminal cycle. The last byte pushed corresponds to
    // (n-1) % 26.
    let last_idx = (n - 1) % 26;
    let last_c = char::from(b'a' + u8::try_from(last_idx).expect("mod 26 fits in u8"));
    assert!(b.tail().ends_with(last_c.to_string().as_str()));
    // Rendered form fits in the cap.
    assert!(b.len() <= BOUNDED_TEXT_CAP, "rendered len = {}", b.len());
    // Elided count is exact.
    assert_eq!(
        b.elided_bytes(),
        b.total_bytes() - b.head().len() as u64 - b.tail().len() as u64,
    );
    // Marker is stable and deterministic.
    let m = b.marker().expect("marker present when elided");
    assert_eq!(m, format!("\n…[{} bytes elided]\n", b.elided_bytes()));
    let rendered = b.render();
    assert!(rendered.contains(&m));
    assert!(rendered.starts_with(b.head()));
    assert!(rendered.ends_with(b.tail()));
}

#[test]
fn multibyte_char_never_split_at_head_boundary() {
    // Fill head with ASCII up to HEAD_CAP - 2, then push a 4-byte codepoint
    // that straddles the head/tail boundary. The 4-byte char must NOT be
    // sliced: it lands wholly in the tail, and head trims to HEAD_CAP - 2.
    let fill = HEAD_CAP - 2;
    let mut b = BoundedText::new();
    b.push_str(&"x".repeat(fill));
    // U+1F4A9 (🚀 is U+1F680; we use 🚀).
    b.push_str("🚀tail");
    assert!(b.head().is_char_boundary(b.head().len()));
    assert!(b.tail().is_char_boundary(b.tail().len()));
    // Head grew only by whole chars: still `fill` bytes because '🚀' is 4 bytes
    // and only 2 bytes of room remained, so it can't fit.
    assert_eq!(b.head().len(), fill);
    assert!(b.tail().starts_with("🚀"));
    // No bytes elided — everything is either in head or tail.
    assert_eq!(b.elided_bytes(), 0);
    // Rendered form is valid UTF-8 by construction (String is UTF-8).
    let _ = b.render();
}

#[test]
fn multibyte_char_never_split_at_tail_compaction() {
    // Build a stream well past the compaction trigger using 4-byte codepoints
    // exclusively. Every compaction must leave the tail on a char boundary.
    let mut b = BoundedText::new();
    let chunk = "🚀".repeat(4096); // 16 KiB per push
    // Push enough to force many compaction cycles.
    for _ in 0..64 {
        b.push_str(&chunk);
    }
    assert!(b.is_elided());
    // Head and tail are valid UTF-8 (accessing them as &str already guarantees
    // that; verify explicitly that the last byte is a boundary).
    assert!(b.head().is_char_boundary(b.head().len()));
    assert!(b.tail().is_char_boundary(b.tail().len()));
    // Every character is a rocket.
    assert!(b.head().chars().all(|c| c == '🚀'));
    assert!(b.tail().chars().all(|c| c == '🚀'));
    assert!(b.len() <= BOUNDED_TEXT_CAP);
}

#[test]
fn repeated_appends_stay_bounded() {
    // 10 000 small appends totalling ~10 MiB. Head + rendered tail must stay
    // within `HEAD_CAP + TAIL_CAP` regardless of iteration count, and the
    // rendered form must never exceed `BOUNDED_TEXT_CAP`.
    let mut b = BoundedText::new();
    let chunk = "y".repeat(1024);
    for _ in 0..10_000 {
        b.push_str(&chunk);
        assert!(
            b.head().len() + b.tail().len() <= HEAD_CAP + TAIL_CAP,
            "head+tail grew beyond invariant",
        );
        assert!(b.len() <= BOUNDED_TEXT_CAP);
    }
    assert!(b.is_elided());
    assert_eq!(b.total_bytes(), 10_000u64 * 1024);
    assert_eq!(
        b.elided_bytes(),
        b.total_bytes() - b.head().len() as u64 - b.tail().len() as u64,
    );
    assert!(b.len() <= BOUNDED_TEXT_CAP);
}

#[test]
fn head_and_tail_both_present_after_large_stream() {
    // Unique sentinels at the very start and very end must both survive.
    let head_sentinel = "HEAD_SENTINEL_QRSTUV";
    let tail_sentinel = "TAIL_SENTINEL_MNOPQR";
    let filler = "-".repeat(BOUNDED_TEXT_CAP * 2);
    let mut b = BoundedText::new();
    b.push_str(head_sentinel);
    b.push_str(&filler);
    b.push_str(tail_sentinel);
    assert!(b.head().starts_with(head_sentinel));
    assert!(b.tail().ends_with(tail_sentinel));
    let rendered = b.render();
    assert!(rendered.starts_with(head_sentinel));
    assert!(rendered.ends_with(tail_sentinel));
    assert!(rendered.contains("bytes elided]"));
}

#[test]
fn huge_single_append_bounded_in_one_shot() {
    // A single 4 MiB push must produce a valid bounded state with head+tail
    // within the invariant.
    let s = "z".repeat(4 * 1024 * 1024);
    let b = BoundedText::from_text(&s);
    assert!(b.is_elided());
    assert_eq!(b.total_bytes(), s.len() as u64);
    assert_eq!(b.head().len(), HEAD_CAP);
    assert!(b.tail().len() <= TAIL_CAP);
    assert_eq!(
        b.elided_bytes(),
        b.total_bytes() - b.head().len() as u64 - b.tail().len() as u64,
    );
    assert!(b.len() <= BOUNDED_TEXT_CAP);
    // Head and tail both contain 'z' (deterministic content).
    assert!(b.head().chars().all(|c| c == 'z'));
    assert!(b.tail().chars().all(|c| c == 'z'));
}

#[test]
fn marker_format_is_stable_and_deterministic() {
    // Same input sequence → identical marker.
    let build = || {
        let mut b = BoundedText::new();
        b.push_str(&"q".repeat(BOUNDED_TEXT_CAP * 2));
        b
    };
    let a = build();
    let c = build();
    assert_eq!(a.marker(), c.marker());
    assert_eq!(a.elided_bytes(), c.elided_bytes());
    // Marker uses U+2026 (…) between two newlines with an explicit byte count.
    let m = a.marker().expect("elided");
    assert!(m.starts_with('\n'));
    assert!(m.ends_with('\n'));
    assert!(m.contains("…["));
    assert!(m.contains(" bytes elided]"));
    // Parseable count matches.
    let n_str: String = m.chars().filter(char::is_ascii_digit).collect();
    let parsed: u64 = n_str.parse().expect("marker digits parse");
    assert_eq!(parsed, a.elided_bytes());
}

#[test]
fn clear_resets_all_counters() {
    let mut b = BoundedText::from_text(&"w".repeat(BOUNDED_TEXT_CAP * 2));
    assert!(b.is_elided());
    b.clear();
    assert!(b.is_empty());
    assert_eq!(b.total_bytes(), 0);
    assert_eq!(b.elided_bytes(), 0);
    assert_eq!(b.head(), "");
    assert_eq!(b.tail(), "");
    assert_eq!(b.marker(), None);
    assert_eq!(b.render(), "");
}

#[test]
fn empty_push_is_noop() {
    let mut b = BoundedText::new();
    b.push_str("");
    assert!(b.is_empty());
    assert_eq!(b.total_bytes(), 0);
}

#[test]
fn tool_call_default_shape() {
    let tc = ToolCall::new_running("call-1", "bash", serde_json::json!({"cmd": "ls"}));
    assert_eq!(tc.tool_call_id, "call-1");
    assert_eq!(tc.name, "bash");
    assert_eq!(tc.status, ToolStatus::Running);
    assert!(tc.output.is_empty());
    assert_eq!(tc.duration_ms, None);
    assert!(!tc.expanded);
}

#[test]
fn transcript_entry_serde_roundtrip() {
    let entries = vec![
        TranscriptEntry::User { text: "hi".into() },
        TranscriptEntry::AssistantText {
            markdown: Markdown::new("# reply"),
            streaming: false,
        },
        TranscriptEntry::Thinking {
            text: "…".into(),
            streaming: true,
            collapsed: true,
        },
        TranscriptEntry::ToolCall(ToolCall::new_running(
            "t1",
            "read",
            serde_json::json!({"path": "x"}),
        )),
        TranscriptEntry::Notice("hello".into()),
        TranscriptEntry::Error {
            message: "boom".into(),
            code: Some("E42".into()),
        },
        TranscriptEntry::CommandOutput("done".into()),
        TranscriptEntry::Compaction {
            phase: CompactionPhase::Started,
        },
        TranscriptEntry::RetryInfo {
            detail: "n=1".into(),
        },
        TranscriptEntry::Unknown {
            raw: serde_json::json!({"weird": true}),
        },
    ];
    let s = serde_json::to_string(&entries).expect("serialize");
    let back: Vec<TranscriptEntry> = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(entries, back);
}

#[test]
fn bounded_text_serde_roundtrip_preserves_state() {
    let mut b = BoundedText::new();
    b.push_str(&"p".repeat(BOUNDED_TEXT_CAP * 2));
    let s = serde_json::to_string(&b).expect("serialize");
    let back: BoundedText = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(b, back);
    assert_eq!(b.head(), back.head());
    assert_eq!(b.tail(), back.tail());
    assert_eq!(b.elided_bytes(), back.elided_bytes());
    assert_eq!(b.total_bytes(), back.total_bytes());
}

#[test]
fn bounded_text_serde_rejects_invalid_retained_byte_count() {
    let invalid = serde_json::json!({
        "head": "prefix",
        "tail": "suffix",
        "total_bytes": 1,
    });
    let err = serde_json::from_value::<BoundedText>(invalid).expect_err("invalid byte count");
    assert!(
        err.to_string()
            .contains("invalid BoundedText representation")
    );
}
