use super::OracleCase;

/// Insert-entry cases protect Vim's cursor placement, count, and undo semantics
/// for the normal-mode `I` and `A` commands.
pub(super) const INSERT_ENTRY_CASES: &[OracleCase] = &[
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
