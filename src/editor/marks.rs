use std::collections::HashMap;
use std::path::PathBuf;

/// A mark position in a buffer
#[derive(Debug, Clone)]
pub struct Mark {
    /// File path (for global marks A-Z)
    pub path: Option<PathBuf>,
    /// Line number (0-indexed)
    pub line: usize,
    /// Column number (0-indexed)
    pub col: usize,
}

impl Mark {
    pub fn new(line: usize, col: usize) -> Self {
        Self {
            path: None,
            line,
            col,
        }
    }

    pub fn with_path(path: PathBuf, line: usize, col: usize) -> Self {
        Self {
            path: Some(path),
            line,
            col,
        }
    }
}

/// Manages marks for the editor
/// Local marks (a-z) are per-buffer
/// Global marks (A-Z) are shared across all buffers
#[derive(Debug, Clone, Default)]
pub struct Marks {
    /// Local marks per buffer (keyed by buffer path or index)
    /// HashMap<buffer_key, HashMap<mark_char, Mark>>
    local: HashMap<String, HashMap<char, Mark>>,
    /// Global marks (A-Z) - shared across all buffers
    global: HashMap<char, Mark>,
}

impl Marks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a local mark (a-z) for a specific buffer
    pub fn set_local(&mut self, buffer_key: &str, name: char, line: usize, col: usize) {
        let buffer_marks = self.local.entry(buffer_key.to_string()).or_default();
        buffer_marks.insert(name, Mark::new(line, col));
    }

    /// Set a global mark (A-Z)
    pub fn set_global(&mut self, name: char, path: PathBuf, line: usize, col: usize) {
        self.global.insert(name, Mark::with_path(path, line, col));
    }

    /// Get a local mark for a specific buffer
    pub fn get_local(&self, buffer_key: &str, name: char) -> Option<&Mark> {
        self.local.get(buffer_key).and_then(|marks| marks.get(&name))
    }

    /// Get a global mark
    pub fn get_global(&self, name: char) -> Option<&Mark> {
        self.global.get(&name)
    }

    /// Get a mark by name (checks if local or global based on case)
    pub fn get(&self, buffer_key: &str, name: char) -> Option<&Mark> {
        if name.is_lowercase() {
            self.get_local(buffer_key, name)
        } else {
            self.get_global(name)
        }
    }

    /// Set a mark by name (determines local vs global based on case)
    pub fn set(&mut self, buffer_key: &str, path: Option<PathBuf>, name: char, line: usize, col: usize) {
        if name.is_lowercase() {
            self.set_local(buffer_key, name, line, col);
        } else if let Some(p) = path {
            self.set_global(name, p, line, col);
        }
    }

    /// Check if a character is a valid mark name
    pub fn is_valid_mark(c: char) -> bool {
        c.is_ascii_alphabetic()
    }

    /// Get all local marks for a specific buffer (sorted by name)
    pub fn get_local_marks(&self, buffer_key: &str) -> Vec<(char, &Mark)> {
        let mut marks: Vec<(char, &Mark)> = self
            .local
            .get(buffer_key)
            .map(|m| m.iter().map(|(c, mark)| (*c, mark)).collect())
            .unwrap_or_default();
        marks.sort_by_key(|(c, _)| *c);
        marks
    }

    /// Get all global marks (sorted by name)
    pub fn get_global_marks(&self) -> Vec<(char, &Mark)> {
        let mut marks: Vec<(char, &Mark)> = self.global.iter().map(|(c, mark)| (*c, mark)).collect();
        marks.sort_by_key(|(c, _)| *c);
        marks
    }

    /// Delete a mark by name
    pub fn delete(&mut self, buffer_key: &str, name: char) -> bool {
        if name.is_lowercase() {
            self.local
                .get_mut(buffer_key)
                .map(|marks| marks.remove(&name).is_some())
                .unwrap_or(false)
        } else {
            self.global.remove(&name).is_some()
        }
    }
}
