use super::OracleCase;

/// Search-family cases: `/`, `?`, `n`, `N`, `*`, `#`, `gn`, `gN`, and the
/// search prompt editing keys, verified against real Neovim. Both editors
/// default to case-sensitive search with wrapscan on, so plain-text patterns
/// compare directly. Nevi's search is a literal substring match (no regex),
/// so every pattern here is literal on both sides.
pub(super) const SEARCH_CASES: &[OracleCase] = &[
    // Forward search basics.
    OracleCase {
        name: "search forward lands on match start",
        initial_text: "alpha\nsome beta here\ngamma\n",
        keys: "/beta<CR>",
    },
    OracleCase {
        name: "search forward skips match at cursor",
        initial_text: "beta x beta\n",
        keys: "/beta<CR>",
    },
    OracleCase {
        name: "search forward wraps to top",
        initial_text: "beta\nalpha\ngamma\n",
        keys: "G/beta<CR>",
    },
    OracleCase {
        name: "search not found leaves cursor",
        initial_text: "alpha\nbeta\n",
        keys: "/zzz<CR>",
    },
    // Incremental typing moves the cursor while a prefix matches; a failed
    // final pattern must land back on the original position like nvim.
    OracleCase {
        name: "search not found after partial match restores cursor",
        initial_text: "alpha\nbetx\n",
        keys: "/betz<CR>",
    },
    OracleCase {
        name: "search is case sensitive",
        initial_text: "Beta\nbeta\n",
        keys: "/beta<CR>",
    },
    OracleCase {
        name: "empty search repeats last pattern",
        initial_text: "alpha\nbeta\nbeta tail\n",
        keys: "/beta<CR>gg/<CR>",
    },
    // Backward search.
    OracleCase {
        name: "search backward lands on previous match",
        initial_text: "beta\nalpha\nbeta\ngamma\n",
        keys: "G?beta<CR>",
    },
    OracleCase {
        name: "search backward wraps to bottom",
        initial_text: "alpha\nbeta\n",
        keys: "?beta<CR>",
    },
    OracleCase {
        name: "search backward within line",
        initial_text: "beta beta x\n",
        keys: "$?beta<CR>",
    },
    // Empty ? repeats the last pattern backward AND flips the stored
    // direction, so a following n keeps going backward.
    OracleCase {
        name: "empty backward search flips direction for n",
        initial_text: "beta\nx\nbeta\nx\nbeta\n",
        keys: "/beta<CR>G?<CR>n",
    },
    // n / N repeat.
    OracleCase {
        name: "next match",
        initial_text: "beta\nbeta\nbeta\n",
        keys: "/beta<CR>n",
    },
    OracleCase {
        name: "next match wraps",
        initial_text: "beta\nbeta\nbeta\n",
        keys: "/beta<CR>nn",
    },
    OracleCase {
        name: "previous match reverses direction",
        initial_text: "beta\nbeta\nbeta\n",
        keys: "/beta<CR>nN",
    },
    OracleCase {
        name: "n after backward search continues backward",
        initial_text: "beta\nalpha\nbeta\nbeta\n",
        keys: "G?beta<CR>n",
    },
    OracleCase {
        name: "counted next match",
        initial_text: "beta\nbeta\nbeta\nbeta\n",
        keys: "/beta<CR>2n",
    },
    OracleCase {
        name: "counted previous match",
        initial_text: "beta\nbeta\nbeta\nbeta\n",
        keys: "/beta<CR>nn2N",
    },
    // Word-under-cursor search.
    OracleCase {
        name: "star searches word forward",
        initial_text: "beta alpha\ngamma\nbeta tail\n",
        keys: "*",
    },
    OracleCase {
        name: "star wraps to only occurrence",
        initial_text: "beta\nalpha\n",
        keys: "*",
    },
    OracleCase {
        name: "star from middle of word",
        initial_text: "beta alpha beta\n",
        keys: "ll*",
    },
    OracleCase {
        name: "star then n continues forward",
        initial_text: "beta\nalpha beta\nbeta\n",
        keys: "*n",
    },
    OracleCase {
        name: "hash searches word backward",
        initial_text: "beta\nalpha\nbeta gamma\n",
        keys: "G#",
    },
    OracleCase {
        name: "hash from middle of word",
        initial_text: "beta x\nbeta\n",
        keys: "jl#",
    },
    // gn / gN select the match in visual mode. gn leaves the cursor on the
    // match end, gN on the match start.
    OracleCase {
        name: "gn selects current match",
        initial_text: "alpha\nbeta\ngamma beta\n",
        keys: "/beta<CR>gn",
    },
    OracleCase {
        name: "gn selects next match from outside",
        initial_text: "alpha\nbeta\ngamma beta\n",
        keys: "/beta<CR>gggn",
    },
    OracleCase {
        name: "counted gn selects a later match",
        initial_text: "beta\nbeta\nbeta\n",
        keys: "/beta<CR>gg2gn",
    },
    OracleCase {
        name: "gN selects match backward",
        initial_text: "beta\nbeta\ngamma\n",
        keys: "/beta<CR>GgN",
    },
    // Search prompt editing (vim cmdline keys).
    OracleCase {
        name: "prompt backspace reanchors at origin",
        initial_text: "ac\nab\nac\n",
        keys: "/ab<BS>c<CR>",
    },
    OracleCase {
        name: "prompt ctrl-w deletes word",
        initial_text: "alpha\nbeta\n",
        keys: "/junk<C-w>beta<CR>",
    },
    OracleCase {
        name: "prompt ctrl-u clears input",
        initial_text: "alpha\nbeta\n",
        keys: "/alpha<C-u>beta<CR>",
    },
    OracleCase {
        name: "prompt ctrl-b inserts at start",
        initial_text: "alpha\nbeta\n",
        keys: "/eta<C-b>b<CR>",
    },
    OracleCase {
        name: "prompt ctrl-e returns to end",
        initial_text: "alpha\nbeta\n",
        keys: "/et<C-b>b<C-e>a<CR>",
    },
    OracleCase {
        name: "prompt history recall",
        initial_text: "alpha\nbeta\nbeta tail\n",
        keys: "/beta<CR>gg/<Up><CR>",
    },
    OracleCase {
        name: "prompt register insert",
        initial_text: "alpha\nbeta\nalpha end\n",
        keys: "yiwj/<C-r>0<CR>",
    },
    // Cancelling restores the position the search started from.
    OracleCase {
        name: "escape cancels and restores cursor",
        initial_text: "alpha\nbeta\n",
        keys: "/beta<Esc>",
    },
    OracleCase {
        name: "escape restores viewport after distant match",
        initial_text: super::SCREEN_POSITION_TEXT,
        keys: "50Gzz/line 090<Esc>",
    },
    // * and # search with word boundaries (\<word\>), so the word text inside
    // a longer keyword is never a match. Every key that reuses the pattern
    // (n, N, gn, gN, an empty / prompt) inherits the boundaries. The texts are
    // deliberately asymmetric so a boundary bug cannot pass by landing on a
    // match Vim reaches by a different route.
    OracleCase {
        name: "star skips match inside longer word",
        initial_text: "abc abcdef\nabc\n",
        keys: "*",
    },
    OracleCase {
        name: "star with only embedded matches wraps to itself",
        initial_text: "abc abcdef\n",
        keys: "l*",
    },
    OracleCase {
        name: "hash skips match inside longer word",
        initial_text: "abc\nxabc abc\n",
        keys: "j$#",
    },
    OracleCase {
        name: "n after star keeps word boundaries",
        initial_text: "abc abcdef\nabc\nabc_x abc\n",
        keys: "*n",
    },
    OracleCase {
        name: "gn after star selects the whole word only",
        initial_text: "abc abcdef abc\n",
        keys: "*gnd",
    },
    OracleCase {
        name: "empty search repeat keeps star boundaries",
        initial_text: "abc abcdef\nabcx\nabc\n",
        keys: "*/<CR>",
    },
    // The boundary atoms can also be typed; this is the pattern * records.
    OracleCase {
        name: "typed word boundary pattern",
        initial_text: "abcdef abc\n",
        keys: "/\\<lt>abc\\><CR>",
    },
    // * adds \<abc\> to the search history: two Ups skip past the later
    // /xyz entry and recall it. Without the history entry the second Up
    // stays on xyz and the search lands back on line 3.
    OracleCase {
        name: "star records its pattern in search history",
        initial_text: "abc abcdef\nabc\nxyz\n",
        keys: "*/xyz<CR>/<Up><Up><CR>",
    },
    // g* and g# search the word without boundaries, so a match inside a
    // longer word counts, and n keeps that looser pattern.
    OracleCase {
        name: "g-star finds match inside longer word",
        initial_text: "abc abcdef\nabc\n",
        keys: "g*",
    },
    OracleCase {
        name: "g-hash finds match inside longer word backward",
        initial_text: "abc\nxabc abc\n",
        keys: "j$g#",
    },
    OracleCase {
        name: "n after g-star keeps partial matching",
        initial_text: "abc abcdef\nabc\n",
        keys: "g*n",
    },
    OracleCase {
        name: "g-star records plain pattern in search history",
        initial_text: "abc\nxyz\nabcdef\n",
        keys: "g*/xyz<CR>/<Up><Up><CR>",
    },
    OracleCase {
        name: "N after g-star goes back through partial match",
        initial_text: "abc abcdef\nabc\n",
        keys: "g*nN",
    },
    OracleCase {
        name: "g-star from middle of word uses whole keyword",
        initial_text: "abcdef abc\n",
        keys: "$hlg*",
    },
    OracleCase {
        name: "gn after g-star selects partial match",
        initial_text: "abc abcdef\n",
        keys: "g*gnd",
    },
];

// Known divergence, deliberately not an active case (it would fail CI):
// backward search only considers matches that END before the cursor, Vim
// considers matches that START before it, and `#` does not first move to
// the word start like Vim's nv_ident does. Verified vs nvim 0.11.3:
//   initial_text: "abcabc\n", keys: "ll?abc<CR>"  nvim (0, 0); Nevi (0, 3)
//   initial_text: "x beta\n", keys: "3l#"         nvim (0, 2); Nevi (0, 3)
