use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, ClearType},
    style::{SetForegroundColor, SetBackgroundColor, ResetColor, Color},
};
use std::io::{self, Write, Stdout};

use crate::editor::{Editor, Mode, PaneDirection};
use crate::input::{KeyAction, InsertPosition, Operator, TextObject, TextObjectModifier, TextObjectType};
use crate::commands::{Command, CommandResult, parse_command};
use crate::config::LeaderAction;
use crate::syntax::HighlightSpan;

/// Terminal handler responsible for rendering and input
pub struct Terminal {
    stdout: Stdout,
}

impl Terminal {
    pub fn new() -> anyhow::Result<Self> {
        let mut stdout = io::stdout();

        // Enter raw mode and alternate screen
        terminal::enable_raw_mode()?;
        execute!(
            stdout,
            terminal::EnterAlternateScreen,
            cursor::Hide
        )?;

        Ok(Self { stdout })
    }

    /// Get terminal size
    pub fn size() -> anyhow::Result<(u16, u16)> {
        Ok(terminal::size()?)
    }

    /// Clear the screen
    #[allow(dead_code)]
    pub fn clear(&mut self) -> anyhow::Result<()> {
        execute!(self.stdout, terminal::Clear(ClearType::All))?;
        Ok(())
    }

    /// Run an external process (like lazygit) suspending the editor
    /// The terminal is restored before running and re-initialized after
    pub fn run_external_process(&mut self, command: &str) -> anyhow::Result<()> {
        // Leave alternate screen and show cursor
        execute!(
            self.stdout,
            cursor::Show,
            terminal::LeaveAlternateScreen
        )?;
        self.stdout.flush()?;

        // Disable raw mode so the external process can use normal terminal
        terminal::disable_raw_mode()?;

        // Run the command
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .status();

        // Re-enable raw mode
        terminal::enable_raw_mode()?;

        // Re-enter alternate screen and hide cursor
        execute!(
            self.stdout,
            terminal::EnterAlternateScreen,
            cursor::Hide
        )?;

        // Check if command succeeded
        match status {
            Ok(exit_status) => {
                if !exit_status.success() {
                    // Command failed but we don't treat this as an error for the editor
                }
                Ok(())
            }
            Err(e) => {
                // If we can't run the command, return an error
                Err(anyhow::anyhow!("Failed to run command '{}': {}", command, e))
            }
        }
    }

    /// Render the editor state to the terminal
    pub fn render(&mut self, editor: &Editor) -> anyhow::Result<()> {
        execute!(self.stdout, cursor::MoveTo(0, 0))?;

        let text_rows = editor.text_rows();
        let width = editor.term_width as usize;
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3);

        // Get visual selection range if in visual mode
        let visual_range = if editor.mode.is_visual() {
            Some(editor.get_visual_range())
        } else {
            None
        };

        // Determine line number width based on settings
        let show_line_numbers = editor.settings.editor.line_numbers;
        let show_relative = editor.settings.editor.relative_numbers;
        let highlight_cursor_line = editor.settings.editor.cursor_line;

        // Render each row
        for row in 0..text_rows {
            let file_line = editor.viewport_offset + row;
            let is_cursor_line = file_line == editor.cursor.line;

            // Apply cursor line background if enabled
            if highlight_cursor_line && is_cursor_line && file_line < editor.buffer().len_lines() {
                execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
            }

            if file_line < editor.buffer().len_lines() {
                // Line number (if enabled)
                if show_line_numbers {
                    let line_num = if show_relative {
                        // Relative line numbers: show distance from cursor, current line shows absolute
                        let distance = (file_line as isize - editor.cursor.line as isize).abs() as usize;
                        if distance == 0 {
                            format!("{:>width$} ", file_line + 1, width = line_num_width)
                        } else {
                            format!("{:>width$} ", distance, width = line_num_width)
                        }
                    } else {
                        format!("{:>width$} ", file_line + 1, width = line_num_width)
                    };

                    // Highlight current line number
                    if is_cursor_line {
                        execute!(self.stdout, SetForegroundColor(Color::Yellow))?;
                    } else {
                        execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                    }
                    print!("{}", line_num);
                    execute!(self.stdout, ResetColor)?;

                    // Re-apply cursor line background after reset
                    if highlight_cursor_line && is_cursor_line {
                        execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                    }
                }

                // Line content with syntax highlighting and visual selection
                if let Some(line) = editor.buffer().line(file_line) {
                    let effective_width = if show_line_numbers {
                        width - line_num_width - 1
                    } else {
                        width
                    };
                    let line_str: String = line.chars().take(effective_width).collect();
                    let line_str = line_str.trim_end_matches('\n');

                    // Get syntax highlights for this line
                    let highlights = editor.syntax.get_line_highlights(file_line);

                    self.render_line_with_highlights(
                        line_str,
                        file_line,
                        &highlights,
                        visual_range,
                        &editor.mode,
                        highlight_cursor_line && is_cursor_line,
                    )?;
                }
            } else {
                // Empty line indicator
                execute!(self.stdout, SetForegroundColor(Color::Blue))?;
                if show_line_numbers {
                    print!("{:>width$} ~", "", width = line_num_width);
                } else {
                    print!("~");
                }
                execute!(self.stdout, ResetColor)?;
            }

            // Reset background for cursor line
            if highlight_cursor_line && is_cursor_line {
                execute!(self.stdout, ResetColor)?;
            }

            // Clear to end of line and move to next
            execute!(self.stdout, terminal::Clear(ClearType::UntilNewLine))?;
            if row < text_rows - 1 {
                print!("\r\n");
            }
        }

        // Render status line
        self.render_status_line(editor, line_num_width)?;

        // Render command/message line
        self.render_command_line(editor)?;

        // Render finder if in finder mode
        if editor.mode == Mode::Finder {
            self.render_finder(editor)?;
        }

        // Position cursor based on mode
        match editor.mode {
            Mode::Command => {
                // Cursor in command line
                let cmd_cursor_col = 1 + editor.command_line.cursor; // +1 for ':'
                execute!(
                    self.stdout,
                    cursor::MoveTo(cmd_cursor_col as u16, editor.term_height - 1),
                    cursor::Show,
                    cursor::SetCursorStyle::BlinkingBar
                )?;
            }
            Mode::Search => {
                // Cursor in search line
                let search_cursor_col = 1 + editor.search.cursor; // +1 for '/' or '?'
                execute!(
                    self.stdout,
                    cursor::MoveTo(search_cursor_col as u16, editor.term_height - 1),
                    cursor::Show,
                    cursor::SetCursorStyle::BlinkingBar
                )?;
            }
            Mode::Finder => {
                // Cursor in finder input
                let win = crate::finder::FloatingWindow::centered(editor.term_width, editor.term_height);
                let cursor_x = win.x + 2 + editor.finder.cursor as u16;
                let cursor_y = win.y + 1;
                execute!(
                    self.stdout,
                    cursor::MoveTo(cursor_x, cursor_y),
                    cursor::Show,
                    cursor::SetCursorStyle::BlinkingBar
                )?;
            }
            _ => {
                // Cursor in buffer
                let cursor_row = editor.cursor.line - editor.viewport_offset;
                let cursor_col = if show_line_numbers {
                    line_num_width + 1 + editor.cursor.col
                } else {
                    editor.cursor.col
                };
                execute!(
                    self.stdout,
                    cursor::MoveTo(cursor_col as u16, cursor_row as u16),
                    cursor::Show
                )?;

                // Set cursor shape based on mode
                match editor.mode {
                    Mode::Insert => execute!(self.stdout, cursor::SetCursorStyle::BlinkingBar)?,
                    Mode::Normal | Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
                        execute!(self.stdout, cursor::SetCursorStyle::BlinkingBlock)?
                    }
                    Mode::Command | Mode::Search | Mode::Finder => {} // Handled above
                }
            }
        }

        self.stdout.flush()?;
        Ok(())
    }

    fn render_status_line(&mut self, editor: &Editor, _line_num_width: usize) -> anyhow::Result<()> {
        print!("\r\n");

        let width = editor.term_width as usize;

        // Left side: mode and filename
        let mode_str = if editor.mode == Mode::Command {
            "NORMAL" // Show NORMAL in status while in command mode (like vim)
        } else {
            editor.mode.as_str()
        };

        // Show pending operator if any
        let pending = if editor.input_state.pending_operator.is_some() || editor.input_state.count.is_some() {
            let mut s = String::new();
            if let Some(count) = editor.input_state.count {
                s.push_str(&count.to_string());
            }
            if let Some(op) = editor.input_state.pending_operator {
                s.push(match op {
                    Operator::Delete => 'd',
                    Operator::Change => 'c',
                    Operator::Yank => 'y',
                    Operator::Indent => '>',
                    Operator::Dedent => '<',
                });
            }
            if !s.is_empty() {
                format!(" [{}]", s)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let filename = editor.buffer().display_name();
        let modified = if editor.buffer().dirty { " [+]" } else { "" };
        let left = format!(" {}{} | {}{} ", mode_str, pending, filename, modified);

        // Right side: language and position
        let lang = editor.syntax.language_name().unwrap_or("plain");
        let right = format!(" {} | {}:{} ", lang, editor.cursor.line + 1, editor.cursor.col + 1);

        // Calculate padding
        let padding = width.saturating_sub(left.len() + right.len());

        // Render status line with background
        execute!(
            self.stdout,
            SetBackgroundColor(Color::DarkGrey),
            SetForegroundColor(Color::White)
        )?;
        print!("{}{:padding$}{}", left, "", right, padding = padding);
        execute!(self.stdout, ResetColor)?;

        Ok(())
    }

    fn render_command_line(&mut self, editor: &Editor) -> anyhow::Result<()> {
        print!("\r\n");
        execute!(self.stdout, terminal::Clear(ClearType::CurrentLine))?;

        if editor.mode == Mode::Command {
            // Show command line input
            print!("{}", editor.command_line.display());
        } else if editor.mode == Mode::Search {
            // Show search prompt
            print!("{}", editor.search.display());
        } else if let Some(ref msg) = editor.status_message {
            // Show status message
            print!("{}", msg);
        }

        Ok(())
    }

    /// Render the fuzzy finder floating window
    fn render_finder(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let win = crate::finder::FloatingWindow::centered(editor.term_width, editor.term_height);

        // Border colors
        let border_color = Color::Rgb { r: 100, g: 100, b: 100 };
        let title_color = Color::Cyan;
        let selected_bg = Color::Rgb { r: 60, g: 60, b: 100 };
        let input_bg = Color::Rgb { r: 30, g: 30, b: 40 };

        // Draw top border
        execute!(self.stdout, cursor::MoveTo(win.x, win.y), SetForegroundColor(border_color))?;
        print!("\u{250c}"); // ┌
        let title = match editor.finder.mode {
            crate::finder::FinderMode::Files => " Find Files ",
            crate::finder::FinderMode::Grep => " Live Grep ",
            crate::finder::FinderMode::Buffers => " Buffers ",
        };
        let title_start = (win.width as usize - title.len()) / 2;
        for i in 1..(win.width - 1) {
            if i as usize == title_start {
                execute!(self.stdout, SetForegroundColor(title_color))?;
                print!("{}", title);
                execute!(self.stdout, SetForegroundColor(border_color))?;
            } else if i as usize >= title_start && (i as usize) < title_start + title.len() {
                // Skip - already printed title
            } else {
                print!("\u{2500}"); // ─
            }
        }
        print!("\u{2510}"); // ┐

        // Draw input line (row after top border)
        execute!(self.stdout, cursor::MoveTo(win.x, win.y + 1))?;
        print!("\u{2502}"); // │
        execute!(self.stdout, SetBackgroundColor(input_bg), ResetColor)?;
        execute!(self.stdout, SetBackgroundColor(input_bg))?;
        print!(">");
        let query_display: String = editor.finder.query.chars().take((win.width - 4) as usize).collect();
        print!("{}", query_display);
        // Pad to fill line
        for _ in (query_display.len() + 1)..(win.width - 2) as usize {
            print!(" ");
        }
        execute!(self.stdout, ResetColor, SetForegroundColor(border_color))?;
        print!("\u{2502}"); // │

        // Draw separator
        execute!(self.stdout, cursor::MoveTo(win.x, win.y + 2))?;
        print!("\u{251c}"); // ├
        for _ in 1..(win.width - 1) {
            print!("\u{2500}"); // ─
        }
        print!("\u{2524}"); // ┤

        // Draw items with scrolling
        let list_height = (win.height - 4) as usize; // Minus borders and input
        let total_items = editor.finder.filtered.len();
        let scroll_offset = editor.finder.scroll_offset;
        let status = format!(" {}/{} ", editor.finder.filtered.len(), editor.finder.items.len());

        // Calculate scroll indicator
        let show_scroll_indicator = total_items > list_height;
        let scroll_indicator_color = Color::DarkGrey;

        for row in 0..list_height {
            let y = win.y + 3 + row as u16;
            execute!(self.stdout, cursor::MoveTo(win.x, y), SetForegroundColor(border_color))?;
            print!("\u{2502}"); // │

            let list_idx = scroll_offset + row;
            if list_idx < total_items {
                let item_idx = editor.finder.filtered[list_idx];
                let item = &editor.finder.items[item_idx];
                let is_selected = list_idx == editor.finder.selected;

                if is_selected {
                    execute!(self.stdout, SetBackgroundColor(selected_bg))?;
                }

                // Truncate display to fit and highlight matches
                // Leave space for scroll indicator if needed
                let max_len = if show_scroll_indicator {
                    (win.width - 4) as usize
                } else {
                    (win.width - 3) as usize
                };
                let display_chars: Vec<char> = item.display.chars().take(max_len).collect();
                let match_color = Color::Yellow;

                for (char_idx, ch) in display_chars.iter().enumerate() {
                    if item.match_indices.contains(&char_idx) {
                        execute!(self.stdout, SetForegroundColor(match_color))?;
                        print!("{}", ch);
                        execute!(self.stdout, ResetColor)?;
                        if is_selected {
                            execute!(self.stdout, SetBackgroundColor(selected_bg))?;
                        }
                    } else {
                        print!("{}", ch);
                    }
                }

                // Pad to fill line
                for _ in display_chars.len()..max_len {
                    print!(" ");
                }

                if is_selected {
                    execute!(self.stdout, ResetColor)?;
                }
            } else {
                // Empty row
                let pad_len = if show_scroll_indicator {
                    (win.width - 4) as usize
                } else {
                    (win.width - 3) as usize
                };
                for _ in 0..pad_len {
                    print!(" ");
                }
            }

            // Draw scroll indicator
            if show_scroll_indicator {
                // Calculate which part of the scrollbar to highlight
                let scroll_bar_pos = if total_items > 0 {
                    (row * total_items) / list_height
                } else {
                    0
                };
                let selected_in_range = scroll_bar_pos <= editor.finder.selected
                    && editor.finder.selected < scroll_bar_pos + (total_items / list_height).max(1);

                if selected_in_range || (row == 0 && scroll_offset == 0) || (row == list_height - 1 && scroll_offset + list_height >= total_items) {
                    execute!(self.stdout, SetForegroundColor(Color::Cyan))?;
                    print!("\u{2588}"); // █ (full block for thumb)
                } else if scroll_offset > 0 || scroll_offset + list_height < total_items {
                    execute!(self.stdout, SetForegroundColor(scroll_indicator_color))?;
                    print!("\u{2591}"); // ░ (light shade for track)
                } else {
                    print!(" ");
                }
            }

            execute!(self.stdout, SetForegroundColor(border_color))?;
            print!("\u{2502}"); // │
        }

        // Draw bottom border with status
        execute!(self.stdout, cursor::MoveTo(win.x, win.y + win.height - 1), SetForegroundColor(border_color))?;
        print!("\u{2514}"); // └
        let status_start = (win.width as usize - status.len()) / 2;
        for i in 1..(win.width - 1) {
            if i as usize == status_start {
                execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                print!("{}", status);
                execute!(self.stdout, SetForegroundColor(border_color))?;
            } else if i as usize >= status_start && (i as usize) < status_start + status.len() {
                // Skip - already printed status
            } else {
                print!("\u{2500}"); // ─
            }
        }
        print!("\u{2518}"); // ┘

        execute!(self.stdout, ResetColor)?;
        Ok(())
    }

    /// Render a line with syntax highlighting and optional visual selection
    fn render_line_with_highlights(
        &mut self,
        line: &str,
        line_idx: usize,
        highlights: &[HighlightSpan],
        visual_range: Option<(usize, usize, usize, usize)>,
        mode: &Mode,
        is_cursor_line: bool,
    ) -> anyhow::Result<()> {
        let chars: Vec<char> = line.chars().collect();
        let line_len = chars.len();

        // Determine selection range for this line based on visual mode
        let (in_selection, sel_start, sel_end) = if let Some((range_start_line, range_start_col, range_end_line, range_end_col)) = visual_range {
            if line_idx < range_start_line || line_idx > range_end_line {
                (false, 0, 0)
            } else {
                match mode {
                    Mode::VisualLine => {
                        // Line-wise: entire line is selected
                        (true, 0, line_len)
                    }
                    Mode::VisualBlock => {
                        // Block-wise: select columns range_start_col to range_end_col (inclusive)
                        // range returns (top, left, bottom, right)
                        (true, range_start_col, range_end_col + 1)
                    }
                    Mode::Visual => {
                        // Character-wise: depends on line position
                        let start = if line_idx == range_start_line { range_start_col } else { 0 };
                        let end = if line_idx == range_end_line { range_end_col + 1 } else { line_len };
                        (true, start, end)
                    }
                    _ => (false, 0, 0)
                }
            }
        } else {
            (false, 0, 0)
        };

        // Cursor line background color
        let cursor_line_bg = Color::Rgb { r: 40, g: 44, b: 52 };

        // Render character by character
        let mut highlight_idx = 0;
        for (col, ch) in chars.iter().enumerate() {
            // Find syntax color for this column
            let syntax_color = Self::get_syntax_color_at(highlights, col, &mut highlight_idx);

            // Check if in visual selection
            let is_selected = in_selection && col >= sel_start && col < sel_end;

            // Apply colors
            if is_selected {
                execute!(self.stdout, SetBackgroundColor(Color::DarkBlue))?;
            } else if is_cursor_line {
                execute!(self.stdout, SetBackgroundColor(cursor_line_bg))?;
            }
            if let Some(fg) = syntax_color {
                execute!(self.stdout, SetForegroundColor(fg))?;
            }

            print!("{}", ch);

            // Reset colors after each character
            execute!(self.stdout, ResetColor)?;

            // Re-apply cursor line background if needed
            if is_cursor_line && !is_selected {
                execute!(self.stdout, SetBackgroundColor(cursor_line_bg))?;
            }
        }

        // Handle selection extending past line end
        if in_selection && sel_end > line_len {
            execute!(self.stdout, SetBackgroundColor(Color::DarkBlue))?;
            print!(" ");
            execute!(self.stdout, ResetColor)?;
        }

        Ok(())
    }

    /// Get the syntax color at a given column position
    fn get_syntax_color_at(highlights: &[HighlightSpan], col: usize, hint_idx: &mut usize) -> Option<Color> {
        // Start searching from hint_idx for efficiency
        while *hint_idx < highlights.len() {
            let span = &highlights[*hint_idx];
            if col < span.start_col {
                // Not yet at this span
                return None;
            } else if col < span.end_col {
                // Inside this span
                return Some(span.fg);
            } else {
                // Past this span, try next
                *hint_idx += 1;
            }
        }
        None
    }

    /// Read a key event (blocking)
    pub fn read_key(&self) -> anyhow::Result<KeyEvent> {
        loop {
            if let Event::Key(key_event) = event::read()? {
                return Ok(key_event);
            }
        }
    }

    /// Check if a key is available (non-blocking)
    #[allow(dead_code)]
    pub fn poll_key(&self, timeout: std::time::Duration) -> anyhow::Result<bool> {
        Ok(event::poll(timeout)?)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Restore terminal state
        let _ = execute!(
            self.stdout,
            cursor::SetCursorStyle::DefaultUserShape,
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

/// Handle a key event and update editor state
pub fn handle_key(editor: &mut Editor, key: KeyEvent) {
    // Clear status message on any key (except for pending operations, command mode, search mode)
    if editor.mode != Mode::Command
        && editor.mode != Mode::Search
        && !editor.mode.is_visual()
        && editor.input_state.pending_operator.is_none()
        && editor.input_state.count.is_none()
    {
        editor.clear_status();
    }

    match editor.mode {
        Mode::Normal => handle_normal_mode(editor, key),
        Mode::Insert => handle_insert_mode(editor, key),
        Mode::Command => handle_command_mode(editor, key),
        Mode::Search => handle_search_mode(editor, key),
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => handle_visual_mode(editor, key),
        Mode::Finder => handle_finder_mode(editor, key),
    }
}

fn handle_normal_mode(editor: &mut Editor, key: KeyEvent) {
    // Handle leader key sequences
    if let Some(ref mut sequence) = editor.leader_sequence {
        // We're in leader mode, accumulating a sequence
        // Escape cancels leader mode
        if key.code == KeyCode::Esc {
            editor.leader_sequence = None;
            editor.clear_status();
            return;
        }

        // Convert key to character and append
        if let KeyCode::Char(c) = key.code {
            sequence.push(c);
            let seq = sequence.clone();

            // Check for exact match
            if let Some(action) = editor.keymap.get_leader_action(&seq) {
                let action = action.clone();
                editor.leader_sequence = None;
                editor.clear_status();
                execute_leader_action(editor, &action);
                return;
            }

            // Check if this could be a prefix of a longer mapping
            if editor.keymap.is_leader_prefix(&seq) {
                // Stay in leader mode, update status to show sequence
                editor.set_status(format!("<leader>{}", seq));
                return;
            }

            // No match and not a prefix - cancel leader mode
            editor.leader_sequence = None;
            editor.clear_status();
            return;
        }

        // Non-character key in leader mode - cancel
        editor.leader_sequence = None;
        editor.clear_status();
        return;
    }

    // Check if this key is the leader key
    if editor.keymap.has_leader_mappings() {
        if editor.keymap.is_leader_key(key) {
            editor.leader_sequence = Some(String::new());
            editor.set_status("<leader>");
            return;
        }
    } else {
        // Debug: no leader mappings loaded
        // Uncomment next line to debug: editor.set_status("No leader mappings");
    }

    // Apply custom keymap remapping
    let key = editor.keymap.remap_normal(key);

    let action = editor.input_state.process_normal_key(key);

    match action {
        KeyAction::Pending => {
            // Key was consumed, waiting for more input
        }

        KeyAction::Motion(motion, count) => {
            editor.apply_motion(motion, count);
        }

        KeyAction::OperatorMotion(op, motion, count) => {
            let register = editor.input_state.take_register();
            match op {
                Operator::Delete => editor.delete_motion(motion, count, register),
                Operator::Change => editor.change_motion(motion, count, register),
                Operator::Yank => editor.yank_motion(motion, count, register),
                Operator::Indent | Operator::Dedent => {
                    editor.set_status("Indent/dedent not implemented yet");
                }
            }
        }

        KeyAction::OperatorLine(op, count) => {
            let register = editor.input_state.take_register();
            match op {
                Operator::Delete => editor.delete_line(count, register),
                Operator::Change => editor.change_line(count, register),
                Operator::Yank => editor.yank_line(count, register),
                Operator::Indent | Operator::Dedent => {
                    editor.set_status("Indent/dedent not implemented yet");
                }
            }
        }

        KeyAction::OperatorTextObject(op, text_object) => {
            let register = editor.input_state.take_register();
            match op {
                Operator::Delete => editor.delete_text_object(text_object, register),
                Operator::Change => editor.change_text_object(text_object, register),
                Operator::Yank => editor.yank_text_object(text_object, register),
                Operator::Indent | Operator::Dedent => {
                    editor.set_status("Indent/dedent not implemented yet");
                }
            }
        }

        KeyAction::SelectTextObject(text_object) => {
            editor.select_text_object(text_object);
        }

        KeyAction::EnterInsert(pos) => {
            match pos {
                InsertPosition::AtCursor => editor.enter_insert_mode(),
                InsertPosition::AfterCursor => editor.enter_insert_mode_append(),
                InsertPosition::LineStart => editor.enter_insert_mode_start(),
                InsertPosition::LineEnd => editor.enter_insert_mode_end(),
                InsertPosition::NewLineBelow => editor.open_line_below(),
                InsertPosition::NewLineAbove => editor.open_line_above(),
            }
        }

        KeyAction::DeleteChar => {
            editor.delete_char_at();
        }

        KeyAction::DeleteCharBefore => {
            editor.delete_char_before_normal();
        }

        KeyAction::PasteAfter => {
            let register = editor.input_state.take_register();
            editor.paste_after(register);
        }

        KeyAction::PasteBefore => {
            let register = editor.input_state.take_register();
            editor.paste_before(register);
        }

        KeyAction::Undo => {
            editor.undo();
        }

        KeyAction::Redo => {
            editor.redo();
        }

        KeyAction::ReplaceChar(c) => {
            editor.replace_char(c);
        }

        KeyAction::JoinLines => {
            editor.join_lines();
        }

        KeyAction::ScrollCenter => {
            editor.scroll_cursor_center();
        }

        KeyAction::ScrollTop => {
            editor.scroll_cursor_top();
        }

        KeyAction::ScrollBottom => {
            editor.scroll_cursor_bottom();
        }

        KeyAction::RepeatLastChange => {
            editor.repeat_last_change();
        }

        KeyAction::EnterCommand => {
            editor.enter_command_mode();
        }

        KeyAction::EnterSearchForward => {
            editor.enter_search_forward();
        }

        KeyAction::EnterSearchBackward => {
            editor.enter_search_backward();
        }

        KeyAction::SearchNext => {
            editor.search_next();
        }

        KeyAction::SearchPrev => {
            editor.search_prev();
        }

        KeyAction::SearchWordForward => {
            editor.search_word_forward();
        }

        KeyAction::SearchWordBackward => {
            editor.search_word_backward();
        }

        KeyAction::EnterVisual => {
            editor.enter_visual_mode();
        }

        KeyAction::EnterVisualLine => {
            editor.enter_visual_line_mode();
        }

        KeyAction::EnterVisualBlock => {
            editor.enter_visual_block_mode();
        }

        KeyAction::Quit => {
            editor.should_quit = true;
        }

        KeyAction::Save => {
            if let Err(e) = editor.save() {
                editor.set_status(format!("Error saving: {}", e));
            }
        }

        // Window/pane operations
        KeyAction::WindowSplitVertical => {
            if let Err(e) = editor.vsplit(None) {
                editor.set_status(format!("Error: {}", e));
            }
        }

        KeyAction::WindowSplitHorizontal => {
            if let Err(e) = editor.hsplit(None) {
                editor.set_status(format!("Error: {}", e));
            }
        }

        KeyAction::WindowClose => {
            if !editor.close_pane() {
                // Last pane - quit the editor
                editor.should_quit = true;
            }
        }

        KeyAction::WindowCloseOthers => {
            editor.close_other_panes();
        }

        KeyAction::WindowNext => {
            editor.next_pane();
        }

        KeyAction::WindowPrev => {
            editor.prev_pane();
        }

        KeyAction::WindowLeft => {
            editor.move_to_pane_direction(PaneDirection::Left);
        }

        KeyAction::WindowRight => {
            editor.move_to_pane_direction(PaneDirection::Right);
        }

        KeyAction::WindowUp => {
            editor.move_to_pane_direction(PaneDirection::Up);
        }

        KeyAction::WindowDown => {
            editor.move_to_pane_direction(PaneDirection::Down);
        }

        KeyAction::Unknown => {
            // Unknown key, ignore
        }
    }
}

fn handle_insert_mode(editor: &mut Editor, key: KeyEvent) {
    // Apply custom keymap remapping for insert mode
    let key = editor.keymap.remap_insert(key);

    match (key.modifiers, key.code) {
        // Exit insert mode
        (KeyModifiers::NONE, KeyCode::Esc) => {
            editor.enter_normal_mode();
        }

        // Also allow Ctrl-[ as escape (like vim)
        (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
            editor.enter_normal_mode();
        }

        // Backspace
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            editor.delete_char_before();
        }

        // Enter
        (KeyModifiers::NONE, KeyCode::Enter) => {
            editor.insert_char('\n');
        }

        // Tab
        (KeyModifiers::NONE, KeyCode::Tab) => {
            // Insert spaces based on configured tab width
            for _ in 0..editor.settings.editor.tab_width {
                editor.insert_char(' ');
            }
        }

        // Regular character
        (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) => {
            editor.insert_char(c);
        }

        // Arrow keys work in insert mode too
        (_, KeyCode::Left) => {
            if editor.cursor.col > 0 {
                editor.cursor.col -= 1;
            }
        }
        (_, KeyCode::Right) => {
            let line_len = editor.buffer().line_len(editor.cursor.line);
            if editor.cursor.col < line_len {
                editor.cursor.col += 1;
            }
        }
        (_, KeyCode::Up) => {
            if editor.cursor.line > 0 {
                editor.cursor.line -= 1;
                editor.clamp_cursor();
                editor.scroll_to_cursor();
            }
        }
        (_, KeyCode::Down) => {
            if editor.cursor.line < editor.buffer().len_lines() - 1 {
                editor.cursor.line += 1;
                editor.clamp_cursor();
                editor.scroll_to_cursor();
            }
        }

        _ => {}
    }
}

fn handle_command_mode(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Cancel command
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) |
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            editor.exit_command_mode();
        }

        // Execute command
        (KeyModifiers::NONE, KeyCode::Enter) => {
            let cmd = editor.command_line.execute();
            editor.mode = Mode::Normal;
            execute_command(editor, cmd);
        }

        // Backspace
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            if editor.command_line.input.is_empty() {
                editor.exit_command_mode();
            } else {
                editor.command_line.delete_char_before();
            }
        }

        // Delete
        (KeyModifiers::NONE, KeyCode::Delete) => {
            editor.command_line.delete_char_at();
        }

        // Cursor movement
        (KeyModifiers::NONE, KeyCode::Left) => {
            editor.command_line.move_left();
        }
        (KeyModifiers::NONE, KeyCode::Right) => {
            editor.command_line.move_right();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('a')) |
        (KeyModifiers::NONE, KeyCode::Home) => {
            editor.command_line.move_to_start();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) |
        (KeyModifiers::NONE, KeyCode::End) => {
            editor.command_line.move_to_end();
        }

        // History navigation
        (KeyModifiers::NONE, KeyCode::Up) => {
            editor.command_line.history_prev();
        }
        (KeyModifiers::NONE, KeyCode::Down) => {
            editor.command_line.history_next();
        }

        // Clear line
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            editor.command_line.clear();
        }

        // Regular character
        (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) => {
            editor.command_line.insert_char(c);
        }

        _ => {}
    }
}

fn handle_search_mode(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Cancel search
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) |
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            editor.exit_search_mode();
        }

        // Execute search
        (KeyModifiers::NONE, KeyCode::Enter) => {
            editor.execute_search();
        }

        // Backspace
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            if editor.search.input.is_empty() {
                editor.exit_search_mode();
            } else {
                editor.search.delete_char_before();
            }
        }

        // Cursor movement
        (KeyModifiers::NONE, KeyCode::Left) => {
            editor.search.move_left();
        }
        (KeyModifiers::NONE, KeyCode::Right) => {
            editor.search.move_right();
        }

        // Regular character
        (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) => {
            editor.search.insert_char(c);
        }

        _ => {}
    }
}

fn handle_visual_mode(editor: &mut Editor, key: KeyEvent) {
    use crate::input::Motion;

    // Handle text object selection (after i or a was pressed)
    if let Some(modifier) = editor.input_state.pending_text_object.take() {
        let object_type = match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Char('w')) => Some(TextObjectType::Word),
            (KeyModifiers::SHIFT, KeyCode::Char('W')) => Some(TextObjectType::BigWord),
            (_, KeyCode::Char('"')) => Some(TextObjectType::DoubleQuote),
            (_, KeyCode::Char('\'')) => Some(TextObjectType::SingleQuote),
            (_, KeyCode::Char('`')) => Some(TextObjectType::BackTick),
            (_, KeyCode::Char('(')) | (_, KeyCode::Char(')')) => Some(TextObjectType::Paren),
            (KeyModifiers::NONE, KeyCode::Char('b')) => Some(TextObjectType::Paren),
            (_, KeyCode::Char('{')) | (_, KeyCode::Char('}')) => Some(TextObjectType::Brace),
            (KeyModifiers::SHIFT, KeyCode::Char('B')) => Some(TextObjectType::Brace),
            (_, KeyCode::Char('[')) | (_, KeyCode::Char(']')) => Some(TextObjectType::Bracket),
            (_, KeyCode::Char('<')) | (_, KeyCode::Char('>')) => Some(TextObjectType::AngleBracket),
            _ => None,
        };

        if let Some(obj_type) = object_type {
            let text_object = TextObject {
                modifier,
                object_type: obj_type,
            };
            editor.select_text_object(text_object);
        }
        return;
    }

    match (key.modifiers, key.code) {
        // Exit visual mode
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) |
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            editor.exit_visual_mode();
        }

        // Toggle visual mode type
        (KeyModifiers::NONE, KeyCode::Char('v')) => {
            if editor.mode == Mode::Visual {
                editor.exit_visual_mode();
            } else {
                editor.mode = Mode::Visual;
            }
        }
        (KeyModifiers::SHIFT, KeyCode::Char('V')) => {
            if editor.mode == Mode::VisualLine {
                editor.exit_visual_mode();
            } else {
                editor.mode = Mode::VisualLine;
            }
        }
        (KeyModifiers::CONTROL, KeyCode::Char('v')) => {
            if editor.mode == Mode::VisualBlock {
                editor.exit_visual_mode();
            } else {
                editor.mode = Mode::VisualBlock;
            }
        }

        // Operators
        (KeyModifiers::NONE, KeyCode::Char('d')) |
        (KeyModifiers::NONE, KeyCode::Char('x')) => {
            editor.visual_delete();
        }
        (KeyModifiers::NONE, KeyCode::Char('c')) |
        (KeyModifiers::NONE, KeyCode::Char('s')) => {
            editor.visual_change();
        }
        (KeyModifiers::NONE, KeyCode::Char('y')) => {
            editor.visual_yank();
        }

        // Motions - extend selection
        (KeyModifiers::NONE, KeyCode::Char('h')) | (_, KeyCode::Left) => {
            editor.apply_motion(Motion::Left, 1);
        }
        (KeyModifiers::NONE, KeyCode::Char('j')) | (_, KeyCode::Down) => {
            editor.apply_motion(Motion::Down, 1);
        }
        (KeyModifiers::NONE, KeyCode::Char('k')) | (_, KeyCode::Up) => {
            editor.apply_motion(Motion::Up, 1);
        }
        (KeyModifiers::NONE, KeyCode::Char('l')) | (_, KeyCode::Right) => {
            editor.apply_motion(Motion::Right, 1);
        }

        // Word motions
        (KeyModifiers::NONE, KeyCode::Char('w')) => {
            editor.apply_motion(Motion::WordForward, 1);
        }
        (KeyModifiers::SHIFT, KeyCode::Char('W')) => {
            editor.apply_motion(Motion::BigWordForward, 1);
        }
        (KeyModifiers::NONE, KeyCode::Char('b')) => {
            editor.apply_motion(Motion::WordBackward, 1);
        }
        (KeyModifiers::SHIFT, KeyCode::Char('B')) => {
            editor.apply_motion(Motion::BigWordBackward, 1);
        }
        (KeyModifiers::NONE, KeyCode::Char('e')) => {
            editor.apply_motion(Motion::WordEnd, 1);
        }
        (KeyModifiers::SHIFT, KeyCode::Char('E')) => {
            editor.apply_motion(Motion::BigWordEnd, 1);
        }

        // Line motions
        (KeyModifiers::NONE, KeyCode::Char('0')) => {
            editor.apply_motion(Motion::LineStart, 1);
        }
        (_, KeyCode::Char('^')) => {
            editor.apply_motion(Motion::FirstNonBlank, 1);
        }
        (_, KeyCode::Char('$')) => {
            editor.apply_motion(Motion::LineEnd, 1);
        }

        // Paragraph motions
        (_, KeyCode::Char('}')) => {
            editor.apply_motion(Motion::ParagraphForward, 1);
        }
        (_, KeyCode::Char('{')) => {
            editor.apply_motion(Motion::ParagraphBackward, 1);
        }

        // Bracket matching
        (_, KeyCode::Char('%')) => {
            editor.apply_motion(Motion::MatchingBracket, 1);
        }

        // File motions
        (KeyModifiers::NONE, KeyCode::Char('g')) => {
            // Note: would need to handle gg sequence, for now just go to start
        }
        (KeyModifiers::SHIFT, KeyCode::Char('G')) => {
            editor.apply_motion(Motion::FileEnd, 1);
        }

        // Page motions
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            editor.apply_motion(Motion::HalfPageDown, 1);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            editor.apply_motion(Motion::HalfPageUp, 1);
        }

        // Swap cursor to other end of selection
        (KeyModifiers::NONE, KeyCode::Char('o')) => {
            // Swap anchor and cursor
            let old_anchor_line = editor.visual.anchor_line;
            let old_anchor_col = editor.visual.anchor_col;
            editor.visual.anchor_line = editor.cursor.line;
            editor.visual.anchor_col = editor.cursor.col;
            editor.cursor.line = old_anchor_line;
            editor.cursor.col = old_anchor_col;
            editor.scroll_to_cursor();
        }

        // Text object selection (i = inner, a = around)
        (KeyModifiers::NONE, KeyCode::Char('i')) => {
            editor.input_state.pending_text_object = Some(TextObjectModifier::Inner);
        }
        (KeyModifiers::NONE, KeyCode::Char('a')) => {
            editor.input_state.pending_text_object = Some(TextObjectModifier::Around);
        }

        _ => {}
    }
}

fn handle_finder_mode(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Cancel finder
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) |
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            editor.close_finder();
        }

        // Select item
        (KeyModifiers::NONE, KeyCode::Enter) => {
            if let Some((path, line)) = editor.finder_select() {
                // Open the selected file
                if let Err(e) = editor.open_file(path) {
                    editor.set_status(format!("Error opening file: {}", e));
                } else if let Some(line_num) = line {
                    // Jump to the line (for grep results)
                    editor.cursor.line = line_num.saturating_sub(1);
                    editor.cursor.col = 0;
                    editor.scroll_to_cursor();
                }
            }
        }

        // Navigate up
        (KeyModifiers::NONE, KeyCode::Up) |
        (KeyModifiers::CONTROL, KeyCode::Char('k')) |
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            editor.finder.select_prev();
            // Adjust scroll to keep selection visible
            let win = crate::finder::FloatingWindow::centered(editor.term_width, editor.term_height);
            let list_height = (win.height - 4) as usize;
            editor.finder.adjust_scroll(list_height);
        }

        // Navigate down
        (KeyModifiers::NONE, KeyCode::Down) |
        (KeyModifiers::CONTROL, KeyCode::Char('j')) |
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            editor.finder.select_next();
            // Adjust scroll to keep selection visible
            let win = crate::finder::FloatingWindow::centered(editor.term_width, editor.term_height);
            let list_height = (win.height - 4) as usize;
            editor.finder.adjust_scroll(list_height);
        }

        // Backspace
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            editor.finder.delete_char_before();
        }

        // Regular character
        (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) => {
            editor.finder.insert_char(c);
        }

        _ => {}
    }
}

/// Execute a parsed command
fn execute_command(editor: &mut Editor, cmd: Command) {
    let result = match cmd {
        Command::Write(path) => {
            if let Some(p) = path {
                match editor.save_as(p) {
                    Ok(()) => CommandResult::Ok,
                    Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
                }
            } else if editor.buffer().path.is_some() {
                match editor.save() {
                    Ok(()) => CommandResult::Ok,
                    Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
                }
            } else {
                CommandResult::Error("No filename".to_string())
            }
        }

        Command::Quit => {
            if editor.has_unsaved_changes() {
                CommandResult::Error("No write since last change (add ! to override)".to_string())
            } else {
                CommandResult::Quit
            }
        }

        Command::ForceQuit => {
            CommandResult::Quit
        }

        Command::WriteQuit => {
            if editor.buffer().path.is_some() {
                match editor.save() {
                    Ok(()) => CommandResult::Quit,
                    Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
                }
            } else {
                CommandResult::Error("No filename".to_string())
            }
        }

        Command::WriteQuitIfModified => {
            if editor.has_unsaved_changes() {
                if editor.buffer().path.is_some() {
                    match editor.save() {
                        Ok(()) => CommandResult::Quit,
                        Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
                    }
                } else {
                    CommandResult::Error("No filename".to_string())
                }
            } else {
                CommandResult::Quit
            }
        }

        Command::Edit(path) => {
            if let Some(p) = path {
                match editor.open_file(p) {
                    Ok(()) => CommandResult::Message(format!("\"{}\"", editor.buffer().display_name())),
                    Err(e) => CommandResult::Error(format!("Error opening file: {}", e)),
                }
            } else if editor.buffer().path.is_some() {
                if editor.has_unsaved_changes() {
                    CommandResult::Error("No write since last change (add ! to override)".to_string())
                } else {
                    match editor.reload() {
                        Ok(()) => CommandResult::Ok,
                        Err(e) => CommandResult::Error(format!("Error reloading: {}", e)),
                    }
                }
            } else {
                CommandResult::Error("No filename".to_string())
            }
        }

        Command::Reload => {
            match editor.reload() {
                Ok(()) => CommandResult::Ok,
                Err(e) => CommandResult::Error(format!("Error reloading: {}", e)),
            }
        }

        Command::GotoLine(line) => {
            editor.goto_line(line);
            CommandResult::Ok
        }

        Command::Next => {
            if editor.buffer_count() > 1 {
                editor.next_buffer();
                CommandResult::Message(format!(
                    "Buffer {}/{}",
                    editor.current_buffer_index() + 1,
                    editor.buffer_count()
                ))
            } else {
                CommandResult::Message("Only one buffer".to_string())
            }
        }

        Command::Prev => {
            if editor.buffer_count() > 1 {
                editor.prev_buffer();
                CommandResult::Message(format!(
                    "Buffer {}/{}",
                    editor.current_buffer_index() + 1,
                    editor.buffer_count()
                ))
            } else {
                CommandResult::Message("Only one buffer".to_string())
            }
        }

        Command::Set(option, _value) => {
            CommandResult::Error(format!("Unknown option: {}", option))
        }

        Command::LazyGit => {
            CommandResult::RunExternal("lazygit".to_string())
        }

        Command::Shell(shell_cmd) => {
            if shell_cmd.is_empty() {
                CommandResult::Error("No command specified".to_string())
            } else {
                CommandResult::RunExternal(shell_cmd)
            }
        }

        Command::VSplit(path) => {
            match editor.vsplit(path) {
                Ok(()) => CommandResult::Ok,
                Err(e) => CommandResult::Error(format!("Error: {}", e)),
            }
        }

        Command::HSplit(path) => {
            match editor.hsplit(path) {
                Ok(()) => CommandResult::Ok,
                Err(e) => CommandResult::Error(format!("Error: {}", e)),
            }
        }

        Command::Only => {
            editor.close_other_panes();
            CommandResult::Ok
        }

        Command::FindFiles => {
            editor.open_finder_files();
            CommandResult::Ok
        }

        Command::FindBuffers => {
            editor.open_finder_buffers();
            CommandResult::Ok
        }

        Command::LiveGrep => {
            editor.open_finder_grep();
            CommandResult::Ok
        }

        Command::Unknown(cmd) => {
            if cmd.is_empty() {
                CommandResult::Ok
            } else {
                CommandResult::Error(format!("Not an editor command: {}", cmd))
            }
        }
    };

    // Handle the result
    match result {
        CommandResult::Ok => {}
        CommandResult::Message(msg) => {
            editor.set_status(msg);
        }
        CommandResult::Error(err) => {
            editor.set_status(format!("E: {}", err));
        }
        CommandResult::Quit => {
            editor.should_quit = true;
        }
        CommandResult::RunExternal(cmd) => {
            editor.pending_external_command = Some(cmd);
        }
    }
}

/// Execute a leader key action
fn execute_leader_action(editor: &mut Editor, action: &LeaderAction) {
    match action {
        LeaderAction::Command(cmd_str) => {
            // Parse and execute the command
            let cmd = parse_command(cmd_str);
            execute_command(editor, cmd);
        }
        LeaderAction::Keys(keys) => {
            // Execute each key in the sequence
            for key in keys {
                handle_normal_mode(editor, *key);
            }
        }
    }
}
