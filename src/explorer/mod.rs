use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::fs;

/// Pending action in the file explorer
#[derive(Debug, Clone, PartialEq)]
pub enum ExplorerAction {
    /// Adding a new file or folder
    Add,
    /// Renaming an item
    Rename,
    /// Deleting an item (waiting for confirmation)
    Delete,
}

/// Clipboard operation type
#[derive(Debug, Clone, PartialEq)]
pub enum ClipboardOp {
    Copy,
    Cut,
}

/// Clipboard content for copy/cut/paste operations
#[derive(Debug, Clone)]
pub struct Clipboard {
    /// Path that was copied/cut
    pub path: PathBuf,
    /// Whether this is a copy or cut operation
    pub op: ClipboardOp,
}

/// A node in the file tree
#[derive(Debug, Clone)]
pub struct TreeNode {
    /// File/directory name (not full path)
    pub name: String,
    /// Full path
    pub path: PathBuf,
    /// Whether this is a directory
    pub is_dir: bool,
    /// Child nodes (only for directories)
    pub children: Vec<TreeNode>,
    /// Depth in tree (for indentation)
    pub depth: usize,
}

impl TreeNode {
    /// Create a new tree node from a path
    pub fn new(path: PathBuf, depth: usize) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let is_dir = path.is_dir();

        Self {
            name,
            path,
            is_dir,
            children: Vec::new(),
            depth,
        }
    }

    /// Load children for a directory node
    pub fn load_children(&mut self) {
        if !self.is_dir {
            return;
        }

        self.children.clear();

        if let Ok(entries) = fs::read_dir(&self.path) {
            let mut dirs: Vec<TreeNode> = Vec::new();
            let mut files: Vec<TreeNode> = Vec::new();

            for entry in entries.flatten() {
                let path = entry.path();
                let node = TreeNode::new(path, self.depth + 1);
                if node.is_dir {
                    dirs.push(node);
                } else {
                    files.push(node);
                }
            }

            // Sort directories first, then files, both alphabetically
            dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

            self.children.extend(dirs);
            self.children.extend(files);
        }
    }
}

/// Represents a flattened view of the tree for rendering
#[derive(Debug, Clone)]
pub struct FlatNode {
    /// The tree node reference index
    pub path: PathBuf,
    /// Display name
    pub name: String,
    /// Is directory
    pub is_dir: bool,
    /// Depth for indentation
    pub depth: usize,
    /// Is expanded (for directories)
    pub is_expanded: bool,
}

/// File explorer sidebar state
#[derive(Debug)]
pub struct FileExplorer {
    /// Root directory
    pub root: Option<PathBuf>,
    /// Root tree node
    pub tree: Option<TreeNode>,
    /// Set of expanded directory paths
    pub expanded: HashSet<PathBuf>,
    /// Currently selected index in the flattened view
    pub selected: usize,
    /// Flattened view for rendering
    pub flat_view: Vec<FlatNode>,
    /// Whether the explorer is visible
    pub visible: bool,
    /// Width of the sidebar
    pub width: u16,
    /// Pending action (add, rename, delete)
    pub pending_action: Option<ExplorerAction>,
    /// Input buffer for pending action
    pub input_buffer: String,
    /// Cursor position in input buffer
    pub input_cursor: usize,
    /// Clipboard for copy/cut operations
    pub clipboard: Option<Clipboard>,
    /// Whether search mode is active
    pub is_searching: bool,
    /// Search query buffer
    pub search_buffer: String,
    /// Cursor position in search buffer
    pub search_cursor: usize,
    /// Filtered indices (indices into flat_view that match search)
    pub search_matches: Vec<usize>,
    /// Current match index in search_matches
    pub current_match: usize,
}

impl Default for FileExplorer {
    fn default() -> Self {
        Self {
            root: None,
            tree: None,
            expanded: HashSet::new(),
            selected: 0,
            flat_view: Vec::new(),
            visible: false,
            width: 35,
            pending_action: None,
            input_buffer: String::new(),
            input_cursor: 0,
            clipboard: None,
            is_searching: false,
            search_buffer: String::new(),
            search_cursor: 0,
            search_matches: Vec::new(),
            current_match: 0,
        }
    }
}

impl FileExplorer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the root directory and build the tree
    pub fn set_root(&mut self, path: PathBuf) {
        self.root = Some(path.clone());
        let mut root_node = TreeNode::new(path.clone(), 0);
        root_node.load_children();
        self.tree = Some(root_node);
        self.expanded.insert(path);
        self.rebuild_flat_view();
    }

    /// Toggle visibility
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible {
            self.rebuild_flat_view();
        }
    }

    /// Show the explorer
    pub fn show(&mut self) {
        self.visible = true;
        self.rebuild_flat_view();
    }

    /// Hide the explorer
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Move selection up
    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Move selection down
    pub fn move_down(&mut self) {
        if !self.flat_view.is_empty() && self.selected < self.flat_view.len() - 1 {
            self.selected += 1;
        }
    }

    /// Get the currently selected path
    pub fn selected_path(&self) -> Option<&PathBuf> {
        self.flat_view.get(self.selected).map(|n| &n.path)
    }

    /// Toggle expand/collapse for selected directory
    pub fn toggle_expand(&mut self) {
        if let Some(node) = self.flat_view.get(self.selected) {
            if node.is_dir {
                let path = node.path.clone();
                if self.expanded.contains(&path) {
                    self.expanded.remove(&path);
                } else {
                    self.expanded.insert(path.clone());
                    // Load children if needed
                    self.load_children_for(&path);
                }
                self.rebuild_flat_view();
            }
        }
    }

    /// Expand selected directory (if collapsed)
    pub fn expand(&mut self) {
        if let Some(node) = self.flat_view.get(self.selected) {
            if node.is_dir && !self.expanded.contains(&node.path) {
                let path = node.path.clone();
                self.expanded.insert(path.clone());
                self.load_children_for(&path);
                self.rebuild_flat_view();
            }
        }
    }

    /// Collapse selected directory (if expanded) or go to parent
    pub fn collapse(&mut self) {
        if let Some(node) = self.flat_view.get(self.selected) {
            if node.is_dir && self.expanded.contains(&node.path) {
                // Collapse this directory
                self.expanded.remove(&node.path);
                self.rebuild_flat_view();
            } else {
                // Go to parent directory
                self.go_to_parent();
            }
        }
    }

    /// Go to parent directory in the tree
    pub fn go_to_parent(&mut self) {
        if let Some(node) = self.flat_view.get(self.selected) {
            if let Some(parent) = node.path.parent() {
                // Find the parent in the flat view
                for (i, n) in self.flat_view.iter().enumerate() {
                    if n.path == parent {
                        self.selected = i;
                        break;
                    }
                }
            }
        }
    }

    /// Collapse all directories
    pub fn collapse_all(&mut self) {
        // Keep only the root expanded
        if let Some(root) = &self.root {
            self.expanded.clear();
            self.expanded.insert(root.clone());
        }
        self.rebuild_flat_view();
    }

    /// Refresh the tree (reload from filesystem)
    pub fn refresh(&mut self) {
        if let Some(root) = self.root.clone() {
            let mut root_node = TreeNode::new(root.clone(), 0);
            root_node.load_children();
            self.tree = Some(root_node);

            // Reload children for expanded directories
            let expanded: Vec<PathBuf> = self.expanded.iter().cloned().collect();
            for path in expanded {
                self.load_children_for(&path);
            }

            self.rebuild_flat_view();

            // Ensure selected is in bounds
            if self.selected >= self.flat_view.len() {
                self.selected = self.flat_view.len().saturating_sub(1);
            }
        }
    }

    /// Load children for a directory path in the tree
    fn load_children_for(&mut self, path: &Path) {
        if let Some(tree) = &mut self.tree {
            Self::load_children_recursive(tree, path);
        }
    }

    fn load_children_recursive(node: &mut TreeNode, target: &Path) {
        if node.path == target {
            node.load_children();
            return;
        }

        for child in &mut node.children {
            if target.starts_with(&child.path) {
                Self::load_children_recursive(child, target);
            }
        }
    }

    /// Rebuild the flattened view from the tree
    fn rebuild_flat_view(&mut self) {
        self.flat_view.clear();

        if let Some(tree) = &self.tree {
            Self::flatten_tree_into(&mut self.flat_view, tree, &self.expanded);
        }

        // Ensure selected is in bounds
        if self.selected >= self.flat_view.len() {
            self.selected = self.flat_view.len().saturating_sub(1);
        }
    }

    fn flatten_tree_into(flat_view: &mut Vec<FlatNode>, node: &TreeNode, expanded: &HashSet<PathBuf>) {
        let is_expanded = expanded.contains(&node.path);

        flat_view.push(FlatNode {
            path: node.path.clone(),
            name: node.name.clone(),
            is_dir: node.is_dir,
            depth: node.depth,
            is_expanded,
        });

        // Only add children if expanded
        if is_expanded {
            for child in &node.children {
                Self::flatten_tree_into(flat_view, child, expanded);
            }
        }
    }

    /// Reveal a file in the tree (expand parents and select)
    pub fn reveal_file(&mut self, path: &Path) {
        // Expand all parent directories
        let mut current = path.parent();
        while let Some(parent) = current {
            if self.root.as_ref().map(|r| parent.starts_with(r)).unwrap_or(false) {
                self.expanded.insert(parent.to_path_buf());
                self.load_children_for(parent);
            }
            current = parent.parent();
        }

        // Rebuild and find the file
        self.rebuild_flat_view();

        // Select the file
        for (i, node) in self.flat_view.iter().enumerate() {
            if node.path == path {
                self.selected = i;
                break;
            }
        }
    }

    /// Get the display icon for a node
    pub fn get_icon(&self, node: &FlatNode) -> &'static str {
        if node.is_dir {
            if node.is_expanded {
                ""  // Folder open icon
            } else {
                ""  // Folder closed icon
            }
        } else {
            // File icon based on extension
            let ext = node.path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            match ext {
                "rs" => "",      // Rust
                "js" | "jsx" => "",  // JavaScript
                "ts" | "tsx" => "",  // TypeScript
                "py" => "",      // Python
                "go" => "",      // Go
                "md" => "",      // Markdown
                "json" => "",    // JSON
                "toml" => "",    // TOML
                "yaml" | "yml" => "",
                "html" => "",
                "css" | "scss" => "",
                "lua" => "",
                "sh" | "bash" | "zsh" => "",
                "git" => "",
                "lock" => "",
                _ => "",          // Default file icon
            }
        }
    }

    // === File operation methods ===

    /// Start adding a new file/folder
    pub fn start_add(&mut self) {
        self.pending_action = Some(ExplorerAction::Add);
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

    /// Start renaming the selected item
    pub fn start_rename(&mut self) {
        // Get the name first to avoid borrow conflict
        let name = self.selected_node().map(|n| n.name.clone());
        if let Some(name) = name {
            self.pending_action = Some(ExplorerAction::Rename);
            self.input_buffer = name;
            self.input_cursor = self.input_buffer.len();
        }
    }

    /// Start delete confirmation for the selected item
    pub fn start_delete(&mut self) {
        self.pending_action = Some(ExplorerAction::Delete);
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

    /// Cancel any pending action
    pub fn cancel_action(&mut self) {
        self.pending_action = None;
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

    /// Check if there's a pending action requiring input
    pub fn has_pending_action(&self) -> bool {
        self.pending_action.is_some()
    }

    /// Get the current selected node
    pub fn selected_node(&self) -> Option<&FlatNode> {
        self.flat_view.get(self.selected)
    }

    /// Get the directory path where new items should be created
    /// If a directory is selected, use it; otherwise use parent of selected file
    pub fn target_directory(&self) -> Option<PathBuf> {
        self.selected_node().map(|node| {
            if node.is_dir {
                node.path.clone()
            } else {
                node.path.parent().map(|p| p.to_path_buf()).unwrap_or_default()
            }
        })
    }

    /// Insert a character at the cursor position
    pub fn input_insert(&mut self, c: char) {
        self.input_buffer.insert(self.input_cursor, c);
        self.input_cursor += 1;
    }

    /// Delete character before cursor (backspace)
    pub fn input_backspace(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
            self.input_buffer.remove(self.input_cursor);
        }
    }

    /// Delete character at cursor (delete)
    pub fn input_delete(&mut self) {
        if self.input_cursor < self.input_buffer.len() {
            self.input_buffer.remove(self.input_cursor);
        }
    }

    /// Move cursor left
    pub fn input_cursor_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn input_cursor_right(&mut self) {
        if self.input_cursor < self.input_buffer.len() {
            self.input_cursor += 1;
        }
    }

    /// Move cursor to start
    pub fn input_cursor_home(&mut self) {
        self.input_cursor = 0;
    }

    /// Move cursor to end
    pub fn input_cursor_end(&mut self) {
        self.input_cursor = self.input_buffer.len();
    }

    /// Get the prompt text for the current action
    pub fn action_prompt(&self) -> &'static str {
        match self.pending_action {
            Some(ExplorerAction::Add) => "Name: ",
            Some(ExplorerAction::Rename) => "Rename: ",
            Some(ExplorerAction::Delete) => "Delete? (y/n): ",
            None => "",
        }
    }

    /// Get the help text for the current action (shown above prompt)
    pub fn action_help(&self) -> &'static str {
        match self.pending_action {
            Some(ExplorerAction::Add) => "(/ for dir)",
            Some(ExplorerAction::Rename) => "",
            Some(ExplorerAction::Delete) => "",
            None => "",
        }
    }

    // === Copy/Cut/Paste methods ===

    /// Copy the selected item to clipboard
    pub fn copy_selected(&mut self) {
        if let Some(node) = self.selected_node() {
            self.clipboard = Some(Clipboard {
                path: node.path.clone(),
                op: ClipboardOp::Copy,
            });
        }
    }

    /// Cut (mark for move) the selected item
    pub fn cut_selected(&mut self) {
        if let Some(node) = self.selected_node() {
            self.clipboard = Some(Clipboard {
                path: node.path.clone(),
                op: ClipboardOp::Cut,
            });
        }
    }

    /// Check if there's something in the clipboard
    pub fn has_clipboard(&self) -> bool {
        self.clipboard.is_some()
    }

    /// Clear the clipboard
    pub fn clear_clipboard(&mut self) {
        self.clipboard = None;
    }

    /// Get clipboard info for status display
    pub fn clipboard_info(&self) -> Option<String> {
        self.clipboard.as_ref().map(|cb| {
            let op = match cb.op {
                ClipboardOp::Copy => "Copy",
                ClipboardOp::Cut => "Cut",
            };
            let name = cb.path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| cb.path.to_string_lossy().to_string());
            format!("{}: {}", op, name)
        })
    }

    // === Search methods ===

    /// Start search mode
    pub fn start_search(&mut self) {
        self.is_searching = true;
        self.search_buffer.clear();
        self.search_cursor = 0;
        self.search_matches.clear();
        self.current_match = 0;
    }

    /// Cancel search mode
    pub fn cancel_search(&mut self) {
        self.is_searching = false;
        self.search_buffer.clear();
        self.search_cursor = 0;
        self.search_matches.clear();
        self.current_match = 0;
    }

    /// Insert character into search buffer
    pub fn search_insert(&mut self, c: char) {
        self.search_buffer.insert(self.search_cursor, c);
        self.search_cursor += 1;
        self.update_search_matches();
    }

    /// Backspace in search buffer
    pub fn search_backspace(&mut self) {
        if self.search_cursor > 0 {
            self.search_cursor -= 1;
            self.search_buffer.remove(self.search_cursor);
            self.update_search_matches();
        }
    }

    /// Move search cursor left
    pub fn search_cursor_left(&mut self) {
        if self.search_cursor > 0 {
            self.search_cursor -= 1;
        }
    }

    /// Move search cursor right
    pub fn search_cursor_right(&mut self) {
        if self.search_cursor < self.search_buffer.len() {
            self.search_cursor += 1;
        }
    }

    /// Update search matches based on current query
    fn update_search_matches(&mut self) {
        self.search_matches.clear();
        self.current_match = 0;

        if self.search_buffer.is_empty() {
            return;
        }

        let query = self.search_buffer.to_lowercase();
        for (idx, node) in self.flat_view.iter().enumerate() {
            if node.name.to_lowercase().contains(&query) {
                self.search_matches.push(idx);
            }
        }

        // Jump to first match
        if !self.search_matches.is_empty() {
            self.selected = self.search_matches[0];
        }
    }

    /// Go to next search match
    pub fn next_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        self.current_match = (self.current_match + 1) % self.search_matches.len();
        self.selected = self.search_matches[self.current_match];
    }

    /// Go to previous search match
    pub fn prev_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        if self.current_match == 0 {
            self.current_match = self.search_matches.len() - 1;
        } else {
            self.current_match -= 1;
        }
        self.selected = self.search_matches[self.current_match];
    }

    /// Confirm search and stay on current selection
    /// Keeps matches so n/N can continue navigating
    pub fn confirm_search(&mut self) {
        self.is_searching = false;
        self.search_buffer.clear();
        self.search_cursor = 0;
        // Keep search_matches and current_match for n/N navigation
    }

    /// Clear search matches (called when selection changes manually)
    pub fn clear_search_matches(&mut self) {
        self.search_matches.clear();
        self.current_match = 0;
    }

    /// Check if there are active search matches for n/N navigation
    pub fn has_search_matches(&self) -> bool {
        !self.search_matches.is_empty()
    }

    /// Check if a node matches the current search
    pub fn is_search_match(&self, idx: usize) -> bool {
        self.search_matches.contains(&idx)
    }

    /// Get search match info for status display
    pub fn search_match_info(&self) -> String {
        if self.search_matches.is_empty() {
            if self.search_buffer.is_empty() {
                String::new()
            } else {
                "No matches".to_string()
            }
        } else {
            format!("{}/{}", self.current_match + 1, self.search_matches.len())
        }
    }
}
