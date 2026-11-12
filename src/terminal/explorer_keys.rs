//! The explorer's `?` key card: the everyday explorer keys on one screen.
//! Keys are read from the keymap, so a remapped key shows as it is bound.

use super::Terminal;
use super::explorer_prompt::edge;
use crate::config::ExplorerModeMapping;
use crate::editor::Editor;
use crossterm::{
    cursor, queue,
    style::{SetBackgroundColor, SetForegroundColor},
};
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

/// Action and the word shown for it, in reading order.
const CARD: [(&str, &str); 16] = [
    ("create", "new"),
    ("rename", "rename"),
    ("delete", "delete"),
    ("copy", "copy"),
    ("cut", "cut"),
    ("paste", "paste"),
    ("search", "search"),
    ("refresh", "refresh"),
    ("expand_or_open", "open"),
    ("collapse_or_parent", "fold"),
    ("go_to_parent", "parent"),
    ("collapse_all", "fold all"),
    ("widen_sidebar", "wider"),
    ("narrow_sidebar", "narrower"),
    ("reset_sidebar_width", "reset"),
    ("close", "close"),
];

const MAX_COLUMNS: usize = 4;
const GAP: usize = 3;
const TITLE: &str = "Explorer keys";
const FOOTER: &str = ":Keymaps for everything";

/// One `key word` pair per card action, using the first plain key bound to
/// it (`q` over `<Esc>`). An action with no key is left out.
pub(super) fn key_cells(mappings: &[ExplorerModeMapping]) -> Vec<(String, &'static str)> {
    CARD.iter()
        .filter_map(|(action, word)| {
            let bound = || {
                mappings
                    .iter()
                    .filter(|m| m.action.trim().eq_ignore_ascii_case(action))
            };
            let mapping = bound()
                .find(|m| !m.key.starts_with('<'))
                .or_else(|| bound().next())?;
            Some((mapping.key.clone(), *word))
        })
        .collect()
}

/// The card's place on screen, centered over the editor.
#[derive(Debug, PartialEq)]
pub(super) struct KeyCard {
    pub x: u16,
    pub y: u16,
    /// Columns, borders included.
    pub width: usize,
    pub columns: usize,
    pub cell_width: usize,
    pub cells: Vec<(String, &'static str)>,
}

pub(super) fn key_card(editor: &Editor) -> Option<KeyCard> {
    let cells = key_cells(&editor.settings.keymap.explorer);
    let cell_width = cells
        .iter()
        .map(|(key, word)| key.width() + 1 + word.width())
        .max()?;
    let term_width = editor.term_width as usize;
    let card_width = |columns: usize| columns * (cell_width + GAP) - GAP + 4;
    let columns = (1..=MAX_COLUMNS.min(cells.len()))
        .rev()
        .find(|&columns| card_width(columns) <= term_width)?;
    let rows = cells.len().div_ceil(columns);
    let height = rows + 2;
    let text_rows = editor.text_rows();
    if height > text_rows {
        return None;
    }
    let width = card_width(columns);
    let top = editor.settings.editor.bufferline as usize;
    Some(KeyCard {
        x: ((term_width - width) / 2) as u16,
        y: (top + (text_rows - height) / 2) as u16,
        width,
        columns,
        cell_width,
        cells,
    })
}

impl Terminal {
    pub(super) fn render_explorer_keys(&mut self, editor: &Editor) -> io::Result<()> {
        let Some(card) = key_card(editor) else {
            return Ok(());
        };
        let theme = editor.theme();
        let glyphs = editor.ui_glyphs();
        let border = theme.ui.popup_border;
        let fg = theme.ui.foreground;
        let quiet = theme.ui.line_number;
        let key_color = theme.ui.explorer_directory;

        let (start, title, end) = edge(card.width, glyphs.corner_tl, TITLE, glyphs.corner_tr);
        queue!(
            self.stdout,
            cursor::MoveTo(card.x, card.y),
            SetBackgroundColor(theme.ui.popup_bg),
            SetForegroundColor(border)
        )?;
        write!(self.stdout, "{start}")?;
        queue!(self.stdout, SetForegroundColor(fg))?;
        write!(self.stdout, "{title}")?;
        queue!(self.stdout, SetForegroundColor(border))?;
        write!(self.stdout, "{end}")?;

        let inner = card.width - 4;
        let mut y = card.y;
        for row in card.cells.chunks(card.columns) {
            y += 1;
            queue!(
                self.stdout,
                cursor::MoveTo(card.x, y),
                SetForegroundColor(border)
            )?;
            write!(self.stdout, "│ ")?;
            let mut used = 0;
            for (i, (key, word)) in row.iter().enumerate() {
                let start = i * (card.cell_width + GAP);
                write!(self.stdout, "{:1$}", "", start - used)?;
                queue!(self.stdout, SetForegroundColor(key_color))?;
                write!(self.stdout, "{key} ")?;
                queue!(self.stdout, SetForegroundColor(fg))?;
                write!(self.stdout, "{word}")?;
                used = start + key.width() + 1 + word.width();
            }
            queue!(self.stdout, SetForegroundColor(border))?;
            write!(self.stdout, "{:1$} │", "", inner - used)?;
        }

        let (start, footer, end) = edge(card.width, glyphs.corner_bl, FOOTER, glyphs.corner_br);
        queue!(self.stdout, cursor::MoveTo(card.x, y + 1))?;
        write!(self.stdout, "{start}")?;
        queue!(self.stdout, SetForegroundColor(quiet))?;
        write!(self.stdout, "{footer}")?;
        queue!(self.stdout, SetForegroundColor(border))?;
        write!(self.stdout, "{end}")?;
        Ok(())
    }
}
