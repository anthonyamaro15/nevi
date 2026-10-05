//! PERF.md rows for editing big files: the reparse after an edit in an
//! 18,060-line Rust file, and `gg=G` on a 2,100-line one with scrambled
//! indentation. Both files come from one deterministic generator, so every
//! version is measured on identical input. Like the finder benches, it only
//! runs with NEVI_PERF_BENCH=1, in release:
//!
//! ```text
//! NEVI_PERF_BENCH=1 cargo test --release --lib large_file_edit_bench -- --ignored --nocapture
//! ```

use crate::editor::Editor;
use crate::perf::print_perf_row;
use crate::terminal::handle_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fs;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Valid Rust with its indentation scrambled, 21 lines per function.
fn messy_rust(functions: usize) -> String {
    let mut text = String::new();
    for i in 0..functions {
        text.push_str(&format!("fn block_{i}(input: usize) -> usize {{\n"));
        text.push_str("      let mut total = input;\n");
        text.push_str(" if total > 10 {\n");
        text.push_str(&format!("          total += {i};\n"));
        text.push_str("    } else {\n");
        text.push_str("  total = total.saturating_sub(1);\n");
        text.push_str("     }\n");
        text.push_str("   for idx in 0..total {\n");
        text.push_str("if idx % 2 == 0 {\n");
        text.push_str("        total += idx;\n");
        text.push_str("           } else {\n");
        text.push_str("   total = total.saturating_sub(idx);\n");
        text.push_str("  }\n");
        text.push_str("      }\n");
        text.push_str(" match total {\n");
        text.push_str("       0 => println!(\"zero\"),\n");
        text.push_str("  n if n > 100 => println!(\"big {}\", n),\n");
        text.push_str("          _ => {}\n");
        text.push_str("   }\n");
        text.push_str("total\n");
        text.push_str("}\n");
    }
    text
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn open_parsed(path: &Path) -> Editor {
    let mut editor = Editor::default();
    editor.set_size(120, 40);
    editor
        .open_file(path.to_path_buf())
        .expect("open the generated file");
    editor.maybe_update_syntax();
    editor
}

#[test]
#[ignore = "measuring tool for PERF.md: NEVI_PERF_BENCH=1, release, --nocapture"]
fn large_file_edit_bench() {
    if std::env::var_os("NEVI_PERF_BENCH").is_none() {
        println!("large_file_edit_bench skipped: set NEVI_PERF_BENCH=1 to run it");
        return;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "nevi_large_file_edit_{}_{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();

    // Type one character into an identifier, which keeps the file valid, then
    // reparse the way the main loop does once its debounce expires.
    let big = dir.join("big.rs");
    fs::write(&big, messy_rust(860)).unwrap();
    let mut editor = open_parsed(&big);
    let mut reparses = Vec::new();
    for i in 0..21 {
        // Inside `total` on a `let mut total = input;` line, spread over the file.
        editor.cursor.line = (i * 41 % 860) * 21 + 1;
        editor.cursor.col = 16;
        handle_key(&mut editor, key(KeyCode::Char('i'), KeyModifiers::NONE));
        handle_key(&mut editor, key(KeyCode::Char('x'), KeyModifiers::NONE));
        handle_key(&mut editor, key(KeyCode::Esc, KeyModifiers::NONE));
        let started = Instant::now();
        editor.maybe_update_syntax();
        reparses.push(started.elapsed());
    }
    reparses.sort();
    println!(
        "large file edit bench: {} lines, 21 edits",
        editor.buffer().len_lines()
    );
    print_perf_row(
        "Editing: reparse after an edit, large Rust file",
        reparses[reparses.len() / 2],
    );

    // Re-indent a whole file of scrambled indentation in one command.
    let messy = dir.join("messy.rs");
    fs::write(&messy, messy_rust(100)).unwrap();
    let mut reindents = Vec::new();
    for _ in 0..3 {
        let mut editor = open_parsed(&messy);
        let started = Instant::now();
        handle_key(&mut editor, key(KeyCode::Char('g'), KeyModifiers::NONE));
        handle_key(&mut editor, key(KeyCode::Char('g'), KeyModifiers::NONE));
        handle_key(&mut editor, key(KeyCode::Char('='), KeyModifiers::NONE));
        handle_key(&mut editor, key(KeyCode::Char('G'), KeyModifiers::SHIFT));
        reindents.push(started.elapsed());
        // Timing a no-op would be meaningless: the scrambled line must move.
        let second = editor.buffer().line(1).expect("second line").to_string();
        assert_ne!(second, "      let mut total = input;\n");
    }
    reindents.sort();
    println!(
        "large file edit bench: gg=G on {} lines",
        messy_rust(100).lines().count()
    );
    print_perf_row(
        "Editing: gg=G, file with scrambled indentation",
        reindents[reindents.len() / 2],
    );

    let _ = fs::remove_dir_all(dir);
}
