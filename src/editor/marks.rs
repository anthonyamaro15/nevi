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

    /// Delete all lowercase marks for a specific buffer
    /// Used by :delmarks! command
    pub fn delete_all_local(&mut self, buffer_key: &str) -> usize {
        if let Some(marks) = self.local.get_mut(buffer_key) {
            let count = marks.len();
            marks.clear();
            count
        } else {
            0
        }
    }

    /// Parse a delmarks argument string and return marks to delete
    /// Supports: "a", "a b c", "a-d", "aB", "a-dXY"
    pub fn parse_delmarks_arg(arg: &str) -> Vec<char> {
        let mut marks = Vec::new();
        let chars: Vec<char> = arg.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let c = chars[i];

            // Skip whitespace
            if c.is_whitespace() {
                i += 1;
                continue;
            }

            // Check for range (e.g., "a-d")
            if i + 2 < chars.len() && chars[i + 1] == '-' && c.is_ascii_alphabetic() {
                let start = c;
                let end = chars[i + 2];

                // Range must be same case and end >= start
                if end.is_ascii_alphabetic()
                    && ((start.is_lowercase() && end.is_lowercase())
                        || (start.is_uppercase() && end.is_uppercase()))
                    && end >= start
                {
                    for mark in start..=end {
                        if !marks.contains(&mark) {
                            marks.push(mark);
                        }
                    }
                    i += 3;
                    continue;
                }
            }

            // Single mark
            if c.is_ascii_alphabetic() && !marks.contains(&c) {
                marks.push(c);
            }

            i += 1;
        }

        marks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_delmarks_single() {
        assert_eq!(Marks::parse_delmarks_arg("a"), vec!['a']);
        assert_eq!(Marks::parse_delmarks_arg("A"), vec!['A']);
    }

    #[test]
    fn test_parse_delmarks_multiple() {
        assert_eq!(Marks::parse_delmarks_arg("a b c"), vec!['a', 'b', 'c']);
        assert_eq!(Marks::parse_delmarks_arg("aB"), vec!['a', 'B']);
        assert_eq!(Marks::parse_delmarks_arg("abc"), vec!['a', 'b', 'c']);
    }

    #[test]
    fn test_parse_delmarks_range() {
        assert_eq!(Marks::parse_delmarks_arg("a-d"), vec!['a', 'b', 'c', 'd']);
        assert_eq!(Marks::parse_delmarks_arg("A-C"), vec!['A', 'B', 'C']);
    }

    #[test]
    fn test_parse_delmarks_mixed() {
        assert_eq!(
            Marks::parse_delmarks_arg("a-c X Y"),
            vec!['a', 'b', 'c', 'X', 'Y']
        );
    }

    #[test]
    fn test_parse_delmarks_no_duplicates() {
        // Range followed by individual mark that's in range
        assert_eq!(Marks::parse_delmarks_arg("a-c b"), vec!['a', 'b', 'c']);
    }

    #[test]
    fn test_delete_marks() {
        let mut marks = Marks::new();
        marks.set_local("test", 'a', 10, 5);
        marks.set_local("test", 'b', 20, 0);

        assert!(marks.get_local("test", 'a').is_some());
        assert!(marks.delete("test", 'a'));
        assert!(marks.get_local("test", 'a').is_none());

        // Delete non-existent mark
        assert!(!marks.delete("test", 'z'));
    }

    #[test]
    fn test_delete_all_local() {
        let mut marks = Marks::new();
        marks.set_local("test", 'a', 10, 5);
        marks.set_local("test", 'b', 20, 0);
        marks.set_local("test", 'c', 30, 0);

        let count = marks.delete_all_local("test");
        assert_eq!(count, 3);
        assert!(marks.get_local("test", 'a').is_none());
        assert!(marks.get_local("test", 'b').is_none());
        assert!(marks.get_local("test", 'c').is_none());
    }
}
