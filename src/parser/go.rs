//! Go symbol, import, and call extraction.

use super::common::node_text;
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
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let sig = build_fn_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "function".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    extract_calls(&child, src, result, idx);
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
                        parent_index: find_receiver_parent(&child, src, result),
                        signature: Some(sig),
                    });
                    extract_calls(&child, src, result, idx);
                }
            }
            "type_declaration" => {
                extract_type_decl(&child, src, result);
            }
            "import_declaration" => {
                extract_imports(&child, src, result);
            }
            _ => {
                extract_node(child, src, result, parent_index);
            }
        }
    }
}

fn extract_type_decl(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_spec"
            && let Some(name_node) = child.child_by_field_name("name")
        {
            let type_node = child.child_by_field_name("type");
            let kind = match type_node.map(|t| t.kind()) {
                Some("struct_type") => "struct",
                Some("interface_type") => "interface",
                _ => "type",
            };
            result.symbols.push(Symbol {
                name: node_text(name_node, src).to_string(),
                kind: kind.to_string(),
                line_start: child.start_position().row + 1,
                line_end: child.end_position().row + 1,
                parent_index: None,
                signature: None,
            });
        }
    }
}

fn extract_imports(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
    // Recursively find all import_spec or interpreted_string_literal nodes
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => {
                if let Some(path_node) = child.child_by_field_name("path") {
                    let module = node_text(path_node, src)
                        .trim_matches(&['"', '`'] as &[char])
                        .to_string();
                    result.imports.push(Import {
                        module,
                        names: None,
                    });
                }
            }
            "interpreted_string_literal" => {
                // Single import: import "fmt"
                let module = node_text(child, src)
                    .trim_matches(&['"', '`'] as &[char])
                    .to_string();
                result.imports.push(Import {
                    module,
                    names: None,
                });
            }
            _ => {
                // Recurse into import_spec_list etc.
                extract_imports(&child, src, result);
            }
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
        if child.kind() == "call_expression"
            && let Some(func) = child.child_by_field_name("function")
        {
            let name = match func.kind() {
                "identifier" => node_text(func, src).to_string(),
                "selector_expression" => {
                    if let Some(field) = func.child_by_field_name("field") {
                        node_text(field, src).to_string()
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };
            result.calls.push(Call {
                caller_index,
                callee_name: name,
                line: child.start_position().row + 1,
            });
        }
        extract_calls(&child, src, result, caller_index);
    }
}

/// Extract the base type name from a Go type node, handling pointers and generics.
/// e.g. `Server` -> "Server", `*Server` -> "Server", `Box[T]` -> "Box", `*Box[T]` -> "Box"
fn extract_base_type<'a>(type_node: tree_sitter::Node<'a>, src: &'a [u8]) -> Option<&'a str> {
    match type_node.kind() {
        "type_identifier" => Some(node_text(type_node, src)),
        "pointer_type" | "generic_type" => {
            // Recurse into child to find the type_identifier
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                if let Some(name) = extract_base_type(child, src) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_receiver_parent(
    method: &tree_sitter::Node,
    src: &[u8],
    result: &ParseResult,
) -> Option<usize> {
    let receiver = method.child_by_field_name("receiver")?;
    // Receiver is a parameter_list like (s *Server) or (s Server)
    let mut cursor = receiver.walk();
    for child in receiver.children(&mut cursor) {
        if child.kind() == "parameter_declaration"
            && let Some(type_node) = child.child_by_field_name("type")
        {
            let type_name = extract_base_type(type_node, src);
            if let Some(name) = type_name {
                return result
                    .symbols
                    .iter()
                    .position(|s| s.name == name && (s.kind == "struct" || s.kind == "interface"));
            }
        }
    }
    None
}

fn build_fn_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .unwrap_or("?");
    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(n, src))
        .unwrap_or("()");
    let ret = node
        .child_by_field_name("result")
        .map(|n| format!(" {}", node_text(n, src)))
        .unwrap_or_default();
    format!("func {name}{params}{ret}")
}

fn build_method_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
    let receiver = node
        .child_by_field_name("receiver")
        .map(|n| format!("{} ", node_text(n, src)))
        .unwrap_or_default();
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .unwrap_or("?");
    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(n, src))
        .unwrap_or("()");
    let ret = node
        .child_by_field_name("result")
        .map(|n| format!(" {}", node_text(n, src)))
        .unwrap_or_default();
    format!("func {receiver}{name}{params}{ret}")
}
