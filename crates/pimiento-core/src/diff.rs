//! Parse edit/write tool diffs for transcript review.

use serde_json::Value;

/// One rendered diff line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Raw line text including any leading marker (`+`/`-`/` `) when present.
    pub text: String,
    /// Semantic kind for coloring.
    pub kind: DiffLineKind,
}

/// Diff line classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// Unchanged / context.
    Context,
    /// Added line.
    Add,
    /// Removed line.
    Remove,
    /// Header / meta.
    Meta,
}

/// Structured edit/write diff view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditDiffView {
    /// Target path when known.
    pub path: Option<String>,
    /// Operation label (`update`, `create`, …).
    pub op: Option<String>,
    /// Parsed lines.
    pub lines: Vec<DiffLine>,
}

/// Prefer a human diff payload from tool result JSON.
#[must_use]
pub fn extract_tool_diff_text(result: &Value) -> Option<String> {
    let candidates = [
        result.pointer("/details/diff"),
        result.get("diff"),
        result.pointer("/result/details/diff"),
        result.pointer("/result/diff"),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Some(text) = candidate.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return Some(text.to_owned());
        }
    }
    None
}

/// Build an [`EditDiffView`] from tool name/args/result-or-output text.
#[must_use]
pub fn parse_edit_diff(
    tool_name: &str,
    args: &Value,
    output_or_result: &Value,
) -> Option<EditDiffView> {
    let name = tool_name.to_ascii_lowercase();
    if !(name == "edit" || name == "write" || name == "ast_edit" || name == "ast-edit") {
        return None;
    }

    let path = args
        .pointer("/input/path")
        .or_else(|| args.get("path"))
        .or_else(|| output_or_result.pointer("/details/path"))
        .or_else(|| output_or_result.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let op = output_or_result
        .pointer("/details/op")
        .or_else(|| output_or_result.get("op"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let diff_text = extract_tool_diff_text(output_or_result).or_else(|| {
        output_or_result.as_str().map(str::to_owned).filter(|s| {
            s.lines()
                .any(|line| line.starts_with('+') || line.starts_with('-'))
        })
    })?;

    let lines = parse_unified_diff_lines(&diff_text);
    if lines.is_empty() {
        return None;
    }
    Some(EditDiffView { path, op, lines })
}

/// Parse a compact/unified diff body into colored line kinds.
#[must_use]
pub fn parse_unified_diff_lines(diff: &str) -> Vec<DiffLine> {
    diff.lines()
        .map(|line| {
            let kind = if line.starts_with("+++")
                || line.starts_with("---")
                || line.starts_with("@@")
                || line.starts_with("diff ")
            {
                DiffLineKind::Meta
            } else if let Some(rest) = line.strip_prefix('+') {
                // Compact OMP diffs sometimes look like `+5| text`
                let _ = rest;
                DiffLineKind::Add
            } else if let Some(rest) = line.strip_prefix('-') {
                let _ = rest;
                DiffLineKind::Remove
            } else {
                DiffLineKind::Context
            };
            DiffLine {
                text: line.to_owned(),
                kind,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_details_diff_and_classifies_lines() {
        let result = json!({
            "details": {
                "diff": " 1| keep\n-2| old\n+2| new\n",
                "path": "src/main.rs",
                "op": "update"
            }
        });
        let args = json!({"path": "src/main.rs"});
        let view = parse_edit_diff("edit", &args, &result).expect("diff");
        assert_eq!(view.path.as_deref(), Some("src/main.rs"));
        assert_eq!(view.op.as_deref(), Some("update"));
        assert!(view.lines.iter().any(|l| l.kind == DiffLineKind::Add));
        assert!(view.lines.iter().any(|l| l.kind == DiffLineKind::Remove));
    }
}
