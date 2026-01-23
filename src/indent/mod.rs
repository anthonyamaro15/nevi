//! Smart indentation module using tree-sitter syntax analysis
//!
//! This module provides tree-sitter based indentation calculation for
//! JavaScript, TypeScript, TSX, and JSX files.

use tree_sitter::Tree;

/// Node types that increase indentation for their children
const INDENT_NODES: &[&str] = &[
    // === JavaScript/TypeScript ===
    // Block structures
    "statement_block",
    "class_body",
    "switch_body",
    "enum_body",
    "interface_body",
    "object_type",
    // Literals and expressions
    "object",
    "object_pattern",
    "array",
    "array_pattern",
    "arguments",
    "formal_parameters",
    "template_string",
    // JSX
    "jsx_element",
    "jsx_opening_element",
    "jsx_self_closing_element",
    "jsx_expression",
    // Other JS/TS
    "named_imports",
    "export_clause",
    "switch_case",
    "switch_default",
    "parenthesized_expression",

    // === CSS/SCSS ===
    "block",              // selector { ... }
    "declaration_list",   // same as block in some grammars
    "media_statement",
    "keyframe_block_list",
    "feature_query",

    // === JSON ===
    // "object" and "array" already included above

    // === TOML ===
    "inline_table",       // { key = value }
    // "array" already included above

    // === HTML ===
    "element",
    "script_element",
    "style_element",
];

/// Calculate the indentation level for a new line after pressing Enter.
///
/// # Arguments
/// * `tree` - The parsed tree-sitter syntax tree
/// * `source` - The source code as a string
/// * `cursor_byte` - The byte offset of the cursor in the source
/// * `tab_width` - The width of one indentation level in spaces
///
/// # Returns
/// The number of spaces to indent the new line
pub fn calculate_indent(
    tree: &Tree,
    source: &str,
    cursor_byte: usize,
    tab_width: usize,
) -> usize {
    // Get the base indent from the current line
    let line_indent = get_line_indent_at_byte(source, cursor_byte);
    let base_level = line_indent / tab_width;

    // Check if the line ends with an opening delimiter that should increase indent
    let line_before_cursor = get_line_before_cursor(source, cursor_byte);
    let trimmed = line_before_cursor.trim_end();

    // If line ends with opening bracket, increase indent
    if trimmed.ends_with('{') || trimmed.ends_with('[') || trimmed.ends_with('(') {
        return (base_level + 1) * tab_width;
    }

    // For JSX opening tags, increase indent
    if is_jsx_opening_tag_line(source, cursor_byte) {
        return (base_level + 1) * tab_width;
    }

    // Use tree-sitter to check for nested structures
    let root = tree.root_node();
    let node = find_node_at_position(root, cursor_byte);

    if let Some(node) = node {
        // Count unclosed indent-increasing nodes
        let mut indent_level: usize = 0;
        let mut current = Some(node);

        while let Some(n) = current {
            let kind = n.kind();

            if INDENT_NODES.contains(&kind) {
                // Check if cursor is inside this node (not at boundaries)
                let start = n.start_byte();
                let end = n.end_byte();

                if cursor_byte > start && cursor_byte < end {
                    indent_level += 1;
                }
            }

            current = n.parent();
        }

        if indent_level > 0 {
            return indent_level * tab_width;
        }
    }

    // Fallback: maintain the current line's indent
    line_indent
}

/// Calculate the expected indent for a closing bracket based on its matching opener.
///
/// # Arguments
/// * `tree` - The parsed tree-sitter syntax tree
/// * `source` - The source code as a string
/// * `cursor_byte` - The byte offset of the cursor
/// * `bracket` - The closing bracket character (}, ], or ))
///
/// # Returns
/// The number of spaces the closing bracket should be indented
pub fn calculate_closing_bracket_indent(
    tree: &Tree,
    source: &str,
    cursor_byte: usize,
    bracket: char,
) -> Option<usize> {
    let root = tree.root_node();

    // Find the node at cursor position
    let node = find_node_at_position(root, cursor_byte)?;

    // Walk up to find the matching container
    let mut current = Some(node);
    let container_kinds = match bracket {
        '}' => vec!["statement_block", "class_body", "object", "object_pattern",
                   "switch_body", "enum_body", "interface_body", "object_type",
                   "named_imports", "export_clause"],
        ']' => vec!["array", "array_pattern"],
        ')' => vec!["arguments", "formal_parameters", "parenthesized_expression"],
        _ => return None,
    };

    while let Some(n) = current {
        let kind = n.kind();

        if container_kinds.contains(&kind) {
            // Found the container - return the indent of the line where it starts
            let start_byte = n.start_byte();
            let line_indent = get_line_indent_at_byte(source, start_byte);
            return Some(line_indent);
        }

        current = n.parent();
    }

    None
}

/// Check if a closing bracket should trigger auto-dedent.
///
/// This returns true if:
/// 1. The line before the cursor contains only whitespace
/// 2. The current indentation is greater than the expected indent for the bracket
///
/// # Arguments
/// * `tree` - The parsed tree-sitter syntax tree
/// * `source` - The source code as a string
/// * `cursor_byte` - The byte offset of the cursor
/// * `bracket` - The closing bracket character
/// * `tab_width` - The width of one indentation level in spaces
///
/// # Returns
/// `true` if the bracket should trigger dedent
pub fn should_dedent_closing_bracket(
    tree: &Tree,
    source: &str,
    cursor_byte: usize,
    bracket: char,
    tab_width: usize,
) -> bool {
    // First check if we're on a line with only whitespace before cursor
    let line_before = get_line_before_cursor(source, cursor_byte);
    if !line_before.chars().all(|c| c == ' ' || c == '\t') {
        return false;
    }

    let current_indent = line_before.len();

    // Get the expected indent for this closing bracket
    if let Some(expected_indent) = calculate_closing_bracket_indent(tree, source, cursor_byte, bracket) {
        // Dedent if current indent is more than expected
        return current_indent > expected_indent;
    }

    // Fallback: dedent if we have at least one level of indent
    current_indent >= tab_width
}

/// Get the amount to dedent for a closing bracket.
///
/// # Returns
/// The number of spaces to remove, or 0 if no dedent needed
pub fn get_dedent_amount(
    tree: &Tree,
    source: &str,
    cursor_byte: usize,
    bracket: char,
    tab_width: usize,
) -> usize {
    let line_before = get_line_before_cursor(source, cursor_byte);
    if !line_before.chars().all(|c| c == ' ' || c == '\t') {
        return 0;
    }

    let current_indent = line_before.len();

    if let Some(expected_indent) = calculate_closing_bracket_indent(tree, source, cursor_byte, bracket) {
        if current_indent > expected_indent {
            return current_indent - expected_indent;
        }
    } else if current_indent >= tab_width {
        // Fallback: dedent one level
        return tab_width;
    }

    0
}

/// Find the deepest node at or containing the given byte position
fn find_node_at_position(root: tree_sitter::Node, byte: usize) -> Option<tree_sitter::Node> {
    let mut cursor = root.walk();
    let mut result = None;

    loop {
        let node = cursor.node();

        // Check if byte is within this node's range
        if byte >= node.start_byte() && byte <= node.end_byte() {
            result = Some(node);

            // Try to go deeper
            if cursor.goto_first_child() {
                // Find child that contains the position
                loop {
                    let child = cursor.node();
                    if byte >= child.start_byte() && byte <= child.end_byte() {
                        break;
                    }
                    if !cursor.goto_next_sibling() {
                        // No child contains the position, stay at parent
                        cursor.goto_parent();
                        return result;
                    }
                }
            } else {
                return result;
            }
        } else {
            return result;
        }
    }
}

/// Get the content of the current line before the cursor
fn get_line_before_cursor(source: &str, cursor_byte: usize) -> &str {
    // Find the start of the current line
    let line_start = source[..cursor_byte]
        .rfind('\n')
        .map(|pos| pos + 1)
        .unwrap_or(0);

    &source[line_start..cursor_byte]
}

/// Get the indentation (in spaces) at the start of the line containing the given byte
fn get_line_indent_at_byte(source: &str, byte: usize) -> usize {
    // Find the start of the line
    let line_start = source[..byte]
        .rfind('\n')
        .map(|pos| pos + 1)
        .unwrap_or(0);

    // Count leading whitespace
    let line = &source[line_start..];
    let mut indent = 0;
    for ch in line.chars() {
        match ch {
            ' ' => indent += 1,
            '\t' => indent += 4, // Assume tab width of 4
            _ => break,
        }
    }

    indent
}

/// Check if the line ending at cursor is a JSX opening tag
fn is_jsx_opening_tag_line(source: &str, cursor_byte: usize) -> bool {
    let line = get_line_before_cursor(source, cursor_byte);
    let trimmed = line.trim();

    // Simple heuristic: line starts with < and doesn't end with />
    trimmed.starts_with('<')
        && !trimmed.starts_with("</")
        && !trimmed.ends_with("/>")
        && trimmed.ends_with('>')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_js(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_javascript::LANGUAGE.into()).unwrap();
        parser.parse(source, None).unwrap()
    }

    fn parse_ts(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_function_block_indent() {
        let source = "function foo() {";
        let tree = parse_js(source);
        let indent = calculate_indent(&tree, source, source.len(), 4);
        assert_eq!(indent, 4);
    }

    #[test]
    fn test_object_literal_indent() {
        let source = "const obj = {";
        let tree = parse_js(source);
        let indent = calculate_indent(&tree, source, source.len(), 4);
        assert_eq!(indent, 4);
    }

    #[test]
    fn test_array_literal_indent() {
        let source = "const arr = [";
        let tree = parse_js(source);
        let indent = calculate_indent(&tree, source, source.len(), 4);
        assert_eq!(indent, 4);
    }

    #[test]
    fn test_nested_indent() {
        let source = "const obj = {\n    items: [";
        let tree = parse_js(source);
        let indent = calculate_indent(&tree, source, source.len(), 4);
        assert_eq!(indent, 8); // Two levels
    }

    #[test]
    fn test_no_indent_for_statement() {
        let source = "const x = 5;";
        let tree = parse_js(source);
        let indent = calculate_indent(&tree, source, source.len(), 4);
        assert_eq!(indent, 0);
    }

    #[test]
    fn test_closing_bracket_dedent() {
        let source = "function foo() {\n    return bar;\n    ";
        let tree = parse_js(source);
        assert!(should_dedent_closing_bracket(&tree, source, source.len(), '}', 4));
    }

    #[test]
    fn test_get_line_before_cursor() {
        let source = "line1\n  line2";
        assert_eq!(get_line_before_cursor(source, 13), "  line2");
        assert_eq!(get_line_before_cursor(source, 5), "line1");
    }

    #[test]
    fn test_typescript_interface() {
        let source = "interface Foo {";
        let tree = parse_ts(source);
        let indent = calculate_indent(&tree, source, source.len(), 4);
        assert_eq!(indent, 4);
    }
}
