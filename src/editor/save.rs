use super::Editor;

impl Editor {
    /// Save all modified buffers
    /// Returns the count of buffers saved, or error if any buffer fails
    pub fn save_all(&mut self) -> anyhow::Result<usize> {
        let mut saved_count = 0;
        for i in 0..self.buffers.len() {
            if self.buffers[i].dirty && self.buffers[i].path.is_some() {
                self.ensure_buffer_can_save(i, false)?;
                let was_dirty = self.buffers[i].dirty;
                self.buffers[i].save()?;
                self.mark_bufferline_if_dirty_changed(i, was_dirty);
                saved_count += 1;
            }
        }
        self.update_git_diff();
        self.refresh_explorer_git_statuses();
        Ok(saved_count)
    }

    /// Save to a specific file
    pub fn save_as(&mut self, path: std::path::PathBuf) -> anyhow::Result<()> {
        if self.buffers[self.current_buffer_idx].is_read_only() {
            anyhow::bail!("Buffer is read-only");
        }
        self.buffers[self.current_buffer_idx].set_file_path(path);
        self.mark_bufferline_damage();
        self.save()
    }

    /// Save to a specific file, intentionally overwriting external disk changes.
    pub fn save_as_force(&mut self, path: std::path::PathBuf) -> anyhow::Result<()> {
        if self.buffers[self.current_buffer_idx].is_read_only() {
            anyhow::bail!("Buffer is read-only");
        }
        self.buffers[self.current_buffer_idx].set_file_path(path);
        self.mark_bufferline_damage();
        self.save_force()
    }

    /// Save the current buffer
    pub fn save(&mut self) -> anyhow::Result<()> {
        self.save_buffer(self.current_buffer_idx, false)
    }

    /// Save the current buffer, intentionally overwriting external disk changes.
    pub fn save_force(&mut self) -> anyhow::Result<()> {
        self.save_buffer(self.current_buffer_idx, true)
    }

    pub fn save_buffer(&mut self, buffer_idx: usize, force: bool) -> anyhow::Result<()> {
        self.ensure_buffer_can_save(buffer_idx, force)?;
        let was_dirty = self.buffers[buffer_idx].dirty;
        self.buffers[buffer_idx].save()?;
        self.mark_bufferline_if_dirty_changed(buffer_idx, was_dirty);
        self.status_message = Some(format!(
            "\"{}\" written",
            self.buffers[buffer_idx].display_name()
        ));
        // Update git diff after save (file now matches HEAD if no other changes)
        self.update_git_diff();
        self.refresh_explorer_git_statuses();
        Ok(())
    }

    pub(super) fn ensure_buffer_can_save(
        &mut self,
        buffer_idx: usize,
        force: bool,
    ) -> anyhow::Result<()> {
        if self.buffers[buffer_idx].is_read_only() {
            self.set_status("Buffer is read-only");
            anyhow::bail!("Buffer is read-only");
        }
        if !force
            && self.buffers[buffer_idx].dirty
            && self.buffers[buffer_idx].has_external_changes()
        {
            let message = "Write blocked: file changed on disk; use :w! to overwrite".to_string();
            self.set_status(message.clone());
            anyhow::bail!(message);
        }
        Ok(())
    }
}

use super::{BufferFormatOutcome, FormatPolicy};
use crate::lsp::types::TextEdit;
use std::{collections::VecDeque, path::PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct WriteOptions {
    pub force: bool,
    pub format: Option<FormatPolicy>,
}
#[derive(Debug, Default, Clone)]
pub struct SaveSummary {
    pub saved: usize,
    pub formatted: usize,
    pub warnings: Vec<String>,
    kind: SaveKind,
    formatter: Option<String>,
    formatter_failed: bool,
    lsp_edits: usize,
    previous_error: Option<String>,
}
#[derive(Debug, Default, Clone)]
enum SaveKind {
    Current(String),
    All,
    #[default]
    Format,
}
impl SaveSummary {
    fn with_previous_error(&self, message: String) -> String {
        match &self.previous_error {
            Some(error) => format!("Previous write failed: {error}; {message}"),
            None => message,
        }
    }
    fn message(&self) -> String {
        let mut message = match &self.kind {
            SaveKind::Current(name) => {
                if let Some(command) = &self.formatter {
                    if self.formatted > 0 {
                        format!("Formatted with {command} and saved")
                    } else {
                        format!("Saved (formatted with {command})")
                    }
                } else if self.formatter_failed {
                    "Saved".into()
                } else if self.formatted > 0 {
                    format!("Formatted and saved ({} edits)", self.lsp_edits)
                } else {
                    format!("\"{name}\" written")
                }
            }
            SaveKind::All if self.saved == 0 => "No modified buffers to save".into(),
            SaveKind::All => {
                let noun = if self.saved == 1 { "buffer" } else { "buffers" };
                if self.formatted > 0 {
                    format!(
                        "Formatted with {} and saved {} {noun}",
                        self.formatter.as_deref().unwrap_or("LSP"),
                        self.saved
                    )
                } else {
                    format!("Saved {} {noun}", self.saved)
                }
            }
            SaveKind::Format if self.formatted > 0 => "Formatted".into(),
            SaveKind::Format => "No changes to format or save".into(),
        };
        if !self.warnings.is_empty() {
            message.push_str(&format!(" ({})", self.warnings.join("; ")));
        }
        self.with_previous_error(message)
    }
}
#[derive(Debug)]
pub enum SaveProgress {
    Complete(SaveSummary),
    Pending,
}
#[derive(Debug, Clone)]
pub struct FormatRequest {
    pub token: u64,
    pub buffer_id: u64,
    pub path: PathBuf,
    pub version: u64,
    pub content: String,
}
#[derive(Debug, Clone)]
struct SaveTarget {
    id: u64,
    path: PathBuf,
    options: WriteOptions,
    format_done: bool,
}
#[derive(Debug)]
struct PendingFormat {
    request: FormatRequest,
    write: bool,
    sent: bool,
}
#[derive(Debug, Default)]
pub(super) struct SaveState {
    queue: VecDeque<SaveTarget>,
    pending: Option<PendingFormat>,
    summary: SaveSummary,
    next_token: u64,
}

impl Editor {
    pub fn has_pending_saves(&self) -> bool {
        !self.save_state.queue.is_empty()
    }
    pub fn format_request_is_active(&self, token: u64) -> bool {
        self.save_state
            .pending
            .as_ref()
            .is_some_and(|p| p.request.token == token)
    }
    pub fn discard_pending_saves(&mut self) {
        self.save_state.queue.clear();
        self.save_state.pending = None;
        self.save_state.summary = SaveSummary::default();
    }
    pub fn note_failed_flush(&mut self, error: String) {
        self.save_state.summary.previous_error = Some(error);
    }
    pub fn save_progress_message(&self, progress: SaveProgress) -> String {
        match progress {
            SaveProgress::Complete(summary) => summary.message(),
            SaveProgress::Pending => self
                .save_state
                .summary
                .with_previous_error("Formatting with LSP...".into()),
        }
    }
    fn end_failed_save<T>(&mut self, result: anyhow::Result<T>) -> anyhow::Result<T> {
        result.map_err(|error| {
            let error = if self.save_state.summary.previous_error.is_some() {
                anyhow::anyhow!(
                    self.save_state
                        .summary
                        .with_previous_error(error.to_string())
                )
            } else {
                error
            };
            // A failed transaction has ended. Dirty buffers retain the unsaved
            // work; a later command must choose its own targets and force policy.
            self.discard_pending_saves();
            error
        })
    }
    fn start_save_command(
        &mut self,
        command: impl FnOnce(&mut Self) -> anyhow::Result<SaveProgress>,
    ) -> anyhow::Result<SaveProgress> {
        let flush_error = self.flush_pending_saves().err();
        self.save_state.summary = SaveSummary::default();
        if let Some(error) = flush_error {
            self.note_failed_flush(error.to_string());
        }
        let result = command(self);
        self.end_failed_save(result)
    }
    fn save_target_index(&self, id: u64, path: &PathBuf) -> anyhow::Result<usize> {
        self.buffers
            .iter()
            .position(|b| b.id() == id && b.path.as_ref() == Some(path))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Save target was closed, replaced, or renamed: {}",
                    path.display()
                )
            })
    }
    fn enqueue_format(&mut self, idx: usize, write: bool) -> anyhow::Result<()> {
        anyhow::ensure!(!self.buffers[idx].is_read_only(), "Buffer is read-only");
        let b = &self.buffers[idx];
        let path = b
            .path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No filename"))?;
        self.save_state.next_token += 1;
        self.save_state.pending = Some(PendingFormat {
            request: FormatRequest {
                token: self.save_state.next_token,
                buffer_id: b.id(),
                path,
                version: b.version(),
                content: b.content(),
            },
            write,
            sent: false,
        });
        Ok(())
    }
    pub fn take_format_request(&mut self) -> Option<FormatRequest> {
        let p = self.save_state.pending.as_mut()?;
        if p.sent {
            return None;
        }
        if let Some(buffer) = self.buffers.iter().find(|buffer| {
            buffer.id() == p.request.buffer_id && buffer.path.as_ref() == Some(&p.request.path)
        }) {
            p.request.version = buffer.version();
            p.request.content = buffer.content();
        }
        p.sent = true;
        Some(p.request.clone())
    }
    pub fn validate_format_request(&self, request: &FormatRequest) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.format_request_is_active(request.token),
            "Formatting request is no longer active"
        );
        let idx = self.save_target_index(request.buffer_id, &request.path)?;
        anyhow::ensure!(!self.buffers[idx].is_read_only(), "Buffer is read-only");
        Ok(())
    }
    pub fn begin_save_current(&mut self, options: WriteOptions) -> anyhow::Result<SaveProgress> {
        self.start_save_command(|editor| editor.start_save_current(options))
    }
    fn start_save_current(&mut self, options: WriteOptions) -> anyhow::Result<SaveProgress> {
        self.save_state.summary.kind = SaveKind::Current(self.buffer().display_name());
        self.ensure_buffer_can_save(self.current_buffer_idx, options.force)?;
        let b = self.buffer();
        let target = SaveTarget {
            id: b.id(),
            path: b
                .path
                .clone()
                .ok_or_else(|| anyhow::anyhow!("No filename"))?,
            options,
            format_done: false,
        };
        self.save_state.queue.push_back(target);
        self.drive_saves(false)
    }
    pub fn begin_save_all(&mut self, options: WriteOptions) -> anyhow::Result<SaveProgress> {
        self.start_save_command(|editor| editor.start_save_all(options))
    }
    fn start_save_all(&mut self, options: WriteOptions) -> anyhow::Result<SaveProgress> {
        self.save_state.summary.kind = SaveKind::All;
        self.save_state.queue = self
            .buffers
            .iter()
            .filter(|b| b.dirty)
            .filter_map(|b| {
                Some(SaveTarget {
                    id: b.id(),
                    path: b.path.clone()?,
                    options,
                    format_done: false,
                })
            })
            .collect();
        self.drive_saves(false)
    }
    pub fn begin_format_current(&mut self) -> anyhow::Result<SaveProgress> {
        self.start_save_command(Self::start_format_current)
    }
    fn start_format_current(&mut self) -> anyhow::Result<SaveProgress> {
        match self.format_current_buffer()? {
            BufferFormatOutcome::RequestLsp => {
                self.enqueue_format(self.current_buffer_idx, false)?;
                Ok(SaveProgress::Pending)
            }
            BufferFormatOutcome::Applied { changed, .. } => {
                self.save_state.summary.formatted = usize::from(changed);
                Ok(SaveProgress::Complete(self.save_state.summary.clone()))
            }
            BufferFormatOutcome::Failed { command, error } => {
                anyhow::bail!("{} error: {}", command, error)
            }
            BufferFormatOutcome::Skipped => Ok(SaveProgress::Complete(SaveSummary::default())),
        }
    }
    fn drive_saves(&mut self, external_only: bool) -> anyhow::Result<SaveProgress> {
        let result = self.drive_saves_inner(external_only);
        self.end_failed_save(result)
    }
    fn drive_saves_inner(&mut self, external_only: bool) -> anyhow::Result<SaveProgress> {
        while let Some(target) = self.save_state.queue.front().cloned() {
            let idx = self.save_target_index(target.id, &target.path)?;
            self.ensure_buffer_can_save(idx, target.options.force)?;
            if !target.format_done {
                if let Some(policy) = target.options.format {
                    let policy = if external_only {
                        FormatPolicy::ExternalOnly
                    } else {
                        policy
                    };
                    let outcome = self.format_buffer(idx, policy)?;
                    self.save_state.queue.front_mut().unwrap().format_done = true;
                    match outcome {
                        BufferFormatOutcome::RequestLsp => {
                            self.enqueue_format(idx, true)?;
                            return Ok(SaveProgress::Pending);
                        }
                        BufferFormatOutcome::Applied { command, changed } => {
                            self.save_state.summary.formatted += usize::from(changed);
                            if changed || self.save_state.summary.formatter.is_none() {
                                self.save_state.summary.formatter = Some(command);
                            }
                        }
                        BufferFormatOutcome::Failed { command, error } => {
                            self.save_state.summary.formatter_failed = true;
                            self.save_state
                                .summary
                                .warnings
                                .push(format!("{} failed: {}", command, error));
                        }
                        BufferFormatOutcome::Skipped => {}
                    }
                }
            }
            self.save_buffer(idx, target.options.force)?;
            self.save_state.queue.pop_front();
            self.save_state.summary.saved += 1;
        }
        Ok(SaveProgress::Complete(self.save_state.summary.clone()))
    }
    pub fn finish_format_request(
        &mut self,
        token: u64,
        result: Result<Vec<TextEdit>, String>,
    ) -> anyhow::Result<SaveProgress> {
        self.finish_format(token, result, false)
    }
    /// No language server handles this file, which is the same as having no
    /// formatter: a save writes the text as is, and only `:Format` reports it.
    pub fn skip_format_request(&mut self, token: u64) -> anyhow::Result<SaveProgress> {
        let write = self
            .save_state
            .pending
            .as_ref()
            .is_some_and(|p| p.request.token == token && p.write);
        let result = if write {
            Ok(Vec::new())
        } else {
            Err("LSP not available".into())
        };
        self.finish_format_request(token, result)
    }
    fn finish_format(
        &mut self,
        token: u64,
        result: Result<Vec<TextEdit>, String>,
        external_only: bool,
    ) -> anyhow::Result<SaveProgress> {
        let result = self.finish_format_inner(token, result, external_only);
        self.end_failed_save(result)
    }
    fn finish_format_inner(
        &mut self,
        token: u64,
        result: Result<Vec<TextEdit>, String>,
        external_only: bool,
    ) -> anyhow::Result<SaveProgress> {
        if !self.format_request_is_active(token) {
            return Ok(SaveProgress::Pending);
        }
        let pending = self.save_state.pending.take().unwrap();
        let r = pending.request;
        let idx = self.save_target_index(r.buffer_id, &r.path)?;
        anyhow::ensure!(!self.buffers[idx].is_read_only(), "Buffer is read-only");
        if pending.write {
            let target = self
                .save_state
                .queue
                .front()
                .expect("pending save has a target");
            self.ensure_buffer_can_save(idx, target.options.force)?;
        }
        let result = if self.buffers[idx].version() != r.version {
            Err("Formatting result is stale; ignored edits".into())
        } else {
            result
        };
        match result {
            Ok(edits) => {
                self.save_state.summary.lsp_edits += edits.len();
                self.save_state.summary.formatted +=
                    usize::from(self.apply_format_edits(idx, &edits)?)
            }
            Err(error) if pending.write => self.save_state.summary.warnings.push(error),
            Err(error) => anyhow::bail!(error),
        }
        self.drive_saves(external_only)
    }
    pub fn flush_pending_saves(&mut self) -> anyhow::Result<SaveSummary> {
        if let Some(pending) = self.save_state.pending.as_ref() {
            let token = pending.request.token;
            if pending.write {
                self.finish_format(
                    token,
                    Err("LSP formatting flushed; saved without edits".into()),
                    true,
                )?;
            } else {
                self.save_state.pending = None;
                self.set_status("Pending formatting interrupted");
            }
        }
        match self.drive_saves(true)? {
            SaveProgress::Complete(summary) => Ok(summary),
            SaveProgress::Pending => unreachable!("external-only flush never requests LSP"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{FormatPolicy, save_test_support::Fixture};
    fn options(force: bool) -> WriteOptions {
        WriteOptions {
            force,
            format: Some(FormatPolicy::ExternalThenLsp),
        }
    }

    #[test]
    fn save_rechecks_conflicts_after_formatting() {
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        e.replace_buffer_content("edit\n");
        e.begin_save_current(options(false)).unwrap();
        let r = e.take_format_request().unwrap();
        std::fs::write(&r.path, "external change\n").unwrap();
        assert!(e.finish_format_request(r.token, Ok(vec![])).is_err());
        assert_eq!(
            std::fs::read_to_string(&r.path).unwrap(),
            "external change\n"
        );
        assert!(e.buffer().dirty);
        assert!(!e.has_pending_saves());
        e.begin_save_current(options(true)).unwrap();
        let r = e.take_format_request().unwrap();
        e.finish_format_request(r.token, Ok(vec![])).unwrap();
        assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "edit\n");
        for force in [false, true] {
            let f = Fixture::new();
            let mut e = f.editor("a.md");
            e.replace_buffer_content("foo\n");
            let path = f.0.join("a.md");
            f.formatter(
                &mut e,
                "sh",
                &[
                    "-c",
                    "printf external-change > \"$1\"; sed s/foo/bar/",
                    "formatter",
                    path.to_str().unwrap(),
                ],
            );
            let result = e.begin_save_current(options(force));
            assert_eq!(result.is_ok(), force);
            assert_eq!(e.buffer().content(), "bar\n");
            assert_eq!(e.buffer().dirty, !force);
            assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                if force { "bar\n" } else { "external-change" }
            );
        }
    }
    #[test]
    fn save_all_stops_on_write_error_but_continues_on_format_error() {
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        f.formatter(&mut e, "false", &[]);
        e.replace_buffer_content("first\n");
        let a = e.current_buffer_idx;
        e.open_file(f.file("b.md", "foo\n")).unwrap();
        e.replace_buffer_content("second\n");
        let b = e.current_buffer_idx;
        e.open_file(f.file("c.md", "foo\n")).unwrap();
        e.replace_buffer_content("third\n");
        e.buffers[b].set_read_only(true);
        assert!(e.begin_save_all(options(false)).is_err());
        assert!(!e.buffers[a].dirty);
        assert!(e.buffers[b].dirty);
        assert!(e.buffer().dirty);
        assert_eq!(
            std::fs::read_to_string(f.0.join("a.md")).unwrap(),
            "first\n"
        );
        assert_eq!(std::fs::read_to_string(f.0.join("c.md")).unwrap(), "foo\n");
        assert!(!e.has_pending_saves());
        assert!(e.take_format_request().is_none());
        e.buffers[b].set_read_only(false);
        let SaveProgress::Complete(summary) = e.begin_save_all(options(false)).unwrap() else {
            panic!("external formatter finishes synchronously");
        };
        assert_eq!(summary.saved, 2);
        assert!(!summary.warnings.is_empty());
        assert!(!e.has_pending_saves());
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        e.replace_buffer_content("foo\n");
        e.open_file(f.file("b.md", "old\n")).unwrap();
        e.replace_buffer_content("foo\n");
        e.buffer_mut().set_read_only(true);
        assert!(
            e.begin_save_all(WriteOptions {
                force: false,
                format: None
            })
            .is_err()
        );
        f.formatter(&mut e, "sed", &["s/foo/bar/"]);
        e.buffer_mut().set_read_only(false);
        assert!(!e.has_pending_saves());
        let SaveProgress::Complete(summary) = e
            .begin_save_all(WriteOptions {
                force: false,
                format: None,
            })
            .unwrap()
        else {
            panic!("raw writes finish synchronously");
        };
        assert_eq!(summary.saved, 1);
        assert_eq!(summary.formatted, 0);
        assert_eq!(std::fs::read_to_string(f.0.join("b.md")).unwrap(), "foo\n");
    }
    #[test]
    fn save_request_tracks_buffer_identity_not_vector_index() {
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        let a = e.current_buffer_idx;
        e.open_file(f.file("b.md", "foo\n")).unwrap();
        e.replace_buffer_content("target\n");
        e.begin_save_current(options(false)).unwrap();
        let r = e.take_format_request().unwrap();
        e.buffers.remove(a);
        e.current_buffer_idx -= 1;
        e.finish_format_request(r.token, Ok(vec![])).unwrap();
        assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "target\n");
        e.begin_save_current(options(false)).unwrap();
        let r = e.take_format_request().unwrap();
        e.buffers[e.current_buffer_idx] = crate::editor::Buffer::from_file(r.path.clone()).unwrap();
        assert!(e.finish_format_request(r.token, Ok(vec![])).is_err());
        assert_eq!(e.buffer().content(), "target\n");
        assert!(!e.has_pending_saves());
        assert!(!e.format_request_is_active(r.token));
        e.begin_save_current(WriteOptions {
            force: true,
            format: None,
        })
        .unwrap();
    }
    #[test]
    fn format_dispatch_uses_latest_buffer_snapshot() {
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        e.begin_save_current(options(false)).unwrap();
        e.replace_buffer_content("typed before dispatch\n");
        let request = e.take_format_request().unwrap();
        assert_eq!(request.content, "typed before dispatch\n");
        assert_eq!(request.version, e.buffer().version());
        e.finish_format_request(request.token, Ok(vec![])).unwrap();
        assert_eq!(
            std::fs::read_to_string(&request.path).unwrap(),
            "typed before dispatch\n"
        );
    }
    #[test]
    fn begin_save_replaces_failed_flush_with_new_force_policy() {
        for all in [false, true] {
            let f = Fixture::new();
            let mut e = f.editor("a.md");
            e.replace_buffer_content("foo\n");
            e.begin_save_current(options(false)).unwrap();
            let old = e.take_format_request().unwrap();
            std::fs::write(&old.path, "external\n").unwrap();
            let options = WriteOptions {
                force: true,
                format: None,
            };
            let result = if all {
                e.begin_save_all(options)
            } else {
                e.begin_save_current(options)
            };
            let message = e.save_progress_message(result.unwrap());
            assert!(message.contains("Previous write failed:"));
            assert!(message.contains(if all {
                "Saved 1 buffer"
            } else {
                "\"a.md\" written"
            }));
            assert_eq!(std::fs::read_to_string(&old.path).unwrap(), "foo\n");
            assert!(!e.has_pending_saves());
            assert!(!e.format_request_is_active(old.token));
        }
    }
}
