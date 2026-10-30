//! Event-loop integration for targeted, nonblocking format/save requests.
use nevi::editor::{FormatRequest, SaveProgress};
use nevi::lsp::types::TextEdit;
use nevi::{Editor, MultiLspManager};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const FORMAT_TIMEOUT: Duration = Duration::from_secs(5);

trait FormatTransport {
    fn has_server(&self, request: &FormatRequest) -> bool;
    fn prepare(&mut self, request: &FormatRequest) -> anyhow::Result<bool>;
    fn request(&mut self, request: &FormatRequest, tab_width: u32) -> anyhow::Result<()>;
    fn change(&mut self, path: &PathBuf, text: &str) -> anyhow::Result<()>;
    fn close(&mut self, path: &PathBuf) -> anyhow::Result<()>;
}
impl FormatTransport for MultiLspManager {
    fn has_server(&self, r: &FormatRequest) -> bool {
        self.has_server_for_file(&r.path, r.content.lines().next())
    }
    fn prepare(&mut self, r: &FormatRequest) -> anyhow::Result<bool> {
        self.open_format_target(&r.path, &r.content)
    }
    fn request(&mut self, r: &FormatRequest, tab: u32) -> anyhow::Result<()> {
        self.formatting(&r.path, tab, r.version, r.token)
    }
    fn change(&mut self, p: &PathBuf, text: &str) -> anyhow::Result<()> {
        self.did_change(p, text)
    }
    fn close(&mut self, p: &PathBuf) -> anyhow::Result<()> {
        self.did_close(p)
    }
}
struct Pending {
    request: FormatRequest,
    started: Instant,
    temporary: bool,
}
#[derive(Default)]
pub(super) struct SaveRuntime {
    pending: Option<Pending>,
}

impl SaveRuntime {
    pub(super) fn finish_session(&mut self, e: &Editor, transport: Option<&mut MultiLspManager>) {
        self.release(e, &mut transport.map(|t| t as &mut dyn FormatTransport));
    }
    pub(super) fn poll(
        &mut self,
        e: &mut Editor,
        transport: Option<&mut MultiLspManager>,
        now: Instant,
    ) -> bool {
        self.tick(e, transport.map(|t| t as &mut dyn FormatTransport), now)
    }
    fn tick(
        &mut self,
        e: &mut Editor,
        mut transport: Option<&mut dyn FormatTransport>,
        now: Instant,
    ) -> bool {
        let mut changed = false;
        if let Some(p) = self.pending.as_ref() {
            if !e.format_request_is_active(p.request.token) {
                self.release(e, &mut transport);
                changed = true;
            } else if now.saturating_duration_since(p.started) >= FORMAT_TIMEOUT {
                let token = p.request.token;
                let result = e.finish_format_request(token, Err("LSP formatting timed out".into()));
                report(e, result);
                self.release(e, &mut transport);
                changed = true;
            }
        }
        while self.pending.is_none() {
            let Some(request) = e.take_format_request() else {
                break;
            };
            changed = true;
            let mut temporary = false;
            let tab_width = e.settings.editor.tab_width as u32;
            // Ok(false): no server handles this file at all, which is the same
            // as having no formatter. A server that is still starting is a
            // real miss and keeps its "LSP not available" note.
            let sent =
                e.validate_format_request(&request)
                    .and_then(|()| match transport.as_mut() {
                        Some(t) if t.has_server(&request) => {
                            temporary = t.prepare(&request)?;
                            t.request(&request, tab_width).map(|()| true)
                        }
                        _ => Ok(false),
                    });
            self.pending = Some(Pending {
                request: request.clone(),
                started: now,
                temporary,
            });
            let result = match sent {
                Ok(true) => continue,
                Ok(false) => e.skip_format_request(request.token),
                Err(error) => e.finish_format_request(request.token, Err(error.to_string())),
            };
            report(e, result);
            self.release(e, &mut transport);
        }
        changed
    }
    pub(super) fn handle_response(
        &mut self,
        e: &mut Editor,
        t: &mut MultiLspManager,
        token: u64,
        uri: &str,
        version: u64,
        result: Result<Vec<TextEdit>, String>,
    ) {
        self.complete(e, t, token, uri, version, result);
    }
    fn complete(
        &mut self,
        e: &mut Editor,
        t: &mut dyn FormatTransport,
        token: u64,
        uri: &str,
        version: u64,
        result: Result<Vec<TextEdit>, String>,
    ) {
        let Some(p) = self.pending.as_ref() else {
            return;
        };
        if p.request.token != token
            || nevi::lsp::path_to_uri(&p.request.path) != uri
            || p.request.version != version
        {
            return;
        }
        if !e.format_request_is_active(token) {
            self.release(e, &mut Some(t));
            return;
        }
        let request = p.request.clone();
        let document_open = p.temporary || e.buffer().path.as_ref() == Some(&request.path);
        let before = e
            .buffers()
            .iter()
            .find(|b| b.id() == request.buffer_id)
            .map(|b| b.version());
        let result = e.finish_format_request(token, result);
        if let Some(b) = e
            .buffers()
            .iter()
            .find(|b| b.id() == request.buffer_id && b.path.as_ref() == Some(&request.path))
        {
            // A current target switched away from receives the ordinary didClose.
            // Its late result may still save, but must not reopen it via didChange.
            if document_open && Some(b.version()) != before {
                let _ = t.change(&request.path, &b.content());
            }
        }
        report(e, result);
        self.release(e, &mut Some(t));
    }
    fn release(&mut self, e: &Editor, transport: &mut Option<&mut dyn FormatTransport>) {
        if let Some(p) = self.pending.take() {
            if p.temporary && e.buffer().path.as_ref() != Some(&p.request.path) {
                if let Some(t) = transport {
                    let _ = t.close(&p.request.path);
                }
            }
        }
    }
    // A temporary hidden document becoming current must not change the normal
    // switch lifecycle. Release our open before the existing didOpen executes.
    pub(super) fn before_current_open(&mut self, path: &PathBuf, t: &mut MultiLspManager) {
        self.release_temporary_open(path, t);
    }
    fn release_temporary_open(&mut self, path: &PathBuf, t: &mut dyn FormatTransport) {
        if let Some(p) = self.pending.as_mut() {
            if p.temporary && &p.request.path == path {
                let _ = t.close(path);
                p.temporary = false;
            }
        }
    }
}
fn report(e: &mut Editor, result: anyhow::Result<SaveProgress>) {
    match result {
        Ok(progress) => e.set_status(e.save_progress_message(progress)),
        Err(error) => e.set_status(format!("Error saving/formatting: {error}")),
    }
}
/// The existing background-write eligibility and raw-save path.
pub(super) fn try_autosave(e: &mut Editor, pending: &mut Option<Instant>, now: Instant) -> bool {
    if !pending.is_some_and(|deadline| now >= deadline) {
        return false;
    }
    *pending = None;
    let modal = e.mode == nevi::Mode::Command || e.finder.populated || e.explorer.visible;
    if modal || !e.has_unsaved_changes() || e.buffer().path.is_none() {
        return false;
    }
    match e.save() {
        Ok(()) => e.set_status("Autosaved"),
        Err(error) => e.set_status(format!("Autosave failed: {error}")),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use nevi::editor::{FormatPolicy, WriteOptions};
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "nevi-runtime-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn editor(&self) -> Editor {
            let mut e = Editor::default();
            e.languages_config = Default::default();
            let p = self.0.join("a.md");
            std::fs::write(&p, "old\n").unwrap();
            e.open_file(p).unwrap();
            e.replace_buffer_content("foo\n");
            e
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn options(force: bool) -> WriteOptions {
        WriteOptions {
            force,
            format: Some(FormatPolicy::ExternalThenLsp),
        }
    }
    fn edits() -> Vec<TextEdit> {
        vec![TextEdit {
            start_line: 0,
            start_col: 0,
            end_line: 0,
            end_col: 3,
            new_text: "bar".into(),
        }]
    }
    #[derive(Default)]
    struct Transport {
        events: Vec<String>,
        open_documents: std::collections::HashSet<PathBuf>,
        unavailable: bool,
        no_server: bool,
        fail: bool,
    }
    impl FormatTransport for Transport {
        fn has_server(&self, _r: &FormatRequest) -> bool {
            !self.no_server
        }
        fn prepare(&mut self, r: &FormatRequest) -> anyhow::Result<bool> {
            if self.unavailable {
                anyhow::bail!("LSP not available");
            }
            if self.open_documents.contains(&r.path) {
                self.change(&r.path, &r.content)?;
                return Ok(false);
            }
            self.open_documents.insert(r.path.clone());
            self.events.push(format!(
                "open:{}:{}",
                r.path.file_name().unwrap().to_string_lossy(),
                r.content.trim()
            ));
            Ok(true)
        }
        fn request(&mut self, r: &FormatRequest, _tab: u32) -> anyhow::Result<()> {
            self.events.push(format!("format:{}", r.token));
            if self.fail {
                anyhow::bail!("dispatch failed");
            }
            Ok(())
        }
        fn change(&mut self, p: &PathBuf, _text: &str) -> anyhow::Result<()> {
            assert!(
                self.open_documents.contains(p),
                "didChange requires an open document"
            );
            self.events.push(format!("change:{}", p.display()));
            Ok(())
        }
        fn close(&mut self, p: &PathBuf) -> anyhow::Result<()> {
            self.open_documents.remove(p);
            self.events.push(format!(
                "close:{}",
                p.file_name().unwrap().to_string_lossy()
            ));
            Ok(())
        }
    }
    #[test]
    fn lsp_save_completes_for_original_buffer_after_focus_change() {
        let f = Fixture::new();
        let mut e = f.editor();
        let mut rt = SaveRuntime::default();
        let mut t = Transport::default();
        let now = Instant::now();
        e.begin_save_current(options(false)).unwrap();
        rt.tick(&mut e, Some(&mut t), now);
        let r = rt.pending.as_ref().unwrap().request.clone();
        let b = f.0.join("b.md");
        std::fs::write(&b, "visible\n").unwrap();
        e.open_file(b).unwrap();
        let cursor = e.cursor;
        rt.complete(
            &mut e,
            &mut t,
            r.token,
            &nevi::lsp::path_to_uri(&r.path),
            r.version,
            Ok(edits()),
        );
        assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "bar\n");
        assert_eq!(e.buffer().content(), "visible\n");
        assert_eq!(e.cursor, cursor);
        assert!(!e.has_pending_saves());
        assert!(rt.pending.is_none());
    }
    #[test]
    fn closed_document_is_not_changed_after_format_completion() {
        for promote in [false, true] {
            let f = Fixture::new();
            let mut e = f.editor();
            let a = e.buffer().path.clone().unwrap();
            let mut rt = SaveRuntime::default();
            let mut t = Transport::default();
            // The ordinary current-buffer didOpen already owns A.
            t.open_documents.insert(a.clone());
            e.begin_format_current().unwrap();
            rt.tick(&mut e, Some(&mut t), Instant::now());
            let r = rt.pending.as_ref().unwrap().request.clone();
            assert!(!rt.pending.as_ref().unwrap().temporary);
            let b = f.0.join("b.md");
            std::fs::write(&b, "visible\n").unwrap();
            e.open_file(b.clone()).unwrap();
            t.close(&a).unwrap();
            t.open_documents.insert(b);
            t.events.clear();
            rt.complete(
                &mut e,
                &mut t,
                r.token,
                &nevi::lsp::path_to_uri(&a),
                r.version,
                Ok(edits()),
            );
            assert!(t.events.is_empty(), "closed A must not receive didChange");
            assert!(!t.open_documents.contains(&a));
            assert_eq!(std::fs::read_to_string(&a).unwrap(), "old\n");
            // A remains dirty after format-only, so :wa must temporarily reopen it.
            e.begin_save_all(options(false)).unwrap();
            rt.tick(&mut e, Some(&mut t), Instant::now());
            assert_eq!(t.events[0], "open:a.md:bar");
            let r = rt.pending.as_ref().unwrap().request.clone();
            assert!(rt.pending.as_ref().unwrap().temporary);
            if promote {
                t.close(e.buffer().path.as_ref().unwrap()).unwrap();
                rt.release_temporary_open(&a, &mut t);
                assert!(!t.open_documents.contains(&a));
                assert!(!rt.pending.as_ref().unwrap().temporary);
                e.open_file(a.clone()).unwrap();
                t.open_documents.insert(a.clone());
            }
            rt.complete(
                &mut e,
                &mut t,
                r.token,
                &nevi::lsp::path_to_uri(&a),
                r.version,
                Ok(vec![]),
            );
            assert_eq!(std::fs::read_to_string(&a).unwrap(), "bar\n");
            assert_eq!(t.open_documents.contains(&a), promote);
        }
    }
    #[test]
    fn lsp_failure_paths_still_save_without_formatting() {
        for mode in 0..6 {
            let f = Fixture::new();
            let mut e = f.editor();
            let mut rt = SaveRuntime::default();
            let mut t = Transport {
                unavailable: mode == 1,
                fail: mode == 2,
                ..Default::default()
            };
            let now = Instant::now();
            e.begin_save_current(options(false)).unwrap();
            if mode == 0 {
                rt.tick(&mut e, None, now);
            } else {
                rt.tick(&mut e, Some(&mut t), now);
            }
            if let Some(p) = &rt.pending {
                let r = p.request.clone();
                if mode == 5 {
                    rt.tick(&mut e, Some(&mut t), now + Duration::from_secs(6));
                } else {
                    rt.complete(
                        &mut e,
                        &mut t,
                        r.token,
                        &nevi::lsp::path_to_uri(&r.path),
                        r.version,
                        if mode == 3 {
                            Err("unsupported or malformed".into())
                        } else {
                            Ok(vec![])
                        },
                    );
                }
            }
            assert_eq!(
                std::fs::read_to_string(f.0.join("a.md")).unwrap(),
                "foo\n",
                "mode {mode}"
            );
            assert!(!e.has_pending_saves());
            assert!(rt.pending.is_none());
        }
        for mode in 0..3 {
            let f = Fixture::new();
            let mut e = f.editor();
            let mut rt = SaveRuntime::default();
            let mut t = Transport::default();
            let now = Instant::now();
            e.begin_format_current().unwrap();
            rt.tick(&mut e, Some(&mut t), now);
            let r = rt.pending.as_ref().unwrap().request.clone();
            if mode == 0 {
                rt.tick(&mut e, Some(&mut t), now + FORMAT_TIMEOUT);
            } else {
                if mode == 1 {
                    e.replace_buffer_content("newer\n");
                }
                rt.complete(
                    &mut e,
                    &mut t,
                    r.token,
                    &nevi::lsp::path_to_uri(&r.path),
                    r.version,
                    if mode == 1 {
                        Ok(edits())
                    } else {
                        Err("format failed".into())
                    },
                );
            }
            assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "old\n");
            assert!(!e.status_message.as_ref().unwrap().contains("saved"));
            assert!(!e.has_pending_saves());
            assert!(rt.pending.is_none());
        }
    }
    // A file no server handles (markdown without marksman, plain text, LSP
    // disabled) is the same as having no formatter: write it, no warning.
    #[test]
    fn saves_without_a_language_server_skip_formatting_quietly() {
        for (all, no_transport) in [(false, false), (false, true), (true, false), (true, true)] {
            let f = Fixture::new();
            let mut e = f.editor();
            let mut rt = SaveRuntime::default();
            let mut t = Transport {
                no_server: true,
                ..Default::default()
            };
            if all {
                e.begin_save_all(options(false)).unwrap();
            } else {
                e.begin_save_current(options(false)).unwrap();
            }
            rt.tick(&mut e, (!no_transport).then_some(&mut t), Instant::now());
            assert_eq!(std::fs::read_to_string(f.0.join("a.md")).unwrap(), "foo\n");
            assert!(t.events.is_empty(), "no didOpen or format request");
            assert!(!e.has_pending_saves());
            assert!(rt.pending.is_none());
            let status = e.status_message.clone().unwrap();
            let expected = if all {
                "Saved 1 buffer"
            } else {
                "\"a.md\" written"
            };
            assert_eq!(status, expected, "all={all} no_transport={no_transport}");
        }
        // A server that is still starting is a real miss and says so.
        let f = Fixture::new();
        let mut e = f.editor();
        let mut t = Transport {
            unavailable: true,
            ..Default::default()
        };
        e.begin_save_current(options(false)).unwrap();
        SaveRuntime::default().tick(&mut e, Some(&mut t), Instant::now());
        assert_eq!(std::fs::read_to_string(f.0.join("a.md")).unwrap(), "foo\n");
        assert!(e.status_message.unwrap().ends_with("(LSP not available)"));
        // An explicit :Format still reports that nothing can format the file.
        let f = Fixture::new();
        let mut e = f.editor();
        e.begin_format_current().unwrap();
        SaveRuntime::default().tick(&mut e, None, Instant::now());
        assert_eq!(std::fs::read_to_string(f.0.join("a.md")).unwrap(), "old\n");
        assert!(e.status_message.unwrap().contains("LSP not available"));
    }
    #[test]
    fn write_all_synchronizes_hidden_documents_before_formatting() {
        let f = Fixture::new();
        let mut e = f.editor();
        let b = f.0.join("b.md");
        std::fs::write(&b, "old\n").unwrap();
        e.open_file(b).unwrap();
        e.replace_buffer_content("foo\n");
        let active = e.buffer().id();
        e.begin_save_all(options(false)).unwrap();
        let mut rt = SaveRuntime::default();
        let mut t = Transport::default();
        for _ in 0..2 {
            rt.tick(&mut e, Some(&mut t), Instant::now());
            let r = rt.pending.as_ref().unwrap().request.clone();
            rt.complete(
                &mut e,
                &mut t,
                r.token,
                &nevi::lsp::path_to_uri(&r.path),
                r.version,
                Ok(edits()),
            );
        }
        assert_eq!(e.buffer().id(), active);
        assert!(!e.has_pending_saves());
        assert_eq!(
            &t.events[0..2],
            &["open:a.md:foo".to_string(), "format:1".to_string()]
        );
        assert!(t.events.iter().any(|s| s == "close:a.md"));
        assert!(t.events.iter().any(|s| s == "open:b.md:foo"));
        assert_eq!(std::fs::read_to_string(f.0.join("a.md")).unwrap(), "bar\n");
        assert_eq!(std::fs::read_to_string(f.0.join("b.md")).unwrap(), "bar\n");
        for ending in 0..4 {
            let f = Fixture::new();
            let mut e = f.editor();
            let a = e.buffer().path.clone().unwrap();
            let visible = f.0.join("visible.md");
            std::fs::write(&visible, "visible\n").unwrap();
            e.open_file(visible).unwrap();
            e.begin_save_all(options(false)).unwrap();
            let mut rt = SaveRuntime::default();
            let mut t = Transport {
                fail: ending == 3,
                ..Default::default()
            };
            let now = Instant::now();
            rt.tick(&mut e, Some(&mut t), now);
            match ending {
                0 => {
                    rt.tick(&mut e, Some(&mut t), now + FORMAT_TIMEOUT);
                }
                1 => {
                    e.flush_pending_saves().unwrap();
                    rt.tick(&mut e, Some(&mut t), now);
                }
                2 => {
                    e.discard_pending_saves();
                    rt.tick(&mut e, Some(&mut t), now);
                }
                _ => {}
            }
            assert!(rt.pending.is_none());
            assert!(!t.open_documents.contains(&a));
            assert!(t.events.iter().any(|event| event == "close:a.md"));
            assert_eq!(
                std::fs::read_to_string(a).unwrap(),
                if ending == 2 { "old\n" } else { "foo\n" }
            );
        }
    }
    #[test]
    fn write_all_handles_stale_closed_and_late_responses() {
        let f = Fixture::new();
        let mut e = f.editor();
        let mut rt = SaveRuntime::default();
        let mut t = Transport::default();
        e.begin_save_all(options(false)).unwrap();
        rt.tick(&mut e, Some(&mut t), Instant::now());
        let r = rt.pending.as_ref().unwrap().request.clone();
        rt.complete(
            &mut e,
            &mut t,
            r.token,
            "file:///wrong",
            r.version,
            Ok(edits()),
        );
        assert!(e.has_pending_saves());
        rt.complete(
            &mut e,
            &mut t,
            r.token + 99,
            &nevi::lsp::path_to_uri(&r.path),
            r.version,
            Ok(edits()),
        );
        assert!(e.has_pending_saves());
        e.replace_buffer_content("newer\n");
        rt.complete(
            &mut e,
            &mut t,
            r.token,
            &nevi::lsp::path_to_uri(&r.path),
            r.version,
            Ok(edits()),
        );
        assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "newer\n");
        rt.complete(
            &mut e,
            &mut t,
            r.token,
            &nevi::lsp::path_to_uri(&r.path),
            r.version,
            Ok(edits()),
        );
        assert_eq!(e.buffer().content(), "newer\n");
        let f = Fixture::new();
        let mut e = f.editor();
        let mut rt = SaveRuntime::default();
        let mut t = Transport::default();
        e.begin_save_current(options(false)).unwrap();
        rt.tick(&mut e, Some(&mut t), Instant::now());
        let r = rt.pending.as_ref().unwrap().request.clone();
        let renamed = f.0.join("renamed.md");
        e.buffer_mut().set_file_path(renamed.clone());
        rt.complete(
            &mut e,
            &mut t,
            r.token,
            &nevi::lsp::path_to_uri(&r.path),
            r.version,
            Ok(edits()),
        );
        assert_eq!(e.buffer().content(), "foo\n");
        assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "old\n");
        assert!(!renamed.exists());
        assert!(e.status_message.as_ref().unwrap().contains("renamed"));
        assert!(!e.has_pending_saves());
        assert!(!e.format_request_is_active(r.token));
    }
    #[test]
    fn force_intent_survives_lsp_success_failure_and_timeout() {
        for force in [false, true] {
            for mode in 0..5 {
                let f = Fixture::new();
                let mut e = f.editor();
                let mut rt = SaveRuntime::default();
                let mut t = Transport::default();
                let now = Instant::now();
                e.begin_save_current(options(force)).unwrap();
                rt.tick(&mut e, Some(&mut t), now);
                let r = rt.pending.as_ref().unwrap().request.clone();
                std::fs::write(&r.path, "external changes\n").unwrap();
                if mode == 2 {
                    rt.tick(&mut e, Some(&mut t), now + Duration::from_secs(6));
                } else if mode == 3 {
                    let result = e.flush_pending_saves();
                    assert_eq!(result.is_ok(), force);
                    rt.tick(&mut e, Some(&mut t), now);
                } else {
                    rt.complete(
                        &mut e,
                        &mut t,
                        r.token,
                        &nevi::lsp::path_to_uri(&r.path),
                        r.version,
                        if mode == 4 {
                            Ok(vec![])
                        } else if mode == 0 {
                            Ok(edits())
                        } else {
                            Err("failed".into())
                        },
                    );
                }
                assert_eq!(
                    std::fs::read_to_string(&r.path).unwrap(),
                    if !force {
                        "external changes\n"
                    } else if mode == 0 {
                        "bar\n"
                    } else {
                        "foo\n"
                    }
                );
            }
        }
    }
    fn command(e: &mut Editor, text: &str) {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        if text == "Ctrl-S" {
            nevi::terminal::handle_key(e, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
            return;
        }
        if text == "Ctrl-W q" {
            nevi::terminal::handle_key(e, KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
            nevi::terminal::handle_key(e, KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
            return;
        }
        if text == "ZZ" {
            for _ in 0..2 {
                nevi::terminal::handle_key(
                    e,
                    KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT),
                );
            }
            return;
        }
        for c in format!(":{text}").chars() {
            nevi::terminal::handle_key(e, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        nevi::terminal::handle_key(e, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }
    fn mixed_batch(f: &Fixture) -> Editor {
        let mut e = f.editor();
        e.settings.editor.format_on_save = true;
        for name in ["b.md", "c.rs"] {
            let p = f.0.join(name);
            std::fs::write(&p, "old\n").unwrap();
            e.open_file(p).unwrap();
            e.replace_buffer_content("foo\n");
        }
        e.begin_save_all(options(false)).unwrap();
        e.languages_config.languages.insert(
            "markdown".into(),
            nevi::config::LanguageConfig {
                formatter: Some(nevi::config::FormatterConfig {
                    command: "sed".into(),
                    args: vec!["s/foo/bar/".into()],
                    timeout: 1,
                }),
                tab_width: None,
            },
        );
        e
    }
    #[test]
    fn closed_pending_target_does_not_block_later_writes() {
        for reply_first in [false, true] {
            let f = Fixture::new();
            let mut e = f.editor();
            e.settings.editor.format_on_save = true;
            let a = e.buffer().path.clone().unwrap();
            let b = f.0.join("b.md");
            std::fs::write(&b, "old\n").unwrap();
            e.open_file(b.clone()).unwrap();
            e.replace_buffer_content("second\n");
            e.open_file(a.clone()).unwrap();
            command(&mut e, "w");
            let mut rt = SaveRuntime::default();
            let mut t = Transport::default();
            rt.tick(&mut e, Some(&mut t), Instant::now());
            let request = rt.pending.as_ref().unwrap().request.clone();
            command(&mut e, &format!("bd! {}", a.display()));
            assert!(
                !e.buffers()
                    .iter()
                    .any(|buffer| buffer.id() == request.buffer_id)
            );
            e.open_file(b.clone()).unwrap();
            if reply_first {
                rt.complete(
                    &mut e,
                    &mut t,
                    request.token,
                    &nevi::lsp::path_to_uri(&a),
                    request.version,
                    Ok(edits()),
                );
                assert!(
                    e.status_message
                        .as_ref()
                        .unwrap()
                        .contains("closed, replaced, or renamed")
                );
                assert!(!e.has_pending_saves());
            }
            e.settings.editor.format_on_save = false;
            command(&mut e, "w");
            assert_eq!(std::fs::read_to_string(&b).unwrap(), "second\n");
            assert!(!e.has_pending_saves());
            if !reply_first {
                let status = e.status_message.as_ref().unwrap();
                assert!(status.contains("closed, replaced, or renamed"));
                assert!(status.contains("written"));
            }
            rt.complete(
                &mut e,
                &mut t,
                request.token,
                &nevi::lsp::path_to_uri(&a),
                request.version,
                Ok(edits()),
            );
            assert_eq!(std::fs::read_to_string(&a).unwrap(), "old\n");
            assert_eq!(std::fs::read_to_string(&b).unwrap(), "second\n");
        }
    }
    #[test]
    fn lsp_write_conflict_allows_force_write_retry() {
        let f = Fixture::new();
        let mut e = f.editor();
        e.settings.editor.format_on_save = true;
        command(&mut e, "w");
        let mut rt = SaveRuntime::default();
        let mut t = Transport::default();
        rt.tick(&mut e, Some(&mut t), Instant::now());
        let request = rt.pending.as_ref().unwrap().request.clone();
        std::fs::write(&request.path, "external\n").unwrap();
        rt.complete(
            &mut e,
            &mut t,
            request.token,
            &nevi::lsp::path_to_uri(&request.path),
            request.version,
            Ok(edits()),
        );
        assert!(e.status_message.as_ref().unwrap().contains("use :w!"));
        assert!(e.buffer().dirty);
        assert!(!e.has_pending_saves());
        command(&mut e, "w!");
        rt.tick(&mut e, Some(&mut t), Instant::now());
        let retry = rt.pending.as_ref().unwrap().request.clone();
        assert_ne!(retry.token, request.token);
        rt.complete(
            &mut e,
            &mut t,
            retry.token,
            &nevi::lsp::path_to_uri(&retry.path),
            retry.version,
            Ok(edits()),
        );
        assert_eq!(std::fs::read_to_string(&retry.path).unwrap(), "bar\n");
        assert!(!e.has_pending_saves());
    }
    #[test]
    fn failed_flush_runs_new_command_and_keeps_both_results() {
        for next in [
            "w",
            "w!",
            "Format",
            "w dest.md",
            "w! dest.md",
            "wq",
            "x",
            "wa",
            "wqa",
            "xa",
        ] {
            let f = Fixture::new();
            let mut e = f.editor();
            let a = e.buffer().path.clone().unwrap();
            e.begin_save_current(options(false)).unwrap();
            let old = e.take_format_request().unwrap();
            std::fs::write(&a, "external\n").unwrap();
            let b = f.0.join("b.md");
            std::fs::write(&b, "old\n").unwrap();
            e.open_file(b.clone()).unwrap();
            e.replace_buffer_content("foo\n");
            e.settings.editor.format_on_save = false;
            e.languages_config.languages.insert(
                "markdown".into(),
                nevi::config::LanguageConfig {
                    formatter: Some(nevi::config::FormatterConfig {
                        command: "sed".into(),
                        args: vec!["s/foo/bar/".into()],
                        timeout: 1,
                    }),
                    tab_width: None,
                },
            );
            let next_command = next.replace("dest.md", f.0.join("dest.md").to_str().unwrap());
            command(&mut e, &next_command);
            let status = e.status_message.as_deref().unwrap();
            assert!(
                status.contains("Previous write failed:"),
                "{next}: {status}"
            );
            assert!(status.contains("changed on disk"), "{next}: {status}");
            if next == "Format" {
                assert!(status.contains("Formatted"), "{status}");
                assert_eq!(e.buffer().content(), "bar\n");
                assert_eq!(std::fs::read_to_string(&b).unwrap(), "old\n");
            } else if next.contains("dest.md") {
                assert!(status.contains("written"), "{status}");
                assert_eq!(
                    std::fs::read_to_string(f.0.join("dest.md")).unwrap(),
                    "foo\n"
                );
            } else if ["wa", "wqa", "xa"].contains(&next) {
                assert!(status.contains("Error saving:"), "{status}");
                assert_eq!(std::fs::read_to_string(&b).unwrap(), "old\n");
            } else {
                assert_eq!(std::fs::read_to_string(&b).unwrap(), "foo\n");
                assert!(
                    status.contains("written") || status.contains("No write since last change"),
                    "{status}"
                );
            }
            assert!(!e.should_quit, "A remains dirty: {next}");
            assert!(!e.has_pending_saves());
            assert!(!e.format_request_is_active(old.token));
        }

        // An asynchronous replacement command keeps the flush error through
        // its eventual success or failure, rather than only while scheduling.
        for failed in [false, true] {
            let f = Fixture::new();
            let mut e = f.editor();
            e.settings.editor.format_on_save = true;
            command(&mut e, "w");
            let mut rt = SaveRuntime::default();
            let mut t = Transport::default();
            rt.tick(&mut e, Some(&mut t), Instant::now());
            let a = e.buffer().path.clone().unwrap();
            std::fs::write(&a, "external A\n").unwrap();
            let b = f.0.join("b.md");
            std::fs::write(&b, "old\n").unwrap();
            e.open_file(b.clone()).unwrap();
            e.replace_buffer_content("foo\n");
            command(&mut e, "w");
            assert!(
                e.status_message
                    .as_deref()
                    .unwrap()
                    .contains("Previous write failed:")
            );
            assert!(
                e.status_message
                    .as_deref()
                    .unwrap()
                    .contains("Formatting with LSP")
            );
            rt.tick(&mut e, Some(&mut t), Instant::now());
            let r = rt.pending.as_ref().unwrap().request.clone();
            if failed {
                e.buffer_mut().set_read_only(true);
            }
            rt.complete(
                &mut e,
                &mut t,
                r.token,
                &nevi::lsp::path_to_uri(&b),
                r.version,
                Ok(vec![]),
            );
            let status = e.status_message.as_deref().unwrap();
            assert!(status.contains("Previous write failed:"), "{status}");
            assert!(
                status.contains(if failed {
                    "read-only"
                } else {
                    "\"b.md\" written"
                }),
                "{status}"
            );
            assert!(!e.has_pending_saves());
            assert_eq!(
                std::fs::read_to_string(&b).unwrap(),
                if failed { "old\n" } else { "foo\n" }
            );
        }
    }
    #[test]
    fn new_save_or_format_flushes_the_previous_transaction() {
        for new in [
            "w",
            "w!",
            "wa",
            "Ctrl-S",
            "Format",
            "w dest.md",
            "w! dest.md",
        ] {
            let f = Fixture::new();
            let mut e = mixed_batch(&f);
            let mut rt = SaveRuntime::default();
            let mut t = Transport::default();
            rt.tick(&mut e, Some(&mut t), Instant::now());
            let old = rt.pending.as_ref().unwrap().request.clone();
            let new = if new.contains("dest.md") {
                new.replace("dest.md", f.0.join("dest.md").to_str().unwrap())
            } else {
                new.into()
            };
            command(&mut e, &new);
            assert_eq!(
                std::fs::read_to_string(f.0.join("a.md")).unwrap(),
                "foo\n",
                "{new}"
            );
            assert_eq!(std::fs::read_to_string(f.0.join("b.md")).unwrap(), "bar\n");
            assert_eq!(std::fs::read_to_string(f.0.join("c.rs")).unwrap(), "foo\n");
            assert_eq!(
                t.events.iter().filter(|x| x.starts_with("format:")).count(),
                1,
                "flush cannot dispatch LSP"
            );
            rt.complete(
                &mut e,
                &mut t,
                old.token,
                &nevi::lsp::path_to_uri(&old.path),
                old.version,
                Ok(edits()),
            );
            assert_eq!(std::fs::read_to_string(&old.path).unwrap(), "foo\n");
            if new == "Format" {
                rt.tick(&mut e, Some(&mut t), Instant::now());
                let r = rt.pending.as_ref().unwrap().request.clone();
                rt.complete(
                    &mut e,
                    &mut t,
                    r.token,
                    &nevi::lsp::path_to_uri(&r.path),
                    r.version,
                    Ok(edits()),
                );
                assert_eq!(e.buffer().content(), "bar\n");
                assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "foo\n");
            }
        }
        let f = Fixture::new();
        let mut e = f.editor();
        e.begin_format_current().unwrap();
        let old = e.take_format_request().unwrap();
        e.begin_format_current().unwrap();
        assert_eq!(std::fs::read_to_string(&old.path).unwrap(), "old\n");
        let f = Fixture::new();
        let mut e = mixed_batch(&f);
        e.languages_config
            .languages
            .get_mut("markdown")
            .unwrap()
            .formatter
            .as_mut()
            .unwrap()
            .command = "false".into();
        let summary = e.flush_pending_saves().unwrap();
        assert_eq!(summary.saved, 3);
        assert!(!summary.warnings.is_empty());
        for name in ["a.md", "b.md", "c.rs"] {
            assert_eq!(std::fs::read_to_string(f.0.join(name)).unwrap(), "foo\n");
        }
    }
    #[test]
    fn flush_write_failure_ends_transaction_and_preserves_dirty_buffers() {
        let f = Fixture::new();
        let mut e = mixed_batch(&f);
        let old = e.take_format_request().unwrap();
        std::fs::write(&old.path, "external conflict\n").unwrap();
        command(&mut e, "wqa");
        assert!(!e.should_quit);
        assert!(!e.has_pending_saves());
        assert!(
            e.status_message
                .as_ref()
                .unwrap()
                .contains("Previous write failed")
        );
        assert!(
            e.status_message
                .as_ref()
                .unwrap()
                .contains("changed on disk")
        );
        assert_eq!(std::fs::read_to_string(f.0.join("b.md")).unwrap(), "old\n");
        command(&mut e, "qa!");
        assert!(e.should_quit);
        assert!(!e.has_pending_saves());
        let f = Fixture::new();
        let mut e = f.editor();
        e.begin_save_current(options(false)).unwrap();
        let r = e.take_format_request().unwrap();
        e.buffer_mut().set_read_only(true);
        assert!(e.flush_pending_saves().is_err());
        assert!(!e.has_pending_saves());
        assert!(!e.format_request_is_active(r.token));
        assert!(e.buffer().dirty);
        e.buffer_mut().set_read_only(false);
        e.begin_save_current(WriteOptions {
            force: false,
            format: None,
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "foo\n");
        let f = Fixture::new();
        let mut e = f.editor();
        let parent = f.0.join("blocked");
        std::fs::create_dir(&parent).unwrap();
        let b = parent.join("b.md");
        let c = f.0.join("c.md");
        for path in [&b, &c] {
            std::fs::write(path, "old\n").unwrap();
            e.open_file(path.clone()).unwrap();
            e.replace_buffer_content("foo\n");
        }
        e.begin_save_all(options(false)).unwrap();
        std::fs::remove_file(&b).unwrap();
        std::fs::remove_dir(&parent).unwrap();
        std::fs::write(&parent, "not a directory").unwrap();
        let error = e.flush_pending_saves().unwrap_err();
        assert!(!error.to_string().contains("changed on disk"));
        assert!(!e.has_pending_saves());
        assert!(e.take_format_request().is_none());
        assert_eq!(std::fs::read_to_string(f.0.join("a.md")).unwrap(), "foo\n");
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "old\n");
        assert!(
            e.buffers()
                .iter()
                .find(|buffer| buffer.path.as_ref() == Some(&b))
                .unwrap()
                .dirty
        );
        assert!(e.buffer().dirty);
        std::fs::remove_file(&parent).unwrap();
        std::fs::create_dir(&parent).unwrap();
        let SaveProgress::Complete(summary) = e
            .begin_save_all(WriteOptions {
                force: false,
                format: None,
            })
            .unwrap()
        else {
            panic!("raw writes finish synchronously");
        };
        assert_eq!(summary.saved, 2);
        assert!(!e.has_pending_saves());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "foo\n");
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "foo\n");
    }
    #[test]
    fn write_quit_flushes_pending_saves_without_waiting_for_lsp() {
        for cmd in ["wq", "x", "ZZ", "wqa", "wqall", "xall", "xa"] {
            let f = Fixture::new();
            let mut e = mixed_batch(&f);
            let mut rt = SaveRuntime::default();
            let mut t = Transport::default();
            rt.tick(&mut e, Some(&mut t), Instant::now());
            let r = rt.pending.as_ref().unwrap().request.clone();
            command(&mut e, cmd);
            assert!(e.should_quit, "{cmd}");
            assert!(!e.has_pending_saves());
            assert!(e.take_format_request().is_none());
            assert_eq!(std::fs::read_to_string(f.0.join("a.md")).unwrap(), "foo\n");
            assert_eq!(std::fs::read_to_string(f.0.join("b.md")).unwrap(), "bar\n");
            assert_eq!(std::fs::read_to_string(f.0.join("c.rs")).unwrap(), "foo\n");
            rt.complete(
                &mut e,
                &mut t,
                r.token,
                &nevi::lsp::path_to_uri(&r.path),
                r.version,
                Ok(edits()),
            );
            assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "foo\n");
        }
    }
    #[test]
    fn only_discard_quits_abandon_pending_saves() {
        for cmd in [
            "q!", "quit!", "qa!", "qall!", "q", "quit", "qa", "qall", "Ctrl-W q",
        ] {
            let f = Fixture::new();
            let mut e = mixed_batch(&f);
            let r = e.take_format_request().unwrap();
            command(&mut e, cmd);
            for name in ["a.md", "b.md", "c.rs"] {
                assert_eq!(std::fs::read_to_string(f.0.join(name)).unwrap(), "old\n");
            }
            if cmd.ends_with('!') {
                assert!(!e.has_pending_saves());
                e.finish_format_request(r.token, Ok(edits())).unwrap();
                assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "old\n");
            } else {
                assert!(!e.should_quit);
                assert!(e.has_pending_saves());
                e.flush_pending_saves().unwrap();
                assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "foo\n");
            }
        }
        // Closing a split is not an exit, so it must not drop requested writes.
        for cmd in ["Ctrl-W q", "q!", "quit!"] {
            let f = Fixture::new();
            let mut e = mixed_batch(&f);
            let r = e.take_format_request().unwrap();
            e.vsplit(None).unwrap();
            command(&mut e, cmd);
            assert_eq!(e.panes().len(), 1, "{cmd}");
            assert!(!e.should_quit, "{cmd}");
            assert!(e.has_pending_saves(), "{cmd} dropped pending writes");
            e.finish_format_request(r.token, Ok(edits())).unwrap();
            e.flush_pending_saves().unwrap();
            assert_eq!(std::fs::read_to_string(&r.path).unwrap(), "bar\n", "{cmd}");
        }
    }

    #[test]
    fn autosave_remains_unformatted() {
        for mode in 0..8 {
            let f = Fixture::new();
            let mut e = f.editor();
            e.settings.editor.format_on_save = true;
            let marker = f.0.join("formatted");
            if mode != 1 {
                e.languages_config.languages.insert(
                    "markdown".into(),
                    nevi::config::LanguageConfig {
                        formatter: Some(nevi::config::FormatterConfig {
                            command: "sh".into(),
                            args: vec![
                                "-c".into(),
                                "touch \"$1\"; sed s/foo/bar/".into(),
                                "format".into(),
                                marker.to_string_lossy().into(),
                            ],
                            timeout: 1,
                        }),
                        tab_width: None,
                    },
                );
            }
            let now = Instant::now();
            let mut pending = Some(now);
            match mode {
                2 => e.mode = nevi::Mode::Command,
                3 => e.buffer_mut().dirty = false,
                4 => e.buffer_mut().path = None,
                5 => pending = Some(now + Duration::from_secs(1)),
                6 => e.buffer_mut().set_read_only(true),
                7 => {
                    std::fs::write(f.0.join("a.md"), "external\n").unwrap();
                }
                _ => {}
            }
            try_autosave(&mut e, &mut pending, now);
            assert!(!marker.exists());
            assert!(e.take_format_request().is_none());
            assert_eq!(
                std::fs::read_to_string(f.0.join("a.md")).unwrap(),
                if mode <= 1 {
                    "foo\n"
                } else if mode == 7 {
                    "external\n"
                } else {
                    "old\n"
                }
            );
            assert_eq!(pending.is_some(), mode == 5);
        }
    }
}
