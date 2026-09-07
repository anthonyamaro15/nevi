use crate::editor::{Editor, RegisterContent};
use crate::input::key_notation::parse_key_sequence;
use crate::shada::{MarkEntry, RegisterEntry, ShadaState};
use std::path::PathBuf;

fn populated_editor() -> Editor {
    let mut editor = Editor::default();
    editor.registers.use_in_memory_clipboard_for_tests();
    editor
        .macros
        .set_macro('a', parse_key_sequence("0f,ci\"hi<Esc>").unwrap());
    editor.registers.set(
        Some('r'),
        RegisterContent::Lines("a full line\n".to_string()),
    );
    // A linewise delete shifts into "1 through the real delete path.
    editor.registers.delete(
        None,
        RegisterContent::Lines("removed line\n".to_string()),
        false,
    );
    editor
        .registers
        .set(None, RegisterContent::Chars("word".to_string()));
    editor
        .marks
        .set_global('A', PathBuf::from("/tmp/lib.rs"), 12, 4);
    editor.search.history = vec!["needle".to_string(), "haystack".to_string()];
    editor
        .jump_list
        .record(Some(PathBuf::from("/tmp/old.rs")), 8, 2);
    editor
        .jump_list
        .record(Some(PathBuf::from("/tmp/new.rs")), 30, 0);
    // Scratch-buffer jumps have no path and must stay session-only.
    editor.jump_list.record(None, 1, 0);
    editor
}

#[test]
fn shada_roundtrips_through_a_fresh_editor() {
    let exported = populated_editor().export_shada();

    let mut restored = Editor::default();
    restored.apply_shada(exported);

    assert_eq!(
        restored.macros.get_macro('a'),
        Some(&parse_key_sequence("0f,ci\"hi<Esc>").unwrap()),
        "macros survive via notation"
    );

    let register = restored.registers.get(Some('r')).expect("register r");
    assert_eq!(register.as_str(), "a full line\n");
    assert!(register.is_linewise(), "linewise-ness must survive");
    assert_eq!(
        restored.registers.get(None).map(|c| c.as_str()),
        Some("word"),
        "unnamed register survives"
    );

    let mark = restored.marks.get_global('A').expect("global mark A");
    assert_eq!(
        mark.path.as_deref(),
        Some(std::path::Path::new("/tmp/lib.rs"))
    );
    assert_eq!((mark.line, mark.col), (12, 4));

    assert_eq!(
        restored.search.history,
        vec!["needle".to_string(), "haystack".to_string()]
    );

    assert_eq!(
        restored.registers.get(Some('1')).map(|c| c.as_str()),
        Some("removed line\n"),
        "delete-history register \"1 survives"
    );

    let jumps = restored.jump_list.entries();
    assert_eq!(jumps.len(), 2, "scratch-buffer jumps stay session-only");
    assert_eq!(
        jumps[1].path.as_deref(),
        Some(std::path::Path::new("/tmp/new.rs"))
    );
    assert_eq!((jumps[1].line, jumps[1].col), (30, 0));
}

#[test]
fn restored_jumplist_walks_back_from_the_end() {
    let exported = populated_editor().export_shada();

    let mut editor = Editor::default();
    editor.apply_shada(exported);

    // The restored list is positioned at its end, so the first Ctrl+o lands
    // on the newest persisted jump, exactly as if the session had continued.
    let loc = editor
        .jump_list
        .go_back(None, 0, 0)
        .cloned()
        .expect("restored history to walk back into");
    assert_eq!(
        loc.path.as_deref(),
        Some(std::path::Path::new("/tmp/new.rs"))
    );
    assert_eq!((loc.line, loc.col), (30, 0));
}

#[test]
fn unencodable_macro_stays_session_only_without_losing_others() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut editor = populated_editor();
    editor
        .macros
        .set_macro('q', vec![KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)]);

    let exported = editor.export_shada();

    assert!(!exported.macros.contains_key(&'q'));
    assert!(exported.macros.contains_key(&'a'));
}

#[test]
fn apply_skips_unparseable_macro_notation() {
    let mut state = ShadaState::default();
    state.macros.insert('a', "d<Esc".to_string());
    state.macros.insert('b', "dw".to_string());

    let mut editor = Editor::default();
    editor.apply_shada(state);

    assert_eq!(editor.macros.get_macro('a'), None);
    assert_eq!(
        editor.macros.get_macro('b'),
        Some(&parse_key_sequence("dw").unwrap())
    );
}

#[test]
fn applied_registers_are_usable_and_marks_jumpable() {
    let mut state = ShadaState::default();
    state.registers.insert(
        'x',
        RegisterEntry {
            text: "restored".to_string(),
            linewise: false,
        },
    );
    state.global_marks.insert(
        'B',
        MarkEntry {
            path: PathBuf::from("/tmp/other.rs"),
            line: 3,
            col: 0,
        },
    );

    let mut editor = Editor::default();
    editor.apply_shada(state);

    assert_eq!(
        editor.registers.get(Some('x')).map(|c| c.as_str()),
        Some("restored")
    );
    assert!(editor.marks.get_global('B').is_some());
}

#[test]
fn export_after_apply_is_stable() {
    let first = populated_editor().export_shada();

    let mut editor = Editor::default();
    editor.apply_shada(first.clone());
    let second = editor.export_shada();

    assert_eq!(first, second, "load→save must not mutate state");
}
