use grep_regex::RegexMatcherBuilder;
use grep_searcher::{sinks::UTF8, SearcherBuilder};
use ignore::WalkBuilder;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::FinderItem;

/// Live grep searcher using ripgrep's grep crate for fast searching
#[derive(Clone)]
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

    /// Set maximum grep results.
    #[cfg(test)]
    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }

    /// Replace ignore patterns.
    pub fn with_ignore_patterns(mut self, patterns: Vec<String>) -> Self {
        self.ignore_patterns = patterns;
        self
    }

    /// Check if a path should be ignored
    fn should_ignore_path(root: &Path, path: &Path, patterns: &[String]) -> bool {
        let rel_path = path.strip_prefix(root).unwrap_or(path);
        if rel_path.as_os_str().is_empty() {
            return false;
        }
        Self::path_matches_patterns(rel_path, patterns)
    }

    fn path_matches_patterns(path: &Path, patterns: &[String]) -> bool {
        for pattern in patterns {
            if pattern == "*" {
                return true;
            }

            if pattern.starts_with('*') && pattern.ends_with('*') {
                let middle = &pattern[1..pattern.len() - 1];
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
                let prefix = &pattern[..pattern.len() - 1];
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

    /// Search for a pattern in all files under root using ripgrep's grep crate
    pub fn search(&self, root: &Path, pattern: &str) -> Vec<FinderItem> {
        let mut results = Vec::new();
        self.search_stream(root, pattern, usize::MAX, |batch| {
            results.extend(batch);
            true
        });
        results
    }

    /// Search for a pattern and emit result batches while walking files.
    /// Returning false from on_batch stops the search early.
    pub fn search_stream<F>(&self, root: &Path, pattern: &str, batch_size: usize, mut on_batch: F)
    where
        F: FnMut(Vec<FinderItem>) -> bool,
    {
        if pattern.is_empty() {
            return;
        }

        // Escape regex special characters for literal search, then make case-insensitive
        let escaped_pattern = regex::escape(pattern);

        // Build a case-insensitive matcher
        let matcher = match RegexMatcherBuilder::new()
            .case_insensitive(true)
            .build(&escaped_pattern)
        {
            Ok(m) => m,
            Err(_) => return,
        };

        // Build searcher with line numbers
        let mut searcher = SearcherBuilder::new().line_number(true).build();

        let batch_size = batch_size.max(1);
        let mut batch = Vec::new();
        let result_count = Arc::new(AtomicUsize::new(0));
        let mut stop_requested = false;

        // Walk directory respecting .gitignore. filter_entry prevents descending
        // into custom-ignored directories before we inspect their files.
        let root_buf = root.to_path_buf();
        let ignore_patterns = self.ignore_patterns.clone();
        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .max_depth(Some(20))
            .filter_entry(move |entry| {
                !Self::should_ignore_path(&root_buf, entry.path(), &ignore_patterns)
            });
        let walker = builder.build();

        for entry in walker.flatten() {
            // Check if we've hit the max results
            if result_count.load(Ordering::Relaxed) >= self.max_results {
                break;
            }

            // Skip directories
            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }

            let path = entry.path();

            // Skip ignored paths (build directories, etc.)
            if Self::should_ignore_path(root, path, &self.ignore_patterns) {
                continue;
            }

            // Skip binary files by extension
            if self.is_binary_extension(path) {
                continue;
            }

            // Use grep-searcher for fast searching
            let rel_path = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            let path_buf = path.to_path_buf();
            let max_results = self.max_results;
            let count_ref = Arc::clone(&result_count);

            let search_result = searcher.search_path(
                &matcher,
                path,
                UTF8(|line_num, line| {
                    // Check limit
                    if count_ref.load(Ordering::Relaxed) >= max_results {
                        return Ok(false); // Stop searching
                    }

                    // Truncate long lines (safely handle UTF-8)
                    let line_trimmed = line.trim();
                    let line_display = if line_trimmed.chars().count() > 100 {
                        let truncated: String = line_trimmed.chars().take(100).collect();
                        format!("{}...", truncated)
                    } else {
                        line_trimmed.to_string()
                    };

                    let display = format!("{}:{}: {}", rel_path, line_num, line_display);

                    let match_col = find_case_insensitive_char_index(line, pattern);
                    let item = FinderItem::new(display, path_buf.clone())
                        .with_line(line_num as usize)
                        .with_col(match_col);

                    batch.push(item);
                    count_ref.fetch_add(1, Ordering::Relaxed);

                    if batch.len() >= batch_size {
                        let items = std::mem::take(&mut batch);
                        if !on_batch(items) {
                            stop_requested = true;
                            return Ok(false);
                        }
                    }

                    Ok(true)
                }),
            );

            // Ignore search errors (binary files, permission denied, etc.), but
            // stop immediately when the receiver asks us to quit.
            if stop_requested {
                break;
            }
            if search_result.is_err() {
                continue;
            }
        }

        if !batch.is_empty() && !stop_requested {
            let _ = on_batch(batch);
        }
    }

    /// Check if file has a binary extension
    fn is_binary_extension(&self, path: &Path) -> bool {
        let binary_extensions = [
            "png", "jpg", "jpeg", "gif", "bmp", "ico", "svg", "pdf", "doc", "docx", "xls", "xlsx",
            "ppt", "pptx", "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "exe", "dll", "so",
            "dylib", "o", "a", "wasm", "class", "pyc", "pyo", "mp3", "mp4", "wav", "avi", "mkv",
            "mov", "ttf", "otf", "woff", "woff2", "eot", "db", "sqlite", "sqlite3",
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

fn find_case_insensitive_char_index(line: &str, pattern: &str) -> usize {
    let pattern_lower = pattern.to_lowercase();

    for (char_idx, (byte_idx, _)) in line.char_indices().enumerate() {
        if line[byte_idx..].to_lowercase().starts_with(&pattern_lower) {
            return char_idx;
        }
    }

    0
}

#[cfg(test)]
mod tests {
    use super::GrepSearcher;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("nevi_{}_{}_{}", name, std::process::id(), nanos))
    }

    #[test]
    fn custom_ignore_patterns_exclude_matches_under_ignored_directories() {
        let root = unique_temp_dir("grep_ignore");
        fs::create_dir_all(root.join("ignored/deep")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("ignored/deep/hidden.rs"), "needle hidden").unwrap();
        fs::write(root.join("src/visible.rs"), "needle visible").unwrap();

        let searcher = GrepSearcher::new().with_ignore_patterns(vec!["ignored".to_string()]);
        let results = searcher.search(&root, "needle");

        assert_eq!(results.len(), 1);
        assert!(results[0].display.contains("src/visible.rs"));
        assert_eq!(results[0].col, Some(0));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn grep_results_store_match_column() {
        let root = unique_temp_dir("grep_column");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "  let value = Needle;\n").unwrap();

        let searcher = GrepSearcher::new().with_max_results(10);
        let results = searcher.search(&root, "needle");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, Some(1));
        assert_eq!(results[0].col, Some(14));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn streaming_search_emits_multiple_batches() {
        let root = unique_temp_dir("grep_stream");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/main.rs"),
            "needle one\nneedle two\nneedle three\nneedle four\nneedle five\n",
        )
        .unwrap();

        let searcher = GrepSearcher::new().with_max_results(10);
        let mut batch_lengths = Vec::new();
        let mut total = 0;
        searcher.search_stream(&root, "needle", 2, |batch| {
            total += batch.len();
            batch_lengths.push(batch.len());
            true
        });

        assert_eq!(total, 5);
        assert_eq!(batch_lengths, vec![2, 2, 1]);

        let _ = fs::remove_dir_all(root);
    }
}
