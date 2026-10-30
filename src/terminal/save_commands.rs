//! Policy selection for existing write commands. Primitive writes stay raw.
use super::{Command, CommandResult, Editor, close_pane_or_quit};
use crate::editor::{FormatPolicy, WriteOptions};

pub(super) fn execute_write_command(editor: &mut Editor, command: Command) -> CommandResult {
    let flush_error = editor.flush_pending_saves().err();
    let result = match execute(editor, command) {
        Ok(result) => result,
        Err(error) if error.to_string() == "No filename" => {
            CommandResult::Error("No filename".into())
        }
        Err(error) => CommandResult::Error(format!("Error saving: {error}")),
    };
    with_flush_error(editor, result, flush_error)
}
fn execute(editor: &mut Editor, command: Command) -> anyhow::Result<CommandResult> {
    let quit = matches!(
        command,
        Command::WriteQuit
            | Command::WriteQuitIfModified
            | Command::WriteQuitAll
            | Command::WriteQuitAllIfModified
    );
    let force = matches!(command, Command::ForceWrite(_));
    let policy = if quit {
        FormatPolicy::ExternalOnly
    } else {
        FormatPolicy::ExternalThenLsp
    };
    let options = WriteOptions {
        force,
        format: editor.settings.editor.format_on_save.then_some(policy),
    };
    let progress = match command {
        Command::Write(path) | Command::ForceWrite(path) => {
            if let Some(path) = path {
                if force {
                    editor.save_as_force(path)?;
                } else {
                    editor.save_as(path)?;
                }
                return Ok(CommandResult::Ok);
            }
            if editor.macro_edit_register().is_some() {
                return Ok(match editor.save_macro_edit_buffer() {
                    Ok(m) => CommandResult::Message(m),
                    Err(e) => CommandResult::Error(e),
                });
            }
            editor.begin_save_current(options)?
        }
        Command::WriteAll => editor.begin_save_all(options)?,
        Command::WriteQuit => {
            editor.begin_save_current(options)?;
            return Ok(close_pane_or_quit(editor));
        }
        Command::WriteQuitIfModified => {
            if editor.has_unsaved_changes() {
                editor.begin_save_current(options)?;
            }
            return Ok(close_pane_or_quit(editor));
        }
        Command::WriteQuitAll | Command::WriteQuitAllIfModified => {
            editor.begin_save_all(options)?;
            return Ok(CommandResult::Quit);
        }
        _ => unreachable!("write adapter only accepts write commands"),
    };
    Ok(CommandResult::Message(
        editor.save_progress_message(progress),
    ))
}
pub(super) fn execute_format(editor: &mut Editor) -> CommandResult {
    let flush_error = editor.flush_pending_saves().err();
    let result = match editor.begin_format_current() {
        Ok(p) => CommandResult::Message(editor.save_progress_message(p)),
        Err(e) => CommandResult::Error(e.to_string()),
    };
    with_flush_error(editor, result, flush_error)
}
fn with_flush_error(
    editor: &mut Editor,
    result: CommandResult,
    error: Option<anyhow::Error>,
) -> CommandResult {
    let Some(error) = error else {
        return result;
    };
    let warning = format!("Previous write failed: {error}");
    // Keep the warning through asynchronous completion of the new command too.
    editor.note_failed_flush(error.to_string());
    match result {
        CommandResult::Message(message) => CommandResult::Message(format!("{warning}; {message}")),
        CommandResult::Error(error) => CommandResult::Error(format!("{warning}; {error}")),
        CommandResult::Ok => CommandResult::Message(format!(
            "{warning}; {}",
            editor
                .status_message
                .as_deref()
                .unwrap_or("Command completed")
        )),
        CommandResult::Quit => {
            editor.set_status(format!("{warning}; Quit"));
            CommandResult::Quit
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{execute_command, handle_key};
    use crate::config::{FormatterConfig, LanguageConfig, LanguagesConfig};
    use crate::{
        commands::{Command, parse_command},
        editor::{Editor, Mode, save_test_support::Fixture},
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::collections::HashMap;
    fn run(e: &mut Editor, command: &str) {
        execute_command(e, parse_command(command));
    }
    fn external(f: &Fixture, e: &mut Editor) {
        f.formatter(
            e,
            "sh",
            &[
                "-c",
                "touch \"$1\"; sed s/foo/bar/",
                "formatter",
                f.0.join("formatter-ran").to_str().unwrap(),
            ],
        );
    }
    fn disk(f: &Fixture, name: &str) -> String {
        std::fs::read_to_string(f.0.join(name)).unwrap()
    }

    #[test]
    fn failed_raw_write_all_allows_force_write_retry() {
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        e.settings.editor.format_on_save = false;
        e.replace_buffer_content("first\n");
        e.open_file(f.file("b.md", "old\n")).unwrap();
        e.replace_buffer_content("second\n");
        std::fs::write(f.0.join("b.md"), "external\n").unwrap();
        run(&mut e, "wa");
        assert!(e.status_message.as_ref().unwrap().contains("use :w!"));
        assert_eq!(disk(&f, "a.md"), "first\n");
        assert_eq!(disk(&f, "b.md"), "external\n");
        run(&mut e, "q");
        assert!(!e.should_quit);
        assert!(
            e.status_message
                .as_ref()
                .unwrap()
                .contains("No write since last change")
        );
        run(&mut e, "w!");
        assert_eq!(disk(&f, "b.md"), "second\n");
        assert!(!e.has_pending_saves());
        run(&mut e, "q");
        assert!(e.should_quit);
    }
    #[test]
    fn save_status_preserves_filename_formatter_and_batch_wording() {
        for command in ["w", "w!", "wa"] {
            for backend in ["raw", "sed", "cat", "false"] {
                let f = Fixture::new();
                let mut e = f.editor("a.md");
                e.settings.editor.format_on_save = backend != "raw";
                if backend != "raw" {
                    f.formatter(
                        &mut e,
                        backend,
                        if backend == "sed" {
                            &["s/foo/bar/"]
                        } else {
                            &[]
                        },
                    );
                }
                e.replace_buffer_content("foo\n");
                if command == "wa" {
                    e.open_file(f.file("b.md", "old\n")).unwrap();
                    e.replace_buffer_content("foo\n");
                }
                run(&mut e, command);
                let status = e.status_message.as_deref().unwrap();
                match (command, backend) {
                    ("wa", "sed") => assert_eq!(status, "Formatted with sed and saved 2 buffers"),
                    ("wa", "false") => assert!(
                        status.starts_with("Saved 2 buffers (false failed: "),
                        "{status}"
                    ),
                    ("wa", _) => assert_eq!(status, "Saved 2 buffers"),
                    (_, "sed") => assert_eq!(status, "Formatted with sed and saved"),
                    (_, "cat") => assert_eq!(status, "Saved (formatted with cat)"),
                    (_, "false") => {
                        assert!(status.starts_with("Saved (false failed: "), "{status}")
                    }
                    _ => assert_eq!(status, "\"a.md\" written"),
                }
            }
        }
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        run(&mut e, "wa");
        assert_eq!(
            e.status_message.as_deref(),
            Some("No modified buffers to save")
        );
        e.settings.editor.format_on_save = false;
        e.replace_buffer_content("foo\n");
        run(&mut e, "wa");
        assert_eq!(e.status_message.as_deref(), Some("Saved 1 buffer"));
    }

    #[test]
    fn write_uses_shared_format_policy_and_saves_after_failure() {
        for backend in 0..3 {
            let f = Fixture::new();
            let mut e = f.editor("a.md");
            e.replace_buffer_content("foo\n");
            if backend == 0 {
                external(&f, &mut e);
            } else if backend == 1 {
                f.formatter(&mut e, "false", &[]);
            }
            run(&mut e, "w");
            if backend == 2 {
                let r = e.take_format_request().expect("targeted LSP save");
                assert!(e.has_pending_saves());
                e.finish_format_request(r.token, Ok(vec![])).unwrap();
            }
            assert_eq!(
                disk(&f, "a.md"),
                if backend == 0 { "bar\n" } else { "foo\n" }
            );
            assert!(!e.buffer().dirty);
        }
    }
    #[test]
    fn ctrl_s_matches_colon_write() {
        for backend in 0..4 {
            let f = Fixture::new();
            let mut e = f.editor("a.md");
            e.replace_buffer_content("foo\n");
            if backend == 0 || backend == 3 {
                external(&f, &mut e);
            } else if backend == 1 {
                f.formatter(&mut e, "false", &[]);
            }
            if backend == 3 {
                e.settings.editor.format_on_save = false;
            }
            handle_key(
                &mut e,
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            );
            if backend == 2 {
                let r = e.take_format_request().expect("Ctrl-S requests formatting");
                e.finish_format_request(r.token, Ok(vec![])).unwrap();
            }
            assert_eq!(
                disk(&f, "a.md"),
                if backend == 0 { "bar\n" } else { "foo\n" }
            );
            assert_eq!(e.mode, Mode::Normal);
            assert!(!e.buffer().dirty);
        }
    }
    #[test]
    fn write_all_resolves_each_dirty_named_buffer() {
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        external(&f, &mut e);
        e.replace_buffer_content("foo\n");
        for name in ["b.rs", "c.rs"] {
            e.open_file(f.file(name, "old\n")).unwrap();
            e.replace_buffer_content("foo\n");
        }
        e.open_file(f.file("unnamed.md", "foo\n")).unwrap();
        e.buffer_mut().path = None;
        e.replace_buffer_content("unnamed\n");
        let unnamed = e.buffer().id();
        e.open_file(f.file("clean.md", "foo\n")).unwrap();
        let id = e.buffer().id();
        let cursor = e.cursor;
        run(&mut e, "wa");
        for target in 0..2 {
            let r = e.take_format_request().expect("each LSP target");
            let progress = e.finish_format_request(r.token, Ok(vec![])).unwrap();
            if target == 1 {
                let crate::editor::SaveProgress::Complete(summary) = progress else {
                    panic!("batch must complete");
                };
                assert_eq!(summary.saved, 3);
                assert_eq!(summary.formatted, 1);
            }
        }
        assert_eq!(disk(&f, "a.md"), "bar\n");
        assert_eq!(disk(&f, "b.rs"), "foo\n");
        assert_eq!(disk(&f, "c.rs"), "foo\n");
        assert_eq!(disk(&f, "clean.md"), "foo\n");
        assert_eq!(e.buffer().id(), id);
        assert_eq!(e.cursor, cursor);
        assert!(
            e.buffers()
                .iter()
                .find(|b| b.id() == unnamed)
                .unwrap()
                .dirty
        );
    }
    #[test]
    fn write_quit_formats_external_then_closes() {
        for backend in 0..3 {
            let f = Fixture::new();
            let mut e = f.editor("a.md");
            if backend == 0 {
                external(&f, &mut e);
            } else if backend == 1 {
                f.formatter(&mut e, "false", &[]);
            }
            run(&mut e, "wq");
            assert!(e.should_quit);
            assert!(e.take_format_request().is_none());
            assert_eq!(
                disk(&f, "a.md"),
                if backend == 0 { "bar\n" } else { "foo\n" }
            );
        }
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        e.buffer_mut().set_read_only(true);
        run(&mut e, "wq");
        assert!(!e.should_quit);
    }
    #[test]
    fn exit_formats_only_when_dirty_and_skips_lsp() {
        for command in ["x", "exit"] {
            for dirty in [false, true] {
                for backend in 0..3 {
                    let f = Fixture::new();
                    let mut e = f.editor("a.md");
                    if backend == 0 {
                        external(&f, &mut e);
                    } else if backend == 1 {
                        f.formatter(&mut e, "false", &[]);
                    }
                    if dirty {
                        e.replace_buffer_content("foo\n");
                    }
                    run(&mut e, command);
                    assert!(e.should_quit);
                    assert!(e.take_format_request().is_none());
                    assert_eq!(
                        disk(&f, "a.md"),
                        if dirty && backend == 0 {
                            "bar\n"
                        } else {
                            "foo\n"
                        }
                    );
                    assert_eq!(f.0.join("formatter-ran").exists(), dirty && backend == 0);
                }
            }
        }
    }
    fn zz(e: &mut Editor) {
        for _ in 0..2 {
            handle_key(e, KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT));
        }
    }
    #[test]
    fn zz_formats_only_when_dirty_and_preserves_pane_rules() {
        for dirty in [false, true] {
            let f = Fixture::new();
            let mut e = f.editor("a.md");
            external(&f, &mut e);
            if dirty {
                e.replace_buffer_content("foo\n");
            }
            zz(&mut e);
            assert!(e.should_quit);
            assert_eq!(disk(&f, "a.md"), if dirty { "bar\n" } else { "foo\n" });
            assert_eq!(f.0.join("formatter-ran").exists(), dirty);
        }
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        external(&f, &mut e);
        e.replace_buffer_content("foo\n");
        e.vsplit(None).unwrap();
        zz(&mut e);
        assert_eq!(e.panes().len(), 1);
        assert!(!e.should_quit);
        let f = Fixture::new();
        let mut e = f.editor("hidden.md");
        e.replace_buffer_content("dirty\n");
        e.open_file(f.file("a.md", "foo\n")).unwrap();
        external(&f, &mut e);
        e.replace_buffer_content("foo\n");
        zz(&mut e);
        assert!(!e.should_quit);
        assert!(e.buffer().path.as_ref().unwrap().ends_with("hidden.md"));
    }
    #[test]
    fn write_quit_all_formats_external_only_per_dirty_buffer() {
        for command in ["wqa", "wqall", "xall", "xa"] {
            for backend in 0..3 {
                let f = Fixture::new();
                let mut e = f.editor("a.md");
                if backend == 0 {
                    external(&f, &mut e);
                } else if backend == 1 {
                    f.formatter(&mut e, "false", &[]);
                }
                e.replace_buffer_content("foo\n");
                e.open_file(f.file("b.rs", "old\n")).unwrap();
                e.replace_buffer_content("raw\n");
                e.open_file(f.file("unnamed.md", "foo\n")).unwrap();
                e.buffer_mut().path = None;
                e.replace_buffer_content("unnamed\n");
                let unnamed = e.buffer().id();
                e.open_file(f.file("clean.md", "foo\n")).unwrap();
                run(&mut e, command);
                assert!(e.should_quit);
                assert!(e.take_format_request().is_none());
                assert_eq!(
                    disk(&f, "a.md"),
                    if backend == 0 { "bar\n" } else { "foo\n" }
                );
                assert_eq!(disk(&f, "b.rs"), "raw\n");
                assert_eq!(disk(&f, "clean.md"), "foo\n");
                assert!(
                    e.buffers()
                        .iter()
                        .find(|b| b.id() == unnamed)
                        .unwrap()
                        .dirty
                );
            }
            let f = Fixture::new();
            let mut e = f.editor("a.md");
            e.replace_buffer_content("dirty\n");
            e.buffer_mut().set_read_only(true);
            run(&mut e, command);
            assert!(!e.should_quit);
            assert_eq!(disk(&f, "a.md"), "foo\n");
        }
    }
    #[test]
    fn force_write_formats_then_overwrites_external_changes() {
        for command in ["w!", "write!"] {
            for failed in [false, true] {
                let f = Fixture::new();
                let mut e = f.editor("a.md");
                if failed {
                    f.formatter(&mut e, "false", &[]);
                } else {
                    external(&f, &mut e);
                }
                e.replace_buffer_content("foo\n");
                std::fs::write(f.0.join("a.md"), "external\n").unwrap();
                run(&mut e, command);
                assert_eq!(disk(&f, "a.md"), if failed { "foo\n" } else { "bar\n" });
                assert!(!e.buffer().dirty);
            }
        }
        let f = Fixture::new();
        let mut e = f.editor("a.md");
        external(&f, &mut e);
        e.buffer_mut().set_read_only(true);
        run(&mut e, "w!");
        assert_eq!(disk(&f, "a.md"), "foo\n");
        assert!(e.status_message.as_ref().unwrap().contains("read-only"));
    }
    #[test]
    fn force_save_as_skips_format_and_overwrites_destination() {
        for same in [false, true] {
            let f = Fixture::new();
            let mut e = f.editor("a.md");
            external(&f, &mut e);
            e.replace_buffer_content("foo edited\n");
            let dest = if same {
                f.0.join("a.md")
            } else {
                f.file("dest.md", "existing\n")
            };
            execute_command(&mut e, Command::ForceWrite(Some(dest.clone())));
            assert_eq!(std::fs::read_to_string(dest).unwrap(), "foo edited\n");
            assert_eq!(
                disk(&f, "a.md"),
                if same { "foo edited\n" } else { "foo\n" }
            );
            assert!(!f.0.join("formatter-ran").exists());
            assert!(e.take_format_request().is_none());
        }
    }
    #[test]
    fn save_as_skips_external_and_lsp_formatting() {
        for backend in [false, true] {
            for same in [false, true] {
                for unnamed in [false, true] {
                    let f = Fixture::new();
                    let mut e = f.editor("a.md");
                    if backend {
                        external(&f, &mut e);
                    }
                    e.replace_buffer_content("foo\n");
                    if unnamed {
                        e.buffer_mut().path = None;
                    }
                    let dest = f.0.join(if same { "a.md" } else { "dest.md" });
                    execute_command(&mut e, Command::Write(Some(dest.clone())));
                    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "foo\n");
                    assert_eq!(e.buffer().path.as_ref(), Some(&dest));
                    assert!(e.take_format_request().is_none());
                    assert!(!f.0.join("formatter-ran").exists());
                }
            }
        }
    }
    #[test]
    fn force_quit_never_formats_or_writes() {
        for command in ["q!", "qa!"] {
            let f = Fixture::new();
            let mut e = f.editor("a.md");
            external(&f, &mut e);
            e.replace_buffer_content("unsaved\n");
            run(&mut e, command);
            assert_eq!(disk(&f, "a.md"), "foo\n");
            assert_eq!(e.buffer().content(), "unsaved\n");
            assert!(e.take_format_request().is_none());
            assert!(!f.0.join("formatter-ran").exists());
        }
    }
    #[test]
    fn save_adapters_preserve_macro_and_filename_behavior() {
        for cmd in ["w", "w!"] {
            let mut e = Editor::default();
            e.settings.editor.format_on_save = true;
            run(&mut e, "MacroEdit a");
            e.replace_buffer_content("dw");
            run(&mut e, cmd);
            assert!(e.macros.get_macro('a').is_some());
            assert!(!e.buffer().dirty);
            assert!(e.take_format_request().is_none());
            let f = Fixture::new();
            let dest = f.0.join("macro.md");
            execute_command(&mut e, Command::Write(Some(dest.clone())));
            assert_eq!(std::fs::read_to_string(dest).unwrap(), "dw\n");
        }
        let mut e = Editor::default();
        run(&mut e, "w");
        assert!(e.status_message.as_ref().unwrap().contains("No filename"));
        for cmd in ["w", "w!", "wa", "wq", "x", "ZZ", "wqa", "xa"] {
            let f = Fixture::new();
            let mut e = f.editor("a.md");
            external(&f, &mut e);
            e.settings.editor.format_on_save = false;
            e.replace_buffer_content("foo\n");
            if cmd == "ZZ" {
                zz(&mut e);
            } else {
                run(&mut e, cmd);
            }
            assert_eq!(disk(&f, "a.md"), "foo\n");
            assert!(e.take_format_request().is_none());
            assert!(!f.0.join("formatter-ran").exists());
        }
    }
    fn markdown_languages_with_formatter(command: &str, args: &[&str]) -> LanguagesConfig {
        LanguagesConfig {
            languages: HashMap::from([(
                "markdown".to_string(),
                LanguageConfig {
                    formatter: Some(FormatterConfig {
                        command: command.to_string(),
                        args: args.iter().map(|arg| arg.to_string()).collect(),
                        timeout: 5,
                    }),
                    tab_width: None,
                },
            )]),
        }
    }

    #[test]
    fn colon_format_uses_languages_toml_formatter() {
        let fixture = Fixture::new();
        let tmp = fixture.0.clone();
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("note.md");
        std::fs::write(&path, "foo\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.languages_config = markdown_languages_with_formatter("sed", &["-e", "s/foo/bar/"]);
        editor.open_file(path.clone()).expect("open markdown");

        execute_command(&mut editor, Command::Format);

        assert_eq!(editor.buffer().content(), "bar\n");
        assert!(editor.buffer().dirty);
        assert!(editor.pending_lsp_action.is_none());
        assert_eq!(std::fs::read_to_string(&path).expect("read file"), "foo\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn colon_format_without_formatter_requests_lsp() {
        let fixture = Fixture::new();
        let tmp = fixture.0.clone();
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("note.md");
        std::fs::write(&path, "foo\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.languages_config = LanguagesConfig::default();
        editor.open_file(path.clone()).expect("open markdown");

        execute_command(&mut editor, Command::Format);

        assert!(editor.take_format_request().is_some());
        assert_eq!(editor.buffer().content(), "foo\n");
        assert!(!editor.buffer().dirty);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn colon_format_formatter_error_does_not_request_lsp() {
        let fixture = Fixture::new();
        let tmp = fixture.0.clone();
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("note.md");
        std::fs::write(&path, "foo\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.languages_config = markdown_languages_with_formatter("false", &[]);
        editor.open_file(path.clone()).expect("open markdown");

        execute_command(&mut editor, Command::Format);

        assert_eq!(editor.buffer().content(), "foo\n");
        assert!(!editor.buffer().dirty);
        assert!(editor.pending_lsp_action.is_none());
        assert!(
            editor
                .status_message
                .as_deref()
                .is_some_and(|message| message.contains("false")),
            "expected formatter error to mention command, got {:?}",
            editor.status_message
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn colon_format_unchanged_output_does_not_dirty_buffer() {
        let fixture = Fixture::new();
        let tmp = fixture.0.clone();
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("note.md");
        std::fs::write(&path, "foo\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.languages_config = markdown_languages_with_formatter("cat", &[]);
        editor.open_file(path.clone()).expect("open markdown");

        execute_command(&mut editor, Command::Format);

        assert_eq!(editor.buffer().content(), "foo\n");
        assert!(!editor.buffer().dirty);
        assert!(editor.pending_lsp_action.is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn colon_write_formats_with_languages_toml_then_saves() {
        let fixture = Fixture::new();
        let tmp = fixture.0.clone();
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("note.md");
        std::fs::write(&path, "foo\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.settings.editor.format_on_save = true;
        editor.languages_config = markdown_languages_with_formatter("sed", &["-e", "s/foo/bar/"]);
        editor.open_file(path.clone()).expect("open markdown");
        editor.replace_buffer_content("foo\n");

        execute_command(&mut editor, Command::Write(None));

        assert_eq!(editor.buffer().content(), "bar\n");
        assert!(!editor.buffer().dirty);
        assert!(editor.pending_lsp_action.is_none());
        assert_eq!(std::fs::read_to_string(&path).expect("read file"), "bar\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn colon_write_saves_when_languages_toml_formatter_fails() {
        let fixture = Fixture::new();
        let tmp = fixture.0.clone();
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("note.md");
        std::fs::write(&path, "original\n").expect("write markdown");

        let mut editor = Editor::default();
        editor.settings.editor.format_on_save = true;
        editor.languages_config = markdown_languages_with_formatter("false", &[]);
        editor.open_file(path.clone()).expect("open markdown");
        editor.replace_buffer_content("edited\n");

        execute_command(&mut editor, Command::Write(None));

        assert_eq!(editor.buffer().content(), "edited\n");
        assert!(!editor.buffer().dirty);
        assert!(editor.pending_lsp_action.is_none());
        assert_eq!(
            std::fs::read_to_string(&path).expect("read file"),
            "edited\n"
        );
        assert!(
            editor
                .status_message
                .as_deref()
                .is_some_and(|message| message.contains("false")),
            "expected save message to mention formatter failure, got {:?}",
            editor.status_message
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
