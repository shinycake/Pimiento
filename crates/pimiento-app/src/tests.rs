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
fn version_gate_notice_only_formats_outside_tested_baseline() {
    assert_eq!(format_version_gate_notice(MIN_SUPPORTED), None);

    let below = OmpVersion {
        major: 17,
        minor: 2,
        patch: 9,
    };
    assert_eq!(
        format_version_gate_notice(below).as_deref(),
        Some(
            "Pimiento was tested with omp 17.2.10+; you have 17.2.9 — unknown events will still render"
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
            "Pimiento was tested with omp 17.2.10+; you have 17.3.0 — unknown events will still render"
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
    assert!(filter_palette_entries("zzzz-nope").is_empty());
}

#[test]
fn palette_theme_entry_shows_current_preference() {
    assert_eq!(
        palette_entry_display_label(
            &PaletteEntry {
                id: PaletteActionId::ToggleTheme,
                label: "Theme",
                hint: "cycle",
            },
            ThemePreference::Light
        ),
        "Theme: Light · cycle system → light → dark"
    );
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
fn groups_sessions_by_workspace_name_and_preserves_session_order() {
    let entries = vec![
        RailEntry {
            ix: 2,
            label: "later".to_owned(),
            phase: "idle".to_owned(),
            cwd: PathBuf::from("/tmp/zulu"),
            attention: RailAttention::Quiet,
        },
        RailEntry {
            ix: 1,
            label: "second".to_owned(),
            phase: "stream".to_owned(),
            cwd: PathBuf::from("/tmp/alpha"),
            attention: RailAttention::Active,
        },
        RailEntry {
            ix: 0,
            label: "first".to_owned(),
            phase: "idle".to_owned(),
            cwd: PathBuf::from("/tmp/alpha"),
            attention: RailAttention::Unread,
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
fn theme_preference_cycles_system_light_dark() {
    assert_eq!(
        next_theme_preference(ThemePreference::System),
        ThemePreference::Light
    );
    assert_eq!(
        next_theme_preference(ThemePreference::Light),
        ThemePreference::Dark
    );
    assert_eq!(
        next_theme_preference(ThemePreference::Dark),
        ThemePreference::System
    );
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
    assert!(!parsed.rail_collapsed);

    let legacy: PersistedUi =
        serde_json::from_str(r#"{"inspector_open":true}"#).expect("parse legacy ui");
    assert_eq!(legacy.theme, ThemePreference::System);
    assert!(!legacy.rail_collapsed);

    let value = serde_json::to_value(PersistedUi {
        inspector_open: true,
        rail_collapsed: true,
        theme: ThemePreference::Light,
    })
    .expect("serialize ui");
    assert_eq!(value["theme"], "light");
    assert_eq!(value["rail_collapsed"], true);
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
    assert_eq!(persistence.load_theme_preference(), ThemePreference::System);

    persistence.save_rail_collapsed(true);
    assert!(persistence.load_rail_collapsed());
    assert!(persistence.load_inspector_open());
    assert_eq!(persistence.load_theme_preference(), ThemePreference::System);

    persistence.save_theme_preference(ThemePreference::Dark);
    assert_eq!(
        initial_theme_preference(None, &persistence),
        ThemePreference::Dark
    );
    assert_eq!(
        initial_theme_preference(Some(OsStr::new("light")), &persistence),
        ThemePreference::Light
    );
    assert_eq!(persistence.load_theme_preference(), ThemePreference::Dark);

    persistence.save_inspector_open(false);
    assert!(!persistence.load_inspector_open());
    assert!(persistence.load_rail_collapsed());
    assert_eq!(persistence.load_theme_preference(), ThemePreference::Dark);

    persistence.save_rail_collapsed(false);
    assert!(!persistence.load_inspector_open());
    assert!(!persistence.load_rail_collapsed());
    assert_eq!(persistence.load_theme_preference(), ThemePreference::Dark);

    persistence.save_theme_preference(ThemePreference::Light);
    assert!(!persistence.load_inspector_open());
    assert!(!persistence.load_rail_collapsed());
    assert_eq!(persistence.load_theme_preference(), ThemePreference::Light);

    persistence.save_inspector_open(true);
    assert!(persistence.load_inspector_open());
    assert!(!persistence.load_rail_collapsed());
    assert_eq!(persistence.load_theme_preference(), ThemePreference::Light);

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
