mod highlighter;
mod theme;

pub use highlighter::HighlightSpan;
pub use theme::{HighlightGroup, Theme};

use std::path::Path;
use tree_sitter::{Parser, Query, Tree};

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
        }
    }

    /// Detect language from file path and set up parser
    pub fn set_language_from_path(&mut self, path: &Path) {
        let extension = path.extension().and_then(|e| e.to_str());

        match extension {
            Some("rs") => self.set_rust_language(),
            // Add more languages here as needed
            _ => {
                self.language = None;
                self.query = None;
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

    /// Parse the entire buffer
    pub fn parse(&mut self, buffer: &Buffer) {
        if self.language.is_none() {
            return;
        }

        // Convert buffer to string for parsing
        self.source_cache = buffer_to_string(buffer);
        self.tree = self.parser.parse(&self.source_cache, None);
    }

    /// Check if syntax highlighting is available
    pub fn has_highlighting(&self) -> bool {
        self.tree.is_some() && self.query.is_some()
    }

    /// Get highlights for a specific line
    pub fn get_line_highlights(&self, line: usize) -> Vec<HighlightSpan> {
        match (&self.tree, &self.query) {
            (Some(tree), Some(query)) => {
                highlighter::get_line_highlights(tree, query, &self.source_cache, line, &self.theme)
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
