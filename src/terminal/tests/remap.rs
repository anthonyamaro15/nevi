//! User mappings whose `to` contains their own `from`, or another mapped key.

use super::handle_key;
use crate::config::{KeymapEntry, LeaderMapping, Settings};
use crate::editor::{Editor, Mode};
use crate::input::key_notation::parse_key_sequence;

fn feed(editor: &mut Editor, keys: &str) {
    for key in parse_key_sequence(keys).unwrap() {
        handle_key(editor, key);
    }
}

fn entry(from: &str, to: &str) -> KeymapEntry {
    KeymapEntry {
        from: from.to_string(),
        to: to.to_string(),
    }
}

fn leader(key: &str, action: &str) -> LeaderMapping {
    LeaderMapping {
        key: key.to_string(),
        action: action.to_string(),
        desc: None,
    }
}

fn editor_with(settings: Settings, text: &str) -> Editor {
    let mut editor = Editor::new(settings);
    editor.set_size(80, 24);
    editor.replace_buffer_content(text);
    editor
}

#[test]
fn normal_mapping_that_starts_with_its_own_key_runs_the_builtin() {
    // `n` -> `nzz` is one of the most common vimrc lines. It used to call
    // itself until the stack overflowed.
    let mut settings = Settings::default();
    settings.keymap.normal.push(entry("n", "nzz"));
    let mut editor = editor_with(settings, "foo\nbar\nfoo\nbar\nfoo\n");

    feed(&mut editor, "/foo<CR>");
    assert_eq!(editor.cursor.line, 2);
    feed(&mut editor, "n");

    assert_eq!(editor.cursor.line, 4);
    assert_eq!(editor.mode, Mode::Normal);
}

#[test]
fn normal_mapping_with_its_own_key_later_in_the_sequence_runs_the_builtin() {
    let mut settings = Settings::default();
    settings.keymap.normal.push(entry("x", "\"_x"));
    let mut editor = editor_with(settings, "abc\n");

    feed(&mut editor, "x");

    assert_eq!(editor.buffer().content(), "bc\n");
}

#[test]
fn visual_mapping_that_starts_with_its_own_key_runs_the_builtin() {
    let mut settings = Settings::default();
    settings.keymap.visual.push(entry(">", ">gv"));
    let mut editor = editor_with(settings, "one\ntwo\n");

    feed(&mut editor, "V>");

    // Indented once. The `gv` is dropped because a visual mapping stops
    // feeding keys once Visual mode ends, which is a separate limitation.
    assert_eq!(editor.buffer().content(), "    one\ntwo\n");
}

#[test]
fn mapped_keys_are_not_mapped_again() {
    // `a` runs the built-in `b` (back a word), not the mapping on `b`.
    let mut settings = Settings::default();
    settings.keymap.normal.push(entry("a", "b"));
    settings.keymap.normal.push(entry("b", "x"));
    let mut editor = editor_with(settings, "one two\n");

    feed(&mut editor, "$a");

    assert_eq!(editor.buffer().content(), "one two\n");
    assert_eq!(editor.cursor.col, 4);
}

#[test]
fn a_mapping_can_still_reach_a_leader_mapping() {
    let mut settings = Settings::default();
    settings.keymap.normal.push(entry("Q", "<Space>z"));
    settings.keymap.leader_mappings.push(leader("z", "x"));
    let mut editor = editor_with(settings, "abc\n");

    feed(&mut editor, "Q");

    assert_eq!(editor.buffer().content(), "bc\n");
}

#[test]
fn leader_mapping_that_runs_itself_stops_without_running_its_other_keys() {
    let mut settings = Settings::default();
    settings
        .keymap
        .leader_mappings
        .push(leader("z", "<Space>zx"));
    let mut editor = editor_with(settings, "abcdefghijklmnop\n");

    feed(&mut editor, "<Space>z");

    assert!(!editor.should_quit);
    assert_eq!(editor.mode, Mode::Normal);
    assert_eq!(editor.buffer().content(), "abcdefghijklmnop\n");
    assert!(editor.leader_sequence.is_none());
    assert!(!editor.input_state.has_pending_sequence());
    assert!(
        editor
            .status_message
            .as_deref()
            .is_some_and(|msg| msg.contains("Recursive mapping stopped")),
        "status was {:?}",
        editor.status_message
    );

    // The guard resets, so mappings work again afterwards.
    feed(&mut editor, "x");
    assert_eq!(editor.buffer().content(), "bcdefghijklmnop\n");
}
