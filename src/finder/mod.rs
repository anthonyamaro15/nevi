mod file_picker;
mod grep;
mod matcher;

pub use file_picker::FilePicker;
pub use grep::GrepSearcher;
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
    /// Indices of matched characters (for highlighting)
    pub match_indices: Vec<usize>,
}

impl FinderItem {
    pub fn new(display: String, path: PathBuf) -> Self {
        Self {
            display,
            path,
            line: None,
            score: 0,
            match_indices: Vec::new(),
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
    /// Grep searcher for live grep
    grep_searcher: GrepSearcher,
    /// Current working directory (for grep)
    cwd: PathBuf,
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
            grep_searcher: GrepSearcher::new(),
            cwd: PathBuf::new(),
            populated: false,
        }
    }

    /// Create from config settings
    pub fn from_settings(settings: &crate::config::FinderSettings) -> Self {
        Self {
            mode: FinderMode::Files,
            query: String::new(),
            cursor: 0,
            items: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            matcher: FuzzyMatcher::new(),
            file_picker: FilePicker::from_settings(settings),
            grep_searcher: GrepSearcher::from_settings(settings),
            cwd: PathBuf::new(),
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

    /// Open the finder in grep mode (live search)
    pub fn open_grep(&mut self, cwd: &std::path::Path) {
        self.mode = FinderMode::Grep;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;
        self.cwd = cwd.to_path_buf();

        // Start with empty results - will populate as user types
        self.items.clear();
        self.filtered.clear();
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

    /// Select next item (with scroll adjustment)
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

    /// Adjust scroll offset for given visible height
    pub fn adjust_scroll(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }

        // Ensure selected item is visible
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected - visible_height + 1;
        }
    }

    /// Get visible item range (start_idx, end_idx) for rendering
    pub fn visible_range(&self, visible_height: usize) -> (usize, usize) {
        let start = self.scroll_offset;
        let end = (self.scroll_offset + visible_height).min(self.filtered.len());
        (start, end)
    }

    /// Get the currently selected item
    pub fn selected_item(&self) -> Option<&FinderItem> {
        self.filtered.get(self.selected).and_then(|&idx| self.items.get(idx))
    }

    /// Update the filtered list based on the current query
    fn update_filter(&mut self) {
        // Clear previous match indices
        for item in &mut self.items {
            item.match_indices.clear();
        }

        match self.mode {
            FinderMode::Grep => {
                // In grep mode, search file contents
                if self.query.len() >= 2 {
                    // Only search after 2+ chars to avoid too many results
                    self.items = self.grep_searcher.search(&self.cwd, &self.query);
                    self.filtered = (0..self.items.len()).collect();

                    // For grep, highlight the search query in results
                    let query_lower = self.query.to_lowercase();
                    for item in &mut self.items {
                        let display_lower = item.display.to_lowercase();
                        if let Some(pos) = display_lower.find(&query_lower) {
                            item.match_indices = (pos..pos + self.query.len()).collect();
                        }
                    }
                } else {
                    self.items.clear();
                    self.filtered.clear();
                }
            }
            _ => {
                // For files/buffers, use fuzzy matching
                if self.query.is_empty() {
                    // No filter, show all items
                    self.filtered = (0..self.items.len()).collect();
                } else {
                    // Filter and sort by match score, and get match indices
                    let mut scored: Vec<(usize, u32, Vec<usize>)> = self.items
                        .iter()
                        .enumerate()
                        .filter_map(|(idx, item)| {
                            self.matcher.match_score(&self.query, &item.display).map(|score| {
                                let indices = self.matcher.match_indices(&self.query, &item.display);
                                (idx, score, indices)
                            })
                        })
                        .collect();

                    // Sort by score (higher is better)
                    scored.sort_by(|a, b| b.1.cmp(&a.1));

                    // Store match indices in items
                    for (idx, _, indices) in &scored {
                        self.items[*idx].match_indices = indices.clone();
                    }

                    self.filtered = scored.into_iter().map(|(idx, _, _)| idx).collect();
                }
            }
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
