use super::OracleCase;

/// Insert-entry cases protect Vim's cursor placement, count, and undo semantics
/// for the normal-mode `i`, `a`, `I`, and `A` commands.
pub(super) const INSERT_ENTRY_CASES: &[OracleCase] = &[
    // Counted i and a repeat the typed text like I and A do. The texts put
    // the cursor mid-line so a repeat at the wrong position shows up.
    OracleCase {
        name: "counted insert at cursor",
        initial_text: "alpha\n",
        keys: "l3ix<Esc>",
    },
    OracleCase {
        name: "counted append after cursor",
        initial_text: "alpha\n",
        keys: "l3ax<Esc>",
    },
    OracleCase {
        name: "counted append on empty line",
        initial_text: "\n",
        keys: "3ax<Esc>",
    },
    OracleCase {
        name: "counted insert with newline",
        initial_text: "alpha\n",
        keys: "l2iab<CR><Esc>",
    },
    OracleCase {
        name: "undo counted insert at cursor",
        initial_text: "alpha\n",
        keys: "l3ix<Esc>u",
    },
    OracleCase {
        name: "dot repeat replays counted insert",
        initial_text: "alpha\n",
        keys: "3ix<Esc>$.",
    },
    OracleCase {
        name: "insert at first nonblank",
        initial_text: "    alpha\n",
        keys: "Istart-<Esc>",
    },
    OracleCase {
        name: "insert on whitespace-only line",
        initial_text: "    \n",
        keys: "Istart<Esc>",
    },
    OracleCase {
        name: "insert on empty line",
        initial_text: "\n",
        keys: "Istart<Esc>",
    },
    OracleCase {
        name: "append on empty line",
        initial_text: "\n",
        keys: "Aend<Esc>",
    },
    OracleCase {
        name: "append without final newline",
        initial_text: "alpha",
        keys: "A-end<Esc>",
    },
    OracleCase {
        name: "counted insert at first nonblank",
        initial_text: "    alpha\n",
        keys: "3Ix<Esc>",
    },
    OracleCase {
        name: "counted append at line end",
        initial_text: "alpha\n",
        keys: "3Ax<Esc>",
    },
    OracleCase {
        name: "undo counted insert at first nonblank",
        initial_text: "    alpha\n",
        keys: "3Ix<Esc>u",
    },
    OracleCase {
        name: "redo multi-character insert at first nonblank",
        initial_text: "    alpha\n",
        keys: "Ixyz<Esc>u<C-r>",
    },
    OracleCase {
        name: "redo multi-character append at line end",
        initial_text: "alpha\n",
        keys: "Axyz<Esc>u<C-r>",
    },
    OracleCase {
        name: "redo counted append at line end",
        initial_text: "alpha\n",
        keys: "3Ax<Esc>u<C-r>",
    },
];
