//! Pure, UI-free session projection reducer.
//!
//! Consumes decoded [`IncomingFrame`]s from `omp-rpc-client` and updates a
//! deterministic snapshot ([`SessionProjection`]) suitable for `insta`
//! replay tests and for GPUI to project onto. See `docs/architecture.md`.
//!
//! Invariants:
//!
//! * The reducer is pure: it never reads wall-clock time. Durations only
//!   come from wire payloads. `run_phase` transitions are driven by frames,
//!   with the sole exception of `mark_restarting` / `mark_dead`, which are
//!   invoked by the supervisor when the child process itself is affected.
//! * Unknown wire data is always rendered as a raw [`TranscriptEntry::Unknown`]
//!   row — never dropped, never panicked on.
//! * Concurrent tool calls are addressed by `toolCallId`; the transcript
//!   remains append-mostly and lifecycle updates mutate the correct row
//!   regardless of interleaving.
//! * Every state field promoted onto [`RuntimeState`] is preserved losslessly
//!   under [`RuntimeState::state`] so no upstream field is ever lost.

use std::collections::BTreeMap;

use omp_rpc_client::frames::{
    AgentEndFrame, AssistantMessageEventKind, ExtensionUiMethod, ExtensionUiRequestFrame,
    IncomingFrame, IncomingFrameKind, MessageUpdateFrame, NoticeFrame, NoticeLevel,
    PromptResultFrame, RpcFrameErrorFrame, ThinkingLevelChangedFrame, ToolExecutionEndFrame,
    ToolExecutionStartFrame, ToolExecutionUpdateFrame,
};
use serde::Serialize;
use serde_json::Value;

use crate::transcript::{
    BoundedText, CompactionPhase, Markdown, ToolCall, ToolStatus, TranscriptEntry,
};

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Doctrine §0.5: four state qualities, rendered distinctly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StateQuality {
    /// No `get_state` snapshot has ever been hydrated. Say "unknown", never
    /// fabricate a default.
    #[default]
    Unknown,
    /// Last known `get_state` snapshot — durable OMP truth.
    Durable,
    /// Streaming from an active turn; may be superseded any moment.
    Live,
    /// Child restarted / disconnected; last known snapshot is stale.
    Stale,
}

/// Mirror of `get_state` plus promoted scalar fields. Raw `state` is retained
/// verbatim so no upstream key is dropped when we don't recognize it.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct RuntimeState {
    /// Freshness of the promoted state relative to the wire source.
    pub quality: StateQuality,
    /// Full lossless `state` object from the latest `get_state` (or
    /// `session_info_update`). `None` until first hydration.
    pub state: Option<Value>,

    // Promoted stable scalars.
    /// Current model identifier, when reported by OMP.
    pub model: Option<String>,
    /// Current thinking-level payload, preserved in its wire shape.
    pub thinking: Option<Value>,
    /// Whether OMP reports an active streaming turn.
    pub is_streaming: Option<bool>,
    /// Whether OMP reports an active compaction.
    pub is_compacting: Option<bool>,
    /// Current OMP session id.
    pub session_id: Option<String>,
    /// Current OMP session file path.
    pub session_file: Option<String>,
    /// Parent session id, if OMP reports one.
    pub parent_session: Option<String>,
    /// Whether fast mode is enabled.
    pub fast_mode_enabled: Option<bool>,
    /// Whether fast mode is currently active.
    pub fast_mode_active: Option<bool>,
    /// How OMP applies steering messages to the active turn.
    pub steering_mode: Option<String>,
    /// How OMP dispatches queued follow-up messages.
    pub follow_up_mode: Option<String>,
    /// Whether queued messages interrupt immediately or wait.
    pub interrupt_mode: Option<String>,
    /// Whether OMP automatically compacts the session context.
    pub auto_compaction_enabled: Option<bool>,
    /// Whether OMP automatically retries recoverable failures.
    pub auto_retry_enabled: Option<bool>,
    /// Number of messages currently queued by OMP.
    pub queued_message_count: Option<u64>,
    /// Token metrics blob — shape is provider-specific, kept lossless.
    pub tokens: Option<Value>,
    /// Context-window info blob.
    pub context: Option<Value>,
}

/// Doctrine §5.4 run-phase machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    /// No turn is active.
    #[default]
    Idle,
    /// OMP is streaming an active turn.
    Streaming,
    /// OMP accepted a non-terminal turn and is awaiting resume.
    AwaitingResume,
    /// OMP is compacting conversation state.
    Compacting,
    /// OMP is retrying after a recoverable failure.
    Retrying,
    /// The child process is restarting.
    Restarting,
    /// The child process is dead until explicitly restarted.
    Dead,
}

/// A pending `extension_ui_request` awaiting a client response. `payload`
/// keeps the full raw JSON so unknown extras survive. `method` is the
/// stable wire name.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UiDialog {
    /// Stable dialog request id.
    pub id: String,
    /// Stable wire method: `"select" | "confirm" | "input" | "editor" | "open_url"`.
    pub method: String,
    /// Full raw request JSON — preserved losslessly.
    pub payload: Value,
    /// Wire timeout value in milliseconds. `None` if the wire omitted it.
    /// No timers run in core; the UI (or a supervisor) enforces this.
    pub timeout_ms: Option<f64>,
}

/// Non-dialog extension UI display state: `setTitle` / `setStatus` /
/// `setWidget` / `set_editor_text`. Every value stays raw so unknown extras
/// survive.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct DisplayState {
    /// Current title text.
    pub title: Option<String>,
    /// Current keyed status lines.
    pub statuses: BTreeMap<String, Option<String>>,
    /// Current keyed raw widget payloads.
    pub widgets: BTreeMap<String, Value>,
    /// Current editor text payload.
    pub editor_text: Option<String>,
}

/// Deterministic, UI-free snapshot of a single OMP session.
///
/// Consumers feed [`IncomingFrame`]s via [`SessionProjection::apply`]. All
/// state derives from the wire; nothing is guessed.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct SessionProjection {
    /// Promoted runtime state.
    pub state: RuntimeState,
    /// Current OMP-published goal text. `None` means OMP has cleared the goal
    /// or no `goal_updated` event has supplied one yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Ordered visible transcript rows.
    pub transcript: Vec<TranscriptEntry>,
    /// Pending extension UI dialogs.
    pub pending_dialogs: Vec<UiDialog>,
    /// Current run-phase state machine value.
    pub run_phase: RunPhase,
    /// Wire-derived fallback-model notice shown while a retry fallback is
    /// active. Cleared once OMP reports retry completion or fallback success.
    pub fallback_banner: Option<String>,
    /// Non-dialog extension UI display state.
    pub display: DisplayState,

    // Raw side-channel storage. Typed shapes ship post-M2 (Tier-2).
    /// Raw todo payload retained until typed projection ships.
    pub todos_raw: Option<Value>,
    /// Raw subagent payloads retained until typed projection ships.
    pub subagents_raw: Vec<Value>,
    /// Raw available-command payload from hydration or update frames.
    pub available_commands_raw: Option<Value>,
    /// Reason the child was declared dead (from [`Self::mark_dead`]).
    pub dead_reason: Option<String>,

    // Reducer bookkeeping. Not part of the observable projection; excluded
    // from serialized snapshots.
    #[serde(skip)]
    tool_index: BTreeMap<String, usize>,
    #[serde(skip)]
    current_message: BTreeMap<u64, usize>,
}

impl SessionProjection {
    /// Construct an empty projection with unknown runtime state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // -----------------------------------------------------------------
    // Public entry points
    // -----------------------------------------------------------------

    /// Apply one decoded inbound frame.
    ///
    /// This is the entire reducer surface for wire-driven state. Any frame
    /// whose semantics we don't project explicitly leaves a visible
    /// [`TranscriptEntry::Unknown`] row so upstream additions never
    /// silently drop.
    pub fn apply(&mut self, frame: &IncomingFrame) {
        match &frame.kind {
            IncomingFrameKind::Ready(_)
            | IncomingFrameKind::Response(_)
            | IncomingFrameKind::RpcChunk(_)
            | IncomingFrameKind::TurnStart
            | IncomingFrameKind::TurnEnd(_) => {}

            IncomingFrameKind::RpcFrameError(err) => self.apply_rpc_frame_error(err, &frame.raw),
            IncomingFrameKind::AgentStart => self.apply_agent_start(),
            IncomingFrameKind::AgentEnd(end) => self.apply_agent_end(end),
            IncomingFrameKind::MessageStart | IncomingFrameKind::MessageEnd => {
                self.close_current_message();
            }
            IncomingFrameKind::MessageUpdate(mu) => self.apply_message_update(mu),

            IncomingFrameKind::ToolExecutionStart(s) => self.tool_start(s),
            IncomingFrameKind::ToolExecutionUpdate(u) => self.tool_update(u),
            IncomingFrameKind::ToolExecutionEnd(e) => self.tool_end(e),

            IncomingFrameKind::AutoCompactionStart => self.apply_compaction_start(),
            IncomingFrameKind::AutoCompactionEnd => self.apply_compaction_end(),
            IncomingFrameKind::AutoRetryStart => self.apply_retry_start(&frame.raw),
            IncomingFrameKind::AutoRetryEnd => self.apply_retry_end(&frame.raw),
            IncomingFrameKind::RetryFallbackApplied => {
                self.apply_retry_fallback_applied(&frame.raw);
            }
            IncomingFrameKind::RetryFallbackSucceeded => {
                self.apply_retry_fallback_succeeded(&frame.raw);
            }
            IncomingFrameKind::ModelChanged => self.apply_model_changed(&frame.raw),
            IncomingFrameKind::ThinkingLevelChanged(t) => self.apply_thinking_level(t),
            IncomingFrameKind::TtsrTriggered => {
                self.apply_quiet_event_notice("TTSR triggered", &frame.raw);
            }
            IncomingFrameKind::TodoReminder => {
                self.apply_quiet_event_notice("Todo reminder", &frame.raw);
            }
            IncomingFrameKind::TodoAutoClear => {
                self.todos_raw = None;
            }
            IncomingFrameKind::Notice(n) => self.apply_notice(n),
            IncomingFrameKind::GoalUpdated => self.apply_goal_updated(&frame.raw),

            IncomingFrameKind::PromptResult(pr) => self.apply_prompt_result(pr),
            IncomingFrameKind::AvailableCommandsUpdate => {
                self.apply_available_commands_update(&frame.raw);
            }
            IncomingFrameKind::CommandOutput => self.apply_command_output(&frame.raw),
            IncomingFrameKind::SessionInfoUpdate => self.apply_session_info_update(&frame.raw),
            IncomingFrameKind::ExtensionError => self.apply_extension_error(&frame.raw),
            IncomingFrameKind::ExtensionUiRequest(ui) => self.apply_ui(ui),

            IncomingFrameKind::SubagentLifecycle(p)
            | IncomingFrameKind::SubagentProgress(p)
            | IncomingFrameKind::SubagentEvent(p) => self.subagents_raw.push(p.payload.clone()),

            IncomingFrameKind::IrcMessage
            | IncomingFrameKind::ConfigUpdate
            | IncomingFrameKind::HostToolCall(_)
            | IncomingFrameKind::HostToolCancel(_)
            | IncomingFrameKind::HostUriRequest(_)
            | IncomingFrameKind::HostUriCancel(_)
            | IncomingFrameKind::Unknown { .. } => self.push_unknown(&frame.raw),
        }
    }

    /// Explicitly hydrate the projection from a `get_state` response's
    /// `data.state` payload. The reducer never issues commands itself;
    /// callers pass in the deserialized `data` (or `data.state`).
    pub fn hydrate_get_state(&mut self, data: &Value) {
        self.hydrate_state_object(data);
        self.state.quality = StateQuality::Durable;
    }

    /// Hydrate the ordered transcript from a `get_messages` response.
    ///
    /// The caller supplies a fresh projection. Tool calls are indexed while
    /// walking the message history so later `toolResult` messages can update
    /// the corresponding row in place.
    /// Drop transcript rows produced by history hydration so paging can restart
    /// cleanly after a `stale_cursor` error. Preserves non-history UI state.
    pub fn clear_hydrated_history(&mut self) {
        self.transcript.clear();
        self.tool_index.clear();
        self.current_message.clear();
    }

    pub fn hydrate_messages(&mut self, data: &Value) {
        let Some(messages) = data.get("messages").and_then(Value::as_array) else {
            return;
        };

        for message in messages {
            match message.get("role").and_then(Value::as_str) {
                Some("user") => {
                    let text = message_content_text(message.get("content"));
                    self.transcript.push(TranscriptEntry::User { text });
                }
                Some("assistant") => self.hydrate_assistant_message(message),
                Some("toolResult") => self.hydrate_tool_result(message),
                // Includes `fileMention` — preserved as Unknown for app-side rendering.
                _ => self.push_unknown(message),
            }
        }
    }

    /// Explicitly hydrate the `available_commands` snapshot from a
    /// `list_commands` (or equivalent) command response.
    pub fn hydrate_available_commands(&mut self, data: &Value) {
        let commands = data.get("commands").cloned().or_else(|| Some(data.clone()));
        self.available_commands_raw.clone_from(&commands);
    }

    /// Record a user prompt just sent to OMP. Called once per prompt.
    /// The reducer trusts the caller to fire this exactly once per user
    /// message (OMP never echoes it back on the event stream).
    pub fn push_user_message(&mut self, text: String) {
        self.transcript.push(TranscriptEntry::User { text });
    }

    /// Supervisor signaled the child is restarting. Marks state stale,
    /// clears pending dialogs (they will never be answered by the dying
    /// child), and enters [`RunPhase::Restarting`].
    pub fn mark_restarting(&mut self) {
        self.run_phase = RunPhase::Restarting;
        self.state.quality = StateQuality::Stale;
        self.pending_dialogs.clear();
    }

    /// Supervisor confirmed the child is dead. Non-recoverable without an
    /// explicit user restart action.
    pub fn mark_dead(&mut self, reason: String) {
        self.run_phase = RunPhase::Dead;
        self.state.quality = StateQuality::Stale;
        self.dead_reason = Some(reason);
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    fn promote_quality(&self) -> StateQuality {
        if matches!(self.state.quality, StateQuality::Unknown) {
            StateQuality::Unknown
        } else {
            StateQuality::Live
        }
    }

    fn apply_rpc_frame_error(&mut self, err: &RpcFrameErrorFrame, raw: &Value) {
        self.transcript.push(TranscriptEntry::Error {
            message: err.error.clone(),
            code: err.original_type.clone(),
        });
        self.push_unknown(raw);
    }

    fn apply_agent_start(&mut self) {
        self.run_phase = RunPhase::Streaming;
        self.state.is_streaming = Some(true);
        self.state.quality = self.promote_quality();
    }

    fn apply_compaction_start(&mut self) {
        self.state.is_compacting = Some(true);
        self.run_phase = RunPhase::Compacting;
        self.transcript.push(TranscriptEntry::Compaction {
            phase: CompactionPhase::Started,
        });
    }

    fn apply_compaction_end(&mut self) {
        self.state.is_compacting = Some(false);
        self.run_phase = self.streaming_or_idle_phase();
        self.transcript.push(TranscriptEntry::Compaction {
            phase: CompactionPhase::Completed,
        });
    }

    fn apply_retry_start(&mut self, raw: &Value) {
        self.run_phase = RunPhase::Retrying;
        self.push_retry_info(retry_event_detail("auto-retry started", raw));
    }

    fn apply_retry_end(&mut self, raw: &Value) {
        self.run_phase = self.streaming_or_idle_phase();
        self.fallback_banner = None;
        self.push_retry_info(retry_event_detail("auto-retry ended", raw));
    }

    fn apply_retry_fallback_applied(&mut self, raw: &Value) {
        let from = raw_string(raw, "from");
        let to = raw_string(raw, "to");
        let role = raw_string(raw, "role");
        let detail = fallback_applied_detail(from.as_deref(), to.as_deref(), role.as_deref());
        self.fallback_banner = Some(fallback_banner_text(
            from.as_deref(),
            to.as_deref(),
            role.as_deref(),
        ));
        self.push_retry_info(detail);
    }

    fn apply_retry_fallback_succeeded(&mut self, raw: &Value) {
        let model = raw_string(raw, "model");
        let role = raw_string(raw, "role");
        self.fallback_banner = None;
        self.push_retry_info(fallback_succeeded_detail(model.as_deref(), role.as_deref()));
    }

    fn streaming_or_idle_phase(&self) -> RunPhase {
        if matches!(self.state.is_streaming, Some(true)) {
            RunPhase::Streaming
        } else {
            RunPhase::Idle
        }
    }

    fn push_retry_info(&mut self, detail: impl Into<String>) {
        self.transcript.push(TranscriptEntry::RetryInfo {
            detail: detail.into(),
        });
    }

    fn apply_model_changed(&mut self, raw: &Value) {
        // OMP 17.2.10 emits a bare `{ type: "model_changed" }` with no model
        // payload. When a model field *is* present (string or object), promote
        // it; otherwise leave the prior display model untouched.
        if let Some(m) = raw.get("model").and_then(format_model_label) {
            self.state.model = Some(m);
        }
        self.push_unknown(raw);
    }

    fn apply_available_commands_update(&mut self, raw: &Value) {
        let commands = raw.get("commands").cloned();
        self.available_commands_raw.clone_from(&commands);
    }

    fn apply_command_output(&mut self, raw: &Value) {
        let text = raw
            .get("output")
            .or_else(|| raw.get("text"))
            .and_then(Value::as_str)
            .map_or_else(|| raw.to_string(), str::to_owned);
        self.transcript
            .push(TranscriptEntry::CommandOutput(BoundedText::from_text(
                &text,
            )));
    }

    fn apply_session_info_update(&mut self, raw: &Value) {
        self.hydrate_state_object(raw);
        self.state.quality = StateQuality::Durable;
    }

    fn apply_extension_error(&mut self, raw: &Value) {
        let message = raw
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(|| "extension_error".to_owned(), str::to_owned);
        let code = raw.get("code").and_then(Value::as_str).map(str::to_owned);
        self.transcript
            .push(TranscriptEntry::Error { message, code });
    }

    fn apply_agent_end(&mut self, end: &AgentEndFrame) {
        self.state.is_streaming = Some(false);
        // Doctrine §0.4: acceptance ≠ completion. `isTerminal == false`
        // means OMP is awaiting a resume before continuing this turn.
        let terminal = end.is_terminal != Some(false);
        self.run_phase = if terminal {
            RunPhase::Idle
        } else {
            RunPhase::AwaitingResume
        };
        if matches!(self.state.quality, StateQuality::Live) {
            self.state.quality = StateQuality::Durable;
        }
        if terminal {
            self.emit_terminal_assistant_error(&end.messages);
        }
    }

    /// On terminal `agent_end`, surface an [`TranscriptEntry::Error`] row for the
    /// last assistant message whose `stopReason` is `aborted` or `error` and that
    /// carries an `errorMessage`. Idempotent across duplicate terminal `agent_end`
    /// frames: the same `(message, code)` pair is emitted at most once per turn.
    fn emit_terminal_assistant_error(&mut self, messages: &[Value]) {
        let Some(assistant) = messages
            .iter()
            .rev()
            .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
        else {
            return;
        };
        let stop_reason = assistant.get("stopReason").and_then(Value::as_str);
        if !matches!(stop_reason, Some("aborted" | "error")) {
            return;
        }
        let error_message = assistant.get("errorMessage").and_then(Value::as_str);
        let message = match error_message {
            Some(m) if !m.is_empty() => m.to_owned(),
            _ => match stop_reason {
                Some("aborted") => "assistant aborted".to_owned(),
                _ => "assistant error".to_owned(),
            },
        };
        let code = stop_reason.map(str::to_owned);
        if self.transcript.iter().any(|e| {
            matches!(e, TranscriptEntry::Error { message: m, code: c }
                if m == &message && c == &code)
        }) {
            return;
        }
        self.transcript
            .push(TranscriptEntry::Error { message, code });
    }

    fn apply_message_update(&mut self, mu: &MessageUpdateFrame) {
        match &mu.assistant_message_event.kind {
            AssistantMessageEventKind::Start => {
                // A fresh assistant message boundary. Older streaming
                // entries from a *previous* message must already have been
                // closed by `MessageEnd`; if not, close them defensively.
                if !self.current_message.is_empty() {
                    self.close_current_message();
                }
            }
            AssistantMessageEventKind::TextStart { content_index } => {
                self.ensure_text_row(*content_index);
            }
            AssistantMessageEventKind::TextDelta {
                content_index,
                delta,
            } => {
                let i = self.ensure_text_row(*content_index);
                if let Some(TranscriptEntry::AssistantText { markdown, .. }) =
                    self.transcript.get_mut(i)
                {
                    markdown.push_str(delta);
                }
            }
            AssistantMessageEventKind::TextEnd {
                content_index,
                content,
            } => {
                let i = self.ensure_text_row(*content_index);
                if let Some(TranscriptEntry::AssistantText {
                    markdown,
                    streaming,
                }) = self.transcript.get_mut(i)
                {
                    // Reconcile the final content authoritatively, do NOT
                    // append (which would duplicate the deltas). Always go
                    // through BoundedText so oversized wire content cannot
                    // bypass the cap.
                    markdown.set_text(content);
                    *streaming = false;
                }
            }
            AssistantMessageEventKind::ThinkingStart { content_index } => {
                self.ensure_thinking_row(*content_index);
            }
            AssistantMessageEventKind::ThinkingDelta {
                content_index,
                delta,
            } => {
                let i = self.ensure_thinking_row(*content_index);
                if let Some(TranscriptEntry::Thinking { text, .. }) = self.transcript.get_mut(i) {
                    text.push_str(delta);
                }
            }
            AssistantMessageEventKind::ThinkingEnd {
                content_index,
                content,
            } => {
                let i = self.ensure_thinking_row(*content_index);
                if let Some(TranscriptEntry::Thinking {
                    text, streaming, ..
                }) = self.transcript.get_mut(i)
                {
                    text.set_text(content);
                    *streaming = false;
                }
            }
            AssistantMessageEventKind::ImageEnd { content_index } => {
                // Surface a visible row so image blocks are never dropped
                // silently (Doctrine §9). Prefer mime/type summary from the
                // wire `content` object when present.
                let summary = mu
                    .assistant_message_event
                    .image_content()
                    .map_or_else(|| format!("Image #{content_index}"), image_content_summary);
                self.transcript.push(TranscriptEntry::Notice(summary));
            }
            AssistantMessageEventKind::ToolcallStart { .. }
            | AssistantMessageEventKind::ToolcallDelta { .. }
            | AssistantMessageEventKind::ToolcallEnd { .. } => {
                // Tool-row lifecycle is driven by `tool_execution_*` frames.
                // Toolcall content blocks are represented there (and in the
                // retained raw `message` for lossless round-tripping).
            }
            AssistantMessageEventKind::Done { .. } => {
                // Final message reconciliation: mark every streaming row
                // for this message non-streaming without duplicating the
                // content (deltas have already accumulated).
                self.close_current_message();
            }
            AssistantMessageEventKind::Error { .. } => {
                let message = mu
                    .assistant_message_event
                    .error_message()
                    .and_then(|v| v.get("error"))
                    .and_then(Value::as_str)
                    .map_or_else(|| "assistant error".to_owned(), str::to_owned);
                self.transcript.push(TranscriptEntry::Error {
                    message,
                    code: None,
                });
                self.close_current_message();
            }
            AssistantMessageEventKind::Unknown { .. } => {
                self.push_unknown(&mu.raw);
            }
        }
    }

    fn close_current_message(&mut self) {
        for &idx in self.current_message.values() {
            if let Some(
                TranscriptEntry::AssistantText { streaming, .. }
                | TranscriptEntry::Thinking { streaming, .. },
            ) = self.transcript.get_mut(idx)
            {
                *streaming = false;
            }
        }
        self.current_message.clear();
    }

    fn ensure_text_row(&mut self, ci: u64) -> usize {
        if let Some(&i) = self.current_message.get(&ci) {
            return i;
        }
        let i = self.transcript.len();
        self.transcript.push(TranscriptEntry::AssistantText {
            markdown: Markdown::new(""),
            streaming: true,
        });
        self.current_message.insert(ci, i);
        i
    }

    fn ensure_thinking_row(&mut self, ci: u64) -> usize {
        if let Some(&i) = self.current_message.get(&ci) {
            return i;
        }
        let i = self.transcript.len();
        self.transcript.push(TranscriptEntry::Thinking {
            text: BoundedText::new(),
            streaming: true,
            collapsed: true,
        });
        self.current_message.insert(ci, i);
        i
    }

    // -------- Tool lifecycle --------

    fn tool_start(&mut self, s: &ToolExecutionStartFrame) {
        if let Some(&i) = self.tool_index.get(&s.tool_call_id) {
            // Duplicate start — protocol tolerance. Refresh args, leave
            // status alone (may already be Ok/Err if end raced ahead).
            if let Some(TranscriptEntry::ToolCall(call)) = self.transcript.get_mut(i) {
                call.args_json.clone_from(&s.args);
                if call.name.is_empty() {
                    call.name.clone_from(&s.tool_name);
                }
            }
            return;
        }
        let call =
            ToolCall::new_running(s.tool_call_id.clone(), s.tool_name.clone(), s.args.clone());
        let idx = self.transcript.len();
        self.transcript.push(TranscriptEntry::ToolCall(call));
        self.tool_index.insert(s.tool_call_id.clone(), idx);
    }

    fn tool_update(&mut self, u: &ToolExecutionUpdateFrame) {
        let idx = self.tool_index.get(&u.tool_call_id).copied();
        let idx =
            idx.unwrap_or_else(|| self.synth_tool_row(&u.tool_call_id, &u.tool_name, &u.args));
        if let Some(TranscriptEntry::ToolCall(call)) = self.transcript.get_mut(idx) {
            call.args_json.clone_from(&u.args);
            // partialResult is the cumulative running snapshot; replace the
            // visible output rather than append.
            let visible = extract_visible_output(&u.partial_result);
            call.output.clear();
            call.output.push_str(&visible);
        }
    }

    fn tool_end(&mut self, e: &ToolExecutionEndFrame) {
        let idx = self.tool_index.get(&e.tool_call_id).copied();
        let idx =
            idx.unwrap_or_else(|| self.synth_tool_row(&e.tool_call_id, &e.tool_name, &Value::Null));
        if let Some(TranscriptEntry::ToolCall(call)) = self.transcript.get_mut(idx) {
            call.status = if matches!(e.is_error, Some(true)) {
                ToolStatus::Err
            } else {
                ToolStatus::Ok
            };
            let visible = extract_visible_output(&e.result);
            call.output.clear();
            call.output.push_str(&visible);
        }
    }

    fn synth_tool_row(&mut self, tool_call_id: &str, tool_name: &str, args: &Value) -> usize {
        let call = ToolCall::new_running(tool_call_id, tool_name, args.clone());
        let idx = self.transcript.len();
        self.transcript.push(TranscriptEntry::ToolCall(call));
        self.tool_index.insert(tool_call_id.to_owned(), idx);
        idx
    }

    // -------- Notice / prompt_result / thinking-level --------

    fn apply_notice(&mut self, n: &NoticeFrame) {
        match n.level {
            NoticeLevel::Error => self.transcript.push(TranscriptEntry::Error {
                message: n.message.clone(),
                code: n.source.clone(),
            }),
            NoticeLevel::Info | NoticeLevel::Warning | NoticeLevel::Unknown(_) => {
                self.transcript
                    .push(TranscriptEntry::Notice(n.message.clone()));
            }
        }
    }

    fn apply_quiet_event_notice(&mut self, label: &str, raw: &Value) {
        let notice =
            event_text(raw).map_or_else(|| label.to_owned(), |text| format!("{label}: {text}"));
        self.transcript.push(TranscriptEntry::Notice(notice));
    }

    fn apply_goal_updated(&mut self, raw: &Value) {
        if raw.get("goal").is_some_and(Value::is_null) {
            self.goal = None;
            self.transcript
                .push(TranscriptEntry::Notice("Goal cleared".to_owned()));
            return;
        }

        if let Some(goal) = event_text(raw) {
            self.goal = Some(goal.clone());
            self.transcript
                .push(TranscriptEntry::Notice(format!("Goal updated: {goal}")));
        } else {
            // The event is recognized, but its payload is not. Keep the prior
            // authoritative goal and leave a visible row rather than guessing.
            self.transcript
                .push(TranscriptEntry::Notice("Goal updated".to_owned()));
        }
    }

    fn apply_prompt_result(&mut self, pr: &PromptResultFrame) {
        if !pr.agent_invoked {
            // Local-only prompt: OMP handled it without invoking the agent
            // (e.g. `/help`). No turn will start; nothing to project onto
            // run_phase beyond staying Idle. Emit a notice so it's visible.
            self.transcript
                .push(TranscriptEntry::Notice("local-only prompt".to_owned()));
        }
    }

    fn apply_thinking_level(&mut self, t: &ThinkingLevelChangedFrame) {
        self.state.thinking = t.resolved.clone().or_else(|| t.thinking_level.clone());
    }

    // -------- Extension UI --------

    fn apply_ui(&mut self, ui: &ExtensionUiRequestFrame) {
        match &ui.method {
            ExtensionUiMethod::Cancel { target_id } => {
                self.pending_dialogs.retain(|d| d.id != *target_id);
            }
            ExtensionUiMethod::SetTitle { title } => {
                self.display.title = Some(title.clone());
            }
            ExtensionUiMethod::SetStatus {
                status_key,
                status_text,
            } => {
                self.display
                    .statuses
                    .insert(status_key.clone(), status_text.clone());
            }
            ExtensionUiMethod::SetWidget { widget_key, .. } => {
                self.display
                    .widgets
                    .insert(widget_key.clone(), ui.raw.clone());
            }
            ExtensionUiMethod::SetEditorText { text } => {
                self.display.editor_text = Some(text.clone());
            }
            ExtensionUiMethod::Notify {
                message,
                notify_type,
            } => {
                if matches!(notify_type, Some(NoticeLevel::Error)) {
                    self.transcript.push(TranscriptEntry::Error {
                        message: message.clone(),
                        code: None,
                    });
                } else {
                    self.transcript
                        .push(TranscriptEntry::Notice(message.clone()));
                }
            }
            ExtensionUiMethod::Select { timeout, .. } => {
                self.enqueue_dialog(ui, "select", *timeout);
            }
            ExtensionUiMethod::Confirm { timeout, .. } => {
                self.enqueue_dialog(ui, "confirm", *timeout);
            }
            ExtensionUiMethod::Input { timeout, .. } => {
                self.enqueue_dialog(ui, "input", *timeout);
            }
            ExtensionUiMethod::Editor { .. } => {
                self.enqueue_dialog(ui, "editor", None);
            }
            ExtensionUiMethod::OpenUrl { .. } => {
                self.enqueue_dialog(ui, "open_url", None);
            }
            ExtensionUiMethod::Unknown { .. } => {
                self.push_unknown(&ui.raw);
            }
        }
    }

    fn enqueue_dialog(
        &mut self,
        ui: &ExtensionUiRequestFrame,
        method: &str,
        timeout_ms: Option<f64>,
    ) {
        // A dialog with the same id replaces the previous entry rather than
        // stacking — OMP can only have one pending request per id.
        if let Some(existing) = self.pending_dialogs.iter_mut().find(|d| d.id == ui.id) {
            method.clone_into(&mut existing.method);
            existing.payload = ui.raw.clone();
            existing.timeout_ms = timeout_ms;
            return;
        }
        self.pending_dialogs.push(UiDialog {
            id: ui.id.clone(),
            method: method.to_owned(),
            payload: ui.raw.clone(),
            timeout_ms,
        });
    }

    // -------- Hydration --------

    fn hydrate_assistant_message(&mut self, message: &Value) {
        let Some(content) = message.get("content") else {
            return;
        };

        match content {
            Value::String(text) => {
                if !text.is_empty() {
                    self.transcript.push(TranscriptEntry::AssistantText {
                        markdown: Markdown::new(text),
                        streaming: false,
                    });
                }
            }
            Value::Array(parts) => {
                for part in parts {
                    match part.get("type").and_then(Value::as_str) {
                        Some("thinking") => {
                            let text = part
                                .get("thinking")
                                .or_else(|| part.get("text"))
                                .and_then(Value::as_str);
                            if let Some(text) = text.filter(|text| !text.is_empty()) {
                                self.transcript.push(TranscriptEntry::Thinking {
                                    text: BoundedText::from_text(text),
                                    streaming: false,
                                    collapsed: true,
                                });
                            }
                        }
                        Some("text") => {
                            if let Some(text) = part
                                .get("text")
                                .and_then(Value::as_str)
                                .filter(|text| !text.is_empty())
                            {
                                self.transcript.push(TranscriptEntry::AssistantText {
                                    markdown: Markdown::new(text),
                                    streaming: false,
                                });
                            }
                        }
                        Some("toolCall") => self.hydrate_tool_call(part),
                        _ => self.push_unknown(part),
                    }
                }
            }
            _ => {}
        }
    }

    fn hydrate_tool_call(&mut self, part: &Value) {
        let tool_call_id = part
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| part.get("toolCallId").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned();
        let name = part
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let args = part.get("arguments").cloned().unwrap_or(Value::Null);
        let idx = self.transcript.len();
        self.transcript
            .push(TranscriptEntry::ToolCall(ToolCall::new_running(
                tool_call_id.clone(),
                name,
                args,
            )));
        self.tool_index.insert(tool_call_id, idx);
    }

    fn hydrate_tool_result(&mut self, message: &Value) {
        let tool_call_id = message
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = message_content_text(message.get("content"));
        let status = if message
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            ToolStatus::Err
        } else {
            ToolStatus::Ok
        };

        if let Some(idx) = self.tool_index.get(tool_call_id).copied()
            && let Some(TranscriptEntry::ToolCall(call)) = self.transcript.get_mut(idx)
        {
            call.output.push_str(&text);
            call.status = status;
        } else {
            self.transcript
                .push(TranscriptEntry::CommandOutput(BoundedText::from_text(
                    &text,
                )));
        }
    }

    fn hydrate_state_object(&mut self, data: &Value) {
        // Accept either `{state: {...}}` (get_state response envelope) or a
        // bare state object.
        let state = data.get("state").unwrap_or(data);
        self.state.state = Some(state.clone());

        let str_field = |k: &str| state.get(k).and_then(Value::as_str).map(str::to_owned);
        let bool_field = |k: &str| state.get(k).and_then(Value::as_bool);
        let u64_field = |k: &str| state.get(k).and_then(Value::as_u64);
        let val_field = |k: &str| state.get(k).cloned();

        // OMP reports `model` as a Model object `{provider,id,...}`; older
        // fixtures may use a plain string. Accept both.
        if let Some(m) = state.get("model").and_then(format_model_label) {
            self.state.model = Some(m);
        }
        if let Some(t) = val_field("thinking").or_else(|| val_field("thinkingLevel")) {
            self.state.thinking = Some(t);
        }
        if let Some(b) = bool_field("isStreaming") {
            self.state.is_streaming = Some(b);
        }
        if let Some(b) = bool_field("isCompacting") {
            self.state.is_compacting = Some(b);
        }
        if let Some(s) = str_field("sessionId") {
            self.state.session_id = Some(s);
        }
        if let Some(s) = str_field("sessionFile") {
            self.state.session_file = Some(s);
        }
        if let Some(s) = str_field("parentSession") {
            self.state.parent_session = Some(s);
        }
        // OMP 17.2.10 `get_state` publishes top-level `fastModeEnabled` /
        // `fastModeActive`; older fixtures used nested `fastMode.{enabled,active}`.
        if let Some(fm) = state.get("fastMode") {
            self.state.fast_mode_enabled = fm.get("enabled").and_then(Value::as_bool);
            self.state.fast_mode_active = fm.get("active").and_then(Value::as_bool);
        }
        if let Some(b) = bool_field("fastModeEnabled") {
            self.state.fast_mode_enabled = Some(b);
        }
        if let Some(b) = bool_field("fastModeActive") {
            self.state.fast_mode_active = Some(b);
        }
        if let Some(s) = str_field("steeringMode") {
            self.state.steering_mode = Some(s);
        }
        if let Some(s) = str_field("followUpMode") {
            self.state.follow_up_mode = Some(s);
        }
        if let Some(s) = str_field("interruptMode") {
            self.state.interrupt_mode = Some(s);
        }
        if let Some(b) = bool_field("autoCompactionEnabled") {
            self.state.auto_compaction_enabled = Some(b);
        }
        if let Some(b) = bool_field("autoRetryEnabled") {
            self.state.auto_retry_enabled = Some(b);
        }
        if let Some(count) = u64_field("queuedMessageCount") {
            self.state.queued_message_count = Some(count);
        }
        if let Some(t) = val_field("tokens").or_else(|| val_field("usage")) {
            self.state.tokens = Some(t);
        } else if let Some(tps) = val_field("tokensPerSecond") {
            // OMP often reports a bare tokensPerSecond scalar until a turn
            // accumulates richer usage — keep it under the tokens blob.
            self.state.tokens = Some(serde_json::json!({ "tokensPerSecond": tps }));
        }
        // OMP 17.2.10 names this `contextUsage` (`{tokens,contextWindow,percent}`).
        if let Some(c) = val_field("context").or_else(|| val_field("contextUsage")) {
            self.state.context = Some(c);
        }
        if let Some(todos) = val_field("todoPhases").or_else(|| val_field("todos")) {
            self.todos_raw = Some(todos);
        }
    }

    fn push_unknown(&mut self, raw: &Value) {
        self.transcript
            .push(TranscriptEntry::Unknown { raw: raw.clone() });
    }
}

fn raw_string(raw: &Value, field: &str) -> Option<String> {
    raw.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn event_text(raw: &Value) -> Option<String> {
    ["message", "text", "goal"]
        .iter()
        .find_map(|field| raw.get(*field).and_then(event_text_value))
}

fn event_text_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_owned())
        }
        Value::Object(object) => ["objective", "message", "text", "goal"]
            .iter()
            .find_map(|field| object.get(*field).and_then(event_text_value)),
        _ => None,
    }
}

fn raw_scalar(raw: &Value, field: &str) -> Option<String> {
    raw.get(field).and_then(|value| match value {
        Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        }
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn retry_event_detail(prefix: &str, raw: &Value) -> String {
    let attempt = ["attempt", "attemptNumber", "retryAttempt"]
        .iter()
        .find_map(|field| raw_scalar(raw, field));
    let maximum = ["maxAttempts", "max_attempts", "totalAttempts"]
        .iter()
        .find_map(|field| raw_scalar(raw, field));
    let attempts = raw_scalar(raw, "attempts");

    match (attempt, maximum, attempts) {
        (Some(attempt), Some(maximum), _) => format!("{prefix} (attempt {attempt}/{maximum})"),
        (Some(attempt), None, _) => format!("{prefix} (attempt {attempt})"),
        (None, _, Some(attempts)) => format!("{prefix} (attempts: {attempts})"),
        (None, Some(maximum), None) => format!("{prefix} (max attempts: {maximum})"),
        (None, None, None) => prefix.to_owned(),
    }
}

fn fallback_applied_detail(from: Option<&str>, to: Option<&str>, role: Option<&str>) -> String {
    match (from, to, role) {
        (Some(from), Some(to), Some(role)) => {
            format!("fallback applied: {from} → {to} (role={role})")
        }
        (Some(from), None, Some(role)) => {
            format!("fallback applied from: {from} (role={role})")
        }
        (None, Some(to), Some(role)) => format!("fallback applied: {to} (role={role})"),
        (None, None, Some(role)) => format!("fallback applied (role={role})"),
        (Some(from), Some(to), None) => format!("fallback applied: {from} → {to}"),
        (Some(from), None, None) => format!("fallback applied from: {from}"),
        (None, Some(to), None) => format!("fallback applied: {to}"),
        (None, None, None) => "fallback applied".to_owned(),
    }
}

fn fallback_succeeded_detail(model: Option<&str>, role: Option<&str>) -> String {
    match (model, role) {
        (Some(model), Some(role)) => format!("fallback succeeded: {model} (role={role})"),
        (Some(model), None) => format!("fallback succeeded: {model}"),
        (None, Some(role)) => format!("fallback succeeded (role={role})"),
        (None, None) => "fallback succeeded".to_owned(),
    }
}

fn fallback_banner_text(from: Option<&str>, to: Option<&str>, role: Option<&str>) -> String {
    match (from, to, role) {
        (Some(from), Some(to), Some(role)) => {
            format!("Using fallback model {to} (instead of {from}) for {role}")
        }
        (None, Some(to), Some(role)) => format!("Using fallback model {to} for {role}"),
        (Some(from), None, Some(role)) => {
            format!("Using a fallback model (instead of {from}) for {role}")
        }
        (None, None, Some(role)) => format!("Using a fallback model for {role}"),
        (Some(from), Some(to), None) => format!("Using fallback model {to} (instead of {from})"),
        (None, Some(to), None) => format!("Using fallback model {to}"),
        (Some(from), None, None) => format!("Using a fallback model (instead of {from})"),
        (None, None, None) => "Using a fallback model".to_owned(),
    }
}

fn image_content_summary(content: &Value) -> String {
    let mime = content
        .get("mimeType")
        .or_else(|| content.get("mime_type"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let ty = content
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match (mime, ty) {
        (Some(mime), Some(ty)) if ty != "image" => format!("Image ({ty}, {mime})"),
        (Some(mime), _) => format!("Image ({mime})"),
        (None, Some(ty)) => format!("Image ({ty})"),
        (None, None) => "Image".to_owned(),
    }
}

fn message_content_text(content: Option<&Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };

    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.get("type")
                    .and_then(Value::as_str)
                    .filter(|kind| *kind == "text")
                    .and_then(|_| part.get("text"))
                    .and_then(Value::as_str)
            })
            .collect(),
        Value::Object(part) if part.get("type").and_then(Value::as_str) == Some("text") => part
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}

/// Format an OMP model reference for display / `set_model` targeting.
///
/// Accepts:
/// - a plain string (`"provider/id"` or opaque label)
/// - an object with `provider` + `id` (canonical Model shape)
/// - an object with `provider` + `modelId` (command-adjacent shape)
#[must_use]
pub fn format_model_label(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_owned());
    }
    let provider = v.get("provider").and_then(Value::as_str)?.trim();
    let id = v
        .get("id")
        .or_else(|| v.get("modelId"))
        .and_then(Value::as_str)?
        .trim();
    if provider.is_empty() || id.is_empty() {
        return None;
    }
    Some(format!("{provider}/{id}"))
}

/// Split a `provider/id` label into `(provider, model_id)`.
///
/// Uses the first `/` as the separator so ids like `kimi-k3:max` stay intact.
#[must_use]
pub fn split_model_label(label: &str) -> Option<(String, String)> {
    let (provider, id) = label.split_once('/')?;
    let provider = provider.trim();
    let id = id.trim();
    if provider.is_empty() || id.is_empty() {
        return None;
    }
    Some((provider.to_owned(), id.to_owned()))
}

// ---------------------------------------------------------------------------
// Lossless visible-output extraction
// ---------------------------------------------------------------------------

/// Recursively extract a human-visible string from a `partialResult` /
/// `result` value while preserving all information.
///
/// Rules (stable, deterministic):
///
/// * If the value is a bare string, return it.
/// * If it's an object with a `text` string, return that.
/// * If it's an object with a `content` array, extract each element and
///   join with `\n`.
/// * If it's an array, extract each element and join with `\n`.
/// * Otherwise return canonical pretty JSON — losslessly captured, still
///   visible.
///
/// The reducer never *drops* the original `result` — the raw value is kept
/// on the tool-execution frame in the fixture stream and can be re-derived
/// at any time.
///
/// # Panics
///
/// Never panics for any [`Value`]. JSON pretty-printing is fallible in the
/// `serde_json` API; the reducer falls back to compact JSON if that ever
/// reports an error.
#[must_use]
pub fn extract_visible_output(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(_) | Value::Number(_) => v.to_string(),
        Value::Array(a) => join_visible_output(a),
        Value::Object(o) => {
            if let Some(diff) = crate::diff::extract_tool_diff_text(v) {
                let path = o
                    .get("details")
                    .and_then(|d| d.get("path"))
                    .or_else(|| o.get("path"))
                    .and_then(Value::as_str);
                return match path {
                    Some(path) => format!("{path}\n{diff}"),
                    None => diff,
                };
            }
            if let Some(s) = o.get("text").and_then(Value::as_str) {
                return s.to_owned();
            }
            if let Some(arr) = o.get("content").and_then(Value::as_array) {
                return join_visible_output(arr);
            }
            serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
        }
    }
}

fn join_visible_output(values: &[Value]) -> String {
    let mut out = String::new();
    for (i, item) in values.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&extract_visible_output(item));
    }
    out
}
