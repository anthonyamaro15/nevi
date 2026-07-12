use super::OracleCase;

/// Editing cases verify both the resulting text and Vim's final cursor/mode.
/// Yank behavior is made observable by pasting the captured register contents.
pub(super) const EDITING_CASES: &[OracleCase] = &[
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
    OracleCase {
        name: "counted char delete",
        initial_text: "abcdef\n",
        keys: "4x",
    },
    OracleCase {
        name: "counted line delete",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2dd",
    },
    OracleCase {
        name: "delete to line end",
        initial_text: "alpha beta\n",
        keys: "wD",
    },
    OracleCase {
        name: "insert before cursor",
        initial_text: "alpha\n",
        keys: "iX<Esc>",
    },
    OracleCase {
        name: "append after cursor",
        initial_text: "alpha\n",
        keys: "aX<Esc>",
    },
    OracleCase {
        name: "open line below",
        initial_text: "alpha\n",
        keys: "ochild<Esc>",
    },
    OracleCase {
        name: "open line above",
        initial_text: "alpha\n",
        keys: "Oparent<Esc>",
    },
    OracleCase {
        name: "delete word",
        initial_text: "alpha beta\n",
        keys: "dw",
    },
    OracleCase {
        name: "delete enter motion",
        initial_text: "zero\n    one\n  two\nthree\n",
        keys: "d<CR>",
    },
    OracleCase {
        name: "change inner word",
        initial_text: "alpha beta\n",
        keys: "ciwdone<Esc>",
    },
    OracleCase {
        name: "change current line",
        initial_text: "  alpha beta\nsecond\n",
        keys: "ccreplacement<Esc>",
    },
    OracleCase {
        name: "change current line register shape",
        initial_text: "  alpha beta\nsecond\n",
        keys: "ccreplacement<Esc>p",
    },
    OracleCase {
        name: "counted line change",
        initial_text: "  alpha beta\nsecond\nthird\n",
        keys: "2ccreplacement<Esc>",
    },
    OracleCase {
        name: "undo indented line change",
        initial_text: "  alpha beta\nsecond\n",
        keys: "ccreplacement<Esc>u",
    },
    OracleCase {
        name: "redo indented line change",
        initial_text: "  alpha beta\nsecond\n",
        keys: "ccreplacement<Esc>u<C-r>",
    },
    OracleCase {
        name: "change to line end",
        initial_text: "alpha beta gamma\n",
        keys: "wCreplaced<Esc>",
    },
    OracleCase {
        name: "change to line end register shape",
        initial_text: "alpha beta gamma\n",
        keys: "wCreplaced<Esc>p",
    },
    OracleCase {
        name: "counted change to line end",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2Creplaced<Esc>",
    },
    OracleCase {
        name: "counted change to line end past eof",
        initial_text: "alpha\n",
        keys: "2C",
    },
    OracleCase {
        name: "yank current line",
        initial_text: "alpha\nbeta\n",
        keys: "yyp",
    },
    OracleCase {
        name: "linewise yank without final newline",
        initial_text: "abc",
        keys: "yypp",
    },
    OracleCase {
        name: "yank to line end",
        initial_text: "alpha\nbeta\n",
        keys: "YP",
    },
    OracleCase {
        name: "counted yank to line end",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2YP",
    },
    OracleCase {
        name: "counted yank to line end past eof",
        initial_text: "alpha\n",
        keys: "x2Yp",
    },
    OracleCase {
        name: "paste after linewise yank",
        initial_text: "alpha\nbeta\n",
        keys: "ddp",
    },
    OracleCase {
        name: "paste before linewise yank",
        initial_text: "alpha\nbeta\n",
        keys: "ddP",
    },
    OracleCase {
        name: "delete to word end",
        initial_text: "alpha beta gamma\n",
        keys: "wde",
    },
    OracleCase {
        name: "delete to word end register shape",
        initial_text: "alpha beta gamma\n",
        keys: "wdep",
    },
    OracleCase {
        name: "delete to previous word start",
        initial_text: "alpha beta gamma\n",
        keys: "wdb",
    },
    OracleCase {
        name: "delete to previous word start register shape",
        initial_text: "alpha beta gamma\n",
        keys: "wdbp",
    },
    OracleCase {
        name: "delete with line-end motion",
        initial_text: "alpha beta gamma\n",
        keys: "wd$",
    },
    OracleCase {
        name: "counted delete with line-end motion",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2d$",
    },
    OracleCase {
        name: "counted delete with line-end motion past eof",
        initial_text: "alpha\n",
        keys: "2d$",
    },
    OracleCase {
        name: "counted delete line-end register",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2d$p",
    },
    OracleCase {
        name: "change around word",
        initial_text: "alpha beta gamma\n",
        keys: "wcawdone<Esc>",
    },
    OracleCase {
        name: "change around word register shape",
        initial_text: "alpha beta gamma\n",
        keys: "wcawdone<Esc>p",
    },
    OracleCase {
        name: "counted change around word before operator",
        initial_text: "alpha beta gamma delta\n",
        keys: "w2cawdone<Esc>",
    },
    OracleCase {
        name: "counted change around word after operator",
        initial_text: "alpha beta gamma delta\n",
        keys: "wc2awdone<Esc>",
    },
    OracleCase {
        name: "counted line yank",
        initial_text: "alpha\nbeta\ngamma\ndelta\n",
        keys: "3yyp",
    },
    OracleCase {
        name: "counted linewise paste",
        initial_text: "alpha\nbeta\n",
        keys: "dd2p",
    },
    OracleCase {
        name: "undo counted linewise paste",
        initial_text: "alpha\nbeta\n",
        keys: "dd2pu",
    },
    OracleCase {
        name: "redo counted linewise paste",
        initial_text: "alpha\nbeta\n",
        keys: "dd2pu<C-r>",
    },
    OracleCase {
        name: "undo counted linewise paste before",
        initial_text: "alpha\nbeta\n",
        keys: "dd2Pu",
    },
    OracleCase {
        name: "counted multiline linewise paste",
        initial_text: "alpha\nbeta\ngamma\ndelta\n",
        keys: "3yy2p",
    },
    OracleCase {
        name: "counted empty linewise paste",
        initial_text: "\nalpha\n",
        keys: "yy2p",
    },
    OracleCase {
        name: "counted characterwise paste",
        initial_text: "abcd\n",
        keys: "x2p",
    },
    OracleCase {
        name: "undo counted characterwise paste",
        initial_text: "abcd\n",
        keys: "x2pu",
    },
    OracleCase {
        name: "redo counted characterwise paste",
        initial_text: "abcd\n",
        keys: "x2pu<C-r>",
    },
    OracleCase {
        name: "counted characterwise paste before",
        initial_text: "abcd\n",
        keys: "x2P",
    },
    OracleCase {
        name: "counted characterwise paste after and move",
        initial_text: "abcd\n",
        keys: "x2gp",
    },
    OracleCase {
        name: "multiline characterwise paste after and move",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2Ygp",
    },
    OracleCase {
        name: "counted characterwise paste before and move",
        initial_text: "abcd\n",
        keys: "x2gP",
    },
];
