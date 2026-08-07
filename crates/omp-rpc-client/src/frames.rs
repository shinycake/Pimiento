//! PLAN §4 wire model for the installed OMP 17.2.10 RPC protocol.
//!
//! Design invariants (see `docs/protocol-notes.md`):
//!
//! * Stable discriminants are typed Rust enums / structs; every discriminated
//!   enum has an `Unknown` fallback so a future OMP variant never causes a
//!   decode failure.
//! * Open-ended payloads (provider blobs, tool args/results, session state,
//!   assistant messages, etc.) are preserved as `serde_json::Value`.
//! * Every inbound frame retains the full original JSON object in a `raw`
//!   field so unknown extras on known variants survive round-tripping through
//!   the projection layer.
//! * Outbound [`RpcCommand`] serializes to the exact camelCase wire keys that
//!   OMP accepts, and omits absent optional fields.
//!
//! This module is transport-agnostic — it does not know about newline framing,
//! chunk reassembly, or child-process supervision. Those live in `decoder.rs`
//! and `supervisor.rs`.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};

// ---------------------------------------------------------------------------
// Transport constants (from rpc-frame.ts on 17.2.10).
// ---------------------------------------------------------------------------

/// Maximum UTF-8 size of one newline-delimited physical frame, including `\n`.
pub const MAX_RPC_FRAME_BYTES: usize = 1024 * 1024;

/// Maximum UTF-8 size of one logical frame after v2 chunk reassembly.
pub const MAX_RPC_REASSEMBLED_BYTES: usize = 64 * 1024 * 1024;

/// Server chunk payload size before base64 encoding.
pub const RPC_CHUNK_PAYLOAD_BYTES: usize = 256 * 1024;

// ---------------------------------------------------------------------------
// Ready / chunk / transport-error frames.
// ---------------------------------------------------------------------------

/// Initial `ready` handshake frame; advertised protocol version is always `1`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReadyFrame {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(rename = "supportedProtocolVersions")]
    pub supported_protocol_versions: Vec<u32>,
    #[serde(rename = "maxFrameBytes")]
    pub max_frame_bytes: u64,
    #[serde(rename = "maxReassembledFrameBytes")]
    pub max_reassembled_frame_bytes: u64,
}

/// One physical `rpc_chunk` frame carrying a slice of a larger logical frame.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcChunkFrame {
    #[serde(rename = "chunkId")]
    pub chunk_id: String,
    pub index: u32,
    pub count: u32,
    #[serde(rename = "byteLength")]
    pub byte_length: u64,
    pub data: String,
}

/// Non-response transport failure emitted when a frame or reassembled frame
/// exceeds its limit and cannot be compacted (see rpc-frame.ts `overflowFrame`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcFrameErrorFrame {
    #[serde(
        default,
        rename = "originalType",
        skip_serializing_if = "Option::is_none"
    )]
    pub original_type: Option<String>,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Responses.
// ---------------------------------------------------------------------------

/// `type: "response"` — reply to a client command. `id` is optional because
/// malformed client JSON produces an id-less `command: "parse"` failure and
/// unknown commands also drop the id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub command: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

// ---------------------------------------------------------------------------
// Outbound commands.
// ---------------------------------------------------------------------------

/// Enum-with-unknown-fallback pattern used across the wire model. Serialize
/// writes the exact camelCase string; deserialize accepts unknown values as
/// [`StringEnum::Unknown`].
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $variant:ident => $wire:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        $vis enum $name {
            $( $variant, )+
            Unknown(String),
        }

        impl $name {
            pub fn as_wire(&self) -> &str {
                match self {
                    $( Self::$variant => $wire, )+
                    Self::Unknown(s) => s.as_str(),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
                ser.serialize_str(self.as_wire())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
                let s = String::deserialize(de)?;
                Ok(match s.as_str() {
                    $( $wire => Self::$variant, )+
                    _ => Self::Unknown(s),
                })
            }
        }
    };
}

string_enum! {
    pub enum StreamingBehavior {
        Steer => "steer",
        FollowUp => "followUp",
    }
}

string_enum! {
    pub enum QueueMode {
        All => "all",
        OneAtATime => "one-at-a-time",
    }
}

string_enum! {
    pub enum InterruptMode {
        Immediate => "immediate",
        Wait => "wait",
    }
}

string_enum! {
    pub enum SubagentSubscriptionLevel {
        Off => "off",
        Progress => "progress",
        Events => "events",
    }
}

/// Every outbound command carries an optional correlation `id` and a typed
/// body. Serializing flattens the body's `{type, ...}` alongside `id`.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcCommand {
    pub id: Option<String>,
    pub body: RpcCommandBody,
}

impl RpcCommand {
    pub fn new(id: impl Into<Option<String>>, body: RpcCommandBody) -> Self {
        Self {
            id: id.into(),
            body,
        }
    }
}

impl Serialize for RpcCommand {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        // Flatten the body into the same object as `id`. Serializing the body
        // via `serde_json::to_value` is simplest and preserves camelCase field
        // renames declared on each variant.
        let mut body = serde_json::to_value(&self.body).map_err(serde::ser::Error::custom)?;
        let obj = body.as_object_mut().ok_or_else(|| {
            serde::ser::Error::custom("RpcCommandBody must serialize to a JSON object")
        })?;
        let mut map = ser.serialize_map(Some(obj.len() + usize::from(self.id.is_some())))?;
        if let Some(id) = &self.id {
            map.serialize_entry("id", id)?;
        }
        for (k, v) in obj.iter() {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for RpcCommand {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let mut v = Value::deserialize(de)?;
        let obj = v
            .as_object_mut()
            .ok_or_else(|| de::Error::custom("expected object"))?;
        let id = match obj.remove("id") {
            Some(Value::String(s)) => Some(s),
            Some(Value::Null) | None => None,
            Some(other) => {
                return Err(de::Error::custom(format!("id must be string, got {other}")));
            }
        };
        let body: RpcCommandBody =
            serde_json::from_value(Value::Object(obj.clone())).map_err(de::Error::custom)?;
        Ok(Self { id, body })
    }
}

/// Discriminated union of every command OMP 17.2.10 accepts on stdin.
///
/// Optional fields use `skip_serializing_if = "Option::is_none"` so absent
/// values never appear on the wire; camelCase renames match `rpc-types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcCommandBody {
    NegotiateProtocol {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
    },

    Prompt {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<Value>>,
        #[serde(
            default,
            rename = "streamingBehavior",
            skip_serializing_if = "Option::is_none"
        )]
        streaming_behavior: Option<StreamingBehavior>,
    },
    Steer {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<Value>>,
    },
    FollowUp {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<Value>>,
    },
    Abort,
    AbortAndPrompt {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<Value>>,
    },
    NewSession {
        #[serde(
            default,
            rename = "parentSession",
            skip_serializing_if = "Option::is_none"
        )]
        parent_session: Option<String>,
    },

    GetState,
    SetFastMode {
        enabled: bool,
    },
    GetAvailableCommands,
    SetTodos {
        phases: Value,
    },
    SetHostTools {
        tools: Value,
    },
    SetHostUriSchemes {
        schemes: Value,
    },
    SetSubagentSubscription {
        level: SubagentSubscriptionLevel,
    },
    GetSubagents,
    GetSubagentMessages {
        #[serde(
            default,
            rename = "subagentId",
            skip_serializing_if = "Option::is_none"
        )]
        subagent_id: Option<String>,
        #[serde(
            default,
            rename = "sessionFile",
            skip_serializing_if = "Option::is_none"
        )]
        session_file: Option<String>,
        #[serde(default, rename = "fromByte", skip_serializing_if = "Option::is_none")]
        from_byte: Option<u64>,
    },

    SetModel {
        provider: String,
        #[serde(rename = "modelId")]
        model_id: String,
    },
    CycleModel,
    GetAvailableModels,

    SetThinkingLevel {
        /// `ThinkingLevel` is a provider-defined string; kept as `Value` for
        /// forward-compat with configured selectors like `"auto"` maps.
        level: Value,
    },
    CycleThinkingLevel,

    SetSteeringMode {
        mode: QueueMode,
    },
    SetFollowUpMode {
        mode: QueueMode,
    },
    SetInterruptMode {
        mode: InterruptMode,
    },

    Compact {
        #[serde(
            default,
            rename = "customInstructions",
            skip_serializing_if = "Option::is_none"
        )]
        custom_instructions: Option<String>,
    },
    SetAutoCompaction {
        enabled: bool,
    },

    SetAutoRetry {
        enabled: bool,
    },
    AbortRetry,

    Bash {
        command: String,
    },
    AbortBash,

    GetSessionStats,
    ExportHtml {
        #[serde(
            default,
            rename = "outputPath",
            skip_serializing_if = "Option::is_none"
        )]
        output_path: Option<String>,
    },
    SwitchSession {
        #[serde(rename = "sessionPath")]
        session_path: String,
    },
    Branch {
        #[serde(rename = "entryId")]
        entry_id: String,
    },
    GetBranchMessages,
    GetLastAssistantText,
    SetSessionName {
        name: String,
    },
    Handoff {
        #[serde(
            default,
            rename = "customInstructions",
            skip_serializing_if = "Option::is_none"
        )]
        custom_instructions: Option<String>,
    },

    GetMessages,
    GetMessagesPage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },

    GetLoginProviders,
    Login {
        #[serde(rename = "providerId")]
        provider_id: String,
    },

    /// Extension UI response — sent on the same stdin channel; shape depends on
    /// the request method. Kept lossless.
    ExtensionUiResponse {
        id: String,
        #[serde(flatten)]
        fields: Map<String, Value>,
    },

    /// Host tool result reply.
    HostToolResult {
        id: String,
        result: Value,
        #[serde(default, rename = "isError", skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    HostToolUpdate {
        id: String,
        #[serde(rename = "partialResult")]
        partial_result: Value,
    },
    HostUriResult {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(
            default,
            rename = "contentType",
            skip_serializing_if = "Option::is_none"
        )]
        content_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notes: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        immutable: Option<bool>,
        #[serde(default, rename = "isError", skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Assistant message events.
// ---------------------------------------------------------------------------

string_enum! {
    pub enum DoneReason {
        Stop => "stop",
        Length => "length",
        ToolUse => "toolUse",
    }
}

string_enum! {
    pub enum ErrorReason {
        Aborted => "aborted",
        Error => "error",
    }
}

/// Discriminated `assistantMessageEvent` variants from 17.2.10.
///
/// Every variant retains the original JSON in [`AssistantMessageEvent::raw`];
/// stable scalar fields (`contentIndex`, `delta`, `content`, `reason`) are
/// promoted for consumers that don't want to poke at the raw map. `partial`,
/// `message`, `toolCall`, and image content stay in `raw` and are exposed by
/// convenience accessors below.
#[derive(Debug, Clone, PartialEq)]
pub struct AssistantMessageEvent {
    pub kind: AssistantMessageEventKind,
    /// The full event JSON object, verbatim.
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssistantMessageEventKind {
    Start,
    TextStart { content_index: u64 },
    TextDelta { content_index: u64, delta: String },
    TextEnd { content_index: u64, content: String },
    ThinkingStart { content_index: u64 },
    ThinkingDelta { content_index: u64, delta: String },
    ThinkingEnd { content_index: u64, content: String },
    ImageEnd { content_index: u64 },
    ToolcallStart { content_index: u64 },
    ToolcallDelta { content_index: u64, delta: String },
    ToolcallEnd { content_index: u64 },
    Done { reason: DoneReason },
    Error { reason: ErrorReason },
    Unknown { type_field: Option<String> },
}

impl AssistantMessageEvent {
    /// Full `partial` message, if present. Kept as `Value` because assistant
    /// content blocks are provider-defined and open-ended.
    #[must_use]
    pub fn partial(&self) -> Option<&Value> {
        self.raw.get("partial")
    }
    /// Terminal `message` on `done`.
    #[must_use]
    pub fn done_message(&self) -> Option<&Value> {
        self.raw.get("message")
    }
    /// Terminal `error` payload on `error`.
    #[must_use]
    pub fn error_message(&self) -> Option<&Value> {
        self.raw.get("error")
    }
    /// `toolCall` object emitted with `toolcall_end`.
    #[must_use]
    pub fn tool_call(&self) -> Option<&Value> {
        self.raw.get("toolCall")
    }
    /// Image content object emitted with `image_end`.
    #[must_use]
    pub fn image_content(&self) -> Option<&Value> {
        self.raw.get("content")
    }
}

impl Serialize for AssistantMessageEvent {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(ser)
    }
}

impl<'de> Deserialize<'de> for AssistantMessageEvent {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(de)?;
        let obj = raw
            .as_object()
            .ok_or_else(|| de::Error::custom("assistantMessageEvent must be an object"))?;
        let ty = obj.get("type").and_then(Value::as_str);
        let ci = || -> Result<u64, D::Error> {
            obj.get("contentIndex")
                .and_then(Value::as_u64)
                .ok_or_else(|| de::Error::custom("missing contentIndex"))
        };
        let delta = || -> Result<String, D::Error> {
            obj.get("delta")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| de::Error::custom("missing delta"))
        };
        let content = || -> Result<String, D::Error> {
            obj.get("content")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| de::Error::custom("missing content"))
        };
        let kind = match ty {
            Some("start") => AssistantMessageEventKind::Start,
            Some("text_start") => AssistantMessageEventKind::TextStart {
                content_index: ci()?,
            },
            Some("text_delta") => AssistantMessageEventKind::TextDelta {
                content_index: ci()?,
                delta: delta()?,
            },
            Some("text_end") => AssistantMessageEventKind::TextEnd {
                content_index: ci()?,
                content: content()?,
            },
            Some("thinking_start") => AssistantMessageEventKind::ThinkingStart {
                content_index: ci()?,
            },
            Some("thinking_delta") => AssistantMessageEventKind::ThinkingDelta {
                content_index: ci()?,
                delta: delta()?,
            },
            Some("thinking_end") => AssistantMessageEventKind::ThinkingEnd {
                content_index: ci()?,
                content: content()?,
            },
            Some("image_end") => AssistantMessageEventKind::ImageEnd {
                content_index: ci()?,
            },
            Some("toolcall_start") => AssistantMessageEventKind::ToolcallStart {
                content_index: ci()?,
            },
            Some("toolcall_delta") => AssistantMessageEventKind::ToolcallDelta {
                content_index: ci()?,
                delta: delta()?,
            },
            Some("toolcall_end") => AssistantMessageEventKind::ToolcallEnd {
                content_index: ci()?,
            },
            Some("done") => {
                let reason: DoneReason =
                    serde_json::from_value(obj.get("reason").cloned().unwrap_or(Value::Null))
                        .map_err(de::Error::custom)?;
                AssistantMessageEventKind::Done { reason }
            }
            Some("error") => {
                let reason: ErrorReason =
                    serde_json::from_value(obj.get("reason").cloned().unwrap_or(Value::Null))
                        .map_err(de::Error::custom)?;
                AssistantMessageEventKind::Error { reason }
            }
            other => AssistantMessageEventKind::Unknown {
                type_field: other.map(str::to_owned),
            },
        };
        Ok(AssistantMessageEvent { kind, raw })
    }
}

// ---------------------------------------------------------------------------
// Tool execution lifecycle frames.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolExecutionStartFrame {
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub args: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolExecutionUpdateFrame {
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub args: Value,
    #[serde(rename = "partialResult")]
    pub partial_result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolExecutionEndFrame {
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub result: Value,
    /// Optional per the core `AgentEvent` source; absence is preserved and MUST
    /// NOT default to `false` in the protocol model.
    #[serde(default, rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

// ---------------------------------------------------------------------------
// Message lifecycle & other Tier-1 session events.
// ---------------------------------------------------------------------------

/// `message_update` — carries typed discriminant of the inner
/// `assistantMessageEvent` plus the enveloping `message` as lossless JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageUpdateFrame {
    pub assistant_message_event: AssistantMessageEvent,
    /// Full `AgentMessage` (assistant message with all content blocks).
    pub message: Value,
    pub raw: Value,
}

impl Serialize for MessageUpdateFrame {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(ser)
    }
}

impl<'de> Deserialize<'de> for MessageUpdateFrame {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(de)?;
        let obj = raw
            .as_object()
            .ok_or_else(|| de::Error::custom("message_update must be an object"))?;
        let evt_raw = obj
            .get("assistantMessageEvent")
            .ok_or_else(|| de::Error::custom("message_update missing assistantMessageEvent"))?
            .clone();
        let assistant_message_event: AssistantMessageEvent =
            serde_json::from_value(evt_raw).map_err(de::Error::custom)?;
        let message = obj.get("message").cloned().unwrap_or(Value::Null);
        Ok(MessageUpdateFrame {
            assistant_message_event,
            message,
            raw,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentEndFrame {
    pub messages: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Value>,
    #[serde(
        default,
        rename = "isTerminal",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_terminal: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnEndFrame {
    pub message: Value,
    #[serde(rename = "toolResults")]
    pub tool_results: Value,
}

string_enum! {
    pub enum NoticeLevel {
        Info => "info",
        Warning => "warning",
        Error => "error",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoticeFrame {
    pub level: NoticeLevel,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// `thinking_level_changed` — every field is optional per 17.2.10.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThinkingLevelChangedFrame {
    #[serde(
        default,
        rename = "thinkingLevel",
        skip_serializing_if = "Option::is_none"
    )]
    pub thinking_level: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configured: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptResultFrame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "agentInvoked")]
    pub agent_invoked: bool,
}

// ---------------------------------------------------------------------------
// Extension UI requests.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionUiRequestFrame {
    pub id: String,
    pub method: ExtensionUiMethod,
    pub raw: Value,
}

/// Every 17.2.10 UI method with its typed stable fields.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionUiMethod {
    Select {
        title: String,
        options: Vec<String>,
        timeout: Option<f64>,
    },
    Confirm {
        title: String,
        message: String,
        timeout: Option<f64>,
    },
    Input {
        title: String,
        placeholder: Option<String>,
        timeout: Option<f64>,
    },
    Editor {
        title: String,
        prefill: Option<String>,
        prompt_style: Option<bool>,
    },
    Cancel {
        target_id: String,
    },
    Notify {
        message: String,
        notify_type: Option<NoticeLevel>,
    },
    SetStatus {
        status_key: String,
        status_text: Option<String>,
    },
    SetWidget {
        widget_key: String,
        widget_lines: Option<Vec<String>>,
        widget_placement: Option<String>,
    },
    SetTitle {
        title: String,
    },
    SetEditorText {
        text: String,
    },
    OpenUrl {
        url: String,
        launch_url: Option<String>,
        instructions: Option<String>,
    },
    Unknown {
        method: Option<String>,
    },
}

impl Serialize for ExtensionUiRequestFrame {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(ser)
    }
}

impl<'de> Deserialize<'de> for ExtensionUiRequestFrame {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(de)?;
        let obj = raw
            .as_object()
            .ok_or_else(|| de::Error::custom("extension_ui_request must be an object"))?;
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| de::Error::custom("extension_ui_request missing id"))?;
        let method_str = obj.get("method").and_then(Value::as_str);
        let s = |k: &str| obj.get(k).and_then(Value::as_str).map(str::to_owned);
        let s_req = |k: &str| -> Result<String, D::Error> {
            s(k).ok_or_else(|| de::Error::custom(format!("missing {k}")))
        };
        let f = |k: &str| obj.get(k).and_then(Value::as_f64);
        let b = |k: &str| obj.get(k).and_then(Value::as_bool);
        let method = match method_str {
            Some("select") => ExtensionUiMethod::Select {
                title: s_req("title")?,
                options: obj
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                timeout: f("timeout"),
            },
            Some("confirm") => ExtensionUiMethod::Confirm {
                title: s_req("title")?,
                message: s_req("message")?,
                timeout: f("timeout"),
            },
            Some("input") => ExtensionUiMethod::Input {
                title: s_req("title")?,
                placeholder: s("placeholder"),
                timeout: f("timeout"),
            },
            Some("editor") => ExtensionUiMethod::Editor {
                title: s_req("title")?,
                prefill: s("prefill"),
                prompt_style: b("promptStyle"),
            },
            Some("cancel") => ExtensionUiMethod::Cancel {
                target_id: s_req("targetId")?,
            },
            Some("notify") => ExtensionUiMethod::Notify {
                message: s_req("message")?,
                notify_type: match s("notifyType") {
                    Some(t) => {
                        Some(serde_json::from_value(Value::String(t)).map_err(de::Error::custom)?)
                    }
                    None => None,
                },
            },
            Some("setStatus") => ExtensionUiMethod::SetStatus {
                status_key: s_req("statusKey")?,
                status_text: s("statusText"),
            },
            Some("setWidget") => ExtensionUiMethod::SetWidget {
                widget_key: s_req("widgetKey")?,
                widget_lines: obj.get("widgetLines").and_then(Value::as_array).map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                }),
                widget_placement: s("widgetPlacement"),
            },
            Some("setTitle") => ExtensionUiMethod::SetTitle {
                title: s_req("title")?,
            },
            Some("set_editor_text") => ExtensionUiMethod::SetEditorText {
                text: s_req("text")?,
            },
            Some("open_url") => ExtensionUiMethod::OpenUrl {
                url: s_req("url")?,
                launch_url: s("launchUrl"),
                instructions: s("instructions"),
            },
            other => ExtensionUiMethod::Unknown {
                method: other.map(str::to_owned),
            },
        };
        Ok(ExtensionUiRequestFrame { id, method, raw })
    }
}

// ---------------------------------------------------------------------------
// Host tool & host URI frames.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostToolCallRequest {
    pub id: String,
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostToolCancelRequest {
    pub id: String,
    #[serde(rename = "targetId")]
    pub target_id: String,
}

string_enum! {
    pub enum HostUriOperation {
        Read => "read",
        Write => "write",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostUriRequest {
    pub id: String,
    pub operation: HostUriOperation,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostUriCancelRequest {
    pub id: String,
    #[serde(rename = "targetId")]
    pub target_id: String,
}

// ---------------------------------------------------------------------------
// Subagent frames.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentPayloadFrame {
    /// Opaque `SubagentLifecyclePayload | SubagentProgressPayload | SubagentEventPayload`.
    pub payload: Value,
}

// ---------------------------------------------------------------------------
// Top-level incoming frame envelope.
// ---------------------------------------------------------------------------

/// Every parsed inbound RPC frame. Consumers dispatch on [`IncomingFrame::kind`]
/// and reach into [`IncomingFrame::raw`] for open payload data. The `raw`
/// field always contains the full original JSON object so unknown extras on
/// known variants (and entirely unknown frame types) round-trip without loss.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingFrame {
    pub kind: IncomingFrameKind,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IncomingFrameKind {
    // Transport
    Ready(ReadyFrame),
    Response(RpcResponse),
    RpcChunk(RpcChunkFrame),
    RpcFrameError(RpcFrameErrorFrame),

    // Agent lifecycle
    AgentStart,
    AgentEnd(AgentEndFrame),
    TurnStart,
    TurnEnd(TurnEndFrame),
    MessageStart,
    MessageUpdate(MessageUpdateFrame),
    MessageEnd,

    // Tool execution
    ToolExecutionStart(ToolExecutionStartFrame),
    ToolExecutionUpdate(ToolExecutionUpdateFrame),
    ToolExecutionEnd(ToolExecutionEndFrame),

    // Session-level (Tier-1)
    AutoCompactionStart,
    AutoCompactionEnd,
    AutoRetryStart,
    AutoRetryEnd,
    RetryFallbackApplied,
    RetryFallbackSucceeded,
    ModelChanged,
    ThinkingLevelChanged(ThinkingLevelChangedFrame),
    TtsrTriggered,
    TodoReminder,
    TodoAutoClear,
    IrcMessage,
    Notice(NoticeFrame),
    GoalUpdated,

    // Side channels
    PromptResult(PromptResultFrame),
    AvailableCommandsUpdate,
    CommandOutput,
    SessionInfoUpdate,
    ConfigUpdate,
    ExtensionError,

    // Extension UI
    ExtensionUiRequest(ExtensionUiRequestFrame),

    // Host tool / URI
    HostToolCall(HostToolCallRequest),
    HostToolCancel(HostToolCancelRequest),
    HostUriRequest(HostUriRequest),
    HostUriCancel(HostUriCancelRequest),

    // Subagents
    SubagentLifecycle(SubagentPayloadFrame),
    SubagentProgress(SubagentPayloadFrame),
    SubagentEvent(SubagentPayloadFrame),

    /// Frame with a `type` we don't recognize. Raw JSON is preserved verbatim
    /// in the enclosing [`IncomingFrame::raw`].
    Unknown {
        type_field: Option<String>,
    },
}

impl IncomingFrame {
    /// Returns the wire `type` string, if any.
    pub fn type_str(&self) -> Option<&str> {
        self.raw.get("type").and_then(Value::as_str)
    }
}

impl Serialize for IncomingFrame {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(ser)
    }
}

impl<'de> Deserialize<'de> for IncomingFrame {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct FrameVisitor;
        impl<'de> Visitor<'de> for FrameVisitor {
            type Value = IncomingFrame;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an RPC frame object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<IncomingFrame, A::Error> {
                let mut fields: BTreeMap<String, Value> = BTreeMap::new();
                while let Some((k, v)) = map.next_entry::<String, Value>()? {
                    fields.insert(k, v);
                }
                let raw = Value::Object(fields.into_iter().collect::<Map<_, _>>());
                decode_incoming(raw).map_err(de::Error::custom)
            }
        }
        de.deserialize_map(FrameVisitor)
    }
}

fn decode_incoming(raw: Value) -> Result<IncomingFrame, serde_json::Error> {
    let ty = raw.get("type").and_then(Value::as_str).map(str::to_owned);
    let kind = match ty.as_deref() {
        Some("ready") => IncomingFrameKind::Ready(serde_json::from_value(raw.clone())?),
        Some("response") => IncomingFrameKind::Response(serde_json::from_value(raw.clone())?),
        Some("rpc_chunk") => IncomingFrameKind::RpcChunk(serde_json::from_value(raw.clone())?),
        Some("rpc_frame_error") => {
            IncomingFrameKind::RpcFrameError(serde_json::from_value(raw.clone())?)
        }

        Some("agent_start") => IncomingFrameKind::AgentStart,
        Some("agent_end") => IncomingFrameKind::AgentEnd(serde_json::from_value(raw.clone())?),
        Some("turn_start") => IncomingFrameKind::TurnStart,
        Some("turn_end") => IncomingFrameKind::TurnEnd(serde_json::from_value(raw.clone())?),
        Some("message_start") => IncomingFrameKind::MessageStart,
        Some("message_update") => {
            IncomingFrameKind::MessageUpdate(serde_json::from_value(raw.clone())?)
        }
        Some("message_end") => IncomingFrameKind::MessageEnd,

        Some("tool_execution_start") => {
            IncomingFrameKind::ToolExecutionStart(serde_json::from_value(raw.clone())?)
        }
        Some("tool_execution_update") => {
            IncomingFrameKind::ToolExecutionUpdate(serde_json::from_value(raw.clone())?)
        }
        Some("tool_execution_end") => {
            IncomingFrameKind::ToolExecutionEnd(serde_json::from_value(raw.clone())?)
        }

        Some("auto_compaction_start") => IncomingFrameKind::AutoCompactionStart,
        Some("auto_compaction_end") => IncomingFrameKind::AutoCompactionEnd,
        Some("auto_retry_start") => IncomingFrameKind::AutoRetryStart,
        Some("auto_retry_end") => IncomingFrameKind::AutoRetryEnd,
        Some("retry_fallback_applied") => IncomingFrameKind::RetryFallbackApplied,
        Some("retry_fallback_succeeded") => IncomingFrameKind::RetryFallbackSucceeded,
        Some("model_changed") => IncomingFrameKind::ModelChanged,
        Some("thinking_level_changed") => {
            IncomingFrameKind::ThinkingLevelChanged(serde_json::from_value(raw.clone())?)
        }
        Some("ttsr_triggered") => IncomingFrameKind::TtsrTriggered,
        Some("todo_reminder") => IncomingFrameKind::TodoReminder,
        Some("todo_auto_clear") => IncomingFrameKind::TodoAutoClear,
        Some("irc_message") => IncomingFrameKind::IrcMessage,
        Some("notice") => IncomingFrameKind::Notice(serde_json::from_value(raw.clone())?),
        Some("goal_updated") => IncomingFrameKind::GoalUpdated,

        Some("prompt_result") => {
            IncomingFrameKind::PromptResult(serde_json::from_value(raw.clone())?)
        }
        Some("available_commands_update") => IncomingFrameKind::AvailableCommandsUpdate,
        Some("command_output") => IncomingFrameKind::CommandOutput,
        Some("session_info_update") => IncomingFrameKind::SessionInfoUpdate,
        Some("config_update") => IncomingFrameKind::ConfigUpdate,
        Some("extension_error") => IncomingFrameKind::ExtensionError,

        Some("extension_ui_request") => {
            IncomingFrameKind::ExtensionUiRequest(serde_json::from_value(raw.clone())?)
        }

        Some("host_tool_call") => {
            IncomingFrameKind::HostToolCall(serde_json::from_value(raw.clone())?)
        }
        Some("host_tool_cancel") => {
            IncomingFrameKind::HostToolCancel(serde_json::from_value(raw.clone())?)
        }
        Some("host_uri_request") => {
            IncomingFrameKind::HostUriRequest(serde_json::from_value(raw.clone())?)
        }
        Some("host_uri_cancel") => {
            IncomingFrameKind::HostUriCancel(serde_json::from_value(raw.clone())?)
        }

        Some("subagent_lifecycle") => {
            IncomingFrameKind::SubagentLifecycle(serde_json::from_value(raw.clone())?)
        }
        Some("subagent_progress") => {
            IncomingFrameKind::SubagentProgress(serde_json::from_value(raw.clone())?)
        }
        Some("subagent_event") => {
            IncomingFrameKind::SubagentEvent(serde_json::from_value(raw.clone())?)
        }

        other => IncomingFrameKind::Unknown {
            type_field: other.map(str::to_owned),
        },
    };
    Ok(IncomingFrame { kind, raw })
}

/// Parse one already-reassembled logical frame from a raw JSON object.
///
/// # Errors
///
/// Returns an error if the raw value cannot be interpreted as an incoming
/// frame (structural mismatch during typed field extraction).
pub fn decode_frame(raw: Value) -> Result<IncomingFrame, serde_json::Error> {
    decode_incoming(raw)
}
