"""Repository indexing: walks files, parses them, and populates the DB."""

from __future__ import annotations

import os
from pathlib import Path

from .db import IndexDB
from .languages import detect_language, is_test_file, should_ignore
from .parser import parse_file


def index_repo(repo_path: str | Path, db: IndexDB, verbose: bool = False) -> dict:
    """Index a repository into the database. Returns stats."""
    repo = Path(repo_path).resolve()
    if not repo.is_dir():
        raise ValueError(f"Not a directory: {repo}")

    db.clear()
    stats = {"files": 0, "symbols": 0, "imports": 0, "calls": 0, "edges": 0, "skipped": 0}

    # Symbol name -> list of symbol IDs (for resolving calls)
    symbol_map: dict[str, list[int]] = {}
    # Pending calls to resolve after all symbols are indexed
    pending_calls: list[tuple[int, str, int | None]] = []  # (caller_symbol_id, callee_name, line)

    for dirpath, dirnames, filenames in os.walk(repo):
        # Filter ignored directories in-place
        dirnames[:] = [d for d in dirnames if not should_ignore(Path(d))]

        for fname in filenames:
            fpath = Path(dirpath) / fname
            rel_path = str(fpath.relative_to(repo))

            if should_ignore(fpath):
                stats["skipped"] += 1
                continue

            language = detect_language(fpath)
            is_test = is_test_file(fpath)

            try:
                content = fpath.read_text(encoding="utf-8", errors="ignore")
            except (OSError, PermissionError):
                stats["skipped"] += 1
                continue

            line_count = content.count("\n") + 1
            size = fpath.stat().st_size

            file_id = db.insert_file(rel_path, language, size, line_count, is_test)
            stats["files"] += 1

            if verbose:
                print(f"  indexed: {rel_path} ({language or 'unknown'})")

            # Parse with tree-sitter if we support the language
            if language:
                parse_result = parse_file(content, language)
                if parse_result:
                    # Map from local symbol name to symbol_id for this file
                    file_symbol_ids: dict[str, int] = {}

                    for sym in parse_result.symbols:
                        parent_id = file_symbol_ids.get(sym["parent"]) if sym["parent"] else None
                        sym_id = db.insert_symbol(
                            file_id, sym["name"], sym["kind"],
                            sym["line_start"], sym["line_end"],
                            parent_symbol_id=parent_id,
                            signature=sym["signature"],
                        )
                        file_symbol_ids[sym["name"]] = sym_id
                        symbol_map.setdefault(sym["name"], []).append(sym_id)
                        stats["symbols"] += 1

                        # Edge: file contains symbol
                        db.insert_edge("contains", source_file_id=file_id, target_symbol_id=sym_id)
                        stats["edges"] += 1

                    for imp in parse_result.imports:
                        db.insert_import(file_id, imp["module"], imp["names"])
                        stats["imports"] += 1

                        # Edge: file imports module
                        db.insert_edge("imports", source_file_id=file_id, target_name=imp["module"])
                        stats["edges"] += 1

                    for call in parse_result.calls:
                        caller_id = file_symbol_ids.get(call["caller"])
                        if caller_id:
                            pending_calls.append((caller_id, call["callee"], call["line"]))

    # Resolve calls
    for caller_id, callee_name, line in pending_calls:
        db.insert_call(caller_id, callee_name, line)
        stats["calls"] += 1

        # Try to resolve callee to a symbol
        targets = symbol_map.get(callee_name, [])
        if targets:
            for target_id in targets[:1]:  # Link to first match
                db.insert_edge("calls", source_symbol_id=caller_id, target_symbol_id=target_id)
                stats["edges"] += 1
        else:
            db.insert_edge("calls", source_symbol_id=caller_id, target_name=callee_name)
            stats["edges"] += 1

    # Build test edges: link test files to the files/symbols they likely test
    _build_test_edges(db, stats)

    return stats


def _build_test_edges(db: IndexDB, stats: dict):
    """Heuristically link test files to the modules they test."""
    test_files = db.get_test_files()
    all_files = {f["path"]: f for f in db.get_all_files()}

    for tf in test_files:
        tf_path = tf["path"]
        # Check imports in test file for references to project modules
        imports = db.get_imports_for_file(tf["id"])
        for imp in imports:
            module = imp["module"]
            # Try to find a matching source file
            candidates = _module_to_file_candidates(module)
            for cand in candidates:
                for path, f in all_files.items():
                    if path.endswith(cand) and not f["is_test"]:
                        db.insert_edge("tests", source_file_id=tf["id"], target_file_id=f["id"])
                        stats["edges"] += 1
                        break

        # Name-based heuristic: test_foo.py -> foo.py
        base = Path(tf_path).stem
        for prefix in ("test_", "test"):
            if base.startswith(prefix):
                target_stem = base[len(prefix):]
                for path, f in all_files.items():
                    if Path(path).stem == target_stem and not f["is_test"]:
                        db.insert_edge("tests", source_file_id=tf["id"], target_file_id=f["id"])
                        stats["edges"] += 1
                        break


def _module_to_file_candidates(module: str) -> list[str]:
    """Convert a module name to possible file path suffixes."""
    parts = module.replace(".", "/")
    return [
        f"{parts}.py",
        f"{parts}/index.py",
        f"{parts}.js",
        f"{parts}.ts",
        f"{parts}/index.js",
        f"{parts}/index.ts",
    ]
