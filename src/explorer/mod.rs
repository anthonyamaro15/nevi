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
    /// If `use_nerd_fonts` is true, uses Nerd Font icons; otherwise uses Unicode fallback
    pub fn get_icon(&self, node: &FlatNode, use_nerd_fonts: bool) -> &'static str {
        get_file_icon(&node.name, node.is_dir, node.is_expanded, use_nerd_fonts)
    }
}

/// RGB color for icons
#[derive(Debug, Clone, Copy)]
pub struct IconColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl IconColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

// Common icon colors
const COLOR_FOLDER: IconColor = IconColor::new(86, 182, 194);    // Cyan
const COLOR_RUST: IconColor = IconColor::new(222, 165, 132);     // Rust orange
const COLOR_JS: IconColor = IconColor::new(241, 224, 90);        // JavaScript yellow
const COLOR_TS: IconColor = IconColor::new(49, 120, 198);        // TypeScript blue
const COLOR_REACT: IconColor = IconColor::new(97, 218, 251);     // React cyan
const COLOR_PYTHON: IconColor = IconColor::new(55, 118, 171);    // Python blue
const COLOR_GO: IconColor = IconColor::new(0, 173, 216);         // Go cyan
const COLOR_HTML: IconColor = IconColor::new(228, 79, 38);       // HTML orange
const COLOR_CSS: IconColor = IconColor::new(86, 61, 124);        // CSS purple
const COLOR_JSON: IconColor = IconColor::new(241, 224, 90);      // JSON yellow
const COLOR_MARKDOWN: IconColor = IconColor::new(66, 165, 245);  // Markdown blue
const COLOR_GIT: IconColor = IconColor::new(240, 80, 50);        // Git orange-red
const COLOR_CONFIG: IconColor = IconColor::new(140, 140, 140);   // Config gray
const COLOR_LOCK: IconColor = IconColor::new(255, 213, 79);      // Lock yellow
const COLOR_ENV: IconColor = IconColor::new(255, 213, 79);       // Env yellow
const COLOR_DOCKER: IconColor = IconColor::new(33, 150, 243);    // Docker blue
const COLOR_RUBY: IconColor = IconColor::new(204, 52, 45);       // Ruby red
const COLOR_PHP: IconColor = IconColor::new(119, 123, 180);      // PHP purple
const COLOR_JAVA: IconColor = IconColor::new(176, 114, 25);      // Java brown
const COLOR_SWIFT: IconColor = IconColor::new(240, 81, 57);      // Swift orange
const COLOR_C: IconColor = IconColor::new(85, 85, 255);          // C blue
const COLOR_LUA: IconColor = IconColor::new(0, 0, 128);          // Lua dark blue
const COLOR_SHELL: IconColor = IconColor::new(137, 224, 81);     // Shell green
const COLOR_IMAGE: IconColor = IconColor::new(168, 128, 194);    // Image purple
const COLOR_ARCHIVE: IconColor = IconColor::new(175, 180, 43);   // Archive olive
const COLOR_LICENSE: IconColor = IconColor::new(203, 166, 93);   // License gold
const COLOR_DEFAULT: IconColor = IconColor::new(165, 165, 165);  // Default gray

/// Get the color for a file icon based on file type
pub fn get_icon_color(name: &str, is_dir: bool) -> IconColor {
    if is_dir {
        return COLOR_FOLDER;
    }

    // Check exact filename first
    match name {
        "Cargo.toml" | "Cargo.lock" => COLOR_RUST,
        "package.json" | "package-lock.json" => COLOR_JS,
        "tsconfig.json" | "jsconfig.json" => COLOR_TS,
        "webpack.config.js" | "vite.config.js" | "vite.config.ts" => COLOR_CONFIG,
        "Makefile" | "CMakeLists.txt" => COLOR_CONFIG,
        "Dockerfile" | "docker-compose.yml" | "docker-compose.yaml" => COLOR_DOCKER,
        ".gitignore" | ".gitattributes" | ".gitmodules" => COLOR_GIT,
        ".editorconfig" | ".prettierrc" | ".prettierrc.json" | ".prettierrc.js"
        | ".eslintrc" | ".eslintrc.json" | ".eslintrc.js" => COLOR_CONFIG,
        "README.md" | "README" | "readme.md" => COLOR_MARKDOWN,
        "LICENSE" | "LICENSE.md" | "LICENSE.txt" => COLOR_LICENSE,
        "CHANGELOG.md" | "CHANGELOG" => COLOR_MARKDOWN,
        ".env" | ".env.local" | ".env.development" | ".env.production" => COLOR_ENV,
        _ => {
            // Fall back to extension matching
            let ext = std::path::Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            match ext {
                "rs" => COLOR_RUST,
                "js" | "mjs" | "cjs" => COLOR_JS,
                "jsx" | "tsx" => COLOR_REACT,
                "ts" | "mts" | "cts" => COLOR_TS,
                "html" | "htm" => COLOR_HTML,
                "css" => COLOR_CSS,
                "scss" | "sass" | "less" => COLOR_CSS,
                "vue" => IconColor::new(65, 184, 131),  // Vue green
                "svelte" => IconColor::new(255, 62, 0), // Svelte orange
                "json" | "jsonc" => COLOR_JSON,
                "toml" | "yaml" | "yml" | "xml" => COLOR_CONFIG,
                "csv" => IconColor::new(77, 175, 80),   // CSV green
                "md" | "markdown" => COLOR_MARKDOWN,
                "txt" => COLOR_DEFAULT,
                "pdf" => IconColor::new(244, 67, 54),   // PDF red
                "py" | "pyi" => COLOR_PYTHON,
                "go" => COLOR_GO,
                "rb" => COLOR_RUBY,
                "php" => COLOR_PHP,
                "java" => COLOR_JAVA,
                "kt" | "kts" => IconColor::new(169, 123, 255), // Kotlin purple
                "swift" => COLOR_SWIFT,
                "c" | "h" => COLOR_C,
                "cpp" | "cc" | "cxx" | "hpp" => IconColor::new(0, 89, 156), // C++ darker blue
                "cs" => IconColor::new(104, 33, 122),  // C# purple
                "lua" => COLOR_LUA,
                "zig" => IconColor::new(247, 164, 29), // Zig orange
                "sh" | "bash" | "zsh" | "fish" => COLOR_SHELL,
                "ps1" | "psm1" => IconColor::new(1, 36, 86), // PowerShell dark blue
                "lock" => COLOR_LOCK,
                "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "webp" | "svg" => COLOR_IMAGE,
                "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" => COLOR_ARCHIVE,
                "exe" | "dll" | "so" | "dylib" => COLOR_CONFIG,
                "wasm" => IconColor::new(101, 79, 240), // WASM purple
                "ttf" | "otf" | "woff" | "woff2" => IconColor::new(245, 83, 83), // Font red
                "git" => COLOR_GIT,
                _ => COLOR_DEFAULT,
            }
        }
    }
}

/// Get the appropriate icon for a file or directory
/// Checks exact filename first, then extension, with Nerd Font and Unicode variants
pub fn get_file_icon(name: &str, is_dir: bool, is_expanded: bool, use_nerd_fonts: bool) -> &'static str {
    if is_dir {
        if use_nerd_fonts {
            if is_expanded {
                "\u{f07c}"  // nf-fa-folder_open
            } else {
                "\u{f07b}"  // nf-fa-folder
            }
        } else {
            // Unicode fallback
            if is_expanded {
                "\u{1F4C2}"  // 📂 Open folder
            } else {
                "\u{1F4C1}"  // 📁 Closed folder
            }
        }
    } else {
        // Check exact filename first
        let icon = match name {
            // Config files
            "Cargo.toml" | "Cargo.lock" => if use_nerd_fonts { "\u{e7a8}" } else { "\u{1F4E6}" },  // Rust icon / 📦
            "package.json" | "package-lock.json" => if use_nerd_fonts { "\u{e74e}" } else { "\u{1F4E6}" },  // JS / 📦
            "tsconfig.json" | "jsconfig.json" => if use_nerd_fonts { "\u{e628}" } else { "\u{2699}" },  // TS / ⚙
            "webpack.config.js" | "vite.config.js" | "vite.config.ts" => if use_nerd_fonts { "\u{f0ad}" } else { "\u{2699}" },  // wrench / ⚙
            "Makefile" | "CMakeLists.txt" => if use_nerd_fonts { "\u{f0ad}" } else { "\u{2699}" },  // wrench / ⚙
            "Dockerfile" | "docker-compose.yml" | "docker-compose.yaml" => if use_nerd_fonts { "\u{f308}" } else { "\u{1F433}" },  // docker / 🐳

            // Git files
            ".gitignore" | ".gitattributes" | ".gitmodules" => if use_nerd_fonts { "\u{f1d3}" } else { "\u{E0A0}" },  // git / branch

            // Editor/IDE config
            ".editorconfig" => if use_nerd_fonts { "\u{e615}" } else { "\u{2699}" },  // config / ⚙
            ".prettierrc" | ".prettierrc.json" | ".prettierrc.js" => if use_nerd_fonts { "\u{e615}" } else { "\u{2699}" },
            ".eslintrc" | ".eslintrc.json" | ".eslintrc.js" => if use_nerd_fonts { "\u{e615}" } else { "\u{2699}" },

            // Readme/docs
            "README.md" | "README" | "readme.md" => if use_nerd_fonts { "\u{f48a}" } else { "\u{1F4D6}" },  // book / 📖
            "LICENSE" | "LICENSE.md" | "LICENSE.txt" => if use_nerd_fonts { "\u{f0219}" } else { "\u{1F4DC}" },  // certificate / 📜
            "CHANGELOG.md" | "CHANGELOG" => if use_nerd_fonts { "\u{f543}" } else { "\u{1F4CB}" },  // list / 📋

            // Environment
            ".env" | ".env.local" | ".env.development" | ".env.production" => if use_nerd_fonts { "\u{f023}" } else { "\u{1F510}" },  // lock / 🔐

            _ => {
                // Fall back to extension matching
                let ext = std::path::Path::new(name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");

                match ext {
                    // Rust
                    "rs" => if use_nerd_fonts { "\u{e7a8}" } else { "\u{1F980}" },  // rust / 🦀

                    // JavaScript/TypeScript
                    "js" | "mjs" | "cjs" => if use_nerd_fonts { "\u{e74e}" } else { "\u{1F4DC}" },  // js / 📜
                    "jsx" => if use_nerd_fonts { "\u{e7ba}" } else { "\u{269B}" },  // react / ⚛
                    "ts" | "mts" | "cts" => if use_nerd_fonts { "\u{e628}" } else { "\u{1F4DC}" },  // ts / 📜
                    "tsx" => if use_nerd_fonts { "\u{e7ba}" } else { "\u{269B}" },  // react / ⚛

                    // Web
                    "html" | "htm" => if use_nerd_fonts { "\u{e736}" } else { "\u{1F310}" },  // html5 / 🌐
                    "css" => if use_nerd_fonts { "\u{e749}" } else { "\u{1F3A8}" },  // css3 / 🎨
                    "scss" | "sass" | "less" => if use_nerd_fonts { "\u{e603}" } else { "\u{1F3A8}" },  // sass / 🎨
                    "vue" => if use_nerd_fonts { "\u{e6a0}" } else { "\u{1F7E2}" },  // vue / 🟢
                    "svelte" => if use_nerd_fonts { "\u{e697}" } else { "\u{1F525}" },  // svelte / 🔥

                    // Data/Config
                    "json" | "jsonc" => if use_nerd_fonts { "\u{e60b}" } else { "\u{1F4CB}" },  // json / 📋
                    "toml" => if use_nerd_fonts { "\u{e6b2}" } else { "\u{2699}" },  // settings / ⚙
                    "yaml" | "yml" => if use_nerd_fonts { "\u{e6a8}" } else { "\u{2699}" },  // yaml / ⚙
                    "xml" => if use_nerd_fonts { "\u{e619}" } else { "\u{1F4CB}" },  // code / 📋
                    "csv" => if use_nerd_fonts { "\u{f0ce}" } else { "\u{1F4CA}" },  // table / 📊

                    // Documentation
                    "md" | "markdown" => if use_nerd_fonts { "\u{e73e}" } else { "\u{1F4DD}" },  // markdown / 📝
                    "txt" => if use_nerd_fonts { "\u{f15c}" } else { "\u{1F4C4}" },  // file-text / 📄
                    "pdf" => if use_nerd_fonts { "\u{f1c1}" } else { "\u{1F4D5}" },  // file-pdf / 📕

                    // Programming languages
                    "py" | "pyi" => if use_nerd_fonts { "\u{e73c}" } else { "\u{1F40D}" },  // python / 🐍
                    "go" => if use_nerd_fonts { "\u{e626}" } else { "\u{1F535}" },  // go / 🔵
                    "rb" => if use_nerd_fonts { "\u{e791}" } else { "\u{1F48E}" },  // ruby / 💎
                    "php" => if use_nerd_fonts { "\u{e73d}" } else { "\u{1F418}" },  // php / 🐘
                    "java" => if use_nerd_fonts { "\u{e738}" } else { "\u{2615}" },  // java / ☕
                    "kt" | "kts" => if use_nerd_fonts { "\u{e634}" } else { "\u{1F7E3}" },  // kotlin / 🟣
                    "swift" => if use_nerd_fonts { "\u{e755}" } else { "\u{1F34E}" },  // swift / 🍎
                    "c" => if use_nerd_fonts { "\u{e61e}" } else { "\u{00A9}" },  // c / ©
                    "cpp" | "cc" | "cxx" => if use_nerd_fonts { "\u{e61d}" } else { "\u{00A9}" },  // c++ / ©
                    "h" | "hpp" => if use_nerd_fonts { "\u{e61e}" } else { "\u{00A9}" },  // c / ©
                    "cs" => if use_nerd_fonts { "\u{f031b}" } else { "#" },  // c# / #
                    "lua" => if use_nerd_fonts { "\u{e620}" } else { "\u{1F319}" },  // lua / 🌙
                    "zig" => if use_nerd_fonts { "\u{e6a9}" } else { "\u{26A1}" },  // zig / ⚡

                    // Shell/Scripts
                    "sh" | "bash" | "zsh" | "fish" => if use_nerd_fonts { "\u{f489}" } else { "\u{1F5A5}" },  // terminal / 🖥
                    "ps1" | "psm1" => if use_nerd_fonts { "\u{e683}" } else { "\u{1F5A5}" },  // powershell / 🖥

                    // Build/Lock files
                    "lock" => if use_nerd_fonts { "\u{f023}" } else { "\u{1F512}" },  // lock / 🔒

                    // Images
                    "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "webp" => if use_nerd_fonts { "\u{f1c5}" } else { "\u{1F5BC}" },  // file-image / 🖼
                    "svg" => if use_nerd_fonts { "\u{f1c5}" } else { "\u{1F5BC}" },  // file-image / 🖼

                    // Archives
                    "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" => if use_nerd_fonts { "\u{f1c6}" } else { "\u{1F4E6}" },  // file-archive / 📦

                    // Binary/Executable
                    "exe" | "dll" | "so" | "dylib" => if use_nerd_fonts { "\u{f013}" } else { "\u{2699}" },  // gear / ⚙
                    "wasm" => if use_nerd_fonts { "\u{e6a1}" } else { "\u{1F527}" },  // wasm / 🔧

                    // Fonts
                    "ttf" | "otf" | "woff" | "woff2" => if use_nerd_fonts { "\u{f031}" } else { "\u{1F524}" },  // font / 🔤

                    // Git
                    "git" => if use_nerd_fonts { "\u{f1d3}" } else { "\u{E0A0}" },  // git / branch

                    // Default
                    _ => if use_nerd_fonts { "\u{f15b}" } else { "\u{1F4C4}" },  // file / 📄
                }
            }
        };
        icon
    }
}

impl FileExplorer {
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
