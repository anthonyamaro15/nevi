//! `:set` typed on the command line, end to end.

use super::handle_key;
use crate::commands::{Command, parse_command};
use crate::editor::{Editor, Mode};
use crate::input::key_notation::parse_key_sequence;

fn feed(editor: &mut Editor, keys: &str) {
    for key in parse_key_sequence(keys).unwrap() {
        handle_key(editor, key);
    }
}

fn editor_with(text: &str) -> Editor {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.update_pane_rects();
    editor.replace_buffer_content(text);
    editor
}

#[test]
fn set_takes_everything_after_the_command_name() {
    assert!(matches!(
        parse_command("set nonu rnu so=3"),
        Command::Set(args) if args == "nonu rnu so=3"
    ));
    assert!(matches!(
        parse_command("set"),
        Command::Unknown(message) if message == "set: missing option"
    ));
}

#[test]
fn typed_set_changes_options_and_answers_queries() {
    let mut editor = editor_with("one\ntwo\n");

    feed(&mut editor, ":set nonu rnu cul<CR>");
    assert_eq!(editor.mode, Mode::Normal);
    assert!(!editor.settings.editor.line_numbers);
    assert!(editor.settings.editor.relative_numbers);
    assert!(editor.settings.editor.cursor_line);

    feed(&mut editor, ":set ts?<CR>");
    assert_eq!(editor.status_message.as_deref(), Some("  tabstop=4"));

    feed(&mut editor, ":set nots<CR>");
    assert!(
        editor
            .status_message
            .as_deref()
            .is_some_and(|message| message.contains("Invalid argument: nots")),
        "status was {:?}",
        editor.status_message
    );
}

#[test]
fn a_scroll_margin_larger_than_the_window_keeps_the_cursor_on_screen() {
    // `:set so=999` is the Vim idiom for keeping the cursor centered.
    let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
    let mut editor = editor_with(&text);

    feed(&mut editor, ":set so=999<CR>");
    feed(&mut editor, "50jkkG");
    let text_rows = 22;
    assert_eq!(editor.cursor.line, 199);
    assert!(editor.viewport_offset <= editor.cursor.line);
    assert!(editor.cursor.line < editor.viewport_offset + text_rows);

    feed(&mut editor, "gg10j");
    assert_eq!(editor.cursor.line, 10);
    assert!(editor.viewport_offset <= editor.cursor.line);
    assert!(editor.cursor.line < editor.viewport_offset + text_rows);
}
