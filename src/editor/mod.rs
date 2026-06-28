mod buffer;
mod cursor;
mod macros;
mod marks;
mod register;
mod undo;

pub use buffer::Buffer;
pub use cursor::Cursor;
pub use macros::MacroState;
pub use marks::{Mark, Marks};
pub use register::{RegisterContent, Registers};
pub use undo::{Change, UndoEntry, UndoStack};

use crate::commands::CommandLine;
use crate::config::{KeymapLookup, LeaderAction, LeaderHint, Settings};
use crate::explorer::FileExplorer;
use crate::finder::FuzzyFinder;
use crate::frecency::FrecencyDb;
use crate::input::{
    apply_motion, CaseOperator, InputState, Motion, TextObject, TextObjectModifier, TextObjectType,
};
use crate::lsp::types::{CodeActionItem, CompletionItem, Diagnostic, Location, TextEdit};
use crate::syntax::SyntaxManager;
use crate::theme::ThemeManager;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthChar;

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

/// Where the expression-register result should be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionRegisterTarget {
    /// Normal mode: keep the result in the expression register for the next operation.
    Normal,
    /// Insert mode: insert the result immediately at the cursor.
    Insert,
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

#[derive(Debug, Clone)]
struct TagToken {
    name: String,
    start: usize,
    end: usize,
    kind: TagTokenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagTokenKind {
    Open,
    Close,
    SelfClosing,
}

#[derive(Debug, Clone, Copy)]
struct DisplayLineSegment {
    start_col: usize,
    end_col: usize,
    indent_width: usize,
}

#[derive(Debug, Clone, Copy)]
enum DisplayLineTarget {
    Start,
    End,
    FirstNonBlank,
}

/// Pending LSP action requested by key handler
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspAction {
    /// Go to definition (gd)
    GotoDefinition,
    /// Go to declaration (gD)
    GotoDeclaration,
    /// Go to implementation (gI)
    GotoImplementation,
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
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// A pane/window in the editor showing a buffer
#[derive(Debug, Clone)]
pub struct Pane {
    /// Index of the buffer this pane displays
    pub buffer_idx: usize,
    /// Cursor position in this pane
    pub cursor: Cursor,
    /// Vertical scroll offset for this pane
    pub viewport_offset: usize,
    /// Horizontal scroll offset for this pane
    pub h_offset: usize,
    /// Screen region for this pane
    pub rect: Rect,
}

impl Pane {
    pub fn new(buffer_idx: usize) -> Self {
        Self {
            buffer_idx,
            cursor: Cursor::default(),
            viewport_offset: 0,
            h_offset: 0,
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
    pub fn get_block_range(
        &self,
        cursor_line: usize,
        cursor_col: usize,
    ) -> (usize, usize, usize, usize) {
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

    /// Insert a character at cursor (cursor is character index, not byte index)
    pub fn insert_char(&mut self, ch: char) {
        // Convert character index to byte index for String::insert
        let byte_idx = self.char_to_byte_index(self.cursor);
        self.input.insert(byte_idx, ch);
        self.cursor += 1;
    }

    /// Delete character before cursor
    pub fn delete_char_before(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            // Convert character index to byte index for String::remove
            let byte_idx = self.char_to_byte_index(self.cursor);
            self.input.remove(byte_idx);
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
        if self.cursor < self.input.chars().count() {
            self.cursor += 1;
        }
    }

    /// Convert character index to byte index
    fn char_to_byte_index(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(byte_idx, _)| byte_idx)
            .unwrap_or(self.input.len())
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
    pub fn go_back(
        &mut self,
        current_path: Option<std::path::PathBuf>,
        current_line: usize,
        current_col: usize,
    ) -> Option<&JumpLocation> {
        let mut inserted_current_snapshot = false;

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
                    inserted_current_snapshot = true;
                }
            }
        }

        if self.position == 0 {
            None
        } else {
            // Move to the previous jump.
            self.position -= 1;

            // If we just inserted the current location snapshot, skip over it so
            // the first Ctrl+o actually goes to an older position.
            if inserted_current_snapshot && self.position > 0 {
                self.position -= 1;
            }

            self.jumps.get(self.position)
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

/// A position in the change list (where edits occurred)
#[derive(Debug, Clone)]
pub struct ChangeLocation {
    /// Line number where change occurred
    pub line: usize,
    /// Column number where change occurred
    pub col: usize,
}

/// Change list for navigating to previous edit positions (g; and g,)
#[derive(Debug, Default)]
pub struct ChangeList {
    /// List of change locations
    changes: Vec<ChangeLocation>,
    /// Current position in the change list
    /// When position == changes.len(), we're "at the end" (not navigating)
    position: usize,
}

impl ChangeList {
    /// Maximum number of changes to track
    const MAX_CHANGES: usize = 100;

    /// Record a change position
    pub fn record(&mut self, line: usize, col: usize) {
        // Don't record duplicate consecutive changes on the same line
        if let Some(last) = self.changes.last() {
            if last.line == line {
                // Update the column position instead of adding a new entry
                self.changes.last_mut().unwrap().col = col;
                self.position = self.changes.len();
                return;
            }
        }

        self.changes.push(ChangeLocation { line, col });
        self.position = self.changes.len();

        // Limit change list size
        if self.changes.len() > Self::MAX_CHANGES {
            self.changes.remove(0);
            self.position = self.changes.len();
        }
    }

    /// Go to older change position (g;)
    pub fn go_older(&mut self) -> Option<&ChangeLocation> {
        if self.position > 0 {
            self.position -= 1;
            self.changes.get(self.position)
        } else {
            None
        }
    }

    /// Go to newer change position (g,)
    pub fn go_newer(&mut self) -> Option<&ChangeLocation> {
        if self.position < self.changes.len().saturating_sub(1) {
            self.position += 1;
            self.changes.get(self.position)
        } else if self.position < self.changes.len() {
            // At the last recorded change, return it
            self.changes.get(self.position)
        } else {
            None
        }
    }

    /// Get the most recent change position without changing list navigation.
    pub fn latest(&self) -> Option<&ChangeLocation> {
        self.changes.last()
    }

    /// Check if we have any changes recorded
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
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
    pub fn show(
        &mut self,
        items: Vec<CompletionItem>,
        line: usize,
        col: usize,
        is_incomplete: bool,
    ) {
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
            let mut indices: Vec<(usize, f64, &str)> = self
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let frecency_score = frecency.map(|f| f.score(&item.label)).unwrap_or(1.0);
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
            // Preserve server sortText for prefix matches. Servers such as
            // tsserver use sortText to rank auto-import candidates correctly.
            let query_lower = self.filter_text.to_lowercase();
            let mut scored: Vec<(usize, bool, String, u32, f64)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    let match_text = item.filter_text.as_deref().unwrap_or(&item.label);
                    self.matcher
                        .match_score(&self.filter_text, match_text)
                        .map(|fuzzy_score| {
                            let frecency_score =
                                frecency.map(|f| f.score(&item.label)).unwrap_or(1.0);
                            let is_prefix = match_text.to_lowercase().starts_with(&query_lower);
                            let sort_key =
                                item.sort_text.as_deref().unwrap_or(&item.label).to_string();
                            (i, is_prefix, sort_key, fuzzy_score, frecency_score)
                        })
                })
                .collect();
            scored.sort_by(|a, b| {
                b.1.cmp(&a.1)
                    .then_with(|| a.2.cmp(&b.2))
                    .then_with(|| b.3.cmp(&a.3))
                    .then_with(|| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal))
            });
            self.filtered = scored.into_iter().map(|(i, _, _, _, _)| i).collect();
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
        self.filtered
            .get(self.selected)
            .and_then(|&idx| self.items.get(idx))
    }

    /// Get the text to insert for the selected item
    pub fn selected_insert_text(&self) -> Option<&str> {
        self.selected_item()
            .map(|item| item.insert_text.as_deref().unwrap_or(&item.label))
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
    /// Path of the previously active file buffer, for the `"#` register.
    alternate_file_path: Option<std::path::PathBuf>,
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
    /// Vertical viewport offset (for scrolling, active pane's viewport)
    pub viewport_offset: usize,
    /// Horizontal viewport offset (for scrolling, active pane's h_offset)
    pub h_offset: usize,
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
    /// Saved undo/redo history for each open buffer.
    undo_stacks: Vec<UndoStack>,
    /// Search state
    pub search: SearchState,
    /// Visual selection state
    pub visual: VisualSelection,
    /// Syntax highlighting manager
    pub syntax: SyntaxManager,
    /// Syntax highlighting manager for finder preview
    pub preview_syntax: SyntaxManager,
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
    /// When leader mode started (for timeout tracking)
    pub leader_sequence_start: Option<Instant>,
    /// Action to execute when leader timeout expires
    pub leader_pending_action: Option<LeaderAction>,
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
    /// Change list for g;/g, navigation (positions where edits occurred)
    pub change_list: ChangeList,
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
    /// Floating rendered Markdown preview state.
    pub markdown_preview: Option<crate::markdown_preview::MarkdownPreviewState>,
    /// Marks for navigation (m{a-z}, '{a-z}, `{a-z})
    pub marks: Marks,
    /// Last visual selection for gv command
    pub last_visual_selection: Option<LastVisualSelection>,
    /// Macro recording and playback state
    pub macros: MacroState,
    /// Last insert position for `gi` command (line, col)
    pub last_insert_position: Option<(usize, usize)>,
    /// Text inserted during the most recently completed insert session.
    pub last_inserted_text: Option<String>,
    /// Text inserted during the active insert session.
    current_inserted_text: String,
    /// Insert mode is waiting for a register name after `<C-r>`.
    pub pending_insert_register: bool,
    /// Expression register input is active after `"=` or `<C-r>=`.
    pub pending_expression_register: Option<ExpressionRegisterTarget>,
    /// Expression being typed for the expression register.
    pub expression_register_input: String,
    /// Last evaluated expression register value.
    expression_register_value: Option<String>,
    /// Insert mode temporarily handed control to one normal-mode command after `<C-o>`.
    pub pending_insert_normal_once: bool,
    /// Previous jump position for `''` command (path, line, col)
    /// This is the position from which the last jump was made
    pub previous_jump_position: Option<(Option<std::path::PathBuf>, usize, usize)>,
    /// Language-specific configuration (formatters, tab_width overrides)
    pub languages_config: crate::config::LanguagesConfig,
    /// Startup errors/warnings to display to user (config parse errors, etc.)
    startup_errors: Vec<String>,
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
        let all_items: Vec<(String, bool)> =
            items.into_iter().map(|(s, b)| (s.to_string(), b)).collect();
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
        self.filtered
            .get(self.selected)
            .and_then(|&idx| self.all_items.get(idx))
            .map(|(name, _)| name.as_str())
    }

    /// Get the items that should be displayed (filtered list)
    pub fn visible_items(&self) -> Vec<&(String, bool)> {
        self.filtered
            .iter()
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
            self.filtered = self
                .all_items
                .iter()
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

fn evaluate_expression_register(input: &str) -> Result<String, String> {
    let expression = input.trim();
    if expression.is_empty() {
        return Err("empty expression".to_string());
    }

    if let Some(text) = parse_quoted_expression(expression)? {
        return Ok(text);
    }

    let mut parser = ExpressionParser::new(expression);
    let value = parser.parse_expression()?;
    parser.skip_ws();
    if !parser.is_done() {
        return Err("unexpected trailing input".to_string());
    }
    Ok(format_expression_number(value))
}

fn parse_quoted_expression(input: &str) -> Result<Option<String>, String> {
    let mut chars = input.chars();
    let Some(quote @ ('\'' | '"')) = chars.next() else {
        return Ok(None);
    };

    let mut output = String::new();
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if escaped {
            let resolved = match ch {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '\\' => '\\',
                '\'' => '\'',
                '"' => '"',
                other => other,
            };
            output.push(resolved);
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == quote {
            if chars.as_str().trim().is_empty() {
                return Ok(Some(output));
            }
            return Err("unexpected trailing input".to_string());
        }
        output.push(ch);
    }

    Err("unterminated string".to_string())
}

fn format_expression_number(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let normalized = if value.abs() < 1e-12 { 0.0 } else { value };
    if normalized.fract().abs() < 1e-12 {
        return format!("{}", normalized.trunc() as i64);
    }

    let mut text = format!("{:.12}", normalized);
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

struct ExpressionParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> ExpressionParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn is_done(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn consume(&mut self, expected: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(expected) {
            self.pos += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn parse_expression(&mut self) -> Result<f64, String> {
        let mut value = self.parse_term()?;
        loop {
            if self.consume('+') {
                value += self.parse_term()?;
            } else if self.consume('-') {
                value -= self.parse_term()?;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_term(&mut self) -> Result<f64, String> {
        let mut value = self.parse_factor()?;
        loop {
            if self.consume('*') {
                value *= self.parse_factor()?;
            } else if self.consume('/') {
                let divisor = self.parse_factor()?;
                if divisor.abs() < f64::EPSILON {
                    return Err("division by zero".to_string());
                }
                value /= divisor;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_factor(&mut self) -> Result<f64, String> {
        self.skip_ws();
        if self.consume('+') {
            return self.parse_factor();
        }
        if self.consume('-') {
            return Ok(-self.parse_factor()?);
        }
        if self.consume('(') {
            let value = self.parse_expression()?;
            if !self.consume(')') {
                return Err("expected ')'".to_string());
            }
            return Ok(value);
        }
        self.parse_number()
    }

    fn parse_number(&mut self) -> Result<f64, String> {
        self.skip_ws();
        let start = self.pos;
        let mut seen_digit = false;
        let mut seen_dot = false;

        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                seen_digit = true;
                self.pos += ch.len_utf8();
            } else if ch == '.' && !seen_dot {
                seen_dot = true;
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }

        if !seen_digit {
            return Err("expected number".to_string());
        }

        self.input[start..self.pos]
            .parse::<f64>()
            .map_err(|_| "invalid number".to_string())
    }
}

impl Editor {
    pub fn new(settings: Settings) -> Self {
        let (keymap, keymap_errors) = KeymapLookup::from_settings(&settings.keymap);
        let finder = FuzzyFinder::from_settings(&settings.finder);

        // Initialize theme manager with bundled + user themes
        let mut theme_manager = ThemeManager::new();
        let theme_errors = theme_manager.load_user_themes();
        // Set initial theme from config
        theme_manager.set_theme(&settings.theme.colorscheme);

        // Collect startup errors from config parsing
        let mut startup_errors = keymap_errors;
        startup_errors.extend(theme_errors);

        // Create syntax manager and sync it with the UI theme
        let mut syntax = SyntaxManager::new();
        syntax.sync_theme(theme_manager.theme());

        // Create preview syntax manager for finder preview
        let mut preview_syntax = SyntaxManager::new();
        preview_syntax.sync_theme(theme_manager.theme());

        Self {
            buffers: vec![Buffer::new()],
            current_buffer_idx: 0,
            alternate_file_path: None,
            panes: vec![Pane::new(0)],
            active_pane: 0,
            split_layout: SplitLayout::Vertical,
            cursor: Cursor::default(),
            mode: Mode::default(),
            viewport_offset: 0,
            h_offset: 0,
            term_height: 24,
            term_width: 80,
            should_quit: false,
            status_message: None,
            registers: Registers::new(),
            input_state: InputState::new(),
            command_line: CommandLine::new(),
            undo_stack: UndoStack::new(),
            undo_stacks: vec![UndoStack::new()],
            search: SearchState::default(),
            visual: VisualSelection::default(),
            syntax,
            preview_syntax,
            last_syntax_version: 0,
            last_edit_at: None,
            settings,
            keymap,
            leader_sequence: None,
            leader_sequence_start: None,
            leader_pending_action: None,
            pending_external_command: None,
            finder,
            lsp_status: None,
            diagnostics: HashMap::new(),
            completion: CompletionState::default(),
            pending_lsp_action: None,
            jump_list: JumpList::default(),
            change_list: ChangeList::default(),
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
            markdown_preview: None,
            marks: Marks::new(),
            last_visual_selection: None,
            macros: MacroState::new(),
            last_insert_position: None,
            last_inserted_text: None,
            current_inserted_text: String::new(),
            pending_insert_register: false,
            pending_expression_register: None,
            expression_register_input: String::new(),
            expression_register_value: None,
            pending_insert_normal_once: false,
            previous_jump_position: None,
            languages_config: crate::config::load_languages_config(),
            startup_errors,
        }
    }

    /// Visible leader continuations for the current leader prefix.
    pub fn leader_popup_items(&self) -> Vec<LeaderHint> {
        if !self.settings.keymap.show_leader_popup {
            return Vec::new();
        }
        if !matches!(self.mode, Mode::Normal | Mode::Explorer) {
            return Vec::new();
        }
        let Some(prefix) = self.leader_sequence.as_deref() else {
            return Vec::new();
        };
        self.keymap.leader_hints(prefix)
    }

    /// Take any startup errors (clears them after returning)
    pub fn take_startup_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.startup_errors)
    }

    /// Check for and display any clipboard errors via status message
    pub fn check_clipboard_error(&mut self) {
        if let Some(err) = self.registers.take_clipboard_error() {
            self.set_status(err);
        }
    }

    /// Refresh editor state that can change while the terminal is unfocused.
    pub fn handle_focus_gained(&mut self) -> Option<String> {
        let reload_result = self.check_and_reload_external_changes();
        if self.explorer.visible {
            self.explorer.refresh();
        }
        self.refresh_git_state();
        reload_result
    }

    /// Refresh editor state after returning from an external terminal command.
    pub fn handle_external_process_finished(&mut self) -> Option<String> {
        self.handle_focus_gained()
    }

    /// Set the project root directory
    pub fn set_project_root(&mut self, path: std::path::PathBuf) {
        self.project_root = Some(path.clone());
        self.explorer.set_root(path.clone());
        self.refresh_explorer_git_statuses();
        self.harpoon.set_project_root(path.clone());
        self.floating_terminal.set_working_dir(path);
    }

    /// Get the language name from a file extension
    /// Used for looking up language-specific config
    pub fn extension_to_language(ext: &str) -> String {
        let ext_lower = ext.to_lowercase();
        match ext_lower.as_str() {
            "rs" => "rust".to_string(),
            "ts" => "typescript".to_string(),
            "tsx" => "tsx".to_string(),
            "mts" | "cts" => "typescript".to_string(),
            "js" => "javascript".to_string(),
            "jsx" => "jsx".to_string(),
            "mjs" | "cjs" => "javascript".to_string(),
            "css" => "css".to_string(),
            "scss" => "scss".to_string(),
            "sass" => "sass".to_string(),
            "less" => "less".to_string(),
            "json" | "jsonc" => "json".to_string(),
            "toml" => "toml".to_string(),
            "md" | "markdown" => "markdown".to_string(),
            "html" | "htm" => "html".to_string(),
            "py" | "pyi" | "pyw" => "python".to_string(),
            "go" => "go".to_string(),
            "yaml" | "yml" => "yaml".to_string(),
            "sh" | "bash" | "zsh" => "shell".to_string(),
            _ => ext_lower,
        }
    }

    /// Get the formatter config for the current buffer (if any)
    pub fn get_current_formatter(&self) -> Option<&crate::config::FormatterConfig> {
        let buffer = self.buffer();
        let path = buffer.path.as_ref()?;
        let ext = path.extension()?.to_str()?;
        let language = Self::extension_to_language(ext);
        self.languages_config.get_formatter(&language)
    }

    /// Get the formatter config for a specific buffer by index (if any)
    pub fn get_formatter_for_buffer(
        &self,
        buffer_idx: usize,
    ) -> Option<&crate::config::FormatterConfig> {
        let buffer = self.buffers.get(buffer_idx)?;
        let path = buffer.path.as_ref()?;
        let ext = path.extension()?.to_str()?;
        let language = Self::extension_to_language(ext);
        self.languages_config.get_formatter(&language)
    }

    /// Get the effective tab width for the current buffer
    /// Returns language-specific override or falls back to editor default
    pub fn get_effective_tab_width(&self) -> usize {
        if let Some(path) = self.buffer().path.as_ref() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let language = Self::extension_to_language(ext);
                if let Some(tab_width) = self.languages_config.get_tab_width(&language) {
                    return tab_width;
                }
            }
        }
        self.settings.editor.tab_width
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

    /// Open a snapshot-based Markdown preview for the active buffer.
    pub fn open_markdown_preview(&mut self) -> Result<(), &'static str> {
        let is_markdown = self
            .buffer()
            .path
            .as_deref()
            .and_then(crate::lsp::LanguageId::from_path)
            == Some(crate::lsp::LanguageId::Markdown);

        if !is_markdown {
            return Err("Markdown preview is only available for Markdown buffers");
        }

        let rendered = crate::markdown_preview::render_markdown(&self.buffer().content());
        let width = crate::markdown_preview::preview_content_width(self.term_width);
        self.markdown_preview = Some(crate::markdown_preview::MarkdownPreviewState::new(
            rendered, width,
        ));
        Ok(())
    }

    /// Open a snapshot health report in the read-only preview overlay.
    pub fn open_health_report(&mut self) {
        let report = crate::health::collect_health_report(&self.settings);
        let rendered = crate::markdown_preview::render_markdown(&report);
        let width = crate::markdown_preview::preview_content_width(self.term_width);
        self.markdown_preview = Some(crate::markdown_preview::MarkdownPreviewState::with_title(
            rendered, width, "Health",
        ));
    }

    /// Close the floating Markdown preview.
    pub fn close_markdown_preview(&mut self) {
        self.markdown_preview = None;
    }

    /// Scroll the Markdown preview while clamping to its visible content range.
    pub fn scroll_markdown_preview(&mut self, delta: isize, visible_rows: usize) {
        let Some(preview) = &mut self.markdown_preview else {
            return;
        };

        let next = if delta.is_negative() {
            preview.scroll.saturating_sub(delta.unsigned_abs())
        } else {
            preview.scroll.saturating_add(delta as usize)
        };
        preview.scroll = next.min(preview.max_scroll(visible_rows));
    }

    /// Jump the Markdown preview to the first rendered row.
    pub fn jump_markdown_preview_to_top(&mut self) {
        if let Some(preview) = &mut self.markdown_preview {
            preview.scroll = 0;
        }
    }

    /// Jump the Markdown preview to the last rendered row that can start a page.
    pub fn jump_markdown_preview_to_bottom(&mut self, visible_rows: usize) {
        if let Some(preview) = &mut self.markdown_preview {
            preview.scroll = preview.max_scroll(visible_rows);
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
    pub fn show_completions(
        &mut self,
        items: Vec<CompletionItem>,
        line: usize,
        col: usize,
        is_incomplete: bool,
    ) {
        self.completion.show(items, line, col, is_incomplete);
        self.completion.refilter_with_frecency(Some(&self.frecency));
    }

    /// Update completion filter with frecency-aware sorting
    pub fn update_completion_filter(&mut self, prefix: &str) {
        self.completion.update_filter(prefix);
        self.completion.refilter_with_frecency(Some(&self.frecency));
    }

    /// Update a completion item with resolved documentation and edits.
    pub fn update_completion_item_resolution(
        &mut self,
        item_id: u64,
        label: &str,
        documentation: Option<String>,
        detail: Option<String>,
        text_edit: Option<TextEdit>,
        additional_text_edits: Vec<TextEdit>,
    ) {
        for item in &mut self.completion.items {
            if item.item_id == item_id && item.label == label {
                if documentation.is_some() {
                    item.documentation = documentation;
                }
                if detail.is_some() {
                    item.detail = detail;
                }
                if text_edit.is_some() {
                    item.text_edit = text_edit;
                }
                if !additional_text_edits.is_empty() {
                    item.additional_text_edits = additional_text_edits;
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
        let diags = if let Some(buffer) = self.buffers.iter().find(|buffer| {
            buffer.path.as_ref().map(crate::lsp::path_to_uri).as_deref() == Some(uri.as_str())
        }) {
            diags
                .into_iter()
                .map(|diag| Self::diagnostic_lsp_to_buffer_cols(buffer, diag))
                .collect()
        } else {
            diags
        };

        self.diagnostics.insert(uri, diags);
    }

    /// Apply text edits from LSP formatting (or other sources)
    /// Edits are applied in reverse order to preserve positions
    pub fn apply_text_edits(&mut self, edits: &[TextEdit]) {
        if edits.is_empty() {
            return;
        }

        // Sort edits by position (reverse order) so we can apply from end to start
        let mut sorted_edits: Vec<(usize, &TextEdit)> = edits.iter().enumerate().collect();
        sorted_edits.sort_by(|(a_idx, a), (b_idx, b)| {
            b.start_line
                .cmp(&a.start_line)
                .then_with(|| b.start_col.cmp(&a.start_col))
                .then_with(|| b.end_line.cmp(&a.end_line))
                .then_with(|| b.end_col.cmp(&a.end_col))
                .then_with(|| b_idx.cmp(a_idx))
        });

        // Begin an undo group for all formatting changes
        self.begin_change();

        // Apply each edit
        for (_, edit) in sorted_edits {
            let start_col = self.lsp_utf16_col_to_buffer_col(edit.start_line, edit.start_col);
            let end_col = self.lsp_utf16_col_to_buffer_col(edit.end_line, edit.end_col);

            // Get the text being replaced for undo
            // Note: get_range_text uses inclusive end, but LSP uses exclusive end
            let deleted_text = if edit.end_line > edit.start_line && end_col == 0 {
                // Multi-line edit ending at column 0 means we delete up to (but not including)
                // the start of end_line. Get text from start to end of the line before end_line,
                // including the newline character.
                let prev_line = edit.end_line - 1;
                let prev_line_len_with_newline =
                    self.buffers[self.current_buffer_idx].line_len_including_newline(prev_line);
                self.get_range_text(
                    edit.start_line,
                    start_col,
                    prev_line,
                    prev_line_len_with_newline.saturating_sub(1),
                )
            } else if end_col > 0 || edit.end_line > edit.start_line {
                // Normal case: convert exclusive end_col to inclusive by subtracting 1
                self.get_range_text(
                    edit.start_line,
                    start_col,
                    edit.end_line,
                    end_col.saturating_sub(1),
                )
            } else {
                String::new()
            };

            // Record the deletion for undo
            if !deleted_text.is_empty() {
                self.undo_stack.record_change(Change::delete(
                    edit.start_line,
                    start_col,
                    deleted_text,
                ));
            }

            // Delete the range from the buffer (LSP end_col is exclusive)
            if end_col > 0 || edit.end_line > edit.start_line {
                self.buffers[self.current_buffer_idx].delete_range(
                    edit.start_line,
                    start_col,
                    edit.end_line,
                    end_col,
                );
            }

            // Insert the new text
            if !edit.new_text.is_empty() {
                self.undo_stack.record_change(Change::insert(
                    edit.start_line,
                    start_col,
                    edit.new_text.clone(),
                ));

                // Insert the text using insert_str method
                self.buffers[self.current_buffer_idx].insert_str(
                    edit.start_line,
                    start_col,
                    &edit.new_text,
                );
            }
        }

        // Mark buffer as modified and invalidate syntax
        self.buffers[self.current_buffer_idx].mark_modified();
        self.last_edit_at = Some(Instant::now());

        // End the undo group so LSP edits are a single undo operation
        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);

        // Ensure cursor is in valid position
        self.clamp_cursor();
    }

    fn text_position_after_insert(
        start_line: usize,
        start_col: usize,
        inserted_text: &str,
    ) -> (usize, usize) {
        let mut line = start_line;
        let mut col = start_col;
        for ch in inserted_text.chars() {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn text_edit_buffer_range(&self, edit: &TextEdit) -> ((usize, usize), (usize, usize)) {
        let start_col = self.lsp_utf16_col_to_buffer_col(edit.start_line, edit.start_col);
        let end_col = self.lsp_utf16_col_to_buffer_col(edit.end_line, edit.end_col);
        ((edit.start_line, start_col), (edit.end_line, end_col))
    }

    fn position_after_text_edit(
        &self,
        position: (usize, usize),
        edit: &TextEdit,
    ) -> (usize, usize) {
        let ((start_line, start_col), (end_line, end_col)) = self.text_edit_buffer_range(edit);
        let (pos_line, pos_col) = position;

        if (pos_line, pos_col) < (start_line, start_col) {
            return position;
        }

        let inserted_end = Self::text_position_after_insert(start_line, start_col, &edit.new_text);
        if (pos_line, pos_col) <= (end_line, end_col) {
            return inserted_end;
        }

        let replaced_line_count = end_line.saturating_sub(start_line);
        let inserted_line_count = inserted_end.0.saturating_sub(start_line);

        if pos_line == end_line {
            let trailing_col = pos_col.saturating_sub(end_col);
            (inserted_end.0, inserted_end.1 + trailing_col)
        } else {
            let line = if inserted_line_count >= replaced_line_count {
                pos_line + (inserted_line_count - replaced_line_count)
            } else {
                pos_line.saturating_sub(replaced_line_count - inserted_line_count)
            };
            (line, pos_col)
        }
    }

    fn completion_cursor_after_edit(
        &self,
        main_edit: &TextEdit,
        additional_edits: &[TextEdit],
    ) -> (usize, usize) {
        let ((start_line, start_col), _) = self.text_edit_buffer_range(main_edit);
        let mut position =
            Self::text_position_after_insert(start_line, start_col, &main_edit.new_text);

        let mut ordered_edits: Vec<&TextEdit> = additional_edits.iter().collect();
        ordered_edits.sort_by(|a, b| {
            let (a_start, _) = self.text_edit_buffer_range(a);
            let (b_start, _) = self.text_edit_buffer_range(b);
            a_start.cmp(&b_start)
        });

        for edit in ordered_edits {
            position = self.position_after_text_edit(position, edit);
        }

        position
    }

    /// Apply LSP completion edits, including auto-import companion edits.
    /// Returns the main inserted text when the completion had a server edit.
    pub fn apply_completion_item_edits(&mut self, item: &CompletionItem) -> Option<String> {
        let main_edit = item.text_edit.as_ref()?;
        let cursor_after =
            self.completion_cursor_after_edit(main_edit, &item.additional_text_edits);
        let mut edits = item.additional_text_edits.clone();
        edits.push(main_edit.clone());
        self.apply_text_edits(&edits);
        self.cursor.line = cursor_after.0;
        self.cursor.col = cursor_after.1;
        self.clamp_cursor();
        self.scroll_to_cursor();
        Some(main_edit.new_text.clone())
    }

    fn lsp_utf16_col_to_buffer_col(&self, line: usize, utf16_col: usize) -> usize {
        Self::lsp_utf16_col_to_buffer_col_in_buffer(
            &self.buffers[self.current_buffer_idx],
            line,
            utf16_col,
        )
    }

    fn diagnostic_lsp_to_buffer_cols(buffer: &Buffer, mut diag: Diagnostic) -> Diagnostic {
        diag.col_start =
            Self::lsp_utf16_col_to_buffer_col_in_buffer(buffer, diag.line, diag.col_start);
        diag.col_end =
            Self::lsp_utf16_col_to_buffer_col_in_buffer(buffer, diag.end_line, diag.col_end);
        diag
    }

    fn lsp_utf16_col_to_buffer_col_in_buffer(
        buffer: &Buffer,
        line: usize,
        utf16_col: usize,
    ) -> usize {
        let Some(line) = buffer.line(line) else {
            return 0;
        };
        let line_text = line.to_string();
        let line_text = line_text.trim_end_matches('\n');
        crate::copilot::utf16_to_utf8_col(line_text, utf16_col as u32)
    }

    /// Get the URI for the current buffer (cached for performance during render)
    /// This avoids repeated string allocations when checking diagnostics
    pub fn current_buffer_uri(&self) -> Option<String> {
        self.buffer().path.as_ref().map(crate::lsp::path_to_uri)
    }

    /// Get diagnostics for the current buffer
    pub fn current_diagnostics(&self) -> &[Diagnostic] {
        if let Some(path) = &self.buffer().path {
            let uri = crate::lsp::path_to_uri(path);
            self.diagnostics
                .get(&uri)
                .map(|v| v.as_slice())
                .unwrap_or(&[])
        } else {
            &[]
        }
    }

    /// Get diagnostics for the current buffer using a pre-computed URI (avoids repeated allocations)
    pub fn current_diagnostics_cached(&self, uri: &str) -> &[Diagnostic] {
        self.diagnostics
            .get(uri)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get diagnostics for a specific line in the current buffer
    /// Handles multi-line diagnostics by checking if line falls within range
    pub fn diagnostics_for_line(&self, line: usize) -> Vec<&Diagnostic> {
        self.current_diagnostics()
            .iter()
            .filter(|d| line >= d.line && line <= d.end_line)
            .collect()
    }

    /// Get diagnostics for a line using a pre-computed URI (avoids repeated allocations during render)
    pub fn diagnostics_for_line_cached<'a>(
        &'a self,
        line: usize,
        uri: &str,
    ) -> Vec<&'a Diagnostic> {
        self.current_diagnostics_cached(uri)
            .iter()
            .filter(|d| line >= d.line && line <= d.end_line)
            .collect()
    }

    /// Get the first diagnostic message for the cursor line (for status display)
    pub fn diagnostic_at_cursor(&self) -> Option<&Diagnostic> {
        self.diagnostics_for_line(self.cursor.line)
            .into_iter()
            .next()
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
        self.refresh_explorer_git_statuses();
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
    pub fn git_status_for_line_in_file(
        &self,
        path: &std::path::Path,
        line: usize,
    ) -> Option<crate::git::GitLineStatus> {
        let path_str = path.to_string_lossy().to_string();
        let diff = self.git_diffs.get(&path_str)?;
        diff.status_for_line(line)
    }

    /// Update git diff for the current buffer
    pub fn update_git_diff(&mut self) {
        let Some(repo) = &self.git_repo else { return };

        if let Some((path, diff)) = Self::git_diff_for_buffer(repo, self.buffer()) {
            self.set_git_diff(path, diff);
        } else if let Some(path) = self.buffer().path.as_ref() {
            // File not tracked by git or new file - clear any existing diff
            self.git_diffs.remove(&path.to_string_lossy().to_string());
        }
    }

    /// Update git diffs for every open buffer.
    pub fn update_all_git_diffs(&mut self) {
        let Some(repo) = &self.git_repo else {
            self.git_diffs.clear();
            return;
        };

        let mut git_diffs = HashMap::new();
        for buffer in &self.buffers {
            if let Some((path, diff)) = Self::git_diff_for_buffer(repo, buffer) {
                git_diffs.insert(path, diff);
            }
        }
        self.git_diffs = git_diffs;
    }

    fn git_diff_for_buffer(
        repo: &crate::git::GitRepo,
        buffer: &Buffer,
    ) -> Option<(String, crate::git::GitDiff)> {
        let path = buffer.path.as_ref()?;
        let head_content = repo.head_content(path)?;
        let current_content = buffer.content();
        let diff = crate::git::compute_diff(&head_content, &current_content);
        Some((path.to_string_lossy().to_string(), diff))
    }

    /// Refresh all git-derived editor state.
    pub fn refresh_git_state(&mut self) {
        self.update_all_git_diffs();
        self.refresh_explorer_git_statuses();
    }

    /// Refresh git-backed explorer markers.
    pub fn refresh_explorer_git_statuses(&mut self) {
        if let Some(repo) = &self.git_repo {
            let mut statuses = repo.file_statuses();
            for buffer in &self.buffers {
                if buffer.dirty {
                    if let Some(path) = &buffer.path {
                        statuses.insert(path.clone(), crate::git::GitFileStatus::Modified);
                    }
                }
            }
            self.explorer.rebuild_git_statuses_from(statuses);
        } else {
            self.explorer.clear_git_statuses();
        }
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

    fn save_current_undo_stack(&mut self) {
        if let Some(stack) = self.undo_stacks.get_mut(self.current_buffer_idx) {
            *stack = self.undo_stack.clone();
        }
    }

    fn load_current_undo_stack(&mut self) {
        self.undo_stack = self
            .undo_stacks
            .get(self.current_buffer_idx)
            .cloned()
            .unwrap_or_else(UndoStack::new);
    }

    fn reset_current_undo_stack(&mut self) {
        self.undo_stack.clear();
        if let Some(stack) = self.undo_stacks.get_mut(self.current_buffer_idx) {
            stack.clear();
        }
    }

    fn reset_undo_stack_for_buffer(&mut self, idx: usize) {
        if idx == self.current_buffer_idx {
            self.reset_current_undo_stack();
        } else if let Some(stack) = self.undo_stacks.get_mut(idx) {
            stack.clear();
        }
    }

    /// Replace the entire buffer content (used by external formatters)
    pub fn replace_buffer_content(&mut self, content: &str) {
        // Save cursor position
        let cursor_line = self.cursor.line;
        let cursor_col = self.cursor.col;

        // Replace content
        self.buffer_mut().set_content(content);
        self.reset_current_undo_stack();

        // Try to restore cursor position (clamp to valid range)
        let max_line = self.buffer().len_lines().saturating_sub(1);
        self.cursor.line = cursor_line.min(max_line);
        let max_col = self.buffer().line_len(self.cursor.line);
        self.cursor.col = cursor_col.min(max_col);

        // Increment syntax version to trigger re-parse
        self.last_syntax_version = 0;
    }

    /// Replace the current buffer content as a single undoable change.
    pub fn replace_buffer_content_with_undo(&mut self, content: &str) {
        let old_content = self.buffer().content();
        if old_content == content {
            return;
        }

        let cursor_line = self.cursor.line;
        let cursor_col = self.cursor.col;
        self.undo_stack.end_undo_group(cursor_line, cursor_col);
        self.undo_stack.begin_undo_group(cursor_line, cursor_col);
        self.undo_stack
            .record_change(Change::new(0, 0, old_content, content.to_string()));

        self.buffer_mut().set_content(content);

        let max_line = self.buffer().len_lines().saturating_sub(1);
        self.cursor.line = cursor_line.min(max_line);
        let max_col = self.buffer().line_len(self.cursor.line);
        self.cursor.col = cursor_col.min(max_col);

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        self.save_current_undo_stack();
        self.last_syntax_version = 0;
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
            let next_idx = (self.current_buffer_idx + 1) % self.buffers.len();
            self.switch_to_buffer(next_idx);
        }
    }

    /// Switch to the previous buffer
    pub fn prev_buffer(&mut self) {
        if self.buffers.len() > 1 {
            let prev_idx = if self.current_buffer_idx == 0 {
                self.buffers.len() - 1
            } else {
                self.current_buffer_idx - 1
            };
            self.switch_to_buffer(prev_idx);
        }
    }

    fn remember_current_file_as_alternate(&mut self) {
        self.alternate_file_path = self.buffers[self.current_buffer_idx].path.clone();
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
        self.save_current_undo_stack();
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].cursor = self.cursor;
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
            self.panes[self.active_pane].h_offset = self.h_offset;
            self.panes[self.active_pane].buffer_idx = self.current_buffer_idx;
        }
    }

    /// Load pane state when switching to it
    fn load_pane_state(&mut self) {
        if self.active_pane < self.panes.len() {
            self.cursor = self.panes[self.active_pane].cursor;
            self.viewport_offset = self.panes[self.active_pane].viewport_offset;
            self.h_offset = self.panes[self.active_pane].h_offset;
            self.current_buffer_idx = self.panes[self.active_pane].buffer_idx;
            self.load_current_undo_stack();
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

    fn split_pane(
        &mut self,
        file_path: Option<std::path::PathBuf>,
        layout: SplitLayout,
    ) -> anyhow::Result<()> {
        // Save current pane state
        self.save_pane_state();
        self.split_layout = layout;

        // Determine which buffer the new pane shows
        let new_buffer_idx = if let Some(path) = file_path {
            // Open file in new buffer
            let new_buffer = Buffer::from_file(path)?;
            self.buffers.push(new_buffer);
            self.undo_stacks.push(UndoStack::new());
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

        self.set_status(format!(
            "Pane {}/{}",
            self.active_pane + 1,
            self.panes.len()
        ));
        Ok(())
    }

    /// Switch to the next pane
    pub fn next_pane(&mut self) {
        if self.panes.len() > 1 {
            self.save_pane_state();
            self.active_pane = (self.active_pane + 1) % self.panes.len();
            self.load_pane_state();
            self.set_status(format!(
                "Pane {}/{}",
                self.active_pane + 1,
                self.panes.len()
            ));
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
            self.set_status(format!(
                "Pane {}/{}",
                self.active_pane + 1,
                self.panes.len()
            ));
        }
    }

    /// Close the current pane
    pub fn close_pane(&mut self) -> bool {
        if self.panes.len() > 1 {
            self.save_current_undo_stack();
            self.panes.remove(self.active_pane);
            if self.active_pane >= self.panes.len() {
                self.active_pane = self.panes.len() - 1;
            }
            self.update_pane_rects();
            self.load_pane_state();
            self.set_status(format!(
                "Pane {}/{}",
                self.active_pane + 1,
                self.panes.len()
            ));
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

    /// Make all pane rectangles equal according to the current split layout.
    pub fn equalize_windows(&mut self) {
        self.update_pane_rects();
        self.set_status("Windows equalized");
    }

    /// Rotate pane order down/right according to the current split layout.
    pub fn rotate_windows_down_right(&mut self) {
        self.rotate_windows(true);
    }

    /// Rotate pane order up/left according to the current split layout.
    pub fn rotate_windows_up_left(&mut self) {
        self.rotate_windows(false);
    }

    fn rotate_windows(&mut self, down_right: bool) {
        let pane_count = self.panes.len();
        if pane_count <= 1 {
            self.set_status("Only one window");
            return;
        }

        self.save_pane_state();
        let old_active = self.active_pane;
        if down_right {
            self.panes.rotate_right(1);
            self.active_pane = (old_active + 1) % pane_count;
            self.set_status("Windows rotated");
        } else {
            self.panes.rotate_left(1);
            self.active_pane = if old_active == 0 {
                pane_count - 1
            } else {
                old_active - 1
            };
            self.set_status("Windows rotated reverse");
        }
        self.update_pane_rects();
        self.load_pane_state();
    }

    /// Exchange the current pane with the next pane, or previous if current is last.
    pub fn exchange_window_with_next(&mut self) {
        let pane_count = self.panes.len();
        if pane_count <= 1 {
            self.set_status("Only one window");
            return;
        }

        self.save_pane_state();
        let old_active = self.active_pane;
        let target = if old_active + 1 < pane_count {
            old_active + 1
        } else {
            old_active - 1
        };

        self.panes.swap(old_active, target);
        self.active_pane = target;
        self.update_pane_rects();
        self.load_pane_state();
        self.set_status("Windows exchanged");
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
                PaneDirection::Left | PaneDirection::Right => current_center_x.abs_diff(center_x),
                PaneDirection::Up | PaneDirection::Down => current_center_y.abs_diff(center_y),
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
            self.set_status(format!(
                "Pane {}/{}",
                self.active_pane + 1,
                self.panes.len()
            ));
        } else if direction == PaneDirection::Left && self.explorer.visible {
            // If moving left and no pane found, focus the explorer
            self.focus_explorer();
        }
    }

    /// Open a file in the editor (replaces current buffer or adds new one)
    pub fn open_file(&mut self, path: std::path::PathBuf) -> anyhow::Result<()> {
        // Check if file is already open in an existing buffer
        let canonical_path = path.canonicalize().ok();
        if let Some(existing_idx) = self
            .buffers
            .iter()
            .position(|b| b.path.as_ref().and_then(|p| p.canonicalize().ok()) == canonical_path)
        {
            // File already open, switch to that buffer
            if existing_idx != self.current_buffer_idx {
                self.remember_current_file_as_alternate();
            }
            self.save_pane_state();
            self.current_buffer_idx = existing_idx;
            self.load_current_undo_stack();
            self.cursor = Cursor::default();
            self.viewport_offset = 0;
            self.h_offset = 0;
            // Sync active pane state
            if self.active_pane < self.panes.len() {
                self.panes[self.active_pane].buffer_idx = existing_idx;
                self.panes[self.active_pane].cursor = self.cursor;
                self.panes[self.active_pane].viewport_offset = self.viewport_offset;
                self.panes[self.active_pane].h_offset = self.h_offset;
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
            self.reset_current_undo_stack();
            // Update active pane's buffer_idx (it's already pointing to current_buffer_idx)
        } else {
            self.remember_current_file_as_alternate();
            self.save_pane_state();
            self.buffers.push(new_buffer);
            self.undo_stacks.push(UndoStack::new());
            self.current_buffer_idx = self.buffers.len() - 1;
            self.load_current_undo_stack();
            // Update active pane to point to the new buffer
            if self.active_pane < self.panes.len() {
                self.panes[self.active_pane].buffer_idx = self.current_buffer_idx;
            }
        }

        self.cursor = Cursor::default();
        self.viewport_offset = 0;
        self.h_offset = 0;
        self.reset_current_undo_stack();

        // Sync active pane's cursor and viewport
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].cursor = self.cursor;
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
            self.panes[self.active_pane].h_offset = self.h_offset;
        }

        // Parse the buffer for syntax highlighting
        self.parse_current_buffer();

        // Update git diff for the newly opened file
        self.update_git_diff();

        Ok(())
    }

    /// Close the current buffer
    pub fn close_current_buffer(&mut self) {
        self.save_current_undo_stack();
        let removed_idx = self.current_buffer_idx;

        if self.buffers.len() <= 1 {
            // If it's the last buffer, just create a new empty one
            self.buffers[0] = Buffer::new();
            self.undo_stacks = vec![UndoStack::new()];
            self.current_buffer_idx = 0;
            self.cursor = Cursor::default();
            self.viewport_offset = 0;
            self.h_offset = 0;
            self.load_current_undo_stack();
            for pane in &mut self.panes {
                pane.buffer_idx = 0;
                pane.cursor = Cursor::default();
                pane.viewport_offset = 0;
                pane.h_offset = 0;
            }
        } else {
            // Remove the current buffer
            self.buffers.remove(removed_idx);
            self.undo_stacks.remove(removed_idx);

            // Adjust current_buffer_idx if needed
            if self.current_buffer_idx >= self.buffers.len() {
                self.current_buffer_idx = self.buffers.len() - 1;
            }

            // Keep every pane pointing at a valid buffer after the Vec index shift.
            for pane in &mut self.panes {
                if pane.buffer_idx == removed_idx {
                    pane.buffer_idx = self.current_buffer_idx;
                    pane.cursor = Cursor::default();
                    pane.viewport_offset = 0;
                    pane.h_offset = 0;
                } else if pane.buffer_idx > removed_idx {
                    pane.buffer_idx -= 1;
                }
            }

            // Reset cursor state
            self.cursor = Cursor::default();
            self.viewport_offset = 0;
            self.h_offset = 0;
            self.load_current_undo_stack();
        }

        // Sync pane state
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].cursor = self.cursor;
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
            self.panes[self.active_pane].h_offset = self.h_offset;
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
        if let Some(preview) = &mut self.markdown_preview {
            preview.reflow(crate::markdown_preview::preview_content_width(width));
        }
        self.update_pane_rects();
        self.sync_floating_terminal_size();
    }

    /// Keep the PTY dimensions aligned with the floating terminal content area.
    pub fn sync_floating_terminal_size(&mut self) {
        let terminal = &self.settings.terminal;
        let (rows, cols) = crate::floating_terminal::content_size_for_screen(
            self.term_width,
            self.term_height,
            terminal.popup_width_ratio,
            terminal.popup_height_ratio,
        );
        self.floating_terminal.resize(rows, cols);
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
        let max_line = self.buffers[self.current_buffer_idx]
            .len_lines()
            .saturating_sub(1);
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

    fn enter_insert_mode_at_change(&mut self, line: usize, col: usize) {
        self.mode = Mode::Insert;
        self.begin_insert_session();
        self.cursor.line = line.min(
            self.buffers[self.current_buffer_idx]
                .len_lines()
                .saturating_sub(1),
        );
        self.cursor.col = col.min(self.buffers[self.current_buffer_idx].line_len(self.cursor.line));
        self.last_insert_position = Some((self.cursor.line, self.cursor.col));
        self.clamp_cursor();
        self.scroll_to_cursor();
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

        // Horizontal scrolling (only in non-wrap mode)
        if !self.settings.editor.wrap {
            let text_area_width = self.text_area_width();
            if text_area_width > 0 {
                // Scroll right if cursor is past visible area
                if self.cursor.col >= self.h_offset + text_area_width {
                    self.h_offset = self.cursor.col - text_area_width + 1;
                }
                // Scroll left if cursor is before visible area
                if self.cursor.col < self.h_offset {
                    self.h_offset = self.cursor.col;
                }
            }
        }

        // Sync to active pane
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].viewport_offset = self.viewport_offset;
            self.panes[self.active_pane].h_offset = self.h_offset;
            self.panes[self.active_pane].cursor = self.cursor;
        }
    }

    /// Calculate the text area width (columns available for text) for the active pane
    fn text_area_width(&self) -> usize {
        let pane_width = if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].rect.width as usize
        } else {
            self.term_width as usize
        };
        const SIGN_COLUMN_WIDTH: usize = 2;
        let line_num_width = self.buffer().len_lines().to_string().len().max(3);
        if self.settings.editor.line_numbers {
            pane_width.saturating_sub(SIGN_COLUMN_WIDTH + line_num_width + 1)
        } else {
            pane_width.saturating_sub(SIGN_COLUMN_WIDTH)
        }
    }

    fn effective_wrap_width(&self) -> usize {
        self.settings.editor.wrap_width.min(self.text_area_width())
    }

    fn display_char_width(ch: char, tab_width: usize) -> usize {
        if ch == '\t' {
            tab_width.max(1)
        } else if ch.is_control() {
            1
        } else {
            UnicodeWidthChar::width(ch).unwrap_or(0)
        }
    }

    fn display_width_between_cols(
        line: &str,
        start_col: usize,
        end_col: usize,
        tab_width: usize,
    ) -> usize {
        if end_col <= start_col {
            return 0;
        }

        line.chars()
            .enumerate()
            .skip(start_col)
            .take(end_col - start_col)
            .take_while(|(_, ch)| *ch != '\n')
            .map(|(_, ch)| Self::display_char_width(ch, tab_width))
            .sum()
    }

    fn display_line_segments(
        line: &str,
        max_width: usize,
        tab_width: usize,
    ) -> Vec<DisplayLineSegment> {
        let line = line.trim_end_matches('\n');
        let chars: Vec<char> = line.chars().collect();

        if chars.is_empty() {
            return vec![DisplayLineSegment {
                start_col: 0,
                end_col: 0,
                indent_width: 0,
            }];
        }

        if max_width == 0
            || Self::display_width_between_cols(line, 0, chars.len(), tab_width) <= max_width
        {
            return vec![DisplayLineSegment {
                start_col: 0,
                end_col: chars.len(),
                indent_width: 0,
            }];
        }

        let mut indent_width = 0;
        for ch in chars.iter().take_while(|c| c.is_whitespace()) {
            let ch_width = Self::display_char_width(*ch, tab_width);
            if indent_width + ch_width >= max_width {
                break;
            }
            indent_width += ch_width;
        }

        let mut segments = Vec::new();
        let mut current_col = 0;
        let mut is_first = true;

        while current_col < chars.len() {
            let segment_indent_width = if is_first { 0 } else { indent_width };
            let available_width = if is_first {
                max_width
            } else {
                max_width.saturating_sub(indent_width)
            };

            let mut take_count = 0;
            let mut segment_width = 0;

            if available_width == 0 {
                take_count = 1;
            } else {
                while current_col + take_count < chars.len() {
                    let ch = chars[current_col + take_count];
                    let ch_width = Self::display_char_width(ch, tab_width);
                    if segment_width + ch_width > available_width {
                        if take_count == 0 {
                            take_count = 1;
                        }
                        break;
                    }
                    segment_width += ch_width;
                    take_count += 1;
                }
            }

            segments.push(DisplayLineSegment {
                start_col: current_col,
                end_col: (current_col + take_count).min(chars.len()),
                indent_width: segment_indent_width,
            });
            current_col += take_count;
            is_first = false;
        }

        segments
    }

    fn display_segment_for_col(
        line: &str,
        segments: &[DisplayLineSegment],
        col: usize,
        tab_width: usize,
    ) -> (usize, usize) {
        let line_len = line.trim_end_matches('\n').chars().count();
        if line_len == 0 {
            return (0, 0);
        }

        let target_col = col.min(line_len.saturating_sub(1));
        for (idx, segment) in segments.iter().enumerate() {
            if target_col >= segment.start_col && target_col < segment.end_col {
                let display_col = segment.indent_width
                    + Self::display_width_between_cols(
                        line,
                        segment.start_col,
                        target_col,
                        tab_width,
                    );
                return (idx, display_col);
            }
        }

        let last_idx = segments.len().saturating_sub(1);
        let last = &segments[last_idx];
        let display_col = last.indent_width
            + Self::display_width_between_cols(line, last.start_col, target_col, tab_width);
        (last_idx, display_col)
    }

    fn display_col_to_buffer_col(
        line: &str,
        segment: DisplayLineSegment,
        desired_display_col: usize,
        tab_width: usize,
    ) -> usize {
        if segment.end_col <= segment.start_col {
            return segment.start_col;
        }

        let desired = desired_display_col.saturating_sub(segment.indent_width);
        let mut width = 0;
        for (idx, ch) in line
            .chars()
            .enumerate()
            .skip(segment.start_col)
            .take(segment.end_col - segment.start_col)
        {
            if width >= desired {
                return idx;
            }
            let ch_width = Self::display_char_width(ch, tab_width);
            if width + ch_width > desired {
                return idx;
            }
            width += ch_width;
        }

        segment.end_col.saturating_sub(1)
    }

    /// Get the text in a range (for yank/delete operations)
    pub fn get_range_text(
        &self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) -> String {
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
    pub fn delete_range(
        &mut self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) -> String {
        let text = self.get_range_text(start_line, start_col, end_line, end_col);

        // Delete from buffer (end to start to preserve positions)
        self.buffers[self.current_buffer_idx].delete_range(
            start_line,
            start_col,
            end_line,
            end_col + 1,
        );

        // Move cursor to start of deleted range
        self.cursor.line = start_line;
        self.cursor.col = start_col;
        self.clamp_cursor();
        self.scroll_to_cursor();

        text
    }

    /// Delete lines and return them
    pub fn delete_lines(&mut self, start_line: usize, count: usize) -> String {
        let end_line = (start_line + count - 1).min(
            self.buffers[self.current_buffer_idx]
                .len_lines()
                .saturating_sub(1),
        );
        let text = self.get_lines_text(start_line, end_line);

        // Delete from start of first line to end of last line (including newline)
        let end_col = self.buffers[self.current_buffer_idx].line_len_including_newline(end_line);
        self.buffers[self.current_buffer_idx].delete_range(start_line, 0, end_line, end_col);

        // Position cursor
        self.cursor.line = start_line.min(
            self.buffers[self.current_buffer_idx]
                .len_lines()
                .saturating_sub(1),
        );
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
            self.begin_change();
            self.undo_stack
                .record_change(Change::delete(start_line, start_col, text.clone()));

            let deleted = self.delete_range(start_line, start_col, end_line, end_col);

            self.undo_stack
                .end_undo_group(self.cursor.line, self.cursor.col);

            let is_small = !deleted.contains('\n');
            self.registers
                .delete(register, RegisterContent::Chars(deleted), is_small);
        }
    }

    /// Delete count lines (dd operation)
    pub fn delete_line(&mut self, count: usize, register: Option<char>) {
        let start_line = self.cursor.line;
        let end_line = (start_line + count - 1).min(
            self.buffers[self.current_buffer_idx]
                .len_lines()
                .saturating_sub(1),
        );
        let text = self.get_lines_text(start_line, end_line);

        // Record for undo
        self.begin_change();
        self.undo_stack
            .record_change(Change::delete(start_line, 0, text.clone()));

        let deleted = self.delete_lines(self.cursor.line, count);

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);

        self.registers
            .delete(register, RegisterContent::Lines(deleted), false);
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
        let end_line = (self.cursor.line + count - 1).min(
            self.buffers[self.current_buffer_idx]
                .len_lines()
                .saturating_sub(1),
        );
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
            self.begin_change();
            self.undo_stack
                .record_change(Change::delete(start_line, start_col, text.clone()));

            let deleted = self.delete_range(start_line, start_col, end_line, end_col);

            let is_small = !deleted.contains('\n');
            self.registers
                .delete(register, RegisterContent::Chars(deleted), is_small);

            // Enter insert mode (don't start new undo group, reuse the one from change)
            self.enter_insert_mode_at_change(start_line, start_col);
        }
    }

    /// Substitute count characters under the cursor and enter insert mode.
    pub fn substitute_chars_at(&mut self, count: usize, register: Option<char>) {
        let line = self.cursor.line;
        let line_len = self.buffers[self.current_buffer_idx].line_len(line);
        let start_col = self.cursor.col.min(line_len.saturating_sub(1));

        self.begin_change();

        if line_len > 0 {
            let end_col = (start_col + count.max(1) - 1).min(line_len.saturating_sub(1));
            let deleted = self.get_range_text(line, start_col, line, end_col);

            if !deleted.is_empty() {
                self.undo_stack
                    .record_change(Change::delete(line, start_col, deleted.clone()));
                self.buffers[self.current_buffer_idx].delete_range(
                    line,
                    start_col,
                    line,
                    end_col + 1,
                );
                self.registers
                    .delete(register, RegisterContent::Chars(deleted), true);
            }
        }

        self.enter_insert_mode_at_change(line, start_col);
    }

    /// Change count lines (cc operation)
    pub fn change_line(&mut self, count: usize, register: Option<char>) {
        let end_line = (self.cursor.line + count - 1).min(
            self.buffers[self.current_buffer_idx]
                .len_lines()
                .saturating_sub(1),
        );

        // For cc, we delete the content but keep the line structure
        // Get the text that will be deleted for undo
        let text = self.get_lines_text(self.cursor.line, end_line);

        // Begin undo group (will include the delete and subsequent inserts)
        self.begin_change();
        self.undo_stack
            .record_change(Change::delete(self.cursor.line, 0, text.clone()));

        self.registers
            .delete(register, RegisterContent::Lines(text), false);

        // Delete all lines except keep one empty line
        for _ in 0..count.saturating_sub(1) {
            if self.cursor.line < self.buffers[self.current_buffer_idx].len_lines() - 1 {
                self.delete_lines(self.cursor.line + 1, 1);
            }
        }

        // Clear current line content (keep the newline)
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        if line_len > 0 {
            self.buffers[self.current_buffer_idx].delete_range(
                self.cursor.line,
                0,
                self.cursor.line,
                line_len,
            );
        }

        // Enter insert mode (don't start new undo group, reuse the one from change)
        self.enter_insert_mode_at_change(self.cursor.line, 0);
    }

    /// Paste after cursor from a register
    pub fn paste_after(&mut self, register: Option<char>) {
        self.paste_after_with_cursor_after(register, false);
    }

    /// Paste after cursor and leave cursor after the pasted text.
    pub fn paste_after_move(&mut self, register: Option<char>) {
        self.paste_after_with_cursor_after(register, true);
    }

    fn register_content_for_paste(&mut self, register: Option<char>) -> Option<RegisterContent> {
        match register {
            Some('%') => self
                .buffer()
                .path
                .as_ref()
                .map(|path| RegisterContent::Chars(path.to_string_lossy().to_string())),
            Some(':') => self
                .command_line
                .history
                .last()
                .cloned()
                .map(RegisterContent::Chars),
            Some('#') => self
                .alternate_file_path
                .as_ref()
                .map(|path| RegisterContent::Chars(path.to_string_lossy().to_string())),
            Some('.') => self
                .last_inserted_text
                .as_ref()
                .filter(|text| !text.is_empty())
                .cloned()
                .map(RegisterContent::Chars),
            Some('=') => self
                .expression_register_value
                .as_ref()
                .filter(|text| !text.is_empty())
                .cloned()
                .map(RegisterContent::Chars),
            _ => self.registers.get_content(register),
        }
    }

    fn paste_after_with_cursor_after(&mut self, register: Option<char>, cursor_after: bool) {
        if let Some(content) = self.register_content_for_paste(register) {
            self.begin_change();

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

                    self.buffers[self.current_buffer_idx].insert_char(
                        self.cursor.line,
                        line_len,
                        '\n',
                    );
                    self.cursor.line += 1;
                    self.cursor.col = 0;
                    let first_inserted_line = self.cursor.line;
                    let inserted_line_count = Self::linewise_paste_line_count(trimmed);

                    // Insert the lines (without trailing newline if present)
                    self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, 0, trimmed);
                    if cursor_after {
                        self.cursor.line = (first_inserted_line + inserted_line_count).min(
                            self.buffers[self.current_buffer_idx]
                                .len_lines()
                                .saturating_sub(1),
                        );
                        self.cursor.col = 0;
                    }

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

                    self.buffers[self.current_buffer_idx].insert_str(
                        self.cursor.line,
                        insert_col,
                        &text,
                    );
                    let pasted_len = text.chars().count();
                    self.cursor.col = insert_col
                        + if cursor_after {
                            pasted_len
                        } else {
                            pasted_len.saturating_sub(1)
                        };
                    self.clamp_cursor();
                }
            }

            self.undo_stack
                .end_undo_group(self.cursor.line, self.cursor.col);
        }
        self.check_clipboard_error();
    }

    /// Paste after cursor count times.
    pub fn paste_after_count(&mut self, register: Option<char>, count: usize) {
        for _ in 0..count.max(1) {
            self.paste_after(register);
        }
    }

    /// Paste after cursor count times and leave cursor after the pasted text.
    pub fn paste_after_move_count(&mut self, register: Option<char>, count: usize) {
        for _ in 0..count.max(1) {
            self.paste_after_move(register);
        }
    }

    /// Paste before cursor from a register
    pub fn paste_before(&mut self, register: Option<char>) {
        self.paste_before_with_cursor_after(register, false);
    }

    /// Paste before cursor and leave cursor after the pasted text.
    pub fn paste_before_move(&mut self, register: Option<char>) {
        self.paste_before_with_cursor_after(register, true);
    }

    fn paste_before_with_cursor_after(&mut self, register: Option<char>, cursor_after: bool) {
        if let Some(content) = self.register_content_for_paste(register) {
            self.begin_change();

            match content {
                RegisterContent::Lines(text) => {
                    // Paste on new line above
                    let insert_line = self.cursor.line;
                    let inserted_line_count = Self::linewise_paste_line_count(&text);
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
                        self.buffers[self.current_buffer_idx].insert_char(
                            self.cursor.line,
                            text.len(),
                            '\n',
                        );
                    }
                    self.cursor.col = 0;
                    if cursor_after {
                        self.cursor.line = (insert_line + inserted_line_count).min(
                            self.buffers[self.current_buffer_idx]
                                .len_lines()
                                .saturating_sub(1),
                        );
                    }
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
                    self.buffers[self.current_buffer_idx].insert_str(
                        self.cursor.line,
                        self.cursor.col,
                        &text,
                    );
                    let pasted_len = text.chars().count();
                    self.cursor.col = self.cursor.col
                        + if cursor_after {
                            pasted_len
                        } else {
                            pasted_len.saturating_sub(1)
                        };
                    self.clamp_cursor();
                }
            }

            self.undo_stack
                .end_undo_group(self.cursor.line, self.cursor.col);
        }
        self.check_clipboard_error();
    }

    /// Paste before cursor count times.
    pub fn paste_before_count(&mut self, register: Option<char>, count: usize) {
        for _ in 0..count.max(1) {
            self.paste_before(register);
        }
    }

    /// Paste before cursor count times and leave cursor after the pasted text.
    pub fn paste_before_move_count(&mut self, register: Option<char>, count: usize) {
        for _ in 0..count.max(1) {
            self.paste_before_move(register);
        }
    }

    fn linewise_paste_line_count(text: &str) -> usize {
        text.trim_end_matches('\n').split('\n').count().max(1)
    }

    /// Enter insert mode
    pub fn enter_insert_mode(&mut self) {
        self.last_insert_position = Some((self.cursor.line, self.cursor.col));
        self.mode = Mode::Insert;
        self.begin_insert_session();
        self.begin_change();
    }

    /// Enter insert mode after cursor
    pub fn enter_insert_mode_append(&mut self) {
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        if line_len > 0 {
            self.cursor.col = (self.cursor.col + 1).min(line_len);
        }
        self.last_insert_position = Some((self.cursor.line, self.cursor.col));
        self.mode = Mode::Insert;
        self.begin_insert_session();
        self.begin_change();
    }

    /// Enter insert mode at end of line
    pub fn enter_insert_mode_end(&mut self) {
        self.cursor.col = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        self.last_insert_position = Some((self.cursor.line, self.cursor.col));
        self.mode = Mode::Insert;
        self.begin_insert_session();
        self.begin_change();
    }

    /// Enter insert mode at start of line (first non-blank)
    pub fn enter_insert_mode_start(&mut self) {
        // Find first non-blank character
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        for col in 0..line_len {
            if let Some(ch) = self.buffers[self.current_buffer_idx].char_at(self.cursor.line, col) {
                if !ch.is_whitespace() {
                    self.cursor.col = col;
                    self.last_insert_position = Some((self.cursor.line, self.cursor.col));
                    self.mode = Mode::Insert;
                    self.begin_insert_session();
                    self.begin_change();
                    return;
                }
            }
        }
        self.cursor.col = 0;
        self.last_insert_position = Some((self.cursor.line, self.cursor.col));
        self.mode = Mode::Insert;
        self.begin_insert_session();
        self.begin_change();
    }

    /// Temporarily leave insert mode so the next normal command can run.
    pub fn enter_insert_normal_once(&mut self) {
        self.pending_insert_register = false;
        self.pending_insert_normal_once = true;
        self.enter_normal_mode();
    }

    /// Return to insert mode after the one-shot normal command has completed.
    pub fn finish_insert_normal_once(&mut self) {
        self.pending_insert_normal_once = false;
        self.last_insert_position = Some((self.cursor.line, self.cursor.col));
        self.mode = Mode::Insert;
        self.begin_insert_session();
        self.begin_change();
        self.clamp_cursor();
        self.scroll_to_cursor();
    }

    fn begin_insert_session(&mut self) {
        self.current_inserted_text.clear();
    }

    fn finish_insert_session(&mut self) {
        if !self.current_inserted_text.is_empty() {
            self.last_inserted_text = Some(std::mem::take(&mut self.current_inserted_text));
        } else {
            self.current_inserted_text.clear();
        }
    }

    fn record_inserted_text(&mut self, text: &str) {
        if self.mode == Mode::Insert {
            self.current_inserted_text.push_str(text);
        }
    }

    fn record_inserted_char(&mut self, ch: char) {
        if self.mode == Mode::Insert {
            self.current_inserted_text.push(ch);
        }
    }

    fn insert_text_at_cursor(&mut self, text: &str) {
        let start_line = self.cursor.line;
        let start_col = self.cursor.col;
        self.undo_stack
            .record_change(Change::insert(start_line, start_col, text.to_string()));
        self.buffers[self.current_buffer_idx].insert_str(start_line, start_col, text);
        self.record_inserted_text(text);
        let (end_line, end_col) = Self::text_position_after_insert(start_line, start_col, text);
        self.cursor.line = end_line;
        self.cursor.col = end_col;
        self.scroll_to_cursor();
    }

    /// Insert text from a register at the cursor while in insert mode.
    pub fn insert_register_text(&mut self, register: char) -> bool {
        let Some(content) = self.register_content_for_paste(Some(register)) else {
            return false;
        };
        let text = match content {
            RegisterContent::Chars(text) | RegisterContent::Lines(text) => text,
        };
        if text.is_empty() {
            return false;
        }

        self.insert_text_at_cursor(&text);
        true
    }

    /// Insert text from a register at the command-line cursor.
    pub fn insert_register_text_into_command_line(&mut self, register: char) -> bool {
        let Some(content) = self.register_content_for_paste(Some(register)) else {
            return false;
        };
        let text = match content {
            RegisterContent::Chars(text) | RegisterContent::Lines(text) => text,
        };
        if text.is_empty() {
            return false;
        }

        self.command_line.insert_text(&text);
        true
    }

    /// Begin collecting expression-register input.
    pub fn start_expression_register(&mut self, target: ExpressionRegisterTarget) {
        self.pending_expression_register = Some(target);
        self.expression_register_input.clear();
        self.set_status("=");
    }

    /// Append one typed character to the expression-register prompt.
    pub fn push_expression_register_char(&mut self, ch: char) {
        self.expression_register_input.push(ch);
        self.set_status(format!("={}", self.expression_register_input));
    }

    /// Remove the last character from the expression-register prompt.
    pub fn pop_expression_register_char(&mut self) {
        self.expression_register_input.pop();
        self.set_status(format!("={}", self.expression_register_input));
    }

    /// Cancel expression-register input and clear the pending target.
    pub fn cancel_expression_register(&mut self) {
        self.pending_expression_register = None;
        self.expression_register_input.clear();
        self.input_state.selected_register = None;
        self.clear_status();
    }

    /// Evaluate the pending expression and route the result to normal/insert behavior.
    pub fn submit_expression_register(&mut self) -> bool {
        let Some(target) = self.pending_expression_register.take() else {
            return false;
        };
        let expression = std::mem::take(&mut self.expression_register_input);
        let result = match evaluate_expression_register(&expression) {
            Ok(result) => result,
            Err(err) => {
                self.input_state.selected_register = None;
                self.set_status(format!("Expression error: {}", err));
                return false;
            }
        };

        self.expression_register_value = Some(result.clone());
        match target {
            ExpressionRegisterTarget::Normal => {
                self.input_state.selected_register = Some('=');
                self.set_status(format!("={}", expression));
            }
            ExpressionRegisterTarget::Insert => {
                self.insert_text_at_cursor(&result);
                self.clear_status();
            }
        }
        true
    }

    /// Insert the text from the previous completed insert session.
    pub fn insert_last_inserted_text(&mut self) -> bool {
        let Some(text) = self
            .last_inserted_text
            .as_ref()
            .filter(|text| !text.is_empty())
            .cloned()
        else {
            return false;
        };

        self.insert_text_at_cursor(&text);
        true
    }

    /// Enter replace mode
    pub fn enter_replace_mode(&mut self) {
        self.begin_change();
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
        if self.mode == Mode::Insert {
            self.finish_insert_session();
        }

        // End any current undo group
        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);

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
        } else if matches!(ch, '}' | ']' | ')') && self.settings.editor.auto_indent {
            self.insert_closing_bracket(ch);
        } else {
            // Standard character insertion
            self.undo_stack.record_change(Change::insert(
                self.cursor.line,
                self.cursor.col,
                ch.to_string(),
            ));

            if ch == '\n' {
                self.buffers[self.current_buffer_idx].insert_char(
                    self.cursor.line,
                    self.cursor.col,
                    '\n',
                );
                self.cursor.line += 1;
                self.cursor.col = 0;
            } else {
                self.buffers[self.current_buffer_idx].insert_char(
                    self.cursor.line,
                    self.cursor.col,
                    ch,
                );
                self.cursor.col += 1;
            }
            self.record_inserted_char(ch);
        }
        self.scroll_to_cursor();
    }

    /// Insert newline with smart indentation using tree-sitter when available
    fn insert_newline_with_indent(&mut self) {
        let tab_width = self.settings.editor.tab_width;
        let language = self.syntax.language_name().map(|s| s.to_string());

        // Check if cursor is between matching brackets like {|} or [|] or (|)
        let between_brackets = self.is_cursor_between_brackets();

        // Try tree-sitter based indentation for supported languages
        if matches!(
            language.as_deref(),
            Some("javascript" | "typescript" | "tsx" | "css" | "json" | "toml" | "html")
        ) {
            if let Some((tree, source)) = self.syntax.get_tree_and_source() {
                if let Some(cursor_byte) = self
                    .syntax
                    .position_to_byte(self.cursor.line, self.cursor.col)
                {
                    let indent_spaces =
                        crate::indent::calculate_indent(tree, source, cursor_byte, tab_width);
                    let indent = " ".repeat(indent_spaces);

                    if between_brackets {
                        // Bracket expansion: insert two newlines
                        // First line: indented content line (where cursor goes)
                        // Second line: closing bracket at base indent
                        let base_indent =
                            self.buffers[self.current_buffer_idx].get_line_indent(self.cursor.line);
                        let insert_text = format!("\n{}\n{}", indent, base_indent);

                        self.undo_stack.record_change(Change::insert(
                            self.cursor.line,
                            self.cursor.col,
                            insert_text.clone(),
                        ));

                        self.buffers[self.current_buffer_idx].insert_str(
                            self.cursor.line,
                            self.cursor.col,
                            &insert_text,
                        );
                        self.record_inserted_text(&insert_text);

                        // Move cursor to the indented middle line
                        self.cursor.line += 1;
                        self.cursor.col = indent.len();
                    } else {
                        // Regular newline with indent
                        let insert_text = format!("\n{}", indent);
                        self.undo_stack.record_change(Change::insert(
                            self.cursor.line,
                            self.cursor.col,
                            insert_text.clone(),
                        ));

                        self.buffers[self.current_buffer_idx].insert_str(
                            self.cursor.line,
                            self.cursor.col,
                            &insert_text,
                        );
                        self.record_inserted_text(&insert_text);

                        self.cursor.line += 1;
                        self.cursor.col = indent.len();
                    }
                    return;
                }
            }
        }

        // Fallback to basic indentation
        self.insert_newline_basic_indent_with_expansion(between_brackets);
    }

    /// Check if cursor is positioned between matching brackets like {|} or [|] or (|)
    fn is_cursor_between_brackets(&self) -> bool {
        let buffer = &self.buffers[self.current_buffer_idx];
        let Some(line) = buffer.line(self.cursor.line) else {
            return false;
        };

        let chars: Vec<char> = line.chars().collect();
        let col = self.cursor.col;

        // Need at least one char before and one after cursor
        if col == 0 || col >= chars.len() {
            return false;
        }

        let char_before = chars[col - 1];
        let char_after = chars[col];

        matches!(
            (char_before, char_after),
            ('{', '}') | ('[', ']') | ('(', ')')
        )
    }

    /// Basic indentation fallback for non-tree-sitter languages
    fn insert_newline_basic_indent_with_expansion(&mut self, between_brackets: bool) {
        let buffer = &self.buffers[self.current_buffer_idx];
        let base_indent = buffer.get_line_indent(self.cursor.line);
        let ends_with_brace = buffer.line_ends_with(self.cursor.line, '{');
        let tab_width = self.settings.editor.tab_width;

        // Calculate the full indent for the new line
        let mut indent = base_indent.clone();
        if ends_with_brace || between_brackets {
            // Add one level of indentation after { or between brackets
            indent.push_str(&" ".repeat(tab_width));
        }

        if between_brackets {
            // Bracket expansion: insert two newlines
            let insert_text = format!("\n{}\n{}", indent, base_indent);
            self.undo_stack.record_change(Change::insert(
                self.cursor.line,
                self.cursor.col,
                insert_text.clone(),
            ));

            self.buffers[self.current_buffer_idx].insert_str(
                self.cursor.line,
                self.cursor.col,
                &insert_text,
            );
            self.record_inserted_text(&insert_text);

            // Move cursor to the indented middle line
            self.cursor.line += 1;
            self.cursor.col = indent.len();
        } else {
            // Regular newline with indent
            let insert_text = format!("\n{}", indent);
            self.undo_stack.record_change(Change::insert(
                self.cursor.line,
                self.cursor.col,
                insert_text.clone(),
            ));

            self.buffers[self.current_buffer_idx].insert_str(
                self.cursor.line,
                self.cursor.col,
                &insert_text,
            );
            self.record_inserted_text(&insert_text);

            self.cursor.line += 1;
            self.cursor.col = indent.len();
        }
    }

    /// Insert closing bracket with smart auto-dedent
    ///
    /// Handles }, ], and ) characters with tree-sitter based dedent detection
    fn insert_closing_bracket(&mut self, bracket: char) {
        let tab_width = self.settings.editor.tab_width;
        let language = self.syntax.language_name().map(|s| s.to_string());

        // Try tree-sitter based dedent for supported languages
        if matches!(
            language.as_deref(),
            Some("javascript" | "typescript" | "tsx" | "css" | "json" | "toml" | "html")
        ) {
            if let Some((tree, source)) = self.syntax.get_tree_and_source() {
                if let Some(cursor_byte) = self
                    .syntax
                    .position_to_byte(self.cursor.line, self.cursor.col)
                {
                    let dedent_amount = crate::indent::get_dedent_amount(
                        tree,
                        source,
                        cursor_byte,
                        bracket,
                        tab_width,
                    );

                    if dedent_amount > 0 && self.cursor.col >= dedent_amount {
                        let delete_start = self.cursor.col - dedent_amount;

                        // Record the deletion for undo
                        let deleted_text = " ".repeat(dedent_amount);
                        self.undo_stack.record_change(Change::delete(
                            self.cursor.line,
                            delete_start,
                            deleted_text,
                        ));

                        // Delete the indent
                        for _ in 0..dedent_amount {
                            self.cursor.col -= 1;
                            self.buffers[self.current_buffer_idx]
                                .delete_char(self.cursor.line, self.cursor.col);
                        }
                    }

                    // Insert the bracket
                    self.undo_stack.record_change(Change::insert(
                        self.cursor.line,
                        self.cursor.col,
                        bracket.to_string(),
                    ));
                    self.buffers[self.current_buffer_idx].insert_char(
                        self.cursor.line,
                        self.cursor.col,
                        bracket,
                    );
                    self.cursor.col += 1;
                    self.record_inserted_char(bracket);
                    return;
                }
            }
        }

        // Fallback to basic dedent logic (only for closing brace)
        if bracket == '}' {
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
                    self.buffers[self.current_buffer_idx]
                        .delete_char(self.cursor.line, self.cursor.col);
                }
            }
        }

        // Record and insert the bracket
        self.undo_stack.record_change(Change::insert(
            self.cursor.line,
            self.cursor.col,
            bracket.to_string(),
        ));
        self.buffers[self.current_buffer_idx].insert_char(
            self.cursor.line,
            self.cursor.col,
            bracket,
        );
        self.cursor.col += 1;
        self.record_inserted_char(bracket);
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
            let deleted = self.buffers[self.current_buffer_idx]
                .get_char_str(self.cursor.line, self.cursor.col);
            self.undo_stack.record_change(Change::delete(
                self.cursor.line,
                self.cursor.col,
                deleted,
            ));
            self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, self.cursor.col);
        } else if self.cursor.line > 0 {
            // Join with previous line
            let prev_line_len =
                self.buffers[self.current_buffer_idx].line_len(self.cursor.line - 1);
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
        self.delete_chars_at(1);
    }

    /// Delete count characters at cursor (x with count)
    pub fn delete_chars_at(&mut self, count: usize) {
        let count = count.max(1);
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        if line_len == 0 || self.cursor.col >= line_len {
            return;
        }

        let start_col = self.cursor.col;
        let end_col = (start_col + count - 1).min(line_len.saturating_sub(1));
        let deleted = self.get_range_text(self.cursor.line, start_col, self.cursor.line, end_col);
        if deleted.is_empty() {
            return;
        }

        self.begin_change();
        self.undo_stack
            .record_change(Change::delete(self.cursor.line, start_col, deleted.clone()));
        self.buffers[self.current_buffer_idx].delete_range(
            self.cursor.line,
            start_col,
            self.cursor.line,
            end_col + 1,
        );
        self.cursor.col = start_col;
        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        self.registers
            .delete(None, RegisterContent::Chars(deleted), true);
        self.clamp_cursor();
    }

    /// Delete character at cursor as part of an already-open undo group.
    pub fn delete_char_at_in_current_group(&mut self) {
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        if line_len > 0 {
            if let Some(ch) =
                self.buffers[self.current_buffer_idx].char_at(self.cursor.line, self.cursor.col)
            {
                self.undo_stack.record_change(Change::delete(
                    self.cursor.line,
                    self.cursor.col,
                    ch.to_string(),
                ));
                self.registers
                    .delete(None, RegisterContent::Chars(ch.to_string()), true);
            }
            self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, self.cursor.col);
            self.clamp_cursor();
        }
    }

    /// Delete character before cursor in normal mode (X)
    pub fn delete_char_before_normal(&mut self) {
        self.delete_chars_before_normal(1);
    }

    /// Delete count characters before cursor in normal mode (X with count)
    pub fn delete_chars_before_normal(&mut self, count: usize) {
        if self.cursor.col == 0 {
            return;
        }

        let count = count.max(1);
        let end_col = self.cursor.col - 1;
        let start_col = self.cursor.col.saturating_sub(count);
        let deleted = self.get_range_text(self.cursor.line, start_col, self.cursor.line, end_col);
        if deleted.is_empty() {
            return;
        }

        self.begin_change();
        self.undo_stack
            .record_change(Change::delete(self.cursor.line, start_col, deleted.clone()));
        self.buffers[self.current_buffer_idx].delete_range(
            self.cursor.line,
            start_col,
            self.cursor.line,
            end_col + 1,
        );
        self.cursor.col = start_col;
        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        self.registers
            .delete(None, RegisterContent::Chars(deleted), true);
    }

    /// Delete word before cursor (Ctrl+w in insert mode)
    /// Deletes backwards to the start of the current word
    pub fn delete_word_before(&mut self) {
        if self.cursor.col == 0 {
            // At start of line, join with previous line (like backspace)
            self.delete_char_before();
            return;
        }

        let Some(line) = self.buffer().line(self.cursor.line) else {
            return;
        };
        let chars: Vec<char> = line.chars().collect();
        let start_col = self.cursor.col;
        let mut col = start_col;

        // Skip whitespace backwards
        while col > 0
            && chars
                .get(col - 1)
                .map(|c| c.is_whitespace())
                .unwrap_or(false)
        {
            col -= 1;
        }

        // Skip word characters backwards (or non-whitespace non-word chars)
        if col > 0 {
            let is_word_char = chars
                .get(col - 1)
                .map(|c| c.is_alphanumeric() || *c == '_')
                .unwrap_or(false);
            if is_word_char {
                // Delete word characters
                while col > 0
                    && chars
                        .get(col - 1)
                        .map(|c| c.is_alphanumeric() || *c == '_')
                        .unwrap_or(false)
                {
                    col -= 1;
                }
            } else {
                // Delete non-word, non-whitespace characters
                while col > 0 {
                    let c = chars.get(col - 1);
                    if c.map(|c| c.is_whitespace() || c.is_alphanumeric() || *c == '_')
                        .unwrap_or(true)
                    {
                        break;
                    }
                    col -= 1;
                }
            }
        }

        // Delete from col to start_col
        if col < start_col {
            let deleted: String = chars[col..start_col].iter().collect();
            self.cursor.col = col;

            // Record for undo
            self.undo_stack.record_change(Change::delete(
                self.cursor.line,
                self.cursor.col,
                deleted,
            ));

            // Delete the characters
            for _ in 0..(start_col - col) {
                self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, col);
            }
        }
    }

    /// Delete to start of line (Ctrl+u in insert mode)
    pub fn delete_to_line_start(&mut self) {
        if self.cursor.col == 0 {
            return;
        }

        let Some(line) = self.buffer().line(self.cursor.line) else {
            return;
        };
        let chars: Vec<char> = line.chars().collect();
        let deleted: String = chars[..self.cursor.col].iter().collect();
        let delete_col = self.cursor.col;

        // Record for undo
        self.undo_stack
            .record_change(Change::delete(self.cursor.line, 0, deleted));

        // Delete from start to cursor
        for _ in 0..delete_col {
            self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, 0);
        }

        self.cursor.col = 0;
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
        self.begin_change();
        self.undo_stack.record_change(Change::insert(
            self.cursor.line,
            line_len,
            insert_text.clone(),
        ));

        self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, line_len, &insert_text);
        self.cursor.line += 1;
        self.cursor.col = indent.len();
        self.last_insert_position = Some((self.cursor.line, self.cursor.col));
        self.mode = Mode::Insert;
        self.begin_insert_session();
        self.record_inserted_text(&insert_text);
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
        self.begin_change();
        self.undo_stack
            .record_change(Change::insert(self.cursor.line, 0, insert_text.clone()));

        self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, 0, &insert_text);
        // Cursor stays on same line number (which is now the new line with indent)
        self.cursor.col = indent.len();
        self.last_insert_position = Some((self.cursor.line, self.cursor.col));
        self.mode = Mode::Insert;
        self.begin_insert_session();
        self.record_inserted_text(&insert_text);
        self.scroll_to_cursor();
    }

    /// Save the current buffer
    pub fn save(&mut self) -> anyhow::Result<()> {
        self.buffers[self.current_buffer_idx].save()?;
        self.status_message = Some(format!(
            "\"{}\" written",
            self.buffers[self.current_buffer_idx].display_name()
        ));
        // Update git diff after save (file now matches HEAD if no other changes)
        self.update_git_diff();
        self.refresh_explorer_git_statuses();
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
        self.command_line.begin_prompt();
    }

    /// Enter command mode with prefilled input.
    pub fn enter_command_mode_with_input(&mut self, input: impl Into<String>) {
        self.mode = Mode::Command;
        self.command_line.begin_prompt_with_input(input);
    }

    /// Exit command mode back to normal
    pub fn exit_command_mode(&mut self) {
        self.mode = Mode::Normal;
        self.command_line.clear();
    }

    /// Go to a specific line number (1-indexed)
    pub fn goto_line(&mut self, line: usize) {
        let target = line.saturating_sub(1).min(
            self.buffers[self.current_buffer_idx]
                .len_lines()
                .saturating_sub(1),
        );
        self.cursor.line = target;
        self.cursor.col = 0;
        self.clamp_cursor();
        self.scroll_to_cursor();
    }

    /// Record current position in jump list (call before jumping)
    pub fn record_jump(&mut self) {
        let path = self.buffer().path.clone();
        // Save current position as "previous jump position" for '' command
        self.previous_jump_position = Some((path.clone(), self.cursor.line, self.cursor.col));
        self.jump_list
            .record(path, self.cursor.line, self.cursor.col);
    }

    /// Go back in jump list (Ctrl+o)
    pub fn jump_back(&mut self) -> bool {
        let current_path = self.buffer().path.clone();
        let current_line = self.cursor.line;
        let current_col = self.cursor.col;

        if let Some(loc) = self
            .jump_list
            .go_back(current_path, current_line, current_col)
            .cloned()
        {
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

    /// Jump to the line of the previous position ('' command).
    pub fn jump_to_previous_position_line(&mut self) -> bool {
        self.jump_to_previous_position(false)
    }

    /// Jump to the exact previous position (`` command).
    pub fn jump_to_previous_position_exact(&mut self) -> bool {
        self.jump_to_previous_position(true)
    }

    /// Jump to previous position and toggle back on repeated use.
    fn jump_to_previous_position(&mut self, exact: bool) -> bool {
        if let Some((prev_path, prev_line, prev_col)) = self.previous_jump_position.take() {
            // Save current position so we can toggle back
            let current_path = self.buffer().path.clone();
            let current_line = self.cursor.line;
            let current_col = self.cursor.col;
            self.previous_jump_position = Some((current_path.clone(), current_line, current_col));

            // Check if we need to switch files
            if prev_path != current_path {
                if let Some(path) = prev_path {
                    if self.open_file(path).is_err() {
                        return false;
                    }
                }
            }

            self.cursor.line = prev_line;
            self.cursor.col = if exact {
                prev_col
            } else {
                self.find_first_non_blank(self.cursor.line)
            };
            self.clamp_cursor();
            self.scroll_to_cursor();
            true
        } else {
            false
        }
    }

    /// Go to older change position (g;)
    pub fn change_list_older(&mut self) -> bool {
        if let Some(loc) = self.change_list.go_older().cloned() {
            self.cursor.line = loc.line;
            self.cursor.col = loc.col;
            self.clamp_cursor();
            self.scroll_to_cursor();
            true
        } else {
            false
        }
    }

    /// Go to newer change position (g,)
    pub fn change_list_newer(&mut self) -> bool {
        if let Some(loc) = self.change_list.go_newer().cloned() {
            self.cursor.line = loc.line;
            self.cursor.col = loc.col;
            self.clamp_cursor();
            self.scroll_to_cursor();
            true
        } else {
            false
        }
    }

    /// Jump to the line of the last change ('. command).
    pub fn jump_to_last_change_line(&mut self) -> bool {
        self.jump_to_last_change(false)
    }

    /// Jump to the exact position of the last change (`. command).
    pub fn jump_to_last_change_exact(&mut self) -> bool {
        self.jump_to_last_change(true)
    }

    fn jump_to_last_change(&mut self, exact: bool) -> bool {
        if let Some(loc) = self.change_list.latest().cloned() {
            self.cursor.line = loc.line;
            self.cursor.col = if exact {
                loc.col
            } else {
                self.find_first_non_blank(loc.line)
            };
            self.clamp_cursor();
            self.scroll_to_cursor();
            true
        } else {
            false
        }
    }

    /// Jump to the line of the last insert ('^ command).
    pub fn jump_to_last_insert_line(&mut self) -> bool {
        self.jump_to_last_insert(false)
    }

    /// Jump to the exact position of the last insert (`^ command).
    pub fn jump_to_last_insert_exact(&mut self) -> bool {
        self.jump_to_last_insert(true)
    }

    fn jump_to_last_insert(&mut self, exact: bool) -> bool {
        if let Some((line, col)) = self.last_insert_position {
            self.cursor.line = line;
            self.cursor.col = if exact {
                col
            } else {
                self.find_first_non_blank(line)
            };
            self.clamp_cursor();
            self.scroll_to_cursor();
            true
        } else {
            false
        }
    }

    /// Begin an undo group and record change position
    /// This should be called before making changes to the buffer
    pub fn begin_change(&mut self) {
        let line = self.cursor.line;
        let col = self.cursor.col;
        let new_group = self.undo_stack.begin_undo_group(line, col);
        if new_group {
            // Record to change list when starting a new undo group
            self.change_list.record(line, col);
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
            self.h_offset = 0;
            self.reset_current_undo_stack();
            self.parse_current_buffer();
            self.set_status("File reloaded");
            Ok(())
        } else {
            anyhow::bail!("No file to reload")
        }
    }

    /// Check for external changes to open buffers and reload if safe
    /// Returns a status message if any action was taken
    pub fn check_and_reload_external_changes(&mut self) -> Option<String> {
        let mut reloaded = Vec::new();
        let mut warnings = Vec::new();

        let mut reloaded_indices = Vec::new();
        for i in 0..self.buffers.len() {
            if self.buffers[i].has_external_changes() {
                if let Some(path) = &self.buffers[i].path {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("file")
                        .to_string();

                    if !self.buffers[i].dirty {
                        // Buffer is clean, safe to auto-reload
                        if self.buffers[i].reload().is_ok() {
                            reloaded_indices.push(i);
                            reloaded.push(name);
                        }
                    } else {
                        // Buffer has unsaved changes, warn user
                        warnings.push(name);
                    }
                }
            }
        }

        for idx in reloaded_indices {
            self.reset_undo_stack_for_buffer(idx);
        }

        // Re-parse current buffer if it was reloaded
        if !reloaded.is_empty() {
            self.parse_current_buffer();
        }

        // Build status message
        if !reloaded.is_empty() && !warnings.is_empty() {
            Some(format!(
                "Reloaded: {}. Warning: {} changed externally (unsaved)",
                reloaded.join(", "),
                warnings.join(", ")
            ))
        } else if !reloaded.is_empty() {
            Some(format!("Reloaded: {}", reloaded.join(", ")))
        } else if !warnings.is_empty() {
            Some(format!(
                "Warning: {} changed externally (unsaved changes)",
                warnings.join(", ")
            ))
        } else {
            None
        }
    }

    /// Check if buffer has unsaved changes
    pub fn has_unsaved_changes(&self) -> bool {
        self.buffers[self.current_buffer_idx].dirty
    }

    /// Check if any buffer has unsaved changes
    pub fn has_any_unsaved_changes(&self) -> bool {
        self.buffers.iter().any(|b| b.dirty)
    }

    /// Save all modified buffers
    /// Returns the count of buffers saved, or error if any buffer fails
    pub fn save_all(&mut self) -> anyhow::Result<usize> {
        let mut saved_count = 0;
        for i in 0..self.buffers.len() {
            if self.buffers[i].dirty && self.buffers[i].path.is_some() {
                self.buffers[i].save()?;
                saved_count += 1;
            }
        }
        self.update_git_diff();
        self.refresh_explorer_git_statuses();
        Ok(saved_count)
    }

    /// Format and save all modified buffers (respects format_on_save setting)
    /// Returns (saved_count, formatted_count, formatter_name)
    pub fn format_and_save_all(&mut self) -> anyhow::Result<(usize, usize, Option<String>)> {
        let mut saved_count = 0;
        let mut formatted_count = 0;
        let mut formatter_name: Option<String> = None;
        let format_on_save = self.settings.editor.format_on_save;

        for i in 0..self.buffers.len() {
            if self.buffers[i].dirty && self.buffers[i].path.is_some() {
                // Try to format if format_on_save is enabled
                if format_on_save {
                    if let Some(formatter_config) = self.get_formatter_for_buffer(i).cloned() {
                        let content = self.buffers[i].content();
                        let file_path = self.buffers[i]
                            .path
                            .as_ref()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();

                        match crate::formatter::format_with_external(
                            &content,
                            &file_path,
                            &formatter_config,
                        ) {
                            Ok(formatted) => {
                                if formatted != content {
                                    if i == self.current_buffer_idx {
                                        self.replace_buffer_content_with_undo(&formatted);
                                    } else {
                                        self.buffers[i].set_content(&formatted);
                                        self.reset_undo_stack_for_buffer(i);
                                    }
                                    formatted_count += 1;
                                }
                                formatter_name = Some(formatter_config.command.clone());
                            }
                            Err(_e) => {
                                // Formatter failed - continue with save anyway
                            }
                        }
                    }
                }
                // Save the buffer
                self.buffers[i].save()?;
                saved_count += 1;
            }
        }
        self.update_git_diff();
        self.refresh_explorer_git_statuses();
        Ok((saved_count, formatted_count, formatter_name))
    }

    /// Get list of buffers with unsaved changes (for error messages)
    pub fn unsaved_buffer_names(&self) -> Vec<String> {
        self.buffers
            .iter()
            .filter(|b| b.dirty)
            .map(|b| b.display_name())
            .collect()
    }

    /// Undo the last change
    pub fn undo(&mut self) {
        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        if let Some(entry) = self.undo_stack.pop_undo() {
            // Apply changes in reverse order
            for change in entry.changes.iter().rev() {
                // To undo: we need the inverse operation
                // If it was an insert (old_text empty, new_text has content), we delete new_text
                // If it was a delete (old_text has content, new_text empty), we insert old_text
                self.buffers[self.current_buffer_idx].apply_change(
                    change.start_line,
                    change.start_col,
                    &change.new_text, // Remove what was inserted
                    &change.old_text, // Restore what was deleted
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
        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        if let Some(entry) = self.undo_stack.pop_redo() {
            // Apply changes in forward order
            for change in entry.changes.iter() {
                // To redo: apply the original change
                self.buffers[self.current_buffer_idx].apply_change(
                    change.start_line,
                    change.start_col,
                    &change.old_text, // Remove old text
                    &change.new_text, // Insert new text
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
                    self.search_matches
                        .iter()
                        .find(|(line, col, _)| {
                            *line > cursor_line || (*line == cursor_line && *col > cursor_col)
                        })
                        .or_else(|| self.search_matches.first())
                }
                SearchDirection::Backward => {
                    // Find last match before cursor
                    self.search_matches
                        .iter()
                        .rev()
                        .find(|(line, col, _)| {
                            *line < cursor_line || (*line == cursor_line && *col < cursor_col)
                        })
                        .or_else(|| self.search_matches.last())
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
            // Record jump before searching (search is a jump motion)
            self.record_jump();
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
            // Record jump before searching (search is a jump motion)
            self.record_jump();
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

    /// Search forward for the last pattern and select the match (gn).
    pub fn search_select_next(&mut self, count: usize) {
        self.search_select_match(SearchDirection::Forward, count);
    }

    /// Search backward for the last pattern and select the match (gN).
    pub fn search_select_prev(&mut self, count: usize) {
        self.search_select_match(SearchDirection::Backward, count);
    }

    fn search_select_match(&mut self, direction: SearchDirection, count: usize) {
        let Some(pattern) = self.search.last_pattern.clone() else {
            self.set_status("No previous search pattern");
            return;
        };

        self.update_search_matches_from_pattern(&pattern);

        let Some((line, start_col, end_col)) =
            self.search_match_from_cursor(direction, count.max(1))
        else {
            self.set_status(format!("Pattern not found: {}", pattern));
            return;
        };

        self.record_jump();
        self.mode = Mode::Visual;
        self.visual = VisualSelection::new(line, start_col);
        self.cursor.line = line;
        self.cursor.col = end_col.saturating_sub(1).max(start_col);
        self.scroll_to_cursor();
    }

    fn search_match_from_cursor(
        &self,
        direction: SearchDirection,
        count: usize,
    ) -> Option<(usize, usize, usize)> {
        if self.search_matches.is_empty() {
            return None;
        }

        let cursor_line = self.cursor.line;
        let cursor_col = self.cursor.col;
        let mut ordered = Vec::with_capacity(self.search_matches.len());

        match direction {
            SearchDirection::Forward => {
                ordered.extend(
                    self.search_matches
                        .iter()
                        .copied()
                        .filter(|(line, _, end_col)| {
                            *line > cursor_line || (*line == cursor_line && *end_col > cursor_col)
                        }),
                );
                ordered.extend(
                    self.search_matches
                        .iter()
                        .copied()
                        .filter(|(line, _, end_col)| {
                            !(*line > cursor_line
                                || (*line == cursor_line && *end_col > cursor_col))
                        }),
                );
            }
            SearchDirection::Backward => {
                ordered.extend(self.search_matches.iter().rev().copied().filter(
                    |(line, start_col, _)| {
                        *line < cursor_line || (*line == cursor_line && *start_col <= cursor_col)
                    },
                ));
                ordered.extend(self.search_matches.iter().rev().copied().filter(
                    |(line, start_col, _)| {
                        !(*line < cursor_line || (*line == cursor_line && *start_col <= cursor_col))
                    },
                ));
            }
        }

        ordered.get((count - 1) % ordered.len()).copied()
    }

    /// Get the word under the cursor
    pub fn get_word_under_cursor(&self) -> Option<String> {
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

    /// Resolve a path-like token under the cursor for `gf`.
    pub fn file_path_under_cursor(&self) -> Option<std::path::PathBuf> {
        let token = self.get_file_token_under_cursor()?;
        let primary = self.resolve_cursor_file_token(&token);
        if primary.exists() {
            return Some(primary);
        }

        let trimmed = token.trim_end_matches(|ch| matches!(ch, '.' | ',' | ';' | ':'));
        if trimmed != token {
            let fallback = self.resolve_cursor_file_token(trimmed);
            if fallback.exists() {
                return Some(fallback);
            }
        }

        Some(primary)
    }

    pub fn open_file_under_cursor(&mut self) -> Result<std::path::PathBuf, String> {
        let path = self
            .file_path_under_cursor()
            .ok_or_else(|| "No file under cursor".to_string())?;
        if !path.exists() {
            return Err(format!("File not found: {}", path.display()));
        }

        self.open_file(path.clone())
            .map_err(|err| format!("Error opening file: {}", err))?;
        Ok(path)
    }

    /// Open the URL under the cursor using the platform default browser.
    pub fn open_url_under_cursor(&mut self) -> Result<String, String> {
        self.open_url_under_cursor_with(Self::open_url_external)
    }

    /// Open the URL under the cursor with an injected opener.
    pub fn open_url_under_cursor_with<F>(&mut self, mut opener: F) -> Result<String, String>
    where
        F: FnMut(&str) -> Result<(), String>,
    {
        let url = self
            .url_under_cursor()
            .ok_or_else(|| "No URL under cursor".to_string())?;
        opener(&url)?;
        Ok(url)
    }

    /// Resolve the URL token under the cursor for `gx`.
    pub fn url_under_cursor(&self) -> Option<String> {
        let line = self.buffers[self.current_buffer_idx].line(self.cursor.line)?;
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            return None;
        }

        let col = self.cursor.col.min(chars.len().saturating_sub(1));
        if Self::is_url_delimiter(chars[col]) {
            return None;
        }

        let mut start = col;
        while start > 0 && !Self::is_url_delimiter(chars[start - 1]) {
            start -= 1;
        }

        let mut end = col;
        while end < chars.len() && !Self::is_url_delimiter(chars[end]) {
            end += 1;
        }

        let token: String = chars[start..end].iter().collect();
        Self::trim_url_token(&token)
    }

    fn trim_url_token(token: &str) -> Option<String> {
        let trimmed = token
            .trim_start_matches(|ch| matches!(ch, '(' | '[' | '{' | '<' | '"' | '\''))
            .trim_end_matches(|ch| {
                matches!(
                    ch,
                    '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>' | '"' | '\''
                )
            });

        Self::is_supported_url(trimmed).then(|| trimmed.to_string())
    }

    fn is_url_delimiter(ch: char) -> bool {
        ch.is_whitespace()
    }

    fn is_supported_url(token: &str) -> bool {
        let lower = token.to_ascii_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    }

    fn open_url_external(url: &str) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = std::process::Command::new("open");
            command.arg(url);
            command
        };

        #[cfg(target_os = "linux")]
        let mut command = {
            let mut command = std::process::Command::new("xdg-open");
            command.arg(url);
            command
        };

        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = std::process::Command::new("cmd");
            command.args(["/C", "start", "", url]);
            command
        };

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            let _ = url;
            return Err("Opening URLs is not supported on this platform".to_string());
        }

        command
            .spawn()
            .map(|_| ())
            .map_err(|err| format!("Failed to open URL: {}", err))
    }

    fn get_file_token_under_cursor(&self) -> Option<String> {
        let line = self.buffers[self.current_buffer_idx].line(self.cursor.line)?;
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            return None;
        }

        let col = self.cursor.col.min(chars.len().saturating_sub(1));
        if !Self::is_file_path_char(chars[col]) {
            return None;
        }

        let mut start = col;
        while start > 0 && Self::is_file_path_char(chars[start - 1]) {
            start -= 1;
        }

        let mut end = col;
        while end < chars.len() && Self::is_file_path_char(chars[end]) {
            end += 1;
        }

        (start < end).then(|| chars[start..end].iter().collect())
    }

    fn resolve_cursor_file_token(&self, token: &str) -> std::path::PathBuf {
        let path = std::path::PathBuf::from(token);
        if path.is_absolute() {
            return path;
        }

        let base = self
            .buffer()
            .path
            .as_ref()
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| self.working_directory());
        base.join(path)
    }

    fn is_file_path_char(ch: char) -> bool {
        ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '\\' | '~')
    }

    /// Check if a character is a word character (alphanumeric or underscore)
    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    /// Search and replace text
    /// Returns the number of replacements made
    pub fn substitute(
        &mut self,
        pattern: &str,
        replacement: &str,
        entire_file: bool,
        global: bool,
    ) -> usize {
        if pattern.is_empty() {
            return 0;
        }

        // Begin undo group for all replacements
        self.begin_change();

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
                    self.undo_stack
                        .record_change(crate::editor::undo::Change::replace_line(
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
        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);

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
                            if let Some(pos) =
                                line_str[..end_byte.min(line_str.len())].find(pattern)
                            {
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
                                    self.cursor.col =
                                        Self::byte_to_char_idx(&line_str, start_byte + pos);
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
            Mode::Visual => self.visual.get_range(self.cursor.line, self.cursor.col),
            Mode::VisualLine => {
                let (start_line, end_line) = self.visual.get_line_range(self.cursor.line);
                let end_col = self.buffers[self.current_buffer_idx].line_len(end_line);
                (start_line, 0, end_line, end_col)
            }
            Mode::VisualBlock => {
                // For block mode, return (top, left, bottom, right)
                self.visual
                    .get_block_range(self.cursor.line, self.cursor.col)
            }
            _ => (
                self.cursor.line,
                self.cursor.col,
                self.cursor.line,
                self.cursor.col,
            ),
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
                self.registers
                    .delete(None, RegisterContent::Lines(text), false);
            }
            Mode::Visual => {
                // Character-wise delete
                let text = self.get_range_text(start_line, start_col, end_line, end_col);

                // Record for undo
                self.begin_change();
                self.undo_stack
                    .record_change(Change::delete(start_line, start_col, text.clone()));

                self.buffers[self.current_buffer_idx].delete_range(
                    start_line,
                    start_col,
                    end_line,
                    end_col + 1,
                );

                self.undo_stack.end_undo_group(start_line, start_col);

                self.cursor.line = start_line;
                self.cursor.col = start_col;
                self.clamp_cursor();

                let is_small = !text.contains('\n');
                self.registers
                    .delete(None, RegisterContent::Chars(text), is_small);
            }
            Mode::VisualBlock => {
                // Block-wise delete
                let (top, left, bottom, right) = self
                    .visual
                    .get_block_range(self.cursor.line, self.cursor.col);

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
                                .filter_map(|c| {
                                    self.buffers[self.current_buffer_idx].char_at(line_idx, c)
                                })
                                .collect();
                            deleted_lines.push(deleted.clone());

                            // Record the delete for undo
                            self.undo_stack
                                .record_change(Change::delete(line_idx, left, deleted));

                            // Delete the range on this line
                            self.buffers[self.current_buffer_idx].delete_range(
                                line_idx,
                                left,
                                line_idx,
                                actual_right + 1,
                            );
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

                self.registers
                    .delete(None, RegisterContent::Chars(block_text), false);
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
                let (top, left, bottom, right) = self
                    .visual
                    .get_block_range(self.cursor.line, self.cursor.col);

                // Collect text from each line in the block
                let mut yanked_lines: Vec<String> = Vec::new();
                for line_idx in top..=bottom {
                    let line_len = self.buffers[self.current_buffer_idx].line_len(line_idx);
                    if left < line_len {
                        let actual_right = right.min(line_len.saturating_sub(1));
                        if left <= actual_right {
                            let text: String = (left..=actual_right)
                                .filter_map(|c| {
                                    self.buffers[self.current_buffer_idx].char_at(line_idx, c)
                                })
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
                self.registers
                    .yank(None, RegisterContent::Chars(block_text));
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
                self.begin_change();
                self.undo_stack
                    .record_change(Change::delete(start_line, 0, text.clone()));

                self.registers
                    .delete(None, RegisterContent::Lines(text), false);

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
                    self.buffers[self.current_buffer_idx]
                        .delete_range(start_line, 0, start_line, line_len);
                }

                self.enter_insert_mode_at_change(start_line, 0);
            }
            Mode::Visual => {
                // Character-wise change
                let text = self.get_range_text(start_line, start_col, end_line, end_col);

                // Begin undo group
                self.begin_change();
                self.undo_stack
                    .record_change(Change::delete(start_line, start_col, text.clone()));

                self.buffers[self.current_buffer_idx].delete_range(
                    start_line,
                    start_col,
                    end_line,
                    end_col + 1,
                );

                let is_small = !text.contains('\n');
                self.registers
                    .delete(None, RegisterContent::Chars(text), is_small);

                self.enter_insert_mode_at_change(start_line, start_col);
            }
            Mode::VisualBlock => {
                // Block-wise change: delete the block and enter insert mode
                let (top, left, bottom, right) = self
                    .visual
                    .get_block_range(self.cursor.line, self.cursor.col);

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
                                .filter_map(|c| {
                                    self.buffers[self.current_buffer_idx].char_at(line_idx, c)
                                })
                                .collect();
                            deleted_lines.push(deleted.clone());

                            // Record the delete for undo
                            self.undo_stack
                                .record_change(Change::delete(line_idx, left, deleted));

                            // Delete the range on this line
                            self.buffers[self.current_buffer_idx].delete_range(
                                line_idx,
                                left,
                                line_idx,
                                actual_right + 1,
                            );
                        }
                    }
                }

                // Note: We don't end undo group here - insert mode will continue it
                // Reverse to get top-to-bottom order
                deleted_lines.reverse();
                let block_text = deleted_lines.join("\n");

                self.registers
                    .delete(None, RegisterContent::Chars(block_text), false);

                self.enter_insert_mode_at_change(top, left);
            }
            _ => {}
        }

        self.scroll_to_cursor();
    }

    /// Paste over the current visual selection.
    pub fn visual_paste(&mut self, register: Option<char>) {
        let Some(content) = self.registers.get_content(register) else {
            return;
        };

        let replacement = content.as_str().to_string();
        let mode = self.mode;
        let (start_line, start_col, end_line, end_col) = self.get_visual_range();

        match mode {
            Mode::VisualLine => {
                let deleted = self.get_lines_text(start_line, end_line);
                let count = end_line - start_line + 1;

                self.begin_change();
                self.undo_stack
                    .record_change(Change::delete(start_line, 0, deleted.clone()));
                self.delete_lines(start_line, count);
                self.undo_stack
                    .record_change(Change::insert(start_line, 0, replacement.clone()));
                self.buffers[self.current_buffer_idx].insert_str(start_line, 0, &replacement);
                self.cursor.line = start_line.min(
                    self.buffers[self.current_buffer_idx]
                        .len_lines()
                        .saturating_sub(1),
                );
                self.cursor.col = 0;
                self.undo_stack
                    .end_undo_group(self.cursor.line, self.cursor.col);
                self.registers
                    .delete(None, RegisterContent::Lines(deleted), false);
            }
            Mode::Visual => {
                let deleted = self.get_range_text(start_line, start_col, end_line, end_col);

                self.begin_change();
                self.undo_stack.record_change(Change::delete(
                    start_line,
                    start_col,
                    deleted.clone(),
                ));
                self.buffers[self.current_buffer_idx].delete_range(
                    start_line,
                    start_col,
                    end_line,
                    end_col + 1,
                );
                self.undo_stack.record_change(Change::insert(
                    start_line,
                    start_col,
                    replacement.clone(),
                ));
                self.buffers[self.current_buffer_idx].insert_str(
                    start_line,
                    start_col,
                    &replacement,
                );
                self.cursor.line = start_line;
                self.cursor.col = start_col;
                self.undo_stack
                    .end_undo_group(self.cursor.line, self.cursor.col);

                let is_small = !deleted.contains('\n');
                self.registers
                    .delete(None, RegisterContent::Chars(deleted), is_small);
            }
            Mode::VisualBlock => {
                let (top, left, bottom, right) = self
                    .visual
                    .get_block_range(self.cursor.line, self.cursor.col);
                let replacement_line = replacement
                    .lines()
                    .next()
                    .unwrap_or(&replacement)
                    .to_string();
                let mut deleted_lines = Vec::new();

                self.begin_change();
                for line_idx in (top..=bottom).rev() {
                    let line_len = self.buffers[self.current_buffer_idx].line_len(line_idx);
                    let insert_col = left.min(line_len);
                    if left < line_len {
                        let actual_right = right.min(line_len.saturating_sub(1));
                        let deleted = self.get_range_text(line_idx, left, line_idx, actual_right);
                        deleted_lines.push(deleted.clone());
                        self.undo_stack
                            .record_change(Change::delete(line_idx, left, deleted));
                        self.buffers[self.current_buffer_idx].delete_range(
                            line_idx,
                            left,
                            line_idx,
                            actual_right + 1,
                        );
                    } else {
                        deleted_lines.push(String::new());
                    }

                    self.undo_stack.record_change(Change::insert(
                        line_idx,
                        insert_col,
                        replacement_line.clone(),
                    ));
                    self.buffers[self.current_buffer_idx].insert_str(
                        line_idx,
                        insert_col,
                        &replacement_line,
                    );
                }

                deleted_lines.reverse();
                self.cursor.line = top;
                self.cursor.col = left;
                self.undo_stack
                    .end_undo_group(self.cursor.line, self.cursor.col);
                self.registers.delete(
                    None,
                    RegisterContent::Chars(deleted_lines.join("\n")),
                    false,
                );
            }
            _ => return,
        }

        self.mode = Mode::Normal;
        self.clamp_cursor();
        self.scroll_to_cursor();
    }

    /// Surround the current visual selection.
    pub fn surround_visual_selection(&mut self, surround_char: char) {
        let (open, close) = Self::get_surround_pair(surround_char);
        let mode = self.mode;
        let (start_line, start_col, end_line, end_col) = self.get_visual_range();

        self.begin_change();
        match mode {
            Mode::VisualLine => {
                let close_col = self.buffers[self.current_buffer_idx].line_len(end_line);
                self.undo_stack.record_change(Change::insert(
                    end_line,
                    close_col,
                    close.to_string(),
                ));
                self.buffers[self.current_buffer_idx].insert_char(end_line, close_col, close);
                self.undo_stack
                    .record_change(Change::insert(start_line, 0, open.to_string()));
                self.buffers[self.current_buffer_idx].insert_char(start_line, 0, open);
                self.cursor.line = start_line;
                self.cursor.col = 0;
            }
            Mode::Visual => {
                self.undo_stack.record_change(Change::insert(
                    end_line,
                    end_col + 1,
                    close.to_string(),
                ));
                self.buffers[self.current_buffer_idx].insert_char(end_line, end_col + 1, close);
                self.undo_stack.record_change(Change::insert(
                    start_line,
                    start_col,
                    open.to_string(),
                ));
                self.buffers[self.current_buffer_idx].insert_char(start_line, start_col, open);
                self.cursor.line = start_line;
                self.cursor.col = start_col;
            }
            Mode::VisualBlock => {
                let (top, left, bottom, right) = self
                    .visual
                    .get_block_range(self.cursor.line, self.cursor.col);
                for line_idx in (top..=bottom).rev() {
                    let line_len = self.buffers[self.current_buffer_idx].line_len(line_idx);
                    let close_col = (right + 1).min(line_len);
                    let open_col = left.min(line_len);
                    self.undo_stack.record_change(Change::insert(
                        line_idx,
                        close_col,
                        close.to_string(),
                    ));
                    self.buffers[self.current_buffer_idx].insert_char(line_idx, close_col, close);
                    self.undo_stack.record_change(Change::insert(
                        line_idx,
                        open_col,
                        open.to_string(),
                    ));
                    self.buffers[self.current_buffer_idx].insert_char(line_idx, open_col, open);
                }
                self.cursor.line = top;
                self.cursor.col = left;
            }
            _ => return,
        }

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        self.mode = Mode::Normal;
        self.clamp_cursor();
        self.scroll_to_cursor();
    }

    // ============================================
    // Text Object Operations
    // ============================================

    /// Find the range of a text object at the cursor position
    /// Returns Option<(start_line, start_col, end_line, end_col)>
    pub fn find_text_object_range(
        &self,
        text_object: TextObject,
    ) -> Option<(usize, usize, usize, usize)> {
        match text_object.object_type {
            TextObjectType::Word => self.find_word_object(text_object.modifier, false),
            TextObjectType::BigWord => self.find_word_object(text_object.modifier, true),
            TextObjectType::DoubleQuote => self.find_quote_object(text_object.modifier, '"'),
            TextObjectType::SingleQuote => self.find_quote_object(text_object.modifier, '\''),
            TextObjectType::BackTick => self.find_quote_object(text_object.modifier, '`'),
            TextObjectType::Paren => self.find_bracket_object(text_object.modifier, '(', ')'),
            TextObjectType::Brace => self.find_bracket_object(text_object.modifier, '{', '}'),
            TextObjectType::Bracket => self.find_bracket_object(text_object.modifier, '[', ']'),
            TextObjectType::AngleBracket => {
                self.find_bracket_object(text_object.modifier, '<', '>')
            }
            TextObjectType::Paragraph => self.find_paragraph_object(text_object.modifier),
            TextObjectType::Sentence => self.find_sentence_object(text_object.modifier),
            TextObjectType::Tag => self.find_tag_object(text_object.modifier),
        }
    }

    /// Find word text object boundaries
    fn find_word_object(
        &self,
        modifier: TextObjectModifier,
        big_word: bool,
    ) -> Option<(usize, usize, usize, usize)> {
        let line = self.cursor.line;
        let col = self.cursor.col;
        let line_text: String = self.buffers[self.current_buffer_idx]
            .line(line)?
            .chars()
            .collect();

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
            while start > 0 && !is_word_char(chars[start - 1]) && !chars[start - 1].is_whitespace()
            {
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
            while end < chars.len() - 1
                && !is_word_char(chars[end + 1])
                && !chars[end + 1].is_whitespace()
            {
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

    /// Find paragraph text object boundaries separated by blank lines.
    fn find_paragraph_object(
        &self,
        modifier: TextObjectModifier,
    ) -> Option<(usize, usize, usize, usize)> {
        let buffer = &self.buffers[self.current_buffer_idx];
        let line_count = buffer.len_lines();
        if line_count == 0 {
            return None;
        }

        let is_blank = |line: usize| -> bool {
            buffer
                .line(line)
                .map(|text| text.chars().all(char::is_whitespace))
                .unwrap_or(true)
        };

        let cursor_line = self.cursor.line.min(line_count.saturating_sub(1));
        let line = if is_blank(cursor_line) {
            let next = (cursor_line + 1..line_count).find(|&line| !is_blank(line));
            next.or_else(|| (0..cursor_line).rev().find(|&line| !is_blank(line)))?
        } else {
            cursor_line
        };

        let mut start_line = line;
        while start_line > 0 && !is_blank(start_line - 1) {
            start_line -= 1;
        }

        let mut end_line = line;
        while end_line + 1 < line_count && !is_blank(end_line + 1) {
            end_line += 1;
        }

        if modifier == TextObjectModifier::Around {
            if end_line + 1 < line_count && is_blank(end_line + 1) {
                end_line += 1;
            } else if start_line > 0 && is_blank(start_line - 1) {
                start_line -= 1;
            }
        }

        Some((start_line, 0, end_line, buffer.line_len(end_line)))
    }

    /// Find sentence text object boundaries.
    fn find_sentence_object(
        &self,
        modifier: TextObjectModifier,
    ) -> Option<(usize, usize, usize, usize)> {
        let buffer = &self.buffers[self.current_buffer_idx];
        let chars: Vec<char> = buffer.content().chars().collect();
        if chars.is_empty() {
            return None;
        }

        let ranges = Self::sentence_object_ranges(&chars);
        if ranges.is_empty() {
            return None;
        }

        let cursor_idx = buffer
            .line_col_to_char(self.cursor.line, self.cursor.col)
            .min(chars.len().saturating_sub(1));
        let &(start, body_end, around_end) = ranges
            .iter()
            .find(|(start, _, around_end)| cursor_idx >= *start && cursor_idx <= *around_end)
            .or_else(|| ranges.iter().find(|(start, _, _)| *start > cursor_idx))
            .or_else(|| ranges.last())?;

        let end = if modifier == TextObjectModifier::Around {
            around_end
        } else {
            body_end
        };
        let (start_line, start_col) = Self::char_index_to_line_col(buffer, start);
        let (end_line, end_col) = Self::char_index_to_line_col(buffer, end);
        Some((start_line, start_col, end_line, end_col))
    }

    fn sentence_object_ranges(chars: &[char]) -> Vec<(usize, usize, usize)> {
        let mut ranges = Vec::new();
        let Some(mut start) = Self::skip_sentence_space(chars, 0) else {
            return ranges;
        };

        while start < chars.len() {
            let mut scan = start;
            let mut body_end = None;
            while scan < chars.len() {
                if Self::is_sentence_end(chars[scan]) {
                    let after_closers = Self::skip_sentence_closers(chars, scan + 1);
                    if after_closers >= chars.len() || chars[after_closers].is_whitespace() {
                        body_end = Some(after_closers.saturating_sub(1));
                        break;
                    }
                    scan = after_closers;
                } else {
                    scan += 1;
                }
            }

            let body_end = body_end.unwrap_or_else(|| {
                let mut end = chars.len().saturating_sub(1);
                while end > start && chars[end].is_whitespace() {
                    end -= 1;
                }
                end
            });
            let mut around_end = body_end;
            while around_end + 1 < chars.len() && chars[around_end + 1].is_whitespace() {
                around_end += 1;
            }
            ranges.push((start, body_end, around_end));

            let Some(next_start) = Self::skip_sentence_space(chars, around_end + 1) else {
                break;
            };
            if next_start <= start {
                break;
            }
            start = next_start;
        }

        ranges
    }

    fn is_sentence_end(ch: char) -> bool {
        matches!(ch, '.' | '!' | '?')
    }

    fn skip_sentence_closers(chars: &[char], mut idx: usize) -> usize {
        while idx < chars.len() && matches!(chars[idx], '"' | '\'' | ')' | ']' | '}') {
            idx += 1;
        }
        idx
    }

    fn skip_sentence_space(chars: &[char], mut idx: usize) -> Option<usize> {
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        (idx < chars.len()).then_some(idx)
    }

    fn char_index_to_line_col(buffer: &Buffer, char_idx: usize) -> (usize, usize) {
        let mut remaining = char_idx;
        for line in 0..buffer.len_lines() {
            let len = buffer.line_len_including_newline(line);
            if remaining < len {
                return (line, remaining.min(buffer.line_len(line)));
            }
            remaining = remaining.saturating_sub(len);
        }

        let last_line = buffer.len_lines().saturating_sub(1);
        (last_line, buffer.line_len(last_line))
    }

    /// Find HTML/XML-style tag text object boundaries.
    fn find_tag_object(
        &self,
        modifier: TextObjectModifier,
    ) -> Option<(usize, usize, usize, usize)> {
        let buffer = &self.buffers[self.current_buffer_idx];
        let chars: Vec<char> = buffer.content().chars().collect();
        if chars.is_empty() {
            return None;
        }

        let tokens = Self::tag_tokens(&chars);
        let pairs = Self::tag_pairs(&tokens);
        let cursor_idx = buffer
            .line_col_to_char(self.cursor.line, self.cursor.col)
            .min(chars.len().saturating_sub(1));
        let &(open_idx, close_idx) = pairs
            .iter()
            .filter(|(open_idx, close_idx)| {
                cursor_idx >= tokens[*open_idx].start && cursor_idx <= tokens[*close_idx].end
            })
            .min_by_key(|(open_idx, close_idx)| {
                tokens[*close_idx]
                    .end
                    .saturating_sub(tokens[*open_idx].start)
            })?;

        let open = &tokens[open_idx];
        let close = &tokens[close_idx];
        let (start, end) = match modifier {
            TextObjectModifier::Inner => {
                let start = open.end + 1;
                let end = close.start.checked_sub(1)?;
                if start > end {
                    return None;
                }
                (start, end)
            }
            TextObjectModifier::Around => (open.start, close.end),
        };

        let (start_line, start_col) = Self::char_index_to_line_col(buffer, start);
        let (end_line, end_col) = Self::char_index_to_line_col(buffer, end);
        Some((start_line, start_col, end_line, end_col))
    }

    fn tag_pairs(tokens: &[TagToken]) -> Vec<(usize, usize)> {
        let mut stack: Vec<usize> = Vec::new();
        let mut pairs = Vec::new();

        for (idx, token) in tokens.iter().enumerate() {
            match token.kind {
                TagTokenKind::Open => stack.push(idx),
                TagTokenKind::SelfClosing => {}
                TagTokenKind::Close => {
                    if let Some(open_pos) = stack
                        .iter()
                        .rposition(|&open_idx| tokens[open_idx].name == token.name)
                    {
                        let open_idx = stack.remove(open_pos);
                        pairs.push((open_idx, idx));
                    }
                }
            }
        }

        pairs
    }

    fn tag_tokens(chars: &[char]) -> Vec<TagToken> {
        let mut tokens = Vec::new();
        let mut idx = 0;

        while idx < chars.len() {
            if chars[idx] != '<' {
                idx += 1;
                continue;
            }

            let Some(end) = (idx + 1..chars.len()).find(|&candidate| chars[candidate] == '>')
            else {
                break;
            };

            if let Some(token) = Self::parse_tag_token(chars, idx, end) {
                tokens.push(token);
            }
            idx = end + 1;
        }

        tokens
    }

    fn parse_tag_token(chars: &[char], start: usize, end: usize) -> Option<TagToken> {
        let mut idx = start + 1;
        while idx < end && chars[idx].is_whitespace() {
            idx += 1;
        }

        if idx >= end || matches!(chars[idx], '!' | '?') {
            return None;
        }

        let closing = chars[idx] == '/';
        if closing {
            idx += 1;
            while idx < end && chars[idx].is_whitespace() {
                idx += 1;
            }
        }

        let name_start = idx;
        while idx < end && Self::is_tag_name_char(chars[idx]) {
            idx += 1;
        }
        if idx == name_start {
            return None;
        }

        let name: String = chars[name_start..idx].iter().collect();
        let kind = if closing {
            TagTokenKind::Close
        } else if Self::is_self_closing_tag(chars, start, end) {
            TagTokenKind::SelfClosing
        } else {
            TagTokenKind::Open
        };

        Some(TagToken {
            name,
            start,
            end,
            kind,
        })
    }

    fn is_tag_name_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':')
    }

    fn is_self_closing_tag(chars: &[char], start: usize, end: usize) -> bool {
        let mut idx = end;
        while idx > start && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        idx > start && chars[idx - 1] == '/'
    }

    /// Find quote text object boundaries
    fn find_quote_object(
        &self,
        modifier: TextObjectModifier,
        quote: char,
    ) -> Option<(usize, usize, usize, usize)> {
        let line = self.cursor.line;
        let col = self.cursor.col;
        let line_text: String = self.buffers[self.current_buffer_idx]
            .line(line)?
            .chars()
            .collect();

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
    fn find_bracket_object(
        &self,
        modifier: TextObjectModifier,
        open_bracket: char,
        close_bracket: char,
    ) -> Option<(usize, usize, usize, usize)> {
        let cursor_line = self.cursor.line;
        let cursor_col = self.cursor.col;

        // Search backward for opening bracket
        let mut open_pos = None;
        let mut depth = 0;

        // First, search backward from cursor
        'outer: for line_idx in (0..=cursor_line).rev() {
            let line_text: String = self.buffers[self.current_buffer_idx]
                .line(line_idx)?
                .chars()
                .collect();
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
            let line_text: String = self.buffers[self.current_buffer_idx]
                .line(line_idx)?
                .chars()
                .collect();
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
                    Some((
                        open_line,
                        open_col + 1,
                        close_line,
                        close_col.saturating_sub(1),
                    ))
                }
            }
            TextObjectModifier::Around => Some((open_line, open_col, close_line, close_col)),
        }
    }

    /// Delete text object
    pub fn delete_text_object(&mut self, text_object: TextObject, register: Option<char>) {
        if let Some((start_line, start_col, end_line, end_col)) =
            self.find_text_object_range(text_object)
        {
            // Get text for register
            let text = self.get_range_text(start_line, start_col, end_line, end_col);

            // Record for undo
            self.begin_change();
            self.undo_stack
                .record_change(Change::delete(start_line, start_col, text.clone()));

            // Delete the range (inclusive)
            self.buffers[self.current_buffer_idx].delete_range(
                start_line,
                start_col,
                end_line,
                end_col + 1,
            );

            self.undo_stack.end_undo_group(start_line, start_col);

            // Store in register
            let is_small = !text.contains('\n');
            self.registers
                .delete(register, RegisterContent::Chars(text), is_small);

            // Move cursor to start
            self.cursor.line = start_line;
            self.cursor.col = start_col;
            self.clamp_cursor();
            self.scroll_to_cursor();
        }
    }

    /// Change text object (delete and enter insert mode)
    pub fn change_text_object(&mut self, text_object: TextObject, register: Option<char>) {
        if let Some((start_line, start_col, end_line, end_col)) =
            self.find_text_object_range(text_object)
        {
            // Get text for register
            let text = self.get_range_text(start_line, start_col, end_line, end_col);

            // Record for undo (will be continued in insert mode)
            self.begin_change();
            self.undo_stack
                .record_change(Change::delete(start_line, start_col, text.clone()));

            // Delete the range (inclusive)
            self.buffers[self.current_buffer_idx].delete_range(
                start_line,
                start_col,
                end_line,
                end_col + 1,
            );

            // Store in register
            let is_small = !text.contains('\n');
            self.registers
                .delete(register, RegisterContent::Chars(text), is_small);

            // Enter insert mode (undo group stays open)
            self.enter_insert_mode_at_change(start_line, start_col);
        }
    }

    /// Yank text object
    pub fn yank_text_object(&mut self, text_object: TextObject, register: Option<char>) {
        if let Some((start_line, start_col, end_line, end_col)) =
            self.find_text_object_range(text_object)
        {
            let text = self.get_range_text(start_line, start_col, end_line, end_col);
            self.registers.yank(register, RegisterContent::Chars(text));
            self.set_status("Yanked");
        }
    }

    /// Select text object in visual mode
    pub fn select_text_object(&mut self, text_object: TextObject) {
        if let Some((start_line, start_col, end_line, end_col)) =
            self.find_text_object_range(text_object)
        {
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
        self.replace_chars(ch, 1);
    }

    /// Replace count characters at cursor with the given character (r with count)
    pub fn replace_chars(&mut self, ch: char, count: usize) {
        let count = count.max(1);
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        if line_len == 0 || self.cursor.col >= line_len {
            return;
        }

        let start_col = self.cursor.col;
        let end_col = (start_col + count - 1).min(line_len.saturating_sub(1));
        let old_text = self.get_range_text(self.cursor.line, start_col, self.cursor.line, end_col);
        if old_text.is_empty() {
            return;
        }
        let replacement: String = std::iter::repeat(ch)
            .take(old_text.chars().count())
            .collect();

        self.begin_change();
        self.undo_stack
            .record_change(Change::delete(self.cursor.line, start_col, old_text));
        self.undo_stack.record_change(Change::insert(
            self.cursor.line,
            start_col,
            replacement.clone(),
        ));

        self.buffers[self.current_buffer_idx].delete_range(
            self.cursor.line,
            start_col,
            self.cursor.line,
            end_col + 1,
        );
        self.buffers[self.current_buffer_idx].insert_str(self.cursor.line, start_col, &replacement);
        self.cursor.col = start_col + replacement.chars().count().saturating_sub(1);
        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        self.clamp_cursor();
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
        let next_line: String = self.buffers[self.current_buffer_idx]
            .line(self.cursor.line + 1)
            .map(|l| l.chars().collect())
            .unwrap_or_default();

        // Count leading whitespace to strip
        let leading_ws = next_line
            .chars()
            .take_while(|c| c.is_whitespace() && *c != '\n')
            .count();

        // Begin undo group
        self.begin_change();

        // Record deletion of the newline at end of current line (always happens)
        self.undo_stack.record_change(Change::delete(
            self.cursor.line,
            current_line_len,
            "\n".to_string(),
        ));

        // Record deletion of leading whitespace from next line (if any)
        if leading_ws > 0 {
            let ws: String = next_line.chars().take(leading_ws).collect();
            self.undo_stack
                .record_change(Change::delete(self.cursor.line, current_line_len, ws));
        }

        // Record insertion of single space ONLY if we will actually insert one
        // (space is only inserted when current line is non-empty AND next line has content)
        if current_line_len > 0
            && !next_line.is_empty()
            && !next_line.chars().all(|c| c.is_whitespace())
        {
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
                self.buffers[self.current_buffer_idx]
                    .delete_char(self.cursor.line, current_line_len);
            }
        }

        // Insert a single space if the current line didn't end at column 0
        if current_line_len > 0
            && !next_line.is_empty()
            && !next_line.chars().all(|c| c.is_whitespace())
        {
            self.buffers[self.current_buffer_idx].insert_char(
                self.cursor.line,
                current_line_len,
                ' ',
            );
            // Position cursor at the space
            self.cursor.col = current_line_len;
        } else if current_line_len > 0 {
            self.cursor.col = current_line_len.saturating_sub(1);
        } else {
            self.cursor.col = 0;
        }

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        self.clamp_cursor();
    }

    /// Join count lines total, matching Vim's J count behavior.
    pub fn join_lines_count(&mut self, count: usize) {
        let joins = count.max(2).saturating_sub(1);
        for _ in 0..joins {
            let before = self.buffers[self.current_buffer_idx].len_lines();
            self.join_lines();
            if self.buffers[self.current_buffer_idx].len_lines() == before {
                break;
            }
        }
    }

    /// Join current line with next line without inserting space (gJ command)
    pub fn join_lines_no_space(&mut self) {
        let total_lines = self.buffers[self.current_buffer_idx].len_lines();
        if self.cursor.line >= total_lines.saturating_sub(1) {
            // Already on last line, nothing to join
            return;
        }

        // Get current line length (before newline)
        let current_line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);

        // Begin undo group
        self.begin_change();

        // Record the deletion of newline for undo
        self.undo_stack.record_change(Change::delete(
            self.cursor.line,
            current_line_len,
            "\n".to_string(),
        ));

        // Delete the newline character at end of current line
        // This joins the lines together without adding space or removing whitespace
        self.buffers[self.current_buffer_idx].delete_char(self.cursor.line, current_line_len);

        // Position cursor at the join point
        self.cursor.col = current_line_len;

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        self.clamp_cursor();
    }

    /// Join count lines total without inserting spaces, matching gJ with count.
    pub fn join_lines_no_space_count(&mut self, count: usize) {
        let joins = count.max(2).saturating_sub(1);
        for _ in 0..joins {
            let before = self.buffers[self.current_buffer_idx].len_lines();
            self.join_lines_no_space();
            if self.buffers[self.current_buffer_idx].len_lines() == before {
                break;
            }
        }
    }

    // ============================================
    // Surround operations (vim-surround style)
    // ============================================

    /// Delete surrounding pair (ds command)
    pub fn delete_surrounding(&mut self, surround_char: char) {
        let (open, close) = Self::get_surround_pair(surround_char);

        // Find the surrounding pair
        if let Some((start_pos, end_pos)) = self.find_surrounding_pair(open, close) {
            self.begin_change();

            // Delete closing char first (so positions don't shift)
            self.undo_stack
                .record_change(Change::delete(end_pos.0, end_pos.1, close.to_string()));
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

            self.undo_stack
                .end_undo_group(self.cursor.line, self.cursor.col);
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
            self.begin_change();

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

            self.undo_stack
                .end_undo_group(self.cursor.line, self.cursor.col);
        } else {
            self.set_status(format!("No surrounding {} found", old_char));
        }
    }

    /// Add surrounding to text object (ys command)
    pub fn add_surrounding(&mut self, text_object: crate::input::TextObject, surround_char: char) {
        let (open, close) = Self::get_surround_pair(surround_char);

        // Find the range of the text object
        if let Some((start_line, start_col, end_line, end_col)) =
            self.find_text_object_range(text_object)
        {
            self.begin_change();

            // Insert closing char first (so start position doesn't shift if on same line)
            let close_col = if start_line == end_line {
                end_col + 1
            } else {
                end_col + 1
            };
            self.undo_stack
                .record_change(Change::insert(end_line, close_col, close.to_string()));
            self.buffers[self.current_buffer_idx].insert_char(end_line, close_col, close);

            // Insert opening char
            self.undo_stack
                .record_change(Change::insert(start_line, start_col, open.to_string()));
            self.buffers[self.current_buffer_idx].insert_char(start_line, start_col, open);

            self.undo_stack
                .end_undo_group(self.cursor.line, self.cursor.col);
        } else {
            self.set_status("Could not find text object");
        }
    }

    /// Add surrounding to a motion range (ys{motion}{char}).
    pub fn add_surrounding_motion(&mut self, motion: Motion, count: usize, surround_char: char) {
        let (open, close) = Self::get_surround_pair(surround_char);

        if let Some((start_line, start_col, end_line, end_col)) = self.motion_range(motion, count) {
            self.begin_change();
            self.undo_stack
                .record_change(Change::insert(end_line, end_col + 1, close.to_string()));
            self.buffers[self.current_buffer_idx].insert_char(end_line, end_col + 1, close);
            self.undo_stack
                .record_change(Change::insert(start_line, start_col, open.to_string()));
            self.buffers[self.current_buffer_idx].insert_char(start_line, start_col, open);
            self.cursor.line = start_line;
            self.cursor.col = start_col;
            self.undo_stack
                .end_undo_group(self.cursor.line, self.cursor.col);
            self.clamp_cursor();
            self.scroll_to_cursor();
        }
    }

    /// Add surrounding to the current line (yss{char}).
    pub fn add_surrounding_line(&mut self, surround_char: char) {
        let (open, close) = Self::get_surround_pair(surround_char);
        let line = self.cursor.line;
        let line_len = self.buffers[self.current_buffer_idx].line_len(line);

        self.begin_change();
        self.undo_stack
            .record_change(Change::insert(line, line_len, close.to_string()));
        self.buffers[self.current_buffer_idx].insert_char(line, line_len, close);
        self.undo_stack
            .record_change(Change::insert(line, 0, open.to_string()));
        self.buffers[self.current_buffer_idx].insert_char(line, 0, open);
        self.cursor.col = 0;
        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        self.clamp_cursor();
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

        self.begin_change();

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

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
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
            self.undo_stack
                .record_change(Change::delete(line_num, 0, line_str.to_string()));

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

            self.undo_stack
                .record_change(Change::insert(line_num, 0, new_line.clone()));
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
            self.undo_stack
                .record_change(Change::delete(line_num, 0, line_str.to_string()));

            let new_line = format!("{}{}", indent, content);

            // Delete old content and insert new
            let old_len = self.buffers[self.current_buffer_idx].line_len(line_num);
            for _ in 0..old_len {
                self.buffers[self.current_buffer_idx].delete_char(line_num, 0);
            }

            self.undo_stack
                .record_change(Change::insert(line_num, 0, new_line.clone()));
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

        self.begin_change();

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
                self.undo_stack
                    .record_change(Change::insert(line_num, 0, indent_str.clone()));

                // Insert the indentation at the beginning
                self.buffers[self.current_buffer_idx].insert_str(line_num, 0, &indent_str);
            }
        }

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
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

        self.begin_change();

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
                self.undo_stack
                    .record_change(Change::delete(line_num, 0, deleted_text));

                // Delete the leading whitespace
                for _ in 0..spaces_to_remove {
                    self.buffers[self.current_buffer_idx].delete_char(line_num, 0);
                }
            }
        }

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
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

    /// Auto-indent a range of lines based on surrounding delimiters and syntax when available.
    pub fn auto_indent_lines(&mut self, start_line: usize, end_line: usize) {
        let buffer = &self.buffers[self.current_buffer_idx];
        if buffer.len_lines() == 0 {
            return;
        }

        let max_line = buffer.len_lines().saturating_sub(1);
        let start_line = start_line.min(max_line);
        let end_line = end_line.min(max_line);
        if start_line > end_line {
            return;
        }

        self.begin_change();
        let mut changed = false;

        for line_num in start_line..=end_line {
            let Some(line) = self.buffers[self.current_buffer_idx].line(line_num) else {
                continue;
            };
            let line_str: String = line.chars().collect();
            let line_without_newline = line_str.trim_end_matches('\n');

            if line_without_newline.trim().is_empty() {
                continue;
            }

            let old_indent_len = line_without_newline
                .chars()
                .take_while(|ch| *ch == ' ' || *ch == '\t')
                .count();
            let old_indent: String = line_without_newline.chars().take(old_indent_len).collect();
            let new_indent = " ".repeat(self.expected_auto_indent_for_line(line_num));

            if old_indent == new_indent {
                continue;
            }

            if !old_indent.is_empty() {
                self.undo_stack
                    .record_change(Change::delete(line_num, 0, old_indent.clone()));
                self.buffers[self.current_buffer_idx].delete_range(
                    line_num,
                    0,
                    line_num,
                    old_indent_len,
                );
            }

            if !new_indent.is_empty() {
                self.undo_stack
                    .record_change(Change::insert(line_num, 0, new_indent.clone()));
                self.buffers[self.current_buffer_idx].insert_str(line_num, 0, &new_indent);
            }

            changed = true;
        }

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);

        if changed {
            self.buffers[self.current_buffer_idx].mark_modified();
            self.parse_current_buffer();
        }

        self.cursor.line = start_line;
        self.cursor.col = self.find_first_non_blank(start_line);
        self.clamp_cursor();
    }

    fn expected_auto_indent_for_line(&mut self, line_num: usize) -> usize {
        let tab_width = self.settings.editor.tab_width;
        if tab_width == 0 {
            return 0;
        }

        let fallback = self.delimiter_auto_indent_for_line(line_num);

        if line_num == 0 {
            return if self.line_starts_with_closing_delimiter(line_num) {
                fallback.saturating_sub(tab_width)
            } else {
                fallback
            };
        }

        self.parse_current_buffer();

        let prev_line = line_num.saturating_sub(1);
        let prev_col = self.buffers[self.current_buffer_idx].line_len(prev_line);
        let Some(cursor_byte) = self.syntax.position_to_byte(prev_line, prev_col) else {
            return fallback;
        };
        let Some((tree, source)) = self.syntax.get_tree_and_source() else {
            return fallback;
        };

        let mut indent = crate::indent::calculate_indent(tree, source, cursor_byte, tab_width);

        if let Some(bracket) = self.line_start_closing_delimiter(line_num) {
            if let Some(current_byte) = self.syntax.position_to_byte(line_num, 0) {
                indent = crate::indent::calculate_closing_bracket_indent(
                    tree,
                    source,
                    current_byte,
                    bracket,
                )
                .unwrap_or_else(|| indent.saturating_sub(tab_width));
            } else {
                indent = indent.saturating_sub(tab_width);
            }
        }

        if indent == 0 && fallback > 0 {
            fallback
        } else {
            indent
        }
    }

    fn delimiter_auto_indent_for_line(&self, line_num: usize) -> usize {
        let tab_width = self.settings.editor.tab_width;
        let mut level = 0usize;

        for prev_line in 0..line_num {
            let Some(line) = self.buffers[self.current_buffer_idx].line(prev_line) else {
                continue;
            };
            let line_str: String = line.chars().collect();
            for ch in Self::code_portion_before_line_comment(&line_str).chars() {
                match ch {
                    '{' | '[' | '(' => level = level.saturating_add(1),
                    '}' | ']' | ')' => level = level.saturating_sub(1),
                    _ => {}
                }
            }
        }

        if self.line_starts_with_closing_delimiter(line_num) {
            level = level.saturating_sub(1);
        }

        level * tab_width
    }

    fn line_starts_with_closing_delimiter(&self, line_num: usize) -> bool {
        self.line_start_closing_delimiter(line_num).is_some()
    }

    fn line_start_closing_delimiter(&self, line_num: usize) -> Option<char> {
        let line = self.buffers[self.current_buffer_idx].line(line_num)?;
        let line_str: String = line.chars().collect();
        line_str
            .trim_start()
            .chars()
            .next()
            .and_then(|ch| match ch {
                '}' | ']' | ')' => Some(ch),
                _ => None,
            })
    }

    fn code_portion_before_line_comment(line: &str) -> &str {
        line.split_once("//")
            .map(|(code, _)| code)
            .unwrap_or(line)
            .trim_end_matches('\n')
    }

    /// Auto-indent with motion (={motion}).
    pub fn auto_indent_motion(&mut self, motion: Motion, count: usize) {
        let start_line = self.cursor.line;
        let text_rows = self.text_rows().max(1);
        let Some((target_line, _)) = apply_motion(
            &self.buffers[self.current_buffer_idx],
            motion,
            self.cursor.line,
            self.cursor.col,
            count,
            text_rows,
        ) else {
            return;
        };

        let range_start = start_line.min(target_line);
        let range_end = start_line.max(target_line);
        self.auto_indent_lines(range_start, range_end);
    }

    /// Indent current line and count-1 lines below (>> operation)
    pub fn indent_line(&mut self, count: usize) {
        let start_line = self.cursor.line;
        let end_line = start_line + count.saturating_sub(1);
        self.indent_lines(start_line, end_line);
    }

    /// Auto-indent current line and count-1 lines below (== operation).
    pub fn auto_indent_line(&mut self, count: usize) {
        let start_line = self.cursor.line;
        let end_line = start_line + count.saturating_sub(1);
        self.auto_indent_lines(start_line, end_line);
    }

    /// Increase indentation of the current line while preserving insert position.
    pub fn indent_current_line_in_insert_mode(&mut self) {
        let tab_width = self.settings.editor.tab_width;
        if tab_width == 0 {
            return;
        }

        let line = self.cursor.line;
        if line >= self.buffers[self.current_buffer_idx].len_lines() {
            return;
        }

        let indent = " ".repeat(tab_width);
        self.undo_stack
            .record_change(Change::insert(line, 0, indent.clone()));
        self.buffers[self.current_buffer_idx].insert_str(line, 0, &indent);
        self.cursor.col += tab_width;
        self.scroll_to_cursor();
    }

    /// Dedent current line and count-1 lines below (<< operation)
    pub fn dedent_line(&mut self, count: usize) {
        let start_line = self.cursor.line;
        let end_line = start_line + count.saturating_sub(1);
        self.dedent_lines(start_line, end_line);
    }

    /// Decrease indentation of the current line while preserving insert position.
    pub fn dedent_current_line_in_insert_mode(&mut self) {
        let tab_width = self.settings.editor.tab_width;
        if tab_width == 0 {
            return;
        }

        let line = self.cursor.line;
        let Some(line_text) = self.buffers[self.current_buffer_idx].line(line) else {
            return;
        };

        let mut chars_to_remove = 0;
        let mut indent_width = 0;
        for ch in line_text.chars() {
            if indent_width >= tab_width {
                break;
            }

            match ch {
                ' ' => {
                    chars_to_remove += 1;
                    indent_width += 1;
                }
                '\t' => {
                    chars_to_remove += 1;
                    break;
                }
                _ => break,
            }
        }

        if chars_to_remove == 0 {
            return;
        }

        let deleted_text: String = line_text.chars().take(chars_to_remove).collect();
        self.undo_stack
            .record_change(Change::delete(line, 0, deleted_text));

        for _ in 0..chars_to_remove {
            self.buffers[self.current_buffer_idx].delete_char(line, 0);
        }

        self.cursor.col = self.cursor.col.saturating_sub(chars_to_remove);
        self.scroll_to_cursor();
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

    /// Auto-indent text object.
    pub fn auto_indent_text_object(&mut self, text_object: TextObject) {
        if let Some((start_line, _, end_line, _)) = self.find_text_object_range(text_object) {
            self.auto_indent_lines(start_line, end_line);
        }
    }

    // ============================================
    // Case transformation operations
    // ============================================

    /// Transform the case of text in a range
    pub fn transform_case(
        &mut self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
        op: CaseOperator,
    ) {
        let text = self.get_range_text(start_line, start_col, end_line, end_col);
        if text.is_empty() {
            return;
        }

        let transformed: String = match op {
            CaseOperator::Lowercase => text.to_lowercase(),
            CaseOperator::Uppercase => text.to_uppercase(),
            CaseOperator::ToggleCase => text
                .chars()
                .map(|c| {
                    if c.is_lowercase() {
                        c.to_uppercase().next().unwrap_or(c)
                    } else if c.is_uppercase() {
                        c.to_lowercase().next().unwrap_or(c)
                    } else {
                        c
                    }
                })
                .collect(),
        };

        if text == transformed {
            return;
        }

        self.begin_change();

        // Record deletion of original text
        self.undo_stack
            .record_change(Change::delete(start_line, start_col, text.clone()));

        // Delete the original text
        self.buffers[self.current_buffer_idx].delete_range(
            start_line,
            start_col,
            end_line,
            end_col + 1,
        );

        // Record and insert the transformed text
        self.undo_stack
            .record_change(Change::insert(start_line, start_col, transformed.clone()));
        self.buffers[self.current_buffer_idx].insert_str(start_line, start_col, &transformed);

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
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

    /// Toggle case for count characters from cursor (~ command).
    pub fn toggle_case_chars(&mut self, count: usize) {
        let line_len = self.buffers[self.current_buffer_idx].line_len(self.cursor.line);
        if line_len == 0 || self.cursor.col >= line_len {
            return;
        }

        let start_col = self.cursor.col;
        let end_col = (start_col + count.max(1) - 1).min(line_len.saturating_sub(1));
        self.transform_case(
            self.cursor.line,
            start_col,
            self.cursor.line,
            end_col,
            CaseOperator::ToggleCase,
        );
        self.cursor.col = end_col;
        self.clamp_cursor();
    }

    /// Case transformation on current line (guu, gUU, g~~)
    pub fn case_line(&mut self, op: CaseOperator, count: usize) {
        let start_line = self.cursor.line;
        let buffer = &self.buffers[self.current_buffer_idx];
        let end_line =
            (start_line + count.saturating_sub(1)).min(buffer.len_lines().saturating_sub(1));

        self.begin_change();

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
                    CaseOperator::ToggleCase => line_str
                        .chars()
                        .map(|c| {
                            if c.is_lowercase() {
                                c.to_uppercase().next().unwrap_or(c)
                            } else if c.is_uppercase() {
                                c.to_lowercase().next().unwrap_or(c)
                            } else {
                                c
                            }
                        })
                        .collect(),
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
                    self.undo_stack
                        .record_change(Change::insert(line_num, 0, transformed.clone()));
                    self.buffers[self.current_buffer_idx].insert_str(line_num, 0, &transformed);
                }
            }
        }

        self.undo_stack
            .end_undo_group(self.cursor.line, self.cursor.col);
        self.buffers[self.current_buffer_idx].mark_modified();

        // Move cursor to first non-blank of start line
        self.cursor.line = start_line;
        self.cursor.col = self.find_first_non_blank(start_line);
        self.clamp_cursor();
    }

    /// Case transformation on text object (guiw, gUaw, etc.)
    pub fn case_text_object(&mut self, op: CaseOperator, text_object: TextObject) {
        if let Some((start_line, start_col, end_line, end_col)) =
            self.find_text_object_range(text_object)
        {
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

        self.marks
            .set(&buffer_key, path, name, self.cursor.line, self.cursor.col);
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
                        self.cursor.line = target_line.min(
                            self.buffers[self.current_buffer_idx]
                                .len_lines()
                                .saturating_sub(1),
                        );
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
            self.jump_list
                .record(current_path, self.cursor.line, self.cursor.col);

            self.cursor.line = mark.line.min(
                self.buffers[self.current_buffer_idx]
                    .len_lines()
                    .saturating_sub(1),
            );
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
                        self.cursor.line = target_line.min(
                            self.buffers[self.current_buffer_idx]
                                .len_lines()
                                .saturating_sub(1),
                        );
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
            self.jump_list
                .record(current_path, self.cursor.line, self.cursor.col);

            self.cursor.line = mark.line.min(
                self.buffers[self.current_buffer_idx]
                    .len_lines()
                    .saturating_sub(1),
            );
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
    fn find_surrounding_pair(
        &self,
        open: char,
        close: char,
    ) -> Option<((usize, usize), (usize, usize))> {
        let buffer = &self.buffers[self.current_buffer_idx];
        let line = self.cursor.line;
        let col = self.cursor.col;

        // For same open/close chars (quotes), use simpler logic
        if open == close {
            // Look on current line for quote pairs
            let line_content: String = buffer.line(line)?.chars().collect();
            let chars: Vec<char> = line_content.chars().collect();

            // Find all positions of the quote char
            let positions: Vec<usize> = chars
                .iter()
                .enumerate()
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
            let start_col = if l == start_search_line {
                start_search_col
            } else {
                0
            };

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
        // Record jump for "jump motions" (motions that move cursor significantly)
        // These are motions that should be tracked in the jump list for Ctrl+o/Ctrl+i
        let is_jump_motion = matches!(
            motion,
            Motion::FileStart
                | Motion::FileEnd
                | Motion::GotoLine(_)
                | Motion::ScreenTop
                | Motion::ScreenMiddle
                | Motion::ScreenBottom
                | Motion::ParagraphForward
                | Motion::ParagraphBackward
                | Motion::MatchingBracket
        );
        if is_jump_motion {
            self.record_jump();
        }

        // Handle screen-relative motions specially
        match motion {
            Motion::ScreenTop => {
                // H - move to top of visible screen (+ count lines from top)
                let target_line = self.viewport_offset + count.saturating_sub(1);
                let target_line = target_line.min(
                    self.buffers[self.current_buffer_idx]
                        .len_lines()
                        .saturating_sub(1),
                );
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
                let target_line = (self.viewport_offset + middle).min(
                    self.buffers[self.current_buffer_idx]
                        .len_lines()
                        .saturating_sub(1),
                );
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
                let target_line = target_line.min(
                    self.buffers[self.current_buffer_idx]
                        .len_lines()
                        .saturating_sub(1),
                );
                self.cursor.line = target_line;
                // Move to first non-blank
                self.cursor.col = self.find_first_non_blank(self.cursor.line);
                self.clamp_cursor();
                self.scroll_to_cursor();
            }
            Motion::DisplayLineDown => {
                self.move_display_lines(true, count);
            }
            Motion::DisplayLineUp => {
                self.move_display_lines(false, count);
            }
            Motion::DisplayLineStart => {
                self.move_to_display_line_position(DisplayLineTarget::Start);
            }
            Motion::DisplayLineEnd => {
                self.move_to_display_line_position(DisplayLineTarget::End);
            }
            Motion::DisplayLineFirstNonBlank => {
                self.move_to_display_line_position(DisplayLineTarget::FirstNonBlank);
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

    fn move_display_lines(&mut self, down: bool, count: usize) {
        if !self.settings.editor.wrap || self.effective_wrap_width() == 0 {
            let fallback = if down { Motion::Down } else { Motion::Up };
            if let Some((new_line, new_col)) = apply_motion(
                &self.buffers[self.current_buffer_idx],
                fallback,
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
            return;
        }

        for _ in 0..count.max(1) {
            if !self.move_display_line_once(down) {
                break;
            }
        }

        self.clamp_cursor();
        self.scroll_to_cursor();
    }

    fn move_display_line_once(&mut self, down: bool) -> bool {
        let tab_width = self.get_effective_tab_width();
        let wrap_width = self.effective_wrap_width();
        let current_line = self.cursor.line;
        let current_text = self
            .buffers
            .get(self.current_buffer_idx)
            .and_then(|buffer| buffer.line(current_line))
            .map(|line| line.to_string())
            .unwrap_or_default();
        let current_segments = Self::display_line_segments(&current_text, wrap_width, tab_width);
        let (segment_idx, display_col) = Self::display_segment_for_col(
            &current_text,
            &current_segments,
            self.cursor.col,
            tab_width,
        );

        if down {
            if segment_idx + 1 < current_segments.len() {
                let target_segment = current_segments[segment_idx + 1];
                self.cursor.col = Self::display_col_to_buffer_col(
                    &current_text,
                    target_segment,
                    display_col,
                    tab_width,
                );
                return true;
            }

            let next_line = current_line + 1;
            if next_line >= self.buffers[self.current_buffer_idx].len_lines() {
                return false;
            }

            let next_text = self
                .buffers
                .get(self.current_buffer_idx)
                .and_then(|buffer| buffer.line(next_line))
                .map(|line| line.to_string())
                .unwrap_or_default();
            let next_segments = Self::display_line_segments(&next_text, wrap_width, tab_width);
            self.cursor.line = next_line;
            self.cursor.col = Self::display_col_to_buffer_col(
                &next_text,
                next_segments[0],
                display_col,
                tab_width,
            );
            return true;
        }

        if segment_idx > 0 {
            let target_segment = current_segments[segment_idx - 1];
            self.cursor.col = Self::display_col_to_buffer_col(
                &current_text,
                target_segment,
                display_col,
                tab_width,
            );
            return true;
        }

        let Some(prev_line) = current_line.checked_sub(1) else {
            return false;
        };

        let prev_text = self
            .buffers
            .get(self.current_buffer_idx)
            .and_then(|buffer| buffer.line(prev_line))
            .map(|line| line.to_string())
            .unwrap_or_default();
        let prev_segments = Self::display_line_segments(&prev_text, wrap_width, tab_width);
        let target_segment = prev_segments[prev_segments.len().saturating_sub(1)];
        self.cursor.line = prev_line;
        self.cursor.col =
            Self::display_col_to_buffer_col(&prev_text, target_segment, display_col, tab_width);
        true
    }

    fn move_to_display_line_position(&mut self, target: DisplayLineTarget) {
        if !self.settings.editor.wrap || self.effective_wrap_width() == 0 {
            let fallback = match target {
                DisplayLineTarget::Start => Motion::LineStart,
                DisplayLineTarget::End => Motion::LineEnd,
                DisplayLineTarget::FirstNonBlank => Motion::FirstNonBlank,
            };
            if let Some((new_line, new_col)) = apply_motion(
                &self.buffers[self.current_buffer_idx],
                fallback,
                self.cursor.line,
                self.cursor.col,
                1,
                self.text_rows(),
            ) {
                self.cursor.line = new_line;
                self.cursor.col = new_col;
                self.clamp_cursor();
                self.scroll_to_cursor();
            }
            return;
        }

        let tab_width = self.get_effective_tab_width();
        let wrap_width = self.effective_wrap_width();
        let current_line = self.cursor.line;
        let current_text = self
            .buffers
            .get(self.current_buffer_idx)
            .and_then(|buffer| buffer.line(current_line))
            .map(|line| line.to_string())
            .unwrap_or_default();
        let segments = Self::display_line_segments(&current_text, wrap_width, tab_width);
        let (segment_idx, _) =
            Self::display_segment_for_col(&current_text, &segments, self.cursor.col, tab_width);
        let segment = segments[segment_idx];

        self.cursor.col = match target {
            DisplayLineTarget::Start => segment.start_col,
            DisplayLineTarget::End => segment.end_col.saturating_sub(1),
            DisplayLineTarget::FirstNonBlank => {
                Self::display_line_first_non_blank_col(&current_text, segment, tab_width)
            }
        };
        self.clamp_cursor();
        self.scroll_to_cursor();
    }

    fn display_line_first_non_blank_col(
        line: &str,
        segment: DisplayLineSegment,
        tab_width: usize,
    ) -> usize {
        if segment.end_col <= segment.start_col {
            return segment.start_col;
        }

        line.chars()
            .enumerate()
            .skip(segment.start_col)
            .take(segment.end_col - segment.start_col)
            .find_map(|(idx, ch)| {
                if ch == '\n' {
                    None
                } else if Self::display_char_width(ch, tab_width) > 0 && !ch.is_whitespace() {
                    Some(idx)
                } else {
                    None
                }
            })
            .unwrap_or(segment.start_col)
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
        // Initialize preview for the first selected item
        self.update_finder_preview();
    }

    /// Open the fuzzy finder in git changes mode
    pub fn open_finder_git_changes(&mut self) {
        use crate::finder::FinderItem;

        let Some(repo) = &self.git_repo else {
            self.set_status("Not in a Git repository");
            return;
        };

        let Some(workdir) = repo.workdir() else {
            self.set_status("Not in a Git repository");
            return;
        };

        let mut statuses: Vec<_> = repo.file_statuses().into_iter().collect();
        statuses.sort_by(|(left_path, left_status), (right_path, right_status)| {
            left_status
                .picker_sort_rank()
                .cmp(&right_status.picker_sort_rank())
                .then_with(|| left_path.cmp(right_path))
        });

        let items: Vec<_> = statuses
            .into_iter()
            .map(|(path, status)| {
                let relative_path = path.strip_prefix(workdir).unwrap_or(&path);
                let display = format!(
                    "{} {}",
                    status.picker_prefix(),
                    relative_path.to_string_lossy()
                );
                FinderItem::new(display, path)
                    .with_git_status(status)
                    .with_icon("GT")
            })
            .collect();

        if items.is_empty() {
            self.set_status("No Git changes");
            return;
        }

        self.finder.open_git_changes(items);
        self.mode = Mode::Finder;
        self.update_finder_preview();
    }

    /// Open the fuzzy finder in buffer mode
    pub fn open_finder_buffers(&mut self) {
        let buffer_info: Vec<(usize, String, std::path::PathBuf)> = self
            .buffers
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

    /// Open the fuzzy finder in harpoon mode
    pub fn open_finder_harpoon(&mut self) {
        let files: Vec<_> = self.harpoon.files().to_vec();
        self.finder.open_harpoon(files);
        self.mode = Mode::Finder;
    }

    /// Open the fuzzy finder in terminal session mode
    pub fn open_terminal_picker(&mut self) {
        use crate::finder::FinderItem;

        let terminal_items: Vec<FinderItem> = self
            .floating_terminal
            .session_infos()
            .into_iter()
            .map(|session| {
                let marker = if session.active { "*" } else { " " };
                let title = session
                    .metadata
                    .filter(|title| title != &session.name)
                    .map(|title| format!("  {}", title))
                    .unwrap_or_default();
                let display = format!(
                    "{} {:>2}  {:<18} #{} {:<7}{}",
                    marker, session.position, session.name, session.id, session.state, title
                );
                let mut item = FinderItem::new(display, std::path::PathBuf::new())
                    .with_terminal_session_position(session.position)
                    .with_icon("TR");
                item.score = session.position as u32;
                item
            })
            .collect();

        self.finder.open_terminals(terminal_items);
        self.mode = Mode::Finder;
    }

    /// Open the fuzzy finder in keymaps cheatsheet mode (read-only).
    pub fn open_keymaps_picker(&mut self) {
        self.open_keymaps_picker_with_query("");
    }

    /// Open the fuzzy finder in keymaps mode with a pre-filled filter query.
    pub fn open_keymaps_picker_with_query(&mut self, query: &str) {
        let items = crate::finder::keymap_finder_items(&self.settings.keymap);
        self.finder.open_keymaps_with_query(items, query);
        self.mode = Mode::Finder;
    }

    /// Open the fuzzy finder in marks mode
    pub fn open_finder_marks(&mut self) {
        use crate::finder::MarkInfo;

        let mut marks_info: Vec<MarkInfo> = Vec::new();
        let current_file_name = self.buffer().display_name().to_string();
        let current_file_path = self.buffer().path.clone();

        // Get buffer key for local marks
        let buffer_key = self
            .buffer()
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("buffer_{}", self.current_buffer_index()));

        // Collect local marks (a-z) from current buffer
        for (name, mark) in self.marks.get_local_marks(&buffer_key) {
            marks_info.push(MarkInfo {
                name,
                line: mark.line,
                col: mark.col,
                file_path: current_file_path.clone(),
                file_name: current_file_name.clone(),
            });
        }

        // Collect global marks (A-Z)
        for (name, mark) in self.marks.get_global_marks() {
            let file_name = mark
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            marks_info.push(MarkInfo {
                name,
                line: mark.line,
                col: mark.col,
                file_path: mark.path.clone(),
                file_name,
            });
        }

        // Sort marks alphabetically
        marks_info.sort_by_key(|m| m.name);

        if marks_info.is_empty() {
            self.set_status("No marks set");
            return;
        }

        self.finder.open_marks(marks_info);
        self.mode = Mode::Finder;
    }

    /// Open the fuzzy finder in grep mode with word under cursor pre-filled
    pub fn open_finder_grep_word(&mut self) {
        if let Some(word) = self.get_word_under_cursor() {
            let root = self.working_directory();
            self.finder.open_grep_with_query(&root, &word);
            self.mode = Mode::Finder;
        } else {
            self.set_status("No word under cursor".to_string());
        }
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
            a_severity
                .cmp(&b_severity)
                .then_with(|| a.line.cmp(&b.line))
        });

        for (uri, diag) in all_diags {
            // Get relative path from URI
            let path = if uri.starts_with("file://") {
                std::path::PathBuf::from(&uri[7..])
            } else {
                std::path::PathBuf::from(uri)
            };

            let rel_path = path.strip_prefix(&cwd).unwrap_or(&path).to_string_lossy();

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

            let item = FinderItem::new(display, path.clone()).with_line(diag.line + 1); // 1-indexed for jumping

            diagnostic_items.push(item);
        }

        self.finder.open_diagnostics(diagnostic_items);
        self.mode = Mode::Finder;
    }

    /// Close the finder and return to normal mode
    pub fn close_finder(&mut self) {
        self.mode = Mode::Normal;
        self.clear_status();
        self.finder.cancel_grep_search();
        self.finder.clear_preview_cache();
    }

    /// Update the preview syntax highlighting for the currently selected finder item
    pub fn update_finder_preview(&mut self) {
        if !self.finder.preview_enabled || !self.finder.mode_supports_preview() {
            return;
        }

        if self.finder.mode == crate::finder::FinderMode::GitChanges {
            self.update_git_changes_preview();
            return;
        }

        // Get the selected item's path
        let (selected_path, selected_line) = match self.finder.selected_item() {
            Some(item) => (item.path.clone(), item.line),
            None => return,
        };

        // Check if we need to update (path changed)
        if self.finder.preview_path.as_ref() == Some(&selected_path)
            && (self.finder.mode != crate::finder::FinderMode::Grep
                || self.finder.preview_line == selected_line)
        {
            return;
        }

        // Update the preview content (this sets preview_path)
        self.finder.update_preview_content();

        // Skip syntax parsing for error messages / non-file content
        // These start with '(' like "(Binary file...)", "(Directory)", "(Unable to read...)"
        if self.finder.preview_content.len() <= 1 {
            if let Some(first) = self.finder.preview_content.first() {
                if first.starts_with('(') {
                    return;
                }
            }
        }

        // Set up syntax highlighting for the preview file
        self.preview_syntax.set_language_from_path(&selected_path);

        // Sync theme
        self.preview_syntax.sync_theme(self.theme_manager.theme());

        // Parse the content
        let content = self.finder.preview_content.join("\n");
        self.preview_syntax.parse_string(&content);
    }

    fn update_git_changes_preview(&mut self) {
        let Some(item) = self.finder.selected_item().cloned() else {
            self.finder.clear_preview_cache();
            self.reset_preview_syntax();
            return;
        };
        let Some(status) = item.git_status else {
            self.finder.clear_preview_cache();
            self.reset_preview_syntax();
            return;
        };

        if self.finder.preview_path.as_ref() == Some(&item.path) {
            return;
        }

        self.reset_preview_syntax();

        const GIT_CHANGES_PREVIEW_LINES: usize = 150;
        let content = self
            .git_repo
            .as_ref()
            .map(|repo| repo.diff_preview(&item.path, status, GIT_CHANGES_PREVIEW_LINES))
            .unwrap_or_else(|| vec!["Not in a Git repository".to_string()]);

        self.finder.set_preview_content(item.path, content);
    }

    fn reset_preview_syntax(&mut self) {
        self.preview_syntax = SyntaxManager::new();
        self.preview_syntax.sync_theme(self.theme_manager.theme());
    }

    // === File Explorer Methods ===

    /// Toggle the file explorer sidebar
    pub fn toggle_explorer(&mut self) {
        self.explorer.toggle();
        self.update_pane_rects();
        if self.explorer.visible {
            self.refresh_explorer_git_statuses();
            self.mode = Mode::Explorer;
        } else {
            self.mode = Mode::Normal;
        }
    }

    /// Open the file explorer sidebar
    pub fn open_explorer(&mut self) {
        self.explorer.show();
        self.refresh_explorer_git_statuses();
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
        if idx == self.current_buffer_idx {
            return true;
        }

        self.remember_current_file_as_alternate();
        self.save_pane_state();
        self.current_buffer_idx = idx;
        self.load_current_undo_stack();
        if self.active_pane < self.panes.len() {
            self.panes[self.active_pane].buffer_idx = idx;
            self.panes[self.active_pane].cursor = Cursor::default();
            self.panes[self.active_pane].viewport_offset = 0;
            self.panes[self.active_pane].h_offset = 0;
        }
        self.cursor = Cursor::default();
        self.viewport_offset = 0;
        self.h_offset = 0;
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

        let (start_line, start_col, mut end_line, mut end_col) =
            if (target_line, target_col) < (self.cursor.line, self.cursor.col) {
                (target_line, target_col, self.cursor.line, self.cursor.col)
            } else {
                (self.cursor.line, self.cursor.col, target_line, target_col)
            };

        if !inclusive {
            if end_line == start_line {
                end_col = end_col.saturating_sub(1).max(start_col);
            } else if end_col == 0 {
                let prev_line = end_line.saturating_sub(1);
                let prev_len =
                    self.buffers[self.current_buffer_idx].line_len_including_newline(prev_line);
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
        self.set_status(format!(
            "{} references - j/k to navigate, Enter to go, Esc to close",
            count
        ));
    }

    /// Hide the references picker
    pub fn hide_references_picker(&mut self) {
        self.references_picker = None;
    }

    /// Show the code actions picker with the given actions
    pub fn show_code_actions_picker(&mut self, actions: Vec<CodeActionItem>) {
        let count = actions.len();
        self.code_actions_picker = Some(CodeActionsPicker::new(actions));
        self.set_status(format!(
            "{} code actions - j/k to navigate, Enter to apply, Esc to close",
            count
        ));
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
        let mut skipped_file_edits = 0;
        let current_uri = self.current_buffer_uri();

        // Apply edits from the selected action
        for (uri, edits) in &action.edits {
            if current_uri.as_ref() == Some(uri) {
                self.apply_text_edits(edits);
                total_edits += edits.len();
            } else {
                skipped_file_edits += edits.len();
            }
        }

        if total_edits > 0 {
            Some(format!("Applied '{}' ({} edits)", title, total_edits))
        } else if skipped_file_edits > 0 {
            Some(format!(
                "Action '{}' edits another file ({} edits skipped)",
                title, skipped_file_edits
            ))
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

#[cfg(test)]
mod tests {
    use super::{Editor, JumpList, Mode};
    use crate::input::Motion;
    use crate::lsp::types::{
        CodeActionItem, CompletionItem, CompletionKind, Diagnostic, DiagnosticSeverity, TextEdit,
    };
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos))
    }

    fn commit_file(repo: &git2::Repository, relative_path: &Path, message: &str) {
        let signature =
            git2::Signature::now("Nevi Test", "nevi-test@example.com").expect("signature");
        let mut index = repo.index().expect("index");
        index.add_path(relative_path).expect("add file");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");

        if let Some(parent_id) = repo.head().ok().and_then(|head| head.target()) {
            let parent = repo.find_commit(parent_id).expect("find parent");
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[&parent],
            )
            .expect("commit");
        } else {
            repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
                .expect("initial commit");
        }
    }

    #[test]
    fn open_url_under_cursor_uses_url_token_without_trailing_punctuation() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("see (https://example.test/docs?q=nevi).\n");
        editor.cursor.line = 0;
        editor.cursor.col = "see (https://example".chars().count();

        let mut opened = Vec::new();
        let opened_url = editor
            .open_url_under_cursor_with(|url| {
                opened.push(url.to_string());
                Ok(())
            })
            .expect("open url");

        assert_eq!(opened_url, "https://example.test/docs?q=nevi");
        assert_eq!(opened, vec!["https://example.test/docs?q=nevi"]);
    }

    #[test]
    fn equalize_windows_restores_even_vertical_pane_rects() {
        let mut editor = Editor::default();
        editor.set_size(100, 20);
        editor.vsplit(None).expect("first split");
        editor.vsplit(None).expect("second split");

        editor.panes[0].rect = super::Rect::new(0, 0, 10, 5);
        editor.panes[1].rect = super::Rect::new(10, 0, 70, 5);
        editor.panes[2].rect = super::Rect::new(80, 0, 20, 5);

        editor.equalize_windows();

        let widths: Vec<u16> = editor.panes.iter().map(|pane| pane.rect.width).collect();
        assert_eq!(widths, vec![33, 33, 34]);
        assert_eq!(editor.status_message.as_deref(), Some("Windows equalized"));
    }

    #[test]
    fn rotate_windows_moves_panes_down_right_and_up_left() {
        let tmp = unique_temp_dir("nevi_rotate_windows");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let first = tmp.join("first.txt");
        let second = tmp.join("second.txt");
        let third = tmp.join("third.txt");
        std::fs::write(&first, "first\n").expect("write first");
        std::fs::write(&second, "second\n").expect("write second");
        std::fs::write(&third, "third\n").expect("write third");

        let mut editor = Editor::default();
        editor.open_file(first).expect("open first");
        editor.vsplit(Some(second)).expect("split second");
        editor.vsplit(Some(third)).expect("split third");
        editor.prev_pane();
        let active_buffer = editor.panes[editor.active_pane].buffer_idx;

        assert_eq!(
            editor
                .panes
                .iter()
                .map(|pane| pane.buffer_idx)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );

        editor.rotate_windows_down_right();

        assert_eq!(
            editor
                .panes
                .iter()
                .map(|pane| pane.buffer_idx)
                .collect::<Vec<_>>(),
            vec![2, 0, 1]
        );
        assert_eq!(editor.panes[editor.active_pane].buffer_idx, active_buffer);
        assert_eq!(editor.status_message.as_deref(), Some("Windows rotated"));

        editor.rotate_windows_up_left();

        assert_eq!(
            editor
                .panes
                .iter()
                .map(|pane| pane.buffer_idx)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(editor.panes[editor.active_pane].buffer_idx, active_buffer);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Windows rotated reverse")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn display_line_motions_move_within_wrapped_lines() {
        let mut editor = Editor::default();
        editor.set_size(40, 12);
        editor.settings.editor.wrap = true;
        editor.settings.editor.wrap_width = 5;
        editor.settings.editor.line_numbers = false;
        editor.replace_buffer_content("abcdefghij\nklmno\n");
        editor.cursor.line = 0;
        editor.cursor.col = 1;

        editor.apply_motion(Motion::DisplayLineDown, 1);

        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 6));

        editor.apply_motion(Motion::DisplayLineDown, 1);

        assert_eq!((editor.cursor.line, editor.cursor.col), (1, 1));

        editor.apply_motion(Motion::DisplayLineUp, 2);

        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 1));
    }

    #[test]
    fn display_line_horizontal_motions_target_current_wrapped_row() {
        let mut editor = Editor::default();
        editor.set_size(40, 12);
        editor.settings.editor.wrap = true;
        editor.settings.editor.wrap_width = 5;
        editor.settings.editor.line_numbers = false;
        editor.replace_buffer_content("  abcdefghij\n");
        editor.cursor.line = 0;

        editor.cursor.col = 6;
        editor.apply_motion(Motion::DisplayLineStart, 1);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 5));

        editor.cursor.col = 6;
        editor.apply_motion(Motion::DisplayLineEnd, 1);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 7));

        editor.cursor.col = 6;
        editor.apply_motion(Motion::DisplayLineFirstNonBlank, 1);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 5));

        editor.cursor.col = 3;
        editor.apply_motion(Motion::DisplayLineFirstNonBlank, 1);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 2));
    }

    #[test]
    fn exchange_window_swaps_current_with_next_or_previous_at_end() {
        let tmp = unique_temp_dir("nevi_exchange_window");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let first = tmp.join("first.txt");
        let second = tmp.join("second.txt");
        let third = tmp.join("third.txt");
        std::fs::write(&first, "first\n").expect("write first");
        std::fs::write(&second, "second\n").expect("write second");
        std::fs::write(&third, "third\n").expect("write third");

        let mut editor = Editor::default();
        editor.open_file(first).expect("open first");
        editor.vsplit(Some(second)).expect("split second");
        editor.vsplit(Some(third)).expect("split third");
        editor.prev_pane();
        let active_buffer = editor.panes[editor.active_pane].buffer_idx;

        assert_eq!(
            editor
                .panes
                .iter()
                .map(|pane| pane.buffer_idx)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );

        editor.exchange_window_with_next();

        assert_eq!(
            editor
                .panes
                .iter()
                .map(|pane| pane.buffer_idx)
                .collect::<Vec<_>>(),
            vec![0, 2, 1]
        );
        assert_eq!(editor.active_pane, 2);
        assert_eq!(editor.panes[editor.active_pane].buffer_idx, active_buffer);
        assert_eq!(editor.status_message.as_deref(), Some("Windows exchanged"));

        editor.exchange_window_with_next();

        assert_eq!(
            editor
                .panes
                .iter()
                .map(|pane| pane.buffer_idx)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(editor.active_pane, 1);
        assert_eq!(editor.panes[editor.active_pane].buffer_idx, active_buffer);
        assert_eq!(editor.status_message.as_deref(), Some("Windows exchanged"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn undo_history_follows_the_active_buffer() {
        let tmp = unique_temp_dir("nevi_undo_per_buffer");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let first = tmp.join("first.txt");
        let second = tmp.join("second.txt");
        std::fs::write(&first, "abc\n").expect("write first");
        std::fs::write(&second, "xyz\n").expect("write second");

        let mut editor = Editor::default();
        editor.open_file(first).expect("open first");
        editor.enter_insert_mode_end();
        editor.insert_char('!');
        editor.enter_normal_mode();
        assert_eq!(editor.buffer().content(), "abc!\n");

        editor.open_file(second).expect("open second");
        editor.undo();
        assert_eq!(editor.buffer().content(), "xyz\n");

        assert!(editor.switch_to_buffer(0));
        editor.undo();
        assert_eq!(editor.buffer().content(), "abc\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn markdown_preview_opens_for_markdown_buffers_and_snapshots_content() {
        let tmp = unique_temp_dir("nevi_markdown_preview_snapshot");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let markdown = tmp.join("notes.md");
        std::fs::write(&markdown, "# Seed\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.open_file(markdown).expect("open markdown buffer");
        editor.replace_buffer_content("# Before\n");

        editor.open_markdown_preview().expect("open preview");
        assert_eq!(
            editor.markdown_preview.as_ref().expect("preview").lines[0].plain_text(),
            "Before"
        );

        editor.replace_buffer_content("# After\n");
        assert_eq!(
            editor.markdown_preview.as_ref().expect("preview").lines[0].plain_text(),
            "Before"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn markdown_preview_rejects_non_markdown_buffers() {
        let tmp = unique_temp_dir("nevi_markdown_preview_non_markdown");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let rust = tmp.join("main.rs");
        std::fs::write(&rust, "fn main() {}\n").expect("write rust");

        let mut editor = Editor::default();
        editor.open_file(rust).expect("open rust buffer");

        let result = editor.open_markdown_preview();

        assert!(result.is_err());
        assert!(editor.markdown_preview.is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn markdown_preview_scroll_is_clamped_to_visible_content() {
        let tmp = unique_temp_dir("nevi_markdown_preview_scroll");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let markdown = tmp.join("notes.md");
        std::fs::write(&markdown, "# Seed\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.open_file(markdown).expect("open markdown buffer");
        editor.replace_buffer_content(&(0..20).map(|i| format!("line {i}\n")).collect::<String>());
        editor.open_markdown_preview().expect("open preview");

        editor.scroll_markdown_preview(100, 5);
        assert_eq!(editor.markdown_preview.as_ref().unwrap().scroll, 15);

        editor.scroll_markdown_preview(-100, 5);
        assert_eq!(editor.markdown_preview.as_ref().unwrap().scroll, 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn undo_normal_x_restores_original_cursor_position() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abc\n");
        editor.cursor.col = 2;

        editor.delete_char_before_normal();
        assert_eq!(editor.buffer().content(), "ac\n");
        assert_eq!(editor.cursor.col, 1);

        editor.undo();
        assert_eq!(editor.buffer().content(), "abc\n");
        assert_eq!(editor.cursor.line, 0);
        assert_eq!(editor.cursor.col, 2);
    }

    #[test]
    fn autopair_backspace_keeps_insert_undo_group_open() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("");
        editor.enter_insert_mode();
        editor.insert_char('(');
        editor.insert_char(')');
        editor.cursor.col = 1;

        editor.delete_char_before();
        editor.delete_char_at_in_current_group();
        editor.insert_char('a');
        editor.enter_normal_mode();
        assert_eq!(editor.buffer().content(), "a");

        editor.undo();
        assert_eq!(editor.buffer().content(), "");
    }

    #[test]
    fn external_buffer_replacement_is_undoable() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("abc\n");

        editor.replace_buffer_content_with_undo("ABC\n");
        assert_eq!(editor.buffer().content(), "ABC\n");

        editor.undo();
        assert_eq!(editor.buffer().content(), "abc\n");
    }

    #[test]
    fn jump_list_back_skips_current_snapshot_on_first_press() {
        let path = Some(PathBuf::from("/tmp/file.rs"));
        let mut jumps = JumpList::default();

        jumps.record(path.clone(), 10, 0);
        jumps.record(path.clone(), 20, 0);

        // Simulate cursor currently at a new location not yet in jump list.
        let first = jumps.go_back(path.clone(), 30, 0).expect("first back");
        assert_eq!(first.line, 20, "first Ctrl+o should go to previous jump");

        // Ctrl+i should return to the current snapshot we saved (line 30).
        let forward = jumps.go_forward().expect("forward after back");
        assert_eq!(forward.line, 30, "Ctrl+i should return to current position");
    }

    #[test]
    fn jump_list_back_then_back_reaches_older_entries() {
        let path = Some(PathBuf::from("/tmp/file.rs"));
        let mut jumps = JumpList::default();

        jumps.record(path.clone(), 10, 0);
        jumps.record(path.clone(), 20, 0);
        jumps.record(path.clone(), 40, 0);

        let first = jumps.go_back(path.clone(), 50, 0).expect("first back");
        assert_eq!(first.line, 40);

        let second = jumps.go_back(path.clone(), 40, 0).expect("second back");
        assert_eq!(second.line, 20);
    }

    #[test]
    fn visual_delete_to_file_end_removes_all_text_with_trailing_newline() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("const a = 1;\nconst b = 2;\n");

        editor.enter_visual_mode();
        editor.apply_motion(Motion::FileEnd, 1);
        editor.visual_delete();

        assert_eq!(editor.buffer().content(), "");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));
    }

    #[test]
    fn visual_delete_to_true_eof_removes_all_text_without_trailing_newline() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("const a = 1;\nconst b = 2;");

        editor.enter_visual_mode();
        editor.apply_motion(Motion::FileEnd, 1);
        editor.apply_motion(Motion::LineEnd, 1);
        editor.visual_delete();

        assert_eq!(editor.buffer().content(), "");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));
    }

    #[test]
    fn visual_line_delete_to_file_end_still_removes_all_text() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("const a = 1;\nconst b = 2;\n");

        editor.enter_visual_line_mode();
        editor.apply_motion(Motion::FileEnd, 1);
        editor.visual_delete();

        assert_eq!(editor.buffer().content(), "");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));
    }

    #[test]
    fn buffer_navigation_keeps_active_pane_on_selected_buffer() {
        let tmp = unique_temp_dir("nevi_buffer_nav");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let first = tmp.join("first.rs");
        let second = tmp.join("second.rs");
        std::fs::write(&first, "fn first() {}\n").expect("write first");
        std::fs::write(&second, "fn second() {}\n").expect("write second");

        let mut editor = Editor::default();
        editor.open_file(first).expect("open first");
        editor.open_file(second).expect("open second");

        editor.prev_buffer();

        let active_pane = &editor.panes()[editor.active_pane_idx()];
        assert_eq!(active_pane.buffer_idx, editor.current_buffer_index());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn closing_buffer_remaps_all_panes_to_valid_buffer_indices() {
        let tmp = unique_temp_dir("nevi_close_buffer");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let first = tmp.join("first.rs");
        let second = tmp.join("second.rs");
        let third = tmp.join("third.rs");
        std::fs::write(&first, "fn first() {}\n").expect("write first");
        std::fs::write(&second, "fn second() {}\n").expect("write second");
        std::fs::write(&third, "fn third() {}\n").expect("write third");

        let mut editor = Editor::default();
        editor.open_file(first).expect("open first");
        editor.open_file(second).expect("open second");
        editor.open_file(third).expect("open third");

        editor.vsplit(None).expect("split");
        editor.switch_to_buffer(1);
        editor.close_current_buffer();

        assert!(editor
            .panes()
            .iter()
            .all(|pane| pane.buffer_idx < editor.buffer_count()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn update_all_git_diffs_recomputes_against_new_head() {
        let tmp = unique_temp_dir("nevi_git_refresh_head");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let root = tmp.canonicalize().expect("canonical temp dir");
        let path = root.join("tracked.rs");
        std::fs::write(&path, "old\n").expect("write tracked file");
        let repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&repo, Path::new("tracked.rs"), "initial");

        let mut editor = Editor::default();
        editor.set_project_root(root.clone());
        editor.init_git();
        editor.open_file(path.clone()).expect("open tracked file");

        editor.replace_buffer_content("new\n");
        editor.save().expect("save modified file");
        assert_eq!(
            editor.git_status_for_line(0),
            Some(crate::git::GitLineStatus::Modified)
        );

        commit_file(&repo, Path::new("tracked.rs"), "external commit");
        editor.update_all_git_diffs();

        assert_eq!(editor.git_status_for_line(0), None);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn git_changes_picker_reports_missing_repository() {
        let tmp = unique_temp_dir("nevi_git_changes_no_repo");
        std::fs::create_dir_all(&tmp).expect("create temp dir");

        let mut editor = Editor::default();
        editor.set_project_root(tmp.clone());
        editor.init_git();
        editor.open_finder_git_changes();

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Not in a Git repository")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn git_changes_picker_reports_clean_repository() {
        let tmp = unique_temp_dir("nevi_git_changes_clean");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let root = tmp.canonicalize().expect("canonical temp dir");
        let path = root.join("tracked.rs");
        std::fs::write(&path, "clean\n").expect("write tracked");
        let repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&repo, Path::new("tracked.rs"), "initial");

        let mut editor = Editor::default();
        editor.set_project_root(root.clone());
        editor.init_git();
        editor.open_finder_git_changes();

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.status_message.as_deref(), Some("No Git changes"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn git_changes_picker_lists_modified_files_and_loads_diff_preview() {
        let tmp = unique_temp_dir("nevi_git_changes_modified");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let root = tmp.canonicalize().expect("canonical temp dir");
        let path = root.join("tracked.rs");
        std::fs::write(&path, "old\n").expect("write original");
        let repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&repo, Path::new("tracked.rs"), "initial");
        std::fs::write(&path, "new\n").expect("write modified");

        let mut editor = Editor::default();
        editor.set_project_root(root.clone());
        editor.init_git();
        editor.open_finder_git_changes();

        assert_eq!(editor.mode, Mode::Finder);
        assert_eq!(editor.finder.mode, crate::finder::FinderMode::GitChanges);
        assert!(editor.finder.preview_enabled);
        assert_eq!(editor.finder.items.len(), 1);
        assert!(editor.finder.items[0].display.contains("M tracked.rs"));

        editor.update_finder_preview();
        let preview = editor.finder.preview_content.join("\n");
        assert!(preview.contains("diff --git"));
        assert!(preview.contains("-old"));
        assert!(preview.contains("+new"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn git_changes_picker_clears_diff_preview_when_filter_has_no_selection() {
        let tmp = unique_temp_dir("nevi_git_changes_filter_empty");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let root = tmp.canonicalize().expect("canonical temp dir");
        let path = root.join("tracked.rs");
        std::fs::write(&path, "old\n").expect("write original");
        let repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&repo, Path::new("tracked.rs"), "initial");
        std::fs::write(&path, "new\n").expect("write modified");

        let mut editor = Editor::default();
        editor.set_project_root(root.clone());
        editor.init_git();
        editor.open_finder_git_changes();

        assert!(!editor.finder.preview_content.is_empty());
        assert_eq!(editor.finder.preview_path.as_deref(), Some(path.as_path()));

        for ch in "ZZZZZZZZ".chars() {
            editor.finder.insert_char(ch);
        }
        assert!(editor.finder.selected_item().is_none());

        editor.update_finder_preview();

        assert!(editor.finder.preview_content.is_empty());
        assert_eq!(editor.finder.preview_path, None);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn git_changes_picker_resets_stale_preview_syntax_for_diff_preview() {
        let tmp = unique_temp_dir("nevi_git_changes_preview_syntax");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let root = tmp.canonicalize().expect("canonical temp dir");
        let path = root.join("tracked.rs");
        std::fs::write(&path, "old\n").expect("write original");
        let repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&repo, Path::new("tracked.rs"), "initial");
        std::fs::write(&path, "new\n").expect("write modified");

        let mut editor = Editor::default();
        editor.preview_syntax.set_language_from_path(&path);
        editor.preview_syntax.parse_string("fn main() {}\n");
        assert!(editor.preview_syntax.has_highlighting());

        editor.set_project_root(root.clone());
        editor.init_git();
        editor.open_finder_git_changes();

        assert!(!editor.preview_syntax.has_highlighting());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn focus_gained_refreshes_visible_explorer_tree() {
        let tmp = unique_temp_dir("nevi_explorer_focus_refresh");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let root = tmp.canonicalize().expect("canonical temp dir");
        std::fs::create_dir(root.join("existing")).expect("create existing dir");

        let mut editor = Editor::default();
        editor.set_project_root(root.clone());
        editor.open_explorer();
        assert!(editor
            .explorer
            .flat_view
            .iter()
            .any(|node| node.name == "existing"));
        assert!(!editor
            .explorer
            .flat_view
            .iter()
            .any(|node| node.name == "created-outside"));

        std::fs::create_dir(root.join("created-outside")).expect("create external dir");

        let _ = editor.handle_focus_gained();

        assert!(editor
            .explorer
            .flat_view
            .iter()
            .any(|node| node.name == "created-outside"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn external_process_return_removes_deleted_files_from_visible_explorer_tree() {
        let tmp = unique_temp_dir("nevi_explorer_external_refresh");
        let examples = tmp.join("examples");
        std::fs::create_dir_all(&examples).expect("create examples dir");
        let removed_file = examples.join("dump_toml_nodes.rs");
        std::fs::write(&removed_file, "fn main() {}\n").expect("write file");
        let root = tmp.clone();

        let mut editor = Editor::default();
        editor.set_project_root(root.clone());
        editor.open_explorer();
        editor.explorer.selected = editor
            .explorer
            .flat_view
            .iter()
            .position(|node| node.path == root.join("examples"))
            .expect("examples dir should be visible");
        editor.explorer.expand();
        assert!(editor
            .explorer
            .flat_view
            .iter()
            .any(|node| node.path == removed_file));

        std::fs::remove_file(&removed_file).expect("remove external file");

        let _ = editor.handle_external_process_finished();

        assert!(!editor
            .explorer
            .flat_view
            .iter()
            .any(|node| node.path == removed_file));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn lsp_text_edits_treat_columns_as_utf16_offsets() {
        let mut editor = Editor::default();
        editor.buffer_mut().insert_str(0, 0, "a😀b\n");

        // LSP columns are UTF-16 code units: a=1, 😀=2, b starts at 3.
        editor.apply_text_edits(&[TextEdit {
            start_line: 0,
            start_col: 3,
            end_line: 0,
            end_col: 4,
            new_text: "X".to_string(),
        }]);

        assert_eq!(editor.buffer().content(), "a😀X\n");
    }

    #[test]
    fn lsp_same_position_insert_edits_preserve_server_order() {
        let mut editor = Editor::default();
        editor.buffer_mut().insert_str(0, 0, "ab\n");

        editor.apply_text_edits(&[
            TextEdit {
                start_line: 0,
                start_col: 1,
                end_line: 0,
                end_col: 1,
                new_text: "X".to_string(),
            },
            TextEdit {
                start_line: 0,
                start_col: 1,
                end_line: 0,
                end_col: 1,
                new_text: "Y".to_string(),
            },
        ]);

        assert_eq!(editor.buffer().content(), "aXYb\n");
    }

    #[test]
    fn lsp_adjacent_insert_and_delete_use_original_positions() {
        let mut editor = Editor::default();
        editor.buffer_mut().insert_str(0, 0, "ab\n");

        editor.apply_text_edits(&[
            TextEdit {
                start_line: 0,
                start_col: 1,
                end_line: 0,
                end_col: 1,
                new_text: "X".to_string(),
            },
            TextEdit {
                start_line: 0,
                start_col: 0,
                end_line: 0,
                end_col: 1,
                new_text: String::new(),
            },
        ]);

        assert_eq!(editor.buffer().content(), "Xb\n");
    }

    fn completion_item(label: &str, sort_text: &str) -> CompletionItem {
        CompletionItem {
            item_id: 0,
            label: label.to_string(),
            kind: CompletionKind::Function,
            detail: None,
            documentation: None,
            insert_text: None,
            filter_text: None,
            sort_text: Some(sort_text.to_string()),
            text_edit: None,
            additional_text_edits: Vec::new(),
            raw_data: None,
        }
    }

    #[test]
    fn completion_filter_preserves_lsp_sort_order_for_prefix_matches() {
        let mut completion = super::CompletionState::default();
        completion.show(
            vec![
                completion_item("useElementSize", "02"),
                completion_item("useEffect", "01"),
                completion_item("useEmotionCache", "03"),
                completion_item("useLayoutEffect", "04"),
            ],
            0,
            0,
            false,
        );

        completion.update_filter("useE");

        assert_eq!(
            completion.selected_item().map(|item| item.label.as_str()),
            Some("useEffect")
        );
    }

    #[test]
    fn completion_edits_apply_auto_import_and_main_text_edit() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("const App = () => {\n  useE\n};\n");
        editor.mode = Mode::Insert;
        editor.cursor.line = 1;
        editor.cursor.col = 6;

        let item = CompletionItem {
            item_id: 1,
            label: "useEffect".to_string(),
            kind: CompletionKind::Function,
            detail: None,
            documentation: None,
            insert_text: None,
            filter_text: None,
            sort_text: Some("01".to_string()),
            text_edit: Some(TextEdit {
                start_line: 1,
                start_col: 2,
                end_line: 1,
                end_col: 6,
                new_text: "useEffect".to_string(),
            }),
            additional_text_edits: vec![TextEdit {
                start_line: 0,
                start_col: 0,
                end_line: 0,
                end_col: 0,
                new_text: "import { useEffect } from 'react';\n".to_string(),
            }],
            raw_data: None,
        };

        let inserted = editor
            .apply_completion_item_edits(&item)
            .expect("completion edit applied");

        assert_eq!(inserted, "useEffect");
        assert_eq!(
            editor.buffer().content(),
            "import { useEffect } from 'react';\nconst App = () => {\n  useEffect\n};\n"
        );
        assert_eq!((editor.cursor.line, editor.cursor.col), (2, 11));
    }

    #[test]
    fn code_action_does_not_apply_other_file_edits_to_current_buffer() {
        let tmp = unique_temp_dir("nevi_code_action_uri");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let current = tmp.join("current.rs");
        let other = tmp.join("other.rs");
        std::fs::write(&current, "abc\n").expect("write current");
        std::fs::write(&other, "xyz\n").expect("write other");

        let mut editor = Editor::default();
        editor.open_file(current).expect("open current");
        let other_uri = crate::lsp::path_to_uri(&other);

        editor.show_code_actions_picker(vec![CodeActionItem {
            title: "Edit other file".to_string(),
            kind: None,
            is_preferred: false,
            edits: vec![(
                other_uri,
                vec![TextEdit {
                    start_line: 0,
                    start_col: 0,
                    end_line: 0,
                    end_col: 1,
                    new_text: "Q".to_string(),
                }],
            )],
            command: None,
        }]);

        let _ = editor.apply_selected_code_action();

        assert_eq!(editor.buffer().content(), "abc\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn diagnostics_treat_columns_as_utf16_offsets() {
        let tmp = unique_temp_dir("nevi_diagnostic_utf16");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("unicode.rs");
        std::fs::write(&path, "a😀b\n").expect("write file");

        let mut editor = Editor::default();
        editor.open_file(path.clone()).expect("open file");
        let uri = crate::lsp::path_to_uri(&path);

        editor.set_diagnostics(
            uri,
            vec![Diagnostic {
                line: 0,
                end_line: 0,
                col_start: 3,
                col_end: 4,
                severity: DiagnosticSeverity::Error,
                message: "problem".to_string(),
                source: None,
                code: None,
            }],
        );
        editor.cursor.line = 0;
        editor.cursor.col = 2;

        assert_eq!(editor.current_diagnostics()[0].col_start, 2);
        assert_eq!(editor.current_diagnostics()[0].col_end, 3);
        assert_eq!(editor.all_diagnostics_at_cursor().len(), 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
