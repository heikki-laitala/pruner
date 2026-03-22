"""Tests for the parser module."""

from pruner.parser import parse_file


def test_parse_python_function():
    source = """
def hello(name):
    print(f"Hello {name}")

def goodbye():
    hello("world")
"""
    result = parse_file(source, "python")
    assert result is not None
    names = [s["name"] for s in result.symbols]
    assert "hello" in names
    assert "goodbye" in names

    # Check calls
    callers = {c["caller"] for c in result.calls}
    assert "goodbye" in callers
    callees = {c["callee"] for c in result.calls}
    assert "hello" in callees


def test_parse_python_class():
    source = """
class MyClass:
    def __init__(self):
        pass

    def method(self, x):
        return x * 2
"""
    result = parse_file(source, "python")
    assert result is not None
    names = [s["name"] for s in result.symbols]
    assert "MyClass" in names
    assert "__init__" in names
    assert "method" in names

    cls = next(s for s in result.symbols if s["name"] == "MyClass")
    assert cls["kind"] == "class"

    method = next(s for s in result.symbols if s["name"] == "method")
    assert method["kind"] == "method"
    assert method["parent"] == "MyClass"


def test_parse_python_imports():
    source = """
import os
import sys
from pathlib import Path
from collections import defaultdict, OrderedDict
"""
    result = parse_file(source, "python")
    assert result is not None
    modules = [i["module"] for i in result.imports]
    assert "os" in modules
    assert "sys" in modules
    assert "pathlib" in modules
    assert "collections" in modules


def test_parse_javascript_function():
    source = """
function greet(name) {
    console.log("Hello " + name);
}

const add = (a, b) => a + b;
"""
    result = parse_file(source, "javascript")
    assert result is not None
    names = [s["name"] for s in result.symbols]
    assert "greet" in names
    assert "add" in names


def test_parse_javascript_class():
    source = """
class Animal {
    constructor(name) {
        this.name = name;
    }

    speak() {
        return this.name;
    }
}
"""
    result = parse_file(source, "javascript")
    assert result is not None
    names = [s["name"] for s in result.symbols]
    assert "Animal" in names
    assert "constructor" in names
    assert "speak" in names


def test_parse_javascript_imports():
    source = """
import React from 'react';
import { useState, useEffect } from 'react';
import * as path from 'path';
"""
    result = parse_file(source, "javascript")
    assert result is not None
    modules = [i["module"] for i in result.imports]
    assert "react" in modules
    assert "path" in modules


def test_parse_unsupported_language():
    result = parse_file("some code", "go")
    assert result is None


def test_parse_typescript():
    source = """
function fetchData(url: string): Promise<Response> {
    return fetch(url);
}

class ApiClient {
    baseUrl: string;

    constructor(baseUrl: string) {
        this.baseUrl = baseUrl;
    }

    async get(path: string) {
        return fetchData(this.baseUrl + path);
    }
}
"""
    result = parse_file(source, "typescript")
    assert result is not None
    names = [s["name"] for s in result.symbols]
    assert "fetchData" in names
    assert "ApiClient" in names


def test_parse_rust_functions():
    source = """
pub fn process(data: &str) -> Result<String, Error> {
    let result = transform(data);
    Ok(result)
}

fn transform(input: &str) -> String {
    input.to_uppercase()
}
"""
    result = parse_file(source, "rust")
    assert result is not None
    names = [s["name"] for s in result.symbols]
    assert "process" in names
    assert "transform" in names

    process = next(s for s in result.symbols if s["name"] == "process")
    assert process["kind"] == "function"
    assert "pub " in process["signature"]

    transform = next(s for s in result.symbols if s["name"] == "transform")
    assert "pub " not in transform["signature"]

    # Check calls
    callees = {c["callee"] for c in result.calls}
    assert "transform" in callees


def test_parse_rust_struct_and_impl():
    source = """
pub struct Config {
    pub name: String,
    count: usize,
}

impl Config {
    pub fn new(name: String) -> Self {
        Self { name, count: 0 }
    }

    fn increment(&mut self) {
        self.count += 1;
        self.validate();
    }

    fn validate(&self) -> bool {
        self.count > 0
    }
}
"""
    result = parse_file(source, "rust")
    assert result is not None
    names = [s["name"] for s in result.symbols]
    assert "Config" in names
    assert "new" in names
    assert "increment" in names
    assert "validate" in names

    config = next(s for s in result.symbols if s["name"] == "Config")
    assert config["kind"] == "struct"

    new = next(s for s in result.symbols if s["name"] == "new")
    assert new["kind"] == "method"
    assert new["parent"] == "Config"

    # increment calls validate
    callees = {c["callee"] for c in result.calls if c["caller"] == "increment"}
    assert "validate" in callees


def test_parse_rust_enum():
    source = """
pub enum Status {
    Active,
    Inactive(String),
}
"""
    result = parse_file(source, "rust")
    assert result is not None
    names = [s["name"] for s in result.symbols]
    assert "Status" in names

    status = next(s for s in result.symbols if s["name"] == "Status")
    assert status["kind"] == "enum"
    assert "pub " in status["signature"]


def test_parse_rust_trait():
    source = """
pub trait Handler {
    fn handle(&self, input: &str) -> String;
    fn name(&self) -> &str;
}
"""
    result = parse_file(source, "rust")
    assert result is not None
    names = [s["name"] for s in result.symbols]
    assert "Handler" in names

    handler = next(s for s in result.symbols if s["name"] == "Handler")
    assert handler["kind"] == "trait"


def test_parse_rust_imports():
    source = """
use std::collections::HashMap;
use crate::db::IndexDB;
use std::io::{Read, Write};
mod utils;
"""
    result = parse_file(source, "rust")
    assert result is not None
    modules = [i["module"] for i in result.imports]
    assert "std::collections" in modules
    assert "crate::db" in modules
    assert "std::io" in modules
    assert "utils" in modules

    # Check named imports
    hashmap_imp = next(i for i in result.imports if i["module"] == "std::collections")
    assert hashmap_imp["names"] == "HashMap"

    io_imp = next(i for i in result.imports if i["module"] == "std::io")
    assert "Read" in io_imp["names"]
    assert "Write" in io_imp["names"]


def test_parse_rust_macro_calls():
    source = """
fn main() {
    println!("hello");
    let v = vec![1, 2, 3];
}
"""
    result = parse_file(source, "rust")
    assert result is not None
    callees = {c["callee"] for c in result.calls}
    assert "println" in callees
    assert "vec" in callees


def test_parse_rust_method_calls():
    source = """
fn process(data: &str) -> String {
    data.to_string().to_uppercase()
}
"""
    result = parse_file(source, "rust")
    assert result is not None
    callees = {c["callee"] for c in result.calls}
    assert "to_string" in callees
    assert "to_uppercase" in callees
