#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum KeybindMode {
    Normal,
    Insert,
    Visual,
    Leader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoverageKind {
    /// Behavior should match Vim/Neovim and is protected by a Vim oracle case.
    VimOracle,
    /// Behavior is Nevi-owned and protected by a focused Nevi regression test.
    NeviRegression,
    /// Behavior is configuration/default-keymap plumbing.
    ConfigMapping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoverageState {
    /// The entry has automated regression coverage.
    ///
    /// For `VimOracle`, `test_id` is an oracle case name. For other kinds, it is
    /// the Rust test name that protects the behavior.
    Protected { test_id: &'static str },
    /// The keybind is claimed/supported, but still needs a focused regression.
    ///
    /// Keeping explicit gaps in the inventory lets us grow coverage without
    /// pretending every documented key is already protected.
    // Retained so a newly inventoried default can be tracked explicitly
    // before its real-Neovim oracle case lands.
    #[allow(dead_code)]
    NeedsCoverage { reason: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeybindCoverage {
    pub(crate) mode: KeybindMode,
    pub(crate) key: &'static str,
    pub(crate) description: &'static str,
    pub(crate) kind: CoverageKind,
    pub(crate) state: CoverageState,
}

const KEYBIND_COVERAGE: &[KeybindCoverage] = &[
    vim_oracle("h", "Move cursor left", "move left"),
    vim_oracle("j", "Move cursor down", "move down"),
    vim_oracle("k", "Move cursor up", "move up"),
    vim_oracle("l", "Move cursor right", "move right"),
    vim_oracle("w", "Move to start of next word", "word forward"),
    vim_oracle("b", "Move to start of previous word", "word backward"),
    vim_oracle("e", "Move to end of word", "word end"),
    vim_oracle("0", "Move to start of line", "line start"),
    vim_oracle("^", "Move to first non-blank character", "first nonblank"),
    vim_oracle("$", "Move to end of line", "line end"),
    vim_oracle("g_", "Move to last non-blank character", "last non blank"),
    vim_oracle("|", "Move to column [count]", "to column"),
    vim_oracle("gM", "Move to middle of the line's text", "middle of line"),
    vim_oracle("]]", "Move to next section start", "section forward"),
    vim_oracle("[[", "Move to previous section start", "section backward"),
    vim_oracle("][", "Move to next section end", "section end forward"),
    vim_oracle("[]", "Move to previous section end", "section end backward"),
    vim_oracle("[{", "Move to previous unmatched {", "unmatched open brace"),
    vim_oracle("]}", "Move to next unmatched }", "unmatched close brace"),
    vim_oracle(
        "[(",
        "Move to previous unmatched (",
        "unmatched open paren skips nested pair",
    ),
    vim_oracle(
        "])",
        "Move to next unmatched )",
        "unmatched close paren skips nested pair",
    ),
    // Method motions deliberately deviate from Vim's brace heuristic (they
    // use tree-sitter function boundaries), so they carry Nevi regression
    // tests instead of oracle cases.
    // Quit and buffer keys cannot be oracle cases: quitting ends the nvim
    // snapshot and the harness runs a single scratch buffer.
    nevi_regression(
        "ZZ",
        "Save if modified and quit",
        "normal_zz_writes_modified_file_and_quits",
    ),
    nevi_regression(
        "ZQ",
        "Quit without saving",
        "normal_zq_quits_without_saving",
    ),
    nevi_regression(
        "<C-^>",
        "Switch to the alternate buffer",
        "normal_ctrl_caret_toggles_between_the_last_two_buffers",
    ),
    nevi_regression(
        "[b",
        "Go to the previous buffer",
        "bracket_b_cycles_buffers_with_a_count",
    ),
    nevi_regression(
        "]b",
        "Go to the next buffer",
        "bracket_b_cycles_buffers_with_a_count",
    ),
    vim_oracle(
        "[<Space>",
        "Add empty lines above the cursor line",
        "blank line above moves with the text",
    ),
    vim_oracle(
        "]<Space>",
        "Add empty lines below the cursor line",
        "blank line below",
    ),
    nevi_regression(
        "]m",
        "Move to next method/function start (tree-sitter)",
        "method_motion_jumps_between_function_starts",
    ),
    nevi_regression(
        "[m",
        "Move to previous method/function start (tree-sitter)",
        "method_motion_jumps_between_function_starts",
    ),
    nevi_regression(
        "]M",
        "Move to next method/function end (tree-sitter)",
        "method_motion_ends_land_on_closing_brace",
    ),
    nevi_regression(
        "[M",
        "Move to previous method/function end (tree-sitter)",
        "method_motion_ends_land_on_closing_brace",
    ),
    // Motions, marks, and jump/change-list batch.
    vim_oracle("{", "Move to previous paragraph", "paragraph backward"),
    vim_oracle("}", "Move to next paragraph", "paragraph forward"),
    vim_oracle("(", "Move to previous sentence", "sentence backward"),
    vim_oracle(")", "Move to next sentence", "sentence forward"),
    vim_oracle(
        "-",
        "Move to first non-blank of previous line",
        "first non blank of previous line",
    ),
    vim_oracle("{n}G", "Move to line n", "goto line with count"),
    vim_oracle("{n}go", "Go to byte n of the file", "go to byte"),
    vim_oracle("m{a-z}", "Set local mark", "jump to mark line"),
    vim_oracle("'{a-z}", "Jump to line of local mark", "jump to mark line"),
    vim_oracle(
        "<C-o>",
        "Jump to older position",
        "jump list older position",
    ),
    vim_oracle(
        "<C-i>",
        "Jump to newer position",
        "jump list newer position",
    ),
    vim_oracle(
        "''",
        "Jump to the line before the last jump",
        "jump back to line before last jump",
    ),
    vim_oracle(
        "g;",
        "Jump to older change position",
        "older change position",
    ),
    vim_oracle(
        "g,",
        "Jump to newer change position",
        "newer change position",
    ),
    vim_oracle(
        "'.",
        "Jump to the line of the last change",
        "line of last change",
    ),
    vim_oracle(
        "'^",
        "Jump to the line of the last insert",
        "line of last insert",
    ),
    vim_oracle(
        "gi",
        "Go to last insert position and enter insert mode",
        "go to last insert position and insert",
    ),
    vim_oracle(
        "gm",
        "Move to middle of the screen line",
        "gm on short line",
    ),
    vim_oracle("go", "Go to [count] byte of the buffer", "go to byte"),
    vim_oracle(
        "gj",
        "Move down by display line",
        "display line down without wrap",
    ),
    vim_oracle(
        "gk",
        "Move up by display line",
        "display line up without wrap",
    ),
    vim_oracle(
        "g0",
        "Move to start of display line",
        "display line start without wrap",
    ),
    vim_oracle(
        "g$",
        "Move to end of display line",
        "display line end without wrap",
    ),
    vim_oracle(
        "g^",
        "Move to first non-blank of display line",
        "display line first non blank without wrap",
    ),
    vim_oracle(
        "<CR>",
        "Move to first non-blank of next line",
        "enter next line first nonblank",
    ),
    vim_oracle("gg", "Move to start of file", "file top"),
    vim_oracle("G", "Move to end of file", "file bottom"),
    vim_oracle(
        "x",
        "Delete character under cursor",
        "delete first char on second line",
    ),
    vim_oracle("dd", "Delete current line", "delete current line"),
    vim_oracle("D", "Delete to end of line", "delete to line end"),
    vim_oracle("i", "Insert before cursor", "insert before cursor"),
    vim_oracle("I", "Insert at first non-blank", "insert at first nonblank"),
    vim_oracle("a", "Append after cursor", "append after cursor"),
    vim_oracle(
        "A",
        "Append at end of line",
        "append punctuation at line end",
    ),
    vim_oracle("o", "Open line below", "open line below"),
    vim_oracle("O", "Open line above", "open line above"),
    vim_oracle("dw", "Delete word with motion", "delete word"),
    vim_oracle(
        "cw",
        "Change word without trailing spaces",
        "change word excludes trailing spaces",
    ),
    vim_oracle(
        "cW",
        "Change big word without trailing spaces",
        "change big word excludes trailing spaces",
    ),
    vim_oracle("ciw", "Change inner word", "change inner word"),
    vim_oracle("cc", "Change current line", "change current line"),
    vim_oracle("C", "Change to end of line", "change to line end"),
    vim_oracle("yy", "Yank current line", "yank current line"),
    vim_oracle("Y", "Yank through line end", "yank to line end"),
    vim_oracle("p", "Paste after cursor", "paste after linewise yank"),
    vim_oracle("P", "Paste before cursor", "paste before linewise yank"),
    vim_oracle("de", "Delete through word end", "delete to word end"),
    vim_oracle(
        "db",
        "Delete to previous word start",
        "delete to previous word start",
    ),
    vim_oracle(
        "d$",
        "Delete through line end",
        "delete with line-end motion",
    ),
    vim_oracle("caw", "Change around word", "change around word"),
    // Macros, registers, insert-mode, and visual-basics batch.
    vim_oracle(
        "q{a-z}",
        "Record macro into register",
        "record and play macro",
    ),
    vim_oracle("q", "Stop recording", "record and play macro"),
    vim_oracle(
        "@{a-z}",
        "Play macro from register",
        "record and play macro",
    ),
    vim_oracle("@@", "Replay last executed macro", "replay last macro"),
    vim_oracle("{n}@{a-z}", "Play macro n times", "counted macro play"),
    vim_oracle("\"a", "Named registers", "named register yank and paste"),
    vim_oracle("\"A", "Append to named registers", "named register append"),
    vim_oracle(
        "\"_",
        "Black hole register",
        "black hole delete keeps unnamed register",
    ),
    vim_oracle(
        "\"0",
        "Last yank register",
        "register zero keeps last yank after delete",
    ),
    vim_oracle(
        "\".",
        "Last inserted text register",
        "last inserted text register",
    ),
    insert_oracle(
        "<C-[>",
        "Exit insert mode",
        "ctrl-bracket exits insert like escape",
    ),
    insert_oracle(
        "Backspace",
        "Delete character before cursor in insert",
        "insert backspace deletes typed chars",
    ),
    insert_oracle(
        "<C-w>",
        "Delete word before cursor in insert",
        "insert ctrl-w deletes word before cursor",
    ),
    insert_oracle(
        "<C-a>",
        "Insert previously inserted text",
        "insert ctrl-a repeats last inserted text",
    ),
    insert_oracle(
        "Ctrl+r {reg}",
        "Insert register contents",
        "insert ctrl-r pastes named register",
    ),
    insert_oracle(
        "<C-e>",
        "Insert the character below the cursor",
        "insert ctrl-e copies char from line below",
    ),
    insert_oracle(
        "<C-y>",
        "Insert the character above the cursor",
        "insert ctrl-y copies char from line above",
    ),
    vim_oracle(
        "<C-a>",
        "Add count to the number at or after the cursor",
        "increment number after cursor",
    ),
    vim_oracle(
        "<C-x>",
        "Subtract count from the number at or after the cursor",
        "decrement number after cursor",
    ),
    vim_oracle("v", "Character-wise visual mode", "visual charwise delete"),
    vim_oracle("V", "Line-wise visual mode", "visual linewise delete"),
    vim_oracle("<C-v>", "Block visual mode", "visual block delete"),
    vim_oracle("Esc", "Exit visual mode", "escape cancels visual selection"),
    vim_oracle(
        "gv",
        "Reselect last visual selection",
        "reselect last visual selection",
    ),
    // Operators pressed inside visual mode.
    visual_oracle("u", "Lowercase selection", "visual lowercase charwise"),
    visual_oracle("U", "Uppercase selection", "visual uppercase linewise"),
    visual_oracle("~", "Toggle case of selection", "visual toggle case block"),
    visual_oracle("gu", "Lowercase selection", "visual g-lowercase charwise"),
    visual_oracle("gU", "Uppercase selection", "visual g-uppercase block"),
    visual_oracle(
        "g~",
        "Toggle case of selection",
        "visual g-toggle case charwise",
    ),
    visual_oracle(
        "r{char}",
        "Replace every selected character",
        "visual replace block skips short lines",
    ),
    visual_oracle(
        "J",
        "Join selected lines with spaces",
        "visual join three lines",
    ),
    visual_oracle(
        "gJ",
        "Join selected lines without spaces",
        "visual join without spaces keeps whitespace",
    ),
    // `=` follows Nevi's own indenter rather than Vim's C-indenting, so it
    // is pinned natively instead of against the oracle.
    KeybindCoverage {
        mode: KeybindMode::Visual,
        key: "=",
        description: "Re-indent selected lines",
        kind: CoverageKind::NeviRegression,
        state: CoverageState::Protected {
            test_id: "visual_equals_reindents_selection_like_double_equals",
        },
    },
    // Text-object batch: one entry per documented object family.
    vim_oracle("iw", "Inner/around word objects", "delete inner word"),
    vim_oracle("iW", "Inner/around WORD objects", "delete inner big word"),
    vim_oracle(
        "i\"",
        "Inner/around double-quote objects",
        "change inner double quotes from before the string",
    ),
    vim_oracle(
        "i'",
        "Inner/around single-quote objects",
        "delete inner single quotes",
    ),
    vim_oracle(
        "i`",
        "Inner/around backtick objects",
        "delete inner backticks",
    ),
    vim_oracle(
        "i(",
        "Inner/around parentheses objects",
        "delete inner parens",
    ),
    vim_oracle(
        "ib",
        "Inner/around parentheses alias",
        "inner parens via b alias",
    ),
    vim_oracle(
        "i{",
        "Inner/around brace objects",
        "nested braces inner targets innermost",
    ),
    vim_oracle("iB", "Inner/around brace alias", "inner braces via B alias"),
    vim_oracle(
        "i[",
        "Inner/around bracket objects",
        "delete inner brackets",
    ),
    vim_oracle(
        "i<",
        "Inner/around angle bracket objects",
        "delete inner angle brackets",
    ),
    vim_oracle(
        "ip",
        "Inner/around paragraph objects",
        "delete inner paragraph",
    ),
    vim_oracle(
        "is",
        "Inner/around sentence objects",
        "delete inner sentence",
    ),
    vim_oracle("it", "Inner/around tag objects", "delete inner tag"),
    // Editing-core batch: bare operators, substitute family, paste
    // variants, replace, case changing, joins.
    vim_oracle("d", "Delete with a motion", "delete word"),
    vim_oracle("c", "Change with a motion", "change inner word"),
    vim_oracle("y", "Yank with a motion", "yank to line end"),
    vim_oracle(
        "X",
        "Delete character before cursor",
        "delete char before cursor",
    ),
    vim_oracle("s", "Substitute character and insert", "substitute char"),
    vim_oracle("S", "Substitute entire line", "substitute line"),
    vim_oracle(
        "gp",
        "Paste after, cursor after pasted text",
        "linewise paste after and move",
    ),
    vim_oracle(
        "]p",
        "Paste after, matching the current line's indent",
        "bracket p pastes below with the current indent",
    ),
    vim_oracle(
        "[p",
        "Paste before, matching the current line's indent",
        "bracket open p pastes above with the current indent",
    ),
    vim_oracle(
        "[P",
        "Paste before with adjusted indent, capital alias of [p",
        "bracket open P pastes above with the current indent",
    ),
    vim_oracle(
        "]P",
        "Paste before with adjusted indent, close-bracket alias of [p",
        "bracket close P pastes above with the current indent",
    ),
    vim_oracle(
        "gP",
        "Paste before, cursor after pasted text",
        "linewise paste before and move",
    ),
    vim_oracle(
        "r{char}",
        "Replace exactly one character",
        "replace character",
    ),
    vim_oracle(
        "R",
        "Enter replace mode",
        "counted replace mode repeats inserted text",
    ),
    vim_oracle(
        "~",
        "Toggle case of character under cursor",
        "toggle case single char",
    ),
    vim_oracle(
        "gu{motion}",
        "Lowercase with a motion",
        "lowercase to word end",
    ),
    vim_oracle("guu", "Lowercase entire line", "lowercase entire line"),
    vim_oracle(
        "gU{motion}",
        "Uppercase with a motion",
        "uppercase to word end",
    ),
    vim_oracle("gUU", "Uppercase entire line", "uppercase entire line"),
    vim_oracle(
        "g~{motion}",
        "Toggle case with a motion",
        "toggle case to word end",
    ),
    vim_oracle(
        "g~~",
        "Toggle case of entire line",
        "toggle case entire line",
    ),
    vim_oracle(".", "Repeat the last change", "repeat char delete"),
    vim_oracle("J", "Join lines with a space", "join lines with space"),
    vim_oracle(
        "gJ",
        "Join lines without a space",
        "join without added space",
    ),
    vim_oracle("u", "Undo latest change", "undo insert"),
    vim_oracle("<C-r>", "Redo latest undone change", "redo insert"),
    KeybindCoverage {
        mode: KeybindMode::Leader,
        key: "<leader>j",
        description: "Start labeled jump navigation",
        kind: CoverageKind::NeviRegression,
        state: CoverageState::Protected {
            test_id: "labeled_jump_jumps_to_selected_visible_match",
        },
    },
    KeybindCoverage {
        mode: KeybindMode::Leader,
        key: "<leader>fk",
        description: "Open searchable keymap picker",
        kind: CoverageKind::ConfigMapping,
        state: CoverageState::Protected {
            test_id: "default_leader_includes_keymaps_picker",
        },
    },
    vim_oracle("W", "Move to start of next WORD", "big word forward"),
    vim_oracle("B", "Move to start of previous WORD", "big word backward"),
    vim_oracle("E", "Move to end of WORD", "big word end"),
    vim_oracle("ge", "Move to end of previous word", "previous word end"),
    vim_oracle(
        "gE",
        "Move to end of previous WORD",
        "previous big word end",
    ),
    vim_oracle("%", "Jump to matching bracket", "matching bracket"),
    vim_oracle("H", "Move to top of visible screen", "screen top"),
    vim_oracle("M", "Move to middle of visible screen", "screen middle"),
    vim_oracle("L", "Move to bottom of visible screen", "screen bottom"),
    vim_oracle(
        "f{char}",
        "Find character forward on current line",
        "find char forward",
    ),
    vim_oracle(
        "F{char}",
        "Find character backward on current line",
        "find char backward",
    ),
    vim_oracle(
        "t{char}",
        "Move before character forward on current line",
        "till char forward",
    ),
    vim_oracle(
        "T{char}",
        "Move after character backward on current line",
        "till char backward",
    ),
    vim_oracle(
        ";",
        "Repeat latest find-character search",
        "repeat find char",
    ),
    vim_oracle(
        ",",
        "Repeat latest find-character search in reverse",
        "reverse repeat find char",
    ),
    vim_oracle("<C-f>", "Scroll page down", "page down"),
    vim_oracle("<C-b>", "Scroll page up", "page up"),
    vim_oracle("<C-d>", "Scroll half page down", "half page down"),
    vim_oracle("<C-u>", "Scroll half page up", "half page up"),
    vim_oracle("<C-e>", "Scroll view down one line", "line scroll down"),
    vim_oracle("<C-y>", "Scroll view up one line", "line scroll up"),
    vim_oracle("zz", "Center cursor line", "center cursor line"),
    vim_oracle("zt", "Move cursor line to top", "cursor line to top"),
    vim_oracle("zb", "Move cursor line to bottom", "cursor line to bottom"),
    vim_oracle(
        "z<CR>",
        "Cursor line to top, cursor to first non-blank",
        "z enter puts line at top and goes to first non-blank",
    ),
    vim_oracle(
        "z.",
        "Center cursor line, cursor to first non-blank",
        "z dot centers line and goes to first non-blank",
    ),
    vim_oracle(
        "z-",
        "Cursor line to bottom, cursor to first non-blank",
        "z minus puts line at bottom and goes to first non-blank",
    ),
    // The horizontal offset is invisible to the oracle; zl is pinned by its
    // cursor side effect, the rest by editor-level tests.
    vim_oracle(
        "zl",
        "Scroll the view right, count columns",
        "zl drags the cursor to the new left edge",
    ),
    nevi_regression(
        "zh",
        "Scroll the view left, count columns",
        "scroll_columns_left_pulls_a_cursor_past_the_right_edge_back_in",
    ),
    nevi_regression(
        "zL",
        "Scroll the view right half a screen",
        "half_screen_column_scroll_uses_half_the_text_width",
    ),
    nevi_regression(
        "zH",
        "Scroll the view left half a screen",
        "half_screen_column_scroll_uses_half_the_text_width",
    ),
    nevi_regression(
        "zs",
        "Scroll so the cursor is at the left edge",
        "zs_and_ze_put_the_cursor_at_the_screen_edges",
    ),
    nevi_regression(
        "ze",
        "Scroll so the cursor is at the right edge",
        "zs_and_ze_put_the_cursor_at_the_screen_edges",
    ),
    // Search-family batch. The prompt-editing keys (Ctrl+b/e/w/u, Ctrl+r,
    // Up/Down) share key spellings with insert/scroll entries above, so they
    // stay pinned by oracle cases without their own inventory rows.
    vim_oracle("/", "Search forward", "search forward lands on match start"),
    vim_oracle(
        "?",
        "Search backward",
        "search backward lands on previous match",
    ),
    vim_oracle("n", "Go to next match", "next match"),
    vim_oracle(
        "N",
        "Go to previous match",
        "previous match reverses direction",
    ),
    vim_oracle(
        "*",
        "Search word under cursor forward",
        "star searches word forward",
    ),
    vim_oracle(
        "#",
        "Search word under cursor backward",
        "hash searches word backward",
    ),
    vim_oracle(
        "g*",
        "Search word under cursor forward, also inside longer words",
        "g-star finds match inside longer word",
    ),
    vim_oracle(
        "g#",
        "Search word under cursor backward, also inside longer words",
        "g-hash finds match inside longer word backward",
    ),
    vim_oracle(
        "gn",
        "Search forward and select match",
        "gn selects next match from outside",
    ),
    vim_oracle(
        "gN",
        "Search backward and select match",
        "gN selects match backward",
    ),
    KeybindCoverage {
        mode: KeybindMode::Normal,
        key: "1-9 (start screen)",
        description: "Open the numbered start screen entry",
        kind: CoverageKind::NeviRegression,
        state: CoverageState::Protected {
            test_id: "dashboard_digit_opens_numbered_entry_via_key",
        },
    },
    KeybindCoverage {
        mode: KeybindMode::Normal,
        key: "h1-h9 (start screen)",
        description: "Jump to the numbered harpoon slot from the start screen",
        kind: CoverageKind::NeviRegression,
        state: CoverageState::Protected {
            test_id: "dashboard_h_digit_opens_harpoon_slot",
        },
    },
];

const fn vim_oracle(
    key: &'static str,
    description: &'static str,
    oracle_case: &'static str,
) -> KeybindCoverage {
    KeybindCoverage {
        mode: KeybindMode::Normal,
        key,
        description,
        kind: CoverageKind::VimOracle,
        state: CoverageState::Protected {
            test_id: oracle_case,
        },
    }
}

/// Nevi-owned behavior protected by a focused regression test rather than an
/// oracle case (used where we deliberately deviate from Vim).
/// Same as `vim_oracle`, for keys that act in insert mode. Kept separate so a
/// key like `<C-a>` can be inventoried once per mode it means something in.
const fn insert_oracle(
    key: &'static str,
    description: &'static str,
    oracle_case: &'static str,
) -> KeybindCoverage {
    KeybindCoverage {
        mode: KeybindMode::Insert,
        key,
        description,
        kind: CoverageKind::VimOracle,
        state: CoverageState::Protected {
            test_id: oracle_case,
        },
    }
}

/// Same as `vim_oracle`, for keys pressed inside visual mode, where `J`,
/// `r`, and the case operators mean something different from normal mode.
const fn visual_oracle(
    key: &'static str,
    description: &'static str,
    oracle_case: &'static str,
) -> KeybindCoverage {
    KeybindCoverage {
        mode: KeybindMode::Visual,
        key,
        description,
        kind: CoverageKind::VimOracle,
        state: CoverageState::Protected {
            test_id: oracle_case,
        },
    }
}

const fn nevi_regression(
    key: &'static str,
    description: &'static str,
    test_id: &'static str,
) -> KeybindCoverage {
    KeybindCoverage {
        mode: KeybindMode::Normal,
        key,
        description,
        kind: CoverageKind::NeviRegression,
        state: CoverageState::Protected { test_id },
    }
}

#[allow(dead_code)]
const fn needs_oracle(key: &'static str, description: &'static str) -> KeybindCoverage {
    KeybindCoverage {
        mode: KeybindMode::Normal,
        key,
        description,
        kind: CoverageKind::VimOracle,
        state: CoverageState::NeedsCoverage {
            reason: "documented Vim/Neovim default without a dedicated oracle case yet",
        },
    }
}

pub(crate) fn coverage_entries() -> &'static [KeybindCoverage] {
    KEYBIND_COVERAGE
}

pub(crate) fn coverage_for(mode: KeybindMode, key: &str) -> Option<&'static KeybindCoverage> {
    coverage_entries()
        .iter()
        .find(|entry| entry.mode == mode && entry.key == key)
}

pub(crate) fn uncovered_entries() -> Vec<&'static KeybindCoverage> {
    coverage_entries()
        .iter()
        .filter(|entry| matches!(entry.state, CoverageState::NeedsCoverage { .. }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        CoverageKind, CoverageState, KeybindMode, coverage_entries, coverage_for, uncovered_entries,
    };
    use crate::vim_oracle;
    use std::collections::HashSet;

    #[test]
    fn vim_oracle_cases_can_be_referenced_by_coverage_inventory() {
        assert!(vim_oracle::has_oracle_case("word forward"));
    }

    #[test]
    fn inventory_tracks_high_value_supported_default_keybinds() {
        let required = [
            (KeybindMode::Normal, "h"),
            (KeybindMode::Normal, "j"),
            (KeybindMode::Normal, "k"),
            (KeybindMode::Normal, "l"),
            (KeybindMode::Normal, "w"),
            (KeybindMode::Normal, "b"),
            (KeybindMode::Normal, "e"),
            (KeybindMode::Normal, "0"),
            (KeybindMode::Normal, "^"),
            (KeybindMode::Normal, "$"),
            (KeybindMode::Normal, "gg"),
            (KeybindMode::Normal, "G"),
            (KeybindMode::Normal, "x"),
            (KeybindMode::Normal, "dd"),
            (KeybindMode::Normal, "D"),
            (KeybindMode::Normal, "i"),
            (KeybindMode::Normal, "I"),
            (KeybindMode::Normal, "a"),
            (KeybindMode::Normal, "A"),
            (KeybindMode::Normal, "u"),
            (KeybindMode::Normal, "<C-r>"),
            (KeybindMode::Leader, "<leader>j"),
            (KeybindMode::Leader, "<leader>fk"),
        ];

        for (mode, key) in required {
            assert!(
                coverage_for(mode, key).is_some(),
                "missing coverage inventory entry for {mode:?} `{key}`"
            );
        }
    }

    #[test]
    fn inventory_entries_have_complete_metadata_and_explicit_status() {
        for entry in coverage_entries() {
            assert!(!entry.key.is_empty(), "entry has empty key: {entry:?}");
            assert!(
                !entry.description.is_empty(),
                "entry has empty description: {entry:?}"
            );

            match entry.state {
                CoverageState::Protected { test_id } => assert!(
                    !test_id.is_empty(),
                    "protected entry needs a test id: {entry:?}"
                ),
                CoverageState::NeedsCoverage { reason } => assert!(
                    !reason.is_empty(),
                    "uncovered entry needs an explicit reason: {entry:?}"
                ),
            }
        }
    }

    #[test]
    fn inventory_entries_are_unique_by_mode_and_key() {
        let mut seen = HashSet::new();

        for entry in coverage_entries() {
            assert!(
                seen.insert((entry.mode, entry.key)),
                "duplicate coverage inventory entry for {:?} `{}`",
                entry.mode,
                entry.key
            );
        }
    }

    #[test]
    fn vim_oracle_entries_reference_real_oracle_cases() {
        for entry in coverage_entries() {
            if entry.kind != CoverageKind::VimOracle {
                continue;
            }

            let CoverageState::Protected { test_id } = entry.state else {
                continue;
            };

            assert!(
                vim_oracle::has_oracle_case(test_id),
                "coverage entry {:?} references missing oracle case `{test_id}`",
                entry
            );
        }
    }

    #[test]
    fn tracked_inventory_has_no_current_oracle_gaps() {
        let gaps = uncovered_entries();

        assert!(gaps.is_empty(), "unprotected Vim defaults: {gaps:#?}");
    }

    #[test]
    fn word_motion_defaults_are_oracle_covered() {
        let expected = [
            ("W", "big word forward"),
            ("B", "big word backward"),
            ("E", "big word end"),
            ("ge", "previous word end"),
            ("gE", "previous big word end"),
        ];

        for (key, oracle_case) in expected {
            let entry = coverage_for(KeybindMode::Normal, key)
                .unwrap_or_else(|| panic!("missing coverage entry for `{key}`"));

            assert_eq!(entry.kind, CoverageKind::VimOracle);
            assert_eq!(
                entry.state,
                CoverageState::Protected {
                    test_id: oracle_case,
                },
                "`{key}` should be protected by oracle case `{oracle_case}`"
            );
        }
    }

    #[test]
    fn high_use_editing_operators_are_oracle_covered() {
        let expected = [
            ("cc", "change current line"),
            ("C", "change to line end"),
            ("yy", "yank current line"),
            ("Y", "yank to line end"),
            ("p", "paste after linewise yank"),
            ("P", "paste before linewise yank"),
            ("de", "delete to word end"),
            ("db", "delete to previous word start"),
            ("d$", "delete with line-end motion"),
            ("caw", "change around word"),
        ];

        for (key, oracle_case) in expected {
            let entry = coverage_for(KeybindMode::Normal, key)
                .unwrap_or_else(|| panic!("missing coverage entry for `{key}`"));

            assert_eq!(entry.kind, CoverageKind::VimOracle);
            assert_eq!(
                entry.state,
                CoverageState::Protected {
                    test_id: oracle_case,
                },
                "`{key}` should be protected by oracle case `{oracle_case}`"
            );
        }
    }

    #[test]
    fn find_char_defaults_are_oracle_covered() {
        let expected = [
            ("f{char}", "find char forward"),
            ("F{char}", "find char backward"),
            ("t{char}", "till char forward"),
            ("T{char}", "till char backward"),
            (";", "repeat find char"),
            (",", "reverse repeat find char"),
        ];

        for (key, oracle_case) in expected {
            let entry = coverage_for(KeybindMode::Normal, key)
                .unwrap_or_else(|| panic!("missing coverage entry for `{key}`"));

            assert_eq!(entry.kind, CoverageKind::VimOracle);
            assert_eq!(
                entry.state,
                CoverageState::Protected {
                    test_id: oracle_case,
                },
                "`{key}` should be protected by oracle case `{oracle_case}`"
            );
        }
    }

    #[test]
    fn matching_bracket_default_is_oracle_covered() {
        let entry = coverage_for(KeybindMode::Normal, "%")
            .expect("missing coverage entry for matching-bracket motion");

        assert_eq!(entry.kind, CoverageKind::VimOracle);
        assert_eq!(
            entry.state,
            CoverageState::Protected {
                test_id: "matching bracket",
            },
            "`%` should be protected by the matching-bracket oracle case"
        );
    }

    #[test]
    fn screen_position_defaults_are_oracle_covered() {
        let expected = [
            ("H", "screen top"),
            ("M", "screen middle"),
            ("L", "screen bottom"),
        ];

        for (key, oracle_case) in expected {
            let entry = coverage_for(KeybindMode::Normal, key)
                .unwrap_or_else(|| panic!("missing coverage entry for `{key}`"));

            assert_eq!(entry.kind, CoverageKind::VimOracle);
            assert_eq!(
                entry.state,
                CoverageState::Protected {
                    test_id: oracle_case,
                },
                "`{key}` should be protected by oracle case `{oracle_case}`"
            );
        }
    }

    #[test]
    fn viewport_position_defaults_are_oracle_covered() {
        let expected = [
            ("zz", "center cursor line"),
            ("zt", "cursor line to top"),
            ("zb", "cursor line to bottom"),
        ];

        for (key, oracle_case) in expected {
            let entry = coverage_for(KeybindMode::Normal, key)
                .unwrap_or_else(|| panic!("missing coverage entry for `{key}`"));

            assert_eq!(entry.kind, CoverageKind::VimOracle);
            assert_eq!(
                entry.state,
                CoverageState::Protected {
                    test_id: oracle_case,
                },
                "`{key}` should be protected by oracle case `{oracle_case}`"
            );
        }
    }

    #[test]
    fn page_scroll_defaults_are_oracle_covered() {
        let expected = [
            ("<C-f>", "page down"),
            ("<C-b>", "page up"),
            ("<C-d>", "half page down"),
            ("<C-u>", "half page up"),
        ];

        for (key, oracle_case) in expected {
            let entry = coverage_for(KeybindMode::Normal, key)
                .unwrap_or_else(|| panic!("missing coverage entry for `{key}`"));

            assert_eq!(entry.kind, CoverageKind::VimOracle);
            assert_eq!(
                entry.state,
                CoverageState::Protected {
                    test_id: oracle_case,
                },
                "`{key}` should be protected by oracle case `{oracle_case}`"
            );
        }
    }
}
