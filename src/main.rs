use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nevi::copilot::{
    utf16_to_utf8_col, utf8_to_utf16_col, AuthStatus, CopilotCompletion, CopilotManager,
    CopilotNotification, CopilotStatus,
};
use nevi::editor::{CopilotAction, CopilotGhostText, LspAction};
use nevi::lsp;
use nevi::terminal::{execute_leader_action, handle_key, EditorEvent};
use nevi::{
    editor::RegisterContent,
    floating_terminal::{
        is_terminal_selection_clear_key, is_terminal_selection_copy_key,
        is_terminal_selection_platform_copy_key, TerminalMouseEventResult,
    },
    load_config, AutosaveMode, Editor, LanguageId, LspNotification, Mode, MultiLspManager,
    Terminal,
};

fn profile_enabled_from_env() -> bool {
    profile_enabled_from_value(env::var("NEVI_PROFILE").ok().as_deref())
}

fn profile_enabled_from_value(value: Option<&str>) -> bool {
    let Some(value) = value.map(str::trim) else {
        return false;
    };

    value == "1"
        || value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("yes")
        || value.eq_ignore_ascii_case("on")
}

fn editor_lsp_cursor_col(editor: &Editor) -> u32 {
    editor_lsp_col(editor, editor.cursor.line, editor.cursor.col)
}

fn editor_lsp_line_len(editor: &Editor, line: usize) -> u32 {
    let Some(line_text) = editor.buffer().line(line).map(|line| line.to_string()) else {
        return 0;
    };
    let line_text = line_text.trim_end_matches('\n');
    utf8_to_utf16_col(line_text, line_text.chars().count())
}

fn editor_lsp_col(editor: &Editor, line: usize, col: usize) -> u32 {
    let Some(line_text) = editor.buffer().line(line).map(|line| line.to_string()) else {
        return 0;
    };
    let line_text = line_text.trim_end_matches('\n');
    utf8_to_utf16_col(line_text, col.min(line_text.chars().count()))
}

fn diagnostic_to_lsp_offsets(
    editor: &Editor,
    mut diagnostic: lsp::types::Diagnostic,
) -> lsp::types::Diagnostic {
    diagnostic.col_start = editor_lsp_col(editor, diagnostic.line, diagnostic.col_start) as usize;
    diagnostic.col_end = editor_lsp_col(editor, diagnostic.end_line, diagnostic.col_end) as usize;
    diagnostic
}

fn lsp_response_matches_current_buffer(
    editor: &Editor,
    request_uri: &str,
    request_version: u64,
) -> bool {
    let current_uri = editor.buffer().path.as_ref().map(lsp::path_to_uri);
    current_uri.as_deref() == Some(request_uri) && editor.buffer().version() == request_version
}

fn lsp_completion_response_matches_current_cursor(
    editor: &Editor,
    request_uri: &str,
    request_version: u64,
    request_line: u32,
    request_character: u32,
) -> bool {
    lsp_response_matches_current_buffer(editor, request_uri, request_version)
        && editor.cursor.line as u32 == request_line
        && editor_lsp_cursor_col(editor) == request_character
}

fn request_selected_completion_resolve(
    editor: &mut Editor,
    mlsp: &mut MultiLspManager,
    last_resolved_completion: &mut Option<u64>,
) {
    let Some((item_id, label, raw_data)) = editor.completion.selected_item().and_then(|item| {
        if item.documentation.is_none() {
            item.raw_data
                .clone()
                .map(|raw_data| (item.item_id, item.label.clone(), raw_data))
        } else {
            None
        }
    }) else {
        return;
    };

    if last_resolved_completion.as_ref() == Some(&item_id) {
        return;
    }

    let Some(path) = editor.buffer().path.clone() else {
        return;
    };
    if !mlsp.is_ready_for_file(&path) {
        return;
    }

    *last_resolved_completion = Some(item_id);
    let _ = mlsp.completion_resolve(&path, raw_data, item_id, label);
}

fn main() -> anyhow::Result<()> {
    // Profiling is opt-in with NEVI_PROFILE=1/true/yes/on.
    let profile_enabled = profile_enabled_from_env();
    let mut profile_file = if profile_enabled {
        Some(std::fs::File::create("/tmp/nevi_profile.log").ok())
    } else {
        None
    };

    // Helper macro for profiling
    macro_rules! profile {
        ($file:expr, $($arg:tt)*) => {
            if profile_enabled {
                if let Some(Some(ref mut f)) = $file {
                    let _ = writeln!(f, $($arg)*);
                }
            }
        };
    }

    // Load configuration
    let settings = load_config();
    // Store LSP settings before moving settings into editor
    let lsp_enabled = settings.lsp.enabled;
    let lsp_servers = settings.lsp.servers.clone();
    // Store autosave settings
    let autosave_mode = settings.editor.autosave.clone();
    let autosave_delay = Duration::from_millis(settings.editor.autosave_delay_ms);
    // Store Copilot settings
    let copilot_settings = settings.copilot.clone();

    // Initialize editor with settings
    let mut editor = Editor::new(settings);

    // Enable finder profiling when profiling is enabled.
    if profile_enabled {
        nevi::terminal::FINDER_PROFILE_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
        // Clear the finder profile log
        let _ = std::fs::write("/tmp/nevi_finder_profile.log", "");
    }

    // Display any startup errors (config parse errors, etc.)
    let startup_errors = editor.take_startup_errors();
    if !startup_errors.is_empty() {
        // Join multiple errors with semicolons for display
        let error_msg = startup_errors.join("; ");
        editor.set_status(format!("Config errors: {}", error_msg));
    }

    // Check command line argument - could be file or directory
    let arg_path = env::args().nth(1).map(PathBuf::from);
    let mut initial_file: Option<PathBuf> = None;
    let mut open_file_picker = false;

    if let Some(ref path) = arg_path {
        // Canonicalize the path to get absolute path
        let abs_path = path.canonicalize().unwrap_or_else(|_| path.clone());

        if abs_path.is_dir() {
            // Directory: set as project root and open file picker
            editor.set_project_root(abs_path);
            open_file_picker = true;
        } else if abs_path.is_file() || !abs_path.exists() {
            // File (or new file): open it and set parent as project root
            initial_file = Some(abs_path.clone());
            if let Some(parent) = abs_path.parent() {
                editor.set_project_root(parent.to_path_buf());
            }
            editor.open_file(abs_path)?;
        }
    }

    // If no argument, use current directory as project root
    if arg_path.is_none() {
        if let Ok(cwd) = env::current_dir() {
            editor.set_project_root(cwd);
        }
    }

    // Initialize git repository for git signs
    editor.init_git();

    // Update git diff for initial file if opened
    if initial_file.is_some() {
        editor.update_git_diff();
    }

    // Initialize terminal
    let mut terminal = Terminal::new()?;

    // Get initial size
    let (width, height) = Terminal::size()?;
    editor.set_size(width, height);

    // If we opened a directory, open the file picker
    if open_file_picker {
        editor.open_finder_files();
    }

    // Initialize Multi-LSP manager if enabled
    // Servers are started lazily when files of that type are opened
    let mut multi_lsp: Option<MultiLspManager> = None;
    let mut lsp_current_file: Option<PathBuf> = None; // Track which file LSP knows about

    if lsp_enabled {
        // Collect configured root markers so workspace detection is not Cargo-only.
        let mut root_markers: Vec<String> = Vec::new();
        for cfg in [
            &lsp_servers.rust,
            &lsp_servers.typescript,
            &lsp_servers.javascript,
            &lsp_servers.css,
            &lsp_servers.json,
            &lsp_servers.toml,
            &lsp_servers.markdown,
            &lsp_servers.html,
            &lsp_servers.python,
        ] {
            for marker in &cfg.root_patterns {
                if marker.trim().is_empty() {
                    continue;
                }
                if !root_markers.iter().any(|existing| existing == marker) {
                    root_markers.push(marker.clone());
                }
            }
        }
        if root_markers.is_empty() {
            root_markers.push("Cargo.toml".to_string());
        }

        // Determine workspace root - prefer project root, fall back to file's workspace
        let workspace_root = if let Some(ref project_root) = editor.project_root {
            find_workspace_root(project_root.as_path(), &root_markers)
        } else if let Some(ref path) = initial_file {
            find_workspace_root(path.as_path(), &root_markers)
        } else {
            env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        };

        // Create MultiLspManager with all server configs
        let mgr = MultiLspManager::new(
            workspace_root,
            lsp_servers.rust,
            lsp_servers.typescript,
            lsp_servers.javascript,
            lsp_servers.css,
            lsp_servers.json,
            lsp_servers.toml,
            lsp_servers.markdown,
            lsp_servers.html,
            lsp_servers.python,
        );
        multi_lsp = Some(mgr);
        editor.set_lsp_status("LSP: (no server)");
    }

    // Initialize Copilot manager if enabled
    let mut copilot: Option<CopilotManager> = None;
    if copilot_settings.enabled {
        let mut mgr = CopilotManager::new(copilot_settings);
        // Try to start the Copilot server
        match mgr.start() {
            Ok(()) => {
                // Started successfully, will get Initialized notification later
            }
            Err(e) => {
                // Failed to start - show status but don't block the editor
                editor.set_status(format!("Copilot: {}", e));
            }
        }
        copilot = Some(mgr);
    }

    // Copilot debouncing: delay completion requests
    let copilot_debounce = Duration::from_millis(150);
    let mut copilot_last_request: Option<Instant> = None;
    let mut copilot_current_file: Option<PathBuf> = None; // Track which file Copilot knows about

    let debounce = Duration::from_millis(200);
    let poll_timeout = Duration::from_millis(16);
    let mut needs_redraw = true;
    let render_interval = Duration::from_millis(16);
    let lsp_render_interval = Duration::from_millis(50);
    let mut last_render = Instant::now() - render_interval;
    let max_key_events_per_frame: usize = 8;
    let mut redraw_from_input = false;
    let mut terminal_redraw_pending = false;
    let typing_pause = Duration::from_millis(50);
    let mut last_input_at: Option<Instant> = None;

    // Autosave state: track when the last edit occurred
    // When autosave_pending is Some, an autosave is scheduled for that time
    let mut autosave_pending: Option<Instant> = None;

    // Completion debouncing: delay completion requests to avoid flooding LSP
    // Stores (request_time, path, line, col) - request is sent after debounce period
    let completion_debounce = Duration::from_millis(50);
    let mut completion_pending: Option<(Instant, PathBuf, u32, u32)> = None;

    // Track which completion item we've already requested resolve for
    // (to avoid spamming resolve requests on every key press)
    let mut last_resolved_completion: Option<u64> = None;

    // Finder preview debouncing: delay preview updates to avoid tree-sitter parsing on every keystroke
    let preview_debounce = Duration::from_millis(50);
    let mut preview_pending_since: Option<Instant> = None;

    // Grep search debouncing: delay grep searches to avoid searching on every keystroke
    let grep_debounce = Duration::from_millis(150);
    let mut grep_pending_since: Option<Instant> = None;

    // Main event loop
    let mut loop_start = Instant::now();
    'main_loop: loop {
        // Track loop cycle time
        let cycle_time = loop_start.elapsed();
        if cycle_time.as_millis() > 100 {
            profile!(profile_file, "SLOW_CYCLE: {:?}", cycle_time);
        }
        loop_start = Instant::now();

        if terminal.poll_key(poll_timeout)? {
            let mut events_processed = 0usize;
            loop {
                if events_processed >= max_key_events_per_frame {
                    break;
                }
                let prev_version = editor.buffer().version();
                if let Some(event) = terminal.read_event()? {
                    // Handle focus gained for autoread
                    let key = match event {
                        EditorEvent::FocusGained => {
                            // Check all open buffers for external changes
                            let reload_result = editor.check_and_reload_external_changes();
                            editor.refresh_git_state();
                            if let Some(msg) = reload_result {
                                editor.set_status(msg);
                            }
                            needs_redraw = true;
                            continue 'main_loop; // No key to handle
                        }
                        EditorEvent::Resize(cols, rows) => {
                            editor.set_size(cols, rows);
                            needs_redraw = true;
                            redraw_from_input = true;
                            continue 'main_loop;
                        }
                        EditorEvent::Mouse(mouse) => {
                            if editor.floating_terminal.is_visible() {
                                let terminal_settings = &editor.settings.terminal;
                                let content_area = nevi::floating_terminal::content_area_for_screen(
                                    editor.term_width,
                                    editor.term_height,
                                    terminal_settings.popup_width_ratio,
                                    terminal_settings.popup_height_ratio,
                                );
                                match editor
                                    .floating_terminal
                                    .send_mouse_event(mouse, content_area)
                                {
                                    TerminalMouseEventResult::Ignored => {}
                                    TerminalMouseEventResult::Handled => {
                                        last_input_at = Some(Instant::now());
                                        terminal_redraw_pending = true;
                                    }
                                }
                            }
                            events_processed += 1;
                            if !terminal.poll_key(Duration::from_millis(0))? {
                                break;
                            }
                            continue;
                        }
                        EditorEvent::Paste(text) => {
                            if editor.floating_terminal.is_visible() {
                                if editor.floating_terminal.send_paste(&text) {
                                    last_input_at = Some(Instant::now());
                                    terminal_redraw_pending = true;
                                }
                            } else {
                                for ch in text.chars() {
                                    let paste_key = match ch {
                                        '\n' | '\r' => {
                                            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
                                        }
                                        '\t' => KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                                        ch => KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                                    };
                                    handle_key(&mut editor, paste_key);
                                }
                                last_input_at = Some(Instant::now());
                                needs_redraw = true;
                                redraw_from_input = true;
                            }
                            events_processed += 1;
                            if !terminal.poll_key(Duration::from_millis(0))? {
                                break;
                            }
                            continue;
                        }
                        EditorEvent::Key(k) => k,
                    };

                    // Dismiss hover popup on any key press
                    editor.hover_content = None;
                    // Dismiss diagnostic float on any key press (it can be reopened with gl)
                    editor.show_diagnostic_float = false;

                    if editor.floating_terminal.is_visible() {
                        let has_selection = editor.floating_terminal.has_selection();
                        let platform_copy = is_terminal_selection_platform_copy_key(key);
                        let selection_copy = has_selection && is_terminal_selection_copy_key(key);
                        if platform_copy || selection_copy {
                            if let Some(text) = editor.floating_terminal.copy_selection() {
                                editor.registers.yank(None, RegisterContent::Chars(text));
                                editor.check_clipboard_error();
                                editor.set_status("Terminal selection copied");
                            } else {
                                editor.set_status("No terminal selection");
                            }
                            last_input_at = Some(Instant::now());
                            needs_redraw = true;
                            redraw_from_input = true;
                            events_processed += 1;
                            if !terminal.poll_key(Duration::from_millis(0))? {
                                break;
                            }
                            continue;
                        }

                        if has_selection && is_terminal_selection_clear_key(key) {
                            editor.floating_terminal.clear_selection();
                            last_input_at = Some(Instant::now());
                            terminal_redraw_pending = true;
                            events_processed += 1;
                            if !terminal.poll_key(Duration::from_millis(0))? {
                                break;
                            }
                            continue;
                        }
                    }

                    // Check for manual completion trigger (Ctrl+Space) in insert mode
                    let manual_completion = editor.mode == Mode::Insert
                        && !editor.floating_terminal.is_visible()
                        && key.modifiers == KeyModifiers::CONTROL
                        && key.code == KeyCode::Char(' ');

                    let mut key_went_to_terminal = false;
                    if manual_completion {
                        // Request completion from LSP (only if ready for this file type)
                        if let Some(ref mut mlsp) = multi_lsp {
                            if let Some(path) = editor.buffer().path.clone() {
                                if mlsp.is_ready_for_file(&path) {
                                    let _ = mlsp.completion(
                                        &path,
                                        editor.cursor.line as u32,
                                        editor_lsp_cursor_col(&editor),
                                        editor.buffer().version(),
                                    );
                                }
                            }
                        }
                    } else {
                        let t_handle_key = Instant::now();
                        let mode_before = editor.mode;
                        let terminal_visible_before_key = editor.floating_terminal.is_visible();
                        handle_key(&mut editor, key);
                        key_went_to_terminal =
                            terminal_visible_before_key && editor.floating_terminal.is_visible();
                        let elapsed = t_handle_key.elapsed();
                        // Only log slow handle_key operations (>1ms) with details
                        if elapsed.as_micros() > 1000 {
                            profile!(
                                profile_file,
                                "SLOW handle_key: {:?} mode={:?} key={:?}",
                                elapsed,
                                mode_before,
                                key.code
                            );
                        } else {
                            profile!(profile_file, "handle_key: {:?}", elapsed);
                        }
                    }
                    last_input_at = Some(Instant::now());

                    if key_went_to_terminal {
                        events_processed += 1;
                        if !terminal.poll_key(Duration::from_millis(0))? {
                            break;
                        }
                        continue;
                    }

                    // Resolve completion documentation/details for the selected item.
                    if editor.completion.active {
                        if let Some(ref mut mlsp) = multi_lsp {
                            request_selected_completion_resolve(
                                &mut editor,
                                mlsp,
                                &mut last_resolved_completion,
                            );
                        }
                    } else {
                        // Clear tracking when completion popup closes
                        last_resolved_completion = None;
                    }

                    // Handle pending LSP actions (gd, K) - only if LSP is ready
                    if let Some(action) = editor.pending_lsp_action.take() {
                        if let Some(ref mut mlsp) = multi_lsp {
                            if let Some(path) = editor.buffer().path.clone() {
                                if !mlsp.is_ready_for_file(&path) {
                                    // Try to start server for this file type
                                    if let Err(e) = mlsp.ensure_server_for_file(&path) {
                                        let message = mlsp
                                            .language_for_path(&path)
                                            .map(|lang| {
                                                mlsp.user_facing_error(lang, &e.to_string())
                                            })
                                            .unwrap_or_else(|| e.to_string());
                                        editor.set_status(format!("LSP: {}", message));
                                    } else {
                                        let status = mlsp.status(Some(path.as_path()));
                                        editor.set_lsp_status(status.clone());
                                        editor.set_status(status);
                                    }
                                } else {
                                    let line = editor.cursor.line as u32;
                                    let col = editor_lsp_cursor_col(&editor);
                                    match action {
                                        LspAction::GotoDefinition => {
                                            let _ = mlsp.goto_definition(&path, line, col);
                                        }
                                        LspAction::Hover => {
                                            let _ = mlsp.hover(&path, line, col);
                                        }
                                        LspAction::Formatting => {
                                            editor.pending_format = true;
                                            let _ = mlsp.formatting(
                                                &path,
                                                editor.settings.editor.tab_width as u32,
                                                editor.buffer().version(),
                                            );
                                        }
                                        LspAction::FindReferences => {
                                            let _ = mlsp.references(&path, line, col);
                                        }
                                        LspAction::CodeActions => {
                                            // Get diagnostics at cursor position
                                            let diagnostics = editor
                                                .all_diagnostics_at_cursor()
                                                .into_iter()
                                                .map(|diagnostic| {
                                                    diagnostic_to_lsp_offsets(&editor, diagnostic)
                                                })
                                                .collect::<Vec<_>>();
                                            // Use full line range to get all code actions (import fixes, etc.)
                                            let line_len =
                                                editor_lsp_line_len(&editor, editor.cursor.line);
                                            let _ = mlsp.code_action(
                                                &path,
                                                line,
                                                0, // start of line
                                                line,
                                                line_len, // end of line
                                                editor.buffer().version(),
                                                diagnostics,
                                            );
                                        }
                                        LspAction::RenameSymbol(new_name) => {
                                            let _ = mlsp.rename(
                                                &path,
                                                line,
                                                col,
                                                new_name,
                                                editor.buffer().version(),
                                            );
                                        }
                                    }
                                }
                            } else {
                                editor.set_status("No file path for LSP");
                            }
                        } else {
                            editor.set_status("LSP not available");
                        }
                    }

                    // Handle pending Copilot actions
                    if let Some(action) = editor.pending_copilot_action.take() {
                        if let Some(ref mut cop) = copilot {
                            match action {
                                CopilotAction::Auth => {
                                    if let Err(e) = cop.sign_in() {
                                        editor.set_status(format!("Copilot auth error: {}", e));
                                    } else {
                                        editor.set_status("Copilot: Check for sign-in prompt...");
                                    }
                                }
                                CopilotAction::SignOut => {
                                    if let Err(e) = cop.sign_out() {
                                        editor.set_status(format!("Copilot sign-out error: {}", e));
                                    } else {
                                        editor.set_status("Copilot: Signed out");
                                    }
                                }
                                CopilotAction::Status => {
                                    editor.set_status(cop.status_string());
                                }
                                CopilotAction::Toggle => {
                                    cop.toggle();
                                    if cop.is_enabled() {
                                        editor.set_status("Copilot: Enabled");
                                    } else {
                                        editor.set_status("Copilot: Disabled");
                                        editor.copilot_ghost = None;
                                    }
                                }
                                CopilotAction::Accept => {
                                    // Accept the current Copilot completion
                                    if let Some(completion) = cop.accept_completion() {
                                        apply_copilot_completion(&mut editor, &completion);
                                        editor.copilot_ghost = None;
                                    }
                                }
                                CopilotAction::CycleNext => {
                                    cop.cycle_next();
                                    sync_copilot_ghost(&mut editor, cop);
                                }
                                CopilotAction::CyclePrev => {
                                    cop.cycle_prev();
                                    sync_copilot_ghost(&mut editor, cop);
                                }
                                CopilotAction::Dismiss => {
                                    cop.reject_completions();
                                    editor.copilot_ghost = None;
                                }
                            }
                        } else {
                            editor.set_status("Copilot not available");
                        }
                    }

                    // Check if the current file has changed (e.g., opened from finder)
                    // If so, notify LSP with did_close for old file and did_open for new file
                    let current_file = editor.buffer().path.clone();
                    if current_file != lsp_current_file {
                        if let Some(ref mut mlsp) = multi_lsp {
                            // Close the old file if we had one
                            if let Some(ref old_path) = lsp_current_file {
                                let _ = mlsp.did_close(old_path);
                                lsp_current_file = None;
                            }

                            // Try to start server for new file type and open the file
                            if let Some(ref new_path) = current_file {
                                // Ensure server is started for this file type
                                match mlsp.ensure_server_for_file(new_path) {
                                    Ok(Some(_lang)) => {
                                        editor
                                            .set_lsp_status(mlsp.status(Some(new_path.as_path())));
                                    }
                                    Ok(None) => {
                                        // No LSP for this file type
                                        editor
                                            .set_lsp_status(mlsp.status(Some(new_path.as_path())));
                                    }
                                    Err(e) => {
                                        let message = mlsp
                                            .language_for_path(new_path)
                                            .map(|lang| {
                                                mlsp.user_facing_error(lang, &e.to_string())
                                            })
                                            .unwrap_or_else(|| e.to_string());
                                        editor.set_lsp_status(format!("LSP: {}", message));
                                    }
                                }

                                // If server is ready, send did_open
                                if mlsp.is_ready_for_file(new_path) {
                                    let text = editor.buffer().content();
                                    if let Err(e) = mlsp.did_open(new_path, &text) {
                                        editor.set_lsp_status(format!("LSP: open error: {}", e));
                                    } else {
                                        lsp_current_file = Some(new_path.clone());
                                    }
                                }
                            } else {
                                lsp_current_file = None;
                            }
                        }
                    }

                    // Track file changes for Copilot and send did_open/did_close
                    // Only update copilot_current_file when did_open is actually sent
                    let copilot_file = editor.buffer().path.clone();
                    if copilot_file != copilot_current_file {
                        if let Some(ref mut cop) = copilot {
                            if cop.status == CopilotStatus::Ready {
                                // Close the old file if we had one
                                if let Some(ref old_path) = copilot_current_file {
                                    let old_uri = lsp::path_to_uri(old_path);
                                    let _ = cop.did_close(&old_uri);
                                }

                                // Open the new file
                                if let Some(ref new_path) = copilot_file {
                                    let uri = lsp::path_to_uri(new_path);
                                    let text = editor.buffer().content();
                                    let version = editor.buffer().version() as i32;
                                    let lang_id = LanguageId::from_path(new_path)
                                        .map(|l| l.as_lsp_id().to_string())
                                        .unwrap_or_else(|| "plaintext".to_string());
                                    let _ = cop.did_open(&uri, &lang_id, version, &text);
                                }
                                // Only update tracking when we actually sent did_open
                                copilot_current_file = copilot_file;
                            }
                        }
                    }

                    let new_version = editor.buffer().version();
                    if new_version != prev_version {
                        editor.note_buffer_change();

                        // Schedule autosave if enabled (AfterDelay mode)
                        if autosave_mode == AutosaveMode::AfterDelay {
                            autosave_pending = Some(Instant::now() + autosave_delay);
                        }

                        // Clone path once for reuse in LSP and Copilot notifications
                        let current_buffer_path = editor.buffer().path.clone();

                        // Send document change to LSP (only if ready for this file type)
                        if let Some(ref mut mlsp) = multi_lsp {
                            if let Some(ref path) = current_buffer_path {
                                if mlsp.is_ready_for_file(path) {
                                    let t_lsp = Instant::now();
                                    let text = editor.buffer().content();
                                    profile!(
                                        profile_file,
                                        "buffer.content() for LSP: {:?}",
                                        t_lsp.elapsed()
                                    );
                                    let t_send = Instant::now();
                                    let _ = mlsp.did_change(path, &text);
                                    profile!(
                                        profile_file,
                                        "mlsp.did_change: {:?}",
                                        t_send.elapsed()
                                    );
                                }
                            }
                        }

                        // Send document change to Copilot
                        if let Some(ref mut cop) = copilot {
                            if cop.status == CopilotStatus::Ready {
                                if let Some(ref path) = current_buffer_path {
                                    let t_cop = Instant::now();
                                    let uri = lsp::path_to_uri(path);
                                    let text = editor.buffer().content();
                                    let version = editor.buffer().version() as i32;
                                    let _ = cop.did_change(&uri, version, &text);
                                    profile!(
                                        profile_file,
                                        "copilot.did_change: {:?}",
                                        t_cop.elapsed()
                                    );
                                }
                            }
                        }
                        // Continue LSP triggers
                        if let Some(ref mut mlsp) = multi_lsp {
                            if let Some(ref path) = current_buffer_path {
                                if mlsp.is_ready_for_file(path) {
                                    // Check for auto-completion triggers (. or :: or word chars)
                                    // Use debouncing to avoid flooding LSP with requests
                                    // Don't re-trigger if completion popup is already active (preserves resolved docs)
                                    // Explicit triggers like Ctrl+Space or isIncomplete refresh bypass this
                                    if editor.mode == Mode::Insert
                                        && !editor.completion.active
                                        && should_trigger_completion(&editor)
                                    {
                                        completion_pending = Some((
                                            Instant::now(),
                                            path.clone(),
                                            editor.cursor.line as u32,
                                            editor_lsp_cursor_col(&editor),
                                        ));
                                    }

                                    // Check for signature help triggers (( or ,)
                                    if editor.mode == Mode::Insert
                                        && should_trigger_signature_help(&editor)
                                    {
                                        let _ = mlsp.signature_help(
                                            &path,
                                            editor.cursor.line as u32,
                                            editor_lsp_cursor_col(&editor),
                                        );
                                    }
                                }
                            }
                        }

                        // Request Copilot completions (with debouncing)
                        if let Some(ref mut cop) = copilot {
                            if cop.is_enabled() && cop.status == CopilotStatus::Ready {
                                // Clear stale ghost text if cursor moved to different line
                                // or moved BEFORE the trigger column (typing backwards/deleting)
                                let is_stale = cop.ghost_text.as_ref().map_or(false, |g| {
                                    g.trigger_line != editor.cursor.line
                                        || editor.cursor.col < g.trigger_col
                                });
                                if is_stale {
                                    cop.reject_completions();
                                    editor.copilot_ghost = None;
                                }

                                // Check if we should request new completions
                                // Request when: in insert mode and no current ghost text
                                // Note: We allow requests even with LSP popup active (they can coexist)
                                let ghost_renderable =
                                    cop.ghost_text.as_ref().map_or(false, |ghost| {
                                        if !ghost.visible {
                                            return false;
                                        }
                                        if ghost.trigger_line != editor.cursor.line
                                            || editor.cursor.col < ghost.trigger_col
                                        {
                                            return false;
                                        }
                                        ghost
                                            .current()
                                            .and_then(|completion| {
                                                copilot_inline_completion(&editor, completion)
                                            })
                                            .map(|(inline, _)| !inline.is_empty())
                                            .unwrap_or(false)
                                    });
                                let should_request =
                                    editor.mode == Mode::Insert && !ghost_renderable;

                                if should_request {
                                    let now = Instant::now();
                                    let can_request = match copilot_last_request {
                                        Some(last) => now.duration_since(last) >= copilot_debounce,
                                        None => true,
                                    };

                                    if can_request {
                                        if let Some(path) = editor.buffer().path.clone() {
                                            // Get current line content for UTF-16 conversion
                                            let line_content = editor
                                                .buffer()
                                                .line(editor.cursor.line)
                                                .map(|l| l.to_string())
                                                .unwrap_or_default();

                                            // Get full source content (required by Copilot)
                                            let source = editor.buffer().content();

                                            // Get language ID
                                            let lang_id = LanguageId::from_path(&path)
                                                .map(|l| l.as_lsp_id().to_string())
                                                .unwrap_or_else(|| "plaintext".to_string());

                                            // Get relative path
                                            let relative_path = editor
                                                .project_root
                                                .as_ref()
                                                .and_then(|root| path.strip_prefix(root).ok())
                                                .map(|p| p.to_string_lossy().to_string())
                                                .unwrap_or_else(|| {
                                                    path.to_string_lossy().to_string()
                                                });

                                            let uri = lsp::path_to_uri(&path);
                                            let version = editor.buffer().version() as i32;

                                            let _ = cop.request_completions_with_line(
                                                &uri,
                                                version,
                                                editor.cursor.line,
                                                editor.cursor.col,
                                                &line_content,
                                                &source,
                                                &lang_id,
                                                &relative_path,
                                                4,    // tab_size
                                                true, // insert_spaces
                                            );
                                            copilot_last_request = Some(now);
                                        }
                                    }
                                }
                            }

                            // Update editor ghost text from Copilot state
                            // Note: Ghost text can coexist with LSP completion popup
                            sync_copilot_ghost(&mut editor, cop);
                        }

                        // Check if signature help should be dismissed
                        if editor.mode == Mode::Insert && should_dismiss_signature_help(&editor) {
                            editor.signature_help = None;
                        }
                    }

                    // Handle isIncomplete: re-request completions if filter text changed
                    if editor.needs_completion_refresh {
                        editor.needs_completion_refresh = false;
                        if let Some(ref mut mlsp) = multi_lsp {
                            if let Some(path) = editor.buffer().path.clone() {
                                if mlsp.is_ready_for_file(&path) {
                                    let _ = mlsp.completion(
                                        &path,
                                        editor.cursor.line as u32,
                                        editor_lsp_cursor_col(&editor),
                                        editor.buffer().version(),
                                    );
                                }
                            }
                        }
                    }

                    editor.maybe_update_syntax();
                    needs_redraw = true;
                    redraw_from_input = true;
                }
                events_processed += 1;
                if !terminal.poll_key(Duration::from_millis(0))? {
                    break;
                }
            }
            // Note: Don't set needs_redraw for events we don't handle (FocusLost, Mouse, etc.)
        } else {
            let t_syntax = Instant::now();
            let syntax_updated = editor.maybe_update_syntax_debounced(debounce);
            if syntax_updated {
                profile!(profile_file, "syntax_update: {:?}", t_syntax.elapsed());
                needs_redraw = true;
            }
        }

        let input_pending = terminal.poll_key(Duration::from_millis(0))?;
        let typing_recently = last_input_at.map_or(false, |t| t.elapsed() < typing_pause);
        let skip_notifications = input_pending || typing_recently;

        // Process LSP notifications (non-blocking). While input is active, keep a
        // small budget so progress/status/hover responses do not freeze behind
        // cursor repeat, but avoid draining a large batch in one frame.
        let lsp_notification_limit = if skip_notifications { Some(4) } else { None };
        {
            if let Some(ref mut mlsp) = multi_lsp {
                let t_lsp_poll = Instant::now();
                let mut lsp_notification_count = 0;
                for (lang, notification) in mlsp.poll_notifications_limited(lsp_notification_limit)
                {
                    lsp_notification_count += 1;
                    match notification {
                        LspNotification::Initialized => {
                            // Update status - server is now ready
                            let current_path = editor.buffer().path.clone();
                            editor.set_lsp_status(
                                mlsp.status(current_path.as_ref().map(|p| p.as_path())),
                            );

                            // Now that this server is ready, send did_open for current file if it matches
                            if let Some(path) = current_path {
                                if mlsp.language_for_path(&path) == Some(lang) {
                                    let text = editor.buffer().content();
                                    if let Err(e) = mlsp.did_open(&path, &text) {
                                        editor.set_lsp_status(format!("LSP: open error: {}", e));
                                    } else {
                                        lsp_current_file = Some(path);
                                    }
                                }
                            }
                            needs_redraw = true;
                        }
                        LspNotification::Error { message } => {
                            // Update status with error
                            editor.set_lsp_status(format!(
                                "LSP: {}",
                                mlsp.user_facing_error(lang, &message)
                            ));
                            needs_redraw = true;
                        }
                        LspNotification::Diagnostics { uri, diagnostics } => {
                            let t_diag = Instant::now();
                            let diag_count = diagnostics.len();
                            // Store diagnostics for rendering
                            let errors = diagnostics
                                .iter()
                                .filter(|d| {
                                    matches!(d.severity, lsp::types::DiagnosticSeverity::Error)
                                })
                                .count();
                            let warnings = diagnostics
                                .iter()
                                .filter(|d| {
                                    matches!(d.severity, lsp::types::DiagnosticSeverity::Warning)
                                })
                                .count();

                            editor.set_diagnostics(uri, diagnostics);

                            if errors > 0 || warnings > 0 {
                                editor.set_lsp_status(format!("LSP: {}E {}W", errors, warnings));
                            } else {
                                editor.set_lsp_status("LSP: ✓");
                            }
                            profile!(
                                profile_file,
                                "diagnostics: {} items in {:?}",
                                diag_count,
                                t_diag.elapsed()
                            );
                            needs_redraw = true;
                        }
                        LspNotification::Completions {
                            items,
                            is_incomplete,
                            request_uri,
                            request_line,
                            request_character,
                            request_version,
                        } => {
                            if lsp_completion_response_matches_current_cursor(
                                &editor,
                                &request_uri,
                                request_version,
                                request_line,
                                request_character,
                            ) {
                                // Show completion popup if we have items (with frecency sorting)
                                if !items.is_empty() {
                                    let t_comp = Instant::now();
                                    let item_count = items.len();
                                    let line = editor.cursor.line;
                                    let col = editor.cursor.col;
                                    // Calculate trigger_col as start of current word, not cursor position
                                    let trigger_col = calculate_word_start(&editor, line, col);
                                    editor.show_completions(
                                        items,
                                        line,
                                        trigger_col,
                                        is_incomplete,
                                    );
                                    profile!(
                                        profile_file,
                                        "completions: {} items in {:?}",
                                        item_count,
                                        t_comp.elapsed()
                                    );

                                    // Immediately apply filter with current prefix
                                    // (user may have typed more characters while waiting for LSP response)
                                    if col > trigger_col {
                                        if let Some(line_content) = editor.buffer().line(line) {
                                            let line_str: String = line_content.chars().collect();
                                            let prefix: String = line_str
                                                .chars()
                                                .skip(trigger_col)
                                                .take(col - trigger_col)
                                                .collect();
                                            editor.update_completion_filter(&prefix);

                                            // Hide if no matches after filtering
                                            if editor.completion.filtered.is_empty() {
                                                editor.completion.hide();
                                            }
                                        }
                                    }
                                } else {
                                    editor.completion.hide();
                                }
                                request_selected_completion_resolve(
                                    &mut editor,
                                    mlsp,
                                    &mut last_resolved_completion,
                                );
                                needs_redraw = true;
                            }
                            // NOTE: Only redraw when we actually process completions, not for stale responses
                        }
                        LspNotification::Definition {
                            locations,
                            request_uri,
                        } => {
                            // Validate response is for current file
                            let current_uri =
                                editor.buffer().path.as_ref().map(|p| lsp::path_to_uri(p));
                            if current_uri.as_ref() != Some(&request_uri) {
                                // Stale response - ignore (no redraw needed)
                                continue;
                            }
                            // Handle go-to-definition with support for multiple locations
                            match locations.len() {
                                0 => {
                                    editor.set_status("No definition found");
                                }
                                1 => {
                                    // Single result - jump directly
                                    let loc = &locations[0];
                                    if let Some(path) = lsp::uri_to_path(&loc.uri) {
                                        // Record current position before jumping
                                        editor.record_jump();
                                        editor.open_file(path)?;
                                        editor.goto_line(loc.line + 1); // LSP is 0-indexed
                                        editor.cursor.col = loc.col;
                                        editor.set_status("Jumped to definition");
                                    }
                                }
                                n => {
                                    // Multiple results - for now just jump to first
                                    // TODO: Show picker when multiple definitions exist
                                    let loc = &locations[0];
                                    if let Some(path) = lsp::uri_to_path(&loc.uri) {
                                        // Record current position before jumping
                                        editor.record_jump();
                                        editor.open_file(path)?;
                                        editor.goto_line(loc.line + 1);
                                        editor.cursor.col = loc.col;
                                        editor.set_status(format!(
                                            "Jumped to definition (1 of {})",
                                            n
                                        ));
                                    }
                                }
                            }
                            needs_redraw = true;
                        }
                        LspNotification::Hover {
                            contents,
                            request_uri,
                            request_line,
                            request_character,
                        } => {
                            // Validate response is for current file
                            let current_uri =
                                editor.buffer().path.as_ref().map(|p| lsp::path_to_uri(p));
                            if current_uri.as_ref() != Some(&request_uri) {
                                // Stale response - wrong file (no redraw needed)
                                continue;
                            }

                            // Validate cursor position hasn't moved too far
                            // Allow some tolerance (user might have moved slightly while waiting)
                            let cursor_line = editor.cursor.line as u32;
                            let cursor_col = editor_lsp_cursor_col(&editor);
                            let line_diff = (cursor_line as i32 - request_line as i32).abs();
                            let col_diff = (cursor_col as i32 - request_character as i32).abs();

                            if line_diff > 2 || (line_diff == 0 && col_diff > 10) {
                                // Cursor moved too far - discard stale hover (no redraw needed)
                                continue;
                            }

                            // Handle hover - show popup with full content
                            match contents {
                                Some(text) => {
                                    editor.hover_content = Some(text);
                                }
                                None => {
                                    editor.set_status("No hover info");
                                    editor.hover_content = None;
                                }
                            }
                            needs_redraw = true;
                        }
                        LspNotification::SignatureHelp {
                            help,
                            request_uri,
                            request_line,
                            request_character: _,
                        } => {
                            // Validate response is for current file
                            let current_uri =
                                editor.buffer().path.as_ref().map(|p| lsp::path_to_uri(p));
                            if current_uri.as_ref() != Some(&request_uri) {
                                // Stale response - wrong file (no redraw needed)
                                continue;
                            }

                            // Validate cursor is still on the same line
                            // Signature help is tied to function call position
                            let cursor_line = editor.cursor.line as u32;
                            if cursor_line != request_line {
                                // Cursor moved to different line - signature help is stale (no redraw needed)
                                continue;
                            }

                            // Store signature help for rendering
                            editor.signature_help = help;
                            needs_redraw = true;
                        }
                        LspNotification::Status { message } => {
                            editor.set_lsp_status(format!("LSP: {}", message));
                            needs_redraw = true;
                        }
                        LspNotification::Progress { .. } => {
                            let current_path = editor.buffer().path.clone();
                            editor.set_lsp_status(
                                mlsp.status(current_path.as_ref().map(|p| p.as_path())),
                            );
                            needs_redraw = true;
                        }
                        LspNotification::Formatting {
                            edits,
                            request_uri,
                            request_version,
                        } => {
                            if !lsp_response_matches_current_buffer(
                                &editor,
                                &request_uri,
                                request_version,
                            ) {
                                // Stale response - ignore
                                editor.pending_format = false;
                                needs_redraw = true;
                                continue;
                            }

                            // Apply formatting edits to the buffer
                            if !edits.is_empty() {
                                editor.apply_text_edits(&edits);
                                editor.set_status(format!(
                                    "Applied {} formatting edits",
                                    edits.len()
                                ));

                                // Send didChange to LSP so it knows about the formatted content
                                if let Some(path) = editor.buffer().path.clone() {
                                    let text = editor.buffer().content();
                                    let _ = mlsp.did_change(&path, &text);
                                }
                            }
                            // Clear the pending format flag
                            editor.pending_format = false;

                            // If save_after_format is set, save the file now
                            if editor.save_after_format {
                                editor.save_after_format = false;
                                match editor.save() {
                                    Ok(()) => {
                                        let msg = if edits.is_empty() {
                                            "Saved".to_string()
                                        } else {
                                            format!("Formatted and saved ({} edits)", edits.len())
                                        };
                                        editor.set_status(msg);
                                    }
                                    Err(e) => {
                                        editor.set_status(format!("Error saving: {}", e));
                                    }
                                }
                            }
                            needs_redraw = true;
                        }
                        LspNotification::References {
                            locations,
                            request_uri,
                        } => {
                            // Validate response is for current file
                            let current_uri =
                                editor.buffer().path.as_ref().map(|p| lsp::path_to_uri(p));
                            if current_uri.as_ref() != Some(&request_uri) {
                                // Stale response - ignore (no redraw needed)
                                continue;
                            }

                            if locations.is_empty() {
                                editor.set_status("No references found");
                            } else if locations.len() == 1 {
                                // Single reference - jump directly
                                let loc = &locations[0];
                                if let Some(path) = lsp::uri_to_path(&loc.uri) {
                                    editor.record_jump();
                                    // Open the file if different
                                    let current_path = editor.buffer().path.clone();
                                    if current_path.as_ref() != Some(&path) {
                                        let _ = editor.open_file(path);
                                    }
                                    editor.goto_line(loc.line + 1);
                                    editor.cursor.col = loc.col;
                                }
                                editor.set_status("1 reference");
                            } else {
                                // Multiple references - show picker
                                editor.show_references_picker(locations);
                            }
                            needs_redraw = true;
                        }
                        LspNotification::CodeActions {
                            actions,
                            request_uri,
                            request_version,
                        } => {
                            if !lsp_response_matches_current_buffer(
                                &editor,
                                &request_uri,
                                request_version,
                            ) {
                                // Stale response - ignore (no redraw needed)
                                continue;
                            }

                            if actions.is_empty() {
                                editor.set_status("No code actions available");
                            } else {
                                // Show code actions picker
                                editor.show_code_actions_picker(actions);
                            }
                            needs_redraw = true;
                        }
                        LspNotification::RenameResult {
                            edits,
                            request_uri,
                            request_version,
                        } => {
                            if !lsp_response_matches_current_buffer(
                                &editor,
                                &request_uri,
                                request_version,
                            ) {
                                // Stale response - ignore (no redraw needed)
                                continue;
                            }

                            if edits.is_empty() {
                                editor.set_status("Rename: no changes needed");
                            } else {
                                // Apply rename edits to all affected files
                                let mut total_edits = 0;
                                let mut files_changed = 0;
                                let mut errors: Vec<String> = Vec::new();

                                for (uri, file_edits) in edits {
                                    if let Some(path) = lsp::uri_to_path(&uri) {
                                        // Check if this is the current file
                                        let is_current =
                                            editor.buffer().path.as_ref() == Some(&path);
                                        if is_current {
                                            editor.apply_text_edits(&file_edits);
                                            total_edits += file_edits.len();
                                            files_changed += 1;
                                        } else {
                                            // Apply edits to other files: read, modify, write
                                            match apply_edits_to_file(&path, &file_edits) {
                                                Ok(edit_count) => {
                                                    total_edits += edit_count;
                                                    files_changed += 1;
                                                }
                                                Err(e) => {
                                                    errors.push(format!(
                                                        "{}: {}",
                                                        path.display(),
                                                        e
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }

                                if !errors.is_empty() {
                                    editor.set_status(format!(
                                        "Rename: {} error(s) - {}",
                                        errors.len(),
                                        errors.first().unwrap_or(&String::new())
                                    ));
                                } else if total_edits > 0 {
                                    editor.set_status(format!(
                                        "Renamed: {} edits in {} file(s)",
                                        total_edits, files_changed
                                    ));
                                    // Send didChange to LSP for current file
                                    if let Some(path) = editor.buffer().path.clone() {
                                        let text = editor.buffer().content();
                                        let _ = mlsp.did_change(&path, &text);
                                    }
                                }
                            }
                            needs_redraw = true;
                        }
                        LspNotification::CompletionResolved {
                            item_id,
                            label,
                            documentation,
                            detail,
                            text_edit,
                            additional_text_edits,
                        } => {
                            editor.update_completion_item_resolution(
                                item_id,
                                &label,
                                documentation,
                                detail,
                                text_edit,
                                additional_text_edits,
                            );
                            needs_redraw = true;
                        }
                    }
                }
                // Log if LSP processing was slow
                let lsp_elapsed = t_lsp_poll.elapsed();
                if lsp_elapsed.as_millis() > 50 || lsp_notification_count > 10 {
                    profile!(
                        profile_file,
                        "LSP_POLL: {:?} notifications={}",
                        lsp_elapsed,
                        lsp_notification_count
                    );
                }
            }
        }

        // Process Copilot notifications (non-blocking)
        if !skip_notifications {
            if let Some(ref mut cop) = copilot {
                let t_copilot_poll = Instant::now();
                let notifications = cop.poll_notifications();
                let copilot_count = notifications.len();
                for notif in notifications {
                    match notif {
                        CopilotNotification::Initialized => {
                            // Server initialized, check auth status
                            needs_redraw = true;
                        }
                        CopilotNotification::AuthStatus(ref auth) => {
                            match auth {
                                AuthStatus::SignedIn { user } => {
                                    editor.set_status(format!("Copilot: Signed in as {}", user));
                                    // Copilot just became ready - send did_open for current file
                                    // This handles the case where the file was opened before Copilot was ready
                                    if let Some(path) = editor.buffer().path.clone() {
                                        let uri = lsp::path_to_uri(&path);
                                        let text = editor.buffer().content();
                                        let version = editor.buffer().version() as i32;
                                        let lang_id = LanguageId::from_path(&path)
                                            .map(|l| l.as_lsp_id().to_string())
                                            .unwrap_or_else(|| "plaintext".to_string());
                                        let _ = cop.did_open(&uri, &lang_id, version, &text);
                                        copilot_current_file = Some(path);
                                    }
                                }
                                AuthStatus::NotSignedIn => {
                                    editor.set_status("Copilot: Run :CopilotAuth to sign in");
                                }
                                AuthStatus::SigningIn => {
                                    editor.set_status("Copilot: Signing in...");
                                }
                                AuthStatus::Failed { message } => {
                                    editor
                                        .set_status(format!("Copilot: Auth failed - {}", message));
                                }
                            }
                            needs_redraw = true;
                        }
                        CopilotNotification::SignInRequired(ref info) => {
                            // Show device code to user
                            editor.set_status(format!(
                                "Copilot: Visit {} and enter code: {}",
                                info.verification_uri, info.user_code
                            ));
                            needs_redraw = true;
                        }
                        CopilotNotification::Completions(_) => {
                            // Update ghost text from Copilot manager
                            // Note: We allow ghost text to coexist with LSP completion popup
                            // (similar to how VSCode shows both simultaneously)
                            sync_copilot_ghost(&mut editor, cop);
                            needs_redraw = true;
                        }
                        CopilotNotification::Error { ref message } => {
                            editor.set_status(format!("Copilot error: {}", message));
                            needs_redraw = true;
                        }
                        CopilotNotification::Status { message } => {
                            // Log status messages (could show in debug mode)
                            let _ = message; // Suppress unused warning
                        }
                    }
                }
                // Log if Copilot processing was slow
                let copilot_elapsed = t_copilot_poll.elapsed();
                if copilot_elapsed.as_millis() > 50 || copilot_count > 5 {
                    profile!(
                        profile_file,
                        "COPILOT_POLL: {:?} notifications={}",
                        copilot_elapsed,
                        copilot_count
                    );
                }
            }
        }

        // Drain PTY output even while hidden so background commands keep flowing.
        let terminal_was_visible = editor.floating_terminal.is_visible();
        if editor.floating_terminal.process_output() {
            if editor.floating_terminal.is_visible() {
                terminal_redraw_pending = true;
            } else if terminal_was_visible {
                terminal_redraw_pending = false;
                needs_redraw = true;
                redraw_from_input = true;
            }
        }

        // Check if we should quit
        if editor.should_quit {
            break;
        }

        // Handle pending external command (like lazygit)
        if let Some(cmd) = editor.pending_external_command.take() {
            if let Err(e) = terminal.run_external_process(&cmd) {
                editor.set_status(format!("Error running command: {}", e));
            }
            editor.refresh_git_state();
            needs_redraw = true;
            continue;
        }

        // Check for leader key timeout
        // If we're in leader mode with a pending action (exact match that's also a prefix),
        // execute it after timeoutlen milliseconds
        if editor.leader_sequence.is_some() {
            if let Some(start) = editor.leader_sequence_start {
                let timeoutlen = Duration::from_millis(editor.settings.keymap.timeoutlen);
                if start.elapsed() >= timeoutlen {
                    if let Some(action) = editor.leader_pending_action.take() {
                        editor.leader_sequence = None;
                        editor.leader_sequence_start = None;
                        editor.clear_status();
                        execute_leader_action(&mut editor, &action);
                        needs_redraw = true;
                    }
                }
            }
        }

        // Check for pending completion requests (debounced)
        if let Some((request_time, ref path, line, col)) = completion_pending {
            if request_time.elapsed() >= completion_debounce {
                // Only send if still in insert mode and cursor hasn't moved significantly
                if editor.mode == Mode::Insert {
                    if let Some(ref mut mlsp) = multi_lsp {
                        if mlsp.is_ready_for_file(path) {
                            // Verify cursor position matches (user might have moved)
                            if editor.cursor.line as u32 == line
                                && editor_lsp_cursor_col(&editor) == col
                            {
                                let _ = mlsp.completion(path, line, col, editor.buffer().version());
                            }
                        }
                    }
                }
                completion_pending = None;
            }
        }

        // Poll for async grep results. The search itself runs off the UI loop;
        // this only applies the newest matching result set when it is ready.
        if editor.finder.poll_grep_search() {
            needs_redraw = true;
        }

        // Check for pending finder preview updates (debounced)
        // This avoids the 10-40ms tree-sitter parsing on every keystroke
        if editor.finder.preview_update_pending {
            // Track when the preview update was first requested
            if preview_pending_since.is_none() {
                preview_pending_since = Some(Instant::now());
            }

            // Only update if debounce time has passed and no input is pending
            if let Some(pending_since) = preview_pending_since {
                if pending_since.elapsed() >= preview_debounce && !input_pending {
                    let t_preview = Instant::now();
                    editor.update_finder_preview();
                    editor.finder.preview_update_pending = false;
                    preview_pending_since = None;
                    needs_redraw = true;
                    profile!(
                        profile_file,
                        "preview_update (debounced): {:?}",
                        t_preview.elapsed()
                    );
                }
            }
        } else {
            // Clear pending time when no update is needed
            preview_pending_since = None;
        }

        // Check for pending grep searches (debounced)
        // This avoids running expensive grep on every keystroke
        if editor.finder.grep_search_pending {
            // Track when the grep search was first requested
            if grep_pending_since.is_none() {
                grep_pending_since = Some(Instant::now());
            }

            // Only search if debounce time has passed and no input is pending
            if let Some(pending_since) = grep_pending_since {
                if pending_since.elapsed() >= grep_debounce && !input_pending {
                    let t_grep = Instant::now();
                    editor.finder.execute_grep_search();
                    grep_pending_since = None;
                    needs_redraw = true;
                    profile!(
                        profile_file,
                        "grep_search (debounced): {:?}",
                        t_grep.elapsed()
                    );
                }
            }
        } else {
            // Clear pending time when no search is needed
            grep_pending_since = None;
        }

        // Check for autosave (only if not in modal/picker and buffer has file path)
        if let Some(scheduled_time) = autosave_pending {
            if Instant::now() >= scheduled_time {
                // Only autosave if:
                // - Not in command mode (user might be typing :w manually)
                // - Finder is not open
                // - File explorer is not open
                // - Buffer has a file path
                // - Buffer is modified
                let is_modal_open = editor.mode == Mode::Command
                    || editor.finder.populated
                    || editor.explorer.visible;

                if !is_modal_open && editor.has_unsaved_changes() && editor.buffer().path.is_some()
                {
                    // Save without formatting to avoid cursor jumping
                    match editor.save() {
                        Ok(()) => {
                            editor.set_status("Autosaved");
                        }
                        Err(e) => {
                            editor.set_status(format!("Autosave failed: {}", e));
                        }
                    }
                    needs_redraw = true;
                }
                // Clear the pending autosave regardless of whether we saved
                autosave_pending = None;
            }
        }
        // Render (debounced to avoid excessive redraws during LSP spam)
        if needs_redraw {
            let now = Instant::now();
            let interval = if redraw_from_input {
                render_interval
            } else {
                lsp_render_interval
            };
            if now.duration_since(last_render) >= interval {
                // Update size before render
                if let Ok((w, h)) = Terminal::size() {
                    editor.set_size(w, h);
                }
                let t_render = Instant::now();
                terminal.render(&editor)?;
                profile!(profile_file, "render: {:?}", t_render.elapsed());
                needs_redraw = false;
                terminal_redraw_pending = false;
                last_render = now;
                redraw_from_input = false;
            }
        } else if terminal_redraw_pending && editor.floating_terminal.is_visible() {
            let now = Instant::now();
            if now.duration_since(last_render) >= render_interval {
                editor.sync_floating_terminal_size();
                let t_render = Instant::now();
                terminal.render_terminal_only(&editor)?;
                profile!(profile_file, "terminal_render: {:?}", t_render.elapsed());
                terminal_redraw_pending = false;
                last_render = now;
                redraw_from_input = false;
            }
        }
    }

    // Shutdown all LSP servers gracefully
    if let Some(mut mlsp) = multi_lsp {
        mlsp.shutdown();
    }

    Ok(())
}

/// Check if we should auto-trigger completion based on the character just typed
fn should_trigger_completion(editor: &Editor) -> bool {
    let col = editor.cursor.col;
    if col == 0 {
        return false;
    }

    // Get the current line
    if let Some(line) = editor.buffer().line(editor.cursor.line) {
        let line_str: String = line.chars().collect();
        let chars: Vec<char> = line_str.chars().collect();

        if col > chars.len() {
            return false;
        }

        let last_char = chars[col - 1];

        // Check for '.' trigger
        if last_char == '.' {
            return true;
        }

        // Check for '::' trigger
        if col >= 2 && chars[col - 2] == ':' && last_char == ':' {
            return true;
        }

        // Auto-trigger on word characters (letters, digits, underscore)
        // if we have at least 1 character of a word prefix
        if is_word_char(last_char) {
            let word_len = word_prefix_length(&chars, col);
            if word_len >= 1 {
                return true;
            }
        }
    }

    false
}

/// Check if a character is a word character (letter, digit, or underscore)
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Get the length of the word prefix ending at the given column
fn word_prefix_length(chars: &[char], col: usize) -> usize {
    let mut len = 0;
    let mut i = col;
    while i > 0 {
        let c = chars[i - 1];
        if is_word_char(c) {
            len += 1;
            i -= 1;
        } else {
            break;
        }
    }
    len
}

/// Calculate the starting column of the current word being typed
/// This is used to position the completion popup and set the correct trigger_col
fn calculate_word_start(editor: &Editor, line_idx: usize, col: usize) -> usize {
    if col == 0 {
        return 0;
    }

    if let Some(line) = editor.buffer().line(line_idx) {
        let line_str: String = line.chars().collect();
        let chars: Vec<char> = line_str.chars().collect();

        if col > chars.len() {
            return col;
        }

        // Check if cursor is right after a trigger character (. or :)
        // In this case, trigger_col should be the cursor position
        if col > 0 && !is_word_char(chars[col - 1]) {
            return col;
        }

        // Walk backwards to find the start of the word
        let mut start = col;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        return start;
    }

    col
}

/// Apply LSP text edits to a file on disk
/// Reads the file, applies edits in reverse order, and writes back
fn apply_edits_to_file(
    path: &std::path::Path,
    edits: &[lsp::types::TextEdit],
) -> anyhow::Result<usize> {
    use std::fs;

    let content = fs::read_to_string(path)?;
    let mut text = ropey::Rope::from_str(&content);

    // Sort edits by position (reverse order) so we can apply from end to start
    let mut sorted_edits: Vec<&lsp::types::TextEdit> = edits.iter().collect();
    sorted_edits.sort_by(|a, b| match b.end_line.cmp(&a.end_line) {
        std::cmp::Ordering::Equal => b.end_col.cmp(&a.end_col),
        other => other,
    });

    // Apply each edit
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

    fs::write(path, text.to_string())?;

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
    let col = utf16_to_utf8_col(line_text, utf16_col as u32).min(line_len);

    line_start + col
}

/// Check if we should auto-trigger signature help based on the character just typed
fn should_trigger_signature_help(editor: &Editor) -> bool {
    let col = editor.cursor.col;
    if col == 0 {
        return false;
    }

    // Get the current line
    if let Some(line) = editor.buffer().line(editor.cursor.line) {
        let line_str: String = line.chars().collect();
        let chars: Vec<char> = line_str.chars().collect();

        // Check for '(' or ',' trigger
        if col > 0 && col <= chars.len() {
            let c = chars[col - 1];
            if c == '(' || c == ',' {
                return true;
            }
        }
    }

    false
}

/// Check if signature help should be dismissed (cursor moved out of function call)
fn should_dismiss_signature_help(editor: &Editor) -> bool {
    let col = editor.cursor.col;

    // Get the current line
    if let Some(line) = editor.buffer().line(editor.cursor.line) {
        let line_str: String = line.chars().collect();
        let chars: Vec<char> = line_str.chars().collect();

        // Check if we just typed ')' - dismiss signature help
        if col > 0 && col <= chars.len() && chars[col - 1] == ')' {
            return true;
        }
    }

    false
}

fn sync_copilot_ghost(editor: &mut Editor, cop: &mut CopilotManager) {
    let ghost_state = match cop.ghost_text.as_ref() {
        Some(ghost_state) if ghost_state.visible => ghost_state,
        _ => {
            editor.copilot_ghost = None;
            return;
        }
    };

    let Some(completion) = ghost_state.current() else {
        editor.copilot_ghost = None;
        cop.ghost_text = None;
        return;
    };

    if let Some((inline, additional)) = copilot_inline_completion(editor, completion) {
        editor.copilot_ghost = Some(CopilotGhostText {
            inline_text: inline,
            additional_lines: additional,
            trigger_line: ghost_state.trigger_line,
            trigger_col: ghost_state.trigger_col,
            count_display: ghost_state.count_display(),
        });
    } else {
        editor.copilot_ghost = None;
        cop.ghost_text = None;
    }
}

fn copilot_inline_completion(
    editor: &Editor,
    completion: &CopilotCompletion,
) -> Option<(String, Vec<String>)> {
    let start_line = completion.range.start.line as usize;
    let _end_line = completion.range.end.line as usize;
    if start_line != editor.cursor.line {
        return None;
    }

    let line_text = editor
        .buffer()
        .line(start_line)
        .map(|l| l.to_string())
        .unwrap_or_default();
    let line_text = line_text.trim_end_matches('\n');

    let start_col = utf16_to_utf8_col(line_text, completion.range.start.character);
    let _end_col = utf16_to_utf8_col(line_text, completion.range.end.character);

    if editor.cursor.col < start_col {
        return None;
    }

    let prefix_len = editor.cursor.col.saturating_sub(start_col);
    let prefix: String = line_text.chars().skip(start_col).take(prefix_len).collect();

    let completion_text = completion.text.as_str();
    let suffix = if completion_text.starts_with(&prefix) {
        completion_text
            .chars()
            .skip(prefix.chars().count())
            .collect()
    } else {
        let prefix_trimmed = prefix.trim_start_matches(|c| c == ' ' || c == '\t');
        let completion_trimmed = completion_text.trim_start_matches(|c| c == ' ' || c == '\t');
        if !completion_trimmed.starts_with(prefix_trimmed) {
            return None;
        }
        completion_trimmed
            .chars()
            .skip(prefix_trimmed.chars().count())
            .collect()
    };

    let suffix: String = suffix;
    if suffix.is_empty() {
        return None;
    }

    let mut lines = suffix.lines();
    let inline = lines.next().unwrap_or("").to_string();
    let additional_lines = lines.map(|s| s.to_string()).collect();

    Some((inline, additional_lines))
}

fn apply_copilot_completion(editor: &mut Editor, completion: &CopilotCompletion) {
    let start_line = completion.range.start.line as usize;
    let end_line = completion.range.end.line as usize;
    if end_line < start_line {
        return;
    }

    let max_line = editor.buffer().len_lines().saturating_sub(1);
    if start_line > max_line || end_line > max_line {
        return;
    }

    let start_line_text = editor
        .buffer()
        .line(start_line)
        .map(|l| l.to_string())
        .unwrap_or_default();
    let start_line_text = start_line_text.trim_end_matches('\n');
    let start_col = utf16_to_utf8_col(start_line_text, completion.range.start.character);

    let end_line_text = editor
        .buffer()
        .line(end_line)
        .map(|l| l.to_string())
        .unwrap_or_default();
    let end_line_text = end_line_text.trim_end_matches('\n');
    let mut end_col = utf16_to_utf8_col(end_line_text, completion.range.end.character);
    if start_line == end_line && editor.cursor.line == start_line && editor.cursor.col > end_col {
        end_col = editor.cursor.col;
    }

    // Force a new undo group so acceptance is a separate step from typing.
    editor
        .undo_stack
        .end_undo_group(editor.cursor.line, editor.cursor.col);
    editor
        .undo_stack
        .begin_undo_group(editor.cursor.line, editor.cursor.col);

    if end_line > start_line || end_col > start_col {
        let deleted_text = if end_col > 0 || end_line > start_line {
            editor.get_range_text(start_line, start_col, end_line, end_col.saturating_sub(1))
        } else {
            String::new()
        };

        if !deleted_text.is_empty() {
            editor
                .undo_stack
                .record_change(nevi::editor::Change::delete(
                    start_line,
                    start_col,
                    deleted_text,
                ));
        }

        editor
            .buffer_mut()
            .delete_range(start_line, start_col, end_line, end_col);
    }

    if !completion.text.is_empty() {
        editor
            .undo_stack
            .record_change(nevi::editor::Change::insert(
                start_line,
                start_col,
                completion.text.clone(),
            ));
        editor
            .buffer_mut()
            .insert_str(start_line, start_col, &completion.text);
    }

    let mut new_line = start_line;
    let mut new_col = start_col;
    for ch in completion.text.chars() {
        if ch == '\n' {
            new_line += 1;
            new_col = 0;
        } else {
            new_col += 1;
        }
    }

    editor.cursor.line = new_line;
    editor.cursor.col = new_col;
    editor.clamp_cursor();
    editor.scroll_to_cursor();
    editor
        .undo_stack
        .end_undo_group(editor.cursor.line, editor.cursor.col);
}

/// Find workspace root by walking up the tree and checking root markers.
fn find_workspace_root(file_path: &Path, root_markers: &[String]) -> PathBuf {
    let mut current = if file_path.is_dir() {
        Some(file_path.to_path_buf())
    } else {
        file_path.parent().map(|p| p.to_path_buf())
    };
    let markers: Vec<&str> = root_markers
        .iter()
        .map(|m| m.as_str())
        .filter(|m| !m.trim().is_empty())
        .collect();

    while let Some(dir) = current {
        if markers.iter().any(|marker| dir.join(marker).exists()) {
            return dir;
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }

    // Fallback to file's directory/current directory.
    if file_path.is_dir() {
        file_path.to_path_buf()
    } else {
        file_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_edits_to_file, diagnostic_to_lsp_offsets, editor_lsp_cursor_col, editor_lsp_line_len,
        lsp_completion_response_matches_current_cursor, lsp_response_matches_current_buffer,
        profile_enabled_from_value,
    };
    use nevi::lsp::types::{Diagnostic, DiagnosticSeverity, TextEdit};
    use nevi::Editor;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos))
    }

    #[test]
    fn apply_edits_to_file_treats_columns_as_utf16_offsets() {
        let tmp = unique_temp_dir("nevi_disk_lsp_edit");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("unicode.txt");
        std::fs::write(&path, "a😀b\n").expect("write file");

        let count = apply_edits_to_file(
            &path,
            &[TextEdit {
                start_line: 0,
                start_col: 3,
                end_line: 0,
                end_col: 4,
                new_text: "X".to_string(),
            }],
        )
        .expect("apply edits");

        assert_eq!(count, 1);
        assert_eq!(std::fs::read_to_string(&path).expect("read file"), "a😀X\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn profiling_is_disabled_unless_env_value_opts_in() {
        assert!(!profile_enabled_from_value(None));
        assert!(!profile_enabled_from_value(Some("")));
        assert!(!profile_enabled_from_value(Some("0")));
        assert!(!profile_enabled_from_value(Some("false")));

        assert!(profile_enabled_from_value(Some("1")));
        assert!(profile_enabled_from_value(Some("true")));
        assert!(profile_enabled_from_value(Some("YES")));
        assert!(profile_enabled_from_value(Some("on")));
    }

    #[test]
    fn lsp_request_columns_use_utf16_offsets() {
        let mut editor = Editor::default();
        editor.buffer_mut().insert_str(0, 0, "a😀b\n");
        editor.cursor.line = 0;
        editor.cursor.col = 2;

        assert_eq!(editor_lsp_cursor_col(&editor), 3);
        assert_eq!(editor_lsp_line_len(&editor, 0), 4);
    }

    #[test]
    fn code_action_diagnostics_are_converted_back_to_utf16_offsets() {
        let mut editor = Editor::default();
        editor.buffer_mut().insert_str(0, 0, "a😀b\n");

        let diagnostic = diagnostic_to_lsp_offsets(
            &editor,
            Diagnostic {
                line: 0,
                end_line: 0,
                col_start: 2,
                col_end: 3,
                severity: DiagnosticSeverity::Error,
                message: "problem".to_string(),
                source: None,
                code: None,
            },
        );

        assert_eq!(diagnostic.col_start, 3);
        assert_eq!(diagnostic.col_end, 4);
    }

    #[test]
    fn lsp_response_context_rejects_stale_buffer_version() {
        let tmp = unique_temp_dir("nevi_lsp_response_context");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("main.tsx");
        std::fs::write(&path, "const a = 1;\n").expect("write file");

        let mut editor = Editor::default();
        editor.open_file(path.clone()).expect("open file");
        let uri = nevi::lsp::path_to_uri(&path);
        let request_version = editor.buffer().version();

        assert!(lsp_response_matches_current_buffer(
            &editor,
            &uri,
            request_version
        ));

        editor.insert_char('x');

        assert!(!lsp_response_matches_current_buffer(
            &editor,
            &uri,
            request_version
        ));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn lsp_completion_context_requires_current_position() {
        let tmp = unique_temp_dir("nevi_lsp_completion_context");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let path = tmp.join("main.tsx");
        std::fs::write(&path, "useE\n").expect("write file");

        let mut editor = Editor::default();
        editor.open_file(path.clone()).expect("open file");
        editor.cursor.line = 0;
        editor.cursor.col = 4;
        let uri = nevi::lsp::path_to_uri(&path);
        let request_version = editor.buffer().version();
        let request_col = editor_lsp_cursor_col(&editor);

        assert!(lsp_completion_response_matches_current_cursor(
            &editor,
            &uri,
            request_version,
            0,
            request_col
        ));

        editor.cursor.col = 3;

        assert!(!lsp_completion_response_matches_current_cursor(
            &editor,
            &uri,
            request_version,
            0,
            request_col
        ));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
