mod highlighter;
mod theme;

pub use highlighter::HighlightSpan;
pub use theme::{HighlightGroup, Theme};

use std::path::Path;
use tree_sitter::{Parser, Query, Tree};
use std::cell::{Cell, RefCell};

use crate::editor::Buffer;

/// Manages syntax highlighting for a buffer
pub struct SyntaxManager {
    /// Tree-sitter parser
    parser: Parser,
    /// Parsed syntax tree
    tree: Option<Tree>,
    /// Highlight query for the current language
    query: Option<Query>,
    /// Current language name
    language: Option<String>,
    /// Color theme
    theme: Theme,
    /// Cached source text (for querying)
    source_cache: String,
    /// Line start byte offsets for source_cache
    line_start_bytes: Vec<usize>,
    /// Cached highlights per line
    highlight_cache: RefCell<Vec<Option<Vec<HighlightSpan>>>>,
    /// Version for which the cache is valid
    cache_version: Cell<u64>,
    /// Version of the buffer last parsed
    parse_version: u64,
}

impl SyntaxManager {
    /// Create a new syntax manager
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            tree: None,
            query: None,
            language: None,
            theme: Theme::default(),
            source_cache: String::new(),
            line_start_bytes: Vec::new(),
            highlight_cache: RefCell::new(Vec::new()),
            cache_version: Cell::new(0),
            parse_version: 0,
        }
    }

    /// Detect language from file path and set up parser
    pub fn set_language_from_path(&mut self, path: &Path) {
        let extension = path.extension().and_then(|e| e.to_str());

        match extension {
            Some("rs") => self.set_rust_language(),
            Some("js") | Some("mjs") | Some("cjs") => self.set_javascript_language(),
            Some("jsx") => self.set_javascript_language(), // JSX uses same parser
            Some("ts") | Some("mts") | Some("cts") => self.set_typescript_language(),
            Some("tsx") => self.set_tsx_language(),
            Some("css") => self.set_css_language(),
            Some("scss") | Some("sass") => self.set_css_language(), // SCSS uses CSS parser
            Some("json") => self.set_json_language(),
            Some("md") | Some("markdown") => self.set_markdown_language(),
            _ => {
                self.language = None;
                self.query = None;
                self.tree = None;
                self.source_cache.clear();
                self.line_start_bytes.clear();
                self.highlight_cache.borrow_mut().clear();
                self.cache_version.set(0);
                self.parse_version = 0;
            }
        }
    }

    /// Detect language from optional file path
    pub fn set_language_from_path_option(&mut self, path: Option<&std::path::PathBuf>) {
        if let Some(p) = path {
            self.set_language_from_path(p);
        } else {
            self.language = None;
            self.query = None;
            self.tree = None;
            self.source_cache.clear();
            self.line_start_bytes.clear();
            self.highlight_cache.borrow_mut().clear();
            self.cache_version.set(0);
            self.parse_version = 0;
        }
    }

    /// Set up Rust language parser
    fn set_rust_language(&mut self) {
        let language = tree_sitter_rust::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("rust".to_string());

                // Create the highlight query
                let query_source = highlighter::rust_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        // Query failed - store error for debugging
                        self.language = Some(format!("rust (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("rust (lang error: {:?})", e));
            }
        }
    }

    /// Set up JavaScript language parser
    fn set_javascript_language(&mut self) {
        let language = tree_sitter_javascript::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("javascript".to_string());

                let query_source = highlighter::javascript_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("javascript (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("javascript (lang error: {:?})", e));
            }
        }
    }

    /// Set up TypeScript language parser
    fn set_typescript_language(&mut self) {
        let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("typescript".to_string());

                let query_source = highlighter::typescript_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("typescript (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("typescript (lang error: {:?})", e));
            }
        }
    }

    /// Set up TSX (TypeScript + JSX) language parser
    fn set_tsx_language(&mut self) {
        let language = tree_sitter_typescript::LANGUAGE_TSX;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("tsx".to_string());

                let query_source = highlighter::tsx_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("tsx (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("tsx (lang error: {:?})", e));
            }
        }
    }

    /// Set up CSS language parser
    fn set_css_language(&mut self) {
        let language = tree_sitter_css::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("css".to_string());

                let query_source = highlighter::css_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("css (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("css (lang error: {:?})", e));
            }
        }
    }

    /// Set up JSON language parser
    fn set_json_language(&mut self) {
        let language = tree_sitter_json::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("json".to_string());

                let query_source = highlighter::json_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("json (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("json (lang error: {:?})", e));
            }
        }
    }

    /// Set up Markdown language parser
    fn set_markdown_language(&mut self) {
        let language = tree_sitter_md::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("markdown".to_string());

                let query_source = highlighter::markdown_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("markdown (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("markdown (lang error: {:?})", e));
            }
        }
    }

    /// Parse the entire buffer
    pub fn parse(&mut self, buffer: &Buffer) {
        if self.language.is_none() {
            return;
        }

        const MAX_HIGHLIGHT_LINES: usize = 200_000;
        const MAX_HIGHLIGHT_CHARS: usize = 2_000_000;

        if buffer.len_lines() > MAX_HIGHLIGHT_LINES || buffer.len_chars() > MAX_HIGHLIGHT_CHARS {
            self.tree = None;
            self.source_cache.clear();
            self.line_start_bytes.clear();
            self.highlight_cache.borrow_mut().clear();
            self.cache_version.set(0);
            self.parse_version = buffer.version();
            return;
        }

        // Convert buffer to string for parsing
        self.source_cache = buffer_to_string(buffer);
        self.line_start_bytes.clear();
        self.line_start_bytes.push(0);
        for (idx, b) in self.source_cache.bytes().enumerate() {
            if b == b'\n' {
                self.line_start_bytes.push(idx + 1);
            }
        }
        self.tree = self.parser.parse(&self.source_cache, None);
        self.parse_version = buffer.version();
        self.cache_version.set(self.parse_version);
        self.highlight_cache.replace(vec![None; self.line_start_bytes.len()]);
    }

    /// Check if syntax highlighting is available
    pub fn has_highlighting(&self) -> bool {
        self.tree.is_some() && self.query.is_some()
    }

    /// Get highlights for a specific line
    pub fn get_line_highlights(&self, line: usize) -> Vec<HighlightSpan> {
        match (&self.tree, &self.query) {
            (Some(tree), Some(query)) => {
                if self.cache_version.get() != self.parse_version {
                    self.highlight_cache.replace(vec![None; self.line_start_bytes.len()]);
                    self.cache_version.set(self.parse_version);
                } else if self.highlight_cache.borrow().len() != self.line_start_bytes.len() {
                    self.highlight_cache.replace(vec![None; self.line_start_bytes.len()]);
                    self.cache_version.set(self.parse_version);
                }

                if let Some(cached) = self
                    .highlight_cache
                    .borrow()
                    .get(line)
                    .and_then(|entry| entry.as_ref())
                {
                    return cached.clone();
                }

                let spans = highlighter::get_line_highlights(
                    tree,
                    query,
                    &self.source_cache,
                    &self.line_start_bytes,
                    line,
                    &self.theme,
                );
                if let Some(entry) = self.highlight_cache.borrow_mut().get_mut(line) {
                    *entry = Some(spans.clone());
                }
                spans
            }
            _ => Vec::new(),
        }
    }

    /// Get the current language name
    pub fn language_name(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Set a new theme
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// Sync theme from the UI theme system
    pub fn sync_theme(&mut self, ui_theme: &crate::theme::Theme) {
        self.theme = Theme::from_ui_theme(ui_theme);
        // Invalidate cache since colors changed
        self.highlight_cache.borrow_mut().clear();
        self.cache_version.set(0);
    }
}

impl Default for SyntaxManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a buffer to a string for tree-sitter parsing
fn buffer_to_string(buffer: &Buffer) -> String {
    let mut result = String::new();
    for i in 0..buffer.len_lines() {
        if let Some(line) = buffer.line(i) {
            for ch in line.chars() {
                result.push(ch);
            }
        }
    }
    result
}

/// Get the line comment string for a language
/// Returns the comment prefix (e.g., "// " for Rust/JS, "# " for Python)
pub fn get_comment_string(language: Option<&str>) -> &'static str {
    match language {
        Some("rust") => "// ",
        Some("javascript") | Some("typescript") | Some("tsx") => "// ",
        Some("css") => "/* ",  // CSS only has block comments, but we use line-style
        Some("json") => "// ",  // JSON doesn't support comments, but some tools allow //
        Some("markdown") => "<!-- ",  // HTML-style for markdown
        Some("python") => "# ",
        Some("bash") | Some("shell") => "# ",
        Some("lua") => "-- ",
        Some("yaml") | Some("toml") => "# ",
        Some("go") | Some("c") | Some("cpp") | Some("java") | Some("swift") => "// ",
        Some("ruby") | Some("perl") => "# ",
        Some("html") | Some("xml") => "<!-- ",
        _ => "// ",  // Default fallback
    }
}

/// Get the closing comment string for block-style comments (if any)
/// Returns None for line-style comments like //
pub fn get_comment_end(language: Option<&str>) -> Option<&'static str> {
    match language {
        Some("css") => Some(" */"),
        Some("markdown") | Some("html") | Some("xml") => Some(" -->"),
        _ => None,
    }
}
