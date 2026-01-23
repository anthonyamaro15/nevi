use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, ClearType},
    style::{SetForegroundColor, SetBackgroundColor, ResetColor, Color, SetAttribute, Attribute, SetUnderlineColor},
};
use std::io::{self, Write, Stdout};
use std::time::Instant;

use crate::editor::{Editor, Mode, PaneDirection, Pane, SplitLayout, LspAction, MarkEntry, MarksPicker};
use crate::input::{KeyAction, InsertPosition, Operator, TextObject, TextObjectModifier, TextObjectType};
use crate::commands::{Command, CommandResult, parse_command};
use crate::config::LeaderAction;
use crate::syntax::HighlightSpan;
use crate::lsp::types::{CompletionKind, Diagnostic, DiagnosticSeverity};

/// Events from the terminal that the editor cares about
pub enum EditorEvent {
    /// A key press
    Key(KeyEvent),
    /// Terminal gained focus (for autoread)
    FocusGained,
}

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

/// Dim a color by reducing its brightness (for hidden files, etc.)
fn dim_color(color: Color) -> Color {
    match color {
        Color::Rgb { r, g, b } => Color::Rgb {
            r: (r as f32 * 0.6) as u8,
            g: (g as f32 * 0.6) as u8,
            b: (b as f32 * 0.6) as u8,
        },
        // For non-RGB colors, return a generic dim gray
        _ => Color::DarkGrey,
    }
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
            cursor::Hide,
            event::EnableFocusChange
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

        // Re-enter alternate screen, hide cursor, and re-enable focus change reporting
        execute!(
            self.stdout,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            event::EnableFocusChange
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

        // Render diagnostic floating popup if active
        if editor.show_diagnostic_float {
            self.render_diagnostic_float(editor)?;
        }

        // Render marks picker if active
        if editor.marks_picker.is_some() {
            self.render_marks_picker(editor)?;
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

        // Render theme picker if active
        if editor.theme_picker.is_some() {
            self.render_theme_picker(editor)?;
        }

        // Render floating terminal if visible
        if editor.floating_terminal.is_visible() {
            self.render_floating_terminal(editor)?;
        }

        // Position cursor
        self.position_cursor(editor)?;

        self.stdout.flush()?;
        Ok(())
    }

    /// Render a single pane's content
    fn render_pane(&mut self, editor: &Editor, pane: &Pane, is_active: bool) -> anyhow::Result<()> {
        let buffer = editor.buffer_at(pane.buffer_idx).unwrap();
        let buffer_path = buffer.path.clone();
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
                buffer_path.as_ref(),
            )?;
        } else {
            // Original non-wrapped rendering
            self.render_pane_nowrap(
                editor, pane, buffer, rect, is_active,
                line_num_width, visual_range, show_line_numbers,
                show_relative, highlight_cursor_line,
                pane_height, pane_width, text_area_width,
                buffer_path.as_ref(),
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
        buffer_path: Option<&std::path::PathBuf>,
    ) -> anyhow::Result<()> {
        // Get theme colors once at the start
        let theme = editor.theme();
        let cursor_line_bg = theme.ui.cursor_line;
        let editor_bg = theme.ui.background;
        let editor_fg = theme.ui.foreground;
        let selection_bg = theme.ui.selection;
        let search_bg = theme.ui.search_match_bg;
        let search_fg = theme.ui.search_match_fg;

        // Pre-compute URI for diagnostic lookups (avoids repeated string allocations)
        let cached_uri = if is_active { editor.current_buffer_uri() } else { None };

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
            let line_diagnostics = match &cached_uri {
                Some(uri) => editor.diagnostics_for_line_cached(file_line, uri),
                None => Vec::new(),
            };
            let has_error = line_diagnostics.iter().any(|d| {
                matches!(d.severity, DiagnosticSeverity::Error)
            });
            let has_warning = line_diagnostics.iter().any(|d| {
                matches!(d.severity, DiagnosticSeverity::Warning)
            });

            // Render each segment
            for segment in &segments {
                if current_row >= pane_height {
                    break;
                }

                let screen_y = rect.y + current_row as u16;
                execute!(self.stdout, cursor::MoveTo(rect.x, screen_y))?;

                // Calculate row background (cursor line or normal editor background)
                let row_bg = if highlight_cursor_line && is_cursor_line {
                    cursor_line_bg
                } else {
                    editor_bg
                };

                // Set background and foreground for this row
                execute!(self.stdout, SetBackgroundColor(row_bg), SetForegroundColor(editor_fg))?;

                // Sign column (git signs + diagnostic icons) - only on first segment
                // Layout: [Git Sign][Diagnostic] = 2 chars total
                if segment.is_first {
                    // Git sign (first char)
                    let git_status = if is_active {
                        buffer_path.and_then(|p| editor.git_status_for_line_in_file(p, file_line))
                    } else {
                        None
                    };

                    match git_status {
                        Some(crate::git::GitLineStatus::Added) => {
                            execute!(self.stdout, SetForegroundColor(theme.git.added))?;
                            print!("▎");
                            execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
                        }
                        Some(crate::git::GitLineStatus::Modified) => {
                            execute!(self.stdout, SetForegroundColor(theme.git.modified))?;
                            print!("▎");
                            execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
                        }
                        Some(crate::git::GitLineStatus::Deleted) => {
                            execute!(self.stdout, SetForegroundColor(theme.git.deleted))?;
                            print!("▁");
                            execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
                        }
                        None => {
                            print!(" ");
                        }
                    }

                    // Diagnostic sign (second char)
                    if has_error {
                        execute!(self.stdout, SetForegroundColor(theme.diagnostic.error))?;
                        print!("●");
                        execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
                    } else if has_warning {
                        execute!(self.stdout, SetForegroundColor(theme.diagnostic.warning))?;
                        print!("▲");
                        execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
                    } else {
                        print!(" ");
                    }
                } else {
                    print!("  "); // Empty sign column for continuation lines
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

                        // Use theme colors for line numbers
                        let line_num_color = if has_error {
                            theme.diagnostic.error
                        } else if has_warning {
                            theme.diagnostic.warning
                        } else if is_cursor_line {
                            theme.ui.line_number_active
                        } else {
                            theme.ui.line_number
                        };

                        execute!(self.stdout, SetForegroundColor(line_num_color))?;
                        print!("{}", line_num);
                        execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
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
                    &line_diagnostics,
                    editor_bg,
                    editor_fg,
                    cursor_line_bg,
                    selection_bg,
                    search_bg,
                    search_fg,
                    theme.diagnostic.error,
                    theme.diagnostic.warning,
                    theme.diagnostic.info,
                    theme.diagnostic.hint,
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
                                DiagnosticSeverity::Error => (Color::Red, "●"),
                                DiagnosticSeverity::Warning => (Color::Yellow, "●"),
                                DiagnosticSeverity::Information => (Color::Blue, "●"),
                                DiagnosticSeverity::Hint => (Color::Cyan, "○"),
                            };

                            let msg: String = diag.message
                                .lines()
                                .next()
                                .unwrap_or(&diag.message)
                                .chars()
                                .take(remaining)
                                .collect();

                            // Row background already set at start of row
                            execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                            print!(" ");
                            execute!(self.stdout, SetForegroundColor(color))?;
                            print!("{}", icon);
                            execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                            print!(" {}", msg);
                            execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;

                            chars_printed += 3 + msg.chars().count();
                        }
                    }
                }

                for _ in chars_printed..pane_width {
                    print!(" ");
                }

                current_row += 1;
            }

            file_line += 1;
        }

        // Fill remaining rows with ~ indicators
        while current_row < pane_height {
            let screen_y = rect.y + current_row as u16;
            execute!(self.stdout, cursor::MoveTo(rect.x, screen_y))?;

            // Set editor background for empty rows
            execute!(self.stdout, SetBackgroundColor(editor_bg), SetForegroundColor(editor_fg))?;

            print!("  "); // Empty sign column

            execute!(self.stdout, SetForegroundColor(Color::Blue))?;
            if show_line_numbers {
                print!("{:>width$} ~", "", width = line_num_width);
            } else {
                print!("~");
            }
            execute!(self.stdout, SetForegroundColor(editor_fg))?;

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
        buffer_path: Option<&std::path::PathBuf>,
    ) -> anyhow::Result<()> {
        // Get theme colors once
        let theme = editor.theme();
        let cursor_line_bg = theme.ui.cursor_line;
        let editor_bg = theme.ui.background;
        let editor_fg = theme.ui.foreground;

        // Pre-compute URI for diagnostic lookups (avoids repeated string allocations)
        let cached_uri = if is_active { editor.current_buffer_uri() } else { None };

        // Render each row in this pane
        for row in 0..pane_height {
            let screen_y = rect.y + row as u16;
            let file_line = pane.viewport_offset + row;
            let is_cursor_line = is_active && file_line == pane.cursor.line;

            // Move to start of this row in the pane
            execute!(self.stdout, cursor::MoveTo(rect.x, screen_y))?;

            // Set background color for this row (cursor line or normal)
            let row_bg = if highlight_cursor_line && is_cursor_line && file_line < buffer.len_lines() {
                cursor_line_bg
            } else {
                editor_bg
            };
            execute!(self.stdout, SetBackgroundColor(row_bg), SetForegroundColor(editor_fg))?;

            if file_line < buffer.len_lines() {
                // Check for diagnostics on this line (only for active pane)
                let line_diagnostics = match &cached_uri {
                    Some(uri) => editor.diagnostics_for_line_cached(file_line, uri),
                    None => Vec::new(),
                };
                let has_error = line_diagnostics.iter().any(|d| {
                    matches!(d.severity, DiagnosticSeverity::Error)
                });
                let has_warning = line_diagnostics.iter().any(|d| {
                    matches!(d.severity, DiagnosticSeverity::Warning)
                });

                // Sign column (git signs + diagnostic icons)
                // Layout: [Git Sign][Diagnostic] = 2 chars total

                // Git sign (first char)
                let git_status = if is_active {
                    buffer_path.and_then(|p| editor.git_status_for_line_in_file(p, file_line))
                } else {
                    None
                };

                match git_status {
                    Some(crate::git::GitLineStatus::Added) => {
                        execute!(self.stdout, SetForegroundColor(theme.git.added))?;
                        print!("▎");
                        execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
                    }
                    Some(crate::git::GitLineStatus::Modified) => {
                        execute!(self.stdout, SetForegroundColor(theme.git.modified))?;
                        print!("▎");
                        execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
                    }
                    Some(crate::git::GitLineStatus::Deleted) => {
                        execute!(self.stdout, SetForegroundColor(theme.git.deleted))?;
                        print!("▁");
                        execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
                    }
                    None => {
                        print!(" ");
                    }
                }

                // Diagnostic sign (second char)
                if has_error {
                    execute!(self.stdout, SetForegroundColor(theme.diagnostic.error))?;
                    print!("●");
                    execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
                } else if has_warning {
                    execute!(self.stdout, SetForegroundColor(theme.diagnostic.warning))?;
                    print!("▲");
                    execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
                } else {
                    print!(" ");
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

                    // Use theme colors for line numbers
                    let line_num_color = if has_error {
                        theme.diagnostic.error
                    } else if has_warning {
                        theme.diagnostic.warning
                    } else if is_cursor_line {
                        theme.ui.line_number_active
                    } else {
                        theme.ui.line_number
                    };

                    execute!(self.stdout, SetForegroundColor(line_num_color))?;
                    print!("{}", line_num);
                    execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;
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

                    // Get search/selection colors from theme
                    let selection_bg = theme.ui.selection;
                    let search_bg = theme.ui.search_match_bg;
                    let search_fg = theme.ui.search_match_fg;

                    self.render_line_with_highlights(
                        line_str,
                        file_line,
                        &highlights,
                        visual_range,
                        &editor.mode,
                        highlight_cursor_line && is_cursor_line,
                        &editor.search_matches,
                        &line_diagnostics,
                        editor_bg,
                        editor_fg,
                        cursor_line_bg,
                        selection_bg,
                        search_bg,
                        search_fg,
                        theme.diagnostic.error,
                        theme.diagnostic.warning,
                        theme.diagnostic.info,
                        theme.diagnostic.hint,
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
                                execute!(self.stdout, SetForegroundColor(Color::DarkGrey), SetBackgroundColor(row_bg))?;
                                print!("{}", ghost_chars);
                                execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;

                                chars_printed += ghost_chars.chars().count();
                            }
                        }
                    }

                    // Render Copilot ghost text on cursor line
                    // Note: Copilot ghost text can coexist with LSP completion popup
                    // (the popup shows as a dropdown, ghost text shows inline after cursor)
                    if is_cursor_line && is_active && editor.mode == Mode::Insert
                        && editor.copilot_ghost.is_some()
                    {
                        if let Some(ref ghost) = editor.copilot_ghost {
                            // Only show if trigger position matches cursor
                            if ghost.trigger_line == file_line && ghost.trigger_col <= pane.cursor.col {
                                let remaining = pane_width.saturating_sub(chars_printed);
                                let ghost_chars: String = ghost.inline_text.chars().take(remaining).collect();
                                let ghost_len = ghost_chars.chars().count();

                                // Render Copilot ghost text in a slightly different gray
                                execute!(self.stdout, SetForegroundColor(Color::Rgb { r: 100, g: 100, b: 110 }), SetBackgroundColor(row_bg))?;
                                print!("{}", ghost_chars);

                                // Show count if multiple completions
                                if !ghost.count_display.is_empty() {
                                    let count_remaining = remaining.saturating_sub(ghost_len + 1);
                                    if count_remaining >= ghost.count_display.len() {
                                        execute!(self.stdout, SetForegroundColor(Color::Rgb { r: 80, g: 80, b: 90 }))?;
                                        print!(" {}", ghost.count_display);
                                        chars_printed += 1 + ghost.count_display.len();
                                    }
                                }

                                execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;

                                chars_printed += ghost_len;
                            }
                        }
                    }

                    // Render inline diagnostic (virtual text) for this line
                    // Reuse line_diagnostics from earlier instead of calling diagnostics_for_line again
                    if is_active {
                        if let Some(diag) = line_diagnostics.first() {
                            // Calculate remaining space for diagnostic
                            let remaining = pane_width.saturating_sub(chars_printed + 3); // 3 for " ● "
                            if remaining > 5 {
                                // Determine color based on severity
                                let (color, icon) = match diag.severity {
                                    DiagnosticSeverity::Error => (Color::Red, "●"),
                                    DiagnosticSeverity::Warning => (Color::Yellow, "●"),
                                    DiagnosticSeverity::Information => (Color::Blue, "●"),
                                    DiagnosticSeverity::Hint => (Color::Cyan, "○"),
                                };

                                // Truncate message to fit
                                let msg: String = diag.message
                                    .lines()
                                    .next()
                                    .unwrap_or(&diag.message)
                                    .chars()
                                    .take(remaining)
                                    .collect();

                                // Render: space, icon, space, message
                                execute!(self.stdout, SetForegroundColor(Color::DarkGrey), SetBackgroundColor(row_bg))?;
                                print!(" ");
                                execute!(self.stdout, SetForegroundColor(color))?;
                                print!("{}", icon);
                                execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                                print!(" {}", msg);
                                execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;

                                chars_printed += 3 + msg.chars().count();
                            }
                        }
                    }

                    // Fill remaining space in pane with theme background
                    execute!(self.stdout, SetBackgroundColor(row_bg), SetForegroundColor(editor_fg))?;
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
                execute!(self.stdout, SetForegroundColor(editor_fg), SetBackgroundColor(row_bg))?;

                // Fill remaining space (sign column = 2)
                let chars_printed = 2 + if show_line_numbers { line_num_width + 2 } else { 1 };
                for _ in chars_printed..pane_width {
                    print!(" ");
                }
            }
            // Keep background color set (don't reset to terminal default)
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
        is_cursor_line: bool,
        search_matches: &[(usize, usize, usize)],
        diagnostics: &[&Diagnostic],
        editor_bg: Color,
        editor_fg: Color,
        cursor_line_bg: Color,
        selection_bg: Color,
        search_match_bg: Color,
        search_match_fg: Color,
        diagnostic_error_color: Color,
        diagnostic_warning_color: Color,
        diagnostic_info_color: Color,
        diagnostic_hint_color: Color,
    ) -> anyhow::Result<()> {
        let chars: Vec<char> = text.chars().collect();

        // Determine the base background for this line
        let base_bg = if is_cursor_line { cursor_line_bg } else { editor_bg };

        // Check if a column is within a search match for this line
        let in_search_match = |col: usize| -> bool {
            search_matches.iter().any(|(l, start, end)| {
                *l == line_num && col >= *start && col < *end
            })
        };

        // Find diagnostic at this column (prioritize by severity: Error > Warning > Info > Hint)
        let get_diagnostic_at = |col: usize| -> Option<&Diagnostic> {
            diagnostics.iter()
                .filter(|d| col >= d.col_start && col < d.col_end)
                .min_by_key(|d| match d.severity {
                    DiagnosticSeverity::Error => 0,
                    DiagnosticSeverity::Warning => 1,
                    DiagnosticSeverity::Information => 2,
                    DiagnosticSeverity::Hint => 3,
                })
                .copied()
        };

        let mut current_fg: Option<Color> = None;
        let mut current_bg: Option<Color> = None;
        let mut current_underline: Option<Color> = None;

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

            // Check for diagnostic underline
            let diag_at_col = get_diagnostic_at(actual_col);
            let diag_underline_color = diag_at_col.map(|d| match d.severity {
                DiagnosticSeverity::Error => diagnostic_error_color,
                DiagnosticSeverity::Warning => diagnostic_warning_color,
                DiagnosticSeverity::Information => diagnostic_info_color,
                DiagnosticSeverity::Hint => diagnostic_hint_color,
            });

            // Find syntax highlight for this position
            let syntax_color = highlights.iter()
                .find(|h| actual_col >= h.start_col && actual_col < h.end_col)
                .map(|h| h.fg);

            // Priority: visual selection > search match > base (cursor line or editor bg)
            let (desired_bg, desired_fg) = if in_visual {
                (selection_bg, syntax_color.unwrap_or(editor_fg))
            } else if is_search {
                (search_match_bg, search_match_fg)
            } else {
                (base_bg, syntax_color.unwrap_or(editor_fg))
            };

            // Only change colors when necessary
            if Some(desired_bg) != current_bg {
                execute!(self.stdout, SetBackgroundColor(desired_bg))?;
                current_bg = Some(desired_bg);
            }
            if Some(desired_fg) != current_fg {
                execute!(self.stdout, SetForegroundColor(desired_fg))?;
                current_fg = Some(desired_fg);
            }

            // Handle underline attribute changes for diagnostics
            if diag_underline_color != current_underline {
                if let Some(color) = diag_underline_color {
                    // Use colored underline (keeps syntax highlighting intact)
                    execute!(
                        self.stdout,
                        SetUnderlineColor(color),
                        SetAttribute(Attribute::Underlined)
                    )?;
                } else {
                    execute!(self.stdout, SetAttribute(Attribute::NoUnderline))?;
                }
                current_underline = diag_underline_color;
            }

            print!("{}", ch);
        }

        // Restore to base background/foreground and reset underline
        execute!(
            self.stdout,
            SetAttribute(Attribute::NoUnderline),
            SetBackgroundColor(base_bg),
            SetForegroundColor(editor_fg)
        )?;

        Ok(())
    }

    /// Draw separator lines between panes
    /// Render the file explorer sidebar
    fn render_explorer(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let width = editor.explorer.width as usize;
        let height = editor.text_rows();

        // Use theme colors for explorer
        let theme = editor.theme();
        let explorer_bg = theme.ui.explorer_bg;
        let explorer_fg = theme.ui.foreground;
        let selected_bg = theme.ui.explorer_selected;
        let dir_color = theme.ui.explorer_directory;
        let file_color = theme.ui.foreground;
        // Dimmer colors for hidden (dot) files - derive from theme colors
        let hidden_dir_color = dim_color(dir_color);
        let hidden_file_color = dim_color(file_color);
        let separator_color = theme.ui.explorer_border;
        let line_num_color = theme.ui.line_number;
        let current_line_num_color = theme.ui.line_number_active;
        let tree_line_color = dim_color(theme.ui.line_number);
        let match_color = theme.ui.finder_match; // Use finder match color for search matches

        // Line number column width (3 chars + 1 space)
        let line_num_width = 4;

        // Render header with project name
        execute!(self.stdout, cursor::MoveTo(0, 0), SetBackgroundColor(explorer_bg))?;

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

        execute!(self.stdout, SetForegroundColor(explorer_fg))?;
        execute!(self.stdout, SetAttribute(Attribute::Bold))?;
        print!("{:width$}", header, width = width);
        execute!(self.stdout, SetAttribute(Attribute::Reset), SetBackgroundColor(explorer_bg))?;

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

        // Available width for file entries (subtract line number column)
        let content_width = width.saturating_sub(line_num_width);

        // Render file tree
        for row in 0..list_height {
            let y = (row + 1) as u16; // +1 for header
            execute!(self.stdout, cursor::MoveTo(0, y), SetBackgroundColor(explorer_bg))?;

            let idx = scroll_offset + row;
            if idx < flat_view.len() {
                let node = &flat_view[idx];
                let is_selected = idx == selected;

                // Set background for selected item or normal explorer background
                if is_selected {
                    execute!(self.stdout, SetBackgroundColor(selected_bg))?;
                }

                // Render relative line number
                let rel_line = if is_selected {
                    0
                } else if idx > selected {
                    idx - selected
                } else {
                    selected - idx
                };

                if is_selected {
                    execute!(self.stdout, SetForegroundColor(current_line_num_color))?;
                } else {
                    execute!(self.stdout, SetForegroundColor(line_num_color))?;
                }
                print!("{:>3} ", rel_line);

                // Get icon
                let icon = editor.explorer.get_icon(node);

                // Draw tree connector lines for each depth level
                if node.depth > 1 {
                    execute!(self.stdout, SetForegroundColor(tree_line_color))?;
                    for _ in 0..(node.depth - 1) {
                        print!("│ ");
                    }
                }

                // Set colors based on file type and hidden status
                let is_hidden = node.name.starts_with('.');
                let is_match = editor.explorer.is_search_match(idx);

                if is_match {
                    execute!(self.stdout, SetForegroundColor(match_color))?;
                } else if node.is_dir {
                    if is_hidden {
                        execute!(self.stdout, SetForegroundColor(hidden_dir_color))?;
                    } else {
                        execute!(self.stdout, SetForegroundColor(dir_color))?;
                    }
                } else if is_hidden {
                    execute!(self.stdout, SetForegroundColor(hidden_file_color))?;
                } else {
                    execute!(self.stdout, SetForegroundColor(file_color))?;
                }

                // Calculate remaining width for the content
                let tree_indent_width = if node.depth > 1 { (node.depth - 1) * 2 } else { 0 };
                let remaining_width = content_width.saturating_sub(tree_indent_width);

                // Build the line (icon + name)
                let line = format!("{} {}", icon, node.name);
                let line = if line.len() > remaining_width {
                    format!("{}…", &line[..remaining_width.saturating_sub(1)])
                } else {
                    line
                };

                print!("{:width$}", line, width = remaining_width);
            } else {
                // Empty line - use explorer background
                execute!(self.stdout, SetForegroundColor(explorer_fg), SetBackgroundColor(explorer_bg))?;
                print!("{:width$}", "", width = width);
            }
        }

        // Render input prompt if there's a pending action
        if editor.explorer.has_pending_action() {
            let prompt_bg = explorer_bg;
            let prompt_y = height.saturating_sub(1) as u16;

            execute!(self.stdout, cursor::MoveTo(0, prompt_y))?;
            execute!(self.stdout, SetBackgroundColor(prompt_bg))?;

            // Prompt text
            let prompt = editor.explorer.action_prompt();
            execute!(self.stdout, SetForegroundColor(theme.ui.finder_prompt))?;
            print!("{}", prompt);

            // Input buffer
            execute!(self.stdout, SetForegroundColor(explorer_fg))?;
            let input = &editor.explorer.input_buffer;

            let available = width.saturating_sub(prompt.len());
            if input.len() <= available {
                print!("{}", input);
                let remaining = available.saturating_sub(input.len());
                print!("{:remaining$}", "", remaining = remaining);
            } else {
                let visible = &input[..available.saturating_sub(1)];
                print!("{}…", visible);
            }

            // Show help text if available
            let help = editor.explorer.action_help();
            if !help.is_empty() && height > 2 {
                let help_y = height.saturating_sub(2) as u16;
                execute!(self.stdout, cursor::MoveTo(0, help_y))?;
                execute!(self.stdout, SetBackgroundColor(prompt_bg))?;
                execute!(self.stdout, SetForegroundColor(line_num_color))?;
                let help_display = if help.len() > width {
                    format!("{}…", &help[..width.saturating_sub(1)])
                } else {
                    format!("{:width$}", help, width = width)
                };
                print!("{}", help_display);
            }
        }

        // Render search input if in search mode
        if editor.explorer.is_searching {
            let prompt_bg = explorer_bg;
            let prompt_y = height.saturating_sub(1) as u16;

            execute!(self.stdout, cursor::MoveTo(0, prompt_y))?;
            execute!(self.stdout, SetBackgroundColor(prompt_bg))?;

            // Search icon
            execute!(self.stdout, SetForegroundColor(theme.ui.statusline_mode_insert))?;
            print!("/");

            // Search buffer
            execute!(self.stdout, SetForegroundColor(explorer_fg))?;
            let search = &editor.explorer.search_buffer;

            let available = width.saturating_sub(1);
            if search.len() <= available {
                print!("{}", search);
                let match_info = editor.explorer.search_match_info();
                let padding = available.saturating_sub(search.len()).saturating_sub(match_info.len());
                print!("{:padding$}", "", padding = padding);
                execute!(self.stdout, SetForegroundColor(line_num_color))?;
                print!("{}", match_info);
            } else {
                let visible = &search[..available.saturating_sub(1)];
                print!("{}…", visible);
            }
        }

        // Draw vertical separator
        execute!(self.stdout, SetBackgroundColor(explorer_bg))?;
        execute!(self.stdout, SetForegroundColor(separator_color))?;
        for y in 0..height {
            execute!(self.stdout, cursor::MoveTo(width as u16, y as u16))?;
            print!("\u{2502}"); // │
        }
        execute!(self.stdout, SetForegroundColor(explorer_fg), SetBackgroundColor(explorer_bg))?;

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
                if editor.finder.is_normal_mode() {
                    // Hide cursor in normal mode - selection is shown visually
                    execute!(self.stdout, cursor::Hide)?;
                } else {
                    // Cursor in finder input line (at bottom of finder window)
                    let win = crate::finder::FloatingWindow::centered(editor.term_width, editor.term_height);
                    // Input line is 2 rows above the bottom border:
                    // bottom border at win.y + win.height - 1
                    // input line at win.y + win.height - 2
                    let input_y = win.y + win.height - 2;
                    // Cursor x: border(1) + mode indicator "[I] "(4) + "> "(2) + cursor position
                    let cursor_x = win.x + 1 + 6 + editor.finder.cursor as u16;
                    execute!(
                        self.stdout,
                        cursor::MoveTo(cursor_x, input_y),
                        cursor::Show,
                        cursor::SetCursorStyle::BlinkingBar
                    )?;
                }
            }
            Mode::Explorer => {
                // Show cursor in input/search modes, hide otherwise
                if editor.explorer.has_pending_action() {
                    let prompt = editor.explorer.action_prompt();
                    let cursor_x = prompt.len() + editor.explorer.input_cursor;
                    let cursor_y = editor.text_rows().saturating_sub(1) as u16;
                    execute!(
                        self.stdout,
                        cursor::MoveTo(cursor_x as u16, cursor_y),
                        cursor::Show,
                        cursor::SetCursorStyle::BlinkingBar
                    )?;
                } else if editor.explorer.is_searching {
                    let cursor_x = 1 + editor.explorer.search_cursor; // +1 for '/'
                    let cursor_y = editor.text_rows().saturating_sub(1) as u16;
                    execute!(
                        self.stdout,
                        cursor::MoveTo(cursor_x as u16, cursor_y),
                        cursor::Show,
                        cursor::SetCursorStyle::BlinkingBar
                    )?;
                } else {
                    execute!(self.stdout, cursor::Hide)?;
                }
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
        let theme = editor.theme();

        // Left side: mode and filename
        let mode_str = if editor.mode == Mode::Command {
            "NORMAL" // Show NORMAL in status while in command mode (like vim)
        } else {
            editor.mode.as_str()
        };

        // Get mode color from theme
        let mode_color = match editor.mode {
            Mode::Normal | Mode::Command => theme.ui.statusline_mode_normal,
            Mode::Insert => theme.ui.statusline_mode_insert,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => theme.ui.statusline_mode_visual,
            Mode::Replace => theme.ui.statusline_mode_replace,
            _ => theme.ui.statusline_mode_normal,
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

        // Show macro recording indicator
        let recording = if let Some(register) = editor.macros.recording_register() {
            format!(" [recording @{}]", register)
        } else {
            String::new()
        };

        // Get project name (last component of project_root)
        let project_name = editor.project_root.as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| format!("[{}] ", s))
            .unwrap_or_default();

        let mode_display = format!(" {} ", mode_str);
        let rest_left = format!("{}{} | {}{}{} ", pending, recording, project_name, filename, modified);

        // Right side: LSP status, language and position
        let lsp_status = editor.lsp_status.as_deref().unwrap_or("");
        let lang = editor.syntax.language_name().unwrap_or("plain");
        let right = if lsp_status.is_empty() {
            format!(" {} | {}:{} ", lang, editor.cursor.line + 1, editor.cursor.col + 1)
        } else {
            format!(" {} | {} | {}:{} ", lsp_status, lang, editor.cursor.line + 1, editor.cursor.col + 1)
        };

        // Calculate padding
        let left_len = mode_display.len() + rest_left.len();
        let padding = width.saturating_sub(left_len + right.len());

        // Render status line: mode badge (colored) + rest (status bar colors)
        // Mode badge with mode-specific color
        execute!(self.stdout, SetBackgroundColor(mode_color), SetForegroundColor(theme.ui.statusline_bg))?;
        print!("{}", mode_display);

        // Rest of status line with standard colors
        execute!(self.stdout, SetBackgroundColor(theme.ui.statusline_bg), SetForegroundColor(theme.ui.statusline_fg))?;
        print!("{}{:padding$}{}", rest_left, "", right, padding = padding);
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
                DiagnosticSeverity::Error => (Color::Red, "Error"),
                DiagnosticSeverity::Warning => (Color::Yellow, "Warning"),
                DiagnosticSeverity::Information => (Color::Blue, "Info"),
                DiagnosticSeverity::Hint => (Color::Cyan, "Hint"),
            };
            execute!(self.stdout, SetForegroundColor(color))?;
            // Take only the first line of the message (LSP messages can be multi-line)
            let first_line = diag.message.lines().next().unwrap_or(&diag.message);
            // Truncate message to fit terminal width (use chars count for proper Unicode handling)
            let max_len = editor.term_width as usize - prefix.len() - 3;
            let msg: String = first_line.chars().take(max_len).collect();
            let msg = if first_line.chars().count() > max_len {
                format!("{}...", &msg[..msg.len().saturating_sub(3)])
            } else {
                msg
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
        // Account for active pane's position on screen
        let active_pane = &editor.panes()[editor.active_pane_idx()];
        let pane_x = active_pane.rect.x;
        let pane_y = active_pane.rect.y;

        let line_num_width = editor.buffer().len_lines().to_string().len().max(3);
        let cursor_in_pane_col = (line_num_width + 1 + completion.trigger_col) as u16;
        let cursor_in_pane_row = (editor.cursor.line.saturating_sub(active_pane.viewport_offset)) as u16;

        // Convert to screen coordinates
        let popup_screen_col = pane_x + cursor_in_pane_col;
        let cursor_screen_row = pane_y + cursor_in_pane_row;

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
        // Account for active pane's position on screen
        let active_pane = &editor.panes()[editor.active_pane_idx()];
        let pane_x = active_pane.rect.x;
        let pane_y = active_pane.rect.y;

        let line_num_width = editor.buffer().len_lines().to_string().len().max(3);
        let cursor_in_pane_col = (line_num_width + 1 + editor.cursor.col) as u16;
        let cursor_in_pane_row = (editor.cursor.line - editor.viewport_offset) as u16;

        // Convert to screen coordinates
        let cursor_screen_col = pane_x + cursor_in_pane_col;
        let cursor_screen_row = pane_y + cursor_in_pane_row;

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

    /// Render the diagnostic floating popup (like vim.diagnostic.open_float())
    fn render_diagnostic_float(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let diagnostics = editor.diagnostics_for_line(editor.cursor.line);
        if diagnostics.is_empty() {
            return Ok(());
        }

        // Prepare diagnostic lines with numbers
        let mut lines: Vec<(Color, String)> = Vec::new();
        lines.push((Color::White, "Diagnostics:".to_string()));

        for (idx, diag) in diagnostics.iter().enumerate() {
            let (color, prefix) = match diag.severity {
                DiagnosticSeverity::Error => (Color::Red, ""),
                DiagnosticSeverity::Warning => (Color::Yellow, ""),
                DiagnosticSeverity::Information => (Color::Blue, ""),
                DiagnosticSeverity::Hint => (Color::Cyan, ""),
            };

            // Split message into lines and indent continuation lines
            for (line_idx, msg_line) in diag.message.lines().enumerate() {
                if line_idx == 0 {
                    lines.push((color, format!("{}. {} {}", idx + 1, prefix, msg_line)));
                } else {
                    // Indent continuation lines
                    lines.push((color, format!("   {}", msg_line)));
                }
            }
        }

        // Calculate popup dimensions
        let max_line_width = lines.iter().map(|(_, l)| l.chars().count()).max().unwrap_or(20);
        let popup_width = (max_line_width + 4).min(editor.term_width as usize - 4) as u16;
        let popup_height = (lines.len() + 2).min(editor.term_height as usize - 4) as u16;
        let content_width = (popup_width - 4) as usize;

        // Position popup below the cursor line
        let active_pane = &editor.panes()[editor.active_pane_idx()];
        let cursor_row = (editor.cursor.line.saturating_sub(active_pane.viewport_offset)) as u16;
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3);

        // Try to position below cursor, or above if not enough space
        let popup_y = if cursor_row + 1 + popup_height < editor.term_height.saturating_sub(2) {
            cursor_row + 1
        } else if cursor_row > popup_height {
            cursor_row - popup_height
        } else {
            1 // Fallback to near top
        };

        // Position at the left edge of text area
        let popup_x = (2 + line_num_width) as u16;

        // Colors
        let border_color = Color::Rgb { r: 100, g: 100, b: 140 };
        let bg_color = Color::Rgb { r: 30, g: 30, b: 45 };
        let title_color = Color::Rgb { r: 180, g: 180, b: 200 };

        // Draw top border with title
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╭");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╮");

        // Draw content lines
        let visible_lines = (popup_height - 2) as usize;
        for (i, (color, line)) in lines.iter().take(visible_lines).enumerate() {
            execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 1 + i as u16))?;
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│ ");

            // Determine text color based on line type
            let text_color = if i == 0 { title_color } else { *color };
            execute!(self.stdout, SetForegroundColor(text_color))?;

            // Truncate line to fit
            let display_line: String = line.chars().take(content_width).collect();
            print!("{}", display_line);

            // Pad remaining space
            let line_len = display_line.chars().count();
            if line_len < content_width {
                print!("{:width$}", "", width = content_width - line_len);
            }

            execute!(self.stdout, SetForegroundColor(border_color))?;
            print!(" │");
        }

        // Fill remaining rows if content is shorter than popup
        for i in lines.len()..visible_lines {
            execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 1 + i as u16))?;
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│ {:width$} │", "", width = content_width);
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

    /// Render the marks picker as an interactive popup
    fn render_marks_picker(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let picker = match &editor.marks_picker {
            Some(p) => p,
            None => return Ok(()),
        };

        if picker.items.is_empty() {
            return Ok(());
        }

        // Get theme colors
        let theme = &editor.theme_manager.current;
        let bg_color = theme.ui.finder_bg;
        let border_color = theme.ui.finder_border;
        let selection_bg = theme.ui.finder_selected;
        let text_color = theme.ui.foreground;
        let dim_color = Color::Rgb { r: 108, g: 112, b: 134 };
        let local_mark_color = Color::Rgb { r: 137, g: 180, b: 250 }; // Blue for local
        let global_mark_color = Color::Rgb { r: 249, g: 226, b: 175 }; // Yellow for global

        // Calculate popup dimensions
        let popup_width: u16 = 60.min(editor.term_width.saturating_sub(4)); // More room for file names
        let max_visible_items = 10.min(picker.items.len());
        let popup_height: u16 = (max_visible_items + 4) as u16; // +4 for top border, header, separator, bottom border
        let inner_width = (popup_width - 2) as usize; // Width inside the borders

        // Center the popup
        let popup_x = (editor.term_width.saturating_sub(popup_width)) / 2;
        let popup_y = (editor.term_height.saturating_sub(popup_height)) / 2;

        // Helper to draw a full-width line with borders
        let draw_line = |stdout: &mut std::io::Stdout, y: u16, content: &str, fg: Color, bg: Color, left_border: &str, right_border: &str| -> anyhow::Result<()> {
            execute!(stdout, cursor::MoveTo(popup_x, y))?;
            execute!(stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("{}", left_border);
            execute!(stdout, SetForegroundColor(fg), SetBackgroundColor(bg))?;
            let content_chars: Vec<char> = content.chars().collect();
            let content_len = content_chars.len();
            if content_len >= inner_width {
                let truncated: String = content_chars.into_iter().take(inner_width).collect();
                print!("{}", truncated);
            } else {
                print!("{}", content);
                print!("{:width$}", "", width = inner_width - content_len);
            }
            execute!(stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("{}", right_border);
            Ok(())
        };

        // Draw top border with title
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        let title = " Marks ";
        let border_inner = inner_width.saturating_sub(title.len());
        let left_dashes = border_inner / 2;
        let right_dashes = border_inner - left_dashes;
        print!("╭");
        for _ in 0..left_dashes { print!("─"); }
        execute!(self.stdout, SetForegroundColor(text_color))?;
        print!("{}", title);
        execute!(self.stdout, SetForegroundColor(border_color))?;
        for _ in 0..right_dashes { print!("─"); }
        print!("╮");

        // Draw header row
        let header = format!(" {:<4}{:>6}{:>5}  {:<}", "mark", "line", "col", "file");
        draw_line(&mut self.stdout, popup_y + 1, &header, dim_color, bg_color, "│", "│")?;

        // Draw separator
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 2))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("├");
        for _ in 0..inner_width { print!("─"); }
        print!("┤");

        // Calculate visible range based on selection
        let scroll_offset = if picker.selected >= max_visible_items {
            picker.selected - max_visible_items + 1
        } else {
            0
        };

        // Draw items
        for (i, item) in picker.items.iter().skip(scroll_offset).take(max_visible_items).enumerate() {
            let actual_idx = scroll_offset + i;
            let is_selected = actual_idx == picker.selected;
            let row_y = popup_y + 3 + i as u16;

            execute!(self.stdout, cursor::MoveTo(popup_x, row_y))?;

            // Left border
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");

            // Row background
            let row_bg = if is_selected { selection_bg } else { bg_color };
            execute!(self.stdout, SetBackgroundColor(row_bg))?;

            // Selection indicator
            if is_selected {
                execute!(self.stdout, SetForegroundColor(local_mark_color))?;
                print!(" ▸");
            } else {
                print!("  ");
            }

            // Mark name
            let mark_color = if item.is_global { global_mark_color } else { local_mark_color };
            execute!(self.stdout, SetForegroundColor(mark_color))?;
            print!("{:<3}", item.name);

            // Line and col
            execute!(self.stdout, SetForegroundColor(text_color))?;
            print!("{:>6}{:>5}  ", item.line + 1, item.col);

            // File name - calculate remaining space
            // Used so far: 2 (indicator) + 3 (mark) + 6 (line) + 5 (col) + 2 (spaces) = 18
            let file_max_len = inner_width.saturating_sub(18);
            let file_display: String = item.file_name.chars().take(file_max_len).collect();
            execute!(self.stdout, SetForegroundColor(dim_color))?;
            print!("{:<width$}", file_display, width = file_max_len);

            // Right border
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");
        }

        // Fill remaining rows if fewer items than max
        for i in picker.items.len()..max_visible_items {
            let row_y = popup_y + 3 + i as u16;
            execute!(self.stdout, cursor::MoveTo(popup_x, row_y))?;
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");
            print!("{:width$}", "", width = inner_width);
            print!("│");
        }

        // Draw bottom border with hints
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + popup_height - 1))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        let hint = " j/k Enter Esc ";
        let hint_inner = inner_width.saturating_sub(hint.len());
        let hint_left = hint_inner / 2;
        let hint_right = hint_inner - hint_left;
        print!("╰");
        for _ in 0..hint_left { print!("─"); }
        execute!(self.stdout, SetForegroundColor(dim_color))?;
        print!("{}", hint);
        execute!(self.stdout, SetForegroundColor(border_color))?;
        for _ in 0..hint_right { print!("─"); }
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
        let popup_width = 70u16.min(editor.term_width.saturating_sub(4));
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
            } else if i as usize > title_start && (i as usize) < title_start + title.len() {
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
            let content_width = (popup_width - 5) as usize; // │ + " 1 " + content + │
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

    /// Render the theme picker floating window
    fn render_theme_picker(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let picker = match &editor.theme_picker {
            Some(p) => p,
            None => return Ok(()),
        };

        let theme = editor.theme();

        // Calculate popup dimensions
        let popup_width = 60u16.min(editor.term_width.saturating_sub(4));
        let filtered_count = picker.filtered.len();
        let max_visible = 10usize; // Max visible items
        let visible_count = filtered_count.min(max_visible);
        // +5 for: top border, search input, separator, help, bottom border
        let popup_height = (visible_count as u16 + 5).max(7);

        // Center the popup
        let popup_x = (editor.term_width.saturating_sub(popup_width)) / 2;
        let popup_y = (editor.term_height.saturating_sub(popup_height)) / 2;

        // Colors from theme
        let border_color = theme.ui.finder_border;
        let bg_color = theme.ui.finder_bg;
        let selected_bg = theme.ui.finder_selected;
        let text_color = theme.ui.foreground;
        let user_color = theme.ui.line_number; // dimmer color for user themes indicator
        let prompt_color = theme.ui.finder_prompt;
        let match_color = theme.ui.finder_match;

        // Inner width = popup_width - 2 (for left and right borders)
        let inner_width = (popup_width - 2) as usize;

        // Draw top border with title
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        let title = " Themes ";
        let title_start = (popup_width as usize - title.len()) / 2;
        print!("╭");
        for i in 1..(popup_width - 1) {
            if i as usize == title_start {
                print!("{}", title);
            } else if i as usize > title_start && (i as usize) < title_start + title.len() {
                // Skip - title already printed
            } else {
                print!("─");
            }
        }
        print!("╮");

        // Draw search input line
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 1))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("│");
        execute!(self.stdout, SetForegroundColor(prompt_color))?;
        print!(" > ");
        execute!(self.stdout, SetForegroundColor(text_color))?;
        let query_max = inner_width.saturating_sub(3); // " > " prefix
        let query_display: String = picker.query.chars().take(query_max).collect();
        print!("{}", query_display);
        // Padding to fill rest of line
        let query_padding = inner_width.saturating_sub(3 + query_display.len());
        print!("{:width$}", "", width = query_padding);
        execute!(self.stdout, SetForegroundColor(border_color))?;
        print!("│");

        // Draw theme items
        let scroll_offset = if picker.selected >= visible_count {
            picker.selected - visible_count + 1
        } else {
            0
        };

        for row in 0..visible_count {
            let list_idx = scroll_offset + row;
            execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 2 + row as u16))?;

            // Left border
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");

            if list_idx < filtered_count {
                let item_idx = picker.filtered[list_idx];
                let (name, is_bundled) = &picker.all_items[item_idx];
                let is_selected = list_idx == picker.selected;
                let current_bg = if is_selected { selected_bg } else { bg_color };

                execute!(self.stdout, SetBackgroundColor(current_bg))?;

                // Build the line content: " > name     " or "   name     "
                let prefix = if is_selected { " > " } else { "   " };
                let suffix = if !*is_bundled { " (user)" } else { "" };

                // Available space for name
                let name_max = inner_width.saturating_sub(prefix.len() + suffix.len() + 1); // +1 for trailing space
                let display_name: String = if name.len() > name_max {
                    name.chars().take(name_max.saturating_sub(1)).collect::<String>() + "…"
                } else {
                    name.clone()
                };

                // Selection indicator
                if is_selected {
                    execute!(self.stdout, SetForegroundColor(match_color))?;
                } else {
                    execute!(self.stdout, SetForegroundColor(text_color))?;
                }
                print!("{}", prefix);

                // Theme name - highlight matching characters if there's a query
                if !picker.query.is_empty() {
                    let query_lower = picker.query.to_lowercase();
                    let name_lower = display_name.to_lowercase();
                    if let Some(match_start) = name_lower.find(&query_lower) {
                        // Before match
                        execute!(self.stdout, SetForegroundColor(text_color))?;
                        print!("{}", &display_name[..match_start]);
                        // Match
                        execute!(self.stdout, SetForegroundColor(match_color))?;
                        print!("{}", &display_name[match_start..match_start + picker.query.len()]);
                        // After match
                        execute!(self.stdout, SetForegroundColor(text_color))?;
                        print!("{}", &display_name[match_start + picker.query.len()..]);
                    } else {
                        execute!(self.stdout, SetForegroundColor(text_color))?;
                        print!("{}", display_name);
                    }
                } else {
                    execute!(self.stdout, SetForegroundColor(text_color))?;
                    print!("{}", display_name);
                }

                // User indicator or padding
                if !*is_bundled {
                    execute!(self.stdout, SetForegroundColor(user_color))?;
                    let padding = inner_width.saturating_sub(prefix.len() + display_name.len() + suffix.len());
                    print!("{:width$}{}", "", suffix, width = padding);
                } else {
                    let padding = inner_width.saturating_sub(prefix.len() + display_name.len());
                    print!("{:width$}", "", width = padding);
                }
            } else {
                // Empty row
                execute!(self.stdout, SetBackgroundColor(bg_color))?;
                print!("{:width$}", "", width = inner_width);
            }

            // Right border
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");
        }

        // Draw separator line (immediately after items)
        let separator_y = popup_y + 2 + visible_count as u16;
        execute!(self.stdout, cursor::MoveTo(popup_x, separator_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("├");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("┤");

        // Draw help line with count
        execute!(self.stdout, cursor::MoveTo(popup_x, separator_y + 1))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("│");
        execute!(self.stdout, SetForegroundColor(user_color), SetBackgroundColor(bg_color))?;
        let count_str = format!("{}/{}", filtered_count, picker.all_items.len());
        let help = "Type to filter • j/k nav • Enter";
        let combined = format!("{} {}", help, count_str);
        print!("{:^width$}", combined, width = inner_width);
        execute!(self.stdout, SetForegroundColor(border_color))?;
        print!("│");

        // Draw bottom border
        execute!(self.stdout, cursor::MoveTo(popup_x, separator_y + 2))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╰");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, SetForegroundColor(text_color), SetBackgroundColor(bg_color))?;
        Ok(())
    }

    /// Render the floating terminal
    fn render_floating_terminal(&mut self, editor: &Editor) -> anyhow::Result<()> {
        // Calculate terminal dimensions (60% of screen, centered)
        let term_width = (editor.term_width as f32 * 0.6) as u16;
        let term_height = (editor.term_height as f32 * 0.6) as u16;

        // Ensure minimum size
        let term_width = term_width.max(40);
        let term_height = term_height.max(10);

        // Center the terminal
        let term_x = (editor.term_width.saturating_sub(term_width)) / 2;
        let term_y = (editor.term_height.saturating_sub(term_height)) / 2;

        // Colors
        let border_color = Color::Rgb { r: 100, g: 180, b: 100 };
        let bg_color = Color::Rgb { r: 20, g: 20, b: 25 };
        let text_color = Color::Rgb { r: 200, g: 200, b: 200 };
        let title_color = Color::Rgb { r: 150, g: 220, b: 150 };

        // Draw top border with title
        execute!(self.stdout, cursor::MoveTo(term_x, term_y))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        let title = " Terminal ";
        let close_hint = " [<C-\\>] ";
        let title_start = 2usize;
        let close_start = (term_width as usize).saturating_sub(close_hint.len() + 2);

        print!("╭");
        for i in 1..(term_width - 1) {
            let i = i as usize;
            if i == title_start {
                execute!(self.stdout, SetForegroundColor(title_color))?;
                print!("{}", title);
                execute!(self.stdout, SetForegroundColor(border_color))?;
            } else if i > title_start && i < title_start + title.len() {
                // Skip - title already printed
            } else if i == close_start {
                execute!(self.stdout, SetForegroundColor(Color::Rgb { r: 100, g: 100, b: 110 }))?;
                print!("{}", close_hint);
                execute!(self.stdout, SetForegroundColor(border_color))?;
            } else if i > close_start && i < close_start + close_hint.len() {
                // Skip - close hint already printed
            } else {
                print!("─");
            }
        }
        print!("╮");

        // Get terminal content
        let content_height = (term_height - 2) as usize;
        let content_width = (term_width - 2) as usize;
        let lines = editor.floating_terminal.get_visible_lines(content_height, content_width);
        let (cursor_row, cursor_col) = editor.floating_terminal.get_cursor_pos();

        // Draw terminal content
        for (row, line) in lines.iter().enumerate() {
            execute!(self.stdout, cursor::MoveTo(term_x, term_y + 1 + row as u16))?;
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");

            execute!(self.stdout, SetForegroundColor(text_color), SetBackgroundColor(bg_color))?;

            // Print line content, padding to fill width
            let display: String = line.chars().take(content_width).collect();
            print!("{:<width$}", display, width = content_width);

            execute!(self.stdout, SetForegroundColor(border_color))?;
            print!("│");
        }

        // Fill remaining rows if content is shorter
        for row in lines.len()..content_height {
            execute!(self.stdout, cursor::MoveTo(term_x, term_y + 1 + row as u16))?;
            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
            print!("│");
            execute!(self.stdout, SetForegroundColor(text_color))?;
            print!("{:<width$}", "", width = content_width);
            execute!(self.stdout, SetForegroundColor(border_color))?;
            print!("│");
        }

        // Draw bottom border
        execute!(self.stdout, cursor::MoveTo(term_x, term_y + term_height - 1))?;
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(bg_color))?;
        print!("╰");
        for _ in 1..(term_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, ResetColor)?;

        // Position cursor inside the terminal
        let cursor_x = term_x + 1 + cursor_col.min(content_width.saturating_sub(1)) as u16;
        let cursor_y = term_y + 1 + cursor_row.min(content_height.saturating_sub(1)) as u16;
        execute!(self.stdout, cursor::MoveTo(cursor_x, cursor_y))?;

        Ok(())
    }

    /// Render only the floating terminal overlay (for efficient updates)
    pub fn render_terminal_only(&mut self, editor: &Editor) -> anyhow::Result<()> {
        if editor.floating_terminal.is_visible() {
            self.render_floating_terminal(editor)?;
            self.stdout.flush()?;
        }
        Ok(())
    }

    /// Render the fuzzy finder floating window
    fn render_finder(&mut self, editor: &Editor) -> anyhow::Result<()> {
        use crate::finder::FuzzyFinder;

        let win = crate::finder::FloatingWindow::centered(editor.term_width, editor.term_height);

        // Use theme colors for finder
        let theme = editor.theme();
        let border_color = theme.ui.finder_border;
        let title_color = theme.ui.finder_prompt;
        let selected_bg = theme.ui.finder_selected;
        let input_bg = theme.ui.finder_bg;
        let finder_bg = theme.ui.finder_bg;
        let finder_fg = theme.ui.foreground;
        let match_color = theme.ui.finder_match;
        let mode_color = if editor.finder.is_normal_mode() {
            theme.ui.statusline_mode_normal // Use normal mode color
        } else {
            theme.ui.statusline_mode_insert // Use insert mode color
        };

        // Draw top border with title
        execute!(self.stdout, cursor::MoveTo(win.x, win.y), SetForegroundColor(border_color))?;
        print!("\u{250c}"); // ┌
        let title = match editor.finder.mode {
            crate::finder::FinderMode::Files => " Find Files ",
            crate::finder::FinderMode::Grep => " Live Grep ",
            crate::finder::FinderMode::Buffers => " Buffers ",
            crate::finder::FinderMode::Diagnostics => " Diagnostics ",
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

        // Layout: top border, items list, separator, input line, bottom border
        // Height calculation: total height - 4 (top border, separator, input, bottom border)
        let list_height = (win.height - 4) as usize;
        let total_items = editor.finder.filtered.len();
        let scroll_offset = editor.finder.scroll_offset;

        // Calculate scroll indicator
        let show_scroll_indicator = total_items > list_height;
        let scroll_indicator_color = Color::DarkGrey;

        // Draw items with scrolling (starts at row 1, after top border)
        // Telescope-style: best matches at BOTTOM (closest to input)
        let visible_count = list_height.min(total_items.saturating_sub(scroll_offset));
        let first_item_row = list_height - visible_count; // Empty rows above items

        for row in 0..list_height {
            let y = win.y + 1 + row as u16;
            execute!(self.stdout, cursor::MoveTo(win.x, y), SetForegroundColor(border_color), SetBackgroundColor(finder_bg))?;
            print!("\u{2502}"); // │

            // Reverse display: row 0 = least relevant, bottom row = best match
            let list_idx = if row >= first_item_row {
                // Map row to item index (reversed)
                scroll_offset + (list_height - 1 - row)
            } else {
                usize::MAX // Empty row marker
            };

            if list_idx < total_items {
                let item_idx = editor.finder.filtered[list_idx];
                let item = &editor.finder.items[item_idx];
                let is_selected = list_idx == editor.finder.selected;

                if is_selected {
                    execute!(self.stdout, SetBackgroundColor(selected_bg))?;
                }

                // Get file type indicator (2 chars + space)
                let icon = FuzzyFinder::get_file_icon(&item.path);
                let icon_color = match icon {
                    "RS" => Color::Rgb { r: 255, g: 100, b: 50 },   // Rust - orange
                    "TS" | "TX" => Color::Rgb { r: 50, g: 150, b: 255 }, // TypeScript - blue
                    "JS" | "JX" => Color::Rgb { r: 255, g: 220, b: 50 }, // JavaScript - yellow
                    "PY" => Color::Rgb { r: 80, g: 180, b: 80 },    // Python - green
                    "GO" => Color::Rgb { r: 100, g: 200, b: 220 },  // Go - cyan
                    "RB" => Color::Rgb { r: 220, g: 50, b: 50 },    // Ruby - red
                    "HT" => Color::Rgb { r: 230, g: 100, b: 50 },   // HTML - orange
                    "CS" | "SC" => Color::Rgb { r: 100, g: 150, b: 255 }, // CSS - blue
                    "MD" => Color::Rgb { r: 150, g: 150, b: 150 },  // Markdown - gray
                    "YM" | "TM" | "CF" => Color::Rgb { r: 180, g: 140, b: 100 }, // Config - tan
                    "GT" => Color::Rgb { r: 240, g: 80, b: 50 },    // Git - red-orange
                    "EN" => Color::Rgb { r: 255, g: 200, b: 50 },   // Env - yellow
                    "SH" | "ZS" | "FS" => Color::Rgb { r: 100, g: 200, b: 100 }, // Shell - green
                    _ => Color::Rgb { r: 120, g: 120, b: 120 },     // Default - gray
                };
                execute!(self.stdout, SetForegroundColor(icon_color))?;
                print!("{} ", icon);

                // Truncate display to fit and highlight matches
                // Leave space for icon (3 chars) and scroll indicator if needed
                let icon_width = 3; // icon + space
                let base_width = if show_scroll_indicator {
                    (win.width as usize).saturating_sub(3) // left border + scroll + right border
                } else {
                    (win.width as usize).saturating_sub(2) // left + right borders only
                };
                let max_len = base_width.saturating_sub(icon_width);
                let display_chars: Vec<char> = item.display.chars().take(max_len).collect();

                // Reset foreground color for text
                if is_selected {
                    execute!(self.stdout, SetForegroundColor(finder_fg))?;
                } else {
                    execute!(self.stdout, SetForegroundColor(finder_fg), SetBackgroundColor(finder_bg))?;
                }

                // For diagnostics mode, color the severity indicator
                let is_diagnostics_mode = editor.finder.mode == crate::finder::FinderMode::Diagnostics;
                let mut skip_severity_coloring = 0;

                // Check for severity prefix and render it with color
                if is_diagnostics_mode && display_chars.len() >= 3 {
                    let prefix: String = display_chars[0..3].iter().collect();
                    let severity_color = match prefix.as_str() {
                        "[E]" => Some(Color::Rgb { r: 255, g: 80, b: 80 }),   // Red for errors
                        "[W]" => Some(Color::Rgb { r: 255, g: 200, b: 50 }),  // Yellow for warnings
                        "[I]" => Some(Color::Rgb { r: 100, g: 180, b: 255 }), // Blue for info
                        "[H]" => Some(Color::Rgb { r: 150, g: 150, b: 150 }), // Gray for hints
                        _ => None,
                    };

                    if let Some(color) = severity_color {
                        execute!(self.stdout, SetForegroundColor(color))?;
                        print!("{}", prefix);
                        skip_severity_coloring = 3;

                        // Reset to normal text color
                        if is_selected {
                            execute!(self.stdout, SetForegroundColor(finder_fg), SetBackgroundColor(selected_bg))?;
                        } else {
                            execute!(self.stdout, SetForegroundColor(finder_fg), SetBackgroundColor(finder_bg))?;
                        }
                    }
                }

                for (char_idx, ch) in display_chars.iter().enumerate() {
                    // Skip chars already printed as severity indicator
                    if char_idx < skip_severity_coloring {
                        continue;
                    }

                    if item.match_indices.contains(&char_idx) {
                        execute!(self.stdout, SetForegroundColor(match_color))?;
                        print!("{}", ch);
                        if is_selected {
                            execute!(self.stdout, SetForegroundColor(finder_fg), SetBackgroundColor(selected_bg))?;
                        } else {
                            execute!(self.stdout, SetForegroundColor(finder_fg), SetBackgroundColor(finder_bg))?;
                        }
                    } else {
                        print!("{}", ch);
                    }
                }

                // Pad to fill line
                for _ in display_chars.len()..max_len {
                    print!(" ");
                }

                // Reset after selected item
                if is_selected {
                    execute!(self.stdout, SetForegroundColor(finder_fg), SetBackgroundColor(finder_bg))?;
                }
            } else {
                // Empty row - set finder background
                execute!(self.stdout, SetBackgroundColor(finder_bg))?;
                let pad_len = if show_scroll_indicator {
                    (win.width as usize).saturating_sub(3) // left border + scroll + right border
                } else {
                    (win.width as usize).saturating_sub(2) // left + right borders only
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
                    execute!(self.stdout, SetForegroundColor(title_color))?;
                    print!("\u{2588}"); // █ (full block for thumb)
                } else if scroll_offset > 0 || scroll_offset + list_height < total_items {
                    execute!(self.stdout, SetForegroundColor(scroll_indicator_color))?;
                    print!("\u{2591}"); // ░ (light shade for track)
                } else {
                    print!(" ");
                }
            }

            execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(finder_bg))?;
            print!("\u{2502}"); // │
        }

        // Draw separator above input
        let sep_y = win.y + 1 + list_height as u16;
        execute!(self.stdout, cursor::MoveTo(win.x, sep_y), SetForegroundColor(border_color))?;
        print!("\u{251c}"); // ├
        for _ in 1..(win.width - 1) {
            print!("\u{2500}"); // ─
        }
        print!("\u{2524}"); // ┤

        // Draw input line (above bottom border)
        let input_y = sep_y + 1;
        execute!(self.stdout, cursor::MoveTo(win.x, input_y), SetForegroundColor(border_color), SetBackgroundColor(finder_bg))?;
        print!("\u{2502}"); // │
        execute!(self.stdout, SetBackgroundColor(input_bg))?;

        // Mode indicator
        let mode_str = if editor.finder.is_normal_mode() { "N" } else { "I" };
        execute!(self.stdout, SetForegroundColor(mode_color))?;
        print!("[{}]", mode_str);
        execute!(self.stdout, SetForegroundColor(finder_fg))?;
        print!(" > ");

        // Query text
        let prefix_len = 6; // "[N] > " or "[I] > "
        let query_display: String = editor.finder.query.chars().take((win.width - prefix_len as u16 - 3) as usize).collect();
        print!("{}", query_display);

        // Pad to fill line (terminal cursor will be positioned separately)
        let used = prefix_len + query_display.len();
        for _ in used..(win.width - 2) as usize {
            print!(" ");
        }
        execute!(self.stdout, SetForegroundColor(border_color), SetBackgroundColor(finder_bg))?;
        print!("\u{2502}"); // │

        // Draw bottom border with count indicator
        let status = format!(" {}/{} ", editor.finder.filtered.len(), editor.finder.items.len());
        execute!(self.stdout, cursor::MoveTo(win.x, win.y + win.height - 1), SetForegroundColor(border_color))?;
        print!("\u{2514}"); // └
        let status_start = (win.width as usize - status.len()) / 2;
        for i in 1..(win.width - 1) {
            if i as usize == status_start {
                execute!(self.stdout, SetForegroundColor(theme.ui.line_number))?;
                print!("{}", status);
                execute!(self.stdout, SetForegroundColor(border_color))?;
            } else if i as usize >= status_start && (i as usize) < status_start + status.len() {
                // Skip - already printed status
            } else {
                print!("\u{2500}"); // ─
            }
        }
        print!("\u{2518}"); // ┘

        execute!(self.stdout, SetForegroundColor(finder_fg), SetBackgroundColor(finder_bg))?;
        Ok(())
    }

    /// Render a line with syntax highlighting and optional visual selection
    /// Now accepts theme colors to maintain proper background
    fn render_line_with_highlights(
        &mut self,
        line: &str,
        line_idx: usize,
        highlights: &[HighlightSpan],
        visual_range: Option<(usize, usize, usize, usize)>,
        mode: &Mode,
        is_cursor_line: bool,
        search_matches: &[(usize, usize, usize)],
        diagnostics: &[&Diagnostic],
        editor_bg: Color,
        editor_fg: Color,
        cursor_line_bg: Color,
        selection_bg: Color,
        search_match_bg: Color,
        search_match_fg: Color,
        diagnostic_error_color: Color,
        diagnostic_warning_color: Color,
        diagnostic_info_color: Color,
        diagnostic_hint_color: Color,
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

        // Determine the base background for this line (cursor line or editor background)
        let base_bg = if is_cursor_line { cursor_line_bg } else { editor_bg };

        // Check if a column is within a search match for this line
        let in_search_match = |col: usize| -> bool {
            search_matches.iter().any(|(l, start, end)| {
                *l == line_idx && col >= *start && col < *end
            })
        };

        // Find diagnostic at this column (prioritize by severity: Error > Warning > Info > Hint)
        let get_diagnostic_at = |col: usize| -> Option<&Diagnostic> {
            diagnostics.iter()
                .filter(|d| col >= d.col_start && col < d.col_end)
                .min_by_key(|d| match d.severity {
                    DiagnosticSeverity::Error => 0,
                    DiagnosticSeverity::Warning => 1,
                    DiagnosticSeverity::Information => 2,
                    DiagnosticSeverity::Hint => 3,
                })
                .copied()
        };

        // Render character by character
        let mut highlight_idx = 0;
        let mut current_fg: Option<Color> = None;
        let mut current_bg: Option<Color> = None;
        let mut current_underline: Option<Color> = None;
        for (col, ch) in chars.iter().enumerate() {
            // Find syntax color for this column
            let syntax_color = Self::get_syntax_color_at(highlights, col, &mut highlight_idx);

            // Check if in visual selection
            let is_selected = in_selection && col >= sel_start && col < sel_end;

            // Check if in search match
            let is_search_match = in_search_match(col);

            // Check for diagnostic underline
            let diag_at_col = get_diagnostic_at(col);
            let diag_underline_color = diag_at_col.map(|d| match d.severity {
                DiagnosticSeverity::Error => diagnostic_error_color,
                DiagnosticSeverity::Warning => diagnostic_warning_color,
                DiagnosticSeverity::Information => diagnostic_info_color,
                DiagnosticSeverity::Hint => diagnostic_hint_color,
            });

            // Priority: visual selection > search match > base (cursor line or editor bg)
            let (desired_bg, desired_fg) = if is_selected {
                (selection_bg, syntax_color.unwrap_or(editor_fg))
            } else if is_search_match {
                (search_match_bg, search_match_fg)
            } else {
                (base_bg, syntax_color.unwrap_or(editor_fg))
            };

            // Only change colors when necessary
            if Some(desired_bg) != current_bg {
                execute!(self.stdout, SetBackgroundColor(desired_bg))?;
                current_bg = Some(desired_bg);
            }
            if Some(desired_fg) != current_fg {
                execute!(self.stdout, SetForegroundColor(desired_fg))?;
                current_fg = Some(desired_fg);
            }

            // Handle underline attribute changes for diagnostics
            if diag_underline_color != current_underline {
                if let Some(color) = diag_underline_color {
                    // Use colored underline (keeps syntax highlighting intact)
                    execute!(
                        self.stdout,
                        SetUnderlineColor(color),
                        SetAttribute(Attribute::Underlined)
                    )?;
                } else {
                    execute!(self.stdout, SetAttribute(Attribute::NoUnderline))?;
                }
                current_underline = diag_underline_color;
            }

            print!("{}", ch);
        }

        // Handle selection extending past line end
        if in_selection && sel_end > line_len {
            execute!(self.stdout, SetBackgroundColor(selection_bg))?;
            print!(" ");
        }

        // Restore to base background/foreground and reset underline
        execute!(
            self.stdout,
            SetAttribute(Attribute::NoUnderline),
            SetBackgroundColor(base_bg),
            SetForegroundColor(editor_fg)
        )?;

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

    /// Read an event (blocking for the next event)
    /// Returns EditorEvent for key presses and focus changes
    pub fn read_event(&self) -> anyhow::Result<Option<EditorEvent>> {
        match event::read()? {
            Event::Key(key_event) => Ok(Some(EditorEvent::Key(key_event))),
            Event::FocusGained => Ok(Some(EditorEvent::FocusGained)),
            _ => Ok(None),
        }
    }

    /// Read a key event (blocking for the next event)
    /// Deprecated: prefer read_event() for autoread support
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
            event::DisableFocusChange,
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

/// Handle key input for the marks picker
fn handle_marks_picker_key(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Close picker
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) |
        (KeyModifiers::NONE, KeyCode::Char('q')) => {
            editor.marks_picker = None;
        }

        // Navigate up
        (KeyModifiers::NONE, KeyCode::Up) |
        (KeyModifiers::NONE, KeyCode::Char('k')) |
        (KeyModifiers::CONTROL, KeyCode::Char('k')) |
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            if let Some(picker) = &mut editor.marks_picker {
                picker.move_up();
            }
        }

        // Navigate down
        (KeyModifiers::NONE, KeyCode::Down) |
        (KeyModifiers::NONE, KeyCode::Char('j')) |
        (KeyModifiers::CONTROL, KeyCode::Char('j')) |
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            if let Some(picker) = &mut editor.marks_picker {
                picker.move_down();
            }
        }

        // Jump to selected mark
        (KeyModifiers::NONE, KeyCode::Enter) |
        (KeyModifiers::NONE, KeyCode::Char('l')) => {
            if let Some(picker) = &editor.marks_picker {
                if let Some(mark) = picker.selected_mark() {
                    let line = mark.line;
                    let col = mark.col;
                    let file_path = mark.file_path.clone();
                    let is_global = mark.is_global;

                    // Close picker first
                    editor.marks_picker = None;

                    // If it's a global mark with a different file, open it
                    if is_global {
                        if let Some(path) = file_path {
                            let current_path = editor.buffer().path.clone();
                            if current_path.as_ref() != Some(&path) {
                                // Open the file
                                if let Err(e) = editor.open_file(path.clone()) {
                                    editor.set_status(format!("Error opening file: {}", e));
                                    return;
                                }
                            }
                        }
                    }

                    // Record jump before moving
                    editor.record_jump();

                    // Jump to position
                    editor.cursor.line = line;
                    editor.cursor.col = col;
                    editor.scroll_cursor_center();
                }
            } else {
                editor.marks_picker = None;
            }
        }

        // Go to top
        (KeyModifiers::NONE, KeyCode::Char('g')) => {
            if let Some(picker) = &mut editor.marks_picker {
                picker.selected = 0;
            }
        }

        // Go to bottom
        (KeyModifiers::SHIFT, KeyCode::Char('G')) => {
            if let Some(picker) = &mut editor.marks_picker {
                if !picker.items.is_empty() {
                    picker.selected = picker.items.len() - 1;
                }
            }
        }

        _ => {}
    }
}

/// Handle key input for the theme picker
fn handle_theme_picker_key(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Close picker (cancel)
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
            editor.close_theme_picker(false); // Cancel
        }

        // Navigate up (Ctrl-k or Ctrl-p, since j/k are now for typing)
        (KeyModifiers::NONE, KeyCode::Up) |
        (KeyModifiers::CONTROL, KeyCode::Char('k')) |
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            if let Some(picker) = &mut editor.theme_picker {
                picker.move_up();
                // Preview the theme
                if let Some(name) = picker.selected_name() {
                    let name = name.to_string();
                    editor.preview_theme(&name);
                }
            }
        }

        // Navigate down (Ctrl-j or Ctrl-n, since j/k are now for typing)
        (KeyModifiers::NONE, KeyCode::Down) |
        (KeyModifiers::CONTROL, KeyCode::Char('j')) |
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            if let Some(picker) = &mut editor.theme_picker {
                picker.move_down();
                // Preview the theme
                if let Some(name) = picker.selected_name() {
                    let name = name.to_string();
                    editor.preview_theme(&name);
                }
            }
        }

        // Select theme (Enter)
        (KeyModifiers::NONE, KeyCode::Enter) => {
            if let Some(picker) = &editor.theme_picker {
                if picker.filtered.is_empty() {
                    // No matches, don't close
                    return;
                }
                if let Some(name) = picker.selected_name() {
                    let name = name.to_string();
                    editor.set_theme(&name);
                    // Save theme to config
                    match crate::config::save_theme(&name) {
                        Ok(()) => editor.set_status(format!("Theme set to '{}' (saved)", name)),
                        Err(e) => editor.set_status(format!("Theme set to '{}' (save failed: {})", name, e)),
                    }
                }
            }
            editor.close_theme_picker(true); // Confirm
        }

        // Backspace - delete character from query
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            if let Some(picker) = &mut editor.theme_picker {
                picker.delete_char();
                // Preview first matching theme
                if let Some(name) = picker.selected_name() {
                    let name = name.to_string();
                    editor.preview_theme(&name);
                }
            }
        }

        // Type character - add to query
        (KeyModifiers::NONE, KeyCode::Char(c)) |
        (KeyModifiers::SHIFT, KeyCode::Char(c)) => {
            if let Some(picker) = &mut editor.theme_picker {
                picker.add_char(c);
                // Preview first matching theme
                if let Some(name) = picker.selected_name() {
                    let name = name.to_string();
                    editor.preview_theme(&name);
                }
            }
        }

        _ => {}
    }
}

/// Play a macro from a register
fn play_macro(editor: &mut Editor, register: char, count: usize) {
    // Get the macro keys (clone to avoid borrow issues)
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

    // Wrap the entire playback in an undo group
    editor.undo_stack.begin_undo_group(editor.cursor.line, editor.cursor.col);

    // Play the macro `count` times
    for _ in 0..count {
        for key in &keys {
            // Process each key - note: we DON'T record during playback
            // because is_recording() will be false
            handle_key(editor, *key);
        }
    }

    editor.undo_stack.end_undo_group(editor.cursor.line, editor.cursor.col);
}

/// Handle a key event and update editor state
pub fn handle_key(editor: &mut Editor, key: KeyEvent) {
    // Check for floating terminal toggle (Ctrl-\) - works in any mode
    // Note: Ctrl-\ sends ASCII 28 (File Separator) on Unix terminals
    // We check for both the character and the raw control code
    let is_ctrl_backslash = match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('\\')) => true,
        (KeyModifiers::CONTROL, KeyCode::Char('4')) => true,  // Ctrl-4 = Ctrl-\ on some terminals
        (_, KeyCode::Char('\x1c')) => true,  // ASCII 28 = File Separator (Ctrl-\)
        _ => false,
    };
    if is_ctrl_backslash {
        editor.floating_terminal.toggle();
        return;
    }

    // If floating terminal is visible, handle its keys
    if editor.floating_terminal.is_visible() {
        // Escape closes the terminal (alternative to Ctrl-\)
        if key.code == KeyCode::Esc && key.modifiers.is_empty() {
            editor.floating_terminal.toggle();
            return;
        }
        // All other keys go to the terminal
        editor.floating_terminal.send_key(key);
        return;
    }

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

    // Handle marks picker if active
    if editor.marks_picker.is_some() {
        handle_marks_picker_key(editor, key);
        return;
    }

    // Handle theme picker if active
    if editor.theme_picker.is_some() {
        handle_theme_picker_key(editor, key);
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

    // Handle macro recording
    if editor.macros.is_recording() {
        // Check if 'q' is pressed in Normal mode to stop recording
        if editor.mode == Mode::Normal
            && key.code == KeyCode::Char('q')
            && key.modifiers == KeyModifiers::NONE
        {
            editor.macros.stop_recording();
            editor.set_status("Recording stopped");
            return;
        }
        // Record the key (we record before processing so all keys including motions are captured)
        editor.macros.record_key(key);
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
    let t_start = std::time::Instant::now();

    // Handle leader key sequences
    if let Some(ref mut sequence) = editor.leader_sequence {
        // We're in leader mode, accumulating a sequence
        // Escape cancels leader mode
        if key.code == KeyCode::Esc {
            editor.leader_sequence = None;
            editor.leader_sequence_start = None;
            editor.leader_pending_action = None;
            editor.clear_status();
            return;
        }

        // Convert key to character and append
        if let KeyCode::Char(c) = key.code {
            sequence.push(c);
            let seq = sequence.clone();

            // New key typed - reset timeout tracking (sequence changed)
            editor.leader_sequence_start = Some(Instant::now());
            editor.leader_pending_action = None;

            // Check for exact match
            let exact_match = editor.keymap.get_leader_action(&seq).cloned();
            let is_prefix = editor.keymap.is_leader_prefix(&seq);

            match (exact_match, is_prefix) {
                (Some(action), true) => {
                    // Exact match AND prefix of longer mapping
                    // Store pending action and wait for timeout or more input
                    editor.leader_pending_action = Some(action);
                    editor.set_status(format!("<leader>{} (waiting...)", seq));
                }
                (Some(action), false) => {
                    // Exact match only, no longer mappings - execute immediately
                    editor.leader_sequence = None;
                    editor.leader_sequence_start = None;
                    editor.leader_pending_action = None;
                    editor.clear_status();
                    execute_leader_action(editor, &action);
                    return;
                }
                (None, true) => {
                    // No exact match but could be prefix - wait for more input
                    editor.set_status(format!("<leader>{}", seq));
                }
                (None, false) => {
                    // No match and not a prefix - cancel leader mode
                    editor.leader_sequence = None;
                    editor.leader_sequence_start = None;
                    editor.leader_pending_action = None;
                    editor.clear_status();
                }
            }
            return;
        }

        // Non-character key in leader mode - cancel
        editor.leader_sequence = None;
        editor.leader_sequence_start = None;
        editor.leader_pending_action = None;
        editor.clear_status();
        return;
    }

    // Check if this key is the leader key
    let t_leader_check = std::time::Instant::now();
    if editor.keymap.has_leader_mappings() {
        if editor.keymap.is_leader_key(key) {
            editor.leader_sequence = Some(String::new());
            editor.leader_sequence_start = Some(Instant::now());
            editor.leader_pending_action = None;
            editor.set_status("<leader>");
            return;
        }
    }
    let leader_check_elapsed = t_leader_check.elapsed();

    // Check for normal mode custom mapping first
    if let Some(mapping) = editor.keymap.get_normal_mapping(key) {
        let mapping = mapping.clone();
        let t_exec = std::time::Instant::now();
        execute_leader_action(editor, &mapping);
        let total = t_start.elapsed();
        if total.as_micros() > 1000 {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open("/tmp/nevi_debug.log") {
                let _ = writeln!(f, "SLOW custom_mapping: total={:?} exec={:?} key={:?}", total, t_exec.elapsed(), key.code);
            }
        }
        return;
    }

    let t_process = std::time::Instant::now();
    let action = editor.input_state.process_normal_key(key);
    let process_elapsed = t_process.elapsed();

    let t_action = std::time::Instant::now();
    match action {
        KeyAction::Pending => {
            // Key was consumed, waiting for more input
        }

        KeyAction::Motion(motion, count) => {
            editor.apply_motion(motion, count);
            // Clear search highlights on non-search movement (like Neovim)
            editor.clear_search_highlights();
        }

        KeyAction::OperatorMotion(op, motion, count) => {
            let register = editor.input_state.take_register();
            match op {
                Operator::Delete => editor.delete_motion(motion, count, register),
                Operator::Change => editor.change_motion(motion, count, register),
                Operator::Yank => editor.yank_motion(motion, count, register),
                Operator::Indent => editor.indent_motion(motion, count),
                Operator::Dedent => editor.dedent_motion(motion, count),
            }
        }

        KeyAction::OperatorLine(op, count) => {
            let register = editor.input_state.take_register();
            match op {
                Operator::Delete => editor.delete_line(count, register),
                Operator::Change => editor.change_line(count, register),
                Operator::Yank => editor.yank_line(count, register),
                Operator::Indent => editor.indent_line(count),
                Operator::Dedent => editor.dedent_line(count),
            }
        }

        KeyAction::OperatorTextObject(op, text_object) => {
            let register = editor.input_state.take_register();
            match op {
                Operator::Delete => editor.delete_text_object(text_object, register),
                Operator::Change => editor.change_text_object(text_object, register),
                Operator::Yank => editor.yank_text_object(text_object, register),
                Operator::Indent => editor.indent_text_object(text_object),
                Operator::Dedent => editor.dedent_text_object(text_object),
            }
        }

        KeyAction::SelectTextObject(text_object) => {
            editor.select_text_object(text_object);
        }

        KeyAction::CaseMotion(case_op, motion, count) => {
            editor.case_motion(case_op, motion, count);
        }

        KeyAction::CaseLine(case_op, count) => {
            editor.case_line(case_op, count);
        }

        KeyAction::CaseTextObject(case_op, text_object) => {
            editor.case_text_object(case_op, text_object);
        }

        KeyAction::SetMark(name) => {
            editor.set_mark(name);
        }

        KeyAction::GotoMarkLine(name) => {
            editor.goto_mark_line(name);
        }

        KeyAction::GotoMarkExact(name) => {
            editor.goto_mark_exact(name);
        }

        KeyAction::ReselectVisual => {
            editor.reselect_visual();
        }

        KeyAction::GotoLastInsert => {
            if let Some((line, col)) = editor.last_insert_position {
                editor.cursor.line = line;
                editor.cursor.col = col;
                editor.clamp_cursor();
                editor.scroll_to_cursor();
                editor.enter_insert_mode();
            } else {
                editor.set_status("No previous insert position");
            }
        }

        KeyAction::StartRecordMacro(register) => {
            editor.macros.start_recording(register);
            editor.set_status(&format!("Recording @{}", register));
        }

        KeyAction::StopRecordMacro => {
            // This is normally handled at the top of handle_key, but just in case
            editor.macros.stop_recording();
            editor.set_status("Recording stopped");
        }

        KeyAction::PlayMacro(register, count) => {
            play_macro(editor, register, count);
        }

        KeyAction::ReplayLastMacro(count) => {
            if let Some(register) = editor.macros.last_executed() {
                play_macro(editor, register, count);
            } else {
                editor.set_status("No macro recorded");
            }
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
            let action_elapsed = t_action.elapsed();
            let total = t_start.elapsed();
            if total.as_micros() > 1000 {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open("/tmp/nevi_debug.log") {
                    let _ = writeln!(f, "SLOW EnterInsert: total={:?} leader_check={:?} process={:?} action={:?}", total, leader_check_elapsed, process_elapsed, action_elapsed);
                }
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

        KeyAction::JoinLinesNoSpace => {
            editor.join_lines_no_space();
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
                        DiagnosticSeverity::Error => "Error",
                        DiagnosticSeverity::Warning => "Warning",
                        DiagnosticSeverity::Information => "Info",
                        DiagnosticSeverity::Hint => "Hint",
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
                        DiagnosticSeverity::Error => "Error",
                        DiagnosticSeverity::Warning => "Warning",
                        DiagnosticSeverity::Information => "Info",
                        DiagnosticSeverity::Hint => "Hint",
                    };
                    editor.set_status(format!("{}: {}", prefix, diag.message));
                }
            } else {
                editor.set_status("No diagnostics");
            }
        }

        KeyAction::ShowDiagnosticFloat => {
            // Toggle diagnostic floating popup
            if editor.show_diagnostic_float {
                editor.show_diagnostic_float = false;
            } else {
                let diagnostics = editor.diagnostics_for_line(editor.cursor.line);
                if !diagnostics.is_empty() {
                    editor.show_diagnostic_float = true;
                } else {
                    editor.set_status("No diagnostics on this line");
                }
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

        // Copilot actions - these are handled by the main event loop
        // They set flags that main.rs picks up
        KeyAction::CopilotAccept => {
            // Signal to main loop to accept Copilot completion
            editor.pending_copilot_action = Some(crate::editor::CopilotAction::Auth);
            // Note: Actual accept is handled in main.rs with access to CopilotManager
        }
        KeyAction::CopilotNextCompletion => {
            // Signal to main loop to cycle next
        }
        KeyAction::CopilotPrevCompletion => {
            // Signal to main loop to cycle prev
        }
        KeyAction::CopilotDismiss => {
            // Signal to main loop to dismiss
        }

        KeyAction::Unknown => {
            // Unknown key, ignore
        }
    }
}

fn handle_insert_mode(editor: &mut Editor, key: KeyEvent) {
    let t_insert_start = std::time::Instant::now();

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

    // Handle Copilot keybindings when ghost text is visible
    if editor.copilot_ghost.is_some() {
        match (key.modifiers, key.code) {
            // Accept Copilot completion with Ctrl+L
            (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                // Signal to main loop to accept completion
                editor.pending_copilot_action = Some(crate::editor::CopilotAction::Accept);
                return;
            }
            // Cycle to next completion with Alt+]
            (KeyModifiers::ALT, KeyCode::Char(']')) => {
                editor.pending_copilot_action = Some(crate::editor::CopilotAction::CycleNext);
                return;
            }
            // Cycle to previous completion with Alt+[
            (KeyModifiers::ALT, KeyCode::Char('[')) => {
                editor.pending_copilot_action = Some(crate::editor::CopilotAction::CyclePrev);
                return;
            }
            // Dismiss on Esc (will fall through to enter normal mode below)
            (KeyModifiers::NONE, KeyCode::Esc) => {
                editor.copilot_ghost = None;
                editor.pending_copilot_action = Some(crate::editor::CopilotAction::Dismiss);
                // Continue to enter normal mode below
            }
            // Movement keys dismiss ghost text
            (KeyModifiers::NONE, KeyCode::Left)
            | (KeyModifiers::NONE, KeyCode::Right)
            | (KeyModifiers::NONE, KeyCode::Up)
            | (KeyModifiers::NONE, KeyCode::Down)
            | (KeyModifiers::NONE, KeyCode::Home)
            | (KeyModifiers::NONE, KeyCode::End)
            | (KeyModifiers::NONE, KeyCode::PageUp)
            | (KeyModifiers::NONE, KeyCode::PageDown) => {
                editor.copilot_ghost = None;
                editor.pending_copilot_action = Some(crate::editor::CopilotAction::Dismiss);
                // Continue to handle the movement
            }
            // Word characters and other typing - let ghost text persist
            // Stale detection in main.rs will dismiss if cursor moves before trigger
            _ => {
                // Don't dismiss - ghost text stays visible while typing continues
                // The main loop's stale detection handles invalidation
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

        // Delete word before cursor (Ctrl+w)
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            editor.delete_word_before();
        }

        // Delete to start of line (Ctrl+u)
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            editor.delete_to_line_start();
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

    // Log slow insert mode operations
    let insert_elapsed = t_insert_start.elapsed();
    if insert_elapsed.as_micros() > 500 {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open("/tmp/nevi_debug.log") {
            let _ = writeln!(f, "SLOW insert_mode: {:?} key={:?}", insert_elapsed, key.code);
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

        // Indent/dedent selection
        (KeyModifiers::SHIFT, KeyCode::Char('>')) | (KeyModifiers::NONE, KeyCode::Char('>')) => {
            let (start_line, _, end_line, _) = editor.get_visual_range();
            editor.indent_lines(start_line, end_line);
            editor.enter_normal_mode();
        }
        (KeyModifiers::SHIFT, KeyCode::Char('<')) | (KeyModifiers::NONE, KeyCode::Char('<')) => {
            let (start_line, _, end_line, _) = editor.get_visual_range();
            editor.dedent_lines(start_line, end_line);
            editor.enter_normal_mode();
        }

        // Case transformation on selection
        (KeyModifiers::NONE, KeyCode::Char('u')) => {
            editor.case_visual(crate::input::CaseOperator::Lowercase);
            editor.enter_normal_mode();
        }
        (KeyModifiers::SHIFT, KeyCode::Char('U')) => {
            editor.case_visual(crate::input::CaseOperator::Uppercase);
            editor.enter_normal_mode();
        }
        (KeyModifiers::SHIFT, KeyCode::Char('~')) | (KeyModifiers::NONE, KeyCode::Char('~')) => {
            editor.case_visual(crate::input::CaseOperator::ToggleCase);
            editor.enter_normal_mode();
        }

        _ => {}
    }
}

fn handle_finder_mode(editor: &mut Editor, key: KeyEvent) {
    // Check if we're in normal mode for vim-like navigation
    let is_normal_mode = editor.finder.is_normal_mode();

    // Helper to adjust scroll after navigation
    let adjust_scroll = |editor: &mut Editor| {
        let win = crate::finder::FloatingWindow::centered(editor.term_width, editor.term_height);
        let list_height = (win.height - 4) as usize;
        editor.finder.adjust_scroll(list_height);
    };

    match (key.modifiers, key.code) {
        // Cancel finder - Ctrl+c always closes
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            editor.close_finder();
        }

        // Esc behavior depends on mode
        (KeyModifiers::NONE, KeyCode::Esc) |
        (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
            if is_normal_mode {
                // In normal mode, Esc closes the finder
                editor.close_finder();
            } else {
                // In insert mode, Esc switches to normal mode
                editor.finder.enter_normal_mode();
            }
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

        // Navigate up - works in both modes
        // Note: List is rendered Telescope-style (best match at bottom)
        // Visual UP means going toward higher indices (at visual top)
        (KeyModifiers::NONE, KeyCode::Up) |
        (KeyModifiers::CONTROL, KeyCode::Char('k')) |
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            editor.finder.select_next();  // Visually goes UP
            adjust_scroll(editor);
        }

        // Navigate down - works in both modes
        // Visual DOWN means going toward lower indices (at visual bottom)
        (KeyModifiers::NONE, KeyCode::Down) |
        (KeyModifiers::CONTROL, KeyCode::Char('j')) |
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            editor.finder.select_prev();  // Visually goes DOWN
            adjust_scroll(editor);
        }

        // Normal mode specific: j/k for navigation
        // Note: List is rendered Telescope-style (best match at bottom, index 0)
        // So j (down visually) = decrement index, k (up visually) = increment index
        (KeyModifiers::NONE, KeyCode::Char('j')) if is_normal_mode => {
            editor.finder.select_prev();  // Visually goes DOWN (toward index 0 at bottom)
            adjust_scroll(editor);
        }
        (KeyModifiers::NONE, KeyCode::Char('k')) if is_normal_mode => {
            editor.finder.select_next();  // Visually goes UP (toward higher indices at top)
            adjust_scroll(editor);
        }

        // Normal mode: 'i' to enter insert mode
        (KeyModifiers::NONE, KeyCode::Char('i')) if is_normal_mode => {
            editor.finder.enter_insert_mode();
        }

        // Normal mode: 'gg' to go to top (simplified to just 'g' for now)
        (KeyModifiers::NONE, KeyCode::Char('g')) if is_normal_mode => {
            editor.finder.selected = 0;
            editor.finder.scroll_offset = 0;
        }

        // Normal mode: 'G' to go to bottom
        (KeyModifiers::SHIFT, KeyCode::Char('G')) if is_normal_mode => {
            if !editor.finder.filtered.is_empty() {
                editor.finder.selected = editor.finder.filtered.len() - 1;
                adjust_scroll(editor);
            }
        }

        // Backspace
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            editor.finder.delete_char_before();
        }

        // Regular character - insert mode types, normal mode switches to insert first
        (_, KeyCode::Char(c)) if !c.is_control() => {
            // insert_char already switches to insert mode if needed
            editor.finder.insert_char(c);
        }

        _ => {}
    }
}

fn handle_explorer_mode(editor: &mut Editor, key: KeyEvent) {
    // Handle search input mode
    if editor.explorer.is_searching {
        match (key.modifiers, key.code) {
            // Cancel search
            (KeyModifiers::NONE, KeyCode::Esc) |
            (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
                editor.explorer.cancel_search();
            }
            // Confirm search
            (KeyModifiers::NONE, KeyCode::Enter) => {
                editor.explorer.confirm_search();
            }
            // Next match (Ctrl+n or Tab)
            (KeyModifiers::CONTROL, KeyCode::Char('n')) |
            (KeyModifiers::NONE, KeyCode::Tab) => {
                editor.explorer.next_match();
            }
            // Previous match (Ctrl+p or Shift+Tab)
            (KeyModifiers::CONTROL, KeyCode::Char('p')) |
            (KeyModifiers::SHIFT, KeyCode::BackTab) => {
                editor.explorer.prev_match();
            }
            // Backspace
            (KeyModifiers::NONE, KeyCode::Backspace) => {
                editor.explorer.search_backspace();
            }
            // Move cursor left
            (KeyModifiers::NONE, KeyCode::Left) => {
                editor.explorer.search_cursor_left();
            }
            // Move cursor right
            (KeyModifiers::NONE, KeyCode::Right) => {
                editor.explorer.search_cursor_right();
            }
            // Type character
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) => {
                editor.explorer.search_insert(c);
            }
            _ => {}
        }
        return;
    }

    // Handle input mode for add/rename/delete
    if editor.explorer.has_pending_action() {
        match (key.modifiers, key.code) {
            // Cancel action
            (KeyModifiers::NONE, KeyCode::Esc) |
            (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
                editor.explorer.cancel_action();
            }
            // Confirm action
            (KeyModifiers::NONE, KeyCode::Enter) => {
                execute_explorer_action(editor);
            }
            // Backspace
            (KeyModifiers::NONE, KeyCode::Backspace) => {
                editor.explorer.input_backspace();
            }
            // Delete
            (KeyModifiers::NONE, KeyCode::Delete) => {
                editor.explorer.input_delete();
            }
            // Move cursor
            (KeyModifiers::NONE, KeyCode::Left) => {
                editor.explorer.input_cursor_left();
            }
            (KeyModifiers::NONE, KeyCode::Right) => {
                editor.explorer.input_cursor_right();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('a')) |
            (KeyModifiers::NONE, KeyCode::Home) => {
                editor.explorer.input_cursor_home();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('e')) |
            (KeyModifiers::NONE, KeyCode::End) => {
                editor.explorer.input_cursor_end();
            }
            // Type character
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) => {
                editor.explorer.input_insert(c);
            }
            _ => {}
        }
        return;
    }

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

        // Enter - toggle directory or open file
        (KeyModifiers::NONE, KeyCode::Enter) => {
            if let Some(path) = editor.explorer_selected_path() {
                if path.is_dir() {
                    // Toggle directory expand/collapse
                    editor.explorer.toggle_expand();
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

        // l/Right - expand directory or open file
        (KeyModifiers::NONE, KeyCode::Char('l')) |
        (KeyModifiers::NONE, KeyCode::Right) => {
            if let Some(path) = editor.explorer_selected_path() {
                if path.is_dir() {
                    editor.explorer.expand();
                } else {
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
        (KeyModifiers::SHIFT, KeyCode::Char('R')) => {
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

        // Add file/folder
        (KeyModifiers::NONE, KeyCode::Char('a')) => {
            editor.explorer.start_add();
        }

        // Rename
        (KeyModifiers::NONE, KeyCode::Char('r')) => {
            editor.explorer.start_rename();
        }

        // Delete
        (KeyModifiers::NONE, KeyCode::Char('d')) => {
            editor.explorer.start_delete();
        }

        // Search
        (KeyModifiers::NONE, KeyCode::Char('/')) => {
            editor.explorer.start_search();
        }

        // Next search match
        (KeyModifiers::NONE, KeyCode::Char('n')) => {
            editor.explorer.next_match();
        }

        // Previous search match
        (KeyModifiers::SHIFT, KeyCode::Char('N')) => {
            editor.explorer.prev_match();
        }

        // Copy
        (KeyModifiers::NONE, KeyCode::Char('c')) => {
            editor.explorer.copy_selected();
        }

        // Cut
        (KeyModifiers::NONE, KeyCode::Char('x')) => {
            editor.explorer.cut_selected();
        }

        // Paste
        (KeyModifiers::NONE, KeyCode::Char('p')) => {
            execute_explorer_paste(editor);
        }

        _ => {}
    }
}

fn execute_explorer_action(editor: &mut Editor) {
    use crate::explorer::ExplorerAction;

    let action = editor.explorer.pending_action.clone();
    let input = editor.explorer.input_buffer.clone();

    match action {
        Some(ExplorerAction::Add) => {
            if input.is_empty() {
                editor.explorer.cancel_action();
                return;
            }

            // Get parent directory
            let parent = if let Some(path) = editor.explorer_selected_path() {
                if path.is_dir() {
                    path.clone()
                } else {
                    path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| path.clone())
                }
            } else if let Some(root) = &editor.project_root {
                root.clone()
            } else {
                editor.set_status("No directory selected");
                editor.explorer.cancel_action();
                return;
            };

            let new_path = parent.join(&input);

            // Check if it's a directory (ends with /)
            if input.ends_with('/') {
                match std::fs::create_dir_all(&new_path) {
                    Ok(_) => {
                        editor.set_status(format!("Created: {}", new_path.display()));
                        editor.explorer.refresh();
                    }
                    Err(e) => {
                        editor.set_status(format!("Error: {}", e));
                    }
                }
            } else {
                // Create parent dirs if needed
                if let Some(parent) = new_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::File::create(&new_path) {
                    Ok(_) => {
                        editor.set_status(format!("Created: {}", new_path.display()));
                        editor.explorer.refresh();
                        // Auto-open the newly created file
                        if let Err(e) = editor.open_file(new_path.clone()) {
                            editor.set_status(format!("Created but failed to open: {}", e));
                        } else {
                            editor.mode = Mode::Normal;
                        }
                    }
                    Err(e) => {
                        editor.set_status(format!("Error: {}", e));
                    }
                }
            }
            editor.explorer.cancel_action();
        }
        Some(ExplorerAction::Rename) => {
            if input.is_empty() {
                editor.explorer.cancel_action();
                return;
            }

            if let Some(old_path) = editor.explorer_selected_path() {
                let new_path = if let Some(parent) = old_path.parent() {
                    parent.join(&input)
                } else {
                    std::path::PathBuf::from(&input)
                };

                match std::fs::rename(&old_path, &new_path) {
                    Ok(_) => {
                        editor.set_status(format!("Renamed to: {}", new_path.display()));
                        editor.explorer.refresh();
                    }
                    Err(e) => {
                        editor.set_status(format!("Error: {}", e));
                    }
                }
            }
            editor.explorer.cancel_action();
        }
        Some(ExplorerAction::Delete) => {
            if input.to_lowercase() == "y" || input.to_lowercase() == "yes" {
                if let Some(path) = editor.explorer_selected_path() {
                    let result = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };

                    match result {
                        Ok(_) => {
                            editor.set_status(format!("Deleted: {}", path.display()));
                            editor.explorer.refresh();
                        }
                        Err(e) => {
                            editor.set_status(format!("Error: {}", e));
                        }
                    }
                }
            }
            editor.explorer.cancel_action();
        }
        None => {}
    }
}

fn execute_explorer_paste(editor: &mut Editor) {
    use crate::explorer::ClipboardOp;

    let clipboard = editor.explorer.clipboard.clone();
    if clipboard.is_none() {
        editor.set_status("Nothing to paste");
        return;
    }
    let clipboard = clipboard.unwrap();

    // Get destination directory
    let dest_dir = if let Some(path) = editor.explorer_selected_path() {
        if path.is_dir() {
            path.clone()
        } else {
            path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| path.clone())
        }
    } else if let Some(root) = &editor.project_root {
        root.clone()
    } else {
        editor.set_status("No destination directory");
        return;
    };

    let file_name = clipboard.path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let dest_path = dest_dir.join(&file_name);

    // Check if destination exists
    if dest_path.exists() {
        editor.set_status(format!("Already exists: {}", dest_path.display()));
        return;
    }

    let result = match clipboard.op {
        ClipboardOp::Copy => {
            if clipboard.path.is_dir() {
                copy_dir_recursive(&clipboard.path, &dest_path)
            } else {
                std::fs::copy(&clipboard.path, &dest_path).map(|_| ())
            }
        }
        ClipboardOp::Cut => {
            std::fs::rename(&clipboard.path, &dest_path)
        }
    };

    match result {
        Ok(_) => {
            let action = match clipboard.op {
                ClipboardOp::Copy => "Copied",
                ClipboardOp::Cut => "Moved",
            };
            editor.set_status(format!("{} to: {}", action, dest_path.display()));
            if matches!(clipboard.op, ClipboardOp::Cut) {
                editor.explorer.clear_clipboard();
            }
            editor.explorer.refresh();
        }
        Err(e) => {
            editor.set_status(format!("Error: {}", e));
        }
    }
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
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
                    // Check for external formatter first
                    if let Some(formatter_config) = editor.get_current_formatter().cloned() {
                        // Use external formatter (blocking)
                        let formatter_name = &formatter_config.command;
                        let content = editor.buffer().content();
                        let file_path = editor.buffer().path.as_ref()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();

                        match crate::formatter::format_with_external(&content, &file_path, &formatter_config) {
                            Ok(formatted) => {
                                if formatted != content {
                                    // Replace buffer content with formatted version
                                    editor.replace_buffer_content(&formatted);
                                }
                                // Save the file
                                match editor.save() {
                                    Ok(()) => {
                                        if formatted != content {
                                            CommandResult::Message(format!("Formatted with {} and saved", formatter_name))
                                        } else {
                                            CommandResult::Message(format!("Saved (formatted with {})", formatter_name))
                                        }
                                    }
                                    Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
                                }
                            }
                            Err(e) => {
                                // Formatter failed - show error but still save
                                editor.set_status(format!("{} error: {}", formatter_name, e));
                                match editor.save() {
                                    Ok(()) => CommandResult::Message(format!("Saved ({} failed: {})", formatter_name, e)),
                                    Err(save_err) => CommandResult::Error(format!("Error saving: {}", save_err)),
                                }
                            }
                        }
                    } else {
                        // No external formatter - use LSP formatting
                        // Set flag to save after formatting completes
                        editor.save_after_format = true;
                        // Trigger formatting (which will save when done)
                        editor.pending_lsp_action = Some(LspAction::Formatting);
                        CommandResult::Message("Formatting with LSP...".to_string())
                    }
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

        Command::WriteAll => {
            match editor.save_all() {
                Ok(count) => {
                    if count == 0 {
                        CommandResult::Message("No modified buffers to save".to_string())
                    } else if count == 1 {
                        CommandResult::Message("Saved 1 buffer".to_string())
                    } else {
                        CommandResult::Message(format!("Saved {} buffers", count))
                    }
                }
                Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
            }
        }

        Command::QuitAll => {
            if editor.has_any_unsaved_changes() {
                let names = editor.unsaved_buffer_names();
                CommandResult::Error(format!(
                    "No write since last change in: {} (add ! to override)",
                    names.join(", ")
                ))
            } else {
                CommandResult::Quit
            }
        }

        Command::ForceQuitAll => {
            CommandResult::Quit
        }

        Command::WriteQuitAll => {
            match editor.save_all() {
                Ok(_) => CommandResult::Quit,
                Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
            }
        }

        Command::WriteQuitAllIfModified => {
            if editor.has_any_unsaved_changes() {
                match editor.save_all() {
                    Ok(_) => CommandResult::Quit,
                    Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
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

        Command::FindDiagnostics => {
            editor.open_finder_diagnostics();
            CommandResult::Ok
        }

        Command::DiagnosticFloat => {
            let diagnostics = editor.diagnostics_for_line(editor.cursor.line);
            if !diagnostics.is_empty() {
                editor.show_diagnostic_float = true;
                CommandResult::Ok
            } else {
                CommandResult::Message("No diagnostics on this line".to_string())
            }
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

        Command::ToggleTerminal => {
            editor.floating_terminal.toggle();
            CommandResult::Ok
        }

        // Copilot commands - these are handled by main.rs through editor flags
        Command::CopilotAuth => {
            editor.pending_copilot_action = Some(crate::editor::CopilotAction::Auth);
            CommandResult::Ok
        }
        Command::CopilotSignOut => {
            editor.pending_copilot_action = Some(crate::editor::CopilotAction::SignOut);
            CommandResult::Ok
        }
        Command::CopilotStatus => {
            editor.pending_copilot_action = Some(crate::editor::CopilotAction::Status);
            CommandResult::Ok
        }
        Command::CopilotToggle => {
            editor.pending_copilot_action = Some(crate::editor::CopilotAction::Toggle);
            CommandResult::Ok
        }

        // Theme commands
        Command::Theme(name) => {
            if editor.set_theme(&name) {
                CommandResult::Message(format!("Theme set to '{}'", name))
            } else {
                CommandResult::Error(format!("Theme '{}' not found", name))
            }
        }
        Command::Themes => {
            editor.open_theme_picker();
            CommandResult::Ok
        }

        Command::Marks => {
            // Get buffer key for local marks
            let buffer_key = editor
                .buffer()
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("buffer_{}", editor.current_buffer_index()));

            let local_marks = editor.marks.get_local_marks(&buffer_key);
            let global_marks = editor.marks.get_global_marks();

            if local_marks.is_empty() && global_marks.is_empty() {
                CommandResult::Message("No marks set".to_string())
            } else {
                // Build mark entries
                let mut items = Vec::new();

                // Add local marks
                let current_file = editor.buffer().path.clone();
                let current_file_name = current_file
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                for (name, mark) in local_marks {
                    items.push(MarkEntry {
                        name,
                        line: mark.line,
                        col: mark.col,
                        file_path: current_file.clone(),
                        file_name: current_file_name.clone(),
                        is_global: false,
                    });
                }

                // Add global marks
                for (name, mark) in global_marks {
                    let file_name = mark
                        .path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    items.push(MarkEntry {
                        name,
                        line: mark.line,
                        col: mark.col,
                        file_path: mark.path.clone(),
                        file_name,
                        is_global: true,
                    });
                }

                editor.marks_picker = Some(MarksPicker::new(items));
                CommandResult::Ok
            }
        }

        Command::DeleteMarks(arg) => {
            use crate::editor::Marks;

            let buffer_key = editor
                .buffer()
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("buffer_{}", editor.current_buffer_index()));

            let marks_to_delete = Marks::parse_delmarks_arg(&arg);
            let mut deleted_count = 0;

            for mark in marks_to_delete {
                if editor.marks.delete(&buffer_key, mark) {
                    deleted_count += 1;
                }
            }

            if deleted_count > 0 {
                CommandResult::Message(format!(
                    "Deleted {} mark{}",
                    deleted_count,
                    if deleted_count == 1 { "" } else { "s" }
                ))
            } else {
                CommandResult::Message("No marks deleted".to_string())
            }
        }

        Command::DeleteMarksAll => {
            let buffer_key = editor
                .buffer()
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("buffer_{}", editor.current_buffer_index()));

            let deleted_count = editor.marks.delete_all_local(&buffer_key);

            if deleted_count > 0 {
                CommandResult::Message(format!(
                    "Deleted {} mark{}",
                    deleted_count,
                    if deleted_count == 1 { "" } else { "s" }
                ))
            } else {
                CommandResult::Message("No local marks to delete".to_string())
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
pub fn execute_leader_action(editor: &mut Editor, action: &LeaderAction) {
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
