use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolGroupPosition {
    pub(crate) grouped: bool,
    pub(crate) first: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolVisualKind {
    Terminal,
    ReadFile,
    WriteFile,
    Search,
    Agent,
    Web,
    Hub,
    Ask,
    Todo,
    Generic,
}

/// Classify only from the wire tool name; no tool state is inferred.
pub(crate) fn tool_visual_kind(tool_name: &str) -> ToolVisualKind {
    match tool_name.to_ascii_lowercase().as_str() {
        "bash" | "shell" | "terminal" => ToolVisualKind::Terminal,
        "read" | "read_file" => ToolVisualKind::ReadFile,
        "write" | "edit" | "ast_edit" | "write_file" => ToolVisualKind::WriteFile,
        "grep" | "glob" | "search" => ToolVisualKind::Search,
        "task" | "agent" | "subagent" => ToolVisualKind::Agent,
        "web" | "web_search" | "browser" => ToolVisualKind::Web,
        "hub" | "network" => ToolVisualKind::Hub,
        "ask" | "user" => ToolVisualKind::Ask,
        "todo" | "checklist" => ToolVisualKind::Todo,
        _ => ToolVisualKind::Generic,
    }
}

fn tool_icon(kind: ToolVisualKind) -> IconName {
    match kind {
        ToolVisualKind::Terminal => IconName::SquareTerminal,
        ToolVisualKind::ReadFile | ToolVisualKind::WriteFile => IconName::File,
        ToolVisualKind::Search => IconName::Search,
        ToolVisualKind::Agent => IconName::Bot,
        ToolVisualKind::Web => IconName::Globe,
        ToolVisualKind::Hub => IconName::Network,
        ToolVisualKind::Ask => IconName::User,
        ToolVisualKind::Todo => IconName::CircleCheck,
        ToolVisualKind::Generic => IconName::Asterisk,
    }
}

fn tool_icon_color(kind: ToolVisualKind, theme: &Theme) -> gpui::Hsla {
    match kind {
        ToolVisualKind::Terminal | ToolVisualKind::Web => theme.info,
        ToolVisualKind::ReadFile | ToolVisualKind::Search | ToolVisualKind::Ask => theme.accent,
        ToolVisualKind::WriteFile | ToolVisualKind::Hub => theme.warning,
        ToolVisualKind::Agent => theme.primary,
        ToolVisualKind::Todo => theme.success,
        ToolVisualKind::Generic => theme.muted_foreground,
    }
}

/// Readable prose measure for user/assistant rows (~48rem). Tools stay full-width.
pub(crate) fn transcript_prose_max() -> Pixels {
    px(768.)
}

/// Shared height for the compose field and primary Send/Steer control.
pub(crate) fn composer_control_height() -> Pixels {
    px(40.)
}

pub(crate) fn entry_starts_user_turn(transcript: &[TranscriptEntry], row_ix: usize) -> bool {
    matches!(transcript.get(row_ix), Some(TranscriptEntry::User { .. }))
        && (row_ix == 0
            || !matches!(
                transcript.get(row_ix.saturating_sub(1)),
                Some(TranscriptEntry::User { .. })
            ))
}

/// Derive action-group chrome without merging transcript entries.
///
/// Keeping one rendered item per transcript entry preserves every `ListState`
/// index and lets tool expansion remeasure only the affected row.
pub(crate) fn tool_group_position(
    transcript: &[TranscriptEntry],
    row_ix: usize,
) -> ToolGroupPosition {
    let is_tool = transcript
        .get(row_ix)
        .is_some_and(|entry| matches!(entry, TranscriptEntry::ToolCall(_)));
    let previous_is_tool = row_ix.checked_sub(1).is_some_and(|previous_ix| {
        transcript
            .get(previous_ix)
            .is_some_and(|entry| matches!(entry, TranscriptEntry::ToolCall(_)))
    });
    let next_is_tool = transcript
        .get(row_ix + 1)
        .is_some_and(|entry| matches!(entry, TranscriptEntry::ToolCall(_)));

    ToolGroupPosition {
        grouped: is_tool && (previous_is_tool || next_is_tool),
        first: is_tool && !previous_is_tool,
    }
}

#[allow(clippy::too_many_lines)] // Match arms mirror transcript variants.
pub(crate) fn render_entry(
    row_ix: usize,
    entry: &TranscriptEntry,
    tool_group: ToolGroupPosition,
    turn_start: bool,
    expanded: &HashSet<String>,
    running_tool_started: &HashMap<String, Instant>,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let row = match entry {
        TranscriptEntry::User { text } => {
            let text_for_copy = text.clone();
            h_flex()
                .w_full()
                .items_start()
                .gap_3()
                .when(turn_start, gpui::Styled::pt_5)
                .when(!turn_start, gpui::Styled::pt_3)
                .pb_2()
                .group("transcript-row")
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .max_w(transcript_prose_max())
                        .border_l_2()
                        .border_color(theme.accent)
                        .pl_4()
                        .pr_2()
                        .py_1()
                        .text_sm()
                        .text_color(theme.foreground)
                        .child(soft_wrap_dynamic_text(text)),
                )
                .child(
                    Button::new(("copy-user", row_ix))
                        .icon(IconName::Copy)
                        .tooltip("Copy message")
                        .small()
                        .ghost()
                        .invisible()
                        .group_hover("transcript-row", gpui::Styled::visible)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text_for_copy.clone()));
                        }),
                )
                .into_any_element()
        }
        TranscriptEntry::AssistantText {
            markdown,
            streaming,
        } => {
            let markdown_for_copy = markdown.as_str().to_owned();
            let assistant_content = if *streaming && markdown.as_str().trim().is_empty() {
                Label::new("…")
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .into_any_element()
            } else {
                TextView::markdown(("assistant", row_ix), markdown.as_str())
                    .selectable(true)
                    .code_block_actions(move |code_block, _, _cx| {
                        let code = code_block.code().to_string();
                        let lang = code_block.lang().map(|lang| lang.to_string());
                        Button::new(code_block_copy_id(row_ix, lang.as_deref(), &code))
                            .icon(IconName::Copy)
                            .tooltip("Copy code")
                            .small()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                            })
                    })
                    .into_any_element()
            };
            h_flex()
                .w_full()
                .items_start()
                .gap_3()
                .pt_1()
                .pb_4()
                .group("transcript-row")
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .max_w(transcript_prose_max())
                        .pl_1()
                        .text_sm()
                        .text_color(theme.foreground)
                        .child(assistant_content),
                )
                .child(
                    Button::new(("copy-assistant", row_ix))
                        .icon(IconName::Copy)
                        .tooltip("Copy response")
                        .small()
                        .ghost()
                        .invisible()
                        .group_hover("transcript-row", gpui::Styled::visible)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                markdown_for_copy.clone(),
                            ));
                        }),
                )
                .into_any_element()
        }
        TranscriptEntry::Thinking {
            text,
            streaming: false,
            ..
        } if text.trim().is_empty() => {
            // Empty completed thinking blocks are wire noise (start/end with no
            // deltas). Keep the list index but render nothing.
            div().w_full().into_any_element()
        }
        TranscriptEntry::Thinking {
            collapsed: true,
            text,
            ..
        } => {
            let view = cx.entity().downgrade();
            let text_for_copy = text.clone();
            let preview = thinking_collapse_preview(text);
            h_flex()
                .w_full()
                .items_start()
                .gap_2()
                .py_1()
                .child(
                    div()
                        .id(("thinking-collapsed", row_ix))
                        .flex_1()
                        .min_w_0()
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.toggle_thinking_row(row_ix, cx);
                            });
                        })
                        .child(
                            div()
                                .py_0p5()
                                .text_color(theme.muted_foreground)
                                .text_xs()
                                .italic()
                                .child(preview),
                        ),
                )
                .child(
                    Button::new(("copy-thinking", row_ix))
                        .icon(IconName::Copy)
                        .tooltip("Copy thinking")
                        .small()
                        .ghost()
                        .invisible()
                        .group_hover("transcript-row", gpui::Styled::visible)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text_for_copy.clone()));
                        }),
                )
                .into_any_element()
        }
        TranscriptEntry::Thinking { text, .. } => {
            let view = cx.entity().downgrade();
            let text_for_copy = text.clone();
            div()
                .id(("thinking-expanded", row_ix))
                .w_full()
                .py_2()
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Button::new(("thinking-collapse", row_ix))
                                        .icon(IconName::ChevronUp)
                                        .label("Thinking")
                                        .tooltip("Collapse thinking")
                                        .small()
                                        .ghost()
                                        .on_click(move |_, _, cx| {
                                            let _ = view.update(cx, |this, cx| {
                                                this.toggle_thinking_row(row_ix, cx);
                                            });
                                        }),
                                )
                                .child(
                                    Button::new(("copy-thinking", row_ix))
                                        .icon(IconName::Copy)
                                        .tooltip("Copy thinking")
                                        .small()
                                        .ghost()
                                        .invisible()
                                        .group_hover("transcript-row", gpui::Styled::visible)
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                text_for_copy.clone(),
                                            ));
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .bg(theme.secondary)
                                .text_color(theme.muted_foreground)
                                .italic()
                                .child(TextView::markdown(("thinking", row_ix), text.clone())),
                        ),
                )
                .into_any_element()
        }
        TranscriptEntry::ToolCall(tc) => render_tool_card(
            row_ix,
            tc,
            tool_group,
            expanded.contains(&tc.tool_call_id),
            running_tool_started,
            cx,
        ),
        TranscriptEntry::Notice(text) => {
            let text_for_copy = text.clone();
            let mount_noise = notice_looks_like_mount_event(text);
            div()
                .w_full()
                .when(mount_noise, gpui::Styled::py_0p5)
                .when(!mount_noise, gpui::Styled::py_1)
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .gap_2()
                        .when(!mount_noise, |row| {
                            row.child(
                                Label::new("Notice")
                                    .text_xs()
                                    .text_color(theme.muted_foreground),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .opacity(if mount_noise { 0.72 } else { 1. })
                                .child(soft_wrap_dynamic_text(text)),
                        )
                        .child(
                            Button::new(("copy-notice", row_ix))
                                .icon(IconName::Copy)
                                .tooltip("Copy notice")
                                .small()
                                .ghost()
                                .invisible()
                                .group_hover("transcript-row", gpui::Styled::visible)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        text_for_copy.clone(),
                                    ));
                                }),
                        ),
                )
                .into_any_element()
        }
        TranscriptEntry::Error { message, code } => {
            let copy_text = match code {
                Some(code) => format!("{message}\ncode: {code}"),
                None => message.clone(),
            };
            div()
                .w_full()
                .py_2()
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(theme.danger)
                        .text_color(theme.danger_foreground)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .child(soft_wrap_dynamic_text(message)),
                        )
                        .child(
                            Button::new(("copy-error", row_ix))
                                .icon(IconName::Copy)
                                .tooltip("Copy error")
                                .small()
                                .ghost()
                                .invisible()
                                .group_hover("transcript-row", gpui::Styled::visible)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy_text.clone(),
                                    ));
                                }),
                        ),
                )
                .into_any_element()
        }
        TranscriptEntry::CommandOutput(text) => {
            let text_for_copy = text.clone();
            div()
                .w_full()
                .py_2()
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(theme.secondary)
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme.mono_font_size)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .max_h(px(320.))
                                .overflow_scrollbar()
                                .child(text.clone()),
                        )
                        .child(
                            Button::new(("copy-command-output", row_ix))
                                .icon(IconName::Copy)
                                .tooltip("Copy command output")
                                .small()
                                .ghost()
                                .invisible()
                                .group_hover("transcript-row", gpui::Styled::visible)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        text_for_copy.clone(),
                                    ));
                                }),
                        ),
                )
                .into_any_element()
        }
        TranscriptEntry::Compaction { phase } => {
            let (label, tint) = match phase {
                CompactionPhase::Started | CompactionPhase::Progress => {
                    ("Compacting…", theme.warning)
                }
                CompactionPhase::Completed => ("Compaction complete", theme.success),
                CompactionPhase::Failed => ("Compaction failed", theme.danger),
            };
            let label_for_copy = label.to_owned();
            div()
                .w_full()
                .py_2()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(theme.secondary)
                        .text_xs()
                        .text_color(tint)
                        .child(
                            h_flex()
                                .w_full()
                                .items_start()
                                .gap_2()
                                .child(div().flex_1().min_w_0().child(label))
                                .child(
                                    Button::new(("copy-compaction", row_ix))
                                        .icon(IconName::Copy)
                                        .tooltip("Copy compaction details")
                                        .small()
                                        .ghost()
                                        .invisible()
                                        .group_hover("transcript-row", gpui::Styled::visible)
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                label_for_copy.clone(),
                                            ));
                                        }),
                                ),
                        ),
                )
                .into_any_element()
        }
        TranscriptEntry::RetryInfo { detail } => {
            let detail_for_copy = detail.clone();
            let retrying =
                detail.starts_with("auto-retry started") || detail.starts_with("fallback applied");
            let tint = if retrying {
                theme.warning
            } else if detail.starts_with("fallback succeeded") {
                theme.success
            } else {
                theme.muted_foreground
            };
            div()
                .w_full()
                .py_2()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(theme.secondary)
                        .text_xs()
                        .text_color(tint)
                        .child(
                            h_flex()
                                .w_full()
                                .items_start()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(soft_wrap_dynamic_text(detail)),
                                )
                                .child(
                                    Button::new(("copy-retry-info", row_ix))
                                        .icon(IconName::Copy)
                                        .tooltip("Copy retry details")
                                        .small()
                                        .ghost()
                                        .invisible()
                                        .group_hover("transcript-row", gpui::Styled::visible)
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                detail_for_copy.clone(),
                                            ));
                                        }),
                                ),
                        ),
                )
                .into_any_element()
        }
        TranscriptEntry::Unknown { raw } => {
            if let Some(summary) = format_file_mention_summary(raw) {
                let summary_for_copy = summary.clone();
                return div()
                    .w_full()
                    .group("transcript-row")
                    .py_2()
                    .child(
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(theme.secondary)
                            .border_l_2()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(soft_wrap_dynamic_text(&summary)),
                            )
                            .child(
                                Button::new(("copy-file-mention", row_ix))
                                    .icon(IconName::Copy)
                                    .tooltip("Copy file mention")
                                    .small()
                                    .ghost()
                                    .invisible()
                                    .group_hover("transcript-row", gpui::Styled::visible)
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            summary_for_copy.clone(),
                                        ));
                                    }),
                            ),
                    )
                    .into_any_element();
            }
            let raw_for_copy = compact_json(raw);
            div()
                .w_full()
                .py_2()
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(theme.warning)
                        .text_color(theme.warning_foreground)
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(soft_wrap_dynamic_text(&format!("{raw:#}"))),
                        )
                        .child(
                            Button::new(("copy-unknown", row_ix))
                                .icon(IconName::Copy)
                                .tooltip("Copy raw frame")
                                .small()
                                .ghost()
                                .invisible()
                                .group_hover("transcript-row", gpui::Styled::visible)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        raw_for_copy.clone(),
                                    ));
                                }),
                        ),
                )
                .into_any_element()
        }
    };
    div()
        .w_full()
        .group("transcript-row")
        .child(row)
        .into_any_element()
}

pub(crate) fn code_block_copy_id(row_ix: usize, lang: Option<&str>, code: &str) -> ElementId {
    let mut hasher = DefaultHasher::new();
    (row_ix, lang, code).hash(&mut hasher);
    ElementId::Name(format!("code-block-copy-{}", hasher.finish()).into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HubJobSummary {
    pub(crate) op: Option<String>,
    pub(crate) jobs: Vec<HubJobSummaryRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HubJobSummaryRow {
    pub(crate) id: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvalCardSummary {
    pub(crate) title: String,
    pub(crate) digest: String,
}

fn nonempty_wire_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn wire_snippet(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut snippet = compact.chars().take(max_chars).collect::<String>();
    if compact.chars().count() > max_chars {
        snippet.push('…');
    }
    snippet
}

fn hub_job_command(job: &serde_json::Value) -> Option<String> {
    if let Some(command) = nonempty_wire_string(job.get("command").or_else(|| job.get("label"))) {
        return Some(wire_snippet(&command, 96));
    }

    let application = nonempty_wire_string(job.get("application"))?;
    let args = job
        .get("args")
        .and_then(serde_json::Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|args| !args.is_empty());
    Some(wire_snippet(
        &args.map_or(application.clone(), |args| format!("{application} {args}")),
        96,
    ))
}

/// Parse OMP's structured `hub jobs` result without deriving any missing state.
pub(crate) fn parse_hub_job_summary(
    tool_name: &str,
    args: &serde_json::Value,
    result: &serde_json::Value,
) -> Option<HubJobSummary> {
    if !tool_name.eq_ignore_ascii_case("hub") {
        return None;
    }

    let details = result.get("details").unwrap_or(result);
    let op =
        nonempty_wire_string(details.get("op")).or_else(|| nonempty_wire_string(args.get("op")));
    let jobs_value = details.get("jobs").or_else(|| result.get("jobs"));
    let is_jobs_op = op
        .as_deref()
        .is_some_and(|op| matches!(op.to_ascii_lowercase().as_str(), "jobs" | "wait" | "cancel"));
    if jobs_value.is_none() && !is_jobs_op {
        return None;
    }

    let jobs = jobs_value
        .and_then(serde_json::Value::as_array)
        .map(|jobs| {
            jobs.iter()
                .filter(|job| job.is_object())
                .map(|job| HubJobSummaryRow {
                    id: nonempty_wire_string(job.get("id").or_else(|| job.get("jobId"))),
                    status: nonempty_wire_string(job.get("status")),
                    command: hub_job_command(job),
                })
                .filter(|job| job.id.is_some() || job.status.is_some() || job.command.is_some())
                .collect()
        })
        .unwrap_or_default();

    Some(HubJobSummary { op, jobs })
}

fn eval_language_label(language: Option<&str>) -> Option<&'static str> {
    match language? {
        "py" | "python" => Some("Python"),
        "js" | "javascript" => Some("JavaScript"),
        "rb" | "ruby" => Some("Ruby"),
        "jl" | "julia" => Some("Julia"),
        _ => None,
    }
}

/// Derive a compact eval label from the title/language/code supplied by OMP.
pub(crate) fn parse_eval_card_summary(
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<EvalCardSummary> {
    if !tool_name.eq_ignore_ascii_case("eval") {
        return None;
    }

    let first_cell = args
        .get("cells")
        .and_then(serde_json::Value::as_array)
        .and_then(|cells| cells.iter().find(|cell| cell.is_object()));
    let title = nonempty_wire_string(args.get("title"))
        .or_else(|| first_cell.and_then(|cell| nonempty_wire_string(cell.get("title"))));
    let language = nonempty_wire_string(args.get("language"))
        .or_else(|| first_cell.and_then(|cell| nonempty_wire_string(cell.get("language"))));
    let code = nonempty_wire_string(args.get("code"))
        .or_else(|| first_cell.and_then(|cell| nonempty_wire_string(cell.get("code"))));
    let language_label = eval_language_label(language.as_deref());

    if title.is_none() && language_label.is_none() && code.is_none() {
        return None;
    }

    let heading = title
        .as_deref()
        .or(language_label)
        .map_or_else(|| "Eval".to_owned(), |label| format!("Eval · {label}"));
    let digest = [
        title
            .is_some()
            .then(|| language_label.map(str::to_owned))
            .flatten(),
        code.as_deref().map(|code| wire_snippet(code, 80)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");

    Some(EvalCardSummary {
        title: heading,
        digest,
    })
}

fn find_named_linkage(value: &serde_json::Value) -> Option<String> {
    const LINKAGE_KEYS: [&str; 4] = ["subagentId", "subagent_id", "toolCallId", "tool_call_id"];
    match value {
        serde_json::Value::Object(fields) => {
            for key in LINKAGE_KEYS {
                if let Some(id) = nonempty_wire_string(fields.get(key)) {
                    return Some(id);
                }
            }
            fields.values().find_map(find_named_linkage)
        }
        serde_json::Value::Array(values) => values.iter().find_map(find_named_linkage),
        _ => None,
    }
}

/// Return only an explicit task/subagent linkage field already present on wire data.
pub(crate) fn task_linkage_id(
    tool_name: &str,
    args: &serde_json::Value,
    result: &serde_json::Value,
) -> Option<String> {
    tool_name
        .eq_ignore_ascii_case("task")
        .then(|| find_named_linkage(args).or_else(|| find_named_linkage(result)))
        .flatten()
}

#[allow(clippy::too_many_lines)] // GPUI render fns are declaratively dense; splitting hurts readability.
pub(crate) fn render_tool_card(
    row_ix: usize,
    tc: &pimiento_core::transcript::ToolCall,
    group: ToolGroupPosition,
    expanded: bool,
    running_tool_started: &HashMap<String, Instant>,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let (status_color, status_foreground, status_label) = match tc.status {
        ToolStatus::Running => (theme.info, theme.info_foreground, "running"),
        ToolStatus::Ok => (theme.success, theme.success_foreground, "ok"),
        ToolStatus::Err => (theme.danger, theme.danger_foreground, "error"),
    };
    let output_text = tc.output.to_string();
    let has_output = !tc.output.is_empty();
    let args_text =
        serde_json::to_string_pretty(&tc.args_json).unwrap_or_else(|_| compact_json(&tc.args_json));
    let output_value = serde_json::from_str::<serde_json::Value>(&output_text)
        .unwrap_or_else(|_| serde_json::Value::String(output_text.clone()));
    let edit_diff = parse_edit_diff(&tc.name, &tc.args_json, &output_value).or_else(|| {
        // Fallback: treat plain tool output as a unified/compact diff body.
        let lines = parse_unified_diff_lines(&output_text);
        lines
            .iter()
            .any(|line| matches!(line.kind, DiffLineKind::Add | DiffLineKind::Remove))
            .then(|| pimiento_core::diff::EditDiffView {
                path: tc
                    .args_json
                    .get("path")
                    .or_else(|| tc.args_json.pointer("/input/path"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                op: None,
                lines,
            })
    });
    let hub_summary = parse_hub_job_summary(&tc.name, &tc.args_json, &output_value);
    let hub_lines = hub_summary
        .as_ref()
        .map(hub_job_summary_display_lines)
        .unwrap_or_default();
    let eval_summary = parse_eval_card_summary(&tc.name, &tc.args_json);
    let task_subagent = task_linkage_id(&tc.name, &tc.args_json, &output_value);
    let visual_kind = tool_visual_kind(&tc.name);
    let tool_title = if hub_summary.is_some() {
        "Jobs".to_owned()
    } else {
        eval_summary
            .as_ref()
            .map_or_else(|| tc.name.clone(), |summary| summary.title.clone())
    };
    let arg_digest: String = edit_diff
        .as_ref()
        .and_then(|diff| {
            diff.path.as_ref().map(|path| {
                format!(
                    "{}{}",
                    diff.op
                        .as_deref()
                        .map(|op| format!("{op} "))
                        .unwrap_or_default(),
                    path
                )
            })
        })
        .or_else(|| {
            eval_summary
                .as_ref()
                .map(|summary| summary.digest.clone())
                .filter(|digest| !digest.is_empty())
        })
        .or_else(|| hub_lines.first().cloned())
        .unwrap_or_else(|| wire_snippet(&tc.args_json.to_string(), 80));
    let duration_str = tc
        .duration_ms
        .map(|ms| format!("{}.{:03}s", ms / 1000, ms % 1000))
        .or_else(|| {
            (tc.status == ToolStatus::Running)
                .then(|| {
                    running_tool_started
                        .get(&tc.tool_call_id)
                        .map(|started| format_running_elapsed(started.elapsed()))
                })
                .flatten()
        })
        .unwrap_or_default();
    let tc_id = tc.tool_call_id.clone();
    let view = cx.entity().downgrade();
    let view_for_toggle = view.clone();
    let view_for_revert = view.clone();
    let view_for_agents = view.clone();
    let tc_id_for_toggle = tc_id.clone();

    v_flex()
        .w_full()
        .when(!group.grouped || group.first, gpui::Styled::pt_3)
        .when(!group.grouped, gpui::Styled::pb_3)
        .when(group.grouped, gpui::Styled::pb_1)
        .px_3()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .when(group.grouped && group.first, |card| {
            card.child(
                div()
                    .w_full()
                    .pb_1()
                    .mb_0p5()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child("Tools"),
            )
        })
        .child(
            h_flex()
                .w_full()
                .items_start()
                .gap_2()
                .child(
                    Icon::new(tool_icon(visual_kind))
                        .small()
                        .mt_0p5()
                        .text_color(tool_icon_color(visual_kind, &theme)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(soft_wrap_dynamic_text(&tool_title)),
                        )
                        .child(
                            div()
                                .w_full()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(soft_wrap_dynamic_text(&arg_digest)),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .bg(status_color)
                        .text_color(status_foreground)
                        .text_xs()
                        .child(status_label),
                )
                .when(!duration_str.is_empty(), |el| {
                    el.child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("·"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(duration_str),
                            ),
                    )
                })
                .child(
                    Button::new(("toggle-tool", row_ix))
                        .icon(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .tooltip(if expanded {
                            "Collapse tool"
                        } else {
                            "Expand tool"
                        })
                        .small()
                        .ghost()
                        .flex_shrink_0()
                        .on_click(move |_, _, cx| {
                            let _ = view_for_toggle.update(cx, |this, cx| {
                                this.toggle_tool_expanded(&tc_id_for_toggle, cx);
                            });
                        }),
                ),
        )
        .when(tc.status == ToolStatus::Running, |card| {
            card.child(
                Label::new("Cancel via turn Abort — per-tool cancel is not on the wire")
                    .text_xs()
                    .text_color(theme.muted_foreground),
            )
        })
        .when(!hub_lines.is_empty(), |card| {
            card.child(
                v_flex()
                    .w_full()
                    .gap_0p5()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .children(hub_lines.into_iter().map(|line| {
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(soft_wrap_dynamic_text(&line))
                    })),
            )
        })
        .when(expanded, |parent| {
            let args_for_copy = args_text.clone();
            parent.child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap_2()
                            .child(div().flex_1().min_w_0().text_xs().child("Arguments"))
                            .child(
                                Button::new(("copy-tool-args", row_ix))
                                    .icon(IconName::Copy)
                                    .tooltip("Copy arguments")
                                    .small()
                                    .ghost()
                                    .invisible()
                                    .group_hover("transcript-row", gpui::Styled::visible)
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            args_for_copy.clone(),
                                        ));
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_h(px(320.))
                            .overflow_scrollbar()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(theme.background)
                            .font_family(theme.mono_font_family.clone())
                            .text_size(theme.mono_font_size)
                            .child(args_text.clone()),
                    ),
            )
        })
        .when(expanded && has_output, |parent| {
            parent.child(if let Some(diff) = edit_diff.as_ref() {
                v_flex()
                    .w_full()
                    .max_h(px(320.))
                    .overflow_scrollbar()
                    .px_2()
                    .py_1()
                    .gap_0p5()
                    .rounded_sm()
                    .bg(theme.background)
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .children(diff.lines.iter().enumerate().map(|(ix, line)| {
                        let color = match line.kind {
                            DiffLineKind::Add => theme.success,
                            DiffLineKind::Remove => theme.danger,
                            DiffLineKind::Meta => theme.warning,
                            DiffLineKind::Context => theme.muted_foreground,
                        };
                        div()
                            .id(format!("diff-line-{row_ix}-{ix}"))
                            .text_color(color)
                            .child(line.text.clone())
                    }))
                    .into_any_element()
            } else {
                div()
                    .w_full()
                    .max_h(px(320.))
                    .overflow_scrollbar()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(theme.background)
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .child(output_text.clone())
                    .into_any_element()
            })
        })
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .when(has_output, |controls| {
                    controls.child(
                        Button::new(("copy-tool-output", row_ix))
                            .icon(IconName::Copy)
                            .tooltip("Copy output")
                            .small()
                            .ghost()
                            .invisible()
                            .group_hover("transcript-row", gpui::Styled::visible)
                            .on_click({
                                let output_text = output_text.clone();
                                move |_, _window, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        output_text.clone(),
                                    ));
                                }
                            }),
                    )
                })
                .when(task_subagent.is_some(), |controls| {
                    controls.child(
                        Button::new(("open-agents", row_ix))
                            .icon(IconName::Bot)
                            .tooltip("Open agents")
                            .small()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                let _ = view_for_agents.update(cx, |this, cx| {
                                    this.request_inspector_focus(PaletteActionId::ToggleAgents, cx);
                                });
                            }),
                    )
                })
                .children({
                    let revert_path =
                        edit_diff.as_ref().and_then(|d| d.path.clone()).or_else(|| {
                            tc.args_json
                                .get("path")
                                .or_else(|| tc.args_json.pointer("/input/path"))
                                .and_then(|v| v.as_str())
                                .map(str::to_owned)
                        });
                    revert_path.map(|path| {
                        let tc_id = tc_id.clone();
                        Button::new(format!("revert-tool-{tc_id}"))
                            .label("Revert file…")
                            .small()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                let path = path.clone();
                                let _ = view_for_revert.update(cx, |this, cx| {
                                    this.request_file_revert(path, cx);
                                });
                            })
                    })
                }),
        )
        .into_any_element()
}

// ── crash card ────────────────────────────────────────────────────────────

pub(crate) fn render_crash_card(
    status_message: &str,
    dead_reason: Option<&str>,
    can_restart: bool,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let detail = match dead_reason {
        Some(reason) if reason != status_message => format!("{reason}\n{status_message}"),
        Some(reason) => reason.to_owned(),
        None => status_message.to_owned(),
    };
    let detail_for_copy = detail.clone();

    v_flex()
        .w_full()
        .px_3()
        .py_2()
        .gap_2()
        .bg(theme.background)
        .border_t_1()
        .border_color(theme.border)
        .child(
            v_flex()
                .w_full()
                .p_3()
                .gap_2()
                .rounded_md()
                .bg(theme.secondary)
                .border_1()
                .border_color(theme.danger)
                .child(
                    Label::new("Session crashed")
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.danger),
                )
                .child(
                    div()
                        .w_full()
                        .max_h(px(240.))
                        .overflow_scrollbar()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(soft_wrap_dynamic_text(&detail)),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("crash-restart")
                                .primary()
                                .label("Restart")
                                .disabled(!can_restart)
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.do_restart(window, cx);
                                })),
                        )
                        .child(
                            Button::new("crash-copy")
                                .icon(IconName::Copy)
                                .tooltip("Copy crash details")
                                .small()
                                .ghost()
                                .on_click(move |_, _window, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        detail_for_copy.clone(),
                                    ));
                                }),
                        ),
                ),
        )
        .into_any_element()
}

// ── dialog rendering ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DialogOption {
    pub(crate) value: String,
    pub(crate) label: String,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DialogQuestion {
    pub(crate) header: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) options: Vec<DialogOption>,
    pub(crate) recommended: Option<usize>,
}

fn nonempty_dialog_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn parse_dialog_options(payload: &serde_json::Value) -> Vec<DialogOption> {
    payload
        .get("options")
        .and_then(serde_json::Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    if let Some(value) = option.as_str() {
                        return Some(DialogOption {
                            value: value.to_owned(),
                            label: value.to_owned(),
                            description: None,
                        });
                    }
                    let label = nonempty_dialog_string(
                        option
                            .get("label")
                            .or_else(|| option.get("title"))
                            .or_else(|| option.get("value")),
                    )?;
                    let value =
                        nonempty_dialog_string(option.get("value").or_else(|| option.get("id")))
                            .unwrap_or_else(|| label.clone());
                    let description = nonempty_dialog_string(
                        option.get("description").or_else(|| option.get("preview")),
                    );
                    Some(DialogOption {
                        value,
                        label,
                        description,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn dialog_recommended_index(
    payload: &serde_json::Value,
    options: &[DialogOption],
) -> Option<usize> {
    let recommended = payload
        .get("recommended")
        .or_else(|| payload.get("recommendedIndex"));
    if let Some(index) = recommended
        .and_then(serde_json::Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .filter(|index| *index < options.len())
    {
        return Some(index);
    }
    if let Some(value) = nonempty_dialog_string(recommended) {
        return options
            .iter()
            .position(|option| option.value == value || option.label == value);
    }
    payload
        .get("options")
        .and_then(serde_json::Value::as_array)
        .and_then(|raw_options| {
            raw_options.iter().enumerate().find_map(|(index, option)| {
                option
                    .get("recommended")
                    .and_then(serde_json::Value::as_bool)
                    .is_some_and(|recommended| recommended)
                    .then_some(index)
            })
        })
        .filter(|index| *index < options.len())
}

pub(crate) fn dialog_questions(dialog: &UiDialog) -> Vec<DialogQuestion> {
    let nested = dialog
        .payload
        .get("questions")
        .and_then(serde_json::Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .filter(|question| question.is_object())
                .map(|question| {
                    let options = parse_dialog_options(question);
                    DialogQuestion {
                        header: nonempty_dialog_string(
                            question.get("header").or_else(|| question.get("title")),
                        ),
                        prompt: nonempty_dialog_string(
                            question.get("question").or_else(|| question.get("message")),
                        ),
                        description: nonempty_dialog_string(question.get("description")),
                        recommended: dialog_recommended_index(question, &options),
                        options,
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !nested.is_empty() {
        return nested;
    }

    let options = parse_dialog_options(&dialog.payload);
    vec![DialogQuestion {
        header: None,
        prompt: None,
        description: None,
        recommended: dialog_recommended_index(&dialog.payload, &options),
        options,
    }]
}

pub(crate) fn dialog_primary_options(dialog: &UiDialog) -> Vec<DialogOption> {
    dialog_questions(dialog)
        .into_iter()
        .find(|question| !question.options.is_empty())
        .map(|question| question.options)
        .unwrap_or_default()
}

pub(crate) fn render_dialog(dialog: &UiDialog, cx: &mut Context<SessionView>) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let title = dialog
        .payload
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&dialog.method);
    v_flex()
        .w_full()
        .p_4()
        .gap_3()
        .rounded_md()
        .bg(theme.secondary)
        .border_1()
        .border_color(theme.border)
        .child(
            Label::new(soft_wrap_dynamic_text(title))
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD),
        )
        .when_some(
            dialog
                .payload
                .get("message")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            |parent, msg| {
                parent.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(soft_wrap_dynamic_text(&msg)),
                )
            },
        )
        .child(match dialog.method.as_str() {
            "select" => render_select_dialog(dialog, &dialog_questions(dialog), cx),
            "confirm" => render_confirm_dialog(dialog, cx),
            "input" | "editor" => render_text_dialog(dialog, cx),
            "open_url" => render_open_url_dialog(dialog, cx),
            _ => render_cancel_button(dialog, cx),
        })
        .into_any_element()
}

#[allow(clippy::too_many_lines)] // Options keep full wrapping content and response wiring together.
pub(crate) fn render_select_dialog(
    dialog: &UiDialog,
    questions: &[DialogQuestion],
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let view = cx.entity().downgrade();
    let mut blocks = v_flex().w_full().gap_3();
    for (question_ix, question) in questions.iter().enumerate() {
        let mut block = v_flex().w_full().gap_1();
        if let Some(header) = &question.header {
            block = block.child(
                Label::new(soft_wrap_dynamic_text(header))
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD),
            );
        }
        if let Some(prompt) = &question.prompt {
            block = block.child(div().text_sm().child(soft_wrap_dynamic_text(prompt)));
        }
        if let Some(description) = &question.description {
            block = block.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(soft_wrap_dynamic_text(description)),
            );
        }
        for (option_ix, option) in question.options.iter().take(9).enumerate() {
            let value = option.value.clone();
            let id = dialog.id.clone();
            let view = view.clone();
            let is_recommended = question.recommended == Some(option_ix);
            block = block.child(
                h_flex()
                    .id(format!("dialog-option-{question_ix}-{option_ix}"))
                    .w_full()
                    .items_start()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        let mut fields = serde_json::Map::new();
                        fields.insert("value".into(), serde_json::Value::String(value.clone()));
                        do_dialog_response(&view, &id, fields, cx);
                    })
                    .child(
                        div()
                            .w(px(20.))
                            .flex_shrink_0()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child((option_ix + 1).to_string()),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().text_sm().child(soft_wrap_dynamic_text(&option.label)))
                            .when_some(option.description.clone(), |option, description| {
                                option.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(soft_wrap_dynamic_text(&description)),
                                )
                            }),
                    )
                    .when(is_recommended, |row| {
                        row.child(
                            Tag::secondary()
                                .small()
                                .flex_shrink_0()
                                .child("Recommended"),
                        )
                    }),
            );
        }
        blocks = blocks.child(block);
    }
    blocks
        .child(
            h_flex()
                .w_full()
                .items_start()
                .flex_wrap()
                .gap_2()
                .justify_between()
                .child(
                    Label::new("Press 1–9 · Esc cancel")
                        .text_xs()
                        .text_color(theme.muted_foreground),
                )
                .child({
                    let view = view.clone();
                    let id = dialog.id.clone();
                    Button::new("cancel-select")
                        .label("Cancel")
                        .small()
                        .ghost()
                        .on_click(move |_, _, cx| do_cancel_dialog(&view, &id, cx))
                }),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)] // Two structured choices keep labels, descriptions, tags, and callbacks together.
pub(crate) fn render_confirm_dialog(
    dialog: &UiDialog,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let parsed_options = parse_dialog_options(&dialog.payload);
    let recommended = dialog_recommended_index(&dialog.payload, &parsed_options);
    let yes_label = parsed_options
        .first()
        .map_or_else(|| "Yes".to_owned(), |option| option.label.clone());
    let no_label = parsed_options
        .get(1)
        .map_or_else(|| "No".to_owned(), |option| option.label.clone());
    let yes_description = parsed_options
        .first()
        .and_then(|option| option.description.clone());
    let no_description = parsed_options
        .get(1)
        .and_then(|option| option.description.clone());
    let view = cx.entity().downgrade();
    let id_yes = dialog.id.clone();
    let id_no = dialog.id.clone();
    v_flex()
        .w_full()
        .gap_2()
        .child({
            let view = view.clone();
            h_flex()
                .id("confirm-yes")
                .w_full()
                .items_start()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_sm()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    let mut fields = serde_json::Map::new();
                    fields.insert("confirmed".into(), serde_json::Value::Bool(true));
                    do_dialog_response(&view, &id_yes, fields, cx);
                })
                .child(
                    div()
                        .w(px(20.))
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child("Y"),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().text_sm().child(soft_wrap_dynamic_text(&yes_label)))
                        .when_some(yes_description, |option, description| {
                            option.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(soft_wrap_dynamic_text(&description)),
                            )
                        }),
                )
                .when(recommended == Some(0), |row| {
                    row.child(
                        Tag::secondary()
                            .small()
                            .flex_shrink_0()
                            .child("Recommended"),
                    )
                })
        })
        .child({
            let view = view.clone();
            h_flex()
                .id("confirm-no")
                .w_full()
                .items_start()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_sm()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    let mut fields = serde_json::Map::new();
                    fields.insert("confirmed".into(), serde_json::Value::Bool(false));
                    do_dialog_response(&view, &id_no, fields, cx);
                })
                .child(
                    div()
                        .w(px(20.))
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child("N"),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().text_sm().child(soft_wrap_dynamic_text(&no_label)))
                        .when_some(no_description, |option, description| {
                            option.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(soft_wrap_dynamic_text(&description)),
                            )
                        }),
                )
                .when(recommended == Some(1), |row| {
                    row.child(
                        Tag::secondary()
                            .small()
                            .flex_shrink_0()
                            .child("Recommended"),
                    )
                })
        })
        .child(
            Label::new("Press Y/N · Esc cancel")
                .text_xs()
                .text_color(theme.muted_foreground),
        )
        .into_any_element()
}

pub(crate) fn open_url_target(dialog: &UiDialog) -> Option<String> {
    dialog
        .payload
        .get("url")
        .or_else(|| dialog.payload.get("launchUrl"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) fn open_url_in_os_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    let _ = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub(crate) fn render_open_url_dialog(
    dialog: &UiDialog,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let url = open_url_target(dialog).unwrap_or_default();
    let instructions = dialog
        .payload
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or("Open this URL to continue (e.g. OAuth login).");
    let id = dialog.id.clone();
    v_flex()
        .w_full()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(soft_wrap_dynamic_text(instructions)),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .overflow_x_scrollbar()
                .text_xs()
                .font_family("Menlo")
                .child(url.clone()),
        )
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .justify_end()
                .gap_2()
                .child(render_cancel_button(dialog, cx))
                .child({
                    let url_c = url.clone();
                    Button::new(format!("copy-url-{id}"))
                        .label("Copy URL")
                        .small()
                        .ghost()
                        .disabled(url.is_empty())
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(url_c.clone()));
                        })
                })
                .child({
                    let url_c = url.clone();
                    Button::new(format!("open-url-{id}"))
                        .label("Open")
                        .small()
                        .primary()
                        .disabled(url.is_empty())
                        .on_click(move |_, _, _cx| {
                            open_url_in_os_browser(&url_c);
                        })
                }),
        )
        .into_any_element()
}

pub(crate) fn render_text_dialog(
    dialog: &UiDialog,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let view = cx.entity().downgrade();
    let id = dialog.id.clone();
    let id_submit = dialog.id.clone();
    let dialog_input = cx.entity().read(cx).dialog_input.clone();
    h_flex()
        .w_full()
        .flex_wrap()
        .gap_2()
        .items_end()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .px_2()
                .py_1()
                .child(
                    Input::new(&dialog_input)
                        .appearance(false)
                        .focus_bordered(false),
                ),
        )
        .child({
            let view = view.clone();
            Button::new("dialog-submit")
                .primary()
                .label("Submit")
                .small()
                .on_click(move |_, _, cx| {
                    let Some(entity) = view.upgrade() else {
                        return;
                    };
                    let value = entity.read(cx).dialog_input.read(cx).value().to_string();
                    let mut fields = serde_json::Map::new();
                    fields.insert("value".into(), serde_json::Value::String(value));
                    do_dialog_response(&view, &id_submit, fields, cx);
                })
        })
        .child({
            let view = view.clone();
            Button::new("cancel-text-dialog")
                .label("Cancel")
                .small()
                .ghost()
                .on_click(move |_, _, cx| do_cancel_dialog(&view, &id, cx))
        })
        .into_any_element()
}

pub(crate) fn render_cancel_button(
    dialog: &UiDialog,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let view = cx.entity().downgrade();
    let id = dialog.id.clone();
    Button::new("cancel-dialog")
        .label("Cancel")
        .small()
        .ghost()
        .on_click(move |_, _, cx| do_cancel_dialog(&view, &id, cx))
        .into_any_element()
}

/// OMP `extension_ui_response` cancel payload (`cancelled` + optional `timedOut`).
pub(crate) fn dialog_cancel_fields(timed_out: bool) -> serde_json::Map<String, serde_json::Value> {
    let mut fields = serde_json::Map::new();
    fields.insert("cancelled".into(), serde_json::Value::Bool(true));
    if timed_out {
        fields.insert("timedOut".into(), serde_json::Value::Bool(true));
    }
    fields
}

pub(crate) fn do_cancel_dialog(view: &gpui::WeakEntity<SessionView>, id: &str, cx: &mut gpui::App) {
    do_dialog_response(view, id, dialog_cancel_fields(false), cx);
}

/// OMP often emits tool-mount chatter as notices; keep the text, drop the label.
pub(crate) fn notice_looks_like_mount_event(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("mounted") && (lower.contains("mcp") || lower.contains("tool"))
}

/// Collapsed thinking cue: first non-empty line, truncated — wire text only.
pub(crate) fn thinking_collapse_preview(text: &str) -> String {
    const MAX_CHARS: usize = 56;
    let preview = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if preview.is_empty() {
        return "Thinking · expand".to_owned();
    }
    let mut truncated = preview.chars().take(MAX_CHARS).collect::<String>();
    if preview.chars().count() > MAX_CHARS {
        truncated.push('…');
    }
    format!("Thinking · {truncated}")
}

pub(crate) fn do_dialog_response(
    view: &gpui::WeakEntity<SessionView>,
    id: &str,
    fields: serde_json::Map<String, serde_json::Value>,
    cx: &mut gpui::App,
) {
    let id_owned = id.to_owned();
    let Some(entity) = view.upgrade() else { return };
    if let Some((client, _)) = entity.read(cx).client_and_dialog_id(id) {
        cx.spawn(async move |_| {
            let _ = client
                .send(RpcCommandBody::ExtensionUiResponse {
                    id: id_owned.clone(),
                    fields,
                })
                .await;
        })
        .detach();
        let id2 = id.to_owned();
        let _ = view.update(cx, |this, cx| {
            this.projection.pending_dialogs.retain(|d| d.id != id2);
            this.sync_pending_dialogs(cx);
            cx.notify();
        });
    }
}

pub(crate) fn hub_job_summary_display_lines(summary: &HubJobSummary) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(op) = &summary.op {
        lines.push(format!("op: {op}"));
    }
    for job in &summary.jobs {
        let mut parts = Vec::new();
        if let Some(id) = &job.id {
            parts.push(format!("job: {id}"));
        }
        if let Some(status) = &job.status {
            parts.push(status.clone());
        }
        if let Some(command) = &job.command {
            parts.push(command.clone());
        }
        if !parts.is_empty() {
            lines.push(parts.join(" · "));
        }
    }
    lines
}
