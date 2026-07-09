#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum KeybindMode {
    Normal,
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
    vim_oracle("a", "Append after cursor", "append after cursor"),
    vim_oracle("o", "Open line below", "open line below"),
    vim_oracle("O", "Open line above", "open line above"),
    vim_oracle("dw", "Delete word with motion", "delete word"),
    vim_oracle("ciw", "Change inner word", "change inner word"),
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
    needs_oracle("%", "Jump to matching bracket"),
    needs_oracle("H", "Move to top of visible screen"),
    needs_oracle("M", "Move to middle of visible screen"),
    needs_oracle("L", "Move to bottom of visible screen"),
    needs_oracle("f{char}", "Find character forward on current line"),
    needs_oracle("F{char}", "Find character backward on current line"),
    needs_oracle("t{char}", "Move before character forward on current line"),
    needs_oracle("T{char}", "Move after character backward on current line"),
    needs_oracle(";", "Repeat latest find-character search"),
    needs_oracle(",", "Repeat latest find-character search in reverse"),
    needs_oracle("<C-f>", "Scroll page down"),
    needs_oracle("<C-b>", "Scroll page up"),
    needs_oracle("<C-d>", "Scroll half page down"),
    needs_oracle("<C-u>", "Scroll half page up"),
    needs_oracle("zz", "Center cursor line"),
    needs_oracle("zt", "Move cursor line to top"),
    needs_oracle("zb", "Move cursor line to bottom"),
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
            (KeybindMode::Normal, "a"),
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
    fn known_gaps_are_explicit_instead_of_silent() {
        let gaps = uncovered_entries();

        assert!(
            gaps.iter()
                .any(|entry| entry.mode == KeybindMode::Normal && entry.key == "%"),
            "`%` should stay visible as a tracked Vim parity gap until oracle-covered"
        );
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
}
