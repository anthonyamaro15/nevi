use super::{OracleCase, SCREEN_POSITION_TEXT};

/// How the view follows a cursor that lands off screen (#340). Neovim's
/// update_topline scrolls the minimum when the cursor lands less than about
/// half a screen below the window, and centers the cursor line when it lands
/// further away. Going up, it centers once the view would have to scroll
/// about half a screen. A centered view near the end of the file shows the
/// last line at the bottom instead of empty rows, and nothing scrolls down
/// while the last line is on screen. The window has 22 text rows and
/// scrolloff 8; the cases start at the top unless they jump to the end first.
pub(super) const VIEWPORT_CASES: &[OracleCase] = &[
    OracleCase {
        name: "far jump down centers the cursor line",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "50G",
    },
    OracleCase {
        name: "far jump near the end shows the last line at the bottom",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "90G",
    },
    OracleCase {
        name: "far jump to the next to last line",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "99G",
    },
    // A jump of half a screen goes through scroll_cursor_bot, which centers
    // with the cursor one row lower than the far jump above does.
    OracleCase {
        name: "jump less than half a screen below scrolls minimally",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "33G",
    },
    OracleCase {
        name: "jump half a screen below centers one row lower",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "34G",
    },
    OracleCase {
        name: "k after G leaves the view alone",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "Gkkk",
    },
    OracleCase {
        name: "k up to the top margin after G does not scroll",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "G13k",
    },
    OracleCase {
        name: "far jump up centers the cursor line",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "G60G",
    },
    OracleCase {
        name: "jump up just past the threshold centers",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "G77G",
    },
    OracleCase {
        name: "short jump up scrolls minimally",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "G78G",
    },
    OracleCase {
        name: "far search centers the match",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "/line 050<CR>",
    },
    OracleCase {
        name: "far search near the end shows the last line at the bottom",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "/line 090<CR>",
    },
    OracleCase {
        name: "far backward search centers the match",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "G?line 020<CR>",
    },
    OracleCase {
        name: "far substitute centers the changed line",
        initial_text: SCREEN_POSITION_TEXT,
        keys: ":%s/line 060/x/<CR>",
    },
    OracleCase {
        name: "far mark jump centers the line",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "30Gma90G'a",
    },
    OracleCase {
        name: "undo far away centers the change",
        initial_text: SCREEN_POSITION_TEXT,
        keys: "90Gx50Gu",
    },
];
