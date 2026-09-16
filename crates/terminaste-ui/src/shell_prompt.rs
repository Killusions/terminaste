use terminaste_core::{TerminalCellSnapshot, TerminalModel, TerminalSnapshot};

use super::TerminalPane;

pub(super) struct PromptLayout {
    pub header: Vec<Vec<TerminalCellSnapshot>>,
    pub prefix: Vec<TerminalCellSnapshot>,
    pub suffix: Vec<TerminalCellSnapshot>,
}

impl PromptLayout {
    pub fn new(pane: &TerminalPane, columns: usize, max_rows: usize) -> Self {
        let snapshot = snapshot(
            pane.surface
                .prompt_ansi
                .as_deref()
                .unwrap_or(&pane.input_prompt_label()),
            usize::from(pane.surface.prompt_cols.unwrap_or(pane.cols)),
            usize::from(pane.rows),
        );
        let first = snapshot
            .cells
            .iter()
            .position(|row| has_text(row))
            .unwrap_or(snapshot.cursor_row);
        let row = &snapshot.cells[snapshot.cursor_row];
        let cursor = snapshot.cursor_col.min(row.len());
        let prefix = row[..cursor].to_vec();
        let mut suffix = trim_end(&row[cursor..]).to_vec();
        if let Some(right) = pane
            .surface
            .right_prompt_ansi
            .as_deref()
            .filter(|text| !text.is_empty())
        {
            let right = snapshot_text(right, columns);
            suffix.extend(right);
        }
        let mut header =
            snapshot.cells[first.min(snapshot.cursor_row)..snapshot.cursor_row].to_vec();
        let mut prefix = trim_prefix(
            prefix,
            columns.saturating_sub(suffix.len()).max(1),
            &mut header,
        );
        if header.len() > max_rows {
            header = header.split_off(header.len() - max_rows);
        }
        for row in &mut header {
            align_right_section(row, columns);
        }
        prefix.truncate(columns.saturating_sub(suffix.len()).max(1));
        Self {
            header,
            prefix,
            suffix,
        }
    }
}

fn trim_prefix(
    mut prefix: Vec<TerminalCellSnapshot>,
    columns: usize,
    header: &mut Vec<Vec<TerminalCellSnapshot>>,
) -> Vec<TerminalCellSnapshot> {
    while prefix.len() > columns {
        header.push(prefix.drain(..columns).collect());
    }
    prefix
}

fn align_right_section(row: &mut Vec<TerminalCellSnapshot>, columns: usize) {
    let end = trim_end(row).len();
    let blank = |cell: &TerminalCellSnapshot| {
        cell.text.trim().is_empty() && cell.style == terminaste_core::CellStyle::default()
    };
    let section = (2..end)
        .rev()
        .find(|&index| !blank(&row[index]) && blank(&row[index - 1]) && blank(&row[index - 2]));
    if let Some(start) = section {
        let right = row[start..end].to_vec();
        let left = trim_end(&row[..start]).to_vec();
        if left.len() + right.len() < columns {
            let padding = row[start - 1].clone();
            *row = left;
            row.resize(columns - right.len(), padding);
            row.extend(right);
        } else {
            row.truncate(columns);
        }
    } else {
        row.truncate(end.min(columns));
    }
}

fn snapshot(text: &str, cols: usize, rows: usize) -> TerminalSnapshot {
    let mut terminal = TerminalModel::new(cols.max(1), rows.max(1), rows.max(1));
    terminal.process_bytes(b"\x1b[20h");
    terminal.process_bytes(text.as_bytes());
    terminal.snapshot()
}

pub(super) fn output_snapshot(text: &str, cols: usize, rows: usize) -> TerminalSnapshot {
    let mut terminal = TerminalModel::new(cols.max(1), rows.max(1), rows.max(1));
    terminal.process_bytes(b"\x1b[20h");
    terminal.process_bytes(text.as_bytes());
    terminal.snapshot()
}

fn snapshot_text(text: &str, cols: usize) -> Vec<TerminalCellSnapshot> {
    let snapshot = snapshot(text, cols, 4);
    snapshot
        .cells
        .iter()
        .filter(|row| has_text(row))
        .flat_map(|row| {
            let row = trim_end(row);
            let start = row
                .iter()
                .position(|cell| !cell.text.trim().is_empty())
                .unwrap_or(row.len());
            row[start..].to_vec()
        })
        .collect()
}
fn has_text(row: &[TerminalCellSnapshot]) -> bool {
    row.iter().any(|cell| !cell.text.trim().is_empty())
}
fn trim_end(row: &[TerminalCellSnapshot]) -> &[TerminalCellSnapshot] {
    &row[..row
        .iter()
        .rposition(|cell| !cell.text.trim().is_empty())
        .map_or(0, |index| index + 1)]
}

pub(super) fn plain_text(text: &str) -> String {
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => match chars.next() {
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']' | 'P' | '^' | '_') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' | '\t' => output.push(ch),
            '\u{8}' | '\u{7f}' => {
                output.pop();
            }
            ch if ch.is_control() => {}
            ch => output.push(ch),
        }
    }
    output.trim_end_matches('\n').to_owned()
}
