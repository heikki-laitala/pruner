"""Query analysis: given a natural language ask, infer relevant code areas."""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path

from .db import IndexDB


@dataclass
class QueryResult:
    """Result of analyzing a query against the index."""
    ask: str
    keywords: list[str] = field(default_factory=list)
    matching_files: list[dict] = field(default_factory=list)
    matching_symbols: list[dict] = field(default_factory=list)
    related_tests: list[dict] = field(default_factory=list)
    execution_paths: list[list[dict]] = field(default_factory=list)
    subsystems: list[str] = field(default_factory=list)

    @property
    def all_relevant_file_ids(self) -> set[int]:
        ids = set()
        for f in self.matching_files:
            ids.add(f["id"])
        for s in self.matching_symbols:
            ids.add(s["file_id"])
        for t in self.related_tests:
            ids.add(t["id"])
        for path in self.execution_paths:
            for step in path:
                if "file_id" in step:
                    ids.add(step["file_id"])
        return ids


def analyze_query(ask: str, db: IndexDB) -> QueryResult:
    """Analyze a natural language ask and find relevant code."""
    result = QueryResult(ask=ask)

    # Extract keywords from the ask
    result.keywords = _extract_keywords(ask)

    # Search files and symbols by keywords
    seen_file_ids = set()
    seen_symbol_ids = set()

    for kw in result.keywords:
        for f in db.search_files(kw):
            if f["id"] not in seen_file_ids:
                seen_file_ids.add(f["id"])
                result.matching_files.append(f)

        for s in db.search_symbols(kw):
            if s["id"] not in seen_symbol_ids:
                seen_symbol_ids.add(s["id"])
                result.matching_symbols.append(s)

    # Find related tests
    test_file_ids = set()
    for f in result.matching_files:
        edges = db.get_edges(kind="tests", target_file_id=f["id"])
        for e in edges:
            if e["source_file_id"] and e["source_file_id"] not in test_file_ids:
                test_file_ids.add(e["source_file_id"])
                tf = db.get_file_by_id(e["source_file_id"])
                if tf:
                    result.related_tests.append(tf)

    # Also check if any matching symbols have tests
    for s in result.matching_symbols:
        f = db.get_file_by_id(s["file_id"])
        if f:
            edges = db.get_edges(kind="tests", target_file_id=f["id"])
            for e in edges:
                if e["source_file_id"] and e["source_file_id"] not in test_file_ids:
                    test_file_ids.add(e["source_file_id"])
                    tf = db.get_file_by_id(e["source_file_id"])
                    if tf:
                        result.related_tests.append(tf)

    # Build execution paths from matching symbols
    for s in result.matching_symbols:
        path = _trace_execution_path(s, db)
        if path:
            result.execution_paths.append(path)

    # Infer subsystems from file paths
    result.subsystems = _infer_subsystems(result.matching_files + [
        db.get_file_by_id(s["file_id"]) for s in result.matching_symbols
        if db.get_file_by_id(s["file_id"])
    ])

    return result


def _extract_keywords(ask: str) -> list[str]:
    """Extract searchable keywords from a natural language ask."""
    # Remove common stop words and question words
    stop_words = {
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would", "could",
        "should", "may", "might", "can", "shall", "to", "of", "in", "for",
        "on", "with", "at", "by", "from", "as", "into", "through", "during",
        "before", "after", "above", "below", "between", "out", "off", "over",
        "under", "again", "further", "then", "once", "here", "there", "when",
        "where", "why", "how", "all", "both", "each", "few", "more", "most",
        "other", "some", "such", "no", "nor", "not", "only", "own", "same",
        "so", "than", "too", "very", "just", "because", "but", "and", "or",
        "if", "while", "that", "this", "what", "which", "who", "whom",
        "it", "its", "i", "me", "my", "we", "our", "you", "your",
        "he", "him", "his", "she", "her", "they", "them", "their",
        "need", "needs", "needed", "want", "wants", "change", "changes",
        "changed", "make", "makes", "made", "get", "gets", "got",
    }

    # Split on non-alphanumeric, keep underscores and hyphens
    words = re.findall(r'[a-zA-Z_][a-zA-Z0-9_-]*', ask)
    keywords = []

    for word in words:
        lower = word.lower()
        if lower not in stop_words and len(lower) > 1:
            keywords.append(lower)

        # Also try splitting camelCase and snake_case
        parts = re.findall(r'[A-Z]?[a-z]+|[A-Z]+(?=[A-Z][a-z]|\d|\b)', word)
        for p in parts:
            pl = p.lower()
            if pl not in stop_words and pl not in keywords and len(pl) > 2:
                keywords.append(pl)

    # Also extract any quoted strings
    quoted = re.findall(r'"([^"]+)"|\'([^\']+)\'', ask)
    for q in quoted:
        for part in q:
            if part:
                keywords.append(part.lower())

    return list(dict.fromkeys(keywords))  # dedupe preserving order


def _trace_execution_path(symbol: dict, db: IndexDB, depth: int = 0, max_depth: int = 5) -> list[dict]:
    """Trace an execution path from a symbol through its callees."""
    if depth >= max_depth:
        return []

    path = [{
        "symbol_id": symbol["id"],
        "name": symbol["name"],
        "kind": symbol["kind"],
        "file_id": symbol["file_id"],
        "line_start": symbol["line_start"],
        "depth": depth,
    }]

    # Get calls made by this symbol
    calls = db.get_calls_for_symbol(symbol["id"])
    for call in calls[:3]:  # Limit breadth
        # Try to resolve the callee
        callees = db.get_symbol_by_name(call["callee_name"])
        if callees:
            sub_path = _trace_execution_path(callees[0], db, depth + 1, max_depth)
            path.extend(sub_path)

    return path


def _infer_subsystems(files: list[dict]) -> list[str]:
    """Infer subsystem names from file paths."""
    subsystems = set()
    for f in files:
        if not f:
            continue
        parts = Path(f["path"]).parts
        # Use the first meaningful directory as subsystem
        for part in parts:
            if part not in ("src", "lib", "app", "pkg", "internal", "cmd", ".", ".."):
                subsystems.add(part)
                break
    return sorted(subsystems)
