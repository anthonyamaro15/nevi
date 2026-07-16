use super::{Change, Editor, Mode};

#[derive(Debug)]
enum ReplaceEdit {
    Character {
        line: usize,
        col: usize,
        original: Option<char>,
        replacement: char,
    },
    Newline {
        line: usize,
        col: usize,
    },
}

/// Transient state for one interactive `R` session.
#[derive(Debug)]
pub(super) struct ReplaceSession {
    repeat_count: usize,
    inserted_text: String,
    edits: Vec<ReplaceEdit>,
}

impl Default for ReplaceSession {
    fn default() -> Self {
        Self {
            repeat_count: 1,
            inserted_text: String::new(),
            edits: Vec::new(),
        }
    }
}

impl ReplaceSession {
    fn with_count(count: usize) -> Self {
        Self {
            repeat_count: count.max(1),
            ..Self::default()
        }
    }
}

impl Editor {
    /// Enter interactive replace mode, preserving the command coordinate for redo.
    pub fn enter_replace_mode(&mut self, count: usize) {
        let start = (self.cursor.line, self.cursor.col);
        self.begin_change();
        self.undo_stack
            .prefer_current_cursor_after(start.0, start.1);
        self.replace_session = ReplaceSession::with_count(count);
        self.mode = Mode::Replace;
    }

    /// Replace the character under the cursor while remaining in `R` mode.
    pub fn replace_mode_char(&mut self, ch: char) {
        self.apply_replace_mode_char(ch, true);
    }

    fn apply_replace_mode_char(&mut self, ch: char, track_session: bool) {
        if ch == '\n' {
            self.apply_replace_mode_newline(track_session);
            return;
        }

        let line = self.cursor.line;
        let col = self.cursor.col;
        let original = if col < self.buffers[self.current_buffer_idx].line_len(line) {
            self.buffers[self.current_buffer_idx].char_at(line, col)
        } else {
            None
        };

        if let Some(old_char) = original {
            self.undo_stack
                .record_change(Change::delete(line, col, old_char.to_string()));
            self.buffers[self.current_buffer_idx].delete_char(line, col);
        }

        self.undo_stack
            .record_change(Change::insert(line, col, ch.to_string()));
        self.buffers[self.current_buffer_idx].insert_char(line, col, ch);

        if track_session {
            self.replace_session.edits.push(ReplaceEdit::Character {
                line,
                col,
                original,
                replacement: ch,
            });
            self.replace_session.inserted_text.push(ch);
        }

        self.cursor.col += 1;
        self.scroll_to_cursor();
    }

    /// Insert a newline in `R` mode and continue replacing on the new line.
    pub fn replace_mode_newline(&mut self) {
        self.apply_replace_mode_newline(true);
    }

    fn apply_replace_mode_newline(&mut self, track_session: bool) {
        let line = self.cursor.line;
        let col = self.cursor.col;

        self.undo_stack
            .record_change(Change::insert(line, col, "\n".to_string()));
        self.buffers[self.current_buffer_idx].insert_char(line, col, '\n');

        if track_session {
            self.replace_session
                .edits
                .push(ReplaceEdit::Newline { line, col });
            self.replace_session.inserted_text.push('\n');
        }

        self.cursor.line += 1;
        self.cursor.col = 0;
        self.scroll_to_cursor();
    }

    /// Backspace in `R` mode restores text overwritten during this session.
    pub fn replace_mode_backspace(&mut self) {
        let Some(edit) = self.replace_session.edits.pop() else {
            let moved = if self.cursor.col > 0 {
                self.cursor.col -= 1;
                true
            } else if self.cursor.line > 0 {
                self.cursor.line -= 1;
                self.cursor.col = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
                true
            } else {
                false
            };
            if moved {
                self.replace_mode_cursor_moved();
                self.scroll_to_cursor();
            }
            return;
        };
        self.replace_session.inserted_text.pop();

        match edit {
            ReplaceEdit::Character {
                line,
                col,
                original,
                replacement,
            } => {
                self.undo_stack
                    .record_change(Change::delete(line, col, replacement.to_string()));
                self.buffers[self.current_buffer_idx].delete_char(line, col);

                if let Some(old_char) = original {
                    self.undo_stack
                        .record_change(Change::insert(line, col, old_char.to_string()));
                    self.buffers[self.current_buffer_idx].insert_char(line, col, old_char);
                }

                self.cursor.line = line;
                self.cursor.col = col;
            }
            ReplaceEdit::Newline { line, col } => {
                self.undo_stack
                    .record_change(Change::delete(line, col, "\n".to_string()));
                self.buffers[self.current_buffer_idx].delete_char(line, col);
                self.cursor.line = line;
                self.cursor.col = col;
            }
        }

        self.scroll_to_cursor();
    }

    /// Cursor movement ends Vim's straight-line replacement history.
    /// Subsequent Backspace must not restore text from before the move, and a
    /// numeric prefix must not replay text entered after the move.
    pub fn replace_mode_cursor_moved(&mut self) {
        self.replace_session.repeat_count = 1;
        self.replace_session.inserted_text.clear();
        self.replace_session.edits.clear();
    }

    /// Apply the numeric prefix's extra copies before leaving `R` mode.
    pub(super) fn finish_replace_session(&mut self) {
        let repeat_count = self.replace_session.repeat_count;
        let inserted_text = self.replace_session.inserted_text.clone();

        for _ in 1..repeat_count {
            for ch in inserted_text.chars() {
                self.apply_replace_mode_char(ch, false);
            }
        }

        self.replace_session = ReplaceSession::default();
    }

    /// Replace one character under the cursor with the `r` command.
    pub fn replace_char(&mut self, ch: char) {
        self.replace_chars(ch, 1);
    }

    /// Replace exactly `count` characters; Vim leaves the line untouched if they do not fit.
    pub fn replace_chars(&mut self, ch: char, count: usize) {
        let count = count.max(1);
        let line = self.cursor.line;
        let start_col = self.cursor.col;
        let line_len = self.buffers[self.current_buffer_idx].line_len(line);
        let available = line_len.saturating_sub(start_col);
        if available < count {
            return;
        }

        let end_col = start_col + count - 1;
        let old_text = self.get_range_text(line, start_col, line, end_col);
        if old_text.chars().count() != count {
            return;
        }

        let replacement = if ch == '\n' {
            "\n".to_string()
        } else {
            std::iter::repeat(ch).take(count).collect()
        };

        self.begin_change();
        self.undo_stack.prefer_current_cursor_after(line, start_col);
        self.undo_stack
            .record_change(Change::delete(line, start_col, old_text));
        self.undo_stack
            .record_change(Change::insert(line, start_col, replacement.clone()));

        self.buffers[self.current_buffer_idx].delete_range(line, start_col, line, end_col + 1);
        self.buffers[self.current_buffer_idx].insert_str(line, start_col, &replacement);

        if ch == '\n' {
            self.cursor.line += 1;
            self.cursor.col = 0;
        } else {
            self.cursor.col = start_col + count - 1;
        }

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        self.clamp_cursor();
        self.scroll_to_cursor();
    }
}
