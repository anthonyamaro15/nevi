use super::{handle_key, unique_temp_dir};
use crate::config::{KeymapEntry, KeymapLookup, LeaderAction, LeaderMapping};
use crate::editor::{Editor, Mode};
use crate::input::key_notation::parse_key_sequence;
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
