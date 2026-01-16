use crate::editor::Buffer;

/// Represents a motion that can move the cursor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    // Character motions
    Left,
    Right,
    Up,
    Down,

    // Word motions
    WordForward,      // w
    WordBackward,     // b
    WordEnd,          // e
    BigWordForward,   // W
    BigWordBackward,  // B
    BigWordEnd,       // E

    // Line motions
    LineStart,        // 0
    FirstNonBlank,    // ^
    LineEnd,          // $

    // File motions
    FileStart,        // gg
    FileEnd,          // G
    GotoLine(usize),  // {count}G

    // Screen motions
    HalfPageDown,     // Ctrl-d
    HalfPageUp,       // Ctrl-u
    PageDown,         // Ctrl-f
    PageUp,           // Ctrl-b
    ScreenTop,        // H - top of screen
    ScreenMiddle,     // M - middle of screen
    ScreenBottom,     // L - bottom of screen

    // Find char motions
    FindChar(char),       // f{char}
    FindCharBack(char),   // F{char}
    TillChar(char),       // t{char}
    TillCharBack(char),   // T{char}

    // Paragraph motions
    ParagraphForward,     // }
    ParagraphBackward,    // {

    // Bracket matching
    MatchingBracket,      // %
}

/// Check if a character is a "word" character (alphanumeric or underscore)
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Check if a character is a "keyword" character (non-blank, non-word)
#[allow(dead_code)]
fn is_keyword_char(ch: char) -> bool {
    !ch.is_whitespace() && !is_word_char(ch)
}

/// Character classification for word motion
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Word,      // alphanumeric + underscore
    Keyword,   // punctuation, symbols
}

fn classify_char(ch: char) -> CharClass {
    if ch.is_whitespace() {
        CharClass::Whitespace
    } else if is_word_char(ch) {
        CharClass::Word
    } else {
        CharClass::Keyword
    }
}

/// Find the position after applying a motion
/// Returns (line, col) or None if motion is invalid
pub fn apply_motion(
    buffer: &Buffer,
    motion: Motion,
    line: usize,
    col: usize,
    count: usize,
    text_rows: usize,
) -> Option<(usize, usize)> {
    let count = count.max(1);

    match motion {
        Motion::Left => {
            Some((line, col.saturating_sub(count)))
        }

        Motion::Right => {
            let line_len = buffer.line_len(line);
            let new_col = (col + count).min(line_len.saturating_sub(1));
            Some((line, new_col))
        }

        Motion::Up => {
            Some((line.saturating_sub(count), col))
        }

        Motion::Down => {
            let max_line = buffer.len_lines().saturating_sub(1);
            let new_line = (line + count).min(max_line);
            Some((new_line, col))
        }

        Motion::WordForward => {
            let mut l = line;
            let mut c = col;
            for _ in 0..count {
                if let Some((nl, nc)) = find_word_forward(buffer, l, c, false) {
                    l = nl;
                    c = nc;
                } else {
                    break;
                }
            }
            Some((l, c))
        }

        Motion::BigWordForward => {
            let mut l = line;
            let mut c = col;
            for _ in 0..count {
                if let Some((nl, nc)) = find_word_forward(buffer, l, c, true) {
                    l = nl;
                    c = nc;
                } else {
                    break;
                }
            }
            Some((l, c))
        }

        Motion::WordBackward => {
            let mut l = line;
            let mut c = col;
            for _ in 0..count {
                if let Some((nl, nc)) = find_word_backward(buffer, l, c, false) {
                    l = nl;
                    c = nc;
                } else {
                    break;
                }
            }
            Some((l, c))
        }

        Motion::BigWordBackward => {
            let mut l = line;
            let mut c = col;
            for _ in 0..count {
                if let Some((nl, nc)) = find_word_backward(buffer, l, c, true) {
                    l = nl;
                    c = nc;
                } else {
                    break;
                }
            }
            Some((l, c))
        }

        Motion::WordEnd => {
            let mut l = line;
            let mut c = col;
            for _ in 0..count {
                if let Some((nl, nc)) = find_word_end(buffer, l, c, false) {
                    l = nl;
                    c = nc;
                } else {
                    break;
                }
            }
            Some((l, c))
        }

        Motion::BigWordEnd => {
            let mut l = line;
            let mut c = col;
            for _ in 0..count {
                if let Some((nl, nc)) = find_word_end(buffer, l, c, true) {
                    l = nl;
                    c = nc;
                } else {
                    break;
                }
            }
            Some((l, c))
        }

        Motion::LineStart => {
            Some((line, 0))
        }

        Motion::FirstNonBlank => {
            let first_non_blank = find_first_non_blank(buffer, line);
            Some((line, first_non_blank))
        }

        Motion::LineEnd => {
            let line_len = buffer.line_len(line);
            Some((line, line_len.saturating_sub(1)))
        }

        Motion::FileStart => {
            Some((0, 0))
        }

        Motion::FileEnd => {
            let last_line = buffer.len_lines().saturating_sub(1);
            Some((last_line, 0))
        }

        Motion::GotoLine(target) => {
            let target_line = target.saturating_sub(1).min(buffer.len_lines().saturating_sub(1));
            Some((target_line, 0))
        }

        Motion::HalfPageDown => {
            let half = text_rows / 2;
            let max_line = buffer.len_lines().saturating_sub(1);
            let new_line = (line + half * count).min(max_line);
            Some((new_line, col))
        }

        Motion::HalfPageUp => {
            let half = text_rows / 2;
            let new_line = line.saturating_sub(half * count);
            Some((new_line, col))
        }

        Motion::PageDown => {
            let max_line = buffer.len_lines().saturating_sub(1);
            let new_line = (line + text_rows * count).min(max_line);
            Some((new_line, col))
        }

        Motion::PageUp => {
            let new_line = line.saturating_sub(text_rows * count);
            Some((new_line, col))
        }

        Motion::FindChar(target) => {
            find_char_forward(buffer, line, col, target, count, false)
        }

        Motion::FindCharBack(target) => {
            find_char_backward(buffer, line, col, target, count, false)
        }

        Motion::TillChar(target) => {
            find_char_forward(buffer, line, col, target, count, true)
        }

        Motion::TillCharBack(target) => {
            find_char_backward(buffer, line, col, target, count, true)
        }

        Motion::ScreenTop => {
            // H - move to top of screen
            // Note: This requires the caller to provide viewport_offset (first visible line)
            // Since we don't have access to that here, we return (0, col) as placeholder
            // The actual implementation should be handled in terminal.rs where viewport info is available
            Some((0, col))
        }

        Motion::ScreenMiddle => {
            // M - move to middle of screen
            // Similar to ScreenTop, needs viewport info from caller
            let middle = text_rows / 2;
            Some((middle, col))
        }

        Motion::ScreenBottom => {
            // L - move to bottom of screen
            // Similar to ScreenTop, needs viewport info from caller
            let bottom = text_rows.saturating_sub(1);
            Some((bottom, col))
        }

        Motion::ParagraphForward => {
            // } - move to next paragraph (next blank line after non-blank content)
            let mut l = line;
            let total_lines = buffer.len_lines();

            // Apply count times
            for _ in 0..count {
                // Skip current blank lines
                while l < total_lines && is_blank_line(buffer, l) {
                    l += 1;
                }
                // Skip non-blank lines until we find a blank line
                while l < total_lines && !is_blank_line(buffer, l) {
                    l += 1;
                }
            }

            let target_line = l.min(total_lines.saturating_sub(1));
            Some((target_line, 0))
        }

        Motion::ParagraphBackward => {
            // { - move to previous paragraph (previous blank line before non-blank content)
            let mut l = line;

            // Apply count times
            for _ in 0..count {
                // If we're on line 0, stay there
                if l == 0 {
                    break;
                }

                // Move back one line to start
                l = l.saturating_sub(1);

                // Skip current blank lines
                while l > 0 && is_blank_line(buffer, l) {
                    l -= 1;
                }
                // Skip non-blank lines until we find a blank line
                while l > 0 && !is_blank_line(buffer, l) {
                    l -= 1;
                }
            }

            Some((l, 0))
        }

        Motion::MatchingBracket => {
            // % - jump to matching bracket
            find_matching_bracket(buffer, line, col)
        }
    }
}

/// Check if a line is blank (empty or only whitespace)
fn is_blank_line(buffer: &Buffer, line: usize) -> bool {
    let line_len = buffer.line_len(line);
    if line_len == 0 {
        return true;
    }
    for c in 0..line_len {
        if let Some(ch) = buffer.char_at(line, c) {
            if !ch.is_whitespace() {
                return false;
            }
        }
    }
    true
}

/// Find the matching bracket for the character at (line, col)
/// Supports (), [], {}, <>
fn find_matching_bracket(buffer: &Buffer, line: usize, col: usize) -> Option<(usize, usize)> {
    // First, find a bracket on or after the cursor on the current line
    let line_len = buffer.line_len(line);
    let mut search_col = col;
    let mut bracket_char = None;

    // Search for a bracket starting at cursor position
    while search_col < line_len {
        if let Some(ch) = buffer.char_at(line, search_col) {
            if is_bracket(ch) {
                bracket_char = Some((ch, search_col));
                break;
            }
        }
        search_col += 1;
    }

    let (bracket, start_col) = bracket_char?;

    // Determine direction and matching bracket
    let (is_open, matching) = match bracket {
        '(' => (true, ')'),
        ')' => (false, '('),
        '[' => (true, ']'),
        ']' => (false, '['),
        '{' => (true, '}'),
        '}' => (false, '{'),
        '<' => (true, '>'),
        '>' => (false, '<'),
        _ => return None,
    };

    if is_open {
        // Search forward for matching close bracket
        find_matching_forward(buffer, line, start_col, bracket, matching)
    } else {
        // Search backward for matching open bracket
        find_matching_backward(buffer, line, start_col, bracket, matching)
    }
}

/// Check if a character is a bracket
fn is_bracket(ch: char) -> bool {
    matches!(ch, '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>')
}

/// Search forward for matching bracket
fn find_matching_forward(buffer: &Buffer, start_line: usize, start_col: usize, open: char, close: char) -> Option<(usize, usize)> {
    let total_lines = buffer.len_lines();
    let mut depth = 0;

    for l in start_line..total_lines {
        let line_len = buffer.line_len(l);
        let start_c = if l == start_line { start_col } else { 0 };

        for c in start_c..line_len {
            if let Some(ch) = buffer.char_at(l, c) {
                if ch == open {
                    depth += 1;
                } else if ch == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some((l, c));
                    }
                }
            }
        }
    }
    None
}

/// Search backward for matching bracket
fn find_matching_backward(buffer: &Buffer, start_line: usize, start_col: usize, close: char, open: char) -> Option<(usize, usize)> {
    let mut depth = 0;

    for l in (0..=start_line).rev() {
        let line_len = buffer.line_len(l);
        let end_c = if l == start_line { start_col } else { line_len.saturating_sub(1) };

        if line_len == 0 {
            continue;
        }

        for c in (0..=end_c).rev() {
            if let Some(ch) = buffer.char_at(l, c) {
                if ch == close {
                    depth += 1;
                } else if ch == open {
                    depth -= 1;
                    if depth == 0 {
                        return Some((l, c));
                    }
                }
            }
        }
    }
    None
}

/// Find the start of the next word (w motion)
fn find_word_forward(buffer: &Buffer, line: usize, col: usize, big_word: bool) -> Option<(usize, usize)> {
    let mut l = line;
    let mut c = col;
    let total_lines = buffer.len_lines();

    // Get current character class
    let start_class = buffer.char_at(l, c).map(|ch| classify_char(ch));

    // Phase 1: Move past current word (same class characters)
    loop {
        if l >= total_lines {
            return Some((total_lines.saturating_sub(1), 0));
        }

        let line_len = buffer.line_len(l);

        if c >= line_len {
            // Move to next line
            l += 1;
            c = 0;
            break;
        }

        if let Some(ch) = buffer.char_at(l, c) {
            let class = classify_char(ch);
            let same_class = if big_word {
                // For WORD, only whitespace breaks
                class != CharClass::Whitespace && start_class.map_or(false, |sc| sc != CharClass::Whitespace)
            } else {
                // For word, same class continues
                Some(class) == start_class && class != CharClass::Whitespace
            };

            if same_class {
                c += 1;
            } else {
                break;
            }
        } else {
            l += 1;
            c = 0;
            break;
        }
    }

    // Phase 2: Skip whitespace
    loop {
        if l >= total_lines {
            return Some((total_lines.saturating_sub(1), 0));
        }

        let line_len = buffer.line_len(l);

        if c >= line_len {
            // Move to next line
            l += 1;
            c = 0;
            continue;
        }

        if let Some(ch) = buffer.char_at(l, c) {
            if ch.is_whitespace() && ch != '\n' {
                c += 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // Clamp to valid position
    if l >= total_lines {
        l = total_lines.saturating_sub(1);
        c = 0;
    }

    Some((l, c))
}

/// Find the start of the previous word (b motion)
fn find_word_backward(buffer: &Buffer, line: usize, col: usize, big_word: bool) -> Option<(usize, usize)> {
    let mut l = line;
    let mut c = col;

    // Move back one character to start
    if c == 0 {
        if l == 0 {
            return Some((0, 0));
        }
        l -= 1;
        c = buffer.line_len(l).saturating_sub(1);
    } else {
        c -= 1;
    }

    // Phase 1: Skip whitespace going backward
    loop {
        if let Some(ch) = buffer.char_at(l, c) {
            if ch.is_whitespace() && ch != '\n' {
                if c == 0 {
                    if l == 0 {
                        return Some((0, 0));
                    }
                    l -= 1;
                    c = buffer.line_len(l).saturating_sub(1);
                } else {
                    c -= 1;
                }
            } else if ch == '\n' || buffer.line_len(l) == 0 {
                if l == 0 {
                    return Some((0, 0));
                }
                l -= 1;
                c = buffer.line_len(l).saturating_sub(1);
            } else {
                break;
            }
        } else {
            if l == 0 {
                return Some((0, 0));
            }
            l -= 1;
            c = buffer.line_len(l).saturating_sub(1);
        }
    }

    // Get the class of the character we landed on
    let target_class = buffer.char_at(l, c).map(|ch| classify_char(ch))?;

    // Phase 2: Move back through same-class characters
    loop {
        if c == 0 {
            break;
        }

        let prev_c = c - 1;
        if let Some(ch) = buffer.char_at(l, prev_c) {
            let class = classify_char(ch);
            let same_class = if big_word {
                class != CharClass::Whitespace
            } else {
                class == target_class
            };

            if same_class {
                c = prev_c;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    Some((l, c))
}

/// Find the end of the current/next word (e motion)
fn find_word_end(buffer: &Buffer, line: usize, col: usize, big_word: bool) -> Option<(usize, usize)> {
    let mut l = line;
    let mut c = col;
    let total_lines = buffer.len_lines();

    // Move forward one character first
    c += 1;
    let line_len = buffer.line_len(l);
    if c >= line_len {
        l += 1;
        c = 0;
    }

    if l >= total_lines {
        return Some((total_lines.saturating_sub(1), buffer.line_len(total_lines.saturating_sub(1)).saturating_sub(1)));
    }

    // Phase 1: Skip whitespace
    loop {
        if l >= total_lines {
            return Some((total_lines.saturating_sub(1), 0));
        }

        let line_len = buffer.line_len(l);
        if c >= line_len {
            l += 1;
            c = 0;
            continue;
        }

        if let Some(ch) = buffer.char_at(l, c) {
            if ch.is_whitespace() && ch != '\n' {
                c += 1;
            } else if ch == '\n' || line_len == 0 {
                l += 1;
                c = 0;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    if l >= total_lines {
        return Some((total_lines.saturating_sub(1), 0));
    }

    // Get the class of the character we're on
    let target_class = buffer.char_at(l, c).map(|ch| classify_char(ch))?;

    // Phase 2: Move forward through same-class characters
    loop {
        let next_c = c + 1;
        let line_len = buffer.line_len(l);

        if next_c >= line_len {
            break;
        }

        if let Some(ch) = buffer.char_at(l, next_c) {
            let class = classify_char(ch);
            let same_class = if big_word {
                class != CharClass::Whitespace
            } else {
                class == target_class
            };

            if same_class {
                c = next_c;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    Some((l, c))
}

/// Find the first non-blank character on a line
fn find_first_non_blank(buffer: &Buffer, line: usize) -> usize {
    let line_len = buffer.line_len(line);
    for c in 0..line_len {
        if let Some(ch) = buffer.char_at(line, c) {
            if !ch.is_whitespace() {
                return c;
            }
        }
    }
    0
}

/// Find character forward on the same line (f/t motions)
/// If `till` is true, stop one position before the character (t motion)
fn find_char_forward(
    buffer: &Buffer,
    line: usize,
    col: usize,
    target: char,
    count: usize,
    till: bool,
) -> Option<(usize, usize)> {
    let line_len = buffer.line_len(line);
    let mut found_count = 0;

    // Search forward from col+1 to end of line
    for c in (col + 1)..line_len {
        if let Some(ch) = buffer.char_at(line, c) {
            if ch == target {
                found_count += 1;
                if found_count == count {
                    let result_col = if till { c.saturating_sub(1) } else { c };
                    // For till, don't move if we'd stay at the same position
                    if till && result_col <= col {
                        return None;
                    }
                    return Some((line, result_col));
                }
            }
        }
    }

    // Character not found (or not enough occurrences)
    None
}

/// Find character backward on the same line (F/T motions)
/// If `till` is true, stop one position after the character (T motion)
fn find_char_backward(
    buffer: &Buffer,
    line: usize,
    col: usize,
    target: char,
    count: usize,
    till: bool,
) -> Option<(usize, usize)> {
    if col == 0 {
        return None;
    }

    let mut found_count = 0;

    // Search backward from col-1 to start of line
    for c in (0..col).rev() {
        if let Some(ch) = buffer.char_at(line, c) {
            if ch == target {
                found_count += 1;
                if found_count == count {
                    let result_col = if till { c + 1 } else { c };
                    // For till, don't move if we'd stay at the same position
                    if till && result_col >= col {
                        return None;
                    }
                    return Some((line, result_col));
                }
            }
        }
    }

    // Character not found (or not enough occurrences)
    None
}
