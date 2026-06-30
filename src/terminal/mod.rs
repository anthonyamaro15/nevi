use crossterm::{
    cursor,
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers,
        KeyboardEnhancementFlags, MouseEvent, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute, queue,
    style::{Attribute, Color, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal::{self, ClearType},
};
use std::io::{self, Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use unicode_width::UnicodeWidthChar;

/// Enable finder profiling (writes to /tmp/nevi_finder_profile.log)
pub static FINDER_PROFILE_ENABLED: AtomicBool = AtomicBool::new(false);

fn log_finder_profile(msg: &str) {
    if FINDER_PROFILE_ENABLED.load(Ordering::Relaxed) {
        use std::fs::OpenOptions;
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/nevi_finder_profile.log")
        {
            let _ = writeln!(f, "{}", msg);
        }
    }
}

fn finder_preview_match_ranges(line: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }

    let query_lower = query.to_lowercase();
    let query_len = query.chars().count();
    let mut ranges = Vec::new();
    let mut next_start_col = 0;

    for (char_idx, (byte_idx, _)) in line.char_indices().enumerate() {
        if char_idx < next_start_col {
            continue;
        }

        if line[byte_idx..].to_lowercase().starts_with(&query_lower) {
            let end_col = char_idx + query_len;
            ranges.push((char_idx, end_col));
            next_start_col = end_col.max(char_idx + 1);
        }
    }

    ranges
}

use crate::commands::{parse_command, Command, CommandPopupMode, CommandResult, PendingDigraph};
use crate::config::{CommandModeAction, LeaderAction};
use crate::editor::{
    Editor, ExpressionRegisterTarget, LspAction, Mode, Pane, PaneDirection, SplitLayout,
};
use crate::input::{
    InsertPosition, KeyAction, Operator, TextObject, TextObjectModifier, TextObjectType,
};
use crate::lsp::types::{CompletionKind, Diagnostic, DiagnosticSeverity};
use crate::syntax::{HighlightSpan, SyntaxStyle};

/// Events from the terminal that the editor cares about
pub enum EditorEvent {
    /// A key press
    Key(KeyEvent),
    /// A bracketed paste payload
    Paste(String),
    /// A mouse event
    Mouse(MouseEvent),
    /// Terminal gained focus (for autoread)
    FocusGained,
    /// Terminal was resized
    Resize(u16, u16),
}

/// Section types for hover content parsing
enum HoverSection {
    #[allow(dead_code)]
    Code {
        language: String,
        lines: Vec<String>,
    },
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
    /// Number of display characters inserted before source text on this segment.
    virtual_prefix_chars: usize,
    /// The text content of this segment
    text: String,
    /// Whether this is the first segment (shows line number)
    is_first: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TerminalRenderStyle {
    fg: Color,
    bg: Color,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    double_underline: bool,
    undercurl: bool,
    underdotted: bool,
    underdashed: bool,
    strikeout: bool,
}

impl TerminalRenderStyle {
    fn from_terminal_cell(
        cell: &crate::floating_terminal::TerminalCell,
        default_fg: Color,
        default_bg: Color,
    ) -> Self {
        let mut fg = cell.fg.unwrap_or(default_fg);
        let mut bg = cell.bg.unwrap_or(default_bg);
        if cell.inverse {
            std::mem::swap(&mut fg, &mut bg);
        }

        Self {
            fg,
            bg,
            bold: cell.bold,
            dim: cell.dim,
            italic: cell.italic,
            underline: cell.underline,
            double_underline: cell.double_underline,
            undercurl: cell.undercurl,
            underdotted: cell.underdotted,
            underdashed: cell.underdashed,
            strikeout: cell.strikeout,
        }
    }
}

fn terminal_cursor_style(
    cursor_info: crate::floating_terminal::TerminalCursorInfo,
) -> cursor::SetCursorStyle {
    use crate::floating_terminal::TerminalCursorShape;

    match (cursor_info.shape, cursor_info.blinking) {
        (TerminalCursorShape::Block | TerminalCursorShape::HollowBlock, true) => {
            cursor::SetCursorStyle::BlinkingBlock
        }
        (TerminalCursorShape::Block | TerminalCursorShape::HollowBlock, false) => {
            cursor::SetCursorStyle::SteadyBlock
        }
        (TerminalCursorShape::Underline, true) => cursor::SetCursorStyle::BlinkingUnderScore,
        (TerminalCursorShape::Underline, false) => cursor::SetCursorStyle::SteadyUnderScore,
        (TerminalCursorShape::Beam, true) => cursor::SetCursorStyle::BlinkingBar,
        (TerminalCursorShape::Beam, false) => cursor::SetCursorStyle::SteadyBar,
        (TerminalCursorShape::Hidden, _) => cursor::SetCursorStyle::DefaultUserShape,
    }
}

fn diagnostic_severity_priority(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Information => 2,
        DiagnosticSeverity::Hint => 3,
    }
}

fn diagnostic_at_col<'a>(
    diagnostics: &[&'a Diagnostic],
    line_num: usize,
    col: usize,
) -> Option<&'a Diagnostic> {
    diagnostics
        .iter()
        .copied()
        .filter(|d| {
            if line_num < d.line || line_num > d.end_line {
                return false;
            }
            if d.line == d.end_line {
                col >= d.col_start && col < d.col_end
            } else if line_num == d.line {
                col >= d.col_start
            } else if line_num == d.end_line {
                col < d.col_end
            } else {
                true
            }
        })
        .min_by_key(|d| diagnostic_severity_priority(d.severity))
}

fn diagnostic_underline_color(
    diagnostic: &Diagnostic,
    diagnostic_error_color: Color,
) -> Option<Color> {
    match diagnostic.severity {
        DiagnosticSeverity::Error => Some(diagnostic_error_color),
        DiagnosticSeverity::Warning
        | DiagnosticSeverity::Information
        | DiagnosticSeverity::Hint => None,
    }
}

fn ansi_underline_color_code(color: Color) -> String {
    match color {
        Color::Reset => "59".to_string(),
        Color::Black => "58:5:0".to_string(),
        Color::DarkRed => "58:5:1".to_string(),
        Color::DarkGreen => "58:5:2".to_string(),
        Color::DarkYellow => "58:5:3".to_string(),
        Color::DarkBlue => "58:5:4".to_string(),
        Color::DarkMagenta => "58:5:5".to_string(),
        Color::DarkCyan => "58:5:6".to_string(),
        Color::Grey => "58:5:7".to_string(),
        Color::DarkGrey => "58:5:8".to_string(),
        Color::Red => "58:5:9".to_string(),
        Color::Green => "58:5:10".to_string(),
        Color::Yellow => "58:5:11".to_string(),
        Color::Blue => "58:5:12".to_string(),
        Color::Magenta => "58:5:13".to_string(),
        Color::Cyan => "58:5:14".to_string(),
        Color::White => "58:5:15".to_string(),
        Color::Rgb { r, g, b } => format!("58:2:{r}:{g}:{b}"),
        Color::AnsiValue(value) => format!("58:5:{value}"),
    }
}

fn write_ansi_underline_color<W: Write>(writer: &mut W, color: Color) -> io::Result<()> {
    let code = ansi_underline_color_code(color);
    write!(writer, "\x1b[{code}m")
}

fn apply_diagnostic_underline<W: Write>(
    writer: &mut W,
    underline_color: Option<Color>,
    token_fg: Color,
) -> io::Result<()> {
    if let Some(color) = underline_color {
        execute!(
            writer,
            SetAttribute(Attribute::Undercurled),
            SetForegroundColor(token_fg)
        )?;
        write_ansi_underline_color(writer, color)
    } else {
        execute!(writer, SetAttribute(Attribute::NoUnderline))?;
        write_ansi_underline_color(writer, Color::Reset)?;
        execute!(writer, SetForegroundColor(token_fg))
    }
}

fn editor_char_display_width(ch: char, tab_width: usize) -> usize {
    if ch == '\t' {
        tab_width.max(1)
    } else if ch.is_control() {
        1
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0)
    }
}

fn text_display_width(text: &str, tab_width: usize) -> usize {
    text.chars()
        .filter(|ch| *ch != '\n')
        .map(|ch| editor_char_display_width(ch, tab_width))
        .sum()
}

fn display_width_between_char_cols(
    text: &str,
    start_col: usize,
    end_col: usize,
    tab_width: usize,
) -> usize {
    if end_col <= start_col {
        return 0;
    }

    text.chars()
        .enumerate()
        .skip(start_col)
        .take(end_col - start_col)
        .take_while(|(_, ch)| *ch != '\n')
        .map(|(_, ch)| editor_char_display_width(ch, tab_width))
        .sum()
}

fn take_display_width(text: &str, start_col: usize, max_width: usize, tab_width: usize) -> String {
    let mut width = 0;
    let mut out = String::new();

    for ch in text.chars().skip(start_col) {
        if ch == '\n' {
            break;
        }

        let ch_width = editor_char_display_width(ch, tab_width);
        if width + ch_width > max_width {
            break;
        }

        out.push(ch);
        width += ch_width;
    }

    out
}

fn explorer_action_input_view(
    input: &str,
    cursor_byte_idx: usize,
    field_width: usize,
) -> (String, usize) {
    if field_width == 0 {
        return (String::new(), 0);
    }

    let mut cursor_byte_idx = cursor_byte_idx.min(input.len());
    while cursor_byte_idx > 0 && !input.is_char_boundary(cursor_byte_idx) {
        cursor_byte_idx -= 1;
    }

    let cursor_col = input[..cursor_byte_idx].chars().count();
    let input_width = input.chars().count();

    if input_width <= field_width {
        return (
            input.to_string(),
            cursor_col.min(field_width.saturating_sub(1)),
        );
    }

    let max_cursor_col = field_width.saturating_sub(1);
    let start_col = cursor_col.saturating_sub(max_cursor_col);
    let mut visible = String::with_capacity(field_width);

    if start_col > 0 && field_width > 1 {
        visible.push('…');
        visible.extend(input.chars().skip(start_col + 1).take(field_width - 1));
    } else {
        visible.extend(input.chars().skip(start_col).take(field_width));
    }

    let cursor_x = cursor_col
        .saturating_sub(start_col)
        .min(field_width.saturating_sub(1));
    (visible, cursor_x)
}

fn print_editor_char(ch: char, tab_width: usize) -> usize {
    let width = editor_char_display_width(ch, tab_width);
    if ch == '\t' {
        for _ in 0..width {
            print!(" ");
        }
        width
    } else if ch.is_control() {
        print!(" ");
        1
    } else {
        print!("{}", ch);
        width
    }
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
fn calculate_wrap_segments(
    line: &str,
    max_width: usize,
    preserve_indent: bool,
    tab_width: usize,
) -> Vec<WrapSegment> {
    if max_width == 0 {
        return vec![WrapSegment {
            start_col: 0,
            virtual_prefix_chars: 0,
            text: line.to_string(),
            is_first: true,
        }];
    }

    let line = line.trim_end_matches('\n');
    let chars: Vec<char> = line.chars().collect();

    if text_display_width(line, tab_width) <= max_width {
        return vec![WrapSegment {
            start_col: 0,
            virtual_prefix_chars: 0,
            text: line.to_string(),
            is_first: true,
        }];
    }

    // Calculate the indentation of the original line
    let indent: String = if preserve_indent {
        let mut width = 0;
        let mut indent = String::new();
        for ch in chars.iter().take_while(|c| c.is_whitespace()) {
            let ch_width = editor_char_display_width(*ch, tab_width);
            if width + ch_width >= max_width {
                break;
            }
            indent.push(*ch);
            width += ch_width;
        }
        indent
    } else {
        String::new()
    };
    let indent_width = text_display_width(&indent, tab_width);

    let mut segments = Vec::new();
    let mut current_col = 0;
    let mut is_first = true;

    while current_col < chars.len() {
        let segment_indent = if is_first { "" } else { &indent };
        let available_width = if is_first {
            max_width
        } else {
            max_width.saturating_sub(indent_width)
        };

        if available_width == 0 {
            // Can't fit anything, just take one char to avoid infinite loop
            let text: String = std::iter::once(chars[current_col]).collect();
            segments.push(WrapSegment {
                start_col: current_col,
                virtual_prefix_chars: segment_indent.chars().count(),
                text: format!("{}{}", segment_indent, text),
                is_first,
            });
            current_col += 1;
        } else {
            let mut take_count = 0;
            let mut segment_width = 0;

            while current_col + take_count < chars.len() {
                let ch = chars[current_col + take_count];
                let ch_width = editor_char_display_width(ch, tab_width);
                if segment_width + ch_width > available_width {
                    if take_count == 0 {
                        take_count = 1;
                    }
                    break;
                }

                segment_width += ch_width;
                take_count += 1;
            }

            let text: String = chars[current_col..current_col + take_count]
                .iter()
                .collect();

            segments.push(WrapSegment {
                start_col: current_col,
                virtual_prefix_chars: segment_indent.chars().count(),
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
            virtual_prefix_chars: 0,
            text: String::new(),
            is_first: true,
        });
    }

    segments
}

fn source_col_for_wrap_segment_position(
    start_col: usize,
    virtual_prefix_chars: usize,
    visual_idx: usize,
) -> Option<usize> {
    if visual_idx < virtual_prefix_chars {
        None
    } else {
        Some(start_col + visual_idx - virtual_prefix_chars)
    }
}

#[cfg(test)]
fn source_col_for_wrap_segment_char(segment: &WrapSegment, visual_idx: usize) -> Option<usize> {
    source_col_for_wrap_segment_position(
        segment.start_col,
        segment.virtual_prefix_chars,
        visual_idx,
    )
}

/// Terminal handler responsible for rendering and input
pub struct Terminal {
    stdout: Stdout,
    mouse_capture_enabled: bool,
    leader_popup_rect: Option<(u16, u16)>,
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
            event::EnableFocusChange,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;

        Ok(Self {
            stdout,
            mouse_capture_enabled: false,
            leader_popup_rect: None,
        })
    }

    fn set_mouse_capture(&mut self, enabled: bool) -> anyhow::Result<()> {
        if self.mouse_capture_enabled == enabled {
            return Ok(());
        }

        if enabled {
            execute!(self.stdout, event::EnableMouseCapture, EnableBracketedPaste)?;
        } else {
            execute!(
                self.stdout,
                event::DisableMouseCapture,
                DisableBracketedPaste
            )?;
        }
        self.mouse_capture_enabled = enabled;
        Ok(())
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
        let restore_mouse_capture = self.mouse_capture_enabled;
        self.set_mouse_capture(false)?;

        // Leave alternate screen and show cursor
        execute!(
            self.stdout,
            PopKeyboardEnhancementFlags,
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
            event::EnableFocusChange,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        if restore_mouse_capture {
            self.set_mouse_capture(true)?;
        }

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
                Err(anyhow::anyhow!(
                    "Failed to run command '{}': {}",
                    command,
                    e
                ))
            }
        }
    }

    /// Render the editor state to the terminal
    pub fn render(&mut self, editor: &Editor) -> anyhow::Result<()> {
        self.set_mouse_capture(Self::should_capture_mouse(editor))?;
        execute!(self.stdout, cursor::Hide, cursor::MoveTo(0, 0))?;

        // Large overlays fully cover the content area, so repainting the editor
        // behind them only wastes work and can flash between redraw phases.
        let skip_background = Self::should_skip_background(editor);

        if !skip_background {
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
        }

        if self.leader_popup_rect.is_some() && editor.leader_popup_items().is_empty() {
            if skip_background {
                self.clear_leader_popup_area(editor)?;
            } else {
                self.leader_popup_rect = None;
            }
        }

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

        // Render references picker if active
        if editor.references_picker.is_some() {
            self.render_references_picker(editor)?;
        }

        // Render code actions picker if active
        if editor.code_actions_picker.is_some() {
            self.render_code_actions_picker(editor)?;
        }

        // Render Markdown preview if active
        if editor.markdown_preview.is_some() {
            self.render_markdown_preview(editor)?;
        }

        // Render theme picker if active
        if editor.theme_picker.is_some() {
            self.render_theme_picker(editor)?;
        }

        if !editor.leader_popup_items().is_empty() {
            self.render_leader_popup(editor)?;
        }

        // Position cursor
        if editor.floating_terminal.is_visible() {
            self.render_floating_terminal(editor)?;
        } else if editor.markdown_preview.is_some() {
            execute!(self.stdout, cursor::Hide)?;
        } else {
            self.position_cursor(editor)?;
        }

        self.stdout.flush()?;
        Ok(())
    }

    /// Render available leader-key continuations above the status line.
    fn render_leader_popup(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let Some(prefix) = editor.leader_sequence.as_deref() else {
            self.clear_leader_popup_area(editor)?;
            return Ok(());
        };

        let items = editor.leader_popup_items();
        if items.is_empty() {
            self.clear_leader_popup_area(editor)?;
            return Ok(());
        }

        let status_row = editor.term_height.saturating_sub(2);
        if status_row == 0 {
            return Ok(());
        }

        let term_width = editor.term_width as usize;
        if term_width < 12 {
            return Ok(());
        }

        let available_rows = status_row as usize;
        if available_rows < 2 {
            return Ok(());
        }

        let item_rows = items.len().min(8).min(available_rows.saturating_sub(1));
        if item_rows == 0 {
            self.clear_leader_popup_area(editor)?;
            return Ok(());
        }

        let popup_top_row = status_row.saturating_sub((item_rows + 1) as u16);
        self.leader_popup_rect = Some((popup_top_row, (item_rows + 1) as u16));

        let prefix_label = if prefix.is_empty() {
            "<leader>".to_string()
        } else {
            format!("<leader>{prefix}")
        };
        let header_text = format!("[LEADER {prefix_label}] Esc:cancel");

        let key_col_width = term_width.saturating_sub(4).min(16).max(8);
        let desc_col_width = term_width.saturating_sub(key_col_width + 3);

        let theme = editor.theme();
        let popup_fg = theme.ui.foreground;
        let popup_bg = theme.ui.popup_bg;
        let header_fg = theme.ui.statusline_mode_normal;
        let header_bg = theme.ui.statusline_bg;

        execute!(
            self.stdout,
            cursor::MoveTo(0, popup_top_row),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(header_fg),
            SetBackgroundColor(header_bg)
        )?;
        let mut header_line = Self::truncate_inline(&header_text, term_width);
        let header_len = header_line.chars().count();
        if header_len < term_width {
            header_line.push_str(&" ".repeat(term_width - header_len));
        }
        print!("{}", header_line);
        execute!(self.stdout, ResetColor)?;

        for (row_offset, item) in items.iter().take(item_rows).enumerate() {
            let row = popup_top_row + row_offset as u16 + 1;
            execute!(
                self.stdout,
                cursor::MoveTo(0, row),
                terminal::Clear(ClearType::CurrentLine),
                SetForegroundColor(popup_fg),
                SetBackgroundColor(popup_bg)
            )?;

            let key_raw = if item.has_children {
                format!("{} +", item.key)
            } else {
                item.key.clone()
            };
            let mut desc_raw = if item.description == "prefix" {
                "more...".to_string()
            } else {
                item.description.clone()
            };
            if item.is_exact && item.has_children && !desc_raw.is_empty() {
                desc_raw.push_str(" (more)");
            }

            let key_text = Self::truncate_inline(&key_raw, key_col_width);
            let desc_text = Self::truncate_inline(&desc_raw, desc_col_width);
            let mut line = format!(
                "  {:key_width$} {}",
                key_text,
                desc_text,
                key_width = key_col_width
            );

            let printed = line.chars().count();
            if printed < term_width {
                line.push_str(&" ".repeat(term_width - printed));
            } else if printed > term_width {
                line = line.chars().take(term_width).collect();
            }
            print!("{}", line);
            execute!(self.stdout, ResetColor)?;
        }

        Ok(())
    }

    fn clear_leader_popup_area(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let Some((top_row, rows)) = self.leader_popup_rect.take() else {
            return Ok(());
        };

        let max_row = editor.term_height.saturating_sub(1);
        let end_row = top_row.saturating_add(rows).min(max_row);
        for row in top_row..end_row {
            execute!(
                self.stdout,
                cursor::MoveTo(0, row),
                terminal::Clear(ClearType::CurrentLine)
            )?;
        }
        execute!(self.stdout, ResetColor)?;
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
                editor,
                pane,
                buffer,
                rect,
                is_active,
                line_num_width,
                visual_range,
                show_line_numbers,
                show_relative,
                highlight_cursor_line,
                pane_height,
                pane_width,
                effective_wrap_width,
                buffer_path.as_ref(),
            )?;
        } else {
            // Original non-wrapped rendering
            self.render_pane_nowrap(
                editor,
                pane,
                buffer,
                rect,
                is_active,
                line_num_width,
                visual_range,
                show_line_numbers,
                show_relative,
                highlight_cursor_line,
                pane_height,
                pane_width,
                text_area_width,
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
        let tab_width = editor.get_effective_tab_width();

        // Pre-compute URI for diagnostic lookups (avoids repeated string allocations)
        let cached_uri = if is_active {
            editor.current_buffer_uri()
        } else {
            None
        };

        let mut current_row = 0;
        let mut file_line = pane.viewport_offset;

        while current_row < pane_height && file_line < buffer.len_lines() {
            let is_cursor_line = is_active && file_line == pane.cursor.line;

            // Get line content
            let line_content = buffer
                .line(file_line)
                .map(|l| l.to_string())
                .unwrap_or_default();

            // Calculate wrapped segments
            let segments = calculate_wrap_segments(&line_content, wrap_width, true, tab_width);

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
            // Compute severity flags in single pass for performance
            let (has_error, has_warning, has_info, has_hint) = {
                let mut e = false;
                let mut w = false;
                let mut i = false;
                let mut h = false;
                for d in &line_diagnostics {
                    match d.severity {
                        DiagnosticSeverity::Error => e = true,
                        DiagnosticSeverity::Warning => w = true,
                        DiagnosticSeverity::Information => i = true,
                        DiagnosticSeverity::Hint => h = true,
                    }
                }
                (e, w, i, h)
            };

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
                execute!(
                    self.stdout,
                    SetAttribute(Attribute::Reset),
                    SetBackgroundColor(row_bg),
                    SetForegroundColor(editor_fg)
                )?;

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
                            execute!(
                                self.stdout,
                                SetForegroundColor(editor_fg),
                                SetBackgroundColor(row_bg)
                            )?;
                        }
                        Some(crate::git::GitLineStatus::Modified) => {
                            execute!(self.stdout, SetForegroundColor(theme.git.modified))?;
                            print!("▎");
                            execute!(
                                self.stdout,
                                SetForegroundColor(editor_fg),
                                SetBackgroundColor(row_bg)
                            )?;
                        }
                        Some(crate::git::GitLineStatus::Deleted) => {
                            execute!(self.stdout, SetForegroundColor(theme.git.deleted))?;
                            print!("▁");
                            execute!(
                                self.stdout,
                                SetForegroundColor(editor_fg),
                                SetBackgroundColor(row_bg)
                            )?;
                        }
                        None => {
                            print!(" ");
                        }
                    }

                    // Diagnostic sign (second char) - priority: error > warning > info > hint
                    if has_error {
                        execute!(self.stdout, SetForegroundColor(theme.diagnostic.error))?;
                        print!("●");
                        execute!(
                            self.stdout,
                            SetForegroundColor(editor_fg),
                            SetBackgroundColor(row_bg)
                        )?;
                    } else if has_warning {
                        execute!(self.stdout, SetForegroundColor(theme.diagnostic.warning))?;
                        print!("▲");
                        execute!(
                            self.stdout,
                            SetForegroundColor(editor_fg),
                            SetBackgroundColor(row_bg)
                        )?;
                    } else if has_info {
                        execute!(self.stdout, SetForegroundColor(theme.diagnostic.info))?;
                        print!("■");
                        execute!(
                            self.stdout,
                            SetForegroundColor(editor_fg),
                            SetBackgroundColor(row_bg)
                        )?;
                    } else if has_hint {
                        execute!(self.stdout, SetForegroundColor(theme.diagnostic.hint))?;
                        print!("○");
                        execute!(
                            self.stdout,
                            SetForegroundColor(editor_fg),
                            SetBackgroundColor(row_bg)
                        )?;
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
                            let distance =
                                (file_line as isize - pane.cursor.line as isize).abs() as usize;
                            if distance == 0 {
                                format!("{:>width$} ", file_line + 1, width = line_num_width)
                            } else {
                                format!("{:>width$} ", distance, width = line_num_width)
                            }
                        } else {
                            format!("{:>width$} ", file_line + 1, width = line_num_width)
                        };

                        // Use theme colors for line numbers - priority: error > warning > info > hint
                        let line_num_color = if has_error {
                            theme.diagnostic.error
                        } else if has_warning {
                            theme.diagnostic.warning
                        } else if has_info {
                            theme.diagnostic.info
                        } else if has_hint {
                            theme.diagnostic.hint
                        } else if is_cursor_line {
                            theme.ui.line_number_active
                        } else {
                            theme.ui.line_number
                        };

                        execute!(self.stdout, SetForegroundColor(line_num_color))?;
                        print!("{}", line_num);
                        execute!(
                            self.stdout,
                            SetForegroundColor(editor_fg),
                            SetBackgroundColor(row_bg)
                        )?;
                    } else {
                        // Continuation line - empty line number gutter
                        print!("{:>width$} ", "", width = line_num_width);
                    }
                }

                // Render segment content with syntax highlighting
                let segment_text = segment.text.trim_end_matches('\n');

                let rendered_cols = self.render_line_segment_with_highlights(
                    segment_text,
                    file_line,
                    segment.start_col,
                    segment.virtual_prefix_chars,
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
                    theme.ui.line_number, // Use grey color for unused code (hint diagnostics)
                    tab_width,
                )?;

                // Fill remaining space (sign column = 2)
                let mut chars_printed =
                    2 + if show_line_numbers {
                        line_num_width + 1
                    } else {
                        0
                    } + rendered_cols;

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

                            let msg: String = diag
                                .message
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
                            execute!(
                                self.stdout,
                                SetForegroundColor(editor_fg),
                                SetBackgroundColor(row_bg)
                            )?;

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
            execute!(
                self.stdout,
                SetAttribute(Attribute::Reset),
                SetBackgroundColor(editor_bg),
                SetForegroundColor(editor_fg)
            )?;

            print!("  "); // Empty sign column

            execute!(self.stdout, SetForegroundColor(Color::Blue))?;
            if show_line_numbers {
                print!("{:>width$} ~", "", width = line_num_width);
            } else {
                print!("~");
            }
            execute!(self.stdout, SetForegroundColor(editor_fg))?;

            // Fill remaining space (sign column = 2)
            let chars_printed = 2 + if show_line_numbers {
                line_num_width + 2
            } else {
                1
            };
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
        let tab_width = editor.get_effective_tab_width();

        // Pre-compute URI for diagnostic lookups (avoids repeated string allocations)
        let cached_uri = if is_active {
            editor.current_buffer_uri()
        } else {
            None
        };

        // Render each row in this pane
        for row in 0..pane_height {
            let screen_y = rect.y + row as u16;
            let file_line = pane.viewport_offset + row;
            let is_cursor_line = is_active && file_line == pane.cursor.line;

            // Move to start of this row in the pane
            execute!(self.stdout, cursor::MoveTo(rect.x, screen_y))?;

            // Set background color for this row (cursor line or normal)
            let row_bg =
                if highlight_cursor_line && is_cursor_line && file_line < buffer.len_lines() {
                    cursor_line_bg
                } else {
                    editor_bg
                };
            execute!(
                self.stdout,
                SetAttribute(Attribute::Reset),
                SetBackgroundColor(row_bg),
                SetForegroundColor(editor_fg)
            )?;

            if file_line < buffer.len_lines() {
                // Check for diagnostics on this line (only for active pane)
                let line_diagnostics = match &cached_uri {
                    Some(uri) => editor.diagnostics_for_line_cached(file_line, uri),
                    None => Vec::new(),
                };
                // Compute severity flags in single pass for performance
                let (has_error, has_warning, has_info, has_hint) = {
                    let mut e = false;
                    let mut w = false;
                    let mut i = false;
                    let mut h = false;
                    for d in &line_diagnostics {
                        match d.severity {
                            DiagnosticSeverity::Error => e = true,
                            DiagnosticSeverity::Warning => w = true,
                            DiagnosticSeverity::Information => i = true,
                            DiagnosticSeverity::Hint => h = true,
                        }
                    }
                    (e, w, i, h)
                };

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
                        execute!(
                            self.stdout,
                            SetForegroundColor(editor_fg),
                            SetBackgroundColor(row_bg)
                        )?;
                    }
                    Some(crate::git::GitLineStatus::Modified) => {
                        execute!(self.stdout, SetForegroundColor(theme.git.modified))?;
                        print!("▎");
                        execute!(
                            self.stdout,
                            SetForegroundColor(editor_fg),
                            SetBackgroundColor(row_bg)
                        )?;
                    }
                    Some(crate::git::GitLineStatus::Deleted) => {
                        execute!(self.stdout, SetForegroundColor(theme.git.deleted))?;
                        print!("▁");
                        execute!(
                            self.stdout,
                            SetForegroundColor(editor_fg),
                            SetBackgroundColor(row_bg)
                        )?;
                    }
                    None => {
                        print!(" ");
                    }
                }

                // Diagnostic sign (second char) - priority: error > warning > info > hint
                if has_error {
                    execute!(self.stdout, SetForegroundColor(theme.diagnostic.error))?;
                    print!("●");
                    execute!(
                        self.stdout,
                        SetForegroundColor(editor_fg),
                        SetBackgroundColor(row_bg)
                    )?;
                } else if has_warning {
                    execute!(self.stdout, SetForegroundColor(theme.diagnostic.warning))?;
                    print!("▲");
                    execute!(
                        self.stdout,
                        SetForegroundColor(editor_fg),
                        SetBackgroundColor(row_bg)
                    )?;
                } else if has_info {
                    execute!(self.stdout, SetForegroundColor(theme.diagnostic.info))?;
                    print!("■");
                    execute!(
                        self.stdout,
                        SetForegroundColor(editor_fg),
                        SetBackgroundColor(row_bg)
                    )?;
                } else if has_hint {
                    execute!(self.stdout, SetForegroundColor(theme.diagnostic.hint))?;
                    print!("○");
                    execute!(
                        self.stdout,
                        SetForegroundColor(editor_fg),
                        SetBackgroundColor(row_bg)
                    )?;
                } else {
                    print!(" ");
                }

                // Line number (if enabled)
                if show_line_numbers {
                    let line_num = if show_relative && is_active {
                        // Relative line numbers: show distance from cursor, current line shows absolute
                        let distance =
                            (file_line as isize - pane.cursor.line as isize).abs() as usize;
                        if distance == 0 {
                            format!("{:>width$} ", file_line + 1, width = line_num_width)
                        } else {
                            format!("{:>width$} ", distance, width = line_num_width)
                        }
                    } else {
                        format!("{:>width$} ", file_line + 1, width = line_num_width)
                    };

                    // Use theme colors for line numbers - priority: error > warning > info > hint
                    let line_num_color = if has_error {
                        theme.diagnostic.error
                    } else if has_warning {
                        theme.diagnostic.warning
                    } else if has_info {
                        theme.diagnostic.info
                    } else if has_hint {
                        theme.diagnostic.hint
                    } else if is_cursor_line {
                        theme.ui.line_number_active
                    } else {
                        theme.ui.line_number
                    };

                    execute!(self.stdout, SetForegroundColor(line_num_color))?;
                    print!("{}", line_num);
                    execute!(
                        self.stdout,
                        SetForegroundColor(editor_fg),
                        SetBackgroundColor(row_bg)
                    )?;
                }

                // Line content with syntax highlighting and visual selection
                if let Some(line) = buffer.line(file_line) {
                    let h_offset = pane.h_offset;
                    let line_content = line.to_string();
                    let full_line_len = line_content.chars().filter(|c| *c != '\n').count();
                    let line_str =
                        take_display_width(&line_content, h_offset, effective_width, tab_width);

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

                    let rendered_cols = self.render_line_with_highlights(
                        &line_str,
                        file_line,
                        h_offset,
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
                        theme.ui.line_number, // Use grey color for unused code (hint diagnostics)
                        tab_width,
                    )?;

                    // Track characters printed for fill calculation (sign column = 2)
                    let mut chars_printed =
                        2 + if show_line_numbers {
                            line_num_width + 1
                        } else {
                            0
                        } + rendered_cols;

                    // Render ghost text on cursor line when completion is active
                    if is_cursor_line
                        && is_active
                        && editor.mode == Mode::Insert
                        && editor.completion.active
                    {
                        // Only show ghost text if cursor is at or near end of line
                        let cursor_at_end = pane.cursor.col >= full_line_len.saturating_sub(1);
                        if cursor_at_end {
                            if let Some(ghost) = editor.completion.ghost_text() {
                                // Limit ghost text to remaining space
                                let remaining = pane_width.saturating_sub(chars_printed);
                                let ghost_chars: String = ghost.chars().take(remaining).collect();

                                // Render ghost text in dim gray
                                execute!(
                                    self.stdout,
                                    SetForegroundColor(Color::DarkGrey),
                                    SetBackgroundColor(row_bg)
                                )?;
                                print!("{}", ghost_chars);
                                execute!(
                                    self.stdout,
                                    SetForegroundColor(editor_fg),
                                    SetBackgroundColor(row_bg)
                                )?;

                                chars_printed += ghost_chars.chars().count();
                            }
                        }
                    }

                    // Render Copilot ghost text on cursor line
                    // Note: Copilot ghost text can coexist with LSP completion popup
                    // (the popup shows as a dropdown, ghost text shows inline after cursor)
                    if is_cursor_line
                        && is_active
                        && editor.mode == Mode::Insert
                        && editor.copilot_ghost.is_some()
                    {
                        if let Some(ref ghost) = editor.copilot_ghost {
                            // Only show if trigger position matches cursor
                            if ghost.trigger_line == file_line
                                && ghost.trigger_col <= pane.cursor.col
                            {
                                let remaining = pane_width.saturating_sub(chars_printed);
                                let ghost_chars: String =
                                    ghost.inline_text.chars().take(remaining).collect();
                                let ghost_len = ghost_chars.chars().count();

                                // Render Copilot ghost text in a slightly different gray
                                execute!(
                                    self.stdout,
                                    SetForegroundColor(Color::Rgb {
                                        r: 100,
                                        g: 100,
                                        b: 110
                                    }),
                                    SetBackgroundColor(row_bg)
                                )?;
                                print!("{}", ghost_chars);

                                // Show count if multiple completions
                                if !ghost.count_display.is_empty() {
                                    let count_remaining = remaining.saturating_sub(ghost_len + 1);
                                    if count_remaining >= ghost.count_display.len() {
                                        execute!(
                                            self.stdout,
                                            SetForegroundColor(Color::Rgb {
                                                r: 80,
                                                g: 80,
                                                b: 90
                                            })
                                        )?;
                                        print!(" {}", ghost.count_display);
                                        chars_printed += 1 + ghost.count_display.len();
                                    }
                                }

                                execute!(
                                    self.stdout,
                                    SetForegroundColor(editor_fg),
                                    SetBackgroundColor(row_bg)
                                )?;

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
                                let msg: String = diag
                                    .message
                                    .lines()
                                    .next()
                                    .unwrap_or(&diag.message)
                                    .chars()
                                    .take(remaining)
                                    .collect();

                                // Render: space, icon, space, message
                                execute!(
                                    self.stdout,
                                    SetForegroundColor(Color::DarkGrey),
                                    SetBackgroundColor(row_bg)
                                )?;
                                print!(" ");
                                execute!(self.stdout, SetForegroundColor(color))?;
                                print!("{}", icon);
                                execute!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                                print!(" {}", msg);
                                execute!(
                                    self.stdout,
                                    SetForegroundColor(editor_fg),
                                    SetBackgroundColor(row_bg)
                                )?;

                                chars_printed += 3 + msg.chars().count();
                            }
                        }
                    }

                    // Fill remaining space in pane with theme background
                    execute!(
                        self.stdout,
                        SetAttribute(Attribute::Reset),
                        SetBackgroundColor(row_bg),
                        SetForegroundColor(editor_fg)
                    )?;
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
                execute!(
                    self.stdout,
                    SetForegroundColor(editor_fg),
                    SetBackgroundColor(row_bg)
                )?;

                // Fill remaining space (sign column = 2)
                let chars_printed = 2 + if show_line_numbers {
                    line_num_width + 2
                } else {
                    1
                };
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
        virtual_prefix_chars: usize,
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
        diagnostic_hint_color: Color,
        tab_width: usize,
    ) -> anyhow::Result<usize> {
        let chars: Vec<char> = text.chars().collect();

        // Determine the base background for this line
        let base_bg = if is_cursor_line {
            cursor_line_bg
        } else {
            editor_bg
        };

        // Check if a column is within a search match for this line
        let in_search_match = |col: usize| -> bool {
            search_matches
                .iter()
                .any(|(l, start, end)| *l == line_num && col >= *start && col < *end)
        };

        let mut current_fg: Option<Color> = None;
        let mut current_bg: Option<Color> = None;
        let mut current_bold = false;
        let mut current_italic = false;
        let mut current_underline_color: Option<Color> = None;
        let mut rendered_cols = 0;

        for (i, ch) in chars.iter().enumerate() {
            // Calculate the actual column in the original line. Wrapped continuation
            // indentation is virtual text, so it has no source column.
            let source_col =
                source_col_for_wrap_segment_position(col_offset, virtual_prefix_chars, i);

            // Check for visual selection
            let in_visual =
                if let (Some(actual_col), Some((start_line, start_col, end_line, end_col))) =
                    (source_col, visual_range)
                {
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
                        Mode::VisualLine => line_num >= start_line && line_num <= end_line,
                        Mode::VisualBlock => {
                            line_num >= start_line
                                && line_num <= end_line
                                && actual_col >= start_col
                                && actual_col <= end_col
                        }
                        _ => false,
                    }
                } else {
                    false
                };

            // Check if in search match
            let is_search = source_col.map_or(false, in_search_match);

            let diag_at_col = source_col
                .and_then(|actual_col| diagnostic_at_col(diagnostics, line_num, actual_col));

            // Find syntax highlight for this position
            let syntax_style = source_col.and_then(|actual_col| {
                highlights
                    .iter()
                    .find(|h| actual_col >= h.start_col && actual_col < h.end_col)
                    .map(|h| h.style)
            });
            let syntax_color = syntax_style.map(|style| style.fg);

            // Check if within a hint diagnostic (unused variable/import) - grey out the text
            let is_hint_diagnostic =
                diag_at_col.map_or(false, |d| d.severity == DiagnosticSeverity::Hint);

            // Priority: visual selection > search match > hint (grey out) > base
            let (desired_bg, desired_fg) = if in_visual {
                (selection_bg, syntax_color.unwrap_or(editor_fg))
            } else if is_search {
                (search_match_bg, search_match_fg)
            } else if is_hint_diagnostic {
                // Grey out unused code (like Neovim does)
                (base_bg, diagnostic_hint_color)
            } else {
                (base_bg, syntax_color.unwrap_or(editor_fg))
            };
            let desired_style = if in_visual || (!is_search && !is_hint_diagnostic) {
                syntax_style
            } else {
                None
            };
            let desired_bold = desired_style.map_or(false, |style| style.bold);
            let desired_italic = desired_style.map_or(false, |style| style.italic);
            let desired_underline_color = if in_visual || is_search {
                None
            } else {
                diag_at_col
                    .and_then(|diag| diagnostic_underline_color(diag, diagnostic_error_color))
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
            if desired_bold != current_bold {
                execute!(
                    self.stdout,
                    SetAttribute(if desired_bold {
                        Attribute::Bold
                    } else {
                        Attribute::NoBold
                    })
                )?;
                current_bold = desired_bold;
            }
            if desired_italic != current_italic {
                execute!(
                    self.stdout,
                    SetAttribute(if desired_italic {
                        Attribute::Italic
                    } else {
                        Attribute::NoItalic
                    })
                )?;
                current_italic = desired_italic;
            }
            if desired_underline_color != current_underline_color {
                apply_diagnostic_underline(&mut self.stdout, desired_underline_color, desired_fg)?;
                current_underline_color = desired_underline_color;
            }

            rendered_cols += print_editor_char(*ch, tab_width);
        }

        // Restore to base background/foreground and clear text attributes.
        execute!(
            self.stdout,
            SetAttribute(Attribute::Reset),
            SetBackgroundColor(base_bg),
            SetForegroundColor(editor_fg)
        )?;

        Ok(rendered_cols)
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
        execute!(
            self.stdout,
            cursor::MoveTo(0, 0),
            SetBackgroundColor(explorer_bg)
        )?;

        let project_name = editor
            .project_root
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
        execute!(
            self.stdout,
            SetAttribute(Attribute::Reset),
            SetBackgroundColor(explorer_bg)
        )?;

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
            execute!(
                self.stdout,
                cursor::MoveTo(0, y),
                SetBackgroundColor(explorer_bg)
            )?;

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

                // Get icon (use config for Nerd Font vs Unicode fallback)
                let use_nerd_fonts = editor.settings.editor.use_nerd_font_icons;
                let icon = editor.explorer.get_icon(node, use_nerd_fonts);

                // Get icon color
                let icon_color_rgb = crate::explorer::get_icon_color(&node.name, node.is_dir);
                let icon_color = Color::Rgb {
                    r: icon_color_rgb.r,
                    g: icon_color_rgb.g,
                    b: icon_color_rgb.b,
                };

                // Draw tree connector lines for each depth level
                if node.depth > 1 {
                    execute!(self.stdout, SetForegroundColor(tree_line_color))?;
                    for _ in 0..(node.depth - 1) {
                        print!("│ ");
                    }
                }

                // Determine file name color based on file type and hidden status
                let is_hidden = node.name.starts_with('.');
                let is_match = editor.explorer.is_search_match(idx);

                let name_color = if is_match {
                    match_color
                } else if node.is_dir {
                    if is_hidden {
                        hidden_dir_color
                    } else {
                        dir_color
                    }
                } else if is_hidden {
                    hidden_file_color
                } else {
                    file_color
                };

                // Calculate remaining width for the content
                let tree_indent_width = if node.depth > 1 {
                    (node.depth - 1) * 2
                } else {
                    0
                };
                let remaining_width = content_width.saturating_sub(tree_indent_width);

                // Build the full line content first, then pad to exact width
                // Icon (assume 1 char) + space + name
                let dimmed_icon_color = if is_hidden {
                    dim_color(icon_color)
                } else {
                    icon_color
                };

                let git_status = editor.explorer.git_status_for_path(&node.path);

                // Calculate available width for name.
                // Layout: icon + space + git marker column + space + name
                let prefix_width = 4;
                let name_max_width = remaining_width.saturating_sub(prefix_width);

                // Truncate name if needed
                let name_chars: Vec<char> = node.name.chars().collect();
                let name_display = if name_chars.len() > name_max_width {
                    let truncated: String = name_chars
                        .iter()
                        .take(name_max_width.saturating_sub(1))
                        .collect();
                    format!("{}…", truncated)
                } else {
                    node.name.clone()
                };

                // Print icon with color
                execute!(self.stdout, SetForegroundColor(dimmed_icon_color))?;
                print!("{} ", icon);

                // Print git marker with git-sign colors. Keep the marker column
                // stable so file names do not shift when status appears.
                match git_status {
                    Some(crate::git::GitFileStatus::Added) => {
                        execute!(self.stdout, SetForegroundColor(theme.git.added))?;
                        print!("+ ");
                    }
                    Some(crate::git::GitFileStatus::Modified) => {
                        execute!(self.stdout, SetForegroundColor(theme.git.modified))?;
                        print!("x ");
                    }
                    Some(crate::git::GitFileStatus::Deleted) => {
                        execute!(self.stdout, SetForegroundColor(theme.git.deleted))?;
                        print!("- ");
                    }
                    Some(crate::git::GitFileStatus::Untracked) => {
                        execute!(self.stdout, SetForegroundColor(theme.git.added))?;
                        print!("? ");
                    }
                    Some(crate::git::GitFileStatus::Conflicted) => {
                        execute!(self.stdout, SetForegroundColor(theme.diagnostic.error))?;
                        print!("! ");
                    }
                    None => {
                        execute!(self.stdout, SetForegroundColor(name_color))?;
                        print!("  ");
                    }
                }

                // Print name with color, padded to fill remaining width
                execute!(self.stdout, SetForegroundColor(name_color))?;
                let name_display_len = name_display.chars().count();
                print!("{}", name_display);

                // Fill any remaining space with background
                let used_width = prefix_width + name_display_len;
                if used_width < remaining_width {
                    print!("{:width$}", "", width = remaining_width - used_width);
                }
            } else {
                // Empty line - use explorer background
                execute!(
                    self.stdout,
                    SetForegroundColor(explorer_fg),
                    SetBackgroundColor(explorer_bg)
                )?;
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
            if available > 0 {
                let (visible, _) =
                    explorer_action_input_view(input, editor.explorer.input_cursor, available);
                print!("{}", visible);
                let remaining = available.saturating_sub(visible.chars().count());
                print!("{:remaining$}", "", remaining = remaining);
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
            execute!(
                self.stdout,
                SetForegroundColor(theme.ui.statusline_mode_insert)
            )?;
            print!("/");

            // Search buffer
            execute!(self.stdout, SetForegroundColor(explorer_fg))?;
            let search = &editor.explorer.search_buffer;

            let available = width.saturating_sub(1);
            if search.len() <= available {
                print!("{}", search);
                let match_info = editor.explorer.search_match_info();
                let padding = available
                    .saturating_sub(search.len())
                    .saturating_sub(match_info.len());
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
        execute!(
            self.stdout,
            SetForegroundColor(explorer_fg),
            SetBackgroundColor(explorer_bg)
        )?;

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
                let cmd_cursor_col = editor.command_line.display_cursor_col();
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
                    // Must use same window calculation as render_finder
                    let preview_enabled =
                        editor.finder.preview_enabled && editor.finder.mode_supports_preview();
                    let win = crate::finder::FloatingWindow::centered_with_preview(
                        editor.term_width,
                        editor.term_height,
                        preview_enabled,
                    );
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
                    let explorer_width = editor.explorer.width as usize;
                    let available = explorer_width.saturating_sub(prompt.len());
                    let (_, input_cursor_x) = explorer_action_input_view(
                        &editor.explorer.input_buffer,
                        editor.explorer.input_cursor,
                        available,
                    );
                    let cursor_x =
                        (prompt.len() + input_cursor_x).min(explorer_width.saturating_sub(1));
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
                let tab_width = editor.get_effective_tab_width();

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
                            let line_content = buffer
                                .line(line_idx)
                                .map(|l| l.to_string())
                                .unwrap_or_default();
                            let segments = calculate_wrap_segments(
                                &line_content,
                                effective_wrap_width,
                                true,
                                tab_width,
                            );
                            visual_row += segments.len();
                        }
                    }

                    // Now find which segment of the cursor line contains the cursor column
                    let cursor_line_content = buffer
                        .line(editor.cursor.line)
                        .map(|l| l.to_string())
                        .unwrap_or_default();
                    let segments = calculate_wrap_segments(
                        &cursor_line_content,
                        effective_wrap_width,
                        true,
                        tab_width,
                    );

                    let mut cursor_visual_row = visual_row;
                    let mut cursor_visual_col = editor.cursor.col;

                    for (seg_idx, segment) in segments.iter().enumerate() {
                        let segment_end = if seg_idx + 1 < segments.len() {
                            segments[seg_idx + 1].start_col
                        } else {
                            cursor_line_content.chars().count()
                        };

                        if editor.cursor.col >= segment.start_col && editor.cursor.col < segment_end
                        {
                            // Cursor is in this segment
                            cursor_visual_col = display_width_between_char_cols(
                                &cursor_line_content,
                                segment.start_col,
                                editor.cursor.col,
                                tab_width,
                            );
                            // Add indentation offset for wrapped lines
                            if !segment.is_first {
                                let indent_len = cursor_line_content
                                    .chars()
                                    .take_while(|c| c.is_whitespace())
                                    .map(|c| editor_char_display_width(c, tab_width))
                                    .sum::<usize>();
                                cursor_visual_col += indent_len;
                            }
                            break;
                        }
                        cursor_visual_row += 1;
                    }

                    // Handle cursor at end of line
                    if editor.cursor.col
                        >= cursor_line_content.trim_end_matches('\n').chars().count()
                    {
                        cursor_visual_row = visual_row + segments.len().saturating_sub(1);
                        let last_segment = segments.last().unwrap();
                        cursor_visual_col =
                            text_display_width(last_segment.text.trim_end_matches('\n'), tab_width);
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
                    let cursor_row = editor
                        .cursor
                        .line
                        .saturating_sub(active_pane.viewport_offset);
                    // Sign column (2) + line numbers + cursor position (adjusted for horizontal scroll)
                    let cursor_line_content = editor
                        .buffer()
                        .line(editor.cursor.line)
                        .map(|l| l.to_string())
                        .unwrap_or_default();
                    let display_col = display_width_between_char_cols(
                        &cursor_line_content,
                        active_pane.h_offset,
                        editor.cursor.col,
                        tab_width,
                    );
                    let cursor_col = 2 + if show_line_numbers {
                        line_num_width + 1 + display_col
                    } else {
                        display_col
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
                    Mode::Replace => {
                        execute!(self.stdout, cursor::SetCursorStyle::BlinkingUnderScore)?
                    }
                    Mode::Normal
                    | Mode::Visual
                    | Mode::VisualLine
                    | Mode::VisualBlock
                    | Mode::Explorer => {
                        execute!(self.stdout, cursor::SetCursorStyle::BlinkingBlock)?
                    }
                    Mode::Command | Mode::Search | Mode::Finder | Mode::RenamePrompt => {} // Handled above/separately
                }
            }
        }

        Ok(())
    }

    fn render_status_line(
        &mut self,
        editor: &Editor,
        _line_num_width: usize,
    ) -> anyhow::Result<()> {
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
        let pending = if editor.input_state.pending_operator.is_some()
            || editor.input_state.count.is_some()
        {
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
                    Operator::AutoIndent => '=',
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
        let project_name = editor
            .project_root
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| format!("[{}] ", s))
            .unwrap_or_default();

        let mode_display = format!(" {} ", mode_str);
        let rest_left = format!(
            "{}{} | {}{}{} ",
            pending, recording, project_name, filename, modified
        );

        // Right side: LSP status, language and position
        let lsp_status = editor.lsp_status.as_deref().unwrap_or("");
        let lang = editor.syntax.language_name().unwrap_or("plain");
        let right = if lsp_status.is_empty() {
            format!(
                " {} | {}:{} ",
                lang,
                editor.cursor.line + 1,
                editor.cursor.col + 1
            )
        } else {
            format!(
                " {} | {} | {}:{} ",
                lsp_status,
                lang,
                editor.cursor.line + 1,
                editor.cursor.col + 1
            )
        };

        // Calculate padding
        let left_len = mode_display.len() + rest_left.len();
        let padding = width.saturating_sub(left_len + right.len());

        // Render status line: mode badge (colored) + rest (status bar colors)
        // Mode badge with mode-specific color
        execute!(
            self.stdout,
            SetBackgroundColor(mode_color),
            SetForegroundColor(theme.ui.statusline_bg)
        )?;
        print!("{}", mode_display);

        // Rest of status line with standard colors
        execute!(
            self.stdout,
            SetBackgroundColor(theme.ui.statusline_bg),
            SetForegroundColor(theme.ui.statusline_fg)
        )?;
        print!("{}{:padding$}{}", rest_left, "", right, padding = padding);
        execute!(self.stdout, ResetColor)?;

        Ok(())
    }

    fn truncate_inline(text: &str, max_chars: usize) -> String {
        if max_chars == 0 {
            return String::new();
        }

        let total_chars = text.chars().count();
        if total_chars <= max_chars {
            return text.to_string();
        }

        if max_chars <= 3 {
            return text.chars().take(max_chars).collect();
        }

        let mut out: String = text.chars().take(max_chars - 3).collect();
        out.push_str("...");
        out
    }

    /// Render command suggestions/history window above the status line.
    fn render_command_popup(&mut self, editor: &Editor) -> anyhow::Result<()> {
        if editor.mode != Mode::Command {
            return Ok(());
        }

        let command_line = &editor.command_line;
        let (total_items, selected) = match command_line.popup_mode {
            CommandPopupMode::Completion => (
                command_line.suggestions.len(),
                command_line.suggestion_index,
            ),
            CommandPopupMode::History => (
                command_line.history_popup_items.len(),
                command_line.history_popup_index,
            ),
            CommandPopupMode::None => (0, 0),
        };

        if total_items == 0 {
            return Ok(());
        }

        let status_row = editor.term_height.saturating_sub(2);
        if status_row == 0 {
            return Ok(());
        }

        let term_width = editor.term_width as usize;
        if term_width < 8 {
            return Ok(());
        }

        // Reserve one row for a mode label/header.
        let available_rows = status_row as usize;
        if available_rows < 2 {
            return Ok(());
        }
        let item_rows = total_items.min(8).min(available_rows.saturating_sub(1));
        if item_rows == 0 {
            return Ok(());
        }

        let header_text = match command_line.popup_mode {
            CommandPopupMode::Completion => "[COMMANDS] Tab:accept  Shift+Tab:prev  Alt+r:history",
            CommandPopupMode::History => "[HISTORY] Enter/Tab:use  Ctrl+n/p:move  Alt+r:close",
            CommandPopupMode::None => "",
        };

        let mut first_visible = selected.saturating_sub(item_rows.saturating_sub(1));
        if first_visible + item_rows > total_items {
            first_visible = total_items.saturating_sub(item_rows);
        }
        let popup_top_row = status_row.saturating_sub((item_rows + 1) as u16);

        let left_col_width = ((term_width * 2) / 5)
            .max(12)
            .min(30)
            .min(term_width.saturating_sub(4));
        let right_col_width = term_width.saturating_sub(left_col_width + 3);

        let theme = editor.theme();
        let popup_fg = theme.ui.foreground;
        let popup_bg = theme.ui.popup_bg;
        let header_fg = theme.ui.statusline_mode_command;
        let header_bg = theme.ui.statusline_bg;
        let selected_fg = theme.ui.foreground;
        let selected_bg = theme.ui.popup_selection;

        execute!(
            self.stdout,
            cursor::MoveTo(0, popup_top_row),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(header_fg),
            SetBackgroundColor(header_bg)
        )?;
        let mut header_line = Self::truncate_inline(header_text, term_width);
        let header_len = header_line.chars().count();
        if header_len < term_width {
            header_line.push_str(&" ".repeat(term_width - header_len));
        }
        print!("{}", header_line);
        execute!(self.stdout, ResetColor)?;

        for row_offset in 0..item_rows {
            let item_idx = first_visible + row_offset;
            let row = popup_top_row + row_offset as u16 + 1;
            let is_selected = item_idx == selected;

            execute!(
                self.stdout,
                cursor::MoveTo(0, row),
                terminal::Clear(ClearType::CurrentLine)
            )?;

            if is_selected {
                execute!(
                    self.stdout,
                    SetForegroundColor(selected_fg),
                    SetBackgroundColor(selected_bg)
                )?;
            } else {
                execute!(
                    self.stdout,
                    SetForegroundColor(popup_fg),
                    SetBackgroundColor(popup_bg)
                )?;
            }

            let (left_raw, right_raw) = match command_line.popup_mode {
                CommandPopupMode::Completion => {
                    let suggestion = &command_line.suggestions[item_idx];
                    let left = if suggestion
                        .matched_alias
                        .eq_ignore_ascii_case(suggestion.command)
                    {
                        format!(":{}", suggestion.command)
                    } else {
                        format!(":{} [{}]", suggestion.command, suggestion.matched_alias)
                    };
                    (left, suggestion.description.to_string())
                }
                CommandPopupMode::History => {
                    let history_item = &command_line.history_popup_items[item_idx];
                    (format!(":{}", history_item), "history".to_string())
                }
                CommandPopupMode::None => (String::new(), String::new()),
            };

            let marker = if is_selected { ">" } else { " " };
            let left_text = Self::truncate_inline(&left_raw, left_col_width);
            let right_text = Self::truncate_inline(&right_raw, right_col_width);
            let mut line = format!(
                "{} {:left_width$} {}",
                marker,
                left_text,
                right_text,
                left_width = left_col_width
            );

            let printed = line.chars().count();
            if printed < term_width {
                line.push_str(&" ".repeat(term_width - printed));
            } else if printed > term_width {
                line = line.chars().take(term_width).collect();
            }
            print!("{}", line);
            execute!(self.stdout, ResetColor)?;
        }

        Ok(())
    }

    fn render_command_line(&mut self, editor: &Editor) -> anyhow::Result<()> {
        if editor.mode == Mode::Command {
            self.render_command_popup(editor)?;
        }

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
        let cursor_in_pane_row = (editor
            .cursor
            .line
            .saturating_sub(active_pane.viewport_offset)) as u16;

        // Convert to screen coordinates
        let popup_screen_col = pane_x + cursor_in_pane_col;
        let cursor_screen_row = pane_y + cursor_in_pane_row;

        // Calculate widths for label and detail columns (only from filtered items)
        let max_label_len = completion
            .filtered
            .iter()
            .filter_map(|&idx| completion.items.get(idx))
            .map(|item| item.label.len())
            .max()
            .unwrap_or(10)
            .min(30);
        let max_detail_len = completion
            .filtered
            .iter()
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
        let detail_col_width = if max_detail_len > 0 {
            max_detail_len + 2
        } else {
            0
        };
        let popup_width = (label_col_width + detail_col_width + 3) as u16; // +3 for borders
        let popup_width = popup_width.min(editor.term_width - 4);

        // Position popup below cursor with 1 row gap, or above if no room
        let available_below = editor.term_height.saturating_sub(cursor_screen_row + 4);
        let popup_y = if available_below >= popup_height {
            cursor_screen_row + 2 // 1 row gap below cursor line
        } else {
            cursor_screen_row.saturating_sub(popup_height + 1) // 1 row gap above
        };
        let popup_x = popup_screen_col.min(editor.term_width.saturating_sub(popup_width + 2));

        // Colors (Zed-inspired dark theme)
        let border_color = Color::Rgb {
            r: 55,
            g: 55,
            b: 65,
        };
        let bg_color = Color::Rgb {
            r: 30,
            g: 30,
            b: 36,
        };
        let selected_bg = Color::Rgb {
            r: 55,
            g: 65,
            b: 95,
        };
        let detail_color = Color::Rgb {
            r: 100,
            g: 100,
            b: 115,
        };
        let doc_bg = Color::Rgb {
            r: 35,
            g: 35,
            b: 42,
        };

        // Draw top border (rounded corners for Zed-style)
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
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

        for (display_idx, &item_idx) in completion
            .filtered
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(max_items)
        {
            let item = match completion.items.get(item_idx) {
                Some(item) => item,
                None => continue,
            };
            let row = popup_y + 1 + (display_idx - scroll_offset) as u16;
            execute!(self.stdout, cursor::MoveTo(popup_x, row))?;

            let is_selected = display_idx == completion.selected;
            let item_bg = if is_selected { selected_bg } else { bg_color };

            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
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
                Color::Rgb {
                    r: 220,
                    g: 220,
                    b: 225,
                }
            };
            execute!(self.stdout, SetForegroundColor(label_color))?;
            let available_label_width = (popup_width as usize).saturating_sub(detail_col_width + 7);
            let label = if item.label.len() > available_label_width {
                format!(
                    "{}…",
                    &item.label[..available_label_width.saturating_sub(1)]
                )
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

            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│");
        }

        // Draw bottom border (rounded corners for Zed-style)
        let bottom_row = popup_y + 1 + max_items as u16;
        execute!(self.stdout, cursor::MoveTo(popup_x, bottom_row))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
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
                                doc_lines.push((
                                    line.to_string(),
                                    Color::Rgb {
                                        r: 180,
                                        g: 180,
                                        b: 180,
                                    },
                                ));
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
                                        doc_lines.push((
                                            current_line,
                                            Color::Rgb {
                                                r: 180,
                                                g: 180,
                                                b: 180,
                                            },
                                        ));
                                        current_line = word.to_string();
                                    }
                                }
                                if !current_line.is_empty() {
                                    doc_lines.push((
                                        current_line,
                                        Color::Rgb {
                                            r: 180,
                                            g: 180,
                                            b: 180,
                                        },
                                    ));
                                }
                            }
                        }
                    }

                    if !doc_lines.is_empty() {
                        // Calculate separator position (after signature lines, before doc lines)
                        let sig_line_count = if item.detail.is_some() {
                            doc_lines
                                .iter()
                                .take_while(|(_, c)| *c == Color::Cyan)
                                .count()
                        } else {
                            0
                        };

                        // Doc panel height: content + 2 for borders + 1 for separator if needed
                        let separator_height = if has_separator { 1 } else { 0 };
                        let doc_height =
                            (doc_lines.len() as u16 + 2 + separator_height).min(popup_height + 4);
                        let available_height = editor.term_height.saturating_sub(popup_y + 2);
                        let doc_height = doc_height.min(available_height);

                        // Draw doc panel with rounded corners
                        // Top border
                        execute!(self.stdout, cursor::MoveTo(doc_panel_x, popup_y))?;
                        execute!(
                            self.stdout,
                            SetForegroundColor(border_color),
                            SetBackgroundColor(doc_bg)
                        )?;
                        print!("╭");
                        for _ in 0..(doc_width - 2) {
                            print!("─");
                        }
                        print!("╮");

                        // Content lines
                        let mut row_offset = 1u16;
                        let max_content_lines = doc_height.saturating_sub(2) as usize;
                        let mut lines_drawn = 0;

                        for (idx, (line, color)) in
                            doc_lines.iter().enumerate().take(max_content_lines)
                        {
                            // Insert separator after signature lines
                            if has_separator
                                && idx == sig_line_count
                                && lines_drawn < max_content_lines
                            {
                                execute!(
                                    self.stdout,
                                    cursor::MoveTo(doc_panel_x, popup_y + row_offset)
                                )?;
                                execute!(
                                    self.stdout,
                                    SetForegroundColor(border_color),
                                    SetBackgroundColor(doc_bg)
                                )?;
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

                            execute!(
                                self.stdout,
                                cursor::MoveTo(doc_panel_x, popup_y + row_offset)
                            )?;
                            execute!(
                                self.stdout,
                                SetForegroundColor(border_color),
                                SetBackgroundColor(doc_bg)
                            )?;
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
                        execute!(
                            self.stdout,
                            cursor::MoveTo(doc_panel_x, popup_y + row_offset)
                        )?;
                        execute!(
                            self.stdout,
                            SetForegroundColor(border_color),
                            SetBackgroundColor(doc_bg)
                        )?;
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
        let popup_x = cursor_screen_col
            .saturating_sub(2)
            .min(editor.term_width.saturating_sub(popup_width + 1));

        // Colors (Neovim-inspired)
        let border_color = Color::Rgb {
            r: 90,
            g: 90,
            b: 120,
        };
        let bg_color = Color::Rgb {
            r: 25,
            g: 25,
            b: 35,
        };
        let code_bg = Color::Rgb {
            r: 35,
            g: 35,
            b: 50,
        };
        let text_color = Color::Rgb {
            r: 200,
            g: 200,
            b: 210,
        };
        let code_color = Color::Rgb {
            r: 150,
            g: 200,
            b: 255,
        }; // Blue for signatures
        let keyword_color = Color::Rgb {
            r: 255,
            g: 150,
            b: 150,
        }; // Red/pink for keywords
        let type_color = Color::Rgb {
            r: 180,
            g: 220,
            b: 180,
        }; // Green for types
        let separator_color = Color::Rgb {
            r: 70,
            g: 70,
            b: 90,
        };

        // Draw top border (rounded corners)
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
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
                    execute!(
                        self.stdout,
                        SetForegroundColor(border_color),
                        SetBackgroundColor(code_bg)
                    )?;
                    print!("│ ");
                    // Simple syntax highlighting for Rust
                    self.render_hover_code_line(
                        line,
                        content_width,
                        code_color,
                        keyword_color,
                        type_color,
                        code_bg,
                    )?;
                    execute!(
                        self.stdout,
                        SetForegroundColor(border_color),
                        SetBackgroundColor(code_bg)
                    )?;
                    print!(" │");
                }
                HoverLineType::Text => {
                    execute!(
                        self.stdout,
                        SetForegroundColor(border_color),
                        SetBackgroundColor(bg_color)
                    )?;
                    print!("│ ");
                    execute!(self.stdout, SetForegroundColor(text_color))?;
                    let display = if line.chars().count() > content_width {
                        format!(
                            "{}…",
                            line.chars()
                                .take(content_width.saturating_sub(1))
                                .collect::<String>()
                        )
                    } else {
                        format!("{:width$}", line, width = content_width)
                    };
                    print!("{}", display);
                    execute!(self.stdout, SetForegroundColor(border_color))?;
                    print!(" │");
                }
                HoverLineType::Separator => {
                    execute!(
                        self.stdout,
                        SetForegroundColor(separator_color),
                        SetBackgroundColor(bg_color)
                    )?;
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
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│ {:width$} │", "", width = content_width);
        }

        // Draw bottom border (rounded corners)
        let bottom_row = popup_y + popup_height - 1;
        execute!(self.stdout, cursor::MoveTo(popup_x, bottom_row))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
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
            "fn", "pub", "let", "mut", "const", "static", "struct", "enum", "impl", "trait",
            "where", "for", "loop", "while", "if", "else", "match", "return", "async", "await",
            "unsafe", "mod", "use", "crate", "self", "Self", "super", "dyn", "ref", "move", "type",
            "as", "in",
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
                } else if word
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false)
                {
                    type_color // Types typically start with uppercase
                } else {
                    default_color
                };

                execute!(
                    self.stdout,
                    SetForegroundColor(color),
                    SetBackgroundColor(bg_color)
                )?;
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
                execute!(
                    self.stdout,
                    SetForegroundColor(default_color),
                    SetBackgroundColor(bg_color)
                )?;
                print!("{}", chars[i]);
                chars_printed += 1;
                i += 1;
            }
        }

        // Pad remaining space
        if chars_printed < width {
            execute!(
                self.stdout,
                SetForegroundColor(default_color),
                SetBackgroundColor(bg_color)
            )?;
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
        let popup_x = cursor_screen_col
            .saturating_sub(2)
            .min(editor.term_width.saturating_sub(popup_width + 1));

        // Colors
        let border_color = Color::Rgb {
            r: 100,
            g: 100,
            b: 140,
        };
        let bg_color = Color::Rgb {
            r: 30,
            g: 30,
            b: 45,
        };
        let text_color = Color::Rgb {
            r: 200,
            g: 200,
            b: 210,
        };
        let highlight_color = Color::Rgb {
            r: 255,
            g: 200,
            b: 100,
        }; // Yellow for active param

        // Draw top border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        print!("╭");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╮");

        // Draw signature with highlighted parameter
        let content_width = (popup_width - 4) as usize;
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 1))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        print!("│ ");

        // Render the signature with active parameter highlighted
        let label = &signature.label;
        let active_param = help.active_parameter;

        // Find the active parameter offsets
        let highlight_range = active_param
            .and_then(|idx| signature.parameters.get(idx).and_then(|p| p.label_offsets));

        let mut chars_printed = 0;
        let label_chars: Vec<char> = label.chars().collect();
        let mut i = 0;

        while i < label_chars.len() && chars_printed < content_width {
            let in_highlight = highlight_range
                .map(|(start, end)| i >= start && i < end)
                .unwrap_or(false);

            if in_highlight {
                execute!(
                    self.stdout,
                    SetForegroundColor(highlight_color),
                    SetBackgroundColor(bg_color)
                )?;
            } else {
                execute!(
                    self.stdout,
                    SetForegroundColor(text_color),
                    SetBackgroundColor(bg_color)
                )?;
            }

            print!("{}", label_chars[i]);
            chars_printed += 1;
            i += 1;
        }

        // Pad remaining space
        if chars_printed < content_width {
            execute!(
                self.stdout,
                SetForegroundColor(text_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("{:width$}", "", width = content_width - chars_printed);
        }

        execute!(self.stdout, SetForegroundColor(border_color))?;
        print!(" │");

        // Draw bottom border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 2))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        print!("╰");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, ResetColor)?;
        Ok(())
    }

    fn diagnostic_float_text_area_x(editor: &Editor) -> u16 {
        let active_pane = &editor.panes()[editor.active_pane_idx()];
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3) as u16;
        active_pane.rect.x.saturating_add(2 + line_num_width + 1)
    }

    fn diagnostic_float_width(editor: &Editor, max_line_width: usize) -> u16 {
        let active_pane = &editor.panes()[editor.active_pane_idx()];
        let text_area_x = Self::diagnostic_float_text_area_x(editor);
        let pane_right = active_pane.rect.x.saturating_add(active_pane.rect.width);
        let max_width = pane_right
            .saturating_sub(text_area_x)
            .min(editor.term_width.saturating_sub(text_area_x))
            .max(4);

        (max_line_width + 4).min(max_width as usize).max(4) as u16
    }

    /// Calculate where the diagnostic floating popup should start on screen.
    fn diagnostic_float_position(
        editor: &Editor,
        popup_width: u16,
        popup_height: u16,
    ) -> (u16, u16) {
        let active_pane = &editor.panes()[editor.active_pane_idx()];
        let cursor_row = (editor
            .cursor
            .line
            .saturating_sub(active_pane.viewport_offset)) as u16;
        let cursor_screen_row = active_pane.rect.y.saturating_add(cursor_row);

        let popup_y = if cursor_screen_row
            .saturating_add(1)
            .saturating_add(popup_height)
            < editor.term_height.saturating_sub(2)
        {
            cursor_screen_row + 1
        } else if cursor_screen_row > popup_height {
            cursor_screen_row - popup_height
        } else {
            active_pane.rect.y.min(
                editor
                    .term_height
                    .saturating_sub(popup_height.saturating_add(1)),
            )
        };

        let text_area_x = Self::diagnostic_float_text_area_x(editor);
        let cursor_col = editor.cursor.col.saturating_sub(active_pane.h_offset) as u16;
        let cursor_screen_col = text_area_x.saturating_add(cursor_col);
        let pane_right = active_pane.rect.x.saturating_add(active_pane.rect.width);
        let max_popup_x = pane_right.saturating_sub(popup_width).max(text_area_x);
        let popup_x = cursor_screen_col.min(max_popup_x).max(text_area_x);

        (popup_x, popup_y)
    }

    fn markdown_preview_rect(editor: &Editor) -> crate::editor::Rect {
        let width = crate::markdown_preview::preview_popup_width(editor.term_width);
        let max_height = editor.term_height.saturating_sub(4);
        let height = max_height.max(editor.term_height.min(5));

        crate::editor::Rect::new(
            (editor.term_width.saturating_sub(width)) / 2,
            (editor.term_height.saturating_sub(height)) / 2,
            width,
            height,
        )
    }

    fn markdown_preview_visible_rows(editor: &Editor) -> usize {
        Self::markdown_preview_rect(editor).height.saturating_sub(3) as usize
    }

    fn markdown_preview_footer(scroll: usize, total_rows: usize, visible_rows: usize) -> String {
        let position = if total_rows == 0 {
            "0/0".to_string()
        } else {
            let start = scroll.saturating_add(1).min(total_rows);
            let end = scroll
                .saturating_add(visible_rows)
                .min(total_rows)
                .max(start);
            if start == end {
                format!("{start}/{total_rows}")
            } else {
                format!("{start}-{end}/{total_rows}")
            }
        };

        format!(" j/k scroll • Ctrl-d/u page • g/G top/bottom • {position} • q close ")
    }

    fn should_capture_mouse(editor: &Editor) -> bool {
        editor.floating_terminal.is_visible()
    }

    fn should_skip_background(editor: &Editor) -> bool {
        editor.mode == Mode::Finder || editor.markdown_preview.is_some()
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
        let max_line_width = lines
            .iter()
            .map(|(_, l)| l.chars().count())
            .max()
            .unwrap_or(20);
        let popup_width = Self::diagnostic_float_width(editor, max_line_width);
        let popup_height = (lines.len() + 2).min(editor.term_height as usize - 4) as u16;
        let content_width = (popup_width - 4) as usize;

        // Position at the active pane's text area, below the cursor line.
        let (popup_x, popup_y) = Self::diagnostic_float_position(editor, popup_width, popup_height);

        // Colors
        let border_color = Color::Rgb {
            r: 100,
            g: 100,
            b: 140,
        };
        let bg_color = Color::Rgb {
            r: 30,
            g: 30,
            b: 45,
        };
        let title_color = Color::Rgb {
            r: 180,
            g: 180,
            b: 200,
        };

        // Draw top border with title
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        print!("╭");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╮");

        // Draw content lines
        let visible_lines = (popup_height - 2) as usize;
        for (i, (color, line)) in lines.iter().take(visible_lines).enumerate() {
            execute!(self.stdout, cursor::MoveTo(popup_x, popup_y + 1 + i as u16))?;
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
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
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│ {:width$} │", "", width = content_width);
        }

        // Draw bottom border
        execute!(
            self.stdout,
            cursor::MoveTo(popup_x, popup_y + popup_height - 1)
        )?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        print!("╰");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, ResetColor)?;
        Ok(())
    }

    fn render_markdown_preview(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let Some(preview) = &editor.markdown_preview else {
            return Ok(());
        };

        let rect = Self::markdown_preview_rect(editor);
        let visible_rows = rect.height.saturating_sub(3) as usize;
        let inner_width = rect.width.saturating_sub(2) as usize;
        let display_lines = preview.display_lines();
        let scroll = preview
            .scroll
            .min(display_lines.len().saturating_sub(visible_rows));
        let theme = editor.theme();

        execute!(
            self.stdout,
            SetForegroundColor(theme.ui.popup_border),
            SetBackgroundColor(theme.ui.popup_bg),
            cursor::MoveTo(rect.x, rect.y)
        )?;
        print!("╭");
        let title = format!(" {} ", preview.title);
        let title_start = (rect.width as usize).saturating_sub(title.len()) / 2;
        for i in 1..rect.width.saturating_sub(1) {
            if i as usize == title_start {
                print!("{}", title);
            } else if i as usize > title_start && (i as usize) < title_start + title.len() {
                continue;
            } else {
                print!("─");
            }
        }
        print!("╮");

        for row in 0..visible_rows {
            execute!(
                self.stdout,
                cursor::MoveTo(rect.x, rect.y + 1 + row as u16),
                SetForegroundColor(theme.ui.popup_border),
                SetBackgroundColor(theme.ui.popup_bg)
            )?;
            print!("│");

            if let Some(line) = display_lines.get(scroll + row) {
                Self::render_markdown_preview_line(&mut self.stdout, line, inner_width, editor)?;
            } else {
                print!("{:width$}", "", width = inner_width);
            }

            execute!(
                self.stdout,
                SetForegroundColor(theme.ui.popup_border),
                SetBackgroundColor(theme.ui.popup_bg)
            )?;
            print!("│");
        }

        execute!(
            self.stdout,
            cursor::MoveTo(rect.x, rect.y + rect.height.saturating_sub(2)),
            SetForegroundColor(theme.ui.line_number),
            SetBackgroundColor(theme.ui.popup_bg)
        )?;
        print!("│");
        let footer = Self::markdown_preview_footer(scroll, display_lines.len(), visible_rows);
        let footer = take_display_width(&footer, 0, inner_width, editor.settings.editor.tab_width);
        print!("{:^width$}", footer, width = inner_width);
        execute!(self.stdout, SetForegroundColor(theme.ui.popup_border))?;
        print!("│");

        execute!(
            self.stdout,
            cursor::MoveTo(rect.x, rect.y + rect.height.saturating_sub(1)),
            SetForegroundColor(theme.ui.popup_border),
            SetBackgroundColor(theme.ui.popup_bg)
        )?;
        print!("╰");
        for _ in 1..rect.width.saturating_sub(1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, ResetColor)?;
        Ok(())
    }

    fn render_markdown_preview_line<W: Write>(
        output: &mut W,
        line: &crate::markdown_preview::PreviewLine,
        width: usize,
        editor: &Editor,
    ) -> anyhow::Result<()> {
        use crate::markdown_preview::{PreviewLineKind, PreviewSpanStyle};

        let theme = editor.theme();
        let mut written = 0usize;

        if line.kind == PreviewLineKind::Rule {
            execute!(output, SetForegroundColor(theme.ui.line_number))?;
            write!(output, "{}", "─".repeat(width))?;
            return Ok(());
        }

        for span in &line.spans {
            let remaining = width.saturating_sub(written);
            if remaining == 0 {
                break;
            }
            let display: String = span.text.chars().take(remaining).collect();
            let char_count = display.chars().count();

            match span.style {
                PreviewSpanStyle::Plain => {
                    let fg = match line.kind {
                        PreviewLineKind::Heading(_) => theme.syntax.function.fg,
                        PreviewLineKind::Quote => theme.syntax.comment.fg,
                        PreviewLineKind::CodeBlock => theme.syntax.string.fg,
                        PreviewLineKind::Placeholder => theme.ui.line_number,
                        _ => theme.ui.foreground,
                    };
                    execute!(
                        output,
                        SetForegroundColor(fg),
                        SetAttribute(Attribute::Reset)
                    )?;
                }
                PreviewSpanStyle::Emphasis => {
                    execute!(
                        output,
                        SetForegroundColor(theme.ui.foreground),
                        SetAttribute(Attribute::Italic)
                    )?;
                }
                PreviewSpanStyle::Strong => {
                    execute!(
                        output,
                        SetForegroundColor(theme.ui.foreground),
                        SetAttribute(Attribute::Bold)
                    )?;
                }
                PreviewSpanStyle::InlineCode => {
                    execute!(
                        output,
                        SetForegroundColor(theme.syntax.string.fg),
                        SetAttribute(Attribute::Reset)
                    )?;
                }
                PreviewSpanStyle::Link => {
                    execute!(
                        output,
                        SetForegroundColor(theme.syntax.function.fg),
                        SetAttribute(Attribute::Underlined)
                    )?;
                }
            }

            write!(output, "{display}")?;
            written += char_count;
        }

        execute!(
            output,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(theme.ui.foreground)
        )?;
        write!(
            output,
            "{:width$}",
            "",
            width = width.saturating_sub(written)
        )?;
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
        let border_color = Color::Rgb {
            r: 100,
            g: 140,
            b: 180,
        };
        let bg_color = Color::Rgb {
            r: 25,
            g: 25,
            b: 30,
        };
        let selected_bg = Color::Rgb {
            r: 50,
            g: 70,
            b: 100,
        };
        let text_color = Color::Rgb {
            r: 200,
            g: 200,
            b: 210,
        };
        let file_color = Color::Rgb {
            r: 130,
            g: 180,
            b: 250,
        };
        let _line_num_color = Color::Rgb {
            r: 180,
            g: 180,
            b: 100,
        };

        // Draw top border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
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
                execute!(
                    self.stdout,
                    SetForegroundColor(border_color),
                    SetBackgroundColor(bg_color)
                )?;
                print!("│{:width$}│", "", width = (popup_width - 2) as usize);
                continue;
            }

            let loc = &picker.items[idx];
            let is_selected = idx == picker.selected;

            let current_bg = if is_selected { selected_bg } else { bg_color };

            // Border
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│");

            // Item content
            execute!(self.stdout, SetBackgroundColor(current_bg))?;

            // Format: filename:line:col
            let path_str = crate::lsp::uri_to_path(&loc.uri)
                .map(|p| {
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                })
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
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│");
        }

        // Draw bottom border
        execute!(
            self.stdout,
            cursor::MoveTo(popup_x, popup_y + popup_height - 1)
        )?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
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
        let max_title_len = picker
            .items
            .iter()
            .map(|a| a.title.len())
            .max()
            .unwrap_or(20);

        let max_width = 80u16;
        let max_height = 12u16;
        let popup_width = (max_title_len as u16 + 4)
            .min(max_width)
            .min(editor.term_width.saturating_sub(4));
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
        let border_color = Color::Rgb {
            r: 140,
            g: 100,
            b: 180,
        };
        let bg_color = Color::Rgb {
            r: 25,
            g: 25,
            b: 30,
        };
        let selected_bg = Color::Rgb {
            r: 70,
            g: 50,
            b: 100,
        };
        let text_color = Color::Rgb {
            r: 200,
            g: 200,
            b: 210,
        };
        let _kind_color = Color::Rgb {
            r: 130,
            g: 180,
            b: 130,
        };
        let preferred_color = Color::Rgb {
            r: 255,
            g: 200,
            b: 100,
        };

        // Draw top border
        execute!(self.stdout, cursor::MoveTo(popup_x, popup_y))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        let title = " Code Actions ";
        let title_start = (popup_width as usize - title.len()) / 2;
        print!("╭");
        for i in 1..(popup_width - 1) {
            if i as usize == title_start {
                print!("{}", title);
            } else if i as usize > title_start && (i as usize) < title_start + title.len() {
                // Skip (title already printed)
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
                execute!(
                    self.stdout,
                    SetForegroundColor(border_color),
                    SetBackgroundColor(bg_color)
                )?;
                print!("│{:width$}│", "", width = (popup_width - 2) as usize);
                continue;
            }

            let action = &picker.items[idx];
            let is_selected = idx == picker.selected;

            let current_bg = if is_selected { selected_bg } else { bg_color };

            // Border
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│");

            // Item content
            execute!(self.stdout, SetBackgroundColor(current_bg))?;

            let content_width = (popup_width - 2) as usize;

            // Title (highlight preferred items with different color)
            if action.is_preferred {
                execute!(self.stdout, SetForegroundColor(preferred_color))?;
            } else {
                execute!(self.stdout, SetForegroundColor(text_color))?;
            }
            let title_display: String = action.title.chars().take(content_width).collect();
            print!("{}", title_display);

            // Pad
            let printed = title_display.len();
            if printed < content_width {
                print!("{:width$}", "", width = content_width - printed);
            }

            // Closing border
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│");
        }

        // Draw bottom border
        execute!(
            self.stdout,
            cursor::MoveTo(popup_x, popup_y + popup_height - 1)
        )?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
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
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
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
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
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
            execute!(
                self.stdout,
                cursor::MoveTo(popup_x, popup_y + 2 + row as u16)
            )?;

            // Left border
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
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
                    name.chars()
                        .take(name_max.saturating_sub(1))
                        .collect::<String>()
                        + "…"
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
                        print!(
                            "{}",
                            &display_name[match_start..match_start + picker.query.len()]
                        );
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
                    let padding = inner_width
                        .saturating_sub(prefix.len() + display_name.len() + suffix.len());
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
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│");
        }

        // Draw separator line (immediately after items)
        let separator_y = popup_y + 2 + visible_count as u16;
        execute!(self.stdout, cursor::MoveTo(popup_x, separator_y))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        print!("├");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("┤");

        // Draw help line with count
        execute!(self.stdout, cursor::MoveTo(popup_x, separator_y + 1))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        print!("│");
        execute!(
            self.stdout,
            SetForegroundColor(user_color),
            SetBackgroundColor(bg_color)
        )?;
        let count_str = format!("{}/{}", filtered_count, picker.all_items.len());
        let help = "Type to filter • j/k nav • Enter";
        let combined = format!("{} {}", help, count_str);
        print!("{:^width$}", combined, width = inner_width);
        execute!(self.stdout, SetForegroundColor(border_color))?;
        print!("│");

        // Draw bottom border
        execute!(self.stdout, cursor::MoveTo(popup_x, separator_y + 2))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        print!("╰");
        for _ in 1..(popup_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(
            self.stdout,
            SetForegroundColor(text_color),
            SetBackgroundColor(bg_color)
        )?;
        Ok(())
    }

    /// Render the floating terminal
    fn render_floating_terminal(&mut self, editor: &Editor) -> anyhow::Result<()> {
        // Calculate configured terminal dimensions, centered.
        let terminal = &editor.settings.terminal;
        let (term_width, term_height) = crate::floating_terminal::popup_size_for_screen(
            editor.term_width,
            editor.term_height,
            terminal.popup_width_ratio,
            terminal.popup_height_ratio,
        );

        // Center the terminal
        let term_x = (editor.term_width.saturating_sub(term_width)) / 2;
        let term_y = (editor.term_height.saturating_sub(term_height)) / 2;

        let theme = editor.theme();
        let border_color = theme.ui.popup_border;
        let bg_color = theme.ui.popup_bg;
        let text_color = theme.ui.foreground;
        let title_color = theme.ui.finder_prompt;
        let hint_color = theme.ui.line_number;

        // Draw top border with title
        execute!(self.stdout, cursor::MoveTo(term_x, term_y))?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        let full_hint = " [C-\\ hide  C-S-t new  C-Tab next] ";
        let compact_hint = " [<C-\\>] ";
        let close_hint = if term_width as usize >= full_hint.len() + 24 {
            full_hint
        } else {
            compact_hint
        };
        let title_start = 2usize;
        let close_start = (term_width as usize).saturating_sub(close_hint.len() + 2);
        let max_title_width = close_start.saturating_sub(title_start).saturating_sub(1);
        let mut title = editor.floating_terminal.title();
        if title.chars().count() > max_title_width {
            title = title.chars().take(max_title_width).collect();
        }
        let title_width = title.chars().count();

        print!("╭");
        for i in 1..(term_width - 1) {
            let i = i as usize;
            if i == title_start {
                execute!(self.stdout, SetForegroundColor(title_color))?;
                print!("{}", title);
                execute!(self.stdout, SetForegroundColor(border_color))?;
            } else if i > title_start && i < title_start + title_width {
                // Skip - title already printed
            } else if i == close_start {
                execute!(self.stdout, SetForegroundColor(hint_color))?;
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
        let cells = editor
            .floating_terminal
            .get_visible_cells(content_height, content_width);
        let cursor_info = editor.floating_terminal.get_cursor_info();

        // Draw terminal content
        for (row, line) in cells.iter().enumerate() {
            execute!(self.stdout, cursor::MoveTo(term_x, term_y + 1 + row as u16))?;
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│");

            let mut active_style = None;
            for cell in line {
                let mut style = TerminalRenderStyle::from_terminal_cell(cell, text_color, bg_color);
                if cell.search_match {
                    style.fg = theme.ui.search_match_fg;
                    style.bg = theme.ui.search_match_bg;
                }
                if cell.active_search_match {
                    style.fg = text_color;
                    style.bg = theme.ui.popup_selection;
                }
                if cell.selected {
                    style.bg = theme.ui.selection;
                }
                if active_style != Some(style) {
                    self.apply_terminal_cell_style(style)?;
                    active_style = Some(style);
                }
                print!("{}", if cell.hidden { ' ' } else { cell.ch });
            }

            execute!(
                self.stdout,
                SetAttribute(Attribute::Reset),
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│");
        }

        // Fill remaining rows if content is shorter
        for row in cells.len()..content_height {
            execute!(self.stdout, cursor::MoveTo(term_x, term_y + 1 + row as u16))?;
            execute!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(bg_color)
            )?;
            print!("│");
            execute!(self.stdout, SetForegroundColor(text_color))?;
            print!("{:<width$}", "", width = content_width);
            execute!(self.stdout, SetForegroundColor(border_color))?;
            print!("│");
        }

        // Draw bottom border
        execute!(
            self.stdout,
            cursor::MoveTo(term_x, term_y + term_height - 1)
        )?;
        execute!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(bg_color)
        )?;
        print!("╰");
        for _ in 1..(term_width - 1) {
            print!("─");
        }
        print!("╯");

        execute!(self.stdout, ResetColor)?;

        // Position cursor inside the terminal
        if editor.floating_terminal.has_selection()
            || editor.floating_terminal.is_searching()
            || cursor_info.shape == crate::floating_terminal::TerminalCursorShape::Hidden
        {
            execute!(self.stdout, cursor::Hide)?;
        } else {
            let cursor_x = term_x + 1 + cursor_info.col.min(content_width.saturating_sub(1)) as u16;
            let cursor_y =
                term_y + 1 + cursor_info.row.min(content_height.saturating_sub(1)) as u16;
            execute!(
                self.stdout,
                cursor::MoveTo(cursor_x, cursor_y),
                terminal_cursor_style(cursor_info),
                cursor::Show
            )?;
        }

        Ok(())
    }

    fn apply_terminal_cell_style(&mut self, style: TerminalRenderStyle) -> anyhow::Result<()> {
        execute!(
            self.stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(style.fg),
            SetBackgroundColor(style.bg)
        )?;

        if style.bold {
            execute!(self.stdout, SetAttribute(Attribute::Bold))?;
        }
        if style.dim {
            execute!(self.stdout, SetAttribute(Attribute::Dim))?;
        }
        if style.italic {
            execute!(self.stdout, SetAttribute(Attribute::Italic))?;
        }
        if style.underline {
            execute!(self.stdout, SetAttribute(Attribute::Underlined))?;
        }
        if style.double_underline {
            execute!(self.stdout, SetAttribute(Attribute::DoubleUnderlined))?;
        }
        if style.undercurl {
            execute!(self.stdout, SetAttribute(Attribute::Undercurled))?;
        }
        if style.underdotted {
            execute!(self.stdout, SetAttribute(Attribute::Underdotted))?;
        }
        if style.underdashed {
            execute!(self.stdout, SetAttribute(Attribute::Underdashed))?;
        }
        if style.strikeout {
            execute!(self.stdout, SetAttribute(Attribute::CrossedOut))?;
        }

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
        let t_start = Instant::now();
        use crate::finder::FuzzyFinder;

        let preview_enabled =
            editor.finder.preview_enabled && editor.finder.mode_supports_preview();
        let win = crate::finder::FloatingWindow::centered_with_preview(
            editor.term_width,
            editor.term_height,
            preview_enabled,
        );

        // No clearing needed - window size is constant, rows fully overwrite their content

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

        // Calculate panel widths
        // Window is always same size - only internal layout changes
        let (results_width, preview_width) = if preview_enabled {
            // 50/50 split for results and preview (minus borders)
            let total_inner = (win.width - 3) as usize; // -2 outer borders, -1 separator
            let results = total_inner / 2;
            let preview = total_inner.saturating_sub(results);
            (results, preview)
        } else {
            // Results takes full width when preview is off
            ((win.width - 2) as usize, 0)
        };

        // Draw top border with title
        queue!(
            self.stdout,
            cursor::MoveTo(win.x, win.y),
            SetForegroundColor(border_color)
        )?;
        write!(self.stdout, "\u{250c}")?; // ┌
        let title = match editor.finder.mode {
            crate::finder::FinderMode::Files => " Find Files ",
            crate::finder::FinderMode::Grep => " Live Grep ",
            crate::finder::FinderMode::Buffers => " Buffers ",
            crate::finder::FinderMode::Diagnostics => " Diagnostics ",
            crate::finder::FinderMode::Harpoon => " Harpoon ",
            crate::finder::FinderMode::Marks => " Marks ",
            crate::finder::FinderMode::GitChanges => " Git Changes ",
            crate::finder::FinderMode::Terminals => " Terminals ",
            crate::finder::FinderMode::Keymaps => " Key Maps ",
        };

        if preview_enabled {
            // Title centered over results panel
            let title_start = (results_width.saturating_sub(title.len())) / 2;
            for i in 1..=results_width {
                if i == title_start + 1 {
                    queue!(self.stdout, SetForegroundColor(title_color))?;
                    write!(self.stdout, "{}", title)?;
                    queue!(self.stdout, SetForegroundColor(border_color))?;
                } else if i > title_start && i <= title_start + title.len() {
                    // Skip - already printed title
                } else {
                    write!(self.stdout, "\u{2500}")?; // ─
                }
            }
            // Separator junction
            write!(self.stdout, "\u{252c}")?; // ┬

            // Preview header with filename
            let preview_title = if let Some(item) = editor.finder.selected_item() {
                let filename = item
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Preview");
                format!(" {} ", filename)
            } else {
                " Preview ".to_string()
            };
            let preview_title_start = (preview_width.saturating_sub(preview_title.len())) / 2;
            for i in 0..preview_width {
                if i == preview_title_start {
                    queue!(self.stdout, SetForegroundColor(title_color))?;
                    write!(self.stdout, "{}", preview_title)?;
                    queue!(self.stdout, SetForegroundColor(border_color))?;
                } else if i > preview_title_start && i < preview_title_start + preview_title.len() {
                    // Skip - already printed title
                } else {
                    write!(self.stdout, "\u{2500}")?; // ─
                }
            }
        } else {
            let title_start = (win.width as usize - title.len()) / 2;
            for i in 1..(win.width - 1) {
                if i as usize == title_start {
                    queue!(self.stdout, SetForegroundColor(title_color))?;
                    write!(self.stdout, "{}", title)?;
                    queue!(self.stdout, SetForegroundColor(border_color))?;
                } else if i as usize >= title_start && (i as usize) < title_start + title.len() {
                    // Skip - already printed title
                } else {
                    write!(self.stdout, "\u{2500}")?; // ─
                }
            }
        }
        write!(self.stdout, "\u{2510}")?; // ┐

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
            queue!(
                self.stdout,
                cursor::MoveTo(win.x, y),
                SetForegroundColor(border_color),
                SetBackgroundColor(finder_bg)
            )?;
            write!(self.stdout, "\u{2502}")?; // │

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
                    queue!(self.stdout, SetBackgroundColor(selected_bg))?;
                }

                // Get file type indicator (2 chars + space)
                let icon = item
                    .icon
                    .unwrap_or_else(|| FuzzyFinder::get_file_icon(&item.path));
                let icon_color = match icon {
                    "TR" => Color::Rgb {
                        r: 90,
                        g: 210,
                        b: 120,
                    }, // Terminal - green
                    "RS" => Color::Rgb {
                        r: 255,
                        g: 100,
                        b: 50,
                    }, // Rust - orange
                    "TS" | "TX" => Color::Rgb {
                        r: 50,
                        g: 150,
                        b: 255,
                    }, // TypeScript - blue
                    "JS" | "JX" => Color::Rgb {
                        r: 255,
                        g: 220,
                        b: 50,
                    }, // JavaScript - yellow
                    "PY" => Color::Rgb {
                        r: 80,
                        g: 180,
                        b: 80,
                    }, // Python - green
                    "GO" => Color::Rgb {
                        r: 100,
                        g: 200,
                        b: 220,
                    }, // Go - cyan
                    "RB" => Color::Rgb {
                        r: 220,
                        g: 50,
                        b: 50,
                    }, // Ruby - red
                    "HT" => Color::Rgb {
                        r: 230,
                        g: 100,
                        b: 50,
                    }, // HTML - orange
                    "CS" | "SC" => Color::Rgb {
                        r: 100,
                        g: 150,
                        b: 255,
                    }, // CSS - blue
                    "MD" => Color::Rgb {
                        r: 150,
                        g: 150,
                        b: 150,
                    }, // Markdown - gray
                    "YM" | "TM" | "CF" => Color::Rgb {
                        r: 180,
                        g: 140,
                        b: 100,
                    }, // Config - tan
                    "GT" => Color::Rgb {
                        r: 240,
                        g: 80,
                        b: 50,
                    }, // Git - red-orange
                    "EN" => Color::Rgb {
                        r: 255,
                        g: 200,
                        b: 50,
                    }, // Env - yellow
                    "SH" | "ZS" | "FS" => Color::Rgb {
                        r: 100,
                        g: 200,
                        b: 100,
                    }, // Shell - green
                    _ => Color::Rgb {
                        r: 120,
                        g: 120,
                        b: 120,
                    }, // Default - gray
                };
                queue!(self.stdout, SetForegroundColor(icon_color))?;
                write!(self.stdout, "{} ", icon)?;

                // Truncate display to fit and highlight matches
                // Leave space for icon (3 chars) and scroll indicator if needed
                let icon_width = 3; // icon + space
                let base_width = if show_scroll_indicator {
                    results_width.saturating_sub(2) // -1 for scroll indicator, -1 for spacing
                } else {
                    results_width.saturating_sub(1) // -1 for spacing
                };
                let max_len = base_width.saturating_sub(icon_width);
                let display_chars: Vec<char> = item.display.chars().take(max_len).collect();

                // Reset foreground color for text
                if is_selected {
                    queue!(self.stdout, SetForegroundColor(finder_fg))?;
                } else {
                    queue!(
                        self.stdout,
                        SetForegroundColor(finder_fg),
                        SetBackgroundColor(finder_bg)
                    )?;
                }

                // For diagnostics mode, color the severity indicator
                let is_diagnostics_mode =
                    editor.finder.mode == crate::finder::FinderMode::Diagnostics;
                let mut skip_severity_coloring = 0;

                // Check for severity prefix and render it with color
                if is_diagnostics_mode && display_chars.len() >= 3 {
                    let prefix: String = display_chars[0..3].iter().collect();
                    let severity_color = match prefix.as_str() {
                        "[E]" => Some(Color::Rgb {
                            r: 255,
                            g: 80,
                            b: 80,
                        }), // Red for errors
                        "[W]" => Some(Color::Rgb {
                            r: 255,
                            g: 200,
                            b: 50,
                        }), // Yellow for warnings
                        "[I]" => Some(Color::Rgb {
                            r: 100,
                            g: 180,
                            b: 255,
                        }), // Blue for info
                        "[H]" => Some(Color::Rgb {
                            r: 150,
                            g: 150,
                            b: 150,
                        }), // Gray for hints
                        _ => None,
                    };

                    if let Some(color) = severity_color {
                        queue!(self.stdout, SetForegroundColor(color))?;
                        write!(self.stdout, "{}", prefix)?;
                        skip_severity_coloring = 3;

                        // Reset to normal text color
                        if is_selected {
                            queue!(
                                self.stdout,
                                SetForegroundColor(finder_fg),
                                SetBackgroundColor(selected_bg)
                            )?;
                        } else {
                            queue!(
                                self.stdout,
                                SetForegroundColor(finder_fg),
                                SetBackgroundColor(finder_bg)
                            )?;
                        }
                    }
                }

                // Batch characters into spans to reduce print calls and color changes
                // Build normal text and highlighted text separately, then print in spans
                let mut current_span = String::new();
                let mut in_highlight = false;

                for (char_idx, ch) in display_chars.iter().enumerate() {
                    if char_idx < skip_severity_coloring {
                        continue;
                    }

                    let is_match = item.match_indices.contains(&char_idx);

                    if is_match != in_highlight {
                        // Flush current span
                        if !current_span.is_empty() {
                            if in_highlight {
                                queue!(self.stdout, SetForegroundColor(match_color))?;
                            }
                            write!(self.stdout, "{}", current_span)?;
                            if in_highlight {
                                if is_selected {
                                    queue!(
                                        self.stdout,
                                        SetForegroundColor(finder_fg),
                                        SetBackgroundColor(selected_bg)
                                    )?;
                                } else {
                                    queue!(
                                        self.stdout,
                                        SetForegroundColor(finder_fg),
                                        SetBackgroundColor(finder_bg)
                                    )?;
                                }
                            }
                            current_span.clear();
                        }
                        in_highlight = is_match;
                    }
                    current_span.push(*ch);
                }

                // Flush final span
                if !current_span.is_empty() {
                    if in_highlight {
                        queue!(self.stdout, SetForegroundColor(match_color))?;
                    }
                    write!(self.stdout, "{}", current_span)?;
                    if in_highlight {
                        if is_selected {
                            queue!(
                                self.stdout,
                                SetForegroundColor(finder_fg),
                                SetBackgroundColor(selected_bg)
                            )?;
                        } else {
                            queue!(
                                self.stdout,
                                SetForegroundColor(finder_fg),
                                SetBackgroundColor(finder_bg)
                            )?;
                        }
                    }
                }

                // Pad to fill results panel (batched)
                let pad_len = max_len.saturating_sub(display_chars.len());
                if pad_len > 0 {
                    write!(self.stdout, "{}", " ".repeat(pad_len))?;
                }
                // Print the spacing column that was reserved
                write!(self.stdout, " ")?;

                // Reset after selected item
                if is_selected {
                    queue!(
                        self.stdout,
                        SetForegroundColor(finder_fg),
                        SetBackgroundColor(finder_bg)
                    )?;
                }
            } else {
                // Empty row - set finder background (batched padding)
                queue!(self.stdout, SetBackgroundColor(finder_bg))?;
                let pad_len = if show_scroll_indicator {
                    results_width.saturating_sub(1) // -1 for scroll indicator
                } else {
                    results_width
                };
                if pad_len > 0 {
                    write!(self.stdout, "{}", " ".repeat(pad_len))?;
                }
            }

            // Draw scroll indicator for results panel
            if show_scroll_indicator {
                // Calculate which part of the scrollbar to highlight
                let scroll_bar_pos = if total_items > 0 {
                    (row * total_items) / list_height
                } else {
                    0
                };
                let selected_in_range = scroll_bar_pos <= editor.finder.selected
                    && editor.finder.selected < scroll_bar_pos + (total_items / list_height).max(1);

                if selected_in_range
                    || (row == 0 && scroll_offset == 0)
                    || (row == list_height - 1 && scroll_offset + list_height >= total_items)
                {
                    queue!(self.stdout, SetForegroundColor(title_color))?;
                    write!(self.stdout, "\u{2588}")?; // █ (full block for thumb)
                } else if scroll_offset > 0 || scroll_offset + list_height < total_items {
                    queue!(self.stdout, SetForegroundColor(scroll_indicator_color))?;
                    write!(self.stdout, "\u{2591}")?; // ░ (light shade for track)
                } else {
                    write!(self.stdout, " ")?;
                }
            }

            // Draw separator and preview panel if enabled
            if preview_enabled {
                queue!(
                    self.stdout,
                    SetForegroundColor(border_color),
                    SetBackgroundColor(finder_bg)
                )?;
                write!(self.stdout, "\u{2502}")?; // │ vertical separator

                // Render preview content for this row
                self.render_finder_preview_row(editor, row, list_height, preview_width)?;
            }

            queue!(
                self.stdout,
                SetForegroundColor(border_color),
                SetBackgroundColor(finder_bg)
            )?;
            write!(self.stdout, "\u{2502}")?; // │ right border
        }

        // Draw separator above input
        let sep_y = win.y + 1 + list_height as u16;
        queue!(
            self.stdout,
            cursor::MoveTo(win.x, sep_y),
            SetForegroundColor(border_color)
        )?;
        write!(self.stdout, "\u{251c}")?; // ├
        if preview_enabled {
            for _ in 0..results_width {
                write!(self.stdout, "\u{2500}")?; // ─
            }
            write!(self.stdout, "\u{2534}")?; // ┴ (junction with preview separator)
            for _ in 0..preview_width {
                write!(self.stdout, "\u{2500}")?; // ─
            }
        } else {
            for _ in 1..(win.width - 1) {
                write!(self.stdout, "\u{2500}")?; // ─
            }
        }
        write!(self.stdout, "\u{2524}")?; // ┤

        // Draw input line (above bottom border)
        let input_y = sep_y + 1;
        queue!(
            self.stdout,
            cursor::MoveTo(win.x, input_y),
            SetForegroundColor(border_color),
            SetBackgroundColor(finder_bg)
        )?;
        write!(self.stdout, "\u{2502}")?; // │
        queue!(self.stdout, SetBackgroundColor(input_bg))?;

        // Mode indicator
        let mode_str = if editor.finder.is_normal_mode() {
            "N"
        } else {
            "I"
        };
        queue!(self.stdout, SetForegroundColor(mode_color))?;
        write!(self.stdout, "[{}]", mode_str)?;
        queue!(self.stdout, SetForegroundColor(finder_fg))?;
        write!(self.stdout, " > ")?;

        // Query text
        let prefix_len = 6; // "[N] > " or "[I] > "
        let query_display: String = editor
            .finder
            .query
            .chars()
            .take((win.width - prefix_len as u16 - 3) as usize)
            .collect();
        write!(self.stdout, "{}", query_display)?;

        // Pad to fill line (terminal cursor will be positioned separately)
        let used = prefix_len + query_display.len();
        for _ in used..(win.width - 2) as usize {
            write!(self.stdout, " ")?;
        }
        queue!(
            self.stdout,
            SetForegroundColor(border_color),
            SetBackgroundColor(finder_bg)
        )?;
        write!(self.stdout, "\u{2502}")?; // │

        // Draw bottom border with count indicator
        let status = format!(" {} ", editor.finder.status_text());
        queue!(
            self.stdout,
            cursor::MoveTo(win.x, win.y + win.height - 1),
            SetForegroundColor(border_color)
        )?;
        write!(self.stdout, "\u{2514}")?; // └

        if preview_enabled {
            // Status centered over results panel, preview indicator shown
            let status_start = (results_width.saturating_sub(status.len())) / 2;
            for i in 0..results_width {
                if i == status_start {
                    queue!(self.stdout, SetForegroundColor(theme.ui.line_number))?;
                    write!(self.stdout, "{}", status)?;
                    queue!(self.stdout, SetForegroundColor(border_color))?;
                } else if i > status_start && i < status_start + status.len() {
                    // Skip - already printed status
                } else {
                    write!(self.stdout, "\u{2500}")?; // ─
                }
            }
            // Preview toggle hint
            let hint = " Ctrl+t: toggle ";
            let hint_start = (preview_width.saturating_sub(hint.len())) / 2;
            for i in 0..preview_width {
                if i == hint_start {
                    queue!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                    write!(self.stdout, "{}", hint)?;
                    queue!(self.stdout, SetForegroundColor(border_color))?;
                } else if i > hint_start && i < hint_start + hint.len() {
                    // Skip - already printed hint
                } else {
                    write!(self.stdout, "\u{2500}")?; // ─
                }
            }
        } else {
            let status_start = (win.width as usize - status.len()) / 2;
            for i in 1..(win.width - 1) {
                if i as usize == status_start {
                    queue!(self.stdout, SetForegroundColor(theme.ui.line_number))?;
                    write!(self.stdout, "{}", status)?;
                    queue!(self.stdout, SetForegroundColor(border_color))?;
                } else if i as usize >= status_start && (i as usize) < status_start + status.len() {
                    // Skip - already printed status
                } else {
                    write!(self.stdout, "\u{2500}")?; // ─
                }
            }
        }
        write!(self.stdout, "\u{2518}")?; // ┘

        queue!(
            self.stdout,
            SetForegroundColor(finder_fg),
            SetBackgroundColor(finder_bg)
        )?;

        // Flush all queued commands at once to prevent flicker
        self.stdout.flush()?;

        log_finder_profile(&format!(
            "render_finder: {:?} items={} preview={}",
            t_start.elapsed(),
            editor.finder.filtered.len(),
            preview_enabled
        ));

        Ok(())
    }

    /// Render a single row of the preview panel
    fn render_finder_preview_row(
        &mut self,
        editor: &Editor,
        row: usize,
        list_height: usize,
        preview_width: usize,
    ) -> anyhow::Result<()> {
        let theme = editor.theme();
        let finder_bg = theme.ui.finder_bg;
        let finder_fg = theme.ui.foreground;
        let line_num_color = theme.ui.line_number;
        let search_match_bg = theme.ui.search_match_bg;
        let search_match_fg = theme.ui.search_match_fg;

        queue!(
            self.stdout,
            SetBackgroundColor(finder_bg),
            SetForegroundColor(finder_fg)
        )?;

        // Get preview content
        let preview_content = &editor.finder.preview_content;
        let preview_scroll = editor.finder.preview_scroll;
        let preview_line_offset = editor.finder.preview_line_offset;

        if preview_content.is_empty() {
            // No preview available
            if row == list_height / 2 {
                let msg = "No preview";
                let padding = (preview_width.saturating_sub(msg.len())) / 2;
                queue!(self.stdout, SetForegroundColor(Color::DarkGrey))?;
                for _ in 0..padding {
                    write!(self.stdout, " ")?;
                }
                write!(self.stdout, "{}", msg)?;
                for _ in 0..(preview_width.saturating_sub(padding + msg.len())) {
                    write!(self.stdout, " ")?;
                }
            } else {
                for _ in 0..preview_width {
                    write!(self.stdout, " ")?;
                }
            }
            return Ok(());
        }

        let line_idx = preview_scroll + row;

        if line_idx < preview_content.len() {
            let line = &preview_content[line_idx];

            // Line number (4 chars + separator)
            let line_num_width = 4;
            queue!(self.stdout, SetForegroundColor(line_num_color))?;
            write!(
                self.stdout,
                "{:>width$}\u{2502}",
                preview_line_offset + line_idx + 1,
                width = line_num_width
            )?;

            // Get syntax highlights if available
            let highlights = self.get_preview_highlights(editor, line_idx);
            let grep_match_ranges = if editor.finder.mode == crate::finder::FinderMode::Grep
                && editor.finder.query.len() >= 2
            {
                finder_preview_match_ranges(line, &editor.finder.query)
            } else {
                Vec::new()
            };

            // Render line content with syntax highlighting
            let content_width = preview_width.saturating_sub(line_num_width + 1);
            let chars: Vec<char> = line.chars().take(content_width).collect();

            queue!(
                self.stdout,
                SetBackgroundColor(finder_bg),
                SetForegroundColor(finder_fg)
            )?;

            if highlights.is_empty() && grep_match_ranges.is_empty() {
                // No highlighting, just render the text
                for ch in &chars {
                    write!(self.stdout, "{}", ch)?;
                }
            } else {
                // Render with syntax highlighting, with grep matches overlaid.
                let mut highlight_idx = 0;
                let mut current_fg: Option<Color> = Some(finder_fg);
                let mut current_bg: Option<Color> = Some(finder_bg);
                for (col, ch) in chars.iter().enumerate() {
                    let syntax_color =
                        Self::get_syntax_color_at(&highlights, col, &mut highlight_idx);
                    let is_grep_match = grep_match_ranges
                        .iter()
                        .any(|(start, end)| col >= *start && col < *end);

                    let (desired_bg, desired_fg) = if is_grep_match {
                        (search_match_bg, search_match_fg)
                    } else {
                        (finder_bg, syntax_color.unwrap_or(finder_fg))
                    };

                    if Some(desired_bg) != current_bg {
                        queue!(self.stdout, SetBackgroundColor(desired_bg))?;
                        current_bg = Some(desired_bg);
                    }
                    if Some(desired_fg) != current_fg {
                        queue!(self.stdout, SetForegroundColor(desired_fg))?;
                        current_fg = Some(desired_fg);
                    }
                    write!(self.stdout, "{}", ch)?;
                }
            }

            // Pad remaining space
            queue!(
                self.stdout,
                SetBackgroundColor(finder_bg),
                SetForegroundColor(finder_fg)
            )?;
            for _ in chars.len()..content_width {
                write!(self.stdout, " ")?;
            }
        } else {
            // Beyond content - empty row
            for _ in 0..preview_width {
                write!(self.stdout, " ")?;
            }
        }

        queue!(
            self.stdout,
            SetForegroundColor(finder_fg),
            SetBackgroundColor(finder_bg)
        )?;
        Ok(())
    }

    /// Get syntax highlights for a preview line
    fn get_preview_highlights(&self, editor: &Editor, line_idx: usize) -> Vec<HighlightSpan> {
        // Use the preview syntax manager from editor if available
        editor.preview_syntax.get_line_highlights(line_idx)
    }

    /// Render a line with syntax highlighting and optional visual selection
    /// Now accepts theme colors to maintain proper background
    fn render_line_with_highlights(
        &mut self,
        line: &str,
        line_idx: usize,
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
        diagnostic_hint_color: Color,
        tab_width: usize,
    ) -> anyhow::Result<usize> {
        let chars: Vec<char> = line.chars().collect();
        let line_len = chars.len();

        // Determine selection range for this line based on visual mode
        let (in_selection, sel_start, sel_end) =
            if let Some((range_start_line, range_start_col, range_end_line, range_end_col)) =
                visual_range
            {
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
                            let start = if line_idx == range_start_line {
                                range_start_col
                            } else {
                                0
                            };
                            let end = if line_idx == range_end_line {
                                range_end_col + 1
                            } else {
                                line_len
                            };
                            (true, start, end)
                        }
                        _ => (false, 0, 0),
                    }
                }
            } else {
                (false, 0, 0)
            };

        // Determine the base background for this line (cursor line or editor background)
        let base_bg = if is_cursor_line {
            cursor_line_bg
        } else {
            editor_bg
        };

        // Check if a column is within a search match for this line
        let in_search_match = |actual_col: usize| -> bool {
            search_matches
                .iter()
                .any(|(l, start, end)| *l == line_idx && actual_col >= *start && actual_col < *end)
        };

        // Render character by character
        let mut highlight_idx = 0;
        let mut current_fg: Option<Color> = None;
        let mut current_bg: Option<Color> = None;
        let mut current_bold = false;
        let mut current_italic = false;
        let mut current_underline_color: Option<Color> = None;
        let mut rendered_cols = 0;
        for (i, ch) in chars.iter().enumerate() {
            // Map display column to actual buffer column
            let actual_col = col_offset + i;

            // Find syntax color for this column
            let syntax_style =
                Self::get_syntax_style_at(highlights, actual_col, &mut highlight_idx);
            let syntax_color = syntax_style.map(|style| style.fg);

            // Check if in visual selection
            let is_selected = in_selection && actual_col >= sel_start && actual_col < sel_end;

            // Check if in search match
            let is_search_match = in_search_match(actual_col);

            let diag_at_col = diagnostic_at_col(diagnostics, line_idx, actual_col);

            // Check if within a hint diagnostic (unused variable/import) - grey out the text
            let is_hint_diagnostic =
                diag_at_col.map_or(false, |d| d.severity == DiagnosticSeverity::Hint);

            // Priority: visual selection > search match > hint (grey out) > base
            let (desired_bg, desired_fg) = if is_selected {
                (selection_bg, syntax_color.unwrap_or(editor_fg))
            } else if is_search_match {
                (search_match_bg, search_match_fg)
            } else if is_hint_diagnostic {
                // Grey out unused code (like Neovim does)
                (base_bg, diagnostic_hint_color)
            } else {
                (base_bg, syntax_color.unwrap_or(editor_fg))
            };
            let desired_style = if is_selected || (!is_search_match && !is_hint_diagnostic) {
                syntax_style
            } else {
                None
            };
            let desired_bold = desired_style.map_or(false, |style| style.bold);
            let desired_italic = desired_style.map_or(false, |style| style.italic);
            let desired_underline_color = if is_selected || is_search_match {
                None
            } else {
                diag_at_col
                    .and_then(|diag| diagnostic_underline_color(diag, diagnostic_error_color))
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
            if desired_bold != current_bold {
                execute!(
                    self.stdout,
                    SetAttribute(if desired_bold {
                        Attribute::Bold
                    } else {
                        Attribute::NoBold
                    })
                )?;
                current_bold = desired_bold;
            }
            if desired_italic != current_italic {
                execute!(
                    self.stdout,
                    SetAttribute(if desired_italic {
                        Attribute::Italic
                    } else {
                        Attribute::NoItalic
                    })
                )?;
                current_italic = desired_italic;
            }
            if desired_underline_color != current_underline_color {
                apply_diagnostic_underline(&mut self.stdout, desired_underline_color, desired_fg)?;
                current_underline_color = desired_underline_color;
            }

            rendered_cols += print_editor_char(*ch, tab_width);
        }

        // Handle selection extending past line end
        if in_selection && sel_end > line_len {
            execute!(self.stdout, SetBackgroundColor(selection_bg))?;
            print!(" ");
            rendered_cols += 1;
        }

        // Restore to base background/foreground and clear text attributes.
        execute!(
            self.stdout,
            SetAttribute(Attribute::Reset),
            SetBackgroundColor(base_bg),
            SetForegroundColor(editor_fg)
        )?;

        Ok(rendered_cols)
    }

    /// Get the syntax color at a given column position
    fn get_syntax_color_at(
        highlights: &[HighlightSpan],
        col: usize,
        hint_idx: &mut usize,
    ) -> Option<Color> {
        Self::get_syntax_style_at(highlights, col, hint_idx).map(|style| style.fg)
    }

    /// Get the syntax style at a given column position
    fn get_syntax_style_at(
        highlights: &[HighlightSpan],
        col: usize,
        hint_idx: &mut usize,
    ) -> Option<SyntaxStyle> {
        // Start searching from hint_idx for efficiency
        while *hint_idx < highlights.len() {
            let span = &highlights[*hint_idx];
            if col < span.start_col {
                // Not yet at this span
                return None;
            } else if col < span.end_col {
                // Inside this span
                return Some(span.style);
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
            Event::Paste(text) => Ok(Some(EditorEvent::Paste(text))),
            Event::Mouse(mouse_event) => Ok(Some(EditorEvent::Mouse(mouse_event))),
            Event::FocusGained => Ok(Some(EditorEvent::FocusGained)),
            Event::Resize(cols, rows) => Ok(Some(EditorEvent::Resize(cols, rows))),
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
            event::DisableMouseCapture,
            event::DisableFocusChange,
            DisableBracketedPaste,
            PopKeyboardEnhancementFlags,
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

/// Handle key input for the theme picker
fn handle_theme_picker_key(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Close picker (cancel)
        (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
            editor.close_theme_picker(false); // Cancel
        }

        // Navigate up (Ctrl-k or Ctrl-p, since j/k are now for typing)
        (KeyModifiers::NONE, KeyCode::Up)
        | (KeyModifiers::CONTROL, KeyCode::Char('k'))
        | (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
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
        (KeyModifiers::NONE, KeyCode::Down)
        | (KeyModifiers::CONTROL, KeyCode::Char('j'))
        | (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
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
                        Err(e) => editor
                            .set_status(format!("Theme set to '{}' (save failed: {})", name, e)),
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
        (KeyModifiers::NONE, KeyCode::Char(c)) | (KeyModifiers::SHIFT, KeyCode::Char(c)) => {
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

fn handle_markdown_preview_key(editor: &mut Editor, key: KeyEvent) {
    let visible_rows = Terminal::markdown_preview_visible_rows(editor).max(1);
    let half_page = (visible_rows / 2).max(1) as isize;

    match (key.modifiers, key.code) {
        (KeyModifiers::NONE, KeyCode::Esc)
        | (KeyModifiers::NONE, KeyCode::Char('q'))
        | (KeyModifiers::CONTROL, KeyCode::Char('[')) => editor.close_markdown_preview(),
        (KeyModifiers::NONE, KeyCode::Char('j')) | (KeyModifiers::NONE, KeyCode::Down) => {
            editor.scroll_markdown_preview(1, visible_rows)
        }
        (KeyModifiers::NONE, KeyCode::Char('k')) | (KeyModifiers::NONE, KeyCode::Up) => {
            editor.scroll_markdown_preview(-1, visible_rows)
        }
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            editor.scroll_markdown_preview(half_page, visible_rows)
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            editor.scroll_markdown_preview(-half_page, visible_rows)
        }
        (KeyModifiers::NONE, KeyCode::Char('g')) => editor.jump_markdown_preview_to_top(),
        (KeyModifiers::SHIFT, KeyCode::Char('G')) | (KeyModifiers::NONE, KeyCode::Char('G')) => {
            editor.jump_markdown_preview_to_bottom(visible_rows)
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
    editor
        .undo_stack
        .begin_compound_group(editor.cursor.line, editor.cursor.col);

    // Play the macro `count` times
    for _ in 0..count {
        for key in &keys {
            // Process each key - note: we DON'T record during playback
            // because is_recording() will be false
            handle_key(editor, *key);
        }
    }

    editor
        .undo_stack
        .end_compound_group(editor.cursor.line, editor.cursor.col);
}

/// Handle a key event and update editor state
pub fn handle_key(editor: &mut Editor, key: KeyEvent) {
    // Check for floating terminal toggle (Ctrl-\) - works in any mode
    // Note: Ctrl-\ sends ASCII 28 (File Separator) on Unix terminals
    // We check for both the character and the raw control code
    let is_ctrl_backslash = match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('\\')) => true,
        (KeyModifiers::CONTROL, KeyCode::Char('4')) => true, // Ctrl-4 = Ctrl-\ on some terminals
        (_, KeyCode::Char('\x1c')) => true,                  // ASCII 28 = File Separator (Ctrl-\)
        _ => false,
    };
    if is_ctrl_backslash {
        editor.floating_terminal.toggle();
        return;
    }

    // If floating terminal is visible, handle its keys
    if editor.floating_terminal.is_visible() {
        if editor
            .floating_terminal
            .handle_session_control_key(key, &editor.settings.terminal)
        {
            return;
        }

        // Terminal apps need Escape; Ctrl-\ is the dedicated toggle key.
        editor.floating_terminal.send_key(key);
        return;
    }

    // If Markdown preview is visible, it owns input until closed.
    if editor.markdown_preview.is_some() {
        handle_markdown_preview_key(editor, key);
        return;
    }

    if editor.pending_expression_register.is_some() {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
                editor.cancel_expression_register();
            }
            (KeyModifiers::NONE, KeyCode::Enter) => {
                editor.submit_expression_register();
            }
            (KeyModifiers::NONE, KeyCode::Backspace) => {
                editor.pop_expression_register_char();
            }
            (_, KeyCode::Char(ch)) if !ch.is_control() => {
                editor.push_expression_register_char(ch);
            }
            _ => {}
        }
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

    if editor.pending_insert_normal_once && editor.mode == Mode::Normal {
        handle_normal_mode(editor, key);
        if !editor.input_state.has_pending_sequence() {
            if editor.mode == Mode::Normal {
                editor.finish_insert_normal_once();
            } else {
                editor.pending_insert_normal_once = false;
            }
        }
        return;
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
    // But skip if we're in a partial sequence (like g; or g,) - let the input state handle it
    if editor.input_state.partial_key.is_none() && !editor.input_state.pending_window_cmd {
        if let Some(mapping) = editor.keymap.get_normal_mapping(key) {
            let mapping = mapping.clone();
            let t_exec = std::time::Instant::now();
            execute_leader_action(editor, &mapping);
            let total = t_start.elapsed();
            if total.as_micros() > 1000 {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open("/tmp/nevi_debug.log")
                {
                    let _ = writeln!(
                        f,
                        "SLOW custom_mapping: total={:?} exec={:?} key={:?}",
                        total,
                        t_exec.elapsed(),
                        key.code
                    );
                }
            }
            return;
        }
    }

    let register_before_action = editor.input_state.selected_register;
    let t_process = std::time::Instant::now();
    let action = editor.input_state.process_normal_key(key);
    let process_elapsed = t_process.elapsed();

    let t_action = std::time::Instant::now();
    match action {
        KeyAction::Pending => {
            // Key was consumed, waiting for more input
            if editor.input_state.selected_register == Some('=') {
                editor.input_state.selected_register = None;
                editor.start_expression_register(ExpressionRegisterTarget::Normal);
            }
        }

        KeyAction::Motion(motion, count) => {
            editor.apply_motion(motion, count);
            // Clear search highlights on non-search movement (like Neovim)
            editor.clear_search_highlights();
        }

        KeyAction::OperatorMotion(op, motion, count) => {
            let register = editor
                .input_state
                .take_register()
                .or(register_before_action);
            match op {
                Operator::Delete => editor.delete_motion(motion, count, register),
                Operator::Change => editor.change_motion(motion, count, register),
                Operator::Yank => editor.yank_motion(motion, count, register),
                Operator::Indent => editor.indent_motion(motion, count),
                Operator::Dedent => editor.dedent_motion(motion, count),
                Operator::AutoIndent => editor.auto_indent_motion(motion, count),
            }
        }

        KeyAction::OperatorLine(op, count) => {
            let register = editor
                .input_state
                .take_register()
                .or(register_before_action);
            match op {
                Operator::Delete => editor.delete_line(count, register),
                Operator::Change => editor.change_line(count, register),
                Operator::Yank => editor.yank_line(count, register),
                Operator::Indent => editor.indent_line(count),
                Operator::Dedent => editor.dedent_line(count),
                Operator::AutoIndent => editor.auto_indent_line(count),
            }
        }

        KeyAction::OperatorTextObject(op, text_object) => {
            let register = editor
                .input_state
                .take_register()
                .or(register_before_action);
            match op {
                Operator::Delete => editor.delete_text_object(text_object, register),
                Operator::Change => editor.change_text_object(text_object, register),
                Operator::Yank => editor.yank_text_object(text_object, register),
                Operator::Indent => editor.indent_text_object(text_object),
                Operator::Dedent => editor.dedent_text_object(text_object),
                Operator::AutoIndent => editor.auto_indent_text_object(text_object),
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
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open("/tmp/nevi_debug.log")
                {
                    let _ = writeln!(
                        f,
                        "SLOW EnterInsert: total={:?} leader_check={:?} process={:?} action={:?}",
                        total, leader_check_elapsed, process_elapsed, action_elapsed
                    );
                }
            }
        }

        KeyAction::DeleteChar(count) => {
            editor.delete_chars_at(count);
        }

        KeyAction::DeleteCharBefore(count) => {
            editor.delete_chars_before_normal(count);
        }

        KeyAction::SubstituteChars(count) => {
            let register = editor
                .input_state
                .take_register()
                .or(register_before_action);
            editor.substitute_chars_at(count, register);
        }

        KeyAction::PasteAfter(count) => {
            let register = editor
                .input_state
                .take_register()
                .or(register_before_action);
            editor.paste_after_count(register, count);
        }

        KeyAction::PasteBefore(count) => {
            let register = editor
                .input_state
                .take_register()
                .or(register_before_action);
            editor.paste_before_count(register, count);
        }

        KeyAction::PasteAfterMove(count) => {
            let register = editor
                .input_state
                .take_register()
                .or(register_before_action);
            editor.paste_after_move_count(register, count);
        }

        KeyAction::PasteBeforeMove(count) => {
            let register = editor
                .input_state
                .take_register()
                .or(register_before_action);
            editor.paste_before_move_count(register, count);
        }

        KeyAction::Undo => {
            editor.undo();
        }

        KeyAction::Redo => {
            editor.redo();
        }

        KeyAction::ReplaceChar(c, count) => {
            editor.replace_chars(c, count);
        }

        KeyAction::ToggleCaseChars(count) => {
            editor.toggle_case_chars(count);
        }

        KeyAction::JoinLines(count) => {
            editor.join_lines_count(count);
        }

        KeyAction::JoinLinesNoSpace(count) => {
            editor.join_lines_no_space_count(count);
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

        KeyAction::SearchSelectNext(count) => {
            editor.search_select_next(count);
        }

        KeyAction::SearchSelectPrev(count) => {
            editor.search_select_prev(count);
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

        KeyAction::WindowEqualize => {
            editor.equalize_windows();
        }

        KeyAction::WindowRotateDownRight => {
            editor.rotate_windows_down_right();
        }

        KeyAction::WindowRotateUpLeft => {
            editor.rotate_windows_up_left();
        }

        KeyAction::WindowExchangeNext => {
            editor.exchange_window_with_next();
        }

        KeyAction::WindowMoveFarLeft => {
            editor.move_window_far_left();
        }

        KeyAction::WindowMoveFarRight => {
            editor.move_window_far_right();
        }

        KeyAction::WindowMoveTop => {
            editor.move_window_top();
        }

        KeyAction::WindowMoveBottom => {
            editor.move_window_bottom();
        }

        KeyAction::WindowIncreaseHeight => {
            editor.increase_window_height();
        }

        KeyAction::WindowDecreaseHeight => {
            editor.decrease_window_height();
        }

        KeyAction::WindowIncreaseWidth => {
            editor.increase_window_width();
        }

        KeyAction::WindowDecreaseWidth => {
            editor.decrease_window_width();
        }

        KeyAction::WindowMaximizeHeight => {
            editor.maximize_window_height();
        }

        KeyAction::WindowMaximizeWidth => {
            editor.maximize_window_width();
        }

        KeyAction::GotoDefinition => {
            editor.pending_lsp_action = Some(crate::editor::LspAction::GotoDefinition);
        }

        KeyAction::GotoDeclaration => {
            editor.pending_lsp_action = Some(crate::editor::LspAction::GotoDeclaration);
        }

        KeyAction::GotoImplementation => {
            editor.pending_lsp_action = Some(crate::editor::LspAction::GotoImplementation);
        }

        KeyAction::GotoFile => match editor.open_file_under_cursor() {
            Ok(path) => editor.set_status(format!("Opened {}", path.display())),
            Err(err) => editor.set_status(err),
        },

        KeyAction::OpenUrl => match editor.open_url_under_cursor() {
            Ok(url) => editor.set_status(format!("Opened {}", url)),
            Err(err) => editor.set_status(err),
        },

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

        KeyAction::JumpToPreviousPosition => {
            if !editor.jump_to_previous_position_line() {
                editor.set_status("No previous jump position");
            }
        }

        KeyAction::JumpToPreviousPositionExact => {
            if !editor.jump_to_previous_position_exact() {
                editor.set_status("No previous jump position");
            }
        }

        KeyAction::JumpToLastChange => {
            if !editor.jump_to_last_change_line() {
                editor.set_status("No last change position");
            }
        }

        KeyAction::JumpToLastChangeExact => {
            if !editor.jump_to_last_change_exact() {
                editor.set_status("No last change position");
            }
        }

        KeyAction::JumpToLastInsert => {
            if !editor.jump_to_last_insert_line() {
                editor.set_status("No previous insert position");
            }
        }

        KeyAction::JumpToLastInsertExact => {
            if !editor.jump_to_last_insert_exact() {
                editor.set_status("No previous insert position");
            }
        }

        KeyAction::ChangeListOlder => {
            if !editor.change_list_older() {
                editor.set_status("Already at oldest change");
            }
        }

        KeyAction::ChangeListNewer => {
            if !editor.change_list_newer() {
                editor.set_status("Already at newest change");
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

        KeyAction::AddSurroundMotion(motion, count, surround_char) => {
            editor.add_surrounding_motion(motion, count, surround_char);
        }

        KeyAction::AddSurroundLine(surround_char) => {
            editor.add_surrounding_line(surround_char);
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
            )
            .unwrap_or((start_line, 0));

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
            editor.open_finder_harpoon();
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

fn completion_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn completion_word_suffix_len(editor: &Editor) -> usize {
    let Some(line) = editor.buffer().line(editor.cursor.line) else {
        return 0;
    };
    let chars: Vec<char> = line.chars().collect();
    let mut col = editor.cursor.col;
    let mut len = 0;
    while col < chars.len() && completion_word_char(chars[col]) {
        len += 1;
        col += 1;
    }
    len
}

fn replace_completion_text(editor: &mut Editor, trigger_col: usize, text: &str) {
    let chars_after_cursor = completion_word_suffix_len(editor);
    for _ in 0..chars_after_cursor {
        editor.delete_char_at();
    }

    let chars_before_cursor = editor.cursor.col.saturating_sub(trigger_col);
    for _ in 0..chars_before_cursor {
        editor.delete_char_before();
    }

    for ch in text.chars() {
        editor.insert_char(ch);
    }
}

fn handle_insert_mode(editor: &mut Editor, key: KeyEvent) {
    let t_insert_start = std::time::Instant::now();

    // Apply custom keymap remapping for insert mode
    let key = editor.keymap.remap_insert(key);

    if editor.pending_insert_register {
        editor.pending_insert_register = false;
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {}
            (_, KeyCode::Char(register)) => {
                if register == '=' {
                    editor.start_expression_register(ExpressionRegisterTarget::Insert);
                } else {
                    editor.insert_register_text(register);
                }
            }
            _ => {}
        }
        return;
    }

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
                let completion_info = editor.completion.selected_item().cloned();

                if let Some(item) = completion_info {
                    // Record frecency usage
                    editor.record_completion_use(&item.label);

                    let text =
                        if let Some(inserted_text) = editor.apply_completion_item_edits(&item) {
                            inserted_text
                        } else {
                            let text = item
                                .insert_text
                                .as_deref()
                                .unwrap_or(&item.label)
                                .to_string();
                            replace_completion_text(editor, editor.completion.trigger_col, &text);
                            text
                        };

                    // Auto-brackets: add () for functions/methods and position cursor inside
                    let needs_brackets = matches!(
                        item.kind,
                        CompletionKind::Function
                            | CompletionKind::Method
                            | CompletionKind::Constructor
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
            (_, KeyCode::Char(c))
                if matches!(c, ' ' | ';' | '(' | ')' | '{' | '}' | '[' | ']' | ',' | ':') =>
            {
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

        // Increase indentation of current line (Ctrl+t)
        (KeyModifiers::CONTROL, KeyCode::Char('t')) => {
            editor.indent_current_line_in_insert_mode();
        }

        // Decrease indentation of current line (Ctrl+d)
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            editor.dedent_current_line_in_insert_mode();
        }

        // Insert previously inserted text (Ctrl+a)
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            editor.insert_last_inserted_text();
        }

        // Insert the contents of a register (Ctrl+r {register})
        (KeyModifiers::CONTROL, KeyCode::Char('r')) => {
            editor.pending_insert_register = true;
        }

        // Execute one normal-mode command, then return to insert mode (Ctrl+o)
        (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
            editor.enter_insert_normal_once();
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
                            ('(', ')')
                                | ('[', ']')
                                | ('{', '}')
                                | ('"', '"')
                                | ('\'', '\'')
                                | ('`', '`')
                        );
                        if is_matching_pair {
                            // Delete both characters
                            editor.delete_char_before(); // Delete opening
                            editor.delete_char_at_in_current_group(); // Delete closing (now at cursor)
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
                let next_char = editor
                    .buffer()
                    .char_at(editor.cursor.line, editor.cursor.col);
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
                    let prefix: String = line_str
                        .chars()
                        .skip(trigger_col)
                        .take(col - trigger_col)
                        .collect();

                    // TypeScript auto-import and other server-ranked completions can change as
                    // the prefix becomes more specific. Ask LSP for a fresh list instead of only
                    // fuzzy-filtering stale broad results locally.
                    if prefix != editor.completion.filter_text {
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
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open("/tmp/nevi_debug.log")
        {
            let _ = writeln!(
                f,
                "SLOW insert_mode: {:?} key={:?}",
                insert_elapsed, key.code
            );
        }
    }
}

fn handle_replace_mode(editor: &mut Editor, key: KeyEvent) {
    match (key.modifiers, key.code) {
        // Exit replace mode
        (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
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
        (KeyModifiers::NONE, KeyCode::Esc)
        | (KeyModifiers::CONTROL, KeyCode::Char('['))
        | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
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

fn execute_command_mode_action(editor: &mut Editor, action: CommandModeAction) {
    match action {
        CommandModeAction::HistoryToggle => {
            editor.command_line.toggle_history_popup();
        }
        CommandModeAction::InsertRegister => {
            editor.command_line.pending_register = true;
        }
        CommandModeAction::InsertLiteral => {
            editor.command_line.pending_literal = true;
        }
        CommandModeAction::InsertDigraph => {
            editor.command_line.pending_digraph = PendingDigraph::First;
        }
        CommandModeAction::ListCompletions => {
            editor.command_line.list_completions();
        }
        CommandModeAction::CompleteLongestCommonPrefix => {
            editor.command_line.complete_longest_common_prefix();
        }
        CommandModeAction::InsertAllCompletions => {
            editor.command_line.insert_all_matching_completions();
        }
        CommandModeAction::OpenCommandLineWindow => {
            editor.command_line.open_command_line_window();
        }
        CommandModeAction::Complete => {
            if editor.command_line.popup_mode == CommandPopupMode::History {
                editor.command_line.accept_history_popup_selection();
            } else {
                editor.command_line.accept_completion_selection();
            }
        }
        CommandModeAction::CompletePrev => {
            if editor.command_line.popup_mode == CommandPopupMode::History {
                editor.command_line.popup_prev();
            } else {
                editor.command_line.popup_prev();
                editor.command_line.accept_completion_selection();
            }
        }
        CommandModeAction::PopupNext => {
            editor.command_line.popup_next();
        }
        CommandModeAction::PopupPrev => {
            editor.command_line.popup_prev();
        }
    }
}

fn handle_command_mode(editor: &mut Editor, key: KeyEvent) {
    if editor.command_line.pending_digraph != PendingDigraph::None {
        handle_command_digraph_key(editor, key);
        return;
    }

    if editor.command_line.pending_literal {
        editor.command_line.pending_literal = false;
        if let Some(ch) = command_literal_char(key) {
            editor.command_line.insert_char(ch);
        }
        return;
    }

    if editor.command_line.pending_register {
        editor.command_line.pending_register = false;
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {}
            (_, KeyCode::Char(register)) => {
                editor.insert_register_text_into_command_line(register);
            }
            _ => {}
        }
        return;
    }

    if let Some(action) = editor.keymap.get_command_action(key) {
        execute_command_mode_action(editor, action);
        return;
    }

    match (key.modifiers, key.code) {
        // Cancel command
        (KeyModifiers::NONE, KeyCode::Esc)
        | (KeyModifiers::CONTROL, KeyCode::Char('['))
        | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            editor.exit_command_mode();
        }

        // Execute command (or accept history selection when history window is open)
        (KeyModifiers::NONE, KeyCode::Enter) => {
            if editor.command_line.popup_mode == CommandPopupMode::History {
                editor.command_line.accept_history_popup_selection();
            } else {
                let cmd = editor.command_line.execute();
                editor.mode = Mode::Normal;
                execute_command(editor, cmd);
            }
        }

        // Backspace
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            if editor.command_line.input.is_empty() {
                if editor.command_line.popup_mode == CommandPopupMode::History {
                    editor.command_line.toggle_history_popup();
                } else {
                    editor.exit_command_mode();
                }
            } else {
                editor.command_line.delete_char_before();
            }
        }

        // Delete
        (KeyModifiers::NONE, KeyCode::Delete) => {
            editor.command_line.delete_char_at();
        }

        // Delete word before cursor
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            editor.command_line.delete_word_before();
        }

        // Cursor movement
        (KeyModifiers::NONE, KeyCode::Left) => {
            editor.command_line.move_left();
        }
        (KeyModifiers::NONE, KeyCode::Right) => {
            editor.command_line.move_right();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('a'))
        | (KeyModifiers::CONTROL, KeyCode::Char('b'))
        | (KeyModifiers::NONE, KeyCode::Home) => {
            editor.command_line.move_to_start();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) | (KeyModifiers::NONE, KeyCode::End) => {
            editor.command_line.move_to_end();
        }

        // History navigation
        (KeyModifiers::NONE, KeyCode::Up) => {
            if editor.command_line.popup_mode == CommandPopupMode::History {
                editor.command_line.popup_prev();
            } else {
                editor.command_line.history_prev();
            }
        }
        (KeyModifiers::NONE, KeyCode::Down) => {
            if editor.command_line.popup_mode == CommandPopupMode::History {
                editor.command_line.popup_next();
            } else {
                editor.command_line.history_next();
            }
        }

        // Delete from cursor to start
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            editor.command_line.delete_to_start();
        }

        // Regular character - accept any modifier for printable chars
        (_, KeyCode::Char(c)) if !c.is_control() => {
            editor.command_line.insert_char(c);
        }

        _ => {}
    }
}

fn handle_command_digraph_key(editor: &mut Editor, key: KeyEvent) {
    match editor.command_line.pending_digraph {
        PendingDigraph::None => {}
        PendingDigraph::First => {
            if let Some(first) = command_digraph_char(key) {
                editor.command_line.pending_digraph = PendingDigraph::Second(first);
            } else {
                editor.command_line.pending_digraph = PendingDigraph::None;
            }
        }
        PendingDigraph::Second(first) => {
            editor.command_line.pending_digraph = PendingDigraph::None;
            let Some(second) = command_digraph_char(key) else {
                return;
            };
            if let Some(ch) = vim_latin1_digraph(first, second) {
                editor.command_line.insert_char(ch);
            } else {
                editor.set_status(format!("Unknown digraph: {first}{second}"));
            }
        }
    }
}

fn command_digraph_char(key: KeyEvent) -> Option<char> {
    match (key.modifiers, key.code) {
        (KeyModifiers::NONE, KeyCode::Char(ch))
        | (KeyModifiers::SHIFT, KeyCode::Char(ch))
        | (KeyModifiers::ALT, KeyCode::Char(ch)) => Some(ch),
        _ => None,
    }
}

fn command_literal_char(key: KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            control_literal_char(c).or(Some(c))
        }
        KeyCode::Char(c) => Some(c),
        KeyCode::Enter => Some('\r'),
        KeyCode::Tab => Some('\t'),
        KeyCode::BackTab => Some('\t'),
        KeyCode::Backspace => Some('\u{8}'),
        KeyCode::Esc => Some('\u{1b}'),
        KeyCode::Delete => Some('\u{7f}'),
        _ => None,
    }
}

fn control_literal_char(ch: char) -> Option<char> {
    let byte = if ch.is_ascii_alphabetic() {
        ch.to_ascii_uppercase() as u8
    } else if ch.is_ascii() {
        ch as u8
    } else {
        return None;
    };

    let code = match byte {
        b' ' | b'@' => 0x00,
        b'A'..=b'Z' => byte - b'@',
        b'[' => 0x1b,
        b'\\' => 0x1c,
        b']' => 0x1d,
        b'^' => 0x1e,
        b'_' => 0x1f,
        b'?' => 0x7f,
        _ => return None,
    };
    char::from_u32(code as u32)
}

fn vim_latin1_digraph(first: char, second: char) -> Option<char> {
    match (first, second) {
        ('A', '!') => Some('À'),
        ('A', '\'') => Some('Á'),
        ('A', '>') => Some('Â'),
        ('A', '?') => Some('Ã'),
        ('A', ':') => Some('Ä'),
        ('A', 'A') => Some('Å'),
        ('A', 'E') => Some('Æ'),
        ('C', ',') => Some('Ç'),
        ('E', '!') => Some('È'),
        ('E', '\'') => Some('É'),
        ('E', '>') => Some('Ê'),
        ('E', ':') => Some('Ë'),
        ('I', '!') => Some('Ì'),
        ('I', '\'') => Some('Í'),
        ('I', '>') => Some('Î'),
        ('I', ':') => Some('Ï'),
        ('N', '?') => Some('Ñ'),
        ('O', '!') => Some('Ò'),
        ('O', '\'') => Some('Ó'),
        ('O', '>') => Some('Ô'),
        ('O', '?') => Some('Õ'),
        ('O', ':') => Some('Ö'),
        ('O', '/') => Some('Ø'),
        ('U', '!') => Some('Ù'),
        ('U', '\'') => Some('Ú'),
        ('U', '>') => Some('Û'),
        ('U', ':') => Some('Ü'),
        ('Y', '\'') => Some('Ý'),
        ('T', 'H') => Some('Þ'),
        ('s', 's') => Some('ß'),
        ('a', '!') => Some('à'),
        ('a', '\'') => Some('á'),
        ('a', '>') => Some('â'),
        ('a', '?') => Some('ã'),
        ('a', ':') => Some('ä'),
        ('a', 'a') => Some('å'),
        ('a', 'e') => Some('æ'),
        ('c', ',') => Some('ç'),
        ('e', '!') => Some('è'),
        ('e', '\'') => Some('é'),
        ('e', '>') => Some('ê'),
        ('e', ':') => Some('ë'),
        ('i', '!') => Some('ì'),
        ('i', '\'') => Some('í'),
        ('i', '>') => Some('î'),
        ('i', ':') => Some('ï'),
        ('n', '?') => Some('ñ'),
        ('o', '!') => Some('ò'),
        ('o', '\'') => Some('ó'),
        ('o', '>') => Some('ô'),
        ('o', '?') => Some('õ'),
        ('o', ':') => Some('ö'),
        ('o', '/') => Some('ø'),
        ('u', '!') => Some('ù'),
        ('u', '\'') => Some('ú'),
        ('u', '>') => Some('û'),
        ('u', ':') => Some('ü'),
        ('y', '\'') => Some('ý'),
        ('t', 'h') => Some('þ'),
        ('y', ':') => Some('ÿ'),
        ('<', '<') => Some('«'),
        ('>', '>') => Some('»'),
        ('!', 'I') => Some('¡'),
        ('?', 'I') => Some('¿'),
        ('C', 'o') => Some('©'),
        ('R', 'g') => Some('®'),
        ('+', '-') => Some('±'),
        ('D', 'G') => Some('°'),
        ('*', 'X') => Some('×'),
        ('-', ':') => Some('÷'),
        ('1', '2') => Some('½'),
        ('1', '4') => Some('¼'),
        ('3', '4') => Some('¾'),
        _ => None,
    }
}

fn handle_search_mode(editor: &mut Editor, key: KeyEvent) {
    if editor.search.pending_register {
        editor.search.pending_register = false;
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {}
            (_, KeyCode::Char(register)) => {
                if editor.insert_register_text_into_search_prompt(register) {
                    editor.update_incremental_search();
                }
            }
            _ => {}
        }
        return;
    }

    match (key.modifiers, key.code) {
        // Cancel search
        (KeyModifiers::NONE, KeyCode::Esc)
        | (KeyModifiers::CONTROL, KeyCode::Char('['))
        | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
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
        (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
            editor.search.move_to_start();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
            editor.search.move_to_end();
        }
        (KeyModifiers::NONE, KeyCode::Up) => {
            editor.search.history_prev();
            editor.update_incremental_search();
        }
        (KeyModifiers::NONE, KeyCode::Down) => {
            editor.search.history_next();
            editor.update_incremental_search();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            editor.search.delete_word_before();
            editor.update_incremental_search();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            editor.search.delete_to_start();
            editor.update_incremental_search();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('r')) => {
            editor.search.pending_register = true;
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

    if editor.input_state.pending_visual_surround {
        editor.input_state.pending_visual_surround = false;
        if let KeyCode::Char(c) = key.code {
            editor.surround_visual_selection(c);
        }
        return;
    }

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
            (KeyModifiers::NONE, KeyCode::Char('p')) => Some(TextObjectType::Paragraph),
            (KeyModifiers::NONE, KeyCode::Char('s')) => Some(TextObjectType::Sentence),
            (KeyModifiers::NONE, KeyCode::Char('t')) => Some(TextObjectType::Tag),
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

    if let Some(mapping) = editor.keymap.get_visual_mapping(key).cloned() {
        execute_visual_keymap_action(editor, &mapping);
        return;
    }

    match (key.modifiers, key.code) {
        // Exit visual mode
        (KeyModifiers::NONE, KeyCode::Esc)
        | (KeyModifiers::CONTROL, KeyCode::Char('['))
        | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
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
        (KeyModifiers::NONE, KeyCode::Char('d')) | (KeyModifiers::NONE, KeyCode::Char('x')) => {
            editor.visual_delete();
        }
        (KeyModifiers::NONE, KeyCode::Char('c')) | (KeyModifiers::NONE, KeyCode::Char('s')) => {
            editor.visual_change();
        }
        (KeyModifiers::NONE, KeyCode::Char('y')) => {
            editor.visual_yank();
        }
        (KeyModifiers::NONE, KeyCode::Char('p')) => {
            let register = editor.input_state.take_register();
            editor.visual_paste(register);
        }
        (KeyModifiers::SHIFT, KeyCode::Char('I')) if editor.mode == Mode::VisualBlock => {
            editor.enter_visual_block_insert_mode(false);
        }
        (KeyModifiers::SHIFT, KeyCode::Char('A')) if editor.mode == Mode::VisualBlock => {
            editor.enter_visual_block_insert_mode(true);
        }
        (KeyModifiers::SHIFT, KeyCode::Char('S')) => {
            editor.input_state.pending_visual_surround = true;
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
        (KeyModifiers::SHIFT, KeyCode::Char('O')) if editor.mode == Mode::VisualBlock => {
            let old_anchor_col = editor.visual.anchor_col;
            editor.visual.anchor_col = editor.cursor.col;
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

fn execute_visual_keymap_action(editor: &mut Editor, action: &LeaderAction) {
    match action {
        LeaderAction::Command(cmd_str) => {
            let cmd = parse_command(cmd_str);
            execute_command(editor, cmd);
        }
        LeaderAction::Keys(keys) => {
            for key in keys {
                if !editor.mode.is_visual() {
                    break;
                }
                handle_visual_mode(editor, *key);
            }
        }
    }
}

fn finder_preview_scroll_amount(editor: &Editor) -> usize {
    let preview_enabled = editor.finder.preview_enabled && editor.finder.mode_supports_preview();
    let win = crate::finder::FloatingWindow::centered_with_preview(
        editor.term_width,
        editor.term_height,
        preview_enabled,
    );
    let visible_rows = win.height.saturating_sub(4) as usize;
    (visible_rows / 2).max(1)
}

fn handle_finder_mode(editor: &mut Editor, key: KeyEvent) {
    let t_start = Instant::now();

    // Check if we're in normal mode for vim-like navigation
    let is_normal_mode = editor.finder.is_normal_mode();

    // Track if selection might have changed (for preview update)
    let mut selection_changed = false;

    // Helper to adjust scroll after navigation
    let adjust_scroll = |editor: &mut Editor| {
        let preview_enabled =
            editor.finder.preview_enabled && editor.finder.mode_supports_preview();
        let win = crate::finder::FloatingWindow::centered_with_preview(
            editor.term_width,
            editor.term_height,
            preview_enabled,
        );
        let list_height = (win.height - 4) as usize;
        editor.finder.adjust_scroll(list_height);
    };

    match (key.modifiers, key.code) {
        // Toggle preview panel - Ctrl+t (works in both modes)
        (KeyModifiers::CONTROL, KeyCode::Char('t')) => {
            editor.finder.toggle_preview();
            if editor.finder.preview_enabled && editor.finder.mode_supports_preview() {
                // Mark preview as needing immediate update (skip debounce on toggle)
                editor.update_finder_preview();
            }
        }

        // Scroll preview panel down - Ctrl+d (works in both modes)
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            let amount = finder_preview_scroll_amount(editor);
            editor.finder.scroll_preview_down(amount);
        }

        // Scroll preview panel up - Ctrl+u (works in both modes)
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            let amount = finder_preview_scroll_amount(editor);
            editor.finder.scroll_preview_up(amount);
        }

        // Cancel finder - Ctrl+c always closes
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            editor.close_finder();
        }

        // Esc behavior depends on mode
        (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
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
            if editor.finder.mode == crate::finder::FinderMode::Keymaps {
                // Read-only cheatsheet: Enter just closes, no action dispatched.
                editor.finder_select();
            } else if let Some(item) = editor.finder_select() {
                if let Some(position) = item.terminal_session_position {
                    match editor.floating_terminal.select_session(position) {
                        Ok(message) => editor.set_status(message),
                        Err(e) => editor.set_status(format!("Terminal select failed: {}", e)),
                    }
                } else if let Some(buf_idx) = item.buffer_idx {
                    if !editor.switch_to_buffer(buf_idx) {
                        editor.set_status("Buffer not found");
                    }
                } else if item
                    .git_status
                    .map(|status| status.is_deleted())
                    .unwrap_or(false)
                {
                    editor.set_status("File was deleted");
                } else {
                    // Open the selected file
                    let target_line = item.line;
                    let target_col = item.col;
                    if let Err(e) = editor.open_file(item.path) {
                        editor.set_status(format!("Error opening file: {}", e));
                    } else if let Some(line_num) = target_line {
                        // Jump to the line (for grep results)
                        editor.cursor.line = line_num.saturating_sub(1);
                        editor.cursor.col = target_col.unwrap_or(0);
                        editor.clamp_cursor();
                        editor.scroll_to_cursor();
                    }
                }
            }
        }

        // Navigate up - works in both modes
        // Note: List is rendered Telescope-style (best match at bottom)
        // Visual UP means going toward higher indices (at visual top)
        (KeyModifiers::NONE, KeyCode::Up)
        | (KeyModifiers::CONTROL, KeyCode::Char('k'))
        | (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            editor.finder.select_next(); // Visually goes UP
            adjust_scroll(editor);
            selection_changed = true;
        }

        // Navigate down - works in both modes
        // Visual DOWN means going toward lower indices (at visual bottom)
        (KeyModifiers::NONE, KeyCode::Down)
        | (KeyModifiers::CONTROL, KeyCode::Char('j'))
        | (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            editor.finder.select_prev(); // Visually goes DOWN
            adjust_scroll(editor);
            selection_changed = true;
        }

        // Normal mode specific: j/k for navigation
        // Note: List is rendered Telescope-style (best match at bottom, index 0)
        // So j (down visually) = decrement index, k (up visually) = increment index
        (KeyModifiers::NONE, KeyCode::Char('j')) if is_normal_mode => {
            editor.finder.select_prev(); // Visually goes DOWN (toward index 0 at bottom)
            adjust_scroll(editor);
            selection_changed = true;
        }
        (KeyModifiers::NONE, KeyCode::Char('k')) if is_normal_mode => {
            editor.finder.select_next(); // Visually goes UP (toward higher indices at top)
            adjust_scroll(editor);
            selection_changed = true;
        }

        // Normal mode: 'i' to enter insert mode
        (KeyModifiers::NONE, KeyCode::Char('i')) if is_normal_mode => {
            editor.finder.enter_insert_mode();
        }

        // Normal mode: 'p' to toggle preview
        (KeyModifiers::NONE, KeyCode::Char('p')) if is_normal_mode => {
            editor.finder.toggle_preview();
            if editor.finder.preview_enabled && editor.finder.mode_supports_preview() {
                editor.update_finder_preview();
            }
        }

        // Terminal picker: 'n' creates a new terminal session and opens it
        (KeyModifiers::NONE, KeyCode::Char('n'))
            if is_normal_mode && editor.finder.mode == crate::finder::FinderMode::Terminals =>
        {
            match editor.floating_terminal.create_session(None) {
                Ok(message) => {
                    editor.close_finder();
                    editor.set_status(message);
                }
                Err(e) => editor.set_status(format!("Terminal new failed: {}", e)),
            }
        }

        // Normal mode: 'gg' to go to top (simplified to just 'g' for now)
        (KeyModifiers::NONE, KeyCode::Char('g')) if is_normal_mode => {
            editor.finder.selected = 0;
            editor.finder.scroll_offset = 0;
            selection_changed = true;
        }

        // Normal mode: 'G' to go to bottom
        (KeyModifiers::SHIFT, KeyCode::Char('G')) if is_normal_mode => {
            if !editor.finder.filtered.is_empty() {
                editor.finder.selected = editor.finder.filtered.len() - 1;
                adjust_scroll(editor);
                selection_changed = true;
            }
        }

        // Terminal picker: 'd' kills the selected terminal session
        (KeyModifiers::NONE, KeyCode::Char('d'))
            if is_normal_mode && editor.finder.mode == crate::finder::FinderMode::Terminals =>
        {
            let selected = editor.finder.selected;
            let position = editor
                .finder
                .selected_item()
                .and_then(|item| item.terminal_session_position);

            if let Some(position) = position {
                match editor.floating_terminal.close_session(position) {
                    Ok(message) => {
                        if editor.floating_terminal.session_infos().is_empty() {
                            editor.close_finder();
                            editor.set_status(message);
                        } else {
                            editor.open_terminal_picker();
                            if !editor.finder.filtered.is_empty() {
                                editor.finder.selected =
                                    selected.min(editor.finder.filtered.len() - 1);
                                adjust_scroll(editor);
                            }
                            editor.set_status(message);
                        }
                    }
                    Err(e) => editor.set_status(format!("Terminal kill failed: {}", e)),
                }
            }
        }

        // Terminal picker: 'r' opens command mode prefilled for renaming the selected session
        (KeyModifiers::NONE, KeyCode::Char('r'))
            if is_normal_mode && editor.finder.mode == crate::finder::FinderMode::Terminals =>
        {
            let position = editor
                .finder
                .selected_item()
                .and_then(|item| item.terminal_session_position);

            if let Some(position) = position {
                let name = editor
                    .floating_terminal
                    .session_infos()
                    .into_iter()
                    .find(|session| session.position == position)
                    .map(|session| session.name)
                    .unwrap_or_default();
                editor.close_finder();
                editor.enter_command_mode_with_input(format!("termrename {} {}", position, name));
            }
        }

        // Harpoon mode: 'd' to delete selected item
        (KeyModifiers::NONE, KeyCode::Char('d'))
            if is_normal_mode && editor.finder.mode == crate::finder::FinderMode::Harpoon =>
        {
            if editor.finder.selected < editor.finder.filtered.len() {
                let item_idx = editor.finder.filtered[editor.finder.selected];
                editor.harpoon.remove(item_idx);
                // Check if any files remain
                if editor.harpoon.is_empty() {
                    editor.set_status("Harpoon is empty");
                    editor.close_finder();
                } else {
                    // Refresh the finder with updated harpoon files
                    let files: Vec<_> = editor.harpoon.files().to_vec();
                    editor.finder.open_harpoon(files);
                    adjust_scroll(editor);
                }
            }
        }

        // Harpoon mode: 'K' (shift+k) to move item up
        (KeyModifiers::SHIFT, KeyCode::Char('K'))
            if is_normal_mode && editor.finder.mode == crate::finder::FinderMode::Harpoon =>
        {
            if editor.finder.selected < editor.finder.filtered.len() {
                let item_idx = editor.finder.filtered[editor.finder.selected];
                if item_idx > 0 {
                    editor.harpoon.swap(item_idx, item_idx - 1);
                    // Refresh and adjust selection to follow the moved item
                    let new_selected = editor.finder.selected.saturating_sub(1);
                    let files: Vec<_> = editor.harpoon.files().to_vec();
                    editor.finder.open_harpoon(files);
                    editor.finder.selected = new_selected;
                    adjust_scroll(editor);
                }
            }
        }

        // Harpoon mode: 'J' (shift+j) to move item down
        (KeyModifiers::SHIFT, KeyCode::Char('J'))
            if is_normal_mode && editor.finder.mode == crate::finder::FinderMode::Harpoon =>
        {
            if editor.finder.selected < editor.finder.filtered.len() {
                let item_idx = editor.finder.filtered[editor.finder.selected];
                let harpoon_len = editor.harpoon.len();
                if item_idx + 1 < harpoon_len {
                    editor.harpoon.swap(item_idx, item_idx + 1);
                    // Refresh and adjust selection to follow the moved item
                    let new_selected = (editor.finder.selected + 1).min(harpoon_len - 1);
                    let files: Vec<_> = editor.harpoon.files().to_vec();
                    editor.finder.open_harpoon(files);
                    editor.finder.selected = new_selected;
                    adjust_scroll(editor);
                }
            }
        }

        // Marks mode: 'd' to delete selected mark
        (KeyModifiers::NONE, KeyCode::Char('d'))
            if is_normal_mode && editor.finder.mode == crate::finder::FinderMode::Marks =>
        {
            if editor.finder.selected < editor.finder.filtered.len() {
                let item_idx = editor.finder.filtered[editor.finder.selected];
                if let Some(item) = editor.finder.items.get(item_idx) {
                    // Extract mark name from display string (format: " X   line:col  filename")
                    let mark_name = item.display.chars().nth(1);
                    if let Some(name) = mark_name {
                        // Get buffer key for deletion
                        let buffer_key = editor
                            .buffer()
                            .path
                            .as_ref()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|| format!("buffer_{}", editor.current_buffer_index()));

                        if editor.marks.delete(&buffer_key, name) {
                            editor.set_status(format!("Deleted mark '{}'", name));
                            // Check if any marks remain
                            let local_marks = editor.marks.get_local_marks(&buffer_key);
                            let global_marks = editor.marks.get_global_marks();
                            if local_marks.is_empty() && global_marks.is_empty() {
                                // No more marks, close the finder
                                editor.close_finder();
                            } else {
                                // Refresh the finder with updated marks
                                editor.open_finder_marks();
                                adjust_scroll(editor);
                            }
                        }
                    }
                }
            }
        }

        // Backspace
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            editor.finder.delete_char_before();
            selection_changed = true; // Filter might have changed selection
        }

        // Regular character - insert mode types, normal mode switches to insert first
        (_, KeyCode::Char(c)) if !c.is_control() => {
            // insert_char already switches to insert mode if needed
            editor.finder.insert_char(c);
            selection_changed = true; // Filter might have changed selection
        }

        _ => {}
    }

    // Mark preview as needing update (debounced in main loop)
    // This avoids the 10-40ms tree-sitter parsing on every keystroke
    if selection_changed && editor.finder.preview_enabled && editor.finder.mode_supports_preview() {
        editor.finder.preview_update_pending = true;
    }

    log_finder_profile(&format!(
        "handle_finder: {:?} key={:?}",
        t_start.elapsed(),
        key.code
    ));
}

fn explorer_visible_rows(editor: &Editor) -> usize {
    editor.text_rows().saturating_sub(1).max(1)
}

fn explorer_half_page_rows(editor: &Editor) -> usize {
    (explorer_visible_rows(editor) / 2).max(1)
}

fn handle_explorer_mode(editor: &mut Editor, key: KeyEvent) {
    // Handle search input mode
    if editor.explorer.is_searching {
        match (key.modifiers, key.code) {
            // Cancel search
            (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
                editor.explorer.cancel_search();
            }
            // Confirm search
            (KeyModifiers::NONE, KeyCode::Enter) => {
                editor.explorer.confirm_search();
            }
            // Next match (Ctrl+n or Tab)
            (KeyModifiers::CONTROL, KeyCode::Char('n')) | (KeyModifiers::NONE, KeyCode::Tab) => {
                editor.explorer.next_match();
            }
            // Previous match (Ctrl+p or Shift+Tab)
            (KeyModifiers::CONTROL, KeyCode::Char('p'))
            | (KeyModifiers::SHIFT, KeyCode::BackTab) => {
                editor.explorer.prev_match();
            }
            // Backspace
            (KeyModifiers::NONE, KeyCode::Backspace) => {
                editor.explorer.search_backspace();
            }
            // Delete previous word / delete to start, matching command-line editing.
            (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                editor.explorer.search_delete_word_before();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                editor.explorer.search_delete_to_start();
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
            (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
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
            (KeyModifiers::CONTROL, KeyCode::Char('a')) | (KeyModifiers::NONE, KeyCode::Home) => {
                editor.explorer.input_cursor_home();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('e')) | (KeyModifiers::NONE, KeyCode::End) => {
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

    if let Some(ref mut sequence) = editor.explorer_key_sequence {
        if let Some(part) = explorer_sequence_part(key) {
            sequence.push_str(&part);
            let seq = sequence.clone();

            if let Some(action) = editor.keymap.get_explorer_sequence_action(&seq) {
                editor.explorer_key_sequence = None;
                execute_explorer_mode_action(editor, action);
                return;
            }

            if editor.keymap.is_explorer_sequence_prefix(&seq) {
                return;
            }
        }

        editor.explorer_key_sequence = None;
        return;
    }

    // Check if this key is the leader key
    if editor.keymap.has_leader_mappings() && editor.keymap.is_leader_key(key) {
        editor.leader_sequence = Some(String::new());
        editor.set_status("<leader>");
        return;
    }

    if let Some(part) = explorer_sequence_part(key) {
        if editor.keymap.is_explorer_sequence_prefix(&part) {
            editor.explorer_key_sequence = Some(part);
            return;
        }

        if let Some(action) = editor.keymap.get_explorer_sequence_action(&part) {
            execute_explorer_mode_action(editor, action);
            return;
        }
    }

    if let Some(action) = editor.keymap.get_explorer_action(key) {
        execute_explorer_mode_action(editor, action);
    }
}

fn explorer_sequence_part(key: KeyEvent) -> Option<String> {
    match key {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => Some(c.to_string()),
        _ => None,
    }
}

fn execute_explorer_mode_action(editor: &mut Editor, action: crate::config::ExplorerModeAction) {
    use crate::config::ExplorerModeAction;

    match action {
        ExplorerModeAction::Close => editor.close_explorer(),
        ExplorerModeAction::MoveDown => editor.explorer.move_down(),
        ExplorerModeAction::MoveUp => editor.explorer.move_up(),
        ExplorerModeAction::MoveToTop => editor.explorer.move_to_top(),
        ExplorerModeAction::MoveToBottom => editor.explorer.move_to_bottom(),
        ExplorerModeAction::HalfPageDown => editor
            .explorer
            .move_page_down(explorer_half_page_rows(editor)),
        ExplorerModeAction::HalfPageUp => editor
            .explorer
            .move_page_up(explorer_half_page_rows(editor)),
        ExplorerModeAction::PageDown => editor
            .explorer
            .move_page_down(explorer_visible_rows(editor)),
        ExplorerModeAction::PageUp => editor.explorer.move_page_up(explorer_visible_rows(editor)),
        ExplorerModeAction::ToggleOrOpen => toggle_or_open_explorer_selection(editor),
        ExplorerModeAction::ExpandOrOpen => expand_or_open_explorer_selection(editor),
        ExplorerModeAction::CollapseOrParent => editor.explorer.collapse(),
        ExplorerModeAction::ToggleExpand => editor.explorer.toggle_expand(),
        ExplorerModeAction::CollapseAll => editor.explorer.collapse_all(),
        ExplorerModeAction::Refresh => {
            editor.explorer.refresh();
            editor.refresh_explorer_git_statuses();
        }
        ExplorerModeAction::ShowExplorerKeymaps => editor.open_keymaps_picker_with_query("expl"),
        ExplorerModeAction::GoToParent => editor.explorer.go_to_parent(),
        ExplorerModeAction::FocusEditor => editor.unfocus_explorer(),
        ExplorerModeAction::WidenSidebar => editor.widen_explorer(),
        ExplorerModeAction::NarrowSidebar => editor.narrow_explorer(),
        ExplorerModeAction::ResetSidebarWidth => editor.reset_explorer_width(),
        ExplorerModeAction::Create => editor.explorer.start_add(),
        ExplorerModeAction::Rename => editor.explorer.start_rename(),
        ExplorerModeAction::Delete => editor.explorer.start_delete(),
        ExplorerModeAction::Copy => editor.explorer.copy_selected(),
        ExplorerModeAction::Cut => editor.explorer.cut_selected(),
        ExplorerModeAction::Paste => execute_explorer_paste(editor),
        ExplorerModeAction::Search => editor.explorer.start_search(),
        ExplorerModeAction::NextMatch => editor.explorer.next_match(),
        ExplorerModeAction::PreviousMatch => editor.explorer.prev_match(),
    }
}

fn toggle_or_open_explorer_selection(editor: &mut Editor) {
    if let Some(path) = editor.explorer_selected_path() {
        if path.is_dir() {
            editor.explorer.toggle_expand();
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

fn expand_or_open_explorer_selection(editor: &mut Editor) {
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
                    path.parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| path.clone())
                }
            } else if let Some(root) = &editor.project_root {
                root.clone()
            } else {
                editor.set_status("No directory selected");
                editor.explorer.cancel_action();
                return;
            };

            let new_path = parent.join(&input);
            if new_path.exists() {
                editor.set_status(format!("Already exists: {}", new_path.display()));
                return;
            }

            // Check if it's a directory (ends with /)
            if input.ends_with('/') {
                match std::fs::create_dir_all(&new_path) {
                    Ok(_) => {
                        editor.set_status(format!("Created: {}", new_path.display()));
                        editor.explorer.refresh_and_select_path(&new_path);
                        editor.refresh_explorer_git_statuses();
                    }
                    Err(e) => {
                        editor.set_status(format!("Error: {}", e));
                        return;
                    }
                }
            } else {
                // Create parent dirs if needed
                if let Some(parent) = new_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        editor.set_status(format!("Error creating parent directory: {}", e));
                        return;
                    }
                }
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&new_path)
                {
                    Ok(_) => {
                        editor.set_status(format!("Created: {}", new_path.display()));
                        editor.explorer.refresh_and_select_path(&new_path);
                        editor.refresh_explorer_git_statuses();
                        // Auto-open the newly created file
                        if let Err(e) = editor.open_file(new_path.clone()) {
                            editor.set_status(format!("Created but failed to open: {}", e));
                        } else {
                            editor.mode = Mode::Normal;
                        }
                    }
                    Err(e) => {
                        editor.set_status(format!("Error: {}", e));
                        return;
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

                if new_path != old_path && new_path.exists() {
                    editor.set_status(format!("Already exists: {}", new_path.display()));
                    return;
                }

                match std::fs::rename(&old_path, &new_path) {
                    Ok(_) => {
                        editor.set_status(format!("Renamed to: {}", new_path.display()));
                        editor.explorer.refresh_and_select_path(&new_path);
                        editor.refresh_explorer_git_statuses();
                    }
                    Err(e) => {
                        editor.set_status(format!("Error: {}", e));
                        return;
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
                            editor.refresh_explorer_git_statuses();
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
            path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| path.clone())
        }
    } else if let Some(root) = &editor.project_root {
        root.clone()
    } else {
        editor.set_status("No destination directory");
        return;
    };

    let file_name = clipboard
        .path
        .file_name()
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
        ClipboardOp::Cut => std::fs::rename(&clipboard.path, &dest_path),
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
            editor.explorer.refresh_and_select_path(&dest_path);
            editor.refresh_explorer_git_statuses();
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
fn rename_file_impl(
    editor: &mut Editor,
    old_path: std::path::PathBuf,
    new_path: std::path::PathBuf,
) -> CommandResult {
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
                        let file_path = editor
                            .buffer()
                            .path
                            .as_ref()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();

                        match crate::formatter::format_with_external(
                            &content,
                            &file_path,
                            &formatter_config,
                        ) {
                            Ok(formatted) => {
                                if formatted != content {
                                    // Replace buffer content with formatted version
                                    editor.replace_buffer_content_with_undo(&formatted);
                                }
                                // Save the file
                                match editor.save() {
                                    Ok(()) => {
                                        if formatted != content {
                                            CommandResult::Message(format!(
                                                "Formatted with {} and saved",
                                                formatter_name
                                            ))
                                        } else {
                                            CommandResult::Message(format!(
                                                "Saved (formatted with {})",
                                                formatter_name
                                            ))
                                        }
                                    }
                                    Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
                                }
                            }
                            Err(e) => {
                                // Formatter failed - show error but still save
                                editor.set_status(format!("{} error: {}", formatter_name, e));
                                match editor.save() {
                                    Ok(()) => CommandResult::Message(format!(
                                        "Saved ({} failed: {})",
                                        formatter_name, e
                                    )),
                                    Err(save_err) => {
                                        CommandResult::Error(format!("Error saving: {}", save_err))
                                    }
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
            // If multiple panes, :q closes just the active pane after checking
            // the active buffer. If this is the last pane, quitting exits the
            // editor, so all dirty buffers must be accounted for.
            if editor.panes().len() > 1 {
                if editor.has_unsaved_changes() {
                    CommandResult::Error(
                        "No write since last change (add ! to override)".to_string(),
                    )
                } else {
                    editor.close_pane();
                    CommandResult::Ok
                }
            } else if editor.has_any_unsaved_changes() {
                let names = editor.unsaved_buffer_names();
                CommandResult::Error(format!(
                    "No write since last change in: {} (add ! to override)",
                    names.join(", ")
                ))
            } else {
                CommandResult::Quit
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

        Command::WriteAll => match editor.format_and_save_all() {
            Ok((saved_count, formatted_count, formatter_name)) => {
                if saved_count == 0 {
                    CommandResult::Message("No modified buffers to save".to_string())
                } else {
                    let buffer_word = if saved_count == 1 {
                        "buffer"
                    } else {
                        "buffers"
                    };
                    if formatted_count > 0 {
                        let formatter = formatter_name.unwrap_or_else(|| "formatter".to_string());
                        CommandResult::Message(format!(
                            "Formatted with {} and saved {} {}",
                            formatter, saved_count, buffer_word
                        ))
                    } else {
                        CommandResult::Message(format!("Saved {} {}", saved_count, buffer_word))
                    }
                }
            }
            Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
        },

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

        Command::ForceQuitAll => CommandResult::Quit,

        Command::WriteQuitAll => match editor.save_all() {
            Ok(_) => CommandResult::Quit,
            Err(e) => CommandResult::Error(format!("Error saving: {}", e)),
        },

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
                    Ok(()) => {
                        CommandResult::Message(format!("\"{}\"", editor.buffer().display_name()))
                    }
                    Err(e) => CommandResult::Error(format!("Error opening file: {}", e)),
                }
            } else if editor.buffer().path.is_some() {
                if editor.has_unsaved_changes() {
                    CommandResult::Error(
                        "No write since last change (add ! to override)".to_string(),
                    )
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

        Command::Reload => match editor.reload() {
            Ok(()) => CommandResult::Ok,
            Err(e) => CommandResult::Error(format!("Error reloading: {}", e)),
        },

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

        Command::BufferDelete(force) => {
            if !force && editor.has_unsaved_changes() {
                CommandResult::Error("No write since last change (add ! to override)".to_string())
            } else {
                editor.close_current_buffer();
                CommandResult::Message("Buffer deleted".to_string())
            }
        }

        Command::Set(option, _value) => CommandResult::Error(format!("Unknown option: {}", option)),

        Command::LazyGit => CommandResult::RunExternal("lazygit".to_string()),

        Command::Shell(shell_cmd) => {
            if shell_cmd.is_empty() {
                CommandResult::Error("No command specified".to_string())
            } else {
                CommandResult::RunExternal(shell_cmd)
            }
        }

        Command::VSplit(path) => match editor.vsplit(path) {
            Ok(()) => CommandResult::Ok,
            Err(e) => CommandResult::Error(format!("Error: {}", e)),
        },

        Command::HSplit(path) => match editor.hsplit(path) {
            Ok(()) => CommandResult::Ok,
            Err(e) => CommandResult::Error(format!("Error: {}", e)),
        },

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

        Command::SearchWord => {
            editor.open_finder_grep_word();
            CommandResult::Ok
        }

        Command::FindDiagnostics => {
            editor.open_finder_diagnostics();
            CommandResult::Ok
        }

        Command::GitChanges => {
            editor.open_finder_git_changes();
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

        Command::MarkdownPreview => match editor.open_markdown_preview() {
            Ok(()) => CommandResult::Ok,
            Err(message) => CommandResult::Message(message.to_string()),
        },

        Command::NoHighlight => {
            editor.search_matches.clear();
            CommandResult::Ok
        }

        Command::Substitute {
            entire_file,
            pattern,
            replacement,
            global,
        } => {
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
                    old_path
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join(&new_name)
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
                Ok(_) => {
                    CommandResult::Message(format!("Created directory: {}", full_path.display()))
                }
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
            editor.open_finder_harpoon();
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
        Command::TerminalNew(name) => match editor.floating_terminal.create_session(name) {
            Ok(message) => CommandResult::Message(message),
            Err(e) => CommandResult::Error(format!("Terminal new failed: {}", e)),
        },
        Command::TerminalNext => match editor.floating_terminal.next_session() {
            Ok(message) => CommandResult::Message(message),
            Err(e) => CommandResult::Error(format!("Terminal next failed: {}", e)),
        },
        Command::TerminalPrev => match editor.floating_terminal.previous_session() {
            Ok(message) => CommandResult::Message(message),
            Err(e) => CommandResult::Error(format!("Terminal previous failed: {}", e)),
        },
        Command::TerminalList => CommandResult::Message(editor.floating_terminal.list_sessions()),
        Command::TerminalPicker => {
            editor.open_terminal_picker();
            CommandResult::Ok
        }
        Command::TerminalSelect(index) => match editor.floating_terminal.select_session(index) {
            Ok(message) => CommandResult::Message(message),
            Err(e) => CommandResult::Error(format!("Terminal select failed: {}", e)),
        },
        Command::TerminalRename(position, name) => {
            let result = match position {
                Some(position) => editor.floating_terminal.rename_session(position, name),
                None => editor.floating_terminal.rename_active_session(name),
            };
            match result {
                Ok(message) => CommandResult::Message(message),
                Err(e) => CommandResult::Error(format!("Terminal rename failed: {}", e)),
            }
        }
        Command::TerminalRenamePrompt => {
            if let Some(session) = editor
                .floating_terminal
                .session_infos()
                .into_iter()
                .find(|session| session.active)
            {
                editor.enter_command_mode_with_input(format!(
                    "termrename {} {}",
                    session.position, session.name
                ));
                CommandResult::Ok
            } else {
                CommandResult::Error("Terminal rename failed: No terminal session".to_string())
            }
        }
        Command::TerminalKill => {
            editor.floating_terminal.close();
            CommandResult::Message("Terminal killed".to_string())
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
        Command::Keymaps => {
            editor.open_keymaps_picker();
            CommandResult::Ok
        }
        Command::CheckHealth => {
            editor.open_health_report();
            CommandResult::Ok
        }
        Command::ConfigOpen => match editor.open_user_config_file() {
            Ok(path) => CommandResult::Message(format!("Opened config: {}", path.display())),
            Err(message) => CommandResult::Error(message),
        },
        Command::ConfigDefaults => {
            editor.open_config_defaults_preview();
            CommandResult::Ok
        }

        Command::Marks => {
            editor.open_finder_marks();
            CommandResult::Ok
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
            editor.set_status(format!(
                "Delete {}? Use :delete! to confirm",
                path.display()
            ));
        }
    }
}

/// Execute a leader key action
pub fn execute_leader_action(editor: &mut Editor, action: &LeaderAction) {
    editor.leader_sequence = None;
    editor.leader_sequence_start = None;
    editor.leader_pending_action = None;

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

#[cfg(test)]
mod tests {
    use super::{
        apply_diagnostic_underline, diagnostic_at_col, diagnostic_underline_color, execute_command,
        execute_leader_action, finder_preview_match_ranges, handle_insert_mode, handle_key,
        replace_completion_text, Terminal,
    };
    use crate::commands::{Command, CommandPopupMode};
    use crate::config::{KeymapEntry, Settings};
    use crate::editor::{Editor, Mode, RegisterContent};
    use crate::explorer::{ExplorerAction, FlatNode};
    use crate::finder::FinderMode;
    use crate::input::Motion;
    use crate::lsp::types::{CompletionItem, CompletionKind, Diagnostic, DiagnosticSeverity};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use crossterm::style::Color;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn shift_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    }

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn alt_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    fn enter_key() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    fn up_key() -> KeyEvent {
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)
    }

    fn down_key() -> KeyEvent {
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
    }

    fn esc_key() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }

    fn numbered_lines(count: usize) -> Vec<String> {
        (1..=count).map(|line| format!("line {}", line)).collect()
    }

    #[test]
    fn command_ctrl_b_moves_to_beginning_of_command_line() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("write buffer");

        handle_key(&mut editor, ctrl_key('b'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "write buffer");
        assert_eq!(editor.command_line.cursor, 0);
    }

    #[test]
    fn command_ctrl_e_moves_to_end_of_command_line() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("write buffer");
        editor.command_line.cursor = 2;

        handle_key(&mut editor, ctrl_key('e'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "write buffer");
        assert_eq!(editor.command_line.cursor, "write buffer".chars().count());
    }

    #[test]
    fn search_ctrl_b_moves_to_beginning_of_search_prompt() {
        let mut editor = Editor::default();
        editor.enter_search_forward();
        editor.search.input = "needle".to_string();
        editor.search.cursor = 4;

        handle_key(&mut editor, ctrl_key('b'));

        assert_eq!(editor.mode, Mode::Search);
        assert_eq!(editor.search.input, "needle");
        assert_eq!(editor.search.cursor, 0);
    }

    #[test]
    fn search_ctrl_e_moves_to_end_of_search_prompt() {
        let mut editor = Editor::default();
        editor.enter_search_backward();
        editor.search.input = "naive".to_string();
        editor.search.cursor = 2;

        handle_key(&mut editor, ctrl_key('e'));

        assert_eq!(editor.mode, Mode::Search);
        assert_eq!(editor.search.input, "naive");
        assert_eq!(editor.search.cursor, "naive".chars().count());
    }

    #[test]
    fn search_up_down_navigates_search_history() {
        let mut editor = Editor::default();

        editor.enter_search_forward();
        editor.search.input = "alpha".to_string();
        editor.search.cursor = "alpha".chars().count();
        handle_key(&mut editor, enter_key());

        editor.enter_search_forward();
        editor.search.input = "beta".to_string();
        editor.search.cursor = "beta".chars().count();
        handle_key(&mut editor, enter_key());

        editor.enter_search_forward();
        handle_key(&mut editor, up_key());
        assert_eq!(editor.mode, Mode::Search);
        assert_eq!(editor.search.input, "beta");
        assert_eq!(editor.search.cursor, "beta".chars().count());

        handle_key(&mut editor, up_key());
        assert_eq!(editor.search.input, "alpha");
        assert_eq!(editor.search.cursor, "alpha".chars().count());

        handle_key(&mut editor, down_key());
        assert_eq!(editor.search.input, "beta");
        assert_eq!(editor.search.cursor, "beta".chars().count());

        handle_key(&mut editor, down_key());
        assert_eq!(editor.search.input, "");
        assert_eq!(editor.search.cursor, 0);
    }

    #[test]
    fn search_history_restores_in_progress_input_and_resets_after_edit() {
        let mut editor = Editor::default();

        editor.enter_search_forward();
        editor.search.input = "stored".to_string();
        editor.search.cursor = "stored".chars().count();
        handle_key(&mut editor, enter_key());

        editor.enter_search_forward();
        editor.search.input = "draft".to_string();
        editor.search.cursor = "draft".chars().count();
        handle_key(&mut editor, up_key());
        assert_eq!(editor.search.input, "stored");

        handle_key(&mut editor, down_key());
        assert_eq!(editor.search.input, "draft");
        assert_eq!(editor.search.cursor, "draft".chars().count());

        handle_key(&mut editor, up_key());
        handle_key(&mut editor, key('x'));
        assert_eq!(editor.search.input, "storedx");
        assert_eq!(editor.search.cursor, "storedx".chars().count());

        handle_key(&mut editor, down_key());
        assert_eq!(editor.search.input, "storedx");
        assert_eq!(editor.search.cursor, "storedx".chars().count());
    }

    #[test]
    fn search_ctrl_w_deletes_word_before_search_prompt_cursor() {
        let mut editor = Editor::default();
        editor.enter_search_forward();
        editor.search.input = "foo bar baz".to_string();
        editor.search.cursor = "foo bar baz".chars().count();

        handle_key(&mut editor, ctrl_key('w'));

        assert_eq!(editor.mode, Mode::Search);
        assert_eq!(editor.search.input, "foo bar ");
        assert_eq!(editor.search.cursor, "foo bar ".chars().count());
    }

    #[test]
    fn search_ctrl_w_skips_trailing_spaces_before_deleting_word() {
        let mut editor = Editor::default();
        editor.enter_search_forward();
        editor.search.input = "foo bar   ".to_string();
        editor.search.cursor = "foo bar   ".chars().count();

        handle_key(&mut editor, ctrl_key('w'));

        assert_eq!(editor.mode, Mode::Search);
        assert_eq!(editor.search.input, "foo ");
        assert_eq!(editor.search.cursor, "foo ".chars().count());
    }

    #[test]
    fn search_ctrl_u_deletes_from_search_prompt_cursor_to_start() {
        let mut editor = Editor::default();
        editor.enter_search_forward();
        editor.search.input = "foo bar baz".to_string();
        editor.search.cursor = "foo bar ".chars().count();

        handle_key(&mut editor, ctrl_key('u'));

        assert_eq!(editor.mode, Mode::Search);
        assert_eq!(editor.search.input, "baz");
        assert_eq!(editor.search.cursor, 0);
    }

    #[test]
    fn search_ctrl_r_inserts_named_register_text_at_search_cursor() {
        let mut editor = Editor::default();
        editor
            .registers
            .yank(Some('a'), RegisterContent::Chars("needle".to_string()));
        editor.enter_search_backward();
        editor.search.input = "foo bar".to_string();
        editor.search.cursor = "foo ".chars().count();

        handle_key(&mut editor, ctrl_key('r'));
        handle_key(&mut editor, key('a'));

        assert_eq!(editor.mode, Mode::Search);
        assert_eq!(editor.search.input, "foo needlebar");
        assert_eq!(editor.search.cursor, "foo needle".chars().count());
    }

    #[test]
    fn command_ctrl_w_deletes_word_before_command_line_cursor() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("write foo bar");

        handle_key(&mut editor, ctrl_key('w'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "write foo ");
        assert_eq!(editor.command_line.cursor, "write foo ".chars().count());
    }

    #[test]
    fn command_ctrl_w_skips_trailing_spaces_before_deleting_word() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("write foo   ");

        handle_key(&mut editor, ctrl_key('w'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "write ");
        assert_eq!(editor.command_line.cursor, "write ".chars().count());
    }

    #[test]
    fn command_ctrl_u_deletes_from_command_line_cursor_to_start() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("write foo bar");
        editor.command_line.cursor = "write foo ".chars().count();

        handle_key(&mut editor, ctrl_key('u'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "bar");
        assert_eq!(editor.command_line.cursor, 0);
    }

    #[test]
    fn command_ctrl_r_inserts_named_register_text_at_command_cursor() {
        let mut editor = Editor::default();
        editor
            .registers
            .yank(Some('a'), RegisterContent::Chars("nevi.txt".to_string()));
        editor.enter_command_mode_with_input("write bar");
        editor.command_line.cursor = "write ".chars().count();

        handle_key(&mut editor, ctrl_key('r'));
        handle_key(&mut editor, key('a'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "write nevi.txtbar");
        assert_eq!(editor.command_line.cursor, "write nevi.txt".chars().count());
    }

    #[test]
    fn command_ctrl_d_lists_command_line_completions_without_accepting() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("w");
        editor.command_line.popup_mode = CommandPopupMode::None;

        handle_key(&mut editor, ctrl_key('d'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "w");
        assert_eq!(editor.command_line.cursor, 1);
        assert_eq!(editor.command_line.popup_mode, CommandPopupMode::Completion);
        assert!(editor
            .command_line
            .suggestions
            .iter()
            .any(|item| item.command == "w"));
    }

    #[test]
    fn command_ctrl_l_completes_longest_common_command_prefix() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("Term");

        handle_key(&mut editor, ctrl_key('l'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "Terminal");
        assert_eq!(editor.command_line.cursor, "Terminal".chars().count());
        assert_eq!(editor.command_line.popup_mode, CommandPopupMode::Completion);
    }

    #[test]
    fn command_ctrl_a_inserts_all_matching_command_completions() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("Term");

        handle_key(&mut editor, ctrl_key('a'));

        let expected = "Terminal TerminalList TerminalNew TerminalSelect TerminalKill TerminalNext TerminalPrev Terminals TerminalRename";
        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, expected);
        assert_eq!(editor.command_line.cursor, expected.chars().count());
        assert_eq!(editor.command_line.popup_mode, CommandPopupMode::None);
    }

    #[test]
    fn command_ctrl_f_opens_command_line_window_without_changing_input() {
        let mut editor = Editor::default();
        editor.command_line.history = vec!["write".to_string(), "quit".to_string()];
        editor.enter_command_mode_with_input("w");
        editor.command_line.popup_mode = CommandPopupMode::None;

        handle_key(&mut editor, ctrl_key('f'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "w");
        assert_eq!(editor.command_line.cursor, 1);
        assert_eq!(editor.command_line.popup_mode, CommandPopupMode::History);
        assert!(editor
            .command_line
            .history_popup_items
            .iter()
            .any(|item| item == "write"));
    }

    #[test]
    fn command_alt_r_toggles_command_history_popup() {
        let mut editor = Editor::default();
        editor.command_line.history = vec!["write".to_string()];
        editor.enter_command_mode();

        handle_key(&mut editor, alt_key('r'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.popup_mode, CommandPopupMode::History);

        handle_key(&mut editor, alt_key('r'));

        assert_ne!(editor.command_line.popup_mode, CommandPopupMode::History);
    }

    #[test]
    fn command_ctrl_v_inserts_next_control_key_literally() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("write keep");

        handle_key(&mut editor, ctrl_key('v'));
        handle_key(&mut editor, ctrl_key('u'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "write keep\u{15}");
        assert_eq!(editor.command_line.display(), ":write keep^U");
        assert_eq!(
            editor.command_line.cursor,
            "write keep\u{15}".chars().count()
        );
    }

    #[test]
    fn command_ctrl_q_inserts_next_control_key_literally() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("write keep");

        handle_key(&mut editor, ctrl_key('q'));
        handle_key(&mut editor, ctrl_key('w'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "write keep\u{17}");
        assert_eq!(editor.command_line.display(), ":write keep^W");
        assert_eq!(
            editor.command_line.cursor,
            "write keep\u{17}".chars().count()
        );
    }

    #[test]
    fn command_ctrl_k_inserts_vim_digraph() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("echo ");

        handle_key(&mut editor, ctrl_key('k'));
        handle_key(&mut editor, key('a'));
        handle_key(&mut editor, key(':'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "echo ä");
        assert_eq!(editor.command_line.cursor, "echo ä".chars().count());
    }

    #[test]
    fn command_ctrl_k_inserts_second_vim_digraph() {
        let mut editor = Editor::default();
        editor.enter_command_mode_with_input("echo ");

        handle_key(&mut editor, ctrl_key('k'));
        handle_key(&mut editor, key('n'));
        handle_key(&mut editor, key('?'));

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "echo ñ");
        assert_eq!(editor.command_line.cursor, "echo ñ".chars().count());
    }

    #[test]
    fn editor_char_display_width_expands_tabs_and_sanitizes_controls() {
        assert_eq!(super::editor_char_display_width('\t', 4), 4);
        assert_eq!(super::editor_char_display_width('\t', 0), 1);
        assert_eq!(super::editor_char_display_width('\u{0007}', 4), 1);
        assert_eq!(super::editor_char_display_width('a', 4), 1);
    }

    #[test]
    fn editor_char_display_width_counts_full_width_characters() {
        assert_eq!(super::editor_char_display_width('，', 4), 2);
        assert_eq!(super::text_display_width("，，，，", 4), 8);
        assert_eq!(super::take_display_width("，，，", 0, 4, 4), "，，");
    }

    #[test]
    fn take_display_width_stops_before_expanded_tab_overflows() {
        assert_eq!(super::take_display_width("ab\tcd", 0, 5, 4), "ab");
        assert_eq!(super::take_display_width("ab\tcd", 0, 6, 4), "ab\t");
    }

    #[test]
    fn display_width_between_char_cols_measures_visible_cursor_prefix() {
        assert_eq!(
            super::display_width_between_char_cols("，，，，", 0, 4, 4),
            8
        );
        assert_eq!(super::display_width_between_char_cols("a，b", 0, 2, 4), 3);
        assert_eq!(super::display_width_between_char_cols("ab，，", 1, 4, 4), 5);
        assert_eq!(super::display_width_between_char_cols("x\tz", 0, 2, 4), 5);
    }

    #[test]
    fn explorer_action_input_view_keeps_cursor_visible_for_long_names() {
        let input = "abcdefghijklmnopqrstuvwxyz";

        let (visible, cursor_x) = super::explorer_action_input_view(input, input.len(), 10);

        assert_eq!(visible, "…stuvwxyz");
        assert_eq!(cursor_x, 9);
    }

    #[test]
    fn explorer_action_input_view_preserves_prefix_while_cursor_near_start() {
        let input = "abcdefghijklmnopqrstuvwxyz";

        let (visible, cursor_x) = super::explorer_action_input_view(input, 5, 10);

        assert_eq!(visible, "abcdefghij");
        assert_eq!(cursor_x, 5);
    }

    #[test]
    fn explorer_g_and_gg_follow_regular_buffer_top_bottom_motions() {
        let mut editor = editor_with_explorer_rows(5);
        editor.explorer.selected = 2;

        handle_key(&mut editor, shift_key('G'));
        assert_eq!(editor.mode, Mode::Explorer);
        assert_eq!(editor.explorer.selected, 4);

        handle_key(&mut editor, key('g'));
        assert_eq!(
            editor.explorer.selected, 4,
            "single g should wait for a second g"
        );

        handle_key(&mut editor, key('g'));
        assert_eq!(editor.explorer.selected, 0);
    }

    #[test]
    fn explorer_ctrl_page_motions_follow_regular_buffer_scrolling() {
        let mut editor = editor_with_explorer_rows(30);
        editor.set_size(100, 12);
        editor.explorer.selected = 10;

        handle_key(&mut editor, ctrl_key('d'));
        assert_eq!(editor.explorer.selected, 14);

        handle_key(&mut editor, ctrl_key('u'));
        assert_eq!(editor.explorer.selected, 10);

        handle_key(&mut editor, ctrl_key('f'));
        assert_eq!(editor.explorer.selected, 19);

        handle_key(&mut editor, ctrl_key('b'));
        assert_eq!(editor.explorer.selected, 10);
    }

    #[test]
    fn explorer_width_keys_resize_and_reset_sidebar() {
        let settings: Settings = toml::from_str(
            r#"
            [explorer]
            width = 42
            "#,
        )
        .expect("parse settings");
        let mut editor = Editor::new(settings);
        editor.set_size(100, 24);
        editor.open_explorer();

        assert_eq!(editor.explorer.width, 42);
        assert_eq!(editor.panes()[0].rect.x, 43);

        handle_key(&mut editor, key('>'));
        assert_eq!(editor.mode, Mode::Explorer);
        assert_eq!(editor.explorer.width, 47);
        assert_eq!(editor.panes()[0].rect.x, 48);

        handle_key(&mut editor, key('<'));
        assert_eq!(editor.explorer.width, 42);
        assert_eq!(editor.panes()[0].rect.x, 43);

        handle_key(&mut editor, key('>'));
        handle_key(&mut editor, key('>'));
        assert_eq!(editor.explorer.width, 52);

        handle_key(&mut editor, key('='));
        assert_eq!(editor.explorer.width, 42);
        assert_eq!(editor.panes()[0].rect.x, 43);
    }

    #[test]
    fn explorer_keymap_can_rebind_sidebar_width_action() {
        let settings: Settings = toml::from_str(
            r#"
            [[keymap.explorer]]
            key = "o"
            action = "widen_sidebar"
            desc = "Widen explorer sidebar"
            "#,
        )
        .expect("parse settings");
        let mut editor = Editor::new(settings);
        editor.set_size(100, 24);
        editor.open_explorer();

        assert_eq!(editor.explorer.width, 35);

        handle_key(&mut editor, key('o'));
        assert_eq!(editor.mode, Mode::Explorer);
        assert_eq!(editor.explorer.width, 40);

        handle_key(&mut editor, key('>'));
        assert_eq!(
            editor.explorer.width, 40,
            "user binding should replace the default for the same explorer action"
        );
    }

    #[test]
    fn invalid_explorer_keymap_keeps_default_binding_and_reports_error() {
        let settings: Settings = toml::from_str(
            r#"
            [[keymap.explorer]]
            key = "<Nope>"
            action = "widen_sidebar"
            desc = "Widen explorer sidebar"
            "#,
        )
        .expect("parse settings");
        let mut editor = Editor::new(settings);
        editor.set_size(100, 24);
        editor.open_explorer();

        handle_key(&mut editor, key('>'));
        assert_eq!(editor.explorer.width, 40);

        let errors = editor.take_startup_errors();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("invalid explorer mode key '<Nope>'")),
            "expected invalid explorer keymap error, got {errors:?}"
        );
    }

    #[test]
    fn explorer_question_opens_keymaps_filtered_to_explorer() {
        let mut editor = editor_with_explorer_rows(5);

        handle_key(&mut editor, key('?'));

        assert_eq!(editor.mode, Mode::Finder);
        assert_eq!(editor.finder.mode, FinderMode::Keymaps);
        assert_eq!(editor.finder.query, "expl");
        assert!(
            editor.finder.filtered.iter().any(|idx| {
                editor
                    .finder
                    .items
                    .get(*idx)
                    .map(|item| item.display.contains("gg") && item.display.contains("top"))
                    .unwrap_or(false)
            }),
            "filtered keymaps should include explorer mappings"
        );
    }

    #[test]
    fn explorer_search_ctrl_w_and_ctrl_u_edit_query() {
        let mut editor = editor_with_explorer_rows(12);
        editor.explorer.flat_view[1].name = "file_1 match.txt".to_string();

        handle_key(&mut editor, key('/'));
        for ch in "file_1 file_".chars() {
            handle_key(&mut editor, key(ch));
        }

        handle_key(&mut editor, ctrl_key('w'));
        assert_eq!(editor.mode, Mode::Explorer);
        assert!(editor.explorer.is_searching);
        assert_eq!(editor.explorer.search_buffer, "file_1 ");
        assert_eq!(editor.explorer.search_matches, vec![1]);

        handle_key(&mut editor, ctrl_key('u'));
        assert_eq!(editor.explorer.search_buffer, "");
        assert!(editor.explorer.search_matches.is_empty());
    }

    #[test]
    fn explorer_create_existing_file_does_not_overwrite_and_keeps_prompt_open() {
        let root = unique_temp_dir("nevi_explorer_create_collision");
        std::fs::create_dir_all(&root).expect("create root");
        let existing = root.join("existing.rs");
        std::fs::write(&existing, "keep me").expect("write existing file");

        let mut editor = editor_with_project_explorer(&root);
        editor.explorer.start_add();
        editor.explorer.input_buffer = "existing.rs".to_string();

        super::execute_explorer_action(&mut editor);

        assert_eq!(
            std::fs::read_to_string(&existing).expect("read existing file"),
            "keep me"
        );
        assert_eq!(editor.explorer.pending_action, Some(ExplorerAction::Add));
        assert!(editor
            .status_message
            .as_deref()
            .unwrap_or_default()
            .contains("Already exists"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn explorer_rename_existing_file_does_not_overwrite_and_keeps_prompt_open() {
        let root = unique_temp_dir("nevi_explorer_rename_collision");
        std::fs::create_dir_all(&root).expect("create root");
        let source = root.join("source.rs");
        let target = root.join("target.rs");
        std::fs::write(&source, "source").expect("write source file");
        std::fs::write(&target, "target").expect("write target file");

        let mut editor = editor_with_project_explorer(&root);
        select_explorer_path(&mut editor, &source);
        editor.explorer.start_rename();
        editor.explorer.input_buffer = "target.rs".to_string();

        super::execute_explorer_action(&mut editor);

        assert_eq!(
            std::fs::read_to_string(&source).expect("read source file"),
            "source"
        );
        assert_eq!(
            std::fs::read_to_string(&target).expect("read target file"),
            "target"
        );
        assert_eq!(editor.explorer.pending_action, Some(ExplorerAction::Rename));
        assert!(editor
            .status_message
            .as_deref()
            .unwrap_or_default()
            .contains("Already exists"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn wrap_segments_measure_tabs_by_display_width() {
        let segments = super::calculate_wrap_segments("ab\tcd", 5, true, 4);

        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "ab");
        assert_eq!(segments[1].text, "\tc");
        assert_eq!(segments[2].text, "d");
        assert!(segments
            .iter()
            .all(|segment| super::text_display_width(&segment.text, 4) <= 5));
    }

    #[test]
    fn wrapped_continuation_indent_does_not_shift_source_columns() {
        let line = "    url: 'https://services.arcgis.com/EDxZDh4hQQ1a9KvA/arcgis/rest/services/whippoints/FeatureServer/0',";
        let segments = super::calculate_wrap_segments(line, 80, true, 4);
        let continuation = segments
            .iter()
            .find(|segment| !segment.is_first)
            .expect("long URL should wrap");

        assert_eq!(continuation.virtual_prefix_chars, 4);
        assert_eq!(
            super::source_col_for_wrap_segment_char(
                continuation,
                continuation.virtual_prefix_chars
            ),
            Some(continuation.start_col)
        );
        assert_eq!(
            super::source_col_for_wrap_segment_char(
                continuation,
                continuation.virtual_prefix_chars + 1
            ),
            Some(continuation.start_col + 1)
        );
        assert_eq!(
            super::source_col_for_wrap_segment_char(continuation, 0),
            None
        );
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos))
    }

    fn editor_with_explorer_rows(count: usize) -> Editor {
        let mut editor = Editor::default();
        editor.mode = Mode::Explorer;
        editor.explorer.visible = true;
        editor.explorer.flat_view = (0..count)
            .map(|idx| FlatNode {
                path: PathBuf::from(format!("/tmp/nevi-explorer-motion/file_{idx}.txt")),
                name: format!("file_{idx}.txt"),
                is_dir: false,
                depth: 1,
                is_expanded: false,
            })
            .collect();
        editor
    }

    fn editor_with_project_explorer(root: &std::path::Path) -> Editor {
        let mut editor = Editor::default();
        editor.set_project_root(root.to_path_buf());
        editor.open_explorer();
        editor
    }

    fn select_explorer_path(editor: &mut Editor, path: &std::path::Path) {
        editor.explorer.selected = editor
            .explorer
            .flat_view
            .iter()
            .position(|node| node.path == path)
            .expect("path should be visible in explorer");
    }

    fn completion_item(label: &str) -> CompletionItem {
        CompletionItem {
            item_id: 0,
            label: label.to_string(),
            kind: CompletionKind::Function,
            detail: None,
            documentation: None,
            insert_text: None,
            filter_text: None,
            sort_text: Some(label.to_string()),
            text_edit: None,
            additional_text_edits: Vec::new(),
            raw_data: None,
        }
    }

    fn diagnostic(
        line: usize,
        col_start: usize,
        col_end: usize,
        severity: DiagnosticSeverity,
    ) -> Diagnostic {
        Diagnostic {
            line,
            col_start,
            end_line: line,
            col_end,
            message: "diagnostic".to_string(),
            severity,
            source: None,
            code: None,
        }
    }

    #[test]
    fn diagnostic_underline_color_marks_error_range_only() {
        let diag = diagnostic(0, 4, 10, DiagnosticSeverity::Error);
        let diagnostics = vec![&diag];
        let underline_color = Color::Rgb { r: 255, g: 0, b: 0 };

        assert_eq!(
            diagnostic_at_col(&diagnostics, 0, 6)
                .and_then(|diag| diagnostic_underline_color(diag, underline_color)),
            Some(underline_color)
        );
        assert!(diagnostic_at_col(&diagnostics, 0, 3).is_none());
        assert!(diagnostic_at_col(&diagnostics, 0, 10).is_none());
    }

    #[test]
    fn diagnostic_underline_color_skips_hints() {
        let diag = diagnostic(0, 0, 5, DiagnosticSeverity::Hint);

        assert_eq!(
            diagnostic_underline_color(&diag, Color::Rgb { r: 255, g: 0, b: 0 }),
            None
        );
    }

    #[test]
    fn diagnostic_underline_keeps_token_foreground_after_enabling() {
        crossterm::style::force_color_output(true);
        let mut output = Vec::new();
        let token_fg = Color::Rgb {
            r: 97,
            g: 175,
            b: 239,
        };

        apply_diagnostic_underline(
            &mut output,
            Some(Color::Rgb { r: 255, g: 0, b: 0 }),
            token_fg,
        )
        .expect("apply underline");

        let rendered = String::from_utf8(output).expect("utf8");
        assert!(rendered.contains("\x1b[4:3m"));
        assert!(rendered.contains("\x1b[38;2;97;175;239m"));
        assert!(rendered.ends_with("\x1b[58:2:255:0:0m"));
    }

    #[test]
    fn diagnostic_underline_keeps_token_foreground_after_disabling() {
        crossterm::style::force_color_output(true);
        let mut output = Vec::new();
        let token_fg = Color::Rgb {
            r: 198,
            g: 120,
            b: 221,
        };

        apply_diagnostic_underline(&mut output, None, token_fg).expect("clear underline");

        let rendered = String::from_utf8(output).expect("utf8");
        assert!(rendered.contains("\x1b[24m"));
        assert!(rendered.contains("\x1b[59m"));
        assert!(rendered.ends_with("\x1b[38;2;198;120;221m"));
    }

    #[test]
    fn markdown_preview_command_opens_only_for_markdown_buffers() {
        let tmp = unique_temp_dir("nevi_markdown_preview_command");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let markdown_path = tmp.join("notes.md");
        let rust_path = tmp.join("main.rs");
        std::fs::write(&markdown_path, "# Notes\n").expect("write markdown");
        std::fs::write(&rust_path, "fn main() {}\n").expect("write rust");

        let mut markdown = Editor::default();
        markdown.open_file(markdown_path).expect("open markdown");
        execute_command(&mut markdown, Command::MarkdownPreview);
        assert!(markdown.markdown_preview.is_some());

        let mut rust = Editor::default();
        rust.open_file(rust_path).expect("open rust");
        execute_command(&mut rust, Command::MarkdownPreview);
        assert!(rust.markdown_preview.is_none());
        assert_eq!(
            rust.status_message.as_deref(),
            Some("Markdown preview is only available for Markdown buffers")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn checkhealth_command_opens_health_report_preview() {
        let mut editor = Editor::default();

        execute_command(&mut editor, Command::CheckHealth);

        let preview = editor.markdown_preview.as_ref().expect("health preview");
        assert_eq!(preview.title, "Health");
        let text = preview
            .lines
            .iter()
            .map(|line| line.plain_text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Nevi Health"));
        assert!(text.contains("Configuration"));
        assert!(text.contains("Performance"));
        assert!(text.contains("LSP"));
    }

    #[test]
    fn configdefaults_command_opens_default_config_preview() {
        let mut editor = Editor::default();

        execute_command(
            &mut editor,
            crate::commands::parse_command("ConfigDefaults"),
        );

        let preview = editor
            .markdown_preview
            .as_ref()
            .expect("config defaults preview");
        assert_eq!(preview.title, "Config Defaults");
        let text = preview
            .lines
            .iter()
            .map(|line| line.plain_text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("# Nevi Configuration"));
        assert!(text.contains("show_leader_popup"));
        assert!(text.contains("explorer"));
    }

    #[test]
    fn markdown_preview_rect_uses_ninety_percent_width_for_reading() {
        let mut editor = Editor::default();
        editor.set_size(140, 50);

        let rect = Terminal::markdown_preview_rect(&editor);

        assert_eq!(rect.width, 126);
        assert_eq!(rect.height, 46);
        assert_eq!(rect.x, 7);
        assert_eq!(rect.y, 2);
    }

    #[test]
    fn markdown_preview_keys_scroll_and_close_the_overlay() {
        let tmp = unique_temp_dir("nevi_markdown_preview_keys");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let markdown_path = tmp.join("notes.md");
        std::fs::write(&markdown_path, "# Notes\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.open_file(markdown_path).expect("open markdown");
        editor.replace_buffer_content(&(0..30).map(|i| format!("line {i}\n")).collect::<String>());
        editor.open_markdown_preview().expect("open preview");

        handle_key(
            &mut editor,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(editor.markdown_preview.as_ref().unwrap().scroll, 1);

        handle_key(
            &mut editor,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        );
        assert!(editor.markdown_preview.as_ref().unwrap().scroll > 1);

        handle_key(
            &mut editor,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert!(editor.markdown_preview.is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn markdown_preview_keys_jump_to_top_and_bottom() {
        let tmp = unique_temp_dir("nevi_markdown_preview_jump_keys");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let markdown_path = tmp.join("notes.md");
        std::fs::write(&markdown_path, "# Notes\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.set_size(80, 20);
        editor.open_file(markdown_path).expect("open markdown");
        editor.replace_buffer_content(&(0..50).map(|i| format!("line {i}\n")).collect::<String>());
        editor.open_markdown_preview().expect("open preview");

        handle_key(
            &mut editor,
            KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE),
        );
        let visible_rows = Terminal::markdown_preview_visible_rows(&editor).max(1);
        let max_scroll = editor
            .markdown_preview
            .as_ref()
            .unwrap()
            .max_scroll(visible_rows);
        assert_eq!(editor.markdown_preview.as_ref().unwrap().scroll, max_scroll);

        handle_key(
            &mut editor,
            KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
        );
        assert_eq!(editor.markdown_preview.as_ref().unwrap().scroll, 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn markdown_preview_footer_includes_position_and_jump_hint() {
        assert_eq!(
            Terminal::markdown_preview_footer(12, 42, 10),
            " j/k scroll • Ctrl-d/u page • g/G top/bottom • 13-22/42 • q close "
        );
        assert_eq!(
            Terminal::markdown_preview_footer(0, 1, 10),
            " j/k scroll • Ctrl-d/u page • g/G top/bottom • 1/1 • q close "
        );
    }

    #[test]
    fn markdown_preview_does_not_capture_mouse_while_overlay_is_open() {
        let tmp = unique_temp_dir("nevi_markdown_preview_mouse_capture");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let markdown_path = tmp.join("notes.md");
        std::fs::write(&markdown_path, "# Notes\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.open_file(markdown_path).expect("open markdown");
        assert!(!Terminal::should_capture_mouse(&editor));

        editor.open_markdown_preview().expect("open preview");
        assert!(!Terminal::should_capture_mouse(&editor));

        editor.close_markdown_preview();
        assert!(!Terminal::should_capture_mouse(&editor));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn markdown_preview_skips_background_redraw_while_overlay_is_open() {
        let tmp = unique_temp_dir("nevi_markdown_preview_skip_background");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let markdown_path = tmp.join("notes.md");
        std::fs::write(&markdown_path, "# Notes\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.open_file(markdown_path).expect("open markdown");
        assert!(!Terminal::should_skip_background(&editor));

        editor.open_markdown_preview().expect("open preview");
        assert!(Terminal::should_skip_background(&editor));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn diagnostic_float_position_uses_active_pane_cursor_origin() {
        let mut editor = Editor::default();
        editor.set_size(120, 40);
        editor.replace_buffer_content("fn main() {\n    missing();\n}\n");
        editor.hsplit(None).expect("split");
        editor.open_explorer();
        editor.cursor.line = 1;
        editor.cursor.col = 4;

        let active_pane = &editor.panes()[editor.active_pane_idx()];
        assert!(active_pane.rect.x > 0, "explorer should offset pane x");
        assert!(
            active_pane.rect.y > 0,
            "horizontal split should offset pane y"
        );

        let line_num_width = editor.buffer().len_lines().to_string().len().max(3) as u16;
        let (popup_x, popup_y) = Terminal::diagnostic_float_position(&editor, 24, 5);

        assert_eq!(popup_x, active_pane.rect.x + 2 + line_num_width + 1 + 4);
        assert_eq!(popup_y, active_pane.rect.y + 2);
    }

    #[test]
    fn normal_ctrl_w_equal_equalizes_windows() {
        let mut editor = Editor::default();
        editor.set_size(100, 20);
        editor.vsplit(None).expect("split");

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('='));

        assert_eq!(editor.panes().len(), 2);
        assert_eq!(editor.status_message.as_deref(), Some("Windows equalized"));
    }

    #[test]
    fn normal_ctrl_w_width_keys_resize_and_maximize_vertical_windows() {
        let mut editor = Editor::default();
        editor.set_size(100, 20);
        editor.vsplit(None).expect("split");

        assert_eq!(editor.active_pane_idx(), 1);
        assert_eq!(editor.panes()[0].rect.width, 50);
        assert_eq!(editor.panes()[1].rect.width, 50);

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('>'));

        assert_eq!(editor.panes()[0].rect.width, 45);
        assert_eq!(editor.panes()[1].rect.width, 55);
        assert_eq!(editor.status_message.as_deref(), Some("Window resized"));

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('<'));

        assert_eq!(editor.panes()[0].rect.width, 50);
        assert_eq!(editor.panes()[1].rect.width, 50);

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('|'));

        assert_eq!(editor.panes()[0].rect.width, 10);
        assert_eq!(editor.panes()[1].rect.width, 90);
        assert_eq!(editor.status_message.as_deref(), Some("Window maximized"));

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('='));

        assert_eq!(editor.panes()[0].rect.width, 50);
        assert_eq!(editor.panes()[1].rect.width, 50);
    }

    #[test]
    fn normal_ctrl_w_height_keys_resize_and_maximize_horizontal_windows() {
        let mut editor = Editor::default();
        editor.set_size(100, 22);
        editor.hsplit(None).expect("split");

        assert_eq!(editor.active_pane_idx(), 1);
        assert_eq!(editor.panes()[0].rect.height, 10);
        assert_eq!(editor.panes()[1].rect.height, 10);

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('+'));

        assert_eq!(editor.panes()[0].rect.height, 5);
        assert_eq!(editor.panes()[1].rect.height, 15);
        assert_eq!(editor.status_message.as_deref(), Some("Window resized"));

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('-'));

        assert_eq!(editor.panes()[0].rect.height, 10);
        assert_eq!(editor.panes()[1].rect.height, 10);

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('_'));

        assert_eq!(editor.panes()[0].rect.height, 3);
        assert_eq!(editor.panes()[1].rect.height, 17);
        assert_eq!(editor.status_message.as_deref(), Some("Window maximized"));

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('='));

        assert_eq!(editor.panes()[0].rect.height, 10);
        assert_eq!(editor.panes()[1].rect.height, 10);
    }

    #[test]
    fn normal_ctrl_w_r_and_shift_r_rotate_windows() {
        let mut editor = Editor::default();
        editor.set_size(100, 20);
        editor.vsplit(None).expect("first split");
        editor.vsplit(None).expect("second split");

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('r'));

        assert_eq!(editor.status_message.as_deref(), Some("Windows rotated"));

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, shift_key('R'));

        assert_eq!(
            editor.status_message.as_deref(),
            Some("Windows rotated reverse")
        );
    }

    #[test]
    fn normal_ctrl_w_x_exchanges_window() {
        let mut editor = Editor::default();
        editor.set_size(100, 20);
        editor.vsplit(None).expect("first split");
        editor.vsplit(None).expect("second split");

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, key('x'));

        assert_eq!(editor.status_message.as_deref(), Some("Windows exchanged"));
    }

    #[test]
    fn normal_keymaps_do_not_intercept_pending_window_commands() {
        let mut settings = Settings::default();
        settings.keymap.normal.push(KeymapEntry {
            from: "H".to_string(),
            to: "^".to_string(),
        });
        settings.keymap.normal.push(KeymapEntry {
            from: "L".to_string(),
            to: "$".to_string(),
        });
        let mut editor = Editor::new(settings);
        editor.set_size(100, 20);
        editor.vsplit(None).expect("first split");
        editor.vsplit(None).expect("second split");

        assert_eq!(editor.active_pane_idx(), 2);

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, shift_key('H'));

        assert_eq!(editor.active_pane_idx(), 0);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Window moved far left")
        );

        handle_key(&mut editor, ctrl_key('w'));
        handle_key(&mut editor, shift_key('L'));

        assert_eq!(editor.active_pane_idx(), 2);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Window moved far right")
        );
    }

    #[test]
    fn diagnostic_float_position_tracks_cursor_column_inside_right_split() {
        let mut editor = Editor::default();
        editor.set_size(160, 40);
        editor.replace_buffer_content("const value = getMergedRepos();\n");
        editor.open_explorer();
        editor.vsplit(None).expect("split");
        editor.mode = Mode::Normal;
        editor.cursor.line = 0;
        editor.cursor.col = 14;

        let active_pane = &editor.panes()[editor.active_pane_idx()];
        assert!(active_pane.rect.x > editor.explorer.width);

        let line_num_width = editor.buffer().len_lines().to_string().len().max(3) as u16;
        let text_area_x = active_pane.rect.x + 2 + line_num_width + 1;
        let (popup_x, _) = Terminal::diagnostic_float_position(&editor, 24, 5);

        assert_eq!(popup_x, text_area_x + 14);
    }

    #[test]
    fn diagnostic_float_width_fits_active_pane_text_area() {
        let mut editor = Editor::default();
        editor.set_size(80, 24);
        editor.replace_buffer_content("fn main() {\n    let bodyStream = ;\n}\n");
        editor.vsplit(None).expect("split");
        editor.cursor.line = 1;

        let active_pane = &editor.panes()[editor.active_pane_idx()];
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3) as u16;
        let text_area_x = active_pane.rect.x + 2 + line_num_width + 1;

        let popup_width = Terminal::diagnostic_float_width(&editor, 80);

        assert!(popup_width >= 4);
        assert!(text_area_x + popup_width <= active_pane.rect.x + active_pane.rect.width);
    }

    #[test]
    fn diagnostic_float_position_does_not_shift_before_active_pane_for_wide_messages() {
        let mut editor = Editor::default();
        editor.set_size(80, 24);
        editor.replace_buffer_content("fn main() {\n    let bodyStream = ;\n}\n");
        editor.vsplit(None).expect("split");
        editor.cursor.line = 1;

        let active_pane = &editor.panes()[editor.active_pane_idx()];
        let line_num_width = editor.buffer().len_lines().to_string().len().max(3) as u16;
        let text_area_x = active_pane.rect.x + 2 + line_num_width + 1;
        let (popup_x, _) = Terminal::diagnostic_float_position(&editor, 41, 5);

        assert_eq!(popup_x, text_area_x);
    }

    #[test]
    fn quit_last_pane_refuses_when_hidden_buffer_is_dirty() {
        let tmp = unique_temp_dir("nevi_quit_dirty_hidden");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let dirty = tmp.join("dirty.rs");
        let clean = tmp.join("clean.rs");
        std::fs::write(&dirty, "fn dirty() {}\n").expect("write dirty");
        std::fs::write(&clean, "fn clean() {}\n").expect("write clean");

        let mut editor = Editor::default();
        editor.open_file(dirty).expect("open dirty");
        editor.buffer_mut().insert_char(0, 0, 'x');
        editor.open_file(clean).expect("open clean");

        execute_command(&mut editor, Command::Quit);

        assert!(!editor.should_quit);
        assert!(editor
            .status_message
            .as_deref()
            .unwrap_or_default()
            .contains("No write since last change"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn buffer_delete_closes_clean_buffer_and_refuses_dirty_buffer_without_force() {
        let tmp = unique_temp_dir("nevi_buffer_delete");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let dirty = tmp.join("dirty.rs");
        let clean = tmp.join("clean.rs");
        std::fs::write(&dirty, "fn dirty() {}\n").expect("write dirty");
        std::fs::write(&clean, "fn clean() {}\n").expect("write clean");

        let mut editor = Editor::default();
        editor.open_file(dirty.clone()).expect("open dirty");
        editor.buffer_mut().insert_char(0, 0, 'x');
        editor.open_file(clean.clone()).expect("open clean");
        assert_eq!(editor.buffer_count(), 2);

        execute_command(&mut editor, Command::BufferDelete(false));

        assert_eq!(editor.buffer_count(), 1);
        assert_eq!(editor.buffer().path.as_ref(), Some(&dirty));

        execute_command(&mut editor, Command::BufferDelete(false));

        assert_eq!(editor.buffer_count(), 1);
        assert_eq!(editor.buffer().path.as_ref(), Some(&dirty));
        assert!(editor
            .status_message
            .as_deref()
            .unwrap_or_default()
            .contains("No write since last change"));

        execute_command(&mut editor, Command::BufferDelete(true));

        assert_eq!(editor.buffer_count(), 1);
        assert!(editor.buffer().path.is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn active_completion_prefix_changes_request_lsp_refresh() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("us\n");
        editor.mode = Mode::Insert;
        editor.cursor.line = 0;
        editor.cursor.col = 2;
        editor.show_completions(vec![completion_item("useEffect")], 0, 0, false);
        editor.update_completion_filter("us");
        editor.needs_completion_refresh = false;

        handle_insert_mode(&mut editor, key('E'));

        assert_eq!(editor.buffer().content(), "usE\n");
        assert_eq!(editor.completion.filter_text, "usE");
        assert!(editor.needs_completion_refresh);
    }

    #[test]
    fn insert_ctrl_t_indents_current_line_and_keeps_insert_position() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("  alpha\nbeta\n");
        editor.mode = Mode::Insert;
        editor.cursor.line = 0;
        editor.cursor.col = 7;

        handle_insert_mode(&mut editor, ctrl_key('t'));

        assert_eq!(editor.buffer().content(), "      alpha\nbeta\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 11));
    }

    #[test]
    fn insert_ctrl_d_dedents_current_line_and_keeps_insert_position() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("      alpha\nbeta\n");
        editor.mode = Mode::Insert;
        editor.cursor.line = 0;
        editor.cursor.col = 11;

        handle_insert_mode(&mut editor, ctrl_key('d'));

        assert_eq!(editor.buffer().content(), "  alpha\nbeta\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 7));
    }

    #[test]
    fn finder_preview_match_ranges_are_literal_case_insensitive() {
        assert_eq!(
            finder_preview_match_ranges("main Main remain", "MAIN"),
            vec![(0, 4), (5, 9), (12, 16)]
        );
    }

    #[test]
    fn finder_preview_match_ranges_skip_overlapping_matches() {
        assert_eq!(
            finder_preview_match_ranges("aaaa", "aa"),
            vec![(0, 2), (2, 4)]
        );
    }

    #[test]
    fn finder_insert_mode_navigation_escape_and_close_keys_follow_documented_behavior() {
        let mut editor = Editor::default();
        editor.finder.open_buffers(vec![
            (0, "alpha.rs".to_string(), PathBuf::from("alpha.rs")),
            (1, "beta.rs".to_string(), PathBuf::from("beta.rs")),
            (2, "gamma.rs".to_string(), PathBuf::from("gamma.rs")),
        ]);
        editor.mode = Mode::Finder;

        handle_key(&mut editor, ctrl_key('k'));
        assert_eq!(editor.finder.selected, 1);
        handle_key(&mut editor, ctrl_key('j'));
        assert_eq!(editor.finder.selected, 0);

        handle_key(&mut editor, ctrl_key('n'));
        assert_eq!(editor.finder.selected, 0);
        handle_key(&mut editor, ctrl_key('p'));
        assert_eq!(editor.finder.selected, 1);

        handle_key(&mut editor, key('b'));
        assert_eq!(editor.finder.query, "b");
        assert_eq!(editor.finder.cursor, 1);
        assert_eq!(editor.finder.selected, 0);
        assert_eq!(
            editor
                .finder
                .selected_item()
                .map(|item| item.display.as_str()),
            Some("2: beta.rs")
        );

        handle_key(&mut editor, esc_key());
        assert!(editor.finder.is_normal_mode());
        assert_eq!(editor.mode, Mode::Finder);

        handle_key(&mut editor, ctrl_key('c'));
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn finder_normal_mode_navigation_preview_and_insert_keys_follow_documented_behavior() {
        let mut editor = Editor::default();
        editor.finder.open_buffers(vec![
            (0, "alpha.rs".to_string(), PathBuf::from("alpha.rs")),
            (1, "beta.rs".to_string(), PathBuf::from("beta.rs")),
            (2, "gamma.rs".to_string(), PathBuf::from("gamma.rs")),
        ]);
        editor.mode = Mode::Finder;
        editor.finder.enter_normal_mode();

        handle_key(&mut editor, shift_key('G'));
        assert_eq!(editor.finder.selected, 2);
        handle_key(&mut editor, key('j'));
        assert_eq!(editor.finder.selected, 1);
        handle_key(&mut editor, key('k'));
        assert_eq!(editor.finder.selected, 2);
        handle_key(&mut editor, key('g'));
        assert_eq!(editor.finder.selected, 0);

        assert!(!editor.finder.preview_enabled);
        handle_key(&mut editor, key('p'));
        assert!(editor.finder.preview_enabled);

        handle_key(&mut editor, key('i'));
        assert!(!editor.finder.is_normal_mode());

        handle_key(&mut editor, esc_key());
        assert!(editor.finder.is_normal_mode());
        handle_key(&mut editor, esc_key());
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn finder_preview_ctrl_d_and_ctrl_u_scroll_preview_without_changing_query_or_selection() {
        let mut editor = Editor::default();
        editor.set_size(100, 30);
        editor.finder.open_harpoon(vec![PathBuf::from("alpha.rs")]);
        editor.mode = Mode::Finder;
        editor.finder.preview_enabled = true;
        editor
            .finder
            .set_preview_content(PathBuf::from("alpha.rs"), numbered_lines(40));

        handle_key(&mut editor, ctrl_key('d'));

        assert_eq!(editor.finder.preview_scroll, 8);
        assert_eq!(editor.finder.query, "");
        assert_eq!(editor.finder.selected, 0);

        handle_key(&mut editor, ctrl_key('u'));

        assert_eq!(editor.finder.preview_scroll, 0);
        assert_eq!(editor.finder.query, "");
        assert_eq!(editor.finder.selected, 0);
    }

    #[test]
    fn finder_enter_opens_selected_buffer_item() {
        let tmp = unique_temp_dir("nevi_finder_buffer_select");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let first = tmp.join("first.rs");
        let second = tmp.join("second.rs");
        std::fs::write(&first, "fn first() {}\n").expect("write first");
        std::fs::write(&second, "fn second() {}\n").expect("write second");

        let mut editor = Editor::default();
        editor.open_file(first).expect("open first");
        editor.open_file(second).expect("open second");
        assert_eq!(editor.current_buffer_index(), 1);

        editor.open_finder_buffers();
        handle_key(
            &mut editor,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.current_buffer_index(), 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn git_changes_enter_opens_existing_file() {
        let tmp = unique_temp_dir("nevi_git_changes_enter_open");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("changed.rs");
        std::fs::write(&path, "fn changed() {}\n").expect("write file");

        let mut editor = Editor::default();
        editor
            .finder
            .open_git_changes(vec![crate::finder::FinderItem::new(
                "M changed.rs".to_string(),
                path.clone(),
            )
            .with_git_status(crate::git::GitFileStatus::Modified)]);
        editor.mode = Mode::Finder;

        handle_key(
            &mut editor,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.buffer().path.as_ref(), Some(&path));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn git_changes_enter_refuses_deleted_file() {
        let tmp = unique_temp_dir("nevi_git_changes_enter_deleted");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("deleted.rs");

        let mut editor = Editor::default();
        editor
            .finder
            .open_git_changes(vec![crate::finder::FinderItem::new(
                "D deleted.rs".to_string(),
                path,
            )
            .with_git_status(crate::git::GitFileStatus::Deleted)]);
        editor.mode = Mode::Finder;

        handle_key(
            &mut editor,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.status_message.as_deref(), Some("File was deleted"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn normal_gf_opens_file_under_cursor_relative_to_current_file() {
        let tmp = unique_temp_dir("nevi_gf_file_under_cursor");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let current = tmp.join("current.txt");
        let target = tmp.join("target.txt");
        std::fs::write(&current, "open target.txt now\n").expect("write current");
        std::fs::write(&target, "target file\n").expect("write target");

        let mut editor = Editor::default();
        editor.open_file(current.clone()).expect("open current");
        editor.cursor.line = 0;
        editor.cursor.col = 7;

        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('f'));

        assert_eq!(editor.buffer().path.as_ref(), Some(&target));
        assert_eq!(editor.buffer().content(), "target file\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn normal_gx_reports_missing_url_under_cursor() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("not a url\n");
        editor.cursor.line = 0;
        editor.cursor.col = 2;

        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('x'));

        assert_eq!(
            editor.status_message.as_deref(),
            Some("No URL under cursor")
        );
    }

    #[test]
    fn normal_g_shift_d_sets_goto_declaration_lsp_action() {
        let mut editor = Editor::default();

        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, shift_key('D'));

        assert_eq!(
            format!("{:?}", editor.pending_lsp_action),
            "Some(GotoDeclaration)"
        );
    }

    #[test]
    fn normal_g_shift_i_sets_goto_implementation_lsp_action() {
        let mut editor = Editor::default();

        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, shift_key('I'));

        assert_eq!(
            format!("{:?}", editor.pending_lsp_action),
            "Some(GotoImplementation)"
        );
    }

    #[test]
    fn normal_apostrophe_apostrophe_jumps_to_previous_jump_line_first_non_blank() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("current\n  previous spot\n");
        editor.cursor.line = 0;
        editor.cursor.col = 3;
        editor.previous_jump_position = Some((None, 1, 11));

        handle_key(&mut editor, key('\''));
        handle_key(&mut editor, key('\''));

        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 2));
    }

    #[test]
    fn normal_backtick_backtick_jumps_to_previous_jump_exact_position() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("current\n  previous spot\n");
        editor.cursor.line = 0;
        editor.cursor.col = 3;
        editor.previous_jump_position = Some((None, 1, 11));

        handle_key(&mut editor, key('`'));
        handle_key(&mut editor, key('`'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 11));
    }

    #[test]
    fn normal_apostrophe_dot_jumps_to_last_change_line_first_non_blank() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("current\n  changed spot\n");
        editor.cursor.line = 0;
        editor.cursor.col = 3;
        editor.change_list.record(1, 10);

        handle_key(&mut editor, key('\''));
        handle_key(&mut editor, key('.'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 2));
    }

    #[test]
    fn normal_backtick_dot_jumps_to_last_change_exact_position() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("current\n  changed spot\n");
        editor.cursor.line = 0;
        editor.cursor.col = 3;
        editor.change_list.record(1, 10);

        handle_key(&mut editor, key('`'));
        handle_key(&mut editor, key('.'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 10));
    }

    #[test]
    fn normal_apostrophe_caret_jumps_to_last_insert_line_first_non_blank() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("current\n  insert spot\n");
        editor.cursor.line = 0;
        editor.cursor.col = 3;
        editor.last_insert_position = Some((1, 10));

        handle_key(&mut editor, key('\''));
        handle_key(&mut editor, key('^'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 2));
    }

    #[test]
    fn normal_backtick_caret_jumps_to_last_insert_exact_position() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("current\n  insert spot\n");
        editor.cursor.line = 0;
        editor.cursor.col = 3;
        editor.last_insert_position = Some((1, 10));

        handle_key(&mut editor, key('`'));
        handle_key(&mut editor, key('^'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 10));
    }

    #[test]
    fn harpoon_finder_delete_and_reorder_keys_follow_documented_behavior() {
        let mut editor = Editor::default();
        let one = PathBuf::from("/tmp/one.rs");
        let two = PathBuf::from("/tmp/two.rs");
        let three = PathBuf::from("/tmp/three.rs");
        editor.harpoon.add_file(&one);
        editor.harpoon.add_file(&two);
        editor.harpoon.add_file(&three);

        editor.open_finder_harpoon();
        editor.finder.selected = 1;
        handle_key(&mut editor, shift_key('K'));
        assert_eq!(
            editor.harpoon.files(),
            [two.clone(), one.clone(), three.clone()]
        );
        assert_eq!(editor.finder.selected, 0);

        handle_key(&mut editor, shift_key('J'));
        assert_eq!(
            editor.harpoon.files(),
            [one.clone(), two.clone(), three.clone()]
        );
        assert_eq!(editor.finder.selected, 1);

        handle_key(&mut editor, key('d'));
        assert_eq!(editor.harpoon.files(), [one, three]);
        assert_eq!(editor.mode, Mode::Finder);
    }

    #[test]
    fn normal_tilde_toggles_counted_chars() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("aBcD\n");

        handle_key(&mut editor, key('3'));
        handle_key(&mut editor, shift_key('~'));

        assert_eq!(editor.buffer().content(), "AbCD\n");
    }

    #[test]
    fn normal_counted_x_deletes_multiple_chars() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abcd\n");

        handle_key(&mut editor, key('3'));
        handle_key(&mut editor, key('x'));

        assert_eq!(editor.buffer().content(), "d\n");
    }

    #[test]
    fn normal_plus_minus_move_to_first_non_blank_on_target_line() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("zero\n    one\n  two\nthree\n");
        editor.cursor.line = 0;
        editor.cursor.col = 2;

        handle_key(&mut editor, key('+'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 4));

        handle_key(&mut editor, key('+'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (2, 2));

        handle_key(&mut editor, key('-'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 4));

        let mut counted = Editor::default();
        counted.replace_buffer_content("zero\n    one\n  two\nthree\n");
        counted.cursor.line = 0;
        counted.cursor.col = 2;

        handle_key(&mut counted, key('2'));
        handle_key(&mut counted, key('+'));

        assert_eq!((counted.cursor.line, counted.cursor.col), (2, 2));
    }

    #[test]
    fn normal_gj_gk_move_by_wrapped_display_lines() {
        let mut editor = Editor::default();
        editor.set_size(40, 12);
        editor.settings.editor.wrap = true;
        editor.settings.editor.wrap_width = 5;
        editor.settings.editor.line_numbers = false;
        editor.replace_buffer_content("abcdefghij\nklmno\n");
        editor.cursor.line = 0;
        editor.cursor.col = 1;

        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('j'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 6));

        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('j'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 1));

        handle_key(&mut editor, key('2'));
        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('k'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 1));
    }

    #[test]
    fn normal_g0_g_dollar_g_caret_target_current_wrapped_row() {
        let mut editor = Editor::default();
        editor.set_size(40, 12);
        editor.settings.editor.wrap = true;
        editor.settings.editor.wrap_width = 5;
        editor.settings.editor.line_numbers = false;
        editor.replace_buffer_content("  abcdefghij\n");
        editor.cursor.line = 0;

        editor.cursor.col = 6;
        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('0'));
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 5));

        editor.cursor.col = 6;
        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('$'));
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 7));

        editor.cursor.col = 6;
        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('^'));
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 5));
    }

    #[test]
    fn normal_equal_equal_auto_indents_current_line() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("fn main() {\nprintln!(\"hi\");\n}\n");
        editor.cursor.line = 1;

        handle_key(&mut editor, key('='));
        handle_key(&mut editor, key('='));

        assert_eq!(
            editor.buffer().content(),
            "fn main() {\n    println!(\"hi\");\n}\n"
        );
        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 4));
    }

    #[test]
    fn normal_equal_motion_auto_indents_motion_line_range() {
        let mut editor = Editor::default();
        editor.replace_buffer_content(
            "fn main() {\nprintln!(\"hi\");\nif true {\nprintln!(\"nested\");\n}\n}\n",
        );
        editor.cursor.line = 1;

        handle_key(&mut editor, key('='));
        handle_key(&mut editor, key('j'));

        assert_eq!(
            editor.buffer().content(),
            "fn main() {\n    println!(\"hi\");\n    if true {\nprintln!(\"nested\");\n}\n}\n"
        );
        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 4));
    }

    #[test]
    fn normal_gn_selects_next_search_match() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("alpha target beta\ntarget gamma\n");
        editor.search.last_pattern = Some("target".to_string());
        editor.cursor.line = 0;
        editor.cursor.col = 0;

        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('n'));

        assert_eq!(editor.mode, Mode::Visual);
        assert_eq!(editor.get_visual_range(), (0, 6, 0, 11));

        handle_key(&mut editor, key('y'));

        assert_eq!(
            editor.registers.get(None),
            Some(&RegisterContent::Chars("target".to_string()))
        );
    }

    #[test]
    fn normal_g_shift_n_selects_previous_search_match() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("target alpha\nmiddle\ngamma target\n");
        editor.search.last_pattern = Some("target".to_string());
        editor.cursor.line = 1;
        editor.cursor.col = 0;

        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, shift_key('N'));

        assert_eq!(editor.mode, Mode::Visual);
        assert_eq!(editor.get_visual_range(), (0, 0, 0, 5));

        handle_key(&mut editor, key('y'));

        assert_eq!(
            editor.registers.get(None),
            Some(&RegisterContent::Chars("target".to_string()))
        );
    }

    #[test]
    fn normal_paren_motions_move_between_sentence_starts() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("First sentence. Second sentence! Third sentence?\n");
        editor.cursor.line = 0;
        editor.cursor.col = 3;

        handle_key(&mut editor, key(')'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 16));

        handle_key(&mut editor, key(')'));

        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 33));

        handle_key(&mut editor, key('('));

        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 16));

        handle_key(&mut editor, key('('));

        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));

        let mut counted = Editor::default();
        counted.replace_buffer_content("First sentence. Second sentence! Third sentence?\n");

        handle_key(&mut counted, key('2'));
        handle_key(&mut counted, key(')'));

        assert_eq!((counted.cursor.line, counted.cursor.col), (0, 33));
    }

    #[test]
    fn normal_shift_c_keeps_insert_point_at_deleted_range_start() {
        let mut editor = Editor::default();
        let prefix = "EXPO_PUBLIC_API_URL=";
        editor.replace_buffer_content(&format!("{prefix}https://example.test\n"));
        editor.cursor.col = prefix.chars().count();

        handle_key(&mut editor, shift_key('C'));

        assert_eq!(editor.buffer().content(), format!("{prefix}\n"));
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor.col, prefix.chars().count());

        handle_key(&mut editor, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.col, prefix.chars().count() - 1);
    }

    #[test]
    fn normal_shift_d_deletes_to_eol_and_stays_on_last_remaining_char() {
        let mut editor = Editor::default();
        let prefix = "EXPO_PUBLIC_API_URL=";
        editor.replace_buffer_content(&format!("{prefix}https://example.test\n"));
        editor.cursor.col = prefix.chars().count();

        handle_key(&mut editor, shift_key('D'));

        assert_eq!(editor.buffer().content(), format!("{prefix}\n"));
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.col, prefix.chars().count() - 1);
    }

    #[test]
    fn normal_c_dollar_keeps_insert_point_at_eol_without_trailing_newline() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abc");
        editor.cursor.col = 2;

        handle_key(&mut editor, key('c'));
        handle_key(&mut editor, key('$'));

        assert_eq!(editor.buffer().content(), "ab");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor.col, 2);

        handle_key(&mut editor, esc_key());

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.col, 1);
    }

    #[test]
    fn normal_s_substitutes_counted_chars_and_enters_insert() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abcdef\n");
        editor.cursor.col = 2;

        handle_key(&mut editor, key('3'));
        handle_key(&mut editor, key('s'));

        assert_eq!(editor.buffer().content(), "abf\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor.col, 2);

        handle_key(&mut editor, key('X'));
        handle_key(&mut editor, esc_key());

        assert_eq!(editor.buffer().content(), "abXf\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.col, 2);
    }

    #[test]
    fn normal_s_on_empty_line_enters_insert() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("\n");

        handle_key(&mut editor, key('s'));
        handle_key(&mut editor, key('x'));
        handle_key(&mut editor, esc_key());

        assert_eq!(editor.buffer().content(), "x\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.col, 0);
    }

    #[test]
    fn normal_shift_s_changes_current_line() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abc\nnext\n");
        editor.cursor.col = 1;

        handle_key(&mut editor, shift_key('S'));

        assert_eq!(editor.buffer().content(), "\nnext\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor.col, 0);

        handle_key(&mut editor, key('X'));
        handle_key(&mut editor, esc_key());

        assert_eq!(editor.buffer().content(), "X\nnext\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.col, 0);
    }

    #[test]
    fn normal_insert_entry_commands_keep_expected_positions() {
        let mut append = Editor::default();
        append.replace_buffer_content("  abc\n");
        handle_key(&mut append, shift_key('A'));
        assert_eq!(append.mode, Mode::Insert);
        assert_eq!(append.cursor.col, 5);
        handle_key(&mut append, esc_key());
        assert_eq!(append.cursor.col, 4);

        let mut insert = Editor::default();
        insert.replace_buffer_content("  abc\n");
        insert.cursor.col = 4;
        handle_key(&mut insert, shift_key('I'));
        assert_eq!(insert.mode, Mode::Insert);
        assert_eq!(insert.cursor.col, 2);

        let mut below = Editor::default();
        below.replace_buffer_content("  abc\nnext\n");
        handle_key(&mut below, key('o'));
        assert_eq!(below.buffer().content(), "  abc\n  \nnext\n");
        assert_eq!(below.mode, Mode::Insert);
        assert_eq!((below.cursor.line, below.cursor.col), (1, 2));

        let mut above = Editor::default();
        above.replace_buffer_content("  abc\nnext\n");
        above.cursor.line = 1;
        handle_key(&mut above, shift_key('O'));
        assert_eq!(above.buffer().content(), "  abc\n\nnext\n");
        assert_eq!(above.mode, Mode::Insert);
        assert_eq!((above.cursor.line, above.cursor.col), (1, 0));
    }

    #[test]
    fn visual_mode_entry_toggle_exit_and_reselect_keys_manage_selection_modes() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abc\ndef\n");

        handle_key(&mut editor, key('v'));
        assert_eq!(editor.mode, Mode::Visual);
        assert_eq!(
            (editor.visual.anchor_line, editor.visual.anchor_col),
            (0, 0)
        );

        handle_key(&mut editor, key('l'));
        assert_eq!(editor.get_visual_range(), (0, 0, 0, 1));

        handle_key(&mut editor, key('o'));
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));
        assert_eq!(
            (editor.visual.anchor_line, editor.visual.anchor_col),
            (0, 1)
        );
        assert_eq!(editor.get_visual_range(), (0, 0, 0, 1));

        handle_key(&mut editor, esc_key());
        assert_eq!(editor.mode, Mode::Normal);

        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('v'));
        assert_eq!(editor.mode, Mode::Visual);
        assert_eq!(editor.get_visual_range(), (0, 0, 0, 1));

        handle_key(&mut editor, key('v'));
        assert_eq!(editor.mode, Mode::Normal);

        handle_key(&mut editor, shift_key('V'));
        assert_eq!(editor.mode, Mode::VisualLine);
        handle_key(&mut editor, shift_key('V'));
        assert_eq!(editor.mode, Mode::Normal);

        handle_key(&mut editor, ctrl_key('v'));
        assert_eq!(editor.mode, Mode::VisualBlock);
        handle_key(&mut editor, ctrl_key('v'));
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn visual_block_shift_o_moves_to_other_corner_on_cursor_line() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abcd\nefgh\n");

        handle_key(&mut editor, ctrl_key('v'));
        handle_key(&mut editor, key('j'));
        handle_key(&mut editor, key('l'));
        handle_key(&mut editor, key('l'));
        assert_eq!(editor.mode, Mode::VisualBlock);
        assert_eq!(
            (editor.visual.anchor_line, editor.visual.anchor_col),
            (0, 0)
        );
        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 2));
        assert_eq!(editor.get_visual_range(), (0, 0, 1, 2));

        handle_key(&mut editor, shift_key('O'));

        assert_eq!(editor.mode, Mode::VisualBlock);
        assert_eq!(
            (editor.visual.anchor_line, editor.visual.anchor_col),
            (0, 2)
        );
        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 0));
        assert_eq!(editor.get_visual_range(), (0, 0, 1, 2));
    }

    #[test]
    fn visual_block_i_inserts_typed_text_on_each_selected_line() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("alpha\nbravo\ncharlie\n");
        editor.cursor.col = 1;

        handle_key(&mut editor, ctrl_key('v'));
        handle_key(&mut editor, key('j'));
        handle_key(&mut editor, key('j'));
        handle_key(&mut editor, shift_key('I'));

        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 1));

        handle_key(&mut editor, key('X'));
        handle_key(&mut editor, key('Y'));
        handle_key(&mut editor, esc_key());

        assert_eq!(editor.buffer().content(), "aXYlpha\nbXYravo\ncXYharlie\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 2));

        handle_key(&mut editor, key('u'));

        assert_eq!(editor.buffer().content(), "alpha\nbravo\ncharlie\n");
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 1));
    }

    #[test]
    fn visual_block_a_appends_typed_text_after_each_selected_block() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("alpha\nbravo\ncharlie\n");
        editor.cursor.col = 1;

        handle_key(&mut editor, ctrl_key('v'));
        handle_key(&mut editor, key('j'));
        handle_key(&mut editor, key('j'));
        handle_key(&mut editor, key('l'));
        handle_key(&mut editor, shift_key('A'));

        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 3));

        handle_key(&mut editor, key('_'));
        handle_key(&mut editor, esc_key());

        assert_eq!(editor.buffer().content(), "alp_ha\nbra_vo\ncha_rlie\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 3));
    }

    #[test]
    fn visual_yank_and_delete_keys_update_registers_and_buffer() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("hello world\n");

        handle_key(&mut editor, key('v'));
        handle_key(&mut editor, key('e'));
        handle_key(&mut editor, key('y'));

        assert_eq!(editor.buffer().content(), "hello world\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(
            editor.registers.get(None),
            Some(&RegisterContent::Chars("hello".to_string()))
        );

        editor.cursor.col = 6;
        handle_key(&mut editor, key('v'));
        handle_key(&mut editor, key('e'));
        handle_key(&mut editor, key('d'));

        assert_eq!(editor.buffer().content(), "hello \n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(
            editor.registers.get(None),
            Some(&RegisterContent::Chars("world".to_string()))
        );
    }

    #[test]
    fn visual_change_keeps_insert_point_at_selection_start() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("foo bar\n");
        editor.cursor.col = 4;

        editor.enter_visual_mode();
        editor.apply_motion(Motion::WordEnd, 1);
        handle_key(&mut editor, key('c'));

        assert_eq!(editor.buffer().content(), "foo \n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor.col, 4);

        handle_key(&mut editor, key('X'));
        handle_key(&mut editor, esc_key());

        assert_eq!(editor.buffer().content(), "foo X\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.col, 4);
    }

    #[test]
    fn visual_s_changes_selection_like_c() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("foo bar\n");
        editor.cursor.col = 4;

        editor.enter_visual_mode();
        editor.apply_motion(Motion::WordEnd, 1);
        handle_key(&mut editor, key('s'));

        assert_eq!(editor.buffer().content(), "foo \n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor.col, 4);
    }

    #[test]
    fn visual_indent_dedent_and_comment_keys_operate_on_selected_lines() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("one\ntwo\nthree\n");

        handle_key(&mut editor, shift_key('V'));
        handle_key(&mut editor, key('j'));
        handle_key(&mut editor, key('>'));

        assert_eq!(editor.buffer().content(), "    one\n    two\nthree\n");
        assert_eq!(editor.mode, Mode::Normal);

        handle_key(&mut editor, shift_key('V'));
        handle_key(&mut editor, key('j'));
        handle_key(&mut editor, key('<'));

        assert_eq!(editor.buffer().content(), "one\ntwo\nthree\n");
        assert_eq!(editor.mode, Mode::Normal);

        let mut comments = Editor::default();
        comments.replace_buffer_content("one\ntwo\nthree\n");

        handle_key(&mut comments, shift_key('V'));
        handle_key(&mut comments, key('j'));
        handle_key(&mut comments, key('g'));
        handle_key(&mut comments, key('c'));

        assert_eq!(comments.buffer().content(), "// one\n// two\nthree\n");
        assert_eq!(comments.mode, Mode::Normal);
    }

    #[test]
    fn visual_case_keys_transform_selection_and_exit_to_normal() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("FOO Bar\n");

        handle_key(&mut editor, key('v'));
        handle_key(&mut editor, key('e'));
        handle_key(&mut editor, key('u'));

        assert_eq!(editor.buffer().content(), "foo Bar\n");
        assert_eq!(editor.mode, Mode::Normal);

        editor.cursor.col = 4;
        handle_key(&mut editor, key('v'));
        handle_key(&mut editor, key('e'));
        handle_key(&mut editor, shift_key('U'));

        assert_eq!(editor.buffer().content(), "foo BAR\n");
        assert_eq!(editor.mode, Mode::Normal);

        editor.cursor.col = 0;
        handle_key(&mut editor, key('v'));
        handle_key(&mut editor, key('e'));
        handle_key(&mut editor, shift_key('~'));

        assert_eq!(editor.buffer().content(), "FOO BAR\n");
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn visual_text_object_keys_select_and_yank_documented_objects() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("foo (bar baz) qux\n");
        editor.cursor.col = 6;

        handle_key(&mut editor, key('v'));
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('('));
        assert_eq!(editor.get_visual_range(), (0, 5, 0, 11));

        handle_key(&mut editor, key('y'));
        assert_eq!(
            editor.registers.get(None),
            Some(&RegisterContent::Chars("bar baz".to_string()))
        );
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn normal_dip_deletes_inner_paragraph_without_blank_separator() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("one\n\ntwo\nthree\n\nfour\n");
        editor.cursor.line = 2;
        editor.cursor.col = 1;

        handle_key(&mut editor, key('d'));
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('p'));

        assert_eq!(editor.buffer().content(), "one\n\n\nfour\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (2, 0));
    }

    #[test]
    fn normal_dap_deletes_paragraph_and_following_blank_separator() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("one\n\ntwo\nthree\n\nfour\n");
        editor.cursor.line = 2;
        editor.cursor.col = 1;

        handle_key(&mut editor, key('d'));
        handle_key(&mut editor, key('a'));
        handle_key(&mut editor, key('p'));

        assert_eq!(editor.buffer().content(), "one\n\nfour\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (2, 0));
    }

    #[test]
    fn normal_dis_deletes_inner_sentence_without_separator() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("Alpha one. Beta two! Gamma three?\n");
        editor.cursor.line = 0;
        editor.cursor.col = 14;

        handle_key(&mut editor, key('d'));
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('s'));

        assert_eq!(editor.buffer().content(), "Alpha one.  Gamma three?\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 11));
    }

    #[test]
    fn normal_das_deletes_sentence_and_following_separator() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("Alpha one. Beta two! Gamma three?\n");
        editor.cursor.line = 0;
        editor.cursor.col = 14;

        handle_key(&mut editor, key('d'));
        handle_key(&mut editor, key('a'));
        handle_key(&mut editor, key('s'));

        assert_eq!(editor.buffer().content(), "Alpha one. Gamma three?\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 11));
    }

    #[test]
    fn visual_is_selects_and_yanks_inner_sentence() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("Alpha one. Beta two! Gamma three?\n");
        editor.cursor.line = 0;
        editor.cursor.col = 14;

        handle_key(&mut editor, key('v'));
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('s'));
        handle_key(&mut editor, key('y'));

        assert_eq!(
            editor.registers.get(None),
            Some(&RegisterContent::Chars("Beta two!".to_string()))
        );
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn normal_dit_deletes_inner_tag_text_object_with_attributes() {
        let mut editor = Editor::default();
        editor.replace_buffer_content(
            "before <div class=\"x\">hello <span>world</span></div> after\n",
        );
        editor.cursor.line = 0;
        editor.cursor.col = 24;

        handle_key(&mut editor, key('d'));
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('t'));

        assert_eq!(
            editor.buffer().content(),
            "before <div class=\"x\"></div> after\n"
        );
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 22));
    }

    #[test]
    fn normal_dat_deletes_around_tag_text_object_with_attributes() {
        let mut editor = Editor::default();
        editor.replace_buffer_content(
            "before <div class=\"x\">hello <span>world</span></div> after\n",
        );
        editor.cursor.line = 0;
        editor.cursor.col = 24;

        handle_key(&mut editor, key('d'));
        handle_key(&mut editor, key('a'));
        handle_key(&mut editor, key('t'));

        assert_eq!(editor.buffer().content(), "before  after\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 7));
    }

    #[test]
    fn normal_dit_prefers_nearest_nested_same_name_tag_text_object() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("<div>outer <div>inner</div> tail</div>\n");
        editor.cursor.line = 0;
        editor.cursor.col = 18;

        handle_key(&mut editor, key('d'));
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('t'));

        assert_eq!(
            editor.buffer().content(),
            "<div>outer <div></div> tail</div>\n"
        );
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 16));
    }

    #[test]
    fn visual_it_yanks_inner_tag_text_object() {
        let mut editor = Editor::default();
        editor.replace_buffer_content(
            "before <div class=\"x\">hello <span>world</span></div> after\n",
        );
        editor.cursor.line = 0;
        editor.cursor.col = 24;

        handle_key(&mut editor, key('v'));
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('t'));
        handle_key(&mut editor, key('y'));

        assert_eq!(
            editor.registers.get(None),
            Some(&RegisterContent::Chars(
                "hello <span>world</span>".to_string()
            ))
        );
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn change_text_object_keeps_insert_point_at_deleted_word_start() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("foo bar\n");
        editor.cursor.col = 5;

        handle_key(&mut editor, key('c'));
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('w'));

        assert_eq!(editor.buffer().content(), "foo \n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor.col, 4);

        handle_key(&mut editor, key('X'));
        handle_key(&mut editor, esc_key());

        assert_eq!(editor.buffer().content(), "foo X\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.col, 4);
    }

    #[test]
    fn completion_replacement_removes_word_suffix_after_cursor() {
        let mut editor = Editor::default();
        editor.buffer_mut().insert_str(0, 0, "use std::fs;\n");
        editor.cursor.line = 0;
        editor.cursor.col = "use st".chars().count();

        replace_completion_text(&mut editor, "use ".chars().count(), "std");

        assert_eq!(editor.buffer().content(), "use std::fs;\n");
        assert_eq!(editor.cursor.col, "use std".chars().count());
    }

    #[test]
    fn normal_counted_replace_replaces_multiple_chars() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abcd\n");

        handle_key(&mut editor, key('3'));
        handle_key(&mut editor, key('r'));
        handle_key(&mut editor, key('x'));

        assert_eq!(editor.buffer().content(), "xxxd\n");
    }

    #[test]
    fn normal_counted_named_paste_repeats_content() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("ab\n");
        editor
            .registers
            .yank(Some('a'), RegisterContent::Chars("x".to_string()));

        handle_key(&mut editor, key('"'));
        handle_key(&mut editor, key('a'));
        handle_key(&mut editor, key('3'));
        handle_key(&mut editor, key('p'));

        assert_eq!(editor.buffer().content(), "axxxb\n");
    }

    #[test]
    fn normal_quote_percent_pastes_current_filename() {
        let tmp = unique_temp_dir("nevi_current_filename_register");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("current.txt");
        std::fs::write(&path, "x\n").expect("write current file");

        let mut editor = Editor::default();
        editor.open_file(path.clone()).expect("open current file");
        editor.cursor.line = 0;
        editor.cursor.col = 0;

        handle_key(&mut editor, key('"'));
        handle_key(&mut editor, key('%'));
        handle_key(&mut editor, key('p'));

        let expected = format!("x{}\n", path.to_string_lossy());
        assert_eq!(editor.buffer().content(), expected);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn normal_quote_colon_pastes_last_command() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("x\n");
        editor.command_line.history = vec!["write".to_string(), "buffers".to_string()];
        editor.cursor.line = 0;
        editor.cursor.col = 0;

        handle_key(&mut editor, key('"'));
        handle_key(&mut editor, key(':'));
        handle_key(&mut editor, key('p'));

        assert_eq!(editor.buffer().content(), "xbuffers\n");
    }

    #[test]
    fn normal_quote_hash_pastes_alternate_filename() {
        let tmp = unique_temp_dir("nevi_alternate_filename_register");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let first = tmp.join("first.txt");
        let second = tmp.join("second.txt");
        std::fs::write(&first, "first\n").expect("write first");
        std::fs::write(&second, "x\n").expect("write second");

        let mut editor = Editor::default();
        editor.open_file(first.clone()).expect("open first");
        editor.open_file(second).expect("open second");
        editor.cursor.line = 0;
        editor.cursor.col = 0;

        handle_key(&mut editor, key('"'));
        handle_key(&mut editor, key('#'));
        handle_key(&mut editor, key('p'));

        let expected = format!("x{}\n", first.to_string_lossy());
        assert_eq!(editor.buffer().content(), expected);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn normal_quote_dot_pastes_last_inserted_text() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("x\n\n");

        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('n'));
        handle_key(&mut editor, key('e'));
        handle_key(&mut editor, key('v'));
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, esc_key());

        editor.cursor.line = 1;
        editor.cursor.col = 0;
        handle_key(&mut editor, key('"'));
        handle_key(&mut editor, key('.'));
        handle_key(&mut editor, key('p'));

        assert_eq!(editor.buffer().content(), "nevix\nnevi\n");
    }

    #[test]
    fn normal_expression_register_evaluates_expression_before_paste() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("x\n");

        handle_key(&mut editor, key('"'));
        handle_key(&mut editor, key('='));
        handle_key(&mut editor, key('1'));
        handle_key(&mut editor, key('+'));
        handle_key(&mut editor, key('2'));
        handle_key(&mut editor, key('*'));
        handle_key(&mut editor, key('3'));
        handle_key(
            &mut editor,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        handle_key(&mut editor, key('p'));

        assert_eq!(editor.buffer().content(), "x7\n");
    }

    #[test]
    fn insert_ctrl_a_inserts_previously_inserted_text() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("x\n\n");

        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, key('n'));
        handle_key(&mut editor, key('e'));
        handle_key(&mut editor, key('v'));
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, esc_key());

        editor.cursor.line = 1;
        editor.cursor.col = 0;
        handle_key(&mut editor, key('i'));
        handle_key(&mut editor, ctrl_key('a'));

        assert_eq!(editor.buffer().content(), "nevix\nnevi\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 4));
    }

    #[test]
    fn insert_ctrl_r_inserts_named_register_text() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("x\n");
        editor
            .registers
            .yank(Some('a'), RegisterContent::Chars("nevi".to_string()));

        handle_key(&mut editor, shift_key('A'));
        handle_key(&mut editor, ctrl_key('r'));
        handle_key(&mut editor, key('a'));

        assert_eq!(editor.buffer().content(), "xnevi\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 5));
    }

    #[test]
    fn insert_ctrl_r_equals_evaluates_expression_at_cursor() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("x\n");

        handle_key(&mut editor, shift_key('A'));
        handle_key(&mut editor, ctrl_key('r'));
        handle_key(&mut editor, key('='));
        handle_key(&mut editor, key('1'));
        handle_key(&mut editor, key('0'));
        handle_key(&mut editor, key('/'));
        handle_key(&mut editor, key('4'));
        handle_key(
            &mut editor,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert_eq!(editor.buffer().content(), "x2.5\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 4));
    }

    #[test]
    fn insert_ctrl_o_runs_one_normal_command_then_returns_to_insert() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abc\n");

        handle_key(&mut editor, shift_key('A'));
        handle_key(&mut editor, ctrl_key('o'));
        handle_key(&mut editor, key('0'));

        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));

        handle_key(&mut editor, shift_key('X'));

        assert_eq!(editor.buffer().content(), "Xabc\n");
        assert_eq!(editor.mode, Mode::Insert);
    }

    #[test]
    fn normal_gp_pastes_after_and_leaves_cursor_after_pasted_text() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("ab\n");
        editor
            .registers
            .yank(Some('a'), RegisterContent::Chars("xy".to_string()));

        handle_key(&mut editor, key('"'));
        handle_key(&mut editor, key('a'));
        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('p'));

        assert_eq!(editor.buffer().content(), "axyb\n");
        assert_eq!(editor.cursor.col, 3);
    }

    #[test]
    fn normal_g_shift_p_pastes_before_and_leaves_cursor_after_pasted_text() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("ab\n");
        editor.cursor.col = 1;
        editor
            .registers
            .yank(Some('a'), RegisterContent::Chars("xy".to_string()));

        handle_key(&mut editor, key('"'));
        handle_key(&mut editor, key('a'));
        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, shift_key('P'));

        assert_eq!(editor.buffer().content(), "axyb\n");
        assert_eq!(editor.cursor.col, 3);
    }

    #[test]
    fn normal_gp_linewise_paste_leaves_cursor_after_inserted_block() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("aa\nbb\ncc\n");
        editor.cursor.line = 1;
        editor
            .registers
            .yank(Some('a'), RegisterContent::Lines("xx\n".to_string()));

        handle_key(&mut editor, key('"'));
        handle_key(&mut editor, key('a'));
        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, key('p'));

        assert_eq!(editor.buffer().content(), "aa\nbb\nxx\ncc\n");
        assert_eq!((editor.cursor.line, editor.cursor.col), (3, 0));
    }

    #[test]
    fn normal_g_shift_p_linewise_paste_leaves_cursor_after_inserted_block() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("aa\nbb\ncc\n");
        editor.cursor.line = 1;
        editor
            .registers
            .yank(Some('a'), RegisterContent::Lines("xx\n".to_string()));

        handle_key(&mut editor, key('"'));
        handle_key(&mut editor, key('a'));
        handle_key(&mut editor, key('g'));
        handle_key(&mut editor, shift_key('P'));

        assert_eq!(editor.buffer().content(), "aa\nxx\nbb\ncc\n");
        assert_eq!((editor.cursor.line, editor.cursor.col), (2, 0));
    }

    #[test]
    fn normal_leader_popup_items_follow_current_prefix() {
        let mut editor = Editor::default();

        handle_key(&mut editor, key(' '));

        assert_eq!(editor.leader_sequence.as_deref(), Some(""));
        let root_items = editor.leader_popup_items();
        assert!(root_items
            .iter()
            .any(|item| item.key == "f" && item.has_children));
        assert!(root_items
            .iter()
            .any(|item| item.key == "w" && item.description == "Save file"));

        handle_key(&mut editor, key('f'));

        assert_eq!(editor.leader_sequence.as_deref(), Some("f"));
        let file_items = editor.leader_popup_items();
        assert!(file_items
            .iter()
            .any(|item| item.key == "f" && item.sequence == "ff"));
        assert!(file_items
            .iter()
            .any(|item| item.key == "k" && item.description == "Search keymaps"));
    }

    #[test]
    fn normal_leader_popup_items_respect_config_toggle() {
        let mut editor = Editor::default();
        editor.settings.keymap.show_leader_popup = false;

        handle_key(&mut editor, key(' '));

        assert_eq!(editor.leader_sequence.as_deref(), Some(""));
        assert!(editor.leader_popup_items().is_empty());
    }

    #[test]
    fn leader_popup_items_are_hidden_while_finder_owns_ui() {
        let mut editor = Editor::default();
        editor.mode = Mode::Finder;
        editor.leader_sequence = Some("f".to_string());

        assert!(editor.leader_popup_items().is_empty());
    }

    #[test]
    fn normal_leader_mapping_clears_popup_state_when_opening_finder() {
        let mut editor = Editor::default();

        handle_key(&mut editor, key(' '));
        handle_key(&mut editor, key('f'));
        handle_key(&mut editor, key('f'));

        assert_eq!(editor.mode, Mode::Finder);
        assert_eq!(editor.finder.mode, FinderMode::Files);
        assert!(editor.leader_sequence.is_none());
        assert!(editor.leader_popup_items().is_empty());
    }

    #[test]
    fn executing_leader_action_clears_stale_popup_state() {
        let mut editor = Editor::default();
        editor.leader_sequence = Some("f".to_string());
        editor.leader_sequence_start = Some(std::time::Instant::now());
        editor.leader_pending_action = editor.keymap.get_leader_action("f").cloned();
        let action = editor.keymap.get_leader_action("ff").cloned().unwrap();

        execute_leader_action(&mut editor, &action);

        assert_eq!(editor.mode, Mode::Finder);
        assert_eq!(editor.finder.mode, FinderMode::Files);
        assert!(editor.leader_sequence.is_none());
        assert!(editor.leader_popup_items().is_empty());
    }

    #[test]
    fn leader_terminal_rename_prefills_active_session_command() {
        let mut editor = Editor::default();
        editor
            .floating_terminal
            .create_session(Some("server".to_string()))
            .unwrap();

        let action = editor.keymap.get_leader_action("tr").cloned().unwrap();
        execute_leader_action(&mut editor, &action);

        assert_eq!(editor.mode, Mode::Command);
        assert_eq!(editor.command_line.input, "termrename 1 server");
    }

    #[test]
    fn normal_counted_join_joins_that_many_lines() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("a\nb\nc\nd\n");

        handle_key(&mut editor, key('3'));
        handle_key(&mut editor, shift_key('J'));

        assert_eq!(editor.buffer().content(), "a b c\nd\n");
    }

    #[test]
    fn visual_paste_replaces_selection() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("hello world\n");
        editor
            .registers
            .yank(Some('a'), RegisterContent::Chars("nevi".to_string()));

        editor.enter_visual_mode();
        editor.apply_motion(Motion::WordEnd, 1);
        editor.input_state.selected_register = Some('a');
        handle_key(&mut editor, key('p'));

        assert_eq!(editor.buffer().content(), "nevi world\n");
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn visual_surround_wraps_selection() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("hello\n");

        editor.enter_visual_mode();
        editor.apply_motion(Motion::WordEnd, 1);
        handle_key(&mut editor, shift_key('S'));
        handle_key(&mut editor, key('"'));

        assert_eq!(editor.buffer().content(), "\"hello\"\n");
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn visual_keymap_can_override_change_with_paste() {
        let mut settings = Settings::default();
        settings.keymap.visual.push(KeymapEntry {
            from: "s".to_string(),
            to: "p".to_string(),
        });
        let mut editor = Editor::new(settings);
        editor.replace_buffer_content("hello world\n");
        editor
            .registers
            .yank(Some('a'), RegisterContent::Chars("nevi".to_string()));

        editor.enter_visual_mode();
        editor.apply_motion(Motion::WordEnd, 1);
        editor.input_state.selected_register = Some('a');
        handle_key(&mut editor, key('s'));

        assert_eq!(editor.buffer().content(), "nevi world\n");
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn visual_keymap_can_override_change_with_surround() {
        let mut settings = Settings::default();
        settings.keymap.visual.push(KeymapEntry {
            from: "s".to_string(),
            to: "S".to_string(),
        });
        let mut editor = Editor::new(settings);
        editor.replace_buffer_content("hello\n");

        editor.enter_visual_mode();
        editor.apply_motion(Motion::WordEnd, 1);
        handle_key(&mut editor, key('s'));
        handle_key(&mut editor, key('"'));

        assert_eq!(editor.buffer().content(), "\"hello\"\n");
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn ys_line_surrounds_current_line() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("hello\n");

        handle_key(&mut editor, key('y'));
        handle_key(&mut editor, key('s'));
        handle_key(&mut editor, key('s'));
        handle_key(&mut editor, key(')'));

        assert_eq!(editor.buffer().content(), "(hello)\n");
    }

    #[test]
    fn ys_motion_surrounds_motion_range() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("hello world\n");

        handle_key(&mut editor, key('y'));
        handle_key(&mut editor, key('s'));
        handle_key(&mut editor, shift_key('$'));
        handle_key(&mut editor, key('"'));

        assert_eq!(editor.buffer().content(), "\"hello world\"\n");
    }
}
