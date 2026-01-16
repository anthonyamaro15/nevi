/// Cursor position in the buffer (0-indexed)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    /// Line number (0-indexed)
    pub line: usize,
    /// Column number (0-indexed)
    pub col: usize,
}

impl Cursor {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }

    /// Move cursor up by n lines
    pub fn move_up(&mut self, n: usize) {
        self.line = self.line.saturating_sub(n);
    }

    /// Move cursor down by n lines (caller should clamp to buffer length)
    pub fn move_down(&mut self, n: usize) {
        self.line = self.line.saturating_add(n);
    }

    /// Move cursor left by n columns
    pub fn move_left(&mut self, n: usize) {
        self.col = self.col.saturating_sub(n);
    }

    /// Move cursor right by n columns (caller should clamp to line length)
    pub fn move_right(&mut self, n: usize) {
        self.col = self.col.saturating_add(n);
    }

    /// Set cursor to start of line
    pub fn move_to_line_start(&mut self) {
        self.col = 0;
    }

    /// Set cursor to a specific position
    pub fn set(&mut self, line: usize, col: usize) {
        self.line = line;
        self.col = col;
    }
}
