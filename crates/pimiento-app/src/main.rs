#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Pimiento — first live OMP session workspace.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    App, ClickEvent, Context, FollowMode, KeyDownEvent, ListAlignment, ListState, Render, Task,
    Window, WindowOptions, div, list, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Root, Sizable as _, Theme, ThemeMode,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    label::Label,
    scroll::ScrollableElement as _,
    text::TextView,
    v_flex,
};
use omp_rpc_client::{
    client::{ClientConfig, ClientEvent, RpcClient},
    discovery::{DiscoveryInputs, SystemRunner, discover},
    frames::{RpcCommandBody, SubagentSubscriptionLevel},
};
use pimiento_core::{
    projection::{RunPhase, SessionProjection, UiDialog, format_model_label, split_model_label},
    transcript::{ToolStatus, TranscriptEntry},
};

// ── theme toggle ──────────────────────────────────────────────────────────

fn next_theme_mode(current: ThemeMode) -> ThemeMode {
    if current.is_dark() {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    }
}

fn toggle_theme(_: &ClickEvent, window: &mut Window, cx: &mut App) {
    let next = next_theme_mode(cx.theme().mode);
    Theme::change(next, Some(window), cx);
}

// ── SessionView ───────────────────────────────────────────────────────────

/// `(provider, model_id)` pair from `get_available_models`.
type ModelChoice = (String, String);

const MODEL_PICKER_VISIBLE_CAP: usize = 200;

struct SessionView {
    projection: SessionProjection,
    client: Option<RpcClient>,
    composer: gpui::Entity<InputState>,
    model_search: gpui::Entity<InputState>,
    model_picker_open: bool,
    status_message: String,
    available_models: Vec<ModelChoice>,
    expanded_tools: HashSet<String>,
    clear_composer: bool,
    clear_model_search: bool,
    /// Virtualized transcript list (GPUI `ListState`, bottom-aligned chat).
    transcript_list: ListState,
    last_transcript_len: usize,
    /// Count of rows appended while the user was scrolled away from the tail.
    unread_below: usize,
    _subscriptions: Vec<gpui::Subscription>,
    pump: Option<Task<()>>,
}

impl SessionView {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        client: Option<RpcClient>,
        status: String,
        initial_projection: SessionProjection,
        available_models: Vec<ModelChoice>,
    ) -> Self {
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("Type… Enter send (steers while streaming); Shift+Enter newline")
        });
        let model_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search models… (or provider/id, Enter)")
                .submit_on_enter(true)
        });

        let subscriptions = vec![
            cx.subscribe(&composer, Self::on_composer_event),
            cx.subscribe(&model_search, Self::on_model_search_event),
        ];

        let initial_len = initial_projection.transcript.len();
        let transcript_list = ListState::new(initial_len, ListAlignment::Bottom, px(400.));
        transcript_list.set_follow_mode(FollowMode::Tail);
        {
            let weak = cx.weak_entity();
            transcript_list.set_scroll_handler(move |ev, _window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if ev.is_following_tail {
                        this.unread_below = 0;
                    }
                    cx.notify();
                });
            });
        }

        // Start foreground pump if we have a live client
        let pump = client.as_ref().map(|c| {
            let events = c.events();
            cx.spawn(async move |view, cx| {
                while let Ok(event) = events.recv().await {
                    let _ = view.update(cx, |this, cx| {
                        match &event {
                            ClientEvent::Frame(frame) => {
                                let is_model_changed =
                                    frame.raw.get("type").and_then(|v| v.as_str())
                                        == Some("model_changed");
                                this.projection.apply(frame);
                                if is_model_changed {
                                    this.refresh_state_after_model_change(cx);
                                }
                            }
                            ClientEvent::Closed(info) => {
                                let reason = info
                                    .error_msg
                                    .clone()
                                    .unwrap_or_else(|| format!("exit code {:?}", info.exit_code));
                                this.projection.mark_dead(reason);
                                this.client = None;
                                let tail = &info.stderr_tail;
                                this.status_message =
                                    format!("OMP closed — {}", &tail[..256.min(tail.len())]);
                            }
                        }
                        cx.notify();
                    });
                }
            })
        });

        let mut view = Self {
            projection: initial_projection,
            client,
            composer,
            model_search,
            model_picker_open: false,
            status_message: status,
            available_models,
            expanded_tools: HashSet::new(),
            clear_composer: false,
            clear_model_search: false,
            transcript_list,
            last_transcript_len: initial_len,
            unread_below: 0,
            _subscriptions: subscriptions,
            pump,
        };
        view.start_catalog_load(cx);
        view
    }

    /// Keep `ListState` item count / measurements in sync with the projection.
    ///
    /// PLAN SH: `splice`/`reset` on count changes; `remeasure_items` when a
    /// row's height may have changed (streaming growth, card expand).
    fn sync_transcript_list(&mut self) {
        let new_len = self.projection.transcript.len();
        let old_len = self.last_transcript_len;
        if new_len > old_len {
            self.transcript_list
                .splice(old_len..old_len, new_len - old_len);
            if self.transcript_list.is_following_tail() {
                self.unread_below = 0;
            } else {
                self.unread_below = self.unread_below.saturating_add(new_len - old_len);
            }
        } else if new_len < old_len {
            self.transcript_list.reset(new_len);
            self.unread_below = 0;
        }
        if new_len > 0 {
            let start = new_len.saturating_sub(4);
            self.transcript_list.remeasure_items(start..new_len);
        }
        self.last_transcript_len = new_len;
    }

    fn jump_to_transcript_tail(&mut self, cx: &mut Context<Self>) {
        self.transcript_list.scroll_to_end();
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        self.unread_below = 0;
        cx.notify();
    }

    fn start_catalog_load(&mut self, cx: &mut Context<Self>) {
        if !self.available_models.is_empty() {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        let current = self.projection.state.model.clone();
        cx.spawn(async move |view, cx| {
            let resp = client
                .send_with_timeout(RpcCommandBody::GetAvailableModels, Duration::from_mins(3))
                .await;
            let catalog = match resp {
                Ok(r) if r.success => r
                    .data
                    .unwrap_or_else(|| serde_json::json!({ "models": [] })),
                _ => return,
            };
            let models = load_all_model_choices(&catalog, current.as_deref());
            let _ = view.update(cx, |this, cx| {
                this.available_models = models;
                cx.notify();
            });
        })
        .detach();
    }

    fn close_model_picker(&mut self, _cx: &mut Context<Self>) {
        self.model_picker_open = false;
        self.clear_model_search = true;
    }

    fn toggle_model_picker(&mut self, cx: &mut Context<Self>) {
        self.model_picker_open = !self.model_picker_open;
        if !self.model_picker_open {
            self.clear_model_search = true;
        }
        cx.notify();
    }

    fn pick_model_from_search(&mut self, cx: &mut Context<Self>) {
        let query = self.model_search.read(cx).value().to_string();
        let filtered = filter_models(&self.available_models, &query);
        let choice = filtered
            .first()
            .cloned()
            .or_else(|| split_model_label(query.trim()));
        let Some((provider, id)) = choice else {
            return;
        };
        self.model_picker_open = false;
        self.clear_model_search = true;
        self.set_model(provider, id, cx);
    }

    fn refresh_state_after_model_change(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn(async move |view, cx| {
            if let Ok(resp) = client.send(RpcCommandBody::GetState).await
                && resp.success
                && let Some(data) = resp.data
            {
                let _ = view.update(cx, |this, cx| {
                    this.projection.hydrate_get_state(&data);
                    this.sync_status_model();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn sync_status_model(&mut self) {
        // Keep the version prefix; refresh model / thinking / context suffix.
        let model = self
            .projection
            .state
            .model
            .as_deref()
            .unwrap_or("(no model)");
        let thinking = thinking_label(self.projection.state.thinking.as_ref());
        let ctx = context_percent_label(self.projection.state.context.as_ref());
        let tps = tokens_per_second_label(self.projection.state.tokens.as_ref());
        if let Some((ver, _)) = self.status_message.split_once("  •  ") {
            let mut parts = vec![ver.to_owned(), model.to_owned()];
            if let Some(t) = thinking {
                parts.push(format!("think:{t}"));
            }
            if let Some(c) = ctx {
                parts.push(format!("ctx:{c}"));
            }
            if let Some(t) = tps {
                parts.push(format!("{t}/s"));
            }
            self.status_message = parts.join("  •  ");
        }
    }

    fn set_model(&mut self, provider: String, model_id: String, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let label = format!("{provider}/{model_id}");
        // Optimistic display; corrected from response / get_state refresh.
        self.projection.state.model = Some(label.clone());
        self.sync_status_model();
        cx.notify();

        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::SetModel {
                    provider: provider.clone(),
                    model_id: model_id.clone(),
                })
                .await
            {
                Ok(resp) if resp.success => {
                    let label = resp
                        .data
                        .as_ref()
                        .and_then(format_model_label)
                        .unwrap_or_else(|| format!("{provider}/{model_id}"));
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.model = Some(label);
                        this.sync_status_model();
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let err = resp
                        .error
                        .clone()
                        .unwrap_or_else(|| "set_model failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: err,
                            code: Some("set_model".into()),
                        });
                        cx.notify();
                    });
                }
                Err(e) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("set_model: {e}"),
                            code: Some("set_model".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn on_model_search_event(
        &mut self,
        _input: gpui::Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter {
                secondary: false,
                shift: false,
            } => self.pick_model_from_search(cx),
            _ => {}
        }
    }

    // ── composer ──────────────────────────────────────────────────────

    fn on_composer_event(
        &mut self,
        _input: gpui::Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::PressEnter {
            secondary: false,
            shift: false,
        } = event
        {
            let text = self.composer.read(cx).value().to_string();
            if text.trim().is_empty() {
                return;
            }
            let Some(client) = self.client.clone() else {
                return;
            };

            let steer = composer_uses_steer(&self.projection.run_phase);
            self.projection.push_user_message(text.clone());
            self.clear_composer = true;
            cx.notify();

            cx.spawn(async move |_, _| {
                let body = if steer {
                    RpcCommandBody::Steer {
                        message: text,
                        images: None,
                    }
                } else {
                    RpcCommandBody::Prompt {
                        message: text,
                        images: None,
                        streaming_behavior: None,
                    }
                };
                let _ = client.send(body).await;
            })
            .detach();
        }
    }

    fn can_send(&self) -> bool {
        self.client.is_some() && phase_allows_send(&self.projection.run_phase)
    }

    fn can_restart(&self) -> bool {
        matches!(self.projection.run_phase, RunPhase::Dead)
    }

    fn restart_resume_path(&self) -> Option<PathBuf> {
        if let Some(session) = self.projection.state.session_file.as_ref()
            && !session.is_empty()
        {
            return Some(PathBuf::from(session));
        }
        let raw = std::fs::read_to_string(last_session_path()).ok()?;
        let raw = raw.trim();
        (!raw.is_empty()).then(|| PathBuf::from(raw))
    }

    fn do_restart(&mut self, cx: &mut Context<Self>) {
        let resume = self.restart_resume_path();
        self.projection.mark_restarting();
        self.client = None;
        self.pump = None;
        "Restarting session…".clone_into(&mut self.status_message);
        cx.notify();

        match try_connect_omp(resume) {
            Ok((client, proj, status, models)) => {
                self.available_models = models;
                self.projection = proj;
                self.status_message = status;
                self.client = Some(client.clone());
                let n = self.projection.transcript.len();
                self.transcript_list.reset(n);
                self.transcript_list.set_follow_mode(FollowMode::Tail);
                self.last_transcript_len = n;
                self.unread_below = 0;
                let events = client.events();
                self.pump = Some(cx.spawn(async move |view, cx| {
                    while let Ok(event) = events.recv().await {
                        let _ = view.update(cx, |this, cx| {
                            match &event {
                                ClientEvent::Frame(frame) => {
                                    let is_model_changed =
                                        frame.raw.get("type").and_then(|v| v.as_str())
                                            == Some("model_changed");
                                    this.projection.apply(frame);
                                    if is_model_changed {
                                        this.refresh_state_after_model_change(cx);
                                    }
                                }
                                ClientEvent::Closed(info) => {
                                    let reason = info.error_msg.clone().unwrap_or_else(|| {
                                        format!("exit code {:?}", info.exit_code)
                                    });
                                    this.projection.mark_dead(reason);
                                    this.client = None;
                                    let tail = &info.stderr_tail;
                                    this.status_message =
                                        format!("OMP closed — {}", &tail[..256.min(tail.len())]);
                                }
                            }
                            cx.notify();
                        });
                    }
                }));
                self.start_catalog_load(cx);
            }
            Err(e) => {
                self.projection.mark_dead(e.clone());
                self.status_message = e;
            }
        }
        cx.notify();
    }

    fn can_abort(&self) -> bool {
        self.client.is_some() && phase_allows_abort(&self.projection.run_phase)
    }

    fn do_abort(&self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn(async move |_, _| {
            let _ = client.send(RpcCommandBody::Abort).await;
        })
        .detach();
    }

    fn toggle_tool_expanded(&mut self, tool_call_id: &str, cx: &mut Context<Self>) {
        if self.expanded_tools.contains(tool_call_id) {
            self.expanded_tools.remove(tool_call_id);
        } else {
            self.expanded_tools.insert(tool_call_id.to_owned());
        }
        if let Some(ix) = self.projection.transcript.iter().position(
            |e| matches!(e, TranscriptEntry::ToolCall(tc) if tc.tool_call_id == tool_call_id),
        ) {
            self.transcript_list.remeasure_items(ix..ix + 1);
        }
        cx.notify();
    }

    fn client_and_dialog_id(&self, dialog_id: &str) -> Option<(RpcClient, bool)> {
        let client = self.client.clone()?;
        let exists = self
            .projection
            .pending_dialogs
            .iter()
            .any(|d| d.id == dialog_id);
        Some((client, exists))
    }

    fn handle_dialog_key(&self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if event.keystroke.modifiers.modified() {
            return false;
        }
        let Some(dialog) = self.projection.pending_dialogs.first() else {
            return false;
        };
        let option_count = select_dialog_options(dialog).len();
        let action =
            dialog_key_action(&event.keystroke.key, &dialog.method, option_count).or_else(|| {
                event
                    .keystroke
                    .key_char
                    .as_deref()
                    .and_then(|key| dialog_key_action(key, &dialog.method, option_count))
            });
        let Some(action) = action else {
            return false;
        };

        let view = cx.entity().downgrade();
        let id = dialog.id.clone();
        match action {
            DialogKeyAction::Confirm => {
                let mut fields = serde_json::Map::new();
                fields.insert("accepted".into(), serde_json::Value::Bool(true));
                do_dialog_response(&view, &id, fields, cx);
            }
            DialogKeyAction::Deny => {
                let mut fields = serde_json::Map::new();
                fields.insert("accepted".into(), serde_json::Value::Bool(false));
                do_dialog_response(&view, &id, fields, cx);
            }
            DialogKeyAction::Cancel => do_cancel_dialog(&view, &id, cx),
            DialogKeyAction::Select(idx) => {
                if let Some(opt) = select_dialog_options(dialog).into_iter().nth(idx) {
                    let mut fields = serde_json::Map::new();
                    fields.insert("value".into(), serde_json::Value::String(opt));
                    do_dialog_response(&view, &id, fields, cx);
                }
            }
        }
        true
    }

    fn can_follow_up(&self, cx: &Context<Self>) -> bool {
        self.client.is_some()
            && matches!(self.projection.run_phase, RunPhase::Streaming)
            && !self.composer.read(cx).value().trim().is_empty()
    }

    fn do_follow_up(&mut self, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };

        self.projection.push_user_message(text.clone());
        self.clear_composer = true;
        cx.notify();

        cx.spawn(async move |_, _| {
            let _ = client
                .send(RpcCommandBody::FollowUp {
                    message: text,
                    images: None,
                })
                .await;
        })
        .detach();
    }
}

// ── guards ────────────────────────────────────────────────────────────────

fn composer_uses_steer(phase: &RunPhase) -> bool {
    matches!(phase, RunPhase::Streaming)
}

fn phase_allows_send(phase: &RunPhase) -> bool {
    !matches!(phase, RunPhase::Dead | RunPhase::Restarting)
}

fn phase_allows_abort(phase: &RunPhase) -> bool {
    matches!(
        phase,
        RunPhase::Streaming | RunPhase::AwaitingResume | RunPhase::Compacting | RunPhase::Retrying
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogKeyAction {
    Confirm,
    Deny,
    Cancel,
    Select(usize),
}

fn dialog_key_action(key: &str, method: &str, option_count: usize) -> Option<DialogKeyAction> {
    match key {
        "escape" => Some(DialogKeyAction::Cancel),
        "y" | "Y" if method == "confirm" => Some(DialogKeyAction::Confirm),
        "n" | "N" if method == "confirm" => Some(DialogKeyAction::Deny),
        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" if method == "select" => {
            let idx = key.chars().next()?.to_digit(10)? as usize - 1;
            (idx < option_count).then_some(DialogKeyAction::Select(idx))
        }
        _ => None,
    }
}

fn select_dialog_options(dialog: &UiDialog) -> Vec<String> {
    dialog
        .payload
        .get("options")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

// ── render ────────────────────────────────────────────────────────────────

impl Render for SessionView {
    #[allow(clippy::too_many_lines)] // GPUI render fns are declaratively dense; splitting hurts readability.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.clear_composer {
            self.clear_composer = false;
            self.composer.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
        if self.clear_model_search {
            self.clear_model_search = false;
            self.model_search.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }

        let theme = cx.theme().clone();
        let is_dark = theme.mode.is_dark();
        let toggle_label = if is_dark { "☀ Light" } else { "☾ Dark" };
        let phase_label = match self.projection.run_phase {
            RunPhase::Idle => "idle",
            RunPhase::Streaming => "streaming",
            RunPhase::AwaitingResume => "awaiting…",
            RunPhase::Compacting => "compacting",
            RunPhase::Retrying => "retrying",
            RunPhase::Restarting => "restarting",
            RunPhase::Dead => "dead",
        };
        let model_label = self
            .projection
            .state
            .model
            .clone()
            .unwrap_or_else(|| "(no model)".to_owned());
        let can_pick = self.client.is_some();
        let query = self.model_search.read(cx).value().to_string();
        let filtered = filter_models(&self.available_models, &query);
        let total_matches = filtered.len();
        let visible_count = total_matches.min(MODEL_PICKER_VISIBLE_CAP);
        let visible = &filtered[..visible_count];
        let truncated = total_matches > visible_count;
        let empty_query = query.trim().is_empty();
        let footer = if truncated {
            format!("Showing {visible_count} of {total_matches} matches")
        } else if empty_query && total_matches > visible_count {
            format!("Showing {visible_count} of {total_matches} models")
        } else {
            String::new()
        };
        let view = cx.entity();

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .capture_key_down(cx.listener(|this, event, _window, cx| {
                if this.handle_dialog_key(event, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                v_flex()
                    .w_full()
                    .bg(theme.muted)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .w_full()
                            .px_3()
                            .py_1()
                            .gap_3()
                            .child(Label::new(self.status_message.clone()).text_xs())
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Label::new(phase_label)
                                            .text_xs()
                                            .text_color(theme.muted_foreground),
                                    )
                                    .child(
                                        Button::new("model-picker")
                                            .label(model_label)
                                            .small()
                                            .ghost()
                                            .disabled(!can_pick)
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _window, cx| {
                                                    this.toggle_model_picker(cx);
                                                },
                                            )),
                                    ),
                            )
                            .child(
                                h_flex().flex_1().justify_end().child(
                                    Button::new("theme-toggle")
                                        .label(toggle_label)
                                        .small()
                                        .ghost()
                                        .on_click(toggle_theme),
                                ),
                            ),
                    )
                    .when(self.model_picker_open, |parent| {
                        parent.child(
                            v_flex()
                                .w_full()
                                .px_3()
                                .pb_2()
                                .gap_1()
                                .border_b_1()
                                .border_color(theme.border)
                                .child(
                                    Input::new(&self.model_search)
                                        .appearance(true)
                                        .focus_bordered(true),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .overflow_y_scrollbar()
                                        .max_h(px(240.))
                                        .children(visible.iter().enumerate().map(
                                            |(ix, (provider, id))| {
                                                let label = format!("{provider}/{id}");
                                                let provider_c = provider.clone();
                                                let id_c = id.clone();
                                                Button::new(("model-choice", ix))
                                                    .label(label)
                                                    .ghost()
                                                    .small()
                                                    .w_full()
                                                    .on_click(window.listener_for(
                                                        &view,
                                                        move |this, _, _window, cx| {
                                                            this.close_model_picker(cx);
                                                            this.set_model(
                                                                provider_c.clone(),
                                                                id_c.clone(),
                                                                cx,
                                                            );
                                                        },
                                                    ))
                                            },
                                        )),
                                )
                                .when(!footer.is_empty(), |panel| {
                                    panel.child(
                                        Label::new(footer)
                                            .text_xs()
                                            .text_color(theme.muted_foreground),
                                    )
                                })
                                .when(visible.is_empty(), |panel| {
                                    panel.child(
                                        Label::new("(no matches)")
                                            .text_xs()
                                            .text_color(theme.muted_foreground),
                                    )
                                }),
                        )
                    }),
            )
            .child({
                self.sync_transcript_list();
                let list_state = self.transcript_list.clone();
                let view = cx.entity().downgrade();
                let unread = self.unread_below;
                div()
                    .flex_1()
                    .w_full()
                    .relative()
                    .child(
                        list(list_state, move |ix, _window, cx| {
                            view.update(cx, |this, cx| {
                                this.projection.transcript.get(ix).map_or_else(
                                    || div().into_any_element(),
                                    |e| render_entry(e, &this.expanded_tools, cx),
                                )
                            })
                            .unwrap_or_else(|_| div().into_any_element())
                        })
                        .size_full()
                        .px_3()
                        .py_2(),
                    )
                    .when(unread > 0, |parent| {
                        parent.child(
                            div().absolute().bottom_3().right_3().child(
                                Button::new("jump-transcript-tail")
                                    .label(format!("{unread} new ↓"))
                                    .small()
                                    .primary()
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.jump_to_transcript_tail(cx);
                                    })),
                            ),
                        )
                    })
            })
            .when(!self.projection.pending_dialogs.is_empty(), |parent| {
                parent.child(
                    v_flex()
                        .w_full()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .bg(theme.secondary)
                        .border_t_1()
                        .border_color(theme.border)
                        .children(
                            self.projection
                                .pending_dialogs
                                .iter()
                                .map(|d| render_dialog(d, cx)),
                        ),
                )
            })
            .when(
                matches!(self.projection.run_phase, RunPhase::Dead),
                |parent| {
                    parent.child(render_crash_card(
                        &self.status_message,
                        self.projection.dead_reason.as_deref(),
                        self.can_restart(),
                        cx,
                    ))
                },
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .bg(theme.muted)
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        div().flex_1().child(
                            Input::new(&self.composer)
                                .appearance(false)
                                .focus_bordered(false),
                        ),
                    )
                    .child(
                        Button::new("send")
                            .primary()
                            .label("Send")
                            .disabled(!self.can_send())
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.on_composer_event(
                                    this.composer.clone(),
                                    &InputEvent::PressEnter {
                                        secondary: false,
                                        shift: false,
                                    },
                                    cx,
                                );
                            })),
                    )
                    .when(
                        matches!(self.projection.run_phase, RunPhase::Streaming),
                        |parent| {
                            parent.child(
                                Button::new("follow-up")
                                    .label("Follow-up")
                                    .disabled(!self.can_follow_up(cx))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.do_follow_up(cx);
                                    })),
                            )
                        },
                    )
                    .when(self.can_abort(), |parent| {
                        parent.child(Button::new("abort").danger().label("Abort").on_click(
                            cx.listener(|this, _: &ClickEvent, _window, cx| this.do_abort(cx)),
                        ))
                    })
                    .when(self.can_restart(), |parent| {
                        parent.child(Button::new("restart").primary().label("Restart").on_click(
                            cx.listener(|this, _: &ClickEvent, _window, cx| this.do_restart(cx)),
                        ))
                    }),
            )
    }
}

// ── transcript rows ───────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)] // GPUI render fns are declaratively dense; splitting hurts readability.
fn render_entry(
    entry: &TranscriptEntry,
    expanded: &HashSet<String>,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    match entry {
        TranscriptEntry::User { text } => h_flex()
            .w_full()
            .justify_end()
            .py_1()
            .child(
                div()
                    .max_w(px(480.))
                    .px_3()
                    .py_1p5()
                    .rounded_md()
                    .bg(theme.primary)
                    .text_color(theme.primary_foreground)
                    .child(text.clone()),
            )
            .into_any_element(),
        TranscriptEntry::AssistantText { markdown, .. } => div()
            .w_full()
            .py_1()
            .child(TextView::markdown("assistant", markdown.as_str()).selectable(true))
            .into_any_element(),
        TranscriptEntry::Thinking {
            collapsed: true, ..
        } => div()
            .w_full()
            .py_1()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(theme.muted)
                    .text_color(theme.muted_foreground)
                    .text_xs()
                    .child("💭 thinking…"),
            )
            .into_any_element(),
        TranscriptEntry::Thinking { text, .. } => div()
            .w_full()
            .py_1()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(theme.muted)
                    .child(TextView::markdown("thinking", text.clone())),
            )
            .into_any_element(),
        TranscriptEntry::ToolCall(tc) => {
            render_tool_card(tc, expanded.contains(&tc.tool_call_id), cx)
        }
        TranscriptEntry::Notice(text) => div()
            .w_full()
            .py_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(text.clone()),
            )
            .into_any_element(),
        TranscriptEntry::Error { message, .. } => div()
            .w_full()
            .py_1()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(theme.danger)
                    .text_sm()
                    .child(message.clone()),
            )
            .into_any_element(),
        TranscriptEntry::CommandOutput(text) => div()
            .w_full()
            .py_1()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(theme.muted)
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .child(text.clone()),
            )
            .into_any_element(),
        TranscriptEntry::Compaction { phase } => div()
            .w_full()
            .py_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("compaction: {phase:?}")),
            )
            .into_any_element(),
        TranscriptEntry::RetryInfo { detail } => div()
            .w_full()
            .py_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("retry: {detail}")),
            )
            .into_any_element(),
        TranscriptEntry::Unknown { raw } => div()
            .w_full()
            .py_1()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(theme.warning)
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .child(format!("{raw:#}")),
            )
            .into_any_element(),
    }
}

#[allow(clippy::too_many_lines)] // GPUI render fns are declaratively dense; splitting hurts readability.
fn render_tool_card(
    tc: &pimiento_core::transcript::ToolCall,
    expanded: bool,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let (status_color, status_label) = match tc.status {
        ToolStatus::Running => (theme.info, "running"),
        ToolStatus::Ok => (theme.success, "ok"),
        ToolStatus::Err => (theme.danger, "error"),
    };
    let arg_digest: String = tc.args_json.to_string().chars().take(80).collect();
    let duration_str = tc
        .duration_ms
        .map(|ms| format!("{}.{:03}s", ms / 1000, ms % 1000))
        .unwrap_or_default();
    let tc_id = tc.tool_call_id.clone();
    let view = cx.entity().downgrade();
    let output_text = tc.output.to_string();
    let has_output = !tc.output.is_empty();

    v_flex()
        .w_full()
        .py_1()
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
        .when(expanded && has_output, |parent| {
            parent.child(
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
                    .child(output_text.clone()),
            )
        })
        .when(has_output, |parent| {
            parent.child(
                h_flex()
                    .gap_2()
                    .child({
                        let tc_id = tc_id.clone();
                        Button::new(format!("toggle-tool-{tc_id}"))
                            .label(if expanded {
                                "▲ collapse"
                            } else {
                                "▼ output"
                            })
                            .small()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                let _ = view
                                    .update(cx, |this, cx| this.toggle_tool_expanded(&tc_id, cx));
                            })
                    })
                    .child(
                        Button::new(format!("copy-tool-{tc_id}"))
                            .label("📋 copy")
                            .small()
                            .ghost()
                            .on_click({
                                let output_text = output_text.clone();
                                move |_, _window, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        output_text.clone(),
                                    ));
                                }
                            }),
                    ),
            )
        })
        .into_any_element()
}

// ── crash card ────────────────────────────────────────────────────────────

fn render_crash_card(
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
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.do_restart(cx);
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

fn render_dialog(dialog: &UiDialog, cx: &mut Context<SessionView>) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let title = dialog
        .payload
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&dialog.method);
    v_flex()
        .w_full()
        .p_3()
        .gap_2()
        .rounded_md()
        .bg(theme.secondary)
        .border_1()
        .border_color(theme.border)
        .child(Label::new(gpui::SharedString::from(title.to_owned())).text_sm())
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

fn render_select_dialog(
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

fn render_confirm_dialog(dialog: &UiDialog, cx: &mut Context<SessionView>) -> gpui::AnyElement {
    let view = cx.entity().downgrade();
    let id_yes = dialog.id.clone();
    let id_no = dialog.id.clone();
    h_flex()
        .gap_2()
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
        .child({
            let view = view.clone();
            Button::new("confirm-no")
                .label("N No")
                .small()
                .on_click(move |_, _, cx| {
                    let mut fields = serde_json::Map::new();
                    fields.insert("accepted".into(), serde_json::Value::Bool(false));
                    do_dialog_response(&view, &id_no, fields, cx);
                })
        })
        .into_any_element()
}

fn open_url_target(dialog: &UiDialog) -> Option<String> {
    dialog
        .payload
        .get("url")
        .or_else(|| dialog.payload.get("launchUrl"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn open_url_in_os_browser(url: &str) {
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

fn render_open_url_dialog(dialog: &UiDialog, cx: &mut Context<SessionView>) -> gpui::AnyElement {
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
                .gap_2()
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
                })
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
                .child(render_cancel_button(dialog, cx)),
        )
        .into_any_element()
}

fn render_cancel_button(dialog: &UiDialog, cx: &mut Context<SessionView>) -> gpui::AnyElement {
    let view = cx.entity().downgrade();
    let id = dialog.id.clone();
    Button::new("cancel-dialog")
        .label("Cancel")
        .small()
        .ghost()
        .on_click(move |_, _, cx| do_cancel_dialog(&view, &id, cx))
        .into_any_element()
}

fn do_cancel_dialog(view: &gpui::WeakEntity<SessionView>, id: &str, cx: &mut gpui::App) {
    let mut fields = serde_json::Map::new();
    fields.insert("cancel".into(), serde_json::Value::Bool(true));
    do_dialog_response(view, id, fields, cx);
}

fn do_dialog_response(
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

// ── OMP connection helper ─────────────────────────────────────────────────

fn last_session_path() -> PathBuf {
    dirs_next_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pimiento")
        .join("last-session")
}

fn dirs_next_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn remember_session_file(session_file: Option<&str>) {
    let Some(session_file) = session_file.filter(|s| !s.is_empty()) else {
        return;
    };
    let path = last_session_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, session_file);
}

fn try_connect_omp(
    resume: Option<PathBuf>,
) -> Result<(RpcClient, SessionProjection, String, Vec<ModelChoice>), String> {
    let inputs = DiscoveryInputs {
        override_bin: std::env::var_os("PIMIENTO_OMP_BIN")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute()),
        login_shell: std::env::var_os("SHELL").map(PathBuf::from),
        current_env: std::env::vars_os().collect(),
        ..Default::default()
    };

    let discovered = smol::block_on(async { discover(&inputs, &SystemRunner) })
        .map_err(|e| format!("OMP not found: {e}"))?;

    let cwd = std::env::var_os("PIMIENTO_CWD")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::current_dir().ok());

    let resume = resume.or_else(|| {
        let raw = std::fs::read_to_string(last_session_path()).ok()?;
        let raw = raw.trim();
        (!raw.is_empty()).then(|| PathBuf::from(raw))
    });

    let mut cfg = ClientConfig {
        program: discovered.path,
        env: discovered.env,
        cwd,
        no_session: false,
        resume: resume.clone(),
        ..Default::default()
    };

    let client = match smol::block_on(async { RpcClient::connect(cfg.clone()).await }) {
        Ok(c) => c,
        Err(e) if resume.is_some() => {
            let _ = std::fs::remove_file(last_session_path());
            cfg.resume = None;
            smol::block_on(async { RpcClient::connect(cfg).await })
                .map_err(|e2| format!("OMP connect failed (resume {e}; fresh {e2})"))?
        }
        Err(e) => return Err(format!("OMP connect failed: {e}")),
    };

    let get_state = smol::block_on(async { client.send(RpcCommandBody::GetState).await });
    let avail = smol::block_on(async { client.send(RpcCommandBody::GetAvailableCommands).await });
    let _sub = smol::block_on(async {
        client
            .send(RpcCommandBody::SetSubagentSubscription {
                level: SubagentSubscriptionLevel::Progress,
            })
            .await
    });

    let mut proj = SessionProjection::new();
    if let Ok(r) = &get_state
        && r.success
        && let Some(data) = &r.data
    {
        proj.hydrate_get_state(data);
        remember_session_file(proj.state.session_file.as_deref());
    }
    if let Ok(r) = &avail
        && r.success
        && let Some(data) = &r.data
    {
        proj.hydrate_available_commands(data);
    }

    // Full model catalog is loaded asynchronously after the window opens —
    // get_available_models can be large enough to exceed the default RPC timeout.
    let models = Vec::new();

    let model = proj.state.model.as_deref().unwrap_or("(no model)");
    let mut parts = vec![discovered.version_text.trim().to_owned(), model.to_owned()];
    if let Some(t) = thinking_label(proj.state.thinking.as_ref()) {
        parts.push(format!("think:{t}"));
    }
    if let Some(c) = context_percent_label(proj.state.context.as_ref()) {
        parts.push(format!("ctx:{c}"));
    }
    if let Some(t) = tokens_per_second_label(proj.state.tokens.as_ref()) {
        parts.push(format!("{t}/s"));
    }
    let status = parts.join("  •  ");

    Ok((client, proj, status, models))
}

fn context_percent_label(v: Option<&serde_json::Value>) -> Option<String> {
    let v = v?;
    let pct = v
        .get("percent")
        .and_then(serde_json::Value::as_f64)
        .or_else(|| v.as_f64())?;
    Some(format!("{pct:.0}%"))
}

fn tokens_per_second_label(v: Option<&serde_json::Value>) -> Option<String> {
    let v = v?;
    let tps = v
        .get("tokensPerSecond")
        .and_then(serde_json::Value::as_f64)
        .or_else(|| v.as_f64())?;
    if tps <= 0.0 {
        return None;
    }
    Some(format!("{tps:.1}"))
}

fn thinking_label(v: Option<&serde_json::Value>) -> Option<String> {
    let v = v?;
    if let Some(s) = v.as_str() {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_owned());
    }
    if let Some(s) = v.get("level").and_then(|x| x.as_str()) {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_owned());
    }
    if let Some(s) = v.get("thinkingLevel").and_then(|x| x.as_str()) {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_owned());
    }
    None
}

fn load_all_model_choices(catalog: &serde_json::Value, current: Option<&str>) -> Vec<ModelChoice> {
    let current_choice = current.and_then(split_model_label);
    let mut out = Vec::new();
    if let Some(arr) = catalog.get("models").and_then(|v| v.as_array()) {
        for m in arr {
            if let Some(label) = format_model_label(m)
                && let Some((provider, id)) = split_model_label(&label)
            {
                out.push((provider, id));
            }
        }
    }
    out.sort_by(|a, b| {
        model_sort_key(a, current_choice.as_ref()).cmp(&model_sort_key(b, current_choice.as_ref()))
    });
    out
}

fn model_matches_query(provider: &str, id: &str, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let q = q.to_ascii_lowercase();
    provider.to_ascii_lowercase().contains(&q) || id.to_ascii_lowercase().contains(&q)
}

fn model_sort_key(choice: &ModelChoice, current: Option<&ModelChoice>) -> (u8, String, String) {
    let (provider, id) = choice;
    if current == Some(choice) {
        return (0, provider.clone(), id.clone());
    }
    let is_composer_boost =
        provider == "cursor" && (id == "composer-2.5" || id.contains("composer-2.5"));
    let tier = if is_composer_boost {
        1
    } else if provider == "cursor" {
        2
    } else {
        3
    };
    (tier, provider.clone(), id.clone())
}

fn filter_models(models: &[ModelChoice], query: &str) -> Vec<ModelChoice> {
    models
        .iter()
        .filter(|(provider, id)| model_matches_query(provider, id, query))
        .cloned()
        .collect()
}

// ── entry ─────────────────────────────────────────────────────────────────

fn main() {
    let connect_result = try_connect_omp(None);

    gpui_platform::application().run(|cx| {
        gpui_component::init(cx);
        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let (client, proj, status, models) = match &connect_result {
                    Ok((c, p, s, m)) => (Some(c.clone()), p.clone(), s.clone(), m.clone()),
                    Err(e) => (None, SessionProjection::new(), e.clone(), Vec::new()),
                };
                let view = cx.new(|cx| SessionView::new(window, cx, client, status, proj, models));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("open primary window");
        })
        .detach();
    });
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_allows_send_idle() {
        assert!(phase_allows_send(&RunPhase::Idle));
    }
    #[test]
    fn phase_allows_send_streaming() {
        assert!(phase_allows_send(&RunPhase::Streaming));
    }
    #[test]
    fn composer_steers_only_while_streaming() {
        assert!(composer_uses_steer(&RunPhase::Streaming));
        assert!(!composer_uses_steer(&RunPhase::Idle));
        assert!(!composer_uses_steer(&RunPhase::Dead));
    }
    #[test]
    fn thinking_label_reads_string_or_level_object() {
        assert_eq!(
            thinking_label(Some(&serde_json::json!("high"))).as_deref(),
            Some("high")
        );
        assert_eq!(
            thinking_label(Some(&serde_json::json!({"level":"medium"}))).as_deref(),
            Some("medium")
        );
    }
    #[test]
    fn context_and_tps_labels() {
        assert_eq!(
            context_percent_label(Some(&serde_json::json!({"percent": 8.378}))).as_deref(),
            Some("8%")
        );
        assert_eq!(
            tokens_per_second_label(Some(&serde_json::json!({"tokensPerSecond": 12.34})))
                .as_deref(),
            Some("12.3")
        );
        assert_eq!(tokens_per_second_label(Some(&serde_json::json!(0.0))), None);
    }
    #[test]
    fn phase_disallows_send_dead() {
        assert!(!phase_allows_send(&RunPhase::Dead));
    }
    #[test]
    fn phase_disallows_send_restarting() {
        assert!(!phase_allows_send(&RunPhase::Restarting));
    }
    #[test]
    fn phase_allows_abort_streaming() {
        assert!(phase_allows_abort(&RunPhase::Streaming));
    }
    #[test]
    fn phase_disallows_abort_idle() {
        assert!(!phase_allows_abort(&RunPhase::Idle));
    }
    #[test]
    fn phase_disallows_abort_dead() {
        assert!(!phase_allows_abort(&RunPhase::Dead));
    }
    #[test]
    fn dialog_key_confirm_yes_no_escape() {
        assert_eq!(
            dialog_key_action("y", "confirm", 0),
            Some(DialogKeyAction::Confirm)
        );
        assert_eq!(
            dialog_key_action("Y", "confirm", 0),
            Some(DialogKeyAction::Confirm)
        );
        assert_eq!(
            dialog_key_action("n", "confirm", 0),
            Some(DialogKeyAction::Deny)
        );
        assert_eq!(
            dialog_key_action("N", "confirm", 0),
            Some(DialogKeyAction::Deny)
        );
        assert_eq!(
            dialog_key_action("escape", "confirm", 0),
            Some(DialogKeyAction::Cancel)
        );
        assert_eq!(dialog_key_action("1", "confirm", 0), None);
    }

    #[test]
    fn dialog_key_select_digits_and_escape() {
        assert_eq!(
            dialog_key_action("1", "select", 3),
            Some(DialogKeyAction::Select(0))
        );
        assert_eq!(
            dialog_key_action("3", "select", 3),
            Some(DialogKeyAction::Select(2))
        );
        assert_eq!(dialog_key_action("4", "select", 3), None);
        assert_eq!(
            dialog_key_action("escape", "select", 3),
            Some(DialogKeyAction::Cancel)
        );
        assert_eq!(dialog_key_action("y", "select", 3), None);
    }

    #[test]
    fn next_theme_mode_flips_both_ways() {
        assert_eq!(next_theme_mode(ThemeMode::Light), ThemeMode::Dark);
        assert_eq!(next_theme_mode(ThemeMode::Dark), ThemeMode::Light);
        assert_eq!(
            next_theme_mode(next_theme_mode(ThemeMode::Light)),
            ThemeMode::Light
        );
    }

    #[test]
    fn load_all_model_choices_reads_full_catalog() {
        let catalog = serde_json::json!({
            "models": [
                {"provider": "opencode-go", "id": "gpt-5.6-luna"},
                {"provider": "cursor", "id": "composer-2.5"},
                {"provider": "other", "id": "m1"}
            ]
        });
        let models = load_all_model_choices(&catalog, None);
        assert_eq!(models.len(), 3);
        assert!(models.contains(&("cursor".into(), "composer-2.5".into())));
    }

    #[test]
    fn search_composer_includes_cursor_model() {
        let models = load_all_model_choices(
            &serde_json::json!({
                "models": [
                    {"provider": "opencode-go", "id": "gpt-5.6-luna"},
                    {"provider": "cursor", "id": "composer-2.5"},
                    {"provider": "other", "id": "m1"}
                ]
            }),
            None,
        );
        let filtered = filter_models(&models, "composer");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0], ("cursor".into(), "composer-2.5".into()));
    }

    #[test]
    fn model_sort_current_then_cursor_then_alpha() {
        let catalog = serde_json::json!({
            "models": [
                {"provider": "zeta", "id": "z"},
                {"provider": "cursor", "id": "other"},
                {"provider": "cursor", "id": "composer-2.5"},
                {"provider": "alpha", "id": "a"}
            ]
        });
        let sorted = load_all_model_choices(&catalog, Some("alpha/a"));
        assert_eq!(sorted[0], ("alpha".into(), "a".into()));
        assert_eq!(sorted[1], ("cursor".into(), "composer-2.5".into()));
        assert_eq!(sorted[2], ("cursor".into(), "other".into()));
        assert_eq!(sorted[3], ("zeta".into(), "z".into()));
    }
}

#[cfg(test)]
mod open_url_tests {
    use super::*;
    use pimiento_core::projection::UiDialog;
    use serde_json::json;

    #[test]
    fn open_url_target_reads_url_or_launch() {
        let d = UiDialog {
            id: "1".into(),
            method: "open_url".into(),
            payload: json!({"url": "https://example.com/a"}),
            timeout_ms: None,
        };
        assert_eq!(
            open_url_target(&d).as_deref(),
            Some("https://example.com/a")
        );
        let d2 = UiDialog {
            id: "2".into(),
            method: "open_url".into(),
            payload: json!({"launchUrl": "https://example.com/b"}),
            timeout_ms: None,
        };
        assert_eq!(
            open_url_target(&d2).as_deref(),
            Some("https://example.com/b")
        );
    }
}
