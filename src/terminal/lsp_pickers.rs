//! The references (`gr`) and code actions pickers. References show each
//! hit's code with the symbol marked, laid out like a live grep row; code
//! actions are numbered so 1-9 picks one, with a quiet tag for the kind.

use super::Terminal;
use super::explorer_prompt::{edge, fit};
use crate::editor::Editor;
use crate::finder::row::{GrepCell, cell_width, grep_row};
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, SetAttribute, SetBackgroundColor, SetForegroundColor},
};
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

const MAX_WIDTH: usize = 80;
const REFERENCE_ROWS: usize = 13;
const ACTION_ROWS: usize = 10;

/// A few letters for an LSP code action kind, such as "fix" for
/// `quickfix` or "extract" for `refactor.extract.function`.
pub(super) fn kind_tag(kind: Option<&str>) -> String {
    let Some(kind) = kind.filter(|kind| !kind.is_empty()) else {
        return String::new();
    };
    let mut parts = kind.split('.');
    let first = parts.next().unwrap_or_default();
    let tag = match (first, parts.next()) {
        ("quickfix", _) => "fix",
        ("refactor", Some(sub @ ("extract" | "inline" | "rewrite" | "move"))) => sub,
        // `source.doc`, and gopls's `gopls.doc.features`.
        (_, Some("doc")) => "docs",
        ("refactor", _) => "refactor",
        ("source", Some("organizeImports")) => "imports",
        ("source", Some("fixAll")) => "fix all",
        ("source", Some("addTest")) => "test",
        ("source", Some("assembly")) => "asm",
        ("source", _) => "source",
        (other, _) => other,
    };
    tag.to_string()
}

/// First row on screen so `selected` stays visible.
fn scroll(selected: usize, rows: usize) -> usize {
    (selected + 1).saturating_sub(rows)
}

impl Terminal {
    fn write_edge(
        &mut self,
        (start, label, end): (String, String, String),
        border: Color,
        label_color: Color,
    ) -> io::Result<()> {
        queue!(self.stdout, SetForegroundColor(border))?;
        write!(self.stdout, "{start}")?;
        queue!(self.stdout, SetForegroundColor(label_color))?;
        write!(self.stdout, "{label}")?;
        queue!(self.stdout, SetForegroundColor(border))?;
        write!(self.stdout, "{end}")
    }

    pub(super) fn render_references_picker(&mut self, editor: &Editor) -> io::Result<()> {
        let Some(picker) = &editor.references_picker else {
            return Ok(());
        };
        let width = MAX_WIDTH.min((editor.term_width as usize).saturating_sub(4));
        if picker.rows.is_empty() || width < 12 {
            return Ok(());
        }
        let rows = picker.rows.len().min(REFERENCE_ROWS);
        let x = ((editor.term_width as usize - width) / 2) as u16;
        let y = ((editor.term_height as usize).saturating_sub(rows + 2) / 2) as u16;
        let theme = editor.theme();
        let glyphs = editor.ui_glyphs();
        let bg = theme.ui.popup_bg;
        let border = theme.ui.popup_border;
        let quiet = theme.ui.line_number;

        let symbol = picker.symbol();
        let title = if symbol.is_empty() {
            format!("References · {}", picker.rows.len())
        } else {
            format!("References · {symbol} · {}", picker.rows.len())
        };
        queue!(self.stdout, cursor::MoveTo(x, y), SetBackgroundColor(bg))?;
        let top = edge(width, glyphs.corner_tl, &title, glyphs.corner_tr);
        self.write_edge(top, border, theme.ui.foreground)?;

        // "│", the selection bar (or a space), the row, a space, "│".
        let text_width = width - 4;
        let first = scroll(picker.selected, rows);
        for (i, row) in picker.rows.iter().enumerate().skip(first).take(rows) {
            let selected = i == picker.selected;
            let row_bg = if selected {
                theme.ui.popup_selection
            } else {
                bg
            };
            queue!(
                self.stdout,
                cursor::MoveTo(x, y + 1 + (i - first) as u16),
                SetBackgroundColor(bg),
                SetForegroundColor(border)
            )?;
            write!(self.stdout, "│")?;
            queue!(
                self.stdout,
                SetBackgroundColor(row_bg),
                SetForegroundColor(theme.ui.finder_match)
            )?;
            let bar = if selected && !glyphs.finder_selection_bar.is_empty() {
                glyphs.finder_selection_bar
            } else {
                " "
            };
            write!(self.stdout, "{bar}")?;

            let cells = grep_row(&row.path, row.line, &row.text, &row.matched, text_width);
            let mut used = 0;
            let mut current = None;
            for (ch, kind) in cells {
                let color = match kind {
                    GrepCell::Location => quiet,
                    GrepCell::Text => theme.ui.foreground,
                    GrepCell::Match => theme.ui.finder_match,
                };
                if current != Some(color) {
                    queue!(self.stdout, SetForegroundColor(color))?;
                    current = Some(color);
                }
                let ch = if ch.is_control() { ' ' } else { ch };
                write!(self.stdout, "{ch}")?;
                used += cell_width(ch);
            }
            write!(
                self.stdout,
                "{:1$}",
                "",
                text_width.saturating_sub(used) + 1
            )?;
            queue!(
                self.stdout,
                SetBackgroundColor(bg),
                SetForegroundColor(border)
            )?;
            write!(self.stdout, "│")?;
        }

        queue!(self.stdout, cursor::MoveTo(x, y + 1 + rows as u16))?;
        let bottom = edge(
            width,
            glyphs.corner_bl,
            "Enter go · Esc close",
            glyphs.corner_br,
        );
        self.write_edge(bottom, border, quiet)
    }

    pub(super) fn render_code_actions_picker(&mut self, editor: &Editor) -> io::Result<()> {
        let Some(picker) = &editor.code_actions_picker else {
            return Ok(());
        };
        if picker.items.is_empty() {
            return Ok(());
        }
        let title = format!("Code actions · {}", picker.items.len());
        let footer = "1-9 pick · Enter apply";
        let tags: Vec<String> = picker
            .items
            .iter()
            .map(|action| kind_tag(action.kind.as_deref()))
            .collect();
        let tag_width = tags.iter().map(|tag| tag.width()).max().unwrap_or(0);
        let tag_room = if tag_width > 0 { tag_width + 2 } else { 0 };
        let title_width = picker
            .items
            .iter()
            .map(|action| action.title.width())
            .max()
            .unwrap_or(0);
        // "│ 1  ", the title, "  tag", " │".
        let width = (title_width + tag_room + 7)
            .max(title.width() + 6)
            .max(footer.width() + 6)
            .min(MAX_WIDTH)
            .min((editor.term_width as usize).saturating_sub(4));
        if width < 12 {
            return Ok(());
        }
        let rows = picker.items.len().min(ACTION_ROWS);
        let height = rows + 2;
        let (col, row) = Self::cursor_screen_cell(editor);
        let x = (col as usize).min((editor.term_width as usize).saturating_sub(width + 2)) as u16;
        let y = if row as usize + height < editor.term_height as usize {
            row + 1
        } else {
            row.saturating_sub(height as u16)
        };
        let theme = editor.theme();
        let glyphs = editor.ui_glyphs();
        let bg = theme.ui.popup_bg;
        let border = theme.ui.popup_border;
        let quiet = theme.ui.line_number;

        queue!(self.stdout, cursor::MoveTo(x, y), SetBackgroundColor(bg))?;
        let top = edge(width, glyphs.corner_tl, &title, glyphs.corner_tr);
        self.write_edge(top, border, theme.ui.foreground)?;

        let title_room = width - 7 - tag_room;
        let first = scroll(picker.selected, rows);
        for (i, action) in picker.items.iter().enumerate().skip(first).take(rows) {
            let row_bg = if i == picker.selected {
                theme.ui.popup_selection
            } else {
                bg
            };
            queue!(
                self.stdout,
                cursor::MoveTo(x, y + 1 + (i - first) as u16),
                SetBackgroundColor(bg),
                SetForegroundColor(border)
            )?;
            write!(self.stdout, "│")?;
            queue!(
                self.stdout,
                SetBackgroundColor(row_bg),
                SetForegroundColor(quiet)
            )?;
            let number = if i < 9 {
                char::from(b'1' + i as u8)
            } else {
                ' '
            };
            write!(self.stdout, " {number}  ")?;

            let text = fit(&action.title, title_room);
            queue!(self.stdout, SetForegroundColor(theme.ui.foreground))?;
            if action.is_preferred {
                queue!(self.stdout, SetAttribute(Attribute::Bold))?;
            }
            write!(self.stdout, "{text}")?;
            if action.is_preferred {
                queue!(self.stdout, SetAttribute(Attribute::NormalIntensity))?;
            }
            write!(self.stdout, "{:1$}", "", title_room - text.width())?;
            if tag_room > 0 {
                queue!(self.stdout, SetForegroundColor(quiet))?;
                write!(self.stdout, "  {:>1$}", tags[i], tag_width)?;
            }
            write!(self.stdout, " ")?;
            queue!(
                self.stdout,
                SetBackgroundColor(bg),
                SetForegroundColor(border)
            )?;
            write!(self.stdout, "│")?;
        }

        queue!(self.stdout, cursor::MoveTo(x, y + 1 + rows as u16))?;
        let bottom = edge(width, glyphs.corner_bl, footer, glyphs.corner_br);
        self.write_edge(bottom, border, quiet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_tags_are_short_and_quiet_about_unknown_kinds() {
        assert_eq!(kind_tag(Some("quickfix")), "fix");
        assert_eq!(kind_tag(Some("refactor.extract.function")), "extract");
        assert_eq!(kind_tag(Some("refactor.rewrite.fillStruct")), "rewrite");
        assert_eq!(kind_tag(Some("refactor")), "refactor");
        assert_eq!(kind_tag(Some("refactor.other")), "refactor");
        assert_eq!(kind_tag(Some("source.organizeImports")), "imports");
        assert_eq!(kind_tag(Some("source.addTest")), "test");
        assert_eq!(kind_tag(Some("source.doc")), "docs");
        assert_eq!(kind_tag(Some("gopls.doc.features")), "docs");
        assert_eq!(kind_tag(Some("source.freesymbols")), "source");
        assert_eq!(kind_tag(Some("notebook")), "notebook");
        assert_eq!(kind_tag(Some("")), "");
        assert_eq!(kind_tag(None), "");
    }

    #[test]
    fn the_selected_row_stays_on_screen() {
        assert_eq!(scroll(0, 10), 0);
        assert_eq!(scroll(9, 10), 0);
        assert_eq!(scroll(10, 10), 1);
    }
}
