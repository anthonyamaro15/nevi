//! The `K` hover popup: a framed float with the symbol as its title, the
//! docs wrapped to the box, and a page counter when they run longer.

use super::Terminal;
use super::explorer_prompt::edge;
use crate::editor::Editor;
use crate::hover::{CodeSpan, HoverRow};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::{
    cursor, queue,
    style::{Color, SetBackgroundColor, SetForegroundColor},
};
use std::io::Write;

/// Tallest body before the hover pages.
const MAX_BODY_ROWS: usize = 16;

#[derive(Debug, PartialEq)]
pub(super) struct HoverBox {
    pub x: u16,
    pub y: u16,
    /// Columns, borders included.
    pub width: usize,
    pub body_rows: usize,
}

/// Above the cursor when it fits, else below, else on the roomier side
/// with fewer rows (and more pages).
pub(super) fn hover_box(editor: &Editor) -> Option<HoverBox> {
    let hover = editor.hover.as_ref()?;
    let width = hover.width + 4;
    let wanted = hover.rows.len().clamp(1, MAX_BODY_ROWS) + 2;
    let (col, row) = Terminal::cursor_screen_cell(editor);
    let row = row as usize;
    let above = row;
    // Keep the command line clear.
    let below = (editor.term_height as usize).saturating_sub(row + 2);
    let (y, height) = if wanted <= above {
        (row - wanted, wanted)
    } else if wanted <= below {
        (row + 1, wanted)
    } else if above >= below {
        (0, above)
    } else {
        (row + 1, below)
    };
    if height < 3 {
        return None;
    }
    let x = (col as usize)
        .saturating_sub(2)
        .min((editor.term_width as usize).saturating_sub(width + 1));
    Some(HoverBox {
        x: x as u16,
        y: y as u16,
        width,
        body_rows: height - 2,
    })
}

/// Ctrl-d / Ctrl-u turn the page of a hover that doesn't fit. Returns
/// false for every other key (and for a hover that fits), which closes it.
pub fn scroll_hover(editor: &mut Editor, key: KeyEvent) -> bool {
    if key.modifiers != KeyModifiers::CONTROL {
        return false;
    }
    let Some(body) = hover_box(editor).map(|b| b.body_rows) else {
        return false;
    };
    let Some(hover) = editor.hover.as_mut() else {
        return false;
    };
    if hover.rows.len() <= body {
        return false;
    }
    match key.code {
        KeyCode::Char('d') if hover.scroll + body < hover.rows.len() => hover.scroll += body,
        KeyCode::Char('u') => hover.scroll = hover.scroll.saturating_sub(body),
        KeyCode::Char('d') => {}
        _ => return false,
    }
    editor.render_damage.mark_full();
    true
}

impl Terminal {
    pub(super) fn render_hover(&mut self, editor: &Editor) -> anyhow::Result<()> {
        let (Some(hover), Some(b)) = (editor.hover.as_ref(), hover_box(editor)) else {
            return Ok(());
        };
        let theme = editor.theme();
        let glyphs = editor.ui_glyphs();
        let bg = theme.ui.popup_bg;
        let border = theme.ui.popup_border;
        let fg = theme.ui.foreground;
        let inner = b.width - 4;

        queue!(
            self.stdout,
            cursor::MoveTo(b.x, b.y),
            SetBackgroundColor(bg),
            SetForegroundColor(border)
        )?;
        let title: String = hover.title.iter().map(|s| s.text.as_str()).collect();
        if title.is_empty() {
            write!(
                self.stdout,
                "{}{}{}",
                glyphs.corner_tl,
                "─".repeat(b.width - 2),
                glyphs.corner_tr
            )?;
        } else {
            let (start, label, end) = edge(b.width, glyphs.corner_tl, &title, glyphs.corner_tr);
            write!(self.stdout, "{start}")?;
            if label == title {
                self.write_code_spans(&hover.title, fg)?;
            } else {
                queue!(self.stdout, SetForegroundColor(fg))?;
                write!(self.stdout, "{label}")?;
            }
            queue!(self.stdout, SetForegroundColor(border))?;
            write!(self.stdout, "{end}")?;
        }

        for i in 0..b.body_rows {
            queue!(
                self.stdout,
                cursor::MoveTo(b.x, b.y + 1 + i as u16),
                SetBackgroundColor(bg),
                SetForegroundColor(border)
            )?;
            let row = hover.rows.get(hover.scroll + i);
            if row == Some(&HoverRow::Rule) {
                write!(self.stdout, "├{}┤", "─".repeat(b.width - 2))?;
                continue;
            }
            write!(self.stdout, "│ ")?;
            match row {
                Some(HoverRow::Text(line)) => {
                    Self::render_markdown_preview_line(&mut self.stdout, line, inner, editor)?;
                }
                Some(HoverRow::Code(spans)) => {
                    self.write_code_spans(spans, fg)?;
                    let used = crate::hover::spans_width(spans);
                    write!(self.stdout, "{:1$}", "", inner.saturating_sub(used))?;
                }
                _ => write!(self.stdout, "{:inner$}", "")?,
            }
            queue!(
                self.stdout,
                SetBackgroundColor(bg),
                SetForegroundColor(border)
            )?;
            write!(self.stdout, " │")?;
        }

        queue!(
            self.stdout,
            cursor::MoveTo(b.x, b.y + 1 + b.body_rows as u16),
            SetForegroundColor(border)
        )?;
        let pages = hover.rows.len().div_ceil(b.body_rows);
        if pages > 1 {
            let page = hover.scroll / b.body_rows + 1;
            let hint = if page < pages {
                "Ctrl-d more"
            } else {
                "Ctrl-u back"
            };
            let (start, label, end) = edge(
                b.width,
                glyphs.corner_bl,
                &format!("{page}/{pages} · {hint}"),
                glyphs.corner_br,
            );
            write!(self.stdout, "{start}")?;
            queue!(self.stdout, SetForegroundColor(theme.ui.line_number))?;
            write!(self.stdout, "{label}")?;
            queue!(self.stdout, SetForegroundColor(border))?;
            write!(self.stdout, "{end}")?;
        } else {
            write!(
                self.stdout,
                "{}{}{}",
                glyphs.corner_bl,
                "─".repeat(b.width - 2),
                glyphs.corner_br
            )?;
        }
        Ok(())
    }

    fn write_code_spans(&mut self, spans: &[CodeSpan], fallback: Color) -> anyhow::Result<()> {
        for span in spans {
            let color = span.style.map_or(fallback, |style| style.fg);
            queue!(self.stdout, SetForegroundColor(color))?;
            write!(self.stdout, "{}", span.text)?;
        }
        Ok(())
    }
}
