use super::handle_mouse_event;
use crate::config::SignColumn;
use crate::editor::{Editor, Mode};
use crate::terminal::handle_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

fn editor(text: &str) -> Editor {
    let mut editor = Editor::default();
    editor.replace_buffer_content(text);
    editor.settings.editor.line_numbers = false;
    editor.settings.editor.sign_column = SignColumn::No;
    editor.settings.editor.scroll_off = 0;
    editor.settings.editor.wrap = false;
    editor.settings.editor.tab_width = 4;
    editor.set_size(80, 24);
    editor.update_pane_rects();
    editor
}

fn mouse(editor: &mut Editor, kind: MouseEventKind, col: u16, row: u16) -> bool {
    handle_mouse_event(
        editor,
        MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        },
    )
}

fn press(editor: &mut Editor, col: u16, row: u16) {
    mouse(editor, MouseEventKind::Down(MouseButton::Left), col, row);
}

fn drag(editor: &mut Editor, col: u16, row: u16) {
    mouse(editor, MouseEventKind::Drag(MouseButton::Left), col, row);
}

fn release(editor: &mut Editor, col: u16, row: u16) {
    mouse(editor, MouseEventKind::Up(MouseButton::Left), col, row);
}

fn key(editor: &mut Editor, code: KeyCode) {
    handle_key(editor, KeyEvent::new(code, KeyModifiers::NONE));
}

fn selected_text(editor: &Editor) -> String {
    assert_eq!(editor.mode, Mode::Visual);
    let (sl, sc, el, ec) = editor.get_visual_range();
    editor.get_range_text(sl, sc, el, ec)
}

#[test]
fn drag_preserves_anchor_and_release_updates_the_endpoint() {
    let mut editor = editor("alpha beta gamma\nsecond line\n");
    press(&mut editor, 1, 0);
    assert_eq!(editor.mode, Mode::Normal);
    editor.render_damage.clear_after_full_render();
    assert!(mouse(
        &mut editor,
        MouseEventKind::Drag(MouseButton::Left),
        8,
        0
    ));
    assert_eq!(selected_text(&editor), "lpha bet");
    assert!(editor.render_damage.requires_full_render());
    release(&mut editor, 4, 0);
    assert_eq!(selected_text(&editor), "lpha");
    assert_eq!(editor.get_visual_range(), (0, 1, 0, 4));
    assert_eq!(editor.panes()[0].cursor, editor.cursor);
    drag(&mut editor, 12, 0);
    assert_eq!(selected_text(&editor), "lpha");
}

#[test]
fn reverse_multiline_drag_keeps_the_original_anchor() {
    let mut editor = editor("alpha beta gamma\nsecond line\n");
    press(&mut editor, 3, 1);
    drag(&mut editor, 1, 0);
    release(&mut editor, 1, 0);
    assert_eq!(selected_text(&editor), "lpha beta gamma\nseco");
    assert_eq!(
        (editor.visual.anchor_line, editor.visual.anchor_col),
        (1, 3)
    );
}

#[test]
fn stationary_click_does_not_enter_visual_mode() {
    let mut editor = editor("alpha\n");
    press(&mut editor, 1, 0);
    drag(&mut editor, 1, 0);
    release(&mut editor, 1, 0);
    assert_eq!(editor.mode, Mode::Normal);
    assert_eq!(editor.cursor.col, 1);
}

#[test]
fn release_can_select_without_intermediate_drag_events() {
    let mut editor = editor("alpha\n");
    press(&mut editor, 1, 0);
    release(&mut editor, 4, 0);
    assert_eq!(selected_text(&editor), "lpha");
}

#[test]
fn drag_selection_supports_yank_delete_change_and_undo() {
    for (op, text, mode) in [
        ('y', "alpha beta\n", Mode::Normal),
        ('d', "a beta\n", Mode::Normal),
        ('c', "a beta\n", Mode::Insert),
    ] {
        let mut editor = editor("alpha beta\n");
        press(&mut editor, 1, 0);
        drag(&mut editor, 4, 0);
        release(&mut editor, 4, 0);
        assert_eq!(selected_text(&editor), "lpha");
        key(&mut editor, KeyCode::Char(op));
        assert_eq!(editor.buffer().content(), text, "operator {op}");
        assert_eq!(editor.mode, mode, "operator {op}");
        if op == 'y' {
            key(&mut editor, KeyCode::Char('p'));
            assert_eq!(editor.buffer().content(), "allphapha beta\n");
        } else {
            if editor.mode == Mode::Insert {
                key(&mut editor, KeyCode::Esc);
            }
            key(&mut editor, KeyCode::Char('u'));
            assert_eq!(editor.buffer().content(), "alpha beta\n");
        }
    }
}

#[test]
fn drag_maps_display_cells_for_wide_characters_and_tabs() {
    for (text, start, end, expected) in [("日本語abc\n", 2, 6, "本語a"), ("\tabc\n", 1, 5, "\tab")]
    {
        let mut editor = editor(text);
        press(&mut editor, start, 0);
        drag(&mut editor, end, 0);
        release(&mut editor, end, 0);
        assert_eq!(selected_text(&editor), expected);
    }
}

#[test]
fn drag_maps_wrapped_rows() {
    let mut editor = editor("abcdefghij\nnext\n");
    editor.settings.editor.wrap = true;
    editor.settings.editor.wrap_width = 4;
    press(&mut editor, 1, 0);
    drag(&mut editor, 2, 1);
    release(&mut editor, 2, 1);
    assert_eq!(selected_text(&editor), "bcdefg");
    assert_eq!(editor.get_visual_range(), (0, 1, 0, 6));
}

#[test]
fn drag_maps_scrolled_text_and_gutter() {
    let mut editor = editor("first\n0123456789\nlast\n");
    editor.settings.editor.line_numbers = true;
    editor.settings.editor.sign_column = SignColumn::Yes;
    editor.viewport_offset = 1;
    editor.h_offset = 3;
    editor.sync_active_pane_view();
    press(&mut editor, 8, 0); // six gutter cells + two text cells
    drag(&mut editor, 10, 0);
    release(&mut editor, 10, 0);
    assert_eq!(selected_text(&editor), "567");
    assert_eq!(editor.get_visual_range(), (1, 5, 1, 7));
}

#[test]
fn drag_clamps_past_line_end_and_below_eof() {
    let mut editor = editor("alpha\n\nbeta\n");
    press(&mut editor, 1, 0);
    drag(&mut editor, 70, 20);
    release(&mut editor, 70, 20);
    assert_eq!(selected_text(&editor), "lpha\n\nbeta");
    assert_eq!(editor.get_visual_range(), (0, 1, 2, 3));
}

#[test]
fn drag_stays_in_the_pane_where_the_press_started() {
    let mut editor = editor("alpha beta gamma\nsecond line\n");
    editor.vsplit(None).unwrap();
    let right = editor.panes()[1].rect.x;
    press(&mut editor, right + 3, 0);
    drag(&mut editor, 0, 0);
    release(&mut editor, 0, 0);
    assert_eq!(editor.active_pane_index(), 1);
    assert_eq!(selected_text(&editor), "alph");
    assert_eq!(editor.panes()[1].cursor, editor.cursor);
}

#[test]
fn release_outside_the_pane_ends_drag_tracking() {
    let mut editor = editor("alpha\nbeta\n");
    press(&mut editor, 1, 0);
    drag(&mut editor, 2, 1);
    release(&mut editor, 2, 23);
    assert_eq!(editor.mode, Mode::Visual);
    let selection = editor.get_visual_range();
    drag(&mut editor, 4, 0);
    assert_eq!(editor.get_visual_range(), selection);
}

#[test]
fn drag_requires_a_press_in_the_editor() {
    let mut editor = editor("alpha\n");
    drag(&mut editor, 3, 0);
    release(&mut editor, 3, 0);
    assert_eq!(editor.mode, Mode::Normal);
    assert_eq!(editor.cursor.col, 0);
    press(&mut editor, 0, 23);
    drag(&mut editor, 4, 0);
    assert_eq!(editor.mode, Mode::Normal);
}

#[test]
fn escape_cancels_a_drag_before_mouse_release() {
    let mut editor = editor("alpha\n");
    press(&mut editor, 1, 0);
    drag(&mut editor, 3, 0);
    key(&mut editor, KeyCode::Esc);
    release(&mut editor, 4, 0);
    assert_eq!(editor.mode, Mode::Normal);
    assert_eq!(editor.cursor.col, 3);
}

#[test]
fn interrupted_drags_do_not_resume_in_another_context() {
    for interruption in ["pane", "resize", "buffer", "overlay", "disabled"] {
        let mut editor = editor("alpha\n");
        editor.vsplit(None).unwrap();
        editor.focus_pane(0);
        press(&mut editor, 1, 0);
        match interruption {
            "pane" => editor.focus_pane(1),
            "resize" => editor.set_size(60, 20),
            "buffer" => editor.replace_buffer_content("different\n"),
            "overlay" => {
                editor.mode = Mode::Finder;
                drag(&mut editor, 4, 0);
                editor.mode = Mode::Normal;
            }
            "disabled" => editor.settings.editor.mouse = false,
            _ => unreachable!(),
        }
        drag(&mut editor, 4, 0);
        release(&mut editor, 4, 0);
        assert!(!editor.mode.is_visual(), "{interruption}");
    }
}

#[test]
fn new_click_replaces_the_previous_selection() {
    let mut editor = editor("alpha beta\n");
    press(&mut editor, 1, 0);
    drag(&mut editor, 4, 0);
    release(&mut editor, 4, 0);
    press(&mut editor, 6, 0);
    assert_eq!(editor.mode, Mode::Normal);
    release(&mut editor, 8, 0);
    assert_eq!(selected_text(&editor), "bet");
}

#[test]
fn insert_drag_returns_to_insert_after_an_operator_or_escape() {
    for op in [
        KeyCode::Char('y'),
        KeyCode::Char('d'),
        KeyCode::Char('c'),
        KeyCode::Esc,
    ] {
        let mut editor = editor("alpha beta\n");
        key(&mut editor, KeyCode::Char('i'));
        press(&mut editor, 1, 0);
        assert_eq!(editor.mode, Mode::Insert);
        drag(&mut editor, 4, 0);
        release(&mut editor, 4, 0);
        assert_eq!(selected_text(&editor), "lpha");
        key(&mut editor, op);
        assert_eq!(editor.mode, Mode::Insert, "{op:?}");
        key(&mut editor, KeyCode::Char('X'));
        assert_eq!(
            editor.buffer().content(),
            match op {
                KeyCode::Char('y') => "aXlpha beta\n",
                KeyCode::Esc => "alphXa beta\n",
                _ => "aX beta\n",
            },
            "{op:?}"
        );
    }
}

#[test]
fn drag_clears_pending_keyboard_sequences() {
    let mut editor = editor("alpha beta\n");
    key(&mut editor, KeyCode::Char('g'));
    press(&mut editor, 1, 0);
    drag(&mut editor, 4, 0);
    release(&mut editor, 4, 0);
    key(&mut editor, KeyCode::Char('d'));
    assert_eq!(editor.buffer().content(), "a beta\n");
    assert!(!editor.input_state.has_pending_sequence());
}

#[test]
fn dragging_past_eol_selects_the_line_break() {
    let mut editor = editor("alpha\nbeta\n");
    press(&mut editor, 1, 0);
    drag(&mut editor, 70, 0);
    release(&mut editor, 70, 0);
    assert_eq!(selected_text(&editor), "lpha\n");
    assert_eq!(editor.cursor.col, 5);
    key(&mut editor, KeyCode::Char('d'));
    assert_eq!(editor.buffer().content(), "abeta\n");
    key(&mut editor, KeyCode::Char('u'));
    assert_eq!(editor.buffer().content(), "alpha\nbeta\n");
}

#[test]
fn dragging_to_eof_preserves_the_final_file_newline() {
    let mut editor = editor("alpha\nbeta\n");
    press(&mut editor, 1, 0);
    drag(&mut editor, 70, 1);
    release(&mut editor, 70, 1);
    assert_eq!(editor.cursor.col, 4);
    key(&mut editor, KeyCode::Char('d'));
    assert_eq!(editor.buffer().content(), "a\n");
    key(&mut editor, KeyCode::Char('u'));
    assert_eq!(editor.buffer().content(), "alpha\nbeta\n");
}

#[test]
fn insert_mouse_selection_keeps_separate_undo_groups() {
    let mut editor = editor("alpha beta\n");
    key(&mut editor, KeyCode::Char('i'));
    key(&mut editor, KeyCode::Char('X'));
    press(&mut editor, 2, 0);
    drag(&mut editor, 5, 0);
    release(&mut editor, 5, 0);
    key(&mut editor, KeyCode::Char('d'));
    key(&mut editor, KeyCode::Char('Z'));
    key(&mut editor, KeyCode::Esc);
    assert_eq!(editor.buffer().content(), "XaZ beta\n");
    key(&mut editor, KeyCode::Char('u'));
    assert_eq!(editor.buffer().content(), "Xa beta\n");
    key(&mut editor, KeyCode::Char('u'));
    assert_eq!(editor.buffer().content(), "Xalpha beta\n");
    key(&mut editor, KeyCode::Char('u'));
    assert_eq!(editor.buffer().content(), "alpha beta\n");
}

mod oracle;

#[test]
fn stationary_click_past_eol_stays_a_plain_click() {
    let mut editor = editor("alpha\nbeta\n");
    press(&mut editor, 70, 0);
    drag(&mut editor, 70, 0);
    release(&mut editor, 70, 0);
    assert_eq!(editor.mode, Mode::Normal);
    assert_eq!(editor.cursor.col, 4);
}

#[test]
fn mouse_press_returns_focus_from_explorer_to_text() {
    let mut editor = editor("alpha beta\n");
    editor.explorer.show();
    editor.update_pane_rects();
    editor.mode = Mode::Explorer;
    let x = editor.panes()[0].rect.x;
    press(&mut editor, x + 1, 0);
    drag(&mut editor, x + 4, 0);
    release(&mut editor, x + 4, 0);
    assert_eq!(selected_text(&editor), "lpha");
}

#[test]
fn mouse_selection_finishes_insert_ctrl_o_before_resuming_typing() {
    let mut editor = editor("alpha beta\n");
    key(&mut editor, KeyCode::Char('i'));
    handle_key(
        &mut editor,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
    );
    press(&mut editor, 1, 0);
    assert_eq!(editor.mode, Mode::Insert);
    assert!(!editor.pending_insert_normal_once);
    drag(&mut editor, 4, 0);
    release(&mut editor, 4, 0);
    key(&mut editor, KeyCode::Char('d'));
    assert_eq!(editor.mode, Mode::Insert);
    assert!(!editor.pending_insert_normal_once);
    key(&mut editor, KeyCode::Char('X'));
    assert_eq!(editor.buffer().content(), "aX beta\n");
}

#[test]
fn mouse_selection_from_replace_resumes_replace_after_delete() {
    let mut editor = editor("alpha beta\n");
    key(&mut editor, KeyCode::Char('R'));
    press(&mut editor, 1, 0);
    drag(&mut editor, 4, 0);
    release(&mut editor, 4, 0);
    assert_eq!(selected_text(&editor), "lpha");
    key(&mut editor, KeyCode::Char('d'));
    assert_eq!(editor.mode, Mode::Replace);
    key(&mut editor, KeyCode::Char('X'));
    assert_eq!(editor.buffer().content(), "aXbeta\n");
}

#[test]
fn mouse_selection_does_not_replay_a_counted_insert_at_the_click() {
    let mut editor = editor("alpha beta gamma\nsecond line\n");
    for ch in ['3', 'i', 'X'] {
        key(&mut editor, KeyCode::Char(ch));
    }
    press(&mut editor, 2, 0);
    drag(&mut editor, 5, 0);
    release(&mut editor, 5, 0);
    assert_eq!(
        editor.buffer().content(),
        "Xalpha beta gamma\nsecond line\n"
    );
    assert_eq!(selected_text(&editor), "lpha");
    key(&mut editor, KeyCode::Char('d'));
    assert_eq!(editor.buffer().content(), "Xa beta gamma\nsecond line\n");
    assert_eq!(editor.mode, Mode::Insert);
}

#[test]
fn mouse_selection_does_not_replay_pending_block_insert() {
    let mut editor = editor("alpha beta gamma\nsecond line\n");
    handle_key(
        &mut editor,
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
    );
    key(&mut editor, KeyCode::Char('j'));
    handle_key(
        &mut editor,
        KeyEvent::new(KeyCode::Char('I'), KeyModifiers::SHIFT),
    );
    key(&mut editor, KeyCode::Char('X'));
    press(&mut editor, 2, 0);
    drag(&mut editor, 5, 1);
    release(&mut editor, 5, 1);
    assert_eq!(
        editor.buffer().content(),
        "Xalpha beta gamma\nsecond line\n"
    );
    key(&mut editor, KeyCode::Char('d'));
    assert_eq!(editor.buffer().content(), "Xa line\n");
}

#[test]
fn escape_from_mouse_eol_selection_restores_a_normal_cursor() {
    let mut editor = editor("alpha\nbeta\n");
    press(&mut editor, 1, 0);
    drag(&mut editor, 70, 0);
    release(&mut editor, 70, 0);
    key(&mut editor, KeyCode::Esc);
    assert_eq!(editor.mode, Mode::Normal);
    assert_eq!(editor.cursor.col, 4);
    key(&mut editor, KeyCode::Char('x'));
    assert_eq!(editor.buffer().content(), "alph\nbeta\n");
}

#[test]
fn clicking_another_pane_ends_insert_selection_and_resumes_typing() {
    let mut editor = editor("alpha beta\n");
    editor.vsplit(None).unwrap();
    editor.focus_pane(0);
    key(&mut editor, KeyCode::Char('i'));
    press(&mut editor, 1, 0);
    drag(&mut editor, 4, 0);
    release(&mut editor, 4, 0);
    let x = editor.panes()[1].rect.x;
    press(&mut editor, x + 1, 0);
    release(&mut editor, x + 1, 0);
    assert_eq!(editor.mode, Mode::Insert);
    key(&mut editor, KeyCode::Char('X'));
    assert_eq!(editor.buffer().content(), "aXlpha beta\n");
}
