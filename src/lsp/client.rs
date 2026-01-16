//! LSP client implementation using JSON-RPC over stdio

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

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

use std::sync::mpsc::Receiver;

use super::types::{
    CompletionItem, CompletionKind, Diagnostic, DiagnosticSeverity, Location, LspNotification,
    ParameterInfo, RequestKind, SignatureHelpResult, SignatureInfo,
};

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

/// JSON-RPC response
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<u64>,
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

/// LSP client that communicates with a language server
pub struct LspClient {
    process: Child,
    stdin: ChildStdin,
    request_id: AtomicU64,
    pending_requests: HashMap<u64, String>, // id -> method name for tracking
}

impl LspClient {
    /// Spawn a new LSP server process
    pub fn spawn(command: &str, args: &[String]) -> Result<Self> {
        let mut process = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn LSP server '{}': {}", command, e))?;

        let stdin = process.stdin.take().ok_or_else(|| anyhow!("Failed to get stdin"))?;

        Ok(Self {
            process,
            stdin,
            request_id: AtomicU64::new(1),
            pending_requests: HashMap::new(),
        })
    }

    /// Get stdout for reading responses
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.process.stdout.take()
    }

    /// Send initialize request
    pub fn initialize(&mut self, root_path: &std::path::Path) -> Result<u64> {
        let root_uri = format!("file://{}", root_path.display());

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_path: Some(root_path.to_string_lossy().to_string()),
            root_uri: Some(lsp_types::Url::parse(&root_uri)?),
            initialization_options: None,
            capabilities: ClientCapabilities {
                text_document: Some(lsp_types::TextDocumentClientCapabilities {
                    completion: Some(lsp_types::CompletionClientCapabilities {
                        completion_item: Some(lsp_types::CompletionItemCapability {
                            snippet_support: Some(false),
                            documentation_format: Some(vec![lsp_types::MarkupKind::PlainText]),
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
                uri: lsp_types::Url::parse(&root_uri)?,
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

        self.send_request("initialize", serde_json::to_value(params)?)
    }

    /// Send initialized notification (after initialize response)
    pub fn initialized(&mut self) -> Result<()> {
        self.send_notification("initialized", json!({}))
    }

    /// Send shutdown request
    pub fn shutdown(&mut self) -> Result<u64> {
        self.send_request("shutdown", Value::Null)
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
        self.send_request("textDocument/completion", serde_json::to_value(params)?)
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
        self.send_request("textDocument/definition", serde_json::to_value(params)?)
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
        self.send_request("textDocument/hover", serde_json::to_value(params)?)
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
        self.send_request("textDocument/signatureHelp", serde_json::to_value(params)?)
    }

    /// Send a JSON-RPC request
    fn send_request(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        self.pending_requests.insert(id, method.to_string());

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params: if params.is_null() { None } else { Some(params) },
        };

        self.send_message(&serde_json::to_string(&request)?)?;
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
        self.stdin.write_all(message.as_bytes())?;
        self.stdin.flush()?;
        Ok(())
    }

    /// Get the method name for a pending request
    pub fn get_pending_method(&self, id: u64) -> Option<&str> {
        self.pending_requests.get(&id).map(|s| s.as_str())
    }

    /// Remove a pending request
    pub fn remove_pending(&mut self, id: u64) {
        self.pending_requests.remove(&id);
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Try to gracefully shutdown
        let _ = self.exit();
        let _ = self.process.kill();
    }
}

/// Read JSON-RPC messages from the server stdout
pub fn read_messages(
    stdout: ChildStdout,
    tx: Sender<LspNotification>,
    tracking_rx: Receiver<(u64, RequestKind)>,
) {
    let mut reader = BufReader::new(stdout);
    let mut headers = String::new();
    let mut pending: HashMap<u64, RequestKind> = HashMap::new();

    loop {
        headers.clear();

        // Drain any new request trackings (non-blocking)
        while let Ok((id, kind)) = tracking_rx.try_recv() {
            pending.insert(id, kind);
        }

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

        // Drain tracking channel again AFTER reading, before handling
        // This fixes the race condition where tracking is sent while we're blocked reading
        while let Ok((id, kind)) = tracking_rx.try_recv() {
            pending.insert(id, kind);
        }

        // Handle the message with pending request tracking
        if let Some(notification) = handle_message(response, &mut pending) {
            if tx.send(notification).is_err() {
                return;
            }
        }
    }
}

/// Handle an incoming JSON-RPC message using proper ID-based dispatch
fn handle_message(
    msg: JsonRpcResponse,
    pending: &mut HashMap<u64, RequestKind>,
) -> Option<LspNotification> {
    // Check if it's a notification (no id) - these are server-initiated
    if msg.id.is_none() {
        if let Some(method) = &msg.method {
            return handle_notification(method, msg.params);
        }
        return None;
    }

    // It's a response to one of our requests - look up the request kind by ID
    let id = msg.id.unwrap();

    // Handle JSON-RPC errors
    if let Some(error) = msg.error {
        pending.remove(&id);
        return Some(LspNotification::Error {
            message: format!("LSP error ({}): {}", error.code, error.message),
        });
    }

    // Look up what kind of request this was
    let kind = match pending.remove(&id) {
        Some(k) => k,
        None => {
            // Unknown response ID - might be initialize response before tracking started
            // Try to detect initialize response by checking for capabilities
            if let Some(ref result) = msg.result {
                if result.get("capabilities").is_some() {
                    return Some(LspNotification::Initialized);
                }
            }
            return None;
        }
    };

    // Dispatch based on request kind
    match kind {
        RequestKind::Initialize => {
            Some(LspNotification::Initialized)
        }
        RequestKind::Shutdown => {
            // Shutdown response - nothing to notify
            None
        }
        RequestKind::Completion => {
            match msg.result {
                Some(result) if !result.is_null() => handle_completion_response(result),
                _ => Some(LspNotification::Completions { items: vec![], is_incomplete: false }),
            }
        }
        RequestKind::Definition => {
            match msg.result {
                Some(result) if !result.is_null() => handle_definition_response(result),
                _ => Some(LspNotification::Definition { locations: vec![] }),
            }
        }
        RequestKind::Hover => {
            match msg.result {
                Some(result) if !result.is_null() => handle_hover_response(result),
                _ => Some(LspNotification::Hover { contents: None }),
            }
        }
        RequestKind::SignatureHelp => {
            match msg.result {
                Some(result) if !result.is_null() => handle_signature_help_response(result),
                _ => Some(LspNotification::SignatureHelp { help: None }),
            }
        }
    }
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
fn handle_completion_response(result: Value) -> Option<LspNotification> {
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
            })
        })
        .collect();

    Some(LspNotification::Completions { items, is_incomplete })
}

/// Handle definition response - returns all locations for multi-definition support
fn handle_definition_response(result: Value) -> Option<LspNotification> {
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

    Some(LspNotification::Definition { locations })
}

/// Handle hover response
fn handle_hover_response(result: Value) -> Option<LspNotification> {
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
    })
}

/// Handle signature help response
fn handle_signature_help_response(result: Value) -> Option<LspNotification> {
    let signatures_json = result.get("signatures")?.as_array()?;

    if signatures_json.is_empty() {
        return Some(LspNotification::SignatureHelp { help: None });
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
        return Some(LspNotification::SignatureHelp { help: None });
    }

    Some(LspNotification::SignatureHelp {
        help: Some(SignatureHelpResult {
            signatures,
            active_signature,
            active_parameter,
        }),
    })
}
