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
    pub fn new() -> Self {
        Self {
            max_files: 10000,
            ignore_patterns: vec![
                ".git".to_string(),
                "node_modules".to_string(),
                "target".to_string(),
                "*.log".to_string(),
            ],
        }
    }

    /// Create from config settings
    pub fn from_settings(settings: &crate::config::FinderSettings) -> Self {
        Self {
            max_files: settings.max_files,
            ignore_patterns: settings.ignore_patterns.clone(),
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
            if pattern.starts_with('*') {
                // Extension pattern like *.log
                let ext = &pattern[1..];
                if path_str.ends_with(ext) {
                    return true;
                }
            } else {
                // Directory/file name pattern
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
