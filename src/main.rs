use std::env;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use nevi::editor::LspAction;
use nevi::lsp;
use nevi::terminal::handle_key;
use nevi::{load_config, Editor, LspManager, LspNotification, Mode, Terminal};

fn main() -> anyhow::Result<()> {
    // Load configuration
    let settings = load_config();
    // Store LSP settings before moving settings into editor
    let lsp_enabled = settings.lsp.enabled;
    let lsp_rust_command = settings.lsp.servers.rust.command.clone();
    let lsp_rust_args = settings.lsp.servers.rust.args.clone();

    // Initialize editor with settings
    let mut editor = Editor::new(settings);

    // Open file from command line argument if provided
    let initial_file = env::args().nth(1).map(PathBuf::from);
    if let Some(ref path) = initial_file {
        editor.open_file(path.clone())?;
    }

    // Initialize terminal
    let mut terminal = Terminal::new()?;

    // Get initial size
    let (width, height) = Terminal::size()?;
    editor.set_size(width, height);

    // Initialize LSP if enabled and we have a Rust file
    let mut lsp_manager: Option<LspManager> = None;
    if lsp_enabled {
        if let Some(ref path) = initial_file {
            if let Some(ext) = path.extension() {
                if ext == "rs" {
                    // Find workspace root (directory with Cargo.toml)
                    let root_path = find_workspace_root(path);
                    match LspManager::start(&lsp_rust_command, &lsp_rust_args, root_path.clone()) {
                        Ok(mgr) => {
                            // Notify LSP that we opened the file
                            let text = editor.buffer().content();
                            if let Err(e) = mgr.did_open(path, &text) {
                                editor.set_lsp_status(format!("LSP: open error: {}", e));
                            }
                            lsp_manager = Some(mgr);
                            editor.set_lsp_status("LSP: starting...");
                        }
                        Err(e) => {
                            editor.set_lsp_status(format!("LSP: failed - {}", e));
                        }
                    }
                }
            }
        }
    }

    let debounce = Duration::from_millis(200);
    let poll_timeout = Duration::from_millis(16);
    let mut needs_redraw = true;
    let mut lsp_document_version = 1;

    // Main event loop
    loop {
        // Process LSP notifications (non-blocking)
        if let Some(ref mut lsp) = lsp_manager {
            while let Some(notification) = lsp.try_recv() {
                match notification {
                    LspNotification::Initialized => {
                        editor.set_lsp_status("LSP: ready");
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
                    } => {
                        // Show completion popup if we have items (with frecency sorting)
                        if !items.is_empty() {
                            let line = editor.cursor.line;
                            let col = editor.cursor.col;
                            editor.show_completions(items, line, col, is_incomplete);
                        } else {
                            editor.completion.hide();
                        }
                        needs_redraw = true;
                    }
                    LspNotification::Definition { locations } => {
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
                    LspNotification::Hover { contents } => {
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
                    LspNotification::SignatureHelp { help } => {
                        // Store signature help for rendering
                        editor.signature_help = help;
                        needs_redraw = true;
                    }
                    LspNotification::Status { message } => {
                        editor.set_lsp_status(format!("LSP: {}", message));
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
                    // Request completion from LSP
                    if let Some(ref lsp) = lsp_manager {
                        if let Some(path) = editor.buffer().path.clone() {
                            let _ = lsp.completion(
                                &path,
                                editor.cursor.line as u32,
                                editor.cursor.col as u32,
                            );
                        }
                    }
                } else {
                    handle_key(&mut editor, key);
                }

                // Handle pending LSP actions (gd, K)
                if let Some(action) = editor.pending_lsp_action.take() {
                    if let Some(ref lsp) = lsp_manager {
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
                            }
                        } else {
                            editor.set_status("No file path for LSP");
                        }
                    } else {
                        editor.set_status("LSP not available");
                    }
                }

                let new_version = editor.buffer().version();
                if new_version != prev_version {
                    editor.note_buffer_change();
                    // Send document change to LSP
                    if let Some(ref lsp) = lsp_manager {
                        if let Some(path) = editor.buffer().path.clone() {
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

                            // Check if signature help should be dismissed
                            if editor.mode == Mode::Insert && should_dismiss_signature_help(&editor)
                            {
                                editor.signature_help = None;
                            }
                        }
                    }
                }

                // Handle isIncomplete: re-request completions if filter text changed
                if editor.needs_completion_refresh {
                    editor.needs_completion_refresh = false;
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

                editor.maybe_update_syntax();
                needs_redraw = true;
            } else {
                needs_redraw = true;
            }
        } else if editor.maybe_update_syntax_debounced(debounce) {
            needs_redraw = true;
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

        // Check for '.' trigger
        if col > 0 && col <= chars.len() && chars[col - 1] == '.' {
            return true;
        }

        // Check for '::' trigger
        if col >= 2 && col <= chars.len() && chars[col - 2] == ':' && chars[col - 1] == ':' {
            return true;
        }
    }

    false
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
    let mut current = file_path.parent().map(|p| p.to_path_buf());
    while let Some(dir) = current {
        if dir.join("Cargo.toml").exists() {
            return dir;
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }
    // Fallback to file's directory
    file_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}
