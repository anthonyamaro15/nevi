mod highlighter;
mod theme;

pub use highlighter::HighlightSpan;
pub use theme::{HighlightGroup, SyntaxStyle, Theme};

use std::cell::{Cell, RefCell};
use std::path::Path;
use tree_sitter::{Parser, Query, Tree};

use crate::editor::Buffer;

pub const MAX_HIGHLIGHT_LINES: usize = 200_000;
pub const MAX_HIGHLIGHT_CHARS: usize = 2_000_000;

fn to_input_edit(edit: &crate::editor::BufferEdit) -> tree_sitter::InputEdit {
    tree_sitter::InputEdit {
        start_byte: edit.start_byte,
        old_end_byte: edit.old_end_byte,
        new_end_byte: edit.new_end_byte,
        start_position: tree_sitter::Point {
            row: edit.start_point.0,
            column: edit.start_point.1,
        },
        old_end_position: tree_sitter::Point {
            row: edit.old_end_point.0,
            column: edit.old_end_point.1,
        },
        new_end_position: tree_sitter::Point {
            row: edit.new_end_point.0,
            column: edit.new_end_point.1,
        },
    }
}

pub fn exceeds_highlight_limits(line_count: usize, char_count: usize) -> bool {
    line_count > MAX_HIGHLIGHT_LINES || char_count > MAX_HIGHLIGHT_CHARS
}

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
    /// Sorted function boundaries for `]m`-family motions, computed lazily
    /// from the current tree (the collecting walk costs ~5ms on a 13k-line
    /// file, so keypresses after the first are a binary search). Cleared on
    /// every parse rather than version-keyed: parse_version mirrors
    /// buffer.version(), which can collide across buffer switches.
    method_boundaries: RefCell<Option<crate::method_motion::MethodBoundaries>>,
    /// (buffer id, language) the current tree was parsed from. Incremental
    /// reparse is only safe when both still match; anything else falls back
    /// to a full parse.
    incremental_identity: Option<(u64, String)>,
}

/// Kill switch: NEVI_INCREMENTAL_PARSE=0 forces full reparses, in case an
/// incremental corruption slips past the equivalence tests in the wild.
fn incremental_parse_disabled() -> bool {
    static DISABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DISABLED
        .get_or_init(|| std::env::var("NEVI_INCREMENTAL_PARSE").is_ok_and(|value| value == "0"))
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
            method_boundaries: RefCell::new(None),
            incremental_identity: None,
        }
    }

    /// Detect language from file path and set up parser
    pub fn set_language_from_path(&mut self, path: &Path) {
        self.set_language_from_path_and_first_line(path, None);
    }

    /// Detect language from path, then the first line (shebang) if the path is unknown.
    pub fn set_language_from_path_and_first_line(&mut self, path: &Path, first_line: Option<&str>) {
        let extension = path.extension().and_then(|e| e.to_str());

        if is_ruby_path(path, extension) {
            self.set_ruby_language();
            return;
        }

        if is_shell_path(path) {
            self.set_shell_language();
            return;
        }

        match extension {
            Some("rs") => self.set_rust_language(),
            Some("js") | Some("mjs") | Some("cjs") => self.set_javascript_language(),
            Some("jsx") => self.set_javascript_language(), // JSX uses same parser
            Some("ts") | Some("mts") | Some("cts") => self.set_typescript_language(),
            Some("tsx") => self.set_tsx_language(),
            Some("css") => self.set_css_language(),
            Some("scss") | Some("sass") => self.set_scss_language(),
            Some("json") | Some("jsonc") => self.set_json_language(),
            Some("md") | Some("markdown") => self.set_markdown_language(),
            Some("toml") => self.set_toml_language(),
            Some("yaml") | Some("yml") => self.set_yaml_language(),
            Some("html") | Some("htm") => self.set_html_language(),
            Some("py") | Some("pyi") | Some("pyw") => self.set_python_language(),
            Some("php") => self.set_php_language(),
            Some("go") => self.set_go_language(),
            _ => {
                if first_line.is_some_and(shebang_is_shell) {
                    self.set_shell_language();
                    return;
                }
                self.clear_language();
            }
        }
    }

    /// Detect language from optional file path
    pub fn set_language_from_path_option(&mut self, path: Option<&std::path::PathBuf>) {
        self.set_language_from_path_option_and_first_line(path, None);
    }

    pub fn set_language_from_path_option_and_first_line(
        &mut self,
        path: Option<&std::path::PathBuf>,
        first_line: Option<&str>,
    ) {
        if let Some(p) = path {
            self.set_language_from_path_and_first_line(p, first_line);
        } else if first_line.is_some_and(shebang_is_shell) {
            self.set_shell_language();
        } else {
            self.clear_language();
        }
    }

    fn clear_language(&mut self) {
        self.language = None;
        self.query = None;
        self.tree = None;
        self.incremental_identity = None;
        self.source_cache.clear();
        self.line_start_bytes.clear();
        self.highlight_cache.borrow_mut().clear();
        self.cache_version.set(0);
        self.parse_version = 0;
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

    /// Set up SCSS/Sass highlighting.
    ///
    /// This currently reuses the CSS grammar and SCSS query path. It preserves
    /// the filetype name so language-aware behavior does not collapse SCSS into
    /// plain CSS.
    fn set_scss_language(&mut self) {
        let language = tree_sitter_css::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("scss".to_string());

                let query_source = highlighter::scss_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("scss (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("scss (lang error: {:?})", e));
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

    /// Set up TOML language parser
    fn set_toml_language(&mut self) {
        let language = tree_sitter_toml_ng::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("toml".to_string());

                let query_source = highlighter::toml_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("toml (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("toml (lang error: {:?})", e));
            }
        }
    }

    /// Set up YAML language highlighting.
    /// Uses lightweight tokenization instead of tree-sitter grammar.
    fn set_yaml_language(&mut self) {
        self.language = Some("yaml".to_string());
        self.query = None;
        self.tree = None;
    }

    /// Set up HTML language parser
    fn set_html_language(&mut self) {
        let language = tree_sitter_html::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("html".to_string());

                let query_source = highlighter::html_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("html (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("html (lang error: {:?})", e));
            }
        }
    }

    /// Set up Python language parser
    fn set_python_language(&mut self) {
        let language = tree_sitter_python::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("python".to_string());

                let query_source = highlighter::python_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("python (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("python (lang error: {:?})", e));
            }
        }
    }

    /// Set up PHP language parser
    fn set_php_language(&mut self) {
        let language = tree_sitter_php::LANGUAGE_PHP;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("php".to_string());

                let query_source = highlighter::php_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("php (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("php (lang error: {:?})", e));
            }
        }
    }

    /// Set up Go language parser
    fn set_go_language(&mut self) {
        let language = tree_sitter_go::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("go".to_string());

                let query_source = highlighter::go_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("go (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("go (lang error: {:?})", e));
            }
        }
    }

    /// Set up Ruby language parser
    fn set_ruby_language(&mut self) {
        let language = tree_sitter_ruby::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("ruby".to_string());

                let query_source = highlighter::ruby_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("ruby (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("ruby (lang error: {:?})", e));
            }
        }
    }

    /// Set up Bash / POSIX shell language parser
    fn set_shell_language(&mut self) {
        let language = tree_sitter_bash::LANGUAGE;
        match self.parser.set_language(&language.into()) {
            Ok(()) => {
                self.language = Some("shell".to_string());

                let query_source = highlighter::shell_highlight_query();
                match Query::new(&language.into(), query_source) {
                    Ok(query) => {
                        self.query = Some(query);
                    }
                    Err(e) => {
                        self.language = Some(format!("shell (query error: {:?})", e));
                        self.query = None;
                    }
                }
            }
            Err(e) => {
                self.language = Some(format!("shell (lang error: {:?})", e));
            }
        }
    }

    /// Parse the buffer. Reuses the previous tree via tree-sitter's
    /// incremental parsing when the buffer's recorded edits exactly cover
    /// the span since the last parse; otherwise falls back to a full parse.
    pub fn parse(&mut self, buffer: &mut Buffer) {
        if self.language.is_none() {
            // No language means no tree; keep the buffer's edit queue from
            // growing without bound.
            buffer.reset_edit_history();
            return;
        }
        self.method_boundaries.replace(None);

        if self.language.as_deref() == Some("yaml") {
            self.source_cache = buffer_to_string(buffer);
            self.line_start_bytes.clear();
            self.line_start_bytes.push(0);
            for (idx, b) in self.source_cache.bytes().enumerate() {
                if b == b'\n' {
                    self.line_start_bytes.push(idx + 1);
                }
            }
            self.tree = None;
            self.query = None;
            self.incremental_identity = None;
            buffer.reset_edit_history();
            self.parse_version = buffer.version();
            self.cache_version.set(self.parse_version);
            self.highlight_cache
                .replace(vec![None; self.line_start_bytes.len()]);
            return;
        }

        if exceeds_highlight_limits(buffer.len_lines(), buffer.len_chars()) {
            self.tree = None;
            self.incremental_identity = None;
            buffer.reset_edit_history();
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
        // Reuse the old tree when it belongs to this exact buffer and
        // language AND the buffer's edit queue covers precisely the span
        // since the last parse. tree.edit() maps the old tree onto the new
        // byte layout; tree-sitter then re-lexes only around the changes.
        let identity = (buffer.id(), self.language.clone().unwrap_or_default());
        let mut old_tree = None;
        if !incremental_parse_disabled() && self.incremental_identity.as_ref() == Some(&identity) {
            if let (Some(mut tree), Some(edits)) = (
                self.tree.take(),
                buffer.take_edits_since(self.parse_version),
            ) {
                for edit in &edits {
                    tree.edit(&to_input_edit(edit));
                }
                old_tree = Some(tree);
            }
        }
        if old_tree.is_none() {
            buffer.reset_edit_history();
        }
        self.tree = self.parser.parse(&self.source_cache, old_tree.as_ref());
        self.incremental_identity = self.tree.is_some().then_some(identity);
        self.parse_version = buffer.version();
        self.cache_version.set(self.parse_version);
        self.highlight_cache
            .replace(vec![None; self.line_start_bytes.len()]);
    }

    /// Parse string content directly (for preview panels, etc.)
    /// Designed for small content like finder preview (~150 lines max)
    pub fn parse_string(&mut self, content: &str) {
        if self.language.is_none() {
            return;
        }
        self.method_boundaries.replace(None);

        if self.language.as_deref() == Some("yaml") {
            self.source_cache = content.to_string();
            self.line_start_bytes.clear();
            self.line_start_bytes.push(0);
            for (idx, b) in self.source_cache.bytes().enumerate() {
                if b == b'\n' {
                    self.line_start_bytes.push(idx + 1);
                }
            }
            self.tree = None;
            self.query = None;
            self.parse_version = self.parse_version.wrapping_add(1);
            self.cache_version.set(self.parse_version);
            self.highlight_cache
                .replace(vec![None; self.line_start_bytes.len()]);
            return;
        }

        // Safety limits - preview content is already capped at ~150 lines
        // These are just failsafes in case of unexpected input
        const MAX_HIGHLIGHT_LINES: usize = 200;
        const MAX_HIGHLIGHT_CHARS: usize = 20_000;

        let line_count = content.lines().count();
        let char_count = content.chars().count();

        if line_count > MAX_HIGHLIGHT_LINES || char_count > MAX_HIGHLIGHT_CHARS {
            self.tree = None;
            self.source_cache.clear();
            self.line_start_bytes.clear();
            self.highlight_cache.borrow_mut().clear();
            self.cache_version.set(0);
            return;
        }

        self.source_cache = content.to_string();
        self.line_start_bytes.clear();
        self.line_start_bytes.push(0);
        for (idx, b) in self.source_cache.bytes().enumerate() {
            if b == b'\n' {
                self.line_start_bytes.push(idx + 1);
            }
        }
        self.tree = self.parser.parse(&self.source_cache, None);
        self.incremental_identity = None;
        self.parse_version = self.parse_version.wrapping_add(1);
        self.cache_version.set(self.parse_version);
        self.highlight_cache
            .replace(vec![None; self.line_start_bytes.len()]);
    }

    /// Check if syntax highlighting is available
    pub fn has_highlighting(&self) -> bool {
        self.language.as_deref() == Some("yaml") || (self.tree.is_some() && self.query.is_some())
    }

    /// Get highlights for a specific line
    pub fn get_line_highlights(&self, line: usize) -> Vec<HighlightSpan> {
        if self.language.as_deref() == Some("yaml") {
            if self.cache_version.get() != self.parse_version {
                self.highlight_cache
                    .replace(vec![None; self.line_start_bytes.len()]);
                self.cache_version.set(self.parse_version);
            } else if self.highlight_cache.borrow().len() != self.line_start_bytes.len() {
                self.highlight_cache
                    .replace(vec![None; self.line_start_bytes.len()]);
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

            let spans = highlighter::get_line_highlights_yaml(
                &self.source_cache,
                &self.line_start_bytes,
                line,
                &self.theme,
            );
            if let Some(entry) = self.highlight_cache.borrow_mut().get_mut(line) {
                *entry = Some(spans.clone());
            }
            return spans;
        }

        match (&self.tree, &self.query) {
            (Some(tree), Some(query)) => {
                if self.cache_version.get() != self.parse_version {
                    self.highlight_cache
                        .replace(vec![None; self.line_start_bytes.len()]);
                    self.cache_version.set(self.parse_version);
                } else if self.highlight_cache.borrow().len() != self.line_start_bytes.len() {
                    self.highlight_cache
                        .replace(vec![None; self.line_start_bytes.len()]);
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

    /// Resolve a `]m`-family target in the current tree's snapshot.
    ///
    /// The boundary list is computed on first use after each parse and then
    /// answered by binary search, so repeated presses cost microseconds.
    pub fn method_motion_target(
        &self,
        cursor_byte: usize,
        boundary: crate::method_motion::MethodBoundary,
        count: usize,
    ) -> Option<(usize, usize)> {
        let (tree, source) = self.get_tree_and_source()?;
        let language = self.language.as_deref()?;
        let mut cache = self.method_boundaries.borrow_mut();
        cache
            .get_or_insert_with(|| {
                crate::method_motion::MethodBoundaries::collect(tree, source, language)
            })
            .find(boundary, cursor_byte, count)
    }

    /// Get the syntax tree and source for indent calculation.
    ///
    /// Returns a reference to the parsed tree and the cached source text.
    pub fn get_tree_and_source(&self) -> Option<(&Tree, &str)> {
        self.tree.as_ref().map(|t| (t, self.source_cache.as_str()))
    }

    /// Convert a (line, col) position to a byte offset in the source.
    ///
    /// # Arguments
    /// * `line` - Zero-based line number
    /// * `col` - Zero-based column number (in characters, not bytes)
    ///
    /// # Returns
    /// The byte offset, or None if the position is invalid
    pub fn position_to_byte(&self, line: usize, col: usize) -> Option<usize> {
        if line >= self.line_start_bytes.len() {
            return None;
        }

        let line_start = self.line_start_bytes[line];

        // Get the line content and convert character offset to byte offset
        let line_end = self
            .line_start_bytes
            .get(line + 1)
            .copied()
            .unwrap_or(self.source_cache.len());

        let line_content = &self.source_cache[line_start..line_end];

        // Convert character column to byte offset within the line
        let mut byte_offset = 0;
        for (char_idx, ch) in line_content.chars().enumerate() {
            if char_idx >= col {
                break;
            }
            byte_offset += ch.len_utf8();
        }

        Some(line_start + byte_offset)
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

fn is_ruby_path(path: &Path, extension: Option<&str>) -> bool {
    if matches!(
        extension,
        Some("rb" | "rake" | "gemspec" | "ru" | "podspec")
    ) {
        return true;
    }

    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(
            "Appraisals"
                | "Berksfile"
                | "Brewfile"
                | "Capfile"
                | "Cheffile"
                | "Fastfile"
                | "Gemfile"
                | "Guardfile"
                | "Podfile"
                | "Rakefile"
                | "Thorfile"
                | "Vagrantfile"
        )
    )
}

/// True when the path is a shell script by extension or a common rc/profile filename.
pub fn is_shell_path(path: &Path) -> bool {
    let extension = path.extension().and_then(|e| e.to_str());
    if matches!(
        extension,
        Some("sh" | "bash" | "zsh" | "ksh" | "bats" | "ebuild" | "eclass")
    ) {
        return true;
    }

    is_shell_filename(path)
}

fn is_shell_filename(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(
            ".bashrc"
                | "bashrc"
                | "bash.bashrc"
                | ".bashrc.local"
                | ".bash_profile"
                | ".bashprofile"
                | "bash_profile"
                | ".bash_login"
                | ".bash_logout"
                | ".bash_aliases"
                | ".bash_functions"
                | ".profile"
                | "profile"
                | ".zshrc"
                | "zshrc"
                | ".zshrc.local"
                | ".zshenv"
                | "zshenv"
                | ".zprofile"
                | "zprofile"
                | ".zlogin"
                | "zlogin"
                | ".zlogout"
                | "zlogout"
                | ".zaliases"
                | ".zsh_aliases"
                | ".kshrc"
                | ".mkshrc"
                | ".envrc"
                | "PKGBUILD"
                | "APKBUILD"
        )
    )
}

/// True when the first line is a POSIX/bash/zsh shebang (`#!/bin/bash`, `#!/usr/bin/env bash`).
pub fn shebang_is_shell(first_line: &str) -> bool {
    matches!(
        shebang_interpreter(first_line),
        Some("sh" | "bash" | "dash" | "ash" | "ksh" | "mksh" | "zsh")
    )
}

fn shebang_interpreter(first_line: &str) -> Option<&str> {
    let line = first_line.trim_end_matches(['\r', '\n']);
    let rest = line.strip_prefix("#!")?.trim();
    let mut parts = rest.split_whitespace();
    let command = parts.next()?;
    let name = Path::new(command).file_name()?.to_str()?;
    if name.eq_ignore_ascii_case("env") {
        for arg in parts {
            if arg.starts_with('-') {
                continue;
            }
            return Path::new(arg).file_name()?.to_str();
        }
        return None;
    }
    Some(name)
}

/// Get the line comment string for a language
/// Returns the comment prefix (e.g., "// " for Rust/JS, "# " for Python)
pub fn get_comment_string(language: Option<&str>) -> &'static str {
    match language {
        Some("rust") => "// ",
        Some("javascript") | Some("typescript") | Some("tsx") => "// ",
        Some("css") | Some("scss") => "/* ", // CSS only has block comments, but we use line-style
        Some("json") => "// ", // JSON doesn't support comments, but some tools allow //
        Some("markdown") => "<!-- ", // HTML-style for markdown
        Some("python") => "# ",
        Some("bash") | Some("shell") => "# ",
        Some("lua") => "-- ",
        Some("yaml") | Some("toml") => "# ",
        Some("php") | Some("go") | Some("c") | Some("cpp") | Some("java") | Some("swift") => "// ",
        Some("ruby") | Some("perl") => "# ",
        Some("html") | Some("xml") => "<!-- ",
        _ => "// ", // Default fallback
    }
}

/// Get the closing comment string for block-style comments (if any)
/// Returns None for line-style comments like //
pub fn get_comment_end(language: Option<&str>) -> Option<&'static str> {
    match language {
        Some("css") | Some("scss") => Some(" */"),
        Some("markdown") | Some("html") | Some("xml") => Some(" -->"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn jsonc_extension_uses_json_highlighting() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path(Path::new("settings.jsonc"));

        let mut buffer = Buffer::new();
        buffer.set_content("{\"enabled\": true}\n");
        syntax.parse(&mut buffer);

        assert_eq!(syntax.language_name(), Some("json"));
        assert!(syntax.has_highlighting());
        assert!(
            !syntax.get_line_highlights(0).is_empty(),
            "jsonc should reuse JSON syntax highlighting"
        );
    }

    #[test]
    fn scss_extension_keeps_scss_language_name_and_highlights() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path(Path::new("styles.scss"));

        let mut buffer = Buffer::new();
        buffer.set_content("$accent: #ff00aa;\n.button { color: $accent; }\n");
        syntax.parse(&mut buffer);

        assert_eq!(syntax.language_name(), Some("scss"));
        assert!(syntax.has_highlighting());
        assert!(
            !syntax.get_line_highlights(1).is_empty(),
            "scss should use the SCSS highlight path instead of plain text"
        );
    }

    #[test]
    fn go_extension_uses_go_highlighting() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path(Path::new("main.go"));

        let mut buffer = Buffer::new();
        buffer.set_content("package main\n\nfunc main() {\n\tprintln(\"hello\")\n}\n");
        syntax.parse(&mut buffer);

        assert_eq!(syntax.language_name(), Some("go"));
        assert!(syntax.has_highlighting());
        assert!(
            !syntax.get_line_highlights(0).is_empty(),
            "go files should use Go syntax highlighting"
        );
    }

    #[test]
    fn ruby_extension_uses_ruby_highlighting() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path(Path::new("example.rb"));

        let mut buffer = Buffer::new();
        buffer.set_content(
            "class Greeter\n  def hello(name)\n    puts \"hello #{name}\"\n  end\nend\n",
        );
        syntax.parse(&mut buffer);

        assert_eq!(syntax.language_name(), Some("ruby"));
        assert!(syntax.has_highlighting());
        assert!(
            !syntax.get_line_highlights(0).is_empty(),
            "ruby files should use Ruby syntax highlighting"
        );
    }

    #[test]
    fn ruby_known_filenames_use_ruby_highlighting() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path(Path::new("Gemfile"));

        let mut buffer = Buffer::new();
        buffer.set_content("source \"https://rubygems.org\"\ngem \"rails\"\n");
        syntax.parse(&mut buffer);

        assert_eq!(syntax.language_name(), Some("ruby"));
        assert!(syntax.has_highlighting());
        assert!(
            !syntax.get_line_highlights(1).is_empty(),
            "common Ruby filenames should use Ruby syntax highlighting"
        );
    }

    #[test]
    fn php_extension_uses_php_highlighting() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path(Path::new("index.php"));

        let mut buffer = Buffer::new();
        buffer.set_content(
            "<?php\nfunction greet(string $name): void {\n    echo \"Hello $name\";\n}\n",
        );
        syntax.parse(&mut buffer);

        assert_eq!(syntax.language_name(), Some("php"));
        assert!(syntax.has_highlighting());
        assert!(
            !syntax.get_line_highlights(1).is_empty(),
            "php files should use PHP syntax highlighting"
        );
    }

    #[test]
    fn php_uses_slash_slash_line_comments() {
        assert_eq!(get_comment_string(Some("php")), "// ");
        assert_eq!(get_comment_end(Some("php")), None);
    }

    fn parse_shell_snippet(syntax: &mut SyntaxManager) {
        let mut buffer = Buffer::new();
        buffer.set_content("if true; then\n  echo hello\nfi\n");
        syntax.parse(&mut buffer);
        assert_eq!(syntax.language_name(), Some("shell"));
        assert!(syntax.has_highlighting());
        assert!(
            !syntax.get_line_highlights(0).is_empty(),
            "shell files should use Bash syntax highlighting"
        );
    }

    #[test]
    fn sh_extension_uses_shell_highlighting() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path(Path::new("bin/setup.sh"));
        parse_shell_snippet(&mut syntax);
    }

    #[test]
    fn bash_and_zsh_extensions_use_shell_highlighting() {
        for path in ["script.bash", "script.zsh"] {
            let mut syntax = SyntaxManager::new();
            syntax.set_language_from_path(Path::new(path));
            parse_shell_snippet(&mut syntax);
        }
    }

    #[test]
    fn common_shell_rc_filenames_use_shell_highlighting() {
        for path in [
            ".bashrc",
            ".bash_profile",
            ".bashprofile",
            ".zshrc",
            ".zshenv",
            ".profile",
            "PKGBUILD",
            ".envrc",
        ] {
            let mut syntax = SyntaxManager::new();
            syntax.set_language_from_path(Path::new(path));
            parse_shell_snippet(&mut syntax);
        }
    }

    #[test]
    fn extensionless_shebang_uses_shell_highlighting() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path_and_first_line(
            Path::new("bin/deploy"),
            Some("#!/usr/bin/env bash\n"),
        );
        parse_shell_snippet(&mut syntax);
    }

    #[test]
    fn env_dash_s_shebang_uses_shell_highlighting() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path_and_first_line(
            Path::new("bin/deploy"),
            Some("#!/usr/bin/env -S bash -eu\n"),
        );
        parse_shell_snippet(&mut syntax);
    }

    #[test]
    fn fish_and_python_shebangs_do_not_use_shell_highlighting() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path_and_first_line(
            Path::new("bin/deploy"),
            Some("#!/usr/bin/env fish\n"),
        );
        assert_eq!(syntax.language_name(), None);

        syntax.set_language_from_path_and_first_line(
            Path::new("bin/deploy"),
            Some("#!/usr/bin/env python3\n"),
        );
        assert_eq!(syntax.language_name(), None);
    }

    #[test]
    fn path_wins_over_mismatched_shebang() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path_and_first_line(
            Path::new("main.py"),
            Some("#!/usr/bin/env bash\n"),
        );
        assert_eq!(syntax.language_name(), Some("python"));
    }

    #[test]
    fn shebang_is_shell_recognizes_common_interpreters() {
        assert!(shebang_is_shell("#!/bin/bash"));
        assert!(shebang_is_shell("#!/bin/sh\n"));
        assert!(shebang_is_shell("#!/usr/bin/env zsh"));
        assert!(!shebang_is_shell("#!/usr/bin/env fish"));
        assert!(!shebang_is_shell("echo hi"));
    }

    use crate::editor::Buffer;

    /// The incremental tree must be indistinguishable from a from-scratch
    /// parse of the same text. This is the guard whose absence caused the
    /// original old-tree corruption.
    fn assert_tree_matches_fresh_parse(syntax: &SyntaxManager) {
        let (tree, source) = syntax.get_tree_and_source().expect("tree after parse");
        let mut fresh_parser = Parser::new();
        fresh_parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("rust grammar");
        let fresh = fresh_parser.parse(source, None).expect("fresh parse");
        assert_eq!(
            tree.root_node().to_sexp(),
            fresh.root_node().to_sexp(),
            "incremental tree diverged from full parse\nsource:\n{source}"
        );
    }

    fn rust_syntax_and_buffer(content: &str) -> (SyntaxManager, Buffer) {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path(Path::new("fuzz.rs"));
        let mut buffer = Buffer::new();
        buffer.insert_str(0, 0, content);
        syntax.parse(&mut buffer);
        (syntax, buffer)
    }

    #[test]
    fn incremental_parse_survives_targeted_edits() {
        let (mut syntax, mut buffer) = rust_syntax_and_buffer("fn main() {\n    let x = 1;\n}\n");

        // Insert a newline mid-line, splitting a statement.
        buffer.insert_char(1, 7, '\n');
        syntax.parse(&mut buffer);
        assert_tree_matches_fresh_parse(&syntax);

        // Multibyte insert before existing code.
        buffer.insert_str(0, 3, "h\u{e9}llo_\u{2713}");
        syntax.parse(&mut buffer);
        assert_tree_matches_fresh_parse(&syntax);

        // Delete a range spanning lines.
        buffer.delete_range(0, 2, 2, 1);
        syntax.parse(&mut buffer);
        assert_tree_matches_fresh_parse(&syntax);

        // replace_line and the undo-shaped apply_change.
        buffer.replace_line(0, "fn other() { let y = 2; }");
        syntax.parse(&mut buffer);
        assert_tree_matches_fresh_parse(&syntax);

        buffer.apply_change(0, 3, "other", "renamed");
        syntax.parse(&mut buffer);
        assert_tree_matches_fresh_parse(&syntax);
    }

    #[test]
    fn incremental_parse_matches_full_parse_under_random_edits() {
        let (mut syntax, mut buffer) = rust_syntax_and_buffer(
            "fn main() {\n    let x = 1;\n    println!(\"h\u{e9}llo {}\", x);\n}\n",
        );
        assert_tree_matches_fresh_parse(&syntax);

        // Deterministic xorshift so failures reproduce.
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let snippets = [
            "let y = 2;",
            "fn f() {}",
            "\u{e9}\u{2713}",
            "\"s\"",
            "}",
            "{",
            "// note \u{f8}\n",
        ];

        for _ in 0..150 {
            // 1-4 edits per parse, like a debounced typing burst.
            for _ in 0..(1 + rand() % 4) {
                let line_count = buffer.addressable_line_count().max(1);
                let line = (rand() as usize) % line_count;
                let col = (rand() as usize) % (buffer.line_len(line) + 1);
                match rand() % 4 {
                    0 => {
                        let s = snippets[(rand() as usize) % snippets.len()];
                        buffer.insert_str(line, col, s);
                    }
                    1 => {
                        let ch = ['a', '\u{e9}', '\n', '}'][(rand() as usize) % 4];
                        buffer.insert_char(line, col, ch);
                    }
                    2 => buffer.delete_char(line, col),
                    _ => {
                        let end_line = (line + (rand() as usize) % 2)
                            .min(buffer.addressable_line_count().saturating_sub(1));
                        let end_col = (rand() as usize) % (buffer.line_len(end_line) + 1);
                        buffer.delete_range(line, col, end_line, end_col);
                    }
                }
            }
            syntax.parse(&mut buffer);
            assert_tree_matches_fresh_parse(&syntax);
        }
    }

    #[test]
    fn bulk_replace_and_buffer_switch_fall_back_to_full_parse() {
        let (mut syntax, mut buffer) = rust_syntax_and_buffer("fn a() {}\n");

        // set_content breaks the edit history; the next parse must be full
        // and still correct.
        buffer.set_content("fn b() { let z = 3; }\n");
        syntax.parse(&mut buffer);
        assert_tree_matches_fresh_parse(&syntax);

        // A different buffer must never reuse this buffer's tree.
        let mut other = Buffer::new();
        other.insert_str(0, 0, "fn c() {}\n");
        syntax.parse(&mut other);
        assert_tree_matches_fresh_parse(&syntax);
    }
}
