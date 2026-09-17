use std::ops::Range;

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum FindPart {
    Command,
    Output,
}

#[derive(Clone, PartialEq)]
pub(super) struct FindMatch {
    pane: Uuid,
    block: Uuid,
    part: FindPart,
    range: Range<usize>,
    block_index: usize,
    offset: Pixels,
}

pub(super) struct FindOutput {
    source: String,
    columns: usize,
    text: String,
    row_starts: Vec<usize>,
}

impl FindOutput {
    fn new(source: String, columns: usize) -> Self {
        let snapshot = shell_prompt::output_snapshot(&source, columns, 1_000);
        let rows = output_rows(&snapshot);
        let layout = output_layout(
            &snapshot,
            rows,
            Bounds::new(
                point(px(0.), px(0.)),
                size(px(columns as f32), px(rows as f32)),
            ),
            size(px(1.), px(1.)),
        );
        let mut row_starts = Vec::with_capacity(rows);
        for (index, position) in &layout.input.positions {
            if position.y >= px(row_starts.len() as f32) {
                row_starts.push(*index);
            }
        }
        Self {
            source,
            columns,
            text: layout.text,
            row_starts,
        }
    }

    fn row_for(&self, index: usize) -> usize {
        self.row_starts
            .partition_point(|start| *start <= index)
            .saturating_sub(1)
    }
}

fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    text.to_ascii_lowercase()
        .match_indices(&query.to_ascii_lowercase())
        .map(|(start, matched)| start..start + matched.len())
        .collect()
}

fn next_match(current: Option<usize>, count: usize, backwards: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match current {
        None => {
            if backwards {
                count - 1
            } else {
                0
            }
        }
        Some(index) => {
            if backwards {
                (index + count - 1) % count
            } else {
                (index + 1) % count
            }
        }
    })
}

impl TerminalWindow {
    pub(super) fn current_find_offset(&self, block: Uuid, part: FindPart) -> Option<usize> {
        self.find_current
            .and_then(|index| self.find_matches.get(index))
            .filter(|found| found.block == block && found.part == part)
            .map(|found| found.range.start)
    }

    pub(super) fn refresh_find(&mut self) {
        let panes = self.app.tabs[self.app.active_tab]
            .panes
            .iter()
            .map(|pane| pane.id)
            .collect::<Vec<_>>();
        if !self.find_dirty && self.find_panes == panes {
            return;
        }
        self.find_panes = panes;
        self.find_dirty = false;
        let current = self
            .find_current
            .and_then(|index| self.find_matches.get(index))
            .cloned();
        self.find_matches.clear();
        self.find_ranges.clear();
        if self.find_query.is_empty() {
            self.find_current = None;
            self.find_output_cache.clear();
            return;
        }
        let mut retained = std::collections::HashSet::new();
        for pane in &self.app.tabs[self.app.active_tab].panes {
            let columns = self
                .pane_views
                .get(&pane.id)
                .map_or(usize::from(pane.cols), |state| state.output_columns)
                .max(1);
            for (block_index, block) in pane.model.command_blocks().iter().enumerate() {
                let start = self.find_matches.len();
                for range in match_ranges(&block.command, &self.find_query) {
                    self.find_matches.push(FindMatch {
                        pane: pane.id,
                        block: block.id,
                        part: FindPart::Command,
                        range,
                        block_index,
                        offset: px(0.),
                    });
                }
                self.find_ranges.insert(
                    (block.id, FindPart::Command),
                    start..self.find_matches.len(),
                );
                let output = if block.running && block.output.is_empty() {
                    strip_running_command_echo(&pane.model.running_command_output(), &block.command)
                } else {
                    block.output.clone()
                };
                if output.is_empty() {
                    continue;
                }
                retained.insert(block.id);
                if !self
                    .find_output_cache
                    .get(&block.id)
                    .is_some_and(|cached| cached.source == output && cached.columns == columns)
                {
                    self.find_output_cache
                        .insert(block.id, FindOutput::new(output, columns));
                }
                let output = &self.find_output_cache[&block.id];
                let start = self.find_matches.len();
                for range in match_ranges(&output.text, &self.find_query) {
                    let offset = px(28.) + self.cell.height * output.row_for(range.start) as f32;
                    self.find_matches.push(FindMatch {
                        pane: pane.id,
                        block: block.id,
                        part: FindPart::Output,
                        range,
                        block_index,
                        offset,
                    });
                }
                self.find_ranges
                    .insert((block.id, FindPart::Output), start..self.find_matches.len());
            }
        }
        self.find_output_cache.retain(|id, _| retained.contains(id));
        self.find_current = current.and_then(|current| {
            self.find_matches.iter().position(|item| {
                item.pane == current.pane
                    && item.block == current.block
                    && item.part == current.part
                    && item.range == current.range
            })
        });
    }

    pub(super) fn advance_find(&mut self, backwards: bool) {
        self.refresh_find();
        self.find_current = next_match(self.find_current, self.find_matches.len(), backwards);
        if let Some(found) = self
            .find_current
            .and_then(|index| self.find_matches.get(index))
        {
            let tab = &mut self.app.tabs[self.app.active_tab];
            if let Some(index) = tab.panes.iter().position(|pane| pane.id == found.pane) {
                tab.active_pane = index;
            }
            if let Some(state) = self.pane_views.get(&found.pane) {
                state.history_list.scroll_to(gpui::ListOffset {
                    item_ix: found.block_index,
                    offset_in_item: (found.offset - px(160.)).max(px(0.)),
                });
            }
        }
    }

    pub(super) fn paint_find_matches(
        &self,
        block: Uuid,
        part: FindPart,
        layout: &InputLayout,
        window: &mut Window,
    ) {
        let Some(range) = self
            .find_ranges
            .get(&(block, part))
            .filter(|range| !range.is_empty())
        else {
            return;
        };
        let mut matches = self.find_matches[range.clone()]
            .iter()
            .enumerate()
            .map(|(index, found)| (range.start + index, found))
            .peekable();
        for pair in layout.positions.windows(2) {
            let (index, origin) = pair[0];
            let (end, next) = pair[1];
            while matches
                .peek()
                .is_some_and(|(_, found)| found.range.end <= index)
            {
                matches.next();
            }
            if let Some((match_index, found)) = matches.peek() {
                if found.range.start < end && origin.y == next.y && next.x > origin.x {
                    let color = if self.find_current == Some(*match_index) {
                        self.theme.search_current
                    } else {
                        self.theme.search_match
                    };
                    window.paint_quad(fill(
                        Bounds::new(origin, size(next.x - origin.x, layout.line_height)),
                        color,
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_preserve_byte_offsets_and_include_every_occurrence() {
        let text = "界 origin ORIGIN\norigin";
        let matches = match_ranges(text, "origin");
        assert_eq!(matches.len(), 3);
        assert_eq!(&text[matches[0].clone()], "origin");
        assert_eq!(&text[matches[1].clone()], "ORIGIN");
        assert!(match_ranges(text, "").is_empty());
    }

    #[test]
    fn first_enter_selects_a_match_and_navigation_wraps() {
        assert_eq!(next_match(None, 3, false), Some(0));
        assert_eq!(next_match(Some(0), 3, false), Some(1));
        assert_eq!(next_match(Some(2), 3, false), Some(0));
        assert_eq!(next_match(Some(0), 3, true), Some(2));
        assert_eq!(next_match(None, 0, false), None);
    }

    #[test]
    fn output_search_uses_visible_text_and_tracks_wrapped_rows() {
        let output = FindOutput::new(
            "old text\r\x1b[2K\x1b[31morigin origin\x1b[0m".to_owned(),
            8,
        );
        assert_eq!(output.text, "origin origin");
        assert!(match_ranges(&output.text, "old").is_empty());
        let matches = match_ranges(&output.text, "origin");
        assert_eq!(matches.len(), 2);
        assert_eq!(output.row_for(matches[0].start), 0);
        assert_eq!(output.row_for(matches[1].end - 1), 1);
    }
}
