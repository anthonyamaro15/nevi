//! Mouse gestures use the same Visual selection and pane coordinates as the keyboard.

use super::{Cursor, DisplayLineSegment, Editor, Mode, last_addressable_line};

#[derive(Default)]
pub(super) struct MouseState {
    drag: Option<MouseDrag>,
    // Insert/Replace mouse selection temporarily enters Visual mode. Resume
    // typing once its operator or Escape completes, including after release.
    resume_mode: Option<(usize, Mode)>,
}

#[derive(Clone, Copy)]
struct MouseDrag {
    pane: usize,
    buffer: usize,
    version: u64,
    anchor: Cursor,
    press_cell: (u16, u16),
    started: bool,
}

impl Editor {
    pub(crate) fn begin_mouse_drag(&mut self, col: u16, row: u16) {
        if matches!(self.mode, Mode::Normal | Mode::Insert | Mode::Replace) {
            self.mouse.drag = Some(MouseDrag {
                pane: self.active_pane,
                buffer: self.current_buffer_idx,
                version: self.buffer().version(),
                anchor: self.cursor,
                press_cell: (col, row),
                started: false,
            });
        }
    }

    pub(crate) fn has_mouse_drag(&self) -> bool {
        self.mouse.drag.is_some()
    }

    pub(crate) fn cancel_mouse_drag(&mut self) {
        self.mouse.drag = None;
    }

    pub(crate) fn extend_mouse_drag(&mut self, screen_col: u16, screen_row: u16) -> bool {
        let Some(mut drag) = self.mouse.drag else {
            return false;
        };
        if drag.pane != self.active_pane
            || drag.buffer != self.current_buffer_idx
            || drag.version != self.buffer().version()
            || !matches!(
                self.mode,
                Mode::Normal | Mode::Insert | Mode::Replace | Mode::Visual
            )
        {
            self.cancel_mouse_drag();
            return false;
        }
        if !drag.started && drag.press_cell == (screen_col, screen_row) {
            return false;
        }
        let Some(pane) = self.panes.get(drag.pane) else {
            return false;
        };
        let rect = pane.rect;
        if rect.width == 0 || rect.height == 0 {
            self.cancel_mouse_drag();
            return false;
        }
        // A drag belongs to its original window, even across a split or
        // sidebar boundary. Releasing outside must still end the gesture.
        let col = screen_col.clamp(rect.x, rect.x.saturating_add(rect.width - 1));
        let row = screen_row.clamp(rect.y, rect.y.saturating_add(rect.height - 1));
        let Some((line, col)) = self.position_for_mouse(drag.pane, col, row, true) else {
            return false;
        };
        let endpoint = Cursor { line, col };
        if !drag.started && endpoint == drag.anchor {
            return false;
        }
        let changed = !drag.started || self.cursor != endpoint;
        if !drag.started {
            if matches!(self.mode, Mode::Insert | Mode::Replace) {
                self.mouse.resume_mode = Some((drag.buffer, self.mode));
                self.enter_normal_mode();
            }
            self.pending_insert_normal_once = false;
            self.input_state.reset();
            self.pending_insert_register = false;
            self.pending_insert_literal = false;
            self.cursor = drag.anchor;
            self.enter_visual_mode();
            drag.started = true;
            self.mouse.drag = Some(drag);
        }
        self.cursor = endpoint;
        self.desired_col = None;
        self.sync_active_pane_view();
        changed
    }

    pub(crate) fn end_mouse_drag(&mut self, col: u16, row: u16) -> bool {
        let changed = self.extend_mouse_drag(col, row);
        self.cancel_mouse_drag();
        changed
    }

    pub(crate) fn finish_mouse_selection(&mut self) {
        let Some((buffer, mode)) = self.mouse.resume_mode else {
            return;
        };
        if buffer != self.current_buffer_idx {
            self.mouse.resume_mode = None;
            return;
        }
        match self.mode {
            Mode::Normal => {
                self.mouse.resume_mode = None;
                if mode == Mode::Replace {
                    self.enter_replace_mode(1);
                } else {
                    self.enter_insert_mode();
                }
            }
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock | Mode::Command | Mode::Search => {}
            _ => self.mouse.resume_mode = None,
        }
    }

    /// Left click inside a pane: focus it and move the cursor to the clicked
    /// cell, like nvim with 'mouse' enabled. A plain click drops visual mode;
    /// insert mode stays insert. The view never scrolls (mouse positioning
    /// ignores 'scrolloff'), so only the cursor and pane mirror change.
    pub fn click_at(&mut self, pane_idx: usize, screen_col: u16, screen_row: u16) {
        if self.mode == Mode::Normal && self.pending_insert_normal_once {
            self.pending_insert_normal_once = false;
            self.enter_insert_mode();
        }
        // Mouse positioning ends counted/block insertion at the current
        // edit. Replaying it later would insert text at the clicked location.
        if self.mode == Mode::Insert {
            self.insert_session_repeat_count = 1;
            self.pending_visual_block_edit = None;
        } else if self.mode == Mode::Replace {
            self.replace_mode_cursor_moved();
        }
        if matches!(
            self.mode,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        ) {
            self.exit_visual_mode();
            self.finish_mouse_selection();
        }
        self.focus_pane(pane_idx);
        self.unfocus_explorer();
        if let Some((line, col)) = self.position_for_mouse(pane_idx, screen_col, screen_row, false)
        {
            self.cursor.line = line;
            self.cursor.col = col;
            self.desired_col = None;
            if self.active_pane < self.panes.len() {
                self.panes[self.active_pane].cursor = self.cursor;
                self.panes[self.active_pane].desired_col = None;
            }
        }
    }

    /// Map a screen cell inside a pane to a buffer (line, col), mirroring
    /// what rendering shows: gutter, h_offset (nowrap) or wrapped display
    /// segments (wrap). Clicks in the gutter map to the row's first column;
    /// clicks past the end of text clamp like vim's normal-mode cursor.
    fn position_for_mouse(
        &self,
        pane_idx: usize,
        screen_col: u16,
        screen_row: u16,
        allow_eol: bool,
    ) -> Option<(usize, usize)> {
        let pane = self.panes.get(pane_idx)?;
        if !pane.rect.contains(screen_col, screen_row) {
            return None;
        }
        let buffer = self.buffers.get(pane.buffer_idx)?;
        let text_width = self.pane_text_area_width(pane_idx);
        let gutter = (pane.rect.width as usize).saturating_sub(text_width);
        let cell = ((screen_col - pane.rect.x) as usize).saturating_sub(gutter);
        let row = (screen_row - pane.rect.y) as usize;
        let last_line = last_addressable_line(buffer);
        let tab_width = self.get_effective_tab_width();

        let line_text = |line: usize| {
            let mut text = buffer.line(line).map(|l| l.to_string()).unwrap_or_default();
            // Remove Ropey line endings before mapping display cells.
            while text.ends_with(['\n', '\r']) {
                text.pop();
            }
            text
        };

        let position_in_segment = |text: &str, segment: DisplayLineSegment| {
            // Visual dragging can include the line break; an ordinary click
            // still stops on the last character. Wrapped continuations must
            // reach the final segment before they can select that break.
            if allow_eol
                && segment.end_col == text.chars().count()
                && cell
                    >= segment.indent_width
                        + Self::display_width_between_cols(
                            text,
                            segment.start_col,
                            segment.end_col,
                            tab_width,
                        )
            {
                segment.end_col
            } else {
                Self::display_col_to_buffer_col(text, segment, cell, tab_width)
            }
        };

        if self.settings.editor.wrap {
            let wrap_width = self.settings.editor.wrap_width.min(text_width);
            let mut line = pane.viewport_offset.min(last_line);
            let mut rows_left = row;
            loop {
                let text = line_text(line);
                let segments = Self::display_line_segments(&text, wrap_width, tab_width);
                if rows_left < segments.len() {
                    let col = position_in_segment(&text, segments[rows_left]);
                    return Some((line, col));
                }
                if line == last_line {
                    // Clicked below the end of the buffer: last segment.
                    let segment = *segments.last()?;
                    let col = position_in_segment(&text, segment);
                    return Some((line, col));
                }
                rows_left -= segments.len();
                line += 1;
            }
        } else {
            let line = pane.viewport_offset.saturating_add(row).min(last_line);
            let text = line_text(line);
            let len = text.chars().count();
            let segment = DisplayLineSegment {
                start_col: pane.h_offset.min(len),
                end_col: len,
                indent_width: 0,
            };
            let col = position_in_segment(&text, segment);
            Some((line, col))
        }
    }
}
