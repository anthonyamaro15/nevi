use super::{Buffer, Editor};
use crate::lsp::types::TextEdit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatPolicy {
    ExternalThenLsp,
    ExternalOnly,
}

/// Outcome of formatting the current buffer from `languages.toml` or LSP.
#[derive(Debug)]
pub enum BufferFormatOutcome {
    /// External formatter ran successfully.
    Applied { command: String, changed: bool },
    /// No external formatter is configured; request LSP formatting.
    RequestLsp,
    /// No backend is allowed by this policy.
    Skipped,
    /// External formatter was configured but failed.
    Failed {
        command: String,
        error: crate::formatter::FormatterError,
    },
}

impl Editor {
    /// Get the formatter config for the current buffer (if any)
    pub fn get_current_formatter(&self) -> Option<&crate::config::FormatterConfig> {
        self.get_formatter_for_buffer(self.current_buffer_idx)
    }

    /// Get the formatter config for a specific buffer by index (if any)
    pub fn get_formatter_for_buffer(
        &self,
        buffer_idx: usize,
    ) -> Option<&crate::config::FormatterConfig> {
        let buffer = self.buffers.get(buffer_idx)?;
        let path = buffer.path.as_ref()?;
        let language = Self::config_language_for_path(path)?;
        self.languages_config.get_formatter(&language)
    }

    pub fn format_current_buffer(&mut self) -> anyhow::Result<BufferFormatOutcome> {
        self.format_buffer(self.current_buffer_idx, FormatPolicy::ExternalThenLsp)
    }

    pub fn format_buffer(
        &mut self,
        buffer_idx: usize,
        policy: FormatPolicy,
    ) -> anyhow::Result<BufferFormatOutcome> {
        anyhow::ensure!(
            !self.buffers[buffer_idx].is_read_only(),
            "Buffer is read-only"
        );
        let Some(config) = self.get_formatter_for_buffer(buffer_idx).cloned() else {
            return Ok(match policy {
                FormatPolicy::ExternalThenLsp => BufferFormatOutcome::RequestLsp,
                FormatPolicy::ExternalOnly => BufferFormatOutcome::Skipped,
            });
        };
        let content = self.buffers[buffer_idx].content();
        let path = self.buffers[buffer_idx]
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(
            match crate::formatter::format_with_external(&content, &path, &config) {
                Ok(formatted) => {
                    let changed = self.apply_formatted_content(buffer_idx, &formatted)?;
                    BufferFormatOutcome::Applied {
                        command: config.command,
                        changed,
                    }
                }
                Err(error) => BufferFormatOutcome::Failed {
                    command: config.command,
                    error,
                },
            },
        )
    }

    fn apply_formatted_content(&mut self, idx: usize, content: &str) -> anyhow::Result<bool> {
        anyhow::ensure!(!self.buffers[idx].is_read_only(), "Buffer is read-only");
        if self.buffers[idx].content() == content {
            return Ok(false);
        }
        if idx == self.current_buffer_idx {
            self.replace_buffer_content_with_undo(content);
        } else {
            let dirty = self.buffers[idx].dirty;
            self.buffers[idx].set_content(content);
            self.reset_undo_stack_for_buffer(idx);
            self.mark_bufferline_if_dirty_changed(idx, dirty);
        }
        Ok(true)
    }

    pub(super) fn apply_format_edits(
        &mut self,
        idx: usize,
        edits: &[TextEdit],
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(!self.buffers[idx].is_read_only(), "Buffer is read-only");
        if edits.is_empty() {
            return Ok(false);
        }
        if idx == self.current_buffer_idx {
            let before = self.buffer().content();
            self.apply_text_edits(edits);
            return Ok(self.buffer().content() != before);
        }
        // Work on a scratch buffer so UTF-16 conversion and reverse edit ordering
        // do not borrow or switch the visible pane's cursor and undo history.
        let mut scratch = Buffer::new();
        scratch.set_content(&self.buffers[idx].content());
        let mut sorted: Vec<_> = edits.iter().enumerate().collect();
        sorted.sort_by_key(|(i, e)| {
            std::cmp::Reverse((e.start_line, e.start_col, e.end_line, e.end_col, *i))
        });
        for (_, edit) in sorted {
            let start = Self::lsp_utf16_col_to_buffer_col_in_buffer(
                &scratch,
                edit.start_line,
                edit.start_col,
            );
            let end =
                Self::lsp_utf16_col_to_buffer_col_in_buffer(&scratch, edit.end_line, edit.end_col);
            scratch.delete_range(edit.start_line, start, edit.end_line, end);
            scratch.insert_str(edit.start_line, start, &edit.new_text);
        }
        self.apply_formatted_content(idx, &scratch.content())
    }
}

#[cfg(test)]
mod tests {
    use crate::editor::save_test_support::Fixture;

    #[test]
    fn read_only_buffers_never_run_a_formatter() {
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        let marker = f.0.join("ran");
        f.formatter(
            &mut e,
            "sh",
            &[
                "-c",
                "touch \"$1\"; cat",
                "formatter",
                marker.to_str().unwrap(),
            ],
        );
        e.buffer_mut().set_read_only(true);
        let version = e.buffer().version();
        let _ = e.format_current_buffer();
        assert!(
            !marker.exists(),
            "read-only format must not launch the formatter"
        );
        assert_eq!(e.buffer().content(), "foo\n");
        assert_eq!(e.buffer().version(), version);
        e.buffer_mut().dirty = true;
        for backend in [true, false] {
            if !backend {
                e.languages_config = Default::default();
            }
            for policy in [
                super::FormatPolicy::ExternalThenLsp,
                super::FormatPolicy::ExternalOnly,
            ] {
                assert!(
                    e.format_buffer(e.current_buffer_idx, policy)
                        .unwrap_err()
                        .to_string()
                        .contains("read-only")
                );
                assert!(e.begin_format_current().is_err());
                for force in [false, true] {
                    let options = crate::editor::WriteOptions {
                        force,
                        format: Some(policy),
                    };
                    assert!(e.begin_save_current(options).is_err());
                    assert!(e.begin_save_all(options).is_err());
                    assert!(e.take_format_request().is_none());
                    e.discard_pending_saves();
                }
            }
        }
        assert_eq!(e.buffer().version(), version);
        assert_eq!(e.buffer().content(), "foo\n");
        assert!(e.buffer().dirty);
        assert_eq!(std::fs::read_to_string(f.0.join("a.md")).unwrap(), "foo\n");
        assert!(!marker.exists());
        e.open_virtual_read_only_buffer("report", "foo\n", Some("report.md"));
        for policy in [
            super::FormatPolicy::ExternalThenLsp,
            super::FormatPolicy::ExternalOnly,
        ] {
            assert!(e.format_buffer(e.current_buffer_idx, policy).is_err());
            assert!(
                e.begin_save_current(crate::editor::WriteOptions {
                    force: true,
                    format: Some(policy)
                })
                .is_err()
            );
            assert!(e.take_format_request().is_none());
        }
    }
    #[test]
    fn formatter_policy_prefers_external_without_error_fallback() {
        use super::{BufferFormatOutcome as O, FormatPolicy as P};
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        let i = e.current_buffer_idx;
        assert!(matches!(
            e.format_buffer(i, P::ExternalThenLsp).unwrap(),
            O::RequestLsp
        ));
        assert!(matches!(
            e.format_buffer(i, P::ExternalOnly).unwrap(),
            O::Skipped
        ));
        f.formatter(&mut e, "false", &[]);
        assert!(matches!(
            e.format_buffer(i, P::ExternalThenLsp).unwrap(),
            O::Failed { .. }
        ));
        f.formatter(&mut e, "sed", &["s/foo/bar/"]);
        assert!(matches!(
            e.format_buffer(i, P::ExternalOnly).unwrap(),
            O::Applied { changed: true, .. }
        ));
        assert_eq!(e.buffer().content(), "bar\n");
    }

    #[test]
    fn explicit_format_shares_resolver_without_saving() {
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        e.settings.editor.format_on_save = false;
        f.formatter(&mut e, "cat", &[]);
        let v = e.buffer().version();
        e.format_current_buffer().unwrap();
        assert_eq!(e.buffer().version(), v);
        assert!(!e.buffer().dirty);
        f.formatter(&mut e, "sed", &["s/foo/bar/"]);
        e.format_current_buffer().unwrap();
        assert_eq!(e.buffer().content(), "bar\n");
        assert_eq!(std::fs::read_to_string(f.0.join("a.md")).unwrap(), "foo\n");
        e.undo();
        assert_eq!(e.buffer().content(), "foo\n");
    }

    #[test]
    fn read_only_transition_blocks_pending_format_application() {
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        e.begin_save_current(crate::editor::WriteOptions {
            force: true,
            format: Some(super::FormatPolicy::ExternalThenLsp),
        })
        .unwrap();
        let request = e.take_format_request().unwrap();
        e.buffer_mut().set_read_only(true);
        let edit = crate::lsp::types::TextEdit {
            start_line: 0,
            start_col: 0,
            end_line: 0,
            end_col: 3,
            new_text: "bar".into(),
        };
        assert!(
            e.finish_format_request(request.token, Ok(vec![edit.clone()]))
                .is_err()
        );
        assert!(!e.format_request_is_active(request.token));
        assert!(e.apply_format_edits(e.current_buffer_idx, &[edit]).is_err());
        assert_eq!(e.buffer().content(), "foo\n");
        assert!(!e.buffer().dirty);
        e.open_virtual_read_only_buffer("report", "foo\n", Some("report.md"));
        assert!(e.format_current_buffer().is_err());
    }

    #[test]
    fn hidden_formatting_preserves_active_editor_state() {
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        let a = e.current_buffer_idx;
        e.replace_buffer_content("a😀b\n");
        e.open_file(f.file("b.md", "visible\n")).unwrap();
        e.replace_buffer_content_with_undo("edited visible\n");
        let active = e.current_buffer_idx;
        let cursor = e.cursor;
        let edit = crate::lsp::types::TextEdit {
            start_line: 0,
            start_col: 1,
            end_line: 0,
            end_col: 3,
            new_text: "Z".into(),
        };
        assert!(e.apply_format_edits(a, &[edit]).unwrap());
        assert_eq!(e.buffers[a].content(), "aZb\n");
        assert_eq!(e.buffer().content(), "edited visible\n");
        assert_eq!(e.current_buffer_idx, active);
        assert_eq!(e.cursor, cursor);
        let whole_document = crate::lsp::types::TextEdit {
            start_line: 0,
            start_col: 0,
            end_line: 1,
            end_col: 0,
            new_text: "without final newline".into(),
        };
        e.apply_format_edits(a, &[whole_document]).unwrap();
        assert_eq!(e.buffers[a].content(), "without final newline\n");
        f.formatter(&mut e, "sh", &["-c", "printf new"]);
        e.format_buffer(a, super::FormatPolicy::ExternalOnly)
            .unwrap();
        assert_eq!(e.buffers[a].content(), "new\n");
        assert_eq!(e.buffer().content(), "edited visible\n");
        assert_eq!(e.current_buffer_idx, active);
        assert_eq!(e.cursor, cursor);
        e.undo();
        assert_eq!(e.buffer().content(), "visible\n");
        e.open_file(f.0.join("a.md")).unwrap();
        e.undo();
        assert_eq!(e.buffer().content(), "new\n");
    }
}
