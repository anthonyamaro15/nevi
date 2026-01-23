mod file_picker;
mod grep;
mod matcher;

pub use file_picker::FilePicker;
pub use grep::GrepSearcher;
pub use matcher::FuzzyMatcher;

use std::path::PathBuf;
use std::fs::File;
use std::io::{BufRead, BufReader};

/// Mode for the fuzzy finder
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderMode {
    Files,
    Grep,
    Buffers,
    Diagnostics,
}

/// Input mode for the fuzzy finder (like vim modes)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FinderInputMode {
    /// Insert mode - typing adds to query
    #[default]
    Insert,
    /// Normal mode - j/k navigate, typing switches to insert
    Normal,
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
    /// Buffer index (for buffer picker results)
    pub buffer_idx: Option<usize>,
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
            buffer_idx: None,
            score: 0,
            match_indices: Vec::new(),
        }
    }

    pub fn with_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }

    pub fn with_buffer_idx(mut self, idx: usize) -> Self {
        self.buffer_idx = Some(idx);
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
        Self::centered_with_preview(term_width, term_height, false)
    }

    /// Calculate centered position for a floating window with optional preview panel
    /// Window size is always the same - only internal layout changes with preview toggle
    pub fn centered_with_preview(term_width: u16, term_height: u16, _preview_enabled: bool) -> Self {
        // Window is always 90% width (same size whether preview is on or off)
        let width = (term_width * 90 / 100).min(200).max(80);
        let height = (term_height * 70 / 100).min(40).max(10);
        let x = (term_width.saturating_sub(width)) / 2;
        let y = (term_height.saturating_sub(height)) / 2;
        Self { x, y, width, height }
    }
}

/// The main fuzzy finder state
pub struct FuzzyFinder {
    /// Current mode (Files/Grep/Buffers)
    pub mode: FinderMode,
    /// Input mode (Insert/Normal for vim-like navigation)
    pub input_mode: FinderInputMode,
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
    /// Preview panel enabled
    pub preview_enabled: bool,
    /// Cached preview content (lines)
    pub preview_content: Vec<String>,
    /// Preview scroll offset
    pub preview_scroll: usize,
    /// Path of currently previewed file
    pub preview_path: Option<PathBuf>,
    /// Pending preview update (debounce) - stores the time when update was requested
    pub preview_update_pending: bool,
    /// Pending grep search (debounce) - set when query changes in grep mode
    pub grep_search_pending: bool,
}

impl FuzzyFinder {
    pub fn new() -> Self {
        Self {
            mode: FinderMode::Files,
            input_mode: FinderInputMode::Insert,
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
            preview_enabled: false,
            preview_content: Vec::new(),
            preview_scroll: 0,
            preview_path: None,
            preview_update_pending: false,
            grep_search_pending: false,
        }
    }

    /// Create from config settings
    pub fn from_settings(settings: &crate::config::FinderSettings) -> Self {
        Self {
            mode: FinderMode::Files,
            input_mode: FinderInputMode::Insert,
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
            preview_enabled: false,
            preview_content: Vec::new(),
            preview_scroll: 0,
            preview_path: None,
            preview_update_pending: false,
            grep_search_pending: false,
        }
    }

    /// Open the finder in file mode
    pub fn open_files(&mut self, cwd: &std::path::Path) {
        self.mode = FinderMode::Files;
        self.input_mode = FinderInputMode::Insert;
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
        self.input_mode = FinderInputMode::Insert;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;

        // Populate buffers
        self.items = buffer_names
            .into_iter()
            .map(|(idx, name, path)| {
                let mut item = FinderItem::new(format!("{}: {}", idx + 1, name), path)
                    .with_buffer_idx(idx);
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
        self.input_mode = FinderInputMode::Insert;
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

    /// Open the finder in diagnostics mode
    /// Takes diagnostic items pre-formatted by the editor
    pub fn open_diagnostics(&mut self, diagnostic_items: Vec<FinderItem>) {
        self.mode = FinderMode::Diagnostics;
        self.input_mode = FinderInputMode::Insert;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;

        self.items = diagnostic_items;
        self.filtered = (0..self.items.len()).collect();
        self.populated = true;
    }

    /// Enter normal mode (for j/k navigation)
    pub fn enter_normal_mode(&mut self) {
        self.input_mode = FinderInputMode::Normal;
    }

    /// Enter insert mode (for typing)
    pub fn enter_insert_mode(&mut self) {
        self.input_mode = FinderInputMode::Insert;
    }

    /// Check if in normal mode
    pub fn is_normal_mode(&self) -> bool {
        self.input_mode == FinderInputMode::Normal
    }

    /// Convert char index to byte index for string operations
    fn char_to_byte_index(&self, char_idx: usize) -> usize {
        self.query
            .char_indices()
            .nth(char_idx)
            .map(|(byte_idx, _)| byte_idx)
            .unwrap_or(self.query.len())
    }

    /// Get the number of characters in the query
    fn char_count(&self) -> usize {
        self.query.chars().count()
    }

    /// Get icon for a file based on extension
    /// Uses 2-character type indicators for consistent terminal width
    pub fn get_file_icon(path: &std::path::Path) -> &'static str {
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // Check for special filenames first
        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        match filename.to_lowercase().as_str() {
            ".gitignore" | ".gitattributes" => "GT",
            ".env" | ".env.local" | ".env.development" | ".env.production" => "EN",
            ".prettierrc" | ".prettierrc.json" => "PR",
            ".eslintrc" | ".eslintrc.json" | ".eslintrc.js" => "ES",
            "dockerfile" => "DK",
            "makefile" => "MK",
            "cargo.toml" => "RS",
            "package.json" => "PK",
            "tsconfig.json" => "TS",
            _ => {
                // Fall back to extension-based icons
                match ext.to_lowercase().as_str() {
                    // Programming languages
                    "rs" => "RS",
                    "py" => "PY",
                    "js" | "mjs" | "cjs" => "JS",
                    "ts" | "mts" | "cts" => "TS",
                    "tsx" => "TX",
                    "jsx" => "JX",
                    "go" => "GO",
                    "rb" => "RB",
                    "java" => "JV",
                    "c" => "C ",
                    "h" => "H ",
                    "cpp" | "cc" | "cxx" => "C+",
                    "hpp" => "H+",
                    "cs" => "C#",
                    "php" => "HP",
                    "swift" => "SW",
                    "kt" | "kts" => "KT",
                    "lua" => "LU",
                    // Web
                    "html" | "htm" => "HT",
                    "css" => "CS",
                    "scss" | "sass" => "SC",
                    "vue" => "VU",
                    "svelte" => "SV",
                    // Data/Config
                    "json" | "jsonc" => "JS",
                    "xml" => "XM",
                    "yaml" | "yml" => "YM",
                    "toml" => "TM",
                    "ini" | "cfg" | "conf" => "CF",
                    "env" => "EN",
                    // Documents
                    "md" | "markdown" => "MD",
                    "txt" => "TX",
                    "pdf" => "PD",
                    "doc" | "docx" => "DC",
                    // Images
                    "png" => "PN",
                    "jpg" | "jpeg" => "JP",
                    "gif" => "GF",
                    "svg" => "SV",
                    "webp" => "WP",
                    "ico" => "IC",
                    // Shell
                    "sh" | "bash" => "SH",
                    "zsh" => "ZS",
                    "fish" => "FS",
                    // Lock files
                    "lock" => "LK",
                    // Default
                    _ => "  ",
                }
            }
        }
    }

    /// Insert a character at the cursor position
    pub fn insert_char(&mut self, ch: char) {
        // Typing always switches to insert mode
        self.input_mode = FinderInputMode::Insert;
        let byte_idx = self.char_to_byte_index(self.cursor);
        self.query.insert(byte_idx, ch);
        self.cursor += 1;
        self.update_filter();
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char_before(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte_idx = self.char_to_byte_index(self.cursor);
            self.query.remove(byte_idx);
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
        if self.cursor < self.char_count() {
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
                // In grep mode, defer search to debounce mechanism
                // This avoids running expensive grep on every keystroke
                if self.query.len() >= 2 {
                    self.grep_search_pending = true;
                } else {
                    self.items.clear();
                    self.filtered.clear();
                    self.grep_search_pending = false;
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

    /// Execute the pending grep search (called after debounce from main loop)
    pub fn execute_grep_search(&mut self) {
        if self.mode != FinderMode::Grep || !self.grep_search_pending {
            return;
        }

        self.grep_search_pending = false;

        if self.query.len() >= 2 {
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

            // Reset selection after new results
            self.selected = 0;
            self.scroll_offset = 0;
        }
    }

    /// Toggle preview panel on/off
    pub fn toggle_preview(&mut self) {
        self.preview_enabled = !self.preview_enabled;
        if self.preview_enabled {
            // Clear cache to force reload when re-enabled
            self.preview_path = None;
            self.preview_content.clear();
            self.preview_scroll = 0;
        }
    }

    /// Update preview content if the selected file changed
    /// Returns the current preview path and content for rendering
    pub fn update_preview_content(&mut self) -> Option<(PathBuf, &[String])> {
        if !self.preview_enabled {
            return None;
        }

        // Only show preview for Files and Grep modes
        if self.mode != FinderMode::Files && self.mode != FinderMode::Grep {
            return None;
        }

        let selected_item = self.selected_item()?;
        let selected_path = selected_item.path.clone();
        let selected_line = selected_item.line;

        // Check if we need to load new content
        if self.preview_path.as_ref() != Some(&selected_path) {
            self.preview_content.clear();
            self.preview_scroll = 0;
            self.preview_path = Some(selected_path.clone());

            // Check if file is likely binary
            if is_likely_binary(&selected_path) {
                self.preview_content = vec!["(Binary file - no preview)".to_string()];
                return Some((selected_path, &self.preview_content));
            }

            // Check if it's a directory
            if selected_path.is_dir() {
                self.preview_content = vec!["(Directory)".to_string()];
                return Some((selected_path, &self.preview_content));
            }

            // Read only the lines we need using buffered reader
            // This avoids reading entire large files into memory
            const MAX_PREVIEW_LINES: usize = 150;
            match File::open(&selected_path) {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    let mut line_count = 0;
                    for line in reader.lines().take(MAX_PREVIEW_LINES + 1) {
                        match line {
                            Ok(l) => {
                                if line_count < MAX_PREVIEW_LINES {
                                    self.preview_content.push(l);
                                }
                                line_count += 1;
                            }
                            Err(_) => {
                                // Binary file or encoding issue - stop reading
                                if self.preview_content.is_empty() {
                                    self.preview_content = vec!["(Unable to read file)".to_string()];
                                }
                                break;
                            }
                        }
                    }
                    if line_count > MAX_PREVIEW_LINES {
                        self.preview_content.push("... (truncated)".to_string());
                    }
                }
                Err(_) => {
                    self.preview_content = vec!["(Unable to read file)".to_string()];
                }
            }

            // For grep results, scroll preview to show the matching line
            if self.mode == FinderMode::Grep {
                if let Some(line_num) = selected_line {
                    // Center the matching line in the preview
                    let preview_height = 20; // Approximate, will be adjusted by render
                    self.preview_scroll = line_num.saturating_sub(preview_height / 2);
                }
            }
        }

        Some((self.preview_path.clone()?, &self.preview_content))
    }

    /// Scroll preview down
    pub fn scroll_preview_down(&mut self, amount: usize) {
        if !self.preview_content.is_empty() {
            self.preview_scroll = self.preview_scroll.saturating_add(amount)
                .min(self.preview_content.len().saturating_sub(1));
        }
    }

    /// Scroll preview up
    pub fn scroll_preview_up(&mut self, amount: usize) {
        self.preview_scroll = self.preview_scroll.saturating_sub(amount);
    }

    /// Reset preview scroll to top
    pub fn reset_preview_scroll(&mut self) {
        self.preview_scroll = 0;
    }
}

/// Check if a file is likely binary based on extension
fn is_likely_binary(path: &PathBuf) -> bool {
    let binary_exts = [
        "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "svg",
        "mp3", "mp4", "wav", "avi", "mov", "mkv", "flv",
        "zip", "tar", "gz", "bz2", "xz", "7z", "rar",
        "exe", "dll", "so", "dylib", "bin",
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
        "wasm", "o", "a", "class", "pyc",
        "ttf", "otf", "woff", "woff2", "eot",
        "db", "sqlite", "sqlite3",
    ];

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return binary_exts.contains(&ext.to_lowercase().as_str());
    }

    false
}

impl Default for FuzzyFinder {
    fn default() -> Self {
        Self::new()
    }
}
