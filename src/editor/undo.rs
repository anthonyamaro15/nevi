/// A single change that can be undone/redone
#[derive(Debug, Clone)]
pub struct Change {
    /// Starting position (line, col) of the change
    pub start_line: usize,
    pub start_col: usize,
    /// The text that was removed (empty for pure insertions)
    pub old_text: String,
    /// The text that was inserted (empty for pure deletions)
    pub new_text: String,
}

impl Change {
    pub fn new(start_line: usize, start_col: usize, old_text: String, new_text: String) -> Self {
        Self {
            start_line,
            start_col,
            old_text,
            new_text,
        }
    }

    /// Create a change for inserting text
    pub fn insert(line: usize, col: usize, text: String) -> Self {
        Self::new(line, col, String::new(), text)
    }

    /// Create a change for deleting text
    pub fn delete(line: usize, col: usize, text: String) -> Self {
        Self::new(line, col, text, String::new())
    }

    /// Create a change for replacing an entire line
    pub fn replace_line(line: usize, old_text: String, new_text: String) -> Self {
        Self::new(line, 0, old_text, new_text)
    }

    /// Create the inverse of this change (for undo)
    pub fn inverse(&self) -> Self {
        Self {
            start_line: self.start_line,
            start_col: self.start_col,
            old_text: self.new_text.clone(),
            new_text: self.old_text.clone(),
        }
    }
}

/// A group of changes that form a single undoable action
#[derive(Debug, Clone, Default)]
pub struct UndoEntry {
    /// The changes in this entry (in order they were made)
    pub changes: Vec<Change>,
    /// Cursor position before this entry
    pub cursor_before: (usize, usize),
    /// Cursor position after this entry
    pub cursor_after: (usize, usize),
}

impl UndoEntry {
    pub fn new(cursor_line: usize, cursor_col: usize) -> Self {
        Self {
            changes: Vec::new(),
            cursor_before: (cursor_line, cursor_col),
            cursor_after: (cursor_line, cursor_col),
        }
    }

    /// Add a change to this entry
    pub fn push(&mut self, change: Change) {
        self.changes.push(change);
    }

    /// Check if this entry has any changes
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Set the cursor position after all changes
    pub fn set_cursor_after(&mut self, line: usize, col: usize) {
        self.cursor_after = (line, col);
    }
}

/// Manages the undo/redo history
#[derive(Debug, Clone, Default)]
pub struct UndoStack {
    /// Stack of undoable entries
    undo_stack: Vec<UndoEntry>,
    /// Stack of redoable entries
    redo_stack: Vec<UndoEntry>,
    /// Current entry being built (during editing)
    current_entry: Option<UndoEntry>,
    /// Maximum number of undo entries to keep
    max_entries: usize,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            current_entry: None,
            max_entries: 1000,
        }
    }

    /// Start a new undo group (call before making changes)
    pub fn begin_undo_group(&mut self, cursor_line: usize, cursor_col: usize) {
        // Finalize any existing group first
        self.end_undo_group(cursor_line, cursor_col);
        self.current_entry = Some(UndoEntry::new(cursor_line, cursor_col));
    }

    /// End the current undo group (call after changes are done)
    pub fn end_undo_group(&mut self, cursor_line: usize, cursor_col: usize) {
        if let Some(mut entry) = self.current_entry.take() {
            if !entry.is_empty() {
                entry.set_cursor_after(cursor_line, cursor_col);
                self.undo_stack.push(entry);
                // Clear redo stack when new changes are made
                self.redo_stack.clear();
                // Trim if too many entries
                while self.undo_stack.len() > self.max_entries {
                    self.undo_stack.remove(0);
                }
            }
        }
    }

    /// Record a change in the current undo group
    pub fn record_change(&mut self, change: Change) {
        if let Some(ref mut entry) = self.current_entry {
            entry.push(change);
        } else {
            // No group started, create a single-change entry
            let mut entry = UndoEntry::new(0, 0);
            entry.push(change);
            self.undo_stack.push(entry);
            self.redo_stack.clear();
        }
    }

    /// Pop an entry from the undo stack
    pub fn pop_undo(&mut self) -> Option<UndoEntry> {
        // First finalize any current entry
        if let Some(entry) = self.current_entry.take() {
            if !entry.is_empty() {
                self.undo_stack.push(entry);
            }
        }

        if let Some(entry) = self.undo_stack.pop() {
            // Move to redo stack
            self.redo_stack.push(entry.clone());
            Some(entry)
        } else {
            None
        }
    }

    /// Pop an entry from the redo stack
    pub fn pop_redo(&mut self) -> Option<UndoEntry> {
        if let Some(entry) = self.redo_stack.pop() {
            // Move back to undo stack
            self.undo_stack.push(entry.clone());
            Some(entry)
        } else {
            None
        }
    }

    /// Check if undo is available
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty() || self.current_entry.as_ref().map_or(false, |e| !e.is_empty())
    }

    /// Check if redo is available
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Get the number of undo entries
    pub fn undo_count(&self) -> usize {
        self.undo_stack.len() + if self.current_entry.as_ref().map_or(false, |e| !e.is_empty()) { 1 } else { 0 }
    }

    /// Get the number of redo entries
    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }

    /// Clear all history
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.current_entry = None;
    }
}
