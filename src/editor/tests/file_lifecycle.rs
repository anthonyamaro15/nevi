use crate::editor::Editor;
use crate::terminal::handle_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), nanos))
}

#[test]
fn opening_nonexistent_path_attaches_it_to_the_unnamed_buffer() {
    let temp_dir = unique_temp_dir("nevi_new_file_path");
    std::fs::create_dir_all(&temp_dir).expect("create temp directory");
    let path = temp_dir.join("new.txt");
    assert!(!path.exists(), "fixture path must not exist yet");

    let mut editor = Editor::default();
    editor.open_file(path.clone()).expect("open new file path");

    assert_eq!(editor.buffer().path.as_ref(), Some(&path));
    assert!(editor.buffer().is_file_backed());
    assert_eq!(editor.buffer_count(), 1, "initial unnamed buffer is reused");

    std::fs::remove_dir_all(temp_dir).expect("remove temp directory");
}

#[test]
fn reported_new_file_yypp_zz_workflow_saves_three_terminated_lines() {
    let temp_dir = unique_temp_dir("nevi_issue_238_workflow");
    std::fs::create_dir_all(&temp_dir).expect("create temp directory");
    let path = temp_dir.join("new.txt");

    let mut editor = Editor::default();
    editor.registers.use_in_memory_clipboard_for_tests();
    editor.open_file(path.clone()).expect("open new file path");

    for key in [
        KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT),
    ] {
        handle_key(&mut editor, key);
    }

    assert!(editor.should_quit, "ZZ should save and request quit");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read saved file"),
        "abc\nabc\nabc\n"
    );

    std::fs::remove_dir_all(temp_dir).expect("remove temp directory");
}

#[test]
fn newly_opened_path_saves_without_an_explicit_save_as() {
    let temp_dir = unique_temp_dir("nevi_new_file_save");
    std::fs::create_dir_all(&temp_dir).expect("create temp directory");
    let path = temp_dir.join("new.txt");

    let mut editor = Editor::default();
    editor.open_file(path.clone()).expect("open new file path");
    editor.enter_insert_mode();
    for ch in "abc".chars() {
        editor.insert_char(ch);
    }
    editor.enter_normal_mode();

    editor.save().expect("save to original path");

    assert_eq!(
        std::fs::read_to_string(&path).expect("read saved file"),
        "abc"
    );

    std::fs::remove_dir_all(temp_dir).expect("remove temp directory");
}

#[test]
fn reopening_same_nonexistent_path_reuses_its_buffer() {
    let temp_dir = unique_temp_dir("nevi_new_file_reopen");
    std::fs::create_dir_all(&temp_dir).expect("create temp directory");
    let path = temp_dir.join("new.txt");

    let mut editor = Editor::default();
    editor.open_file(path.clone()).expect("open new file path");
    editor
        .open_file(path.clone())
        .expect("reopen same new file path");

    assert_eq!(editor.buffer_count(), 1);
    assert_eq!(editor.buffer().path.as_ref(), Some(&path));

    std::fs::remove_dir_all(temp_dir).expect("remove temp directory");
}

#[test]
fn distinct_nonexistent_paths_open_in_distinct_buffers() {
    let temp_dir = unique_temp_dir("nevi_new_file_distinct");
    std::fs::create_dir_all(&temp_dir).expect("create temp directory");
    let first = temp_dir.join("first.txt");
    let second = temp_dir.join("second.txt");

    let mut editor = Editor::default();
    editor.open_file(first).expect("open first new file path");
    editor
        .open_file(second.clone())
        .expect("open second new file path");

    assert_eq!(editor.buffer_count(), 2);
    assert_eq!(editor.buffer().path.as_ref(), Some(&second));

    std::fs::remove_dir_all(temp_dir).expect("remove temp directory");
}
