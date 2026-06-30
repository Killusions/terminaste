use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use base64::prelude::{Engine, BASE64_URL_SAFE_NO_PAD};
use crossbeam_channel::TryRecvError;
use eframe::egui::{self, Color32, FontData, FontFamily, FontId, Key, RichText, Sense, Stroke};
use serde::{Deserialize, Serialize};
use terminaste_completion::{
    apply_completion, complete_if_current, CompletionItem, CompletionKind, CompletionRequest,
};
use terminaste_core::{encode_paste, CommandBlock, TerminalEvent, TerminalModel};
use terminaste_pty::{PtyCommand, PtyConfig, PtyEvent, PtySession};
use terminaste_settings::{
    save_settings, AppearanceMode, BlockSpacing, ClipboardEscapePolicy, LoadedSettings, Settings,
};
use uuid::Uuid;

mod icons;
mod pane_tree;
mod session;
mod shell_prompt;
mod terminal_surface;

#[derive(Debug, Clone)]
pub struct TerminasteTheme {
    pub background: Color32,
    pub surface: Color32,
    pub surface_high: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub card: Color32,
    pub input: Color32,
}

impl TerminasteTheme {
    pub fn dark() -> Self {
        Self {
            background: Color32::from_rgb(10, 11, 15),
            surface: Color32::from_rgb(17, 19, 26),
            surface_high: Color32::from_rgb(25, 28, 38),
            text: Color32::from_rgb(238, 242, 255),
            muted: Color32::from_rgb(148, 163, 184),
            border: Color32::from_rgb(42, 48, 62),
            accent: Color32::from_rgb(112, 125, 150),
            success: Color32::from_rgb(34, 197, 94),
            warning: Color32::from_rgb(245, 158, 11),
            error: Color32::from_rgb(239, 68, 68),
            card: Color32::from_rgb(13, 15, 20),
            input: Color32::from_rgb(12, 14, 19),
        }
    }

    pub fn light() -> Self {
        Self {
            background: Color32::from_rgb(247, 248, 250),
            surface: Color32::from_rgb(255, 255, 255),
            surface_high: Color32::from_rgb(239, 242, 247),
            text: Color32::from_rgb(18, 24, 38),
            muted: Color32::from_rgb(100, 116, 139),
            border: Color32::from_rgb(218, 223, 232),
            accent: Color32::from_rgb(100, 110, 130),
            success: Color32::from_rgb(22, 163, 74),
            warning: Color32::from_rgb(217, 119, 6),
            error: Color32::from_rgb(220, 38, 38),
            card: Color32::from_rgb(253, 254, 255),
            input: Color32::from_rgb(250, 251, 253),
        }
    }
}

pub struct TerminasteApp {
    loaded: LoadedSettings,
    theme: TerminasteTheme,
    tabs: Vec<TabState>,
    active_tab: usize,
    closed_tabs: Vec<TabState>,
    settings_open: bool,
    command_palette_open: bool,
    actions_opened_this_frame: bool,
    find_open: bool,
    find_query: String,
    keybinding_search: String,
    settings_category: SettingsCategory,
    toast: Option<(String, Instant)>,
    headless: bool,
    watcher: Option<terminaste_settings::SettingsWatcher>,
    configured_font_family: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalPaneLayoutSnapshot {
    pub blocks_stick_to_bottom: bool,
    pub input_below_blocks: bool,
    pub input_fixed_to_bottom: bool,
}

impl TerminasteApp {
    pub fn new(loaded: LoadedSettings) -> Self {
        let theme = theme_for(&loaded.settings, None);
        let mut app = Self {
            watcher: Some(terminaste_settings::SettingsWatcher::new(
                loaded.path.clone(),
                Duration::from_millis(200),
            )),
            loaded,
            theme,
            tabs: Vec::new(),
            active_tab: 0,
            closed_tabs: Vec::new(),
            settings_open: false,
            command_palette_open: false,
            actions_opened_this_frame: false,
            find_open: false,
            find_query: String::new(),
            keybinding_search: String::new(),
            settings_category: SettingsCategory::Appearance,
            toast: None,
            headless: false,
            configured_font_family: String::new(),
        };
        app.restore_session();
        if app.tabs.is_empty() {
            app.tabs
                .push(TabState::new("local".to_owned(), &app.loaded.settings));
        }
        app
    }

    pub fn headless_for_tests(settings: Settings) -> Self {
        let loaded = LoadedSettings {
            settings,
            path: PathBuf::from("test-settings.toml"),
            issues: Vec::new(),
            keybindings: terminaste_settings::resolve_keybindings(
                &std::collections::BTreeMap::new(),
            ),
        };
        Self {
            theme: theme_for(&loaded.settings, None),
            loaded,
            tabs: vec![TabState::fake("test".to_owned())],
            active_tab: 0,
            closed_tabs: Vec::new(),
            settings_open: false,
            command_palette_open: false,
            actions_opened_this_frame: false,
            find_open: false,
            find_query: String::new(),
            keybinding_search: String::new(),
            settings_category: SettingsCategory::Appearance,
            toast: None,
            headless: true,
            watcher: None,
            configured_font_family: String::new(),
        }
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn active_pane_count(&self) -> usize {
        self.tabs
            .get(self.active_tab)
            .map(|tab| tab.panes.len())
            .unwrap_or(0)
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
        self.active_terminal().map(|pane| pane.layout)
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
        let ctx = egui::Context::default();
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                pane.drain_pty_events(&ctx);
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
                .map(|focused| (focused.block_id, "block"))
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
                .map(|focused| (focused.block_id, "block"))
        })
    }

    pub fn new_tab_for_tests(&mut self) {
        let title = format!("session {}", self.tabs.len() + 1);
        self.tabs.push(TabState::fake(title));
        self.active_tab = self.tabs.len() - 1;
    }

    pub fn close_active_tab_for_tests(&mut self) {
        self.close_active_tab();
    }

    pub fn reopen_closed_tab_for_tests(&mut self) {
        self.reopen_closed_tab();
    }

    pub fn split_active_for_tests(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.panes.push(TerminalPane::fake());
            tab.tree.split(
                tab.panes[tab.active_pane].id,
                tab.panes.last().unwrap().id,
                SplitDirection::Right,
            );
            tab.active_pane = tab.panes.len() - 1;
        }
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

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use terminaste_settings::KeybindingAction as Action;
        let events = ctx.input(|input| input.events.clone());
        let mut retained = Vec::new();
        for event in events {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = &event
            else {
                retained.push(event);
                continue;
            };
            if *key == Key::Escape
                && (self.settings_open || self.find_open || self.command_palette_open)
            {
                self.settings_open = false;
                self.find_open = false;
                self.command_palette_open = false;
                self.find_query.clear();
                ctx.memory_mut(|memory| {
                    if let Some(id) = memory.focused() {
                        memory.surrender_focus(id);
                    }
                });
                continue;
            }
            let supported = [
                Action::NewTab,
                Action::CloseTab,
                Action::ReopenClosedTab,
                Action::NextTab,
                Action::PreviousTab,
                Action::SplitRight,
                Action::SplitDown,
                Action::Find,
                Action::CommandPalette,
                Action::Settings,
                Action::ClearView,
                Action::PreviousBlock,
                Action::NextBlock,
                Action::FirstBlock,
                Action::FocusInput,
                Action::CopyBlock,
                Action::FocusNextPane,
                Action::FocusPreviousPane,
            ];
            let action = supported.into_iter().find(|action| {
                self.loaded
                    .keybindings
                    .bindings
                    .get(action)
                    .is_some_and(|bindings| {
                        bindings
                            .iter()
                            .any(|binding| shortcut_matches(binding, *key, *modifiers))
                    })
            });
            match action {
                Some(Action::NewTab) => self.new_tab(),
                Some(Action::CloseTab) => self.close_active_tab(),
                Some(Action::ReopenClosedTab) => self.reopen_closed_tab(),
                Some(Action::NextTab) => self.activate_relative_tab(1),
                Some(Action::PreviousTab) => self.activate_relative_tab(-1),
                Some(Action::SplitRight) => self.split_active(SplitDirection::Right),
                Some(Action::SplitDown) => self.split_active(SplitDirection::Down),
                Some(Action::Find) => self.find_open = true,
                Some(Action::CommandPalette) => self.open_actions(),
                Some(Action::Settings) => self.settings_open = true,
                Some(Action::ClearView) => {
                    if let Some(pane) = self.active_terminal_mut() {
                        pane.clear_view();
                    }
                }
                Some(Action::PreviousBlock) => self.jump_active_block(-1),
                Some(Action::NextBlock) => self.jump_active_block(1),
                Some(Action::FirstBlock) => {
                    if let Some(pane) = self.active_terminal_mut() {
                        pane.focus_first_block();
                    }
                }
                Some(Action::FocusInput) => {
                    if let Some(pane) = self.active_terminal_mut() {
                        pane.focus_input();
                    }
                }
                Some(Action::CopyBlock) => {
                    if let Some(text) = self
                        .active_terminal()
                        .and_then(TerminalPane::focused_block_text)
                    {
                        ctx.copy_text(text);
                    }
                }
                Some(Action::FocusNextPane) => self.focus_relative_pane(1),
                Some(Action::FocusPreviousPane) => self.focus_relative_pane(-1),
                _ => {
                    retained.push(event);
                    continue;
                }
            }
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        if self
            .active_terminal()
            .is_some_and(|pane| pane.surface.submit_pending && pane.active_command.is_none())
        {
            retained.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Paste(_)
                )
            });
        }
        ctx.input_mut(|input| input.events = retained);
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

    fn close_active_tab(&mut self) {
        self.close_tab(self.active_tab);
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
        let removed = self.tabs.remove(index);
        self.closed_tabs.push(removed);
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
        if self.tabs.is_empty() {
            return;
        }
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
            if tab.panes.is_empty() {
                return;
            }
            tab.active_pane = wrap_index(tab.active_pane, tab.panes.len(), delta);
        }
    }

    fn jump_active_block(&mut self, delta: isize) {
        if let Some(pane) = self.active_terminal_mut() {
            pane.navigate_block(delta);
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

    fn toast(&mut self, message: String) {
        self.toast = Some((message, Instant::now()));
    }

    fn open_actions(&mut self) {
        self.command_palette_open = true;
        self.actions_opened_this_frame = true;
    }
}

impl eframe::App for TerminasteApp {
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.persist_session();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.actions_opened_this_frame = false;
        if let Some(loaded) = self
            .watcher
            .as_mut()
            .and_then(terminaste_settings::SettingsWatcher::poll)
        {
            self.loaded = loaded;
        }
        let font_family = self.loaded.settings.font.family.clone();
        if self.configured_font_family != font_family {
            install_system_fonts(ctx, &font_family);
            self.configured_font_family = font_family;
        }
        self.theme = theme_for(&self.loaded.settings, ctx.system_theme());
        apply_egui_theme(ctx, &self.theme);
        self.handle_shortcuts(ctx);
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                pane.drain_pty_events(ctx);
            }
        }

        self.render_terminal_panel(ctx);

        if self.settings_open {
            self.render_settings(ctx);
        }
        if self.command_palette_open {
            self.render_command_palette(ctx);
        }
        if self.find_open {
            self.render_find(ctx);
        }
        self.render_toast(ctx);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

impl TerminasteApp {
    fn render_terminal_panel(&mut self, ctx: &egui::Context) {
        let interactive = self
            .active_terminal()
            .is_some_and(TerminalPane::has_interactive_surface);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(self.theme.background)
                    .inner_margin(if interactive {
                        egui::Margin::ZERO
                    } else {
                        egui::Margin {
                            left: 4,
                            right: 4,
                            top: 4,
                            bottom: 8,
                        }
                    }),
            )
            .show(ctx, |ui| {
                if interactive {
                    ui.spacing_mut().item_spacing.y = 0.0;
                }
                self.render_header(ui);
                if !interactive {
                    ui.add_space(4.0);
                }
                self.render_workspace(ui);
            });
    }
    fn render_header(&mut self, ui: &mut egui::Ui) {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, self.theme.surface);
        let control_rect = |index: usize| {
            egui::Rect::from_center_size(
                egui::pos2(rect.right() - 12.0 - index as f32 * 24.0, rect.center().y),
                egui::vec2(22.0, 24.0),
            )
        };
        if icons::button(
            ui,
            control_rect(0),
            ui.make_persistent_id("settings"),
            icons::Icon::Settings,
            "Settings",
            &self.theme,
        )
        .clicked()
        {
            self.settings_open = true;
        }
        if icons::button(
            ui,
            control_rect(1),
            ui.make_persistent_id("actions"),
            icons::Icon::Actions,
            "Actions",
            &self.theme,
        )
        .clicked()
        {
            self.open_actions();
        }
        if icons::button(
            ui,
            control_rect(2),
            ui.make_persistent_id("new-tab"),
            icons::Icon::Add,
            "New tab",
            &self.theme,
        )
        .clicked()
        {
            self.new_tab();
        }
        let available = (rect.width() - 74.0).max(0.0);
        let tab_width = (available / self.tabs.len().max(1) as f32).min(160.0);
        let empty_rect = egui::Rect::from_min_max(
            rect.min + egui::vec2(tab_width * self.tabs.len() as f32, 0.0),
            egui::pos2(rect.left() + available, rect.bottom()),
        );
        if empty_rect.is_positive()
            && ui
                .interact(
                    empty_rect,
                    ui.make_persistent_id("new-tab-space"),
                    Sense::click(),
                )
                .double_clicked()
        {
            self.new_tab();
        }
        let mut close_tab = None;
        let mut focus_editor = false;
        for (index, tab) in self.tabs.iter().enumerate() {
            let bounds = egui::Rect::from_min_size(
                rect.min + egui::vec2(index as f32 * tab_width, 1.0),
                egui::vec2((tab_width - 2.0).max(0.0), 26.0),
            );
            let selected = index == self.active_tab;
            if selected {
                ui.painter()
                    .rect_filled(bounds, 4.0, self.theme.surface_high);
            }
            let compact = tab_width < 44.0;
            let title = if tab_width < 28.0 {
                (index + 1).to_string()
            } else {
                tab.compact_title()
            };
            let close_width = if compact { 0.0 } else { 18.0 };
            let label_rect =
                egui::Rect::from_min_max(bounds.min, bounds.max - egui::vec2(close_width, 0.0));
            let response = ui
                .interact(
                    label_rect,
                    ui.make_persistent_id(("tab", index)),
                    Sense::click(),
                )
                .on_hover_text(tab.display_title());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::SelectableLabel,
                    ui.is_enabled(),
                    tab.compact_title(),
                )
            });
            let mut job = egui::text::LayoutJob::simple_singleline(
                title,
                FontId::proportional(12.0),
                if selected {
                    self.theme.text
                } else {
                    self.theme.muted
                },
            );
            job.wrap.max_width = (label_rect.width() - 8.0).max(0.0);
            job.wrap.max_rows = 1;
            let galley = ui.fonts(|fonts| fonts.layout_job(job));
            ui.painter_at(label_rect).galley(
                label_rect.center() - galley.size() * 0.5,
                galley,
                self.theme.text,
            );
            if response.clicked() {
                self.active_tab = index;
                focus_editor = true;
            }
            if response.clicked_by(egui::PointerButton::Middle) {
                close_tab = Some(index);
            }
            response.context_menu(|ui| {
                if ui.button("Close tab").clicked() {
                    close_tab = Some(index);
                    ui.close_menu();
                }
            });
            if !compact {
                let close_rect = egui::Rect::from_min_max(
                    egui::pos2(label_rect.right(), bounds.top()),
                    bounds.max,
                );
                if icons::button(
                    ui,
                    close_rect,
                    ui.make_persistent_id(("close-tab", index)),
                    icons::Icon::Close,
                    "Close tab",
                    &self.theme,
                )
                .clicked()
                {
                    close_tab = Some(index);
                }
            }
        }
        if let Some(index) = close_tab {
            focus_editor |= index == self.active_tab;
            self.close_tab(index);
        }
        if focus_editor {
            if let Some(pane) = self.active_terminal_mut() {
                pane.editor.focus_requested = true;
            }
        }
    }

    fn render_workspace(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let rect = ui.available_rect_before_wrap();
        let mut panes = Vec::new();
        tab.tree.layout(ui, rect, &self.theme, &mut panes);
        for (id, rect) in panes {
            let Some(index) = tab.panes.iter().position(|pane| pane.id == id) else {
                continue;
            };
            if ui.input(|input| {
                input.pointer.any_pressed()
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|pos| rect.contains(pos))
            }) {
                tab.active_pane = index;
            }
            ui.scope_builder(egui::UiBuilder::new().id_salt(id).max_rect(rect), |ui| {
                ui.set_clip_rect(rect);
                ui.set_min_size(rect.size());
                tab.panes[index].render(
                    ui,
                    index == tab.active_pane
                        && !self.settings_open
                        && !self.find_open
                        && !self.command_palette_open,
                    &self.loaded.settings,
                    &self.theme,
                    &self.find_query,
                );
            });
        }
    }

    fn render_find(&mut self, ctx: &egui::Context) {
        egui::Window::new("Find")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, [-18.0, 82.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.text_edit_singleline(&mut self.find_query);
                    if !self.find_query.trim().is_empty() {
                        let matches = self
                            .tabs
                            .get(self.active_tab)
                            .map(|tab| tab.match_count(&self.find_query))
                            .unwrap_or(0);
                        ui.label(RichText::new(format!("{matches} matches")).small());
                    }
                    if ui.button("Clear").clicked() {
                        self.find_query.clear();
                    }
                    if ui.button("Close").clicked() {
                        self.find_open = false;
                    }
                });
            });
    }

    fn render_command_palette(&mut self, ctx: &egui::Context) {
        let window = egui::Window::new("Actions")
            .collapsible(false)
            .resizable(false)
            .fixed_size([420.0, 240.0])
            .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
            .show(ctx, |ui| {
                for (label, action) in [
                    ("New tab", PaletteAction::NewTab),
                    ("Reopen closed tab", PaletteAction::ReopenClosedTab),
                    ("Split right", PaletteAction::SplitRight),
                    ("Split down", PaletteAction::SplitDown),
                    ("Focus next pane", PaletteAction::FocusNextPane),
                    ("Focus previous pane", PaletteAction::FocusPreviousPane),
                    ("Find", PaletteAction::Find),
                    ("Copy selected block", PaletteAction::CopyBlock),
                    ("Settings", PaletteAction::Settings),
                    ("Refresh completions", PaletteAction::RefreshCompletions),
                    ("Dismiss completions", PaletteAction::DismissCompletions),
                ] {
                    if ui.button(label).clicked() {
                        match action {
                            PaletteAction::NewTab => self.new_tab(),
                            PaletteAction::ReopenClosedTab => self.reopen_closed_tab(),
                            PaletteAction::SplitRight => self.split_active(SplitDirection::Right),
                            PaletteAction::SplitDown => self.split_active(SplitDirection::Down),
                            PaletteAction::FocusNextPane => self.focus_relative_pane(1),
                            PaletteAction::FocusPreviousPane => self.focus_relative_pane(-1),
                            PaletteAction::Find => self.find_open = true,
                            PaletteAction::CopyBlock => {
                                if let Some(text) = self
                                    .active_terminal()
                                    .and_then(TerminalPane::focused_block_text)
                                {
                                    ctx.copy_text(text);
                                }
                            }
                            PaletteAction::Settings => self.settings_open = true,
                            PaletteAction::RefreshCompletions => {
                                if let Some(pane) = self.active_terminal_mut() {
                                    pane.refresh_completions();
                                }
                            }
                            PaletteAction::DismissCompletions => {
                                if let Some(pane) = self.active_terminal_mut() {
                                    pane.dismiss_completions();
                                }
                            }
                        }
                        self.command_palette_open = false;
                    }
                }
            });
        let clicked_outside = !self.actions_opened_this_frame
            && ctx.input(|input| input.pointer.primary_clicked())
            && ctx
                .input(|input| input.pointer.interact_pos())
                .is_some_and(|position| {
                    window
                        .as_ref()
                        .is_none_or(|window| !window.response.rect.contains(position))
                });
        if clicked_outside {
            self.command_palette_open = false;
        }
    }

    fn render_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([820.0, 620.0])
            .min_size([720.0, 480.0])
            .show(ctx, |ui| {
                let height = ui.available_height();
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(180.0, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.render_settings_navigation(ui),
                    );
                    ui.separator();
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| self.render_settings_category(ui));
                        },
                    );
                });
            });
        self.settings_open = open;
    }

    fn render_settings_navigation(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.add_space(8.0);
        for category in SettingsCategory::ALL {
            if ui
                .selectable_label(self.settings_category == category, category.label())
                .clicked()
            {
                self.settings_category = category;
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.label(
                RichText::new(compact_path(&self.loaded.path.display().to_string()))
                    .small()
                    .color(self.theme.muted),
            )
            .on_hover_text(self.loaded.path.display().to_string());
        });
    }

    fn render_settings_category(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());
        match self.settings_category {
            SettingsCategory::Appearance => settings_section(ui, "Appearance", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    egui::ComboBox::from_id_salt("theme-mode")
                        .selected_text(format!("{:?}", self.loaded.settings.appearance.mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.loaded.settings.appearance.mode,
                                AppearanceMode::System,
                                "System",
                            );
                            ui.selectable_value(
                                &mut self.loaded.settings.appearance.mode,
                                AppearanceMode::Dark,
                                "Dark",
                            );
                            ui.selectable_value(
                                &mut self.loaded.settings.appearance.mode,
                                AppearanceMode::Light,
                                "Light",
                            );
                        });
                });
                ui.checkbox(
                    &mut self.loaded.settings.appearance.minimum_contrast,
                    "Minimum contrast",
                );
                ui.checkbox(
                    &mut self.loaded.settings.appearance.zero_state_blocks,
                    "Show zero-state blocks",
                );
                ui.add(
                    egui::Slider::new(
                        &mut self.loaded.settings.appearance.window_opacity,
                        0.2..=1.0,
                    )
                    .text("Window opacity"),
                );
                egui::ComboBox::from_id_salt("spacing")
                    .selected_text(format!(
                        "{:?}",
                        self.loaded.settings.appearance.block_spacing
                    ))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.loaded.settings.appearance.block_spacing,
                            BlockSpacing::Normal,
                            "Normal",
                        );
                        ui.selectable_value(
                            &mut self.loaded.settings.appearance.block_spacing,
                            BlockSpacing::Compact,
                            "Compact",
                        );
                    });
            }),
            SettingsCategory::Fonts => settings_section(ui, "Fonts", |ui| {
                ui.label("Font family");
                ui.text_edit_singleline(&mut self.loaded.settings.font.family);
                ui.add(
                    egui::Slider::new(&mut self.loaded.settings.font.size, 8.0..=48.0)
                        .text("Terminal font size"),
                );
                ui.add(
                    egui::Slider::new(&mut self.loaded.settings.font.line_height, 0.8..=2.2)
                        .text("Line height"),
                );
                ui.checkbox(&mut self.loaded.settings.font.ligatures, "Ligatures");
            }),
            SettingsCategory::Terminal => settings_section(ui, "Terminal", |ui| {
                ui.checkbox(
                    &mut self.loaded.settings.terminal.audible_bell,
                    "Audible bell",
                );
                ui.add(
                    egui::Slider::new(
                        &mut self.loaded.settings.terminal.max_grid_rows,
                        100..=1_000_000,
                    )
                    .text("Max grid rows"),
                );
                ui.add(
                    egui::Slider::new(
                        &mut self.loaded.settings.terminal.alternate_screen_padding,
                        0..=48,
                    )
                    .text("Alternate-screen padding"),
                );
                egui::ComboBox::from_id_salt("clipboard-policy")
                    .selected_text(format!(
                        "{:?}",
                        self.loaded.settings.terminal.clipboard_escape_policy
                    ))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.loaded.settings.terminal.clipboard_escape_policy,
                            ClipboardEscapePolicy::Deny,
                            "Deny",
                        );
                        ui.selectable_value(
                            &mut self.loaded.settings.terminal.clipboard_escape_policy,
                            ClipboardEscapePolicy::WriteOnly,
                            "Write only",
                        );
                        ui.selectable_value(
                            &mut self.loaded.settings.terminal.clipboard_escape_policy,
                            ClipboardEscapePolicy::ReadWrite,
                            "Read/write",
                        );
                    });
            }),
            SettingsCategory::Input => settings_section(ui, "Input", |ui| {
                ui.checkbox(
                    &mut self.loaded.settings.input.left_alt_is_meta,
                    "Left Alt as Meta",
                );
                ui.checkbox(
                    &mut self.loaded.settings.input.right_alt_is_meta,
                    "Right Alt as Meta",
                );
                ui.checkbox(
                    &mut self.loaded.settings.input.vim_like_editing,
                    "Vim-like editing",
                );
            }),
            SettingsCategory::Keybindings => settings_section(ui, "Keybindings", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.text_edit_singleline(&mut self.keybinding_search);
                });
                ui.label(format!(
                    "{} active bindings, {} conflicts",
                    self.loaded.keybindings.bindings.len(),
                    self.loaded.keybindings.conflicts.len()
                ));
                let query = self.keybinding_search.to_ascii_lowercase();
                for (action, shortcuts) in &self.loaded.keybindings.bindings {
                    let label = action.label();
                    let shortcut_text = shortcuts.join(", ");
                    if query.is_empty()
                        || label.to_ascii_lowercase().contains(&query)
                        || shortcut_text.to_ascii_lowercase().contains(&query)
                    {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(RichText::new(shortcut_text).color(self.theme.muted));
                                },
                            );
                        });
                        ui.separator();
                    }
                }
            }),
            SettingsCategory::Privacy => settings_section(ui, "Privacy", |ui| {
                ui.label("Commands are redacted locally before logging or sharing.");
                for pattern in &mut self.loaded.settings.privacy.redaction_patterns {
                    ui.text_edit_singleline(pattern);
                }
                if ui.button("Add redaction pattern").clicked() {
                    self.loaded
                        .settings
                        .privacy
                        .redaction_patterns
                        .push(String::new());
                }
            }),
        }
        for issue in &self.loaded.issues {
            ui.colored_label(
                self.theme.warning,
                format!("{}: {}", issue.path, issue.message),
            );
        }
        ui.add_space(12.0);
        if ui.button("Save settings").clicked() {
            match save_settings(&self.loaded.path, &self.loaded.settings) {
                Ok(()) => self.toast("Settings saved".to_owned()),
                Err(error) => self.toast(format!("Settings save failed: {error}")),
            }
        }
    }

    fn render_toast(&mut self, ctx: &egui::Context) {
        let Some((message, created_at)) = &self.toast else {
            return;
        };
        if created_at.elapsed() > Duration::from_secs(4) {
            self.toast = None;
            return;
        }
        egui::Area::new("toast".into())
            .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -18.0])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.theme.surface_high)
                    .stroke(Stroke::new(1.0_f32, self.theme.border))
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.label(message);
                    });
            });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteAction {
    NewTab,
    ReopenClosedTab,
    SplitRight,
    SplitDown,
    FocusNextPane,
    FocusPreviousPane,
    Find,
    CopyBlock,
    Settings,
    RefreshCompletions,
    DismissCompletions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsCategory {
    Appearance,
    Fonts,
    Terminal,
    Input,
    Keybindings,
    Privacy,
}

impl SettingsCategory {
    const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Fonts,
        Self::Terminal,
        Self::Input,
        Self::Keybindings,
        Self::Privacy,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Fonts => "Fonts",
            Self::Terminal => "Terminal",
            Self::Input => "Input",
            Self::Keybindings => "Keybindings",
            Self::Privacy => "Privacy",
        }
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

    fn match_count(&self, query: &str) -> usize {
        self.panes.iter().map(|pane| pane.match_count(query)).sum()
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
    cwd: PathBuf,
    aliases: Vec<(String, String)>,
    title: String,
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
    hovered_block: Option<Uuid>,
    scroll_focused_block: bool,
    layout: TerminalPaneLayoutSnapshot,
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

    fn clear_if_missing(&mut self, targets: &[TerminalBlockFocus]) {
        if self
            .focused
            .is_some_and(|focused| !targets.contains(&focused))
        {
            self.focused = None;
        }
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
        let next_index = if self.focused.is_some() {
            current.saturating_add_signed(delta)
        } else {
            current
        };
        if next_index >= targets.len() {
            self.focused = None;
            return None;
        }
        let next = targets[next_index];
        self.focused = Some(next);
        Some(next)
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
    cwd: PathBuf,
    started_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandEditorAction {
    Submit,
    Complete,
    CompletionNext,
    CompletionPrevious,
    DismissCompletions,
}

#[derive(Debug, Clone, Default)]
struct CommandEditorOutput {
    changed: bool,
    action: Option<CommandEditorAction>,
}

#[derive(Debug, Clone, Default)]
struct CommandEditorState {
    text: String,
    cursor: usize,
    selection_anchor: Option<usize>,
    drag_anchor: Option<usize>,
    scroll_y: f32,
    preedit: String,
    shell_bridge: bool,
    shell_command_mode: bool,
    alt_is_meta: bool,
    shell_events: Vec<egui::Event>,
    focus_requested: bool,
    completion_open: bool,
    history_active: bool,
}

impl CommandEditorState {
    fn new() -> Self {
        Self::default()
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.selection_anchor = None;
        self.clamp_cursor();
    }

    fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.selection_anchor = None;
        self.drag_anchor = None;
        self.scroll_y = 0.0;
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn has_selection(&self) -> bool {
        self.selection_range().is_some()
    }

    fn selection_range(&self) -> Option<std::ops::Range<usize>> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor {
            return None;
        }
        Some(anchor.min(self.cursor)..anchor.max(self.cursor))
    }

    fn selected_text(&self) -> Option<&str> {
        let range = self.selection_range()?;
        Some(&self.text[range])
    }

    fn select_all(&mut self) {
        self.cursor = self.text.len();
        self.selection_anchor = Some(0);
    }

    fn insert_text(&mut self, value: &str) {
        if value.is_empty() {
            return;
        }
        self.delete_selection();
        self.text.insert_str(self.cursor, value);
        self.cursor += value.len();
        self.clamp_cursor();
    }

    fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let previous = previous_char_boundary(&self.text, self.cursor);
        if previous == self.cursor {
            return false;
        }
        self.text.drain(previous..self.cursor);
        self.cursor = previous;
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let next = next_char_boundary(&self.text, self.cursor);
        if next == self.cursor {
            return false;
        }
        self.text.drain(self.cursor..next);
        true
    }

    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection_range() else {
            self.selection_anchor = None;
            return false;
        };
        self.text.drain(range.clone());
        self.cursor = range.start;
        self.selection_anchor = None;
        true
    }

    fn move_left(&mut self, selecting: bool) {
        let target = if !selecting {
            self.selection_range()
                .map(|range| range.start)
                .unwrap_or_else(|| previous_char_boundary(&self.text, self.cursor))
        } else {
            previous_char_boundary(&self.text, self.cursor)
        };
        self.move_cursor(target, selecting);
    }

    fn move_right(&mut self, selecting: bool) {
        let target = if !selecting {
            self.selection_range()
                .map(|range| range.end)
                .unwrap_or_else(|| next_char_boundary(&self.text, self.cursor))
        } else {
            next_char_boundary(&self.text, self.cursor)
        };
        self.move_cursor(target, selecting);
    }

    fn move_home(&mut self, selecting: bool) {
        self.move_cursor(self.current_line_bounds().start, selecting);
    }

    fn move_end(&mut self, selecting: bool) {
        self.move_cursor(self.current_line_bounds().end, selecting);
    }

    fn move_to_start(&mut self, selecting: bool) {
        self.move_cursor(0, selecting);
    }

    fn move_to_end(&mut self, selecting: bool) {
        self.move_cursor(self.text.len(), selecting);
    }

    fn set_cursor_from_mouse_index(&mut self, index: usize, selecting: bool) {
        self.move_cursor(self.clamp_index(index), selecting);
    }

    fn select_word_at(&mut self, index: usize) {
        let index = self.clamp_index(index);
        let (start, end) = word_bounds(&self.text, index);
        self.selection_anchor = Some(start);
        self.cursor = end;
    }

    fn select_line_at(&mut self, index: usize) {
        let index = self.clamp_index(index);
        let start = self.text[..index]
            .rfind('\n')
            .map_or(0, |position| position + 1);
        let end = self.text[index..]
            .find('\n')
            .map_or(self.text.len(), |offset| index + offset + 1);
        self.selection_anchor = Some(start);
        self.cursor = end;
    }

    #[cfg(test)]
    fn index_from_monospace_point(
        &self,
        point: egui::Pos2,
        origin: egui::Pos2,
        char_width: f32,
        row_height: f32,
    ) -> usize {
        let line = ((point.y - origin.y) / row_height).floor().max(0.0) as usize;
        let column = ((point.x - origin.x) / char_width).round().max(0.0) as usize;
        self.index_from_line_column(line, column)
    }

    #[cfg(test)]
    fn index_from_line_column(&self, line: usize, column: usize) -> usize {
        let mut line_start = 0;
        let mut current_line = 0;
        for (index, character) in self.text.char_indices() {
            if current_line == line {
                break;
            }
            if character == '\n' {
                current_line += 1;
                line_start = index + character.len_utf8();
            }
        }
        if current_line < line {
            return self.text.len();
        }
        let line_end = self.text[line_start..]
            .find('\n')
            .map(|offset| line_start + offset)
            .unwrap_or(self.text.len());
        self.text[line_start..line_end]
            .char_indices()
            .nth(column)
            .map(|(offset, _)| line_start + offset)
            .unwrap_or(line_end)
    }

    fn move_cursor(&mut self, target: usize, selecting: bool) {
        let target = self.clamp_index(target);
        if selecting {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }
        self.cursor = target;
    }

    fn move_visual_row(&mut self, galley: &egui::Galley, delta: isize, selecting: bool) -> bool {
        let character = self.text[..self.cursor].chars().count();
        let current = galley.from_ccursor(egui::text::CCursor::new(character));
        let target_row = current.rcursor.row.saturating_add_signed(delta);
        if target_row == current.rcursor.row || target_row >= galley.rows.len() {
            return false;
        }
        let x = galley.rows[current.rcursor.row].x_offset(current.rcursor.column);
        let column = galley.rows[target_row].char_at(x);
        let target_character = galley.rows[..target_row]
            .iter()
            .map(egui::epaint::text::Row::char_count_including_newline)
            .sum::<usize>()
            + column;
        self.move_cursor(char_to_byte_index(&self.text, target_character), selecting);
        true
    }

    fn current_line_bounds(&self) -> std::ops::Range<usize> {
        let start = self.text[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let end = self.text[self.cursor..]
            .find('\n')
            .map(|offset| self.cursor + offset)
            .unwrap_or(self.text.len());
        start..end
    }

    fn clamp_cursor(&mut self) {
        self.cursor = self.clamp_index(self.cursor);
        self.selection_anchor = self.selection_anchor.map(|anchor| self.clamp_index(anchor));
    }

    fn clamp_index(&self, index: usize) -> usize {
        if index >= self.text.len() {
            return self.text.len();
        }
        if self.text.is_char_boundary(index) {
            return index;
        }
        previous_char_boundary(&self.text, index)
    }

    fn show(
        &mut self,
        ui: &mut egui::Ui,
        id_source: impl std::hash::Hash,
        font: FontId,
        theme: &TerminasteTheme,
        desired_height: f32,
        active: bool,
    ) -> (egui::Response, CommandEditorOutput) {
        let row_height = ui.fonts(|fonts| fonts.row_height(&font));
        let width = ui.available_width().max(1.0);
        let wrap_width = (width - 2.0).max(1.0);
        let mut galley =
            ui.painter()
                .layout(self.text.clone(), font.clone(), theme.text, wrap_width);
        let (_, rect) = ui.allocate_space(egui::vec2(width, desired_height));
        let id = ui.make_persistent_id(id_source);
        let mut response = ui.interact(rect, id, Sense::click_and_drag());
        let content_rect = rect.shrink2(egui::vec2(0.0, shell_prompt::EDITOR_PADDING_Y));
        let text_rect = content_rect;
        let maximum_scroll = (galley.size().y - text_rect.height()).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, maximum_scroll);
        if response.hovered() {
            ui.output_mut(|output| output.mutable_text_under_cursor = true);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            let wheel = ui.input(|input| input.smooth_scroll_delta.y);
            if maximum_scroll > 0.0 && wheel != 0.0 {
                self.scroll_y = (self.scroll_y - wheel).clamp(0.0, maximum_scroll);
            }
        }
        let mut text_origin = text_rect.min - egui::vec2(0.0, self.scroll_y);
        let mut output = CommandEditorOutput::default();

        if active && (self.focus_requested || response.clicked() || response.drag_started()) {
            response.request_focus();
            self.focus_requested = false;
        }

        let pointer_index = |position: egui::Pos2| {
            char_to_byte_index(
                &self.text,
                galley.cursor_from_pos(position - text_origin).ccursor.index,
            )
        };

        if response.triple_clicked() {
            if let Some(position) = response.interact_pointer_pos() {
                self.select_line_at(pointer_index(position));
            }
        } else if response.double_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                self.select_word_at(pointer_index(pos));
            }
        } else if response.drag_started() {
            if let Some(pos) = ui
                .input(|input| input.pointer.press_origin())
                .or_else(|| response.interact_pointer_pos())
            {
                let index = pointer_index(pos);
                if ui.input(|input| input.modifiers.shift) {
                    let anchor = self.selection_anchor.unwrap_or(self.cursor);
                    self.drag_anchor = Some(anchor);
                    self.selection_anchor = Some(anchor);
                    self.cursor = index;
                } else {
                    self.drag_anchor = Some(index);
                    self.selection_anchor = Some(index);
                    self.cursor = index;
                }
            }
        } else if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let selecting = ui.input(|input| input.modifiers.shift);
                let index = pointer_index(pos);
                self.set_cursor_from_mouse_index(index, selecting);
            }
        }

        if response.dragged() {
            if let (Some(anchor), Some(pos)) = (self.drag_anchor, ui.ctx().pointer_interact_pos()) {
                if pos.y < text_rect.top() {
                    self.scroll_y = (self.scroll_y - row_height * 0.6).max(0.0);
                } else if pos.y > text_rect.bottom() {
                    self.scroll_y = (self.scroll_y + row_height * 0.6).min(maximum_scroll);
                }
                text_origin = text_rect.min - egui::vec2(0.0, self.scroll_y);
                let index = char_to_byte_index(
                    &self.text,
                    galley.cursor_from_pos(pos - text_origin).ccursor.index,
                );
                self.selection_anchor = Some(anchor);
                self.cursor = index;
                ui.ctx().request_repaint();
            }
        }

        if response.drag_stopped() {
            self.drag_anchor = None;
        }

        if active && response.has_focus() {
            output.changed |= self.handle_keyboard(ui, &galley, &mut output.action);
        }

        if output.changed {
            galley = ui
                .painter()
                .layout(self.text.clone(), font.clone(), theme.text, wrap_width);
        }
        let maximum_scroll = (galley.size().y - text_rect.height()).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, maximum_scroll);

        let cursor_character = self.text[..self.cursor].chars().count();
        let cursor_rect = galley.pos_from_ccursor(egui::text::CCursor::new(cursor_character));
        let cursor_top = cursor_rect.top() - self.scroll_y;
        let cursor_bottom = cursor_rect.bottom() - self.scroll_y;
        if cursor_top < 0.0 {
            self.scroll_y = cursor_rect.top().clamp(0.0, maximum_scroll);
        } else if cursor_bottom > text_rect.height() {
            self.scroll_y = (cursor_rect.bottom() - text_rect.height()).clamp(0.0, maximum_scroll);
        }
        text_origin = text_rect.min - egui::vec2(0.0, self.scroll_y);

        if output.changed {
            response.mark_changed();
        }

        let painter = ui.painter_at(text_rect);
        self.paint_selection(&painter, &galley, text_origin, theme.accent);
        if !self.text.is_empty() {
            painter.galley(text_origin, galley.clone(), theme.text);
        }
        if response.has_focus() {
            self.paint_cursor(&painter, &galley, text_origin, row_height, theme.text);
            let cursor = galley
                .pos_from_ccursor(egui::text::CCursor::new(
                    self.text[..self.cursor].chars().count(),
                ))
                .translate(text_origin.to_vec2());
            painter.text(
                cursor.min,
                egui::Align2::LEFT_TOP,
                &self.preedit,
                font,
                theme.accent,
            );
            ui.output_mut(|output| {
                output.ime = Some(egui::output::IMEOutput {
                    rect,
                    cursor_rect: cursor,
                })
            });
        }

        if maximum_scroll > 0.0 {
            let track = egui::Rect::from_min_max(
                egui::pos2(content_rect.right() - 3.0, content_rect.top()),
                content_rect.right_bottom(),
            );
            let thumb_height = (text_rect.height() / galley.size().y * track.height()).max(18.0);
            let thumb_top =
                track.top() + (self.scroll_y / maximum_scroll) * (track.height() - thumb_height);
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(track.left(), thumb_top),
                    egui::vec2(track.width(), thumb_height),
                ),
                2.0,
                theme.muted.linear_multiply(0.5),
            );
        }

        ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, event_filter_all()));
        (response, output)
    }

    fn handle_keyboard(
        &mut self,
        ui: &mut egui::Ui,
        galley: &egui::Galley,
        action: &mut Option<CommandEditorAction>,
    ) -> bool {
        let mut changed = false;
        let events = ui.input(|input| input.filtered_events(&event_filter_all()));
        for event in events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = &event
            {
                if self.preedit.is_empty()
                    && !modifiers.ctrl
                    && !modifiers.alt
                    && !modifiers.mac_cmd
                {
                    if self.completion_open {
                        *action = match key {
                            Key::ArrowLeft | Key::ArrowRight => {
                                Some(CommandEditorAction::DismissCompletions)
                            }
                            Key::ArrowDown => Some(CommandEditorAction::CompletionNext),
                            Key::ArrowUp => Some(CommandEditorAction::CompletionPrevious),
                            Key::Tab if modifiers.shift => {
                                Some(CommandEditorAction::CompletionPrevious)
                            }
                            Key::Tab | Key::Enter => Some(CommandEditorAction::Complete),
                            Key::Escape => Some(CommandEditorAction::DismissCompletions),
                            _ => None,
                        };
                        if action.is_some() {
                            break;
                        }
                    } else if *key == Key::Tab {
                        *action = Some(CommandEditorAction::Complete);
                        break;
                    }
                }
            }
            if self.shell_bridge && self.preedit.is_empty() && !self.has_selection() {
                let forward = match &event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        !modifiers.mac_cmd
                            && (modifiers.ctrl
                                || (modifiers.alt && self.alt_is_meta)
                                || matches!(
                                    key,
                                    Key::Tab
                                        | Key::Escape
                                        | Key::F1
                                        | Key::F2
                                        | Key::F3
                                        | Key::F4
                                        | Key::F5
                                        | Key::F6
                                        | Key::F7
                                        | Key::F8
                                        | Key::F9
                                        | Key::F10
                                        | Key::F11
                                        | Key::F12
                                ))
                    }
                    egui::Event::Text(_) => {
                        self.shell_command_mode
                            || ui.input(|input| input.modifiers.alt && self.alt_is_meta)
                    }
                    _ => false,
                };
                if forward {
                    self.shell_events.push(event);
                    break;
                }
            }
            match event {
                egui::Event::Copy => {
                    if let Some(selected) = self.selected_text() {
                        ui.ctx().copy_text(selected.to_owned());
                    }
                }
                egui::Event::Cut => {
                    if let Some(selected) = self.selected_text() {
                        ui.ctx().copy_text(selected.to_owned());
                        changed |= self.delete_selection();
                    }
                }
                egui::Event::Paste(value) => {
                    self.insert_text(&value);
                    changed = true;
                }
                egui::Event::Text(value) => {
                    if !value.is_empty() && value != "\n" && value != "\r" {
                        self.insert_text(&value);
                        changed = true;
                    }
                }
                egui::Event::Ime(egui::ImeEvent::Commit(value)) => {
                    self.preedit.clear();
                    if self.shell_bridge && self.shell_command_mode && !self.has_selection() {
                        self.shell_events
                            .push(egui::Event::Ime(egui::ImeEvent::Commit(value)));
                    } else {
                        self.insert_text(&value);
                        changed = true;
                    }
                }
                egui::Event::Ime(egui::ImeEvent::Preedit(value)) => self.preedit = value,
                egui::Event::Ime(egui::ImeEvent::Disabled) => self.preedit.clear(),
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if self.preedit.is_empty() => {
                    changed |= self.handle_key(key, modifiers, galley, action);
                    if action.is_some() {
                        break;
                    }
                }
                _ => {}
            }
        }
        changed
    }

    fn handle_key(
        &mut self,
        key: Key,
        modifiers: egui::Modifiers,
        galley: &egui::Galley,
        action: &mut Option<CommandEditorAction>,
    ) -> bool {
        if modifiers.mac_cmd && key == Key::A {
            self.select_all();
            return false;
        }
        if modifiers.command && key == Key::ArrowLeft {
            self.move_to_start(modifiers.shift);
            return false;
        }
        if modifiers.command && key == Key::ArrowRight {
            self.move_to_end(modifiers.shift);
            return false;
        }

        match key {
            Key::Backspace => self.backspace(),
            Key::Delete => self.delete_forward(),
            Key::ArrowLeft => {
                self.move_left(modifiers.shift);
                false
            }
            Key::ArrowRight => {
                self.move_right(modifiers.shift);
                false
            }
            Key::ArrowUp => {
                if self.has_selection() || modifiers.shift {
                    self.move_visual_row(galley, -1, modifiers.shift);
                } else if self.history_active
                    || !self.text.contains('\n')
                    || !self.move_visual_row(galley, -1, false)
                {
                    *action = Some(CommandEditorAction::CompletionPrevious);
                }
                false
            }
            Key::ArrowDown => {
                if self.has_selection() || modifiers.shift {
                    self.move_visual_row(galley, 1, modifiers.shift);
                } else if self.history_active || !self.text.contains('\n') {
                    *action = Some(CommandEditorAction::CompletionNext);
                } else {
                    self.move_visual_row(galley, 1, false);
                }
                false
            }
            Key::Home => {
                self.move_home(modifiers.shift);
                false
            }
            Key::End => {
                self.move_end(modifiers.shift);
                false
            }
            Key::Enter => {
                if modifiers.shift {
                    self.insert_text("\n");
                    true
                } else {
                    *action = Some(CommandEditorAction::Submit);
                    false
                }
            }
            Key::Tab => {
                *action = Some(CommandEditorAction::Complete);
                false
            }
            Key::Escape => {
                *action = Some(CommandEditorAction::DismissCompletions);
                false
            }
            _ => false,
        }
    }

    fn paint_selection(
        &self,
        painter: &egui::Painter,
        galley: &egui::text::Galley,
        origin: egui::Pos2,
        color: Color32,
    ) {
        let Some(selection) = self.selection_range() else {
            return;
        };
        let start = galley
            .from_ccursor(egui::text::CCursor::new(
                self.text[..selection.start].chars().count(),
            ))
            .rcursor;
        let end = galley
            .from_ccursor(egui::text::CCursor::new(
                self.text[..selection.end].chars().count(),
            ))
            .rcursor;
        for row_index in start.row..=end.row {
            let row = &galley.rows[row_index];
            let left = if row_index == start.row {
                row.x_offset(start.column)
            } else {
                row.rect.left()
            };
            let right = if row_index == end.row {
                row.x_offset(end.column)
            } else {
                row.rect.right()
                    + if row.ends_with_newline {
                        row.height() / 2.0
                    } else {
                        0.0
                    }
            };
            let rect = egui::Rect::from_min_max(
                origin + egui::vec2(left, row.rect.top()),
                origin + egui::vec2(right.max(left + 2.0), row.rect.bottom()),
            );
            painter.rect_filled(rect, 2.0, color.linear_multiply(0.3));
        }
    }

    fn paint_cursor(
        &self,
        painter: &egui::Painter,
        galley: &egui::text::Galley,
        origin: egui::Pos2,
        row_height: f32,
        color: Color32,
    ) {
        let char_index = self.text[..self.cursor].chars().count();
        let cursor_rect = galley.pos_from_ccursor(egui::text::CCursor::new(char_index));
        let min = origin + cursor_rect.min.to_vec2();
        let max = egui::pos2(min.x, min.y + cursor_rect.height().max(row_height));
        painter.line_segment([min, max], Stroke::new(1.5_f32, color));
    }
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
            cwd,
            aliases: Vec::new(),
            title: "local".to_owned(),
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
            hovered_block: None,
            scroll_focused_block: false,
            layout: default_pane_layout_snapshot(),
            surface: terminal_surface::SurfaceState::default(),
        }
    }

    fn fake() -> Self {
        let (pty, events, commands) = PtySession::fake();
        Self {
            id: Uuid::new_v4(),
            model: TerminalModel::new(80, 24, 1000),
            pty: Some(pty),
            fake_pty_channels: Some(FakePtyChannels { events, commands }),
            editor: CommandEditorState::new(),
            editor_has_focus: false,
            completion_revision: 0,
            completions: Vec::new(),
            selected_completion: 0,
            completion_navigating: false,
            history_offset: 0,
            history_search: None,
            cwd: PathBuf::from("."),
            aliases: Vec::new(),
            title: "test".to_owned(),
            cols: 80,
            rows: 24,
            status: "fake".to_owned(),
            active_command: None,
            last_command_summary: None,
            last_editor_cursor: 0,
            suppress_prompt_output: false,
            prompt_capture: Vec::new(),
            input_prompt_text: None,
            integration_ready: false,
            block_focus: BlockFocusState::default(),
            hovered_block: None,
            scroll_focused_block: false,
            layout: default_pane_layout_snapshot(),
            surface: terminal_surface::SurfaceState::default(),
        }
    }

    fn drain_pty_events(&mut self, ctx: &egui::Context) {
        if let Some(pty) = &self.pty {
            pty.set_output_wakeup({
                let ctx = ctx.clone();
                move || ctx.request_repaint()
            });
        }
        let Some(rx) = self.pty.as_ref().map(|pty| pty.rx.clone()) else {
            return;
        };
        let deadline = Instant::now() + Duration::from_millis(4);
        loop {
            match rx.try_recv() {
                Ok(PtyEvent::Output(bytes)) => {
                    self.process_ordered_pty_bytes(&bytes);
                    ctx.request_repaint();
                }
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
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    break;
                }
            }
            if Instant::now() >= deadline {
                ctx.request_repaint();
                break;
            }
        }
    }

    fn render(
        &mut self,
        ui: &mut egui::Ui,
        active: bool,
        settings: &Settings,
        theme: &TerminasteTheme,
        find_query: &str,
    ) {
        self.render_surface(ui, active, settings, theme, find_query);
    }

    fn render_composer(
        &mut self,
        ui: &mut egui::Ui,
        active: bool,
        settings: &Settings,
        theme: &TerminasteTheme,
        find_query: &str,
    ) {
        self.resize_from_ui(ui, settings);
        let rect = ui.available_rect_before_wrap();
        let font = FontId::monospace(settings.font.size);
        let row_height = ui.fonts(|fonts| fonts.row_height(&font));
        let prompt = shell_prompt::PromptLayout::new(self, ui, font.clone(), theme);
        let text_height = ui
            .painter()
            .layout(
                self.editor.text().to_owned(),
                font,
                theme.text,
                (prompt.editor_width(rect.width()) - 2.0).max(1.0),
            )
            .size()
            .y;
        let editor_height = text_height
            .clamp(row_height, row_height * 6.0)
            .max(prompt.input_height);
        let running = self.model.has_running_command();
        let input_height = if running {
            0.0
        } else {
            (editor_height
                + prompt.header_height
                + shell_prompt::INSET * 2.0
                + shell_prompt::EDITOR_PADDING_Y * 2.0)
                .min(rect.height())
        };
        let input_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - input_height),
            rect.max,
        );
        let history_bottom = if running {
            rect.bottom()
        } else {
            (input_rect.top() - command_block_spacing(settings).1).max(rect.top())
        };
        let history_rect =
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), history_bottom));
        self.layout = default_pane_layout_snapshot();
        let mut history_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("history")
                .max_rect(history_rect),
        );
        history_ui.set_clip_rect(history_rect.intersect(ui.clip_rect()));
        self.render_terminal_blocks(&mut history_ui, settings, theme, find_query);
        let mut input_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("composer")
                .max_rect(input_rect),
        );
        input_ui.set_clip_rect(input_rect.intersect(ui.clip_rect()));
        self.render_input(
            &mut input_ui,
            settings,
            theme,
            active,
            input_height,
            &prompt,
        );
        ui.advance_cursor_after_rect(rect);
    }

    fn resize_from_ui(&mut self, ui: &egui::Ui, settings: &Settings) {
        let width = (ui.available_width() - shell_prompt::INSET * 2.0).max(1.0);
        let height = (ui.available_height() - input_row_height(settings)).max(180.0);
        let font = FontId::monospace(settings.font.size);
        let pixels_per_point = ui.ctx().pixels_per_point();
        let (cell_width, cell_height) = ui.fonts(|fonts| {
            (
                (fonts.glyph_width(&font, 'M') * pixels_per_point).ceil() / pixels_per_point,
                fonts
                    .row_height(&font)
                    .max(settings.font.size * settings.font.line_height),
            )
        });
        let cols = (width / cell_width).floor().clamp(2.0, 400.0) as u16;
        let rows = (height / cell_height).floor().clamp(5.0, 200.0) as u16;
        if cols != self.cols || rows != self.rows {
            self.cols = cols;
            self.rows = rows;
            self.model.resize(usize::from(cols), usize::from(rows));
            if let Some(pty) = &self.pty {
                let _ = pty.tx.send(PtyCommand::Resize { cols, rows });
            }
            if self.surface.input_bridge && !self.surface.submit_pending {
                self.write_terminal(self.bridge_key(98));
            }
        }
    }

    fn render_terminal_blocks(
        &mut self,
        ui: &mut egui::Ui,
        settings: &Settings,
        theme: &TerminasteTheme,
        find_query: &str,
    ) {
        let snapshot = self.model.snapshot();
        let font = FontId::monospace(settings.font.size);
        let (block_margin, block_gap) = command_block_spacing(settings);
        let focus_targets = focusable_targets_for_blocks(&snapshot.blocks, find_query);
        self.block_focus.clear_if_missing(&focus_targets);
        self.handle_block_keyboard_navigation(ui, &focus_targets);
        let viewport_height = ui.available_height();
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt((self.id, "blocks"))
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height(viewport_height);
        if self.surface.history_height <= viewport_height {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        let clicked_background = scroll
            .show(ui, |ui| {
                let background = ui.interact(
                    ui.clip_rect(),
                    ui.id().with("block-background"),
                    Sense::click(),
                );
                if self.surface.history_height > 0.0 {
                    ui.add_space((viewport_height - self.surface.history_height).max(0.0));
                }
                let content = ui.with_layout(
                    egui::Layout::top_down(egui::Align::Min).with_cross_justify(true),
                    |ui| {
                        let content_spacing = ui.spacing().item_spacing.y;
                        ui.spacing_mut().item_spacing.y = block_gap;
                        for block in &snapshot.blocks {
                            ui.push_id(block.id, |ui| {
                                if block_matches(&block_context_text(block), find_query) {
                                    let target = TerminalBlockFocus { block_id: block.id };
                                    let selected = self.block_focus.focused() == Some(target);
                                    let show_actions =
                                        selected || self.hovered_block == Some(block.id);
                                    let response = egui::Frame::new()
                                        .fill(if selected {
                                            theme.surface_high
                                        } else {
                                            theme.card
                                        })
                                        .stroke(Stroke::new(
                                            if selected { 1.5_f32 } else { 1.0_f32 },
                                            if selected { theme.accent } else { theme.border },
                                        ))
                                        .corner_radius(8.0)
                                        .inner_margin(egui::Margin::same(block_margin))
                                        .show(ui, |ui| {
                                            ui.spacing_mut().item_spacing.y =
                                                content_spacing.min(2.0);
                                            self.render_command_input_block(
                                                ui,
                                                block,
                                                theme,
                                                font.clone(),
                                                find_query,
                                                show_actions,
                                            );
                                            if !block.output.trim().is_empty() {
                                                thin_separator(ui, theme.border);
                                                self.render_command_output_block(
                                                    ui,
                                                    block,
                                                    theme,
                                                    font.clone(),
                                                    find_query,
                                                );
                                            } else if block.running {
                                                thin_separator(ui, theme.border);
                                                ui.add_space(ui.spacing().interact_size.y + 8.0);
                                            }
                                        })
                                        .response;
                                    if ui.rect_contains_pointer(response.rect)
                                        && ui.input(|input| {
                                            input.pointer.any_click()
                                                && input
                                                    .pointer
                                                    .interact_pos()
                                                    .is_some_and(|pos| response.rect.contains(pos))
                                        })
                                    {
                                        self.block_focus.focus(target);
                                        self.editor_has_focus = block.running;
                                        self.editor.focus_requested = block.running;
                                        if !block.running {
                                            ui.memory_mut(|memory| {
                                                if let Some(id) = memory.focused() {
                                                    memory.surrender_focus(id);
                                                }
                                            });
                                        }
                                    }
                                    if selected && self.scroll_focused_block {
                                        response.scroll_to_me(Some(egui::Align::Center));
                                        self.scroll_focused_block = false;
                                    }
                                    if ui.rect_contains_pointer(response.rect) {
                                        self.hovered_block = Some(block.id);
                                    } else if self.hovered_block == Some(block.id) {
                                        self.hovered_block = None;
                                    }
                                }
                            });
                        }
                    },
                );
                let height = content.response.rect.height();
                if (height - self.surface.history_height).abs() > 0.5 {
                    self.surface.history_height = height;
                    ui.ctx().request_discard("command history height changed");
                }
                background.clicked()
            })
            .inner;
        if clicked_background && self.model.has_running_command() {
            self.focus_input();
        }
    }

    fn render_command_input_block(
        &mut self,
        ui: &mut egui::Ui,
        block: &CommandBlock,
        theme: &TerminasteTheme,
        font: FontId,
        find_query: &str,
        show_actions: bool,
    ) {
        let cwd = self
            .active_command
            .as_ref()
            .filter(|active| block.running && active.command == block.command)
            .map(|active| active.cwd.display().to_string())
            .unwrap_or_else(|| self.cwd.display().to_string());
        let target = TerminalBlockFocus { block_id: block.id };
        let height = ui.fonts(|fonts| fonts.row_height(&font)).max(20.0);
        let response = ui
            .allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_height(height);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if show_actions {
                            let menu = egui::menu::menu_custom_button(
                                ui,
                                egui::Button::new("")
                                    .min_size(egui::vec2(20.0, 20.0))
                                    .frame(false),
                                |ui| render_block_actions(ui, block),
                            )
                            .response
                            .on_hover_text("More actions");
                            let center = menu.rect.center();
                            for offset in [-5.0, 0.0, 5.0] {
                                ui.painter().circle_filled(
                                    center + egui::vec2(0.0, offset),
                                    1.5,
                                    theme.muted,
                                );
                            }
                        }
                        if block.running {
                            ui.add(egui::Spinner::new().size(12.0))
                                .on_hover_text("Running");
                        } else if let Some(code) = block.exit_code {
                            ui.label(RichText::new(code.to_string()).small().color(if code == 0 {
                                theme.success
                            } else {
                                theme.error
                            }))
                            .on_hover_text(format!("Exit code {code}"));
                        }
                        if let Some(duration) =
                            block.duration_ms.filter(|_| ui.available_width() > 220.0)
                        {
                            ui.label(
                                RichText::new(format_duration(duration))
                                    .small()
                                    .color(theme.muted),
                            );
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.label(RichText::new(">").color(theme.muted).font(font.clone()));
                            let command = ui
                                .add(
                                    egui::Label::new(highlighted_text(
                                        &block.command,
                                        find_query,
                                        font.clone(),
                                        theme,
                                    ))
                                    .truncate(),
                                )
                                .on_hover_text(format!("{cwd}\n{}", block.command));
                            command.context_menu(|ui| render_block_actions(ui, block));
                        });
                    });
                },
            )
            .response;
        if ui.rect_contains_pointer(response.rect)
            && ui.input(|input| {
                input.pointer.any_click()
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|pos| response.rect.contains(pos))
            })
        {
            self.block_focus.focus(target);
            self.editor_has_focus = false;
            self.editor.focus_requested = false;
            ui.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        if ui.rect_contains_pointer(response.rect) {
            self.hovered_block = Some(block.id);
        }
    }

    fn render_command_output_block(
        &mut self,
        ui: &mut egui::Ui,
        block: &CommandBlock,
        theme: &TerminasteTheme,
        font: FontId,
        find_query: &str,
    ) {
        let target = TerminalBlockFocus { block_id: block.id };
        let response = render_ansi_output(ui, &block.output, find_query, font, theme);
        if response.clicked() || response.secondary_clicked() {
            response.request_focus();
            self.block_focus.focus(target);
            self.editor_has_focus = false;
            self.editor.focus_requested = false;
        }
        response.context_menu(|ui| render_block_actions(ui, block));
    }

    fn handle_block_keyboard_navigation(&mut self, ui: &egui::Ui, targets: &[TerminalBlockFocus]) {
        if self.model.has_running_command()
            && self.block_focus.focused().is_some()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            self.focus_input();
            return;
        }
        if targets.is_empty() {
            return;
        }
        if self.editor_has_focus {
            return;
        }
        let navigate_up = ui.input(|input| input.key_pressed(Key::ArrowUp));
        let navigate_down = ui.input(|input| input.key_pressed(Key::ArrowDown));
        if navigate_up {
            self.block_focus.navigate(targets, -1);
        }
        if navigate_down && self.block_focus.navigate(targets, 1).is_none() {
            self.focus_input();
        }
    }

    fn render_input(
        &mut self,
        ui: &mut egui::Ui,
        settings: &Settings,
        theme: &TerminasteTheme,
        active: bool,
        height: f32,
        prompt: &shell_prompt::PromptLayout,
    ) {
        if self.model.has_running_command() {
            let response =
                ui.allocate_response(egui::vec2(ui.available_width(), height), Sense::click());
            let accepts_input = self.block_focus.focused().is_none()
                || self.block_focus.focused() == self.running_block_target();
            if active
                && accepts_input
                && (self.editor.focus_requested
                    || response.clicked()
                    || (self.block_focus.focused().is_none()
                        && ui.memory(|memory| memory.focused().is_none())))
            {
                response.request_focus();
                self.editor.focus_requested = false;
            }
            self.editor_has_focus = active && accepts_input && response.has_focus();
            if self.editor_has_focus {
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(response.id, event_filter_all())
                });
                if self.active_command.is_some() {
                    let events = ui.input(|input| input.filtered_events(&event_filter_all()));
                    self.forward_terminal_events(ui, settings, events);
                }
            }
            return;
        }
        let previous_cursor = self.editor.cursor();
        if !active
            || ui.input(|input| {
                input.pointer.any_pressed()
                    && input.pointer.interact_pos().is_some_and(|pos| {
                        self.surface
                            .completion_rect
                            .is_none_or(|rect| !rect.contains(pos))
                    })
            })
        {
            self.dismiss_completions();
        }
        self.editor.completion_open =
            !self.completions.is_empty() || self.surface.completion_request.is_some();
        self.editor.history_active = self.history_search.is_some();
        if active
            && (self.editor.completion_open
                || (self.editor_has_focus && !ui.input(|input| input.pointer.any_pressed())))
        {
            self.editor.focus_requested = true;
        }
        self.editor.alt_is_meta =
            settings.input.left_alt_is_meta || settings.input.right_alt_is_meta;
        let font = FontId::monospace(settings.font.size);
        let row_height = ui.fonts(|fonts| fonts.row_height(&font));
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
        let frame_response = ui.interact(rect, ui.id().with("input-frame"), Sense::click());
        if active && frame_response.clicked() {
            self.focus_input();
        }
        ui.painter().rect(
            rect,
            6.0,
            theme.input,
            Stroke::new(
                1.0_f32,
                if active {
                    theme.accent.gamma_multiply(0.8)
                } else {
                    theme.border
                },
            ),
            egui::StrokeKind::Inside,
        );
        let content = rect.shrink(shell_prompt::INSET);
        let prompt_height = prompt
            .header_height
            .min((content.height() - row_height - shell_prompt::EDITOR_PADDING_Y * 2.0).max(0.0));
        let input_top = content.top() + prompt_height;
        prompt.paint(&ui.painter_at(content), content, input_top, theme);
        let editor_rect = egui::Rect::from_min_max(
            egui::pos2(content.left() + prompt.prefix_width, input_top),
            egui::pos2(
                content.right() - prompt.suffix_width - 2.0,
                content.bottom(),
            ),
        );
        let mut editor_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("editor")
                .max_rect(editor_rect),
        );
        editor_ui.set_clip_rect(editor_rect.intersect(ui.clip_rect()));
        let (response, editor_output) = self.editor.show(
            &mut editor_ui,
            ("command-editor", self.id),
            font,
            theme,
            editor_rect.height(),
            active && (!self.surface.submit_pending || self.active_command.is_some()),
        );
        self.last_editor_cursor = self.editor.cursor().min(self.editor.text().len());

        if self.surface.input_bridge
            && editor_output.action != Some(CommandEditorAction::Submit)
            && (editor_output.changed || previous_cursor != self.editor.cursor())
        {
            self.sync_shell_editor();
        }
        let shell_events = std::mem::take(&mut self.editor.shell_events);
        self.forward_terminal_events(ui, settings, shell_events);
        if editor_output.changed || previous_cursor != self.editor.cursor() {
            self.history_search = None;
            self.history_offset = 0;
            self.dismiss_completions();
        }
        if editor_output.changed && !self.surface.input_bridge && !self.surface.query_bridge {
            self.refresh_completions();
        }
        if response.has_focus() {
            self.block_focus.focused = None;
            match editor_output.action {
                Some(CommandEditorAction::CompletionNext) if !self.completions.is_empty() => {
                    self.completion_navigating = true;
                    self.selected_completion = self.selected_completion.saturating_sub(1);
                }
                Some(CommandEditorAction::CompletionPrevious) if !self.completions.is_empty() => {
                    self.completion_navigating = true;
                    self.selected_completion =
                        (self.selected_completion + 1).min(self.completions.len() - 1);
                }
                Some(CommandEditorAction::Complete) if !self.completions.is_empty() => {
                    self.accept_completion();
                    ui.ctx().request_repaint();
                }
                Some(CommandEditorAction::Complete) => self.refresh_completions(),
                Some(CommandEditorAction::DismissCompletions) => self.dismiss_completions(),
                Some(
                    CommandEditorAction::CompletionNext | CommandEditorAction::CompletionPrevious,
                ) if self.surface.completion_request.is_some() => {}
                Some(CommandEditorAction::Submit)
                    if self.completion_navigating && !self.completions.is_empty() =>
                {
                    self.accept_completion()
                }
                Some(CommandEditorAction::CompletionNext) if self.surface.input_bridge => {
                    self.history_search
                        .get_or_insert_with(|| self.editor.text().to_owned());
                    self.write_terminal(self.bridge_key(95));
                }
                Some(CommandEditorAction::CompletionPrevious) => self.show_history(),
                Some(CommandEditorAction::CompletionNext) => self.recall_history(-1),
                Some(CommandEditorAction::Submit) => {
                    self.submit_input();
                    response.surrender_focus();
                    ui.ctx().request_repaint();
                }
                _ => {}
            }
        }
        if !self.completions.is_empty() {
            let max_height = (rect.top() - ui.ctx().screen_rect().top() - 16.0).clamp(1.0, 360.0);
            let popup = egui::Area::new(egui::Id::new(("completions", self.id)))
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::LEFT_BOTTOM)
                .fixed_pos(egui::pos2(rect.left(), rect.top() - 4.0))
                .show(ui.ctx(), |ui| {
                    ui.set_width((rect.width() - 12.0).max(1.0));
                    self.render_completions(ui, theme, max_height);
                });
            self.surface.completion_rect = Some(popup.response.rect);
        }
        self.editor_has_focus = response.has_focus();
    }

    fn input_prompt_label(&self) -> String {
        self.input_prompt_text
            .as_deref()
            .filter(|prompt| !prompt.trim().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| prompt_label(&self.cwd))
    }

    fn render_completions(&mut self, ui: &mut egui::Ui, theme: &TerminasteTheme, max_height: f32) {
        egui::Frame::new()
            .fill(theme.surface_high)
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("completion-list", self.completion_revision))
                    .stick_to_bottom(true)
                    .max_height(max_height)
                    .show(ui, |ui| {
                        for index in (0..self.completions.len()).rev() {
                            let item = &self.completions[index];
                            let selected = index == self.selected_completion;
                            let mut text = egui::text::LayoutJob::simple(
                                item.label.clone(),
                                egui::TextStyle::Monospace.resolve(ui.style()),
                                theme.text,
                                (ui.available_width() - ui.spacing().button_padding.x * 2.0)
                                    .max(1.0),
                            );
                            text.wrap.max_rows = 2;
                            let galley = ui.fonts(|fonts| fonts.layout_job(text));
                            let response =
                                ui.add(egui::Button::new(galley).selected(selected).wrap());
                            if selected
                                && (self.surface.completion_rect.is_none()
                                    || ui.input(|input| {
                                        input.key_pressed(Key::ArrowUp)
                                            || input.key_pressed(Key::ArrowDown)
                                            || input.key_pressed(Key::Tab)
                                    }))
                            {
                                response.scroll_to_me(Some(egui::Align::Max));
                            }
                            if response.clicked() {
                                self.selected_completion = index;
                                self.accept_completion();
                                break;
                            }
                        }
                    });
            });
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
        let request = CompletionRequest::from_env(
            self.editor.text().to_owned(),
            self.last_editor_cursor.min(self.editor.text().len()),
            self.cwd.clone(),
            self.completion_revision,
        );
        let mut request = request;
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
        self.surface.completion_rect = None;
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
            let command = self.editor.text().to_owned();
            self.model.start_integrated_command(command);
            self.select_running_block();
            self.editor.clear();
            self.surface.submit_pending = true;
            self.dismiss_completions();
            return;
        }
        let command_text = self.editor.text().trim_end_matches(['\n', '\r']).to_owned();
        if command_text.trim().is_empty() {
            self.editor.clear();
            self.completions.clear();
            return;
        }
        self.active_command = Some(ActiveCommandMeta {
            command: command_text.clone(),
            cwd: self.cwd.clone(),
            started_at: Instant::now(),
        });
        self.model.discard_pending_output();
        let mut command = command_text;
        command.push('\r');
        self.model
            .start_integrated_command(command.trim_end_matches('\r').to_owned());
        self.select_running_block();
        let mut bytes = encode_paste(command.trim_end_matches('\r'), self.model.modes());
        bytes.push(b'\r');
        if let Some(pty) = &self.pty {
            let _ = pty.tx.send(PtyCommand::Write(bytes));
        }
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
        focusable_targets_for_blocks(&self.model.snapshot().blocks, "")
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

    fn running_block_target(&self) -> Option<TerminalBlockFocus> {
        self.model
            .snapshot()
            .blocks
            .iter()
            .rev()
            .find(|block| block.running)
            .map(|block| TerminalBlockFocus { block_id: block.id })
    }

    fn select_running_block(&mut self) {
        if let Some(target) = self.running_block_target() {
            self.block_focus.focus(target);
            self.editor_has_focus = true;
            self.editor.focus_requested = true;
            self.scroll_focused_block = false;
        }
    }

    fn focused_block_text(&self) -> Option<String> {
        let focused = self.block_focus.focused()?;
        self.model
            .snapshot()
            .blocks
            .into_iter()
            .find(|block| block.id == focused.block_id)
            .map(|block| block_input_and_output(&block))
    }

    fn match_count(&self, query: &str) -> usize {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return 0;
        }
        let snapshot = self.model.snapshot();
        let block_matches = snapshot
            .blocks
            .iter()
            .map(|block| count_matches(&block_context_text(block), &query))
            .sum::<usize>();
        let live_matches = count_matches(&snapshot.visible_lines.join("\n"), &query);
        block_matches + live_matches
    }

    fn process_ordered_pty_bytes(&mut self, bytes: &[u8]) {
        self.process_terminal_bytes(bytes);
    }

    fn finish_prompt_capture(&mut self) {
        if self.prompt_capture.is_empty() {
            return;
        }
        let prompt = sanitize_terminal_output(&String::from_utf8_lossy(&self.prompt_capture));
        self.prompt_capture.clear();
        self.set_input_prompt_text(prompt);
    }

    fn set_input_prompt_text(&mut self, prompt: String) {
        if let Some(prompt) = normalize_prompt_text(&prompt) {
            self.input_prompt_text = Some(prompt);
        }
    }

    fn handle_terminal_event(&mut self, event: TerminalEvent) {
        match event {
            TerminalEvent::Bell => self.status = "bell".to_owned(),
            TerminalEvent::TitleChanged(title) => self.title = title,
            TerminalEvent::ClipboardWriteRequested(payload) => {
                if let Some((_, data)) = payload.split_once(';') {
                    if let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(data) {
                        self.surface.clipboard = String::from_utf8(bytes).ok();
                    }
                }
            }
            TerminalEvent::Integration(event) => {
                self.apply_integration_event(event.name, event.payload)
            }
        }
    }

    fn apply_integration_event(&mut self, name: String, payload: String) {
        let Ok(decoded) = BASE64_URL_SAFE_NO_PAD.decode(payload.as_bytes()) else {
            return;
        };
        let Ok(envelope) = serde_json::from_slice::<IntegrationEnvelope>(&decoded) else {
            return;
        };
        let session = self.pty.as_ref().map(|pty| pty.id.to_string());
        if session.as_deref() != Some(envelope.session.as_str()) {
            return;
        }
        if let Some(shell_id) = &envelope.shell_id {
            if self.surface.shell_id.as_ref() != Some(shell_id) {
                if let Some(index) = self
                    .surface
                    .parent_shells
                    .iter()
                    .position(|shell| &shell.id == shell_id)
                {
                    let parent = self.surface.parent_shells.remove(index);
                    self.surface.parent_shells.truncate(index);
                    if self.active_command.take().is_some() {
                        self.model.finish_running_command(0);
                    }
                    self.surface.last_sequence = parent.sequence;
                    self.surface.input_revision = parent.revision;
                    self.surface.input_bridge = parent.input_bridge;
                    self.surface.query_bridge = parent.query_bridge;
                    self.surface.control_keys = parent.control_keys;
                    self.surface.isolated_shell = parent.isolated_shell;
                    self.editor.shell_bridge = parent.input_bridge;
                    self.surface.submit_pending = false;
                    self.surface.prompt_ansi = None;
                    self.surface.right_prompt_ansi = None;
                    self.surface.shell_id = Some(parent.id);
                    self.editor.clear();
                    self.focus_input();
                    self.dismiss_completions();
                } else if name == "ready" {
                    if let Some(id) = self.surface.shell_id.take() {
                        self.surface
                            .parent_shells
                            .push(terminal_surface::ShellContext {
                                id,
                                sequence: self.surface.last_sequence,
                                revision: self.surface.input_revision,
                                input_bridge: self.surface.input_bridge,
                                query_bridge: self.surface.query_bridge,
                                control_keys: self.surface.control_keys,
                                isolated_shell: self.surface.isolated_shell,
                            });
                    }
                    self.surface.shell_id = Some(shell_id.clone());
                    self.surface.last_sequence = 0;
                } else {
                    return;
                }
            }
        }
        if envelope.sequence <= self.surface.last_sequence {
            if name != "ready" || self.active_command.is_none() {
                return;
            }
            self.surface.last_sequence = 0;
        }
        self.surface.last_sequence = envelope.sequence;
        self.apply_integration_cwd(&envelope);
        match name.as_str() {
            "editor-ready" => {
                if envelope
                    .data
                    .get("revision")
                    .and_then(|value| value.as_u64())
                    .is_none_or(|revision| revision < self.surface.input_revision)
                {
                    return;
                }
                self.surface.submit_pending = false;
                if self.active_command.is_none() {
                    self.model.discard_submitted_command();
                }
                self.editor.focus_requested = true;
            }
            "completions" => {
                let revision = envelope
                    .data
                    .get("revision")
                    .and_then(|value| value.as_u64());
                if revision.is_some()
                    && revision == self.surface.completion_request
                    && revision == Some(self.surface.input_revision)
                    && envelope.data.get("text").and_then(|value| value.as_str())
                        == Some(self.editor.text())
                {
                    self.surface.completion_request = None;
                    self.completion_revision = self.completion_revision.saturating_add(1);
                    if let Some(items) = envelope
                        .data
                        .get("items")
                        .and_then(|value| value.as_array())
                    {
                        self.completions.clear();
                        self.surface.completion_cursors.clear();
                        let mut seen = std::collections::HashSet::new();
                        for item in items {
                            if let (Some(text), Some(cursor)) = (
                                item.get("text").and_then(|value| value.as_str()),
                                item.get("cursor").and_then(|value| value.as_u64()),
                            ) {
                                if !seen.insert(text) {
                                    continue;
                                }
                                self.completions.push(CompletionItem {
                                    label: text.to_owned(),
                                    replacement: text.to_owned(),
                                    description: if self.history_search.is_some() {
                                        "history".to_owned()
                                    } else {
                                        String::new()
                                    },
                                    kind: CompletionKind::Command,
                                    range: 0..self.editor.text().len(),
                                    score: 0,
                                });
                                self.surface.completion_cursors.push(cursor as usize);
                            }
                        }
                        self.selected_completion = 0;
                        self.completion_navigating = false;
                        if self.completions.is_empty() && self.history_search.is_none() {
                            self.refresh_local_completions();
                        }
                    }
                }
            }
            "input-buffer" => {
                if self.surface.submit_pending || self.active_command.is_some() {
                    return;
                }
                let revision = envelope
                    .data
                    .get("revision")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                if revision < self.surface.input_revision {
                    return;
                }
                self.surface.prompt_cols = envelope
                    .data
                    .get("columns")
                    .and_then(|value| value.as_u64())
                    .and_then(|cols| u16::try_from(cols).ok())
                    .filter(|cols| *cols > 0);
                if let Some(text) = envelope.data.get("text").and_then(|value| value.as_str()) {
                    if self.editor.text() != text {
                        self.dismiss_completions();
                        self.editor.set_text(text.to_owned());
                    }
                    if let Some(cursor) =
                        envelope.data.get("cursor").and_then(|value| value.as_u64())
                    {
                        self.editor.cursor = char_to_byte_index(text, cursor as usize);
                    }
                }
                self.editor.shell_command_mode = envelope
                    .data
                    .get("keymap")
                    .and_then(|value| value.as_str())
                    .is_some_and(|keymap| keymap == "vicmd");
                if let Some(prompt) = envelope.data.get("prompt").and_then(|value| value.as_str()) {
                    self.surface.prompt_ansi = Some(prompt.to_owned());
                    self.set_input_prompt_text(sanitize_terminal_output(prompt));
                }
                self.surface.right_prompt_ansi = envelope
                    .data
                    .get("right_prompt")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned);
            }
            "prompt-start" => {
                let snapshot = self.model.snapshot();
                self.surface.prompt_row = snapshot.visible_row_start + snapshot.cursor_row as u64;
                self.finish_prompt_capture();
                self.prompt_capture.clear();
                self.suppress_prompt_output = true;
            }
            "prompt-end" => {
                self.finish_prompt_capture();
                self.suppress_prompt_output = false;
            }
            "ready" => {
                if let Some(active) = self.active_command.take() {
                    self.model
                        .finish_running_command_matching(Some(&active.command), 0);
                }
                self.integration_ready = true;
                self.surface.input_revision = 0;
                self.surface.submit_pending = false;
                self.surface.prompt_ansi = None;
                self.surface.right_prompt_ansi = None;
                self.editor.clear();
                self.focus_input();
                self.dismiss_completions();
                self.surface.input_bridge = envelope
                    .data
                    .get("input_bridge")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                self.editor.shell_bridge = self.surface.input_bridge;
                self.surface.query_bridge = envelope
                    .data
                    .get("query_bridge")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                self.surface.control_keys = envelope
                    .data
                    .get("control_keys")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                self.surface.isolated_shell = envelope
                    .data
                    .get("isolated")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if let Some(shell) = envelope
                    .data
                    .get("shell_family")
                    .and_then(|value| value.as_str())
                {
                    self.status = format!("{shell} integration ready");
                }
            }
            "directory-change" => {}
            "aliases" => {
                if let Some(map) = envelope.data.get("map").and_then(|value| value.as_object()) {
                    self.aliases = map
                        .iter()
                        .filter_map(|(key, value)| {
                            value.as_str().map(|value| (key.clone(), value.to_owned()))
                        })
                        .collect();
                }
            }
            "command-start" => {
                self.dismiss_completions();
                self.editor.clear();
                self.editor.focus_requested = true;
                self.finish_prompt_capture();
                self.suppress_prompt_output = false;
                if let Some(command) = envelope
                    .data
                    .get("command")
                    .and_then(|value| value.as_str())
                {
                    let command = command.to_owned();
                    if self
                        .active_command
                        .as_ref()
                        .is_none_or(|active| active.command != command)
                    {
                        self.active_command = Some(ActiveCommandMeta {
                            command: command.clone(),
                            cwd: self.cwd.clone(),
                            started_at: Instant::now(),
                        });
                    }
                    self.status = "command running".to_owned();
                    if self.surface.submit_pending && self.model.has_running_command() {
                        self.model.confirm_submitted_command(command);
                    } else {
                        self.model.start_integrated_command(command);
                    }
                    self.surface.submit_pending = false;
                    self.select_running_block();
                }
            }
            "command-end" => {
                let exit_code = envelope
                    .data
                    .get("exit_code")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0) as i32;
                let command = envelope
                    .data
                    .get("command")
                    .and_then(|value| value.as_str())
                    .or_else(|| {
                        self.active_command
                            .as_ref()
                            .map(|active| active.command.as_str())
                    })
                    .map(str::to_owned);
                self.model
                    .finish_running_command_matching(command.as_deref(), exit_code);
                if self.active_command.as_ref().is_some_and(|active| {
                    command
                        .as_deref()
                        .is_none_or(|command| active.command == command)
                }) {
                    let active = self.active_command.take().unwrap();
                    self.last_command_summary = Some(format!(
                        "{} exit {exit_code} in {}",
                        active.command,
                        format_duration_millis(millis(active.started_at.elapsed()))
                    ));
                }
                self.status = if exit_code == 0 {
                    "ready".to_owned()
                } else {
                    format!("last command failed: {exit_code}")
                };
                self.focus_input();
            }
            _ => {}
        }
    }

    fn apply_integration_cwd(&mut self, envelope: &IntegrationEnvelope) {
        if let Some(cwd) = envelope
            .data
            .get("current_directory")
            .and_then(|value| value.as_str())
        {
            self.cwd = PathBuf::from(cwd);
        }
    }
}

pub fn integration_frame_for_tests(name: &str, data_json: &str) -> Vec<u8> {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let data = serde_json::from_str::<serde_json::Value>(data_json).unwrap_or_default();
    let payload = BASE64_URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&serde_json::json!({
            "session": Uuid::nil().to_string(),
            "sequence": SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            "time_ms": 0,
            "data": data,
        }))
        .unwrap(),
    );
    format!("\x1bPterminaste;1;{name};json64;{payload}\x1b\\").into_bytes()
}

fn default_pane_layout_snapshot() -> TerminalPaneLayoutSnapshot {
    TerminalPaneLayoutSnapshot {
        blocks_stick_to_bottom: true,
        input_below_blocks: true,
        input_fixed_to_bottom: true,
    }
}

#[derive(Debug, Deserialize)]
struct IntegrationEnvelope {
    session: String,
    shell_id: Option<String>,
    sequence: u64,
    data: serde_json::Value,
}

fn settings_section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.group(|ui| {
        ui.heading(title);
        add_contents(ui);
    });
    ui.add_space(10.0);
}

#[cfg(test)]
fn strip_command_echo_text(text: &str, command: &str) -> String {
    let trimmed = text.trim_start_matches(['\r', '\n']);
    let Some(rest) = trimmed.strip_prefix(command) else {
        return text.to_owned();
    };
    rest.strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))
        .or_else(|| rest.strip_prefix('\r'))
        .unwrap_or(rest)
        .to_owned()
}

fn normalize_prompt_text(text: &str) -> Option<String> {
    let lines = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn theme_for(settings: &Settings, system_theme: Option<egui::Theme>) -> TerminasteTheme {
    match settings.appearance.mode {
        AppearanceMode::Light => TerminasteTheme::light(),
        AppearanceMode::Dark => TerminasteTheme::dark(),
        AppearanceMode::System if system_theme == Some(egui::Theme::Light) => {
            TerminasteTheme::light()
        }
        AppearanceMode::System => TerminasteTheme::dark(),
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

fn apply_egui_theme(ctx: &egui::Context, theme: &TerminasteTheme) {
    let mut visuals = if theme.background.r() > 128 {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    visuals.panel_fill = theme.background;
    visuals.window_fill = theme.surface;
    visuals.widgets.inactive.bg_fill = theme.surface_high;
    visuals.widgets.inactive.weak_bg_fill = theme.surface_high;
    visuals.widgets.hovered.bg_fill = theme.surface_high;
    visuals.widgets.hovered.weak_bg_fill = theme.surface_high;
    visuals.widgets.active.bg_fill = theme.surface_high;
    visuals.widgets.active.weak_bg_fill = theme.surface_high;
    visuals.selection.bg_fill = theme.accent.gamma_multiply(0.3);
    visuals.selection.stroke = Stroke::new(1.0_f32, theme.accent);
    visuals.override_text_color = Some(theme.text);
    ctx.set_visuals(visuals);
}

fn install_system_fonts(ctx: &egui::Context, configured_family: &str) {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let mut definitions = egui::FontDefinitions::default();
    let mut families = configured_family
        .split(',')
        .map(str::trim)
        .filter(|family| !family.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let configured_count = families.len();
    for fallback in [
        "Symbols Nerd Font Mono",
        "Symbols Nerd Font",
        "Apple Symbols",
        "Noto Sans Symbols 2",
        "Segoe UI Symbol",
        "Noto Sans",
    ] {
        if !families.iter().any(|family| family == fallback) {
            families.push(fallback.to_owned());
        }
    }
    let mut loaded = Vec::new();
    for (index, family) in families.into_iter().enumerate() {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(&family)],
            ..fontdb::Query::default()
        };
        let Some(id) = database.query(&query) else {
            continue;
        };
        let Some((bytes, face_index)) =
            database.with_face_data(id, |bytes, face_index| (bytes.to_vec(), face_index))
        else {
            continue;
        };
        let name = format!("terminaste-system-font-{}", loaded.len());
        let mut data = FontData::from_owned(bytes);
        data.index = face_index;
        definitions.font_data.insert(name.clone(), data.into());
        loaded.push((name, index < configured_count));
    }
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        if let Some(fonts) = definitions.families.get_mut(&family) {
            let primary = loaded
                .iter()
                .filter(|(_, configured)| *configured && family == FontFamily::Monospace)
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            fonts.splice(0..0, primary);
            fonts.extend(
                loaded
                    .iter()
                    .filter(|(_, configured)| !*configured)
                    .map(|(name, _)| name.clone()),
            );
        }
    }
    ctx.set_fonts(definitions);
}

fn thin_separator(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(0.5_f32, color));
}

fn command_block_spacing(settings: &Settings) -> (i8, f32) {
    match settings.appearance.block_spacing {
        BlockSpacing::Normal => (4, 8.0),
        BlockSpacing::Compact => (3, 6.0),
    }
}

fn event_filter_all() -> egui::EventFilter {
    egui::EventFilter {
        tab: true,
        horizontal_arrows: true,
        vertical_arrows: true,
        escape: true,
    }
}

fn wrap_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as isize;
    (current as isize + delta).rem_euclid(len) as usize
}

fn shortcut_matches(binding: &str, key: Key, modifiers: egui::Modifiers) -> bool {
    let parts = binding.split('+').collect::<Vec<_>>();
    let name = match key {
        Key::ArrowUp => "up",
        Key::ArrowDown => "down",
        Key::ArrowLeft => "left",
        Key::ArrowRight => "right",
        _ => key.name(),
    };
    parts
        .last()
        .is_some_and(|part| part.eq_ignore_ascii_case(name))
        && parts.contains(&"cmd") == modifiers.mac_cmd
        && parts.contains(&"ctrl") == modifiers.ctrl
        && parts.contains(&"shift") == modifiers.shift
        && parts.contains(&"alt") == modifiers.alt
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn format_duration(duration_ms: u64) -> String {
    format_duration_millis(duration_ms)
}

fn format_duration_millis(duration_ms: u64) -> String {
    if duration_ms < 1_000 {
        return format!("{duration_ms}ms");
    }
    format!("{:.1}s", duration_ms as f64 / 1_000.0)
}

fn input_row_height(settings: &Settings) -> f32 {
    (settings.font.size * settings.font.line_height).max(18.0) + 18.0
}

fn prompt_label(cwd: &std::path::Path) -> String {
    let name = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("/");
    format!("{name} ›")
}

fn compact_path(path: &str) -> String {
    const MAX_PATH_CHARS: usize = 36;
    if path.chars().count() <= MAX_PATH_CHARS {
        return path.to_owned();
    }
    let tail: String = path
        .chars()
        .rev()
        .take(MAX_PATH_CHARS - 1)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("…{tail}")
}

fn block_matches(text: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty()
        || text
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
}

fn focusable_targets_for_blocks(
    blocks: &[CommandBlock],
    find_query: &str,
) -> Vec<TerminalBlockFocus> {
    blocks
        .iter()
        .filter(|block| block_matches(&block_context_text(block), find_query))
        .map(|block| TerminalBlockFocus { block_id: block.id })
        .collect()
}

fn count_matches(text: &str, lowercase_query: &str) -> usize {
    if lowercase_query.is_empty() {
        return 0;
    }
    text.to_ascii_lowercase()
        .match_indices(lowercase_query)
        .count()
}

fn block_context_text(block: &CommandBlock) -> String {
    let mut text = format!("$ {}", block.command);
    if let Some(exit_code) = block.exit_code {
        text.push_str(&format!("\nexit {exit_code}"));
    }
    if let Some(duration_ms) = block.duration_ms {
        text.push_str(&format!("\nduration {}", format_duration(duration_ms)));
    }
    if !block.output.is_empty() {
        text.push('\n');
        text.push_str(&block.output);
    }
    text
}

fn highlighted_text(
    text: &str,
    find_query: &str,
    font: FontId,
    theme: &TerminasteTheme,
) -> RichText {
    let mut display = text.to_owned();
    let query = find_query.trim();
    if !query.is_empty()
        && text
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
    {
        display = format!("▸ {display}");
    }
    RichText::new(display).font(font).color(theme.text)
}

fn render_ansi_output(
    ui: &mut egui::Ui,
    text: &str,
    find_query: &str,
    font: FontId,
    theme: &TerminasteTheme,
) -> egui::Response {
    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.add(egui::Label::new(ansi_layout_job(text, find_query, font, theme)).wrap())
    })
    .inner
}

fn render_block_actions(ui: &mut egui::Ui, block: &CommandBlock) {
    for (label, text) in [
        ("Copy input", block.command.clone()),
        ("Copy output", block.output.clone()),
        ("Copy input and output", block_input_and_output(block)),
    ] {
        if ui.button(label).clicked() {
            ui.ctx().copy_text(text);
            ui.close_menu();
        }
    }
}

fn block_input_and_output(block: &CommandBlock) -> String {
    if block.output.is_empty() {
        block.command.clone()
    } else {
        format!("{}\n{}", block.command, block.output)
    }
}

fn ansi_layout_job(
    text: &str,
    find_query: &str,
    font: FontId,
    theme: &TerminasteTheme,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let query = find_query.trim().to_ascii_lowercase();
    for (index, line) in ansi_lines(text, theme).into_iter().enumerate() {
        if index > 0 {
            job.append(
                "\n",
                0.0,
                egui::TextFormat::simple(font.clone(), theme.text),
            );
        }
        for span in line {
            let mut format = egui::TextFormat::simple(font.clone(), span.color);
            if !query.is_empty() && span.text.to_ascii_lowercase().contains(&query) {
                format.background = theme.warning.gamma_multiply(0.25);
            }
            job.append(&span.text, 0.0, format);
        }
    }
    job
}

#[derive(Debug, Clone)]
struct AnsiSpan {
    text: String,
    color: Color32,
}

fn ansi_lines(text: &str, theme: &TerminasteTheme) -> Vec<Vec<AnsiSpan>> {
    let mut lines = vec![Vec::new()];
    let mut current = String::new();
    let mut color = theme.text;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => {
                flush_ansi_span(&mut lines, &mut current, color);
                if chars.peek() == Some(&'[') {
                    chars.next();
                    let mut seq = String::new();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            if next == 'm' {
                                color = apply_ansi_color(&seq, color, theme);
                            }
                            break;
                        }
                        seq.push(next);
                    }
                } else if matches!(chars.peek(), Some(']' | 'P' | '^' | '_')) {
                    chars.next();
                    consume_ansi_string(&mut chars);
                } else {
                    chars.next();
                }
            }
            '\r' => {}
            '\n' => {
                flush_ansi_span(&mut lines, &mut current, color);
                lines.push(Vec::new());
            }
            '\u{8}' | '\u{7f}' => {
                current.pop();
            }
            ch if ch.is_control() => {}
            ch => current.push(ch),
        }
    }
    flush_ansi_span(&mut lines, &mut current, color);
    while lines.last().is_some_and(Vec::is_empty) && lines.len() > 1 {
        lines.pop();
    }
    lines
}

fn consume_ansi_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(ch) = chars.next() {
        if ch == '\u{7}' {
            break;
        }
        if ch == '\x1b' && chars.peek() == Some(&'\\') {
            chars.next();
            break;
        }
    }
}

fn flush_ansi_span(lines: &mut [Vec<AnsiSpan>], current: &mut String, color: Color32) {
    if current.is_empty() {
        return;
    }
    if let Some(line) = lines.last_mut() {
        line.push(AnsiSpan {
            text: std::mem::take(current),
            color,
        });
    }
}

fn apply_ansi_color(seq: &str, current: Color32, theme: &TerminasteTheme) -> Color32 {
    let values = seq
        .split(';')
        .filter_map(|value| value.parse::<u16>().ok())
        .collect::<Vec<_>>();
    if values.is_empty() || values.contains(&0) || values.contains(&39) {
        return theme.text;
    }
    let mut color = current;
    let mut index = 0;
    while index < values.len() {
        match values[index] {
            30..=37 => color = ansi_palette((values[index] - 30) as usize, theme),
            90..=97 => color = ansi_palette((values[index] - 90 + 8) as usize, theme),
            38 if values.get(index + 1) == Some(&5) => {
                if let Some(value) = values.get(index + 2) {
                    color = xterm_color(*value as usize, theme);
                    index += 2;
                }
            }
            38 if values.get(index + 1) == Some(&2) => {
                if let (Some(r), Some(g), Some(b)) = (
                    values.get(index + 2),
                    values.get(index + 3),
                    values.get(index + 4),
                ) {
                    color = Color32::from_rgb(*r as u8, *g as u8, *b as u8);
                    index += 4;
                }
            }
            _ => {}
        }
        index += 1;
    }
    color
}

fn ansi_palette(index: usize, theme: &TerminasteTheme) -> Color32 {
    match index {
        0 => theme.text,
        1 => theme.error,
        2 => theme.success,
        3 => theme.warning,
        4 => Color32::from_rgb(96, 165, 250),
        5 => Color32::from_rgb(167, 139, 250),
        6 => theme.accent,
        7 => theme.text,
        8 => theme.muted,
        9 => Color32::from_rgb(248, 113, 113),
        10 => Color32::from_rgb(74, 222, 128),
        11 => Color32::from_rgb(251, 191, 36),
        12 => Color32::from_rgb(147, 197, 253),
        13 => Color32::from_rgb(196, 181, 253),
        14 => Color32::from_rgb(103, 232, 249),
        _ => theme.text,
    }
}

fn xterm_color(index: usize, theme: &TerminasteTheme) -> Color32 {
    if index < 16 {
        return ansi_palette(index, theme);
    }
    if (16..=231).contains(&index) {
        let value = index - 16;
        let component = |n: usize| if n == 0 { 0 } else { 55 + n * 40 } as u8;
        return Color32::from_rgb(
            component(value / 36),
            component((value / 6) % 6),
            component(value % 6),
        );
    }
    let gray = (8 + (index.saturating_sub(232) * 10)).min(255) as u8;
    Color32::from_rgb(gray, gray, gray)
}

fn sanitize_terminal_output(text: &str) -> String {
    ansi_lines(text, &TerminasteTheme::dark())
        .into_iter()
        .map(|line| line.into_iter().map(|span| span.text).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .map(|(index, _)| index)
        .nth(char_index)
        .unwrap_or(text.len())
}

fn previous_char_boundary(text: &str, index: usize) -> usize {
    if index == 0 {
        return 0;
    }
    let mut current = index.min(text.len());
    while current > 0 && !text.is_char_boundary(current) {
        current -= 1;
    }
    text[..current]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut current = index + 1;
    while current < text.len() && !text.is_char_boundary(current) {
        current += 1;
    }
    current
}

fn word_bounds(text: &str, index: usize) -> (usize, usize) {
    if text.is_empty() {
        return (0, 0);
    }
    let index = if index == text.len() {
        previous_char_boundary(text, index)
    } else {
        index
    };
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    let current_is_word = text[index..].chars().next().map(is_word).unwrap_or(false);
    let mut start = index;
    while start > 0 {
        let previous = previous_char_boundary(text, start);
        let Some(character) = text[previous..start].chars().next() else {
            break;
        };
        if is_word(character) != current_is_word || character.is_whitespace() {
            break;
        }
        start = previous;
    }
    let mut end = next_char_boundary(text, index);
    while end < text.len() {
        let next = next_char_boundary(text, end);
        let Some(character) = text[end..next].chars().next() else {
            break;
        };
        if is_word(character) != current_is_word || character.is_whitespace() {
            break;
        }
        end = next;
    }
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_uses_home_and_new_tabs_inherit_the_active_directory() {
        let mut settings = Settings::default();
        let home = directories::BaseDirs::new().unwrap().home_dir().to_owned();
        assert_eq!(startup_directory(&settings), home);
        settings.startup.working_directory = Some(PathBuf::from("/configured"));
        assert_eq!(startup_directory(&settings), PathBuf::from("/configured"));
        let mut app = TerminasteApp::headless_for_tests(settings);
        app.active_terminal_mut().unwrap().cwd = PathBuf::from("/active-project");
        app.new_tab();
        assert_eq!(
            app.active_terminal().unwrap().cwd,
            PathBuf::from("/active-project")
        );
    }

    #[test]
    fn starts_with_one_tab_and_terminal_pane() {
        let app = TerminasteApp::headless_for_tests(Settings::default());
        assert_eq!(app.tab_count(), 1);
        assert_eq!(app.active_pane_count(), 1);
    }

    #[test]
    fn submit_input_clears_editor() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.submit_active_input_for_tests("echo ok");
        assert!(app.active_terminal_mut().unwrap().editor.text().is_empty());
    }

    #[test]
    fn command_editor_inserts_at_cursor() {
        let mut editor = CommandEditorState::new();
        editor.insert_text("helo");
        editor.move_left(false);
        editor.insert_text("l");
        assert_eq!(editor.text(), "hello");
        assert_eq!(editor.cursor(), 4);
    }

    #[test]
    fn command_editor_deletes_around_cursor() {
        let mut editor = CommandEditorState::new();
        editor.insert_text("abçd");
        editor.move_left(false);
        assert!(editor.backspace());
        assert_eq!(editor.text(), "abd");
        assert!(editor.delete_forward());
        assert_eq!(editor.text(), "ab");
    }

    #[test]
    fn command_editor_replaces_selection() {
        let mut editor = CommandEditorState::new();
        editor.insert_text("echo old value");
        editor.set_cursor_from_mouse_index(5, false);
        editor.set_cursor_from_mouse_index(8, true);
        editor.insert_text("new");
        assert_eq!(editor.text(), "echo new value");
        assert_eq!(editor.cursor(), 8);
    }

    #[test]
    fn command_editor_shift_selection_navigation() {
        let mut editor = CommandEditorState::new();
        editor.insert_text("abc");
        editor.move_left(true);
        editor.move_left(true);
        assert_eq!(editor.selected_text(), Some("bc"));
        editor.move_left(false);
        assert_eq!(editor.cursor(), 1);
        assert!(!editor.has_selection());
    }

    #[test]
    fn command_editor_maps_mouse_point_to_index() {
        let mut editor = CommandEditorState::new();
        editor.insert_text("ab\ncd");
        let index = editor.index_from_monospace_point(
            egui::pos2(24.0, 20.0),
            egui::pos2(0.0, 0.0),
            10.0,
            16.0,
        );
        assert_eq!(index, 5);
    }

    #[test]
    fn command_editor_selects_complete_multiline_row() {
        let mut editor = CommandEditorState::new();
        editor.insert_text("one two\nthree four\nfive");
        editor.select_line_at(10);
        assert_eq!(editor.selected_text(), Some("three four\n"));
    }

    #[test]
    fn command_editor_moves_across_soft_wrapped_rows() {
        let mut editor = CommandEditorState::new();
        editor.insert_text("one two three four");
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |_| {});
        let galley = context.fonts(|fonts| {
            fonts.layout(
                editor.text().to_owned(),
                FontId::monospace(14.0),
                Color32::WHITE,
                45.0,
            )
        });
        editor.move_to_start(false);
        assert!(editor.move_visual_row(&galley, 1, false));
        assert!(editor.cursor() > 0);
        editor.move_visual_row(&galley, 1, true);
        assert!(editor.has_selection());
    }

    #[test]
    fn command_editor_mouse_drag_selects_across_lines_and_uses_text_cursor() {
        let mut editor = CommandEditorState::new();
        editor.shell_bridge = true;
        editor.set_text("alpha\nbeta gamma".to_owned());
        let context = egui::Context::default();
        let render = |editor: &mut CommandEditorState,
                      events: Vec<egui::Event>|
         -> (egui::Rect, egui::FullOutput) {
            let rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 180.0),
                    )),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let (response, _) = editor.show(
                            ui,
                            "mouse-editor",
                            FontId::monospace(14.0),
                            &TerminasteTheme::dark(),
                            90.0,
                            true,
                        );
                        rect.set(response.rect);
                    });
                },
            );
            (rect.get(), output)
        };

        let (rect, _) = render(&mut editor, Vec::new());
        let start = rect.min + egui::vec2(10.0, 12.0);
        let end = rect.min + egui::vec2(48.0, 34.0);
        render(
            &mut editor,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let (_, output) = render(&mut editor, vec![egui::Event::PointerMoved(end)]);
        render(
            &mut editor,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        assert!(editor
            .selected_text()
            .is_some_and(|text| text.contains('\n')));
        assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::Text);
        let selected = editor.selected_text().unwrap().to_owned();
        let original = editor.text().to_owned();
        let (_, copied) = render(&mut editor, vec![egui::Event::Copy]);
        assert!(copied
            .platform_output
            .commands
            .contains(&egui::OutputCommand::CopyText(selected.clone())));
        assert_eq!(editor.text(), original);
        assert_eq!(editor.selected_text(), Some(selected.as_str()));

        let (_, cut) = render(&mut editor, vec![egui::Event::Cut]);
        assert!(cut
            .platform_output
            .commands
            .contains(&egui::OutputCommand::CopyText(selected.clone())));
        assert!(!editor.has_selection());
        render(&mut editor, vec![egui::Event::Paste(selected)]);
        assert_eq!(editor.text(), original);

        render(
            &mut editor,
            vec![egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::MAC_CMD,
            }],
        );
        assert_eq!(editor.selected_text(), Some(original.as_str()));
        render(
            &mut editor,
            vec![egui::Event::Paste("echo café".to_owned())],
        );
        assert_eq!(editor.text(), "echo café");
        assert!(!editor.has_selection());
    }

    #[test]
    fn malformed_integration_data_does_not_replace_visible_status() {
        let mut pane = TerminalPane::fake();
        pane.status = "ready".to_owned();
        pane.apply_integration_event("ready".to_owned(), "not-base64".to_owned());
        assert_eq!(pane.status, "ready");
    }

    #[test]
    fn closing_inactive_tab_keeps_active_tab_selected() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.new_tab_for_tests();
        app.new_tab_for_tests();
        assert_eq!(app.active_tab, 2);
        app.close_tab(0);
        assert_eq!(app.active_tab, 1);
        assert_eq!(app.tab_count(), 2);
    }

    #[test]
    fn wrap_index_handles_reverse_navigation() {
        assert_eq!(wrap_index(0, 3, -1), 2);
        assert_eq!(wrap_index(2, 3, 1), 0);
    }

    #[test]
    fn block_focus_navigation_moves_between_command_blocks() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        {
            let pane = app.active_terminal_mut().unwrap();
            pane.model.start_integrated_command("echo one".to_owned());
            pane.model.process_bytes(b"one\n");
            pane.model.finish_running_command(0);
            pane.model.start_integrated_command("echo two".to_owned());
            pane.model.finish_running_command(0);
        }
        let snapshot = app.active_pane_snapshot_for_tests().unwrap();
        let first_block_id = snapshot.blocks[0].id;
        let second_block_id = snapshot.blocks[1].id;

        assert_eq!(
            app.navigate_active_block_focus_for_tests(1),
            Some((first_block_id, "block"))
        );
        assert_eq!(
            app.navigate_active_block_focus_for_tests(1),
            Some((second_block_id, "block"))
        );
        assert_eq!(
            app.navigate_active_block_focus_for_tests(-1),
            Some((first_block_id, "block"))
        );
        assert_eq!(
            app.active_block_focus_for_tests(),
            Some((first_block_id, "block"))
        );
    }

    #[test]
    fn running_command_is_selected_and_completion_returns_to_input() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());

        app.submit_active_input_for_tests("sleep 1");
        let running = app.active_pane_snapshot_for_tests().unwrap().blocks[0].id;

        assert_eq!(app.active_block_focus_for_tests(), Some((running, "block")));

        let pane = app.active_terminal_mut().unwrap();
        pane.finish_command_for_tests(0);

        assert!(pane.block_focus.focused().is_none());
        assert!(pane.editor.focus_requested);
    }

    #[test]
    fn match_filter_counts_block_context() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let pane = app.active_terminal_mut().unwrap();
        pane.model
            .start_integrated_command("echo needle".to_owned());
        pane.model.process_bytes(b"needle\n");
        pane.model.finish_running_command(0);
        assert!(pane.match_count("needle") >= 2);
        assert_eq!(pane.match_count("missing"), 0);
    }

    #[test]
    fn ordered_integration_captures_output_in_same_chunk() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let pane = app.active_terminal_mut().unwrap();
        let mut chunk = Vec::new();
        chunk.extend_from_slice(
            test_frame("command-start", serde_json::json!({"command":"echo ok"})).as_bytes(),
        );
        chunk.extend_from_slice(b"ok\n");
        chunk.extend_from_slice(
            test_frame("command-end", serde_json::json!({"exit_code":0})).as_bytes(),
        );

        pane.process_ordered_pty_bytes(&chunk);
        let snapshot = pane.model.snapshot();

        assert_eq!(snapshot.blocks.len(), 1);
        assert_eq!(snapshot.blocks[0].command, "echo ok");
        assert_eq!(snapshot.blocks[0].output, "ok");
    }

    #[test]
    fn prompt_region_stays_in_grid_without_creating_output_block() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let pane = app.active_terminal_mut().unwrap();
        let mut chunk = Vec::new();
        chunk.extend_from_slice(test_frame("prompt-start", serde_json::json!({})).as_bytes());
        chunk.extend_from_slice(b"oh-my-zsh prompt % ");
        chunk.extend_from_slice(test_frame("prompt-end", serde_json::json!({})).as_bytes());

        pane.process_ordered_pty_bytes(&chunk);

        assert!(pane.model.snapshot().blocks.is_empty());
        assert!(pane
            .model
            .snapshot()
            .visible_lines
            .join("\n")
            .contains("prompt %"));
    }

    #[test]
    fn oh_my_zsh_prompt_region_updates_input_prompt_without_output() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let pane = app.active_terminal_mut().unwrap();
        let mut chunk = Vec::new();
        chunk.extend_from_slice(
            test_frame(
                "prompt-start",
                serde_json::json!({"current_directory":"/tmp/work"}),
            )
            .as_bytes(),
        );
        chunk.extend_from_slice(
            b"\r\x1b[0m\x1b[32m\xE2\x9E\x9C  \x1b[36mwork \x1b[33mgit:(main) \x1b[0m",
        );
        chunk.extend_from_slice(b"% ");
        chunk.extend_from_slice(
            test_frame(
                "prompt-end",
                serde_json::json!({"current_directory":"/tmp/work"}),
            )
            .as_bytes(),
        );

        pane.process_ordered_pty_bytes(&chunk);
        let snapshot = pane.model.snapshot();

        assert!(snapshot.blocks.is_empty());
        assert!(snapshot
            .visible_lines
            .join("\n")
            .contains("work git:(main) %"));
        assert_eq!(pane.input_prompt_label(), "➜  work git:(main) %");
        assert_eq!(pane.cwd, PathBuf::from("/tmp/work"));
    }

    #[test]
    fn repeated_prompt_region_replaces_input_prompt_without_leaking() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let pane = app.active_terminal_mut().unwrap();

        for prompt in ["➜  one git:(main) % ", "➜  two git:(dev) % "] {
            let mut chunk = Vec::new();
            chunk.extend_from_slice(test_frame("prompt-start", serde_json::json!({})).as_bytes());
            chunk.extend_from_slice(prompt.as_bytes());
            chunk.extend_from_slice(test_frame("prompt-end", serde_json::json!({})).as_bytes());
            pane.process_ordered_pty_bytes(&chunk);
        }

        assert!(pane.model.snapshot().blocks.is_empty());
        assert_eq!(pane.input_prompt_label(), "➜  two git:(dev) %");
    }

    #[test]
    fn missing_markers_preserve_raw_terminal_output() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.submit_active_input_for_tests("printf ok");
        let pane = app.active_terminal_mut().unwrap();
        pane.process_ordered_pty_bytes(b"printf ok\r\nok\n");
        pane.finish_command_for_tests(0);

        pane.process_ordered_pty_bytes(
            b"\r\x1b[0m\x1b[32m\xE2\x9E\x9C  \x1b[36mwork \x1b[33mgit:(main) \x1b[0m% ",
        );
        let snapshot = pane.model.snapshot();

        assert_eq!(snapshot.blocks.last().unwrap().output, "printf ok\nok");
        assert!(snapshot
            .visible_lines
            .join("\n")
            .contains("work git:(main) %"));
    }

    #[test]
    fn fallback_prompt_detection_does_not_hide_normal_output() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let pane = app.active_terminal_mut().unwrap();

        pane.process_ordered_pty_bytes(b"total $5\n");

        assert!(pane
            .model
            .snapshot()
            .visible_lines
            .join("\n")
            .contains("total $5"));
    }

    #[test]
    fn local_echo_is_not_captured_as_command_output() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.submit_active_input_for_tests("echo ok");
        let pane = app.active_terminal_mut().unwrap();
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"echo ok\r\n");
        chunk.extend_from_slice(
            test_frame("command-start", serde_json::json!({"command":"echo ok"})).as_bytes(),
        );
        chunk.extend_from_slice(b"ok\n");
        chunk.extend_from_slice(
            test_frame("command-end", serde_json::json!({"exit_code":0})).as_bytes(),
        );

        pane.process_ordered_pty_bytes(&chunk);
        let snapshot = pane.model.snapshot();

        assert_eq!(snapshot.blocks.len(), 1);
        assert_eq!(snapshot.blocks[0].output, "ok");
    }

    #[test]
    fn subsequent_command_gets_its_own_output_block_content() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        {
            let pane = app.active_terminal_mut().unwrap();
            pane.model.start_integrated_command("echo ok".to_owned());
            pane.model.process_bytes(b"ok\n");
            pane.model.finish_running_command(0);
        }

        app.submit_active_input_for_tests("echo temp");
        let pane = app.active_terminal_mut().unwrap();
        pane.process_ordered_pty_bytes(b"echo temp\r\n");
        pane.process_ordered_pty_bytes(
            test_frame("command-start", serde_json::json!({"command":"echo temp"})).as_bytes(),
        );
        pane.process_ordered_pty_bytes(b"temp\n");
        assert!(pane.model.snapshot().blocks.last().unwrap().running);
        let snapshot = pane.model.snapshot();

        assert_eq!(snapshot.blocks.last().unwrap().command, "echo temp");
        assert_eq!(snapshot.blocks.last().unwrap().output, "temp");
    }

    #[test]
    fn normal_command_echo_is_stripped_from_output_capture() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.submit_active_input_for_tests("ls");
        let pane = app.active_terminal_mut().unwrap();

        pane.process_ordered_pty_bytes(b"ls\r\n");
        pane.process_ordered_pty_bytes(
            test_frame("command-start", serde_json::json!({"command":"ls"})).as_bytes(),
        );
        pane.process_ordered_pty_bytes(b"Cargo.toml\r\ncrates\r\n");
        let snapshot = pane.model.snapshot();

        assert_eq!(snapshot.blocks.last().unwrap().command, "ls");
        assert_eq!(snapshot.blocks.last().unwrap().output, "Cargo.toml\ncrates");
    }

    #[test]
    fn sequential_local_commands_strip_echo_and_keep_outputs_separate() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());

        app.submit_active_input_for_tests("echo temp");
        let pane = app.active_terminal_mut().unwrap();
        pane.process_ordered_pty_bytes(b"echo temp\r\n");
        pane.process_ordered_pty_bytes(
            test_frame("command-start", serde_json::json!({"command":"echo temp"})).as_bytes(),
        );
        pane.process_ordered_pty_bytes(b"temp\n");
        pane.model.finish_running_command(0);
        pane.active_command = None;

        app.submit_active_input_for_tests("ls");
        let pane = app.active_terminal_mut().unwrap();
        pane.process_ordered_pty_bytes(b"ls\r\n");
        pane.process_ordered_pty_bytes(
            test_frame("command-start", serde_json::json!({"command":"ls"})).as_bytes(),
        );
        pane.process_ordered_pty_bytes(b"Cargo.toml\ncrates\n");
        pane.model.finish_running_command(0);

        let snapshot = pane.model.snapshot();
        assert_eq!(snapshot.blocks.len(), 2);
        assert_eq!(snapshot.blocks[0].command, "echo temp");
        assert_eq!(snapshot.blocks[0].output, "temp");
        assert_eq!(snapshot.blocks[1].command, "ls");
        assert_eq!(snapshot.blocks[1].output, "Cargo.toml\ncrates");
    }

    #[test]
    fn integrated_command_keeps_echo_frames_and_output_in_active_block() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let pane = app.active_terminal_mut().unwrap();
        pane.integration_ready = true;

        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"echo temp\r\n");
        chunk.extend_from_slice(
            test_frame("command-start", serde_json::json!({"command":"echo temp"})).as_bytes(),
        );
        chunk.extend_from_slice(b"temp\n");
        chunk.extend_from_slice(b"stale raw grid text\n");
        chunk.extend_from_slice(
            test_frame(
                "command-end",
                serde_json::json!({"command":"echo temp","exit_code":0}),
            )
            .as_bytes(),
        );

        pane.process_ordered_pty_bytes(&chunk);
        let snapshot = pane.model.snapshot();

        assert_eq!(snapshot.blocks.len(), 1);
        assert_eq!(snapshot.blocks[0].command, "echo temp");
        assert_eq!(snapshot.blocks[0].output, "temp\nstale raw grid text");
        assert!(snapshot
            .visible_lines
            .join("\n")
            .contains("stale raw grid text"));
    }

    #[test]
    fn command_start_for_local_command_does_not_duplicate_or_reset_echo_strip() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.submit_active_input_for_tests("echo temp");
        let pane = app.active_terminal_mut().unwrap();

        pane.process_ordered_pty_bytes(b"echo temp\r\n");
        pane.process_ordered_pty_bytes(
            test_frame("command-start", serde_json::json!({"command":"echo temp"})).as_bytes(),
        );
        pane.process_ordered_pty_bytes(b"temp\n");
        pane.process_ordered_pty_bytes(
            test_frame(
                "command-end",
                serde_json::json!({"command":"echo temp","exit_code":0}),
            )
            .as_bytes(),
        );

        let snapshot = pane.model.snapshot();
        assert_eq!(snapshot.blocks.len(), 1);
        assert_eq!(snapshot.blocks[0].output, "temp");
    }

    #[test]
    fn command_echo_strip_helper_handles_line_endings() {
        assert_eq!(strip_command_echo_text("ls\r\nfile\n", "ls"), "file\n");
        assert_eq!(strip_command_echo_text("\r\nls\nfile", "ls"), "file");
        assert_eq!(strip_command_echo_text("file\n", "ls"), "file\n");
    }

    #[test]
    fn terminal_output_sanitizer_removes_sgr_and_control_bytes() {
        assert_eq!(
            sanitize_terminal_output("\x1b[7m\x1b[27mok\r\n\x1b[K"),
            "ok"
        );
    }

    #[test]
    fn terminal_output_sanitizer_removes_osc_and_dcs_payloads() {
        assert_eq!(
            sanitize_terminal_output("\x1b]0;title\x07ok\x1bPignored\x1b\\"),
            "ok"
        );
    }

    #[test]
    fn ansi_lines_keep_color_spans_without_rendering_escape_text() {
        let lines = ansi_lines("\x1b[31mred\x1b[0m plain", &TerminasteTheme::dark());
        assert_eq!(lines[0][0].text, "red");
        assert_eq!(lines[0][1].text, " plain");
        assert_ne!(lines[0][0].color, lines[0][1].color);
    }

    #[test]
    fn prompt_uses_directory_name() {
        assert!(prompt_label(std::path::Path::new("/tmp/work")).contains("work"));
    }

    fn test_frame(name: &str, data: serde_json::Value) -> String {
        String::from_utf8(integration_frame_for_tests(name, &data.to_string())).unwrap()
    }
}
