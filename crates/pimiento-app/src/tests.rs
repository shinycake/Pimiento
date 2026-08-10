use super::*;

#[test]
fn host_bridge_flag_is_explicit_opt_in() {
    assert!(host_bridge_enabled_value(Some(OsStr::new("1"))));
    assert!(!host_bridge_enabled_value(None));
    assert!(!host_bridge_enabled_value(Some(OsStr::new("true"))));
    assert!(!host_bridge_enabled_value(Some(OsStr::new("0"))));
}

#[test]
fn open_file_host_tool_registration_matches_omp_schema() {
    assert_eq!(
        host_tool_definitions(),
        serde_json::json!([{
            "name": "pimiento.open_file",
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
    );
}

#[test]
fn host_tool_call_is_queued_only_when_bridge_is_enabled_and_cancel_dismisses_it() {
    let call = omp_rpc_client::frames::decode_frame(serde_json::json!({
        "type": "host_tool_call",
        "id": "host-1",
        "toolCallId": "tool-1",
        "toolName": "pimiento.open_file",
        "arguments": {"path": "/tmp/example.txt"}
    }))
    .expect("host tool call decodes");
    let cancel = omp_rpc_client::frames::decode_frame(serde_json::json!({
        "type": "host_tool_cancel",
        "id": "cancel-1",
        "targetId": "host-1"
    }))
    .expect("host tool cancel decodes");

    let mut disabled = HostBridgeState::new(false);
    disabled.observe_frame(&call);
    assert!(disabled.pending_calls.is_empty());

    let mut enabled = HostBridgeState::new(true);
    enabled.observe_frame(&call);
    assert_eq!(enabled.pending_calls.len(), 1);
    assert_eq!(enabled.pending_calls[0].tool_call_id, "tool-1");
    enabled.observe_frame(&cancel);
    assert!(enabled.pending_calls.is_empty());
}

#[test]
fn unsupported_host_uri_request_waits_for_deny_and_cancel_dismisses_it() {
    let request = omp_rpc_client::frames::decode_frame(serde_json::json!({
        "type": "host_uri_request",
        "id": "uri-1",
        "operation": "read",
        "url": "example://resource"
    }))
    .expect("host URI request decodes");
    let cancel = omp_rpc_client::frames::decode_frame(serde_json::json!({
        "type": "host_uri_cancel",
        "id": "uri-cancel-1",
        "targetId": "uri-1"
    }))
    .expect("host URI cancel decodes");

    let mut state = HostBridgeState::new(true);
    state.observe_frame(&request);
    assert!(state.has_pending_requests());
    assert_eq!(state.pending_uri_requests[0].operation, "read");
    state.observe_frame(&cancel);
    assert!(!state.has_pending_requests());
    assert_eq!(
        host_uri_denied_frame("uri-1", "unsupported"),
        serde_json::json!({
            "type": "host_uri_result",
            "id": "uri-1",
            "isError": true,
            "error": "unsupported"
        })
    );
}

#[test]
fn host_tool_result_and_open_file_arguments_are_strict() {
    assert_eq!(
        open_file_path(&serde_json::json!({"path": "/tmp/a.txt"})),
        Ok(PathBuf::from("/tmp/a.txt"))
    );
    assert!(open_file_path(&serde_json::json!({"path": "relative.txt"})).is_err());
    assert!(open_file_path(&serde_json::json!({"path": 7})).is_err());

    assert_eq!(
        host_tool_result_frame("host-1", "denied", true),
        serde_json::json!({
            "type": "host_tool_result",
            "id": "host-1",
            "result": {"content": [{"type": "text", "text": "denied"}]},
            "isError": true
        })
    );
    assert_eq!(
        host_tool_result_frame("host-1", "opened", false),
        serde_json::json!({
            "type": "host_tool_result",
            "id": "host-1",
            "result": {"content": [{"type": "text", "text": "opened"}]}
        })
    );
}

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
fn queue_mode_cycles_between_all_and_one_at_a_time() {
    assert_eq!(cycle_queue_mode(Some("all")), QueueMode::OneAtATime);
    assert_eq!(cycle_queue_mode(Some("one-at-a-time")), QueueMode::All);
    assert_eq!(cycle_queue_mode(None), QueueMode::OneAtATime);
    assert_eq!(cycle_queue_mode(Some("future-mode")), QueueMode::OneAtATime);
}

#[test]
fn interrupt_mode_cycles_between_immediate_and_wait() {
    assert_eq!(cycle_interrupt_mode(Some("immediate")), InterruptMode::Wait);
    assert_eq!(cycle_interrupt_mode(Some("wait")), InterruptMode::Immediate);
    assert_eq!(cycle_interrupt_mode(None), InterruptMode::Immediate);
}

#[test]
fn version_gate_notice_only_formats_outside_tested_baseline() {
    assert_eq!(format_version_gate_notice(MIN_SUPPORTED), None);
    assert_eq!(format_version_gate_notice(MAX_SUPPORTED), None);

    let below = OmpVersion {
        major: 17,
        minor: 2,
        patch: 9,
    };
    assert_eq!(
        format_version_gate_notice(below).as_deref(),
        Some(
            "Pimiento was tested with omp 17.2.10–17.2.11; you have 17.2.9 — unknown events will still render"
        )
    );

    let newer = OmpVersion {
        major: 17,
        minor: 3,
        patch: 0,
    };
    assert_eq!(
        format_version_gate_notice(newer).as_deref(),
        Some(
            "Pimiento was tested with omp 17.2.10–17.2.11; you have 17.3.0 — unknown events will still render"
        )
    );
}

#[test]
fn workspace_digit_key_maps_one_through_nine() {
    assert_eq!(workspace_digit_key("1"), Some(1));
    assert_eq!(workspace_digit_key("9"), Some(9));
    assert_eq!(workspace_digit_key("0"), None);
    assert_eq!(workspace_digit_key("a"), None);
}

#[test]
fn classify_messages_page_error_detects_busy_and_stale() {
    assert_eq!(
        classify_messages_page_error(Some("session_busy"), None),
        MessagesPageErrorKind::Busy
    );
    assert_eq!(
        classify_messages_page_error(Some("stale_cursor"), None),
        MessagesPageErrorKind::Stale
    );
    assert_eq!(
        classify_messages_page_error(None, Some("RPC message cursor is stale")),
        MessagesPageErrorKind::Stale
    );
    assert_eq!(
        classify_messages_page_error(None, Some("boom")),
        MessagesPageErrorKind::Other
    );
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
fn parse_omp_model_roles_yaml_reads_provider_id_pairs() {
    let roles = parse_omp_model_roles_yaml(
        r#"
modelRoles:
  task: cursor/gpt-5.6-luna
  draft: openai/gpt-4.1-mini
  smol: cursor/composer-2.5
  broken: not-a-label
  empty: ""
modelTags:
  draft:
    name: "Draft work"
    color: warning
other: ignore
"#,
    );
    assert_eq!(roles.len(), 3);
    assert_eq!(roles[0].name, "draft");
    assert_eq!(roles[0].display_name, "Draft work");
    assert_eq!(roles[0].color, OmpRoleColor::Warning);
    assert_eq!(roles[0].provider, "openai");
    assert_eq!(roles[1].name, "smol");
    assert_eq!(roles[1].display_name, "Fast");
    assert_eq!(roles[1].color, OmpRoleColor::Warning);
    assert_eq!(roles[2].name, "task");
    assert_eq!(roles[2].display_name, "Subtask");
    assert_eq!(roles[2].color, OmpRoleColor::Muted);
    assert!(parse_omp_model_roles_yaml("not: yaml: [[[").is_empty());
}

#[test]
fn model_supports_fast_mode_matches_omp_service_tier_families() {
    assert!(model_supports_fast_mode("openai", None, "gpt-4.1"));
    assert!(model_supports_fast_mode(
        "anthropic",
        Some("anthropic-messages"),
        "claude-opus-4"
    ));
    assert!(model_supports_fast_mode(
        "openrouter",
        None,
        "anthropic/claude-3-haiku"
    ));
    assert!(!model_supports_fast_mode(
        "cursor",
        None,
        "cursor-grok-4.5-high"
    ));
    assert!(!model_supports_fast_mode("cursor", None, "composer-2.5"));
}

#[test]
fn pending_images_to_wire_uses_mime_and_base64_data() {
    assert_eq!(
        image_mime_for_path(Path::new("shot.PNG")),
        Some("image/png")
    );
    assert_eq!(image_mime_for_path(Path::new("notes.txt")), None);
    assert_eq!(image_mime_for_path(Path::new("legacy.bmp")), None);
    assert!(is_supported_image_path(Path::new("shot.webp")));
    assert!(!is_supported_image_path(Path::new("notes.txt")));

    let attachments = [
        PendingAttachment::Image {
            path: Some(PathBuf::from("/tmp/a.png")),
            mime: "image/png".into(),
            width: 64,
            height: 48,
            data_b64: "Zm9v".into(),
            label: "a.png".into(),
            marker_index: 1,
        },
        PendingAttachment::PathMention {
            path: PathBuf::from("/tmp/notes.txt"),
            display: "@/tmp/notes.txt".into(),
        },
    ];
    let images = pending_images_to_wire(&attachments);
    assert_eq!(
        images,
        vec![serde_json::json!({
            "type": "image",
            "mimeType": "image/png",
            "data": "Zm9v",
        })]
    );
}

#[test]
fn encode_image_for_rpc_matches_omp_budget_and_returns_dims() {
    let mut png = Vec::new();
    {
        // Above min edge so auto-resize can keep the original when under budget.
        let img = image::RgbImage::from_pixel(240, 240, image::Rgb([20, 40, 60]));
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        dyn_img
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("write png");
    }
    let encoded = encode_image_for_rpc_with_auto_resize(&png, true).expect("encode");
    assert!(
        encoded.mime == "image/png" || encoded.mime == "image/jpeg" || encoded.mime == "image/webp"
    );
    assert!(!encoded.data_b64.is_empty());
    assert!(encoded.data_b64.len() < 700_000);
    assert!(encoded.width >= 200);
    assert!(encoded.height >= 200);
    assert!(encoded.width <= 1568);
    assert!(encoded.height <= 1568);

    let kept = encode_image_for_rpc_with_auto_resize(&png, false).expect("no-resize");
    assert_eq!(kept.mime, "image/png");
    assert_eq!(kept.width, 240);
    assert_eq!(kept.height, 240);
}

#[test]
fn image_markers_and_paste_helpers() {
    assert_eq!(image_marker(2, 800, 600), "[Image #2, 800x600]");
    assert_eq!(wrap_attachment("hi"), "<attachment>\nhi\n</attachment>");
    assert_eq!(inline_paste_marker(1, 30, 900), "[Paste #1, +30 lines]");
    assert_eq!(inline_paste_marker(3, 4, 120), "[Paste #3, 120 chars]");
    let _threshold = large_paste_threshold(); // peeks config; default 100

    let attachments = [PendingAttachment::Image {
        path: None,
        mime: "image/png".into(),
        width: 10,
        height: 20,
        data_b64: "Zm9v".into(),
        label: "clip".into(),
        marker_index: 1,
    }];
    let composed = compose_message_with_image_markers("look", &attachments);
    assert!(composed.contains("[Image #1, 10x20]"));
    assert!(composed.starts_with("look"));

    let already = compose_message_with_image_markers("see [Image #1, 10x20] please", &attachments);
    assert_eq!(already, "see [Image #1, 10x20] please");
}

#[test]
fn load_pending_attachment_path_mention_for_non_images() {
    let att = load_pending_attachment(Path::new("/tmp/readme.md"), 1).expect("path mention");
    match att {
        PendingAttachment::PathMention { path, display } => {
            assert_eq!(path, PathBuf::from("/tmp/readme.md"));
            assert_eq!(display, "@/tmp/readme.md");
        }
        PendingAttachment::Image { .. } => panic!("expected PathMention"),
    }
}

#[test]
fn compose_and_strip_attachment_markers() {
    let attachments = [
        PendingAttachment::Image {
            path: None,
            mime: "image/png".into(),
            width: 100,
            height: 80,
            data_b64: "Zm9v".into(),
            label: "a".into(),
            marker_index: 1,
        },
        PendingAttachment::PathMention {
            path: PathBuf::from("src/main.rs"),
            display: "@src/main.rs".into(),
        },
    ];
    let composed = compose_message_with_attachments("hello", &attachments);
    assert!(composed.contains("[Image #1, 100x80]"));
    assert!(composed.contains("@src/main.rs"));
    assert_eq!(
        strip_image_marker("see [Image #1, 100x80] please", 1),
        "see please"
    );
    assert_eq!(
        strip_path_mention("read @src/main.rs now", "@src/main.rs"),
        "read now"
    );
    assert_eq!(at_mention_query("look at @src/m"), Some("src/m"));
    assert_eq!(at_mention_query("email me@host.com"), None);
    assert_eq!(
        format_file_mention_summary(&serde_json::json!({
            "role": "fileMention",
            "files": [{"path": "a.rs", "lineCount": 12}],
        })),
        Some("File mention: a.rs (12 lines)".into())
    );
}

#[test]
fn roles_matching_model_finds_assigned_roles() {
    let roles = vec![
        OmpRole {
            name: "default".into(),
            display_name: "Default".into(),
            provider: "cursor".into(),
            id: "cursor-grok-4.5-high".into(),
            color: OmpRoleColor::Success,
        },
        OmpRole {
            name: "smol".into(),
            display_name: "Fast".into(),
            provider: "cursor".into(),
            id: "composer-2.5".into(),
            color: OmpRoleColor::Warning,
        },
    ];
    let matched = roles_matching_model(&roles, Some("cursor/cursor-grok-4.5-high"));
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].name, "default");
}

#[test]
fn rail_close_forgets_recent_session_file() {
    let root = std::env::temp_dir().join(format!(
        "pimiento-rail-forget-{}-{}",
        std::process::id(),
        current_unix_seconds()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp persistence root");
    let persistence = SessionPersistence::from_root(root.clone());
    persistence.remember_recent_session(
        Some("/tmp/rail-session.jsonl"),
        Some(Path::new("/tmp/work")),
        Some("work"),
    );
    assert_eq!(persistence.load_recent_sessions().len(), 1);
    // Rail × closes the in-app tab and forgets Pimiento recent.json only —
    // never deletes OMP session files under ~/.omp.
    persistence.forget_session(Path::new("/tmp/rail-session.jsonl"));
    assert!(persistence.load_recent_sessions().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn todo_count_only_includes_open_and_in_progress() {
    let raw = serde_json::json!([{
        "name": "Ship",
        "tasks": [
            {"content": "Open", "status": "open"},
            {"content": "Running", "status": "in_progress"},
            {"content": "Done", "status": "completed"},
            {"content": "Blocked", "status": "blocked"}
        ]
    }]);
    let phases = parse_todo_phases(Some(&raw));
    assert_eq!(todo_open_count(&phases), 2);
}

#[test]
fn toggle_todo_in_phases_json_cycles_status() {
    let raw = serde_json::json!([{
        "name": "Ship",
        "tasks": [
            {"content": "Build", "status": "pending"},
            {"content": "Done", "status": "completed"}
        ]
    }]);
    let toggled = toggle_todo_in_phases_json(&raw, 0, 0).expect("toggle");
    assert_eq!(toggled[0]["tasks"][0]["status"].as_str(), Some("completed"));
    let toggled_back = toggle_todo_in_phases_json(&toggled, 0, 1).expect("toggle back");
    assert_eq!(
        toggled_back[0]["tasks"][1]["status"].as_str(),
        Some("pending")
    );
}

#[test]
fn parse_branch_and_login_wire_shapes() {
    let branch = parse_branch_messages(Some(&serde_json::json!({
        "messages": [
            {"entryId": "e1", "text": "hello\nworld"},
            {"entryId": "", "text": "skip"},
            {"entryId": "e2", "text": "second"}
        ]
    })));
    assert_eq!(branch.len(), 2);
    assert_eq!(branch[0].entry_id, "e1");
    assert_eq!(branch_message_preview(&branch[0].text, 8), "hello wo…");

    let providers = parse_login_providers(Some(&serde_json::json!({
        "providers": [
            {"id": "openai", "name": "OpenAI", "available": true, "authenticated": false},
            {"id": "gh", "available": false, "authenticated": true}
        ]
    })));
    assert_eq!(providers.len(), 2);
    assert_eq!(providers[1].name, "gh");
    assert!(!providers[1].available);
}

#[test]
fn inspector_extra_status_lines_are_honest() {
    let lines = inspector_extra_status_lines(Some(&serde_json::json!({
        "queuedMessageCount": 2,
        "messageCount": 10,
        "tokens": {"input": 100, "output": 50},
        "cost": 0.0123
    })));
    assert!(lines.iter().any(|l| l == "Queue: 2"));
    assert!(lines.iter().any(|l| l.contains("Tokens: 100 in / 50 out")));
    assert!(lines.iter().any(|l| l.starts_with("Cost: $")));
    assert!(inspector_extra_status_lines(None).is_empty());
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
        context_percent(Some(&serde_json::json!({"percent": 120.0}))),
        Some(100.0)
    );
    assert_eq!(
        context_percent(Some(&serde_json::json!({"percent": -1.0}))),
        Some(0.0)
    );
    assert_eq!(
        tokens_per_second_label(Some(&serde_json::json!({"tokensPerSecond": 12.34}))).as_deref(),
        Some("12.3")
    );
    assert_eq!(tokens_per_second_label(Some(&serde_json::json!(0.0))), None);
}

#[test]
fn context_high_starts_at_eighty_percent() {
    assert!(!context_high(None));
    assert!(!context_high(Some(&serde_json::json!({"percent": 79.99}))));
    assert!(context_high(Some(&serde_json::json!({"percent": 80.0}))));
    assert!(context_high(Some(&serde_json::json!(95.0))));
    assert!(!context_high(Some(
        &serde_json::json!({"percent": "unknown"})
    )));
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
fn rail_attention_prioritizes_activity_then_unread() {
    for phase in [
        RunPhase::Streaming,
        RunPhase::AwaitingResume,
        RunPhase::Compacting,
        RunPhase::Retrying,
    ] {
        assert_eq!(classify_rail_attention(&phase, 3), RailAttention::Active);
    }
    assert_eq!(
        classify_rail_attention(&RunPhase::Idle, 1),
        RailAttention::Unread
    );
    assert_eq!(
        classify_rail_attention(&RunPhase::Restarting, 0),
        RailAttention::Quiet
    );
    assert_eq!(
        classify_rail_attention(&RunPhase::Dead, 0),
        RailAttention::Quiet
    );
}

#[test]
fn workspace_title_includes_authoritative_name_and_phase() {
    assert_eq!(
        workspace_window_title("Pimiento", &RunPhase::Idle),
        "Pimiento · idle"
    );
    assert_eq!(
        workspace_window_title("Fix renderer", &RunPhase::Streaming),
        "Pimiento — Fix renderer · streaming"
    );
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
fn shell_single_quote_escapes_embedded_quotes() {
    assert_eq!(shell_single_quote("plain"), "'plain'");
    let quoted = shell_single_quote("a'b");
    assert_eq!(quoted.len(), 8);
    assert_eq!(&quoted[..2], "'a");
    assert_eq!(&quoted[2..6], "'\\''");
    assert_eq!(&quoted[6..], "b'");
    assert_eq!(
        revert_command_for_path("crates/x.rs"),
        "git restore --worktree -- 'crates/x.rs'"
    );
}

#[test]
fn filter_palette_entries_matches_label_and_hint() {
    let hits = filter_palette_entries("about");
    assert!(hits.iter().any(|e| e.id == PaletteActionId::About));
    let hits = filter_palette_entries("theme");
    assert!(hits.iter().any(|e| e.id == PaletteActionId::ToggleTheme));
    let hits = filter_palette_entries("rail");
    assert!(hits.iter().any(|e| e.id == PaletteActionId::ToggleRail));
    let hits = filter_palette_entries("inspector");
    assert!(
        hits.iter()
            .any(|e| e.id == PaletteActionId::ToggleInspector)
    );
    let hits = filter_palette_entries("home folder");
    assert!(hits.iter().any(|e| e.id == PaletteActionId::RevealLogs));
    let hits = filter_palette_entries("tokens cost");
    assert!(hits.iter().any(|e| e.id == PaletteActionId::SessionStats));
    let hits = filter_palette_entries("terminal");
    assert!(hits.iter().any(|e| e.id == PaletteActionId::Handoff));
    let hits = filter_palette_entries("share");
    assert!(hits.iter().any(|e| e.id == PaletteActionId::ShareSession));
    let hits = filter_palette_entries("branch from turn");
    assert!(hits.iter().any(|e| e.id == PaletteActionId::BranchSession));
    assert!(filter_palette_entries("zzzz-nope").is_empty());
}

#[test]
fn native_menu_spec_has_product_labels_and_action_mapping() {
    let menus = app_menus();
    assert_eq!(
        menus
            .iter()
            .map(|menu| menu.name.as_ref())
            .collect::<Vec<_>>(),
        MENU_TITLES
    );

    let action_name = |menu_name: &str, item_name: &str| {
        menus
            .iter()
            .find(|menu| menu.name.as_ref() == menu_name)
            .and_then(|menu| {
                menu.items.iter().find_map(|item| match item {
                    gpui::MenuItem::Action { name, action, .. } if name.as_ref() == item_name => {
                        Some(action.name())
                    }
                    _ => None,
                })
            })
    };

    assert_eq!(
        action_name("Pimiento", "About Pimiento"),
        Some("pimiento_menu::AboutPimiento")
    );
    assert_eq!(
        action_name("File", "Open Workspace…"),
        Some("pimiento_menu::OpenWorkspace")
    );
    assert_eq!(
        action_name("View", "Command Palette…"),
        Some("pimiento_menu::OpenCommandPalette")
    );
    assert_eq!(
        action_name("Session", "Branch/Fork from Turn…"),
        Some("pimiento_menu::BranchFromTurn")
    );
    assert_eq!(
        action_name("Window", "Enter Full Screen"),
        Some("pimiento_menu::EnterFullScreen")
    );
    assert_eq!(action_name("Edit", "Undo"), Some("input::Undo"));
    assert_eq!(action_name("Edit", "Paste"), Some("input::Paste"));
    assert_eq!(action_name("Edit", "Select All"), Some("input::SelectAll"));

    let app_menu = &menus[0];
    assert!(app_menu.items.iter().any(|item| matches!(
        item,
        gpui::MenuItem::SystemMenu(menu)
            if menu.name.as_ref() == "Services"
                && menu.menu_type == gpui::SystemMenuType::Services
    )));
}

#[test]
fn native_slash_catalog_maps_only_supported_gui_actions() {
    let mappings = native_slash_catalog()
        .iter()
        .map(|entry| (entry.name, entry.action))
        .collect::<Vec<_>>();
    assert_eq!(
        mappings,
        vec![
            ("/fork", PaletteActionId::BranchSession),
            ("/branch", PaletteActionId::BranchSession),
            ("/new", PaletteActionId::NewSession),
            ("/resume", PaletteActionId::SessionsLauncher),
            ("/login", PaletteActionId::LoginProviders),
            ("/setup", PaletteActionId::LoginProviders),
            ("/switch", PaletteActionId::ToggleModels),
            ("/agents", PaletteActionId::ToggleAgents),
            ("/hotkeys", PaletteActionId::About),
            ("/help", PaletteActionId::About),
            ("/handoff", PaletteActionId::Handoff),
            ("/theme", PaletteActionId::ToggleTheme),
        ]
    );
}

#[test]
fn native_slash_filter_matches_slash_name_and_description() {
    assert_eq!(filter_native_slash_entries("/fork")[0].name, "/fork");
    assert!(
        filter_native_slash_entries("prior turn")
            .iter()
            .any(|entry| entry.name == "/fork")
    );
    assert!(
        filter_native_slash_entries("keyboard shortcuts")
            .iter()
            .any(|entry| entry.name == "/hotkeys")
    );
    assert!(filter_native_slash_entries("unsupported-tui-command").is_empty());
}

#[test]
fn mount_notices_are_detected_without_inventing_content() {
    assert!(notice_looks_like_mount_event(
        "cli_8: mounted mcp_node_repl_js, mcp_node_repl_js.reset"
    ));
    assert!(!notice_looks_like_mount_event("compaction finished"));
}

#[test]
fn thinking_collapse_preview_uses_first_wire_line() {
    assert_eq!(
        thinking_collapse_preview("  first beat\nsecond"),
        "Thinking · first beat"
    );
    assert_eq!(thinking_collapse_preview("   \n"), "Thinking · expand");
    let long = "x".repeat(80);
    let preview = thinking_collapse_preview(&long);
    assert!(preview.starts_with("Thinking · "));
    assert!(preview.ends_with('…'));
    assert_eq!(
        preview.chars().count(),
        "Thinking · ".chars().count() + 56 + 1
    );
}

#[test]
fn soft_wrap_dynamic_text_preserves_every_visible_character() {
    let original =
        "/very/long/path/with/a-model-name-that-keeps-going-without-visible-clipping.json";
    let wrapped = soft_wrap_dynamic_text(original);
    assert!(wrapped.contains('\u{200b}'));
    assert_eq!(wrapped.replace('\u{200b}', ""), original);
    assert!(!wrapped.contains('…'));
}

#[test]
fn subagent_detail_helpers_keep_full_wire_text() {
    let detail = "x".repeat(320);
    let value = serde_json::json!({"text": detail});
    assert_eq!(compact_subagent_value(&value), detail);

    let message = serde_json::json!({"role": "assistant", "content": detail});
    let digest = subagent_message_digest(&message);
    assert_eq!(digest, format!("assistant: {detail}"));
    assert!(!digest.contains('…'));
}

#[test]
fn host_argument_detail_is_pretty_and_never_truncated() {
    let detail = "argument".repeat(80);
    let rendered = host_arguments_summary(&serde_json::json!({"path": detail}));
    assert!(rendered.contains(&detail));
    assert!(rendered.contains('\n'));
    assert!(!rendered.contains('…'));
}

#[test]
fn groups_sessions_by_workspace_name_and_preserves_session_order() {
    let entries = vec![
        RailEntry {
            ix: 2,
            label: "later".to_owned(),
            phase: RunPhase::Idle,
            cwd: PathBuf::from("/tmp/zulu"),
            attention: RailAttention::Quiet,
            session_file: None,
        },
        RailEntry {
            ix: 1,
            label: "second".to_owned(),
            phase: RunPhase::Streaming,
            cwd: PathBuf::from("/tmp/alpha"),
            attention: RailAttention::Active,
            session_file: None,
        },
        RailEntry {
            ix: 0,
            label: "first".to_owned(),
            phase: RunPhase::Idle,
            cwd: PathBuf::from("/tmp/alpha"),
            attention: RailAttention::Unread,
            session_file: None,
        },
    ];

    let groups = group_sessions_by_workspace(entries);

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].0, PathBuf::from("/tmp/alpha"));
    assert_eq!(
        groups[0].1.iter().map(|entry| entry.ix).collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(groups[1].0, PathBuf::from("/tmp/zulu"));
    assert_eq!(groups[1].1[0].ix, 2);
}

#[test]
fn rail_cwd_label_uses_tilde_and_end_truncates() {
    let home = PathBuf::from("/home/ada");
    assert_eq!(rail_cwd_label(Path::new("/home/ada"), Some(&home)), "~");
    assert_eq!(
        rail_cwd_label(Path::new("/home/ada/code/pimiento"), Some(&home)),
        "~/code/pimiento"
    );
    assert_eq!(
        rail_cwd_label(Path::new("/tmp/elsewhere"), Some(&home)),
        "/tmp/elsewhere"
    );
    let long = PathBuf::from(
        "/home/ada/very/long/nested/path/that/should/not/fit/in/the/narrow/rail/column",
    );
    let labeled = rail_cwd_label(&long, Some(&home));
    assert!(labeled.starts_with('…'), "{labeled}");
    assert!(labeled.ends_with("rail/column"), "{labeled}");
    assert!(labeled.chars().count() <= 34, "{labeled}");
}

#[test]
fn workspace_status_rollup_uses_child_phase_priority() {
    let entry = |ix, phase| RailEntry {
        ix,
        label: format!("session-{ix}"),
        phase,
        cwd: PathBuf::from("/tmp/workspace"),
        attention: RailAttention::Quiet,
        session_file: None,
    };
    let entries = vec![
        entry(0, RunPhase::Idle),
        entry(1, RunPhase::Streaming),
        entry(2, RunPhase::AwaitingResume),
        entry(3, RunPhase::Dead),
    ];
    assert_eq!(workspace_status_for_entries(&entries), StatusKind::Error);
    assert_eq!(
        workspace_status_for_entries(&entries[..3]),
        StatusKind::AwaitingInput
    );
    assert_eq!(
        workspace_status_for_entries(&entries[..2]),
        StatusKind::Working
    );
    assert_eq!(
        workspace_status_for_entries(&entries[..1]),
        StatusKind::Idle
    );
}

#[test]
fn inspector_groups_known_tools_and_detects_mode_tags() {
    let names = vec![
        "bash".to_owned(),
        "read".to_owned(),
        "mcp_linear".to_owned(),
        "browser_open".to_owned(),
    ];
    let groups = group_tool_names(&names);
    assert_eq!(groups.builtin, vec!["bash", "read"]);
    assert_eq!(groups.extensions, vec!["mcp_linear", "browser_open"]);
    assert_eq!(
        mode_indicators(&names, &["computer-status".to_owned(), "vision".to_owned()]),
        vec!["Computer", "Browser", "Vision"]
    );
}

#[test]
fn tool_visual_classification_covers_semantic_families_and_fallback() {
    assert_eq!(tool_visual_kind("bash"), ToolVisualKind::Terminal);
    assert_eq!(tool_visual_kind("read"), ToolVisualKind::ReadFile);
    assert_eq!(tool_visual_kind("ast_edit"), ToolVisualKind::WriteFile);
    assert_eq!(tool_visual_kind("grep"), ToolVisualKind::Search);
    assert_eq!(tool_visual_kind("task"), ToolVisualKind::Agent);
    assert_eq!(tool_visual_kind("web_search"), ToolVisualKind::Web);
    assert_eq!(tool_visual_kind("hub"), ToolVisualKind::Hub);
    assert_eq!(tool_visual_kind("ask"), ToolVisualKind::Ask);
    assert_eq!(tool_visual_kind("todo"), ToolVisualKind::Todo);
    assert_eq!(tool_visual_kind("plugin_custom"), ToolVisualKind::Generic);
}

#[test]
fn subagent_subscription_cycles_off_progress_events() {
    assert_eq!(
        next_subagent_subscription_level(&SubagentSubscriptionLevel::Off),
        SubagentSubscriptionLevel::Progress
    );
    assert_eq!(
        next_subagent_subscription_level(&SubagentSubscriptionLevel::Progress),
        SubagentSubscriptionLevel::Events
    );
    assert_eq!(
        next_subagent_subscription_level(&SubagentSubscriptionLevel::Events),
        SubagentSubscriptionLevel::Off
    );
}

#[test]
fn subagent_snapshot_refresh_preserves_only_an_explicit_present_selection() {
    let snapshots = vec![serde_json::json!({
        "id": "worker-1",
        "agent": "task",
        "status": "working",
        "description": "Reviewing the UI"
    })];
    assert_eq!(retained_subagent_selection(None, &snapshots), None);
    assert_eq!(
        retained_subagent_selection(Some("worker-1"), &snapshots).as_deref(),
        Some("worker-1")
    );
    assert_eq!(
        retained_subagent_selection(Some("worker-2"), &snapshots),
        None
    );
}

#[test]
fn subagent_events_refresh_snapshots_only_for_unseen_agents() {
    let snapshots = vec![serde_json::json!({"id": "worker-1"})];
    assert!(!subagent_event_needs_snapshot_refresh(
        &serde_json::json!({"id": "worker-1"}),
        &snapshots
    ));
    assert!(subagent_event_needs_snapshot_refresh(
        &serde_json::json!({"subagentId": "worker-2"}),
        &snapshots
    ));
    assert!(subagent_event_needs_snapshot_refresh(
        &serde_json::json!({"kind": "started"}),
        &[]
    ));
    assert!(!subagent_event_needs_snapshot_refresh(
        &serde_json::json!({"kind": "progress"}),
        &snapshots
    ));
}

#[test]
fn theme_preference_from_env_parses_known_values() {
    assert_eq!(
        theme_preference_from_env(Some("light")),
        ThemePreference::Light
    );
    assert_eq!(
        theme_preference_from_env(Some("DARK")),
        ThemePreference::Dark
    );
    assert_eq!(
        theme_preference_from_env(Some("system")),
        ThemePreference::System
    );
    assert_eq!(theme_preference_from_env(None), ThemePreference::System);
    assert_eq!(
        theme_preference_from_env(Some("nope")),
        ThemePreference::System
    );
}

#[test]
fn persisted_ui_theme_parses_and_serializes_lowercase_values() {
    let parsed: PersistedUi =
        serde_json::from_str(r#"{"inspector_open":false,"theme":"dark"}"#).expect("parse ui");
    assert_eq!(parsed.theme, ThemePreference::Dark);
    assert_eq!(parsed.light_theme, DEFAULT_LIGHT_THEME);
    assert_eq!(parsed.dark_theme, DEFAULT_DARK_THEME);
    assert!(!parsed.rail_collapsed);

    let legacy: PersistedUi =
        serde_json::from_str(r#"{"inspector_open":true}"#).expect("parse legacy ui");
    assert_eq!(legacy.theme, ThemePreference::System);
    assert!(!legacy.rail_collapsed);

    let value = serde_json::to_value(PersistedUi {
        inspector_open: true,
        rail_collapsed: true,
        theme: ThemePreference::Light,
        light_theme: "Pepperwood Light".into(),
        dark_theme: "Pepperwood Dark".into(),
    })
    .expect("serialize ui");
    assert_eq!(value["theme"], "light");
    assert_eq!(value["rail_collapsed"], true);
    assert_eq!(value["light_theme"], "Pepperwood Light");
    assert_eq!(value["dark_theme"], "Pepperwood Dark");
}

#[test]
fn theme_family_pairing_matches_modes_and_preserves_unknown_families() {
    let themes = vec![
        ("Quiet Pepper Light".into(), ThemeMode::Light),
        ("Quiet Pepper Dark".into(), ThemeMode::Dark),
        ("Unpaired Dark".into(), ThemeMode::Dark),
    ];
    assert_eq!(theme_family_name("Quiet Pepper Dark"), "Quiet Pepper");
    assert_eq!(
        paired_theme_name("Quiet Pepper Dark", ThemeMode::Light, &themes).as_deref(),
        Some("Quiet Pepper Light")
    );
    assert_eq!(
        paired_theme_name("Unpaired Dark", ThemeMode::Light, &themes),
        None
    );
}

#[test]
fn bundled_theme_asset_contains_two_complete_light_dark_families() {
    let set: gpui_component::ThemeSet =
        serde_json::from_str(BUNDLED_THEMES).expect("bundled themes parse");
    let names = set
        .themes
        .iter()
        .map(|theme| theme.name.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(set.themes.len(), 4);
    assert!(names.contains(&"Quiet Pepper Light"));
    assert!(names.contains(&"Quiet Pepper Dark"));
    assert!(names.contains(&"Pepperwood Light"));
    assert!(names.contains(&"Pepperwood Dark"));
    assert_eq!(
        set.themes
            .iter()
            .filter(|theme| theme.mode == ThemeMode::Light)
            .count(),
        2
    );
    assert_eq!(
        set.themes
            .iter()
            .filter(|theme| theme.mode == ThemeMode::Dark)
            .count(),
        2
    );
}

#[test]
fn bundled_custom_theme_catalog_contains_zed_defaults_catppuccin_and_dracula() {
    let zed: gpui_component::ThemeSet =
        serde_json::from_str(ZED_DEFAULT_THEMES).expect("Zed themes parse");
    let community: gpui_component::ThemeSet =
        serde_json::from_str(COMMUNITY_THEMES).expect("community themes parse");
    let zed_names = zed
        .themes
        .iter()
        .map(|theme| theme.name.as_ref())
        .collect::<std::collections::HashSet<_>>();
    let community_names = community
        .themes
        .iter()
        .map(|theme| theme.name.as_ref())
        .collect::<std::collections::HashSet<_>>();

    assert_eq!(zed.themes.len(), 11);
    for name in [
        "One Dark",
        "One Light",
        "Ayu Dark",
        "Ayu Light",
        "Ayu Mirage",
        "Gruvbox Dark",
        "Gruvbox Dark Hard",
        "Gruvbox Dark Soft",
        "Gruvbox Light",
        "Gruvbox Light Hard",
        "Gruvbox Light Soft",
    ] {
        assert!(zed_names.contains(name), "missing Zed theme {name}");
    }

    assert_eq!(community.themes.len(), 5);
    for name in [
        "Catppuccin Latte",
        "Catppuccin Frappe",
        "Catppuccin Macchiato",
        "Catppuccin Mocha",
        "Dracula",
    ] {
        assert!(
            community_names.contains(name),
            "missing community theme {name}"
        );
    }
}

#[test]
fn theme_picker_search_and_selected_state_cover_appearance_and_names() {
    let themes = vec![
        RegisteredThemeChoice {
            name: "Quiet Pepper Light".into(),
            mode: ThemeMode::Light,
            swatches: [None, None, None],
        },
        RegisteredThemeChoice {
            name: "Pepperwood Dark".into(),
            mode: ThemeMode::Dark,
            swatches: [None, None, None],
        },
    ];
    let dark = filter_theme_picker_items(&themes, "pepperwood");
    assert_eq!(
        dark,
        vec![ThemePickerItem::Theme {
            name: "Pepperwood Dark".into(),
            mode: ThemeMode::Dark
        }]
    );
    assert_eq!(
        filter_theme_picker_items(&themes, "follow"),
        vec![ThemePickerItem::Appearance(ThemePreference::System)]
    );
    let selection = ThemeSelection {
        appearance: ThemePreference::System,
        light: "Quiet Pepper Light".into(),
        dark: "Pepperwood Dark".into(),
    };
    assert!(theme_picker_item_is_active(
        &ThemePickerItem::Appearance(ThemePreference::System),
        &selection
    ));
    assert!(theme_picker_item_is_active(&dark[0], &selection));
}

#[test]
fn theme_picker_derives_preview_from_opening_selection_and_can_revert_exactly() {
    let opening = ThemeSelection {
        appearance: ThemePreference::System,
        light: "Quiet Pepper Light".into(),
        dark: "Pepperwood Dark".into(),
    };
    let themes = vec![
        ("Quiet Pepper Light".into(), ThemeMode::Light),
        ("Quiet Pepper Dark".into(), ThemeMode::Dark),
        ("Pepperwood Light".into(), ThemeMode::Light),
        ("Pepperwood Dark".into(), ThemeMode::Dark),
    ];
    let preview = theme_selection_for_picker_item(
        &opening,
        &ThemePickerItem::Theme {
            name: "Quiet Pepper Dark".into(),
            mode: ThemeMode::Dark,
        },
        &themes,
    );
    assert_eq!(
        preview,
        ThemeSelection {
            appearance: ThemePreference::Dark,
            light: "Quiet Pepper Light".into(),
            dark: "Quiet Pepper Dark".into(),
        }
    );

    let appearance_preview = theme_selection_for_picker_item(
        &opening,
        &ThemePickerItem::Appearance(ThemePreference::Light),
        &themes,
    );
    assert_eq!(appearance_preview.appearance, ThemePreference::Light);
    assert_eq!(appearance_preview.light, opening.light);
    assert_eq!(appearance_preview.dark, opening.dark);
    assert_eq!(opening.appearance, ThemePreference::System);
}

#[test]
fn theme_picker_selection_index_prefers_the_opening_appearance_or_active_filtered_theme() {
    let selection = ThemeSelection {
        appearance: ThemePreference::Dark,
        light: "Quiet Pepper Light".into(),
        dark: "Pepperwood Dark".into(),
    };
    let all = vec![
        ThemePickerItem::Appearance(ThemePreference::System),
        ThemePickerItem::Appearance(ThemePreference::Light),
        ThemePickerItem::Appearance(ThemePreference::Dark),
    ];
    assert_eq!(theme_picker_selected_index(&all, &selection), 2);

    let filtered = vec![
        ThemePickerItem::Theme {
            name: "Other Dark".into(),
            mode: ThemeMode::Dark,
        },
        ThemePickerItem::Theme {
            name: "Pepperwood Dark".into(),
            mode: ThemeMode::Dark,
        },
    ];
    assert_eq!(theme_picker_selected_index(&filtered, &selection), 1);
    assert_eq!(theme_picker_selected_index(&[], &selection), 0);
}

#[test]
fn initial_theme_selection_overrides_only_appearance_from_environment() {
    let root = std::env::temp_dir().join(format!(
        "pimiento-theme-env-{}-{}",
        std::process::id(),
        current_unix_seconds()
    ));
    let persistence = SessionPersistence::from_root(root.clone());
    persistence.save_theme_selection(&ThemeSelection {
        appearance: ThemePreference::System,
        light: "Pepperwood Light".into(),
        dark: "Pepperwood Dark".into(),
    });
    let selected = initial_theme_selection(Some(OsStr::new("dark")), &persistence);
    assert_eq!(selected.appearance, ThemePreference::Dark);
    assert_eq!(selected.light, "Pepperwood Light");
    assert_eq!(selected.dark, "Pepperwood Dark");
    assert_eq!(themes_directory(&root), root.join("themes"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn workspace_blocks_close_for_every_abortable_phase() {
    for phase in [
        RunPhase::Streaming,
        RunPhase::AwaitingResume,
        RunPhase::Compacting,
        RunPhase::Retrying,
    ] {
        assert!(workspace_should_block_close(&[RunPhase::Idle, phase]));
    }
    assert!(!workspace_should_block_close(&[
        RunPhase::Idle,
        RunPhase::Restarting,
        RunPhase::Dead,
    ]));
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
fn slash_commands_parse_full_live_metadata_shape() {
    let raw = serde_json::json!({
        "commands": [
            {
                "name": "mcp",
                "description": " Manage MCP servers ",
                "aliases": ["servers", "/mcp", "servers"],
                "input": {"hint": " server name "},
                "subcommands": [
                    {
                        "name": "reconnect",
                        "description": " Reconnect a server ",
                        "usage": " <server> "
                    },
                    {
                        "name": "list",
                        "description": "List servers"
                    }
                ],
                "source": "extension"
            },
            "status"
        ]
    });
    let commands = parse_slash_commands(Some(&raw));
    assert_eq!(
        commands,
        vec![
            SlashCommand {
                name: "/mcp".into(),
                description: "Manage MCP servers".into(),
                aliases: vec!["/servers".into()],
                input_hint: Some("server name".into()),
                subcommands: vec![
                    SlashSubcommand {
                        name: "reconnect".into(),
                        description: "Reconnect a server".into(),
                        usage: Some("<server>".into()),
                    },
                    SlashSubcommand {
                        name: "list".into(),
                        description: "List servers".into(),
                        usage: None,
                    },
                ],
                source: Some("extension".into()),
            },
            SlashCommand {
                name: "/status".into(),
                description: String::new(),
                aliases: Vec::new(),
                input_hint: None,
                subcommands: Vec::new(),
                source: None,
            },
        ]
    );

    let array = serde_json::json!([{ "name": "/quit", "aliases": ["q"] }]);
    assert_eq!(parse_slash_commands(Some(&array))[0].name, "/quit");
}

#[test]
fn slash_top_level_filter_matches_aliases_and_caps_after_filtering() {
    let commands = (0..(SLASH_COMMAND_VISIBLE_CAP + 2))
        .map(|ix| SlashCommand {
            name: format!("/command-{ix}"),
            description: String::new(),
            aliases: if ix == 0 {
                vec!["/go".into()]
            } else {
                Vec::new()
            },
            input_hint: None,
            subcommands: Vec::new(),
            source: Some("builtin".into()),
        })
        .collect::<Vec<_>>();
    let matches = filter_slash_commands(&commands, "/GO");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].title, "/command-0");
    assert_eq!(matches[0].completion_text, "/command-0");

    let capped = filter_slash_commands(&commands, "/");
    assert_eq!(capped.len(), SLASH_COMMAND_VISIBLE_CAP);
}

#[test]
fn slash_nested_subcommands_flatten_and_filter_dynamically() {
    let raw = serde_json::json!([{
        "name": "mcp",
        "description": "Manage servers",
        "subcommands": [
            {"name": "reconnect", "description": "Reconnect", "usage": "<server>"},
            {"name": "remove", "description": "Remove", "usage": "<server>"},
            {"name": "list", "description": "List"}
        ],
        "source": "builtin"
    }]);
    let commands = parse_slash_commands(Some(&raw));

    let all = filter_slash_commands(&commands, "/mcp ");
    assert_eq!(all.len(), 3);
    assert!(all.iter().all(|suggestion| suggestion.is_subcommand));
    assert_eq!(all[0].title, "/mcp reconnect");
    assert_eq!(all[0].source.as_deref(), Some("builtin"));

    let filtered = filter_slash_commands(&commands, "/mcp re");
    assert_eq!(
        filtered
            .iter()
            .map(|suggestion| suggestion.title.as_str())
            .collect::<Vec<_>>(),
        vec!["/mcp reconnect", "/mcp remove"]
    );
    assert_eq!(
        filter_slash_commands(&commands, "/mcp list")[0].title,
        "/mcp list"
    );
}

#[test]
fn slash_draft_predicate_tracks_metadata_completion_states() {
    let raw = serde_json::json!([
        {
            "name": "mcp",
            "subcommands": [
                {"name": "reconnect", "description": "Reconnect", "usage": "<server>"},
                {"name": "list", "description": "List"}
            ]
        },
        {"name": "build", "input": {"hint": "<target>"}}
    ]);
    let commands = parse_slash_commands(Some(&raw));

    assert!(slash_draft_is_open(&commands, "/"));
    assert!(slash_draft_is_open(&commands, "  /m"));
    assert!(slash_draft_is_open(&commands, "/mcp "));
    assert!(slash_draft_is_open(&commands, "/mcp re"));
    assert!(!slash_draft_is_open(&commands, "/mcp reconnect server"));
    assert!(!slash_draft_is_open(&commands, "/build target"));
    assert!(!slash_draft_is_open(&commands, "build /"));
    assert!(slash_draft_is_open(&commands, "/future.command"));
}

#[test]
fn slash_enter_accepts_only_when_menu_has_matches() {
    assert_eq!(
        composer_enter_action(true, 1, false),
        ComposerEnterAction::AcceptCompletion
    );
    assert_eq!(
        composer_enter_action(true, 0, false),
        ComposerEnterAction::Send
    );
    assert_eq!(
        composer_enter_action(false, 1, false),
        ComposerEnterAction::Send
    );
}

#[test]
fn secondary_enter_sends_instead_of_accepting_slash_completion() {
    assert_eq!(
        composer_enter_action(true, 1, true),
        ComposerEnterAction::Send
    );
}

#[test]
fn slash_completion_text_distinguishes_required_args_and_argless_leaves() {
    let raw = serde_json::json!([{
        "name": "mcp",
        "aliases": ["servers"],
        "subcommands": [
            {"name": "reconnect", "description": "Reconnect", "usage": "<server>"},
            {"name": "list", "description": "List"}
        ]
    }]);
    let commands = parse_slash_commands(Some(&raw));

    let parent = &filter_slash_commands(&commands, "/servers")[0];
    assert_eq!(slash_completion_text(parent), "/mcp ");
    assert!(parent.expects_input);
    assert_eq!(
        filter_slash_commands(&commands, "/servers ")
            .iter()
            .map(|suggestion| suggestion.title.as_str())
            .collect::<Vec<_>>(),
        vec!["/mcp reconnect", "/mcp list"]
    );

    let required = &filter_slash_commands(&commands, "/mcp re")[0];
    assert_eq!(slash_completion_text(required), "/mcp reconnect ");
    assert!(required.expects_input);

    let argless = &filter_slash_commands(&commands, "/mcp li")[0];
    assert_eq!(slash_completion_text(argless), "/mcp list");
    assert!(!argless.expects_input);
}

#[test]
fn slash_future_source_is_preserved_without_a_closed_enum() {
    let raw = serde_json::json!([{
        "name": "future-command",
        "description": "Discovered at runtime",
        "source": "remote-registry-v2"
    }]);
    let commands = parse_slash_commands(Some(&raw));
    assert_eq!(commands[0].source.as_deref(), Some("remote-registry-v2"));

    let suggestions = filter_slash_commands(&commands, "/future");
    assert_eq!(suggestions[0].title, "/future-command");
    assert_eq!(suggestions[0].source.as_deref(), Some("remote-registry-v2"));
}

#[test]
fn command_palette_combines_static_actions_with_every_top_level_slash_command() {
    let commands = parse_slash_commands(Some(&serde_json::json!([
        {
            "name": "mcp",
            "description": "Manage servers",
            "subcommands": [{"name": "list", "description": "List servers"}]
        },
        {
            "name": "future-command",
            "description": "Discovered at runtime",
            "source": "remote-registry-v2"
        }
    ])));
    let entries = filter_command_palette_entries(&commands, "");

    assert!(matches!(
        entries.first(),
        Some(CommandPaletteEntry::NativeSlash(entry)) if entry.name == "/fork"
    ));
    assert!(entries.iter().any(|entry| matches!(
        entry,
        CommandPaletteEntry::Action(action)
            if action.id == PaletteActionId::ToggleTheme && action.label == "Theme…"
    )));
    let slash_titles = entries
        .iter()
        .filter_map(|entry| match entry {
            CommandPaletteEntry::Slash { suggestion, .. } => Some(suggestion.title.as_str()),
            CommandPaletteEntry::Action(_) | CommandPaletteEntry::NativeSlash(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(slash_titles, vec!["/mcp", "/future-command"]);
}

#[test]
fn command_palette_native_fork_deduplicates_dynamic_fork() {
    let commands = parse_slash_commands(Some(&serde_json::json!([
        {
            "name": "fork",
            "description": "TUI fork command exposed unexpectedly"
        },
        {
            "name": "future-command",
            "description": "Still dynamic"
        }
    ])));
    let entries = filter_command_palette_entries(&commands, "/fork");
    assert_eq!(entries.len(), 1);
    assert!(matches!(
        entries[0],
        CommandPaletteEntry::NativeSlash(entry)
            if entry.name == "/fork" && entry.action == PaletteActionId::BranchSession
    ));
    assert!(!entries.iter().any(|entry| {
        matches!(
            entry,
            CommandPaletteEntry::Slash { suggestion, .. } if suggestion.title == "/fork"
        )
    }));
}

#[test]
fn command_palette_row_data_distinguishes_native_execution_from_dynamic_insertion() {
    let native = CommandPaletteEntry::NativeSlash(
        native_slash_catalog()
            .iter()
            .find(|entry| entry.name == "/fork")
            .expect("native fork"),
    );
    let native_row = command_palette_row_data(&native);
    assert_eq!(native_row.title, "/fork");
    assert_eq!(native_row.metadata, "Pimiento action · runs immediately");
    assert_eq!(native_row.usage, None);

    let dynamic = CommandPaletteEntry::Slash {
        suggestion: SlashSuggestion {
            completion_text: "/mcp reconnect ".into(),
            title: "/mcp reconnect".into(),
            description: "Reconnect a server".into(),
            usage_hint: Some("<server>".into()),
            source: Some("extension".into()),
            expects_input: true,
            is_subcommand: true,
        },
        aliases: vec!["/servers".into()],
    };
    let dynamic_row = command_palette_row_data(&dynamic);
    assert_eq!(dynamic_row.title, "/mcp reconnect");
    assert_eq!(dynamic_row.description, "Reconnect a server");
    assert!(
        dynamic_row
            .metadata
            .starts_with("OMP slash command · selection inserts; Enter sends")
    );
    assert!(dynamic_row.metadata.contains("extension"));
    assert!(dynamic_row.metadata.contains("/servers"));
    assert_eq!(dynamic_row.usage.as_deref(), Some("Usage: <server>"));
}

#[test]
fn command_palette_searches_slash_metadata_aliases_and_nested_subcommands() {
    let commands = parse_slash_commands(Some(&serde_json::json!([{
        "name": "mcp",
        "aliases": ["servers"],
        "description": "Manage MCP servers",
        "input": {"hint": "<operation>"},
        "source": "future-extension-source",
        "subcommands": [
            {
                "name": "reconnect",
                "description": "Reconnect a remote server",
                "usage": "<server-name>"
            },
            {"name": "list", "description": "List active servers"}
        ]
    }])));

    for query in [
        "manage mcp",
        "servers",
        "<operation>",
        "future-extension-source",
    ] {
        assert!(
            filter_command_palette_entries(&commands, query)
                .iter()
                .any(|entry| entry.title() == "/mcp"),
            "top-level metadata did not match {query}"
        );
    }

    for query in [
        "reconnect",
        "remote server",
        "<server-name>",
        "future-extension-source",
        "servers",
    ] {
        assert!(
            filter_command_palette_entries(&commands, query)
                .iter()
                .any(|entry| entry.title() == "/mcp reconnect"),
            "nested metadata did not match {query}"
        );
    }
}

#[test]
fn command_palette_slash_entries_reuse_safe_completion_text_without_execution_data() {
    let commands = parse_slash_commands(Some(&serde_json::json!([{
        "name": "mcp",
        "aliases": ["servers"],
        "subcommands": [
            {"name": "reconnect", "description": "Reconnect", "usage": "<server>"},
            {"name": "list", "description": "List"}
        ]
    }])));

    let parent = filter_command_palette_entries(&commands, "servers")
        .into_iter()
        .find_map(|entry| match entry {
            CommandPaletteEntry::Slash { suggestion, .. } if suggestion.title == "/mcp" => {
                Some(suggestion)
            }
            _ => None,
        })
        .expect("parent slash palette entry");
    assert_eq!(parent.completion_text, slash_completion_text(&parent));
    assert_eq!(parent.completion_text, "/mcp ");
    assert!(parent.expects_input);

    let nested = filter_command_palette_entries(&commands, "reconnect")
        .into_iter()
        .find_map(|entry| match entry {
            CommandPaletteEntry::Slash { suggestion, .. }
                if suggestion.title == "/mcp reconnect" =>
            {
                Some(suggestion)
            }
            _ => None,
        })
        .expect("nested slash palette entry");
    assert_eq!(nested.completion_text, slash_completion_text(&nested));
    assert_eq!(nested.completion_text, "/mcp reconnect ");
    assert!(nested.is_subcommand);
}

#[test]
fn load_all_model_choices_reads_full_catalog() {
    let catalog = serde_json::json!({
        "models": [
            {"provider": "opencode-go", "id": "gpt-5.6-luna"},
            {
                "provider": "cursor",
                "id": "composer-2.5",
                "thinking": {"efforts": ["minimal", "low", "high"]}
            },
            {"provider": "other", "id": "m1"}
        ]
    });
    let models = load_all_model_choices(&catalog, None);
    assert_eq!(models.len(), 3);
    let composer = find_model_choice(&models, Some("cursor/composer-2.5")).expect("composer model");
    assert_eq!(
        composer.thinking_efforts.as_deref(),
        Some(["minimal".to_owned(), "low".to_owned(), "high".to_owned()].as_slice())
    );
}

#[test]
fn model_catalog_missing_or_empty_thinking_has_no_controls() {
    let models = load_all_model_choices(
        &serde_json::json!({
            "models": [
                {"provider": "x", "id": "missing"},
                {"provider": "x", "id": "null", "thinking": null},
                {"provider": "x", "id": "empty", "thinking": {"efforts": []}}
            ]
        }),
        None,
    );
    for choice in &models {
        assert_eq!(choice.thinking_efforts, None);
        assert!(thinking_options_for_model(Some(choice)).is_empty());
    }
}

#[test]
fn thinking_options_preserve_supported_order_and_deduplicate() {
    let choice = ModelChoice {
        provider: "anthropic".into(),
        id: "claude".into(),
        api: Some("anthropic-messages".into()),
        thinking_efforts: Some(vec![
            "minimal".into(),
            "low".into(),
            "low".into(),
            "high".into(),
        ]),
    };
    assert_eq!(
        thinking_options_for_model(Some(&choice)),
        ["off", "minimal", "low", "high", "auto"]
    );
    assert!(thinking_options_for_model(None).is_empty());
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
    assert_eq!(filtered[0].provider, "cursor");
    assert_eq!(filtered[0].id, "composer-2.5");
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
fn window_bounds_persistence_roundtrip_clamps_and_rejects_invalid_sizes() {
    let root = std::env::temp_dir().join(format!(
        "pimiento-window-bounds-{}-{}",
        std::process::id(),
        current_unix_seconds()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let persistence = SessionPersistence::from_root(root.clone());
    let bounds = Bounds {
        origin: point(px(100.0), px(50.0)),
        size: size(px(1200.0), px(800.0)),
    };

    persistence.save_window_bounds(bounds);
    assert_eq!(persistence.load_window_bounds(), Some(bounds));

    std::fs::write(
        persistence.window_bounds_path(),
        r#"{"x":10.0,"y":20.0,"width":200.0,"height":100.0}"#,
    )
    .expect("write small window bounds");
    assert_eq!(
        persistence.load_window_bounds(),
        Some(Bounds {
            origin: point(px(10.0), px(20.0)),
            size: size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT)),
        })
    );

    std::fs::write(
        persistence.window_bounds_path(),
        r#"{"x":10.0,"y":20.0,"width":0.0,"height":800.0}"#,
    )
    .expect("write invalid window bounds");
    assert_eq!(persistence.load_window_bounds(), None);
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
fn short_model_label_strips_only_cursor_provider() {
    assert_eq!(short_model_label("cursor/composer-2.5"), "composer-2.5");
    assert_eq!(
        short_model_label("anthropic/claude-opus"),
        "anthropic/claude-opus"
    );
    assert_eq!(short_model_label("unqualified"), "unqualified");
}

#[test]
fn ui_preferences_default_and_roundtrip_without_overwriting_each_other() {
    let root = std::env::temp_dir().join(format!(
        "pimiento-ui-persistence-{}-{}",
        std::process::id(),
        current_unix_seconds()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let persistence = SessionPersistence::from_root(root.clone());
    assert!(persistence.load_inspector_open());
    assert!(!persistence.load_rail_collapsed());
    assert_eq!(
        persistence.load_theme_selection().appearance,
        ThemePreference::System
    );

    persistence.save_rail_collapsed(true);
    assert!(persistence.load_rail_collapsed());
    assert!(persistence.load_inspector_open());
    assert_eq!(
        persistence.load_theme_selection().appearance,
        ThemePreference::System
    );

    let mut selection = persistence.load_theme_selection();
    selection.appearance = ThemePreference::Dark;
    persistence.save_theme_selection(&selection);
    assert_eq!(
        initial_theme_selection(None, &persistence).appearance,
        ThemePreference::Dark
    );
    assert_eq!(
        initial_theme_selection(Some(OsStr::new("light")), &persistence).appearance,
        ThemePreference::Light
    );
    assert_eq!(
        persistence.load_theme_selection().appearance,
        ThemePreference::Dark
    );

    persistence.save_inspector_open(false);
    assert!(!persistence.load_inspector_open());
    assert!(persistence.load_rail_collapsed());
    assert_eq!(
        persistence.load_theme_selection().appearance,
        ThemePreference::Dark
    );

    persistence.save_rail_collapsed(false);
    assert!(!persistence.load_inspector_open());
    assert!(!persistence.load_rail_collapsed());
    assert_eq!(
        persistence.load_theme_selection().appearance,
        ThemePreference::Dark
    );

    let mut selection = persistence.load_theme_selection();
    selection.appearance = ThemePreference::Light;
    persistence.save_theme_selection(&selection);
    assert!(!persistence.load_inspector_open());
    assert!(!persistence.load_rail_collapsed());
    assert_eq!(
        persistence.load_theme_selection().appearance,
        ThemePreference::Light
    );

    persistence.save_theme_selection(&ThemeSelection {
        appearance: ThemePreference::System,
        light: "Pepperwood Light".into(),
        dark: "Pepperwood Dark".into(),
    });
    assert_eq!(
        persistence.load_theme_selection(),
        ThemeSelection {
            appearance: ThemePreference::System,
            light: "Pepperwood Light".into(),
            dark: "Pepperwood Dark".into(),
        }
    );

    persistence.save_inspector_open(true);
    assert!(persistence.load_inspector_open());
    assert!(!persistence.load_rail_collapsed());
    assert_eq!(
        persistence.load_theme_selection().appearance,
        ThemePreference::System
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn running_tool_elapsed_uses_compact_seconds_and_minutes() {
    assert_eq!(format_running_elapsed(Duration::from_secs(9)), "9s");
    assert_eq!(format_running_elapsed(Duration::from_secs(125)), "2m 5s");
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
    let labels = sorted
        .iter()
        .map(|choice| format!("{}/{}", choice.provider, choice.id))
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        ["alpha/a", "cursor/composer-2.5", "cursor/other", "zeta/z"]
    );
}
#[test]
fn tool_group_positions_preserve_one_row_per_entry() {
    let transcript = vec![
        TranscriptEntry::Notice("before".into()),
        TranscriptEntry::ToolCall(pimiento_core::transcript::ToolCall::new_running(
            "tool-1",
            "read",
            serde_json::json!({"path": "a.rs"}),
        )),
        TranscriptEntry::ToolCall(pimiento_core::transcript::ToolCall::new_running(
            "tool-2",
            "bash",
            serde_json::json!({"command": "cargo check"}),
        )),
        TranscriptEntry::Notice("after".into()),
    ];

    assert_eq!(
        tool_group_position(&transcript, 1),
        ToolGroupPosition {
            grouped: true,
            first: true
        }
    );
    assert_eq!(
        tool_group_position(&transcript, 2),
        ToolGroupPosition {
            grouped: true,
            first: false
        }
    );
    assert_eq!(
        tool_group_position(&transcript, 0),
        ToolGroupPosition {
            grouped: false,
            first: false
        }
    );
}

#[test]
fn dialog_questions_parse_descriptions_and_recommendations() {
    let dialog = UiDialog {
        id: "ask-1".into(),
        method: "select".into(),
        payload: serde_json::json!({
            "questions": [
                {
                    "header": "Approach",
                    "question": "Which implementation?",
                    "description": "Choose one path.",
                    "recommended": 1,
                    "options": [
                        {"label": "Small patch", "description": "Minimal surface", "value": "small"},
                        {"label": "Full pass", "preview": "Includes tests", "value": "full"}
                    ]
                },
                {
                    "question": "Run checks?",
                    "options": [
                        {"label": "Yes", "recommended": true},
                        {"label": "No"}
                    ]
                }
            ]
        }),
        timeout_ms: None,
    };

    let questions = dialog_questions(&dialog);
    assert_eq!(questions.len(), 2);
    assert_eq!(questions[0].header.as_deref(), Some("Approach"));
    assert_eq!(
        questions[0].prompt.as_deref(),
        Some("Which implementation?")
    );
    assert_eq!(questions[0].recommended, Some(1));
    assert_eq!(
        questions[0].options[0].description.as_deref(),
        Some("Minimal surface")
    );
    assert_eq!(
        questions[0].options[1].description.as_deref(),
        Some("Includes tests")
    );
    assert_eq!(questions[1].recommended, Some(0));
}

#[test]
fn dialog_primary_options_keep_wire_values_for_keyboard_selection() {
    let dialog = UiDialog {
        id: "ask-2".into(),
        method: "select".into(),
        payload: serde_json::json!({
            "options": [
                {"label": "Readable label", "description": "More context", "value": "wire-value"},
                "plain"
            ],
            "recommended": "wire-value"
        }),
        timeout_ms: None,
    };

    assert_eq!(
        select_dialog_options(&dialog),
        vec!["wire-value".to_owned(), "plain".to_owned()]
    );
    let options = dialog_primary_options(&dialog);
    assert_eq!(dialog_recommended_index(&dialog.payload, &options), Some(0));
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

#[test]
fn dialog_cancel_fields_use_cancelled_and_timed_out() {
    let cancel = dialog_cancel_fields(false);
    assert_eq!(
        cancel.get("cancelled"),
        Some(&serde_json::Value::Bool(true))
    );
    assert!(cancel.get("timedOut").is_none());
    assert!(cancel.get("cancel").is_none());

    let timed = dialog_cancel_fields(true);
    assert_eq!(timed.get("cancelled"), Some(&serde_json::Value::Bool(true)));
    assert_eq!(timed.get("timedOut"), Some(&serde_json::Value::Bool(true)));
}

#[test]
fn display_status_lines_skip_empty() {
    let mut display = DisplayState::default();
    display.statuses.insert("k".into(), Some("hello".into()));
    display.statuses.insert("empty".into(), Some("  ".into()));
    display.statuses.insert("cleared".into(), None);
    let lines = display_status_lines(&display);
    assert_eq!(lines, vec![("k".into(), "hello".into())]);
}

#[test]
fn hub_job_summary_reads_wire_fields_only() {
    let args = serde_json::json!({"op": "jobs"});
    let result = serde_json::json!({
        "details": {
            "op": "jobs",
            "jobs": [
                {"id": "j1", "status": "running", "command": "cargo test"},
                {"jobId": "j2", "status": "done"}
            ]
        }
    });
    let summary = parse_hub_job_summary("hub", &args, &result).expect("hub summary");
    assert_eq!(summary.op.as_deref(), Some("jobs"));
    assert_eq!(summary.jobs[0].id.as_deref(), Some("j1"));
    let lines = hub_job_summary_display_lines(&summary);
    assert!(lines.iter().any(|line| line.contains("j1")));
    assert!(parse_hub_job_summary("bash", &args, &result).is_none());
}

#[test]
fn abort_bash_has_no_correlatable_target_field() {
    assert_eq!(
        serde_json::to_value(RpcCommandBody::AbortBash).expect("serialize abort_bash"),
        serde_json::json!({"type": "abort_bash"})
    );
}

#[test]
fn task_and_eval_digests_require_wire_fields() {
    assert_eq!(
        task_linkage_id(
            "task",
            &serde_json::json!({"subagentId": "sa-1"}),
            &serde_json::json!({})
        )
        .as_deref(),
        Some("sa-1")
    );
    assert_eq!(
        task_linkage_id(
            "TASK",
            &serde_json::json!({}),
            &serde_json::json!({"details": {"toolCallId": "call-2"}})
        )
        .as_deref(),
        Some("call-2")
    );
    assert!(task_linkage_id("task", &serde_json::json!({}), &serde_json::json!({})).is_none());
    let eval = parse_eval_card_summary(
        "eval",
        &serde_json::json!({"language": "py", "title": "imports", "code": "import json"}),
    )
    .expect("eval summary");
    assert_eq!(eval.title, "Eval · imports");
    assert_eq!(eval.digest, "Python · import json");
    assert!(parse_eval_card_summary("bash", &serde_json::json!({"code": "1 + 1"})).is_none());
}
