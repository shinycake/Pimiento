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
    pub(crate) phase: RunPhase,
    pub(crate) cwd: PathBuf,
    pub(crate) attention: RailAttention,
    pub(crate) session_file: Option<PathBuf>,
}

pub(crate) fn workspace_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| path.display().to_string(), str::to_owned)
}

/// Compact cwd for the session rail: `~/…` when under home, end-truncated to fit.
pub(crate) fn rail_cwd_label(path: &Path, home: Option<&Path>) -> String {
    const MAX_CHARS: usize = 34;
    let display = match home {
        Some(home) => match path.strip_prefix(home) {
            Ok(stripped) if stripped.as_os_str().is_empty() => "~".to_owned(),
            Ok(stripped) => format!("~/{}", stripped.display()),
            Err(_) => path.display().to_string(),
        },
        None => path.display().to_string(),
    };
    let count = display.chars().count();
    if count <= MAX_CHARS {
        return display;
    }
    let keep = display
        .chars()
        .skip(count - (MAX_CHARS - 1))
        .collect::<String>();
    format!("…{keep}")
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

pub(crate) fn workspace_status_for_entries(entries: &[RailEntry]) -> StatusKind {
    entries
        .iter()
        .max_by_key(|entry| match entry.phase {
            RunPhase::Dead => 4,
            RunPhase::AwaitingResume => 3,
            RunPhase::Streaming
            | RunPhase::Compacting
            | RunPhase::Retrying
            | RunPhase::Restarting => 2,
            RunPhase::Idle => 1,
        })
        .map_or(StatusKind::Idle, |entry| {
            StatusKind::from_run_phase(&entry.phase)
        })
}

pub(crate) fn run_phase_label(phase: &RunPhase) -> &'static str {
    match phase {
        RunPhase::Idle => "Idle",
        RunPhase::Streaming => "Working",
        RunPhase::AwaitingResume => "Awaiting input",
        RunPhase::Compacting | RunPhase::Retrying | RunPhase::Restarting => "Busy",
        RunPhase::Dead => "Error",
    }
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
        let inspector_open = persistence.load_inspector_open();
        first.update(cx, |session, _cx| {
            session.inspector_open = inspector_open;
        });
        Self {
            sessions: vec![first],
            session_subscriptions: vec![first_subscription],
            active: 0,
            inspector_open,
            rail_collapsed: persistence.load_rail_collapsed(),
            persistence,
            initial_cwd,
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
        self.push_launcher_session(self.initial_cwd.clone(), window, cx);
    }

    pub(crate) fn push_launcher_session(
        &mut self,
        cwd: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<SessionView> {
        let remembered = self.persistence.load_recent_sessions();
        let last_session = self
            .persistence
            .load_last_session()
            .filter(|resume| resume.exists());
        let recent = collect_launcher_sessions(
            &self.persistence,
            &cwd,
            omp_sessions_root().as_deref(),
            home_dir().as_deref(),
            std::env::temp_dir().as_path(),
        );
        let persistence = self.persistence.clone();
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
        self.sessions.push(session.clone());
        self.session_subscriptions.push(subscription);
        self.active = self.sessions.len() - 1;
        self.sync_inspector_open_to_sessions(cx);
        cx.notify();
        session
    }

    pub(crate) fn add_session_for_cwd(
        &mut self,
        cwd: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.push_launcher_session(cwd.clone(), window, cx);
        session.update(cx, |session, cx| {
            session.begin_connection(window, cwd, None, false, cx);
        });
    }

    #[allow(clippy::unused_self)] // instance API for listeners; work is in the path-prompt future
    pub(crate) fn prompt_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select a workspace directory".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().find(|path| path.is_dir()) else {
                return;
            };
            let _ = view.update_in(cx, |this, window, cx| {
                this.add_session_for_cwd(path, window, cx);
            });
        })
        .detach();
    }

    pub(crate) fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sessions.is_empty() {
            return;
        }
        self.close_session_at(self.active, window, cx);
    }

    pub(crate) fn close_session_at(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.sessions.len() {
            return;
        }
        if let Some(session) = self.sessions.get(index).cloned() {
            let forget = session.read(cx).rail_entry(index).session_file;
            if let Some(path) = forget {
                self.persistence.forget_session(&path);
            }
            session.update(cx, SessionView::shutdown_session);
        }
        self.sessions.remove(index);
        drop(self.session_subscriptions.remove(index));
        if self.sessions.is_empty() {
            self.add_session(window, cx);
        } else {
            self.clamp_active();
            cx.notify();
        }
    }

    pub(crate) fn toggle_rail(&mut self, cx: &mut Context<Self>) {
        self.rail_collapsed = !self.rail_collapsed;
        self.persistence.save_rail_collapsed(self.rail_collapsed);
        cx.notify();
    }

    pub(crate) fn open_inspector(&mut self, focus: InspectorFocus, cx: &mut Context<Self>) {
        self.inspector_open = true;
        self.persistence.save_inspector_open(true);
        self.inspector_focus = focus;
        self.sync_inspector_open_to_sessions(cx);
        if let Some(session) = self.sessions.get(self.active).cloned() {
            session.update(cx, SessionView::ensure_subagent_snapshots);
        }
        cx.notify();
    }

    pub(crate) fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector_open = !self.inspector_open;
        self.persistence.save_inspector_open(self.inspector_open);
        self.sync_inspector_open_to_sessions(cx);
        if self.inspector_open
            && let Some(session) = self.sessions.get(self.active).cloned()
        {
            session.update(cx, SessionView::ensure_subagent_snapshots);
        }
        cx.notify();
    }

    pub(crate) fn sync_inspector_open_to_sessions(&self, cx: &mut Context<Self>) {
        let open = self.inspector_open;
        for session in &self.sessions {
            session.update(cx, |session, cx| {
                if session.inspector_open != open {
                    session.inspector_open = open;
                    cx.notify();
                }
            });
        }
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
        // Cmd/Ctrl+Shift+P — command palette (VS Code / Zed muscle memory).
        if mods.shift && matches!(key, "p" | "P") {
            if let Some(session) = self.sessions.get(self.active).cloned() {
                session.update(cx, SessionView::toggle_palette);
            }
            return true;
        }
        if mods.shift {
            // Other Shift+chords (e.g. future) are not workspace shortcuts.
            return false;
        }
        if let Some(digit) = workspace_digit_key(key) {
            let index = digit.saturating_sub(1);
            if index < self.sessions.len() {
                self.select_session(index, cx);
                return true;
            }
            return false;
        }
        let _ = window;
        false
    }

    pub(crate) fn rename_session_at(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.sessions.len() {
            return;
        }
        self.select_session(index, cx);
        if let Some(session) = self.sessions.get(index).cloned() {
            session.update(cx, |session, cx| {
                session.rename_session(window, cx);
            });
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolNameGroups {
    pub(crate) builtin: Vec<String>,
    pub(crate) extensions: Vec<String>,
}

pub(crate) fn group_tool_names(names: &[String]) -> ToolNameGroups {
    let (builtin, extensions) = names
        .iter()
        .cloned()
        .partition(|name| is_builtin_tool_name(name));
    ToolNameGroups {
        builtin,
        extensions,
    }
}

pub(crate) fn is_builtin_tool_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "read"
            | "bash"
            | "ask"
            | "eval"
            | "glob"
            | "grep"
            | "task"
            | "hub"
            | "todo"
            | "web_search"
            | "web_fetch"
            | "write"
            | "edit"
            | "ast_edit"
            | "lsp"
            | "ls"
            | "find"
            | "fetch"
            | "patch"
            | "apply_patch"
            | "skill"
            | "skills"
    )
}

pub(crate) fn mode_indicators(tool_names: &[String], widget_keys: &[String]) -> Vec<&'static str> {
    const MODES: [(&str, &str); 3] = [
        ("computer", "Computer"),
        ("browser", "Browser"),
        ("vision", "Vision"),
    ];
    MODES
        .into_iter()
        .filter_map(|(needle, label)| {
            tool_names
                .iter()
                .chain(widget_keys)
                .any(|value| value.to_ascii_lowercase().contains(needle))
                .then_some(label)
        })
        .collect()
}

/// Button content for user/OMP-provided labels must wrap instead of inheriting
/// the button primitive's single-line label treatment.
pub(crate) fn wrapped_button_text(text: impl Into<String>) -> impl IntoElement {
    div()
        .w_full()
        .min_w_0()
        .child(soft_wrap_dynamic_text(&text.into()))
}

/// Add invisible wrap opportunities without removing any visible text.
///
/// GPUI wraps at whitespace but paths, model ids, URLs, and JSON tokens can be
/// arbitrarily long. Break opportunities after separators and within long
/// uninterrupted runs keep those values inside responsive containers.
pub(crate) fn soft_wrap_dynamic_text(text: &str) -> String {
    const LONG_RUN: usize = 24;
    let mut wrapped = String::with_capacity(text.len());
    let mut run = 0usize;
    for ch in text.chars() {
        wrapped.push(ch);
        if ch == '\n' || ch.is_whitespace() {
            run = 0;
        } else {
            run += 1;
            let is_separator = matches!(
                ch,
                '/' | '\\' | '.' | '_' | '-' | ':' | '@' | '?' | '&' | '='
            );
            if is_separator || run >= LONG_RUN {
                wrapped.push('\u{200b}');
                run = 0;
            }
        }
    }
    wrapped
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
    let inspector_width = if window.viewport_size().width < px(900.) {
        px(200.)
    } else {
        px(248.)
    };
    let (
        cwd,
        model,
        thinking,
        phase,
        context,
        tokens,
        connected,
        steering_mode,
        follow_up_mode,
        interrupt_mode,
        auto_compaction_enabled,
        auto_retry_enabled,
        queued_message_count,
        todo_phases,
        subagent_rows,
        subagent_status,
        subagent_subscription,
        tool_names,
        mode_tags,
        display_title,
        display_statuses,
        display_widgets,
        display_editor_text,
        extra_status_lines,
        git_info,
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
        let display = &session_view.projection.display;
        let tool_names = tool_names_from_state(raw_state);
        let widget_keys = display.widgets.keys().cloned().collect::<Vec<_>>();
        let mode_tags = mode_indicators(&tool_names, &widget_keys);
        let extra_status_lines = inspector_extra_status_lines(raw_state);
        let git_info = probe_git_inspector(&cwd);
        let display_title = display
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned);
        let display_statuses = display_status_lines(display);
        let display_widgets = display
            .widgets
            .iter()
            .map(|(key, raw)| (key.clone(), display_widget_lines(raw)))
            .collect::<Vec<_>>();
        let display_editor_text = display
            .editor_text
            .as_ref()
            .filter(|t| !t.is_empty())
            .cloned();
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
            session_view.client.is_some(),
            state.steering_mode.clone(),
            state.follow_up_mode.clone(),
            state.interrupt_mode.clone(),
            state.auto_compaction_enabled,
            state.auto_retry_enabled,
            state.queued_message_count.unwrap_or_default(),
            parse_todo_phases(session_view.projection.todos_raw.as_ref()),
            session_view
                .subagent_snapshots
                .iter()
                .filter_map(|snapshot| {
                    subagent_snapshot_id(snapshot)
                        .map(|id| (id.to_owned(), subagent_snapshot_summary(snapshot)))
                })
                .collect::<Vec<_>>(),
            session_view.subagent_drawer_status.clone(),
            session_view.subagent_subscription.as_wire().to_owned(),
            tool_names,
            mode_tags,
            display_title,
            display_statuses,
            display_widgets,
            display_editor_text,
            extra_status_lines,
            git_info,
        )
    };
    let todo_count = todo_open_count(&todo_phases);
    let path = cwd.display().to_string();
    let refresh_session = session.clone();
    let subscription_session = session.clone();
    let steering_session = session.clone();
    let follow_up_session = session.clone();
    let interrupt_session = session.clone();
    let auto_compaction_session = session.clone();
    let auto_retry_session = session.clone();
    let steering_label = steering_mode.unwrap_or_else(|| "unknown".to_owned());
    let follow_up_label = follow_up_mode.unwrap_or_else(|| "unknown".to_owned());
    let interrupt_label = interrupt_mode.unwrap_or_else(|| "unknown".to_owned());
    let phase_status = StatusKind::from_phase_label(&phase);

    v_flex()
        .w(inspector_width)
        .h_full()
        .flex_shrink_0()
        .overflow_y_scrollbar()
        .gap_4()
        .p_3()
        .border_l_1()
        .border_color(theme.sidebar_border)
        .bg(theme.sidebar)
        .text_color(theme.sidebar_foreground)
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Icon::new(IconName::Inspector)
                                .small()
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            Label::new("Context")
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            Label::new("⌘J")
                                .text_xs()
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            Button::new("inspector-hide")
                                .icon(IconName::PanelRightClose)
                                .tooltip("Hide context inspector (⌘J)")
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                    this.toggle_inspector(cx);
                                })),
                        ),
                ),
        )
        .child(Separator::horizontal())
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .when(focus == InspectorFocus::Session, |section| {
                    section.bg(theme.secondary).rounded_sm().p_2()
                })
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_start()
                        .gap_2()
                        .child(
                            Label::new("Session")
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.muted_foreground),
                        )
                        .child(phase_tag(&phase).small().child(phase_status.label())),
                )
                .child(
                    Label::new(soft_wrap_dynamic_text(&workspace_display_name(&cwd)))
                        .text_sm()
                        .w_full(),
                )
                .child(
                    Label::new(soft_wrap_dynamic_text(&path))
                        .text_xs()
                        .text_color(theme.muted_foreground),
                )
                .child(Label::new(soft_wrap_dynamic_text(&model)).text_xs())
                .child(
                    Label::new(soft_wrap_dynamic_text(&format!("Thinking: {thinking}")))
                        .text_xs()
                        .text_color(theme.muted_foreground),
                )
                .when(!mode_tags.is_empty(), |section| {
                    section.child(
                        h_flex().w_full().flex_wrap().gap_1().children(
                            mode_tags
                                .into_iter()
                                .map(|mode| Tag::secondary().small().child(mode)),
                        ),
                    )
                })
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
                })
                .children(extra_status_lines.into_iter().map(|line| {
                    Label::new(soft_wrap_dynamic_text(&line))
                        .text_xs()
                        .text_color(theme.muted_foreground)
                })),
        )
        .child(Separator::horizontal())
        .child(
            v_flex()
                .w_full()
                .gap_2()
                .p_2()
                .rounded_sm()
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(
                            Label::new("Queue")
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.muted_foreground),
                        )
                        .when(queued_message_count > 0, |row| {
                            row.child(
                                Tag::secondary()
                                    .small()
                                    .child(format!("queue:{queued_message_count}")),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Label::new("Steering").text_xs()),
                        )
                        .child(
                            Button::new("inspector-steering-mode")
                                .small()
                                .ghost()
                                .max_w(gpui::relative(0.65))
                                .child(wrapped_button_text(steering_label))
                                .disabled(!connected)
                                .on_click(cx.listener(
                                    move |_this, _: &ClickEvent, _window, cx| {
                                        steering_session.update(cx, |session, cx| {
                                            session.toggle_steering_mode(cx);
                                        });
                                    },
                                )),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Label::new("Follow-up").text_xs()),
                        )
                        .child(
                            Button::new("inspector-follow-up-mode")
                                .small()
                                .ghost()
                                .max_w(gpui::relative(0.65))
                                .child(wrapped_button_text(follow_up_label))
                                .disabled(!connected)
                                .on_click(cx.listener(
                                    move |_this, _: &ClickEvent, _window, cx| {
                                        follow_up_session.update(cx, |session, cx| {
                                            session.toggle_follow_up_mode(cx);
                                        });
                                    },
                                )),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Label::new("Interrupt").text_xs()),
                        )
                        .child(
                            Button::new("inspector-interrupt-mode")
                                .small()
                                .ghost()
                                .max_w(gpui::relative(0.65))
                                .child(wrapped_button_text(interrupt_label))
                                .disabled(!connected)
                                .on_click(cx.listener(
                                    move |_this, _: &ClickEvent, _window, cx| {
                                        interrupt_session.update(cx, |session, cx| {
                                            session.toggle_interrupt_mode(cx);
                                        });
                                    },
                                )),
                        ),
                )
                .child(
                    Switch::new("inspector-auto-compaction")
                        .label("Auto compact")
                        .small()
                        .checked(auto_compaction_enabled.unwrap_or(false))
                        .disabled(!connected)
                        .on_click(cx.listener(move |_this, _checked: &bool, _window, cx| {
                            auto_compaction_session.update(cx, |session, cx| {
                                session.toggle_auto_compaction(cx);
                            });
                        })),
                )
                .child(
                    Switch::new("inspector-auto-retry")
                        .label("Auto retry")
                        .small()
                        .checked(auto_retry_enabled.unwrap_or(false))
                        .disabled(!connected)
                        .on_click(cx.listener(move |_this, _checked: &bool, _window, cx| {
                            auto_retry_session.update(cx, |session, cx| {
                                session.toggle_auto_retry(cx);
                            });
                        })),
                ),
        )
        .when_some(git_info, |parent, git| {
            let head = git.head_line();
            let diff = git.diff_line();
            let working = git.working_tree_line();
            let sync = git.sync_line();
            let remote_line = match (&git.remote, &git.fetch_age) {
                (Some(remote), Some(age)) => Some(format!("{remote} · fetched {age}")),
                (Some(remote), None) => Some(format!("{remote} · never fetched")),
                (None, Some(age)) => Some(format!("fetched {age}")),
                (None, None) => None,
            };
            parent.child(Separator::horizontal()).child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        Label::new("Git")
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.muted_foreground),
                    )
                    .child(inspector_kv_row("Branch", git.branch_or_detached, &theme))
                    .child(inspector_kv_row("Upstream", sync, &theme))
                    .when_some(head, |section, line| {
                        section.child(inspector_kv_row("HEAD", line, &theme))
                    })
                    .child(inspector_kv_row("Working", working, &theme))
                    .when_some(diff, |section, line| {
                        section.child(inspector_kv_row("Diff", line, &theme))
                    })
                    .when_some(remote_line, |section, line| {
                        section.child(inspector_kv_row("Remote", line, &theme))
                    })
                    .when(git.stash_count > 0, |section| {
                        section.child(inspector_kv_row(
                            "Stash",
                            format!("{}", git.stash_count),
                            &theme,
                        ))
                    })
                    .when_some(git.worktree_label, |section, label| {
                        section.child(inspector_kv_row("Worktree", label, &theme))
                    }),
            )
        })
        .when(
            display_title.is_some()
                || !display_statuses.is_empty()
                || !display_widgets.is_empty()
                || display_editor_text.is_some(),
            |parent| {
                parent.child(Separator::horizontal()).child(
                    v_flex()
                        .w_full()
                        .gap_1()
                        .child(
                            Label::new("Display")
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.muted_foreground),
                        )
                        .when_some(display_title, |section, title| {
                            section.child(
                                Label::new(soft_wrap_dynamic_text(&format!("Title: {title}")))
                                    .text_xs(),
                            )
                        })
                        .children(display_statuses.into_iter().map(|(key, text)| {
                            Label::new(soft_wrap_dynamic_text(&format!("{key}: {text}")))
                                .text_xs()
                                .text_color(theme.muted_foreground)
                        }))
                        .children(display_widgets.into_iter().map(|(key, lines)| {
                            v_flex()
                                .w_full()
                                .gap_0p5()
                                .child(
                                    Label::new(soft_wrap_dynamic_text(&format!("widget:{key}")))
                                        .text_xs()
                                        .text_color(theme.muted_foreground),
                                )
                                .when(lines.is_empty(), |slot| {
                                    slot.child(
                                        Label::new("(empty)")
                                            .text_xs()
                                            .text_color(theme.muted_foreground),
                                    )
                                })
                                .children(lines.into_iter().map(|line| {
                                    Label::new(soft_wrap_dynamic_text(&line)).text_xs()
                                }))
                        }))
                        .when_some(display_editor_text, |section, text| {
                            section.child(
                                v_flex()
                                    .w_full()
                                    .gap_0p5()
                                    .child(
                                        Label::new("Editor text")
                                            .text_xs()
                                            .text_color(theme.muted_foreground),
                                    )
                                    .child(
                                        Label::new(soft_wrap_dynamic_text(&text))
                                            .text_xs()
                                            .w_full(),
                                    ),
                            )
                        }),
                )
            },
        )
        .when(!todo_phases.is_empty(), |parent| {
            parent.child(Separator::horizontal()).child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .when(focus == InspectorFocus::Checklist, |section| {
                        section.bg(theme.secondary).rounded_sm().p_2()
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        Icon::new(IconName::CircleCheck)
                                            .xsmall()
                                            .text_color(theme.muted_foreground),
                                    )
                                    .child(
                                        Label::new("Checklist")
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(theme.muted_foreground),
                                    ),
                            )
                            .child(Tag::secondary().small().child(todo_count.to_string())),
                    )
                    .children(todo_phases.iter().enumerate().map(|(phase_ix, phase)| {
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                Label::new(soft_wrap_dynamic_text(&phase.name))
                                    .text_xs()
                                    .text_color(theme.muted_foreground),
                            )
                            .children(phase.tasks.iter().enumerate().map(|(task_ix, task)| {
                                render_todo_task_editable(
                                    task, phase_ix, task_ix, connected, session, window, &theme,
                                )
                            }))
                    })),
            )
        })
        .child(Separator::horizontal())
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .when(focus == InspectorFocus::Agents, |section| {
                    section.bg(theme.secondary).rounded_sm().p_2()
                })
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_start()
                        .gap_2()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    Icon::new(IconName::Bot)
                                        .xsmall()
                                        .text_color(theme.muted_foreground),
                                )
                                .child(
                                    Label::new("Agents")
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme.muted_foreground),
                                ),
                        )
                        .child(
                            h_flex()
                                .items_start()
                                .flex_wrap()
                                .justify_end()
                                .gap_1()
                                .child(
                                    Button::new("inspector-agents-subscription")
                                        .icon(IconName::Network)
                                        .small()
                                        .ghost()
                                        .max_w(gpui::relative(0.72))
                                        .child(wrapped_button_text(subagent_subscription.clone()))
                                        .tooltip("Cycle agent event subscription")
                                        .disabled(!connected)
                                        .on_click(window.listener_for(
                                            &subscription_session,
                                            |this, _: &ClickEvent, _window, cx| {
                                                this.cycle_subagent_subscription(cx);
                                            },
                                        )),
                                )
                                .child(
                                    Button::new("inspector-agents-refresh")
                                        .icon(IconName::Redo2)
                                        .tooltip("Refresh agents")
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
                        ),
                )
                .when(
                    !(subagent_status.is_empty()
                        || subagent_rows.is_empty() && subagent_status == "No agents reported"),
                    |section| {
                        section.child(
                            Label::new(soft_wrap_dynamic_text(&subagent_status))
                                .text_xs()
                                .text_color(theme.muted_foreground),
                        )
                    },
                )
                .children(subagent_rows.iter().enumerate().map(|(ix, (id, summary))| {
                    let id = id.clone();
                    Button::new(("inspector-agent", ix))
                        .small()
                        .w_full()
                        .ghost()
                        .child(wrapped_button_text(summary.clone()))
                        .on_click(window.listener_for(
                            session,
                            move |this, _: &ClickEvent, _window, cx| {
                                this.open_subagent_modal(id.clone(), cx);
                            },
                        ))
                })),
        )
        .when(!tool_names.is_empty(), |parent| {
            let grouped = group_tool_names(&tool_names);
            parent.child(Separator::horizontal()).child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(if tool_names.len() > 8 {
                        Button::new("inspector-tools-toggle")
                            .icon(if tools_expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .label(format!("Tools ({})", tool_names.len()))
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
                    .when(tool_names.len() <= 8 || tools_expanded, |section| {
                        let visible_builtin = grouped.builtin.len().min(12);
                        let visible_extensions = grouped.extensions.len().min(12);
                        let hidden = tool_names
                            .len()
                            .saturating_sub(visible_builtin + visible_extensions);
                        section
                            .when(!grouped.builtin.is_empty(), |section| {
                                section.child(
                                    v_flex()
                                        .w_full()
                                        .gap_1()
                                        .child(
                                            Label::new("Builtin")
                                                .text_xs()
                                                .text_color(theme.muted_foreground),
                                        )
                                        .child(h_flex().w_full().flex_wrap().gap_1().children(
                                            grouped.builtin.iter().take(12).map(|name| {
                                                Tag::secondary()
                                                    .small()
                                                    .child(soft_wrap_dynamic_text(name))
                                            }),
                                        )),
                                )
                            })
                            .when(!grouped.extensions.is_empty(), |section| {
                                section.child(
                                    v_flex()
                                        .w_full()
                                        .gap_1()
                                        .child(
                                            Label::new("Extensions / MCP")
                                                .text_xs()
                                                .text_color(theme.muted_foreground),
                                        )
                                        .child(h_flex().w_full().flex_wrap().gap_1().children(
                                            grouped.extensions.iter().take(12).map(|name| {
                                                Tag::secondary()
                                                    .small()
                                                    .child(soft_wrap_dynamic_text(name))
                                            }),
                                        )),
                                )
                            })
                            .when(hidden > 0, |section| {
                                section.child(
                                    Label::new(format!("+{hidden} more"))
                                        .text_xs()
                                        .text_color(theme.muted_foreground),
                                )
                            })
                    }),
            )
        })
        .into_any_element()
}

impl Render for WorkspaceView {
    #[allow(clippy::too_many_lines)]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.clamp_active();
        let rail_width = if window.viewport_size().width < px(900.) {
            px(184.)
        } else {
            px(240.)
        };
        if let Some(session) = self.sessions.get(self.active).cloned() {
            let pending =
                session.update(cx, |session, _cx| session.take_pending_workspace_palette());
            if let Some(action) = pending {
                self.run_workspace_palette_action(action, window, cx);
            }
            let pending_cwd = session.update(cx, |session, _cx| session.take_pending_new_tab_cwd());
            if let Some(cwd) = pending_cwd {
                self.add_session_for_cwd(cwd, window, cx);
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
            .on_action(cx.listener(Self::handle_about_menu))
            .on_action(cx.listener(Self::handle_open_workspace_menu))
            .on_action(cx.listener(Self::handle_new_session_menu))
            .on_action(cx.listener(Self::handle_close_session_menu))
            .on_action(cx.listener(Self::handle_palette_menu))
            .on_action(cx.listener(Self::handle_theme_menu))
            .on_action(cx.listener(Self::handle_toggle_rail_menu))
            .on_action(cx.listener(Self::handle_toggle_inspector_menu))
            .on_action(cx.listener(Self::handle_rename_menu))
            .on_action(cx.listener(Self::handle_branch_menu))
            .on_action(cx.listener(Self::handle_export_menu))
            .on_action(cx.listener(Self::handle_share_menu))
            .on_action(cx.listener(Self::handle_abort_menu))
            .on_action(cx.listener(Self::handle_minimize_menu))
            .on_action(cx.listener(Self::handle_zoom_menu))
            .on_action(cx.listener(Self::handle_fullscreen_menu))
            .when(!self.rail_collapsed, |parent| {
                let home = home_dir();
                parent.child(
                    v_flex()
                        .w(rail_width)
                        .h_full()
                        .border_r_1()
                        .border_color(theme.sidebar_border)
                        .bg(theme.sidebar)
                        .text_color(theme.sidebar_foreground)
                        // Top chrome — workspace actions, separated from session list.
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .pt_2()
                                .pb_2()
                                .child(
                                    Button::new("workspace-new-workspace")
                                        .icon(IconName::FolderOpen)
                                        .label("Workspace…")
                                        .tooltip("Open workspace directory")
                                        .small()
                                        .ghost()
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                this.prompt_new_workspace(window, cx);
                                            },
                                        )),
                                )
                                .child(div().flex_1())
                                .child(
                                    Button::new("workspace-hide-rail")
                                        .icon(IconName::PanelLeftClose)
                                        .tooltip("Hide sessions (⌘B)")
                                        .small()
                                        .ghost()
                                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                            this.toggle_rail(cx);
                                        })),
                                ),
                        )
                        .child(Separator::horizontal())
                        // Session list — workspaces as section headers, sessions nested.
                        .child(
                            v_flex()
                                .w_full()
                                .flex_1()
                                .px_2()
                                .pt_2()
                                .pb_2()
                                .gap_3()
                                .overflow_y_scrollbar()
                                .children(groups.into_iter().enumerate().map(
                                    |(group_ix, (cwd, entries))| {
                                        let cwd_for_add = cwd.clone();
                                        let display = workspace_display_name(&cwd);
                                        let group_status = workspace_status_for_entries(&entries);
                                        let path_label =
                                            rail_cwd_label(&cwd, home.as_deref());
                                        let group_count = entries.len();
                                        v_flex()
                                            .w_full()
                                            .gap_1()
                                            .when(group_ix > 0, |group| {
                                                group.child(Separator::horizontal())
                                            })
                                            // Workspace header — folder identity + path meta.
                                            .child(
                                                v_flex()
                                                    .w_full()
                                                    .gap_1()
                                                    .px_2()
                                                    .py_1()
                                                    .rounded_sm()
                                                    .bg(theme.secondary.opacity(0.55))
                                                    .child(
                                                        h_flex()
                                                            .w_full()
                                                            .items_center()
                                                            .gap_2()
                                                            .child(
                                                                Icon::new(IconName::FolderOpen)
                                                                    .small()
                                                                    .text_color(
                                                                        theme.muted_foreground,
                                                                    ),
                                                            )
                                                            .child(
                                                                Label::new(
                                                                    soft_wrap_dynamic_text(
                                                                        &display,
                                                                    ),
                                                                )
                                                                .text_sm()
                                                                .font_weight(
                                                                    gpui::FontWeight::SEMIBOLD,
                                                                )
                                                                .flex_1()
                                                                .min_w_0(),
                                                            )
                                                            .when(
                                                                group_status != StatusKind::Idle,
                                                                |row| {
                                                                    row.child(
                                                                        group_status
                                                                            .tag()
                                                                            .small()
                                                                            .child(
                                                                                group_status
                                                                                    .label(),
                                                                            ),
                                                                    )
                                                                },
                                                            )
                                                            .child(
                                                                Button::new((
                                                                    "workspace-add-session",
                                                                    group_ix,
                                                                ))
                                                                .icon(IconName::Plus)
                                                                .tooltip(
                                                                    "Add session in this workspace",
                                                                )
                                                                .small()
                                                                .ghost()
                                                                .on_click(cx.listener(
                                                                    move |this,
                                                                          _: &ClickEvent,
                                                                          window,
                                                                          cx| {
                                                                        this.add_session_for_cwd(
                                                                            cwd_for_add.clone(),
                                                                            window,
                                                                            cx,
                                                                        );
                                                                    },
                                                                )),
                                                            ),
                                                    )
                                                    .child(
                                                        h_flex()
                                                            .w_full()
                                                            .items_center()
                                                            .gap_2()
                                                            .pl(px(22.))
                                                            .child(
                                                                Label::new(
                                                                    soft_wrap_dynamic_text(
                                                                        &path_label,
                                                                    ),
                                                                )
                                                                .text_xs()
                                                                .text_color(
                                                                    theme.muted_foreground,
                                                                )
                                                                .flex_1()
                                                                .min_w_0(),
                                                            )
                                                            .child(
                                                                Label::new(if group_count == 1 {
                                                                    "1 session".to_owned()
                                                                } else {
                                                                    format!(
                                                                        "{group_count} sessions"
                                                                    )
                                                                })
                                                                .text_xs()
                                                                .text_color(
                                                                    theme.muted_foreground,
                                                                ),
                                                            ),
                                                    ),
                                            )
                                            // Nested session rows — visually under the workspace.
                                            .child(
                                                v_flex().w_full().gap_0().pl_1().children(
                                                    entries.into_iter().map(|entry| {
                                                        let selected = entry.ix == active;
                                                        let ix = entry.ix;
                                                        let phase_label =
                                                            run_phase_label(&entry.phase);
                                                        let attention_color =
                                                            match entry.attention {
                                                                RailAttention::Quiet => None,
                                                                RailAttention::Active => {
                                                                    Some(theme.info)
                                                                }
                                                                RailAttention::Unread => {
                                                                    Some(theme.warning)
                                                                }
                                                            };
                                                        h_flex()
                                                            .id(("workspace-session", ix))
                                                            .group("rail-row")
                                                            .w_full()
                                                            .justify_between()
                                                            .items_center()
                                                            .gap_2()
                                                            .px_2()
                                                            .py_1()
                                                            .rounded_sm()
                                                            .cursor_pointer()
                                                            .when(selected, |row| {
                                                                row.bg(theme.sidebar_accent)
                                                            })
                                                            .when(!selected, |row| {
                                                                row.hover(|row| {
                                                                    row.bg(theme.secondary)
                                                                })
                                                            })
                                                            .child(
                                                                h_flex()
                                                                    .flex_1()
                                                                    .min_w_0()
                                                                    .items_center()
                                                                    .gap_2()
                                                                    .child(
                                                                        // Attention / quiet slot keeps titles aligned.
                                                                        div()
                                                                            .size(px(7.))
                                                                            .rounded_full()
                                                                            .flex_shrink_0()
                                                                            .when_some(
                                                                                attention_color,
                                                                                |dot, color| {
                                                                                    dot.bg(color)
                                                                                },
                                                                            )
                                                                            .when(
                                                                                attention_color
                                                                                    .is_none(),
                                                                                |dot| {
                                                                                    dot.bg(
                                                                                        theme
                                                                                            .muted_foreground
                                                                                            .opacity(0.28),
                                                                                    )
                                                                                },
                                                                            ),
                                                                    )
                                                                    .child(
                                                                        Label::new(
                                                                            soft_wrap_dynamic_text(
                                                                                &entry.label,
                                                                            ),
                                                                        )
                                                                        .text_sm()
                                                                        .flex_1()
                                                                        .min_w_0()
                                                                        .when(
                                                                            selected,
                                                                            |label| {
                                                                                label.font_weight(
                                                                                    gpui::FontWeight::MEDIUM,
                                                                                )
                                                                            },
                                                                        ),
                                                                    ),
                                                            )
                                                            .child(
                                                                status_pill_for_phase(
                                                                    &entry.phase,
                                                                )
                                                                .small()
                                                                .child(phase_label),
                                                            )
                                                            .child(
                                                                Button::new((
                                                                    "workspace-close-session",
                                                                    ix,
                                                                ))
                                                                .icon(IconName::Close)
                                                                .tooltip("Close session")
                                                                .small()
                                                                .ghost()
                                                                .invisible()
                                                                .group_hover(
                                                                    "rail-row",
                                                                    gpui::Styled::visible,
                                                                )
                                                                .on_click(cx.listener(
                                                                    move |this,
                                                                          _: &ClickEvent,
                                                                          window,
                                                                          cx| {
                                                                        this.close_session_at(
                                                                            ix, window, cx,
                                                                        );
                                                                    },
                                                                )),
                                                            )
                                                            .on_click(cx.listener(
                                                                move |this,
                                                                      _: &ClickEvent,
                                                                      _window,
                                                                      cx| {
                                                                    this.select_session(ix, cx);
                                                                },
                                                            ))
                                                            .context_menu({
                                                                let workspace = cx.weak_entity();
                                                                move |menu, _window, _cx| {
                                                                    menu.item(
                                                                        PopupMenuItem::new(
                                                                            "Rename",
                                                                        )
                                                                        .on_click({
                                                                            let workspace =
                                                                                workspace
                                                                                    .clone();
                                                                            move |_,
                                                                                  window,
                                                                                  cx| {
                                                                                let _ = workspace
                                                                                    .update(
                                                                                        cx,
                                                                                        |this,
                                                                                         cx| {
                                                                                            this.rename_session_at(
                                                                                                ix, window, cx,
                                                                                            );
                                                                                        },
                                                                                    );
                                                                            }
                                                                        }),
                                                                    )
                                                                    .separator()
                                                                    .item(
                                                                        PopupMenuItem::new(
                                                                            "Close",
                                                                        )
                                                                        .on_click({
                                                                            let workspace =
                                                                                workspace
                                                                                    .clone();
                                                                            move |_,
                                                                                  window,
                                                                                  cx| {
                                                                                let _ = workspace
                                                                                    .update(
                                                                                        cx,
                                                                                        |this,
                                                                                         cx| {
                                                                                            this.close_session_at(
                                                                                                ix, window, cx,
                                                                                            );
                                                                                        },
                                                                                    );
                                                                            }
                                                                        }),
                                                                    )
                                                                }
                                                            })
                                                    }),
                                                ),
                                            )
                                    },
                                )),
                        ),
                )
            })
            .when(self.rail_collapsed, |parent| {
                parent.child(
                    v_flex()
                        .id("workspace-show-rail")
                        .w(px(40.))
                        .h_full()
                        .items_center()
                        .gap_2()
                        .pt_2()
                        .border_r_1()
                        .border_color(theme.sidebar_border)
                        .bg(theme.sidebar)
                        .text_color(theme.sidebar_foreground)
                        .child(
                            Button::new("workspace-show-rail-button")
                                .icon(IconName::PanelLeftOpen)
                                .tooltip("Show sessions (⌘B)")
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    this.toggle_rail(cx);
                                })),
                        ),
                )
            })
            .child(div().flex_1().min_w(px(0.)).h_full().child(
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
            .when(!inspector_open, |parent| {
                parent.child(
                    v_flex()
                        .id("workspace-show-inspector")
                        .w(px(40.))
                        .h_full()
                        .items_center()
                        .gap_1()
                        .pt_3()
                        .border_l_1()
                        .border_color(theme.sidebar_border)
                        .bg(theme.sidebar)
                        .text_color(theme.sidebar_foreground)
                        .child(
                            Button::new("workspace-show-inspector-button")
                                .icon(IconName::PanelRightOpen)
                                .tooltip("Show context inspector (⌘J)")
                                .small()
                                .ghost()
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _w, cx| {
                                        this.toggle_inspector(cx);
                                    },
                                )),
                        )
                        .child(
                            Label::new("⌘J")
                                .text_xs()
                                .text_color(theme.muted_foreground),
                        ),
                )
            })
            .when(self.pending_quit_confirm, |parent| {
                parent.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_4()
                        .bg(theme.overlay)
                        .child(
                            v_flex()
                                .w_full()
                                .max_w(px(360.))
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

fn inspector_kv_row(
    key: &str,
    value: impl Into<String>,
    theme: &gpui_component::Theme,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_2()
        .items_start()
        .child(
            Label::new(key)
                .text_xs()
                .w(px(64.))
                .flex_shrink_0()
                .text_color(theme.muted_foreground),
        )
        .child(
            Label::new(soft_wrap_dynamic_text(&value.into()))
                .text_xs()
                .flex_1()
                .min_w_0(),
        )
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
    reveal_path_in_file_manager(path)
}

/// Reveal an existing file or directory in the platform file manager.
pub(crate) fn reveal_path_in_file_manager(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        if path.is_file() {
            cmd.arg("-R");
        }
        cmd.arg(path).spawn()?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        // xdg-open opens files; for directories it opens the folder.
        let target = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        std::process::Command::new("xdg-open").arg(target).spawn()?;
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "revealing paths is unsupported on this platform",
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

/// Presentation helper for enabled vs active fast mode (used by tests / notices).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn fast_mode_label(enabled: Option<bool>, active: Option<bool>) -> &'static str {
    match (enabled, active) {
        (_, Some(true)) => "fast:active",
        (Some(true), _) => "fast:on",
        (Some(false), _) => "fast:off",
        (None, _) => "fast:?",
    }
}
