use super::OracleCase;

/// Operators with linewise motions, verified against real Neovim. `j` `k`
/// `G` `gg` `{n}G` `H` `M` `L` (and `+` `-` `<CR>`) cover whole lines under
/// an operator: `dj` deletes two lines, `cj` changes them into one line,
/// `yj` yanks them linewise. `j` and `k` fail at the buffer edge and
/// cancel the operator; `G` and `gg` on their own line still act on it.
/// With Neovim's `nostartofline` the cursor keeps its column.
pub(super) const LINEWISE_CASES: &[OracleCase] = &[
    OracleCase {
        name: "dj deletes two lines",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jdj",
    },
    OracleCase {
        name: "dj keeps the cursor column",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jlldj",
    },
    OracleCase {
        name: "dk deletes two lines upward",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jjdk",
    },
    OracleCase {
        name: "dj on the last line does nothing",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "Gdj",
    },
    OracleCase {
        name: "dk on the first line does nothing",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "dk",
    },
    OracleCase {
        name: "counted dj clamps at the last line",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "j5dj",
    },
    OracleCase {
        name: "count before d multiplies the motion",
        initial_text: "one two\nthree four\nfive six\nseven eight\nnine ten\n",
        keys: "2dj",
    },
    OracleCase {
        name: "dot repeats dj",
        initial_text: "one two\nthree four\nfive six\nseven eight\nnine ten\nx\n",
        keys: "jdj.",
    },
    OracleCase {
        name: "dj undo restores both lines",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jdju",
    },
    OracleCase {
        name: "yj yanks two lines linewise",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jyjGp",
    },
    OracleCase {
        name: "yk moves the cursor to the upper line",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jjlyk",
    },
    OracleCase {
        name: "yk yanks linewise",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jjykGp",
    },
    OracleCase {
        name: "dG deletes to the end of the buffer",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jdG",
    },
    OracleCase {
        name: "dG on the last line deletes it",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "GdG",
    },
    OracleCase {
        name: "dgg deletes to the start of the buffer",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jjdgg",
    },
    OracleCase {
        name: "dgg on the first line deletes it",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "dgg",
    },
    OracleCase {
        name: "counted G under d deletes through that line",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jd3G",
    },
    OracleCase {
        name: "yG yanks to the end linewise",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jjyGggP",
    },
    OracleCase {
        name: "cj changes two lines into one",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jcjX<Esc>",
    },
    OracleCase {
        name: "ck changes two lines upward",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jjckX<Esc>",
    },
    OracleCase {
        name: "cj keeps the indent of the first line",
        initial_text: "one\n    two\n  three\nfour\n",
        keys: "jcjX<Esc>",
    },
    OracleCase {
        name: "cG changes to the end of the buffer",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jcGX<Esc>",
    },
    OracleCase {
        name: "cgg changes to the start of the buffer",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jcggX<Esc>",
    },
    OracleCase {
        name: "dH deletes to the top of the window",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jjdH",
    },
    OracleCase {
        name: "dL deletes to the bottom of the window",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jdL",
    },
    OracleCase {
        name: "dM deletes to the middle of the window",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jjjdM",
    },
    OracleCase {
        name: "M on a short buffer goes to the middle of the lines shown",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "GM",
    },
    OracleCase {
        name: "dH under an operator reaches the true top line",
        initial_text: super::SCREEN_POSITION_TEXT,
        keys: "50GztdH",
    },
    OracleCase {
        name: "dL under an operator reaches the true bottom line",
        initial_text: super::SCREEN_POSITION_TEXT,
        keys: "50GzbdL",
    },
    OracleCase {
        name: "yM yanks to the middle of a scrolled window",
        initial_text: super::SCREEN_POSITION_TEXT,
        keys: "50GzzjjyM",
    },
    OracleCase {
        name: "d minus deletes two lines upward",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jjd-",
    },
    OracleCase {
        name: "d enter deletes two lines downward",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jd<CR>",
    },
    OracleCase {
        name: "gUj uppercases two lines",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jlgUj",
    },
    OracleCase {
        name: "g tilde k toggles two lines",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jjlg~k",
    },
    OracleCase {
        name: "dj on a two line buffer leaves one empty line",
        initial_text: "one two\nthree four\n",
        keys: "dj",
    },
    OracleCase {
        name: "dj then p puts the deleted lines back below",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jdjp",
    },
    OracleCase {
        name: "dot repeats cj",
        initial_text: "one two\nthree four\nfive six\nseven eight\nnine ten\neleven twelve\n",
        keys: "jcjX<Esc>jj.",
    },
    OracleCase {
        name: "counted G under c changes through that line",
        initial_text: "one two\nthree four\nfive six\nseven eight\n",
        keys: "jc3GX<Esc>",
    },
    OracleCase {
        name: "dj with a count from the operator side",
        initial_text: "one two\nthree four\nfive six\nseven eight\nnine ten\n",
        keys: "d2j",
    },
];
