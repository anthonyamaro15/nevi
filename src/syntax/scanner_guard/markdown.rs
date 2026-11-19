//! Bound for the block scanner of `tree-sitter-md` 0.3.2
//! (`tree-sitter-markdown/src/scanner.c`).
//!
//! `serialize` writes 5 header bytes and then 4 bytes per open block, so more
//! than 254 open blocks overflow the 1024-byte buffer.
//!
//! What opens a block (`push_block`): `>` opens a block quote; `-`, `+` or `*`
//! followed by a space, tab or line end, and 1 to 9 digits plus `.` or `)`
//! followed by the same, open a list item. Indented code, fenced code and HTML
//! blocks are leaves: nothing opens inside them, so at most one sits on top.
//! Lines end at `\n` or `\r`.
//!
//! At the start of each line the scanner re-matches the open blocks in order:
//! a block quote needs another `>`, a list item at least 2 columns of
//! indentation (or a blank line), indented code 4 columns, and fenced code and
//! HTML blocks match anything. A tab counts up to 4 columns. When a line opens
//! a new block, the blocks that failed to match are popped first (BLOCK_CLOSE
//! in `scan`). A lazy continuation line keeps every open block but opens
//! nothing: a block start there makes the grammar request an error, and the
//! scanner returns the error token without pushing.
//!
//! So after each line the stack is either the same as after the previous line,
//! or (blocks matched on this line) + (blocks opened on it) + at most one leaf.
//! Each `>` on the line either matches a block quote or opens one. Every other
//! matched block is a list item or indented code, which takes at least 2
//! columns of the line's indentation, and is one of the blocks already open.
//! Every other opened block needs a list marker. Counting tabs as a full 4
//! columns and every marker-shaped prefix as a list item only over-counts, so
//! `bound` below never falls under the real stack.

/// Upper bound on how many blocks the scanner can hold open at once while
/// parsing `text`. Scans each line's leading run of indentation and block
/// markers only.
pub(super) fn open_blocks_bound(text: &[u8]) -> usize {
    let mut bound = 0;
    let mut i = 0;
    loop {
        let (mut quotes, mut markers, mut columns) = (0, 0, 0);
        loop {
            match text.get(i) {
                Some(b' ') => columns += 1,
                Some(b'\t') => columns += 4,
                Some(b'>') => quotes += 1,
                Some(b'-' | b'+' | b'*') if ends_marker(text.get(i + 1)) => markers += 1,
                Some(b'0'..=b'9') => {
                    let digits = text[i..].iter().take_while(|b| b.is_ascii_digit()).count();
                    let end = i + digits;
                    if digits > 9
                        || !matches!(text.get(end), Some(b'.' | b')'))
                        || !ends_marker(text.get(end + 1))
                    {
                        break;
                    }
                    markers += 1;
                    i = end;
                }
                _ => break,
            }
            i += 1;
        }
        bound = bound.max(quotes + bound.min(columns / 2) + markers + 1);
        match text[i..].iter().position(|&b| b == b'\n' || b == b'\r') {
            Some(offset) => i += offset + 1,
            None => return bound,
        }
    }
}

/// A list marker needs whitespace or the line end after it. End of input is
/// accepted too, which only over-counts.
fn ends_marker(next: Option<&u8>) -> bool {
    matches!(next, None | Some(b' ' | b'\t' | b'\n' | b'\r'))
}
