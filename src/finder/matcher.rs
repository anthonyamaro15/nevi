use nucleo::Matcher;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::Utf32Str;

/// Fuzzy matcher wrapper using nucleo
pub struct FuzzyMatcher {
    matcher: Matcher,
    /// Cached query string (to detect when pattern needs recompiling)
    cached_query: String,
    /// Cached compiled pattern
    cached_pattern: Pattern,
}

impl FuzzyMatcher {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(nucleo::Config::DEFAULT),
            cached_query: String::new(),
            cached_pattern: Pattern::new(
                "",
                CaseMatching::Smart,
                Normalization::Smart,
                nucleo::pattern::AtomKind::Fuzzy,
            ),
        }
    }

    /// Update cached pattern if query changed
    fn ensure_pattern(&mut self, query: &str) {
        if self.cached_query != query {
            self.cached_query = query.to_string();
            self.cached_pattern = Pattern::new(
                query,
                CaseMatching::Smart,
                Normalization::Smart,
                nucleo::pattern::AtomKind::Fuzzy,
            );
        }
    }

    /// Check if a query matches a string and return the score
    /// Higher score = better match
    /// Returns None if no match
    pub fn match_score(&mut self, query: &str, text: &str) -> Option<u32> {
        if query.is_empty() {
            return Some(0);
        }

        // Update pattern if query changed (avoids recompiling for same query)
        self.ensure_pattern(query);

        // Fast path: ASCII-only text avoids Vec<char> allocation
        if text.is_ascii() {
            let utf32_str = Utf32Str::Ascii(text.as_bytes());
            self.cached_pattern.score(utf32_str, &mut self.matcher)
        } else {
            // Slow path: convert to UTF-32 for non-ASCII text
            let text_chars: Vec<char> = text.chars().collect();
            let utf32_str = Utf32Str::Unicode(&text_chars);
            self.cached_pattern.score(utf32_str, &mut self.matcher)
        }
    }

    /// Get match indices for highlighting
    pub fn match_indices(&mut self, query: &str, text: &str) -> Vec<usize> {
        if query.is_empty() {
            return Vec::new();
        }

        // Update pattern if query changed (avoids recompiling for same query)
        self.ensure_pattern(query);

        let mut indices = Vec::new();

        // Fast path: ASCII-only text avoids Vec<char> allocation
        if text.is_ascii() {
            let utf32_str = Utf32Str::Ascii(text.as_bytes());
            self.cached_pattern
                .indices(utf32_str, &mut self.matcher, &mut indices);
        } else {
            // Slow path: convert to UTF-32 for non-ASCII text
            let text_chars: Vec<char> = text.chars().collect();
            let utf32_str = Utf32Str::Unicode(&text_chars);
            self.cached_pattern
                .indices(utf32_str, &mut self.matcher, &mut indices);
        }

        indices.into_iter().map(|i| i as usize).collect()
    }
}

impl Default for FuzzyMatcher {
    fn default() -> Self {
        Self::new()
    }
}
