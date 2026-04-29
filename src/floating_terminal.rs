//! Floating terminal implementation using PTY
//!
//! Provides a toggleable floating terminal window that runs the user's shell.

use alacritty_terminal::{
    event::VoidListener,
    grid::Dimensions,
    term::{
        cell::{Cell, Flags},
        color::Colors,
        point_to_viewport, Config, Term, TermMode,
    },
    vte::ansi::{
        Color as AlacrittyColor, NamedColor, Processor as VteProcessor, Rgb as AlacrittyRgb,
    },
};
use crossterm::style::Color as CrosstermColor;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// Terminal buffer size (scrollback)
const BUFFER_ROWS: usize = 1000;
const BUFFER_COLS: usize = 200;
const FLOATING_TERMINAL_RATIO: f32 = 0.6;
const MIN_POPUP_WIDTH: u16 = 40;
const MIN_POPUP_HEIGHT: u16 = 10;

/// Calculate the floating terminal popup size for the editor screen.
pub fn popup_size_for_screen(screen_width: u16, screen_height: u16) -> (u16, u16) {
    let width = ((screen_width as f32 * FLOATING_TERMINAL_RATIO) as u16)
        .max(MIN_POPUP_WIDTH)
        .min(screen_width.max(2));
    let height = ((screen_height as f32 * FLOATING_TERMINAL_RATIO) as u16)
        .max(MIN_POPUP_HEIGHT)
        .min(screen_height.max(2));

    (width, height)
}

/// Calculate the PTY content size for the floating terminal popup.
pub fn content_size_for_screen(screen_width: u16, screen_height: u16) -> (u16, u16) {
    let (popup_width, popup_height) = popup_size_for_screen(screen_width, screen_height);
    let rows = popup_height.saturating_sub(2).max(1);
    let cols = popup_width.saturating_sub(2).max(1);

    (rows.min(BUFFER_ROWS as u16), cols.min(BUFFER_COLS as u16))
}

#[derive(Clone, Copy)]
struct TerminalDimensions {
    rows: usize,
    cols: usize,
}

impl TerminalDimensions {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            rows: rows.max(1) as usize,
            cols: cols.max(2) as usize,
        }
    }
}

impl Dimensions for TerminalDimensions {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

/// A renderable terminal cell with the style resolved from Alacritty's grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalCell {
    pub ch: char,
    pub fg: Option<CrosstermColor>,
    pub bg: Option<CrosstermColor>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub double_underline: bool,
    pub undercurl: bool,
    pub underdotted: bool,
    pub underdashed: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub strikeout: bool,
}

impl Default for TerminalCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: None,
            bg: None,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            double_underline: false,
            undercurl: false,
            underdotted: false,
            underdashed: false,
            inverse: false,
            hidden: false,
            strikeout: false,
        }
    }
}

/// Floating terminal state
pub struct FloatingTerminal {
    /// Whether the terminal is currently visible
    pub visible: bool,
    /// Alacritty terminal emulator grid
    term: Term<VoidListener>,
    /// VTE parser feeding the emulator grid
    processor: VteProcessor,
    /// Terminal dimensions
    rows: u16,
    cols: u16,
    /// PTY master (for resizing)
    pty_master: Option<Box<dyn MasterPty + Send>>,
    /// PTY writer (for sending input) - taken once from master and reused
    pty_writer: Option<Box<dyn Write + Send>>,
    /// Child process
    child: Option<Box<dyn Child + Send + Sync>>,
    /// Reader thread output buffer
    output_buffer: Arc<Mutex<Vec<u8>>>,
    /// Reader thread handle (for joining on close)
    reader_thread: Option<JoinHandle<()>>,
    /// Working directory
    working_dir: PathBuf,
    /// Whether terminal process has exited
    process_exited: bool,
}

impl FloatingTerminal {
    /// Create a new floating terminal (not yet spawned)
    pub fn new() -> Self {
        let rows = 24;
        let cols = 80;

        Self {
            visible: false,
            term: Self::new_term(rows, cols),
            processor: VteProcessor::new(),
            rows,
            cols,
            pty_master: None,
            pty_writer: None,
            child: None,
            output_buffer: Arc::new(Mutex::new(Vec::new())),
            reader_thread: None,
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            process_exited: false,
        }
    }

    fn new_term(rows: u16, cols: u16) -> Term<VoidListener> {
        let mut config = Config::default();
        config.scrolling_history = BUFFER_ROWS;
        Term::new(config, &TerminalDimensions::new(rows, cols), VoidListener)
    }

    /// Set the working directory for the terminal
    pub fn set_working_dir(&mut self, path: PathBuf) {
        self.working_dir = path;
    }

    /// Toggle terminal visibility
    pub fn toggle(&mut self) -> bool {
        if self.visible && !self.process_exited {
            self.visible = false;
        } else {
            // Spawn if not already running or if process exited
            if self.pty_master.is_none() || self.process_exited {
                if let Err(e) = self.spawn() {
                    eprintln!("Failed to spawn terminal: {}", e);
                    return false;
                }
            }
            self.visible = true;
        }
        self.visible
    }

    /// Spawn the terminal process
    fn spawn(&mut self) -> anyhow::Result<()> {
        let pty_system = native_pty_system();

        let pair = pty_system.openpty(PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // Get user's shell
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

        let mut cmd = CommandBuilder::new(&shell);
        cmd.cwd(&self.working_dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        // Spawn the shell
        let child = pair.slave.spawn_command(cmd)?;

        // Set up reader thread
        let mut reader = pair.master.try_clone_reader()?;
        let output_buffer = self.output_buffer.clone();

        let reader_handle = thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        if let Ok(mut output) = output_buffer.lock() {
                            output.extend_from_slice(&buf[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Take the writer once for reuse (take_writer should only be called once)
        self.pty_writer = pair.master.take_writer().ok();
        self.pty_master = Some(pair.master);
        self.child = Some(child);
        self.reader_thread = Some(reader_handle);
        self.process_exited = false;

        if let Ok(mut output) = self.output_buffer.lock() {
            output.clear();
        }

        // Clear emulator state for a fresh shell.
        self.term = Self::new_term(self.rows, self.cols);
        self.processor = VteProcessor::new();

        Ok(())
    }

    /// Resize the terminal
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1).min(BUFFER_ROWS as u16);
        let cols = cols.max(2).min(BUFFER_COLS as u16);
        if self.rows == rows && self.cols == cols {
            return;
        }

        self.rows = rows;
        self.cols = cols;
        self.term.resize(TerminalDimensions::new(rows, cols));

        if let Some(ref master) = self.pty_master {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    /// Send input to the terminal
    pub fn send_input(&mut self, data: &[u8]) {
        if let Some(ref mut writer) = self.pty_writer {
            if writer.write_all(data).and_then(|_| writer.flush()).is_err() {
                self.mark_process_exited();
            }
        }
    }

    /// Send a key to the terminal
    pub fn send_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        let app_cursor = self.term.mode().contains(TermMode::APP_CURSOR);
        let cursor_sequence = |normal: u8, app: u8| {
            if app_cursor {
                vec![0x1b, b'O', app]
            } else {
                vec![0x1b, b'[', normal]
            }
        };

        let data: Vec<u8> = match (key.modifiers, key.code) {
            // Control characters
            (KeyModifiers::CONTROL, KeyCode::Char(c)) => {
                let ctrl_char = (c.to_ascii_lowercase() as u8).wrapping_sub(b'a' - 1);
                vec![ctrl_char]
            }
            // Special keys
            (_, KeyCode::Enter) => vec![b'\r'],
            (_, KeyCode::Backspace) => vec![127],
            (_, KeyCode::Tab) => vec![b'\t'],
            (_, KeyCode::Esc) => vec![0x1b],
            (_, KeyCode::Up) => cursor_sequence(b'A', b'A'),
            (_, KeyCode::Down) => cursor_sequence(b'B', b'B'),
            (_, KeyCode::Right) => cursor_sequence(b'C', b'C'),
            (_, KeyCode::Left) => cursor_sequence(b'D', b'D'),
            (_, KeyCode::Home) => cursor_sequence(b'H', b'H'),
            (_, KeyCode::End) => cursor_sequence(b'F', b'F'),
            (_, KeyCode::PageUp) => vec![0x1b, b'[', b'5', b'~'],
            (_, KeyCode::PageDown) => vec![0x1b, b'[', b'6', b'~'],
            (_, KeyCode::Delete) => vec![0x1b, b'[', b'3', b'~'],
            (_, KeyCode::Insert) => vec![0x1b, b'[', b'2', b'~'],
            // Regular characters
            (KeyModifiers::ALT, KeyCode::Char(c)) => {
                let mut data = vec![0x1b];
                let mut buf = [0u8; 4];
                data.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                data
            }
            (_, KeyCode::Char(c)) => {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
            _ => return,
        };

        self.send_input(&data);
    }

    /// Process output from the terminal and update buffer
    /// Returns true if there was new output to process
    pub fn process_output(&mut self) -> bool {
        // Check if process has exited
        let process_exited = self
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok())
            .flatten()
            .is_some();
        if process_exited {
            self.mark_process_exited();
        }

        // Get output from buffer (recover from poisoned mutex if needed)
        let data = {
            let mut output = self
                .output_buffer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if output.is_empty() {
                return process_exited;
            }
            std::mem::take(&mut *output)
        };

        // Process the output bytes
        self.process_bytes(&data);
        true
    }

    fn mark_process_exited(&mut self) {
        self.visible = false;
        self.process_exited = true;
        self.pty_writer = None;
        self.pty_master = None;
        self.child = None;
        self.reader_thread = None;
    }

    /// Process bytes and update terminal buffer
    fn process_bytes(&mut self, data: &[u8]) {
        self.processor.advance(&mut self.term, data);
    }

    /// Get visible styled cells for rendering.
    pub fn get_visible_cells(&self, rows: usize, cols: usize) -> Vec<Vec<TerminalCell>> {
        let mut lines = vec![vec![TerminalCell::default(); cols]; rows];
        let content = self.term.renderable_content();

        for indexed in content.display_iter {
            let Some(point) = point_to_viewport(content.display_offset, indexed.point) else {
                continue;
            };
            let row = point.line;
            let col = point.column.0;
            if row < rows && col < cols {
                lines[row][col] = Self::terminal_cell_from_alacritty(&indexed.cell, content.colors);
            }
        }

        lines
    }

    /// Get visible lines for tests and plain rendering fallbacks.
    pub fn get_visible_lines(&self, rows: usize, cols: usize) -> Vec<String> {
        self.get_visible_cells(rows, cols)
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|cell| if cell.hidden { ' ' } else { cell.ch })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Get cursor position within visible area
    pub fn get_cursor_pos(&self) -> (usize, usize) {
        let content = self.term.renderable_content();
        point_to_viewport(content.display_offset, content.cursor.point)
            .map(|point| (point.line, point.column.0))
            .unwrap_or((0, 0))
    }

    /// Check if terminal is visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Close the terminal (kill process)
    pub fn close(&mut self) {
        self.visible = false;

        // Kill the child process first
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
        }
        self.child = None;

        // Drop pty_master to signal EOF to the reader thread
        self.pty_master = None;
        self.pty_writer = None;

        // Join the reader thread to prevent leaks
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }

        self.process_exited = true;
    }

    fn terminal_cell_from_alacritty(cell: &Cell, colors: &Colors) -> TerminalCell {
        let flags = cell.flags;
        let hidden = flags.contains(Flags::HIDDEN);
        let is_spacer = flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
        TerminalCell {
            ch: if hidden || is_spacer { ' ' } else { cell.c },
            fg: Self::resolve_color(cell.fg, colors),
            bg: Self::resolve_color(cell.bg, colors),
            bold: flags.contains(Flags::BOLD),
            dim: flags.contains(Flags::DIM),
            italic: flags.contains(Flags::ITALIC),
            underline: flags.contains(Flags::UNDERLINE),
            double_underline: flags.contains(Flags::DOUBLE_UNDERLINE),
            undercurl: flags.contains(Flags::UNDERCURL),
            underdotted: flags.contains(Flags::DOTTED_UNDERLINE),
            underdashed: flags.contains(Flags::DASHED_UNDERLINE),
            inverse: flags.contains(Flags::INVERSE),
            hidden,
            strikeout: flags.contains(Flags::STRIKEOUT),
        }
    }

    fn resolve_color(color: AlacrittyColor, colors: &Colors) -> Option<CrosstermColor> {
        match color {
            AlacrittyColor::Spec(rgb) => Some(Self::rgb_to_crossterm(rgb)),
            AlacrittyColor::Indexed(index) => colors[index as usize]
                .map(Self::rgb_to_crossterm)
                .or_else(|| Some(Self::indexed_color(index))),
            AlacrittyColor::Named(named) => colors[named]
                .map(Self::rgb_to_crossterm)
                .or_else(|| Self::default_named_color(named)),
        }
    }

    fn rgb_to_crossterm(rgb: AlacrittyRgb) -> CrosstermColor {
        CrosstermColor::Rgb {
            r: rgb.r,
            g: rgb.g,
            b: rgb.b,
        }
    }

    fn default_named_color(named: NamedColor) -> Option<CrosstermColor> {
        let (r, g, b) = match named {
            NamedColor::Black => (0, 0, 0),
            NamedColor::Red => (205, 49, 49),
            NamedColor::Green => (13, 188, 121),
            NamedColor::Yellow => (229, 229, 16),
            NamedColor::Blue => (36, 114, 200),
            NamedColor::Magenta => (188, 63, 188),
            NamedColor::Cyan => (17, 168, 205),
            NamedColor::White => (229, 229, 229),
            NamedColor::BrightBlack => (102, 102, 102),
            NamedColor::BrightRed => (241, 76, 76),
            NamedColor::BrightGreen => (35, 209, 139),
            NamedColor::BrightYellow => (245, 245, 67),
            NamedColor::BrightBlue => (59, 142, 234),
            NamedColor::BrightMagenta => (214, 112, 214),
            NamedColor::BrightCyan => (41, 184, 219),
            NamedColor::BrightWhite | NamedColor::BrightForeground => (255, 255, 255),
            NamedColor::DimBlack => (0, 0, 0),
            NamedColor::DimRed => (122, 29, 29),
            NamedColor::DimGreen => (7, 112, 72),
            NamedColor::DimYellow => (137, 137, 9),
            NamedColor::DimBlue => (21, 68, 120),
            NamedColor::DimMagenta => (112, 37, 112),
            NamedColor::DimCyan => (10, 100, 123),
            NamedColor::DimWhite | NamedColor::DimForeground => (137, 137, 137),
            NamedColor::Foreground | NamedColor::Background | NamedColor::Cursor => return None,
        };

        Some(CrosstermColor::Rgb { r, g, b })
    }

    fn indexed_color(index: u8) -> CrosstermColor {
        if index < 16 {
            return Self::default_ansi_index_color(index);
        }

        if index <= 231 {
            let cube = index - 16;
            let r = cube / 36;
            let g = (cube % 36) / 6;
            let b = cube % 6;
            return CrosstermColor::Rgb {
                r: Self::xterm_color_component(r),
                g: Self::xterm_color_component(g),
                b: Self::xterm_color_component(b),
            };
        }

        let gray = 8 + (index - 232).saturating_mul(10);
        CrosstermColor::Rgb {
            r: gray,
            g: gray,
            b: gray,
        }
    }

    fn default_ansi_index_color(index: u8) -> CrosstermColor {
        let named = match index {
            0 => NamedColor::Black,
            1 => NamedColor::Red,
            2 => NamedColor::Green,
            3 => NamedColor::Yellow,
            4 => NamedColor::Blue,
            5 => NamedColor::Magenta,
            6 => NamedColor::Cyan,
            7 => NamedColor::White,
            8 => NamedColor::BrightBlack,
            9 => NamedColor::BrightRed,
            10 => NamedColor::BrightGreen,
            11 => NamedColor::BrightYellow,
            12 => NamedColor::BrightBlue,
            13 => NamedColor::BrightMagenta,
            14 => NamedColor::BrightCyan,
            _ => NamedColor::BrightWhite,
        };

        Self::default_named_color(named).unwrap_or(CrosstermColor::White)
    }

    fn xterm_color_component(component: u8) -> u8 {
        if component == 0 {
            0
        } else {
            55 + component * 40
        }
    }
}

impl Default for FloatingTerminal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_size_matches_popup_inner_area() {
        assert_eq!(popup_size_for_screen(120, 40), (72, 24));
        assert_eq!(content_size_for_screen(120, 40), (22, 70));
    }

    #[test]
    fn printable_output_wraps_on_next_character() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 4);

        terminal.process_bytes(b"abcd");
        assert_eq!(terminal.get_visible_lines(3, 4)[0], "abcd");
        assert_eq!(terminal.get_cursor_pos(), (0, 3));

        terminal.process_bytes(b"e");
        let lines = terminal.get_visible_lines(3, 4);
        assert_eq!(lines[0], "abcd");
        assert_eq!(lines[1], "e");
        assert_eq!(terminal.get_cursor_pos(), (1, 1));
    }

    #[test]
    fn high_volume_output_does_not_overrun_scrollback() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 4);

        for _ in 0..1200 {
            terminal.process_bytes(b"line\n");
        }

        assert_eq!(terminal.get_visible_lines(3, 4).len(), 3);
    }

    #[test]
    fn charset_escape_sequences_do_not_render_as_text() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);

        terminal.process_bytes(b"\x1b(Bhello\x1b(Bworld\x1b(B");

        assert_eq!(terminal.get_visible_lines(3, 20)[0], "helloworld");
    }

    #[test]
    fn osc_st_sequences_do_not_leave_trailing_bytes() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);

        terminal.process_bytes(b"before\x1b]0;title\x1b\\after");

        assert_eq!(terminal.get_visible_lines(3, 20)[0], "beforeafter");
    }

    #[test]
    fn sgr_color_sequences_are_preserved_in_cells() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);

        terminal.process_bytes(b"\x1b[31mred\x1b[0m");
        let cells = terminal.get_visible_cells(3, 20);

        assert_eq!(cells[0][0].ch, 'r');
        assert!(cells[0][0].fg.is_some());
    }
}
