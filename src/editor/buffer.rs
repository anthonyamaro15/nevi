use ropey::Rope;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Unicode scalar values taken from the first line for shebang detection.
const FIRST_LINE_PREFIX_CHARS: usize = 256;

/// A text mutation in tree-sitter's InputEdit shape: byte offsets plus
/// (row, byte-column) points. Recorded on every change so the syntax layer
/// can reparse incrementally instead of from scratch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start_point: (usize, usize),
    pub old_end_point: (usize, usize),
    pub new_end_point: (usize, usize),
}

/// Beyond this the queue is dropped and the next parse is a full one; an
/// unparsed buffer (no language) must not accumulate edits forever.
const MAX_PENDING_EDITS: usize = 10_000;

/// Sentinel for a broken edit history (bulk replace, overflow): no version
/// can match it, so the next parse falls back to a full reparse.
const EDIT_HISTORY_BROKEN: u64 = u64::MAX;

static NEXT_BUFFER_ID: AtomicU64 = AtomicU64::new(1);

fn next_buffer_id() -> u64 {
    NEXT_BUFFER_ID.fetch_add(1, Ordering::Relaxed)
}

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
    /// Last known modification time of the file on disk (for autoread)
    last_mtime: Option<SystemTime>,
    kind: BufferKind,
    /// Unique identity, so the syntax layer only reuses a tree for the
    /// buffer that produced it
    id: u64,
    /// Text edits since edits_base_version, drained by incremental reparse
    pending_edits: Vec<BufferEdit>,
    /// Buffer version where pending_edits starts; EDIT_HISTORY_BROKEN after
    /// a bulk replace or queue overflow
    edits_base_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BufferKind {
    File {
        read_only: bool,
    },
    Untitled,
    Virtual {
        name: String,
        read_only: bool,
        syntax_hint_path: Option<PathBuf>,
    },
}

impl Buffer {
    /// Create a new empty buffer
    pub fn new() -> Self {
        Self {
            id: next_buffer_id(),
            pending_edits: Vec::new(),
            edits_base_version: 0,
            text: Rope::new(),
            path: None,
            dirty: false,
            version: 0,
            last_mtime: None,
            kind: BufferKind::Untitled,
        }
    }

    /// Create a buffer from a file
    pub fn from_file(path: PathBuf) -> anyhow::Result<Self> {
        Self::from_file_with_read_only(path, false)
    }

    /// Create a read-only buffer from a file.
    pub fn from_file_read_only(path: PathBuf) -> anyhow::Result<Self> {
        Self::from_file_with_read_only(path, true)
    }

    fn from_file_with_read_only(path: PathBuf, read_only: bool) -> anyhow::Result<Self> {
        let (text, last_mtime) = if path.exists() {
            let mtime = std::fs::metadata(&path)?.modified().ok();
            let mut rope = Rope::from_reader(std::fs::File::open(&path)?)?;
            Self::fix_missing_final_newline(&mut rope);
            (rope, mtime)
        } else {
            // New file that doesn't exist yet
            (Rope::new(), None)
        };

        Ok(Self {
            id: next_buffer_id(),
            pending_edits: Vec::new(),
            edits_base_version: 0,
            text,
            path: Some(path),
            dirty: false,
            version: 0,
            last_mtime,
            kind: BufferKind::File { read_only },
        })
    }

    /// Create a named virtual buffer whose content is not backed by a file.
    pub fn virtual_read_only(
        name: impl Into<String>,
        content: &str,
        syntax_hint_path: Option<PathBuf>,
    ) -> Self {
        Self {
            id: next_buffer_id(),
            pending_edits: Vec::new(),
            edits_base_version: 0,
            text: Rope::from_str(content),
            path: None,
            dirty: false,
            version: 0,
            last_mtime: None,
            kind: BufferKind::Virtual {
                name: name.into(),
                read_only: true,
                syntax_hint_path,
            },
        }
    }

    /// Create a named editable virtual buffer (e.g. the macro-lens edit view).
    pub fn virtual_writable(
        name: impl Into<String>,
        content: &str,
        syntax_hint_path: Option<PathBuf>,
    ) -> Self {
        Self {
            id: next_buffer_id(),
            pending_edits: Vec::new(),
            edits_base_version: 0,
            text: Rope::from_str(content),
            path: None,
            dirty: false,
            version: 0,
            last_mtime: None,
            kind: BufferKind::Virtual {
                name: name.into(),
                read_only: false,
                syntax_hint_path,
            },
        }
    }

    /// Whether this buffer should reject direct content changes.
    pub fn is_read_only(&self) -> bool {
        matches!(
            self.kind,
            BufferKind::File { read_only: true }
                | BufferKind::Virtual {
                    read_only: true,
                    ..
                }
        )
    }

    /// Update read-only state for file-backed buffers.
    pub fn set_read_only(&mut self, read_only: bool) {
        if let BufferKind::File {
            read_only: ref mut file_read_only,
        } = self.kind
        {
            *file_read_only = read_only;
        }
    }

    /// Whether this is a file-backed buffer.
    pub fn is_file_backed(&self) -> bool {
        matches!(self.kind, BufferKind::File { .. })
    }

    /// Mark this buffer as file-backed.
    pub fn set_file_path(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.kind = BufferKind::File { read_only: false };
    }

    /// Path used for syntax detection. Virtual buffers may provide a synthetic hint.
    pub fn syntax_hint_path(&self) -> Option<&PathBuf> {
        match &self.kind {
            BufferKind::Virtual {
                syntax_hint_path, ..
            } => syntax_hint_path.as_ref(),
            BufferKind::File { .. } | BufferKind::Untitled => self.path.as_ref(),
        }
    }

    /// Save buffer to its file path
    pub fn save(&mut self) -> anyhow::Result<()> {
        if self.is_read_only() {
            anyhow::bail!("Buffer is read-only");
        }

        let path = self
            .path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No file path set"))?;

        write_file_atomically(path, |writer| {
            self.text.write_to(writer)?;
            Ok(())
        })?;
        self.dirty = false;

        // Update mtime after save
        if let Some(ref p) = self.path {
            self.last_mtime = std::fs::metadata(p).ok().and_then(|m| m.modified().ok());
        }
        Ok(())
    }

    /// Check if the file has been modified externally since we last loaded/saved it
    pub fn has_external_changes(&self) -> bool {
        let Some(ref path) = self.path else {
            return false;
        };
        let Some(last_mtime) = self.last_mtime else {
            return false;
        };

        if let Ok(metadata) = std::fs::metadata(path) {
            if let Ok(current_mtime) = metadata.modified() {
                return current_mtime > last_mtime;
            }
        }
        false
    }

    /// Reload buffer content from disk
    pub fn reload(&mut self) -> anyhow::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No file path set"))?;

        if path.exists() {
            self.text = Rope::from_reader(std::fs::File::open(&path)?)?;
            Self::fix_missing_final_newline(&mut self.text);
            self.break_edit_history();
            self.last_mtime = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            self.dirty = false;
            self.version = self.version.wrapping_add(1);
        }
        Ok(())
    }

    /// Neovim parity ('fixendofline' default): a non-empty buffer whose text
    /// does not end in a newline gets one appended, so the missing-final-
    /// newline state is unrepresentable while editing. Without this, opening
    /// a line at end of file parks the cursor on the rope's trailing
    /// sentinel line (fuzz-found), and saving writes the newline exactly
    /// like Neovim does.
    fn fix_missing_final_newline(text: &mut Rope) {
        let len = text.len_chars();
        if len > 0 && text.char(len - 1) != '\n' {
            text.insert(len, "\n");
        }
    }

    /// Get total number of lines
    pub fn len_lines(&self) -> usize {
        self.text.len_lines()
    }

    /// Get the number of user-addressable lines.
    ///
    /// Ropey represents a trailing newline with an additional empty line slice.
    /// That slice is a storage boundary, not a line the cursor can enter or the
    /// terminal should render. An empty buffer still has one addressable line.
    pub fn addressable_line_count(&self) -> usize {
        let line_count = self.text.len_lines();
        if line_count > 1 && self.text.line(line_count - 1).len_chars() == 0 {
            line_count - 1
        } else {
            line_count
        }
    }

    /// Get a specific line (0-indexed)
    pub fn line(&self, idx: usize) -> Option<ropey::RopeSlice<'_>> {
        if idx < self.text.len_lines() {
            Some(self.text.line(idx))
        } else {
            None
        }
    }

    /// Prefix of the first line, including a trailing newline when it falls
    /// inside the cap. Capped at [`FIRST_LINE_PREFIX_CHARS`] Unicode scalar values.
    pub fn first_line_prefix(&self) -> Option<String> {
        self.line(0)
            .map(|line| line.chars().take(FIRST_LINE_PREFIX_CHARS).collect())
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

    /// Replace the entire buffer content with new text
    /// Used by external formatters to apply formatting
    pub fn set_content(&mut self, content: &str) {
        if self.is_read_only() {
            return;
        }
        self.text = Rope::from_str(content);
        Self::fix_missing_final_newline(&mut self.text);
        self.break_edit_history();
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
    }

    /// Buffer identity for syntax-tree reuse guards.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Byte offset and (row, byte-column) point of a char index, in the
    /// rope's CURRENT state. Callers capture old-state values before
    /// mutating and new-state values after.
    fn point_at_char(&self, char_idx: usize) -> (usize, (usize, usize)) {
        let char_idx = char_idx.min(self.text.len_chars());
        let byte = self.text.char_to_byte(char_idx);
        let line = self.text.char_to_line(char_idx);
        (byte, (line, byte - self.text.line_to_byte(line)))
    }

    fn push_edit(&mut self, edit: BufferEdit) {
        if self.edits_base_version == EDIT_HISTORY_BROKEN {
            return;
        }
        if self.pending_edits.len() >= MAX_PENDING_EDITS {
            self.break_edit_history();
            return;
        }
        self.pending_edits.push(edit);
    }

    fn break_edit_history(&mut self) {
        self.pending_edits.clear();
        self.edits_base_version = EDIT_HISTORY_BROKEN;
    }

    /// Drain the edits covering exactly (since_version ..= now). Returns
    /// None when the history does not start at since_version (bulk replace,
    /// overflow, or a different consumer drained it); the caller must then
    /// do a full reparse and call reset_edit_history.
    pub fn take_edits_since(&mut self, since_version: u64) -> Option<Vec<BufferEdit>> {
        if self.edits_base_version == since_version {
            self.edits_base_version = self.version;
            Some(std::mem::take(&mut self.pending_edits))
        } else {
            None
        }
    }

    /// Restart edit history from the current version, after a full parse.
    pub fn reset_edit_history(&mut self) {
        self.pending_edits.clear();
        self.edits_base_version = self.version;
    }

    /// Get the char index for a given line and column
    pub fn line_col_to_char(&self, line: usize, col: usize) -> usize {
        if line >= self.text.len_lines() {
            return self.text.len_chars();
        }

        let line_start = self.text.line_to_char(line);
        let max_col = self.text.line(line).len_chars();
        line_start + col.min(max_col)
    }

    /// Insert a character at the given line and column
    pub fn insert_char(&mut self, line: usize, col: usize, ch: char) {
        if self.is_read_only() {
            return;
        }
        let idx = self.line_col_to_char(line, col);
        let (start_byte, start_point) = self.point_at_char(idx);
        self.text.insert_char(idx, ch);
        let (new_end_byte, new_end_point) = self.point_at_char(idx + 1);
        self.push_edit(BufferEdit {
            start_byte,
            old_end_byte: start_byte,
            new_end_byte,
            start_point,
            old_end_point: start_point,
            new_end_point,
        });
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
    }

    /// Insert a string at the given line and column
    pub fn insert_str(&mut self, line: usize, col: usize, s: &str) {
        if self.is_read_only() {
            return;
        }
        let idx = self.line_col_to_char(line, col);
        let (start_byte, start_point) = self.point_at_char(idx);
        self.text.insert(idx, s);
        let (new_end_byte, new_end_point) = self.point_at_char(idx + s.chars().count());
        self.push_edit(BufferEdit {
            start_byte,
            old_end_byte: start_byte,
            new_end_byte,
            start_point,
            old_end_point: start_point,
            new_end_point,
        });
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
    }

    /// Delete a character at the given line and column
    pub fn delete_char(&mut self, line: usize, col: usize) {
        if self.is_read_only() {
            return;
        }
        let idx = self.line_col_to_char(line, col);
        if idx < self.text.len_chars() {
            let (start_byte, start_point) = self.point_at_char(idx);
            let (old_end_byte, old_end_point) = self.point_at_char(idx + 1);
            self.text.remove(idx..idx + 1);
            self.push_edit(BufferEdit {
                start_byte,
                old_end_byte,
                new_end_byte: start_byte,
                start_point,
                old_end_point,
                new_end_point: start_point,
            });
            self.dirty = true;
            self.version = self.version.wrapping_add(1);
        }
    }

    /// Delete a range of characters
    pub fn delete_range(
        &mut self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) {
        if self.is_read_only() {
            return;
        }
        let start = self.line_col_to_char(start_line, start_col);
        let end = self.line_col_to_char(end_line, end_col);
        if start < end && end <= self.text.len_chars() {
            let (start_byte, start_point) = self.point_at_char(start);
            let (old_end_byte, old_end_point) = self.point_at_char(end);
            self.text.remove(start..end);
            self.push_edit(BufferEdit {
                start_byte,
                old_end_byte,
                new_end_byte: start_byte,
                start_point,
                old_end_point,
                new_end_point: start_point,
            });
            self.dirty = true;
            self.version = self.version.wrapping_add(1);
        }
    }

    /// Replace an entire line with new content
    pub fn replace_line(&mut self, line: usize, new_content: &str) {
        if self.is_read_only() {
            return;
        }
        if line >= self.text.len_lines() {
            return;
        }

        // Get the start and end char indices for this line
        let start_idx = self.text.line_to_char(line);
        let end_idx = if line + 1 < self.text.len_lines() {
            self.text.line_to_char(line + 1)
        } else {
            self.text.len_chars()
        };

        let (start_byte, start_point) = self.point_at_char(start_idx);
        let (old_end_byte, old_end_point) = self.point_at_char(end_idx);

        // Remove the old line content
        if start_idx < end_idx {
            self.text.remove(start_idx..end_idx);
        }

        // Insert new content (preserve newline handling)
        let content_to_insert = if line + 1 < self.len_lines() || new_content.ends_with('\n') {
            new_content.to_string()
        } else if line == self.len_lines().saturating_sub(1) && !new_content.ends_with('\n') {
            // Last line, no newline needed
            new_content.to_string()
        } else {
            format!("{}\n", new_content.trim_end_matches('\n'))
        };

        self.text.insert(start_idx, &content_to_insert);
        let (new_end_byte, new_end_point) =
            self.point_at_char(start_idx + content_to_insert.chars().count());
        self.push_edit(BufferEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_point,
            old_end_point,
            new_end_point,
        });
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
    }

    /// Mark the buffer as modified
    pub fn mark_modified(&mut self) {
        if self.is_read_only() {
            return;
        }
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
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

    /// Convert a 0-based byte offset to (line, col), clamping to the last
    /// byte. A byte inside a multibyte char resolves to that char (Vim go).
    pub fn byte_to_line_col(&self, byte: usize) -> (usize, usize) {
        if self.text.len_bytes() == 0 {
            return (0, 0);
        }
        let byte = byte.min(self.text.len_bytes() - 1);
        let ch = self.text.byte_to_char(byte);
        let line = self.text.char_to_line(ch);
        (line, ch - self.text.line_to_char(line))
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
        if let BufferKind::Virtual { name, .. } = &self.kind {
            return name.clone();
        }

        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| "[No Name]".to_string())
    }

    /// Get text in a range as a string
    pub fn get_text_range(
        &self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) -> String {
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
        self.char_at(line, col)
            .map(|c| c.to_string())
            .unwrap_or_default()
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
        let (start_byte, start_point) = self.point_at_char(idx);
        let (mut old_end_byte, mut old_end_point) = (start_byte, start_point);

        // Delete old text if any
        if !old_text.is_empty() {
            let end_idx = idx + old_text.chars().count();
            if end_idx <= self.text.len_chars() {
                let old_end = self.point_at_char(end_idx);
                old_end_byte = old_end.0;
                old_end_point = old_end.1;
                self.text.remove(idx..end_idx);
            }
        }

        // Insert new text if any
        if !new_text.is_empty() {
            self.text.insert(idx, new_text);
        }

        let (new_end_byte, new_end_point) = self.point_at_char(idx + new_text.chars().count());
        self.push_edit(BufferEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_point,
            old_end_point,
            new_end_point,
        });
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

fn write_file_atomically(
    path: &Path,
    write_contents: impl FnOnce(&mut dyn Write) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid file path: {}", path.display()))?;
    let target_permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let (temp_path, temp_file) = create_save_temp_file(parent, file_name)?;
    let mut writer = io::BufWriter::new(temp_file);

    let write_result = (|| -> anyhow::Result<()> {
        write_contents(&mut writer)?;
        writer.flush()?;
        let temp_file = writer.get_ref();
        if let Some(permissions) = target_permissions {
            temp_file.set_permissions(permissions)?;
        }
        temp_file.sync_all()?;
        Ok(())
    })();

    drop(writer);

    if let Err(err) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(err.into());
    }

    sync_parent_dir(parent);
    Ok(())
}

fn create_save_temp_file(parent: &Path, file_name: &OsStr) -> io::Result<(PathBuf, File)> {
    for attempt in 0..100 {
        let temp_path = parent.join(save_temp_file_name(file_name, attempt));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create unique save temp file",
    ))
}

fn save_temp_file_name(file_name: &OsStr, attempt: u32) -> OsString {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut temp_name = OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(
        ".nevi-save-{}-{}-{}",
        std::process::id(),
        nanos,
        attempt
    ));
    temp_name
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) {
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) {}

#[cfg(test)]
mod tests {
    use super::{Buffer, write_file_atomically};
    use std::io;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos))
    }

    #[test]
    fn missing_final_newline_is_fixed_on_load_and_set_content() {
        let mut buffer = Buffer::new();
        buffer.set_content("abc");
        assert_eq!(buffer.content(), "abc\n");
        assert_eq!(buffer.addressable_line_count(), 1);

        buffer.set_content("abc\n");
        assert_eq!(buffer.content(), "abc\n", "already terminated: unchanged");

        buffer.set_content("");
        assert_eq!(buffer.content(), "", "empty buffer stays empty");

        let dir = unique_temp_dir("nevi_noeol");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("noeol.txt");
        std::fs::write(&path, "alpha\nbeta").expect("write file");
        let from_disk = Buffer::from_file(path).expect("load");
        assert_eq!(from_disk.content(), "alpha\nbeta\n");
        assert!(!from_disk.dirty, "normalization must not mark dirty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn byte_to_line_col_counts_bytes_and_clamps() {
        let mut buffer = Buffer::new();
        buffer.set_content("h\u{e9}llo\nworld\n");
        assert_eq!(buffer.byte_to_line_col(0), (0, 0));
        // Byte 2 is the second byte of the 2-byte 'é' → resolves to 'é'.
        assert_eq!(buffer.byte_to_line_col(2), (0, 1));
        // 'é' shifts bytes vs chars: byte 7 is 'w', char col 0 of line 1.
        assert_eq!(buffer.byte_to_line_col(7), (1, 0));
        // Past the end clamps to the last byte (the trailing newline).
        assert_eq!(buffer.byte_to_line_col(999), (1, 5));

        let empty = Buffer::new();
        assert_eq!(empty.byte_to_line_col(5), (0, 0));
    }

    #[test]
    fn addressable_line_count_ignores_only_the_trailing_rope_sentinel() {
        let cases = [
            ("", 1),
            ("alpha", 1),
            ("alpha\n", 1),
            ("alpha\nbeta\n", 2),
            ("alpha\nbeta\n\n", 3),
        ];

        for (content, expected) in cases {
            let mut buffer = Buffer::new();
            buffer.insert_str(0, 0, content);

            assert_eq!(
                buffer.addressable_line_count(),
                expected,
                "content={content:?}"
            );
        }
    }

    #[test]
    fn first_line_prefix_returns_short_first_line_including_newline() {
        let mut buffer = Buffer::new();
        buffer.insert_str(0, 0, "#!/usr/bin/env bash\nrest\n");
        assert_eq!(
            buffer.first_line_prefix().as_deref(),
            Some("#!/usr/bin/env bash\n")
        );
    }

    #[test]
    fn first_line_prefix_caps_at_256_chars_not_the_whole_line() {
        let mut buffer = Buffer::new();
        buffer.insert_str(0, 0, &"a".repeat(1000));
        let prefix = buffer.first_line_prefix().expect("first line");
        assert_eq!(prefix.chars().count(), super::FIRST_LINE_PREFIX_CHARS);
        assert_eq!(prefix, "a".repeat(super::FIRST_LINE_PREFIX_CHARS));
    }

    #[test]
    fn atomic_write_preserves_existing_file_when_writer_fails() {
        let tmp = unique_temp_dir("nevi_atomic_save");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("file.txt");
        std::fs::write(&path, "original").expect("write original");

        let result = write_file_atomically(&path, |writer| {
            writer.write_all(b"partial replacement")?;
            Err(io::Error::other("simulated write failure").into())
        });

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(&path).expect("read original"),
            "original"
        );
        assert_eq!(
            std::fs::read_dir(&tmp)
                .expect("read temp dir")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains(".nevi-save-"))
                .count(),
            0
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn edit_history_drains_only_for_the_matching_version() {
        let mut buffer = Buffer::new();
        buffer.insert_str(0, 0, "hello\n");
        let v1 = buffer.version();
        buffer.insert_char(0, 5, '!');

        // Wrong base version: no drain, full reparse required.
        assert!(buffer.take_edits_since(v1 + 10).is_none());

        // After a full parse resets history, draining from the reset point
        // yields exactly the edits since then.
        buffer.reset_edit_history();
        let base = buffer.version();
        buffer.insert_char(0, 0, 'x');
        buffer.delete_char(0, 0);
        let edits = buffer.take_edits_since(base).expect("matching base drains");
        assert_eq!(edits.len(), 2);

        // Drained: the next drain from the new version is empty but valid.
        let edits = buffer
            .take_edits_since(buffer.version())
            .expect("empty drain");
        assert!(edits.is_empty());
    }

    #[test]
    fn bulk_replace_breaks_edit_history_until_reset() {
        let mut buffer = Buffer::new();
        buffer.insert_str(0, 0, "one\n");
        buffer.reset_edit_history();
        let base = buffer.version();

        buffer.set_content("two\n");
        assert!(
            buffer.take_edits_since(base).is_none(),
            "set_content must force a full reparse"
        );
        assert!(buffer.take_edits_since(buffer.version()).is_none());

        buffer.reset_edit_history();
        buffer.insert_char(0, 0, 'a');
        // The old pre-replace base still cannot drain; only the reset
        // point can.
        assert!(buffer.take_edits_since(base).is_none());
        let reset_point = buffer.version().wrapping_sub(1);
        assert_eq!(
            buffer
                .take_edits_since(reset_point)
                .map(|edits| edits.len()),
            Some(1)
        );
    }

    #[test]
    fn recorded_edit_has_correct_bytes_for_multibyte_text() {
        let mut buffer = Buffer::new();
        buffer.insert_str(0, 0, "h\u{e9}llo\nworld\n");
        buffer.reset_edit_history();
        let base = buffer.version();

        // Insert after the 2-byte 'é': char col 2 is byte col 3.
        buffer.insert_char(0, 2, '\u{2713}');
        let edits = buffer.take_edits_since(base).expect("drain");
        assert_eq!(edits.len(), 1);
        let edit = edits[0];
        assert_eq!(edit.start_byte, 3);
        assert_eq!(edit.start_point, (0, 3));
        assert_eq!(edit.old_end_byte, 3);
        assert_eq!(edit.new_end_byte, 6, "check mark is 3 bytes");
        assert_eq!(edit.new_end_point, (0, 6));

        // Delete across the newline: old end lands on line 1.
        let base = buffer.version();
        buffer.delete_range(0, 5, 1, 2);
        let edits = buffer.take_edits_since(base).expect("drain");
        assert_eq!(edits[0].old_end_point, (1, 2));
        assert_eq!(edits[0].new_end_point, edits[0].start_point);
    }
}
