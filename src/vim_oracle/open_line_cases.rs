use super::OracleCase;

/// Open-line cases protect Vim's indentation, count, cursor, and undo semantics
/// for the normal-mode `o` and `O` commands.
pub(super) const OPEN_LINE_CASES: &[OracleCase] = &[
    OracleCase {
        name: "open below inherits indentation",
        initial_text: "    alpha\n",
        keys: "ochild<Esc>",
    },
    OracleCase {
        name: "open above inherits indentation",
        initial_text: "    alpha\n",
        keys: "Oparent<Esc>",
    },
    OracleCase {
        name: "open below from empty line",
        initial_text: "\n",
        keys: "obelow<Esc>",
    },
    OracleCase {
        name: "open above from empty line",
        initial_text: "\n",
        keys: "Oabove<Esc>",
    },
    OracleCase {
        name: "open below without final newline",
        initial_text: "alpha",
        keys: "obelow<Esc>",
    },
    OracleCase {
        name: "open above without final newline",
        initial_text: "alpha",
        keys: "Oabove<Esc>",
    },
    OracleCase {
        name: "counted open below",
        initial_text: "alpha\nomega\n",
        keys: "3obelow<Esc>",
    },
    OracleCase {
        name: "counted open above",
        initial_text: "alpha\nomega\n",
        keys: "j3Oabove<Esc>",
    },
    OracleCase {
        name: "counted open below inherits indentation",
        initial_text: "    alpha\nomega\n",
        keys: "3obelow<Esc>",
    },
    OracleCase {
        name: "counted open above inherits indentation",
        initial_text: "alpha\n    omega\n",
        keys: "j3Oabove<Esc>",
    },
    OracleCase {
        name: "counted open below without text",
        initial_text: "alpha\nomega\n",
        keys: "3o<Esc>",
    },
    OracleCase {
        name: "counted open above without text",
        initial_text: "alpha\nomega\n",
        keys: "j3O<Esc>",
    },
    OracleCase {
        name: "undo counted open below",
        initial_text: "alpha\nomega\n",
        keys: "3obelow<Esc>u",
    },
    OracleCase {
        name: "redo counted open below",
        initial_text: "alpha\nomega\n",
        keys: "3obelow<Esc>u<C-r>",
    },
    OracleCase {
        name: "undo counted open above",
        initial_text: "alpha\nomega\n",
        keys: "j3Oabove<Esc>u",
    },
    OracleCase {
        name: "redo counted open above",
        initial_text: "alpha\nomega\n",
        keys: "j3Oabove<Esc>u<C-r>",
    },
    OracleCase {
        name: "redo open below from line end",
        initial_text: "alpha\nomega\n",
        keys: "$obelow<Esc>u<C-r>",
    },
    OracleCase {
        name: "redo open above from line end",
        initial_text: "alpha\nomega\n",
        keys: "j$Oabove<Esc>u<C-r>",
    },
];
