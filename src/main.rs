use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};
use nevi::editor::LspAction;
use nevi::lsp;
use nevi::terminal::handle_key;
use nevi::{load_config, AutosaveMode, Editor, LspManager, LspNotification, Mode, Terminal};

fn main() -> anyhow::Result<()> {
    // Load configuration
    let settings = load_config();
    // Store LSP settings before moving settings into editor
    let lsp_enabled = settings.lsp.enabled;
    let lsp_rust_command = settings.lsp.servers.rust.command.clone();
    let lsp_rust_args = settings.lsp.servers.rust.args.clone();
    // Store autosave settings
    let autosave_mode = settings.editor.autosave.clone();
    let autosave_delay = Duration::from_millis(settings.editor.autosave_delay_ms);

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

    // Initialize terminal
    let mut terminal = Terminal::new()?;

    // Get initial size
    let (width, height) = Terminal::size()?;
    editor.set_size(width, height);

    // If we opened a directory, open the file picker
    if open_file_picker {
        editor.open_finder_files();
    }

    // Initialize LSP if enabled
    // Start LSP even in project mode (when opening a directory) so it's ready when files are opened
    let mut lsp_manager: Option<LspManager> = None;
    let mut lsp_current_file: Option<PathBuf> = None; // Track which file LSP knows about
    let mut lsp_ready = false; // Track if LSP has finished initializing

    if lsp_enabled {
        // Determine workspace root - prefer project root, fall back to file's workspace
        let workspace_root = if let Some(ref project_root) = editor.project_root {
            find_workspace_root(project_root)
        } else if let Some(ref path) = initial_file {
            find_workspace_root(path)
        } else {
            env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        };

        // Start LSP with the workspace root
        // NOTE: Don't send did_open yet - wait for Initialized notification first
        match LspManager::start(&lsp_rust_command, &lsp_rust_args, workspace_root.clone()) {
            Ok(mgr) => {
                lsp_manager = Some(mgr);
                editor.set_lsp_status("LSP: starting...");
            }
            Err(e) => {
                editor.set_lsp_status(format!("LSP: failed - {}", e));
            }
        }
    }

    let debounce = Duration::from_millis(200);
    let poll_timeout = Duration::from_millis(16);
    let mut needs_redraw = true;
    let mut lsp_document_version = 1;

    // Autosave state: track when the last edit occurred
    // When autosave_pending is Some, an autosave is scheduled for that time
    let mut autosave_pending: Option<Instant> = None;

    // Main event loop
    loop {
        // Process LSP notifications (non-blocking)
        if let Some(ref mut lsp) = lsp_manager {
            while let Some(notification) = lsp.try_recv() {
                match notification {
                    LspNotification::Initialized => {
                        lsp_ready = true;
                        editor.set_lsp_status("LSP: ready");

                        // Now that LSP is ready, send did_open for current file if it's a Rust file
                        if let Some(path) = editor.buffer().path.clone() {
                            if path.extension().map_or(false, |ext| ext == "rs") {
                                let text = editor.buffer().content();
                                lsp_document_version = 1;
                                if let Err(e) = lsp.did_open(&path, &text) {
                                    editor.set_lsp_status(format!("LSP: open error: {}", e));
                                }
                                lsp_current_file = Some(path);
                            }
                        }
                        needs_redraw = true;
                    }
                    LspNotification::Error { message } => {
                        editor.set_lsp_status(format!("LSP: error - {}", message));
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
                        request_line: _,
                        request_character: _,
                    } => {
                        // Validate response is for current file
                        let current_uri = editor.buffer().path.as_ref()
                            .map(|p| lsp::path_to_uri(p));
                        if current_uri.as_ref() != Some(&request_uri) {
                            // Stale response - ignore
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
                        request_line: _,
                        request_character: _,
                    } => {
                        // Validate response is for current file
                        let current_uri = editor.buffer().path.as_ref()
                            .map(|p| lsp::path_to_uri(p));
                        if current_uri.as_ref() != Some(&request_uri) {
                            // Stale response - ignore
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
                                if path.extension().map_or(false, |ext| ext == "rs") {
                                    lsp_document_version += 1;
                                    let text = editor.buffer().content();
                                    let _ = lsp.did_change(&path, lsp_document_version, &text);
                                }
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
                            for (uri, file_edits) in edits {
                                if let Some(path) = lsp::uri_to_path(&uri) {
                                    // Check if this is the current file
                                    let is_current = editor.buffer().path.as_ref() == Some(&path);
                                    if is_current {
                                        editor.apply_text_edits(&file_edits);
                                        total_edits += file_edits.len();
                                        files_changed += 1;
                                    } else {
                                        // For other files, we need to open, edit, and save them
                                        // This is more complex - for now just note it
                                        editor.set_status(format!(
                                            "Rename affects {} files - only current file modified",
                                            files_changed + 1
                                        ));
                                    }
                                }
                            }
                            if total_edits > 0 {
                                editor.set_status(format!(
                                    "Renamed: {} edits in {} file(s)",
                                    total_edits,
                                    files_changed
                                ));
                                // Send didChange to LSP
                                if let Some(path) = editor.buffer().path.clone() {
                                    if path.extension().map_or(false, |ext| ext == "rs") {
                                        lsp_document_version += 1;
                                        let text = editor.buffer().content();
                                        let _ = lsp.did_change(&path, lsp_document_version, &text);
                                    }
                                }
                            }
                        }
                        needs_redraw = true;
                    }
                }
            }
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

                // Check for manual completion trigger (Ctrl+Space) in insert mode
                let manual_completion = editor.mode == Mode::Insert
                    && key.modifiers == KeyModifiers::CONTROL
                    && key.code == KeyCode::Char(' ');

                if manual_completion {
                    // Request completion from LSP (only if ready)
                    if lsp_ready {
                        if let Some(ref lsp) = lsp_manager {
                            if let Some(path) = editor.buffer().path.clone() {
                                let _ = lsp.completion(
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

                // Handle pending LSP actions (gd, K) - only if LSP is ready
                if let Some(action) = editor.pending_lsp_action.take() {
                    if !lsp_ready {
                        editor.set_status("LSP not ready");
                    } else if let Some(ref lsp) = lsp_manager {
                        if let Some(path) = editor.buffer().path.clone() {
                            let line = editor.cursor.line as u32;
                            let col = editor.cursor.col as u32;
                            match action {
                                LspAction::GotoDefinition => {
                                    let _ = lsp.goto_definition(&path, line, col);
                                }
                                LspAction::Hover => {
                                    let _ = lsp.hover(&path, line, col);
                                }
                                LspAction::Formatting => {
                                    editor.pending_format = true;
                                    let _ = lsp.formatting(&path);
                                }
                                LspAction::FindReferences => {
                                    let _ = lsp.references(&path, line, col);
                                }
                                LspAction::CodeActions => {
                                    // Get diagnostics at cursor position
                                    let diagnostics = editor.all_diagnostics_at_cursor();
                                    let _ = lsp.code_action(
                                        &path,
                                        line,
                                        col,
                                        line,
                                        col,
                                        diagnostics,
                                    );
                                }
                                LspAction::RenameSymbol(new_name) => {
                                    let _ = lsp.rename(&path, line, col, new_name);
                                }
                            }
                        } else {
                            editor.set_status("No file path for LSP");
                        }
                    } else {
                        editor.set_status("LSP not available");
                    }
                }

                // Check if the current file has changed (e.g., opened from finder)
                // If so, notify LSP with did_close for old file and did_open for new file
                // Only do this when LSP is ready (has finished initializing)
                let current_file = editor.buffer().path.clone();
                if lsp_ready && current_file != lsp_current_file {
                    if let Some(ref lsp) = lsp_manager {
                        // Close the old file if we had one
                        if let Some(ref old_path) = lsp_current_file {
                            if old_path.extension().map_or(false, |ext| ext == "rs") {
                                let _ = lsp.did_close(old_path);
                            }
                        }

                        // Open the new file if it's a Rust file
                        if let Some(ref new_path) = current_file {
                            if new_path.extension().map_or(false, |ext| ext == "rs") {
                                let text = editor.buffer().content();
                                lsp_document_version = 1; // Reset version for new file
                                if let Err(e) = lsp.did_open(new_path, &text) {
                                    editor.set_lsp_status(format!("LSP: open error: {}", e));
                                } else {
                                    editor.set_lsp_status("LSP: ready");
                                }
                            } else {
                                // Non-Rust file - clear LSP status
                                editor.set_lsp_status("LSP: (not Rust)");
                            }
                        }
                    }
                    lsp_current_file = current_file.clone();
                }

                let new_version = editor.buffer().version();
                if new_version != prev_version {
                    editor.note_buffer_change();

                    // Schedule autosave if enabled (AfterDelay mode)
                    if autosave_mode == AutosaveMode::AfterDelay {
                        autosave_pending = Some(Instant::now() + autosave_delay);
                    }

                    // Send document change to LSP (only if ready)
                    if lsp_ready {
                        if let Some(ref lsp) = lsp_manager {
                            if let Some(path) = editor.buffer().path.clone() {
                                if path.extension().map_or(false, |ext| ext == "rs") {
                                    lsp_document_version += 1;
                                    let text = editor.buffer().content();
                                    let _ = lsp.did_change(&path, lsp_document_version, &text);

                                    // Check for auto-completion triggers (. or ::)
                                    if editor.mode == Mode::Insert && should_trigger_completion(&editor) {
                                        let _ = lsp.completion(
                                            &path,
                                            editor.cursor.line as u32,
                                            editor.cursor.col as u32,
                                        );
                                    }

                                    // Check for signature help triggers (( or ,)
                                    if editor.mode == Mode::Insert && should_trigger_signature_help(&editor)
                                    {
                                        let _ = lsp.signature_help(
                                            &path,
                                            editor.cursor.line as u32,
                                            editor.cursor.col as u32,
                                        );
                                    }
                                }
                            }
                        }
                    }

                    // Check if signature help should be dismissed (don't need lsp_ready)
                    if editor.mode == Mode::Insert && should_dismiss_signature_help(&editor) {
                        editor.signature_help = None;
                    }
                }

                // Handle isIncomplete: re-request completions if filter text changed
                if lsp_ready && editor.needs_completion_refresh {
                    editor.needs_completion_refresh = false;
                    if let Some(ref lsp) = lsp_manager {
                        if let Some(path) = editor.buffer().path.clone() {
                            if path.extension().map_or(false, |ext| ext == "rs") {
                                let _ = lsp.completion(
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

    // Shutdown LSP gracefully
    if let Some(mut lsp) = lsp_manager {
        lsp.shutdown();
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
