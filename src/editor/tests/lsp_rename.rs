use crate::editor::{Editor, LspAction, Mode};
use crate::lsp::types::PrepareRenameOutcome;
use crate::terminal::handle_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn press(editor: &mut Editor, keys: &str) {
    for ch in keys.chars() {
        let code = if ch == '\n' {
            KeyCode::Enter
        } else {
            KeyCode::Char(ch)
        };
        handle_key(editor, KeyEvent::new(code, KeyModifiers::NONE));
    }
}

fn swift_editor() -> Editor {
    let mut editor = Editor::default();
    editor.replace_buffer_content("func makeSUT(\n    preferredCountry: String? = \"US\",\n) {}\n");
    editor.cursor.line = 1;
    editor.cursor.col = 4;
    editor
}

#[test]
fn leader_rn_asks_the_server_before_opening_the_prompt() {
    let mut editor = swift_editor();

    press(&mut editor, " rn");

    assert_eq!(editor.pending_lsp_action, Some(LspAction::PrepareRename));
    assert_eq!(editor.mode, Mode::Normal);
}

#[test]
fn rn_command_without_a_name_asks_the_server_first() {
    let mut editor = swift_editor();

    press(&mut editor, ":rn\n");

    assert_eq!(editor.pending_lsp_action, Some(LspAction::PrepareRename));
    assert_eq!(editor.mode, Mode::Normal);
}

// Issue #385: a Swift argument label belongs to the compound function name, so
// sourcekit-lsp needs `makeSUT(preferredCountry2:)` back. A bare label renames
// the function instead.
#[test]
fn swift_label_rename_sends_the_compound_name() {
    let mut editor = swift_editor();

    editor.open_rename_prompt(PrepareRenameOutcome::Placeholder(
        "makeSUT(preferredCountry:)".to_string(),
    ));
    assert_eq!(editor.mode, Mode::RenamePrompt);
    assert_eq!(editor.rename_input, "makeSUT(preferredCountry:)");

    editor.rename_input_backspace();
    editor.rename_input_backspace();
    for ch in "2:)".chars() {
        editor.rename_input_char(ch);
    }
    editor.confirm_rename();

    assert_eq!(
        editor.pending_lsp_action,
        Some(LspAction::RenameSymbol(
            "makeSUT(preferredCountry2:)".to_string()
        ))
    );
    assert_eq!(editor.mode, Mode::Normal);
}

#[test]
fn range_answer_prefills_the_buffer_text_using_utf16_columns() {
    let mut editor = Editor::default();
    // The emoji is one char but two UTF-16 units, so `name` starts at UTF-16
    // column 3 and char column 2.
    editor.replace_buffer_content("😀 name = 1\n");
    editor.cursor.col = 2;

    editor.open_rename_prompt(PrepareRenameOutcome::Range {
        line: 0,
        start_col: 3,
        end_col: 7,
    });

    assert_eq!(editor.mode, Mode::RenamePrompt);
    assert_eq!(editor.rename_input, "name");
}

#[test]
fn unusable_answers_fall_back_to_the_word_under_the_cursor() {
    let mut editor = swift_editor();
    editor.open_rename_prompt(PrepareRenameOutcome::WordUnderCursor);
    assert_eq!(editor.rename_input, "preferredCountry");

    let mut editor = swift_editor();
    editor.open_rename_prompt(PrepareRenameOutcome::Range {
        line: 1,
        start_col: 4,
        end_col: 4,
    });
    assert_eq!(editor.rename_input, "preferredCountry", "empty range");
}

#[test]
fn null_answer_reports_nothing_to_rename() {
    let mut editor = swift_editor();

    editor.open_rename_prompt(PrepareRenameOutcome::NotRenameable);

    assert_eq!(editor.mode, Mode::Normal);
    assert!(editor.rename_input.is_empty());
    assert_eq!(
        editor.status_message.as_deref(),
        Some("Nothing to rename here")
    );
}
