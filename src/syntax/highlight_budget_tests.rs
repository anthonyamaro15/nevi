//! A line whose highlight query runs out of time, or a frame whose queries
//! do, turns highlighting off for the rest of that parse instead of stalling
//! every draw.

use std::path::Path;
use std::time::{Duration, Instant};

use super::{HIGHLIGHT_FRAME_BUDGET, HIGHLIGHT_QUERY_BUDGET, SyntaxManager};
use crate::editor::Buffer;

/// A Rust file whose first line has enough nodes for tree-sitter to reach
/// its first time check (every 100 query steps), followed by short lines.
fn rust_source() -> String {
    let long_line = format!("fn long() {{ let x = {}0; }}", "1 + ".repeat(400));
    format!("{long_line}\nfn main() {{}}\nlet y = \"text\";\nfn other() {{}}\n")
}

fn parsed_rust(text: &str) -> (SyntaxManager, Buffer) {
    let mut syntax = SyntaxManager::new();
    syntax.set_language_from_path(Path::new("budget.rs"));
    let mut buffer = Buffer::new();
    buffer.set_content(text);
    syntax.parse(&mut buffer);
    (syntax, buffer)
}

fn use_real_budgets(syntax: &mut SyntaxManager) {
    syntax.set_highlight_budgets(HIGHLIGHT_QUERY_BUDGET, HIGHLIGHT_FRAME_BUDGET);
}

/// Highlights `lines` the way the screen does, one frame per `rows` lines.
fn draw_in_frames(syntax: &SyntaxManager, lines: usize, rows: usize) -> usize {
    let mut highlighted = 0;
    for line in 0..lines {
        if line % rows == 0 {
            syntax.begin_highlight_frame();
        }
        if !syntax.get_line_highlights(line).is_empty() {
            highlighted += 1;
        }
    }
    highlighted
}

#[test]
fn slow_highlight_query_stops_at_budget() {
    let (mut syntax, _buffer) = parsed_rust(&rust_source());
    syntax.set_highlight_budgets(Duration::ZERO, HIGHLIGHT_FRAME_BUDGET);

    assert!(syntax.get_line_highlights(0).is_empty());
    assert!(syntax.highlight_budget_hit.get());
}

#[test]
fn lines_after_a_budget_hit_skip_the_query_until_the_next_parse() {
    let (mut syntax, mut buffer) = parsed_rust(&rust_source());
    assert!(!syntax.get_line_highlights(1).is_empty());

    syntax.set_highlight_budgets(Duration::ZERO, HIGHLIGHT_FRAME_BUDGET);
    assert!(syntax.get_line_highlights(0).is_empty());

    // With the budget back to normal a query would find spans, so empty
    // results show the query did not run. Line 1 was coloured before the
    // trip and is plain now, so the whole file looks the same.
    use_real_budgets(&mut syntax);
    syntax.begin_highlight_frame();
    assert!(syntax.get_line_highlights(1).is_empty());
    assert!(syntax.get_line_highlights(2).is_empty());
    assert!(syntax.get_line_highlights(0).is_empty());

    buffer.insert_char(1, 0, ' ');
    syntax.parse(&mut buffer);
    assert!(!syntax.highlight_budget_hit.get());
    assert!(!syntax.get_line_highlights(0).is_empty());
    assert!(!syntax.get_line_highlights(1).is_empty());
}

#[test]
fn lines_that_each_fit_but_together_pass_the_frame_budget_trip_it() {
    let (mut syntax, _buffer) = parsed_rust(&rust_source());
    syntax.begin_highlight_frame();
    assert!(!syntax.get_line_highlights(1).is_empty());

    // Leave this frame no time beyond what line 1 already used, so the next
    // line pushes the frame total over even though it fits its own budget.
    let spent = syntax.frame_highlight_spent.get();
    syntax.set_highlight_budgets(Duration::from_secs(30), spent + Duration::from_nanos(1));
    assert!(syntax.get_line_highlights(2).is_empty());
    assert!(syntax.highlight_budget_hit.get());

    syntax.set_highlight_budgets(Duration::from_secs(30), Duration::from_secs(30));
    syntax.begin_highlight_frame();
    assert!(syntax.get_line_highlights(3).is_empty());
    assert!(syntax.get_line_highlights(1).is_empty());
    assert!(syntax.take_slow_highlight_notice());
}

#[test]
fn each_frame_gets_a_fresh_highlight_budget() {
    let text = "fn main() { let x = 1; }\n".repeat(50);
    let (mut syntax, mut buffer) = parsed_rust(&text);
    let frame_budget = Duration::from_secs(1);
    syntax.set_highlight_budgets(HIGHLIGHT_QUERY_BUDGET, frame_budget);

    // Every frame ends with its whole budget used; the next frame must not
    // inherit that, or a file would lose colours while scrolling.
    for line in 0..50 {
        syntax.frame_highlight_spent.set(frame_budget);
        syntax.begin_highlight_frame();
        assert!(!syntax.get_line_highlights(line).is_empty(), "line {line}");
    }
    assert!(!syntax.highlight_budget_hit.get());

    // A parse starts a fresh count too.
    syntax.frame_highlight_spent.set(frame_budget);
    buffer.insert_char(0, 0, ' ');
    syntax.parse(&mut buffer);
    assert!(!syntax.get_line_highlights(0).is_empty());
}

#[test]
fn normal_file_keeps_highlights_within_the_default_budget() {
    let text = include_str!("theme.rs");
    let (mut syntax, _buffer) = parsed_rust(text);
    use_real_budgets(&mut syntax);

    let lines = text.lines().count();
    let highlighted = draw_in_frames(&syntax, lines, 30);
    assert!(highlighted > lines / 2, "{highlighted}");
    assert!(!syntax.highlight_budget_hit.get());
    assert!(!syntax.take_slow_highlight_notice());
}

#[test]
fn slow_highlight_notice_is_given_once_per_buffer() {
    let (mut syntax, mut buffer) = parsed_rust(&rust_source());
    syntax.set_highlight_budgets(Duration::ZERO, HIGHLIGHT_FRAME_BUDGET);
    assert!(!syntax.take_slow_highlight_notice());

    syntax.get_line_highlights(0);
    assert!(syntax.take_slow_highlight_notice());
    assert!(!syntax.take_slow_highlight_notice());

    // Still slow after an edit: the budget trips again, the notice does not.
    buffer.insert_char(1, 0, ' ');
    syntax.parse(&mut buffer);
    syntax.get_line_highlights(0);
    assert!(syntax.highlight_budget_hit.get());
    assert!(!syntax.take_slow_highlight_notice());

    // Another slow buffer gets its own notice.
    let mut other = Buffer::new();
    other.set_content(&rust_source());
    syntax.parse(&mut other);
    syntax.get_line_highlights(0);
    assert!(syntax.take_slow_highlight_notice());
}

#[test]
fn preview_text_over_budget_gives_no_notice() {
    let mut syntax = SyntaxManager::new();
    syntax.set_language_from_path(Path::new("preview.rs"));
    syntax.set_highlight_budgets(Duration::ZERO, HIGHLIGHT_FRAME_BUDGET);
    syntax.parse_string(&rust_source());

    assert!(syntax.get_line_highlights(0).is_empty());
    assert!(syntax.highlight_budget_hit.get());
    assert!(!syntax.take_slow_highlight_notice());
}

/// Unclosed brackets nest the tree deeper with every character, which makes
/// each line's query slow even though it finds nothing.
fn unclosed_brackets(lines: usize, chars: usize) -> String {
    let line: String = "([{".chars().cycle().take(chars).collect();
    format!("{}\n", vec![line.as_str(); lines].join("\n"))
}

fn first_frame_time(text: &str) -> (Duration, bool) {
    let (mut syntax, _buffer) = parsed_rust(text);
    use_real_budgets(&mut syntax);
    let started = Instant::now();
    draw_in_frames(&syntax, 30, 30);
    (started.elapsed(), syntax.highlight_budget_hit.get())
}

#[test]
fn unclosed_brackets_screen_highlights_in_bounded_time() {
    let (elapsed, tripped) = first_frame_time(&unclosed_brackets(30, 3_000));
    assert!(tripped);
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
}

#[test]
fn short_slow_lines_add_up_to_a_bounded_frame() {
    // A line budget that never runs out leaves only the frame budget to stop
    // the screen. Each line here costs tens of milliseconds, and the 60 lines
    // took over 4 s together before the frame budget existed.
    let (mut syntax, _buffer) = parsed_rust(&unclosed_brackets(60, 150));
    syntax.set_highlight_budgets(Duration::from_secs(30), HIGHLIGHT_FRAME_BUDGET);
    let started = Instant::now();
    draw_in_frames(&syntax, 60, 60);
    let elapsed = started.elapsed();
    assert!(syntax.highlight_budget_hit.get());
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
}
