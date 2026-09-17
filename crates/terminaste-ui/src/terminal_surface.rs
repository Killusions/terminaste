use gpui::{Bounds, Pixels, Point};
use terminaste_core::{encode_key, KeyCode, KeyInput, Modifiers};

use super::*;

#[derive(Default)]
pub(super) struct SurfaceState {
    pub shell_id: Option<String>,
    pub parent_shells: Vec<ShellContext>,
    pub prompt_row: u64,
    pub last_sequence: u64,
    pub focused: bool,
    pub scroll: usize,
    pub wheel: f32,
    pub clipboard: Option<String>,
    pub bell: bool,
    pub input_bridge: bool,
    pub query_bridge: bool,
    pub control_keys: bool,
    pub isolated_shell: bool,
    pub input_revision: u64,
    pub completion_request: Option<u64>,
    pub completion_context: Option<CompletionContext>,
    pub completion_cursors: Vec<usize>,
    pub submit_pending: bool,
    pub prompt_ansi: Option<String>,
    pub right_prompt_ansi: Option<String>,
    pub prompt_cols: Option<u16>,
    pub bounds: Bounds<Pixels>,
    pub origin: Point<Pixels>,
    pub selecting: bool,
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
        let modes = self.model.modes();
        modes.alternate_screen
            || (self.model.running_command_uses_terminal_surface()
                && (modes.mouse_reporting || modes.application_cursor))
    }
    pub(super) fn uses_raw_input(&self) -> bool {
        self.model.has_running_command() || self.has_interactive_surface()
    }
    pub(super) fn write_terminal(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            if let Some(pty) = &self.pty {
                let _ = pty.tx.send(PtyCommand::Write(bytes));
            }
        }
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
        let context = CompletionContext {
            text: self.editor.text().to_owned(),
            cursor: self.editor.cursor(),
            cwd: self.cwd.clone(),
            shell_id: self.surface.shell_id.clone(),
            history,
        };
        self.completions.clear();
        self.surface.completion_cursors.clear();
        if !self.ghost_history_request {
            if let Some(cached) = self.completion_cache[usize::from(history)]
                .as_ref()
                .filter(|cached| cached.context == context)
            {
                self.completions.clone_from(&cached.items);
                self.surface.completion_cursors.clone_from(&cached.cursors);
            }
        }
        self.surface.completion_context = Some(context);
        self.completion_revision = self.completion_revision.saturating_add(1);
        self.selected_completion = 0;
        self.completion_navigating = false;
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

pub(super) fn terminal_key(key: &str) -> Option<KeyCode> {
    Some(match key {
        "enter" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "escape" => KeyCode::Escape,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "space" => KeyCode::Char(' '),
        _ if key.starts_with('f')
            && key[1..]
                .parse::<u8>()
                .is_ok_and(|number| (1..=24).contains(&number)) =>
        {
            KeyCode::Function(key[1..].parse().ok()?)
        }
        _ if key.chars().count() == 1 => KeyCode::Char(key.chars().next()?),
        _ => return None,
    })
}

pub(super) fn paint_cells(
    cells: &[Vec<terminaste_core::TerminalCellSnapshot>],
    origin: Point<Pixels>,
    style: &super::theme::TerminalTextStyle,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) {
    let super::theme::TerminalTextStyle {
        font,
        font_size,
        cell,
        theme,
        drop_background_if_readable,
    } = style.clone();
    let color = |rgb: terminaste_core::Rgb| {
        gpui::rgb((u32::from(rgb.0) << 16) | (u32::from(rgb.1) << 8) | u32::from(rgb.2)).into()
    };
    let light_surface = theme.background == gpui::rgb(0xf7f8fa).into()
        || theme.background == gpui::rgb(0xffffff).into();
    for (row, cells) in cells.iter().enumerate() {
        let row_position = origin + gpui::point(gpui::px(0.), cell.height * row as f32);
        let row_bounds = Bounds::new(
            row_position,
            gpui::size(cell.width * cells.len() as f32, cell.height),
        );
        if !window.content_mask().bounds.intersects(&row_bounds) {
            continue;
        }
        let mut text = String::new();
        let mut runs = Vec::new();
        for (column, value) in cells.iter().enumerate() {
            let mut foreground_rgb = value.style.foreground;
            let mut background_rgb = value.style.background;
            if value.style.inverse {
                std::mem::swap(&mut foreground_rgb, &mut background_rgb);
            }
            let mut foreground = foreground_rgb
                .map(|rgb| {
                    terminal_foreground_color(
                        rgb,
                        theme,
                        light_surface && drop_background_if_readable,
                        drop_background_if_readable,
                    )
                })
                .unwrap_or(theme.text);
            let mut background = background_rgb.map(color).unwrap_or(theme.background);
            if light_surface && drop_background_if_readable {
                if value
                    .style
                    .foreground
                    .is_some_and(|rgb| rgb.0 > 230 && rgb.1 > 230 && rgb.2 > 230)
                {
                    foreground = theme.text;
                }
                if value
                    .style
                    .background
                    .is_some_and(|rgb| rgb.0 < 32 && rgb.1 < 32 && rgb.2 < 32)
                {
                    background = theme.background;
                }
            }
            let background_is_dark = background_rgb.is_some_and(|background| {
                u16::from(background.0) + u16::from(background.1) + u16::from(background.2) < 420
            });
            let background_is_light = background_rgb.is_some_and(|background| {
                u16::from(background.0) + u16::from(background.1) + u16::from(background.2) > 650
            });
            let foreground_is_light = foreground_rgb.is_some_and(|foreground| {
                foreground.0 > 230 && foreground.1 > 230 && foreground.2 > 230
            });
            let foreground_is_dark = foreground_rgb.is_some_and(|foreground| {
                u16::from(foreground.0) + u16::from(foreground.1) + u16::from(foreground.2) < 420
            });
            if drop_background_if_readable
                && background_rgb.is_some()
                && ((light_surface && background_is_dark && !foreground_is_light)
                    || (!light_surface && background_is_light && !foreground_is_dark))
            {
                background = theme.background;
            }
            if value.style.dim {
                foreground = foreground.opacity(0.6);
            }
            let value_text = if value.width == terminaste_core::TerminalCellWidth::Spacer {
                " "
            } else if value.secret {
                "•"
            } else if value.style.hidden || value.text.is_empty() {
                " "
            } else {
                &value.text
            };
            let mut run_font = font.clone();
            if value.style.bold {
                run_font = run_font.bold();
            }
            if value.style.italic {
                run_font = run_font.italic();
            }
            let bounds = Bounds::new(
                row_position + gpui::point(cell.width * column as f32, gpui::px(0.)),
                cell,
            );
            if background != theme.background {
                window.paint_quad(gpui::fill(bounds, background));
            }
            text.push_str(value_text);
            runs.push(gpui::TextRun {
                len: value_text.len(),
                font: run_font,
                color: foreground,
                background_color: None,
                underline: value.style.underline.then_some(gpui::UnderlineStyle {
                    color: Some(foreground),
                    thickness: gpui::px(1.),
                    wavy: false,
                }),
                strikethrough: value
                    .style
                    .strikethrough
                    .then_some(gpui::StrikethroughStyle {
                        color: Some(foreground),
                        thickness: gpui::px(1.),
                    }),
            });
        }
        if text.trim().is_empty() {
            continue;
        }
        let line = window
            .text_system()
            .shape_line(text.into(), font_size, &runs, None);
        let _ = line.paint(
            row_position,
            cell.height,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

fn terminal_foreground_color(
    color: terminaste_core::Rgb,
    theme: TerminasteTheme,
    light_surface: bool,
    normalize: bool,
) -> gpui::Hsla {
    if !normalize {
        return gpui::rgb(
            (u32::from(color.0) << 16) | (u32::from(color.1) << 8) | u32::from(color.2),
        )
        .into();
    }
    let luminance = |component: u8| {
        let value = f32::from(component) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance =
        0.2126 * luminance(color.0) + 0.7152 * luminance(color.1) + 0.0722 * luminance(color.2);
    if light_surface && luminance > 0.88 {
        theme.text
    } else if !light_surface && luminance < 0.12 {
        theme.muted
    } else {
        gpui::rgb((u32::from(color.0) << 16) | (u32::from(color.1) << 8) | u32::from(color.2))
            .into()
    }
}
