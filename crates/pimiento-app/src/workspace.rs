use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RailAttention {
    Quiet,
    Active,
    Unread,
}

pub(crate) fn classify_rail_attention(phase: &RunPhase, unread_below: usize) -> RailAttention {
    if phase_allows_abort(phase) {
        RailAttention::Active
    } else if unread_below > 0 {
        RailAttention::Unread
    } else {
        RailAttention::Quiet
    }
}

pub(crate) fn workspace_window_title(session_name: &str, phase: &RunPhase) -> String {
    let phase = match phase {
        RunPhase::Idle => "idle",
        RunPhase::Streaming => "streaming",
        RunPhase::AwaitingResume => "awaiting",
        RunPhase::Compacting => "compacting",
        RunPhase::Retrying => "retrying",
        RunPhase::Restarting => "restarting",
        RunPhase::Dead => "dead",
    };
    if session_name == "Pimiento" || session_name.trim().is_empty() {
        format!("Pimiento · {phase}")
    } else {
        format!("Pimiento — {session_name} · {phase}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RailEntry {
    pub(crate) ix: usize,
    pub(crate) label: String,
    pub(crate) phase: String,
    pub(crate) cwd: PathBuf,
    pub(crate) attention: RailAttention,
}

pub(crate) fn workspace_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| path.display().to_string(), str::to_owned)
}

pub(crate) fn group_sessions_by_workspace(
    entries: Vec<RailEntry>,
) -> Vec<(PathBuf, Vec<RailEntry>)> {
    let mut groups = BTreeMap::<PathBuf, Vec<RailEntry>>::new();
    for entry in entries {
        groups.entry(entry.cwd.clone()).or_default().push(entry);
    }
    let mut groups = groups.into_iter().collect::<Vec<_>>();
    groups.sort_by(|(left, _), (right, _)| {
        workspace_display_name(left)
            .to_ascii_lowercase()
            .cmp(&workspace_display_name(right).to_ascii_lowercase())
            .then_with(|| left.cmp(right))
    });
    for (_, entries) in &mut groups {
        entries.sort_by_key(|entry| entry.ix);
    }
    groups
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectorFocus {
    Session,
    Checklist,
    Agents,
}

// Layout visibility and quit confirmation are independent UI concerns.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct WorkspaceView {
    pub(crate) sessions: Vec<gpui::Entity<SessionView>>,
    pub(crate) session_subscriptions: Vec<gpui::Subscription>,
    pub(crate) active: usize,
    pub(crate) persistence: SessionPersistence,
    pub(crate) initial_cwd: PathBuf,
    pub(crate) rail_collapsed: bool,
    pub(crate) inspector_open: bool,
    pub(crate) inspector_focus: InspectorFocus,
    pub(crate) tools_expanded: bool,
    pub(crate) pending_quit_confirm: bool,
    pub(crate) quit_in_progress: bool,
    pub(crate) last_window_title: String,
}

impl WorkspaceView {
    pub(crate) fn new(
        first: gpui::Entity<SessionView>,
        persistence: SessionPersistence,
        initial_cwd: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let first_subscription = cx.observe(&first, |_this, _session, cx| {
            cx.notify();
        });
        Self {
            sessions: vec![first],
            session_subscriptions: vec![first_subscription],
            active: 0,
            inspector_open: persistence.load_inspector_open(),
            persistence,
            initial_cwd,
            rail_collapsed: false,
            inspector_focus: InspectorFocus::Session,
            tools_expanded: false,
            pending_quit_confirm: false,
            quit_in_progress: false,
            last_window_title: String::new(),
        }
    }

    pub(crate) fn should_close_window(&mut self, cx: &mut Context<Self>) -> bool {
        if self.quit_in_progress {
            return true;
        }
        let phases = self
            .sessions
            .iter()
            .map(|session| session.read(cx).projection.run_phase.clone())
            .collect::<Vec<_>>();
        if workspace_should_block_close(&phases) {
            self.pending_quit_confirm = true;
            cx.notify();
            false
        } else {
            true
        }
    }

    pub(crate) fn cancel_pending_quit(&mut self, cx: &mut Context<Self>) {
        if !self.quit_in_progress {
            self.pending_quit_confirm = false;
            cx.notify();
        }
    }

    pub(crate) fn confirm_pending_quit(&mut self, cx: &mut Context<Self>) {
        if !self.pending_quit_confirm || self.quit_in_progress {
            return;
        }
        self.quit_in_progress = true;
        cx.notify();

        let clients = self
            .sessions
            .iter()
            .filter_map(|session| {
                let session = session.read(cx);
                session
                    .client
                    .clone()
                    .map(|client| (client, phase_allows_abort(&session.projection.run_phase)))
            })
            .collect::<Vec<_>>();

        cx.spawn(async move |_, cx| {
            for (client, should_abort) in clients {
                if should_abort {
                    let _ = client
                        .send_with_timeout(RpcCommandBody::Abort, Duration::from_secs(1))
                        .await;
                }
                client.close_stdin().await;
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    }

    pub(crate) fn handle_pending_quit_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.pending_quit_confirm {
            return false;
        }
        if self.quit_in_progress || event.keystroke.modifiers.modified() {
            return true;
        }
        match event.keystroke.key.as_str() {
            "y" | "Y" => self.confirm_pending_quit(cx),
            "n" | "N" | "escape" | "esc" => self.cancel_pending_quit(cx),
            _ => {}
        }
        true
    }

    pub(crate) fn clamp_active(&mut self) {
        if self.sessions.is_empty() {
            self.active = 0;
            return;
        }
        self.active = self.active.min(self.sessions.len() - 1);
    }

    pub(crate) fn select_session(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.sessions.len() {
            self.active = index;
            if self.inspector_open
                && let Some(session) = self.sessions.get(index).cloned()
            {
                session.update(cx, SessionView::ensure_subagent_snapshots);
            }
            cx.notify();
        }
    }

    pub(crate) fn add_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remembered = self.persistence.load_recent_sessions();
        let last_session = self
            .persistence
            .load_last_session()
            .filter(|resume| resume.exists());
        let recent = collect_launcher_sessions(
            &self.persistence,
            &self.initial_cwd,
            omp_sessions_root().as_deref(),
            home_dir().as_deref(),
            std::env::temp_dir().as_path(),
        );
        let persistence = self.persistence.clone();
        let cwd = self.initial_cwd.clone();
        let session = cx.new(|cx| {
            SessionView::new(
                window,
                cx,
                None,
                "Choose a working directory to begin".to_owned(),
                SessionProjection::new(),
                Vec::new(),
                LauncherBootstrap {
                    persistence,
                    launcher_cwd: cwd,
                    recent_sessions: if recent.is_empty() {
                        remembered
                    } else {
                        recent
                    },
                    last_session,
                },
            )
        });
        let subscription = cx.observe(&session, |_this, _session, cx| {
            cx.notify();
        });
        self.sessions.push(session);
        self.session_subscriptions.push(subscription);
        self.active = self.sessions.len() - 1;
        cx.notify();
    }

    pub(crate) fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sessions.is_empty() {
            return;
        }
        let idx = self.active;
        if let Some(session) = self.sessions.get(idx).cloned() {
            session.update(cx, SessionView::shutdown_session);
        }
        self.sessions.remove(idx);
        drop(self.session_subscriptions.remove(idx));
        if self.sessions.is_empty() {
            self.add_session(window, cx);
        } else {
            self.clamp_active();
            cx.notify();
        }
    }

    pub(crate) fn toggle_rail(&mut self, cx: &mut Context<Self>) {
        self.rail_collapsed = !self.rail_collapsed;
        cx.notify();
    }

    pub(crate) fn open_inspector(&mut self, focus: InspectorFocus, cx: &mut Context<Self>) {
        self.inspector_open = true;
        self.persistence.save_inspector_open(true);
        self.inspector_focus = focus;
        if let Some(session) = self.sessions.get(self.active).cloned() {
            session.update(cx, SessionView::ensure_subagent_snapshots);
        }
        cx.notify();
    }

    pub(crate) fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector_open = !self.inspector_open;
        self.persistence.save_inspector_open(self.inspector_open);
        if self.inspector_open
            && let Some(session) = self.sessions.get(self.active).cloned()
        {
            session.update(cx, SessionView::ensure_subagent_snapshots);
        }
        cx.notify();
    }

    pub(crate) fn handle_workspace_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.handle_pending_quit_key(event, cx) {
            return true;
        }
        let mods = &event.keystroke.modifiers;
        if !(mods.platform || mods.control) {
            return false;
        }
        let key = event.keystroke.key.as_str();
        if let Some(digit) = workspace_digit_key(key) {
            let index = digit.saturating_sub(1);
            if index < self.sessions.len() {
                self.select_session(index, cx);
                return true;
            }
            return false;
        }
        match key {
            "t" | "T" => {
                self.add_session(window, cx);
                true
            }
            "w" | "W" => {
                self.close_active(window, cx);
                true
            }
            "b" | "B" => {
                self.toggle_rail(cx);
                true
            }
            "j" | "J" => {
                self.toggle_inspector(cx);
                true
            }
            "k" | "K" => {
                if let Some(session) = self.sessions.get(self.active).cloned() {
                    session.update(cx, SessionView::toggle_palette);
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn run_workspace_palette_action(
        &mut self,
        id: PaletteActionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            PaletteActionId::NewSession => self.add_session(window, cx),
            PaletteActionId::CloseSession => self.close_active(window, cx),
            PaletteActionId::ToggleRail => self.toggle_rail(cx),
            PaletteActionId::ToggleInspector => self.toggle_inspector(cx),
            PaletteActionId::ToggleTodos => self.open_inspector(InspectorFocus::Checklist, cx),
            PaletteActionId::ToggleAgents => self.open_inspector(InspectorFocus::Agents, cx),
            other => {
                if let Some(session) = self.sessions.get(self.active).cloned() {
                    session.update(cx, |session, cx| {
                        session.run_palette_action(other, window, cx);
                    });
                }
            }
        }
    }
}

pub(crate) fn workspace_digit_key(key: &str) -> Option<usize> {
    match key {
        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => key.parse().ok(),
        _ => None,
    }
}

pub(crate) fn tool_names_from_state(state: Option<&serde_json::Value>) -> Vec<String> {
    let Some(tools) = state.and_then(|state| state.get("dumpTools")) else {
        return Vec::new();
    };
    let values = tools
        .as_array()
        .or_else(|| tools.get("tools").and_then(serde_json::Value::as_array));
    let mut names = values
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            tool.as_str()
                .map(str::to_owned)
                .or_else(|| {
                    tool.get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .or_else(|| {
                    tool.get("id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

// Keeping the declarative pane together makes its visual section order auditable.
#[allow(clippy::too_many_lines)]
pub(crate) fn render_inspector(
    session: &gpui::Entity<SessionView>,
    focus: InspectorFocus,
    tools_expanded: bool,
    window: &mut Window,
    cx: &mut Context<WorkspaceView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let (
        cwd,
        model,
        thinking,
        phase,
        context,
        tokens,
        fast_enabled,
        fast_active,
        connected,
        todo_phases,
        subagent_rows,
        selected_subagent_id,
        subagent_tail_lines,
        subagent_status,
        fallback_subagent_events,
        tool_names,
        has_tool_state,
    ) = {
        let session_view = session.read(cx);
        let cwd = session_view
            .session_cwd
            .clone()
            .unwrap_or_else(|| session_view.launcher_cwd.clone());
        let phase = match session_view.projection.run_phase {
            RunPhase::Idle => "idle",
            RunPhase::Streaming => "streaming",
            RunPhase::AwaitingResume => "awaiting",
            RunPhase::Compacting => "compacting",
            RunPhase::Retrying => "retrying",
            RunPhase::Restarting => "restarting",
            RunPhase::Dead => "dead",
        }
        .to_owned();
        let state = &session_view.projection.state;
        let raw_state = state.state.as_ref();
        let tool_names = tool_names_from_state(raw_state);
        (
            cwd,
            state
                .model
                .as_deref()
                .map_or_else(|| "Unknown model".to_owned(), short_model_label),
            thinking_label(state.thinking.as_ref()).unwrap_or_else(|| "unknown".to_owned()),
            phase,
            context_percent(state.context.as_ref()),
            tokens_per_second_label(state.tokens.as_ref()),
            state.fast_mode_enabled,
            state.fast_mode_active,
            session_view.client.is_some(),
            parse_todo_phases(session_view.projection.todos_raw.as_ref()),
            session_view
                .subagent_snapshots
                .iter()
                .filter_map(|snapshot| {
                    subagent_snapshot_id(snapshot)
                        .map(|id| (id.to_owned(), subagent_snapshot_summary(snapshot)))
                })
                .collect::<Vec<_>>(),
            session_view.selected_subagent_id.clone(),
            session_view.subagent_tail_lines.clone(),
            session_view.subagent_drawer_status.clone(),
            session_view
                .projection
                .subagents_raw
                .iter()
                .rev()
                .take(12)
                .map(subagent_payload_summary)
                .collect::<Vec<_>>(),
            tool_names,
            raw_state.is_some_and(|state| state.get("dumpTools").is_some()),
        )
    };
    let todo_count = todo_open_count(&todo_phases);
    let path = cwd.display().to_string();
    let path = truncate_subagent_text(&path, 44);
    let fast_diverges =
        fast_enabled.is_some() && fast_active.is_some() && fast_enabled != fast_active;
    let fast_detail = match (fast_enabled, fast_active) {
        (Some(enabled), Some(active)) if fast_diverges => Some(format!(
            "{} · enabled: {enabled} · active: {active}",
            fast_mode_label(fast_enabled, fast_active)
        )),
        (None, _) | (_, None) => Some("Fast state is not published yet".to_owned()),
        _ => None,
    };
    let switch_session = session.clone();
    let refresh_session = session.clone();

    v_flex()
        .w(px(272.))
        .h_full()
        .flex_shrink_0()
        .overflow_y_scrollbar()
        .gap_4()
        .p_3()
        .border_l_1()
        .border_color(theme.border)
        .bg(theme.muted)
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .child(
                    Label::new("Context")
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(
                    Label::new("⌘J")
                        .text_xs()
                        .text_color(theme.muted_foreground),
                ),
        )
        .child(Separator::horizontal())
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .when(focus == InspectorFocus::Session, |section| {
                    section.border_l_2().border_color(theme.primary).pl_2()
                })
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(
                            Label::new("Session")
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.muted_foreground),
                        )
                        .child(phase_tag(&phase).small().child(phase)),
                )
                .child(Label::new(workspace_display_name(&cwd)).text_sm())
                .child(
                    Label::new(path)
                        .text_xs()
                        .text_color(theme.muted_foreground),
                )
                .child(Label::new(model).text_xs())
                .child(
                    Label::new(format!("Thinking: {thinking}"))
                        .text_xs()
                        .text_color(theme.muted_foreground),
                )
                .when_some(context, |section, value| {
                    section.child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                Label::new(format!("ctx:{value:.0}%"))
                                    .text_xs()
                                    .text_color(theme.muted_foreground),
                            )
                            .child(
                                Progress::new("inspector-context-progress")
                                    .value(value)
                                    .xsmall(),
                            ),
                    )
                })
                .when_some(tokens, |section, value| {
                    section.child(
                        Label::new(format!("Speed: {value}/s"))
                            .text_xs()
                            .text_color(theme.muted_foreground),
                    )
                }),
        )
        .child(Separator::horizontal())
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    Label::new("Fast")
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.muted_foreground),
                )
                .child(
                    Switch::new("inspector-fast-mode")
                        .label("Fast mode")
                        .small()
                        .checked(fast_enabled.unwrap_or(false))
                        .disabled(!connected)
                        .on_click(window.listener_for(
                            &switch_session,
                            |this, _checked: &bool, _window, cx| {
                                this.toggle_fast_mode(cx);
                            },
                        )),
                )
                .when_some(fast_detail, |section, detail| {
                    section.child(
                        Label::new(detail)
                            .text_xs()
                            .text_color(theme.muted_foreground),
                    )
                }),
        )
        .child(Separator::horizontal())
        .child(
            v_flex()
                .w_full()
                .gap_2()
                .when(focus == InspectorFocus::Checklist, |section| {
                    section.border_l_2().border_color(theme.primary).pl_2()
                })
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(
                            Label::new("Checklist")
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.muted_foreground),
                        )
                        .child(Tag::secondary().small().child(todo_count.to_string())),
                )
                .children(todo_phases.iter().map(|phase| {
                    v_flex()
                        .w_full()
                        .gap_1()
                        .child(
                            Label::new(phase.name.clone())
                                .text_xs()
                                .text_color(theme.muted_foreground),
                        )
                        .children(
                            phase
                                .tasks
                                .iter()
                                .map(|task| render_todo_task(task, &theme)),
                        )
                })),
        )
        .child(Separator::horizontal())
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .when(focus == InspectorFocus::Agents, |section| {
                    section.border_l_2().border_color(theme.primary).pl_2()
                })
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(
                            Label::new("Agents")
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            Button::new("inspector-agents-refresh")
                                .label("Refresh")
                                .small()
                                .ghost()
                                .disabled(!connected)
                                .on_click(window.listener_for(
                                    &refresh_session,
                                    |this, _: &ClickEvent, _window, cx| {
                                        this.refresh_subagents(cx);
                                    },
                                )),
                        ),
                )
                .when(
                    !(subagent_status.is_empty()
                        || subagent_rows.is_empty() && subagent_status == "No agents reported"),
                    |section| {
                        section.child(
                            Label::new(subagent_status.clone())
                                .text_xs()
                                .text_color(theme.muted_foreground),
                        )
                    },
                )
                .children(subagent_rows.iter().enumerate().map(|(ix, (id, summary))| {
                    let id = id.clone();
                    let selected = selected_subagent_id.as_deref() == Some(id.as_str());
                    Button::new(("inspector-agent", ix))
                        .label(summary.clone())
                        .small()
                        .w_full()
                        .when(selected, Button::primary)
                        .when(!selected, Button::ghost)
                        .on_click(window.listener_for(
                            session,
                            move |this, _: &ClickEvent, _window, cx| {
                                this.select_subagent(id.clone(), cx);
                            },
                        ))
                }))
                .children(subagent_tail_lines.iter().enumerate().map(|(ix, line)| {
                    Label::new(format!("{ix}: {line}"))
                        .text_xs()
                        .text_color(theme.muted_foreground)
                }))
                .when(
                    subagent_rows.is_empty() && !fallback_subagent_events.is_empty(),
                    |section| {
                        section.children(fallback_subagent_events.iter().enumerate().map(
                            |(ix, summary)| {
                                Label::new(format!("#{ix} {summary}"))
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                            },
                        ))
                    },
                ),
        )
        .child(Separator::horizontal())
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .child(if tool_names.len() > 8 {
                    Button::new("inspector-tools-toggle")
                        .label(format!(
                            "{} Tools ({})",
                            if tools_expanded { "▾" } else { "▸" },
                            tool_names.len()
                        ))
                        .small()
                        .ghost()
                        .w_full()
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.tools_expanded = !this.tools_expanded;
                            cx.notify();
                        }))
                        .into_any_element()
                } else {
                    Label::new(format!("Tools ({})", tool_names.len()))
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.muted_foreground)
                        .into_any_element()
                })
                .when(
                    !tool_names.is_empty() && (tool_names.len() <= 8 || tools_expanded),
                    |section| {
                        let hidden = tool_names.len().saturating_sub(12);
                        section
                            .child(
                                h_flex().w_full().flex_wrap().gap_1().children(
                                    tool_names
                                        .iter()
                                        .take(12)
                                        .map(|name| Tag::secondary().small().child(name.clone())),
                                ),
                            )
                            .when(hidden > 0, |section| {
                                section.child(
                                    Label::new(format!("+{hidden} more"))
                                        .text_xs()
                                        .text_color(theme.muted_foreground),
                                )
                            })
                    },
                )
                .when(tool_names.is_empty(), |section| {
                    section.child(
                        Label::new(if has_tool_state {
                            "No tools reported"
                        } else {
                            "Tools not in session state"
                        })
                        .text_xs()
                        .text_color(theme.muted_foreground),
                    )
                }),
        )
        .child(Separator::horizontal())
        .child(
            Label::new("LSP/MCP status is not published on OMP rpc-ui.")
                .text_xs()
                .text_color(theme.muted_foreground),
        )
        .into_any_element()
}

impl Render for WorkspaceView {
    #[allow(clippy::too_many_lines)]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.clamp_active();
        if let Some(session) = self.sessions.get(self.active).cloned() {
            let pending =
                session.update(cx, |session, _cx| session.take_pending_workspace_palette());
            if let Some(action) = pending {
                self.run_workspace_palette_action(action, window, cx);
            }
        }
        let theme = cx.theme().clone();
        let active = self.active;
        let quit_in_progress = self.quit_in_progress;
        let inspector_open = self.inspector_open;
        let inspector_focus = self.inspector_focus;
        let groups = group_sessions_by_workspace(
            self.sessions
                .iter()
                .enumerate()
                .map(|(ix, session)| session.read(cx).rail_entry(ix))
                .collect(),
        );
        let active_session = self.sessions.get(active).cloned();
        let window_title = active_session.as_ref().map_or_else(
            || "Pimiento".to_owned(),
            |session| session.read(cx).window_title(),
        );
        if self.last_window_title != window_title {
            window.set_window_title(&window_title);
            self.last_window_title = window_title;
        }
        if inspector_open
            && let Some(session) = active_session.clone()
            && session.read(cx).subagent_snapshots.is_empty()
            && session.read(cx).subagent_drawer_status.is_empty()
        {
            session.update(cx, SessionView::ensure_subagent_snapshots);
        }

        h_flex()
            .size_full()
            .relative()
            .bg(theme.background)
            .text_color(theme.foreground)
            .capture_key_down(cx.listener(|this, event, window, cx| {
                if this.handle_workspace_key(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .when(!self.rail_collapsed, |parent| {
                parent.child(
                    v_flex()
                        .w(px(252.))
                        .h_full()
                        .p_3()
                        .border_r_1()
                        .border_color(theme.border)
                        .bg(theme.muted)
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    Button::new("workspace-new-session")
                                        .label("New")
                                        .small()
                                        .ghost()
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                this.add_session(window, cx);
                                            },
                                        )),
                                )
                                .child(
                                    Button::new("workspace-close-session")
                                        .label("Close")
                                        .small()
                                        .ghost()
                                        .disabled(self.sessions.is_empty())
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                this.close_active(window, cx);
                                            },
                                        )),
                                )
                                .child(
                                    Button::new("workspace-hide-rail")
                                        .label("Hide")
                                        .small()
                                        .ghost()
                                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                            this.toggle_rail(cx);
                                        })),
                                ),
                        )
                        .child(div().h(px(8.)))
                        .children(groups.into_iter().enumerate().map(
                            |(group_ix, (cwd, entries))| {
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .when(group_ix > 0, gpui::Styled::mt_2)
                                    .child(
                                        Separator::horizontal().label(workspace_display_name(&cwd)),
                                    )
                                    .children(entries.into_iter().map(|entry| {
                                        let selected = entry.ix == active;
                                        let ix = entry.ix;
                                        let attention_color = match entry.attention {
                                            RailAttention::Quiet => None,
                                            RailAttention::Active => Some(theme.info),
                                            RailAttention::Unread => Some(theme.warning),
                                        };
                                        h_flex()
                                            .id(("workspace-session", ix))
                                            .w_full()
                                            .justify_between()
                                            .gap_2()
                                            .px_2()
                                            .py_1()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .when(selected, |row| {
                                                row.bg(theme.primary)
                                                    .text_color(theme.primary_foreground)
                                            })
                                            .when(!selected, |row| {
                                                row.hover(|row| row.bg(theme.secondary))
                                            })
                                            .child(
                                                h_flex()
                                                    .flex_1()
                                                    .gap_2()
                                                    .when_some(attention_color, |label, color| {
                                                        label.child(
                                                            div()
                                                                .size(px(7.))
                                                                .rounded_full()
                                                                .bg(color),
                                                        )
                                                    })
                                                    .child(
                                                        Label::new(entry.label)
                                                            .text_sm()
                                                            .flex_1()
                                                            .truncate(),
                                                    ),
                                            )
                                            .child(
                                                phase_tag(&entry.phase).small().child(entry.phase),
                                            )
                                            .on_click(cx.listener(
                                                move |this, _: &ClickEvent, _window, cx| {
                                                    this.select_session(ix, cx);
                                                },
                                            ))
                                    }))
                            },
                        )),
                )
            })
            .when(self.rail_collapsed, |parent| {
                parent.child(
                    v_flex()
                        .id("workspace-show-rail")
                        .w(px(64.))
                        .h_full()
                        .items_center()
                        .gap_1()
                        .pt_2()
                        .border_r_1()
                        .border_color(theme.border)
                        .bg(theme.muted)
                        .cursor_pointer()
                        .hover(|rail| rail.bg(theme.secondary))
                        .child(
                            Label::new("☰")
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM),
                        )
                        .child(
                            Label::new("Sessions")
                                .text_xs()
                                .text_color(theme.muted_foreground),
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.toggle_rail(cx);
                        })),
                )
            })
            .child(div().flex_1().min_w(px(480.)).h_full().child(
                active_session.clone().map_or_else(
                    || div().into_any_element(),
                    gpui::IntoElement::into_any_element,
                ),
            ))
            .when_some(
                inspector_open.then_some(active_session).flatten(),
                |parent, session| {
                    parent.child(render_inspector(
                        &session,
                        inspector_focus,
                        self.tools_expanded,
                        window,
                        cx,
                    ))
                },
            )
            .when(self.pending_quit_confirm, |parent| {
                parent.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::rgba(0x0000_0080))
                        .child(
                            v_flex()
                                .w(px(360.))
                                .gap_3()
                                .p_4()
                                .rounded_md()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.background)
                                .child(
                                    Label::new(if quit_in_progress {
                                        "Aborting runs and quitting…"
                                    } else {
                                        "Abort run and quit?"
                                    })
                                    .font_weight(gpui::FontWeight::MEDIUM),
                                )
                                .when(!quit_in_progress, |card| {
                                    card.child(
                                        Label::new("Yes / No · y / n · Esc = No")
                                            .text_xs()
                                            .text_color(theme.muted_foreground),
                                    )
                                })
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Button::new("confirm-quit-yes")
                                                .label("Yes")
                                                .small()
                                                .danger()
                                                .disabled(quit_in_progress)
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _window, cx| {
                                                        this.confirm_pending_quit(cx);
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new("confirm-quit-no")
                                                .label("No")
                                                .small()
                                                .ghost()
                                                .disabled(quit_in_progress)
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _window, cx| {
                                                        this.cancel_pending_quit(cx);
                                                    },
                                                )),
                                        ),
                                ),
                        ),
                )
            })
    }
}
pub(crate) fn context_percent_label(v: Option<&serde_json::Value>) -> Option<String> {
    context_percent(v).map(|pct| format!("{pct:.0}%"))
}

// The value is clamped to 0–100 before conversion; progress rendering needs f32.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn context_percent(v: Option<&serde_json::Value>) -> Option<f32> {
    let v = v?;
    v.get("percent")
        .and_then(serde_json::Value::as_f64)
        .or_else(|| v.as_f64())
        .filter(|pct| pct.is_finite())
        .map(|pct| pct.clamp(0.0, 100.0) as f32)
}

pub(crate) fn reveal_in_file_manager(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "revealing the Pimiento home folder is unsupported on this platform",
        ))
    }
}

pub(crate) fn tokens_per_second_label(v: Option<&serde_json::Value>) -> Option<String> {
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

pub(crate) fn fast_mode_label(enabled: Option<bool>, active: Option<bool>) -> &'static str {
    match (enabled, active) {
        (_, Some(true)) => "fast:active",
        (Some(true), _) => "fast:on",
        (Some(false), _) => "fast:off",
        (None, _) => "fast:?",
    }
}
