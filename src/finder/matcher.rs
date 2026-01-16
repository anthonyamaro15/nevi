use nucleo::Matcher;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::Utf32Str;

/// Fuzzy matcher wrapper using nucleo
pub struct FuzzyMatcher {
    matcher: Matcher,
}

impl FuzzyMatcher {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(nucleo::Config::DEFAULT),
        }
    }

    /// Check if a query matches a string and return the score
    /// Higher score = better match
    /// Returns None if no match
    pub fn match_score(&mut self, query: &str, text: &str) -> Option<u32> {
        if query.is_empty() {
            return Some(0);
        }

        let pattern = Pattern::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            nucleo::pattern::AtomKind::Fuzzy,
        );

        // Convert text to UTF-32 for nucleo
        let text_chars: Vec<char> = text.chars().collect();
        let utf32_str = Utf32Str::Unicode(&text_chars);

        pattern.score(utf32_str, &mut self.matcher)
    }

    /// Get match indices for highlighting
    pub fn match_indices(&mut self, query: &str, text: &str) -> Vec<usize> {
        if query.is_empty() {
            return Vec::new();
        }

        let pattern = Pattern::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            nucleo::pattern::AtomKind::Fuzzy,
        );

        let text_chars: Vec<char> = text.chars().collect();
        let utf32_str = Utf32Str::Unicode(&text_chars);
        let mut indices = Vec::new();

        pattern.indices(utf32_str, &mut self.matcher, &mut indices);

        indices.into_iter().map(|i| i as usize).collect()
    }
}

impl Default for FuzzyMatcher {
    fn default() -> Self {
        Self::new()
    }
}
