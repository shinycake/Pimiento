//! Typed views over OMP `todoPhases` payloads.

use serde_json::Value;

/// One task inside a todo phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoTaskView {
    /// User-visible task text.
    pub content: String,
    /// Wire status: `pending` / `in_progress` / `completed` / `abandoned` / `blocked`.
    pub status: String,
    /// Optional blocker note when status is blocked.
    pub blocker: Option<String>,
}

/// One named phase of tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoPhaseView {
    /// Phase title.
    pub name: String,
    /// Tasks in this phase.
    pub tasks: Vec<TodoTaskView>,
}

/// Parse `todos_raw` from `get_state` (`todoPhases`) or a `{phases:[…]}` wrapper.
#[must_use]
pub fn parse_todo_phases(raw: Option<&Value>) -> Vec<TodoPhaseView> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let entries = raw
        .as_array()
        .or_else(|| raw.get("phases").and_then(Value::as_array))
        .or_else(|| raw.get("todoPhases").and_then(Value::as_array));
    let Some(entries) = entries else {
        return Vec::new();
    };
    entries.iter().filter_map(parse_phase).collect()
}

/// Count tasks that are not terminal (completed/abandoned).
#[must_use]
pub fn actionable_todo_count(phases: &[TodoPhaseView]) -> usize {
    phases
        .iter()
        .flat_map(|phase| phase.tasks.iter())
        .filter(|task| !matches!(task.status.as_str(), "completed" | "abandoned"))
        .count()
}

/// Compact status glyph for UI lists.
#[must_use]
pub fn todo_status_glyph(status: &str) -> &'static str {
    match status {
        "completed" => "[x]",
        "in_progress" => "[*]",
        "blocked" => "[!]",
        "abandoned" => "[-]",
        _ => "[ ]",
    }
}

fn parse_phase(value: &Value) -> Option<TodoPhaseView> {
    let name = value.get("name").and_then(Value::as_str)?.trim();
    if name.is_empty() {
        return None;
    }
    let tasks = value
        .get("tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_task)
        .collect::<Vec<_>>();
    Some(TodoPhaseView {
        name: name.to_owned(),
        tasks,
    })
}

fn parse_task(value: &Value) -> Option<TodoTaskView> {
    let content = value.get("content").and_then(Value::as_str)?.trim();
    if content.is_empty() {
        return None;
    }
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("pending")
        .trim();
    let status = if status.is_empty() {
        "pending".to_owned()
    } else {
        status.to_owned()
    };
    let blocker = value
        .get("blocker")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    Some(TodoTaskView {
        content: content.to_owned(),
        status,
        blocker,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_array_and_wrapper_shapes() {
        let raw = json!([
            {
                "name": "Ship",
                "tasks": [
                    {"content": "Build", "status": "completed"},
                    {"content": "Test", "status": "in_progress"},
                    {"content": "Blocked wait", "status": "blocked", "blocker": "CI"},
                ]
            }
        ]);
        let phases = parse_todo_phases(Some(&raw));
        assert_eq!(phases.len(), 1);
        assert_eq!(phases[0].name, "Ship");
        assert_eq!(phases[0].tasks.len(), 3);
        assert_eq!(actionable_todo_count(&phases), 2);
        assert_eq!(todo_status_glyph("completed"), "[x]");
        assert_eq!(todo_status_glyph("in_progress"), "[*]");

        let wrapped = json!({ "phases": raw });
        assert_eq!(parse_todo_phases(Some(&wrapped)).len(), 1);
    }

    #[test]
    fn empty_or_invalid_raw_yields_no_phases() {
        assert!(parse_todo_phases(None).is_empty());
        assert!(parse_todo_phases(Some(&json!({}))).is_empty());
        assert!(parse_todo_phases(Some(&json!("nope"))).is_empty());
    }
}
