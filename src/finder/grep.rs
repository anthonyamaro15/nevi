use std::path::Path;
use std::fs::File;
use std::io::{BufRead, BufReader};
use ignore::WalkBuilder;

use super::FinderItem;

/// Live grep searcher that respects .gitignore
pub struct GrepSearcher {
    /// Maximum number of results
    max_results: usize,
}

impl GrepSearcher {
    pub fn new() -> Self {
        Self {
            max_results: 1000,
        }
    }

    /// Create from config settings
    pub fn from_settings(settings: &crate::config::FinderSettings) -> Self {
        Self {
            max_results: settings.max_grep_results,
        }
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

                            // Truncate long lines
                            let line_display = if line.len() > 100 {
                                format!("{}...", &line[..100])
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
