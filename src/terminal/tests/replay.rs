use super::handle_key;
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
    editor.registers.use_in_memory_clipboard_for_tests();
    editor.replace_buffer_content(text);
    editor
}

fn set_macro(editor: &mut Editor, register: char, keys: &str) {
    editor
        .macros
        .set_macro(register, parse_key_sequence(keys).unwrap());
}

/// The nesting limit stopped playback and left a usable Normal mode.
fn assert_stopped_at_limit(editor: &Editor) {
    assert!(!editor.should_quit);
    assert_eq!(editor.mode, Mode::Normal);
    assert!(!editor.pending_insert_normal_once);
    assert!(!editor.input_state.has_pending_sequence());
    assert!(
        editor
            .status_message
            .as_deref()
            .is_some_and(|s| s.contains("limit")),
        "expected the nesting limit message, got {:?}",
        editor.status_message
    );
}

// #338: the dot typed after Ctrl-O was replayed as a normal-mode `.`, which
// repeated the same keys again until the stack overflowed.
#[test]
fn dot_typed_after_insert_ctrl_o_repeats_as_text() {
    let mut editor = editor_with("original\n");
    feed(&mut editor, "i<C-o>h.<Esc>.");
    assert_eq!(editor.buffer().content(), "..original\n");
    assert_eq!(editor.mode, Mode::Normal);
}

#[test]
fn self_recursive_macro_stops_at_the_limit() {
    let mut editor = editor_with("original\n");
    // @a is still empty while recording, so the macro is just `@a`.
    feed(&mut editor, "qa@aq@a");
    assert_stopped_at_limit(&editor);
    assert_eq!(editor.buffer().content(), "original\n");

    set_macro(&mut editor, 'a', "A!<Esc>");
    feed(&mut editor, "@a.");
    assert_eq!(editor.buffer().content(), "original!!\n");
}

#[test]
fn mutually_recursive_macros_stop_at_the_limit() {
    let mut editor = editor_with("original\n");
    set_macro(&mut editor, 'a', "@bAx<Esc>");
    set_macro(&mut editor, 'b', "@aAy<Esc>");
    feed(&mut editor, "3@a");
    assert_stopped_at_limit(&editor);
    assert_eq!(editor.buffer().content(), "original\n");
}

// A failed motion does not end playback yet, so nothing tells a recursive
// macro where to stop and its edits up to the limit are rolled back.
#[test]
fn edits_before_the_limit_are_rolled_back() {
    let mut editor = editor_with("original\n");
    set_macro(&mut editor, 'a', "Ax<Esc>@aAy<Esc>");
    feed(&mut editor, "@a");
    assert_stopped_at_limit(&editor);
    assert_eq!(editor.buffer().content(), "original\n");
}

#[test]
fn rollback_never_undoes_an_earlier_change() {
    let mut editor = editor_with("original\n");
    feed(&mut editor, "A!<Esc>");
    set_macro(&mut editor, 'a', "@a");
    feed(&mut editor, "@a");
    assert_stopped_at_limit(&editor);
    assert_eq!(editor.buffer().content(), "original!\n");
    feed(&mut editor, "u");
    assert_eq!(editor.buffer().content(), "original\n");
}

#[test]
fn recursive_macro_under_insert_ctrl_o_ends_in_normal_mode() {
    let mut editor = editor_with("original\n");
    set_macro(&mut editor, 'a', "@@Ax<Esc>");
    feed(&mut editor, "i!<C-o>@a");
    assert_stopped_at_limit(&editor);
    assert_eq!(editor.buffer().content(), "!original\n");
    // A still-pending one-shot command would run `l` and resume Insert.
    feed(&mut editor, "l");
    assert_eq!(editor.mode, Mode::Normal);
}

#[test]
fn nested_macros_and_dot_repeat_still_work() {
    let mut editor = editor_with("abcdefghijklmnop\n");
    set_macro(&mut editor, 'a', "x.");
    set_macro(&mut editor, 'b', "@a@a");
    feed(&mut editor, "2@b");
    assert_eq!(editor.buffer().content(), "ijklmnop\n");
    feed(&mut editor, "u");
    assert_eq!(editor.buffer().content(), "abcdefghijklmnop\n");
}

#[test]
fn counted_macro_is_not_limited_by_nesting() {
    let mut editor = editor_with("original\n");
    set_macro(&mut editor, 'a', "A!<Esc>");
    feed(&mut editor, "100@a");
    assert_eq!(
        editor.buffer().content(),
        format!("original{}\n", "!".repeat(100))
    );
    feed(&mut editor, "u");
    assert_eq!(editor.buffer().content(), "original\n");
}
