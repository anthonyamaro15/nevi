//! The reproductions below make tree-sitter call C `abort()` (an assertion in
//! `ts_parser__external_scanner_serialize`) when they reach the parser. If
//! the guard ever stops catching one, this whole test binary dies with
//! SIGABRT instead of reporting a failed test; a sudden abort of the suite
//! points here.

use super::{markdown, too_deep_to_parse};
use crate::editor::{Buffer, Editor, Mode};
use crate::finder::{FinderItem, FinderMode};
use crate::syntax::SyntaxManager;
use std::path::{Path, PathBuf};

fn nested_list(depth: usize) -> String {
    (0..depth)
        .map(|level| format!("{}- a\n", "  ".repeat(level)))
        .collect()
}

/// The reported inputs, each of which aborted the editor on open.
fn reproductions() -> Vec<(&'static str, String)> {
    vec![
        ("quotes.md", "> ".repeat(400)),
        ("list.md", nested_list(300)),
    ]
}

fn parsed(name: &str, text: &str) -> (SyntaxManager, Buffer) {
    let mut syntax = SyntaxManager::new();
    syntax.set_language_from_path(Path::new(name));
    let mut buffer = Buffer::new();
    buffer.insert_str(0, 0, text);
    syntax.parse(&mut buffer);
    (syntax, buffer)
}

fn highlights_something(syntax: &SyntaxManager, text: &str) -> bool {
    (0..text.lines().count()).any(|line| !syntax.get_line_highlights(line).is_empty())
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nevi-scanner-guard-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn reproductions_open_without_a_tree() {
    for (name, text) in reproductions() {
        let (syntax, _) = parsed(name, &text);
        assert!(syntax.get_tree_and_source().is_none(), "{name} was parsed");
        assert!(!syntax.has_highlighting(), "{name} is highlighted");
        assert!(syntax.get_line_highlights(0).is_empty(), "{name}");
    }
}

#[test]
fn reproductions_are_not_parsed_from_a_string() {
    // The finder preview and hover code blocks go through parse_string.
    for (name, text) in reproductions() {
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path(Path::new(name));
        syntax.parse_string(&text);
        assert!(syntax.get_tree_and_source().is_none(), "{name} was parsed");
        assert!(!syntax.has_highlighting(), "{name} is highlighted");
    }
}

#[test]
fn finder_preview_of_a_reproduction_does_not_parse_it() {
    // Selecting a file in the picker is enough to parse it, and the 800-byte
    // quote line fits the preview's own limits.
    let dir = temp_dir("preview");
    let preview = |path: &Path| {
        let mut editor = Editor::default();
        editor.finder.mode = FinderMode::Files;
        editor.finder.items = vec![FinderItem::new("file".to_string(), path.to_path_buf())];
        editor.finder.filtered = vec![0];
        editor.finder.preview_enabled = true;
        editor.mode = Mode::Finder;
        editor.update_finder_preview();
        editor.preview_syntax.has_highlighting()
    };
    for (name, text) in reproductions() {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write file");
        assert!(!preview(&path), "{name} preview is highlighted");
    }
    let safe = dir.join("safe.md");
    std::fs::write(&safe, "# Title\n\n> quoted *text*\n").expect("write file");
    assert!(preview(&safe), "an ordinary file should still highlight");
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn deepening_edit_drops_the_tree_and_undoing_it_restores_highlighting() {
    // A list 80 deep, then 40 more levels typed below it.
    let text = nested_list(80);
    let (mut syntax, mut buffer) = parsed("notes.md", &text);
    assert!(syntax.has_highlighting());
    let deeper: String = (80..120)
        .map(|level| format!("{}- a\n", "  ".repeat(level)))
        .collect();
    buffer.insert_str(80, 0, &deeper);
    syntax.parse(&mut buffer);
    assert!(!syntax.has_highlighting(), "the deeper list was parsed");
    buffer.delete_range(80, 0, 120, 0);
    syntax.parse(&mut buffer);
    assert!(syntax.has_highlighting());
    assert!(highlights_something(&syntax, &text));
}

#[test]
fn other_languages_are_parsed_as_before() {
    let deep = "> ".repeat(400);
    assert!(too_deep_to_parse("markdown", &deep));
    assert!(!too_deep_to_parse("rust", &deep));
    assert!(!too_deep_to_parse("json", &deep));
}

#[test]
fn markdown_bound_follows_the_real_stack() {
    // Each line: matched blocks + opened blocks + one leaf of slack.
    assert_eq!(markdown::open_blocks_bound(b"plain text"), 1);
    assert_eq!(markdown::open_blocks_bound(b"> > > a"), 4);
    assert_eq!(markdown::open_blocks_bound(b"- - - a"), 4);
    assert_eq!(markdown::open_blocks_bound(nested_list(99).as_bytes()), 100);
    // The real parser first aborted at 255 levels for each of these shapes
    // (probed one process per case); the guard lets through 99.
    for depth in [99, 254] {
        let quotes = "> ".repeat(depth);
        assert_eq!(markdown::open_blocks_bound(quotes.as_bytes()), depth + 1);
        assert_eq!(
            markdown::open_blocks_bound(nested_list(depth).as_bytes()),
            depth + 1
        );
    }
    assert!(!too_deep_to_parse("markdown", &"> ".repeat(99)));
    assert!(too_deep_to_parse("markdown", &"> ".repeat(100)));
    // Tabs count as 4 columns, which can only add.
    let tabbed: String = (0..60)
        .map(|level| format!("{}- a\n", "\t".repeat(level)))
        .collect();
    assert!(markdown::open_blocks_bound(tabbed.as_bytes()) >= 61);
}

#[test]
fn ordinary_markdown_keeps_highlighting() {
    let table: String = std::iter::once("# Results\n\n| a | b | c |\n|---|:-:|--:|\n")
        .chain(std::iter::repeat_n("| 1 | 2 | 3 |\n", 200))
        .collect();
    let art = "\
```
      +--------+       +--------+
      | client | ----> | server |
      +--------+       +--------+
```

        +---+
        |   |   <- indented art
        +---+
";
    let deep_list: String = (0..10)
        .map(|level| format!("{}- level {level}\n", "    ".repeat(level)))
        .collect();
    let docs = [
        ("rule.md", format!("Title\n\n{}\n\ntext\n", "-".repeat(300))),
        ("stars.md", "a\n\n* * *\n\nb\n\n_____\n".to_string()),
        ("setext.md", format!("Heading\n{}\n\nbody\n", "=".repeat(200))),
        ("table.md", table),
        (
            "numbers.md",
            "# Numbers\n\n12345678901234567890 is a big number\n2024. A year\n1234567890. Ten digits\n"
                .to_string(),
        ),
        ("art.md", art.to_string()),
        ("deep_list.md", deep_list),
        ("quote.md", "> a long quoted line\n".repeat(500)),
        ("readme.md", include_str!("../../../README.md").to_string()),
        ("changelog.md", include_str!("../../../CHANGELOG.md").to_string()),
    ];
    for (name, text) in docs {
        assert!(
            markdown::open_blocks_bound(text.as_bytes()) <= 30,
            "{name} looks deep"
        );
        let (syntax, _) = parsed(name, &text);
        assert!(
            syntax.get_tree_and_source().is_some(),
            "{name} was not parsed"
        );
        assert!(
            highlights_something(&syntax, &text),
            "{name} has no highlights"
        );
    }
}

/// Small deterministic xorshift, so the test needs no new dependency.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Pieces that push scanner state: markers that nest on one line, markers
/// that start a nested line, the indentation between levels, and everything
/// else worth mixing in.
struct Pieces {
    same_line: &'static [&'static str],
    line_start: &'static [&'static str],
    indents: &'static [&'static str],
    mixed: &'static [&'static str],
}

const MARKDOWN: Pieces = Pieces {
    same_line: &[
        "> ", ">", "- ", "* ", "+ ", "1. ", "12) ", "-\t", ">\t", "1.\t",
    ],
    line_start: &["- a", "* a", "+ a", "1. a", "9) a", "> a", "-", "> - a"],
    indents: &["  ", " ", "\t", " \t", "> ", ">"],
    mixed: &[
        "> ",
        "- ",
        "* ",
        "1. ",
        "-",
        "*",
        "  ",
        "    ",
        "\t",
        "\n",
        "\r\n",
        "\r",
        "a",
        "text ",
        "```\n",
        "~~~\n",
        "    code\n",
        "<div>\n",
        "---\n",
        "***\n",
        "# h\n",
        "| a |\n",
        "\n\n",
    ],
};

fn pick<'a>(rng: &mut XorShift, from: &[&'a str]) -> &'a str {
    from[rng.below(from.len())]
}

/// Keeps the test within a few seconds in a debug build, where tree-sitter
/// parses hostile input slowly. The deep indented shapes this cuts short are
/// covered by the fixed cases above.
const MAX_CASE_BYTES: usize = 2_000;

/// One input with roughly `depth` levels of one construct, or a random mix.
fn hostile_text(rng: &mut XorShift, pieces: &Pieces, depth: usize) -> String {
    let mut text = String::new();
    match rng.below(3) {
        0 => {
            let units: Vec<&str> = (0..1 + rng.below(3))
                .map(|_| pick(rng, pieces.same_line))
                .collect();
            for _ in 0..depth {
                text.push_str(pick(rng, &units));
            }
        }
        1 => {
            let indent = pick(rng, pieces.indents);
            let marker = pick(rng, pieces.line_start);
            for level in 0..depth {
                if text.len() > MAX_CASE_BYTES {
                    break;
                }
                text.push_str(&indent.repeat(level));
                text.push_str(marker);
                text.push('\n');
            }
        }
        _ => {
            let favourites: Vec<&str> = (0..3).map(|_| pick(rng, pieces.mixed)).collect();
            for _ in 0..depth * 2 {
                if text.len() > MAX_CASE_BYTES {
                    break;
                }
                let piece = if rng.below(4) == 0 {
                    pick(rng, pieces.mixed)
                } else {
                    pick(rng, &favourites)
                };
                text.push_str(piece);
            }
        }
    }
    text
}

/// Random nesting of everything that pushes scanner state, at depths spread
/// below, around and well above the guard's limit, on one line and across
/// lines. The property is only that the process survives: either the guard
/// skips the text or tree-sitter parses it.
#[test]
fn random_nesting_never_reaches_a_parse_that_aborts() {
    let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
    for (extension, pieces, cases) in [("md", &MARKDOWN, 2000)] {
        // One manager for the whole run: building one compiles the highlight query.
        let mut syntax = SyntaxManager::new();
        syntax.set_language_from_path(Path::new(&format!("fuzz.{extension}")));
        let (mut parsed_count, mut skipped) = (0, 0);
        for _ in 0..cases {
            let depth = rng.below(320);
            let text = hostile_text(&mut rng, pieces, depth);
            let mut buffer = Buffer::new();
            buffer.insert_str(0, 0, &text);
            syntax.parse(&mut buffer);
            match syntax.get_tree_and_source() {
                Some(_) => parsed_count += 1,
                None => skipped += 1,
            }
        }
        // Both outcomes must actually occur for the test to mean anything.
        assert!(
            parsed_count > cases / 5,
            "{extension}: only {parsed_count} parsed"
        );
        assert!(skipped > cases / 5, "{extension}: only {skipped} skipped");
    }
}
