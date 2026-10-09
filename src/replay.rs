//! Key playback for macros (`@`) and dot repeat (`.`).
//!
//! Both feed recorded keys back through `handle_key`, so a macro that calls
//! itself recurses on the Rust stack. Vim replays through a typeahead queue
//! instead and stops a macro at the first failing command; Nevi has neither
//! yet, so a nesting limit keeps the stack safe.

use crossterm::event::KeyEvent;

use crate::editor::{Editor, Mode};
use crate::terminal::handle_key;

/// Playbacks allowed inside one another. Each level re-enters the whole key
/// dispatcher, so this stays far below what a 2 MiB test thread holds in a
/// debug build. Counts (`100@a`) repeat at the same level and are not
/// limited by it.
const MAX_REPLAY_DEPTH: usize = 32;

#[derive(Debug, Default)]
pub(crate) struct ReplayState {
    depth: usize,
    /// Some playback hit the limit. Every level stops at once, including
    /// the remaining keys and counts of its callers, so branching recursion
    /// cannot keep the editor busy for long.
    aborted: bool,
}

/// Repeat the last change (`.`) by replaying its keys, like Vim's redobuff.
pub(crate) fn repeat_last_change(editor: &mut Editor, count: Option<usize>) {
    // The `.` keystroke (and its count) is sitting in the capture candidate;
    // it must never become the recorded change itself.
    editor.dot_repeat.abandon_candidate();

    let Some(keys) = editor.dot_repeat.take_replay_keys(count) else {
        editor.set_status("No change to repeat");
        return;
    };

    editor.dot_repeat.begin_replay();
    replay_keys(editor, &keys, 1);
    editor.dot_repeat.end_replay();
}

/// Play a macro from a register.
pub(crate) fn play_macro(editor: &mut Editor, register: char, count: usize) {
    let Some(keys) = editor.macros.get_macro(register).cloned() else {
        editor.set_status(&format!("Macro @{} not recorded", register));
        return;
    };

    if keys.is_empty() {
        editor.set_status(&format!("Macro @{} is empty", register));
        return;
    }

    // Set this as the last executed macro for @@
    editor.macros.set_last_executed(register);

    replay_keys(editor, &keys, count);
}

/// Run `keys` `count` times as one compound undo group, so a single `u`
/// reverts the whole playback like Vim.
fn replay_keys(editor: &mut Editor, keys: &[KeyEvent], count: usize) {
    if editor.replay.depth == MAX_REPLAY_DEPTH {
        editor.replay.aborted = true;
        return;
    }

    let undo_entries = editor.undo_stack.undo_count();
    let buffer_idx = editor.current_buffer_index();
    editor.replay.depth += 1;
    editor
        .undo_stack
        .begin_compound_group(editor.cursor.line, editor.cursor.col);

    'playback: for _ in 0..count {
        for &key in keys {
            handle_key(editor, key);
            if editor.replay.aborted {
                break 'playback;
            }
        }
    }

    editor.replay.depth -= 1;
    let stopped = editor.replay.aborted && editor.replay.depth == 0;
    if stopped {
        // Clean up once every level has unwound. On the way out each Ctrl-O
        // level may already have resumed Insert mode.
        editor.pending_insert_normal_once = false;
        editor.input_state.reset();
        if matches!(editor.mode, Mode::Insert | Mode::Replace) {
            editor.enter_normal_mode();
        }
        editor.dot_repeat.abandon_candidate();
    }
    editor
        .undo_stack
        .end_compound_group(editor.cursor.line, editor.cursor.col);

    if stopped {
        editor.replay.aborted = false;
        // Nothing tells a recursive macro where it should have stopped (a
        // failed motion does not end playback yet, #372), so its edits are
        // rolled back. A playback that changed nothing pushed no undo entry,
        // and undoing then would revert the user's previous change instead.
        let rolled_back = editor.current_buffer_index() == buffer_idx
            && editor.undo_stack.undo_count() > undo_entries;
        if rolled_back {
            editor.undo();
        }
        editor.clamp_cursor();
        editor.scroll_to_cursor();
        editor.set_status(if rolled_back {
            format!("Macro nesting limit ({MAX_REPLAY_DEPTH}) reached, changes undone")
        } else {
            format!("Macro nesting limit ({MAX_REPLAY_DEPTH}) reached")
        });
    }
}
