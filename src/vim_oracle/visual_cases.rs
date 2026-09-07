use super::OracleCase;

/// Visual-mode operator cases, verified against real Neovim: the case
/// operators in both spellings (`u`/`U`/`~` and `gu`/`gU`/`g~`), `r{char}`,
/// and `J`/`gJ`, across charwise, linewise, and blockwise selections. The
/// cursor position after each operator is part of the snapshot, so these
/// also pin where Vim leaves the cursor.
pub(super) const VISUAL_CASES: &[OracleCase] = &[
    // Case operators.
    OracleCase {
        name: "visual lowercase charwise",
        initial_text: "ABC DEF\n",
        keys: "lvlu",
    },
    OracleCase {
        name: "visual uppercase linewise",
        initial_text: "abc\ndef\nghi\n",
        keys: "jVjU",
    },
    OracleCase {
        name: "visual uppercase linewise from mid column",
        initial_text: "abc\ndef\n",
        keys: "lVjU",
    },
    OracleCase {
        name: "visual toggle case block",
        initial_text: "aBc\nDeF\nghi\n",
        keys: "l<C-v>j~",
    },
    OracleCase {
        name: "visual toggle case across lines",
        initial_text: "abc\ndef\n",
        keys: "lvj~",
    },
    OracleCase {
        name: "visual g-lowercase charwise",
        initial_text: "ABC DEF\n",
        keys: "wvlgu",
    },
    OracleCase {
        name: "visual g-uppercase linewise",
        initial_text: "abc\ndef\n",
        keys: "VjgU",
    },
    OracleCase {
        name: "visual g-toggle case charwise",
        initial_text: "aBc dEf\n",
        keys: "v$g~",
    },
    OracleCase {
        name: "visual g-uppercase block",
        initial_text: "abc\ndef\nghi\n",
        keys: "l<C-v>jlgU",
    },
    // r{char} replaces every selected character.
    OracleCase {
        name: "visual replace charwise",
        initial_text: "abcdef\n",
        keys: "lvlrx",
    },
    OracleCase {
        name: "visual replace across lines keeps newlines",
        initial_text: "abc\ndef\n",
        keys: "lvjrx",
    },
    OracleCase {
        name: "visual replace linewise",
        initial_text: "ab cd\nef\nx\n",
        keys: "Vjr-",
    },
    OracleCase {
        name: "visual replace block",
        initial_text: "abcd\nefgh\nijkl\n",
        keys: "l<C-v>jlrx",
    },
    OracleCase {
        name: "visual replace block skips short lines",
        initial_text: "abcd\nef\nijkl\n",
        keys: "ll<C-v>jjlrx",
    },
    OracleCase {
        name: "visual replace with multibyte char",
        initial_text: "abc\n",
        keys: "vlré",
    },
    OracleCase {
        name: "visual replace then undo",
        initial_text: "abcdef\n",
        keys: "vllrxu",
    },
    // J and gJ join the selected lines, at least two.
    OracleCase {
        name: "visual join two lines",
        initial_text: "one\ntwo\nthree\n",
        keys: "VjJ",
    },
    OracleCase {
        name: "visual join three lines",
        initial_text: "one\ntwo\nthree\nfour\n",
        keys: "VjjJ",
    },
    OracleCase {
        name: "visual join single line selection joins with next",
        initial_text: "one\ntwo\nthree\n",
        keys: "VJ",
    },
    OracleCase {
        name: "visual join charwise across lines",
        initial_text: "one\ntwo\nthree\n",
        keys: "lvjJ",
    },
    OracleCase {
        name: "visual join strips leading whitespace",
        initial_text: "one\n    two\n",
        keys: "VjJ",
    },
    OracleCase {
        name: "visual join without spaces keeps whitespace",
        initial_text: "one\n    two\n",
        keys: "VjgJ",
    },
    OracleCase {
        name: "visual join on last line does nothing",
        initial_text: "one\ntwo\n",
        keys: "jVJ",
    },
    OracleCase {
        name: "visual join then undo",
        initial_text: "one\ntwo\nthree\n",
        keys: "VjjJu",
    },
];
