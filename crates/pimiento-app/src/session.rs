use crate::*;

pub(crate) const MODEL_PICKER_VISIBLE_CAP: usize = 200;
pub(crate) const SLASH_COMMAND_VISIBLE_CAP: usize = 12;
pub(crate) const MAX_RECENT_SESSIONS: usize = 12;
/// How many recents the launcher card shows (persistence still keeps `MAX_RECENT_SESSIONS`).
pub(crate) const LAUNCHER_RECENT_VISIBLE_CAP: usize = 5;
pub(crate) const MAX_DISCOVERED_SESSIONS: usize = 24;
pub(crate) const SESSION_HEADER_PREFIX_BYTES: usize = 8192;
pub(crate) const ABORT_ARM_WINDOW: Duration = Duration::from_millis(1200);
pub(crate) const ABORT_ARM_STATUS: &str = "Press Esc again to abort";
/// `abort_bash` is synchronous on OMP; a short deadline only covers a wedged queue.
pub(crate) const ABORT_BASH_TIMEOUT: Duration = Duration::from_secs(1);
/// Cursor-hosted bash ignores `abort_bash` (execute is called with no signal).
/// If turn `abort` has not received an ACK by this deadline, the rpc-ui serial queue is
/// wedged on `waitForIdle` and the child must be recycled.
pub(crate) const ABORT_RPC_TIMEOUT: Duration = Duration::from_secs(4);
pub(crate) const MIN_WINDOW_WIDTH: f32 = 480.0;
pub(crate) const MIN_WINDOW_HEIGHT: f32 = 320.0;
pub(crate) static PERSISTENCE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
pub(crate) type ConnectionResult = Result<ConnectedSession, String>;

pub(crate) struct ConnectedSession {
    pub(crate) client: RpcClient,
    pub(crate) projection: SessionProjection,
    pub(crate) status: String,
    pub(crate) models: Vec<ModelChoice>,
    pub(crate) version_gate_notice: Option<String>,
    pub(crate) omp_bin: PathBuf,
    pub(crate) omp_env: BTreeMap<OsString, OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OmpUpdateUi {
    Idle,
    Checking,
    Available { current: String, latest: String },
    Updating { current: String, latest: String },
    Failed { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlashCommand {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) input_hint: Option<String>,
    pub(crate) subcommands: Vec<SlashSubcommand>,
    pub(crate) source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlashSubcommand {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) usage: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlashSuggestion {
    pub(crate) completion_text: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) usage_hint: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) expects_input: bool,
    pub(crate) is_subcommand: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum CommandPaletteEntry {
    Action(&'static PaletteEntry),
    NativeSlash(&'static NativeSlashEntry),
    Slash {
        suggestion: SlashSuggestion,
        aliases: Vec<String>,
    },
}

impl CommandPaletteEntry {
    pub(crate) fn title(&self) -> &str {
        match self {
            Self::Action(entry) => entry.label,
            Self::NativeSlash(entry) => entry.name,
            Self::Slash { suggestion, .. } => &suggestion.title,
        }
    }

    pub(crate) fn description(&self) -> &str {
        match self {
            Self::Action(entry) => entry.hint,
            Self::NativeSlash(entry) => entry.description,
            Self::Slash { suggestion, .. } => &suggestion.description,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandPaletteRowData {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) metadata: String,
    pub(crate) usage: Option<String>,
}

pub(crate) fn command_palette_row_data(entry: &CommandPaletteEntry) -> CommandPaletteRowData {
    let (metadata, usage) = match entry {
        CommandPaletteEntry::Action(_) => ("Pimiento command".to_owned(), None),
        CommandPaletteEntry::NativeSlash(_) => {
            ("Pimiento action · runs immediately".to_owned(), None)
        }
        CommandPaletteEntry::Slash {
            suggestion,
            aliases,
        } => {
            let mut metadata = "OMP slash command · selection inserts; Enter sends".to_owned();
            if let Some(source) = suggestion.source.as_deref() {
                metadata.push_str(" · ");
                metadata.push_str(source);
            }
            if !aliases.is_empty() {
                metadata.push_str(" · aliases ");
                metadata.push_str(&aliases.join(", "));
            }
            (
                metadata,
                suggestion
                    .usage_hint
                    .as_ref()
                    .map(|hint| format!("Usage: {hint}")),
            )
        }
    };
    CommandPaletteRowData {
        title: entry.title().to_owned(),
        description: entry.description().to_owned(),
        metadata,
        usage,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashMenuState {
    Closed,
    Open,
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LargePasteChoice {
    Wrap,
    SaveLocal,
    Inline,
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
    /// Crash-loop / restart policy for the live OMP child. Constructed on
    /// successful connect; retained across auto-restarts until explicit
    /// shutdown or a fresh session.
    pub(crate) supervisor: Option<Supervisor>,
    pub(crate) composer: gpui::Entity<InputState>,
    /// Tiny view that owns `Input::new` so keystrokes do not invalidate `SessionView`.
    pub(crate) composer_view: gpui::Entity<ComposerInputView>,
    pub(crate) model_search: gpui::Entity<InputState>,
    pub(crate) palette_search: gpui::Entity<InputState>,
    /// Shared text field for `input` / `editor` extension UI dialogs.
    pub(crate) dialog_input: gpui::Entity<InputState>,
    /// Isolated view around [`Self::dialog_input`] so editor keystrokes do not
    /// lease `SessionView` (GPUI `double_lease_panic`).
    pub(crate) dialog_input_view: gpui::Entity<ComposerInputView>,
    /// Dialog id currently bound into [`Self::dialog_input`].
    pub(crate) dialog_input_bound_id: Option<String>,
    /// Applied in `render` (needs `Window`) when a text dialog appears/clears.
    pub(crate) pending_dialog_input_sync: Option<(String, String)>,
    /// Marked option for the current `select` dialog; submit is explicit Continue.
    pub(crate) select_draft: Option<SelectDraft>,
    /// Last ask-question choice, kept across sequential `select` request ids.
    pub(crate) select_memory: Option<SelectMemory>,
    /// Typed custom answer waiting for OMP's follow-up `editor` request.
    pub(crate) pending_custom_answer: Option<String>,
    /// True when [`Self::dialog_input`] is empty; avoids reading `InputState` in render.
    pub(crate) dialog_input_empty: bool,
    /// Per-dialog timeout generation so superseded timers no-op.
    pub(crate) dialog_timeout_gens: HashMap<String, u64>,
    pub(crate) dialog_timeout_generation: u64,
    /// Hand-rolled rename overlay on `SessionView` (same pattern as About/palette).
    pub(crate) rename_open: bool,
    pub(crate) rename_input: gpui::Entity<InputState>,
    /// Optional instructions for an explicit `compact` RPC.
    pub(crate) compact_open: bool,
    pub(crate) compact_input: gpui::Entity<InputState>,
    pub(crate) pending_compact_sync: bool,
    pub(crate) refocus_compact_input: bool,
    /// Confirmation surface before handing the authoritative session to TUI.
    pub(crate) handoff_confirm_open: bool,
    /// Branch-into-new-tab message picker (`get_branch_messages`).
    pub(crate) branch_picker: Option<Vec<BranchMessageChoice>>,
    pub(crate) branch_picker_selected: usize,
    /// Login providers floating list (`get_login_providers`).
    pub(crate) login_picker: Option<Vec<LoginProviderChoice>>,
    pub(crate) login_picker_selected: usize,
    /// Workspace opens a fresh tab for this cwd after a successful branch.
    pub(crate) pending_new_tab_cwd: Option<PathBuf>,
    pub(crate) model_picker_open: bool,
    pub(crate) thinking_picker_open: bool,
    pub(crate) pending_attachments: Vec<PendingAttachment>,
    /// Monotonic counter for OMP-style `local://paste-N.md` saves.
    pub(crate) paste_counter: u64,
    /// Large-paste menu: raw text awaiting Wrap / Save / Inline.
    pub(crate) large_paste_pending: Option<String>,
    pub(crate) at_mention_open: bool,
    pub(crate) at_mention_selected: usize,
    pub(crate) at_mention_candidates: Vec<PathBuf>,
    pub(crate) at_mention_index: Vec<PathBuf>,
    pub(crate) at_mention_index_cwd: Option<PathBuf>,
    /// Last observed composer emptiness, used to avoid session-wide paints on each keystroke.
    pub(crate) composer_empty: bool,
    /// Last observed hard line count, used to relayout chrome when the composer grows.
    pub(crate) composer_rows: usize,
    pub(crate) omp_roles: Vec<OmpRole>,
    /// Latest `get_subagents` response, retained losslessly for tolerant rendering.
    pub(crate) subagent_snapshots: Vec<serde_json::Value>,
    pub(crate) subagent_subscription: SubagentSubscriptionLevel,
    pub(crate) subagent_refresh_in_flight: bool,
    pub(crate) state_refresh_in_flight: bool,
    pub(crate) state_refresh_queued: bool,
    pub(crate) selected_subagent_id: Option<String>,
    pub(crate) subagent_modal_open: bool,
    pub(crate) subagent_modal_status: String,
    pub(crate) subagent_modal_request_generation: u64,
    pub(crate) subagent_tail_next_byte: Option<u64>,
    pub(crate) subagent_tail_lines: Vec<String>,
    pub(crate) subagent_drawer_status: String,
    pub(crate) pending_revert: Option<PendingRevert>,
    /// Experimental OMP host-tool bridge. Disabled unless explicitly opted in.
    pub(crate) host_bridge: HostBridgeState,
    pub(crate) palette_open: bool,
    pub(crate) theme_picker_open: bool,
    pub(crate) theme_search: gpui::Entity<InputState>,
    pub(crate) recent_filter: gpui::Entity<InputState>,
    pub(crate) theme_picker_selected: usize,
    pub(crate) theme_picker_opening_selection: Option<ThemeSelection>,
    pub(crate) clear_theme_search: bool,
    pub(crate) refocus_theme_search: bool,
    pub(crate) about_open: bool,
    /// Mirrors workspace inspector visibility so the session toolbar can
    /// defer Checklist/Agents/ctx chrome while the Context pane is open.
    pub(crate) inspector_open: bool,
    pub(crate) palette_selected: usize,
    pub(crate) pending_workspace_palette: Option<PaletteActionId>,
    pub(crate) slash_menu: SlashMenuState,
    pub(crate) slash_selected: usize,
    pub(crate) status_message: String,
    pub(crate) omp_version: Option<String>,
    pub(crate) version_gate_notice: Option<String>,
    pub(crate) omp_update_ui: OmpUpdateUi,
    pub(crate) omp_bin: Option<PathBuf>,
    pub(crate) omp_env: BTreeMap<OsString, OsString>,
    pub(crate) omp_update_generation: u64,
    pub(crate) pending_omp_update_failure: Option<String>,
    pub(crate) abort_arm: Option<AbortArm>,
    pub(crate) abort_arm_generation: u64,
    pub(crate) available_models: Vec<ModelChoice>,
    pub(crate) expanded_tools: HashSet<String>,
    pub(crate) running_tool_started: HashMap<String, Instant>,
    pub(crate) running_tool_timer: Option<Task<()>>,
    /// Polls `get_state` while a turn is live so inspector ctx%/tps stay current.
    pub(crate) live_state_timer: Option<Task<()>>,
    pub(crate) clear_composer: bool,
    pub(crate) pending_composer_value: Option<String>,
    pub(crate) refocus_composer: bool,
    pub(crate) refocus_palette_search: bool,
    pub(crate) clear_model_search: bool,
    pub(crate) clear_palette_search: bool,
    pub(crate) pending_palette_enter: bool,
    /// Virtualized transcript list (GPUI `ListState`, bottom-aligned chat).
    pub(crate) transcript_list: ListState,
    pub(crate) last_transcript_len: usize,
    /// Cheap tail signature so streaming paints do not `remeasure_items` unless content grew.
    pub(crate) last_transcript_fingerprint: u64,
    /// Persistent selectable Markdown/HTML views; `TextView::markdown` keyed state
    /// notifies `SessionView` on every drag and the virtualized list drops it.
    pub(crate) transcript_text_views:
        HashMap<(usize, TranscriptTextSlot), gpui::Entity<TextViewState>>,
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
    #[allow(clippy::too_many_lines)]
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
                .auto_grow(COMPOSER_GROW_MIN_ROWS, COMPOSER_GROW_MAX_ROWS)
                .submit_on_enter(true)
                .placeholder("Message · Enter to send")
        });
        let composer_view = cx.new({
            let input = composer.clone();
            move |_cx| ComposerInputView { input }
        });
        let model_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search models… (or provider/id, Enter)")
                .submit_on_enter(true)
        });
        let palette_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Type to filter commands…")
                .submit_on_enter(true)
        });
        let theme_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search themes…")
                .submit_on_enter(true)
        });
        let recent_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter recent sessions…"));
        let dialog_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(1, 6)
                .placeholder("Enter a value…")
        });
        let dialog_input_view = cx.new({
            let input = dialog_input.clone();
            move |_cx| ComposerInputView { input }
        });
        let rename_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Session name")
                .submit_on_enter(true)
        });
        let compact_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(1, 5)
                .placeholder("Optional instructions for the compacted context")
                .submit_on_enter(true)
        });

        let subscriptions = vec![
            cx.subscribe(&composer, Self::on_composer_event),
            cx.subscribe(&model_search, Self::on_model_search_event),
            cx.subscribe(&palette_search, Self::on_palette_search_event),
            cx.subscribe(&theme_search, Self::on_theme_search_event),
            cx.subscribe(&recent_filter, Self::on_recent_filter_event),
            cx.subscribe(&dialog_input, Self::on_dialog_input_event),
            cx.subscribe(&rename_input, Self::on_rename_input_event),
            cx.subscribe(&compact_input, Self::on_compact_input_event),
        ];

        let initial_len = initial_projection.transcript.len();
        let transcript_list = ListState::new(initial_len, ListAlignment::Bottom, px(400.));
        transcript_list.set_follow_mode(FollowMode::Tail);
        {
            let weak = cx.weak_entity();
            transcript_list.set_scroll_handler(move |ev, _window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if ev.is_following_tail {
                        if this.unread_below == 0 {
                            return;
                        }
                        this.unread_below = 0;
                        cx.notify();
                    }
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
            supervisor: None,
            composer,
            composer_view,
            model_search,
            palette_search,
            dialog_input,
            dialog_input_view,
            dialog_input_bound_id: None,
            pending_dialog_input_sync: None,
            select_draft: None,
            select_memory: None,
            pending_custom_answer: None,
            dialog_input_empty: true,
            dialog_timeout_gens: HashMap::new(),
            dialog_timeout_generation: 0,
            rename_open: false,
            rename_input,
            compact_open: false,
            compact_input,
            pending_compact_sync: false,
            refocus_compact_input: false,
            handoff_confirm_open: false,
            branch_picker: None,
            branch_picker_selected: 0,
            login_picker: None,
            login_picker_selected: 0,
            pending_new_tab_cwd: None,
            model_picker_open: false,
            thinking_picker_open: false,
            pending_attachments: Vec::new(),
            paste_counter: 0,
            large_paste_pending: None,
            at_mention_open: false,
            at_mention_selected: 0,
            at_mention_candidates: Vec::new(),
            at_mention_index: Vec::new(),
            at_mention_index_cwd: None,
            composer_empty: true,
            composer_rows: COMPOSER_GROW_MIN_ROWS,
            omp_roles: load_omp_roles_from_home(home_dir().as_deref()),
            subagent_snapshots: Vec::new(),
            subagent_subscription: SubagentSubscriptionLevel::Events,
            subagent_refresh_in_flight: false,
            state_refresh_in_flight: false,
            state_refresh_queued: false,
            selected_subagent_id: None,
            subagent_modal_open: false,
            subagent_modal_status: String::new(),
            subagent_modal_request_generation: 0,
            subagent_tail_next_byte: None,
            subagent_tail_lines: Vec::new(),
            subagent_drawer_status: String::new(),
            pending_revert: None,
            host_bridge: HostBridgeState::from_environment(),
            palette_open: false,
            theme_picker_open: false,
            theme_search,
            recent_filter,
            theme_picker_selected: 0,
            theme_picker_opening_selection: None,
            clear_theme_search: false,
            refocus_theme_search: false,
            about_open: false,
            inspector_open: false,
            palette_selected: 0,
            pending_workspace_palette: None,
            slash_menu: SlashMenuState::Closed,
            slash_selected: 0,
            status_message: status,
            omp_version,
            version_gate_notice: None,
            omp_update_ui: OmpUpdateUi::Idle,
            omp_bin: None,
            omp_env: BTreeMap::new(),
            omp_update_generation: 0,
            pending_omp_update_failure: None,
            abort_arm: None,
            abort_arm_generation: 0,
            available_models,
            expanded_tools: HashSet::new(),
            running_tool_started: HashMap::new(),
            running_tool_timer: None,
            live_state_timer: None,
            clear_composer: false,
            pending_composer_value: None,
            refocus_composer: false,
            refocus_palette_search: false,
            clear_model_search: false,
            clear_palette_search: false,
            pending_palette_enter: false,
            transcript_list,
            last_transcript_len: initial_len,
            last_transcript_fingerprint: 0,
            transcript_text_views: HashMap::new(),
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
            view.attach_supervisor(None);
            view.start_event_pump(window, &client, cx);
        }
        view.sync_running_tools(cx);
        view.start_catalog_load(cx);
        view
    }

    pub(crate) fn start_event_pump(
        &mut self,
        window: &Window,
        client: &RpcClient,
        cx: &mut Context<Self>,
    ) {
        let events = client.events();
        self.pump = Some(cx.spawn_in(window, async move |view, cx| {
            // PLAN §5.3: drain the whole channel per foreground wakeup, apply
            // batched deltas as one entity update, then emit one notify.
            loop {
                let Ok(first) = events.recv().await else {
                    break;
                };
                let mut batch = vec![first];
                while let Ok(event) = events.try_recv() {
                    batch.push(event);
                }
                let _ = view.update_in(cx, |this, window, cx| {
                    for event in &batch {
                        match event {
                            ClientEvent::Frame(frame) => {
                                let is_model_changed =
                                    frame.raw.get("type").and_then(|v| v.as_str())
                                        == Some("model_changed");
                                let refresh_subagents = match &frame.kind {
                                    omp_rpc_client::frames::IncomingFrameKind::SubagentLifecycle(
                                        payload,
                                    )
                                    | omp_rpc_client::frames::IncomingFrameKind::SubagentProgress(
                                        payload,
                                    )
                                    | omp_rpc_client::frames::IncomingFrameKind::SubagentEvent(
                                        payload,
                                    ) => {
                                        !this.subagent_refresh_in_flight
                                            && subagent_event_needs_snapshot_refresh(
                                                &payload.payload,
                                                &this.subagent_snapshots,
                                            )
                                    }
                                    _ => false,
                                };
                                this.observe_host_bridge_frame(frame);
                                this.projection.apply(frame);
                                this.sync_pending_dialogs(cx);
                                let refresh_runtime_state = is_model_changed
                                    || matches!(
                                        &frame.kind,
                                        omp_rpc_client::frames::IncomingFrameKind::AgentEnd(_)
                                            | omp_rpc_client::frames::IncomingFrameKind::AutoCompactionEnd
                                    );
                                if refresh_runtime_state {
                                    this.refresh_state(cx);
                                }
                                if refresh_subagents {
                                    this.refresh_subagents(cx);
                                }
                            }
                            ClientEvent::Closed(info) => {
                                this.handle_client_closed(info, window, cx);
                            }
                        }
                    }
                    this.sync_running_tools(cx);
                    this.sync_live_state_timer(cx);
                    if !this.can_abort() {
                        this.clear_abort_arm();
                    }
                    cx.notify();
                });
            }
        }));
    }

    /// Apply supervisor policy on a terminal `Closed` event and optionally
    /// schedule an auto-restart under the crash-loop breaker.
    fn handle_client_closed(
        &mut self,
        info: &omp_rpc_client::client::ClosedInfo,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_omp_update();
        self.client = None;
        self.host_bridge.reset();
        self.clear_live_timers();
        self.dialog_timeout_gens.clear();
        self.dialog_input_bound_id = None;
        self.pending_custom_answer = None;
        self.dialog_input_empty = true;

        let Some(supervisor) = self.supervisor.clone() else {
            let reason = info
                .error_msg
                .clone()
                .unwrap_or_else(|| format!("exit code {:?}", info.exit_code));
            self.projection.mark_dead(reason);
            self.status_message = omp_closed_status_message(&info.stderr_tail);
            return;
        };

        let mapped = map_closed(info);
        let state =
            supervisor.on_child_died(Instant::now(), mapped.exit_code, mapped.stderr_tail.clone());
        match state {
            SupervisorState::Restarting { attempt } => {
                self.projection.mark_restarting();
                self.status_message = format!("OMP restarting (attempt {attempt})…");
                let resume = self.restart_resume_path();
                let cwd = self.restart_cwd(resume.as_deref());
                // Defer begin_connection so this pump task can finish; taking
                // `self.pump` from inside the pump would cancel ourselves mid-update.
                cx.spawn_in(window, async move |view, cx| {
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.begin_connection(window, cwd, resume, false, cx);
                    });
                })
                .detach();
            }
            SupervisorState::Dead {
                reason,
                stderr_tail,
                ..
            } => {
                let reason_msg = supervisor_death_reason_message(&reason);
                self.projection.mark_dead(reason_msg.clone());
                self.status_message = match reason {
                    DeathReason::CrashLoop => crash_loop_status_message(&stderr_tail),
                    _ => omp_closed_status_message(&stderr_tail),
                };
            }
            SupervisorState::Running => {
                // on_child_died never returns Running; keep a truthful fallback.
                let reason = info
                    .error_msg
                    .clone()
                    .unwrap_or_else(|| format!("exit code {:?}", info.exit_code));
                self.projection.mark_dead(reason);
                self.status_message = omp_closed_status_message(&info.stderr_tail);
            }
        }
    }

    /// Take the live RPC client (and its event pump) and gracefully close it
    /// on a foreground task: close stdin, then wait / kill within 10s.
    ///
    /// Call sites still clear their own UI / bridge state; this only owns the
    /// drop-path that would otherwise orphan an `omp` child while reader tasks
    /// hold the inner `Arc`.
    fn take_and_close_client(&mut self, cx: &mut Context<Self>) {
        self.pump.take();
        let Some(client) = self.client.take() else {
            return;
        };
        cx.spawn(async move |_, _| {
            let _ = gracefully_shutdown_rpc_client(client).await;
        })
        .detach();
    }

    fn mark_explicit_supervisor_shutdown(&mut self) {
        if let Some(supervisor) = &self.supervisor {
            supervisor.on_explicit_shutdown(None, String::new());
        }
    }

    /// Bind or refresh the supervisor after a successful connect.
    ///
    /// Auto-restarts reuse the existing supervisor (breaker window intact) and
    /// only flip it back to `Running`. Fresh connects replace any prior handle.
    fn attach_supervisor(&mut self, connect_resume: Option<PathBuf>) {
        let session_resume = self
            .projection
            .state
            .session_file
            .as_deref()
            .map(str::trim)
            .filter(|session| !session.is_empty())
            .map(PathBuf::from)
            .or(connect_resume);
        if let Some(supervisor) = &self.supervisor
            && matches!(supervisor.state(), SupervisorState::Restarting { .. })
        {
            supervisor.set_resume(session_resume);
            supervisor.on_child_running();
            return;
        }
        let supervisor = Supervisor::new(SupervisorConfig {
            extra_args: vec![],
            // Match try_connect_omp: persistent session by default (PLAN §4.7).
            no_session: false,
            resume: session_resume,
        });
        supervisor.on_child_running();
        self.supervisor = Some(supervisor);
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
        self.invalidate_omp_update();
        self.clear_abort_arm();
        self.take_and_close_client(cx);
        self.host_bridge.reset();
        self.clear_live_timers();
        self.version_gate_notice = None;
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
            self.dialog_timeout_gens.clear();
            self.dialog_input_bound_id = None;
            self.pending_custom_answer = None;
            self.dialog_input_empty = true;
            self.slash_menu = SlashMenuState::Closed;
            self.slash_selected = 0;
            self.transcript_list.reset(0);
            self.last_transcript_len = 0;
            self.last_transcript_fingerprint = 0;
            self.transcript_text_views.clear();
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
            let _ = view.update_in(cx, |this, window, cx| {
                this.finish_connection(result, cwd, resume, launcher_mode, window, cx);
            });
        })
        .detach();
    }

    pub(crate) fn finish_connection(
        &mut self,
        result: ConnectionResult,
        cwd: PathBuf,
        resume: Option<PathBuf>,
        launcher_mode: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.launcher_phase = if launcher_mode {
            LauncherPhase::Visible
        } else {
            LauncherPhase::Hidden
        };
        match result {
            Ok(ConnectedSession {
                client,
                projection,
                status,
                models,
                version_gate_notice,
                omp_bin,
                omp_env,
            }) => {
                self.available_models = models;
                self.projection = projection;
                self.subagent_subscription = SubagentSubscriptionLevel::Events;
                self.omp_version = Some(status.clone());
                self.version_gate_notice = version_gate_notice;
                self.omp_bin = Some(omp_bin);
                self.omp_env = omp_env;
                self.status_message = status;
                self.client = Some(client.clone());
                self.session_cwd = Some(cwd);
                self.launcher_phase = LauncherPhase::Hidden;
                self.launcher_error = None;
                self.transcript_list.reset(self.projection.transcript.len());
                self.transcript_list.set_follow_mode(FollowMode::Tail);
                self.last_transcript_len = self.projection.transcript.len();
                self.last_transcript_fingerprint = 0;
                self.transcript_text_views.clear();
                self.unread_below = 0;
                self.attach_supervisor(resume);
                self.sync_running_tools(cx);
                self.start_event_pump(window, &client, cx);
                self.last_session = self.persistence.load_last_session();
                self.recent_sessions = self.persistence.load_recent_sessions();
                self.start_catalog_load(cx);
                if let Some(detail) = self.pending_omp_update_failure.take() {
                    self.omp_update_ui = OmpUpdateUi::Failed { detail };
                } else {
                    self.start_omp_update_check(cx);
                }
            }
            Err(error) => {
                if let Some(detail) = self.pending_omp_update_failure.take() {
                    self.omp_update_ui = OmpUpdateUi::Failed { detail };
                }
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

    pub(crate) fn invalidate_omp_update(&mut self) {
        self.omp_update_generation = self.omp_update_generation.wrapping_add(1);
        self.omp_update_ui = OmpUpdateUi::Idle;
        self.pending_omp_update_failure = None;
    }

    pub(crate) fn start_omp_update_check(&mut self, cx: &mut Context<Self>) {
        let (Some(program), env) = (self.omp_bin.clone(), self.omp_env.clone()) else {
            self.omp_update_ui = OmpUpdateUi::Idle;
            return;
        };
        let generation = self.omp_update_generation;
        self.omp_update_ui = OmpUpdateUi::Checking;
        cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move { check_omp_update(&program, &env, &SystemRunner) })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.omp_update_generation != generation {
                    return;
                }
                this.omp_update_ui = match result {
                    OmpUpdateCheck::Available { current, latest } => {
                        OmpUpdateUi::Available { current, latest }
                    }
                    OmpUpdateCheck::UpToDate { .. } | OmpUpdateCheck::Failed { .. } => {
                        OmpUpdateUi::Idle
                    }
                };
                cx.notify();
            });
        })
        .detach();
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
        self.invalidate_omp_update();
        self.omp_bin = None;
        self.omp_env.clear();
        self.clear_abort_arm();
        self.mark_explicit_supervisor_shutdown();
        self.take_and_close_client(cx);
        self.host_bridge.reset();
        self.model_picker_open = false;
        self.thinking_picker_open = false;
        self.compact_open = false;
        self.handoff_confirm_open = false;
        self.close_slash_menu();
        self.clear_composer = true;
        self.composer_rows = COMPOSER_GROW_MIN_ROWS;
        self.composer_empty = true;
        if let Some(cwd) = self.session_cwd.take() {
            self.launcher_cwd = cwd;
        }
        self.projection = SessionProjection::new();
        self.clear_subagent_drawer_state();
        self.available_models.clear();
        self.expanded_tools.clear();
        self.running_tool_started.clear();
        self.clear_live_timers();
        self.dialog_timeout_gens.clear();
        self.dialog_input_bound_id = None;
        self.pending_custom_answer = None;
        self.dialog_input_empty = true;
        self.pending_dialog_input_sync = Some(("Enter a value…".to_owned(), String::new()));
        self.transcript_list.reset(0);
        self.last_transcript_len = 0;
        self.last_transcript_fingerprint = 0;
        self.transcript_text_views.clear();
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
        self.transcript_text_views
            .retain(|&(row_ix, _), _| row_ix < new_len);
        let fingerprint =
            transcript_tail_fingerprint(&self.projection.transcript, &self.expanded_tools);
        if (new_len != old_len || fingerprint != self.last_transcript_fingerprint) && new_len > 0 {
            let start = new_len.saturating_sub(4);
            self.transcript_list.remeasure_items(start..new_len);
        }
        self.last_transcript_len = new_len;
        self.last_transcript_fingerprint = fingerprint;
    }

    pub(crate) fn render_transcript_row(
        &mut self,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let tool_group = tool_group_position(&self.projection.transcript, ix);
        let turn_start = entry_starts_user_turn(&self.projection.transcript, ix);
        let Some(entry) = self.projection.transcript.get(ix).cloned() else {
            return div().into_any_element();
        };
        render_entry(
            &mut self.transcript_text_views,
            ix,
            &entry,
            tool_group,
            turn_start,
            &self.expanded_tools,
            &self.running_tool_started,
            cx,
        )
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

    pub(crate) fn sync_live_state_timer(&mut self, cx: &mut Context<Self>) {
        let live = matches!(
            self.projection.run_phase,
            RunPhase::Streaming | RunPhase::Compacting | RunPhase::Retrying
        );
        if !live || self.client.is_none() {
            self.live_state_timer.take();
            return;
        }
        if self.live_state_timer.is_some() {
            return;
        }
        self.live_state_timer = Some(cx.spawn(async move |view, cx| {
            loop {
                smol::Timer::after(Duration::from_secs(2)).await;
                let keep_going = view.update(cx, |this, cx| {
                    let live = matches!(
                        this.projection.run_phase,
                        RunPhase::Streaming | RunPhase::Compacting | RunPhase::Retrying
                    );
                    if live && this.client.is_some() {
                        this.refresh_state(cx);
                        true
                    } else {
                        this.live_state_timer.take();
                        false
                    }
                });
                if !matches!(keep_going, Ok(true)) {
                    break;
                }
            }
        }));
    }

    fn clear_live_timers(&mut self) {
        self.running_tool_timer.take();
        self.live_state_timer.take();
        self.state_refresh_in_flight = false;
        self.state_refresh_queued = false;
    }

    pub(crate) fn jump_to_transcript_tail(&mut self, cx: &mut Context<Self>) {
        self.transcript_list.scroll_to_end();
        self.transcript_list.set_follow_mode(FollowMode::Tail);
        self.unread_below = 0;
        cx.notify();
    }

    /// Ordinary compositor keystrokes should reach `InputState` without walking
    /// every overlay handler or `read`ing sibling inputs first.
    fn composer_typing_should_bypass_capture(
        &self,
        event: &KeyDownEvent,
        window: &Window,
        cx: &Context<Self>,
    ) -> bool {
        if event.keystroke.modifiers.modified() {
            return false;
        }
        match event.keystroke.key.as_str() {
            "escape" | "esc" | "up" | "down" | "tab" | "enter" | "pageup" | "page-up"
            | "pagedown" | "page-down" | "home" | "end" => return false,
            _ => {}
        }
        if self.slash_menu == SlashMenuState::Open
            || self.at_mention_open
            || self.palette_open
            || self.theme_picker_open
            || self.model_picker_open
            || self.thinking_picker_open
            || self.about_open
            || self.rename_open
            || self.compact_open
            || self.handoff_confirm_open
            || self.subagent_modal_open
            || self.branch_picker.is_some()
            || self.login_picker.is_some()
            || self.large_paste_pending.is_some()
            || !self.projection.pending_dialogs.is_empty()
            || self.host_bridge.has_pending_requests()
        {
            return false;
        }
        self.composer.read(cx).focus_handle(cx).is_focused(window)
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
        if self.subagent_snapshots.is_empty()
            && self.client.is_some()
            && !self.subagent_refresh_in_flight
        {
            self.refresh_subagents(cx);
        }
    }

    pub(crate) fn clear_subagent_drawer_state(&mut self) {
        self.subagent_snapshots.clear();
        self.subagent_refresh_in_flight = false;
        self.selected_subagent_id = None;
        self.subagent_modal_open = false;
        self.subagent_modal_status.clear();
        self.subagent_modal_request_generation =
            self.subagent_modal_request_generation.wrapping_add(1);
        self.subagent_tail_next_byte = None;
        self.subagent_tail_lines.clear();
        self.subagent_drawer_status.clear();
    }

    pub(crate) fn refresh_subagents(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            self.subagent_refresh_in_flight = false;
            "OMP is not connected".clone_into(&mut self.subagent_drawer_status);
            return;
        };
        if self.subagent_refresh_in_flight {
            return;
        }
        self.subagent_refresh_in_flight = true;
        "Loading agents…".clone_into(&mut self.subagent_drawer_status);
        cx.spawn(async move |view, cx| {
            let result = client.send(RpcCommandBody::GetSubagents).await;
            let _ = view.update(cx, |this, cx| {
                this.subagent_refresh_in_flight = false;
                match result {
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
                }
            });
        })
        .detach();
    }

    pub(crate) fn cycle_subagent_subscription(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let next = next_subagent_subscription_level(&self.subagent_subscription);
        let next_for_command = next.clone();
        cx.spawn(async move |view, cx| {
            let result = client
                .send(RpcCommandBody::SetSubagentSubscription {
                    level: next_for_command,
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(response) if response.success => {
                        this.subagent_subscription = next;
                        this.subagent_drawer_status = format!(
                            "Agent event subscription: {}",
                            this.subagent_subscription.as_wire()
                        );
                    }
                    Ok(response) => {
                        this.subagent_drawer_status = response
                            .error
                            .unwrap_or_else(|| "set_subagent_subscription failed".to_owned());
                    }
                    Err(error) => {
                        this.subagent_drawer_status = format!("set_subagent_subscription: {error}");
                    }
                }
                cx.notify();
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
        let retained_selection = retained_subagent_selection(
            self.selected_subagent_id.as_deref(),
            &self.subagent_snapshots,
        );
        if self.selected_subagent_id.is_some() && retained_selection.is_none() {
            self.subagent_modal_open = false;
            self.subagent_modal_request_generation =
                self.subagent_modal_request_generation.wrapping_add(1);
            self.subagent_modal_status.clear();
            self.subagent_tail_next_byte = None;
            self.subagent_tail_lines.clear();
        }
        self.selected_subagent_id = retained_selection;

        if self.subagent_snapshots.is_empty() {
            "No agents reported".clone_into(&mut self.subagent_drawer_status);
        } else {
            self.subagent_drawer_status.clear();
        }
        cx.notify();
    }

    pub(crate) fn open_subagent_modal(&mut self, subagent_id: String, cx: &mut Context<Self>) {
        let same_selection = self.selected_subagent_id.as_deref() == Some(subagent_id.as_str());
        if !same_selection {
            self.selected_subagent_id = Some(subagent_id.clone());
            self.subagent_tail_next_byte = None;
            self.subagent_tail_lines.clear();
        }
        self.subagent_modal_open = true;
        self.subagent_modal_request_generation =
            self.subagent_modal_request_generation.wrapping_add(1);
        let request_generation = self.subagent_modal_request_generation;
        let from_byte = same_selection
            .then_some(self.subagent_tail_next_byte)
            .flatten();
        self.fetch_subagent_messages(subagent_id, from_byte, request_generation, cx);
        cx.notify();
    }

    pub(crate) fn close_subagent_modal(&mut self, cx: &mut Context<Self>) {
        if self.subagent_modal_open {
            self.subagent_modal_open = false;
            self.subagent_modal_request_generation =
                self.subagent_modal_request_generation.wrapping_add(1);
            cx.notify();
        }
    }

    fn fetch_subagent_messages(
        &mut self,
        subagent_id: String,
        from_byte: Option<u64>,
        request_generation: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.client.clone() else {
            "OMP is not connected".clone_into(&mut self.subagent_modal_status);
            return;
        };
        let session_file = self
            .subagent_snapshots
            .iter()
            .find(|snapshot| subagent_snapshot_id(snapshot) == Some(subagent_id.as_str()))
            .and_then(subagent_snapshot_session_file)
            .map(str::to_owned);
        self.subagent_modal_status = if from_byte.is_some() {
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
                if !this.subagent_modal_open
                    || this.selected_subagent_id.as_deref() != Some(subagent_id.as_str())
                    || this.subagent_modal_request_generation != request_generation
                {
                    return;
                }
                match result {
                    Ok(response) if response.success => {
                        this.apply_subagent_message_page(response.data.as_ref());
                    }
                    Ok(response) => {
                        this.subagent_modal_status = response
                            .error
                            .unwrap_or_else(|| "get_subagent_messages failed".to_owned());
                    }
                    Err(error) => {
                        this.subagent_modal_status = format!("get_subagent_messages: {error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn apply_subagent_message_page(&mut self, data: Option<&serde_json::Value>) {
        let Some(data) = data else {
            "No message payload returned".clone_into(&mut self.subagent_modal_status);
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
        self.subagent_modal_status = format!(
            "{} message(s){}",
            self.subagent_tail_lines.len(),
            if data.get("reset").and_then(serde_json::Value::as_bool) == Some(true) {
                " (reset)"
            } else {
                ""
            }
        );
    }

    pub(crate) fn handle_subagent_modal_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.subagent_modal_open {
            return false;
        }
        if !event.keystroke.modifiers.modified()
            && matches!(event.keystroke.key.as_str(), "escape" | "esc")
        {
            self.close_subagent_modal(cx);
        }
        true
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
                    let reveal_path = PathBuf::from(&path);
                    let _ = reveal_path_in_file_manager(&reveal_path);
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

    pub(crate) fn share_session(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "Connect to OMP, then run /share.".to_owned(),
            ));
            cx.notify();
            return;
        };
        let streaming_behavior =
            composer_uses_steer(&self.projection.run_phase).then_some(StreamingBehavior::Steer);
        self.projection.push_user_message("/share".to_owned());
        cx.notify();
        cx.spawn(async move |view, cx| {
            let result = client
                .send(RpcCommandBody::Prompt {
                    message: "/share".to_owned(),
                    images: None,
                    streaming_behavior,
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(response) if response.success => {
                        this.projection.transcript.push(TranscriptEntry::Notice(
                            "Share requested via /share; OMP will report the result.".to_owned(),
                        ));
                    }
                    Ok(response) => {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: response
                                .error
                                .unwrap_or_else(|| "/share prompt failed".to_owned()),
                            code: Some("share".to_owned()),
                        });
                    }
                    Err(error) => {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("/share: {error}"),
                            code: Some("share".to_owned()),
                        });
                    }
                }
                cx.notify();
            });
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
            self.state_refresh_in_flight = false;
            self.state_refresh_queued = false;
            return;
        };
        if self.state_refresh_in_flight {
            self.state_refresh_queued = true;
            return;
        }
        self.state_refresh_in_flight = true;
        cx.spawn(async move |view, cx| {
            let resp = client.send(RpcCommandBody::GetState).await;
            let _ = view.update(cx, |this, cx| {
                this.state_refresh_in_flight = false;
                if let Ok(resp) = resp
                    && resp.success
                    && let Some(data) = resp.data
                {
                    this.projection.hydrate_get_state(&data);
                    if let Some(supervisor) = &this.supervisor {
                        let path = this
                            .projection
                            .state
                            .session_file
                            .as_deref()
                            .map(str::trim)
                            .filter(|session| !session.is_empty())
                            .map(PathBuf::from);
                        supervisor.set_resume(path);
                    }
                    this.sync_status_model();
                    cx.notify();
                }
                if this.state_refresh_queued {
                    this.state_refresh_queued = false;
                    this.refresh_state(cx);
                }
            });
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
        let previous = self.projection.state.fast_mode_enabled;
        let enabled = !previous.unwrap_or(false);
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
                        this.projection.state.fast_mode_enabled = previous;
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(error));
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.fast_mode_enabled = previous;
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

    pub(crate) fn toggle_steering_mode(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let previous = self.projection.state.steering_mode.clone();
        let mode = cycle_queue_mode(previous.as_deref());
        self.projection.state.steering_mode = Some(mode.as_wire().to_owned());
        cx.notify();

        cx.spawn(async move |view, cx| {
            match client.send(RpcCommandBody::SetSteeringMode { mode }).await {
                Ok(resp) if resp.success => {
                    let _ = view.update(cx, SessionView::refresh_state);
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .unwrap_or_else(|| "set_steering_mode failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.steering_mode = previous;
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(error));
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.steering_mode = previous;
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("set_steering_mode: {error}"),
                            code: Some("set_steering_mode".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn toggle_follow_up_mode(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let previous = self.projection.state.follow_up_mode.clone();
        let mode = cycle_queue_mode(previous.as_deref());
        self.projection.state.follow_up_mode = Some(mode.as_wire().to_owned());
        cx.notify();

        cx.spawn(async move |view, cx| {
            match client.send(RpcCommandBody::SetFollowUpMode { mode }).await {
                Ok(resp) if resp.success => {
                    let _ = view.update(cx, SessionView::refresh_state);
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .unwrap_or_else(|| "set_follow_up_mode failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.follow_up_mode = previous;
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(error));
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.follow_up_mode = previous;
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("set_follow_up_mode: {error}"),
                            code: Some("set_follow_up_mode".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn toggle_interrupt_mode(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let previous = self.projection.state.interrupt_mode.clone();
        let mode = cycle_interrupt_mode(previous.as_deref());
        self.projection.state.interrupt_mode = Some(mode.as_wire().to_owned());
        cx.notify();

        cx.spawn(async move |view, cx| {
            match client.send(RpcCommandBody::SetInterruptMode { mode }).await {
                Ok(resp) if resp.success => {
                    let _ = view.update(cx, SessionView::refresh_state);
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .unwrap_or_else(|| "set_interrupt_mode failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.interrupt_mode = previous;
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(error));
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.interrupt_mode = previous;
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("set_interrupt_mode: {error}"),
                            code: Some("set_interrupt_mode".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn toggle_auto_compaction(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let previous = self.projection.state.auto_compaction_enabled;
        let enabled = !previous.unwrap_or(false);
        self.projection.state.auto_compaction_enabled = Some(enabled);
        cx.notify();

        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::SetAutoCompaction { enabled })
                .await
            {
                Ok(resp) if resp.success => {
                    let _ = view.update(cx, SessionView::refresh_state);
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .unwrap_or_else(|| "set_auto_compaction failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.auto_compaction_enabled = previous;
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(error));
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.auto_compaction_enabled = previous;
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("set_auto_compaction: {error}"),
                            code: Some("set_auto_compaction".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn toggle_auto_retry(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let previous = self.projection.state.auto_retry_enabled;
        let enabled = !previous.unwrap_or(false);
        self.projection.state.auto_retry_enabled = Some(enabled);
        cx.notify();

        cx.spawn(async move |view, cx| {
            match client.send(RpcCommandBody::SetAutoRetry { enabled }).await {
                Ok(resp) if resp.success => {
                    let _ = view.update(cx, SessionView::refresh_state);
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .unwrap_or_else(|| "set_auto_retry failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.auto_retry_enabled = previous;
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(error));
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.state.auto_retry_enabled = previous;
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("set_auto_retry: {error}"),
                            code: Some("set_auto_retry".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn assign_current_model_to_role(&mut self, role: &str, cx: &mut Context<Self>) {
        let Some((provider, id)) = self
            .projection
            .state
            .model
            .as_deref()
            .and_then(split_model_label)
        else {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "No current model to assign to a role".into(),
            ));
            cx.notify();
            return;
        };
        let Some(omp_bin) = self.omp_bin.as_deref() else {
            self.projection.transcript.push(TranscriptEntry::Error {
                message: "omp binary not discovered yet; cannot assign model role".into(),
                code: Some("modelRoles".into()),
            });
            cx.notify();
            return;
        };
        match assign_omp_model_role(omp_bin, role, &provider, &id) {
            Ok(()) => {
                self.omp_roles = load_omp_roles_from_home(home_dir().as_deref());
                self.projection
                    .transcript
                    .push(TranscriptEntry::Notice(format!(
                        "Assigned {provider}/{id} → @{role} (via omp config)"
                    )));
            }
            Err(error) => {
                self.projection.transcript.push(TranscriptEntry::Error {
                    message: error,
                    code: Some("modelRoles".into()),
                });
            }
        }
        cx.notify();
    }

    #[allow(clippy::unused_self)] // instance API for listeners; work is in the path-prompt future
    pub(crate) fn prompt_attach_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let _ = view.update(cx, |this, cx| {
                this.add_attachment_paths(&paths, cx);
            });
        })
        .detach();
    }

    pub(crate) fn session_cwd_path(&self) -> PathBuf {
        self.session_cwd
            .clone()
            .unwrap_or_else(|| self.launcher_cwd.clone())
    }

    pub(crate) fn composer_draft(&self, cx: &Context<Self>) -> String {
        self.pending_composer_value
            .clone()
            .unwrap_or_else(|| self.composer.read(cx).value().to_string())
    }

    pub(crate) fn set_composer_draft(&mut self, text: String, cx: &mut Context<Self>) {
        self.composer_rows = composer_hard_rows(&text);
        self.composer_empty = text.trim().is_empty();
        self.pending_composer_value = Some(text);
        self.refocus_composer = true;
        cx.notify();
    }

    pub(crate) fn append_composer_fragment(&mut self, fragment: &str, cx: &mut Context<Self>) {
        let mut draft = self.composer_draft(cx);
        if !draft.is_empty() && !draft.ends_with(' ') && !draft.ends_with('\n') {
            draft.push(' ');
        }
        draft.push_str(fragment);
        if !fragment.ends_with(' ') {
            draft.push(' ');
        }
        self.set_composer_draft(draft, cx);
    }

    pub(crate) fn add_attachment_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let mut failed = 0usize;
        let cwd = self.session_cwd_path();
        for path in paths {
            if self
                .pending_attachments
                .iter()
                .any(|existing| existing.matches_path(path))
            {
                continue;
            }
            let next_index = next_image_marker_index(&self.pending_attachments);
            match load_pending_attachment(path, next_index) {
                Ok(attachment) => {
                    let attachment = match attachment {
                        PendingAttachment::PathMention {
                            path: mention_path, ..
                        } => {
                            let display = path_mention_display(&mention_path, Some(cwd.as_path()));
                            PendingAttachment::PathMention {
                                path: mention_path,
                                display,
                            }
                        }
                        other @ PendingAttachment::Image { .. } => other,
                    };
                    self.add_pending_attachment(attachment, cx);
                }
                Err(_) => failed += 1,
            }
        }
        if failed > 0 {
            self.projection
                .transcript
                .push(TranscriptEntry::Notice(format!(
                    "skipped {failed} unreadable attachment(s)"
                )));
        }
        cx.notify();
    }

    pub(crate) fn add_pending_attachment(
        &mut self,
        attachment: PendingAttachment,
        cx: &mut Context<Self>,
    ) {
        match &attachment {
            PendingAttachment::Image {
                marker_index,
                width,
                height,
                ..
            } => {
                let marker = image_marker(*marker_index, *width, *height);
                if !image_marker_present(&self.composer_draft(cx), *marker_index) {
                    self.append_composer_fragment(&marker, cx);
                }
            }
            PendingAttachment::PathMention { display, .. } => {
                if !self.composer_draft(cx).contains(display.as_str()) {
                    let insert = display.clone();
                    self.append_composer_fragment(&insert, cx);
                }
            }
        }
        self.pending_attachments.push(attachment);
        cx.notify();
    }

    pub(crate) fn remove_attachment_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.pending_attachments.len() {
            return;
        }
        let removed = self.pending_attachments.remove(index);
        let mut draft = self.composer_draft(cx);
        match removed {
            PendingAttachment::Image { marker_index, .. } => {
                draft = strip_image_marker(&draft, marker_index);
            }
            PendingAttachment::PathMention { display, .. } => {
                draft = strip_path_mention(&draft, &display);
            }
        }
        self.set_composer_draft(draft, cx);
        cx.notify();
    }

    pub(crate) fn on_recent_filter_event(
        &mut self,
        _input: gpui::Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) && self.launcher_phase != LauncherPhase::Hidden {
            cx.notify();
        }
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

    #[allow(clippy::needless_pass_by_value)] // gpui subscribe callback owns the input entity.
    pub(crate) fn on_dialog_input_event(
        &mut self,
        input: gpui::Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let empty = input.read(cx).value().trim().is_empty();
                if self.dialog_input_empty != empty {
                    self.dialog_input_empty = empty;
                    cx.notify();
                }
            }
            // Single-line `input` submits on Enter; multiline fields keep
            // newlines and submit with ⌘↩ / Submit.
            InputEvent::PressEnter {
                secondary: false,
                shift: false,
            } if self
                .projection
                .pending_dialogs
                .first()
                .is_some_and(|d| d.method == "input") =>
            {
                self.submit_dialog_input(cx);
            }
            InputEvent::PressEnter {
                secondary: true,
                shift: false,
            } => {
                if self.current_select_is_custom() {
                    self.confirm_dialog_selection(cx);
                } else {
                    self.submit_dialog_input(cx);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn submit_dialog_input(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.projection.pending_dialogs.first() else {
            return;
        };
        if !matches!(dialog.method.as_str(), "input" | "editor") {
            return;
        }
        let id = dialog.id.clone();
        let value = self.dialog_input.read(cx).value().to_string();
        if value.trim().is_empty() {
            return;
        }
        let mut fields = serde_json::Map::new();
        fields.insert("value".into(), serde_json::Value::String(value));
        self.submit_dialog_response(&id, fields, cx);
    }

    /// Bind dialog text field + schedule timeouts when pending dialogs change.
    pub(crate) fn sync_pending_dialogs(&mut self, cx: &mut Context<Self>) {
        self.sync_select_draft();
        self.flush_pending_custom_answer(cx);
        self.sync_dialog_input(cx);
        self.sync_dialog_timeouts(cx);
    }

    pub(crate) fn current_select_is_custom(&self) -> bool {
        let Some(dialog) = self.projection.pending_dialogs.first() else {
            return false;
        };
        self.select_draft
            .as_ref()
            .is_some_and(|draft| select_option_is_custom(dialog, draft))
    }

    pub(crate) fn sync_select_draft(&mut self) {
        let Some(dialog) = self.projection.pending_dialogs.first() else {
            self.select_draft = None;
            return;
        };
        if dialog.method != "select" {
            self.select_draft = None;
            return;
        }
        if self
            .select_draft
            .as_ref()
            .is_some_and(|draft| draft.dialog_id == dialog.id)
        {
            return;
        }
        let (title, choice_values) = select_question_fingerprint(dialog);
        let (restore_value, last_sent_value, checked_values) = match &self.select_memory {
            Some(memory) if memory.title == title && memory.choice_values == choice_values => (
                memory.selected_value.clone(),
                memory.last_sent_value.clone(),
                memory.checked_values.clone(),
            ),
            _ => (None, None, Vec::new()),
        };
        let mut draft = select_draft_for_dialog(dialog);
        if let Some(value) = restore_value.as_deref()
            && let Some(index) = dialog_primary_options(dialog)
                .iter()
                .position(|option| option.value == value || option.label == value)
        {
            draft.selected = Some(index);
        }
        draft.checked_values.clone_from(&checked_values);
        let selected_value = draft
            .selected
            .and_then(|index| choice_values.get(index).cloned());
        self.select_draft = Some(draft);
        self.select_memory = Some(SelectMemory {
            title,
            choice_values,
            selected_value,
            last_sent_value,
            checked_values,
        });
    }

    fn remember_select_choice(&mut self, value: &str) {
        let Some(memory) = self.select_memory.as_mut() else {
            return;
        };
        memory.last_sent_value = Some(value.to_owned());
        let is_choice = memory.choice_values.iter().any(|choice| choice == value)
            && !is_custom_ask_option(value)
            && !is_select_completion_option(value);
        if is_choice {
            memory.selected_value = Some(value.to_owned());
            toggle_checked_value(&mut memory.checked_values, value);
        }
        if let Some(draft) = self.select_draft.as_mut() {
            draft.checked_values.clone_from(&memory.checked_values);
        }
    }

    pub(crate) fn mark_dialog_selection(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some((id, value, toggle_now, checked_values)) =
            self.projection.pending_dialogs.first().and_then(|dialog| {
                if dialog.method != "select" {
                    return None;
                }
                let options = dialog_primary_options(dialog);
                let option = options.get(idx)?;
                Some((
                    dialog.id.clone(),
                    option.value.clone(),
                    select_completion_value(dialog).is_some() && option_is_choice(option),
                    self.select_memory
                        .as_ref()
                        .map(|memory| memory.checked_values.clone())
                        .unwrap_or_default(),
                ))
            })
        else {
            return;
        };
        self.select_draft = Some(SelectDraft {
            dialog_id: id.clone(),
            selected: Some(idx),
            checked_values,
        });
        if let Some(memory) = self.select_memory.as_mut() {
            memory.selected_value = Some(value.clone());
        }
        if toggle_now {
            self.remember_select_choice(&value);
            let mut fields = serde_json::Map::new();
            fields.insert("value".into(), serde_json::Value::String(value));
            self.submit_dialog_response(&id, fields, cx);
            return;
        }
        self.sync_dialog_input(cx);
        cx.notify();
    }

    pub(crate) fn confirm_dialog_selection(&mut self, cx: &mut Context<Self>) {
        let Some((id, choice_options, completion, selected, last_sent, checked, needs_typed)) =
            self.projection.pending_dialogs.first().and_then(|dialog| {
                if dialog.method != "select" {
                    return None;
                }
                let draft = self.select_draft.as_ref()?;
                if draft.dialog_id != dialog.id {
                    return None;
                }
                let choice_options = dialog_primary_options(dialog);
                let selected = draft.selected;
                let needs_typed = selected
                    .and_then(|idx| choice_options.get(idx))
                    .is_some_and(option_is_custom);
                Some((
                    dialog.id.clone(),
                    choice_options,
                    select_completion_value(dialog),
                    selected,
                    self.select_memory
                        .as_ref()
                        .and_then(|memory| memory.last_sent_value.clone()),
                    self.select_memory
                        .as_ref()
                        .map(|memory| memory.checked_values.clone())
                        .unwrap_or_default(),
                    needs_typed,
                ))
            })
        else {
            return;
        };
        let typed = if needs_typed {
            self.dialog_input.read(cx).value().to_string()
        } else {
            String::new()
        };
        match resolve_select_continue(
            &choice_options,
            selected,
            completion.as_deref(),
            last_sent.as_deref(),
            &checked,
            &typed,
        ) {
            SelectContinue::Nothing | SelectContinue::NeedCustomText => {}
            SelectContinue::Send(value) => {
                self.remember_select_choice(&value);
                let mut fields = serde_json::Map::new();
                fields.insert("value".into(), serde_json::Value::String(value));
                self.submit_dialog_response(&id, fields, cx);
            }
            SelectContinue::SendCustom { wire_value } => {
                self.pending_custom_answer = Some(typed);
                self.remember_select_choice(&wire_value);
                let mut fields = serde_json::Map::new();
                fields.insert("value".into(), serde_json::Value::String(wire_value));
                self.submit_dialog_response(&id, fields, cx);
            }
            SelectContinue::FinishCustom {
                other_value,
                answer,
            } => {
                self.pending_custom_answer = Some(answer);
                self.remember_select_choice(&other_value);
                let mut fields = serde_json::Map::new();
                fields.insert("value".into(), serde_json::Value::String(other_value));
                self.submit_dialog_response(&id, fields, cx);
            }
        }
    }

    pub(crate) fn flush_pending_custom_answer(&mut self, cx: &mut Context<Self>) {
        let Some(answer) = self.pending_custom_answer.take() else {
            return;
        };
        let Some(id) = self.projection.pending_dialogs.first().and_then(|dialog| {
            matches!(dialog.method.as_str(), "input" | "editor").then(|| dialog.id.clone())
        }) else {
            self.pending_custom_answer = Some(answer);
            return;
        };
        let mut fields = serde_json::Map::new();
        fields.insert("value".into(), serde_json::Value::String(answer.clone()));
        if !self.send_dialog_response(&id, fields, cx) {
            self.pending_custom_answer = Some(answer);
        }
    }

    /// Send an extension-UI response while `SessionView` is already leased.
    /// Callers that only have a `WeakEntity` should use [`do_dialog_response`].
    pub(crate) fn submit_dialog_response(
        &mut self,
        id: &str,
        fields: serde_json::Map<String, serde_json::Value>,
        cx: &mut Context<Self>,
    ) {
        if !self.send_dialog_response(id, fields, cx) {
            return;
        }
        self.sync_pending_dialogs(cx);
        cx.notify();
    }

    pub(crate) fn send_dialog_response(
        &mut self,
        id: &str,
        fields: serde_json::Map<String, serde_json::Value>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(client) = self.client.clone() else {
            return false;
        };
        if !self.projection.pending_dialogs.iter().any(|d| d.id == id) {
            return false;
        }
        let id_owned = id.to_owned();
        cx.spawn(async move |_, _| {
            let _ = client
                .send(RpcCommandBody::ExtensionUiResponse {
                    id: id_owned,
                    fields,
                })
                .await;
        })
        .detach();
        self.projection.pending_dialogs.retain(|d| d.id != id);
        true
    }

    pub(crate) fn sync_dialog_input(&mut self, _cx: &mut Context<Self>) {
        let text_dialog = self
            .projection
            .pending_dialogs
            .iter()
            .find(|d| matches!(d.method.as_str(), "input" | "editor"))
            .cloned();
        if let Some(dialog) = text_dialog {
            if self.dialog_input_bound_id.as_deref() == Some(dialog.id.as_str()) {
                return;
            }
            self.dialog_input_bound_id = Some(dialog.id.clone());
            let placeholder = dialog
                .payload
                .get("placeholder")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(if dialog.method == "editor" {
                    "Edit text…"
                } else {
                    "Enter a value…"
                })
                .to_owned();
            let prefill = dialog
                .payload
                .get("prefill")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            self.dialog_input_empty = prefill.trim().is_empty();
            self.pending_dialog_input_sync = Some((placeholder, prefill));
            return;
        }

        if self.current_select_is_custom() {
            let Some(dialog_id) = self
                .projection
                .pending_dialogs
                .first()
                .map(|dialog| dialog.id.clone())
            else {
                return;
            };
            let bind_id = format!("{dialog_id}#custom");
            if self.dialog_input_bound_id.as_deref() == Some(bind_id.as_str()) {
                return;
            }
            self.dialog_input_bound_id = Some(bind_id);
            self.dialog_input_empty = true;
            self.pending_dialog_input_sync =
                Some(("Type your own answer…".to_owned(), String::new()));
            return;
        }

        if self.dialog_input_bound_id.take().is_some() {
            self.dialog_input_empty = true;
            self.pending_dialog_input_sync = Some(("Enter a value…".to_owned(), String::new()));
        }
    }

    pub(crate) fn sync_dialog_timeouts(&mut self, cx: &mut Context<Self>) {
        let pending: HashMap<String, Option<f64>> = self
            .projection
            .pending_dialogs
            .iter()
            .map(|d| (d.id.clone(), d.timeout_ms))
            .collect();
        self.dialog_timeout_gens
            .retain(|id, _| pending.contains_key(id));

        for (id, timeout_ms) in pending {
            let Some(ms) = timeout_ms else {
                continue;
            };
            if !(ms.is_finite() && ms > 0.0) || self.dialog_timeout_gens.contains_key(&id) {
                continue;
            }
            self.dialog_timeout_generation = self.dialog_timeout_generation.wrapping_add(1);
            let generation = self.dialog_timeout_generation;
            self.dialog_timeout_gens.insert(id.clone(), generation);
            let duration = Duration::from_secs_f64(ms / 1000.0);
            cx.spawn(async move |view, cx| {
                smol::Timer::after(duration).await;
                let _ = view.update(cx, |this, cx| {
                    if this.dialog_timeout_gens.get(&id) != Some(&generation) {
                        return;
                    }
                    this.dialog_timeout_gens.remove(&id);
                    if !this.projection.pending_dialogs.iter().any(|d| d.id == id) {
                        return;
                    }
                    let Some(client) = this.client.clone() else {
                        this.projection.pending_dialogs.retain(|d| d.id != id);
                        this.sync_pending_dialogs(cx);
                        cx.notify();
                        return;
                    };
                    this.projection.pending_dialogs.retain(|d| d.id != id);
                    this.sync_pending_dialogs(cx);
                    let fields = dialog_cancel_fields(true);
                    let id_owned = id.clone();
                    cx.spawn(async move |_, _| {
                        let _ = client
                            .send(RpcCommandBody::ExtensionUiResponse {
                                id: id_owned,
                                fields,
                            })
                            .await;
                    })
                    .detach();
                    cx.notify();
                });
            })
            .detach();
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
                let slash_was_open = self.slash_menu == SlashMenuState::Open;
                let mention_was_open = self.at_mention_open;
                if self.slash_menu == SlashMenuState::Dismissed {
                    self.slash_menu = SlashMenuState::Closed;
                }
                let text = self.composer.read(cx).value().to_string();
                self.update_slash_menu_from(&text);
                self.update_at_mention_menu_from(&text);
                let empty = text.trim().is_empty();
                let emptiness_flipped = empty != self.composer_empty;
                self.composer_empty = empty;
                let rows = composer_hard_rows(&text);
                let rows_changed = rows != self.composer_rows;
                self.composer_rows = rows;
                let menus_need_paint = slash_was_open
                    || mention_was_open
                    || self.slash_menu == SlashMenuState::Open
                    || self.at_mention_open;
                if menus_need_paint || emptiness_flipped || rows_changed {
                    cx.notify();
                }
            }
            InputEvent::PressEnter {
                secondary,
                shift: false,
            } => {
                if self.at_mention_open {
                    if let Some(path) = self.at_mention_candidates.get(self.at_mention_selected) {
                        let path = path.clone();
                        self.accept_at_mention(&path, cx);
                    }
                    return;
                }
                if self.large_paste_pending.is_some() {
                    return;
                }
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

    pub(crate) fn update_at_mention_menu_from(&mut self, text: &str) {
        let Some(query) = at_mention_query(text) else {
            self.at_mention_open = false;
            self.at_mention_candidates.clear();
            return;
        };
        let cwd = self.session_cwd_path();
        if self.at_mention_index_cwd.as_deref() != Some(cwd.as_path()) {
            self.at_mention_index = list_cwd_files_for_at_mention(&cwd, "");
            self.at_mention_index_cwd = Some(cwd.clone());
        }
        let needle = query.to_ascii_lowercase();
        self.at_mention_candidates = self
            .at_mention_index
            .iter()
            .filter(|path| {
                needle.is_empty()
                    || path
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .contains(&needle)
            })
            .take(12)
            .cloned()
            .collect();
        self.at_mention_selected = self
            .at_mention_selected
            .min(self.at_mention_candidates.len().saturating_sub(1));
        self.at_mention_open = !self.at_mention_candidates.is_empty();
    }

    pub(crate) fn accept_at_mention(&mut self, path: &Path, cx: &mut Context<Self>) {
        let cwd = self.session_cwd_path();
        let display = path_mention_display(path, Some(cwd.as_path()));
        let draft = self.composer_draft(cx);
        let next = replace_at_mention_token(&draft, &display);
        self.set_composer_draft(next, cx);
        if !self
            .pending_attachments
            .iter()
            .any(|existing| existing.matches_path(path))
        {
            self.pending_attachments
                .push(PendingAttachment::PathMention {
                    path: path.to_owned(),
                    display,
                });
        }
        self.at_mention_open = false;
        self.at_mention_candidates.clear();
        cx.notify();
    }

    pub(crate) fn handle_composer_paste_key(
        &mut self,
        event: &KeyDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.to_ascii_lowercase();
        let is_paste =
            key == "v" && (event.keystroke.modifiers.platform || event.keystroke.modifiers.control);
        if !is_paste {
            return false;
        }
        if !self.composer.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        self.handle_clipboard_item(&item, cx)
    }

    pub(crate) fn handle_clipboard_item(
        &mut self,
        item: &ClipboardItem,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut handled = false;
        let mut path_bufs = Vec::new();
        for entry in item.entries() {
            match entry {
                gpui::ClipboardEntry::Image(image) => {
                    let next = next_image_marker_index(&self.pending_attachments);
                    match load_pending_image_bytes(&image.bytes, next, &format!("clipboard-{next}"))
                    {
                        Ok(attachment) => {
                            self.add_pending_attachment(attachment, cx);
                            handled = true;
                        }
                        Err(err) => {
                            self.projection
                                .transcript
                                .push(TranscriptEntry::Notice(format!("clipboard image: {err}")));
                            handled = true;
                        }
                    }
                }
                gpui::ClipboardEntry::ExternalPaths(paths) => {
                    path_bufs.extend(paths.paths().iter().cloned());
                    handled = true;
                }
                gpui::ClipboardEntry::String(_) => {}
            }
        }
        if !path_bufs.is_empty() {
            self.add_attachment_paths(&path_bufs, cx);
            return true;
        }
        if handled {
            return true;
        }

        let Some(text) = item.text() else {
            return false;
        };
        let paths = paths_from_paste_text(&text);
        if !paths.is_empty() {
            self.add_attachment_paths(&paths, cx);
            return true;
        }

        let lines = count_text_lines(&text);
        let threshold = large_paste_threshold();
        if threshold > 0 && lines >= threshold {
            self.large_paste_pending = Some(text);
            cx.notify();
            return true;
        }
        false
    }

    pub(crate) fn apply_large_paste_choice(
        &mut self,
        choice: LargePasteChoice,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.large_paste_pending.take() else {
            return;
        };
        match choice {
            LargePasteChoice::Wrap => {
                self.append_composer_fragment(&wrap_attachment(&text), cx);
            }
            LargePasteChoice::SaveLocal => {
                if let Ok(reference) = self.save_local_paste(&text) {
                    self.append_composer_fragment(&reference, cx);
                } else {
                    let lines = count_text_lines(&text);
                    self.paste_counter = self.paste_counter.saturating_add(1);
                    let marker = inline_paste_marker(
                        usize::try_from(self.paste_counter).unwrap_or(usize::MAX),
                        lines,
                        text.len(),
                    );
                    self.append_composer_fragment(&format!("{marker}\n{text}"), cx);
                    self.projection.transcript.push(TranscriptEntry::Notice(
                        "failed to save local://paste — inlined instead".into(),
                    ));
                }
            }
            LargePasteChoice::Inline => {
                let lines = count_text_lines(&text);
                self.paste_counter = self.paste_counter.saturating_add(1);
                let marker = inline_paste_marker(
                    usize::try_from(self.paste_counter).unwrap_or(usize::MAX),
                    lines,
                    text.len(),
                );
                // Keep full text in the draft (marker is a visual cue for the model).
                self.append_composer_fragment(&format!("{marker}\n{text}"), cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn save_local_paste(&mut self, text: &str) -> Result<String, String> {
        let dir = omp_local_paste_dir(self.projection.state.session_file.as_deref());
        std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
        loop {
            self.paste_counter = self.paste_counter.saturating_add(1);
            let name = format!("paste-{}.md", self.paste_counter);
            let path = dir.join(&name);
            if path.exists() {
                continue;
            }
            std::fs::write(&path, text).map_err(|err| err.to_string())?;
            return Ok(format!("local://{name}"));
        }
    }

    pub(crate) fn handle_attachment_overlay_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.large_paste_pending.is_some() {
            let key = event.keystroke.key.to_ascii_lowercase();
            return match key.as_str() {
                "1" | "w" => {
                    self.apply_large_paste_choice(LargePasteChoice::Wrap, cx);
                    true
                }
                "2" | "s" => {
                    self.apply_large_paste_choice(LargePasteChoice::SaveLocal, cx);
                    true
                }
                "escape" | "esc" | "3" | "i" => {
                    self.apply_large_paste_choice(LargePasteChoice::Inline, cx);
                    true
                }
                _ => true, // consume while menu open
            };
        }
        if !self.at_mention_open || self.at_mention_candidates.is_empty() {
            return false;
        }
        let key = event.keystroke.key.to_ascii_lowercase();
        match key.as_str() {
            "escape" | "esc" => {
                self.at_mention_open = false;
                cx.notify();
                true
            }
            "up" | "arrowup" => {
                let len = self.at_mention_candidates.len();
                self.at_mention_selected = (self.at_mention_selected + len - 1) % len;
                cx.notify();
                true
            }
            "down" | "arrowdown" => {
                let len = self.at_mention_candidates.len();
                self.at_mention_selected = (self.at_mention_selected + 1) % len;
                cx.notify();
                true
            }
            "enter" | "return" | "tab" => {
                if let Some(path) = self.at_mention_candidates.get(self.at_mention_selected) {
                    let path = path.clone();
                    self.accept_at_mention(&path, cx);
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn send_composer_message(&mut self, cx: &mut Context<Self>) {
        let text = self.composer_draft(cx);
        if text.trim().is_empty() && self.pending_attachments.is_empty() {
            return;
        }
        if !self.can_send() {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };

        let steer = composer_uses_steer(&self.projection.run_phase);
        let wire_images = pending_images_to_wire(&self.pending_attachments);
        let images = (!wire_images.is_empty()).then_some(wire_images);
        let message = compose_message_with_attachments(&text, &self.pending_attachments);
        let display = if message.trim().is_empty() {
            let n = self
                .pending_attachments
                .iter()
                .filter(|a| a.is_image())
                .count();
            format!("[{n} image{}]", if n == 1 { "" } else { "s" })
        } else {
            message.clone()
        };
        self.projection.push_user_message(display);
        self.close_slash_menu();
        self.at_mention_open = false;
        self.large_paste_pending = None;
        // Prefer an explicit empty pending value so the next paint always
        // clears even if a later flag race skips `clear_composer`.
        self.pending_composer_value = Some(String::new());
        self.clear_composer = true;
        self.composer_rows = COMPOSER_GROW_MIN_ROWS;
        self.composer_empty = true;
        self.pending_attachments.clear();
        self.refocus_composer = true;
        cx.notify();

        let message = if message.trim().is_empty() {
            "(image)".to_owned()
        } else {
            message
        };
        cx.spawn(async move |view, cx| {
            let body = if steer {
                RpcCommandBody::Steer { message, images }
            } else {
                RpcCommandBody::Prompt {
                    message,
                    images,
                    streaming_behavior: None,
                }
            };
            match client.send(body).await {
                Ok(resp) if resp.success => {}
                Ok(resp) => {
                    let error = resp.error.unwrap_or_else(|| "prompt failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("prompt".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("failed to send prompt (images?): {error}"),
                            code: Some("prompt".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn filtered_slash_commands(&self, text: &str) -> Vec<SlashSuggestion> {
        let commands = parse_slash_commands(self.projection.available_commands_raw.as_ref());
        filter_slash_commands(&commands, text)
    }

    pub(crate) fn update_slash_menu_from(&mut self, text: &str) {
        if self.slash_menu == SlashMenuState::Dismissed {
            return;
        }
        if !slash_menu_should_scan(self.slash_menu, text) {
            return;
        }
        let commands = parse_slash_commands(self.projection.available_commands_raw.as_ref());
        if !slash_draft_is_open(&commands, text) {
            self.close_slash_menu();
            return;
        }

        self.slash_menu = SlashMenuState::Open;
        let match_count = filter_slash_commands(&commands, text).len();
        self.slash_selected = self.slash_selected.min(match_count.saturating_sub(1));
    }

    pub(crate) fn close_slash_menu(&mut self) {
        self.slash_menu = SlashMenuState::Closed;
        self.slash_selected = 0;
    }

    pub(crate) fn accept_slash_command(
        &mut self,
        suggestion: &SlashSuggestion,
        cx: &mut Context<Self>,
    ) {
        self.pending_composer_value = Some(slash_completion_text(suggestion));
        self.refocus_composer = true;
        self.slash_selected = 0;
        self.slash_menu = if suggestion.is_subcommand {
            SlashMenuState::Dismissed
        } else if suggestion.expects_input {
            SlashMenuState::Open
        } else {
            SlashMenuState::Dismissed
        };
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
        let commands = parse_slash_commands(self.projection.available_commands_raw.as_ref());
        if !slash_draft_is_open(&commands, &text) {
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
        self.client.is_some()
            && phase_allows_send(&self.projection.run_phase)
            && self.projection.pending_dialogs.is_empty()
            && !self.host_bridge.has_pending_requests()
    }

    /// Honest reason when Send/Steer is disabled (Doctrine: disabled-with-reason).
    pub(crate) fn send_disabled_reason(&self) -> Option<&'static str> {
        if self.can_send() {
            return None;
        }
        if !self.projection.pending_dialogs.is_empty() {
            return Some("Answer the dialog above first");
        }
        if self.host_bridge.has_pending_requests() {
            return Some("Resolve the host request above");
        }
        if self.client.is_none() {
            return Some("Not connected to omp");
        }
        match self.projection.run_phase {
            RunPhase::Dead => Some("Session dead — Restart from the crash card"),
            RunPhase::Restarting => Some("Restarting omp…"),
            _ => Some("Send is unavailable"),
        }
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
        if let Some(supervisor) = &self.supervisor
            && matches!(supervisor.state(), SupervisorState::Dead { .. })
        {
            let _ = supervisor.reset();
        }
        let resume = self.restart_resume_path();
        let cwd = self.restart_cwd(resume.as_deref());
        self.begin_connection(window, cwd, resume, false, cx);
    }

    /// Palette/About visibility: offer Update whenever a check found a newer
    /// build. Install itself shuts the child down first, so mid-run is OK.
    pub(crate) fn omp_update_is_offered(&self) -> bool {
        matches!(self.omp_update_ui, OmpUpdateUi::Available { .. })
    }

    pub(crate) fn omp_update_disabled_reason(&self) -> Option<&'static str> {
        if !self.omp_update_is_offered() {
            return Some("No OMP update is available");
        }
        if self.omp_bin.is_none() {
            return Some("The discovered OMP binary is no longer available");
        }
        None
    }

    pub(crate) fn can_start_omp_update(&self) -> bool {
        self.omp_update_disabled_reason().is_none()
    }

    pub(crate) fn start_omp_update(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.can_start_omp_update() {
            return;
        }
        let OmpUpdateUi::Available { current, latest } = self.omp_update_ui.clone() else {
            return;
        };
        let (Some(program), env) = (self.omp_bin.clone(), self.omp_env.clone()) else {
            self.omp_update_ui = OmpUpdateUi::Failed {
                detail: "The discovered OMP binary is no longer available.".to_owned(),
            };
            cx.notify();
            return;
        };
        let resume = self.restart_resume_path();
        let cwd = self.restart_cwd(resume.as_deref());
        let client = self.client.take();

        self.omp_update_generation = self.omp_update_generation.wrapping_add(1);
        let generation = self.omp_update_generation;
        self.omp_update_ui = OmpUpdateUi::Updating { current, latest };
        self.pending_omp_update_failure = None;
        self.clear_abort_arm();
        self.pump.take();
        self.host_bridge.reset();
        self.running_tool_started.clear();
        self.clear_live_timers();
        self.dialog_timeout_gens.clear();
        self.dialog_input_bound_id = None;
        self.pending_custom_answer = None;
        self.dialog_input_empty = true;
        self.palette_open = false;
        self.about_open = false;
        self.projection.mark_restarting();
        "Updating OMP…".clone_into(&mut self.status_message);
        cx.notify();

        cx.spawn_in(window, async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some(client) = client {
                        let _ = gracefully_shutdown_rpc_client(client).await;
                    }
                    install_omp_update(&program, &env, &SystemRunner)
                })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                if this.omp_update_generation != generation {
                    return;
                }
                match result {
                    OmpUpdateInstall::Updated { .. } => {
                        this.begin_connection(window, cwd, resume, false, cx);
                    }
                    OmpUpdateInstall::Failed { detail, .. } => {
                        this.begin_connection(window, cwd, resume, false, cx);
                        this.pending_omp_update_failure = Some(detail);
                    }
                }
            });
        })
        .detach();
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
        window: &Window,
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
            self.do_abort(window, cx);
        } else {
            self.arm_abort(cx);
        }
        true
    }

    pub(crate) fn do_abort(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.clear_abort_arm();
        // Cursor-hosted bash is started with no AbortSignal, so OMP's
        // `abort`/`abort_bash` often never emit `tool_execution_end`. Stop the
        // spinner on the user action; a later end frame can still overwrite.
        self.projection.settle_orphaned_running_tools();
        self.sync_running_tools(cx);
        self.sync_transcript_list();
        cx.notify();
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn_in(window, async move |view, cx| {
            let _ = client
                .send_with_timeout(RpcCommandBody::AbortBash, ABORT_BASH_TIMEOUT)
                .await;
            let abort = client
                .send_with_timeout(RpcCommandBody::Abort, ABORT_RPC_TIMEOUT)
                .await;
            let abort_ok = matches!(abort, Ok(resp) if resp.success);
            if abort_ok {
                return;
            }
            let _ = view.update_in(cx, |this, window, cx| {
                if !this.can_abort() {
                    return;
                }
                // Turn abort did not ACK — rpc-ui is blocked on waitForIdle
                // behind a tool that ignores abort. Recycle the child so the
                // serial queue (Steer/prompt/get_state) can move again.
                let resume = this.restart_resume_path();
                let cwd = this.restart_cwd(resume.as_deref());
                this.begin_connection(window, cwd, resume, false, cx);
            });
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

    pub(crate) fn handle_dialog_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
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

        let id = dialog.id.clone();
        match action {
            DialogKeyAction::Confirm => {
                let mut fields = serde_json::Map::new();
                fields.insert("confirmed".into(), serde_json::Value::Bool(true));
                self.submit_dialog_response(&id, fields, cx);
            }
            DialogKeyAction::Deny => {
                let mut fields = serde_json::Map::new();
                fields.insert("confirmed".into(), serde_json::Value::Bool(false));
                self.submit_dialog_response(&id, fields, cx);
            }
            DialogKeyAction::Cancel => {
                self.submit_dialog_response(&id, dialog_cancel_fields(false), cx);
            }
            DialogKeyAction::Select(idx) => self.mark_dialog_selection(idx, cx),
            DialogKeyAction::SubmitSelect => {
                if self.current_select_is_custom() {
                    return false;
                }
                self.confirm_dialog_selection(cx);
            }
        }
        true
    }

    pub(crate) fn can_follow_up(&self) -> bool {
        self.client.is_some()
            && matches!(self.projection.run_phase, RunPhase::Streaming)
            && self.projection.pending_dialogs.is_empty()
            && !self.host_bridge.has_pending_requests()
            && (!self.composer_empty || !self.pending_attachments.is_empty())
    }

    pub(crate) fn do_follow_up(&mut self, cx: &mut Context<Self>) {
        let text = self.composer_draft(cx);
        if text.trim().is_empty() && self.pending_attachments.is_empty() {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };

        let wire_images = pending_images_to_wire(&self.pending_attachments);
        let images = (!wire_images.is_empty()).then_some(wire_images);
        let message = compose_message_with_attachments(&text, &self.pending_attachments);
        let display = if message.trim().is_empty() {
            let n = self
                .pending_attachments
                .iter()
                .filter(|a| a.is_image())
                .count();
            format!("[{n} image{}]", if n == 1 { "" } else { "s" })
        } else {
            message.clone()
        };
        self.projection.push_user_message(display);
        let message = if message.trim().is_empty() {
            "(image)".to_owned()
        } else {
            message
        };
        self.pending_composer_value = Some(String::new());
        self.clear_composer = true;
        self.composer_rows = COMPOSER_GROW_MIN_ROWS;
        self.composer_empty = true;
        self.pending_attachments.clear();
        self.at_mention_open = false;
        self.large_paste_pending = None;
        self.refocus_composer = true;
        cx.notify();

        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::FollowUp { message, images })
                .await
            {
                Ok(resp) if resp.success => {}
                Ok(resp) => {
                    let error = resp.error.unwrap_or_else(|| "follow_up failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("follow_up".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("failed to send follow_up (images?): {error}"),
                            code: Some("follow_up".into()),
                        });
                        cx.notify();
                    });
                }
            }
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
        let filter_query = self.recent_filter.read(cx).value().to_string();
        let filter_active = !filter_query.trim().is_empty();
        let recents = filter_launcher_recents(&self.recent_sessions, &filter_query);
        let has_recents = !self.recent_sessions.is_empty();
        let last_resume = self.last_session.clone().filter(|resume| {
            !self
                .recent_sessions
                .iter()
                .any(|recent| recent.session_file == *resume)
        });
        let connecting = self.launcher_phase == LauncherPhase::Connecting;

        v_flex()
            .size_full()
            .overflow_y_scrollbar()
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
                    .flex_shrink_0()
                    .px_6()
                    .py_6()
                    .gap_4()
                    .rounded_md()
                    .bg(theme.secondary)
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
                                    .child(soft_wrap_dynamic_text(&cwd)),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("start-working-directory")
                                    .label("Start here")
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
                                .text_color(theme.danger_foreground)
                                .text_xs()
                                .child(soft_wrap_dynamic_text(&error))
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
                    .when(has_recents, |parent| {
                        parent.child(
                            v_flex()
                                .w_full()
                                .flex_shrink_0()
                                .gap_2()
                                .child(Separator::horizontal().label("Recent"))
                                .child(
                                    Input::new(&self.recent_filter)
                                        .appearance(true)
                                        .focus_bordered(true)
                                        .cleanable(true),
                                )
                                .child(
                                    // Fixed max height (no min_h_0) so overflow_y_scrollbar
                                    // gets a real viewport — matching the theme picker pattern.
                                    v_flex()
                                        .id("launcher-recent-list")
                                        .w_full()
                                        .max_h(px(240.))
                                        .overflow_y_scrollbar()
                                        .gap_1()
                                        .when(recents.is_empty(), |list| {
                                            list.child(
                                                Label::new(if filter_active {
                                                    "No matching sessions"
                                                } else {
                                                    "No recent sessions"
                                                })
                                                .text_xs()
                                                .text_color(theme.muted_foreground),
                                            )
                                        })
                                        .children(recents.into_iter().enumerate().map(
                                            |(ix, recent)| {
                                                let cwd = recent.cwd.clone();
                                                let resume = recent.session_file.clone();
                                                let title = truncate_for_display(
                                                    &if recent.name.trim().is_empty() {
                                                        workspace_display_name(&recent.cwd)
                                                    } else {
                                                        recent.name.clone()
                                                    },
                                                    72,
                                                );
                                                let path = truncate_for_display(
                                                    &recent.cwd.display().to_string(),
                                                    64,
                                                );
                                                // Avoid Button here: its fixed h_8 clips/overlaps
                                                // multi-line soft-wrapped labels.
                                                v_flex()
                                                    .id(("recent-session", ix))
                                                    .w_full()
                                                    .min_w_0()
                                                    .gap_0p5()
                                                    .px_2()
                                                    .py_2()
                                                    .rounded_sm()
                                                    .when(!connecting, |row| {
                                                        row.cursor_pointer()
                                                            .hover(|row| {
                                                                row.bg(theme.background)
                                                            })
                                                            .on_click(cx.listener(
                                                                move |this,
                                                                      _: &ClickEvent,
                                                                      window,
                                                                      cx| {
                                                                    this.begin_connection(
                                                                        window,
                                                                        cwd.clone(),
                                                                        Some(resume.clone()),
                                                                        true,
                                                                        cx,
                                                                    );
                                                                },
                                                            ))
                                                    })
                                                    .when(connecting, |row| row.opacity(0.55))
                                                    .child(
                                                        div()
                                                            .w_full()
                                                            .min_w_0()
                                                            .text_sm()
                                                            .child(soft_wrap_dynamic_text(
                                                                &title,
                                                            )),
                                                    )
                                                    .child(
                                                        Label::new(soft_wrap_dynamic_text(&path))
                                                            .text_xs()
                                                            .text_color(theme.muted_foreground),
                                                    )
                                            },
                                        )),
                                ),
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

    pub(crate) fn rename_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.client.is_none() {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "Rename requires a live session".into(),
            ));
            cx.notify();
            return;
        }
        let cwd = self
            .session_cwd
            .as_deref()
            .unwrap_or(self.launcher_cwd.as_path());
        let current = projection_session_name(&self.projection, cwd);
        self.palette_open = false;
        self.about_open = false;
        self.branch_picker = None;
        self.login_picker = None;
        self.compact_open = false;
        self.handoff_confirm_open = false;
        // Prefer a SessionView overlay (same path as About/palette). gpui-component
        // `open_dialog` was not surfacing reliably from this daemonized app.
        window.close_all_dialogs(cx);
        self.rename_open = true;
        self.rename_input.update(cx, |input, cx| {
            input.set_value(current, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn close_rename(&mut self, cx: &mut Context<Self>) {
        if self.rename_open {
            self.rename_open = false;
            self.refocus_composer = true;
            cx.notify();
        }
    }

    pub(crate) fn submit_rename(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = name.trim().to_owned();
        if name.is_empty() {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "session name cannot be empty".into(),
            ));
            cx.notify();
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        self.rename_open = false;
        self.refocus_composer = true;
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

    pub(crate) fn confirm_rename(&mut self, cx: &mut Context<Self>) {
        let name = self.rename_input.read(cx).value().to_string();
        self.submit_rename(&name, cx);
    }

    pub(crate) fn on_rename_input_event(
        &mut self,
        _input: gpui::Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.rename_open {
            return;
        }
        if let InputEvent::PressEnter {
            secondary: false,
            shift: false,
        } = event
        {
            self.confirm_rename(cx);
        }
    }

    pub(crate) fn open_compact_dialog(&mut self, cx: &mut Context<Self>) {
        if self.client.is_none() {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "Compact requires a live session".into(),
            ));
            cx.notify();
            return;
        }
        self.palette_open = false;
        self.about_open = false;
        self.rename_open = false;
        self.branch_picker = None;
        self.login_picker = None;
        self.handoff_confirm_open = false;
        self.compact_open = true;
        self.pending_compact_sync = true;
        self.refocus_compact_input = true;
        cx.notify();
    }

    pub(crate) fn close_compact_dialog(&mut self, cx: &mut Context<Self>) {
        if self.compact_open {
            self.compact_open = false;
            self.pending_compact_sync = false;
            self.refocus_composer = true;
            cx.notify();
        }
    }

    pub(crate) fn confirm_compact(&mut self, cx: &mut Context<Self>) {
        let instructions = self.compact_input.read(cx).value().trim().to_owned();
        let custom_instructions = (!instructions.is_empty()).then_some(instructions);
        self.compact_open = false;
        self.refocus_composer = true;
        self.request_compact(custom_instructions, cx);
    }

    pub(crate) fn on_compact_input_event(
        &mut self,
        _input: gpui::Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.compact_open {
            return;
        }
        if let InputEvent::PressEnter {
            secondary: false,
            shift: false,
        } = event
        {
            self.confirm_compact(cx);
        }
    }

    pub(crate) fn open_handoff_confirmation(&mut self, cx: &mut Context<Self>) {
        if self.client.is_none() {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "Handoff requires a live session".into(),
            ));
            cx.notify();
            return;
        }
        self.palette_open = false;
        self.about_open = false;
        self.rename_open = false;
        self.compact_open = false;
        self.branch_picker = None;
        self.login_picker = None;
        self.handoff_confirm_open = true;
        cx.notify();
    }

    pub(crate) fn close_handoff_confirmation(&mut self, cx: &mut Context<Self>) {
        if self.handoff_confirm_open {
            self.handoff_confirm_open = false;
            self.refocus_composer = true;
            cx.notify();
        }
    }

    pub(crate) fn confirm_handoff(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            self.close_handoff_confirmation(cx);
            return;
        };
        self.handoff_confirm_open = false;
        self.refocus_composer = true;
        cx.notify();
        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::Handoff {
                    custom_instructions: None,
                })
                .await
            {
                Ok(resp) if resp.success => {
                    let detail = pretty_rpc_data(resp.data.as_ref());
                    let _ = view.update(cx, |this, cx| {
                        let mut message =
                            "Handoff accepted by OMP. Continue this session in the TUI.".to_owned();
                        if let Some(detail) = detail {
                            message.push('\n');
                            message.push_str(&detail);
                        }
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(message));
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp.error.unwrap_or_else(|| "handoff failed".into());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("handoff".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("handoff: {error}"),
                            code: Some("handoff".into()),
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

    pub(crate) fn take_pending_new_tab_cwd(&mut self) -> Option<PathBuf> {
        self.pending_new_tab_cwd.take()
    }

    pub(crate) fn toggle_palette(&mut self, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        if self.palette_open {
            if let Some(opening_selection) = self.theme_picker_opening_selection.take() {
                apply_theme_selection_without_window(&opening_selection, cx);
            }
            self.theme_picker_open = false;
            self.about_open = false;
            self.rename_open = false;
            self.compact_open = false;
            self.handoff_confirm_open = false;
            self.branch_picker = None;
            self.login_picker = None;
            self.palette_selected = 0;
            self.model_picker_open = false;
            self.thinking_picker_open = false;
            self.slash_menu = SlashMenuState::Closed;
            self.clear_palette_search = true;
            self.refocus_palette_search = true;
            self.refocus_composer = false;
        } else {
            self.clear_palette_search = true;
            self.refocus_composer = true;
        }
        cx.notify();
    }

    pub(crate) fn close_palette(&mut self, cx: &mut Context<Self>) {
        if self.palette_open {
            self.palette_open = false;
            self.palette_selected = 0;
            self.clear_palette_search = true;
            self.refocus_composer = true;
            cx.notify();
        }
    }

    pub(crate) fn open_theme_picker(&mut self, cx: &mut Context<Self>) {
        self.palette_open = false;
        self.about_open = false;
        self.rename_open = false;
        self.compact_open = false;
        self.handoff_confirm_open = false;
        self.branch_picker = None;
        self.login_picker = None;
        self.model_picker_open = false;
        self.thinking_picker_open = false;
        self.theme_picker_open = true;
        let opening_selection = cx.global::<ThemeSelectionState>().0.clone();
        let items = filter_theme_picker_items(&registered_theme_choices(cx), "");
        self.theme_picker_selected = theme_picker_selected_index(&items, &opening_selection);
        self.theme_picker_opening_selection = Some(opening_selection);
        self.clear_theme_search = true;
        self.refocus_theme_search = true;
        self.refocus_composer = false;
        cx.notify();
    }

    pub(crate) fn close_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.theme_picker_open {
            if let Some(opening_selection) = self.theme_picker_opening_selection.take() {
                apply_theme_selection(&opening_selection, window, cx);
            }
            self.theme_picker_open = false;
            self.theme_picker_selected = 0;
            self.clear_theme_search = true;
            self.refocus_composer = true;
            cx.notify();
        }
    }

    fn theme_selection_for_item(&self, item: &ThemePickerItem, cx: &App) -> Option<ThemeSelection> {
        let opening_selection = self.theme_picker_opening_selection.as_ref()?;
        let themes = registered_theme_choices(cx)
            .into_iter()
            .map(|theme| (theme.name, theme.mode))
            .collect::<Vec<_>>();
        Some(theme_selection_for_picker_item(
            opening_selection,
            item,
            &themes,
        ))
    }

    fn preview_theme_picker_item(
        &mut self,
        item: &ThemePickerItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = self.theme_selection_for_item(item, cx) {
            apply_theme_selection(&selection, window, cx);
        }
    }

    fn choose_theme_picker_item(
        &mut self,
        item: &ThemePickerItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = self.theme_selection_for_item(item, cx) {
            self.persistence.save_theme_selection(&selection);
            self.theme_picker_opening_selection = None;
            apply_theme_selection(&selection, window, cx);
        }
        self.theme_picker_open = false;
        self.theme_picker_selected = 0;
        self.clear_theme_search = true;
        self.refocus_composer = true;
        cx.notify();
    }

    pub(crate) fn show_about(&mut self, cx: &mut Context<Self>) {
        self.palette_open = false;
        self.rename_open = false;
        self.compact_open = false;
        self.handoff_confirm_open = false;
        self.branch_picker = None;
        self.login_picker = None;
        self.about_open = true;
        cx.notify();
    }

    pub(crate) fn close_about(&mut self, cx: &mut Context<Self>) {
        if self.about_open {
            self.about_open = false;
            cx.notify();
        }
    }

    pub(crate) fn request_file_revert(&mut self, path: String, cx: &mut Context<Self>) {
        let command = revert_command_for_path(&path);
        self.pending_revert = Some(PendingRevert { path, command });
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
                            .push(TranscriptEntry::CommandOutput(
                                format!("$ {command}\n{detail}").into(),
                            ));
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
            PaletteActionId::UpdateOmp => self.start_omp_update(window, cx),
            PaletteActionId::ToggleTheme => {
                self.open_theme_picker(cx);
            }
            PaletteActionId::ToggleModels => self.toggle_model_picker(cx),
            PaletteActionId::ToggleThinking => self.toggle_thinking_picker(cx),
            PaletteActionId::ToggleFast => self.toggle_fast_mode(cx),
            PaletteActionId::ExportHtml => self.export_html(cx),
            PaletteActionId::ShareSession => self.share_session(cx),
            PaletteActionId::RenameSession => self.rename_session(window, cx),
            PaletteActionId::AbortRun => self.do_abort(window, cx),
            PaletteActionId::SessionsLauncher => self.return_to_launcher(cx),
            PaletteActionId::RevealLogs => {
                if reveal_in_file_manager(&self.persistence.root).is_err() {
                    "Could not reveal the Pimiento home folder"
                        .clone_into(&mut self.status_message);
                    cx.notify();
                }
            }
            PaletteActionId::CycleModel => self.cycle_model(cx),
            PaletteActionId::CycleThinking => self.cycle_thinking(cx),
            PaletteActionId::Compact => self.open_compact_dialog(cx),
            PaletteActionId::SessionStats => self.fetch_session_stats(cx),
            PaletteActionId::FreshSession => self.request_fresh_session(cx),
            PaletteActionId::Handoff => self.open_handoff_confirmation(cx),
            PaletteActionId::AbortRetry => self.request_abort_retry(cx),
            PaletteActionId::AbortAndPrompt => self.request_abort_and_prompt(cx),
            PaletteActionId::BranchSession => self.open_branch_picker(cx),
            PaletteActionId::LoginProviders => self.open_login_picker(cx),
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

    fn run_command_palette_entry(
        &mut self,
        entry: CommandPaletteEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match entry {
            CommandPaletteEntry::Action(entry) => {
                self.run_palette_action(entry.id, window, cx);
            }
            CommandPaletteEntry::NativeSlash(entry) => {
                self.run_palette_action(entry.action, window, cx);
            }
            CommandPaletteEntry::Slash { suggestion, .. } => {
                self.close_palette(cx);
                self.accept_slash_command(&suggestion, cx);
            }
        }
    }

    pub(crate) fn request_abort_and_prompt(&mut self, cx: &mut Context<Self>) {
        let text = self.composer_draft(cx);
        if text.trim().is_empty() && self.pending_attachments.is_empty() {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "abort_and_prompt needs composer text or attachments".into(),
            ));
            self.refocus_composer = true;
            cx.notify();
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        let wire_images = pending_images_to_wire(&self.pending_attachments);
        let images = (!wire_images.is_empty()).then_some(wire_images);
        let message = compose_message_with_attachments(&text, &self.pending_attachments);
        let display = if message.trim().is_empty() {
            let n = self
                .pending_attachments
                .iter()
                .filter(|a| a.is_image())
                .count();
            format!("[{n} image{}]", if n == 1 { "" } else { "s" })
        } else {
            message.clone()
        };
        self.projection.push_user_message(display);
        self.close_slash_menu();
        self.at_mention_open = false;
        self.large_paste_pending = None;
        self.pending_composer_value = Some(String::new());
        self.clear_composer = true;
        self.composer_rows = COMPOSER_GROW_MIN_ROWS;
        self.composer_empty = true;
        self.pending_attachments.clear();
        self.refocus_composer = true;
        self.clear_abort_arm();
        cx.notify();

        let message = if message.trim().is_empty() {
            "(image)".to_owned()
        } else {
            message
        };
        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::AbortAndPrompt { message, images })
                .await
            {
                Ok(resp) if resp.success => {}
                Ok(resp) => {
                    let error = resp
                        .error
                        .unwrap_or_else(|| "abort_and_prompt failed".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("abort_and_prompt".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("abort_and_prompt: {error}"),
                            code: Some("abort_and_prompt".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn cycle_model(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        // Prefer OMP's cycle_model; fall back to local catalog walk if the
        // response is hard to parse or the command fails.
        cx.spawn(
            async move |view, cx| match client.send(RpcCommandBody::CycleModel).await {
                Ok(resp) if resp.success => {
                    let label = resp
                        .data
                        .as_ref()
                        .and_then(|data| data.get("model").or(Some(data)))
                        .and_then(format_model_label);
                    let thinking = resp
                        .data
                        .as_ref()
                        .and_then(|data| data.get("thinkingLevel"))
                        .cloned();
                    let _ = view.update(cx, |this, cx| {
                        if let Some(label) = label {
                            this.projection.state.model = Some(label);
                        }
                        if let Some(thinking) = thinking {
                            this.projection.state.thinking = Some(thinking);
                        }
                        this.sync_status_model();
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Ok(_) | Err(_) => {
                    let _ = view.update(cx, |this, cx| {
                        this.cycle_model_local(cx);
                    });
                }
            },
        )
        .detach();
    }

    pub(crate) fn cycle_model_local(&mut self, cx: &mut Context<Self>) {
        if self.available_models.is_empty() {
            self.start_catalog_load(cx);
            return;
        }
        let current = self.projection.state.model.clone();
        let idx = self
            .available_models
            .iter()
            .position(|m| {
                current
                    .as_deref()
                    .is_some_and(|cur| format!("{}/{}", m.provider, m.id) == cur)
            })
            .unwrap_or(0);
        let next = &self.available_models[(idx + 1) % self.available_models.len()];
        self.set_model(next.provider.clone(), next.id.clone(), cx);
    }

    pub(crate) fn cycle_thinking(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn(async move |view, cx| {
            match client.send(RpcCommandBody::CycleThinkingLevel).await {
                Ok(resp) if resp.success => {
                    let level = resp
                        .data
                        .as_ref()
                        .and_then(|data| data.get("level"))
                        .cloned();
                    let _ = view.update(cx, |this, cx| {
                        if let Some(level) = level {
                            this.projection.state.thinking = Some(level);
                            this.sync_status_model();
                        }
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Ok(_) | Err(_) => {
                    let _ = view.update(cx, |this, cx| {
                        this.cycle_thinking_local(cx);
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn cycle_thinking_local(&mut self, cx: &mut Context<Self>) {
        let options = thinking_options_for_model(find_model_choice(
            &self.available_models,
            self.projection.state.model.as_deref(),
        ));
        if options.is_empty() {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "current model has no controllable thinking levels".into(),
            ));
            cx.notify();
            return;
        }
        let current =
            thinking_label(self.projection.state.thinking.as_ref()).unwrap_or_else(|| "off".into());
        let idx = options.iter().position(|o| o == &current).unwrap_or(0);
        let next = options[(idx + 1) % options.len()].clone();
        self.set_thinking_level(&next, cx);
    }

    pub(crate) fn request_compact(
        &mut self,
        custom_instructions: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::Compact {
                    custom_instructions,
                })
                .await
            {
                Ok(resp) if resp.success => {}
                Ok(resp) => {
                    let error = resp.error.unwrap_or_else(|| "compact failed".into());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("compact".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("compact: {error}"),
                            code: Some("compact".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn fetch_session_stats(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "Session stats require a live session".into(),
            ));
            cx.notify();
            return;
        };
        cx.spawn(
            async move |view, cx| match client.send(RpcCommandBody::GetSessionStats).await {
                Ok(resp) if resp.success => {
                    let detail = pretty_rpc_data(resp.data.as_ref())
                        .unwrap_or_else(|| "(no stats payload returned)".to_owned());
                    let _ = view.update(cx, |this, cx| {
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(format!("Session stats\n{detail}")));
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .unwrap_or_else(|| "get_session_stats failed".into());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("get_session_stats".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("get_session_stats: {error}"),
                            code: Some("get_session_stats".into()),
                        });
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    pub(crate) fn request_fresh_session(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.projection.run_phase, RunPhase::Idle)
            || !self.projection.pending_dialogs.is_empty()
        {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "Fresh session is available when the current session is idle".into(),
            ));
            cx.notify();
            return;
        }
        let Some(client) = self.client.clone() else {
            self.projection.transcript.push(TranscriptEntry::Notice(
                "Fresh session requires a live session".into(),
            ));
            cx.notify();
            return;
        };
        self.projection.push_user_message("/fresh".into());
        cx.notify();
        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::Prompt {
                    message: "/fresh".into(),
                    images: None,
                    streaming_behavior: None,
                })
                .await
            {
                Ok(resp) if resp.success => {}
                Ok(resp) => {
                    let error = resp.error.unwrap_or_else(|| "/fresh failed".into());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("fresh".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("/fresh: {error}"),
                            code: Some("fresh".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn request_abort_retry(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn(
            async move |view, cx| match client.send(RpcCommandBody::AbortRetry).await {
                Ok(resp) if resp.success => {}
                Ok(resp) => {
                    let error = resp.error.unwrap_or_else(|| "abort_retry failed".into());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("abort_retry".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("abort_retry: {error}"),
                            code: Some("abort_retry".into()),
                        });
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    pub(crate) fn open_branch_picker(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.rename_open = false;
        self.login_picker = None;
        self.about_open = false;
        cx.spawn(async move |view, cx| {
            match client.send(RpcCommandBody::GetBranchMessages).await {
                Ok(resp) if resp.success => {
                    let messages = parse_branch_messages(resp.data.as_ref());
                    let _ = view.update(cx, |this, cx| {
                        if messages.is_empty() {
                            this.projection.transcript.push(TranscriptEntry::Notice(
                                "no branchable user messages yet".into(),
                            ));
                        } else {
                            this.branch_picker_selected = 0;
                            this.branch_picker = Some(messages);
                        }
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .unwrap_or_else(|| "get_branch_messages failed".into());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("get_branch_messages".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("get_branch_messages: {error}"),
                            code: Some("get_branch_messages".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn close_branch_picker(&mut self, cx: &mut Context<Self>) {
        if self.branch_picker.take().is_some() {
            self.branch_picker_selected = 0;
            self.refocus_composer = true;
            cx.notify();
        }
    }

    pub(crate) fn confirm_branch_pick(&mut self, entry_id: String, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let cwd = self
            .session_cwd
            .clone()
            .unwrap_or_else(|| self.launcher_cwd.clone());
        self.branch_picker = None;
        self.branch_picker_selected = 0;
        cx.notify();
        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::Branch {
                    entry_id: entry_id.clone(),
                })
                .await
            {
                Ok(resp) if resp.success => {
                    let cancelled = resp
                        .data
                        .as_ref()
                        .and_then(|d| d.get("cancelled"))
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let _ = view.update(cx, |this, cx| {
                        if cancelled {
                            this.projection
                                .transcript
                                .push(TranscriptEntry::Notice("branch cancelled".into()));
                        } else {
                            this.projection
                                .transcript
                                .push(TranscriptEntry::Notice(format!("branched from {entry_id}")));
                            // Current connection is now the branch; open a parallel
                            // tab for the same cwd (fresh connect — not switch_session).
                            this.pending_new_tab_cwd = Some(cwd);
                            this.refresh_state(cx);
                        }
                        this.refocus_composer = true;
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp.error.unwrap_or_else(|| "branch failed".into());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("branch".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("branch: {error}"),
                            code: Some("branch".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn open_login_picker(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.rename_open = false;
        self.branch_picker = None;
        self.about_open = false;
        cx.spawn(async move |view, cx| {
            match client.send(RpcCommandBody::GetLoginProviders).await {
                Ok(resp) if resp.success => {
                    let providers = parse_login_providers(resp.data.as_ref());
                    let _ = view.update(cx, |this, cx| {
                        if providers.is_empty() {
                            this.projection.transcript.push(TranscriptEntry::Notice(
                                "no login providers reported".into(),
                            ));
                        } else {
                            this.login_picker_selected = 0;
                            this.login_picker = Some(providers);
                        }
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp
                        .error
                        .unwrap_or_else(|| "get_login_providers failed".into());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("get_login_providers".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("get_login_providers: {error}"),
                            code: Some("get_login_providers".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn close_login_picker(&mut self, cx: &mut Context<Self>) {
        if self.login_picker.take().is_some() {
            self.login_picker_selected = 0;
            self.refocus_composer = true;
            cx.notify();
        }
    }

    pub(crate) fn confirm_login_provider(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.login_picker = None;
        self.login_picker_selected = 0;
        cx.notify();
        cx.spawn(async move |view, cx| {
            match client
                .send(RpcCommandBody::Login {
                    provider_id: provider_id.clone(),
                })
                .await
            {
                Ok(resp) if resp.success => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection
                            .transcript
                            .push(TranscriptEntry::Notice(format!(
                                "login started for {provider_id}"
                            )));
                        this.refocus_composer = true;
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp.error.unwrap_or_else(|| "login failed".into());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("login".into()),
                        });
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("login: {error}"),
                            code: Some("login".into()),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn toggle_todo_task(
        &mut self,
        phase_ix: usize,
        task_ix: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(raw) = self.projection.todos_raw.clone() else {
            return;
        };
        let Some(phases) = toggle_todo_in_phases_json(&raw, phase_ix, task_ix) else {
            return;
        };
        let Some(client) = self.client.clone() else {
            return;
        };
        // Optimistic local mirror; OMP `set_todos` response / get_state win.
        self.projection.todos_raw = Some(phases.clone());
        cx.notify();
        cx.spawn(async move |view, cx| {
            match client.send(RpcCommandBody::SetTodos { phases }).await {
                Ok(resp) if resp.success => {
                    let _ = view.update(cx, |this, cx| {
                        if let Some(data) = resp.data.as_ref() {
                            if let Some(todos) = data
                                .get("todoPhases")
                                .or_else(|| data.get("phases"))
                                .cloned()
                            {
                                this.projection.todos_raw = Some(todos);
                            } else if data.is_array() {
                                this.projection.todos_raw = Some(data.clone());
                            }
                        }
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Ok(resp) => {
                    let error = resp.error.unwrap_or_else(|| "set_todos failed".into());
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: error,
                            code: Some("set_todos".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update(cx, |this, cx| {
                        this.projection.transcript.push(TranscriptEntry::Error {
                            message: format!("set_todos: {error}"),
                            code: Some("set_todos".into()),
                        });
                        this.refresh_state(cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn handle_theme_picker_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.theme_picker_open {
            return false;
        }
        let query = self.theme_search.read(cx).value().to_string();
        let themes = registered_theme_choices(cx);
        let items = filter_theme_picker_items(&themes, &query);
        if items.is_empty() {
            self.theme_picker_selected = 0;
        } else {
            self.theme_picker_selected = self.theme_picker_selected.min(items.len() - 1);
        }
        match event.keystroke.key.to_ascii_lowercase().as_str() {
            "escape" | "esc" => {
                self.close_theme_picker(window, cx);
                true
            }
            "up" | "arrowup" => {
                if !items.is_empty() {
                    self.theme_picker_selected =
                        (self.theme_picker_selected + items.len() - 1) % items.len();
                    self.preview_theme_picker_item(&items[self.theme_picker_selected], window, cx);
                    cx.notify();
                }
                true
            }
            "down" | "arrowdown" => {
                if !items.is_empty() {
                    self.theme_picker_selected = (self.theme_picker_selected + 1) % items.len();
                    self.preview_theme_picker_item(&items[self.theme_picker_selected], window, cx);
                    cx.notify();
                }
                true
            }
            "enter" | "return" => {
                if let Some(item) = items.get(self.theme_picker_selected) {
                    self.choose_theme_picker_item(item, window, cx);
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn on_theme_search_event(
        &mut self,
        _input: gpui::Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if self.theme_picker_open && matches!(event, InputEvent::Change) {
            self.theme_picker_selected = 0;
            let query = self.theme_search.read(cx).value().to_string();
            let items = filter_theme_picker_items(&registered_theme_choices(cx), &query);
            if let Some(item) = items.first()
                && let Some(selection) = self.theme_selection_for_item(item, cx)
            {
                apply_theme_selection_without_window(&selection, cx);
            }
            cx.notify();
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
        let key = event.keystroke.key.to_ascii_lowercase();
        let key = key.as_str();
        let query = self.palette_search.read(cx).value().to_string();
        let commands = parse_slash_commands(self.projection.available_commands_raw.as_ref());
        let matches =
            filter_command_palette_entries(&commands, &query, self.omp_update_is_offered());
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
                    let entry = entry.clone();
                    self.run_command_palette_entry(entry, window, cx);
                }
                true
            }
            // Let printable keys reach the focused palette Input.
            _ => false,
        }
    }

    pub(crate) fn on_palette_search_event(
        &mut self,
        _input: gpui::Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.palette_open {
            return;
        }
        match event {
            InputEvent::Change => {
                self.palette_selected = 0;
                cx.notify();
            }
            InputEvent::PressEnter {
                secondary: false,
                shift: false,
            } => {
                self.pending_palette_enter = true;
                cx.notify();
            }
            _ => {}
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

    pub(crate) fn handle_rename_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.rename_open {
            return false;
        }
        if !event.keystroke.modifiers.modified()
            && matches!(event.keystroke.key.as_str(), "escape" | "esc")
        {
            self.close_rename(cx);
            return true;
        }
        // Let printable keys / Enter reach the rename Input.
        false
    }

    pub(crate) fn handle_compact_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.compact_open {
            return false;
        }
        if !event.keystroke.modifiers.modified()
            && matches!(event.keystroke.key.as_str(), "escape" | "esc")
        {
            self.close_compact_dialog(cx);
            return true;
        }
        // Let printable keys and Enter reach the focused instructions Input.
        false
    }

    pub(crate) fn handle_handoff_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.handoff_confirm_open {
            return false;
        }
        if event.keystroke.modifiers.modified() {
            return true;
        }
        match event.keystroke.key.to_ascii_lowercase().as_str() {
            "enter" | "return" | "y" => self.confirm_handoff(cx),
            "escape" | "esc" | "n" => self.close_handoff_confirmation(cx),
            _ => {}
        }
        true
    }

    pub(crate) fn handle_branch_picker_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(len) = self.branch_picker.as_ref().map(Vec::len) else {
            return false;
        };
        if len == 0 {
            return false;
        }
        let key = event.keystroke.key.to_ascii_lowercase();
        match key.as_str() {
            "escape" | "esc" => {
                self.close_branch_picker(cx);
                true
            }
            "up" | "arrowup" => {
                self.branch_picker_selected = (self.branch_picker_selected + len - 1) % len;
                cx.notify();
                true
            }
            "down" | "arrowdown" => {
                self.branch_picker_selected = (self.branch_picker_selected + 1) % len;
                cx.notify();
                true
            }
            "enter" | "return" => {
                let entry_id = self
                    .branch_picker
                    .as_ref()
                    .and_then(|m| m.get(self.branch_picker_selected))
                    .map(|c| c.entry_id.clone());
                if let Some(entry_id) = entry_id {
                    self.confirm_branch_pick(entry_id, cx);
                }
                true
            }
            _ => true,
        }
    }

    pub(crate) fn handle_login_picker_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(len) = self.login_picker.as_ref().map(Vec::len) else {
            return false;
        };
        if len == 0 {
            return false;
        }
        let key = event.keystroke.key.to_ascii_lowercase();
        match key.as_str() {
            "escape" | "esc" => {
                self.close_login_picker(cx);
                true
            }
            "up" | "arrowup" => {
                self.login_picker_selected = (self.login_picker_selected + len - 1) % len;
                cx.notify();
                true
            }
            "down" | "arrowdown" => {
                self.login_picker_selected = (self.login_picker_selected + 1) % len;
                cx.notify();
                true
            }
            "enter" | "return" => {
                let choice = self
                    .login_picker
                    .as_ref()
                    .and_then(|p| p.get(self.login_picker_selected))
                    .cloned();
                if let Some(choice) = choice
                    && choice.available
                {
                    self.confirm_login_provider(choice.id, cx);
                }
                true
            }
            _ => true,
        }
    }

    pub(crate) fn rail_entry(&self, ix: usize) -> RailEntry {
        let cwd = self
            .session_cwd
            .as_deref()
            .unwrap_or(self.launcher_cwd.as_path());
        let label = projection_session_name(&self.projection, cwd);
        RailEntry {
            ix,
            label,
            phase: self.projection.run_phase.clone(),
            cwd: cwd.to_owned(),
            attention: self.rail_attention(),
            session_file: self
                .projection
                .state
                .session_file
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        }
    }

    pub(crate) fn rail_attention(&self) -> RailAttention {
        classify_rail_attention(&self.projection.run_phase, self.unread_below)
    }

    /// Fields the workspace rail / window title actually paint. Used so
    /// transcript streaming does not rebuild the whole workspace tree.
    pub(crate) fn workspace_chrome_sig(&self) -> SessionChromeSig {
        let cwd = self
            .session_cwd
            .as_deref()
            .unwrap_or(self.launcher_cwd.as_path());
        SessionChromeSig {
            label: projection_session_name(&self.projection, cwd),
            phase: self.projection.run_phase.clone(),
            unread: self.unread_below,
            attention: self.rail_attention(),
            launcher: self.launcher_phase,
        }
    }

    pub(crate) fn window_title(&self) -> String {
        if self.launcher_phase != LauncherPhase::Hidden {
            return "Pimiento".to_owned();
        }
        let cwd = self
            .session_cwd
            .as_deref()
            .unwrap_or(self.launcher_cwd.as_path());
        let base = workspace_window_title(
            &projection_session_name(&self.projection, cwd),
            &self.projection.run_phase,
        );
        // OMP only emits setTitle when PI_RPC_EMIT_TITLE is set; mirror that
        // for the OS window title, while inspector always shows display.title.
        if emit_rpc_titles()
            && let Some(title) = self
                .projection
                .display
                .title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
        {
            format!("{base} — {title}")
        } else {
            base
        }
    }

    pub(crate) fn shutdown_session(&mut self, cx: &mut Context<Self>) {
        self.invalidate_omp_update();
        self.clear_abort_arm();
        self.mark_explicit_supervisor_shutdown();
        self.take_and_close_client(cx);
        self.host_bridge.reset();
        self.running_tool_started.clear();
        self.clear_live_timers();
        self.dialog_timeout_gens.clear();
        self.dialog_input_bound_id = None;
        self.pending_custom_answer = None;
        self.dialog_input_empty = true;
        cx.notify();
    }
}

pub(crate) const STDERR_STATUS_CAP: usize = 240;

/// Human-readable label for a supervisor [`DeathReason`].
#[must_use]
pub(crate) fn supervisor_death_reason_message(reason: &DeathReason) -> String {
    match reason {
        DeathReason::CrashLoop => "crash loop (3 restarts in 60s)".to_owned(),
        DeathReason::Explicit => "explicit shutdown".to_owned(),
        DeathReason::Fatal(msg) => msg.clone(),
    }
}

#[must_use]
pub(crate) fn crash_loop_status_message(stderr_tail: &str) -> String {
    let scrubbed = redact_secrets_for_display(stderr_tail);
    let trimmed = scrubbed.trim();
    if trimmed.is_empty() {
        return "OMP crash loop (3 restarts in 60s)".to_owned();
    }
    format!(
        "OMP crash loop (3 restarts in 60s) — {}",
        truncate_for_display(trimmed, STDERR_STATUS_CAP)
    )
}

pub(crate) fn omp_closed_status_message(stderr_tail: &str) -> String {
    let scrubbed = redact_secrets_for_display(stderr_tail);
    let trimmed = scrubbed.trim();
    if trimmed.is_empty() {
        return "OMP closed".to_owned();
    }
    format!(
        "OMP closed — {}",
        truncate_for_display(trimmed, STDERR_STATUS_CAP)
    )
}

fn truncate_for_display(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// Launcher Recent list: unfiltered shows at most `LAUNCHER_RECENT_VISIBLE_CAP`;
/// with a query, return every session whose name/cwd/session file matches.
pub(crate) fn filter_launcher_recents(
    sessions: &[RecentSession],
    query: &str,
) -> Vec<RecentSession> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return sessions
            .iter()
            .take(LAUNCHER_RECENT_VISIBLE_CAP)
            .cloned()
            .collect();
    }
    sessions
        .iter()
        .filter(|session| {
            session.name.to_ascii_lowercase().contains(&needle)
                || session
                    .cwd
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains(&needle)
                || session
                    .session_file
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains(&needle)
        })
        .cloned()
        .collect()
}

/// Scrub common secret patterns from text destined for UI status / logs.
///
/// Case-insensitive matches for `Bearer <token>`, `api[_-]?key[=:]…`,
/// `token[=:]…`, `sk-…`, and long base64-looking runs. Replacement is
/// `[redacted]`.
pub(crate) fn redact_secrets_for_display(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some(end) = match_secret_span(bytes, i) {
            out.push_str("[redacted]");
            i = end;
            continue;
        }
        let ch = s[i..].chars().next().expect("valid utf-8 at index");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn ascii_eq_ignore_case(hay: &[u8], needle: &[u8]) -> bool {
    hay.len() == needle.len()
        && hay
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn is_secret_value_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+' | b'/' | b'=')
}

fn skip_secret_value(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && is_secret_value_byte(bytes[i]) {
        i += 1;
    }
    i
}

/// If `bytes[i..]` starts a secret pattern, return exclusive end index of the
/// span that should be replaced.
fn match_secret_span(bytes: &[u8], i: usize) -> Option<usize> {
    // Bearer <token>
    if i + 6 < bytes.len() && ascii_eq_ignore_case(&bytes[i..i + 6], b"bearer") {
        let mut j = i + 6;
        if j < bytes.len() && bytes[j].is_ascii_whitespace() {
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let value_end = skip_secret_value(bytes, j);
            if value_end > j {
                return Some(value_end);
            }
        }
    }

    // api[_-]?key[=:]<value>
    if i + 6 <= bytes.len() && ascii_eq_ignore_case(&bytes[i..i + 3], b"api") {
        let after_api = i + 3;
        let key_start = match bytes.get(after_api).copied() {
            Some(b'_' | b'-') => after_api + 1,
            _ => after_api,
        };
        if key_start + 3 <= bytes.len()
            && ascii_eq_ignore_case(&bytes[key_start..key_start + 3], b"key")
        {
            let after_key = key_start + 3;
            if let Some(sep) = bytes.get(after_key).copied()
                && matches!(sep, b'=' | b':')
            {
                let mut j = after_key + 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let value_end = skip_secret_value(bytes, j);
                if value_end > j {
                    return Some(value_end);
                }
            }
        }
    }

    // token[=:]<value> — require a non-alnum boundary before `token` so we do
    // not redact inside identifiers like `mytoken`.
    if i + 5 <= bytes.len()
        && ascii_eq_ignore_case(&bytes[i..i + 5], b"token")
        && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
    {
        let after = i + 5;
        if let Some(sep) = bytes.get(after).copied()
            && matches!(sep, b'=' | b':')
        {
            let mut j = after + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let value_end = skip_secret_value(bytes, j);
            if value_end > j {
                return Some(value_end);
            }
        }
    }

    // sk-[A-Za-z0-9_-]+
    if i + 3 < bytes.len()
        && bytes[i].eq_ignore_ascii_case(&b's')
        && bytes[i + 1].eq_ignore_ascii_case(&b'k')
        && bytes[i + 2] == b'-'
    {
        let value_end = skip_secret_value(bytes, i + 3);
        if value_end > i + 3 {
            return Some(value_end);
        }
    }

    // Long base64-looking runs (optional): 40+ alphabet chars that look like
    // encoded material (mixed charset and/or +/=), not a repeated filler.
    if bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'+' | b'/' | b'=') {
        let mut j = i;
        let mut has_plus_slash = false;
        let mut has_letter = false;
        let mut has_digit = false;
        let mut has_upper = false;
        let mut has_lower = false;
        while j < bytes.len() {
            let b = bytes[j];
            if b.is_ascii_uppercase() {
                has_letter = true;
                has_upper = true;
            } else if b.is_ascii_lowercase() {
                has_letter = true;
                has_lower = true;
            } else if b.is_ascii_digit() {
                has_digit = true;
            } else if matches!(b, b'+' | b'/') {
                has_plus_slash = true;
            } else if b == b'=' {
                // padding only allowed at end of the run
            } else {
                break;
            }
            j += 1;
        }
        // Trim trailing `=` padding from the span end check already included.
        let len = j - i;
        let mixed = has_plus_slash
            || (has_letter && has_digit)
            || (has_upper && has_lower && has_digit)
            || (has_upper && has_lower && len >= 48);
        if len >= 40 && mixed {
            return Some(j);
        }
    }

    None
}

pub(crate) fn omp_update_available_label(current: &str, latest: &str) -> String {
    format!("OMP {current} → {latest} is available")
}

fn render_omp_update_banner(
    update: &OmpUpdateUi,
    disabled_reason: Option<&'static str>,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    match update {
        OmpUpdateUi::Available { current, latest } => h_flex()
            .w_full()
            .px_3()
            .py_1()
            .items_start()
            .gap_2()
            .bg(theme.secondary)
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .child(div().flex_1().min_w_0().text_color(theme.warning).child(
                soft_wrap_dynamic_text(&omp_update_available_label(current, latest)),
            ))
            .child(
                Button::new("update-omp-banner")
                    .label("Update OMP")
                    .small()
                    .primary()
                    .tooltip(
                        disabled_reason.unwrap_or("Install the update and resume this session"),
                    )
                    .disabled(disabled_reason.is_some())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.start_omp_update(window, cx);
                    })),
            )
            .into_any_element(),
        OmpUpdateUi::Updating { .. } => div()
            .w_full()
            .px_3()
            .py_1()
            .bg(theme.secondary)
            .border_b_1()
            .border_color(theme.border)
            .text_color(theme.warning)
            .text_xs()
            .child("Updating OMP…")
            .into_any_element(),
        OmpUpdateUi::Failed { detail } => h_flex()
            .w_full()
            .px_3()
            .py_1()
            .items_start()
            .gap_2()
            .bg(theme.danger)
            .text_color(theme.danger_foreground)
            .text_xs()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(soft_wrap_dynamic_text(&format!(
                        "OMP update failed: {detail}"
                    ))),
            )
            .child(
                Button::new("dismiss-omp-update-failure")
                    .label("Dismiss")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.omp_update_ui = OmpUpdateUi::Idle;
                        cx.notify();
                    })),
            )
            .into_any_element(),
        OmpUpdateUi::Idle | OmpUpdateUi::Checking => div().into_any_element(),
    }
}

fn render_omp_update_about(
    update: &OmpUpdateUi,
    disabled_reason: Option<&'static str>,
    cx: &mut Context<SessionView>,
) -> Option<gpui::AnyElement> {
    let theme = cx.theme().clone();
    match update {
        OmpUpdateUi::Available { current, latest } => Some(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .p_3()
                .rounded_md()
                .bg(theme.secondary)
                .border_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(soft_wrap_dynamic_text(&omp_update_available_label(
                            current, latest,
                        ))),
                )
                .child(
                    Button::new("update-omp-about")
                        .label("Update OMP")
                        .small()
                        .primary()
                        .tooltip(
                            disabled_reason.unwrap_or("Install the update and resume this session"),
                        )
                        .disabled(disabled_reason.is_some())
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.start_omp_update(window, cx);
                        })),
                )
                .into_any_element(),
        ),
        OmpUpdateUi::Updating { .. } => Some(
            div()
                .w_full()
                .p_3()
                .rounded_md()
                .bg(theme.secondary)
                .border_1()
                .border_color(theme.border)
                .text_xs()
                .text_color(theme.warning)
                .child("Updating OMP…")
                .into_any_element(),
        ),
        OmpUpdateUi::Idle | OmpUpdateUi::Checking | OmpUpdateUi::Failed { .. } => None,
    }
}

// ── guards ────────────────────────────────────────────────────────────────

pub(crate) fn short_model_label(full: &str) -> String {
    full.strip_prefix("cursor/").unwrap_or(full).to_owned()
}

pub(crate) fn role_color_tag(color: OmpRoleColor) -> Tag {
    match color {
        OmpRoleColor::Success => Tag::success(),
        OmpRoleColor::Warning => Tag::warning(),
        OmpRoleColor::Accent => Tag::info(),
        OmpRoleColor::Error => Tag::danger(),
        OmpRoleColor::Muted | OmpRoleColor::Dim => Tag::secondary(),
    }
}

pub(crate) fn phase_tag(phase: &str) -> Tag {
    status_pill_for_label(phase)
}

pub(crate) fn cycle_queue_mode(current: Option<&str>) -> QueueMode {
    match current {
        Some("one-at-a-time") => QueueMode::All,
        _ => QueueMode::OneAtATime,
    }
}

pub(crate) fn cycle_interrupt_mode(current: Option<&str>) -> InterruptMode {
    match current {
        Some("immediate") => InterruptMode::Wait,
        _ => InterruptMode::Immediate,
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

pub(crate) fn emit_rpc_titles() -> bool {
    match std::env::var("PI_RPC_EMIT_TITLE") {
        Ok(raw) => {
            let normalized = raw.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
}

/// Non-empty `setStatus` lines for chrome (key → text).
pub(crate) fn display_status_lines(display: &DisplayState) -> Vec<(String, String)> {
    display
        .statuses
        .iter()
        .filter_map(|(key, text)| {
            let text = text.as_deref()?.trim();
            (!text.is_empty()).then(|| (key.clone(), text.to_owned()))
        })
        .collect()
}

pub(crate) fn display_widget_lines(raw: &serde_json::Value) -> Vec<String> {
    raw.get("widgetLines")
        .and_then(serde_json::Value::as_array)
        .map(|lines| {
            lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_owned))
                .filter(|line| !line.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogKeyAction {
    Confirm,
    Deny,
    Cancel,
    Select(usize),
    SubmitSelect,
}

pub(crate) fn dialog_key_action(
    key: &str,
    method: &str,
    option_count: usize,
) -> Option<DialogKeyAction> {
    match key {
        "escape" => Some(DialogKeyAction::Cancel),
        "enter" | "return" if method == "select" => Some(DialogKeyAction::SubmitSelect),
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
    dialog_primary_options(dialog)
        .into_iter()
        .map(|option| option.value)
        .collect()
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

pub(crate) fn slash_draft_is_open(commands: &[SlashCommand], text: &str) -> bool {
    slash_completion_context(commands, text).is_some()
}

pub(crate) fn slash_menu_should_scan(menu: SlashMenuState, text: &str) -> bool {
    match menu {
        SlashMenuState::Dismissed => false,
        SlashMenuState::Open => true,
        SlashMenuState::Closed => text.trim_start().starts_with('/'),
    }
}

fn transcript_tail_fingerprint(
    transcript: &[TranscriptEntry],
    expanded_tools: &HashSet<String>,
) -> u64 {
    let start = transcript.len().saturating_sub(4);
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for entry in &transcript[start..] {
        h ^= transcript_measure_key(entry, expanded_tools);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= transcript.len() as u64;
    h
}

fn transcript_measure_key(entry: &TranscriptEntry, expanded_tools: &HashSet<String>) -> u64 {
    match entry {
        TranscriptEntry::User { text } => text.len() as u64,
        TranscriptEntry::AssistantText {
            markdown,
            streaming,
        } => (markdown.0.len() as u64) << 1 | u64::from(*streaming),
        TranscriptEntry::Thinking {
            text,
            streaming,
            collapsed,
        } => (text.len() as u64) << 2 | u64::from(*streaming) << 1 | u64::from(*collapsed),
        TranscriptEntry::ToolCall(tool) => {
            let expanded = u64::from(expanded_tools.contains(&tool.tool_call_id) || tool.expanded);
            let status = match tool.status {
                ToolStatus::Running => 1,
                ToolStatus::Ok => 2,
                ToolStatus::Err => 3,
            };
            (tool.output.len() as u64) << 3 | expanded << 2 | status
        }
        TranscriptEntry::Notice(text) | TranscriptEntry::RetryInfo { detail: text } => {
            text.len() as u64
        }
        TranscriptEntry::Error { message, .. } => message.len() as u64,
        TranscriptEntry::CommandOutput(text) => text.len() as u64,
        TranscriptEntry::Compaction { phase } => match phase {
            CompactionPhase::Started => 1,
            CompactionPhase::Progress => 2,
            CompactionPhase::Completed => 3,
            CompactionPhase::Failed => 4,
        },
        TranscriptEntry::Unknown { .. } => 7,
    }
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
            input_hint: None,
            subcommands: Vec::new(),
            source: None,
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
    let input_hint = raw
        .get("input")
        .and_then(|input| input.get("hint"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
        .map(str::to_owned);
    let subcommands = raw
        .get("subcommands")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|subcommand| {
            let name = subcommand
                .get("name")
                .and_then(serde_json::Value::as_str)?
                .trim();
            if name.is_empty() {
                return None;
            }
            let description = subcommand
                .get("description")
                .and_then(serde_json::Value::as_str)
                .map_or_else(String::new, |description| description.trim().to_owned());
            let usage = subcommand
                .get("usage")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|usage| !usage.is_empty())
                .map(str::to_owned);
            Some(SlashSubcommand {
                name: name.to_owned(),
                description,
                usage,
            })
        })
        .collect();
    let source = raw
        .get("source")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .map(str::to_owned);

    Some(SlashCommand {
        name,
        description,
        aliases,
        input_hint,
        subcommands,
        source,
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
    let query = query.to_ascii_lowercase();
    command.name.to_ascii_lowercase().starts_with(&query)
        || command
            .aliases
            .iter()
            .any(|alias| alias.to_ascii_lowercase().starts_with(&query))
}

enum SlashCompletionContext<'a> {
    TopLevel(&'a str),
    Subcommands(&'a SlashCommand, &'a str),
}

fn valid_slash_fragment(fragment: &str) -> bool {
    fragment.chars().all(|ch| !ch.is_whitespace())
}

fn slash_completion_context<'a>(
    commands: &'a [SlashCommand],
    text: &'a str,
) -> Option<SlashCompletionContext<'a>> {
    let draft = text.trim_start().strip_prefix('/')?;
    if draft.contains(['\n', '\r', '\t']) {
        return None;
    }
    let Some((command_fragment, subcommand_fragment)) = draft.split_once(' ') else {
        return valid_slash_fragment(draft).then_some(SlashCompletionContext::TopLevel(draft));
    };
    if command_fragment.is_empty()
        || !valid_slash_fragment(command_fragment)
        || subcommand_fragment.contains(' ')
        || !valid_slash_fragment(subcommand_fragment)
    {
        return None;
    }
    let command_token = format!("/{command_fragment}");
    let command = commands.iter().find(|command| {
        command.name.eq_ignore_ascii_case(&command_token)
            || command
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(&command_token))
    })?;
    (!command.subcommands.is_empty()).then_some(SlashCompletionContext::Subcommands(
        command,
        subcommand_fragment,
    ))
}

fn slash_suggestion_for_command(command: &SlashCommand) -> SlashSuggestion {
    let expects_input = command.input_hint.is_some() || !command.subcommands.is_empty();
    SlashSuggestion {
        completion_text: if expects_input {
            format!("{} ", command.name)
        } else {
            command.name.clone()
        },
        title: command.name.clone(),
        description: command.description.clone(),
        usage_hint: command.input_hint.clone(),
        source: command.source.clone(),
        expects_input,
        is_subcommand: false,
    }
}

fn slash_suggestion_for_subcommand(
    command: &SlashCommand,
    subcommand: &SlashSubcommand,
) -> SlashSuggestion {
    let expects_input = subcommand.usage.is_some();
    let exact = format!("{} {}", command.name, subcommand.name);
    SlashSuggestion {
        completion_text: if expects_input {
            format!("{exact} ")
        } else {
            exact.clone()
        },
        title: exact,
        description: subcommand.description.clone(),
        usage_hint: subcommand.usage.clone(),
        source: command.source.clone(),
        expects_input,
        is_subcommand: true,
    }
}

fn palette_text_matches(query: &str, values: impl IntoIterator<Item = impl AsRef<str>>) -> bool {
    values
        .into_iter()
        .any(|value| value.as_ref().to_ascii_lowercase().contains(query))
}

pub(crate) fn filter_command_palette_entries(
    commands: &[SlashCommand],
    query: &str,
    show_update_omp: bool,
) -> Vec<CommandPaletteEntry> {
    let query = query.trim().to_ascii_lowercase();
    let slash_query = query.strip_prefix('/').unwrap_or(&query);
    let mut entries = filter_palette_entries(&query)
        .into_iter()
        .filter(|entry| show_update_omp || entry.id != PaletteActionId::UpdateOmp)
        .map(CommandPaletteEntry::Action)
        .collect::<Vec<_>>();
    let action_count = entries.len();
    entries.extend(
        filter_native_slash_entries(&query)
            .into_iter()
            .map(CommandPaletteEntry::NativeSlash),
    );
    let native_names = native_slash_catalog()
        .iter()
        .map(|entry| entry.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();

    for command in commands {
        if native_names.contains(&command.name.to_ascii_lowercase()) {
            continue;
        }
        let aliases = command.aliases.clone();
        let top_level_matches = query.is_empty()
            || palette_text_matches(
                slash_query,
                [
                    command.name.as_str(),
                    command.description.as_str(),
                    command.input_hint.as_deref().unwrap_or_default(),
                    command.source.as_deref().unwrap_or_default(),
                ]
                .into_iter()
                .chain(command.aliases.iter().map(String::as_str)),
            );
        if top_level_matches {
            entries.push(CommandPaletteEntry::Slash {
                suggestion: slash_suggestion_for_command(command),
                aliases: aliases.clone(),
            });
        }

        if query.is_empty() {
            continue;
        }
        for subcommand in &command.subcommands {
            let title = format!("{} {}", command.name, subcommand.name);
            if palette_text_matches(
                slash_query,
                [
                    title.as_str(),
                    subcommand.name.as_str(),
                    subcommand.description.as_str(),
                    subcommand.usage.as_deref().unwrap_or_default(),
                    command.source.as_deref().unwrap_or_default(),
                ]
                .into_iter()
                .chain(command.aliases.iter().map(String::as_str)),
            ) {
                entries.push(CommandPaletteEntry::Slash {
                    suggestion: slash_suggestion_for_subcommand(command, subcommand),
                    aliases: aliases.clone(),
                });
            }
        }
    }

    // With no query, lead with OMP commands so they are immediately
    // discoverable instead of sitting below the full Pimiento action catalog.
    // Once the user types, static exact matches retain first position.
    if query.is_empty() {
        entries.rotate_left(action_count);
    }

    entries
}

pub(crate) fn filter_slash_commands(commands: &[SlashCommand], text: &str) -> Vec<SlashSuggestion> {
    let Some(context) = slash_completion_context(commands, text) else {
        return Vec::new();
    };
    match context {
        SlashCompletionContext::TopLevel(query) => {
            let query = format!("/{query}");
            commands
                .iter()
                .filter(|command| slash_command_matches(command, &query))
                .map(slash_suggestion_for_command)
                .take(SLASH_COMMAND_VISIBLE_CAP)
                .collect()
        }
        SlashCompletionContext::Subcommands(command, query) => command
            .subcommands
            .iter()
            .filter(|subcommand| {
                subcommand
                    .name
                    .to_ascii_lowercase()
                    .starts_with(&query.to_ascii_lowercase())
            })
            .map(|subcommand| slash_suggestion_for_subcommand(command, subcommand))
            .take(SLASH_COMMAND_VISIBLE_CAP)
            .collect(),
    }
}

pub(crate) fn slash_completion_text(suggestion: &SlashSuggestion) -> String {
    suggestion.completion_text.clone()
}

pub(crate) fn todo_open_count(phases: &[TodoPhaseView]) -> usize {
    phases
        .iter()
        .flat_map(|phase| phase.tasks.iter())
        .filter(|task| matches!(task.status.as_str(), "open" | "in_progress"))
        .count()
}

pub(crate) fn render_todo_task_editable(
    task: &TodoTaskView,
    phase_ix: usize,
    task_ix: usize,
    connected: bool,
    session: &gpui::Entity<SessionView>,
    window: &mut Window,
    theme: &Theme,
) -> gpui::AnyElement {
    let blocker = (task.status == "blocked")
        .then(|| task.blocker.clone())
        .flatten();
    let label = format!("{} {}", todo_status_glyph(&task.status), task.content);
    v_flex()
        .w_full()
        .gap_0()
        .child(
            Button::new((
                "todo-toggle",
                phase_ix.saturating_mul(10_000).saturating_add(task_ix),
            ))
            .small()
            .ghost()
            .w_full()
            .child(wrapped_button_text(label))
            .disabled(!connected)
            .on_click(window.listener_for(
                session,
                move |this, _: &ClickEvent, _window, cx| {
                    this.toggle_todo_task(phase_ix, task_ix, cx);
                },
            )),
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

#[allow(clippy::too_many_lines)] // One declarative element keeps the panel's content and click wiring together.
fn render_slash_suggestion_panel(
    suggestions: &[SlashSuggestion],
    selected: usize,
    session: &gpui::Entity<SessionView>,
    window: &mut Window,
    theme: &Theme,
) -> gpui::AnyElement {
    div()
        .id("slash-suggestion-backdrop")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom(px(128.))
        .flex()
        .items_end()
        .justify_center()
        .px_3()
        .on_click(window.listener_for(session, |this, _, _window, cx| {
            this.close_slash_menu();
            cx.notify();
        }))
        .child(
            v_flex()
                .id("slash-suggestion-panel")
                .w_full()
                .max_w(px(720.))
                .max_h(px(280.))
                .gap_0()
                .p_1()
                .occlude()
                .bg(theme.popover)
                .border_1()
                .border_color(theme.border)
                .shadow_lg()
                .rounded_md()
                .on_click(window.listener_for(session, |_this, _, _window, cx| {
                    cx.stop_propagation();
                }))
                .overflow_y_scrollbar()
                .children(suggestions.iter().enumerate().map(|(ix, suggestion)| {
                    let suggestion_for_click = suggestion.clone();
                    v_flex()
                        .id(("slash-command", ix))
                        .w_full()
                        .gap_1()
                        .p_2()
                        .rounded_sm()
                        .cursor_pointer()
                        .when(ix == selected, |row| row.bg(theme.secondary))
                        .hover(|row| row.bg(theme.secondary_hover))
                        .on_click(window.listener_for(session, move |this, _, _window, cx| {
                            this.accept_slash_command(&suggestion_for_click, cx);
                        }))
                        .child(
                            h_flex()
                                .w_full()
                                .items_start()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_sm()
                                        .child(soft_wrap_dynamic_text(&suggestion.title)),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .max_w(gpui::relative(0.4))
                                        .px_1()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme.secondary)
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(soft_wrap_dynamic_text(
                                            &suggestion.source.clone().unwrap_or_else(|| {
                                                if suggestion.is_subcommand {
                                                    "subcommand".into()
                                                } else {
                                                    "command".into()
                                                }
                                            }),
                                        )),
                                ),
                        )
                        .when(!suggestion.description.is_empty(), |col| {
                            col.child(
                                div()
                                    .w_full()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(soft_wrap_dynamic_text(&suggestion.description)),
                            )
                        })
                        .when_some(suggestion.usage_hint.clone(), |col, hint| {
                            col.child(
                                div()
                                    .w_full()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(soft_wrap_dynamic_text(&format!("Usage: {hint}"))),
                            )
                        })
                }))
                .when(suggestions.is_empty(), |panel| {
                    panel.child(
                        Label::new("(no matches)")
                            .text_xs()
                            .text_color(theme.muted_foreground),
                    )
                })
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .flex_wrap()
                        .gap_2()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .border_t_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("↑↓ Navigate")
                        .child("Enter Complete · Esc Dismiss"),
                ),
        )
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
        if self.clear_palette_search {
            self.clear_palette_search = false;
            self.palette_search.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
        if self.clear_theme_search {
            self.clear_theme_search = false;
            self.theme_search.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
        if let Some((placeholder, value)) = self.pending_dialog_input_sync.take() {
            self.dialog_input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx);
                input.set_value(value, window, cx);
                input.focus(window, cx);
            });
        }
        if self.pending_compact_sync {
            self.pending_compact_sync = false;
            self.compact_input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
        if self.refocus_compact_input {
            self.refocus_compact_input = false;
            self.compact_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        if self.refocus_palette_search {
            self.refocus_palette_search = false;
            self.palette_search.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        if self.refocus_theme_search {
            self.refocus_theme_search = false;
            self.theme_search.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        if self.pending_palette_enter {
            self.pending_palette_enter = false;
            let query = self.palette_search.read(cx).value().to_string();
            let commands = parse_slash_commands(self.projection.available_commands_raw.as_ref());
            let matches =
                filter_command_palette_entries(&commands, &query, self.omp_update_is_offered());
            if let Some(entry) = matches.get(self.palette_selected) {
                let entry = entry.clone();
                self.run_command_palette_entry(entry, window, cx);
            }
        }

        if self.launcher_phase != LauncherPhase::Hidden {
            return self.render_launcher(window, cx);
        }

        let theme = cx.theme().clone();
        let has_pending_approval =
            !self.projection.pending_dialogs.is_empty() || self.host_bridge.has_pending_requests();
        let toolbar_status = if has_pending_approval {
            StatusKind::Approval
        } else {
            StatusKind::from_run_phase(&self.projection.run_phase)
        };
        let toolbar_status_tag = if has_pending_approval {
            StatusKind::Approval.tag()
        } else {
            status_pill_for_phase(&self.projection.run_phase)
        };
        let queued_message_count = self
            .projection
            .state
            .queued_message_count
            .unwrap_or_default();
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
        let show_context_high = matches!(self.projection.run_phase, RunPhase::Idle)
            && context_high(self.projection.state.context.as_ref());
        let tokens_label = tokens_per_second_label(self.projection.state.tokens.as_ref())
            .map(|tokens| format!("{tokens}/s"));
        let compacting = matches!(self.projection.run_phase, RunPhase::Compacting);
        let retrying = matches!(self.projection.run_phase, RunPhase::Retrying);
        let fallback_banner = self.projection.fallback_banner.clone();
        let show_activity_banner = compacting || retrying || fallback_banner.is_some();
        let version_gate_notice = self.version_gate_notice.clone();
        let omp_update_ui = self.omp_update_ui.clone();
        let omp_update_disabled_reason = self.omp_update_disabled_reason();
        let can_pick = self.client.is_some();
        let query = if self.model_picker_open {
            self.model_search.read(cx).value().to_string()
        } else {
            String::new()
        };
        let filtered = if self.model_picker_open {
            filter_models(&self.available_models, &query)
        } else {
            Vec::new()
        };
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
        let composer_text = if self.slash_menu == SlashMenuState::Open || self.palette_open {
            self.composer.read(cx).value().to_string()
        } else {
            String::new()
        };
        let slash_commands = if self.slash_menu == SlashMenuState::Open || self.palette_open {
            parse_slash_commands(self.projection.available_commands_raw.as_ref())
        } else {
            Vec::new()
        };
        let slash_menu_visible = self.slash_menu == SlashMenuState::Open
            && slash_draft_is_open(&slash_commands, &composer_text);
        let slash_matches = if slash_menu_visible {
            self.filtered_slash_commands(&composer_text)
        } else {
            Vec::new()
        };
        self.slash_selected = self
            .slash_selected
            .min(slash_matches.len().saturating_sub(1));
        let large_paste_lines = self
            .large_paste_pending
            .as_ref()
            .map(|text| count_text_lines(text));
        let at_mention_cwd = self.session_cwd_path();
        let at_mention_items = if self.at_mention_open {
            self.at_mention_candidates
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let at_mention_selected = self
            .at_mention_selected
            .min(at_mention_items.len().saturating_sub(1));
        let palette_query = if self.palette_open {
            self.palette_search.read(cx).value().to_string()
        } else {
            String::new()
        };
        let palette_matches = if self.palette_open {
            filter_command_palette_entries(
                &slash_commands,
                &palette_query,
                self.omp_update_is_offered(),
            )
        } else {
            Vec::new()
        };
        if palette_matches.is_empty() {
            self.palette_selected = 0;
        } else {
            self.palette_selected = self.palette_selected.min(palette_matches.len() - 1);
        }
        let palette_selected = self.palette_selected;
        let registered_themes = if self.theme_picker_open {
            registered_theme_choices(cx)
        } else {
            Vec::new()
        };
        let theme_query = if self.theme_picker_open {
            self.theme_search.read(cx).value().to_string()
        } else {
            String::new()
        };
        let theme_picker_items = if self.theme_picker_open {
            filter_theme_picker_items(&registered_themes, &theme_query)
        } else {
            Vec::new()
        };
        if theme_picker_items.is_empty() {
            self.theme_picker_selected = 0;
        } else {
            self.theme_picker_selected =
                self.theme_picker_selected.min(theme_picker_items.len() - 1);
        }
        let theme_picker_selected = self.theme_picker_selected;
        let theme_selection = self
            .theme_picker_opening_selection
            .clone()
            .unwrap_or_else(|| cx.global::<ThemeSelectionState>().0.clone());
        let pending_revert = self.pending_revert.clone();
        let transcript_empty = self.projection.transcript.is_empty();
        let subagent_modal_agent = self.selected_subagent_id.as_ref().and_then(|selected| {
            self.subagent_snapshots
                .iter()
                .find(|snapshot| subagent_snapshot_id(snapshot) == Some(selected.as_str()))
                .map(|snapshot| (selected.clone(), subagent_snapshot_summary(snapshot)))
        });
        let subagent_modal_status = self.subagent_modal_status.clone();
        let subagent_modal_lines = self.subagent_tail_lines.clone();
        let display_statuses = display_status_lines(&self.projection.display);
        let fast_supported = current_model.is_some_and(|choice| {
            model_supports_fast_mode(&choice.provider, choice.api.as_deref(), &choice.id)
        }) || self.projection.state.model.as_deref().is_some_and(|label| {
            split_model_label(label)
                .is_some_and(|(provider, id)| model_supports_fast_mode(&provider, None, &id))
        });

        div()
            .size_full()
            .relative()
            .capture_key_down(cx.listener(|this, event, window, cx| {
                if this.composer_typing_should_bypass_capture(event, window, cx) {
                    return;
                }
                let handled = this.handle_subagent_modal_key(event, cx)
                    || this.handle_handoff_key(event, cx)
                    || this.handle_compact_key(event, cx)
                    || this.handle_about_key(event, cx)
                    || this.handle_rename_key(event, cx)
                    || this.handle_branch_picker_key(event, cx)
                    || this.handle_login_picker_key(event, cx)
                    || this.handle_theme_picker_key(event, window, cx)
                    || this.handle_palette_key(event, window, cx)
                    || this.handle_dialog_key(event, cx)
                    || this.handle_attachment_overlay_key(event, cx)
                    || this.handle_composer_paste_key(event, window, cx)
                    || this.handle_slash_key(event, window, cx)
                    || this.handle_abort_esc_key(event, window, cx)
                    || this.handle_transcript_nav_key(event, window, cx);
                if handled {
                    cx.stop_propagation();
                }
            }))
            .child(
                v_flex()
                    .size_full()
                    .bg(theme.background)
                    .text_color(theme.foreground)
                    .child(
                        v_flex()
                            .w_full()
                            .bg(theme.status_bar)
                            .border_b_1()
                            .border_color(theme.status_bar_border)
                            .child(
                                h_flex()
                                    .w_full()
                                    .px_4()
                                    .py_2()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .size(px(8.))
                                                    .rounded_sm()
                                                    .bg(identity_paprika()),
                                            )
                                            .child(
                                                div()
                                                    .w_full()
                                                    .max_h(px(40.))
                                                    .overflow_hidden()
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                                    .flex_1()
                                                    .min_w_0()
                                                    .child(soft_wrap_dynamic_text(
                                                        &self.status_message,
                                                    ))
                                                    .when(
                                                        self.status_message == ABORT_ARM_STATUS,
                                                        |label| label.text_color(theme.warning),
                                                    ),
                                            )
                                            .child(
                                                toolbar_status_tag
                                                    .small()
                                                    .child(toolbar_status.label()),
                                            ),
                                    )
                                    // The inspector already owns ctx% / tps. Do not leave an
                                    // empty flex spacer that can squeeze toolbar actions away.
                                    .when(!self.inspector_open, |bar| {
                                        bar.child(
                                            h_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .flex_wrap()
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
                                    })
                                    .child(
                                        h_flex()
                                            .flex_shrink_0()
                                            .flex_wrap()
                                            .justify_end()
                                            .gap_2()
                                            .when(!self.inspector_open, |group| {
                                                group
                                                    .child(
                                                        Button::new("todo-panel-toggle")
                                                            .icon(IconName::CircleCheck)
                                                            .tooltip("Open checklist")
                                                            .small()
                                                            .ghost()
                                                            .on_click(cx.listener(
                                                                |this,
                                                                 _: &ClickEvent,
                                                                 _window,
                                                                 cx| {
                                                                    this.request_inspector_focus(
                                                                        PaletteActionId::ToggleTodos,
                                                                        cx,
                                                                    );
                                                                },
                                                            )),
                                                    )
                                                    .child(
                                                        Button::new("subagent-drawer-toggle")
                                                            .icon(IconName::Bot)
                                                            .tooltip("Open agents")
                                                            .small()
                                                            .ghost()
                                                            .disabled(!can_pick)
                                                            .on_click(cx.listener(
                                                                |this,
                                                                 _: &ClickEvent,
                                                                 _window,
                                                                 cx| {
                                                                    this.request_inspector_focus(
                                                                        PaletteActionId::ToggleAgents,
                                                                        cx,
                                                                    );
                                                                },
                                                            )),
                                                    )
                                                    .child(
                                                        div()
                                                            .w(px(1.))
                                                            .h(px(16.))
                                                            .mx_1()
                                                            .bg(theme.border),
                                                    )
                                            })
                                            .child(
                                                Button::new("more-actions")
                                                    .icon(IconName::Ellipsis)
                                                    .tooltip("Command palette (⌘K)")
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
                            .when(!display_statuses.is_empty(), |bar| {
                                bar.child(
                                    h_flex()
                                        .w_full()
                                        .px_4()
                                        .pb_2()
                                        .gap_2()
                                        .flex_wrap()
                                        .children(display_statuses.into_iter().map(
                                            |(key, text)| {
                                                Label::new(soft_wrap_dynamic_text(&format!(
                                                    "{key}: {text}"
                                                )))
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                            },
                                        )),
                                )
                            }),
                    )
                    .when_some(version_gate_notice, |parent, notice| {
                        parent.child(
                            h_flex()
                                .w_full()
                                .px_3()
                                .py_1()
                                .items_start()
                                .gap_2()
                                .bg(theme.warning)
                                .text_color(theme.warning_foreground)
                                .text_xs()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(soft_wrap_dynamic_text(&notice)),
                                )
                                .child(
                                    Button::new("dismiss-version-gate")
                                        .label("Dismiss")
                                        .small()
                                        .ghost()
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, _window, cx| {
                                                this.version_gate_notice = None;
                                                cx.notify();
                                            },
                                        )),
                                ),
                        )
                    })
                    .when(
                        !matches!(
                            &omp_update_ui,
                            OmpUpdateUi::Idle | OmpUpdateUi::Checking
                        ),
                        |parent| {
                            parent.child(render_omp_update_banner(
                                &omp_update_ui,
                                omp_update_disabled_reason,
                                cx,
                            ))
                        },
                    )
                    .when(show_context_high, |parent| {
                        parent.child(
                            h_flex()
                                .w_full()
                                .px_3()
                                .py_1()
                                .items_start()
                                .gap_2()
                                .bg(theme.secondary)
                                .border_b_1()
                                .border_color(theme.border)
                                .text_xs()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(theme.warning)
                                        .child("Context is high. Compact before a long turn."),
                                )
                                .child(
                                    Button::new("context-high-compact")
                                        .label("Compact…")
                                        .small()
                                        .ghost()
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, _window, cx| {
                                                this.open_compact_dialog(cx);
                                            },
                                        )),
                                ),
                        )
                    })
                    .when(show_activity_banner, |parent| {
                        parent.child(
                            div()
                                .w_full()
                                .px_3()
                                .py_1()
                                .bg(theme.warning)
                                .text_color(theme.warning_foreground)
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
                                    view.update(cx, |this, cx| this.render_transcript_row(ix, cx))
                                        .unwrap_or_else(|_| div().into_any_element())
                                })
                                .size_full()
                                .px_6()
                                .py_4(),
                            )
                            .when(transcript_empty, |parent| {
                                parent.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .px_4()
                                        .child(
                                            v_flex()
                                                .items_center()
                                                .gap_2()
                                                .max_w(px(360.))
                                                .child(
                                                    Label::new("Pimiento is ready")
                                                        .text_sm()
                                                        .font_weight(gpui::FontWeight::MEDIUM)
                                                        .text_color(theme.foreground),
                                                )
                                                .child(
                                                    Label::new(
                                                        "Send a message below to start this session.",
                                                    )
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground),
                                                ),
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
                            .text_color(theme.warning_foreground)
                            .border_t_1()
                            .border_color(theme.border)
                            .child(
                                Label::new(soft_wrap_dynamic_text(&format!(
                                    "Revert file {path}?"
                                )))
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::MEDIUM),
                            )
                            .child(
                                Label::new(soft_wrap_dynamic_text(&format!("Runs: {command}")))
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
                    .when(!self.host_bridge.pending_calls.is_empty(), |parent| {
                        parent.child(
                            v_flex()
                                .w_full()
                                .max_h(gpui::relative(0.4))
                                .overflow_y_scrollbar()
                                .px_3()
                                .py_2()
                                .gap_2()
                                .bg(theme.secondary)
                                .border_t_1()
                                .border_color(theme.border)
                                .children(
                                    self.host_bridge
                                        .pending_calls
                                        .iter()
                                        .enumerate()
                                        .map(|(index, call)| {
                                            render_host_tool_call(call, index, cx)
                                        }),
                                ),
                        )
                    })
                    .when(
                        !self.host_bridge.pending_uri_requests.is_empty(),
                        |parent| {
                            parent.child(
                                v_flex()
                                    .w_full()
                                    .max_h(gpui::relative(0.4))
                                    .overflow_y_scrollbar()
                                    .px_3()
                                    .py_2()
                                    .gap_2()
                                    .bg(theme.secondary)
                                    .border_t_1()
                                    .border_color(theme.border)
                                    .children(
                                        self.host_bridge
                                            .pending_uri_requests
                                            .iter()
                                            .enumerate()
                                            .map(|(index, request)| {
                                                render_host_uri_request(request, index, cx)
                                            }),
                                    ),
                            )
                        },
                    )
                    .when(!self.projection.pending_dialogs.is_empty(), |parent| {
                        parent.child(
                            v_flex()
                                .w_full()
                                .max_h(gpui::relative(0.55))
                                .px_3()
                                .py_3()
                                .gap_2()
                                .bg(theme.secondary)
                                .border_t_1()
                                .border_color(theme.border)
                                .children(self.projection.pending_dialogs.iter().map(|d| {
                                    render_dialog(
                                        d,
                                        self.dialog_input_view.clone(),
                                        self.select_draft.as_ref(),
                                        self.dialog_input_empty,
                                        cx,
                                    )
                                })),
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
                        v_flex()
                            .w_full()
                            .bg(theme.secondary)
                            .border_t_1()
                            .border_color(theme.border)
                            .shadow_lg()
                            .on_drop(cx.listener(|this, paths: &ExternalPaths, _window, cx| {
                                let paths = paths.paths().to_vec();
                                this.add_attachment_paths(&paths, cx);
                            }))
                            .when(has_pending_approval, |band| {
                                band.opacity(0.55)
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .px_4()
                                    .pt_3()
                                    .pb_2()
                                    .gap_2()
                                    .items_center()
                                    .flex_wrap()
                                    .child(
                                        Button::new("composer-model-picker")
                                            .small()
                                            .ghost()
                                            .child(wrapped_button_text(model_button_label.clone()))
                                            .disabled(!can_pick)
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _window, cx| {
                                                    this.toggle_model_picker(cx);
                                                },
                                            )),
                                    )
                                    .children(
                                        roles_matching_model(
                                            &self.omp_roles,
                                            self.projection.state.model.as_deref(),
                                        )
                                        .into_iter()
                                        .map(|role| {
                                            role_color_tag(role.color)
                                                .small()
                                                .child(soft_wrap_dynamic_text(&role.display_name))
                                        }),
                                    )
                                    .when(show_thinking_control, |row| {
                                        row.child(
                                            Button::new("composer-thinking-picker")
                                                .small()
                                                .ghost()
                                                .child(wrapped_button_text(
                                                    thinking_button_label.clone(),
                                                ))
                                                .disabled(!can_pick)
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _window, cx| {
                                                        this.toggle_thinking_picker(cx);
                                                    },
                                                )),
                                        )
                                    })
                                    .child(div().flex_1().min_w(px(8.)))
                                    .when(queued_message_count > 0, |row| {
                                        row.child(
                                            Tag::secondary()
                                                .small()
                                                .child(format!("queue:{queued_message_count}")),
                                        )
                                    })
                                    .child(
                                        Switch::new("composer-fast-mode")
                                            .label(if fast_supported {
                                                "Fast"
                                            } else {
                                                "Fast · n/a"
                                            })
                                            .small()
                                            .checked(
                                                self.projection
                                                    .state
                                                    .fast_mode_enabled
                                                    .unwrap_or(false),
                                            )
                                            .disabled(!can_pick || !fast_supported)
                                            .on_click(cx.listener(
                                                |this, _checked: &bool, _window, cx| {
                                                    this.toggle_fast_mode(cx);
                                                },
                                            )),
                                    ),
                            )
                            .when(self.model_picker_open, |band| {
                                band.child(
                                    v_flex()
                                        .w_full()
                                        .px_4()
                                        .pb_2()
                                        .child(
                                            v_flex()
                                                .w_full()
                                                .gap_2()
                                                .p_3()
                                                .rounded_lg()
                                                .border_1()
                                                .border_color(theme.border)
                                                .bg(theme.popover)
                                                .shadow_xl()
                                                .child(
                                                    Label::new("Model")
                                                        .text_sm()
                                                        .font_weight(gpui::FontWeight::SEMIBOLD),
                                                )
                                                .child(
                                                    Input::new(&self.model_search)
                                                        .appearance(true)
                                                        .focus_bordered(true),
                                                )
                                                .when(!self.omp_roles.is_empty(), |panel| {
                                                    let current_label = self
                                                        .projection
                                                        .state
                                                        .model
                                                        .as_deref()
                                                        .map_or_else(
                                                            || "(no model)".into(),
                                                            short_model_label,
                                                        );
                                                    panel
                                                        .child(Separator::horizontal())
                                                        .child(
                                                            Label::new(format!(
                                                                "Assign current model ({current_label}) to a role — writes omp config"
                                                            ))
                                                            .text_xs()
                                                            .text_color(theme.muted_foreground),
                                                        )
                                                        .child(
                                                            h_flex().w_full().flex_wrap().gap_1().children(
                                                                self.omp_roles.iter().enumerate().map(
                                                                    |(ix, role)| {
                                                                        let role_name =
                                                                            role.name.clone();
                                                                        let label = format!(
                                                                            "→ {}",
                                                                            role.display_name
                                                                        );
                                                                        Button::new((
                                                                            "omp-role-assign",
                                                                            ix,
                                                                        ))
                                                                        .small()
                                                                        .ghost()
                                                                        .child(wrapped_button_text(label))
                                                                        .on_click(window.listener_for(
                                                                            &view,
                                                                            move |this,
                                                                                  _,
                                                                                  _window,
                                                                                  cx| {
                                                                                this.assign_current_model_to_role(
                                                                                    &role_name,
                                                                                    cx,
                                                                                );
                                                                            },
                                                                        ))
                                                                    },
                                                                ),
                                                            ),
                                                        )
                                                        .child(
                                                            Label::new(
                                                                "Or switch session to a role’s model",
                                                            )
                                                            .text_xs()
                                                            .text_color(theme.muted_foreground),
                                                        )
                                                        .child(
                                                            h_flex().w_full().flex_wrap().gap_1().children(
                                                                self.omp_roles.iter().enumerate().map(
                                                                    |(ix, role)| {
                                                                        let provider =
                                                                            role.provider.clone();
                                                                        let id = role.id.clone();
                                                                        let chip = format!(
                                                                            "{} · {}",
                                                                            role.display_name,
                                                                            short_model_label(
                                                                                &format!(
                                                                                    "{}/{}",
                                                                                    role.provider,
                                                                                    role.id
                                                                                ),
                                                                            )
                                                                        );
                                                                        h_flex()
                                                                            .gap_1()
                                                                            .items_center()
                                                                            .child(
                                                                                role_color_tag(
                                                                                    role.color,
                                                                                )
                                                                                .small()
                                                                                .child(
                                                                                    role.display_name
                                                                                        .clone(),
                                                                                ),
                                                                            )
                                                                            .child(
                                                                                Button::new((
                                                                                    "omp-role-switch",
                                                                                    ix,
                                                                                ))
                                                                                .small()
                                                                                .ghost()
                                                                                .child(wrapped_button_text(chip))
                                                                                .on_click(window.listener_for(
                                                                                    &view,
                                                                                    move |this,
                                                                                          _,
                                                                                          _window,
                                                                                          cx| {
                                                                                        this.close_model_picker(
                                                                                            cx,
                                                                                        );
                                                                                        this.set_model(
                                                                                            provider.clone(),
                                                                                            id.clone(),
                                                                                            cx,
                                                                                        );
                                                                                    },
                                                                                )),
                                                                            )
                                                                    },
                                                                ),
                                                            ),
                                                        )
                                                })
                                                .child(
                                                    div()
                                                        .w_full()
                                                        .max_h(px(260.))
                                                        .overflow_y_scrollbar()
                                                        .gap_1()
                                                        .children(visible.iter().enumerate().map(
                                                            |(ix, choice)| {
                                                                let label = format!(
                                                                    "{}/{}",
                                                                    choice.provider, choice.id
                                                                );
                                                                let provider =
                                                                    choice.provider.clone();
                                                                let id = choice.id.clone();
                                                                let current =
                                                                    model_label.as_str()
                                                                        == label.as_str();
                                                                Button::new(("model-choice", ix))
                                                                    .small()
                                                                    .w_full()
                                                                    .child(wrapped_button_text(label))
                                                                    .when(current, Button::primary)
                                                                    .when(!current, Button::ghost)
                                                                    .on_click(window.listener_for(
                                                                        &view,
                                                                        move |this,
                                                                              _,
                                                                              _window,
                                                                              cx| {
                                                                            this.close_model_picker(
                                                                                cx,
                                                                            );
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
                                                        Label::new(footer.clone())
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
                                        ),
                                )
                            })
                            .when(self.thinking_picker_open, |band| {
                                band.child(
                                    h_flex()
                                        .w_full()
                                        .px_4()
                                        .pb_2()
                                        .gap_1()
                                        .flex_wrap()
                                        .children(thinking_options.iter().enumerate().map(
                                            |(ix, level)| {
                                                let level = level.clone();
                                                Button::new(("thinking-choice", ix))
                                                    .ghost()
                                                    .small()
                                                    .child(wrapped_button_text(level.clone()))
                                                    .on_click(window.listener_for(
                                                        &view,
                                                        move |this, _, _window, cx| {
                                                            this.set_thinking_level(&level, cx);
                                                        },
                                                    ))
                                            },
                                        )),
                                )
                            })
                            .when(!self.pending_attachments.is_empty(), |band| {
                                band.child(
                                    h_flex()
                                        .w_full()
                                        .px_4()
                                        .pb_2()
                                        .gap_2()
                                        .flex_wrap()
                                        .children(self.pending_attachments.iter().enumerate().map(
                                            |(ix, attachment)| {
                                                let kind = if attachment.is_image() {
                                                    "img"
                                                } else {
                                                    "@"
                                                };
                                                let label =
                                                    format!("{kind} {}", attachment.chip_label());
                                                h_flex()
                                                    .gap_1()
                                                    .items_center()
                                                    .px_2()
                                                    .py_1()
                                                    .rounded_md()
                                                    .bg(theme.background)
                                                    .border_1()
                                                    .border_color(theme.border)
                                                    .child(
                                                        Label::new(soft_wrap_dynamic_text(&label))
                                                            .text_xs(),
                                                    )
                                                    .child(
                                                        Button::new(("remove-attachment", ix))
                                                            .icon(IconName::Close)
                                                            .tooltip("Remove attachment")
                                                            .small()
                                                            .ghost()
                                                            .on_click(cx.listener(
                                                                move |this,
                                                                      _: &ClickEvent,
                                                                      _window,
                                                                      cx| {
                                                                    this.remove_attachment_at(
                                                                        ix, cx,
                                                                    );
                                                                },
                                                            )),
                                                    )
                                            },
                                        )),
                                )
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .px_4()
                                    .pb_3()
                                    .pt_1()
                                    .items_end()
                                    .gap_2()
                                    .child(
                                        // One bordered chrome owns height: input + attach + Send
                                        // share the same top/bottom edges (no optical border mismatch).
                                        // min_h (not h) so auto_grow can raise the field; overflow_hidden
                                        // only clips Send's square corners to the rounded chrome.
                                        h_flex()
                                            .relative()
                                            .flex_1()
                                            .min_w_0()
                                            .min_h(composer_control_height())
                                            .items_stretch()
                                            .rounded_md()
                                            .border_1()
                                            .border_color(theme.border)
                                            .bg(theme.background)
                                            .overflow_hidden()
                                            .child(
                                                h_flex()
                                                    .relative()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .min_h(composer_control_height())
                                                    .items_stretch()
                                                    .gap_2()
                                                    .pl_3()
                                                    .pr_1()
                                                    .child(
                                                        div()
                                                            .relative()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .flex()
                                                            .items_center()
                                                            .py_1()
                                                            .child(self.composer_view.clone())
                                                            .when_some(
                                                                large_paste_lines,
                                                                |parent, lines| {
                                                                    parent.child(
                                                                        v_flex()
                                                                            .absolute()
                                                                            .bottom_full()
                                                                            .left_0()
                                                                            .right_0()
                                                                            .gap_1()
                                                                            .p_2()
                                                                            .mb_1()
                                                                            .bg(theme.popover)
                                                                            .border_1()
                                                                            .border_color(
                                                                                theme.border,
                                                                            )
                                                                            .shadow_lg()
                                                                            .rounded_md()
                                                                            .child(
                                                                                Label::new(
                                                                                    format!(
                                                                                        "Pasted {lines} lines"
                                                                                    ),
                                                                                )
                                                                                .text_sm()
                                                                                .font_weight(
                                                                                    gpui::FontWeight::SEMIBOLD,
                                                                                ),
                                                                            )
                                                                            .child(
                                                                                Label::new(
                                                                                    "Wrap in <attachment>, save local://paste, or paste inline",
                                                                                )
                                                                                .text_xs()
                                                                                .text_color(
                                                                                    theme.muted_foreground,
                                                                                ),
                                                                            )
                                                                            .child(
                                                                                h_flex()
                                                                                    .flex_wrap()
                                                                                    .gap_1()
                                                                                    .child(
                                                                                        Button::new(
                                                                                            "large-paste-wrap",
                                                                                        )
                                                                                        .label(
                                                                                            "Wrap",
                                                                                        )
                                                                                        .small()
                                                                                        .primary()
                                                                                        .on_click(
                                                                                            cx.listener(
                                                                                                |this,
                                                                                                 _: &ClickEvent,
                                                                                                 _w,
                                                                                                 cx| {
                                                                                                    this.apply_large_paste_choice(
                                                                                                        LargePasteChoice::Wrap,
                                                                                                        cx,
                                                                                                    );
                                                                                                },
                                                                                            ),
                                                                                        ),
                                                                                    )
                                                                                    .child(
                                                                                        Button::new(
                                                                                            "large-paste-save",
                                                                                        )
                                                                                        .label(
                                                                                            "Save local://",
                                                                                        )
                                                                                        .small()
                                                                                        .ghost()
                                                                                        .on_click(
                                                                                            cx.listener(
                                                                                                |this,
                                                                                                 _: &ClickEvent,
                                                                                                 _w,
                                                                                                 cx| {
                                                                                                    this.apply_large_paste_choice(
                                                                                                        LargePasteChoice::SaveLocal,
                                                                                                        cx,
                                                                                                    );
                                                                                                },
                                                                                            ),
                                                                                        ),
                                                                                    )
                                                                                    .child(
                                                                                        Button::new(
                                                                                            "large-paste-inline",
                                                                                        )
                                                                                        .label(
                                                                                            "Inline",
                                                                                        )
                                                                                        .small()
                                                                                        .ghost()
                                                                                        .on_click(
                                                                                            cx.listener(
                                                                                                |this,
                                                                                                 _: &ClickEvent,
                                                                                                 _w,
                                                                                                 cx| {
                                                                                                    this.apply_large_paste_choice(
                                                                                                        LargePasteChoice::Inline,
                                                                                                        cx,
                                                                                                    );
                                                                                                },
                                                                                            ),
                                                                                        ),
                                                                                    ),
                                                                            ),
                                                                    )
                                                                },
                                                            )
                                                            .when(
                                                                !at_mention_items.is_empty(),
                                                                |parent| {
                                                                    parent.child(
                                                                        v_flex()
                                                                            .absolute()
                                                                            .bottom_full()
                                                                            .left_0()
                                                                            .right_0()
                                                                            .max_h(px(240.))
                                                                            .overflow_y_scrollbar()
                                                                            .gap_0()
                                                                            .p_1()
                                                                            .mb_1()
                                                                            .bg(theme.popover)
                                                                            .border_1()
                                                                            .border_color(
                                                                                theme.border,
                                                                            )
                                                                            .shadow_lg()
                                                                            .rounded_md()
                                                                            .children(
                                                                                at_mention_items
                                                                                    .iter()
                                                                                    .enumerate()
                                                                                    .map(|(ix, path)| {
                                                                                        let path_for_click =
                                                                                            path.clone();
                                                                                        let label =
                                                                                            path_mention_display(
                                                                                                path,
                                                                                                Some(
                                                                                                    at_mention_cwd
                                                                                                        .as_path(),
                                                                                                ),
                                                                                            );
                                                                                        Button::new((
                                                                                            "at-mention",
                                                                                            ix,
                                                                                        ))
                                                                                        .ghost()
                                                                                        .small()
                                                                                        .w_full()
                                                                                        .child(
                                                                                            wrapped_button_text(
                                                                                                label,
                                                                                            ),
                                                                                        )
                                                                                        .when(
                                                                                            ix == at_mention_selected,
                                                                                            |button| {
                                                                                                button.bg(
                                                                                                    theme.secondary,
                                                                                                )
                                                                                            },
                                                                                        )
                                                                                        .on_click(
                                                                                            window.listener_for(
                                                                                                &view,
                                                                                                move |this,
                                                                                                      _,
                                                                                                      _window,
                                                                                                      cx| {
                                                                                                    this.accept_at_mention(
                                                                                                        &path_for_click,
                                                                                                        cx,
                                                                                                    );
                                                                                                },
                                                                                            ),
                                                                                        )
                                                                                    }),
                                                                            ),
                                                                    )
                                                                },
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .flex_shrink_0()
                                                            .child(
                                                                Button::new("attach-files")
                                                                    .icon(IconName::Plus)
                                                                    .tooltip("Attach files")
                                                                    .small()
                                                                    .ghost()
                                                                    .disabled(!can_pick)
                                                                    .on_click(cx.listener(
                                                                        |this,
                                                                         _: &ClickEvent,
                                                                         window,
                                                                         cx| {
                                                                            this.prompt_attach_files(
                                                                                window, cx,
                                                                            );
                                                                        },
                                                                    )),
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .w(px(1.))
                                                    .self_stretch()
                                                    .bg(theme.border)
                                                    .flex_shrink_0(),
                                            )
                                            .child(
                                                Button::new("send")
                                                    .primary()
                                                    // Avoid Medium's h_8; fill the chrome height.
                                                    .with_size(gpui_component::Size::Size(px(0.)))
                                                    .h_full()
                                                    .rounded_none()
                                                    .px_3()
                                                    .label(if composer_uses_steer(
                                                        &self.projection.run_phase,
                                                    ) {
                                                        "Steer"
                                                    } else {
                                                        "Send"
                                                    })
                                                    .tooltip(if composer_uses_steer(
                                                        &self.projection.run_phase,
                                                    ) {
                                                        if cfg!(target_os = "macos") {
                                                            "Steer running turn (⌘↩)"
                                                        } else {
                                                            "Steer running turn (Ctrl+Enter)"
                                                        }
                                                    } else if cfg!(target_os = "macos") {
                                                        "Send (⌘↩)"
                                                    } else {
                                                        "Send (Ctrl+Enter)"
                                                    })
                                                    .disabled(!self.can_send())
                                                    .on_click(cx.listener(
                                                        |this, _: &ClickEvent, window, cx| {
                                                            this.send_composer_message(cx);
                                                            this.clear_composer = false;
                                                            this.pending_composer_value = None;
                                                            this.composer.update(cx, |input, cx| {
                                                                input.set_value("", window, cx);
                                                            });
                                                        },
                                                    )),
                                            ),
                                    )
                                    .when_some(self.send_disabled_reason(), |row, reason| {
                                        row.child(
                                            Label::new(reason)
                                                .text_xs()
                                                .text_color(theme.muted_foreground),
                                        )
                                    })
                                    .when(
                                        matches!(
                                            self.projection.run_phase,
                                            RunPhase::Streaming
                                        ),
                                        |parent| {
                                            parent.child(
                                                Button::new("follow-up")
                                                    .label("Follow-up")
                                                    .ghost()
                                                    .with_size(gpui_component::Size::Size(px(0.)))
                                                    .h(composer_control_height())
                                                    .px_3()
                                                    .disabled(!self.can_follow_up())
                                                    .on_click(cx.listener(
                                                        |this, _: &ClickEvent, _window, cx| {
                                                            this.do_follow_up(cx);
                                                        },
                                                    )),
                                            )
                                        },
                                    )
                                    .when(self.can_abort(), |parent| {
                                        parent.child(
                                            Button::new("abort")
                                                .danger()
                                                .with_size(gpui_component::Size::Size(px(0.)))
                                                .h(composer_control_height())
                                                .px_3()
                                                .label("Abort")
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, window, cx| {
                                                        this.do_abort(window, cx);
                                                    },
                                                )),
                                        )
                                    }),
                            ),
                    ),
            )
            .when(slash_menu_visible, |root| {
                root.child(render_slash_suggestion_panel(
                    &slash_matches,
                    self.slash_selected,
                    &view,
                    window,
                    &theme,
                ))
            })
            .when(self.subagent_modal_open, |parent| {
                let (agent_id, agent_summary) = subagent_modal_agent.unwrap_or_else(|| {
                    (
                        "Unavailable".to_owned(),
                        "The selected agent is no longer present in OMP's snapshot.".to_owned(),
                    )
                });
                parent.child(
                    div()
                        .id("subagent-modal-backdrop")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_4()
                        .bg(theme.overlay)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.close_subagent_modal(cx);
                        }))
                        .child(
                            v_flex()
                                .id("subagent-modal-panel")
                                .w_full()
                                .max_w(px(720.))
                                .max_h(gpui::relative(0.74))
                                .gap_3()
                                .p_4()
                                .rounded_lg()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_xl()
                                .cursor_default()
                                .on_click(cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .items_start()
                                        .justify_between()
                                        .gap_3()
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .gap_1()
                                                .child(
                                                    h_flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .child(
                                                            Icon::new(IconName::Bot)
                                                                .text_color(theme.primary),
                                                        )
                                                        .child(
                                                            Label::new("Subagent work")
                                                                .text_lg()
                                                                .font_weight(
                                                                    gpui::FontWeight::SEMIBOLD,
                                                                ),
                                                        ),
                                                )
                                                .child(
                                                    Label::new(soft_wrap_dynamic_text(
                                                        &agent_summary,
                                                    ))
                                                        .text_sm()
                                                        .text_color(theme.muted_foreground),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .font_family(
                                                            theme.mono_font_family.clone(),
                                                        )
                                                        .text_color(theme.muted_foreground)
                                                        .child(soft_wrap_dynamic_text(&agent_id)),
                                                ),
                                        )
                                        .child(
                                            Button::new("subagent-modal-close")
                                                .icon(IconName::Close)
                                                .tooltip("Close subagent work")
                                                .small()
                                                .ghost()
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _w, cx| {
                                                        this.close_subagent_modal(cx);
                                                    },
                                                )),
                                        ),
                                )
                                .child(Separator::horizontal())
                                .child(
                                    v_flex()
                                        .w_full()
                                        .flex_1()
                                        .min_h(px(0.))
                                        .overflow_y_scrollbar()
                                        .gap_2()
                                        .child(
                                            Label::new(soft_wrap_dynamic_text(
                                                &subagent_modal_status,
                                            ))
                                                .text_xs()
                                                .text_color(theme.muted_foreground),
                                        )
                                        .when(subagent_modal_lines.is_empty(), |body| {
                                            body.child(
                                                Label::new("No work messages to display yet.")
                                                    .text_sm()
                                                    .text_color(theme.muted_foreground),
                                            )
                                        })
                                        .children(subagent_modal_lines.into_iter().map(|line| {
                                            div()
                                                .w_full()
                                                .p_2()
                                                .rounded_sm()
                                                .bg(theme.secondary)
                                                .text_xs()
                                                .font_family(theme.mono_font_family.clone())
                                                .overflow_x_scrollbar()
                                                .child(line)
                                        })),
                                ),
                        ),
                )
            })
            .when(self.theme_picker_open, |parent| {
                parent.child(
                    div()
                        .id("theme-picker-backdrop")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_start()
                        .justify_center()
                        .px_4()
                        .pt_16()
                        .bg(theme.overlay)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.close_theme_picker(window, cx);
                        }))
                        .child(
                            v_flex()
                                .id("theme-picker-panel")
                                .w_full()
                                .max_w(px(520.))
                                .max_h(px(520.))
                                .gap_2()
                                .p_3()
                                .rounded_lg()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_xl()
                                .on_click(cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .items_start()
                                        .flex_wrap()
                                        .gap_2()
                                        .justify_between()
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .items_center()
                                                .child(
                                                    Icon::new(IconName::Palette)
                                                        .small()
                                                        .text_color(theme.muted_foreground),
                                                )
                                                .child(
                                                    Label::new("Theme")
                                                        .text_sm()
                                                        .flex_shrink_0(),
                                                ),
                                        )
                                        .child(
                                            Label::new("↑↓ choose · Enter apply · Esc close")
                                                .text_xs()
                                                .flex_1()
                                                .min_w_0()
                                                .text_color(theme.muted_foreground),
                                        )
                                        .child(
                                            Button::new("theme-picker-close")
                                                .icon(IconName::Close)
                                                .tooltip("Close theme picker")
                                                .small()
                                                .ghost()
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, window, cx| {
                                                        this.close_theme_picker(window, cx);
                                                    },
                                                )),
                                        ),
                                )
                                .child(
                                    Input::new(&self.theme_search)
                                        .appearance(true)
                                        .focus_bordered(true),
                                )
                                .child(
                                    v_flex()
                                        .w_full()
                                        .max_h(px(420.))
                                        .overflow_y_scrollbar()
                                        .gap_1()
                                        .children(theme_picker_items.iter().enumerate().map(
                                            |(ix, item)| {
                                                let item = item.clone();
                                                let item_for_hover = item.clone();
                                                let selected = ix == theme_picker_selected;
                                                let active = theme_picker_item_is_active(
                                                    &item,
                                                    &theme_selection,
                                                );
                                                let (section, label, swatches) = match &item {
                                                    ThemePickerItem::Appearance(preference) => (
                                                        "Appearance",
                                                        preference.label().to_owned(),
                                                        [None, None, None],
                                                    ),
                                                    ThemePickerItem::Theme { name, mode } => {
                                                        let section = if mode.is_dark() {
                                                            "Dark theme"
                                                        } else {
                                                            "Light theme"
                                                        };
                                                        let swatches = registered_themes
                                                            .iter()
                                                            .find(|theme| theme.name == *name)
                                                            .map_or([None, None, None], |theme| {
                                                                theme.swatches
                                                            });
                                                        (section, name.clone(), swatches)
                                                    }
                                                };
                                                h_flex()
                                                    .id(("theme-picker-entry", ix))
                                                    .w_full()
                                                    .min_h(px(44.))
                                                    .flex_shrink_0()
                                                    .items_start()
                                                    .gap_2()
                                                    .px_3()
                                                    .py_2()
                                                    .rounded_sm()
                                                    .cursor_pointer()
                                                    .when(selected, |row| {
                                                        row.bg(theme.secondary)
                                                    })
                                                    .on_hover(cx.listener(
                                                        move |this, hovered, window, cx| {
                                                            if *hovered {
                                                                this.theme_picker_selected = ix;
                                                                this.preview_theme_picker_item(
                                                                    &item_for_hover,
                                                                    window,
                                                                    cx,
                                                                );
                                                                cx.notify();
                                                            }
                                                        },
                                                    ))
                                                    .child(
                                                        div()
                                                            .w(px(18.))
                                                            .flex_shrink_0()
                                                            .text_sm()
                                                            .text_color(theme.primary)
                                                            .child(if active { "✓" } else { "" }),
                                                    )
                                                    .child(
                                                        v_flex()
                                                            .flex_1()
                                                            .min_w(px(0.))
                                                            .gap_0p5()
                                                            .child(
                                                                Label::new(section)
                                                                    .text_xs()
                                                                    .text_color(
                                                                        theme.muted_foreground,
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .w_full()
                                                                    .text_sm()
                                                                    .child(soft_wrap_dynamic_text(
                                                                        &label,
                                                                    )),
                                                            ),
                                                    )
                                                    .child(
                                                        h_flex().flex_shrink_0().gap_1().children(
                                                            swatches
                                                                .into_iter()
                                                                .flatten()
                                                                .enumerate()
                                                                .map(|(swatch_ix, color)| {
                                                                    div()
                                                                        .id((
                                                                            "theme-swatch",
                                                                            ix * 3 + swatch_ix,
                                                                        ))
                                                                        .size(px(14.))
                                                                        .rounded_full()
                                                                        .border_1()
                                                                        .border_color(theme.border)
                                                                        .bg(color)
                                                                }),
                                                        ),
                                                    )
                                                    .on_click(cx.listener(
                                                        move |this,
                                                              _: &ClickEvent,
                                                              window,
                                                              cx| {
                                                            this.choose_theme_picker_item(
                                                                &item, window, cx,
                                                            );
                                                        },
                                                    ))
                                            },
                                        ))
                                        .when(theme_picker_items.is_empty(), |list| {
                                            list.child(
                                                Label::new("No matching themes")
                                                    .text_sm()
                                                    .text_color(theme.muted_foreground),
                                            )
                                        }),
                                ),
                        ),
                )
            })
            .when(self.palette_open, |parent| {
                parent.child(
                    div()
                        .id("command-palette-backdrop")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_start()
                        .justify_center()
                        .px_4()
                        .pt_16()
                        .bg(theme.overlay)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.close_palette(cx);
                        }))
                        .child(
                            v_flex()
                                .id("command-palette-panel")
                                .w_full()
                                .max_w(px(720.))
                                .h(gpui::relative(0.8))
                                .max_h(px(680.))
                                .min_h(px(0.))
                                .gap_1()
                                .p_3()
                                .rounded_lg()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_xl()
                                .on_click(cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    Icon::new(IconName::Search)
                                                        .small()
                                                        .text_color(theme.muted_foreground),
                                                )
                                                .child(
                                                    Label::new("Command palette")
                                                        .text_sm()
                                                        .font_weight(
                                                            gpui::FontWeight::SEMIBOLD,
                                                        ),
                                                ),
                                        )
                                        .child(
                                            Button::new("command-palette-close")
                                                .icon(IconName::Close)
                                                .tooltip("Close command palette (Esc)")
                                                .small()
                                                .ghost()
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _w, cx| {
                                                        this.close_palette(cx);
                                                    },
                                                )),
                                        ),
                                )
                                .child(
                                    Input::new(&self.palette_search)
                                        .appearance(true)
                                        .focus_bordered(true),
                                )
                                .child(
                                    v_flex()
                                        .w_full()
                                        .flex_1()
                                        .min_h(px(0.))
                                        .overflow_y_scrollbar()
                                        .gap_1()
                                        .children(
                                            palette_matches.iter().enumerate().map(
                                                |(ix, entry)| {
                                                    let entry = entry.clone();
                                                    let selected = ix == palette_selected;
                                                    let row_data =
                                                        command_palette_row_data(&entry);
                                                    v_flex()
                                                        .id(("palette-entry", ix))
                                                        .w_full()
                                                        .min_h(px(44.))
                                                        .flex_shrink_0()
                                                        .gap_0p5()
                                                        .px_3()
                                                        .py_2()
                                                        .rounded_sm()
                                                        .cursor_pointer()
                                                        .when(selected, |row| {
                                                            row.bg(theme.secondary)
                                                        })
                                                        .on_hover(cx.listener(
                                                            move |this, hovered, _window, cx| {
                                                                if *hovered {
                                                                    this.palette_selected = ix;
                                                                    cx.notify();
                                                                }
                                                            },
                                                        ))
                                                        .child(
                                                            div()
                                                                .w_full()
                                                                .text_sm()
                                                                .child(soft_wrap_dynamic_text(
                                                                    &row_data.title,
                                                                )),
                                                        )
                                                        .child(
                                                            Label::new(row_data.metadata)
                                                                .text_xs()
                                                                .text_color(
                                                                    theme.muted_foreground,
                                                                ),
                                                        )
                                                        .when(
                                                            !row_data.description.is_empty(),
                                                            |row| {
                                                            row.child(
                                                                div()
                                                                    .w_full()
                                                                    .text_xs()
                                                                    .text_color(
                                                                        theme.muted_foreground,
                                                                    )
                                                                    .child(
                                                                        soft_wrap_dynamic_text(
                                                                            &row_data.description,
                                                                        ),
                                                                    ),
                                                            )
                                                        },
                                                        )
                                                        .when_some(row_data.usage, |row, usage| {
                                                            row.child(
                                                                div()
                                                                    .w_full()
                                                                    .text_xs()
                                                                    .text_color(
                                                                        theme.muted_foreground,
                                                                    )
                                                                    .child(
                                                                        soft_wrap_dynamic_text(
                                                                            &usage,
                                                                        ),
                                                                    ),
                                                            )
                                                        })
                                                        .on_click(cx.listener(
                                                            move |this,
                                                                  _: &ClickEvent,
                                                                  window,
                                                                  cx| {
                                                                this.run_command_palette_entry(
                                                                    entry.clone(),
                                                                    window,
                                                                    cx,
                                                                );
                                                            },
                                                        ))
                                                },
                                            ),
                                        )
                                        .when(palette_matches.is_empty(), |list| {
                                            list.child(
                                                Label::new("(no matches)")
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground),
                                            )
                                        }),
                                ),
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
                        .px_4()
                        .bg(theme.overlay)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.close_about(cx);
                        }))
                        .child(
                            v_flex()
                                .id("about-panel")
                                .w_full()
                                .max_w(px(420.))
                                .max_h(gpui::relative(0.84))
                                .overflow_y_scroll()
                                .gap_3()
                                .p_5()
                                .rounded_lg()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_xl()
                                .on_click(cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            Icon::new(IconName::Asterisk)
                                                .text_color(identity_paprika()),
                                        )
                                        .child(
                                            Label::new("About Pimiento")
                                                .text_lg()
                                                .font_weight(gpui::FontWeight::SEMIBOLD),
                                        ),
                                )
                                .child(
                                    Label::new(
                                        "Native GPUI client for your existing omp install.",
                                    )
                                    .text_sm()
                                    .text_color(theme.muted_foreground),
                                )
                                .child(Label::new(version).text_sm())
                                .when_some(
                                    render_omp_update_about(
                                        &omp_update_ui,
                                        omp_update_disabled_reason,
                                        cx,
                                    ),
                                    gpui::ParentElement::child,
                                )
                                .child(
                                    Label::new(
                                        "⌘/Ctrl+Shift+P palette · ⌘/Ctrl+K palette · ⌘/Ctrl+B sessions · ⌘/Ctrl+J inspector · ⌘/Ctrl+1–9 switch · ⌘/Ctrl+T/W new/close · Enter send · Esc×2 abort · PageUp/Down Home/End transcript · right-click session: Rename",
                                    )
                                    .text_xs()
                                    .text_color(theme.muted_foreground),
                                )
                                .child(
                                    Label::new("Local-only · no telemetry")
                                        .text_xs()
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
            .when(self.compact_open, |parent| {
                parent.child(
                    div()
                        .id("compact-backdrop")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_4()
                        .bg(theme.overlay)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.close_compact_dialog(cx);
                        }))
                        .child(
                            v_flex()
                                .id("compact-panel")
                                .w_full()
                                .max_w(px(420.))
                                .max_h(gpui::relative(0.84))
                                .overflow_y_scroll()
                                .gap_3()
                                .p_4()
                                .rounded_lg()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_xl()
                                .on_click(cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            Icon::new(IconName::ChevronsUpDown)
                                                .small()
                                                .text_color(theme.muted_foreground),
                                        )
                                        .child(
                                            Label::new("Compact context")
                                                .text_sm()
                                                .font_weight(gpui::FontWeight::SEMIBOLD),
                                        ),
                                )
                                .child(
                                    Label::new(
                                        "Optionally tell OMP what the compacted context must preserve.",
                                    )
                                    .text_xs()
                                    .text_color(theme.muted_foreground),
                                )
                                .child(
                                    Input::new(&self.compact_input)
                                        .appearance(true)
                                        .focus_bordered(true),
                                )
                                .child(
                                    h_flex()
                                        .w_full()
                                        .justify_between()
                                        .items_start()
                                        .flex_wrap()
                                        .gap_2()
                                        .child(
                                            Label::new("Enter compacts · Shift+Enter adds a line")
                                                .text_xs()
                                                .flex_1()
                                                .min_w_0()
                                                .text_color(theme.muted_foreground),
                                        )
                                        .child(
                                            h_flex()
                                                .flex_wrap()
                                                .gap_2()
                                                .child(
                                                    Button::new("compact-cancel")
                                                        .label("Cancel")
                                                        .ghost()
                                                        .on_click(cx.listener(
                                                            |this, _: &ClickEvent, _w, cx| {
                                                                this.close_compact_dialog(cx);
                                                            },
                                                        )),
                                                )
                                                .child(
                                                    Button::new("compact-confirm")
                                                        .label("Compact")
                                                        .primary()
                                                        .on_click(cx.listener(
                                                            |this, _: &ClickEvent, _w, cx| {
                                                                this.confirm_compact(cx);
                                                            },
                                                        )),
                                                ),
                                        ),
                                ),
                        ),
                )
            })
            .when(self.handoff_confirm_open, |parent| {
                parent.child(
                    div()
                        .id("handoff-backdrop")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_4()
                        .bg(theme.overlay)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.close_handoff_confirmation(cx);
                        }))
                        .child(
                            v_flex()
                                .id("handoff-panel")
                                .w_full()
                                .max_w(px(420.))
                                .max_h(gpui::relative(0.84))
                                .overflow_y_scroll()
                                .gap_3()
                                .p_4()
                                .rounded_lg()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_xl()
                                .on_click(cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            Icon::new(IconName::SquareTerminal)
                                                .small()
                                                .text_color(theme.muted_foreground),
                                        )
                                        .child(
                                            Label::new("Handoff to TUI?")
                                                .text_sm()
                                                .font_weight(gpui::FontWeight::SEMIBOLD),
                                        ),
                                )
                                .child(
                                    Label::new(
                                        "OMP will hand off this authoritative session. Continue the work in its terminal UI.",
                                    )
                                    .text_sm()
                                    .text_color(theme.muted_foreground),
                                )
                                .child(
                                    h_flex()
                                        .w_full()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("handoff-cancel")
                                                .label("Cancel")
                                                .ghost()
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _w, cx| {
                                                        this.close_handoff_confirmation(cx);
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new("handoff-confirm")
                                                .label("Handoff")
                                                .primary()
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _w, cx| {
                                                        this.confirm_handoff(cx);
                                                    },
                                                )),
                                        ),
                                ),
                        ),
                )
            })
            .when(self.rename_open, |parent| {
                parent.child(
                    div()
                        .id("rename-backdrop")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_4()
                        .bg(theme.overlay)
                        .cursor_pointer()
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _w, cx| {
                            this.close_rename(cx);
                        }))
                        .child(
                            v_flex()
                                .id("rename-panel")
                                .w_full()
                                .max_w(px(420.))
                                .max_h(gpui::relative(0.84))
                                .overflow_y_scroll()
                                .gap_3()
                                .p_4()
                                .rounded_lg()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_xl()
                                .on_mouse_down(gpui::MouseButton::Left, cx.listener(|_this, _, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    Label::new("Rename session")
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD),
                                )
                                .child(
                                    Label::new(
                                        "Updates OMP sessionName — shown in the rail and window title.",
                                    )
                                    .text_xs()
                                    .text_color(theme.muted_foreground),
                                )
                                .child(
                                    Input::new(&self.rename_input)
                                        .appearance(true)
                                        .focus_bordered(true),
                                )
                                .child(
                                    h_flex()
                                        .w_full()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("rename-cancel")
                                                .label("Cancel")
                                                .ghost()
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _w, cx| {
                                                        this.close_rename(cx);
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new("rename-confirm")
                                                .label("Rename")
                                                .primary()
                                                .on_click(cx.listener(
                                                    |this, _: &ClickEvent, _w, cx| {
                                                        this.confirm_rename(cx);
                                                    },
                                                )),
                                        ),
                                ),
                        ),
                )
            })
            .when_some(self.branch_picker.clone(), |parent, messages| {
                let selected = self.branch_picker_selected;
                parent.child(
                    div()
                        .id("branch-backdrop")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_4()
                        .bg(theme.overlay)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.close_branch_picker(cx);
                        }))
                        .child(
                            v_flex()
                                .id("branch-panel")
                                .w_full()
                                .max_w(px(440.))
                                .max_h(px(420.))
                                .gap_2()
                                .p_4()
                                .rounded_lg()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_xl()
                                .on_click(cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    Label::new("Branch into new tab")
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD),
                                )
                                .child(
                                    Label::new("Pick a user message to fork from")
                                        .text_xs()
                                        .text_color(theme.muted_foreground),
                                )
                                .child(
                                    v_flex()
                                        .w_full()
                                        .max_h(px(320.))
                                        .overflow_y_scrollbar()
                                        .gap_1()
                                        .children(messages.into_iter().enumerate().map(
                                            |(ix, msg)| {
                                                let entry_id = msg.entry_id.clone();
                                                let preview =
                                                    branch_message_preview(&msg.text, 96);
                                                let label =
                                                    format!("{} · {preview}", msg.entry_id);
                                                Button::new(("branch-msg", ix))
                                                    .small()
                                                    .w_full()
                                                    .child(wrapped_button_text(label))
                                                    .when(ix == selected, Button::primary)
                                                    .when(ix != selected, Button::ghost)
                                                    .on_click(cx.listener(
                                                        move |this, _: &ClickEvent, _w, cx| {
                                                            this.confirm_branch_pick(
                                                                entry_id.clone(),
                                                                cx,
                                                            );
                                                        },
                                                    ))
                                            },
                                        )),
                                ),
                        ),
                )
            })
            .when_some(self.login_picker.clone(), |parent, providers| {
                let selected = self.login_picker_selected;
                parent.child(
                    div()
                        .id("login-backdrop")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_4()
                        .bg(theme.overlay)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.close_login_picker(cx);
                        }))
                        .child(
                            v_flex()
                                .id("login-panel")
                                .w_full()
                                .max_w(px(400.))
                                .max_h(px(360.))
                                .gap_2()
                                .p_4()
                                .rounded_lg()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_xl()
                                .on_click(cx.listener(|_this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                }))
                                .child(
                                    Label::new("Login providers")
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD),
                                )
                                .child(
                                    v_flex()
                                        .w_full()
                                        .max_h(px(280.))
                                        .overflow_y_scrollbar()
                                        .gap_1()
                                        .children(providers.into_iter().enumerate().map(
                                            |(ix, provider)| {
                                                let provider_id = provider.id.clone();
                                                let status = if provider.authenticated {
                                                    "signed in"
                                                } else if provider.available {
                                                    "available"
                                                } else {
                                                    "unavailable"
                                                };
                                                let label = format!(
                                                    "{} · {status}",
                                                    provider.name
                                                );
                                                Button::new(("login-provider", ix))
                                                    .small()
                                                    .w_full()
                                                    .child(wrapped_button_text(label))
                                                    .disabled(!provider.available)
                                                    .when(ix == selected, Button::primary)
                                                    .when(ix != selected, Button::ghost)
                                                    .on_click(cx.listener(
                                                        move |this, _: &ClickEvent, _w, cx| {
                                                            this.confirm_login_provider(
                                                                provider_id.clone(),
                                                                cx,
                                                            );
                                                        },
                                                    ))
                                            },
                                        )),
                                ),
                        ),
                )
            })
            .into_any_element()
    }
}

// ── transcript rows ───────────────────────────────────────────────────────

pub(crate) fn next_subagent_subscription_level(
    current: &SubagentSubscriptionLevel,
) -> SubagentSubscriptionLevel {
    match current {
        SubagentSubscriptionLevel::Off => SubagentSubscriptionLevel::Progress,
        SubagentSubscriptionLevel::Progress => SubagentSubscriptionLevel::Events,
        SubagentSubscriptionLevel::Events | SubagentSubscriptionLevel::Unknown(_) => {
            SubagentSubscriptionLevel::Off
        }
    }
}

pub(crate) fn retained_subagent_selection(
    selected: Option<&str>,
    snapshots: &[serde_json::Value],
) -> Option<String> {
    match selected {
        Some(selected)
            if snapshots
                .iter()
                .any(|snapshot| subagent_snapshot_id(snapshot) == Some(selected)) =>
        {
            Some(selected.to_owned())
        }
        _ => None,
    }
}

pub(crate) fn subagent_event_needs_snapshot_refresh(
    payload: &serde_json::Value,
    snapshots: &[serde_json::Value],
) -> bool {
    let event_id = payload
        .get("id")
        .or_else(|| payload.get("subagentId"))
        .or_else(|| payload.get("subagent_id"))
        .and_then(serde_json::Value::as_str);
    event_id.map_or_else(
        || snapshots.is_empty(),
        |id| {
            !snapshots
                .iter()
                .any(|snapshot| subagent_snapshot_id(snapshot) == Some(id))
        },
    )
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
    let meta = subagent_snapshot_meta(snapshot);
    let description = snapshot
        .get("description")
        .or_else(|| snapshot.get("task"))
        .or_else(|| snapshot.get("assignment"))
        .map(compact_subagent_value)
        .unwrap_or_default();
    if description.is_empty() {
        format!("{meta} · {id}")
    } else {
        format!("{meta} · {id}: {description}")
    }
}

/// Primary label for an inspector agent row (id when present).
pub(crate) fn subagent_snapshot_title(snapshot: &serde_json::Value) -> String {
    subagent_snapshot_id(snapshot).map_or_else(|| "unknown".to_owned(), str::to_owned)
}

/// Secondary meta line: `agent · status` from wire fields only.
pub(crate) fn subagent_snapshot_meta(snapshot: &serde_json::Value) -> String {
    let agent = snapshot
        .get("agent")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("agent");
    let status = snapshot
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    format!("{agent} · {status}")
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
    match value {
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
    }
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
    proj.settle_orphaned_running_tools();
}

pub(crate) fn try_connect_omp(
    cwd: Option<PathBuf>,
    resume: Option<&Path>,
    persistence: &SessionPersistence,
) -> ConnectionResult {
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
    let omp_bin = discovered.path.clone();
    let omp_env = discovered.env.clone();

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

    let host_bridge_notice = register_host_bridge(&client)
        .err()
        .map(|error| format!("Host bridge unavailable: {error}"));

    let get_state = smol::block_on(async { client.send(RpcCommandBody::GetState).await });
    let avail = smol::block_on(async { client.send(RpcCommandBody::GetAvailableCommands).await });
    let _sub = smol::block_on(async {
        client
            .send(RpcCommandBody::SetSubagentSubscription {
                level: SubagentSubscriptionLevel::Events,
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
    let version_gate_notice = match (
        format_version_gate_notice(discovered.version),
        host_bridge_notice,
    ) {
        (Some(version), Some(host)) => Some(format!("{version} · {host}")),
        (Some(version), None) => Some(version),
        (None, Some(host)) => Some(host),
        (None, None) => None,
    };

    Ok(ConnectedSession {
        client,
        projection: proj,
        status,
        models,
        version_gate_notice,
        omp_bin,
        omp_env,
    })
}

pub(crate) fn format_version_gate_notice(version: OmpVersion) -> Option<String> {
    match version.support() {
        VersionSupport::Supported => None,
        VersionSupport::BelowMinimum | VersionSupport::Newer => Some(format!(
            "Pimiento was tested with omp {MIN_SUPPORTED}–{MAX_SUPPORTED}; you have {version} — unknown events will still render"
        )),
    }
}

pub(crate) fn context_high(context: Option<&serde_json::Value>) -> bool {
    context_percent(context).is_some_and(|percent| percent >= 80.0)
}

pub(crate) fn pretty_rpc_data(data: Option<&serde_json::Value>) -> Option<String> {
    data.filter(|value| !value.is_null())
        .and_then(|value| serde_json::to_string_pretty(value).ok())
}
