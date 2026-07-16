use super::OracleCase;

/// Replace cases cover both the one-shot `r` command and interactive `R` mode.
/// The oracle compares text, cursor position, viewport, and final mode.
pub(super) const REPLACE_CASES: &[OracleCase] = &[
    OracleCase {
        name: "replace character",
        initial_text: "abc\n",
        keys: "rx",
    },
    OracleCase {
        name: "counted replace character",
        initial_text: "abcdef\n",
        keys: "3rx",
    },
    OracleCase {
        name: "counted replace character at exact line boundary",
        initial_text: "abc\n",
        keys: "3rx",
    },
    OracleCase {
        name: "counted replace character past line boundary",
        initial_text: "abc\n",
        keys: "4rx",
    },
    OracleCase {
        name: "replace character with newline",
        initial_text: "abc\n",
        keys: "lr<CR>",
    },
    OracleCase {
        name: "counted replace character with newlines",
        initial_text: "abcd\n",
        keys: "2r<CR>",
    },
    OracleCase {
        name: "replace multibyte characters",
        initial_text: "🙂éz\n",
        keys: "2rX",
    },
    OracleCase {
        name: "undo counted replace character",
        initial_text: "abcdef\n",
        keys: "l3rxu",
    },
    OracleCase {
        name: "redo counted replace character",
        initial_text: "abcdef\n",
        keys: "l3rxu<C-r>",
    },
    OracleCase {
        name: "replace mode overwrites characters",
        initial_text: "abcdef\n",
        keys: "llRXY<Esc>",
    },
    OracleCase {
        name: "replace mode escape without edits",
        initial_text: "abcdef\n",
        keys: "llR<Esc>",
    },
    OracleCase {
        name: "replace mode extends past line end",
        initial_text: "abc\n",
        keys: "llRXYZ<Esc>",
    },
    OracleCase {
        name: "replace mode backspace restores original text",
        initial_text: "abc\n",
        keys: "RXY<BS><Esc>",
    },
    OracleCase {
        name: "replace mode backspace removes text appended past line end",
        initial_text: "abc\n",
        keys: "$RXY<BS><Esc>",
    },
    OracleCase {
        name: "replace mode inserts newline and continues",
        initial_text: "abc\ndef\n",
        keys: "lRXY<CR>Z<Esc>",
    },
    OracleCase {
        name: "replace mode backspace removes inserted newline",
        initial_text: "abc\ndef\n",
        keys: "lRXY<CR><BS><Esc>",
    },
    OracleCase {
        name: "replace mode movement invalidates backspace restore history",
        initial_text: "abcdef\n",
        keys: "RXY<Left><BS><Esc>",
    },
    OracleCase {
        name: "replace mode vertical movement invalidates backspace restore history",
        initial_text: "abcdef\nuvwxyz\n",
        keys: "RXY<Down><BS><Esc>",
    },
    OracleCase {
        name: "replace mode backspace crosses a line boundary after movement",
        initial_text: "abc\nuvwxyz\n",
        keys: "R<Down><BS><Esc>",
    },
    OracleCase {
        name: "replace mode continues replacing after backspace crosses a line boundary",
        initial_text: "abc\nuvwxyz\n",
        keys: "jR<BS>X<Esc>",
    },
    OracleCase {
        name: "replace mode line-boundary backspace cancels counted replay",
        initial_text: "abc\nuvwxyz\n",
        keys: "j2R<BS>X<Esc>",
    },
    OracleCase {
        name: "counted replace mode repeats inserted text",
        initial_text: "abcdefghij\n",
        keys: "2RXY<Esc>",
    },
    OracleCase {
        name: "counted replace mode repeats newline",
        initial_text: "abc\ndef\n",
        keys: "2RZ<CR><Esc>",
    },
    OracleCase {
        name: "replace mode movement cancels counted replay",
        initial_text: "abcdefghi\n",
        keys: "2R<Right>X<Esc>",
    },
    OracleCase {
        name: "replace mode vertical movement cancels counted replay",
        initial_text: "abcdefghi\nuvwxyz\n",
        keys: "2R<Down>X<Esc>",
    },
    OracleCase {
        name: "undo replace mode session",
        initial_text: "abcdef\n",
        keys: "llRXY<Esc>u",
    },
    OracleCase {
        name: "redo replace mode session",
        initial_text: "abcdef\n",
        keys: "llRXY<Esc>u<C-r>",
    },
];
