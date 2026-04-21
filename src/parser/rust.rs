//! Rust symbol, import, and call extraction.

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
            "function_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let kind = if parent_index.is_some() {
                        "method"
                    } else {
                        "function"
                    };
                    let sig = build_fn_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: kind.to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    extract_calls(&child, src, result, idx);
                }
            }
            "struct_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "struct".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                }
            }
            "enum_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "enum".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                }
            }
            "trait_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "trait".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                    extract_node(child, src, result, Some(idx));
                }
            }
            "impl_item" => {
                // Find the type being implemented
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_name = node_text(type_node, src);
                    // Find the parent symbol index for this impl
                    let impl_parent = result.symbols.iter().position(|s| {
                        s.name == type_name && (s.kind == "struct" || s.kind == "enum")
                    });
                    extract_node(child, src, result, impl_parent);
                } else {
                    extract_node(child, src, result, parent_index);
                }
            }
            "use_declaration" => {
                let text = node_text(child, src);
                if let Some(rest) = text.strip_prefix("use ") {
                    let module = rest.trim_end_matches(';').trim().to_string();
                    result.imports.push(Import {
                        module,
                        names: None,
                    });
                }
            }
            _ => {
                extract_node(child, src, result, parent_index);
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
                "field_expression" => {
                    if let Some(field) = func.child_by_field_name("field") {
                        node_text(field, src).to_string()
                    } else {
                        continue;
                    }
                }
                "scoped_identifier" => {
                    if let Some(name) = func.child_by_field_name("name") {
                        node_text(name, src).to_string()
                    } else {
                        node_text(func, src).to_string()
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
        // Also handle macro invocations
        if child.kind() == "macro_invocation"
            && let Some(macro_name) = child.child_by_field_name("macro")
        {
            result.calls.push(Call {
                caller_index,
                callee_name: node_text(macro_name, src).to_string(),
                line: child.start_position().row + 1,
            });
        }
        extract_calls(&child, src, result, caller_index);
    }
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
        .child_by_field_name("return_type")
        .map(|n| format!(" -> {}", node_text(n, src)))
        .unwrap_or_default();
    format!("fn {name}{params}{ret}")
}
