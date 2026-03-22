"""Tree-sitter based code parsing for symbol extraction."""

from __future__ import annotations

import tree_sitter_python as tspython
import tree_sitter_javascript as tsjavascript
import tree_sitter_typescript as tstypescript
import tree_sitter_rust as tsrust
from tree_sitter import Language, Parser, Node

PY_LANGUAGE = Language(tspython.language())
JS_LANGUAGE = Language(tsjavascript.language())
TS_LANGUAGE = Language(tstypescript.language_typescript())
TSX_LANGUAGE = Language(tstypescript.language_tsx())
RUST_LANGUAGE = Language(tsrust.language())

LANGUAGES = {
    "python": PY_LANGUAGE,
    "javascript": JS_LANGUAGE,
    "typescript": TS_LANGUAGE,
    "tsx": TSX_LANGUAGE,
    "rust": RUST_LANGUAGE,
}


def get_parser(language: str) -> Parser | None:
    lang = LANGUAGES.get(language)
    if lang is None:
        return None
    parser = Parser(lang)
    return parser


class ParseResult:
    """Holds extracted symbols, imports, and calls from a single file."""

    def __init__(self):
        self.symbols: list[dict] = []  # {name, kind, line_start, line_end, parent, signature}
        self.imports: list[dict] = []  # {module, names}
        self.calls: list[dict] = []    # {caller, callee, line}


def parse_file(source: str, language: str) -> ParseResult | None:
    parser = get_parser(language)
    if parser is None:
        return None

    tree = parser.parse(source.encode("utf-8"))
    result = ParseResult()

    if language == "python":
        _extract_python(tree.root_node, result, source)
    elif language in ("javascript", "typescript", "tsx"):
        _extract_js_ts(tree.root_node, result, source, language)
    elif language == "rust":
        _extract_rust(tree.root_node, result, source)

    return result


def _node_text(node: Node, source: str) -> str:
    return source[node.start_byte:node.end_byte]


def _extract_python(root: Node, result: ParseResult, source: str):
    """Extract symbols, imports, and calls from Python AST."""
    _walk_python(root, result, source, parent_name=None)


def _walk_python(node: Node, result: ParseResult, source: str, parent_name: str | None):
    for child in node.children:
        if child.type == "function_definition":
            name_node = child.child_by_field_name("name")
            if name_node:
                name = _node_text(name_node, source)
                params_node = child.child_by_field_name("parameters")
                sig = _node_text(params_node, source) if params_node else ""
                kind = "method" if parent_name else "function"
                result.symbols.append({
                    "name": name,
                    "kind": kind,
                    "line_start": child.start_point[0] + 1,
                    "line_end": child.end_point[0] + 1,
                    "parent": parent_name,
                    "signature": f"def {name}{sig}",
                })
                # Extract calls within this function
                _extract_calls_python(child, result, source, name)

        elif child.type == "class_definition":
            name_node = child.child_by_field_name("name")
            if name_node:
                name = _node_text(name_node, source)
                result.symbols.append({
                    "name": name,
                    "kind": "class",
                    "line_start": child.start_point[0] + 1,
                    "line_end": child.end_point[0] + 1,
                    "parent": parent_name,
                    "signature": f"class {name}",
                })
                _walk_python(child, result, source, parent_name=name)
                continue  # already walked children

        elif child.type == "import_statement":
            # import foo, bar
            for mod_child in child.children:
                if mod_child.type == "dotted_name":
                    result.imports.append({"module": _node_text(mod_child, source), "names": None})
                elif mod_child.type == "aliased_import":
                    name_n = mod_child.child_by_field_name("name")
                    if name_n:
                        result.imports.append({"module": _node_text(name_n, source), "names": None})

        elif child.type == "import_from_statement":
            module_node = child.child_by_field_name("module_name")
            if module_node:
                module = _node_text(module_node, source)
                names = []
                for imp_child in child.children:
                    if imp_child.type == "dotted_name" and imp_child != module_node:
                        names.append(_node_text(imp_child, source))
                    elif imp_child.type == "aliased_import":
                        name_n = imp_child.child_by_field_name("name")
                        if name_n:
                            names.append(_node_text(name_n, source))
                result.imports.append({"module": module, "names": ",".join(names) if names else None})

        _walk_python(child, result, source, parent_name)


def _extract_calls_python(node: Node, result: ParseResult, source: str, caller_name: str):
    """Extract function/method calls within a node."""
    if node.type == "call":
        func_node = node.child_by_field_name("function")
        if func_node:
            callee = _node_text(func_node, source)
            # Simplify attribute calls: obj.method -> method
            if "." in callee:
                callee = callee.rsplit(".", 1)[-1]
            result.calls.append({
                "caller": caller_name,
                "callee": callee,
                "line": node.start_point[0] + 1,
            })
    for child in node.children:
        _extract_calls_python(child, result, source, caller_name)


def _extract_js_ts(root: Node, result: ParseResult, source: str, language: str):
    """Extract symbols, imports, and calls from JS/TS AST."""
    _walk_js_ts(root, result, source, parent_name=None)


def _walk_js_ts(node: Node, result: ParseResult, source: str, parent_name: str | None):
    for child in node.children:
        if child.type in ("function_declaration", "generator_function_declaration"):
            name_node = child.child_by_field_name("name")
            if name_node:
                name = _node_text(name_node, source)
                params_node = child.child_by_field_name("parameters")
                sig = _node_text(params_node, source) if params_node else ""
                result.symbols.append({
                    "name": name,
                    "kind": "function",
                    "line_start": child.start_point[0] + 1,
                    "line_end": child.end_point[0] + 1,
                    "parent": parent_name,
                    "signature": f"function {name}{sig}",
                })
                _extract_calls_js(child, result, source, name)

        elif child.type == "class_declaration":
            name_node = child.child_by_field_name("name")
            if name_node:
                name = _node_text(name_node, source)
                result.symbols.append({
                    "name": name,
                    "kind": "class",
                    "line_start": child.start_point[0] + 1,
                    "line_end": child.end_point[0] + 1,
                    "parent": parent_name,
                    "signature": f"class {name}",
                })
                _walk_js_ts(child, result, source, parent_name=name)
                continue

        elif child.type in ("method_definition",):
            name_node = child.child_by_field_name("name")
            if name_node:
                name = _node_text(name_node, source)
                params_node = child.child_by_field_name("parameters")
                sig = _node_text(params_node, source) if params_node else ""
                result.symbols.append({
                    "name": name,
                    "kind": "method",
                    "line_start": child.start_point[0] + 1,
                    "line_end": child.end_point[0] + 1,
                    "parent": parent_name,
                    "signature": f"{name}{sig}",
                })
                _extract_calls_js(child, result, source, name)

        elif child.type == "lexical_declaration" or child.type == "variable_declaration":
            # const foo = () => {} or const foo = function() {}
            for declarator in child.children:
                if declarator.type == "variable_declarator":
                    name_node = declarator.child_by_field_name("name")
                    value_node = declarator.child_by_field_name("value")
                    if name_node and value_node and value_node.type in ("arrow_function", "function_expression"):
                        name = _node_text(name_node, source)
                        params_node = value_node.child_by_field_name("parameters")
                        sig = _node_text(params_node, source) if params_node else ""
                        result.symbols.append({
                            "name": name,
                            "kind": "function",
                            "line_start": child.start_point[0] + 1,
                            "line_end": child.end_point[0] + 1,
                            "parent": parent_name,
                            "signature": f"const {name} = {sig} =>",
                        })
                        _extract_calls_js(value_node, result, source, name)

        elif child.type == "import_statement":
            _extract_js_import(child, result, source)

        elif child.type == "export_statement":
            # Walk into exports to find declarations
            _walk_js_ts(child, result, source, parent_name)
            continue

        _walk_js_ts(child, result, source, parent_name)


def _extract_js_import(node: Node, result: ParseResult, source: str):
    """Extract import statements from JS/TS."""
    source_node = node.child_by_field_name("source")
    if source_node:
        module = _node_text(source_node, source).strip("'\"")
        names = []
        for child in node.children:
            if child.type == "import_clause":
                for sub in child.children:
                    if sub.type == "identifier":
                        names.append(_node_text(sub, source))
                    elif sub.type == "named_imports":
                        for spec in sub.children:
                            if spec.type == "import_specifier":
                                name_n = spec.child_by_field_name("name")
                                if name_n:
                                    names.append(_node_text(name_n, source))
                    elif sub.type == "namespace_import":
                        for ns_child in sub.children:
                            if ns_child.type == "identifier":
                                names.append(f"* as {_node_text(ns_child, source)}")
        result.imports.append({"module": module, "names": ",".join(names) if names else None})


def _extract_calls_js(node: Node, result: ParseResult, source: str, caller_name: str):
    """Extract function/method calls within a JS/TS node."""
    if node.type == "call_expression":
        func_node = node.child_by_field_name("function")
        if func_node:
            callee = _node_text(func_node, source)
            if "." in callee:
                callee = callee.rsplit(".", 1)[-1]
            result.calls.append({
                "caller": caller_name,
                "callee": callee,
                "line": node.start_point[0] + 1,
            })
    for child in node.children:
        _extract_calls_js(child, result, source, caller_name)


# --- Rust ---


def _extract_rust(root: Node, result: ParseResult, source: str):
    """Extract symbols, imports, and calls from Rust AST."""
    _walk_rust(root, result, source, parent_name=None)


def _walk_rust(node: Node, result: ParseResult, source: str, parent_name: str | None):
    for child in node.children:
        if child.type == "function_item":
            name_node = child.child_by_field_name("name")
            if name_node:
                name = _node_text(name_node, source)
                params_node = child.child_by_field_name("parameters")
                sig = _node_text(params_node, source) if params_node else "()"
                kind = "method" if parent_name else "function"
                # Check for pub visibility
                vis = "pub " if any(c.type == "visibility_modifier" for c in child.children) else ""
                result.symbols.append({
                    "name": name,
                    "kind": kind,
                    "line_start": child.start_point[0] + 1,
                    "line_end": child.end_point[0] + 1,
                    "parent": parent_name,
                    "signature": f"{vis}fn {name}{sig}",
                })
                _extract_calls_rust(child, result, source, name)

        elif child.type == "struct_item":
            name_node = child.child_by_field_name("name")
            if name_node:
                name = _node_text(name_node, source)
                vis = "pub " if any(c.type == "visibility_modifier" for c in child.children) else ""
                result.symbols.append({
                    "name": name,
                    "kind": "struct",
                    "line_start": child.start_point[0] + 1,
                    "line_end": child.end_point[0] + 1,
                    "parent": parent_name,
                    "signature": f"{vis}struct {name}",
                })

        elif child.type == "enum_item":
            name_node = child.child_by_field_name("name")
            if name_node:
                name = _node_text(name_node, source)
                vis = "pub " if any(c.type == "visibility_modifier" for c in child.children) else ""
                result.symbols.append({
                    "name": name,
                    "kind": "enum",
                    "line_start": child.start_point[0] + 1,
                    "line_end": child.end_point[0] + 1,
                    "parent": parent_name,
                    "signature": f"{vis}enum {name}",
                })

        elif child.type == "trait_item":
            name_node = child.child_by_field_name("name")
            if name_node:
                name = _node_text(name_node, source)
                vis = "pub " if any(c.type == "visibility_modifier" for c in child.children) else ""
                result.symbols.append({
                    "name": name,
                    "kind": "trait",
                    "line_start": child.start_point[0] + 1,
                    "line_end": child.end_point[0] + 1,
                    "parent": parent_name,
                    "signature": f"{vis}trait {name}",
                })
                # Walk into trait body for method signatures
                body = child.child_by_field_name("body")
                if body:
                    _walk_rust(body, result, source, parent_name=name)
                continue

        elif child.type == "impl_item":
            # impl Type { ... } or impl Trait for Type { ... }
            type_node = child.child_by_field_name("type")
            impl_name = _node_text(type_node, source) if type_node else None
            # Check for trait impl: impl Trait for Type
            trait_node = child.child_by_field_name("trait")
            if trait_node and impl_name:
                impl_name = f"{_node_text(trait_node, source)} for {impl_name}"
            body = child.child_by_field_name("body")
            if body:
                _walk_rust(body, result, source, parent_name=impl_name or parent_name)
            continue

        elif child.type == "use_declaration":
            _extract_rust_use(child, result, source)

        elif child.type == "mod_item":
            name_node = child.child_by_field_name("name")
            if name_node:
                name = _node_text(name_node, source)
                result.imports.append({"module": name, "names": None})

        # Walk into declaration_list (inside impl/trait blocks)
        elif child.type == "declaration_list":
            _walk_rust(child, result, source, parent_name)
            continue

        _walk_rust(child, result, source, parent_name)


def _extract_rust_use(node: Node, result: ParseResult, source: str):
    """Extract use declarations from Rust."""
    arg = node.child_by_field_name("argument")
    if not arg:
        return

    text = _node_text(arg, source)

    if arg.type == "scoped_identifier":
        # use std::collections::HashMap;
        path_node = arg.child_by_field_name("path")
        name_node = arg.child_by_field_name("name")
        if path_node:
            module = _node_text(path_node, source)
            name = _node_text(name_node, source) if name_node else None
            result.imports.append({"module": module, "names": name})
    elif arg.type == "scoped_use_list":
        # use std::collections::{HashMap, BTreeMap};
        path_node = arg.child_by_field_name("path")
        module = _node_text(path_node, source) if path_node else ""
        list_node = arg.child_by_field_name("list")
        names = []
        if list_node:
            for child in list_node.children:
                if child.type in ("identifier", "scoped_identifier", "type_identifier"):
                    names.append(_node_text(child, source))
                elif child.type == "use_as_clause":
                    name_n = child.child_by_field_name("path") or child.children[0]
                    names.append(_node_text(name_n, source))
        result.imports.append({"module": module, "names": ",".join(names) if names else None})
    elif arg.type == "identifier":
        # use something;
        result.imports.append({"module": text, "names": None})
    elif arg.type == "use_wildcard":
        # use something::*;
        path_node = arg.child_by_field_name("path") or arg.children[0] if arg.children else None
        module = _node_text(path_node, source) if path_node else text
        result.imports.append({"module": module, "names": "*"})
    else:
        # Fallback: store the whole use path
        result.imports.append({"module": text, "names": None})


def _extract_calls_rust(node: Node, result: ParseResult, source: str, caller_name: str):
    """Extract function/method calls within a Rust node."""
    if node.type == "call_expression":
        func_node = node.child_by_field_name("function")
        if func_node:
            callee = _node_text(func_node, source)
            # Simplify: path::to::func -> func, obj.method -> method
            if "::" in callee:
                callee = callee.rsplit("::", 1)[-1]
            if "." in callee:
                callee = callee.rsplit(".", 1)[-1]
            result.calls.append({
                "caller": caller_name,
                "callee": callee,
                "line": node.start_point[0] + 1,
            })
    elif node.type == "method_call_expression":
        # obj.method(args)
        name_node = node.child_by_field_name("name")
        if name_node:
            callee = _node_text(name_node, source)
            result.calls.append({
                "caller": caller_name,
                "callee": callee,
                "line": node.start_point[0] + 1,
            })
    # Also catch macro invocations like println!(), vec![]
    elif node.type == "macro_invocation":
        macro_node = node.child_by_field_name("macro")
        if macro_node:
            callee = _node_text(macro_node, source).rstrip("!")
            result.calls.append({
                "caller": caller_name,
                "callee": callee,
                "line": node.start_point[0] + 1,
            })
    for child in node.children:
        _extract_calls_rust(child, result, source, caller_name)
