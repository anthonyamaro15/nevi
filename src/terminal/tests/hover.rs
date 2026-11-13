//! The `K` hover popup: symbol title in the border, docs wrapped to the
//! box, and Ctrl-d / Ctrl-u paging with a counter when they don't fit.

use super::{render_editor_to_string, screen_row_text};
use crate::editor::Editor;
use crate::hover::Hover;
use crate::terminal::scroll_hover;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn hover_editor(markdown: &str) -> Editor {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.replace_buffer_content(&"line\n".repeat(30));
    editor.cursor.line = 20;
    editor.hover = Hover::for_editor(&editor, markdown);
    editor
}

fn screen(editor: &Editor) -> Vec<String> {
    let rendered = render_editor_to_string(editor);
    (0..24).map(|row| screen_row_text(&rendered, row)).collect()
}

fn row_with<'a>(screen: &'a [String], needle: &str) -> Option<&'a String> {
    screen.iter().find(|row| row.contains(needle))
}

#[test]
fn hover_shows_the_symbol_in_its_border_and_wraps_the_docs() {
    let editor = hover_editor(
        "```go\npackage fmt\n```\n\nPackage fmt implements formatted I/O with functions analogous\nto C's printf and scanf. The format 'verbs' are derived from C's but are simpler.",
    );
    let screen = screen(&editor);

    let top = screen
        .iter()
        .position(|row| row.contains("╭─ package fmt ─"))
        .expect("titled top border");
    assert_eq!(
        screen[top..top + 4],
        [
            " ╭─ package fmt ──────────────────────────────────────────────────────────────╮",
            " │ Package fmt implements formatted I/O with functions analogous to C's       │",
            " │ printf and scanf. The format 'verbs' are derived from C's but are simpler. │",
            " ╰────────────────────────────────────────────────────────────────────────────╯",
        ]
    );
    assert!(!screen.iter().any(|row| row.contains('…')));
    assert!(row_with(&screen, "Ctrl-d").is_none());
}

#[test]
fn ctrl_d_and_ctrl_u_page_a_long_hover() {
    let items: Vec<String> = (1..=40).map(|i| format!("- item {i}")).collect();
    let mut editor = hover_editor(&items.join("\n"));

    let first = screen(&editor);
    assert!(row_with(&first, "1/3 · Ctrl-d more").is_some());
    assert!(row_with(&first, "• item 16 ").is_some());
    assert!(row_with(&first, "• item 17 ").is_none());

    assert!(scroll_hover(&mut editor, ctrl('d')));
    let second = screen(&editor);
    assert!(row_with(&second, "2/3 · Ctrl-d more").is_some());
    assert!(row_with(&second, "• item 17 ").is_some());
    assert!(row_with(&second, "• item 1 ").is_none());

    assert!(scroll_hover(&mut editor, ctrl('d')));
    assert!(scroll_hover(&mut editor, ctrl('d')), "the last page holds");
    let last = screen(&editor);
    assert!(row_with(&last, "3/3 · Ctrl-u back").is_some());
    assert!(row_with(&last, "• item 40 ").is_some());

    assert!(scroll_hover(&mut editor, ctrl('u')));
    assert!(row_with(&screen(&editor), "2/3 · Ctrl-d more").is_some());

    let plain = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE);
    assert!(!scroll_hover(&mut editor, plain), "any other key closes it");
}

#[test]
fn a_hover_that_fits_lets_ctrl_d_scroll_the_buffer() {
    let mut editor = hover_editor("short doc");

    assert!(!scroll_hover(&mut editor, ctrl('d')));
}
