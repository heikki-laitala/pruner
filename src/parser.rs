//! Tree-sitter based symbol, import, and call extraction.
//!

use crate::languages::Language;
use anyhow::Result;

/// Extracted symbols, imports, and calls from a single file.
#[derive(Debug, Default)]
pub struct ParseResult {
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
    pub calls: Vec<Call>,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub line_start: usize,
    pub line_end: usize,
    pub parent_index: Option<usize>,
    pub signature: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub module: String,
    pub names: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Call {
    /// Index into ParseResult.symbols for the caller.
    pub caller_index: usize,
    pub callee_name: String,
    pub line: usize,
}

/// Parse source code and extract symbols, imports, and calls.
pub fn parse_source(source: &str, language: Language) -> Result<ParseResult> {
    let ts_language = ts_language_for(language);
    let mut ts_parser = tree_sitter::Parser::new();
    ts_parser.set_language(&ts_language)?;

    let tree = ts_parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter parse failed"))?;

    let root = tree.root_node();
    let src = source.as_bytes();

    match language {
        Language::Python => extract_python(root, src),
        Language::JavaScript => extract_js_ts(root, src),
        Language::TypeScript | Language::Tsx => extract_js_ts(root, src),
        Language::Rust => extract_rust(root, src),
        Language::Go => extract_go(root, src),
        Language::Java => extract_java(root, src),
    }
}

fn ts_language_for(language: Language) -> tree_sitter::Language {
    match language {
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
    }
}

fn node_text<'a>(node: tree_sitter::Node<'a>, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

// -- Python extraction --

fn extract_python(root: tree_sitter::Node, src: &[u8]) -> Result<ParseResult> {
    let mut result = ParseResult::default();
    extract_python_node(root, src, &mut result, None);
    Ok(result)
}

fn extract_python_node(
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
                    let sig = build_python_signature(&child, src);
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
                    extract_python_calls(&child, src, result, idx);
                    // Recurse for nested definitions
                    extract_python_node(child, src, result, Some(idx));
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
                    extract_python_node(child, src, result, Some(idx));
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
                extract_python_node(child, src, result, parent_index);
            }
        }
    }
}

fn extract_python_calls(
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
        extract_python_calls(&child, src, result, caller_index);
    }
}

fn build_python_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
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

// -- JavaScript/TypeScript extraction --

fn extract_js_ts(root: tree_sitter::Node, src: &[u8]) -> Result<ParseResult> {
    let mut result = ParseResult::default();
    extract_js_ts_node(root, src, &mut result, None);
    Ok(result)
}

fn extract_js_ts_node(
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
                    extract_js_ts_calls(&child, src, result, idx);
                    extract_js_ts_node(child, src, result, Some(idx));
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
                    extract_js_ts_node(child, src, result, Some(idx));
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
                    extract_js_ts_calls(&child, src, result, idx);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                // Arrow functions: const foo = () => ...
                extract_js_ts_arrow_functions(&child, src, result, parent_index);
            }
            "import_statement" => {
                extract_js_ts_import(&child, src, result);
            }
            "export_statement" => {
                // Recurse into export to find declarations
                extract_js_ts_node(child, src, result, parent_index);
            }
            _ => {
                extract_js_ts_node(child, src, result, parent_index);
            }
        }
    }
}

fn extract_js_ts_arrow_functions(
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
                extract_js_ts_calls(&value, src, result, idx);
            }
        }
    }
}

fn extract_js_ts_import(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
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

fn extract_js_ts_calls(
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
                "member_expression" => {
                    if let Some(prop) = func.child_by_field_name("property") {
                        node_text(prop, src).to_string()
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
        extract_js_ts_calls(&child, src, result, caller_index);
    }
}

// -- Rust extraction --

fn extract_rust(root: tree_sitter::Node, src: &[u8]) -> Result<ParseResult> {
    let mut result = ParseResult::default();
    extract_rust_node(root, src, &mut result, None);
    Ok(result)
}

fn extract_rust_node(
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
                    let sig = build_rust_fn_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: kind.to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    extract_rust_calls(&child, src, result, idx);
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
                    extract_rust_node(child, src, result, Some(idx));
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
                    extract_rust_node(child, src, result, impl_parent);
                } else {
                    extract_rust_node(child, src, result, parent_index);
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
                extract_rust_node(child, src, result, parent_index);
            }
        }
    }
}

fn extract_rust_calls(
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
        extract_rust_calls(&child, src, result, caller_index);
    }
}

fn build_rust_fn_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
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

// -- Go extraction --

fn extract_go(root: tree_sitter::Node, src: &[u8]) -> Result<ParseResult> {
    let mut result = ParseResult::default();
    extract_go_node(root, src, &mut result, None);
    Ok(result)
}

fn extract_go_node(
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
                    let sig = build_go_fn_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "function".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    extract_go_calls(&child, src, result, idx);
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let sig = build_go_method_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "method".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index: find_go_receiver_parent(&child, src, result),
                        signature: Some(sig),
                    });
                    extract_go_calls(&child, src, result, idx);
                }
            }
            "type_declaration" => {
                extract_go_type_decl(&child, src, result);
            }
            "import_declaration" => {
                extract_go_imports(&child, src, result);
            }
            _ => {
                extract_go_node(child, src, result, parent_index);
            }
        }
    }
}

fn extract_go_type_decl(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
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

fn extract_go_imports(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
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
                extract_go_imports(&child, src, result);
            }
        }
    }
}

fn extract_go_calls(
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
        extract_go_calls(&child, src, result, caller_index);
    }
}

/// Extract the base type name from a Go type node, handling pointers and generics.
/// e.g. `Server` -> "Server", `*Server` -> "Server", `Box[T]` -> "Box", `*Box[T]` -> "Box"
fn extract_go_base_type<'a>(type_node: tree_sitter::Node<'a>, src: &'a [u8]) -> Option<&'a str> {
    match type_node.kind() {
        "type_identifier" => Some(node_text(type_node, src)),
        "pointer_type" | "generic_type" => {
            // Recurse into child to find the type_identifier
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                if let Some(name) = extract_go_base_type(child, src) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_go_receiver_parent(
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
            let type_name = extract_go_base_type(type_node, src);
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

fn build_go_fn_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
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

fn build_go_method_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
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

// -- Java extraction --

fn extract_java(root: tree_sitter::Node, src: &[u8]) -> Result<ParseResult> {
    let mut result = ParseResult::default();
    extract_java_node(root, src, &mut result, None);
    Ok(result)
}

fn extract_java_node(
    node: tree_sitter::Node,
    src: &[u8],
    result: &mut ParseResult,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
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
                    extract_java_node(child, src, result, Some(idx));
                }
            }
            "interface_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "interface".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                    extract_java_node(child, src, result, Some(idx));
                }
            }
            "enum_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "enum".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: None,
                    });
                    extract_java_node(child, src, result, Some(idx));
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let sig = build_java_method_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "method".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    extract_java_calls(&child, src, result, idx);
                }
            }
            "constructor_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let sig = build_java_constructor_signature(&child, src);
                    let idx = result.symbols.len();
                    result.symbols.push(Symbol {
                        name: node_text(name_node, src).to_string(),
                        kind: "constructor".to_string(),
                        line_start: child.start_position().row + 1,
                        line_end: child.end_position().row + 1,
                        parent_index,
                        signature: Some(sig),
                    });
                    extract_java_calls(&child, src, result, idx);
                }
            }
            "import_declaration" => {
                extract_java_import(&child, src, result);
            }
            _ => {
                extract_java_node(child, src, result, parent_index);
            }
        }
    }
}

fn extract_java_import(node: &tree_sitter::Node, src: &[u8], result: &mut ParseResult) {
    // import_declaration contains scoped_identifier (or asterisk_import for wildcard)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier" => {
                let module = node_text(child, src).to_string();
                result.imports.push(Import {
                    module,
                    names: None,
                });
                return;
            }
            "scoped_absolute_identifier" => {
                // static imports: import static org.junit.Assert.assertEquals
                let module = node_text(child, src).to_string();
                result.imports.push(Import {
                    module,
                    names: None,
                });
                return;
            }
            _ => {}
        }
    }
}

fn extract_java_calls(
    node: &tree_sitter::Node,
    src: &[u8],
    result: &mut ParseResult,
    caller_index: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "method_invocation" {
            // method_invocation children: [object.]name(args)
            // The last identifier before argument_list is the method name
            let name = extract_java_invocation_name(&child, src);
            if let Some(name) = name {
                result.calls.push(Call {
                    caller_index,
                    callee_name: name,
                    line: child.start_position().row + 1,
                });
            }
        }
        if child.kind() == "object_creation_expression" {
            // new Foo(...) — treat as call to constructor
            if let Some(type_node) = child.child_by_field_name("type") {
                let name = node_text(type_node, src).to_string();
                result.calls.push(Call {
                    caller_index,
                    callee_name: name,
                    line: child.start_position().row + 1,
                });
            }
        }
        extract_java_calls(&child, src, result, caller_index);
    }
}

/// Extract the method name from a method_invocation node.
/// Java method_invocation has children like: [object, ".", name, argument_list]
/// or just [name, argument_list] for simple calls.
fn extract_java_invocation_name(node: &tree_sitter::Node, src: &[u8]) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(node_text(name_node, src).to_string());
    }
    // Fallback: find last identifier before argument_list
    let mut cursor = node.walk();
    let mut last_ident = None;
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            last_ident = Some(node_text(child, src).to_string());
        }
        if child.kind() == "argument_list" {
            break;
        }
    }
    last_ident
}

fn build_java_method_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
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

fn build_java_constructor_signature(node: &tree_sitter::Node, src: &[u8]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_python_function() -> anyhow::Result<()> {
        let src = r#"
def hello(name):
    print(name)
    return greet(name)
"#;
        let result = parse_source(src, Language::Python)?;
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "hello");
        assert_eq!(result.symbols[0].kind, "function");
        assert!(
            result.symbols[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("def hello")
        );

        // Should find calls to print and greet
        assert!(result.calls.iter().any(|c| c.callee_name == "print"));
        assert!(result.calls.iter().any(|c| c.callee_name == "greet"));
        Ok(())
    }

    #[test]
    fn test_parse_python_class() -> anyhow::Result<()> {
        let src = r#"
class MyClass:
    def method_one(self):
        pass

    def method_two(self, x):
        self.method_one()
"#;
        let result = parse_source(src, Language::Python)?;
        assert_eq!(result.symbols.len(), 3); // class + 2 methods
        assert_eq!(result.symbols[0].kind, "class");
        assert_eq!(result.symbols[1].kind, "method");
        assert_eq!(result.symbols[1].parent_index, Some(0));
        Ok(())
    }

    #[test]
    fn test_parse_python_imports() -> anyhow::Result<()> {
        let src = r#"
import os
from pathlib import Path
from typing import Optional, List
"#;
        let result = parse_source(src, Language::Python)?;
        assert_eq!(result.imports.len(), 3);
        assert_eq!(result.imports[0].module, "os");
        assert_eq!(result.imports[1].module, "pathlib");
        Ok(())
    }

    #[test]
    fn test_parse_rust_function() -> anyhow::Result<()> {
        let src = r#"
fn process(input: &str) -> Result<()> {
    let x = parse(input);
    validate(x)
}
"#;
        let result = parse_source(src, Language::Rust)?;
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "process");
        assert!(
            result.symbols[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("-> Result<()>")
        );

        assert!(result.calls.iter().any(|c| c.callee_name == "parse"));
        assert!(result.calls.iter().any(|c| c.callee_name == "validate"));
        Ok(())
    }

    #[test]
    fn test_parse_rust_struct_and_impl() -> anyhow::Result<()> {
        let src = r#"
struct Config {
    name: String,
}

impl Config {
    fn new(name: String) -> Self {
        Self { name }
    }
}
"#;
        let result = parse_source(src, Language::Rust)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Config" && s.kind == "struct")
        );
        let new_fn = result.symbols.iter().find(|s| s.name == "new").unwrap();
        assert_eq!(new_fn.kind, "method");
        // Parent should point to Config
        assert_eq!(new_fn.parent_index, Some(0));
        Ok(())
    }

    #[test]
    fn test_parse_js_function() -> anyhow::Result<()> {
        let src = r#"
function handleRequest(req, res) {
    const data = parseBody(req);
    res.send(data);
}
"#;
        let result = parse_source(src, Language::JavaScript)?;
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "handleRequest");
        assert!(result.calls.iter().any(|c| c.callee_name == "parseBody"));
        Ok(())
    }

    #[test]
    fn test_parse_js_arrow_function() -> anyhow::Result<()> {
        let src = r#"
const greet = (name) => {
    return format(name);
};
"#;
        let result = parse_source(src, Language::JavaScript)?;
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "greet");
        assert!(result.calls.iter().any(|c| c.callee_name == "format"));
        Ok(())
    }

    #[test]
    fn test_parse_ts_imports() -> anyhow::Result<()> {
        let src = r#"
import { Router } from 'express';
import path from 'path';
"#;
        let result = parse_source(src, Language::TypeScript)?;
        assert_eq!(result.imports.len(), 2);
        assert_eq!(result.imports[0].module, "express");
        assert_eq!(result.imports[1].module, "path");
        Ok(())
    }

    #[test]
    fn test_parse_js_class_and_method() -> anyhow::Result<()> {
        let src = r#"
class UserService {
    getUser(id) {
        return findById(id);
    }
}
"#;
        let result = parse_source(src, Language::JavaScript)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "UserService" && s.kind == "class")
        );
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "getUser" && s.kind == "method")
        );
        assert!(result.calls.iter().any(|c| c.callee_name == "findById"));
        Ok(())
    }

    #[test]
    fn test_parse_rust_enum() -> anyhow::Result<()> {
        let src = r#"
enum Color {
    Red,
    Green,
    Blue,
}
"#;
        let result = parse_source(src, Language::Rust)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Color" && s.kind == "enum")
        );
        Ok(())
    }

    #[test]
    fn test_parse_rust_trait() -> anyhow::Result<()> {
        let src = r#"
trait Drawable {
    fn draw(&self) {}
    fn resize(&mut self, w: u32, h: u32) {}
}
"#;
        let result = parse_source(src, Language::Rust)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Drawable" && s.kind == "trait")
        );
        // Methods with bodies inside trait should be parsed as children
        let draw = result.symbols.iter().find(|s| s.name == "draw");
        assert!(
            draw.is_some(),
            "draw should be found; symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| format!("{} ({})", s.name, s.kind))
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn test_parse_rust_use_declaration() -> anyhow::Result<()> {
        let src = r#"
use std::collections::HashMap;
use anyhow::Result;
"#;
        let result = parse_source(src, Language::Rust)?;
        assert_eq!(result.imports.len(), 2);
        assert!(result.imports.iter().any(|i| i.module.contains("HashMap")));
        assert!(result.imports.iter().any(|i| i.module.contains("Result")));
        Ok(())
    }

    #[test]
    fn test_parse_rust_macro_invocation() -> anyhow::Result<()> {
        let src = r#"
fn main() {
    println!("hello");
    vec![1, 2, 3];
}
"#;
        let result = parse_source(src, Language::Rust)?;
        assert!(result.calls.iter().any(|c| c.callee_name == "println"));
        assert!(result.calls.iter().any(|c| c.callee_name == "vec"));
        Ok(())
    }

    #[test]
    fn test_parse_rust_method_call() -> anyhow::Result<()> {
        let src = r#"
fn process(items: Vec<String>) {
    items.iter().map(|x| x.len()).collect();
}
"#;
        let result = parse_source(src, Language::Rust)?;
        // field_expression calls: iter, map, collect
        assert!(result.calls.iter().any(|c| c.callee_name == "iter"));
        assert!(result.calls.iter().any(|c| c.callee_name == "map"));
        Ok(())
    }

    #[test]
    fn test_parse_rust_scoped_call() -> anyhow::Result<()> {
        let src = r#"
fn build() {
    let x = String::from("hello");
    let y = std::fs::read_to_string("file");
}
"#;
        let result = parse_source(src, Language::Rust)?;
        assert!(result.calls.iter().any(|c| c.callee_name == "from"));
        assert!(
            result
                .calls
                .iter()
                .any(|c| c.callee_name == "read_to_string")
        );
        Ok(())
    }

    #[test]
    fn test_parse_python_attribute_call() -> anyhow::Result<()> {
        let src = r#"
def process(data):
    result = data.transform()
    result.save()
"#;
        let result = parse_source(src, Language::Python)?;
        assert!(result.calls.iter().any(|c| c.callee_name == "transform"));
        assert!(result.calls.iter().any(|c| c.callee_name == "save"));
        Ok(())
    }

    #[test]
    fn test_parse_js_member_call() -> anyhow::Result<()> {
        let src = r#"
function handler(req) {
    const body = req.json();
    console.log(body);
}
"#;
        let result = parse_source(src, Language::JavaScript)?;
        assert!(result.calls.iter().any(|c| c.callee_name == "json"));
        assert!(result.calls.iter().any(|c| c.callee_name == "log"));
        Ok(())
    }

    #[test]
    fn test_parse_tsx() -> anyhow::Result<()> {
        let src = r#"
function App() {
    return <div>Hello</div>;
}
"#;
        let result = parse_source(src, Language::Tsx)?;
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "App");
        Ok(())
    }

    #[test]
    fn test_parse_python_aliased_import() -> anyhow::Result<()> {
        let src = r#"
from collections import OrderedDict as OD
"#;
        let result = parse_source(src, Language::Python)?;
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].module, "collections");
        assert!(
            result.imports[0]
                .names
                .as_ref()
                .unwrap()
                .contains("OrderedDict")
        );
        Ok(())
    }

    #[test]
    fn test_parse_js_named_imports() -> anyhow::Result<()> {
        let src = r#"
import { useState, useEffect } from 'react';
"#;
        let result = parse_source(src, Language::JavaScript)?;
        assert_eq!(result.imports.len(), 1);
        let names = result.imports[0].names.as_ref().unwrap();
        assert!(names.contains("useState"));
        assert!(names.contains("useEffect"));
        Ok(())
    }

    #[test]
    fn test_parse_rust_impl_without_matching_struct() -> anyhow::Result<()> {
        // impl for a type not defined in this file
        let src = r#"
impl ExternalType {
    fn method(&self) {}
}
"#;
        let result = parse_source(src, Language::Rust)?;
        let method = result.symbols.iter().find(|s| s.name == "method");
        assert!(method.is_some());
        // Parent should be None since ExternalType isn't defined here
        assert!(method.unwrap().parent_index.is_none());
        Ok(())
    }

    #[test]
    fn test_parse_js_export_function() -> anyhow::Result<()> {
        let src = r#"
export function serve(port) {
    listen(port);
}
"#;
        let result = parse_source(src, Language::JavaScript)?;
        assert!(result.symbols.iter().any(|s| s.name == "serve"));
        assert!(result.calls.iter().any(|c| c.callee_name == "listen"));
        Ok(())
    }

    #[test]
    fn test_parse_rust_impl_for_enum() -> anyhow::Result<()> {
        let src = r#"
enum Status {
    Active,
    Inactive,
}

impl Status {
    fn is_active(&self) -> bool {
        matches!(self, Status::Active)
    }
}
"#;
        let result = parse_source(src, Language::Rust)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Status" && s.kind == "enum")
        );
        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "is_active")
            .unwrap();
        // Parent should be the enum
        assert_eq!(method.parent_index, Some(0));
        Ok(())
    }

    #[test]
    fn test_parse_ts_function() -> anyhow::Result<()> {
        let src = r#"
function greet(name: string): string {
    return format(name);
}
"#;
        let result = parse_source(src, Language::TypeScript)?;
        assert!(result.symbols.iter().any(|s| s.name == "greet"));
        assert!(result.calls.iter().any(|c| c.callee_name == "format"));
        Ok(())
    }

    #[test]
    fn test_parse_js_class_in_export() -> anyhow::Result<()> {
        let src = r#"
export class Router {
    route(path) {
        return match(path);
    }
}
"#;
        let result = parse_source(src, Language::JavaScript)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Router" && s.kind == "class")
        );
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "route" && s.kind == "method")
        );
        Ok(())
    }

    #[test]
    fn test_parse_python_nested_calls() -> anyhow::Result<()> {
        let src = r#"
def process():
    result = transform(parse(read_file("data.txt")))
"#;
        let result = parse_source(src, Language::Python)?;
        assert!(result.calls.iter().any(|c| c.callee_name == "transform"));
        assert!(result.calls.iter().any(|c| c.callee_name == "parse"));
        assert!(result.calls.iter().any(|c| c.callee_name == "read_file"));
        Ok(())
    }

    #[test]
    fn test_parse_go_function() -> anyhow::Result<()> {
        let src = r#"
package main

func processData(input string) error {
    result := parse(input)
    return validate(result)
}
"#;
        let result = parse_source(src, Language::Go)?;
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "processData");
        assert_eq!(result.symbols[0].kind, "function");
        assert!(
            result.symbols[0]
                .signature
                .as_ref()
                .unwrap()
                .contains("func processData")
        );

        assert!(result.calls.iter().any(|c| c.callee_name == "parse"));
        assert!(result.calls.iter().any(|c| c.callee_name == "validate"));
        Ok(())
    }

    #[test]
    fn test_parse_go_struct_and_method() -> anyhow::Result<()> {
        let src = r#"
package main

type Server struct {
    port int
}

func (s *Server) Start() error {
    return listen(s.port)
}
"#;
        let result = parse_source(src, Language::Go)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Server" && s.kind == "struct")
        );
        let method = result.symbols.iter().find(|s| s.name == "Start").unwrap();
        assert_eq!(method.kind, "method");
        assert_eq!(method.parent_index, Some(0));
        assert!(result.calls.iter().any(|c| c.callee_name == "listen"));
        Ok(())
    }

    #[test]
    fn test_parse_go_interface() -> anyhow::Result<()> {
        let src = r#"
package main

type Handler interface {
    ServeHTTP(w ResponseWriter, r *Request)
}
"#;
        let result = parse_source(src, Language::Go)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Handler" && s.kind == "interface")
        );
        Ok(())
    }

    #[test]
    fn test_parse_go_imports() -> anyhow::Result<()> {
        let src = r#"
package main

import (
    "fmt"
    "net/http"
)
"#;
        let result = parse_source(src, Language::Go)?;
        assert_eq!(result.imports.len(), 2);
        assert!(result.imports.iter().any(|i| i.module == "fmt"));
        assert!(result.imports.iter().any(|i| i.module == "net/http"));
        Ok(())
    }

    #[test]
    fn test_parse_go_selector_call() -> anyhow::Result<()> {
        let src = r#"
package main

import "fmt"

func main() {
    fmt.Println("hello")
}
"#;
        let result = parse_source(src, Language::Go)?;
        assert!(result.calls.iter().any(|c| c.callee_name == "Println"));
        Ok(())
    }

    #[test]
    fn test_parse_go_return_type() -> anyhow::Result<()> {
        let src = r#"
package main

func NewServer(port int) *Server {
    return &Server{port: port}
}
"#;
        let result = parse_source(src, Language::Go)?;
        let sig = result.symbols[0].signature.as_ref().unwrap();
        assert!(
            sig.contains("*Server"),
            "signature should contain return type: {sig}"
        );
        Ok(())
    }

    #[test]
    fn test_parse_go_generic_receiver() -> anyhow::Result<()> {
        let src = r#"
package main

type Box[T any] struct {
    value T
}

func (b *Box[T]) Get() T {
    return b.value
}

func (b Box[T]) String() string {
    return "box"
}
"#;
        let result = parse_source(src, Language::Go)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Box" && s.kind == "struct")
        );
        let get = result.symbols.iter().find(|s| s.name == "Get").unwrap();
        assert_eq!(get.kind, "method");
        assert_eq!(get.parent_index, Some(0), "Get should link to Box struct");
        let string_method = result.symbols.iter().find(|s| s.name == "String").unwrap();
        assert_eq!(
            string_method.parent_index,
            Some(0),
            "String should link to Box struct"
        );
        Ok(())
    }

    #[test]
    fn test_parse_go_raw_string_import() -> anyhow::Result<()> {
        let src = "package main\n\nimport `net/http`\n";
        let result = parse_source(src, Language::Go)?;
        assert!(
            result.imports.iter().any(|i| i.module == "net/http"),
            "raw string import should be stripped of backticks: {:?}",
            result.imports
        );
        Ok(())
    }

    #[test]
    fn test_parse_empty_source() -> anyhow::Result<()> {
        let result = parse_source("", Language::Python)?;
        assert!(result.symbols.is_empty());
        assert!(result.calls.is_empty());
        assert!(result.imports.is_empty());
        Ok(())
    }

    // -- Java tests --

    #[test]
    fn test_parse_java_class_and_methods() -> anyhow::Result<()> {
        let src = r#"
public class AuthService {
    public User authenticate(String username, String password) {
        return null;
    }

    public void logout() {
    }
}
"#;
        let result = parse_source(src, Language::Java)?;
        let class = result.symbols.iter().find(|s| s.name == "AuthService");
        assert!(class.is_some(), "should find AuthService class");
        assert_eq!(class.unwrap().kind, "class");

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == "method")
            .collect();
        assert_eq!(methods.len(), 2);
        assert!(methods.iter().any(|m| m.name == "authenticate"));
        assert!(methods.iter().any(|m| m.name == "logout"));

        // Methods should be children of the class
        let class_idx = result
            .symbols
            .iter()
            .position(|s| s.name == "AuthService")
            .unwrap();
        for m in &methods {
            assert_eq!(m.parent_index, Some(class_idx));
        }
        Ok(())
    }

    #[test]
    fn test_parse_java_interface() -> anyhow::Result<()> {
        let src = r#"
interface Authenticator {
    User authenticate(String username, String password);
    boolean isValid(String token);
}
"#;
        let result = parse_source(src, Language::Java)?;
        let iface = result.symbols.iter().find(|s| s.name == "Authenticator");
        assert!(iface.is_some(), "should find Authenticator interface");
        assert_eq!(iface.unwrap().kind, "interface");

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == "method")
            .collect();
        assert_eq!(methods.len(), 2);
        Ok(())
    }

    #[test]
    fn test_parse_java_enum() -> anyhow::Result<()> {
        let src = r#"
enum Role {
    ADMIN,
    USER,
    GUEST
}
"#;
        let result = parse_source(src, Language::Java)?;
        let enm = result.symbols.iter().find(|s| s.name == "Role");
        assert!(enm.is_some(), "should find Role enum");
        assert_eq!(enm.unwrap().kind, "enum");
        Ok(())
    }

    #[test]
    fn test_parse_java_imports() -> anyhow::Result<()> {
        let src = r#"
import java.util.List;
import com.example.models.User;
"#;
        let result = parse_source(src, Language::Java)?;
        assert_eq!(result.imports.len(), 2);
        assert!(result.imports.iter().any(|i| i.module == "java.util.List"));
        assert!(
            result
                .imports
                .iter()
                .any(|i| i.module == "com.example.models.User")
        );
        Ok(())
    }

    #[test]
    fn test_parse_java_method_calls() -> anyhow::Result<()> {
        let src = r#"
class Foo {
    public void bar() {
        baz();
        obj.method();
    }
}
"#;
        let result = parse_source(src, Language::Java)?;
        assert!(
            result.calls.iter().any(|c| c.callee_name == "baz"),
            "should find plain call: {:?}",
            result.calls
        );
        assert!(
            result.calls.iter().any(|c| c.callee_name == "method"),
            "should find qualified call: {:?}",
            result.calls
        );
        Ok(())
    }

    #[test]
    fn test_parse_java_constructor() -> anyhow::Result<()> {
        let src = r#"
public class AuthService {
    private final UserRepository userRepo;

    public AuthService(UserRepository userRepo) {
        this.userRepo = userRepo;
    }
}
"#;
        let result = parse_source(src, Language::Java)?;
        let ctor = result.symbols.iter().find(|s| s.kind == "constructor");
        assert!(ctor.is_some(), "should find constructor");
        assert_eq!(ctor.unwrap().name, "AuthService");
        assert!(
            ctor.unwrap()
                .signature
                .as_ref()
                .unwrap()
                .contains("UserRepository")
        );
        Ok(())
    }

    #[test]
    fn test_parse_java_method_signature() -> anyhow::Result<()> {
        let src = r#"
class Foo {
    public List<User> listUsers(String filter) {
        return null;
    }
}
"#;
        let result = parse_source(src, Language::Java)?;
        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "listUsers")
            .unwrap();
        let sig = method.signature.as_ref().unwrap();
        assert!(
            sig.contains("List<User>"),
            "sig should have return type: {sig}"
        );
        assert!(
            sig.contains("String filter"),
            "sig should have params: {sig}"
        );
        Ok(())
    }

    #[test]
    fn test_parse_java_new_expression() -> anyhow::Result<()> {
        let src = r#"
class Foo {
    public void bar() {
        User u = new User();
    }
}
"#;
        let result = parse_source(src, Language::Java)?;
        assert!(
            result.calls.iter().any(|c| c.callee_name == "User"),
            "should find constructor call via new: {:?}",
            result.calls
        );
        Ok(())
    }
}
