//! LSP-style client for GitHub Copilot server
//!
//! Communicates with the Copilot language server over stdio using JSON-RPC.
//! The protocol is similar to LSP but uses custom methods.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::types::*;

/// Debug logging helper - writes to /tmp/copilot_debug.log
/// Disabled by default for performance - enable only when debugging Copilot issues
#[allow(dead_code)]
fn debug_log(_msg: &str) {
    // Disabled for performance - file I/O on every call is too expensive
    // To enable: uncomment the code below
    /*
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/copilot_debug.log")
    {
        let _ = writeln!(file, "[client] {}", msg);
    }
    */
}

/// Shared pending requests map - maps request ID to request kind
pub type PendingRequests = Arc<Mutex<HashMap<u64, CopilotRequestKind>>>;
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

/// Copilot client that communicates with the language server
pub struct CopilotClient {
    process: Child,
    stdin: SharedStdin,
    request_id: AtomicU64,
    pending_requests: PendingRequests,
}

impl CopilotClient {
    /// Spawn a new Copilot server process
    pub fn spawn(node_path: &str, server_path: &str) -> Result<(Self, PendingRequests, SharedStdin)> {
        let mut process = Command::new(node_path)
            .arg(server_path)
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn Copilot server: {}", e))?;

        let stdin = process
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to get stdin"))?;
        let stdin = Arc::new(Mutex::new(stdin));
        let stdin_clone = stdin.clone();

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

    /// Send initialize request
    pub fn initialize(&mut self) -> Result<u64> {
        let init_options = CopilotInitOptions {
            editor_info: EditorInfo {
                name: "nevi".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            editor_plugin_info: EditorPluginInfo {
                name: "nevi-copilot".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let params = json!({
            "processId": std::process::id(),
            "capabilities": {},
            "initializationOptions": init_options,
            "clientInfo": {
                "name": "nevi",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        self.send_request("initialize", params, CopilotRequestKind::Initialize)
    }

    /// Send initialized notification
    pub fn initialized(&mut self) -> Result<()> {
        self.send_notification("initialized", json!({}))
    }

    /// Send workspace/didChangeConfiguration with Copilot settings
    pub fn did_change_configuration(&mut self, config: &CopilotConfiguration) -> Result<()> {
        let params = json!({
            "settings": {
                "github.copilot": config
            }
        });
        self.send_notification("workspace/didChangeConfiguration", params)
    }

    /// Check authentication status
    pub fn check_status(&mut self) -> Result<u64> {
        self.send_request("checkStatus", json!({}), CopilotRequestKind::CheckStatus)
    }

    /// Initiate device flow sign-in
    pub fn sign_in_initiate(&mut self) -> Result<u64> {
        self.send_request("signInInitiate", json!({}), CopilotRequestKind::SignInInitiate)
    }

    /// Confirm sign-in with user code
    pub fn sign_in_confirm(&mut self, user_code: &str) -> Result<u64> {
        let params = json!({
            "userCode": user_code
        });
        self.send_request(
            "signInConfirm",
            params,
            CopilotRequestKind::SignInConfirm {
                user_code: user_code.to_string(),
            },
        )
    }

    /// Sign out
    pub fn sign_out(&mut self) -> Result<()> {
        self.send_notification("signOut", json!({}))
    }

    /// Request completions at position
    pub fn get_completions(&mut self, doc: &CopilotDocument) -> Result<u64> {
        let params = json!({
            "doc": doc
        });
        self.send_request(
            "getCompletions",
            params,
            CopilotRequestKind::GetCompletions {
                uri: doc.uri.clone(),
                version: doc.version,
                line: doc.position.line,
                character: doc.position.character,
            },
        )
    }

    /// Request completions for cycling (additional suggestions)
    pub fn get_completions_cycling(&mut self, doc: &CopilotDocument) -> Result<u64> {
        let params = json!({
            "doc": doc
        });
        self.send_request(
            "getCompletionsCycling",
            params,
            CopilotRequestKind::GetCompletionsCycling {
                uri: doc.uri.clone(),
                version: doc.version,
                line: doc.position.line,
                character: doc.position.character,
            },
        )
    }

    /// Notify that a completion was accepted
    pub fn notify_accepted(&mut self, uuid: &str, accepted_length: usize) -> Result<()> {
        let params = json!({
            "uuid": uuid,
            "acceptedLength": accepted_length
        });
        self.send_notification("notifyAccepted", params)
    }

    /// Notify that completions were rejected
    pub fn notify_rejected(&mut self, uuids: &[String]) -> Result<()> {
        let params = json!({
            "uuids": uuids
        });
        self.send_notification("notifyRejected", params)
    }

    /// Notify that a completion was shown to the user
    pub fn notify_shown(&mut self, uuid: &str) -> Result<()> {
        let params = json!({
            "uuid": uuid
        });
        self.send_notification("notifyShown", params)
    }

    /// Notify server that a document was opened
    pub fn did_open(&mut self, uri: &str, language_id: &str, version: i32, text: &str) -> Result<()> {
        let params = json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": version,
                "text": text
            }
        });
        self.send_notification("textDocument/didOpen", params)
    }

    /// Notify server that a document changed
    pub fn did_change(&mut self, uri: &str, version: i32, text: &str) -> Result<()> {
        let params = json!({
            "textDocument": {
                "uri": uri,
                "version": version
            },
            "contentChanges": [{
                "text": text
            }]
        });
        self.send_notification("textDocument/didChange", params)
    }

    /// Notify server that a document was closed
    pub fn did_close(&mut self, uri: &str) -> Result<()> {
        let params = json!({
            "textDocument": {
                "uri": uri
            }
        });
        self.send_notification("textDocument/didClose", params)
    }

    /// Shutdown the server
    pub fn shutdown(&mut self) -> Result<()> {
        let _ = self.send_notification("shutdown", json!({}));
        let _ = self.send_notification("exit", json!({}));
        let _ = self.process.kill();
        Ok(())
    }

    /// Send a JSON-RPC request and track it
    fn send_request(&mut self, method: &str, params: Value, kind: CopilotRequestKind) -> Result<u64> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);

        // Track request before sending
        if let Ok(mut pending) = self.pending_requests.lock() {
            pending.insert(id, kind);
        }

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params: if params.is_null() { None } else { Some(params.clone()) },
        };

        debug_log(&format!("SEND REQUEST id={} method={}", id, method));
        if method == "getCompletions" {
            debug_log(&format!("  params={}", serde_json::to_string_pretty(&params).unwrap_or_default()));
        }

        if let Err(err) = self.send_message(&serde_json::to_string(&request)?) {
            if let Ok(mut pending) = self.pending_requests.lock() {
                pending.remove(&id);
            }
            debug_log(&format!("  SEND ERROR: {}", err));
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

impl Drop for CopilotClient {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// Read JSON-RPC messages from the server stdout
pub fn read_messages(
    stdout: ChildStdout,
    tx: Sender<CopilotNotification>,
    pending: PendingRequests,
    stdin: SharedStdin,
) {
    let mut reader = BufReader::new(stdout);

    loop {
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
        if std::io::Read::read_exact(&mut reader, &mut content).is_err() {
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

        // Handle the message
        let (notification, response_to_server) = handle_message(response, &pending);

        // Send response to server if needed
        if let Some(response_msg) = response_to_server {
            if let Ok(mut stdin_lock) = stdin.lock() {
                let _ = stdin_lock.write_all(response_msg.as_bytes());
                let _ = stdin_lock.flush();
            }
        }

        // Send notification to editor
        if let Some(notif) = notification {
            if tx.send(notif).is_err() {
                return;
            }
        }
    }
}

/// Handle an incoming JSON-RPC message
fn handle_message(
    msg: JsonRpcResponse,
    pending: &PendingRequests,
) -> (Option<CopilotNotification>, Option<String>) {
    // Check if it's a notification (no id)
    if msg.id.is_none() {
        if let Some(method) = &msg.method {
            debug_log(&format!("RECV NOTIFICATION method={}", method));
            return (handle_notification(method, msg.params), None);
        }
        return (None, None);
    }

    let id = msg.id.unwrap();
    debug_log(&format!("RECV RESPONSE id={:?}", id));

    // Check if it's a server-initiated request
    if let Some(method) = &msg.method {
        let response = handle_server_request(id, method, msg.params);
        return (None, response);
    }

    // It's a response to our request
    let id_num = match id {
        JsonRpcId::Num(value) => value,
        JsonRpcId::Str(_) => return (None, None),
    };

    // Handle errors
    if let Some(error) = msg.error {
        debug_log(&format!("  ERROR code={} message={}", error.code, error.message));
        if let Ok(mut pending_map) = pending.lock() {
            pending_map.remove(&id_num);
        }
        return (
            Some(CopilotNotification::Error {
                message: format!("Copilot error ({}): {}", error.code, error.message),
            }),
            None,
        );
    }

    // Look up request kind
    let kind = match pending.lock() {
        Ok(mut pending_map) => pending_map.remove(&id_num),
        Err(_) => None,
    };

    let kind = match kind {
        Some(k) => k,
        None => return (None, None),
    };

    // Dispatch based on request kind
    let notification = match kind {
        CopilotRequestKind::Initialize => Some(CopilotNotification::Initialized),
        CopilotRequestKind::CheckStatus => {
            if let Some(result) = msg.result {
                handle_check_status_response(result)
            } else {
                None
            }
        }
        CopilotRequestKind::SignInInitiate => {
            if let Some(result) = msg.result {
                handle_sign_in_initiate_response(result)
            } else {
                None
            }
        }
        CopilotRequestKind::SignInConfirm { .. } => {
            if let Some(result) = msg.result {
                handle_sign_in_confirm_response(result)
            } else {
                None
            }
        }
        CopilotRequestKind::GetCompletions { .. }
        | CopilotRequestKind::GetCompletionsCycling { .. } => {
            if let Some(result) = msg.result {
                handle_completions_response(result, id_num)
            } else {
                Some(CopilotNotification::Completions(CopilotCompletionResult {
                    completions: Vec::new(),
                    request_id: id_num,
                }))
            }
        }
        CopilotRequestKind::NotifyAccepted
        | CopilotRequestKind::NotifyRejected
        | CopilotRequestKind::NotifyShown => None,
    };

    (notification, None)
}

/// Handle a server notification
fn handle_notification(method: &str, params: Option<Value>) -> Option<CopilotNotification> {
    match method {
        "statusNotification" => {
            let params = params?;
            let message = params.get("message")?.as_str()?.to_string();
            Some(CopilotNotification::Status { message })
        }
        "window/logMessage" | "window/showMessage" => {
            let params = params?;
            let message = params.get("message")?.as_str()?.to_string();
            Some(CopilotNotification::Status { message })
        }
        _ => None,
    }
}

/// Handle a server-initiated request
fn handle_server_request(id: JsonRpcId, method: &str, _params: Option<Value>) -> Option<String> {
    match method {
        "workspace/configuration" | "client/registerCapability" | "window/workDoneProgress/create" => {
            // Return empty/null result
            build_response(JsonRpcResponseOut {
                jsonrpc: "2.0",
                id,
                result: Some(Value::Null),
                error: None,
            })
        }
        _ => {
            // Unknown method - return error
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

/// Handle checkStatus response
fn handle_check_status_response(result: Value) -> Option<CopilotNotification> {
    let status = result.get("status")?.as_str()?;
    let user = result.get("user").and_then(|u| u.as_str()).map(|s| s.to_string());

    let auth_status = match status {
        "OK" | "Normal" => {
            if let Some(user) = user {
                AuthStatus::SignedIn { user }
            } else {
                AuthStatus::NotSignedIn
            }
        }
        "NotSignedIn" | "MaybeOK" => AuthStatus::NotSignedIn,
        _ => AuthStatus::NotSignedIn,
    };

    Some(CopilotNotification::AuthStatus(auth_status))
}

/// Handle signInInitiate response
fn handle_sign_in_initiate_response(result: Value) -> Option<CopilotNotification> {
    let verification_uri = result.get("verificationUri")?.as_str()?.to_string();
    let user_code = result.get("userCode")?.as_str()?.to_string();
    let expires_in = result.get("expiresIn").and_then(|e| e.as_u64()).unwrap_or(900) as u32;
    let interval = result.get("interval").and_then(|i| i.as_u64()).unwrap_or(5) as u32;

    Some(CopilotNotification::SignInRequired(SignInInfo {
        verification_uri,
        user_code,
        expires_in,
        interval,
    }))
}

/// Handle signInConfirm response
fn handle_sign_in_confirm_response(result: Value) -> Option<CopilotNotification> {
    let status = result.get("status")?.as_str()?;
    let user = result.get("user").and_then(|u| u.as_str()).map(|s| s.to_string());

    let auth_status = match status {
        "OK" | "Normal" => {
            if let Some(user) = user {
                AuthStatus::SignedIn { user }
            } else {
                AuthStatus::Failed {
                    message: "Sign-in succeeded but no user returned".to_string(),
                }
            }
        }
        "NotAuthorized" => AuthStatus::Failed {
            message: "Not authorized".to_string(),
        },
        _ => AuthStatus::Failed {
            message: format!("Unknown status: {}", status),
        },
    };

    Some(CopilotNotification::AuthStatus(auth_status))
}

/// Handle completions response
fn handle_completions_response(
    result: Value,
    request_id: u64,
) -> Option<CopilotNotification> {
    debug_log(&format!("  Parsing completions response: {}", serde_json::to_string(&result).unwrap_or_default()));
    let completions_json = result.get("completions")?.as_array()?;
    debug_log(&format!("  Found {} completions in response", completions_json.len()));

    let completions: Vec<CopilotCompletion> = completions_json
        .iter()
        .enumerate()
        .filter_map(|(index, c)| {
            let uuid = c.get("uuid")?.as_str()?.to_string();
            let text = c.get("text")?.as_str()?.to_string();
            let display_text = c
                .get("displayText")
                .and_then(|d| d.as_str())
                .unwrap_or(&text)
                .to_string();

            let range = c.get("range")?;
            let start = range.get("start")?;
            let end = range.get("end")?;

            Some(CopilotCompletion {
                uuid,
                text,
                display_text,
                range: CopilotRange {
                    start: CopilotPosition {
                        line: start.get("line")?.as_u64()? as u32,
                        character: start.get("character")?.as_u64()? as u32,
                    },
                    end: CopilotPosition {
                        line: end.get("line")?.as_u64()? as u32,
                        character: end.get("character")?.as_u64()? as u32,
                    },
                },
                index,
            })
        })
        .collect();

    Some(CopilotNotification::Completions(CopilotCompletionResult {
        completions,
        request_id,
    }))
}
