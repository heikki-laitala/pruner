//! Java symbol, import, and call extraction.

use super::common::{node_text, normalize_type_name};
use super::{Call, Import, ParseResult, Symbol};
use anyhow::Result;

pub(super) fn extract(root: tree_sitter::Node, src: &[u8]) -> Result<ParseResult> {
    let mut result = ParseResult::default();
    extract_node(root, src, &mut result, None);
    Ok(result)
}

fn extract_node(
    node: tree_sitter::Node,
    src: &[u8],
    result: &mut ParseResult,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            kind @ ("class_declaration" | "interface_declaration" | "enum_declaration") => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let sym_kind = kind.strip_suffix("_declaration").unwrap_or(kind);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: sym_kind.to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                    extract_node(child, src, result, Some(idx));
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let sig = build_method_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "method".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    extract_calls(&child, src, result, idx);
                }
            }
            "constructor_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let sig = build_constructor_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "constructor".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    extract_calls(&child, src, result, idx);
                }
            }
            "import_declaration" => {
                extract_import(&child, src, result);
            }
            _ => {
                extract_node(child, src, result, parent_index);
            }
        }
    }
}

fn extract_import(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "scoped_identifier" | "scoped_absolute_identifier"
        ) {
            result.imports.push(Import {
                module: node_text(child, src).to_string(),
                names: None,
            });
            return;
        }
    }
}

fn extract_calls(
    node: &tree_sitter::Node,
    src: &[u8],
    result: &mut ParseResult,
    caller_index: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let callee = match child.kind() {
            "method_invocation" => child
                .child_by_field_name("name")
                .map(|n| node_text(n, src).to_string()),
            "object_creation_expression" => child
                .child_by_field_name("type")
                .map(|n| normalize_type_name(node_text(n, src))),
            _ => None,
        };
        if let Some(name) = callee {
            result.calls.push(Call {
                caller_index,
                callee_name: name,
                line: child.start_position().row + 1,
            });
        }
        extract_calls(&child, src, result, caller_index);
    }
}

fn build_method_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
    let ret_type = node
        .child_by_field_name("type")
        .map(|n| node_text(n, src))
        .unwrap_or("void");
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .unwrap_or("?");
    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(n, src))
        .unwrap_or("()");
    format!("{ret_type} {name}{params}")
}

fn build_constructor_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .unwrap_or("?");
    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(n, src))
        .unwrap_or("()");
    format!("{name}{params}")
}
