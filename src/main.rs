use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};
use nevi::editor::{LspAction, CopilotAction, CopilotGhostText};
use nevi::lsp;
use nevi::terminal::handle_key;
use nevi::{load_config, AutosaveMode, Editor, LanguageId, LspNotification, Mode, MultiLspManager, Terminal};
use nevi::copilot::{utf16_to_utf8_col, AuthStatus, CopilotCompletion, CopilotManager, CopilotNotification, CopilotStatus};

fn main() -> anyhow::Result<()> {
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
        // Determine workspace root - prefer project root, fall back to file's workspace
        let workspace_root = if let Some(ref project_root) = editor.project_root {
            find_workspace_root(project_root)
        } else if let Some(ref path) = initial_file {
            find_workspace_root(path)
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

    // Autosave state: track when the last edit occurred
    // When autosave_pending is Some, an autosave is scheduled for that time
    let mut autosave_pending: Option<Instant> = None;

    // Completion debouncing: delay completion requests to avoid flooding LSP
    // Stores (request_time, path, line, col) - request is sent after debounce period
    let completion_debounce = Duration::from_millis(50);
    let mut completion_pending: Option<(Instant, PathBuf, u32, u32)> = None;

    // Track which completion item we've already requested resolve for
    // (to avoid spamming resolve requests on every key press)
    let mut last_resolved_completion: Option<String> = None;

    // Main event loop
    loop {
        // Process LSP notifications (non-blocking)
        if let Some(ref mut mlsp) = multi_lsp {
            for (lang, notification) in mlsp.poll_notifications() {
                match notification {
                    LspNotification::Initialized => {
                        // Update status - server is now ready
                        let current_path = editor.buffer().path.clone();
                        editor.set_lsp_status(mlsp.status(current_path.as_ref().map(|p| p.as_path())));

                        // Now that this server is ready, send did_open for current file if it matches
                        if let Some(path) = current_path {
                            if LanguageId::from_path(&path) == Some(lang) {
                                let text = editor.buffer().content();
                                if let Err(e) = mlsp.did_open(&path, &text) {
                                    editor.set_lsp_status(format!("LSP: open error: {}", e));
                                }
                                lsp_current_file = Some(path);
                            }
                        }
                        needs_redraw = true;
                    }
                    LspNotification::Error { message } => {
                        // Update status with error
                        editor.set_lsp_status(format!("LSP {}: error - {}", lang.as_lsp_id(), message));
                        needs_redraw = true;
                    }
                    LspNotification::Diagnostics { uri, diagnostics } => {
                        // Store diagnostics for rendering
                        let errors = diagnostics
                            .iter()
                            .filter(|d| matches!(d.severity, lsp::types::DiagnosticSeverity::Error))
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
                        needs_redraw = true;
                    }
                    LspNotification::Completions {
                        items,
                        is_incomplete,
                        request_uri,
                        request_line: _,
                        request_character: _,
                    } => {
                        // Validate response is for current file before applying
                        let current_uri = editor.buffer().path.as_ref()
                            .map(|p| lsp::path_to_uri(p));
                        if current_uri.as_ref() == Some(&request_uri) {
                            // Show completion popup if we have items (with frecency sorting)
                            if !items.is_empty() {
                                let line = editor.cursor.line;
                                let col = editor.cursor.col;
                                // Calculate trigger_col as start of current word, not cursor position
                                let trigger_col = calculate_word_start(&editor, line, col);
                                editor.show_completions(items, line, trigger_col, is_incomplete);

                                // Immediately apply filter with current prefix
                                // (user may have typed more characters while waiting for LSP response)
                                if col > trigger_col {
                                    if let Some(line_content) = editor.buffer().line(line) {
                                        let line_str: String = line_content.chars().collect();
                                        let prefix: String = line_str.chars()
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
                        }
                        // Ignore stale responses for different files
                        needs_redraw = true;
                    }
                    LspNotification::Definition { locations, request_uri } => {
                        // Validate response is for current file
                        let current_uri = editor.buffer().path.as_ref()
                            .map(|p| lsp::path_to_uri(p));
                        if current_uri.as_ref() != Some(&request_uri) {
                            // Stale response - ignore
                            needs_redraw = true;
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
                                    editor.set_status(format!("Jumped to definition (1 of {})", n));
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
                        let current_uri = editor.buffer().path.as_ref()
                            .map(|p| lsp::path_to_uri(p));
                        if current_uri.as_ref() != Some(&request_uri) {
                            // Stale response - wrong file
                            needs_redraw = true;
                            continue;
                        }

                        // Validate cursor position hasn't moved too far
                        // Allow some tolerance (user might have moved slightly while waiting)
                        let cursor_line = editor.cursor.line as u32;
                        let cursor_col = editor.cursor.col as u32;
                        let line_diff = (cursor_line as i32 - request_line as i32).abs();
                        let col_diff = (cursor_col as i32 - request_character as i32).abs();

                        if line_diff > 2 || (line_diff == 0 && col_diff > 10) {
                            // Cursor moved too far - discard stale hover
                            needs_redraw = true;
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
                        let current_uri = editor.buffer().path.as_ref()
                            .map(|p| lsp::path_to_uri(p));
                        if current_uri.as_ref() != Some(&request_uri) {
                            // Stale response - wrong file
                            needs_redraw = true;
                            continue;
                        }

                        // Validate cursor is still on the same line
                        // Signature help is tied to function call position
                        let cursor_line = editor.cursor.line as u32;
                        if cursor_line != request_line {
                            // Cursor moved to different line - signature help is stale
                            needs_redraw = true;
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
                    LspNotification::Formatting { edits, request_uri } => {
                        // Validate response is for current file
                        let current_uri = editor.buffer().path.as_ref()
                            .map(|p| lsp::path_to_uri(p));
                        if current_uri.as_ref() != Some(&request_uri) {
                            // Stale response - ignore
                            editor.pending_format = false;
                            needs_redraw = true;
                            continue;
                        }

                        // Apply formatting edits to the buffer
                        if !edits.is_empty() {
                            editor.apply_text_edits(&edits);
                            editor.set_status(format!("Applied {} formatting edits", edits.len()));

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
                    LspNotification::References { locations, request_uri } => {
                        // Validate response is for current file
                        let current_uri = editor.buffer().path.as_ref()
                            .map(|p| lsp::path_to_uri(p));
                        if current_uri.as_ref() != Some(&request_uri) {
                            needs_redraw = true;
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
                    LspNotification::CodeActions { actions, request_uri } => {
                        // Validate response is for current file
                        let current_uri = editor.buffer().path.as_ref()
                            .map(|p| lsp::path_to_uri(p));
                        if current_uri.as_ref() != Some(&request_uri) {
                            needs_redraw = true;
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
                    LspNotification::RenameResult { edits, request_uri } => {
                        // Validate response is for current file
                        let current_uri = editor.buffer().path.as_ref()
                            .map(|p| lsp::path_to_uri(p));
                        if current_uri.as_ref() != Some(&request_uri) {
                            needs_redraw = true;
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
                                    let is_current = editor.buffer().path.as_ref() == Some(&path);
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
                                                errors.push(format!("{}: {}", path.display(), e));
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
                                    total_edits,
                                    files_changed
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
                    LspNotification::CompletionResolved { label, documentation, detail } => {
                        // Update the completion item with resolved documentation
                        let has_doc = documentation.is_some();
                        let has_detail = detail.is_some();
                        editor.update_completion_item_documentation(&label, documentation, detail);
                        // Always show debug info in status to trace the flow
                        editor.set_status(format!("Resolved '{}': doc={}, detail={}", label, has_doc, has_detail));
                        needs_redraw = true;
                    }
                }
            }
        }

        // Process Copilot notifications (non-blocking)
        if let Some(ref mut cop) = copilot {
            let notifications = cop.poll_notifications();
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
                                editor.set_status(format!("Copilot: Auth failed - {}", message));
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
        }

        // Process floating terminal output if visible
        if editor.floating_terminal.is_visible() {
            editor.floating_terminal.process_output();
        }

        if needs_redraw {
            // Update size before render
            if let Ok((w, h)) = Terminal::size() {
                editor.set_size(w, h);
            }
            terminal.render(&editor)?;
            needs_redraw = false;
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
            needs_redraw = true;
            continue;
        }

        if terminal.poll_key(poll_timeout)? {
            let prev_version = editor.buffer().version();
            if let Some(key) = terminal.read_key()? {
                // Dismiss hover popup on any key press
                editor.hover_content = None;
                // Dismiss diagnostic float on any key press (it can be reopened with gl)
                editor.show_diagnostic_float = false;

                // Check for manual completion trigger (Ctrl+Space) in insert mode
                let manual_completion = editor.mode == Mode::Insert
                    && key.modifiers == KeyModifiers::CONTROL
                    && key.code == KeyCode::Char(' ');

                if manual_completion {
                    // Request completion from LSP (only if ready for this file type)
                    if let Some(ref mut mlsp) = multi_lsp {
                        if let Some(path) = editor.buffer().path.clone() {
                            if mlsp.is_ready_for_file(&path) {
                                let _ = mlsp.completion(
                                    &path,
                                    editor.cursor.line as u32,
                                    editor.cursor.col as u32,
                                );
                            }
                        }
                    }
                } else {
                    handle_key(&mut editor, key);
                }

                // Check if we should resolve completion item documentation
                // Only resolve when selection changes (tracked by last_resolved_label)
                if editor.completion.active {
                    if let Some(item) = editor.completion.selected_item() {
                        let current_label = &item.label;
                        // Resolve if: no documentation AND has raw_data AND not already resolved
                        let should_resolve = item.documentation.is_none()
                            && item.raw_data.is_some()
                            && last_resolved_completion.as_ref() != Some(current_label);

                        if should_resolve {
                            last_resolved_completion = Some(current_label.clone());
                            if let Some(ref mut mlsp) = multi_lsp {
                                if let Some(path) = editor.buffer().path.clone() {
                                    if mlsp.is_ready_for_file(&path) {
                                        let raw_data = item.raw_data.clone().unwrap();
                                        let label = item.label.clone();
                                        // Debug: show when resolve is triggered
                                        editor.set_status(format!("Resolving '{}'...", label));
                                        let _ = mlsp.completion_resolve(&path, raw_data, label);
                                    }
                                }
                            }
                        }
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
                                    editor.set_status(format!("LSP: {}", e));
                                } else {
                                    editor.set_status("LSP starting...");
                                }
                            } else {
                                let line = editor.cursor.line as u32;
                                let col = editor.cursor.col as u32;
                                match action {
                                    LspAction::GotoDefinition => {
                                        let _ = mlsp.goto_definition(&path, line, col);
                                    }
                                    LspAction::Hover => {
                                        let _ = mlsp.hover(&path, line, col);
                                    }
                                    LspAction::Formatting => {
                                        editor.pending_format = true;
                                        let _ = mlsp.formatting(&path);
                                    }
                                    LspAction::FindReferences => {
                                        let _ = mlsp.references(&path, line, col);
                                    }
                                    LspAction::CodeActions => {
                                        // Get diagnostics at cursor position
                                        let diagnostics = editor.all_diagnostics_at_cursor();
                                        let _ = mlsp.code_action(
                                            &path,
                                            line,
                                            col,
                                            line,
                                            col,
                                            diagnostics,
                                        );
                                    }
                                    LspAction::RenameSymbol(new_name) => {
                                        let _ = mlsp.rename(&path, line, col, new_name);
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
                        }

                        // Try to start server for new file type and open the file
                        if let Some(ref new_path) = current_file {
                            // Ensure server is started for this file type
                            match mlsp.ensure_server_for_file(new_path) {
                                Ok(Some(lang)) => {
                                    editor.set_lsp_status(format!("LSP: {} starting...", lang.as_lsp_id()));
                                }
                                Ok(None) => {
                                    // No LSP for this file type
                                    editor.set_lsp_status(mlsp.status(Some(new_path.as_path())));
                                }
                                Err(e) => {
                                    editor.set_lsp_status(format!("LSP: failed - {}", e));
                                }
                            }

                            // If server is ready, send did_open
                            if mlsp.is_ready_for_file(new_path) {
                                let text = editor.buffer().content();
                                if let Err(e) = mlsp.did_open(new_path, &text) {
                                    editor.set_lsp_status(format!("LSP: open error: {}", e));
                                }
                            }
                        }
                    }
                    lsp_current_file = current_file.clone();
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

                    // Send document change to LSP (only if ready for this file type)
                    if let Some(ref mut mlsp) = multi_lsp {
                        if let Some(path) = editor.buffer().path.clone() {
                            if mlsp.is_ready_for_file(&path) {
                                let text = editor.buffer().content();
                                let _ = mlsp.did_change(&path, &text);
                            }
                        }
                    }

                    // Send document change to Copilot
                    if let Some(ref mut cop) = copilot {
                        if cop.status == CopilotStatus::Ready {
                            if let Some(path) = editor.buffer().path.clone() {
                                let uri = lsp::path_to_uri(&path);
                                let text = editor.buffer().content();
                                let version = editor.buffer().version() as i32;
                                let _ = cop.did_change(&uri, version, &text);
                            }
                        }
                    }

                    // Continue LSP triggers
                    if let Some(ref mut mlsp) = multi_lsp {
                        if let Some(path) = editor.buffer().path.clone() {
                            if mlsp.is_ready_for_file(&path) {

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
                                        editor.cursor.col as u32,
                                    ));
                                }

                                // Check for signature help triggers (( or ,)
                                if editor.mode == Mode::Insert && should_trigger_signature_help(&editor)
                                {
                                    let _ = mlsp.signature_help(
                                        &path,
                                        editor.cursor.line as u32,
                                        editor.cursor.col as u32,
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
                                g.trigger_line != editor.cursor.line || editor.cursor.col < g.trigger_col
                            });
                            if is_stale {
                                cop.reject_completions();
                                editor.copilot_ghost = None;
                            }

                            // Check if we should request new completions
                            // Request when: in insert mode and no current ghost text
                            // Note: We allow requests even with LSP popup active (they can coexist)
                            let ghost_renderable = cop.ghost_text.as_ref().map_or(false, |ghost| {
                                if !ghost.visible {
                                    return false;
                                }
                                if ghost.trigger_line != editor.cursor.line || editor.cursor.col < ghost.trigger_col {
                                    return false;
                                }
                                ghost.current()
                                    .and_then(|completion| copilot_inline_completion(&editor, completion))
                                    .map(|(inline, _)| !inline.is_empty())
                                    .unwrap_or(false)
                            });
                            let should_request = editor.mode == Mode::Insert && !ghost_renderable;

                            if should_request {
                                let now = Instant::now();
                                let can_request = match copilot_last_request {
                                    Some(last) => now.duration_since(last) >= copilot_debounce,
                                    None => true,
                                };

                                if can_request {
                                    if let Some(path) = editor.buffer().path.clone() {
                                        // Get current line content for UTF-16 conversion
                                        let line_content = editor.buffer().line(editor.cursor.line)
                                            .map(|l| l.to_string())
                                            .unwrap_or_default();

                                        // Get full source content (required by Copilot)
                                        let source = editor.buffer().content();

                                        // Get language ID
                                        let lang_id = LanguageId::from_path(&path)
                                            .map(|l| l.as_lsp_id().to_string())
                                            .unwrap_or_else(|| "plaintext".to_string());

                                        // Get relative path
                                        let relative_path = editor.project_root.as_ref()
                                            .and_then(|root| path.strip_prefix(root).ok())
                                            .map(|p| p.to_string_lossy().to_string())
                                            .unwrap_or_else(|| path.to_string_lossy().to_string());

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
                                            4, // tab_size
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
                                    editor.cursor.col as u32,
                                );
                            }
                        }
                    }
                }

                editor.maybe_update_syntax();
                needs_redraw = true;
            } else {
                needs_redraw = true;
            }
        } else if editor.maybe_update_syntax_debounced(debounce) {
            needs_redraw = true;
        }

        // Check for pending completion requests (debounced)
        if let Some((request_time, ref path, line, col)) = completion_pending {
            if request_time.elapsed() >= completion_debounce {
                // Only send if still in insert mode and cursor hasn't moved significantly
                if editor.mode == Mode::Insert {
                    if let Some(ref mut mlsp) = multi_lsp {
                        if mlsp.is_ready_for_file(path) {
                            // Verify cursor position matches (user might have moved)
                            if editor.cursor.line as u32 == line && editor.cursor.col as u32 == col {
                                let _ = mlsp.completion(path, line, col);
                            }
                        }
                    }
                }
                completion_pending = None;
            }
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
fn apply_edits_to_file(path: &std::path::Path, edits: &[lsp::types::TextEdit]) -> anyhow::Result<usize> {
    use std::fs;

    // Read the file content
    let content = fs::read_to_string(path)?;
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

    // Handle files that end with newline (lines() strips the trailing newline)
    if content.ends_with('\n') && !lines.is_empty() {
        // Add empty string to represent the trailing newline
    }

    // Sort edits by position (reverse order) so we can apply from end to start
    let mut sorted_edits: Vec<&lsp::types::TextEdit> = edits.iter().collect();
    sorted_edits.sort_by(|a, b| {
        match b.end_line.cmp(&a.end_line) {
            std::cmp::Ordering::Equal => b.end_col.cmp(&a.end_col),
            other => other,
        }
    });

    // Apply each edit
    for edit in &sorted_edits {
        // Delete the range
        if edit.start_line < lines.len() {
            if edit.start_line == edit.end_line {
                // Single line edit
                let line = &mut lines[edit.start_line];
                let start = edit.start_col.min(line.len());
                let end = edit.end_col.min(line.len());
                line.replace_range(start..end, &edit.new_text);
            } else {
                // Multi-line edit
                let start_line_content = if edit.start_line < lines.len() {
                    lines[edit.start_line].chars().take(edit.start_col).collect::<String>()
                } else {
                    String::new()
                };
                let end_line_content = if edit.end_line < lines.len() {
                    lines[edit.end_line].chars().skip(edit.end_col).collect::<String>()
                } else {
                    String::new()
                };

                // Remove the affected lines
                let remove_start = edit.start_line;
                let remove_end = (edit.end_line + 1).min(lines.len());
                lines.drain(remove_start..remove_end);

                // Insert the new content
                let new_content = format!("{}{}{}", start_line_content, edit.new_text, end_line_content);
                let new_lines: Vec<String> = new_content.lines().map(|s| s.to_string()).collect();
                for (i, line) in new_lines.into_iter().enumerate() {
                    lines.insert(edit.start_line + i, line);
                }
            }
        }
    }

    // Write back to file
    let new_content = lines.join("\n");
    // Preserve trailing newline if original had one
    let final_content = if content.ends_with('\n') && !new_content.ends_with('\n') {
        format!("{}\n", new_content)
    } else {
        new_content
    };
    fs::write(path, final_content)?;

    Ok(sorted_edits.len())
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
    let end_line = completion.range.end.line as usize;
    if start_line != editor.cursor.line {
        return None;
    }

    let line_text = editor.buffer().line(start_line)
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
        completion_text.chars().skip(prefix.chars().count()).collect()
    } else {
        let prefix_trimmed = prefix.trim_start_matches(|c| c == ' ' || c == '\t');
        let completion_trimmed = completion_text.trim_start_matches(|c| c == ' ' || c == '\t');
        if !completion_trimmed.starts_with(prefix_trimmed) {
            return None;
        }
        completion_trimmed.chars().skip(prefix_trimmed.chars().count()).collect()
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

    let start_line_text = editor.buffer().line(start_line)
        .map(|l| l.to_string())
        .unwrap_or_default();
    let start_line_text = start_line_text.trim_end_matches('\n');
    let start_col = utf16_to_utf8_col(start_line_text, completion.range.start.character);

    let end_line_text = editor.buffer().line(end_line)
        .map(|l| l.to_string())
        .unwrap_or_default();
    let end_line_text = end_line_text.trim_end_matches('\n');
    let mut end_col = utf16_to_utf8_col(end_line_text, completion.range.end.character);
    if start_line == end_line && editor.cursor.line == start_line && editor.cursor.col > end_col {
        end_col = editor.cursor.col;
    }

    // Force a new undo group so acceptance is a separate step from typing.
    editor.undo_stack.end_undo_group(editor.cursor.line, editor.cursor.col);
    editor.undo_stack.begin_undo_group(editor.cursor.line, editor.cursor.col);

    if end_line > start_line || end_col > start_col {
        let deleted_text = if end_col > 0 || end_line > start_line {
            editor.get_range_text(
                start_line,
                start_col,
                end_line,
                end_col.saturating_sub(1),
            )
        } else {
            String::new()
        };

        if !deleted_text.is_empty() {
            editor.undo_stack.record_change(nevi::editor::Change::delete(
                start_line,
                start_col,
                deleted_text,
            ));
        }

        editor.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
    }

    if !completion.text.is_empty() {
        editor.undo_stack.record_change(nevi::editor::Change::insert(
            start_line,
            start_col,
            completion.text.clone(),
        ));
        editor.buffer_mut().insert_str(start_line, start_col, &completion.text);
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
    editor.undo_stack.end_undo_group(editor.cursor.line, editor.cursor.col);
}

/// Find the workspace root by looking for Cargo.toml
fn find_workspace_root(file_path: &PathBuf) -> PathBuf {
    let mut current = if file_path.is_dir() {
        Some(file_path.clone())
    } else {
        file_path.parent().map(|p| p.to_path_buf())
    };
    while let Some(dir) = current {
        if dir.join("Cargo.toml").exists() {
            return dir;
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }
    // Fallback to file's directory
    if file_path.is_dir() {
        file_path.clone()
    } else {
        file_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }
}
