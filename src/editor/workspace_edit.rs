//! Applying a language server's multi-file edit (a rename) across the
//! workspace. Like Neovim, a file that is open in any buffer is edited in
//! that buffer and left for the user to save; only files that are not open
//! are rewritten on disk. Writing an open file on disk instead would leave
//! the buffer stale, and saving it later would silently drop the edit.

use super::Editor;
use super::buffer::{regular_file_metadata, write_file_atomically};
use crate::lsp::types::TextEdit;
use std::path::Path;

/// What a workspace edit changed, for the status line and the LSP sync.
#[derive(Debug, Default)]
pub struct WorkspaceEditOutcome {
    pub total_edits: usize,
    pub files_changed: usize,
    /// The current buffer was edited, so the server needs a didChange for it.
    pub current_buffer_changed: bool,
    /// Open buffers other than the current one that now hold unsaved edits.
    pub other_buffers_modified: usize,
    /// One "path: reason" entry per file that could not be edited.
    pub errors: Vec<String>,
}

impl WorkspaceEditOutcome {
    /// Status line text for a rename that had no errors.
    pub fn summary(&self) -> String {
        let mut message = format!(
            "Renamed: {} edits in {} file(s)",
            self.total_edits, self.files_changed
        );
        if self.other_buffers_modified > 0 {
            message.push_str(&format!(
                ", {} other buffer(s) unsaved (:wa)",
                self.other_buffers_modified
            ));
        }
        message
    }
}

impl Editor {
    /// Apply edits grouped by file URI, as a rename returns them.
    pub fn apply_workspace_edit(
        &mut self,
        edits: &[(String, Vec<TextEdit>)],
    ) -> WorkspaceEditOutcome {
        let mut outcome = WorkspaceEditOutcome::default();
        let mut other_buffers: Vec<usize> = Vec::new();

        for (uri, file_edits) in edits {
            let Some(path) = crate::lsp::uri_to_path(uri) else {
                continue;
            };
            let result = match self.buffer_index_for_path(&path) {
                // Buffer-level edits are no-ops on a read-only buffer, so
                // applying them would report a rename that never happened.
                Some(idx) if self.buffers[idx].is_read_only() => {
                    Err(anyhow::anyhow!("buffer is read-only"))
                }
                Some(idx) => {
                    self.apply_text_edits_to_buffer(idx, file_edits);
                    if idx == self.current_buffer_idx {
                        outcome.current_buffer_changed = true;
                    } else if !other_buffers.contains(&idx) {
                        other_buffers.push(idx);
                    }
                    Ok(file_edits.len())
                }
                None => apply_edits_to_file(&path, file_edits),
            };
            match result {
                Ok(count) => {
                    outcome.total_edits += count;
                    outcome.files_changed += 1;
                }
                Err(err) => outcome.errors.push(format!("{}: {}", path.display(), err)),
            }
        }

        outcome.other_buffers_modified = other_buffers.len();
        self.clamp_inactive_panes();
        // Any pane may show an edited buffer, not just the active one.
        self.render_damage.mark_full();
        outcome
    }

    /// `apply_text_edits` for any buffer. It only knows the current buffer,
    /// so the target is made current for the call and everything the call
    /// touches on the Editor is put back afterwards.
    fn apply_text_edits_to_buffer(&mut self, idx: usize, edits: &[TextEdit]) {
        if idx == self.current_buffer_idx {
            self.apply_text_edits(edits);
            return;
        }

        // The undo group records a cursor to return to; use a pane's view of
        // the buffer when one shows it.
        let target_cursor = self
            .panes
            .iter()
            .find(|pane| pane.buffer_idx == idx)
            .map(|pane| pane.cursor)
            .unwrap_or_default();

        self.save_current_undo_stack();
        let current_idx = std::mem::replace(&mut self.current_buffer_idx, idx);
        let cursor = std::mem::replace(&mut self.cursor, target_cursor);
        // The change list holds positions without a buffer, so an entry for
        // the other buffer would send g; to a wrong place in this one. The
        // edit timestamp drives the current buffer's syntax debounce.
        let change_list = std::mem::take(&mut self.change_list);
        let last_edit_at = self.last_edit_at;
        self.load_current_undo_stack();

        self.apply_text_edits(edits);

        self.save_current_undo_stack();
        self.current_buffer_idx = current_idx;
        self.cursor = cursor;
        self.change_list = change_list;
        self.last_edit_at = last_edit_at;
        self.load_current_undo_stack();
    }

    /// Keep panes other than the active one inside their buffer after an
    /// edit they did not make. The active pane is mirrored on the Editor and
    /// clamped there; these are only checked again when they are focused.
    fn clamp_inactive_panes(&mut self) {
        for (pane_idx, pane) in self.panes.iter_mut().enumerate() {
            if pane_idx == self.active_pane {
                continue;
            }
            let Some(buffer) = self.buffers.get(pane.buffer_idx) else {
                continue;
            };
            let last_line = buffer.addressable_line_count().saturating_sub(1);
            pane.cursor.line = pane.cursor.line.min(last_line);
            let max_col = buffer.line_len(pane.cursor.line).saturating_sub(1);
            pane.cursor.col = pane.cursor.col.min(max_col);
            pane.viewport_offset = pane.viewport_offset.min(pane.cursor.line);
        }
    }
}

/// Apply LSP text edits to a file that is not open: read it, apply the edits
/// from the end backwards, and write it through the same atomic path as a
/// save (temp file, permissions kept, a symlink's target updated).
fn apply_edits_to_file(path: &Path, edits: &[TextEdit]) -> anyhow::Result<usize> {
    regular_file_metadata(path)?;
    let content = std::fs::read_to_string(path)?;
    let mut text = ropey::Rope::from_str(&content);

    let mut sorted_edits: Vec<&TextEdit> = edits.iter().collect();
    sorted_edits.sort_by(|a, b| match b.end_line.cmp(&a.end_line) {
        std::cmp::Ordering::Equal => b.end_col.cmp(&a.end_col),
        other => other,
    });

    for edit in &sorted_edits {
        let start = utf16_position_to_rope_char(&text, edit.start_line, edit.start_col);
        let end = utf16_position_to_rope_char(&text, edit.end_line, edit.end_col);

        if start <= end && end <= text.len_chars() {
            if start < end {
                text.remove(start..end);
            }
            if !edit.new_text.is_empty() {
                text.insert(start, &edit.new_text);
            }
        }
    }

    write_file_atomically(path, |writer| {
        text.write_to(writer)?;
        Ok(())
    })?;

    Ok(sorted_edits.len())
}

fn utf16_position_to_rope_char(text: &ropey::Rope, line: usize, utf16_col: usize) -> usize {
    if line >= text.len_lines() {
        return text.len_chars();
    }

    let line_start = text.line_to_char(line);
    let line_text = text.line(line).to_string();
    let line_text = line_text.trim_end_matches('\n');
    let line_len = line_text.chars().count();
    let col = crate::copilot::utf16_to_utf8_col(line_text, utf16_col as u32).min(line_len);

    line_start + col
}

#[cfg(test)]
mod tests {
    use super::apply_edits_to_file;
    use crate::editor::{Cursor, Editor};
    use crate::lsp::path_to_uri;
    use crate::lsp::types::TextEdit;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.canonicalize().expect("canonical temp dir")
    }

    fn edit(line: usize, start_col: usize, end_col: usize, new_text: &str) -> TextEdit {
        TextEdit {
            start_line: line,
            start_col,
            end_line: line,
            end_col,
            new_text: new_text.to_string(),
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("read file")
    }

    // The rename reaches a file that is open in a background buffer with an
    // unsaved change. It used to be rewritten on disk from the old disk text,
    // so the buffer missed the rename and saving it later dropped it.
    #[test]
    fn rename_edits_an_open_background_buffer_instead_of_its_file() {
        let tmp = unique_temp_dir("nevi_ws_edit_background");
        let a = tmp.join("a.rs");
        let b = tmp.join("b.rs");
        std::fs::write(&a, "fn old() {}\n").expect("write a");
        std::fs::write(&b, "use old;\n").expect("write b");

        let mut editor = Editor::default();
        editor.open_file(b.clone()).expect("open b");
        editor.enter_insert_mode_end();
        editor.insert_char('!');
        editor.enter_normal_mode();
        editor.open_file(a.clone()).expect("open a");
        let b_idx = editor.buffer_index_for_path(&b).expect("b is open");

        let outcome = editor.apply_workspace_edit(&[
            (path_to_uri(&a), vec![edit(0, 3, 6, "new")]),
            (path_to_uri(&b), vec![edit(0, 4, 7, "new")]),
        ]);

        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert_eq!((outcome.total_edits, outcome.files_changed), (2, 2));
        assert!(outcome.current_buffer_changed);
        assert_eq!(outcome.other_buffers_modified, 1);
        assert_eq!(
            outcome.summary(),
            "Renamed: 2 edits in 2 file(s), 1 other buffer(s) unsaved (:wa)"
        );
        assert_eq!(read(&b), "use old;\n", "an open file is not written");
        assert_eq!(editor.buffer().content(), "fn new() {}\n");

        assert!(editor.switch_to_buffer(b_idx));
        assert_eq!(editor.buffer().content(), "use new;!\n");
        assert!(editor.buffer().dirty);
        crate::syntax::assert_tree_matches_fresh_parse(&editor.syntax, editor.buffer());
        editor.undo();
        assert_eq!(
            editor.buffer().content(),
            "use old;!\n",
            "one undo per rename"
        );
        editor.undo();
        assert_eq!(editor.buffer().content(), "use old;\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rename_in_the_current_buffer_is_one_undo_and_writes_nothing() {
        let tmp = unique_temp_dir("nevi_ws_edit_current");
        let a = tmp.join("a.rs");
        std::fs::write(&a, "fn old() { old() }\n").expect("write a");

        let mut editor = Editor::default();
        editor.open_file(a.clone()).expect("open a");
        let outcome = editor.apply_workspace_edit(&[(
            path_to_uri(&a),
            vec![edit(0, 3, 6, "new"), edit(0, 11, 14, "new")],
        )]);

        assert!(outcome.current_buffer_changed);
        assert_eq!(outcome.other_buffers_modified, 0);
        assert_eq!(outcome.summary(), "Renamed: 2 edits in 1 file(s)");
        assert_eq!(editor.buffer().content(), "fn new() { new() }\n");
        assert!(editor.buffer().dirty);
        assert_eq!(read(&a), "fn old() { old() }\n");
        editor.undo();
        assert_eq!(editor.buffer().content(), "fn old() { old() }\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // Saves write a temp file and rename it over the target; a rename into a
    // file that is not open now does the same, so the inode changes, the mode
    // is kept, and a symlink keeps pointing at its updated target.
    #[cfg(unix)]
    #[test]
    fn rename_writes_a_file_that_is_not_open_atomically() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let tmp = unique_temp_dir("nevi_ws_edit_disk");
        let plain = tmp.join("plain.rs");
        let target = tmp.join("target.rs");
        let link = tmp.join("link.rs");
        std::fs::write(&plain, "use old;\n").expect("write plain");
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o640))
            .expect("chmod plain");
        std::fs::write(&target, "use old;\n").expect("write target");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let inode_before = std::fs::metadata(&plain).expect("stat").ino();

        let mut editor = Editor::default();
        let outcome = editor.apply_workspace_edit(&[
            (path_to_uri(&plain), vec![edit(0, 4, 7, "new")]),
            (path_to_uri(&link), vec![edit(0, 4, 7, "new")]),
        ]);

        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert!(!outcome.current_buffer_changed);
        assert_eq!(outcome.other_buffers_modified, 0);
        assert_eq!(read(&plain), "use new;\n");
        let metadata = std::fs::metadata(&plain).expect("stat");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o640);
        assert_ne!(metadata.ino(), inode_before, "written through a temp file");
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("lstat")
                .file_type()
                .is_symlink()
        );
        assert_eq!(read(&target), "use new;\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // A server reports the path it resolved, which need not be the one the
    // buffer was opened with.
    #[cfg(unix)]
    #[test]
    fn rename_finds_a_buffer_opened_through_a_symlink() {
        let tmp = unique_temp_dir("nevi_ws_edit_symlink_buffer");
        let target = tmp.join("target.rs");
        let link = tmp.join("link.rs");
        std::fs::write(&target, "use old;\n").expect("write target");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        let mut editor = Editor::default();
        editor.open_file(link.clone()).expect("open link");
        assert_eq!(editor.buffer().path.as_deref(), Some(link.as_path()));
        let outcome =
            editor.apply_workspace_edit(&[(path_to_uri(&target), vec![edit(0, 4, 7, "new")])]);

        assert!(outcome.current_buffer_changed);
        assert_eq!(editor.buffer().content(), "use new;\n");
        assert_eq!(read(&target), "use old;\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rename_reports_a_read_only_open_buffer_without_touching_it() {
        let tmp = unique_temp_dir("nevi_ws_edit_read_only");
        let a = tmp.join("a.rs");
        let b = tmp.join("b.rs");
        std::fs::write(&a, "fn a() {}\n").expect("write a");
        std::fs::write(&b, "use old;\n").expect("write b");

        let mut editor = Editor::default();
        editor.open_file_read_only(b.clone()).expect("view b");
        editor.open_file(a).expect("open a");
        let outcome = editor.apply_workspace_edit(&[(path_to_uri(&b), vec![edit(0, 4, 7, "new")])]);

        assert_eq!(outcome.errors.len(), 1);
        assert!(outcome.errors[0].ends_with("buffer is read-only"));
        assert_eq!(outcome.total_edits, 0);
        let b_idx = editor.buffer_index_for_path(&b).expect("b is open");
        assert_eq!(editor.buffers[b_idx].content(), "use old;\n");
        assert_eq!(read(&b), "use old;\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rename_keeps_a_pane_on_the_edited_buffer_inside_it() {
        let tmp = unique_temp_dir("nevi_ws_edit_pane");
        let a = tmp.join("a.rs");
        let b = tmp.join("b.rs");
        std::fs::write(&a, "fn a() {}\n").expect("write a");
        std::fs::write(&b, "one\ntwo\nthree long line\n").expect("write b");

        let mut editor = Editor::default();
        editor.set_size(80, 24);
        editor.open_file(a).expect("open a");
        editor.vsplit(Some(b.clone())).expect("split b");
        editor.cursor = Cursor { line: 2, col: 10 };
        editor.viewport_offset = 2;
        editor.focus_pane(0);

        // Collapse b to a single short line.
        let outcome = editor.apply_workspace_edit(&[(
            path_to_uri(&b),
            vec![TextEdit {
                start_line: 0,
                start_col: 0,
                end_line: 3,
                end_col: 0,
                new_text: "x\n".to_string(),
            }],
        )]);

        assert_eq!(outcome.other_buffers_modified, 1);
        let pane = &editor.panes()[1];
        assert_eq!(editor.buffers[pane.buffer_idx].content(), "x\n");
        assert_eq!((pane.cursor.line, pane.cursor.col), (0, 0));
        assert_eq!(pane.viewport_offset, 0);

        editor.focus_pane(1);
        assert_eq!((editor.cursor.line, editor.cursor.col), (0, 0));
        editor.undo();
        assert_eq!(editor.buffer().content(), "one\ntwo\nthree long line\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn apply_edits_to_file_treats_columns_as_utf16_offsets() {
        let tmp = unique_temp_dir("nevi_disk_lsp_edit");
        let path = tmp.join("unicode.txt");
        std::fs::write(&path, "a😀b\n").expect("write file");

        let count = apply_edits_to_file(&path, &[edit(0, 3, 4, "X")]).expect("apply edits");

        assert_eq!(count, 1);
        assert_eq!(read(&path), "a😀X\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // A rename from a language server can name any path; a device behind a
    // symlink is refused before it is read or written.
    #[cfg(unix)]
    #[test]
    fn apply_edits_to_file_refuses_a_symlink_to_a_device() {
        let tmp = unique_temp_dir("nevi_disk_lsp_edit_device");
        let link = tmp.join("zero.rs");
        std::os::unix::fs::symlink("/dev/null", &link).expect("symlink");

        let err =
            apply_edits_to_file(&link, &[edit(0, 0, 0, "X")]).expect_err("a device is not a file");

        assert_eq!(
            err.to_string(),
            format!("\"{}\" is not a file", link.display())
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
