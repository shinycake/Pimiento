use crate::*;
use gpui::{KeyBinding, Menu, MenuItem, SystemMenuType, actions};
use gpui_component::input;

actions!(
    pimiento_menu,
    [
        AboutPimiento,
        QuitPimiento,
        HidePimiento,
        HideOtherApplications,
        ShowAllApplications,
        OpenWorkspace,
        NewSessionTab,
        CloseSessionTab,
        OpenCommandPalette,
        ChooseTheme,
        ToggleSessionRail,
        ToggleContextInspector,
        RenameSession,
        BranchFromTurn,
        ExportSessionHtml,
        ShareSession,
        AbortRun,
        MinimizeWindow,
        ZoomWindow,
        EnterFullScreen,
        BringAllToFront,
    ]
);

pub(crate) const MENU_TITLES: [&str; 6] = ["Pimiento", "File", "Edit", "View", "Session", "Window"];

pub(crate) fn app_menus() -> Vec<Menu> {
    vec![
        Menu::new(MENU_TITLES[0]).items([
            MenuItem::action("About Pimiento", AboutPimiento),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Pimiento", HidePimiento),
            MenuItem::action("Hide Others", HideOtherApplications),
            MenuItem::action("Show All", ShowAllApplications),
            MenuItem::separator(),
            MenuItem::action("Quit Pimiento", QuitPimiento),
        ]),
        Menu::new(MENU_TITLES[1]).items([
            MenuItem::action("Open Workspace…", OpenWorkspace),
            MenuItem::separator(),
            MenuItem::action("New Session Tab", NewSessionTab),
            MenuItem::action("Close Session Tab", CloseSessionTab),
        ]),
        Menu::new(MENU_TITLES[2]).items([
            MenuItem::action("Undo", input::Undo),
            MenuItem::action("Redo", input::Redo),
            MenuItem::separator(),
            MenuItem::action("Cut", input::Cut),
            MenuItem::action("Copy", input::Copy),
            MenuItem::action("Paste", input::Paste),
            MenuItem::action("Select All", input::SelectAll),
        ]),
        Menu::new(MENU_TITLES[3]).items([
            MenuItem::action("Command Palette…", OpenCommandPalette),
            MenuItem::action("Theme…", ChooseTheme),
            MenuItem::separator(),
            MenuItem::action("Toggle Session Rail", ToggleSessionRail),
            MenuItem::action("Toggle Context Inspector", ToggleContextInspector),
        ]),
        Menu::new(MENU_TITLES[4]).items([
            MenuItem::action("Rename Session…", RenameSession),
            MenuItem::action("Branch/Fork from Turn…", BranchFromTurn),
            MenuItem::separator(),
            MenuItem::action("Export HTML", ExportSessionHtml),
            MenuItem::action("Share Session", ShareSession),
            MenuItem::separator(),
            MenuItem::action("Abort Run", AbortRun),
        ]),
        Menu::new(MENU_TITLES[5]).items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::action("Zoom", ZoomWindow),
            MenuItem::action("Enter Full Screen", EnterFullScreen),
            MenuItem::separator(),
            MenuItem::action("Bring All to Front", BringAllToFront),
        ]),
    ]
}

pub(crate) fn install_app_menu(cx: &mut App) {
    cx.on_action(|_: &QuitPimiento, cx| cx.quit());
    cx.on_action(|_: &HidePimiento, cx| cx.hide());
    cx.on_action(|_: &HideOtherApplications, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAllApplications, cx| cx.unhide_other_apps());
    cx.on_action(|_: &BringAllToFront, cx| {
        cx.activate(true);
        for handle in cx.windows() {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
    });

    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-q", QuitPimiento, None),
        KeyBinding::new("cmd-h", HidePimiento, None),
        KeyBinding::new("cmd-alt-h", HideOtherApplications, None),
        KeyBinding::new("cmd-o", OpenWorkspace, None),
        KeyBinding::new("cmd-t", NewSessionTab, None),
        KeyBinding::new("cmd-w", CloseSessionTab, None),
        KeyBinding::new("cmd-k", OpenCommandPalette, None),
        KeyBinding::new("cmd-b", ToggleSessionRail, None),
        KeyBinding::new("cmd-j", ToggleContextInspector, None),
        KeyBinding::new("ctrl-cmd-f", EnterFullScreen, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("cmd-z", input::Undo, None),
        KeyBinding::new("cmd-shift-z", input::Redo, None),
        KeyBinding::new("cmd-x", input::Cut, None),
        KeyBinding::new("cmd-c", input::Copy, None),
        KeyBinding::new("cmd-v", input::Paste, None),
        KeyBinding::new("cmd-a", input::SelectAll, None),
    ]);

    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-q", QuitPimiento, None),
        KeyBinding::new("ctrl-o", OpenWorkspace, None),
        KeyBinding::new("ctrl-t", NewSessionTab, None),
        KeyBinding::new("ctrl-w", CloseSessionTab, None),
        KeyBinding::new("ctrl-k", OpenCommandPalette, None),
        KeyBinding::new("ctrl-b", ToggleSessionRail, None),
        KeyBinding::new("ctrl-j", ToggleContextInspector, None),
        KeyBinding::new("ctrl-z", input::Undo, None),
        KeyBinding::new("ctrl-y", input::Redo, None),
        KeyBinding::new("ctrl-x", input::Cut, None),
        KeyBinding::new("ctrl-c", input::Copy, None),
        KeyBinding::new("ctrl-v", input::Paste, None),
        KeyBinding::new("ctrl-a", input::SelectAll, None),
    ]);

    cx.set_menus(app_menus());
}

impl WorkspaceView {
    fn active_session_action(
        &mut self,
        action: PaletteActionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_workspace_palette_action(action, window, cx);
    }

    pub(crate) fn handle_about_menu(
        &mut self,
        _: &AboutPimiento,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_session_action(PaletteActionId::About, window, cx);
    }

    pub(crate) fn handle_open_workspace_menu(
        &mut self,
        _: &OpenWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_new_workspace(window, cx);
    }

    pub(crate) fn handle_new_session_menu(
        &mut self,
        _: &NewSessionTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_workspace_palette_action(PaletteActionId::NewSession, window, cx);
    }

    pub(crate) fn handle_close_session_menu(
        &mut self,
        _: &CloseSessionTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_workspace_palette_action(PaletteActionId::CloseSession, window, cx);
    }

    pub(crate) fn handle_palette_menu(
        &mut self,
        _: &OpenCommandPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.sessions.get(self.active).cloned() {
            session.update(cx, SessionView::toggle_palette);
        }
    }

    pub(crate) fn handle_theme_menu(
        &mut self,
        _: &ChooseTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_session_action(PaletteActionId::ToggleTheme, window, cx);
    }

    pub(crate) fn handle_toggle_rail_menu(
        &mut self,
        _: &ToggleSessionRail,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_rail(cx);
    }

    pub(crate) fn handle_toggle_inspector_menu(
        &mut self,
        _: &ToggleContextInspector,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_inspector(cx);
    }

    pub(crate) fn handle_rename_menu(
        &mut self,
        _: &RenameSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_session_action(PaletteActionId::RenameSession, window, cx);
    }

    pub(crate) fn handle_branch_menu(
        &mut self,
        _: &BranchFromTurn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_session_action(PaletteActionId::BranchSession, window, cx);
    }

    pub(crate) fn handle_export_menu(
        &mut self,
        _: &ExportSessionHtml,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_session_action(PaletteActionId::ExportHtml, window, cx);
    }

    pub(crate) fn handle_share_menu(
        &mut self,
        _: &ShareSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_session_action(PaletteActionId::ShareSession, window, cx);
    }

    pub(crate) fn handle_abort_menu(
        &mut self,
        _: &AbortRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_session_action(PaletteActionId::AbortRun, window, cx);
    }

    #[allow(clippy::unused_self)] // Context::listener requires a WorkspaceView receiver.
    pub(crate) fn handle_minimize_menu(
        &mut self,
        _: &MinimizeWindow,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.minimize_window();
    }

    #[allow(clippy::unused_self)] // Context::listener requires a WorkspaceView receiver.
    pub(crate) fn handle_zoom_menu(
        &mut self,
        _: &ZoomWindow,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.zoom_window();
    }

    #[allow(clippy::unused_self)] // Context::listener requires a WorkspaceView receiver.
    pub(crate) fn handle_fullscreen_menu(
        &mut self,
        _: &EnterFullScreen,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
    }
}
