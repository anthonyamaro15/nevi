//! GitHub Copilot integration for nevi
//!
//! Provides ghost text completions using the Copilot language server.
//! The server is a proprietary Node.js application bundled with copilot.lua/copilot.vim.
//!
//! ## Server Acquisition
//!
//! The language-server.js is auto-detected from common installation paths:
//! - ~/.local/share/nvim/lazy/copilot.lua/copilot/js/language-server.js (lazy.nvim)
//! - ~/Library/Application Support/nvim/lazy/copilot.lua/copilot/js/ (lazy.nvim alt)
//! - ~/.local/share/nvim/site/pack/packer/start/copilot.lua/copilot/js/ (packer)
//! - ~/.vscode/extensions/github.copilot-*/dist/language-server.js (VSCode)
//!
//! Users can also specify the path manually in config.

pub mod auth;
pub mod client;
pub mod types;
pub mod utf16;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::thread::{self, JoinHandle};

use anyhow::{anyhow, Result};

/// Debug logging helper - writes to /tmp/copilot_debug.log
fn debug_log(msg: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/copilot_debug.log")
    {
        let _ = writeln!(file, "{}", msg);
    }
}

pub use client::CopilotClient;
pub use types::*;
pub use utf16::*;

use crate::config::CopilotSettings;

/// Manages the Copilot server connection and state
pub struct CopilotManager {
    /// Current status
    pub status: CopilotStatus,
    /// Last error message (for display)
    pub last_error: Option<String>,
    /// Authentication status
    pub auth_status: AuthStatus,
    /// Copilot client (if running)
    client: Option<CopilotClient>,
    /// Channel for receiving notifications from the reader thread
    rx: Option<Receiver<CopilotNotification>>,
    /// Handle to the reader thread for cleanup
    reader_handle: Option<JoinHandle<()>>,
    /// Current ghost text state
    pub ghost_text: Option<GhostTextState>,
    /// Pending completion context (for stale response detection)
    pending_context: Option<CompletionContext>,
    /// Latest completion request ID (used to ignore out-of-order responses)
    pending_request_id: Option<u64>,
    /// Settings
    settings: CopilotSettings,
    /// Sign-in info for device flow
    pub sign_in_info: Option<SignInInfo>,
}

/// Ghost text state for rendering
#[derive(Debug, Clone)]
pub struct GhostTextState {
    /// Completions available
    pub completions: Vec<CopilotCompletion>,
    /// Currently selected completion index
    pub current_index: usize,
    /// Line where ghost text was triggered
    pub trigger_line: usize,
    /// Column where ghost text was triggered (character count)
    pub trigger_col: usize,
    /// Whether ghost text is currently visible
    pub visible: bool,
}

impl GhostTextState {
    /// Get the current completion
    pub fn current(&self) -> Option<&CopilotCompletion> {
        self.completions.get(self.current_index)
    }

    /// Cycle to the next completion
    pub fn next(&mut self) {
        if !self.completions.is_empty() {
            self.current_index = (self.current_index + 1) % self.completions.len();
        }
    }

    /// Cycle to the previous completion
    pub fn prev(&mut self) {
        if !self.completions.is_empty() {
            self.current_index = if self.current_index == 0 {
                self.completions.len() - 1
            } else {
                self.current_index - 1
            };
        }
    }

    /// Get ghost text lines for rendering
    /// Returns (inline_text, additional_lines) where:
    /// - inline_text: text to show after cursor on current line
    /// - additional_lines: virtual lines to show below
    pub fn ghost_lines(&self) -> Option<(String, Vec<String>)> {
        let completion = self.current()?;
        let mut lines: Vec<&str> = completion.display_text.lines().collect();

        if lines.is_empty() {
            return None;
        }

        let inline_text = lines.remove(0).to_string();
        let additional_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();

        Some((inline_text, additional_lines))
    }

    /// Get completion count display string (e.g., "1/3")
    pub fn count_display(&self) -> String {
        if self.completions.len() <= 1 {
            String::new()
        } else {
            format!("({}/{})", self.current_index + 1, self.completions.len())
        }
    }
}

/// Context for tracking completion requests (stale response detection)
#[derive(Debug, Clone, PartialEq)]
struct CompletionContext {
    uri: String,
    version: i32,
    line: usize,
    col: usize,
}

impl CopilotManager {
    /// Create a new CopilotManager with the given settings
    pub fn new(settings: CopilotSettings) -> Self {
        Self {
            status: CopilotStatus::Stopped,
            last_error: None,
            auth_status: AuthStatus::NotSignedIn,
            client: None,
            rx: None,
            reader_handle: None,
            ghost_text: None,
            pending_context: None,
            pending_request_id: None,
            settings,
            sign_in_info: None,
        }
    }

    /// Start the Copilot server
    pub fn start(&mut self) -> Result<()> {
        if self.client.is_some() {
            return Ok(()); // Already running
        }

        self.status = CopilotStatus::Starting;

        // Find Node.js
        let node_path = self.find_node()?;

        // Find server
        let server_path = self.find_server()?;

        // Spawn client
        let (mut client, pending, stdin) = CopilotClient::spawn(&node_path, &server_path)?;

        // Get stdout for reader thread
        let stdout = client
            .take_stdout()
            .ok_or_else(|| anyhow!("Failed to get stdout"))?;

        // Create notification channel
        let (tx, rx) = channel();
        self.rx = Some(rx);

        // Start reader thread and store handle for cleanup
        let reader_handle = thread::spawn(move || {
            client::read_messages(stdout, tx, pending, stdin);
        });
        self.reader_handle = Some(reader_handle);

        // Send initialize request
        client.initialize()?;

        self.client = Some(client);
        Ok(())
    }

    /// Stop the Copilot server
    pub fn stop(&mut self) {
        if let Some(mut client) = self.client.take() {
            let _ = client.shutdown();
        }
        // Wait for reader thread to finish (it will exit when stdout closes)
        if let Some(handle) = self.reader_handle.take() {
            let _ = handle.join();
        }
        self.rx = None;
        self.status = CopilotStatus::Stopped;
        self.ghost_text = None;
        self.pending_context = None;
        self.pending_request_id = None;
    }

    /// Poll for notifications from the server (non-blocking)
    pub fn poll_notifications(&mut self) -> Vec<CopilotNotification> {
        let mut notifications = Vec::new();

        if let Some(rx) = &self.rx {
            while let Ok(notif) = rx.try_recv() {
                notifications.push(notif);
            }
        }

        // Process notifications
        for notif in &notifications {
            self.handle_notification(notif.clone());
        }

        notifications
    }

    /// Handle a notification and update state
    fn handle_notification(&mut self, notif: CopilotNotification) {
        match notif {
            CopilotNotification::Initialized => {
                self.status = CopilotStatus::Starting;
                // Send initialized notification and check status
                if let Some(client) = &mut self.client {
                    let _ = client.initialized();
                    let _ = client.did_change_configuration(&CopilotConfiguration {
                        enable_auto_completions: self.settings.auto_trigger,
                        disabled_languages: self.settings.disabled_languages.clone(),
                    });
                    let _ = client.check_status();
                }
            }
            CopilotNotification::AuthStatus(auth) => {
                self.auth_status = auth.clone();
                match &auth {
                    AuthStatus::SignedIn { .. } => {
                        self.status = CopilotStatus::Ready;
                        self.sign_in_info = None;
                    }
                    AuthStatus::NotSignedIn => {
                        self.status = CopilotStatus::SignInRequired;
                    }
                    AuthStatus::SigningIn => {
                        self.status = CopilotStatus::Starting;
                    }
                    AuthStatus::Failed { .. } => {
                        self.status = CopilotStatus::SignInRequired;
                    }
                }
            }
            CopilotNotification::SignInRequired(info) => {
                self.sign_in_info = Some(info);
                self.status = CopilotStatus::SignInRequired;
            }
            CopilotNotification::Completions(result) => {
                let Some(pending_id) = self.pending_request_id else {
                    debug_log("RESPONSE: Ignoring completions with no pending request");
                    return;
                };
                if result.request_id != 0 && result.request_id != pending_id {
                    debug_log(&format!(
                        "RESPONSE: Ignoring completions for stale request_id={} (latest={})",
                        result.request_id, pending_id
                    ));
                    return;
                }
                // Clear pending request once we accept this response.
                self.pending_request_id = None;
                debug_log(&format!(
                    "RESPONSE: Completions received, count={}",
                    result.completions.len()
                ));
                if !result.completions.is_empty() {
                    // Log first completion for debugging
                    if let Some(first) = result.completions.first() {
                        debug_log(&format!(
                            "  First completion: display_text={:?}",
                            first.display_text.chars().take(50).collect::<String>()
                        ));
                    }

                    // Notify server that completion was shown
                    if let Some(client) = &mut self.client {
                        if let Some(first) = result.completions.first() {
                            let _ = client.notify_shown(&first.uuid);
                        }
                    }

                    let ctx = match self.pending_context.take() {
                        Some(ctx) => ctx,
                        None => {
                            debug_log("  WARNING: No pending_context, cannot set ghost_text");
                            self.ghost_text = None;
                            return;
                        }
                    };

                    // Store ghost text state
                    debug_log(&format!(
                        "  Setting ghost_text: trigger_line={} trigger_col={}",
                        ctx.line, ctx.col
                    ));
                    self.ghost_text = Some(GhostTextState {
                        completions: result.completions,
                        current_index: 0,
                        trigger_line: ctx.line,
                        trigger_col: ctx.col,
                        visible: true,
                    });
                } else {
                    debug_log("  No completions in response");
                    self.ghost_text = None;
                    self.pending_context = None;
                }
            }
            CopilotNotification::Error { message } => {
                self.status = CopilotStatus::Error;
                self.last_error = Some(message.clone());
            }
            CopilotNotification::Status { message } => {
                // Log status messages (for debugging)
                let _ = message;
            }
        }
    }

    /// Request completions with proper UTF-16 position
    /// Note: Debouncing should be handled by caller (main.rs)
    pub fn request_completions_with_line(
        &mut self,
        uri: &str,
        version: i32,
        line: usize,
        col: usize,
        line_content: &str,
        source: &str,
        language_id: &str,
        relative_path: &str,
        tab_size: u32,
        insert_spaces: bool,
    ) -> Result<()> {
        // Check if enabled
        if !self.settings.enabled {
            return Ok(());
        }

        // Check if language is disabled
        if self.settings.disabled_languages.contains(&language_id.to_string()) {
            return Ok(());
        }

        // Check if server is ready
        if self.status != CopilotStatus::Ready {
            return Ok(());
        }

        // Store context for stale response detection
        self.pending_context = Some(CompletionContext {
            uri: uri.to_string(),
            version,
            line,
            col,
        });

        // Convert column to UTF-16
        let utf16_col = utf8_to_utf16_col(line_content, col);

        // Build document
        let doc = CopilotDocument {
            uri: uri.to_string(),
            version,
            relative_path: relative_path.to_string(),
            insert_spaces,
            tab_size,
            position: CopilotPosition {
                line: line as u32,
                character: utf16_col,
            },
            language_id: language_id.to_string(),
            source: source.to_string(),
        };

        debug_log(&format!(
            "REQUEST: getCompletions uri={} line={} col={} utf16_col={} source_len={}",
            relative_path, line, col, utf16_col, source.len()
        ));

        // Send request
        if let Some(client) = &mut self.client {
            let request_id = client.get_completions(&doc)?;
            self.pending_request_id = Some(request_id);
        }

        Ok(())
    }

    /// Accept the current completion
    pub fn accept_completion(&mut self) -> Option<CopilotCompletion> {
        let ghost = self.ghost_text.take()?;
        self.pending_context = None;
        self.pending_request_id = None;
        let completion = ghost.completions.get(ghost.current_index)?.clone();

        // Notify server
        if let Some(client) = &mut self.client {
            let accepted_len = utf16_len(&completion.text);
            let _ = client.notify_accepted(&completion.uuid, accepted_len);

            // Reject other completions
            let rejected: Vec<String> = ghost
                .completions
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != ghost.current_index)
                .map(|(_, c)| c.uuid.clone())
                .collect();
            if !rejected.is_empty() {
                let _ = client.notify_rejected(&rejected);
            }
        }

        Some(completion)
    }

    /// Reject all completions (dismiss ghost text)
    pub fn reject_completions(&mut self) {
        if let Some(ghost) = self.ghost_text.take() {
            if let Some(client) = &mut self.client {
                let uuids: Vec<String> = ghost.completions.iter().map(|c| c.uuid.clone()).collect();
                let _ = client.notify_rejected(&uuids);
            }
        }
        self.pending_context = None;
        self.pending_request_id = None;
    }

    /// Cycle to next completion
    pub fn cycle_next(&mut self) {
        if let Some(ghost) = &mut self.ghost_text {
            ghost.next();
            // Notify server about new shown completion
            if let Some(client) = &mut self.client {
                if let Some(completion) = ghost.current() {
                    let _ = client.notify_shown(&completion.uuid);
                }
            }
        }
    }

    /// Cycle to previous completion
    pub fn cycle_prev(&mut self) {
        if let Some(ghost) = &mut self.ghost_text {
            ghost.prev();
            // Notify server about new shown completion
            if let Some(client) = &mut self.client {
                if let Some(completion) = ghost.current() {
                    let _ = client.notify_shown(&completion.uuid);
                }
            }
        }
    }

    /// Hide ghost text (without rejecting - may show again)
    pub fn hide_ghost_text(&mut self) {
        if let Some(ghost) = &mut self.ghost_text {
            ghost.visible = false;
        }
    }

    /// Show ghost text (if we have completions)
    pub fn show_ghost_text(&mut self) {
        if let Some(ghost) = &mut self.ghost_text {
            ghost.visible = true;
        }
    }

    /// Check if cursor has moved away from trigger position
    pub fn is_context_stale(&self, line: usize, col: usize) -> bool {
        if let Some(ghost) = &self.ghost_text {
            // Stale if moved to different line or before trigger column
            ghost.trigger_line != line || col < ghost.trigger_col
        } else {
            true
        }
    }

    /// Invalidate completions if context changed
    pub fn invalidate_if_stale(&mut self, line: usize, col: usize) {
        if self.is_context_stale(line, col) {
            self.reject_completions();
        }
    }

    /// Initiate sign-in
    pub fn sign_in(&mut self) -> Result<()> {
        if let Some(client) = &mut self.client {
            client.sign_in_initiate()?;
        }
        Ok(())
    }

    /// Confirm sign-in with user code
    pub fn confirm_sign_in(&mut self, user_code: &str) -> Result<()> {
        if let Some(client) = &mut self.client {
            client.sign_in_confirm(user_code)?;
        }
        Ok(())
    }

    /// Sign out
    pub fn sign_out(&mut self) -> Result<()> {
        if let Some(client) = &mut self.client {
            client.sign_out()?;
        }
        auth::clear_token()?;
        self.auth_status = AuthStatus::NotSignedIn;
        self.status = CopilotStatus::SignInRequired;
        Ok(())
    }

    /// Notify that a document was opened
    pub fn did_open(&mut self, uri: &str, language_id: &str, version: i32, text: &str) -> Result<()> {
        if let Some(client) = &mut self.client {
            client.did_open(uri, language_id, version, text)?;
        }
        Ok(())
    }

    /// Notify that a document changed
    pub fn did_change(&mut self, uri: &str, version: i32, text: &str) -> Result<()> {
        if let Some(client) = &mut self.client {
            client.did_change(uri, version, text)?;
        }
        Ok(())
    }

    /// Notify that a document was closed
    pub fn did_close(&mut self, uri: &str) -> Result<()> {
        if let Some(client) = &mut self.client {
            client.did_close(uri)?;
        }
        Ok(())
    }

    /// Toggle Copilot enabled state
    pub fn toggle(&mut self) {
        self.settings.enabled = !self.settings.enabled;
        if !self.settings.enabled {
            self.reject_completions();
        }
    }

    /// Check if Copilot is enabled
    pub fn is_enabled(&self) -> bool {
        self.settings.enabled
    }

    /// Get status string for display
    pub fn status_string(&self) -> String {
        if !self.settings.enabled {
            return "Copilot: Off".to_string();
        }

        match &self.status {
            CopilotStatus::Stopped => "Copilot: Stopped".to_string(),
            CopilotStatus::Starting => "Copilot: Starting...".to_string(),
            CopilotStatus::SignInRequired => "Copilot: Sign-in required".to_string(),
            CopilotStatus::Ready => {
                if let AuthStatus::SignedIn { user } = &self.auth_status {
                    format!("Copilot: {} ✓", user)
                } else {
                    "Copilot: Ready".to_string()
                }
            }
            CopilotStatus::Error => {
                if let Some(ref err) = self.last_error {
                    format!("Copilot: Error - {}", err)
                } else {
                    "Copilot: Error".to_string()
                }
            }
        }
    }

    /// Find Node.js executable
    fn find_node(&self) -> Result<String> {
        // Check config first
        if !self.settings.node_path.is_empty() {
            let path = PathBuf::from(&self.settings.node_path);
            if path.exists() {
                return Ok(self.settings.node_path.clone());
            }
        }

        // Try common paths
        let paths = [
            "/usr/local/bin/node",
            "/usr/bin/node",
            "/opt/homebrew/bin/node",
        ];

        for path in paths {
            if PathBuf::from(path).exists() {
                return Ok(path.to_string());
            }
        }

        // Try PATH
        if let Ok(output) = std::process::Command::new("which").arg("node").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() && PathBuf::from(&path).exists() {
                    return Ok(path);
                }
            }
        }

        Err(anyhow!(
            "Node.js not found. Install Node.js >= 22.0 or set copilot.node_path in config."
        ))
    }

    /// Find Copilot language server
    fn find_server(&self) -> Result<String> {
        // Check config first
        if !self.settings.server_path.is_empty() {
            let path = PathBuf::from(&self.settings.server_path);
            if path.exists() {
                return Ok(self.settings.server_path.clone());
            }
        }

        // Try common installation paths (macOS)
        let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;

        let search_paths = [
            // lazy.nvim (most common)
            home.join(".local/share/nvim/lazy/copilot.lua/copilot/dist/language-server.js"),
            home.join(".local/share/nvim/lazy/copilot.lua/copilot/js/language-server.js"),
            // lazy.nvim alternate location
            home.join("Library/Application Support/nvim/lazy/copilot.lua/copilot/dist/language-server.js"),
            // packer
            home.join(".local/share/nvim/site/pack/packer/start/copilot.lua/copilot/dist/language-server.js"),
            // vim-plug
            home.join(".vim/plugged/copilot.vim/copilot/dist/language-server.js"),
            home.join(".local/share/nvim/plugged/copilot.vim/copilot/dist/language-server.js"),
        ];

        for path in &search_paths {
            if path.exists() {
                return Ok(path.to_string_lossy().to_string());
            }
        }

        // Try VSCode extension (glob for version)
        let vscode_ext = home.join(".vscode/extensions");
        if vscode_ext.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&vscode_ext) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("github.copilot-") {
                        let server = entry.path().join("dist/language-server.js");
                        if server.exists() {
                            return Ok(server.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }

        Err(anyhow!(
            "Copilot server not found. Install copilot.lua/copilot.vim or set copilot.server_path in config."
        ))
    }
}

impl Drop for CopilotManager {
    fn drop(&mut self) {
        self.stop();
    }
}
