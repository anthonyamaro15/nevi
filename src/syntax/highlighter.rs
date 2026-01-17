use crossterm::style::Color;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use super::theme::Theme;

/// Maximum query byte range to prevent freezing on minified files
/// (e.g., minified JavaScript with 100KB+ single lines)
const MAX_QUERY_BYTES: usize = 16 * 1024; // 16KB

/// A highlighted span within a line
#[derive(Debug, Clone, Copy)]
pub struct HighlightSpan {
    /// Start column (character index, 0-based)
    pub start_col: usize,
    /// End column (exclusive)
    pub end_col: usize,
    /// Foreground color for this span
    pub fg: Color,
}

/// Get highlights for a specific line from the parsed tree
pub fn get_line_highlights(
    tree: &Tree,
    query: &Query,
    source: &str,
    line_start_bytes: &[usize],
    line: usize,
    theme: &Theme,
) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    let mut cursor = QueryCursor::new();

    // Get the byte range for this line
    if line >= line_start_bytes.len() {
        return spans;
    }

    let line_start_byte = line_start_bytes[line];
    let mut line_end_byte = if line + 1 < line_start_bytes.len() {
        line_start_bytes[line + 1].saturating_sub(1)
    } else {
        source.len()
    };
    if line_end_byte < line_start_byte {
        line_end_byte = line_start_byte;
    }

    // Skip highlighting for very long lines (e.g., minified files)
    // This prevents the editor from freezing on pathological input
    let line_byte_len = line_end_byte.saturating_sub(line_start_byte);
    if line_byte_len > MAX_QUERY_BYTES {
        return spans; // Graceful degradation: no highlighting for this line
    }

    let root = tree.root_node();

    // Query only the nodes that intersect with this line
    cursor.set_byte_range(line_start_byte..line_end_byte);

    let mut matches = cursor.matches(query, root, source.as_bytes());

    while let Some(m) = matches.next() {
        for capture in m.captures {
            let node = capture.node;
            let capture_name = query.capture_names()[capture.index as usize];

            // Get the color for this capture
            if let Some(color) = theme.get_color_for_capture(capture_name) {
                let node_start = node.start_byte();
                let node_end = node.end_byte();

                // Skip if node doesn't intersect with this line
                if node_end <= line_start_byte || node_start >= line_end_byte {
                    continue;
                }

                // Clamp to line boundaries and convert to column indices
                let start_byte = node_start.max(line_start_byte);
                let end_byte = node_end.min(line_end_byte);

                let start_col = start_byte - line_start_byte;
                let end_col = end_byte - line_start_byte;

                if start_col < end_col {
                    spans.push(HighlightSpan {
                        start_col,
                        end_col,
                        fg: color,
                    });
                }
            }
        }
    }

    // Sort spans by start column
    spans.sort_by_key(|s| s.start_col);

    spans
}

/// Get the highlight query for Rust
pub fn rust_highlight_query() -> &'static str {
    // Query using named node types from tree-sitter-rust
    // Avoid anonymous string tokens that may not exist in grammar
    r##"
; Comments (highest priority)
(line_comment) @comment
(block_comment) @comment

; Literals
(string_literal) @string
(raw_string_literal) @string
(char_literal) @string
(boolean_literal) @constant
(integer_literal) @number
(float_literal) @number

; Function definitions and calls
(function_item name: (identifier) @function)
(call_expression function: (identifier) @function.call)
(call_expression function: (field_expression field: (field_identifier) @function.call))
(macro_invocation macro: (identifier) @function.macro)

; Types
(type_identifier) @type
(primitive_type) @type
(generic_type type: (type_identifier) @type)
(scoped_type_identifier name: (type_identifier) @type)

; Struct/enum/trait definitions
(struct_item name: (type_identifier) @type)
(enum_item name: (type_identifier) @type)
(trait_item name: (type_identifier) @type)
(impl_item type: (type_identifier) @type)
(type_item name: (type_identifier) @type)

; Use declarations - capture the module path
(use_declaration argument: (scoped_identifier name: (identifier) @namespace))
(use_declaration argument: (identifier) @namespace)
(mod_item name: (identifier) @namespace)

; Attributes
(attribute_item) @attribute
(inner_attribute_item) @attribute

; Field access
(field_identifier) @property

; Let bindings
(let_declaration pattern: (identifier) @variable)

; Parameters
(parameter pattern: (identifier) @variable.parameter)

; Self
(self) @variable.builtin

; Mutable specifier
(mutable_specifier) @keyword

; Reference operator
(reference_type) @operator
(reference_expression) @operator

; Lifetime
(lifetime) @label
"##
}

/// Get the highlight query for JavaScript/JSX
pub fn javascript_highlight_query() -> &'static str {
    r##"
; Comments
(comment) @comment

; Strings
(string) @string
(template_string) @string

; Numbers
(number) @number

; Booleans
(true) @constant
(false) @constant
(null) @constant

; Functions
(function_declaration name: (identifier) @function)
(call_expression function: (identifier) @function.call)

; Classes
(class_declaration name: (identifier) @type)

; Properties
(property_identifier) @property

; JSX
(jsx_opening_element name: (identifier) @tag)
(jsx_closing_element name: (identifier) @tag)
(jsx_self_closing_element name: (identifier) @tag)

; This
(this) @variable.builtin
"##
}

/// Get the highlight query for TypeScript
pub fn typescript_highlight_query() -> &'static str {
    r##"
; Comments
(comment) @comment

; Strings
(string) @string
(template_string) @string

; Numbers
(number) @number

; Booleans
(true) @constant
(false) @constant
(null) @constant

; Functions
(function_declaration name: (identifier) @function)
(call_expression function: (identifier) @function.call)

; Types
(type_identifier) @type
(predefined_type) @type

; Interfaces
(interface_declaration name: (type_identifier) @type)

; Properties
(property_identifier) @property

; This
(this) @variable.builtin
"##
}

/// Get the highlight query for TSX (TypeScript + JSX)
pub fn tsx_highlight_query() -> &'static str {
    r##"
; Comments
(comment) @comment

; Strings
(string) @string
(template_string) @string

; Numbers
(number) @number

; Booleans
(true) @constant
(false) @constant
(null) @constant

; Functions
(function_declaration name: (identifier) @function)
(call_expression function: (identifier) @function.call)

; Types
(type_identifier) @type
(predefined_type) @type

; Interfaces
(interface_declaration name: (type_identifier) @type)

; Properties
(property_identifier) @property

; JSX
(jsx_opening_element name: (identifier) @tag)
(jsx_closing_element name: (identifier) @tag)
(jsx_self_closing_element name: (identifier) @tag)

; This
(this) @variable.builtin
"##
}

/// Get the highlight query for CSS
pub fn css_highlight_query() -> &'static str {
    r##"
; Comments
(comment) @comment

; Selectors
(tag_name) @tag
(class_name) @type
(id_name) @constant

; Properties
(property_name) @property
(plain_value) @string
(integer_value) @number
(float_value) @number

; Strings
(string_value) @string

; At-rules
(at_keyword) @keyword
"##
}

/// Get the highlight query for SCSS (extends CSS)
pub fn scss_highlight_query() -> &'static str {
    // SCSS uses the same CSS grammar with some extensions
    // We'll use the CSS query which covers most SCSS syntax
    css_highlight_query()
}
