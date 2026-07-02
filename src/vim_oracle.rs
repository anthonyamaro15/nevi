use crate::editor::{Editor, Mode};
use crate::terminal::handle_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy)]
struct OracleCase {
    name: &'static str,
    initial_text: &'static str,
    keys: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorSnapshot {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
    mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OracleComparison {
    passed: bool,
    report: String,
}

const SMOKE_CASES: &[OracleCase] = &[
    OracleCase {
        name: "delete first char on second line",
        initial_text: "alpha\nbeta\n",
        keys: "j0x",
    },
    OracleCase {
        name: "append punctuation at line end",
        initial_text: "alpha\n",
        keys: "A!<Esc>",
    },
    OracleCase {
        name: "delete current line",
        initial_text: "alpha\nbeta\n",
        keys: "dd",
    },
];

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
    let mut editor = Editor::default();
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
            report: format!(
                "Vim oracle case `{}` diverged:\n{}",
                case.name,
                mismatches.join("\n")
            ),
        }
    }
}

fn compare_with_neovim(case: &OracleCase) -> Result<OracleComparison, String> {
    let nevi = run_nevi_case(case)?;
    let nvim = run_neovim_case(case)?;
    Ok(compare_snapshots(case, nevi, nvim))
}

fn run_neovim_case(case: &OracleCase) -> Result<EditorSnapshot, String> {
    let tmp = unique_temp_dir("nevi_vim_oracle");
    std::fs::create_dir_all(&tmp).map_err(|err| format!("create temp dir: {err}"))?;
    let file_path = tmp.join("case.txt");
    let script_path = tmp.join("snapshot.lua");
    std::fs::write(&file_path, case.initial_text).map_err(|err| format!("write case: {err}"))?;
    std::fs::write(&script_path, neovim_snapshot_lua(case.keys))
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

fn neovim_snapshot_lua(keys: &str) -> String {
    format!(
        r#"
local keys = vim.api.nvim_replace_termcodes("{}", true, false, true)
vim.api.nvim_feedkeys(keys, "xt", false)
local pos = vim.api.nvim_win_get_cursor(0)
local snapshot = {{
  lines = vim.api.nvim_buf_get_lines(0, 0, -1, false),
  cursor_line = pos[1] - 1,
  cursor_col = pos[2],
  mode = vim.api.nvim_get_mode().mode,
}}
io.stdout:write(vim.fn.json_encode(snapshot) .. "\n")
"#,
        lua_escape(keys)
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
    let raw_mode = value
        .get("mode")
        .and_then(|mode| mode.as_str())
        .ok_or_else(|| format!("nvim snapshot missing mode: {line}"))?;

    Ok(EditorSnapshot {
        lines,
        cursor_line,
        cursor_col,
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
        let keys = parse_key_sequence("jG<C-d><Esc><CR>").expect("parse keys");

        assert_eq!(keys.len(), 5);
        assert_eq!(keys[0].code, KeyCode::Char('j'));
        assert_eq!(keys[0].modifiers, KeyModifiers::NONE);
        assert_eq!(keys[1].code, KeyCode::Char('G'));
        assert_eq!(keys[1].modifiers, KeyModifiers::SHIFT);
        assert_eq!(keys[2].code, KeyCode::Char('d'));
        assert_eq!(keys[2].modifiers, KeyModifiers::CONTROL);
        assert_eq!(keys[3].code, KeyCode::Esc);
        assert_eq!(keys[4].code, KeyCode::Enter);
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
            mode: "normal".to_string(),
        };
        let nvim = EditorSnapshot {
            lines: vec!["abc".to_string()],
            cursor_line: 0,
            cursor_col: 2,
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
    #[ignore = "requires NEVI_VIM_ORACLE=1 and nvim on PATH"]
    fn vim_oracle_smoke_matches_neovim_for_basic_normal_edit() {
        if std::env::var_os("NEVI_VIM_ORACLE").is_none() {
            eprintln!("skipping Vim oracle smoke test; set NEVI_VIM_ORACLE=1 to run");
            return;
        }

        let mut reports = Vec::new();
        for case in SMOKE_CASES {
            let comparison = compare_with_neovim(case).expect("run oracle comparison");
            if !comparison.passed {
                reports.push(comparison.report);
            }
        }

        assert!(reports.is_empty(), "{}", reports.join("\n\n"));
    }
}
