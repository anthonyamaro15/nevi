use crate::editor::Editor;

fn editor_with_lines(count: usize) -> Editor {
    let mut editor = Editor::default();
    let content: String = (1..=count).map(|i| format!("line{i}\n")).collect();
    editor.replace_buffer_content(&content);
    editor.set_size(120, 40); // text_rows = 38
    editor.update_pane_rects();
    editor
}

fn editor_with_vsplit() -> Editor {
    let mut editor = editor_with_lines(100);
    editor.vsplit(None).expect("vsplit");
    editor.update_pane_rects();
    editor
}

#[test]
fn scroll_pane_viewport_moves_view_not_cursor() {
    let mut editor = editor_with_lines(100);
    editor.cursor.line = 20;
    editor.scroll_to_cursor();
    let before = editor.cursor.line;

    editor.scroll_pane_viewport(editor.active_pane_idx(), 3);

    assert_eq!(editor.viewport_offset, 3);
    assert_eq!(editor.cursor.line, before);
}

#[test]
fn scroll_pane_viewport_drags_cursor_at_view_edge() {
    let mut editor = editor_with_lines(100);
    editor.cursor.line = 0;

    editor.scroll_pane_viewport(editor.active_pane_idx(), 3);

    assert_eq!(editor.viewport_offset, 3);
    assert_eq!(
        editor.cursor.line,
        3 + editor.settings.editor.scroll_off,
        "cursor pulled down to top margin like <C-e> in vim"
    );
}

#[test]
fn scroll_pane_viewport_clamps_at_ends() {
    let mut editor = editor_with_lines(100);

    editor.scroll_pane_viewport(editor.active_pane_idx(), -5);
    assert_eq!(editor.viewport_offset, 0);

    editor.scroll_pane_viewport(editor.active_pane_idx(), 10_000);
    // Vim lets <C-e> run until the last line sits at the top of the window.
    assert_eq!(editor.viewport_offset, 99);
    assert_eq!(editor.cursor.line, 99);
}

#[test]
fn scroll_up_at_file_top_leaves_cursor_inside_margin() {
    // Oracle regression (`5Gzz20<C-y>`): with the view already at the top,
    // vim does not drag a cursor sitting inside the scrolloff margin.
    let mut editor = editor_with_lines(100);
    editor.cursor.line = 4;

    editor.scroll_pane_viewport(editor.active_pane_idx(), -20);

    assert_eq!(editor.viewport_offset, 0);
    assert_eq!(editor.cursor.line, 4);
}

#[test]
fn scroll_pane_viewport_syncs_active_pane_mirror() {
    let mut editor = editor_with_lines(100);
    let active = editor.active_pane_idx();

    editor.scroll_pane_viewport(active, 7);

    assert_eq!(
        editor.panes()[active].viewport_offset,
        editor.viewport_offset
    );
    assert_eq!(editor.panes()[active].cursor, editor.cursor);
}

#[test]
fn scroll_pane_viewport_on_inactive_pane_leaves_mirror_untouched() {
    let mut editor = editor_with_vsplit();
    let inactive = 1 - editor.active_pane_idx();
    let mirror_before = (editor.viewport_offset, editor.cursor);

    editor.scroll_pane_viewport(inactive, 3);

    assert_eq!(editor.panes()[inactive].viewport_offset, 3);
    assert_eq!((editor.viewport_offset, editor.cursor), mirror_before);
}

// The z horizontal scroll family. h_offset never reaches the oracle
// snapshot and the text width differs from Neovim's (gutter), so the
// column math is pinned here; the oracle pins the cursor side effects.
fn nowrap_editor_with_long_line() -> Editor {
    let mut editor = Editor::default();
    editor.set_size(40, 10);
    editor.settings.editor.wrap = false;
    editor.replace_buffer_content(&format!("{}\nshort\n\n", "x".repeat(200)));
    editor.update_pane_rects();
    editor
}

#[test]
fn scroll_columns_moves_the_view_and_drags_the_cursor_into_it() {
    let mut editor = nowrap_editor_with_long_line();
    editor.scroll_columns(5);
    assert_eq!((editor.h_offset, editor.cursor.col), (5, 5));
    // Scrolling back leaves a cursor that is still visible alone.
    editor.scroll_columns(-2);
    assert_eq!((editor.h_offset, editor.cursor.col), (3, 5));
    editor.scroll_columns(-10);
    assert_eq!((editor.h_offset, editor.cursor.col), (0, 5));
    assert_eq!(editor.panes[editor.active_pane_idx()].h_offset, 0);
}

#[test]
fn scroll_columns_left_pulls_a_cursor_past_the_right_edge_back_in() {
    let mut editor = nowrap_editor_with_long_line();
    let width = editor.text_area_width();
    editor.cursor.col = 199;
    editor.scroll_to_cursor();
    assert_eq!(editor.h_offset, 200 - width);

    editor.scroll_columns(-(editor.h_offset as isize));

    assert_eq!(editor.h_offset, 0);
    assert_eq!(editor.cursor.col, width - 1);
}

#[test]
fn scroll_columns_stops_at_the_cursor_lines_last_character() {
    let mut editor = nowrap_editor_with_long_line();
    editor.cursor.line = 1; // "short"
    editor.scroll_columns(9);
    assert_eq!((editor.h_offset, editor.cursor.col), (4, 4));

    editor.cursor.line = 2; // empty line
    editor.cursor.col = 0;
    editor.scroll_columns(3);
    assert_eq!((editor.h_offset, editor.cursor.col), (0, 0));
}

#[test]
fn horizontal_scroll_keys_are_no_ops_with_wrap_on() {
    let mut editor = nowrap_editor_with_long_line();
    editor.settings.editor.wrap = true;
    editor.scroll_columns(5);
    editor.scroll_half_screen_columns(1);
    editor.scroll_cursor_to_screen_start();
    editor.scroll_cursor_to_screen_end();
    assert_eq!((editor.h_offset, editor.cursor.col), (0, 0));
}

#[test]
fn half_screen_column_scroll_uses_half_the_text_width() {
    let mut editor = nowrap_editor_with_long_line();
    let half = editor.text_area_width() / 2;
    editor.scroll_half_screen_columns(2);
    assert_eq!((editor.h_offset, editor.cursor.col), (2 * half, 2 * half));
    editor.scroll_half_screen_columns(-1);
    assert_eq!(editor.h_offset, half);
}

#[test]
fn zs_and_ze_put_the_cursor_at_the_screen_edges() {
    let mut editor = nowrap_editor_with_long_line();
    let width = editor.text_area_width();
    editor.cursor.col = 100;
    editor.scroll_to_cursor();

    editor.scroll_cursor_to_screen_start();
    assert_eq!((editor.h_offset, editor.cursor.col), (100, 100));

    editor.scroll_cursor_to_screen_end();
    assert_eq!((editor.h_offset, editor.cursor.col), (101 - width, 100));

    // Near the start of the line ze cannot scroll before column 0.
    editor.cursor.col = 3;
    editor.scroll_cursor_to_screen_end();
    assert_eq!(editor.h_offset, 0);
}
