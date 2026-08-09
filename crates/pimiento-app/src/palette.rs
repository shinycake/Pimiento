#[allow(clippy::wildcard_imports)]
// Matches the crate-local module facade used by sibling modules.
use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteActionId {
    About,
    ToggleTheme,
    ToggleTodos,
    ToggleAgents,
    ToggleModels,
    ToggleThinking,
    ToggleFast,
    CycleModel,
    CycleThinking,
    Compact,
    SessionStats,
    FreshSession,
    Handoff,
    AbortRetry,
    AbortAndPrompt,
    ExportHtml,
    ShareSession,
    RenameSession,
    AbortRun,
    SessionsLauncher,
    RevealLogs,
    NewSession,
    CloseSession,
    ToggleRail,
    ToggleInspector,
    BranchSession,
    LoginProviders,
}

#[derive(Debug, Clone)]
pub(crate) struct PaletteEntry {
    pub(crate) id: PaletteActionId,
    pub(crate) label: &'static str,
    pub(crate) hint: &'static str,
}

#[allow(
    clippy::too_many_lines,
    reason = "static command catalog; splitting would obscure the action list"
)]
pub(crate) fn palette_catalog() -> &'static [PaletteEntry] {
    &[
        PaletteEntry {
            id: PaletteActionId::About,
            label: "About Pimiento",
            hint: "version app information",
        },
        PaletteEntry {
            id: PaletteActionId::ToggleTheme,
            label: "Theme",
            hint: "cycle system light dark appearance",
        },
        PaletteEntry {
            id: PaletteActionId::ToggleTodos,
            label: "Show checklist in inspector",
            hint: "checklist",
        },
        PaletteEntry {
            id: PaletteActionId::ToggleAgents,
            label: "Show agents in inspector",
            hint: "subagents inspector",
        },
        PaletteEntry {
            id: PaletteActionId::ToggleModels,
            label: "Model picker",
            hint: "models",
        },
        PaletteEntry {
            id: PaletteActionId::ToggleThinking,
            label: "Thinking level",
            hint: "thinking",
        },
        PaletteEntry {
            id: PaletteActionId::ToggleFast,
            label: "Toggle fast mode",
            hint: "fast",
        },
        PaletteEntry {
            id: PaletteActionId::CycleModel,
            label: "Cycle model",
            hint: "cycle_model next",
        },
        PaletteEntry {
            id: PaletteActionId::CycleThinking,
            label: "Cycle thinking level",
            hint: "cycle_thinking_level",
        },
        PaletteEntry {
            id: PaletteActionId::Compact,
            label: "Compact…",
            hint: "compact context with optional instructions",
        },
        PaletteEntry {
            id: PaletteActionId::SessionStats,
            label: "Session stats",
            hint: "messages tokens cost context usage",
        },
        PaletteEntry {
            id: PaletteActionId::FreshSession,
            label: "Fresh session",
            hint: "start fresh via /fresh",
        },
        PaletteEntry {
            id: PaletteActionId::Handoff,
            label: "Handoff to TUI…",
            hint: "continue this session in the terminal",
        },
        PaletteEntry {
            id: PaletteActionId::AbortRetry,
            label: "Abort retry",
            hint: "abort_retry",
        },
        PaletteEntry {
            id: PaletteActionId::AbortAndPrompt,
            label: "Abort and prompt",
            hint: "abort_and_prompt composer",
        },
        PaletteEntry {
            id: PaletteActionId::ExportHtml,
            label: "Export HTML",
            hint: "export",
        },
        PaletteEntry {
            id: PaletteActionId::ShareSession,
            label: "Share session",
            hint: "run /share with OMP",
        },
        PaletteEntry {
            id: PaletteActionId::RenameSession,
            label: "Rename session",
            hint: "rename",
        },
        PaletteEntry {
            id: PaletteActionId::AbortRun,
            label: "Abort run",
            hint: "stop",
        },
        PaletteEntry {
            id: PaletteActionId::BranchSession,
            label: "Branch from turn…",
            hint: "choose a prior turn and open the branch in a new tab",
        },
        PaletteEntry {
            id: PaletteActionId::LoginProviders,
            label: "Login providers",
            hint: "get_login_providers oauth",
        },
        PaletteEntry {
            id: PaletteActionId::SessionsLauncher,
            label: "Sessions launcher",
            hint: "back",
        },
        PaletteEntry {
            id: PaletteActionId::RevealLogs,
            label: "Reveal logs",
            hint: "pimiento home folder",
        },
        PaletteEntry {
            id: PaletteActionId::NewSession,
            label: "New session tab",
            hint: "workspace",
        },
        PaletteEntry {
            id: PaletteActionId::CloseSession,
            label: "Close session tab",
            hint: "workspace",
        },
        PaletteEntry {
            id: PaletteActionId::ToggleRail,
            label: "Toggle session rail",
            hint: "rail",
        },
        PaletteEntry {
            id: PaletteActionId::ToggleInspector,
            label: "Toggle context inspector",
            hint: "inspector sidebar context",
        },
    ]
}

pub(crate) fn filter_palette_entries(query: &str) -> Vec<&'static PaletteEntry> {
    let q = query.trim().to_ascii_lowercase();
    palette_catalog()
        .iter()
        .filter(|entry| {
            if q.is_empty() {
                return true;
            }
            entry.label.to_ascii_lowercase().contains(&q)
                || entry.hint.to_ascii_lowercase().contains(&q)
        })
        .collect()
}

pub(crate) fn palette_entry_display_label(entry: &PaletteEntry, theme: ThemePreference) -> String {
    if entry.id == PaletteActionId::ToggleTheme {
        let current = match theme {
            ThemePreference::System => "System",
            ThemePreference::Light => "Light",
            ThemePreference::Dark => "Dark",
        };
        format!("Theme: {current} · cycle system → light → dark")
    } else {
        format!("{} · {}", entry.label, entry.hint)
    }
}

pub(crate) fn shell_single_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

pub(crate) fn revert_command_for_path(path: &str) -> String {
    format!("git restore --worktree -- {}", shell_single_quote(path))
}
