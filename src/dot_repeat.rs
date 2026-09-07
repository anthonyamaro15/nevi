//! Dot repeat (`.`): records the key sequence of the last buffer change and
//! replays it, mirroring Vim's redobuff approach.
//!
//! Capture is behavior-based rather than command-enumerated: every key that
//! reaches normal editing flow joins a candidate sequence, and the candidate
//! is committed as "the last change" once the editor returns to a settled
//! normal-mode state with the buffer version moved (or after an insert-mode
//! session). New editing features therefore become repeatable without
//! touching this module. Sequences that pass through visual, command, or
//! search mode are never committed — Vim repeats visual changes structurally
//! rather than as keys, and that variant is deferred.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::Mode;

#[derive(Debug, Default)]
pub struct DotRepeat {
    /// The committed last change, replayed by `.`.
    redo_keys: Vec<KeyEvent>,
    /// Keys of the sequence currently being built.
    candidate: Vec<KeyEvent>,
    /// Buffer version when the candidate started.
    start_version: u64,
    /// Buffer index when the candidate started; a switch discards the
    /// candidate rather than committing keys that straddle buffers.
    start_buffer_idx: usize,
    /// The candidate ran an insert/replace session. Committed even when the
    /// buffer is unchanged (`i<Esc>`), because Vim records those too and a
    /// stale earlier change must not survive as "the last change".
    ran_insert_session: bool,
    /// The candidate touched a flow `.` never repeats (visual mode, command
    /// line, search, undo/redo).
    poisoned: bool,
    /// True while `.` is replaying; replayed keys are not observed.
    replaying: bool,
}

impl DotRepeat {
    /// Observe one key before it is handled. `mode`, `pending`, `version`,
    /// and `buffer_idx` describe the editor state BEFORE the key runs; a
    /// fresh normal-mode key (no pending sequence) means whatever came
    /// before has fully settled, so the previous candidate is committed or
    /// discarded here rather than through a fragile post-key hook.
    pub fn observe(
        &mut self,
        mode: Mode,
        pending: bool,
        version: u64,
        buffer_idx: usize,
        key: KeyEvent,
    ) {
        if self.replaying {
            return;
        }

        if mode == Mode::Normal && !pending {
            self.settle(version, buffer_idx);
        }

        match mode {
            Mode::Normal | Mode::Insert | Mode::Replace => {}
            _ => self.poisoned = true,
        }
        if matches!(mode, Mode::Insert | Mode::Replace) {
            self.ran_insert_session = true;
        }
        // u / Ctrl-r change the buffer but are never a change for `.`.
        if self.candidate.is_empty() && is_undo_redo(key) {
            self.poisoned = true;
        }

        self.candidate.push(key);
    }

    /// Commit or discard the finished candidate and start a fresh one.
    fn settle(&mut self, version: u64, buffer_idx: usize) {
        let changed = version != self.start_version || self.ran_insert_session;
        let same_buffer = buffer_idx == self.start_buffer_idx;
        if !self.candidate.is_empty() && changed && same_buffer && !self.poisoned {
            self.redo_keys = std::mem::take(&mut self.candidate);
        } else {
            self.candidate.clear();
        }
        self.start_version = version;
        self.start_buffer_idx = buffer_idx;
        self.ran_insert_session = false;
        self.poisoned = false;
    }

    /// The keys `.` should replay. A count replaces the change's original
    /// leading count, as in Vim, and the rewritten sequence becomes the
    /// remembered change so a following plain `.` reuses the new count.
    pub fn take_replay_keys(&mut self, count: Option<usize>) -> Option<Vec<KeyEvent>> {
        if self.redo_keys.is_empty() {
            return None;
        }
        if let Some(count) = count {
            let skip = leading_count_len(&self.redo_keys);
            let mut keys: Vec<KeyEvent> = count
                .to_string()
                .chars()
                .map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
                .collect();
            keys.extend_from_slice(&self.redo_keys[skip..]);
            self.redo_keys = keys;
        }
        Some(self.redo_keys.clone())
    }

    pub fn begin_replay(&mut self) {
        self.replaying = true;
    }

    pub fn end_replay(&mut self) {
        self.replaying = false;
    }

    /// Replace the key just observed with the keys `.` should replay for it.
    /// Vim's redo buffer stores what a key inserted rather than the key when
    /// the result depends on context: insert `<C-e>`/`<C-y>` copy a character
    /// from a neighbouring line, and `.` must re-insert that same character.
    /// An empty `keys` drops the key from the change entirely.
    pub fn replace_last_key(&mut self, keys: &[KeyEvent]) {
        if self.replaying || self.candidate.is_empty() {
            return;
        }
        self.candidate.pop();
        self.candidate.extend_from_slice(keys);
    }

    /// Drop the in-flight candidate. The `.` keystroke (and its count) must
    /// never become the recorded change.
    pub fn abandon_candidate(&mut self) {
        self.candidate.clear();
        self.ran_insert_session = false;
        self.poisoned = false;
    }

    pub fn has_recorded_change(&self) -> bool {
        !self.redo_keys.is_empty()
    }
}

fn is_undo_redo(key: KeyEvent) -> bool {
    matches!(
        (key.modifiers, key.code),
        (KeyModifiers::NONE, KeyCode::Char('u')) | (KeyModifiers::CONTROL, KeyCode::Char('r'))
    )
}

/// Length of the count prefix in a recorded sequence (`3x` → 1). A leading
/// `0` is a motion, never a count.
fn leading_count_len(keys: &[KeyEvent]) -> usize {
    let mut len = 0;
    for key in keys {
        match key.code {
            KeyCode::Char(c)
                if key.modifiers == KeyModifiers::NONE
                    && c.is_ascii_digit()
                    && !(len == 0 && c == '0') =>
            {
                len += 1;
            }
            _ => break,
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }

    fn chars(keys: &[KeyEvent]) -> String {
        keys.iter()
            .map(|k| match k.code {
                KeyCode::Char(c) => c,
                KeyCode::Esc => '␛',
                _ => '?',
            })
            .collect()
    }

    #[test]
    fn change_commits_when_next_key_arrives() {
        let mut dot = DotRepeat::default();
        // `x` changed the buffer (v1 -> v2); the following `j` settles it.
        dot.observe(Mode::Normal, false, 1, 0, key('x'));
        dot.observe(Mode::Normal, false, 2, 0, key('j'));
        assert_eq!(chars(&dot.take_replay_keys(None).unwrap()), "x");
    }

    #[test]
    fn motion_never_becomes_the_change() {
        let mut dot = DotRepeat::default();
        dot.observe(Mode::Normal, false, 1, 0, key('x'));
        dot.observe(Mode::Normal, false, 2, 0, key('w'));
        // `w` did not change the version; the next key discards it and the
        // committed change stays `x`.
        dot.observe(Mode::Normal, false, 2, 0, key('j'));
        assert_eq!(chars(&dot.take_replay_keys(None).unwrap()), "x");
    }

    #[test]
    fn operator_motion_commits_as_one_sequence() {
        let mut dot = DotRepeat::default();
        dot.observe(Mode::Normal, false, 1, 0, key('d'));
        // Mid-sequence: pending operator, no settle.
        dot.observe(Mode::Normal, true, 1, 0, key('w'));
        dot.observe(Mode::Normal, false, 2, 0, key('j'));
        assert_eq!(chars(&dot.take_replay_keys(None).unwrap()), "dw");
    }

    #[test]
    fn insert_session_commits_with_typed_text() {
        let mut dot = DotRepeat::default();
        dot.observe(Mode::Normal, false, 1, 0, key('i'));
        dot.observe(Mode::Insert, false, 1, 0, key('h'));
        dot.observe(Mode::Insert, false, 2, 0, key('i'));
        dot.observe(Mode::Insert, false, 3, 0, esc());
        dot.observe(Mode::Normal, false, 3, 0, key('j'));
        assert_eq!(chars(&dot.take_replay_keys(None).unwrap()), "ihi␛");
    }

    #[test]
    fn replaced_key_is_what_gets_replayed() {
        let mut dot = DotRepeat::default();
        let ctrl_e = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL);
        dot.observe(Mode::Normal, false, 1, 0, key('i'));
        // Insert <C-e> copied a `c`; Vim redoes the `c`, not the key.
        dot.observe(Mode::Insert, false, 1, 0, ctrl_e);
        dot.replace_last_key(&[key('c')]);
        // A second press copied nothing and must vanish from the change.
        dot.observe(Mode::Insert, false, 2, 0, ctrl_e);
        dot.replace_last_key(&[]);
        dot.observe(Mode::Insert, false, 2, 0, esc());
        dot.observe(Mode::Normal, false, 2, 0, key('j'));
        assert_eq!(chars(&dot.take_replay_keys(None).unwrap()), "ic␛");
    }

    #[test]
    fn empty_insert_session_still_replaces_the_change() {
        let mut dot = DotRepeat::default();
        dot.observe(Mode::Normal, false, 1, 0, key('x'));
        // i<Esc> typed nothing, but Vim still records it as the last change.
        dot.observe(Mode::Normal, false, 2, 0, key('i'));
        dot.observe(Mode::Insert, false, 2, 0, esc());
        dot.observe(Mode::Normal, false, 2, 0, key('j'));
        assert_eq!(chars(&dot.take_replay_keys(None).unwrap()), "i␛");
    }

    #[test]
    fn visual_change_is_not_captured() {
        let mut dot = DotRepeat::default();
        dot.observe(Mode::Normal, false, 1, 0, key('x'));
        dot.observe(Mode::Normal, false, 2, 0, key('v'));
        dot.observe(Mode::Visual, false, 2, 0, key('l'));
        dot.observe(Mode::Visual, false, 2, 0, key('d'));
        // Version moved, but the sequence went through visual mode.
        dot.observe(Mode::Normal, false, 3, 0, key('j'));
        assert_eq!(chars(&dot.take_replay_keys(None).unwrap()), "x");
    }

    #[test]
    fn undo_is_never_the_change() {
        let mut dot = DotRepeat::default();
        dot.observe(Mode::Normal, false, 1, 0, key('x'));
        dot.observe(Mode::Normal, false, 2, 0, key('u'));
        dot.observe(Mode::Normal, false, 1, 0, key('j'));
        assert_eq!(chars(&dot.take_replay_keys(None).unwrap()), "x");
    }

    #[test]
    fn buffer_switch_discards_the_candidate() {
        let mut dot = DotRepeat::default();
        dot.observe(Mode::Normal, false, 1, 0, key('x'));
        // Buffer 1 happens to share the bumped version number.
        dot.observe(Mode::Normal, false, 2, 1, key('j'));
        assert!(dot.take_replay_keys(None).is_none());
    }

    #[test]
    fn count_override_rewrites_the_leading_count() {
        let mut dot = DotRepeat::default();
        dot.observe(Mode::Normal, false, 1, 0, key('2'));
        dot.observe(Mode::Normal, true, 1, 0, key('x'));
        dot.observe(Mode::Normal, false, 2, 0, key('j'));
        assert_eq!(chars(&dot.take_replay_keys(Some(5)).unwrap()), "5x");
        // The rewritten count is remembered for the next plain repeat.
        assert_eq!(chars(&dot.take_replay_keys(None).unwrap()), "5x");
    }

    #[test]
    fn zero_is_a_motion_not_a_count_prefix() {
        assert_eq!(leading_count_len(&[key('0'), key('x')]), 0);
        assert_eq!(leading_count_len(&[key('1'), key('0'), key('x')]), 2);
    }

    #[test]
    fn replaying_keys_are_not_observed() {
        let mut dot = DotRepeat::default();
        dot.observe(Mode::Normal, false, 1, 0, key('x'));
        dot.observe(Mode::Normal, false, 2, 0, key('j'));
        dot.begin_replay();
        dot.observe(Mode::Normal, false, 2, 0, key('d'));
        dot.observe(Mode::Normal, true, 2, 0, key('d'));
        dot.end_replay();
        assert_eq!(chars(&dot.take_replay_keys(None).unwrap()), "x");
    }
}
