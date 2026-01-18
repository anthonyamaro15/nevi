//! UTF-16 ↔ UTF-8 position conversion utilities
//!
//! Copilot uses UTF-16 code units for ALL positions, including:
//! - Document positions (line, character)
//! - Completion ranges
//! - acceptedLength in telemetry
//!
//! This module provides conversion functions between UTF-8 byte offsets
//! (used internally by the editor) and UTF-16 code units (used by Copilot).

/// Convert a UTF-8 column offset to UTF-16 code units
///
/// # Arguments
/// * `line` - The line content as a string slice
/// * `utf8_col` - Column position in UTF-8 bytes/chars
///
/// # Returns
/// The equivalent column position in UTF-16 code units
pub fn utf8_to_utf16_col(line: &str, utf8_col: usize) -> u32 {
    let mut utf16_col = 0u32;
    let mut char_count = 0usize;

    for ch in line.chars() {
        if char_count >= utf8_col {
            break;
        }
        // Count UTF-16 code units for this character
        // Characters in BMP (U+0000 to U+FFFF) are 1 code unit
        // Characters outside BMP (U+10000 and above) are 2 code units (surrogate pair)
        utf16_col += ch.len_utf16() as u32;
        char_count += 1;
    }

    utf16_col
}

/// Convert a UTF-16 column offset to UTF-8 character count
///
/// # Arguments
/// * `line` - The line content as a string slice
/// * `utf16_col` - Column position in UTF-16 code units
///
/// # Returns
/// The equivalent column position in character count (for editor use)
pub fn utf16_to_utf8_col(line: &str, utf16_col: u32) -> usize {
    let mut current_utf16 = 0u32;
    let mut char_count = 0usize;

    for ch in line.chars() {
        if current_utf16 >= utf16_col {
            break;
        }
        current_utf16 += ch.len_utf16() as u32;
        char_count += 1;
    }

    char_count
}

/// Calculate the UTF-16 length of a string
///
/// This is used for the `acceptedLength` field in telemetry
/// which reports how much of a completion was accepted.
///
/// # Arguments
/// * `s` - The string to measure
///
/// # Returns
/// The length in UTF-16 code units
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(|c| c.len_utf16()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_conversion() {
        let line = "hello world";
        assert_eq!(utf8_to_utf16_col(line, 0), 0);
        assert_eq!(utf8_to_utf16_col(line, 5), 5);
        assert_eq!(utf8_to_utf16_col(line, 11), 11);

        assert_eq!(utf16_to_utf8_col(line, 0), 0);
        assert_eq!(utf16_to_utf8_col(line, 5), 5);
        assert_eq!(utf16_to_utf8_col(line, 11), 11);
    }

    #[test]
    fn test_bmp_unicode() {
        // BMP characters (1 UTF-16 code unit each)
        let line = "héllo wörld"; // accented characters
        assert_eq!(utf8_to_utf16_col(line, 0), 0);
        assert_eq!(utf8_to_utf16_col(line, 1), 1); // 'h'
        assert_eq!(utf8_to_utf16_col(line, 2), 2); // 'é' (1 char = 1 UTF-16 unit)

        assert_eq!(utf16_to_utf8_col(line, 0), 0);
        assert_eq!(utf16_to_utf8_col(line, 1), 1);
        assert_eq!(utf16_to_utf8_col(line, 2), 2);
    }

    #[test]
    fn test_emoji_surrogate_pair() {
        // Emoji outside BMP (2 UTF-16 code units)
        let line = "a😀b"; // 'a' (1) + '😀' (2) + 'b' (1) = 4 UTF-16 units

        assert_eq!(utf8_to_utf16_col(line, 0), 0); // start
        assert_eq!(utf8_to_utf16_col(line, 1), 1); // after 'a'
        assert_eq!(utf8_to_utf16_col(line, 2), 3); // after '😀' (takes 2 UTF-16 units)
        assert_eq!(utf8_to_utf16_col(line, 3), 4); // after 'b'

        assert_eq!(utf16_to_utf8_col(line, 0), 0);
        assert_eq!(utf16_to_utf8_col(line, 1), 1); // 'a'
        assert_eq!(utf16_to_utf8_col(line, 3), 2); // after emoji
        assert_eq!(utf16_to_utf8_col(line, 4), 3); // 'b'
    }

    #[test]
    fn test_utf16_len() {
        assert_eq!(utf16_len("hello"), 5);
        assert_eq!(utf16_len("😀"), 2); // surrogate pair
        assert_eq!(utf16_len("a😀b"), 4);
        assert_eq!(utf16_len("héllo"), 5); // BMP chars
    }
}
