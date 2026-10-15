//! Where the view goes when the cursor moves: a port of Neovim's
//! update_topline() (move.c) for a window whose lines each take one row.
//!
//! A cursor that lands a little past the window scrolls it the minimum,
//! keeping 'scrolloff' lines of context. One that lands about half a screen
//! or more away centers the cursor line instead. The context below the
//! cursor stops at the last line, so the view never scrolls down while the
//! last line is on screen, and a centered view near the end shows the last
//! line at the bottom instead of empty rows. The loops mirror Neovim's own
//! (scroll_cursor_top, scroll_cursor_bot, scroll_cursor_halfway) so their
//! thresholds match exactly; folds, filler lines, 'scrolljump' and
//! 'smoothscroll' are left out because Nevi has none of them.
//!
//! ponytail: one row per line. With 'wrap' the caller still fixes up long
//! lines afterwards; pass a row count per line here if that gets in the way.

/// The new top line after the cursor moved to `cursor`, for a window of
/// `rows` rows that showed `top` first, over a buffer whose last line is
/// `last`, with 'scrolloff' `so`. `so` is not capped: like Neovim, a value
/// above half the window ends up keeping the cursor line in the middle.
pub(super) fn follow_cursor(
    top: usize,
    cursor: usize,
    rows: usize,
    so: usize,
    last: usize,
) -> usize {
    let view = View {
        rows: rows.max(1),
        so,
        last,
    };
    view.update_topline(top, cursor.min(last))
}

struct View {
    rows: usize,
    so: usize,
    last: usize,
}

impl View {
    fn update_topline(&self, mut top: usize, cursor: usize) -> usize {
        // Above the window, or inside the top margin: scroll up, or center
        // once the scroll would be about half a screen.
        // Saturating: 'scrolloff' comes from the config unchecked, and a huge
        // value must behave like Vim's so=999, not overflow.
        if top > 0 && cursor < top.saturating_add(self.so) {
            let halfheight = (self.rows / 2).saturating_sub(1).max(2);
            if top.saturating_add(self.so) - cursor >= halfheight {
                return self.scroll_cursor_halfway(cursor, false);
            }
            top = self.scroll_cursor_top(top, cursor);
        }

        // Below the window, or inside the bottom margin. Nothing scrolls
        // down while the last line is on screen.
        let botline = top + self.rows; // first line below the window
        if botline > self.last || cursor.saturating_add(self.so) < botline {
            return top;
        }
        // Neovim's line_count: cursor - botline + 1 + so > height + 1.
        if cursor.saturating_add(self.so) > botline + self.rows {
            return self.scroll_cursor_halfway(cursor, false);
        }
        self.scroll_cursor_bot(top, cursor)
    }

    /// Scroll up the least that leaves 'scrolloff' lines above the cursor,
    /// or center when the cursor and its context do not fit.
    fn scroll_cursor_top(&self, top: usize, cursor: usize) -> usize {
        let mut used = 1;
        let mut extra = 0;
        let mut scrolled = usize::from(cursor < top);
        let mut new_top = cursor;
        for (n, line) in (0..cursor).rev().enumerate() {
            if line < top {
                scrolled += 1;
            }
            if (new_top >= top || scrolled > 1) && extra >= self.so {
                break;
            }
            used += 1;
            // The matching line below the cursor, if there is one.
            if extra < self.so && cursor + 1 + n < self.last {
                used += 1;
            }
            if used > self.rows {
                return self.scroll_cursor_halfway(cursor, false);
            }
            extra += 1;
            new_top = line;
        }
        new_top.min(top)
    }

    /// Scroll down the least that leaves 'scrolloff' lines below the cursor
    /// (fewer at the end of the buffer), or center when that would scroll a
    /// screenful. Only called with the window full of lines.
    fn scroll_cursor_bot(&self, top: usize, cursor: usize) -> usize {
        let botline = top + self.rows;
        let mut used = 1;
        let mut extra = 0;
        let mut scrolled = usize::from(cursor >= botline);
        let (mut above, mut below) = (cursor, cursor);
        // Count lines around the cursor until the ones between the window
        // and the cursor are covered and there is enough context below.
        // Every line below counts against the height, so a cursor about
        // half a screen past the window no longer fits and gets centered.
        while above > 0 {
            if (extra >= self.so || below >= self.last) && above <= botline {
                break;
            }
            above -= 1;
            used += 1;
            if used > self.rows {
                break;
            }
            if above >= botline {
                scrolled += 1;
            }
            if below < self.last {
                below += 1;
                used += 1;
                if used > self.rows {
                    break;
                }
                if extra < self.so || scrolled == 0 {
                    extra += 1;
                    if below >= botline {
                        scrolled += 1;
                    }
                }
            }
        }

        if scrolled == 0 {
            return top;
        }
        let line_count = if used > self.rows { used } else { scrolled };
        if line_count >= self.rows && line_count > 1 {
            return self.scroll_cursor_halfway(cursor, true);
        }
        (top + line_count).min(self.last)
    }

    /// Put the cursor line in the middle. Rows past the last line cost
    /// nothing, so near the end the lines above fill the window and the
    /// last line sits on the bottom row. With an even height the spare row
    /// goes below the cursor, or above it when `prefer_above` (Neovim's
    /// scroll_cursor_bot asks for that).
    fn scroll_cursor_halfway(&self, cursor: usize, prefer_above: bool) -> usize {
        let mut used = 1;
        let (mut above, mut below) = (0, 0);
        let (mut top, mut bottom) = (cursor, cursor);
        while top > 0 {
            if below < above || (below == above && !prefer_above) {
                if bottom < self.last {
                    bottom += 1;
                    used += 1;
                    if used > self.rows {
                        break;
                    }
                }
                below += 1;
            }
            if below > above || (below == above && prefer_above) {
                used += 1;
                if used > self.rows {
                    break;
                }
                top -= 1;
                above += 1;
            }
        }
        top
    }
}

#[cfg(test)]
mod tests {
    use super::follow_cursor;

    /// (rows, scrolloff, top before, cursor, top after), every row measured
    /// in nvim 0.11.3 over 100 one-row lines.
    const NVIM: &[(usize, usize, usize, usize, usize)] = &[
        // The #340 report: far jumps center, near the end they pack the
        // last line at the bottom, and k after G does not scroll.
        (22, 8, 0, 49, 39),
        (22, 8, 0, 89, 78),
        (22, 8, 0, 98, 78),
        (22, 8, 78, 98, 78),
        (22, 8, 78, 86, 78),
        // Down: minimal below half a screen, then centered one row lower
        // (scroll_cursor_bot), then centered by update_topline itself.
        (22, 8, 0, 32, 19),
        (22, 8, 0, 33, 22),
        (22, 8, 0, 36, 25),
        (22, 8, 0, 37, 27),
        // Up: minimal until the scroll would reach half a screen.
        (22, 8, 78, 85, 77),
        (22, 8, 78, 77, 69),
        (22, 8, 78, 76, 66),
        (22, 8, 78, 59, 49),
        (22, 8, 30, 0, 0),
        // Inside the margins the view stays; one line into the bottom
        // margin scrolls just enough.
        (22, 8, 30, 40, 30),
        (22, 8, 30, 45, 32),
        // scrolloff larger than half the window keeps the cursor central.
        (10, 8, 0, 5, 0),
        (10, 8, 0, 6, 1),
        (10, 8, 1, 7, 3),
        (10, 8, 0, 49, 45),
        // No scrolloff: odd and even heights center differently.
        (9, 0, 0, 13, 5),
        (10, 0, 0, 15, 10),
        (11, 0, 0, 16, 6),
        (11, 0, 0, 49, 44),
    ];

    #[test]
    fn follows_the_cursor_like_neovim() {
        let mismatches: Vec<String> = NVIM
            .iter()
            .filter_map(|&(rows, so, top, cursor, want)| {
                let got = follow_cursor(top, cursor, rows, so, 99);
                (got != want).then(|| {
                    format!("rows={rows} so={so} top={top} cursor={cursor}: got {got}, nvim {want}")
                })
            })
            .collect();
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    #[test]
    fn a_buffer_shorter_than_the_window_never_scrolls() {
        assert_eq!(follow_cursor(0, 5, 22, 8, 5), 0);
    }

    #[test]
    fn a_huge_scrolloff_centers_instead_of_overflowing() {
        assert_eq!(follow_cursor(0, 49, 22, usize::MAX, 99), 39);
        assert_eq!(follow_cursor(78, 59, 22, usize::MAX, 99), 49);
    }
}
