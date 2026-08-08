#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Pimiento — first live OMP session workspace.

use std::collections::HashSet;
use std::fmt::Write;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{
    App, ClickEvent, ClipboardItem, Context, ElementId, FollowMode, KeyDownEvent, ListAlignment,
    ListState, PathPromptOptions, Render, Task, Window, WindowOptions, div, list, prelude::*, px,
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
    todos::{actionable_todo_count, parse_todo_phases, todo_status_glyph},
    transcript::{ToolStatus, TranscriptEntry},
};
use serde::{Deserialize, Serialize};

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
const SLASH_COMMAND_VISIBLE_CAP: usize = 12;
const THINKING_LEVELS: &[&str] = &[
    "off", "minimal", "low", "medium", "high", "xhigh", "max", "auto",
];
const MAX_RECENT_SESSIONS: usize = 12;
const MAX_DISCOVERED_SESSIONS: usize = 24;
const SESSION_HEADER_PREFIX_BYTES: usize = 8192;
static PERSISTENCE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
struct SlashCommand {
    name: String,
    description: String,
    aliases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlashMenuState {
    Closed,
    Open,
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LauncherPhase {
    Visible,
    Connecting,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RecentSession {
    #[serde(rename = "sessionFile")]
    session_file: PathBuf,
    cwd: PathBuf,
    #[serde(default)]
    name: String,
    #[serde(rename = "lastUsed", default)]
    last_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionPersistence {
    root: PathBuf,
}

impl SessionPersistence {
    fn from_environment() -> Self {
        let home_override = std::env::var_os("PIMIENTO_HOME").map(PathBuf::from);
        let root = app_data_dir(home_override.as_deref(), home_dir().as_deref());
        Self { root }
    }

    #[cfg(test)]
    fn from_root(root: PathBuf) -> Self {
        Self { root }
    }

    fn last_session_path(&self) -> PathBuf {
        self.root.join("last-session")
    }

    fn recent_sessions_path(&self) -> PathBuf {
        self.root.join("recent.json")
    }

    fn load_last_session(&self) -> Option<PathBuf> {
        let raw = std::fs::read_to_string(self.last_session_path()).ok()?;
        let raw = raw.trim();
        (!raw.is_empty()).then(|| PathBuf::from(raw))
    }

    fn remember_last_session(&self, session_file: Option<&str>) {
        let Some(session_file) = session_file.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        let _ = write_persistence_file(&self.last_session_path(), session_file);
    }

    fn load_recent_sessions(&self) -> Vec<RecentSession> {
        let Ok(raw) = std::fs::read_to_string(self.recent_sessions_path()) else {
            return Vec::new();
        };
        parse_recent_sessions(&raw)
    }

    fn save_recent_sessions(&self, sessions: &[RecentSession]) -> std::io::Result<()> {
        let sessions = normalize_recent_sessions(sessions.to_vec());
        let contents = serde_json::to_string_pretty(&sessions).map_err(|error| {
            std::io::Error::other(format!("serialize recent sessions: {error}"))
        })?;
        write_persistence_file(&self.recent_sessions_path(), &contents)
    }

    fn remember_recent_session(
        &self,
        session_file: Option<&str>,
        cwd: Option<&Path>,
        name: Option<&str>,
    ) {
        let Some(session_file) = session_file.map(str::trim).filter(|file| !file.is_empty()) else {
            return;
        };
        let Some(cwd) = cwd.filter(|path| !path.as_os_str().is_empty()) else {
            return;
        };

        let mut sessions = self.load_recent_sessions();
        let last_used = next_last_used(&sessions);
        let record = RecentSession {
            session_file: PathBuf::from(session_file),
            cwd: cwd.to_owned(),
            name: name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map_or_else(|| default_session_name(cwd), str::to_owned),
            last_used,
        };
        sessions.retain(|existing| existing.session_file != record.session_file);
        sessions.push(record);
        let _ = self.save_recent_sessions(&sessions);
    }

    fn forget_session(&self, session_file: &Path) {
        let mut sessions = self.load_recent_sessions();
        let original_len = sessions.len();
        sessions.retain(|session| session.session_file != session_file);
        if sessions.len() != original_len {
            let _ = self.save_recent_sessions(&sessions);
        }
        if self.load_last_session().as_deref() == Some(session_file) {
            let _ = std::fs::remove_file(self.last_session_path());
        }
    }
}

#[derive(Debug, Clone)]
struct LauncherBootstrap {
    persistence: SessionPersistence,
    launcher_cwd: PathBuf,
    recent_sessions: Vec<RecentSession>,
    last_session: Option<PathBuf>,
}

#[allow(clippy::struct_excessive_bools)]
struct SessionView {
    projection: SessionProjection,
    client: Option<RpcClient>,
    composer: gpui::Entity<InputState>,
    model_search: gpui::Entity<InputState>,
    model_picker_open: bool,
    thinking_picker_open: bool,
    todo_panel_open: bool,
    slash_menu: SlashMenuState,
    slash_selected: usize,
    status_message: String,
    available_models: Vec<ModelChoice>,
    expanded_tools: HashSet<String>,
    clear_composer: bool,
    pending_composer_value: Option<String>,
    clear_model_search: bool,
    /// Virtualized transcript list (GPUI `ListState`, bottom-aligned chat).
    transcript_list: ListState,
    last_transcript_len: usize,
    /// Count of rows appended while the user was scrolled away from the tail.
    unread_below: usize,
    _subscriptions: Vec<gpui::Subscription>,
    pump: Option<Task<()>>,
    persistence: SessionPersistence,
    session_cwd: Option<PathBuf>,
    launcher_cwd: PathBuf,
    recent_sessions: Vec<RecentSession>,
    last_session: Option<PathBuf>,
    launcher_phase: LauncherPhase,
    launcher_error: Option<String>,
}

impl SessionView {
    fn new(
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

        let launcher_phase = if client.is_none() {
            LauncherPhase::Visible
        } else {
            LauncherPhase::Hidden
        };
        let mut view = Self {
            projection: initial_projection,
            client,
            composer,
            model_search,
            model_picker_open: false,
            thinking_picker_open: false,
            todo_panel_open: true,
            slash_menu: SlashMenuState::Closed,
            slash_selected: 0,
            status_message: status,
            available_models,
            expanded_tools: HashSet::new(),
            clear_composer: false,
            pending_composer_value: None,
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
        view.start_catalog_load(cx);
        view
    }

    fn start_event_pump(&mut self, client: &RpcClient, cx: &mut Context<Self>) {
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
                    cx.notify();
                });
            }
        }));
    }

    fn begin_connection(
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
        self.client.take();
        self.pump.take();
        self.launcher_phase = LauncherPhase::Connecting;
        if launcher_mode {
            self.launcher_cwd.clone_from(&cwd);
            self.launcher_error = None;
            self.session_cwd = None;
            self.projection = SessionProjection::new();
            self.available_models.clear();
            self.model_picker_open = false;
            self.thinking_picker_open = false;
            self.expanded_tools.clear();
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

    fn finish_connection(
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
                self.status_message = status;
                self.client = Some(client.clone());
                self.session_cwd = Some(cwd);
                self.launcher_phase = LauncherPhase::Hidden;
                self.launcher_error = None;
                self.transcript_list.reset(self.projection.transcript.len());
                self.transcript_list.set_follow_mode(FollowMode::Tail);
                self.last_transcript_len = self.projection.transcript.len();
                self.unread_below = 0;
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

    fn choose_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn set_launcher_cwd(&mut self, cwd: PathBuf, cx: &mut Context<Self>) {
        self.launcher_cwd = cwd;
        self.launcher_error = None;
        self.refresh_launcher_sessions();
        cx.notify();
    }

    fn refresh_launcher_sessions(&mut self) {
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

    fn return_to_launcher(&mut self, cx: &mut Context<Self>) {
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
        self.available_models.clear();
        self.expanded_tools.clear();
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
        if self.model_picker_open {
            self.thinking_picker_open = false;
        }
        if !self.model_picker_open {
            self.clear_model_search = true;
        }
        cx.notify();
    }

    fn close_thinking_picker(&mut self) {
        self.thinking_picker_open = false;
    }

    fn toggle_todo_panel(&mut self, cx: &mut Context<Self>) {
        self.todo_panel_open = !self.todo_panel_open;
        cx.notify();
    }

    fn toggle_thinking_picker(&mut self, cx: &mut Context<Self>) {
        self.thinking_picker_open = !self.thinking_picker_open;
        if self.thinking_picker_open {
            self.model_picker_open = false;
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

    fn refresh_state(&mut self, cx: &mut Context<Self>) {
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

    fn set_thinking_level(&mut self, level: &'static str, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.close_thinking_picker();
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

    fn toggle_fast_mode(&mut self, cx: &mut Context<Self>) {
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
        match event {
            InputEvent::Change => {
                if self.slash_menu == SlashMenuState::Dismissed {
                    self.slash_menu = SlashMenuState::Closed;
                }
                self.update_slash_menu(cx);
                cx.notify();
            }
            InputEvent::PressEnter {
                secondary: false,
                shift: false,
            } => {
                let text = self.composer.read(cx).value().to_string();
                if text.trim().is_empty() {
                    return;
                }
                let matches = self.filtered_slash_commands(&text);
                if composer_enter_action(self.slash_menu == SlashMenuState::Open, matches.len())
                    == ComposerEnterAction::AcceptCompletion
                {
                    if let Some(command) = matches.get(self.slash_selected) {
                        let command = command.clone();
                        self.accept_slash_command(&command, cx);
                    }
                    return;
                }
                let Some(client) = self.client.clone() else {
                    return;
                };

                let steer = composer_uses_steer(&self.projection.run_phase);
                self.projection.push_user_message(text.clone());
                self.close_slash_menu();
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
            _ => {}
        }
    }

    fn filtered_slash_commands(&self, text: &str) -> Vec<SlashCommand> {
        let commands = parse_slash_commands(self.projection.available_commands_raw.as_ref());
        filter_slash_commands(&commands, text.trim_start())
    }

    fn update_slash_menu(&mut self, cx: &Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        if self.slash_menu == SlashMenuState::Dismissed || !slash_draft_is_open(&text) {
            self.close_slash_menu();
            return;
        }

        self.slash_menu = SlashMenuState::Open;
        let match_count = self.filtered_slash_commands(&text).len();
        self.slash_selected = self.slash_selected.min(match_count.saturating_sub(1));
    }

    fn close_slash_menu(&mut self) {
        self.slash_menu = SlashMenuState::Closed;
        self.slash_selected = 0;
    }

    fn accept_slash_command(&mut self, command: &SlashCommand, cx: &mut Context<Self>) {
        self.pending_composer_value = Some(slash_completion_text(command));
        self.close_slash_menu();
        cx.notify();
    }

    fn handle_slash_key(
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

    fn can_send(&self) -> bool {
        self.client.is_some() && phase_allows_send(&self.projection.run_phase)
    }

    fn can_restart(&self) -> bool {
        matches!(self.projection.run_phase, RunPhase::Dead)
    }

    fn restart_resume_path(&self) -> Option<PathBuf> {
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

    fn restart_cwd(&self, resume: Option<&Path>) -> PathBuf {
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

    fn do_restart(&mut self, window: &Window, cx: &mut Context<Self>) {
        let resume = self.restart_resume_path();
        let cwd = self.restart_cwd(resume.as_deref());
        self.begin_connection(window, cwd, resume, false, cx);
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

    #[allow(clippy::too_many_lines)] // Launcher layout remains easier to audit as one declarative block.
    fn render_launcher(&self, _window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
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
            .gap_3()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(720.))
                    .p_5()
                    .gap_3()
                    .rounded_md()
                    .bg(theme.muted)
                    .border_1()
                    .border_color(theme.border)
                    .child(Label::new("Start a Pimiento session").text_lg())
                    .child(
                        v_flex()
                            .gap_1()
                            .child(Label::new("Working directory").text_sm())
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .rounded_sm()
                                    .bg(theme.background)
                                    .child(cwd),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("choose-working-directory")
                                    .label("Choose directory…")
                                    .primary()
                                    .disabled(connecting)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.choose_directory(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("start-working-directory")
                                    .label("Start here")
                                    .disabled(connecting)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        let cwd = this.launcher_cwd.clone();
                                        this.begin_connection(window, cwd, None, true, cx);
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
                                .primary()
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
                                .gap_1()
                                .child(Label::new("Sessions for this directory").text_sm())
                                .children(recents.into_iter().enumerate().map(|(ix, recent)| {
                                    let cwd = recent.cwd.clone();
                                    let resume = recent.session_file.clone();
                                    let label = if recent.name.trim().is_empty() {
                                        recent.cwd.display().to_string()
                                    } else {
                                        format!("{}  —  {}", recent.name, recent.cwd.display())
                                    };
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
                                        ))
                                })),
                        )
                    }),
            )
            .into_any_element()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlashKeyAction {
    Up,
    Down,
    Accept,
    Dismiss,
}

fn slash_key_action(key: &str) -> Option<SlashKeyAction> {
    match key {
        "up" | "arrowup" => Some(SlashKeyAction::Up),
        "down" | "arrowdown" => Some(SlashKeyAction::Down),
        "enter" | "return" => Some(SlashKeyAction::Accept),
        "escape" | "esc" => Some(SlashKeyAction::Dismiss),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComposerEnterAction {
    AcceptCompletion,
    Send,
}

fn composer_enter_action(menu_open: bool, match_count: usize) -> ComposerEnterAction {
    if menu_open && match_count > 0 {
        ComposerEnterAction::AcceptCompletion
    } else {
        ComposerEnterAction::Send
    }
}

fn slash_draft_is_open(text: &str) -> bool {
    let Some(command) = text.trim_start().strip_prefix('/') else {
        return false;
    };
    command
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn normalize_slash_name(name: &str) -> Option<String> {
    let name = name.trim().trim_start_matches('/');
    (!name.is_empty()).then(|| format!("/{name}"))
}

fn parse_slash_command(raw: &serde_json::Value) -> Option<SlashCommand> {
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

fn parse_slash_commands(raw: Option<&serde_json::Value>) -> Vec<SlashCommand> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let entries: &[serde_json::Value] = raw
        .as_array()
        .or_else(|| raw.get("commands").and_then(serde_json::Value::as_array))
        .map_or(&[], Vec::as_slice);
    entries.iter().filter_map(parse_slash_command).collect()
}

fn slash_command_matches(command: &SlashCommand, query: &str) -> bool {
    let query = query.trim_start().to_ascii_lowercase();
    command.name.to_ascii_lowercase().starts_with(&query)
        || command
            .aliases
            .iter()
            .any(|alias| alias.to_ascii_lowercase().starts_with(&query))
}

fn filter_slash_commands(commands: &[SlashCommand], query: &str) -> Vec<SlashCommand> {
    commands
        .iter()
        .filter(|command| slash_command_matches(command, query))
        .take(SLASH_COMMAND_VISIBLE_CAP)
        .cloned()
        .collect()
}

fn slash_completion_text(command: &SlashCommand) -> String {
    format!("{} ", command.name)
}

// ── render ────────────────────────────────────────────────────────────────

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
        let thinking_button_label = thinking_label(self.projection.state.thinking.as_ref())
            .map_or_else(|| "think:?".to_owned(), |level| format!("think:{level}"));
        let fast_button_label = fast_mode_label(
            self.projection.state.fast_mode_enabled,
            self.projection.state.fast_mode_active,
        );
        let todo_phases = parse_todo_phases(self.projection.todos_raw.as_ref());
        let todo_actionable = actionable_todo_count(&todo_phases);
        let todo_total = todo_phases
            .iter()
            .map(|phase| phase.tasks.len())
            .sum::<usize>();
        let todo_button_label = if todo_phases.is_empty() {
            "Todos".to_owned()
        } else if self.todo_panel_open {
            format!("Todos ({todo_actionable}/{todo_total}) v")
        } else {
            format!("Todos ({todo_actionable}/{todo_total}) >")
        };
        let show_todo_panel = self.todo_panel_open && !todo_phases.is_empty();
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

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .capture_key_down(cx.listener(|this, event, window, cx| {
                let handled =
                    this.handle_dialog_key(event, cx) || this.handle_slash_key(event, window, cx);
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
                                    )
                                    .child(
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
                                    .child(
                                        Button::new("fast-mode")
                                            .label(fast_button_label)
                                            .small()
                                            .ghost()
                                            .disabled(!can_pick)
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _window, cx| {
                                                    this.toggle_fast_mode(cx);
                                                },
                                            )),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("todo-panel-toggle")
                                            .label(todo_button_label)
                                            .small()
                                            .ghost()
                                            .disabled(todo_phases.is_empty())
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _window, cx| {
                                                    this.toggle_todo_panel(cx);
                                                },
                                            )),
                                    )
                                    .child(
                                        Button::new("sessions-launcher")
                                            .label("Sessions")
                                            .small()
                                            .ghost()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _window, cx| {
                                                    this.return_to_launcher(cx);
                                                },
                                            )),
                                    )
                                    .child(
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
                                .children(THINKING_LEVELS.iter().enumerate().map(|(ix, level)| {
                                    let level = *level;
                                    Button::new(("thinking-choice", ix))
                                        .label(level)
                                        .ghost()
                                        .small()
                                        .on_click(window.listener_for(
                                            &view,
                                            move |this, _, _window, cx| {
                                                this.set_thinking_level(level, cx);
                                            },
                                        ))
                                })),
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
                                    |e| render_entry(ix, e, &this.expanded_tools, cx),
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
            .when(show_todo_panel, |parent| {
                parent.child(
                    v_flex()
                        .w_full()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .bg(theme.secondary)
                        .border_t_1()
                        .border_color(theme.border)
                        .max_h(px(220.))
                        .overflow_y_scrollbar()
                        .children(
                            todo_phases
                                .into_iter()
                                .enumerate()
                                .map(|(phase_ix, phase)| {
                                    v_flex()
                                        .w_full()
                                        .gap_1()
                                        .child(
                                            Label::new(phase.name.clone())
                                                .text_xs()
                                                .text_color(theme.muted_foreground),
                                        )
                                        .children(phase.tasks.into_iter().enumerate().map(
                                            |(task_ix, task)| {
                                                let glyph = todo_status_glyph(&task.status);
                                                let mut line = format!("{glyph} {}", task.content);
                                                if let Some(blocker) = task.blocker.as_deref() {
                                                    let _ = write!(line, " — {blocker}");
                                                }
                                                div()
                                                    .id(("todo-task", phase_ix * 1000 + task_ix))
                                                    .child(Label::new(line).text_sm())
                                            },
                                        ))
                                }),
                        ),
                )
            })
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
                                                    .when(ix == self.slash_selected, |button| {
                                                        button.bg(theme.secondary)
                                                    })
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
                                                                Label::new(command.name.clone())
                                                                    .text_sm(),
                                                            )
                                                            .when(
                                                                !command.description.is_empty(),
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
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.do_restart(window, cx);
                            }),
                        ))
                    }),
            )
            .into_any_element()
    }
}

// ── transcript rows ───────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)] // GPUI render fns are declaratively dense; splitting hurts readability.
fn render_entry(
    row_ix: usize,
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
            .child(
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
                                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                            })
                    }),
            )
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

fn code_block_copy_id(row_ix: usize, lang: Option<&str>, code: &str) -> ElementId {
    let mut hasher = DefaultHasher::new();
    (row_ix, lang, code).hash(&mut hasher);
    ElementId::Name(format!("code-block-copy-{}", hasher.finish()).into())
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

fn app_data_dir(home_override: Option<&Path>, home: Option<&Path>) -> PathBuf {
    if let Some(path) = home_override.filter(|path| !path.as_os_str().is_empty()) {
        return path.to_owned();
    }
    home.map_or_else(|| PathBuf::from(".pimiento"), |path| path.join(".pimiento"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn omp_agent_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_DIR") {
        return Some(PathBuf::from(dir));
    }
    home_dir().map(|home| home.join(".omp").join("agent"))
}

fn omp_sessions_root() -> Option<PathBuf> {
    omp_agent_dir().map(|dir| dir.join("sessions"))
}

fn encode_relative_session_dir_name(prefix: &str, relative: &str) -> String {
    let encoded = relative.replace(['/', '\\', ':'], "-");
    if encoded.is_empty() {
        prefix.to_owned()
    } else if prefix.ends_with('-') {
        format!("{prefix}{encoded}")
    } else {
        format!("{prefix}-{encoded}")
    }
}

fn encode_legacy_absolute_session_dir_name(cwd: &Path) -> String {
    let trimmed = cwd
        .to_string_lossy()
        .trim_start_matches(['/', '\\'])
        .replace(['/', '\\', ':'], "-");
    format!("--{trimmed}--")
}

fn encode_omp_session_dir_name(cwd: &Path, home: Option<&Path>, temp_root: &Path) -> String {
    if let Some(home) = home
        && let Ok(relative) = cwd.strip_prefix(home)
    {
        return encode_relative_session_dir_name("-", &relative.to_string_lossy());
    }
    if let Ok(relative) = cwd.strip_prefix(temp_root) {
        return encode_relative_session_dir_name("-tmp", &relative.to_string_lossy());
    }
    encode_legacy_absolute_session_dir_name(cwd)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OmpSessionHeader {
    id: String,
    cwd: Option<PathBuf>,
    title: Option<String>,
    first_user_message: Option<String>,
}

fn extract_message_text(content: &serde_json::Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    let Some(parts) = content.as_array() else {
        return String::new();
    };
    parts
        .iter()
        .filter_map(|part| {
            (part.get("type").and_then(|v| v.as_str()) == Some("text"))
                .then(|| part.get("text").and_then(|v| v.as_str()).map(str::to_owned))
                .flatten()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_omp_session_header_prefix(raw: &str) -> Option<OmpSessionHeader> {
    let mut id = None;
    let mut cwd = None;
    let mut title = None;
    let mut first_user_message = None;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match value.get("type").and_then(|v| v.as_str()) {
            Some("session") => {
                id = value
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .or(id);
                cwd = value
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .or(cwd);
                title = value
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or(title);
            }
            Some("title") => {
                title = value
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or(title);
            }
            Some("message") => {
                if first_user_message.is_some() {
                    continue;
                }
                let Some(message) = value.get("message") else {
                    continue;
                };
                if message.get("role").and_then(|v| v.as_str()) != Some("user") {
                    continue;
                }
                let text = message
                    .get("content")
                    .map(extract_message_text)
                    .unwrap_or_default();
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    first_user_message = Some(text.chars().take(96).collect());
                }
            }
            _ => {}
        }
    }
    let id = id?;
    Some(OmpSessionHeader {
        id,
        cwd,
        title,
        first_user_message,
    })
}

fn read_omp_session_header(path: &Path) -> Option<OmpSessionHeader> {
    let file = std::fs::File::open(path).ok()?;
    let mut limited = file.take(SESSION_HEADER_PREFIX_BYTES as u64);
    let mut raw = String::new();
    std::io::Read::read_to_string(&mut limited, &mut raw).ok()?;
    parse_omp_session_header_prefix(&raw)
}

fn mtime_unix_seconds(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs())
}

fn discover_omp_sessions_for_cwd(
    cwd: &Path,
    sessions_root: Option<&Path>,
    home: Option<&Path>,
    temp_root: &Path,
) -> Vec<RecentSession> {
    let Some(sessions_root) = sessions_root else {
        return Vec::new();
    };
    let dir_name = encode_omp_session_dir_name(cwd, home, temp_root);
    let session_dir = sessions_root.join(dir_name);
    let Ok(entries) = std::fs::read_dir(&session_dir) else {
        return Vec::new();
    };
    let mut discovered = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(header) = read_omp_session_header(&path) else {
            continue;
        };
        if let Some(session_cwd) = header.cwd.as_deref()
            && session_cwd != cwd
        {
            continue;
        }
        let name = header
            .title
            .filter(|title| !title.trim().is_empty())
            .or(header.first_user_message)
            .unwrap_or_else(|| default_session_name(cwd));
        let last_used = mtime_unix_seconds(&path);
        discovered.push(RecentSession {
            session_file: path,
            cwd: cwd.to_owned(),
            name,
            last_used,
        });
    }
    discovered.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| a.session_file.cmp(&b.session_file))
    });
    discovered.truncate(MAX_DISCOVERED_SESSIONS);
    discovered
}

fn collect_launcher_sessions(
    persistence: &SessionPersistence,
    cwd: &Path,
    sessions_root: Option<&Path>,
    home: Option<&Path>,
    temp_root: &Path,
) -> Vec<RecentSession> {
    let mut sessions = discover_omp_sessions_for_cwd(cwd, sessions_root, home, temp_root);
    for remembered in persistence.load_recent_sessions() {
        if remembered.cwd == cwd {
            sessions.push(remembered);
        }
    }
    sessions.retain(|session| session.cwd == cwd && session.session_file.exists());
    // Prefer richer names when duplicates collide on session_file
    sessions.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| a.session_file.cmp(&b.session_file))
    });
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for session in sessions {
        if seen.insert(session.session_file.clone()) {
            deduped.push(session);
        }
    }
    deduped.truncate(MAX_DISCOVERED_SESSIONS);
    deduped
}

fn write_persistence_file(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let Some(file_name) = path.file_name() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "persistence path has no file name",
        ));
    };
    let nonce = PERSISTENCE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(
        ".{}.tmp-{}-{nonce}",
        file_name.to_string_lossy(),
        std::process::id(),
    ));
    if let Err(error) = std::fs::write(&temporary, contents) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    match std::fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            let _ = std::fs::remove_file(&temporary);
            std::fs::write(path, contents).map_err(|write_error| {
                std::io::Error::new(
                    write_error.kind(),
                    format!("rename failed ({rename_error}); fallback write failed: {write_error}"),
                )
            })
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RecentSessionsFile {
    List(Vec<RecentSession>),
    Wrapped { sessions: Vec<RecentSession> },
}

fn normalize_recent_sessions(mut sessions: Vec<RecentSession>) -> Vec<RecentSession> {
    sessions.retain(|session| {
        !session.session_file.as_os_str().is_empty() && !session.cwd.as_os_str().is_empty()
    });
    sessions.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| a.session_file.cmp(&b.session_file))
    });
    let mut seen = HashSet::new();
    sessions.retain(|session| seen.insert(session.session_file.clone()));
    sessions.truncate(MAX_RECENT_SESSIONS);
    sessions
}

fn parse_recent_sessions(raw: &str) -> Vec<RecentSession> {
    match serde_json::from_str::<RecentSessionsFile>(raw) {
        Ok(RecentSessionsFile::List(sessions) | RecentSessionsFile::Wrapped { sessions }) => {
            normalize_recent_sessions(sessions)
        }
        Err(_) => Vec::new(),
    }
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn next_last_used(sessions: &[RecentSession]) -> u64 {
    let now = current_unix_seconds();
    let previous = sessions
        .iter()
        .map(|session| session.last_used)
        .max()
        .unwrap_or(0);
    now.max(previous.saturating_add(1))
}

fn default_session_name(cwd: &Path) -> String {
    cwd.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| cwd.display().to_string())
}

fn projection_session_name(projection: &SessionProjection, cwd: &Path) -> String {
    projection
        .state
        .state
        .as_ref()
        .and_then(|state| state.get("sessionName"))
        .and_then(|name| name.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or_else(|| default_session_name(cwd), str::to_owned)
}

fn resolve_launcher_path(path: &Path, current_dir: Option<&Path>) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        return None;
    }
    if path.is_absolute() {
        return Some(path.to_owned());
    }
    current_dir.map(|base| base.join(path))
}

fn initial_launcher_directory(
    cwd_override: Option<&Path>,
    recent: &[RecentSession],
    current: Option<PathBuf>,
) -> Option<PathBuf> {
    let current = current.filter(|path| path.is_absolute());
    let current_dir = current.as_deref();
    cwd_override
        .and_then(|path| resolve_launcher_path(path, current_dir))
        .or_else(|| {
            recent
                .iter()
                .find_map(|session| resolve_launcher_path(&session.cwd, current_dir))
        })
        .or(current)
}

fn auto_connect_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.trim() == "1")
}

fn latest_resume_path(
    persistence: &SessionPersistence,
    recent: &[RecentSession],
) -> Option<PathBuf> {
    persistence
        .load_last_session()
        .or_else(|| recent.first().map(|session| session.session_file.clone()))
}

fn try_connect_omp(
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
    let messages = smol::block_on(async { client.send(RpcCommandBody::GetMessages).await });
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
    if let Ok(r) = &messages
        && r.success
        && let Some(data) = &r.data
    {
        proj.hydrate_messages(data);
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

fn fast_mode_label(enabled: Option<bool>, active: Option<bool>) -> &'static str {
    match (enabled, active) {
        (_, Some(true)) => "fast:active",
        (Some(true), _) => "fast:on",
        (Some(false), _) => "fast:off",
        (None, _) => "fast:?",
    }
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
    let persistence = SessionPersistence::from_environment();
    let remembered = persistence.load_recent_sessions();
    let last_session = persistence
        .load_last_session()
        .filter(|resume| resume.exists());
    let cwd_override = std::env::var_os("PIMIENTO_CWD").map(PathBuf::from);
    let initial_cwd = initial_launcher_directory(
        cwd_override.as_deref(),
        &remembered,
        std::env::current_dir().ok(),
    )
    .unwrap_or_else(|| PathBuf::from("."));
    let recent = collect_launcher_sessions(
        &persistence,
        &initial_cwd,
        omp_sessions_root().as_deref(),
        home_dir().as_deref(),
        std::env::temp_dir().as_path(),
    );
    let auto_connect = auto_connect_enabled(std::env::var("PIMIENTO_AUTO_CONNECT").ok().as_deref());
    let auto_resume = auto_connect
        .then(|| latest_resume_path(&persistence, &recent))
        .flatten();

    gpui_platform::application().run(move |cx| {
        gpui_component::init(cx);
        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| {
                    SessionView::new(
                        window,
                        cx,
                        None,
                        "Choose a working directory to begin".to_owned(),
                        SessionProjection::new(),
                        Vec::new(),
                        LauncherBootstrap {
                            persistence: persistence.clone(),
                            launcher_cwd: initial_cwd.clone(),
                            recent_sessions: recent.clone(),
                            last_session: last_session.clone(),
                        },
                    )
                });
                if auto_connect {
                    let cwd = initial_cwd.clone();
                    let resume = auto_resume.clone();
                    view.update(cx, |this, cx| {
                        this.begin_connection(window, cwd, resume, true, cx);
                    });
                }
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
    fn thinking_levels_are_non_empty_and_include_auto() {
        assert!(!THINKING_LEVELS.is_empty());
        assert!(THINKING_LEVELS.contains(&"auto"));
        assert!(THINKING_LEVELS.contains(&"max"));
    }

    #[test]
    fn fast_mode_label_distinguishes_off_on_and_active() {
        assert_eq!(fast_mode_label(Some(false), Some(false)), "fast:off");
        assert_eq!(fast_mode_label(Some(true), Some(false)), "fast:on");
        assert_eq!(fast_mode_label(Some(true), Some(true)), "fast:active");
        assert_eq!(fast_mode_label(Some(false), Some(true)), "fast:active");
        assert_eq!(fast_mode_label(None, None), "fast:?");
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
    fn code_block_copy_ids_are_stable_and_distinct() {
        let id = code_block_copy_id(3, Some("rust"), "fn main() {}");
        assert_eq!(id, code_block_copy_id(3, Some("rust"), "fn main() {}"));
        assert_ne!(id, code_block_copy_id(4, Some("rust"), "fn main() {}"));
        assert_ne!(id, code_block_copy_id(3, Some("python"), "fn main() {}"));
        assert_ne!(id, code_block_copy_id(3, Some("rust"), "print()"));
        assert_ne!(id, code_block_copy_id(3, None, "fn main() {}"));
    }

    #[test]
    fn slash_commands_normalize_names_and_wrapper_shapes() {
        let raw = serde_json::json!({
            "commands": [
                {
                    "name": "help",
                    "description": " Show help ",
                    "aliases": ["h", "/help", "h"]
                },
                "status"
            ]
        });
        let commands = parse_slash_commands(Some(&raw));
        assert_eq!(
            commands,
            vec![
                SlashCommand {
                    name: "/help".into(),
                    description: "Show help".into(),
                    aliases: vec!["/h".into()],
                },
                SlashCommand {
                    name: "/status".into(),
                    description: String::new(),
                    aliases: Vec::new(),
                },
            ]
        );

        let array = serde_json::json!([{ "name": "/quit", "aliases": ["q"] }]);
        assert_eq!(parse_slash_commands(Some(&array))[0].name, "/quit");
    }

    #[test]
    fn slash_filter_matches_aliases_and_caps_results() {
        let commands = (0..(SLASH_COMMAND_VISIBLE_CAP + 2))
            .map(|ix| SlashCommand {
                name: format!("/command-{ix}"),
                description: String::new(),
                aliases: if ix == 0 {
                    vec!["/go".into()]
                } else {
                    Vec::new()
                },
            })
            .collect::<Vec<_>>();
        let matches = filter_slash_commands(&commands, "/GO");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "/command-0");

        let capped = filter_slash_commands(&commands, "/");
        assert_eq!(capped.len(), SLASH_COMMAND_VISIBLE_CAP);
    }

    #[test]
    fn slash_draft_predicate_requires_a_slash_only_draft() {
        assert!(slash_draft_is_open("/"));
        assert!(slash_draft_is_open("  /build-2"));
        assert!(!slash_draft_is_open("build /"));
        assert!(!slash_draft_is_open("/build "));
        assert!(!slash_draft_is_open("/build.task"));
    }

    #[test]
    fn slash_enter_accepts_only_when_menu_has_matches() {
        assert_eq!(
            composer_enter_action(true, 1),
            ComposerEnterAction::AcceptCompletion
        );
        assert_eq!(composer_enter_action(true, 0), ComposerEnterAction::Send);
        assert_eq!(composer_enter_action(false, 1), ComposerEnterAction::Send);
    }

    #[test]
    fn slash_completion_uses_primary_name_with_trailing_space() {
        let command = SlashCommand {
            name: "/help".into(),
            description: String::new(),
            aliases: vec!["/h".into()],
        };
        assert_eq!(slash_completion_text(&command), "/help ");
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
    fn app_data_dir_prefers_override_over_home() {
        assert_eq!(
            app_data_dir(
                Some(Path::new("/tmp/pimiento-override")),
                Some(Path::new("/tmp/user")),
            ),
            PathBuf::from("/tmp/pimiento-override")
        );
        assert_eq!(
            app_data_dir(None, Some(Path::new("/tmp/user"))),
            PathBuf::from("/tmp/user/.pimiento")
        );
    }

    #[test]
    fn recent_session_json_uses_wire_field_names() {
        let record = RecentSession {
            session_file: PathBuf::from("/tmp/session.jsonl"),
            cwd: PathBuf::from("/tmp/worktree"),
            name: "worktree".to_owned(),
            last_used: 42,
        };
        let value = serde_json::to_value(record).expect("recent session serializes");
        assert_eq!(value["sessionFile"], "/tmp/session.jsonl");
        assert_eq!(value["lastUsed"], 42);
        assert!(value.get("session_file").is_none());
    }

    #[test]
    fn recent_session_parser_tolerates_bad_json_and_wrapped_files() {
        assert!(parse_recent_sessions("not json").is_empty());
        let wrapped = serde_json::json!({
            "sessions": [{
                "sessionFile": "/tmp/session.jsonl",
                "cwd": "/tmp/worktree",
                "name": "worktree",
                "lastUsed": 7
            }]
        });
        let parsed = parse_recent_sessions(&wrapped.to_string());
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "worktree");
    }

    #[test]
    fn recent_sessions_sort_deduplicate_and_cap() {
        let mut sessions = (0..(MAX_RECENT_SESSIONS + 2))
            .map(|ix| RecentSession {
                session_file: PathBuf::from(format!("/tmp/session-{ix}.jsonl")),
                cwd: PathBuf::from(format!("/tmp/worktree-{ix}")),
                name: ix.to_string(),
                last_used: ix as u64,
            })
            .collect::<Vec<_>>();
        sessions.push(RecentSession {
            session_file: PathBuf::from("/tmp/session-0.jsonl"),
            cwd: PathBuf::from("/tmp/new-worktree"),
            name: "new".to_owned(),
            last_used: 999,
        });
        let normalized = normalize_recent_sessions(sessions);
        assert_eq!(normalized.len(), MAX_RECENT_SESSIONS);
        assert_eq!(normalized[0].name, "new");
        assert_eq!(
            normalized
                .iter()
                .filter(|session| session.session_file == Path::new("/tmp/session-0.jsonl"))
                .count(),
            1
        );
    }

    #[test]
    fn launcher_directory_precedence_is_override_recent_then_current() {
        let recent = vec![RecentSession {
            session_file: PathBuf::from("/tmp/session.jsonl"),
            cwd: PathBuf::from("/tmp/recent"),
            name: "recent".to_owned(),
            last_used: 1,
        }];
        assert_eq!(
            initial_launcher_directory(
                Some(Path::new("/tmp/override")),
                &recent,
                Some(PathBuf::from("/tmp/current")),
            ),
            Some(PathBuf::from("/tmp/override"))
        );
        assert_eq!(
            initial_launcher_directory(None, &recent, Some(PathBuf::from("/tmp/current"))),
            Some(PathBuf::from("/tmp/recent"))
        );
        assert_eq!(
            initial_launcher_directory(None, &[], Some(PathBuf::from("/tmp/current"))),
            Some(PathBuf::from("/tmp/current"))
        );
    }

    #[test]
    fn encode_omp_session_dir_name_matches_home_relative_layout() {
        let home = Path::new("/Users/idan");
        let cwd = Path::new("/Users/idan/Developer/Projects/Pimiento");
        assert_eq!(
            encode_omp_session_dir_name(cwd, Some(home), Path::new("/tmp")),
            "-Developer-Projects-Pimiento"
        );
    }

    #[test]
    fn parse_omp_session_header_reads_title_and_first_user() {
        let raw = concat!(
            r#"{"type":"title","v":1,"title":"Proceed with M2 implementation"}"#,
            "
",
            r#"{"type":"session","version":3,"id":"abc","timestamp":"2026-08-06T22:47:47.681Z","cwd":"/Users/idan/Developer/Projects/Pimiento","title":"Proceed with M2 implementation"}"#,
            "
",
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"hello world"}]}}"#,
            "
",
        );
        let header = parse_omp_session_header_prefix(raw).expect("header");
        assert_eq!(header.id, "abc");
        assert_eq!(
            header.cwd.as_deref(),
            Some(Path::new("/Users/idan/Developer/Projects/Pimiento"))
        );
        assert_eq!(
            header.title.as_deref(),
            Some("Proceed with M2 implementation")
        );
        assert_eq!(header.first_user_message.as_deref(), Some("hello world"));
    }

    #[test]
    fn session_persistence_roundtrip_uses_home_root() {
        let root = std::env::temp_dir().join(format!(
            "pimiento-persistence-{}-{}",
            std::process::id(),
            current_unix_seconds()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp persistence root");
        let persistence = SessionPersistence::from_root(root.clone());
        persistence.remember_last_session(Some("/tmp/session.jsonl"));
        persistence.remember_recent_session(
            Some("/tmp/session.jsonl"),
            Some(Path::new("/tmp/work")),
            Some("work"),
        );
        assert_eq!(
            persistence.load_last_session(),
            Some(PathBuf::from("/tmp/session.jsonl"))
        );
        let recent = persistence.load_recent_sessions();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].name, "work");
        persistence.forget_session(Path::new("/tmp/session.jsonl"));
        assert!(persistence.load_recent_sessions().is_empty());
        assert!(persistence.load_last_session().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn auto_connect_requires_explicit_one() {
        assert!(auto_connect_enabled(Some("1")));
        assert!(!auto_connect_enabled(Some("true")));
        assert!(!auto_connect_enabled(Some("0")));
        assert!(!auto_connect_enabled(None));
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
