#!/usr/bin/env python3
"""Categorize why pruner missed files in A/B test runs.

For each file Claude read that pruner didn't suggest, classify the miss:

  structural_import — miss's basename appears in at least one suggested file's
      content (the suggestion imports/references it). Graph expansion, not
      synonyms, would fix this.

  neighbor_dir — miss shares a parent directory with at least one suggestion.
      Directory-co-location lever, not vocabulary.

  ranking_below_cutoff — at least one query keyword (or its stem) appears as
      a token in the miss's path, but the file didn't survive the 0.25× cutoff
      or the MAX_RESULT_FILES cap. Scoring/ranking issue.

  vocabulary_gap — no query keyword (or stem, or current-cluster synonym)
      appears in the miss's path. Only a wider synonym table would bridge it.

  structural_module — miss is a *.module.ts / index.ts / config.* / main.ts
      wiring file. These show up repeatedly as "Claude reads the module to
      understand the wiring"; graph/import-aware expansion is the real fix.

Usage:
    python3 tests/miss_categorization.py tests/ab-tests/sonnet_nest_oneshot_n10_v027_20260406.json \\
        --repo /tmp/pruner-bench/nest --pruner ./target/release/pruner
"""

import argparse
import json
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path


NAVIGATION_TOOLS = {"Grep", "Glob", "Bash", "Agent"}
PRODUCTIVE_TOOLS = {"Read", "Edit", "Write", "NotebookEdit"}

# Module-wiring file patterns — these are structural misses by nature
MODULE_PATTERNS = [
    re.compile(r"\.module\.(ts|js|tsx|jsx)$"),
    re.compile(r"(^|/)index\.(ts|js|tsx|jsx|py|rs)$"),
    re.compile(r"(^|/)main\.(ts|js|py|rs|go)$"),
    re.compile(r"(^|/)config\.(ts|js|py|toml|yaml|yml|json)$"),
    re.compile(r"\.config\.(ts|js)$"),
    re.compile(r"(^|/)mod\.rs$"),
    re.compile(r"(^|/)__init__\.py$"),
]

NEST_QUERIES = {
    "understanding": "How does the NestJS dependency injection system work? Trace how @Injectable() decorators and the injector resolve dependencies.",
    "implement": "Add a request timing interceptor that measures how long each request takes and logs it. Find where interceptors are registered and add it there.",
    "iterative_refinement": "Add a request timing interceptor that measures how long each request takes and adds an X-Response-Time header to the response.",
}

OPENCLAW_QUERIES = {
    "narrow_fix": "What files handle WebSocket reconnection in this repo? List the file paths and briefly explain what each does.",
    "cross_package": "How does a message flow from a webhook received by an extension to the core message handler in this repo? Trace the path through the key files.",
    "understanding": "How does the plugin/extension loading system work in this repo? What are the key files and entry points?",
    "data_flow": "How does authentication and token validation work in this repo? List the key files and describe the flow.",
    "implement": "Implement a health check endpoint that returns JSON with the server version and uptime. Find where HTTP routes are registered and add it there.",
    "implement_large": "Add a rate limiting system for incoming messages. Create a RateLimiter class that tracks per-channel message counts with a sliding window (default: 30 messages per 60 seconds). Integrate it into the message routing pipeline so that messages exceeding the limit are rejected with a user-friendly reply. Add configuration options to set custom limits per channel. Include unit tests.",
}

STOP_WORDS = {
    "the", "a", "an", "and", "or", "but", "is", "are", "was", "were", "be",
    "been", "being", "have", "has", "had", "do", "does", "did", "how", "what",
    "when", "where", "why", "who", "which", "to", "of", "in", "on", "for",
    "with", "from", "by", "at", "as", "this", "that", "it", "its", "i", "we",
    "you", "they", "them", "find", "list", "show", "trace", "add", "each",
    "there", "here", "your", "my", "some", "any", "all", "can", "could",
    "should", "would", "will", "also", "about", "their", "describe", "briefly",
    "explain", "path", "key",
}


def tokenize(text):
    """Split text into lowercased alpha tokens, drop stop words and short."""
    toks = re.findall(r"[a-zA-Z][a-zA-Z0-9]+", text.lower())
    return [t for t in toks if len(t) >= 4 and t not in STOP_WORDS]


def path_tokens(path):
    """Tokenize a file path: split on /, -, _, ., camelCase."""
    parts = re.split(r"[/\-_.]", path)
    out = set()
    for p in parts:
        if not p:
            continue
        # camelCase split
        sub = re.findall(r"[A-Z]?[a-z]+|[A-Z]+(?=[A-Z]|$)", p)
        for s in sub:
            if len(s) >= 3:
                out.add(s.lower())
        if len(p) >= 3:
            out.add(p.lower())
    return out


def extract_pruner_suggestions(repo_path, query, pruner_bin):
    """Run pruner context --full --format json. Return set of suggested paths."""
    result = subprocess.run(
        [pruner_bin, "context", repo_path, query, "--format", "json", "--full"],
        capture_output=True, text=True, timeout=60,
    )
    if result.returncode != 0:
        print(f"Warning: pruner failed: {result.stderr[:200]}", file=sys.stderr)
        return set()
    data = json.loads(result.stdout)
    suggested = {f["path"] for f in data.get("key_files", [])}
    for s in data.get("snippets", []):
        suggested.add(s["file"])
    for s in data.get("key_symbols", []):
        suggested.add(s["file"])
    for t in data.get("relevant_tests", []):
        suggested.add(t["path"])
    return suggested


def extract_tool_files(tools_list, workspace_prefix=None):
    """Return (files_read, files_written) as sets of repo-relative paths."""
    files_read = set()
    files_written = set()
    for t in tools_list:
        tool = t.get("name", "")
        if tool not in PRODUCTIVE_TOOLS:
            continue
        preview = t.get("input_preview", "")
        m = re.search(r"'file_path':\s*'([^']+)'?", preview)
        if not m:
            continue
        path = m.group(1)
        if not re.search(r"\.\w+$", path):
            continue
        if workspace_prefix and path.startswith(workspace_prefix):
            path = path[len(workspace_prefix):].lstrip("/")
        if tool == "Write":
            files_written.add(path)
        else:
            files_read.add(path)
    return files_read - files_written, files_written


def detect_workspace_prefix(tools_list):
    for t in tools_list:
        preview = t.get("input_preview", "")
        m = re.search(r"'file_path':\s*'([^']+)'", preview)
        if not m:
            continue
        fp = m.group(1)
        m2 = re.match(r"(.*/pruner-bench/ab-workspace/(?:with-pruner|without-pruner)/)", fp)
        if m2:
            return m2.group(1)
    return None


def is_module_file(path):
    return any(pat.search(path) for pat in MODULE_PATTERNS)


def read_file_safe(repo_path, rel_path, max_bytes=200_000):
    full = os.path.join(repo_path, rel_path)
    try:
        with open(full, "rb") as f:
            return f.read(max_bytes).decode("utf-8", errors="replace")
    except (FileNotFoundError, IsADirectoryError, PermissionError):
        return None


def miss_is_imported_by_hit(miss_path, hit_paths, repo_path):
    """True if some hit file's source mentions the miss's basename-stem."""
    basename = os.path.basename(miss_path)
    stem = basename.rsplit(".", 1)[0]
    if len(stem) < 4:
        return False
    # Look for the stem as a standalone token/string in any hit
    pattern = re.compile(r"[\"'/]" + re.escape(stem) + r"[\"'/.]")
    for hit in hit_paths:
        content = read_file_safe(repo_path, hit)
        if content and pattern.search(content):
            return True
    return False


def classify_miss(miss_path, hits, query_tokens, query_synonyms, repo_path):
    """Classify a miss. Returns one of: structural_module, structural_import,
    neighbor_dir, ranking_below_cutoff, vocabulary_gap."""
    if is_module_file(miss_path):
        return "structural_module"

    if miss_is_imported_by_hit(miss_path, hits, repo_path):
        return "structural_import"

    miss_dir = os.path.dirname(miss_path)
    hit_dirs = {os.path.dirname(h) for h in hits}
    if miss_dir and miss_dir in hit_dirs:
        return "neighbor_dir"

    mtoks = path_tokens(miss_path)
    if mtoks & query_tokens:
        return "ranking_below_cutoff"
    if mtoks & query_synonyms:
        return "ranking_below_cutoff"

    return "vocabulary_gap"


# ---------------------------------------------------------------------------
# Current synonym clusters — mirror src/synonyms.rs so we can tell "covered by
# existing clusters" apart from "true vocabulary gap".
# ---------------------------------------------------------------------------
CURRENT_CLUSTERS = [
    ["auth", "authenticate", "authentication", "login", "signin", "logon"],
    ["logout", "signout", "logoff"],
    ["authorize", "authorization"],
    ["credential", "credentials"],
    ["password", "passwd"],
    ["token", "jwt", "bearer"],
    ["endpoint", "route", "handler"],
    ["request", "req"],
    ["response", "resp", "reply"],
    ["websocket", "ws", "socket"],
    ["connect", "connection"],
    ["disconnect", "teardown", "close"],
    ["reconnect", "reconnection"],
    ["retry", "retries"],
    ["timeout", "deadline"],
    ["ratelimit", "throttle", "quota"],
    ["backpressure", "backoff"],
    ["cache", "caching", "memoize"],
    ["invalidate", "evict", "expire"],
    ["database", "db"],
    ["query", "queries"],
    ["migration", "migrate"],
    ["transaction", "txn"],
    ["schema", "ddl"],
    ["publish", "publisher", "produce", "producer"],
    ["subscribe", "subscriber", "consume", "consumer"],
    ["queue", "topic", "channel"],
    ["async", "asynchronous"],
    ["concurrent", "parallel"],
    ["log", "logger", "logging"],
    ["trace", "tracing"],
    ["metric", "metrics", "telemetry"],
    ["exception", "panic"],
    ["test", "spec"],
    ["mock", "stub", "fake"],
    ["assert", "expect"],
    ["config", "configuration", "settings"],
    ["env", "environment"],
    ["flag", "toggle"],
    ["secret", "credential"],
    ["build", "compile"],
    ["deploy", "release", "rollout"],
    ["container", "docker"],
    ["parse", "parser", "parsing"],
    ["serialize", "marshal", "encode"],
    ["deserialize", "unmarshal", "decode"],
    ["error", "err"],
    ["failure", "fail"],
    ["list", "array"],
    ["map", "dict", "dictionary"],
    ["init", "initialize", "initialise", "setup", "bootstrap"],
    ["start", "startup"],
    ["stop", "shutdown"],
    ["render", "draw"],
    ["component", "widget"],
    ["event", "signal"],
]


def expand_with_current_synonyms(tokens):
    """Return tokens ∪ all cluster members where any member matched a token."""
    result = set(tokens)
    for cluster in CURRENT_CLUSTERS:
        if any(t in cluster for t in tokens):
            result.update(cluster)
    return result


def analyze_results(results_path, repo_path, pruner_bin):
    with open(results_path) as f:
        data = json.load(f)

    categories = Counter()
    per_category = defaultdict(Counter)
    miss_examples = defaultdict(list)

    # Determine query set
    first_round = data["rounds"][0] if isinstance(data["rounds"][0], list) else data["rounds"][0].get("tasks", [])
    cats = {t.get("category") for t in first_round}
    task_queries = OPENCLAW_QUERIES if cats & {"narrow_fix", "data_flow", "implement_large"} else NEST_QUERIES

    # Cache pruner suggestions per query
    suggestion_cache = {}

    total_misses = 0
    for round_idx, rd in enumerate(data.get("rounds", [])):
        task_list = rd if isinstance(rd, list) else rd.get("tasks", [])
        for task in task_list:
            category = task.get("category")
            wp = task.get("with_pruner", {})
            if not wp:
                continue
            tools = wp.get("tools", [])
            prefix = detect_workspace_prefix(tools)
            files_read, _ = extract_tool_files(tools, prefix)

            query = task_queries.get(category)
            if not query:
                continue

            if query not in suggestion_cache:
                suggestion_cache[query] = extract_pruner_suggestions(repo_path, query, pruner_bin)
            suggested = suggestion_cache[query]

            # Drop pruner's own output files — Claude reading .pruner/context.md
            # isn't a real "miss", it's Claude consulting pruner's suggestions.
            files_read = {f for f in files_read if not f.startswith(".pruner/") and not f.endswith("CLAUDE.md")}

            hits = files_read & suggested
            misses = files_read - suggested

            qtoks = set(tokenize(query))
            qsyns = expand_with_current_synonyms(qtoks)

            for miss in misses:
                cat = classify_miss(miss, hits, qtoks, qsyns, repo_path)
                categories[cat] += 1
                per_category[category][cat] += 1
                total_misses += 1
                if len(miss_examples[cat]) < 6:
                    miss_examples[cat].append((category, miss))

    print(f"\nDataset: {os.path.basename(results_path)}")
    print(f"Total misses across all runs: {total_misses}")
    print()
    print("Miss categorization (all tasks combined):")
    print("-" * 50)
    for cat, count in categories.most_common():
        pct = count / total_misses * 100 if total_misses else 0
        print(f"  {cat:25s} {count:4d}  ({pct:5.1f}%)")

    print("\nBy task category:")
    print("-" * 50)
    for task_cat, cat_counter in sorted(per_category.items()):
        total = sum(cat_counter.values())
        print(f"\n  [{task_cat}]  ({total} misses)")
        for mcat, count in cat_counter.most_common():
            pct = count / total * 100 if total else 0
            print(f"    {mcat:25s} {count:4d}  ({pct:5.1f}%)")

    print("\nExamples by miss type:")
    print("-" * 50)
    for mcat, examples in miss_examples.items():
        print(f"\n  [{mcat}]")
        for task_cat, path in examples:
            print(f"    ({task_cat}) {path}")

    return {
        "dataset": os.path.basename(results_path),
        "total_misses": total_misses,
        "by_category": dict(categories),
    }


def main():
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("path", help="results.json file (one or more)", nargs="+")
    p.add_argument("--repo", required=True, help="Repo root for re-running pruner")
    p.add_argument("--pruner", default="pruner", help="Path to pruner binary")
    args = p.parse_args()

    summaries = []
    for path in args.path:
        summaries.append(analyze_results(path, args.repo, args.pruner))

    if len(summaries) > 1:
        print("\n\n=== COMBINED ACROSS DATASETS ===")
        combined = Counter()
        for s in summaries:
            combined.update(s["by_category"])
        total = sum(combined.values())
        for cat, count in combined.most_common():
            pct = count / total * 100 if total else 0
            print(f"  {cat:25s} {count:4d}  ({pct:5.1f}%)")


if __name__ == "__main__":
    main()
