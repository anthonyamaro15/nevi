use super::OracleCase;

/// The special marks, verified against real Neovim. `'<` `'>` `` `< `` `` `> ``
/// hold the last Visual selection in buffer order; a linewise selection
/// spans whole lines, so its exact end clamps to the line end. `'[` `']`
/// `` `[ `` `` `] `` hold the last changed or yanked text: a yank or put
/// brackets the text, a delete leaves both at the deleted spot, an insert
/// runs from where typing started, and loading a file brackets the whole
/// buffer.
pub(super) const MARK_CASES: &[OracleCase] = &[
    // Last Visual selection.
    OracleCase {
        name: "visual start mark goes to the first line of the selection",
        initial_text: "alpha\n  beta\n  gamma\n",
        keys: "jlvjl<Esc>gg'<lt>",
    },
    OracleCase {
        name: "visual end mark goes to the last line of the selection",
        initial_text: "alpha\n  beta\n  gamma\n",
        keys: "jlvjl<Esc>gg'>",
    },
    OracleCase {
        name: "exact visual start mark",
        initial_text: "alpha\n  beta\n  gamma\n",
        keys: "jlvjl<Esc>gg`<lt>",
    },
    OracleCase {
        name: "exact visual end mark",
        initial_text: "alpha\n  beta\n  gamma\n",
        keys: "jlvjl<Esc>gg`>",
    },
    OracleCase {
        name: "visual marks use buffer order for a backwards selection",
        initial_text: "alpha\n  beta\n  gamma\n",
        keys: "jjllvkh<Esc>G`<lt>",
    },
    OracleCase {
        name: "exact end mark of a linewise selection clamps to the line end",
        initial_text: "alpha\n  beta\n  gamma\n",
        keys: "jVj<Esc>gg`>",
    },
    OracleCase {
        name: "exact start mark of a linewise selection is column zero",
        initial_text: "alpha\n  beta\n  gamma\n",
        keys: "jlVj<Esc>gg`<lt>",
    },
    OracleCase {
        name: "visual marks stay after the selection is yanked",
        initial_text: "alpha\n  beta\n  gamma\n",
        keys: "lvjly`>",
    },
    OracleCase {
        name: "change marks bracket a visual yank",
        initial_text: "alpha\n  beta\n  gamma\n",
        keys: "lvjly`]",
    },
    // Last change or yank: yanks.
    OracleCase {
        name: "start mark after a line yank",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jyyG`[",
    },
    OracleCase {
        name: "end mark after a line yank clamps to the line end",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jyyG`]",
    },
    OracleCase {
        name: "start line mark after a two line yank",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "j2yyG'[",
    },
    OracleCase {
        name: "end line mark after a two line yank",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "j2yygg']",
    },
    OracleCase {
        name: "start mark after an inner word yank",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "wyiw$`[",
    },
    OracleCase {
        name: "end mark after an inner word yank",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "wyiw0`]",
    },
    // Puts.
    OracleCase {
        name: "start mark after a linewise put",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "yyjpgg`[",
    },
    OracleCase {
        name: "end mark after a linewise put",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "yyjpgg`]",
    },
    OracleCase {
        name: "start mark after a charwise put",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "yiwj$pgg`[",
    },
    OracleCase {
        name: "end mark after a charwise put",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "yiwj$pgg`]",
    },
    OracleCase {
        name: "end mark after a charwise put before the cursor",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "yiwjPgg`]",
    },
    // Inserts.
    OracleCase {
        name: "start mark after inserting text",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jixy<Esc>gg`[",
    },
    OracleCase {
        name: "end mark after inserting text",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jixy<Esc>gg`]",
    },
    OracleCase {
        name: "end mark after appending at the line end",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jAxy<Esc>gg`]",
    },
    OracleCase {
        name: "start line mark after opening a line",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jofoo<Esc>gg'[",
    },
    OracleCase {
        name: "end line mark after opening a line",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jofoo<Esc>gg']",
    },
    OracleCase {
        name: "start mark after a multi line insert",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jixy<CR>z<Esc>gg`[",
    },
    OracleCase {
        name: "end mark after a multi line insert",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jixy<CR>z<Esc>gg`]",
    },
    // Deletes leave both marks at the deleted spot.
    OracleCase {
        name: "start mark after deleting a line",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jddG`[",
    },
    OracleCase {
        name: "end mark after deleting a line",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jddG`]",
    },
    OracleCase {
        name: "start mark after deleting a word",
        initial_text: "one two three\nx\n",
        keys: "wdwj`[",
    },
    OracleCase {
        name: "end mark after deleting a word",
        initial_text: "one two three\nx\n",
        keys: "wdwj`]",
    },
    OracleCase {
        name: "marks after deleting a character",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "lxj`[",
    },
    OracleCase {
        name: "marks after deleting backwards",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "$dbj`]",
    },
    // Changes and replacements bracket the new text.
    OracleCase {
        name: "start mark after changing a word",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "wcwXY<Esc>j`[",
    },
    OracleCase {
        name: "end mark after changing a word",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "wcwXY<Esc>j`]",
    },
    OracleCase {
        name: "start mark after a counted replace",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "l3rxj`[",
    },
    OracleCase {
        name: "end mark after a counted replace",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "3rxj`]",
    },
    OracleCase {
        name: "end mark after a case toggle",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "3~j`]",
    },
    OracleCase {
        name: "start mark after joining lines",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "J`[",
    },
    OracleCase {
        name: "end mark after joining lines",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "J`]",
    },
    // Undo brackets the restored lines.
    OracleCase {
        name: "start mark after undoing a line delete",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jdduG`[",
    },
    OracleCase {
        name: "end mark after undoing a line delete",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jdduG`]",
    },
    OracleCase {
        name: "end mark after undoing an insert",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "jixy<Esc>uG`]",
    },
    // Loading the file brackets the whole buffer.
    OracleCase {
        name: "start mark of a fresh buffer is its first line",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "j'[",
    },
    OracleCase {
        name: "end mark of a fresh buffer is its last line",
        initial_text: "one two\nthree four\nfive six\n",
        keys: "`]",
    },
];
