//! C++ symbol, import, and call extraction. Reuses several helpers from [`super::c`]
//! since the languages share declarator/call grammar.

use super::c;
use super::common::node_text;
use super::{Import, ParseResult, Symbol};
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
                    && let Some(name) = extract_fn_name(&declarator, src)
                {
                    let display_name = name.rsplit("::").next().unwrap_or(&name).to_string();
                    let sig = c::build_fn_signature(&child, src);
                    let idx = result.symbols.len();
                    // For qualified names (Foo::bar, ns::Foo::bar), resolve parent
                    // from the second-to-last segment. Classify as method if the
                    // immediate scope matches a known class/struct, or if it's not
                    // a known namespace (out-of-line method with class in a header).
                    let (kind, parent) = if name.contains("::") {
                        let segments: Vec<&str> = name.rsplitn(2, "::").collect();
                        let scope = segments.get(1).unwrap_or(&"");
                        let class_name = scope.rsplit("::").next().unwrap_or(scope);
                        if let Some(pos) = result.symbols.iter().position(|s| {
                            s.name == class_name && (s.kind == "class" || s.kind == "struct")
                        }) {
                            ("method", Some(pos))
                        } else if result
                            .symbols
                            .iter()
                            .any(|s| s.name == class_name && s.kind == "namespace")
                        {
                            // Scope is a known namespace — this is a free function
                            ("function", parent_index)
                        } else {
                            // Scope is unknown (likely a class declared in a header)
                            ("method", parent_index)
                        }
                    } else {
                        ("function", parent_index)
                    };
                    result.symbols.push(Symbol {
                        name: display_name,
                        kind: kind.to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index: parent,
                        signature: Some(sig),
                    });
                    c::extract_calls(&child, src, result, idx);
                }
            }
            "class_specifier" => {
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
                    // Extract inline method definitions inside the class body
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_class_body(&body, src, result, Some(idx));
                    }
                }
            }
            "struct_specifier" => {
                if let Some(name_node) = child.child_by_field_name("name")
                    && child.child_by_field_name("body").is_some()
                {
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "struct".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_class_body(&body, src, result, Some(idx));
                    }
                }
            }
            "enum_specifier" => {
                c::extract_type(&child, src, result);
            }
            "namespace_definition" => {
                let ns_parent = if let Some(name_node) = child.child_by_field_name("name") {
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "namespace".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                    Some(idx)
                } else {
                    // Anonymous namespace — recurse with current parent
                    parent_index
                };
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, result, ns_parent);
                }
            }
            "type_definition" => {
                c::extract_typedef(&child, src, result);
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
            "template_declaration" => {
                // Recurse into template to find the class/function inside
                extract_node(child, src, result, parent_index);
            }
            _ => {
                extract_node(child, src, result, parent_index);
            }
        }
    }
}

/// Extract function/method name from a C++ declarator, handling qualified names like Class::method.
fn extract_fn_name(node: &tree_sitter::Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" | "destructor_name" => {
            Some(node_text(*node, src).to_string())
        }
        "qualified_identifier" => Some(node_text(*node, src).to_string()),
        "function_declarator" | "reference_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|d| extract_fn_name(&d, src)),
        "pointer_declarator" | "parenthesized_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|d| extract_fn_name(&d, src)),
        "operator_name" => Some(node_text(*node, src).to_string()),
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if let Some(name) = extract_fn_name(&child, src) {
                    return Some(name);
                }
            }
            None
        }
    }
}

/// Extract inline method definitions from a class/struct body.
fn extract_class_body(
    body: &tree_sitter::Node,
    src: &[u8],
    result: &mut ParseResult,
    parent_index: Option<usize>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(declarator) = child.child_by_field_name("declarator")
                    && let Some(name) = extract_fn_name(&declarator, src)
                {
                    let sig = c::build_fn_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name,
                        kind: "method".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    c::extract_calls(&child, src, result, idx);
                }
            }
            "access_specifier" | "field_declaration" => {}
            _ => {}
        }
    }
}
