//! Tree-sitter based symbol, import, and call extraction.
//!
//! Python reference: src/pruner/parser.py

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
    }
}

fn ts_language_for(language: Language) -> tree_sitter::Language {
    match language {
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
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
                    let kind = if parent_index.is_some() { "method" } else { "function" };
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
                        names: if names.is_empty() { None } else { Some(names.join(", ")) },
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
        let module = node_text(source, src).trim_matches(|c| c == '\'' || c == '"').to_string();
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
            names: if names.is_empty() { None } else { Some(names.join(", ")) },
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
                    let kind = if parent_index.is_some() { "method" } else { "function" };
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
                    let impl_parent = result
                        .symbols
                        .iter()
                        .position(|s| s.name == type_name && (s.kind == "struct" || s.kind == "enum"));
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
        assert!(result.symbols[0].signature.as_ref().unwrap().contains("def hello"));

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
        assert!(result.symbols[0].signature.as_ref().unwrap().contains("-> Result<()>"));

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
        assert!(result.symbols.iter().any(|s| s.name == "Config" && s.kind == "struct"));
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
}
