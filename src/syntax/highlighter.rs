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

    // Extract the line content for byte-to-char conversion
    let line_content = &source[line_start_byte..line_end_byte];

    // Build a byte-to-char mapping for this line
    // This converts tree-sitter byte offsets to character indices for rendering
    let byte_to_char = build_byte_to_char_map(line_content);

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

                // Clamp to line boundaries (in bytes)
                let start_byte = node_start.max(line_start_byte);
                let end_byte = node_end.min(line_end_byte);

                // Convert byte offsets (relative to line start) to char indices
                let start_byte_rel = start_byte - line_start_byte;
                let end_byte_rel = end_byte - line_start_byte;

                let start_col = byte_offset_to_char_index(&byte_to_char, start_byte_rel);
                let end_col = byte_offset_to_char_index(&byte_to_char, end_byte_rel);

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

/// Build a mapping from byte offsets to char indices for a given string
/// Returns a vector where index is byte offset and value is char index
fn build_byte_to_char_map(s: &str) -> Vec<usize> {
    let mut map = Vec::with_capacity(s.len() + 1);
    let mut char_idx = 0;

    for (byte_idx, _c) in s.char_indices() {
        // Fill in the mapping for all bytes of this character
        while map.len() < byte_idx {
            map.push(char_idx);
        }
        map.push(char_idx);
        char_idx += 1;
    }

    // Fill remaining bytes (for the end position)
    while map.len() <= s.len() {
        map.push(char_idx);
    }

    map
}

/// Convert a byte offset to a char index using the precomputed map
fn byte_offset_to_char_index(byte_to_char: &[usize], byte_offset: usize) -> usize {
    if byte_offset < byte_to_char.len() {
        byte_to_char[byte_offset]
    } else if !byte_to_char.is_empty() {
        // Past the end - return the last char index
        *byte_to_char.last().unwrap()
    } else {
        0
    }
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
/// Written for nevi based on tree-sitter-javascript node types
pub fn javascript_highlight_query() -> &'static str {
    r##"
; Literals and constants
(comment) @comment
(string) @string
(template_string) @string
(regex) @string
(number) @number
(true) @constant
(false) @constant
(null) @constant
(undefined) @constant

; Keywords - using tree-sitter's bracket syntax for grouping
["import" "export" "from" "as" "default"] @keyword
["const" "let" "var" "function" "class" "extends" "static" "get" "set"] @keyword
["async" "await" "yield" "new" "delete" "typeof" "instanceof" "in" "of" "void" "with"] @keyword
["if" "else" "switch" "case" "for" "while" "do" "break" "continue" "return" "throw" "try" "catch" "finally"] @keyword

; Functions - definitions and calls
(function_declaration name: (identifier) @function)
(function_expression name: (identifier) @function)
(method_definition name: (property_identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (property_identifier) @function))

; Classes and types
(class_declaration name: (identifier) @type)
(new_expression constructor: (identifier) @type)

; Properties
(property_identifier) @property
(shorthand_property_identifier) @property

; Variables - general catch-all
(identifier) @variable
(this) @variable
(super) @variable

; JSX elements
(jsx_opening_element (identifier) @tag)
(jsx_closing_element (identifier) @tag)
(jsx_self_closing_element (identifier) @tag)
(jsx_attribute (property_identifier) @attribute)

; Operators
["=" "+=" "-=" "*=" "/=" "%=" "+" "-" "*" "/" "%" "==" "===" "!=" "!==" "<" ">" "<=" ">=" "&&" "||" "!" "=>" "..." "??" "&" "|" "^" "~"] @operator
"##
}

/// Get the highlight query for TypeScript
/// Written for nevi based on tree-sitter-typescript node types
pub fn typescript_highlight_query() -> &'static str {
    r##"
; Literals and constants
(comment) @comment
(string) @string
(template_string) @string
(regex) @string
(number) @number
(true) @constant
(false) @constant
(null) @constant
(undefined) @constant

; Keywords - JS base
["import" "export" "from" "as" "default"] @keyword
["const" "let" "var" "function" "class" "extends" "static" "get" "set"] @keyword
["async" "await" "yield" "new" "delete" "typeof" "instanceof" "in" "of" "void" "with"] @keyword
["if" "else" "switch" "case" "for" "while" "do" "break" "continue" "return" "throw" "try" "catch" "finally"] @keyword

; Keywords - TypeScript specific
["type" "interface" "enum" "namespace" "module" "declare" "implements"] @keyword
["public" "private" "protected" "readonly" "abstract" "override"] @keyword
["keyof" "infer" "is" "asserts" "satisfies"] @keyword

; Type annotations - TypeScript's key feature
(type_identifier) @type
(predefined_type) @type
(type_alias_declaration name: (type_identifier) @type)
(interface_declaration name: (type_identifier) @type)
(enum_declaration name: (identifier) @type)

; Functions - definitions and calls
(function_declaration name: (identifier) @function)
(function_expression name: (identifier) @function)
(method_definition name: (property_identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (property_identifier) @function))

; Classes and constructors
(class_declaration name: (type_identifier) @type)
(new_expression constructor: (identifier) @type)

; Properties
(property_identifier) @property
(shorthand_property_identifier) @property

; Variables - general catch-all
(identifier) @variable
(this) @variable
(super) @variable

; Decorators
(decorator "@" @attribute)
(decorator (identifier) @attribute)

; Operators
["=" "+=" "-=" "*=" "/=" "%=" "+" "-" "*" "/" "%" "==" "===" "!=" "!==" "<" ">" "<=" ">=" "&&" "||" "!" "=>" "..." "??" "&" "|" "^" "~"] @operator
"##
}

/// Get the highlight query for TSX (TypeScript + JSX)
/// Written for nevi based on tree-sitter-typescript node types
pub fn tsx_highlight_query() -> &'static str {
    r##"
; Literals and constants
(comment) @comment
(string) @string
(template_string) @string
(regex) @string
(number) @number
(true) @constant
(false) @constant
(null) @constant
(undefined) @constant

; Keywords - JS base
["import" "export" "from" "as" "default"] @keyword
["const" "let" "var" "function" "class" "extends" "static" "get" "set"] @keyword
["async" "await" "yield" "new" "delete" "typeof" "instanceof" "in" "of" "void" "with"] @keyword
["if" "else" "switch" "case" "for" "while" "do" "break" "continue" "return" "throw" "try" "catch" "finally"] @keyword

; Keywords - TypeScript specific
["type" "interface" "enum" "namespace" "module" "declare" "implements"] @keyword
["public" "private" "protected" "readonly" "abstract" "override"] @keyword
["keyof" "infer" "is" "asserts" "satisfies"] @keyword

; Type annotations - TypeScript's key feature
(type_identifier) @type
(predefined_type) @type
(type_alias_declaration name: (type_identifier) @type)
(interface_declaration name: (type_identifier) @type)
(enum_declaration name: (identifier) @type)

; Functions - definitions and calls
(function_declaration name: (identifier) @function)
(function_expression name: (identifier) @function)
(method_definition name: (property_identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (property_identifier) @function))

; Classes and constructors
(class_declaration name: (type_identifier) @type)
(new_expression constructor: (identifier) @type)

; Properties
(property_identifier) @property
(shorthand_property_identifier) @property

; Variables - general catch-all
(identifier) @variable
(this) @variable
(super) @variable

; JSX elements - React components and HTML tags
(jsx_opening_element (identifier) @tag)
(jsx_closing_element (identifier) @tag)
(jsx_self_closing_element (identifier) @tag)
(jsx_attribute (property_identifier) @attribute)

; Decorators
(decorator "@" @attribute)
(decorator (identifier) @attribute)

; Operators
["=" "+=" "-=" "*=" "/=" "%=" "+" "-" "*" "/" "%" "==" "===" "!=" "!==" "<" ">" "<=" ">=" "&&" "||" "!" "=>" "..." "??" "&" "|" "^" "~"] @operator
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

/// Get the highlight query for JSON
pub fn json_highlight_query() -> &'static str {
    r##"
; Strings (keys and values)
(string) @string

; Object keys (property names)
(pair key: (string) @property)

; Numbers
(number) @number

; Booleans
(true) @constant
(false) @constant

; Null
(null) @constant

; Punctuation - optional, can be noisy
; "{" @punctuation
; "}" @punctuation
; "[" @punctuation
; "]" @punctuation
; ":" @punctuation
; "," @punctuation
"##
}

/// Get the highlight query for Markdown
pub fn markdown_highlight_query() -> &'static str {
    r##"
; Heading markers (# ## ### etc.)
(atx_h1_marker) @keyword
(atx_h2_marker) @keyword
(atx_h3_marker) @keyword
(atx_h4_marker) @keyword
(atx_h5_marker) @keyword
(atx_h6_marker) @keyword

; Heading content - the text after #
(atx_heading (inline) @type)

; Setext headings (underlined with === or ---)
(setext_heading) @type
(setext_h1_underline) @keyword
(setext_h2_underline) @keyword

; Fenced code blocks (```code```)
(fenced_code_block_delimiter) @punctuation
(info_string (language) @label)
(code_fence_content) @string

; Indented code blocks
(indented_code_block) @string

; Block quotes
(block_quote_marker) @comment
(block_quote (paragraph) @comment)

; List markers
(list_marker_minus) @operator
(list_marker_plus) @operator
(list_marker_star) @operator
(list_marker_dot) @operator

; Thematic breaks (horizontal rules ---, ***, ___)
(thematic_break) @comment

; HTML blocks in markdown
(html_block) @tag
"##
}

/// Get the highlight query for TOML
pub fn toml_highlight_query() -> &'static str {
    r##"
; Comments
(comment) @comment

; Table headers - capture the key inside tables
(table (bare_key) @type)
(table (quoted_key) @type)
(table (dotted_key (bare_key) @type))
(table_array_element (bare_key) @type)
(table_array_element (quoted_key) @type)
(table_array_element (dotted_key (bare_key) @type))

; Keys in key-value pairs
(pair (bare_key) @property)
(pair (quoted_key) @property)
(pair (dotted_key (bare_key) @property))

; Strings (all string types use the same node)
(string) @string

; Numbers
(integer) @number
(float) @number

; Booleans
(boolean) @constant

; Dates and times
(offset_date_time) @string
(local_date_time) @string
(local_date) @string
(local_time) @string
"##
}
