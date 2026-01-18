use std::path::Path;
use ignore::WalkBuilder;

use super::FinderItem;

/// File picker that respects .gitignore
pub struct FilePicker {
    /// Maximum number of files to scan
    max_files: usize,
    /// Additional ignore patterns
    ignore_patterns: Vec<String>,
}

impl FilePicker {
    /// Default ignore patterns for common build/cache directories
    fn default_ignore_patterns() -> Vec<String> {
        vec![
            // Version control
            ".git".to_string(),
            ".svn".to_string(),
            ".hg".to_string(),
            // Dependencies
            "node_modules".to_string(),
            "vendor".to_string(),
            // Build outputs
            "target".to_string(),
            "build".to_string(),
            "dist".to_string(),
            "out".to_string(),
            ".next".to_string(),
            ".nuxt".to_string(),
            ".output".to_string(),
            // Build output pattern (aws-build, my-build, etc.)
            "*-build".to_string(),
            // Cache directories
            ".cache".to_string(),
            "__pycache__".to_string(),
            ".pytest_cache".to_string(),
            ".mypy_cache".to_string(),
            // IDE/Editor
            ".idea".to_string(),
            ".vscode".to_string(),
            // Logs and temp files
            "*.log".to_string(),
            "*.tmp".to_string(),
            "*.bak".to_string(),
            // Coverage and test outputs
            "coverage".to_string(),
            ".nyc_output".to_string(),
            // Package lock files (optional, can be removed)
            // "package-lock.json".to_string(),
            // "yarn.lock".to_string(),
        ]
    }

    pub fn new() -> Self {
        Self {
            max_files: 10000,
            ignore_patterns: Self::default_ignore_patterns(),
        }
    }

    /// Create from config settings (merges with defaults)
    pub fn from_settings(settings: &crate::config::FinderSettings) -> Self {
        // Start with defaults, then add user patterns
        let mut patterns = Self::default_ignore_patterns();
        for pattern in &settings.ignore_patterns {
            if !patterns.contains(pattern) {
                patterns.push(pattern.clone());
            }
        }
        Self {
            max_files: settings.max_files,
            ignore_patterns: patterns,
        }
    }

    /// Set maximum files to scan
    pub fn with_max_files(mut self, max: usize) -> Self {
        self.max_files = max;
        self
    }

    /// Add ignore patterns
    pub fn with_ignore_patterns(mut self, patterns: Vec<String>) -> Self {
        self.ignore_patterns = patterns;
        self
    }

    /// List files in a directory, respecting .gitignore
    pub fn list_files(&self, root: &Path) -> Vec<FinderItem> {
        let mut files = Vec::new();

        // Use the ignore crate's WalkBuilder which respects .gitignore
        let walker = WalkBuilder::new(root)
            .hidden(false)  // Don't ignore hidden files by default
            .git_ignore(true)  // Respect .gitignore
            .git_global(true)  // Respect global gitignore
            .git_exclude(true) // Respect .git/info/exclude
            .max_depth(Some(20))  // Limit depth
            .build();

        for entry in walker.flatten() {
            // Skip directories and root
            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                let path = entry.path();

                // Check additional ignore patterns
                if self.should_ignore(path) {
                    continue;
                }

                // Create relative path for display
                let display = path
                    .strip_prefix(root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                files.push(FinderItem::new(display, path.to_path_buf()));

                // Limit number of files
                if files.len() >= self.max_files {
                    break;
                }
            }
        }

        // Sort by path for consistent ordering
        files.sort_by(|a, b| a.display.cmp(&b.display));

        files
    }

    /// Check if a path should be ignored based on custom patterns
    fn should_ignore(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        for pattern in &self.ignore_patterns {
            if pattern.starts_with('*') && pattern.ends_with('*') {
                // Contains pattern like *build* (matches anywhere)
                let middle = &pattern[1..pattern.len()-1];
                if path_str.contains(middle) {
                    return true;
                }
            } else if pattern.starts_with('*') {
                // Suffix pattern like *.log or *-build
                let suffix = &pattern[1..];
                // Check if any path component ends with this suffix
                for component in path.components() {
                    if let std::path::Component::Normal(name) = component {
                        if name.to_string_lossy().ends_with(suffix) {
                            return true;
                        }
                    }
                }
            } else if pattern.ends_with('*') {
                // Prefix pattern like build*
                let prefix = &pattern[..pattern.len()-1];
                for component in path.components() {
                    if let std::path::Component::Normal(name) = component {
                        if name.to_string_lossy().starts_with(prefix) {
                            return true;
                        }
                    }
                }
            } else {
                // Exact directory/file name pattern
                for component in path.components() {
                    if let std::path::Component::Normal(name) = component {
                        if name.to_string_lossy() == *pattern {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }
}

impl Default for FilePicker {
    fn default() -> Self {
        Self::new()
    }
}
