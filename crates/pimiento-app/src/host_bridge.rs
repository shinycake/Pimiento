use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gpui::{
    ClickEvent, Context, IntoElement as _, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    label::Label,
    scroll::ScrollableElement as _,
    v_flex,
};
use omp_rpc_client::{
    client::RpcClient,
    frames::{IncomingFrame, IncomingFrameKind, RpcCommandBody},
};
use pimiento_core::transcript::TranscriptEntry;
use serde_json::{Value, json};

use crate::SessionView;

pub(crate) const HOST_BRIDGE_ENV: &str = "PIMIENTO_HOST_BRIDGE";
pub(crate) const OPEN_FILE_TOOL_NAME: &str = "pimiento.open_file";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingHostToolCall {
    pub(crate) request_id: String,
    pub(crate) tool_call_id: String,
    pub(crate) tool_name: String,
    pub(crate) arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingHostUriRequest {
    pub(crate) request_id: String,
    pub(crate) operation: String,
    pub(crate) url: String,
}

#[derive(Debug)]
pub(crate) struct HostBridgeState {
    pub(crate) enabled: bool,
    pub(crate) pending_calls: Vec<PendingHostToolCall>,
    pub(crate) pending_uri_requests: Vec<PendingHostUriRequest>,
    in_flight: std::collections::HashSet<String>,
}

impl HostBridgeState {
    pub(crate) fn from_environment() -> Self {
        Self::new(host_bridge_enabled_value(
            std::env::var_os(HOST_BRIDGE_ENV).as_deref(),
        ))
    }

    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            pending_calls: Vec::new(),
            pending_uri_requests: Vec::new(),
            in_flight: std::collections::HashSet::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.pending_calls.clear();
        self.pending_uri_requests.clear();
        self.in_flight.clear();
    }

    pub(crate) fn has_pending_requests(&self) -> bool {
        !self.pending_calls.is_empty() || !self.pending_uri_requests.is_empty()
    }

    pub(crate) fn observe_frame(&mut self, frame: &IncomingFrame) {
        if !self.enabled {
            return;
        }

        match &frame.kind {
            IncomingFrameKind::HostToolCall(request) => {
                if self
                    .pending_calls
                    .iter()
                    .any(|call| call.request_id == request.id)
                    || self.in_flight.contains(&request.id)
                {
                    return;
                }
                self.pending_calls.push(PendingHostToolCall {
                    request_id: request.id.clone(),
                    tool_call_id: request.tool_call_id.clone(),
                    tool_name: request.tool_name.clone(),
                    arguments: request.arguments.clone(),
                });
            }
            IncomingFrameKind::HostToolCancel(cancel) => {
                self.cancel_target(&cancel.target_id);
            }
            IncomingFrameKind::HostUriRequest(request) => {
                if self
                    .pending_uri_requests
                    .iter()
                    .any(|pending| pending.request_id == request.id)
                    || self.in_flight.contains(&request.id)
                {
                    return;
                }
                let operation = frame
                    .raw
                    .get("operation")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned();
                self.pending_uri_requests.push(PendingHostUriRequest {
                    request_id: request.id.clone(),
                    operation,
                    url: request.url.clone(),
                });
            }
            IncomingFrameKind::HostUriCancel(cancel) => {
                self.cancel_target(&cancel.target_id);
            }
            _ => {}
        }
    }

    fn cancel_target(&mut self, target_id: &str) {
        self.pending_calls
            .retain(|call| call.request_id != target_id);
        self.pending_uri_requests
            .retain(|request| request.request_id != target_id);
        self.in_flight.remove(target_id);
    }

    fn take_pending(&mut self, request_id: &str) -> Option<PendingHostToolCall> {
        let index = self
            .pending_calls
            .iter()
            .position(|call| call.request_id == request_id)?;
        let call = self.pending_calls.remove(index);
        self.in_flight.insert(request_id.to_owned());
        Some(call)
    }

    fn finish_in_flight(&mut self, request_id: &str) -> bool {
        self.in_flight.remove(request_id)
    }

    fn take_pending_uri(&mut self, request_id: &str) -> Option<PendingHostUriRequest> {
        let index = self
            .pending_uri_requests
            .iter()
            .position(|request| request.request_id == request_id)?;
        let request = self.pending_uri_requests.remove(index);
        self.in_flight.insert(request_id.to_owned());
        Some(request)
    }
}

pub(crate) fn host_bridge_enabled_value(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

pub(crate) fn host_tool_definitions() -> Value {
    json!([{
        "name": OPEN_FILE_TOOL_NAME,
        "label": "Open File in Pimiento",
        "description": "Request that Pimiento open an existing absolute local file in the host's default application. The user must approve every request.",
        "parameters": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path of the existing local file to open"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }
    }])
}

pub(crate) fn register_host_bridge(client: &RpcClient) -> Result<(), String> {
    if !host_bridge_enabled_value(std::env::var_os(HOST_BRIDGE_ENV).as_deref()) {
        return Ok(());
    }

    let response = smol::block_on(async {
        client
            .send(RpcCommandBody::SetHostTools {
                tools: host_tool_definitions(),
            })
            .await
    })
    .map_err(|error| error.to_string())?;

    if !response.success {
        return Err(response
            .error
            .unwrap_or_else(|| "OMP rejected set_host_tools".to_owned()));
    }

    let registered = response
        .data
        .as_ref()
        .and_then(|data| data.get("toolNames"))
        .and_then(Value::as_array)
        .is_some_and(|names| {
            names
                .iter()
                .any(|name| name.as_str() == Some(OPEN_FILE_TOOL_NAME))
        });
    if !registered {
        return Err(format!(
            "OMP did not confirm registration of {OPEN_FILE_TOOL_NAME}"
        ));
    }

    Ok(())
}

pub(crate) fn open_file_path(arguments: &Value) -> Result<PathBuf, String> {
    let path = arguments
        .as_object()
        .and_then(|arguments| arguments.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "pimiento.open_file requires a non-empty string `path`".to_owned())?;
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err("pimiento.open_file requires an absolute path".to_owned());
    }
    Ok(path)
}

fn execute_open_file(arguments: &Value) -> Result<String, String> {
    let path = open_file_path(arguments)?;
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }

    open_path_command(&path)?;
    Ok(format!(
        "Opened {} in the host's default application.",
        path.display()
    ))
}

fn open_path_command(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = Command::new("xdg-open");
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return Err("pimiento.open_file is supported only on macOS and Linux".to_owned());

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        // `--` prevents a path starting with `-` from being parsed as a flag.
        let status = command
            .arg("--")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("Failed to launch file opener: {error}"))?;
        if !status.success() {
            return Err(format!("File opener exited with {status}"));
        }
        Ok(())
    }
}

pub(crate) fn host_tool_result_frame(request_id: &str, message: &str, is_error: bool) -> Value {
    serde_json::to_value(RpcCommandBody::HostToolResult {
        id: request_id.to_owned(),
        result: json!({
            "content": [{
                "type": "text",
                "text": message
            }]
        }),
        is_error: is_error.then_some(true),
    })
    .expect("typed host tool result serializes")
}

pub(crate) fn host_uri_denied_frame(request_id: &str, message: &str) -> Value {
    serde_json::to_value(RpcCommandBody::HostUriResult {
        id: request_id.to_owned(),
        content: None,
        content_type: None,
        notes: None,
        immutable: None,
        is_error: Some(true),
        error: Some(message.to_owned()),
    })
    .expect("typed host URI result serializes")
}

pub(crate) fn host_arguments_summary(arguments: &Value) -> String {
    serde_json::to_string_pretty(arguments).unwrap_or_else(|_| "<invalid JSON>".to_owned())
}

impl SessionView {
    pub(crate) fn observe_host_bridge_frame(&mut self, frame: &IncomingFrame) {
        self.host_bridge.observe_frame(frame);
    }

    pub(crate) fn approve_host_tool(&mut self, request_id: &str, cx: &mut Context<Self>) {
        let Some(call) = self.host_bridge.take_pending(request_id) else {
            return;
        };
        let Some(client) = self.client.clone() else {
            self.host_bridge.finish_in_flight(request_id);
            return;
        };

        if call.tool_name != OPEN_FILE_TOOL_NAME {
            self.complete_host_tool(
                client,
                call.request_id,
                format!("Unsupported host tool: {}", call.tool_name),
                true,
                cx,
            );
            return;
        }

        let request_id = call.request_id;
        let arguments = call.arguments;
        cx.spawn(async move |view, cx| {
            let outcome = smol::unblock(move || execute_open_file(&arguments)).await;
            let (message, is_error) = match outcome {
                Ok(message) => (message, false),
                Err(error) => (error, true),
            };
            let should_send = view
                .update(cx, |this, _cx| {
                    this.host_bridge.finish_in_flight(&request_id)
                })
                .unwrap_or(false);
            if !should_send {
                return;
            }
            let frame = host_tool_result_frame(&request_id, &message, is_error);
            if let Err(error) = client.send_raw(frame).await {
                let _ = view.update(cx, |this, cx| {
                    this.projection.transcript.push(TranscriptEntry::Error {
                        message: format!("Failed to send host tool result: {error}"),
                        code: Some("host_tool_result".to_owned()),
                    });
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn deny_host_tool(&mut self, request_id: &str, cx: &mut Context<Self>) {
        let Some(call) = self.host_bridge.take_pending(request_id) else {
            return;
        };
        let Some(client) = self.client.clone() else {
            self.host_bridge.finish_in_flight(request_id);
            return;
        };
        self.complete_host_tool(
            client,
            call.request_id,
            format!("User denied host tool request for {}.", call.tool_name),
            true,
            cx,
        );
    }

    pub(crate) fn deny_host_uri(&mut self, request_id: &str, cx: &mut Context<Self>) {
        let Some(request) = self.host_bridge.take_pending_uri(request_id) else {
            return;
        };
        let Some(client) = self.client.clone() else {
            self.host_bridge.finish_in_flight(request_id);
            return;
        };
        let message = format!(
            "Pimiento has no registered host URI handler for {}.",
            request.url
        );
        cx.spawn(async move |view, cx| {
            let should_send = view
                .update(cx, |this, _cx| {
                    this.host_bridge.finish_in_flight(&request.request_id)
                })
                .unwrap_or(false);
            if !should_send {
                return;
            }
            let frame = host_uri_denied_frame(&request.request_id, &message);
            if let Err(error) = client.send_raw(frame).await {
                let _ = view.update(cx, |this, cx| {
                    this.projection.transcript.push(TranscriptEntry::Error {
                        message: format!("Failed to send host URI result: {error}"),
                        code: Some("host_uri_result".to_owned()),
                    });
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    #[allow(
        clippy::unused_self,
        reason = "instance listener helper keeps host-tool completion call sites uniform"
    )]
    fn complete_host_tool(
        &mut self,
        client: RpcClient,
        request_id: String,
        message: String,
        is_error: bool,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |view, cx| {
            let should_send = view
                .update(cx, |this, _cx| {
                    this.host_bridge.finish_in_flight(&request_id)
                })
                .unwrap_or(false);
            if !should_send {
                return;
            }
            let frame = host_tool_result_frame(&request_id, &message, is_error);
            if let Err(error) = client.send_raw(frame).await {
                let _ = view.update(cx, |this, cx| {
                    this.projection.transcript.push(TranscriptEntry::Error {
                        message: format!("Failed to send host tool result: {error}"),
                        code: Some("host_tool_result".to_owned()),
                    });
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }
}

pub(crate) fn render_host_tool_call(
    call: &PendingHostToolCall,
    index: usize,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let request_id_deny = call.request_id.clone();
    let request_id_approve = call.request_id.clone();
    let supported = call.tool_name == OPEN_FILE_TOOL_NAME;

    v_flex()
        .w_full()
        .p_4()
        .gap_3()
        .rounded_md()
        .bg(theme.secondary)
        .border_1()
        .border_color(theme.border)
        .child(
            Label::new("Host tool approval")
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .overflow_x_scrollbar()
                .text_xs()
                .font_family(theme.mono_font_family.clone())
                .child(call.tool_name.clone()),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .max_h(px(240.))
                .overflow_scrollbar()
                .text_xs()
                .text_color(theme.muted_foreground)
                .font_family(theme.mono_font_family.clone())
                .child(host_arguments_summary(&call.arguments)),
        )
        .when(!supported, |card| {
            card.child(
                Label::new("Pimiento does not implement this host tool.")
                    .text_xs()
                    .text_color(theme.warning),
            )
        })
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .justify_end()
                .gap_2()
                .child(
                    Button::new(("deny-host-tool", index))
                        .label("Deny")
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.deny_host_tool(&request_id_deny, cx);
                        })),
                )
                .child(
                    Button::new(("approve-host-tool", index))
                        .label("Approve")
                        .small()
                        .primary()
                        .disabled(!supported)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.approve_host_tool(&request_id_approve, cx);
                        })),
                ),
        )
        .into_any_element()
}

pub(crate) fn render_host_uri_request(
    request: &PendingHostUriRequest,
    index: usize,
    cx: &mut Context<SessionView>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let request_id = request.request_id.clone();
    v_flex()
        .w_full()
        .p_4()
        .gap_3()
        .rounded_md()
        .bg(theme.secondary)
        .border_1()
        .border_color(theme.border)
        .child(
            Label::new("Unsupported host URI request")
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .overflow_x_scrollbar()
                .text_xs()
                .font_family(theme.mono_font_family.clone())
                .child(format!("{} {}", request.operation, request.url)),
        )
        .child(
            Label::new("No host URI scheme is registered. Deny this request to return an error.")
                .text_xs()
                .text_color(theme.muted_foreground),
        )
        .child(
            h_flex().w_full().justify_end().child(
                Button::new(("deny-host-uri", index))
                    .label("Deny")
                    .small()
                    .ghost()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.deny_host_uri(&request_id, cx);
                    })),
            ),
        )
        .into_any_element()
}
