use super::*;

fn layout(text: &str, columns: usize, rows: usize) -> OutputLayout {
    let cell = size(px(8.), px(16.8));
    let snapshot = shell_prompt::output_snapshot(text, columns, rows);
    let rows = snapshot
        .cells
        .iter()
        .rposition(|row| row.iter().any(|cell| !cell.text.trim().is_empty()))
        .map_or(1, |row| row + 1);
    output_layout(
        &snapshot,
        rows,
        Bounds::new(
            point(px(12.), px(30.)),
            size(cell.width * columns as f32, cell.height * rows as f32),
        ),
        cell,
    )
}

#[test]
fn selection_uses_rendered_cells_for_tabs_unicode_and_cursor_movements() {
    let layout = layout(
        "\x1b[31mone\t界e\u{301}\x1b[0m\nprogress 10%\r\x1b[2Kdone",
        24,
        10,
    );
    assert_eq!(layout.text, "one     界e\u{301}\ndone");
    let origin = layout.input.bounds.origin;
    let index = layout.input.index_at(origin + point(px(8. * 8.), px(1.)));
    assert!(layout.text[index..].starts_with('界'));
    let index = layout.input.index_at(origin + point(px(8. * 10.), px(1.)));
    assert!(layout.text[index..].starts_with("e\u{301}"));
    let index = layout.input.index_at(origin + point(px(0.), px(17.8)));
    assert_eq!(&layout.text[index..], "done");
}

#[test]
fn soft_wrapping_does_not_insert_newlines_into_copied_text() {
    let layout = layout("abcdefghij界next", 10, 5);
    assert_eq!(layout.text, "abcdefghij界next");
    let index = layout
        .input
        .index_at(layout.input.bounds.origin + point(px(0.), px(17.8)));
    assert_eq!(&layout.text[index..], "界next");
}

#[test]
fn long_running_output_keeps_selection_on_the_displayed_rows() {
    let text = (0..1200)
        .map(|index| format!("line{index:04}\n"))
        .collect::<String>();
    let previous = layout(&text, 20, 1_000);
    let index = previous.text.find("line0750").unwrap();
    let position = previous.input.point_for(index);
    assert_eq!(
        previous.input.index_at(position + point(px(0.), px(1.))),
        index
    );
    let mut editor = CommandEditorState::new();
    editor.set_text(previous.text.clone());
    editor.move_cursor(index, false);
    editor.move_cursor(index + 8, true);
    let next = layout(&format!("{text}line1200\n"), 20, 1_000);
    next.update_selection(&previous, &mut editor);
    assert_eq!(editor.selected_text(), Some("line0750"));
    assert_eq!(editor.text(), next.text);
    assert_eq!(next.first_row, previous.first_row + 1);
}
