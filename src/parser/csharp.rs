//! C# symbol, import, and call extraction.

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
    // Track file-scoped namespace: once seen at this level, all subsequent siblings use it as parent.
    let mut active_ns: Option<usize> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let effective_parent = active_ns.or(parent_index);
        match child.kind() {
            "file_scoped_namespace_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "namespace".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                    active_ns = Some(idx);
                }
            }
            "namespace_declaration" => {
                let ns_parent = if let Some(name_node) = child.child_by_field_name("name") {
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "namespace".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index: effective_parent,
                        signature: None,
                    });
                    Some(idx)
                } else {
                    effective_parent
                };
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, result, ns_parent);
                }
            }
            kind @ ("class_declaration"
            | "struct_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration") => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let sym_kind = kind.strip_suffix("_declaration").unwrap_or(kind);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: sym_kind.to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index: effective_parent,
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
                        parent_index: effective_parent,
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
                        parent_index: effective_parent,
                        signature: Some(sig),
                    });
                    extract_calls(&child, src, result, idx);
                }
            }
            "property_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let prop_type = child
                        .child_by_field_name("type")
                        .map(|n| node_text(n, src))
                        .unwrap_or("?");
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "property".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index: effective_parent,
                        signature: Some(format!("{prop_type} {}", node_text(name_node, src))),
                    });
                }
            }
            "using_directive" => {
                extract_using(&child, src, result);
            }
            _ => {
                extract_node(child, src, result, effective_parent);
            }
        }
    }
}

fn extract_using(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
    // For alias directives (`using Alias = Target;`), collect all name-like children.
    // The last qualified_name/identifier is the actual target; earlier ones are the alias.
    let mut cursor = node.walk();
    let mut last_name: Option<String> = None;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "qualified_name" | "identifier" | "alias_qualified_name" => {
                last_name = Some(node_text(child, src).to_string());
            }
            _ => {}
        }
    }
    if let Some(module) = last_name {
        result.imports.push(Import {
            module,
            names: None,
        });
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
            "invocation_expression" => {
                child
                    .child_by_field_name("function")
                    .and_then(|fn_node| match fn_node.kind() {
                        "identifier" => Some(node_text(fn_node, src).to_string()),
                        "member_access_expression" => fn_node
                            .child_by_field_name("name")
                            .map(|n| normalize_type_name(node_text(n, src))),
                        "generic_name" => fn_node
                            .child_by_field_name("name")
                            .map(|n| node_text(n, src).to_string()),
                        _ => None,
                    })
            }
            "object_creation_expression" => {
                child
                    .child_by_field_name("type")
                    .map(|type_node| match type_node.kind() {
                        "generic_name" => type_node
                            .child_by_field_name("name")
                            .map(|n| node_text(n, src).to_string())
                            .unwrap_or_else(|| normalize_type_name(node_text(type_node, src))),
                        _ => normalize_type_name(node_text(type_node, src)),
                    })
            }
            _ => None,
        };
        if let Some(name) = callee.filter(|n| !n.is_empty()) {
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
