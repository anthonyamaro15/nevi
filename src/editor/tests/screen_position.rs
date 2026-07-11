use crate::editor::Editor;
use crate::input::Motion;

fn centered_editor(term_height: u16) -> Editor {
    let mut editor = Editor::default();
    editor.set_size(80, term_height);
    editor.settings.editor.scroll_off = 8;
    let content = (1..=100)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.apply_motion(Motion::GotoLine(50), 1);
    editor.scroll_cursor_center();
    editor
}

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

#[test]
fn page_scroll_down_matches_neovim_in_normal_and_short_viewports() {
    for (term_height, expected_cursor, expected_top) in [(24, 67, 59), (12, 57, 53)] {
        let mut editor = centered_editor(term_height);

        editor.apply_motion(Motion::PageDown, 1);

        assert_eq!(
            (editor.cursor.line, editor.viewport_offset),
            (expected_cursor, expected_top),
            "term_height={term_height}"
        );
        assert_eq!(
            editor.panes[editor.active_pane].cursor, editor.cursor,
            "term_height={term_height}"
        );
        assert_eq!(
            editor.panes[editor.active_pane].viewport_offset, expected_top,
            "term_height={term_height}"
        );
    }
}

#[test]
fn page_scroll_up_matches_neovim_in_normal_and_short_viewports() {
    for (term_height, expected_cursor, expected_top) in [(24, 32, 19), (12, 41, 37)] {
        let mut editor = centered_editor(term_height);

        editor.apply_motion(Motion::PageUp, 1);

        assert_eq!(
            (editor.cursor.line, editor.viewport_offset),
            (expected_cursor, expected_top),
            "term_height={term_height}"
        );
        assert_eq!(
            editor.panes[editor.active_pane].cursor, editor.cursor,
            "term_height={term_height}"
        );
        assert_eq!(
            editor.panes[editor.active_pane].viewport_offset, expected_top,
            "term_height={term_height}"
        );
    }
}

#[test]
fn half_page_scroll_down_matches_neovim_in_normal_and_short_viewports() {
    for (term_height, expected_cursor, expected_top) in [(24, 60, 50), (12, 54, 50)] {
        let mut editor = centered_editor(term_height);

        editor.apply_motion(Motion::HalfPageDown, 1);

        assert_eq!(
            (editor.cursor.line, editor.viewport_offset),
            (expected_cursor, expected_top),
            "term_height={term_height}"
        );
        assert_eq!(
            editor.panes[editor.active_pane].cursor, editor.cursor,
            "term_height={term_height}"
        );
        assert_eq!(
            editor.panes[editor.active_pane].viewport_offset, expected_top,
            "term_height={term_height}"
        );
    }
}

#[test]
fn half_page_scroll_up_matches_neovim_in_normal_and_short_viewports() {
    for (term_height, expected_cursor, expected_top) in [(24, 38, 28), (12, 44, 40)] {
        let mut editor = centered_editor(term_height);

        editor.apply_motion(Motion::HalfPageUp, 1);

        assert_eq!(
            (editor.cursor.line, editor.viewport_offset),
            (expected_cursor, expected_top),
            "term_height={term_height}"
        );
        assert_eq!(
            editor.panes[editor.active_pane].cursor, editor.cursor,
            "term_height={term_height}"
        );
        assert_eq!(
            editor.panes[editor.active_pane].viewport_offset, expected_top,
            "term_height={term_height}"
        );
    }
}

#[test]
fn page_scroll_matches_neovim_at_file_boundaries() {
    let mut near_end = centered_editor(24);
    near_end.apply_motion(Motion::GotoLine(95), 1);
    near_end.scroll_cursor_center();
    near_end.apply_motion(Motion::PageDown, 1);

    assert_eq!((near_end.cursor.line, near_end.viewport_offset), (99, 99));

    let mut near_start = centered_editor(24);
    near_start.apply_motion(Motion::GotoLine(5), 1);
    near_start.scroll_cursor_center();
    near_start.apply_motion(Motion::PageUp, 1);

    assert_eq!((near_start.cursor.line, near_start.viewport_offset), (4, 0));
}

#[test]
fn half_page_scroll_matches_neovim_at_file_boundaries() {
    let mut near_end = centered_editor(24);
    near_end.apply_motion(Motion::GotoLine(95), 1);
    near_end.scroll_cursor_center();
    near_end.apply_motion(Motion::HalfPageDown, 1);

    assert_eq!((near_end.cursor.line, near_end.viewport_offset), (99, 84));

    let mut near_start = centered_editor(24);
    near_start.apply_motion(Motion::GotoLine(5), 1);
    near_start.scroll_cursor_center();
    near_start.apply_motion(Motion::HalfPageUp, 1);

    assert_eq!((near_start.cursor.line, near_start.viewport_offset), (0, 0));
}

#[test]
fn counted_page_scroll_matches_neovim() {
    let mut page_down = centered_editor(24);
    page_down.apply_motion(Motion::PageDown, 2);
    assert_eq!((page_down.cursor.line, page_down.viewport_offset), (87, 79));

    let mut page_up = centered_editor(24);
    page_up.apply_motion(Motion::PageUp, 2);
    assert_eq!((page_up.cursor.line, page_up.viewport_offset), (13, 0));

    let mut half_down = centered_editor(24);
    half_down.apply_motion(Motion::HalfPageDown, 3);
    assert_eq!((half_down.cursor.line, half_down.viewport_offset), (52, 42));

    let mut half_up = centered_editor(24);
    half_up.apply_motion(Motion::HalfPageUp, 3);
    assert_eq!((half_up.cursor.line, half_up.viewport_offset), (46, 36));
}

#[test]
fn full_page_scroll_matches_neovim_for_partial_boundary_pages() {
    let mut at_end = centered_editor(24);
    at_end.apply_motion(Motion::FileEnd, 1);
    at_end.apply_motion(Motion::PageDown, 1);
    assert_eq!((at_end.cursor.line, at_end.viewport_offset), (99, 99));

    let mut near_start = centered_editor(24);
    near_start.apply_motion(Motion::GotoLine(11), 1);
    near_start.scroll_cursor_top();
    near_start.apply_motion(Motion::PageUp, 1);
    assert_eq!(
        (near_start.cursor.line, near_start.viewport_offset),
        (13, 0)
    );

    at_end.apply_motion(Motion::PageUp, 1);
    assert_eq!((at_end.cursor.line, at_end.viewport_offset), (90, 77));

    let mut eof_visible = centered_editor(24);
    eof_visible.apply_motion(Motion::FileEnd, 1);
    eof_visible.apply_motion(Motion::Up, 13);
    eof_visible.apply_motion(Motion::PageDown, 1);
    assert_eq!(
        (eof_visible.cursor.line, eof_visible.viewport_offset),
        (99, 99)
    );
}

#[test]
fn full_page_scroll_matches_neovim_in_two_row_viewport() {
    let mut page_down = centered_editor(4);
    page_down.apply_motion(Motion::PageDown, 1);
    assert_eq!((page_down.cursor.line, page_down.viewport_offset), (51, 51));

    let mut page_up = centered_editor(4);
    page_up.apply_motion(Motion::PageUp, 1);
    assert_eq!((page_up.cursor.line, page_up.viewport_offset), (47, 47));
}

#[test]
fn full_page_scroll_matches_neovim_in_three_and_four_row_viewports() {
    for term_height in [5, 6] {
        let mut page_down = centered_editor(term_height);
        page_down.apply_motion(Motion::PageDown, 1);
        assert_eq!(
            (page_down.cursor.line, page_down.viewport_offset),
            (52, 51),
            "term_height={term_height}"
        );

        let mut page_up = centered_editor(term_height);
        page_up.apply_motion(Motion::PageUp, 1);
        assert_eq!(
            (page_up.cursor.line, page_up.viewport_offset),
            (46, 45),
            "term_height={term_height}"
        );
    }
}

#[test]
fn page_up_at_visible_file_start_clamps_cursor_to_scrolloff_area() {
    let mut editor = centered_editor(12);
    editor.apply_motion(Motion::GotoLine(6), 1);
    editor.scroll_cursor_bottom();
    assert_eq!((editor.cursor.line, editor.viewport_offset), (5, 0));

    editor.apply_motion(Motion::PageUp, 1);

    assert_eq!((editor.cursor.line, editor.viewport_offset), (4, 0));
}

#[test]
fn page_scroll_repairs_horizontal_offset_after_landing_on_short_line() {
    let mut editor = centered_editor(24);
    let content = (1..=100)
        .map(|line| {
            if line == 50 {
                format!("{}\n", "x".repeat(200))
            } else {
                format!("line {line:03}\n")
            }
        })
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.apply_motion(Motion::GotoLine(50), 1);
    editor.cursor.col = 150;
    editor.scroll_cursor_center();
    editor.scroll_to_cursor();
    assert!(editor.h_offset > 0);

    editor.apply_motion(Motion::PageDown, 1);

    assert_eq!(editor.cursor.col, 7);
    assert_eq!(editor.h_offset, 0);
    assert_eq!(editor.panes[editor.active_pane].h_offset, 0);
}

#[test]
fn page_scroll_keeps_clamped_cursor_visible_on_medium_destination_line() {
    let mut editor = centered_editor(24);
    editor.set_size(40, 24);
    let content = (1..=100)
        .map(|line| {
            if line == 50 {
                format!("{}\n", "x".repeat(200))
            } else {
                format!("{}\n", "y".repeat(56))
            }
        })
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.apply_motion(Motion::GotoLine(50), 1);
    editor.cursor.col = 150;
    editor.scroll_cursor_center();
    editor.scroll_to_cursor();
    assert!(editor.h_offset > 0);

    editor.apply_motion(Motion::PageDown, 1);

    let visible_end = editor.h_offset + editor.text_area_width();
    assert_eq!(editor.cursor.col, 55);
    assert!(editor.h_offset <= editor.cursor.col);
    assert!(editor.cursor.col < visible_end);
}

#[test]
fn half_page_scroll_remembers_explicit_distance_per_pane() {
    let mut editor = centered_editor(24);

    editor.apply_page_motion(Motion::HalfPageDown, Some(2));
    assert_eq!((editor.cursor.line, editor.viewport_offset), (51, 41));
    assert_eq!(
        editor.panes[editor.active_pane].half_page_scroll_rows,
        Some(2)
    );

    editor.apply_page_motion(Motion::HalfPageDown, None);
    assert_eq!((editor.cursor.line, editor.viewport_offset), (53, 43));

    editor.apply_page_motion(Motion::HalfPageUp, None);
    assert_eq!((editor.cursor.line, editor.viewport_offset), (51, 41));

    editor.hsplit(None).expect("horizontal split");
    assert!(
        editor
            .panes
            .iter()
            .all(|pane| pane.half_page_scroll_rows.is_none())
    );
}

#[test]
fn explicit_one_half_page_scroll_moves_one_line() {
    let mut editor = centered_editor(24);

    editor.apply_page_motion(Motion::HalfPageDown, Some(1));

    assert_eq!((editor.cursor.line, editor.viewport_offset), (50, 40));
    assert_eq!(
        editor.panes[editor.active_pane].half_page_scroll_rows,
        Some(1)
    );
}

#[test]
fn wrapped_page_motion_keeps_existing_motion_path() {
    let mut expected = centered_editor(24);
    expected.settings.editor.wrap = true;
    expected.settings.editor.wrap_width = 20;
    let content = (1..=100)
        .map(|line| format!("line {line:03} {}\n", "x".repeat(60)))
        .collect::<String>();
    expected.replace_buffer_content(&content);
    expected.apply_motion(Motion::GotoLine(50), 1);
    expected.scroll_cursor_center();

    let mut actual = centered_editor(24);
    actual.settings.editor.wrap = true;
    actual.settings.editor.wrap_width = 20;
    actual.replace_buffer_content(&content);
    actual.apply_motion(Motion::GotoLine(50), 1);
    actual.scroll_cursor_center();
    expected.apply_motion(Motion::HalfPageDown, 1);
    actual.apply_page_motion(Motion::HalfPageDown, None);

    assert_eq!(actual.cursor, expected.cursor);
    assert_eq!(actual.viewport_offset, expected.viewport_offset);
    assert_eq!(actual.h_offset, expected.h_offset);
}

#[test]
fn wrap_enabled_file_with_unbroken_lines_uses_page_parity_at_eof() {
    let mut editor = centered_editor(24);
    editor.settings.editor.wrap = true;
    editor.settings.editor.wrap_width = 9_999;

    editor.apply_motion(Motion::FileEnd, 1);
    editor.apply_page_motion(Motion::PageDown, None);

    assert_eq!((editor.cursor.line, editor.viewport_offset), (99, 99));
    assert_eq!(editor.panes[editor.active_pane].cursor, editor.cursor);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 99);
}

#[test]
fn page_scroll_uses_active_horizontal_split_height() {
    let mut editor = centered_editor(24);
    editor.hsplit(None).expect("horizontal split");
    editor.cursor.line = 49;
    editor.scroll_cursor_center();
    assert_eq!(editor.panes[editor.active_pane].rect.height, 11);

    editor.apply_page_motion(Motion::PageDown, None);

    assert_eq!((editor.cursor.line, editor.viewport_offset), (58, 53));
    assert_eq!(editor.panes[editor.active_pane].cursor, editor.cursor);
    assert_eq!(editor.panes[editor.active_pane].viewport_offset, 53);
}

#[test]
fn resizing_viewport_resets_remembered_half_page_distance() {
    let mut editor = centered_editor(24);
    editor.apply_page_motion(Motion::HalfPageDown, Some(2));
    assert_eq!(
        editor.panes[editor.active_pane].half_page_scroll_rows,
        Some(2)
    );

    editor.set_size(80, 12);
    assert_eq!(editor.panes[editor.active_pane].half_page_scroll_rows, None);

    let before = (editor.cursor.line, editor.viewport_offset);
    editor.apply_page_motion(Motion::HalfPageDown, None);
    assert_eq!(
        (editor.cursor.line, editor.viewport_offset),
        (before.0 + 5, before.1 + 5)
    );
}
