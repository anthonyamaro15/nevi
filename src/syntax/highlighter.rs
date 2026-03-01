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

/// Internal span with priority for sorting
#[derive(Debug, Clone, Copy)]
struct PrioritySpan {
    start_col: usize,
    end_col: usize,
    fg: Color,
    /// Capture index - higher = later in query = higher priority
    priority: u32,
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
    let mut spans: Vec<PrioritySpan> = Vec::new();
    let mut cursor = QueryCursor::new();

    // Get the byte range for this line
    if line >= line_start_bytes.len() {
        return Vec::new();
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
        return Vec::new(); // Graceful degradation: no highlighting for this line
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
                    spans.push(PrioritySpan {
                        start_col,
                        end_col,
                        fg: color,
                        priority: capture.index,
                    });
                }
            }
        }
    }

    // Sort spans by start column, then by priority (higher priority = later in query = wins)
    spans.sort_by(|a, b| {
        a.start_col.cmp(&b.start_col)
            .then_with(|| a.priority.cmp(&b.priority))
    });

    // Resolve overlapping spans using priority
    resolve_overlapping_spans_with_priority(spans)
}

/// Get highlights for a YAML line using lightweight tokenization.
/// This is used when tree-sitter YAML grammar is not available.
pub fn get_line_highlights_yaml(
    source: &str,
    line_start_bytes: &[usize],
    line: usize,
    theme: &Theme,
) -> Vec<HighlightSpan> {
    if line >= line_start_bytes.len() {
        return Vec::new();
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
    if line_end_byte > source.len() {
        line_end_byte = source.len();
    }

    let line_content = &source[line_start_byte..line_end_byte];
    if line_content.is_empty() {
        return Vec::new();
    }

    let byte_to_char = build_byte_to_char_map(line_content);
    let comment_start = yaml_comment_start(line_content);
    let parse_end = comment_start.unwrap_or(line_content.len());
    let mut spans: Vec<PrioritySpan> = Vec::new();

    let mut push_span = |start_byte: usize, end_byte: usize, capture: &str, priority: u32| {
        if start_byte >= end_byte || end_byte > line_content.len() {
            return;
        }
        if let Some(color) = theme.get_color_for_capture(capture) {
            let start_col = byte_offset_to_char_index(&byte_to_char, start_byte);
            let end_col = byte_offset_to_char_index(&byte_to_char, end_byte);
            if start_col < end_col {
                spans.push(PrioritySpan {
                    start_col,
                    end_col,
                    fg: color,
                    priority,
                });
            }
        }
    };

    if let Some((start, end)) = yaml_key_range(line_content, parse_end) {
        push_span(start, end, "property", 10);
    }

    for (start, end) in yaml_quoted_spans(line_content, parse_end) {
        push_span(start, end, "string", 30);
    }

    for (start, end, kind) in yaml_scalar_spans(line_content, parse_end) {
        match kind {
            YamlScalarKind::Boolean => push_span(start, end, "boolean", 20),
            YamlScalarKind::Null => push_span(start, end, "constant", 20),
            YamlScalarKind::Number => push_span(start, end, "number", 20),
        }
    }

    if let Some(start) = comment_start {
        push_span(start, line_content.len(), "comment", 40);
    }

    resolve_overlapping_spans_with_priority(spans)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YamlScalarKind {
    Boolean,
    Null,
    Number,
}

fn yaml_comment_start(line: &str) -> Option<usize> {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for (idx, ch) in line.char_indices() {
        if in_double && escaped {
            escaped = false;
            continue;
        }
        if in_double && ch == '\\' {
            escaped = true;
            continue;
        }
        if !in_double && ch == '\'' {
            in_single = !in_single;
            continue;
        }
        if !in_single && ch == '"' {
            in_double = !in_double;
            continue;
        }
        if ch == '#' && !in_single && !in_double {
            return Some(idx);
        }
    }

    None
}

fn yaml_key_range(line: &str, parse_end: usize) -> Option<(usize, usize)> {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut colon_idx = None;

    for (idx, ch) in line.char_indices() {
        if idx >= parse_end {
            break;
        }
        if in_double && escaped {
            escaped = false;
            continue;
        }
        if in_double && ch == '\\' {
            escaped = true;
            continue;
        }
        if !in_double && ch == '\'' {
            in_single = !in_single;
            continue;
        }
        if !in_single && ch == '"' {
            in_double = !in_double;
            continue;
        }
        if ch == ':' && !in_single && !in_double {
            colon_idx = Some(idx);
            break;
        }
    }

    let colon = colon_idx?;
    let mut start = 0usize;
    while start < colon {
        let ch = line[start..].chars().next()?;
        if ch.is_whitespace() {
            start += ch.len_utf8();
        } else {
            break;
        }
    }

    // Handle list-item maps: "- name: value"
    if start < colon {
        let ch = line[start..].chars().next()?;
        if ch == '-' {
            start += ch.len_utf8();
            while start < colon {
                let ch = line[start..].chars().next()?;
                if ch.is_whitespace() {
                    start += ch.len_utf8();
                } else {
                    break;
                }
            }
        }
    }

    let mut end = colon;
    while end > start {
        let (prev_idx, ch) = line[..end].char_indices().last()?;
        if ch.is_whitespace() {
            end = prev_idx;
        } else {
            break;
        }
    }

    if start >= end {
        return None;
    }

    let candidate = &line[start..end];
    if !candidate.starts_with('"')
        && !candidate.starts_with('\'')
        && candidate.chars().any(|c| c.is_whitespace())
    {
        return None;
    }

    Some((start, end))
}

fn yaml_quoted_spans(line: &str, parse_end: usize) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut in_single: Option<usize> = None;
    let mut in_double: Option<usize> = None;
    let mut escaped = false;

    for (idx, ch) in line.char_indices() {
        if idx >= parse_end {
            break;
        }
        if let Some(start) = in_double {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                spans.push((start, idx + ch.len_utf8()));
                in_double = None;
            }
            continue;
        }
        if let Some(start) = in_single {
            if ch == '\'' {
                spans.push((start, idx + ch.len_utf8()));
                in_single = None;
            }
            continue;
        }
        if ch == '"' {
            in_double = Some(idx);
        } else if ch == '\'' {
            in_single = Some(idx);
        }
    }

    if let Some(start) = in_double {
        spans.push((start, parse_end));
    }
    if let Some(start) = in_single {
        spans.push((start, parse_end));
    }

    spans
}

fn yaml_scalar_spans(line: &str, parse_end: usize) -> Vec<(usize, usize, YamlScalarKind)> {
    let mut spans = Vec::new();
    let mut idx = 0usize;

    while idx < parse_end {
        let ch = match line[idx..].chars().next() {
            Some(ch) => ch,
            None => break,
        };

        if ch == '"' {
            idx += ch.len_utf8();
            let mut escaped = false;
            while idx < parse_end {
                let q = match line[idx..].chars().next() {
                    Some(q) => q,
                    None => break,
                };
                idx += q.len_utf8();
                if escaped {
                    escaped = false;
                    continue;
                }
                if q == '\\' {
                    escaped = true;
                    continue;
                }
                if q == '"' {
                    break;
                }
            }
            continue;
        }

        if ch == '\'' {
            idx += ch.len_utf8();
            while idx < parse_end {
                let q = match line[idx..].chars().next() {
                    Some(q) => q,
                    None => break,
                };
                idx += q.len_utf8();
                if q == '\'' {
                    break;
                }
            }
            continue;
        }

        if ch == '~' {
            let start = idx;
            idx += ch.len_utf8();
            spans.push((start, idx, YamlScalarKind::Null));
            continue;
        }

        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            let start = idx;
            idx += ch.len_utf8();
            while idx < parse_end {
                let next = match line[idx..].chars().next() {
                    Some(next) => next,
                    None => break,
                };
                if next.is_ascii_alphanumeric() || next == '-' || next == '_' || next == '.' {
                    idx += next.len_utf8();
                } else {
                    break;
                }
            }

            let token = &line[start..idx];
            let lower = token.to_ascii_lowercase();
            if matches!(lower.as_str(), "true" | "false" | "yes" | "no" | "on" | "off") {
                spans.push((start, idx, YamlScalarKind::Boolean));
            } else if lower == "null" {
                spans.push((start, idx, YamlScalarKind::Null));
            } else if yaml_token_is_number(token) {
                spans.push((start, idx, YamlScalarKind::Number));
            }
            continue;
        }

        idx += ch.len_utf8();
    }

    spans
}

fn yaml_token_is_number(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }

    let normalized = token.replace('_', "");
    if normalized.is_empty() || normalized == "-" {
        return false;
    }

    if let Some(hex) = normalized.strip_prefix("0x").or_else(|| normalized.strip_prefix("0X")) {
        return !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    if let Some(bin) = normalized.strip_prefix("0b").or_else(|| normalized.strip_prefix("0B")) {
        return !bin.is_empty() && bin.chars().all(|c| matches!(c, '0' | '1'));
    }
    if let Some(oct) = normalized.strip_prefix("0o").or_else(|| normalized.strip_prefix("0O")) {
        return !oct.is_empty() && oct.chars().all(|c| ('0'..='7').contains(&c));
    }

    normalized.parse::<f64>().is_ok()
}

/// Resolve overlapping spans using priority (capture index)
/// Higher priority captures override lower priority ones in overlapping regions
fn resolve_overlapping_spans_with_priority(spans: Vec<PrioritySpan>) -> Vec<HighlightSpan> {
    if spans.is_empty() {
        return Vec::new();
    }

    // Find the max column we need to cover
    let max_col = spans.iter().map(|s| s.end_col).max().unwrap_or(0);
    if max_col == 0 {
        return Vec::new();
    }

    // For each column position, track the highest priority span covering it
    let mut col_colors: Vec<Option<(Color, u32)>> = vec![None; max_col];

    for span in &spans {
        for col in span.start_col..span.end_col {
            if col < col_colors.len() {
                match col_colors[col] {
                    None => col_colors[col] = Some((span.fg, span.priority)),
                    Some((_, existing_priority)) if span.priority > existing_priority => {
                        col_colors[col] = Some((span.fg, span.priority));
                    }
                    _ => {} // Keep existing higher priority
                }
            }
        }
    }

    // Convert column-based representation back to spans
    let mut result: Vec<HighlightSpan> = Vec::new();
    let mut current_span: Option<(usize, Color)> = None;

    for (col, color_opt) in col_colors.iter().enumerate() {
        match (current_span, color_opt) {
            (None, Some((color, _))) => {
                // Start a new span
                current_span = Some((col, *color));
            }
            (Some((_start, current_color)), Some((color, _))) if current_color == *color => {
                // Continue same span
            }
            (Some((start, current_color)), Some((color, _))) => {
                // Different color - end current and start new
                result.push(HighlightSpan {
                    start_col: start,
                    end_col: col,
                    fg: current_color,
                });
                current_span = Some((col, *color));
            }
            (Some((start, current_color)), None) => {
                // End current span
                result.push(HighlightSpan {
                    start_col: start,
                    end_col: col,
                    fg: current_color,
                });
                current_span = None;
            }
            (None, None) => {}
        }
    }

    // Don't forget the last span
    if let Some((start, color)) = current_span {
        result.push(HighlightSpan {
            start_col: start,
            end_col: max_col,
            fg: color,
        });
    }

    result
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
    // Query using tree-sitter-rust node types and core keyword tokens.
    r##"
; Comments (highest priority)
(line_comment) @comment
(block_comment) @comment

; Literals
(string_literal) @string
(raw_string_literal) @string
(char_literal) @string
(integer_literal) @number
(float_literal) @number

; Boolean literals - distinct from other constants
(boolean_literal) @boolean

; Core Rust keywords
[
  "fn"
  "let"
  "impl"
  "trait"
  "struct"
  "enum"
  "type"
  "const"
  "static"
  "match"
  "if"
  "else"
  "for"
  "while"
  "loop"
  "in"
  "return"
  "break"
  "continue"
  "use"
  "mod"
  "where"
  "as"
  "unsafe"
  "async"
  "await"
] @keyword

; Mutable specifier
(mutable_specifier) @keyword

; Visibility modifier (pub)
(visibility_modifier) @keyword

; Macro definition
(macro_definition) @keyword

; Macros - distinct from functions (cyan)
(macro_invocation macro: (identifier) @function.macro)
(macro_invocation macro: (scoped_identifier name: (identifier) @function.macro))

; Function definitions
(function_item name: (identifier) @function)

; Function calls (regular function calls)
(call_expression function: (identifier) @function.call)

; Method calls - distinct from function calls
(call_expression function: (field_expression field: (field_identifier) @function.method))

; Enum constructors (Some, None, Ok, Err) - must come before general type matching
((identifier) @constructor
 (#match? @constructor "^(Some|None|Ok|Err)$"))

; Types
(type_identifier) @type
(primitive_type) @type
(generic_type type: (type_identifier) @type)
(scoped_type_identifier name: (type_identifier) @type)

; Types in scoped paths (std::path::PathBuf) - match PascalCase identifiers in paths
(scoped_identifier name: (identifier) @type
 (#match? @type "^[A-Z]"))

; Generic type arguments (Vec<String>, Result<T, E>)
(type_arguments (type_identifier) @type)

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

; Reference operator
(reference_type) @operator
(reference_expression) @operator

; Lifetime
(lifetime) @label

; Constants (SCREAMING_CASE) - lower priority, checked last
((identifier) @constant
 (#match? @constant "^[A-Z][A-Z0-9_]*$"))
"##
}

/// Get the highlight query for JavaScript/JSX
/// Written for nevi based on tree-sitter-javascript node types
pub fn javascript_highlight_query() -> &'static str {
    r##"
; Literals and constants
(comment) @comment
(string) @string
(regex) @string
(number) @number
(true) @constant
(false) @constant
(null) @constant
(undefined) @constant

; Template strings base
(template_string) @string

; Keywords - using tree-sitter's bracket syntax for grouping
["import" "export" "from" "as" "default"] @keyword
["const" "let" "var" "function" "class" "extends" "static" "get" "set"] @keyword
["async" "await" "yield" "new" "delete" "typeof" "instanceof" "in" "of" "void" "with"] @keyword
["if" "else" "switch" "case" "for" "while" "do" "break" "continue" "return" "throw" "try" "catch" "finally"] @keyword

; Operators
["=" "+=" "-=" "*=" "/=" "%=" "+" "-" "*" "/" "%" "==" "===" "!=" "!==" "<" ">" "<=" ">=" "&&" "||" "!" "=>" "..." "??" "&" "|" "^" "~"] @operator

; Variables - general catch-all (MUST come before more specific patterns)
(identifier) @variable
(this) @variable
(super) @variable

; Properties (override variable)
(property_identifier) @property
(shorthand_property_identifier) @property

; Functions - definitions and calls (override variable)
(function_declaration name: (identifier) @function)
(function_expression name: (identifier) @function)
(method_definition name: (property_identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (property_identifier) @function))

; Classes and types (override variable)
(class_declaration name: (identifier) @type)
(new_expression constructor: (identifier) @type)

; JSX elements (override variable)
(jsx_opening_element (identifier) @tag)
(jsx_closing_element (identifier) @tag)
(jsx_self_closing_element (identifier) @tag)
(jsx_attribute (property_identifier) @attribute)

; Template string interpolations - highest priority
(template_substitution
  "${" @embedded
  "}" @embedded)
(template_substitution (identifier) @embedded)
(template_substitution (member_expression) @embedded)
"##
}

/// Get the highlight query for TypeScript
/// Written for nevi based on tree-sitter-typescript node types
pub fn typescript_highlight_query() -> &'static str {
    r##"
; Literals and constants
(comment) @comment
(string) @string
(regex) @string
(number) @number
(true) @constant
(false) @constant
(null) @constant
(undefined) @constant

; Template strings base
(template_string) @string

; Keywords - JS base
["import" "export" "from" "as" "default"] @keyword
["const" "let" "var" "function" "class" "extends" "static" "get" "set"] @keyword
["async" "await" "yield" "new" "delete" "typeof" "instanceof" "in" "of" "void" "with"] @keyword
["if" "else" "switch" "case" "for" "while" "do" "break" "continue" "return" "throw" "try" "catch" "finally"] @keyword

; Keywords - TypeScript specific
["type" "interface" "enum" "namespace" "module" "declare" "implements"] @keyword
["public" "private" "protected" "readonly" "abstract" "override"] @keyword
["keyof" "infer" "is" "asserts" "satisfies"] @keyword

; Operators
["=" "+=" "-=" "*=" "/=" "%=" "+" "-" "*" "/" "%" "==" "===" "!=" "!==" "<" ">" "<=" ">=" "&&" "||" "!" "=>" "..." "??" "&" "|" "^" "~"] @operator

; Variables - general catch-all (MUST come before more specific patterns)
(identifier) @variable
(this) @variable
(super) @variable

; Properties (override variable)
(property_identifier) @property
(shorthand_property_identifier) @property

; Type annotations - TypeScript's key feature (override variable)
(type_identifier) @type
(predefined_type) @type
(type_alias_declaration name: (type_identifier) @type)
(interface_declaration name: (type_identifier) @type)
(enum_declaration name: (identifier) @type)

; Functions - definitions and calls (override variable)
(function_declaration name: (identifier) @function)
(function_expression name: (identifier) @function)
(method_definition name: (property_identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (property_identifier) @function))

; Classes and constructors (override variable)
(class_declaration name: (type_identifier) @type)
(new_expression constructor: (identifier) @type)

; Decorators (override variable)
(decorator "@" @attribute)
(decorator (identifier) @attribute)

; Template string interpolations - highest priority
(template_substitution
  "${" @embedded
  "}" @embedded)
(template_substitution (identifier) @embedded)
(template_substitution (member_expression) @embedded)
"##
}

/// Get the highlight query for TSX (TypeScript + JSX)
/// Written for nevi based on tree-sitter-typescript node types
pub fn tsx_highlight_query() -> &'static str {
    r##"
; Literals and constants
(comment) @comment
(string) @string
(regex) @string
(number) @number
(true) @constant
(false) @constant
(null) @constant
(undefined) @constant

; Template strings base
(template_string) @string

; Keywords - JS base
["import" "export" "from" "as" "default"] @keyword
["const" "let" "var" "function" "class" "extends" "static" "get" "set"] @keyword
["async" "await" "yield" "new" "delete" "typeof" "instanceof" "in" "of" "void" "with"] @keyword
["if" "else" "switch" "case" "for" "while" "do" "break" "continue" "return" "throw" "try" "catch" "finally"] @keyword

; Keywords - TypeScript specific
["type" "interface" "enum" "namespace" "module" "declare" "implements"] @keyword
["public" "private" "protected" "readonly" "abstract" "override"] @keyword
["keyof" "infer" "is" "asserts" "satisfies"] @keyword

; Operators
["=" "+=" "-=" "*=" "/=" "%=" "+" "-" "*" "/" "%" "==" "===" "!=" "!==" "<" ">" "<=" ">=" "&&" "||" "!" "=>" "..." "??" "&" "|" "^" "~"] @operator

; Variables - general catch-all (MUST come before more specific patterns)
(identifier) @variable
(this) @variable
(super) @variable

; Properties (override variable)
(property_identifier) @property
(shorthand_property_identifier) @property

; Type annotations - TypeScript's key feature (override variable)
(type_identifier) @type
(predefined_type) @type
(type_alias_declaration name: (type_identifier) @type)
(interface_declaration name: (type_identifier) @type)
(enum_declaration name: (identifier) @type)

; Functions - definitions and calls (override variable)
(function_declaration name: (identifier) @function)
(function_expression name: (identifier) @function)
(method_definition name: (property_identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (property_identifier) @function))

; Classes and constructors (override variable)
(class_declaration name: (type_identifier) @type)
(new_expression constructor: (identifier) @type)

; JSX elements - React components and HTML tags (override variable)
(jsx_opening_element (identifier) @tag)
(jsx_closing_element (identifier) @tag)
(jsx_self_closing_element (identifier) @tag)
(jsx_attribute (property_identifier) @attribute)

; Decorators (override variable)
(decorator "@" @attribute)
(decorator (identifier) @attribute)

; Template string interpolations - highest priority
(template_substitution
  "${" @embedded
  "}" @embedded)
(template_substitution (identifier) @embedded)
(template_substitution (member_expression) @embedded)
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

/// Get the highlight query for HTML
pub fn html_highlight_query() -> &'static str {
    r##"
; Comments
(comment) @comment

; DOCTYPE declaration
(doctype) @keyword

; Tag names in start tags
(start_tag (tag_name) @tag)

; Tag names in end tags
(end_tag (tag_name) @tag)

; Tag names in self-closing tags
(self_closing_tag (tag_name) @tag)

; Attribute names (use @attribute for yellow/orange like Neovim)
(attribute (attribute_name) @attribute)

; Attribute values (green)
(attribute (quoted_attribute_value) @string)
(attribute (attribute_value) @string)

; Script and style content (raw text inside these elements)
(script_element (raw_text) @string)
(style_element (raw_text) @string)

; Text content - don't highlight by default (matches Neovim behavior)
; (text) @variable
"##
}

/// Get the highlight query for Python
pub fn python_highlight_query() -> &'static str {
    r##"
; Comments
(comment) @comment

; Strings (including docstrings)
(string) @string

; Numbers
(integer) @number
(float) @number

; Boolean and None literals
(true) @constant
(false) @constant
(none) @constant

; Keywords - these are named nodes in tree-sitter-python
[
  "import"
  "from"
  "as"
  "def"
  "class"
  "return"
  "yield"
  "pass"
  "break"
  "continue"
  "if"
  "elif"
  "else"
  "for"
  "in"
  "while"
  "try"
  "except"
  "finally"
  "raise"
  "with"
  "assert"
  "del"
  "global"
  "nonlocal"
  "lambda"
  "and"
  "or"
  "not"
  "is"
  "async"
  "await"
  "match"
  "case"
] @keyword

; Function definitions
(function_definition name: (identifier) @function)

; Function calls
(call function: (identifier) @function)
(call function: (attribute attribute: (identifier) @function))

; Class definitions
(class_definition name: (identifier) @type)

; Decorators
(decorator (identifier) @attribute)

; Type annotations
(type (identifier) @type)

; Attribute access
(attribute attribute: (identifier) @property)
"##
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    #[test]
    fn rust_highlight_query_compiles() {
        let language = tree_sitter_rust::LANGUAGE;
        let query = Query::new(&language.into(), rust_highlight_query());
        assert!(query.is_ok(), "Rust query failed to compile: {:?}", query.err());
    }

    #[test]
    fn javascript_highlight_query_compiles() {
        let language = tree_sitter_javascript::LANGUAGE;
        let query = Query::new(&language.into(), javascript_highlight_query());
        assert!(
            query.is_ok(),
            "JavaScript query failed to compile: {:?}",
            query.err()
        );
    }

    #[test]
    fn typescript_highlight_query_compiles() {
        let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT;
        let query = Query::new(&language.into(), typescript_highlight_query());
        assert!(
            query.is_ok(),
            "TypeScript query failed to compile: {:?}",
            query.err()
        );
    }

    #[test]
    fn rust_query_captures_core_keywords() {
        let language = tree_sitter_rust::LANGUAGE;
        let query = Query::new(&language.into(), rust_highlight_query())
            .expect("rust query should compile");

        let mut parser = Parser::new();
        parser
            .set_language(&language.into())
            .expect("rust parser should initialize");
        let source = "impl Foo { fn bar() {} }\nfn main() { let x = match 1 { _ => 1 }; }\n";
        let tree = parser.parse(source, None).expect("parse rust source");

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        let mut keyword_count = 0usize;

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                if capture_name == "keyword" {
                    keyword_count += 1;
                }
            }
        }

        assert!(
            keyword_count >= 5,
            "Expected core keyword captures (fn/let/impl/match), got {}",
            keyword_count
        );
    }

    #[test]
    fn yaml_line_highlights_key_string_and_comment() {
        let source = "name: \"nevi\" # app name";
        let line_starts = vec![0];
        let theme = Theme::default();
        let spans = get_line_highlights_yaml(source, &line_starts, 0, &theme);

        let property = theme
            .get_color_for_capture("property")
            .expect("property color");
        let string = theme.get_color_for_capture("string").expect("string color");
        let comment = theme
            .get_color_for_capture("comment")
            .expect("comment color");

        assert!(spans.iter().any(|s| s.fg == property && s.start_col <= 0 && s.end_col >= 4));
        assert!(spans.iter().any(|s| s.fg == string && s.start_col <= 7 && s.end_col >= 10));
        assert!(spans.iter().any(|s| s.fg == comment && s.start_col <= 13 && s.end_col >= 15));
    }

    #[test]
    fn yaml_line_highlights_boolean_and_number_scalars() {
        let theme = Theme::default();

        let bool_source = "enabled: true";
        let bool_spans = get_line_highlights_yaml(bool_source, &[0], 0, &theme);
        let boolean = theme
            .get_color_for_capture("boolean")
            .expect("boolean color");
        assert!(bool_spans.iter().any(|s| s.fg == boolean && s.start_col <= 9 && s.end_col >= 12));

        let num_source = "port: 8080";
        let num_spans = get_line_highlights_yaml(num_source, &[0], 0, &theme);
        let number = theme.get_color_for_capture("number").expect("number color");
        assert!(num_spans.iter().any(|s| s.fg == number && s.start_col <= 6 && s.end_col >= 9));
    }
}
