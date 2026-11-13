//! References rows show the code with the symbol marked; code actions are
//! numbered and tagged, and 1-9 apply one.

use super::{handle_key, render_editor_to_string, screen_row_text};
use crate::editor::Editor;
use crate::lsp::{CodeActionItem, Location, TextEdit, path_to_uri};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

fn temp_file(name: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nevi-lsp-pickers-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.go");
    std::fs::write(&path, text).unwrap();
    path
}

fn editor_on(path: &PathBuf) -> Editor {
    let mut editor = Editor::default();
    editor.set_size(100, 30);
    editor.open_file(path.clone()).unwrap();
    editor
}

fn screen(editor: &Editor) -> Vec<String> {
    let rendered = render_editor_to_string(editor);
    (0..30).map(|row| screen_row_text(&rendered, row)).collect()
}

fn row_with<'a>(screen: &'a [String], needle: &str) -> &'a str {
    screen
        .iter()
        .find(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?} in {screen:#?}"))
}

#[test]
fn references_show_each_line_of_code_with_the_symbol_title() {
    let path = temp_file(
        "refs",
        "package main\n\nfunc total(orders []int) int {\n\treturn 0\n}\n\nfunc main() {\n\tprintln(total(nil))\n}\n",
    );
    let mut editor = editor_on(&path);
    let uri = path_to_uri(&path);
    editor.show_references_picker(vec![
        Location {
            uri: uri.clone(),
            line: 2,
            col: 5,
        },
        Location {
            uri,
            line: 7,
            col: 9,
        },
    ]);
    let screen = screen(&editor);

    assert!(row_with(&screen, "╭─ References · total · 2 ─").ends_with('╮'));
    let first = row_with(&screen, "main.go:3: func total(orders []int)");
    assert!(first.contains("│▌"), "selected row has the bar: {first:?}");
    let second = row_with(&screen, "main.go:8: println(total(nil))");
    assert!(second.contains("│ "), "{second:?}");
    assert!(row_with(&screen, "╰─ Enter go · Esc close ─").ends_with('╯'));
    assert!(
        editor
            .status_message
            .as_deref()
            .is_none_or(|m| !m.contains("j/k to navigate"))
    );

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

fn action(title: &str, kind: Option<&str>, uri: &str, text: &str) -> CodeActionItem {
    CodeActionItem {
        title: title.to_string(),
        kind: kind.map(str::to_string),
        is_preferred: false,
        edits: vec![(
            uri.to_string(),
            vec![TextEdit {
                start_line: 0,
                start_col: 0,
                end_line: 0,
                end_col: 0,
                new_text: text.to_string(),
            }],
        )],
        command: None,
    }
}

#[test]
fn code_actions_are_numbered_and_tagged_and_a_digit_applies_one() {
    let path = temp_file("actions", "package main\n");
    let mut editor = editor_on(&path);
    let uri = path_to_uri(&path);
    editor.show_code_actions_picker(vec![
        action(
            "Add test for total",
            Some("source.addTest"),
            &uri,
            "// test\n",
        ),
        action(
            "Extract function",
            Some("refactor.extract.function"),
            &uri,
            "// extract\n",
        ),
        action("Fix it", Some("quickfix"), &uri, "// fix\n"),
    ]);
    let screen = screen(&editor);

    assert!(row_with(&screen, "─ Code actions · 3 ─").contains('╮'));
    let row = |needle| {
        row_with(&screen, needle)
            .split('│')
            .nth(1)
            .unwrap()
            .to_string()
    };
    assert_eq!(row("Add test"), " 1  Add test for total     test ");
    assert_eq!(row("Extract"), " 2  Extract function    extract ");
    assert_eq!(row("Fix it"), " 3  Fix it                  fix ");
    assert!(row_with(&screen, "─ 1-9 pick · Enter apply ─").contains('╯'));

    // 4 has no action, so nothing happens and the picker stays.
    handle_key(
        &mut editor,
        KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE),
    );
    assert!(editor.code_actions_picker.is_some());

    handle_key(
        &mut editor,
        KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
    );
    assert!(editor.code_actions_picker.is_none());
    assert_eq!(editor.buffer().content(), "// extract\npackage main\n");

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
