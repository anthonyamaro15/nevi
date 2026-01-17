use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, ClearType},
    style::{SetForegroundColor, SetBackgroundColor, ResetColor, Color, SetAttribute, Attribute},
};
use std::io::{self, Write, Stdout};

use crate::editor::{Editor, Mode, PaneDirection, Pane, SplitLayout, LspAction};
use crate::input::{KeyAction, InsertPosition, Operator, TextObject, TextObjectModifier, TextObjectType};
use crate::commands::{Command, CommandResult, parse_command};
use crate::config::LeaderAction;
use crate::syntax::HighlightSpan;
use crate::lsp::types::CompletionKind;

/// Section types for hover content parsing
enum HoverSection {
    #[allow(dead_code)]
    Code { language: String, lines: Vec<String> },
    Text(String),
}

/// Line type for hover rendering
#[derive(Clone, Copy)]
enum HoverLineType {
    Code,
    Text,
    Separator,
}

/// A wrapped segment of a line
#[derive(Debug, Clone)]
struct WrapSegment {
    /// Start column in the original line
    start_col: usize,
    /// The text content of this segment
    text: String,
    /// Whether this is the first segment (shows line number)
    is_first: bool,
}

/// Calculate wrapped segments for a line
/// Returns a vector of segments, each representing one visual row
fn calculate_wrap_segments(line: &str, max_width: usize, preserve_indent: bool) -> Vec<WrapSegment> {
    if max_width == 0 {
        return vec![WrapSegment {
            start_col: 0,
            text: line.to_string(),
            is_first: true,
        }];
    }

    let line = line.trim_end_matches('\n');
    let chars: Vec<char> = line.chars().collect();

    if chars.len() <= max_width {
        return vec![WrapSegment {
            start_col: 0,
            text: line.to_string(),
            is_first: true,
        }];
    }

    // Calculate the indentation of the original line
    let indent_len = if preserve_indent {
        chars.iter().take_while(|c| c.is_whitespace()).count()
    } else {
        0
    };
    let indent: String = chars.iter().take(indent_len).collect();

    let mut segments = Vec::new();
    let mut current_col = 0;
    let mut is_first = true;

    while current_col < chars.len() {
        let segment_indent = if is_first { "" } else { &indent };
        let available_width = if is_first {
            max_width
        } else {
            max_width.saturating_sub(indent_len)
        };

        if available_width == 0 {
            // Can't fit anything, just take one char to avoid infinite loop
            let text: String = std::iter::once(chars[current_col]).collect();
            segments.push(WrapSegment {
                start_col: current_col,
                text: format!("{}{}", segment_indent, text),
                is_first,
            });
            current_col += 1;
        } else {
            let remaining = chars.len() - current_col;
            let take_count = remaining.min(available_width);
            let text: String = chars[current_col..current_col + take_count].iter().collect();

            segments.push(WrapSegment {
                start_col: current_col,
                text: format!("{}{}", segment_indent, text),
                is_first,
            });
            current_col += take_count;
        }
        is_first = false;
    }

    if segments.is_empty() {
        segments.push(WrapSegment {
            start_col: 0,
            text: String::new(),
            is_first: true,
        });
    }

    segments
}

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

        let num_panes = editor.panes().len();

        // Render file explorer sidebar if visible
        if editor.explorer.visible {
            self.render_explorer(editor)?;
        }

        // Render all panes
        for (pane_idx, pane) in editor.panes().iter().enumerate() {
            let is_active = pane_idx == editor.active_pane_idx();
            self.render_pane(editor, pane, is_active)?;
        }

        // Draw separators between panes if we have multiple panes
        if num_panes > 1 {
            self.render_pane_separators(editor)?;
        }

        // Render status line
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3);
        self.render_status_line(editor, line_num_width)?;

        // Render command/message line
        self.render_command_line(editor)?;

        // Render finder if in finder mode
        if editor.mode == Mode::Finder {
            self.render_finder(editor)?;
        }

        // Render completion popup if active
        if editor.completion.active {
            self.render_completion(editor)?;
        }

        // Render hover popup if active
        if editor.hover_content.is_some() {
            self.render_hover(editor)?;
        }

        // Render signature help popup if active
        if editor.signature_help.is_some() {
            self.render_signature_help(editor)?;
        }

        // Render references picker if active
        if editor.references_picker.is_some() {
            self.render_references_picker(editor)?;
        }

        // Render code actions picker if active
        if editor.code_actions_picker.is_some() {
            self.render_code_actions_picker(editor)?;
        }

        // Render harpoon menu if active
        if editor.harpoon.menu_open {
            self.render_harpoon_menu(editor)?;
        }

        // Position cursor
        self.position_cursor(editor)?;

        self.stdout.flush()?;
        Ok(())
    }

    /// Render a single pane's content
    fn render_pane(&mut self, editor: &Editor, pane: &Pane, is_active: bool) -> anyhow::Result<()> {
        let buffer = editor.buffer_at(pane.buffer_idx).unwrap();
        let rect = &pane.rect;

        // Calculate line number width for this buffer
        let line_num_width = buffer.len_lines().to_string().len().max(3);

        // Get visual selection range if in visual mode and this is active pane
        let visual_range = if is_active && editor.mode.is_visual() {
            Some(editor.get_visual_range())
        } else {
            None
        };

        // Settings
        let show_line_numbers = editor.settings.editor.line_numbers;
        let show_relative = editor.settings.editor.relative_numbers;
        let highlight_cursor_line = is_active && editor.settings.editor.cursor_line;
        let wrap_enabled = editor.settings.editor.wrap;
        let wrap_width = editor.settings.editor.wrap_width;

        let pane_height = rect.height as usize;
        let pane_width = rect.width as usize;

        // Sign column width (for diagnostic icons)
        const SIGN_COLUMN_WIDTH: usize = 2;

        // Calculate effective text width (excluding sign column and line numbers)
        let text_area_width = if show_line_numbers {
            pane_width.saturating_sub(SIGN_COLUMN_WIDTH + line_num_width + 1)
        } else {
            pane_width.saturating_sub(SIGN_COLUMN_WIDTH)
        };

        // Calculate wrap width: use configured wrap_width or text_area_width, whichever is smaller
        let effective_wrap_width = if wrap_enabled {
            wrap_width.min(text_area_width)
        } else {
            text_area_width
        };

        if wrap_enabled {
            // Wrap-aware rendering
            self.render_pane_wrapped(
                editor, pane, buffer, rect, is_active,
                line_num_width, visual_range, show_line_numbers,
                show_relative, highlight_cursor_line,
                pane_height, pane_width, effective_wrap_width,
            )?;
        } else {
            // Original non-wrapped rendering
            self.render_pane_nowrap(
                editor, pane, buffer, rect, is_active,
                line_num_width, visual_range, show_line_numbers,
                show_relative, highlight_cursor_line,
                pane_height, pane_width, text_area_width,
            )?;
        }

        Ok(())
    }

    /// Render pane with soft wrap enabled
    #[allow(clippy::too_many_arguments)]
    fn render_pane_wrapped(
        &mut self,
        editor: &Editor,
        pane: &Pane,
        buffer: &crate::editor::Buffer,
        rect: &crate::editor::Rect,
        is_active: bool,
        line_num_width: usize,
        visual_range: Option<(usize, usize, usize, usize)>,
        show_line_numbers: bool,
        show_relative: bool,
        highlight_cursor_line: bool,
        pane_height: usize,
        pane_width: usize,
        wrap_width: usize,
    ) -> anyhow::Result<()> {
        let mut current_row = 0;
        let mut file_line = pane.viewport_offset;

        while current_row < pane_height && file_line < buffer.len_lines() {
            let is_cursor_line = is_active && file_line == pane.cursor.line;

            // Get line content
            let line_content = buffer.line(file_line)
                .map(|l| l.to_string())
                .unwrap_or_default();

            // Calculate wrapped segments
            let segments = calculate_wrap_segments(&line_content, wrap_width, true);

            // Get syntax highlights for this line
            let highlights = if is_active {
                editor.syntax.get_line_highlights(file_line)
            } else {
                Vec::new()
            };

            // Get diagnostics for line number coloring
            let line_diagnostics = if is_active {
                editor.diagnostics_for_line(file_line)
            } else {
                Vec::new()
            };
            let has_error = line_diagnostics.iter().any(|d| {
                matches!(d.severity, crate::lsp::types::DiagnosticSeverity::Error)
            });
            let has_warning = line_diagnostics.iter().any(|d| {
                matches!(d.severity, crate::lsp::types::DiagnosticSeverity::Warning)
            });

            // Render each segment
            for segment in &segments {
                if current_row >= pane_height {
                    break;
                }

                let screen_y = rect.y + current_row as u16;
                execute!(self.stdout, cursor::MoveTo(rect.x, screen_y))?;

                // Apply cursor line background if enabled (for all segments of cursor line)
                if highlight_cursor_line && is_cursor_line {
                    execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                }

                // Sign column (diagnostic icons) - only on first segment
                if segment.is_first {
                    if has_error {
                        execute!(self.stdout, SetForegroundColor(Color::Rgb { r: 255, g: 100, b: 100 }))?;
                        print!("● ");
                        execute!(self.stdout, ResetColor)?;
                    } else if has_warning {
                        execute!(self.stdout, SetForegroundColor(Color::Rgb { r: 255, g: 200, b: 100 }))?;
                        print!("▲ ");
                        execute!(self.stdout, ResetColor)?;
                    } else {
                        print!("  "); // Empty sign column
                    }
                } else {
                    print!("  "); // Empty sign column for continuation lines
                }

                // Re-apply cursor line background after sign column
                if highlight_cursor_line && is_cursor_line {
                    execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                }

                // Line number (only on first segment)
                if show_line_numbers {
                    if segment.is_first {
                        let line_num = if show_relative && is_active {
                            let distance = (file_line as isize - pane.cursor.line as isize).abs() as usize;
                            if distance == 0 {
                                format!("{:>width$} ", file_line + 1, width = line_num_width)
                            } else {
                                format!("{:>width$} ", distance, width = line_num_width)
                            }
                        } else {
                            format!("{:>width$} ", file_line + 1, width = line_num_width)
                        };

                        // Use brighter colors for better visibility
                        let line_num_color = if has_error {
                            Color::Rgb { r: 255, g: 100, b: 100 } // Bright red
                        } else if has_warning {
                            Color::Rgb { r: 255, g: 200, b: 100 } // Bright yellow/orange
                        } else if is_cursor_line {
                            Color::Yellow
                        } else {
                            Color::DarkGrey
                        };

                        execute!(self.stdout, SetForegroundColor(line_num_color))?;
                        print!("{}", line_num);
                        execute!(self.stdout, ResetColor)?;

                        if highlight_cursor_line && is_cursor_line {
                            execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                        }
                    } else {
                        // Continuation line - empty line number gutter
                        print!("{:>width$} ", "", width = line_num_width);
                    }
                }

                // Render segment content with syntax highlighting
                let segment_text = segment.text.trim_end_matches('\n');
                self.render_line_segment_with_highlights(
                    segment_text,
                    file_line,
                    segment.start_col,
                    &highlights,
                    visual_range,
                    &editor.mode,
                    highlight_cursor_line && is_cursor_line,
                    &editor.search_matches,
                )?;

                // Fill remaining space (sign column = 2)
                let mut chars_printed = 2 + if show_line_numbers { line_num_width + 1 } else { 0 }
                    + segment_text.chars().count();

                // Render inline diagnostic on first segment only
                if segment.is_first && is_active {
                    if let Some(diag) = line_diagnostics.first() {
                        let remaining = pane_width.saturating_sub(chars_printed + 3);
                        if remaining > 5 {
                            let (color, icon) = match diag.severity {
                                crate::lsp::types::DiagnosticSeverity::Error => (Color::Red, "●"),
                                crate::lsp::types::DiagnosticSeverity::Warning => (Color::Yellow, "●"),
                                crate::lsp::types::DiagnosticSeverity::Information => (Color::Blue, "●"),
                                crate::lsp::types::DiagnosticSeverity::Hint => (Color::Cyan, "○"),
                            };

                            let msg: String = diag.message
                                .lines()
                                .next()
                                .unwrap_or(&diag.message)
                                .chars()
                                .take(remaining)
                                .collect();

                            if highlight_cursor_line && is_cursor_line {
                                execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                            }

                            execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                            print!(" ");
                            execute!(self.stdout, SetForegroundColor(color))?;
                            print!("{}", icon);
                            execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                            print!(" {}", msg);
                            execute!(self.stdout, ResetColor)?;

                            chars_printed += 3 + msg.chars().count();

                            if highlight_cursor_line && is_cursor_line {
                                execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                            }
                        }
                    }
                }

                for _ in chars_printed..pane_width {
                    print!(" ");
                }

                // Reset background
                if highlight_cursor_line && is_cursor_line {
                    execute!(self.stdout, ResetColor)?;
                }

                current_row += 1;
            }

            file_line += 1;
        }

        // Fill remaining rows with ~ indicators
        while current_row < pane_height {
            let screen_y = rect.y + current_row as u16;
            execute!(self.stdout, cursor::MoveTo(rect.x, screen_y))?;

            print!("  "); // Empty sign column

            execute!(self.stdout, SetForegroundColor(Color::Blue))?;
            if show_line_numbers {
                print!("{:>width$} ~", "", width = line_num_width);
            } else {
                print!("~");
            }
            execute!(self.stdout, ResetColor)?;

            // Fill remaining space (sign column = 2)
            let chars_printed = 2 + if show_line_numbers { line_num_width + 2 } else { 1 };
            for _ in chars_printed..pane_width {
                print!(" ");
            }

            current_row += 1;
        }

        Ok(())
    }

    /// Render pane without wrapping (original behavior)
    #[allow(clippy::too_many_arguments)]
    fn render_pane_nowrap(
        &mut self,
        editor: &Editor,
        pane: &Pane,
        buffer: &crate::editor::Buffer,
        rect: &crate::editor::Rect,
        is_active: bool,
        line_num_width: usize,
        visual_range: Option<(usize, usize, usize, usize)>,
        show_line_numbers: bool,
        show_relative: bool,
        highlight_cursor_line: bool,
        pane_height: usize,
        pane_width: usize,
        effective_width: usize,
    ) -> anyhow::Result<()> {
        // Render each row in this pane
        for row in 0..pane_height {
            let screen_y = rect.y + row as u16;
            let file_line = pane.viewport_offset + row;
            let is_cursor_line = is_active && file_line == pane.cursor.line;

            // Move to start of this row in the pane
            execute!(self.stdout, cursor::MoveTo(rect.x, screen_y))?;

            // Apply cursor line background if enabled
            if highlight_cursor_line && is_cursor_line && file_line < buffer.len_lines() {
                execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
            }

            if file_line < buffer.len_lines() {
                // Check for diagnostics on this line (only for active pane)
                let line_diagnostics = if is_active {
                    editor.diagnostics_for_line(file_line)
                } else {
                    Vec::new()
                };
                let has_error = line_diagnostics.iter().any(|d| {
                    matches!(d.severity, crate::lsp::types::DiagnosticSeverity::Error)
                });
                let has_warning = line_diagnostics.iter().any(|d| {
                    matches!(d.severity, crate::lsp::types::DiagnosticSeverity::Warning)
                });

                // Sign column (diagnostic icons)
                if has_error {
                    execute!(self.stdout, SetForegroundColor(Color::Rgb { r: 255, g: 100, b: 100 }))?;
                    print!("● ");
                    execute!(self.stdout, ResetColor)?;
                } else if has_warning {
                    execute!(self.stdout, SetForegroundColor(Color::Rgb { r: 255, g: 200, b: 100 }))?;
                    print!("▲ ");
                    execute!(self.stdout, ResetColor)?;
                } else {
                    print!("  "); // Empty sign column
                }

                // Re-apply cursor line background after sign column
                if highlight_cursor_line && is_cursor_line {
                    execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                }

                // Line number (if enabled)
                if show_line_numbers {
                    let line_num = if show_relative && is_active {
                        // Relative line numbers: show distance from cursor, current line shows absolute
                        let distance = (file_line as isize - pane.cursor.line as isize).abs() as usize;
                        if distance == 0 {
                            format!("{:>width$} ", file_line + 1, width = line_num_width)
                        } else {
                            format!("{:>width$} ", distance, width = line_num_width)
                        }
                    } else {
                        format!("{:>width$} ", file_line + 1, width = line_num_width)
                    };

                    // Use brighter colors for better visibility
                    let line_num_color = if has_error {
                        Color::Rgb { r: 255, g: 100, b: 100 } // Bright red
                    } else if has_warning {
                        Color::Rgb { r: 255, g: 200, b: 100 } // Bright yellow/orange
                    } else if is_cursor_line {
                        Color::Yellow
                    } else {
                        Color::DarkGrey
                    };

                    execute!(self.stdout, SetForegroundColor(line_num_color))?;
                    print!("{}", line_num);
                    execute!(self.stdout, ResetColor)?;

                    // Re-apply cursor line background after reset
                    if highlight_cursor_line && is_cursor_line {
                        execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                    }
                }

                // Line content with syntax highlighting and visual selection
                if let Some(line) = buffer.line(file_line) {
                    let line_str: String = line.chars().take(effective_width).collect();
                    let line_str = line_str.trim_end_matches('\n');

                    // Get syntax highlights for this line (only for active pane)
                    let highlights = if is_active {
                        editor.syntax.get_line_highlights(file_line)
                    } else {
                        Vec::new()
                    };

                    self.render_line_with_highlights(
                        line_str,
                        file_line,
                        &highlights,
                        visual_range,
                        &editor.mode,
                        highlight_cursor_line && is_cursor_line,
                        &editor.search_matches,
                    )?;

                    // Track characters printed for fill calculation (sign column = 2)
                    let mut chars_printed = 2 + if show_line_numbers { line_num_width + 1 } else { 0 } + line_str.chars().count();

                    // Render ghost text on cursor line when completion is active
                    if is_cursor_line && is_active && editor.mode == Mode::Insert && editor.completion.active {
                        // Only show ghost text if cursor is at or near end of line
                        let cursor_at_end = pane.cursor.col >= line_str.chars().count().saturating_sub(1);
                        if cursor_at_end {
                            if let Some(ghost) = editor.completion.ghost_text() {
                                // Limit ghost text to remaining space
                                let remaining = pane_width.saturating_sub(chars_printed);
                                let ghost_chars: String = ghost.chars().take(remaining).collect();

                                // Render ghost text in dim gray
                                execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                                if highlight_cursor_line {
                                    execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                                }
                                print!("{}", ghost_chars);
                                execute!(self.stdout, ResetColor)?;

                                // Re-apply cursor line background for remaining fill
                                if highlight_cursor_line {
                                    execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                                }

                                chars_printed += ghost_chars.chars().count();
                            }
                        }
                    }

                    // Render inline diagnostic (virtual text) for this line
                    if is_active {
                        if let Some(diag) = editor.diagnostics_for_line(file_line).first() {
                            // Calculate remaining space for diagnostic
                            let remaining = pane_width.saturating_sub(chars_printed + 3); // 3 for " ● "
                            if remaining > 5 {
                                // Determine color based on severity
                                let (color, icon) = match diag.severity {
                                    crate::lsp::types::DiagnosticSeverity::Error => (Color::Red, "●"),
                                    crate::lsp::types::DiagnosticSeverity::Warning => (Color::Yellow, "●"),
                                    crate::lsp::types::DiagnosticSeverity::Information => (Color::Blue, "●"),
                                    crate::lsp::types::DiagnosticSeverity::Hint => (Color::Cyan, "○"),
                                };

                                // Truncate message to fit
                                let msg: String = diag.message
                                    .lines()
                                    .next()
                                    .unwrap_or(&diag.message)
                                    .chars()
                                    .take(remaining)
                                    .collect();

                                // Apply cursor line background if needed
                                if highlight_cursor_line && is_cursor_line {
                                    execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                                }

                                // Render: space, icon, space, message
                                execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                                print!(" ");
                                execute!(self.stdout, SetForegroundColor(color))?;
                                print!("{}", icon);
                                execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                                print!(" {}", msg);
                                execute!(self.stdout, ResetColor)?;

                                chars_printed += 3 + msg.chars().count();

                                // Re-apply cursor line background for fill
                                if highlight_cursor_line && is_cursor_line {
                                    execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                                }
                            }
                        }
                    }

                    // Fill remaining space in pane
                    for _ in chars_printed..pane_width {
                        print!(" ");
                    }
                }
            } else {
                // Empty line - sign column + line indicator
                print!("  "); // Empty sign column

                execute!(self.stdout, SetForegroundColor(Color::Blue))?;
                if show_line_numbers {
                    print!("{:>width$} ~", "", width = line_num_width);
                } else {
                    print!("~");
                }
                execute!(self.stdout, ResetColor)?;

                // Fill remaining space (sign column = 2)
                let chars_printed = 2 + if show_line_numbers { line_num_width + 2 } else { 1 };
                for _ in chars_printed..pane_width {
                    print!(" ");
                }
            }

            // Reset background for cursor line
            if highlight_cursor_line && is_cursor_line {
                execute!(self.stdout, ResetColor)?;
            }
        }

        Ok(())
    }

    /// Render a line segment with syntax highlighting (for wrapped lines)
    /// col_offset is the starting column in the original line
    #[allow(clippy::too_many_arguments)]
    fn render_line_segment_with_highlights(
        &mut self,
        text: &str,
        line_num: usize,
        col_offset: usize,
        highlights: &[HighlightSpan],
        visual_range: Option<(usize, usize, usize, usize)>,
        mode: &Mode,
        cursor_line_bg: bool,
        search_matches: &[(usize, usize, usize)],
    ) -> anyhow::Result<()> {
        let chars: Vec<char> = text.chars().collect();

        let search_match_bg = Color::Rgb { r: 180, g: 140, b: 40 }; // Yellow/amber for search matches
        let search_match_fg = Color::Black; // Black text on yellow background

        // Check if a column is within a search match for this line
        let in_search_match = |col: usize| -> bool {
            search_matches.iter().any(|(l, start, end)| {
                *l == line_num && col >= *start && col < *end
            })
        };

        for (i, ch) in chars.iter().enumerate() {
            // Calculate the actual column in the original line
            let actual_col = col_offset + i;

            // Check for visual selection
            let in_visual = if let Some((start_line, start_col, end_line, end_col)) = visual_range {
                match mode {
                    Mode::Visual => {
                        if line_num > start_line && line_num < end_line {
                            true
                        } else if line_num == start_line && line_num == end_line {
                            actual_col >= start_col && actual_col <= end_col
                        } else if line_num == start_line {
                            actual_col >= start_col
                        } else if line_num == end_line {
                            actual_col <= end_col
                        } else {
                            false
                        }
                    }
                    Mode::VisualLine => {
                        line_num >= start_line && line_num <= end_line
                    }
                    Mode::VisualBlock => {
                        line_num >= start_line && line_num <= end_line &&
                        actual_col >= start_col && actual_col <= end_col
                    }
                    _ => false
                }
            } else {
                false
            };

            // Check if in search match
            let is_search = in_search_match(actual_col);

            // Find syntax highlight for this position
            let syntax_color = highlights.iter()
                .find(|h| actual_col >= h.start_col && actual_col < h.end_col)
                .map(|h| h.fg);

            // Apply colors - Priority: visual selection > search match > cursor line > none
            if in_visual {
                execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 68, g: 71, b: 90 }))?;
                if let Some(color) = syntax_color {
                    execute!(self.stdout, SetForegroundColor(color))?;
                }
            } else if is_search {
                execute!(self.stdout, SetBackgroundColor(search_match_bg))?;
                execute!(self.stdout, SetForegroundColor(search_match_fg))?;
            } else if cursor_line_bg {
                execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                if let Some(color) = syntax_color {
                    execute!(self.stdout, SetForegroundColor(color))?;
                }
            } else if let Some(color) = syntax_color {
                execute!(self.stdout, SetForegroundColor(color))?;
            }

            print!("{}", ch);

            // Reset colors after each character
            if in_visual || is_search || syntax_color.is_some() {
                execute!(self.stdout, ResetColor)?;
                if cursor_line_bg && !in_visual && !is_search {
                    execute!(self.stdout, SetBackgroundColor(Color::Rgb { r: 40, g: 44, b: 52 }))?;
                }
            }
        }

        Ok(())
    }

    /// Draw separator lines between panes
    /// Render the file explorer sidebar
    fn render_explorer(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let width = editor.explorer.width as usize;
        let height = editor.text_rows();
        let is_focused = editor.mode == Mode::Explorer;

        // Colors
        let bg_color = Color::Rgb { r: 30, g: 30, b: 30 };
        let selected_bg = if is_focused {
            Color::Rgb { r: 60, g: 60, b: 80 }
        } else {
            Color::Rgb { r: 50, g: 50, b: 50 }
        };
        let dir_color = Color::Rgb { r: 100, g: 180, b: 255 };
        let file_color = Color::Rgb { r: 200, g: 200, b: 200 };
        let separator_color = Color::DarkGrey;

        // Render header with project name
        execute!(self.stdout, cursor::MoveTo(0, 0))?;
        execute!(self.stdout, SetBackgroundColor(bg_color))?;

        let project_name = editor.project_root
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Explorer".to_string());

        let header = format!(" {} ", project_name);
        let header = if header.len() > width {
            format!("{}…", &header[..width.saturating_sub(1)])
        } else {
            header
        };

        execute!(self.stdout, SetForegroundColor(Color::White))?;
        execute!(self.stdout, SetAttribute(Attribute::Bold))?;
        print!("{:width$}", header, width = width);
        execute!(self.stdout, SetAttribute(Attribute::Reset))?;

        // Calculate scrolling
        let flat_view = &editor.explorer.flat_view;
        let selected = editor.explorer.selected;
        let list_height = height.saturating_sub(1); // -1 for header

        // Calculate scroll offset to keep selection visible
        let scroll_offset = if selected < list_height / 2 {
            0
        } else if selected >= flat_view.len().saturating_sub(list_height / 2) {
            flat_view.len().saturating_sub(list_height)
        } else {
            selected.saturating_sub(list_height / 2)
        };

        // Render file tree
        for row in 0..list_height {
            let y = (row + 1) as u16; // +1 for header
            execute!(self.stdout, cursor::MoveTo(0, y))?;

            let idx = scroll_offset + row;
            if idx < flat_view.len() {
                let node = &flat_view[idx];
                let is_selected = idx == selected;

                // Set background
                if is_selected {
                    execute!(self.stdout, SetBackgroundColor(selected_bg))?;
                } else {
                    execute!(self.stdout, SetBackgroundColor(bg_color))?;
                }

                // Calculate indent (2 spaces per level, but skip root)
                let indent = if node.depth > 0 {
                    "  ".repeat(node.depth.saturating_sub(1))
                } else {
                    String::new()
                };

                // Get icon
                let icon = editor.explorer.get_icon(node);

                // Set colors
                if node.is_dir {
                    execute!(self.stdout, SetForegroundColor(dir_color))?;
                } else {
                    execute!(self.stdout, SetForegroundColor(file_color))?;
                }

                // Build the line
                let line = format!("{}{} {}", indent, icon, node.name);
                let line = if line.len() > width {
                    format!("{}…", &line[..width.saturating_sub(1)])
                } else {
                    line
                };

                print!("{:width$}", line, width = width);
            } else {
                // Empty line
                execute!(self.stdout, SetBackgroundColor(bg_color))?;
                print!("{:width$}", "", width = width);
            }
        }

        // Draw vertical separator
        execute!(self.stdout, SetBackgroundColor(Color::Reset))?;
        execute!(self.stdout, SetForegroundColor(separator_color))?;
        for y in 0..height {
            execute!(self.stdout, cursor::MoveTo(width as u16, y as u16))?;
            print!("\u{2502}"); // │
        }
        execute!(self.stdout, ResetColor)?;

        Ok(())
    }

    fn render_pane_separators(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let separator_color = Color::DarkGrey;
        let panes = editor.panes();

        match editor.split_layout() {
            SplitLayout::Vertical => {
                // Draw vertical separators between side-by-side panes
                for i in 0..panes.len().saturating_sub(1) {
                    let pane = &panes[i];
                    let separator_x = pane.rect.x + pane.rect.width;

                    // Don't draw if separator is at edge of screen
                    if separator_x >= editor.term_width {
                        continue;
                    }

                    execute!(self.stdout, SetForegroundColor(separator_color))?;
                    for y in 0..pane.rect.height {
                        execute!(self.stdout, cursor::MoveTo(separator_x, pane.rect.y + y))?;
                        print!("\u{2502}"); // │
                    }
                    execute!(self.stdout, ResetColor)?;
                }
            }
            SplitLayout::Horizontal => {
                // Draw horizontal separators between stacked panes
                for i in 0..panes.len().saturating_sub(1) {
                    let pane = &panes[i];
                    let separator_y = pane.rect.y + pane.rect.height;

                    // Don't draw if separator is at edge of text area
                    if separator_y >= editor.text_rows() as u16 {
                        continue;
                    }

                    execute!(self.stdout, SetForegroundColor(separator_color))?;
                    execute!(self.stdout, cursor::MoveTo(0, separator_y))?;
                    for _ in 0..editor.term_width {
                        print!("\u{2500}"); // ─
                    }
                    execute!(self.stdout, ResetColor)?;
                }
            }
        }

        Ok(())
    }

    /// Position the cursor based on editor mode
    fn position_cursor(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let show_line_numbers = editor.settings.editor.line_numbers;
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3);

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
            Mode::Explorer => {
                // Hide cursor in explorer mode - selection is shown visually
                execute!(self.stdout, cursor::Hide)?;
            }
            _ => {
                // Cursor in active pane's buffer
                let active_pane = &editor.panes()[editor.active_pane_idx()];
                let wrap_enabled = editor.settings.editor.wrap;
                let wrap_width = editor.settings.editor.wrap_width;

                let (cursor_row, cursor_col) = if wrap_enabled {
                    // Calculate visual position with wrapping
                    let buffer = editor.buffer();
                    // Account for sign column (2) + line numbers
                    let text_area_width = if show_line_numbers {
                        active_pane.rect.width as usize - 2 - line_num_width - 1
                    } else {
                        active_pane.rect.width as usize - 2
                    };
                    let effective_wrap_width = wrap_width.min(text_area_width);

                    // Count visual rows from viewport_offset to cursor line
                    let mut visual_row = 0;
                    for line_idx in active_pane.viewport_offset..editor.cursor.line {
                        if line_idx < buffer.len_lines() {
                            let line_content = buffer.line(line_idx)
                                .map(|l| l.to_string())
                                .unwrap_or_default();
                            let segments = calculate_wrap_segments(&line_content, effective_wrap_width, true);
                            visual_row += segments.len();
                        }
                    }

                    // Now find which segment of the cursor line contains the cursor column
                    let cursor_line_content = buffer.line(editor.cursor.line)
                        .map(|l| l.to_string())
                        .unwrap_or_default();
                    let segments = calculate_wrap_segments(&cursor_line_content, effective_wrap_width, true);

                    let mut cursor_visual_row = visual_row;
                    let mut cursor_visual_col = editor.cursor.col;

                    for (seg_idx, segment) in segments.iter().enumerate() {
                        let segment_end = if seg_idx + 1 < segments.len() {
                            segments[seg_idx + 1].start_col
                        } else {
                            cursor_line_content.chars().count()
                        };

                        if editor.cursor.col >= segment.start_col && editor.cursor.col < segment_end {
                            // Cursor is in this segment
                            cursor_visual_col = editor.cursor.col - segment.start_col;
                            // Add indentation offset for wrapped lines
                            if !segment.is_first {
                                let indent_len = cursor_line_content.chars()
                                    .take_while(|c| c.is_whitespace())
                                    .count();
                                cursor_visual_col += indent_len;
                            }
                            break;
                        }
                        cursor_visual_row += 1;
                    }

                    // Handle cursor at end of line
                    if editor.cursor.col >= cursor_line_content.trim_end_matches('\n').chars().count() {
                        cursor_visual_row = visual_row + segments.len().saturating_sub(1);
                        let last_segment = segments.last().unwrap();
                        cursor_visual_col = last_segment.text.trim_end_matches('\n').chars().count();
                    }

                    // Sign column (2) + line numbers + cursor position
                    let col = 2 + if show_line_numbers {
                        line_num_width + 1 + cursor_visual_col
                    } else {
                        cursor_visual_col
                    };

                    (cursor_visual_row, col)
                } else {
                    // Original non-wrapped calculation
                    let cursor_row = editor.cursor.line.saturating_sub(active_pane.viewport_offset);
                    // Sign column (2) + line numbers + cursor position
                    let cursor_col = 2 + if show_line_numbers {
                        line_num_width + 1 + editor.cursor.col
                    } else {
                        editor.cursor.col
                    };
                    (cursor_row, cursor_col)
                };

                // Account for pane position
                let screen_x = active_pane.rect.x as usize + cursor_col;
                let screen_y = active_pane.rect.y as usize + cursor_row;

                execute!(
                    self.stdout,
                    cursor::MoveTo(screen_x as u16, screen_y as u16),
                    cursor::Show
                )?;

                // Set cursor shape based on mode
                match editor.mode {
                    Mode::Insert => execute!(self.stdout, cursor::SetCursorStyle::BlinkingBar)?,
                    Mode::Replace => execute!(self.stdout, cursor::SetCursorStyle::BlinkingUnderScore)?,
                    Mode::Normal | Mode::Visual | Mode::VisualLine | Mode::VisualBlock | Mode::Explorer => {
                        execute!(self.stdout, cursor::SetCursorStyle::BlinkingBlock)?
                    }
                    Mode::Command | Mode::Search | Mode::Finder | Mode::RenamePrompt => {} // Handled above/separately
                }
            }
        }

        Ok(())
    }

    fn render_status_line(&mut self, editor: &Editor, _line_num_width: usize) -> anyhow::Result<()> {
        // Position at the status line row (second to last row)
        let status_row = editor.term_height.saturating_sub(2);
        execute!(self.stdout, cursor::MoveTo(0, status_row))?;

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

        // Get project name (last component of project_root)
        let project_name = editor.project_root.as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| format!("[{}] ", s))
            .unwrap_or_default();

        let left = format!(" {}{} | {}{}{} ", mode_str, pending, project_name, filename, modified);

        // Right side: LSP status, language and position
        let lsp_status = editor.lsp_status.as_deref().unwrap_or("");
        let lang = editor.syntax.language_name().unwrap_or("plain");
        let right = if lsp_status.is_empty() {
            format!(" {} | {}:{} ", lang, editor.cursor.line + 1, editor.cursor.col + 1)
        } else {
            format!(" {} | {} | {}:{} ", lsp_status, lang, editor.cursor.line + 1, editor.cursor.col + 1)
        };

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
        // Position at the command line row (last row)
        let cmd_row = editor.term_height.saturating_sub(1);
        execute!(self.stdout, cursor::MoveTo(0, cmd_row))?;
        execute!(self.stdout, terminal::Clear(ClearType::CurrentLine))?;

        if editor.mode == Mode::Command {
            // Show command line input
            print!("{}", editor.command_line.display());
        } else if editor.mode == Mode::Search {
            // Show search prompt
            print!("{}", editor.search.display());
        } else if editor.mode == Mode::RenamePrompt {
            // Show rename prompt
            execute!(self.stdout, SetForegroundColor(Color::Yellow))?;
            print!("Rename");
            execute!(self.stdout, ResetColor)?;
            print!(" '{}' → ", editor.rename_original);
            execute!(self.stdout, SetForegroundColor(Color::Green))?;
            print!("{}", editor.rename_input);
            execute!(self.stdout, ResetColor)?;
            print!("_"); // Cursor indicator
        } else if let Some(ref msg) = editor.status_message {
            // Show status message
            print!("{}", msg);
        } else if let Some(diag) = editor.diagnostic_at_cursor() {
            // Show diagnostic message when cursor is on a line with diagnostics
            let (color, prefix) = match diag.severity {
                crate::lsp::types::DiagnosticSeverity::Error => (Color::Red, "Error"),
                crate::lsp::types::DiagnosticSeverity::Warning => (Color::Yellow, "Warning"),
                crate::lsp::types::DiagnosticSeverity::Information => (Color::Blue, "Info"),
                crate::lsp::types::DiagnosticSeverity::Hint => (Color::Cyan, "Hint"),
            };
            execute!(self.stdout, SetForegroundColor(color))?;
            // Truncate message to fit terminal width
            let max_len = editor.term_width as usize - prefix.len() - 3;
            let msg = if diag.message.len() > max_len {
                format!("{}...", &diag.message[..max_len.saturating_sub(3)])
            } else {
                diag.message.clone()
            };
            print!("{}: {}", prefix, msg);
            execute!(self.stdout, ResetColor)?;
        }

        Ok(())
    }

    /// Render the completion popup with documentation
    fn render_completion(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let completion = &editor.completion;
        // Use filtered list instead of raw items
        if completion.filtered.is_empty() {
            return Ok(());
        }

        // Calculate popup position (below cursor, or above if near bottom)
        // Position at trigger_col (start of word), not current cursor position
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3);
        let popup_screen_col = (line_num_width + 1 + completion.trigger_col) as u16;
        let cursor_screen_row = (editor.cursor.line - editor.viewport_offset) as u16;

        // Calculate widths for label and detail columns (only from filtered items)
        let max_label_len = completion.filtered.iter()
            .filter_map(|&idx| completion.items.get(idx))
            .map(|item| item.label.len())
            .max()
            .unwrap_or(10)
            .min(30);
        let max_detail_len = completion.filtered.iter()
            .filter_map(|&idx| completion.items.get(idx))
            .filter_map(|item| item.detail.as_ref())
            .map(|d| d.len())
            .max()
            .unwrap_or(0)
            .min(35);

        // Popup dimensions (use filtered count)
        let max_items = 10.min(completion.filtered.len());
        let popup_height = max_items as u16 + 2; // +2 for border
        let label_col_width = max_label_len + 5; // +5 for kind and padding
        let detail_col_width = if max_detail_len > 0 { max_detail_len + 2 } else { 0 };
        let popup_width = (label_col_width + detail_col_width + 3) as u16; // +3 for borders
        let popup_width = popup_width.min(editor.term_width - 4);

        // Position popup below cursor with 1 row gap, or above if no room
        let available_below = editor.term_height.saturating_sub(cursor_screen_row + 4);
        let popup_y = if available_below >= popup_height {
            cursor_screen_row + 2  // 1 row gap below cursor line
        } else {
            cursor_screen_row.saturating_sub(popup_height + 1)  // 1 row gap above
        };
        let popup_x = popup_screen_col.min(editor.term_width.saturating_sub(popup_width + 2));

        // Colors (Zed-inspired dark theme)
        let border_color = Color::Rgb { r: 55, g: 55, b: 65 };
        let bg_color = Color::Rgb { r: 30, g: 30, b: 36 };
        let selected_bg = Color::Rgb { r: 55, g: 65, b: 95 };
        let detail_color = Color::Rgb { r: 100, g: 100, b: 115 };
        let doc_bg = Color::Rgb { r: 35, g: 35, b: 42 };

        // Draw top border (rounded corners for Zed-style)
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╭");
        for _ in 0..(popup_width - 2) {
            print!("─");
        }
        print!("╮");

        // Draw items - iterate over filtered indices
        let scroll_offset = if completion.selected >= max_items {
            completion.selected - max_items + 1
        } else {
            0
        };

        for (display_idx, &item_idx) in completion.filtered.iter().enumerate().skip(scroll_offset).take(max_items) {
            let item = match completion.items.get(item_idx) {
                Some(item) => item,
                None => continue,
            };
            let row = popup_y + 1 + (display_idx - scroll_offset) as u16;
            execute!(self.stdout, cursor::MoveTo(popup_x, row))?;

            let is_selected = display_idx == completion.selected;
            let item_bg = if is_selected { selected_bg } else { bg_color };

            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");

            execute!(self.stdout, SetBackgroundColor(item_bg))?;

            // Kind indicator (colored per-kind)
            let (r, g, b) = item.kind.color();
            let kind_color = Color::Rgb { r, g, b };
            execute!(self.stdout, SetForegroundColor(kind_color))?;
            print!(" {} ", item.kind.short_name());

            // Label (brighter when selected)
            let label_color = if is_selected {
                Color::White
            } else {
                Color::Rgb { r: 220, g: 220, b: 225 }
            };
            execute!(self.stdout, SetForegroundColor(label_color))?;
            let available_label_width = (popup_width as usize).saturating_sub(detail_col_width + 7);
            let label = if item.label.len() > available_label_width {
                format!("{}…", &item.label[..available_label_width.saturating_sub(1)])
            } else {
                format!("{:width$}", item.label, width = available_label_width)
            };
            print!("{}", label);

            // Detail/type signature (dimmed, right-aligned)
            if let Some(detail) = &item.detail {
                execute!(self.stdout, SetForegroundColor(detail_color))?;
                let detail_width = detail_col_width;
                let detail_str = if detail.len() > detail_width {
                    format!("{}…", &detail[..detail_width.saturating_sub(1)])
                } else {
                    format!("{:>width$}", detail, width = detail_width)
                };
                print!(" {}", detail_str);
            } else if detail_col_width > 0 {
                print!("{:width$}", "", width = detail_col_width + 1);
            }

            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");
        }

        // Draw bottom border (rounded corners for Zed-style)
        let bottom_row = popup_y + 1 + max_items as u16;
        execute!(self.stdout, cursor::MoveTo(popup_x, bottom_row))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╰");
        for _ in 0..(popup_width - 2) {
            print!("─");
        }
        print!("╯");

        // Draw documentation panel to the RIGHT of the completion popup
        if let Some(item) = completion.selected_item() {
            if item.detail.is_some() || item.documentation.is_some() {
                // Calculate doc panel dimensions
                let doc_width: u16 = 45; // Fixed width for doc panel
                let doc_panel_x = popup_x + popup_width + 1; // 1 char gap

                // Check if there's room on the right
                let has_room_right = doc_panel_x + doc_width < editor.term_width;

                if has_room_right {
                    // Collect content lines for the doc panel
                    let mut doc_lines: Vec<(String, Color)> = Vec::new();
                    let content_width = doc_width as usize - 4;

                    // Add type signature
                    if let Some(detail) = &item.detail {
                        // Wrap long signatures
                        let words: Vec<&str> = detail.split_whitespace().collect();
                        let mut current_line = String::new();
                        for word in words {
                            if current_line.is_empty() {
                                current_line = word.to_string();
                            } else if current_line.len() + 1 + word.len() <= content_width {
                                current_line.push(' ');
                                current_line.push_str(word);
                            } else {
                                doc_lines.push((current_line, Color::Cyan));
                                current_line = word.to_string();
                            }
                        }
                        if !current_line.is_empty() {
                            doc_lines.push((current_line, Color::Cyan));
                        }
                    }

                    // Add separator if we have both signature and docs
                    let has_separator = !doc_lines.is_empty() && item.documentation.is_some();

                    // Add documentation
                    if let Some(docs) = &item.documentation {
                        // Clean up markdown: remove code block markers
                        let clean_docs = docs
                            .lines()
                            .filter(|line| !line.starts_with("```"))
                            .collect::<Vec<_>>()
                            .join("\n");

                        for line in clean_docs.lines().take(10) {
                            // Skip empty lines at the start
                            if doc_lines.is_empty() && line.trim().is_empty() {
                                continue;
                            }
                            // Wrap long lines
                            if line.len() <= content_width {
                                doc_lines.push((line.to_string(), Color::Rgb { r: 180, g: 180, b: 180 }));
                            } else {
                                // Simple word wrap
                                let words: Vec<&str> = line.split_whitespace().collect();
                                let mut current_line = String::new();
                                for word in words {
                                    if current_line.is_empty() {
                                        current_line = word.to_string();
                                    } else if current_line.len() + 1 + word.len() <= content_width {
                                        current_line.push(' ');
                                        current_line.push_str(word);
                                    } else {
                                        doc_lines.push((current_line, Color::Rgb { r: 180, g: 180, b: 180 }));
                                        current_line = word.to_string();
                                    }
                                }
                                if !current_line.is_empty() {
                                    doc_lines.push((current_line, Color::Rgb { r: 180, g: 180, b: 180 }));
                                }
                            }
                        }
                    }

                    if !doc_lines.is_empty() {
                        // Calculate separator position (after signature lines, before doc lines)
                        let sig_line_count = if item.detail.is_some() {
                            doc_lines.iter().take_while(|(_, c)| *c == Color::Cyan).count()
                        } else {
                            0
                        };

                        // Doc panel height: content + 2 for borders + 1 for separator if needed
                        let separator_height = if has_separator { 1 } else { 0 };
                        let doc_height = (doc_lines.len() as u16 + 2 + separator_height).min(popup_height + 4);
                        let available_height = editor.term_height.saturating_sub(popup_y + 2);
                        let doc_height = doc_height.min(available_height);

                        // Draw doc panel with rounded corners
                        // Top border
                        execute!(self.stdout, cursor::MoveTo(doc_panel_x, popup_y))?;
                        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(doc_bg))?;
                        print!("╭");
                        for _ in 0..(doc_width - 2) {
                            print!("─");
                        }
                        print!("╮");

                        // Content lines
                        let mut row_offset = 1u16;
                        let max_content_lines = doc_height.saturating_sub(2) as usize;
                        let mut lines_drawn = 0;

                        for (idx, (line, color)) in doc_lines.iter().enumerate().take(max_content_lines) {
                            // Insert separator after signature lines
                            if has_separator && idx == sig_line_count && lines_drawn < max_content_lines {
                                execute!(self.stdout, cursor::MoveTo(doc_panel_x, popup_y + row_offset))?;
                                execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(doc_bg))?;
                                print!("├");
                                for _ in 0..(doc_width - 2) {
                                    print!("─");
                                }
                                print!("┤");
                                row_offset += 1;
                                lines_drawn += 1;
                                if lines_drawn >= max_content_lines {
                                    break;
                                }
                            }

                            execute!(self.stdout, cursor::MoveTo(doc_panel_x, popup_y + row_offset))?;
                            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(doc_bg))?;
                            print!("│");
                            execute!(self.stdout, SetForegroundColor(*color))?;
                            let padded = format!(" {:width$}", line, width = content_width);
                            print!("{}", &padded[..padded.len().min(content_width + 1)]);
                            execute!(self.stdout, SetForegroundColor(border_color))?;
                            print!(" │");
                            row_offset += 1;
                            lines_drawn += 1;
                        }

                        // Bottom border
                        execute!(self.stdout, cursor::MoveTo(doc_panel_x, popup_y + row_offset))?;
                        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(doc_bg))?;
                        print!("╰");
                        for _ in 0..(doc_width - 2) {
                            print!("─");
                        }
                        print!("╯");
                    }
                }
            }
        }

        execute!(self.stdout, ResetColor)?;
        Ok(())
    }

    /// Parse hover content into structured sections (code blocks and text)
    fn parse_hover_content(content: &str) -> Vec<HoverSection> {
        let mut sections = Vec::new();
        let mut current_text = String::new();
        let mut in_code_block = false;
        let mut code_block_lang = String::new();
        let mut code_lines = Vec::new();

        for line in content.lines() {
            if line.starts_with("```") {
                if in_code_block {
                    // End of code block
                    if !code_lines.is_empty() {
                        sections.push(HoverSection::Code {
                            language: code_block_lang.clone(),
                            lines: code_lines.clone(),
                        });
                    }
                    code_lines.clear();
                    code_block_lang.clear();
                    in_code_block = false;
                } else {
                    // Start of code block - save any pending text
                    let trimmed = current_text.trim();
                    if !trimmed.is_empty() {
                        sections.push(HoverSection::Text(trimmed.to_string()));
                    }
                    current_text.clear();
                    code_block_lang = line.trim_start_matches('`').to_string();
                    in_code_block = true;
                }
            } else if in_code_block {
                code_lines.push(line.to_string());
            } else {
                if !current_text.is_empty() {
                    current_text.push('\n');
                }
                current_text.push_str(line);
            }
        }

        // Handle any remaining content
        if in_code_block && !code_lines.is_empty() {
            sections.push(HoverSection::Code {
                language: code_block_lang,
                lines: code_lines,
            });
        } else {
            let trimmed = current_text.trim();
            if !trimmed.is_empty() {
                sections.push(HoverSection::Text(trimmed.to_string()));
            }
        }

        sections
    }

    /// Render the hover documentation popup (Neovim-style)
    fn render_hover(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let content = match &editor.hover_content {
            Some(c) => c,
            None => return Ok(()),
        };

        // Parse content into sections
        let sections = Self::parse_hover_content(content);
        if sections.is_empty() {
            return Ok(());
        }

        // Build display lines with their types
        let mut display_lines: Vec<(String, HoverLineType)> = Vec::new();

        for (section_idx, section) in sections.iter().enumerate() {
            // Add separator between sections (except before first)
            if section_idx > 0 && !display_lines.is_empty() {
                display_lines.push(("".to_string(), HoverLineType::Separator));
            }

            match section {
                HoverSection::Code { lines, .. } => {
                    for line in lines {
                        display_lines.push((line.clone(), HoverLineType::Code));
                    }
                }
                HoverSection::Text(text) => {
                    for line in text.lines() {
                        display_lines.push((line.to_string(), HoverLineType::Text));
                    }
                }
            }
        }

        // Calculate dimensions
        let max_line_len = display_lines
            .iter()
            .map(|(l, _)| l.chars().count())
            .max()
            .unwrap_or(20);
        let popup_width = (max_line_len + 4).min(80).max(40) as u16;
        let popup_height = (display_lines.len() + 2).min(20) as u16;

        // Calculate popup position (above cursor if possible)
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3);
        let cursor_screen_col = (line_num_width + 1 + editor.cursor.col) as u16;
        let cursor_screen_row = (editor.cursor.line - editor.viewport_offset) as u16;

        let popup_y = if cursor_screen_row >= popup_height + 1 {
            cursor_screen_row - popup_height
        } else {
            (cursor_screen_row + 1).min(editor.term_height.saturating_sub(popup_height + 1))
        };
        let popup_x = cursor_screen_col.saturating_sub(2).min(editor.term_width.saturating_sub(popup_width + 1));

        // Colors (Neovim-inspired)
        let border_color = Color::Rgb { r: 90, g: 90, b: 120 };
        let bg_color = Color::Rgb { r: 25, g: 25, b: 35 };
        let code_bg = Color::Rgb { r: 35, g: 35, b: 50 };
        let text_color = Color::Rgb { r: 200, g: 200, b: 210 };
        let code_color = Color::Rgb { r: 150, g: 200, b: 255 }; // Blue for signatures
        let keyword_color = Color::Rgb { r: 255, g: 150, b: 150 }; // Red/pink for keywords
        let type_color = Color::Rgb { r: 180, g: 220, b: 180 }; // Green for types
        let separator_color = Color::Rgb { r: 70, g: 70, b: 90 };

        // Draw top border (rounded corners)
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╭");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╮");

        // Draw content lines
        let content_width = (popup_width - 4) as usize;
        let max_lines = (popup_height - 2) as usize;

        for (i, (line, line_type)) in display_lines.iter().take(max_lines).enumerate() {
            let row = popup_y + 1 + i as u16;
            execute!(self.stdout, cursor::MoveTo(popup_x, row))?;

            match line_type {
                HoverLineType::Code => {
                    execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(code_bg))?;
                    print!("│ ");
                    // Simple syntax highlighting for Rust
                    self.render_hover_code_line(line, content_width, code_color, keyword_color, type_color, code_bg)?;
                    execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(code_bg))?;
                    print!(" │");
                }
                HoverLineType::Text => {
                    execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
                    print!("│ ");
                    execute!(self.stdout, SetForegroundColor(text_color))?;
                    let display = if line.chars().count() > content_width {
                        format!("{}…", line.chars().take(content_width.saturating_sub(1)).collect::<String>())
                    } else {
                        format!("{:width$}", line, width = content_width)
                    };
                    print!("{}", display);
                    execute!(self.stdout, SetForegroundColor(border_color))?;
                    print!(" │");
                }
                HoverLineType::Separator => {
                    execute!(self.stdout, SetForegroundColor(separator_color), SetBackgroundColor(bg_color))?;
                    print!("├");
                    for _ in 1..(popup_width - 1) {
                        print!("─");
                    }
                    print!("┤");
                }
            }
        }

        // Fill remaining rows if content is shorter
        for i in display_lines.len()..max_lines {
            let row = popup_y + 1 + i as u16;
            execute!(self.stdout, cursor::MoveTo(popup_x, row))?;
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│ {:width$} │", "", width = content_width);
        }

        // Draw bottom border (rounded corners)
        let bottom_row = popup_y + popup_height - 1;
        execute!(self.stdout, cursor::MoveTo(popup_x, bottom_row))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╰");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, ResetColor)?;
        Ok(())
    }

    /// Render a code line with simple syntax highlighting
    fn render_hover_code_line(
        &mut self,
        line: &str,
        width: usize,
        default_color: Color,
        keyword_color: Color,
        type_color: Color,
        bg_color: Color,
    ) -> anyhow::Result<()> {
        let rust_keywords = [
            "fn", "pub", "let", "mut", "const", "static", "struct", "enum", "impl",
            "trait", "where", "for", "loop", "while", "if", "else", "match", "return",
            "async", "await", "unsafe", "mod", "use", "crate", "self", "Self", "super",
            "dyn", "ref", "move", "type", "as", "in",
        ];

        let mut chars_printed = 0;
        let mut i = 0;
        let chars: Vec<char> = line.chars().collect();

        while i < chars.len() && chars_printed < width {
            // Try to match a word
            if chars[i].is_alphabetic() || chars[i] == '_' {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();

                // Determine color based on word type
                let color = if rust_keywords.contains(&word.as_str()) {
                    keyword_color
                } else if word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    type_color // Types typically start with uppercase
                } else {
                    default_color
                };

                execute!(self.stdout, SetForegroundColor(color), SetBackgroundColor(bg_color))?;
                let remaining = width - chars_printed;
                if word.len() <= remaining {
                    print!("{}", word);
                    chars_printed += word.len();
                } else {
                    print!("{}", &word[..remaining]);
                    chars_printed = width;
                }
            } else {
                // Print punctuation/symbols in default color
                execute!(self.stdout, SetForegroundColor(default_color), SetBackgroundColor(bg_color))?;
                print!("{}", chars[i]);
                chars_printed += 1;
                i += 1;
            }
        }

        // Pad remaining space
        if chars_printed < width {
            execute!(self.stdout, SetForegroundColor(default_color), SetBackgroundColor(bg_color))?;
            print!("{:width$}", "", width = width - chars_printed);
        }

        Ok(())
    }

    /// Render the signature help popup above the cursor
    fn render_signature_help(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let help = match &editor.signature_help {
            Some(h) => h,
            None => return Ok(()),
        };

        if help.signatures.is_empty() {
            return Ok(());
        }

        // Get the active signature
        let active_idx = help.active_signature.min(help.signatures.len() - 1);
        let signature = &help.signatures[active_idx];

        // Calculate popup position (above cursor)
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3);
        let cursor_screen_col = (line_num_width + 1 + editor.cursor.col) as u16;
        let cursor_screen_row = (editor.cursor.line - editor.viewport_offset) as u16;

        // Calculate dimensions based on signature
        let popup_width = (signature.label.chars().count() + 4).min(80).max(30) as u16;
        let popup_height = 3u16; // Single line signature + borders

        // Position popup above cursor if possible
        let popup_y = if cursor_screen_row >= popup_height {
            cursor_screen_row - popup_height
        } else {
            cursor_screen_row + 1
        };
        let popup_x = cursor_screen_col.saturating_sub(2).min(editor.term_width.saturating_sub(popup_width + 1));

        // Colors
        let border_color = Color::Rgb { r: 100, g: 100, b: 140 };
        let bg_color = Color::Rgb { r: 30, g: 30, b: 45 };
        let text_color = Color::Rgb { r: 200, g: 200, b: 210 };
        let highlight_color = Color::Rgb { r: 255, g: 200, b: 100 }; // Yellow for active param

        // Draw top border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╭");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╮");

        // Draw signature with highlighted parameter
        let content_width = (popup_width - 4) as usize;
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 1))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("│ ");

        // Render the signature with active parameter highlighted
        let label = &signature.label;
        let active_param = help.active_parameter;

        // Find the active parameter offsets
        let highlight_range = active_param.and_then(|idx| {
            signature.parameters.get(idx).and_then(|p| p.label_offsets)
        });

        let mut chars_printed = 0;
        let label_chars: Vec<char> = label.chars().collect();
        let mut i = 0;

        while i < label_chars.len() && chars_printed < content_width {
            let in_highlight = highlight_range
                .map(|(start, end)| i >= start && i < end)
                .unwrap_or(false);

            if in_highlight {
                execute!(self.stdout, SetForegroundColor(highlight_color), SetBackgroundColor(bg_color))?;
            } else {
                execute!(self.stdout, SetForegroundColor(text_color), SetBackgroundColor(bg_color))?;
            }

            print!("{}", label_chars[i]);
            chars_printed += 1;
            i += 1;
        }

        // Pad remaining space
        if chars_printed < content_width {
            execute!(self.stdout, SetForegroundColor(text_color), SetBackgroundColor(bg_color))?;
            print!("{:width$}", "", width = content_width - chars_printed);
        }

        execute!(self.stdout, SetForegroundColor(border_color))?;
        print!(" │");

        // Draw bottom border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 2))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╰");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, ResetColor)?;
        Ok(())
    }

    /// Render the references picker as a floating popup
    fn render_references_picker(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let picker = match &editor.references_picker {
            Some(p) => p,
            None => return Ok(()),
        };

        if picker.items.is_empty() {
            return Ok(());
        }

        // Calculate popup dimensions
        let max_width = 80u16;
        let max_height = 15u16;
        let popup_width = max_width.min(editor.term_width.saturating_sub(4));
        let popup_height = (picker.items.len() as u16 + 2).min(max_height);

        // Center the popup
        let popup_x = (editor.term_width.saturating_sub(popup_width)) / 2;
        let popup_y = (editor.term_height.saturating_sub(popup_height)) / 2;

        // Colors
        let border_color = Color::Rgb { r: 100, g: 140, b: 180 };
        let bg_color = Color::Rgb { r: 25, g: 25, b: 30 };
        let selected_bg = Color::Rgb { r: 50, g: 70, b: 100 };
        let text_color = Color::Rgb { r: 200, g: 200, b: 210 };
        let file_color = Color::Rgb { r: 130, g: 180, b: 250 };
        let _line_num_color = Color::Rgb { r: 180, g: 180, b: 100 };

        // Draw top border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        let title = " References ";
        let title_start = (popup_width as usize - title.len()) / 2;
        print!("╭");
        for i in 1..(popup_width - 1) {
            if i as usize == title_start {
                print!("{}", title);
            } else if i as usize > title_start && i as usize <= title_start + title.len() {
                // Skip - part of title
            } else {
                print!("─");
            }
        }
        print!("╮");

        // Calculate visible items
        let visible_count = (popup_height - 2) as usize;
        let scroll_offset = if picker.selected >= visible_count {
            picker.selected - visible_count + 1
        } else {
            0
        };

        // Draw items
        for (i, idx) in (scroll_offset..(scroll_offset + visible_count)).enumerate() {
            execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 1 + i as u16))?;

            if idx >= picker.items.len() {
                // Empty line
                execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
                print!("│{:width$}│", "", width = (popup_width - 2) as usize);
                continue;
            }

            let loc = &picker.items[idx];
            let is_selected = idx == picker.selected;

            let current_bg = if is_selected { selected_bg } else { bg_color };

            // Border
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");

            // Item content
            execute!(self.stdout, SetBackgroundColor(current_bg))?;

            // Format: filename:line:col
            let path_str = crate::lsp::uri_to_path(&loc.uri)
                .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
                .unwrap_or_else(|| loc.uri.clone());

            let content = format!("{}:{}:{}", path_str, loc.line + 1, loc.col + 1);
            let content_width = (popup_width - 4) as usize;

            // Print with colors
            execute!(self.stdout, SetForegroundColor(file_color))?;
            let truncated: String = content.chars().take(content_width).collect();
            print!(" {}", truncated);

            // Pad
            let printed = truncated.len() + 1;
            if printed < content_width + 1 {
                execute!(self.stdout, SetForegroundColor(text_color))?;
                print!("{:width$}", "", width = content_width + 1 - printed);
            }

            // Closing border
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");
        }

        // Draw bottom border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + popup_height - 1))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╰");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, ResetColor)?;
        Ok(())
    }

    /// Render the code actions picker as a floating popup
    fn render_code_actions_picker(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let picker = match &editor.code_actions_picker {
            Some(p) => p,
            None => return Ok(()),
        };

        if picker.items.is_empty() {
            return Ok(());
        }

        // Calculate popup dimensions based on content
        let max_title_len = picker.items.iter()
            .map(|a| a.title.len())
            .max()
            .unwrap_or(20);

        let max_width = 60u16;
        let max_height = 12u16;
        let popup_width = (max_title_len as u16 + 6).min(max_width).min(editor.term_width.saturating_sub(4));
        let popup_height = (picker.items.len() as u16 + 2).min(max_height);

        // Position near cursor
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3);
        let cursor_screen_col = (2 + line_num_width + 1 + editor.cursor.col) as u16;
        let cursor_screen_row = (editor.cursor.line - editor.viewport_offset) as u16;

        let popup_x = cursor_screen_col.min(editor.term_width.saturating_sub(popup_width + 2));
        let popup_y = if cursor_screen_row + popup_height + 1 < editor.term_height {
            cursor_screen_row + 1
        } else {
            cursor_screen_row.saturating_sub(popup_height)
        };

        // Colors
        let border_color = Color::Rgb { r: 140, g: 100, b: 180 };
        let bg_color = Color::Rgb { r: 25, g: 25, b: 30 };
        let selected_bg = Color::Rgb { r: 70, g: 50, b: 100 };
        let text_color = Color::Rgb { r: 200, g: 200, b: 210 };
        let _kind_color = Color::Rgb { r: 130, g: 180, b: 130 };
        let preferred_color = Color::Rgb { r: 255, g: 200, b: 100 };

        // Draw top border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        let title = " Code Actions ";
        let title_start = (popup_width as usize - title.len()) / 2;
        print!("╭");
        for i in 1..(popup_width - 1) {
            if i as usize == title_start {
                print!("{}", title);
            } else if i as usize > title_start && i as usize <= title_start + title.len() {
                // Skip
            } else {
                print!("─");
            }
        }
        print!("╮");

        // Calculate visible items
        let visible_count = (popup_height - 2) as usize;
        let scroll_offset = if picker.selected >= visible_count {
            picker.selected - visible_count + 1
        } else {
            0
        };

        // Draw items
        for (i, idx) in (scroll_offset..(scroll_offset + visible_count)).enumerate() {
            execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 1 + i as u16))?;

            if idx >= picker.items.len() {
                execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
                print!("│{:width$}│", "", width = (popup_width - 2) as usize);
                continue;
            }

            let action = &picker.items[idx];
            let is_selected = idx == picker.selected;

            let current_bg = if is_selected { selected_bg } else { bg_color };

            // Border
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");

            // Item content
            execute!(self.stdout, SetBackgroundColor(current_bg))?;

            let content_width = (popup_width - 4) as usize;

            // Preferred marker
            if action.is_preferred {
                execute!(self.stdout, SetForegroundColor(preferred_color))?;
                print!("★ ");
            } else {
                print!("  ");
            }

            // Title
            execute!(self.stdout, SetForegroundColor(text_color))?;
            let title_display: String = action.title.chars().take(content_width - 2).collect();
            print!("{}", title_display);

            // Pad
            let printed = title_display.len() + 2;
            if printed < content_width {
                print!("{:width$}", "", width = content_width - printed);
            }

            // Closing border
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");
        }

        // Draw bottom border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + popup_height - 1))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╰");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, ResetColor)?;
        Ok(())
    }

    /// Render the harpoon menu floating window
    fn render_harpoon_menu(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let files = editor.harpoon.files();
        let selected = editor.harpoon.menu_selection;

        // Calculate popup dimensions
        let popup_width = 50u16.min(editor.term_width.saturating_sub(4));
        let popup_height = (files.len() as u16 + 4).max(6).min(12);

        // Center the popup
        let popup_x = (editor.term_width.saturating_sub(popup_width)) / 2;
        let popup_y = (editor.term_height.saturating_sub(popup_height)) / 2;

        // Colors
        let border_color = Color::Rgb { r: 100, g: 150, b: 200 };
        let bg_color = Color::Rgb { r: 25, g: 25, b: 30 };
        let selected_bg = Color::Rgb { r: 50, g: 80, b: 120 };
        let text_color = Color::Rgb { r: 200, g: 200, b: 210 };
        let slot_color = Color::Rgb { r: 150, g: 200, b: 150 };
        let empty_color = Color::Rgb { r: 100, g: 100, b: 110 };

        // Draw top border with title
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        let title = " Harpoon ";
        let title_start = (popup_width as usize - title.len()) / 2;
        print!("╭");
        for i in 1..(popup_width - 1) {
            if i as usize == title_start {
                print!("{}", title);
            } else if i as usize > title_start && i as usize <= title_start + title.len() {
                // Skip - title already printed
            } else {
                print!("─");
            }
        }
        print!("╮");

        // Draw file slots (always show 4 slots)
        for slot in 0..4usize {
            execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 1 + slot as u16))?;

            let is_selected = slot == selected && slot < files.len();
            let current_bg = if is_selected { selected_bg } else { bg_color };

            // Left border
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");

            execute!(self.stdout, SetBackgroundColor(current_bg))?;

            // Slot number
            execute!(self.stdout, SetForegroundColor(slot_color))?;
            print!(" {} ", slot + 1);

            // File path or empty
            let content_width = (popup_width - 7) as usize;
            if let Some(path) = files.get(slot) {
                execute!(self.stdout, SetForegroundColor(text_color))?;
                // Show just the filename, or relative path if possible
                let display = path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string());
                let truncated: String = if display.len() > content_width {
                    format!("…{}", &display[display.len() - content_width + 1..])
                } else {
                    display
                };
                print!("{:<width$}", truncated, width = content_width);
            } else {
                execute!(self.stdout, SetForegroundColor(empty_color))?;
                print!("{:<width$}", "(empty)", width = content_width);
            }

            // Right border
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");
        }

        // Draw help line
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 5))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("│");
        execute!(self.stdout, SetForegroundColor(empty_color), SetBackgroundColor(bg_color))?;
        let help = "[1-4] jump  [d] delete  [q] close";
        let help_truncated: String = help.chars().take((popup_width - 2) as usize).collect();
        print!("{:^width$}", help_truncated, width = (popup_width - 2) as usize);
        execute!(self.stdout, SetForegroundColor(border_color))?;
        print!("│");

        // Draw bottom border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 6))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╰");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, ResetColor)?;
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
        search_matches: &[(usize, usize, usize)],
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
        let selection_bg = Color::DarkBlue;
        let search_match_bg = Color::Rgb { r: 180, g: 140, b: 40 }; // Yellow/amber for search matches
        let search_match_fg = Color::Black; // Black text on yellow background

        // Check if a column is within a search match for this line
        let in_search_match = |col: usize| -> bool {
            search_matches.iter().any(|(l, start, end)| {
                *l == line_idx && col >= *start && col < *end
            })
        };

        // Render character by character
        let mut highlight_idx = 0;
        let mut current_fg: Option<Color> = None;
        let mut current_bg: Option<Color> = None;
        for (col, ch) in chars.iter().enumerate() {
            // Find syntax color for this column
            let syntax_color = Self::get_syntax_color_at(highlights, col, &mut highlight_idx);

            // Check if in visual selection
            let is_selected = in_selection && col >= sel_start && col < sel_end;

            // Check if in search match
            let is_search_match = in_search_match(col);

            // Priority: visual selection > search match > cursor line > none
            let (desired_bg, desired_fg) = if is_selected {
                (Some(selection_bg), syntax_color)
            } else if is_search_match {
                (Some(search_match_bg), Some(search_match_fg))
            } else if is_cursor_line {
                (Some(cursor_line_bg), syntax_color)
            } else {
                (None, syntax_color)
            };

            if desired_bg != current_bg || desired_fg != current_fg {
                execute!(self.stdout, ResetColor)?;
                current_bg = None;
                current_fg = None;
                if let Some(bg) = desired_bg {
                    execute!(self.stdout, SetBackgroundColor(bg))?;
                    current_bg = Some(bg);
                }
                if let Some(fg) = desired_fg {
                    execute!(self.stdout, SetForegroundColor(fg))?;
                    current_fg = Some(fg);
                }
            }

            print!("{}", ch);
        }

        // Handle selection extending past line end
        if in_selection && sel_end > line_len {
            execute!(self.stdout, SetBackgroundColor(selection_bg))?;
            print!(" ");
            execute!(self.stdout, ResetColor)?;
        }
        execute!(self.stdout, ResetColor)?;

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

    /// Read a key event (blocking for the next event)
    pub fn read_key(&self) -> anyhow::Result<Option<KeyEvent>> {
        if let Event::Key(key_event) = event::read()? {
            Ok(Some(key_event))
        } else {
            Ok(None)
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

/// Handle key input for the references picker
fn handle_references_picker_key(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Close picker
        (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
            editor.hide_references_picker();
        }

        // Navigate up
        (KeyModifiers::NONE, KeyCode::Up) | (KeyModifiers::NONE, KeyCode::Char('k')) => {
            if let Some(ref mut picker) = editor.references_picker {
                picker.move_up();
            }
        }

        // Navigate down
        (KeyModifiers::NONE, KeyCode::Down) | (KeyModifiers::NONE, KeyCode::Char('j')) => {
            if let Some(ref mut picker) = editor.references_picker {
                picker.move_down();
            }
        }

        // Select and jump
        (KeyModifiers::NONE, KeyCode::Enter) => {
            if let Some(picker) = editor.references_picker.take() {
                if let Some(loc) = picker.items.get(picker.selected) {
                    if let Some(path) = crate::lsp::uri_to_path(&loc.uri) {
                        editor.record_jump();
                        // Open the file if different
                        let current_path = editor.buffer().path.clone();
                        if current_path.as_ref() != Some(&path) {
                            let _ = editor.open_file(path);
                        }
                        editor.goto_line(loc.line + 1);
                        editor.cursor.col = loc.col;
                        editor.scroll_to_cursor();
                    }
                }
            }
        }

        _ => {}
    }
}

/// Handle key input for the code actions picker
fn handle_code_actions_picker_key(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Close picker
        (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
            editor.hide_code_actions_picker();
        }

        // Navigate up
        (KeyModifiers::NONE, KeyCode::Up) | (KeyModifiers::NONE, KeyCode::Char('k')) => {
            if let Some(ref mut picker) = editor.code_actions_picker {
                picker.move_up();
            }
        }

        // Navigate down
        (KeyModifiers::NONE, KeyCode::Down) | (KeyModifiers::NONE, KeyCode::Char('j')) => {
            if let Some(ref mut picker) = editor.code_actions_picker {
                picker.move_down();
            }
        }

        // Apply selected action
        (KeyModifiers::NONE, KeyCode::Enter) => {
            if let Some(msg) = editor.apply_selected_code_action() {
                editor.set_status(msg);
            }
        }

        _ => {}
    }
}

/// Handle key input for the harpoon menu
fn handle_harpoon_menu_key(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Close menu
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) |
        (KeyModifiers::NONE, KeyCode::Char('q')) => {
            editor.harpoon.close_menu();
        }

        // Navigate up
        (KeyModifiers::NONE, KeyCode::Up) | (KeyModifiers::NONE, KeyCode::Char('k')) => {
            editor.harpoon.menu_up();
        }

        // Navigate down
        (KeyModifiers::NONE, KeyCode::Down) | (KeyModifiers::NONE, KeyCode::Char('j')) => {
            editor.harpoon.menu_down();
        }

        // Jump to slot by number (1-4)
        (KeyModifiers::NONE, KeyCode::Char('1')) => {
            if let Some(path) = editor.harpoon.get_slot(1).cloned() {
                editor.harpoon.close_menu();
                let _ = editor.open_file(path);
            }
        }
        (KeyModifiers::NONE, KeyCode::Char('2')) => {
            if let Some(path) = editor.harpoon.get_slot(2).cloned() {
                editor.harpoon.close_menu();
                let _ = editor.open_file(path);
            }
        }
        (KeyModifiers::NONE, KeyCode::Char('3')) => {
            if let Some(path) = editor.harpoon.get_slot(3).cloned() {
                editor.harpoon.close_menu();
                let _ = editor.open_file(path);
            }
        }
        (KeyModifiers::NONE, KeyCode::Char('4')) => {
            if let Some(path) = editor.harpoon.get_slot(4).cloned() {
                editor.harpoon.close_menu();
                let _ = editor.open_file(path);
            }
        }

        // Open selected file (Enter)
        (KeyModifiers::NONE, KeyCode::Enter) => {
            if let Some(path) = editor.harpoon.menu_selected_file().cloned() {
                editor.harpoon.close_menu();
                let _ = editor.open_file(path);
            }
        }

        // Delete selected (d or x)
        (KeyModifiers::NONE, KeyCode::Char('d')) |
        (KeyModifiers::NONE, KeyCode::Char('x')) => {
            if editor.harpoon.remove_selected() {
                editor.set_status("Removed from harpoon");
            }
        }

        // Move selected up (K)
        (KeyModifiers::SHIFT, KeyCode::Char('K')) => {
            editor.harpoon.move_up();
        }

        // Move selected down (J)
        (KeyModifiers::SHIFT, KeyCode::Char('J')) => {
            editor.harpoon.move_down();
        }

        _ => {}
    }
}

/// Handle a key event and update editor state
pub fn handle_key(editor: &mut Editor, key: KeyEvent) {
    // Handle references picker if active
    if editor.references_picker.is_some() {
        handle_references_picker_key(editor, key);
        return;
    }

    // Handle code actions picker if active
    if editor.code_actions_picker.is_some() {
        handle_code_actions_picker_key(editor, key);
        return;
    }

    // Handle harpoon menu if active
    if editor.harpoon.menu_open {
        handle_harpoon_menu_key(editor, key);
        return;
    }

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
        Mode::Replace => handle_replace_mode(editor, key),
        Mode::Command => handle_command_mode(editor, key),
        Mode::Search => handle_search_mode(editor, key),
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => handle_visual_mode(editor, key),
        Mode::Finder => handle_finder_mode(editor, key),
        Mode::Explorer => handle_explorer_mode(editor, key),
        Mode::RenamePrompt => handle_rename_prompt_mode(editor, key),
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

    // Check for normal mode custom mapping first
    if let Some(mapping) = editor.keymap.get_normal_mapping(key) {
        let mapping = mapping.clone();
        execute_leader_action(editor, &mapping);
        return;
    }

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

        KeyAction::EnterReplace => {
            editor.enter_replace_mode();
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

        KeyAction::GotoDefinition => {
            editor.pending_lsp_action = Some(crate::editor::LspAction::GotoDefinition);
        }

        KeyAction::Hover => {
            editor.pending_lsp_action = Some(crate::editor::LspAction::Hover);
        }

        KeyAction::FindReferences => {
            editor.pending_lsp_action = Some(crate::editor::LspAction::FindReferences);
        }

        KeyAction::CodeActions => {
            editor.pending_lsp_action = Some(crate::editor::LspAction::CodeActions);
        }

        KeyAction::RenameSymbol => {
            // Enter rename prompt mode
            editor.enter_rename_prompt();
        }

        KeyAction::JumpBack => {
            if !editor.jump_back() {
                editor.set_status("Already at oldest position");
            }
        }

        KeyAction::JumpForward => {
            if !editor.jump_forward() {
                editor.set_status("Already at newest position");
            }
        }

        KeyAction::NextDiagnostic => {
            if editor.goto_next_diagnostic() {
                // Show the diagnostic message in status
                if let Some(diag) = editor.diagnostic_at_cursor() {
                    let prefix = match diag.severity {
                        crate::lsp::types::DiagnosticSeverity::Error => "Error",
                        crate::lsp::types::DiagnosticSeverity::Warning => "Warning",
                        crate::lsp::types::DiagnosticSeverity::Information => "Info",
                        crate::lsp::types::DiagnosticSeverity::Hint => "Hint",
                    };
                    editor.set_status(format!("{}: {}", prefix, diag.message));
                }
            } else {
                editor.set_status("No diagnostics");
            }
        }

        KeyAction::PrevDiagnostic => {
            if editor.goto_prev_diagnostic() {
                // Show the diagnostic message in status
                if let Some(diag) = editor.diagnostic_at_cursor() {
                    let prefix = match diag.severity {
                        crate::lsp::types::DiagnosticSeverity::Error => "Error",
                        crate::lsp::types::DiagnosticSeverity::Warning => "Warning",
                        crate::lsp::types::DiagnosticSeverity::Information => "Info",
                        crate::lsp::types::DiagnosticSeverity::Hint => "Hint",
                    };
                    editor.set_status(format!("{}: {}", prefix, diag.message));
                }
            } else {
                editor.set_status("No diagnostics");
            }
        }

        KeyAction::DeleteSurround(surround_char) => {
            editor.delete_surrounding(surround_char);
        }

        KeyAction::ChangeSurround(old_char, new_char) => {
            editor.change_surrounding(old_char, new_char);
        }

        KeyAction::AddSurround(text_object, surround_char) => {
            editor.add_surrounding(text_object, surround_char);
        }

        KeyAction::ToggleCommentLine => {
            editor.toggle_comment_line();
        }

        KeyAction::ToggleCommentMotion(motion, count) => {
            // Calculate the line range based on the motion
            let start_line = editor.cursor.line;
            let (end_line, _) = crate::input::apply_motion(
                editor.buffer(),
                motion,
                editor.cursor.line,
                editor.cursor.col,
                count,
                editor.text_rows(),
            ).unwrap_or((start_line, 0));

            let (first, last) = if start_line <= end_line {
                (start_line, end_line)
            } else {
                (end_line, start_line)
            };

            editor.toggle_comment_lines(first, last);
        }

        KeyAction::ToggleCommentVisual => {
            let (start_line, _, end_line, _) = editor.get_visual_range();
            let (first, last) = if start_line <= end_line {
                (start_line, end_line)
            } else {
                (end_line, start_line)
            };
            editor.toggle_comment_lines(first, last);
            editor.enter_normal_mode();
        }

        KeyAction::HarpoonAdd => {
            if let Some(path) = editor.buffer().path.clone() {
                let msg = editor.harpoon.add_file(&path);
                editor.set_status(msg);
            } else {
                editor.set_status("Cannot add unsaved buffer to harpoon");
            }
        }

        KeyAction::HarpoonMenu => {
            editor.harpoon.toggle_menu();
        }

        KeyAction::HarpoonJump(slot) => {
            if let Some(path) = editor.harpoon.get_slot(slot).cloned() {
                if let Err(e) = editor.open_file(path) {
                    editor.set_status(format!("Error opening file: {}", e));
                }
            } else {
                editor.set_status(format!("Harpoon slot {} is empty", slot));
            }
        }

        KeyAction::HarpoonNext => {
            if let Some(path) = editor.harpoon.next().cloned() {
                if let Err(e) = editor.open_file(path) {
                    editor.set_status(format!("Error opening file: {}", e));
                }
            } else {
                editor.set_status("Harpoon is empty");
            }
        }

        KeyAction::HarpoonPrev => {
            if let Some(path) = editor.harpoon.prev().cloned() {
                if let Err(e) = editor.open_file(path) {
                    editor.set_status(format!("Error opening file: {}", e));
                }
            } else {
                editor.set_status("Harpoon is empty");
            }
        }

        KeyAction::Unknown => {
            // Unknown key, ignore
        }
    }
}

fn handle_insert_mode(editor: &mut Editor, key: KeyEvent) {
    // Apply custom keymap remapping for insert mode
    let key = editor.keymap.remap_insert(key);

    // If completion popup is active, handle completion keys first
    if editor.completion.active {
        match (key.modifiers, key.code) {
            // Navigate completion
            (KeyModifiers::NONE, KeyCode::Up) | (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
                editor.completion.select_prev();
                return;
            }
            (KeyModifiers::NONE, KeyCode::Down) | (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
                editor.completion.select_next();
                return;
            }
            // Accept completion
            (KeyModifiers::NONE, KeyCode::Enter) | (KeyModifiers::NONE, KeyCode::Tab) => {
                // Get completion info before modifying state
                let completion_info = editor.completion.selected_item()
                    .map(|item| (
                        item.insert_text.as_deref().unwrap_or(&item.label).to_string(),
                        item.label.clone(),
                        item.kind,
                    ));

                if let Some((text, label, kind)) = completion_info {
                    // Record frecency usage
                    editor.record_completion_use(&label);

                    // Delete back to trigger position and insert completion
                    let chars_to_delete = editor.cursor.col.saturating_sub(editor.completion.trigger_col);
                    for _ in 0..chars_to_delete {
                        editor.delete_char_before();
                    }
                    for c in text.chars() {
                        editor.insert_char(c);
                    }

                    // Auto-brackets: add () for functions/methods and position cursor inside
                    let needs_brackets = matches!(kind,
                        CompletionKind::Function |
                        CompletionKind::Method |
                        CompletionKind::Constructor
                    );
                    // Only add brackets if the text doesn't already end with ()
                    if needs_brackets && !text.ends_with("()") && !text.ends_with('(') {
                        editor.insert_char('(');
                        editor.insert_char(')');
                        // Move cursor back inside the parentheses
                        if editor.cursor.col > 0 {
                            editor.cursor.col -= 1;
                        }
                    }
                }
                editor.completion.hide();
                return;
            }
            // Cancel completion
            (KeyModifiers::NONE, KeyCode::Esc) => {
                editor.completion.hide();
                return;
            }
            // Backspace - let it fall through, filter will be updated after
            (KeyModifiers::NONE, KeyCode::Backspace) => {
                // Continue to normal handling below
            }
            // Word-ending characters - hide completion and continue
            (_, KeyCode::Char(c)) if matches!(c, ' ' | ';' | '(' | ')' | '{' | '}' | '[' | ']' | ',' | ':') => {
                editor.completion.hide();
                // Continue to normal handling below
            }
            // Regular word character - let it fall through, filter will be updated after
            (_, KeyCode::Char(c)) if !c.is_control() => {
                // Continue to normal handling below
            }
            // Any other key hides completion and continues normal handling
            _ => {
                editor.completion.hide();
            }
        }
    }

    // Track if completion was active before processing key
    let completion_was_active = editor.completion.active;

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
            // Auto-pairs: delete matching pair if cursor is between them
            if editor.settings.editor.auto_pairs {
                let col = editor.cursor.col;
                let line = editor.cursor.line;
                if col > 0 {
                    let prev_char = editor.buffer().char_at(line, col - 1);
                    let next_char = editor.buffer().char_at(line, col);
                    if let (Some(prev), Some(next)) = (prev_char, next_char) {
                        let is_matching_pair = matches!(
                            (prev, next),
                            ('(', ')') | ('[', ']') | ('{', '}') |
                            ('"', '"') | ('\'', '\'') | ('`', '`')
                        );
                        if is_matching_pair {
                            // Delete both characters
                            editor.delete_char_before(); // Delete opening
                            editor.delete_char_at(); // Delete closing (now at cursor)
                            return;
                        }
                    }
                }
            }
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

        // Regular character - accept any modifier for printable chars
        (_, KeyCode::Char(c)) if !c.is_control() => {
            if editor.settings.editor.auto_pairs {
                // Auto-pairs: skip over closing pair if next char is the same
                let next_char = editor.buffer().char_at(editor.cursor.line, editor.cursor.col);
                let is_closing = matches!(c, ')' | ']' | '}' | '"' | '\'' | '`');
                if is_closing && next_char == Some(c) {
                    // Skip over the closing character
                    editor.cursor.col += 1;
                    return;
                }

                // Auto-pairs: insert matching closing pair
                let closing = match c {
                    '(' => Some(')'),
                    '[' => Some(']'),
                    '{' => Some('}'),
                    '"' => Some('"'),
                    '\'' => Some('\''),
                    '`' => Some('`'),
                    _ => None,
                };

                if let Some(close) = closing {
                    editor.insert_char(c);
                    editor.insert_char(close);
                    // Move cursor back between the pair
                    if editor.cursor.col > 0 {
                        editor.cursor.col -= 1;
                    }
                    return;
                }
            }
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

    // Update completion filter after character changes
    if completion_was_active && editor.completion.active {
        // Get the text typed since trigger position
        if editor.cursor.line == editor.completion.trigger_line {
            let col = editor.cursor.col;
            let trigger_col = editor.completion.trigger_col;

            if col >= trigger_col {
                // Get the prefix from the current line
                if let Some(line) = editor.buffer().line(editor.cursor.line) {
                    let line_str: String = line.chars().collect();
                    let prefix: String = line_str.chars().skip(trigger_col).take(col - trigger_col).collect();

                    // If isIncomplete and filter text changed, request new completions
                    if editor.completion.is_incomplete && prefix != editor.completion.filter_text {
                        editor.needs_completion_refresh = true;
                    }

                    // Update filter with frecency-aware sorting
                    editor.update_completion_filter(&prefix);

                    // Hide if no matches
                    if editor.completion.filtered.is_empty() {
                        editor.completion.hide();
                    }
                }
            } else {
                // Cursor moved before trigger point - hide completion
                editor.completion.hide();
            }
        } else {
            // Cursor moved to different line - hide completion
            editor.completion.hide();
        }
    }
}

fn handle_replace_mode(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Exit replace mode
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
            editor.enter_normal_mode();
        }

        // Backspace - move back (don't undo replacement)
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            if editor.cursor.col > 0 {
                editor.cursor.col -= 1;
            }
        }

        // Arrow keys for navigation
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

        // Regular character - replace
        (_, KeyCode::Char(c)) if !c.is_control() => {
            editor.replace_mode_char(c);
        }

        _ => {}
    }
}

fn handle_rename_prompt_mode(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Cancel rename
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) |
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            editor.cancel_rename();
        }

        // Confirm rename
        (KeyModifiers::NONE, KeyCode::Enter) => {
            editor.confirm_rename();
        }

        // Backspace
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            editor.rename_input_backspace();
        }

        // Clear all (Ctrl+U)
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            editor.rename_input_clear();
        }

        // Regular character input
        (_, KeyCode::Char(c)) if !c.is_control() => {
            editor.rename_input_char(c);
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

        // Regular character - accept any modifier for printable chars
        (_, KeyCode::Char(c)) if !c.is_control() => {
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
                // Update incremental search highlights
                editor.update_incremental_search();
            }
        }

        // Cursor movement
        (KeyModifiers::NONE, KeyCode::Left) => {
            editor.search.move_left();
        }
        (KeyModifiers::NONE, KeyCode::Right) => {
            editor.search.move_right();
        }

        // Regular character - accept any modifier for printable chars
        (_, KeyCode::Char(c)) if !c.is_control() => {
            editor.search.insert_char(c);
            // Update incremental search highlights
            editor.update_incremental_search();
        }

        _ => {}
    }
}

fn handle_visual_mode(editor: &mut Editor, key: KeyEvent) {
    use crate::input::Motion;

    // Handle gc for comment toggle (after g was pressed)
    if editor.input_state.pending_comment {
        editor.input_state.pending_comment = false;
        if matches!(key.code, KeyCode::Char('c')) {
            // gc in visual mode - toggle comments on selection
            let (start_line, _, end_line, _) = editor.get_visual_range();
            let (first, last) = if start_line <= end_line {
                (start_line, end_line)
            } else {
                (end_line, start_line)
            };
            editor.toggle_comment_lines(first, last);
            editor.enter_normal_mode();
            return;
        }
        // If not 'c', fall through to normal handling (e.g., gg)
        if matches!(key.code, KeyCode::Char('g')) {
            editor.apply_motion(Motion::FileStart, 1);
            return;
        }
    }

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

        // File motions and gc for comment toggle
        (KeyModifiers::NONE, KeyCode::Char('g')) => {
            // Set pending_comment flag for gc sequence in visual mode
            editor.input_state.pending_comment = true;
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
            if let Some(item) = editor.finder_select() {
                if let Some(buf_idx) = item.buffer_idx {
                    if !editor.switch_to_buffer(buf_idx) {
                        editor.set_status("Buffer not found");
                    }
                } else {
                    // Open the selected file
                    if let Err(e) = editor.open_file(item.path) {
                        editor.set_status(format!("Error opening file: {}", e));
                    } else if let Some(line_num) = item.line {
                        // Jump to the line (for grep results)
                        editor.cursor.line = line_num.saturating_sub(1);
                        editor.cursor.col = 0;
                        editor.scroll_to_cursor();
                    }
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

        // Regular character - accept any modifier combination for printable chars
        // Some terminals may report SHIFT for uppercase or special chars like _
        (_, KeyCode::Char(c)) if !c.is_control() => {
            editor.finder.insert_char(c);
        }

        _ => {}
    }
}

fn handle_explorer_mode(editor: &mut Editor, key: KeyEvent) {
    // Handle leader key sequences (same as normal mode)
    if let Some(ref mut sequence) = editor.leader_sequence {
        if key.code == KeyCode::Esc {
            editor.leader_sequence = None;
            editor.clear_status();
            return;
        }

        if let KeyCode::Char(c) = key.code {
            sequence.push(c);
            let seq = sequence.clone();

            if let Some(action) = editor.keymap.get_leader_action(&seq) {
                let action = action.clone();
                editor.leader_sequence = None;
                editor.clear_status();
                execute_leader_action(editor, &action);
                return;
            }

            if editor.keymap.is_leader_prefix(&seq) {
                editor.set_status(format!("<leader>{}", seq));
                return;
            }

            editor.leader_sequence = None;
            editor.clear_status();
            return;
        }

        editor.leader_sequence = None;
        editor.clear_status();
        return;
    }

    // Check if this key is the leader key
    if editor.keymap.has_leader_mappings() && editor.keymap.is_leader_key(key) {
        editor.leader_sequence = Some(String::new());
        editor.set_status("<leader>");
        return;
    }

    match (key.modifiers, key.code) {
        // Close explorer
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) |
        (KeyModifiers::NONE, KeyCode::Char('q')) => {
            editor.close_explorer();
        }

        // Move down
        (KeyModifiers::NONE, KeyCode::Char('j')) |
        (KeyModifiers::NONE, KeyCode::Down) => {
            editor.explorer.move_down();
        }

        // Move up
        (KeyModifiers::NONE, KeyCode::Char('k')) |
        (KeyModifiers::NONE, KeyCode::Up) => {
            editor.explorer.move_up();
        }

        // Expand directory or open file
        (KeyModifiers::NONE, KeyCode::Enter) |
        (KeyModifiers::NONE, KeyCode::Char('l')) |
        (KeyModifiers::NONE, KeyCode::Right) => {
            if let Some(path) = editor.explorer_selected_path() {
                if path.is_dir() {
                    // Expand directory
                    editor.explorer.expand();
                } else {
                    // Open file and switch to normal mode
                    let path_clone = path.clone();
                    if let Err(e) = editor.open_file(path_clone) {
                        editor.set_status(format!("Error opening file: {}", e));
                    } else {
                        editor.mode = Mode::Normal;
                    }
                }
            }
        }

        // Collapse directory or go to parent
        (KeyModifiers::NONE, KeyCode::Char('h')) |
        (KeyModifiers::NONE, KeyCode::Left) => {
            editor.explorer.collapse();
        }

        // Toggle expand/collapse
        (KeyModifiers::NONE, KeyCode::Tab) => {
            editor.explorer.toggle_expand();
        }

        // Collapse all
        (KeyModifiers::SHIFT, KeyCode::Char('W')) |
        (KeyModifiers::NONE, KeyCode::Char('W')) => {
            editor.explorer.collapse_all();
        }

        // Refresh
        (KeyModifiers::SHIFT, KeyCode::Char('R')) |
        (KeyModifiers::NONE, KeyCode::Char('R')) => {
            editor.explorer.refresh();
        }

        // Go to parent directory
        (KeyModifiers::NONE, KeyCode::Char('-')) => {
            editor.explorer.go_to_parent();
        }

        // Focus editor (keep explorer open)
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            editor.unfocus_explorer();
        }

        _ => {}
    }
}

/// Execute a parsed command
/// Helper to create a file and open it in the editor
fn create_and_open_file(editor: &mut Editor, path: std::path::PathBuf) -> CommandResult {
    match std::fs::File::create(&path) {
        Ok(_) => {
            if let Err(e) = editor.open_file(path.clone()) {
                CommandResult::Error(format!("Created file but failed to open: {}", e))
            } else {
                CommandResult::Message(format!("Created: {}", path.display()))
            }
        }
        Err(e) => CommandResult::Error(format!("Failed to create file: {}", e)),
    }
}

/// Helper to rename a file
fn rename_file_impl(editor: &mut Editor, old_path: std::path::PathBuf, new_path: std::path::PathBuf) -> CommandResult {
    // Create parent directories if needed
    if let Some(parent) = new_path.parent() {
        if !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return CommandResult::Error(format!("Failed to create directories: {}", e));
            }
        }
    }

    // Rename the file
    match std::fs::rename(&old_path, &new_path) {
        Ok(_) => {
            // Update buffer path
            editor.set_buffer_path(new_path.clone());
            CommandResult::Message(format!("Renamed to: {}", new_path.display()))
        }
        Err(e) => CommandResult::Error(format!("Failed to rename: {}", e)),
    }
}

fn execute_command(editor: &mut Editor, cmd: Command) {
    let result = match cmd {
        Command::Write(path) => {
            if let Some(p) = path {
                // Save as: skip format_on_save for explicit path
                match editor.save_as(p) {
                    Ok(()) => CommandResult::Ok,
                    Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
                }
            } else if editor.buffer().path.is_some() {
                // Check if format_on_save is enabled
                if editor.settings.editor.format_on_save {
                    // Set flag to save after formatting completes
                    editor.save_after_format = true;
                    // Trigger formatting (which will save when done)
                    editor.pending_lsp_action = Some(LspAction::Formatting);
                    CommandResult::Message("Formatting...".to_string())
                } else {
                    // Format on save disabled - save directly
                    match editor.save() {
                        Ok(()) => CommandResult::Ok,
                        Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
                    }
                }
            } else {
                CommandResult::Error("No filename".to_string())
            }
        }

        Command::Quit => {
            if editor.has_unsaved_changes() {
                CommandResult::Error("No write since last change (add ! to override)".to_string())
            } else {
                // If multiple panes, close just the active pane
                if editor.panes().len() > 1 {
                    editor.close_pane();
                    CommandResult::Ok
                } else {
                    CommandResult::Quit
                }
            }
        }

        Command::ForceQuit => {
            // If multiple panes, close just the active pane
            if editor.panes().len() > 1 {
                editor.close_pane();
                CommandResult::Ok
            } else {
                CommandResult::Quit
            }
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

        Command::NoHighlight => {
            editor.search_matches.clear();
            CommandResult::Ok
        }

        Command::Substitute { entire_file, pattern, replacement, global } => {
            let count = editor.substitute(&pattern, &replacement, entire_file, global);
            if count > 0 {
                CommandResult::Message(format!("{} substitution(s)", count))
            } else {
                CommandResult::Message(format!("Pattern not found: {}", pattern))
            }
        }

        Command::NewFile(path) => {
            // Resolve path relative to project root
            let full_path = if path.is_absolute() {
                path
            } else {
                editor.working_directory().join(&path)
            };

            // Create parent directories if needed
            if let Some(parent) = full_path.parent() {
                if !parent.exists() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        CommandResult::Error(format!("Failed to create directories: {}", e))
                    } else {
                        create_and_open_file(editor, full_path)
                    }
                } else {
                    create_and_open_file(editor, full_path)
                }
            } else {
                create_and_open_file(editor, full_path)
            }
        }

        Command::DeleteFile => {
            // Get current file path
            if let Some(path) = editor.buffer().path.clone() {
                CommandResult::ConfirmDelete(path)
            } else {
                CommandResult::Error("No file to delete (buffer has no path)".to_string())
            }
        }

        Command::DeleteFileForce => {
            // Get current file path and delete without confirmation
            if let Some(path) = editor.buffer().path.clone() {
                match std::fs::remove_file(&path) {
                    Ok(_) => {
                        // Close the buffer
                        editor.close_current_buffer();
                        CommandResult::Message(format!("Deleted: {}", path.display()))
                    }
                    Err(e) => CommandResult::Error(format!("Failed to delete: {}", e)),
                }
            } else {
                CommandResult::Error("No file to delete (buffer has no path)".to_string())
            }
        }

        Command::RenameFile(new_name) => {
            if let Some(old_path) = editor.buffer().path.clone() {
                // Resolve new path - if just a name, keep in same directory
                let new_path = if new_name.is_absolute() {
                    new_name
                } else if new_name.components().count() == 1 {
                    // Just a filename, keep in same directory
                    old_path.parent().unwrap_or(std::path::Path::new(".")).join(&new_name)
                } else {
                    // Relative path, resolve from project root
                    editor.working_directory().join(&new_name)
                };

                rename_file_impl(editor, old_path, new_path)
            } else {
                CommandResult::Error("No file to rename (buffer has no path)".to_string())
            }
        }

        Command::MakeDir(path) => {
            // Resolve path relative to project root
            let full_path = if path.is_absolute() {
                path
            } else {
                editor.working_directory().join(&path)
            };

            match std::fs::create_dir_all(&full_path) {
                Ok(_) => CommandResult::Message(format!("Created directory: {}", full_path.display())),
                Err(e) => CommandResult::Error(format!("Failed to create directory: {}", e)),
            }
        }

        Command::ToggleExplorer => {
            editor.toggle_explorer();
            CommandResult::Ok
        }

        Command::OpenExplorer => {
            editor.open_explorer();
            CommandResult::Ok
        }

        Command::Format => {
            // Request formatting via LSP
            editor.pending_lsp_action = Some(LspAction::Formatting);
            CommandResult::Message("Formatting...".to_string())
        }

        Command::CodeAction => {
            // Trigger code actions picker
            editor.pending_lsp_action = Some(LspAction::CodeActions);
            CommandResult::Ok
        }

        Command::Rename(new_name) => {
            // Trigger LSP rename
            editor.pending_lsp_action = Some(LspAction::RenameSymbol(new_name.clone()));
            CommandResult::Message(format!("Renaming to '{}'...", new_name))
        }

        Command::RenamePrompt => {
            // Enter rename prompt mode
            editor.enter_rename_prompt();
            CommandResult::Ok
        }

        Command::HarpoonAdd => {
            if let Some(path) = editor.buffer().path.clone() {
                let msg = editor.harpoon.add_file(&path);
                CommandResult::Message(msg)
            } else {
                CommandResult::Error("Cannot add unsaved buffer to harpoon".to_string())
            }
        }

        Command::HarpoonMenu => {
            editor.harpoon.toggle_menu();
            CommandResult::Ok
        }

        Command::HarpoonJump(slot) => {
            if let Some(path) = editor.harpoon.get_slot(slot).cloned() {
                match editor.open_file(path) {
                    Ok(_) => CommandResult::Ok,
                    Err(e) => CommandResult::Error(format!("Error opening file: {}", e)),
                }
            } else {
                CommandResult::Error(format!("Harpoon slot {} is empty", slot))
            }
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
        CommandResult::ConfirmDelete(path) => {
            editor.set_status(format!("Delete {}? Use :delete! to confirm", path.display()));
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
