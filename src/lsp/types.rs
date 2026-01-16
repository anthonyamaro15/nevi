//! Internal types for LSP communication between threads

use std::path::PathBuf;

/// Kind of LSP request - used for tracking responses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Initialize,
    Shutdown,
    Completion,
    Definition,
    Hover,
    SignatureHelp,
}

/// Requests sent from the editor to the LSP client thread
#[derive(Debug, Clone)]
pub enum LspRequest {
    /// Initialize the LSP server with workspace root
    Initialize { root_path: PathBuf },

    /// Shutdown the LSP server
    Shutdown,

    /// Document was opened
    DidOpen {
        uri: String,
        language_id: String,
        version: i32,
        text: String,
    },

    /// Document content changed
    DidChange {
        uri: String,
        version: i32,
        text: String,
    },

    /// Document was closed
    DidClose { uri: String },

    /// Request completions at position
    Completion {
        uri: String,
        line: u32,
        character: u32,
    },

    /// Request go-to-definition
    GotoDefinition {
        uri: String,
        line: u32,
        character: u32,
    },

    /// Request hover information
    Hover {
        uri: String,
        line: u32,
        character: u32,
    },

    /// Request signature help
    SignatureHelp {
        uri: String,
        line: u32,
        character: u32,
    },
}

/// Notifications sent from the LSP client thread to the editor
#[derive(Debug, Clone)]
pub enum LspNotification {
    /// Server initialization complete
    Initialized,

    /// Server failed to start or crashed
    Error { message: String },

    /// Diagnostics for a document
    Diagnostics {
        uri: String,
        diagnostics: Vec<Diagnostic>,
    },

    /// Completion results
    Completions {
        items: Vec<CompletionItem>,
        /// If true, the completion list is incomplete and typing more should re-request
        is_incomplete: bool,
    },

    /// Definition location result (may have multiple locations for traits, etc.)
    Definition {
        locations: Vec<Location>,
    },

    /// Hover information result
    Hover {
        contents: Option<String>,
    },

    /// Signature help result
    SignatureHelp {
        help: Option<SignatureHelpResult>,
    },

    /// Server status update
    Status { message: String },
}

/// Signature help information
#[derive(Debug, Clone)]
pub struct SignatureHelpResult {
    /// Available signatures
    pub signatures: Vec<SignatureInfo>,
    /// Index of the active signature
    pub active_signature: usize,
    /// Index of the active parameter within the active signature
    pub active_parameter: Option<usize>,
}

/// Information about a single signature
#[derive(Debug, Clone)]
pub struct SignatureInfo {
    /// The signature label (full function signature)
    pub label: String,
    /// Documentation for this signature
    pub documentation: Option<String>,
    /// Information about the parameters
    pub parameters: Vec<ParameterInfo>,
}

/// Information about a single parameter
#[derive(Debug, Clone)]
pub struct ParameterInfo {
    /// Start and end offset in the signature label where this parameter appears
    pub label_offsets: Option<(usize, usize)>,
    /// The parameter label text
    pub label: String,
    /// Documentation for this parameter
    pub documentation: Option<String>,
}

/// A diagnostic message (error, warning, etc.)
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub line: usize,
    pub col_start: usize,
    pub col_end: usize,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: Option<String>,
}

/// Severity level for diagnostics
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// A completion item
#[derive(Debug, Clone)]
pub struct CompletionItem {
    /// Display label
    pub label: String,
    /// Kind of completion (function, variable, etc.)
    pub kind: CompletionKind,
    /// Additional detail (type signature)
    pub detail: Option<String>,
    /// Documentation
    pub documentation: Option<String>,
    /// Text to insert (may differ from label)
    pub insert_text: Option<String>,
    /// Text used for filtering (if different from label)
    pub filter_text: Option<String>,
    /// Text used for sorting (if different from label)
    pub sort_text: Option<String>,
}

/// Kind of completion item
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Text,
    Method,
    Function,
    Constructor,
    Field,
    Variable,
    Class,
    Interface,
    Module,
    Property,
    Unit,
    Value,
    Enum,
    Keyword,
    Snippet,
    Color,
    File,
    Reference,
    Folder,
    EnumMember,
    Constant,
    Struct,
    Event,
    Operator,
    TypeParameter,
}

impl CompletionKind {
    /// Get a short display character for the kind
    pub fn short_name(&self) -> &'static str {
        match self {
            CompletionKind::Text => "T",
            CompletionKind::Method => "M",
            CompletionKind::Function => "F",
            CompletionKind::Constructor => "C",
            CompletionKind::Field => "f",
            CompletionKind::Variable => "V",
            CompletionKind::Class => "C",
            CompletionKind::Interface => "I",
            CompletionKind::Module => "m",
            CompletionKind::Property => "P",
            CompletionKind::Unit => "U",
            CompletionKind::Value => "v",
            CompletionKind::Enum => "E",
            CompletionKind::Keyword => "K",
            CompletionKind::Snippet => "S",
            CompletionKind::Color => "c",
            CompletionKind::File => "F",
            CompletionKind::Reference => "R",
            CompletionKind::Folder => "D",
            CompletionKind::EnumMember => "e",
            CompletionKind::Constant => "c",
            CompletionKind::Struct => "S",
            CompletionKind::Event => "E",
            CompletionKind::Operator => "O",
            CompletionKind::TypeParameter => "T",
        }
    }
}

/// A location in a document
#[derive(Debug, Clone)]
pub struct Location {
    pub uri: String,
    pub line: usize,
    pub col: usize,
}

/// LSP server status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspStatus {
    /// Not started
    Stopped,
    /// Starting up
    Starting,
    /// Ready to handle requests
    Ready,
    /// Server error/crashed
    Error,
}
