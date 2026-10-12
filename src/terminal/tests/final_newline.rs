//! #294: an empty buffer (a new file, an empty file, or one emptied with
//! `dG`) must end in a newline once text goes in, like Neovim's
//! 'fixendofline', and undo must still take it back to empty.
//! Expected contents are the bytes `nvim --clean` writes for the same keys.

use super::handle_key;
use crate::editor::Editor;
use crate::input::key_notation::parse_key_sequence;

fn feed(editor: &mut Editor, keys: &str) {
    for key in parse_key_sequence(keys).unwrap() {
        handle_key(editor, key);
    }
}

fn editor_with(text: &str) -> Editor {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.registers.use_in_memory_clipboard_for_tests();
    editor.replace_buffer_content(text);
    editor
}

#[test]
fn edits_in_an_empty_buffer_end_with_a_newline() {
    let cases = [
        ("ix<Esc>", "x\n"),
        ("ix<BS><Esc>", "\n"),
        ("ix<CR>y<Esc>", "x\ny\n"),
        ("oy<Esc>", "\ny\n"),
        // A blank line opened at the end stays a real line.
        ("o<Esc>", "\n\n"),
        ("O<Esc>", "\n\n"),
        ("i<CR><Esc>", "\n\n"),
        ("ix<Esc>o<Esc>", "x\n\n"),
        ("Ox<Esc>", "x\n\n"),
        ("Ax<CR><Esc>", "x\n\n"),
        ("iabc<Esc>yy\"_ddp", "\nabc\n"),
        ("iabc<Esc>yy\"_ddP", "abc\n\n"),
        ("iabc<Esc>yiw\"_ddp", "abc\n"),
    ];
    for (keys, expected) in cases {
        let mut editor = editor_with("");
        feed(&mut editor, keys);
        assert_eq!(editor.buffer().content(), expected, "keys: {keys}");
    }
}

// These delete every line and put new text in, so the buffer is empty for a
// moment. The new text brings its own final newline; none may be added.
#[test]
fn replacing_every_line_keeps_one_final_newline() {
    let cases = [
        ("abc\n", "yyVp", "abc\n"),
        ("a\nb\n", "yyggVGp", "a\n"),
        ("abc\n", "gUG", "ABC\n"),
        ("abc\n", "g~G", "ABC\n"),
        ("a\nb\n", "gUG", "A\nB\n"),
        ("abc\n", "yyVpu", "abc\n"),
        ("abc\n", "gUGu<C-r>", "ABC\n"),
        // Characterwise text put over a linewise selection gets its own line.
        ("abc\n", "ylVp", "a\n"),
        ("a\nb\nc\n", "yljVp", "a\na\nc\n"),
        ("ab\ncd\n", "ylGVp", "ab\na\n"),
    ];
    for (text, keys, expected) in cases {
        let mut editor = editor_with(text);
        feed(&mut editor, keys);
        assert_eq!(editor.buffer().content(), expected, "{text:?} keys: {keys}");
    }
}

#[test]
fn edits_that_add_no_text_leave_an_empty_buffer_empty() {
    for keys in ["i<Esc>", "A<Esc>", "cc<Esc>", "o<Esc>u", "ix<Esc>u"] {
        let mut editor = editor_with("");
        feed(&mut editor, keys);
        assert_eq!(editor.buffer().content(), "", "keys: {keys}");
    }
}

#[test]
fn redo_after_undoing_to_empty_restores_the_final_newline() {
    let mut editor = editor_with("");
    feed(&mut editor, "ix<Esc>u<C-r>");
    assert_eq!(editor.buffer().content(), "x\n");
}

#[test]
fn undo_after_emptying_and_retyping_restores_the_original_text() {
    let mut editor = editor_with("a\nb\n");
    feed(&mut editor, "gg\"_dGix<Esc>uu");
    assert_eq!(editor.buffer().content(), "a\nb\n");

    feed(&mut editor, "<C-r><C-r>");
    assert_eq!(editor.buffer().content(), "x\n");
}

// A macro replays as one undo entry, so emptying the buffer and typing into
// it happen inside the same entry.
#[test]
fn undoing_a_macro_that_empties_and_retypes_restores_the_text() {
    let mut editor = editor_with("a\nb\n");
    feed(&mut editor, "qagg\"_dGix<Esc>quu");
    assert_eq!(editor.buffer().content(), "a\nb\n");

    feed(&mut editor, "@a");
    assert_eq!(editor.buffer().content(), "x\n");

    feed(&mut editor, "u");
    assert_eq!(editor.buffer().content(), "a\nb\n");
}
