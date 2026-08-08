use crate::*;

// ── theme preference ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemePreference {
    System,
    Light,
    Dark,
}

pub(crate) struct ThemePreferenceState(pub(crate) ThemePreference);

impl Global for ThemePreferenceState {}

pub(crate) fn next_theme_preference(current: ThemePreference) -> ThemePreference {
    match current {
        ThemePreference::System => ThemePreference::Light,
        ThemePreference::Light => ThemePreference::Dark,
        ThemePreference::Dark => ThemePreference::System,
    }
}

/// Parse `PIMIENTO_THEME` (`system` / `light` / `dark`). Unknown or empty → System.
pub(crate) fn theme_preference_from_env(raw: Option<&str>) -> ThemePreference {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("light") => ThemePreference::Light,
        Some("dark") => ThemePreference::Dark,
        _ => ThemePreference::System,
    }
}

pub(crate) fn apply_theme_preference(
    preference: ThemePreference,
    window: &mut Window,
    cx: &mut App,
) {
    cx.set_global(ThemePreferenceState(preference));
    match preference {
        ThemePreference::System => Theme::sync_system_appearance(Some(window), cx),
        ThemePreference::Light => Theme::change(ThemeMode::Light, Some(window), cx),
        ThemePreference::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
    }
    // The label changes even when returning to System keeps the same concrete mode.
    window.refresh();
}

pub(crate) fn cycle_theme_preference(window: &mut Window, cx: &mut App) {
    let next = next_theme_preference(cx.global::<ThemePreferenceState>().0);
    apply_theme_preference(next, window, cx);
}

// ── SessionView ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RecentSession {
    #[serde(rename = "sessionFile")]
    pub(crate) session_file: PathBuf,
    pub(crate) cwd: PathBuf,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(rename = "lastUsed", default)]
    pub(crate) last_used: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct PersistedWindowBounds {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistedUi {
    #[serde(default = "default_inspector_open")]
    pub(crate) inspector_open: bool,
}

pub(crate) fn default_inspector_open() -> bool {
    true
}

impl PersistedWindowBounds {
    pub(crate) fn from_bounds(bounds: Bounds<Pixels>) -> Option<Self> {
        let x = f32::from(bounds.origin.x);
        let y = f32::from(bounds.origin.y);
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if !x.is_finite()
            || !y.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
        {
            return None;
        }
        let bounds = normalize_window_bounds(bounds);
        Some(Self {
            x: bounds.origin.x.into(),
            y: bounds.origin.y.into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
        })
    }

    pub(crate) fn into_bounds(self) -> Option<Bounds<Pixels>> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || !self.width.is_finite()
            || !self.height.is_finite()
            || self.width <= 0.0
            || self.height <= 0.0
        {
            return None;
        }
        Some(Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(
                px(self.width.max(MIN_WINDOW_WIDTH)),
                px(self.height.max(MIN_WINDOW_HEIGHT)),
            ),
        })
    }
}

pub(crate) fn normalize_window_bounds(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds {
        origin: bounds.origin,
        size: size(
            bounds.size.width.max(px(MIN_WINDOW_WIDTH)),
            bounds.size.height.max(px(MIN_WINDOW_HEIGHT)),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionPersistence {
    pub(crate) root: PathBuf,
}

impl SessionPersistence {
    pub(crate) fn from_environment() -> Self {
        let home_override = std::env::var_os("PIMIENTO_HOME").map(PathBuf::from);
        let root = app_data_dir(home_override.as_deref(), home_dir().as_deref());
        Self { root }
    }

    #[cfg(test)]
    pub(crate) fn from_root(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn last_session_path(&self) -> PathBuf {
        self.root.join("last-session")
    }

    pub(crate) fn recent_sessions_path(&self) -> PathBuf {
        self.root.join("recent.json")
    }

    pub(crate) fn window_bounds_path(&self) -> PathBuf {
        self.root.join("window.json")
    }

    pub(crate) fn ui_path(&self) -> PathBuf {
        self.root.join("ui.json")
    }

    pub(crate) fn load_last_session(&self) -> Option<PathBuf> {
        let raw = std::fs::read_to_string(self.last_session_path()).ok()?;
        let raw = raw.trim();
        (!raw.is_empty()).then(|| PathBuf::from(raw))
    }

    pub(crate) fn remember_last_session(&self, session_file: Option<&str>) {
        let Some(session_file) = session_file.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        let _ = write_persistence_file(&self.last_session_path(), session_file);
    }

    pub(crate) fn load_recent_sessions(&self) -> Vec<RecentSession> {
        let Ok(raw) = std::fs::read_to_string(self.recent_sessions_path()) else {
            return Vec::new();
        };
        parse_recent_sessions(&raw)
    }

    pub(crate) fn save_recent_sessions(&self, sessions: &[RecentSession]) -> std::io::Result<()> {
        let sessions = normalize_recent_sessions(sessions.to_vec());
        let contents = serde_json::to_string_pretty(&sessions).map_err(|error| {
            std::io::Error::other(format!("serialize recent sessions: {error}"))
        })?;
        write_persistence_file(&self.recent_sessions_path(), &contents)
    }

    pub(crate) fn load_window_bounds(&self) -> Option<Bounds<Pixels>> {
        let raw = std::fs::read_to_string(self.window_bounds_path()).ok()?;
        serde_json::from_str::<PersistedWindowBounds>(&raw)
            .ok()?
            .into_bounds()
    }

    pub(crate) fn save_window_bounds(&self, bounds: Bounds<Pixels>) {
        let Some(record) = PersistedWindowBounds::from_bounds(bounds) else {
            return;
        };
        let Ok(contents) = serde_json::to_string_pretty(&record) else {
            return;
        };
        let _ = write_persistence_file(&self.window_bounds_path(), &contents);
    }

    pub(crate) fn load_inspector_open(&self) -> bool {
        let Ok(raw) = std::fs::read_to_string(self.ui_path()) else {
            return default_inspector_open();
        };
        serde_json::from_str::<PersistedUi>(&raw)
            .map_or_else(|_| default_inspector_open(), |ui| ui.inspector_open)
    }

    pub(crate) fn save_inspector_open(&self, inspector_open: bool) {
        let Ok(contents) = serde_json::to_string_pretty(&PersistedUi { inspector_open }) else {
            return;
        };
        let _ = write_persistence_file(&self.ui_path(), &contents);
    }

    pub(crate) fn remember_recent_session(
        &self,
        session_file: Option<&str>,
        cwd: Option<&Path>,
        name: Option<&str>,
    ) {
        let Some(session_file) = session_file.map(str::trim).filter(|file| !file.is_empty()) else {
            return;
        };
        let Some(cwd) = cwd.filter(|path| !path.as_os_str().is_empty()) else {
            return;
        };

        let mut sessions = self.load_recent_sessions();
        let last_used = next_last_used(&sessions);
        let record = RecentSession {
            session_file: PathBuf::from(session_file),
            cwd: cwd.to_owned(),
            name: name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map_or_else(|| default_session_name(cwd), str::to_owned),
            last_used,
        };
        sessions.retain(|existing| existing.session_file != record.session_file);
        sessions.push(record);
        let _ = self.save_recent_sessions(&sessions);
    }

    pub(crate) fn forget_session(&self, session_file: &Path) {
        let mut sessions = self.load_recent_sessions();
        let original_len = sessions.len();
        sessions.retain(|session| session.session_file != session_file);
        if sessions.len() != original_len {
            let _ = self.save_recent_sessions(&sessions);
        }
        if self.load_last_session().as_deref() == Some(session_file) {
            let _ = std::fs::remove_file(self.last_session_path());
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LauncherBootstrap {
    pub(crate) persistence: SessionPersistence,
    pub(crate) launcher_cwd: PathBuf,
    pub(crate) recent_sessions: Vec<RecentSession>,
    pub(crate) last_session: Option<PathBuf>,
}

// ── OMP connection helper ─────────────────────────────────────────────────

pub(crate) fn app_data_dir(home_override: Option<&Path>, home: Option<&Path>) -> PathBuf {
    if let Some(path) = home_override.filter(|path| !path.as_os_str().is_empty()) {
        return path.to_owned();
    }
    home.map_or_else(|| PathBuf::from(".pimiento"), |path| path.join(".pimiento"))
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn omp_agent_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_DIR") {
        return Some(PathBuf::from(dir));
    }
    home_dir().map(|home| home.join(".omp").join("agent"))
}

pub(crate) fn omp_sessions_root() -> Option<PathBuf> {
    omp_agent_dir().map(|dir| dir.join("sessions"))
}

pub(crate) fn encode_relative_session_dir_name(prefix: &str, relative: &str) -> String {
    let encoded = relative.replace(['/', '\\', ':'], "-");
    if encoded.is_empty() {
        prefix.to_owned()
    } else if prefix.ends_with('-') {
        format!("{prefix}{encoded}")
    } else {
        format!("{prefix}-{encoded}")
    }
}

pub(crate) fn encode_legacy_absolute_session_dir_name(cwd: &Path) -> String {
    let trimmed = cwd
        .to_string_lossy()
        .trim_start_matches(['/', '\\'])
        .replace(['/', '\\', ':'], "-");
    format!("--{trimmed}--")
}

pub(crate) fn encode_omp_session_dir_name(
    cwd: &Path,
    home: Option<&Path>,
    temp_root: &Path,
) -> String {
    if let Some(home) = home
        && let Ok(relative) = cwd.strip_prefix(home)
    {
        return encode_relative_session_dir_name("-", &relative.to_string_lossy());
    }
    if let Ok(relative) = cwd.strip_prefix(temp_root) {
        return encode_relative_session_dir_name("-tmp", &relative.to_string_lossy());
    }
    encode_legacy_absolute_session_dir_name(cwd)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OmpSessionHeader {
    pub(crate) id: String,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) title: Option<String>,
    pub(crate) first_user_message: Option<String>,
}

pub(crate) fn extract_message_text(content: &serde_json::Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    let Some(parts) = content.as_array() else {
        return String::new();
    };
    parts
        .iter()
        .filter_map(|part| {
            (part.get("type").and_then(|v| v.as_str()) == Some("text"))
                .then(|| part.get("text").and_then(|v| v.as_str()).map(str::to_owned))
                .flatten()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn parse_omp_session_header_prefix(raw: &str) -> Option<OmpSessionHeader> {
    let mut id = None;
    let mut cwd = None;
    let mut title = None;
    let mut first_user_message = None;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match value.get("type").and_then(|v| v.as_str()) {
            Some("session") => {
                id = value
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .or(id);
                cwd = value
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .or(cwd);
                title = value
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or(title);
            }
            Some("title") => {
                title = value
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or(title);
            }
            Some("message") => {
                if first_user_message.is_some() {
                    continue;
                }
                let Some(message) = value.get("message") else {
                    continue;
                };
                if message.get("role").and_then(|v| v.as_str()) != Some("user") {
                    continue;
                }
                let text = message
                    .get("content")
                    .map(extract_message_text)
                    .unwrap_or_default();
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    first_user_message = Some(text.chars().take(96).collect());
                }
            }
            _ => {}
        }
    }
    let id = id?;
    Some(OmpSessionHeader {
        id,
        cwd,
        title,
        first_user_message,
    })
}

pub(crate) fn read_omp_session_header(path: &Path) -> Option<OmpSessionHeader> {
    let file = std::fs::File::open(path).ok()?;
    let mut limited = file.take(SESSION_HEADER_PREFIX_BYTES as u64);
    let mut raw = String::new();
    std::io::Read::read_to_string(&mut limited, &mut raw).ok()?;
    parse_omp_session_header_prefix(&raw)
}

pub(crate) fn mtime_unix_seconds(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs())
}

pub(crate) fn discover_omp_sessions_for_cwd(
    cwd: &Path,
    sessions_root: Option<&Path>,
    home: Option<&Path>,
    temp_root: &Path,
) -> Vec<RecentSession> {
    let Some(sessions_root) = sessions_root else {
        return Vec::new();
    };
    let dir_name = encode_omp_session_dir_name(cwd, home, temp_root);
    let session_dir = sessions_root.join(dir_name);
    let Ok(entries) = std::fs::read_dir(&session_dir) else {
        return Vec::new();
    };
    let mut discovered = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(header) = read_omp_session_header(&path) else {
            continue;
        };
        if let Some(session_cwd) = header.cwd.as_deref()
            && session_cwd != cwd
        {
            continue;
        }
        let name = header
            .title
            .filter(|title| !title.trim().is_empty())
            .or(header.first_user_message)
            .unwrap_or_else(|| default_session_name(cwd));
        let last_used = mtime_unix_seconds(&path);
        discovered.push(RecentSession {
            session_file: path,
            cwd: cwd.to_owned(),
            name,
            last_used,
        });
    }
    discovered.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| a.session_file.cmp(&b.session_file))
    });
    discovered.truncate(MAX_DISCOVERED_SESSIONS);
    discovered
}

pub(crate) fn collect_launcher_sessions(
    persistence: &SessionPersistence,
    cwd: &Path,
    sessions_root: Option<&Path>,
    home: Option<&Path>,
    temp_root: &Path,
) -> Vec<RecentSession> {
    let mut sessions = discover_omp_sessions_for_cwd(cwd, sessions_root, home, temp_root);
    for remembered in persistence.load_recent_sessions() {
        if remembered.cwd == cwd {
            sessions.push(remembered);
        }
    }
    sessions.retain(|session| session.cwd == cwd && session.session_file.exists());
    // Prefer richer names when duplicates collide on session_file
    sessions.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| a.session_file.cmp(&b.session_file))
    });
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for session in sessions {
        if seen.insert(session.session_file.clone()) {
            deduped.push(session);
        }
    }
    deduped.truncate(MAX_DISCOVERED_SESSIONS);
    deduped
}

pub(crate) fn write_persistence_file(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let Some(file_name) = path.file_name() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "persistence path has no file name",
        ));
    };
    let nonce = PERSISTENCE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(
        ".{}.tmp-{}-{nonce}",
        file_name.to_string_lossy(),
        std::process::id(),
    ));
    if let Err(error) = std::fs::write(&temporary, contents) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    match std::fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            let _ = std::fs::remove_file(&temporary);
            std::fs::write(path, contents).map_err(|write_error| {
                std::io::Error::new(
                    write_error.kind(),
                    format!("rename failed ({rename_error}); fallback write failed: {write_error}"),
                )
            })
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum RecentSessionsFile {
    List(Vec<RecentSession>),
    Wrapped { sessions: Vec<RecentSession> },
}

pub(crate) fn normalize_recent_sessions(mut sessions: Vec<RecentSession>) -> Vec<RecentSession> {
    sessions.retain(|session| {
        !session.session_file.as_os_str().is_empty() && !session.cwd.as_os_str().is_empty()
    });
    sessions.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| a.session_file.cmp(&b.session_file))
    });
    let mut seen = HashSet::new();
    sessions.retain(|session| seen.insert(session.session_file.clone()));
    sessions.truncate(MAX_RECENT_SESSIONS);
    sessions
}

pub(crate) fn parse_recent_sessions(raw: &str) -> Vec<RecentSession> {
    match serde_json::from_str::<RecentSessionsFile>(raw) {
        Ok(RecentSessionsFile::List(sessions) | RecentSessionsFile::Wrapped { sessions }) => {
            normalize_recent_sessions(sessions)
        }
        Err(_) => Vec::new(),
    }
}

pub(crate) fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub(crate) fn next_last_used(sessions: &[RecentSession]) -> u64 {
    let now = current_unix_seconds();
    let previous = sessions
        .iter()
        .map(|session| session.last_used)
        .max()
        .unwrap_or(0);
    now.max(previous.saturating_add(1))
}

pub(crate) fn default_session_name(cwd: &Path) -> String {
    cwd.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| cwd.display().to_string())
}

pub(crate) fn projection_session_name(projection: &SessionProjection, cwd: &Path) -> String {
    projection
        .state
        .state
        .as_ref()
        .and_then(|state| state.get("sessionName"))
        .and_then(|name| name.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or_else(|| default_session_name(cwd), str::to_owned)
}

pub(crate) fn resolve_launcher_path(path: &Path, current_dir: Option<&Path>) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        return None;
    }
    if path.is_absolute() {
        return Some(path.to_owned());
    }
    current_dir.map(|base| base.join(path))
}

pub(crate) fn initial_launcher_directory(
    cwd_override: Option<&Path>,
    recent: &[RecentSession],
    current: Option<PathBuf>,
) -> Option<PathBuf> {
    let current = current.filter(|path| path.is_absolute());
    let current_dir = current.as_deref();
    cwd_override
        .and_then(|path| resolve_launcher_path(path, current_dir))
        .or_else(|| {
            recent
                .iter()
                .find_map(|session| resolve_launcher_path(&session.cwd, current_dir))
        })
        .or(current)
}

pub(crate) fn auto_connect_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.trim() == "1")
}

pub(crate) fn latest_resume_path(
    persistence: &SessionPersistence,
    recent: &[RecentSession],
) -> Option<PathBuf> {
    persistence
        .load_last_session()
        .or_else(|| recent.first().map(|session| session.session_file.clone()))
}

pub(crate) const MESSAGE_PAGE_LIMIT: u32 = 100;
pub(crate) const MESSAGE_PAGE_MAX_PAGES: usize = 50;
pub(crate) const MESSAGE_PAGE_BUSY_RETRIES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessagesPageErrorKind {
    Busy,
    Stale,
    Other,
}
