//! Finder drawing. Includes the preview speed checks from issue #355: the
//! frame-budget guard CI runs for a grep preview of a minified one-line file,
//! and the bench that times loading the preview for a hit deep in a large
//! file.
//!
//! The bench is a measuring tool, not a pass/fail gate, so CI skips it (see
//! `opt_in_gates`). It only runs when asked to, in release:
//!
//! ```text
//! NEVI_PERF_BENCH=1 cargo test --release --lib finder_preview_bench -- --ignored --nocapture
//! ```
//!
//! The first run writes a 72 MB file to `~/.cache/nevi-perf/` and keeps it,
//! for the same reason the live grep bench keeps its repo: antivirus scanners
//! inspect freshly written files and slow down the first reads.

use super::{
    ReplayDimensions, SharedOutput, assert_render_frame_budget, background_sequence,
    minified_js_document, render_editor_to_string,
};
use crate::editor::{Editor, Mode};
use crate::finder::{FinderItem, FloatingWindow};
use crate::perf::print_perf_row;
use crate::terminal::Terminal;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::{Config as AlacrittyConfig, Term as AlacrittyTerm};
use alacritty_terminal::vte::ansi::Processor as VteProcessor;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Lines in the large file, and the line its one hit is on.
const LARGE_FILE_LINES: usize = 1_000_000;
const DEEP_HIT_LINE: usize = 999_990;
const DEEP_HIT_QUERY: &str = "account";
const SAMPLES: usize = 5;

/// The grep picker with one result, `path:line`, selected and previewed.
fn grep_preview_editor(path: &Path, line: usize, query: &str) -> Editor {
    let mut editor = Editor::default();
    editor.set_size(120, 40);
    editor
        .finder
        .open_grep(path.parent().unwrap_or(Path::new(".")));
    editor.finder.query = query.to_string();
    editor.finder.items = vec![
        FinderItem::new(format!("{}:{line}: hit", path.display()), path.into()).with_line(line),
    ];
    editor.finder.filtered = vec![0];
    editor.finder.preview_enabled = true;
    editor.mode = Mode::Finder;
    editor
}

#[test]
#[ignore = "CI render frame-budget guard; run explicitly with cargo test render_frame_budget -- --ignored --nocapture"]
fn render_frame_budget_grep_preview_of_minified_one_line_file_stays_bounded() {
    // The same 1.6 MB line as the editor's minified-file guard, shown in the
    // grep preview with matches in view.
    let path = PathBuf::from("data.min.js");
    let mut editor = grep_preview_editor(&path, 1, "render_target");
    let line = minified_js_document(25_000).trim_end().to_string();
    editor.finder.set_preview_content(path, vec![line]);

    assert_render_frame_budget(
        "grep preview of a minified one-line file",
        &editor,
        5,
        Duration::from_millis(100),
    );
}

/// What a 120x40 terminal shows after one full frame.
fn screen_after_frame(editor: &Editor) -> AlacrittyTerm<VoidListener> {
    crossterm::style::force_color_output(true);
    let output = SharedOutput::default();
    let mut terminal = Terminal::new_for_test(Box::new(output.clone()));
    terminal.render(editor).expect("render should succeed");
    let dimensions = ReplayDimensions {
        rows: 40,
        cols: 120,
    };
    let mut screen = AlacrittyTerm::new(AlacrittyConfig::default(), &dimensions, VoidListener);
    let mut processor: VteProcessor = VteProcessor::new();
    processor.advance(&mut screen, output.into_string().as_bytes());
    screen
}

fn screen_row(screen: &AlacrittyTerm<VoidListener>, row: u16) -> String {
    (0..120)
        .map(|col| screen.grid()[Line(row as i32)][Column(col)].c)
        .collect()
}

#[test]
fn finder_preview_draws_tabs_and_control_characters_like_the_editor() {
    let path = PathBuf::from("notes.txt");
    let mut editor = grep_preview_editor(&path, 1, "");
    editor
        .finder
        .set_preview_content(path, vec!["a\tb\u{1b}c\u{7}d".to_string()]);
    let win = FloatingWindow::centered_with_preview(120, 40, true);

    let row = screen_row(&screen_after_frame(&editor), win.y + 1);
    assert!(row.contains("a    b c d"), "row={row:?}");
}

#[test]
fn finder_preview_keeps_wide_characters_inside_the_pane() {
    let path = PathBuf::from("zh-Hans.strings");
    let mut editor = grep_preview_editor(&path, 1, "");
    editor
        .finder
        .set_preview_content(path, vec!["中文".repeat(100)]);
    let win = FloatingWindow::centered_with_preview(120, 40, true);

    let screen = screen_after_frame(&editor);
    let right_border =
        screen.grid()[Line((win.y + 1) as i32)][Column((win.x + win.width - 1) as usize)].c;
    assert_eq!(
        right_border,
        '│',
        "row={:?}",
        screen_row(&screen, win.y + 1)
    );
}

#[test]
fn finder_preview_highlights_a_match_after_a_tab() {
    let path = PathBuf::from("main.rs");
    let mut editor = grep_preview_editor(&path, 1, "needle");
    editor
        .finder
        .set_preview_content(path, vec!["\tneedle".to_string()]);
    // The tab draws as four plain spaces and the highlight starts right
    // after them, on the match.
    let highlight_after_tab = format!(
        "    {}",
        background_sequence(editor.theme().ui.search_match_bg)
    );

    let rendered = render_editor_to_string(&editor);
    assert!(
        rendered.contains(&highlight_after_tab),
        "rendered={rendered:?}"
    );
    assert!(!rendered.contains('\t'), "rendered={rendered:?}");
}

#[test]
fn finder_preview_title_draws_control_characters_as_blanks() {
    let editor = grep_preview_editor(Path::new("x\u{1b}c.txt"), 1, "");
    let win = FloatingWindow::centered_with_preview(120, 40, true);

    let header = screen_row(&screen_after_frame(&editor), win.y);
    assert!(header.contains("x c.txt"), "header={header:?}");
}

#[test]
fn finder_preview_title_keeps_wide_names_inside_the_border() {
    let editor = grep_preview_editor(Path::new("中文.txt"), 1, "");
    let win = FloatingWindow::centered_with_preview(120, 40, true);

    let screen = screen_after_frame(&editor);
    let corner = screen.grid()[Line(win.y as i32)][Column((win.x + win.width - 1) as usize)].c;
    assert_eq!(
        corner.to_string(),
        editor.ui_glyphs().corner_tr,
        "header={:?}",
        screen_row(&screen, win.y)
    );
}

#[test]
fn finder_preview_keeps_the_border_in_place_for_long_line_numbers() {
    // A hit deep in a big file shows six and seven digit line numbers.
    let path = PathBuf::from("big.txt");
    let mut editor = grep_preview_editor(&path, 1_000_000, "");
    let lines = (0..20).map(|i| format!("line {i}")).collect();
    editor.finder.set_preview_content(path, lines);
    editor.finder.preview_line_offset = 999_989;
    let win = FloatingWindow::centered_with_preview(120, 40, true);

    let screen = screen_after_frame(&editor);
    let right = Column((win.x + win.width - 1) as usize);
    for row in win.y + 1..win.y + 21 {
        assert_eq!(
            screen.grid()[Line(row as i32)][right].c,
            '│',
            "row={:?}",
            screen_row(&screen, row)
        );
    }
}

#[test]
fn finder_bottom_border_ends_at_the_corner_with_the_preview_on() {
    let editor = grep_preview_editor(Path::new("notes.txt"), 1, "");
    let win = FloatingWindow::centered_with_preview(120, 40, true);

    let screen = screen_after_frame(&editor);
    let bottom = win.y + win.height - 1;
    let corner = screen.grid()[Line(bottom as i32)][Column((win.x + win.width - 1) as usize)].c;
    assert_eq!(
        corner.to_string(),
        editor.ui_glyphs().corner_br,
        "bottom={:?}",
        screen_row(&screen, bottom)
    );
}

#[test]
fn finder_rows_draw_control_characters_as_blanks() {
    let path = PathBuf::from("notes.txt");
    let mut editor = grep_preview_editor(&path, 1, "");
    editor.finder.items[0].display = "notes.txt:1: a\tb\u{1b}c".to_string();
    editor.finder.preview_enabled = false;

    // Results are drawn bottom-up, so look on every row.
    let screen = screen_after_frame(&editor);
    assert!(
        (0..40).any(|row| screen_row(&screen, row).contains("notes.txt:1: a b c")),
        "no row shows the result as plain text"
    );
}

/// A log-like file with one hit, near the end.
fn large_log() -> String {
    let mut text = String::with_capacity(LARGE_FILE_LINES * 72);
    for line in 1..=LARGE_FILE_LINES {
        let event = if line == DEEP_HIT_LINE {
            "account lookup failed"
        } else {
            "request finished in 12ms"
        };
        let _ = writeln!(
            text,
            "2026-10-05 12:{:02}:{:02}.{:03} INFO worker-{:02} {line:07} {event}",
            (line / 60_000) % 60,
            (line / 1_000) % 60,
            line % 1_000,
            line % 16,
        );
    }
    text
}

/// The large file, written on first use.
fn large_file() -> PathBuf {
    // Bump the version whenever large_log changes.
    let dir = dirs::home_dir()
        .expect("home directory")
        .join(".cache/nevi-perf/preview-v1");
    let path = dir.join("large.log");
    let done = dir.with_extension("done");
    if !done.exists() {
        fs::create_dir_all(&dir).expect("create bench dir");
        fs::write(&path, large_log()).expect("write bench file");
        fs::write(&done, "").expect("write marker");
        println!("wrote {}", path.display());
    }
    path
}

/// Selecting a result loads its preview again.
fn load(editor: &mut Editor) -> Duration {
    editor.finder.clear_preview_cache();
    let started = Instant::now();
    editor.update_finder_preview();
    started.elapsed()
}

fn median_load(editor: &mut Editor) -> Duration {
    // The first reads also warm the file up.
    for _ in 0..3 {
        load(editor);
    }
    let mut samples: Vec<Duration> = (0..SAMPLES).map(|_| load(editor)).collect();
    samples.sort();
    samples[SAMPLES / 2]
}

#[test]
#[ignore = "measuring tool for PERF.md: NEVI_PERF_BENCH=1, release, --nocapture"]
fn finder_preview_bench() {
    if std::env::var_os("NEVI_PERF_BENCH").is_none() {
        println!("finder_preview_bench skipped: set NEVI_PERF_BENCH=1 to run it");
        return;
    }
    let path = large_file();

    let mut deep = grep_preview_editor(&path, DEEP_HIT_LINE, DEEP_HIT_QUERY);
    let deep_load = median_load(&mut deep);
    assert!(
        deep.finder
            .preview_content
            .iter()
            .any(|line| line.contains(DEEP_HIT_QUERY)),
        "the preview should show the deep hit"
    );
    let mut shallow = grep_preview_editor(&path, 10, DEEP_HIT_QUERY);
    let shallow_load = median_load(&mut shallow);

    println!(
        "load preview: hit on line {DEEP_HIT_LINE} of {LARGE_FILE_LINES} {:.3}ms, hit on line 10 {:.3}ms",
        deep_load.as_secs_f64() * 1000.0,
        shallow_load.as_secs_f64() * 1000.0,
    );
    print_perf_row("Finder preview: load a hit deep in a large file", deep_load);
}
