use std::sync::Arc;

use super::*;
use terminaste_core::{TerminalCellSnapshot, TerminalCellWidth, TerminalSnapshot};

pub(super) const INSET: f32 = 4.0;
pub(super) const EDITOR_PADDING_Y: f32 = 2.0;

pub(super) struct PromptLayout {
    header: Arc<egui::Galley>,
    prefix: Arc<egui::Galley>,
    suffix: Arc<egui::Galley>,
    prefix_top: f32,
    pub header_height: f32,
    pub prefix_width: f32,
    pub suffix_width: f32,
    pub input_height: f32,
}

impl PromptLayout {
    pub fn new(pane: &TerminalPane, ui: &egui::Ui, font: FontId, theme: &TerminasteTheme) -> Self {
        let width = (ui.available_width() - INSET * 2.0).max(1.0);
        let pixels_per_point = ui.ctx().pixels_per_point();
        let cell_width = ui.fonts(|fonts| {
            let glyph_width = fonts.glyph_width(&font, 'M');
            (glyph_width * pixels_per_point).ceil() / pixels_per_point
        });
        let snapshot = prompt_snapshot(
            pane.surface
                .prompt_ansi
                .as_deref()
                .unwrap_or(&pane.input_prompt_label()),
            pane.surface.prompt_cols.unwrap_or(pane.cols),
            pane.rows,
        );
        let first = snapshot
            .cells
            .iter()
            .position(|row| row_has_text(row))
            .unwrap_or(snapshot.cursor_row);
        let input_row = &snapshot.cells[snapshot.cursor_row];
        let cursor = snapshot.cursor_col.min(input_row.len());
        let header_job = rows_job(
            ui,
            &snapshot.cells[first.min(snapshot.cursor_row)..snapshot.cursor_row],
            &font,
            theme,
            cell_width,
            width,
        );
        let prefix_job = cells_job(ui, &input_row[..cursor], &font, theme, cell_width);
        let mut suffix_job =
            cells_job(ui, trim_end(&input_row[cursor..]), &font, theme, cell_width);
        if let Some(right) = pane
            .surface
            .right_prompt_ansi
            .as_deref()
            .filter(|text| !text.is_empty())
        {
            let right = prompt_snapshot(
                right,
                pane.surface.prompt_cols.unwrap_or(pane.cols),
                pane.rows,
            );
            let first = right
                .cells
                .iter()
                .position(|row| row_has_text(row))
                .unwrap_or(0);
            let end = right
                .cells
                .iter()
                .rposition(|row| row_has_text(row))
                .map_or(first, |row| row + 1);
            let right_job =
                compact_rows_job(ui, &right.cells[first..end], &font, theme, cell_width);
            if !suffix_job.text.is_empty() && !right_job.text.is_empty() {
                suffix_job.append(" ", 0.0, egui::TextFormat::simple(font.clone(), theme.text));
            }
            append_job(&mut suffix_job, &right_job);
        }
        let header = layout(ui, header_job, f32::INFINITY);
        let suffix = layout(ui, suffix_job, width * 0.45);
        let suffix_width = if suffix.job.text.is_empty() {
            0.0
        } else {
            suffix.size().x.max(suffix.mesh_bounds.right()) + 4.0
        };
        let minimum_input = cell_width * 4.0;
        let prefix = layout(
            ui,
            prefix_job,
            (width - suffix_width - minimum_input).max(width * 0.2),
        );
        let last_row = prefix.rows.last();
        let prefix_top = last_row.map_or(0.0, |row| row.rect.top());
        let prefix_width = last_row.map_or(0.0, |row| row.rect.right());
        let input_height =
            last_row
                .map_or(0.0, |row| row.height())
                .max(if suffix.job.text.is_empty() {
                    0.0
                } else {
                    suffix.size().y
                });
        let header_height = (if header.job.text.is_empty() {
            0.0
        } else {
            header.size().y
        }) + prefix_top;
        Self {
            header,
            prefix,
            suffix,
            prefix_top,
            header_height,
            prefix_width,
            suffix_width,
            input_height,
        }
    }

    pub fn editor_width(&self, width: f32) -> f32 {
        (width - INSET * 2.0 - self.prefix_width - self.suffix_width - 2.0).max(1.0)
    }

    pub fn paint(
        &self,
        painter: &egui::Painter,
        content: egui::Rect,
        input_top: f32,
        theme: &TerminasteTheme,
    ) {
        painter.galley(content.min, self.header.clone(), theme.text);
        if self.prefix_top > 0.0 {
            let rect = egui::Rect::from_min_max(
                content.min + egui::vec2(0.0, self.header_height - self.prefix_top),
                egui::pos2(content.right(), input_top),
            );
            painter
                .with_clip_rect(rect.intersect(painter.clip_rect()))
                .galley(rect.min, self.prefix.clone(), theme.text);
        }
        let input_rect =
            egui::Rect::from_min_max(egui::pos2(content.left(), input_top), content.max);
        let input_painter = painter.with_clip_rect(input_rect.intersect(painter.clip_rect()));
        input_painter.galley(
            egui::pos2(
                content.left(),
                input_top + EDITOR_PADDING_Y - self.prefix_top,
            ),
            self.prefix.clone(),
            theme.text,
        );
        if !self.suffix.job.text.is_empty() {
            input_painter.galley(
                egui::pos2(
                    content.right() - self.suffix.size().x.max(self.suffix.mesh_bounds.right()),
                    input_top + EDITOR_PADDING_Y,
                ),
                self.suffix.clone(),
                theme.text,
            );
        }
    }
}

fn layout(ui: &egui::Ui, mut job: egui::text::LayoutJob, width: f32) -> Arc<egui::Galley> {
    job.wrap.max_width = width;
    job.wrap.break_anywhere = true;
    ui.fonts(|fonts| fonts.layout_job(job))
}

fn prompt_snapshot(text: &str, cols: u16, rows: u16) -> TerminalSnapshot {
    let mut terminal = TerminalModel::new(usize::from(cols), usize::from(rows), usize::from(rows));
    terminal.process_bytes(b"\x1b[20h");
    terminal.process_bytes(text.as_bytes());
    terminal.snapshot()
}

fn row_has_text(row: &[TerminalCellSnapshot]) -> bool {
    row.iter().any(|cell| !cell.text.trim().is_empty())
}

fn trim_end(row: &[TerminalCellSnapshot]) -> &[TerminalCellSnapshot] {
    let end = row
        .iter()
        .rposition(|cell| !cell.text.trim().is_empty())
        .map_or(0, |index| index + 1);
    &row[..end]
}

fn rows_job(
    ui: &egui::Ui,
    rows: &[Vec<TerminalCellSnapshot>],
    font: &FontId,
    theme: &TerminasteTheme,
    cell_width: f32,
    width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            job.append(
                "\n",
                0.0,
                egui::TextFormat::simple(font.clone(), theme.text),
            );
        }
        let cells = trim_end(row);
        let mut line = cells_job(ui, cells, font, theme, cell_width);
        if cells.len() >= row.len().saturating_sub(4) {
            let blank = |cell: &TerminalCellSnapshot| {
                cell.text.trim().is_empty() && cell.style == terminaste_core::CellStyle::default()
            };
            if let Some(start) = (2..cells.len()).rev().find(|&index| {
                !blank(&cells[index]) && blank(&cells[index - 1]) && blank(&cells[index - 2])
            }) {
                let section = cells[..start]
                    .iter()
                    .filter(|cell| cell.width != TerminalCellWidth::Spacer)
                    .count();
                let galley = layout(ui, line.clone(), f32::INFINITY);
                let right = galley.size().x.max(galley.mesh_bounds.right());
                let pixels_per_point = ui.ctx().pixels_per_point();
                let extra = ((width - right) * pixels_per_point).floor() / pixels_per_point;
                line.sections[section].leading_space += extra;
            }
        }
        append_job(&mut job, &line);
    }
    job
}

fn compact_rows_job(
    ui: &egui::Ui,
    rows: &[Vec<TerminalCellSnapshot>],
    font: &FontId,
    theme: &TerminasteTheme,
    cell_width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            job.append(
                "\n",
                0.0,
                egui::TextFormat::simple(font.clone(), theme.text),
            );
        }
        let row = trim_end(row);
        let start = row
            .iter()
            .position(|cell| !cell.text.trim().is_empty())
            .unwrap_or(row.len());
        append_job(
            &mut job,
            &cells_job(ui, &row[start..], font, theme, cell_width),
        );
    }
    job
}

fn append_job(job: &mut egui::text::LayoutJob, other: &egui::text::LayoutJob) {
    for section in &other.sections {
        job.append(
            &other.text[section.byte_range.clone()],
            section.leading_space,
            section.format.clone(),
        );
    }
}

fn cells_job(
    ui: &egui::Ui,
    cells: &[TerminalCellSnapshot],
    font: &FontId,
    theme: &TerminasteTheme,
    cell_width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let pixels_per_point = ui.ctx().pixels_per_point();
    let mut leading_space = 0.0;
    for cell in cells {
        if cell.width == TerminalCellWidth::Spacer {
            continue;
        }
        let rgb = |color: terminaste_core::Rgb| Color32::from_rgb(color.0, color.1, color.2);
        let mut foreground = cell.style.foreground.map(rgb).unwrap_or(theme.text);
        let mut background = cell
            .style
            .background
            .map(rgb)
            .unwrap_or(Color32::TRANSPARENT);
        if cell.style.inverse {
            std::mem::swap(&mut foreground, &mut background);
        }
        if cell.style.dim {
            foreground = foreground.gamma_multiply(0.6);
        }
        let mut format = egui::TextFormat::simple(font.clone(), foreground);
        format.background = background;
        format.italics = cell.style.italic;
        if cell.style.underline {
            format.underline = Stroke::new(1.0_f32, foreground);
        }
        let text = if cell.text.is_empty() {
            " "
        } else {
            &cell.text
        };
        job.append(text, leading_space, format);
        let glyph_width = ui
            .painter()
            .layout_no_wrap(text.to_owned(), font.clone(), foreground)
            .size()
            .x;
        let columns = if cell.width == TerminalCellWidth::Wide {
            2.0
        } else {
            1.0
        };
        leading_space =
            cell_width * columns - (glyph_width * pixels_per_point).round() / pixels_per_point;
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styled_prompt_fits_the_reported_columns_at_each_display_scale() {
        for scale in [1.0, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut pane = TerminalPane::fake();
                        let settings = Settings::default();
                        pane.resize_from_ui(ui, &settings);
                        pane.surface.prompt_cols = Some(pane.cols);
                        let mut prompt = String::new();
                        for index in 0..pane.cols - 1 {
                            prompt.push_str(if index % 2 == 0 {
                                "\x1b[31mX"
                            } else {
                                "\x1b[32mX"
                            });
                        }
                        prompt.push_str("\r\n> ");
                        pane.surface.prompt_ansi = Some(prompt);
                        let prompt = PromptLayout::new(
                            &pane,
                            ui,
                            FontId::monospace(settings.font.size),
                            &TerminasteTheme::dark(),
                        );
                        assert_eq!(prompt.header.rows.len(), 1);
                        assert!(prompt.header.size().x <= ui.available_width() - INSET * 2.0);
                        let pixels_per_point = ui.ctx().pixels_per_point();
                        let cell_width = ui.fonts(|fonts| {
                            (fonts.glyph_width(&FontId::monospace(settings.font.size), 'M')
                                * pixels_per_point)
                                .ceil()
                                / pixels_per_point
                        });
                        let expected_width = f32::from(pane.cols - 1) * cell_width;
                        assert!(
                            (prompt.header.size().x - expected_width).abs() <= 1.0,
                            "scale={scale}, columns={}, cell_width={cell_width}, header_width={}, expected_width={expected_width}",
                            pane.cols,
                            prompt.header.size().x,
                        );
                    });
                },
            );
        }
    }

    #[test]
    fn multiline_prompt_right_sections_reach_the_content_edge() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut pane = TerminalPane::fake();
                pane.cols = 80;
                pane.surface.prompt_cols = Some(80);
                pane.surface.prompt_ansi = Some(format!(
                    "╭{}RAM\r\n├ ~/projects{}27s\r\n╰ ",
                    " ".repeat(72),
                    " ".repeat(61)
                ));
                let prompt =
                    PromptLayout::new(&pane, ui, FontId::monospace(13.0), &TerminasteTheme::dark());
                let right = ui.available_width() - INSET * 2.0;
                for row in &prompt.header.rows {
                    assert!(
                        (row.rect.right() - right).abs() < 1.0,
                        "{:?} vs {right}",
                        row.rect
                    );
                }
                assert!(prompt.prefix_width < 30.0);
            });
        });
    }

    #[test]
    fn right_prompt_ink_fits_after_resizing_at_each_display_scale() {
        for scale in [1.0, 1.5, 2.0] {
            for width in [420.0, 801.0] {
                let ctx = egui::Context::default();
                ctx.set_pixels_per_point(scale);
                install_system_fonts(&ctx, &Settings::default().font.family);
                let _ = ctx.run(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 500.0))),
                    ..Default::default()
                }, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut pane = TerminalPane::fake();
                        pane.cols = 80;
                        pane.surface.prompt_cols = Some(80);
                        pane.surface.prompt_ansi = Some(format!("╭{}\x1b[48;2;40;40;40mRAM IP\x1b[0m\r\n├ ~/projects{}\x1b[48;2;40;40;40m✓\x1b[0m\r\n╰ ", " ".repeat(72), " ".repeat(66)));
                        pane.surface.right_prompt_ansi = Some("✓".to_owned());
                        let prompt = PromptLayout::new(&pane, ui, FontId::monospace(13.0), &TerminasteTheme::dark());
                        let right = ui.available_width() - INSET * 2.0;
                        for row in &prompt.header.rows {
                            let edge = row.visuals.mesh_bounds.right().max(row.rect.right());
                            assert!(edge <= right + 0.01, "scale={scale}, width={width}, edge={edge}, right={right}");
                            assert!(right - edge < 1.5, "the right alignment must stay tight");
                        }
                        let content = ui.available_rect_before_wrap().shrink(INSET);
                        prompt.paint(ui.painter(), content, content.top() + prompt.header_height, &TerminasteTheme::dark());
                    });
                });
            }
        }
    }

    #[test]
    fn mixed_prompt_symbols_keep_the_last_column_aligned() {
        for scale in [1.0, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            install_system_fonts(&ctx, &Settings::default().font.family);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let font = FontId::monospace(13.0);
                    let cell_width =
                        ui.fonts(|fonts| (fonts.glyph_width(&font, 'M') * scale).ceil() / scale);
                    let snapshot =
                        prompt_snapshot("╭─ RAM ↓↑ 33.6G \x1b[32m界 ✓ ~/projects\x1b[0m Z", 80, 4);
                    let cells = trim_end(&snapshot.cells[0]);
                    let galley = layout(
                        ui,
                        cells_job(ui, cells, &font, &TerminasteTheme::dark(), cell_width),
                        f32::INFINITY,
                    );
                    let last = galley.rows[0].glyphs.last().unwrap();
                    assert_eq!(last.chr, 'Z');
                    assert!(
                        (last.pos.x - (cells.len() - 1) as f32 * cell_width).abs() < 1.0,
                        "scale={scale}, x={}, columns={}",
                        last.pos.x,
                        cells.len()
                    );
                });
            });
        }
    }

    #[test]
    fn prompt_cursor_movements_and_last_line_are_split_at_the_input_cursor() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut pane = TerminalPane::fake();
                pane.surface.prompt_ansi = Some(
                    "\x1b]133;A\x07\n\n\n\x1b[A\x1b[A\x1b[A\x1b[32mfirst\nsecond\n> \x1b]133;B\x07"
                        .to_owned(),
                );
                pane.surface.right_prompt_ansi = Some("\x1b[20Ggit:main".to_owned());
                let prompt =
                    PromptLayout::new(&pane, ui, FontId::monospace(13.0), &TerminasteTheme::dark());
                assert_eq!(prompt.header.job.text, "first\nsecond");
                assert_eq!(prompt.prefix.job.text, "> ");
                assert_eq!(prompt.suffix.job.text, "git:main");
                assert_eq!(prompt.header.rows.len(), 2);
                assert!(prompt.prefix_width > 0.0);
                assert!(prompt.suffix_width > 0.0);
                assert!(
                    prompt.editor_width(ui.available_width()) < ui.available_width() - INSET * 2.0
                );
            });
        });
    }
}
