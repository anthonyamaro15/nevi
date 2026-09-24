use crate::editor::Editor;
use crate::input::key_notation::parse_key_sequence;
use crate::terminal::handle_key;

// Operators with linewise motions. Vim parity is pinned by the oracle
// cases in vim_oracle/linewise_cases.rs; these run without nvim so a local
// `cargo test` catches a regression in the classification, the buffer-edge
// rule, the M formula, and the scrolloff rule for H under an operator.
fn editor_with(text: &str, size: (u16, u16), scroll_off: usize) -> Editor {
    let mut editor = Editor::default();
    editor.set_size(size.0, size.1);
    editor.settings.editor.scroll_off = scroll_off;
    editor.replace_buffer_content(text);
    editor.update_pane_rects();
    editor
}

fn feed(editor: &mut Editor, keys: &str) {
    for key in parse_key_sequence(keys).expect("parse keys") {
        handle_key(editor, key);
    }
}

fn lines(editor: &Editor) -> Vec<String> {
    editor
        .buffer()
        .content()
        .lines()
        .map(str::to_string)
        .collect()
}

const FOUR_LINES: &str = "one two\nthree four\nfive six\nseven eight\n";

#[test]
fn dj_deletes_two_whole_lines_and_keeps_the_column() {
    let mut editor = editor_with(FOUR_LINES, (80, 24), 0);
    feed(&mut editor, "jlldj");
    assert_eq!(lines(&editor), ["one two", "seven eight"]);
    assert_eq!((editor.cursor.line, editor.cursor.col), (1, 2));
}

#[test]
fn j_and_k_at_the_buffer_edge_cancel_the_operator() {
    let mut editor = editor_with(FOUR_LINES, (80, 24), 0);
    feed(&mut editor, "Gdj");
    feed(&mut editor, "ggdk");
    assert_eq!(lines(&editor).len(), 4);
}

#[test]
fn g_on_its_own_line_still_deletes_it() {
    let mut editor = editor_with(FOUR_LINES, (80, 24), 0);
    feed(&mut editor, "GdG");
    assert_eq!(lines(&editor), ["one two", "three four", "five six"]);
    feed(&mut editor, "ggdgg");
    assert_eq!(lines(&editor), ["three four", "five six"]);
}

#[test]
fn cj_collapses_the_lines_like_cc() {
    let mut editor = editor_with("one\n    two\n  three\nfour\n", (80, 24), 0);
    feed(&mut editor, "jcjX<Esc>");
    assert_eq!(lines(&editor), ["one", "    X", "four"]);
}

#[test]
fn yk_yanks_linewise_and_moves_up() {
    let mut editor = editor_with(FOUR_LINES, (80, 24), 0);
    // A named register: the unnamed one is shared with the clipboard and
    // other tests running in parallel.
    feed(&mut editor, "jjl\"ayk");
    assert_eq!((editor.cursor.line, editor.cursor.col), (1, 1));
    feed(&mut editor, "G\"ap");
    assert_eq!(lines(&editor).len(), 6);
    assert_eq!(lines(&editor)[4..], ["three four", "five six"]);
}

#[test]
fn m_on_a_short_buffer_is_the_middle_of_the_lines_shown() {
    let mut editor = editor_with(FOUR_LINES, (80, 24), 0);
    feed(&mut editor, "GM");
    assert_eq!(editor.cursor.line, 1);
    feed(&mut editor, "GdM");
    assert_eq!(lines(&editor), ["one two"]);
}

#[test]
fn h_honours_scrolloff_when_moving_but_not_under_an_operator() {
    let text: String = (1..=40).map(|i| format!("line{i}\n")).collect();
    // 12 rows leave 10 text rows; scroll_off 2 keeps H two lines below the top.
    let mut editor = editor_with(&text, (80, 12), 2);
    feed(&mut editor, "20Gztjj");
    let top = editor.viewport_offset;
    let cursor_before = editor.cursor.line;
    assert!(
        top + 2 < cursor_before,
        "setup: top={top} cursor={cursor_before}"
    );

    feed(&mut editor, "H");
    assert_eq!(editor.cursor.line, top + 2, "plain H stops at scrolloff");

    feed(&mut editor, "jjdH");
    let deleted = cursor_before - top + 1;
    assert_eq!(
        lines(&editor).len(),
        40 - deleted,
        "dH reaches the true top line"
    );
    assert_eq!(lines(&editor)[top], format!("line{}", cursor_before + 2));
}

#[test]
fn indent_motion_with_j_covers_two_lines() {
    let mut editor = editor_with(FOUR_LINES, (80, 24), 0);
    feed(&mut editor, "j>j");
    let after = lines(&editor);
    assert_eq!(after[0], "one two");
    assert_eq!(after[1].trim_start(), "three four");
    assert_ne!(after[1], "three four");
    assert_ne!(after[2], "five six");
    assert_eq!(after[3], "seven eight");
    // On the last line j cannot move, so the operator is cancelled.
    feed(&mut editor, "G>j");
    assert_eq!(lines(&editor)[3], "seven eight");
}
