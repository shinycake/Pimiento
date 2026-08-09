#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Pimiento — first live OMP session workspace.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::{
    App, Bounds, ClickEvent, ClipboardItem, Context, ElementId, ExternalPaths, Focusable,
    FollowMode, Global, KeyDownEvent, ListAlignment, ListOffset, ListState, PathPromptOptions,
    Pixels, Render, Task, Window, WindowAppearance, WindowBounds, WindowOptions, div, list, point,
    prelude::*, px, size,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Root, Sizable as _, Theme, ThemeMode,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    label::Label,
    progress::Progress,
    scroll::ScrollableElement as _,
    separator::Separator,
    switch::Switch,
    tag::Tag,
    text::TextView,
    v_flex,
};
use omp_rpc_client::{
    client::{ClientConfig, ClientEvent, RpcClient},
    discovery::{
        DiscoveryInputs, MIN_SUPPORTED, OmpVersion, SystemRunner, VersionSupport, discover,
    },
    frames::{
        InterruptMode, QueueMode, RpcCommandBody, StreamingBehavior, SubagentSubscriptionLevel,
    },
};
use pimiento_core::{
    diff::{DiffLineKind, parse_edit_diff, parse_unified_diff_lines},
    projection::{
        DisplayState, RunPhase, SessionProjection, UiDialog, format_model_label, split_model_label,
    },
    todos::{TodoPhaseView, TodoTaskView, parse_todo_phases, todo_status_glyph},
    transcript::{CompactionPhase, ToolStatus, TranscriptEntry},
};
use serde::{Deserialize, Serialize};

mod app_state;
mod git_status;
mod host_bridge;
mod models;
mod palette;
mod session;
mod tokens;
mod transcript_ui;
mod workspace;

// These glob imports form the crate-local module facade used by sibling modules
// during this behavior-preserving split.
#[allow(clippy::wildcard_imports)]
use app_state::*;
#[allow(clippy::wildcard_imports)]
use git_status::*;
#[allow(clippy::wildcard_imports)]
use host_bridge::*;
#[allow(clippy::wildcard_imports)]
use models::*;
#[allow(clippy::wildcard_imports)]
use palette::*;
#[allow(clippy::wildcard_imports)]
use session::*;
#[allow(clippy::wildcard_imports)]
use tokens::*;
#[allow(clippy::wildcard_imports)]
use transcript_ui::*;
#[allow(clippy::wildcard_imports)]
use workspace::*;
// ── entry ─────────────────────────────────────────────────────────────────

fn main() {
    let persistence = SessionPersistence::from_environment();
    let saved_window_bounds = persistence.load_window_bounds();
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

    let theme_override = std::env::var_os("PIMIENTO_THEME");
    let initial_theme = initial_theme_preference(theme_override.as_deref(), &persistence);

    gpui_platform::application().run(move |cx| {
        gpui_component::init(cx);
        cx.set_global(ThemePreferenceState(initial_theme));
        cx.spawn(async move |cx| {
            let window_options = WindowOptions {
                window_bounds: saved_window_bounds.map(WindowBounds::Windowed),
                ..Default::default()
            };
            cx.open_window(window_options, |window, cx| {
                apply_theme_preference(initial_theme, window, cx);
                window
                    .observe_window_appearance(|window, cx| {
                        let follows_system =
                            cx.global::<ThemePreferenceState>().0 == ThemePreference::System;
                        if follows_system {
                            Theme::sync_system_appearance(Some(window), cx);
                            apply_pimiento_brand(cx);
                        }
                    })
                    .detach();
                let session = cx.new(|cx| {
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
                    session.update(cx, |this, cx| {
                        this.begin_connection(window, cwd, resume, true, cx);
                    });
                }
                let workspace = cx.new(|cx| {
                    WorkspaceView::new(session, persistence.clone(), initial_cwd.clone(), cx)
                });
                let weak_workspace = workspace.downgrade();
                window.on_window_should_close(cx, move |_window, cx| {
                    weak_workspace
                        .update(cx, WorkspaceView::should_close_window)
                        .unwrap_or(true)
                });
                let bounds_persistence = persistence.clone();
                let mut last_saved_bounds = saved_window_bounds;
                cx.new(|cx| {
                    cx.observe_window_bounds(window, move |_, window, _| {
                        let WindowBounds::Windowed(bounds) = window.window_bounds() else {
                            return;
                        };
                        let bounds = normalize_window_bounds(bounds);
                        if last_saved_bounds == Some(bounds) {
                            return;
                        }
                        bounds_persistence.save_window_bounds(bounds);
                        last_saved_bounds = Some(bounds);
                    })
                    .detach();
                    Root::new(workspace, window, cx)
                })
            })
            .expect("open primary window");
        })
        .detach();
    });
}

#[cfg(test)]
mod tests;
