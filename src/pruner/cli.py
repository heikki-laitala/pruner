"""CLI interface for pruner."""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

import click

from .context import format_context_json, format_context_summary, format_context_text, generate_context
from .db import IndexDB
from .indexer import index_repo
from .query import analyze_query
from .tokens import estimate_claude_session, measure as measure_tokens

INDEX_DIR = ".pruner"
DB_NAME = "index.db"


def _get_db(repo: str) -> IndexDB:
    repo_path = Path(repo).resolve()
    db_path = repo_path / INDEX_DIR / DB_NAME
    if not db_path.exists():
        click.echo(f"No index found at {db_path}. Run 'pruner index {repo}' first.", err=True)
        sys.exit(1)
    return IndexDB(db_path)


def _ensure_index_dir(repo: str) -> Path:
    repo_path = Path(repo).resolve()
    index_dir = repo_path / INDEX_DIR
    index_dir.mkdir(exist_ok=True)
    return index_dir


@click.group()
@click.version_option(package_name="pruner")
def cli():
    """Pruner: synthetic code context engine for LLM coding tasks."""
    pass


@cli.command()
@click.argument("repo", default=".")
@click.option("-v", "--verbose", is_flag=True, help="Show each file as it's indexed")
def index(repo: str, verbose: bool):
    """Index a repository for querying."""
    repo_path = Path(repo).resolve()
    if not repo_path.is_dir():
        click.echo(f"Not a directory: {repo_path}", err=True)
        sys.exit(1)

    index_dir = _ensure_index_dir(repo)
    db_path = index_dir / DB_NAME
    db = IndexDB(db_path)

    click.echo(f"Indexing {repo_path}...")
    stats = index_repo(repo_path, db, verbose=verbose)
    db.close()

    click.echo(f"Done. Indexed {stats['files']} files, {stats['symbols']} symbols, "
               f"{stats['imports']} imports, {stats['calls']} calls, {stats['edges']} edges. "
               f"({stats['skipped']} files skipped)")


@cli.command()
@click.argument("repo", default=".")
@click.argument("ask")
@click.option("--json-output", "json_out", is_flag=True, help="Output as JSON")
def query(repo: str, ask: str, json_out: bool):
    """Query the index with a natural language ask."""
    db = _get_db(repo)
    result = analyze_query(ask, db)

    if json_out:
        output = {
            "ask": result.ask,
            "keywords": result.keywords,
            "subsystems": result.subsystems,
            "matching_files": [f["path"] for f in result.matching_files],
            "matching_symbols": [{"name": s["name"], "kind": s["kind"], "file_id": s["file_id"]}
                                  for s in result.matching_symbols],
            "related_tests": [t["path"] for t in result.related_tests],
            "execution_paths_count": len(result.execution_paths),
        }
        click.echo(json.dumps(output, indent=2))
    else:
        click.echo(f"Ask: {result.ask}")
        click.echo(f"Keywords: {', '.join(result.keywords)}")
        click.echo(f"Subsystems: {', '.join(result.subsystems)}")
        click.echo(f"\nMatching files ({len(result.matching_files)}):")
        for f in result.matching_files:
            click.echo(f"  {f['path']}")
        click.echo(f"\nMatching symbols ({len(result.matching_symbols)}):")
        for s in result.matching_symbols:
            click.echo(f"  {s['name']} ({s['kind']})")
        click.echo(f"\nRelated tests ({len(result.related_tests)}):")
        for t in result.related_tests:
            click.echo(f"  {t['path']}")
        click.echo(f"\nExecution paths: {len(result.execution_paths)}")

    db.close()


@cli.command()
@click.argument("repo", default=".")
@click.argument("ask")
@click.option("--format", "fmt", type=click.Choice(["text", "json", "both"]), default="text",
              help="Output format")
@click.option("--max-snippet-lines", default=50, help="Max lines per code snippet")
@click.option("--brief", is_flag=True, help="Compact output: fewer files, shorter snippets, no per-file symbol lists")
@click.option("-o", "--output", "output_path", help="Write output to file")
def context(repo: str, ask: str, fmt: str, max_snippet_lines: int, brief: bool, output_path: str | None):
    """Generate LLM-ready context package for an ask."""
    repo_path = Path(repo).resolve()
    db_path = repo_path / INDEX_DIR / DB_NAME
    if not db_path.exists():
        # Auto-index if no index exists
        click.echo(f"No index found. Indexing {repo_path}...", err=True)
        _ensure_index_dir(repo)
        db = IndexDB(db_path)
        index_repo(repo_path, db)
    else:
        db = IndexDB(db_path)

    result = analyze_query(ask, db)
    ctx = generate_context(result, db, repo_path, max_snippet_lines=max_snippet_lines, brief=brief)

    if brief and not output_path:
        # Brief mode: write full context to .pruner/context.md, print summary to stdout
        context_file = repo_path / INDEX_DIR / "context.md"
        context_file.parent.mkdir(exist_ok=True)
        full_text = format_context_text(ctx)
        context_file.write_text(full_text)
        summary = format_context_summary(ctx)
        click.echo(summary)
        click.echo("")
        # Show index age
        index_mtime = (repo_path / INDEX_DIR / DB_NAME).stat().st_mtime
        age_secs = time.time() - index_mtime
        if age_secs < 60:
            age_str = f"{int(age_secs)}s ago"
        elif age_secs < 3600:
            age_str = f"{int(age_secs / 60)}m ago"
        elif age_secs < 86400:
            age_str = f"{age_secs / 3600:.1f}h ago"
        else:
            age_str = f"{age_secs / 86400:.1f}d ago"
        click.echo(f"Index updated: {age_str}")
        click.echo(f"Full context: {context_file} ({len(full_text):,} chars)")
        click.echo("Use Read/Grep on that file for snippets, call graphs, and imports.")
    else:
        outputs = {}
        if fmt in ("text", "both"):
            outputs["text"] = format_context_text(ctx)
        if fmt in ("json", "both"):
            outputs["json"] = format_context_json(ctx)

        if output_path:
            out = Path(output_path)
            if fmt == "both":
                out.with_suffix(".txt").write_text(outputs["text"])
                out.with_suffix(".json").write_text(outputs["json"])
                click.echo(f"Written to {out.with_suffix('.txt')} and {out.with_suffix('.json')}")
            else:
                out.write_text(outputs.get("text", outputs.get("json", "")))
                click.echo(f"Written to {out}")
        else:
            if "text" in outputs:
                click.echo(outputs["text"])
            if "json" in outputs:
                if "text" in outputs:
                    click.echo("\n---JSON---\n")
                click.echo(outputs["json"])

    db.close()


@cli.command("show-file")
@click.argument("repo", default=".")
@click.argument("path")
def show_file(repo: str, path: str):
    """Show indexed information about a file."""
    db = _get_db(repo)
    f = db.get_file(path)
    if not f:
        # Try partial match
        matches = db.search_files(path)
        if not matches:
            click.echo(f"File not found: {path}", err=True)
            db.close()
            sys.exit(1)
        if len(matches) > 1:
            click.echo(f"Multiple matches for '{path}':")
            for m in matches:
                click.echo(f"  {m['path']}")
            db.close()
            return
        f = matches[0]

    click.echo(f"File: {f['path']}")
    click.echo(f"Language: {f['language'] or 'unknown'}")
    click.echo(f"Lines: {f['line_count']}")
    click.echo(f"Size: {f['size']} bytes")
    click.echo(f"Test: {'yes' if f['is_test'] else 'no'}")

    symbols = db.get_symbols_in_file(f["id"])
    if symbols:
        click.echo(f"\nSymbols ({len(symbols)}):")
        for s in symbols:
            sig = f" - {s['signature']}" if s.get("signature") else ""
            click.echo(f"  {s['name']} ({s['kind']}) L{s['line_start']}-{s['line_end']}{sig}")

    imports = db.get_imports_for_file(f["id"])
    if imports:
        click.echo(f"\nImports ({len(imports)}):")
        for imp in imports:
            names = f" ({imp['names']})" if imp["names"] else ""
            click.echo(f"  {imp['module']}{names}")

    # Show edges
    edges = db.get_edges(source_file_id=f["id"])
    if edges:
        click.echo(f"\nOutgoing edges ({len(edges)}):")
        for e in edges:
            target = ""
            if e["target_file_id"]:
                tf = db.get_file_by_id(e["target_file_id"])
                target = tf["path"] if tf else f"file#{e['target_file_id']}"
            elif e["target_name"]:
                target = e["target_name"]
            click.echo(f"  --{e['kind']}--> {target}")

    db.close()


@cli.command("show-symbol")
@click.argument("repo", default=".")
@click.argument("symbol")
def show_symbol(repo: str, symbol: str):
    """Show indexed information about a symbol."""
    db = _get_db(repo)
    matches = db.search_symbols(symbol)
    if not matches:
        click.echo(f"Symbol not found: {symbol}", err=True)
        db.close()
        sys.exit(1)

    for s in matches:
        f = db.get_file_by_id(s["file_id"])
        click.echo(f"\n{s['name']} ({s['kind']})")
        click.echo(f"  File: {f['path'] if f else 'unknown'}")
        click.echo(f"  Lines: {s['line_start']}-{s['line_end']}")
        if s.get("signature"):
            click.echo(f"  Signature: {s['signature']}")

        calls = db.get_calls_for_symbol(s["id"])
        if calls:
            click.echo(f"  Calls ({len(calls)}):")
            for c in calls:
                click.echo(f"    → {c['callee_name']} (L{c['line']})")

        callers = db.get_callers_of(s["name"])
        if callers:
            click.echo(f"  Called by ({len(callers)}):")
            for c in callers:
                click.echo(f"    ← {c['caller_name']}")

    db.close()


@cli.command()
@click.argument("repo", default=".")
def stats(repo: str):
    """Show index statistics."""
    db = _get_db(repo)
    s = db.get_stats()
    click.echo(f"Index stats for {Path(repo).resolve()}:")
    click.echo(f"  Files:   {s['files']}")
    click.echo(f"  Symbols: {s['symbols']}")
    click.echo(f"  Imports: {s['imports']}")
    click.echo(f"  Calls:   {s['calls']}")
    click.echo(f"  Edges:   {s['edges']}")
    db.close()


@cli.command()
@click.argument("repo", default=".")
@click.argument("ask")
@click.option("--max-snippet-lines", default=50, help="Max lines per code snippet")
@click.option("--json-output", "json_out", is_flag=True, help="Output as JSON")
def measure(repo: str, ask: str, max_snippet_lines: int, json_out: bool):
    """Measure token usage: pruner context vs naive full-file inclusion."""
    db = _get_db(repo)
    repo_path = Path(repo).resolve()

    result = analyze_query(ask, db)
    m = measure_tokens(result, db, repo_path, max_snippet_lines=max_snippet_lines)

    if json_out:
        output = {
            "ask": m.ask,
            "repo_total": {"files": m.repo_total_files, "tokens": m.repo_total_tokens},
            "naive": {
                "files": len(m.naive_files),
                "lines": m.naive_lines,
                "tokens": m.naive_tokens,
            },
            "pruner": {
                "files": m.pruner_files,
                "symbols": m.pruner_symbols,
                "snippets": m.pruner_snippets,
                "tokens_text": m.pruner_tokens_text,
                "tokens_json": m.pruner_tokens_json,
            },
            "reduction_vs_naive_pct": round(m.reduction_vs_naive, 1),
            "reduction_vs_repo_pct": round(m.reduction_vs_repo, 1),
        }
        click.echo(json.dumps(output, indent=2))
    else:
        click.echo(f"Token usage measurement for: {m.ask}")
        click.echo("")
        click.echo("Whole repo (baseline):")
        click.echo(f"  {m.repo_total_files} files, ~{m.repo_total_tokens:,} tokens")
        click.echo("")
        click.echo("Naive (full content of matching files):")
        click.echo(f"  {len(m.naive_files)} files, {m.naive_lines:,} lines, ~{m.naive_tokens:,} tokens")
        for f in m.naive_files:
            click.echo(f"    {f}")
        click.echo("")
        click.echo("Pruner (structured context):")
        click.echo(f"  {m.pruner_files} files, {m.pruner_symbols} symbols, {m.pruner_snippets} snippets")
        click.echo(f"  ~{m.pruner_tokens_text:,} tokens (text) / ~{m.pruner_tokens_json:,} tokens (json)")
        click.echo("")
        click.echo("Savings:")
        click.echo(f"  vs naive:     {m.reduction_vs_naive:+.1f}% tokens ({m.naive_tokens - m.pruner_tokens_text:+,})")
        click.echo(f"  vs whole repo: {m.reduction_vs_repo:+.1f}% tokens ({m.repo_total_tokens - m.pruner_tokens_text:+,})")

    db.close()


@cli.command()
@click.argument("repo", default=".")
@click.argument("ask")
@click.option("--max-snippet-lines", default=50, help="Max lines per code snippet")
@click.option("--json-output", "json_out", is_flag=True, help="Output as JSON")
@click.option("--show-steps", is_flag=True, help="Show individual exploration steps")
def estimate(repo: str, ask: str, max_snippet_lines: int, json_out: bool, show_steps: bool):
    """Estimate realistic Claude Code token usage with and without pruner.

    Models what Claude actually does: glob/grep exploration, reading wrong files,
    following imports — vs running pruner first then reading targeted files.
    """
    db = _get_db(repo)
    repo_path = Path(repo).resolve()

    result = analyze_query(ask, db)
    est = estimate_claude_session(result, db, repo_path, max_snippet_lines=max_snippet_lines)

    if json_out:
        output = {
            "ask": est.ask,
            "without_pruner": {
                "exploration_tokens": est.without_exploration_tokens,
                "relevant_read_tokens": est.without_relevant_tokens,
                "total_tokens": est.without_total_tokens,
                "files_read": est.without_files_read,
                "irrelevant_reads": est.without_irrelevant_reads,
            },
            "with_pruner": {
                "pruner_context_tokens": est.with_pruner_context_tokens,
                "targeted_read_tokens": est.with_targeted_read_tokens,
                "total_tokens": est.with_total_tokens,
                "files_read": est.with_files_read,
            },
            "saving_tokens": est.token_saving,
            "saving_pct": round(est.saving_pct, 1),
        }
        click.echo(json.dumps(output, indent=2))
    else:
        click.echo(f"Claude Code session estimate for: {est.ask}")
        click.echo("")

        click.echo("WITHOUT pruner (explore → read):")
        click.echo(f"  Exploration (glob/grep):   ~{est.without_exploration_tokens:,} tokens")
        irrelevant_detail = f" ({est.without_irrelevant_reads} irrelevant)" if est.without_irrelevant_reads else ""
        click.echo(f"  Reading files:             ~{est.without_relevant_tokens:,} tokens"
                    f" ({len(est.relevant_files)} files)")
        click.echo(f"  Wasted on wrong files:     ~{sum(s.tokens for s in est.without_steps if not s.useful):,} tokens"
                    f"{irrelevant_detail}")
        click.echo(f"  Total:                     ~{est.without_total_tokens:,} tokens"
                    f" ({est.without_files_read} files read)")

        click.echo("")
        click.echo("WITH pruner (context → targeted read):")
        click.echo(f"  Pruner context output:     ~{est.with_pruner_context_tokens:,} tokens")
        click.echo(f"  Targeted file reads:       ~{est.with_targeted_read_tokens:,} tokens"
                    f" ({est.with_files_read} files)")
        click.echo(f"  Total:                     ~{est.with_total_tokens:,} tokens")

        click.echo("")
        saving_sign = "+" if est.saving_pct >= 0 else ""
        click.echo(f"Estimated saving: {saving_sign}{est.saving_pct:.1f}%"
                    f" ({est.token_saving:+,} tokens)")

        if show_steps:
            click.echo("")
            click.echo("Exploration steps (without pruner):")
            for step in est.without_steps:
                marker = "  " if step.useful else "* "
                click.echo(f"  {marker}{step.action:18s} {step.target:50s} ~{step.tokens:,} tokens")
            click.echo("  (* = wasted on irrelevant content)")

    db.close()
