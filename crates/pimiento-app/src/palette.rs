#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteActionId {
    About,
    ToggleTheme,
    ToggleTodos,
    ToggleAgents,
    ToggleModels,
    ToggleThinking,
    ToggleFast,
    ExportHtml,
    RenameSession,
    AbortRun,
    SessionsLauncher,
    RevealLogs,
    NewSession,
    CloseSession,
    ToggleRail,
    ToggleInspector,
}

#[derive(Debug, Clone)]
pub(crate) struct PaletteEntry {
    pub(crate) id: PaletteActionId,
    pub(crate) label: &'static str,
    pub(crate) hint: &'static str,
}

pub(crate) fn palette_catalog() -> &'static [PaletteEntry] {
    &[
        PaletteEntry {
            id: PaletteActionId::About,
            label: "About Pimiento",
            hint: "version app information",
        },
        PaletteEntry {
            id: PaletteActionId::ToggleTheme,
            label: "Toggle theme",
            hint: "light/dark",
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
            id: PaletteActionId::ExportHtml,
            label: "Export HTML",
            hint: "export",
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

pub(crate) fn shell_single_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

pub(crate) fn revert_command_for_path(path: &str) -> String {
    format!("git restore --worktree -- {}", shell_single_quote(path))
}
