use super::OracleCase;

/// `Ctrl-a` / `Ctrl-x` cases, verified against real Neovim with its default
/// `nrformats=bin,hex`: decimal with a leading minus, `0x` hex, `0b` binary,
/// no octal. Width is preserved when the number has leading zeros, hex digit
/// case follows the existing digits, and the cursor lands on the last digit.
pub(super) const INCREMENT_CASES: &[OracleCase] = &[
    OracleCase {
        name: "increment number after cursor",
        initial_text: "x = 5\n",
        keys: "<C-a>",
    },
    OracleCase {
        name: "decrement number after cursor",
        initial_text: "x = 5\n",
        keys: "<C-x>",
    },
    OracleCase {
        name: "count adds to the number",
        initial_text: "x = 5\n",
        keys: "5<C-a>",
    },
    OracleCase {
        name: "count with two digits",
        initial_text: "1\n",
        keys: "10<C-a>",
    },
    OracleCase {
        name: "cursor inside number uses the whole number",
        initial_text: "abc 123 def\n",
        keys: "5l<C-a>",
    },
    OracleCase {
        name: "cursor after last number does nothing",
        initial_text: "12 abc\n",
        keys: "$<C-a>",
    },
    OracleCase {
        name: "number on a later line is not found",
        initial_text: "abc\n5\n",
        keys: "<C-a>",
    },
    OracleCase {
        name: "cursor between numbers takes the next one",
        initial_text: "1 2\n",
        keys: "l<C-a>",
    },
    OracleCase {
        name: "number embedded in a word grows",
        initial_text: "foo9bar\n",
        keys: "<C-a>",
    },
    // Signed decimals.
    OracleCase {
        name: "negative one increments to zero",
        initial_text: "-1\n",
        keys: "<C-a>",
    },
    OracleCase {
        name: "negative crosses zero with count",
        initial_text: "-1\n",
        keys: "3<C-a>",
    },
    OracleCase {
        name: "zero decrements to negative",
        initial_text: "0\n",
        keys: "<C-x>",
    },
    OracleCase {
        name: "count decrement goes negative",
        initial_text: "10\n",
        keys: "15<C-x>",
    },
    OracleCase {
        name: "minus after a word char is still a sign",
        initial_text: "x-5\n",
        keys: "<C-x>",
    },
    OracleCase {
        name: "cursor on the minus sign",
        initial_text: "-5\n",
        keys: "<C-a>",
    },
    // Leading zeros keep the width.
    OracleCase {
        name: "leading zeros keep width",
        initial_text: "007\n",
        keys: "<C-a>",
    },
    OracleCase {
        name: "leading zeros grow when needed",
        initial_text: "099\n",
        keys: "<C-a>",
    },
    OracleCase {
        name: "negative with leading zeros",
        initial_text: "-007\n",
        keys: "<C-a>",
    },
    // Hex.
    OracleCase {
        name: "hex increment keeps width",
        initial_text: "0x0f\n",
        keys: "<C-a>",
    },
    OracleCase {
        name: "hex grows past its width",
        initial_text: "0xff\n",
        keys: "<C-a>",
    },
    OracleCase {
        name: "hex keeps leading zeros",
        initial_text: "0x00ff\n",
        keys: "<C-a>",
    },
    OracleCase {
        name: "hex keeps uppercase digits",
        initial_text: "0XAB\n",
        keys: "<C-x>",
    },
    OracleCase {
        name: "hex cursor on a letter digit",
        initial_text: "0x1f\n",
        keys: "$<C-a>",
    },
    OracleCase {
        name: "hex cursor on the x",
        initial_text: "0x0f\n",
        keys: "l<C-a>",
    },
    OracleCase {
        name: "hex ignores a leading minus",
        initial_text: "-0x10\n",
        keys: "<C-a>",
    },
    OracleCase {
        name: "hex zero wraps on decrement",
        initial_text: "0x0\n",
        keys: "<C-x>",
    },
    // Binary.
    OracleCase {
        name: "binary increment",
        initial_text: "0b101\n",
        keys: "<C-a>",
    },
    OracleCase {
        name: "binary grows past its width",
        initial_text: "0b11\n",
        keys: "<C-a>",
    },
    // Repeat and undo.
    OracleCase {
        name: "dot repeats increment",
        initial_text: "5\n",
        keys: "<C-a>.",
    },
    OracleCase {
        name: "dot repeats counted increment",
        initial_text: "5\n",
        keys: "3<C-a>.",
    },
    OracleCase {
        name: "undo restores number",
        initial_text: "x = 5\n",
        keys: "<C-a>u",
    },
];
