//! Terminal pastes, taken as text and never as keys.
//!
//! Feeding a paste through the key handler let any copied text drive the
//! editor: a pasted `dd` deleted a line and `:!cmd` plus a newline ran a shell
//! command. Every input state below either takes the paste as text or drops
//! it. Nothing here goes through a register or the system clipboard.
//!
//! Normal mode and characterwise Visual mode follow Neovim: the text is put
//! characterwise, even when it ends in a newline, and the cursor ends on the
//! last pasted character, or at the start of the line after the paste when
//! the text ends in a newline.

use super::{Change, Editor, Mode};
use crate::commands::PendingDigraph;
use crate::explorer::ExplorerAction;

impl Editor {
    /// Handle the text of one bracketed paste.
    pub fn paste_text(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if text.is_empty() || self.floating_terminal.is_visible() {
            return;
        }
        self.render_damage.mark_full();
        self.paste_into_current_input(&text);
        self.sync_active_pane_view();
    }

    fn paste_into_current_input(&mut self, text: &str) {
        // Overlays that own the keyboard, in the order `handle_key` checks
        // them. The pickers and labeled jump treat letters as commands.
        if self.markdown_preview.is_some()
            || self.references_picker.is_some()
            || self.code_actions_picker.is_some()
            || self.labeled_jump.is_some()
        {
            return;
        }
        if self.pending_expression_register.is_some() {
            for ch in prompt_line(text).chars() {
                self.push_expression_register_char(ch);
            }
            return;
        }
        if let Some(picker) = self.theme_picker.as_mut() {
            for ch in prompt_line(text).chars() {
                picker.add_char(ch);
            }
            if let Some(name) = picker.selected_name().map(str::to_string) {
                self.preview_theme(&name);
            }
            return;
        }
        // A half-typed leader sequence (and its key popup) has no text input.
        if self.leader_sequence.is_some() {
            return;
        }

        match self.mode {
            Mode::Normal => self.paste_in_normal_mode(text),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
                self.input_state.reset();
                if !self.reject_read_only_edit() {
                    let charwise = self.mode == Mode::Visual;
                    let (start_line, start_col, _, _) = self.get_visual_range();
                    // The replaced text is dropped: a paste never fills a register.
                    let replaced = self.replace_visual_selection_with(text.to_string());
                    // `V` and block Visual keep the cursor where the
                    // replacement puts it. Neovim joins a `V` paste with the
                    // next line, which Nevi does not copy.
                    if charwise && replaced.is_some() {
                        self.move_cursor_to_paste_end(start_line, start_col, text);
                    }
                }
            }
            Mode::Insert => {
                // A pending <C-r> or <C-v> must not take the paste's first
                // character as its argument.
                self.pending_insert_register = false;
                self.pending_insert_literal = false;
                if self.reject_read_only_edit() {
                    return;
                }
                self.completion.hide();
                self.insert_text_at_cursor(text);
                self.dot_repeat.record_paste(text);
            }
            Mode::Replace => {
                if self.reject_read_only_edit() {
                    return;
                }
                for ch in text.chars() {
                    self.replace_mode_char(ch);
                }
                self.dot_repeat.record_paste(text);
            }
            Mode::Command => {
                self.command_line.pending_register = false;
                self.command_line.pending_literal = false;
                self.command_line.pending_digraph = PendingDigraph::None;
                self.command_line.insert_text(&prompt_line(text));
            }
            Mode::Search => {
                self.search.pending_register = false;
                self.search.insert_text(&prompt_line(text));
                self.update_incremental_search();
            }
            Mode::Finder => {
                let line = prompt_line(text);
                if line.is_empty() {
                    return;
                }
                self.finder.insert_text(&line);
                if self.finder.preview_enabled && self.finder.mode_supports_preview() {
                    self.finder.preview_update_pending = true;
                }
                self.ensure_finder_visible_match_indices();
            }
            Mode::Explorer => self.paste_in_explorer(text),
            Mode::RenamePrompt => {
                for ch in prompt_line(text).chars() {
                    self.rename_input_char(ch);
                }
            }
        }
    }

    /// Put the text characterwise after the cursor character, or at column 0
    /// of an empty line, as one undo step. Neovim does the same and cancels a
    /// pending operator or count first, so pasted text can never finish a
    /// command.
    fn paste_in_normal_mode(&mut self, text: &str) {
        // The start screen is not a buffer to edit.
        if self.dashboard_active() {
            return;
        }
        self.input_state.reset();
        let version = self.buffer().version();
        let buffer_idx = self.current_buffer_idx;
        self.dot_repeat.settle_before_paste(version, buffer_idx);

        if !self.reject_read_only_edit() {
            let line = self.cursor.line;
            let col = (self.cursor.col + 1).min(self.buffer().line_len(line));
            self.begin_change();
            self.record_change(Change::insert(line, col, text.to_string()));
            self.buffers[self.current_buffer_idx].insert_str(line, col, text);
            self.move_cursor_to_paste_end(line, col, text);
            self.undo_stack
                .end_undo_group(self.cursor.line, self.cursor.col);
        }

        // After insert-mode <C-o> the paste was the one Normal command.
        if self.pending_insert_normal_once {
            self.finish_insert_normal_once();
        }
    }

    /// Neovim's cursor after a paste put at `line`, `col`: on the last pasted
    /// character, or at the start of the next line when that is a newline.
    fn move_cursor_to_paste_end(&mut self, line: usize, col: usize, text: &str) {
        let (end_line, end_col) = Self::text_position_after_insert(line, col, text);
        self.cursor.line = end_line;
        self.cursor.col = if text.ends_with('\n') {
            end_col
        } else {
            end_col.saturating_sub(1)
        };
        self.clamp_cursor();
        self.scroll_to_cursor();
    }

    fn paste_in_explorer(&mut self, text: &str) {
        let explorer = &mut self.explorer;
        // The `?` card and the delete confirmation read single keys as
        // answers, and the tree reads them as commands.
        if explorer.show_keys
            || explorer.pending_action == Some(ExplorerAction::Delete)
            || self.explorer_key_sequence.is_some()
        {
            return;
        }
        if explorer.is_searching {
            for ch in prompt_line(text).chars() {
                explorer.search_insert(ch);
            }
        } else if explorer.has_pending_action() {
            for ch in prompt_line(text).chars() {
                explorer.input_insert(ch);
            }
        }
    }
}

/// What a one-line prompt takes from a paste: the first line, without
/// control characters, so a pasted newline can never submit the prompt.
pub(crate) fn prompt_line(text: &str) -> String {
    text.split('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_control())
        .collect()
}
