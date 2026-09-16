use std::ops::Range;

use gpui::{App, Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window};
use unicode_width::UnicodeWidthChar;

use super::view::TerminalWindow;

#[derive(Debug, Clone, Default)]
pub(super) struct CommandEditorState {
    pub text: String,
    pub cursor: usize,
    pub selection_anchor: Option<usize>,
    pub preedit: String,
    pub marked_range: Option<Range<usize>>,
    pub shell_bridge: bool,
    pub shell_command_mode: bool,
    pub focus_requested: bool,
}

impl CommandEditorState {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.selection_anchor = None;
        self.marked_range = None;
        self.preedit.clear();
    }
    pub fn clear(&mut self) {
        self.set_text(String::new());
    }
    pub fn selection_range(&self) -> Option<Range<usize>> {
        let anchor = self.selection_anchor?;
        (anchor != self.cursor).then_some(anchor.min(self.cursor)..anchor.max(self.cursor))
    }
    pub fn selected_text(&self) -> Option<&str> {
        self.selection_range().map(|range| &self.text[range])
    }
    pub fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.text.len();
    }
    pub fn insert_text(&mut self, value: &str) {
        let range = self.selection_range().unwrap_or(self.cursor..self.cursor);
        self.replace(range, value);
    }
    pub fn replace(&mut self, range: Range<usize>, value: &str) {
        let start = self.clamp_index(range.start);
        let end = self.clamp_index(range.end).max(start);
        self.text.replace_range(start..end, value);
        self.cursor = start + value.len();
        self.selection_anchor = None;
    }
    pub fn delete_selection(&mut self) -> bool {
        if let Some(range) = self.selection_range() {
            self.replace(range, "");
            true
        } else {
            false
        }
    }
    pub fn backspace(&mut self) {
        if !self.delete_selection() {
            self.replace(previous_boundary(&self.text, self.cursor)..self.cursor, "");
        }
    }
    pub fn delete_forward(&mut self) {
        if !self.delete_selection() {
            self.replace(self.cursor..next_boundary(&self.text, self.cursor), "");
        }
    }
    pub fn move_left(&mut self, selecting: bool) {
        let target = if !selecting {
            self.selection_range().map(|range| range.start)
        } else {
            None
        };
        self.move_cursor(
            target.unwrap_or_else(|| previous_boundary(&self.text, self.cursor)),
            selecting,
        );
    }
    pub fn move_right(&mut self, selecting: bool) {
        let target = if !selecting {
            self.selection_range().map(|range| range.end)
        } else {
            None
        };
        self.move_cursor(
            target.unwrap_or_else(|| next_boundary(&self.text, self.cursor)),
            selecting,
        );
    }
    pub fn move_home(&mut self, selecting: bool) {
        self.move_cursor(
            self.text[..self.cursor]
                .rfind('\n')
                .map_or(0, |index| index + 1),
            selecting,
        );
    }
    pub fn move_end(&mut self, selecting: bool) {
        self.move_cursor(
            self.text[self.cursor..]
                .find('\n')
                .map_or(self.text.len(), |index| self.cursor + index),
            selecting,
        );
    }
    pub fn move_cursor(&mut self, index: usize, selecting: bool) {
        if selecting {
            self.selection_anchor.get_or_insert(self.cursor);
        } else {
            self.selection_anchor = None;
        }
        self.cursor = self.clamp_index(index);
    }
    pub fn set_cursor_from_mouse_index(&mut self, index: usize, selecting: bool) {
        self.move_cursor(index, selecting);
    }
    pub fn select_word_at(&mut self, index: usize) {
        let index = self.clamp_index(index);
        let start = self.text[..index]
            .rfind(|ch: char| ch.is_whitespace())
            .map_or(0, |index| index + 1);
        let end = self.text[index..]
            .find(char::is_whitespace)
            .map_or(self.text.len(), |offset| index + offset);
        self.selection_anchor = Some(start);
        self.cursor = end;
    }
    pub fn select_line_at(&mut self, index: usize) {
        self.cursor = self.clamp_index(index);
        self.move_home(false);
        self.selection_anchor = Some(self.cursor);
        self.move_end(true);
        if self.cursor < self.text.len() {
            self.cursor += 1;
        }
    }
    pub fn clamp_index(&self, index: usize) -> usize {
        let mut index = index.min(self.text.len());
        while !self.text.is_char_boundary(index) {
            index -= 1;
        }
        index
    }
}

fn previous_boundary(text: &str, index: usize) -> usize {
    text[..index]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}
fn next_boundary(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(text.len(), |ch| index + ch.len_utf8())
}

#[derive(Clone, Default)]
pub(super) struct InputLayout {
    pub bounds: Bounds<Pixels>,
    pub positions: Vec<(usize, Point<Pixels>)>,
    pub line_height: Pixels,
    pub cell_width: Pixels,
}

impl InputLayout {
    pub fn index_at(&self, point: Point<Pixels>) -> usize {
        self.positions
            .iter()
            .min_by(|(_, a), (_, b)| {
                let distance = |position: &Point<Pixels>| {
                    let row = ((point.y - position.y) / self.line_height).floor().abs();
                    row * 1_000_000.0 + f32::from((point.x - position.x).abs())
                };
                distance(a).total_cmp(&distance(b))
            })
            .map_or(0, |(index, _)| *index)
    }
    pub fn point_for(&self, index: usize) -> Point<Pixels> {
        self.positions
            .iter()
            .rev()
            .find(|(offset, _)| *offset <= index)
            .map_or(self.bounds.origin, |(_, point)| *point)
    }
    pub fn move_row(&self, editor: &mut CommandEditorState, delta: isize, selecting: bool) -> bool {
        let current = self.point_for(editor.cursor);
        let target = Point {
            x: current.x,
            y: current.y + self.line_height * delta as f32,
        };
        let index = self.index_at(target);
        if self.point_for(index).y == current.y {
            return false;
        }
        editor.move_cursor(index, selecting);
        true
    }
}

pub(super) fn wrapped_rows(text: &str, columns: usize) -> Vec<Range<usize>> {
    let columns = columns.max(1);
    let mut rows = Vec::new();
    let mut start = 0;
    let mut width = 0;
    for (index, ch) in text.char_indices() {
        if ch == '\n' {
            rows.push(start..index);
            start = index + 1;
            width = 0;
            continue;
        }
        let advance = ch.width().unwrap_or(0);
        if width > 0 && width + advance > columns {
            rows.push(start..index);
            start = index;
            width = 0;
        }
        width += advance;
    }
    rows.push(start..text.len());
    rows
}

pub(super) fn from_utf16(text: &str, offset: usize) -> usize {
    let mut count = 0;
    for (index, ch) in text.char_indices() {
        if count >= offset {
            return index;
        }
        count += ch.len_utf16();
    }
    text.len()
}
fn to_utf16(text: &str, offset: usize) -> usize {
    text[..offset].encode_utf16().count()
}
fn from_range(text: &str, range: Range<usize>) -> Range<usize> {
    from_utf16(text, range.start)..from_utf16(text, range.end)
}
fn to_range(text: &str, range: Range<usize>) -> Range<usize> {
    to_utf16(text, range.start)..to_utf16(text, range.end)
}

impl EntityInputHandler for TerminalWindow {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let editor = self.input_editor()?;
        let range = from_range(editor.text(), range);
        *actual = Some(to_range(editor.text(), range.clone()));
        Some(editor.text[range].to_owned())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let editor = self.input_editor()?;
        Some(UTF16Selection {
            range: to_range(
                editor.text(),
                editor
                    .selection_range()
                    .unwrap_or(editor.cursor..editor.cursor),
            ),
            reversed: editor
                .selection_anchor
                .is_some_and(|anchor| anchor > editor.cursor),
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let editor = self.input_editor()?;
        editor
            .marked_range
            .clone()
            .map(|range| to_range(editor.text(), range))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.input_editor_mut() {
            editor.marked_range = None;
            editor.preedit.clear();
        }
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.direct_input() {
            if let Some(pane) = self.app.active_terminal_mut() {
                pane.write_terminal(text.as_bytes().to_vec());
            }
        } else if let Some(editor) = self.input_editor_mut() {
            let range = range
                .map(|range| from_range(editor.text(), range))
                .or(editor.marked_range.clone())
                .or(editor.selection_range())
                .unwrap_or(editor.cursor..editor.cursor);
            editor.replace(range, text);
            editor.marked_range = None;
            editor.preedit.clear();
            self.input_changed();
        }
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.input_editor_mut() {
            let range = range
                .map(|range| from_range(editor.text(), range))
                .or(editor.marked_range.clone())
                .or(editor.selection_range())
                .unwrap_or(editor.cursor..editor.cursor);
            let start = range.start;
            editor.replace(range, text);
            editor.preedit = text.to_owned();
            editor.marked_range = (!text.is_empty()).then_some(start..start + text.len());
            if let Some(selected) = selected {
                let selected = from_range(text, selected);
                editor.selection_anchor = Some(start + selected.start);
                editor.cursor = start + selected.end;
            }
        }
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let editor = self.input_editor()?;
        let start = self
            .input_layout
            .point_for(from_utf16(editor.text(), range.start));
        Some(Bounds::new(
            start,
            gpui::size(self.input_layout.cell_width, self.input_layout.line_height),
        ))
    }
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let editor = self.input_editor()?;
        Some(to_utf16(editor.text(), self.input_layout.index_at(point)))
    }
}

pub(super) fn paint_input(
    editor: &CommandEditorState,
    bounds: Bounds<Pixels>,
    style: &super::theme::TerminalTextStyle,
    focused: bool,
    window: &mut Window,
    cx: &mut App,
) -> InputLayout {
    let super::theme::TerminalTextStyle {
        font,
        font_size,
        cell,
        theme,
        drop_background_if_readable: _,
    } = style.clone();
    let rows = wrapped_rows(
        editor.text(),
        (bounds.size.width / cell.width).floor() as usize,
    );
    let cursor_row = rows
        .iter()
        .rposition(|range| range.start <= editor.cursor)
        .unwrap_or(0);
    let visible = (bounds.size.height / cell.height).floor().max(1.0) as usize;
    let first = if focused {
        cursor_row.saturating_sub(visible - 1)
    } else {
        0
    };
    let mut layout = InputLayout {
        bounds,
        line_height: cell.height,
        cell_width: cell.width,
        positions: Vec::new(),
    };
    for (row, range) in rows.iter().enumerate() {
        let origin =
            bounds.origin + gpui::point(gpui::px(0.), cell.height * (row as f32 - first as f32));
        let text = &editor.text[range.clone()];
        let mut x = origin.x;
        for (index, ch) in text.char_indices() {
            layout
                .positions
                .push((range.start + index, gpui::point(x, origin.y)));
            x += cell.width * ch.width().unwrap_or(0) as f32;
        }
        layout.positions.push((range.end, gpui::point(x, origin.y)));
        if row < first || row >= first + visible {
            continue;
        }
        if let Some(selection) = editor.selection_range() {
            let start = selection.start.max(range.start);
            let end = selection.end.min(range.end);
            if start < end {
                let a = layout.point_for(start);
                let b = layout.point_for(end);
                window.paint_quad(gpui::fill(
                    Bounds::new(a, gpui::size(b.x - a.x, cell.height)),
                    theme.selection,
                ));
            }
        }
        let line = window.text_system().shape_line(
            text.to_owned().into(),
            font_size,
            &[gpui::TextRun {
                len: text.len(),
                font: font.clone(),
                color: theme.text,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            Some(cell.width),
        );
        let _ = line.paint(origin, cell.height, gpui::TextAlign::Left, None, window, cx);
        if let Some(marked) = &editor.marked_range {
            let start = marked.start.max(range.start);
            let end = marked.end.min(range.end);
            if start < end {
                let a = layout.point_for(start);
                let b = layout.point_for(end);
                window.paint_quad(gpui::fill(
                    Bounds::new(
                        a + gpui::point(gpui::px(0.), cell.height - gpui::px(1.)),
                        gpui::size(b.x - a.x, gpui::px(1.)),
                    ),
                    theme.text,
                ));
            }
        }
    }
    if focused {
        window.paint_quad(gpui::fill(
            Bounds::new(
                layout.point_for(editor.cursor),
                gpui::size(gpui::px(1.), cell.height),
            ),
            theme.text,
        ));
    }
    layout
}
