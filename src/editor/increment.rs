//! `Ctrl-a` / `Ctrl-x`: add to or subtract from the number at or after the
//! cursor, following Neovim's default `nrformats=bin,hex`.
//!
//! Decimal numbers take a `-` directly before them as their sign, `0x` hex
//! and `0b` binary numbers never do. A number with leading zeros keeps its
//! width, hex digits follow the case of the letters already present, and
//! decimal results cross zero while hex and binary wrap as unsigned 64-bit.
//! The search for the number mirrors Vim's `do_addsub`: walk back over hex
//! and binary digits to catch a cursor sitting inside `0x..`, otherwise take
//! the first digit at or after the cursor and extend backwards.

/// The span of the old number (in chars) and the text that replaces it.
#[derive(Debug, PartialEq, Eq)]
pub struct NumberChange {
    pub start: usize,
    pub len: usize,
    pub text: String,
}

impl NumberChange {
    /// Vim leaves the cursor on the last char of the new number.
    pub fn cursor_col(&self) -> usize {
        self.start + self.text.chars().count() - 1
    }
}

pub fn add_to_number(chars: &[char], cursor: usize, delta: i64) -> Option<NumberChange> {
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    let is_digit = |c: char| c.is_ascii_digit();
    let is_xdigit = |c: char| c.is_ascii_hexdigit();
    let is_bdigit = |c: char| c == '0' || c == '1';
    let hex_prefix_at =
        |i: usize| i > 0 && matches!(at(i), 'x' | 'X') && at(i - 1) == '0' && is_xdigit(at(i + 1));
    let bin_prefix_at =
        |i: usize| i > 0 && matches!(at(i), 'b' | 'B') && at(i - 1) == '0' && is_bdigit(at(i + 1));

    // Normal mode never puts the cursor past the last char; clamp so any
    // caller gets Vim's answer instead of a decimal read of the tail.
    let cursor = cursor.min(chars.len().saturating_sub(1));
    let mut col = cursor;
    while col > 0 && is_bdigit(at(col)) {
        col -= 1;
    }
    while col > 0 && is_xdigit(at(col)) {
        col -= 1;
    }
    if !hex_prefix_at(col) {
        // Binary digits are also hex digits, so the walk above can overshoot
        // a plain decimal; rescan over decimal digits only.
        col = cursor;
        while col > 0 && is_digit(at(col)) {
            col -= 1;
        }
    }
    if hex_prefix_at(col) || bin_prefix_at(col) {
        col -= 1;
    } else {
        col = cursor;
        while col < chars.len() && !is_digit(at(col)) {
            col += 1;
        }
        while col > 0 && is_digit(at(col - 1)) {
            col -= 1;
        }
    }

    let first_digit = at(col);
    if !is_digit(first_digit) {
        return None;
    }

    let (radix, prefix_len) = if at(col) == '0' && hex_prefix_at(col + 1) {
        (16, 2)
    } else if at(col) == '0' && bin_prefix_at(col + 1) {
        (2, 2)
    } else {
        (10, 0)
    };
    let digits_start = col + prefix_len;
    let mut digits_end = digits_start;
    while digits_end < chars.len() && at(digits_end).is_digit(radix) {
        digits_end += 1;
    }
    let old_digits: String = chars[digits_start..digits_end].iter().collect();
    // Vim clamps a number that does not fit instead of failing.
    let n = u64::from_str_radix(&old_digits, radix).unwrap_or(u64::MAX);

    let negative = radix == 10 && col > 0 && at(col - 1) == '-';
    let (new_negative, new_n) = if radix == 10 {
        let signed = if negative { -(n as i128) } else { n as i128 } + i128::from(delta);
        (
            signed < 0,
            signed.unsigned_abs().min(u128::from(u64::MAX)) as u64,
        )
    } else {
        (false, n.wrapping_add_signed(delta))
    };

    // The last letter in the old number decides the case, and the x/X of the
    // prefix counts as a letter, so `0X1` produces uppercase digits.
    let hex_upper = chars[col..digits_end]
        .iter()
        .filter(|c| c.is_ascii_alphabetic())
        .next_back()
        .is_some_and(|c| c.is_ascii_uppercase());
    let new_digits = match radix {
        2 => format!("{new_n:b}"),
        16 if hex_upper => format!("{new_n:X}"),
        16 => format!("{new_n:x}"),
        _ => new_n.to_string(),
    };

    let start = if negative { col - 1 } else { col };
    let mut text = String::new();
    if new_negative {
        text.push('-');
    }
    text.extend(&chars[col..digits_start]);
    if first_digit == '0' {
        let width = digits_end - digits_start;
        text.extend(std::iter::repeat_n(
            '0',
            width.saturating_sub(new_digits.len()),
        ));
    }
    text.push_str(&new_digits);

    Some(NumberChange {
        start,
        len: digits_end - start,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::{NumberChange, add_to_number};

    fn apply(line: &str, cursor: usize, delta: i64) -> Option<(String, usize)> {
        let chars: Vec<char> = line.chars().collect();
        let change = add_to_number(&chars, cursor, delta)?;
        let mut out: String = chars[..change.start].iter().collect();
        out.push_str(&change.text);
        out.extend(&chars[change.start + change.len..]);
        Some((out, change.cursor_col()))
    }

    #[test]
    fn finds_number_at_or_after_cursor() {
        assert_eq!(apply("x = 5", 0, 1), Some(("x = 6".into(), 4)));
        assert_eq!(apply("abc 123 def", 5, 1), Some(("abc 124 def".into(), 6)));
        assert_eq!(apply("1 2", 1, 1), Some(("1 3".into(), 2)));
        assert_eq!(apply("12 abc", 5, 1), None);
        assert_eq!(apply("", 0, 1), None);
    }

    #[test]
    fn decimal_sign_crosses_zero() {
        assert_eq!(apply("-1", 0, 1), Some(("0".into(), 0)));
        assert_eq!(apply("0", 0, -1), Some(("-1".into(), 1)));
        assert_eq!(apply("10", 0, -15), Some(("-5".into(), 1)));
        assert_eq!(apply("x-5", 0, -1), Some(("x-6".into(), 2)));
        assert_eq!(apply("-5", 1, 1), Some(("-4".into(), 1)));
    }

    #[test]
    fn leading_zeros_keep_width() {
        assert_eq!(apply("007", 0, 1), Some(("008".into(), 2)));
        assert_eq!(apply("099", 0, 1), Some(("100".into(), 2)));
        assert_eq!(apply("-007", 0, 1), Some(("-006".into(), 3)));
        assert_eq!(apply("0x00ff", 0, 1), Some(("0x0100".into(), 5)));
    }

    #[test]
    fn hex_and_binary() {
        assert_eq!(apply("0x0f", 0, 1), Some(("0x10".into(), 3)));
        assert_eq!(apply("0xff", 0, 1), Some(("0x100".into(), 4)));
        assert_eq!(apply("0XAB", 0, -1), Some(("0XAA".into(), 3)));
        assert_eq!(apply("0X9", 0, 1), Some(("0XA".into(), 2)));
        assert_eq!(apply("0x1f", 3, 1), Some(("0x20".into(), 3)));
        assert_eq!(apply("0x0f", 1, 1), Some(("0x10".into(), 3)));
        assert_eq!(apply("-0x10", 0, 1), Some(("-0x11".into(), 4)));
        assert_eq!(apply("0x0", 0, -1), Some(("0xffffffffffffffff".into(), 17)));
        assert_eq!(apply("0b101", 0, 1), Some(("0b110".into(), 4)));
        assert_eq!(apply("0b11", 3, 1), Some(("0b100".into(), 4)));
        assert_eq!(apply("0b11", 9, 1), Some(("0b100".into(), 4)));
    }

    #[test]
    fn change_reports_span_and_cursor() {
        let chars: Vec<char> = "a -12 b".chars().collect();
        assert_eq!(
            add_to_number(&chars, 0, 12),
            Some(NumberChange {
                start: 2,
                len: 3,
                text: "0".into(),
            })
        );
    }
}
