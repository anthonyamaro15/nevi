use std::fs;
use std::path::Path;

use similar::{ChangeTag, TextDiff};

const SIDE_BY_SIDE_MIN_WIDTH: usize = 100;
const DEFAULT_DIFF_WIDTH: usize = 120;
const CHANGE_COL_WIDTH: usize = 7;
const MIN_SIDE_COLUMN_WIDTH: usize = 32;
const MAX_SIDE_COLUMN_WIDTH: usize = 70;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffRowKind {
    Equal,
    Changed,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffRow {
    left: Option<String>,
    right: Option<String>,
    kind: DiffRowKind,
}

pub fn render_file_diff_from_paths(left: &Path, right: &Path) -> Result<String, String> {
    render_file_diff_from_paths_with_width(left, right, DEFAULT_DIFF_WIDTH)
}

pub fn render_file_diff_from_paths_with_width(
    left: &Path,
    right: &Path,
    width: usize,
) -> Result<String, String> {
    let left_content = fs::read_to_string(left)
        .map_err(|err| format!("failed to read {}: {}", left.display(), err))?;
    let right_content = fs::read_to_string(right)
        .map_err(|err| format!("failed to read {}: {}", right.display(), err))?;

    Ok(render_file_diff_from_str_with_width(
        &left.to_string_lossy(),
        &right.to_string_lossy(),
        &left_content,
        &right_content,
        width,
    ))
}

pub fn render_file_diff_from_str(
    left_name: &str,
    right_name: &str,
    left: &str,
    right: &str,
) -> String {
    render_file_diff_from_str_with_width(left_name, right_name, left, right, DEFAULT_DIFF_WIDTH)
}

pub fn render_file_diff_from_str_with_width(
    left_name: &str,
    right_name: &str,
    left: &str,
    right: &str,
    width: usize,
) -> String {
    let rows = diff_rows(left, right);
    if width >= SIDE_BY_SIDE_MIN_WIDTH {
        render_side_by_side(left_name, right_name, &rows, width)
    } else {
        render_stacked(left_name, right_name, &rows)
    }
}

fn diff_rows(left: &str, right: &str) -> Vec<DiffRow> {
    let diff = TextDiff::from_lines(left, right);
    let mut rows = Vec::new();
    let mut deleted = Vec::new();
    let mut inserted = Vec::new();

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => deleted.push(strip_line_ending(change.value()).to_string()),
            ChangeTag::Insert => inserted.push(strip_line_ending(change.value()).to_string()),
            ChangeTag::Equal => {
                flush_changed_rows(&mut rows, &mut deleted, &mut inserted);
                let value = strip_line_ending(change.value()).to_string();
                rows.push(DiffRow {
                    left: Some(value.clone()),
                    right: Some(value),
                    kind: DiffRowKind::Equal,
                });
            }
        }
    }

    flush_changed_rows(&mut rows, &mut deleted, &mut inserted);
    rows
}

fn flush_changed_rows(
    rows: &mut Vec<DiffRow>,
    deleted: &mut Vec<String>,
    inserted: &mut Vec<String>,
) {
    let row_count = deleted.len().max(inserted.len());
    for idx in 0..row_count {
        let left = deleted.get(idx).cloned();
        let right = inserted.get(idx).cloned();
        let kind = match (left.is_some(), right.is_some()) {
            (true, true) => DiffRowKind::Changed,
            (true, false) => DiffRowKind::Removed,
            (false, true) => DiffRowKind::Added,
            (false, false) => DiffRowKind::Equal,
        };
        rows.push(DiffRow { left, right, kind });
    }
    deleted.clear();
    inserted.clear();
}

fn render_side_by_side(
    left_name: &str,
    right_name: &str,
    rows: &[DiffRow],
    width: usize,
) -> String {
    let mut output = render_header(left_name, right_name);
    if rows.iter().all(|row| row.kind == DiffRowKind::Equal) {
        output.push_str("No differences found.\n");
        return output;
    }

    let column_width = side_column_width(width);
    output.push_str(&format!(
        "{} | {} | {}\n",
        pad_cell("Before", column_width),
        pad_cell("Change", CHANGE_COL_WIDTH),
        pad_cell("After", column_width)
    ));
    output.push_str(&format!(
        "{}-+-{}-+-{}\n",
        "-".repeat(column_width),
        "-".repeat(CHANGE_COL_WIDTH),
        "-".repeat(column_width)
    ));

    for row in rows {
        output.push_str(&format!(
            "{} | {} | {}\n",
            pad_cell(row.left.as_deref().unwrap_or(""), column_width),
            pad_cell(change_label(row.kind), CHANGE_COL_WIDTH),
            pad_cell(row.right.as_deref().unwrap_or(""), column_width)
        ));
    }

    output
}

fn render_stacked(left_name: &str, right_name: &str, rows: &[DiffRow]) -> String {
    let mut output = render_header(left_name, right_name);
    let change_groups = changed_groups(rows);
    if change_groups.is_empty() {
        output.push_str("No differences found.\n");
        return output;
    }

    for (idx, group) in change_groups.iter().enumerate() {
        let removed = collect_left_lines(group);
        let added = collect_right_lines(group);
        let title = match (removed.is_empty(), added.is_empty()) {
            (false, false) => "changed",
            (false, true) => "removed",
            (true, false) => "added",
            (true, true) => "changed",
        };

        output.push_str(&format!("Change {}: {}\n", idx + 1, title));
        if !removed.is_empty() && !added.is_empty() {
            output.push_str("Before:\n");
            push_lines(&mut output, &removed);
            output.push_str("After:\n");
            push_lines(&mut output, &added);
        } else if !removed.is_empty() {
            output.push_str("Removed:\n");
            push_lines(&mut output, &removed);
        } else {
            output.push_str("Added:\n");
            push_lines(&mut output, &added);
        }
        output.push('\n');
    }

    output
}

fn render_header(left_name: &str, right_name: &str) -> String {
    format!(
        "Nevi Diff\nBefore: {}\nAfter: {}\n\n",
        left_name, right_name
    )
}

fn side_column_width(width: usize) -> usize {
    let separators_width = " | ".len() * 2;
    width
        .saturating_sub(separators_width + CHANGE_COL_WIDTH)
        .saturating_div(2)
        .max(MIN_SIDE_COLUMN_WIDTH)
        .min(MAX_SIDE_COLUMN_WIDTH)
}

fn pad_cell(value: &str, width: usize) -> String {
    let value = truncate_cell(value, width);
    format!("{value:<width$}")
}

fn truncate_cell(value: &str, width: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= width {
        return value.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }

    let mut truncated = value.chars().take(width - 3).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn change_label(kind: DiffRowKind) -> &'static str {
    match kind {
        DiffRowKind::Equal => "",
        DiffRowKind::Changed => "changed",
        DiffRowKind::Added => "added",
        DiffRowKind::Removed => "removed",
    }
}

fn changed_groups(rows: &[DiffRow]) -> Vec<&[DiffRow]> {
    let mut groups = Vec::new();
    let mut start = None;

    for (idx, row) in rows.iter().enumerate() {
        if row.kind == DiffRowKind::Equal {
            if let Some(start_idx) = start.take() {
                groups.push(&rows[start_idx..idx]);
            }
        } else if start.is_none() {
            start = Some(idx);
        }
    }

    if let Some(start_idx) = start {
        groups.push(&rows[start_idx..]);
    }

    groups
}

fn collect_left_lines(rows: &[DiffRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| row.left.as_ref().cloned())
        .collect()
}

fn collect_right_lines(rows: &[DiffRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| row.right.as_ref().cloned())
        .collect()
}

fn push_lines(output: &mut String, lines: &[String]) {
    for line in lines {
        output.push_str(line);
        output.push('\n');
    }
}

fn strip_line_ending(line: &str) -> &str {
    line.strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .or_else(|| line.strip_suffix('\r'))
        .unwrap_or(line)
}

#[cfg(test)]
mod tests {
    const BEFORE: &str = "fn main() {\n    let name = \"Nevi\";\n    println!(\"hello {name}\");\n}\n\nfn unchanged() {\n    println!(\"same\");\n}\n";
    const AFTER: &str = "fn main() {\n    let name = \"Nevi\";\n    println!(\"hello from {name}\");\n}\n\nfn unchanged() {\n    println!(\"same\");\n}\n\nfn added() {\n    println!(\"new function\");\n}\n";

    #[test]
    fn renders_read_only_file_diff_as_side_by_side_when_wide() {
        let rendered =
            super::render_file_diff_from_str_with_width("left.rs", "right.rs", BEFORE, AFTER, 120);

        assert!(rendered.starts_with("Nevi Diff\n"));
        assert!(!rendered.contains("```"));
        assert!(rendered.contains("Before"));
        assert!(rendered.contains("Change"));
        assert!(rendered.contains("After"));
        assert!(rendered.contains("changed"));
        assert!(rendered.contains("added"));
        assert!(rendered.contains("println!(\"hello {name}\");"));
        assert!(rendered.contains("println!(\"hello from {name}\");"));
        assert!(rendered.contains("fn added() {"));
    }

    #[test]
    fn renders_read_only_file_diff_as_stacked_blocks_when_narrow() {
        let rendered =
            super::render_file_diff_from_str_with_width("left.rs", "right.rs", BEFORE, AFTER, 80);

        assert!(rendered.starts_with("Nevi Diff\n"));
        assert!(rendered.contains("Change 1: changed"));
        assert!(rendered.contains("Before:\n    println!(\"hello {name}\");"));
        assert!(rendered.contains("After:\n    println!(\"hello from {name}\");"));
        assert!(rendered.contains("Change 2: added"));
        assert!(rendered.contains("Added:\n\nfn added() {\n    println!(\"new function\");\n}"));
    }

    #[test]
    fn caps_side_by_side_width_on_large_terminals() {
        let rendered =
            super::render_file_diff_from_str_with_width("left.rs", "right.rs", BEFORE, AFTER, 240);
        let longest_line = rendered
            .lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);

        assert!(longest_line <= 153);
    }
}
