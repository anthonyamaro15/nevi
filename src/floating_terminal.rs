//! Floating terminal implementation using PTY
//!
//! Provides a toggleable floating terminal window that runs the user's shell.

use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line},
    term::{
        cell::{Cell, Flags},
        color::Colors,
        point_to_viewport, ClipboardType, Config, Term, TermMode,
    },
    vte::ansi::{
        Color as AlacrittyColor, CursorShape as AlacrittyCursorShape, NamedColor,
        Processor as VteProcessor, Rgb as AlacrittyRgb,
    },
};
use crossterm::{
    event::{
        KeyCode, KeyEvent, KeyModifiers as CrosstermKeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    style::Color as CrosstermColor,
};
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
const MOUSE_SCROLL_LINES: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalContentArea {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalMouseEventResult {
    Ignored,
    Handled,
}

impl TerminalMouseEventResult {
    pub fn is_handled(&self) -> bool {
        !matches!(self, Self::Ignored)
    }
}

pub fn is_terminal_selection_platform_copy_key(key: KeyEvent) -> bool {
    let is_c = matches!(key.code, KeyCode::Char('c' | 'C'));
    is_c && ((key.modifiers.contains(CrosstermKeyModifiers::CONTROL)
        && key.modifiers.contains(CrosstermKeyModifiers::SHIFT))
        || key.modifiers.contains(CrosstermKeyModifiers::SUPER))
}

pub fn is_terminal_selection_copy_key(key: KeyEvent) -> bool {
    is_terminal_selection_platform_copy_key(key)
        || (matches!(key.code, KeyCode::Char('y' | 'Y'))
            && key.modifiers == CrosstermKeyModifiers::NONE)
}

pub fn is_terminal_selection_clear_key(key: KeyEvent) -> bool {
    matches!(
        (key.modifiers, key.code),
        (CrosstermKeyModifiers::NONE, KeyCode::Esc)
            | (CrosstermKeyModifiers::CONTROL, KeyCode::Char('['))
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalSelectionPoint {
    row: usize,
    col: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalSelection {
    anchor: TerminalSelectionPoint,
    active: TerminalSelectionPoint,
}

impl TerminalSelection {
    fn normalized(&self) -> (TerminalSelectionPoint, TerminalSelectionPoint) {
        let anchor_key = (self.anchor.row, self.anchor.col);
        let active_key = (self.active.row, self.active.col);
        if anchor_key <= active_key {
            (self.anchor, self.active)
        } else {
            (self.active, self.anchor)
        }
    }

    fn contains(&self, row: usize, col: usize) -> bool {
        let (start, end) = self.normalized();
        if row < start.row || row > end.row {
            return false;
        }

        let start_col = if row == start.row { start.col } else { 0 };
        let end_col = if row == end.row { end.col } else { usize::MAX };
        col >= start_col && col <= end_col
    }

    fn is_empty(&self) -> bool {
        self.anchor == self.active
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalSearchMatch {
    line: i32,
    start_col: usize,
    end_col: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TerminalSearchState {
    active: bool,
    query: String,
    matches: Vec<TerminalSearchMatch>,
    active_match: Option<usize>,
}

impl TerminalSearchState {
    fn clear(&mut self) {
        self.active = false;
        self.query.clear();
        self.matches.clear();
        self.active_match = None;
    }

    fn status(&self) -> Option<TerminalSearchStatus> {
        (self.active || !self.query.is_empty()).then(|| TerminalSearchStatus {
            active: self.active,
            query: self.query.clone(),
            match_count: self.matches.len(),
            active_match: self.active_match.map(|idx| idx + 1),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSearchStatus {
    pub active: bool,
    pub query: String,
    pub match_count: usize,
    pub active_match: Option<usize>,
}

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

/// Calculate the mouse-addressable content area of the floating terminal.
pub fn content_area_for_screen(
    screen_width: u16,
    screen_height: u16,
    width_ratio: f32,
    height_ratio: f32,
) -> TerminalContentArea {
    let (popup_width, popup_height) =
        popup_size_for_screen(screen_width, screen_height, width_ratio, height_ratio);
    let (rows, cols) =
        content_size_for_screen(screen_width, screen_height, width_ratio, height_ratio);

    TerminalContentArea {
        x: screen_width.saturating_sub(popup_width) / 2 + 1,
        y: screen_height.saturating_sub(popup_height) / 2 + 1,
        width: cols,
        height: rows,
    }
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
    pty_write_buffer: Arc<Mutex<Vec<u8>>>,
    clipboard_store_buffer: Arc<Mutex<Vec<TerminalClipboardStore>>>,
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
            Event::PtyWrite(text) => {
                if let Ok(mut pending) = self.pty_write_buffer.lock() {
                    pending.extend_from_slice(text.as_bytes());
                }
            }
            Event::ClipboardStore(clipboard_type, text) => {
                if let Ok(mut pending) = self.clipboard_store_buffer.lock() {
                    pending.push(TerminalClipboardStore {
                        clipboard: terminal_clipboard_type(clipboard_type),
                        text,
                    });
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

fn mouse_position_in_content(
    event: MouseEvent,
    content_area: TerminalContentArea,
) -> Option<(u16, u16)> {
    let within_columns = event.column >= content_area.x
        && event.column < content_area.x.saturating_add(content_area.width);
    let within_rows = event.row >= content_area.y
        && event.row < content_area.y.saturating_add(content_area.height);

    if within_columns && within_rows {
        Some((
            event.column.saturating_sub(content_area.x) + 1,
            event.row.saturating_sub(content_area.y) + 1,
        ))
    } else {
        None
    }
}

fn mouse_position_in_content_zero_based(
    event: MouseEvent,
    content_area: TerminalContentArea,
) -> Option<TerminalSelectionPoint> {
    mouse_position_in_content(event, content_area).map(|(col, row)| TerminalSelectionPoint {
        row: row.saturating_sub(1) as usize,
        col: col.saturating_sub(1) as usize,
    })
}

fn mouse_button_code(button: MouseButton) -> Option<u16> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
    }
}

fn mouse_modifier_code(modifiers: CrosstermKeyModifiers) -> u16 {
    let mut code = 0;
    if modifiers.contains(CrosstermKeyModifiers::SHIFT) {
        code += 4;
    }
    if modifiers.contains(CrosstermKeyModifiers::ALT) {
        code += 8;
    }
    if modifiers.contains(CrosstermKeyModifiers::CONTROL) {
        code += 16;
    }
    code
}

fn encode_legacy_mouse_report(button_code: u16, x: u16, y: u16, released: bool) -> Option<Vec<u8>> {
    let button = if released { 3 } else { button_code };
    if button > 223 || x > 223 || y > 223 {
        return None;
    }

    Some(vec![
        0x1b,
        b'[',
        b'M',
        (button + 32) as u8,
        (x + 32) as u8,
        (y + 32) as u8,
    ])
}

fn searchable_char_from_cell(cell: &Cell) -> char {
    let flags = cell.flags;
    if flags.contains(Flags::HIDDEN)
        || flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        ' '
    } else {
        cell.c
    }
}

fn search_ranges_in_line(line: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }

    let line_lower = line.to_ascii_lowercase();
    let query_lower = query.to_ascii_lowercase();
    let mut ranges = Vec::new();
    let mut start_byte = 0;

    while let Some(relative_idx) = line_lower[start_byte..].find(&query_lower) {
        let match_start = start_byte + relative_idx;
        let match_end = match_start + query_lower.len();
        ranges.push((
            line[..match_start].chars().count(),
            line[..match_end].chars().count(),
        ));
        start_byte = match_end.max(match_start + 1);
        if start_byte >= line.len() {
            break;
        }
    }

    ranges
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
    pub selected: bool,
    pub search_match: bool,
    pub active_search_match: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalCursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
    Hidden,
}

impl Default for TerminalCursorShape {
    fn default() -> Self {
        Self::Block
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalCursorInfo {
    pub row: usize,
    pub col: usize,
    pub shape: TerminalCursorShape,
    pub blinking: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalClipboard {
    Clipboard,
    Selection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalClipboardStore {
    pub clipboard: TerminalClipboard,
    pub text: String,
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
            selected: false,
            search_match: false,
            active_search_match: false,
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
    /// Terminal emulator responses that must be written back to the PTY
    pty_write_buffer: Arc<Mutex<Vec<u8>>>,
    /// OSC52 clipboard writes requested by terminal applications
    clipboard_store_buffer: Arc<Mutex<Vec<TerminalClipboardStore>>>,
    /// Reader thread handle (for joining on close)
    reader_thread: Option<JoinHandle<()>>,
    /// Working directory
    working_dir: PathBuf,
    /// Whether terminal process has exited
    process_exited: bool,
    /// Current mouse selection in viewport-relative terminal coordinates
    selection: Option<TerminalSelection>,
    /// Floating-terminal scrollback search state
    search: TerminalSearchState,
}

impl TerminalSession {
    /// Create a new terminal session (not yet spawned)
    fn new(id: usize, name: String, rows: u16, cols: u16, working_dir: PathBuf) -> Self {
        let terminal_title = Arc::new(Mutex::new(None));
        let pty_write_buffer = Arc::new(Mutex::new(Vec::new()));
        let clipboard_store_buffer = Arc::new(Mutex::new(Vec::new()));
        Self {
            id,
            name,
            visible: false,
            term: Self::new_term(
                rows,
                cols,
                terminal_title.clone(),
                pty_write_buffer.clone(),
                clipboard_store_buffer.clone(),
            ),
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
            pty_write_buffer,
            clipboard_store_buffer,
            reader_thread: None,
            working_dir,
            process_exited: false,
            selection: None,
            search: TerminalSearchState::default(),
        }
    }

    fn new_term(
        rows: u16,
        cols: u16,
        terminal_title: Arc<Mutex<Option<String>>>,
        pty_write_buffer: Arc<Mutex<Vec<u8>>>,
        clipboard_store_buffer: Arc<Mutex<Vec<TerminalClipboardStore>>>,
    ) -> Term<TerminalEventListener> {
        let mut config = Config::default();
        config.scrolling_history = BUFFER_ROWS;
        Term::new(
            config,
            &TerminalDimensions::new(rows, cols),
            TerminalEventListener {
                title: terminal_title,
                pty_write_buffer,
                clipboard_store_buffer,
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
        if let Ok(mut pending) = self.pty_write_buffer.lock() {
            pending.clear();
        }
        if let Ok(mut pending) = self.clipboard_store_buffer.lock() {
            pending.clear();
        }

        // Clear emulator state for a fresh shell.
        if let Ok(mut title) = self.terminal_title.lock() {
            *title = None;
        }
        self.pending_command.clear();
        self.last_command = None;
        self.clear_search();
        self.term = Self::new_term(
            self.rows,
            self.cols,
            self.terminal_title.clone(),
            self.pty_write_buffer.clone(),
            self.clipboard_store_buffer.clone(),
        );
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
        if self.search.active && !self.search.query.is_empty() {
            self.recompute_search_matches();
        }

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
        let modifier_param = key_modifier_param(key.modifiers);
        let cursor_sequence = |normal: u8, app: u8| {
            if app_cursor {
                vec![0x1b, b'O', app]
            } else {
                vec![0x1b, b'[', normal]
            }
        };
        let modified_cursor_sequence = |final_byte: u8| {
            modifier_param
                .map(|modifier| format!("\x1b[1;{}{}", modifier, final_byte as char).into_bytes())
        };
        let tilde_sequence = |code: u8| {
            if let Some(modifier) = modifier_param {
                format!("\x1b[{};{}~", code, modifier).into_bytes()
            } else {
                format!("\x1b[{}~", code).into_bytes()
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
            (_, KeyCode::BackTab) => b"\x1b[Z".to_vec(),
            (_, KeyCode::Esc) => vec![0x1b],
            (_, KeyCode::Up) => {
                modified_cursor_sequence(b'A').unwrap_or_else(|| cursor_sequence(b'A', b'A'))
            }
            (_, KeyCode::Down) => {
                modified_cursor_sequence(b'B').unwrap_or_else(|| cursor_sequence(b'B', b'B'))
            }
            (_, KeyCode::Right) => {
                modified_cursor_sequence(b'C').unwrap_or_else(|| cursor_sequence(b'C', b'C'))
            }
            (_, KeyCode::Left) => {
                modified_cursor_sequence(b'D').unwrap_or_else(|| cursor_sequence(b'D', b'D'))
            }
            (_, KeyCode::Home) => {
                modified_cursor_sequence(b'H').unwrap_or_else(|| cursor_sequence(b'H', b'H'))
            }
            (_, KeyCode::End) => {
                modified_cursor_sequence(b'F').unwrap_or_else(|| cursor_sequence(b'F', b'F'))
            }
            (_, KeyCode::PageUp) => tilde_sequence(5),
            (_, KeyCode::PageDown) => tilde_sequence(6),
            (_, KeyCode::Delete) => tilde_sequence(3),
            (_, KeyCode::Insert) => tilde_sequence(2),
            (_, KeyCode::F(n)) => match function_key_sequence(n, modifier_param) {
                Some(data) => data,
                None => return,
            },
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

    fn send_paste(&mut self, text: &str) {
        if self.search.active {
            self.insert_search_text(text);
            return;
        }

        if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            let mut data = Vec::with_capacity(text.len() + 12);
            data.extend_from_slice(b"\x1b[200~");
            data.extend_from_slice(text.as_bytes());
            data.extend_from_slice(b"\x1b[201~");
            self.send_input(&data);
        } else {
            self.send_input(text.as_bytes());
        }
    }

    fn mouse_input_bytes(
        &self,
        event: MouseEvent,
        content_area: TerminalContentArea,
    ) -> Option<Vec<u8>> {
        let mode = self.term.mode();
        if !mode.intersects(TermMode::MOUSE_MODE) {
            return None;
        }

        let (x, y) = mouse_position_in_content(event, content_area)?;
        let (mut button_code, released) = match event.kind {
            MouseEventKind::Down(button) => (mouse_button_code(button)?, false),
            MouseEventKind::Up(button) => (mouse_button_code(button)?, true),
            MouseEventKind::Drag(button) => {
                if !(mode.contains(TermMode::MOUSE_DRAG) || mode.contains(TermMode::MOUSE_MOTION)) {
                    return None;
                }
                (mouse_button_code(button)? + 32, false)
            }
            MouseEventKind::Moved => {
                if !mode.contains(TermMode::MOUSE_MOTION) {
                    return None;
                }
                (35, false)
            }
            MouseEventKind::ScrollUp => (64, false),
            MouseEventKind::ScrollDown => (65, false),
            MouseEventKind::ScrollLeft => (66, false),
            MouseEventKind::ScrollRight => (67, false),
        };

        button_code += mouse_modifier_code(event.modifiers);

        if mode.contains(TermMode::SGR_MOUSE) {
            let suffix = if released { 'm' } else { 'M' };
            Some(format!("\x1b[<{};{};{}{}", button_code, x, y, suffix).into_bytes())
        } else {
            encode_legacy_mouse_report(button_code, x, y, released)
        }
    }

    fn handle_local_mouse_scroll(
        &mut self,
        event: MouseEvent,
        content_area: TerminalContentArea,
    ) -> TerminalMouseEventResult {
        if mouse_position_in_content(event, content_area).is_none() {
            return TerminalMouseEventResult::Ignored;
        }

        let scroll = match event.kind {
            MouseEventKind::ScrollUp => Scroll::Delta(MOUSE_SCROLL_LINES),
            MouseEventKind::ScrollDown => Scroll::Delta(-MOUSE_SCROLL_LINES),
            _ => return TerminalMouseEventResult::Ignored,
        };

        if self
            .term
            .mode()
            .contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
        {
            let key_code = match event.kind {
                MouseEventKind::ScrollUp => KeyCode::Up,
                MouseEventKind::ScrollDown => KeyCode::Down,
                _ => return TerminalMouseEventResult::Ignored,
            };
            for _ in 0..MOUSE_SCROLL_LINES {
                self.send_key(KeyEvent::new(key_code, CrosstermKeyModifiers::NONE));
            }
            return TerminalMouseEventResult::Handled;
        }

        if self.scroll_display(scroll) {
            TerminalMouseEventResult::Handled
        } else {
            TerminalMouseEventResult::Ignored
        }
    }

    fn handle_local_mouse_selection(
        &mut self,
        event: MouseEvent,
        content_area: TerminalContentArea,
    ) -> TerminalMouseEventResult {
        let Some(point) = mouse_position_in_content_zero_based(event, content_area) else {
            return TerminalMouseEventResult::Ignored;
        };

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.selection = Some(TerminalSelection {
                    anchor: point,
                    active: point,
                });
                TerminalMouseEventResult::Handled
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(selection) = self.selection.as_mut() {
                    selection.active = point;
                    TerminalMouseEventResult::Handled
                } else {
                    TerminalMouseEventResult::Ignored
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(selection) = self.selection.as_mut() {
                    selection.active = point;
                }
                if self
                    .selection
                    .map(|selection| selection.is_empty())
                    .unwrap_or(false)
                {
                    self.selection = None;
                }
                TerminalMouseEventResult::Handled
            }
            _ => TerminalMouseEventResult::Ignored,
        }
    }

    fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    fn clear_selection(&mut self) -> bool {
        self.selection.take().is_some()
    }

    fn copy_selection(&mut self) -> Option<String> {
        let selected = self
            .selected_text(self.rows as usize, self.cols as usize)
            .filter(|text| !text.is_empty());
        self.selection = None;
        selected
    }

    fn scroll_display(&mut self, scroll: Scroll) -> bool {
        let before = self.display_offset();
        self.term.scroll_display(scroll);
        self.display_offset() != before
    }

    fn display_offset(&self) -> usize {
        self.term.renderable_content().display_offset
    }

    fn selected_text(&self, rows: usize, cols: usize) -> Option<String> {
        let selection = self.selection?;
        let (start, end) = selection.normalized();
        let cells = self.get_visible_cells(rows, cols);
        let mut selected_lines = Vec::new();

        for row in start.row..=end.row.min(rows.saturating_sub(1)) {
            let row_cells = cells.get(row)?;
            let start_col = if row == start.row { start.col } else { 0 };
            let end_col = if row == end.row {
                end.col.min(cols.saturating_sub(1))
            } else {
                cols.saturating_sub(1)
            };

            if start_col > end_col || start_col >= row_cells.len() {
                selected_lines.push(String::new());
                continue;
            }

            let line: String = row_cells[start_col..=end_col.min(row_cells.len() - 1)]
                .iter()
                .map(|cell| if cell.hidden { ' ' } else { cell.ch })
                .collect::<String>()
                .trim_end()
                .to_string();
            selected_lines.push(line);
        }

        Some(selected_lines.join("\n"))
    }

    fn start_search(&mut self) -> bool {
        self.selection = None;
        self.search.active = true;
        self.recompute_search_matches();
        true
    }

    fn clear_search(&mut self) {
        self.search.clear();
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};

        if !self.search.active {
            let starts_find_search = matches!(key.code, KeyCode::Char('f' | 'F'))
                && !key.modifiers.contains(KeyModifiers::ALT)
                && (key.modifiers.contains(KeyModifiers::CONTROL)
                    || key.modifiers.contains(KeyModifiers::SUPER));

            if starts_find_search {
                return self.start_search();
            }

            return false;
        }

        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('[')) => {
                self.clear_search();
            }
            (_, KeyCode::Backspace) => {
                if self.search.query.is_empty() {
                    self.clear_search();
                } else {
                    self.search.query.pop();
                    self.recompute_search_matches();
                }
            }
            (KeyModifiers::SHIFT, KeyCode::Enter) | (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
                self.previous_search_match();
            }
            (_, KeyCode::Enter) | (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
                self.next_search_match();
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(ch)) => {
                self.search.query.push(ch);
                self.recompute_search_matches();
            }
            _ => {}
        }

        true
    }

    fn search_status(&self) -> Option<TerminalSearchStatus> {
        self.search.status()
    }

    fn is_searching(&self) -> bool {
        self.search.active
    }

    fn insert_search_text(&mut self, text: &str) {
        if !self.search.active {
            return;
        }

        self.search
            .query
            .extend(text.chars().filter(|ch| !ch.is_control()));
        self.recompute_search_matches();
    }

    fn recompute_search_matches(&mut self) {
        self.search.matches = self.find_search_matches(&self.search.query);
        if self.search.matches.is_empty() {
            self.search.active_match = None;
            return;
        }

        let active_match = self.first_search_match_index_from_viewport();
        self.search.active_match = Some(active_match);
        self.scroll_to_active_search_match();
    }

    fn find_search_matches(&self, query: &str) -> Vec<TerminalSearchMatch> {
        if query.is_empty() {
            return Vec::new();
        }

        let grid = self.term.grid();
        let mut matches = Vec::new();
        for line in grid.topmost_line().0..=grid.bottommost_line().0 {
            let line_text = self.grid_line_text(Line(line));
            for (start_col, end_col) in search_ranges_in_line(&line_text, query) {
                matches.push(TerminalSearchMatch {
                    line,
                    start_col,
                    end_col,
                });
            }
        }

        matches
    }

    fn grid_line_text(&self, line: Line) -> String {
        let grid = self.term.grid();
        (0..grid.columns())
            .map(|col| searchable_char_from_cell(&grid[line][Column(col)]))
            .collect()
    }

    fn first_search_match_index_from_viewport(&self) -> usize {
        let viewport_top = -(self.display_offset() as i32);
        self.search
            .matches
            .iter()
            .position(|search_match| search_match.line >= viewport_top)
            .unwrap_or_else(|| self.search.matches.len().saturating_sub(1))
    }

    fn next_search_match(&mut self) -> bool {
        if self.search.matches.is_empty() {
            self.search.active_match = None;
            return false;
        }

        let next = self
            .search
            .active_match
            .map(|idx| (idx + 1) % self.search.matches.len())
            .unwrap_or(0);
        self.search.active_match = Some(next);
        self.scroll_to_active_search_match();
        true
    }

    fn previous_search_match(&mut self) -> bool {
        if self.search.matches.is_empty() {
            self.search.active_match = None;
            return false;
        }

        let previous = self
            .search
            .active_match
            .map(|idx| {
                if idx == 0 {
                    self.search.matches.len() - 1
                } else {
                    idx - 1
                }
            })
            .unwrap_or_else(|| self.search.matches.len() - 1);
        self.search.active_match = Some(previous);
        self.scroll_to_active_search_match();
        true
    }

    fn scroll_to_active_search_match(&mut self) {
        let Some(search_match) = self
            .search
            .active_match
            .and_then(|idx| self.search.matches.get(idx))
            .copied()
        else {
            return;
        };

        let rows = self.rows.max(1) as i32;
        let current_offset = self.display_offset() as i32;
        let viewport_top = -current_offset;
        let viewport_bottom = viewport_top + rows - 1;
        let target_offset = if search_match.line < viewport_top {
            -search_match.line
        } else if search_match.line > viewport_bottom {
            -(search_match.line - rows + 1)
        } else {
            return;
        };
        let delta = target_offset - current_offset;
        if delta != 0 {
            self.term.scroll_display(Scroll::Delta(delta));
        }
    }

    fn search_flags_for_cell(&self, line: i32, col: usize) -> (bool, bool) {
        if self.search.query.is_empty() {
            return (false, false);
        }

        let start = self
            .search
            .matches
            .partition_point(|search_match| search_match.line < line);
        let end = self
            .search
            .matches
            .partition_point(|search_match| search_match.line <= line);

        for (idx, search_match) in self.search.matches[start..end].iter().enumerate() {
            let contains = search_match.line == line
                && col >= search_match.start_col
                && col < search_match.end_col;
            if contains {
                return (true, self.search.active_match == Some(start + idx));
            }
        }

        (false, false)
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
        self.flush_pending_pty_writes();
        if self.search.active && !self.search.query.is_empty() {
            self.recompute_search_matches();
        }
    }

    fn flush_pending_pty_writes(&mut self) {
        if self.pty_writer.is_none() {
            return;
        }

        let data = {
            let mut pending = self
                .pty_write_buffer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if pending.is_empty() {
                return;
            }
            std::mem::take(&mut *pending)
        };

        self.send_input(&data);
    }

    fn take_pending_clipboard_stores(&mut self) -> Vec<TerminalClipboardStore> {
        let mut pending = self
            .clipboard_store_buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *pending)
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
                let mut cell = Self::terminal_cell_from_alacritty(&indexed.cell, content.colors);
                cell.selected = self
                    .selection
                    .map(|selection| selection.contains(row, col))
                    .unwrap_or(false);
                let (search_match, active_search_match) =
                    self.search_flags_for_cell(indexed.point.line.0, col);
                cell.search_match = search_match;
                cell.active_search_match = active_search_match;
                lines[row][col] = cell;
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

    fn get_cursor_info(&self) -> TerminalCursorInfo {
        let content = self.term.renderable_content();
        let (row, col) = point_to_viewport(content.display_offset, content.cursor.point)
            .map(|point| (point.line, point.column.0))
            .unwrap_or((0, 0));
        let style = self.term.cursor_style();
        let shape = terminal_cursor_shape(content.cursor.shape);

        TerminalCursorInfo {
            row,
            col,
            shape,
            blinking: shape != TerminalCursorShape::Hidden && style.blinking,
        }
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
            selected: false,
            search_match: false,
            active_search_match: false,
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

fn key_modifier_param(modifiers: CrosstermKeyModifiers) -> Option<u8> {
    let mut modifier = 1;
    if modifiers.contains(CrosstermKeyModifiers::SHIFT) {
        modifier += 1;
    }
    if modifiers.contains(CrosstermKeyModifiers::ALT) {
        modifier += 2;
    }
    if modifiers.contains(CrosstermKeyModifiers::CONTROL) {
        modifier += 4;
    }

    (modifier > 1).then_some(modifier)
}

fn function_key_sequence(key: u8, modifier_param: Option<u8>) -> Option<Vec<u8>> {
    if (1..=4).contains(&key) {
        let final_byte = match key {
            1 => 'P',
            2 => 'Q',
            3 => 'R',
            4 => 'S',
            _ => unreachable!(),
        };
        return Some(if let Some(modifier) = modifier_param {
            format!("\x1b[1;{}{}", modifier, final_byte).into_bytes()
        } else {
            format!("\x1bO{}", final_byte).into_bytes()
        });
    }

    let code = match key {
        5 => 15,
        6 => 17,
        7 => 18,
        8 => 19,
        9 => 20,
        10 => 21,
        11 => 23,
        12 => 24,
        _ => return None,
    };

    Some(if let Some(modifier) = modifier_param {
        format!("\x1b[{};{}~", code, modifier).into_bytes()
    } else {
        format!("\x1b[{}~", code).into_bytes()
    })
}

fn terminal_cursor_shape(shape: AlacrittyCursorShape) -> TerminalCursorShape {
    match shape {
        AlacrittyCursorShape::Block => TerminalCursorShape::Block,
        AlacrittyCursorShape::Underline => TerminalCursorShape::Underline,
        AlacrittyCursorShape::Beam => TerminalCursorShape::Beam,
        AlacrittyCursorShape::HollowBlock => TerminalCursorShape::HollowBlock,
        AlacrittyCursorShape::Hidden => TerminalCursorShape::Hidden,
    }
}

fn terminal_clipboard_type(clipboard_type: ClipboardType) -> TerminalClipboard {
    match clipboard_type {
        ClipboardType::Clipboard => TerminalClipboard::Clipboard,
        ClipboardType::Selection => TerminalClipboard::Selection,
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
                if let Some(status) = session.search_status().filter(|status| status.active) {
                    let match_count = if status.query.is_empty() {
                        String::new()
                    } else if let Some(active_match) = status.active_match {
                        format!(" {}/{}", active_match, status.match_count)
                    } else {
                        " 0/0".to_string()
                    };
                    return format!(" Terminal Search: /{}{} ", status.query, match_count);
                }

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
                active.clear_selection();
                active.send_key(key);
            }
        }
    }

    pub fn handle_search_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        let Some(active) = self.active_session() else {
            return false;
        };
        if !active.is_visible() {
            return false;
        }

        active.handle_search_key(key)
    }

    pub fn is_searching(&self) -> bool {
        self.active
            .and_then(|idx| self.sessions.get(idx))
            .is_some_and(TerminalSession::is_searching)
    }

    pub fn search_status(&self) -> Option<TerminalSearchStatus> {
        self.active
            .and_then(|idx| self.sessions.get(idx))
            .and_then(TerminalSession::search_status)
    }

    pub fn send_paste(&mut self, text: &str) -> bool {
        let Some(active) = self.active_session() else {
            return false;
        };
        if !active.is_visible() {
            return false;
        }

        active.send_paste(text);
        true
    }

    pub fn has_selection(&self) -> bool {
        self.active
            .and_then(|idx| self.sessions.get(idx))
            .is_some_and(TerminalSession::has_selection)
    }

    pub fn clear_selection(&mut self) -> bool {
        self.active_session()
            .is_some_and(TerminalSession::clear_selection)
    }

    pub fn copy_selection(&mut self) -> Option<String> {
        self.active_session()
            .and_then(TerminalSession::copy_selection)
    }

    pub fn take_pending_clipboard_stores(&mut self) -> Vec<TerminalClipboardStore> {
        self.sessions
            .iter_mut()
            .flat_map(TerminalSession::take_pending_clipboard_stores)
            .collect()
    }

    /// Forward a mouse event to the active visible terminal when it has enabled mouse reporting.
    pub fn send_mouse_event(
        &mut self,
        event: MouseEvent,
        content_area: TerminalContentArea,
    ) -> TerminalMouseEventResult {
        let Some(active) = self.active_session() else {
            return TerminalMouseEventResult::Ignored;
        };
        if !active.is_visible() {
            return TerminalMouseEventResult::Ignored;
        }

        let Some(data) = active.mouse_input_bytes(event, content_area) else {
            let selection_result = active.handle_local_mouse_selection(event, content_area);
            if selection_result.is_handled() {
                return selection_result;
            }

            return active.handle_local_mouse_scroll(event, content_area);
        };

        active.send_input(&data);
        TerminalMouseEventResult::Handled
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

    pub fn get_cursor_info(&self) -> TerminalCursorInfo {
        self.active
            .and_then(|idx| self.sessions.get(idx))
            .map(|session| session.get_cursor_info())
            .unwrap_or_default()
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

    #[cfg(test)]
    fn take_pending_pty_write_for_test(&mut self) -> Vec<u8> {
        let idx = self.ensure_active_index();
        let mut pending = self.sessions[idx]
            .pty_write_buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *pending)
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
    use std::sync::{Arc, Mutex};

    struct TestWriter {
        data: Arc<Mutex<Vec<u8>>>,
    }

    impl std::io::Write for TestWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.data
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn content_size_matches_popup_inner_area() {
        assert_eq!(popup_size_for_screen(120, 40, 0.9, 0.9), (108, 36));
        assert_eq!(content_size_for_screen(120, 40, 0.9, 0.9), (34, 106));
        assert_eq!(
            content_area_for_screen(120, 40, 0.9, 0.9),
            TerminalContentArea {
                x: 7,
                y: 3,
                width: 106,
                height: 34,
            }
        );
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
    fn mouse_input_is_ignored_until_terminal_enables_mouse_mode() {
        let session = TerminalSession::new(1, "server".to_string(), 3, 20, PathBuf::from("/tmp"));
        let event = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 5,
            row: 4,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };

        assert_eq!(
            session.mouse_input_bytes(
                event,
                TerminalContentArea {
                    x: 2,
                    y: 3,
                    width: 10,
                    height: 5,
                },
            ),
            None
        );
    }

    #[test]
    fn sgr_mouse_report_uses_content_relative_coordinates() {
        let mut session =
            TerminalSession::new(1, "server".to_string(), 3, 20, PathBuf::from("/tmp"));
        session.process_bytes(b"\x1b[?1000h\x1b[?1006h");
        let event = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 6,
            row: 4,
            modifiers: crossterm::event::KeyModifiers::SHIFT,
        };

        assert_eq!(
            session.mouse_input_bytes(
                event,
                TerminalContentArea {
                    x: 2,
                    y: 3,
                    width: 10,
                    height: 5,
                },
            ),
            Some(b"\x1b[<4;5;2M".to_vec())
        );
    }

    #[test]
    fn drag_mouse_reports_only_when_drag_mode_is_enabled() {
        let event = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
            column: 2,
            row: 3,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        let content_area = TerminalContentArea {
            x: 2,
            y: 3,
            width: 10,
            height: 5,
        };
        let mut session =
            TerminalSession::new(1, "server".to_string(), 3, 20, PathBuf::from("/tmp"));
        session.process_bytes(b"\x1b[?1000h\x1b[?1006h");

        assert_eq!(session.mouse_input_bytes(event, content_area), None);

        session.process_bytes(b"\x1b[?1002h");
        assert_eq!(
            session.mouse_input_bytes(event, content_area),
            Some(b"\x1b[<32;1;1M".to_vec())
        );
    }

    #[test]
    fn mouse_input_outside_terminal_content_is_ignored() {
        let mut session =
            TerminalSession::new(1, "server".to_string(), 3, 20, PathBuf::from("/tmp"));
        session.process_bytes(b"\x1b[?1000h\x1b[?1006h");
        let event = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollDown,
            column: 1,
            row: 3,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };

        assert_eq!(
            session.mouse_input_bytes(
                event,
                TerminalContentArea {
                    x: 2,
                    y: 3,
                    width: 10,
                    height: 5,
                },
            ),
            None
        );
    }

    #[test]
    fn scroll_wheel_scrolls_scrollback_when_mouse_reporting_is_off() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);
        terminal.process_bytes(b"line-1\r\nline-2\r\nline-3\r\nline-4\r\nline-5\r\n");
        terminal.sessions[0].visible = true;

        let before = terminal.get_visible_lines(3, 20);
        let handled = terminal.send_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::ScrollUp,
                column: 2,
                row: 3,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            TerminalContentArea {
                x: 2,
                y: 3,
                width: 20,
                height: 3,
            },
        );

        assert_eq!(handled, TerminalMouseEventResult::Handled);
        assert_ne!(terminal.get_visible_lines(3, 20), before);
    }

    #[test]
    fn mouse_release_keeps_visible_terminal_selection_without_copying() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);
        terminal.process_bytes(b"alpha\r\nbeta\r\ngamma");
        terminal.sessions[0].visible = true;
        let content_area = TerminalContentArea {
            x: 2,
            y: 3,
            width: 20,
            height: 3,
        };

        assert_eq!(
            terminal.send_mouse_event(
                crossterm::event::MouseEvent {
                    kind: crossterm::event::MouseEventKind::Down(
                        crossterm::event::MouseButton::Left,
                    ),
                    column: 2,
                    row: 3,
                    modifiers: crossterm::event::KeyModifiers::NONE,
                },
                content_area,
            ),
            TerminalMouseEventResult::Handled
        );
        assert_eq!(
            terminal.send_mouse_event(
                crossterm::event::MouseEvent {
                    kind: crossterm::event::MouseEventKind::Drag(
                        crossterm::event::MouseButton::Left,
                    ),
                    column: 6,
                    row: 3,
                    modifiers: crossterm::event::KeyModifiers::NONE,
                },
                content_area,
            ),
            TerminalMouseEventResult::Handled
        );

        assert_eq!(
            terminal.send_mouse_event(
                crossterm::event::MouseEvent {
                    kind: crossterm::event::MouseEventKind::Up(
                        crossterm::event::MouseButton::Left,
                    ),
                    column: 6,
                    row: 3,
                    modifiers: crossterm::event::KeyModifiers::NONE,
                },
                content_area,
            ),
            TerminalMouseEventResult::Handled
        );
        assert!(terminal
            .get_visible_cells(3, 20)
            .iter()
            .flatten()
            .any(|cell| cell.selected));
    }

    #[test]
    fn mouse_click_without_drag_does_not_leave_terminal_selection() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);
        terminal.process_bytes(b"alpha\r\nbeta\r\ngamma");
        terminal.sessions[0].visible = true;
        let content_area = TerminalContentArea {
            x: 2,
            y: 3,
            width: 20,
            height: 3,
        };

        terminal.send_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 10,
                row: 4,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            content_area,
        );
        terminal.send_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
                column: 10,
                row: 4,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            content_area,
        );

        assert!(!terminal
            .get_visible_cells(3, 20)
            .iter()
            .flatten()
            .any(|cell| cell.selected));
    }

    #[test]
    fn cursor_position_query_queues_pty_response() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);

        terminal.process_bytes(b"\x1b[6n");

        assert_eq!(terminal.take_pending_pty_write_for_test(), b"\x1b[1;1R");
    }

    #[test]
    fn explicit_terminal_selection_copy_returns_text_and_clears_selection() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);
        terminal.process_bytes(b"alpha\r\nbeta\r\ngamma");
        terminal.sessions[0].visible = true;
        let content_area = TerminalContentArea {
            x: 2,
            y: 3,
            width: 20,
            height: 3,
        };

        terminal.send_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 2,
                row: 3,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            content_area,
        );
        terminal.send_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
                column: 6,
                row: 3,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            content_area,
        );

        assert!(terminal
            .get_visible_cells(3, 20)
            .iter()
            .flatten()
            .any(|cell| cell.selected));

        terminal.send_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
                column: 6,
                row: 3,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            content_area,
        );

        assert_eq!(terminal.copy_selection(), Some("alpha".to_string()));
        assert!(!terminal
            .get_visible_cells(3, 20)
            .iter()
            .flatten()
            .any(|cell| cell.selected));
    }

    #[test]
    fn typing_clears_active_terminal_selection() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);
        terminal.process_bytes(b"alpha\r\nbeta\r\ngamma");
        terminal.sessions[0].visible = true;
        let content_area = TerminalContentArea {
            x: 2,
            y: 3,
            width: 20,
            height: 3,
        };

        terminal.send_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 2,
                row: 3,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            content_area,
        );
        terminal.send_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
                column: 6,
                row: 3,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            content_area,
        );

        assert!(terminal.has_selection());

        terminal.send_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));

        assert!(!terminal.has_selection());
    }

    #[test]
    fn terminal_search_highlights_visible_matches() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);
        terminal.process_bytes(b"alpha beta beta");
        terminal.sessions[0].visible = true;

        assert!(
            terminal.handle_search_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))
        );
        for ch in "beta".chars() {
            assert!(
                terminal.handle_search_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE,))
            );
        }

        let cells = terminal.get_visible_cells(3, 20);
        let highlighted = cells[0]
            .iter()
            .enumerate()
            .filter_map(|(col, cell)| cell.search_match.then_some((col, cell.ch)))
            .collect::<Vec<_>>();
        let active = cells[0]
            .iter()
            .enumerate()
            .filter_map(|(col, cell)| cell.active_search_match.then_some((col, cell.ch)))
            .collect::<Vec<_>>();

        assert_eq!(
            highlighted,
            vec![
                (6, 'b'),
                (7, 'e'),
                (8, 't'),
                (9, 'a'),
                (11, 'b'),
                (12, 'e'),
                (13, 't'),
                (14, 'a'),
            ]
        );
        assert_eq!(active, vec![(6, 'b'), (7, 'e'), (8, 't'), (9, 'a')]);
        assert_eq!(
            terminal.search_status(),
            Some(TerminalSearchStatus {
                active: true,
                query: "beta".to_string(),
                match_count: 2,
                active_match: Some(1),
            })
        );
    }

    #[test]
    fn terminal_search_scrolls_to_scrollback_match() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);
        terminal.process_bytes(
            b"line one\r\nneedle old\r\nline three\r\nline four\r\nline five\r\nline six\r\n",
        );
        terminal.sessions[0].visible = true;

        assert!(!terminal
            .get_visible_lines(3, 20)
            .iter()
            .any(|line| line.contains("needle old")));

        assert!(
            terminal.handle_search_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))
        );
        for ch in "needle".chars() {
            assert!(
                terminal.handle_search_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE,))
            );
        }

        assert!(terminal.sessions[0].display_offset() > 0);
        assert!(terminal
            .get_visible_lines(3, 20)
            .iter()
            .any(|line| line.contains("needle old")));
    }

    #[test]
    fn terminal_search_consumes_keys_without_writing_to_pty() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut terminal = FloatingTerminal::new();
        let idx = terminal.ensure_active_index();
        terminal.sessions[idx].visible = true;

        let captured = Arc::new(Mutex::new(Vec::new()));
        terminal.sessions[idx].pty_writer = Some(Box::new(TestWriter {
            data: captured.clone(),
        }));

        assert!(
            terminal.handle_search_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))
        );
        assert!(terminal.handle_search_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)));
        assert!(terminal.handle_search_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));

        assert_eq!(
            captured
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            b""
        );
        assert_eq!(terminal.search_status(), None);
    }

    #[test]
    fn terminal_search_does_not_intercept_literal_slash() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut terminal = FloatingTerminal::new();
        let idx = terminal.ensure_active_index();
        terminal.sessions[idx].visible = true;

        assert!(!terminal.handle_search_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)));
    }

    #[test]
    fn terminal_search_enter_cycles_active_match() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);
        terminal.process_bytes(b"needle one\r\nneedle two\r\nneedle three");
        terminal.sessions[0].visible = true;

        assert!(
            terminal.handle_search_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))
        );
        for ch in "needle".chars() {
            assert!(
                terminal.handle_search_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE,))
            );
        }
        assert_eq!(terminal.search_status().unwrap().active_match, Some(1));

        assert!(terminal.handle_search_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        assert_eq!(terminal.search_status().unwrap().active_match, Some(2));

        assert!(terminal.handle_search_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)));
        assert_eq!(terminal.search_status().unwrap().active_match, Some(1));
    }

    #[test]
    fn terminal_selection_copy_key_matches_common_copy_shortcuts() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        assert!(is_terminal_selection_copy_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )));
        assert!(is_terminal_selection_copy_key(KeyEvent::new(
            KeyCode::Char('C'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )));
        assert!(is_terminal_selection_copy_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::SUPER
        )));
        assert!(is_terminal_selection_copy_key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE
        )));
        assert!(!is_terminal_selection_copy_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn cursor_info_tracks_child_terminal_cursor_style() {
        let mut terminal = FloatingTerminal::new();

        terminal.process_bytes(b"\x1b[3 q");
        assert_eq!(
            terminal.get_cursor_info().shape,
            TerminalCursorShape::Underline
        );
        assert!(terminal.get_cursor_info().blinking);

        terminal.process_bytes(b"\x1b[6 q");
        assert_eq!(terminal.get_cursor_info().shape, TerminalCursorShape::Beam);
        assert!(!terminal.get_cursor_info().blinking);

        terminal.process_bytes(b"\x1b[?25l");
        assert_eq!(
            terminal.get_cursor_info().shape,
            TerminalCursorShape::Hidden
        );
    }

    #[test]
    fn alt_screen_scroll_sends_arrow_keys_when_mouse_reporting_is_off() {
        let mut terminal = FloatingTerminal::new();
        terminal.resize(3, 20);
        terminal.process_bytes(b"\x1b[?1049h\x1b[?1h");
        terminal.sessions[0].visible = true;

        let captured = Arc::new(Mutex::new(Vec::new()));
        terminal.sessions[0].pty_writer = Some(Box::new(TestWriter {
            data: captured.clone(),
        }));

        let handled = terminal.send_mouse_event(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::ScrollUp,
                column: 2,
                row: 3,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            TerminalContentArea {
                x: 2,
                y: 3,
                width: 20,
                height: 3,
            },
        );

        assert_eq!(handled, TerminalMouseEventResult::Handled);
        assert_eq!(
            captured
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            b"\x1bOA\x1bOA\x1bOA"
        );
    }

    #[test]
    fn common_special_keys_encode_terminal_sequences() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut terminal = FloatingTerminal::new();
        let idx = terminal.ensure_active_index();
        terminal.sessions[idx].visible = true;

        let captured = Arc::new(Mutex::new(Vec::new()));
        terminal.sessions[idx].pty_writer = Some(Box::new(TestWriter {
            data: captured.clone(),
        }));

        terminal.send_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        terminal.send_key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
        terminal.send_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        terminal.send_key(KeyEvent::new(
            KeyCode::Delete,
            KeyModifiers::SHIFT | KeyModifiers::CONTROL,
        ));

        assert_eq!(
            captured
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            b"\x1b[Z\x1b[15~\x1b[1;5C\x1b[3;6~"
        );
    }

    #[test]
    fn paste_uses_bracketed_paste_when_child_terminal_enabled_it() {
        let mut terminal = FloatingTerminal::new();
        terminal.process_bytes(b"\x1b[?2004h");
        terminal.sessions[0].visible = true;

        let captured = Arc::new(Mutex::new(Vec::new()));
        terminal.sessions[0].pty_writer = Some(Box::new(TestWriter {
            data: captured.clone(),
        }));

        assert!(terminal.send_paste("total 40\n"));
        assert_eq!(
            captured
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            b"\x1b[200~total 40\n\x1b[201~"
        );
    }

    #[test]
    fn paste_uses_raw_text_when_child_terminal_has_not_enabled_bracketed_paste() {
        let mut terminal = FloatingTerminal::new();
        let idx = terminal.ensure_active_index();
        terminal.sessions[idx].visible = true;

        let captured = Arc::new(Mutex::new(Vec::new()));
        terminal.sessions[idx].pty_writer = Some(Box::new(TestWriter {
            data: captured.clone(),
        }));

        assert!(terminal.send_paste("plain text"));
        assert_eq!(
            captured
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            b"plain text"
        );
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
    fn osc52_clipboard_store_is_queued_for_editor_clipboard() {
        let mut terminal = FloatingTerminal::new();

        terminal.process_bytes(b"\x1b]52;c;aGVsbG8gdGVybWluYWw=\x07");

        assert_eq!(
            terminal.take_pending_clipboard_stores(),
            vec![TerminalClipboardStore {
                clipboard: TerminalClipboard::Clipboard,
                text: "hello terminal".to_string(),
            }]
        );
        assert!(terminal.take_pending_clipboard_stores().is_empty());
    }

    #[test]
    fn osc52_selection_store_is_queued_for_editor_clipboard() {
        let mut terminal = FloatingTerminal::new();

        terminal.process_bytes(b"\x1b]52;p;c2VsZWN0aW9u\x07");

        assert_eq!(
            terminal.take_pending_clipboard_stores(),
            vec![TerminalClipboardStore {
                clipboard: TerminalClipboard::Selection,
                text: "selection".to_string(),
            }]
        );
    }

    #[test]
    fn osc52_invalid_payload_is_ignored() {
        let mut terminal = FloatingTerminal::new();

        terminal.process_bytes(b"\x1b]52;c;not-valid-base64!\x07");

        assert!(terminal.take_pending_clipboard_stores().is_empty());
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
