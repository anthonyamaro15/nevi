use crate::editor::Editor;
use crate::input::key_notation::parse_key_sequence;
use crate::terminal::handle_key;

// The `'[` `']` `'<` `'>` marks. Vim parity for each key is pinned by the
// oracle cases in vim_oracle/mark_cases.rs; these cover what the oracle
// snapshot cannot see or what nvim's defaults keep it from replaying.
fn editor_after(text: &str, keys: &str) -> Editor {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.replace_buffer_content(text);
    for key in parse_key_sequence(keys).expect("parse keys") {
        handle_key(&mut editor, key);
    }
    editor
}

#[test]
fn two_line_yank_end_mark_is_on_the_second_line() {
    let editor = editor_after("one two\nthree four\nfive six\n", "j2yygg']");
    assert_eq!(
        (editor.cursor.line, editor.cursor.col),
        (2, 0),
        "op_marks={:?}",
        editor.undo_stack.op_marks()
    );
}

#[test]
fn block_visual_marks_use_the_block_corners() {
    let editor = editor_after("abcd\nefgh\nijkl\n", "l<C-v>jl<Esc>gg`>");
    assert_eq!((editor.cursor.line, editor.cursor.col), (1, 2));
    let editor = editor_after("abcd\nefgh\nijkl\n", "jll<C-v>kh<Esc>G`<lt>");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 1));
}

#[test]
fn rapid_commands_keep_only_the_last_commands_marks() {
    // x and r land in one time-merged undo group; the marks still belong
    // to the last command alone, as in Vim.
    let editor = editor_after("abcdef\n", "lxllrZ0`[");
    assert_eq!(editor.buffer().content(), "acdZf\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 3));
}

#[test]
fn visual_marks_are_unset_before_any_selection() {
    let editor = editor_after("abc\ndef\n", "j'<lt>");
    assert_eq!(editor.cursor.line, 1);
}
