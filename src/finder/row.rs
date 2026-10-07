//! Live grep rows, laid out when they're drawn, once the width is known.
//!
//! Widths are terminal cells: wide characters such as CJK take two, and
//! control characters one, because rows draw them as blanks.

use unicode_width::UnicodeWidthChar;

/// Cells `ch` takes in a finder row.
pub(crate) fn cell_width(ch: char) -> usize {
    if ch.is_control() {
        1
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0)
    }
}

/// Cells `text` takes in a finder row.
pub(crate) fn text_width(text: &str) -> usize {
    text.chars().map(cell_width).sum()
}

/// A live grep hit laid out in `width` cells: its path, `:<line>: `, then
/// the line's text, each char paired with whether it's part of a match.
/// `match_indices` are sorted char indices into `text`.
pub(crate) fn grep_row(
    path: &str,
    line: usize,
    text: &str,
    match_indices: &[usize],
    width: usize,
) -> Vec<(char, bool)> {
    let tag = format!(":{line}: ");
    let (path, text_start) = fit_grep_row(path, &tag, text, first_run(match_indices), width);
    let mut cells: Vec<(char, bool)> = path
        .chars()
        .chain(tag.chars())
        .map(|ch| (ch, false))
        .collect();
    if text_start > 0 {
        cells.push(('…', false));
    }
    cells.extend(
        text.chars()
            .enumerate()
            .skip(text_start)
            .map(|(idx, ch)| (ch, match_indices.binary_search(&idx).is_ok())),
    );

    let mut used = 0;
    let mut fits = 0;
    for &(ch, _) in &cells {
        used += cell_width(ch);
        if used > width {
            break;
        }
        fits += 1;
    }
    cells.truncate(fits);
    cells
}

/// Decides where a row gives way when it doesn't fit, in this order:
///
/// 1. folders shrink to their first letter, from the left, only until the
///    text up to the end of the first match fits (like Vim's `pathshorten()`);
/// 2. the text before the match is cut behind a "…", so the match moves left;
/// 3. the path never leaves the text less than a third of the row (or than
///    the match needs, if that's less): past that it drops folders from the
///    left, then the start of the file name, behind "…".
///
/// Returns the path to show and the char of `text` the row starts from;
/// above 0 the row draws a "…" first. What still doesn't fit is cut at the
/// right edge.
fn fit_grep_row(
    path: &str,
    tag: &str,
    text: &str,
    first_match: Option<(usize, usize)>,
    width: usize,
) -> (String, usize) {
    let (match_start, match_end) = first_match.unwrap_or((0, 0));
    let room = width.saturating_sub(text_width(tag));
    let to_match_end: usize = text.chars().take(match_end).map(cell_width).sum();
    let mut path = shorten_folders(path, room.saturating_sub(to_match_end));

    let match_alone = if match_start == 0 {
        to_match_end
    } else {
        let match_width: usize = text
            .chars()
            .skip(match_start)
            .take(match_end - match_start)
            .map(cell_width)
            .sum();
        1 + match_width
    };
    let path_room = room.saturating_sub(match_alone.min(width / 3));
    if text_width(&path) > path_room {
        path = cut_left(&path, path_room);
    }

    let text_room = room.saturating_sub(text_width(&path));
    let mut needed = to_match_end;
    let mut text_start = 0;
    let mut chars = text.chars();
    while text_start < match_start && needed + usize::from(text_start > 0) > text_room {
        needed -= chars.next().map_or(0, cell_width);
        text_start += 1;
    }
    (path, text_start)
}

/// `path` with its folders shrunk to their first letter, from the left,
/// until it fits `budget` cells or every folder is shrunk. A folder that
/// starts with "." keeps the dot and one letter, like Vim's `pathshorten()`.
/// The file name stays whole.
fn shorten_folders(path: &str, budget: usize) -> String {
    let mut parts: Vec<&str> = path.split('/').collect();
    let folders = parts.len() - 1;
    let mut width = text_width(path);
    for part in &mut parts[..folders] {
        if width <= budget {
            break;
        }
        let name: &str = part;
        let keep = if name.starts_with('.') { 2 } else { 1 };
        let short = name
            .char_indices()
            .nth(keep)
            .map_or(name, |(end, _)| &name[..end]);
        width -= text_width(name) - text_width(short);
        *part = short;
    }
    parts.join("/")
}

/// The end of `path` that fits `budget` cells behind a "…": whole folders
/// and the file name when they fit, else the end of the file name. Empty
/// when `budget` is 0.
pub(crate) fn cut_left(path: &str, budget: usize) -> String {
    if text_width(path) <= budget {
        return path.to_string();
    }
    // The longest tail that starts at a "/" and fits after the "…".
    let mut kept = 0;
    let mut tail = None;
    for (idx, ch) in path.char_indices().rev() {
        if ch == '/' {
            if 2 + kept > budget {
                break;
            }
            tail = Some(idx);
        }
        kept += cell_width(ch);
    }
    if let Some(idx) = tail {
        return format!("…{}", &path[idx..]);
    }
    if budget == 0 {
        return String::new();
    }
    // Not even the file name fits: keep as much of its end as there's room for.
    let mut kept = 1;
    let mut start = path.len();
    for (idx, ch) in path.char_indices().rev() {
        kept += cell_width(ch);
        if kept > budget {
            break;
        }
        start = idx;
    }
    format!("…{}", &path[start..])
}

/// The first run of consecutive indices, as a range.
fn first_run(indices: &[usize]) -> Option<(usize, usize)> {
    let (&start, rest) = indices.split_first()?;
    let len = rest
        .iter()
        .zip(start + 1..)
        .take_while(|&(&idx, next)| idx == next)
        .count();
    Some((start, start + 1 + len))
}

#[cfg(test)]
mod tests {
    use super::{cut_left, grep_row, shorten_folders, text_width};

    // An iOS-style hit like the ones in #351. The match "paragraph_terms" is
    // chars 22..37 of the text.
    const DEEP_PATH: &str = "domain/CustomerAccount/AccountRegistrationFeature/Sources/Resources/Localization/en.lproj/Localizable.strings";
    const TEXT: &str = r#""account_registration_paragraph_termsOfUse" = "By registering you agree to the Terms of Use";"#;

    fn row(path: &str, text: &str, matches: std::ops::Range<usize>, width: usize) -> String {
        let indices: Vec<usize> = matches.collect();
        grep_row(path, 135, text, &indices, width)
            .into_iter()
            .map(|(ch, _)| ch)
            .collect()
    }

    #[test]
    fn a_row_that_fits_is_left_alone() {
        assert_eq!(
            row("src/main.rs", "fn main() {}", 3..7, 80),
            "src/main.rs:135: fn main() {}"
        );
    }

    #[test]
    fn folders_give_way_from_the_left_until_the_match_fits() {
        assert_eq!(
            row(DEEP_PATH, TEXT, 22..37, 100),
            r#"d/C/A/S/R/Localization/en.lproj/Localizable.strings:135: "account_registration_paragraph_termsOfUse""#
        );
    }

    #[test]
    fn the_text_before_the_match_goes_once_every_folder_is_a_letter() {
        assert_eq!(
            row(DEEP_PATH, TEXT, 22..37, 60),
            "d/C/A/S/R/L/e/Localizable.strings:135: …tion_paragraph_terms"
        );
    }

    #[test]
    fn the_text_keeps_a_third_of_a_narrow_row() {
        assert_eq!(
            row(DEEP_PATH, TEXT, 22..37, 40),
            "…/Localizable.strings:135: …paragraph_te"
        );
    }

    #[test]
    fn every_width_fits_and_shows_the_match_once_there_is_room() {
        // Letters-only path (33) + ":135: " (6) + "…" and the match (16).
        for width in 0..=130 {
            let shown = row(DEEP_PATH, TEXT, 22..37, width);
            assert!(text_width(&shown) <= width, "width {width}: {shown:?}");
            if width >= 55 {
                assert!(
                    shown.contains("paragraph_terms"),
                    "width {width}: {shown:?}"
                );
            }
        }
    }

    #[test]
    fn highlights_follow_the_text_after_a_cut() {
        let cells = grep_row(DEEP_PATH, 135, TEXT, &(22..37).collect::<Vec<_>>(), 60);
        let highlighted: String = cells
            .iter()
            .filter(|cell| cell.1)
            .map(|cell| cell.0)
            .collect();
        assert_eq!(highlighted, "paragraph_terms");
    }

    #[test]
    fn a_row_without_a_match_only_shrinks_to_fit() {
        assert_eq!(
            row(DEEP_PATH, "plain text", 0..0, 80),
            "d/C/A/Sources/Resources/Localization/en.lproj/Localizable.strings:135: plain tex"
        );
    }

    #[test]
    fn wide_characters_take_two_columns() {
        let path = "設定/画面/ファイル.txt";
        let text = "名前 = needle";
        assert_eq!(
            row(path, text, 5..11, 39),
            "設/画面/ファイル.txt:135: 名前 = needle"
        );
        assert_eq!(
            row(path, text, 5..11, 37),
            "設/画/ファイル.txt:135: 名前 = needle"
        );
        for width in 0..=60 {
            let shown = row(path, text, 5..11, width);
            assert!(text_width(&shown) <= width, "width {width}: {shown:?}");
        }
    }

    #[test]
    fn dot_folders_keep_the_dot_and_one_letter() {
        assert_eq!(
            shorten_folders(".github/workflows/rust.yml", 0),
            ".g/w/rust.yml"
        );
    }

    #[test]
    fn cut_left_keeps_whole_folders_then_the_end_of_the_name() {
        let path = "d/C/A/Localizable.strings";
        assert_eq!(cut_left(path, 25), path);
        assert_eq!(cut_left(path, 24), "…/A/Localizable.strings");
        assert_eq!(cut_left(path, 21), "…/Localizable.strings");
        assert_eq!(cut_left(path, 8), "…strings");
        assert_eq!(cut_left(path, 0), "");
        for budget in 0..=30 {
            assert!(text_width(&cut_left("設定/ファイル名がとても長い.txt", budget)) <= budget);
        }
    }
}
