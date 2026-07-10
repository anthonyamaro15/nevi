use crate::editor::Editor;
use crate::input::Motion;

#[test]
fn right_after_zt_near_eof_preserves_neovim_top_line() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    let content = (1..=30)
        .map(|line| format!("line {line:02}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);

    editor.apply_motion(Motion::GotoLine(29), 1);
    editor.scroll_cursor_top();
    editor.apply_motion(Motion::Right, 1);

    assert_eq!((editor.cursor.line, editor.cursor.col), (28, 1));
    assert_eq!(editor.viewport_offset, 20);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 20);
}

#[test]
fn scroll_cursor_top_respects_scrolloff() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    let content = (1..=100)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.cursor.line = 49;

    editor.scroll_cursor_top();

    assert_eq!(editor.viewport_offset, 41);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 41);
}

#[test]
fn scroll_cursor_bottom_respects_scrolloff() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    let content = (1..=100)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.cursor.line = 49;

    editor.scroll_cursor_bottom();

    assert_eq!(editor.viewport_offset, 36);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 36);
}

#[test]
fn scroll_cursor_top_caps_scrolloff_for_even_height() {
    let mut editor = Editor::default();
    editor.set_size(80, 12);
    editor.settings.editor.scroll_off = 8;
    let content = (1..=100)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.cursor.line = 49;

    editor.scroll_cursor_top();

    assert_eq!(editor.text_rows(), 10);
    assert_eq!(editor.viewport_offset, 45);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 45);
}

#[test]
fn scroll_cursor_bottom_caps_scrolloff_for_even_height() {
    let mut editor = Editor::default();
    editor.set_size(80, 12);
    editor.settings.editor.scroll_off = 8;
    let content = (1..=100)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.cursor.line = 49;

    editor.scroll_cursor_bottom();

    assert_eq!(editor.text_rows(), 10);
    assert_eq!(editor.viewport_offset, 44);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 44);
}

#[test]
fn screen_bottom_motion_ignores_trailing_newline_line_near_eof() {
    let mut editor = Editor::default();
    let content = (1..=30)
        .map(|line| format!("line {line:02}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);

    editor.apply_motion(Motion::FileEnd, 1);
    editor.apply_motion(Motion::ScreenBottom, 1);

    assert_eq!(editor.cursor.line, 29);
    assert_eq!(editor.cursor.col, 0);
}

#[test]
fn wrapped_file_end_packs_viewport_against_eof() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    editor.settings.editor.wrap = true;
    editor.settings.editor.wrap_width = 80;
    let content = (1..=100)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);

    editor.apply_motion(Motion::FileEnd, 1);

    assert_eq!(editor.cursor.line, 99);
    assert_eq!(
        editor.viewport_offset, 78,
        "G should fill the viewport from EOF instead of pinning the final line to the top"
    );
    assert_eq!(
        editor.panes[editor.active_pane].viewport_offset, 78,
        "rendered pane viewport should stay in sync with editor viewport"
    );
    assert_eq!(editor.panes[editor.active_pane].h_offset, 0);
}

#[test]
fn wrapped_file_end_on_tall_final_line_keeps_cursor_segment_visible() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    editor.settings.editor.wrap = true;
    editor.settings.editor.wrap_width = 10;
    editor.replace_buffer_content(&format!("context\nCURSOR_PREFIX{}\n", "x".repeat(5_000)));

    editor.apply_motion(Motion::FileEnd, 1);

    assert_eq!((editor.cursor.line, editor.cursor.col), (1, 0));
    assert_eq!(editor.viewport_offset, 1);
    assert_eq!(editor.h_offset, 0);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 1);
    assert_eq!(editor.panes[editor.active_pane].h_offset, 0);
    assert_eq!(editor.panes[editor.active_pane].cursor, editor.cursor);
}

#[test]
fn file_end_packs_horizontal_split_to_active_pane_height() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    let content = (1..=100)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.hsplit(None).expect("horizontal split");

    editor.apply_motion(Motion::FileEnd, 1);

    assert_eq!(editor.panes[editor.active_pane].rect.height, 11);
    assert_eq!((editor.cursor.line, editor.cursor.col), (99, 0));
    assert_eq!(editor.viewport_offset, 89);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 89);
    assert_eq!(editor.panes[editor.active_pane].cursor, editor.cursor);
}

#[test]
fn wrapped_file_end_packs_horizontal_split_and_syncs_active_pane() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    editor.settings.editor.wrap = true;
    editor.settings.editor.wrap_width = 80;
    let content = (1..=100)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.hsplit(None).expect("horizontal split");

    editor.apply_motion(Motion::FileEnd, 1);

    assert_eq!(editor.panes[editor.active_pane].rect.height, 11);
    assert_eq!((editor.cursor.line, editor.cursor.col), (99, 0));
    assert_eq!(editor.viewport_offset, 89);
    assert_eq!(editor.h_offset, 0);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 89);
    assert_eq!(editor.panes[editor.active_pane].h_offset, 0);
    assert_eq!(editor.panes[editor.active_pane].cursor, editor.cursor);
}

#[test]
fn screen_position_motions_use_horizontal_split_height() {
    for motion in [
        Motion::ScreenTop,
        Motion::ScreenMiddle,
        Motion::ScreenBottom,
    ] {
        let mut editor = Editor::default();
        editor.set_size(80, 24);
        editor.settings.editor.scroll_off = 8;
        let content = (1..=100)
            .map(|line| format!("line {line:03}\n"))
            .collect::<String>();
        editor.replace_buffer_content(&content);
        editor.hsplit(None).expect("horizontal split");
        editor.cursor.line = 49;
        editor.scroll_cursor_center();

        editor.apply_motion(motion, 1);

        assert_eq!(editor.panes[editor.active_pane].rect.height, 11);
        assert_eq!(editor.cursor.line, 49, "motion={motion:?}");
        assert_eq!(editor.viewport_offset, 44, "motion={motion:?}");
        assert_eq!(
            editor.panes[editor.active_pane].viewport_offset, 44,
            "motion={motion:?}"
        );
        assert_eq!(editor.panes[editor.active_pane].cursor, editor.cursor);
    }
}

#[test]
fn counted_g_beyond_eof_packs_last_real_line() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    let content = (1..=30)
        .map(|line| format!("line {line:02}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);

    editor.apply_motion(Motion::GotoLine(999), 1);

    assert_eq!((editor.cursor.line, editor.cursor.col), (29, 0));
    assert_eq!(editor.viewport_offset, 8);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 8);
    assert_eq!(editor.panes[editor.active_pane].cursor, editor.cursor);
}

#[test]
fn capped_wrapped_segment_rows_stops_at_requested_viewport_cap() {
    let mut editor = Editor::default();
    editor.replace_buffer_content(&"x".repeat(5_000));
    let line = editor.buffer().line(0).expect("long line");

    assert_eq!(
        Editor::capped_wrapped_segment_rows(line, 10, 4, 3),
        (3, true)
    );
}
