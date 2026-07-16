use crate::editor::Editor;
use crate::terminal::handle_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn char_key(ch: char) -> KeyEvent {
    let modifiers = if ch.is_ascii_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    };
    KeyEvent::new(KeyCode::Char(ch), modifiers)
}

fn type_chars(editor: &mut Editor, chars: &str) {
    for ch in chars.chars() {
        handle_key(editor, char_key(ch));
    }
}

fn key(editor: &mut Editor, code: KeyCode) {
    handle_key(editor, KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn counted_replace_past_line_end_leaves_the_line_unchanged() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc\n");

    type_chars(&mut editor, "4rx");

    assert_eq!(editor.buffer().content(), "abc\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));
}

#[test]
fn counted_replace_redo_restores_the_original_command_coordinate() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abcdef\n");

    type_chars(&mut editor, "l3rx");
    editor.undo();
    editor.redo();

    assert_eq!(editor.buffer().content(), "axxxef\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 1));
}

#[test]
fn replace_with_enter_splits_the_line_at_the_cursor() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc\n");

    type_chars(&mut editor, "lr");
    key(&mut editor, KeyCode::Enter);

    assert_eq!(editor.buffer().content(), "a\nc\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (1, 0));
}

#[test]
fn counted_replace_with_enter_removes_the_count_and_inserts_one_newline() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abcd\n");

    type_chars(&mut editor, "2r");
    key(&mut editor, KeyCode::Enter);

    assert_eq!(editor.buffer().content(), "\ncd\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (1, 0));
}

#[test]
fn replace_mode_backspace_restores_the_overwritten_character() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc\n");

    type_chars(&mut editor, "RXY");
    key(&mut editor, KeyCode::Backspace);
    key(&mut editor, KeyCode::Esc);

    assert_eq!(editor.buffer().content(), "Xbc\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));
}

#[test]
fn replace_mode_movement_invalidates_backspace_restore_history() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abcdef\n");

    type_chars(&mut editor, "RXY");
    key(&mut editor, KeyCode::Left);
    key(&mut editor, KeyCode::Backspace);
    key(&mut editor, KeyCode::Esc);

    assert_eq!(editor.buffer().content(), "XYcdef\n");
}

#[test]
fn replace_mode_vertical_movement_keeps_backspace_navigation_without_restoring_text() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abcdef\nuvwxyz\n");

    type_chars(&mut editor, "RXY");
    key(&mut editor, KeyCode::Down);
    key(&mut editor, KeyCode::Backspace);
    key(&mut editor, KeyCode::Esc);

    assert_eq!(editor.buffer().content(), "XYcdef\nuvwxyz\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (1, 0));
}

#[test]
fn replace_mode_backspace_crosses_to_the_previous_line_end_after_movement() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc\nuvwxyz\n");

    type_chars(&mut editor, "R");
    key(&mut editor, KeyCode::Down);
    key(&mut editor, KeyCode::Backspace);
    key(&mut editor, KeyCode::Esc);

    assert_eq!(editor.buffer().content(), "abc\nuvwxyz\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 2));
}

#[test]
fn replace_mode_continues_replacing_after_backspace_crosses_a_line_boundary() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc\nuvwxyz\n");

    type_chars(&mut editor, "jR");
    key(&mut editor, KeyCode::Backspace);
    type_chars(&mut editor, "X");
    key(&mut editor, KeyCode::Esc);

    assert_eq!(editor.buffer().content(), "abcX\nuvwxyz\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 3));
}

#[test]
fn replace_mode_line_boundary_backspace_cancels_counted_replay() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc\nuvwxyz\n");

    type_chars(&mut editor, "j2R");
    key(&mut editor, KeyCode::Backspace);
    type_chars(&mut editor, "X");
    key(&mut editor, KeyCode::Esc);

    assert_eq!(editor.buffer().content(), "abcX\nuvwxyz\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 3));
}

#[test]
fn replace_mode_enter_inserts_a_newline_and_keeps_replacing() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc\ndef\n");

    type_chars(&mut editor, "lRXY");
    key(&mut editor, KeyCode::Enter);
    type_chars(&mut editor, "Z");
    key(&mut editor, KeyCode::Esc);

    assert_eq!(editor.buffer().content(), "aXY\nZ\ndef\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (1, 0));
}

#[test]
fn counted_replace_mode_repeats_the_entered_text() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abcdefghij\n");

    type_chars(&mut editor, "2RXY");
    key(&mut editor, KeyCode::Esc);

    assert_eq!(editor.buffer().content(), "XYXYefghij\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 3));
}

#[test]
fn replace_mode_movement_cancels_counted_replay() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abcdefghi\n");

    type_chars(&mut editor, "2R");
    key(&mut editor, KeyCode::Right);
    type_chars(&mut editor, "X");
    key(&mut editor, KeyCode::Esc);

    assert_eq!(editor.buffer().content(), "aXcdefghi\n");
}

#[test]
fn replace_mode_redo_restores_the_original_command_coordinate() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abcdef\n");

    type_chars(&mut editor, "llRXY");
    key(&mut editor, KeyCode::Esc);
    editor.undo();
    editor.redo();

    assert_eq!(editor.buffer().content(), "abXYef\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 2));
}
