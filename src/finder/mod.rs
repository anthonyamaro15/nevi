mod file_picker;
mod matcher;

pub use file_picker::FilePicker;
pub use matcher::FuzzyMatcher;

use std::path::PathBuf;

/// Mode for the fuzzy finder
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderMode {
    Files,
    Grep,
    Buffers,
}

/// An item in the finder list
#[derive(Debug, Clone)]
pub struct FinderItem {
    /// Display text
    pub display: String,
    /// Associated file path
    pub path: PathBuf,
    /// Line number (for grep results)
    pub line: Option<usize>,
    /// Match score for sorting
    pub score: u32,
}

impl FinderItem {
    pub fn new(display: String, path: PathBuf) -> Self {
        Self {
            display,
            path,
            line: None,
            score: 0,
        }
    }

    pub fn with_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }

    pub fn with_score(mut self, score: u32) -> Self {
        self.score = score;
        self
    }
}

/// Floating window dimensions
#[derive(Debug, Clone, Copy)]
pub struct FloatingWindow {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl FloatingWindow {
    /// Calculate centered position for a floating window
    pub fn centered(term_width: u16, term_height: u16) -> Self {
        let width = (term_width * 80 / 100).min(120).max(40);  // 80% width, max 120, min 40
        let height = (term_height * 70 / 100).min(40).max(10); // 70% height, max 40, min 10
        let x = (term_width.saturating_sub(width)) / 2;
        let y = (term_height.saturating_sub(height)) / 2;
        Self { x, y, width, height }
    }
}

/// The main fuzzy finder state
pub struct FuzzyFinder {
    /// Current mode
    pub mode: FinderMode,
    /// Query string
    pub query: String,
    /// Cursor position in query
    pub cursor: usize,
    /// All items (unfiltered)
    pub items: Vec<FinderItem>,
    /// Filtered and sorted item indices
    pub filtered: Vec<usize>,
    /// Currently selected index (in filtered list)
    pub selected: usize,
    /// Scroll offset for long lists
    pub scroll_offset: usize,
    /// Fuzzy matcher
    matcher: FuzzyMatcher,
    /// File picker for directory traversal
    file_picker: FilePicker,
    /// Whether the finder has been populated
    pub populated: bool,
}

impl FuzzyFinder {
    pub fn new() -> Self {
        Self {
            mode: FinderMode::Files,
            query: String::new(),
            cursor: 0,
            items: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            matcher: FuzzyMatcher::new(),
            file_picker: FilePicker::new(),
            populated: false,
        }
    }

    /// Open the finder in file mode
    pub fn open_files(&mut self, cwd: &std::path::Path) {
        self.mode = FinderMode::Files;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;

        // Populate files
        self.items = self.file_picker.list_files(cwd);
        self.filtered = (0..self.items.len()).collect();
        self.populated = true;
    }

    /// Open the finder in buffer mode
    pub fn open_buffers(&mut self, buffer_names: Vec<(usize, String, PathBuf)>) {
        self.mode = FinderMode::Buffers;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;

        // Populate buffers
        self.items = buffer_names
            .into_iter()
            .map(|(idx, name, path)| {
                let mut item = FinderItem::new(format!("{}: {}", idx + 1, name), path);
                item.score = idx as u32;
                item
            })
            .collect();
        self.filtered = (0..self.items.len()).collect();
        self.populated = true;
    }

    /// Insert a character at the cursor position
    pub fn insert_char(&mut self, ch: char) {
        self.query.insert(self.cursor, ch);
        self.cursor += 1;
        self.update_filter();
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char_before(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.query.remove(self.cursor);
            self.update_filter();
        }
    }

    /// Move cursor left
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn move_right(&mut self) {
        if self.cursor < self.query.len() {
            self.cursor += 1;
        }
    }

    /// Select next item
    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() && self.selected < self.filtered.len() - 1 {
            self.selected += 1;
        }
    }

    /// Select previous item
    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Get the currently selected item
    pub fn selected_item(&self) -> Option<&FinderItem> {
        self.filtered.get(self.selected).and_then(|&idx| self.items.get(idx))
    }

    /// Update the filtered list based on the current query
    fn update_filter(&mut self) {
        if self.query.is_empty() {
            // No filter, show all items
            self.filtered = (0..self.items.len()).collect();
        } else {
            // Filter and sort by match score
            let mut scored: Vec<(usize, u32)> = self.items
                .iter()
                .enumerate()
                .filter_map(|(idx, item)| {
                    self.matcher.match_score(&self.query, &item.display).map(|score| (idx, score))
                })
                .collect();

            // Sort by score (higher is better)
            scored.sort_by(|a, b| b.1.cmp(&a.1));

            self.filtered = scored.into_iter().map(|(idx, _)| idx).collect();
        }

        // Reset selection
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Get display text for the current filter state
    pub fn status_text(&self) -> String {
        format!("{}/{}", self.filtered.len(), self.items.len())
    }
}

impl Default for FuzzyFinder {
    fn default() -> Self {
        Self::new()
    }
}
