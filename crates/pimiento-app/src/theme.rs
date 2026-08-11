use crate::*;
use gpui::Hsla;
use gpui_component::{ThemeConfig, ThemeRegistry, try_parse_color};
use std::rc::Rc;

pub(crate) const DEFAULT_LIGHT_THEME: &str = "Quiet Pepper Light";
pub(crate) const DEFAULT_DARK_THEME: &str = "Quiet Pepper Dark";
pub(crate) const BUNDLED_THEMES: &str = include_str!("pimiento-themes.json");
pub(crate) const ZED_DEFAULT_THEMES: &str = include_str!("zed-default-themes.json");
pub(crate) const COMMUNITY_THEMES: &str = include_str!("community-themes.json");

const BUNDLED_THEME_SETS: [(&str, &str); 3] = [
    ("Pimiento", BUNDLED_THEMES),
    ("Zed defaults", ZED_DEFAULT_THEMES),
    ("community", COMMUNITY_THEMES),
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::System => "Follow system",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThemeSelection {
    pub(crate) appearance: ThemePreference,
    pub(crate) light: String,
    pub(crate) dark: String,
}

impl Default for ThemeSelection {
    fn default() -> Self {
        Self {
            appearance: ThemePreference::System,
            light: DEFAULT_LIGHT_THEME.to_owned(),
            dark: DEFAULT_DARK_THEME.to_owned(),
        }
    }
}

pub(crate) struct ThemeSelectionState(pub(crate) ThemeSelection);

impl Global for ThemeSelectionState {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ThemePickerItem {
    Appearance(ThemePreference),
    Theme { name: String, mode: ThemeMode },
}

impl ThemePickerItem {
    pub(crate) fn search_text(&self) -> String {
        match self {
            Self::Appearance(preference) => {
                format!("appearance {}", preference.label()).to_ascii_lowercase()
            }
            Self::Theme { name, mode } => format!(
                "{} theme {}",
                if mode.is_dark() { "dark" } else { "light" },
                name
            )
            .to_ascii_lowercase(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RegisteredThemeChoice {
    pub(crate) name: String,
    pub(crate) mode: ThemeMode,
    pub(crate) swatches: [Option<Hsla>; 3],
}

pub(crate) fn theme_preference_from_env(raw: Option<&str>) -> ThemePreference {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("light") => ThemePreference::Light,
        Some("dark") => ThemePreference::Dark,
        _ => ThemePreference::System,
    }
}

pub(crate) fn initial_theme_selection(
    env: Option<&OsStr>,
    persistence: &SessionPersistence,
) -> ThemeSelection {
    let mut selection = persistence.load_theme_selection();
    if let Some(raw) = env {
        selection.appearance = theme_preference_from_env(raw.to_str());
    }
    selection
}

pub(crate) fn themes_directory(root: &Path) -> PathBuf {
    root.join("themes")
}

pub(crate) fn register_bundled_themes(cx: &mut App) {
    for (catalog, themes) in BUNDLED_THEME_SETS {
        if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(themes) {
            eprintln!("Pimiento {catalog} themes could not be loaded: {error}");
        }
    }
}

pub(crate) fn initialize_theme_registry(
    persistence: &SessionPersistence,
    selection: ThemeSelection,
    cx: &mut App,
) {
    register_bundled_themes(cx);
    cx.set_global(ThemeSelectionState(selection));
    let themes_dir = themes_directory(&persistence.root);
    if let Err(error) = ThemeRegistry::watch_dir(themes_dir, cx, move |cx| {
        // ThemeRegistry reloads from disk by replacing its custom map.
        register_bundled_themes(cx);
        reapply_theme_without_window(cx);
    }) {
        eprintln!("Pimiento user theme directory could not be watched: {error}");
    }
}

fn configured_theme(name: &str, mode: ThemeMode, cx: &App) -> Option<Rc<ThemeConfig>> {
    ThemeRegistry::global(cx)
        .themes()
        .get(name)
        .filter(|theme| theme.mode == mode)
        .cloned()
}

fn install_selected_pair(selection: &ThemeSelection, cx: &mut App) {
    let light = configured_theme(&selection.light, ThemeMode::Light, cx);
    let dark = configured_theme(&selection.dark, ThemeMode::Dark, cx);
    let theme = Theme::global_mut(cx);
    if let Some(light) = light {
        theme.light_theme = light;
    }
    if let Some(dark) = dark {
        theme.dark_theme = dark;
    }
}

pub(crate) fn apply_theme_selection(selection: &ThemeSelection, window: &mut Window, cx: &mut App) {
    cx.set_global(ThemeSelectionState(selection.clone()));
    install_selected_pair(selection, cx);
    match selection.appearance {
        ThemePreference::System => {
            cx.set_window_appearance(None);
            Theme::sync_system_appearance(Some(window), cx);
        }
        ThemePreference::Light => {
            cx.set_window_appearance(Some(WindowAppearance::Light));
            Theme::change(ThemeMode::Light, Some(window), cx);
        }
        ThemePreference::Dark => {
            cx.set_window_appearance(Some(WindowAppearance::Dark));
            Theme::change(ThemeMode::Dark, Some(window), cx);
        }
    }
    apply_pimiento_brand(cx);
    window.refresh();
}

pub(crate) fn apply_theme_selection_without_window(selection: &ThemeSelection, cx: &mut App) {
    cx.set_global(ThemeSelectionState(selection.clone()));
    install_selected_pair(selection, cx);
    match selection.appearance {
        ThemePreference::System => {
            cx.set_window_appearance(None);
            Theme::sync_system_appearance(None, cx);
        }
        ThemePreference::Light => {
            cx.set_window_appearance(Some(WindowAppearance::Light));
            Theme::change(ThemeMode::Light, None, cx);
        }
        ThemePreference::Dark => {
            cx.set_window_appearance(Some(WindowAppearance::Dark));
            Theme::change(ThemeMode::Dark, None, cx);
        }
    }
    apply_pimiento_brand(cx);
    cx.refresh_windows();
}

pub(crate) fn reapply_theme_without_window(cx: &mut App) {
    let selection = cx.global::<ThemeSelectionState>().0.clone();
    install_selected_pair(&selection, cx);
    let mode = match selection.appearance {
        ThemePreference::System => cx.window_appearance().into(),
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
    };
    Theme::change(mode, None, cx);
    apply_pimiento_brand(cx);
    cx.refresh_windows();
}

pub(crate) fn theme_family_name(name: &str) -> String {
    let trimmed = name.trim();
    for suffix in [" light", " dark", "-light", "-dark", "_light", "_dark"] {
        if trimmed.to_ascii_lowercase().ends_with(suffix) {
            return trimmed[..trimmed.len() - suffix.len()].trim().to_owned();
        }
    }
    trimmed.to_owned()
}

pub(crate) fn paired_theme_name(
    selected_name: &str,
    counterpart_mode: ThemeMode,
    themes: &[(String, ThemeMode)],
) -> Option<String> {
    let family = theme_family_name(selected_name);
    themes
        .iter()
        .find(|(name, mode)| {
            *mode == counterpart_mode && theme_family_name(name).eq_ignore_ascii_case(&family)
        })
        .map(|(name, _)| name.clone())
}

pub(crate) fn registered_theme_choices(cx: &App) -> Vec<RegisteredThemeChoice> {
    ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .map(|theme| RegisteredThemeChoice {
            name: theme.name.to_string(),
            mode: theme.mode,
            swatches: [
                theme
                    .colors
                    .background
                    .as_deref()
                    .and_then(|value| try_parse_color(value).ok()),
                theme
                    .colors
                    .secondary
                    .as_deref()
                    .and_then(|value| try_parse_color(value).ok()),
                theme
                    .colors
                    .accent
                    .as_deref()
                    .and_then(|value| try_parse_color(value).ok()),
            ],
        })
        .collect()
}

pub(crate) fn filter_theme_picker_items(
    themes: &[RegisteredThemeChoice],
    query: &str,
) -> Vec<ThemePickerItem> {
    let query = query.trim().to_ascii_lowercase();
    let mut items = [
        ThemePreference::System,
        ThemePreference::Light,
        ThemePreference::Dark,
    ]
    .into_iter()
    .map(ThemePickerItem::Appearance)
    .chain(themes.iter().map(|theme| ThemePickerItem::Theme {
        name: theme.name.clone(),
        mode: theme.mode,
    }))
    .filter(|item| query.is_empty() || item.search_text().contains(&query))
    .collect::<Vec<_>>();
    items.sort_by_key(|item| match item {
        ThemePickerItem::Appearance(ThemePreference::System) => (0, String::new()),
        ThemePickerItem::Appearance(ThemePreference::Light) => (1, String::new()),
        ThemePickerItem::Appearance(ThemePreference::Dark) => (2, String::new()),
        ThemePickerItem::Theme { name, mode } if !mode.is_dark() => (3, name.to_ascii_lowercase()),
        ThemePickerItem::Theme { name, .. } => (4, name.to_ascii_lowercase()),
    });
    items
}

pub(crate) fn theme_picker_item_is_active(
    item: &ThemePickerItem,
    selection: &ThemeSelection,
) -> bool {
    match item {
        ThemePickerItem::Appearance(preference) => selection.appearance == *preference,
        ThemePickerItem::Theme { name, mode } if mode.is_dark() => selection.dark == *name,
        ThemePickerItem::Theme { name, .. } => selection.light == *name,
    }
}

pub(crate) fn theme_picker_selected_index(
    items: &[ThemePickerItem],
    selection: &ThemeSelection,
) -> usize {
    items
        .iter()
        .position(|item| theme_picker_item_is_active(item, selection))
        .unwrap_or(0)
}

pub(crate) fn theme_selection_for_picker_item(
    opening_selection: &ThemeSelection,
    item: &ThemePickerItem,
    themes: &[(String, ThemeMode)],
) -> ThemeSelection {
    let mut selection = opening_selection.clone();
    match item {
        ThemePickerItem::Appearance(preference) => selection.appearance = *preference,
        ThemePickerItem::Theme { name, mode } if mode.is_dark() => {
            selection.dark.clone_from(name);
            selection.appearance = ThemePreference::Dark;
            if let Some(pair) = paired_theme_name(name, ThemeMode::Light, themes) {
                selection.light = pair;
            }
        }
        ThemePickerItem::Theme { name, .. } => {
            selection.light.clone_from(name);
            selection.appearance = ThemePreference::Light;
            if let Some(pair) = paired_theme_name(name, ThemeMode::Dark, themes) {
                selection.dark = pair;
            }
        }
    }
    selection
}
