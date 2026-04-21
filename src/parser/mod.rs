//! Tree-sitter based symbol, import, and call extraction.
//!
//! Each supported language lives in its own submodule with a `pub(super) fn extract(root, src)`.
//! Shared helpers (`node_text`, `normalize_type_name`) live in [`common`].

mod c;
mod common;
mod cpp;
mod csharp;
mod go;
mod java;
mod javascript;
mod python;
mod rust;

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
        Language::Python => python::extract(root, src),
        Language::JavaScript | Language::TypeScript | Language::Tsx => {
            javascript::extract(root, src)
        }
        Language::Rust => rust::extract(root, src),
        Language::Go => go::extract(root, src),
        Language::Java => java::extract(root, src),
        Language::C => c::extract(root, src),
        Language::Cpp => cpp::extract(root, src),
        Language::Csharp => csharp::extract(root, src),
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
        Language::C => tree_sitter_c::LANGUAGE.into(),
        Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Language::Csharp => tree_sitter_c_sharp::LANGUAGE.into(),
    }
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

    #[test]
    fn test_parse_java_generic_new_expression() -> anyhow::Result<()> {
        let src = r#"
class Foo {
    public void bar() {
        List<String> list = new ArrayList<String>();
    }
}
"#;
        let result = parse_source(src, Language::Java)?;
        assert!(
            result.calls.iter().any(|c| c.callee_name == "ArrayList"),
            "should normalize generic constructor to base name: {:?}",
            result.calls
        );
        Ok(())
    }

    #[test]
    fn test_normalize_type_name() {
        use super::common::normalize_type_name;
        assert_eq!(normalize_type_name("User"), "User");
        assert_eq!(normalize_type_name("Box<String>"), "Box");
        assert_eq!(normalize_type_name("List<Map<K,V>>"), "List");
        assert_eq!(normalize_type_name("com.foo.User"), "User");
        assert_eq!(normalize_type_name("com.foo.Box<String>"), "Box");
    }

    // -- C tests --

    #[test]
    fn test_parse_c_function() -> anyhow::Result<()> {
        let src = r#"
#include <stdio.h>

int authenticate(const char* email, const char* password) {
    return 1;
}
"#;
        let result = parse_source(src, Language::C)?;
        assert!(result.symbols.iter().any(|s| s.name == "authenticate"
            && s.kind == "function"
            && s.signature.as_ref().unwrap().contains("authenticate")));
        assert!(result.imports.iter().any(|i| i.module == "stdio.h"));
        Ok(())
    }

    #[test]
    fn test_parse_c_struct_and_typedef() -> anyhow::Result<()> {
        let src = r#"
struct User {
    int id;
    char* name;
};

typedef struct {
    int x;
    int y;
} Point;
"#;
        let result = parse_source(src, Language::C)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "User" && s.kind == "struct"),
            "should find struct User"
        );
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Point" && s.kind == "type"),
            "should find typedef Point, got: {:?}",
            result.symbols
        );
        Ok(())
    }

    #[test]
    fn test_parse_c_enum() -> anyhow::Result<()> {
        let src = r#"
enum Color {
    RED,
    GREEN,
    BLUE
};
"#;
        let result = parse_source(src, Language::C)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Color" && s.kind == "enum")
        );
        Ok(())
    }

    #[test]
    fn test_parse_c_calls() -> anyhow::Result<()> {
        let src = r#"
void process(void) {
    int x = calculate(42);
    printf("result: %d\n", x);
}
"#;
        let result = parse_source(src, Language::C)?;
        assert!(result.calls.iter().any(|c| c.callee_name == "calculate"));
        assert!(result.calls.iter().any(|c| c.callee_name == "printf"));
        Ok(())
    }

    #[test]
    fn test_parse_c_includes() -> anyhow::Result<()> {
        let src = r#"
#include <stdio.h>
#include "myheader.h"
#include <stdlib.h>
"#;
        let result = parse_source(src, Language::C)?;
        assert_eq!(result.imports.len(), 3);
        assert!(result.imports.iter().any(|i| i.module == "stdio.h"));
        assert!(result.imports.iter().any(|i| i.module == "myheader.h"));
        assert!(result.imports.iter().any(|i| i.module == "stdlib.h"));
        Ok(())
    }

    // -- C++ tests --

    #[test]
    fn test_parse_cpp_class_and_methods() -> anyhow::Result<()> {
        let src = r#"
#include <string>

class UserService {
public:
    void authenticate(const std::string& email) {
        findUser(email);
    }
};
"#;
        let result = parse_source(src, Language::Cpp)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "UserService" && s.kind == "class"),
            "should find class UserService, got: {:?}",
            result.symbols
        );
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "authenticate" && s.kind == "method"),
            "should find method authenticate, got: {:?}",
            result.symbols
        );
        // Method should be parented to class
        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "authenticate")
            .unwrap();
        assert!(method.parent_index.is_some());
        let parent = &result.symbols[method.parent_index.unwrap()];
        assert_eq!(parent.name, "UserService");
        Ok(())
    }

    #[test]
    fn test_parse_cpp_namespace() -> anyhow::Result<()> {
        let src = r#"
namespace auth {
    void login() {}
}
"#;
        let result = parse_source(src, Language::Cpp)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "auth" && s.kind == "namespace")
        );
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "login" && s.kind == "function")
        );
        // login should be parented to namespace
        let login = result.symbols.iter().find(|s| s.name == "login").unwrap();
        assert!(login.parent_index.is_some());
        Ok(())
    }

    #[test]
    fn test_parse_cpp_out_of_line_method() -> anyhow::Result<()> {
        let src = r#"
class Server {
};

void Server::start() {
    listen(8080);
}
"#;
        let result = parse_source(src, Language::Cpp)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "start" && s.kind == "method"),
            "should find out-of-line method, got: {:?}",
            result.symbols
        );
        // Should have a call to listen
        assert!(result.calls.iter().any(|c| c.callee_name == "listen"));
        Ok(())
    }

    #[test]
    fn test_parse_cpp_includes() -> anyhow::Result<()> {
        let src = r#"
#include <iostream>
#include <vector>
#include "server.h"
"#;
        let result = parse_source(src, Language::Cpp)?;
        assert_eq!(result.imports.len(), 3);
        assert!(result.imports.iter().any(|i| i.module == "iostream"));
        assert!(result.imports.iter().any(|i| i.module == "vector"));
        assert!(result.imports.iter().any(|i| i.module == "server.h"));
        Ok(())
    }

    #[test]
    fn test_parse_cpp_struct() -> anyhow::Result<()> {
        let src = r#"
struct Point {
    int x;
    int y;
    double distance() { return 0.0; }
};
"#;
        let result = parse_source(src, Language::Cpp)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Point" && s.kind == "struct")
        );
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "distance" && s.kind == "method"),
            "should find inline method in struct"
        );
        Ok(())
    }

    #[test]
    fn test_parse_cpp_calls() -> anyhow::Result<()> {
        let src = r#"
void process() {
    auto result = compute(42);
    obj->sendMessage("hello");
}
"#;
        let result = parse_source(src, Language::Cpp)?;
        assert!(result.calls.iter().any(|c| c.callee_name == "compute"));
        assert!(result.calls.iter().any(|c| c.callee_name == "sendMessage"));
        Ok(())
    }

    #[test]
    fn test_parse_cpp_qualified_free_function_is_function() -> anyhow::Result<()> {
        let src = r#"
namespace util {
    void log();
}

void util::log() {
    printf("hello");
}
"#;
        let result = parse_source(src, Language::Cpp)?;
        let log_fn = result
            .symbols
            .iter()
            .find(|s| s.name == "log" && s.kind == "function")
            .expect("util::log should be classified as function, not method");
        // Should be parented to namespace, not treated as a class method
        assert!(
            log_fn.parent_index.is_none()
                || result.symbols[log_fn.parent_index.unwrap()].kind == "namespace",
            "free function should not be parented to a class"
        );
        Ok(())
    }

    #[test]
    fn test_parse_cpp_deeply_qualified_method_parent() -> anyhow::Result<()> {
        let src = r#"
namespace auth {
    class AuthService {};
}

void auth::AuthService::authenticate() {
    validate();
}
"#;
        let result = parse_source(src, Language::Cpp)?;
        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "authenticate")
            .expect("should find authenticate");
        assert_eq!(method.kind, "method");
        // Parent should be AuthService, not auth
        if let Some(pi) = method.parent_index {
            assert_eq!(result.symbols[pi].name, "AuthService");
        }
        Ok(())
    }

    #[test]
    fn test_parse_cpp_anonymous_namespace() -> anyhow::Result<()> {
        let src = r#"
namespace {
    void helper() {
        do_work();
    }
}
"#;
        let result = parse_source(src, Language::Cpp)?;
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "helper" && s.kind == "function"),
            "should find function inside anonymous namespace, got: {:?}",
            result.symbols
        );
        assert!(
            result.calls.iter().any(|c| c.callee_name == "do_work"),
            "should find calls inside anonymous namespace"
        );
        Ok(())
    }

    #[test]
    fn test_parse_cpp_qualified_calls() -> anyhow::Result<()> {
        let src = r#"
void process() {
    std::move(x);
    ns::foo(42);
    Type::static_fn();
}
"#;
        let result = parse_source(src, Language::Cpp)?;
        assert!(
            result.calls.iter().any(|c| c.callee_name == "move"),
            "should capture std::move, got: {:?}",
            result.calls
        );
        assert!(
            result.calls.iter().any(|c| c.callee_name == "foo"),
            "should capture ns::foo, got: {:?}",
            result.calls
        );
        assert!(
            result.calls.iter().any(|c| c.callee_name == "static_fn"),
            "should capture Type::static_fn, got: {:?}",
            result.calls
        );
        Ok(())
    }

    // -- C# tests --

    #[test]
    fn test_parse_csharp_class_and_methods() -> anyhow::Result<()> {
        let src = r#"
using System;

namespace Example
{
    public class AuthService
    {
        public string Authenticate(string username, string password)
        {
            return Validate(username, password);
        }

        private bool Validate(string username, string password)
        {
            return true;
        }
    }
}
"#;
        let result = parse_source(src, Language::Csharp)?;
        let class = result.symbols.iter().find(|s| s.name == "AuthService");
        assert!(class.is_some(), "should find AuthService class");
        assert_eq!(class.unwrap().kind, "class");

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == "method")
            .collect();
        assert_eq!(methods.len(), 2);
        assert!(methods.iter().any(|m| m.name == "Authenticate"));

        let class_idx = result
            .symbols
            .iter()
            .position(|s| s.name == "AuthService")
            .unwrap();
        for m in &methods {
            assert_eq!(m.parent_index, Some(class_idx));
        }
        assert!(result.calls.iter().any(|c| c.callee_name == "Validate"));
        Ok(())
    }

    #[test]
    fn test_parse_csharp_namespace() -> anyhow::Result<()> {
        let src = r#"
namespace MyApp.Auth
{
    public class TokenService
    {
        public string Generate() { return "tok"; }
    }
}
"#;
        let result = parse_source(src, Language::Csharp)?;
        let ns = result.symbols.iter().find(|s| s.kind == "namespace");
        assert!(ns.is_some(), "should find namespace");
        assert_eq!(ns.unwrap().name, "MyApp.Auth");

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "TokenService")
            .unwrap();
        let ns_idx = result
            .symbols
            .iter()
            .position(|s| s.kind == "namespace")
            .unwrap();
        assert_eq!(
            class.parent_index,
            Some(ns_idx),
            "class should be child of namespace"
        );
        Ok(())
    }

    #[test]
    fn test_parse_csharp_file_scoped_namespace() -> anyhow::Result<()> {
        let src = "namespace MyApp.Models;\n\npublic class User\n{\n    public string GetName() { return \"\"; }\n}\n";
        let result = parse_source(src, Language::Csharp)?;
        let ns = result.symbols.iter().find(|s| s.kind == "namespace");
        assert!(ns.is_some(), "should find file-scoped namespace");
        let class = result.symbols.iter().find(|s| s.name == "User").unwrap();
        let ns_idx = result
            .symbols
            .iter()
            .position(|s| s.kind == "namespace")
            .unwrap();
        assert_eq!(
            class.parent_index,
            Some(ns_idx),
            "class should be child of file-scoped namespace"
        );
        Ok(())
    }

    #[test]
    fn test_parse_csharp_imports() -> anyhow::Result<()> {
        let src = r#"
using System;
using System.Collections.Generic;
using MyApp.Models;
"#;
        let result = parse_source(src, Language::Csharp)?;
        assert_eq!(result.imports.len(), 3);
        assert!(result.imports.iter().any(|i| i.module == "System"));
        assert!(result.imports.iter().any(|i| i.module.contains("Generic")));
        assert!(result.imports.iter().any(|i| i.module.contains("Models")));
        Ok(())
    }

    #[test]
    fn test_parse_csharp_interface() -> anyhow::Result<()> {
        let src = r#"
public interface IAuthService
{
    string Authenticate(string username, string password);
    bool IsValid(string token);
}
"#;
        let result = parse_source(src, Language::Csharp)?;
        let iface = result.symbols.iter().find(|s| s.name == "IAuthService");
        assert!(iface.is_some(), "should find interface");
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
    fn test_parse_csharp_calls() -> anyhow::Result<()> {
        let src = r#"
class Foo
{
    void Bar()
    {
        Baz();
        obj.Method();
        var x = new User();
    }
}
"#;
        let result = parse_source(src, Language::Csharp)?;
        assert!(
            result.calls.iter().any(|c| c.callee_name == "Baz"),
            "should find plain call: {:?}",
            result.calls
        );
        assert!(
            result.calls.iter().any(|c| c.callee_name == "Method"),
            "should find qualified call: {:?}",
            result.calls
        );
        assert!(
            result.calls.iter().any(|c| c.callee_name == "User"),
            "should find constructor call: {:?}",
            result.calls
        );
        Ok(())
    }

    #[test]
    fn test_parse_csharp_constructor() -> anyhow::Result<()> {
        let src = r#"
public class AuthService
{
    private readonly UserRepository _repo;

    public AuthService(UserRepository repo)
    {
        _repo = repo;
    }
}
"#;
        let result = parse_source(src, Language::Csharp)?;
        let ctor = result.symbols.iter().find(|s| s.kind == "constructor");
        assert!(ctor.is_some(), "should find constructor");
        assert_eq!(ctor.unwrap().name, "AuthService");
        assert!(
            ctor.unwrap()
                .signature
                .as_ref()
                .unwrap()
                .contains("UserRepository"),
            "signature should contain param type"
        );
        Ok(())
    }

    #[test]
    fn test_parse_csharp_alias_using() -> anyhow::Result<()> {
        let src = r#"
using System;
using Auth = MyApp.Services.AuthService;
using MyApp.Models;
"#;
        let result = parse_source(src, Language::Csharp)?;
        assert_eq!(result.imports.len(), 3);
        assert!(result.imports.iter().any(|i| i.module == "System"));
        // Alias using should record the target, not the alias name
        assert!(
            result
                .imports
                .iter()
                .any(|i| i.module.contains("AuthService")),
            "should import target, not alias"
        );
        assert!(
            !result.imports.iter().any(|i| i.module == "Auth"),
            "should not import alias name"
        );
        assert!(result.imports.iter().any(|i| i.module.contains("Models")));
        Ok(())
    }

    #[test]
    fn test_parse_csharp_generic_member_call() -> anyhow::Result<()> {
        let src = r#"
class Foo
{
    void Bar()
    {
        var x = list.FirstOrDefault<string>();
        var y = repo.Get<User>();
    }
}
"#;
        let result = parse_source(src, Language::Csharp)?;
        // Generic type args should be stripped from member call names
        let call_names: Vec<&str> = result
            .calls
            .iter()
            .map(|c| c.callee_name.as_str())
            .collect();
        assert!(
            !call_names.iter().any(|n| n.contains('<')),
            "call names should not contain type arguments, got: {:?}",
            call_names
        );
        Ok(())
    }

    #[test]
    fn test_parse_csharp_properties() -> anyhow::Result<()> {
        let src = r#"
class User
{
    public string Name { get; set; }
    public int Age { get; }
}
"#;
        let result = parse_source(src, Language::Csharp)?;
        let props: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == "property")
            .collect();
        assert_eq!(props.len(), 2, "should find 2 properties");
        assert!(props.iter().any(|p| p.name == "Name"));
        assert!(props.iter().any(|p| p.name == "Age"));
        Ok(())
    }

    #[test]
    fn test_jsx_component_as_call_edge() -> anyhow::Result<()> {
        let src = r#"
function App() {
    return (
        <div>
            <Header title="Hello" />
            <Sidebar items={items} />
        </div>
    );
}
"#;
        let result = parse_source(src, Language::Tsx)?;
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "App");
        // JSX elements with uppercase names should be call edges
        assert!(result.calls.iter().any(|c| c.callee_name == "Header"));
        assert!(result.calls.iter().any(|c| c.callee_name == "Sidebar"));
        // Lowercase elements like <div> should NOT be call edges
        assert!(!result.calls.iter().any(|c| c.callee_name == "div"));
        Ok(())
    }

    #[test]
    fn test_jsx_nested_components() -> anyhow::Result<()> {
        let src = r#"
function Page() {
    return (
        <Layout>
            <Nav.Menu />
        </Layout>
    );
}
"#;
        let result = parse_source(src, Language::Tsx)?;
        assert!(result.calls.iter().any(|c| c.callee_name == "Layout"));
        // Member expression JSX: <Nav.Menu /> → call to "Menu"
        assert!(result.calls.iter().any(|c| c.callee_name == "Menu"));
        Ok(())
    }

    #[test]
    fn test_dynamic_import_in_function() -> anyhow::Result<()> {
        let src = r#"
async function loadModule() {
    const mod = await import('./heavy-module');
    return mod;
}
"#;
        let result = parse_source(src, Language::JavaScript)?;
        assert_eq!(result.symbols.len(), 1);
        assert!(result.imports.iter().any(|i| i.module == "./heavy-module"));
        Ok(())
    }

    #[test]
    fn test_dynamic_import_top_level() -> anyhow::Result<()> {
        let src = r#"
const mod = await import('./plugin');
"#;
        let result = parse_source(src, Language::JavaScript)?;
        assert!(
            result.imports.iter().any(|i| i.module == "./plugin"),
            "top-level dynamic import should be captured, got: {:?}",
            result.imports
        );
        Ok(())
    }

    #[test]
    fn test_export_re_export_extracted() -> anyhow::Result<()> {
        let src = r#"
export { Router } from './router';
export { default as Config } from './config';
export * from './utils';
"#;
        let result = parse_source(src, Language::TypeScript)?;
        // Re-exports should be captured as imports (tracking the source)
        let router_import = result
            .imports
            .iter()
            .find(|i| i.module == "./router")
            .unwrap();
        assert!(
            router_import.names.as_ref().unwrap().contains("Router"),
            "expected Router in names, got: {:?}",
            router_import.names
        );
        let config_import = result
            .imports
            .iter()
            .find(|i| i.module == "./config")
            .unwrap();
        assert!(
            config_import.names.as_ref().unwrap().contains("Config"),
            "expected Config in names, got: {:?}",
            config_import.names
        );
        assert!(result.imports.iter().any(|i| i.module == "./utils"));
        Ok(())
    }
}
