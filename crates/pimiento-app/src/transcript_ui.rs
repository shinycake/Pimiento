use crate::*;

#[allow(clippy::too_many_lines)] // Match arms mirror transcript variants.
pub(crate) fn render_entry(
    row_ix: usize,
    entry: &TranscriptEntry,
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
                .gap_2()
                .py_2()
                .child(
                    div()
                        .flex_1()
                        .border_l_2()
                        .border_color(theme.accent)
                        .pl_4()
                        .pr_3()
                        .py_1p5()
                        .child(text.clone()),
                )
                .child(
                    Button::new(("copy-user", row_ix))
                        .label("Copy")
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
        TranscriptEntry::AssistantText { markdown, .. } => {
            let markdown_for_copy = markdown.as_str().to_owned();
            h_flex()
                .w_full()
                .gap_2()
                .py_2()
                .child(
                    div().flex_1().child(
                        TextView::markdown(("assistant", row_ix), markdown.as_str())
                            .selectable(true)
                            .code_block_actions(move |code_block, _, _cx| {
                                let code = code_block.code().to_string();
                                let lang = code_block.lang().map(|lang| lang.to_string());
                                Button::new(code_block_copy_id(row_ix, lang.as_deref(), &code))
                                    .label("Copy")
                                    .small()
                                    .ghost()
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            code.clone(),
                                        ));
                                    })
                            }),
                    ),
                )
                .child(
                    Button::new(("copy-assistant", row_ix))
                        .label("Copy")
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
            collapsed: true,
            text,
            ..
        } => {
            let view = cx.entity().downgrade();
            let text_for_copy = text.clone();
            h_flex()
                .w_full()
                .gap_2()
                .py_2()
                .child(
                    div()
                        .id(("thinking-collapsed", row_ix))
                        .flex_1()
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.toggle_thinking_row(row_ix, cx);
                            });
                        })
                        .child(
                            div()
                                .py_1()
                                .text_color(theme.muted_foreground)
                                .text_xs()
                                .italic()
                                .child("Thinking · click to expand"),
                        ),
                )
                .child(
                    Button::new(("copy-thinking", row_ix))
                        .label("Copy")
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
                                .gap_2()
                                .child(
                                    Button::new(("thinking-collapse", row_ix))
                                        .label("collapse thinking")
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
                                        .label("Copy")
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
                                .bg(theme.muted)
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
            expanded.contains(&tc.tool_call_id),
            running_tool_started,
            cx,
        ),
        TranscriptEntry::Notice(text) => {
            let text_for_copy = text.clone();
            div()
                .w_full()
                .py_1()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(text.clone()),
                        )
                        .child(
                            Button::new(("copy-notice", row_ix))
                                .label("Copy")
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
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(theme.danger)
                        .child(div().flex_1().text_sm().child(message.clone()))
                        .child(
                            Button::new(("copy-error", row_ix))
                                .label("Copy")
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
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(theme.muted)
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme.mono_font_size)
                        .child(div().flex_1().child(text.clone()))
                        .child(
                            Button::new(("copy-command-output", row_ix))
                                .label("Copy")
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
                        .bg(theme.muted)
                        .text_xs()
                        .text_color(tint)
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(div().flex_1().child(label))
                                .child(
                                    Button::new(("copy-compaction", row_ix))
                                        .label("Copy")
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
                        .bg(theme.muted)
                        .text_xs()
                        .text_color(tint)
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(div().flex_1().child(detail.clone()))
                                .child(
                                    Button::new(("copy-retry-info", row_ix))
                                        .label("Copy")
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
            let raw_for_copy = compact_json(raw);
            div()
                .w_full()
                .py_2()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(theme.warning)
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .child(div().flex_1().child(format!("{raw:#}")))
                        .child(
                            Button::new(("copy-unknown", row_ix))
                                .label("Copy")
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

#[allow(clippy::too_many_lines)] // GPUI render fns are declaratively dense; splitting hurts readability.
pub(crate) fn render_tool_card(
    row_ix: usize,
    tc: &pimiento_core::transcript::ToolCall,
    expanded: bool,
    running_tool_started: &HashMap<String, Instant>,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let (status_color, status_label) = match tc.status {
        ToolStatus::Running => (theme.info, "running"),
        ToolStatus::Ok => (theme.success, "ok"),
        ToolStatus::Err => (theme.danger, "error"),
    };
    let output_text = tc.output.to_string();
    let has_output = !tc.output.is_empty();
    let args_text = compact_json(&tc.args_json);
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
        .unwrap_or_else(|| tc.args_json.to_string().chars().take(80).collect());
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

    v_flex()
        .w_full()
        .py_2()
        .gap_0p5()
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .bg(status_color)
                        .text_xs()
                        .child(status_label),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(tc.name.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(arg_digest),
                )
                .when(!duration_str.is_empty(), |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(duration_str),
                    )
                }),
        )
        .when(expanded, |parent| {
            let args_for_copy = args_text.clone();
            parent.child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(div().flex_1().text_xs().child("Arguments"))
                            .child(
                                Button::new(("copy-tool-args", row_ix))
                                    .label("Copy")
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
                            .overflow_y_scrollbar()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(theme.muted)
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
                    .overflow_y_scrollbar()
                    .px_2()
                    .py_1()
                    .gap_0p5()
                    .rounded_sm()
                    .bg(theme.muted)
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
                    .overflow_y_scrollbar()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(theme.muted)
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .child(output_text.clone())
                    .into_any_element()
            })
        })
        .child(
            h_flex()
                .gap_2()
                .child({
                    let tc_id = tc_id.clone();
                    Button::new(("toggle-tool", row_ix))
                        .label(if expanded {
                            "▲ collapse"
                        } else {
                            "▼ details"
                        })
                        .small()
                        .ghost()
                        .on_click(move |_, _, cx| {
                            let _ = view_for_toggle
                                .update(cx, |this, cx| this.toggle_tool_expanded(&tc_id, cx));
                        })
                })
                .when(has_output, |controls| {
                    controls.child(
                        Button::new(("copy-tool-output", row_ix))
                            .label("Copy")
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
                                let tc_id = tc_id.clone();
                                let _ = view_for_revert.update(cx, |this, cx| {
                                    this.request_file_revert(path, tc_id, cx);
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
    let status_copy = status_message.to_owned();
    let detail = match dead_reason {
        Some(reason) if reason != status_message => format!("{reason}\n{status_message}"),
        Some(reason) => reason.to_owned(),
        None => status_message.to_owned(),
    };

    v_flex()
        .w_full()
        .px_3()
        .py_2()
        .gap_2()
        .bg(theme.muted)
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
                        .text_color(theme.danger),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(detail),
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
                                .label("Copy")
                                .small()
                                .ghost()
                                .on_click(move |_, _window, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        status_copy.clone(),
                                    ));
                                }),
                        ),
                ),
        )
        .into_any_element()
}

// ── dialog rendering ──────────────────────────────────────────────────────

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
            Label::new(gpui::SharedString::from(title.to_owned()))
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
                        .child(msg),
                )
            },
        )
        .child(match dialog.method.as_str() {
            "select" => render_select_dialog(dialog, &select_dialog_options(dialog), cx),
            "confirm" => render_confirm_dialog(dialog, cx),
            "open_url" => render_open_url_dialog(dialog, cx),
            _ => render_cancel_button(dialog, cx),
        })
        .into_any_element()
}

pub(crate) fn render_select_dialog(
    dialog: &UiDialog,
    options: &[String],
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let mut el = h_flex().flex_wrap().gap_2();
    let view = cx.entity().downgrade();
    for (i, opt) in options.iter().enumerate() {
        let opt = opt.clone();
        let id = dialog.id.clone();
        let key_hint = match i {
            0 => "1 ⏎ ",
            n if n < 9 => &format!("{} ", n + 1),
            _ => "",
        };
        el = el.child({
            let view = view.clone();
            Button::new(format!("opt-{i}"))
                .label(format!("{key_hint}{opt}"))
                .small()
                .on_click(move |_, _, cx| {
                    let mut fields = serde_json::Map::new();
                    fields.insert("value".into(), serde_json::Value::String(opt.clone()));
                    do_dialog_response(&view, &id, fields, cx);
                })
        });
    }
    el.child({
        let view = view.clone();
        let id = dialog.id.clone();
        Button::new("cancel-select")
            .label("Esc")
            .small()
            .ghost()
            .on_click(move |_, _, cx| do_cancel_dialog(&view, &id, cx))
    })
    .into_any_element()
}

pub(crate) fn render_confirm_dialog(
    dialog: &UiDialog,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let view = cx.entity().downgrade();
    let id_yes = dialog.id.clone();
    let id_no = dialog.id.clone();
    h_flex()
        .w_full()
        .justify_end()
        .gap_2()
        .child({
            let view = view.clone();
            Button::new("confirm-no")
                .label("N No")
                .small()
                .ghost()
                .on_click(move |_, _, cx| {
                    let mut fields = serde_json::Map::new();
                    fields.insert("accepted".into(), serde_json::Value::Bool(false));
                    do_dialog_response(&view, &id_no, fields, cx);
                })
        })
        .child({
            let view = view.clone();
            Button::new("confirm-yes")
                .primary()
                .label("Y ⏎ Yes")
                .small()
                .on_click(move |_, _, cx| {
                    let mut fields = serde_json::Map::new();
                    fields.insert("accepted".into(), serde_json::Value::Bool(true));
                    do_dialog_response(&view, &id_yes, fields, cx);
                })
        })
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
                .child(instructions.to_owned()),
        )
        .child(div().text_xs().font_family("Menlo").child(url.clone()))
        .child(
            h_flex()
                .w_full()
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

pub(crate) fn do_cancel_dialog(view: &gpui::WeakEntity<SessionView>, id: &str, cx: &mut gpui::App) {
    let mut fields = serde_json::Map::new();
    fields.insert("cancel".into(), serde_json::Value::Bool(true));
    do_dialog_response(view, id, fields, cx);
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
            cx.notify();
        });
    }
}
