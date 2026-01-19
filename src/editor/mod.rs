mod buffer;
mod cursor;
mod marks;
mod register;
mod undo;

pub use buffer::Buffer;
pub use cursor::Cursor;
pub use marks::{Mark, Marks};
pub use register::{RegisterContent, Registers};
pub use undo::{Change, UndoEntry, UndoStack};

use crate::input::{InputState, Motion, apply_motion, TextObject, TextObjectModifier, TextObjectType, CaseOperator};
use crate::commands::CommandLine;
use crate::syntax::SyntaxManager;
use crate::config::{Settings, KeymapLookup};
use crate::explorer::FileExplorer;
use crate::finder::FuzzyFinder;
use crate::frecency::FrecencyDb;
use crate::lsp::types::{CodeActionItem, Diagnostic, CompletionItem, Location, TextEdit};
use crate::theme::ThemeManager;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The current mode of the editor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Replace,
    Command,
    Search,
    Visual,
    VisualLine,
    VisualBlock,
    Finder,
    Explorer,
    RenamePrompt,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Replace => "REPLACE",
            Mode::Command => "COMMAND",
            Mode::Search => "SEARCH",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "V-LINE",
            Mode::VisualBlock => "V-BLOCK",
            Mode::Finder => "FINDER",
            Mode::Explorer => "EXPLORER",
            Mode::RenamePrompt => "RENAME",
        }
    }

    pub fn is_visual(&self) -> bool {
        matches!(self, Mode::Visual | Mode::VisualLine | Mode::VisualBlock)
    }
}

/// Pending LSP action requested by key handler
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspAction {
    /// Go to definition (gd)
    GotoDefinition,
    /// Show hover documentation (K)
    Hover,
    /// Format document
    Formatting,
    /// Find references (gr)
    FindReferences,
    /// Show code actions (ga)
    CodeActions,
    /// Rename symbol
    RenameSymbol(String),
}

/// Rectangle representing a screen region
#[derive(Debug, Clone, Copy, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self { x, y, width, height }
    }
}

/// A pane/window in the editor showing a buffer
#[derive(Debug, Clone)]
pub struct Pane {
    /// Index of the buffer this pane displays
    pub buffer_idx: usize,
    /// Cursor position in this pane
    pub cursor: Cursor,
    /// Scroll offset for this pane
    pub viewport_offset: usize,
    /// Screen region for this pane
    pub rect: Rect,
}

impl Pane {
    pub fn new(buffer_idx: usize) -> Self {
        Self {
            buffer_idx,
            cursor: Cursor::default(),
            viewport_offset: 0,
            rect: Rect::default(),
        }
    }
}

/// Split layout orientation for panes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitLayout {
    /// Side-by-side panes (divide width)
    Vertical,
    /// Stacked panes (divide height)
    Horizontal,
}

/// Direction for pane navigation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Visual selection state
#[derive(Debug, Clone, Copy, Default)]
pub struct VisualSelection {
    /// Anchor position (where selection started)
    pub anchor_line: usize,
    pub anchor_col: usize,
}

/// Stores the last visual selection for gv command
#[derive(Debug, Clone)]
pub struct LastVisualSelection {
    pub mode: Mode,
    pub anchor_line: usize,
    pub anchor_col: usize,
    pub cursor_line: usize,
    pub cursor_col: usize,
}

impl VisualSelection {
    pub fn new(line: usize, col: usize) -> Self {
        Self {
            anchor_line: line,
            anchor_col: col,
        }
    }

    /// Get the selection range as (start_line, start_col, end_line, end_col)
    /// The range is inclusive and normalized (start <= end)
    pub fn get_range(&self, cursor_line: usize, cursor_col: usize) -> (usize, usize, usize, usize) {
        if (self.anchor_line, self.anchor_col) <= (cursor_line, cursor_col) {
            (self.anchor_line, self.anchor_col, cursor_line, cursor_col)
        } else {
            (cursor_line, cursor_col, self.anchor_line, self.anchor_col)
        }
    }

    /// Get the line range for line-wise selection
    pub fn get_line_range(&self, cursor_line: usize) -> (usize, usize) {
        if self.anchor_line <= cursor_line {
            (self.anchor_line, cursor_line)
        } else {
            (cursor_line, self.anchor_line)
        }
    }

    /// Get the block range for visual block mode
    /// Returns (top_line, left_col, bottom_line, right_col)
    pub fn get_block_range(&self, cursor_line: usize, cursor_col: usize) -> (usize, usize, usize, usize) {
        let top = self.anchor_line.min(cursor_line);
        let bottom = self.anchor_line.max(cursor_line);
        let left = self.anchor_col.min(cursor_col);
        let right = self.anchor_col.max(cursor_col);
        (top, left, bottom, right)
    }
}

/// Search direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchDirection {
    #[default]
    Forward,
    Backward,
}

/// Search state
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    /// Search input buffer (what user is typing)
    pub input: String,
    /// Cursor position in input
    pub cursor: usize,
    /// Search direction for current search
    pub direction: SearchDirection,
    /// Last search pattern (for n/N)
    pub last_pattern: Option<String>,
    /// Last search direction
    pub last_direction: SearchDirection,
}

impl SearchState {
    /// Clear the search input
    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor = 0;
    }

    /// Start a new search
    pub fn start(&mut self, direction: SearchDirection) {
        self.input.clear();
        self.cursor = 0;
        self.direction = direction;
    }

    /// Insert a character at cursor
    pub fn insert_char(&mut self, ch: char) {
        self.input.insert(self.cursor, ch);
        self.cursor += 1;
    }

    /// Delete character before cursor
    pub fn delete_char_before(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.input.remove(self.cursor);
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
        if self.cursor < self.input.len() {
            self.cursor += 1;
        }
    }

    /// Execute search and save pattern
    pub fn execute(&mut self) -> Option<String> {
        if self.input.is_empty() {
            // Use last pattern if input is empty
            self.last_pattern.clone()
        } else {
            self.last_pattern = Some(self.input.clone());
            self.last_direction = self.direction;
            Some(self.input.clone())
        }
    }

    /// Get the display string for the search prompt
    pub fn display(&self) -> String {
        let prefix = match self.direction {
            SearchDirection::Forward => "/",
            SearchDirection::Backward => "?",
        };
        format!("{}{}", prefix, self.input)
    }
}

/// A position in the jump list
#[derive(Debug, Clone)]
pub struct JumpLocation {
    /// Path to the file (None for scratch buffers)
    pub path: Option<std::path::PathBuf>,
    /// Line number
    pub line: usize,
    /// Column number
    pub col: usize,
}

/// Jump list for navigation history (like Vim's Ctrl+o/Ctrl+i)
#[derive(Debug, Default)]
pub struct JumpList {
    /// List of jump locations
    jumps: Vec<JumpLocation>,
    /// Current position in the jump list
    /// When position == jumps.len(), we're "at the end" (current location, not navigating)
    position: usize,
}

impl JumpList {
    /// Check if we're at the end (not navigating history)
    fn is_at_end(&self) -> bool {
        self.position >= self.jumps.len()
    }

    /// Record a jump (before jumping to a new location)
    pub fn record(&mut self, path: Option<std::path::PathBuf>, line: usize, col: usize) {
        // When making a new jump while navigating, truncate forward history
        if !self.is_at_end() {
            self.jumps.truncate(self.position + 1);
        }

        // Don't record duplicate consecutive jumps
        if let Some(last) = self.jumps.last() {
            if last.path == path && last.line == line {
                self.position = self.jumps.len();
                return;
            }
        }

        self.jumps.push(JumpLocation { path, line, col });
        self.position = self.jumps.len();

        // Limit jump list size
        const MAX_JUMPS: usize = 100;
        if self.jumps.len() > MAX_JUMPS {
            self.jumps.remove(0);
            self.position = self.jumps.len();
        }
    }

    /// Go back in the jump list (Ctrl+o)
    /// Takes current location to save if we're starting to navigate
    pub fn go_back(&mut self, current_path: Option<std::path::PathBuf>, current_line: usize, current_col: usize) -> Option<&JumpLocation> {
        // If at end and we have history, save current position first
        if self.is_at_end() && !self.jumps.is_empty() {
            // Only save if different from last entry
            if let Some(last) = self.jumps.last() {
                if last.path != current_path || last.line != current_line {
                    self.jumps.push(JumpLocation {
                        path: current_path,
                        line: current_line,
                        col: current_col,
                    });
                    self.position = self.jumps.len();
                }
            }
        }

        if self.position > 0 {
            self.position -= 1;
            self.jumps.get(self.position)
        } else {
            None
        }
    }

    /// Go forward in the jump list (Ctrl+i)
    pub fn go_forward(&mut self) -> Option<&JumpLocation> {
        if self.position < self.jumps.len().saturating_sub(1) {
            self.position += 1;
            self.jumps.get(self.position)
        } else {
            None
        }
    }
}

/// Autocomplete state
pub struct CompletionState {
    /// Whether completion popup is active
    pub active: bool,
    /// List of completion items from LSP (original, unfiltered)
    pub items: Vec<CompletionItem>,
    /// Filtered indices into items, sorted by score
    pub filtered: Vec<usize>,
    /// Currently selected index (into filtered)
    pub selected: usize,
    /// Line where completion was triggered
    pub trigger_line: usize,
    /// Column where completion was triggered
    pub trigger_col: usize,
    /// Current filter text (typed since trigger)
    pub filter_text: String,
    /// Fuzzy matcher for filtering
    matcher: crate::finder::FuzzyMatcher,
    /// If true, the completion list is incomplete and typing more should re-request
    pub is_incomplete: bool,
}

impl Default for CompletionState {
    fn default() -> Self {
        Self {
            active: false,
            items: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            trigger_line: 0,
            trigger_col: 0,
            filter_text: String::new(),
            matcher: crate::finder::FuzzyMatcher::new(),
            is_incomplete: false,
        }
    }
}

impl CompletionState {
    /// Show completion popup with items
    pub fn show(&mut self, items: Vec<CompletionItem>, line: usize, col: usize, is_incomplete: bool) {
        self.active = true;
        self.items = items;
        self.selected = 0;
        self.trigger_line = line;
        self.trigger_col = col;
        self.filter_text.clear();
        self.is_incomplete = is_incomplete;
        // Initialize filtered list with all items, sorted by sortText
        self.refilter();
    }

    /// Hide completion popup
    pub fn hide(&mut self) {
        self.active = false;
        self.items.clear();
        self.filtered.clear();
        self.selected = 0;
        self.filter_text.clear();
        self.is_incomplete = false;
    }

    /// Update filter with new prefix text
    pub fn update_filter(&mut self, prefix: &str) {
        self.filter_text = prefix.to_string();
        self.refilter();
    }

    /// Refilter and resort items based on current filter_text
    fn refilter(&mut self) {
        self.refilter_with_frecency(None);
    }

    /// Refilter completions with optional frecency scoring
    pub fn refilter_with_frecency(&mut self, frecency: Option<&FrecencyDb>) {
        if self.filter_text.is_empty() {
            // No filter - show all items sorted by frecency (if available) then sortText
            let mut indices: Vec<(usize, f64, &str)> = self.items.iter()
                .enumerate()
                .map(|(i, item)| {
                    let frecency_score = frecency
                        .map(|f| f.score(&item.label))
                        .unwrap_or(1.0);
                    let sort_key = item.sort_text.as_deref().unwrap_or(&item.label);
                    (i, frecency_score, sort_key)
                })
                .collect();
            // Sort by frecency (higher first), then by sortText
            indices.sort_by(|a, b| {
                // First compare frecency scores (higher is better)
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.2.cmp(b.2))
            });
            self.filtered = indices.into_iter().map(|(i, _, _)| i).collect();
        } else {
            // Fuzzy filter using filterText (fallback to label), combined with frecency
            let mut scored: Vec<(usize, f64)> = self.items.iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    let match_text = item.filter_text.as_deref().unwrap_or(&item.label);
                    self.matcher.match_score(&self.filter_text, match_text).map(|fuzzy_score| {
                        let frecency_score = frecency
                            .map(|f| f.score(&item.label))
                            .unwrap_or(1.0);
                        // Combined score: fuzzy_score * frecency_boost
                        let combined = fuzzy_score as f64 * frecency_score;
                        (i, combined)
                    })
                })
                .collect();
            // Sort by combined score (higher is better)
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.selected = 0;
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected > 0 {
                self.selected -= 1;
            } else {
                self.selected = self.filtered.len() - 1;
            }
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    /// Get currently selected item
    pub fn selected_item(&self) -> Option<&CompletionItem> {
        self.filtered.get(self.selected)
            .and_then(|&idx| self.items.get(idx))
    }

    /// Get the text to insert for the selected item
    pub fn selected_insert_text(&self) -> Option<&str> {
        self.selected_item().map(|item| {
            item.insert_text.as_deref().unwrap_or(&item.label)
        })
    }

    /// Get the number of visible (filtered) items
    pub fn visible_count(&self) -> usize {
        self.filtered.len()
    }

    /// Get the ghost text (untyped portion of selected completion)
    /// Returns None if no completion is selected or if ghost text would be empty
    pub fn ghost_text(&self) -> Option<String> {
        if !self.active || self.filtered.is_empty() {
            return None;
        }

        let insert_text = self.selected_insert_text()?;
        let prefix = &self.filter_text;

        if prefix.is_empty() {
            // No filter text - show full completion as ghost
            return Some(insert_text.to_string());
        }

        // Try case-insensitive prefix match
        let insert_lower = insert_text.to_lowercase();
        let prefix_lower = prefix.to_lowercase();

        if insert_lower.starts_with(&prefix_lower) {
            // Ghost text is the part after the prefix
            let ghost = &insert_text[prefix.len()..];
            if ghost.is_empty() {
                return None;
            }
            return Some(ghost.to_string());
        }

        // Fuzzy match - no clear prefix, don't show ghost text
        // (could show full text but that might be confusing)
        None
    }
}

/// Main editor state
pub struct Editor {
    /// All open buffers
    buffers: Vec<Buffer>,
    /// Index of the currently active buffer
    current_buffer_idx: usize,
    /// All panes (windows)
    panes: Vec<Pane>,
    /// Index of the currently active pane
    active_pane: usize,
    /// Split layout orientation
    split_layout: SplitLayout,
    /// Cursor position (active pane's cursor)
    pub cursor: Cursor,
    /// Current mode
    pub mode: Mode,
    /// Viewport offset (for scrolling, active pane's viewport)
    pub viewport_offset: usize,
    /// Terminal dimensions
    pub term_height: u16,
    pub term_width: u16,
    /// Whether to quit
    pub should_quit: bool,
    /// Status message
    pub status_message: Option<String>,
    /// Registers for yank/paste
    pub registers: Registers,
    /// Input state machine
    pub input_state: InputState,
    /// Command line state
    pub command_line: CommandLine,
    /// Undo/redo history
    pub undo_stack: UndoStack,
    /// Search state
    pub search: SearchState,
    /// Visual selection state
    pub visual: VisualSelection,
    /// Syntax highlighting manager
    pub syntax: SyntaxManager,
    /// Last parsed syntax version for the active buffer
    last_syntax_version: u64,
    /// Time of the last buffer edit (for syntax debounce)
    last_edit_at: Option<Instant>,
    /// Configuration settings
    pub settings: Settings,
    /// Keymap lookup table
    pub keymap: KeymapLookup,
    /// Leader key sequence being built (None if not in leader mode)
    pub leader_sequence: Option<String>,
    /// Pending external command to run (handled by main loop)
    pub pending_external_command: Option<String>,
    /// Fuzzy finder state
    pub finder: FuzzyFinder,
    /// LSP status message (persistent, shown in status bar)
    pub lsp_status: Option<String>,
    /// LSP diagnostics per file URI
    diagnostics: HashMap<String, Vec<Diagnostic>>,
    /// Autocomplete state
    pub completion: CompletionState,
    /// Pending LSP action to execute (handled by main loop)
    pub pending_lsp_action: Option<LspAction>,
    /// Jump list for Ctrl+o/Ctrl+i navigation
    pub jump_list: JumpList,
    /// Hover popup content (shown with K command)
    pub hover_content: Option<String>,
    /// Flag to signal that completion needs to be re-requested (for isIncomplete)
    pub needs_completion_refresh: bool,
    /// Frecency database for completion ranking
    pub frecency: FrecencyDb,
    /// Signature help popup content
    pub signature_help: Option<crate::lsp::types::SignatureHelpResult>,
    /// Show diagnostic floating popup at cursor
    pub show_diagnostic_float: bool,
    /// Incremental search matches: (line, start_col, end_col)
    pub search_matches: Vec<(usize, usize, usize)>,
    /// Project root directory (for scoping file finder and grep)
    pub project_root: Option<std::path::PathBuf>,
    /// File explorer sidebar
    pub explorer: FileExplorer,
    /// Harpoon quick file marks
    pub harpoon: crate::harpoon::Harpoon,
    /// Flag to indicate a formatting request is pending
    pub pending_format: bool,
    /// Flag to indicate we should save after formatting completes
    pub save_after_format: bool,
    /// References picker state
    pub references_picker: Option<ReferencesPicker>,
    /// Code actions picker state
    pub code_actions_picker: Option<CodeActionsPicker>,
    /// Rename prompt input (new name being entered)
    pub rename_input: String,
    /// Original word for rename (shown in prompt)
    pub rename_original: String,
    /// Floating terminal
    pub floating_terminal: crate::floating_terminal::FloatingTerminal,
    /// Pending Copilot action to execute (handled by main loop)
    pub pending_copilot_action: Option<CopilotAction>,
    /// Copilot ghost text state (updated from main loop)
    pub copilot_ghost: Option<CopilotGhostText>,
    /// Git diff status per file (by file path string)
    git_diffs: HashMap<String, crate::git::GitDiff>,
    /// Cached git repository (if project is in git)
    git_repo: Option<crate::git::GitRepo>,
    /// Theme manager for colors and themes
    pub theme_manager: ThemeManager,
    /// Theme picker state (Some if picker is open)
    pub theme_picker: Option<ThemePicker>,
    /// Marks for navigation (m{a-z}, '{a-z}, `{a-z})
    pub marks: Marks,
    /// Last visual selection for gv command
    pub last_visual_selection: Option<LastVisualSelection>,
}

/// Copilot ghost text state for rendering
#[derive(Debug, Clone)]
pub struct CopilotGhostText {
    /// Text to display inline after cursor
    pub inline_text: String,
    /// Additional lines to display as virtual lines
    pub additional_lines: Vec<String>,
    /// Line where ghost text was triggered
    pub trigger_line: usize,
    /// Column where ghost text was triggered
    pub trigger_col: usize,
    /// Completion count display (e.g., "1/3")
    pub count_display: String,
}

/// Copilot action to execute
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopilotAction {
    /// Initiate sign-in
    Auth,
    /// Sign out
    SignOut,
    /// Show status
    Status,
    /// Toggle on/off
    Toggle,
    /// Accept current ghost text completion
    Accept,
    /// Cycle to next completion
    CycleNext,
    /// Cycle to previous completion
    CyclePrev,
    /// Dismiss ghost text
    Dismiss,
}

/// State for references picker UI
#[derive(Debug, Clone)]
pub struct ReferencesPicker {
    /// List of reference locations
    pub items: Vec<Location>,
    /// Currently selected index
    pub selected: usize,
}

impl ReferencesPicker {
    pub fn new(items: Vec<Location>) -> Self {
        Self { items, selected: 0 }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    pub fn selected_item(&self) -> Option<&Location> {
        self.items.get(self.selected)
    }
}

/// State for code actions picker UI
#[derive(Debug, Clone)]
pub struct CodeActionsPicker {
    /// List of available code actions
    pub items: Vec<CodeActionItem>,
    /// Currently selected index
    pub selected: usize,
}

impl CodeActionsPicker {
    pub fn new(items: Vec<CodeActionItem>) -> Self {
        Self { items, selected: 0 }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    pub fn selected_item(&self) -> Option<&CodeActionItem> {
        self.items.get(self.selected)
    }
}

/// State for theme picker UI
#[derive(Debug, Clone)]
pub struct ThemePicker {
    /// List of all available themes (name, is_bundled)
    pub all_items: Vec<(String, bool)>,
    /// Filtered list of theme indices matching the search query
    pub filtered: Vec<usize>,
    /// Currently selected index in filtered list
    pub selected: usize,
    /// Search query for filtering themes
    pub query: String,
}

impl ThemePicker {
    pub fn new(items: Vec<(&str, bool)>) -> Self {
        let all_items: Vec<(String, bool)> = items.into_iter().map(|(s, b)| (s.to_string(), b)).collect();
        let filtered: Vec<usize> = (0..all_items.len()).collect();
        Self {
            all_items,
            filtered,
            selected: 0,
            query: String::new(),
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    /// Get the currently selected theme name
    pub fn selected_name(&self) -> Option<&str> {
        self.filtered.get(self.selected)
            .and_then(|&idx| self.all_items.get(idx))
            .map(|(name, _)| name.as_str())
    }

    /// Get the items that should be displayed (filtered list)
    pub fn visible_items(&self) -> Vec<&(String, bool)> {
        self.filtered.iter()
            .filter_map(|&idx| self.all_items.get(idx))
            .collect()
    }

    /// Add a character to the search query and update filter
    pub fn add_char(&mut self, c: char) {
        self.query.push(c);
        self.update_filter();
    }

    /// Remove a character from the search query and update filter
    pub fn delete_char(&mut self) {
        self.query.pop();
        self.update_filter();
    }

    /// Update the filtered list based on the current query
    fn update_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.all_items.len()).collect();
        } else {
            let query_lower = self.query.to_lowercase();
            self.filtered = self.all_items.iter()
                .enumerate()
                .filter(|(_, (name, _))| name.to_lowercase().contains(&query_lower))
                .map(|(idx, _)| idx)
                .collect();
        }
        // Reset selection if out of bounds
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }
}

impl Editor {
    pub fn new(settings: Settings) -> Self {
        let keymap = KeymapLookup::from_settings(&settings.keymap);
        let finder = FuzzyFinder::from_settings(&settings.finder);

        // Initialize theme manager with bundled + user themes
        let mut theme_manager = ThemeManager::new();
        theme_manager.load_user_themes();
        // Set initial theme from config
        theme_manager.set_theme(&settings.theme.colorscheme);

        // Create syntax manager and sync it with the UI theme
        let mut syntax = SyntaxManager::new();
        syntax.sync_theme(theme_manager.theme());

        Self {
            buffers: vec![Buffer::new()],
            current_buffer_idx: 0,
            panes: vec![Pane::new(0)],
            active_pane: 0,
            split_layout: SplitLayout::Vertical,
            cursor: Cursor::default(),
            mode: Mode::default(),
            viewport_offset: 0,
            term_height: 24,
            term_width: 80,
            should_quit: false,
            status_message: None,
            registers: Registers::new(),
            input_state: InputState::new(),
            command_line: CommandLine::new(),
            undo_stack: UndoStack::new(),
            search: SearchState::default(),
            visual: VisualSelection::default(),
            syntax,
            last_syntax_version: 0,
            last_edit_at: None,
            settings,
            keymap,
            leader_sequence: None,
            pending_external_command: None,
            finder,
            lsp_status: None,
            diagnostics: HashMap::new(),
            completion: CompletionState::default(),
            pending_lsp_action: None,
            jump_list: JumpList::default(),
            hover_content: None,
            needs_completion_refresh: false,
            frecency: FrecencyDb::load(),
            signature_help: None,
            show_diagnostic_float: false,
            search_matches: Vec::new(),
            project_root: None,
            explorer: FileExplorer::new(),
            harpoon: crate::harpoon::Harpoon::new(),
            pending_format: false,
            save_after_format: false,
            references_picker: None,
            code_actions_picker: None,
            rename_input: String::new(),
            rename_original: String::new(),
            floating_terminal: crate::floating_terminal::FloatingTerminal::new(),
            pending_copilot_action: None,
            copilot_ghost: None,
            git_diffs: HashMap::new(),
            git_repo: None,
            theme_manager,
            theme_picker: None,
            marks: Marks::new(),
            last_visual_selection: None,
        }
    }

    /// Set the project root directory
    pub fn set_project_root(&mut self, path: std::path::PathBuf) {
        self.project_root = Some(path.clone());
        self.explorer.set_root(path.clone());
        self.harpoon.set_project_root(path.clone());
        self.floating_terminal.set_working_dir(path);
    }

    /// Get the current theme
    pub fn theme(&self) -> &crate::theme::Theme {
        self.theme_manager.theme()
    }

    /// Set theme by name and sync syntax highlighting
    pub fn set_theme(&mut self, name: &str) -> bool {
        if self.theme_manager.set_theme(name) {
            self.syntax.sync_theme(self.theme_manager.theme());
            true
        } else {
            false
        }
    }

    /// Open the theme picker
    pub fn open_theme_picker(&mut self) {
        let items = self.theme_manager.list_themes_sorted();
        self.theme_picker = Some(ThemePicker::new(items));
        self.theme_manager.start_preview();
    }

    /// Close the theme picker
    pub fn close_theme_picker(&mut self, confirm: bool) {
        if confirm {
            self.theme_manager.confirm_preview();
            // Sync syntax colors with confirmed theme
            self.syntax.sync_theme(self.theme_manager.theme());
        } else {
            self.theme_manager.cancel_preview();
            // Sync syntax colors with restored theme
            self.syntax.sync_theme(self.theme_manager.theme());
        }
        self.theme_picker = None;
    }

    /// Preview a theme in the picker
    pub fn preview_theme(&mut self, name: &str) {
        if self.theme_manager.preview_theme(name) {
            self.syntax.sync_theme(self.theme_manager.theme());
        }
    }

    /// Get the project root or current working directory
    pub fn working_directory(&self) -> std::path::PathBuf {
        self.project_root.clone().unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        })
    }

    /// Record completion selection for frecency ranking
    pub fn record_completion_use(&mut self, label: &str) {
        self.frecency.record_use(label);
        self.frecency.save();
    }

    /// Show completion popup with frecency-aware sorting
    pub fn show_completions(&mut self, items: Vec<CompletionItem>, line: usize, col: usize, is_incomplete: bool) {
        self.completion.show(items, line, col, is_incomplete);
        self.completion.refilter_with_frecency(Some(&self.frecency));
    }

    /// Update completion filter with frecency-aware sorting
    pub fn update_completion_filter(&mut self, prefix: &str) {
        self.completion.update_filter(prefix);
        self.completion.refilter_with_frecency(Some(&self.frecency));
    }

    /// Update a completion item with resolved documentation
    pub fn update_completion_item_documentation(
        &mut self,
        label: &str,
        documentation: Option<String>,
        detail: Option<String>,
    ) {
        // Find the item by label and update its documentation
        for item in &mut self.completion.items {
            if item.label == label {
                if documentation.is_some() {
                    item.documentation = documentation;
                }
                if detail.is_some() {
                    item.detail = detail;
                }
                break;
            }
        }
    }

    /// Set the LSP status (persistent, shown in status bar)
    pub fn set_lsp_status<S: Into<String>>(&mut self, msg: S) {
        self.lsp_status = Some(msg.into());
    }

    /// Update diagnostics for a file URI
    pub fn set_diagnostics(&mut self, uri: String, diags: Vec<Diagnostic>) {
        self.diagnostics.insert(uri, diags);
    }

    /// Apply text edits from LSP formatting (or other sources)
    /// Edits are applied in reverse order to preserve positions
    pub fn apply_text_edits(&mut self, edits: &[TextEdit]) {
        if edits.is_empty() {
            return;
        }

        // Sort edits by position (reverse order) so we can apply from end to start
        let mut sorted_edits: Vec<&TextEdit> = edits.iter().collect();
        sorted_edits.sort_by(|a, b| {
            // Compare by end position first (descending)
            match b.end_line.cmp(&a.end_line) {
                std::cmp::Ordering::Equal => b.end_col.cmp(&a.end_col),
                other => other,
            }
        });

        // Begin an undo group for all formatting changes
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

        // Apply each edit
        for edit in sorted_edits {
            // Get the text being replaced for undo (end_col - 1 because get_range_text uses inclusive end)
            let deleted_text = if edit.end_col > 0 || edit.end_line > edit.start_line {
                self.get_range_text(
                    edit.start_line,
                    edit.start_col,
                    edit.end_line,
                    edit.end_col.saturating_sub(1),
                )
            } else {
                String::new()
            };

            // Record the deletion for undo
            if !deleted_text.is_empty() {
                self.undo_stack.record_change(Change::delete(
                    edit.start_line,
                    edit.start_col,
                    deleted_text,
                ));
            }

            // Delete the range from the buffer (LSP end_col is exclusive)
            if edit.end_col > 0 || edit.end_line > edit.start_line {
                self.buffers[self.current_buffer_idx].delete_range(
                    edit.start_line,
                    edit.start_col,
                    edit.end_line,
                    edit.end_col,
                );
            }

            // Insert the new text
            if !edit.new_text.is_empty() {
                self.undo_stack.record_change(Change::insert(
                    edit.start_line,
                    edit.start_col,
                    edit.new_text.clone(),
                ));

                // Insert the text using insert_str method
                self.buffers[self.current_buffer_idx].insert_str(
                    edit.start_line,
                    edit.start_col,
                    &edit.new_text,
                );
            }
        }

        // Mark buffer as modified and invalidate syntax
        self.buffers[self.current_buffer_idx].mark_modified();
        self.last_edit_at = Some(Instant::now());

        // End the undo group so LSP edits are a single undo operation
        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);

        // Ensure cursor is in valid position
        self.clamp_cursor();
    }

    /// Get diagnostics for the current buffer
    pub fn current_diagnostics(&self) -> &[Diagnostic] {
        if let Some(path) = &self.buffer().path {
            let uri = crate::lsp::path_to_uri(path);
            self.diagnostics.get(&uri).map(|v| v.as_slice()).unwrap_or(&[])
        } else {
            &[]
        }
    }

    /// Get diagnostics for a specific line in the current buffer
    pub fn diagnostics_for_line(&self, line: usize) -> Vec<&Diagnostic> {
        self.current_diagnostics()
            .iter()
            .filter(|d| d.line == line)
            .collect()
    }

    /// Get the first diagnostic message for the cursor line (for status display)
    pub fn diagnostic_at_cursor(&self) -> Option<&Diagnostic> {
        self.diagnostics_for_line(self.cursor.line).into_iter().next()
    }

    /// Get all diagnostics at cursor position (for code actions)
    pub fn all_diagnostics_at_cursor(&self) -> Vec<Diagnostic> {
        self.diagnostics_for_line(self.cursor.line)
            .into_iter()
            .filter(|d| d.col_start <= self.cursor.col && self.cursor.col <= d.col_end)
            .cloned()
            .collect()
    }

    /// Go to next diagnostic after cursor position
    /// Returns true if a diagnostic was found and cursor moved
    pub fn goto_next_diagnostic(&mut self) -> bool {
        let cursor_line = self.cursor.line;
        let cursor_col = self.cursor.col;

        // Find target position (copy the values to avoid borrow issues)
        let target_pos = {
            let diagnostics = self.current_diagnostics();
            if diagnostics.is_empty() {
                return false;
            }

            // Find first diagnostic after cursor (same line but later column, or later line)
            let next = diagnostics.iter().find(|d| {
                d.line > cursor_line || (d.line == cursor_line && d.col_start > cursor_col)
            });

            // If nothing found after cursor, wrap to first diagnostic
            let target = next.or_else(|| diagnostics.first());
            target.map(|d| (d.line, d.col_start))
        };

        if let Some((line, col)) = target_pos {
            self.cursor.line = line;
            self.cursor.col = col;
            self.scroll_to_cursor();
            true
        } else {
            false
        }
    }

    /// Go to previous diagnostic before cursor position
    /// Returns true if a diagnostic was found and cursor moved
    pub fn goto_prev_diagnostic(&mut self) -> bool {
        let cursor_line = self.cursor.line;
        let cursor_col = self.cursor.col;

        // Find target position (copy the values to avoid borrow issues)
        let target_pos = {
            let diagnostics = self.current_diagnostics();
            if diagnostics.is_empty() {
                return false;
            }

            // Find last diagnostic before cursor (same line but earlier column, or earlier line)
            let prev = diagnostics.iter().rev().find(|d| {
                d.line < cursor_line || (d.line == cursor_line && d.col_start < cursor_col)
            });

            // If nothing found before cursor, wrap to last diagnostic
            let target = prev.or_else(|| diagnostics.last());
            target.map(|d| (d.line, d.col_start))
        };

        if let Some((line, col)) = target_pos {
            self.cursor.line = line;
            self.cursor.col = col;
            self.scroll_to_cursor();
            true
        } else {
            false
        }
    }

    // ============================================
    // Git Integration
    // ============================================

    /// Initialize git repository from project root
    pub fn init_git(&mut self) {
        if let Some(root) = &self.project_root {
            self.git_repo = crate::git::GitRepo::open(root);
        }
    }

    /// Set git diff for a file path
    pub fn set_git_diff(&mut self, path: String, diff: crate::git::GitDiff) {
        self.git_diffs.insert(path, diff);
    }

    /// Get git status for a specific line in the current buffer
    pub fn git_status_for_line(&self, line: usize) -> Option<crate::git::GitLineStatus> {
        let path = self.buffer().path.as_ref()?.to_string_lossy().to_string();
        let diff = self.git_diffs.get(&path)?;
        diff.status_for_line(line)
    }

    /// Get git status for a specific line given a file path
    pub fn git_status_for_line_in_file(&self, path: &std::path::Path, line: usize) -> Option<crate::git::GitLineStatus> {
        let path_str = path.to_string_lossy().to_string();
        let diff = self.git_diffs.get(&path_str)?;
        diff.status_for_line(line)
    }

    /// Update git diff for the current buffer
    pub fn update_git_diff(&mut self) {
        let Some(repo) = &self.git_repo else { return };
        let Some(path) = self.buffer().path.clone() else { return };

        let Some(head_content) = repo.head_content(&path) else {
            // File not tracked by git or new file - clear any existing diff
            let path_str = path.to_string_lossy().to_string();
            self.git_diffs.remove(&path_str);
            return;
        };

        let current_content = self.buffer().content();
        let diff = crate::git::compute_diff(&head_content, &current_content);

        self.set_git_diff(path.to_string_lossy().to_string(), diff);
    }

    /// Get reference to the git repository (if available)
    pub fn git_repo(&self) -> Option<&crate::git::GitRepo> {
        self.git_repo.as_ref()
    }

    // ============================================
    // Buffer Accessors
    // ============================================

    /// Get a reference to the current buffer
    pub fn buffer(&self) -> &Buffer {
        &self.buffers[self.current_buffer_idx]
    }

    /// Get a mutable reference to the current buffer
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current_buffer_idx]
    }

    /// Get the number of open buffers
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Get the index of the current buffer
    pub fn current_buffer_index(&self) -> usize {
        self.current_buffer_idx
    }

    /// Get a reference to all panes
    pub fn panes(&self) -> &[Pane] {
        &self.panes
    }

    /// Get the index of the active pane
    pub fn active_pane_idx(&self) -> usize {
        self.active_pane
    }

    /// Get a reference to all buffers
    pub fn buffers(&self) -> &[Buffer] {
        &self.buffers
    }

    /// Get a reference to a specific buffer by index
    pub fn buffer_at(&self, idx: usize) -> Option<&Buffer> {
        self.buffers.get(idx)
    }

    /// Get the current split layout
    pub fn split_layout(&self) -> SplitLayout {
        self.split_layout
    }

    /// Switch to the next buffer
    pub fn next_buffer(&mut self) {
        if self.buffers.len() > 1 {
            self.current_buffer_idx = (self.current_buffer_idx + 1) % self.buffers.len();
            self.cursor = Cursor::default();
            self.viewport_offset = 0;
            // Re-parse syntax for new buffer
            let path = self.buffers[self.current_buffer_idx].path.clone();
            self.syntax.set_language_from_path_option(path.as_ref());
            self.parse_current_buffer();
        }
    }

    /// Switch to the previous buffer
    pub fn prev_buffer(&mut self) {
        if self.buffers.len() > 1 {
            if self.current_buffer_idx == 0 {
                self.current_buffer_idx = self.buffers.len() - 1;
            } else {
                self.current_buffer_idx -= 1;
            }
            self.cursor = Cursor::default();
            self.viewport_offset = 0;
            // Re-parse syntax for new buffer
            let path = self.buffers[self.current_buffer_idx].path.clone();
            self.syntax.set_language_from_path_option(path.as_ref());
            self.parse_current_buffer();
        }
    }

    // ============================================
    // Pane Management
    // ============================================

    /// Get the number of panes
    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    /// Get the active pane index
    pub fn active_pane_index(&self) -> usize {
        self.active_pane
    }

    /// Save current pane state before switching
    fn save_pane_state(&mut self) {
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].cursor = self.cursor;
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
            self.panes[self.active_pane].buffer_idx = self.current_buffer_idx;
        }
    }

    /// Load pane state when switching to it
    fn load_pane_state(&mut self) {
        if self.active_pane < self.panes.len() {
            self.cursor = self.panes[self.active_pane].cursor;
            self.viewport_offset = self.panes[self.active_pane].viewport_offset;
            self.current_buffer_idx = self.panes[self.active_pane].buffer_idx;
            // Re-parse syntax for the buffer
            let path = self.buffers[self.current_buffer_idx].path.clone();
            self.syntax.set_language_from_path_option(path.as_ref());
            self.parse_current_buffer();
        }
    }

    /// Create a vertical split (new pane to the right)
    pub fn vsplit(&mut self, file_path: Option<std::path::PathBuf>) -> anyhow::Result<()> {
        self.split_pane(file_path, SplitLayout::Vertical)
    }

    /// Create a horizontal split (new pane below)
    pub fn hsplit(&mut self, file_path: Option<std::path::PathBuf>) -> anyhow::Result<()> {
        self.split_pane(file_path, SplitLayout::Horizontal)
    }

    fn split_pane(&mut self, file_path: Option<std::path::PathBuf>, layout: SplitLayout) -> anyhow::Result<()> {
        // Save current pane state
        self.save_pane_state();
        self.split_layout = layout;

        // Determine which buffer the new pane shows
        let new_buffer_idx = if let Some(path) = file_path {
            // Open file in new buffer
            let new_buffer = Buffer::from_file(path)?;
            self.buffers.push(new_buffer);
            self.buffers.len() - 1
        } else {
            // Same buffer as current pane
            self.current_buffer_idx
        };

        // Create new pane
        let new_pane = Pane::new(new_buffer_idx);
        self.panes.push(new_pane);

        // Update pane layout
        self.update_pane_rects();

        // Switch to new pane
        let new_pane_idx = self.panes.len() - 1;
        self.active_pane = new_pane_idx;
        self.load_pane_state();

        self.set_status(format!("Pane {}/{}", self.active_pane + 1, self.panes.len()));
        Ok(())
    }

    /// Switch to the next pane
    pub fn next_pane(&mut self) {
        if self.panes.len() > 1 {
            self.save_pane_state();
            self.active_pane = (self.active_pane + 1) % self.panes.len();
            self.load_pane_state();
            self.set_status(format!("Pane {}/{}", self.active_pane + 1, self.panes.len()));
        }
    }

    /// Switch to the previous pane
    pub fn prev_pane(&mut self) {
        if self.panes.len() > 1 {
            self.save_pane_state();
            if self.active_pane == 0 {
                self.active_pane = self.panes.len() - 1;
            } else {
                self.active_pane -= 1;
            }
            self.load_pane_state();
            self.set_status(format!("Pane {}/{}", self.active_pane + 1, self.panes.len()));
        }
    }

    /// Close the current pane
    pub fn close_pane(&mut self) -> bool {
        if self.panes.len() > 1 {
            self.panes.remove(self.active_pane);
            if self.active_pane >= self.panes.len() {
                self.active_pane = self.panes.len() - 1;
            }
            self.update_pane_rects();
            self.load_pane_state();
            self.set_status(format!("Pane {}/{}", self.active_pane + 1, self.panes.len()));
            true
        } else {
            // Only one pane, can't close
            false
        }
    }

    /// Close all panes except current
    pub fn close_other_panes(&mut self) {
        if self.panes.len() > 1 {
            let current_pane = self.panes[self.active_pane].clone();
            self.panes = vec![current_pane];
            self.active_pane = 0;
            self.update_pane_rects();
            self.set_status("Only pane remaining");
        }
    }

    /// Move to a pane in the specified direction
    pub fn move_to_pane_direction(&mut self, direction: PaneDirection) {
        // Special case: if moving left with only one pane and explorer is visible, focus explorer
        if self.panes.len() <= 1 {
            if direction == PaneDirection::Left && self.explorer.visible {
                self.focus_explorer();
            }
            return;
        }

        let current_rect = &self.panes[self.active_pane].rect;
        let current_center_x = current_rect.x + current_rect.width / 2;
        let current_center_y = current_rect.y + current_rect.height / 2;

        // Find the best candidate pane in the given direction
        let mut best_pane: Option<usize> = None;
        let mut best_distance = u16::MAX;

        for (idx, pane) in self.panes.iter().enumerate() {
            if idx == self.active_pane {
                continue;
            }

            let rect = &pane.rect;
            let center_x = rect.x + rect.width / 2;
            let center_y = rect.y + rect.height / 2;

            // Check if this pane is in the correct direction
            let is_valid = match direction {
                PaneDirection::Left => center_x < current_center_x,
                PaneDirection::Right => center_x > current_center_x,
                PaneDirection::Up => center_y < current_center_y,
                PaneDirection::Down => center_y > current_center_y,
            };

            if !is_valid {
                continue;
            }

            // Calculate distance (Manhattan distance in the direction)
            let distance = match direction {
                PaneDirection::Left | PaneDirection::Right => {
                    current_center_x.abs_diff(center_x)
                }
                PaneDirection::Up | PaneDirection::Down => {
                    current_center_y.abs_diff(center_y)
                }
            };

            if distance < best_distance {
                best_distance = distance;
                best_pane = Some(idx);
            }
        }

        if let Some(new_pane) = best_pane {
            self.save_pane_state();
            self.active_pane = new_pane;
            self.load_pane_state();
            self.set_status(format!("Pane {}/{}", self.active_pane + 1, self.panes.len()));
        } else if direction == PaneDirection::Left && self.explorer.visible {
            // If moving left and no pane found, focus the explorer
            self.focus_explorer();
        }
    }

    /// Open a file in the editor (replaces current buffer or adds new one)
    pub fn open_file(&mut self, path: std::path::PathBuf) -> anyhow::Result<()> {
        // Check if file is already open in an existing buffer
        let canonical_path = path.canonicalize().ok();
        if let Some(existing_idx) = self.buffers.iter().position(|b| {
            b.path.as_ref().and_then(|p| p.canonicalize().ok()) == canonical_path
        }) {
            // File already open, switch to that buffer
            self.current_buffer_idx = existing_idx;
            self.cursor = Cursor::default();
            self.viewport_offset = 0;
            // Sync active pane state
            if self.active_pane < self.panes.len() {
                self.panes[self.active_pane].buffer_idx = existing_idx;
                self.panes[self.active_pane].cursor = self.cursor;
                self.panes[self.active_pane].viewport_offset = self.viewport_offset;
            }
            // Re-parse syntax for this buffer
            self.syntax.set_language_from_path(&path);
            self.parse_current_buffer();
            // Update git diff for this buffer
            self.update_git_diff();
            return Ok(());
        }

        // Set up syntax highlighting based on file extension
        self.syntax.set_language_from_path(&path);

        let new_buffer = Buffer::from_file(path)?;

        // If current buffer is empty and unnamed, replace it; otherwise add new buffer
        if self.buffers[self.current_buffer_idx].is_empty()
            && self.buffers[self.current_buffer_idx].path.is_none()
        {
            self.buffers[self.current_buffer_idx] = new_buffer;
            // Update active pane's buffer_idx (it's already pointing to current_buffer_idx)
        } else {
            self.buffers.push(new_buffer);
            self.current_buffer_idx = self.buffers.len() - 1;
            // Update active pane to point to the new buffer
            if self.active_pane < self.panes.len() {
                self.panes[self.active_pane].buffer_idx = self.current_buffer_idx;
            }
        }

        self.cursor = Cursor::default();
        self.viewport_offset = 0;
        self.undo_stack.clear();

        // Sync active pane's cursor and viewport
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].cursor = self.cursor;
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
        }

        // Parse the buffer for syntax highlighting
        self.parse_current_buffer();

        // Update git diff for the newly opened file
        self.update_git_diff();

        Ok(())
    }

    /// Close the current buffer
    pub fn close_current_buffer(&mut self) {
        if self.buffers.len() <= 1 {
            // If it's the last buffer, just create a new empty one
            self.buffers[0] = Buffer::new();
            self.cursor = Cursor::default();
            self.viewport_offset = 0;
            self.undo_stack.clear();
        } else {
            // Remove the current buffer
            self.buffers.remove(self.current_buffer_idx);

            // Adjust current_buffer_idx if needed
            if self.current_buffer_idx >= self.buffers.len() {
                self.current_buffer_idx = self.buffers.len() - 1;
            }

            // Update pane to point to valid buffer
            if self.active_pane < self.panes.len() {
                self.panes[self.active_pane].buffer_idx = self.current_buffer_idx;
            }

            // Reset cursor state
            self.cursor = Cursor::default();
            self.viewport_offset = 0;
            self.undo_stack.clear();
        }

        // Sync pane state
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].cursor = self.cursor;
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
        }
    }

    /// Set the path of the current buffer (for rename operations)
    pub fn set_buffer_path(&mut self, path: std::path::PathBuf) {
        self.buffers[self.current_buffer_idx].path = Some(path.clone());
        // Update syntax highlighting for new filename
        self.syntax.set_language_from_path(&path);
        self.parse_current_buffer();
    }

    /// Set terminal size
    pub fn set_size(&mut self, width: u16, height: u16) {
        self.term_width = width;
        self.term_height = height;
        self.update_pane_rects();
    }

    /// Get the number of rows available for text (excluding status line)
    pub fn text_rows(&self) -> usize {
        self.term_height.saturating_sub(2) as usize // 1 for status, 1 for command line
    }

    /// Update pane rects based on current layout
    /// For now, uses simple even splits - horizontal for 2 panes
    pub fn update_pane_rects(&mut self) {
        let text_height = self.text_rows() as u16;
        let num_panes = self.panes.len() as u16;

        if num_panes == 0 {
            return;
        }

        // Account for explorer sidebar width
        let explorer_offset = if self.explorer.visible {
            self.explorer.width + 1 // +1 for separator
        } else {
            0
        };

        let available_width = self.term_width.saturating_sub(explorer_offset);

        match self.split_layout {
            SplitLayout::Vertical => {
                // Side-by-side panes
                let pane_width = available_width / num_panes;
                let remainder = available_width % num_panes;

                let mut x = explorer_offset;
                for (i, pane) in self.panes.iter_mut().enumerate() {
                    // Add remainder to last pane
                    let w = if i as u16 == num_panes - 1 {
                        pane_width + remainder
                    } else {
                        pane_width
                    };
                    pane.rect = Rect::new(x, 0, w, text_height);
                    x += w;
                }
            }
            SplitLayout::Horizontal => {
                // Stacked panes
                let pane_height = text_height / num_panes;
                let remainder = text_height % num_panes;

                let mut y = 0u16;
                for (i, pane) in self.panes.iter_mut().enumerate() {
                    let h = if i as u16 == num_panes - 1 {
                        pane_height + remainder
                    } else {
                        pane_height
                    };
                    pane.rect = Rect::new(explorer_offset, y, available_width, h);
                    y += h;
                }
            }
        }
    }

    /// Clamp cursor to valid buffer positions
    pub fn clamp_cursor(&mut self) {
        // Clamp line
        let max_line = self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1);
        if self.cursor.line > max_line {
            self.cursor.line = max_line;
        }

        // Clamp column to line length
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        let max_col = if self.mode == Mode::Insert {
            line_len // In insert mode, can be at end of line
        } else {
            line_len.saturating_sub(1) // In normal mode, on last char
        };

        if self.cursor.col > max_col && line_len > 0 {
            self.cursor.col = max_col;
        } else if line_len == 0 {
            self.cursor.col = 0;
        }
    }

    /// Ensure cursor is visible by adjusting viewport
    pub fn scroll_to_cursor(&mut self) {
        let text_rows = self.text_rows();
        let scroll_off = self.settings.editor.scroll_off.min(text_rows / 2);

        // Scroll up if cursor is above viewport (with scroll_off margin)
        if self.cursor.line < self.viewport_offset + scroll_off {
            self.viewport_offset = self.cursor.line.saturating_sub(scroll_off);
        }

        // Scroll down if cursor is below viewport (with scroll_off margin)
        if self.cursor.line + scroll_off >= self.viewport_offset + text_rows {
            self.viewport_offset = self.cursor.line + scroll_off + 1 - text_rows;
        }

        // Sync to active pane
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
            self.panes[self.active_pane].cursor = self.cursor;
        }
    }

    /// Get the text in a range (for yank/delete operations)
    pub fn get_range_text(&self, start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> String {
        let mut result = String::new();

        if start_line == end_line {
            // Same line
            if let Some(line) = self.buffers[self.current_buffer_idx].line(start_line) {
                let start = start_col.min(line.len_chars());
                let end = (end_col + 1).min(line.len_chars());
                if start < end {
                    for ch in line.chars().skip(start).take(end - start) {
                        result.push(ch);
                    }
                }
            }
        } else {
            // Multiple lines
            for l in start_line..=end_line {
                if let Some(line) = self.buffers[self.current_buffer_idx].line(l) {
                    if l == start_line {
                        for ch in line.chars().skip(start_col) {
                            result.push(ch);
                        }
                    } else if l == end_line {
                        for ch in line.chars().take(end_col + 1) {
                            result.push(ch);
                        }
                    } else {
                        for ch in line.chars() {
                            result.push(ch);
                        }
                    }
                }
            }
        }

        result
    }

    /// Get full lines as text (for line-wise operations)
    pub fn get_lines_text(&self, start_line: usize, end_line: usize) -> String {
        let mut result = String::new();
        for l in start_line..=end_line {
            if let Some(line) = self.buffers[self.current_buffer_idx].line(l) {
                for ch in line.chars() {
                    result.push(ch);
                }
            }
        }
        result
    }

    /// Delete a range of text and return it
    pub fn delete_range(&mut self, start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> String {
        let text = self.get_range_text(start_line, start_col, end_line, end_col);

        // Delete from buffer (end to start to preserve positions)
        self.buffers[self.current_buffer_idx].delete_range(start_line, start_col, end_line, end_col + 1);

        // Move cursor to start of deleted range
        self.cursor.line = start_line;
        self.cursor.col = start_col;
        self.clamp_cursor();
        self.scroll_to_cursor();

        text
    }

    /// Delete lines and return them
    pub fn delete_lines(&mut self, start_line: usize, count: usize) -> String {
        let end_line = (start_line + count - 1).min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
        let text = self.get_lines_text(start_line, end_line);

        // Delete from start of first line to end of last line (including newline)
        let end_col = self.buffers[self.current_buffer_idx].line_len_including_newline(end_line);
        self.buffers[self.current_buffer_idx].delete_range(start_line, 0, end_line, end_col);

        // Position cursor
        self.cursor.line = start_line.min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
        self.cursor.col = 0;
        self.clamp_cursor();
        self.scroll_to_cursor();

        text
    }

    /// Delete from cursor to motion target
    pub fn delete_motion(&mut self, motion: Motion, count: usize, register: Option<char>) {
        if let Some((start_line, start_col, end_line, end_col)) = self.motion_range(motion, count) {
            let text = self.get_range_text(start_line, start_col, end_line, end_col);

            // Record for undo
            self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
            self.undo_stack.record_change(Change::delete(
                start_line,
                start_col,
                text.clone(),
            ));

            let deleted = self.delete_range(start_line, start_col, end_line, end_col);

            self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);

            let is_small = !deleted.contains('\n');
            self.registers.delete(register, RegisterContent::Chars(deleted), is_small);
        }
    }

    /// Delete count lines (dd operation)
    pub fn delete_line(&mut self, count: usize, register: Option<char>) {
        let start_line = self.cursor.line;
        let end_line = (start_line + count - 1).min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
        let text = self.get_lines_text(start_line, end_line);

        // Record for undo
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
        self.undo_stack.record_change(Change::delete(
            start_line,
            0,
            text.clone(),
        ));

        let deleted = self.delete_lines(self.cursor.line, count);

        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);

        self.registers.delete(register, RegisterContent::Lines(deleted), false);
    }

    /// Yank from cursor to motion target
    pub fn yank_motion(&mut self, motion: Motion, count: usize, register: Option<char>) {
        if let Some((start_line, start_col, end_line, end_col)) = self.motion_range(motion, count) {
            let text = self.get_range_text(start_line, start_col, end_line, end_col);
            self.registers.yank(register, RegisterContent::Chars(text));
            self.set_status("Yanked");
        }
    }

    /// Yank count lines (yy operation)
    pub fn yank_line(&mut self, count: usize, register: Option<char>) {
        let end_line = (self.cursor.line + count - 1).min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
        let text = self.get_lines_text(self.cursor.line, end_line);
        self.registers.yank(register, RegisterContent::Lines(text));

        let msg = if count == 1 {
            "1 line yanked".to_string()
        } else {
            format!("{} lines yanked", count)
        };
        self.set_status(msg);
    }

    /// Change from cursor to motion target (delete + insert mode)
    pub fn change_motion(&mut self, motion: Motion, count: usize, register: Option<char>) {
        if let Some((start_line, start_col, end_line, end_col)) = self.motion_range(motion, count) {
            let text = self.get_range_text(start_line, start_col, end_line, end_col);

            // Begin undo group (will include the delete and subsequent inserts)
            self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
            self.undo_stack.record_change(Change::delete(
                start_line,
                start_col,
                text.clone(),
            ));

            let deleted = self.delete_range(start_line, start_col, end_line, end_col);

            let is_small = !deleted.contains('\n');
            self.registers.delete(register, RegisterContent::Chars(deleted), is_small);

            // Enter insert mode (don't start new undo group, reuse the one from change)
            self.mode = Mode::Insert;
        }
    }

    /// Change count lines (cc operation)
    pub fn change_line(&mut self, count: usize, register: Option<char>) {
        let end_line = (self.cursor.line + count - 1).min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));

        // For cc, we delete the content but keep the line structure
        // Get the text that will be deleted for undo
        let text = self.get_lines_text(self.cursor.line, end_line);

        // Begin undo group (will include the delete and subsequent inserts)
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
        self.undo_stack.record_change(Change::delete(
            self.cursor.line,
            0,
            text.clone(),
        ));

        self.registers.delete(register, RegisterContent::Lines(text), false);

        // Delete all lines except keep one empty line
        for _ in 0..count.saturating_sub(1) {
            if self.cursor.line < self.buffers[self.current_buffer_idx].len_lines() - 1 {
                self.delete_lines(self.cursor.line + 1, 1);
            }
        }

        // Clear current line content (keep the newline)
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        if line_len > 0 {
            self.buffers[self.current_buffer_idx].delete_range(self.cursor.line, 0, self.cursor.line, line_len);
        }

        self.cursor.col = 0;
        // Enter insert mode (don't start new undo group, reuse the one from change)
        self.mode = Mode::Insert;
    }

    /// Paste after cursor from a register
    pub fn paste_after(&mut self, register: Option<char>) {
        if let Some(content) = self.registers.get_content(register) {
            self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

            match content {
                RegisterContent::Lines(text) => {
                    // Paste on new line below
                    let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);

                    // Record the insertion for undo
                    let trimmed = text.trim_end_matches('\n');
                    let insert_text = format!("\n{}", trimmed);
                    self.undo_stack.record_change(Change::insert(
                        self.cursor.line,
                        line_len,
                        insert_text,
                    ));

                    self.buffers[self.current_buffer_idx].insert_char(self.cursor.line, line_len, '\n');
                    self.cursor.line += 1;
                    self.cursor.col = 0;

                    // Insert the lines (without trailing newline if present)
                    self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, 0, trimmed);

                    self.scroll_to_cursor();
                }
                RegisterContent::Chars(text) => {
                    // Paste after cursor
                    let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
                    let insert_col = if line_len > 0 {
                        (self.cursor.col + 1).min(line_len)
                    } else {
                        0
                    };

                    // Record the insertion for undo
                    self.undo_stack.record_change(Change::insert(
                        self.cursor.line,
                        insert_col,
                        text.clone(),
                    ));

                    self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, insert_col, &text);
                    self.cursor.col = insert_col + text.len().saturating_sub(1);
                    self.clamp_cursor();
                }
            }

            self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
        }
    }

    /// Paste before cursor from a register
    pub fn paste_before(&mut self, register: Option<char>) {
        if let Some(content) = self.registers.get_content(register) {
            self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

            match content {
                RegisterContent::Lines(text) => {
                    // Paste on new line above
                    let insert_text = if text.ends_with('\n') {
                        text.clone()
                    } else {
                        format!("{}\n", text)
                    };

                    // Record the insertion for undo
                    self.undo_stack.record_change(Change::insert(
                        self.cursor.line,
                        0,
                        insert_text.clone(),
                    ));

                    self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, 0, &text);
                    if !text.ends_with('\n') {
                        self.buffers[self.current_buffer_idx].insert_char(self.cursor.line, text.len(), '\n');
                    }
                    self.cursor.col = 0;
                    self.scroll_to_cursor();
                }
                RegisterContent::Chars(text) => {
                    // Record the insertion for undo
                    self.undo_stack.record_change(Change::insert(
                        self.cursor.line,
                        self.cursor.col,
                        text.clone(),
                    ));

                    // Paste before cursor
                    self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, self.cursor.col, &text);
                    self.cursor.col = self.cursor.col + text.len().saturating_sub(1);
                    self.clamp_cursor();
                }
            }

            self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
        }
    }

    /// Enter insert mode
    pub fn enter_insert_mode(&mut self) {
        self.mode = Mode::Insert;
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
    }

    /// Enter insert mode after cursor
    pub fn enter_insert_mode_append(&mut self) {
        self.mode = Mode::Insert;
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        if line_len > 0 {
            self.cursor.col = (self.cursor.col + 1).min(line_len);
        }
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
    }

    /// Enter insert mode at end of line
    pub fn enter_insert_mode_end(&mut self) {
        self.mode = Mode::Insert;
        self.cursor.col = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
    }

    /// Enter insert mode at start of line (first non-blank)
    pub fn enter_insert_mode_start(&mut self) {
        self.mode = Mode::Insert;
        // Find first non-blank character
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        for col in 0..line_len {
            if let Some(ch) = self.buffers[self.current_buffer_idx].char_at(self.cursor.line, col) {
                if !ch.is_whitespace() {
                    self.cursor.col = col;
                    self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
                    return;
                }
            }
        }
        self.cursor.col = 0;
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
    }

    /// Enter replace mode
    pub fn enter_replace_mode(&mut self) {
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
        self.mode = Mode::Replace;
    }

    /// Replace character at cursor position (for replace mode)
    pub fn replace_mode_char(&mut self, ch: char) {
        let buffer = &self.buffers[self.current_buffer_idx];
        let line_len = buffer.line_len(self.cursor.line);

        if ch == '\n' {
            // Newline exits replace mode and goes to next line
            self.enter_normal_mode();
            return;
        }

        if self.cursor.col < line_len {
            // Replace existing character
            if let Some(old_char) = buffer.char_at(self.cursor.line, self.cursor.col) {
                self.undo_stack.record_change(Change::delete(
                    self.cursor.line,
                    self.cursor.col,
                    old_char.to_string(),
                ));
            }
            self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, self.cursor.col);
        }

        // Insert the new character
        self.undo_stack.record_change(Change::insert(
            self.cursor.line,
            self.cursor.col,
            ch.to_string(),
        ));
        self.buffers[self.current_buffer_idx].insert_char(self.cursor.line, self.cursor.col, ch);
        self.cursor.col += 1;

        self.scroll_to_cursor();
    }

    /// Enter rename prompt mode with the word under cursor
    pub fn enter_rename_prompt(&mut self) {
        let word = self.get_word_under_cursor().unwrap_or_default();
        self.rename_original = word.clone();
        self.rename_input = word;
        self.mode = Mode::RenamePrompt;
    }

    /// Exit rename prompt mode and trigger rename if confirmed
    pub fn confirm_rename(&mut self) {
        if !self.rename_input.is_empty() && self.rename_input != self.rename_original {
            self.pending_lsp_action = Some(LspAction::RenameSymbol(self.rename_input.clone()));
        } else if self.rename_input.is_empty() {
            self.set_status("Rename cancelled: empty name");
        } else {
            self.set_status("Rename cancelled: same name");
        }
        self.rename_input.clear();
        self.rename_original.clear();
        self.mode = Mode::Normal;
    }

    /// Cancel rename prompt and return to normal mode
    pub fn cancel_rename(&mut self) {
        self.rename_input.clear();
        self.rename_original.clear();
        self.mode = Mode::Normal;
    }

    /// Handle character input in rename prompt
    pub fn rename_input_char(&mut self, ch: char) {
        self.rename_input.push(ch);
    }

    /// Handle backspace in rename prompt
    pub fn rename_input_backspace(&mut self) {
        self.rename_input.pop();
    }

    /// Clear rename input
    pub fn rename_input_clear(&mut self) {
        self.rename_input.clear();
    }

    /// Exit to normal mode
    pub fn enter_normal_mode(&mut self) {
        // End any current undo group
        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);

        // Hide any active popups
        self.completion.hide();
        self.signature_help = None;
        self.show_diagnostic_float = false;

        self.mode = Mode::Normal;
        // In normal mode, cursor can't be past last character
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        }
        self.clamp_cursor();
    }

    /// Insert a character at cursor position
    pub fn insert_char(&mut self, ch: char) {
        if ch == '\n' && self.settings.editor.auto_indent {
            self.insert_newline_with_indent();
        } else if ch == '}' && self.settings.editor.auto_indent {
            self.insert_closing_brace();
        } else {
            // Standard character insertion
            self.undo_stack.record_change(Change::insert(
                self.cursor.line,
                self.cursor.col,
                ch.to_string(),
            ));

            if ch == '\n' {
                self.buffers[self.current_buffer_idx].insert_char(self.cursor.line, self.cursor.col, '\n');
                self.cursor.line += 1;
                self.cursor.col = 0;
            } else {
                self.buffers[self.current_buffer_idx].insert_char(self.cursor.line, self.cursor.col, ch);
                self.cursor.col += 1;
            }
        }
        self.scroll_to_cursor();
    }

    /// Insert newline with smart indentation
    fn insert_newline_with_indent(&mut self) {
        let buffer = &self.buffers[self.current_buffer_idx];
        let base_indent = buffer.get_line_indent(self.cursor.line);
        let ends_with_brace = buffer.line_ends_with(self.cursor.line, '{');
        let tab_width = self.settings.editor.tab_width;

        // Calculate the full indent for the new line
        let mut indent = base_indent.clone();
        if ends_with_brace {
            // Add one level of indentation after {
            indent.push_str(&" ".repeat(tab_width));
        }

        // Record the full insertion for undo (newline + indent)
        let insert_text = format!("\n{}", indent);
        self.undo_stack.record_change(Change::insert(
            self.cursor.line,
            self.cursor.col,
            insert_text.clone(),
        ));

        // Insert newline and indent
        self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, self.cursor.col, &insert_text);

        // Move cursor to end of indentation on new line
        self.cursor.line += 1;
        self.cursor.col = indent.len();
    }

    /// Insert closing brace with auto-dedent
    fn insert_closing_brace(&mut self) {
        let tab_width = self.settings.editor.tab_width;
        let should_dedent = self.should_dedent_for_brace();

        if should_dedent && self.cursor.col >= tab_width {
            // Delete one level of indent before inserting }
            let delete_start = self.cursor.col - tab_width;

            // Record the deletion for undo
            let deleted_text = " ".repeat(tab_width);
            self.undo_stack.record_change(Change::delete(
                self.cursor.line,
                delete_start,
                deleted_text,
            ));

            // Delete the indent
            for _ in 0..tab_width {
                self.cursor.col -= 1;
                self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, self.cursor.col);
            }
        }

        // Record and insert the }
        self.undo_stack.record_change(Change::insert(
            self.cursor.line,
            self.cursor.col,
            "}".to_string(),
        ));
        self.buffers[self.current_buffer_idx].insert_char(self.cursor.line, self.cursor.col, '}');
        self.cursor.col += 1;
    }

    /// Check if cursor is preceded only by whitespace on current line
    fn should_dedent_for_brace(&self) -> bool {
        let buffer = &self.buffers[self.current_buffer_idx];
        let Some(line) = buffer.line(self.cursor.line) else {
            return false;
        };

        // Check if all characters before cursor are whitespace
        for (i, ch) in line.chars().enumerate() {
            if i >= self.cursor.col {
                break;
            }
            if ch != ' ' && ch != '\t' {
                return false;
            }
        }
        true
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char_before(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
            // Record the deleted character for undo
            let deleted = self.buffers[self.current_buffer_idx].get_char_str(self.cursor.line, self.cursor.col);
            self.undo_stack.record_change(Change::delete(
                self.cursor.line,
                self.cursor.col,
                deleted,
            ));
            self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, self.cursor.col);
        } else if self.cursor.line > 0 {
            // Join with previous line
            let prev_line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line - 1);
            self.cursor.line -= 1;
            self.cursor.col = prev_line_len;
            // Record the deleted newline for undo
            self.undo_stack.record_change(Change::delete(
                self.cursor.line,
                self.cursor.col,
                "\n".to_string(),
            ));
            // Delete the newline at end of previous line
            self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, self.cursor.col);
        }
        self.scroll_to_cursor();
    }

    /// Delete character at cursor (x in normal mode)
    pub fn delete_char_at(&mut self) {
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        if line_len > 0 {
            if let Some(ch) = self.buffers[self.current_buffer_idx].char_at(self.cursor.line, self.cursor.col) {
                // Record for undo (single operation = single undo group)
                self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
                self.undo_stack.record_change(Change::delete(
                    self.cursor.line,
                    self.cursor.col,
                    ch.to_string(),
                ));
                self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);

                self.registers.delete(None, RegisterContent::Chars(ch.to_string()), true);
            }
            self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, self.cursor.col);
            self.clamp_cursor();
        }
    }

    /// Delete character before cursor in normal mode (X)
    pub fn delete_char_before_normal(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
            if let Some(ch) = self.buffers[self.current_buffer_idx].char_at(self.cursor.line, self.cursor.col) {
                // Record for undo
                self.undo_stack.begin_undo_group(self.cursor.line + 1, self.cursor.col + 1);
                self.undo_stack.record_change(Change::delete(
                    self.cursor.line,
                    self.cursor.col,
                    ch.to_string(),
                ));
                self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);

                self.registers.delete(None, RegisterContent::Chars(ch.to_string()), true);
            }
            self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, self.cursor.col);
        }
    }

    /// Open a new line below and enter insert mode
    pub fn open_line_below(&mut self) {
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);

        // Calculate indent for new line
        let indent = if self.settings.editor.auto_indent {
            let buffer = &self.buffers[self.current_buffer_idx];
            let base_indent = buffer.get_line_indent(self.cursor.line);
            let ends_with_brace = buffer.line_ends_with(self.cursor.line, '{');
            let tab_width = self.settings.editor.tab_width;

            let mut indent = base_indent;
            if ends_with_brace {
                indent.push_str(&" ".repeat(tab_width));
            }
            indent
        } else {
            String::new()
        };

        // Start undo group and record the insertion
        let insert_text = format!("\n{}", indent);
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
        self.undo_stack.record_change(Change::insert(
            self.cursor.line,
            line_len,
            insert_text.clone(),
        ));

        self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, line_len, &insert_text);
        self.cursor.line += 1;
        self.cursor.col = indent.len();
        self.mode = Mode::Insert;
        self.scroll_to_cursor();
    }

    /// Open a new line above and enter insert mode
    pub fn open_line_above(&mut self) {
        // Calculate indent for new line (match current line's indent)
        let indent = if self.settings.editor.auto_indent {
            let buffer = &self.buffers[self.current_buffer_idx];
            buffer.get_line_indent(self.cursor.line)
        } else {
            String::new()
        };

        // Start undo group and record the insertion
        let insert_text = format!("{}\n", indent);
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
        self.undo_stack.record_change(Change::insert(
            self.cursor.line,
            0,
            insert_text.clone(),
        ));

        self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, 0, &insert_text);
        // Cursor stays on same line number (which is now the new line with indent)
        self.cursor.col = indent.len();
        self.mode = Mode::Insert;
        self.scroll_to_cursor();
    }

    /// Save the current buffer
    pub fn save(&mut self) -> anyhow::Result<()> {
        self.buffers[self.current_buffer_idx].save()?;
        self.status_message = Some(format!("\"{}\" written", self.buffers[self.current_buffer_idx].display_name()));
        // Update git diff after save (file now matches HEAD if no other changes)
        self.update_git_diff();
        Ok(())
    }

    /// Set a status message
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
    }

    /// Clear status message
    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    /// Enter command mode
    pub fn enter_command_mode(&mut self) {
        self.mode = Mode::Command;
        self.command_line.clear();
    }

    /// Exit command mode back to normal
    pub fn exit_command_mode(&mut self) {
        self.mode = Mode::Normal;
        self.command_line.clear();
    }

    /// Go to a specific line number (1-indexed)
    pub fn goto_line(&mut self, line: usize) {
        let target = line.saturating_sub(1).min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
        self.cursor.line = target;
        self.cursor.col = 0;
        self.clamp_cursor();
        self.scroll_to_cursor();
    }

    /// Record current position in jump list (call before jumping)
    pub fn record_jump(&mut self) {
        let path = self.buffer().path.clone();
        self.jump_list.record(path, self.cursor.line, self.cursor.col);
    }

    /// Go back in jump list (Ctrl+o)
    pub fn jump_back(&mut self) -> bool {
        let current_path = self.buffer().path.clone();
        let current_line = self.cursor.line;
        let current_col = self.cursor.col;

        if let Some(loc) = self.jump_list.go_back(current_path, current_line, current_col).cloned() {
            // Check if we need to switch files
            if loc.path != self.buffer().path {
                if let Some(path) = loc.path {
                    if self.open_file(path).is_err() {
                        return false;
                    }
                }
            }
            self.cursor.line = loc.line;
            self.cursor.col = loc.col;
            self.clamp_cursor();
            self.scroll_to_cursor();
            true
        } else {
            false
        }
    }

    /// Go forward in jump list (Ctrl+i)
    pub fn jump_forward(&mut self) -> bool {
        if let Some(loc) = self.jump_list.go_forward().cloned() {
            // Check if we need to switch files
            if loc.path != self.buffer().path {
                if let Some(path) = loc.path {
                    if self.open_file(path).is_err() {
                        return false;
                    }
                }
            }
            self.cursor.line = loc.line;
            self.cursor.col = loc.col;
            self.clamp_cursor();
            self.scroll_to_cursor();
            true
        } else {
            false
        }
    }

    /// Save to a specific file
    pub fn save_as(&mut self, path: std::path::PathBuf) -> anyhow::Result<()> {
        self.buffers[self.current_buffer_idx].path = Some(path);
        self.save()
    }

    /// Reload the current file
    pub fn reload(&mut self) -> anyhow::Result<()> {
        if let Some(path) = self.buffers[self.current_buffer_idx].path.clone() {
            self.buffers[self.current_buffer_idx] = Buffer::from_file(path)?;
            self.cursor = Cursor::default();
            self.viewport_offset = 0;
            self.undo_stack.clear();
            self.parse_current_buffer();
            self.set_status("File reloaded");
            Ok(())
        } else {
            anyhow::bail!("No file to reload")
        }
    }

    /// Check if buffer has unsaved changes
    pub fn has_unsaved_changes(&self) -> bool {
        self.buffers[self.current_buffer_idx].dirty
    }

    /// Undo the last change
    pub fn undo(&mut self) {
        if let Some(entry) = self.undo_stack.pop_undo() {
            // Apply changes in reverse order
            for change in entry.changes.iter().rev() {
                // To undo: we need the inverse operation
                // If it was an insert (old_text empty, new_text has content), we delete new_text
                // If it was a delete (old_text has content, new_text empty), we insert old_text
                self.buffers[self.current_buffer_idx].apply_change(
                    change.start_line,
                    change.start_col,
                    &change.new_text,  // Remove what was inserted
                    &change.old_text,  // Restore what was deleted
                );
            }

            // Restore cursor position
            self.cursor.line = entry.cursor_before.0;
            self.cursor.col = entry.cursor_before.1;
            self.clamp_cursor();
            self.scroll_to_cursor();

            let count = self.undo_stack.undo_count();
            self.set_status(format!("Undo: {} change(s) remaining", count));
        } else {
            self.set_status("Already at oldest change");
        }
    }

    /// Redo the last undone change
    pub fn redo(&mut self) {
        if let Some(entry) = self.undo_stack.pop_redo() {
            // Apply changes in forward order
            for change in entry.changes.iter() {
                // To redo: apply the original change
                self.buffers[self.current_buffer_idx].apply_change(
                    change.start_line,
                    change.start_col,
                    &change.old_text,  // Remove old text
                    &change.new_text,  // Insert new text
                );
            }

            // Restore cursor position
            self.cursor.line = entry.cursor_after.0;
            self.cursor.col = entry.cursor_after.1;
            self.clamp_cursor();
            self.scroll_to_cursor();

            let count = self.undo_stack.redo_count();
            self.set_status(format!("Redo: {} change(s) remaining", count));
        } else {
            self.set_status("Already at newest change");
        }
    }

    /// Enter search mode (forward search)
    pub fn enter_search_forward(&mut self) {
        self.mode = Mode::Search;
        self.search.start(SearchDirection::Forward);
    }

    /// Enter search mode (backward search)
    pub fn enter_search_backward(&mut self) {
        self.mode = Mode::Search;
        self.search.start(SearchDirection::Backward);
    }

    /// Exit search mode
    pub fn exit_search_mode(&mut self) {
        self.mode = Mode::Normal;
        self.search.clear();
        self.search_matches.clear();
    }

    /// Clear search highlights (called on non-search movement)
    pub fn clear_search_highlights(&mut self) {
        self.search_matches.clear();
    }

    /// Update incremental search matches based on current search input
    /// This finds all matches in the buffer and highlights them while typing
    pub fn update_incremental_search(&mut self) {
        self.search_matches.clear();

        let pattern = &self.search.input;
        if pattern.is_empty() {
            return;
        }

        let total_lines = self.buffers[self.current_buffer_idx].len_lines();
        if total_lines == 0 {
            return;
        }

        // Find all matches in the buffer
        for line_idx in 0..total_lines {
            if let Some(line) = self.buffers[self.current_buffer_idx].line(line_idx) {
                let line_str: String = line.chars().collect();
                let pattern_len = pattern.chars().count();

                // Find all occurrences in this line
                let mut search_from = 0;
                while search_from < line_str.len() {
                    if let Some(byte_pos) = line_str[search_from..].find(pattern) {
                        let match_byte_start = search_from + byte_pos;
                        let match_byte_end = match_byte_start + pattern.len();

                        // Convert byte positions to char positions
                        let start_col = Self::byte_to_char_idx(&line_str, match_byte_start);
                        let end_col = start_col + pattern_len;

                        self.search_matches.push((line_idx, start_col, end_col));

                        // Move past this match to find more
                        search_from = match_byte_end;
                    } else {
                        break;
                    }
                }
            }
        }

        // Jump to first match in search direction (preview)
        if !self.search_matches.is_empty() {
            let cursor_line = self.cursor.line;
            let cursor_col = self.cursor.col;

            let target = match self.search.direction {
                SearchDirection::Forward => {
                    // Find first match at or after cursor
                    self.search_matches.iter().find(|(line, col, _)| {
                        *line > cursor_line || (*line == cursor_line && *col > cursor_col)
                    }).or_else(|| self.search_matches.first())
                }
                SearchDirection::Backward => {
                    // Find last match before cursor
                    self.search_matches.iter().rev().find(|(line, col, _)| {
                        *line < cursor_line || (*line == cursor_line && *col < cursor_col)
                    }).or_else(|| self.search_matches.last())
                }
            };

            if let Some(&(line, col, _)) = target {
                self.cursor.line = line;
                self.cursor.col = col;
                self.scroll_to_cursor();
            }
        }
    }

    /// Execute the current search
    pub fn execute_search(&mut self) {
        let direction = self.search.direction;
        if let Some(pattern) = self.search.execute() {
            self.mode = Mode::Normal;
            if !self.do_search(&pattern, direction, true) {
                self.set_status(format!("Pattern not found: {}", pattern));
            }
        } else {
            self.mode = Mode::Normal;
            self.set_status("No previous search pattern");
        }
    }

    /// Search for next occurrence (n)
    pub fn search_next(&mut self) {
        if let Some(pattern) = self.search.last_pattern.clone() {
            let direction = self.search.last_direction;
            // Update search highlights
            self.update_search_matches_from_pattern(&pattern);
            if !self.do_search(&pattern, direction, true) {
                self.set_status(format!("Pattern not found: {}", pattern));
            }
        } else {
            self.set_status("No previous search pattern");
        }
    }

    /// Search for previous occurrence (N)
    pub fn search_prev(&mut self) {
        if let Some(pattern) = self.search.last_pattern.clone() {
            // Reverse the direction
            let direction = match self.search.last_direction {
                SearchDirection::Forward => SearchDirection::Backward,
                SearchDirection::Backward => SearchDirection::Forward,
            };
            // Update search highlights
            self.update_search_matches_from_pattern(&pattern);
            if !self.do_search(&pattern, direction, true) {
                self.set_status(format!("Pattern not found: {}", pattern));
            }
        } else {
            self.set_status("No previous search pattern");
        }
    }

    /// Update search matches from a pattern string (used for n/N/*/#)
    fn update_search_matches_from_pattern(&mut self, pattern: &str) {
        self.search_matches.clear();

        if pattern.is_empty() {
            return;
        }

        let total_lines = self.buffers[self.current_buffer_idx].len_lines();
        if total_lines == 0 {
            return;
        }

        let pattern_len = pattern.chars().count();

        // Find all matches in the buffer
        for line_idx in 0..total_lines {
            if let Some(line) = self.buffers[self.current_buffer_idx].line(line_idx) {
                let line_str: String = line.chars().collect();

                // Find all occurrences in this line
                let mut search_from = 0;
                while search_from < line_str.len() {
                    if let Some(byte_pos) = line_str[search_from..].find(pattern) {
                        let match_byte_start = search_from + byte_pos;
                        let match_byte_end = match_byte_start + pattern.len();

                        // Convert byte positions to char positions
                        let start_col = Self::byte_to_char_idx(&line_str, match_byte_start);
                        let end_col = start_col + pattern_len;

                        self.search_matches.push((line_idx, start_col, end_col));

                        // Move past this match to find more
                        search_from = match_byte_end;
                    } else {
                        break;
                    }
                }
            }
        }
    }

    /// Search for word under cursor forward (*)
    pub fn search_word_forward(&mut self) {
        if let Some(word) = self.get_word_under_cursor() {
            // Set as search pattern
            self.search.last_pattern = Some(word.clone());
            self.search.last_direction = SearchDirection::Forward;
            // Update search highlights
            self.update_search_matches_from_pattern(&word);
            // Perform search
            if !self.do_search(&word, SearchDirection::Forward, true) {
                self.set_status(format!("Pattern not found: {}", word));
            }
        } else {
            self.set_status("No word under cursor");
        }
    }

    /// Search for word under cursor backward (#)
    pub fn search_word_backward(&mut self) {
        if let Some(word) = self.get_word_under_cursor() {
            // Set as search pattern
            self.search.last_pattern = Some(word.clone());
            self.search.last_direction = SearchDirection::Backward;
            // Update search highlights
            self.update_search_matches_from_pattern(&word);
            // Perform search
            if !self.do_search(&word, SearchDirection::Backward, true) {
                self.set_status(format!("Pattern not found: {}", word));
            }
        } else {
            self.set_status("No word under cursor");
        }
    }

    /// Get the word under the cursor
    fn get_word_under_cursor(&self) -> Option<String> {
        let line = self.buffers[self.current_buffer_idx].line(self.cursor.line)?;
        let line_str: String = line.chars().collect();
        let col = self.cursor.col;

        // Check if cursor is on a word character
        let chars: Vec<char> = line_str.chars().collect();
        if col >= chars.len() {
            return None;
        }
        if !Self::is_word_char(chars[col]) {
            return None;
        }

        // Find word start (go backward)
        let mut start = col;
        while start > 0 && Self::is_word_char(chars[start - 1]) {
            start -= 1;
        }

        // Find word end (go forward)
        let mut end = col;
        while end < chars.len() && Self::is_word_char(chars[end]) {
            end += 1;
        }

        if start < end {
            Some(chars[start..end].iter().collect())
        } else {
            None
        }
    }

    /// Check if a character is a word character (alphanumeric or underscore)
    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    /// Search and replace text
    /// Returns the number of replacements made
    pub fn substitute(&mut self, pattern: &str, replacement: &str, entire_file: bool, global: bool) -> usize {
        if pattern.is_empty() {
            return 0;
        }

        // Begin undo group for all replacements
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

        let mut total_replacements = 0;
        let pattern_len = pattern.len();

        // Determine line range
        let (start_line, end_line) = if entire_file {
            (0, self.buffers[self.current_buffer_idx].len_lines())
        } else {
            (self.cursor.line, self.cursor.line + 1)
        };

        for line_idx in start_line..end_line {
            if let Some(line) = self.buffers[self.current_buffer_idx].line(line_idx) {
                let line_str: String = line.chars().collect();
                let mut new_line = String::new();
                let mut last_end = 0;
                let mut search_from = 0;
                let mut line_replacements = 0;

                // Find all matches in this line
                while search_from < line_str.len() {
                    if let Some(byte_pos) = line_str[search_from..].find(pattern) {
                        let match_start = search_from + byte_pos;
                        let match_end = match_start + pattern_len;

                        // Copy text before match
                        new_line.push_str(&line_str[last_end..match_start]);
                        // Add replacement
                        new_line.push_str(replacement);

                        last_end = match_end;
                        search_from = match_end;
                        line_replacements += 1;
                        total_replacements += 1;

                        // If not global, only replace first occurrence on line
                        if !global {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                // If we made replacements, update the line
                if line_replacements > 0 {
                    // Copy remaining text after last match
                    new_line.push_str(&line_str[last_end..]);

                    // Record undo
                    self.undo_stack.record_change(crate::editor::undo::Change::replace_line(
                        line_idx,
                        line_str.clone(),
                        new_line.clone(),
                    ));

                    // Replace the line in buffer
                    self.buffers[self.current_buffer_idx].replace_line(line_idx, &new_line);
                }
            }
        }

        // End undo group
        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);

        // Mark buffer as modified if changes were made
        if total_replacements > 0 {
            self.buffers[self.current_buffer_idx].mark_modified();
        }

        total_replacements
    }

    /// Perform the actual search
    /// Returns true if found, false otherwise
    fn do_search(&mut self, pattern: &str, direction: SearchDirection, wrap: bool) -> bool {
        let total_lines = self.buffers[self.current_buffer_idx].len_lines();
        if total_lines == 0 || pattern.is_empty() {
            return false;
        }

        match direction {
            SearchDirection::Forward => {
                // Search from current position forward
                // First check rest of current line after cursor
                if let Some(line) = self.buffers[self.current_buffer_idx].line(self.cursor.line) {
                    let line_str: String = line.chars().collect();
                    let search_start = self.cursor.col + 1;
                    let search_start_byte = Self::char_to_byte_idx(&line_str, search_start);
                    if search_start_byte < line_str.len() {
                        if let Some(pos) = line_str[search_start_byte..].find(pattern) {
                            let byte_pos = search_start_byte + pos;
                            self.cursor.col = Self::byte_to_char_idx(&line_str, byte_pos);
                            self.scroll_to_cursor();
                            return true;
                        }
                    }
                }

                // Search subsequent lines
                for line_idx in (self.cursor.line + 1)..total_lines {
                    if let Some(line) = self.buffers[self.current_buffer_idx].line(line_idx) {
                        let line_str: String = line.chars().collect();
                        if let Some(pos) = line_str.find(pattern) {
                            self.cursor.line = line_idx;
                            self.cursor.col = Self::byte_to_char_idx(&line_str, pos);
                            self.scroll_to_cursor();
                            return true;
                        }
                    }
                }

                // Wrap around if enabled
                if wrap {
                    for line_idx in 0..=self.cursor.line {
                        if let Some(line) = self.buffers[self.current_buffer_idx].line(line_idx) {
                            let line_str: String = line.chars().collect();
                            let end_col = if line_idx == self.cursor.line {
                                self.cursor.col
                            } else {
                                line_str.chars().count()
                            };
                            let end_byte = Self::char_to_byte_idx(&line_str, end_col);
                            if let Some(pos) = line_str[..end_byte.min(line_str.len())].find(pattern) {
                                self.cursor.line = line_idx;
                                self.cursor.col = Self::byte_to_char_idx(&line_str, pos);
                                self.scroll_to_cursor();
                                self.set_status("search hit BOTTOM, continuing at TOP");
                                return true;
                            }
                        }
                    }
                }
            }
            SearchDirection::Backward => {
                // Search from current position backward
                // First check current line before cursor
                if let Some(line) = self.buffers[self.current_buffer_idx].line(self.cursor.line) {
                    let line_str: String = line.chars().collect();
                    if self.cursor.col > 0 {
                        let end_byte = Self::char_to_byte_idx(&line_str, self.cursor.col);
                        if let Some(pos) = line_str[..end_byte].rfind(pattern) {
                            self.cursor.col = Self::byte_to_char_idx(&line_str, pos);
                            self.scroll_to_cursor();
                            return true;
                        }
                    }
                }

                // Search previous lines
                for line_idx in (0..self.cursor.line).rev() {
                    if let Some(line) = self.buffers[self.current_buffer_idx].line(line_idx) {
                        let line_str: String = line.chars().collect();
                        if let Some(pos) = line_str.rfind(pattern) {
                            self.cursor.line = line_idx;
                            self.cursor.col = Self::byte_to_char_idx(&line_str, pos);
                            self.scroll_to_cursor();
                            return true;
                        }
                    }
                }

                // Wrap around if enabled
                if wrap {
                    for line_idx in (self.cursor.line..total_lines).rev() {
                        if let Some(line) = self.buffers[self.current_buffer_idx].line(line_idx) {
                            let line_str: String = line.chars().collect();
                            let start_col = if line_idx == self.cursor.line {
                                self.cursor.col + 1
                            } else {
                                0
                            };
                            let start_byte = Self::char_to_byte_idx(&line_str, start_col);
                            if start_byte < line_str.len() {
                                if let Some(pos) = line_str[start_byte..].rfind(pattern) {
                                    self.cursor.line = line_idx;
                                    self.cursor.col = Self::byte_to_char_idx(&line_str, start_byte + pos);
                                    self.scroll_to_cursor();
                                    self.set_status("search hit TOP, continuing at BOTTOM");
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }

        false
    }

    /// Convert a character index to a byte index in a string
    fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
        if char_idx == 0 {
            return 0;
        }
        s.char_indices()
            .nth(char_idx)
            .map(|(idx, _)| idx)
            .unwrap_or_else(|| s.len())
    }

    /// Convert a byte index to a character index in a string
    fn byte_to_char_idx(s: &str, byte_idx: usize) -> usize {
        s[..byte_idx.min(s.len())].chars().count()
    }

    /// Enter visual mode (character-wise)
    pub fn enter_visual_mode(&mut self) {
        self.mode = Mode::Visual;
        self.visual = VisualSelection::new(self.cursor.line, self.cursor.col);
    }

    /// Enter visual line mode
    pub fn enter_visual_line_mode(&mut self) {
        self.mode = Mode::VisualLine;
        self.visual = VisualSelection::new(self.cursor.line, self.cursor.col);
    }

    /// Enter visual block mode
    pub fn enter_visual_block_mode(&mut self) {
        self.mode = Mode::VisualBlock;
        self.visual = VisualSelection::new(self.cursor.line, self.cursor.col);
    }

    /// Exit visual mode
    pub fn exit_visual_mode(&mut self) {
        // Save the visual selection for gv command
        if self.mode.is_visual() {
            self.last_visual_selection = Some(LastVisualSelection {
                mode: self.mode,
                anchor_line: self.visual.anchor_line,
                anchor_col: self.visual.anchor_col,
                cursor_line: self.cursor.line,
                cursor_col: self.cursor.col,
            });
        }
        self.mode = Mode::Normal;
    }

    /// Reselect the last visual selection (gv command)
    pub fn reselect_visual(&mut self) {
        if let Some(ref sel) = self.last_visual_selection.clone() {
            self.visual = VisualSelection::new(sel.anchor_line, sel.anchor_col);
            self.cursor.line = sel.cursor_line;
            self.cursor.col = sel.cursor_col;
            self.mode = sel.mode;
            self.clamp_cursor();
        }
    }

    /// Toggle between visual and visual line mode
    pub fn toggle_visual_line(&mut self) {
        match self.mode {
            Mode::Visual | Mode::VisualBlock => self.mode = Mode::VisualLine,
            Mode::VisualLine => self.mode = Mode::Visual,
            _ => {}
        }
    }

    /// Toggle to visual block mode
    pub fn toggle_visual_block(&mut self) {
        match self.mode {
            Mode::Visual | Mode::VisualLine => self.mode = Mode::VisualBlock,
            Mode::VisualBlock => self.mode = Mode::Visual,
            _ => {}
        }
    }

    /// Get the current visual selection range
    /// Returns (start_line, start_col, end_line, end_col) inclusive
    pub fn get_visual_range(&self) -> (usize, usize, usize, usize) {
        match self.mode {
            Mode::Visual => {
                self.visual.get_range(self.cursor.line, self.cursor.col)
            }
            Mode::VisualLine => {
                let (start_line, end_line) = self.visual.get_line_range(self.cursor.line);
                let end_col = self.buffers[self.current_buffer_idx].line_len(end_line);
                (start_line, 0, end_line, end_col)
            }
            Mode::VisualBlock => {
                // For block mode, return (top, left, bottom, right)
                self.visual.get_block_range(self.cursor.line, self.cursor.col)
            }
            _ => (self.cursor.line, self.cursor.col, self.cursor.line, self.cursor.col),
        }
    }

    /// Delete visual selection
    pub fn visual_delete(&mut self) {
        let (start_line, start_col, end_line, end_col) = self.get_visual_range();

        match self.mode {
            Mode::VisualLine => {
                // Line-wise delete
                let count = end_line - start_line + 1;
                self.cursor.line = start_line;
                self.cursor.col = 0;
                let text = self.delete_lines(start_line, count);
                self.registers.delete(None, RegisterContent::Lines(text), false);
            }
            Mode::Visual => {
                // Character-wise delete
                let text = self.get_range_text(start_line, start_col, end_line, end_col);

                // Record for undo
                self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
                self.undo_stack.record_change(Change::delete(
                    start_line,
                    start_col,
                    text.clone(),
                ));

                self.buffers[self.current_buffer_idx].delete_range(start_line, start_col, end_line, end_col + 1);

                self.undo_stack.end_undo_group(start_line, start_col);

                self.cursor.line = start_line;
                self.cursor.col = start_col;
                self.clamp_cursor();

                let is_small = !text.contains('\n');
                self.registers.delete(None, RegisterContent::Chars(text), is_small);
            }
            Mode::VisualBlock => {
                // Block-wise delete
                let (top, left, bottom, right) = self.visual.get_block_range(self.cursor.line, self.cursor.col);

                self.undo_stack.begin_undo_group(top, left);

                // Collect deleted text from each line (for register)
                let mut deleted_lines: Vec<String> = Vec::new();

                // Delete from bottom to top to maintain line positions
                for line_idx in (top..=bottom).rev() {
                    let line_len = self.buffers[self.current_buffer_idx].line_len(line_idx);
                    if left < line_len {
                        let actual_right = right.min(line_len.saturating_sub(1));
                        if left <= actual_right {
                            // Get the text being deleted
                            let deleted: String = (left..=actual_right)
                                .filter_map(|c| self.buffers[self.current_buffer_idx].char_at(line_idx, c))
                                .collect();
                            deleted_lines.push(deleted.clone());

                            // Record the delete for undo
                            self.undo_stack.record_change(Change::delete(
                                line_idx,
                                left,
                                deleted,
                            ));

                            // Delete the range on this line
                            self.buffers[self.current_buffer_idx].delete_range(line_idx, left, line_idx, actual_right + 1);
                        }
                    }
                }

                self.undo_stack.end_undo_group(top, left);

                // Reverse to get top-to-bottom order
                deleted_lines.reverse();
                let block_text = deleted_lines.join("\n");

                self.cursor.line = top;
                self.cursor.col = left;
                self.clamp_cursor();

                self.registers.delete(None, RegisterContent::Chars(block_text), false);
            }
            _ => {}
        }

        self.mode = Mode::Normal;
        self.scroll_to_cursor();
    }

    /// Yank visual selection
    pub fn visual_yank(&mut self) {
        let (start_line, start_col, end_line, end_col) = self.get_visual_range();

        match self.mode {
            Mode::VisualLine => {
                // Line-wise yank
                let text = self.get_lines_text(start_line, end_line);
                self.registers.yank(None, RegisterContent::Lines(text));
                let count = end_line - start_line + 1;
                self.set_status(format!("{} line(s) yanked", count));
            }
            Mode::Visual => {
                // Character-wise yank
                let text = self.get_range_text(start_line, start_col, end_line, end_col);
                self.registers.yank(None, RegisterContent::Chars(text));
                self.set_status("Yanked");
            }
            Mode::VisualBlock => {
                // Block-wise yank
                let (top, left, bottom, right) = self.visual.get_block_range(self.cursor.line, self.cursor.col);

                // Collect text from each line in the block
                let mut yanked_lines: Vec<String> = Vec::new();
                for line_idx in top..=bottom {
                    let line_len = self.buffers[self.current_buffer_idx].line_len(line_idx);
                    if left < line_len {
                        let actual_right = right.min(line_len.saturating_sub(1));
                        if left <= actual_right {
                            let text: String = (left..=actual_right)
                                .filter_map(|c| self.buffers[self.current_buffer_idx].char_at(line_idx, c))
                                .collect();
                            yanked_lines.push(text);
                        } else {
                            yanked_lines.push(String::new());
                        }
                    } else {
                        yanked_lines.push(String::new());
                    }
                }

                let block_text = yanked_lines.join("\n");
                self.registers.yank(None, RegisterContent::Chars(block_text));
                let count = bottom - top + 1;
                self.set_status(format!("block of {} line(s) yanked", count));

                // For block yank, cursor goes to top-left
                self.cursor.line = top;
                self.cursor.col = left;
                self.clamp_cursor();
                self.mode = Mode::Normal;
                self.scroll_to_cursor();
                return;
            }
            _ => {}
        }

        // Move cursor to start of selection
        self.cursor.line = start_line;
        self.cursor.col = start_col;
        self.mode = Mode::Normal;
        self.scroll_to_cursor();
    }

    /// Change visual selection (delete + insert mode)
    pub fn visual_change(&mut self) {
        let (start_line, start_col, end_line, end_col) = self.get_visual_range();

        match self.mode {
            Mode::VisualLine => {
                // For line-wise change, delete lines but leave one empty line
                let text = self.get_lines_text(start_line, end_line);

                // Begin undo group
                self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
                self.undo_stack.record_change(Change::delete(
                    start_line,
                    0,
                    text.clone(),
                ));

                self.registers.delete(None, RegisterContent::Lines(text), false);

                // Delete all lines in range
                let count = end_line - start_line + 1;
                for _ in 0..count.saturating_sub(1) {
                    if start_line < self.buffers[self.current_buffer_idx].len_lines() - 1 {
                        self.delete_lines(start_line + 1, 1);
                    }
                }

                // Clear remaining line
                let line_len = self.buffers[self.current_buffer_idx].line_len(start_line);
                if line_len > 0 {
                    self.buffers[self.current_buffer_idx].delete_range(start_line, 0, start_line, line_len);
                }

                self.cursor.line = start_line;
                self.cursor.col = 0;
                self.mode = Mode::Insert;
            }
            Mode::Visual => {
                // Character-wise change
                let text = self.get_range_text(start_line, start_col, end_line, end_col);

                // Begin undo group
                self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
                self.undo_stack.record_change(Change::delete(
                    start_line,
                    start_col,
                    text.clone(),
                ));

                self.buffers[self.current_buffer_idx].delete_range(start_line, start_col, end_line, end_col + 1);

                self.cursor.line = start_line;
                self.cursor.col = start_col;

                let is_small = !text.contains('\n');
                self.registers.delete(None, RegisterContent::Chars(text), is_small);

                self.mode = Mode::Insert;
            }
            Mode::VisualBlock => {
                // Block-wise change: delete the block and enter insert mode
                let (top, left, bottom, right) = self.visual.get_block_range(self.cursor.line, self.cursor.col);

                self.undo_stack.begin_undo_group(top, left);

                // Collect deleted text from each line (for register)
                let mut deleted_lines: Vec<String> = Vec::new();

                // Delete from bottom to top to maintain line positions
                for line_idx in (top..=bottom).rev() {
                    let line_len = self.buffers[self.current_buffer_idx].line_len(line_idx);
                    if left < line_len {
                        let actual_right = right.min(line_len.saturating_sub(1));
                        if left <= actual_right {
                            // Get the text being deleted
                            let deleted: String = (left..=actual_right)
                                .filter_map(|c| self.buffers[self.current_buffer_idx].char_at(line_idx, c))
                                .collect();
                            deleted_lines.push(deleted.clone());

                            // Record the delete for undo
                            self.undo_stack.record_change(Change::delete(
                                line_idx,
                                left,
                                deleted,
                            ));

                            // Delete the range on this line
                            self.buffers[self.current_buffer_idx].delete_range(line_idx, left, line_idx, actual_right + 1);
                        }
                    }
                }

                // Note: We don't end undo group here - insert mode will continue it
                // Reverse to get top-to-bottom order
                deleted_lines.reverse();
                let block_text = deleted_lines.join("\n");

                self.cursor.line = top;
                self.cursor.col = left;
                self.clamp_cursor();

                self.registers.delete(None, RegisterContent::Chars(block_text), false);

                self.mode = Mode::Insert;
            }
            _ => {}
        }

        self.scroll_to_cursor();
    }

    // ============================================
    // Text Object Operations
    // ============================================

    /// Find the range of a text object at the cursor position
    /// Returns Option<(start_line, start_col, end_line, end_col)>
    pub fn find_text_object_range(&self, text_object: TextObject) -> Option<(usize, usize, usize, usize)> {
        match text_object.object_type {
            TextObjectType::Word => self.find_word_object(text_object.modifier, false),
            TextObjectType::BigWord => self.find_word_object(text_object.modifier, true),
            TextObjectType::DoubleQuote => self.find_quote_object(text_object.modifier, '"'),
            TextObjectType::SingleQuote => self.find_quote_object(text_object.modifier, '\''),
            TextObjectType::BackTick => self.find_quote_object(text_object.modifier, '`'),
            TextObjectType::Paren => self.find_bracket_object(text_object.modifier, '(', ')'),
            TextObjectType::Brace => self.find_bracket_object(text_object.modifier, '{', '}'),
            TextObjectType::Bracket => self.find_bracket_object(text_object.modifier, '[', ']'),
            TextObjectType::AngleBracket => self.find_bracket_object(text_object.modifier, '<', '>'),
        }
    }

    /// Find word text object boundaries
    fn find_word_object(&self, modifier: TextObjectModifier, big_word: bool) -> Option<(usize, usize, usize, usize)> {
        let line = self.cursor.line;
        let col = self.cursor.col;
        let line_text: String = self.buffers[self.current_buffer_idx].line(line)?.chars().collect();

        if line_text.is_empty() {
            return None;
        }

        let col = col.min(line_text.len().saturating_sub(1));

        let is_word_char = |c: char| -> bool {
            if big_word {
                !c.is_whitespace()
            } else {
                c.is_alphanumeric() || c == '_'
            }
        };

        let chars: Vec<char> = line_text.chars().collect();

        // Find start of word
        let mut start = col;
        let current_char = chars.get(col)?;
        let in_word = is_word_char(*current_char);
        let in_whitespace = current_char.is_whitespace();

        if in_word {
            // Move back to start of word
            while start > 0 && is_word_char(chars[start - 1]) {
                start -= 1;
            }
        } else if !in_whitespace {
            // In punctuation - find bounds of punctuation sequence
            while start > 0 && !is_word_char(chars[start - 1]) && !chars[start - 1].is_whitespace() {
                start -= 1;
            }
        } else {
            // In whitespace - for "inner", return the whitespace
            // For "around", this is an edge case
            while start > 0 && chars[start - 1].is_whitespace() {
                start -= 1;
            }
        }

        // Find end of word
        let mut end = col;
        if in_word {
            while end < chars.len() - 1 && is_word_char(chars[end + 1]) {
                end += 1;
            }
        } else if !in_whitespace {
            while end < chars.len() - 1 && !is_word_char(chars[end + 1]) && !chars[end + 1].is_whitespace() {
                end += 1;
            }
        } else {
            while end < chars.len() - 1 && chars[end + 1].is_whitespace() {
                end += 1;
            }
        }

        // For "around", include trailing whitespace (or leading if at end)
        if modifier == TextObjectModifier::Around {
            // Try trailing whitespace first
            let mut trailing = end + 1;
            while trailing < chars.len() && chars[trailing].is_whitespace() {
                trailing += 1;
            }
            if trailing > end + 1 {
                end = trailing - 1;
            } else {
                // No trailing whitespace, try leading
                let mut leading = start;
                while leading > 0 && chars[leading - 1].is_whitespace() {
                    leading -= 1;
                }
                if leading < start {
                    start = leading;
                }
            }
        }

        Some((line, start, line, end))
    }

    /// Find quote text object boundaries
    fn find_quote_object(&self, modifier: TextObjectModifier, quote: char) -> Option<(usize, usize, usize, usize)> {
        let line = self.cursor.line;
        let col = self.cursor.col;
        let line_text: String = self.buffers[self.current_buffer_idx].line(line)?.chars().collect();

        let chars: Vec<char> = line_text.chars().collect();

        // Find opening quote (search backward and forward from cursor)
        let mut open_pos = None;
        let mut close_pos = None;

        // Check if we're inside quotes by finding quote pairs
        let mut in_quotes = false;
        let mut last_quote = None;

        for (i, &c) in chars.iter().enumerate() {
            if c == quote {
                if !in_quotes {
                    last_quote = Some(i);
                    in_quotes = true;
                } else {
                    // Found a pair
                    if let Some(start) = last_quote {
                        if col >= start && col <= i {
                            open_pos = Some(start);
                            close_pos = Some(i);
                            break;
                        }
                    }
                    in_quotes = false;
                    last_quote = None;
                }
            }
        }

        let open = open_pos?;
        let close = close_pos?;

        match modifier {
            TextObjectModifier::Inner => {
                if close > open + 1 {
                    Some((line, open + 1, line, close - 1))
                } else {
                    // Empty quotes
                    None
                }
            }
            TextObjectModifier::Around => Some((line, open, line, close)),
        }
    }

    /// Find bracket text object boundaries with nesting support
    fn find_bracket_object(&self, modifier: TextObjectModifier, open_bracket: char, close_bracket: char) -> Option<(usize, usize, usize, usize)> {
        let cursor_line = self.cursor.line;
        let cursor_col = self.cursor.col;

        // Search backward for opening bracket
        let mut open_pos = None;
        let mut depth = 0;

        // First, search backward from cursor
        'outer: for line_idx in (0..=cursor_line).rev() {
            let line_text: String = self.buffers[self.current_buffer_idx].line(line_idx)?.chars().collect();
            let chars: Vec<char> = line_text.chars().collect();

            let start_col = if line_idx == cursor_line {
                cursor_col.min(chars.len().saturating_sub(1))
            } else {
                chars.len().saturating_sub(1)
            };

            for col in (0..=start_col).rev() {
                if col >= chars.len() {
                    continue;
                }
                let c = chars[col];
                if c == close_bracket {
                    depth += 1;
                } else if c == open_bracket {
                    if depth == 0 {
                        open_pos = Some((line_idx, col));
                        break 'outer;
                    }
                    depth -= 1;
                }
            }
        }

        let (open_line, open_col) = open_pos?;

        // Search forward for closing bracket
        let mut close_pos = None;
        depth = 0;

        'outer: for line_idx in open_line..self.buffers[self.current_buffer_idx].len_lines() {
            let line_text: String = self.buffers[self.current_buffer_idx].line(line_idx)?.chars().collect();
            let chars: Vec<char> = line_text.chars().collect();

            let start_col = if line_idx == open_line { open_col } else { 0 };

            for col in start_col..chars.len() {
                let c = chars[col];
                if c == open_bracket {
                    depth += 1;
                } else if c == close_bracket {
                    if depth == 1 {
                        close_pos = Some((line_idx, col));
                        break 'outer;
                    }
                    depth -= 1;
                }
            }
        }

        let (close_line, close_col) = close_pos?;

        match modifier {
            TextObjectModifier::Inner => {
                if close_line == open_line && close_col <= open_col + 1 {
                    // Empty brackets
                    None
                } else if close_line == open_line {
                    Some((open_line, open_col + 1, close_line, close_col - 1))
                } else {
                    // Multi-line
                    Some((open_line, open_col + 1, close_line, close_col.saturating_sub(1)))
                }
            }
            TextObjectModifier::Around => Some((open_line, open_col, close_line, close_col)),
        }
    }

    /// Delete text object
    pub fn delete_text_object(&mut self, text_object: TextObject, register: Option<char>) {
        if let Some((start_line, start_col, end_line, end_col)) = self.find_text_object_range(text_object) {
            // Get text for register
            let text = self.get_range_text(start_line, start_col, end_line, end_col);

            // Record for undo
            self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
            self.undo_stack.record_change(Change::delete(start_line, start_col, text.clone()));

            // Delete the range (inclusive)
            self.buffers[self.current_buffer_idx].delete_range(start_line, start_col, end_line, end_col + 1);

            self.undo_stack.end_undo_group(start_line, start_col);

            // Store in register
            let is_small = !text.contains('\n');
            self.registers.delete(register, RegisterContent::Chars(text), is_small);

            // Move cursor to start
            self.cursor.line = start_line;
            self.cursor.col = start_col;
            self.clamp_cursor();
            self.scroll_to_cursor();
        }
    }

    /// Change text object (delete and enter insert mode)
    pub fn change_text_object(&mut self, text_object: TextObject, register: Option<char>) {
        if let Some((start_line, start_col, end_line, end_col)) = self.find_text_object_range(text_object) {
            // Get text for register
            let text = self.get_range_text(start_line, start_col, end_line, end_col);

            // Record for undo (will be continued in insert mode)
            self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
            self.undo_stack.record_change(Change::delete(start_line, start_col, text.clone()));

            // Delete the range (inclusive)
            self.buffers[self.current_buffer_idx].delete_range(start_line, start_col, end_line, end_col + 1);

            // Store in register
            let is_small = !text.contains('\n');
            self.registers.delete(register, RegisterContent::Chars(text), is_small);

            // Move cursor to start
            self.cursor.line = start_line;
            self.cursor.col = start_col;
            self.clamp_cursor();

            // Enter insert mode (undo group stays open)
            self.mode = Mode::Insert;
            self.scroll_to_cursor();
        }
    }

    /// Yank text object
    pub fn yank_text_object(&mut self, text_object: TextObject, register: Option<char>) {
        if let Some((start_line, start_col, end_line, end_col)) = self.find_text_object_range(text_object) {
            let text = self.get_range_text(start_line, start_col, end_line, end_col);
            self.registers.yank(register, RegisterContent::Chars(text));
            self.set_status("Yanked");
        }
    }

    /// Select text object in visual mode
    pub fn select_text_object(&mut self, text_object: TextObject) {
        if let Some((start_line, start_col, end_line, end_col)) = self.find_text_object_range(text_object) {
            // Set visual selection to cover the text object
            self.visual.anchor_line = start_line;
            self.visual.anchor_col = start_col;
            self.cursor.line = end_line;
            self.cursor.col = end_col;

            // Make sure we're in visual mode
            if !self.mode.is_visual() {
                self.mode = Mode::Visual;
            }

            self.scroll_to_cursor();
        }
    }

    // ============================================
    // New Commands (r, J, zz/zt/zb, .)
    // ============================================

    /// Replace character at cursor with given character (r command)
    pub fn replace_char(&mut self, ch: char) {
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        if line_len == 0 || self.cursor.col >= line_len {
            return;
        }

        // Get the old character for undo
        let old_char = self.buffers[self.current_buffer_idx].char_at(self.cursor.line, self.cursor.col);
        if old_char.is_none() {
            return;
        }
        let old_char = old_char.unwrap();

        // Record for undo
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);
        self.undo_stack.record_change(Change::delete(
            self.cursor.line,
            self.cursor.col,
            old_char.to_string(),
        ));
        self.undo_stack.record_change(Change::insert(
            self.cursor.line,
            self.cursor.col,
            ch.to_string(),
        ));
        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);

        // Delete old char and insert new one
        self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, self.cursor.col);
        self.buffers[self.current_buffer_idx].insert_char(self.cursor.line, self.cursor.col, ch);
    }

    /// Join current line with next line (J command)
    pub fn join_lines(&mut self) {
        let total_lines = self.buffers[self.current_buffer_idx].len_lines();
        if self.cursor.line >= total_lines.saturating_sub(1) {
            // Already on last line, nothing to join
            return;
        }

        // Get current line length (before newline)
        let current_line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);

        // Find the position of the newline at end of current line
        // We need to delete that newline and leading whitespace from next line

        // Get the next line's content
        let next_line: String = self.buffers[self.current_buffer_idx].line(self.cursor.line + 1)
            .map(|l| l.chars().collect())
            .unwrap_or_default();

        // Count leading whitespace to strip
        let leading_ws = next_line.chars().take_while(|c| c.is_whitespace() && *c != '\n').count();

        // Begin undo group
        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

        // Delete the newline at end of current line
        if current_line_len > 0 {
            // The newline is at position current_line_len - 1 if the line includes it
            // Actually we need to delete at current_line_len (after last content char)
            self.undo_stack.record_change(Change::delete(
                self.cursor.line,
                current_line_len,
                "\n".to_string(),
            ));
            // Also record deletion of leading whitespace
            if leading_ws > 0 {
                let ws: String = next_line.chars().take(leading_ws).collect();
                self.undo_stack.record_change(Change::delete(
                    self.cursor.line,
                    current_line_len,
                    ws,
                ));
            }
            // Record insertion of single space
            self.undo_stack.record_change(Change::insert(
                self.cursor.line,
                current_line_len,
                " ".to_string(),
            ));
        }

        // Delete the newline character at end of current line
        // This joins the lines together
        self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, current_line_len);

        // Now the next line is joined. Remove leading whitespace and add single space
        if leading_ws > 0 {
            // Delete leading whitespace
            for _ in 0..leading_ws {
                self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, current_line_len);
            }
        }

        // Insert a single space if the current line didn't end at column 0
        if current_line_len > 0 && !next_line.is_empty() && !next_line.chars().all(|c| c.is_whitespace()) {
            self.buffers[self.current_buffer_idx].insert_char(self.cursor.line, current_line_len, ' ');
            // Position cursor at the space
            self.cursor.col = current_line_len;
        } else if current_line_len > 0 {
            self.cursor.col = current_line_len.saturating_sub(1);
        } else {
            self.cursor.col = 0;
        }

        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
        self.clamp_cursor();
    }

    // ============================================
    // Surround operations (vim-surround style)
    // ============================================

    /// Delete surrounding pair (ds command)
    pub fn delete_surrounding(&mut self, surround_char: char) {
        let (open, close) = Self::get_surround_pair(surround_char);

        // Find the surrounding pair
        if let Some((start_pos, end_pos)) = self.find_surrounding_pair(open, close) {
            self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

            // Delete closing char first (so positions don't shift)
            self.undo_stack.record_change(Change::delete(
                end_pos.0,
                end_pos.1,
                close.to_string(),
            ));
            self.buffers[self.current_buffer_idx].delete_char(end_pos.0, end_pos.1);

            // Delete opening char
            self.undo_stack.record_change(Change::delete(
                start_pos.0,
                start_pos.1,
                open.to_string(),
            ));
            self.buffers[self.current_buffer_idx].delete_char(start_pos.0, start_pos.1);

            // Adjust cursor if needed
            if self.cursor.line == start_pos.0 && self.cursor.col > start_pos.1 {
                self.cursor.col = self.cursor.col.saturating_sub(1);
            }

            self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
            self.clamp_cursor();
        } else {
            self.set_status(format!("No surrounding {} found", surround_char));
        }
    }

    /// Change surrounding pair (cs command)
    pub fn change_surrounding(&mut self, old_char: char, new_char: char) {
        let (old_open, old_close) = Self::get_surround_pair(old_char);
        let (new_open, new_close) = Self::get_surround_pair(new_char);

        // Find the surrounding pair
        if let Some((start_pos, end_pos)) = self.find_surrounding_pair(old_open, old_close) {
            self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

            // Replace closing char first (so positions don't shift for same-line pairs)
            self.undo_stack.record_change(Change::delete(
                end_pos.0,
                end_pos.1,
                old_close.to_string(),
            ));
            self.buffers[self.current_buffer_idx].delete_char(end_pos.0, end_pos.1);
            self.undo_stack.record_change(Change::insert(
                end_pos.0,
                end_pos.1,
                new_close.to_string(),
            ));
            self.buffers[self.current_buffer_idx].insert_char(end_pos.0, end_pos.1, new_close);

            // Replace opening char
            self.undo_stack.record_change(Change::delete(
                start_pos.0,
                start_pos.1,
                old_open.to_string(),
            ));
            self.buffers[self.current_buffer_idx].delete_char(start_pos.0, start_pos.1);
            self.undo_stack.record_change(Change::insert(
                start_pos.0,
                start_pos.1,
                new_open.to_string(),
            ));
            self.buffers[self.current_buffer_idx].insert_char(start_pos.0, start_pos.1, new_open);

            self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
        } else {
            self.set_status(format!("No surrounding {} found", old_char));
        }
    }

    /// Add surrounding to text object (ys command)
    pub fn add_surrounding(&mut self, text_object: crate::input::TextObject, surround_char: char) {
        let (open, close) = Self::get_surround_pair(surround_char);

        // Find the range of the text object
        if let Some((start_line, start_col, end_line, end_col)) = self.find_text_object_range(text_object) {
            self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

            // Insert closing char first (so start position doesn't shift if on same line)
            let close_col = if start_line == end_line { end_col + 1 } else { end_col + 1 };
            self.undo_stack.record_change(Change::insert(
                end_line,
                close_col,
                close.to_string(),
            ));
            self.buffers[self.current_buffer_idx].insert_char(end_line, close_col, close);

            // Insert opening char
            self.undo_stack.record_change(Change::insert(
                start_line,
                start_col,
                open.to_string(),
            ));
            self.buffers[self.current_buffer_idx].insert_char(start_line, start_col, open);

            self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
        } else {
            self.set_status("Could not find text object");
        }
    }

    // ============================================
    // Comment toggle operations (gcc, gc{motion})
    // ============================================

    /// Toggle comment on the current line (gcc command)
    pub fn toggle_comment_line(&mut self) {
        self.toggle_comment_lines(self.cursor.line, self.cursor.line);
    }

    /// Toggle comment on a range of lines (gc{motion} command)
    /// Uses the vim convention: if any line is uncommented, comment all; otherwise uncomment all
    pub fn toggle_comment_lines(&mut self, start_line: usize, end_line: usize) {
        let language = self.syntax.language_name();
        let comment_start = crate::syntax::get_comment_string(language);
        let comment_end = crate::syntax::get_comment_end(language);
        let buffer = &self.buffers[self.current_buffer_idx];

        // Determine if we should comment or uncomment
        // If any line is not commented, we comment all; if all are commented, uncomment all
        let mut all_commented = true;
        for line_num in start_line..=end_line {
            if line_num >= buffer.len_lines() {
                break;
            }
            if !self.is_line_commented(line_num, comment_start) {
                all_commented = false;
                break;
            }
        }

        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

        if all_commented {
            // Uncomment all lines
            for line_num in start_line..=end_line {
                if line_num >= self.buffers[self.current_buffer_idx].len_lines() {
                    break;
                }
                self.uncomment_line(line_num, comment_start, comment_end);
            }
        } else {
            // Comment all lines
            for line_num in start_line..=end_line {
                if line_num >= self.buffers[self.current_buffer_idx].len_lines() {
                    break;
                }
                self.comment_line(line_num, comment_start, comment_end);
            }
        }

        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
        self.buffers[self.current_buffer_idx].mark_modified();
    }

    /// Check if a line is commented
    fn is_line_commented(&self, line_num: usize, comment_start: &str) -> bool {
        let buffer = &self.buffers[self.current_buffer_idx];
        if let Some(line) = buffer.line(line_num) {
            let line_str: String = line.chars().collect();
            let trimmed = line_str.trim_start();
            // Empty lines are considered "commented" for the all_commented check
            if trimmed.is_empty() {
                return true;
            }
            trimmed.starts_with(comment_start.trim_end())
        } else {
            true
        }
    }

    /// Comment a single line
    fn comment_line(&mut self, line_num: usize, comment_start: &str, comment_end: Option<&str>) {
        let buffer = &self.buffers[self.current_buffer_idx];
        if let Some(line) = buffer.line(line_num) {
            let line_str: String = line.chars().collect();
            let line_str = line_str.trim_end_matches('\n');

            // Find the indentation
            let indent_len = line_str.len() - line_str.trim_start().len();

            // Skip empty lines
            if line_str.trim().is_empty() {
                return;
            }

            // Record deletion of entire line content
            self.undo_stack.record_change(Change::delete(
                line_num,
                0,
                line_str.to_string(),
            ));

            // Build new line with comment
            let indent = &line_str[..indent_len];
            let content = &line_str[indent_len..];
            let new_line = if let Some(end) = comment_end {
                format!("{}{}{}{}", indent, comment_start, content, end)
            } else {
                format!("{}{}{}", indent, comment_start, content)
            };

            // Delete old content and insert new
            let old_len = self.buffers[self.current_buffer_idx].line_len(line_num);
            for _ in 0..old_len {
                self.buffers[self.current_buffer_idx].delete_char(line_num, 0);
            }

            self.undo_stack.record_change(Change::insert(
                line_num,
                0,
                new_line.clone(),
            ));
            self.buffers[self.current_buffer_idx].insert_str(line_num, 0, &new_line);
        }
    }

    /// Uncomment a single line
    fn uncomment_line(&mut self, line_num: usize, comment_start: &str, comment_end: Option<&str>) {
        let buffer = &self.buffers[self.current_buffer_idx];
        if let Some(line) = buffer.line(line_num) {
            let line_str: String = line.chars().collect();
            let line_str = line_str.trim_end_matches('\n');
            let trimmed = line_str.trim_start();

            // Check if line is commented
            let comment_prefix = comment_start.trim_end();
            if !trimmed.starts_with(comment_prefix) {
                return;
            }

            let indent_len = line_str.len() - trimmed.len();
            let indent = &line_str[..indent_len];

            // Remove comment prefix
            let mut content = &trimmed[comment_prefix.len()..];

            // Remove leading space after comment if present
            if content.starts_with(' ') {
                content = &content[1..];
            }

            // Remove comment suffix if present
            if let Some(end) = comment_end {
                let end_trimmed = end.trim_start();
                if content.ends_with(end_trimmed) {
                    content = &content[..content.len() - end_trimmed.len()];
                    // Remove trailing space before comment end
                    content = content.trim_end();
                }
            }

            // Record deletion of entire line content
            self.undo_stack.record_change(Change::delete(
                line_num,
                0,
                line_str.to_string(),
            ));

            let new_line = format!("{}{}", indent, content);

            // Delete old content and insert new
            let old_len = self.buffers[self.current_buffer_idx].line_len(line_num);
            for _ in 0..old_len {
                self.buffers[self.current_buffer_idx].delete_char(line_num, 0);
            }

            self.undo_stack.record_change(Change::insert(
                line_num,
                0,
                new_line.clone(),
            ));
            self.buffers[self.current_buffer_idx].insert_str(line_num, 0, &new_line);
        }
    }

    // ============================================
    // Indent/Dedent operations
    // ============================================

    /// Indent a range of lines by one level
    pub fn indent_lines(&mut self, start_line: usize, end_line: usize) {
        let indent_str = " ".repeat(self.settings.editor.tab_width);
        let buffer = &self.buffers[self.current_buffer_idx];
        let max_line = buffer.len_lines().saturating_sub(1);
        let end_line = end_line.min(max_line);

        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

        for line_num in start_line..=end_line {
            // Get current line content
            if let Some(line) = self.buffers[self.current_buffer_idx].line(line_num) {
                let line_str: String = line.chars().collect();
                let line_str = line_str.trim_end_matches('\n');

                // Skip empty lines
                if line_str.is_empty() {
                    continue;
                }

                // Record insertion for undo
                self.undo_stack.record_change(Change::insert(
                    line_num,
                    0,
                    indent_str.clone(),
                ));

                // Insert the indentation at the beginning
                self.buffers[self.current_buffer_idx].insert_str(line_num, 0, &indent_str);
            }
        }

        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
        self.buffers[self.current_buffer_idx].mark_modified();

        // Move cursor to first non-blank of first line
        self.cursor.line = start_line;
        self.cursor.col = self.find_first_non_blank(start_line);
        self.clamp_cursor();
    }

    /// Dedent a range of lines by one level
    pub fn dedent_lines(&mut self, start_line: usize, end_line: usize) {
        let tab_width = self.settings.editor.tab_width;
        let buffer = &self.buffers[self.current_buffer_idx];
        let max_line = buffer.len_lines().saturating_sub(1);
        let end_line = end_line.min(max_line);

        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

        for line_num in start_line..=end_line {
            // Get current line content
            if let Some(line) = self.buffers[self.current_buffer_idx].line(line_num) {
                let line_str: String = line.chars().collect();

                // Count leading whitespace
                let mut spaces_to_remove = 0;
                for ch in line_str.chars() {
                    if ch == ' ' && spaces_to_remove < tab_width {
                        spaces_to_remove += 1;
                    } else if ch == '\t' && spaces_to_remove < tab_width {
                        // Treat tab as filling to tab_width
                        spaces_to_remove = tab_width;
                        break;
                    } else {
                        break;
                    }
                }

                if spaces_to_remove == 0 {
                    continue;
                }

                // Record deletion for undo
                let deleted_text: String = line_str.chars().take(spaces_to_remove).collect();
                self.undo_stack.record_change(Change::delete(
                    line_num,
                    0,
                    deleted_text,
                ));

                // Delete the leading whitespace
                for _ in 0..spaces_to_remove {
                    self.buffers[self.current_buffer_idx].delete_char(line_num, 0);
                }
            }
        }

        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
        self.buffers[self.current_buffer_idx].mark_modified();

        // Move cursor to first non-blank of first line
        self.cursor.line = start_line;
        self.cursor.col = self.find_first_non_blank(start_line);
        self.clamp_cursor();
    }

    /// Indent with motion (>{motion})
    pub fn indent_motion(&mut self, motion: Motion, count: usize) {
        // Get the line range affected by the motion
        if let Some((start_line, _, end_line, _)) = self.motion_range(motion, count) {
            self.indent_lines(start_line, end_line);
        }
    }

    /// Dedent with motion (<{motion})
    pub fn dedent_motion(&mut self, motion: Motion, count: usize) {
        // Get the line range affected by the motion
        if let Some((start_line, _, end_line, _)) = self.motion_range(motion, count) {
            self.dedent_lines(start_line, end_line);
        }
    }

    /// Indent current line and count-1 lines below (>> operation)
    pub fn indent_line(&mut self, count: usize) {
        let start_line = self.cursor.line;
        let end_line = start_line + count.saturating_sub(1);
        self.indent_lines(start_line, end_line);
    }

    /// Dedent current line and count-1 lines below (<< operation)
    pub fn dedent_line(&mut self, count: usize) {
        let start_line = self.cursor.line;
        let end_line = start_line + count.saturating_sub(1);
        self.dedent_lines(start_line, end_line);
    }

    /// Indent text object
    pub fn indent_text_object(&mut self, text_object: TextObject) {
        if let Some((start_line, _, end_line, _)) = self.find_text_object_range(text_object) {
            self.indent_lines(start_line, end_line);
        }
    }

    /// Dedent text object
    pub fn dedent_text_object(&mut self, text_object: TextObject) {
        if let Some((start_line, _, end_line, _)) = self.find_text_object_range(text_object) {
            self.dedent_lines(start_line, end_line);
        }
    }

    // ============================================
    // Case transformation operations
    // ============================================

    /// Transform the case of text in a range
    pub fn transform_case(&mut self, start_line: usize, start_col: usize, end_line: usize, end_col: usize, op: CaseOperator) {
        let text = self.get_range_text(start_line, start_col, end_line, end_col);
        if text.is_empty() {
            return;
        }

        let transformed: String = match op {
            CaseOperator::Lowercase => text.to_lowercase(),
            CaseOperator::Uppercase => text.to_uppercase(),
            CaseOperator::ToggleCase => text.chars().map(|c| {
                if c.is_lowercase() {
                    c.to_uppercase().next().unwrap_or(c)
                } else if c.is_uppercase() {
                    c.to_lowercase().next().unwrap_or(c)
                } else {
                    c
                }
            }).collect(),
        };

        if text == transformed {
            return;
        }

        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

        // Record deletion of original text
        self.undo_stack.record_change(Change::delete(
            start_line,
            start_col,
            text.clone(),
        ));

        // Delete the original text
        self.buffers[self.current_buffer_idx].delete_range(start_line, start_col, end_line, end_col + 1);

        // Record and insert the transformed text
        self.undo_stack.record_change(Change::insert(
            start_line,
            start_col,
            transformed.clone(),
        ));
        self.buffers[self.current_buffer_idx].insert_str(start_line, start_col, &transformed);

        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
        self.buffers[self.current_buffer_idx].mark_modified();
        self.clamp_cursor();
    }

    /// Case transformation with motion (gu{motion}, gU{motion}, g~{motion})
    pub fn case_motion(&mut self, op: CaseOperator, motion: Motion, count: usize) {
        if let Some((start_line, start_col, end_line, end_col)) = self.motion_range(motion, count) {
            self.transform_case(start_line, start_col, end_line, end_col, op);
            // Move cursor to start of range
            self.cursor.line = start_line;
            self.cursor.col = start_col;
        }
    }

    /// Case transformation on current line (guu, gUU, g~~)
    pub fn case_line(&mut self, op: CaseOperator, count: usize) {
        let start_line = self.cursor.line;
        let buffer = &self.buffers[self.current_buffer_idx];
        let end_line = (start_line + count.saturating_sub(1)).min(buffer.len_lines().saturating_sub(1));

        self.undo_stack.begin_undo_group(self.cursor.line, self.cursor.col);

        for line_num in start_line..=end_line {
            if let Some(line) = self.buffers[self.current_buffer_idx].line(line_num) {
                let line_str: String = line.chars().collect();
                let line_str = line_str.trim_end_matches('\n');
                if line_str.is_empty() {
                    continue;
                }

                let transformed: String = match op {
                    CaseOperator::Lowercase => line_str.to_lowercase(),
                    CaseOperator::Uppercase => line_str.to_uppercase(),
                    CaseOperator::ToggleCase => line_str.chars().map(|c| {
                        if c.is_lowercase() {
                            c.to_uppercase().next().unwrap_or(c)
                        } else if c.is_uppercase() {
                            c.to_lowercase().next().unwrap_or(c)
                        } else {
                            c
                        }
                    }).collect(),
                };

                if line_str != transformed {
                    // Record deletion
                    self.undo_stack.record_change(Change::delete(
                        line_num,
                        0,
                        line_str.to_string(),
                    ));

                    // Delete old content
                    let old_len = self.buffers[self.current_buffer_idx].line_len(line_num);
                    for _ in 0..old_len {
                        self.buffers[self.current_buffer_idx].delete_char(line_num, 0);
                    }

                    // Record and insert new content
                    self.undo_stack.record_change(Change::insert(
                        line_num,
                        0,
                        transformed.clone(),
                    ));
                    self.buffers[self.current_buffer_idx].insert_str(line_num, 0, &transformed);
                }
            }
        }

        self.undo_stack.end_undo_group(self.cursor.line, self.cursor.col);
        self.buffers[self.current_buffer_idx].mark_modified();

        // Move cursor to first non-blank of start line
        self.cursor.line = start_line;
        self.cursor.col = self.find_first_non_blank(start_line);
        self.clamp_cursor();
    }

    /// Case transformation on text object (guiw, gUaw, etc.)
    pub fn case_text_object(&mut self, op: CaseOperator, text_object: TextObject) {
        if let Some((start_line, start_col, end_line, end_col)) = self.find_text_object_range(text_object) {
            self.transform_case(start_line, start_col, end_line, end_col, op);
            // Move cursor to start of text object
            self.cursor.line = start_line;
            self.cursor.col = start_col;
        }
    }

    /// Case transformation on visual selection
    pub fn case_visual(&mut self, op: CaseOperator) {
        let (start_line, start_col, end_line, end_col) = self.get_visual_range();
        self.transform_case(start_line, start_col, end_line, end_col, op);
    }

    // ============================================
    // Mark operations
    // ============================================

    /// Get a unique key for the current buffer (used for local marks)
    fn buffer_key(&self) -> String {
        if let Some(ref path) = self.buffers[self.current_buffer_idx].path {
            path.to_string_lossy().to_string()
        } else {
            format!("__unnamed_{}", self.current_buffer_idx)
        }
    }

    /// Set a mark at the current cursor position
    pub fn set_mark(&mut self, name: char) {
        if !Marks::is_valid_mark(name) {
            self.set_status(format!("Invalid mark: {}", name));
            return;
        }

        let buffer_key = self.buffer_key();
        let path = self.buffers[self.current_buffer_idx].path.clone();

        self.marks.set(&buffer_key, path, name, self.cursor.line, self.cursor.col);
        self.set_status(format!("Mark '{}' set", name));
    }

    /// Jump to the line of a mark (first non-blank character)
    pub fn goto_mark_line(&mut self, name: char) {
        if !Marks::is_valid_mark(name) {
            self.set_status(format!("Invalid mark: {}", name));
            return;
        }

        let buffer_key = self.buffer_key();

        // For global marks, we might need to open a different file
        if name.is_uppercase() {
            if let Some(mark) = self.marks.get_global(name) {
                if let Some(ref path) = mark.path {
                    // Check if we need to open a different file
                    let current_path = self.buffers[self.current_buffer_idx].path.as_ref();
                    if current_path != Some(path) {
                        // Store the mark info before opening file
                        let target_line = mark.line;
                        let path_clone = path.clone();

                        // Try to open the file (will be handled by main loop if needed)
                        if let Err(e) = self.open_file(path_clone) {
                            self.set_status(format!("Cannot open file for mark: {}", e));
                            return;
                        }

                        // Jump to the line
                        self.cursor.line = target_line.min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
                        self.cursor.col = self.find_first_non_blank(self.cursor.line);
                        self.clamp_cursor();
                        self.scroll_to_cursor();
                        return;
                    }
                }
            }
        }

        // Local mark or global mark in current file
        if let Some(mark) = self.marks.get(&buffer_key, name) {
            // Record jump in jump list
            let current_path = self.buffers[self.current_buffer_idx].path.clone();
            self.jump_list.record(current_path, self.cursor.line, self.cursor.col);

            self.cursor.line = mark.line.min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
            self.cursor.col = self.find_first_non_blank(self.cursor.line);
            self.clamp_cursor();
            self.scroll_to_cursor();
        } else {
            self.set_status(format!("Mark '{}' not set", name));
        }
    }

    /// Jump to the exact position of a mark (line and column)
    pub fn goto_mark_exact(&mut self, name: char) {
        if !Marks::is_valid_mark(name) {
            self.set_status(format!("Invalid mark: {}", name));
            return;
        }

        let buffer_key = self.buffer_key();

        // For global marks, we might need to open a different file
        if name.is_uppercase() {
            if let Some(mark) = self.marks.get_global(name) {
                if let Some(ref path) = mark.path {
                    // Check if we need to open a different file
                    let current_path = self.buffers[self.current_buffer_idx].path.as_ref();
                    if current_path != Some(path) {
                        // Store the mark info before opening file
                        let target_line = mark.line;
                        let target_col = mark.col;
                        let path_clone = path.clone();

                        // Try to open the file
                        if let Err(e) = self.open_file(path_clone) {
                            self.set_status(format!("Cannot open file for mark: {}", e));
                            return;
                        }

                        // Jump to the exact position
                        self.cursor.line = target_line.min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
                        self.cursor.col = target_col;
                        self.clamp_cursor();
                        self.scroll_to_cursor();
                        return;
                    }
                }
            }
        }

        // Local mark or global mark in current file
        if let Some(mark) = self.marks.get(&buffer_key, name) {
            // Record jump in jump list
            let current_path = self.buffers[self.current_buffer_idx].path.clone();
            self.jump_list.record(current_path, self.cursor.line, self.cursor.col);

            self.cursor.line = mark.line.min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
            self.cursor.col = mark.col;
            self.clamp_cursor();
            self.scroll_to_cursor();
        } else {
            self.set_status(format!("Mark '{}' not set", name));
        }
    }

    /// Get the open and close characters for a surround pair
    fn get_surround_pair(c: char) -> (char, char) {
        match c {
            '(' | ')' => ('(', ')'),
            '[' | ']' => ('[', ']'),
            '{' | '}' => ('{', '}'),
            '<' | '>' => ('<', '>'),
            '"' => ('"', '"'),
            '\'' => ('\'', '\''),
            '`' => ('`', '`'),
            _ => (c, c), // Default to same char for both
        }
    }

    /// Find the positions of a surrounding pair around the cursor
    /// Returns (start_pos, end_pos) where pos is (line, col)
    fn find_surrounding_pair(&self, open: char, close: char) -> Option<((usize, usize), (usize, usize))> {
        let buffer = &self.buffers[self.current_buffer_idx];
        let line = self.cursor.line;
        let col = self.cursor.col;

        // For same open/close chars (quotes), use simpler logic
        if open == close {
            // Look on current line for quote pairs
            let line_content: String = buffer.line(line)?.chars().collect();
            let chars: Vec<char> = line_content.chars().collect();

            // Find all positions of the quote char
            let positions: Vec<usize> = chars.iter().enumerate()
                .filter(|(_, c)| **c == open)
                .map(|(i, _)| i)
                .collect();

            // Find a pair that contains the cursor
            for i in (0..positions.len()).step_by(2) {
                if i + 1 < positions.len() {
                    let start = positions[i];
                    let end = positions[i + 1];
                    if col >= start && col <= end {
                        return Some(((line, start), (line, end)));
                    }
                }
            }
            return None;
        }

        // For bracket pairs, use balance counting
        // Search backward for opening bracket
        let mut depth = 0;
        let mut start_pos = None;

        // Search on current line from cursor backward
        let line_content: String = buffer.line(line)?.chars().collect();
        let chars: Vec<char> = line_content.chars().collect();

        for i in (0..=col.min(chars.len().saturating_sub(1))).rev() {
            if chars[i] == close {
                depth += 1;
            } else if chars[i] == open {
                if depth == 0 {
                    start_pos = Some((line, i));
                    break;
                }
                depth -= 1;
            }
        }

        // If not found on current line, search previous lines
        if start_pos.is_none() {
            for l in (0..line).rev() {
                let line_content: String = buffer.line(l)?.chars().collect();
                let chars: Vec<char> = line_content.chars().collect();
                for i in (0..chars.len()).rev() {
                    if chars[i] == close {
                        depth += 1;
                    } else if chars[i] == open {
                        if depth == 0 {
                            start_pos = Some((l, i));
                            break;
                        }
                        depth -= 1;
                    }
                }
                if start_pos.is_some() {
                    break;
                }
            }
        }

        let start_pos = start_pos?;

        // Search forward for closing bracket
        depth = 0;
        let mut end_pos = None;

        // Start search from position after open bracket
        let start_search_line = start_pos.0;
        let start_search_col = start_pos.1 + 1;

        for l in start_search_line..buffer.len_lines() {
            let line_content: String = buffer.line(l)?.chars().collect();
            let chars: Vec<char> = line_content.chars().collect();
            let start_col = if l == start_search_line { start_search_col } else { 0 };

            for i in start_col..chars.len() {
                if chars[i] == open {
                    depth += 1;
                } else if chars[i] == close {
                    if depth == 0 {
                        end_pos = Some((l, i));
                        break;
                    }
                    depth -= 1;
                }
            }
            if end_pos.is_some() {
                break;
            }
        }

        Some((start_pos, end_pos?))
    }

    /// Scroll viewport so cursor is at center of screen (zz command)
    pub fn scroll_cursor_center(&mut self) {
        let text_rows = self.text_rows();
        let half = text_rows / 2;

        if self.cursor.line >= half {
            self.viewport_offset = self.cursor.line - half;
        } else {
            self.viewport_offset = 0;
        }
        // Sync to active pane for rendering
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
        }
    }

    /// Scroll viewport so cursor is at top of screen (zt command)
    pub fn scroll_cursor_top(&mut self) {
        self.viewport_offset = self.cursor.line;
        // Sync to active pane for rendering
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
        }
    }

    /// Scroll viewport so cursor is at bottom of screen (zb command)
    pub fn scroll_cursor_bottom(&mut self) {
        let text_rows = self.text_rows();
        if self.cursor.line >= text_rows.saturating_sub(1) {
            self.viewport_offset = self.cursor.line - text_rows + 1;
        } else {
            self.viewport_offset = 0;
        }
        // Sync to active pane for rendering
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
        }
    }

    /// Repeat last change (. command)
    /// Note: Full implementation would store last command sequence.
    /// For now, this is a placeholder that shows a message.
    pub fn repeat_last_change(&mut self) {
        // TODO: Implement proper repeat functionality
        // This requires storing the last change sequence (keys or operations)
        self.set_status(". (repeat) not fully implemented yet");
    }

    /// Apply motion with screen-relative awareness
    /// This overrides basic motion for H, M, L which need viewport info
    pub fn apply_motion(&mut self, motion: Motion, count: usize) {
        // Handle screen-relative motions specially
        match motion {
            Motion::ScreenTop => {
                // H - move to top of visible screen (+ count lines from top)
                let target_line = self.viewport_offset + count.saturating_sub(1);
                let target_line = target_line.min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
                self.cursor.line = target_line;
                // Move to first non-blank
                self.cursor.col = self.find_first_non_blank(self.cursor.line);
                self.clamp_cursor();
                self.scroll_to_cursor();
            }
            Motion::ScreenMiddle => {
                // M - move to middle of visible screen
                let text_rows = self.text_rows();
                let middle = text_rows / 2;
                let target_line = (self.viewport_offset + middle).min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
                self.cursor.line = target_line;
                // Move to first non-blank
                self.cursor.col = self.find_first_non_blank(self.cursor.line);
                self.clamp_cursor();
                self.scroll_to_cursor();
            }
            Motion::ScreenBottom => {
                // L - move to bottom of visible screen (- count lines from bottom)
                let text_rows = self.text_rows();
                let bottom_screen_line = self.viewport_offset + text_rows.saturating_sub(1);
                let target_line = bottom_screen_line.saturating_sub(count.saturating_sub(1));
                let target_line = target_line.min(self.buffers[self.current_buffer_idx].len_lines().saturating_sub(1));
                self.cursor.line = target_line;
                // Move to first non-blank
                self.cursor.col = self.find_first_non_blank(self.cursor.line);
                self.clamp_cursor();
                self.scroll_to_cursor();
            }
            _ => {
                // Use standard motion handling
                if let Some((new_line, new_col)) = apply_motion(
                    &self.buffers[self.current_buffer_idx],
                    motion,
                    self.cursor.line,
                    self.cursor.col,
                    count,
                    self.text_rows(),
                ) {
                    self.cursor.line = new_line;
                    self.cursor.col = new_col;
                    self.clamp_cursor();
                    self.scroll_to_cursor();
                }
            }
        }
    }

    /// Find first non-blank character on a line
    fn find_first_non_blank(&self, line: usize) -> usize {
        let line_len = self.buffers[self.current_buffer_idx].line_len(line);
        for col in 0..line_len {
            if let Some(ch) = self.buffers[self.current_buffer_idx].char_at(line, col) {
                if !ch.is_whitespace() {
                    return col;
                }
            }
        }
        0
    }

    // ============================================
    // Fuzzy Finder
    // ============================================

    /// Open the fuzzy finder in file mode
    pub fn open_finder_files(&mut self) {
        let root = self.working_directory();
        self.finder.open_files(&root);
        self.mode = Mode::Finder;
    }

    /// Open the fuzzy finder in buffer mode
    pub fn open_finder_buffers(&mut self) {
        let buffer_info: Vec<(usize, String, std::path::PathBuf)> = self.buffers
            .iter()
            .enumerate()
            .map(|(idx, buf)| {
                let name = buf.display_name().to_string();
                let path = buf.path.clone().unwrap_or_default();
                (idx, name, path)
            })
            .collect();
        self.finder.open_buffers(buffer_info);
        self.mode = Mode::Finder;
    }

    /// Open the fuzzy finder in grep mode (live search)
    pub fn open_finder_grep(&mut self) {
        let root = self.working_directory();
        self.finder.open_grep(&root);
        self.mode = Mode::Finder;
    }

    /// Open the fuzzy finder in diagnostics mode
    pub fn open_finder_diagnostics(&mut self) {
        use crate::finder::FinderItem;
        use crate::lsp::types::DiagnosticSeverity;

        let mut diagnostic_items: Vec<FinderItem> = Vec::new();
        let cwd = self.working_directory();

        // Collect diagnostics from all files, sorted by severity (errors first)
        let mut all_diags: Vec<(&String, &crate::lsp::types::Diagnostic)> = self
            .diagnostics
            .iter()
            .flat_map(|(uri, diags)| diags.iter().map(move |d| (uri, d)))
            .collect();

        // Sort: errors first, then warnings, then info/hints
        all_diags.sort_by(|(_, a), (_, b)| {
            let a_severity = match a.severity {
                DiagnosticSeverity::Error => 0,
                DiagnosticSeverity::Warning => 1,
                DiagnosticSeverity::Information => 2,
                DiagnosticSeverity::Hint => 3,
            };
            let b_severity = match b.severity {
                DiagnosticSeverity::Error => 0,
                DiagnosticSeverity::Warning => 1,
                DiagnosticSeverity::Information => 2,
                DiagnosticSeverity::Hint => 3,
            };
            a_severity.cmp(&b_severity).then_with(|| a.line.cmp(&b.line))
        });

        for (uri, diag) in all_diags {
            // Get relative path from URI
            let path = if uri.starts_with("file://") {
                std::path::PathBuf::from(&uri[7..])
            } else {
                std::path::PathBuf::from(uri)
            };

            let rel_path = path
                .strip_prefix(&cwd)
                .unwrap_or(&path)
                .to_string_lossy();

            // Format: [E/W] line:col message | filepath
            let severity_indicator = match diag.severity {
                DiagnosticSeverity::Error => "[E]",
                DiagnosticSeverity::Warning => "[W]",
                DiagnosticSeverity::Information => "[I]",
                DiagnosticSeverity::Hint => "[H]",
            };

            // Truncate message if too long
            let msg = if diag.message.len() > 60 {
                format!("{}...", &diag.message.chars().take(57).collect::<String>())
            } else {
                diag.message.clone()
            };

            let display = format!(
                "{} {}:{} {} | {}",
                severity_indicator,
                diag.line + 1, // Convert 0-indexed to 1-indexed
                diag.col_start + 1,
                msg,
                rel_path
            );

            let item = FinderItem::new(display, path.clone())
                .with_line(diag.line + 1); // 1-indexed for jumping

            diagnostic_items.push(item);
        }

        self.finder.open_diagnostics(diagnostic_items);
        self.mode = Mode::Finder;
    }

    /// Close the finder and return to normal mode
    pub fn close_finder(&mut self) {
        self.mode = Mode::Normal;
        self.clear_status();
    }

    // === File Explorer Methods ===

    /// Toggle the file explorer sidebar
    pub fn toggle_explorer(&mut self) {
        self.explorer.toggle();
        self.update_pane_rects();
        if self.explorer.visible {
            self.mode = Mode::Explorer;
        } else {
            self.mode = Mode::Normal;
        }
    }

    /// Open the file explorer sidebar
    pub fn open_explorer(&mut self) {
        self.explorer.show();
        self.update_pane_rects();
        self.mode = Mode::Explorer;
    }

    /// Close the file explorer sidebar
    pub fn close_explorer(&mut self) {
        self.explorer.hide();
        self.update_pane_rects();
        if self.mode == Mode::Explorer {
            self.mode = Mode::Normal;
        }
    }

    /// Focus the file explorer (without hiding it)
    pub fn focus_explorer(&mut self) {
        if self.explorer.visible {
            self.mode = Mode::Explorer;
        } else {
            self.open_explorer();
        }
    }

    /// Return focus to the editor from explorer
    pub fn unfocus_explorer(&mut self) {
        if self.mode == Mode::Explorer {
            self.mode = Mode::Normal;
        }
    }

    /// Get the selected file path in the explorer
    pub fn explorer_selected_path(&self) -> Option<std::path::PathBuf> {
        self.explorer.selected_path().cloned()
    }

    /// Reveal the current file in the explorer
    pub fn reveal_in_explorer(&mut self) {
        if let Some(path) = self.buffer().path.clone() {
            if !self.explorer.visible {
                self.explorer.show();
            }
            self.explorer.reveal_file(&path);
        }
    }

    /// Select the current item in the finder and open it
    /// Returns (path, optional_line_number) for grep results
    pub fn finder_select(&mut self) -> Option<crate::finder::FinderItem> {
        let item = self.finder.selected_item().cloned();
        self.close_finder();
        item
    }

    /// Switch to an existing buffer by index
    pub fn switch_to_buffer(&mut self, idx: usize) -> bool {
        if idx >= self.buffers.len() {
            return false;
        }

        self.save_pane_state();
        self.current_buffer_idx = idx;
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].buffer_idx = idx;
            self.panes[self.active_pane].cursor = Cursor::default();
            self.panes[self.active_pane].viewport_offset = 0;
        }
        self.cursor = Cursor::default();
        self.viewport_offset = 0;
        let path = self.buffers[self.current_buffer_idx].path.clone();
        self.syntax.set_language_from_path_option(path.as_ref());
        self.parse_current_buffer();
        true
    }

    fn motion_is_inclusive(motion: Motion) -> bool {
        matches!(
            motion,
            Motion::WordEnd
                | Motion::BigWordEnd
                | Motion::LineEnd
                | Motion::FindChar(_)
                | Motion::FindCharBack(_)
                | Motion::MatchingBracket
        )
    }

    fn motion_range(&self, motion: Motion, count: usize) -> Option<(usize, usize, usize, usize)> {
        let (target_line, target_col) = apply_motion(
            &self.buffers[self.current_buffer_idx],
            motion,
            self.cursor.line,
            self.cursor.col,
            count,
            self.text_rows(),
        )?;

        let forward = (target_line, target_col) >= (self.cursor.line, self.cursor.col);
        let inclusive = forward && Self::motion_is_inclusive(motion);

        let (start_line, start_col, mut end_line, mut end_col) = if (target_line, target_col) < (self.cursor.line, self.cursor.col) {
            (target_line, target_col, self.cursor.line, self.cursor.col)
        } else {
            (self.cursor.line, self.cursor.col, target_line, target_col)
        };

        if !inclusive {
            if end_line == start_line {
                end_col = end_col.saturating_sub(1).max(start_col);
            } else if end_col == 0 {
                let prev_line = end_line.saturating_sub(1);
                let prev_len = self.buffers[self.current_buffer_idx].line_len_including_newline(prev_line);
                end_line = prev_line;
                end_col = prev_len.saturating_sub(1);
            } else {
                end_col = end_col.saturating_sub(1);
            }
        }

        Some((start_line, start_col, end_line, end_col))
    }

    fn parse_current_buffer(&mut self) {
        let buffer_idx = self.current_buffer_idx;
        let buffer = &self.buffers[buffer_idx];
        self.syntax.parse(buffer);
        self.last_syntax_version = buffer.version();
        self.last_edit_at = None;
    }

    pub fn maybe_update_syntax(&mut self) {
        if self.mode == Mode::Insert {
            return;
        }

        let version = self.buffers[self.current_buffer_idx].version();
        if version != self.last_syntax_version {
            self.parse_current_buffer();
        }
    }

    pub fn note_buffer_change(&mut self) {
        self.last_edit_at = Some(Instant::now());
    }

    pub fn maybe_update_syntax_debounced(&mut self, debounce: Duration) -> bool {
        let Some(last) = self.last_edit_at else {
            return false;
        };
        if last.elapsed() < debounce {
            return false;
        }

        let version = self.buffers[self.current_buffer_idx].version();
        if version != self.last_syntax_version {
            self.parse_current_buffer();
            return true;
        }

        self.last_edit_at = None;
        false
    }

    // ============================================
    // References and Code Actions Pickers
    // ============================================

    /// Show the references picker with the given locations
    pub fn show_references_picker(&mut self, locations: Vec<Location>) {
        let count = locations.len();
        self.references_picker = Some(ReferencesPicker::new(locations));
        self.set_status(format!("{} references - j/k to navigate, Enter to go, Esc to close", count));
    }

    /// Hide the references picker
    pub fn hide_references_picker(&mut self) {
        self.references_picker = None;
    }

    /// Show the code actions picker with the given actions
    pub fn show_code_actions_picker(&mut self, actions: Vec<CodeActionItem>) {
        let count = actions.len();
        self.code_actions_picker = Some(CodeActionsPicker::new(actions));
        self.set_status(format!("{} code actions - j/k to navigate, Enter to apply, Esc to close", count));
    }

    /// Hide the code actions picker
    pub fn hide_code_actions_picker(&mut self) {
        self.code_actions_picker = None;
    }

    /// Apply the selected code action's edits
    pub fn apply_selected_code_action(&mut self) -> Option<String> {
        let picker = self.code_actions_picker.take()?;
        let action = picker.items.get(picker.selected)?;

        let title = action.title.clone();
        let mut total_edits = 0;

        // Apply edits from the selected action
        for (_uri, edits) in &action.edits {
            // For now, only apply edits to current file
            // TODO: Handle cross-file edits
            self.apply_text_edits(edits);
            total_edits += edits.len();
        }

        if total_edits > 0 {
            Some(format!("Applied '{}' ({} edits)", title, total_edits))
        } else if action.command.is_some() {
            Some(format!("Action '{}' requires server-side command", title))
        } else {
            Some(format!("Applied '{}'", title))
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new(Settings::default())
    }
}
