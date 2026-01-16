use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::fs;

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
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                // Skip hidden files (starting with .)
                if name.starts_with('.') {
                    continue;
                }

                // Skip common ignored directories
                if path.is_dir() && matches!(name.as_str(), "target" | "node_modules" | ".git" | "__pycache__") {
                    continue;
                }

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
}
