//! Seeded random-key fuzz: drives the headless editor through random key
//! sequences and asserts it never panics and core cursor/viewport invariants
//! hold afterwards. Complements the vim oracle — the oracle proves curated
//! sequences are *correct*, this proves arbitrary sequences don't *crash*.
//!
//! Sequences are fully deterministic per seed (xorshift, no rand crate), so
//! a failure message names the exact seed and key sequence for replay.
//!
//! Deliberately excluded keys: `:` (ex commands can write or open files on
//! disk), `Z` (`ZZ` writes), `<Space>` leader (pickers/floating terminal),
//! and any Ctrl chord not listed (e.g. Ctrl-s saves). The fixed buffer texts
//! must never contain URLs or file paths, since `gx`/`gf` act on them.

use crate::editor::Editor;
use crate::terminal::handle_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// xorshift64* — deterministic across platforms.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Avoid the all-zero state, which xorshift never leaves.
        Self(seed.wrapping_add(0x9E37_79B9_7F4A_7C15))
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() % items.len() as u64) as usize]
    }
}

#[derive(Clone, Copy)]
struct FuzzKey {
    code: KeyCode,
    modifiers: KeyModifiers,
    desc: &'static str,
}

const fn plain(ch: char, desc: &'static str) -> FuzzKey {
    FuzzKey {
        code: KeyCode::Char(ch),
        modifiers: KeyModifiers::NONE,
        desc,
    }
}

const fn shifted(ch: char, desc: &'static str) -> FuzzKey {
    FuzzKey {
        code: KeyCode::Char(ch),
        modifiers: KeyModifiers::SHIFT,
        desc,
    }
}

const fn ctrl(ch: char, desc: &'static str) -> FuzzKey {
    FuzzKey {
        code: KeyCode::Char(ch),
        modifiers: KeyModifiers::CONTROL,
        desc,
    }
}

/// The key alphabet: vim editing keys plus insert-mode text. Escape appears
/// several times so random walks reliably return to normal mode.
const ALPHABET: &[FuzzKey] = &[
    // Motions
    plain('h', "h"),
    plain('j', "j"),
    plain('k', "k"),
    plain('l', "l"),
    plain('w', "w"),
    plain('b', "b"),
    plain('e', "e"),
    shifted('W', "W"),
    shifted('B', "B"),
    shifted('E', "E"),
    plain('0', "0"),
    plain('^', "^"),
    plain('$', "$"),
    plain('{', "{"),
    plain('}', "}"),
    plain('(', "("),
    plain(')', ")"),
    plain('[', "["),
    plain(']', "]"),
    plain('%', "%"),
    plain('|', "|"),
    plain('+', "+"),
    plain('-', "-"),
    plain('_', "_"),
    plain('g', "g"),
    shifted('G', "G"),
    shifted('H', "H"),
    shifted('M', "M"),
    shifted('L', "L"),
    plain('f', "f"),
    plain('t', "t"),
    shifted('F', "F"),
    shifted('T', "T"),
    plain(';', ";"),
    plain(',', ","),
    plain('n', "n"),
    shifted('N', "N"),
    plain('*', "*"),
    plain('#', "#"),
    // Counts
    plain('1', "1"),
    plain('2', "2"),
    plain('3', "3"),
    plain('9', "9"),
    // Operators and edits
    plain('d', "d"),
    plain('c', "c"),
    plain('y', "y"),
    plain('p', "p"),
    shifted('P', "P"),
    plain('x', "x"),
    plain('s', "s"),
    plain('r', "r"),
    plain('~', "~"),
    shifted('J', "J"),
    plain('u', "u"),
    plain('.', "."),
    plain('<', "<"),
    plain('>', ">"),
    shifted('D', "D"),
    shifted('C', "C"),
    shifted('Y', "Y"),
    shifted('S', "S"),
    shifted('X', "X"),
    // Insert entry (alphabet chars above double as insert-mode text)
    plain('i', "i"),
    plain('a', "a"),
    plain('o', "o"),
    shifted('I', "I"),
    shifted('A', "A"),
    shifted('O', "O"),
    // Visual, registers, marks, macros
    plain('v', "v"),
    shifted('V', "V"),
    plain('"', "\""),
    plain('m', "m"),
    plain('\'', "'"),
    plain('`', "`"),
    plain('q', "q"),
    plain('@', "@"),
    // Search (in-memory only)
    plain('/', "/"),
    plain('?', "?"),
    // Scrolling, panes, jumps
    plain('z', "z"),
    ctrl('r', "<C-r>"),
    ctrl('a', "<C-a>"),
    ctrl('x', "<C-x>"),
    ctrl('d', "<C-d>"),
    ctrl('u', "<C-u>"),
    ctrl('f', "<C-f>"),
    ctrl('b', "<C-b>"),
    ctrl('o', "<C-o>"),
    ctrl('i', "<C-i>"),
    ctrl('v', "<C-v>"),
    ctrl('w', "<C-w>"),
    // Mode exits, weighted so sequences keep returning to normal mode
    FuzzKey {
        code: KeyCode::Esc,
        modifiers: KeyModifiers::NONE,
        desc: "<Esc>",
    },
    FuzzKey {
        code: KeyCode::Esc,
        modifiers: KeyModifiers::NONE,
        desc: "<Esc>",
    },
    FuzzKey {
        code: KeyCode::Esc,
        modifiers: KeyModifiers::NONE,
        desc: "<Esc>",
    },
    FuzzKey {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
        desc: "<CR>",
    },
];

/// Plain code-like text and a tricky one (multibyte, tabs, blank and long
/// lines, trailing spaces) to stress UTF-8 and width paths.
const TEXTS: &[&str] = &[
    "fn main() {\n    let total = 1;\n\n    if total > 0 {\n        println(total);\n    }\n}\n",
    "h\u{e9}llo w\u{f6}rld \u{3b1}\u{3b2}\u{3b3}\n\n\tindented\ttabs\t\ntrailing   \nxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n\nshort\n",
];

const SEQUENCE_LEN: usize = 32;

fn run_sequence(seed: u64) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    let text = TEXTS[(seed % TEXTS.len() as u64) as usize];

    let mut keys = Vec::with_capacity(SEQUENCE_LEN);
    for _ in 0..SEQUENCE_LEN {
        keys.push(*rng.pick(ALPHABET));
    }
    let desc: String = keys.iter().map(|k| k.desc).collect();

    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut editor = Editor::default();
        editor.set_size(80, 24);
        editor.replace_buffer_content(text);
        for key in &keys {
            handle_key(&mut editor, KeyEvent::new(key.code, key.modifiers));
        }
        editor
    }));

    let editor = match outcome {
        Ok(editor) => editor,
        Err(_) => return Err(format!("panicked (seed {seed}): {desc}")),
    };

    let buffer = editor.buffer();
    let max_line = buffer.addressable_line_count().saturating_sub(1);
    if editor.cursor.line > max_line {
        return Err(format!(
            "cursor line {} > last line {max_line} (seed {seed}): {desc}",
            editor.cursor.line
        ));
    }
    let line_len = buffer.line_len(editor.cursor.line);
    if editor.cursor.col > line_len {
        return Err(format!(
            "cursor col {} > line len {line_len} (seed {seed}): {desc}",
            editor.cursor.col
        ));
    }
    if editor.viewport_offset > max_line {
        return Err(format!(
            "viewport {} > last line {max_line} (seed {seed}): {desc}",
            editor.viewport_offset
        ));
    }
    Ok(())
}

fn run_seeds(range: std::ops::Range<u64>) {
    let failures: Vec<String> = range.filter_map(|seed| run_sequence(seed).err()).collect();
    assert!(
        failures.is_empty(),
        "{} fuzz failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Replay an explicit key list against a text; Err = invariant violated.
fn run_keys(text: &str, keys: &[FuzzKey]) -> Result<(), String> {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut editor = Editor::default();
        editor.set_size(80, 24);
        editor.replace_buffer_content(text);
        for key in keys {
            handle_key(&mut editor, KeyEvent::new(key.code, key.modifiers));
        }
        editor
    }));
    let desc: String = keys.iter().map(|k| k.desc).collect();
    let editor = match outcome {
        Ok(editor) => editor,
        Err(_) => return Err(format!("panicked: {desc}")),
    };
    let buffer = editor.buffer();
    let max_line = buffer.addressable_line_count().saturating_sub(1);
    if editor.cursor.line > max_line {
        return Err(format!(
            "cursor line {} > {max_line}: {desc}",
            editor.cursor.line
        ));
    }
    let line_len = buffer.line_len(editor.cursor.line);
    if editor.cursor.col > line_len {
        return Err(format!(
            "cursor col {} > {line_len}: {desc}",
            editor.cursor.col
        ));
    }
    if editor.viewport_offset > max_line {
        return Err(format!(
            "viewport {} > {max_line}: {desc}",
            editor.viewport_offset
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ALPHABET, Rng, SEQUENCE_LEN, TEXTS, run_keys, run_seeds};

    /// Greedy delta-debugging for failing seeds. Usage:
    /// `NEVI_FUZZ_MINIMIZE=2230,11672 cargo test fuzz_minimize -- --ignored --nocapture`
    /// Note: greedy single-key removal can stop at a local minimum when a
    /// prefix key changes how later keys parse (e.g. a pending register).
    #[test]
    #[ignore = "run with NEVI_FUZZ_MINIMIZE=<seed,...>"]
    fn fuzz_minimize() {
        let seeds = std::env::var("NEVI_FUZZ_MINIMIZE").unwrap_or_default();
        for seed in seeds
            .split(',')
            .filter_map(|s| s.trim().parse::<u64>().ok())
        {
            let mut rng = Rng::new(seed);
            let text = TEXTS[(seed % TEXTS.len() as u64) as usize];
            let mut keys = Vec::with_capacity(SEQUENCE_LEN);
            for _ in 0..SEQUENCE_LEN {
                keys.push(*rng.pick(ALPHABET));
            }
            if run_keys(text, &keys).is_ok() {
                println!("seed {seed}: does not fail, nothing to minimize");
                continue;
            }
            // Greedy removal until no single key can be dropped.
            loop {
                let mut shrunk = false;
                let mut i = 0;
                while i < keys.len() {
                    let mut candidate = keys.clone();
                    candidate.remove(i);
                    if run_keys(text, &candidate).is_err() {
                        keys = candidate;
                        shrunk = true;
                    } else {
                        i += 1;
                    }
                }
                if !shrunk {
                    break;
                }
            }
            println!(
                "seed {seed}: minimal = {}",
                run_keys(text, &keys).unwrap_err()
            );
        }
    }

    /// Fast deterministic slice that runs in the normal suite (~2s; editor
    /// construction dominates, so keep the seed count modest here).
    #[test]
    fn fuzz_key_sequences_smoke() {
        run_seeds(0..128);
    }

    /// Extended run for CI nightly / manual use:
    /// `cargo test fuzz_key_sequences_extended -- --ignored`
    #[test]
    #[ignore = "extended fuzz; run explicitly"]
    fn fuzz_key_sequences_extended() {
        run_seeds(128..20_000);
    }
}
