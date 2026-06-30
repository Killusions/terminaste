use super::*;
use terminaste_core::{
    encode_focus_event, encode_key, encode_mouse_sgr, encode_mouse_wheel_sgr, KeyCode, KeyInput,
    Modifiers, Osc52Policy, TerminalCellWidth, TerminalPoint,
};

#[derive(Default)]
pub(super) struct SurfaceState {
    pub shell_id: Option<String>,
    pub parent_shells: Vec<ShellContext>,
    pub prompt_row: u64,
    pub last_sequence: u64,
    pub composing: String,
    pub focused: bool,
    pub active: bool,
    pub scroll: usize,
    pub wheel: f32,
    pub clipboard: Option<String>,
    pub input_bridge: bool,
    pub query_bridge: bool,
    pub control_keys: bool,
    pub isolated_shell: bool,
    pub input_revision: u64,
    pub completion_request: Option<u64>,
    pub completion_cursors: Vec<usize>,
    pub completion_rect: Option<egui::Rect>,
    pub submit_pending: bool,
    pub prompt_ansi: Option<String>,
    pub right_prompt_ansi: Option<String>,
    pub prompt_cols: Option<u16>,
    pub composer_active: bool,
    pub history_height: f32,
}

pub(super) struct ShellContext {
    pub id: String,
    pub sequence: u64,
    pub revision: u64,
    pub input_bridge: bool,
    pub query_bridge: bool,
    pub control_keys: bool,
    pub isolated_shell: bool,
}

impl TerminalPane {
    pub(super) fn has_interactive_surface(&self) -> bool {
        self.model.modes().alternate_screen || self.model.running_command_uses_terminal_surface()
    }

    pub(super) fn process_terminal_bytes(&mut self, bytes: &[u8]) {
        // Deliver metadata at its byte boundary, before subsequent output is parsed.
        for byte in bytes {
            if self.suppress_prompt_output && self.prompt_capture.len() < 16 * 1024 {
                self.prompt_capture.push(*byte);
            }
            for event in self.model.process_bytes(std::slice::from_ref(byte)) {
                self.handle_terminal_event(event);
            }
        }
        if self.surface.submit_pending && self.active_command.is_none() {
            self.model.discard_pending_output();
        }
        let responses = self.model.take_responses();
        self.write_terminal(responses);
    }

    pub(super) fn write_terminal(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            if let Some(pty) = &self.pty {
                let _ = pty.tx.send(PtyCommand::Write(bytes));
            }
        }
    }

    pub(super) fn render_surface(
        &mut self,
        ui: &mut egui::Ui,
        active: bool,
        settings: &Settings,
        theme: &TerminasteTheme,
        query: &str,
    ) {
        self.model
            .set_osc52_policy(match settings.terminal.clipboard_escape_policy {
                ClipboardEscapePolicy::Deny => Osc52Policy::Deny,
                ClipboardEscapePolicy::WriteOnly => Osc52Policy::WriteOnly,
                ClipboardEscapePolicy::ReadWrite => Osc52Policy::ReadWrite,
            });
        self.model
            .set_history_limit(settings.terminal.max_grid_rows);
        if let Some(text) = self.surface.clipboard.take() {
            ui.ctx().copy_text(text);
        }
        if self.integration_ready && !self.has_interactive_surface() {
            if active && !self.surface.composer_active {
                self.editor.focus_requested = true;
            }
            self.surface.composer_active = active;
            if self.surface.focused {
                self.write_terminal(encode_focus_event(false, self.model.modes()));
                self.surface.focused = false;
                let id = ui.make_persistent_id(("terminal", self.id));
                ui.memory_mut(|memory| memory.surrender_focus(id));
            }
            self.surface.active = false;
            self.render_composer(ui, active, settings, theme, query);
            return;
        }
        self.surface.composer_active = false;
        let font = FontId::monospace(settings.font.size);
        let pixels_per_point = ui.ctx().pixels_per_point();
        let cell = ui.fonts(|fonts| {
            egui::vec2(
                (fonts.glyph_width(&font, 'M') * pixels_per_point).ceil() / pixels_per_point,
                fonts
                    .row_height(&font)
                    .max(settings.font.size * settings.font.line_height),
            )
        });
        let alternate = self.model.modes().alternate_screen;
        let interactive = self.has_interactive_surface();
        let padding = if alternate {
            f32::from(settings.terminal.alternate_screen_padding)
        } else if interactive {
            0.0
        } else {
            6.0
        };
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        let id = ui.make_persistent_id(("terminal", self.id));
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let grid_rect = rect.shrink(padding);
        let cols = (grid_rect.width() / cell.x).floor().max(1.0) as u16;
        let rows = (grid_rect.height() / cell.y).floor().max(1.0) as u16;
        if (cols, rows) != (self.cols, self.rows) {
            self.resize_for_tests(cols, rows);
        }
        if response.clicked()
            || (active && (!self.surface.active || ui.memory(|memory| memory.focused().is_none())))
        {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        self.surface.active = active;
        let focused =
            active && ui.memory(|memory| memory.has_focus(id)) && ui.input(|input| input.focused);
        ui.memory_mut(|memory| memory.set_focus_lock_filter(id, event_filter_all()));
        if focused != self.surface.focused {
            self.write_terminal(encode_focus_event(focused, self.model.modes()));
            self.surface.focused = focused;
        }
        if focused {
            self.handle_surface_input(ui, settings);
        }
        let snapshot = if alternate {
            self.model.screen_snapshot()
        } else {
            self.model.snapshot_scrolled(self.surface.scroll)
        };
        let cursor_end = snapshot.cursor_row + 1;
        let last_content = snapshot
            .visible_lines
            .iter()
            .rposition(|line| !line.is_empty())
            .map_or(1, |row| row + 1);
        let end = if alternate || self.surface.scroll > 0 {
            snapshot.rows
        } else {
            cursor_end.max(last_content)
        };
        let start = if self.integration_ready && !alternate && self.surface.scroll == 0 {
            self.surface
                .prompt_row
                .saturating_sub(snapshot.visible_row_start)
                .min(end.saturating_sub(1) as u64) as usize
        } else {
            0
        };
        let live_height = (end - start) as f32 * cell.y;
        let origin = if self.integration_ready && !alternate && self.surface.scroll == 0 {
            egui::pos2(
                grid_rect.left(),
                (grid_rect.bottom() - live_height).max(grid_rect.top()),
            )
        } else {
            grid_rect.min
        };
        if origin.y > grid_rect.top() + cell.y {
            let history_rect = egui::Rect::from_min_max(
                grid_rect.min,
                egui::pos2(grid_rect.right(), origin.y - 4.0),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(history_rect), |ui| {
                ui.set_clip_rect(history_rect);
                egui::ScrollArea::vertical()
                    .id_salt((self.id, "blocks"))
                    .stick_to_bottom(true)
                    .max_height(history_rect.height())
                    .show(ui, |ui| {
                        for block in snapshot.blocks.iter().filter(|block| !block.running) {
                            thin_separator(ui, theme.border);
                            let target = TerminalBlockFocus { block_id: block.id };
                            let show_actions = self.block_focus.focused() == Some(target)
                                || self.hovered_block == Some(block.id);
                            self.render_command_input_block(
                                ui,
                                block,
                                theme,
                                font.clone(),
                                query,
                                show_actions,
                            );
                            self.render_command_output_block(ui, block, theme, font.clone(), query);
                        }
                    });
            });
        }
        let painter = ui.painter_at(grid_rect);
        let matches = if query.is_empty() {
            Vec::new()
        } else {
            self.model.search(query)
        };
        for row in start..end {
            for (col, value) in snapshot.cells[row].iter().enumerate() {
                let pos = origin + egui::vec2(col as f32 * cell.x, (row - start) as f32 * cell.y);
                let mut bounds = egui::Rect::from_min_size(pos, cell);
                if interactive {
                    if col + 1 == snapshot.cols {
                        bounds.max.x = grid_rect.right();
                    }
                    if row + 1 == snapshot.rows {
                        bounds.max.y = grid_rect.bottom();
                    }
                }
                let color = |rgb: terminaste_core::Rgb| Color32::from_rgb(rgb.0, rgb.1, rgb.2);
                let mut foreground = value.style.foreground.map(color).unwrap_or(theme.text);
                let mut background = value
                    .style
                    .background
                    .map(color)
                    .unwrap_or(theme.background);
                if value.style.inverse {
                    std::mem::swap(&mut foreground, &mut background);
                }
                if value.style.dim {
                    foreground = foreground.gamma_multiply(0.6);
                }
                if background != theme.background {
                    painter.rect_filled(bounds, 0.0, background);
                }
                let point = TerminalPoint {
                    row: snapshot.visible_row_start + row as u64,
                    col,
                };
                let contains = |range: &terminaste_core::TerminalRange| {
                    (point.row, point.col) >= (range.start.row, range.start.col)
                        && (point.row, point.col) < (range.end.row, range.end.col)
                };
                if matches.iter().any(contains) {
                    painter.rect_filled(bounds, 0.0, theme.warning.gamma_multiply(0.5));
                }
                if snapshot.selection_ranges.iter().any(contains) {
                    painter.rect_filled(bounds, 0.0, theme.accent.gamma_multiply(0.35));
                }
                if value.width != TerminalCellWidth::Spacer && !value.style.hidden {
                    let text = if value.secret { "•" } else { &value.text };
                    if !text.trim().is_empty() {
                        painter.text(pos, egui::Align2::LEFT_TOP, text, font.clone(), foreground);
                    }
                    if value.style.bold && !text.trim().is_empty() {
                        painter.text(
                            pos + egui::vec2(0.4, 0.0),
                            egui::Align2::LEFT_TOP,
                            text,
                            font.clone(),
                            foreground,
                        );
                    }
                    for y in [
                        value.style.underline.then_some(bounds.bottom() - 2.0),
                        value.style.strikethrough.then_some(bounds.center().y),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        painter.line_segment(
                            [egui::pos2(bounds.left(), y), egui::pos2(bounds.right(), y)],
                            Stroke::new(cell.x * 0.15, foreground),
                        );
                    }
                }
            }
        }
        let cursor = origin
            + egui::vec2(
                snapshot.cursor_col.min(snapshot.cols - 1) as f32 * cell.x,
                snapshot.cursor_row.saturating_sub(start) as f32 * cell.y,
            );
        if focused && snapshot.cursor_visible {
            let cursor_rect = egui::Rect::from_min_size(cursor, cell);
            match settings.appearance.cursor_style {
                terminaste_settings::CursorStyle::Bar => {
                    painter.rect_filled(
                        egui::Rect::from_min_size(cursor, egui::vec2(cell.x * 0.15, cell.y)),
                        0.0,
                        theme.text,
                    );
                }
                terminaste_settings::CursorStyle::Underline => {
                    painter.line_segment(
                        [cursor_rect.left_bottom(), cursor_rect.right_bottom()],
                        Stroke::new(cell.x * 0.15, theme.text),
                    );
                }
                terminaste_settings::CursorStyle::Block => {
                    painter.rect_stroke(
                        cursor_rect,
                        0.0,
                        Stroke::new(cell.x * 0.15, theme.text),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            ui.output_mut(|output| {
                output.ime = Some(egui::output::IMEOutput {
                    rect: grid_rect,
                    cursor_rect,
                })
            });
            if !self.surface.composing.is_empty() {
                painter.text(
                    cursor,
                    egui::Align2::LEFT_TOP,
                    &self.surface.composing,
                    font,
                    theme.accent,
                );
            }
        }
        if let Some(pos) = response
            .interact_pointer_pos()
            .filter(|pos| grid_rect.contains(*pos) && pos.y >= origin.y)
        {
            let col = ((pos.x - origin.x) / cell.x)
                .floor()
                .clamp(0.0, f32::from(cols - 1)) as usize;
            let row = (((pos.y - origin.y) / cell.y).floor().max(0.0) as usize + start)
                .min(snapshot.rows - 1);
            if snapshot.modes.mouse_reporting && !ui.input(|input| input.modifiers.shift) {
                for (button, code) in [
                    (egui::PointerButton::Primary, 0),
                    (egui::PointerButton::Middle, 1),
                    (egui::PointerButton::Secondary, 2),
                ] {
                    if ui.input(|input| input.pointer.button_pressed(button)) {
                        self.write_terminal(encode_mouse_sgr(code, col, row, true));
                    }
                    if ui.input(|input| input.pointer.button_released(button)) {
                        self.write_terminal(encode_mouse_sgr(code, col, row, false));
                    }
                }
                if response.dragged() {
                    self.write_terminal(encode_mouse_sgr(32, col, row, true));
                }
            } else {
                let point = TerminalPoint {
                    row: snapshot.visible_row_start + row as u64,
                    col,
                };
                if response.double_clicked() {
                    let text = &snapshot.visible_lines[row];
                    let byte = char_to_byte_index(text, col);
                    let (start, end) = word_bounds(text, byte);
                    self.model.set_selection(
                        TerminalPoint {
                            col: text[..start].chars().count(),
                            ..point
                        },
                        TerminalPoint {
                            col: text[..end].chars().count(),
                            ..point
                        },
                    );
                } else if response.drag_started() || response.clicked() {
                    let anchor = if ui.input(|input| input.modifiers.shift) {
                        self.model
                            .selection()
                            .map_or(point, |selection| selection.anchor)
                    } else {
                        point
                    };
                    self.model.set_selection(anchor, point);
                }
                if response.dragged() {
                    let anchor = self
                        .model
                        .selection()
                        .map_or(point, |selection| selection.anchor);
                    self.model.set_selection(anchor, point);
                }
            }
        }
        if response.hovered()
            && ui.input(|input| {
                input
                    .pointer
                    .hover_pos()
                    .is_some_and(|pos| pos.y >= origin.y)
            })
        {
            self.surface.wheel -= ui.input(|input| input.smooth_scroll_delta.y) / cell.y;
            let lines = self.surface.wheel.trunc() as isize;
            self.surface.wheel -= lines as f32;
            if snapshot.modes.mouse_reporting {
                let pos = ui
                    .input(|input| input.pointer.hover_pos())
                    .unwrap_or(origin);
                let col = ((pos.x - origin.x) / cell.x).max(0.0) as usize;
                let row = ((pos.y - origin.y) / cell.y).max(0.0) as usize + start;
                self.write_terminal(encode_mouse_wheel_sgr(
                    lines,
                    col.min(snapshot.cols - 1),
                    row.min(snapshot.rows - 1),
                ));
            } else if alternate {
                self.write_terminal(terminaste_core::encode_alternate_scroll(
                    lines,
                    snapshot.modes,
                ));
            } else {
                self.surface.scroll = self
                    .surface
                    .scroll
                    .saturating_add_signed(-lines)
                    .min(self.model.scrollback_len());
            }
        }
        response.context_menu(|ui| {
            if ui.button("Copy selection").clicked() {
                ui.ctx().copy_text(self.model.selected_text());
                ui.close_menu();
            }
        });
    }

    fn handle_surface_input(&mut self, ui: &egui::Ui, settings: &Settings) {
        let events = ui.input(|input| input.events.clone());
        self.forward_terminal_events(ui, settings, events);
    }

    pub(super) fn forward_terminal_events(
        &mut self,
        ui: &egui::Ui,
        settings: &Settings,
        events: Vec<egui::Event>,
    ) {
        let modes = self.model.modes();
        let mut bytes = Vec::new();
        for event in events {
            if matches!(
                event,
                egui::Event::Text(_)
                    | egui::Event::Paste(_)
                    | egui::Event::Key { pressed: true, .. }
            ) {
                self.surface.scroll = 0;
            }
            match event {
                egui::Event::Text(text) => {
                    if self.surface.composing.is_empty()
                        && !ui.input(|input| {
                            input.modifiers.ctrl
                                || input.modifiers.mac_cmd
                                || (input.modifiers.alt && settings.input.left_alt_is_meta)
                        })
                    {
                        for ch in text.chars() {
                            bytes.extend(encode_key(
                                KeyInput {
                                    code: KeyCode::Char(ch),
                                    modifiers: Modifiers::default(),
                                },
                                modes,
                            ));
                        }
                    }
                }
                egui::Event::Paste(text) => bytes.extend(encode_paste(&text, modes)),
                egui::Event::Copy => ui.ctx().copy_text(self.model.selected_text()),
                egui::Event::Ime(egui::ImeEvent::Preedit(text)) => self.surface.composing = text,
                egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    self.surface.composing.clear();
                    bytes.extend(text.into_bytes());
                }
                egui::Event::Ime(egui::ImeEvent::Disabled) => self.surface.composing.clear(),
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if self.surface.composing.is_empty() => {
                    let alt = modifiers.alt
                        && (settings.input.left_alt_is_meta || settings.input.right_alt_is_meta);
                    if modifiers.mac_cmd {
                        continue;
                    }
                    if let Some(code) = terminal_key(key, modifiers.ctrl || alt) {
                        bytes.extend(encode_key(
                            KeyInput {
                                code,
                                modifiers: Modifiers {
                                    shift: modifiers.shift,
                                    alt,
                                    control: modifiers.ctrl,
                                    command: false,
                                },
                            },
                            modes,
                        ));
                    }
                }
                _ => {}
            }
        }
        self.write_terminal(bytes);
    }

    pub(super) fn sync_shell_editor(&mut self) {
        if self.surface.submit_pending || self.active_command.is_some() {
            return;
        }
        self.surface.input_revision += 1;
        let cursor = self.editor.text()[..self.editor.cursor()].chars().count();
        let encoded = base64::prelude::BASE64_STANDARD.encode(self.editor.text());
        let payload = format!("{cursor};{};{encoded}", self.surface.input_revision);
        if payload.len() <= 65536 {
            let mut bytes = self.bridge_key(99);
            bytes.extend(format!("{:08x}{payload}", payload.len()).into_bytes());
            bytes.extend(self.bridge_key(98));
            self.write_terminal(bytes);
        }
    }

    pub(super) fn bridge_key(&self, key: u8) -> Vec<u8> {
        if self.surface.control_keys {
            vec![
                0x18,
                match key {
                    99 => b'r',
                    98 => b'f',
                    97 => b'o',
                    96 => b'p',
                    95 => b'n',
                    94 => b'h',
                    _ => unreachable!(),
                },
            ]
        } else {
            format!("\x1b[{key}~").into_bytes()
        }
    }

    pub(super) fn request_shell_completions(&mut self, history: bool) {
        if self.surface.query_bridge && !self.surface.input_bridge {
            self.surface.input_revision += 1;
            let cursor = self.editor.text()[..self.editor.cursor()].chars().count();
            let encoded = base64::prelude::BASE64_STANDARD.encode(self.editor.text());
            let kind = if history { "history" } else { "complete" };
            self.write_terminal(
                format!(
                    "__terminaste_query {cursor} {} '{encoded}' {kind}\r",
                    self.surface.input_revision
                )
                .into_bytes(),
            );
        } else {
            self.sync_shell_editor();
            self.write_terminal(self.bridge_key(if history { 94 } else { 97 }));
        }
        self.surface.completion_request = Some(self.surface.input_revision);
    }

    pub(super) fn send_shell_editor_key(&self, code: KeyCode) {
        self.write_terminal(encode_key(
            KeyInput {
                code,
                modifiers: Modifiers::default(),
            },
            self.model.modes(),
        ));
    }
}

fn terminal_key(key: Key, modified: bool) -> Option<KeyCode> {
    Some(match key {
        Key::Enter => KeyCode::Enter,
        Key::Tab => KeyCode::Tab,
        Key::Escape => KeyCode::Escape,
        Key::Backspace => KeyCode::Backspace,
        Key::Delete => KeyCode::Delete,
        Key::Insert => KeyCode::Insert,
        Key::ArrowUp => KeyCode::Up,
        Key::ArrowDown => KeyCode::Down,
        Key::ArrowLeft => KeyCode::Left,
        Key::ArrowRight => KeyCode::Right,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::PageUp => KeyCode::PageUp,
        Key::PageDown => KeyCode::PageDown,
        Key::F1 => KeyCode::Function(1),
        Key::F2 => KeyCode::Function(2),
        Key::F3 => KeyCode::Function(3),
        Key::F4 => KeyCode::Function(4),
        Key::F5 => KeyCode::Function(5),
        Key::F6 => KeyCode::Function(6),
        Key::F7 => KeyCode::Function(7),
        Key::F8 => KeyCode::Function(8),
        Key::F9 => KeyCode::Function(9),
        Key::F10 => KeyCode::Function(10),
        Key::F11 => KeyCode::Function(11),
        Key::F12 => KeyCode::Function(12),
        Key::Space if modified => KeyCode::Char(' '),
        _ if modified && key.name().chars().count() == 1 => {
            KeyCode::Char(key.name().to_ascii_lowercase().chars().next()?)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(
        app: &mut TerminasteApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        render_size(app, ctx, events, egui::vec2(800.0, 500.0))
    }

    fn render_size(
        app: &mut TerminasteApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| {
                app.handle_shortcuts(ctx);
                app.render_terminal_panel(ctx);
            },
        )
    }

    #[test]
    fn interactive_background_fills_the_window_below_the_tabs() {
        for size in [egui::vec2(800.0, 503.0), egui::vec2(443.0, 999.0)] {
            let mut app = TerminasteApp::headless_for_tests(Settings::default());
            app.send_active_pane_output_for_tests(b"\x1b[?1049h");
            let ctx = egui::Context::default();
            render_size(&mut app, &ctx, Vec::new(), size);
            let pane = app.active_terminal().unwrap();
            let fill = format!(
                "\x1b[48;2;50;60;70m\x1b[H{}",
                " ".repeat(usize::from(pane.cols) * usize::from(pane.rows))
            );
            app.send_active_pane_output_for_tests(fill.as_bytes());
            let output = render_size(&mut app, &ctx, Vec::new(), size);
            let background = Color32::from_rgb(50, 60, 70);
            let mut bounds = egui::Rect::NOTHING;
            for shape in &output.shapes {
                if let egui::Shape::Rect(rect) = &shape.shape {
                    if rect.fill == background {
                        assert!(shape.clip_rect.contains_rect(rect.rect));
                        bounds = bounds.union(rect.rect);
                    }
                }
            }
            assert_eq!(bounds.min, egui::pos2(0.0, 28.0));
            assert_eq!(bounds.max, size.to_pos2());
        }
    }

    #[test]
    fn four_tabs_fit_and_align_in_a_288_point_window() {
        for width in [288.0, 160.0] {
            let mut app = TerminasteApp::headless_for_tests(Settings::default());
            for _ in 0..3 {
                app.new_tab_for_tests();
            }
            for (index, tab) in app.tabs.iter_mut().enumerate() {
                tab.panes[0].title = ["a", "b", "c", "d"][index].to_owned();
            }
            app.send_active_pane_output_for_tests(&integration_frame_for_tests(
                "ready",
                "{\"input_bridge\":true}",
            ));
            let ctx = egui::Context::default();
            let size = egui::vec2(width, 500.0);
            let output = render_size(&mut app, &ctx, Vec::new(), size);
            let labels = if width == 288.0 {
                ["a", "b", "c", "d"]
            } else {
                ["1", "2", "3", "4"]
            };
            let mut text_y: Option<f32> = None;
            let mut previous_right = 0.0;
            for label in labels {
                let bounds = visible_text(&output, label).expect("all four tabs must fit");
                assert!(bounds.left() >= previous_right);
                assert!(bounds.right() < width - 70.0);
                let top = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == label => {
                            Some(text.pos.y)
                        }
                        _ => None,
                    })
                    .unwrap();
                if let Some(y) = text_y {
                    assert!((top - y).abs() < 1.0);
                }
                text_y = Some(top);
                previous_right = bounds.right();
            }
            let typed = render_size(
                &mut app,
                &ctx,
                vec![egui::Event::Text("echo".to_owned())],
                size,
            );
            assert!(
                visible_text(&typed, "echo").is_some(),
                "narrow window must keep the editor visible"
            );
        }
    }

    fn visible_text(output: &egui::FullOutput, expected: &str) -> Option<egui::Rect> {
        fn find(shape: &egui::Shape, clip: egui::Rect, expected: &str) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == expected => {
                    let bounds = shape.visual_bounding_rect();
                    clip.contains_rect(bounds).then_some(bounds)
                }
                egui::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| find(shape, clip, expected))
                }
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, shape.clip_rect, expected))
    }

    fn visible_text_containing(output: &egui::FullOutput, expected: &str) -> Option<egui::Rect> {
        fn find(shape: &egui::Shape, clip: egui::Rect, expected: &str) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text.contains(expected) => {
                    let bounds = shape.visual_bounding_rect();
                    let visible = clip.intersect(bounds);
                    visible.is_positive().then_some(visible)
                }
                egui::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| find(shape, clip, expected))
                }
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, shape.clip_rect, expected))
    }

    #[test]
    fn editor_is_visible_at_startup_and_with_oversized_prompt_and_history() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        let ctx = egui::Context::default();
        let initial = render(&mut app, &ctx, Vec::new());
        assert!(
            initial.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, stroke } =>
                    stroke.width == 1.5
                        && points[0].x == points[1].x
                        && points[0].y > 350.0
                        && shape.clip_rect.contains(points[0])
                        && shape.clip_rect.contains(points[1]),
                _ => false,
            }),
            "startup editor caret must be visible"
        );
        assert!(app.active_terminal().unwrap().editor_has_focus);

        let pane = app.active_terminal_mut().unwrap();
        pane.surface.prompt_ansi = Some(format!(
            "\x1b[32m{}\x1b[0m",
            "prompt theme with lots of metrics ".repeat(100)
        ));
        pane.surface.right_prompt_ansi = Some("right prompt ".repeat(80));
        for index in 0..20 {
            pane.model.start_integrated_command(format!("echo {index}"));
            pane.model.process_bytes("output\n".repeat(20).as_bytes());
            pane.model.finish_running_command(0);
        }
        render(&mut app, &ctx, Vec::new());
        let typed = render(
            &mut app,
            &ctx,
            vec![egui::Event::Text("visible immediately".to_owned())],
        );
        let text = typed
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "visible immediately" => {
                    let visible = shape
                        .shape
                        .visual_bounding_rect()
                        .intersect(shape.clip_rect);
                    visible.is_positive().then_some(visible)
                }
                _ => None,
            })
            .expect("typing must paint in the same frame within the viewport");
        assert!(text.top() > 350.0);
        assert!(text.bottom() <= 500.0);
    }

    #[test]
    fn prompt_prefix_and_suffix_share_the_editor_row_without_overlap_or_divider() {
        for width in [800.0, 288.0] {
            let mut app = TerminasteApp::headless_for_tests(Settings::default());
            app.send_active_pane_output_for_tests(&integration_frame_for_tests(
                "ready",
                "{\"input_bridge\":true}",
            ));
            let pane = app.active_terminal_mut().unwrap();
            pane.surface.prompt_ansi = Some("\x1b[32mwork\n> ".to_owned());
            pane.surface.right_prompt_ansi = Some("main".to_owned());
            pane.editor.set_text("echo".to_owned());
            let ctx = egui::Context::default();
            let output = render_size(&mut app, &ctx, Vec::new(), egui::vec2(width, 500.0));
            let prefix = visible_text(&output, "> ").unwrap();
            let input = visible_text(&output, "echo").unwrap();
            let suffix = visible_text(&output, "main").unwrap();
            assert!(prefix.right() <= input.left());
            assert!(input.right() < suffix.left());
            let positions = ["> ", "echo", "main"].map(|text| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(shape) if shape.galley.job.text == text => {
                            Some(shape.pos.y)
                        }
                        _ => None,
                    })
                    .unwrap()
            });
            assert!(positions
                .windows(2)
                .all(|pair| (pair[0] - pair[1]).abs() < 0.1));
            let border = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == app.theme.input => Some(rect.rect),
                    _ => None,
                })
                .unwrap();
            for bounds in [
                prefix,
                input,
                suffix,
                visible_text(&output, "work").unwrap(),
            ] {
                assert!(border.shrink(1.0).contains_rect(bounds));
            }
            assert!(!output.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, .. } =>
                    points[0].y == points[1].y && border.contains(points[0]),
                _ => false,
            }));
        }
    }

    #[test]
    fn bridged_submission_sends_enter_without_pasting_the_command_again() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        render(&mut app, &ctx, vec![egui::Event::Text("hh".to_owned())]);
        commands.try_iter().for_each(drop);
        render(&mut app, &ctx, vec![key(Key::Enter, egui::Modifiers::NONE)]);
        let writes = commands
            .try_iter()
            .filter_map(|command| match command {
                PtyCommand::Write(bytes) => Some(bytes),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 2);
        assert!(writes[0].starts_with(b"\x1b[99~"));
        assert_eq!(writes[1], b"\r");
        let pending = app.active_pane_snapshot_for_tests().unwrap();
        assert_eq!(pending.blocks.len(), 1);
        assert_eq!(pending.blocks[0].command, "hh");
        assert!(app.active_terminal().unwrap().editor.text().is_empty());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-start",
            "{\"command\":\"hh\"}",
        ));
        app.send_active_pane_output_for_tests(b"zsh: command not found: hh\r\n");
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-end",
            "{\"command\":\"hh\",\"exit_code\":127}",
        ));
        app.send_active_pane_output_for_tests(&integration_frame_for_tests("prompt-start", "{}"));
        app.send_active_pane_output_for_tests(b"custom prompt and metrics\r\n");
        app.send_active_pane_output_for_tests(&integration_frame_for_tests("prompt-end", "{}"));
        let revision = app.active_terminal().unwrap().surface.input_revision;
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "editor-ready",
            &format!("{{\"revision\":{revision}}}"),
        ));
        let snapshot = app.active_pane_snapshot_for_tests().unwrap();
        assert_eq!(snapshot.blocks.len(), 1);
        assert_eq!(snapshot.blocks[0].command, "hh");
        assert_eq!(snapshot.blocks[0].output, "zsh: command not found: hh");
        assert_eq!(snapshot.blocks[0].exit_code, Some(127));
        assert!(!snapshot.blocks[0].running);
        render(&mut app, &ctx, Vec::new());
        let output = render(&mut app, &ctx, vec![egui::Event::Text("next".to_owned())]);
        assert!(visible_text(&output, "next").is_some());
    }

    #[test]
    fn enter_shows_the_final_block_immediately_and_discards_shell_echo() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.active_pane_fake_pty_for_tests().unwrap();
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        let command = "printf 'a  b 界'";
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::Text(command.to_owned()),
                key(Key::Enter, egui::Modifiers::NONE),
            ],
        );
        let pending = app.active_pane_snapshot_for_tests().unwrap().blocks[0].clone();
        assert_eq!(pending.command, command);
        assert!(app.active_terminal().unwrap().editor.text().is_empty());
        app.send_active_pane_output_for_tests(command.as_bytes());
        let output = render(&mut app, &ctx, Vec::new());
        assert!(visible_text(&output, command).is_some());
        assert!(app.active_pane_snapshot_for_tests().unwrap().blocks[0]
            .output
            .is_empty());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-start",
            &serde_json::json!({"command":command}).to_string(),
        ));
        app.send_active_pane_output_for_tests("a  b 界\r\n".as_bytes());
        let blocks = app.active_pane_snapshot_for_tests().unwrap().blocks;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id, pending.id);
        assert_eq!(blocks[0].output, "a  b 界");
    }

    #[test]
    fn nested_shells_restore_parent_revisions_and_completion_capabilities() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.active_pane_fake_pty_for_tests().unwrap();
        let pane = app.active_terminal_mut().unwrap();
        let event = |pane: &mut TerminalPane,
                     shell: &str,
                     sequence: u64,
                     name: &str,
                     data: serde_json::Value| {
            let payload = BASE64_URL_SAFE_NO_PAD.encode(serde_json::to_vec(&serde_json::json!({"session":Uuid::nil().to_string(),"shell_id":shell,"sequence":sequence,"data":data})).unwrap());
            pane.apply_integration_event(name.to_owned(), payload);
        };
        event(
            pane,
            "parent",
            1,
            "ready",
            serde_json::json!({"input_bridge":true}),
        );
        pane.surface.input_revision = 90;
        event(
            pane,
            "parent",
            2,
            "command-start",
            serde_json::json!({"command":"sudo -i"}),
        );
        event(
            pane,
            "child",
            1,
            "ready",
            serde_json::json!({"query_bridge":true,"isolated":true}),
        );
        assert_eq!(pane.surface.input_revision, 0);
        assert!(!pane.surface.input_bridge);
        assert!(pane.surface.query_bridge);
        event(
            pane,
            "child",
            200,
            "command-start",
            serde_json::json!({"command":"exit"}),
        );
        event(
            pane,
            "parent",
            3,
            "command-end",
            serde_json::json!({"command":"sudo -i","exit_code":0}),
        );
        assert_eq!(pane.surface.last_sequence, 3);
        assert_eq!(pane.surface.input_revision, 90);
        assert!(pane.surface.input_bridge);
        assert!(!pane.surface.query_bridge);
        assert!(!pane.surface.isolated_shell);
        assert!(pane.active_command.is_none());
        assert!(pane.editor.focus_requested);
        assert!(!pane.model.has_running_command());
    }

    #[test]
    fn os_delete_repeats_are_all_applied_and_painted_in_the_same_frame() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        render(
            &mut app,
            &ctx,
            vec![egui::Event::Text("abcdefghij".to_owned())],
        );
        let output = render(
            &mut app,
            &ctx,
            (0..5)
                .map(|_| egui::Event::Key {
                    key: Key::Backspace,
                    physical_key: Some(Key::Backspace),
                    pressed: true,
                    repeat: true,
                    modifiers: egui::Modifiers::NONE,
                })
                .collect(),
        );
        assert_eq!(
            app.active_pane_editor_text_for_tests().as_deref(),
            Some("abcde")
        );
        assert!(visible_text(&output, "abcde").is_some());
    }

    fn key(key: Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn click(
        app: &mut TerminasteApp,
        ctx: &egui::Context,
        position: egui::Pos2,
    ) -> egui::FullOutput {
        render(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        render(
            app,
            ctx,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
    }

    #[test]
    fn block_menu_opens_copies_each_part_and_closes_outside() {
        fn app_with_block() -> TerminasteApp {
            let mut app = TerminasteApp::headless_for_tests(Settings::default());
            app.send_active_pane_output_for_tests(&integration_frame_for_tests("ready", "{}"));
            let pane = app.active_terminal_mut().unwrap();
            pane.model
                .start_integrated_command("echo menu-test".to_owned());
            pane.model.process_bytes(b"menu-test\n");
            pane.model.finish_running_command(0);
            app
        }

        fn select_block(app: &mut TerminasteApp, ctx: &egui::Context) -> egui::Pos2 {
            let frame = render(app, ctx, Vec::new());
            let command = visible_text(&frame, "echo menu-test").unwrap();
            click(app, ctx, command.center());
            render(app, ctx, Vec::new())
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle) if circle.radius == 1.5 => Some(circle.center),
                    _ => None,
                })
                .nth(1)
                .unwrap()
        }

        let ctx = egui::Context::default();
        let mut app = app_with_block();
        for (label, expected) in [
            ("Copy input", "echo menu-test"),
            ("Copy output", "menu-test"),
            ("Copy input and output", "echo menu-test\nmenu-test"),
        ] {
            let menu_position = select_block(&mut app, &ctx);
            assert!(
                app.active_terminal()
                    .unwrap()
                    .block_focus
                    .focused()
                    .is_some(),
                "selected before menu"
            );
            click(&mut app, &ctx, menu_position);
            let open = render(&mut app, &ctx, Vec::new());
            assert!(
                app.active_terminal()
                    .unwrap()
                    .block_focus
                    .focused()
                    .is_some(),
                "selected during menu"
            );
            let item = visible_text(&open, label).unwrap();
            let clicked = click(&mut app, &ctx, item.center());
            assert!(
                clicked.platform_output.commands.iter().any(|command| {
                    matches!(command, egui::OutputCommand::CopyText(text) if text == expected)
                }),
                "{label}"
            );
        }
        let menu_position = select_block(&mut app, &ctx);
        click(&mut app, &ctx, menu_position);
        let open = render(&mut app, &ctx, Vec::new());
        assert!(visible_text(&open, "Copy input").is_some());
        click(&mut app, &ctx, egui::pos2(200.0, 300.0));
        let closed = render(&mut app, &ctx, Vec::new());
        assert!(visible_text(&closed, "Copy input").is_none());
    }

    #[test]
    fn prompt_uses_editor_then_running_program_receives_direct_input() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();
        app.send_active_pane_output_for_tests(&integration_frame_for_tests("ready", "{}"));
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        commands.try_iter().for_each(drop);
        render(&mut app, &ctx, vec![egui::Event::Text("cat".to_owned())]);
        assert_eq!(
            app.active_pane_editor_text_for_tests().as_deref(),
            Some("cat")
        );
        assert!(!commands
            .try_iter()
            .any(|command| matches!(command, PtyCommand::Write(_))));
        render(
            &mut app,
            &ctx,
            vec![key(Key::Escape, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx, vec![key(Key::Enter, egui::Modifiers::NONE)]);
        assert!(commands
            .try_iter()
            .any(|command| command == PtyCommand::Write(b"cat\r".to_vec())));
        render(&mut app, &ctx, Vec::new());
        commands.try_iter().for_each(drop);
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::Text("hello".to_owned()),
                key(Key::Enter, egui::Modifiers::NONE),
            ],
        );
        let bytes = commands
            .try_iter()
            .filter_map(|command| match command {
                PtyCommand::Write(bytes) => Some(bytes),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(bytes, b"hello\r");
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-end",
            "{\"exit_code\":0}",
        ));
        render(&mut app, &ctx, Vec::new());
        render(&mut app, &ctx, vec![egui::Event::Text("next".to_owned())]);
        assert_eq!(
            app.active_pane_editor_text_for_tests().as_deref(),
            Some("next")
        );
    }

    #[test]
    fn bridged_typing_updates_editor_before_shell_reply() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        commands.try_iter().for_each(drop);

        render(
            &mut app,
            &ctx,
            vec![egui::Event::Text("instant".to_owned())],
        );

        assert_eq!(
            app.active_pane_editor_text_for_tests().as_deref(),
            Some("instant")
        );
        assert!(commands.try_iter().any(|command| {
            matches!(command, PtyCommand::Write(bytes) if bytes.starts_with(b"\x1b[99~"))
        }));
    }

    fn open_shell_completions(app: &mut TerminasteApp, ctx: &egui::Context) {
        app.active_terminal_mut()
            .unwrap()
            .editor
            .set_text("demo ".to_owned());
        app.active_terminal_mut().unwrap().editor.focus_requested = true;
        render(app, ctx, vec![key(Key::Tab, egui::Modifiers::NONE)]);
        let revision = app.active_terminal().unwrap().surface.input_revision;
        let items = (0..14)
            .chain(0..14)
            .map(
                |index| serde_json::json!({"text": format!("demo choice{index:02}"), "cursor": 13}),
            )
            .collect::<Vec<_>>();
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "completions",
            &serde_json::json!({"revision":revision, "text":"demo ", "items":items}).to_string(),
        ));
        render(app, ctx, Vec::new());
        render(app, ctx, Vec::new());
    }

    #[test]
    fn completion_panel_is_taller_and_elides_long_commands_without_changing_them() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        open_shell_completions(&mut app, &ctx);
        let command = format!(
            "echo {}\nsecond line\nthird line",
            "long-command ".repeat(50)
        );
        let pane = app.active_terminal_mut().unwrap();
        pane.completions[0].label = command.clone();
        pane.completions[0].replacement = command.clone();
        pane.surface.completion_cursors[0] = command.chars().count();
        render(&mut app, &ctx, Vec::new());
        let output = render(&mut app, &ctx, Vec::new());
        let panel = app
            .active_terminal()
            .unwrap()
            .surface
            .completion_rect
            .unwrap();
        assert!(panel.height() > 250.0, "{panel:?}");
        assert!(panel.top() >= 0.0 && panel.bottom() < 500.0);
        let galley = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == command => Some(&text.galley),
                _ => None,
            })
            .unwrap();
        assert_eq!(galley.rows.len(), 2);
        assert!(galley.elided);
        assert_eq!(galley.rows.last().unwrap().glyphs.last().unwrap().chr, '…');
        render(&mut app, &ctx, vec![key(Key::Tab, egui::Modifiers::NONE)]);
        assert_eq!(app.active_terminal().unwrap().editor.text(), command);
    }

    #[test]
    fn shell_completions_support_keyboard_mouse_scrolling_and_dismissal() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        open_shell_completions(&mut app, &ctx);
        assert_eq!(app.active_terminal().unwrap().completions.len(), 14);
        commands.try_iter().for_each(drop);
        for _ in 0..10 {
            render(
                &mut app,
                &ctx,
                vec![key(Key::ArrowUp, egui::Modifiers::NONE)],
            );
        }
        assert_eq!(app.active_terminal().unwrap().selected_completion, 10);
        render(
            &mut app,
            &ctx,
            vec![key(Key::ArrowDown, egui::Modifiers::NONE)],
        );
        assert_eq!(app.active_terminal().unwrap().selected_completion, 9);
        assert!(!commands
            .try_iter()
            .any(|command| matches!(command, PtyCommand::Write(_))));
        render(&mut app, &ctx, vec![key(Key::Tab, egui::Modifiers::NONE)]);
        assert_eq!(
            app.active_terminal().unwrap().editor.text(),
            "demo choice09"
        );
        assert!(app.active_terminal().unwrap().completions.is_empty());
        let writes = commands
            .try_iter()
            .filter_map(|command| match command {
                PtyCommand::Write(bytes) => Some(bytes),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].starts_with(b"\x1b[99~"));

        open_shell_completions(&mut app, &ctx);
        let output = render(&mut app, &ctx, Vec::new());
        let row = visible_text_containing(&output, "demo choice00").unwrap();
        click(&mut app, &ctx, row.center());
        assert_eq!(
            app.active_terminal().unwrap().editor.text(),
            "demo choice00"
        );
        assert!(app.active_terminal().unwrap().completions.is_empty());
        render(&mut app, &ctx, vec![egui::Event::Text("x".to_owned())]);
        assert_eq!(
            app.active_terminal().unwrap().editor.text(),
            "demo choice00x"
        );

        open_shell_completions(&mut app, &ctx);
        commands.try_iter().for_each(drop);
        render(
            &mut app,
            &ctx,
            vec![key(Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(app.active_terminal().unwrap().completions.is_empty());
        assert!(!commands
            .try_iter()
            .any(|command| matches!(command, PtyCommand::Write(_))));
        open_shell_completions(&mut app, &ctx);
        click(&mut app, &ctx, egui::pos2(600.0, 70.0));
        assert!(app.active_terminal().unwrap().completions.is_empty());

        for arrow in [Key::ArrowLeft, Key::ArrowRight] {
            open_shell_completions(&mut app, &ctx);
            render(&mut app, &ctx, vec![key(arrow, egui::Modifiers::NONE)]);
            assert!(app.active_terminal().unwrap().completions.is_empty());
            assert_eq!(app.active_terminal().unwrap().editor.text(), "demo ");
        }

        render(&mut app, &ctx, vec![key(Key::Tab, egui::Modifiers::NONE)]);
        let revision = app.active_terminal().unwrap().surface.input_revision;
        render(
            &mut app,
            &ctx,
            vec![key(Key::Escape, egui::Modifiers::NONE)],
        );
        app.send_active_pane_output_for_tests(&integration_frame_for_tests("completions", &serde_json::json!({"revision":revision,"text":"demo ","items":[{"text":"stale", "cursor":5}]}).to_string()));
        assert!(app.active_terminal().unwrap().completions.is_empty());
    }

    #[test]
    fn enter_bursts_never_send_bridge_frames_after_submission_or_prompt_text_as_input() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        render(&mut app, &ctx, vec![egui::Event::Text("cat".to_owned())]);
        commands.try_iter().for_each(drop);
        render(
            &mut app,
            &ctx,
            (0..20)
                .map(|_| key(Key::Enter, egui::Modifiers::NONE))
                .collect(),
        );
        let first = commands
            .try_iter()
            .filter_map(|command| match command {
                PtyCommand::Write(bytes) => Some(bytes),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(first.len(), 2);
        assert_eq!(first[1], b"\r");
        let revision = app.active_terminal().unwrap().surface.input_revision;
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "editor-ready",
            "{\"revision\":0}",
        ));
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "input-buffer",
            &serde_json::json!({"revision": revision,"text":"stale", "prompt":"\u{1b}[32m> "})
                .to_string(),
        ));
        for _ in 0..10 {
            render(
                &mut app,
                &ctx,
                vec![
                    key(Key::Enter, egui::Modifiers::NONE),
                    key(Key::Tab, egui::Modifiers::NONE),
                ],
            );
        }
        assert!(!commands
            .try_iter()
            .any(|command| matches!(command, PtyCommand::Write(_))));
        assert!(app.active_terminal().unwrap().editor.text().is_empty());
        assert_eq!(
            app.active_pane_snapshot_for_tests().unwrap().blocks[0].command,
            "cat"
        );
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-start",
            "{\"command\":\"cat\"}",
        ));
        render(&mut app, &ctx, Vec::new());
        commands.try_iter().for_each(drop);
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::Text("interactive".to_owned()),
                key(Key::Enter, egui::Modifiers::NONE),
            ],
        );
        let writes = commands
            .try_iter()
            .filter_map(|command| match command {
                PtyCommand::Write(bytes) => Some(bytes),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(writes, vec![b"interactive\r".to_vec()]);
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-end",
            "{\"exit_code\":0}",
        ));
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "editor-ready",
            &format!("{{\"revision\":{revision}}}"),
        ));
        render(&mut app, &ctx, vec![egui::Event::Text("next".to_owned())]);
        assert_eq!(app.active_terminal().unwrap().editor.text(), "next");
    }

    #[test]
    fn double_click_empty_tab_bar_opens_one_tab() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        click(&mut app, &ctx, egui::pos2(500.0, 20.0));
        assert_eq!(app.tab_count(), 1);
        click(&mut app, &ctx, egui::pos2(500.0, 20.0));
        assert_eq!(app.tab_count(), 2);
        assert_eq!(app.active_tab, 1);
        click(&mut app, &ctx, egui::pos2(60.0, 20.0));
        click(&mut app, &ctx, egui::pos2(60.0, 20.0));
        assert_eq!(app.tab_count(), 2);
    }

    #[test]
    fn history_arrows_use_prefix_search_until_tab_opens_completions() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        for command in ["echo older", "pwd", "echo newer"] {
            app.submit_active_input_for_tests(command);
            app.finish_active_command_for_tests(0);
        }
        let pane = app.active_terminal_mut().unwrap();
        pane.editor.set_text("echo ".to_owned());
        pane.recall_history(1);
        assert_eq!(pane.editor.text(), "echo newer");
        pane.recall_history(1);
        assert_eq!(pane.editor.text(), "echo older");
        pane.recall_history(-1);
        assert_eq!(pane.editor.text(), "echo newer");
        pane.recall_history(-1);
        assert_eq!(pane.editor.text(), "echo ");
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        app.active_terminal_mut()
            .unwrap()
            .editor
            .set_text("echo ".to_owned());
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        commands.try_iter().for_each(drop);
        render(
            &mut app,
            &ctx,
            vec![key(Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert!(
            matches!(commands.try_recv().unwrap(), PtyCommand::Write(bytes) if bytes.starts_with(b"\x1b[99~"))
        );
        assert_eq!(
            commands.try_recv().unwrap(),
            PtyCommand::Write(b"\x1b[94~".to_vec())
        );
        render(
            &mut app,
            &ctx,
            vec![key(Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert!(commands.try_recv().is_err());
        let revision = app.active_terminal().unwrap().surface.input_revision;
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "completions",
            &serde_json::json!({
                "revision": revision, "text": "echo ", "items": [
                    {"text": "echo shell history", "cursor": 18},
                    {"text": "echo older", "cursor": 10},
                    {"text": "echo shell history", "cursor": 18}
                ]
            })
            .to_string(),
        ));
        render(&mut app, &ctx, Vec::new());
        assert_eq!(app.active_terminal().unwrap().completions.len(), 2);
        assert_eq!(app.active_terminal().unwrap().editor.text(), "echo ");
        render(
            &mut app,
            &ctx,
            vec![key(Key::ArrowUp, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx, vec![key(Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(app.active_terminal().unwrap().editor.text(), "echo older");
        assert!(app.active_terminal().unwrap().completions.is_empty());
        assert!(app.active_terminal().unwrap().active_command.is_none());
        open_shell_completions(&mut app, &ctx);
        commands.try_iter().for_each(drop);
        render(
            &mut app,
            &ctx,
            vec![key(Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert_eq!(app.active_terminal().unwrap().selected_completion, 1);
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn history_panel_ignores_late_results_and_opens_at_top_of_multiline_input() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        for arrow in [Key::ArrowLeft, Key::ArrowRight] {
            render(
                &mut app,
                &ctx,
                vec![key(Key::ArrowUp, egui::Modifiers::NONE)],
            );
            let revision = app.active_terminal().unwrap().surface.input_revision;
            assert_eq!(
                app.active_terminal().unwrap().surface.completion_request,
                Some(revision)
            );
            render(&mut app, &ctx, vec![key(arrow, egui::Modifiers::NONE)]);
            app.send_active_pane_output_for_tests(&integration_frame_for_tests(
                "completions",
                &serde_json::json!({
                    "revision": revision, "text": "", "items": [{"text": "pwd", "cursor": 3}]
                })
                .to_string(),
            ));
            assert!(app.active_terminal().unwrap().completions.is_empty());
            assert!(app.active_terminal().unwrap().history_search.is_none());
        }
        app.active_terminal_mut()
            .unwrap()
            .editor
            .set_text("echo one\necho two".to_owned());
        render(&mut app, &ctx, Vec::new());
        commands.try_iter().for_each(drop);
        render(
            &mut app,
            &ctx,
            vec![key(Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert!(app
            .active_terminal()
            .unwrap()
            .surface
            .completion_request
            .is_none());
        assert!(!commands
            .try_iter()
            .any(|command| command == PtyCommand::Write(b"\x1b[94~".to_vec())));
        render(
            &mut app,
            &ctx,
            vec![key(Key::ArrowUp, egui::Modifiers::NONE)],
        );
        let pane = app.active_terminal().unwrap();
        let revision = pane.surface.input_revision;
        assert_eq!(pane.surface.completion_request, Some(revision));
        assert_eq!(pane.history_search.as_deref(), Some(""));
        assert!(commands
            .try_iter()
            .any(|command| command == PtyCommand::Write(b"\x1b[94~".to_vec())));
        app.send_active_pane_output_for_tests(&integration_frame_for_tests("completions", &serde_json::json!({
            "revision": revision, "text": "echo one\necho two", "items": [{"text": "pwd", "cursor": 3}]
        }).to_string()));
        render(&mut app, &ctx, Vec::new());
        assert_eq!(app.active_terminal().unwrap().completions.len(), 1);
        render(&mut app, &ctx, vec![key(Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(app.active_terminal().unwrap().editor.text(), "pwd");
        assert!(app.active_terminal().unwrap().active_command.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn completed_output_survives_zsh_prompt_cleanup() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests("ready", "{}"));
        for (command, text) in [("ls", "Cargo.toml\r\ncrates\r\n"), (":", "")] {
            let pane = app.active_terminal_mut().unwrap();
            pane.model.start_integrated_command(command.to_owned());
            pane.process_terminal_bytes(format!("> {command}\r\n").as_bytes());
            app.send_active_pane_output_for_tests(&integration_frame_for_tests(
                "command-start",
                &serde_json::json!({"command":command}).to_string(),
            ));
            app.send_active_pane_output_for_tests(text.as_bytes());
            let cols = app.active_terminal().unwrap().cols;
            let cleanup = format!(
                "\x1b[1m\x1b[7m%\x1b[27m\x1b[1m\x1b[0m{}\r \r",
                " ".repeat(usize::from(cols) - 1)
            );
            for byte in cleanup.bytes() {
                app.send_active_pane_output_for_tests(&[byte]);
            }
            app.send_active_pane_output_for_tests(&integration_frame_for_tests(
                "command-end",
                "{\"exit_code\":0}",
            ));
            let snapshot = app.active_pane_snapshot_for_tests().unwrap();
            assert_eq!(
                snapshot.blocks.last().unwrap().output,
                text.replace('\r', "").trim_end()
            );
        }
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        let output = render(&mut app, &ctx, Vec::new());
        let command = visible_text(&output, "ls").unwrap();
        let text = visible_text(&output, "Cargo.toml\ncrates").unwrap();
        assert!(command.bottom() < text.top());
    }

    #[test]
    fn command_output_keeps_monospace_glyph_spacing() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests("ready", "{}"));
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-start",
            "{\"command\":\"ssh-add key\"}",
        ));
        app.send_active_pane_output_for_tests(b"Identity added:\r\n/path/to/key\r\n");
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-end",
            "{\"exit_code\":0}",
        ));
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        let output = render(&mut app, &ctx, Vec::new());
        let text = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains("Identity added:") => {
                    Some(text)
                }
                _ => None,
            })
            .unwrap();
        assert!(!text.galley.job.justify);
    }

    #[test]
    fn long_output_stays_visible_and_scrolls_back_to_the_command() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests("ready", "{}"));
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-start",
            "{\"command\":\"dir\"}",
        ));
        let output = (0..200)
            .map(|index| format!("entry-{index:03}\r\n"))
            .collect::<String>();
        app.send_active_pane_output_for_tests(output.as_bytes());
        render(&mut app, &ctx, Vec::new());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-end",
            "{\"exit_code\":0}",
        ));
        for _ in 0..4 {
            let frame = render(&mut app, &ctx, Vec::new());
            let last_visible = frame.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains("entry-199") => {
                    let row = text
                        .galley
                        .rows
                        .last()
                        .unwrap()
                        .rect
                        .translate(text.pos.to_vec2());
                    shape.clip_rect.contains_rect(row)
                }
                _ => false,
            });
            assert!(
                last_visible,
                "the end of long output must stay above the editor"
            );
        }
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(400.0, 200.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 100_000.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let top = render(&mut app, &ctx, Vec::new());
        assert!(
            visible_text(&top, "dir").is_some(),
            "the command must be reachable at the top"
        );
    }

    #[test]
    fn command_frames_are_centered_evenly_spaced_and_fully_visible() {
        for spacing in [BlockSpacing::Normal, BlockSpacing::Compact] {
            let mut settings = Settings::default();
            settings.appearance.block_spacing = spacing;
            let mut app = TerminasteApp::headless_for_tests(settings);
            app.send_active_pane_output_for_tests(&integration_frame_for_tests("ready", "{}"));
            let pane = app.active_terminal_mut().unwrap();
            for command in ["first", "second"] {
                pane.model.start_integrated_command(command.to_owned());
                pane.model.finish_running_command(0);
            }
            let ctx = egui::Context::default();
            render(&mut app, &ctx, Vec::new());
            let output = render(&mut app, &ctx, Vec::new());
            let frames = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect)
                        if rect.fill == app.theme.card || rect.fill == app.theme.input =>
                    {
                        assert!(
                            shape.clip_rect.contains_rect(rect.rect),
                            "frame {:?}, clip {:?}",
                            rect.rect,
                            shape.clip_rect
                        );
                        assert!(rect.rect.bottom() <= 492.0);
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(frames.len(), 3);
            let first = frames
                .iter()
                .find(|rect| rect.contains(visible_text(&output, "first").unwrap().center()))
                .unwrap();
            let second = frames
                .iter()
                .find(|rect| rect.contains(visible_text(&output, "second").unwrap().center()))
                .unwrap();
            let input = frames
                .iter()
                .max_by(|a, b| a.bottom().total_cmp(&b.bottom()))
                .unwrap();
            assert!(
                ((second.top() - first.bottom()) - (input.top() - second.bottom())).abs() < 1.0
            );
            for (command, frame) in [("first", first), ("second", second)] {
                let text = visible_text(&output, command).unwrap();
                let prompt_width = ctx.fonts(|fonts| {
                    fonts.glyph_width(&FontId::monospace(app.loaded.settings.font.size), '>')
                });
                assert!(
                    (text.center().y - frame.center().y).abs() < 2.0,
                    "{command}: {text:?} vs {frame:?}"
                );
                let margin = command_block_spacing(&app.loaded.settings).0 as f32;
                assert!(
                    (text.left()
                        - frame.left()
                        - margin
                        - prompt_width
                        - ctx.style().spacing.item_spacing.x)
                        .abs()
                        < 2.0
                );
            }
            app.send_active_pane_output_for_tests(&integration_frame_for_tests(
                "command-start",
                "{\"command\":\"server\"}",
            ));
            app.send_active_pane_output_for_tests(b"listening\r\n");
            let running = render(&mut app, &ctx, Vec::new());
            assert!(running.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.fill == app.theme.surface_high && rect.stroke.width == 1.5 =>
                {
                    assert!(shape.clip_rect.contains_rect(rect.rect));
                    assert!(rect.rect.bottom() <= 492.0);
                    true
                }
                _ => false,
            }));
        }
    }

    #[test]
    fn generated_zsh_events_activate_editor_and_create_output_blocks() {
        if !std::path::Path::new("/bin/zsh").exists() {
            return;
        }
        let script = terminaste_pty::generated_script(terminaste_pty::ShellFamily::Zsh);
        let output = std::process::Command::new("/bin/zsh")
            .args([
                "-f",
                "-c",
                &format!(
                    "{script}\n__terminaste_precmd\n__terminaste_preexec 'echo block-output'\nprintf 'block-output\\n'\n__terminaste_precmd"
                ),
            ])
            .env("TERMINASTE_SESSION", Uuid::nil().to_string())
            .env_remove("__TERMINASTE_INTEGRATION_LOADED")
            .output()
            .unwrap();
        assert!(output.status.success());
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.send_active_pane_output_for_tests(&output.stdout);
        let pane = app.active_terminal().unwrap();
        assert!(pane.integration_ready);
        assert!(pane.surface.input_bridge);
        assert!(pane.active_command.is_none());
        let snapshot = pane.model.snapshot();
        assert_eq!(snapshot.blocks.len(), 1);
        assert_eq!(snapshot.blocks[0].command, "echo block-output");
        assert_eq!(snapshot.blocks[0].output, "block-output");
        assert_eq!(snapshot.blocks[0].exit_code, Some(0));
        assert!(!snapshot.blocks[0].running);

        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        render(&mut app, &ctx, vec![egui::Event::Text("next".to_owned())]);
        assert_eq!(
            app.active_pane_editor_text_for_tests().as_deref(),
            Some("next")
        );
        assert!(!app.active_terminal().unwrap().surface.active);
    }

    #[test]
    fn running_commands_stay_in_blocks_until_they_enter_alternate_screen() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "ready",
            "{\"input_bridge\":true}",
        ));
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-start",
            "{\"command\":\"opencode serve\"}",
        ));
        app.send_active_pane_output_for_tests(b"server listening\r\n");
        let ctx = egui::Context::default();
        let output = render(&mut app, &ctx, Vec::new());
        assert!(app.active_terminal().unwrap().surface.composer_active);
        assert!(visible_text(&output, "opencode serve").is_some());
        assert!(visible_text(&output, "server listening").is_some());

        commands.try_iter().for_each(drop);
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::Text("status".to_owned()),
                key(Key::Enter, egui::Modifiers::NONE),
            ],
        );
        assert!(commands
            .try_iter()
            .any(|command| command == PtyCommand::Write(b"status\r".to_vec())));

        app.send_active_pane_output_for_tests(b"\x1b[?1049h");
        render(&mut app, &ctx, Vec::new());
        assert!(
            app.active_terminal()
                .unwrap()
                .model
                .modes()
                .alternate_screen
        );
        assert!(!app.active_terminal().unwrap().surface.composer_active);
    }

    #[test]
    fn input_focus_clears_block_focus_and_jump_actions_reach_both_ends() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests("ready", "{}"));
        for command in ["first", "second"] {
            let pane = app.active_terminal_mut().unwrap();
            pane.model.start_integrated_command(command.to_owned());
            pane.model.finish_running_command(0);
        }
        let pane = app.active_terminal_mut().unwrap();
        pane.focus_first_block();
        let first = pane.focusable_block_targets()[0];
        assert_eq!(pane.block_focus.focused(), Some(first));
        pane.focus_input();
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        let pane = app.active_terminal().unwrap();
        assert!(pane.block_focus.focused().is_none());
        assert!(pane.editor_has_focus);
    }

    #[test]
    fn system_appearance_tracks_the_current_platform_theme() {
        let settings = Settings::default();
        assert_eq!(
            theme_for(&settings, Some(egui::Theme::Light)).background,
            TerminasteTheme::light().background
        );
        assert_eq!(
            theme_for(&settings, Some(egui::Theme::Dark)).background,
            TerminasteTheme::dark().background
        );
    }

    #[test]
    fn raw_input_and_shortcuts_do_not_duplicate_or_leak() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();
        let ctx = egui::Context::default();
        render(&mut app, &ctx, Vec::new());
        commands.try_iter().for_each(drop);
        render(
            &mut app,
            &ctx,
            vec![
                key(Key::C, egui::Modifiers::CTRL),
                key(Key::ArrowUp, egui::Modifiers::NONE),
            ],
        );
        let bytes = commands
            .try_iter()
            .filter_map(|command| match command {
                PtyCommand::Write(bytes) => Some(bytes),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(bytes, b"\x03\x1b[A");
        app.new_tab_for_tests();
        app.close_active_tab();
        let modifiers = if cfg!(target_os = "macos") {
            egui::Modifiers::MAC_CMD | egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::CTRL | egui::Modifiers::ALT
        };
        render(&mut app, &ctx, vec![key(Key::T, modifiers)]);
        assert_eq!(app.tab_count(), 2);
    }

    #[test]
    fn every_chunk_boundary_preserves_integration_and_utf8() {
        let mut bytes = integration_frame_for_tests("command-start", "{\"command\":\"printf\"}");
        bytes.extend_from_slice("\x1b[31m界\x1b[0m\r\n".as_bytes());
        bytes.extend(integration_frame_for_tests(
            "command-end",
            "{\"exit_code\":7}",
        ));
        for split in 0..bytes.len() {
            let mut pane = TerminalPane::fake();
            pane.process_terminal_bytes(&bytes[..split]);
            pane.process_terminal_bytes(&bytes[split..]);
            let snapshot = pane.model.snapshot();
            assert_eq!(snapshot.blocks.len(), 1);
            assert_eq!(snapshot.blocks[0].output, "界");
            assert_eq!(snapshot.blocks[0].exit_code, Some(7));
            assert_eq!(snapshot.cells[0][0].text, "界");
            assert!(snapshot.cells[0][0].style.foreground.is_some());
        }
    }

    #[test]
    fn stale_or_foreign_metadata_cannot_change_session() {
        let mut pane = TerminalPane::fake();
        let frame = integration_frame_for_tests("command-start", "{\"command\":\"one\"}");
        pane.process_terminal_bytes(&frame);
        pane.process_terminal_bytes(&frame);
        let foreign = terminaste_pty::encode_integration_frame(
            "other",
            u64::MAX,
            "directory-change",
            serde_json::json!({"current_directory":"/foreign"}),
        )
        .unwrap();
        pane.process_terminal_bytes(foreign.as_bytes());
        assert_eq!(pane.model.snapshot().blocks.len(), 1);
        assert_ne!(pane.cwd, PathBuf::from("/foreign"));
    }
}
