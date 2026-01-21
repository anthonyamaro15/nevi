//! LSP client implementation using JSON-RPC over stdio

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use lsp_types::{
    ClientCapabilities, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, GotoDefinitionParams,
    HoverParams, InitializeParams, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams,
    VersionedTextDocumentIdentifier, WorkspaceFolder,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::types::{
    CodeActionItem, CompletionItem, CompletionKind, Diagnostic, DiagnosticSeverity, Location,
    LspNotification, ParameterInfo, RequestKind, SignatureHelpResult, SignatureInfo, TextEdit,
};

/// Shared pending requests map - maps request ID to request kind
/// This is shared between the request sender and response reader threads
pub type PendingRequests = Arc<Mutex<HashMap<u64, RequestKind>>>;
pub type SharedStdin = Arc<Mutex<ChildStdin>>;

/// JSON-RPC request message
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

/// JSON-RPC notification (no id, no response expected)
#[derive(Debug, Serialize)]
struct JsonRpcNotification {
    jsonrpc: &'static str,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
enum JsonRpcId {
    Num(u64),
    Str(String),
}

/// JSON-RPC response
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<JsonRpcId>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
    method: Option<String>,
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponseOut {
    jsonrpc: &'static str,
    id: JsonRpcId,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcErrorOut>,
}

#[derive(Debug, Serialize)]
struct JsonRpcErrorOut {
    code: i64,
    message: String,
}

/// LSP client that communicates with a language server
pub struct LspClient {
    process: Child,
    stdin: SharedStdin,
    request_id: AtomicU64,
    /// Shared pending requests map - also used by the response reader thread
    pending_requests: PendingRequests,
}

impl LspClient {
    /// Spawn a new LSP server process
    pub fn spawn(command: &str, args: &[String]) -> Result<(Self, PendingRequests, SharedStdin)> {
        let mut process = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn LSP server '{}': {}", command, e))?;

        let stdin = process.stdin.take().ok_or_else(|| anyhow!("Failed to get stdin"))?;
        let stdin = Arc::new(Mutex::new(stdin));
        let stdin_clone = stdin.clone();

        // Create the shared pending requests map
        let pending_requests = Arc::new(Mutex::new(HashMap::new()));
        let pending_clone = pending_requests.clone();

        Ok((
            Self {
                process,
                stdin,
                request_id: AtomicU64::new(1),
                pending_requests,
            },
            pending_clone,
            stdin_clone,
        ))
    }

    /// Get stdout for reading responses
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.process.stdout.take()
    }

    /// Get stderr for reading error output
    pub fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.process.stderr.take()
    }

    /// Send initialize request
    pub fn initialize(&mut self, root_path: &std::path::Path) -> Result<u64> {
        let root_uri = lsp_types::Url::from_file_path(root_path)
            .map_err(|_| anyhow!("Failed to convert root path to URI: {}", root_path.display()))?;

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_path: Some(root_path.to_string_lossy().to_string()),
            root_uri: Some(root_uri.clone()),
            initialization_options: None,
            capabilities: ClientCapabilities {
                text_document: Some(lsp_types::TextDocumentClientCapabilities {
                    completion: Some(lsp_types::CompletionClientCapabilities {
                        completion_item: Some(lsp_types::CompletionItemCapability {
                            snippet_support: Some(false),
                            documentation_format: Some(vec![
                                lsp_types::MarkupKind::PlainText,
                                lsp_types::MarkupKind::Markdown,
                            ]),
                            // Tell server we support resolving documentation and detail lazily
                            resolve_support: Some(lsp_types::CompletionItemCapabilityResolveSupport {
                                properties: vec![
                                    "documentation".to_string(),
                                    "detail".to_string(),
                                ],
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    hover: Some(lsp_types::HoverClientCapabilities {
                        content_format: Some(vec![lsp_types::MarkupKind::PlainText]),
                        ..Default::default()
                    }),
                    signature_help: Some(lsp_types::SignatureHelpClientCapabilities {
                        signature_information: Some(lsp_types::SignatureInformationSettings {
                            documentation_format: Some(vec![lsp_types::MarkupKind::PlainText]),
                            parameter_information: Some(lsp_types::ParameterInformationSettings {
                                label_offset_support: Some(true),
                            }),
                            active_parameter_support: Some(true),
                        }),
                        ..Default::default()
                    }),
                    definition: Some(lsp_types::GotoCapability {
                        link_support: Some(false),
                        ..Default::default()
                    }),
                    publish_diagnostics: Some(lsp_types::PublishDiagnosticsClientCapabilities {
                        related_information: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            trace: None,
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri.clone(),
                name: root_path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "workspace".to_string()),
            }]),
            client_info: Some(lsp_types::ClientInfo {
                name: "nevi".to_string(),
                version: Some("0.1.0".to_string()),
            }),
            locale: None,
            work_done_progress_params: Default::default(),
        };

        self.send_request("initialize", serde_json::to_value(params)?, RequestKind::Initialize)
    }

    /// Send initialized notification (after initialize response)
    pub fn initialized(&mut self) -> Result<()> {
        self.send_notification("initialized", json!({}))
    }

    /// Send shutdown request
    pub fn shutdown(&mut self) -> Result<u64> {
        self.send_request("shutdown", Value::Null, RequestKind::Shutdown)
    }

    /// Send exit notification
    pub fn exit(&mut self) -> Result<()> {
        self.send_notification("exit", Value::Null)
    }

    /// Notify server that a document was opened
    pub fn did_open(&mut self, uri: &str, language_id: &str, version: i32, text: &str) -> Result<()> {
        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: lsp_types::Url::parse(uri)?,
                language_id: language_id.to_string(),
                version,
                text: text.to_string(),
            },
        };
        self.send_notification("textDocument/didOpen", serde_json::to_value(params)?)
    }

    /// Notify server that a document changed
    pub fn did_change(&mut self, uri: &str, version: i32, text: &str) -> Result<()> {
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: lsp_types::Url::parse(uri)?,
                version,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: text.to_string(),
            }],
        };
        self.send_notification("textDocument/didChange", serde_json::to_value(params)?)
    }

    /// Notify server that a document was closed
    pub fn did_close(&mut self, uri: &str) -> Result<()> {
        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: lsp_types::Url::parse(uri)?,
            },
        };
        self.send_notification("textDocument/didClose", serde_json::to_value(params)?)
    }

    /// Request completions at position
    pub fn completion(&mut self, uri: &str, line: u32, character: u32) -> Result<u64> {
        let params = lsp_types::CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: lsp_types::Url::parse(uri)?,
                },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        };
        self.send_request(
            "textDocument/completion",
            serde_json::to_value(params)?,
            RequestKind::Completion {
                uri: uri.to_string(),
                line,
                character,
            },
        )
    }

    /// Request go-to-definition
    pub fn goto_definition(&mut self, uri: &str, line: u32, character: u32) -> Result<u64> {
        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: lsp_types::Url::parse(uri)?,
                },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.send_request(
            "textDocument/definition",
            serde_json::to_value(params)?,
            RequestKind::Definition {
                uri: uri.to_string(),
                line,
                character,
            },
        )
    }

    /// Request hover information
    pub fn hover(&mut self, uri: &str, line: u32, character: u32) -> Result<u64> {
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: lsp_types::Url::parse(uri)?,
                },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };
        self.send_request(
            "textDocument/hover",
            serde_json::to_value(params)?,
            RequestKind::Hover {
                uri: uri.to_string(),
                line,
                character,
            },
        )
    }

    /// Request signature help at the given position
    pub fn signature_help(&mut self, uri: &str, line: u32, character: u32) -> Result<u64> {
        let params = lsp_types::SignatureHelpParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: lsp_types::Url::parse(uri)?,
                },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            context: None,
        };
        self.send_request(
            "textDocument/signatureHelp",
            serde_json::to_value(params)?,
            RequestKind::SignatureHelp {
                uri: uri.to_string(),
                line,
                character,
            },
        )
    }

    /// Request document formatting
    pub fn formatting(&mut self, uri: &str, tab_size: u32) -> Result<u64> {
        let params = lsp_types::DocumentFormattingParams {
            text_document: TextDocumentIdentifier {
                uri: lsp_types::Url::parse(uri)?,
            },
            options: lsp_types::FormattingOptions {
                tab_size,
                insert_spaces: true,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
        };
        self.send_request(
            "textDocument/formatting",
            serde_json::to_value(params)?,
            RequestKind::Formatting {
                uri: uri.to_string(),
            },
        )
    }

    /// Request find references
    pub fn references(&mut self, uri: &str, line: u32, character: u32) -> Result<u64> {
        let params = lsp_types::ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: lsp_types::Url::parse(uri)?,
                },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp_types::ReferenceContext {
                include_declaration: true,
            },
        };
        self.send_request(
            "textDocument/references",
            serde_json::to_value(params)?,
            RequestKind::References {
                uri: uri.to_string(),
                line,
                character,
            },
        )
    }

    /// Request code actions
    pub fn code_action(
        &mut self,
        uri: &str,
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
        diagnostics: &[Diagnostic],
    ) -> Result<u64> {
        // Convert our diagnostics to LSP diagnostics
        let lsp_diagnostics: Vec<lsp_types::Diagnostic> = diagnostics
            .iter()
            .map(|d| lsp_types::Diagnostic {
                range: lsp_types::Range {
                    start: lsp_types::Position {
                        line: d.line as u32,
                        character: d.col_start as u32,
                    },
                    end: lsp_types::Position {
                        line: d.line as u32,
                        character: d.col_end as u32,
                    },
                },
                severity: Some(match d.severity {
                    super::types::DiagnosticSeverity::Error => lsp_types::DiagnosticSeverity::ERROR,
                    super::types::DiagnosticSeverity::Warning => lsp_types::DiagnosticSeverity::WARNING,
                    super::types::DiagnosticSeverity::Information => lsp_types::DiagnosticSeverity::INFORMATION,
                    super::types::DiagnosticSeverity::Hint => lsp_types::DiagnosticSeverity::HINT,
                }),
                message: d.message.clone(),
                source: d.source.clone(),
                ..Default::default()
            })
            .collect();

        let params = lsp_types::CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: lsp_types::Url::parse(uri)?,
            },
            range: lsp_types::Range {
                start: lsp_types::Position {
                    line: start_line,
                    character: start_character,
                },
                end: lsp_types::Position {
                    line: end_line,
                    character: end_character,
                },
            },
            context: lsp_types::CodeActionContext {
                diagnostics: lsp_diagnostics,
                only: None,
                trigger_kind: Some(lsp_types::CodeActionTriggerKind::INVOKED),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.send_request(
            "textDocument/codeAction",
            serde_json::to_value(params)?,
            RequestKind::CodeAction {
                uri: uri.to_string(),
                start_line,
                start_character,
                end_line,
                end_character,
            },
        )
    }

    /// Request rename symbol
    pub fn rename(&mut self, uri: &str, line: u32, character: u32, new_name: &str) -> Result<u64> {
        let params = lsp_types::RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: lsp_types::Url::parse(uri)?,
                },
                position: lsp_types::Position { line, character },
            },
            new_name: new_name.to_string(),
            work_done_progress_params: Default::default(),
        };
        self.send_request(
            "textDocument/rename",
            serde_json::to_value(params)?,
            RequestKind::Rename {
                uri: uri.to_string(),
                line,
                character,
                new_name: new_name.to_string(),
            },
        )
    }

    /// Resolve a completion item to get full documentation
    /// Takes the raw LSP completion item data and the label for tracking
    pub fn completion_resolve(&mut self, item: Value, label: String) -> Result<u64> {
        self.send_request(
            "completionItem/resolve",
            item,
            RequestKind::CompletionResolve { label },
        )
    }

    /// Send a JSON-RPC request and track it in the pending map
    fn send_request(&mut self, method: &str, params: Value, kind: RequestKind) -> Result<u64> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);

        // Insert into pending map BEFORE sending the request
        // This ensures the response handler will find the request kind
        // even if the response arrives immediately
        if let Ok(mut pending) = self.pending_requests.lock() {
            pending.insert(id, kind);
        }

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params: if params.is_null() { None } else { Some(params) },
        };

        if let Err(err) = self.send_message(&serde_json::to_string(&request)?) {
            if let Ok(mut pending) = self.pending_requests.lock() {
                pending.remove(&id);
            }
            return Err(err);
        }
        Ok(id)
    }

    /// Send a JSON-RPC notification
    fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0",
            method: method.to_string(),
            params: if params.is_null() { None } else { Some(params) },
        };

        self.send_message(&serde_json::to_string(&notification)?)
    }

    /// Send a raw message with Content-Length header
    fn send_message(&mut self, content: &str) -> Result<()> {
        let message = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);
        let mut stdin = self.stdin.lock().map_err(|_| anyhow!("Failed to lock stdin"))?;
        stdin.write_all(message.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Try to gracefully shutdown, but don't block if stdin is locked
        // This prevents deadlock if another thread holds the lock
        if let Ok(mut stdin) = self.stdin.try_lock() {
            let notification = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "exit",
                "params": null
            });
            if let Ok(content) = serde_json::to_string(&notification) {
                let message = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);
                let _ = stdin.write_all(message.as_bytes());
                let _ = stdin.flush();
            }
        }
        let _ = self.process.kill();
    }
}

/// Send initialized notification using shared stdin
/// This is called from the reader thread immediately after receiving initialize response
fn send_initialized_notification(stdin: &SharedStdin) -> Result<()> {
    let notification = json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    let content = serde_json::to_string(&notification)?;
    let message = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);
    let mut stdin = stdin.lock().map_err(|_| anyhow!("Failed to lock stdin"))?;
    stdin.write_all(message.as_bytes())?;
    stdin.flush()?;
    Ok(())
}

/// Read JSON-RPC messages from the server stdout
///
/// Uses a shared pending requests map that is populated by the client thread
/// BEFORE sending requests. This eliminates the race condition where a response
/// could arrive before tracking info was sent through a channel.
pub fn read_messages(
    stdout: ChildStdout,
    tx: Sender<LspNotification>,
    pending: PendingRequests,
    stdin: SharedStdin,
) {
    let mut reader = BufReader::new(stdout);
    let mut headers = String::new();

    loop {
        headers.clear();

        // Read headers until empty line
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return, // EOF
                Ok(_) => {
                    let line = line.trim();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(len_str) = line.strip_prefix("Content-Length: ") {
                        content_length = len_str.parse().ok();
                    }
                }
                Err(_) => return,
            }
        }

        // Read content
        let content_length = match content_length {
            Some(len) => len,
            None => continue,
        };

        let mut content = vec![0u8; content_length];
        if reader.read_exact(&mut content).is_err() {
            return;
        }

        // Parse JSON
        let content_str = match String::from_utf8(content) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let response: JsonRpcResponse = match serde_json::from_str(&content_str) {
            Ok(r) => r,
            Err(_) => continue,
        };

        // Handle the message using the shared pending map
        let (notification, response_to_server) = handle_message(response, &pending);

        // Send response to server if needed (for server-initiated requests)
        if let Some(response_msg) = response_to_server {
            if let Ok(mut stdin_lock) = stdin.lock() {
                let _ = stdin_lock.write_all(response_msg.as_bytes());
                let _ = stdin_lock.flush();
            }
        }

        // If this is the Initialize response, send 'initialized' notification immediately
        // This must happen before any other requests are sent to the server
        if let Some(LspNotification::Initialized) = &notification {
            if let Err(e) = send_initialized_notification(&stdin) {
                let _ = tx.send(LspNotification::Error {
                    message: format!("Failed to send initialized: {}", e),
                });
            }
        }

        // Send notification to editor if we have one
        if let Some(notif) = notification {
            if tx.send(notif).is_err() {
                return;
            }
        }
    }
}

/// Handle an incoming JSON-RPC message using proper ID-based dispatch
/// Returns (notification_to_send, optional_response_to_server)
fn handle_message(
    msg: JsonRpcResponse,
    pending: &PendingRequests,
) -> (Option<LspNotification>, Option<String>) {
    // Check if it's a notification (no id) - these are server-initiated notifications
    if msg.id.is_none() {
        if let Some(method) = &msg.method {
            return (handle_notification(method, msg.params), None);
        }
        return (None, None);
    }

    let id = msg.id.unwrap();

    // Check if it's a server-initiated REQUEST (has both id AND method)
    // These require us to send a response back
    if let Some(method) = &msg.method {
        let response = handle_server_request(id, method, msg.params);
        return (None, response);
    }

    // It's a response to one of our requests - look up the request kind by ID
    let id_num = match id {
        JsonRpcId::Num(value) => value,
        JsonRpcId::Str(_) => {
            // Unexpected string ID for our requests; ignore.
            return (None, None);
        }
    };

    // Handle JSON-RPC errors - remove from pending map
    if let Some(error) = msg.error {
        if let Ok(mut pending_map) = pending.lock() {
            pending_map.remove(&id_num);
        }
        return (Some(LspNotification::Error {
            message: format!("LSP error ({}): {}", error.code, error.message),
        }), None);
    }

    // Look up what kind of request this was (and remove it from pending)
    let kind = match pending.lock() {
        Ok(mut pending_map) => pending_map.remove(&id_num),
        Err(_) => None,
    };

    let kind = match kind {
        Some(k) => k,
        None => {
            // Unknown response ID - could be a server request we didn't handle
            // or a timing issue. Log and ignore.
            return (None, None);
        }
    };

    // Dispatch based on request kind
    let notification = match kind {
        RequestKind::Initialize => {
            Some(LspNotification::Initialized)
        }
        RequestKind::Shutdown => {
            // Shutdown response - nothing to notify
            None
        }
        RequestKind::Completion { uri, line, character } => {
            match msg.result {
                Some(result) if !result.is_null() => handle_completion_response(result, uri, line, character),
                _ => Some(LspNotification::Completions {
                    items: vec![],
                    is_incomplete: false,
                    request_uri: uri,
                    request_line: line,
                    request_character: character,
                }),
            }
        }
        RequestKind::Definition { uri, line: _, character: _ } => {
            match msg.result {
                Some(result) if !result.is_null() => handle_definition_response(result, uri),
                _ => Some(LspNotification::Definition { locations: vec![], request_uri: uri }),
            }
        }
        RequestKind::Hover { uri, line, character } => {
            match msg.result {
                Some(result) if !result.is_null() => handle_hover_response(result, uri, line, character),
                _ => Some(LspNotification::Hover {
                    contents: None,
                    request_uri: uri,
                    request_line: line,
                    request_character: character,
                }),
            }
        }
        RequestKind::SignatureHelp { uri, line, character } => {
            match msg.result {
                Some(result) if !result.is_null() => handle_signature_help_response(result, uri, line, character),
                _ => Some(LspNotification::SignatureHelp {
                    help: None,
                    request_uri: uri,
                    request_line: line,
                    request_character: character,
                }),
            }
        }
        RequestKind::Formatting { uri } => {
            match msg.result {
                Some(result) if !result.is_null() => handle_formatting_response(result, uri),
                _ => Some(LspNotification::Formatting { edits: vec![], request_uri: uri }),
            }
        }
        RequestKind::References { uri, line: _, character: _ } => {
            match msg.result {
                Some(result) if !result.is_null() => handle_references_response(result, uri),
                _ => Some(LspNotification::References { locations: vec![], request_uri: uri }),
            }
        }
        RequestKind::CodeAction { uri, .. } => {
            match msg.result {
                Some(result) if !result.is_null() => handle_code_action_response(result, uri),
                _ => Some(LspNotification::CodeActions { actions: vec![], request_uri: uri }),
            }
        }
        RequestKind::Rename { uri, .. } => {
            match msg.result {
                Some(result) if !result.is_null() => handle_rename_response(result, uri),
                _ => Some(LspNotification::RenameResult { edits: vec![], request_uri: uri }),
            }
        }
        RequestKind::CompletionResolve { label } => {
            match msg.result {
                Some(result) if !result.is_null() => handle_completion_resolve_response(result, label),
                _ => Some(LspNotification::CompletionResolved { label, documentation: None, detail: None }),
            }
        }
    };

    (notification, None)
}

/// Handle a server-initiated request (server is asking us something)
/// Returns a JSON-RPC response string to send back
fn handle_server_request(id: JsonRpcId, method: &str, params: Option<Value>) -> Option<String> {
    match method {
        "workspace/configuration" => {
            // Server is asking for configuration
            // Return empty configs for each requested item
            let items_count = params
                .as_ref()
                .and_then(|p| p.get("items"))
                .and_then(|items| items.as_array())
                .map(|arr| arr.len())
                .unwrap_or(1);

            // Return an array of empty objects (one per requested item)
            let result: Vec<Value> = (0..items_count).map(|_| serde_json::json!({})).collect();

            build_response(JsonRpcResponseOut {
                jsonrpc: "2.0",
                id,
                result: Some(Value::Array(result)),
                error: None,
            })
        }
        "client/registerCapability" => {
            // Server wants to register dynamic capabilities
            // Acknowledge with null result
            build_response(JsonRpcResponseOut {
                jsonrpc: "2.0",
                id,
                result: Some(Value::Null),
                error: None,
            })
        }
        "window/workDoneProgress/create" => {
            // Server wants to create a progress indicator
            // Acknowledge with null result
            build_response(JsonRpcResponseOut {
                jsonrpc: "2.0",
                id,
                result: Some(Value::Null),
                error: None,
            })
        }
        "workspace/workspaceFolders" => {
            // Server wants the list of workspace folders
            build_response(JsonRpcResponseOut {
                jsonrpc: "2.0",
                id,
                result: Some(Value::Array(vec![])),
                error: None,
            })
        }
        "window/showMessageRequest" => {
            // We don't support interactive choices; return null.
            build_response(JsonRpcResponseOut {
                jsonrpc: "2.0",
                id,
                result: Some(Value::Null),
                error: None,
            })
        }
        "client/unregisterCapability" => {
            // Accept unregister requests
            build_response(JsonRpcResponseOut {
                jsonrpc: "2.0",
                id,
                result: Some(Value::Null),
                error: None,
            })
        }
        "workspace/applyEdit" => {
            // We don't support workspace edits yet; explicitly reject.
            let result = serde_json::json!({
                "applied": false,
                "failureReason": "workspace edits not supported",
            });
            build_response(JsonRpcResponseOut {
                jsonrpc: "2.0",
                id,
                result: Some(result),
                error: None,
            })
        }
        _ => {
            // Unknown server request - return method not found error
            build_response(JsonRpcResponseOut {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(JsonRpcErrorOut {
                    code: -32601,
                    message: format!("Method not found: {}", method),
                }),
            })
        }
    }
}

fn build_response(response: JsonRpcResponseOut) -> Option<String> {
    let body = serde_json::to_string(&response).ok()?;
    Some(format!("Content-Length: {}\r\n\r\n{}", body.len(), body))
}

/// Handle a server notification
fn handle_notification(method: &str, params: Option<Value>) -> Option<LspNotification> {
    match method {
        "textDocument/publishDiagnostics" => {
            let params = params?;
            let uri = params.get("uri")?.as_str()?.to_string();
            let diagnostics_json = params.get("diagnostics")?.as_array()?;

            let diagnostics: Vec<Diagnostic> = diagnostics_json
                .iter()
                .filter_map(|d| {
                    let range = d.get("range")?;
                    let start = range.get("start")?;
                    let end = range.get("end")?;

                    let severity = d
                        .get("severity")
                        .and_then(|s| s.as_u64())
                        .map(|s| match s {
                            1 => DiagnosticSeverity::Error,
                            2 => DiagnosticSeverity::Warning,
                            3 => DiagnosticSeverity::Information,
                            _ => DiagnosticSeverity::Hint,
                        })
                        .unwrap_or(DiagnosticSeverity::Error);

                    Some(Diagnostic {
                        line: start.get("line")?.as_u64()? as usize,
                        end_line: end.get("line")?.as_u64()? as usize,
                        col_start: start.get("character")?.as_u64()? as usize,
                        col_end: end.get("character")?.as_u64()? as usize,
                        severity,
                        message: d.get("message")?.as_str()?.to_string(),
                        source: d.get("source").and_then(|s| s.as_str()).map(|s| s.to_string()),
                    })
                })
                .collect();

            Some(LspNotification::Diagnostics { uri, diagnostics })
        }
        "window/showMessage" | "window/logMessage" => {
            let params = params?;
            let message = params.get("message")?.as_str()?.to_string();
            Some(LspNotification::Status { message })
        }
        _ => None,
    }
}

/// Handle completion response
fn handle_completion_response(
    result: Value,
    request_uri: String,
    request_line: u32,
    request_character: u32,
) -> Option<LspNotification> {
    // Parse isIncomplete flag (defaults to false for array format)
    let is_incomplete = if result.is_object() {
        result.get("isIncomplete").and_then(|v| v.as_bool()).unwrap_or(false)
    } else {
        false
    };

    let items_json = if result.is_array() {
        result.as_array()?.clone()
    } else {
        result.get("items")?.as_array()?.clone()
    };

    let items: Vec<CompletionItem> = items_json
        .iter()
        .filter_map(|item| {
            let label = item.get("label")?.as_str()?.to_string();

            let kind = item
                .get("kind")
                .and_then(|k| k.as_u64())
                .map(|k| match k {
                    1 => CompletionKind::Text,
                    2 => CompletionKind::Method,
                    3 => CompletionKind::Function,
                    4 => CompletionKind::Constructor,
                    5 => CompletionKind::Field,
                    6 => CompletionKind::Variable,
                    7 => CompletionKind::Class,
                    8 => CompletionKind::Interface,
                    9 => CompletionKind::Module,
                    10 => CompletionKind::Property,
                    11 => CompletionKind::Unit,
                    12 => CompletionKind::Value,
                    13 => CompletionKind::Enum,
                    14 => CompletionKind::Keyword,
                    15 => CompletionKind::Snippet,
                    16 => CompletionKind::Color,
                    17 => CompletionKind::File,
                    18 => CompletionKind::Reference,
                    19 => CompletionKind::Folder,
                    20 => CompletionKind::EnumMember,
                    21 => CompletionKind::Constant,
                    22 => CompletionKind::Struct,
                    23 => CompletionKind::Event,
                    24 => CompletionKind::Operator,
                    25 => CompletionKind::TypeParameter,
                    _ => CompletionKind::Text,
                })
                .unwrap_or(CompletionKind::Text);

            let detail = item.get("detail").and_then(|d| d.as_str()).map(|s| s.to_string());

            let documentation = item
                .get("documentation")
                .and_then(|d| {
                    if d.is_string() {
                        d.as_str().map(|s| s.to_string())
                    } else {
                        d.get("value").and_then(|v| v.as_str()).map(|s| s.to_string())
                    }
                });

            let insert_text = item
                .get("insertText")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());

            let filter_text = item
                .get("filterText")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());

            let sort_text = item
                .get("sortText")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());

            Some(CompletionItem {
                label,
                kind,
                detail,
                documentation,
                insert_text,
                filter_text,
                sort_text,
                raw_data: Some(item.clone()),
            })
        })
        .collect();

    Some(LspNotification::Completions {
        items,
        is_incomplete,
        request_uri,
        request_line,
        request_character,
    })
}

/// Handle definition response - returns all locations for multi-definition support
fn handle_definition_response(result: Value, request_uri: String) -> Option<LspNotification> {
    // Can be a single Location, array of Locations, or array of LocationLinks
    let locations_json = if result.is_array() {
        result.as_array()?.clone()
    } else {
        vec![result]
    };

    let locations: Vec<Location> = locations_json
        .iter()
        .filter_map(|loc| {
            // Handle both Location and LocationLink formats
            let uri = loc.get("uri")
                .or_else(|| loc.get("targetUri"))
                .and_then(|u| u.as_str())?
                .to_string();

            let range = loc.get("range")
                .or_else(|| loc.get("targetRange"))?;
            let start = range.get("start")?;
            let line = start.get("line")?.as_u64()? as usize;
            let col = start.get("character")?.as_u64()? as usize;

            Some(Location { uri, line, col })
        })
        .collect();

    Some(LspNotification::Definition { locations, request_uri })
}

/// Handle hover response
fn handle_hover_response(
    result: Value,
    request_uri: String,
    request_line: u32,
    request_character: u32,
) -> Option<LspNotification> {
    let contents = result.get("contents")?;

    let text = if contents.is_string() {
        contents.as_str()?.to_string()
    } else if contents.is_array() {
        // Array of MarkedString
        let arr = contents.as_array()?;
        arr.iter()
            .filter_map(|c| {
                if c.is_string() {
                    c.as_str().map(|s| s.to_string())
                } else {
                    c.get("value").and_then(|v| v.as_str()).map(|s| s.to_string())
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    } else if contents.get("kind").is_some() {
        // MarkupContent
        contents.get("value")?.as_str()?.to_string()
    } else if contents.get("value").is_some() {
        // MarkedString object
        contents.get("value")?.as_str()?.to_string()
    } else {
        return None;
    };

    Some(LspNotification::Hover {
        contents: Some(text),
        request_uri,
        request_line,
        request_character,
    })
}

/// Handle signature help response
fn handle_signature_help_response(
    result: Value,
    request_uri: String,
    request_line: u32,
    request_character: u32,
) -> Option<LspNotification> {
    let signatures_json = result.get("signatures")?.as_array()?;

    if signatures_json.is_empty() {
        return Some(LspNotification::SignatureHelp {
            help: None,
            request_uri,
            request_line,
            request_character,
        });
    }

    let active_signature = result
        .get("activeSignature")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let active_parameter = result
        .get("activeParameter")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let signatures: Vec<SignatureInfo> = signatures_json
        .iter()
        .filter_map(|sig| {
            let label = sig.get("label")?.as_str()?.to_string();

            let documentation = sig.get("documentation").and_then(|d| {
                if d.is_string() {
                    d.as_str().map(|s| s.to_string())
                } else {
                    d.get("value").and_then(|v| v.as_str()).map(|s| s.to_string())
                }
            });

            let parameters: Vec<ParameterInfo> = sig
                .get("parameters")
                .and_then(|p| p.as_array())
                .map(|params| {
                    params
                        .iter()
                        .filter_map(|param| {
                            let param_label = param.get("label")?;
                            let (label_offsets, label_text) = if param_label.is_array() {
                                // Label offsets [start, end]
                                let arr = param_label.as_array()?;
                                let start = arr.first()?.as_u64()? as usize;
                                let end = arr.get(1)?.as_u64()? as usize;
                                // Extract the label text from the signature
                                let text = if end <= label.len() && start < end {
                                    label[start..end].to_string()
                                } else {
                                    String::new()
                                };
                                (Some((start, end)), text)
                            } else {
                                // Label is a string
                                let text = param_label.as_str()?.to_string();
                                // Find the offsets in the signature label
                                let offsets = label.find(&text).map(|start| (start, start + text.len()));
                                (offsets, text)
                            };

                            let doc = param.get("documentation").and_then(|d| {
                                if d.is_string() {
                                    d.as_str().map(|s| s.to_string())
                                } else {
                                    d.get("value").and_then(|v| v.as_str()).map(|s| s.to_string())
                                }
                            });

                            Some(ParameterInfo {
                                label_offsets,
                                label: label_text,
                                documentation: doc,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            Some(SignatureInfo {
                label,
                documentation,
                parameters,
            })
        })
        .collect();

    if signatures.is_empty() {
        return Some(LspNotification::SignatureHelp {
            help: None,
            request_uri,
            request_line,
            request_character,
        });
    }

    Some(LspNotification::SignatureHelp {
        help: Some(SignatureHelpResult {
            signatures,
            active_signature,
            active_parameter,
        }),
        request_uri,
        request_line,
        request_character,
    })
}

/// Handle formatting response
fn handle_formatting_response(result: Value, request_uri: String) -> Option<LspNotification> {
    // Result is an array of TextEdits or null
    let edits_json = result.as_array()?;

    let edits: Vec<TextEdit> = edits_json
        .iter()
        .filter_map(|edit| {
            let range = edit.get("range")?;
            let start = range.get("start")?;
            let end = range.get("end")?;

            Some(TextEdit {
                start_line: start.get("line")?.as_u64()? as usize,
                start_col: start.get("character")?.as_u64()? as usize,
                end_line: end.get("line")?.as_u64()? as usize,
                end_col: end.get("character")?.as_u64()? as usize,
                new_text: edit.get("newText")?.as_str()?.to_string(),
            })
        })
        .collect();

    Some(LspNotification::Formatting { edits, request_uri })
}

/// Handle references response
fn handle_references_response(result: Value, request_uri: String) -> Option<LspNotification> {
    // Result is an array of Locations
    let locations_json = result.as_array()?;

    let locations: Vec<Location> = locations_json
        .iter()
        .filter_map(|loc| {
            let uri = loc.get("uri")?.as_str()?.to_string();
            let range = loc.get("range")?;
            let start = range.get("start")?;
            let line = start.get("line")?.as_u64()? as usize;
            let col = start.get("character")?.as_u64()? as usize;

            Some(Location { uri, line, col })
        })
        .collect();

    Some(LspNotification::References { locations, request_uri })
}

/// Handle code action response
fn handle_code_action_response(result: Value, request_uri: String) -> Option<LspNotification> {
    // Result is an array of CodeAction or Command
    let actions_json = result.as_array()?;

    let actions: Vec<CodeActionItem> = actions_json
        .iter()
        .filter_map(|action| {
            // Can be either a Command or a CodeAction
            let title = action.get("title")?.as_str()?.to_string();
            let kind = action.get("kind").and_then(|k| k.as_str()).map(|s| s.to_string());
            let is_preferred = action.get("isPreferred").and_then(|p| p.as_bool()).unwrap_or(false);

            // Parse workspace edit if present
            let mut edits: Vec<(String, Vec<TextEdit>)> = Vec::new();

            if let Some(edit) = action.get("edit") {
                if let Some(changes) = edit.get("changes").and_then(|c| c.as_object()) {
                    for (uri, file_edits) in changes {
                        if let Some(file_edits_arr) = file_edits.as_array() {
                            let text_edits: Vec<TextEdit> = file_edits_arr
                                .iter()
                                .filter_map(|e| {
                                    let range = e.get("range")?;
                                    let start = range.get("start")?;
                                    let end = range.get("end")?;
                                    Some(TextEdit {
                                        start_line: start.get("line")?.as_u64()? as usize,
                                        start_col: start.get("character")?.as_u64()? as usize,
                                        end_line: end.get("line")?.as_u64()? as usize,
                                        end_col: end.get("character")?.as_u64()? as usize,
                                        new_text: e.get("newText")?.as_str()?.to_string(),
                                    })
                                })
                                .collect();
                            if !text_edits.is_empty() {
                                edits.push((uri.clone(), text_edits));
                            }
                        }
                    }
                }
                // Also handle documentChanges format
                if let Some(doc_changes) = edit.get("documentChanges").and_then(|c| c.as_array()) {
                    for doc_change in doc_changes {
                        if let (Some(text_document), Some(file_edits_arr)) = (
                            doc_change.get("textDocument"),
                            doc_change.get("edits").and_then(|e| e.as_array()),
                        ) {
                            let uri = text_document.get("uri")?.as_str()?.to_string();
                            let text_edits: Vec<TextEdit> = file_edits_arr
                                .iter()
                                .filter_map(|e| {
                                    let range = e.get("range")?;
                                    let start = range.get("start")?;
                                    let end = range.get("end")?;
                                    Some(TextEdit {
                                        start_line: start.get("line")?.as_u64()? as usize,
                                        start_col: start.get("character")?.as_u64()? as usize,
                                        end_line: end.get("line")?.as_u64()? as usize,
                                        end_col: end.get("character")?.as_u64()? as usize,
                                        new_text: e.get("newText")?.as_str()?.to_string(),
                                    })
                                })
                                .collect();
                            if !text_edits.is_empty() {
                                edits.push((uri, text_edits));
                            }
                        }
                    }
                }
            }

            let command = action.get("command").and_then(|c| c.get("command")).and_then(|t| t.as_str()).map(|s| s.to_string());

            Some(CodeActionItem {
                title,
                kind,
                is_preferred,
                edits,
                command,
            })
        })
        .collect();

    Some(LspNotification::CodeActions { actions, request_uri })
}

/// Handle rename response
fn handle_rename_response(result: Value, request_uri: String) -> Option<LspNotification> {
    // Result is a WorkspaceEdit
    let mut edits: Vec<(String, Vec<TextEdit>)> = Vec::new();

    // Handle "changes" format
    if let Some(changes) = result.get("changes").and_then(|c| c.as_object()) {
        for (uri, file_edits) in changes {
            if let Some(file_edits_arr) = file_edits.as_array() {
                let text_edits: Vec<TextEdit> = file_edits_arr
                    .iter()
                    .filter_map(|e| {
                        let range = e.get("range")?;
                        let start = range.get("start")?;
                        let end = range.get("end")?;
                        Some(TextEdit {
                            start_line: start.get("line")?.as_u64()? as usize,
                            start_col: start.get("character")?.as_u64()? as usize,
                            end_line: end.get("line")?.as_u64()? as usize,
                            end_col: end.get("character")?.as_u64()? as usize,
                            new_text: e.get("newText")?.as_str()?.to_string(),
                        })
                    })
                    .collect();
                if !text_edits.is_empty() {
                    edits.push((uri.clone(), text_edits));
                }
            }
        }
    }

    // Handle "documentChanges" format
    if let Some(doc_changes) = result.get("documentChanges").and_then(|c| c.as_array()) {
        for doc_change in doc_changes {
            if let (Some(text_document), Some(file_edits_arr)) = (
                doc_change.get("textDocument"),
                doc_change.get("edits").and_then(|e| e.as_array()),
            ) {
                if let Some(uri) = text_document.get("uri").and_then(|u| u.as_str()) {
                    let text_edits: Vec<TextEdit> = file_edits_arr
                        .iter()
                        .filter_map(|e| {
                            let range = e.get("range")?;
                            let start = range.get("start")?;
                            let end = range.get("end")?;
                            Some(TextEdit {
                                start_line: start.get("line")?.as_u64()? as usize,
                                start_col: start.get("character")?.as_u64()? as usize,
                                end_line: end.get("line")?.as_u64()? as usize,
                                end_col: end.get("character")?.as_u64()? as usize,
                                new_text: e.get("newText")?.as_str()?.to_string(),
                            })
                        })
                        .collect();
                    if !text_edits.is_empty() {
                        edits.push((uri.to_string(), text_edits));
                    }
                }
            }
        }
    }

    Some(LspNotification::RenameResult { edits, request_uri })
}

/// Handle completionItem/resolve response
fn handle_completion_resolve_response(result: Value, label: String) -> Option<LspNotification> {
    // Extract documentation from the resolved item
    let documentation = result
        .get("documentation")
        .and_then(|d| {
            if d.is_string() {
                d.as_str().map(|s| s.to_string())
            } else {
                // MarkupContent format: { kind: "markdown"|"plaintext", value: "..." }
                d.get("value").and_then(|v| v.as_str()).map(|s| s.to_string())
            }
        });

    // Also extract detail if it was updated
    let detail = result
        .get("detail")
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());

    Some(LspNotification::CompletionResolved {
        label,
        documentation,
        detail,
    })
}
