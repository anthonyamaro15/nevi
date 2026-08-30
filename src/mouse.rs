//! Mouse event routing: hit-testing, wheel scrolling, click positioning.
//!
//! Terminal mouse capture is enabled by default (see
//! `docs/adr/0001-mouse-capture-on-by-default.md`); events arrive here from
//! the main loop's `EditorEvent::Mouse` arm whenever the floating terminal
//! isn't consuming them.

use crate::editor::{Editor, Mode};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

/// One wheel tick scrolls 3 lines, matching nvim's `mousescroll=ver:3` and
/// the floating terminal's MOUSE_SCROLL_LINES.
const WHEEL_LINES: isize = 3;
/// Horizontal ticks move 6 columns, matching nvim's `mousescroll=hor:6`.
const WHEEL_COLS: isize = 6;

/// Pane under the screen cell (col, row), if any. Rects are kept current by
/// `update_pane_rects`; the bottom two rows (statusline + command line) and
/// the explorer sidebar are outside every pane rect by construction.
pub fn pane_at(editor: &Editor, col: u16, row: u16) -> Option<usize> {
    editor
        .panes()
        .iter()
        .position(|pane| pane.rect.contains(col, row))
}

/// Route one mouse event to editor state. Returns true when the event was
/// handled and the frame must repaint. Wheel events scroll the pane under
/// the pointer (nvim behavior), not the focused pane.
pub fn handle_mouse_event(editor: &mut Editor, event: MouseEvent) -> bool {
    let handled = route_event(editor, event);
    if handled {
        // Mouse events bypass handle_key, which is where damage is normally
        // marked full; without this a partial-damage plan captured by an
        // earlier key could render a stale frame.
        editor.render_damage.mark_full();
    }
    handled
}

fn route_event(editor: &mut Editor, event: MouseEvent) -> bool {
    // Overlay gates mirror handle_key's interception order (the floating
    // terminal is consumed earlier, in the main loop).
    if editor.markdown_preview.is_some() {
        let delta = match event.kind {
            MouseEventKind::ScrollDown => WHEEL_LINES,
            MouseEventKind::ScrollUp => -WHEEL_LINES,
            _ => return false,
        };
        let visible_rows = crate::terminal::Terminal::markdown_preview_visible_rows(editor).max(1);
        editor.scroll_markdown_preview(delta, visible_rows);
        return true;
    }
    if editor.references_picker.is_some()
        || editor.code_actions_picker.is_some()
        || editor.theme_picker.is_some()
    {
        // Small selection pickers stay keyboard-driven.
        return false;
    }
    if editor.mode == Mode::Finder {
        // The results list follows the selection; the wheel drives the
        // preview pane regardless of pointer position.
        match event.kind {
            MouseEventKind::ScrollDown => editor.finder.scroll_preview_down(WHEEL_LINES as usize),
            MouseEventKind::ScrollUp => editor.finder.scroll_preview_up(WHEEL_LINES as usize),
            _ => return false,
        }
        return true;
    }
    if editor.explorer.visible && event.column <= editor.explorer.width {
        // A selection list, not a viewport: one row per tick.
        match event.kind {
            MouseEventKind::ScrollDown => editor.explorer.move_down(),
            MouseEventKind::ScrollUp => editor.explorer.move_up(),
            MouseEventKind::Down(MouseButton::Left) => {
                let list_height = editor.text_rows().saturating_sub(1);
                return editor
                    .explorer
                    .select_visible_row(event.row as usize, list_height);
            }
            _ => return false,
        }
        return true;
    }

    match event.kind {
        MouseEventKind::ScrollDown => wheel_vertical(editor, event.column, event.row, WHEEL_LINES),
        MouseEventKind::ScrollUp => wheel_vertical(editor, event.column, event.row, -WHEEL_LINES),
        MouseEventKind::ScrollRight => {
            wheel_horizontal(editor, event.column, event.row, WHEEL_COLS)
        }
        MouseEventKind::ScrollLeft => {
            wheel_horizontal(editor, event.column, event.row, -WHEEL_COLS)
        }
        MouseEventKind::Down(MouseButton::Left) => match pane_at(editor, event.column, event.row) {
            Some(idx) => {
                editor.click_at(idx, event.column, event.row);
                true
            }
            None => false,
        },
        // Motion floods and unhandled buttons fall through at match-arm cost.
        _ => false,
    }
}

fn wheel_vertical(editor: &mut Editor, col: u16, row: u16, delta: isize) -> bool {
    match pane_at(editor, col, row) {
        Some(idx) => {
            editor.scroll_pane_viewport(idx, delta);
            true
        }
        None => false,
    }
}

fn wheel_horizontal(editor: &mut Editor, col: u16, row: u16, delta: isize) -> bool {
    match pane_at(editor, col, row) {
        Some(idx) => {
            editor.scroll_pane_columns(idx, delta);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    fn single_pane_editor(width: u16, height: u16) -> Editor {
        let mut editor = Editor::default();
        editor.replace_buffer_content("alpha\nbeta\ngamma\n");
        editor.set_size(width, height);
        editor.update_pane_rects();
        editor
    }

    fn editor_with_lines(count: usize) -> Editor {
        let mut editor = Editor::default();
        let content: String = (1..=count).map(|i| format!("line{i}\n")).collect();
        editor.replace_buffer_content(&content);
        editor.set_size(120, 40);
        editor.update_pane_rects();
        editor
    }

    fn test_editor_with_vsplit() -> Editor {
        let mut editor = editor_with_lines(100);
        editor.vsplit(None).expect("vsplit");
        editor.update_pane_rects();
        editor
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::empty(),
        }
    }

    fn center_of_pane(editor: &Editor, idx: usize) -> (u16, u16) {
        let rect = editor.panes()[idx].rect;
        (rect.x + rect.width / 2, rect.y + rect.height / 2)
    }

    #[test]
    fn wheel_flood_accumulates_into_state() {
        // Perf gate: a trackpad flick is dozens of ticks; each one must be a
        // pure state mutation (renders stay per-frame in the main loop).
        let mut editor = editor_with_lines(200);
        let (col, row) = center_of_pane(&editor, 0);
        for _ in 0..32 {
            assert!(handle_mouse_event(
                &mut editor,
                mouse(MouseEventKind::ScrollDown, col, row)
            ));
        }
        assert_eq!(editor.viewport_offset, 96);
        assert!(editor.render_damage.requires_full_render());
    }

    #[test]
    fn wheel_scrolls_pane_under_pointer_not_active() {
        let mut editor = test_editor_with_vsplit();
        let inactive = 1 - editor.active_pane_idx();
        let (col, row) = center_of_pane(&editor, inactive);

        handle_mouse_event(&mut editor, mouse(MouseEventKind::ScrollDown, col, row));

        assert_eq!(editor.panes()[inactive].viewport_offset, 3);
        assert_eq!(editor.viewport_offset, 0, "focused pane untouched");
    }

    #[test]
    fn wheel_outside_any_pane_is_ignored() {
        let mut editor = single_pane_editor(120, 40);
        assert!(!handle_mouse_event(
            &mut editor,
            mouse(MouseEventKind::ScrollDown, 10, 39)
        ));
        assert_eq!(editor.viewport_offset, 0);
    }

    fn gutter_width(editor: &Editor, idx: usize) -> u16 {
        // These tests use buffers of at most 100 lines, so the gutter is
        // 2 (sign column) + 3 (line numbers) + 1 (separator) = 6 cells.
        editor.panes()[idx].rect.x + 6
    }

    #[test]
    fn click_positions_cursor_accounting_for_gutter_and_offset() {
        let mut editor = editor_with_lines(100);
        editor.scroll_pane_viewport(0, 10);
        let gutter = gutter_width(&editor, 0);

        handle_mouse_event(
            &mut editor,
            mouse(MouseEventKind::Down(MouseButton::Left), gutter + 4, 2),
        );

        assert_eq!(editor.cursor.line, 12, "viewport 10 + clicked row 2");
        assert_eq!(editor.cursor.col, 4);
        assert_eq!(
            editor.panes()[0].cursor,
            editor.cursor,
            "pane mirror synced"
        );
    }

    #[test]
    fn click_beyond_eol_clamps_to_line_end() {
        let mut editor = editor_with_lines(100);
        let gutter = gutter_width(&editor, 0);

        // Row 2 shows "line3" (5 chars): clicking far right lands on its end.
        handle_mouse_event(
            &mut editor,
            mouse(MouseEventKind::Down(MouseButton::Left), gutter + 90, 2),
        );

        assert_eq!(editor.cursor.line, 2);
        assert_eq!(editor.cursor.col, 4);
    }

    #[test]
    fn click_in_other_pane_focuses_and_positions() {
        let mut editor = test_editor_with_vsplit();
        let inactive = 1 - editor.active_pane_idx();
        let (col, row) = center_of_pane(&editor, inactive);

        handle_mouse_event(
            &mut editor,
            mouse(MouseEventKind::Down(MouseButton::Left), col, row),
        );

        assert_eq!(editor.active_pane_idx(), inactive);
    }

    #[test]
    fn click_in_insert_mode_stays_insert() {
        let mut editor = editor_with_lines(100);
        editor.mode = Mode::Insert;
        let gutter = gutter_width(&editor, 0);

        handle_mouse_event(
            &mut editor,
            mouse(MouseEventKind::Down(MouseButton::Left), gutter + 2, 5),
        );

        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor.line, 5);
    }

    #[test]
    fn click_in_visual_mode_clears_to_normal() {
        let mut editor = editor_with_lines(100);
        editor.enter_visual_mode();
        assert_eq!(editor.mode, Mode::Visual);
        let gutter = gutter_width(&editor, 0);

        handle_mouse_event(
            &mut editor,
            mouse(MouseEventKind::Down(MouseButton::Left), gutter + 1, 8),
        );

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.line, 8);
    }

    #[test]
    fn click_on_wide_char_line_maps_display_columns() {
        let mut editor = Editor::default();
        editor.replace_buffer_content("日本語abc\nplain\n");
        editor.set_size(120, 40);
        editor.update_pane_rects();
        let gutter = gutter_width(&editor, 0);

        // The three double-width chars cover display cells 0-5; cell 6 is 'a'.
        handle_mouse_event(
            &mut editor,
            mouse(MouseEventKind::Down(MouseButton::Left), gutter + 6, 0),
        );

        assert_eq!(editor.cursor.line, 0);
        assert_eq!(editor.cursor.col, 3, "char index, not display cell");
    }

    #[test]
    fn click_on_explorer_row_selects_it() {
        let tmp = std::env::temp_dir().join(format!(
            "nevi_mouse_explorer_click_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(tmp.join(name), "x\n").expect("write file");
        }

        let mut editor = editor_with_lines(50);
        editor.explorer.set_root(tmp.clone());
        editor.explorer.show();
        editor.update_pane_rects();

        // Row 0 is the header; row 2 is the second visible entry.
        handle_mouse_event(
            &mut editor,
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 2),
        );
        assert_eq!(editor.explorer.selected, 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn wheel_scrolls_markdown_preview_when_open() {
        let tmp = std::env::temp_dir().join(format!(
            "nevi_mouse_md_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("notes.md");
        let body: String = (1..=80).map(|i| format!("- item {i}\n")).collect();
        std::fs::write(&path, format!("# Notes\n\n{body}")).expect("write markdown");

        let mut editor = Editor::default();
        editor.set_size(120, 40);
        editor.open_file(path).expect("open markdown");
        editor.open_markdown_preview().expect("open preview");
        assert_eq!(editor.markdown_preview.as_ref().unwrap().scroll, 0);

        handle_mouse_event(&mut editor, mouse(MouseEventKind::ScrollDown, 10, 10));
        assert_eq!(editor.markdown_preview.as_ref().unwrap().scroll, 3);

        handle_mouse_event(&mut editor, mouse(MouseEventKind::ScrollUp, 10, 10));
        assert_eq!(editor.markdown_preview.as_ref().unwrap().scroll, 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn wheel_in_finder_scrolls_preview_pane_only() {
        let mut editor = editor_with_lines(50);
        editor.mode = Mode::Finder;
        editor.finder.preview_content = (1..=100).map(|i| format!("line {i}")).collect();
        let viewport_before = editor.viewport_offset;

        handle_mouse_event(&mut editor, mouse(MouseEventKind::ScrollDown, 60, 20));
        assert_eq!(editor.finder.preview_scroll, 3);
        assert_eq!(editor.viewport_offset, viewport_before, "buffer untouched");

        handle_mouse_event(&mut editor, mouse(MouseEventKind::ScrollUp, 60, 20));
        assert_eq!(editor.finder.preview_scroll, 0);
    }

    #[test]
    fn wheel_over_explorer_moves_selection_one_row() {
        let tmp = std::env::temp_dir().join(format!(
            "nevi_mouse_explorer_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(tmp.join(name), "x\n").expect("write file");
        }

        let mut editor = editor_with_lines(50);
        editor.explorer.set_root(tmp.clone());
        editor.explorer.show();
        editor.update_pane_rects();
        assert_eq!(editor.explorer.selected, 0);

        handle_mouse_event(&mut editor, mouse(MouseEventKind::ScrollDown, 1, 5));
        assert_eq!(editor.explorer.selected, 1, "one row per tick");

        handle_mouse_event(&mut editor, mouse(MouseEventKind::ScrollUp, 1, 5));
        assert_eq!(editor.explorer.selected, 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn horizontal_wheel_moves_h_offset() {
        let mut editor = Editor::default();
        let long_line: String = "x".repeat(600);
        editor.replace_buffer_content(&format!("{long_line}\nshort\n"));
        editor.set_size(120, 40);
        editor.update_pane_rects();
        let (col, row) = center_of_pane(&editor, 0);

        handle_mouse_event(&mut editor, mouse(MouseEventKind::ScrollRight, col, row));
        assert_eq!(editor.h_offset, 6);
        assert!(editor.cursor.col >= 6, "cursor dragged into view");

        handle_mouse_event(&mut editor, mouse(MouseEventKind::ScrollLeft, col, row));
        assert_eq!(editor.h_offset, 0);
    }

    #[test]
    fn pane_at_finds_pane_in_vertical_split() {
        let editor = test_editor_with_vsplit();
        let left = editor.panes()[0].rect;
        let right = editor.panes()[1].rect;
        assert_eq!(pane_at(&editor, left.x, left.y), Some(0));
        assert_eq!(pane_at(&editor, right.x + 1, right.y + 1), Some(1));
    }

    #[test]
    fn pane_at_rejects_statusline_and_command_rows() {
        // text_rows = term_height - 2; the bottom two rows belong to no pane.
        let editor = single_pane_editor(120, 40);
        assert_eq!(pane_at(&editor, 10, 38), None);
        assert_eq!(pane_at(&editor, 10, 39), None);
    }

    #[test]
    fn pane_at_rejects_explorer_column_when_visible() {
        let mut editor = single_pane_editor(120, 40);
        editor.explorer.show();
        editor.update_pane_rects();
        assert_eq!(pane_at(&editor, 0, 5), None);
    }
}
