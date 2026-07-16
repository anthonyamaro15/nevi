use crate::editor::{Editor, Mode};
use crate::terminal::handle_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

mod editing_cases;
mod insert_entry_cases;
mod open_line_cases;
mod replace_cases;

use editing_cases::EDITING_CASES;
use insert_entry_cases::INSERT_ENTRY_CASES;
use open_line_cases::OPEN_LINE_CASES;
use replace_cases::REPLACE_CASES;

#[derive(Debug, Clone, Copy)]
struct OracleCase {
    name: &'static str,
    initial_text: &'static str,
    keys: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct OracleCategory {
    name: &'static str,
    cases: &'static [OracleCase],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorSnapshot {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
    viewport_top: usize,
    mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OracleComparison {
    passed: bool,
    report: String,
}

const ORACLE_TERM_WIDTH: u16 = 80;
const ORACLE_TERM_HEIGHT: u16 = 24;
const ORACLE_SHORT_TERM_HEIGHT: u16 = 12;
const ORACLE_SMALL_TERM_HEIGHTS: &[u16] = &[4, 5, 6];
const ORACLE_SCROLL_OFF: usize = 8;
const ORACLE_WRAP_WIDTH: usize = 9_999;

const SCREEN_POSITION_TEXT: &str = concat!(
    "line 001\n",
    "line 002\n",
    "line 003\n",
    "line 004\n",
    "line 005\n",
    "line 006\n",
    "line 007\n",
    "line 008\n",
    "line 009\n",
    "line 010\n",
    "line 011\n",
    "line 012\n",
    "line 013\n",
    "line 014\n",
    "line 015\n",
    "line 016\n",
    "line 017\n",
    "line 018\n",
    "line 019\n",
    "line 020\n",
    "line 021\n",
    "line 022\n",
    "line 023\n",
    "line 024\n",
    "line 025\n",
    "line 026\n",
    "line 027\n",
    "line 028\n",
    "line 029\n",
    "line 030\n",
    "line 031\n",
    "line 032\n",
    "line 033\n",
    "line 034\n",
    "line 035\n",
    "line 036\n",
    "line 037\n",
    "line 038\n",
    "line 039\n",
    "line 040\n",
    "line 041\n",
    "line 042\n",
    "line 043\n",
    "line 044\n",
    "line 045\n",
    "line 046\n",
    "line 047\n",
    "line 048\n",
    "line 049\n",
    "line 050\n",
    "line 051\n",
    "line 052\n",
    "line 053\n",
    "line 054\n",
    "line 055\n",
    "line 056\n",
    "line 057\n",
    "line 058\n",
    "line 059\n",
    "line 060\n",
    "line 061\n",
    "line 062\n",
    "line 063\n",
    "line 064\n",
    "line 065\n",
    "line 066\n",
    "line 067\n",
    "line 068\n",
    "line 069\n",
    "line 070\n",
    "line 071\n",
    "line 072\n",
    "line 073\n",
    "line 074\n",
    "line 075\n",
    "line 076\n",
    "line 077\n",
    "line 078\n",
    "line 079\n",
    "line 080\n",
    "line 081\n",
    "line 082\n",
    "line 083\n",
    "line 084\n",
    "line 085\n",
    "line 086\n",
    "line 087\n",
    "line 088\n",
    "line 089\n",
    "line 090\n",
    "line 091\n",
    "line 092\n",
    "line 093\n",
    "line 094\n",
    "line 095\n",
    "line 096\n",
    "line 097\n",
    "line 098\n",
    "line 099\n",
    "line 100\n",
);

const MOTION_CASES: &[OracleCase] = &[
    OracleCase {
        name: "move right",
        initial_text: "abc\n",
        keys: "l",
    },
    OracleCase {
        name: "move left",
        initial_text: "abc\n",
        keys: "llh",
    },
    OracleCase {
        name: "move down",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "j",
    },
    OracleCase {
        name: "move up",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "jjk",
    },
    OracleCase {
        name: "word forward",
        initial_text: "alpha beta gamma\n",
        keys: "w",
    },
    OracleCase {
        name: "big word forward",
        initial_text: "alpha.beta gamma\n",
        keys: "W",
    },
    OracleCase {
        name: "word backward",
        initial_text: "alpha beta gamma\n",
        keys: "wwb",
    },
    OracleCase {
        name: "big word backward",
        initial_text: "alpha.beta gamma.delta omega\n",
        keys: "WWB",
    },
    OracleCase {
        name: "word end",
        initial_text: "alpha beta\n",
        keys: "e",
    },
    OracleCase {
        name: "big word end",
        initial_text: "alpha.beta gamma\n",
        keys: "E",
    },
    OracleCase {
        name: "previous word end",
        initial_text: "alpha beta gamma\n",
        keys: "wwge",
    },
    OracleCase {
        name: "previous big word end",
        initial_text: "alpha.beta gamma.delta omega\n",
        keys: "WWgE",
    },
    OracleCase {
        name: "line start",
        initial_text: "alpha beta\n",
        keys: "$0",
    },
    OracleCase {
        name: "line end",
        initial_text: "alpha beta\n",
        keys: "$",
    },
    OracleCase {
        name: "first nonblank",
        initial_text: "  alpha\n",
        keys: "$^",
    },
    OracleCase {
        name: "find char forward",
        initial_text: "abcade\n",
        keys: "fa",
    },
    OracleCase {
        name: "find char backward",
        initial_text: "abcade\n",
        keys: "$Fa",
    },
    OracleCase {
        name: "till char forward",
        initial_text: "abcade\n",
        keys: "ta",
    },
    OracleCase {
        name: "till char backward",
        initial_text: "abcade\n",
        keys: "$Ta",
    },
    OracleCase {
        name: "repeat find char",
        initial_text: "abcabcabc\n",
        keys: "fa;",
    },
    OracleCase {
        name: "reverse repeat find char",
        initial_text: "abcabcabc\n",
        keys: "fa;,",
    },
    OracleCase {
        name: "matching bracket",
        initial_text: "(alpha)\n",
        keys: "%",
    },
    OracleCase {
        name: "screen top",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50GzzH",
    },
    OracleCase {
        name: "screen middle",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50GzzM",
    },
    OracleCase {
        name: "screen bottom",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50GzzL",
    },
    OracleCase {
        name: "counted screen top",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz30H",
    },
    OracleCase {
        name: "counted screen bottom",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz30L",
    },
    OracleCase {
        name: "center cursor line",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz",
    },
    OracleCase {
        name: "cursor line to top",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzt",
    },
    OracleCase {
        name: "cursor line to bottom",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzb",
    },
    OracleCase {
        name: "page down",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz<C-f>",
    },
    OracleCase {
        name: "page up",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz<C-b>",
    },
    OracleCase {
        name: "half page down",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz<C-d>",
    },
    OracleCase {
        name: "half page up",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz<C-u>",
    },
    OracleCase {
        name: "page down near file end",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "95Gzz<C-f>",
    },
    OracleCase {
        name: "page up near file start",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "5Gzz<C-b>",
    },
    OracleCase {
        name: "half page down near file end",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "95Gzz<C-d>",
    },
    OracleCase {
        name: "half page up near file start",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "5Gzz<C-u>",
    },
    OracleCase {
        name: "counted page down",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz2<C-f>",
    },
    OracleCase {
        name: "counted page up",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz2<C-b>",
    },
    OracleCase {
        name: "counted half page down",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz3<C-d>",
    },
    OracleCase {
        name: "counted half page up",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz3<C-u>",
    },
    OracleCase {
        name: "explicit one half page down",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz1<C-d>",
    },
    OracleCase {
        name: "remembered half page distance",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz2<C-d><C-d>",
    },
    OracleCase {
        name: "shared half page distance",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz2<C-d><C-u>",
    },
    OracleCase {
        name: "page down from file end",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "G<C-f>",
    },
    OracleCase {
        name: "partial page up at file start",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "11Gzt<C-b>",
    },
    OracleCase {
        name: "page back from file end",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "G<C-f><C-b>",
    },
    OracleCase {
        name: "page down with file end visible",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "G13k<C-f>",
    },
    OracleCase {
        name: "page down resets desired column",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50G7lzz<C-f>",
    },
    OracleCase {
        name: "enter next line first nonblank",
        initial_text: "zero\n    one\n  two\nthree\n",
        keys: "<CR>",
    },
    OracleCase {
        name: "counted enter next line first nonblank",
        initial_text: "zero\n    one\n  two\nthree\n",
        keys: "2<CR>",
    },
    OracleCase {
        name: "file top",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "gg",
    },
    OracleCase {
        name: "file bottom",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "G",
    },
    OracleCase {
        name: "counted file bottom",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "999G",
    },
    OracleCase {
        name: "counted down",
        initial_text: "alpha\nbeta\ngamma\ndelta\n",
        keys: "3j",
    },
    OracleCase {
        name: "counted word forward",
        initial_text: "alpha beta gamma\n",
        keys: "2w",
    },
];

const SHORT_VIEWPORT_CASES: &[OracleCase] = &[
    OracleCase {
        name: "short viewport cursor line to top",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzt",
    },
    OracleCase {
        name: "short viewport cursor line to bottom",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzb",
    },
    OracleCase {
        name: "short viewport page down",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz<C-f>",
    },
    OracleCase {
        name: "short viewport page up",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz<C-b>",
    },
    OracleCase {
        name: "short viewport half page down",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz<C-d>",
    },
    OracleCase {
        name: "short viewport half page up",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz<C-u>",
    },
    OracleCase {
        name: "short viewport page up at file start",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "6Gzb<C-b>",
    },
];

const SMALL_VIEWPORT_CASES: &[OracleCase] = &[
    OracleCase {
        name: "two-row viewport page down",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz<C-f>",
    },
    OracleCase {
        name: "two-row viewport page up",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50Gzz<C-b>",
    },
];

const WRAP_ENABLED_CASES: &[OracleCase] = &[OracleCase {
    name: "wrap enabled page down from file end",
    initial_text: SCREEN_POSITION_TEXT,
    keys: "G<C-f>",
}];

const UNDO_REDO_CASES: &[OracleCase] = &[
    OracleCase {
        name: "undo insert",
        initial_text: "alpha\n",
        keys: "A!<Esc>u",
    },
    OracleCase {
        name: "redo insert",
        initial_text: "alpha\n",
        keys: "A!<Esc>u<C-r>",
    },
];

const ORACLE_CATEGORIES: &[OracleCategory] = &[
    OracleCategory {
        name: "motions",
        cases: MOTION_CASES,
    },
    OracleCategory {
        name: "editing",
        cases: EDITING_CASES,
    },
    OracleCategory {
        name: "insert-entry",
        cases: INSERT_ENTRY_CASES,
    },
    OracleCategory {
        name: "open-line",
        cases: OPEN_LINE_CASES,
    },
    OracleCategory {
        name: "replace",
        cases: REPLACE_CASES,
    },
    OracleCategory {
        name: "undo-redo",
        cases: UNDO_REDO_CASES,
    },
];

fn oracle_categories() -> &'static [OracleCategory] {
    ORACLE_CATEGORIES
}

pub(crate) fn has_oracle_case(name: &str) -> bool {
    oracle_categories()
        .iter()
        .flat_map(|category| category.cases)
        .any(|case| case.name == name)
}

fn parse_key_sequence(input: &str) -> Result<Vec<KeyEvent>, String> {
    let mut keys = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut token = String::new();
            let mut closed = false;
            for token_ch in chars.by_ref() {
                if token_ch == '>' {
                    closed = true;
                    break;
                }
                token.push(token_ch);
            }

            if !closed {
                return Err(format!("unterminated key token in `{input}`"));
            }

            keys.push(parse_key_token(&token)?);
        } else {
            keys.push(char_key(ch));
        }
    }

    Ok(keys)
}

fn parse_key_token(token: &str) -> Result<KeyEvent, String> {
    let lower = token.to_ascii_lowercase();
    match lower.as_str() {
        "esc" => Ok(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        "cr" | "enter" | "return" => Ok(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        "tab" => Ok(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        "bs" | "backspace" => Ok(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        "left" => Ok(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        "right" => Ok(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        "up" => Ok(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        "down" => Ok(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        "space" => Ok(char_key(' ')),
        "lt" => Ok(char_key('<')),
        _ => {
            if let Some(control) = lower
                .strip_prefix("c-")
                .or_else(|| lower.strip_prefix("ctrl-"))
            {
                let mut chars = control.chars();
                if let (Some(ch), None) = (chars.next(), chars.next()) {
                    return Ok(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL));
                }
            }
            Err(format!("unsupported key token `<{token}>`"))
        }
    }
}

fn char_key(ch: char) -> KeyEvent {
    if ch.is_ascii_uppercase() {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::SHIFT)
    } else {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
    }
}

fn run_nevi_case(case: &OracleCase) -> Result<EditorSnapshot, String> {
    run_nevi_case_at_height(case, ORACLE_TERM_HEIGHT)
}

fn run_nevi_case_at_height(case: &OracleCase, term_height: u16) -> Result<EditorSnapshot, String> {
    run_nevi_case_with_options(case, term_height, false)
}

fn run_nevi_case_with_options(
    case: &OracleCase,
    term_height: u16,
    wrap: bool,
) -> Result<EditorSnapshot, String> {
    let mut editor = Editor::default();
    editor.set_size(ORACLE_TERM_WIDTH, term_height);
    editor.settings.editor.scroll_off = ORACLE_SCROLL_OFF;
    editor.settings.editor.wrap = wrap;
    editor.settings.editor.wrap_width = ORACLE_WRAP_WIDTH;
    editor.replace_buffer_content(case.initial_text);

    for key in parse_key_sequence(case.keys)? {
        handle_key(&mut editor, key);
    }

    Ok(snapshot_nevi(&editor))
}

fn snapshot_nevi(editor: &Editor) -> EditorSnapshot {
    EditorSnapshot {
        lines: normalized_lines(&editor.buffer().content()),
        cursor_line: editor.cursor.line,
        cursor_col: editor.cursor.col,
        viewport_top: editor.viewport_offset,
        mode: nevi_mode_name(editor.mode).to_string(),
    }
}

fn nevi_mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "normal",
        Mode::Insert => "insert",
        Mode::Replace => "replace",
        Mode::Command => "command",
        Mode::Search => "search",
        Mode::Visual => "visual",
        Mode::VisualLine => "visual-line",
        Mode::VisualBlock => "visual-block",
        Mode::Finder => "finder",
        Mode::Explorer => "explorer",
        Mode::RenamePrompt => "rename",
    }
}

fn normalized_lines(text: &str) -> Vec<String> {
    let normalized = text.strip_suffix('\n').unwrap_or(text);
    if normalized.is_empty() {
        return vec![String::new()];
    }
    normalized.split('\n').map(str::to_string).collect()
}

fn compare_snapshots(
    case: &OracleCase,
    nevi: EditorSnapshot,
    nvim: EditorSnapshot,
) -> OracleComparison {
    let mut mismatches = Vec::new();
    if nevi.lines != nvim.lines {
        mismatches.push(format!(
            "lines: nevi={:?} nvim={:?}",
            nevi.lines, nvim.lines
        ));
    }
    if nevi.cursor_line != nvim.cursor_line {
        mismatches.push(format!(
            "cursor_line: nevi={} nvim={}",
            nevi.cursor_line, nvim.cursor_line
        ));
    }
    if nevi.cursor_col != nvim.cursor_col {
        mismatches.push(format!(
            "cursor_col: nevi={} nvim={}",
            nevi.cursor_col, nvim.cursor_col
        ));
    }
    if nevi.viewport_top != nvim.viewport_top {
        mismatches.push(format!(
            "viewport_top: nevi={} nvim={}",
            nevi.viewport_top, nvim.viewport_top
        ));
    }
    if nevi.mode != nvim.mode {
        mismatches.push(format!("mode: nevi={} nvim={}", nevi.mode, nvim.mode));
    }

    if mismatches.is_empty() {
        OracleComparison {
            passed: true,
            report: format!("Vim oracle case `{}` matched", case.name),
        }
    } else {
        OracleComparison {
            passed: false,
            report: format_mismatch_report(case, &mismatches, &nevi, &nvim),
        }
    }
}

fn format_mismatch_report(
    case: &OracleCase,
    mismatches: &[String],
    nevi: &EditorSnapshot,
    nvim: &EditorSnapshot,
) -> String {
    format!(
        "Vim oracle case `{}` diverged\n\
         Case: {}\n\
         Keys: {}\n\
         Initial text:\n{}\n\
         Mismatches:\n{}\n\n\
         Nevi snapshot:\n{}\n\n\
         Neovim snapshot:\n{}",
        case.name,
        case.name,
        case.keys,
        format_initial_text(case.initial_text),
        mismatches.join("\n"),
        format_snapshot(nevi),
        format_snapshot(nvim)
    )
}

fn format_initial_text(text: &str) -> String {
    if text.is_empty() {
        "  <empty>".to_string()
    } else {
        text.lines()
            .enumerate()
            .map(|(index, line)| format!("  {:>3}: {}", index + 1, line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn format_snapshot(snapshot: &EditorSnapshot) -> String {
    let lines = snapshot
        .lines
        .iter()
        .enumerate()
        .map(|(index, line)| format!("  {:>3}: {}", index + 1, line))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "  mode={} cursor=({}, {}) viewport_top={}\n{}",
        snapshot.mode, snapshot.cursor_line, snapshot.cursor_col, snapshot.viewport_top, lines
    )
}

fn compare_with_neovim(case: &OracleCase) -> Result<OracleComparison, String> {
    compare_with_neovim_at_height(case, ORACLE_TERM_HEIGHT)
}

fn compare_with_neovim_at_height(
    case: &OracleCase,
    term_height: u16,
) -> Result<OracleComparison, String> {
    let nevi = run_nevi_case_at_height(case, term_height)?;
    let nvim = run_neovim_case_at_height(case, term_height)?;
    Ok(compare_snapshots(case, nevi, nvim))
}

fn compare_with_neovim_with_options(
    case: &OracleCase,
    term_height: u16,
    wrap: bool,
) -> Result<OracleComparison, String> {
    let nevi = run_nevi_case_with_options(case, term_height, wrap)?;
    let nvim = run_neovim_case_with_options(case, term_height, wrap)?;
    Ok(compare_snapshots(case, nevi, nvim))
}

fn run_neovim_case_at_height(
    case: &OracleCase,
    term_height: u16,
) -> Result<EditorSnapshot, String> {
    run_neovim_case_with_options(case, term_height, false)
}

fn run_neovim_case_with_options(
    case: &OracleCase,
    term_height: u16,
    wrap: bool,
) -> Result<EditorSnapshot, String> {
    let tmp = unique_temp_dir("nevi_vim_oracle");
    std::fs::create_dir_all(&tmp).map_err(|err| format!("create temp dir: {err}"))?;
    let file_path = tmp.join("case.txt");
    let script_path = tmp.join("snapshot.lua");
    std::fs::write(&file_path, case.initial_text).map_err(|err| format!("write case: {err}"))?;
    std::fs::write(
        &script_path,
        neovim_snapshot_lua(case.keys, term_height.saturating_sub(2), wrap),
    )
    .map_err(|err| format!("write lua script: {err}"))?;

    let output = Command::new("nvim")
        .args(["--headless", "-u", "NONE", "-i", "NONE", "-n"])
        .arg(&file_path)
        .arg(format!("+luafile {}", script_path.display()))
        .arg("+qa!")
        .output()
        .map_err(|err| format!("failed to run nvim: {err}"))?;

    let _ = std::fs::remove_dir_all(&tmp);

    if !output.status.success() {
        return Err(format!(
            "nvim exited with {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(json_line) = stdout.lines().rev().find(|line| !line.trim().is_empty()) else {
        return Err("nvim produced no snapshot output".to_string());
    };
    snapshot_from_neovim_json(json_line)
}

fn neovim_snapshot_lua(keys: &str, text_rows: u16, wrap: bool) -> String {
    format!(
        r#"
local keys = vim.api.nvim_replace_termcodes("{}", true, false, true)
vim.o.scrolloff = {}
vim.o.wrap = {}
vim.api.nvim_win_set_width(0, {})
vim.api.nvim_win_set_height(0, {})
vim.api.nvim_feedkeys(keys, "xt", false)
local pos = vim.api.nvim_win_get_cursor(0)
local snapshot = {{
  lines = vim.api.nvim_buf_get_lines(0, 0, -1, false),
  cursor_line = pos[1] - 1,
  cursor_col = pos[2],
  viewport_top = vim.fn.line("w0") - 1,
  mode = vim.api.nvim_get_mode().mode,
}}
io.stdout:write(vim.fn.json_encode(snapshot) .. "\n")
"#,
        lua_escape(keys),
        ORACLE_SCROLL_OFF,
        wrap,
        ORACLE_TERM_WIDTH,
        text_rows
    )
}

fn snapshot_from_neovim_json(line: &str) -> Result<EditorSnapshot, String> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|err| format!("parse nvim snapshot: {err}: {line}"))?;

    let lines = value
        .get("lines")
        .and_then(|lines| lines.as_array())
        .ok_or_else(|| format!("nvim snapshot missing lines: {line}"))?
        .iter()
        .map(|line| {
            line.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("nvim line is not a string: {line}"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let cursor_line = json_usize(&value, "cursor_line", line)?;
    let cursor_col = json_usize(&value, "cursor_col", line)?;
    let viewport_top = json_usize(&value, "viewport_top", line)?;
    let raw_mode = value
        .get("mode")
        .and_then(|mode| mode.as_str())
        .ok_or_else(|| format!("nvim snapshot missing mode: {line}"))?;

    Ok(EditorSnapshot {
        lines,
        cursor_line,
        cursor_col,
        viewport_top,
        mode: normalize_neovim_mode(raw_mode).to_string(),
    })
}

fn json_usize(value: &serde_json::Value, key: &str, source: &str) -> Result<usize, String> {
    value
        .get(key)
        .and_then(|number| number.as_u64())
        .and_then(|number| usize::try_from(number).ok())
        .ok_or_else(|| format!("nvim snapshot missing numeric {key}: {source}"))
}

fn normalize_neovim_mode(mode: &str) -> &'static str {
    match mode.chars().next() {
        Some('n') => "normal",
        Some('i') => "insert",
        Some('R') => "replace",
        Some('v') => "visual",
        Some('V') => "visual-line",
        Some('\u{16}') => "visual-block",
        Some('c') => "command",
        _ => "unknown",
    }
}

fn lua_escape(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn parses_plain_shift_control_and_named_keys() {
        let keys =
            parse_key_sequence("jG<C-d><Esc><CR><Left><Right><Up><Down>").expect("parse keys");

        assert_eq!(keys.len(), 9);
        assert_eq!(keys[0].code, KeyCode::Char('j'));
        assert_eq!(keys[0].modifiers, KeyModifiers::NONE);
        assert_eq!(keys[1].code, KeyCode::Char('G'));
        assert_eq!(keys[1].modifiers, KeyModifiers::SHIFT);
        assert_eq!(keys[2].code, KeyCode::Char('d'));
        assert_eq!(keys[2].modifiers, KeyModifiers::CONTROL);
        assert_eq!(keys[3].code, KeyCode::Esc);
        assert_eq!(keys[4].code, KeyCode::Enter);
        assert_eq!(keys[5].code, KeyCode::Left);
        assert_eq!(keys[6].code, KeyCode::Right);
        assert_eq!(keys[7].code, KeyCode::Up);
        assert_eq!(keys[8].code, KeyCode::Down);
    }

    #[test]
    fn nevi_runner_applies_keys_and_snapshots_state() {
        let case = OracleCase {
            name: "delete first char on second line",
            initial_text: "alpha\nbeta\n",
            keys: "j0x",
        };

        let snapshot = run_nevi_case(&case).expect("run nevi case");

        assert_eq!(
            snapshot,
            EditorSnapshot {
                lines: vec!["alpha".to_string(), "eta".to_string()],
                cursor_line: 1,
                cursor_col: 0,
                viewport_top: 0,
                mode: "normal".to_string(),
            }
        );
    }

    #[test]
    fn comparison_report_lists_mismatches() {
        let case = OracleCase {
            name: "cursor mismatch",
            initial_text: "abc\n",
            keys: "l",
        };
        let nevi = EditorSnapshot {
            lines: vec!["abc".to_string()],
            cursor_line: 0,
            cursor_col: 1,
            viewport_top: 0,
            mode: "normal".to_string(),
        };
        let nvim = EditorSnapshot {
            lines: vec!["abc".to_string()],
            cursor_line: 0,
            cursor_col: 2,
            viewport_top: 0,
            mode: "normal".to_string(),
        };

        let comparison = compare_snapshots(&case, nevi, nvim);

        assert!(!comparison.passed);
        assert!(comparison.report.contains("cursor mismatch"));
        assert!(comparison.report.contains("cursor_col"));
        assert!(comparison.report.contains("nevi=1"));
        assert!(comparison.report.contains("nvim=2"));
    }

    #[test]
    fn comparison_detects_viewport_top_mismatch() {
        let case = OracleCase {
            name: "viewport mismatch",
            initial_text: SCREEN_POSITION_TEXT,
            keys: "50Gzz",
        };
        let mut nevi_editor = Editor::default();
        nevi_editor.replace_buffer_content(SCREEN_POSITION_TEXT);
        nevi_editor.cursor.line = 49;
        nevi_editor.viewport_offset = 39;

        let mut nvim_editor = Editor::default();
        nvim_editor.replace_buffer_content(SCREEN_POSITION_TEXT);
        nvim_editor.cursor.line = 49;
        nvim_editor.viewport_offset = 40;

        let comparison = compare_snapshots(
            &case,
            snapshot_nevi(&nevi_editor),
            snapshot_nevi(&nvim_editor),
        );

        assert!(!comparison.passed);
        assert!(comparison.report.contains("viewport_top"));
        assert!(comparison.report.contains("nevi=39"));
        assert!(comparison.report.contains("nvim=40"));
    }

    #[test]
    fn neovim_snapshot_parses_viewport_top() {
        let snapshot = snapshot_from_neovim_json(
            r#"{"lines":["alpha"],"cursor_line":0,"cursor_col":0,"viewport_top":4,"mode":"n"}"#,
        )
        .expect("parse snapshot");

        assert_eq!(snapshot.viewport_top, 4);
    }

    #[test]
    fn oracle_suite_groups_common_vim_parity_cases() {
        let categories = oracle_categories();

        assert!(
            categories
                .iter()
                .any(|category| category.name == "motions" && category.cases.len() >= 10),
            "motions category should cover common cursor movement"
        );
        assert!(
            categories
                .iter()
                .any(|category| category.name == "editing" && category.cases.len() >= 8),
            "editing category should cover common changes"
        );
        assert!(
            categories
                .iter()
                .any(|category| category.name == "undo-redo" && category.cases.len() >= 2),
            "undo-redo category should cover timeline basics"
        );
        assert!(
            categories
                .iter()
                .any(|category| category.name == "replace" && category.cases.len() >= 15),
            "replace category should cover one-shot and interactive replacement"
        );

        let all_keys = categories
            .iter()
            .flat_map(|category| category.cases.iter().map(|case| case.keys))
            .collect::<Vec<_>>();
        for required in [
            "w",
            "wwb",
            "gg",
            "G",
            "2dd",
            "ciwdone<Esc>",
            "2r<CR>",
            "2RXY<Esc>",
            "A!<Esc>u",
            "A!<Esc>u<C-r>",
        ] {
            assert!(
                all_keys.contains(&required),
                "oracle suite should include `{required}`"
            );
        }
    }

    #[test]
    fn comparison_report_includes_repro_context_and_snapshots() {
        let case = OracleCase {
            name: "line mismatch",
            initial_text: "before\n",
            keys: "A!<Esc>",
        };
        let nevi = EditorSnapshot {
            lines: vec!["before!".to_string()],
            cursor_line: 0,
            cursor_col: 6,
            viewport_top: 0,
            mode: "normal".to_string(),
        };
        let nvim = EditorSnapshot {
            lines: vec!["before".to_string()],
            cursor_line: 0,
            cursor_col: 5,
            viewport_top: 0,
            mode: "normal".to_string(),
        };

        let comparison = compare_snapshots(&case, nevi, nvim);

        assert!(!comparison.passed);
        assert!(comparison.report.contains("Case: line mismatch"));
        assert!(comparison.report.contains("Keys: A!<Esc>"));
        assert!(comparison.report.contains("Initial text:"));
        assert!(comparison.report.contains("before"));
        assert!(comparison.report.contains("Nevi snapshot:"));
        assert!(comparison.report.contains("Neovim snapshot:"));
        assert!(comparison.report.contains("Mismatches:"));
    }

    #[test]
    #[ignore = "requires NEVI_VIM_ORACLE=1 and nvim on PATH"]
    fn vim_oracle_smoke_matches_neovim_for_basic_normal_edit() {
        if std::env::var_os("NEVI_VIM_ORACLE").is_none() {
            eprintln!("skipping Vim oracle smoke test; set NEVI_VIM_ORACLE=1 to run");
            return;
        }

        let mut reports = Vec::new();
        for category in oracle_categories() {
            for case in category.cases {
                let comparison = compare_with_neovim(case).expect("run oracle comparison");
                if !comparison.passed {
                    reports.push(format!("[{}] {}", category.name, comparison.report));
                }
            }
        }
        for case in SHORT_VIEWPORT_CASES {
            let comparison = compare_with_neovim_at_height(case, ORACLE_SHORT_TERM_HEIGHT)
                .expect("run short viewport oracle comparison");
            if !comparison.passed {
                reports.push(format!("[short-viewport] {}", comparison.report));
            }
        }
        for term_height in ORACLE_SMALL_TERM_HEIGHTS {
            for case in SMALL_VIEWPORT_CASES {
                let comparison = compare_with_neovim_at_height(case, *term_height)
                    .expect("run small viewport oracle comparison");
                if !comparison.passed {
                    reports.push(format!(
                        "[small-viewport height={term_height}] {}",
                        comparison.report
                    ));
                }
            }
        }
        for case in WRAP_ENABLED_CASES {
            let comparison = compare_with_neovim_with_options(case, ORACLE_TERM_HEIGHT, true)
                .expect("run wrap-enabled oracle comparison");
            if !comparison.passed {
                reports.push(format!("[wrap-enabled] {}", comparison.report));
            }
        }

        assert!(reports.is_empty(), "{}", reports.join("\n\n"));
    }
}
