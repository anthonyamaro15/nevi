//! Guards for replaying the keys of a user mapping: `[[keymap.normal]]`,
//! `[[keymap.visual]]`, and leader actions written as keys.
//!
//! Mapped keys are fed back through the mode's key handler. Two rules keep
//! that from recursing on the Rust stack until it overflows:
//!
//! - Normal and visual mappings are not applied to keys that come from a
//!   mapping, like Vim's `noremap`. `n` -> `nzz` runs the built-in `n`.
//! - The leader key still works inside a mapping, so one mapping can reach a
//!   leader action and that one can reach another. Nesting is capped, and
//!   hitting the cap stops every level at once.
//!
//! A macro played from a mapping (`@a`) is covered by the first rule too, so
//! its keys are not remapped. Vim would remap them.

use crate::editor::Editor;

/// Mappings allowed inside one another through the leader key. Real configs
/// nest once or twice, and each level re-enters the key handler, so this
/// stays small.
const MAX_REMAP_DEPTH: usize = 8;

#[derive(Debug, Default)]
pub(crate) struct RemapState {
    depth: usize,
    /// The nesting limit was hit. Every level drops the keys it has left.
    aborted: bool,
}

impl RemapState {
    /// The key being handled came from a mapping, not from the user.
    pub(crate) fn replaying(&self) -> bool {
        self.depth > 0
    }

    /// The running mappings were cut short and must stop feeding keys.
    pub(crate) fn aborted(&self) -> bool {
        self.aborted
    }
}

/// Start replaying a mapping's keys. Returns false, and stops the mappings
/// already running, when they are nested too deep. Pair with [`finish`].
pub(crate) fn begin(editor: &mut Editor) -> bool {
    if editor.remap.depth == MAX_REMAP_DEPTH {
        editor.remap.aborted = true;
        return false;
    }
    editor.remap.depth += 1;
    true
}

/// End the replay started by [`begin`].
pub(crate) fn finish(editor: &mut Editor) {
    editor.remap.depth -= 1;
    if editor.remap.depth == 0 && editor.remap.aborted {
        editor.remap.aborted = false;
        // The dropped keys may have left half a command or a leader
        // sequence pending.
        editor.input_state.reset();
        editor.leader_sequence = None;
        editor.leader_sequence_start = None;
        editor.leader_pending_action = None;
        editor.set_error("Recursive mapping stopped: a leader mapping runs itself");
    }
}
