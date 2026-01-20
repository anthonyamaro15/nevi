//! Floating terminal implementation using PTY
//!
//! Provides a toggleable floating terminal window that runs the user's shell.

use portable_pty::{native_pty_system, CommandBuilder, PtySize, MasterPty, Child};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

/// Terminal buffer size (scrollback)
const BUFFER_ROWS: usize = 1000;
const BUFFER_COLS: usize = 200;

/// Floating terminal state
pub struct FloatingTerminal {
    /// Whether the terminal is currently visible
    pub visible: bool,
    /// Terminal content buffer (rows of characters)
    buffer: Vec<Vec<char>>,
    /// Current cursor row in buffer
    cursor_row: usize,
    /// Current cursor column in buffer
    cursor_col: usize,
    /// Scroll offset (for scrollback)
    scroll_offset: usize,
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
    /// Working directory
    working_dir: PathBuf,
    /// Whether terminal process has exited
    process_exited: bool,
}

impl FloatingTerminal {
    /// Create a new floating terminal (not yet spawned)
    pub fn new() -> Self {
        Self {
            visible: false,
            buffer: vec![vec![' '; BUFFER_COLS]; BUFFER_ROWS],
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
            rows: 24,
            cols: 80,
            pty_master: None,
            pty_writer: None,
            child: None,
            output_buffer: Arc::new(Mutex::new(Vec::new())),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            process_exited: false,
        }
    }

    /// Set the working directory for the terminal
    pub fn set_working_dir(&mut self, path: PathBuf) {
        self.working_dir = path;
    }

    /// Toggle terminal visibility
    pub fn toggle(&mut self) -> bool {
        if self.visible {
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

        // Spawn the shell
        let child = pair.slave.spawn_command(cmd)?;

        // Set up reader thread
        let mut reader = pair.master.try_clone_reader()?;
        let output_buffer = self.output_buffer.clone();

        thread::spawn(move || {
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
        self.process_exited = false;

        // Clear buffer for fresh start
        self.buffer = vec![vec![' '; BUFFER_COLS]; BUFFER_ROWS];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;

        Ok(())
    }

    /// Resize the terminal
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;

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
            let _ = writer.write_all(data);
            let _ = writer.flush();
        }
    }

    /// Send a key to the terminal
    pub fn send_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

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
            (_, KeyCode::Up) => vec![0x1b, b'[', b'A'],
            (_, KeyCode::Down) => vec![0x1b, b'[', b'B'],
            (_, KeyCode::Right) => vec![0x1b, b'[', b'C'],
            (_, KeyCode::Left) => vec![0x1b, b'[', b'D'],
            (_, KeyCode::Home) => vec![0x1b, b'[', b'H'],
            (_, KeyCode::End) => vec![0x1b, b'[', b'F'],
            (_, KeyCode::PageUp) => vec![0x1b, b'[', b'5', b'~'],
            (_, KeyCode::PageDown) => vec![0x1b, b'[', b'6', b'~'],
            (_, KeyCode::Delete) => vec![0x1b, b'[', b'3', b'~'],
            (_, KeyCode::Insert) => vec![0x1b, b'[', b'2', b'~'],
            // Regular characters
            (_, KeyCode::Char(c)) => {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                s.as_bytes().to_vec()
            }
            _ => return,
        };

        self.send_input(&data);
    }

    /// Process output from the terminal and update buffer
    /// Returns true if there was new output to process
    pub fn process_output(&mut self) -> bool {
        // Check if process has exited
        if let Some(ref mut child) = self.child {
            if let Ok(Some(_)) = child.try_wait() {
                self.process_exited = true;
            }
        }

        // Get output from buffer (recover from poisoned mutex if needed)
        let data = {
            let mut output = self.output_buffer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if output.is_empty() {
                return false;
            }
            std::mem::take(&mut *output)
        };

        // Process the output bytes
        self.process_bytes(&data);
        true
    }

    /// Process bytes and update terminal buffer
    fn process_bytes(&mut self, data: &[u8]) {
        let mut i = 0;
        while i < data.len() {
            let b = data[i];

            if b == 0x1b && i + 1 < data.len() {
                // Escape sequence
                if data[i + 1] == b'[' {
                    // CSI sequence
                    let (consumed, _) = self.process_csi(&data[i..]);
                    i += consumed;
                    continue;
                } else if data[i + 1] == b']' {
                    // OSC sequence (e.g., window title) - skip until ST or BEL
                    i += 2;
                    while i < data.len() && data[i] != 0x07 && data[i] != 0x1b {
                        i += 1;
                    }
                    if i < data.len() && data[i] == 0x07 {
                        i += 1;
                    }
                    continue;
                }
            }

            match b {
                0x07 => {} // Bell - ignore
                0x08 => {
                    // Backspace
                    if self.cursor_col > 0 {
                        self.cursor_col -= 1;
                    }
                }
                0x09 => {
                    // Tab
                    self.cursor_col = (self.cursor_col + 8) & !7;
                    if self.cursor_col >= self.cols as usize {
                        self.cursor_col = self.cols as usize - 1;
                    }
                }
                0x0a => {
                    // Line feed
                    self.cursor_row += 1;
                    if self.cursor_row >= self.rows as usize {
                        self.scroll_up();
                        self.cursor_row = self.rows as usize - 1;
                    }
                }
                0x0d => {
                    // Carriage return
                    self.cursor_col = 0;
                }
                0x1b => {
                    // Standalone escape - skip
                }
                _ => {
                    // Regular character
                    if b >= 0x20 {
                        // Handle UTF-8
                        let (ch, consumed) = self.decode_utf8(&data[i..]);
                        if self.cursor_col < self.cols as usize {
                            let row = self.cursor_row + self.scroll_offset;
                            if row < BUFFER_ROWS && self.cursor_col < BUFFER_COLS {
                                self.buffer[row][self.cursor_col] = ch;
                            }
                            self.cursor_col += 1;
                        }
                        i += consumed - 1; // -1 because loop will increment
                    }
                }
            }
            i += 1;
        }
    }

    /// Decode a UTF-8 character from bytes
    fn decode_utf8(&self, data: &[u8]) -> (char, usize) {
        if data.is_empty() {
            return (' ', 1);
        }

        let b = data[0];
        if b < 0x80 {
            (b as char, 1)
        } else if b < 0xe0 && data.len() >= 2 {
            let cp = ((b as u32 & 0x1f) << 6) | (data[1] as u32 & 0x3f);
            (char::from_u32(cp).unwrap_or(' '), 2)
        } else if b < 0xf0 && data.len() >= 3 {
            let cp = ((b as u32 & 0x0f) << 12)
                | ((data[1] as u32 & 0x3f) << 6)
                | (data[2] as u32 & 0x3f);
            (char::from_u32(cp).unwrap_or(' '), 3)
        } else if data.len() >= 4 {
            let cp = ((b as u32 & 0x07) << 18)
                | ((data[1] as u32 & 0x3f) << 12)
                | ((data[2] as u32 & 0x3f) << 6)
                | (data[3] as u32 & 0x3f);
            (char::from_u32(cp).unwrap_or(' '), 4)
        } else {
            (' ', 1)
        }
    }

    /// Process CSI (Control Sequence Introducer) escape sequence
    fn process_csi(&mut self, data: &[u8]) -> (usize, bool) {
        // Find the end of the CSI sequence
        let mut i = 2; // Skip ESC [
        while i < data.len() {
            let b = data[i];
            if b >= 0x40 && b <= 0x7e {
                // Final byte found
                break;
            }
            i += 1;
        }

        if i >= data.len() {
            return (data.len(), false);
        }

        let final_byte = data[i] as char;
        let params_str = String::from_utf8_lossy(&data[2..i]);
        let params: Vec<usize> = params_str
            .split(';')
            .filter_map(|s| s.parse().ok())
            .collect();

        match final_byte {
            'H' | 'f' => {
                // Cursor position
                let row = params.first().copied().unwrap_or(1).saturating_sub(1);
                let col = params.get(1).copied().unwrap_or(1).saturating_sub(1);
                self.cursor_row = row.min(self.rows as usize - 1);
                self.cursor_col = col.min(self.cols as usize - 1);
            }
            'A' => {
                // Cursor up
                let n = params.first().copied().unwrap_or(1);
                self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            'B' => {
                // Cursor down
                let n = params.first().copied().unwrap_or(1);
                self.cursor_row = (self.cursor_row + n).min(self.rows as usize - 1);
            }
            'C' => {
                // Cursor forward
                let n = params.first().copied().unwrap_or(1);
                self.cursor_col = (self.cursor_col + n).min(self.cols as usize - 1);
            }
            'D' => {
                // Cursor backward
                let n = params.first().copied().unwrap_or(1);
                self.cursor_col = self.cursor_col.saturating_sub(n);
            }
            'J' => {
                // Erase display
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    0 => self.clear_from_cursor(),
                    1 => self.clear_to_cursor(),
                    2 | 3 => self.clear_screen(),
                    _ => {}
                }
            }
            'K' => {
                // Erase line
                let mode = params.first().copied().unwrap_or(0);
                let row = self.cursor_row + self.scroll_offset;
                if row < BUFFER_ROWS {
                    match mode {
                        0 => {
                            // Clear from cursor to end of line
                            for col in self.cursor_col..self.cols as usize {
                                if col < BUFFER_COLS {
                                    self.buffer[row][col] = ' ';
                                }
                            }
                        }
                        1 => {
                            // Clear from start to cursor
                            for col in 0..=self.cursor_col {
                                if col < BUFFER_COLS {
                                    self.buffer[row][col] = ' ';
                                }
                            }
                        }
                        2 => {
                            // Clear entire line
                            for col in 0..self.cols as usize {
                                if col < BUFFER_COLS {
                                    self.buffer[row][col] = ' ';
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            'm' => {
                // SGR (colors/styles) - ignore for now, could add color support later
            }
            'r' => {
                // Set scrolling region - ignore
            }
            'h' | 'l' => {
                // Mode set/reset - ignore
            }
            _ => {}
        }

        (i + 1, true)
    }

    /// Scroll the buffer up by one line
    fn scroll_up(&mut self) {
        self.scroll_offset += 1;
        if self.scroll_offset + self.rows as usize >= BUFFER_ROWS {
            // Wrap around - shift buffer
            let keep_rows = BUFFER_ROWS / 2;
            for i in 0..keep_rows {
                self.buffer[i] = self.buffer[self.scroll_offset + i].clone();
            }
            for i in keep_rows..BUFFER_ROWS {
                self.buffer[i] = vec![' '; BUFFER_COLS];
            }
            self.scroll_offset = 0;
        }
        // Clear new line
        let new_row = self.scroll_offset + self.rows as usize - 1;
        if new_row < BUFFER_ROWS {
            self.buffer[new_row] = vec![' '; BUFFER_COLS];
        }
    }

    /// Clear screen
    fn clear_screen(&mut self) {
        for row in 0..self.rows as usize {
            let buf_row = self.scroll_offset + row;
            if buf_row < BUFFER_ROWS {
                self.buffer[buf_row] = vec![' '; BUFFER_COLS];
            }
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Clear from cursor to end of screen
    fn clear_from_cursor(&mut self) {
        let row = self.cursor_row + self.scroll_offset;
        if row < BUFFER_ROWS {
            for col in self.cursor_col..self.cols as usize {
                if col < BUFFER_COLS {
                    self.buffer[row][col] = ' ';
                }
            }
        }
        for r in (self.cursor_row + 1)..self.rows as usize {
            let buf_row = self.scroll_offset + r;
            if buf_row < BUFFER_ROWS {
                self.buffer[buf_row] = vec![' '; BUFFER_COLS];
            }
        }
    }

    /// Clear from start of screen to cursor
    fn clear_to_cursor(&mut self) {
        for r in 0..self.cursor_row {
            let buf_row = self.scroll_offset + r;
            if buf_row < BUFFER_ROWS {
                self.buffer[buf_row] = vec![' '; BUFFER_COLS];
            }
        }
        let row = self.cursor_row + self.scroll_offset;
        if row < BUFFER_ROWS {
            for col in 0..=self.cursor_col {
                if col < BUFFER_COLS {
                    self.buffer[row][col] = ' ';
                }
            }
        }
    }

    /// Get visible lines for rendering
    pub fn get_visible_lines(&self, rows: usize, cols: usize) -> Vec<String> {
        let mut lines = Vec::with_capacity(rows);
        for r in 0..rows {
            let buf_row = self.scroll_offset + r;
            if buf_row < BUFFER_ROWS {
                let line: String = self.buffer[buf_row]
                    .iter()
                    .take(cols)
                    .collect();
                lines.push(line.trim_end().to_string());
            } else {
                lines.push(String::new());
            }
        }
        lines
    }

    /// Get cursor position within visible area
    pub fn get_cursor_pos(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    /// Check if terminal is visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Close the terminal (kill process)
    pub fn close(&mut self) {
        self.visible = false;
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
        }
        self.child = None;
        self.pty_master = None;
        self.process_exited = true;
    }
}

impl Default for FloatingTerminal {
    fn default() -> Self {
        Self::new()
    }
}
