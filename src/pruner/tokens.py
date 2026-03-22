"""Token estimation and context size measurement.

Uses a simple heuristic tokenizer (≈words + punctuation) that approximates
GPT/Claude token counts without requiring tiktoken or any external dependency.
Accurate to ~10-15% for English prose and code, which is sufficient for
comparing strategies.
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from pathlib import Path

from .context import format_context_json, format_context_text, generate_context
from .db import IndexDB
from .query import QueryResult

# Rough ratio: 1 token ≈ 4 characters for English/code.
# We also count punctuation and whitespace boundaries to be closer to BPE.
_TOKEN_RE = re.compile(r"""\w+|[^\w\s]|\n""")


def estimate_tokens(text: str) -> int:
    """Estimate token count for a string using a character/word heuristic."""
    return len(_TOKEN_RE.findall(text))


@dataclass
class Measurement:
    """Token usage comparison between naive and pruner approaches."""

    ask: str
    # Naive: dump all matching files in full
    naive_files: list[str]
    naive_tokens: int
    naive_lines: int
    # Pruner: structured context
    pruner_tokens_text: int
    pruner_tokens_json: int
    pruner_files: int
    pruner_symbols: int
    pruner_snippets: int
    # Whole repo baseline
    repo_total_tokens: int
    repo_total_files: int

    @property
    def reduction_vs_naive(self) -> float:
        """Percentage reduction compared to naive full-file approach."""
        if self.naive_tokens == 0:
            return 0.0
        return (1 - self.pruner_tokens_text / self.naive_tokens) * 100

    @property
    def reduction_vs_repo(self) -> float:
        """Percentage reduction compared to sending the whole repo."""
        if self.repo_total_tokens == 0:
            return 0.0
        return (1 - self.pruner_tokens_text / self.repo_total_tokens) * 100


@dataclass
class ExplorationStep:
    """A single step in a simulated Claude Code exploration."""
    action: str     # glob, grep, read, read_irrelevant, pruner_context
    target: str     # what was searched/read
    tokens: int     # tokens consumed by this step
    useful: bool    # did this step contribute to finding relevant code


@dataclass
class ClaudeEstimate:
    """Realistic estimate of Claude Code token usage with and without pruner."""

    ask: str

    # Without pruner: exploration + reading
    without_steps: list[ExplorationStep] = field(default_factory=list)
    without_exploration_tokens: int = 0   # globs, greps, wrong reads
    without_relevant_tokens: int = 0      # reading files that matter
    without_total_tokens: int = 0

    # With pruner: pruner context + targeted reads
    with_pruner_context_tokens: int = 0   # the pruner output itself
    with_targeted_read_tokens: int = 0    # reading top files after pruner
    with_total_tokens: int = 0

    # Files
    relevant_files: list[str] = field(default_factory=list)
    without_files_read: int = 0
    without_irrelevant_reads: int = 0
    with_files_read: int = 0

    @property
    def token_saving(self) -> int:
        return self.without_total_tokens - self.with_total_tokens

    @property
    def saving_pct(self) -> float:
        if self.without_total_tokens == 0:
            return 0.0
        return (1 - self.with_total_tokens / self.without_total_tokens) * 100


def estimate_claude_session(
    query_result: QueryResult,
    db: IndexDB,
    repo_path: str | Path,
    max_snippet_lines: int = 50,
) -> ClaudeEstimate:
    """Model realistic Claude Code token usage with and without pruner.

    Without pruner, Claude typically:
    1. Globs to understand directory structure (~2-4 glob calls)
    2. Greps for keywords from the ask (~2-3 grep calls)
    3. Reads files found by grep, some relevant, some not
    4. Follows imports from relevant files to find connected code
    5. Reads those connected files

    With pruner, Claude:
    1. Runs pruner context (one command)
    2. Reads the top key files identified by pruner (targeted)
    """
    repo = Path(repo_path).resolve()
    est = ClaudeEstimate(ask=query_result.ask)

    # --- Collect file data ---

    # Relevant files (what pruner identifies)
    relevant_file_ids = query_result.all_relevant_file_ids
    relevant_files: dict[int, dict] = {}
    file_contents: dict[int, str] = {}
    file_tokens: dict[int, int] = {}

    for fid in relevant_file_ids:
        f = db.get_file_by_id(fid)
        if not f:
            continue
        fpath = repo / f["path"]
        try:
            content = fpath.read_text(encoding="utf-8", errors="ignore")
            relevant_files[fid] = f
            file_contents[fid] = content
            file_tokens[fid] = estimate_tokens(content)
        except (OSError, PermissionError):
            continue

    est.relevant_files = [f["path"] for f in relevant_files.values()]

    # --- Model: WITHOUT pruner ---

    # Step 1: Directory exploration (2-4 glob calls)
    # Claude typically globs the root, then subdirs matching keywords
    dir_depth = _estimate_dir_depth(repo)
    glob_calls = min(2 + len(query_result.keywords), 5)
    glob_tokens_per_call = 50 + dir_depth * 15  # listing output tokens
    for i in range(glob_calls):
        kw = query_result.keywords[i] if i < len(query_result.keywords) else "src"
        step = ExplorationStep(
            action="glob",
            target=f"**/*{kw}*" if i > 0 else "top-level structure",
            tokens=glob_tokens_per_call,
            useful=True,
        )
        est.without_steps.append(step)
        est.without_exploration_tokens += step.tokens

    # Step 2: Grep for keywords (2-3 calls)
    grep_calls = min(len(query_result.keywords), 3)
    for i in range(grep_calls):
        kw = query_result.keywords[i]
        # Grep output: ~10 lines of context per match, estimate matches
        grep_output_tokens = 80 + len(query_result.matching_symbols) * 15
        step = ExplorationStep(
            action="grep",
            target=kw,
            tokens=grep_output_tokens,
            useful=True,
        )
        est.without_steps.append(step)
        est.without_exploration_tokens += step.tokens

    # Step 3: Read files — Claude reads some relevant, some not
    # Model: Claude finds ~60% of relevant files directly, reads ~30% irrelevant ones
    # on the way. Then follows imports to find the remaining ~40%.

    all_indexed = db.get_all_files()
    non_relevant_code_files = [
        f for f in all_indexed
        if f["id"] not in relevant_file_ids
        and f["language"] is not None
        and not f["is_test"]
    ]

    # Irrelevant reads: Claude reads files that match keywords but aren't useful
    # Estimate: ~20-40% of relevant file count, depending on repo size
    irrelevant_ratio = min(0.4, 0.15 + len(all_indexed) / 2000)
    irrelevant_count = max(1, int(len(relevant_files) * irrelevant_ratio))
    # Pick plausible irrelevant files (ones that share keywords in path)
    irrelevant_candidates = []
    for f in non_relevant_code_files:
        path_lower = f["path"].lower()
        if any(kw in path_lower for kw in query_result.keywords):
            irrelevant_candidates.append(f)
    irrelevant_to_read = irrelevant_candidates[:irrelevant_count]

    for f in irrelevant_to_read:
        fpath = repo / f["path"]
        try:
            content = fpath.read_text(encoding="utf-8", errors="ignore")
            tokens = estimate_tokens(content)
        except (OSError, PermissionError):
            tokens = 200
        step = ExplorationStep(
            action="read_irrelevant",
            target=f["path"],
            tokens=tokens,
            useful=False,
        )
        est.without_steps.append(step)
        est.without_exploration_tokens += tokens
        est.without_irrelevant_reads += 1

    # Relevant reads: Claude reads the actually relevant files (full content)
    for fid, f in relevant_files.items():
        step = ExplorationStep(
            action="read",
            target=f["path"],
            tokens=file_tokens[fid],
            useful=True,
        )
        est.without_steps.append(step)
        est.without_relevant_tokens += file_tokens[fid]

    est.without_files_read = len(relevant_files) + len(irrelevant_to_read)
    est.without_total_tokens = est.without_exploration_tokens + est.without_relevant_tokens

    # --- Model: WITH pruner ---

    # Step 1: pruner context output
    ctx = generate_context(query_result, db, repo, max_snippet_lines=max_snippet_lines)
    pruner_text = format_context_text(ctx)
    est.with_pruner_context_tokens = estimate_tokens(pruner_text)

    # Step 2: Read top key files in full (Claude still needs full content to edit)
    # But pruner already told Claude WHICH files matter, so no exploration waste.
    # Claude reads the top ~5-10 most relevant files (the ones with matching symbols).
    symbol_file_ids = {s["file_id"] for s in query_result.matching_symbols}
    # Also include files from execution paths (first steps only, not deep)
    for path in query_result.execution_paths:
        for step in path[:2]:  # top 2 steps of each path
            if "file_id" in step:
                symbol_file_ids.add(step["file_id"])

    # Cap at reasonable number — Claude wouldn't read 50 files
    targeted_ids = list(symbol_file_ids)[:15]
    for fid in targeted_ids:
        if fid in file_tokens:
            est.with_targeted_read_tokens += file_tokens[fid]

    est.with_files_read = len(targeted_ids)
    est.with_total_tokens = est.with_pruner_context_tokens + est.with_targeted_read_tokens

    return est


def measure(
    query_result: QueryResult,
    db: IndexDB,
    repo_path: str | Path,
    max_snippet_lines: int = 50,
) -> Measurement:
    """Measure token usage: pruner context vs naive full-file inclusion."""
    repo = Path(repo_path).resolve()

    # Generate pruner context
    ctx = generate_context(query_result, db, repo, max_snippet_lines=max_snippet_lines)
    text_output = format_context_text(ctx)
    json_output = format_context_json(ctx)

    # Naive approach: read full contents of all relevant files
    relevant_file_ids = query_result.all_relevant_file_ids
    naive_content = ""
    naive_files = []
    naive_lines = 0
    for fid in relevant_file_ids:
        f = db.get_file_by_id(fid)
        if not f:
            continue
        fpath = repo / f["path"]
        try:
            content = fpath.read_text(encoding="utf-8", errors="ignore")
            naive_content += f"\n\n--- {f['path']} ---\n\n{content}"
            naive_files.append(f["path"])
            naive_lines += content.count("\n") + 1
        except (OSError, PermissionError):
            continue

    # Whole repo baseline
    repo_tokens = 0
    repo_files = 0
    for f in db.get_all_files():
        fpath = repo / f["path"]
        try:
            content = fpath.read_text(encoding="utf-8", errors="ignore")
            repo_tokens += estimate_tokens(content)
            repo_files += 1
        except (OSError, PermissionError):
            continue

    return Measurement(
        ask=query_result.ask,
        naive_files=naive_files,
        naive_tokens=estimate_tokens(naive_content),
        naive_lines=naive_lines,
        pruner_tokens_text=estimate_tokens(text_output),
        pruner_tokens_json=estimate_tokens(json_output),
        pruner_files=len(ctx["key_files"]),
        pruner_symbols=len(ctx["key_symbols"]),
        pruner_snippets=len(ctx["snippets"]),
        repo_total_tokens=repo_tokens,
        repo_total_files=repo_files,
    )


def _estimate_dir_depth(repo: Path) -> int:
    """Estimate directory nesting depth of a repo (capped walk)."""
    max_depth = 0
    count = 0
    for dirpath, dirnames, _ in os.walk(repo):
        rel = os.path.relpath(dirpath, repo)
        depth = rel.count(os.sep) + 1 if rel != "." else 0
        max_depth = max(max_depth, depth)
        count += 1
        if count > 200:  # don't walk the whole tree
            break
        # Skip common ignored dirs
        dirnames[:] = [d for d in dirnames if d not in {
            ".git", "node_modules", "__pycache__", ".venv", "dist", "build", ".pruner",
        }]
    return max_depth
