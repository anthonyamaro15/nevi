//! Keeps text drawn in the UI from turning into terminal commands.
//!
//! File names, language server replies, status messages and the like can hold
//! control characters, and an ESC among them would start an escape sequence
//! (clipboard writes, title changes, a redrawn screen). Every formatted write
//! (`write!`, `terminal_print!`) goes through `ChromeWriter::write_fmt`, which
//! draws each control character as one space, the way buffer text does.
//!
//! Raw byte writes (`write_all`) pass through untouched. crossterm's
//! `queue!`/`execute!` use them for the escape sequences we do mean to send,
//! so code that emits its own sequence must write bytes, not format them.
//!
//! Widths agree with `crate::finder::row::cell_width`, which counts a control
//! character as one cell.

use std::fmt;
use std::io::{self, Write};

pub(super) struct ChromeWriter<W: Write>(pub(super) W);

impl<W: Write> Write for ChromeWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write_all(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }

    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> io::Result<()> {
        struct Adapter<'a, W: Write> {
            inner: &'a mut W,
            result: io::Result<()>,
        }

        impl<W: Write> fmt::Write for Adapter<'_, W> {
            fn write_str(&mut self, text: &str) -> fmt::Result {
                write_without_controls(self.inner, text).map_err(|err| {
                    self.result = Err(err);
                    fmt::Error
                })
            }
        }

        let mut adapter = Adapter {
            inner: &mut self.0,
            result: Ok(()),
        };
        match fmt::write(&mut adapter, args) {
            Ok(()) => Ok(()),
            Err(_) => adapter.result.and(Err(io::Error::other("formatter error"))),
        }
    }
}

/// Writes `text` with each control character as a space. Clean text, the
/// common case on every frame, goes out in one write with no allocation.
fn write_without_controls<W: Write>(writer: &mut W, text: &str) -> io::Result<()> {
    let mut rest = text;
    while let Some((idx, ch)) = rest.char_indices().find(|(_, ch)| ch.is_control()) {
        writer.write_all(rest[..idx].as_bytes())?;
        writer.write_all(b" ")?;
        rest = &rest[idx + ch.len_utf8()..];
    }
    writer.write_all(rest.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::ChromeWriter;
    use std::io::Write;

    #[test]
    fn formatted_text_draws_control_characters_as_spaces() {
        let mut out = ChromeWriter(Vec::new());
        write!(out, "a\x1b]0;PWN\x07b{}", "\t\n\r\u{7f}\u{9b}é").unwrap();
        assert_eq!(String::from_utf8(out.0).unwrap(), "a ]0;PWN b     é");
    }

    #[test]
    fn raw_bytes_pass_through_for_intended_escape_sequences() {
        let mut out = ChromeWriter(Vec::new());
        crossterm::queue!(out, crossterm::cursor::MoveTo(0, 0)).unwrap();
        out.write_all(b"\x1b[4m").unwrap();
        assert_eq!(out.0, b"\x1b[1;1H\x1b[4m");
    }
}
