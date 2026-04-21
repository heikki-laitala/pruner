//! Python symbol, import, and call extraction.

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
            "function_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let kind = if parent_index.is_some() {
                        "method"
                    } else {
                        "function"
                    };
                    let sig = build_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: kind.to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    // Extract calls inside this function
                    extract_calls(&child, src, result, idx);
                    // Recurse for nested definitions
                    extract_node(child, src, result, Some(idx));
                }
            }
            "class_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "class".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                    extract_node(child, src, result, Some(idx));
                }
            }
            "import_statement" => {
                let text = node_text(child, src);
                if let Some(module) = text.strip_prefix("import ") {
                    result.imports.push(Import {
                        module: module.trim().to_string(),
                        names: None,
                    });
                }
            }
            "import_from_statement" => {
                if let Some(module_node) = child.child_by_field_name("module_name") {
                    let module = node_text(module_node, src).to_string();
                    let mut names = Vec::new();
                    let mut c2 = child.walk();
                    for n in child.children(&mut c2) {
                        if n.kind() == "dotted_name" && n.id() != module_node.id() {
                            names.push(node_text(n, src).to_string());
                        } else if n.kind() == "aliased_import"
                            && let Some(name) = n.child_by_field_name("name")
                        {
                            names.push(node_text(name, src).to_string());
                        }
                    }
                    result.imports.push(Import {
                        module,
                        names: if names.is_empty() {
                            None
                        } else {
                            Some(names.join(", "))
                        },
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
        if child.kind() == "call"
            && let Some(func) = child.child_by_field_name("function")
        {
            let name = match func.kind() {
                "identifier" => node_text(func, src).to_string(),
                "attribute" => {
                    if let Some(attr) = func.child_by_field_name("attribute") {
                        node_text(attr, src).to_string()
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

fn build_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .unwrap_or("?");
    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(n, src))
        .unwrap_or("()");
    format!("def {name}{params}")
}
