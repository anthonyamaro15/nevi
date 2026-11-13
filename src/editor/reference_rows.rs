//! What the references picker shows for each hit: the file, the line, and
//! the line's code with the symbol marked, laid out like a live grep row.

use super::Editor;
use crate::lsp::Location;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Files not open in a buffer are read once per picker, up to this many,
/// and only when they're small; past that a row shows just its location.
const MAX_FILES_READ: usize = 50;
const MAX_FILE_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceRow {
    /// Relative to the working directory when it's inside it.
    pub path: String,
    /// 1-based.
    pub line: usize,
    /// The line without its indentation; empty when it couldn't be read.
    pub text: String,
    /// Char indices of the symbol in `text`.
    pub matched: Vec<usize>,
}

impl ReferenceRow {
    pub fn symbol(&self) -> String {
        self.matched
            .iter()
            .filter_map(|&idx| self.text.chars().nth(idx))
            .collect()
    }
}

impl Editor {
    pub(crate) fn reference_rows(&self, locations: &[Location]) -> Vec<ReferenceRow> {
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut files: HashMap<PathBuf, Option<Vec<String>>> = HashMap::new();
        locations
            .iter()
            .map(|loc| {
                let path = crate::lsp::uri_to_path(&loc.uri);
                let source = path
                    .as_deref()
                    .and_then(|path| self.source_line(&mut files, path, loc.line));
                let (text, matched) = match source {
                    Some(line) => {
                        let symbol = symbol_range(&line, loc.col);
                        crate::finder::grep::row_text(&line, &[symbol])
                    }
                    None => (String::new(), Vec::new()),
                };
                let path = match &path {
                    Some(path) => path
                        .strip_prefix(&cwd)
                        .unwrap_or(path)
                        .display()
                        .to_string(),
                    None => loc.uri.clone(),
                };
                ReferenceRow {
                    path,
                    line: loc.line + 1,
                    text,
                    matched,
                }
            })
            .collect()
    }

    /// An open buffer's line (it may have unsaved edits), else the file's.
    fn source_line(
        &self,
        files: &mut HashMap<PathBuf, Option<Vec<String>>>,
        path: &Path,
        line: usize,
    ) -> Option<String> {
        if let Some(buffer) = self
            .buffers
            .iter()
            .find(|b| b.path.as_deref() == Some(path))
        {
            let text = buffer.line(line)?.to_string();
            return Some(text.trim_end_matches(['\n', '\r']).to_string());
        }
        if !files.contains_key(path) && files.len() >= MAX_FILES_READ {
            return None;
        }
        files
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let small = std::fs::metadata(path).ok()?.len() <= MAX_FILE_BYTES;
                let text = std::fs::read_to_string(path).ok().filter(|_| small)?;
                Some(text.lines().map(str::to_string).collect())
            })
            .as_ref()?
            .get(line)
            .cloned()
    }
}

/// The identifier starting at LSP column `utf16_col`, as a char range of
/// `line`. A column on punctuation marks that one char.
fn symbol_range(line: &str, utf16_col: usize) -> (usize, usize) {
    let mut units = 0;
    let start = line
        .chars()
        .position(|ch| {
            let here = units >= utf16_col;
            units += ch.len_utf16();
            here
        })
        .unwrap_or(line.chars().count());
    let word = line
        .chars()
        .skip(start)
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .count();
    (start, start + word.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_range_counts_utf16_units_and_stops_at_the_word() {
        assert_eq!(symbol_range("\treturn total(o)", 8), (8, 13));
        // "é" is one UTF-16 unit, "😀" two.
        assert_eq!(symbol_range("é😀 total", 4), (3, 8));
        assert_eq!(symbol_range("a + b", 2), (2, 3));
        assert_eq!(symbol_range("ab", 9), (2, 3));
    }

    #[test]
    fn rows_read_open_buffers_first_and_unopened_files_from_disk() {
        let dir = std::env::temp_dir().join(format!("nevi-reference-rows-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let open = dir.join("open.go");
        let closed = dir.join("closed.go");
        std::fs::write(&open, "package main\n\nfunc total() {}\n").unwrap();
        std::fs::write(&closed, "package main\n\n\tx := total()\n").unwrap();

        let mut editor = Editor::default();
        editor.open_file(open.clone()).unwrap();
        // Unsaved edit: the row should show it, not the file on disk.
        editor.replace_buffer_content("package main\n\nfunc total(orders) {}\n");
        let location = |path: &Path, line, col| Location {
            uri: crate::lsp::path_to_uri(&path.to_path_buf()),
            line,
            col,
        };
        let rows = editor.reference_rows(&[
            location(&open, 2, 5),
            location(&closed, 2, 6),
            location(&dir.join("gone.go"), 0, 0),
        ]);

        assert_eq!(rows[0].text, "func total(orders) {}");
        assert_eq!(rows[0].line, 3);
        assert_eq!(rows[0].symbol(), "total");
        assert_eq!(rows[1].text, "x := total()");
        assert_eq!(rows[1].symbol(), "total");
        assert!(rows[1].path.ends_with("closed.go"));
        assert_eq!(rows[2].text, "");
        assert!(rows[2].matched.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
