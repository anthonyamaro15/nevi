use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks::Lossy};
use ignore::{WalkBuilder, WalkState};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::FinderItem;

/// How long a found result can wait before it is handed over. Without a time
/// limit, a search with fewer hits than a batch showed nothing until the whole
/// repo was scanned. Matches the main loop's background redraw interval, so
/// handing over more often would not reach the screen any sooner.
const FLUSH_INTERVAL: Duration = Duration::from_millis(50);

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
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_stream(root, pattern, usize::MAX, &cancel, |batch| {
            results.extend(batch);
            true
        });
        results
    }

    /// Search for a pattern and emit result batches while walking files.
    /// Files are walked and searched by parallel workers (ripgrep style);
    /// results stream unordered across files. Setting `cancel`, or returning
    /// false from on_batch, stops the search early. The second also sets
    /// `cancel`, so give every search a fresh flag.
    pub fn search_stream<F>(
        &self,
        root: &Path,
        pattern: &str,
        batch_size: usize,
        cancel: &Arc<AtomicBool>,
        on_batch: F,
    ) where
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

        // Walk directory respecting .gitignore. filter_entry prevents
        // descending into custom-ignored directories, so their files never
        // reach the workers.
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
        let walker = builder.build_parallel();

        let (tx, rx) = mpsc::channel::<FinderItem>();
        let result_count = Arc::new(AtomicUsize::new(0));
        let max_results = self.max_results;
        let walk_root = root.to_path_buf();
        let walk_pattern = pattern.to_string();

        // run() blocks until the walk finishes, so it gets its own thread
        // while this one batches results in arrival order.
        let walk_count = Arc::clone(&result_count);
        let walk_stop = Arc::clone(cancel);
        let walk_handle = std::thread::spawn(move || {
            walker.run(|| {
                // One searcher per worker thread; the matcher is shared.
                let mut searcher = SearcherBuilder::new()
                    .line_number(true)
                    // Like ripgrep: stop reading a file at its first NUL byte,
                    // so binaries without a known extension end early instead
                    // of producing garbage rows.
                    .binary_detection(BinaryDetection::quit(b'\x00'))
                    .build();
                let matcher = matcher.clone();
                let tx = tx.clone();
                let count = Arc::clone(&walk_count);
                let stop = Arc::clone(&walk_stop);
                let root = walk_root.clone();
                let pattern = walk_pattern.clone();
                Box::new(move |entry| {
                    if stop.load(Ordering::Relaxed) || count.load(Ordering::Relaxed) >= max_results
                    {
                        return WalkState::Quit;
                    }
                    let Ok(entry) = entry else {
                        return WalkState::Continue;
                    };
                    if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                        return WalkState::Continue;
                    }
                    let path = entry.path();
                    if Self::is_binary_extension(path) {
                        return WalkState::Continue;
                    }

                    let rel_path = path
                        .strip_prefix(&root)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .to_string();
                    let path_buf = path.to_path_buf();

                    let Ok(file) = File::open(path) else {
                        return WalkState::Continue;
                    };
                    let reader = StopReader {
                        inner: file,
                        stop: &stop,
                    };
                    // Ignore per-file errors (permissions, a cancelled read).
                    // Lossy keeps going past a matching line that isn't valid
                    // UTF-8 (it shows U+FFFD) instead of ending the file there.
                    let _ = searcher.search_reader(
                        &matcher,
                        reader,
                        Lossy(|line_num, line| {
                            if stop.load(Ordering::Relaxed)
                                || count.fetch_add(1, Ordering::Relaxed) >= max_results
                            {
                                return Ok(false);
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
                            let match_col = find_case_insensitive_char_index(line, &pattern);
                            let item = FinderItem::new(display, path_buf.clone())
                                .with_line(line_num as usize)
                                .with_col(match_col);

                            Ok(tx.send(item).is_ok())
                        }),
                    );

                    WalkState::Continue
                })
            });
            // run() returning drops every worker's tx clone, ending the
            // batching loop below.
        });

        if !forward_batches(rx, batch_size, max_results, on_batch) {
            cancel.store(true, Ordering::Relaxed);
        }
        // forward_batches dropped the receiver, so pending worker sends fail
        // too; wait for the walk to wind down.
        let _ = walk_handle.join();
    }

    /// Check if file has a binary extension
    fn is_binary_extension(path: &Path) -> bool {
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

/// Reads a file but fails once the search is cancelled, so a search stops
/// partway through one huge file instead of reading to its end. grep-searcher
/// reads 64 KB at a time, so that is how often this checks.
struct StopReader<'a, R> {
    inner: R,
    stop: &'a AtomicBool,
}

impl<R: Read> Read for StopReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            // Not ErrorKind::Interrupted: encoding_rs_io retries those.
            return Err(io::Error::other("search cancelled"));
        }
        self.inner.read(buf)
    }
}

/// Hand results to `on_batch` in batches of `batch_size`, or whatever has
/// arrived once FLUSH_INTERVAL has passed since the last hand-over, so rare
/// hits show up while the scan goes on. The first result goes out right away.
/// Returns false when `on_batch` asked to stop.
fn forward_batches<F>(
    rx: Receiver<FinderItem>,
    batch_size: usize,
    max_results: usize,
    mut on_batch: F,
) -> bool
where
    F: FnMut(Vec<FinderItem>) -> bool,
{
    let batch_size = batch_size.max(1);
    let mut batch = Vec::new();
    let mut emitted = 0usize;
    let mut next_flush = Instant::now();
    loop {
        let received = if batch.is_empty() {
            // Nothing is waiting to go out, so there is no deadline.
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            rx.recv_timeout(next_flush.saturating_duration_since(Instant::now()))
        };
        match received {
            Ok(item) if emitted < max_results => {
                batch.push(item);
                emitted += 1;
            }
            // Past the cap, keep draining so the workers can quit.
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if !batch.is_empty() && (batch.len() >= batch_size || Instant::now() >= next_flush) {
            if !on_batch(std::mem::take(&mut batch)) {
                return false;
            }
            next_flush = Instant::now() + FLUSH_INTERVAL;
        }
    }
    batch.is_empty() || on_batch(batch)
}

/// Non-overlapping case-insensitive matches of `pattern` in `text`, as
/// `(start, end)` char ranges.
pub(crate) fn case_insensitive_match_ranges(text: &str, pattern: &str) -> Vec<(usize, usize)> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let pattern_lower = pattern.to_lowercase();
    let pattern_chars = pattern.chars().count();
    let mut ranges = Vec::new();

    // Lowercase the text once and substring-search it. Falls back to the
    // per-position scan when lowercasing changes the char count (rare
    // expansions like 'İ'), where lowered positions no longer map 1:1.
    let text_lower = text.to_lowercase();
    if text_lower.chars().count() == text.chars().count() {
        let (mut seen_bytes, mut seen_chars) = (0, 0);
        for (byte_pos, _) in text_lower.match_indices(&pattern_lower) {
            seen_chars += text_lower[seen_bytes..byte_pos].chars().count();
            seen_bytes = byte_pos;
            ranges.push((seen_chars, seen_chars + pattern_chars));
        }
        return ranges;
    }

    // A char lowercases to at least one char, so a match can only span the
    // next `probe_chars` chars: lowercase just those, not the whole rest of
    // the line, or a long line costs time quadratic in its length.
    let probe_chars = pattern_lower.chars().count();
    let mut next_start = 0;
    for (char_idx, (byte_idx, _)) in text.char_indices().enumerate() {
        if char_idx < next_start {
            continue;
        }
        let rest = &text[byte_idx..];
        let probe_end = rest
            .char_indices()
            .nth(probe_chars)
            .map_or(rest.len(), |(end, _)| end);
        if rest[..probe_end].to_lowercase().starts_with(&pattern_lower) {
            ranges.push((char_idx, char_idx + pattern_chars));
            next_start = char_idx + pattern_chars.max(1);
        }
    }
    ranges
}

fn find_case_insensitive_char_index(line: &str, pattern: &str) -> usize {
    case_insensitive_match_ranges(line, pattern)
        .first()
        .map_or(0, |&(start, _)| start)
}

#[cfg(test)]
mod tests {
    use super::{
        FinderItem, GrepSearcher, StopReader, case_insensitive_match_ranges, forward_batches,
    };
    use grep_regex::RegexMatcherBuilder;
    use grep_searcher::{SearcherBuilder, sinks::Lossy};
    use std::fs;
    use std::io::{self, Read};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("nevi_{}_{}_{}", name, std::process::id(), nanos))
    }

    #[test]
    fn case_insensitive_match_ranges_handle_lowercase_expansion() {
        // 'İ' lowercases to two chars, so this takes the per-position scan.
        assert_eq!(
            case_insensitive_match_ranges("İstanbul istanbul", "ISTANBUL"),
            vec![(9, 17)]
        );
    }

    #[test]
    #[ignore = "perf budget guard; run explicitly with cargo test grep_index_budget -- --ignored --nocapture"]
    fn grep_index_budget_match_column_on_a_long_line_stays_bounded() {
        use std::time::Instant;

        // One 'İ' sends the whole line down the per-position scan. Live grep
        // runs this on every matching line, so it has to stay linear in the
        // line length (a whole-suffix lowercase per position took seconds).
        // The filler is Turkish so lowercasing can't take its ASCII fast path.
        let line = format!("İstanbul {}needle", "ışık ".repeat(20_000));
        let started = Instant::now();
        let ranges = case_insensitive_match_ranges(&line, "NEEDLE");
        let elapsed = started.elapsed();

        assert_eq!(ranges, vec![(100_009, 100_015)]);
        let budget = Duration::from_millis(500);
        println!("match column, 100k-char line with 'İ': {elapsed:?} budget={budget:?}");
        assert!(
            elapsed <= budget,
            "match column took {elapsed:?}, budget {budget:?}"
        );
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
        let cancel = Arc::new(AtomicBool::new(false));
        searcher.search_stream(&root, "needle", 2, &cancel, |batch| {
            total += batch.len();
            batch_lengths.push(batch.len());
            true
        });

        assert_eq!(total, 5);
        // Batches still cap at the requested size; the first hit now goes out
        // on its own instead of waiting for a full batch.
        assert!(
            batch_lengths.iter().all(|&len| len <= 2),
            "{batch_lengths:?}"
        );
        assert!(batch_lengths.len() >= 3, "{batch_lengths:?}");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parallel_search_finds_matches_across_many_files() {
        let root = unique_temp_dir("grep_parallel_files");
        for dir in 0..4 {
            fs::create_dir_all(root.join(format!("mod_{dir}"))).unwrap();
            for file in 0..5 {
                fs::write(
                    root.join(format!("mod_{dir}/file_{file}.rs")),
                    format!("fn f() {{}}\nlet needle_{dir}_{file} = 1;\n"),
                )
                .unwrap();
            }
        }

        let searcher = GrepSearcher::new();
        let results = searcher.search(&root, "needle");

        assert_eq!(results.len(), 20, "one match per file across all workers");
        assert!(results.iter().all(|item| item.line == Some(2)));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parallel_search_respects_max_results_across_files() {
        let root = unique_temp_dir("grep_parallel_cap");
        fs::create_dir_all(&root).unwrap();
        for file in 0..30 {
            fs::write(
                root.join(format!("file_{file}.txt")),
                "needle one\nneedle two\n",
            )
            .unwrap();
        }

        let searcher = GrepSearcher::new().with_max_results(10);
        let results = searcher.search(&root, "needle");

        assert_eq!(results.len(), 10);

        let _ = fs::remove_dir_all(root);
    }

    fn hit(name: &str) -> FinderItem {
        FinderItem::new(format!("{name}:1: needle"), PathBuf::from(name))
    }

    #[test]
    fn hits_are_handed_over_while_the_scan_is_still_running() {
        let (tx, rx) = mpsc::channel();
        let (batches_tx, batches_rx) = mpsc::channel();
        let forwarder = std::thread::spawn(move || {
            forward_batches(rx, 50, 1000, |batch| batches_tx.send(batch.len()).is_ok())
        });

        // The first hit goes out right away, even though tx (the scan) is alive.
        tx.send(hit("a.rs")).unwrap();
        assert_eq!(batches_rx.recv_timeout(Duration::from_secs(5)), Ok(1));

        // A later hit waits at most FLUSH_INTERVAL for company, not for the end.
        tx.send(hit("b.rs")).unwrap();
        assert_eq!(batches_rx.recv_timeout(Duration::from_secs(5)), Ok(1));

        drop(tx);
        assert!(forwarder.join().unwrap());
    }

    #[test]
    fn cancelled_search_stops_partway_through_a_file() {
        // Stands in for a multi-GB file with no matches: it trips the stop flag
        // on its third read and would go on for 64 MB more without the check.
        struct Endless<'a> {
            reads: usize,
            stop: &'a AtomicBool,
        }
        impl Read for Endless<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.reads += 1;
                if self.reads == 3 {
                    self.stop.store(true, Ordering::Relaxed);
                }
                if self.reads > 1000 {
                    return Ok(0);
                }
                for chunk in buf.chunks_mut(4) {
                    chunk.copy_from_slice(&b"abc\n"[..chunk.len()]);
                }
                Ok(buf.len())
            }
        }

        let stop = AtomicBool::new(false);
        let mut endless = Endless {
            reads: 0,
            stop: &stop,
        };
        let matcher = RegexMatcherBuilder::new().build("needle").unwrap();
        let result = SearcherBuilder::new().build().search_reader(
            &matcher,
            StopReader {
                inner: &mut endless,
                stop: &stop,
            },
            Lossy(|_, _| Ok(true)),
        );

        assert!(result.is_err(), "the read after the flag fails the search");
        assert!(endless.reads < 10, "stopped after {} reads", endless.reads);
    }

    #[test]
    fn cancelled_search_reports_nothing() {
        let root = unique_temp_dir("grep_cancelled");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "needle\n").unwrap();

        let cancel = Arc::new(AtomicBool::new(true));
        let mut batches = 0;
        GrepSearcher::new().search_stream(&root, "needle", 50, &cancel, |_| {
            batches += 1;
            true
        });

        assert_eq!(batches, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn files_with_a_nul_byte_are_skipped_like_ripgrep() {
        let root = unique_temp_dir("grep_binary");
        fs::create_dir_all(&root).unwrap();
        // Extension-less, like a vendored framework binary: NUL up front.
        fs::write(root.join("Framework"), b"\x00\x01binary needle\n").unwrap();
        // A stray NUL 1 MB into a text file: hits in earlier 64 KB reads still
        // count, nothing from the read with the NUL onwards does.
        let mut dump = b"needle before\n".to_vec();
        dump.extend(b"filler\n".repeat(150_000));
        dump.extend(b"\x00needle after\n");
        fs::write(root.join("dump.txt"), dump).unwrap();
        fs::write(root.join("notes.txt"), "needle\n").unwrap();

        let mut results = GrepSearcher::new().search(&root, "needle");
        results.sort_by(|a, b| a.display.cmp(&b.display));

        let displays: Vec<&str> = results.iter().map(|item| item.display.as_str()).collect();
        assert_eq!(
            displays,
            ["dump.txt:1: needle before", "notes.txt:1: needle"]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn utf16_files_with_a_bom_are_still_searched() {
        // Older iOS projects keep Localizable.strings in UTF-16, where every
        // ASCII character comes with a NUL byte. grep-searcher decodes BOM
        // files before the NUL check, so the binary skip must not hide them.
        let root = unique_temp_dir("grep_utf16");
        fs::create_dir_all(&root).unwrap();
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "\"key\" = \"needle\";\n".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        fs::write(root.join("Localizable.strings"), bytes).unwrap();

        let results = GrepSearcher::new().search(&root, "needle");
        assert_eq!(results.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_line_that_is_not_utf8_keeps_the_rest_of_the_file() {
        let root = unique_temp_dir("grep_lossy");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("mixed.txt"), b"needle \xff one\nneedle two\n").unwrap();

        let results = GrepSearcher::new().search(&root, "needle");
        assert_eq!(results.len(), 2);
        let _ = fs::remove_dir_all(root);
    }
}
