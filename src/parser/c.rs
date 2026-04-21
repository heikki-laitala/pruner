//! C symbol, import, and call extraction.
//!
//! Several helpers here (`extract_calls`, `build_fn_signature`, `extract_type`,
//! `extract_typedef`) are reused by the C++ adapter in [`super::cpp`].

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
                if let Some(declarator) = child.child_by_field_name("declarator")
                    && let Some(name) = extract_declarator_name(&declarator, src)
                {
                    let sig = build_fn_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name,
                        kind: "function".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    extract_calls(&child, src, result, idx);
                }
            }
            "declaration" => {
                // Function declarations (prototypes) — skip, only extract definitions
                // But extract global variable declarations if needed
            }
            "struct_specifier" | "enum_specifier" | "union_specifier" => {
                extract_type(&child, src, result);
            }
            "type_definition" => {
                extract_typedef(&child, src, result);
            }
            "preproc_include" => {
                if let Some(path_node) = child.child_by_field_name("path") {
                    let raw = node_text(path_node, src);
                    let module = raw.trim_matches(&['"', '<', '>'] as &[char]).to_string();
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

/// Extract the name from a C declarator (handles function_declarator, pointer_declarator, etc.)
fn extract_declarator_name(node: &tree_sitter::Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" => Some(node_text(*node, src).to_string()),
        "function_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|d| extract_declarator_name(&d, src)),
        "pointer_declarator" | "parenthesized_declarator" | "array_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|d| extract_declarator_name(&d, src)),
        _ => {
            // Try first named child as fallback
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if let Some(name) = extract_declarator_name(&child, src) {
                    return Some(name);
                }
            }
            None
        }
    }
}

pub(super) fn extract_type(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
    let kind = match node.kind() {
        "struct_specifier" => "struct",
        "enum_specifier" => "enum",
        "union_specifier" => "union",
        _ => "type",
    };
    if let Some(name_node) = node.child_by_field_name("name") {
        // Only extract if it has a body (definition, not just forward declaration)
        if node.child_by_field_name("body").is_some() {
            result.symbols.push(Symbol {
                name: node_text(name_node, src).to_string(),
                kind: kind.to_string(),
                line_start: node.start_position().row + 1,
                line_end: node.end_position().row + 1,
                parent_index: None,
                signature: None,
            });
        }
    }
}

pub(super) fn extract_typedef(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
    // typedef struct { ... } Name; — the name is a type_identifier child
    // typedef int MyInt; — the name is also a type_identifier child
    // typedef void (*FuncPtr)(int); — the name is in a function_declarator
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "type_identifier" => {
                result.symbols.push(Symbol {
                    name: node_text(child, src).to_string(),
                    kind: "type".to_string(),
                    line_start: node.start_position().row + 1,
                    line_end: node.end_position().row + 1,
                    parent_index: None,
                    signature: None,
                });
                return;
            }
            "function_declarator" | "pointer_declarator" => {
                if let Some(name) = extract_declarator_name(&child, src) {
                    result.symbols.push(Symbol {
                        name,
                        kind: "type".to_string(),
                        line_start: node.start_position().row + 1,
                        line_end: node.end_position().row + 1,
                        parent_index: None,
                        signature: None,
                    });
                    return;
                }
            }
            _ => {}
        }
    }
}

pub(super) fn extract_calls(
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
                    // obj->method or obj.method
                    func.child_by_field_name("field")
                        .map(|f| node_text(f, src).to_string())
                        .unwrap_or_default()
                }
                "qualified_identifier" | "scoped_identifier" => {
                    // std::move, ns::foo, Type::static_fn — use rightmost segment
                    let full = node_text(func, src);
                    full.rsplit("::").next().unwrap_or(full).to_string()
                }
                _ => String::new(),
            };
            if !name.is_empty() {
                result.calls.push(Call {
                    caller_index,
                    callee_name: name,
                    line: child.start_position().row + 1,
                });
            }
        }
        extract_calls(&child, src, result, caller_index);
    }
}

pub(super) fn build_fn_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
    let return_type = node
        .child_by_field_name("type")
        .map(|n| node_text(n, src))
        .unwrap_or("void");
    let declarator = node.child_by_field_name("declarator");
    if let Some(decl) = declarator {
        let decl_text = node_text(decl, src);
        // Remove the body, just keep the declaration part
        format!("{return_type} {decl_text}")
    } else {
        return_type.to_string()
    }
}
