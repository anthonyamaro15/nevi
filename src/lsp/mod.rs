//! LSP (Language Server Protocol) support for nevi
//!
//! This module provides integration with language servers for features like:
//! - Autocomplete
//! - Go-to-definition
//! - Inline diagnostics
//! - Hover documentation

mod client;
pub mod types;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

pub use client::LspClient;
pub use types::*;

/// Manager for the LSP client thread
pub struct LspManager {
    /// Channel to send requests to the LSP thread
    request_tx: Sender<LspRequest>,
    /// Channel to receive notifications from the LSP thread
    notification_rx: Receiver<LspNotification>,
    /// Handle to the LSP thread
    thread_handle: Option<JoinHandle<()>>,
    /// Current status
    status: LspStatus,
}

impl LspManager {
    /// Start the LSP manager with the given server command
    pub fn start(command: &str, args: &[String], root_path: PathBuf) -> anyhow::Result<Self> {
        let (request_tx, request_rx) = mpsc::channel::<LspRequest>();
        let (notification_tx, notification_rx) = mpsc::channel::<LspNotification>();

        let command = command.to_string();
        let args = args.to_vec();

        let thread_handle = thread::spawn(move || {
            run_lsp_thread(&command, &args, root_path, request_rx, notification_tx);
        });

        Ok(Self {
            request_tx,
            notification_rx,
            thread_handle: Some(thread_handle),
            status: LspStatus::Starting,
        })
    }

    /// Try to receive a notification (non-blocking)
    pub fn try_recv(&mut self) -> Option<LspNotification> {
        match self.notification_rx.try_recv() {
            Ok(notification) => {
                // Update status based on notification
                match &notification {
                    LspNotification::Initialized => self.status = LspStatus::Ready,
                    LspNotification::Error { .. } => self.status = LspStatus::Error,
                    _ => {}
                }
                Some(notification)
            }
            Err(_) => None,
        }
    }

    /// Send a request to the LSP thread
    pub fn send(&self, request: LspRequest) -> anyhow::Result<()> {
        self.request_tx
            .send(request)
            .map_err(|e| anyhow::anyhow!("Failed to send LSP request: {}", e))
    }

    /// Get current status
    pub fn status(&self) -> LspStatus {
        self.status
    }

    /// Check if the LSP is ready
    pub fn is_ready(&self) -> bool {
        self.status == LspStatus::Ready
    }

    /// Shutdown the LSP manager
    pub fn shutdown(&mut self) {
        let _ = self.request_tx.send(LspRequest::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        self.status = LspStatus::Stopped;
    }

    // Helper methods for common operations

    /// Notify that a document was opened
    pub fn did_open(&self, path: &PathBuf, text: &str) -> anyhow::Result<()> {
        let uri = path_to_uri(path);
        let language_id = detect_language(path);
        self.send(LspRequest::DidOpen {
            uri,
            language_id,
            version: 1,
            text: text.to_string(),
        })
    }

    /// Notify that a document changed
    pub fn did_change(&self, path: &PathBuf, version: i32, text: &str) -> anyhow::Result<()> {
        let uri = path_to_uri(path);
        self.send(LspRequest::DidChange {
            uri,
            version,
            text: text.to_string(),
        })
    }

    /// Notify that a document was closed
    pub fn did_close(&self, path: &PathBuf) -> anyhow::Result<()> {
        let uri = path_to_uri(path);
        self.send(LspRequest::DidClose { uri })
    }

    /// Request completions
    pub fn completion(&self, path: &PathBuf, line: u32, character: u32) -> anyhow::Result<()> {
        let uri = path_to_uri(path);
        self.send(LspRequest::Completion {
            uri,
            line,
            character,
        })
    }

    /// Request go-to-definition
    pub fn goto_definition(&self, path: &PathBuf, line: u32, character: u32) -> anyhow::Result<()> {
        let uri = path_to_uri(path);
        self.send(LspRequest::GotoDefinition {
            uri,
            line,
            character,
        })
    }

    /// Request hover
    pub fn hover(&self, path: &PathBuf, line: u32, character: u32) -> anyhow::Result<()> {
        let uri = path_to_uri(path);
        self.send(LspRequest::Hover {
            uri,
            line,
            character,
        })
    }

    /// Request signature help at the given position
    pub fn signature_help(&self, path: &PathBuf, line: u32, character: u32) -> anyhow::Result<()> {
        let uri = path_to_uri(path);
        self.send(LspRequest::SignatureHelp {
            uri,
            line,
            character,
        })
    }
}

impl Drop for LspManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Run the LSP client thread
fn run_lsp_thread(
    command: &str,
    args: &[String],
    root_path: PathBuf,
    request_rx: Receiver<LspRequest>,
    notification_tx: Sender<LspNotification>,
) {
    // Try to spawn the LSP server
    let mut client = match LspClient::spawn(command, args) {
        Ok(c) => c,
        Err(e) => {
            let _ = notification_tx.send(LspNotification::Error {
                message: format!("Failed to start LSP server: {}", e),
            });
            return;
        }
    };

    // Start stdout reader thread
    let stdout = match client.take_stdout() {
        Some(s) => s,
        None => {
            let _ = notification_tx.send(LspNotification::Error {
                message: "Failed to get LSP server stdout".to_string(),
            });
            return;
        }
    };

    // Create tracking channel for request ID -> RequestKind mapping
    let (tracking_tx, tracking_rx) = mpsc::channel::<(u64, RequestKind)>();

    let notification_tx_clone = notification_tx.clone();
    thread::spawn(move || {
        client::read_messages(stdout, notification_tx_clone, tracking_rx);
    });

    // Send initialize request and track it
    match client.initialize(&root_path) {
        Ok(id) => {
            let _ = tracking_tx.send((id, RequestKind::Initialize));
        }
        Err(e) => {
            let _ = notification_tx.send(LspNotification::Error {
                message: format!("Failed to initialize LSP: {}", e),
            });
            return;
        }
    }

    // Process requests
    let mut initialized = false;
    loop {
        match request_rx.recv() {
            Ok(request) => {
                match request {
                    LspRequest::Initialize { .. } => {
                        // Already initialized above
                    }
                    LspRequest::Shutdown => {
                        if let Ok(id) = client.shutdown() {
                            let _ = tracking_tx.send((id, RequestKind::Shutdown));
                        }
                        let _ = client.exit();
                        break;
                    }
                    LspRequest::DidOpen {
                        uri,
                        language_id,
                        version,
                        text,
                    } => {
                        // Send initialized notification if we haven't yet
                        if !initialized {
                            if let Err(e) = client.initialized() {
                                let _ = notification_tx.send(LspNotification::Error {
                                    message: format!("Failed to send initialized: {}", e),
                                });
                            }
                            initialized = true;
                        }
                        if let Err(e) = client.did_open(&uri, &language_id, version, &text) {
                            let _ = notification_tx.send(LspNotification::Error {
                                message: format!("Failed to send didOpen: {}", e),
                            });
                        }
                    }
                    LspRequest::DidChange { uri, version, text } => {
                        if let Err(e) = client.did_change(&uri, version, &text) {
                            let _ = notification_tx.send(LspNotification::Error {
                                message: format!("Failed to send didChange: {}", e),
                            });
                        }
                    }
                    LspRequest::DidClose { uri } => {
                        if let Err(e) = client.did_close(&uri) {
                            let _ = notification_tx.send(LspNotification::Error {
                                message: format!("Failed to send didClose: {}", e),
                            });
                        }
                    }
                    LspRequest::Completion {
                        uri,
                        line,
                        character,
                    } => {
                        match client.completion(&uri, line, character) {
                            Ok(id) => {
                                let _ = tracking_tx.send((id, RequestKind::Completion));
                            }
                            Err(e) => {
                                let _ = notification_tx.send(LspNotification::Error {
                                    message: format!("Failed to request completion: {}", e),
                                });
                            }
                        }
                    }
                    LspRequest::GotoDefinition {
                        uri,
                        line,
                        character,
                    } => {
                        match client.goto_definition(&uri, line, character) {
                            Ok(id) => {
                                let _ = tracking_tx.send((id, RequestKind::Definition));
                            }
                            Err(e) => {
                                let _ = notification_tx.send(LspNotification::Error {
                                    message: format!("Failed to request definition: {}", e),
                                });
                            }
                        }
                    }
                    LspRequest::Hover {
                        uri,
                        line,
                        character,
                    } => {
                        match client.hover(&uri, line, character) {
                            Ok(id) => {
                                let _ = tracking_tx.send((id, RequestKind::Hover));
                            }
                            Err(e) => {
                                let _ = notification_tx.send(LspNotification::Error {
                                    message: format!("Failed to request hover: {}", e),
                                });
                            }
                        }
                    }
                    LspRequest::SignatureHelp {
                        uri,
                        line,
                        character,
                    } => {
                        match client.signature_help(&uri, line, character) {
                            Ok(id) => {
                                let _ = tracking_tx.send((id, RequestKind::SignatureHelp));
                            }
                            Err(e) => {
                                let _ = notification_tx.send(LspNotification::Error {
                                    message: format!("Failed to request signature help: {}", e),
                                });
                            }
                        }
                    }
                }
            }
            Err(_) => {
                // Channel closed, exit thread
                break;
            }
        }
    }
}

/// Convert a file path to a file:// URI
pub fn path_to_uri(path: &PathBuf) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
    format!("file://{}", canonical.display())
}

/// Convert a file:// URI back to a PathBuf
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    uri.strip_prefix("file://").map(PathBuf::from)
}

/// Detect language ID from file extension
fn detect_language(path: &PathBuf) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust".to_string(),
        Some("py") => "python".to_string(),
        Some("js") => "javascript".to_string(),
        Some("ts") => "typescript".to_string(),
        Some("tsx") => "typescriptreact".to_string(),
        Some("jsx") => "javascriptreact".to_string(),
        Some("go") => "go".to_string(),
        Some("c") => "c".to_string(),
        Some("cpp") | Some("cc") | Some("cxx") => "cpp".to_string(),
        Some("h") | Some("hpp") => "cpp".to_string(),
        Some("java") => "java".to_string(),
        Some("rb") => "ruby".to_string(),
        Some("php") => "php".to_string(),
        Some("swift") => "swift".to_string(),
        Some("kt") | Some("kts") => "kotlin".to_string(),
        Some("cs") => "csharp".to_string(),
        Some("lua") => "lua".to_string(),
        Some("zig") => "zig".to_string(),
        Some("toml") => "toml".to_string(),
        Some("json") => "json".to_string(),
        Some("yaml") | Some("yml") => "yaml".to_string(),
        Some("md") => "markdown".to_string(),
        Some("html") => "html".to_string(),
        Some("css") => "css".to_string(),
        Some("scss") => "scss".to_string(),
        Some("sql") => "sql".to_string(),
        Some("sh") | Some("bash") => "shellscript".to_string(),
        _ => "plaintext".to_string(),
    }
}
