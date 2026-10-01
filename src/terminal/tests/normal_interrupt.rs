use super::{handle_key, unique_temp_dir};
use crate::config::{KeymapEntry, KeymapLookup, LeaderAction, LeaderMapping};
use crate::editor::{Editor, Mode};
use crate::input::key_notation::parse_key_sequence;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::Instant;

fn feed(editor: &mut Editor, keys: &str) {
    for key in parse_key_sequence(keys).unwrap() {
        handle_key(editor, key);
    }
}

#[test]
fn normal_ctrl_c_stays_open_and_shows_exit_guidance() {
    for dirty in [false, true] {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abc def\n");
        editor.buffer_mut().dirty = dirty;
        let expected = if dirty {
            "Type  :qa!  and press <Enter> to abandon all changes and exit Nevi"
        } else {
            "Type  :qa  and press <Enter> to exit Nevi"
        };
        for _ in 0..2 {
            feed(&mut editor, "<C-c>");
            assert!(!editor.should_quit);
            assert_eq!(editor.mode, Mode::Normal);
            assert_eq!(editor.buffer().content(), "abc def\n");
            assert_eq!(editor.buffer().dirty, dirty);
            assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));
            assert_eq!(editor.status_message.as_deref(), Some(expected));
        }
        feed(&mut editor, "l");
        assert_eq!(editor.cursor.col, 1);
        assert!(editor.status_message.is_none());
    }
}

#[test]
fn normal_ctrl_c_cancels_pending_input_without_editing() {
    for prefix in [
        "r", "f", "t", "2", "d", "2d3", "\"", "\"a", "g", "z", "<C-w>", "m", "q", "@", "di", "gu",
        "gc",
    ] {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abc def\n");
        let version = editor.buffer().version();
        feed(&mut editor, prefix);
        assert!(editor.input_state.has_pending_sequence(), "{prefix}");
        feed(&mut editor, "<C-c>");
        assert!(!editor.should_quit, "{prefix}");
        assert_eq!(editor.mode, Mode::Normal, "{prefix}");
        assert_eq!(editor.buffer().content(), "abc def\n", "{prefix}");
        assert_eq!(editor.buffer().version(), version, "{prefix}");
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0), "{prefix}");
        assert!(!editor.input_state.has_pending_sequence(), "{prefix}");
        assert!(editor.status_message.is_none(), "{prefix}");
        feed(&mut editor, "l");
        assert_eq!(editor.cursor.col, 1, "{prefix}");
    }
}

#[test]
fn normal_ctrl_c_cancels_pending_leader_action() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc def\n");
    editor.settings.keymap.leader_mappings = vec![LeaderMapping {
        key: "c".into(),
        action: ":qa!<CR>".into(),
        desc: None,
    }];
    editor.keymap = KeymapLookup::from_settings(&editor.settings.keymap).0;
    feed(&mut editor, "<Space>");
    assert!(editor.leader_sequence.is_some());
    editor.leader_sequence_start = Some(Instant::now());
    editor.leader_pending_action = Some(LeaderAction::Command("qa!".into()));
    feed(&mut editor, "<C-c>");
    assert!(!editor.should_quit);
    assert!(editor.leader_sequence.is_none());
    assert!(editor.leader_sequence_start.is_none());
    assert!(editor.leader_pending_action.is_none());
    assert!(editor.status_message.is_none());
    assert_eq!(editor.buffer().content(), "abc def\n");
}

#[test]
fn normal_ctrl_c_preserves_explicit_remapping() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc def\n");
    editor.settings.keymap.normal.push(KeymapEntry {
        from: "<C-c>".into(),
        to: "l".into(),
    });
    editor.keymap = KeymapLookup::from_settings(&editor.settings.keymap).0;
    feed(&mut editor, "<C-c>");
    assert!(!editor.should_quit);
    assert_eq!(editor.cursor.col, 1);
    assert!(editor.status_message.is_none());
}

#[test]
fn normal_ctrl_c_accounts_for_hidden_unsaved_changes() {
    let tmp = unique_temp_dir("nevi_normal_ctrl_c_hidden");
    std::fs::create_dir_all(&tmp).unwrap();
    let first = tmp.join("first.txt");
    let second = tmp.join("second.txt");
    std::fs::write(&first, "first\n").unwrap();
    std::fs::write(&second, "second\n").unwrap();
    let mut editor = Editor::default();
    editor.open_file(first.clone()).unwrap();
    editor.replace_buffer_content("unsaved\n");
    let dirty_index = editor.current_buffer_index();
    editor.open_file(second).unwrap();
    let active = editor.current_buffer_index();
    let panes = editor.panes().len();
    feed(&mut editor, "<C-c>");
    assert!(!editor.should_quit);
    assert_eq!(editor.current_buffer_index(), active);
    assert_eq!(editor.panes().len(), panes);
    assert_eq!(
        editor.buffer_at(dirty_index).unwrap().content(),
        "unsaved\n"
    );
    assert!(editor.buffer_at(dirty_index).unwrap().dirty);
    assert_eq!(std::fs::read_to_string(first).unwrap(), "first\n");
    assert!(editor.status_message.as_deref().unwrap().contains(":qa!"));
    std::fs::remove_dir_all(tmp).unwrap();
}

#[test]
fn insert_ctrl_o_ctrl_c_stays_in_normal_mode() {
    // Vim's Ctrl-C also drops the pending return to Insert (restart_edit)
    // and shows no quit guidance there, whether or not a command was pending.
    for pending in ["", "d", "2"] {
        let mut editor = Editor::default();
        editor.registers.use_in_memory_clipboard_for_tests();
        editor.replace_buffer_content("abc def\n");
        feed(&mut editor, "i<C-o>");
        feed(&mut editor, pending);
        feed(&mut editor, "<C-c>");
        assert!(!editor.should_quit, "{pending:?}");
        assert_eq!(editor.mode, Mode::Normal, "{pending:?}");
        assert!(editor.status_message.is_none(), "{pending:?}");
        feed(&mut editor, "x");
        assert_eq!(editor.buffer().content(), "bc def\n", "{pending:?}");
        assert_eq!(editor.mode, Mode::Normal, "{pending:?}");
    }
}

#[test]
fn ctrl_chords_cancel_leader_sequences_instead_of_completing_mappings() {
    // A Ctrl chord's letter must not complete a mapping: <leader><C-c> would
    // otherwise run <leader>c, and <leader><C-q> would run <leader>q.
    for mode in [Mode::Normal, Mode::Explorer] {
        for chord in ["<C-c>", "<C-q>"] {
            let mut editor = Editor::default();
            editor.replace_buffer_content("abc def\n");
            editor.settings.keymap.leader_mappings = ["c", "q"]
                .into_iter()
                .map(|key| LeaderMapping {
                    key: key.into(),
                    action: ":qa!<CR>".into(),
                    desc: None,
                })
                .collect();
            editor.keymap = KeymapLookup::from_settings(&editor.settings.keymap).0;
            editor.mode = mode;
            editor.explorer.visible = mode == Mode::Explorer;
            feed(&mut editor, "<Space>");
            assert!(editor.leader_sequence.is_some(), "{mode:?} {chord}");
            feed(&mut editor, chord);
            assert!(!editor.should_quit, "{mode:?} {chord}");
            assert!(editor.leader_sequence.is_none(), "{mode:?} {chord}");
            assert_eq!(editor.mode, mode, "{mode:?} {chord}");
        }
    }
}

#[test]
fn ctrl_c_cancels_the_expression_register_prompt() {
    // Normal-mode "= and Insert-mode <C-r>= share one prompt. Ctrl-C closes
    // it without typing a 'c', and the next key acts in the original mode.
    for (open, expected, mode) in [
        ("\"=", "bc def\n", Mode::Normal),
        ("i<C-r>=", "xabc def\n", Mode::Insert),
    ] {
        let mut editor = Editor::default();
        editor.registers.use_in_memory_clipboard_for_tests();
        editor.replace_buffer_content("abc def\n");
        feed(&mut editor, open);
        feed(&mut editor, "<C-c>x");
        assert!(!editor.should_quit, "{open}");
        assert_eq!(editor.buffer().content(), expected, "{open}");
        assert_eq!(editor.mode, mode, "{open}");
    }
}

#[test]
fn altgr_chord_still_extends_a_leader_sequence() {
    // AltGr chars arrive as CONTROL|ALT on some terminals; like Insert mode,
    // they keep typing their character instead of cancelling the sequence.
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc def\n");
    editor.settings.keymap.leader_mappings = vec![LeaderMapping {
        key: "@".into(),
        action: ":qa!<CR>".into(),
        desc: None,
    }];
    editor.keymap = KeymapLookup::from_settings(&editor.settings.keymap).0;
    feed(&mut editor, "<Space>");
    handle_key(
        &mut editor,
        KeyEvent::new(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ),
    );
    assert!(editor.should_quit);
}
