use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use directories::ProjectDirs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AppearanceMode {
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BlockSpacing {
    Normal,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ClipboardEscapePolicy {
    Deny,
    WriteOnly,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ControlTabBehavior {
    OrderedTabs,
    MostRecentTab,
    MostRecentSession,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SupportedPlatform {
    Macos,
    Windows,
    Linux,
    Other,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum KeybindingAction {
    InsertCharacter,
    Backspace,
    Delete,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DeleteWordLeft,
    DeleteWordRight,
    ClearLine,
    AcceptSuggestion,
    DismissSuggestion,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    FunctionKey,
    Escape,
    Tab,
    ReverseTab,
    Enter,
    NewTab,
    CloseTab,
    ReopenClosedTab,
    NextTab,
    PreviousTab,
    MostRecentTab,
    MoveTabLeft,
    MoveTabRight,
    PinTab,
    RenameTab,
    SplitRight,
    SplitDown,
    ClosePane,
    ZoomPane,
    FocusLeftPane,
    FocusRightPane,
    FocusUpPane,
    FocusDownPane,
    FocusNextPane,
    FocusPreviousPane,
    ResizePaneLeft,
    ResizePaneRight,
    ResizePaneUp,
    ResizePaneDown,
    Find,
    FindNext,
    FindPrevious,
    CloseFind,
    OpenBlockFilter,
    ClearBlockFilter,
    PreviousBlock,
    NextBlock,
    FirstBlock,
    FocusInput,
    CopySelection,
    CopyBlock,
    Paste,
    PasteEscaped,
    PastePath,
    SelectAll,
    CommandPalette,
    Settings,
    GlobalSearch,
    ToggleSidePanel,
    ToggleQuickAccess,
    SendInterrupt,
    SendEof,
    SendSuspend,
    SendLiteralEscape,
    SendLiteralTab,
    ClearView,
}

impl KeybindingAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::InsertCharacter => "Insert character",
            Self::Backspace => "Backspace",
            Self::Delete => "Delete",
            Self::WordLeft => "Word left",
            Self::WordRight => "Word right",
            Self::LineStart => "Line start",
            Self::LineEnd => "Line end",
            Self::DeleteWordLeft => "Delete word left",
            Self::DeleteWordRight => "Delete word right",
            Self::ClearLine => "Clear line",
            Self::AcceptSuggestion => "Accept suggestion",
            Self::DismissSuggestion => "Dismiss suggestion",
            Self::ArrowUp => "Arrow up",
            Self::ArrowDown => "Arrow down",
            Self::ArrowLeft => "Arrow left",
            Self::ArrowRight => "Arrow right",
            Self::Home => "Home",
            Self::End => "End",
            Self::PageUp => "Page up",
            Self::PageDown => "Page down",
            Self::FunctionKey => "Function key",
            Self::Escape => "Escape",
            Self::Tab => "Tab",
            Self::ReverseTab => "Reverse tab",
            Self::Enter => "Enter",
            Self::NewTab => "New tab",
            Self::CloseTab => "Close tab",
            Self::ReopenClosedTab => "Reopen closed tab",
            Self::NextTab => "Next tab",
            Self::PreviousTab => "Previous tab",
            Self::MostRecentTab => "Most recent tab",
            Self::MoveTabLeft => "Move tab left",
            Self::MoveTabRight => "Move tab right",
            Self::PinTab => "Pin/unpin tab",
            Self::RenameTab => "Rename tab",
            Self::SplitRight => "Split right",
            Self::SplitDown => "Split down",
            Self::ClosePane => "Close pane",
            Self::ZoomPane => "Zoom pane",
            Self::FocusLeftPane => "Focus left pane",
            Self::FocusRightPane => "Focus right pane",
            Self::FocusUpPane => "Focus up pane",
            Self::FocusDownPane => "Focus down pane",
            Self::FocusNextPane => "Focus next pane",
            Self::FocusPreviousPane => "Focus previous pane",
            Self::ResizePaneLeft => "Resize pane left",
            Self::ResizePaneRight => "Resize pane right",
            Self::ResizePaneUp => "Resize pane up",
            Self::ResizePaneDown => "Resize pane down",
            Self::Find => "Find",
            Self::FindNext => "Find next",
            Self::FindPrevious => "Find previous",
            Self::CloseFind => "Close find",
            Self::OpenBlockFilter => "Open block filter",
            Self::ClearBlockFilter => "Clear block filter",
            Self::PreviousBlock => "Previous command block",
            Self::NextBlock => "Next command block",
            Self::FirstBlock => "First command block",
            Self::FocusInput => "Focus command input",
            Self::CopySelection => "Copy selection",
            Self::CopyBlock => "Copy block",
            Self::Paste => "Paste",
            Self::PasteEscaped => "Paste escaped",
            Self::PastePath => "Paste path",
            Self::SelectAll => "Select all",
            Self::CommandPalette => "Actions",
            Self::Settings => "Settings",
            Self::GlobalSearch => "Global search",
            Self::ToggleSidePanel => "Toggle side panel",
            Self::ToggleQuickAccess => "Toggle quick access",
            Self::SendInterrupt => "Send interrupt",
            Self::SendEof => "Send EOF",
            Self::SendSuspend => "Send suspend",
            Self::SendLiteralEscape => "Send literal escape",
            Self::SendLiteralTab => "Send literal tab",
            Self::ClearView => "Clear view",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct KeybindingSettings {
    pub override_file: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingConflict {
    pub shortcut: String,
    pub actions: Vec<KeybindingAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedKeybindings {
    pub bindings: BTreeMap<KeybindingAction, Vec<String>>,
    pub removed: BTreeSet<KeybindingAction>,
    pub conflicts: Vec<KeybindingConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeybindingOverride {
    Bind(Vec<String>),
    Remove,
}

pub type KeybindingOverrides = BTreeMap<KeybindingAction, KeybindingOverride>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AppearanceSettings {
    pub mode: AppearanceMode,
    pub block_spacing: BlockSpacing,
    pub window_opacity: f32,
    pub minimum_contrast: bool,
    pub zero_state_blocks: bool,
    pub cursor_style: CursorStyle,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            mode: AppearanceMode::System,
            block_spacing: BlockSpacing::Compact,
            window_opacity: 1.0,
            minimum_contrast: true,
            zero_state_blocks: true,
            cursor_style: CursorStyle::Block,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FontSettings {
    pub family: String,
    pub size: f32,
    pub weight: u16,
    pub line_height: f32,
    pub ligatures: bool,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            family: "JetBrains Mono, SF Mono, Menlo, Consolas, monospace".to_owned(),
            size: 13.0,
            weight: 400,
            line_height: 1.25,
            ligatures: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TerminalSettings {
    pub audible_bell: bool,
    pub max_grid_rows: usize,
    pub alternate_screen_padding: u16,
    pub clipboard_escape_policy: ClipboardEscapePolicy,
    pub background_find: bool,
    pub scrollback_lines_per_tick: usize,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            audible_bell: false,
            max_grid_rows: 50_000,
            alternate_screen_padding: 0,
            clipboard_escape_policy: ClipboardEscapePolicy::Deny,
            background_find: true,
            scrollback_lines_per_tick: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct InputSettings {
    pub left_alt_is_meta: bool,
    pub right_alt_is_meta: bool,
    pub control_tab_behavior: ControlTabBehavior,
    pub vim_like_editing: bool,
    pub keybindings: KeybindingSettings,
}

impl Default for InputSettings {
    fn default() -> Self {
        Self {
            left_alt_is_meta: !cfg!(target_os = "macos"),
            right_alt_is_meta: !cfg!(target_os = "macos"),
            control_tab_behavior: ControlTabBehavior::OrderedTabs,
            vim_like_editing: false,
            keybindings: KeybindingSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WorkspaceSettings {
    pub restore_sessions: bool,
    pub close_last_tab_closes_window: bool,
    pub tab_layout: String,
    pub quick_access_enabled: bool,
    pub quick_access_position: String,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            restore_sessions: true,
            close_last_tab_closes_window: false,
            tab_layout: "horizontal".to_owned(),
            quick_access_enabled: false,
            quick_access_position: "top".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct StartupSettings {
    pub shell: Option<String>,
    pub working_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PrivacySettings {
    pub redaction_patterns: Vec<String>,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            redaction_patterns: vec![
                "(?i)(password|token|secret|api[_-]?key)=\\S+".to_owned(),
                "[a-z]+://[^:@\\s]+:[^@\\s]+@".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct Settings {
    pub appearance: AppearanceSettings,
    pub font: FontSettings,
    pub terminal: TerminalSettings,
    pub input: InputSettings,
    pub workspace: WorkspaceSettings,
    pub startup: StartupSettings,
    pub privacy: PrivacySettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct LoadedSettings {
    pub settings: Settings,
    pub path: PathBuf,
    pub issues: Vec<SettingsIssue>,
    pub keybindings: ResolvedKeybindings,
}

#[derive(Debug, Clone)]
pub struct SettingsWatcher {
    path: PathBuf,
    last_modified: Option<std::time::SystemTime>,
    pending_since: Option<Instant>,
    debounce: Duration,
}

impl SettingsWatcher {
    pub fn new(path: PathBuf, debounce: Duration) -> Self {
        let last_modified = modified_at(&path);
        Self {
            path,
            last_modified,
            pending_since: None,
            debounce,
        }
    }

    pub fn poll(&mut self) -> Option<LoadedSettings> {
        let modified = modified_at(&self.path);
        if modified != self.last_modified {
            self.last_modified = modified;
            self.pending_since = Some(Instant::now());
            return None;
        }

        let pending_since = self.pending_since?;
        if pending_since.elapsed() < self.debounce {
            return None;
        }

        self.pending_since = None;
        Some(load_settings_from_path(self.path.clone()))
    }
}

pub fn settings_path() -> PathBuf {
    ProjectDirs::from("dev", "terminaste", "terminaste")
        .map(|dirs| dirs.config_dir().join("settings.toml"))
        .unwrap_or_else(|| PathBuf::from("terminaste-settings.toml"))
}

pub fn keybindings_path_for_settings_path(path: &Path) -> PathBuf {
    path.with_file_name("keybindings.yaml")
}

pub fn current_platform() -> SupportedPlatform {
    if cfg!(target_os = "macos") {
        SupportedPlatform::Macos
    } else if cfg!(target_os = "windows") {
        SupportedPlatform::Windows
    } else if cfg!(target_os = "linux") {
        SupportedPlatform::Linux
    } else {
        SupportedPlatform::Other
    }
}

pub fn platform_is_supported(platforms: &[SupportedPlatform], platform: SupportedPlatform) -> bool {
    platforms.is_empty() || platforms.contains(&platform)
}

pub fn filter_supported_platforms<T: Clone>(
    items: &[(T, Vec<SupportedPlatform>)],
    platform: SupportedPlatform,
) -> Vec<T> {
    items
        .iter()
        .filter(|(_, platforms)| platform_is_supported(platforms, platform))
        .map(|(item, _)| item.clone())
        .collect()
}

pub fn load_settings() -> LoadedSettings {
    load_settings_from_path(settings_path())
}

pub fn load_settings_from_path(path: PathBuf) -> LoadedSettings {
    let mut settings = Settings::default();
    let mut issues = Vec::new();

    match fs::read_to_string(&path) {
        Ok(raw) => match toml::from_str::<toml::Value>(&raw) {
            Ok(parsed) => {
                let mut merged =
                    toml::Value::try_from(Settings::default()).expect("default settings serialize");
                merge_setting_fields(&parsed, &mut merged, &mut Vec::new(), &mut issues);
                settings =
                    sanitize_settings(merged.try_into().expect("validated settings"), &mut issues);
            }
            Err(error) => issues.push(SettingsIssue {
                path: "settings".to_owned(),
                message: error.to_string(),
            }),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => issues.push(SettingsIssue {
            path: path.display().to_string(),
            message: error.to_string(),
        }),
    }

    let keybindings = load_keybindings_for_settings(&path, &settings, &mut issues);

    LoadedSettings {
        settings,
        path,
        issues,
        keybindings,
    }
}

fn merge_setting_fields(
    source: &toml::Value,
    target: &mut toml::Value,
    path: &mut Vec<String>,
    issues: &mut Vec<SettingsIssue>,
) {
    if let Some(table) = source.as_table() {
        for (key, value) in table {
            path.push(key.clone());
            merge_setting_fields(value, target, path, issues);
            path.pop();
        }
        return;
    }
    let mut candidate = target.clone();
    let mut entry = &mut candidate;
    for key in path.iter() {
        if !entry.is_table() {
            *entry = toml::Value::Table(toml::map::Map::new());
        }
        entry = entry
            .as_table_mut()
            .unwrap()
            .entry(key.clone())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    }
    *entry = source.clone();
    match candidate.clone().try_into::<Settings>() {
        Ok(_) => *target = candidate,
        Err(error) => issues.push(SettingsIssue {
            path: path.join("."),
            message: error.to_string(),
        }),
    }
}

pub fn save_settings(path: &Path, settings: &Settings) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, toml::to_string_pretty(settings)?)?;
    Ok(())
}

pub fn settings_schema_json() -> anyhow::Result<String> {
    let schema = schemars::schema_for!(Settings);
    Ok(serde_json::to_string_pretty(&schema)?)
}

pub fn default_action_map() -> BTreeMap<KeybindingAction, Vec<String>> {
    use KeybindingAction::*;

    let app = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl+shift"
    };
    let mut map = BTreeMap::new();
    map.insert(Backspace, vec!["backspace".to_owned()]);
    map.insert(Delete, vec!["delete".to_owned()]);
    map.insert(WordLeft, vec!["alt+left".to_owned()]);
    map.insert(WordRight, vec!["alt+right".to_owned()]);
    map.insert(LineStart, vec!["home".to_owned()]);
    map.insert(LineEnd, vec!["end".to_owned()]);
    map.insert(DeleteWordLeft, vec!["ctrl+backspace".to_owned()]);
    map.insert(DeleteWordRight, vec!["ctrl+delete".to_owned()]);
    map.insert(ClearLine, vec!["ctrl+u".to_owned()]);
    map.insert(AcceptSuggestion, vec!["tab".to_owned()]);
    map.insert(DismissSuggestion, vec!["escape".to_owned()]);
    map.insert(ArrowUp, vec!["up".to_owned()]);
    map.insert(ArrowDown, vec!["down".to_owned()]);
    map.insert(ArrowLeft, vec!["left".to_owned()]);
    map.insert(ArrowRight, vec!["right".to_owned()]);
    map.insert(Home, vec!["home".to_owned()]);
    map.insert(End, vec!["end".to_owned()]);
    map.insert(PageUp, vec!["pageup".to_owned()]);
    map.insert(PageDown, vec!["pagedown".to_owned()]);
    map.insert(Escape, vec!["escape".to_owned()]);
    map.insert(Tab, vec!["tab".to_owned()]);
    map.insert(ReverseTab, vec!["shift+tab".to_owned()]);
    map.insert(Enter, vec!["enter".to_owned()]);
    map.insert(NewTab, vec![format!("{app}+t")]);
    map.insert(CloseTab, vec![format!("{app}+w")]);
    map.insert(ReopenClosedTab, vec![format!("{app}+shift+t")]);
    map.insert(NextTab, vec![format!("{app}+]")]);
    map.insert(PreviousTab, vec![format!("{app}+[")]);
    map.insert(MostRecentTab, vec!["ctrl+tab".to_owned()]);
    map.insert(MoveTabLeft, vec![format!("{app}+shift+[")]);
    map.insert(MoveTabRight, vec![format!("{app}+shift+]")]);
    map.insert(PinTab, vec![format!("{app}+shift+p")]);
    map.insert(RenameTab, vec![format!("{app}+shift+r")]);
    map.insert(SplitRight, vec![format!("{app}+d")]);
    map.insert(SplitDown, vec![format!("{app}+shift+d")]);
    map.insert(ClosePane, vec![format!("{app}+shift+w")]);
    map.insert(ZoomPane, vec![format!("{app}+shift+enter")]);
    map.insert(
        FocusLeftPane,
        vec![if cfg!(target_os = "macos") {
            "cmd+alt+left".to_owned()
        } else {
            "alt+shift+left".to_owned()
        }],
    );
    map.insert(
        FocusRightPane,
        vec![if cfg!(target_os = "macos") {
            "cmd+alt+right".to_owned()
        } else {
            "alt+shift+right".to_owned()
        }],
    );
    map.insert(
        FocusUpPane,
        vec![if cfg!(target_os = "macos") {
            "cmd+alt+up".to_owned()
        } else {
            "alt+shift+up".to_owned()
        }],
    );
    map.insert(
        FocusDownPane,
        vec![if cfg!(target_os = "macos") {
            "cmd+alt+down".to_owned()
        } else {
            "alt+shift+down".to_owned()
        }],
    );
    map.insert(FocusNextPane, vec!["ctrl+alt+tab".to_owned()]);
    map.insert(FocusPreviousPane, vec!["ctrl+alt+shift+tab".to_owned()]);
    map.insert(ResizePaneLeft, vec!["alt+shift+left".to_owned()]);
    map.insert(ResizePaneRight, vec!["alt+shift+right".to_owned()]);
    map.insert(ResizePaneUp, vec!["alt+shift+up".to_owned()]);
    map.insert(ResizePaneDown, vec!["alt+shift+down".to_owned()]);
    map.insert(Find, vec![format!("{app}+f")]);
    map.insert(FindNext, vec![format!("{app}+g")]);
    map.insert(FindPrevious, vec![format!("{app}+shift+g")]);
    map.insert(CloseFind, vec!["escape".to_owned()]);
    map.insert(OpenBlockFilter, vec![format!("{app}+shift+f")]);
    map.insert(ClearBlockFilter, vec![format!("{app}+shift+backspace")]);
    map.insert(
        PreviousBlock,
        vec![if cfg!(target_os = "macos") {
            "cmd+up".to_owned()
        } else {
            "ctrl+up".to_owned()
        }],
    );
    map.insert(
        NextBlock,
        vec![if cfg!(target_os = "macos") {
            "cmd+down".to_owned()
        } else {
            "ctrl+down".to_owned()
        }],
    );
    map.insert(
        FirstBlock,
        vec![if cfg!(target_os = "macos") {
            "cmd+shift+up".to_owned()
        } else {
            "ctrl+shift+up".to_owned()
        }],
    );
    map.insert(
        FocusInput,
        vec![if cfg!(target_os = "macos") {
            "cmd+shift+down".to_owned()
        } else {
            "ctrl+shift+down".to_owned()
        }],
    );
    map.insert(CopySelection, vec![format!("{app}+c")]);
    map.insert(
        CopyBlock,
        vec![if cfg!(target_os = "macos") {
            "cmd+shift+c".to_owned()
        } else {
            "ctrl+shift+c".to_owned()
        }],
    );
    map.insert(Paste, vec![format!("{app}+v")]);
    map.insert(PasteEscaped, vec![format!("{app}+shift+v")]);
    map.insert(PastePath, vec![format!("{app}+alt+v")]);
    map.insert(SelectAll, vec![format!("{app}+a")]);
    map.insert(CommandPalette, vec![format!("{app}+p")]);
    map.insert(Settings, vec![format!("{app}+,")]);
    map.insert(GlobalSearch, vec![format!("{app}+shift+o")]);
    map.insert(ToggleSidePanel, vec![format!("{app}+b")]);
    map.insert(ToggleQuickAccess, vec![format!("{app}+`")]);
    map.insert(SendInterrupt, vec!["ctrl+c".to_owned()]);
    map.insert(SendEof, vec!["ctrl+d".to_owned()]);
    map.insert(SendSuspend, vec!["ctrl+z".to_owned()]);
    map.insert(SendLiteralEscape, vec!["escape".to_owned()]);
    map.insert(SendLiteralTab, vec!["tab".to_owned()]);
    map.insert(ClearView, vec![format!("{app}+k")]);
    map.insert(
        NextTab,
        vec![
            "ctrl+tab".to_owned(),
            if cfg!(target_os = "macos") {
                "cmd+shift+]".to_owned()
            } else {
                "ctrl+pagedown".to_owned()
            },
        ],
    );
    map.insert(
        PreviousTab,
        vec![
            "ctrl+shift+tab".to_owned(),
            if cfg!(target_os = "macos") {
                "cmd+shift+[".to_owned()
            } else {
                "ctrl+pageup".to_owned()
            },
        ],
    );
    if !cfg!(target_os = "macos") {
        map.insert(ReopenClosedTab, vec!["ctrl+alt+t".to_owned()]);
        map.insert(SplitDown, vec!["ctrl+alt+shift+d".to_owned()]);
    }
    map
}

pub fn resolve_keybindings(overrides: &KeybindingOverrides) -> ResolvedKeybindings {
    let mut bindings = default_action_map();
    let mut removed = BTreeSet::new();

    for (action, override_value) in overrides {
        match override_value {
            KeybindingOverride::Bind(shortcuts) => {
                bindings.insert(*action, normalize_shortcuts(shortcuts));
                removed.remove(action);
            }
            KeybindingOverride::Remove => {
                bindings.remove(action);
                removed.insert(*action);
            }
        }
    }

    let conflicts = detect_keybinding_conflicts(&bindings);
    ResolvedKeybindings {
        bindings,
        removed,
        conflicts,
    }
}

pub fn detect_keybinding_conflicts(
    bindings: &BTreeMap<KeybindingAction, Vec<String>>,
) -> Vec<KeybindingConflict> {
    let mut by_shortcut: BTreeMap<String, Vec<KeybindingAction>> = BTreeMap::new();
    for (action, shortcuts) in bindings {
        for shortcut in shortcuts {
            by_shortcut
                .entry(normalize_shortcut(shortcut))
                .or_default()
                .push(*action);
        }
    }

    by_shortcut
        .into_iter()
        .filter_map(|(shortcut, actions)| {
            if actions.len() > 1 {
                Some(KeybindingConflict { shortcut, actions })
            } else {
                None
            }
        })
        .collect()
}

pub fn load_keybinding_overrides(path: &Path) -> Result<KeybindingOverrides, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    parse_keybinding_overrides(&raw)
}

pub fn parse_keybinding_overrides(raw: &str) -> Result<KeybindingOverrides, String> {
    let yaml: BTreeMap<String, serde_yaml::Value> =
        serde_yaml::from_str(raw).map_err(|error| error.to_string())?;
    let entries = match yaml.get("bindings") {
        Some(serde_yaml::Value::Mapping(mapping)) => mapping,
        Some(_) => return Err("bindings must be a mapping".to_owned()),
        None => return Ok(BTreeMap::new()),
    };

    let mut overrides = BTreeMap::new();
    for (raw_action, raw_binding) in entries {
        let Some(action_name) = raw_action.as_str() else {
            return Err("binding action keys must be strings".to_owned());
        };
        let action: KeybindingAction =
            serde_yaml::from_value(serde_yaml::Value::String(action_name.to_owned()))
                .map_err(|error| error.to_string())?;
        let value = parse_keybinding_override(raw_binding)?;
        overrides.insert(action, value);
    }
    Ok(overrides)
}

fn load_keybindings_for_settings(
    settings_path: &Path,
    settings: &Settings,
    issues: &mut Vec<SettingsIssue>,
) -> ResolvedKeybindings {
    let keybindings_path = settings
        .input
        .keybindings
        .override_file
        .clone()
        .unwrap_or_else(|| keybindings_path_for_settings_path(settings_path));

    match load_keybinding_overrides(&keybindings_path) {
        Ok(overrides) => resolved_keybindings_with_issues(overrides, issues),
        Err(error) if is_not_found_message(&error) => resolve_keybindings(&BTreeMap::new()),
        Err(error) => {
            issues.push(SettingsIssue {
                path: keybindings_path.display().to_string(),
                message: error,
            });
            resolve_keybindings(&BTreeMap::new())
        }
    }
}

fn resolved_keybindings_with_issues(
    overrides: KeybindingOverrides,
    issues: &mut Vec<SettingsIssue>,
) -> ResolvedKeybindings {
    let resolved = resolve_keybindings(&overrides);
    for conflict in &resolved.conflicts {
        issues.push(SettingsIssue {
            path: format!("input.keybindings.{}", conflict.shortcut),
            message: format!("shortcut is used by {:?}", conflict.actions),
        });
    }
    resolved
}

fn parse_keybinding_override(value: &serde_yaml::Value) -> Result<KeybindingOverride, String> {
    match value {
        serde_yaml::Value::String(text) if text == "remove" || text == "-" => {
            Ok(KeybindingOverride::Remove)
        }
        serde_yaml::Value::String(text) => {
            Ok(KeybindingOverride::Bind(vec![normalize_shortcut(text)]))
        }
        serde_yaml::Value::Sequence(values) => {
            let mut shortcuts = Vec::new();
            for value in values {
                let Some(shortcut) = value.as_str() else {
                    return Err("shortcut lists must contain only strings".to_owned());
                };
                shortcuts.push(normalize_shortcut(shortcut));
            }
            Ok(KeybindingOverride::Bind(shortcuts))
        }
        serde_yaml::Value::Mapping(mapping) => {
            if mapping
                .get(serde_yaml::Value::String("remove".to_owned()))
                .and_then(serde_yaml::Value::as_bool)
                .unwrap_or(false)
            {
                Ok(KeybindingOverride::Remove)
            } else if let Some(bindings) = mapping.get(serde_yaml::Value::String("keys".to_owned()))
            {
                parse_keybinding_override(bindings)
            } else {
                Err("binding mapping must contain remove: true or keys".to_owned())
            }
        }
        _ => Err("binding must be a string, list, or removal marker".to_owned()),
    }
}

fn normalize_shortcuts(shortcuts: &[String]) -> Vec<String> {
    shortcuts
        .iter()
        .map(|shortcut| normalize_shortcut(shortcut))
        .collect()
}

fn normalize_shortcut(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("+")
}

fn modified_at(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn is_not_found_message(message: &str) -> bool {
    message.contains("No such file")
        || message.contains("os error 2")
        || message.contains("cannot find the file")
}

fn sanitize_settings(mut settings: Settings, issues: &mut Vec<SettingsIssue>) -> Settings {
    if !(8.0..=48.0).contains(&settings.font.size) {
        issues.push(SettingsIssue {
            path: "font.size".to_owned(),
            message: "font size must be between 8 and 48".to_owned(),
        });
        settings.font.size = FontSettings::default().size;
    }
    if !(0.8..=2.2).contains(&settings.font.line_height) {
        issues.push(SettingsIssue {
            path: "font.line_height".to_owned(),
            message: "line height must be between 0.8 and 2.2".to_owned(),
        });
        settings.font.line_height = FontSettings::default().line_height;
    }
    if settings.font.weight < 100 || settings.font.weight > 900 {
        issues.push(SettingsIssue {
            path: "font.weight".to_owned(),
            message: "font weight must be between 100 and 900".to_owned(),
        });
        settings.font.weight = FontSettings::default().weight;
    }
    if settings.terminal.max_grid_rows < 100 || settings.terminal.max_grid_rows > 1_000_000 {
        issues.push(SettingsIssue {
            path: "terminal.max_grid_rows".to_owned(),
            message: "max grid rows must be between 100 and 1000000".to_owned(),
        });
        settings.terminal.max_grid_rows = TerminalSettings::default().max_grid_rows;
    }
    if !(0.2..=1.0).contains(&settings.appearance.window_opacity) {
        issues.push(SettingsIssue {
            path: "appearance.window_opacity".to_owned(),
            message: "window opacity must be between 0.2 and 1.0".to_owned(),
        });
        settings.appearance.window_opacity = AppearanceSettings::default().window_opacity;
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_field_uses_default_and_reports_issue() {
        let raw = r#"
            [font]
            size = 99
        "#;
        let parsed: Settings = toml::from_str(raw).unwrap();
        let mut issues = Vec::new();
        let settings = sanitize_settings(parsed, &mut issues);
        assert_eq!(settings.font.size, FontSettings::default().size);
        assert_eq!(issues[0].path, "font.size");
    }

    #[test]
    fn invalid_toml_reports_issue_and_uses_defaults() {
        let path = temp_path("invalid-settings.toml");
        fs::write(&path, "[font\nsize = 12").unwrap();

        let loaded = load_settings_from_path(path.clone());

        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(loaded.issues[0].path, "settings");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_and_load_settings_round_trip() {
        let path = temp_path("settings-round-trip.toml");
        let mut settings = Settings::default();
        settings.font.size = 18.0;
        settings.workspace.quick_access_enabled = true;

        save_settings(&path, &settings).unwrap();
        let loaded = load_settings_from_path(path.clone());

        assert_eq!(loaded.settings.font.size, 18.0);
        assert!(loaded.settings.workspace.quick_access_enabled);
        assert!(loaded.issues.is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn keybinding_removal_marker_removes_default_binding() {
        let overrides = parse_keybinding_overrides(
            r#"
            bindings:
              new-tab: remove
            "#,
        )
        .unwrap();

        let resolved = resolve_keybindings(&overrides);

        assert!(!resolved.bindings.contains_key(&KeybindingAction::NewTab));
        assert!(resolved.removed.contains(&KeybindingAction::NewTab));
    }

    #[test]
    fn keybinding_conflict_detection_reports_shared_shortcuts() {
        let overrides = parse_keybinding_overrides(
            r#"
            bindings:
              new-tab: ctrl+x
              close-tab: ctrl+x
            "#,
        )
        .unwrap();

        let resolved = resolve_keybindings(&overrides);

        assert!(resolved
            .conflicts
            .iter()
            .any(|conflict| conflict.shortcut == "ctrl+x"
                && conflict.actions.contains(&KeybindingAction::NewTab)
                && conflict.actions.contains(&KeybindingAction::CloseTab)));
    }

    #[test]
    fn watcher_reloads_changed_file_after_debounce() {
        let path = temp_path("watched-settings.toml");
        fs::write(&path, "[font]\nsize = 12\n").unwrap();
        let mut watcher = SettingsWatcher::new(path.clone(), Duration::from_millis(0));

        std::thread::sleep(Duration::from_millis(5));
        fs::write(&path, "[font]\nsize = 20\n").unwrap();

        assert!(watcher.poll().is_none());
        let loaded = watcher.poll().unwrap();
        assert_eq!(loaded.settings.font.size, 20.0);
        let _ = fs::remove_file(path);
    }

    fn temp_path(name: &str) -> PathBuf {
        let unique = format!(
            "terminaste-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn schema_mentions_terminal_settings() {
        let schema = settings_schema_json().unwrap();
        assert!(schema.contains("max_grid_rows"));
    }
}
