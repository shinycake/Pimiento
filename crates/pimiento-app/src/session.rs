use crate::*;

pub(crate) const MODEL_PICKER_VISIBLE_CAP: usize = 200;
pub(crate) const SLASH_COMMAND_VISIBLE_CAP: usize = 12;
pub(crate) const MAX_RECENT_SESSIONS: usize = 12;
pub(crate) const MAX_DISCOVERED_SESSIONS: usize = 24;
pub(crate) const SESSION_HEADER_PREFIX_BYTES: usize = 8192;
pub(crate) const ABORT_ARM_WINDOW: Duration = Duration::from_millis(1200);
pub(crate) const ABORT_ARM_STATUS: &str = "Press Esc again to abort";
pub(crate) const MIN_WINDOW_WIDTH: f32 = 480.0;
pub(crate) const MIN_WINDOW_HEIGHT: f32 = 320.0;
pub(crate) static PERSISTENCE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlashCommand {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) aliases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashMenuState {
    Closed,
    Open,
    Dismissed,
}

pub(crate) struct AbortArm {
    pub(crate) generation: u64,
    pub(crate) deadline: Instant,
    pub(crate) previous_status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LauncherPhase {
    Visible,
    Connecting,
    Hidden,
}

#[allow(clippy::struct_excessive_bools)]
pub(crate) struct SessionView {
    pub(crate) projection: SessionProjection,
    pub(crate) client: Option<RpcClient>,
    pub(crate) composer: gpui::Entity<InputState>,
    pub(crate) model_search: gpui::Entity<InputState>,
    pub(crate) model_picker_open: bool,
    pub(crate) thinking_picker_open: bool,
    /// Latest `get_subagents` response, retained losslessly for tolerant rendering.
    pub(crate) subagent_snapshots: Vec<serde_json::Value>,
    pub(crate) selected_subagent_id: Option<String>,
    pub(crate) subagent_tail_next_byte: Option<u64>,
    pub(crate) subagent_tail_lines: Vec<String>,
    pub(crate) subagent_drawer_status: String,
    pub(crate) pending_revert: Option<PendingRevert>,
    pub(crate) palette_open: bool,
    pub(crate) about_open: bool,
    pub(crate) palette_query: String,
    pub(crate) palette_selected: usize,
    pub(crate) pending_workspace_palette: Option<PaletteActionId>,
    pub(crate) slash_menu: SlashMenuState,
    pub(crate) slash_selected: usize,
    pub(crate) status_message: String,
    pub(crate) omp_version: Option<String>,
    pub(crate) abort_arm: Option<AbortArm>,
    pub(crate) abort_arm_generation: u64,
    pub(crate) available_models: Vec<ModelChoice>,
    pub(crate) expanded_tools: HashSet<String>,
    pub(crate) running_tool_started: HashMap<String, Instant>,
    pub(crate) running_tool_timer: Option<Task<()>>,
    pub(crate) clear_composer: bool,
    pub(crate) pending_composer_value: Option<String>,
    pub(crate) refocus_composer: bool,
    pub(crate) clear_model_search: bool,
    /// Virtualized transcript list (GPUI `ListState`, bottom-aligned chat).
    pub(crate) transcript_list: ListState,
    pub(crate) last_transcript_len: usize,
    /// Count of rows appended while the user was scrolled away from the tail.
    pub(crate) unread_below: usize,
    pub(crate) _subscriptions: Vec<gpui::Subscription>,
    pub(crate) pump: Option<Task<()>>,
    pub(crate) persistence: SessionPersistence,
    pub(crate) session_cwd: Option<PathBuf>,
    pub(crate) launcher_cwd: PathBuf,
    pub(crate) recent_sessions: Vec<RecentSession>,
    pub(crate) last_session: Option<PathBuf>,
    pub(crate) launcher_phase: LauncherPhase,
    pub(crate) launcher_error: Option<String>,
}

impl SessionView {
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        client: Option<RpcClient>,
        status: String,
        initial_projection: SessionProjection,
        available_models: Vec<ModelChoice>,
        bootstrap: LauncherBootstrap,
    ) -> Self {
        let LauncherBootstrap {
            persistence,
            launcher_cwd,
            recent_sessions,
            last_session,
        } = bootstrap;
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("Message · Enter to send")
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

        let launcher_phase = if client.is_none() {
            LauncherPhase::Visible
        } else {
            LauncherPhase::Hidden
        };
        let omp_version = client
            .as_ref()
            .and_then(|_| (!status.trim().is_empty()).then(|| status.trim().to_owned()));
        let mut view = Self {
            projection: initial_projection,
            client,
            composer,
            model_search,
            model_picker_open: false,
            thinking_picker_open: false,
            subagent_snapshots: Vec::new(),
            selected_subagent_id: None,
            subagent_tail_next_byte: None,
            subagent_tail_lines: Vec::new(),
            subagent_drawer_status: String::new(),
            pending_revert: None,
            palette_open: false,
            about_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            pending_workspace_palette: None,
            slash_menu: SlashMenuState::Closed,
            slash_selected: 0,
            status_message: status,
            omp_version,
            abort_arm: None,
            abort_arm_generation: 0,
            available_models,
            expanded_tools: HashSet::new(),
            running_tool_started: HashMap::new(),
            running_tool_timer: None,
            clear_composer: false,
            pending_composer_value: None,
            refocus_composer: false,
            clear_model_search: false,
            transcript_list,
            last_transcript_len: initial_len,
            unread_below: 0,
            _subscriptions: subscriptions,
            pump: None,
            persistence,
            session_cwd: None,
            launcher_cwd,
            recent_sessions,
            last_session,
            launcher_phase,
            launcher_error: None,
        };
        if let Some(client) = view.client.clone() {
            view.start_event_pump(&client, cx);
        }
        view.sync_running_tools(cx);
        view.start_catalog_load(cx);
        view
    }

    pub(crate) fn start_event_pump(&mut self, client: &RpcClient, cx: &mut Context<Self>) {
        let events = client.events();
        self.pump = Some(cx.spawn(async move |view, cx| {
            while let Ok(event) = events.recv().await {
                let _ = view.update(cx, |this, cx| {
                    match &event {
                        ClientEvent::Frame(frame) => {
                            let is_model_changed = frame.raw.get("type").and_then(|v| v.as_str())
                                == Some("model_changed");
                            this.projection.apply(frame);
                            if is_model_changed {
                                this.refresh_state(cx);
                            }
                        }
                        ClientEvent::Closed(info) => {
                            let reason = info
                                .error_msg
                                .clone()
                                .unwrap_or_else(|| format!("exit code {:?}", info.exit_code));
                            this.projection.mark_dead(reason);
                            this.client = None;
                            this.status_message = format!(
                                "OMP closed — {}",
                                info.stderr_tail.chars().take(256).collect::<String>()
                            );
                        }
                    }
                    this.sync_running_tools(cx);
                    if !this.can_abort() {
                        this.clear_abort_arm();
                    }
                    cx.notify();
                });
            }
        }));
    }

    pub(crate) fn begin_connection(
        &mut self,
        window: &Window,
        cwd: PathBuf,
        resume: Option<PathBuf>,
        launcher_mode: bool,
        cx: &mut Context<Self>,
    ) {
        if self.launcher_phase == LauncherPhase::Connecting {
            return;
        }
        self.clear_abort_arm();
        self.client.take();
        self.pump.take();
        self.launcher_phase = LauncherPhase::Connecting;
        if launcher_mode {
            self.launcher_cwd.clone_from(&cwd);
            self.launcher_error = None;
            self.session_cwd = None;
            self.projection = SessionProjection::new();
            self.clear_subagent_drawer_state();
            self.available_models.clear();
            self.model_picker_open = false;
            self.thinking_picker_open = false;
            self.expanded_tools.clear();
            self.running_tool_started.clear();
            self.running_tool_timer.take();
            self.slash_menu = SlashMenuState::Closed;
            self.slash_selected = 0;
            self.transcript_list.reset(0);
            self.last_transcript_len = 0;
            self.unread_below = 0;
            "Connecting to OMP…".clone_into(&mut self.status_message);
        } else {
            self.launcher_phase = LauncherPhase::Hidden;
            self.projection.mark_restarting();
            "Restarting session…".clone_into(&mut self.status_message);
        }
        cx.notify();

        let persistence = self.persistence.clone();
        cx.spawn_in(window, async move |view, cx| {
            let cwd_for_connect = cwd.clone();
            let resume_for_connect = resume.clone();
            let result = cx
                .background_spawn(async move {
                    try_connect_omp(
                        Some(cwd_for_connect),
                        resume_for_connect.as_deref(),
                        &persistence,
                    )
                })
                .await;
            let _ = view.update_in(cx, |this, _window, cx| {
                this.finish_connection(result, cwd, launcher_mode, cx);
            });
        })
        .detach();
    }

    pub(crate) fn finish_connection(
        &mut self,
        result: Result<(RpcClient, SessionProjection, String, Vec<ModelChoice>), String>,
        cwd: PathBuf,
        launcher_mode: bool,
        cx: &mut Context<Self>,
    ) {
        self.launcher_phase = if launcher_mode {
            LauncherPhase::Visible
        } else {
            LauncherPhase::Hidden
        };
        match result {
            Ok((client, projection, status, models)) => {
                self.available_models = models;
                self.projection = projection;
                self.omp_version = Some(status.clone());
                self.status_message = status;
                self.client = Some(client.clone());
                self.session_cwd = Some(cwd);
                self.launcher_phase = LauncherPhase::Hidden;
                self.launcher_error = None;
                self.transcript_list.reset(self.projection.transcript.len());
                self.transcript_list.set_follow_mode(FollowMode::Tail);
                self.last_transcript_len = self.projection.transcript.len();
                self.unread_below = 0;
                self.sync_running_tools(cx);
                self.start_event_pump(&client, cx);
                self.last_session = self.persistence.load_last_session();
                self.recent_sessions = self.persistence.load_recent_sessions();
                self.start_catalog_load(cx);
            }
            Err(error) => {
                if launcher_mode {
                    self.launcher_phase = LauncherPhase::Visible;
                    self.launcher_error = Some(error.clone());
                    "Unable to connect to OMP".clone_into(&mut self.status_message);
                } else {
                    self.launcher_phase = LauncherPhase::Hidden;
                    self.projection.mark_dead(error.clone());
                    self.status_message = error;
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn choose_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launcher_phase == LauncherPhase::Connecting {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select a working directory".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().find(|path| path.is_dir()) else {
                return;
            };
            let _ = view.update_in(cx, |this, _window, cx| {
                this.set_launcher_cwd(path, cx);
            });
        })
        .detach();
    }

    pub(crate) fn set_launcher_cwd(&mut self, cwd: PathBuf, cx: &mut Context<Self>) {
        self.launcher_cwd = cwd;
        self.launcher_error = None;
        self.refresh_launcher_sessions();
        cx.notify();
    }

    pub(crate) fn refresh_launcher_sessions(&mut self) {
        self.recent_sessions = collect_launcher_sessions(
            &self.persistence,
            &self.launcher_cwd,
            omp_sessions_root().as_deref(),
            home_dir().as_deref(),
            std::env::temp_dir().as_path(),
        );
        self.last_session = self
            .persistence
            .load_last_session()
            .filter(|resume| resume.exists());
    }

    pub(crate) fn return_to_launcher(&mut self, cx: &mut Context<Self>) {
        self.clear_abort_arm();
        self.client.take();
        self.pump.take();
        self.model_picker_open = false;
        self.thinking_picker_open = false;
        self.close_slash_menu();
        self.clear_composer = true;
        if let Some(cwd) = self.session_cwd.take() {
            self.launcher_cwd = cwd;
        }
        self.projection = SessionProjection::new();
        self.clear_subagent_drawer_state();
        self.available_models.clear();
        self.expanded_tools.clear();
        self.running_tool_started.clear();
        self.running_tool_timer.take();
        self.transcript_list.reset(0);
        self.last_transcript_len = 0;
        self.unread_below = 0;
        self.launcher_phase = LauncherPhase::Visible;
        self.launcher_error = None;
        "Choose a working directory or session".clone_into(&mut self.status_message);
        self.refresh_launcher_sessions();
        cx.notify();
    }

    /// Keep `ListState` item count / measurements in sync with the projection.
    ///
    /// PLAN SH: `splice`/`reset` on count changes; `remeasure_items` when a
    /// row's height may have changed (streaming growth, card expand).
    pub(crate) fn sync_transcript_list(&mut self) {
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

    pub(crate) fn sync_running_tools(&mut self, cx: &mut Context<Self>) {
        let running_ids = self
            .projection
            .transcript
            .iter()
            .filter_map(|entry| match entry {
                TranscriptEntry::ToolCall(tool) if tool.status == ToolStatus::Running => {
                    Some(tool.tool_call_id.clone())
                }
                _ => None,
            })
            .collect::<HashSet<_>>();
        self.running_tool_started
            .retain(|tool_id, _| running_ids.contains(tool_id));
        let now = Instant::now();
        for tool_id in running_ids {
            self.running_tool_started.entry(tool_id).or_insert(now);
        }

        if self.running_tool_started.is_empty() {
            self.running_tool_timer.take();
        } else if self.running_tool_timer.is_none() {
            self.running_tool_timer = Some(cx.spawn(async move |view, cx| {
                loop {
                    smol::Timer::after(Duration::from_secs(1)).await;
                    if view
                        .update(cx, |_this, cx| {
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
    }

    pub(crate) fn jump_to_transcript_tail(&mut self, cx: &mut Context<Self>) {
        self.transcript_list.scroll_to_end();
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        self.unread_below = 0;
        cx.notify();
    }

    pub(crate) fn handle_transcript_nav_key(
        &mut self,
        event: &KeyDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = &event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return false;
        }
        // capture_key_down runs before children — don't steal Home/End/Page*
        // from the composer or model search while they own focus.
        if self.composer.read(cx).focus_handle(cx).is_focused(window)
            || self
                .model_search
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            return false;
        }

        match event.keystroke.key.as_str() {
            "pageup" | "page_up" => {
                self.transcript_list.set_follow_mode(FollowMode::Normal);
                self.transcript_list.scroll_by(px(-480.));
            }
            "pagedown" | "page_down" => {
                self.transcript_list.set_follow_mode(FollowMode::Normal);
                self.transcript_list.scroll_by(px(480.));
            }
            "home" => {
                self.transcript_list.set_follow_mode(FollowMode::Normal);
                self.transcript_list.scroll_to(ListOffset {
                    item_ix: 0,
                    offset_in_item: px(0.),
                });
            }
            "end" => {
                self.jump_to_transcript_tail(cx);
                return true;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(crate) fn start_catalog_load(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn close_model_picker(&mut self, _cx: &mut Context<Self>) {
        self.model_picker_open = false;
        self.clear_model_search = true;
    }

    pub(crate) fn toggle_model_picker(&mut self, cx: &mut Context<Self>) {
        self.model_picker_open = !self.model_picker_open;
        if self.model_picker_open {
            self.thinking_picker_open = false;
        }
        if !self.model_picker_open {
            self.clear_model_search = true;
        }
        cx.notify();
    }

    pub(crate) fn close_thinking_picker(&mut self) {
        self.thinking_picker_open = false;
    }

    pub(crate) fn request_inspector_focus(
        &mut self,
        action: PaletteActionId,
        cx: &mut Context<Self>,
    ) {
        self.pending_workspace_palette = Some(action);
        cx.notify();
    }

    pub(crate) fn ensure_subagent_snapshots(&mut self, cx: &mut Context<Self>) {
        if self.subagent_snapshots.is_empty() && self.client.is_some() {
            self.refresh_subagents(cx);
        }
    }

    pub(crate) fn clear_subagent_drawer_state(&mut self) {
        self.subagent_snapshots.clear();
        self.selected_subagent_id = None;
        self.subagent_tail_next_byte = None;
        self.subagent_tail_lines.clear();
        self.subagent_drawer_status.clear();
    }

    pub(crate) fn refresh_subagents(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            "OMP is not connected".clone_into(&mut self.subagent_drawer_status);
            return;
        };
        "Loading agents…".clone_into(&mut self.subagent_drawer_status);
        cx.spawn(async move |view, cx| {
            let result = client.send(RpcCommandBody::GetSubagents).await;
            let _ = view.update(cx, |this, cx| match result {
                Ok(response) if response.success => {
                    let snapshots = response
                        .data
                        .as_ref()
                        .and_then(|data| data.get("subagents"))
                        .and_then(serde_json::Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    this.apply_subagent_snapshots(snapshots, cx);
                }
                Ok(response) => {
                    this.subagent_drawer_status = response
                        .error
                        .unwrap_or_else(|| "get_subagents failed".to_owned());
                    cx.notify();
                }
                Err(error) => {
                    this.subagent_drawer_status = format!("get_subagents: {error}");
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn apply_subagent_snapshots(
        &mut self,
        snapshots: Vec<serde_json::Value>,
        cx: &mut Context<Self>,
    ) {
        self.subagent_snapshots = snapshots;
        let selection_is_present = self.selected_subagent_id.as_ref().is_some_and(|selected| {
            self.subagent_snapshots
                .iter()
                .any(|snapshot| subagent_snapshot_id(snapshot) == Some(selected.as_str()))
        });
        if !selection_is_present {
            self.selected_subagent_id = self
                .subagent_snapshots
                .iter()
                .find_map(subagent_snapshot_id)
                .map(str::to_owned);
            self.subagent_tail_next_byte = None;
            self.subagent_tail_lines.clear();
        }

        if let Some(selected) = self.selected_subagent_id.clone() {
            let from_byte = self.subagent_tail_next_byte;
            self.fetch_subagent_messages(selected, from_byte, cx);
        } else if self.projection.subagents_raw.is_empty() {
            "No agents reported".clone_into(&mut self.subagent_drawer_status);
        } else {
            "No snapshots; showing recent agent events"
                .clone_into(&mut self.subagent_drawer_status);
        }
        cx.notify();
    }

    pub(crate) fn select_subagent(&mut self, subagent_id: String, cx: &mut Context<Self>) {
        if self.selected_subagent_id.as_deref() == Some(subagent_id.as_str()) {
            return;
        }
        self.selected_subagent_id = Some(subagent_id.clone());
        self.subagent_tail_next_byte = None;
        self.subagent_tail_lines.clear();
        self.fetch_subagent_messages(subagent_id, None, cx);
        cx.notify();
    }

    pub(crate) fn fetch_subagent_messages(
        &mut self,
        subagent_id: String,
        from_byte: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.client.clone() else {
            "OMP is not connected".clone_into(&mut self.subagent_drawer_status);
            return;
        };
        let session_file = self
            .subagent_snapshots
            .iter()
            .find(|snapshot| subagent_snapshot_id(snapshot) == Some(subagent_id.as_str()))
            .and_then(subagent_snapshot_session_file)
            .map(str::to_owned);
        self.subagent_drawer_status = if from_byte.is_some() {
            "Refreshing messages…".to_owned()
        } else {
            "Loading messages…".to_owned()
        };
        cx.spawn(async move |view, cx| {
            let result = client
                .send(RpcCommandBody::GetSubagentMessages {
                    subagent_id: Some(subagent_id.clone()),
                    session_file,
                    from_byte,
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.selected_subagent_id.as_deref() != Some(subagent_id.as_str()) {
                    return;
                }
                match result {
                    Ok(response) if response.success => {
                        this.apply_subagent_message_page(response.data.as_ref(), &subagent_id);
                    }
                    Ok(response) => {
                        this.subagent_drawer_status = response
                            .error
                            .unwrap_or_else(|| "get_subagent_messages failed".to_owned());
                    }
                    Err(error) => {
                        this.subagent_drawer_status = format!("get_subagent_messages: {error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn apply_subagent_message_page(
        &mut self,
        data: Option<&serde_json::Value>,
        subagent_id: &str,
    ) {
        let Some(data) = data else {
            "No message payload returned".clone_into(&mut self.subagent_drawer_status);
            return;
        };
        if data.get("reset").and_then(serde_json::Value::as_bool) == Some(true) {
            self.subagent_tail_lines.clear();
        }
        self.subagent_tail_next_byte = data.get("nextByte").and_then(serde_json::Value::as_u64);
        // Prefer wire `messages` (AgentMessage[]). Only fall back to `entries`
        // when `messages` is absent — an empty messages array is authoritative.
        let digests: Vec<String> = if let Some(messages) =
            data.get("messages").and_then(serde_json::Value::as_array)
        {
            messages.iter().map(subagent_message_digest).collect()
        } else if let Some(entries) = data.get("entries").and_then(serde_json::Value::as_array) {
            entries.iter().map(subagent_message_digest).collect()
        } else {
            Vec::new()
        };
        if !digests.is_empty() {
            self.subagent_tail_lines.extend(digests);
            let excess = self.subagent_tail_lines.len().saturating_sub(80);
            if excess > 0 {
                self.subagent_tail_lines.drain(..excess);
            }
        }
        self.subagent_drawer_status = format!(
            "{} · {} line(s){}",
            subagent_id,
            self.subagent_tail_lines.len(),
            if data.get("reset").and_then(serde_json::Value::as_bool) == Some(true) {
                " (reset)"
            } else {
                ""
            }
        );
    }

    pub(crate) fn export_html(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let cwd = self
            .session_cwd
            .clone()
            .unwrap_or_else(|| self.launcher_cwd.clone());
        let stamp = current_unix_seconds();
        let output_path = cwd.join(format!("pimiento-export-{stamp}.html"));
        let output_path_str = output_path.display().to_string();
        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::ExportHtml {
                    output_path: Some(output_path_str.clone()),
                })
                .await
            {
                Ok(resp) if resp.success => {
                    let path = resp
                        .data
                        .as_ref()
                        .and_then(|d| d.get("outputPath").or_else(|| d.get("path")))
                        .and_then(|v| v.as_str())
                        .unwrap_or(&output_path_str)
                        .to_owned();
                    let _ = view.update(cx, |this, cx| {
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(format!("exported HTML → {path}")));
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .clone()
                        .unwrap_or_else(|| "export_html failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("export_html".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("export_html: {error}"),
                            code: Some("export_html".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn toggle_thinking_picker(&mut self, cx: &mut Context<Self>) {
        let current_model = find_model_choice(
            &self.available_models,
            self.projection.state.model.as_deref(),
        );
        if thinking_options_for_model(current_model).is_empty() {
            self.thinking_picker_open = false;
            cx.notify();
            return;
        }
        self.thinking_picker_open = !self.thinking_picker_open;
        if self.thinking_picker_open {
            self.model_picker_open = false;
            self.clear_model_search = true;
        }
        cx.notify();
    }

    pub(crate) fn pick_model_from_search(&mut self, cx: &mut Context<Self>) {
        let query = self.model_search.read(cx).value().to_string();
        let filtered = filter_models(&self.available_models, &query);
        let choice = filtered
            .first()
            .map(|choice| (choice.provider.clone(), choice.id.clone()))
            .or_else(|| split_model_label(query.trim()));
        let Some((provider, id)) = choice else {
            return;
        };
        self.model_picker_open = false;
        self.clear_model_search = true;
        self.set_model(provider, id, cx);
    }

    pub(crate) fn refresh_state(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn sync_status_model(&mut self) {
        // Older connection paths included runtime facts after the version.
        // Keep only the connection/version fact; model metadata renders separately.
        if self.status_message.starts_with("omp/")
            && let Some((version, _)) = self.status_message.split_once("  •  ")
        {
            self.status_message = version.to_owned();
        }
    }

    pub(crate) fn set_model(&mut self, provider: String, model_id: String, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let label = format!("{provider}/{model_id}");
        // Optimistic display; corrected from response / get_state refresh.
        self.projection.state.model = Some(label.clone());
        self.close_thinking_picker();
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

    pub(crate) fn set_thinking_level(&mut self, level: &str, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.close_thinking_picker();
        let level = level.to_owned();
        // Optimistic display; corrected by the post-command get_state refresh.
        self.projection.state.thinking = Some(serde_json::json!(level));
        self.sync_status_model();
        cx.notify();

        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::SetThinkingLevel {
                    level: serde_json::json!(level),
                })
                .await
            {
                Ok(resp) if resp.success => {
                    let _ = view.update(cx, SessionView::refresh_state);
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .clone()
                        .unwrap_or_else(|| "set_thinking_level failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("set_thinking_level".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("set_thinking_level: {error}"),
                            code: Some("set_thinking_level".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn toggle_fast_mode(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let enabled = !self.projection.state.fast_mode_enabled.unwrap_or(false);
        // Optimistic display; the command response and get_state are authoritative.
        self.projection.state.fast_mode_enabled = Some(enabled);
        cx.notify();

        cx.spawn(async move |view, cx| {
            match client.send(RpcCommandBody::SetFastMode { enabled }).await {
                Ok(resp) if resp.success => {
                    let response_state = resp.data;
                    let _ = view.update(cx, |this, cx| {
                        if let Some(data) = response_state.as_ref() {
                            if let Some(value) =
                                data.get("enabled").and_then(serde_json::Value::as_bool)
                            {
                                this.projection.state.fast_mode_enabled = Some(value);
                            }
                            if let Some(value) =
                                data.get("active").and_then(serde_json::Value::as_bool)
                            {
                                this.projection.state.fast_mode_active = Some(value);
                            }
                        }
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .clone()
                        .unwrap_or_else(|| "set_fast_mode failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("set_fast_mode".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("set_fast_mode: {error}"),
                            code: Some("set_fast_mode".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn on_model_search_event(
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

    pub(crate) fn on_composer_event(
        &mut self,
        _input: gpui::Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if self.slash_menu == SlashMenuState::Dismissed {
                    self.slash_menu = SlashMenuState::Closed;
                }
                self.update_slash_menu(cx);
                cx.notify();
            }
            InputEvent::PressEnter {
                secondary,
                shift: false,
            } => {
                let text = self.composer.read(cx).value().to_string();
                let matches = self.filtered_slash_commands(&text);
                if composer_enter_action(
                    self.slash_menu == SlashMenuState::Open,
                    matches.len(),
                    *secondary,
                ) == ComposerEnterAction::AcceptCompletion
                {
                    if let Some(command) = matches.get(self.slash_selected) {
                        let command = command.clone();
                        self.accept_slash_command(&command, cx);
                    }
                    return;
                }
                self.send_composer_message(cx);
            }
            _ => {}
        }
    }

    pub(crate) fn send_composer_message(&mut self, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };

        let steer = composer_uses_steer(&self.projection.run_phase);
        self.projection.push_user_message(text.clone());
        self.close_slash_menu();
        self.clear_composer = true;
        self.refocus_composer = true;
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

    pub(crate) fn filtered_slash_commands(&self, text: &str) -> Vec<SlashCommand> {
        let commands = parse_slash_commands(self.projection.available_commands_raw.as_ref());
        filter_slash_commands(&commands, text.trim_start())
    }

    pub(crate) fn update_slash_menu(&mut self, cx: &Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        if self.slash_menu == SlashMenuState::Dismissed || !slash_draft_is_open(&text) {
            self.close_slash_menu();
            return;
        }

        self.slash_menu = SlashMenuState::Open;
        let match_count = self.filtered_slash_commands(&text).len();
        self.slash_selected = self.slash_selected.min(match_count.saturating_sub(1));
    }

    pub(crate) fn close_slash_menu(&mut self) {
        self.slash_menu = SlashMenuState::Closed;
        self.slash_selected = 0;
    }

    pub(crate) fn accept_slash_command(&mut self, command: &SlashCommand, cx: &mut Context<Self>) {
        self.pending_composer_value = Some(slash_completion_text(command));
        self.refocus_composer = true;
        self.close_slash_menu();
        cx.notify();
    }

    pub(crate) fn handle_slash_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.keystroke.modifiers.modified() || self.slash_menu != SlashMenuState::Open {
            return false;
        }

        let text = self.composer.read(cx).value().to_string();
        if !slash_draft_is_open(&text) {
            self.close_slash_menu();
            return false;
        }

        let matches = self.filtered_slash_commands(&text);
        self.slash_selected = self.slash_selected.min(matches.len().saturating_sub(1));
        let key = slash_key_action(&event.keystroke.key).or_else(|| {
            event
                .keystroke
                .key_char
                .as_deref()
                .and_then(slash_key_action)
        });
        match key {
            Some(SlashKeyAction::Dismiss) => {
                self.slash_menu = SlashMenuState::Dismissed;
                self.slash_selected = 0;
                cx.notify();
                true
            }
            Some(SlashKeyAction::Up) if !matches.is_empty() => {
                self.slash_selected = self
                    .slash_selected
                    .checked_sub(1)
                    .unwrap_or(matches.len() - 1);
                cx.notify();
                true
            }
            Some(SlashKeyAction::Down) if !matches.is_empty() => {
                self.slash_selected = (self.slash_selected + 1) % matches.len();
                cx.notify();
                true
            }
            Some(SlashKeyAction::Accept)
                if composer_enter_action(
                    self.slash_menu == SlashMenuState::Open,
                    matches.len(),
                    false,
                ) == ComposerEnterAction::AcceptCompletion =>
            {
                if let Some(command) = matches.get(self.slash_selected) {
                    let command = command.clone();
                    self.accept_slash_command(&command, cx);
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn can_send(&self) -> bool {
        self.client.is_some() && phase_allows_send(&self.projection.run_phase)
    }

    pub(crate) fn can_restart(&self) -> bool {
        matches!(self.projection.run_phase, RunPhase::Dead)
    }

    pub(crate) fn restart_resume_path(&self) -> Option<PathBuf> {
        self.projection
            .state
            .session_file
            .as_deref()
            .map(str::trim)
            .filter(|session| !session.is_empty())
            .map(PathBuf::from)
            .or_else(|| self.persistence.load_last_session())
            .or_else(|| latest_resume_path(&self.persistence, &self.recent_sessions))
    }

    pub(crate) fn restart_cwd(&self, resume: Option<&Path>) -> PathBuf {
        self.session_cwd
            .clone()
            .or_else(|| {
                resume.and_then(|session| {
                    self.recent_sessions
                        .iter()
                        .find(|recent| recent.session_file == session)
                        .map(|recent| recent.cwd.clone())
                })
            })
            .or_else(|| Some(self.launcher_cwd.clone()))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub(crate) fn do_restart(&mut self, window: &Window, cx: &mut Context<Self>) {
        let resume = self.restart_resume_path();
        let cwd = self.restart_cwd(resume.as_deref());
        self.begin_connection(window, cwd, resume, false, cx);
    }

    pub(crate) fn can_abort(&self) -> bool {
        self.client.is_some() && phase_allows_abort(&self.projection.run_phase)
    }

    pub(crate) fn clear_abort_arm(&mut self) {
        let Some(arm) = self.abort_arm.take() else {
            return;
        };
        if self.status_message == ABORT_ARM_STATUS {
            self.status_message = arm.previous_status;
        }
    }

    pub(crate) fn arm_abort(&mut self, cx: &mut Context<Self>) {
        self.clear_abort_arm();
        self.abort_arm_generation = self.abort_arm_generation.wrapping_add(1);
        let generation = self.abort_arm_generation;
        self.abort_arm = Some(AbortArm {
            generation,
            deadline: Instant::now() + ABORT_ARM_WINDOW,
            previous_status: std::mem::replace(
                &mut self.status_message,
                ABORT_ARM_STATUS.to_owned(),
            ),
        });
        cx.notify();

        cx.spawn(async move |view, cx| {
            smol::Timer::after(ABORT_ARM_WINDOW).await;
            let _ = view.update(cx, |this, cx| {
                if this.abort_arm.as_ref().map(|arm| arm.generation) == Some(generation) {
                    this.clear_abort_arm();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn handle_abort_esc_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.keystroke.modifiers.modified()
            || !matches!(event.keystroke.key.as_str(), "escape" | "esc")
            || !self.can_abort()
        {
            return false;
        }

        if self
            .abort_arm
            .as_ref()
            .is_some_and(|arm| Instant::now() < arm.deadline)
        {
            self.do_abort(cx);
        } else {
            self.arm_abort(cx);
        }
        true
    }

    pub(crate) fn do_abort(&mut self, cx: &mut Context<Self>) {
        self.clear_abort_arm();
        cx.notify();
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn(async move |_, _| {
            let _ = client.send(RpcCommandBody::Abort).await;
        })
        .detach();
    }

    pub(crate) fn toggle_tool_expanded(&mut self, tool_call_id: &str, cx: &mut Context<Self>) {
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

    pub(crate) fn client_and_dialog_id(&self, dialog_id: &str) -> Option<(RpcClient, bool)> {
        let client = self.client.clone()?;
        let exists = self
            .projection
            .pending_dialogs
            .iter()
            .any(|d| d.id == dialog_id);
        Some((client, exists))
    }

    pub(crate) fn handle_dialog_key(&self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
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

    pub(crate) fn can_follow_up(&self, cx: &Context<Self>) -> bool {
        self.client.is_some()
            && matches!(self.projection.run_phase, RunPhase::Streaming)
            && !self.composer.read(cx).value().trim().is_empty()
    }

    pub(crate) fn do_follow_up(&mut self, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };

        self.projection.push_user_message(text.clone());
        self.clear_composer = true;
        self.refocus_composer = true;
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

    #[allow(clippy::too_many_lines)] // Launcher layout remains easier to audit as one declarative block.
    pub(crate) fn render_launcher(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.theme().clone();
        let cwd = self.launcher_cwd.display().to_string();
        let recents = self.recent_sessions.clone();
        let last_resume = self.last_session.clone().filter(|resume| {
            !self
                .recent_sessions
                .iter()
                .any(|recent| recent.session_file == *resume)
        });
        let connecting = self.launcher_phase == LauncherPhase::Connecting;

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .px_6()
            .py_6()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(520.))
                    .px_6()
                    .py_6()
                    .gap_4()
                    .rounded_md()
                    .bg(theme.muted)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Pimiento")
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::SEMIBOLD),
                            )
                            .child(
                                Label::new("Drive your existing omp from a focused native workspace.")
                                    .text_sm()
                                    .text_color(theme.muted_foreground),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Label::new("Working directory").text_sm())
                            .child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .rounded_sm()
                                    .bg(theme.background)
                                    .child(cwd),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("start-working-directory")
                                    .label("Start")
                                    .primary()
                                    .disabled(connecting)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        let cwd = this.launcher_cwd.clone();
                                        this.begin_connection(window, cwd, None, true, cx);
                                    })),
                            )
                            .child(
                                Button::new("choose-working-directory")
                                    .label("Choose directory…")
                                    .ghost()
                                    .disabled(connecting)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.choose_directory(window, cx);
                                    })),
                            ),
                    )
                    .when(connecting, |parent| {
                        parent.child(
                            Label::new("Connecting to OMP…")
                                .text_xs()
                                .text_color(theme.muted_foreground),
                        )
                    })
                    .when_some(self.launcher_error.clone(), |parent, error| {
                        parent.child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .bg(theme.danger)
                                .text_xs()
                                .child(error)
                                .child(
                                    Label::new(
                                        "If omp is missing, install manually: curl -fsSL https://omp.sh/install | sh",
                                    )
                                    .text_xs(),
                                )
                                .child(
                                    Button::new("redetect-omp")
                                        .label("Re-detect omp")
                                        .small()
                                        .ghost()
                                        .disabled(connecting)
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                let cwd = this.launcher_cwd.clone();
                                                this.begin_connection(window, cwd, None, true, cx);
                                            },
                                        )),
                                ),
                        )
                    })
                    .when_some(last_resume, |parent, resume| {
                        let cwd = self.launcher_cwd.clone();
                        parent.child(
                            Button::new("resume-last-session")
                                .label("Resume last session")
                                .ghost()
                                .disabled(connecting)
                                .on_click(cx.listener(
                                    move |this, _: &ClickEvent, window, cx| {
                                        this.begin_connection(
                                            window,
                                            cwd.clone(),
                                            Some(resume.clone()),
                                            true,
                                            cx,
                                        );
                                    },
                                )),
                        )
                    })
                    .when(!recents.is_empty(), |parent| {
                        parent.child(
                            v_flex()
                                .gap_2()
                                .child(Separator::horizontal().label("Recent"))
                                .children(recents.into_iter().enumerate().map(|(ix, recent)| {
                                    let cwd = recent.cwd.clone();
                                    let resume = recent.session_file.clone();
                                    let label = if recent.name.trim().is_empty() {
                                        recent.cwd.display().to_string()
                                    } else {
                                        format!("{}  —  {}", recent.name, recent.cwd.display())
                                    };
                                    v_flex()
                                        .w_full()
                                        .gap_1()
                                        .when(ix > 0, |row| {
                                            row.child(Separator::horizontal())
                                        })
                                        .child(
                                            Button::new(("recent-session", ix))
                                                .label(label)
                                                .ghost()
                                                .w_full()
                                                .disabled(connecting)
                                                .on_click(cx.listener(
                                                    move |this, _: &ClickEvent, window, cx| {
                                                        this.begin_connection(
                                                            window,
                                                            cwd.clone(),
                                                            Some(resume.clone()),
                                                            true,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                        )
                                })),
                        )
                    }),
            )
            .into_any_element()
    }
    pub(crate) fn toggle_thinking_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        if let Some(TranscriptEntry::Thinking { collapsed, .. }) =
            self.projection.transcript.get_mut(row_ix)
        {
            *collapsed = !*collapsed;
            self.sync_transcript_list();
            cx.notify();
        }
    }

    pub(crate) fn rename_session(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let cwd = self
            .session_cwd
            .as_deref()
            .unwrap_or(self.launcher_cwd.as_path());
        let current = projection_session_name(&self.projection, cwd);
        // Lightweight rename: append a short stamp so it's actionable without a modal.
        let name = format!(
            "{current} · {stamp}",
            stamp = current_unix_seconds() % 10_000
        );
        let name_for_state = name.clone();
        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::SetSessionName { name: name.clone() })
                .await
            {
                Ok(resp) if resp.success => {
                    let _ = view.update(cx, |this, cx| {
                        if let Some(state) = this.projection.state.state.as_mut()
                            && let Some(obj) = state.as_object_mut()
                        {
                            obj.insert("sessionName".into(), serde_json::json!(name_for_state));
                        }
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(format!("session renamed → {name}")));
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .clone()
                        .unwrap_or_else(|| "set_session_name failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("set_session_name".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("set_session_name: {error}"),
                            code: Some("set_session_name".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn take_pending_workspace_palette(&mut self) -> Option<PaletteActionId> {
        self.pending_workspace_palette.take()
    }

    pub(crate) fn toggle_palette(&mut self, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        if self.palette_open {
            self.about_open = false;
            self.palette_query.clear();
            self.palette_selected = 0;
            self.model_picker_open = false;
            self.thinking_picker_open = false;
            self.slash_menu = SlashMenuState::Closed;
        }
        cx.notify();
    }

    pub(crate) fn close_palette(&mut self, cx: &mut Context<Self>) {
        if self.palette_open {
            self.palette_open = false;
            self.palette_query.clear();
            self.palette_selected = 0;
            cx.notify();
        }
    }

    pub(crate) fn show_about(&mut self, cx: &mut Context<Self>) {
        self.palette_open = false;
        self.about_open = true;
        cx.notify();
    }

    pub(crate) fn close_about(&mut self, cx: &mut Context<Self>) {
        if self.about_open {
            self.about_open = false;
            cx.notify();
        }
    }

    pub(crate) fn request_file_revert(
        &mut self,
        path: String,
        tool_call_id: String,
        cx: &mut Context<Self>,
    ) {
        let command = revert_command_for_path(&path);
        self.pending_revert = Some(PendingRevert {
            path,
            command,
            tool_call_id,
        });
        cx.notify();
    }

    pub(crate) fn cancel_pending_revert(&mut self, cx: &mut Context<Self>) {
        self.pending_revert = None;
        cx.notify();
    }

    pub(crate) fn confirm_pending_revert(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_revert.take() else {
            return;
        };
        let Some(client) = self.client.clone() else {
            self.projection.transcript.push(TranscriptEntry::Error {
                message: "revert requires a live session".into(),
                code: Some("revert".into()),
            });
            cx.notify();
            return;
        };
        let command = pending.command.clone();
        let path = pending.path.clone();
        self.projection
            .transcript
            .push(TranscriptEntry::Notice(format!(
                "reverting {path} via `{command}`"
            )));
        cx.notify();
        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::Bash {
                    command: command.clone(),
                })
                .await
            {
                Ok(resp) if resp.success => {
                    let detail = resp
                        .data
                        .as_ref()
                        .map(std::string::ToString::to_string)
                        .filter(|s| s != "null" && !s.is_empty())
                        .unwrap_or_else(|| "ok".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection
                            .transcript
                            .push(TranscriptEntry::CommandOutput(format!(
                                "$ {command}\n{detail}"
                            )));
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .clone()
                        .unwrap_or_else(|| "bash revert failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("revert failed: {error}"),
                            code: Some("revert".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("revert: {error}"),
                            code: Some("revert".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn run_palette_action(
        &mut self,
        id: PaletteActionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_palette(cx);
        match id {
            PaletteActionId::About => self.show_about(cx),
            PaletteActionId::ToggleTheme => cycle_theme_preference(window, cx),
            PaletteActionId::ToggleModels => self.toggle_model_picker(cx),
            PaletteActionId::ToggleThinking => self.toggle_thinking_picker(cx),
            PaletteActionId::ToggleFast => self.toggle_fast_mode(cx),
            PaletteActionId::ExportHtml => self.export_html(cx),
            PaletteActionId::RenameSession => self.rename_session(cx),
            PaletteActionId::AbortRun => self.do_abort(cx),
            PaletteActionId::SessionsLauncher => self.return_to_launcher(cx),
            PaletteActionId::RevealLogs => {
                if reveal_in_file_manager(&self.persistence.root).is_err() {
                    "Could not reveal the Pimiento home folder"
                        .clone_into(&mut self.status_message);
                    cx.notify();
                }
            }
            PaletteActionId::NewSession
            | PaletteActionId::CloseSession
            | PaletteActionId::ToggleRail
            | PaletteActionId::ToggleTodos
            | PaletteActionId::ToggleAgents
            | PaletteActionId::ToggleInspector => {
                self.pending_workspace_palette = Some(id);
            }
        }
    }

    pub(crate) fn handle_palette_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.palette_open {
            return false;
        }
        let key = event.keystroke.key.as_str();
        let matches = filter_palette_entries(&self.palette_query);
        if matches.is_empty() {
            self.palette_selected = 0;
        } else {
            self.palette_selected = self.palette_selected.min(matches.len() - 1);
        }
        match key {
            "escape" | "esc" => {
                self.close_palette(cx);
                true
            }
            "up" | "arrowup" => {
                if !matches.is_empty() {
                    self.palette_selected =
                        (self.palette_selected + matches.len() - 1) % matches.len();
                    cx.notify();
                }
                true
            }
            "down" | "arrowdown" => {
                if !matches.is_empty() {
                    self.palette_selected = (self.palette_selected + 1) % matches.len();
                    cx.notify();
                }
                true
            }
            "enter" | "return" => {
                if let Some(entry) = matches.get(self.palette_selected) {
                    let id = entry.id;
                    // Workspace-only actions need the parent; emit notice and still try local.
                    self.run_palette_action(id, window, cx);
                }
                true
            }
            "backspace" => {
                self.palette_query.pop();
                self.palette_selected = 0;
                cx.notify();
                true
            }
            _ if key.len() == 1
                && !event.keystroke.modifiers.platform
                && !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.alt =>
            {
                self.palette_query.push_str(key);
                self.palette_selected = 0;
                cx.notify();
                true
            }
            _ => true, // swallow while open
        }
    }

    pub(crate) fn handle_about_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.about_open {
            return false;
        }
        if !event.keystroke.modifiers.modified()
            && matches!(
                event.keystroke.key.as_str(),
                "escape" | "esc" | "enter" | "return"
            )
        {
            self.close_about(cx);
        }
        true
    }

    pub(crate) fn rail_entry(&self, ix: usize) -> RailEntry {
        let cwd = self
            .session_cwd
            .as_deref()
            .unwrap_or(self.launcher_cwd.as_path());
        let label = projection_session_name(&self.projection, cwd);
        let phase = match self.projection.run_phase {
            RunPhase::Idle => "idle",
            RunPhase::Streaming => "stream",
            RunPhase::AwaitingResume => "await",
            RunPhase::Compacting => "compact",
            RunPhase::Retrying => "retry",
            RunPhase::Restarting => "restart",
            RunPhase::Dead => "dead",
        };
        RailEntry {
            ix,
            label,
            phase: phase.to_owned(),
            cwd: cwd.to_owned(),
            attention: self.rail_attention(),
        }
    }

    pub(crate) fn rail_attention(&self) -> RailAttention {
        classify_rail_attention(&self.projection.run_phase, self.unread_below)
    }

    pub(crate) fn window_title(&self) -> String {
        if self.launcher_phase != LauncherPhase::Hidden {
            return "Pimiento".to_owned();
        }
        let cwd = self
            .session_cwd
            .as_deref()
            .unwrap_or(self.launcher_cwd.as_path());
        workspace_window_title(
            &projection_session_name(&self.projection, cwd),
            &self.projection.run_phase,
        )
    }

    pub(crate) fn shutdown_session(&mut self, cx: &mut Context<Self>) {
        self.clear_abort_arm();
        self.client.take();
        self.pump.take();
        self.running_tool_started.clear();
        self.running_tool_timer.take();
        cx.notify();
    }
}

// ── guards ────────────────────────────────────────────────────────────────

pub(crate) fn short_model_label(full: &str) -> String {
    full.strip_prefix("cursor/").unwrap_or(full).to_owned()
}

pub(crate) fn phase_tag(phase: &str) -> Tag {
    match phase {
        "stream" | "streaming" | "await" | "awaiting" => Tag::info(),
        "compact" | "compacting" | "retry" | "retrying" | "restart" | "restarting" => {
            Tag::warning()
        }
        "dead" => Tag::danger(),
        _ => Tag::secondary(),
    }
}

pub(crate) fn format_running_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let minutes = seconds / 60;
    if minutes == 0 {
        format!("{seconds}s")
    } else {
        format!("{minutes}m {}s", seconds % 60)
    }
}

pub(crate) fn composer_uses_steer(phase: &RunPhase) -> bool {
    matches!(phase, RunPhase::Streaming)
}

pub(crate) fn phase_allows_send(phase: &RunPhase) -> bool {
    !matches!(phase, RunPhase::Dead | RunPhase::Restarting)
}

pub(crate) fn phase_allows_abort(phase: &RunPhase) -> bool {
    matches!(
        phase,
        RunPhase::Streaming | RunPhase::AwaitingResume | RunPhase::Compacting | RunPhase::Retrying
    )
}

pub(crate) fn workspace_should_block_close(phases: &[RunPhase]) -> bool {
    phases.iter().any(phase_allows_abort)
}
#[derive(Debug, Clone)]
pub(crate) struct PendingRevert {
    pub(crate) path: String,
    pub(crate) command: String,
    #[allow(dead_code)]
    pub(crate) tool_call_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogKeyAction {
    Confirm,
    Deny,
    Cancel,
    Select(usize),
}

pub(crate) fn dialog_key_action(
    key: &str,
    method: &str,
    option_count: usize,
) -> Option<DialogKeyAction> {
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

pub(crate) fn select_dialog_options(dialog: &UiDialog) -> Vec<String> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashKeyAction {
    Up,
    Down,
    Accept,
    Dismiss,
}

pub(crate) fn slash_key_action(key: &str) -> Option<SlashKeyAction> {
    match key {
        "up" | "arrowup" => Some(SlashKeyAction::Up),
        "down" | "arrowdown" => Some(SlashKeyAction::Down),
        "enter" | "return" => Some(SlashKeyAction::Accept),
        "escape" | "esc" => Some(SlashKeyAction::Dismiss),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComposerEnterAction {
    AcceptCompletion,
    Send,
}

pub(crate) fn composer_enter_action(
    menu_open: bool,
    match_count: usize,
    force_send: bool,
) -> ComposerEnterAction {
    if !force_send && menu_open && match_count > 0 {
        ComposerEnterAction::AcceptCompletion
    } else {
        ComposerEnterAction::Send
    }
}

pub(crate) fn slash_draft_is_open(text: &str) -> bool {
    let Some(command) = text.trim_start().strip_prefix('/') else {
        return false;
    };
    command
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

pub(crate) fn normalize_slash_name(name: &str) -> Option<String> {
    let name = name.trim().trim_start_matches('/');
    (!name.is_empty()).then(|| format!("/{name}"))
}

pub(crate) fn parse_slash_command(raw: &serde_json::Value) -> Option<SlashCommand> {
    if let Some(name) = raw.as_str() {
        return Some(SlashCommand {
            name: normalize_slash_name(name)?,
            description: String::new(),
            aliases: Vec::new(),
        });
    }

    let name = raw.get("name").and_then(serde_json::Value::as_str)?;
    let name = normalize_slash_name(name)?;
    let description = raw
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map_or_else(String::new, |description| description.trim().to_owned());
    let mut aliases = Vec::new();
    if let Some(raw_aliases) = raw.get("aliases") {
        let values: Vec<&str> = raw_aliases
            .as_array()
            .map(|aliases| {
                aliases
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect()
            })
            .or_else(|| raw_aliases.as_str().map(|alias| vec![alias]))
            .unwrap_or_default();
        for alias in values {
            let Some(alias) = normalize_slash_name(alias) else {
                continue;
            };
            if alias != name && !aliases.contains(&alias) {
                aliases.push(alias);
            }
        }
    }

    Some(SlashCommand {
        name,
        description,
        aliases,
    })
}

pub(crate) fn parse_slash_commands(raw: Option<&serde_json::Value>) -> Vec<SlashCommand> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let entries: &[serde_json::Value] = raw
        .as_array()
        .or_else(|| raw.get("commands").and_then(serde_json::Value::as_array))
        .map_or(&[], Vec::as_slice);
    entries.iter().filter_map(parse_slash_command).collect()
}

pub(crate) fn slash_command_matches(command: &SlashCommand, query: &str) -> bool {
    let query = query.trim_start().to_ascii_lowercase();
    command.name.to_ascii_lowercase().starts_with(&query)
        || command
            .aliases
            .iter()
            .any(|alias| alias.to_ascii_lowercase().starts_with(&query))
}

pub(crate) fn filter_slash_commands(commands: &[SlashCommand], query: &str) -> Vec<SlashCommand> {
    commands
        .iter()
        .filter(|command| slash_command_matches(command, query))
        .take(SLASH_COMMAND_VISIBLE_CAP)
        .cloned()
        .collect()
}

pub(crate) fn slash_completion_text(command: &SlashCommand) -> String {
    format!("{} ", command.name)
}

pub(crate) fn todo_open_count(phases: &[TodoPhaseView]) -> usize {
    phases
        .iter()
        .flat_map(|phase| phase.tasks.iter())
        .filter(|task| matches!(task.status.as_str(), "open" | "in_progress"))
        .count()
}

pub(crate) fn render_todo_task(task: &TodoTaskView, theme: &Theme) -> gpui::AnyElement {
    let blocker = (task.status == "blocked")
        .then(|| task.blocker.clone())
        .flatten();
    v_flex()
        .w_full()
        .gap_0()
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .child(Label::new(todo_status_glyph(&task.status)).text_xs())
                .child(Label::new(task.content.clone()).text_sm()),
        )
        .when_some(blocker, |row, blocker| {
            row.child(
                Label::new(format!("blocked: {blocker}"))
                    .text_xs()
                    .text_color(theme.muted_foreground),
            )
        })
        .into_any_element()
}
impl Render for SessionView {
    #[allow(clippy::too_many_lines)] // GPUI render fns are declaratively dense; splitting hurts readability.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(value) = self.pending_composer_value.take() {
            self.composer.update(cx, |input, cx| {
                input.set_value(value, window, cx);
            });
        } else if self.clear_composer {
            self.clear_composer = false;
            self.composer.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
        if self.refocus_composer {
            self.refocus_composer = false;
            self.composer.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        if self.clear_model_search {
            self.clear_model_search = false;
            self.model_search.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }

        if self.launcher_phase != LauncherPhase::Hidden {
            return self.render_launcher(window, cx);
        }

        let theme = cx.theme().clone();
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
        let model_button_label = short_model_label(&model_label);
        let thinking_button_label = thinking_label(self.projection.state.thinking.as_ref())
            .map_or_else(|| "think:?".to_owned(), |level| format!("think:{level}"));
        let current_model = find_model_choice(
            &self.available_models,
            self.projection.state.model.as_deref(),
        );
        let thinking_options = thinking_options_for_model(current_model);
        let show_thinking_control = !thinking_options.is_empty();
        if !show_thinking_control {
            self.thinking_picker_open = false;
        }
        let context_label = context_percent_label(self.projection.state.context.as_ref())
            .map(|context| format!("ctx:{context}"));
        let tokens_label = tokens_per_second_label(self.projection.state.tokens.as_ref())
            .map(|tokens| format!("{tokens}/s"));
        let compacting = matches!(self.projection.run_phase, RunPhase::Compacting);
        let retrying = matches!(self.projection.run_phase, RunPhase::Retrying);
        let fallback_banner = self.projection.fallback_banner.clone();
        let show_activity_banner = compacting || retrying || fallback_banner.is_some();
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
        let composer_text = self.composer.read(cx).value().to_string();
        let slash_menu_visible =
            self.slash_menu == SlashMenuState::Open && slash_draft_is_open(&composer_text);
        let slash_matches = if slash_menu_visible {
            self.filtered_slash_commands(&composer_text)
        } else {
            Vec::new()
        };
        self.slash_selected = self
            .slash_selected
            .min(slash_matches.len().saturating_sub(1));
        let slash_has_matches = !slash_matches.is_empty();
        let palette_matches = filter_palette_entries(&self.palette_query);
        if palette_matches.is_empty() {
            self.palette_selected = 0;
        } else {
            self.palette_selected = self.palette_selected.min(palette_matches.len() - 1);
        }
        let palette_selected = self.palette_selected;
        let palette_query_label = if self.palette_query.is_empty() {
            "Type to filter…".to_owned()
        } else {
            self.palette_query.clone()
        };
        let pending_revert = self.pending_revert.clone();
        let transcript_empty = self.projection.transcript.is_empty();

        div()
            .size_full()
            .relative()
            .child(
                v_flex()
                    .size_full()
                    .bg(theme.background)
                    .text_color(theme.foreground)
                    .capture_key_down(cx.listener(|this, event, window, cx| {
                        let handled = this.handle_about_key(event, cx)
                            || this.handle_palette_key(event, window, cx)
                            || this.handle_dialog_key(event, cx)
                            || this.handle_slash_key(event, window, cx)
                            || this.handle_abort_esc_key(event, cx)
                            || this.handle_transcript_nav_key(event, window, cx);
                        if handled {
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
                                    .py_2()
                                    .gap_4()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                Label::new(self.status_message.clone())
                                                    .text_xs()
                                                    .when(
                                                        self.status_message == ABORT_ARM_STATUS,
                                                        |label| label.text_color(theme.warning),
                                                    ),
                                            )
                                            .child(
                                                Label::new(phase_label)
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .justify_center()
                                            .gap_2()
                                            .when_some(context_label, |group, context| {
                                                group.child(
                                                    Label::new(context)
                                                        .text_xs()
                                                        .text_color(theme.muted_foreground),
                                                )
                                            })
                                            .when_some(tokens_label, |group, tokens| {
                                                group.child(
                                                    Label::new(tokens)
                                                        .text_xs()
                                                        .text_color(theme.muted_foreground),
                                                )
                                            }),
                                    )
                                    .child(
                                        h_flex()
                                            .justify_end()
                                            .gap_2()
                                            .child(
                                                Button::new("model-picker")
                                                    .label(model_button_label)
                                                    .small()
                                                    .ghost()
                                                    .disabled(!can_pick)
                                                    .on_click(cx.listener(
                                                        |this, _: &ClickEvent, _window, cx| {
                                                            this.toggle_model_picker(cx);
                                                        },
                                                    )),
                                            )
                                            .when(show_thinking_control, |group| {
                                                group.child(
                                                    Button::new("thinking-picker")
                                                        .label(thinking_button_label)
                                                        .small()
                                                        .ghost()
                                                        .disabled(!can_pick)
                                                        .on_click(cx.listener(
                                                            |this, _: &ClickEvent, _window, cx| {
                                                                this.toggle_thinking_picker(cx);
                                                            },
                                                        )),
                                                )
                                            })
                                            .child(
                                                div().w(px(1.)).h(px(16.)).mx_1().bg(theme.border),
                                            )
                                            .child(
                                                Button::new("todo-panel-toggle")
                                                    .label("Checklist")
                                                    .small()
                                                    .ghost()
                                                    .on_click(cx.listener(
                                                        |this, _: &ClickEvent, _window, cx| {
                                                            this.request_inspector_focus(
                                                                PaletteActionId::ToggleTodos,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Button::new("subagent-drawer-toggle")
                                                    .label("Agents")
                                                    .small()
                                                    .ghost()
                                                    .disabled(!can_pick)
                                                    .on_click(cx.listener(
                                                        |this, _: &ClickEvent, _window, cx| {
                                                            this.request_inspector_focus(
                                                                PaletteActionId::ToggleAgents,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                            .child(
                                                div().w(px(1.)).h(px(16.)).mx_1().bg(theme.border),
                                            )
                                            .child(
                                                Button::new("more-actions")
                                                    .label("More")
                                                    .small()
                                                    .ghost()
                                                    .on_click(cx.listener(
                                                        |this, _: &ClickEvent, _window, cx| {
                                                            this.toggle_palette(cx);
                                                        },
                                                    )),
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
                                                    |(ix, choice)| {
                                                        let label = format!(
                                                            "{}/{}",
                                                            choice.provider, choice.id
                                                        );
                                                        let provider = choice.provider.clone();
                                                        let id = choice.id.clone();
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
                                                                        provider.clone(),
                                                                        id.clone(),
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
                            })
                            .when(self.thinking_picker_open, |parent| {
                                parent.child(
                                    h_flex()
                                        .w_full()
                                        .px_3()
                                        .pb_2()
                                        .gap_1()
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .children(thinking_options.iter().enumerate().map(
                                            |(ix, level)| {
                                                let level = level.clone();
                                                Button::new(("thinking-choice", ix))
                                                    .label(level.clone())
                                                    .ghost()
                                                    .small()
                                                    .on_click(window.listener_for(
                                                        &view,
                                                        move |this, _, _window, cx| {
                                                            this.set_thinking_level(&level, cx);
                                                        },
                                                    ))
                                            },
                                        )),
                                )
                            }),
                    )
                    .when(show_activity_banner, |parent| {
                        parent.child(
                            div()
                                .w_full()
                                .px_3()
                                .py_1()
                                .bg(theme.warning)
                                .text_xs()
                                .child(if let Some(fallback_banner) = fallback_banner {
                                    fallback_banner
                                } else if compacting {
                                    "Compacting…".to_owned()
                                } else {
                                    "Auto-retry in progress…".to_owned()
                                }),
                        )
                    })
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
                                            |e| {
                                                render_entry(
                                                    ix,
                                                    e,
                                                    &this.expanded_tools,
                                                    &this.running_tool_started,
                                                    cx,
                                                )
                                            },
                                        )
                                    })
                                    .unwrap_or_else(|_| div().into_any_element())
                                })
                                .size_full()
                                .px_3()
                                .py_2(),
                            )
                            .when(transcript_empty, |parent| {
                                parent.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            Label::new("Send a message to start")
                                                .text_sm()
                                                .text_color(theme.muted_foreground),
                                        ),
                                )
                            })
                            .when(unread > 0, |parent| {
                                parent.child(
                                    div().absolute().bottom_3().right_3().child(
                                        Button::new("jump-transcript-tail")
                                            .label(format!("{unread} new ↓"))
                                            .small()
                                            .primary()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _window, cx| {
                                                    this.jump_to_transcript_tail(cx);
                                                },
                                            )),
                                    ),
                                )
                            })
                    })
                    .children(pending_revert.into_iter().map(|pending| {
                        let path = pending.path;
                        let command = pending.command;
                        v_flex()
                            .w_full()
                            .px_3()
                            .py_2()
                            .gap_2()
                            .bg(theme.warning)
                            .border_t_1()
                            .border_color(theme.border)
                            .child(
                                Label::new(format!("Revert file {path}?"))
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::MEDIUM),
                            )
                            .child(
                                Label::new(format!("Runs: {command}"))
                                    .text_xs()
                                    .font_family(theme.mono_font_family.clone()),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("confirm-revert")
                                            .label("Confirm revert")
                                            .small()
                                            .danger()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _w, cx| {
                                                    this.confirm_pending_revert(cx);
                                                },
                                            )),
                                    )
                                    .child(
                                        Button::new("cancel-revert")
                                            .label("Cancel")
                                            .small()
                                            .ghost()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _w, cx| {
                                                    this.cancel_pending_revert(cx);
                                                },
                                            )),
                                    ),
                            )
                    }))
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
                                div()
                                    .relative()
                                    .flex_1()
                                    .child(
                                        Input::new(&self.composer)
                                            .appearance(false)
                                            .focus_bordered(false),
                                    )
                                    .when(slash_menu_visible, |parent| {
                                        parent.child(
                                            v_flex()
                                                .absolute()
                                                .bottom_full()
                                                .left_0()
                                                .right_0()
                                                .max_h(px(280.))
                                                .overflow_y_scrollbar()
                                                .gap_0()
                                                .p_1()
                                                .bg(theme.background)
                                                .border_1()
                                                .border_color(theme.border)
                                                .children(slash_matches.iter().enumerate().map(
                                                    |(ix, command)| {
                                                        let command_for_click = command.clone();
                                                        Button::new(("slash-command", ix))
                                                            .ghost()
                                                            .small()
                                                            .w_full()
                                                            .when(
                                                                ix == self.slash_selected,
                                                                |button| button.bg(theme.secondary),
                                                            )
                                                            .on_click(window.listener_for(
                                                                &view,
                                                                move |this, _, _window, cx| {
                                                                    this.accept_slash_command(
                                                                        &command_for_click,
                                                                        cx,
                                                                    );
                                                                },
                                                            ))
                                                            .child(
                                                                h_flex()
                                                                    .w_full()
                                                                    .gap_2()
                                                                    .child(
                                                                        Label::new(
                                                                            command.name.clone(),
                                                                        )
                                                                        .text_sm(),
                                                                    )
                                                                    .when(
                                                                        !command
                                                                            .description
                                                                            .is_empty(),
                                                                        |row| {
                                                                            row.child(
                                                                        Label::new(
                                                                            command
                                                                                .description
                                                                                .clone(),
                                                                        )
                                                                        .text_xs()
                                                                        .text_color(
                                                                            theme.muted_foreground,
                                                                        ),
                                                                    )
                                                                        },
                                                                    ),
                                                            )
                                                    },
                                                ))
                                                .when(!slash_has_matches, |panel| {
                                                    panel.child(
                                                        Label::new("(no matches)")
                                                            .text_xs()
                                                            .text_color(theme.muted_foreground),
                                                    )
                                                }),
                                        )
                                    }),
                            )
                            .child(
                                Label::new(if cfg!(target_os = "macos") {
                                    "⌘↩"
                                } else {
                                    "Ctrl+Enter"
                                })
                                .text_xs()
                                .text_color(theme.muted_foreground),
                            )
                            .child(
                                Button::new("send")
                                    .primary()
                                    .label("Send")
                                    .disabled(!self.can_send())
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.send_composer_message(cx);
                                    })),
                            )
                            .when(
                                matches!(self.projection.run_phase, RunPhase::Streaming),
                                |parent| {
                                    parent.child(
                                        Button::new("follow-up")
                                            .label("Follow-up")
                                            .disabled(!self.can_follow_up(cx))
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _window, cx| {
                                                    this.do_follow_up(cx);
                                                },
                                            )),
                                    )
                                },
                            )
                            .when(self.can_abort(), |parent| {
                                parent.child(Button::new("abort").danger().label("Abort").on_click(
                                    cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.do_abort(cx);
                                    }),
                                ))
                            })
                            .when(self.can_restart(), |parent| {
                                parent.child(
                                    Button::new("restart").primary().label("Restart").on_click(
                                        cx.listener(|this, _: &ClickEvent, window, cx| {
                                            this.do_restart(window, cx);
                                        }),
                                    ),
                                )
                            }),
                    ),
            )
            .when(self.palette_open, |parent| {
                parent.child(
                    div()
                        .id("command-palette-backdrop")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_start()
                        .justify_center()
                        .pt_16()
                        .bg(gpui::rgba(0x0000_0080))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.close_palette(cx);
                        }))
                        .child(
                            v_flex()
                                .id("command-palette-panel")
                                .w(px(480.))
                                .max_h(px(420.))
                                .gap_1()
                                .p_3()
                                .rounded_md()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.background)
                                .on_click(cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    Label::new("Command palette (Esc closes)")
                                        .text_xs()
                                        .text_color(theme.muted_foreground),
                                )
                                .child(
                                    Label::new(format!("> {palette_query_label}"))
                                        .text_sm()
                                        .font_family(theme.mono_font_family.clone()),
                                )
                                .children(palette_matches.iter().enumerate().map(|(ix, entry)| {
                                    let id = entry.id;
                                    let selected = ix == palette_selected;
                                    Button::new(("palette-entry", ix))
                                        .label(format!("{} · {}", entry.label, entry.hint))
                                        .small()
                                        .w_full()
                                        .when(selected, Button::primary)
                                        .when(!selected, Button::ghost)
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, window, cx| {
                                                this.run_palette_action(id, window, cx);
                                            },
                                        ))
                                }))
                                .when(palette_matches.is_empty(), |panel| {
                                    panel.child(
                                        Label::new("(no matches)")
                                            .text_xs()
                                            .text_color(theme.muted_foreground),
                                    )
                                }),
                        ),
                )
            })
            .when(self.about_open, |parent| {
                let version = self.omp_version.as_deref().map_or_else(
                    || "OMP version unavailable".to_owned(),
                    |version| format!("OMP version: {version}"),
                );
                parent.child(
                    div()
                        .id("about-backdrop")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::rgba(0x0000_0080))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.close_about(cx);
                        }))
                        .child(
                            v_flex()
                                .id("about-panel")
                                .w(px(360.))
                                .gap_3()
                                .p_5()
                                .rounded_md()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.background)
                                .on_click(cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    Label::new("Pimiento")
                                        .text_lg()
                                        .font_weight(gpui::FontWeight::SEMIBOLD),
                                )
                                .child(
                                    Label::new("A native client for OMP rpc-ui.")
                                        .text_sm()
                                        .text_color(theme.muted_foreground),
                                )
                                .child(Label::new(version).text_sm())
                                .child(
                                    Label::new("Local-only · no telemetry")
                                        .text_sm()
                                        .text_color(theme.muted_foreground),
                                )
                                .child(h_flex().w_full().justify_end().child(
                                    Button::new("about-ok").label("OK").primary().on_click(
                                        cx.listener(|this, _: &ClickEvent, _w, cx| {
                                            this.close_about(cx);
                                        }),
                                    ),
                                )),
                        ),
                )
            })
            .into_any_element()
    }
}

// ── transcript rows ───────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)] // GPUI render fns are declaratively dense; splitting hurts readability.
pub(crate) fn subagent_payload_summary(payload: &serde_json::Value) -> String {
    let kind = payload
        .get("type")
        .or_else(|| payload.get("event"))
        .and_then(|v| v.as_str())
        .unwrap_or("subagent");
    let name = payload
        .get("name")
        .or_else(|| payload.get("agent"))
        .or_else(|| payload.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let detail = payload
        .get("description")
        .or_else(|| payload.get("status"))
        .or_else(|| payload.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(72)
        .collect::<String>();
    if detail.is_empty() {
        format!("{kind} {name}")
    } else {
        format!("{kind} {name}: {detail}")
    }
}

pub(crate) fn subagent_snapshot_id(snapshot: &serde_json::Value) -> Option<&str> {
    snapshot.get("id").and_then(serde_json::Value::as_str)
}

pub(crate) fn subagent_snapshot_session_file(snapshot: &serde_json::Value) -> Option<&str> {
    snapshot
        .get("sessionFile")
        .and_then(serde_json::Value::as_str)
}

pub(crate) fn subagent_snapshot_summary(snapshot: &serde_json::Value) -> String {
    let id = subagent_snapshot_id(snapshot).unwrap_or("unknown");
    let agent = snapshot
        .get("agent")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("agent");
    let status = snapshot
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let description = snapshot
        .get("description")
        .or_else(|| snapshot.get("task"))
        .or_else(|| snapshot.get("assignment"))
        .map(compact_subagent_value)
        .unwrap_or_default();
    if description.is_empty() {
        format!("{agent} · {status} · {id}")
    } else {
        format!("{agent} · {status} · {id}: {description}")
    }
}

pub(crate) fn subagent_message_digest(message: &serde_json::Value) -> String {
    let role = message
        .get("role")
        .or_else(|| message.get("message").and_then(|value| value.get("role")))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("message");
    let content = message
        .get("content")
        .or_else(|| {
            message
                .get("message")
                .and_then(|value| value.get("content"))
        })
        .map_or_else(|| compact_subagent_value(message), compact_subagent_value);
    format!("{role}: {content}")
}

pub(crate) fn compact_subagent_value(value: &serde_json::Value) -> String {
    let text = match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .find_map(subagent_text_part)
            .unwrap_or_else(|| compact_json(value)),
        serde_json::Value::Object(object) => object
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| compact_json(value), str::to_owned),
        _ => compact_json(value),
    };
    truncate_subagent_text(&text, 120)
}

pub(crate) fn subagent_text_part(part: &serde_json::Value) -> Option<String> {
    part.as_str()
        .map(str::to_owned)
        .or_else(|| {
            part.get("text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            part.get("content")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
}

pub(crate) fn compact_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unrenderable JSON>".to_owned())
}

pub(crate) fn truncate_subagent_text(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let prefix: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[allow(clippy::too_many_lines)] // Match arms mirror transcript variants.
pub(crate) fn classify_messages_page_error(
    code: Option<&str>,
    message: Option<&str>,
) -> MessagesPageErrorKind {
    let blob = format!("{} {}", code.unwrap_or(""), message.unwrap_or("")).to_ascii_lowercase();
    if code == Some("session_busy")
        || blob.contains("session_busy")
        || blob.contains("session is changing")
    {
        MessagesPageErrorKind::Busy
    } else if code == Some("stale_cursor")
        || blob.contains("stale_cursor")
        || blob.contains("cursor is stale")
    {
        MessagesPageErrorKind::Stale
    } else {
        MessagesPageErrorKind::Other
    }
}

pub(crate) fn history_row_count(proj: &SessionProjection) -> usize {
    proj.transcript
        .iter()
        .filter(|entry| {
            matches!(
                entry,
                TranscriptEntry::User { .. }
                    | TranscriptEntry::AssistantText { .. }
                    | TranscriptEntry::Thinking { .. }
                    | TranscriptEntry::ToolCall { .. }
                    | TranscriptEntry::CommandOutput(_)
            )
        })
        .count()
}

pub(crate) fn hydrate_history_pages(client: &RpcClient, proj: &mut SessionProjection) {
    let mut cursor: Option<String> = None;
    let mut pages = 0usize;
    let mut stale_restarts = 0usize;

    loop {
        if pages >= MESSAGE_PAGE_MAX_PAGES {
            proj.transcript.push(TranscriptEntry::Notice(format!(
                "Stopped history hydration after {MESSAGE_PAGE_MAX_PAGES} pages"
            )));
            break;
        }

        let mut attempt = 0usize;
        let page_result = loop {
            attempt += 1;
            let response = smol::block_on(async {
                client
                    .send(RpcCommandBody::GetMessagesPage {
                        cursor: cursor.clone(),
                        limit: Some(MESSAGE_PAGE_LIMIT),
                    })
                    .await
            });
            match response {
                Ok(resp) if resp.success => break Ok(resp),
                Ok(resp) => {
                    let kind =
                        classify_messages_page_error(resp.code.as_deref(), resp.error.as_deref());
                    match kind {
                        MessagesPageErrorKind::Busy if attempt < MESSAGE_PAGE_BUSY_RETRIES => {
                            std::thread::sleep(Duration::from_millis(150));
                        }
                        MessagesPageErrorKind::Stale if stale_restarts == 0 => {
                            proj.clear_hydrated_history();
                            cursor = None;
                            pages = 0;
                            stale_restarts += 1;
                            break Err("stale_restart".to_owned());
                        }
                        _ => {
                            break Err(resp
                                .error
                                .clone()
                                .unwrap_or_else(|| "get_messages_page failed".to_owned()));
                        }
                    }
                }
                Err(error) => break Err(error.to_string()),
            }
        };

        match page_result {
            Err(msg) if msg == "stale_restart" => {}
            Err(_) => {
                if history_row_count(proj) == 0
                    && let Ok(resp) =
                        smol::block_on(async { client.send(RpcCommandBody::GetMessages).await })
                    && resp.success
                    && let Some(data) = resp.data.as_ref()
                {
                    proj.hydrate_messages(data);
                }
                break;
            }
            Ok(resp) => {
                let Some(data) = resp.data.as_ref() else {
                    break;
                };
                proj.hydrate_messages(data);
                pages += 1;
                match data
                    .get("nextCursor")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .filter(|s| !s.is_empty())
                {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
        }
    }
}

pub(crate) fn try_connect_omp(
    cwd: Option<PathBuf>,
    resume: Option<&Path>,
    persistence: &SessionPersistence,
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

    let cwd = cwd
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::current_dir().ok());
    let resume = resume.map(Path::to_owned);
    let mut cfg = ClientConfig {
        program: discovered.path,
        env: discovered.env,
        cwd: cwd.clone(),
        no_session: false,
        resume: resume.clone(),
        ..Default::default()
    };

    let client = match smol::block_on(async { RpcClient::connect(cfg.clone()).await }) {
        Ok(c) => c,
        Err(e) if resume.is_some() => {
            if let Some(resume) = &resume {
                persistence.forget_session(resume);
            }
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
        persistence.remember_last_session(proj.state.session_file.as_deref());
        if let (Some(session_file), Some(cwd)) =
            (proj.state.session_file.as_deref(), cwd.as_deref())
        {
            let name = projection_session_name(&proj, cwd);
            persistence.remember_recent_session(Some(session_file), Some(cwd), Some(&name));
        }
    }
    hydrate_history_pages(&client, &mut proj);
    if let Ok(r) = &avail
        && r.success
        && let Some(data) = &r.data
    {
        proj.hydrate_available_commands(data);
    }

    // Full model catalog is loaded asynchronously after the window opens —
    // get_available_models can be large enough to exceed the default RPC timeout.
    let models = Vec::new();

    let status = discovered.version_text.trim().to_owned();

    Ok((client, proj, status, models))
}
