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
fn counted_o_repeats_indented_line_as_one_change() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("    alpha\nomega\n");

    type_chars(&mut editor, "3obelow");
    escape(&mut editor);

    assert_eq!(
        editor.buffer().content(),
        "    alpha\n    below\n    below\n    below\nomega\n"
    );
    assert_eq!((editor.cursor.line, editor.cursor.col), (3, 8));

    editor.undo();
    assert_eq!(editor.buffer().content(), "    alpha\nomega\n");
}

#[test]
fn counted_uppercase_o_repeats_indented_line_as_one_change() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("alpha\n    omega\n");

    type_chars(&mut editor, "j3Oabove");
    escape(&mut editor);

    assert_eq!(
        editor.buffer().content(),
        "alpha\n    above\n    above\n    above\n    omega\n"
    );
    assert_eq!((editor.cursor.line, editor.cursor.col), (3, 8));

    editor.undo();
    assert_eq!(editor.buffer().content(), "alpha\n    omega\n");
}

#[test]
fn counted_o_without_text_opens_requested_blank_lines() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("alpha\nomega\n");

    type_chars(&mut editor, "3o");
    escape(&mut editor);

    assert_eq!(editor.buffer().content(), "alpha\n\n\n\nomega\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (3, 0));
}

#[test]
fn open_line_redo_uses_the_original_command_coordinate() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("alpha\nomega\n");

    type_chars(&mut editor, "$obelow");
    escape(&mut editor);
    editor.undo();
    editor.redo();

    assert_eq!(editor.buffer().content(), "alpha\nbelow\nomega\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 4));
}

#[test]
fn counted_uppercase_o_redo_restores_all_lines_and_original_coordinate() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("alpha\nomega\n");

    type_chars(&mut editor, "j3Oabove");
    escape(&mut editor);
    editor.undo();
    editor.redo();

    assert_eq!(
        editor.buffer().content(),
        "alpha\nabove\nabove\nabove\nomega\n"
    );
    assert_eq!((editor.cursor.line, editor.cursor.col), (1, 0));
}
