//! Terminals with no size or almost none. A pty whose size was never set
//! says 0x0, which is what a script or test harness gets when it forgets to
//! set one.

use super::{handle_key, render_editor_to_string};
use crate::editor::Editor;
use crate::input::key_notation::parse_key_sequence;

fn feed(editor: &mut Editor, keys: &str) {
    for key in parse_key_sequence(keys).unwrap() {
        handle_key(editor, key);
    }
}

fn editor_sized(width: u16, height: u16) -> Editor {
    let mut editor = Editor::default();
    editor.replace_buffer_content("one\ntwo\n");
    editor.set_size(width, height);
    editor
}

#[test]
fn a_missing_dimension_falls_back_to_80_by_24() {
    for ((width, height), expected) in [
        ((0, 0), (80, 24)),
        ((0, 10), (80, 10)),
        ((100, 0), (100, 24)),
        ((100, 30), (100, 30)),
    ] {
        let editor = editor_sized(width, height);
        assert_eq!(
            (editor.term_width, editor.term_height),
            expected,
            "{width}x{height}"
        );
    }
}

#[test]
fn every_mode_renders_when_the_terminal_reports_no_size() {
    // The command line and the search prompt used to put the cursor on row
    // `height - 1`, which overflowed at height 0 and aborted the editor.
    for keys in ["", ":", ":set ", "/one", "ihello", "v", "<Space>"] {
        let mut editor = editor_sized(0, 0);
        feed(&mut editor, keys);
        let frame = render_editor_to_string(&editor);
        assert!(!frame.is_empty(), "nothing drawn after {keys:?}");
    }
}

#[test]
fn the_theme_picker_draws_nothing_in_a_window_too_narrow_for_it() {
    // Its frame math subtracted the title and borders from the width
    // without checking, which aborted the editor below 12 columns.
    for (width, height) in [
        (1, 24),
        (5, 24),
        (11, 24),
        (13, 24),
        (14, 24),
        (80, 1),
        (80, 3),
    ] {
        let mut editor = editor_sized(width, height);
        feed(&mut editor, ":Themes<CR>");
        assert!(
            editor.theme_picker.is_some(),
            "picker open at {width}x{height}"
        );
        render_editor_to_string(&editor);
    }
}
