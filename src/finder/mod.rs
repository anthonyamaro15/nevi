mod file_picker;
mod grep;
#[cfg(test)]
mod grep_bench;
mod keybinds;
mod matcher;

pub use file_picker::FilePicker;
pub use grep::GrepSearcher;
pub(crate) use grep::case_insensitive_match_ranges;
pub use keybinds::keymap_finder_items;
pub use matcher::FuzzyMatcher;

use encoding_rs_io::DecodeReaderBytesBuilder;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

enum FileListMessage {
    Batch {
        generation: u64,
        items: Vec<FinderItem>,
    },
    Finished {
        generation: u64,
    },
}

/// Mode for the fuzzy finder
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderMode {
    Files,
    Grep,
    Buffers,
    BufferLines,
    Diagnostics,
    Harpoon,
    Marks,
    GitChanges,
    Terminals,
    Keymaps,
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
    /// Git status metadata for git changes finder results
    pub git_status: Option<crate::git::GitFileStatus>,
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
            git_status: None,
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

    pub fn with_git_status(mut self, status: crate::git::GitFileStatus) -> Self {
        self.git_status = Some(status);
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

fn git_changes_display_path(display: &str) -> (usize, &str) {
    if let Some((prefix, path)) = display.split_once(' ') {
        (prefix.chars().count() + 1, path)
    } else {
        (0, display)
    }
}

fn offset_match_indices(
    matcher: &mut FuzzyMatcher,
    query: &str,
    match_text: &str,
    char_start: usize,
) -> Vec<usize> {
    matcher
        .match_indices(query, match_text)
        .into_iter()
        .map(|idx| char_start + idx)
        .collect()
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
    /// Stop flag of the running grep search. Set when a newer search replaces
    /// it or the picker closes; the search threads check it between files, on
    /// matches, and while reading.
    grep_search_cancel: Option<Arc<AtomicBool>>,
    /// Whether the async file walk is still streaming results
    pub file_list_running: bool,
    /// Monotonic generation used to discard stale file walk results
    file_list_generation: u64,
    /// Receiver for the currently running async file walk
    file_list_receiver: Option<Receiver<FileListMessage>>,
    /// When the current file walk started (for the finder_list_files metric)
    file_list_started: Option<std::time::Instant>,
    /// Wall time of the walk that just finished; the main loop takes this
    /// and records it, since the finder has no recorder access.
    pub last_file_list_duration: Option<std::time::Duration>,
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
            grep_search_cancel: None,
            file_list_running: false,
            file_list_generation: 0,
            file_list_receiver: None,
            file_list_started: None,
            last_file_list_duration: None,
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
            grep_search_cancel: None,
            file_list_running: false,
            file_list_generation: 0,
            file_list_receiver: None,
            file_list_started: None,
            last_file_list_duration: None,
        }
    }

    /// Open the finder in file mode. The walk streams from a background
    /// thread (poll_file_list applies batches), so the picker opens instantly
    /// and the UI never blocks on the file system.
    pub fn open_files(&mut self, cwd: &std::path::Path) {
        self.mode = FinderMode::Files;
        self.input_mode = FinderInputMode::Insert;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;
        self.clear_preview_cache();
        self.cancel_background_work();

        self.items.clear();
        self.filtered.clear();
        self.populated = true;

        const FILE_LIST_BATCH_SIZE: usize = 500;
        self.file_list_generation = self.file_list_generation.wrapping_add(1);
        let generation = self.file_list_generation;
        let picker = self.file_picker.clone();
        let root = cwd.to_path_buf();
        let (tx, rx) = mpsc::channel();
        self.file_list_receiver = Some(rx);
        self.file_list_running = true;
        self.file_list_started = Some(std::time::Instant::now());

        thread::spawn(move || {
            let tx_finished = tx.clone();
            picker.list_files_streaming(&root, FILE_LIST_BATCH_SIZE, |items| {
                tx.send(FileListMessage::Batch { generation, items })
                    .is_ok()
            });
            let _ = tx_finished.send(FileListMessage::Finished { generation });
        });
    }

    fn cancel_file_list(&mut self) {
        self.file_list_receiver = None;
        self.file_list_running = false;
        self.file_list_started = None;
        self.file_list_generation = self.file_list_generation.wrapping_add(1);
    }

    /// Stop every background producer (grep search and file walk). Called by
    /// all open_* entry points and on close so stale threads die promptly.
    pub fn cancel_background_work(&mut self) {
        self.cancel_grep_search();
        self.cancel_file_list();
    }

    /// Apply pending async file walk results. Returns true when the list
    /// changed and a redraw is needed.
    pub fn poll_file_list(&mut self) -> bool {
        let mut changed = false;

        loop {
            let message = match self.file_list_receiver.as_ref() {
                Some(rx) => rx.try_recv(),
                None => return changed,
            };

            match message {
                Ok(FileListMessage::Batch { generation, items }) => {
                    if self.mode != FinderMode::Files || generation != self.file_list_generation {
                        continue;
                    }

                    let had_items = !self.items.is_empty();
                    let new_start = self.items.len();
                    self.items.extend(items);

                    if self.query.is_empty() {
                        self.filtered.extend(new_start..self.items.len());
                    } else {
                        // Score only the new arrivals and keep the filtered
                        // list ordered; items already listed keep their score.
                        let query = self.query.clone();
                        for idx in new_start..self.items.len() {
                            if let Some(score) = self
                                .matcher
                                .match_score(&query, self.items[idx].display.as_str())
                            {
                                self.items[idx].score = score;
                                self.filtered.push(idx);
                            }
                        }
                        let items = &self.items;
                        self.filtered
                            .sort_by_key(|&idx| (std::cmp::Reverse(items[idx].score), idx));
                    }

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
                Ok(FileListMessage::Finished { generation }) => {
                    self.file_list_receiver = None;

                    if self.mode == FinderMode::Files && generation == self.file_list_generation {
                        self.file_list_running = false;
                        self.last_file_list_duration =
                            self.file_list_started.take().map(|start| start.elapsed());
                        // The parallel walk arrives unordered; snap to the
                        // stable alphabetical order now that it is complete.
                        self.items.sort_by(|a, b| a.display.cmp(&b.display));
                        self.update_filter();
                        changed = true;
                    }

                    return changed;
                }
                Err(TryRecvError::Empty) => return changed,
                Err(TryRecvError::Disconnected) => {
                    self.file_list_receiver = None;
                    self.file_list_running = false;
                    return changed;
                }
            }
        }
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
        self.cancel_background_work();

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

    pub fn open_buffer_lines(
        &mut self,
        buffer_idx: usize,
        buffer_path: PathBuf,
        lines: Vec<(usize, String)>,
    ) {
        self.mode = FinderMode::BufferLines;
        self.input_mode = FinderInputMode::Insert;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;
        self.clear_preview_cache();
        self.cancel_background_work();

        self.items = lines
            .into_iter()
            .map(|(line_idx, text)| {
                FinderItem::new(
                    format!("{:>4}: {}", line_idx + 1, text),
                    buffer_path.clone(),
                )
                .with_buffer_idx(buffer_idx)
                .with_line(line_idx + 1)
                .with_col(0)
                .with_icon("LN")
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
        self.cancel_background_work();

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
        self.cancel_background_work();

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
        self.cancel_background_work();

        self.items = terminal_items;
        self.filtered = (0..self.items.len()).collect();
        self.populated = true;
    }

    /// Open the finder in keymaps (cheatsheet) mode — read-only, no preview pane.
    pub fn open_keymaps(&mut self, items: Vec<FinderItem>) {
        self.open_keymaps_with_query(items, "");
    }

    /// Open the finder in keymaps mode with a pre-filled query.
    pub fn open_keymaps_with_query(&mut self, items: Vec<FinderItem>, query: &str) {
        self.mode = FinderMode::Keymaps;
        self.input_mode = FinderInputMode::Insert;
        self.query = query.to_string();
        self.cursor = query.chars().count();
        self.selected = 0;
        self.scroll_offset = 0;
        self.clear_preview_cache();
        self.cancel_background_work();

        self.items = items;
        self.populated = true;
        self.update_filter();
    }

    /// Open the finder in git changes mode
    pub fn open_git_changes(&mut self, items: Vec<FinderItem>) {
        self.mode = FinderMode::GitChanges;
        self.input_mode = FinderInputMode::Insert;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.scroll_offset = 0;
        self.clear_preview_cache();
        self.cancel_background_work();
        self.preview_enabled = true;

        self.items = items;
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
        self.cancel_background_work();

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
        self.cancel_background_work();

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
        self.cancel_background_work();

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
        self.stop_grep_search();
        self.grep_search_pending = false;
        self.grep_search_running = false;
        self.grep_search_receiver = None;
        self.grep_search_generation = self.grep_search_generation.wrapping_add(1);
    }

    /// Tell the running search, if any, to stop. Dropping its receiver is not
    /// enough: a search only noticed that when it handed over results, so one
    /// with few hits scanned the whole repo after it was replaced or closed.
    fn stop_grep_search(&mut self) {
        if let Some(cancel) = self.grep_search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Recompute match highlight indices for items[start..].
    fn update_grep_match_indices_from(&mut self, start: usize) {
        for item in &mut self.items[start..] {
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
                    self.cancel_background_work();
                    self.clear_preview_cache();
                }
            }
            _ => {
                // For files/buffers, use fuzzy matching
                if self.query.is_empty() {
                    // No filter, show all items
                    self.filtered = (0..self.items.len()).collect();
                } else {
                    // Score-and-sort only. Highlight indices are only ever read
                    // for the rows on screen, so ensure_visible_match_indices
                    // computes them lazily instead of paying for every match
                    // on every keystroke.
                    let mode = self.mode;
                    let query = self.query.clone();
                    let matcher = &mut self.matcher;
                    let mut scored: Vec<(usize, u32)> = self
                        .items
                        .iter()
                        .enumerate()
                        .filter_map(|(idx, item)| {
                            let match_text = if mode == FinderMode::GitChanges {
                                git_changes_display_path(&item.display).1
                            } else {
                                item.display.as_str()
                            };
                            matcher
                                .match_score(&query, match_text)
                                .map(|score| (idx, score))
                        })
                        .collect();

                    // Remember scores on the items: while the file walk is
                    // still streaming, poll_file_list merges new arrivals into
                    // the filtered order by item score.
                    for (idx, score) in &scored {
                        self.items[*idx].score = *score;
                    }

                    // Sort by score (higher is better); stable so equal scores
                    // keep their item order.
                    scored.sort_by(|a, b| b.1.cmp(&a.1));

                    self.filtered = scored.into_iter().map(|(idx, _)| idx).collect();
                }
            }
        }

        // Reset selection
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Compute match highlight indices for just the rows about to render.
    /// update_filter no longer computes indices for every match; this tops up
    /// the visible window (a row keeps its indices until the query changes).
    /// Grep mode is excluded: its indices are set when result batches arrive.
    pub fn ensure_visible_match_indices(&mut self, visible_height: usize) {
        if self.query.is_empty() || self.mode == FinderMode::Grep {
            return;
        }

        let (start, end) = self.visible_range(visible_height);
        let start = start.min(end);
        let mode = self.mode;
        let query = self.query.clone();

        for filtered_idx in start..end {
            let item_idx = self.filtered[filtered_idx];
            let Some(item) = self.items.get_mut(item_idx) else {
                continue;
            };
            if !item.match_indices.is_empty() {
                continue;
            }
            let (match_start, match_text) = if mode == FinderMode::GitChanges {
                git_changes_display_path(&item.display)
            } else {
                (0, item.display.as_str())
            };
            item.match_indices =
                offset_match_indices(&mut self.matcher, &query, match_text, match_start);
        }
    }

    /// Get display text for the current filter state
    pub fn status_text(&self) -> String {
        if self.mode == FinderMode::Grep && self.grep_search_running {
            format!("{}/{} searching", self.filtered.len(), self.items.len())
        } else if self.mode == FinderMode::Files && self.file_list_running {
            format!("{}/{} scanning", self.filtered.len(), self.items.len())
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
            // A newer query replaces the running search.
            self.stop_grep_search();
            let cancel = Arc::new(AtomicBool::new(false));
            self.grep_search_cancel = Some(Arc::clone(&cancel));
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

                searcher.search_stream(&cwd, &query, GREP_RESULT_BATCH_SIZE, &cancel, |items| {
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
            self.cancel_background_work();
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
                    // Indices for already-applied batches are still valid (the
                    // query is fixed for a generation), so only the new batch
                    // needs highlighting work.
                    let new_start = self.items.len();
                    self.items.extend(items);
                    self.filtered = (0..self.items.len()).collect();
                    self.update_grep_match_indices_from(new_start);

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
                    // The receiver is this search's own, so its flag goes too.
                    self.grep_search_receiver = None;
                    self.grep_search_cancel = None;

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
                    self.grep_search_cancel = None;
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

    pub fn mode_supports_preview(&self) -> bool {
        matches!(
            self.mode,
            FinderMode::Files
                | FinderMode::Grep
                | FinderMode::Harpoon
                | FinderMode::Marks
                | FinderMode::GitChanges
        )
    }

    pub fn set_preview_content(&mut self, path: PathBuf, content: Vec<String>) {
        self.preview_content = content;
        self.preview_scroll = 0;
        self.preview_line_offset = 0;
        self.preview_path = Some(path);
        self.preview_line = None;
        self.preview_update_pending = false;
    }

    /// Update preview content if the selected file changed
    /// Returns the current preview path and content for rendering
    pub fn update_preview_content(&mut self) -> Option<(PathBuf, &[String])> {
        if !self.preview_enabled {
            return None;
        }

        if !self.mode_supports_preview() || self.mode == FinderMode::GitChanges {
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
                    // Decode a UTF-16 file with a byte order mark the way live
                    // grep does, so its hits preview; other files pass through.
                    let decoded = DecodeReaderBytesBuilder::new()
                        .utf8_passthru(true)
                        .strip_bom(true)
                        .build(file);
                    let mut reader = BufReader::with_capacity(64 * 1024, decoded);
                    // Skip to the window without building a string per line:
                    // ~11ms instead of ~32ms for a hit on line 1,000,000 of a
                    // 72 MB file. It is still a linear read on the main thread
                    // (~150ms per GB); load off-thread if multi-GB files show up.
                    for _ in 0..start_line {
                        if reader.skip_until(b'\n').unwrap_or(0) == 0 {
                            break;
                        }
                    }
                    let mut line_count = 0;
                    for line in reader.split(b'\n').take(MAX_PREVIEW_LINES + 1) {
                        match line {
                            // Live grep treats a NUL byte as a binary file too.
                            Ok(bytes) if bytes.contains(&0) => {
                                self.preview_content =
                                    vec!["(Binary file - no preview)".to_string()];
                                self.preview_line_offset = 0;
                                break;
                            }
                            Ok(bytes) => {
                                if line_count < MAX_PREVIEW_LINES {
                                    let bytes = bytes.strip_suffix(b"\r").unwrap_or(&bytes);
                                    // Bytes that aren't UTF-8 show as U+FFFD,
                                    // like live grep's rows, instead of
                                    // ending the preview there.
                                    self.preview_content
                                        .push(String::from_utf8_lossy(bytes).into_owned());
                                }
                                line_count += 1;
                            }
                            Err(_) => {
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

    // Fast path: lowercase the line once and substring-search the lowered
    // copy. This needs 1:1 char alignment between original and lowered text;
    // lowercasing can expand a char (e.g. 'İ' becomes two chars), so fall
    // back to the per-position scan when counts disagree.
    let display_lower = display.to_lowercase();
    if display_lower.chars().count() != display.chars().count()
        || query_lower.chars().count() != query_len
    {
        return grep_result_match_indices_per_position(
            display,
            &query_lower,
            query_len,
            start_byte,
        );
    }

    let start_char = display[..start_byte].chars().count();
    let mut search_byte = display_lower
        .char_indices()
        .nth(start_char)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(display_lower.len());
    let mut chars_before = start_char;
    let mut indices = Vec::new();

    while let Some(rel) = display_lower[search_byte..].find(&query_lower) {
        let abs = search_byte + rel;
        let match_char = chars_before + display_lower[search_byte..abs].chars().count();
        indices.extend(match_char..match_char + query_len);
        // Non-overlapping, matching the per-position scan: resume after the match.
        chars_before = match_char + query_len;
        search_byte = abs + query_lower.len();
    }

    indices
}

/// Original per-position scan, kept for text where lowercasing changes char
/// counts. O(len^2) allocations, acceptable because this path is rare.
fn grep_result_match_indices_per_position(
    display: &str,
    query_lower: &str,
    query_len: usize,
    start_byte: usize,
) -> Vec<usize> {
    let start_char = display[..start_byte].chars().count();
    let mut next_start_char = start_char;
    let mut indices = Vec::new();

    for (char_idx, (byte_idx, _)) in display.char_indices().enumerate() {
        if char_idx < next_start_char {
            continue;
        }

        if display[byte_idx..].to_lowercase().starts_with(query_lower) {
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
    use super::{
        FileListMessage, FinderInputMode, FinderItem, FinderMode, FuzzyFinder, GrepSearchMessage,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::Ordering;
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
    fn open_keymaps_sets_mode_and_loads_items() {
        let mut finder = FuzzyFinder::new();
        finder.open_keymaps(vec![
            FinderItem::new(
                "n       h                  Move cursor left".to_string(),
                PathBuf::new(),
            ),
            FinderItem::new(
                "leader  <leader>ff         Find files".to_string(),
                PathBuf::new(),
            ),
        ]);

        assert_eq!(finder.mode, FinderMode::Keymaps);
        assert_eq!(finder.items.len(), 2);
        assert!(finder.populated);
        assert!(
            !finder.mode_supports_preview(),
            "keymaps mode has no preview pane"
        );
    }

    #[test]
    fn open_buffer_lines_sets_line_metadata_and_fuzzy_filters() {
        let mut finder = FuzzyFinder::new();
        finder.open_buffer_lines(
            2,
            PathBuf::from("src/main.rs"),
            vec![
                (0, "fn main() {".to_string()),
                (1, "    println!(\"hello\");".to_string()),
                (2, "}".to_string()),
            ],
        );

        assert_eq!(finder.mode, FinderMode::BufferLines);
        assert_eq!(finder.filtered.len(), 3);
        assert_eq!(
            finder
                .selected_item()
                .map(|item| (item.buffer_idx, item.line, item.col)),
            Some((Some(2), Some(1), Some(0)))
        );

        for ch in "print".chars() {
            finder.insert_char(ch);
        }

        assert_eq!(finder.filtered.len(), 1);
        assert_eq!(
            finder.selected_item().map(|item| item.display.as_str()),
            Some("   2:     println!(\"hello\");")
        );
    }

    #[test]
    fn open_keymaps_with_query_prefilters_items() {
        let mut finder = FuzzyFinder::new();
        finder.open_keymaps_with_query(
            vec![
                FinderItem::new(
                    "n       h                  Move cursor left".to_string(),
                    PathBuf::new(),
                ),
                FinderItem::new(
                    "expl    gg                 Move to top".to_string(),
                    PathBuf::new(),
                ),
                FinderItem::new(
                    "expl    <C-d>              Move half page down".to_string(),
                    PathBuf::new(),
                ),
            ],
            "expl",
        );

        assert_eq!(finder.mode, FinderMode::Keymaps);
        assert_eq!(finder.query, "expl");
        assert_eq!(finder.cursor, 4);
        assert_eq!(finder.filtered.len(), 2);
        assert!(finder.filtered.iter().all(|idx| {
            finder
                .items
                .get(*idx)
                .map(|item| item.display.starts_with("expl"))
                .unwrap_or(false)
        }));
    }

    #[test]
    fn keymap_items_are_inert() {
        let items = crate::finder::keymap_finder_items(&crate::config::KeymapSettings::default());
        assert!(!items.is_empty(), "expected some implemented keybindings");
        for item in &items {
            assert_eq!(
                item.path,
                std::path::PathBuf::new(),
                "keymap items must have no path"
            );
            assert!(item.buffer_idx.is_none());
            assert!(item.terminal_session_position.is_none());
            assert!(item.git_status.is_none());
            assert!(item.line.is_none());
        }
    }

    #[test]
    fn git_changes_finder_opens_with_preview_and_status_metadata() {
        let mut finder = FuzzyFinder::new();
        let item = FinderItem::new("M src/main.rs".to_string(), PathBuf::from("src/main.rs"))
            .with_git_status(crate::git::GitFileStatus::Modified);

        finder.open_git_changes(vec![item]);

        assert_eq!(finder.mode, FinderMode::GitChanges);
        assert!(finder.preview_enabled);
        assert!(finder.mode_supports_preview());
        assert_eq!(finder.items.len(), 1);
        assert_eq!(
            finder.selected_item().and_then(|item| item.git_status),
            Some(crate::git::GitFileStatus::Modified)
        );
    }

    #[test]
    fn git_changes_finder_fuzzy_filters_paths() {
        let mut finder = FuzzyFinder::new();
        finder.open_git_changes(vec![
            FinderItem::new("M src/main.rs".to_string(), PathBuf::from("src/main.rs"))
                .with_git_status(crate::git::GitFileStatus::Modified),
            FinderItem::new("A README.md".to_string(), PathBuf::from("README.md"))
                .with_git_status(crate::git::GitFileStatus::Added),
        ]);

        finder.insert_char('r');
        finder.insert_char('e');
        finder.insert_char('a');
        finder.insert_char('d');

        assert_eq!(finder.filtered.len(), 1);
        assert_eq!(
            finder.selected_item().map(|item| item.path.as_path()),
            Some(Path::new("README.md"))
        );
    }

    #[test]
    fn git_changes_finder_highlights_path_matches_in_display_text() {
        let mut finder = FuzzyFinder::new();
        finder.open_git_changes(vec![
            FinderItem::new(
                "M src/lib/types.ts".to_string(),
                PathBuf::from("/repo/src/lib/types.ts"),
            )
            .with_git_status(crate::git::GitFileStatus::Modified),
        ]);

        for ch in "type".chars() {
            finder.insert_char(ch);
        }
        finder.ensure_visible_match_indices(10);

        let item = finder.selected_item().expect("matching item");
        let highlighted: String = item
            .match_indices
            .iter()
            .map(|&idx| item.display.chars().nth(idx).expect("matched char"))
            .collect();
        assert_eq!(highlighted, "type");
    }

    #[test]
    fn git_changes_finder_does_not_filter_by_status_prefix() {
        let mut finder = FuzzyFinder::new();
        finder.open_git_changes(vec![
            FinderItem::new("! src/main.rs".to_string(), PathBuf::from("src/main.rs"))
                .with_git_status(crate::git::GitFileStatus::Conflicted),
            FinderItem::new("? README.md".to_string(), PathBuf::from("README.md"))
                .with_git_status(crate::git::GitFileStatus::Untracked),
        ]);

        finder.insert_char('!');

        assert!(finder.filtered.is_empty());
    }

    #[test]
    fn git_changes_update_preview_waits_for_injected_content() {
        let mut finder = FuzzyFinder::new();
        finder.open_git_changes(vec![
            FinderItem::new("M src/main.rs".to_string(), PathBuf::from("src/main.rs"))
                .with_git_status(crate::git::GitFileStatus::Modified),
        ]);

        assert_eq!(finder.update_preview_content(), None);
    }

    #[test]
    fn set_preview_content_replaces_pending_preview_state() {
        let mut finder = FuzzyFinder::new();
        finder.preview_content = vec!["old".to_string()];
        finder.preview_scroll = 3;
        finder.preview_line_offset = 9;
        finder.preview_path = Some(PathBuf::from("old.rs"));
        finder.preview_line = Some(12);
        finder.preview_update_pending = true;

        finder.set_preview_content(PathBuf::from("src/main.rs"), vec!["diff".to_string()]);

        assert_eq!(finder.preview_content, vec!["diff"]);
        assert_eq!(finder.preview_scroll, 0);
        assert_eq!(finder.preview_line_offset, 0);
        assert_eq!(finder.preview_path, Some(PathBuf::from("src/main.rs")));
        assert_eq!(finder.preview_line, None);
        assert!(!finder.preview_update_pending);
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

    fn wait_for_grep_to_finish(finder: &mut FuzzyFinder) {
        for _ in 0..500 {
            finder.poll_grep_search();
            if !finder.grep_search_running {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("grep search did not finish");
    }

    /// Grep `root` for `query`, then select each hit and return the preview
    /// line shown at it, as (hit line, text), sorted by line.
    fn preview_at_each_hit(root: &Path, query: &str) -> Vec<(usize, String)> {
        let mut finder = FuzzyFinder::new();
        finder.preview_enabled = true;
        finder.open_grep_with_query(root, query);
        wait_for_grep_to_finish(&mut finder);
        let mut shown = Vec::new();
        for idx in 0..finder.filtered.len() {
            finder.selected = idx;
            finder.update_preview_content();
            let hit = finder.selected_item().unwrap().line.unwrap();
            let at = hit - 1 - finder.preview_line_offset;
            let text = finder.preview_content.get(at).cloned().unwrap_or_else(|| {
                format!("(no line {at} in preview {:?})", finder.preview_content)
            });
            shown.push((hit, text));
        }
        shown.sort();
        shown
    }

    #[test]
    fn grep_preview_shows_a_hit_deep_in_a_large_file() {
        let root = unique_temp_dir("finder_preview_large");
        fs::create_dir_all(&root).unwrap();
        // CRLF endings, a line that isn't UTF-8, and a 1 MB line all come
        // before the hit, so the skip has to count each of them as one line.
        let mut content = Vec::new();
        for line in 1..=200_000 {
            match line {
                5 => content.extend_from_slice(b"caf\xe9"),
                6 => content.extend_from_slice("x".repeat(1_000_000).as_bytes()),
                199_990 => content.extend_from_slice(b"the needle is here"),
                _ => content.extend_from_slice(format!("line {line}").as_bytes()),
            }
            content.extend_from_slice(b"\r\n");
        }
        fs::write(root.join("big.txt"), content).unwrap();

        assert_eq!(
            preview_at_each_hit(&root, "needle"),
            vec![(199_990, "the needle is here".to_string())]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn grep_preview_reads_utf16_files_with_a_byte_order_mark() {
        let root = unique_temp_dir("finder_preview_utf16");
        fs::create_dir_all(&root).unwrap();
        let mut text = String::from("\"needle_first\" = \"Primera\";\n");
        for line in 2..300 {
            text.push_str(&format!("\"key_{line}\" = \"value {line}\";\n"));
        }
        text.push_str("\"needle_deep\" = \"Última\";\n");
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        fs::write(root.join("Localizable.strings"), bytes).unwrap();

        assert_eq!(
            preview_at_each_hit(&root, "needle"),
            vec![
                (1, "\"needle_first\" = \"Primera\";".to_string()),
                (300, "\"needle_deep\" = \"Última\";".to_string()),
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn grep_preview_keeps_going_past_bytes_that_are_not_utf8() {
        let root = unique_temp_dir("finder_preview_latin1");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("notes.txt"), b"one\ncaf\xe9\nthe needle\nfour\n").unwrap();

        let mut finder = FuzzyFinder::new();
        finder.preview_enabled = true;
        finder.open_grep_with_query(&root, "needle");
        wait_for_grep_to_finish(&mut finder);
        finder.update_preview_content();

        assert_eq!(
            finder.preview_content,
            vec!["one", "caf\u{FFFD}", "the needle", "four"]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_hides_a_utf8_byte_order_mark() {
        let root = unique_temp_dir("finder_preview_utf8_bom");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), b"\xEF\xBB\xBFfirst needle\nsecond\n").unwrap();

        assert_eq!(
            preview_at_each_hit(&root, "needle"),
            vec![(1, "first needle".to_string())]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_treats_a_file_with_a_nul_byte_as_binary() {
        let root = unique_temp_dir("finder_preview_nul");
        fs::create_dir_all(&root).unwrap();
        // No extension, so the extension list can't catch it.
        let path = root.join("blob");
        fs::write(&path, b"ELF\0\x01\x02 not text\n").unwrap();

        let mut finder = FuzzyFinder::new();
        finder.preview_enabled = true;
        finder.mode = FinderMode::Files;
        finder.items = vec![FinderItem::new("blob".to_string(), path)];
        finder.filtered = vec![0];
        finder.update_preview_content();

        assert_eq!(finder.preview_content, vec!["(Binary file - no preview)"]);
        assert_eq!(finder.preview_line_offset, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_newer_search_or_closing_the_picker_stops_the_running_search() {
        let root = unique_temp_dir("finder_grep_cancel");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "needle\n").unwrap();

        let mut finder = FuzzyFinder::new();
        finder.open_grep_with_query(&root, "needle");
        let first = finder.grep_search_cancel.clone().expect("search started");

        finder.query = "needles".to_string();
        finder.grep_search_pending = true;
        finder.execute_grep_search();
        assert!(
            first.load(Ordering::Relaxed),
            "a newer search stops the old one"
        );

        let second = finder.grep_search_cancel.clone().expect("search started");
        finder.cancel_background_work();
        assert!(
            second.load(Ordering::Relaxed),
            "closing the picker stops the search"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_finished_search_lets_go_of_its_stop_flag() {
        // The flag stands for the running search; once it finishes on its own
        // there is nothing left to stop.
        let root = unique_temp_dir("finder_grep_flag_release");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "needle\n").unwrap();

        let mut finder = FuzzyFinder::new();
        finder.open_grep_with_query(&root, "needle");
        assert!(finder.grep_search_cancel.is_some(), "search started");
        for _ in 0..500 {
            finder.poll_grep_search();
            if !finder.grep_search_running {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(!finder.grep_search_running, "search finished");
        assert!(finder.grep_search_cancel.is_none());
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
        finder.items = vec![
            FinderItem::new(
                "src/main.rs:42: old match".to_string(),
                PathBuf::from("src/main.rs"),
            )
            .with_line(42),
        ];
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

        finder.update_grep_match_indices_from(0);

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

        finder.update_grep_match_indices_from(0);

        let chars: Vec<char> = finder.items[0].display.chars().collect();
        let highlighted: String = finder.items[0]
            .match_indices
            .iter()
            .map(|idx| chars[*idx])
            .collect();

        assert_eq!(highlighted, "mainmain");
        assert!(
            finder.items[0]
                .match_indices
                .iter()
                .all(|idx| *idx >= "src/main.rs:12: ".chars().count())
        );
    }

    #[test]
    fn grep_result_highlight_matches_mixed_case_non_overlapping() {
        let display = "src/a.rs:1: Needle needleNEEDLE end";
        let indices = super::grep_result_match_indices(display, "needle", "src/a.rs:1: ".len());

        let chars: Vec<char> = display.chars().collect();
        let highlighted: String = indices.iter().map(|idx| chars[*idx]).collect();
        assert_eq!(highlighted, "NeedleneedleNEEDLE");

        let snippet_start = "src/a.rs:1: ".chars().count();
        assert_eq!(indices[0], snippet_start);
    }

    #[test]
    fn grep_result_highlight_handles_multibyte_before_match() {
        // 'é' is 2 bytes but 1 char; indices are char positions, so the match
        // position must not drift after multibyte text.
        let display = "src/é.rs:1: caf\u{e9} needle";
        let start_byte = display.find("caf").expect("snippet start");
        let indices = super::grep_result_match_indices(display, "needle", start_byte);

        let chars: Vec<char> = display.chars().collect();
        let highlighted: String = indices.iter().map(|idx| chars[*idx]).collect();
        assert_eq!(highlighted, "needle");
        assert_eq!(
            indices[0],
            display.chars().count() - "needle".chars().count()
        );
    }

    fn files_finder_with_items(count: usize) -> FuzzyFinder {
        let mut finder = FuzzyFinder::new();
        finder.mode = FinderMode::Files;
        finder.items = (0..count)
            .map(|i| {
                FinderItem::new(
                    format!("src/module_{i}/some_file_{i}.rs"),
                    PathBuf::from(format!("src/module_{i}/some_file_{i}.rs")),
                )
            })
            .collect();
        finder.filtered = (0..count).collect();
        finder.populated = true;
        finder
    }

    #[test]
    fn match_indices_computed_only_for_visible_rows() {
        let mut finder = files_finder_with_items(100);
        for ch in "file".chars() {
            finder.insert_char(ch);
        }
        assert!(finder.filtered.len() > 10, "query should match many items");

        // Filtering alone computes no indices.
        assert!(
            finder
                .filtered
                .iter()
                .all(|&idx| finder.items[idx].match_indices.is_empty())
        );

        finder.ensure_visible_match_indices(5);

        let (start, end) = finder.visible_range(5);
        for filtered_idx in start..end {
            let item = &finder.items[finder.filtered[filtered_idx]];
            assert!(
                !item.match_indices.is_empty(),
                "visible row must be highlighted: {}",
                item.display
            );
        }
        for filtered_idx in end..finder.filtered.len() {
            let item = &finder.items[finder.filtered[filtered_idx]];
            assert!(
                item.match_indices.is_empty(),
                "off-screen row must stay lazy: {}",
                item.display
            );
        }
    }

    #[test]
    fn scrolling_tops_up_match_indices_for_new_rows() {
        let mut finder = files_finder_with_items(100);
        for ch in "file".chars() {
            finder.insert_char(ch);
        }
        finder.ensure_visible_match_indices(5);

        // Move selection past the visible window and re-ensure, like the key
        // handler does after navigation.
        for _ in 0..7 {
            finder.select_next();
        }
        finder.adjust_scroll(5);
        finder.ensure_visible_match_indices(5);

        let item = &finder.items[finder.filtered[finder.selected]];
        assert!(
            !item.match_indices.is_empty(),
            "row scrolled into view must be highlighted"
        );
    }

    #[test]
    fn grep_result_highlight_falls_back_when_lowercase_expands() {
        // 'İ' lowercases to two chars, breaking 1:1 alignment; the fallback
        // scan must still find the plain match after it.
        let display = "src/a.rs:1: \u{130}stanbul needle";
        let start_byte = "src/a.rs:1: ".len();
        let indices = super::grep_result_match_indices(display, "needle", start_byte);

        let chars: Vec<char> = display.chars().collect();
        let highlighted: String = indices.iter().map(|idx| chars[*idx]).collect();
        assert_eq!(highlighted, "needle");
    }

    #[test]
    fn poll_grep_search_keeps_earlier_batch_indices_intact() {
        let mut finder = FuzzyFinder::new();
        finder.mode = FinderMode::Grep;
        finder.query = "needle".to_string();
        finder.grep_search_generation = 7;
        finder.grep_search_running = true;

        let (tx, rx) = mpsc::channel();
        finder.grep_search_receiver = Some(rx);

        for batch_num in 0..2 {
            tx.send(GrepSearchMessage::Batch {
                generation: 7,
                query: "needle".to_string(),
                items: vec![
                    FinderItem::new(
                        format!("src/main.rs:{}: has needle here", batch_num + 1),
                        PathBuf::from("src/main.rs"),
                    )
                    .with_line(batch_num + 1),
                ],
            })
            .unwrap();
            assert!(finder.poll_grep_search());
        }

        assert_eq!(finder.items.len(), 2);
        for item in &finder.items {
            let chars: Vec<char> = item.display.chars().collect();
            let highlighted: String = item.match_indices.iter().map(|idx| chars[*idx]).collect();
            assert_eq!(highlighted, "needle", "display={}", item.display);
        }
    }

    #[test]
    #[ignore = "perf budget guard; run explicitly with cargo test grep_index_budget -- --ignored --nocapture"]
    fn grep_index_budget_batch_apply_stays_bounded() {
        use std::time::{Duration, Instant};

        // 20 batches x 50 items, 1000 total: the max_results default. Before
        // the batch-scoped index update this re-scanned every accumulated item
        // per batch with O(len^2) lowercasing; the budget locks in the fix.
        let mut finder = FuzzyFinder::new();
        finder.mode = FinderMode::Grep;
        finder.query = "needle".to_string();
        finder.grep_search_generation = 1;
        finder.grep_search_running = true;

        let (tx, rx) = mpsc::channel();
        finder.grep_search_receiver = Some(rx);

        let mut batch_times = Vec::new();
        for batch_num in 0..20 {
            let items: Vec<FinderItem> = (0..50)
                .map(|i| {
                    FinderItem::new(
                        format!(
                            "src/some/dir/file_{batch_num}_{i}.rs:{i}: let value = needle_probe_{i}; // padding padding padding padding needle",
                        ),
                        PathBuf::from("src/some/dir/file.rs"),
                    )
                    .with_line(i + 1)
                })
                .collect();
            tx.send(GrepSearchMessage::Batch {
                generation: 1,
                query: "needle".to_string(),
                items,
            })
            .unwrap();

            let started = Instant::now();
            assert!(finder.poll_grep_search());
            batch_times.push(started.elapsed());
        }

        assert_eq!(finder.items.len(), 1000);

        batch_times.sort_unstable();
        let p95 = batch_times[(batch_times.len() * 95).div_ceil(100) - 1];
        // Debug-build budget with CI-runner headroom (release p95 is ~114us,
        // debug ~416us). Catches order-of-magnitude regressions like the old
        // full-rescan, which grows past this as batches accumulate.
        let budget = Duration::from_millis(25);
        println!(
            "grep batch apply: p95={p95:?} max={:?} budget={budget:?}",
            batch_times.last().unwrap()
        );
        assert!(
            p95 <= budget,
            "grep batch apply p95 exceeded budget: p95={p95:?} budget={budget:?}"
        );
    }

    #[test]
    fn query_change_invalidates_stale_match_indices() {
        let mut finder = files_finder_with_items(20);
        for ch in "file".chars() {
            finder.insert_char(ch);
        }
        finder.ensure_visible_match_indices(10);
        let before = finder.items[finder.filtered[0]].match_indices.clone();
        assert!(!before.is_empty());

        finder.insert_char('_');
        // Filtering cleared everything; ensure recomputes for the new query.
        finder.ensure_visible_match_indices(10);
        let item = &finder.items[finder.filtered[0]];
        assert!(!item.match_indices.is_empty());
        assert_ne!(item.match_indices, before, "indices must track the query");
    }

    #[test]
    #[ignore = "perf budget guard; run explicitly with cargo test finder_filter_budget -- --ignored --nocapture"]
    fn finder_filter_budget_keystroke_stays_bounded() {
        use std::time::{Duration, Instant};

        // 10k items is the file picker cap. Each keystroke re-filters all of
        // them; the visible-row ensure afterwards mirrors the key handler.
        let mut finder = files_finder_with_items(10_000);
        let mut keystroke_times = Vec::new();

        for ch in "somefil".chars() {
            let started = Instant::now();
            finder.insert_char(ch);
            finder.ensure_visible_match_indices(40);
            keystroke_times.push(started.elapsed());
        }

        keystroke_times.sort_unstable();
        let p95 = keystroke_times[(keystroke_times.len() * 95).div_ceil(100) - 1];
        // Debug-build budget with CI-runner headroom (release p95 is ~3.2ms,
        // debug ~46ms). The laziness itself is guarded by the visible-rows
        // regression tests; this catches order-of-magnitude blowups.
        let budget = Duration::from_millis(250);
        println!(
            "finder filter keystroke: p95={p95:?} max={:?} budget={budget:?}",
            keystroke_times.last().unwrap()
        );
        assert!(
            p95 <= budget,
            "finder filter keystroke p95 exceeded budget: p95={p95:?} budget={budget:?}"
        );
    }

    fn wait_for_file_list(finder: &mut FuzzyFinder) {
        for _ in 0..200 {
            finder.poll_file_list();
            if !finder.file_list_running {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("async file list did not finish");
    }

    #[test]
    fn open_files_streams_and_sorts_alphabetically_on_finish() {
        let root = unique_temp_dir("finder_stream_files");
        fs::create_dir_all(root.join("src")).unwrap();
        for name in ["zeta.rs", "alpha.rs", "midpoint.rs"] {
            fs::write(root.join("src").join(name), "x").unwrap();
        }

        let mut finder = FuzzyFinder::new();
        finder.open_files(&root);
        assert!(
            finder.populated,
            "picker is usable before the walk finishes"
        );
        wait_for_file_list(&mut finder);

        let displays: Vec<&str> = finder
            .filtered
            .iter()
            .map(|&idx| finder.items[idx].display.as_str())
            .collect();
        assert_eq!(
            displays,
            vec!["src/alpha.rs", "src/midpoint.rs", "src/zeta.rs"],
            "finished list must be sorted by path"
        );
        assert!(!finder.file_list_running);
        assert!(finder.last_file_list_duration.is_some());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_list_batches_merge_into_scored_order_while_query_active() {
        let mut finder = FuzzyFinder::new();
        finder.mode = FinderMode::Files;
        finder.populated = true;
        finder.file_list_generation = 3;
        finder.file_list_running = true;

        let (tx, rx) = mpsc::channel();
        finder.file_list_receiver = Some(rx);

        tx.send(FileListMessage::Batch {
            generation: 3,
            items: vec![
                FinderItem::new("src/other.rs".to_string(), PathBuf::from("src/other.rs")),
                FinderItem::new("src/main.rs".to_string(), PathBuf::from("src/main.rs")),
            ],
        })
        .unwrap();
        assert!(finder.poll_file_list());

        for ch in "main".chars() {
            finder.insert_char(ch);
        }
        assert_eq!(finder.filtered.len(), 1);

        // A batch arriving after typing must be scored against the live query
        // and merged in order, not appended blindly.
        tx.send(FileListMessage::Batch {
            generation: 3,
            items: vec![
                FinderItem::new(
                    "src/main_helpers.rs".to_string(),
                    PathBuf::from("src/main_helpers.rs"),
                ),
                FinderItem::new("src/zzz.rs".to_string(), PathBuf::from("src/zzz.rs")),
            ],
        })
        .unwrap();
        assert!(finder.poll_file_list());

        let displays: Vec<&str> = finder
            .filtered
            .iter()
            .map(|&idx| finder.items[idx].display.as_str())
            .collect();
        assert_eq!(displays.len(), 2, "zzz.rs does not match the query");
        assert!(displays.contains(&"src/main.rs"));
        assert!(displays.contains(&"src/main_helpers.rs"));
        let scores: Vec<u32> = finder
            .filtered
            .iter()
            .map(|&idx| finder.items[idx].score)
            .collect();
        assert!(
            scores.windows(2).all(|pair| pair[0] >= pair[1]),
            "filtered must stay ordered by score: {scores:?}"
        );
    }

    #[test]
    fn stale_file_list_batches_are_discarded() {
        let mut finder = FuzzyFinder::new();
        finder.mode = FinderMode::Files;
        finder.populated = true;
        finder.file_list_generation = 5;
        finder.file_list_running = true;

        let (tx, rx) = mpsc::channel();
        finder.file_list_receiver = Some(rx);

        tx.send(FileListMessage::Batch {
            generation: 4,
            items: vec![FinderItem::new(
                "stale.rs".to_string(),
                PathBuf::from("stale.rs"),
            )],
        })
        .unwrap();

        assert!(!finder.poll_file_list());
        assert!(finder.items.is_empty());
    }
}
