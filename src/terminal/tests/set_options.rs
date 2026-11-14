//! `:set` typed on the command line, end to end.

use super::{handle_key, render_editor_to_string, screen_row_text};
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

fn screen(editor: &Editor) -> Vec<String> {
    let rendered = render_editor_to_string(editor);
    (0..24).map(|row| screen_row_text(&rendered, row)).collect()
}

/// The text after `name` on the popup row that lists it.
fn listed_value(screen: &[String], name: &str) -> Option<String> {
    let row = screen.iter().find(|row| row.contains(name))?;
    let after = &row[row.find(name)? + name.len()..];
    Some(after.trim().trim_end_matches('\u{2502}').trim().to_string())
}

#[test]
fn typing_set_lists_the_options_with_their_current_values() {
    let mut editor = editor_with("one\ntwo\n");
    editor.settings.editor.scroll_off = 5;

    feed(&mut editor, ":set ");
    let rows = screen(&editor);

    assert!(rows.iter().any(|row| row.contains("Options")), "{rows:#?}");
    assert!(
        !rows.iter().any(|row| row.contains("TerminalSelect")),
        "commands must not be offered as :set arguments: {rows:#?}"
    );
    assert_eq!(listed_value(&rows, " number").as_deref(), Some("on"));
    assert_eq!(
        listed_value(&rows, "relativenumber").as_deref(),
        Some("off")
    );
    assert_eq!(listed_value(&rows, "tabstop").as_deref(), Some("4"));
    assert_eq!(listed_value(&rows, "scrolloff").as_deref(), Some("5"));
    assert_eq!(listed_value(&rows, "signcolumn").as_deref(), Some("yes"));
}

#[test]
fn the_option_list_narrows_as_you_type_and_shows_new_values() {
    let mut editor = editor_with("one\ntwo\n");
    feed(&mut editor, ":set rnu<CR>");

    feed(&mut editor, ":set rel");
    let rows = screen(&editor);

    assert_eq!(listed_value(&rows, "relativenumber").as_deref(), Some("on"));
    assert!(!rows.iter().any(|row| row.contains("tabstop")), "{rows:#?}");

    // Tab completes the name and Enter runs it.
    feed(&mut editor, "<BS><BS><BS>norel<Tab><CR>");
    assert_eq!(editor.mode, Mode::Normal);
    assert!(!editor.settings.editor.relative_numbers);
}
