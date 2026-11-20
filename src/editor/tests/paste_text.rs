use crate::editor::{Editor, Mode, RegisterContent};
use crate::explorer::ExplorerAction;
use crate::input::key_notation::parse_key_sequence;
use crate::lsp::PrepareRenameOutcome;
use crate::terminal::handle_key;

fn feed(editor: &mut Editor, keys: &str) {
    for key in parse_key_sequence(keys).unwrap() {
        handle_key(editor, key);
    }
}

fn editor_with(content: &str) -> Editor {
    let mut editor = Editor::default();
    editor.settings.editor.auto_pairs = true;
    editor.settings.editor.auto_indent = true;
    editor.replace_buffer_content(content);
    editor
}

fn text_of(content: Option<RegisterContent>) -> Option<String> {
    content.map(|content| content.as_str().to_string())
}

#[test]
fn normal_mode_paste_of_commands_only_adds_text() {
    let mut editor = editor_with("one\ntwo\n");

    editor.paste_text("dd");
    assert_eq!(editor.buffer().content(), "oddne\ntwo\n");

    editor.paste_text(":q!\n");
    editor.paste_text(":!echo pwned\n");
    assert_eq!(editor.buffer().content(), "odd:q!\nn:!echo pwned\ne\ntwo\n");
    assert_eq!(editor.mode, Mode::Normal);
    assert!(!editor.should_quit);
    assert!(editor.pending_external_command.is_none());
}

#[test]
fn normal_mode_paste_cancels_a_pending_operator_and_count() {
    let mut editor = editor_with("abc def\n");
    feed(&mut editor, "d");
    editor.paste_text("w");
    assert!(!editor.input_state.has_pending_sequence());
    feed(&mut editor, "w");
    assert_eq!(editor.buffer().content(), "awbc def\n");

    let mut editor = editor_with("abcdef\n");
    feed(&mut editor, "3");
    editor.paste_text("Z");
    feed(&mut editor, "0x");
    // One character deleted, not three: the count died with the paste.
    assert_eq!(editor.buffer().content(), "Zbcdef\n");
}

#[test]
fn normal_mode_paste_keeps_the_previous_change_for_dot() {
    let mut editor = editor_with("abcdef\n");
    feed(&mut editor, "x");
    feed(&mut editor, "d");
    editor.paste_text("Z");
    assert_eq!(editor.buffer().content(), "bZcdef\n");
    // `.` repeats the `x`, not the cancelled `d` and not the paste.
    feed(&mut editor, "0.");
    assert_eq!(editor.buffer().content(), "Zcdef\n");
}

#[test]
fn undo_reverts_a_normal_mode_paste_in_one_step() {
    let mut editor = editor_with("one\n");
    editor.paste_text("a\nb\n");
    assert_eq!(editor.buffer().content(), "oa\nb\nne\n");
    feed(&mut editor, "u");
    assert_eq!(editor.buffer().content(), "one\n");

    editor.paste_text("xyz");
    assert_eq!(editor.buffer().content(), "oxyzne\n");
    feed(&mut editor, "u");
    assert_eq!(editor.buffer().content(), "one\n");
}

/// Paste `text` into `content` after `keys`, and return the buffer and the
/// cursor position.
fn paste_after(content: &str, keys: &str, text: &str) -> (String, (usize, usize)) {
    let mut editor = editor_with(content);
    feed(&mut editor, keys);
    editor.paste_text(text);
    assert_eq!(editor.mode, Mode::Normal, "pasting {text:?} after {keys:?}");
    (
        editor.buffer().content(),
        (editor.cursor.line, editor.cursor.col),
    )
}

// Expected values in the next two tests were measured in Neovim 0.11.3 with
// bracketed pastes through tmux: the text goes in characterwise and the cursor
// ends on the last pasted character, or at the start of the line after the
// paste when the text ends in a newline.
#[test]
fn normal_mode_paste_matches_neovim() {
    let buffer = "one\ntwo\nthree\n\n";
    for (keys, pasted, expected, cursor) in [
        // On the `w` of `two`.
        ("jl", "Q", "one\ntwQo\nthree\n\n", (1, 2)),
        ("jl", "a\nb", "one\ntwa\nbo\nthree\n\n", (2, 0)),
        ("jl", "X\n", "one\ntwX\no\nthree\n\n", (2, 0)),
        ("jl", "p\nq\n", "one\ntwp\nq\no\nthree\n\n", (3, 0)),
        ("jl", "\n", "one\ntw\no\nthree\n\n", (2, 0)),
        // On the empty last line.
        ("G", "Q", "one\ntwo\nthree\nQ\n", (3, 0)),
        ("G", "a\nb", "one\ntwo\nthree\na\nb\n", (4, 0)),
        ("G", "X\n", "one\ntwo\nthree\nX\n\n", (4, 0)),
        ("G", "p\nq\n", "one\ntwo\nthree\np\nq\n\n", (5, 0)),
        ("G", "\n", "one\ntwo\nthree\n\n\n", (4, 0)),
        // On the last character of a line.
        ("j$", "Q", "one\ntwoQ\nthree\n\n", (1, 3)),
        ("j$", "a\nb", "one\ntwoa\nb\nthree\n\n", (2, 0)),
        ("j$", "X\n", "one\ntwoX\n\nthree\n\n", (2, 0)),
    ] {
        assert_eq!(
            paste_after(buffer, keys, pasted),
            (expected.to_string(), cursor),
            "pasting {pasted:?} after {keys:?}"
        );
    }

    // Last character of the last line. Nevi loads a file without a final
    // newline as if it had one, so this also covers that file.
    for (pasted, expected, cursor) in [
        ("Q", "one\ntwoQ\n", (1, 3)),
        ("a\nb", "one\ntwoa\nb\n", (2, 0)),
        ("X\n", "one\ntwoX\n\n", (2, 0)),
        ("\n", "one\ntwo\n\n", (2, 0)),
    ] {
        assert_eq!(
            paste_after("one\ntwo", "G$", pasted),
            (expected.to_string(), cursor),
            "pasting {pasted:?}"
        );
    }
}

#[test]
fn charwise_visual_paste_matches_neovim() {
    let buffer = "one\ntwo\nthree\n\n";
    for (pasted, expected, cursor) in [
        ("Q", "one\nQ\nthree\n\n", (1, 0)),
        ("a\nb", "one\na\nb\nthree\n\n", (2, 0)),
        ("X\n", "one\nX\n\nthree\n\n", (2, 0)),
        ("p\nq\n", "one\np\nq\n\nthree\n\n", (3, 0)),
        ("\n", "one\n\n\nthree\n\n", (2, 0)),
    ] {
        assert_eq!(
            paste_after(buffer, "jviw", pasted),
            (expected.to_string(), cursor),
            "pasting {pasted:?}"
        );
    }
}

#[test]
fn linewise_and_block_visual_paste_keep_their_shape() {
    // Neovim joins a `V` paste with the next line. Nevi replaces the lines
    // with the text as whole lines and puts the cursor on the first one.
    let buffer = "one\ntwo\nthree\n";
    for (pasted, expected) in [
        ("Q", "one\nQ\nthree\n"),
        ("a\nb", "one\na\nb\nthree\n"),
        ("X\n", "one\nX\nthree\n"),
    ] {
        assert_eq!(
            paste_after(buffer, "jV", pasted),
            (expected.to_string(), (1, 0)),
            "pasting {pasted:?}"
        );
    }

    assert_eq!(
        paste_after("abc\ndef\n", "l<C-v>j", "X"),
        ("aXc\ndXf\n".to_string(), (0, 1))
    );
}

#[test]
fn ctrl_o_paste_puts_the_text_and_returns_to_insert() {
    let mut editor = editor_with("one\ntwo\n");
    feed(&mut editor, "jA<C-o>");
    editor.paste_text("a\nb");
    assert_eq!(editor.mode, Mode::Insert);
    assert_eq!(editor.buffer().content(), "one\ntwoa\nb\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (2, 0));
}

#[test]
fn insert_mode_paste_arrives_byte_for_byte() {
    for (pasted, expected) in [
        ("don't", "don't"),
        ("a\n  b\nc", "a\n  b\nc"),
        ("fn f() {\n    x\n}", "fn f() {\n    x\n}"),
        ("a\r\nb\rc", "a\nb\nc"),
        ("(x) [y] {z} \"q\" `r` 'w'", "(x) [y] {z} \"q\" `r` 'w'"),
    ] {
        let mut editor = editor_with("X\n");
        feed(&mut editor, "i");
        editor.paste_text(pasted);
        assert_eq!(editor.mode, Mode::Insert);
        feed(&mut editor, "<Esc>");
        assert_eq!(
            editor.buffer().content(),
            format!("{expected}X\n"),
            "pasting {pasted:?}"
        );
    }
}

#[test]
fn insert_mode_paste_is_one_undo_step_with_the_typing() {
    let mut editor = editor_with("X\n");
    feed(&mut editor, "ia");
    editor.paste_text("b\n  c");
    feed(&mut editor, "d<Esc>");
    assert_eq!(editor.buffer().content(), "ab\n  cdX\n");
    feed(&mut editor, "u");
    assert_eq!(editor.buffer().content(), "X\n");
}

#[test]
fn insert_mode_paste_does_not_complete_a_pending_ctrl_r() {
    let mut editor = editor_with("X\n");
    editor
        .registers
        .yank(Some('a'), RegisterContent::Chars("SECRET".into()));
    feed(&mut editor, "i<C-r>");
    editor.paste_text("abc");
    feed(&mut editor, "<Esc>");
    assert_eq!(editor.buffer().content(), "abcX\n");
}

#[test]
fn dot_replays_an_insert_mode_paste_literally() {
    let mut editor = editor_with("X\nY\n");
    feed(&mut editor, "i");
    editor.paste_text("it's {a\n  }");
    feed(&mut editor, "<Esc>");
    assert_eq!(editor.buffer().content(), "it's {a\n  }X\nY\n");

    feed(&mut editor, "G0.");
    assert_eq!(editor.buffer().content(), "it's {a\n  }X\nit's {a\n  }Y\n");
}

#[test]
fn replace_mode_paste_overwrites_literally() {
    let mut editor = editor_with("abcdef\n");
    feed(&mut editor, "R");
    editor.paste_text("(x'");
    feed(&mut editor, "<Esc>");
    assert_eq!(editor.buffer().content(), "(x'def\n");
}

#[test]
fn visual_paste_replaces_the_selection_and_keeps_registers() {
    let mut editor = editor_with("hello world\n");
    editor
        .registers
        .yank(None, RegisterContent::Chars("unnamed".into()));
    editor
        .registers
        .yank(Some('a'), RegisterContent::Chars("named".into()));
    let unnamed = text_of(editor.registers.get_content(None));
    let small_delete = text_of(editor.registers.get_content(Some('-')));
    let yank = text_of(editor.registers.get_content(Some('0')));
    let numbered = text_of(editor.registers.get_content(Some('1')));

    feed(&mut editor, "viw");
    editor.paste_text("bye");
    assert_eq!(editor.buffer().content(), "bye world\n");
    assert_eq!(editor.mode, Mode::Normal);

    let mut editor_lines = editor_with("a\nb\nc\n");
    feed(&mut editor_lines, "Vj");
    editor_lines.paste_text("X");
    assert_eq!(editor_lines.buffer().content(), "X\nc\n");
    assert_eq!(editor_lines.mode, Mode::Normal);

    editor.paste_text("dd");
    assert_eq!(text_of(editor.registers.get_content(None)), unnamed);
    assert_eq!(
        text_of(editor.registers.get_content(Some('a'))).as_deref(),
        Some("named")
    );
    assert_eq!(
        text_of(editor.registers.get_content(Some('-'))),
        small_delete
    );
    assert_eq!(text_of(editor.registers.get_content(Some('0'))), yank);
    assert_eq!(text_of(editor.registers.get_content(Some('1'))), numbered);
}

#[test]
fn command_line_paste_takes_the_first_line_and_never_submits() {
    let mut editor = editor_with("one\n");
    feed(&mut editor, ":");
    editor.paste_text("echo one\nq!\n");
    assert_eq!(editor.mode, Mode::Command);
    assert_eq!(editor.command_line.input, "echo one");
    assert!(!editor.should_quit);

    feed(&mut editor, "<Esc>:");
    editor.paste_text("!echo\tpwned\r\n");
    assert_eq!(editor.mode, Mode::Command);
    assert_eq!(editor.command_line.input, "!echopwned");
    assert!(editor.pending_external_command.is_none());

    // A pending <C-r> must not read the paste's first character as a
    // register name.
    feed(&mut editor, "<Esc>:<C-r>");
    editor.paste_text("abc");
    assert_eq!(editor.command_line.input, "abc");
    assert!(!editor.command_line.pending_register);
}

#[test]
fn search_paste_takes_the_first_line_and_never_submits() {
    let mut editor = editor_with("foo\nbar\n");
    feed(&mut editor, "/");
    editor.paste_text("bar\nfoo\n");
    assert_eq!(editor.mode, Mode::Search);
    assert_eq!(editor.search.input, "bar");
    assert_eq!(editor.buffer().content(), "foo\nbar\n");
}

#[test]
fn finder_query_takes_the_first_line() {
    let mut editor = editor_with("one\n");
    editor.open_finder_buffers();
    assert_eq!(editor.mode, Mode::Finder);
    editor.paste_text("src/main\nrs\n");
    assert_eq!(editor.mode, Mode::Finder);
    assert_eq!(editor.finder.query, "src/main");
}

#[test]
fn rename_and_explorer_prompts_take_the_first_line() {
    let mut editor = editor_with("one\n");
    editor.open_rename_prompt(PrepareRenameOutcome::Placeholder("old".into()));
    editor.paste_text("_new\nrest");
    assert_eq!(editor.mode, Mode::RenamePrompt);
    assert_eq!(editor.rename_input, "old_new");
    assert!(editor.pending_lsp_action.is_none());

    let mut editor = editor_with("one\n");
    editor.mode = Mode::Explorer;
    editor.explorer.pending_action = Some(ExplorerAction::Add);
    editor.paste_text("notes.txt\nmore");
    assert_eq!(editor.explorer.input_buffer, "notes.txt");
    assert_eq!(editor.explorer.pending_action, Some(ExplorerAction::Add));
}

#[test]
fn explorer_prompt_takes_a_non_ascii_paste() {
    let mut editor = editor_with("one\n");
    editor.mode = Mode::Explorer;
    editor.explorer.start_add();
    editor.paste_text("café-日本語");
    editor.paste_text(".txt");
    assert_eq!(editor.explorer.input_buffer, "café-日本語.txt");
    assert_eq!(
        editor.explorer.input_cursor,
        "café-日本語.txt".chars().count()
    );
}

#[test]
fn states_without_a_text_input_ignore_the_paste() {
    // Start screen.
    let mut editor = Editor::default();
    assert!(editor.dashboard_active());
    editor.paste_text("1");
    assert!(editor.buffer().is_empty());
    assert!(editor.dashboard_active());

    // Explorer delete confirmation, where `y` would delete.
    let mut editor = editor_with("one\n");
    editor.mode = Mode::Explorer;
    editor.explorer.pending_action = Some(ExplorerAction::Delete);
    editor.paste_text("y\n");
    assert_eq!(editor.explorer.pending_action, Some(ExplorerAction::Delete));
    assert!(editor.explorer.input_buffer.is_empty());

    // Explorer tree, where letters are commands.
    editor.explorer.pending_action = None;
    editor.paste_text("dd");
    assert_eq!(editor.mode, Mode::Explorer);
    assert!(editor.explorer.pending_action.is_none());

    // Half-typed leader sequence.
    let mut editor = editor_with("one\n");
    editor.leader_sequence = Some(String::new());
    editor.paste_text("ff");
    assert_eq!(editor.buffer().content(), "one\n");
    assert_eq!(editor.leader_sequence.as_deref(), Some(""));
}
