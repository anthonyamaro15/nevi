use super::{execute_command, handle_key, key, unique_temp_dir};
use crate::commands::parse_command;
use crate::editor::Editor;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    // Keep paths relative to the current directory so ex commands can exercise
    // relative as well as absolute paths without changing the process cwd.
    let name = unique_temp_dir("nevi_split_buffers")
        .file_name()
        .expect("fixture name")
        .to_owned();
    let dir = PathBuf::from("target").join(name);
    std::fs::create_dir_all(&dir).expect("create fixture directory");
    dir
}

#[test]
fn named_splits_reuse_open_buffer_and_share_edits() {
    let dir = fixture_dir();
    let relative_path = dir.join("open.txt");
    std::fs::write(&relative_path, "original\n").expect("write fixture");
    let absolute_path = relative_path.canonicalize().expect("absolute path");

    for command in ["vs", "sp"] {
        let mut editor = Editor::default();
        editor.open_file(absolute_path.clone()).expect("open file");
        let buffer_idx = editor.current_buffer_index();
        let count = editor.buffer_count();

        execute_command(
            &mut editor,
            parse_command(&format!("{command} {}", relative_path.display())),
        );
        assert_eq!(
            editor.buffer_count(),
            count,
            "{command} created a duplicate buffer"
        );
        assert_eq!(editor.current_buffer_index(), buffer_idx);

        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('x'));
        handle_key(&mut editor, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        editor.focus_pane(0);
        assert_eq!(editor.buffer().content(), "xoriginal\n");
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn named_split_reuses_a_buffer_that_is_not_active_and_opens_new_files() {
    let dir = fixture_dir();
    let first = dir.join("first.txt");
    let second = dir.join("second.txt");
    let third = dir.join("third.txt");
    for path in [&first, &second, &third] {
        std::fs::write(path, "content\n").expect("write fixture");
    }
    let mut editor = Editor::default();
    editor.open_file(first.clone()).expect("open first file");
    let first_idx = editor.current_buffer_index();
    editor.open_file(second).expect("open second file");
    let count = editor.buffer_count();

    execute_command(
        &mut editor,
        parse_command(&format!("vs {}", first.display())),
    );
    assert_eq!(editor.current_buffer_index(), first_idx);
    assert_eq!(editor.buffer_count(), count);
    execute_command(
        &mut editor,
        parse_command(&format!("sp {}", third.display())),
    );
    assert_eq!(editor.buffer_count(), count + 1);

    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn named_split_reuses_buffer_through_symlink() {
    let dir = fixture_dir();
    let path = dir.join("file.txt");
    let link = dir.join("link.txt");
    std::fs::write(&path, "content\n").expect("write fixture");
    std::os::unix::fs::symlink("file.txt", &link).expect("create symlink");
    let mut editor = Editor::default();
    editor.open_file(path).expect("open file");
    let count = editor.buffer_count();

    execute_command(
        &mut editor,
        parse_command(&format!("vs {}", link.display())),
    );
    assert_eq!(editor.buffer_count(), count);

    let _ = std::fs::remove_dir_all(dir);
}
