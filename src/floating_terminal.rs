//! Floating terminal implementation using PTY
//!
//! Provides a toggleable floating terminal window that runs the user's shell.

use alacritty_terminal::{
    event::{Event, EventListener},
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
const DEFAULT_FLOATING_TERMINAL_RATIO: f32 = 0.9;
const MIN_FLOATING_TERMINAL_RATIO: f32 = 0.2;
const MAX_FLOATING_TERMINAL_RATIO: f32 = 1.0;
const MIN_POPUP_WIDTH: u16 = 40;
const MIN_POPUP_HEIGHT: u16 = 10;
const MAX_TITLE_LEN: usize = 120;
const VISIBLE_OUTPUT_CHUNK_BYTES: usize = 256 * 1024;
const BACKGROUND_OUTPUT_CHUNK_BYTES: usize = 32 * 1024;

fn normalize_popup_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(MIN_FLOATING_TERMINAL_RATIO, MAX_FLOATING_TERMINAL_RATIO)
    } else {
        DEFAULT_FLOATING_TERMINAL_RATIO
    }
}

/// Calculate the floating terminal popup size for the editor screen.
pub fn popup_size_for_screen(
    screen_width: u16,
    screen_height: u16,
    width_ratio: f32,
    height_ratio: f32,
) -> (u16, u16) {
    let width_ratio = normalize_popup_ratio(width_ratio);
    let height_ratio = normalize_popup_ratio(height_ratio);
    let width = ((screen_width as f32 * width_ratio) as u16)
        .max(MIN_POPUP_WIDTH)
        .min(screen_width.max(2));
    let height = ((screen_height as f32 * height_ratio) as u16)
        .max(MIN_POPUP_HEIGHT)
        .min(screen_height.max(2));

    (width, height)
}

/// Calculate the PTY content size for the floating terminal popup.
pub fn content_size_for_screen(
    screen_width: u16,
    screen_height: u16,
    width_ratio: f32,
    height_ratio: f32,
) -> (u16, u16) {
    let (popup_width, popup_height) =
        popup_size_for_screen(screen_width, screen_height, width_ratio, height_ratio);
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

#[derive(Clone)]
struct TerminalEventListener {
    title: Arc<Mutex<Option<String>>>,
}

impl EventListener for TerminalEventListener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(title) => {
                if let Ok(mut current_title) = self.title.lock() {
                    *current_title = sanitize_terminal_title(&title);
                }
            }
            Event::ResetTitle => {
                if let Ok(mut current_title) = self.title.lock() {
                    *current_title = None;
                }
            }
            _ => {}
        }
    }
}

fn sanitize_terminal_title(title: &str) -> Option<String> {
    let sanitized: String = title
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_TITLE_LEN)
        .collect();
    let sanitized = sanitized.trim();

    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized.to_string())
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

/// One PTY-backed terminal session.
struct TerminalSession {
    id: usize,
    name: String,
    /// Whether the terminal is currently visible
    visible: bool,
    /// Alacritty terminal emulator grid
    term: Term<TerminalEventListener>,
    /// VTE parser feeding the emulator grid
    processor: VteProcessor,
    /// Current OSC window title emitted by terminal programs
    terminal_title: Arc<Mutex<Option<String>>>,
    /// Command line currently being typed at the shell prompt
    pending_command: String,
    /// Last command submitted from the shell prompt
    last_command: Option<String>,
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

impl TerminalSession {
    /// Create a new terminal session (not yet spawned)
    fn new(id: usize, name: String, rows: u16, cols: u16, working_dir: PathBuf) -> Self {
        let terminal_title = Arc::new(Mutex::new(None));
        Self {
            id,
            name,
            visible: false,
            term: Self::new_term(rows, cols, terminal_title.clone()),
            processor: VteProcessor::new(),
            terminal_title,
            pending_command: String::new(),
            last_command: None,
            rows,
            cols,
            pty_master: None,
            pty_writer: None,
            child: None,
            output_buffer: Arc::new(Mutex::new(Vec::new())),
            reader_thread: None,
            working_dir,
            process_exited: false,
        }
    }

    fn new_term(
        rows: u16,
        cols: u16,
        terminal_title: Arc<Mutex<Option<String>>>,
    ) -> Term<TerminalEventListener> {
        let mut config = Config::default();
        config.scrolling_history = BUFFER_ROWS;
        Term::new(
            config,
            &TerminalDimensions::new(rows, cols),
            TerminalEventListener {
                title: terminal_title,
            },
        )
    }

    /// Set the working directory for the terminal
    fn set_working_dir(&mut self, path: PathBuf) {
        self.working_dir = path;
    }

    fn show(&mut self) -> anyhow::Result<()> {
        // Spawn if not already running or if process exited
        if self.pty_master.is_none() || self.process_exited {
            self.spawn()?;
        }
        self.visible = true;
        Ok(())
    }

    fn hide(&mut self) {
        self.visible = false;
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
        if let Ok(mut title) = self.terminal_title.lock() {
            *title = None;
        }
        self.pending_command.clear();
        self.last_command = None;
        self.term = Self::new_term(self.rows, self.cols, self.terminal_title.clone());
        self.processor = VteProcessor::new();

        Ok(())
    }

    /// Resize the terminal
    fn resize(&mut self, rows: u16, cols: u16) {
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
    fn send_input(&mut self, data: &[u8]) {
        if let Some(ref mut writer) = self.pty_writer {
            if writer.write_all(data).and_then(|_| writer.flush()).is_err() {
                self.mark_process_exited();
            }
        }
    }

    /// Send a key to the terminal
    fn send_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        self.capture_prompt_key_for_metadata(key);

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

    fn capture_prompt_key_for_metadata(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }

        match (key.modifiers, key.code) {
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(ch)) => {
                self.pending_command.push(ch);
            }
            (_, KeyCode::Backspace) => {
                self.pending_command.pop();
            }
            (_, KeyCode::Enter) => {
                if let Some(command) = sanitize_terminal_title(&self.pending_command) {
                    self.last_command = Some(command);
                }
                self.pending_command.clear();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u' | 'c')) => {
                self.pending_command.clear();
            }
            _ => {}
        }
    }

    /// Process output from the terminal and update buffer
    /// Returns true if there was new output to process
    fn process_output(&mut self, max_bytes: usize) -> bool {
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
            if output.len() <= max_bytes {
                std::mem::take(&mut *output)
            } else {
                let remaining = output.split_off(max_bytes);
                std::mem::replace(&mut *output, remaining)
            }
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

    fn terminal_title(&self) -> Option<String> {
        self.terminal_title
            .lock()
            .ok()
            .and_then(|title| title.clone())
    }

    fn display_metadata(&self) -> Option<String> {
        self.last_command.clone().or_else(|| self.terminal_title())
    }

    /// Get visible styled cells for rendering.
    fn get_visible_cells(&self, rows: usize, cols: usize) -> Vec<Vec<TerminalCell>> {
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
    fn get_visible_lines(&self, rows: usize, cols: usize) -> Vec<String> {
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
    fn get_cursor_pos(&self) -> (usize, usize) {
        let content = self.term.renderable_content();
        point_to_viewport(content.display_offset, content.cursor.point)
            .map(|point| (point.line, point.column.0))
            .unwrap_or((0, 0))
    }

    /// Check if terminal is visible
    fn is_visible(&self) -> bool {
        self.visible
    }

    /// Close the terminal (kill process)
    fn close(&mut self) {
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

/// Lightweight metadata for rendering terminal sessions outside the terminal UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSessionInfo {
    pub position: usize,
    pub id: usize,
    pub name: String,
    pub metadata: Option<String>,
    pub active: bool,
    pub state: &'static str,
}

/// Manages multiple floating terminal sessions.
pub struct FloatingTerminal {
    sessions: Vec<TerminalSession>,
    active: Option<usize>,
    rows: u16,
    cols: u16,
    working_dir: PathBuf,
    next_id: usize,
}

impl FloatingTerminal {
    /// Create a new floating terminal manager.
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            active: None,
            rows: 24,
            cols: 80,
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            next_id: 1,
        }
    }

    /// Set the working directory used by newly created terminal sessions.
    pub fn set_working_dir(&mut self, path: PathBuf) {
        self.working_dir = path.clone();
        for session in &mut self.sessions {
            if session.pty_master.is_none() || session.process_exited {
                session.set_working_dir(path.clone());
            }
        }
    }

    /// Toggle the active terminal's visibility, creating the first session if needed.
    pub fn toggle(&mut self) -> bool {
        if let Some(active) = self.active_session() {
            if active.is_visible() && !active.process_exited {
                active.hide();
                return false;
            }
        }

        self.show_active().is_ok()
    }

    /// Create and show a new terminal session.
    pub fn create_session(&mut self, name: Option<String>) -> anyhow::Result<String> {
        let idx = self.push_session(name);
        self.show_session(idx)?;
        Ok(self.active_status())
    }

    /// Switch to the next terminal session and show it.
    pub fn next_session(&mut self) -> anyhow::Result<String> {
        if self.sessions.is_empty() {
            return self.create_session(None);
        }

        let next = self
            .active
            .map(|idx| (idx + 1) % self.sessions.len())
            .unwrap_or(0);
        self.show_session(next)?;
        Ok(self.active_status())
    }

    /// Switch to the previous terminal session and show it.
    pub fn previous_session(&mut self) -> anyhow::Result<String> {
        if self.sessions.is_empty() {
            return self.create_session(None);
        }

        let previous = self
            .active
            .map(|idx| {
                if idx == 0 {
                    self.sessions.len() - 1
                } else {
                    idx - 1
                }
            })
            .unwrap_or(0);
        self.show_session(previous)?;
        Ok(self.active_status())
    }

    /// Select a terminal session by its 1-based list position and show it.
    pub fn select_session(&mut self, position: usize) -> anyhow::Result<String> {
        if position == 0 || position > self.sessions.len() {
            anyhow::bail!("No terminal session {}", position);
        }

        self.show_session(position - 1)?;
        Ok(self.active_status())
    }

    /// Rename the active terminal session.
    pub fn rename_active_session(&mut self, name: String) -> anyhow::Result<String> {
        let Some(idx) = self.active else {
            anyhow::bail!("No terminal session");
        };

        self.rename_session_by_index(idx, name)
    }

    /// Rename a terminal session by its 1-based list position.
    pub fn rename_session(&mut self, position: usize, name: String) -> anyhow::Result<String> {
        if position == 0 || position > self.sessions.len() {
            anyhow::bail!("No terminal session {}", position);
        }

        self.rename_session_by_index(position - 1, name)
    }

    /// Return structured summaries of all terminal sessions.
    pub fn session_infos(&self) -> Vec<TerminalSessionInfo> {
        self.sessions
            .iter()
            .enumerate()
            .map(|(idx, session)| TerminalSessionInfo {
                position: idx + 1,
                id: session.id,
                name: session.name.clone(),
                metadata: session.display_metadata(),
                active: Some(idx) == self.active,
                state: if session.process_exited {
                    "exited"
                } else if session.is_visible() {
                    "visible"
                } else {
                    "hidden"
                },
            })
            .collect()
    }

    /// Return a compact summary of all terminal sessions.
    pub fn list_sessions(&self) -> String {
        if self.sessions.is_empty() {
            return "No terminal sessions".to_string();
        }

        let sessions = self
            .session_infos()
            .into_iter()
            .map(|session| {
                let marker = if session.active { "*" } else { " " };
                let title = session
                    .metadata
                    .filter(|title| title != &session.name)
                    .map(|title| format!(" - {}", title))
                    .unwrap_or_default();
                format!(
                    "{}{}:{}#{} ({}){}",
                    marker, session.position, session.name, session.id, session.state, title
                )
            })
            .collect::<Vec<_>>()
            .join(", ");

        format!("Terminals: {}", sessions)
    }

    /// Title for the active floating terminal window.
    pub fn title(&self) -> String {
        self.active
            .and_then(|idx| self.sessions.get(idx).map(|session| (idx, session)))
            .map(|(idx, session)| {
                format!(
                    " Terminal {}/{}: {} ",
                    idx + 1,
                    self.sessions.len(),
                    session.name
                )
            })
            .unwrap_or_else(|| " Terminal ".to_string())
    }

    /// Resize all sessions to match the floating terminal content area.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows.max(1).min(BUFFER_ROWS as u16);
        self.cols = cols.max(2).min(BUFFER_COLS as u16);
        for session in &mut self.sessions {
            session.resize(self.rows, self.cols);
        }
    }

    /// Send a key to the active visible terminal.
    pub fn send_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Some(active) = self.active_session() {
            if active.is_visible() {
                active.send_key(key);
            }
        }
    }

    /// Process output for all sessions.
    /// Returns true when the active visible terminal needs a redraw or has just hidden.
    pub fn process_output(&mut self) -> bool {
        let active = self.active;
        let mut active_changed = false;

        for (idx, session) in self.sessions.iter_mut().enumerate() {
            let was_visible = session.is_visible();
            let is_active_visible = Some(idx) == active && was_visible;
            let max_bytes = if is_active_visible {
                VISIBLE_OUTPUT_CHUNK_BYTES
            } else {
                BACKGROUND_OUTPUT_CHUNK_BYTES
            };
            if session.process_output(max_bytes)
                && Some(idx) == active
                && (was_visible || session.is_visible())
            {
                active_changed = true;
            }
        }

        active_changed
    }

    /// Get visible styled cells for rendering.
    pub fn get_visible_cells(&self, rows: usize, cols: usize) -> Vec<Vec<TerminalCell>> {
        self.active
            .and_then(|idx| self.sessions.get(idx))
            .map(|session| session.get_visible_cells(rows, cols))
            .unwrap_or_else(|| vec![vec![TerminalCell::default(); cols]; rows])
    }

    /// Get visible lines for tests and plain rendering fallbacks.
    pub fn get_visible_lines(&self, rows: usize, cols: usize) -> Vec<String> {
        self.active
            .and_then(|idx| self.sessions.get(idx))
            .map(|session| session.get_visible_lines(rows, cols))
            .unwrap_or_else(|| vec![String::new(); rows])
    }

    /// Get cursor position within visible area.
    pub fn get_cursor_pos(&self) -> (usize, usize) {
        self.active
            .and_then(|idx| self.sessions.get(idx))
            .map(|session| session.get_cursor_pos())
            .unwrap_or((0, 0))
    }

    /// Check if the active terminal is visible.
    pub fn is_visible(&self) -> bool {
        self.active
            .and_then(|idx| self.sessions.get(idx))
            .map(|session| session.is_visible())
            .unwrap_or(false)
    }

    /// Kill and remove the active terminal session.
    pub fn close(&mut self) {
        if let Some(idx) = self.active {
            let _ = self.close_session(idx + 1);
        }
    }

    /// Kill and remove a terminal session by its 1-based list position.
    pub fn close_session(&mut self, position: usize) -> anyhow::Result<String> {
        if position == 0 || position > self.sessions.len() {
            anyhow::bail!("No terminal session {}", position);
        }

        let idx = position - 1;
        let removed_name = self.sessions[idx].name.clone();
        let mut session = self.sessions.remove(idx);
        session.close();

        if self.sessions.is_empty() {
            self.active = None;
        } else {
            self.active = match self.active {
                Some(active) if active == idx => Some(idx.min(self.sessions.len() - 1)),
                Some(active) if active > idx => Some(active - 1),
                Some(active) if active < self.sessions.len() => Some(active),
                _ => Some(self.sessions.len() - 1),
            };
        }

        Ok(format!("Terminal killed: {}", removed_name))
    }

    fn active_status(&self) -> String {
        self.active
            .and_then(|idx| {
                self.sessions.get(idx).map(|session| {
                    format!(
                        "Terminal {}/{}: {}",
                        idx + 1,
                        self.sessions.len(),
                        session.name
                    )
                })
            })
            .unwrap_or_else(|| "No terminal sessions".to_string())
    }

    fn active_session(&mut self) -> Option<&mut TerminalSession> {
        let idx = self.active?;
        self.sessions.get_mut(idx)
    }

    fn ensure_active_index(&mut self) -> usize {
        if self.active.is_none_or(|idx| idx >= self.sessions.len()) {
            let idx = self.push_session(None);
            self.active = Some(idx);
        }

        self.active.unwrap_or(0)
    }

    fn push_session(&mut self, name: Option<String>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let name = name
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("term-{}", id));

        let session =
            TerminalSession::new(id, name, self.rows, self.cols, self.working_dir.clone());
        self.sessions.push(session);
        let idx = self.sessions.len() - 1;
        self.active = Some(idx);
        idx
    }

    fn show_active(&mut self) -> anyhow::Result<()> {
        let idx = self.ensure_active_index();
        self.show_session(idx)
    }

    fn show_session(&mut self, idx: usize) -> anyhow::Result<()> {
        if idx >= self.sessions.len() {
            anyhow::bail!("No terminal session {}", idx + 1);
        }

        for (session_idx, session) in self.sessions.iter_mut().enumerate() {
            if session_idx != idx {
                session.hide();
            }
        }

        self.sessions[idx].show()?;
        self.active = Some(idx);
        Ok(())
    }

    fn rename_session_by_index(&mut self, idx: usize, name: String) -> anyhow::Result<String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Terminal name cannot be empty");
        }

        let Some(session) = self.sessions.get_mut(idx) else {
            anyhow::bail!("No terminal session {}", idx + 1);
        };

        session.name = trimmed.to_string();
        Ok(format!("Terminal {} renamed to: {}", idx + 1, session.name))
    }

    /// Feed bytes directly into the active emulator. Used by unit tests.
    #[cfg(test)]
    fn process_bytes(&mut self, data: &[u8]) {
        let idx = self.ensure_active_index();
        self.sessions[idx].process_bytes(data);
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
        assert_eq!(popup_size_for_screen(120, 40, 0.9, 0.9), (108, 36));
        assert_eq!(content_size_for_screen(120, 40, 0.9, 0.9), (34, 106));
    }

    #[test]
    fn popup_size_clamps_configured_ratios() {
        assert_eq!(popup_size_for_screen(120, 40, 2.0, f32::NAN), (120, 36));
        assert_eq!(popup_size_for_screen(120, 40, 0.1, 0.1), (40, 10));
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
    fn terminal_output_processing_respects_byte_budget() {
        let mut session =
            TerminalSession::new(1, "server".to_string(), 3, 20, PathBuf::from("/tmp"));
        {
            let mut output = session.output_buffer.lock().unwrap();
            output.extend_from_slice(b"abcdef");
        }

        assert!(session.process_output(3));
        assert_eq!(session.get_visible_lines(1, 20)[0], "abc");
        assert_eq!(session.output_buffer.lock().unwrap().as_slice(), b"def");

        assert!(session.process_output(3));
        assert_eq!(session.get_visible_lines(1, 20)[0], "abcdef");
        assert!(session.output_buffer.lock().unwrap().is_empty());
    }

    #[test]
    fn hidden_terminal_output_is_processed_without_requesting_redraw() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);
        let idx = terminal.ensure_active_index();
        terminal.sessions[idx].hide();
        {
            let mut output = terminal.sessions[idx].output_buffer.lock().unwrap();
            output.extend_from_slice(b"background");
        }

        assert!(!terminal.process_output());
        assert_eq!(terminal.get_visible_lines(1, 20)[0], "background");
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
        assert_eq!(
            terminal.session_infos()[0].metadata.as_deref(),
            Some("title")
        );
    }

    #[test]
    fn osc_title_metadata_is_sanitized() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);

        terminal.process_bytes(b"\x1b]0; npm\x01 run dev \x07");

        assert_eq!(
            terminal.session_infos()[0].metadata.as_deref(),
            Some("npm run dev")
        );
    }

    #[test]
    fn submitted_command_metadata_overrides_stale_title() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut terminal = FloatingTerminal::new();
        terminal.process_bytes(b"\x1b]0;old title\x07");
        terminal.sessions[0].visible = true;

        for ch in "npm run dev".chars() {
            terminal.send_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        terminal.send_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(
            terminal.session_infos()[0].metadata.as_deref(),
            Some("npm run dev")
        );
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

    #[test]
    fn terminal_sessions_keep_independent_buffers() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);

        let first = terminal.ensure_active_index();
        terminal.sessions[first].process_bytes(b"server");
        let second = terminal.push_session(Some("git".to_string()));
        terminal.sessions[second].process_bytes(b"lazygit");

        terminal.active = Some(first);
        assert_eq!(terminal.get_visible_lines(3, 20)[0], "server");

        terminal.active = Some(second);
        assert_eq!(terminal.get_visible_lines(3, 20)[0], "lazygit");
        assert_eq!(terminal.title(), " Terminal 2/2: git ");
    }

    #[test]
    fn terminal_list_marks_active_session() {
        let mut terminal = FloatingTerminal::new();
        terminal.push_session(Some("server".to_string()));
        terminal.push_session(Some("git".to_string()));
        terminal.active = Some(1);

        let list = terminal.list_sessions();

        assert!(list.contains(" 1:server#1"));
        assert!(list.contains("*2:git#2"));
    }

    #[test]
    fn terminal_session_infos_include_position_active_and_state() {
        let mut terminal = FloatingTerminal::new();
        terminal.push_session(Some("server".to_string()));
        terminal.push_session(Some("git".to_string()));
        terminal.active = Some(1);
        terminal.sessions[0].process_exited = true;

        let infos = terminal.session_infos();

        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].position, 1);
        assert_eq!(infos[0].name, "server");
        assert_eq!(infos[0].metadata, None);
        assert!(!infos[0].active);
        assert_eq!(infos[0].state, "exited");
        assert_eq!(infos[1].position, 2);
        assert_eq!(infos[1].name, "git");
        assert_eq!(infos[1].metadata, None);
        assert!(infos[1].active);
        assert_eq!(infos[1].state, "hidden");
    }

    #[test]
    fn close_session_removes_requested_session_and_preserves_active() {
        let mut terminal = FloatingTerminal::new();
        terminal.push_session(Some("server".to_string()));
        terminal.push_session(Some("git".to_string()));
        terminal.push_session(Some("tests".to_string()));
        terminal.active = Some(2);

        terminal.close_session(2).unwrap();

        assert_eq!(terminal.sessions.len(), 2);
        assert_eq!(terminal.sessions[0].name, "server");
        assert_eq!(terminal.sessions[1].name, "tests");
        assert_eq!(terminal.active, Some(1));
    }

    #[test]
    fn rename_terminal_sessions_by_active_or_position() {
        let mut terminal = FloatingTerminal::new();
        terminal.push_session(Some("server".to_string()));
        terminal.push_session(Some("git".to_string()));
        terminal.active = Some(1);

        terminal
            .rename_active_session("lazygit".to_string())
            .unwrap();
        terminal
            .rename_session(1, " dev server ".to_string())
            .unwrap();

        assert_eq!(terminal.sessions[0].name, "dev server");
        assert_eq!(terminal.sessions[1].name, "lazygit");
    }

    #[test]
    fn rename_terminal_rejects_empty_name() {
        let mut terminal = FloatingTerminal::new();
        terminal.push_session(Some("server".to_string()));

        assert!(terminal.rename_active_session("   ".to_string()).is_err());
        assert_eq!(terminal.sessions[0].name, "server");
    }

    #[test]
    fn close_removes_active_session() {
        let mut terminal = FloatingTerminal::new();
        terminal.push_session(Some("server".to_string()));
        terminal.push_session(Some("git".to_string()));
        terminal.active = Some(0);

        terminal.close();

        assert_eq!(terminal.sessions.len(), 1);
        assert_eq!(terminal.sessions[0].name, "git");
        assert_eq!(terminal.active, Some(0));
    }
}
