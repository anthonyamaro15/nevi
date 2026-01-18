use std::path::Path;
use std::fs::File;
use std::io::{BufRead, BufReader};
use ignore::WalkBuilder;

use super::FinderItem;

/// Live grep searcher that respects .gitignore
pub struct GrepSearcher {
    /// Maximum number of results
    max_results: usize,
    /// Ignore patterns (same as file picker)
    ignore_patterns: Vec<String>,
}

impl GrepSearcher {
    /// Default ignore patterns (same as file picker)
    fn default_ignore_patterns() -> Vec<String> {
        vec![
            // Version control
            ".git".to_string(),
            ".svn".to_string(),
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
            "*-build".to_string(),
            // Cache directories
            ".cache".to_string(),
            "__pycache__".to_string(),
            // IDE/Editor
            ".idea".to_string(),
            ".vscode".to_string(),
            // Coverage
            "coverage".to_string(),
            ".nyc_output".to_string(),
        ]
    }

    pub fn new() -> Self {
        Self {
            max_results: 1000,
            ignore_patterns: Self::default_ignore_patterns(),
        }
    }

    /// Create from config settings
    pub fn from_settings(settings: &crate::config::FinderSettings) -> Self {
        let mut patterns = Self::default_ignore_patterns();
        for pattern in &settings.ignore_patterns {
            if !patterns.contains(pattern) {
                patterns.push(pattern.clone());
            }
        }
        Self {
            max_results: settings.max_grep_results,
            ignore_patterns: patterns,
        }
    }

    /// Check if a path should be ignored
    fn should_ignore(&self, path: &Path) -> bool {
        for pattern in &self.ignore_patterns {
            if pattern.starts_with('*') && pattern.ends_with('*') {
                let middle = &pattern[1..pattern.len()-1];
                if path.to_string_lossy().contains(middle) {
                    return true;
                }
            } else if pattern.starts_with('*') {
                let suffix = &pattern[1..];
                for component in path.components() {
                    if let std::path::Component::Normal(name) = component {
                        if name.to_string_lossy().ends_with(suffix) {
                            return true;
                        }
                    }
                }
            } else if pattern.ends_with('*') {
                let prefix = &pattern[..pattern.len()-1];
                for component in path.components() {
                    if let std::path::Component::Normal(name) = component {
                        if name.to_string_lossy().starts_with(prefix) {
                            return true;
                        }
                    }
                }
            } else {
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

    /// Search for a pattern in all files under root
    pub fn search(&self, root: &Path, pattern: &str) -> Vec<FinderItem> {
        if pattern.is_empty() {
            return Vec::new();
        }

        const MAX_GREP_FILE_BYTES: u64 = 2_000_000;

        let mut results = Vec::new();
        let pattern_lower = pattern.to_lowercase();

        // Walk directory respecting .gitignore
        let walker = WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .max_depth(Some(20))
            .build();

        for entry in walker.flatten() {
            if results.len() >= self.max_results {
                break;
            }

            // Skip directories
            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }

            let path = entry.path();

            // Skip ignored paths (build directories, etc.)
            if self.should_ignore(path) {
                continue;
            }

            // Skip binary files by extension
            if self.is_binary_extension(path) {
                continue;
            }

            if let Ok(meta) = path.metadata() {
                if meta.len() > MAX_GREP_FILE_BYTES {
                    continue;
                }
            }

            // Search file contents
            if let Ok(file) = File::open(path) {
                let reader = BufReader::new(file);

                for (line_num, line_result) in reader.lines().enumerate() {
                    if results.len() >= self.max_results {
                        break;
                    }

                    if let Ok(line) = line_result {
                        // Case-insensitive search
                        if line.to_lowercase().contains(&pattern_lower) {
                            let rel_path = path
                                .strip_prefix(root)
                                .unwrap_or(path)
                                .to_string_lossy();

                            // Truncate long lines (safely handle UTF-8)
                            let line_display = if line.chars().count() > 100 {
                                let truncated: String = line.chars().take(100).collect();
                                format!("{}...", truncated)
                            } else {
                                line.clone()
                            };

                            let display = format!(
                                "{}:{}: {}",
                                rel_path,
                                line_num + 1,
                                line_display.trim()
                            );

                            let item = FinderItem::new(display, path.to_path_buf())
                                .with_line(line_num + 1);

                            results.push(item);
                        }
                    }
                }
            }
        }

        results
    }

    /// Check if file has a binary extension
    fn is_binary_extension(&self, path: &Path) -> bool {
        let binary_extensions = [
            "png", "jpg", "jpeg", "gif", "bmp", "ico", "svg",
            "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
            "zip", "tar", "gz", "bz2", "xz", "7z", "rar",
            "exe", "dll", "so", "dylib", "o", "a",
            "wasm", "class", "pyc", "pyo",
            "mp3", "mp4", "wav", "avi", "mkv", "mov",
            "ttf", "otf", "woff", "woff2", "eot",
            "db", "sqlite", "sqlite3",
        ];

        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| binary_extensions.contains(&e.to_lowercase().as_str()))
            .unwrap_or(false)
    }
}

impl Default for GrepSearcher {
    fn default() -> Self {
        Self::new()
    }
}
