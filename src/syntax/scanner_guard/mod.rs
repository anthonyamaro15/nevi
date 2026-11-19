//! Refuses to parse Markdown text that would overflow the grammar scanner's
//! saved state.
//!
//! After every token tree-sitter asks the Markdown grammar's scanner to save
//! its state into a fixed 1024-byte buffer. The scanner in `tree-sitter-md`
//! 0.3.2 copies its whole stack of open blocks into it with no bounds check, so
//! deep enough input writes past the buffer and tree-sitter then aborts the
//! process. Later releases of the grammar truncate the copy instead. That is a C
//! `abort()`, not a panic: no cleanup runs, the terminal stays in raw mode and
//! unsaved buffers are lost. `markdown` computes, from the text alone, an upper
//! bound on how big that stack can get, and callers skip the parse (showing the
//! buffer without highlighting) when it is too big.

mod markdown;

#[cfg(test)]
mod tests;

/// Open blocks we let through. The real limit is 254 (see `markdown`), so this
/// leaves more than twice the room. Real documents stay far below it: even a
/// list nested ten deep with four-space indents needs about a dozen.
const MAX_OPEN_BLOCKS: usize = 100;

/// Whether parsing `source` as `language` could overflow the grammar
/// scanner's state buffer, so it must not be handed to tree-sitter.
pub(super) fn too_deep_to_parse(language: &str, source: &str) -> bool {
    language == "markdown" && markdown::open_blocks_bound(source.as_bytes()) > MAX_OPEN_BLOCKS
}
