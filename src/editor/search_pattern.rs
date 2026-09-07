//! Literal search pattern with Vim's word-boundary atoms.
//!
//! Search is a plain substring match, with one borrowing from Vim's regex
//! syntax: a leading `\<` requires the match to start a keyword and a
//! trailing `\>` requires it to end one. `*` and `#` record `\<word\>`
//! exactly like Vim does, so `n`, `N`, `gn`, `gN`, the highlights, the
//! match counter, and an empty `/` prompt all respect the boundaries
//! without a separate flag that could drift from the stored pattern.

use super::Editor;

pub struct SearchPattern<'a> {
    /// The literal text once the boundary atoms are stripped.
    pub text: &'a str,
    word_start: bool,
    word_end: bool,
}

impl<'a> SearchPattern<'a> {
    pub fn parse(pattern: &'a str) -> Self {
        let (text, word_start) = match pattern.strip_prefix("\\<") {
            Some(rest) => (rest, true),
            None => (pattern, false),
        };
        let (text, word_end) = match text.strip_suffix("\\>") {
            Some(rest) => (rest, true),
            None => (text, false),
        };
        Self {
            text,
            word_start,
            word_end,
        }
    }

    /// The pattern `*` and `#` record for a keyword under the cursor.
    pub fn whole_word(word: &str) -> String {
        format!("\\<{word}\\>")
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Byte offset of the first match starting at or after `from`.
    pub fn find(&self, haystack: &str, from: usize) -> Option<usize> {
        let mut from = from;
        while from <= haystack.len() {
            let pos = from + haystack[from..].find(self.text)?;
            if self.bounded(haystack, pos) {
                return Some(pos);
            }
            // Step one char past the rejected start so overlapping
            // occurrences are still considered.
            from = pos + haystack[pos..].chars().next().map_or(1, char::len_utf8);
        }
        None
    }

    /// Byte offset of the last match that ends at or before `end`.
    pub fn rfind(&self, haystack: &str, end: usize) -> Option<usize> {
        let mut end = end;
        loop {
            let pos = haystack[..end].rfind(self.text)?;
            if self.bounded(haystack, pos) {
                return Some(pos);
            }
            // Retry with the window ending inside the rejected match so an
            // overlapping earlier occurrence is still considered. The last
            // char's start is a char boundary; `pos + len - 1` may not be.
            let last_char_len = self.text.chars().next_back().map_or(1, char::len_utf8);
            end = pos + self.text.len() - last_char_len;
        }
    }

    /// Vim's `\<` needs a keyword char at the match start and none before
    /// it; `\>` needs a keyword char at the match end and none after it.
    fn bounded(&self, haystack: &str, pos: usize) -> bool {
        let is_word = |ch: char| Editor::is_word_char(ch);
        let match_end = pos + self.text.len();
        let start_ok = !self.word_start
            || (self.text.chars().next().is_some_and(is_word)
                && !haystack[..pos].chars().next_back().is_some_and(is_word));
        let end_ok = !self.word_end
            || (self.text.chars().next_back().is_some_and(is_word)
                && !haystack[match_end..].chars().next().is_some_and(is_word));
        start_ok && end_ok
    }
}

#[cfg(test)]
mod tests {
    use super::SearchPattern;

    #[test]
    fn plain_pattern_matches_inside_words() {
        let pat = SearchPattern::parse("abc");
        assert_eq!(pat.find("xabcdef", 0), Some(1));
        assert_eq!(pat.rfind("xabcdef", 7), Some(1));
    }

    #[test]
    fn parse_strips_boundary_atoms() {
        let pat = SearchPattern::parse("\\<abc\\>");
        assert_eq!(pat.text, "abc");
        assert!(pat.word_start && pat.word_end);
        assert_eq!(SearchPattern::whole_word("abc"), "\\<abc\\>");
        assert!(SearchPattern::parse("\\<\\>").is_empty());
    }

    #[test]
    fn whole_word_skips_embedded_and_keeps_searching() {
        let pat = SearchPattern::parse("\\<abc\\>");
        assert_eq!(pat.find("abcdef abc_x abc", 0), Some(13));
        assert_eq!(pat.find("abcdef abc_x abc", 14), None);
        assert_eq!(pat.rfind("abc xabc abcx", 13), Some(0));
        assert_eq!(pat.rfind("xabc abcx", 9), None);
    }

    #[test]
    fn one_sided_atoms_check_one_side() {
        assert_eq!(SearchPattern::parse("\\<abc").find("xabc abcd", 0), Some(5));
        assert_eq!(SearchPattern::parse("abc\\>").find("abcd xabc", 0), Some(6));
    }

    #[test]
    fn boundary_needs_a_keyword_char_on_the_inside() {
        // `\<` never matches when the pattern itself starts with punctuation.
        assert_eq!(SearchPattern::parse("\\<-x").find("a -x", 0), None);
    }

    #[test]
    fn overlapping_occurrences_are_retried_both_directions() {
        let pat = SearchPattern::parse("\\<aa\\>");
        assert_eq!(pat.find("aaa aa", 0), Some(4));
        assert_eq!(pat.rfind("aa aaa", 6), Some(0));
    }

    #[test]
    fn multibyte_neighbors_are_word_chars() {
        let pat = SearchPattern::parse("\\<abc\\>");
        // `é` is two bytes, so the standalone word starts at byte 12.
        assert_eq!(pat.find("éabc abcé abc", 0), Some(12));
        assert_eq!(pat.rfind("abc éabc", 9), Some(0));
    }
}
