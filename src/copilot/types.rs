//! Protocol types for GitHub Copilot communication
//!
//! Copilot uses a custom LSP-style protocol with specific methods like
//! `signInInitiate`, `getCompletions`, etc.

use serde::{Deserialize, Serialize};

/// Copilot server status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CopilotStatus {
    /// Server not started
    #[default]
    Stopped,
    /// Server is starting
    Starting,
    /// Authentication required
    SignInRequired,
    /// Ready to provide completions
    Ready,
    /// Server error or crashed
    Error,
}

impl CopilotStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CopilotStatus::Stopped => "Stopped",
            CopilotStatus::Starting => "Starting...",
            CopilotStatus::SignInRequired => "Sign-in required",
            CopilotStatus::Ready => "Ready",
            CopilotStatus::Error => "Error",
        }
    }
}

/// Authentication status from Copilot
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatus {
    /// Not signed in
    NotSignedIn,
    /// Sign-in in progress (device flow)
    SigningIn,
    /// Signed in as user
    SignedIn { user: String },
    /// Authentication failed
    Failed { message: String },
}

/// Position in a document (UTF-16 code units)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotPosition {
    pub line: u32,
    pub character: u32,
}

/// Range in a document (UTF-16 code units)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotRange {
    pub start: CopilotPosition,
    pub end: CopilotPosition,
}

/// Document information sent to Copilot
#[derive(Debug, Clone, Serialize)]
pub struct CopilotDocument {
    /// File URI (e.g., "file:///path/to/file.rs")
    pub uri: String,
    /// Document version (incremented on each change)
    pub version: i32,
    /// Relative path from workspace root
    #[serde(rename = "relativePath")]
    pub relative_path: String,
    /// Whether to insert spaces (vs tabs)
    #[serde(rename = "insertSpaces")]
    pub insert_spaces: bool,
    /// Tab size in spaces
    #[serde(rename = "tabSize")]
    pub tab_size: u32,
    /// Cursor position (UTF-16)
    pub position: CopilotPosition,
    /// Language ID (e.g., "rust", "typescript")
    #[serde(rename = "languageId")]
    pub language_id: String,
    /// Full document source text (required by Copilot server)
    pub source: String,
}

/// A single completion from Copilot
#[derive(Debug, Clone)]
pub struct CopilotCompletion {
    /// Unique identifier for this completion
    pub uuid: String,
    /// Full text to insert
    pub text: String,
    /// Text to display (may differ from text)
    pub display_text: String,
    /// Range to replace in the document
    pub range: CopilotRange,
    /// Position in the completion list
    pub index: usize,
}

/// Result from getCompletions request
#[derive(Debug, Clone, Default)]
pub struct CopilotCompletionResult {
    /// List of completions
    pub completions: Vec<CopilotCompletion>,
    /// Request ID that produced this result (0 if unknown)
    pub request_id: u64,
}

/// Device flow sign-in information
#[derive(Debug, Clone)]
pub struct SignInInfo {
    /// URL to visit for authentication
    pub verification_uri: String,
    /// Code to enter on the website
    pub user_code: String,
    /// Expiration time in seconds
    pub expires_in: u32,
    /// Polling interval in seconds
    pub interval: u32,
}

/// Notifications from Copilot to the editor
#[derive(Debug, Clone)]
pub enum CopilotNotification {
    /// Server initialized successfully
    Initialized,
    /// Authentication status changed
    AuthStatus(AuthStatus),
    /// Sign-in required - show device flow info
    SignInRequired(SignInInfo),
    /// Completions received
    Completions(CopilotCompletionResult),
    /// Server error
    Error { message: String },
    /// Status message (for logging)
    Status { message: String },
}

/// Request types for tracking pending requests
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopilotRequestKind {
    Initialize,
    CheckStatus,
    SignInInitiate,
    SignInConfirm { user_code: String },
    GetCompletions {
        uri: String,
        version: i32,
        line: u32,
        character: u32,
    },
    GetCompletionsCycling {
        uri: String,
        version: i32,
        line: u32,
        character: u32,
    },
    NotifyAccepted,
    NotifyRejected,
    NotifyShown,
}

/// Editor plugin info sent during initialization
#[derive(Debug, Clone, Serialize)]
pub struct EditorPluginInfo {
    pub name: String,
    pub version: String,
}

/// Editor info sent during initialization
#[derive(Debug, Clone, Serialize)]
pub struct EditorInfo {
    pub name: String,
    pub version: String,
}

/// Initialization options for Copilot server
#[derive(Debug, Clone, Serialize)]
pub struct CopilotInitOptions {
    #[serde(rename = "editorInfo")]
    pub editor_info: EditorInfo,
    #[serde(rename = "editorPluginInfo")]
    pub editor_plugin_info: EditorPluginInfo,
}

/// Workspace configuration sent to Copilot
#[derive(Debug, Clone, Serialize)]
pub struct CopilotConfiguration {
    #[serde(rename = "enableAutoCompletions")]
    pub enable_auto_completions: bool,
    #[serde(rename = "disabledLanguages")]
    pub disabled_languages: Vec<String>,
}
