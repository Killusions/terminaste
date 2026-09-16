use std::collections::HashMap;

use gpui::{
    actions, canvas, div, fill, font, list, point, prelude::*, px, size, App, Bounds,
    ClipboardItem, Context, ElementInputHandler, FocusHandle, Focusable, FollowMode, Font,
    FontFallbacks, FontFeatures, FontWeight, KeyBinding, KeyDownEvent, ListAlignment, ListState,
    Menu, MenuItem, MouseButton, Pixels, Point, ScrollHandle, SharedString, Size, Window,
};
use terminaste_core::{
    encode_focus_event, encode_key, encode_mouse_sgr, encode_mouse_wheel_sgr, KeyInput, Modifiers,
    Osc52Policy, TerminalPoint,
};
use terminaste_settings::KeybindingAction as Action;

use super::*;
use editor::{paint_input, wrapped_rows, InputLayout};
use shell_prompt::PromptLayout;
use terminal_surface::{paint_cells, terminal_key};
use theme::TerminalTextStyle;

mod settings;

actions!(
    terminaste,
    [
        Quit,
        NewTab,
        CloseTab,
        Copy,
        Cut,
        Paste,
        SelectAll,
        OpenSettings,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom
    ]
);

pub fn install_actions(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    cx.set_menus([
        Menu::new("terminaste").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide terminaste", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit terminaste", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Tab", NewTab),
            MenuItem::action("Close Tab", CloseTab),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Cut", Cut),
            MenuItem::action("Copy", Copy),
            MenuItem::action("Paste", Paste),
            MenuItem::separator(),
            MenuItem::action("Select All", SelectAll),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlay {
    Actions,
    Settings,
    Find,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Find,
    FontFamily,
    KeybindingSearch,
    Redaction(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
    fn label(self) -> &'static str {
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

struct PaneView {
    history: ScrollHandle,
    history_list: ListState,
    history_count: usize,
    completions: ScrollHandle,
    completion_revision: u64,
    completion_index: usize,
    pending_command: bool,
    initialized: bool,
}

impl Default for PaneView {
    fn default() -> Self {
        Self {
            history: ScrollHandle::default(),
            history_list: ListState::new(0, ListAlignment::Bottom, px(180.)),
            history_count: 0,
            completions: ScrollHandle::default(),
            completion_revision: 0,
            completion_index: 0,
            pending_command: false,
            initialized: false,
        }
    }
}

pub struct TerminalWindow {
    pub(super) app: TerminasteApp,
    focus: FocusHandle,
    theme: TerminasteTheme,
    font: Font,
    cell: Size<Pixels>,
    overlay: Option<Overlay>,
    field: Option<Field>,
    field_editor: CommandEditorState,
    pub(super) input_layout: InputLayout,
    selecting_input: bool,
    category: SettingsCategory,
    find_query: String,
    keybinding_search: String,
    palette_index: usize,
    pane_views: HashMap<Uuid, PaneView>,
    split_drag: Option<(Uuid, Bounds<Pixels>, SplitDirection)>,
    block_menu: Option<(Uuid, Point<Pixels>)>,
    output_selection: Option<(Uuid, CommandEditorState)>,
    command_layouts: HashMap<Uuid, InputLayout>,
    selecting_command: bool,
    output_layouts: HashMap<Uuid, InputLayout>,
    selecting_output: bool,
    _task: gpui::Task<()>,
}

impl TerminalWindow {
    pub fn new(mut app: TerminasteApp, window: &mut Window, cx: &mut Context<Self>) -> Self {
        app.window_bounds = Some(window.window_bounds());
        cx.observe_window_bounds(window, |this, window, _| {
            this.app.window_bounds = Some(window.window_bounds());
        })
        .detach();
        cx.on_app_quit(|this, _| {
            this.app.persist_session();
            std::future::ready(())
        })
        .detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.on_release(|this, _| this.app.persist_session()).detach();
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        cx.observe_window_activation(window, |_, _, cx| cx.notify())
            .detach();
        let task = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(4))
                .await;
            if this
                .update(cx, |this, cx| {
                    let mut changed = false;
                    if let Some(loaded) = this
                        .app
                        .watcher
                        .as_mut()
                        .and_then(terminaste_settings::SettingsWatcher::poll)
                    {
                        this.app.loaded = loaded;
                        changed = true;
                    }
                    for tab in &mut this.app.tabs {
                        for pane in &mut tab.panes {
                            let scroll = this.pane_views.entry(pane.id).or_default();
                            let bottom = scroll.history.offset().y + scroll.history.max_offset().y
                                >= px(-2.);
                            if pane.drain_pty_events() {
                                changed = true;
                                if bottom {
                                    scroll.history.scroll_to_bottom();
                                }
                            }
                            let pending_command =
                                pane.active_command.as_ref().is_some_and(|command| {
                                    command.started_at.elapsed() >= Duration::from_millis(200)
                                });
                            if pending_command != scroll.pending_command {
                                scroll.pending_command = pending_command;
                                changed = true;
                            }
                            if let Some(text) = pane.surface.clipboard.take() {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        }
                    }
                    if this
                        .app
                        .toast
                        .as_ref()
                        .is_some_and(|(_, created)| created.elapsed() > Duration::from_secs(4))
                    {
                        this.app.toast = None;
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        });
        Self {
            theme: TerminasteTheme::for_settings(&app.loaded.settings, window.appearance()),
            app,
            focus,
            font: font("Menlo"),
            cell: size(px(8.), px(17.)),
            overlay: None,
            field: None,
            field_editor: CommandEditorState::new(),
            input_layout: InputLayout::default(),
            selecting_input: false,
            category: SettingsCategory::Appearance,
            find_query: String::new(),
            keybinding_search: String::new(),
            palette_index: 0,
            pane_views: HashMap::new(),
            split_drag: None,
            block_menu: None,
            output_selection: None,
            command_layouts: HashMap::new(),
            selecting_command: false,
            output_layouts: HashMap::new(),
            selecting_output: false,
            _task: task,
        }
    }

    pub(super) fn input_editor(&self) -> Option<&CommandEditorState> {
        if self.field.is_some() {
            Some(&self.field_editor)
        } else if self.overlay.is_none() {
            self.app.active_terminal().map(|pane| &pane.editor)
        } else {
            None
        }
    }
    pub(super) fn input_editor_mut(&mut self) -> Option<&mut CommandEditorState> {
        if self.field.is_some() {
            Some(&mut self.field_editor)
        } else if self.overlay.is_none() {
            self.app.active_terminal_mut().map(|pane| &mut pane.editor)
        } else {
            None
        }
    }
    pub(super) fn direct_input(&self) -> bool {
        self.overlay.is_none()
            && self
                .app
                .active_terminal()
                .is_some_and(TerminalPane::uses_raw_input)
    }
    pub(super) fn input_changed(&mut self) {
        if let Some(field) = self.field {
            let text = self.field_editor.text().to_owned();
            match field {
                Field::Find => self.find_query = text,
                Field::FontFamily => self.app.loaded.settings.font.family = text,
                Field::KeybindingSearch => self.keybinding_search = text,
                Field::Redaction(index) => {
                    self.app.loaded.settings.privacy.redaction_patterns[index] = text
                }
            }
        } else if let Some(pane) = self.app.active_terminal_mut() {
            pane.update_input();
        }
    }
    fn open_overlay(&mut self, overlay: Overlay) {
        self.overlay = Some(overlay);
        self.block_menu = None;
        self.field = None;
        if overlay == Overlay::Find {
            self.edit_field(Field::Find, self.find_query.clone());
        }
    }
    fn dismiss_overlay(&mut self) {
        self.overlay = None;
        self.field = None;
        self.block_menu = None;
    }
    fn edit_field(&mut self, field: Field, value: String) {
        self.field = Some(field);
        self.field_editor.set_text(value);
    }
    fn focus_pane(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.app.tabs.get_mut(self.app.active_tab) {
            if let Some(index) = tab.panes.iter().position(|pane| pane.id == id) {
                tab.active_pane = index;
            }
        }
        window.focus(&self.focus, cx);
    }

    fn dispatch(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::NewTab => self.app.new_tab(),
            Action::CloseTab => {
                if self.app.tabs.len() == 1
                    && self
                        .app
                        .loaded
                        .settings
                        .workspace
                        .close_last_tab_closes_window
                {
                    self.app.persist_session();
                    window.remove_window();
                    return;
                }
                self.app.close_tab(self.app.active_tab);
            }
            Action::ReopenClosedTab => self.app.reopen_closed_tab(),
            Action::NextTab => self.app.activate_relative_tab(1),
            Action::PreviousTab => self.app.activate_relative_tab(-1),
            Action::SplitRight => self.app.split_active(SplitDirection::Right),
            Action::SplitDown => self.app.split_active(SplitDirection::Down),
            Action::FocusNextPane => self.app.focus_relative_pane(1),
            Action::FocusPreviousPane => self.app.focus_relative_pane(-1),
            Action::Settings => self.open_overlay(Overlay::Settings),
            Action::Find => self.open_overlay(Overlay::Find),
            Action::CommandPalette => {
                self.open_overlay(Overlay::Actions);
                self.palette_index = 0;
            }
            Action::CopyCommand | Action::CopyOutput => {
                if let Some(pane) = self.app.active_terminal() {
                    let id = self
                        .block_menu
                        .map(|(id, _)| id)
                        .or_else(|| pane.block_focus.focused().map(|focus| focus.block_id));
                    if let Some(block) = pane
                        .model
                        .command_blocks()
                        .iter()
                        .find(|block| Some(block.id) == id)
                    {
                        let text = if action == Action::CopyCommand {
                            block.command.clone()
                        } else {
                            shell_prompt::plain_text(&block.output)
                        };
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                self.block_menu = None;
            }
            Action::CopyBlock => {
                if let Some(text) = self
                    .app
                    .active_terminal()
                    .and_then(TerminalPane::focused_block_text)
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            Action::CopySelection => self.copy(false, cx),
            Action::Paste => self.paste(cx),
            Action::SelectAll => {
                if let Some(editor) = self.input_editor_mut() {
                    editor.select_all();
                }
            }
            Action::ClearView => {
                if let Some(pane) = self.app.active_terminal_mut() {
                    pane.clear_view();
                }
            }
            Action::PreviousBlock | Action::NextBlock => {
                if let Some(pane) = self.app.active_terminal_mut() {
                    pane.navigate_block(if action == Action::PreviousBlock {
                        -1
                    } else {
                        1
                    });
                }
            }
            Action::FirstBlock => {
                if let Some(pane) = self.app.active_terminal_mut() {
                    pane.focus_first_block();
                }
            }
            Action::FocusInput => {
                if let Some(pane) = self.app.active_terminal_mut() {
                    pane.focus_input();
                }
            }
            _ => {}
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }
    fn copy(&mut self, cut: bool, cx: &mut Context<Self>) {
        if let Some(text) = self
            .output_selection
            .as_ref()
            .and_then(|(_, editor)| editor.selected_text())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
            return;
        }
        let text = self
            .input_editor()
            .and_then(|editor| editor.selected_text())
            .map(str::to_owned)
            .or_else(|| {
                self.output_selection
                    .as_ref()
                    .and_then(|(_, editor)| editor.selected_text())
                    .map(str::to_owned)
            })
            .or_else(|| {
                self.app
                    .active_terminal()
                    .map(|pane| pane.model.selected_text())
                    .filter(|text| !text.is_empty())
            });
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            if cut && !self.direct_input() {
                if let Some(editor) = self.input_editor_mut() {
                    editor.delete_selection();
                }
                self.input_changed();
            }
        }
        cx.notify();
    }
    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if self.direct_input() {
                if let Some(pane) = self.app.active_terminal_mut() {
                    pane.write_terminal(encode_paste(&text, pane.model.modes()));
                }
            } else if let Some(editor) = self.input_editor_mut() {
                editor.insert_text(&text);
                self.input_changed();
            }
        }
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if self.overlay.is_none() && !self.direct_input() {
            let columns = (self.input_layout.bounds.size.width / self.cell.width)
                .floor()
                .max(1.) as usize;
            if let Some(pane) = self.app.active_terminal_mut() {
                let suggestion = pane.history_suggestion(columns);
                pane.ghost_dismissed = Some(pane.editor.text().to_owned());
                if pane.ghost_history_request {
                    pane.dismiss_completions();
                }
                if key == "right"
                    && !modifiers.shift
                    && !modifiers.control
                    && !modifiers.alt
                    && !modifiers.platform
                {
                    if !pane.completions.is_empty() || pane.surface.completion_request.is_some() {
                        pane.dismiss_completions();
                        pane.focus_input();
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    if let Some(command) = suggestion {
                        pane.editor.set_text(command);
                        self.input_changed();
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                }
                cx.notify();
            }
        }
        if key == "escape" && (self.overlay.is_some() || self.block_menu.is_some()) {
            self.dismiss_overlay();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let supported = [
            Action::NewTab,
            Action::CloseTab,
            Action::ReopenClosedTab,
            Action::NextTab,
            Action::PreviousTab,
            Action::SplitRight,
            Action::SplitDown,
            Action::FocusNextPane,
            Action::FocusPreviousPane,
            Action::Find,
            Action::CommandPalette,
            Action::Settings,
            Action::ClearView,
            Action::PreviousBlock,
            Action::NextBlock,
            Action::FirstBlock,
            Action::FocusInput,
            Action::CopyBlock,
            Action::CopyCommand,
            Action::CopyOutput,
        ];
        let action = supported.into_iter().find(|action| {
            self.app
                .loaded
                .keybindings
                .bindings
                .get(action)
                .is_some_and(|bindings| {
                    bindings
                        .iter()
                        .any(|binding| shortcut_matches(binding, key, modifiers))
                })
        });
        if let Some(action) = action {
            self.dispatch(action, window, cx);
            cx.stop_propagation();
            return;
        }
        let primary = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control && modifiers.shift
        };
        if primary {
            match key {
                "c" => {
                    self.copy(false, cx);
                    cx.stop_propagation();
                    return;
                }
                "x" => {
                    self.copy(true, cx);
                    cx.stop_propagation();
                    return;
                }
                "v" => {
                    self.paste(cx);
                    cx.stop_propagation();
                    return;
                }
                "a" => {
                    if let Some(editor) = self.input_editor_mut() {
                        editor.select_all();
                    }
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }
        if self.overlay == Some(Overlay::Actions) {
            match key {
                "up" => self.palette_index = self.palette_index.saturating_sub(1),
                "down" => self.palette_index = (self.palette_index + 1).min(PALETTE.len() + 1),
                "enter" => {
                    self.palette_action(self.palette_index, window, cx);
                }
                _ => {}
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.overlay == Some(Overlay::Settings) && self.field.is_none() {
            return;
        }
        if self.overlay.is_none()
            && self
                .app
                .active_terminal()
                .is_some_and(|pane| pane.surface.submit_pending && pane.active_command.is_none())
        {
            cx.stop_propagation();
            return;
        }
        let alt = modifiers.alt
            && (self.app.loaded.settings.input.left_alt_is_meta
                || self.app.loaded.settings.input.right_alt_is_meta);
        let shell_mode = self.overlay.is_none()
            && self
                .app
                .active_terminal()
                .is_some_and(|pane| pane.editor.shell_bridge && pane.editor.shell_command_mode);
        if self.direct_input()
            || shell_mode
            || (self.overlay.is_none() && (modifiers.control || alt))
        {
            if let Some(code) = terminal_key(key) {
                if let Some(pane) = self.app.active_terminal_mut() {
                    pane.surface.scroll = 0;
                    pane.write_terminal(encode_key(
                        KeyInput {
                            code,
                            modifiers: Modifiers {
                                shift: modifiers.shift,
                                alt,
                                control: modifiers.control,
                                command: modifiers.platform,
                            },
                        },
                        pane.model.modes(),
                    ));
                    if !pane.uses_raw_input() {
                        pane.write_terminal(pane.bridge_key(98));
                    }
                }
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        if modifiers.platform && key != "left" && key != "right" {
            return;
        }
        let before = self
            .input_editor()
            .map(|editor| (editor.text.clone(), editor.cursor));
        let layout = self.input_layout.clone();
        let field = self.field;
        if self
            .input_editor()
            .is_some_and(|editor| !editor.preedit.is_empty())
        {
            return;
        }
        match key {
            "enter" if field.is_none() && !modifiers.shift => {
                if let Some(pane) = self.app.active_terminal_mut() {
                    if !pane.completions.is_empty() {
                        pane.accept_completion();
                    } else {
                        pane.submit_input();
                    }
                }
            }
            "tab" if field.is_none() => {
                if let Some(pane) = self.app.active_terminal_mut() {
                    if pane.completions.is_empty() {
                        pane.refresh_completions();
                    } else {
                        pane.accept_completion();
                    }
                }
            }
            "escape" if field.is_none() => {
                let vim = self.app.loaded.settings.input.vim_like_editing;
                if let Some(pane) = self.app.active_terminal_mut() {
                    if !pane.completions.is_empty() || pane.surface.completion_request.is_some() {
                        pane.dismiss_completions();
                    } else if vim && pane.surface.input_bridge {
                        pane.send_shell_editor_key(terminaste_core::KeyCode::Escape);
                        pane.write_terminal(pane.bridge_key(98));
                    }
                }
            }
            "up" | "down" if field.is_none() => {
                if let Some(pane) = self.app.active_terminal_mut() {
                    let delta = if key == "up" { -1 } else { 1 };
                    if !pane.completions.is_empty() {
                        pane.move_completion(key == "up");
                    } else if pane.surface.completion_request.is_none()
                        && !layout.move_row(&mut pane.editor, delta, modifiers.shift)
                        && !modifiers.shift
                    {
                        if pane.surface.input_bridge || pane.surface.query_bridge {
                            if key == "up" {
                                pane.show_history();
                            }
                        } else {
                            pane.recall_history(-delta);
                        }
                    }
                }
                if let Some(id) = self.app.active_terminal().map(|pane| pane.id) {
                    let pane_view = self.pane_views.entry(id).or_default();
                    pane_view.history_list.scroll_to_end();
                    if self.app.active_terminal().is_some_and(|pane| {
                        pane.history_search.is_some() || !pane.completions.is_empty()
                    }) {
                        pane_view.completions.scroll_to_bottom();
                    }
                }
            }
            _ => {
                if let Some(editor) = self.input_editor_mut() {
                    match key {
                        "backspace" => editor.backspace(),
                        "delete" => editor.delete_forward(),
                        "left" if modifiers.platform => editor.move_cursor(0, modifiers.shift),
                        "right" if modifiers.platform => {
                            editor.move_cursor(editor.text.len(), modifiers.shift)
                        }
                        "left" => editor.move_left(modifiers.shift),
                        "right" => editor.move_right(modifiers.shift),
                        "home" => editor.move_home(modifiers.shift),
                        "end" => editor.move_end(modifiers.shift),
                        "enter" if field.is_none() => editor.insert_text("\n"),
                        _ => return,
                    }
                }
            }
        }
        let after = self
            .input_editor()
            .map(|editor| (editor.text.clone(), editor.cursor));
        if before != after && !matches!(key, "enter" | "tab" | "up" | "down") {
            self.input_changed();
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn render_header(&mut self, width: Pixels, cx: &mut Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let available = (f32::from(width) - 76.).max(0.);
        let tab_width = (available / self.app.tabs.len().max(1) as f32).min(160.);
        let mut header = div()
            .flex()
            .items_center()
            .h(px(28.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.surface);
        for (index, tab) in self.app.tabs.iter().enumerate() {
            let selected = index == self.app.active_tab;
            let title = if tab_width < 28. {
                (index + 1).to_string()
            } else {
                tab.compact_title()
            };
            let tab = div()
                .id(("tab", index))
                .flex()
                .items_center()
                .h(px(24.))
                .w(px(tab_width))
                .min_w_0()
                .px(px(6.))
                .border_r_1()
                .border_color(theme.border)
                .text_color(if selected { theme.text } else { theme.muted })
                .when(selected, |tab| tab.bg(theme.surface_high))
                .hover(|tab| tab.bg(theme.surface_high))
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.app.active_tab = index;
                        this.dismiss_overlay();
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, _, cx| {
                        this.app.close_tab(index);
                        cx.notify();
                    }),
                )
                .child(div().flex_1().min_w_0().text_ellipsis().child(title))
                .when(tab_width >= 44., |tab| {
                    tab.child(
                        button(("close-tab", index), "×", theme)
                            .w(px(16.))
                            .h(px(20.))
                            .text_size(px(15.))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.app.close_tab(index);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    )
                });
            header = header.child(tab);
        }
        header
            .child(div().id("empty-tab-bar").flex_1().h_full().on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        this.app.new_tab();
                        cx.notify();
                    }
                }),
            ))
            .child(
                button("new-tab", "+", theme)
                    .w(px(24.))
                    .text_size(px(18.))
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.dispatch(Action::NewTab, window, cx)
                        }),
                    ),
            )
            .child(
                button("actions", "≡", theme)
                    .w(px(24.))
                    .text_size(px(17.))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.dispatch(Action::CommandPalette, window, cx)
                    })),
            )
            .child(
                button("settings", "⚙", theme)
                    .w(px(24.))
                    .text_size(px(15.))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.dispatch(Action::Settings, window, cx)
                    })),
            )
    }

    fn render_tree(
        &mut self,
        tree: &pane_tree::PaneTree,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match tree {
            pane_tree::PaneTree::Leaf(id) => self
                .render_pane(*id, bounds.size, window, cx)
                .into_any_element(),
            pane_tree::PaneTree::Split {
                id,
                direction,
                ratio,
                first,
                second,
            } => {
                let horizontal = *direction == SplitDirection::Right;
                let total = if horizontal {
                    bounds.size.width
                } else {
                    bounds.size.height
                };
                let available = (total - px(2.)).max(px(0.));
                let minimum = px(80.).min(available / 2.);
                let first_size =
                    (available * *ratio).clamp(minimum, (available - minimum).max(minimum));
                let mut a = bounds;
                let mut b = bounds;
                if horizontal {
                    a.size.width = first_size;
                    b.origin.x += first_size + px(2.);
                    b.size.width = available - first_size;
                } else {
                    a.size.height = first_size;
                    b.origin.y += first_size + px(2.);
                    b.size.height = available - first_size;
                }
                let id = *id;
                let direction = *direction;
                let divider = div()
                    .id(SharedString::from(format!("split-{id}")))
                    .flex_shrink_0()
                    .bg(self.theme.border)
                    .when(horizontal, |div| div.w(px(2.)).h_full().cursor_col_resize())
                    .when(!horizontal, |div| {
                        div.h(px(2.)).w_full().cursor_row_resize()
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.split_drag = Some((id, bounds, direction));
                            cx.stop_propagation();
                        }),
                    );
                div()
                    .flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .when(!horizontal, |div| div.flex_col())
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(a.size.width)
                            .h(a.size.height)
                            .child(self.render_tree(first, a, window, cx)),
                    )
                    .child(divider)
                    .child(
                        div()
                            .min_w_0()
                            .min_h_0()
                            .w(b.size.width)
                            .h(b.size.height)
                            .child(self.render_tree(second, b, window, cx)),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_pane(
        &mut self,
        id: Uuid,
        available: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let settings = self.app.loaded.settings.clone();
        let cell = self.cell;
        let Some(pane) = self.app.pane_mut(id) else {
            return div();
        };
        let interactive = pane.has_interactive_surface();
        let inset = if interactive {
            f32::from(settings.terminal.alternate_screen_padding)
        } else {
            0.
        };
        let shell_chrome = if interactive { inset * 2. } else { 18. };
        let cols = ((available.width - px(shell_chrome)) / cell.width)
            .floor()
            .max(1.) as u16;
        let rows = ((available.height - px(inset * 2.)) / cell.height)
            .floor()
            .max(1.) as u16;
        if (cols, rows) != (pane.cols, pane.rows) {
            pane.resize_for_tests(cols, rows);
        }
        if interactive {
            return self.render_grid(id, window, cx);
        }
        let prompt = PromptLayout::new(
            pane,
            usize::from(cols),
            ((available.height / cell.height) as usize / 4).max(1),
        );
        let editor_columns = usize::from(cols)
            .saturating_sub(prompt.prefix.len() + prompt.suffix.len())
            .max(1);
        let editor_rows = wrapped_rows(pane.editor.text(), editor_columns)
            .len()
            .min(6);
        let input_height = ((prompt.header.len() + editor_rows) as f32 * f32::from(cell.height)
            + 10.)
            .min(f32::from(available.height) * 0.5)
            .max(f32::from(cell.height) + 10.);
        let block_count = pane.model.command_block_count();
        let focused_block = pane.block_focus.focused().map(|focus| focus.block_id);
        let completions = pane.completions.clone();
        let selected = pane.selected_completion;
        let revision = pane.completion_revision;
        let pending_view = pane
            .active_command
            .as_ref()
            .is_some_and(|command| command.started_at.elapsed() >= Duration::from_millis(200));
        if pending_view && pane.block_focus.focused().is_none() {
            pane.select_running_block();
        }
        if !pending_view && pane.active_command.is_some() {
            pane.focus_input();
        }
        let show_input = pane.active_command.is_none() || !pending_view;
        let status = pane.pty.is_none().then(|| pane.status.clone());
        let state = self.pane_views.entry(id).or_default();
        if state.history_count != block_count {
            state.history_list.reset(block_count);
            state.history_count = block_count;
        }
        if !state.initialized {
            state.history_list.set_follow_mode(FollowMode::Tail);
            state.initialized = true;
        }
        let completion_scroll = state.completions.clone();
        if state.completion_revision != revision || state.completion_index != selected {
            completion_scroll.scroll_to_item(completions.len().saturating_sub(selected + 1));
            state.completion_revision = revision;
            state.completion_index = selected;
        }
        let active =
            self.app.active_terminal().is_some_and(|pane| pane.id == id) && self.overlay.is_none();
        let find_query = self.find_query.clone();
        let zero_state_blocks = settings.appearance.zero_state_blocks;
        let block_gap =
            if settings.appearance.block_spacing == terminaste_settings::BlockSpacing::Compact {
                6.
            } else {
                8.
            };
        let history_list = state.history_list.clone();
        let history_view = list(
            history_list,
            cx.processor(move |this, index, _window, cx| {
                let Some(block) = this
                    .app
                    .active_terminal()
                    .and_then(|pane| pane.model.command_block(index))
                else {
                    return div().h(px(0.)).into_any_element();
                };
                let mut block = block;
                if block.running && block.output.is_empty() {
                    if let Some(pane) = this.app.active_terminal() {
                        block.output = strip_running_command_echo(
                            &pane.model.running_command_output(),
                            &block.command,
                        );
                    }
                }
                if !block_matches(&block_context_text(&block), &find_query)
                    || (!zero_state_blocks && block.output.is_empty() && !block.running)
                {
                    return div().h(px(0.)).into_any_element();
                }
                div()
                    .pb(px(block_gap))
                    .child(this.render_block(
                        id,
                        &block,
                        available.width - px(12.),
                        focused_block == Some(block.id),
                        cx,
                    ))
                    .into_any_element()
            }),
        )
        .flex_1()
        .min_h_0()
        .w_full();
        let mut pane_view = div()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .p(px(4.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.focus_pane(id, window, cx);
                    if let Some(pane) = this.app.pane_mut(id) {
                        let pending = pane.active_command.as_ref().is_some_and(|command| {
                            command.started_at.elapsed() >= Duration::from_millis(200)
                        });
                        if pending {
                            pane.select_running_block();
                        } else {
                            pane.focus_input();
                        }
                    }
                    cx.stop_propagation();
                }),
            )
            .child(history_view);
        if let Some(status) = status {
            pane_view = pane_view.child(div().text_color(theme.error).child(status));
        }
        if show_input {
            let mut input = div()
                .relative()
                .flex_shrink_0()
                .w_full()
                .h(px(input_height))
                .rounded(px(10.))
                .border_1()
                .border_color(if active && self.focus.is_focused(window) {
                    theme.accent.opacity(0.68)
                } else {
                    theme.border.opacity(0.78)
                })
                .bg(theme.surface)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                        this.focus_pane(id, window, cx);
                        let index = this.input_layout.index_at(event.position);
                        if let Some(pane) = this.app.pane_mut(id) {
                            pane.focus_input();
                            if event.click_count >= 3 {
                                pane.editor.select_line_at(index);
                            } else if event.click_count == 2 {
                                pane.editor.select_word_at(index);
                            } else {
                                pane.editor.move_cursor(index, event.modifiers.shift);
                            }
                            pane.dismiss_completions();
                        }
                        this.output_selection = None;
                        this.selecting_input = true;
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(self.composer_canvas(id, prompt, active, cx));
            if !completions.is_empty() {
                let mut list = div()
                    .id(SharedString::from(format!("completions-{id}")))
                    .absolute()
                    .bottom_full()
                    .left_0()
                    .w_full()
                    .mb(px(6.))
                    .max_h(px(
                        (f32::from(available.height) - input_height - 45.).clamp(24., 360.)
                    ))
                    .overflow_y_scroll()
                    .track_scroll(&completion_scroll)
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .shadow_md()
                    .py(px(3.))
                    .occlude();
                for (index, item) in completions.iter().enumerate().rev() {
                    list = list.child(
                        button(("completion", index), item.label.replace('\n', " "), theme)
                            .w_full()
                            .h(px(24.))
                            .justify_start()
                            .px(px(8.))
                            .text_ellipsis()
                            .when(index == selected, |row| {
                                row.bg(theme.active_option).text_color(theme.text)
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.focus_pane(id, window, cx);
                                    if let Some(pane) = this.app.pane_mut(id) {
                                        pane.selected_completion = index;
                                        pane.accept_completion();
                                    }
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    );
                }
                input = input.child(gpui::deferred(list).with_priority(1));
            }
            pane_view = pane_view.child(input);
        }
        pane_view
    }

    fn composer_canvas(
        &self,
        id: Uuid,
        prompt: PromptLayout,
        active: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        let font = self.font.clone();
        let font_size = px(self.app.loaded.settings.font.size);
        let cell = self.cell;
        let theme = self.theme;
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let content = Bounds::new(
                    bounds.origin + point(px(5.), px(4.)),
                    size(
                        (bounds.size.width - px(10.)).max(px(1.)),
                        (bounds.size.height - px(8.)).max(cell.height),
                    ),
                );
                let mut background_theme = theme;
                background_theme.background = theme.surface;
                let style = TerminalTextStyle {
                    font,
                    font_size,
                    cell,
                    theme: background_theme,
                    drop_background_if_readable: true,
                };
                window.with_content_mask(Some(gpui::ContentMask { bounds: content }), |window| {
                    paint_cells(&prompt.header, content.origin, &style, window, cx);
                    let input_top = content.origin.y + cell.height * prompt.header.len() as f32;
                    paint_cells(
                        std::slice::from_ref(&prompt.prefix),
                        point(content.origin.x, input_top),
                        &style,
                        window,
                        cx,
                    );
                    paint_cells(
                        std::slice::from_ref(&prompt.suffix),
                        point(
                            content.right() - cell.width * prompt.suffix.len() as f32,
                            input_top,
                        ),
                        &style,
                        window,
                        cx,
                    );
                    let editor_bounds = Bounds::new(
                        point(
                            content.origin.x + cell.width * prompt.prefix.len() as f32,
                            input_top,
                        ),
                        size(
                            (content.size.width
                                - cell.width * (prompt.prefix.len() + prompt.suffix.len()) as f32)
                                .max(cell.width),
                            content.bottom() - input_top,
                        ),
                    );
                    let editor = view
                        .read(cx)
                        .app
                        .tabs
                        .iter()
                        .flat_map(|tab| &tab.panes)
                        .find(|pane| pane.id == id)
                        .map(|pane| pane.editor.clone());
                    let editor_focused = active
                        && window.is_window_active()
                        && view
                            .read(cx)
                            .app
                            .tabs
                            .iter()
                            .flat_map(|tab| &tab.panes)
                            .find(|pane| pane.id == id)
                            .is_some_and(|pane| pane.block_focus.focused().is_none());
                    if let Some(editor) = editor {
                        let layout =
                            paint_input(&editor, editor_bounds, &style, editor_focused, window, cx);
                        if editor_focused {
                            let columns =
                                (editor_bounds.size.width / cell.width).floor().max(1.) as usize;
                            let suggestion = view
                                .read(cx)
                                .app
                                .active_terminal()
                                .and_then(|pane| pane.history_suggestion(columns));
                            if let Some(command) = suggestion {
                                let suffix = &command[editor.text().len()..];
                                let line = window.text_system().shape_line(
                                    suffix.to_owned().into(),
                                    font_size,
                                    &[gpui::TextRun {
                                        len: suffix.len(),
                                        font: style.font.clone(),
                                        color: theme.muted,
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    }],
                                    Some(cell.width),
                                );
                                let _ = line.paint(
                                    layout.point_for(editor.cursor),
                                    cell.height,
                                    gpui::TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        }
                        if editor_focused {
                            let focus = view.read(cx).focus.clone();
                            window.handle_input(
                                &focus,
                                ElementInputHandler::new(editor_bounds, view.clone()),
                                cx,
                            );
                            view.update(cx, |this, _| this.input_layout = layout);
                        }
                    }
                });
            },
        )
        .size_full()
    }

    fn render_block(
        &mut self,
        pane_id: Uuid,
        block: &CommandBlock,
        width: Pixels,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let id = block.id;
        let status = if block.running {
            "running".to_owned()
        } else {
            block
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_default()
        };
        let has_output = !block.output.is_empty();
        let command = block.command.clone();
        let command_view = cx.entity();
        let command_style = TerminalTextStyle {
            font: self.font.clone(),
            font_size: px(self.app.loaded.settings.font.size),
            cell: self.cell,
            theme,
            drop_background_if_readable: false,
        };
        let mut command_editor = CommandEditorState::new();
        command_editor.set_text(command.clone());
        if self.selecting_command {
            if let Some((selected_id, editor)) = &self.output_selection {
                if *selected_id == id {
                    command_editor = editor.clone();
                }
            }
        }
        let mut header = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(if has_output { 24. } else { 30. }))
            .px(px(6.))
            .font_family(self.font.family.clone())
            .text_size(px(self.app.loaded.settings.font.size))
            .line_height(self.cell.height)
            .child(div().text_color(theme.muted).child("›"))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                            this.focus_pane(pane_id, window, cx);
                            if let Some(pane) = this.app.pane_mut(pane_id) {
                                pane.block_focus.focus(TerminalBlockFocus { block_id: id });
                            }
                            if let Some(layout) = this.command_layouts.get(&id) {
                                let index = layout.index_at(event.position);
                                let mut editor = CommandEditorState::new();
                                editor.set_text(command.clone());
                                if event.click_count >= 3 {
                                    editor.select_all();
                                } else if event.click_count == 2 {
                                    editor.select_word_at(index);
                                } else {
                                    editor.move_cursor(index, false);
                                }
                                this.output_selection = Some((id, editor));
                                this.selecting_command = true;
                                this.selecting_output = true;
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, cx| {
                                let layout = paint_input(
                                    &command_editor,
                                    bounds,
                                    &command_style,
                                    false,
                                    window,
                                    cx,
                                );
                                command_view.update(cx, |this, _| {
                                    this.command_layouts.insert(id, layout);
                                });
                            },
                        )
                        .w_full()
                        .h(self.cell.height),
                    ),
            );
        if let Some(duration) = block.duration_ms {
            header = header.child(
                div()
                    .text_size(px(10.))
                    .text_color(theme.muted)
                    .child(format_duration(duration)),
            );
        }
        header = header
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(if block.exit_code.is_some_and(|code| code != 0) {
                        theme.error.opacity(0.72)
                    } else {
                        theme.muted
                    })
                    .child(status),
            )
            .child(
                button(SharedString::from(format!("block-menu-{id}")), "⋯", theme)
                    .w(px(20.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                            this.focus_pane(pane_id, window, cx);
                            if let Some(pane) = this.app.pane_mut(pane_id) {
                                pane.block_focus.focus(TerminalBlockFocus { block_id: id });
                            }
                            this.block_menu = Some((id, event.position));
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        let mut card = div()
            .id(SharedString::from(format!("command-block-{id}")))
            .flex_shrink_0()
            .min_w_0()
            .rounded(px(10.))
            .border_1()
            .border_color(if selected {
                theme.accent.opacity(0.68)
            } else {
                theme.border
            })
            .bg(theme.surface)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.focus_pane(pane_id, window, cx);
                    if let Some(pane) = this.app.pane_mut(pane_id) {
                        pane.block_focus.focus(TerminalBlockFocus { block_id: id });
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(header);
        if !block.output.is_empty() {
            let output_text = block.output.clone();
            let columns = ((width - px(12.)) / self.cell.width).floor().max(1.) as usize;
            let output = shell_prompt::output_snapshot(&output_text, columns, 1_000);
            let plain_output = shell_prompt::plain_text(&output_text);
            let rows = output
                .cells
                .iter()
                .rposition(|row| row.iter().any(|cell| !cell.text.trim().is_empty()))
                .map_or(1, |row| row + 1);
            let view = cx.entity();
            let cell = self.cell;
            let font = self.font.clone();
            let font_size = px(self.app.loaded.settings.font.size);
            let mut output_theme = theme;
            output_theme.background = theme.surface;
            let style = TerminalTextStyle {
                font,
                font_size,
                cell,
                theme: output_theme,
                drop_background_if_readable: false,
            };
            card = card.child(
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .px(px(6.))
                    .py(px(3.))
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                            this.focus_pane(pane_id, window, cx);
                            if let Some(pane) = this.app.pane_mut(pane_id) {
                                pane.block_focus.focus(TerminalBlockFocus { block_id: id });
                            }
                            if let Some(layout) = this.output_layouts.get(&id) {
                                let index = layout.index_at(event.position);
                                let text = this
                                    .app
                                    .tabs
                                    .iter()
                                    .flat_map(|tab| &tab.panes)
                                    .flat_map(|pane| pane.model.command_blocks())
                                    .find(|block| block.id == id)
                                    .map(|block| shell_prompt::plain_text(&block.output))
                                    .unwrap_or_default();
                                let mut editor = CommandEditorState::new();
                                editor.set_text(text);
                                if event.click_count >= 3 {
                                    editor.select_line_at(index);
                                } else if event.click_count == 2 {
                                    editor.select_word_at(index);
                                } else {
                                    editor.move_cursor(index, false);
                                }
                                this.output_selection = Some((id, editor));
                                this.selecting_command = false;
                                this.selecting_output = true;
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, cx| {
                                let content = Bounds::new(
                                    bounds.origin,
                                    size(bounds.size.width, cell.height * rows as f32),
                                );
                                paint_cells(
                                    &output.cells[..rows],
                                    content.origin,
                                    &style,
                                    window,
                                    cx,
                                );
                                let layout = output_layout(&plain_output, content, cell);
                                if !view.read(cx).selecting_command {
                                    if let Some((selected_id, editor)) =
                                        &view.read(cx).output_selection
                                    {
                                        if *selected_id == id {
                                            if let Some(range) = editor.selection_range() {
                                                for pair in layout.positions.windows(2) {
                                                    let (index, origin) = pair[0];
                                                    let (_, next) = pair[1];
                                                    if range.contains(&index)
                                                        && origin.y == next.y
                                                        && next.x > origin.x
                                                    {
                                                        window.paint_quad(fill(
                                                            Bounds::new(
                                                                origin,
                                                                size(
                                                                    next.x - origin.x,
                                                                    cell.height,
                                                                ),
                                                            ),
                                                            theme.selection.opacity(0.75),
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                view.update(cx, |this, _| {
                                    this.output_layouts.insert(id, layout);
                                });
                            },
                        )
                        .w_full()
                        .h(cell.height * rows as f32),
                    ),
            );
        }
        card
    }

    fn render_grid(&mut self, id: Uuid, _: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let view = cx.entity();
        let font = self.font.clone();
        let cell = self.cell;
        let settings = self.app.loaded.settings.clone();
        let theme = self.theme;
        let alternate_screen = self
            .app
            .tabs
            .iter()
            .flat_map(|tab| &tab.panes)
            .find(|pane| pane.id == id)
            .is_some_and(|pane| pane.model.modes().alternate_screen);
        let surface_theme = if alternate_screen {
            if theme.background == TerminasteTheme::light().background {
                TerminasteTheme::terminal_light()
            } else {
                TerminasteTheme::dark()
            }
        } else {
            theme
        };
        let active =
            self.app.active_terminal().is_some_and(|pane| pane.id == id) && self.overlay.is_none();
        div()
            .size_full()
            .bg(surface_theme.background)
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.focus_pane(id, window, cx);
                    this.grid_mouse(id, event.position, Some(0), true, event.modifiers.shift);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    this.grid_mouse(id, event.position, Some(2), true, event.modifiers.shift);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseUpEvent, _, cx| {
                    this.grid_mouse(id, event.position, Some(2), false, event.modifiers.shift);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseUpEvent, _, cx| {
                    this.grid_mouse(id, event.position, Some(0), false, event.modifiers.shift);
                    cx.notify();
                }),
            )
            .on_scroll_wheel(
                cx.listener(move |this, event: &gpui::ScrollWheelEvent, _, cx| {
                    let cell = this.cell;
                    if let Some(pane) = this.app.pane_mut(id) {
                        pane.surface.wheel -= event.delta.pixel_delta(cell.height).y / cell.height;
                        let lines = pane.surface.wheel.trunc() as isize;
                        pane.surface.wheel -= lines as f32;
                        let col = ((event.position.x - pane.surface.origin.x) / cell.width)
                            .floor()
                            .max(0.) as usize;
                        let row = ((event.position.y - pane.surface.origin.y) / cell.height)
                            .floor()
                            .max(0.) as usize;
                        if pane.model.modes().mouse_reporting && !event.modifiers.shift {
                            pane.write_terminal(encode_mouse_wheel_sgr(
                                lines,
                                col.min(pane.cols as usize - 1),
                                row.min(pane.rows as usize - 1),
                            ));
                        } else if pane.model.modes().alternate_screen {
                            pane.write_terminal(terminaste_core::encode_alternate_scroll(
                                lines,
                                pane.model.modes(),
                            ));
                        } else {
                            pane.surface.scroll = pane
                                .surface
                                .scroll
                                .saturating_add_signed(lines)
                                .min(pane.model.scrollback_len());
                        }
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let padding = px(f32::from(settings.terminal.alternate_screen_padding));
                        let content = Bounds::new(
                            bounds.origin + point(padding, padding),
                            size(
                                (bounds.size.width - padding * 2.).max(cell.width),
                                (bounds.size.height - padding * 2.).max(cell.height),
                            ),
                        );
                        let snapshot = view.update(cx, |this, _| {
                            let pane = this.app.pane_mut(id).unwrap();
                            pane.surface.bounds = content;
                            pane.surface.origin = content.origin;
                            if pane.model.modes().alternate_screen {
                                pane.model.screen_snapshot()
                            } else {
                                pane.model.snapshot_scrolled(pane.surface.scroll)
                            }
                        });
                        let style = TerminalTextStyle {
                            font,
                            font_size: px(settings.font.size),
                            cell,
                            theme: surface_theme,
                            drop_background_if_readable: false,
                        };
                        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                            paint_cells(&snapshot.cells, content.origin, &style, window, cx);
                            for (row, cells) in snapshot.cells.iter().enumerate() {
                                for col in 0..cells.len() {
                                    let point = TerminalPoint {
                                        row: snapshot.visible_row_start + row as u64,
                                        col,
                                    };
                                    if snapshot.selection_ranges.iter().any(|range| {
                                        (point.row, point.col) >= (range.start.row, range.start.col)
                                            && (point.row, point.col)
                                                < (range.end.row, range.end.col)
                                    }) {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                content.origin
                                                    + gpui::point(
                                                        cell.width * col as f32,
                                                        cell.height * row as f32,
                                                    ),
                                                cell,
                                            ),
                                            surface_theme.selection.opacity(0.75),
                                        ));
                                    }
                                }
                            }
                            if active {
                                let cursor = content.origin
                                    + point(
                                        cell.width * snapshot.cursor_col as f32,
                                        cell.height * snapshot.cursor_row as f32,
                                    );
                                if snapshot.cursor_visible && window.is_window_active() {
                                    let cursor_bounds = match settings.appearance.cursor_style {
                                        terminaste_settings::CursorStyle::Bar => {
                                            Bounds::new(cursor, size(px(1.), cell.height))
                                        }
                                        terminaste_settings::CursorStyle::Underline => Bounds::new(
                                            cursor + point(px(0.), cell.height - px(2.)),
                                            size(cell.width, px(2.)),
                                        ),
                                        terminaste_settings::CursorStyle::Block => {
                                            Bounds::new(cursor, cell)
                                        }
                                    };
                                    window.paint_quad(fill(
                                        cursor_bounds,
                                        surface_theme.text.opacity(0.45),
                                    ));
                                }
                                let focus = view.read(cx).focus.clone();
                                window.handle_input(
                                    &focus,
                                    ElementInputHandler::new(content, view.clone()),
                                    cx,
                                );
                                view.update(cx, |this, _| {
                                    this.input_layout = InputLayout {
                                        bounds: content,
                                        positions: vec![(0, cursor)],
                                        line_height: cell.height,
                                        cell_width: cell.width,
                                    };
                                });
                            }
                        });
                    },
                )
                .size_full(),
            )
    }

    fn grid_mouse(
        &mut self,
        id: Uuid,
        position: Point<Pixels>,
        button: Option<usize>,
        pressed: bool,
        shift: bool,
    ) {
        let cell = self.cell;
        if let Some(pane) = self.app.pane_mut(id) {
            let col = ((position.x - pane.surface.origin.x) / cell.width)
                .floor()
                .max(0.) as usize;
            let row = ((position.y - pane.surface.origin.y) / cell.height)
                .floor()
                .max(0.) as usize;
            let col = col.min(pane.cols as usize - 1);
            let row = row.min(pane.rows as usize - 1);
            if pane.model.modes().mouse_reporting && !shift {
                if button.is_some() || pane.surface.selecting {
                    pane.write_terminal(encode_mouse_sgr(
                        button.unwrap_or(32) as u8,
                        col,
                        row,
                        pressed,
                    ));
                }
            } else {
                let snapshot = pane.model.snapshot_scrolled(pane.surface.scroll);
                let point = TerminalPoint {
                    row: snapshot.visible_row_start + row as u64,
                    col,
                };
                if button == Some(0) && pressed {
                    let anchor = if shift {
                        pane.model
                            .selection()
                            .map_or(point, |selection| selection.anchor)
                    } else {
                        point
                    };
                    pane.model.set_selection(anchor, point);
                } else if pane.surface.selecting {
                    let anchor = pane
                        .model
                        .selection()
                        .map_or(point, |selection| selection.anchor);
                    pane.model.set_selection(anchor, point);
                }
            }
            if button == Some(0) {
                pane.surface.selecting = pressed;
            }
        }
    }

    fn palette_action(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_overlay();
        if let Some(action) = PALETTE.get(index) {
            self.dispatch(*action, window, cx);
        } else if let Some(pane) = self.app.active_terminal_mut() {
            if index == PALETTE.len() {
                pane.refresh_completions();
            } else {
                pane.dismiss_completions();
            }
        }
        cx.notify();
    }
    fn render_palette(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let mut menu = self.modal("actions-modal", px(340.), cx).child(
            div()
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .border_b_1()
                .border_color(theme.border)
                .text_color(theme.muted)
                .child("Actions"),
        );
        for (index, action) in PALETTE.iter().enumerate() {
            let shortcuts = self
                .app
                .loaded
                .keybindings
                .bindings
                .get(action)
                .map(|bindings| bindings.join(", "))
                .unwrap_or_default();
            menu = menu.child(
                button(("palette", index), action.label(), theme)
                    .w_full()
                    .h(px(26.))
                    .px(px(10.))
                    .justify_between()
                    .when(index == self.palette_index, |row| {
                        row.bg(theme.surface_high)
                    })
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(theme.muted)
                            .child(shortcuts),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.palette_action(index, window, cx)
                    })),
            );
        }
        for (index, label) in ["Refresh completions", "Dismiss completions"]
            .into_iter()
            .enumerate()
        {
            menu = menu.child(
                button(("palette-extra", index), label, theme)
                    .w_full()
                    .h(px(26.))
                    .px(px(10.))
                    .justify_start()
                    .when(index + PALETTE.len() == self.palette_index, |row| {
                        row.bg(theme.surface_high)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.palette_action(PALETTE.len() + index, window, cx)
                    })),
            );
        }
        menu
    }
    fn modal(
        &self,
        id: &'static str,
        width: Pixels,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        div()
            .id(id)
            .w(width)
            .max_w_full()
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.dismiss_overlay();
                cx.notify();
            }))
    }
}

impl Focusable for TerminalWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

const PALETTE: [Action; 9] = [
    Action::NewTab,
    Action::ReopenClosedTab,
    Action::SplitRight,
    Action::SplitDown,
    Action::FocusNextPane,
    Action::FocusPreviousPane,
    Action::Find,
    Action::CopyBlock,
    Action::Settings,
];

fn strip_running_command_echo(output: &str, command: &str) -> String {
    let output = output.trim_start_matches(['\r', '\n']);
    let first_line_end = output.find('\n').unwrap_or(output.len());
    let first_line = &output[..first_line_end];
    if first_line.contains(command) {
        return output[first_line_end..]
            .trim_start_matches(['\r', '\n'])
            .to_owned();
    }
    output.to_owned()
}

fn output_layout(text: &str, bounds: Bounds<Pixels>, cell: Size<Pixels>) -> InputLayout {
    let rows = wrapped_rows(
        text,
        (bounds.size.width / cell.width).floor().max(1.) as usize,
    );
    let mut positions = Vec::new();
    for (row, range) in rows.iter().enumerate() {
        let origin = bounds.origin + point(px(0.), cell.height * row as f32);
        let mut x = origin.x;
        for (index, character) in text[range.clone()].char_indices() {
            positions.push((range.start + index, point(x, origin.y)));
            x += cell.width * unicode_width::UnicodeWidthChar::width(character).unwrap_or(0) as f32;
        }
        positions.push((range.end, point(x, origin.y)));
    }
    InputLayout {
        bounds,
        positions,
        line_height: cell.height,
        cell_width: cell.width,
    }
}

fn shortcut_matches(binding: &str, key: &str, modifiers: gpui::Modifiers) -> bool {
    let parts = binding.split('+').collect::<Vec<_>>();
    parts
        .last()
        .is_some_and(|part| part.eq_ignore_ascii_case(key))
        && parts.contains(&"cmd") == modifiers.platform
        && parts.contains(&"ctrl") == modifiers.control
        && parts.contains(&"shift") == modifiers.shift
        && parts.contains(&"alt") == modifiers.alt
}

fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    theme: TerminasteTheme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(24.))
        .rounded(px(3.))
        .text_color(theme.text)
        .cursor_pointer()
        .hover(move |style| style.bg(theme.surface_high))
        .child(label.into())
}

impl Render for TerminalWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.theme = TerminasteTheme::for_settings(&self.app.loaded.settings, window.appearance());
        let settings = &self.app.loaded.settings;
        let families = settings
            .font
            .family
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        self.font = font("JetBrains Mono");
        self.font.fallbacks = Some(FontFallbacks::from_fonts(
            families
                .into_iter()
                .filter(|family| family != "JetBrains Mono")
                .chain([
                    "SF Mono".to_owned(),
                    "Menlo".to_owned(),
                    "Symbols Nerd Font Mono".to_owned(),
                    "Apple Symbols".to_owned(),
                ])
                .collect(),
        ));
        self.font.weight = FontWeight(f32::from(settings.font.weight));
        self.font.features = if settings.font.ligatures {
            FontFeatures::default()
        } else {
            FontFeatures::disable_ligatures()
        };
        let font_size = px(settings.font.size);
        let font_id = window.text_system().resolve_font(&self.font);
        let advance = window
            .text_system()
            .advance(font_id, font_size, 'M')
            .expect("terminal font must have M");
        self.cell = size(
            px(f32::from(advance.width)),
            px((settings.font.size * settings.font.line_height).max(settings.font.size)),
        );
        let theme = self.theme;
        let active_id = self.app.active_terminal().map(|pane| pane.id);
        for tab in &mut self.app.tabs {
            for pane in &mut tab.panes {
                let foreground: gpui::Rgba = theme.text.into();
                let background: gpui::Rgba =
                    if theme.background == TerminasteTheme::light().background {
                        TerminasteTheme::terminal_light().background.into()
                    } else {
                        theme.background.into()
                    };
                let rgb = |color: gpui::Rgba| {
                    terminaste_core::Rgb(
                        (color.r * 255.).round() as u8,
                        (color.g * 255.).round() as u8,
                        (color.b * 255.).round() as u8,
                    )
                };
                pane.model
                    .set_default_colors(rgb(foreground), rgb(background));
                let responses = pane.model.take_responses();
                pane.write_terminal(responses);
                pane.model
                    .set_history_limit(settings.terminal.max_grid_rows);
                pane.model
                    .set_osc52_policy(match settings.terminal.clipboard_escape_policy {
                        terminaste_settings::ClipboardEscapePolicy::Deny => Osc52Policy::Deny,
                        terminaste_settings::ClipboardEscapePolicy::WriteOnly => {
                            Osc52Policy::WriteOnly
                        }
                        terminaste_settings::ClipboardEscapePolicy::ReadWrite => {
                            Osc52Policy::ReadWrite
                        }
                    });
                let focused = Some(pane.id) == active_id
                    && self.overlay.is_none()
                    && window.is_window_active();
                if focused != pane.surface.focused {
                    pane.write_terminal(encode_focus_event(focused, pane.model.modes()));
                    pane.surface.focused = focused;
                }
                if std::mem::take(&mut pane.surface.bell) && settings.terminal.audible_bell {
                    window.play_system_bell();
                }
            }
        }
        let viewport = window.viewport_size();
        let tree = self.app.tabs[self.app.active_tab].tree.clone();
        let workspace = Bounds::new(
            point(px(0.), px(28.)),
            size(viewport.width, (viewport.height - px(28.)).max(px(1.))),
        );
        let mut root =
            div()
                .relative()
                .flex()
                .flex_col()
                .size_full()
                .bg(theme.background)
                .text_color(theme.text)
                .font_family(".SystemUIFont")
                .text_size(px(12.))
                .line_height(px(18.))
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::key_down))
                .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                    this.dispatch(Action::NewTab, window, cx)
                }))
                .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                    this.dispatch(Action::CloseTab, window, cx)
                }))
                .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                    this.dispatch(Action::Settings, window, cx)
                }))
                .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(false, cx)))
                .on_action(cx.listener(|this, _: &Cut, _, cx| this.copy(true, cx)))
                .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
                .on_action(cx.listener(|this, _: &SelectAll, window, cx| {
                    this.dispatch(Action::SelectAll, window, cx)
                }))
                .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
                .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    if let Some((id, bounds, direction)) = this.split_drag {
                        let ratio = if direction == SplitDirection::Right {
                            (event.position.x - bounds.origin.x) / bounds.size.width
                        } else {
                            (event.position.y - bounds.origin.y) / bounds.size.height
                        };
                        this.app.tabs[this.app.active_tab].tree.resize(id, ratio);
                        cx.notify();
                    }
                    if this.selecting_input {
                        let index = this.input_layout.index_at(event.position);
                        if let Some(editor) = this.input_editor_mut() {
                            editor.move_cursor(index, true);
                        }
                        cx.notify();
                    }
                    if this.selecting_output {
                        if let Some((id, editor)) = &mut this.output_selection {
                            let layouts = if this.selecting_command {
                                &this.command_layouts
                            } else {
                                &this.output_layouts
                            };
                            if let Some(layout) = layouts.get(id) {
                                editor.move_cursor(layout.index_at(event.position), true);
                                cx.notify();
                            }
                        }
                    }
                    if let Some(id) = this
                        .app
                        .active_terminal()
                        .filter(|pane| pane.surface.selecting)
                        .map(|pane| pane.id)
                    {
                        this.grid_mouse(id, event.position, None, true, event.modifiers.shift);
                        cx.notify();
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.split_drag = None;
                        this.selecting_input = false;
                        this.selecting_output = false;
                        if let Some(pane) = this.app.active_terminal_mut() {
                            pane.surface.selecting = false;
                        }
                        cx.notify();
                    }),
                )
                .child(self.render_header(viewport.width, cx))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(self.render_tree(&tree, workspace, window, cx)),
                );
        if self.overlay.is_none() && !self.focus.is_focused(window) {
            window.focus(&self.focus, cx);
        }
        if let Some(overlay) = self.overlay {
            let panel = match overlay {
                Overlay::Actions => self.render_palette(cx),
                Overlay::Settings => self.render_settings(viewport, cx),
                Overlay::Find => self.render_find(cx),
            };
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .pt(px(if overlay == Overlay::Settings {
                        40.
                    } else {
                        58.
                    }))
                    .px(px(8.))
                    .flex()
                    .justify_center()
                    .items_start()
                    .child(panel),
            );
        }
        if let Some((id, position)) = self.block_menu {
            if let Some(block) = self
                .app
                .tabs
                .iter()
                .flat_map(|tab| &tab.panes)
                .flat_map(|pane| pane.model.command_blocks())
                .find(|block| block.id == id)
            {
                let mut menu = self.modal("block-menu", px(340.), cx);
                for (index, (label, text, action)) in [
                    ("Copy command", block.command.clone(), Action::CopyCommand),
                    (
                        "Copy output",
                        shell_prompt::plain_text(&block.output),
                        Action::CopyOutput,
                    ),
                    (
                        "Copy command and output",
                        block_input_and_output(&block),
                        Action::CopyBlock,
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    menu = menu.child(
                        button(("copy-block", index), label, theme)
                            .w_full()
                            .justify_start()
                            .px(px(8.))
                            .child(div().flex_1())
                            .child(
                                div().text_color(theme.muted).text_size(px(10.)).child(
                                    self.app
                                        .loaded
                                        .keybindings
                                        .bindings
                                        .get(&action)
                                        .map(|bindings| bindings.join(", "))
                                        .unwrap_or_default(),
                                ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                this.block_menu = None;
                                cx.notify();
                            })),
                    );
                }
                root = root.child(
                    div()
                        .absolute()
                        .left(position.x.min((viewport.width - px(344.)).max(px(0.))))
                        .top(position.y.min((viewport.height - px(82.)).max(px(0.))))
                        .child(menu),
                );
            }
        }
        if let Some((message, _)) = &self.app.toast {
            root = root.child(
                div()
                    .absolute()
                    .bottom(px(12.))
                    .right(px(12.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .shadow_md()
                    .child(message.clone()),
            );
        }
        root
    }
}
