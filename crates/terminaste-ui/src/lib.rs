use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use base64::prelude::{Engine, BASE64_URL_SAFE_NO_PAD};
use crossbeam_channel::TryRecvError;
use serde::{Deserialize, Serialize};
use terminaste_completion::{
    apply_completion, complete_if_current, CompletionItem, CompletionKind, CompletionRequest,
};
use terminaste_core::{encode_paste, CommandBlock, TerminalEvent, TerminalModel};
use terminaste_pty::{PtyCommand, PtyConfig, PtyEvent, PtySession};
use terminaste_settings::{LoadedSettings, Settings};
use uuid::Uuid;

mod editor;
mod integration;
mod pane_tree;
mod session;
mod shell_prompt;
mod terminal_surface;
mod theme;
mod view;

use editor::CommandEditorState;
pub use theme::TerminasteTheme;
pub use view::{install_actions, TerminalWindow};

pub struct TerminasteApp {
    window_bounds: Option<gpui::WindowBounds>,
    loaded: LoadedSettings,
    tabs: Vec<TabState>,
    active_tab: usize,
    closed_tabs: Vec<TabState>,
    toast: Option<(String, Instant)>,
    headless: bool,
    watcher: Option<terminaste_settings::SettingsWatcher>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalPaneLayoutSnapshot {
    pub blocks_stick_to_bottom: bool,
    pub input_below_blocks: bool,
    pub input_fixed_to_bottom: bool,
}

impl TerminasteApp {
    pub fn new(loaded: LoadedSettings) -> Self {
        let mut app = Self {
            window_bounds: None,
            watcher: Some(terminaste_settings::SettingsWatcher::new(
                loaded.path.clone(),
                Duration::from_millis(200),
            )),
            loaded,
            tabs: Vec::new(),
            active_tab: 0,
            closed_tabs: Vec::new(),
            toast: None,
            headless: false,
        };
        app.restore_session();
        if app.tabs.is_empty() {
            app.tabs
                .push(TabState::new("local".to_owned(), &app.loaded.settings));
        }
        app
    }

    pub fn headless_for_tests(settings: Settings) -> Self {
        Self {
            window_bounds: None,
            loaded: LoadedSettings {
                settings,
                path: PathBuf::from("test-settings.toml"),
                issues: Vec::new(),
                keybindings: terminaste_settings::resolve_keybindings(
                    &std::collections::BTreeMap::new(),
                ),
            },
            tabs: vec![TabState::fake("test".to_owned())],
            active_tab: 0,
            closed_tabs: Vec::new(),
            toast: None,
            headless: true,
            watcher: None,
        }
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn window_bounds(&self) -> Option<gpui::WindowBounds> {
        self.window_bounds
    }

    pub fn active_pane_count(&self) -> usize {
        self.tabs
            .get(self.active_tab)
            .map_or(0, |tab| tab.panes.len())
    }

    pub fn submit_active_input_for_tests(&mut self, input: &str) {
        if let Some(pane) = self.active_terminal_mut() {
            pane.editor.set_text(input.to_owned());
            pane.submit_input();
        }
    }

    pub fn active_pane_snapshot_for_tests(&self) -> Option<terminaste_core::TerminalSnapshot> {
        self.active_terminal().map(|pane| pane.model.snapshot())
    }

    pub fn active_pane_layout_snapshot_for_tests(&self) -> Option<TerminalPaneLayoutSnapshot> {
        self.active_terminal().map(|_| TerminalPaneLayoutSnapshot {
            blocks_stick_to_bottom: true,
            input_below_blocks: true,
            input_fixed_to_bottom: true,
        })
    }

    pub fn active_pane_editor_text_for_tests(&self) -> Option<String> {
        self.active_terminal()
            .map(|pane| pane.editor.text().to_owned())
    }

    pub fn active_pane_input_prompt_for_tests(&self) -> Option<String> {
        self.active_terminal().map(TerminalPane::input_prompt_label)
    }

    pub fn active_pane_fake_pty_for_tests(
        &self,
    ) -> Option<(
        crossbeam_channel::Sender<PtyEvent>,
        crossbeam_channel::Receiver<PtyCommand>,
    )> {
        self.active_terminal()
            .and_then(|pane| pane.fake_pty_channels.as_ref())
            .map(|channels| (channels.events.clone(), channels.commands.clone()))
    }

    pub fn drain_pty_events_for_tests(&mut self) {
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                pane.drain_pty_events();
            }
        }
    }

    pub fn send_active_pane_output_for_tests(&mut self, bytes: &[u8]) {
        if let Some(pane) = self.active_terminal_mut() {
            pane.process_ordered_pty_bytes(bytes);
        }
    }

    pub fn finish_active_command_for_tests(&mut self, exit_code: i32) {
        if let Some(pane) = self.active_terminal_mut() {
            pane.finish_command_for_tests(exit_code);
        }
    }

    pub fn resize_active_pane_for_tests(&mut self, cols: u16, rows: u16) {
        if let Some(pane) = self.active_terminal_mut() {
            pane.resize_for_tests(cols, rows);
        }
    }

    pub fn active_block_focus_for_tests(&self) -> Option<(Uuid, &'static str)> {
        self.active_terminal().and_then(|pane| {
            pane.block_focus
                .focused()
                .map(|focus| (focus.block_id, "block"))
        })
    }

    pub fn navigate_active_block_focus_for_tests(
        &mut self,
        delta: isize,
    ) -> Option<(Uuid, &'static str)> {
        self.active_terminal_mut().and_then(|pane| {
            let targets = pane.focusable_block_targets();
            pane.block_focus
                .navigate(&targets, delta)
                .map(|focus| (focus.block_id, "block"))
        })
    }

    pub fn new_tab_for_tests(&mut self) {
        self.tabs
            .push(TabState::fake(format!("session {}", self.tabs.len() + 1)));
        self.active_tab = self.tabs.len() - 1;
    }
    pub fn close_active_tab_for_tests(&mut self) {
        self.close_tab(self.active_tab);
    }
    pub fn reopen_closed_tab_for_tests(&mut self) {
        self.reopen_closed_tab();
    }
    pub fn split_active_for_tests(&mut self) {
        self.split_active(SplitDirection::Right);
    }

    pub fn set_font_size_for_tests(&mut self, size: f32) {
        self.loaded.settings.font.size = size;
        let rows = (720.0 / (size * self.loaded.settings.font.line_height).max(10.0))
            .floor()
            .clamp(5.0, 200.0) as u16;
        if let Some(pane) = self.active_terminal_mut() {
            pane.resize_for_tests(pane.cols, rows);
        }
    }

    fn new_tab(&mut self) {
        let title = format!("session {}", self.tabs.len() + 1);
        let mut settings = self.loaded.settings.clone();
        if let Some(pane) = self.active_terminal() {
            settings.startup.working_directory = Some(pane.cwd.clone());
        }
        self.tabs.push(if self.headless {
            let mut tab = TabState::fake(title);
            tab.panes[0].cwd = startup_directory(&settings);
            tab
        } else {
            TabState::new(title, &settings)
        });
        self.active_tab = self.tabs.len() - 1;
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            self.tabs[0] = if self.headless {
                TabState::fake("local".to_owned())
            } else {
                TabState::new("local".to_owned(), &self.loaded.settings)
            };
            self.toast("Restarted last tab".to_owned());
            return;
        }
        self.closed_tabs.push(self.tabs.remove(index));
        if self.closed_tabs.len() > 10 {
            self.closed_tabs.remove(0);
        }
        if index < self.active_tab {
            self.active_tab -= 1;
        } else if index == self.active_tab {
            self.active_tab = index.min(self.tabs.len() - 1);
        }
    }

    fn reopen_closed_tab(&mut self) {
        if let Some(tab) = self.closed_tabs.pop() {
            self.tabs.push(tab);
            self.active_tab = self.tabs.len() - 1;
        }
    }

    fn activate_relative_tab(&mut self, delta: isize) {
        self.active_tab = wrap_index(self.active_tab, self.tabs.len(), delta);
    }

    fn split_active(&mut self, direction: SplitDirection) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            let mut settings = self.loaded.settings.clone();
            settings.startup.working_directory = Some(tab.panes[tab.active_pane].cwd.clone());
            tab.panes.push(if self.headless {
                TerminalPane::fake()
            } else {
                TerminalPane::new(&settings)
            });
            tab.tree.split(
                tab.panes[tab.active_pane].id,
                tab.panes.last().unwrap().id,
                direction,
            );
            tab.active_pane = tab.panes.len() - 1;
        }
    }

    fn focus_relative_pane(&mut self, delta: isize) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.active_pane = wrap_index(tab.active_pane, tab.panes.len(), delta);
        }
    }

    fn active_terminal_mut(&mut self) -> Option<&mut TerminalPane> {
        self.tabs
            .get_mut(self.active_tab)
            .and_then(|tab| tab.panes.get_mut(tab.active_pane))
    }
    fn active_terminal(&self) -> Option<&TerminalPane> {
        self.tabs
            .get(self.active_tab)
            .and_then(|tab| tab.panes.get(tab.active_pane))
    }
    fn pane_mut(&mut self, id: Uuid) -> Option<&mut TerminalPane> {
        self.tabs
            .iter_mut()
            .flat_map(|tab| &mut tab.panes)
            .find(|pane| pane.id == id)
    }
    fn toast(&mut self, message: String) {
        self.toast = Some((message, Instant::now()));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SplitDirection {
    Right,
    Down,
}

struct TabState {
    title: String,
    panes: Vec<TerminalPane>,
    active_pane: usize,
    tree: pane_tree::PaneTree,
}

impl TabState {
    fn new(title: String, settings: &Settings) -> Self {
        let pane = TerminalPane::new(settings);
        Self {
            title,
            tree: pane_tree::PaneTree::Leaf(pane.id),
            panes: vec![pane],
            active_pane: 0,
        }
    }
    fn fake(title: String) -> Self {
        let pane = TerminalPane::fake();
        Self {
            title,
            tree: pane_tree::PaneTree::Leaf(pane.id),
            panes: vec![pane],
            active_pane: 0,
        }
    }
    fn display_title(&self) -> String {
        self.panes
            .get(self.active_pane)
            .map(|pane| pane.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| self.title.clone())
    }
    fn compact_title(&self) -> String {
        static HOST: OnceLock<String> = OnceLock::new();
        let host = HOST.get_or_init(|| {
            Command::new("hostname")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
                .unwrap_or_default()
        });
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        let title = self.display_title();
        if let Some((identity, path)) = title.split_once(':') {
            if let Some((title_user, title_host)) = identity.split_once('@') {
                if title_user == user
                    && title_host
                        .split('.')
                        .next()
                        .unwrap_or("")
                        .eq_ignore_ascii_case(host.split('.').next().unwrap_or(""))
                {
                    return path
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .filter(|part| !part.is_empty())
                        .unwrap_or("/")
                        .to_owned();
                }
                return identity.to_owned();
            }
        }
        if title == "local" || title == "terminaste" {
            return self
                .panes
                .get(self.active_pane)
                .and_then(|pane| pane.cwd.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or(title);
        }
        title
    }
}

struct TerminalPane {
    id: Uuid,
    model: TerminalModel,
    pty: Option<PtySession>,
    fake_pty_channels: Option<FakePtyChannels>,
    editor: CommandEditorState,
    editor_has_focus: bool,
    completion_revision: u64,
    completions: Vec<CompletionItem>,
    selected_completion: usize,
    completion_navigating: bool,
    history_offset: usize,
    history_search: Option<String>,
    ghost_dismissed: Option<String>,
    history_suggestions: Vec<String>,
    cwd: PathBuf,
    aliases: Vec<(String, String)>,
    title: String,
    pending_title: Option<(String, Instant)>,
    cols: u16,
    rows: u16,
    status: String,
    active_command: Option<ActiveCommandMeta>,
    last_command_summary: Option<String>,
    last_editor_cursor: usize,
    suppress_prompt_output: bool,
    prompt_capture: Vec<u8>,
    input_prompt_text: Option<String>,
    integration_ready: bool,
    block_focus: BlockFocusState,
    scroll_focused_block: bool,
    surface: terminal_surface::SurfaceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TerminalBlockFocus {
    block_id: Uuid,
}

#[derive(Debug, Clone, Default)]
struct BlockFocusState {
    focused: Option<TerminalBlockFocus>,
}

impl BlockFocusState {
    fn focused(&self) -> Option<TerminalBlockFocus> {
        self.focused
    }
    fn focus(&mut self, target: TerminalBlockFocus) {
        self.focused = Some(target);
    }
    fn navigate(
        &mut self,
        targets: &[TerminalBlockFocus],
        delta: isize,
    ) -> Option<TerminalBlockFocus> {
        if targets.is_empty() {
            self.focused = None;
            return None;
        }
        let current = self
            .focused
            .and_then(|focused| targets.iter().position(|target| *target == focused))
            .unwrap_or_else(|| if delta < 0 { targets.len() - 1 } else { 0 });
        let next = if self.focused.is_some() {
            current.saturating_add_signed(delta)
        } else {
            current
        };
        self.focused = targets.get(next).copied();
        self.focused
    }
}

#[derive(Clone)]
struct FakePtyChannels {
    events: crossbeam_channel::Sender<PtyEvent>,
    commands: crossbeam_channel::Receiver<PtyCommand>,
}

#[derive(Debug, Clone)]
struct ActiveCommandMeta {
    command: String,
    started_at: Instant,
}

impl TerminalPane {
    fn new(settings: &Settings) -> Self {
        let cwd = startup_directory(settings);
        let config = PtyConfig {
            cols: 80,
            rows: 24,
            shell: settings.startup.shell.clone(),
            working_directory: Some(cwd.clone()),
        };
        let (pty, status) = match PtySession::spawn(config) {
            Ok(session) => (Some(session), "running".to_owned()),
            Err(error) => (None, format!("shell failed: {error}")),
        };
        Self::with_pty(settings, cwd, pty, status)
    }

    fn with_pty(
        settings: &Settings,
        cwd: PathBuf,
        pty: Option<PtySession>,
        status: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            model: TerminalModel::new(80, 24, settings.terminal.max_grid_rows),
            pty,
            fake_pty_channels: None,
            editor: CommandEditorState::new(),
            editor_has_focus: false,
            completion_revision: 0,
            completions: Vec::new(),
            selected_completion: 0,
            completion_navigating: false,
            history_offset: 0,
            history_search: None,
            ghost_dismissed: None,
            history_suggestions: Vec::new(),
            cwd,
            aliases: Vec::new(),
            title: "local".to_owned(),
            pending_title: None,
            cols: 80,
            rows: 24,
            status,
            active_command: None,
            last_command_summary: None,
            last_editor_cursor: 0,
            suppress_prompt_output: false,
            prompt_capture: Vec::new(),
            input_prompt_text: None,
            integration_ready: false,
            block_focus: BlockFocusState::default(),
            scroll_focused_block: false,
            surface: terminal_surface::SurfaceState::default(),
        }
    }

    fn fake() -> Self {
        let (pty, events, commands) = PtySession::fake();
        let mut pane = Self::with_pty(
            &Settings::default(),
            PathBuf::from("."),
            Some(pty),
            "fake".to_owned(),
        );
        pane.fake_pty_channels = Some(FakePtyChannels { events, commands });
        pane.title = "test".to_owned();
        pane
    }

    fn drain_pty_events(&mut self) -> bool {
        let Some(rx) = self.pty.as_ref().map(|pty| pty.rx.clone()) else {
            return false;
        };
        let deadline = Instant::now() + Duration::from_millis(4);
        let mut changed = false;
        loop {
            match rx.try_recv() {
                Ok(PtyEvent::Output(bytes)) => self.process_ordered_pty_bytes(&bytes),
                Ok(PtyEvent::Exited(code)) => {
                    self.status = format!(
                        "exited {}",
                        code.map(|code| code.to_string())
                            .unwrap_or_else(|| "unknown".to_owned())
                    );
                    self.model.finish_running_command(code.unwrap_or(0));
                    self.active_command = None;
                    self.focus_input();
                }
                Ok(PtyEvent::Error(error)) => self.status = error,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
            changed = true;
            if Instant::now() >= deadline {
                break;
            }
        }
        if self
            .pending_title
            .as_ref()
            .is_some_and(|(_, created)| created.elapsed() >= Duration::from_millis(200))
        {
            let (title, _) = self.pending_title.take().unwrap();
            if self.title != title {
                self.title = title;
                changed = true;
            }
        }
        changed
    }

    fn input_prompt_label(&self) -> String {
        self.input_prompt_text
            .as_deref()
            .filter(|prompt| !prompt.trim().is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                self.model
                    .snapshot()
                    .visible_lines
                    .iter()
                    .rev()
                    .map(|line| shell_prompt::plain_text(line))
                    .find(|line| !line.trim().is_empty())
            })
            .unwrap_or_else(|| prompt_label(&self.cwd))
    }

    fn refresh_completions(&mut self) {
        if !self.editor.preedit.is_empty() {
            return;
        }
        if self.surface.input_bridge || self.surface.query_bridge {
            if self.surface.completion_request.is_none() && !self.surface.submit_pending {
                self.request_shell_completions(false);
            }
            return;
        }
        self.refresh_local_completions();
    }

    fn refresh_local_completions(&mut self) {
        if self.surface.isolated_shell {
            return;
        }
        self.completion_revision = self.completion_revision.saturating_add(1);
        let mut request = CompletionRequest::from_env(
            self.editor.text().to_owned(),
            self.last_editor_cursor.min(self.editor.text().len()),
            self.cwd.clone(),
            self.completion_revision,
        );
        request.aliases = self.aliases.clone();
        self.completions = complete_if_current(&request, self.completion_revision)
            .map(|batch| batch.items)
            .unwrap_or_default();
        self.selected_completion = 0;
        self.completion_navigating = false;
    }

    fn show_history(&mut self) {
        let prefix = if self.editor.text().contains('\n') {
            String::new()
        } else {
            self.editor.text().to_owned()
        };
        self.history_search = Some(prefix.clone());
        if self.surface.input_bridge || self.surface.query_bridge {
            self.request_shell_completions(true);
        } else {
            let mut seen = std::collections::HashSet::new();
            self.completions = self
                .model
                .command_blocks()
                .iter()
                .rev()
                .filter(|block| block.command.starts_with(&prefix))
                .filter(|block| seen.insert(block.command.clone()))
                .map(|block| CompletionItem {
                    label: block.command.clone(),
                    replacement: block.command.clone(),
                    description: "history".to_owned(),
                    kind: CompletionKind::Command,
                    range: 0..self.editor.text().len(),
                    score: 0,
                })
                .collect();
            self.selected_completion = 0;
            self.completion_navigating = false;
        }
    }

    fn history_suggestion(&self, columns: usize) -> Option<String> {
        let text = self.editor.text();
        if text.is_empty()
            || text.contains('\n')
            || self.editor.cursor != text.len()
            || self.editor.selection_range().is_some()
            || !self.editor.preedit.is_empty()
            || self.editor.shell_command_mode
            || !self.completions.is_empty()
            || self.surface.completion_request.is_some()
            || self.ghost_dismissed.as_deref() == Some(text)
        {
            return None;
        }
        self.model
            .command_blocks()
            .iter()
            .rev()
            .map(|block| &block.command)
            .chain(self.history_suggestions.iter())
            .find(|command| {
                command.len() > text.len()
                    && command.starts_with(text)
                    && !command.contains(['\n', '\r'])
                    && unicode_width::UnicodeWidthStr::width(command.as_str()) < columns
            })
            .cloned()
    }

    fn move_completion(&mut self, up: bool) {
        if (up && self.selected_completion + 1 >= self.completions.len())
            || (!up && self.selected_completion == 0)
        {
            self.dismiss_completions();
            self.focus_input();
        } else {
            self.selected_completion =
                self.selected_completion
                    .saturating_add_signed(if up { 1 } else { -1 });
            self.completion_navigating = true;
        }
    }

    fn recall_history(&mut self, delta: isize) {
        let prefix = self
            .history_search
            .get_or_insert_with(|| self.editor.text().to_owned());
        let blocks = self.model.command_blocks();
        let matches = blocks
            .iter()
            .filter(|block| block.command.starts_with(prefix.as_str()))
            .collect::<Vec<_>>();
        self.history_offset = self
            .history_offset
            .saturating_add_signed(delta)
            .min(matches.len());
        self.editor.set_text(if self.history_offset == 0 {
            prefix.clone()
        } else {
            matches[matches.len() - self.history_offset].command.clone()
        });
    }

    fn accept_completion(&mut self) {
        self.history_search = None;
        self.history_offset = 0;
        if let Some(item) = self.completions.get(self.selected_completion) {
            self.editor
                .set_text(apply_completion(self.editor.text(), item));
            self.last_editor_cursor = self
                .surface
                .completion_cursors
                .get(self.selected_completion)
                .map(|cursor| char_to_byte_index(self.editor.text(), *cursor))
                .unwrap_or(item.range.start + item.replacement.len());
            self.editor
                .set_cursor_from_mouse_index(self.last_editor_cursor, false);
            if self.surface.input_bridge {
                self.sync_shell_editor();
            }
        }
        self.dismiss_completions();
        self.editor.focus_requested = true;
    }

    fn dismiss_completions(&mut self) {
        self.history_search = None;
        self.history_offset = 0;
        self.completions.clear();
        self.surface.completion_cursors.clear();
        self.surface.completion_request = None;
        self.completion_navigating = false;
    }

    fn submit_input(&mut self) {
        if self.active_command.is_some() && !self.model.modes().alternate_screen {
            let input = self.editor.text().trim_end_matches(['\n', '\r']).to_owned();
            let mut bytes = encode_paste(&input, self.model.modes());
            bytes.push(b'\r');
            self.write_terminal(bytes);
            self.editor.clear();
            self.dismiss_completions();
            return;
        }
        if self.surface.submit_pending || self.surface.completion_request.is_some() {
            return;
        }
        self.history_offset = 0;
        self.history_search = None;
        self.completion_navigating = false;
        if self.surface.input_bridge {
            self.sync_shell_editor();
            self.send_shell_editor_key(terminaste_core::KeyCode::Enter);
            self.model
                .start_integrated_command(self.editor.text().to_owned());
            self.select_running_block();
            self.editor.clear();
            self.surface.submit_pending = true;
            self.dismiss_completions();
            return;
        }
        let command = self.editor.text().trim_end_matches(['\n', '\r']).to_owned();
        if command.trim().is_empty() {
            self.editor.clear();
            self.completions.clear();
            return;
        }
        self.active_command = Some(ActiveCommandMeta {
            command: command.clone(),
            started_at: Instant::now(),
        });
        self.model.discard_pending_output();
        self.model.start_integrated_command(command.clone());
        self.select_running_block();
        let mut bytes = encode_paste(&command, self.model.modes());
        bytes.push(b'\r');
        self.write_terminal(bytes);
        self.editor.clear();
        self.completions.clear();
    }

    fn resize_for_tests(&mut self, cols: u16, rows: u16) {
        self.cols = cols.max(1);
        self.rows = rows.max(1);
        self.model
            .resize(usize::from(self.cols), usize::from(self.rows));
        if let Some(pty) = &self.pty {
            let _ = pty.tx.send(PtyCommand::Resize {
                cols: self.cols,
                rows: self.rows,
            });
        }
    }
    fn clear_view(&mut self) {
        self.model = TerminalModel::new(usize::from(self.cols), usize::from(self.rows), 50_000);
    }
    fn finish_command_for_tests(&mut self, exit_code: i32) {
        self.model.finish_running_command(exit_code);
        self.active_command = None;
        self.focus_input();
        self.status = if exit_code == 0 {
            "ready".to_owned()
        } else {
            format!("last command failed: {exit_code}")
        };
    }
    fn focusable_block_targets(&self) -> Vec<TerminalBlockFocus> {
        self.model
            .command_blocks()
            .iter()
            .map(|block| TerminalBlockFocus { block_id: block.id })
            .collect()
    }
    fn navigate_block(&mut self, delta: isize) {
        if delta > 0 && self.block_focus.focused().is_none() {
            self.focus_input();
            return;
        }
        let targets = self.focusable_block_targets();
        if self.block_focus.navigate(&targets, delta).is_some() {
            self.editor_has_focus = false;
            self.editor.focus_requested = false;
            self.scroll_focused_block = true;
        } else if delta > 0 {
            self.focus_input();
        }
    }
    fn focus_first_block(&mut self) {
        if let Some(target) = self.focusable_block_targets().first().copied() {
            self.editor_has_focus = false;
            self.editor.focus_requested = false;
            self.block_focus.focus(target);
            self.scroll_focused_block = true;
        }
    }
    fn focus_input(&mut self) {
        self.block_focus.focused = None;
        self.editor.focus_requested = true;
    }
    fn select_running_block(&mut self) {
        if let Some(block) = self
            .model
            .command_blocks()
            .iter()
            .rev()
            .find(|block| block.running)
        {
            self.block_focus
                .focus(TerminalBlockFocus { block_id: block.id });
            self.editor_has_focus = true;
            self.editor.focus_requested = true;
            self.scroll_focused_block = false;
        }
    }
    fn focused_block_text(&self) -> Option<String> {
        let focused = self.block_focus.focused()?;
        self.model
            .command_blocks()
            .iter()
            .find(|block| block.id == focused.block_id)
            .map(block_input_and_output)
    }
    fn match_count(&self, query: &str) -> usize {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return 0;
        }
        let snapshot = self.model.snapshot();
        snapshot
            .blocks
            .iter()
            .map(|block| count_matches(&block_context_text(block), &query))
            .sum::<usize>()
            + count_matches(&snapshot.visible_lines.join("\n"), &query)
    }
}

fn startup_directory(settings: &Settings) -> PathBuf {
    settings
        .startup
        .working_directory
        .clone()
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_owned()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn wrap_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize + delta).rem_euclid(len as isize) as usize
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
fn format_duration(duration_ms: u64) -> String {
    if duration_ms < 1_000 {
        return format!("{duration_ms}ms");
    }
    format!("{:.1}s", duration_ms as f64 / 1_000.0)
}
fn prompt_label(cwd: &std::path::Path) -> String {
    let name = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("/");
    format!("{name} ›")
}
fn block_matches(text: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty()
        || text
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
}
fn count_matches(text: &str, query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    text.to_ascii_lowercase().match_indices(query).count()
}
fn block_context_text(block: &CommandBlock) -> String {
    let mut text = format!("$ {}", block.command);
    if let Some(code) = block.exit_code {
        text.push_str(&format!("\nexit {code}"));
    }
    if let Some(duration) = block.duration_ms {
        text.push_str(&format!("\nduration {}", format_duration(duration)));
    }
    if !block.output.is_empty() {
        text.push('\n');
        text.push_str(&block.output);
    }
    text
}
fn block_input_and_output(block: &CommandBlock) -> String {
    if block.output.is_empty() {
        block.command.clone()
    } else {
        format!(
            "{}\n{}",
            block.command,
            shell_prompt::plain_text(&block.output)
        )
    }
}

fn char_to_byte_index(text: &str, index: usize) -> usize {
    text.char_indices()
        .map(|(index, _)| index)
        .nth(index)
        .unwrap_or(text.len())
}

pub fn integration_frame_for_tests(name: &str, data_json: &str) -> Vec<u8> {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let data = serde_json::from_str::<serde_json::Value>(data_json).unwrap_or_default();
    let payload = BASE64_URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&serde_json::json!({
            "session": Uuid::nil().to_string(),
            "sequence": SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            "time_ms": 0, "data": data,
        }))
        .unwrap(),
    );
    format!("\x1bPterminaste;1;{name};json64;{payload}\x1b\\").into_bytes()
}

#[cfg(test)]
mod interaction_tests {
    use super::*;

    #[test]
    fn history_ghost_requires_an_exact_prefix_and_room_on_one_line() {
        let mut pane = TerminalPane::fake();
        pane.history_suggestions = vec!["echo hello".to_owned(), "echo\nworld".to_owned()];
        pane.editor.set_text("echo".to_owned());
        assert_eq!(pane.history_suggestion(80).as_deref(), Some("echo hello"));
        assert_eq!(pane.history_suggestion(8), None);
        pane.ghost_dismissed = Some("echo".to_owned());
        assert_eq!(pane.history_suggestion(80), None);
        pane.editor.set_text("echo ".to_owned());
        assert_eq!(pane.history_suggestion(80).as_deref(), Some("echo hello"));
        pane.editor.set_text("Echo".to_owned());
        assert_eq!(pane.history_suggestion(80), None);
        pane.editor.set_text("echo hello".to_owned());
        assert_eq!(pane.history_suggestion(80), None);
    }

    #[test]
    fn leaving_either_end_of_completion_list_preserves_input() {
        let mut pane = TerminalPane::fake();
        pane.editor.set_text("ec".to_owned());
        let items = ["echo", "echo hello"]
            .map(|text| CompletionItem {
                label: text.to_owned(),
                replacement: text.to_owned(),
                description: String::new(),
                kind: CompletionKind::Command,
                range: 0..2,
                score: 0,
            })
            .to_vec();
        pane.completions = items.clone();
        pane.selected_completion = 0;
        pane.move_completion(true);
        assert_eq!(pane.selected_completion, 1);
        pane.move_completion(true);
        assert!(pane.completions.is_empty());
        assert_eq!(pane.editor.text(), "ec");
        pane.completions = items;
        pane.selected_completion = 0;
        pane.move_completion(false);
        assert!(pane.completions.is_empty());
        assert_eq!(pane.editor.text(), "ec");
    }
}
