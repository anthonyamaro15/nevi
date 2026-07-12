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

fn escape(editor: &mut Editor) {
    handle_key(editor, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
}

#[test]
fn uppercase_i_keeps_existing_whitespace_on_blank_line() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("    \n");

    type_chars(&mut editor, "Istart");
    escape(&mut editor);

    assert_eq!(editor.buffer().content(), "    start\n");
    assert_eq!(editor.cursor.col, 8);
}

#[test]
fn counted_uppercase_i_repeats_inserted_text_as_one_change() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("    alpha\n");

    type_chars(&mut editor, "3Ix");
    escape(&mut editor);

    assert_eq!(editor.buffer().content(), "    xxxalpha\n");
    assert_eq!(editor.cursor.col, 6);

    editor.undo();
    assert_eq!(editor.buffer().content(), "    alpha\n");
}

#[test]
fn counted_uppercase_a_repeats_inserted_text_and_redoes_as_one_change() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("alpha\n");

    type_chars(&mut editor, "3Ax");
    escape(&mut editor);

    assert_eq!(editor.buffer().content(), "alphaxxx\n");
    assert_eq!(editor.cursor.col, 7);

    editor.undo();
    assert_eq!(editor.buffer().content(), "alpha\n");
    editor.redo();
    assert_eq!(editor.buffer().content(), "alphaxxx\n");
    assert_eq!(editor.cursor.col, 5);
}

#[test]
fn uppercase_i_redo_restores_cursor_to_insert_start() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("    alpha\n");

    type_chars(&mut editor, "Ixyz");
    escape(&mut editor);
    editor.undo();
    editor.redo();

    assert_eq!(editor.buffer().content(), "    xyzalpha\n");
    assert_eq!(editor.cursor.col, 4);
}

#[test]
fn uppercase_a_redo_restores_cursor_to_insert_start() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("alpha\n");

    type_chars(&mut editor, "Axyz");
    escape(&mut editor);
    editor.undo();
    editor.redo();

    assert_eq!(editor.buffer().content(), "alphaxyz\n");
    assert_eq!(editor.cursor.col, 5);
}
