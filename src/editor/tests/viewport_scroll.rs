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
