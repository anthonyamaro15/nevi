mod file_picker;
mod grep;
mod matcher;

pub use file_picker::FilePicker;
pub use grep::GrepSearcher;
pub use matcher::FuzzyMatcher;

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

enum GrepSearchMessage {
    Batch {
        generation: u64,
        query: String,
        items: Vec<FinderItem>,
    },
    Finished {
        generation: u64,
        query: String,
    },
}

/// Mode for the fuzzy finder
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderMode {
    Files,
    Grep,
    Buffers,
    Diagnostics,
    Harpoon,
    Marks,
    Terminals,
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
    /// Column number (0-indexed, for grep results)
    pub col: Option<usize>,
    /// Buffer index (for buffer picker results)
    pub buffer_idx: Option<usize>,
    /// Terminal session position (1-indexed, for terminal picker results)
    pub terminal_session_position: Option<usize>,
    /// Optional 2-character icon override
    pub icon: Option<&'static str>,
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
            col: None,
            buffer_idx: None,
            terminal_session_position: None,
            icon: None,
            score: 0,
            match_indices: Vec::new(),
        }
    }

    pub fn with_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }

    pub fn with_col(mut self, col: usize) -> Self {
        self.col = Some(col);
        self
    }

    pub fn with_buffer_idx(mut self, idx: usize) -> Self {
        self.buffer_idx = Some(idx);
        self
    }

    pub fn with_terminal_session_position(mut self, position: usize) -> Self {
        self.terminal_session_position = Some(position);
        self
    }

    pub fn with_icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_score(mut self, score: u32) -> Self {
        self.score = score;
        self
    }
}

/// Mark info for the marks finder
#[derive(Debug, Clone)]
pub struct MarkInfo {
    /// The mark character (a-z for local, A-Z for global)
    pub name: char,
    /// Line number (0-indexed)
    pub line: usize,
    /// Column number
    pub col: usize,
    /// File path (for global marks)
    pub file_path: Option<PathBuf>,
    /// Display file name
    pub file_name: String,
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
    pub fn centered_with_preview(
        term_width: u16,
        term_height: u16,
        _preview_enabled: bool,
    ) -> Self {
        // Window is always 90% width (same size whether preview is on or off)
        let width = (term_width * 90 / 100).min(200).max(80);
        let height = (term_height * 70 / 100).min(40).max(10);
        let x = (term_width.saturating_sub(width)) / 2;
        let y = (term_height.saturating_sub(height)) / 2;
        Self {
            x,
            y,
            width,
            height,
        }
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
    /// 0-indexed source line represented by preview_content[0]
    pub preview_line_offset: usize,
    /// Path of currently previewed file
    pub preview_path: Option<PathBuf>,
    /// Line number currently targeted by the preview, if any
    pub preview_line: Option<usize>,
    /// Pending preview update (debounce) - stores the time when update was requested
    pub preview_update_pending: bool,
    /// Pending grep search (debounce) - set when query changes in grep mode
    pub grep_search_pending: bool,
    /// Whether an async grep search is currently running
    pub grep_search_running: bool,
    /// Monotonic generation used to discard stale async grep results
    grep_search_generation: u64,
    /// Receiver for the currently running async grep search
    grep_search_receiver: Option<Receiver<GrepSearchMessage>>,
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
            preview_line_offset: 0,
            preview_path: None,
            preview_line: None,
            preview_update_pending: false,
            grep_search_pending: false,
            grep_search_running: false,
            grep_search_generation: 0,
            grep_search_receiver: None,
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
            preview_line_offset: 0,
            preview_path: None,
            preview_line: None,
            preview_update_pending: false,
            grep_search_pending: false,
            grep_search_running: false,
            grep_search_generation: 0,
            grep_search_receiver: None,
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
        self.clear_preview_cache();
        self.cancel_grep_search();

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
        self.clear_preview_cache();
        self.cancel_grep_search();

        // Populate buffers
        self.items = buffer_names
            .into_iter()
            .map(|(idx, name, path)| {
                let mut item =
                    FinderItem::new(format!("{}: {}", idx + 1, name), path).with_buffer_idx(idx);
                item.score = idx as u32;
                item
            })
            .collect();
        self.filtered = (0..self.items.len()).collect();
        self.populated = true;
    }

    /// Open the finder in harpoon mode
    pub fn open_harpoon(&mut self, files: Vec<PathBuf>) {
        self.mode = FinderMode::Harpoon;
        self.input_mode = FinderInputMode::Normal; // Start in normal mode for quick navigation
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;
        self.clear_preview_cache();
        self.cancel_grep_search();

        // Populate harpoon files with slot numbers
        self.items = files
            .into_iter()
            .enumerate()
            .map(|(idx, path)| {
                let display = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string());
                let mut item = FinderItem::new(format!("{:>2}  {}", idx + 1, display), path);
                item.score = idx as u32;
                item
            })
            .collect();
        self.filtered = (0..self.items.len()).collect();
        self.populated = true;
    }

    /// Open the finder in marks mode
    pub fn open_marks(&mut self, marks: Vec<MarkInfo>) {
        self.mode = FinderMode::Marks;
        self.input_mode = FinderInputMode::Normal; // Start in normal mode for quick navigation
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;
        self.clear_preview_cache();
        self.cancel_grep_search();

        // Populate marks
        self.items = marks
            .into_iter()
            .map(|mark| {
                let display = format!(
                    " {}   {:>4}:{:<3}  {}",
                    mark.name,
                    mark.line + 1,
                    mark.col,
                    mark.file_name
                );
                let mut item = FinderItem::new(display, mark.file_path.unwrap_or_default());
                item.line = Some(mark.line + 1);
                item
            })
            .collect();
        self.filtered = (0..self.items.len()).collect();
        self.populated = true;
    }

    /// Open the finder in terminal session mode
    pub fn open_terminals(&mut self, terminal_items: Vec<FinderItem>) {
        self.mode = FinderMode::Terminals;
        self.input_mode = FinderInputMode::Normal;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;
        self.clear_preview_cache();
        self.cancel_grep_search();

        self.items = terminal_items;
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
        self.clear_preview_cache();
        self.cwd = cwd.to_path_buf();
        self.cancel_grep_search();

        // Start with empty results - will populate as user types
        self.items.clear();
        self.filtered.clear();
        self.populated = true;
    }

    /// Open the finder in grep mode with a pre-filled query (e.g., word under cursor)
    pub fn open_grep_with_query(&mut self, cwd: &std::path::Path, query: &str) {
        self.mode = FinderMode::Grep;
        self.input_mode = FinderInputMode::Insert;
        self.query = query.to_string();
        self.cursor = query.chars().count();
        self.selected = 0;
        self.scroll_offset = 0;
        self.clear_preview_cache();
        self.cwd = cwd.to_path_buf();
        self.cancel_grep_search();

        if query.len() >= 2 {
            self.grep_search_pending = true;
            self.execute_grep_search();
        } else {
            self.items.clear();
            self.filtered.clear();
        }
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
        self.clear_preview_cache();
        self.cancel_grep_search();

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
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        // Check for special filenames first
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

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
        self.filtered
            .get(self.selected)
            .and_then(|&idx| self.items.get(idx))
    }

    pub fn cancel_grep_search(&mut self) {
        self.grep_search_pending = false;
        self.grep_search_running = false;
        self.grep_search_receiver = None;
        self.grep_search_generation = self.grep_search_generation.wrapping_add(1);
    }

    fn update_grep_match_indices(&mut self) {
        for item in &mut self.items {
            item.match_indices.clear();
            item.match_indices = grep_result_match_indices(
                &item.display,
                &self.query,
                grep_result_snippet_start(&item.display),
            );
        }
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
                    self.cancel_grep_search();
                    self.clear_preview_cache();
                }
            }
            _ => {
                // For files/buffers, use fuzzy matching
                if self.query.is_empty() {
                    // No filter, show all items
                    self.filtered = (0..self.items.len()).collect();
                } else {
                    // Filter and sort by match score, and get match indices
                    let mut scored: Vec<(usize, u32, Vec<usize>)> = self
                        .items
                        .iter()
                        .enumerate()
                        .filter_map(|(idx, item)| {
                            self.matcher
                                .match_score(&self.query, &item.display)
                                .map(|score| {
                                    let indices =
                                        self.matcher.match_indices(&self.query, &item.display);
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
        if self.mode == FinderMode::Grep && self.grep_search_running {
            format!("{}/{} searching", self.filtered.len(), self.items.len())
        } else {
            format!("{}/{}", self.filtered.len(), self.items.len())
        }
    }

    /// Execute the pending grep search (called after debounce from main loop)
    pub fn execute_grep_search(&mut self) {
        if self.mode != FinderMode::Grep || !self.grep_search_pending {
            return;
        }

        self.grep_search_pending = false;

        if self.query.len() >= 2 {
            let cwd = self.cwd.clone();
            let query = self.query.clone();
            let searcher = self.grep_searcher.clone();
            self.grep_search_generation = self.grep_search_generation.wrapping_add(1);
            let generation = self.grep_search_generation;
            let (tx, rx) = mpsc::channel();

            self.grep_search_receiver = Some(rx);
            self.grep_search_running = true;
            self.items.clear();
            self.filtered.clear();
            self.selected = 0;
            self.scroll_offset = 0;
            self.clear_preview_cache();

            thread::spawn(move || {
                const GREP_RESULT_BATCH_SIZE: usize = 50;
                let finished_query = query.clone();
                let tx_finished = tx.clone();

                searcher.search_stream(&cwd, &query, GREP_RESULT_BATCH_SIZE, |items| {
                    tx.send(GrepSearchMessage::Batch {
                        generation,
                        query: query.clone(),
                        items,
                    })
                    .is_ok()
                });

                let _ = tx_finished.send(GrepSearchMessage::Finished {
                    generation,
                    query: finished_query,
                });
            });
        } else {
            self.items.clear();
            self.filtered.clear();
            self.cancel_grep_search();
            self.clear_preview_cache();
        }
    }

    pub fn poll_grep_search(&mut self) -> bool {
        let mut changed = false;

        loop {
            let message = match self.grep_search_receiver.as_ref() {
                Some(rx) => rx.try_recv(),
                None => return changed,
            };

            match message {
                Ok(GrepSearchMessage::Batch {
                    generation,
                    query,
                    items,
                }) => {
                    if self.mode != FinderMode::Grep
                        || generation != self.grep_search_generation
                        || query != self.query
                    {
                        continue;
                    }

                    let had_items = !self.items.is_empty();
                    self.items.extend(items);
                    self.filtered = (0..self.items.len()).collect();
                    self.update_grep_match_indices();

                    if !had_items {
                        self.selected = 0;
                        self.scroll_offset = 0;
                        self.clear_preview_cache();
                        if self.preview_enabled {
                            self.preview_update_pending = true;
                        }
                    }

                    changed = true;
                }
                Ok(GrepSearchMessage::Finished { generation, query }) => {
                    self.grep_search_receiver = None;

                    if self.mode == FinderMode::Grep
                        && generation == self.grep_search_generation
                        && query == self.query
                    {
                        self.grep_search_running = false;
                        changed = true;
                    }

                    return changed;
                }
                Err(TryRecvError::Empty) => return changed,
                Err(TryRecvError::Disconnected) => {
                    self.grep_search_receiver = None;
                    self.grep_search_running = false;
                    return changed;
                }
            }
        }
    }

    /// Toggle preview panel on/off
    pub fn toggle_preview(&mut self) {
        self.preview_enabled = !self.preview_enabled;
        if self.preview_enabled {
            // Clear cache to force reload when re-enabled
            self.clear_preview_cache();
        }
    }

    pub fn clear_preview_cache(&mut self) {
        self.preview_content.clear();
        self.preview_scroll = 0;
        self.preview_line_offset = 0;
        self.preview_path = None;
        self.preview_line = None;
        self.preview_update_pending = false;
    }

    /// Update preview content if the selected file changed
    /// Returns the current preview path and content for rendering
    pub fn update_preview_content(&mut self) -> Option<(PathBuf, &[String])> {
        if !self.preview_enabled {
            return None;
        }

        // Only show preview for Files, Grep, Harpoon, and Marks modes
        if self.mode != FinderMode::Files
            && self.mode != FinderMode::Grep
            && self.mode != FinderMode::Harpoon
            && self.mode != FinderMode::Marks
        {
            return None;
        }

        let selected_item = self.selected_item()?;
        let selected_path = selected_item.path.clone();
        let selected_line = selected_item.line;

        let should_reload = self.preview_path.as_ref() != Some(&selected_path)
            || (self.mode == FinderMode::Grep && self.preview_line != selected_line);

        // Check if we need to load new content
        if should_reload {
            self.preview_content.clear();
            self.preview_scroll = 0;
            self.preview_line_offset = 0;
            self.preview_path = Some(selected_path.clone());
            self.preview_line = selected_line;

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

            // Read only the lines we need using buffered reader.
            // For grep results, read a window around the matching line so deep
            // matches still preview correctly without loading the whole file.
            const MAX_PREVIEW_LINES: usize = 150;
            const GREP_PREVIEW_CONTEXT_BEFORE: usize = 10;
            let start_line = if self.mode == FinderMode::Grep {
                selected_line
                    .map(|line_num| {
                        line_num
                            .saturating_sub(1)
                            .saturating_sub(GREP_PREVIEW_CONTEXT_BEFORE)
                    })
                    .unwrap_or(0)
            } else {
                0
            };
            self.preview_line_offset = start_line;

            match File::open(&selected_path) {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    let mut line_count = 0;
                    for line in reader.lines().skip(start_line).take(MAX_PREVIEW_LINES + 1) {
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
                                    self.preview_content =
                                        vec!["(Unable to read file)".to_string()];
                                }
                                break;
                            }
                        }
                    }
                    if self.preview_content.is_empty() {
                        self.preview_content = vec!["(No preview at requested line)".to_string()];
                        self.preview_line_offset = 0;
                    }
                    if line_count > MAX_PREVIEW_LINES {
                        self.preview_content.push("... (truncated)".to_string());
                    }
                }
                Err(_) => {
                    self.preview_content = vec!["(Unable to read file)".to_string()];
                }
            }

            self.preview_scroll = 0;
        }

        Some((self.preview_path.clone()?, &self.preview_content))
    }

    /// Scroll preview down
    pub fn scroll_preview_down(&mut self, amount: usize) {
        if !self.preview_content.is_empty() {
            self.preview_scroll = self
                .preview_scroll
                .saturating_add(amount)
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

fn grep_result_snippet_start(display: &str) -> usize {
    let mut colon_count = 0;

    for (idx, ch) in display.char_indices() {
        if ch == ':' {
            colon_count += 1;
            if colon_count == 2 {
                let after_colon = idx + ch.len_utf8();
                let whitespace_bytes = display[after_colon..]
                    .chars()
                    .take_while(|ch| ch.is_whitespace())
                    .map(char::len_utf8)
                    .sum::<usize>();
                return after_colon + whitespace_bytes;
            }
        }
    }

    0
}

fn grep_result_match_indices(display: &str, query: &str, start_byte: usize) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }

    let query_lower = query.to_lowercase();
    let query_len = query.chars().count();
    let start_char = display[..start_byte].chars().count();
    let mut next_start_char = start_char;
    let mut indices = Vec::new();

    for (char_idx, (byte_idx, _)) in display.char_indices().enumerate() {
        if char_idx < next_start_char {
            continue;
        }

        if display[byte_idx..].to_lowercase().starts_with(&query_lower) {
            let end_char = char_idx + query_len;
            indices.extend(char_idx..end_char);
            next_start_char = end_char.max(char_idx + 1);
        }
    }

    indices
}

/// Check if a file is likely binary based on extension
fn is_likely_binary(path: &PathBuf) -> bool {
    let binary_exts = [
        "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "svg", "mp3", "mp4", "wav", "avi",
        "mov", "mkv", "flv", "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "exe", "dll", "so",
        "dylib", "bin", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "wasm", "o", "a",
        "class", "pyc", "ttf", "otf", "woff", "woff2", "eot", "db", "sqlite", "sqlite3",
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

#[cfg(test)]
mod tests {
    use super::{FinderInputMode, FinderItem, FinderMode, FuzzyFinder, GrepSearchMessage};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("nevi_{}_{}_{}", name, std::process::id(), nanos))
    }

    fn write_numbered_file(path: &std::path::Path, needle_lines: &[usize]) {
        let mut content = String::new();
        for line_num in 1..=260 {
            if needle_lines.contains(&line_num) {
                content.push_str(&format!("line {} needle\n", line_num));
            } else {
                content.push_str(&format!("line {}\n", line_num));
            }
        }
        fs::write(path, content).unwrap();
    }

    fn wait_for_grep_results(finder: &mut FuzzyFinder) {
        for _ in 0..100 {
            if finder.poll_grep_search() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("async grep search did not finish");
    }

    #[test]
    fn grep_preview_loads_window_around_deep_match() {
        let root = unique_temp_dir("finder_grep_preview");
        fs::create_dir_all(root.join("src")).unwrap();
        let path = root.join("src/main.rs");
        write_numbered_file(&path, &[220]);

        let mut finder = FuzzyFinder::new();
        finder.preview_enabled = true;
        finder.open_grep_with_query(&root, "needle");
        wait_for_grep_results(&mut finder);

        assert_eq!(finder.items.len(), 1);
        finder.update_preview_content();

        assert!(finder.preview_line_offset > 0);
        let match_idx = finder
            .preview_content
            .iter()
            .position(|line| line.contains("needle"))
            .expect("preview should include deep grep match");
        assert_eq!(finder.preview_line_offset + match_idx + 1, 220);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_finder_filters_sessions_by_display() {
        let mut finder = FuzzyFinder::new();
        finder.open_terminals(vec![
            FinderItem::new(
                "  1  server             #1 hidden".to_string(),
                PathBuf::new(),
            )
            .with_terminal_session_position(1)
            .with_icon("TR"),
            FinderItem::new(
                "*  2  git                #2 visible".to_string(),
                PathBuf::new(),
            )
            .with_terminal_session_position(2)
            .with_icon("TR"),
        ]);

        assert_eq!(finder.mode, FinderMode::Terminals);
        assert_eq!(finder.input_mode, FinderInputMode::Normal);

        finder.insert_char('g');

        assert_eq!(finder.filtered.len(), 1);
        assert_eq!(
            finder.selected_item().unwrap().terminal_session_position,
            Some(2)
        );
    }

    #[test]
    fn grep_preview_reloads_for_same_file_different_match_line() {
        let root = unique_temp_dir("finder_grep_preview_same_file");
        fs::create_dir_all(root.join("src")).unwrap();
        let path = root.join("src/main.rs");
        write_numbered_file(&path, &[20, 220]);

        let mut finder = FuzzyFinder::new();
        finder.mode = FinderMode::Grep;
        finder.preview_enabled = true;
        finder.items = vec![
            FinderItem::new("src/main.rs:20: line 20 needle".to_string(), path.clone())
                .with_line(20),
            FinderItem::new("src/main.rs:220: line 220 needle".to_string(), path).with_line(220),
        ];
        finder.filtered = vec![0, 1];

        finder.selected = 0;
        finder.update_preview_content();
        let first_offset = finder.preview_line_offset;

        finder.selected = 1;
        finder.update_preview_content();

        assert_ne!(finder.preview_line_offset, first_offset);
        let match_idx = finder
            .preview_content
            .iter()
            .position(|line| line.contains("line 220 needle"))
            .expect("preview should reload around second match");
        assert_eq!(finder.preview_line_offset + match_idx + 1, 220);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn grep_query_below_threshold_clears_stale_preview() {
        let mut finder = FuzzyFinder::new();
        finder.mode = FinderMode::Grep;
        finder.query = "n".to_string();
        finder.preview_content = vec!["stale preview".to_string()];
        finder.preview_path = Some(PathBuf::from("src/main.rs"));
        finder.preview_line = Some(42);
        finder.preview_line_offset = 41;
        finder.grep_search_pending = true;
        finder.items = vec![FinderItem::new(
            "src/main.rs:42: old match".to_string(),
            PathBuf::from("src/main.rs"),
        )
        .with_line(42)];
        finder.filtered = vec![0];

        finder.update_filter();

        assert!(finder.items.is_empty());
        assert!(finder.filtered.is_empty());
        assert!(!finder.grep_search_pending);
        assert!(finder.preview_content.is_empty());
        assert_eq!(finder.preview_path, None);
        assert_eq!(finder.preview_line, None);
        assert_eq!(finder.preview_line_offset, 0);
    }

    #[test]
    fn poll_grep_search_applies_batches_before_finished() {
        let mut finder = FuzzyFinder::new();
        finder.mode = FinderMode::Grep;
        finder.query = "needle".to_string();
        finder.grep_search_generation = 7;
        finder.grep_search_running = true;

        let (tx, rx) = mpsc::channel();
        finder.grep_search_receiver = Some(rx);

        tx.send(GrepSearchMessage::Batch {
            generation: 7,
            query: "needle".to_string(),
            items: vec![
                FinderItem::new(
                    "src/main.rs:1: needle one".to_string(),
                    PathBuf::from("src/main.rs"),
                )
                .with_line(1),
                FinderItem::new(
                    "src/main.rs:2: needle two".to_string(),
                    PathBuf::from("src/main.rs"),
                )
                .with_line(2),
            ],
        })
        .unwrap();

        assert!(finder.poll_grep_search());
        assert_eq!(finder.items.len(), 2);
        assert_eq!(finder.filtered, vec![0, 1]);
        assert!(finder.grep_search_running);
        assert!(finder.grep_search_receiver.is_some());

        tx.send(GrepSearchMessage::Finished {
            generation: 7,
            query: "needle".to_string(),
        })
        .unwrap();

        assert!(finder.poll_grep_search());
        assert!(!finder.grep_search_running);
        assert!(finder.grep_search_receiver.is_none());
    }

    #[test]
    fn grep_result_highlight_ignores_path_matches() {
        let mut finder = FuzzyFinder::new();
        finder.mode = FinderMode::Grep;
        finder.query = "main".to_string();
        finder.items = vec![FinderItem::new(
            "src/main.rs:12: no match here".to_string(),
            PathBuf::from("src/main.rs"),
        )];

        finder.update_grep_match_indices();

        assert!(finder.items[0].match_indices.is_empty());
    }

    #[test]
    fn grep_result_highlight_starts_in_match_snippet() {
        let mut finder = FuzzyFinder::new();
        finder.mode = FinderMode::Grep;
        finder.query = "main".to_string();
        finder.items = vec![FinderItem::new(
            "src/main.rs:12: fn main() { main(); }".to_string(),
            PathBuf::from("src/main.rs"),
        )];

        finder.update_grep_match_indices();

        let chars: Vec<char> = finder.items[0].display.chars().collect();
        let highlighted: String = finder.items[0]
            .match_indices
            .iter()
            .map(|idx| chars[*idx])
            .collect();

        assert_eq!(highlighted, "mainmain");
        assert!(finder.items[0]
            .match_indices
            .iter()
            .all(|idx| *idx >= "src/main.rs:12: ".chars().count()));
    }
}
