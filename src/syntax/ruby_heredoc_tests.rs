//! Long Ruby heredoc terminators.
//!
//! tree-sitter-ruby 0.23.1 saved the terminator length in one byte and
//! undercounted its state header, so a terminator of 256 bytes or more
//! (and exactly 1,019 bytes through a separate overflow) hits a C assert.
//! With that crate these cases don't fail like a normal test: the whole test
//! binary dies with SIGABRT. If the suite suddenly aborts, check the
//! tree-sitter-ruby pin in Cargo.toml first.

use super::*;
use std::path::Path;

// 256 is the first length that wrapped the one-byte length, 1,018 the last
// before the buffer overflow, 1,019 the one that overflowed it and 1,020 the
// first that 0.23.1 already rejected cleanly.
const TERMINATOR_LENGTHS: [usize; 4] = [256, 1018, 1019, 1020];

fn ruby_syntax() -> SyntaxManager {
    let mut syntax = SyntaxManager::new();
    syntax.set_language_from_path(Path::new("long_heredoc.rb"));
    syntax
}

/// An opening line alone, and a closed heredoc with a body line.
fn long_heredoc_sources(len: usize) -> [String; 2] {
    let word = "A".repeat(len);
    [
        format!("x = <<~{word}\n"),
        format!("x = <<~{word}\n  body\n{word}\n"),
    ]
}

fn assert_tree_and_highlights(syntax: &SyntaxManager, len: usize) {
    assert!(
        syntax.get_tree_and_source().is_some(),
        "no tree for a {len}-byte heredoc terminator"
    );
    assert!(
        !syntax.get_line_highlights(0).is_empty(),
        "no highlights for a {len}-byte heredoc terminator"
    );
}

#[test]
fn long_heredoc_terminator_parses_from_a_buffer() {
    for len in TERMINATOR_LENGTHS {
        for source in long_heredoc_sources(len) {
            let mut syntax = ruby_syntax();
            let mut buffer = Buffer::new();
            buffer.set_content(&source);
            syntax.parse(&mut buffer);
            assert_tree_and_highlights(&syntax, len);
        }
    }
}

#[test]
fn long_heredoc_terminator_parses_from_a_string() {
    for len in TERMINATOR_LENGTHS {
        for source in long_heredoc_sources(len) {
            let mut syntax = ruby_syntax();
            syntax.parse_string(&source);
            assert_tree_and_highlights(&syntax, len);
        }
    }
}

#[test]
fn ordinary_heredoc_highlights_as_a_string() {
    let mut syntax = ruby_syntax();
    let mut buffer = Buffer::new();
    buffer.set_content("text = <<~EOS\n  hello\nEOS\nputs text\n");
    syntax.parse(&mut buffer);

    let string = syntax
        .theme
        .get_color_for_capture("string")
        .expect("string color");
    let string_spans = |line: usize| {
        syntax
            .get_line_highlights(line)
            .into_iter()
            .filter(|span| span.fg == string)
            .map(|span| (span.start_col, span.end_col))
            .collect::<Vec<_>>()
    };

    assert_eq!(string_spans(0), vec![(7, 13)], "<<~EOS opener");
    assert_eq!(string_spans(1), vec![(0, 7)], "heredoc body");
    assert_eq!(string_spans(2), vec![(0, 3)], "EOS terminator");
    assert!(string_spans(3).is_empty(), "code after the heredoc");
    assert!(!syntax.get_line_highlights(3).is_empty());
}
