//! JavaScript, TypeScript, and TSX symbol, import, and call extraction.

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
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "function".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(format!("function {}", node_text(name_node, src))),
                    });
                    extract_calls(&child, src, result, idx);
                    extract_node(child, src, result, Some(idx));
                }
            }
            "class_declaration" => {
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
            "method_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "method".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                    extract_calls(&child, src, result, idx);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                extract_top_level_dynamic_imports(&child, src, result);
                extract_arrow_functions(&child, src, result, parent_index);
            }
            "import_statement" => {
                extract_import(&child, src, result);
            }
            "export_statement" => {
                // Re-exports: export { X } from './module' or export * from './module'
                if let Some(source) = child.child_by_field_name("source") {
                    let module = node_text(source, src)
                        .trim_matches(|c| c == '\'' || c == '"')
                        .to_string();
                    let names = collect_export_names(&child, src);
                    result.imports.push(Import {
                        module,
                        names: if names.is_empty() {
                            None
                        } else {
                            Some(names.join(", "))
                        },
                    });
                }
                // Recurse into export to find declarations
                extract_node(child, src, result, parent_index);
            }
            "expression_statement" => {
                extract_top_level_dynamic_imports(&child, src, result);
                extract_node(child, src, result, parent_index);
            }
            _ => {
                extract_node(child, src, result, parent_index);
            }
        }
    }
}

/// Collect named export specifiers, descending into `export_clause`.
/// `export { Router, Config } from './module'` → ["Router", "Config"]
fn collect_export_names(node: &tree_sitter::Node, src: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "export_clause" {
            let mut c2 = child.walk();
            for spec in child.children(&mut c2) {
                if spec.kind() == "export_specifier" {
                    // Prefer alias: `export { default as Config }` → "Config"
                    let name_node = spec
                        .child_by_field_name("alias")
                        .or_else(|| spec.child_by_field_name("name"));
                    if let Some(n) = name_node {
                        names.push(node_text(n, src).to_string());
                    }
                }
            }
        }
    }
    names
}

/// Extract dynamic `import()` calls from top-level statements.
fn extract_top_level_dynamic_imports(
    node: &tree_sitter::Node,
    src: &[u8],
    result: &mut ParseResult,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression"
            && let Some(func) = child.child_by_field_name("function")
            && func.kind() == "import"
            && let Some(args) = child.child_by_field_name("arguments")
            && let Some(arg) = args.named_child(0)
            && arg.kind() == "string"
        {
            let module = node_text(arg, src)
                .trim_matches(|c| c == '\'' || c == '"')
                .to_string();
            result.imports.push(Import {
                module,
                names: None,
            });
        } else {
            extract_top_level_dynamic_imports(&child, src, result);
        }
    }
}

fn extract_arrow_functions(
    node: &tree_sitter::Node,
    src: &[u8],
    result: &mut ParseResult,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name_node = child.child_by_field_name("name");
            let value_node = child.child_by_field_name("value");
            if let (Some(name), Some(value)) = (name_node, value_node)
                && (value.kind() == "arrow_function" || value.kind() == "function")
            {
                let idx = result.symbols.len();
                result.symbols.push(Symbol {
                    name: node_text(name, src).to_string(),
                    kind: "function".to_string(),
                    line_start: node.start_position().row + 1,
                    line_end: node.end_position().row + 1,
                    parent_index,
                    signature: Some(format!("const {}", node_text(name, src))),
                });
                extract_calls(&value, src, result, idx);
            }
        }
    }
}

fn extract_import(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
    if let Some(source) = node.child_by_field_name("source") {
        let module = node_text(source, src)
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string();
        let mut names = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "import_specifier"
                && let Some(n) = child.child_by_field_name("name")
            {
                names.push(node_text(n, src).to_string());
            } else if child.kind() == "import_clause" {
                let mut c2 = child.walk();
                for inner in child.children(&mut c2) {
                    if inner.kind() == "identifier" {
                        names.push(node_text(inner, src).to_string());
                    } else if inner.kind() == "named_imports" {
                        let mut c3 = inner.walk();
                        for spec in inner.children(&mut c3) {
                            if spec.kind() == "import_specifier"
                                && let Some(n) = spec.child_by_field_name("name")
                            {
                                names.push(node_text(n, src).to_string());
                            }
                        }
                    }
                }
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

fn extract_calls(
    node: &tree_sitter::Node,
    src: &[u8],
    result: &mut ParseResult,
    caller_index: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                if let Some(func) = child.child_by_field_name("function") {
                    match func.kind() {
                        // Dynamic import: import('./path') → add as import entry
                        "import" => {
                            if let Some(args) = child.child_by_field_name("arguments")
                                && let Some(arg) = args.named_child(0)
                                && arg.kind() == "string"
                            {
                                let module = node_text(arg, src)
                                    .trim_matches(|c| c == '\'' || c == '"')
                                    .to_string();
                                result.imports.push(Import {
                                    module,
                                    names: None,
                                });
                            }
                        }
                        "identifier" => {
                            result.calls.push(Call {
                                caller_index,
                                callee_name: node_text(func, src).to_string(),
                                line: child.start_position().row + 1,
                            });
                        }
                        "member_expression" => {
                            if let Some(prop) = func.child_by_field_name("property") {
                                result.calls.push(Call {
                                    caller_index,
                                    callee_name: node_text(prop, src).to_string(),
                                    line: child.start_position().row + 1,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            // JSX: <Component /> and <Component>...</Component> as call edges.
            // Only uppercase names (user components), not lowercase (HTML elements).
            "jsx_self_closing_element" | "jsx_element" => {
                let tag_node = if child.kind() == "jsx_self_closing_element" {
                    child.child_by_field_name("name")
                } else {
                    // jsx_element has an open_tag child with the name
                    child
                        .children(&mut child.walk())
                        .find(|c| c.kind() == "jsx_opening_element")
                        .and_then(|open| open.child_by_field_name("name"))
                };
                if let Some(tag) = tag_node {
                    let name = match tag.kind() {
                        "identifier" => {
                            let n = node_text(tag, src);
                            // Skip lowercase HTML elements (div, span, etc.)
                            if n.starts_with(|c: char| c.is_uppercase()) {
                                n.to_string()
                            } else {
                                extract_calls(&child, src, result, caller_index);
                                continue;
                            }
                        }
                        // <Foo.Bar /> → call to "Bar"
                        "member_expression" => {
                            if let Some(prop) = tag.child_by_field_name("property") {
                                node_text(prop, src).to_string()
                            } else {
                                extract_calls(&child, src, result, caller_index);
                                continue;
                            }
                        }
                        _ => {
                            extract_calls(&child, src, result, caller_index);
                            continue;
                        }
                    };
                    result.calls.push(Call {
                        caller_index,
                        callee_name: name,
                        line: child.start_position().row + 1,
                    });
                }
            }
            _ => {}
        }
        extract_calls(&child, src, result, caller_index);
    }
}
