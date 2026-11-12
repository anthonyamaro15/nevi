//! The explorer's add / rename / delete prompt: a small box drawn right
//! under the selected row (above it near the bottom), so the question shows
//! up where you were looking instead of at the foot of the sidebar.

use super::{Terminal, explorer_action_input_view};
use crate::editor::Editor;
use crate::explorer::ExplorerAction;
use crossterm::{
    cursor, queue,
    style::{Attribute, SetAttribute, SetBackgroundColor, SetForegroundColor},
};
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

/// Border, input row, border.
const BOX_ROWS: u16 = 3;

/// Where the prompt goes and what it says.
#[derive(Debug, PartialEq)]
pub(super) struct PromptBox {
    pub x: u16,
    /// Row of the top border.
    pub y: u16,
    /// Columns, borders included.
    pub width: usize,
    pub title: String,
    pub hint: &'static str,
    /// Delete: drawn in the error color, with `body` instead of an input.
    pub danger: bool,
    /// Folder the typed name is relative to, drawn dim before it.
    pub prefix: String,
    /// Delete only: what goes with it.
    pub body: Option<&'static str>,
}

impl PromptBox {
    /// Columns between "│ " and " │".
    fn inner_width(&self) -> usize {
        self.width.saturating_sub(4)
    }

    /// Columns left for the typed text after the folder prefix.
    fn input_width(&self) -> usize {
        self.inner_width().saturating_sub(self.prefix.width())
    }
}

pub(super) fn prompt_box(editor: &Editor) -> Option<PromptBox> {
    let explorer = &editor.explorer;
    let action = explorer.pending_action.as_ref()?;
    let node = explorer.selected_node()?;

    // Rows as `render_explorer` lays them out: header, then the list.
    let top = editor.settings.editor.bufferline as u16;
    let height = editor.text_rows() as u16;
    let list_height = height.saturating_sub(1);
    let first_row = top + 1;
    let last_row = top + height.saturating_sub(1);
    let offset = explorer.scroll_offset(list_height as usize);
    let selected_row = first_row + explorer.selected.saturating_sub(offset) as u16;
    let y = if selected_row + BOX_ROWS <= last_row {
        selected_row + 1
    } else if selected_row >= first_row + BOX_ROWS {
        selected_row - BOX_ROWS
    } else {
        last_row.saturating_sub(BOX_ROWS - 1)
    };

    // Indented under the row, one column short of the sidebar's edge.
    let sidebar = explorer.width as usize;
    let x: u16 = if sidebar >= 24 { 2 } else { 0 };
    let width = sidebar.saturating_sub(x as usize + 1);

    // Add and rename resolve the typed name against this folder.
    let folder = match action {
        ExplorerAction::Add if node.is_dir => Some(node.path.as_path()),
        _ => node.path.parent(),
    };
    let relative = folder
        .zip(explorer.root.as_deref())
        .and_then(|(folder, root)| folder.strip_prefix(root).ok())
        .map(|rel| rel.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut prefix = if relative.is_empty() {
        String::new()
    } else {
        format!("{relative}/")
    };
    // A deep folder keeps its end, and never more than half the field.
    let budget = width.saturating_sub(4) / 2;
    if prefix.width() > budget {
        prefix = crate::finder::row::cut_left(&prefix, budget);
    }

    let (title, hint, danger, body) = match action {
        ExplorerAction::Add => (
            "New file".to_string(),
            "end with / for a folder",
            false,
            None,
        ),
        ExplorerAction::Rename => ("Rename".to_string(), "Enter to rename", false, None),
        ExplorerAction::Delete => {
            let dirty = editor.buffers().iter().any(|buffer| {
                buffer.dirty
                    && buffer
                        .path
                        .as_ref()
                        .is_some_and(|path| path.starts_with(&node.path))
            });
            let body = match (node.is_dir, dirty) {
                (true, true) => "folder, files with unsaved changes",
                (true, false) => "the folder and all inside it",
                (false, true) => "has unsaved changes",
                (false, false) => "can't be undone",
            };
            prefix.clear();
            (
                format!("Delete {}?", node.name),
                "y delete · n keep",
                true,
                Some(body),
            )
        }
    };

    Some(PromptBox {
        x,
        y,
        width,
        title,
        hint,
        danger,
        prefix,
        body,
    })
}

/// Screen cell of the input cursor, or `None` when the prompt takes no text.
pub(super) fn prompt_cursor(editor: &Editor) -> Option<(u16, u16)> {
    let prompt = prompt_box(editor)?;
    if prompt.danger || prompt.width < 5 {
        return None;
    }
    let (_, cursor_col) = explorer_action_input_view(
        &editor.explorer.input_buffer,
        editor.explorer.input_cursor,
        prompt.input_width(),
    );
    let x = prompt.x as usize + 2 + prompt.prefix.width() + cursor_col;
    Some((x as u16, prompt.y + 1))
}

/// `text` cut to `width` columns, with "…" when it doesn't fit.
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        if out.width() + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) + 1 > width {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

impl Terminal {
    pub(super) fn render_explorer_prompt(&mut self, editor: &Editor) -> io::Result<()> {
        let Some(prompt) = prompt_box(editor) else {
            return Ok(());
        };
        if prompt.width < 5 {
            return Ok(());
        }
        let theme = editor.theme();
        let glyphs = editor.ui_glyphs();
        let bg = theme.ui.popup_bg;
        let fg = theme.ui.foreground;
        let quiet = theme.ui.line_number;
        let border = if prompt.danger {
            theme.diagnostic.error
        } else {
            theme.ui.popup_border
        };
        let inner = prompt.inner_width();
        // "─ label ─────" across the box, between two corners.
        let edge = |left: &str, label: &str, right: &str| {
            let label = fit(label, inner.saturating_sub(1));
            let fill = (prompt.width - 2).saturating_sub(label.width() + 3);
            (
                format!("{left}─ "),
                label,
                format!(" {}{right}", "─".repeat(fill)),
            )
        };

        // Top border with the title.
        let (start, title, end) = edge(glyphs.corner_tl, &prompt.title, glyphs.corner_tr);
        let title_color = if prompt.danger { border } else { fg };
        queue!(
            self.stdout,
            cursor::MoveTo(prompt.x, prompt.y),
            SetBackgroundColor(bg),
            SetForegroundColor(border)
        )?;
        write!(self.stdout, "{start}")?;
        queue!(
            self.stdout,
            SetForegroundColor(title_color),
            SetAttribute(Attribute::Bold)
        )?;
        write!(self.stdout, "{title}")?;
        queue!(
            self.stdout,
            SetAttribute(Attribute::NormalIntensity),
            SetForegroundColor(border)
        )?;
        write!(self.stdout, "{end}")?;

        // Input row: folder prefix (quiet), then the typed name; or the
        // delete warning.
        queue!(self.stdout, cursor::MoveTo(prompt.x, prompt.y + 1))?;
        write!(self.stdout, "│ ")?;
        let used = if let Some(body) = prompt.body {
            let body = fit(body, inner);
            queue!(self.stdout, SetForegroundColor(fg))?;
            write!(self.stdout, "{body}")?;
            body.width()
        } else {
            let (visible, _) = explorer_action_input_view(
                &editor.explorer.input_buffer,
                editor.explorer.input_cursor,
                prompt.input_width(),
            );
            queue!(self.stdout, SetForegroundColor(quiet))?;
            write!(self.stdout, "{}", prompt.prefix)?;
            queue!(self.stdout, SetForegroundColor(fg))?;
            write!(self.stdout, "{visible}")?;
            prompt.prefix.width() + visible.width()
        };
        queue!(self.stdout, SetForegroundColor(border))?;
        write!(self.stdout, "{} │", " ".repeat(inner.saturating_sub(used)))?;

        // Bottom border with the keys.
        let (start, hint, end) = edge(glyphs.corner_bl, prompt.hint, glyphs.corner_br);
        queue!(self.stdout, cursor::MoveTo(prompt.x, prompt.y + 2))?;
        write!(self.stdout, "{start}")?;
        queue!(self.stdout, SetForegroundColor(quiet))?;
        write!(self.stdout, "{hint}")?;
        queue!(self.stdout, SetForegroundColor(border))?;
        write!(self.stdout, "{end}")?;
        Ok(())
    }
}
