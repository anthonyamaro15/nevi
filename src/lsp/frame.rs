//! `Content-Length` framing shared by the LSP and Copilot stdio readers.

use std::io::{self, BufRead, Read};

/// Largest message body accepted. Real servers do send big messages
/// (semantic tokens for a huge file, workspace symbols, long completion
/// lists) but they stay in the low tens of megabytes; 64 MiB leaves plenty of
/// headroom while refusing values that could only come from a broken stream.
pub(crate) const MAX_BODY_LEN: usize = 64 * 1024 * 1024;

/// Longest header line accepted. Real headers are under 100 bytes; the slack
/// keeps tolerating stray log lines some servers print to stdout, which have
/// always been skipped as unknown headers.
const MAX_HEADER_LINE: u64 = 64 * 1024;

/// Read one framed message body.
///
/// Any error means the stream can no longer be trusted: after a refused
/// header there is no reliable way to find where the next message starts, so
/// callers should treat the server as gone. EOF comes back as
/// `UnexpectedEof`, a bad or oversized frame as `InvalidData`.
pub(crate) fn read_frame<R: BufRead>(reader: &mut R) -> io::Result<Vec<u8>> {
    loop {
        let mut content_length = None;
        loop {
            let mut line = String::new();
            let n = reader.by_ref().take(MAX_HEADER_LINE).read_line(&mut line)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            if n as u64 == MAX_HEADER_LINE && !line.ends_with('\n') {
                return Err(invalid("header line too long"));
            }
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                let len: usize = value.parse().map_err(|_| invalid("bad Content-Length"))?;
                if len > MAX_BODY_LEN {
                    return Err(invalid("message too large"));
                }
                content_length = Some(len);
            }
        }

        // A header block without a length has always been skipped.
        let Some(len) = content_length else { continue };

        // Grow with the bytes that actually arrive, never with the claim.
        let mut body = Vec::new();
        reader.by_ref().take(len as u64).read_to_end(&mut body)?;
        if body.len() < len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        return Ok(body);
    }
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn frame(body: &str) -> String {
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
    }

    fn err_kind(input: &[u8]) -> io::ErrorKind {
        read_frame(&mut Cursor::new(input))
            .expect_err("frame should be refused")
            .kind()
    }

    #[test]
    fn reads_a_normal_frame() {
        let mut r = Cursor::new(frame(r#"{"id":1}"#));
        assert_eq!(read_frame(&mut r).unwrap(), br#"{"id":1}"#);
        assert_eq!(
            read_frame(&mut r).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn reads_two_frames_back_to_back() {
        let mut r = Cursor::new(format!("{}{}", frame("first"), frame("second")));
        assert_eq!(read_frame(&mut r).unwrap(), b"first");
        assert_eq!(read_frame(&mut r).unwrap(), b"second");
    }

    #[test]
    fn ignores_other_headers_before_content_length() {
        let input = "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n\
                     Content-Length: 2\r\n\r\n{}";
        assert_eq!(read_frame(&mut Cursor::new(input)).unwrap(), b"{}");
    }

    #[test]
    fn skips_a_header_block_without_a_length() {
        let input = format!("\r\n{}", frame("ok"));
        assert_eq!(read_frame(&mut Cursor::new(input)).unwrap(), b"ok");
    }

    // The huge values are above isize::MAX so that even an implementation
    // that sized a buffer from the header would fail with a capacity panic
    // instead of taking down the test process.
    #[test]
    fn refuses_a_huge_content_length() {
        for len in ["18446744073709551615", "9223372036854775808"] {
            let input = format!("Content-Length: {len}\r\n\r\n{{}}");
            assert_eq!(err_kind(input.as_bytes()), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn refuses_a_length_just_over_the_cap() {
        let input = format!("Content-Length: {}\r\n\r\n{{}}", MAX_BODY_LEN + 1);
        assert_eq!(err_kind(input.as_bytes()), io::ErrorKind::InvalidData);
    }

    #[test]
    fn accepts_a_length_exactly_at_the_cap() {
        let header = format!("Content-Length: {MAX_BODY_LEN}\r\n\r\n");
        let body = io::repeat(b'x').take(MAX_BODY_LEN as u64);
        let mut r = BufReader::new(Cursor::new(header).chain(body));
        assert_eq!(read_frame(&mut r).unwrap().len(), MAX_BODY_LEN);
    }

    #[test]
    fn refuses_a_non_numeric_length() {
        for len in ["abc", "-1", "12abc"] {
            let input = format!("Content-Length: {len}\r\n\r\n{}", frame("{}"));
            assert_eq!(err_kind(input.as_bytes()), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn reports_eof_when_the_body_is_short() {
        assert_eq!(
            err_kind(b"Content-Length: 100\r\n\r\n{\"id\":1}"),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn refuses_an_endless_header_line() {
        // Finite so a reader without the limit hits EOF rather than hanging.
        let mut input = vec![b'a'; MAX_HEADER_LINE as usize * 4];
        input.extend_from_slice(frame("{}").as_bytes());
        assert_eq!(err_kind(&input), io::ErrorKind::InvalidData);
    }
}
