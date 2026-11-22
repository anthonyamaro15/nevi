//! Parsing with a time limit. Parsing runs on the thread that reads the
//! keyboard, and a few short inputs make some grammars take many seconds,
//! so a parse that runs out of time is abandoned and the file is drawn
//! without highlighting until a later parse fits.

use std::cell::Cell;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use tree_sitter::{ParseOptions, Parser, Point, Tree};

use super::highlighter::OverBudget;

/// Fixed part of a parse's time limit, and the time added for each byte of
/// source. On a release build ordinary files parse at 0.02 to 0.11 µs per
/// byte (784 KB of Rust in 45 ms, 1 MB of Go in 104 ms), so a limit of
/// 100 ms plus 0.5 µs per byte is about five times the slowest of them:
/// 150 ms for 100 KB, 0.6 s for 1 MB.
pub const PARSE_BASE_BUDGET: Duration = Duration::from_millis(100);
pub const PARSE_BUDGET_PER_BYTE: Duration = Duration::from_nanos(500);

/// Debug builds compile tree-sitter's C code without optimisation and parse
/// 6 to 8 times slower (`src/terminal/mod.rs`, 784 KB, takes about 310 ms),
/// which the release limits would cut off on large ordinary files.
const BUILD_SLOWDOWN: u32 = if cfg!(debug_assertions) { 10 } else { 1 };

/// The limits a running editor uses, as (base, per byte).
pub const PARSE_BUDGET: (Duration, Duration) = (
    PARSE_BASE_BUDGET.saturating_mul(BUILD_SLOWDOWN),
    PARSE_BUDGET_PER_BYTE.saturating_mul(BUILD_SLOWDOWN),
);

/// The text is handed to the parser this many bytes at a time, so the time
/// limit is checked between chunks as well as in tree-sitter's progress
/// callback. Some grammars spend all their time inside one call to the
/// lexer, which never reaches the progress callback but does ask for more
/// text. Chunk ends need not fall on character boundaries: the lexer asks
/// again for a character split across two chunks.
const READ_CHUNK: usize = 4 * 1024;

/// Time a parse of `len` bytes may take, given (base, per byte) limits.
pub(super) fn parse_budget((base, per_byte): (Duration, Duration), len: usize) -> Duration {
    base.saturating_add(per_byte.saturating_mul(u32::try_from(len).unwrap_or(u32::MAX)))
}

/// Parse `source`, giving up at `deadline`. On `OverBudget` the parser has
/// been reset and no part of the abandoned parse is returned.
pub(super) fn parse_before(
    parser: &mut Parser,
    source: &str,
    old_tree: Option<&Tree>,
    deadline: Instant,
) -> Result<Option<Tree>, OverBudget> {
    let bytes = source.as_bytes();
    let timed_out = Cell::new(false);
    let out_of_time = || {
        if !timed_out.get() && Instant::now() >= deadline {
            timed_out.set(true);
        }
        timed_out.get()
    };
    // Once time runs out, report the end of the text for every later read.
    // The lexer then finishes quickly; the partial tree it builds is thrown
    // away below.
    let mut read = |offset: usize, _: Point| {
        let end = (offset + READ_CHUNK).min(bytes.len());
        if offset < end && out_of_time() {
            return &bytes[..0];
        }
        bytes.get(offset..end).unwrap_or_default()
    };
    let mut progress = |_: &_| {
        if out_of_time() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = ParseOptions::new().progress_callback(&mut progress);
    let tree = parser.parse_with_options(&mut read, old_tree, Some(options));
    if timed_out.get() {
        // A cancelled parse would otherwise resume on the next call.
        parser.reset();
        return Err(OverBudget);
    }
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser_for(language: tree_sitter::Language) -> Parser {
        let mut parser = Parser::new();
        parser.set_language(&language).expect("grammar");
        parser
    }

    fn far_deadline() -> Instant {
        Instant::now() + Duration::from_secs(30)
    }

    #[test]
    fn chunked_parse_matches_a_whole_text_parse() {
        let rust: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let markdown: tree_sitter::Language = tree_sitter_md::LANGUAGE.into();
        // Each is many chunks long and has multi-byte characters. The second
        // splits `é` across the first chunk boundary.
        let sources = [
            (rust.clone(), include_str!("highlighter.rs").to_string()),
            (
                rust.clone(),
                format!("{}let é = \"🙂\";\n", " ".repeat(READ_CHUNK - 5)).repeat(5),
            ),
            (
                rust,
                "fn é() { let s = \"héllo ✓ 🙂\"; } // ünïcödé\n".repeat(400),
            ),
            (
                markdown,
                "# Títle ✓\n\n- ítem 🙂 *émphasis*\n\n```rust\nfn é() {}\n```\n".repeat(400),
            ),
        ];
        for (language, source) in sources {
            assert!(source.len() > 4 * READ_CHUNK);
            let mut parser = parser_for(language);
            let whole = parser.parse(&source, None).expect("whole parse");
            let chunked = parse_before(&mut parser, &source, None, far_deadline())
                .expect("fits the budget")
                .expect("chunked parse");
            assert_eq!(whole.root_node().to_sexp(), chunked.root_node().to_sexp());
            assert_eq!(
                whole.root_node().byte_range(),
                chunked.root_node().byte_range()
            );
            assert!(!chunked.root_node().has_error());
        }
    }

    #[test]
    fn progress_callback_stops_a_slow_parse() {
        // Error recovery on this Rust input reaches the progress callback
        // often. It fits in one chunk, so the read callback is never asked
        // for more text and cannot stop it; a full parse takes about 100 ms
        // in a debug build.
        let source = "a,".repeat(2_000);
        assert!(source.len() < READ_CHUNK);
        let mut parser = parser_for(tree_sitter_rust::LANGUAGE.into());
        let started = Instant::now();
        let result = parse_before(
            &mut parser,
            &source,
            None,
            started + Duration::from_millis(1),
        );
        assert_eq!(result.err(), Some(OverBudget));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn end_of_text_stops_a_parse_the_progress_callback_never_sees() {
        // The Swift lexer spends this whole parse inside one call, so only
        // the read callback gets a chance to stop it.
        let source = "#".repeat(20_000);
        let mut parser = parser_for(tree_sitter_swift::LANGUAGE.into());
        let started = Instant::now();
        let result = parse_before(
            &mut parser,
            &source,
            None,
            started + Duration::from_millis(10),
        );
        assert_eq!(result.err(), Some(OverBudget));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn parser_is_usable_after_a_timeout() {
        let mut parser = parser_for(tree_sitter_rust::LANGUAGE.into());
        let slow = "a,".repeat(2_000);
        assert!(parse_before(&mut parser, &slow, None, Instant::now()).is_err());

        let source = "fn main() {}\n";
        let tree = parse_before(&mut parser, source, None, far_deadline())
            .expect("fits the budget")
            .expect("tree");
        assert_eq!(tree.root_node().byte_range(), 0..source.len());
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn budget_grows_with_the_source() {
        let real = (PARSE_BASE_BUDGET, PARSE_BUDGET_PER_BYTE);
        assert_eq!(parse_budget(real, 0), Duration::from_millis(100));
        assert_eq!(parse_budget(real, 100_000), Duration::from_millis(150));
        assert_eq!(parse_budget(real, 1_000_000), Duration::from_millis(600));
    }

    mod syntax_manager {
        use std::path::Path;

        use super::*;
        use crate::editor::Buffer;
        use crate::syntax::{SyntaxManager, assert_tree_matches_fresh_parse};

        const NO_TIME: (Duration, Duration) = (Duration::ZERO, Duration::ZERO);
        const NO_LIMIT: (Duration, Duration) = (Duration::from_secs(30), Duration::ZERO);

        fn parsed(path: &str, text: &str) -> (SyntaxManager, Buffer) {
            let mut syntax = SyntaxManager::new();
            syntax.set_language_from_path(Path::new(path));
            let mut buffer = Buffer::new();
            buffer.set_content(text);
            syntax.parse(&mut buffer);
            (syntax, buffer)
        }

        /// The state `skip_parse` leaves: nothing from the old tree is kept.
        fn assert_plain(syntax: &SyntaxManager, buffer: &mut Buffer) {
            assert!(syntax.tree.is_none());
            assert!(!syntax.has_highlighting());
            assert!(syntax.get_line_highlights(0).is_empty());
            assert!(syntax.incremental_identity.is_none());
            assert!(syntax.source_cache.is_empty());
            assert!(syntax.line_start_bytes.is_empty());
            assert!(syntax.highlight_cache.borrow().is_empty());
            assert_eq!(syntax.cache_version.get(), 0);
            assert_eq!(syntax.parse_version, buffer.version());
            assert!(syntax.method_boundaries.borrow().is_none());
            assert!(
                buffer
                    .take_edits_since(syntax.parse_version)
                    .is_some_and(|edits| edits.is_empty())
            );
        }

        #[test]
        fn normal_file_keeps_its_tree_within_the_real_budget() {
            let mut syntax = SyntaxManager::new();
            syntax.set_language_from_path(Path::new("normal.rs"));
            syntax.set_parse_budget(PARSE_BUDGET);
            let mut buffer = Buffer::new();
            buffer.set_content(include_str!("highlighter.rs"));
            syntax.parse(&mut buffer);
            assert!(syntax.has_highlighting());

            buffer.insert_char(0, 0, ' ');
            syntax.parse(&mut buffer);
            assert!(syntax.has_highlighting());
            assert!(!syntax.take_slow_highlight_notice());
        }

        #[test]
        fn timed_out_incremental_parse_leaves_no_tree_and_the_next_parse_is_right() {
            let source = "fn main() {\n    let s = \"héllo\";\n}\nfn other() {}\n";
            let (mut syntax, mut buffer) = parsed("slow.rs", source);
            // Fill the method boundary cache, which must not outlive the tree.
            let next_fn = crate::method_motion::MethodBoundary::NextStart;
            assert!(syntax.method_motion_target(0, next_fn, 1).is_some());

            // The tree is clean and belongs to this buffer, so this parse
            // reuses it; time runs out at the first read.
            buffer.insert_str(1, 4, "let y = 2; ");
            syntax.set_parse_budget(NO_TIME);
            syntax.parse(&mut buffer);
            assert_plain(&syntax, &mut buffer);

            buffer.insert_char(2, 0, '\n');
            syntax.set_parse_budget(NO_LIMIT);
            syntax.parse(&mut buffer);
            assert_tree_matches_fresh_parse(&syntax, &buffer);

            // Incremental parsing works again after that.
            buffer.insert_str(0, 0, "// é\n");
            syntax.parse(&mut buffer);
            assert_tree_matches_fresh_parse(&syntax, &buffer);
        }

        #[test]
        fn colours_come_back_when_an_edit_makes_the_file_quick_to_parse() {
            let slow = "\"".repeat(200_000);
            let mut syntax = SyntaxManager::new();
            syntax.set_language_from_path(Path::new("slow.rs"));
            syntax.set_parse_budget((Duration::from_millis(20), Duration::ZERO));
            let mut buffer = Buffer::new();
            buffer.set_content(&slow);
            syntax.parse(&mut buffer);
            assert_plain(&syntax, &mut buffer);

            buffer.set_content("fn main() { let x = 1; }\n");
            syntax.parse(&mut buffer);
            assert!(!syntax.get_line_highlights(0).is_empty());
            assert_tree_matches_fresh_parse(&syntax, &buffer);
        }

        #[test]
        fn slow_parse_notice_is_given_once_per_buffer() {
            let (mut syntax, mut buffer) = parsed("slow.rs", "fn main() {}\n");
            syntax.set_parse_budget(NO_TIME);
            assert!(!syntax.take_slow_highlight_notice());

            buffer.insert_char(0, 0, ' ');
            syntax.parse(&mut buffer);
            assert!(syntax.take_slow_highlight_notice());
            assert!(!syntax.take_slow_highlight_notice());

            // Still slow after an edit: no second notice.
            buffer.insert_char(0, 0, ' ');
            syntax.parse(&mut buffer);
            assert!(!syntax.take_slow_highlight_notice());

            // Quick to parse now but slow to highlight: still the same
            // buffer, so still no second notice.
            syntax.set_parse_budget(NO_LIMIT);
            buffer.insert_char(0, 0, ' ');
            syntax.parse(&mut buffer);
            syntax.set_highlight_budgets(Duration::ZERO, Duration::ZERO);
            syntax.get_line_highlights(0);
            assert!(syntax.highlight_budget_hit.get());
            assert!(!syntax.take_slow_highlight_notice());

            // Another slow buffer gets its own notice.
            syntax.set_parse_budget(NO_TIME);
            let mut other = Buffer::new();
            other.set_content("fn other() {}\n");
            syntax.parse(&mut other);
            assert!(syntax.take_slow_highlight_notice());
        }

        #[test]
        fn preview_text_over_the_parse_budget_is_plain_and_gives_no_notice() {
            let mut syntax = SyntaxManager::new();
            syntax.set_language_from_path(Path::new("preview.rs"));
            syntax.parse_string("fn main() {}\n");
            assert!(syntax.has_highlighting());

            syntax.set_parse_budget(NO_TIME);
            syntax.parse_string("fn other() {}\n");
            assert!(syntax.tree.is_none());
            assert!(syntax.source_cache.is_empty());
            assert!(syntax.highlight_cache.borrow().is_empty());
            assert!(syntax.get_line_highlights(0).is_empty());
            assert!(!syntax.take_slow_highlight_notice());
        }
    }
}
