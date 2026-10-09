use super::{SharedOutput, handle_key};
use crate::config::{KeymapEntry, KeymapLookup};
use crate::editor::{Editor, Mode};
use crate::input::key_notation::parse_key_sequence;
use crate::terminal::{SuspendOutcome, Terminal};
use crossterm::{cursor, event, terminal};
use std::io;

fn feed(editor: &mut Editor, keys: &str) {
    for key in parse_key_sequence(keys).unwrap() {
        handle_key(editor, key);
    }
}

fn ansi(command: impl crossterm::Command) -> String {
    let mut out = String::new();
    command.write_ansi(&mut out).unwrap();
    out
}

fn leave_sequences() -> Vec<String> {
    vec![
        ansi(event::DisableMouseCapture),
        ansi(event::DisableFocusChange),
        ansi(event::DisableBracketedPaste),
        ansi(event::PopKeyboardEnhancementFlags),
        ansi(cursor::SetCursorStyle::DefaultUserShape),
        ansi(cursor::Show),
        ansi(terminal::LeaveAlternateScreen),
    ]
}

fn enter_sequences() -> Vec<String> {
    vec![
        ansi(terminal::EnterAlternateScreen),
        ansi(cursor::Hide),
        ansi(event::EnableFocusChange),
        ansi(event::PushKeyboardEnhancementFlags(
            event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
        )),
    ]
}

fn assert_contains_all(output: &str, expected: &[String], what: &str) {
    for sequence in expected {
        assert!(
            output.contains(sequence.as_str()),
            "{what} is missing {sequence:?} in {output:?}"
        );
    }
}

#[test]
fn ctrl_z_in_normal_mode_asks_to_suspend() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc def\n");
    feed(&mut editor, "<C-z>");
    assert!(editor.pending_suspend);
    assert_eq!(editor.mode, Mode::Normal);
    assert_eq!(editor.buffer().content(), "abc def\n");
    assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));
}

#[test]
fn ctrl_z_drops_a_pending_operator_and_count() {
    // Vim's nv_suspend clears the operator before stopping, so `d<C-z>w`
    // moves instead of deleting, and `3<C-z>x` deletes one character.
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc def\n");
    feed(&mut editor, "d<C-z>");
    assert!(editor.pending_suspend);
    editor.pending_suspend = false;
    feed(&mut editor, "w");
    assert_eq!(editor.buffer().content(), "abc def\n");
    assert_eq!(editor.cursor.col, 4);

    feed(&mut editor, "3<C-z>");
    assert!(editor.pending_suspend);
    feed(&mut editor, "x");
    assert_eq!(editor.buffer().content(), "abc ef\n");
}

#[test]
fn ctrl_z_is_an_argument_after_r_and_f_not_a_suspend() {
    // Vim reads the Ctrl-Z after r and f as a literal ^Z, so the suspend
    // check must not run ahead of keys that are waiting for a character.
    for keys in ["r<C-z>", "f<C-z>", "t<C-z>"] {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abc zz\n");
        feed(&mut editor, keys);
        assert!(!editor.pending_suspend, "{keys} should not suspend");
    }
}

#[test]
fn ctrl_z_in_visual_mode_returns_to_normal_and_asks_to_suspend() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc def\n");
    feed(&mut editor, "vl<C-z>");
    assert!(editor.pending_suspend);
    assert_eq!(editor.mode, Mode::Normal);
    assert_eq!(editor.cursor.col, 1);
}

#[test]
fn ctrl_z_in_insert_mode_does_not_suspend() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc\n");
    feed(&mut editor, "i<C-z>");
    assert!(!editor.pending_suspend);
    assert_eq!(editor.mode, Mode::Insert);
}

#[test]
fn ctrl_z_in_the_floating_terminal_does_not_suspend_nevi() {
    // There Ctrl-Z belongs to the shell inside, to stop its job, so the
    // floating terminal has to get it before the Normal-mode suspend does.
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc\n");
    editor.floating_terminal.show_without_shell_for_test();
    feed(&mut editor, "<C-z>");
    assert!(!editor.pending_suspend);
    assert!(editor.floating_terminal.is_visible());
    assert_eq!(editor.buffer().content(), "abc\n");
}

#[test]
fn suspend_and_stop_commands_ask_to_suspend() {
    for command in ["sus", "suspend", "susp", "st", "stop", "sus!", "stop!"] {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abc\n");
        feed(&mut editor, &format!(":{command}<CR>"));
        assert!(editor.pending_suspend, ":{command} should suspend");
        assert_eq!(editor.mode, Mode::Normal);
    }
}

#[test]
fn a_user_mapping_for_ctrl_z_still_wins() {
    let mut editor = Editor::default();
    editor.replace_buffer_content("abc def\n");
    editor.settings.keymap.normal.push(KeymapEntry {
        from: "<C-z>".into(),
        to: "l".into(),
    });
    editor.keymap = KeymapLookup::from_settings(&editor.settings.keymap).0;
    feed(&mut editor, "<C-z>");
    assert!(!editor.pending_suspend);
    assert_eq!(editor.cursor.col, 1);
}

#[test]
fn suspend_hands_the_screen_back_before_stopping_and_takes_it_after() {
    let output = SharedOutput::default();
    let mut terminal = Terminal::new_for_test(Box::new(output.clone()));
    terminal.set_mouse_capture(true).unwrap();
    let before = output.into_string();

    let mut at_stop = String::new();
    let outcome = terminal
        .suspend_with(|| {
            at_stop = output.into_string();
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome, SuspendOutcome::Resumed);

    let left = &at_stop[before.len()..];
    assert_contains_all(left, &leave_sequences(), "the hand-off before stopping");
    assert!(!left.contains(&ansi(terminal::EnterAlternateScreen)));

    let taken_back = &output.into_string()[at_stop.len()..];
    assert_contains_all(
        taken_back,
        &enter_sequences(),
        "the take-back after resuming",
    );
    // Mouse capture comes back with the next render, which re-applies it.
    assert!(!terminal.mouse_capture_enabled);
}

#[test]
fn suspend_takes_the_screen_back_even_if_the_stop_fails() {
    let output = SharedOutput::default();
    let mut terminal = Terminal::new_for_test(Box::new(output.clone()));
    let mut at_stop = 0;
    let result = terminal.suspend_with(|| {
        at_stop = output.into_string().len();
        Err(io::Error::other("stop failed"))
    });
    assert!(result.is_err());
    let taken_back = &output.into_string()[at_stop..];
    assert_contains_all(
        taken_back,
        &enter_sequences(),
        "the take-back after a failed stop",
    );
}

#[test]
fn external_commands_use_the_same_hand_off_as_suspend() {
    let output = SharedOutput::default();
    let mut terminal = Terminal::new_for_test(Box::new(output.clone()));
    terminal.run_external_process("true").unwrap();
    let written = output.into_string();
    let leave_at = written
        .find(&ansi(terminal::LeaveAlternateScreen))
        .expect("leaves the alternate screen");
    let enter_at = written
        .find(&ansi(terminal::EnterAlternateScreen))
        .expect("re-enters the alternate screen");
    assert!(leave_at < enter_at);
    assert_contains_all(&written[..enter_at], &leave_sequences(), "the hand-off");
    assert_contains_all(&written[leave_at..], &enter_sequences(), "the take-back");
}
