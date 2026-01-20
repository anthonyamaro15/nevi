//! Internal types for LSP communication between threads

use std::path::PathBuf;

/// A text edit returned by LSP formatting
#[derive(Debug, Clone)]
pub struct TextEdit {
    /// Start line (0-indexed)
    pub start_line: usize,
    /// Start column (0-indexed)
    pub start_col: usize,
    /// End line (0-indexed)
    pub end_line: usize,
    /// End column (0-indexed)
    pub end_col: usize,
    /// New text to insert
    pub new_text: String,
}

/// Kind of LSP request - used for tracking responses
/// Includes request context so responses can be validated against current state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestKind {
    Initialize,
    Shutdown,
    Completion {
        uri: String,
        line: u32,
        character: u32,
    },
    Definition {
        uri: String,
        line: u32,
        character: u32,
    },
    Hover {
        uri: String,
        line: u32,
        character: u32,
    },
    SignatureHelp {
        uri: String,
        line: u32,
        character: u32,
    },
    Formatting {
        uri: String,
    },
    References {
        uri: String,
        line: u32,
        character: u32,
    },
    CodeAction {
        uri: String,
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
    },
    Rename {
        uri: String,
        line: u32,
        character: u32,
        new_name: String,
    },
    CompletionResolve {
        /// Label of the item being resolved (for matching response to item)
        label: String,
    },
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

    /// Resolve a completion item to get full documentation
    CompletionResolve {
        /// The raw LSP completion item to resolve
        item: serde_json::Value,
        /// Label for matching the response to the item
        label: String,
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

    /// Request document formatting
    Formatting {
        uri: String,
        tab_size: u32,
    },

    /// Request find references
    References {
        uri: String,
        line: u32,
        character: u32,
    },

    /// Request code actions
    CodeAction {
        uri: String,
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
        diagnostics: Vec<Diagnostic>,
    },

    /// Request rename symbol
    Rename {
        uri: String,
        line: u32,
        character: u32,
        new_name: String,
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
        /// Request context for validation
        request_uri: String,
        request_line: u32,
        request_character: u32,
    },

    /// Definition location result (may have multiple locations for traits, etc.)
    Definition {
        locations: Vec<Location>,
        /// Request context for validation
        request_uri: String,
    },

    /// Hover information result
    Hover {
        contents: Option<String>,
        /// Request context for validation
        request_uri: String,
        request_line: u32,
        request_character: u32,
    },

    /// Signature help result
    SignatureHelp {
        help: Option<SignatureHelpResult>,
        /// Request context for validation
        request_uri: String,
        request_line: u32,
        request_character: u32,
    },

    /// Server status update
    Status { message: String },

    /// Formatting result with text edits to apply
    Formatting {
        edits: Vec<TextEdit>,
        /// Request context for validation
        request_uri: String,
    },

    /// References result
    References {
        locations: Vec<Location>,
        /// Request context for validation
        request_uri: String,
    },

    /// Code actions result
    CodeActions {
        actions: Vec<CodeActionItem>,
        /// Request context for validation
        request_uri: String,
    },

    /// Rename result with workspace edits
    RenameResult {
        /// Edits grouped by file URI
        edits: Vec<(String, Vec<TextEdit>)>,
        /// Request context for validation
        request_uri: String,
    },

    /// Resolved completion item with documentation
    CompletionResolved {
        /// Label of the item that was resolved
        label: String,
        /// Resolved documentation
        documentation: Option<String>,
        /// Resolved detail (may be updated too)
        detail: Option<String>,
    },
}

/// A code action item from LSP
#[derive(Debug, Clone)]
pub struct CodeActionItem {
    /// Display title for the action
    pub title: String,
    /// Kind of action (quickfix, refactor, etc.)
    pub kind: Option<String>,
    /// Whether this is the preferred action
    pub is_preferred: bool,
    /// Edits to apply (if any)
    pub edits: Vec<(String, Vec<TextEdit>)>,
    /// Command to execute (if any)
    pub command: Option<String>,
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
    /// Raw LSP item data for completionItem/resolve
    pub raw_data: Option<serde_json::Value>,
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
    /// Get a short display icon for the kind (Zed-style)
    pub fn icon(&self) -> &'static str {
        match self {
            CompletionKind::Text => "󰦨",      // text icon
            CompletionKind::Method => "󰊕",     // method/function
            CompletionKind::Function => "󰊕",   // function
            CompletionKind::Constructor => "", // constructor
            CompletionKind::Field => "󰜢",      // field
            CompletionKind::Variable => "󰀫",   // variable
            CompletionKind::Class => "󰠱",      // class
            CompletionKind::Interface => "󰜰",  // interface
            CompletionKind::Module => "󰏗",     // module/package
            CompletionKind::Property => "󰖷",   // property
            CompletionKind::Unit => "󰑭",       // unit
            CompletionKind::Value => "󰎠",      // value
            CompletionKind::Enum => "󰕘",       // enum
            CompletionKind::Keyword => "󰌋",    // keyword
            CompletionKind::Snippet => "󰩫",    // snippet
            CompletionKind::Color => "󰏘",      // color
            CompletionKind::File => "󰈔",       // file
            CompletionKind::Reference => "󰈇",  // reference
            CompletionKind::Folder => "󰉋",     // folder
            CompletionKind::EnumMember => "󰕘", // enum member
            CompletionKind::Constant => "󰏿",   // constant
            CompletionKind::Struct => "󰙅",     // struct
            CompletionKind::Event => "󰉁",      // event
            CompletionKind::Operator => "󰆕",   // operator
            CompletionKind::TypeParameter => "󰊄", // type parameter
        }
    }

    /// Get a short ASCII display character for the kind (fallback)
    pub fn short_name(&self) -> &'static str {
        match self {
            CompletionKind::Text => "T",
            CompletionKind::Method => "m",
            CompletionKind::Function => "f",
            CompletionKind::Constructor => "c",
            CompletionKind::Field => "F",
            CompletionKind::Variable => "v",
            CompletionKind::Class => "C",
            CompletionKind::Interface => "I",
            CompletionKind::Module => "M",
            CompletionKind::Property => "P",
            CompletionKind::Unit => "U",
            CompletionKind::Value => "V",
            CompletionKind::Enum => "E",
            CompletionKind::Keyword => "K",
            CompletionKind::Snippet => "S",
            CompletionKind::Color => "c",
            CompletionKind::File => "f",
            CompletionKind::Reference => "R",
            CompletionKind::Folder => "D",
            CompletionKind::EnumMember => "e",
            CompletionKind::Constant => "c",
            CompletionKind::Struct => "s",
            CompletionKind::Event => "E",
            CompletionKind::Operator => "O",
            CompletionKind::TypeParameter => "t",
        }
    }

    /// Get the color for this kind (RGB values)
    pub fn color(&self) -> (u8, u8, u8) {
        match self {
            // Functions/Methods - Blue
            CompletionKind::Method | CompletionKind::Function => (97, 175, 239),
            // Types/Classes - Yellow/Orange
            CompletionKind::Class | CompletionKind::Interface | CompletionKind::Struct => (229, 192, 123),
            // Variables/Fields - Cyan
            CompletionKind::Variable | CompletionKind::Field | CompletionKind::Property => (86, 182, 194),
            // Constants/Values - Purple
            CompletionKind::Constant | CompletionKind::Value | CompletionKind::EnumMember => (198, 120, 221),
            // Enums - Green
            CompletionKind::Enum => (152, 195, 121),
            // Keywords - Red/Pink
            CompletionKind::Keyword => (224, 108, 117),
            // Snippets - Gray
            CompletionKind::Snippet => (150, 150, 160),
            // Modules - Teal
            CompletionKind::Module => (78, 201, 176),
            // Constructors - Orange
            CompletionKind::Constructor => (209, 154, 102),
            // Default - Light blue
            _ => (130, 180, 250),
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
