use std::cmp::Ordering;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;
use uuid::Uuid;

const ESC: u8 = 0x1b;
pub const TERMINAL_CURSOR_THICKNESS_MULTIPLIER: f32 = 0.15;
pub const TERMINAL_UNDERLINE_THICKNESS_MULTIPLIER: f32 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CellStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub inverse: bool,
    pub dim: bool,
    pub hidden: bool,
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCell {
    pub text: String,
    pub style: CellStyle,
    pub wide: bool,
    pub spacer: bool,
    pub secret: bool,
}

impl Default for TerminalCell {
    fn default() -> Self {
        Self {
            text: " ".to_owned(),
            style: CellStyle::default(),
            wide: false,
            spacer: false,
            secret: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalRow {
    cells: Vec<TerminalCell>,
    wrapped: bool,
}

impl TerminalRow {
    pub fn new(cols: usize) -> Self {
        Self {
            cells: vec![TerminalCell::default(); cols.max(1)],
            wrapped: false,
        }
    }

    pub fn text(&self) -> String {
        let mut text = String::new();
        for cell in &self.cells {
            if !cell.spacer {
                text.push_str(&cell.text);
            }
        }
        text.trim_end().to_owned()
    }

    pub fn cells(&self) -> &[TerminalCell] {
        &self.cells
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalCellWidth {
    Narrow,
    Wide,
    Spacer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeBlockGlyph {
    Block,
    BoxDrawing,
    Powerline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCellSnapshot {
    pub text: String,
    pub style: CellStyle,
    pub width: TerminalCellWidth,
    pub native_glyph: Option<NativeBlockGlyph>,
    pub secret: bool,
}

impl From<&TerminalCell> for TerminalCellSnapshot {
    fn from(cell: &TerminalCell) -> Self {
        Self {
            text: cell.text.clone(),
            style: cell.style,
            width: if cell.spacer {
                TerminalCellWidth::Spacer
            } else if cell.wide {
                TerminalCellWidth::Wide
            } else {
                TerminalCellWidth::Narrow
            },
            native_glyph: cell
                .text
                .chars()
                .next()
                .and_then(classify_native_block_glyph),
            secret: cell.secret,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandBlock {
    pub id: Uuid,
    pub command: String,
    pub output: String,
    pub started_row: u64,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalModes {
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub focus_reporting: bool,
    pub mouse_reporting: bool,
    pub sgr_mouse: bool,
    pub alternate_scroll: bool,
    pub alternate_screen: bool,
    pub auto_wrap: bool,
    pub cursor_visible: bool,
    pub insert_mode: bool,
    pub line_feed_new_line: bool,
    pub origin_mode: bool,
    pub keyboard_disambiguate: bool,
    pub keyboard_report_event_types: bool,
    pub keyboard_report_alternate_keys: bool,
    pub all_keys_as_escape: bool,
    pub keyboard_report_associated_text: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CursorStyle {
    #[default]
    Block,
    Underline,
    Bar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalPoint {
    pub row: u64,
    pub col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalRange {
    pub start: TerminalPoint,
    pub end: TerminalPoint,
}

impl TerminalRange {
    pub fn normalized(self) -> Self {
        if compare_points(self.start, self.end).is_gt() {
            Self {
                start: self.end,
                end: self.start,
            }
        } else {
            self
        }
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSelection {
    pub anchor: TerminalPoint,
    pub focus: TerminalPoint,
}

impl TerminalSelection {
    pub fn range(self) -> TerminalRange {
        TerminalRange {
            start: self.anchor,
            end: self.focus,
        }
        .normalized()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalLink {
    pub url: String,
    pub range: TerminalRange,
}

impl Default for TerminalModes {
    fn default() -> Self {
        Self {
            application_cursor: false,
            application_keypad: false,
            bracketed_paste: false,
            focus_reporting: false,
            mouse_reporting: false,
            sgr_mouse: false,
            alternate_scroll: false,
            alternate_screen: false,
            auto_wrap: true,
            cursor_visible: true,
            insert_mode: false,
            line_feed_new_line: false,
            origin_mode: false,
            keyboard_disambiguate: false,
            keyboard_report_event_types: false,
            keyboard_report_alternate_keys: false,
            all_keys_as_escape: false,
            keyboard_report_associated_text: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Osc52Policy {
    #[default]
    Deny,
    WriteOnly,
    ReadWrite,
}

impl Osc52Policy {
    fn allows_write(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct SavedCursor {
    col: usize,
    row: usize,
    style: CellStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    Bell,
    TitleChanged(String),
    ClipboardWriteRequested(String),
    Integration(IntegrationEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationEvent {
    pub name: String,
    pub payload: String,
}

#[derive(Debug, Clone)]
pub struct TerminalSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub visible_row_start: u64,
    pub cursor_col: usize,
    pub cursor_row: usize,
    pub cursor_visible: bool,
    pub cursor_style: CursorStyle,
    pub visible_lines: Vec<String>,
    pub cells: Vec<Vec<TerminalCellSnapshot>>,
    pub blocks: Vec<CommandBlock>,
    pub selection: Option<TerminalSelection>,
    pub selection_ranges: Vec<TerminalRange>,
    pub search_match_ranges: Vec<TerminalRange>,
    pub link_ranges: Vec<TerminalLink>,
    pub modes: TerminalModes,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct TerminalModel {
    cols: usize,
    rows: usize,
    max_history_rows: usize,
    visible: Vec<TerminalRow>,
    scrollback: VecDeque<TerminalRow>,
    alternate: Option<Vec<TerminalRow>>,
    cursor_col: usize,
    cursor_row: usize,
    saved_cursor: Option<SavedCursor>,
    style: CellStyle,
    cursor_style: CursorStyle,
    selection: Option<TerminalSelection>,
    modes: TerminalModes,
    parser: ParserState,
    utf8_buffer: Vec<u8>,
    osc52_policy: Osc52Policy,
    blocks: Vec<CommandBlock>,
    command_buffer: String,
    pending_output: String,
    dynamic_command_output: bool,
    pending_carriage_return: bool,
    command_started_at: Option<Instant>,
    title: String,
    discarded_rows: u64,
    scroll_top: usize,
    scroll_bottom: usize,
    responses: Vec<u8>,
    default_colors: [Rgb; 2],
    report_color_scheme: bool,
}

impl TerminalModel {
    pub fn new(cols: usize, rows: usize, max_history_rows: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Self {
            cols,
            rows,
            max_history_rows: max_history_rows.max(rows),
            visible: (0..rows).map(|_| TerminalRow::new(cols)).collect(),
            scrollback: VecDeque::new(),
            alternate: None,
            cursor_col: 0,
            cursor_row: 0,
            saved_cursor: None,
            style: CellStyle::default(),
            cursor_style: CursorStyle::default(),
            selection: None,
            modes: TerminalModes::default(),
            parser: ParserState::Ground,
            utf8_buffer: Vec::new(),
            osc52_policy: Osc52Policy::Deny,
            blocks: Vec::new(),
            command_buffer: String::new(),
            pending_output: String::new(),
            dynamic_command_output: false,
            pending_carriage_return: false,
            command_started_at: None,
            title: "terminaste".to_owned(),
            discarded_rows: 0,
            scroll_top: 0,
            scroll_bottom: rows - 1,
            responses: Vec::new(),
            default_colors: [Rgb(238, 242, 255), Rgb(10, 11, 15)],
            report_color_scheme: false,
        }
    }

    pub fn with_osc52_policy(mut self, policy: Osc52Policy) -> Self {
        self.osc52_policy = policy;
        self
    }

    pub fn set_osc52_policy(&mut self, policy: Osc52Policy) {
        self.osc52_policy = policy;
    }

    pub fn set_history_limit(&mut self, limit: usize) {
        self.max_history_rows = limit.max(self.rows);
        while self.scrollback.len() > self.max_history_rows {
            self.scrollback.pop_front();
            self.discarded_rows += 1;
        }
    }

    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.responses)
    }

    pub fn set_default_colors(&mut self, foreground: Rgb, background: Rgb) {
        let changed = self.default_colors != [foreground, background];
        self.default_colors = [foreground, background];
        if changed && self.report_color_scheme {
            self.respond_color_scheme();
        }
    }

    fn respond_color_scheme(&mut self) {
        let Rgb(r, g, b) = self.default_colors[1];
        let light = u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114 >= 128_000;
        self.responses.extend_from_slice(if light {
            b"\x1b[?997;2n"
        } else {
            b"\x1b[?997;1n"
        });
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.flush_utf8_buffer();
        let old_cols = self.cols;
        let mut old_rows = self.active_rows().to_vec();
        while old_rows.len() > self.cursor_row + 1
            && old_rows.last().is_some_and(|row| row.text().is_empty())
        {
            old_rows.pop();
        }
        let mut logical_start = self.cursor_row;
        while logical_start > 0 && old_rows[logical_start - 1].wrapped {
            logical_start -= 1;
        }
        let offset = (self.cursor_row - logical_start) * old_cols + self.cursor_col;
        let preceding_rows = if logical_start == 0 {
            0
        } else {
            reflow_rows(&old_rows[..logical_start], cols).len()
        };
        self.cols = cols;
        self.rows = rows;
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        if self.modes.alternate_screen {
            for grid in [&mut self.visible, self.alternate.as_mut().unwrap()] {
                grid.resize_with(rows, || TerminalRow::new(cols));
                for row in grid {
                    row.cells.resize(cols, TerminalCell::default());
                }
            }
            self.cursor_row = self.cursor_row.min(rows - 1);
            self.cursor_col = self.cursor_col.min(cols - 1);
        } else {
            let mut reflowed = reflow_rows(&old_rows, cols);
            let dropped = reflowed.len().saturating_sub(rows);
            self.scrollback.extend(reflowed.drain(..dropped));
            self.visible = last_rows(reflowed, rows, cols);
            self.cursor_row = (preceding_rows + offset / cols)
                .saturating_sub(dropped)
                .min(rows - 1);
            self.cursor_col = offset % cols;
            self.set_history_limit(self.max_history_rows);
        }
        self.saved_cursor = self.saved_cursor.map(|saved| SavedCursor {
            col: saved.col.min(self.cols - 1),
            row: saved.row.min(self.rows - 1),
            style: saved.style,
        });
    }

    pub fn process_bytes(&mut self, bytes: &[u8]) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        for &byte in bytes {
            self.process_byte(byte, &mut events);
        }
        events
    }

    pub fn note_user_input(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\r' | '\n' => {
                    let command = self.command_buffer.trim().to_owned();
                    if !command.is_empty() {
                        self.blocks.push(CommandBlock {
                            id: Uuid::new_v4(),
                            command,
                            output: String::new(),
                            started_row: self.absolute_cursor_row(),
                            exit_code: None,
                            duration_ms: None,
                            running: true,
                        });
                        self.pending_output.clear();
                        self.command_started_at = Some(Instant::now());
                    }
                    self.command_buffer.clear();
                }
                '\u{8}' | '\u{7f}' => {
                    self.command_buffer.pop();
                }
                _ => self.command_buffer.push(ch),
            }
        }
    }

    pub fn finish_running_command(&mut self, exit_code: i32) {
        self.finish_running_command_matching(None, exit_code);
    }

    pub fn finish_running_command_matching(&mut self, command: Option<&str>, exit_code: i32) {
        let index = self.blocks.iter().rposition(|block| {
            block.running && command.is_none_or(|command| block.command == command)
        });
        let Some(index) = index else {
            return;
        };
        if self.dynamic_command_output && !self.modes.alternate_screen {
            let start = self.blocks[index].started_row.max(self.discarded_rows);
            let end = self.visible_row_start() + self.rows as u64;
            let mut output = String::new();
            for row in (start..end).filter_map(|row| self.row_at(row)) {
                if row.wrapped {
                    for cell in row.cells.iter().filter(|cell| !cell.spacer) {
                        output.push_str(&cell.text);
                    }
                } else {
                    output.push_str(&row.text());
                    output.push('\n');
                }
            }
            self.pending_output = output;
        }
        let block = &mut self.blocks[index];
        block.exit_code = Some(exit_code);
        block.running = false;
        if !self.pending_output.is_empty() && command.is_none_or(|command| block.command == command)
        {
            block.output = self.pending_output.trim_end().to_owned();
            self.pending_output.clear();
        }
        block.duration_ms = self.command_started_at.map(|start| millis(start.elapsed()));
        if command.is_none() || !self.blocks.iter().any(|block| block.running) {
            self.command_started_at = None;
            self.dynamic_command_output = false;
            self.pending_carriage_return = false;
        }
    }

    pub fn discard_pending_output(&mut self) {
        self.pending_output.clear();
        self.dynamic_command_output = false;
        self.pending_carriage_return = false;
        if let Some(block) = self.blocks.iter_mut().rev().find(|block| block.running) {
            block.output.clear();
        }
    }

    pub fn append_running_command_output(&mut self, text: &str) {
        let _ = self.append_running_command_output_matching(None, text);
    }

    pub fn append_running_command_output_matching(
        &mut self,
        command: Option<&str>,
        text: &str,
    ) -> bool {
        if text.is_empty() {
            return false;
        }
        let Some(block) =
            self.blocks.iter_mut().rev().find(|block| {
                block.running && command.is_none_or(|command| block.command == command)
            })
        else {
            return false;
        };
        self.pending_output.push_str(text);
        block.output = self.pending_output.trim_end().to_owned();
        true
    }

    pub fn start_integrated_command(&mut self, command: String) {
        if command.trim().is_empty() {
            return;
        }
        if self
            .blocks
            .iter()
            .rev()
            .any(|block| block.running && block.command == command)
        {
            self.confirm_submitted_command(command);
            return;
        }
        self.blocks.push(CommandBlock {
            id: Uuid::new_v4(),
            command,
            output: String::new(),
            started_row: self.absolute_cursor_row(),
            exit_code: None,
            duration_ms: None,
            running: true,
        });
        self.pending_output.clear();
        self.dynamic_command_output = false;
        self.pending_carriage_return = false;
        self.command_started_at = Some(Instant::now());
    }

    pub fn has_running_command(&self) -> bool {
        self.blocks.last().is_some_and(|block| block.running)
    }

    pub fn running_command_uses_terminal_surface(&self) -> bool {
        self.dynamic_command_output && self.has_running_command()
    }

    pub fn confirm_submitted_command(&mut self, command: String) {
        let started_row = self.absolute_cursor_row();
        if let Some(block) = self.blocks.last_mut().filter(|block| block.running) {
            block.command = command;
            block.started_row = started_row;
        }
        self.discard_pending_output();
    }

    pub fn discard_submitted_command(&mut self) {
        if self.has_running_command() {
            self.blocks.pop();
            self.pending_output.clear();
            self.dynamic_command_output = false;
            self.pending_carriage_return = false;
            self.command_started_at = None;
        }
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        let mut snapshot = self.screen_snapshot();
        snapshot.blocks = self.command_blocks();
        snapshot
    }

    pub fn command_blocks(&self) -> Vec<CommandBlock> {
        let mut blocks = self.blocks.clone();
        if let Some(block) = blocks.iter_mut().rev().find(|block| block.running) {
            block.output = self.pending_output.trim_end().to_owned();
        }
        blocks
    }

    pub fn command_block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn command_block(&self, index: usize) -> Option<CommandBlock> {
        let mut block = self.blocks.get(index)?.clone();
        if block.running {
            block.output = self.pending_output.trim_end().to_owned();
        }
        Some(block)
    }

    pub fn running_command_output(&self) -> String {
        let Some(block) = self.blocks.iter().rev().find(|block| block.running) else {
            return String::new();
        };
        let start = block.started_row.max(self.discarded_rows);
        let end = self.visible_row_start() + self.rows as u64;
        let mut output = String::new();
        for row in (start..end).filter_map(|row| self.row_at(row)) {
            if row.wrapped {
                for cell in row.cells.iter().filter(|cell| !cell.spacer) {
                    output.push_str(&cell.text);
                }
            } else {
                output.push_str(&row.text());
                output.push('\n');
            }
        }
        output.trim_end().to_owned()
    }

    pub fn screen_snapshot(&self) -> TerminalSnapshot {
        TerminalSnapshot {
            cols: self.cols,
            rows: self.rows,
            visible_row_start: self.visible_row_start(),
            cursor_col: self.cursor_col,
            cursor_row: self.cursor_row,
            cursor_visible: self.modes.cursor_visible,
            cursor_style: self.cursor_style,
            visible_lines: self.visible_lines(),
            cells: self.cell_snapshots(),
            blocks: Vec::new(),
            selection: self.selection,
            selection_ranges: self.selection_ranges(),
            search_match_ranges: Vec::new(),
            link_ranges: detect_links(&build_text_index(
                self.active_rows(),
                self.visible_row_start(),
            )),
            modes: self.modes,
            title: self.title.clone(),
        }
    }

    pub fn restore_blocks(&mut self, mut blocks: Vec<CommandBlock>) {
        for block in &mut blocks {
            block.running = false;
        }
        self.blocks = blocks;
        self.pending_output.clear();
        self.dynamic_command_output = false;
        self.pending_carriage_return = false;
        self.command_started_at = None;
    }

    pub fn snapshot_scrolled(&self, offset: usize) -> TerminalSnapshot {
        let mut snapshot = self.snapshot();
        let offset = offset.min(self.scrollback.len());
        if offset == 0 || self.modes.alternate_screen {
            return snapshot;
        }
        snapshot.visible_row_start -= offset as u64;
        snapshot.cursor_visible = false;
        snapshot.cells.clear();
        snapshot.visible_lines.clear();
        for row in snapshot.visible_row_start..snapshot.visible_row_start + self.rows as u64 {
            if let Some(row) = self.row_at(row) {
                snapshot.visible_lines.push(row.text());
                snapshot
                    .cells
                    .push(row.cells.iter().map(TerminalCellSnapshot::from).collect());
            }
        }
        snapshot
    }

    pub fn visible_lines(&self) -> Vec<String> {
        self.active_rows().iter().map(TerminalRow::text).collect()
    }

    pub fn scrollback_lines(&self) -> Vec<String> {
        self.scrollback.iter().map(TerminalRow::text).collect()
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    pub fn scrollback_row(&self, absolute_row: u64) -> Option<&TerminalRow> {
        let index = absolute_row.checked_sub(self.discarded_rows)? as usize;
        self.scrollback.get(index)
    }

    pub fn visible_row(&self, absolute_row: u64) -> Option<&TerminalRow> {
        let index = absolute_row.checked_sub(self.visible_row_start())? as usize;
        self.active_rows().get(index)
    }

    pub fn row_text(&self, absolute_row: u64) -> Option<String> {
        self.row_at(absolute_row).map(TerminalRow::text)
    }

    pub fn set_selection(&mut self, anchor: TerminalPoint, focus: TerminalPoint) {
        self.selection = Some(TerminalSelection { anchor, focus });
    }

    pub fn set_visible_selection(
        &mut self,
        anchor_row: usize,
        anchor_col: usize,
        focus_row: usize,
        focus_col: usize,
    ) {
        let row_start = self.visible_row_start();
        self.set_selection(
            TerminalPoint {
                row: row_start + anchor_row as u64,
                col: anchor_col.min(self.cols),
            },
            TerminalPoint {
                row: row_start + focus_row as u64,
                col: focus_col.min(self.cols),
            },
        );
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn selection(&self) -> Option<TerminalSelection> {
        self.selection
    }

    pub fn selected_text(&self) -> String {
        self.selection
            .map(|selection| self.text_for_range(selection.range()))
            .unwrap_or_default()
    }

    pub fn search(&self, query: &str) -> Vec<TerminalRange> {
        if query.is_empty() {
            return Vec::new();
        }
        let haystack = self.text_index();
        find_ranges(&haystack, query)
    }

    pub fn links(&self) -> Vec<TerminalLink> {
        self.detect_links()
    }

    pub fn modes(&self) -> TerminalModes {
        self.modes
    }

    pub fn discarded_rows(&self) -> u64 {
        self.discarded_rows
    }

    pub fn visible_row_start(&self) -> u64 {
        if self.modes.alternate_screen {
            0
        } else {
            self.discarded_rows + self.scrollback.len() as u64
        }
    }

    fn process_byte(&mut self, byte: u8, events: &mut Vec<TerminalEvent>) {
        match std::mem::replace(&mut self.parser, ParserState::Ground) {
            ParserState::Ground => self.ground_byte(byte, events),
            ParserState::Escape => self.escape_byte(byte),
            ParserState::Csi(mut data) => {
                if (0x40..=0x7e).contains(&byte) {
                    self.apply_csi(&data, byte);
                } else if data.len() < 128 {
                    data.push(byte);
                    self.parser = ParserState::Csi(data);
                }
            }
            ParserState::Osc(mut data) => {
                if byte == 0x07 {
                    self.apply_osc(&data, events);
                } else if data.ends_with(&[ESC]) && byte == b'\\' {
                    data.pop();
                    self.apply_osc(&data, events);
                } else if data.len() < 16 * 1024 {
                    data.push(byte);
                    self.parser = ParserState::Osc(data);
                } else {
                    self.parser = ParserState::DiscardString(byte == ESC);
                }
            }
            ParserState::Dcs(mut data) => {
                if data.ends_with(&[ESC]) && byte == b'\\' {
                    data.pop();
                    self.apply_dcs(&data, events);
                } else if data.len() < 1024 * 1024 {
                    data.push(byte);
                    self.parser = ParserState::Dcs(data);
                } else {
                    self.parser = ParserState::DiscardString(byte == ESC);
                }
            }
            ParserState::DiscardString(escape) => {
                if byte != 7 && !(escape && byte == b'\\') {
                    self.parser = ParserState::DiscardString(byte == ESC);
                }
            }
        }
    }

    fn ground_byte(&mut self, byte: u8, events: &mut Vec<TerminalEvent>) {
        if self.pending_carriage_return && byte != b'\n' {
            self.begin_dynamic_command_output();
        }
        self.pending_carriage_return = false;
        match byte {
            ESC => self.parser = ParserState::Escape,
            b'\r' => {
                self.cursor_col = 0;
                self.pending_carriage_return =
                    !self.modes.alternate_screen && self.has_running_command();
            }
            b'\n' => {
                self.append_pending_output_char('\n');
                self.line_feed();
                if self.modes.line_feed_new_line {
                    self.cursor_col = 0;
                }
            }
            b'\x08' => self.cursor_col = self.cursor_col.saturating_sub(1),
            b'\t' => {
                let next = (((self.cursor_col / 8) + 1) * 8).min(self.cols - 1);
                for _ in self.cursor_col..next {
                    self.append_pending_output_char(' ');
                }
                self.cursor_col = next;
            }
            b'\x07' => events.push(TerminalEvent::Bell),
            0x20..=0x7e => {
                self.flush_utf8_buffer();
                self.write_printable_str(&(byte as char).to_string());
            }
            _ => {
                if byte >= 0x80 {
                    self.push_utf8_byte(byte);
                }
            }
        }
    }

    fn push_utf8_byte(&mut self, byte: u8) {
        self.utf8_buffer.push(byte);
        loop {
            if self.utf8_buffer.is_empty() {
                break;
            }
            match std::str::from_utf8(&self.utf8_buffer) {
                Ok(text) => {
                    let text = text.to_owned();
                    self.utf8_buffer.clear();
                    self.write_printable_str(&text);
                    break;
                }
                Err(error) => {
                    if error.valid_up_to() > 0 {
                        let valid = self.utf8_buffer[..error.valid_up_to()].to_vec();
                        let text = String::from_utf8(valid).unwrap_or_default();
                        self.write_printable_str(&text);
                        self.utf8_buffer.drain(..error.valid_up_to());
                        continue;
                    }
                    if let Some(error_len) = error.error_len() {
                        self.write_printable_str("�");
                        self.utf8_buffer.drain(..error_len);
                        continue;
                    }
                    if self.utf8_buffer.len() > 4 {
                        self.write_printable_str("�");
                        self.utf8_buffer.clear();
                    }
                    break;
                }
            }
        }
    }

    fn flush_utf8_buffer(&mut self) {
        if !self.utf8_buffer.is_empty() {
            self.utf8_buffer.clear();
            self.write_printable_str("�");
        }
    }

    fn escape_byte(&mut self, byte: u8) {
        match byte {
            b'[' => self.parser = ParserState::Csi(Vec::new()),
            b']' => self.parser = ParserState::Osc(Vec::new()),
            b'P' => self.parser = ParserState::Dcs(Vec::new()),
            b'_' | b'^' => self.parser = ParserState::DiscardString(false),
            b'=' => self.modes.application_keypad = true,
            b'>' => self.modes.application_keypad = false,
            b'D' => self.line_feed(),
            b'E' => {
                self.cursor_col = 0;
                self.line_feed();
            }
            b'M' => {
                if self.cursor_row == self.scroll_top {
                    let bottom = self.scroll_bottom;
                    let top = self.scroll_top;
                    let cols = self.cols;
                    self.active_rows_mut().remove(bottom);
                    self.active_rows_mut().insert(top, TerminalRow::new(cols));
                } else {
                    self.cursor_row = self.cursor_row.saturating_sub(1);
                }
            }
            b'c' => self.clear_screen(),
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            _ => {}
        }
    }

    fn apply_csi(&mut self, data: &[u8], final_byte: u8) {
        if matches!(
            final_byte,
            b'@' | b'A'
                | b'B'
                | b'C'
                | b'D'
                | b'E'
                | b'F'
                | b'G'
                | b'H'
                | b'J'
                | b'K'
                | b'L'
                | b'M'
                | b'P'
                | b'S'
                | b'T'
                | b'X'
                | b'`'
                | b'd'
                | b'e'
                | b'f'
                | b'r'
                | b's'
                | b'u'
        ) {
            self.begin_dynamic_command_output();
        }
        let text = String::from_utf8_lossy(data);
        match final_byte {
            b'm' => self.apply_sgr(&text),
            b'n' => match text.as_ref() {
                "?996" => self.respond_color_scheme(),
                "5" => self.responses.extend_from_slice(b"\x1b[0n"),
                "6" => self.responses.extend_from_slice(
                    format!(
                        "\x1b[{};{}R",
                        self.cursor_row + 1,
                        self.cursor_col.min(self.cols - 1) + 1
                    )
                    .as_bytes(),
                ),
                _ => {}
            },
            b'c' => self.responses.extend_from_slice(b"\x1b[?1;2c"),
            b'p' if text == "?2031$" => {
                let state = if self.report_color_scheme { 1 } else { 2 };
                self.responses
                    .extend_from_slice(format!("\x1b[?2031;{state}$y").as_bytes());
            }
            b'r' => {
                let nums = parse_numbers(&text);
                let top = nums.first().copied().unwrap_or(1).max(1) - 1;
                let bottom = nums.get(1).copied().unwrap_or(self.rows).max(1) - 1;
                if top < bottom && bottom < self.rows {
                    self.scroll_top = top;
                    self.scroll_bottom = bottom;
                    self.cursor_col = 0;
                    self.cursor_row = if self.modes.origin_mode { top } else { 0 };
                }
            }
            b'G' | b'`' => {
                self.cursor_col = parse_numbers(&text)
                    .first()
                    .copied()
                    .unwrap_or(1)
                    .max(1)
                    .saturating_sub(1)
                    .min(self.cols - 1)
            }
            b'd' => {
                self.cursor_row = parse_numbers(&text)
                    .first()
                    .copied()
                    .unwrap_or(1)
                    .max(1)
                    .saturating_sub(1)
                    .min(self.rows - 1)
            }
            b'P' | b'@' | b'X' => {
                let col = self.cursor_col.min(self.cols - 1);
                let count = parse_numbers(&text)
                    .first()
                    .copied()
                    .unwrap_or(1)
                    .max(1)
                    .min(self.cols - col);
                if final_byte == b'@' {
                    self.cursor_col = col;
                    self.insert_cells(count);
                } else if final_byte == b'X' {
                    self.clear_line_range(col, col + count);
                } else {
                    let row = self.cursor_row;
                    let cols = self.cols;
                    let cells = &mut self.active_rows_mut()[row].cells;
                    for index in col..cols - count {
                        cells[index] = cells[index + count].clone();
                    }
                    for cell in &mut cells[cols - count..] {
                        *cell = TerminalCell::default();
                    }
                }
            }
            b'L' | b'M' => {
                if (self.scroll_top..=self.scroll_bottom).contains(&self.cursor_row) {
                    let row = self.cursor_row;
                    let bottom = self.scroll_bottom;
                    let cols = self.cols;
                    let count = parse_numbers(&text)
                        .first()
                        .copied()
                        .unwrap_or(1)
                        .max(1)
                        .min(bottom - row + 1);
                    for _ in 0..count {
                        if final_byte == b'L' {
                            self.active_rows_mut().remove(bottom);
                            self.active_rows_mut().insert(row, TerminalRow::new(cols));
                        } else {
                            self.active_rows_mut().remove(row);
                            self.active_rows_mut()
                                .insert(bottom, TerminalRow::new(cols));
                        }
                    }
                }
            }
            b'H' | b'f' => {
                let nums = parse_numbers(&text);
                self.cursor_row = nums
                    .first()
                    .copied()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(self.rows - 1);
                self.cursor_col = nums
                    .get(1)
                    .copied()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(self.cols - 1);
            }
            b'A' => {
                self.cursor_row = self
                    .cursor_row
                    .saturating_sub(parse_numbers(&text).first().copied().unwrap_or(1))
            }
            b'B' => {
                self.cursor_row = (self.cursor_row
                    + parse_numbers(&text).first().copied().unwrap_or(1))
                .min(self.rows - 1)
            }
            b'C' => {
                self.cursor_col = (self.cursor_col
                    + parse_numbers(&text).first().copied().unwrap_or(1))
                .min(self.cols - 1)
            }
            b'D' => {
                self.cursor_col = self
                    .cursor_col
                    .saturating_sub(parse_numbers(&text).first().copied().unwrap_or(1))
            }
            b's' => self.save_cursor(),
            b'u' if text.starts_with('>') => {
                self.apply_keyboard_protocol_flags(text.trim_start_matches('>'), true)
            }
            b'u' if text.starts_with('<') => self.apply_keyboard_protocol_flags("0", false),
            b'u' => self.restore_cursor(),
            b'J' => self.erase_in_display(parse_numbers(&text).first().copied().unwrap_or(0)),
            b'K' => self.erase_in_line(parse_numbers(&text).first().copied().unwrap_or(0)),
            b'h' | b'l' => self.apply_mode(&text, final_byte == b'h'),
            b'q' => self.apply_cursor_style(&text),
            _ => {}
        }
    }

    fn apply_cursor_style(&mut self, data: &str) {
        let Some(value) = data
            .trim()
            .strip_suffix(' ')
            .unwrap_or(data.trim())
            .parse::<usize>()
            .ok()
        else {
            return;
        };
        self.cursor_style = match value {
            3 | 4 => CursorStyle::Underline,
            5 | 6 => CursorStyle::Bar,
            _ => CursorStyle::Block,
        };
    }

    fn apply_sgr(&mut self, data: &str) {
        let values = parse_numbers(data);
        let values = if values.is_empty() { vec![0] } else { values };
        let mut iter = values.into_iter();
        while let Some(value) = iter.next() {
            match value {
                0 => self.style = CellStyle::default(),
                1 => self.style.bold = true,
                2 => self.style.dim = true,
                3 => self.style.italic = true,
                4 => self.style.underline = true,
                7 => self.style.inverse = true,
                8 => self.style.hidden = true,
                9 => self.style.strikethrough = true,
                22 => {
                    self.style.bold = false;
                    self.style.dim = false;
                }
                23 => self.style.italic = false,
                24 => self.style.underline = false,
                27 => self.style.inverse = false,
                28 => self.style.hidden = false,
                29 => self.style.strikethrough = false,
                30..=37 => self.style.foreground = ansi_color(value - 30),
                40..=47 => self.style.background = ansi_color(value - 40),
                90..=97 => self.style.foreground = ansi_color(value - 90 + 8),
                100..=107 => self.style.background = ansi_color(value - 100 + 8),
                39 => self.style.foreground = None,
                49 => self.style.background = None,
                38 | 48 => {
                    let target_foreground = value == 38;
                    match iter.next() {
                        Some(2) => {
                            let r = iter.next().unwrap_or(255).min(255) as u8;
                            let g = iter.next().unwrap_or(255).min(255) as u8;
                            let b = iter.next().unwrap_or(255).min(255) as u8;
                            self.set_sgr_color(target_foreground, Some(Rgb(r, g, b)));
                        }
                        Some(5) => {
                            let index = iter.next().unwrap_or(15).min(255);
                            self.set_sgr_color(target_foreground, xterm_256_color(index));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    fn set_sgr_color(&mut self, target_foreground: bool, color: Option<Rgb>) {
        if target_foreground {
            self.style.foreground = color;
        } else {
            self.style.background = color;
        }
    }

    fn apply_mode(&mut self, text: &str, enabled: bool) {
        let private = text.starts_with('?');
        for part in text.trim_start_matches('?').split(';') {
            if private {
                match part {
                    "1" => self.modes.application_cursor = enabled,
                    "6" => self.modes.origin_mode = enabled,
                    "7" => self.modes.auto_wrap = enabled,
                    "25" => self.modes.cursor_visible = enabled,
                    "66" => self.modes.application_keypad = enabled,
                    "1004" => self.modes.focus_reporting = enabled,
                    "1000" | "1002" | "1003" => self.modes.mouse_reporting = enabled,
                    "1006" => self.modes.sgr_mouse = enabled,
                    "1007" => self.modes.alternate_scroll = enabled,
                    "1047" | "1049" => self.set_alternate_screen(enabled),
                    "2004" => self.modes.bracketed_paste = enabled,
                    "2031" => self.report_color_scheme = enabled,
                    "2026" => {}
                    _ => {}
                }
            } else {
                match part {
                    "4" => self.modes.insert_mode = enabled,
                    "7" => self.modes.auto_wrap = enabled,
                    "20" => self.modes.line_feed_new_line = enabled,
                    _ => {
                        if let Some(flags) = part.strip_prefix('>') {
                            self.apply_keyboard_protocol_flags(flags, enabled);
                        }
                    }
                }
            }
        }
    }

    fn apply_keyboard_protocol_flags(&mut self, flags: &str, enabled: bool) {
        let flags = flags.parse::<usize>().unwrap_or_default();
        self.modes.keyboard_disambiguate = enabled && flags & 1 != 0;
        self.modes.keyboard_report_event_types = enabled && flags & 2 != 0;
        self.modes.keyboard_report_alternate_keys = enabled && flags & 4 != 0;
        self.modes.all_keys_as_escape = enabled && flags & 8 != 0;
        self.modes.keyboard_report_associated_text = enabled && flags & 16 != 0;
    }

    fn apply_osc(&mut self, data: &[u8], events: &mut Vec<TerminalEvent>) {
        let text = String::from_utf8_lossy(data);
        if let Some(values) = text.strip_prefix("4;") {
            let mut values = values.split(';');
            while let (Some(index), Some(value)) = (values.next(), values.next()) {
                if value == "?" {
                    if let Ok(index) = index.parse::<usize>() {
                        if index > 255 {
                            continue;
                        }
                        if let Some(Rgb(r, g, b)) = xterm_256_color(index) {
                            self.responses.extend_from_slice(
                                format!(
                                    "\x1b]4;{index};rgb:{:04x}/{:04x}/{:04x}\x1b\\",
                                    u16::from(r) * 257,
                                    u16::from(g) * 257,
                                    u16::from(b) * 257
                                )
                                .as_bytes(),
                            );
                        }
                    }
                }
            }
            return;
        }
        if let Some((code, values)) = text.split_once(';') {
            if let Ok(start @ 10..=12) = code.parse::<usize>() {
                for (offset, value) in values.split(';').enumerate() {
                    let code = start + offset;
                    if value == "?" && code <= 12 {
                        let Rgb(r, g, b) = self.default_colors[usize::from(code == 11)];
                        self.responses.extend_from_slice(
                            format!(
                                "\x1b]{code};rgb:{:04x}/{:04x}/{:04x}\x1b\\",
                                u16::from(r) * 257,
                                u16::from(g) * 257,
                                u16::from(b) * 257
                            )
                            .as_bytes(),
                        );
                    }
                }
                return;
            }
        }
        if let Some(title) = text.strip_prefix("0;").or_else(|| text.strip_prefix("2;")) {
            self.title = title.to_owned();
            events.push(TerminalEvent::TitleChanged(self.title.clone()));
        } else if let Some(payload) = text.strip_prefix("52;") {
            if self.osc52_policy.allows_write() {
                events.push(TerminalEvent::ClipboardWriteRequested(payload.to_owned()));
            }
        }
    }

    fn apply_dcs(&mut self, data: &[u8], events: &mut Vec<TerminalEvent>) {
        let text = String::from_utf8_lossy(data);
        if let Some(rest) = text.strip_prefix("terminaste;1;") {
            let mut parts = rest.splitn(3, ';');
            if let (Some(name), Some("json64"), Some(payload)) =
                (parts.next(), parts.next(), parts.next())
            {
                events.push(TerminalEvent::Integration(IntegrationEvent {
                    name: name.to_owned(),
                    payload: payload.to_owned(),
                }));
            }
        }
    }

    fn set_alternate_screen(&mut self, enabled: bool) {
        if enabled && self.alternate.is_none() {
            self.save_cursor();
            self.alternate = Some(
                (0..self.rows)
                    .map(|_| TerminalRow::new(self.cols))
                    .collect(),
            );
            self.cursor_col = 0;
            self.cursor_row = 0;
            self.modes.alternate_screen = true;
        } else if !enabled && self.alternate.is_some() {
            self.alternate = None;
            self.restore_cursor();
            self.modes.alternate_screen = false;
        }
    }

    fn write_printable_str(&mut self, text: &str) {
        for ch in text.chars() {
            let Some(width) = UnicodeWidthChar::width(ch) else {
                continue;
            };
            if width == 0 {
                self.append_combining_mark(ch);
                continue;
            }
            if self.modes.auto_wrap
                && (self.cursor_col >= self.cols || self.cursor_col + width > self.cols)
            {
                let row = self.cursor_row;
                self.active_rows_mut()[row].wrapped = true;
                self.cursor_col = 0;
                self.line_feed();
            }
            let width = width.min(self.cols);
            let col = self.cursor_col.min(self.cols - width);
            self.cursor_col = col;
            if self.modes.insert_mode {
                self.insert_cells(width);
            }
            let style = self.style;
            let row = self.cursor_row;
            let cols = self.cols;
            let active = self.active_rows_mut();
            active[row].cells[col] = TerminalCell {
                text: ch.to_string(),
                style,
                wide: width == 2,
                spacer: false,
                secret: false,
            };
            if width == 2 && col + 1 < cols {
                active[row].cells[col + 1] = TerminalCell {
                    text: String::new(),
                    style,
                    wide: false,
                    spacer: true,
                    secret: false,
                };
            }
            self.cursor_col = if self.modes.auto_wrap {
                self.cursor_col + width
            } else {
                (self.cursor_col + width).min(self.cols - 1)
            };
            self.append_pending_output_char(ch);
        }
    }

    fn append_combining_mark(&mut self, ch: char) {
        if let Some((row, col)) = self.previous_printable_cell_position() {
            let style = self.style;
            let cell = &mut self.active_rows_mut()[row].cells[col];
            cell.text.push(ch);
            cell.style = style;
        } else {
            self.write_printable_str("�");
        }
        self.append_pending_output_char(ch);
    }

    fn append_pending_output_char(&mut self, ch: char) {
        if !self.modes.alternate_screen
            && !self.dynamic_command_output
            && self.command_started_at.is_some()
            && self.pending_output.len()
                < self
                    .max_history_rows
                    .saturating_mul(self.cols)
                    .saturating_mul(4)
        {
            self.pending_output.push(ch);
        }
    }

    fn begin_dynamic_command_output(&mut self) {
        if self.modes.alternate_screen || !self.has_running_command() || self.dynamic_command_output
        {
            return;
        }
        self.pending_output.clear();
        if let Some(block) = self.blocks.iter_mut().rev().find(|block| block.running) {
            block.output.clear();
        }
        self.dynamic_command_output = true;
    }

    fn previous_printable_cell_position(&self) -> Option<(usize, usize)> {
        let rows = self.active_rows();
        let mut row = self.cursor_row.min(rows.len().saturating_sub(1));
        let mut col = self.cursor_col.min(self.cols);
        loop {
            while col > 0 {
                col -= 1;
                let cell = &rows[row].cells[col];
                if !cell.spacer && cell.text != " " && !cell.text.is_empty() {
                    return Some((row, col));
                }
            }
            if row == 0 || !rows[row - 1].wrapped {
                return None;
            }
            row -= 1;
            col = self.cols;
        }
    }

    fn insert_cells(&mut self, count: usize) {
        let row = self.cursor_row;
        let col = self.cursor_col;
        let cols = self.cols;
        let count = count.min(cols - col);
        let cells = &mut self.active_rows_mut()[row].cells;
        for index in (col + count..cols).rev() {
            cells[index] = cells[index - count].clone();
        }
        for cell in &mut cells[col..col + count] {
            *cell = TerminalCell::default();
        }
    }

    fn line_feed(&mut self) {
        if self.cursor_row == self.scroll_bottom {
            let top = self.scroll_top;
            let bottom = self.scroll_bottom;
            let removed = self.active_rows_mut().remove(top);
            let cols = self.cols;
            if self.modes.alternate_screen || top > 0 || bottom + 1 < self.rows {
                self.active_rows_mut()
                    .insert(bottom, TerminalRow::new(cols));
            } else {
                self.scrollback.push_back(removed);
                while self.scrollback.len() > self.max_history_rows {
                    self.scrollback.pop_front();
                    self.discarded_rows += 1;
                }
                self.visible.push(TerminalRow::new(self.cols));
            }
        } else {
            self.cursor_row = (self.cursor_row + 1).min(self.rows - 1);
        }
    }

    fn clear_screen(&mut self) {
        *self.active_rows_mut() = (0..self.rows)
            .map(|_| TerminalRow::new(self.cols))
            .collect();
        self.cursor_col = 0;
        self.cursor_row = 0;
    }

    fn erase_in_display(&mut self, mode: usize) {
        match mode {
            0 => {
                self.erase_in_line(0);
                let cols = self.cols;
                for row in self.cursor_row + 1..self.rows {
                    self.active_rows_mut()[row] = TerminalRow::new(cols);
                }
            }
            1 => {
                let cols = self.cols;
                for row in 0..self.cursor_row {
                    self.active_rows_mut()[row] = TerminalRow::new(cols);
                }
                self.erase_in_line(1);
            }
            2 => self.clear_screen_preserve_cursor(),
            3 if !self.modes.alternate_screen => {
                self.discarded_rows += self.scrollback.len() as u64;
                self.scrollback.clear();
            }
            _ => {}
        }
    }

    fn clear_screen_preserve_cursor(&mut self) {
        let cursor_col = self.cursor_col;
        let cursor_row = self.cursor_row;
        *self.active_rows_mut() = (0..self.rows)
            .map(|_| TerminalRow::new(self.cols))
            .collect();
        self.cursor_col = cursor_col;
        self.cursor_row = cursor_row;
    }

    fn erase_in_line(&mut self, mode: usize) {
        match mode {
            0 => self.clear_line_range(self.cursor_col, self.cols),
            1 => self.clear_line_range(0, self.cursor_col + 1),
            2 => self.clear_line_range(0, self.cols),
            _ => {}
        }
    }

    fn clear_line_range(&mut self, start: usize, end: usize) {
        let col = self.cursor_col;
        let cols = self.cols;
        let row = self.cursor_row;
        let start = start.min(cols);
        let end = end.min(cols).max(start);
        for cell in &mut self.active_rows_mut()[row].cells[start..end] {
            *cell = TerminalCell::default();
        }
        self.cursor_col = col;
    }

    fn save_cursor(&mut self) {
        self.saved_cursor = Some(SavedCursor {
            col: self.cursor_col,
            row: self.cursor_row,
            style: self.style,
        });
    }

    fn restore_cursor(&mut self) {
        if let Some(saved) = self.saved_cursor {
            self.cursor_col = saved.col.min(self.cols - 1);
            self.cursor_row = saved.row.min(self.rows - 1);
            self.style = saved.style;
        }
    }

    fn active_rows(&self) -> &[TerminalRow] {
        self.alternate.as_deref().unwrap_or(&self.visible)
    }

    fn active_rows_mut(&mut self) -> &mut Vec<TerminalRow> {
        self.alternate.as_mut().unwrap_or(&mut self.visible)
    }

    fn absolute_cursor_row(&self) -> u64 {
        self.discarded_rows + self.scrollback.len() as u64 + self.cursor_row as u64
    }

    fn cell_snapshots(&self) -> Vec<Vec<TerminalCellSnapshot>> {
        self.active_rows()
            .iter()
            .map(|row| {
                row.cells()
                    .iter()
                    .map(|cell| TerminalCellSnapshot {
                        text: cell.text.clone(),
                        style: cell.style,
                        width: if cell.spacer {
                            TerminalCellWidth::Spacer
                        } else if cell.wide {
                            TerminalCellWidth::Wide
                        } else {
                            TerminalCellWidth::Narrow
                        },
                        native_glyph: cell
                            .text
                            .chars()
                            .next()
                            .and_then(classify_native_block_glyph),
                        secret: cell.secret,
                    })
                    .collect()
            })
            .collect()
    }

    fn selection_ranges(&self) -> Vec<TerminalRange> {
        self.selection
            .map(TerminalSelection::range)
            .filter(|range| !range.is_empty())
            .into_iter()
            .collect()
    }

    fn text_for_range(&self, range: TerminalRange) -> String {
        let range = range.normalized();
        let mut out = String::new();
        for row_index in range.start.row..=range.end.row {
            if row_index > range.start.row {
                out.push('\n');
            }
            let Some(row) = self.row_at(row_index) else {
                continue;
            };
            let start_col = if row_index == range.start.row {
                range.start.col
            } else {
                0
            };
            let end_col = if row_index == range.end.row {
                range.end.col
            } else {
                self.cols
            };
            out.push_str(&row_text_range(row, start_col, end_col));
        }
        out.trim_end_matches('\n').to_owned()
    }

    fn row_at(&self, absolute_row: u64) -> Option<&TerminalRow> {
        if !self.modes.alternate_screen && absolute_row < self.visible_row_start() {
            self.scrollback_row(absolute_row)
        } else {
            self.visible_row(absolute_row)
        }
    }

    fn text_index(&self) -> IndexedText {
        let rows = if self.modes.alternate_screen {
            self.active_rows().to_vec()
        } else {
            self.scrollback
                .iter()
                .chain(self.visible.iter())
                .cloned()
                .collect()
        };
        build_text_index(
            &rows,
            if self.modes.alternate_screen {
                0
            } else {
                self.discarded_rows
            },
        )
    }

    fn detect_links(&self) -> Vec<TerminalLink> {
        detect_links(&self.text_index())
    }
}

#[derive(Debug, Clone)]
enum ParserState {
    Ground,
    Escape,
    Csi(Vec<u8>),
    Osc(Vec<u8>),
    Dcs(Vec<u8>),
    DiscardString(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Function(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub command: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInput {
    pub code: KeyCode,
    pub modifiers: Modifiers,
}

pub fn encode_key(input: KeyInput, modes: TerminalModes) -> Vec<u8> {
    if modes.all_keys_as_escape {
        return encode_csi_u(input);
    }
    match input.code {
        KeyCode::Char(ch) => encode_printable(ch, input.modifiers),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab if input.modifiers.shift => b"\x1b[Z".to_vec(),
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Escape => vec![ESC],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Delete => modified_tilde(3, input.modifiers),
        KeyCode::Insert => modified_tilde(2, input.modifiers),
        KeyCode::PageUp => modified_tilde(5, input.modifiers),
        KeyCode::PageDown => modified_tilde(6, input.modifiers),
        KeyCode::Up => movement(b'A', input.modifiers, modes.application_cursor),
        KeyCode::Down => movement(b'B', input.modifiers, modes.application_cursor),
        KeyCode::Right => movement(b'C', input.modifiers, modes.application_cursor),
        KeyCode::Left => movement(b'D', input.modifiers, modes.application_cursor),
        KeyCode::Home => movement(b'H', input.modifiers, modes.application_cursor),
        KeyCode::End => movement(b'F', input.modifiers, modes.application_cursor),
        KeyCode::Function(number) => function_key(number, input.modifiers),
    }
}

pub fn encode_paste(text: &str, modes: TerminalModes) -> Vec<u8> {
    if modes.bracketed_paste {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(text.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.as_bytes().to_vec()
    }
}

pub fn encode_mouse_sgr(button: u8, col: usize, row: usize, pressed: bool) -> Vec<u8> {
    format!(
        "\x1b[<{};{};{}{}",
        button,
        col + 1,
        row + 1,
        if pressed { 'M' } else { 'm' }
    )
    .into_bytes()
}

pub fn encode_mouse_wheel_sgr(lines: isize, col: usize, row: usize) -> Vec<u8> {
    let button = if lines < 0 { 64 } else { 65 };
    let mut out = Vec::new();
    for _ in 0..lines.unsigned_abs() {
        out.extend_from_slice(&encode_mouse_sgr(button, col, row, true));
    }
    out
}

pub fn encode_focus_event(focused: bool, modes: TerminalModes) -> Vec<u8> {
    if modes.focus_reporting {
        if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        }
    } else {
        Vec::new()
    }
}

pub fn encode_alternate_scroll(lines: isize, modes: TerminalModes) -> Vec<u8> {
    if !modes.alternate_screen || !modes.alternate_scroll {
        return Vec::new();
    }
    let code = if lines < 0 {
        KeyCode::Up
    } else {
        KeyCode::Down
    };
    let mut out = Vec::new();
    for _ in 0..lines.unsigned_abs() {
        out.extend_from_slice(&encode_key(
            KeyInput {
                code,
                modifiers: Modifiers::default(),
            },
            modes,
        ));
    }
    out
}

fn encode_printable(ch: char, modifiers: Modifiers) -> Vec<u8> {
    if modifiers.control && !modifiers.shift && !modifiers.alt && !modifiers.command {
        if let Some(byte) = control_mapping(ch) {
            return vec![byte];
        }
    }
    let mut out = Vec::new();
    if modifiers.alt {
        out.push(ESC);
    }
    let mut buf = [0; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    out
}

fn control_mapping(ch: char) -> Option<u8> {
    match ch {
        ' ' | '2' => Some(0x00),
        '3' => Some(0x1b),
        '4' => Some(0x1c),
        '5' => Some(0x1d),
        '6' => Some(0x1e),
        '7' => Some(0x1f),
        '8' => Some(0x7f),
        'a'..='z' => Some((ch as u8) - b'a' + 1),
        'A'..='Z' => Some((ch as u8) - b'A' + 1),
        _ => None,
    }
}

fn movement(final_byte: u8, modifiers: Modifiers, application_cursor: bool) -> Vec<u8> {
    if let Some(modifier) = xterm_modifier(modifiers) {
        format!("\x1b[1;{}{}", modifier, final_byte as char).into_bytes()
    } else if application_cursor {
        vec![ESC, b'O', final_byte]
    } else {
        vec![ESC, b'[', final_byte]
    }
}

fn modified_tilde(number: u8, modifiers: Modifiers) -> Vec<u8> {
    if let Some(modifier) = xterm_modifier(modifiers) {
        format!("\x1b[{};{}~", number, modifier).into_bytes()
    } else {
        format!("\x1b[{}~", number).into_bytes()
    }
}

fn function_key(number: u8, modifiers: Modifiers) -> Vec<u8> {
    let base = match number {
        1 => return modified_ss3_or_csi(b'P', modifiers),
        2 => return modified_ss3_or_csi(b'Q', modifiers),
        3 => return modified_ss3_or_csi(b'R', modifiers),
        4 => return modified_ss3_or_csi(b'S', modifiers),
        5 => 15,
        6 => 17,
        7 => 18,
        8 => 19,
        9 => 20,
        10 => 21,
        11 => 23,
        12 => 24,
        13 => 25,
        14 => 26,
        15 => 28,
        16 => 29,
        17 => 31,
        18 => 32,
        19 => 33,
        20 => 34,
        _ => 24,
    };
    modified_tilde(base, modifiers)
}

fn modified_ss3_or_csi(final_byte: u8, modifiers: Modifiers) -> Vec<u8> {
    if let Some(modifier) = xterm_modifier(modifiers) {
        format!("\x1b[1;{}{}", modifier, final_byte as char).into_bytes()
    } else {
        vec![ESC, b'O', final_byte]
    }
}

fn xterm_modifier(modifiers: Modifiers) -> Option<u8> {
    match (modifiers.shift, modifiers.alt, modifiers.control) {
        (false, false, false) => None,
        (true, false, false) => Some(2),
        (false, true, false) => Some(3),
        (true, true, false) => Some(4),
        (false, false, true) => Some(5),
        (true, false, true) => Some(6),
        (false, true, true) => Some(7),
        (true, true, true) => Some(8),
    }
}

fn encode_csi_u(input: KeyInput) -> Vec<u8> {
    let code = match input.code {
        KeyCode::Char(ch) => ch as u32,
        KeyCode::Enter => 13,
        KeyCode::Tab => 9,
        KeyCode::Escape => 27,
        KeyCode::Backspace => 127,
        KeyCode::Delete => 0x2326,
        KeyCode::Insert => 0x2380,
        KeyCode::Up => 0xF700,
        KeyCode::Down => 0xF701,
        KeyCode::Right => 0xF703,
        KeyCode::Left => 0xF702,
        KeyCode::Home => 0xF729,
        KeyCode::End => 0xF72B,
        KeyCode::PageUp => 0xF72C,
        KeyCode::PageDown => 0xF72D,
        KeyCode::Function(number) => 0xF704 + u32::from(number.saturating_sub(1)),
    };
    let modifiers = 1
        + u8::from(input.modifiers.shift)
        + 2 * u8::from(input.modifiers.alt)
        + 4 * u8::from(input.modifiers.control)
        + 8 * u8::from(input.modifiers.command);
    if modifiers == 1 {
        format!("\x1b[{}u", code).into_bytes()
    } else {
        format!("\x1b[{};{}u", code, modifiers).into_bytes()
    }
}

fn parse_numbers(data: &str) -> Vec<usize> {
    data.trim_start_matches('?')
        .split(';')
        .filter_map(|part| {
            if part.is_empty() {
                None
            } else {
                part.parse::<usize>().ok()
            }
        })
        .collect()
}

fn compare_points(left: TerminalPoint, right: TerminalPoint) -> Ordering {
    left.row.cmp(&right.row).then(left.col.cmp(&right.col))
}

#[derive(Debug, Clone)]
struct IndexedChar {
    byte_index: usize,
    point: TerminalPoint,
}

#[derive(Debug, Clone)]
struct IndexedText {
    text: String,
    chars: Vec<IndexedChar>,
}

fn build_text_index(rows: &[TerminalRow], start_row: u64) -> IndexedText {
    let mut text = String::new();
    let mut chars = Vec::new();
    for (row_offset, row) in rows.iter().enumerate() {
        if row_offset > 0 {
            chars.push(IndexedChar {
                byte_index: text.len(),
                point: TerminalPoint {
                    row: start_row + row_offset as u64,
                    col: 0,
                },
            });
            text.push('\n');
        }
        for (col, cell) in row.cells().iter().enumerate() {
            if cell.spacer {
                continue;
            }
            for (offset, ch) in cell.text.char_indices() {
                chars.push(IndexedChar {
                    byte_index: text.len() + offset,
                    point: TerminalPoint {
                        row: start_row + row_offset as u64,
                        col,
                    },
                });
                let _ = ch;
            }
            text.push_str(&cell.text);
        }
    }
    IndexedText { text, chars }
}

fn find_ranges(index: &IndexedText, query: &str) -> Vec<TerminalRange> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    while let Some(found) = index.text[offset..].find(query) {
        let start_byte = offset + found;
        let end_byte = start_byte + query.len();
        if let Some(range) = byte_range_to_terminal_range(index, start_byte, end_byte) {
            ranges.push(range);
        }
        offset = next_char_boundary(&index.text, start_byte);
    }
    ranges
}

fn detect_links(index: &IndexedText) -> Vec<TerminalLink> {
    index
        .text
        .split_whitespace()
        .scan(0, |offset, token| {
            let relative = index.text[*offset..].find(token).unwrap_or_default();
            let start = *offset + relative;
            *offset = start + token.len();
            Some((start, trim_link_token(token)))
        })
        .filter_map(|(start, token)| {
            if !(token.starts_with("http://") || token.starts_with("https://")) {
                return None;
            }
            byte_range_to_terminal_range(index, start, start + token.len()).map(|range| {
                TerminalLink {
                    url: token.to_owned(),
                    range,
                }
            })
        })
        .collect()
}

fn trim_link_token(token: &str) -> &str {
    token.trim_end_matches(['.', ',', ';', ':', ')', ']', '}'])
}

fn byte_range_to_terminal_range(
    index: &IndexedText,
    start_byte: usize,
    end_byte: usize,
) -> Option<TerminalRange> {
    let start = point_at_byte(index, start_byte)?;
    let mut end = point_at_byte(index, end_byte).unwrap_or_else(|| {
        index
            .chars
            .last()
            .map(|ch| TerminalPoint {
                row: ch.point.row,
                col: ch.point.col + 1,
            })
            .unwrap_or(start)
    });
    if end == start {
        end.col += 1;
    }
    Some(TerminalRange { start, end })
}

fn point_at_byte(index: &IndexedText, byte_index: usize) -> Option<TerminalPoint> {
    if byte_index >= index.text.len() {
        return index.chars.last().map(|ch| TerminalPoint {
            row: ch.point.row,
            col: ch.point.col + 1,
        });
    }
    index
        .chars
        .iter()
        .find(|ch| ch.byte_index >= byte_index)
        .map(|ch| ch.point)
}

fn next_char_boundary(text: &str, byte_index: usize) -> usize {
    text[byte_index..]
        .chars()
        .next()
        .map(|ch| byte_index + ch.len_utf8())
        .unwrap_or(text.len())
}

fn row_text_range(row: &TerminalRow, start_col: usize, end_col: usize) -> String {
    let mut text = String::new();
    for cell in row.cells()[start_col.min(row.cells().len())..end_col.min(row.cells().len())].iter()
    {
        if !cell.spacer {
            text.push_str(&cell.text);
        }
    }
    text.trim_end().to_owned()
}

pub fn classify_native_block_glyph(ch: char) -> Option<NativeBlockGlyph> {
    match ch {
        '\u{2580}'..='\u{259f}' => Some(NativeBlockGlyph::Block),
        '\u{2500}'..='\u{257f}' => Some(NativeBlockGlyph::BoxDrawing),
        '\u{e0b0}'..='\u{e0bf}' => Some(NativeBlockGlyph::Powerline),
        _ => None,
    }
}

fn reflow_rows(rows: &[TerminalRow], cols: usize) -> Vec<TerminalRow> {
    let mut output = vec![TerminalRow::new(cols)];
    let mut row_index = 0;
    let mut col = 0;
    for (index, row) in rows.iter().enumerate() {
        let end = if row.wrapped {
            row.cells.len()
        } else {
            row.cells
                .iter()
                .rposition(|cell| cell.text != " " || cell.style != CellStyle::default())
                .map_or(0, |col| col + 1)
        };
        for cell in &row.cells[..end] {
            if cell.spacer {
                continue;
            }
            let width = (if cell.wide { 2 } else { 1 }).min(cols);
            if col + width > cols {
                output[row_index].wrapped = true;
                output.push(TerminalRow::new(cols));
                row_index += 1;
                col = 0;
            }
            output[row_index].cells[col] = cell.clone();
            if width == 2 && col + 1 < cols {
                output[row_index].cells[col + 1] = TerminalCell {
                    text: String::new(),
                    style: cell.style,
                    wide: false,
                    spacer: true,
                    secret: cell.secret,
                };
            }
            col += width;
            if col == cols && row.wrapped {
                output[row_index].wrapped = true;
                output.push(TerminalRow::new(cols));
                row_index += 1;
                col = 0;
            }
        }
        if !row.wrapped && index + 1 < rows.len() {
            output.push(TerminalRow::new(cols));
            row_index += 1;
            col = 0;
        }
    }
    output
}

fn last_rows(mut rows: Vec<TerminalRow>, wanted: usize, cols: usize) -> Vec<TerminalRow> {
    if rows.len() > wanted {
        rows = rows.split_off(rows.len() - wanted);
    }
    while rows.len() < wanted {
        rows.push(TerminalRow::new(cols));
    }
    rows
}

fn ansi_color(index: usize) -> Option<Rgb> {
    Some(match index {
        0 => Rgb(15, 17, 23),
        1 => Rgb(239, 68, 68),
        2 => Rgb(34, 197, 94),
        3 => Rgb(245, 158, 11),
        4 => Rgb(59, 130, 246),
        5 => Rgb(139, 92, 246),
        6 => Rgb(6, 182, 212),
        7 => Rgb(248, 250, 252),
        8 => Rgb(100, 116, 139),
        9 => Rgb(248, 113, 113),
        10 => Rgb(74, 222, 128),
        11 => Rgb(251, 191, 36),
        12 => Rgb(96, 165, 250),
        13 => Rgb(167, 139, 250),
        14 => Rgb(34, 211, 238),
        15 => Rgb(255, 255, 255),
        _ => return None,
    })
}

fn xterm_256_color(index: usize) -> Option<Rgb> {
    if index < 16 {
        return ansi_color(index);
    }
    if index < 232 {
        let value = index - 16;
        let r = xterm_256_component(value / 36);
        let g = xterm_256_component((value / 6) % 6);
        let b = xterm_256_component(value % 6);
        return Some(Rgb(r, g, b));
    }
    let gray = 8 + ((index - 232) * 10) as u8;
    Some(Rgb(gray, gray, gray))
}

fn xterm_256_component(value: usize) -> u8 {
    if value == 0 {
        0
    } else {
        (55 + value * 40) as u8
    }
}

fn millis(duration: Duration) -> u64 {
    duration.as_secs().saturating_mul(1000) + u64::from(duration.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_scheme_queries_and_notifications_follow_terminal_colors() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"\x1b[?2031$p\x1b[?996n\x1b[?2031h\x1b[?2031$p");
        assert_eq!(
            terminal.take_responses(),
            b"\x1b[?2031;2$y\x1b[?997;1n\x1b[?2031;1$y"
        );
        terminal.set_default_colors(Rgb(0, 0, 0), Rgb(255, 255, 255));
        assert_eq!(terminal.take_responses(), b"\x1b[?997;2n");
        terminal.set_default_colors(Rgb(0, 0, 0), Rgb(255, 255, 255));
        assert!(terminal.take_responses().is_empty());
        terminal.process_bytes(b"\x1b[?2031l");
        terminal.set_default_colors(Rgb(255, 255, 255), Rgb(0, 0, 0));
        assert!(terminal.take_responses().is_empty());
        terminal.process_bytes(b"\x1b[?996n");
        assert_eq!(terminal.take_responses(), b"\x1b[?997;1n");
    }

    #[test]
    fn palette_queries_reply_to_each_requested_entry() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"\x1b]4;16;?;231;?;256;?\x1b\\");
        assert_eq!(
            terminal.take_responses(),
            b"\x1b]4;16;rgb:0000/0000/0000\x1b\\\x1b]4;231;rgb:ffff/ffff/ffff\x1b\\"
        );
    }

    #[test]
    fn color_queries_report_current_theme_with_both_terminators() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.set_default_colors(Rgb(238, 242, 255), Rgb(10, 11, 15));
        for byte in b"\x1b]10;?;?\x07" {
            terminal.process_bytes(&[*byte]);
        }
        assert_eq!(
            terminal.take_responses(),
            b"\x1b]10;rgb:eeee/f2f2/ffff\x1b\\\x1b]11;rgb:0a0a/0b0b/0f0f\x1b\\"
        );
        terminal.set_default_colors(Rgb(0, 0, 0), Rgb(255, 255, 255));
        terminal.process_bytes(b"\x1b]11;?\x1b\\");
        assert_eq!(
            terminal.take_responses(),
            b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\"
        );
    }

    #[test]
    fn printable_bytes_update_cells_and_cursor() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"ok");
        let snapshot = terminal.snapshot();
        assert_eq!(snapshot.visible_lines[0], "ok");
        assert_eq!(snapshot.cursor_col, 2);
    }

    #[test]
    fn utf8_multibyte_prints_once() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes("é界".as_bytes());
        let snapshot = terminal.snapshot();
        assert_eq!(snapshot.visible_lines[0], "é界");
        assert_eq!(snapshot.cursor_col, 3);
    }

    #[test]
    fn sgr_styles_apply_and_reset() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"\x1b[31;1mR\x1b[0mN");
        assert!(terminal.visible[0].cells()[0].style.bold);
        assert_eq!(
            terminal.visible[0].cells()[0].style.foreground,
            Some(Rgb(239, 68, 68))
        );
        assert!(!terminal.visible[0].cells()[1].style.bold);
    }

    #[test]
    fn scrollback_truncates_front() {
        let mut terminal = TerminalModel::new(4, 1, 2);
        terminal.process_bytes(b"one\ntwo\nthree\nfour\n");
        assert!(terminal.discarded_rows() > 0);
        assert!(terminal.scrollback.len() <= 2);
    }

    #[test]
    fn alternate_screen_restores_main_grid() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"main");
        terminal.process_bytes(b"\x1b[?1049halt\x1b[?1049l");
        assert_eq!(terminal.snapshot().visible_lines[0], "main");
    }

    #[test]
    fn bracketed_paste_wraps_when_enabled() {
        let modes = TerminalModes {
            bracketed_paste: true,
            ..Default::default()
        };
        assert_eq!(encode_paste("x", modes), b"\x1b[200~x\x1b[201~");
    }

    #[test]
    fn modified_arrows_use_xterm_modifier() {
        let bytes = encode_key(
            KeyInput {
                code: KeyCode::Left,
                modifiers: Modifiers {
                    control: true,
                    ..Modifiers::default()
                },
            },
            TerminalModes::default(),
        );
        assert_eq!(bytes, b"\x1b[1;5D");
    }

    #[test]
    fn private_dcs_is_event_not_printable() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        let events = terminal.process_bytes(b"\x1bPterminaste;1;ready;json64;e30\x1b\\");
        assert!(matches!(events[0], TerminalEvent::Integration(_)));
        assert_eq!(terminal.snapshot().visible_lines[0], "");
    }

    #[test]
    fn utf8_invalid_byte_keeps_valid_tail() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(&[0xc3, 0xa9, 0xff, b'x']);
        assert_eq!(terminal.snapshot().visible_lines[0], "é�x");
    }

    #[test]
    fn utf8_can_span_multiple_chunks() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(&[0xe7]);
        terminal.process_bytes(&[0x95]);
        terminal.process_bytes(&[0x8c]);
        assert_eq!(terminal.snapshot().visible_lines[0], "界");
        assert_eq!(terminal.snapshot().cursor_col, 2);
    }

    #[test]
    fn erase_line_variants() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"abcdef\x1b[1;4H\x1b[1K");
        assert_eq!(terminal.snapshot().visible_lines[0], "    ef");
        terminal.process_bytes(b"\x1b[1;2H\x1b[0K");
        assert_eq!(terminal.snapshot().visible_lines[0], "");
    }

    #[test]
    fn erase_display_variants_preserve_cursor() {
        let mut terminal = TerminalModel::new(5, 3, 100);
        terminal.process_bytes(b"abc\r\ndef\r\nghi\x1b[2;2H\x1b[0J");
        let snapshot = terminal.snapshot();
        assert_eq!(snapshot.visible_lines, vec!["abc", "d", ""]);
        assert_eq!((snapshot.cursor_col, snapshot.cursor_row), (1, 1));
    }

    #[test]
    fn insert_mode_shifts_cells_right() {
        let mut terminal = TerminalModel::new(8, 2, 100);
        terminal.process_bytes(b"abcd\x1b[1;3H\x1b[4hX");
        assert_eq!(terminal.snapshot().visible_lines[0], "abXcd");
    }

    #[test]
    fn cursor_save_restore_round_trip() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"ab\x1b7cd\x1b8Z");
        let snapshot = terminal.snapshot();
        assert_eq!(snapshot.visible_lines[0], "abZd");
        assert_eq!(snapshot.cursor_col, 3);
    }

    #[test]
    fn bright_and_256_color_sgr_apply() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"\x1b[91mR\x1b[38;5;196mX\x1b[48;5;232mB");
        assert_eq!(
            terminal.visible[0].cells()[0].style.foreground,
            Some(Rgb(248, 113, 113))
        );
        assert_eq!(
            terminal.visible[0].cells()[1].style.foreground,
            Some(Rgb(255, 0, 0))
        );
        assert_eq!(
            terminal.visible[0].cells()[2].style.background,
            Some(Rgb(8, 8, 8))
        );
    }

    #[test]
    fn running_command_output_updates_before_command_end() {
        let mut terminal = TerminalModel::new(20, 4, 100);
        terminal.start_integrated_command("echo ok".to_owned());
        terminal.process_bytes(b"ok\n");
        let snapshot = terminal.snapshot();

        assert_eq!(snapshot.blocks.len(), 1);
        assert!(snapshot.blocks[0].running);
        assert_eq!(snapshot.blocks[0].output, "ok");
    }

    #[test]
    fn cursor_redraws_switch_running_commands_to_the_terminal_surface() {
        let mut terminal = TerminalModel::new(40, 6, 100);
        terminal.start_integrated_command("opencode".to_owned());
        terminal.process_bytes(b"Updating 1/3");

        assert!(!terminal.running_command_uses_terminal_surface());

        terminal.process_bytes(b"\x1b[1G\x1b[2KUpdating 2/3");

        assert!(terminal.running_command_uses_terminal_surface());
        assert_eq!(terminal.snapshot().blocks[0].output, "");

        terminal.finish_running_command(0);

        assert!(!terminal.running_command_uses_terminal_surface());

        terminal.start_integrated_command("progress".to_owned());
        terminal.process_bytes(b"Step 1\rStep 2");

        assert!(terminal.running_command_uses_terminal_surface());
    }

    #[test]
    fn terminal_modes_parse_focus_alternate_scroll_and_keyboard_flags() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"\x1b[?1004h\x1b[?1007h\x1b[>13h");
        let modes = terminal.modes();
        assert!(modes.focus_reporting);
        assert!(modes.alternate_scroll);
        assert!(modes.keyboard_disambiguate);
        assert!(modes.keyboard_report_alternate_keys);
        assert!(modes.all_keys_as_escape);
        assert!(!modes.keyboard_report_event_types);
    }

    #[test]
    fn osc52_is_policy_gated() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        assert!(terminal.process_bytes(b"\x1b]52;c;AAAA\x07").is_empty());
        terminal.set_osc52_policy(Osc52Policy::WriteOnly);
        assert_eq!(
            terminal.process_bytes(b"\x1b]52;c;AAAA\x07"),
            vec![TerminalEvent::ClipboardWriteRequested("c;AAAA".to_owned())]
        );
    }

    #[test]
    fn resize_reflows_wrapped_rows() {
        let mut terminal = TerminalModel::new(4, 3, 100);
        terminal.process_bytes(b"abcdef");
        terminal.resize(6, 3);
        assert_eq!(terminal.snapshot().visible_lines[0], "abcdef");
    }

    #[test]
    fn resize_and_cancel_preserve_spaces_and_blank_lines() {
        let mut terminal = TerminalModel::new(60, 8, 100);
        terminal.start_integrated_command("server".to_owned());
        terminal.process_bytes(
            b"server listening on http://0.0.0.0:5678\r\n\r\n  indented  output\r\n",
        );
        terminal.resize(45, 10);
        terminal.process_bytes(b"\r^C\r\n");
        terminal.finish_running_command(130);
        assert_eq!(
            terminal.snapshot().blocks[0].output,
            "server listening on http://0.0.0.0:5678\n\n  indented  output\n^C"
        );
    }

    #[test]
    fn resize_keeps_spaces_across_soft_wraps() {
        let mut terminal = TerminalModel::new(8, 8, 100);
        terminal.process_bytes(b"one  two  three\r\n\r\n  four");
        terminal.resize(24, 8);
        assert_eq!(
            &terminal.snapshot().visible_lines[..3],
            &["one  two  three", "", "  four"]
        );
    }

    #[test]
    fn command_output_keeps_tab_spacing() {
        let mut terminal = TerminalModel::new(40, 4, 100);
        terminal.start_integrated_command("dir".to_owned());
        terminal.process_bytes(b"one\ttwo\r\n\r\n  three\tfour\r\n");
        terminal.finish_running_command(0);
        assert_eq!(
            terminal.snapshot().blocks[0].output,
            "one     two\n\n  three four"
        );
    }

    #[test]
    fn mouse_wheel_repeats_sgr_sequences() {
        assert_eq!(
            encode_mouse_wheel_sgr(-2, 1, 2),
            b"\x1b[<64;2;3M\x1b[<64;2;3M"
        );
    }

    #[test]
    fn focus_event_only_when_enabled() {
        assert!(encode_focus_event(true, TerminalModes::default()).is_empty());
        let modes = TerminalModes {
            focus_reporting: true,
            ..Default::default()
        };
        assert_eq!(encode_focus_event(false, modes), b"\x1b[O");
    }

    #[test]
    fn alternate_scroll_encodes_arrows() {
        let modes = TerminalModes {
            alternate_screen: true,
            alternate_scroll: true,
            ..Default::default()
        };
        assert_eq!(encode_alternate_scroll(-2, modes), b"\x1b[A\x1b[A");
    }

    #[test]
    fn snapshot_includes_cell_metadata_cursor_selection_and_links() {
        let mut terminal = TerminalModel::new(30, 2, 100);
        terminal.process_bytes(b"\x1b[4mhttps://x.test \x1b[ q");
        terminal.set_visible_selection(0, 0, 0, 5);
        let snapshot = terminal.snapshot();
        assert_eq!(snapshot.cells[0][0].text, "h");
        assert!(snapshot.cells[0][0].style.underline);
        assert_eq!(snapshot.cells[0][0].width, TerminalCellWidth::Narrow);
        assert!(snapshot.cursor_visible);
        assert_eq!(snapshot.cursor_style, CursorStyle::Block);
        assert_eq!(snapshot.selection_ranges.len(), 1);
        assert_eq!(snapshot.link_ranges[0].url, "https://x.test");
    }

    #[test]
    fn selected_text_extracts_across_scrollback_and_visible_rows() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"alpha\r\nbeta\r\ngamma");
        terminal.set_selection(
            TerminalPoint { row: 1, col: 1 },
            TerminalPoint { row: 2, col: 3 },
        );
        assert_eq!(terminal.selected_text(), "eta\ngam");
    }

    #[test]
    fn search_returns_scrollback_and_visible_ranges() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"one\r\ntwo\r\none");
        let ranges = terminal.search("one");
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].start, TerminalPoint { row: 0, col: 0 });
        assert_eq!(ranges[1].start, TerminalPoint { row: 2, col: 0 });
    }

    #[test]
    fn links_detect_url_ranges() {
        let mut terminal = TerminalModel::new(40, 2, 100);
        terminal.process_bytes(b"go https://example.test/path, now");
        let links = terminal.links();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://example.test/path");
        assert_eq!(links[0].range.start, TerminalPoint { row: 0, col: 3 });
    }

    #[test]
    fn scrollback_exposes_absolute_row_indexing() {
        let mut terminal = TerminalModel::new(20, 1, 10);
        terminal.process_bytes(b"first\r\nsecond\r\nthird");
        assert_eq!(terminal.scrollback_len(), 2);
        assert_eq!(
            terminal.row_text(terminal.discarded_rows()).as_deref(),
            Some("first")
        );
        assert_eq!(
            terminal.row_text(terminal.visible_row_start()).as_deref(),
            Some("third")
        );
    }

    #[test]
    fn combining_mark_appends_without_advancing_cursor() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes("e\u{301}x".as_bytes());
        let snapshot = terminal.snapshot();
        assert_eq!(snapshot.cells[0][0].text, "e\u{301}");
        assert_eq!(snapshot.visible_lines[0], "e\u{301}x");
        assert_eq!(snapshot.cursor_col, 2);
    }

    #[test]
    fn native_block_glyphs_are_classified() {
        assert_eq!(
            classify_native_block_glyph('█'),
            Some(NativeBlockGlyph::Block)
        );
        assert_eq!(
            classify_native_block_glyph('─'),
            Some(NativeBlockGlyph::BoxDrawing)
        );
        assert_eq!(
            classify_native_block_glyph('\u{e0b0}'),
            Some(NativeBlockGlyph::Powerline)
        );
        assert_eq!(classify_native_block_glyph('x'), None);
    }

    #[test]
    fn render_thickness_constants_match_design_multiplier() {
        assert_eq!(TERMINAL_CURSOR_THICKNESS_MULTIPLIER, 0.15);
        assert_eq!(TERMINAL_UNDERLINE_THICKNESS_MULTIPLIER, 0.15);
    }

    #[test]
    fn dec_private_modes_round_trip_independently() {
        let mut terminal = TerminalModel::new(10, 2, 100);
        terminal.process_bytes(b"\x1b[?1;25;66;1000;1006;1007;2004h");
        let modes = terminal.modes();
        assert!(modes.application_cursor);
        assert!(modes.cursor_visible);
        assert!(modes.application_keypad);
        assert!(modes.mouse_reporting);
        assert!(modes.sgr_mouse);
        assert!(modes.alternate_scroll);
        assert!(modes.bracketed_paste);

        terminal.process_bytes(b"\x1b[?1;25;66;1000;1006;1007;2004l");
        let modes = terminal.modes();
        assert!(!modes.application_cursor);
        assert!(!modes.cursor_visible);
        assert!(!modes.application_keypad);
        assert!(!modes.mouse_reporting);
        assert!(!modes.sgr_mouse);
        assert!(!modes.alternate_scroll);
        assert!(!modes.bracketed_paste);
    }
}
