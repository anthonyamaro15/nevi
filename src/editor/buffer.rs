use ropey::Rope;
use std::path::PathBuf;

/// A text buffer backed by a rope data structure.
/// Ropes provide O(log n) insertions and deletions, making them
/// ideal for text editors.
pub struct Buffer {
    /// The text content
    text: Rope,
    /// File path (None if unsaved new buffer)
    pub path: Option<PathBuf>,
    /// Whether the buffer has unsaved changes
    pub dirty: bool,
    /// Monotonic version for change tracking
    version: u64,
}

impl Buffer {
    /// Create a new empty buffer
    pub fn new() -> Self {
        Self {
            text: Rope::new(),
            path: None,
            dirty: false,
            version: 0,
        }
    }

    /// Create a buffer from a file
    pub fn from_file(path: PathBuf) -> anyhow::Result<Self> {
        let text = if path.exists() {
            Rope::from_reader(std::fs::File::open(&path)?)?
        } else {
            // New file that doesn't exist yet
            Rope::new()
        };

        Ok(Self {
            text,
            path: Some(path),
            dirty: false,
            version: 0,
        })
    }

    /// Save buffer to its file path
    pub fn save(&mut self) -> anyhow::Result<()> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No file path set"))?;

        let file = std::fs::File::create(path)?;
        self.text.write_to(std::io::BufWriter::new(file))?;
        self.dirty = false;
        Ok(())
    }

    /// Get total number of lines
    pub fn len_lines(&self) -> usize {
        self.text.len_lines()
    }

    /// Get a specific line (0-indexed)
    pub fn line(&self, idx: usize) -> Option<ropey::RopeSlice<'_>> {
        if idx < self.text.len_lines() {
            Some(self.text.line(idx))
        } else {
            None
        }
    }

    /// Get the length of a specific line (excluding newline)
    pub fn line_len(&self, idx: usize) -> usize {
        self.line(idx)
            .map(|l| {
                let len = l.len_chars();
                // Subtract newline if present
                if len > 0 && l.char(len - 1) == '\n' {
                    len - 1
                } else {
                    len
                }
            })
            .unwrap_or(0)
    }

    /// Get the length of a specific line including trailing newline if present
    pub fn line_len_including_newline(&self, idx: usize) -> usize {
        self.line(idx).map(|l| l.len_chars()).unwrap_or(0)
    }

    /// Get the current version of the buffer
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Get the full content of the buffer as a string
    pub fn content(&self) -> String {
        self.text.to_string()
    }

    /// Get the char index for a given line and column
    pub fn line_col_to_char(&self, line: usize, col: usize) -> usize {
        let line_start = self.text.line_to_char(line);
        line_start + col
    }

    /// Insert a character at the given line and column
    pub fn insert_char(&mut self, line: usize, col: usize, ch: char) {
        let idx = self.line_col_to_char(line, col);
        self.text.insert_char(idx, ch);
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
    }

    /// Insert a string at the given line and column
    pub fn insert_str(&mut self, line: usize, col: usize, s: &str) {
        let idx = self.line_col_to_char(line, col);
        self.text.insert(idx, s);
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
    }

    /// Delete a character at the given line and column
    pub fn delete_char(&mut self, line: usize, col: usize) {
        let idx = self.line_col_to_char(line, col);
        if idx < self.text.len_chars() {
            self.text.remove(idx..idx + 1);
            self.dirty = true;
            self.version = self.version.wrapping_add(1);
        }
    }

    /// Delete a range of characters
    pub fn delete_range(&mut self, start_line: usize, start_col: usize, end_line: usize, end_col: usize) {
        let start = self.line_col_to_char(start_line, start_col);
        let end = self.line_col_to_char(end_line, end_col);
        if start < end && end <= self.text.len_chars() {
            self.text.remove(start..end);
            self.dirty = true;
            self.version = self.version.wrapping_add(1);
        }
    }

    /// Get the character at a position
    pub fn char_at(&self, line: usize, col: usize) -> Option<char> {
        let idx = self.line_col_to_char(line, col);
        if idx < self.text.len_chars() {
            Some(self.text.char(idx))
        } else {
            None
        }
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.text.len_chars() == 0
    }

    /// Get total character count
    pub fn len_chars(&self) -> usize {
        self.text.len_chars()
    }

    /// Get the display name for the buffer
    pub fn display_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| "[No Name]".to_string())
    }

    /// Get text in a range as a string
    pub fn get_text_range(&self, start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> String {
        let start = self.line_col_to_char(start_line, start_col);
        let end = self.line_col_to_char(end_line, end_col);
        if start < end && end <= self.text.len_chars() {
            self.text.slice(start..end).to_string()
        } else {
            String::new()
        }
    }

    /// Get a single character as a string
    pub fn get_char_str(&self, line: usize, col: usize) -> String {
        self.char_at(line, col).map(|c| c.to_string()).unwrap_or_default()
    }

    /// Get leading whitespace from a line
    pub fn get_line_indent(&self, line_idx: usize) -> String {
        let Some(line) = self.line(line_idx) else {
            return String::new();
        };
        line.chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    /// Check if line ends with a character (ignoring trailing whitespace/newline)
    pub fn line_ends_with(&self, line_idx: usize, target: char) -> bool {
        let Some(line) = self.line(line_idx) else {
            return false;
        };
        // Collect to string and iterate in reverse
        let line_str: String = line.chars().collect();
        for ch in line_str.chars().rev() {
            if ch == '\n' || ch == ' ' || ch == '\t' {
                continue;
            }
            return ch == target;
        }
        false
    }

    /// Apply text changes for undo/redo
    /// Deletes old_text at position and inserts new_text
    pub fn apply_change(&mut self, line: usize, col: usize, old_text: &str, new_text: &str) {
        let idx = self.line_col_to_char(line, col);

        // Delete old text if any
        if !old_text.is_empty() {
            let end_idx = idx + old_text.chars().count();
            if end_idx <= self.text.len_chars() {
                self.text.remove(idx..end_idx);
            }
        }

        // Insert new text if any
        if !new_text.is_empty() {
            self.text.insert(idx, new_text);
        }

        self.dirty = true;
        self.version = self.version.wrapping_add(1);
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}
