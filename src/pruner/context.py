"""Context generation: produce compact LLM-ready context packages."""

from __future__ import annotations

import json
from pathlib import Path

from .db import IndexDB
from .query import QueryResult


def generate_context(query_result: QueryResult, db: IndexDB, repo_path: str | Path,
                     max_snippet_lines: int = 50, brief: bool = False) -> dict:
    """Generate a context package from a query result.

    If brief=True, caps output for LLM consumption: max 10 key files, 20 symbols,
    5 execution paths, 15 snippets (10 lines each), and omits per-file symbol lists.
    """
    repo = Path(repo_path).resolve()

    # Brief mode limits
    max_files = 10 if brief else 999
    max_symbols = 20 if brief else 999
    max_paths = 5 if brief else 999
    max_snippets = 15 if brief else 999
    if brief:
        max_snippet_lines = min(max_snippet_lines, 10)

    ctx = {
        "ask": query_result.ask,
        "keywords": query_result.keywords,
        "subsystems": query_result.subsystems,
        "execution_paths": [],
        "key_files": [],
        "key_symbols": [],
        "relevant_tests": [],
        "snippets": [],
    }

    # Execution paths (capped)
    for path in query_result.execution_paths[:max_paths]:
        ep = []
        for step in path:
            f = db.get_file_by_id(step["file_id"])
            ep.append({
                "symbol": step["name"],
                "kind": step["kind"],
                "file": f["path"] if f else "unknown",
                "line": step["line_start"],
                "depth": step["depth"],
            })
        if ep:
            ctx["execution_paths"].append(ep)

    # Key files (capped)
    seen_files = set()
    for f in query_result.matching_files:
        if len(ctx["key_files"]) >= max_files:
            break
        if f["id"] not in seen_files:
            seen_files.add(f["id"])
            imports = db.get_imports_for_file(f["id"])
            file_info = {
                "path": f["path"],
                "language": f["language"],
                "lines": f["line_count"],
                "is_test": bool(f["is_test"]),
                "imports": [imp["module"] for imp in imports],
            }
            if not brief:
                symbols = db.get_symbols_in_file(f["id"])
                file_info["symbols"] = [{"name": s["name"], "kind": s["kind"], "line": s["line_start"]} for s in symbols]
            ctx["key_files"].append(file_info)

    # Key symbols with snippets (capped)
    snippet_count = 0
    for s in query_result.matching_symbols[:max_symbols]:
        f = db.get_file_by_id(s["file_id"])
        if not f:
            continue

        sym_info = {
            "name": s["name"],
            "kind": s["kind"],
            "file": f["path"],
            "line_start": s["line_start"],
            "line_end": s["line_end"],
            "signature": s.get("signature", ""),
        }

        # Get calls from this symbol
        calls = db.get_calls_for_symbol(s["id"])
        sym_info["calls"] = [c["callee_name"] for c in calls[:10]]

        # Get callers of this symbol
        callers = db.get_callers_of(s["name"])
        sym_info["called_by"] = list(set(c["caller_name"] for c in callers))[:10]

        ctx["key_symbols"].append(sym_info)

        # Extract code snippet (capped)
        if snippet_count < max_snippets:
            snippet = _extract_snippet(repo, f["path"], s["line_start"], s["line_end"], max_snippet_lines)
            if snippet:
                ctx["snippets"].append({
                    "file": f["path"],
                    "symbol": s["name"],
                    "line_start": s["line_start"],
                    "line_end": min(s["line_end"], s["line_start"] + max_snippet_lines - 1),
                    "code": snippet,
                })
                snippet_count += 1

    # Relevant tests
    for t in query_result.related_tests:
        symbols = db.get_symbols_in_file(t["id"])
        ctx["relevant_tests"].append({
            "path": t["path"],
            "symbols": [{"name": s["name"], "kind": s["kind"]} for s in symbols],
        })

    return ctx


def format_context_text(ctx: dict) -> str:
    """Format context as human-readable text."""
    lines = []
    lines.append(f"# Context for: {ctx['ask']}")
    lines.append("")

    if ctx["keywords"]:
        lines.append(f"**Keywords:** {', '.join(ctx['keywords'])}")
        lines.append("")

    if ctx["subsystems"]:
        lines.append(f"**Subsystems:** {', '.join(ctx['subsystems'])}")
        lines.append("")

    if ctx["execution_paths"]:
        lines.append("## Execution Paths")
        lines.append("")
        for i, path in enumerate(ctx["execution_paths"], 1):
            lines.append(f"### Path {i}")
            for step in path:
                indent = "  " * step["depth"]
                lines.append(f"{indent}→ {step['symbol']} ({step['kind']}) in {step['file']}:{step['line']}")
            lines.append("")

    if ctx["key_files"]:
        lines.append("## Key Files")
        lines.append("")
        for f in ctx["key_files"]:
            test_marker = " [TEST]" if f["is_test"] else ""
            lines.append(f"### {f['path']}{test_marker}")
            lines.append(f"Language: {f['language'] or 'unknown'} | Lines: {f['lines']}")
            if f.get("symbols"):
                lines.append("Symbols:")
                for s in f["symbols"]:
                    lines.append(f"  - {s['name']} ({s['kind']}) L{s['line']}")
            if f["imports"]:
                lines.append(f"Imports: {', '.join(f['imports'])}")
            lines.append("")

    if ctx["key_symbols"]:
        lines.append("## Key Symbols")
        lines.append("")
        for s in ctx["key_symbols"]:
            lines.append(f"### {s['name']} ({s['kind']})")
            lines.append(f"File: {s['file']} L{s['line_start']}-{s['line_end']}")
            if s.get("signature"):
                lines.append(f"Signature: `{s['signature']}`")
            if s.get("calls"):
                lines.append(f"Calls: {', '.join(s['calls'][:10])}")
            if s.get("called_by"):
                lines.append(f"Called by: {', '.join(s['called_by'][:10])}")
            lines.append("")

    if ctx["relevant_tests"]:
        lines.append("## Relevant Tests")
        lines.append("")
        for t in ctx["relevant_tests"]:
            lines.append(f"### {t['path']}")
            for s in t["symbols"]:
                lines.append(f"  - {s['name']} ({s['kind']})")
            lines.append("")

    if ctx["snippets"]:
        lines.append("## Code Snippets")
        lines.append("")
        for snip in ctx["snippets"]:
            lines.append(f"### {snip['file']}:{snip['line_start']}-{snip['line_end']} ({snip['symbol']})")
            lines.append("```")
            lines.append(snip["code"])
            lines.append("```")
            lines.append("")

    return "\n".join(lines)


def format_context_summary(ctx: dict) -> str:
    """Format a compact summary — just file list, symbol list, and test list.

    Designed to be printed to stdout while the full context is written to a file.
    The LLM can then Grep/Read into the full file for details.
    """
    lines = []
    lines.append(f"# Context for: {ctx['ask']}")
    lines.append("")

    if ctx["keywords"]:
        lines.append(f"Keywords: {', '.join(ctx['keywords'])}")
    if ctx["subsystems"]:
        lines.append(f"Subsystems: {', '.join(ctx['subsystems'])}")
    lines.append("")

    if ctx["key_files"]:
        lines.append(f"## Key Files ({len(ctx['key_files'])})")
        for f in ctx["key_files"]:
            test_marker = " [TEST]" if f["is_test"] else ""
            lines.append(f"  {f['path']}{test_marker} ({f['language'] or '?'}, {f['lines']}L)")

    lines.append("")

    if ctx["key_symbols"]:
        lines.append(f"## Key Symbols ({len(ctx['key_symbols'])})")
        for s in ctx["key_symbols"]:
            lines.append(f"  {s['name']} ({s['kind']}) in {s['file']}:{s['line_start']}")

    lines.append("")

    if ctx["execution_paths"]:
        lines.append(f"## Execution Paths ({len(ctx['execution_paths'])})")
        for i, path in enumerate(ctx["execution_paths"], 1):
            chain = " → ".join(step["symbol"] for step in path)
            lines.append(f"  {i}. {chain}")

    lines.append("")

    if ctx["relevant_tests"]:
        lines.append(f"## Tests ({len(ctx['relevant_tests'])})")
        for t in ctx["relevant_tests"]:
            lines.append(f"  {t['path']}")

    return "\n".join(lines)


def format_context_json(ctx: dict) -> str:
    """Format context as JSON."""
    return json.dumps(ctx, indent=2)


def _extract_snippet(repo: Path, rel_path: str, line_start: int, line_end: int,
                     max_lines: int) -> str | None:
    """Extract a code snippet from a file."""
    fpath = repo / rel_path
    try:
        content = fpath.read_text(encoding="utf-8", errors="ignore")
    except (OSError, PermissionError):
        return None

    all_lines = content.splitlines()
    start = max(0, line_start - 1)
    end = min(len(all_lines), start + max_lines, line_end)
    snippet_lines = all_lines[start:end]

    if end < line_end:
        snippet_lines.append(f"  # ... ({line_end - end} more lines)")

    return "\n".join(snippet_lines)
