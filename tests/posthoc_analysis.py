#!/usr/bin/env python3
"""Post-hoc hit rate analysis: correlate pruner suggestions with Claude's actual behavior.

Reads raw JSONL logs from A/B tests and measures:
  - Precision: what fraction of pruner's suggested files did Claude actually use?
  - Recall: what fraction of files Claude used were in pruner's suggestions?
  - Navigation overhead: how many tool calls were pure exploration vs productive work?

Usage:
    # Analyze saved results.json files (from tests/ab-tests/)
    python3 tests/posthoc_analysis.py tests/ab-tests/fast_implement_n10.json --repo /tmp/pruner-bench/nest
    python3 tests/posthoc_analysis.py tests/ab-tests/ --repo /tmp/pruner-bench/nest --pruner ./target/release/pruner

    # Analyze raw JSONL logs (from /tmp/pruner-bench/ab-raw/)
    python3 tests/posthoc_analysis.py /tmp/pruner-bench/ab-raw/ --repo /tmp/pruner-bench/nest

    # Show per-file detail
    python3 tests/posthoc_analysis.py tests/ab-tests/fast_implement_n10.json --repo /tmp/pruner-bench/nest -v

Output:
    Results printed to stderr. JSON summary to stdout (pipe to file).
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path


# Tool calls that indicate file navigation/exploration
NAVIGATION_TOOLS = {"Grep", "Glob", "Bash", "Agent"}
# Tool calls that indicate productive work
PRODUCTIVE_TOOLS = {"Read", "Edit", "Write", "NotebookEdit"}


def parse_args():
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("path", help="JSONL file or directory of round*/ subdirs")
    p.add_argument("--repo", help="Repo path to re-run pruner context for suggestions")
    p.add_argument("--pruner", default="pruner",
                   help="Path to pruner binary (default: pruner)")
    p.add_argument("--verbose", "-v", action="store_true",
                   help="Show per-file hit/miss detail")
    return p.parse_args()


def extract_tool_calls(jsonl_path):
    """Extract all tool calls from a JSONL log file.

    Returns list of dicts: {tool, file_path, is_navigation, input}
    """
    calls = []
    with open(jsonl_path) as f:
        for line in f:
            entry = json.loads(line)
            if entry.get("type") != "assistant":
                continue
            msg = entry.get("message", {})
            content = msg.get("content", [])
            if not isinstance(content, list):
                continue
            for item in content:
                if not isinstance(item, dict) or item.get("type") != "tool_use":
                    continue
                tool = item.get("name", "")
                inp = item.get("input", {})
                file_path = None

                if tool in ("Read", "Edit", "Write"):
                    file_path = inp.get("file_path")
                elif tool == "Glob":
                    # Glob doesn't target a specific file, but the path/pattern
                    # tells us what area is being explored
                    pass
                elif tool == "Grep":
                    pass
                elif tool == "Bash":
                    # Try to extract file paths from bash commands
                    cmd = inp.get("command", "")
                    # Look for file read patterns
                    for m in re.findall(r'cat\s+(\S+)|head\s+.*?(\S+\.(?:ts|js|py|rs|go|java|c|cpp|cs))', cmd):
                        file_path = m[0] or m[1]
                        break

                calls.append({
                    "tool": tool,
                    "file_path": file_path,
                    "is_navigation": tool in NAVIGATION_TOOLS,
                    "input": inp,
                })
    return calls


def extract_files_used(calls, workspace_prefix=None):
    """Extract unique file paths that Claude actually Read/Edit/Wrote.

    Returns (files_read, files_written) — both sets of relative paths.
    files_read: files Claude Read or Edited (existing files).
    files_written: files Claude created with Write (new files pruner couldn't suggest).
    """
    files_read = set()
    files_written = set()
    for c in calls:
        if c["file_path"] and c["tool"] in PRODUCTIVE_TOOLS:
            path = c["file_path"]
            if workspace_prefix and path.startswith(workspace_prefix):
                path = path[len(workspace_prefix):].lstrip("/")
            if c["tool"] == "Write":
                files_written.add(path)
            else:
                files_read.add(path)
    # Files that were both written and then edited count as written (created by Claude)
    return files_read - files_written, files_written


def detect_workspace_prefix(calls):
    """Detect the workspace directory prefix from tool call paths."""
    for c in calls:
        fp = c.get("file_path", "")
        if fp and "/pruner-bench/ab-workspace/" in fp:
            # Extract up to and including with-pruner/ or without-pruner/
            m = re.match(r"(.*/pruner-bench/ab-workspace/(?:with-pruner|without-pruner)/)", fp)
            if m:
                return m.group(1)
    return None


def get_pruner_suggestions(repo_path, query, pruner_bin="pruner"):
    """Run pruner context and extract suggested file paths."""
    try:
        # Use --full to bypass query-aware budget (which may skip/brief repeated queries)
        result = subprocess.run(
            [pruner_bin, "context", repo_path, query, "--format", "json", "--full"],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode != 0:
            # Try cargo run
            result = subprocess.run(
                ["cargo", "run", "--release", "--", "context", repo_path, query, "--format", "json", "--full"],
                capture_output=True, text=True, timeout=60,
                cwd=os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
            )
        if result.returncode != 0:
            print("Warning: pruner context failed", file=sys.stderr)
            return set(), set()

        data = json.loads(result.stdout)
        key_files = {f["path"] for f in data.get("key_files", [])}
        # Also include files from snippets and symbols
        all_files = set(key_files)
        for s in data.get("snippets", []):
            all_files.add(s["file"])
        for s in data.get("key_symbols", []):
            all_files.add(s["file"])
        for t in data.get("relevant_tests", []):
            all_files.add(t["path"])
        return key_files, all_files
    except Exception as e:
        print("Warning: pruner context error: {}".format(e), file=sys.stderr)
        return set(), set()


def detect_task_query(jsonl_path):
    """Extract the task query from the JSONL filename or content."""
    # Filename pattern: implement_with.jsonl, understanding_without_turn0.jsonl
    basename = os.path.basename(jsonl_path)
    # Map task names to their queries (from ab_test.py)
    task_queries = {
        "understanding": "How does the NestJS dependency injection system work? Trace how @Injectable() decorators and the injector resolve dependencies.",
        "implement": "Add a request timing interceptor that measures how long each request takes and logs it. Find where interceptors are registered and add it there.",
        "iterative_refinement": "Add a request timing interceptor that measures how long each request takes and adds an X-Response-Time header to the response.",
    }
    for task, query in task_queries.items():
        if basename.startswith(task):
            return task, query
    return basename.split("_")[0], None


def is_followup_turn(jsonl_path):
    """Check if this is a follow-up turn (turn1, turn2, etc.) — not the initial prompt."""
    basename = os.path.basename(jsonl_path)
    return bool(re.search(r'_turn[1-9]\d*\.jsonl$', basename))


def analyze_log(jsonl_path, repo_path=None, pruner_bin="pruner", verbose=False):
    """Analyze a single JSONL log file."""
    calls = extract_tool_calls(jsonl_path)
    if not calls:
        return None

    prefix = detect_workspace_prefix(calls)
    files_read, files_written = extract_files_used(calls, prefix)
    all_files_used = files_read | files_written
    is_with_pruner = "_with" in os.path.basename(jsonl_path) and "_without" not in os.path.basename(jsonl_path)

    nav_calls = sum(1 for c in calls if c["is_navigation"])
    prod_calls = sum(1 for c in calls if not c["is_navigation"])
    total_calls = len(calls)

    result = {
        "file": os.path.basename(jsonl_path),
        "side": "with" if is_with_pruner else "without",
        "total_tool_calls": total_calls,
        "navigation_calls": nav_calls,
        "productive_calls": prod_calls,
        "files_read": sorted(files_read),
        "files_written": sorted(files_written),
        "files_used": sorted(all_files_used),
        "files_used_count": len(all_files_used),
    }

    # Skip hit rate for follow-up turns (pruner's query would be the follow-up
    # prompt, not the original task — not meaningful to compare)
    if is_followup_turn(jsonl_path):
        result["skipped_hit_rate"] = "follow-up turn"
        return result

    # If this is a with-pruner run and we have a repo, compute hit rates
    if is_with_pruner and repo_path:
        task, query = detect_task_query(jsonl_path)
        if query:
            key_files, all_suggested = get_pruner_suggestions(repo_path, query, pruner_bin)
            # Only compare against files_read (existing files), not files_written
            # (new files Claude created can't be in pruner's suggestions)
            hits = files_read & all_suggested
            misses = files_read - all_suggested
            unused = all_suggested - files_read

            precision = len(hits) / len(all_suggested) * 100 if all_suggested else 0
            recall = len(hits) / len(files_read) * 100 if files_read else 0

            result.update({
                "task": task,
                "query": query,
                "pruner_suggested": sorted(all_suggested),
                "pruner_key_files": sorted(key_files),
                "hits": sorted(hits),
                "misses": sorted(misses),
                "unused_suggestions": sorted(unused),
                "files_created": sorted(files_written),
                "precision": round(precision, 1),
                "recall": round(recall, 1),
            })

            if verbose:
                print("\n  {} — {}".format(os.path.basename(jsonl_path), task), file=sys.stderr)
                print("  Pruner suggested {} files, Claude read {} files, created {} files".format(
                    len(all_suggested), len(files_read), len(files_written)), file=sys.stderr)
                print("  Hits (suggested & read): {}".format(len(hits)), file=sys.stderr)
                for f in sorted(hits):
                    print("    + {}".format(f), file=sys.stderr)
                print("  Misses (read but not suggested): {}".format(len(misses)), file=sys.stderr)
                for f in sorted(misses):
                    print("    - {}".format(f), file=sys.stderr)
                if files_written:
                    print("  Created (new files, not in suggestions): {}".format(len(files_written)), file=sys.stderr)
                    for f in sorted(files_written):
                        print("    * {}".format(f), file=sys.stderr)
                print("  Unused (suggested but not read): {}".format(len(unused)), file=sys.stderr)
                for f in sorted(unused):
                    print("    ~ {}".format(f), file=sys.stderr)
                print("  Precision: {:.1f}%  Recall: {:.1f}%".format(precision, recall), file=sys.stderr)

    return result


def extract_tool_calls_from_results_json(tools_list):
    """Extract tool calls from a results.json tools array (input_preview format).

    Each entry has {name, input_preview} where input_preview is a string repr of a dict.
    """
    calls = []
    for t in tools_list:
        tool = t.get("name", "")
        preview = t.get("input_preview", "")
        file_path = None

        if tool in ("Read", "Edit", "Write"):
            # Extract file_path from preview string like "{'file_path': '/path/to/file'}"
            # Handle truncated previews where closing quote may be missing
            m = re.search(r"'file_path':\s*'([^']+)'?", preview)
            if m:
                path = m.group(1)
                # Skip truncated paths (no file extension = probably cut off)
                if re.search(r'\.\w+$', path):
                    file_path = path

        calls.append({
            "tool": tool,
            "file_path": file_path,
            "is_navigation": tool in NAVIGATION_TOOLS,
            "input": {},
        })
    return calls


def analyze_results_json(results_path, repo_path=None, pruner_bin="pruner", verbose=False):
    """Analyze a results.json file (from --save-raw A/B tests).

    Returns list of result dicts, same format as analyze_log.
    """
    with open(results_path) as f:
        data = json.load(f)

    results = []
    for round_idx, rd in enumerate(data.get("rounds", [])):
        # Normalize: rounds can be a list of task dicts (fast_*.json)
        # or a dict with "tasks" or "results" key (results.json, results_multi_repo.json)
        if isinstance(rd, dict):
            task_list = rd.get("tasks") or rd.get("results") or []
        else:
            task_list = rd
        for r in task_list:
            category = r.get("category", "unknown")
            for side_key, side_label in [("without", "without"), ("with_pruner", "with")]:
                side = r.get(side_key, {})
                tools_list = side.get("tools", [])
                calls = extract_tool_calls_from_results_json(tools_list)

                prefix = detect_workspace_prefix(calls)
                files_read, files_written = extract_files_used(calls, prefix)
                all_files_used = files_read | files_written

                nav_calls = sum(1 for c in calls if c["is_navigation"])
                prod_calls = sum(1 for c in calls if not c["is_navigation"])

                result = {
                    "file": "R{}/{}_{}".format(round_idx, category, side_label),
                    "side": side_label,
                    "total_tool_calls": len(calls),
                    "navigation_calls": nav_calls,
                    "productive_calls": prod_calls,
                    "files_read": sorted(files_read),
                    "files_written": sorted(files_written),
                    "files_used": sorted(all_files_used),
                    "files_used_count": len(all_files_used),
                }

                # Hit rate for with-pruner side
                if side_label == "with" and repo_path:
                    # Queries for NestJS/nest (fast mode)
                    nest_queries = {
                        "understanding": "How does the NestJS dependency injection system work? Trace how @Injectable() decorators and the injector resolve dependencies.",
                        "implement": "Add a request timing interceptor that measures how long each request takes and logs it. Find where interceptors are registered and add it there.",
                        "iterative_refinement": "Add a request timing interceptor that measures how long each request takes and adds an X-Response-Time header to the response.",
                    }
                    # Queries for openclaw (standard mode)
                    openclaw_queries = {
                        "narrow_fix": "What files handle WebSocket reconnection in this repo? List the file paths and briefly explain what each does.",
                        "cross_package": "How does a message flow from a webhook received by an extension to the core message handler in this repo? Trace the path through the key files.",
                        "understanding": "How does the plugin/extension loading system work in this repo? What are the key files and entry points?",
                        "data_flow": "How does authentication and token validation work in this repo? List the key files and describe the flow.",
                        "implement": "Implement a health check endpoint that returns JSON with the server version and uptime. Find where HTTP routes are registered and add it there.",
                        "implement_large": "Add a rate limiting system for incoming messages. Create a RateLimiter class that tracks per-channel message counts with a sliding window (default: 30 messages per 60 seconds). Integrate it into the message routing pipeline so that messages exceeding the limit are rejected with a user-friendly reply. Add configuration options to set custom limits per channel. Include unit tests.",
                    }
                    # Detect query set: openclaw has categories like narrow_fix/data_flow,
                    # nest (fast mode) has iterative_refinement. Use openclaw if any
                    # openclaw-only category is present, otherwise nest.
                    all_categories = {t.get("category") for t in task_list}
                    openclaw_only = {"narrow_fix", "cross_package", "data_flow", "implement_large"}
                    if all_categories & openclaw_only:
                        task_queries = openclaw_queries
                    else:
                        task_queries = nest_queries
                    query = task_queries.get(category)
                    if query:
                        key_files, all_suggested = get_pruner_suggestions(repo_path, query, pruner_bin)
                        hits = files_read & all_suggested
                        misses = files_read - all_suggested
                        unused = all_suggested - files_read

                        precision = len(hits) / len(all_suggested) * 100 if all_suggested else 0
                        recall = len(hits) / len(files_read) * 100 if files_read else 0

                        result.update({
                            "task": category,
                            "query": query,
                            "pruner_suggested": sorted(all_suggested),
                            "pruner_key_files": sorted(key_files),
                            "hits": sorted(hits),
                            "misses": sorted(misses),
                            "unused_suggestions": sorted(unused),
                            "files_created": sorted(files_written),
                            "precision": round(precision, 1),
                            "recall": round(recall, 1),
                        })

                        if verbose:
                            print("\n  R{}/{} [{}] — {}".format(
                                round_idx, category, side_label, category), file=sys.stderr)
                            print("  Pruner suggested {} files, Claude read {} files, created {} files".format(
                                len(all_suggested), len(files_read), len(files_written)), file=sys.stderr)
                            print("  Hits: {}  Misses: {}  Precision: {:.0f}%  Recall: {:.0f}%".format(
                                len(hits), len(misses), precision, recall), file=sys.stderr)

                results.append(result)
    return results


def find_jsonl_files(path):
    """Find all JSONL files in path (handles both files and directories)."""
    p = Path(path)
    if p.is_file():
        return [str(p)]

    files = []
    # Check for round*/ subdirectories
    round_dirs = sorted(p.glob("round*/"))
    if round_dirs:
        for rd in round_dirs:
            files.extend(sorted(str(f) for f in rd.glob("*.jsonl")))
    # Also check top-level JSONL files
    files.extend(sorted(str(f) for f in p.glob("*.jsonl")))
    return files


def print_summary(results, file=sys.stderr):
    """Print summary statistics."""
    with_results = [r for r in results if r["side"] == "with"]
    without_results = [r for r in results if r["side"] == "without"]

    print("\n" + "=" * 70, file=file)
    print("POST-HOC ANALYSIS SUMMARY", file=file)
    print("=" * 70, file=file)

    skipped = sum(1 for r in results if r.get("skipped_hit_rate"))
    print("\nFiles analyzed: {} with-pruner, {} without-pruner{}".format(
        len(with_results), len(without_results),
        " ({} follow-up turns skipped for hit rate)".format(skipped) if skipped else ""), file=file)

    # Navigation overhead comparison
    if with_results and without_results:
        wo_nav = sum(r["navigation_calls"] for r in without_results) / len(without_results)
        wp_nav = sum(r["navigation_calls"] for r in with_results) / len(with_results)
        wo_prod = sum(r["productive_calls"] for r in without_results) / len(without_results)
        wp_prod = sum(r["productive_calls"] for r in with_results) / len(with_results)
        wo_total = sum(r["total_tool_calls"] for r in without_results) / len(without_results)
        wp_total = sum(r["total_tool_calls"] for r in with_results) / len(with_results)
        wo_files = sum(r["files_used_count"] for r in without_results) / len(without_results)
        wp_files = sum(r["files_used_count"] for r in with_results) / len(with_results)

        print("\n{:<25} {:>12} {:>12} {:>8}".format(
            "", "Without", "With", "Delta"), file=file)
        print("-" * 60, file=file)
        print("{:<25} {:>12.1f} {:>12.1f} {:>+7.0f}%".format(
            "Total tool calls", wo_total, wp_total,
            (wp_total - wo_total) / wo_total * 100 if wo_total else 0), file=file)
        print("{:<25} {:>12.1f} {:>12.1f} {:>+7.0f}%".format(
            "Navigation calls", wo_nav, wp_nav,
            (wp_nav - wo_nav) / wo_nav * 100 if wo_nav else 0), file=file)
        print("{:<25} {:>12.1f} {:>12.1f} {:>+7.0f}%".format(
            "Productive calls", wo_prod, wp_prod,
            (wp_prod - wo_prod) / wo_prod * 100 if wo_prod else 0), file=file)
        print("{:<25} {:>12.1f} {:>12.1f} {:>+7.0f}%".format(
            "Unique files touched", wo_files, wp_files,
            (wp_files - wo_files) / wo_files * 100 if wo_files else 0), file=file)

        if wo_total > 0:
            wo_nav_pct = wo_nav / wo_total * 100
            wp_nav_pct = wp_nav / wp_total * 100 if wp_total else 0
            print("\n{:<25} {:>11.0f}% {:>11.0f}%".format(
                "Navigation %", wo_nav_pct, wp_nav_pct), file=file)

    # Hit rate analysis (only for with-pruner runs that have pruner data)
    hit_results = [r for r in with_results if "precision" in r]
    if hit_results:
        print("\n" + "-" * 60, file=file)
        print("PRUNER HIT RATE (with-pruner runs only)", file=file)
        print("-" * 60, file=file)

        precisions = [r["precision"] for r in hit_results]
        recalls = [r["recall"] for r in hit_results]
        n_suggested = [len(r["pruner_suggested"]) for r in hit_results]
        n_hits = [len(r["hits"]) for r in hit_results]
        n_misses = [len(r["misses"]) for r in hit_results]

        import statistics
        print("Precision (suggested & used / suggested):  mean={:.0f}%{}".format(
            statistics.mean(precisions),
            "  stdev={:.0f}pp".format(statistics.stdev(precisions)) if len(precisions) > 1 else ""), file=file)
        print("Recall (suggested & used / actually used): mean={:.0f}%{}".format(
            statistics.mean(recalls),
            "  stdev={:.0f}pp".format(statistics.stdev(recalls)) if len(recalls) > 1 else ""), file=file)
        print("Files suggested (mean):  {:.1f}".format(statistics.mean(n_suggested)), file=file)
        print("Files hit (mean):        {:.1f}".format(statistics.mean(n_hits)), file=file)
        print("Files missed (mean):     {:.1f}".format(statistics.mean(n_misses)), file=file)

        # Per-task breakdown
        tasks = {}
        for r in hit_results:
            task = r.get("task", "unknown")
            if task not in tasks:
                tasks[task] = {"precisions": [], "recalls": [], "misses": []}
            tasks[task]["precisions"].append(r["precision"])
            tasks[task]["recalls"].append(r["recall"])
            tasks[task]["misses"].append(len(r["misses"]))

        if len(tasks) > 1:
            print("\nPer-task breakdown:", file=file)
            for task, t in sorted(tasks.items()):
                print("  {}: precision={:.0f}%  recall={:.0f}%  misses={:.1f}  (N={})".format(
                    task, statistics.mean(t["precisions"]),
                    statistics.mean(t["recalls"]),
                    statistics.mean(t["misses"]),
                    len(t["precisions"])), file=file)

    # Files created by Claude (not expected in pruner suggestions)
    if hit_results:
        all_created = {}
        for r in hit_results:
            for f in r.get("files_created", []):
                all_created[f] = all_created.get(f, 0) + 1
        if all_created:
            n_with_creates = sum(1 for r in hit_results if r.get("files_created"))
            print("\nFiles created by Claude ({}/{} runs):".format(n_with_creates, len(hit_results)), file=file)
            for f, count in sorted(all_created.items(), key=lambda x: -x[1])[:5]:
                print("  {} ({}x)".format(f, count), file=file)

    # Common misses (files Claude needed but pruner didn't suggest)
    if hit_results:
        all_misses = {}
        for r in hit_results:
            for f in r.get("misses", []):
                all_misses[f] = all_misses.get(f, 0) + 1
        if all_misses:
            print("\nMost common misses (files Claude read, pruner missed):", file=file)
            for f, count in sorted(all_misses.items(), key=lambda x: -x[1])[:10]:
                print("  {} ({}x)".format(f, count), file=file)


def find_results_json_files(path):
    """Find results.json files (from --save-raw A/B tests)."""
    p = Path(path)
    if p.is_file() and p.name.endswith(".json"):
        return [str(p)]
    if p.is_dir():
        return sorted(str(f) for f in p.glob("*.json")
                       if f.name != "results.json"  # skip generic name, prefer specific
                       or not list(p.glob("fast_*.json")))
    return []


def main():
    args = parse_args()

    results = []
    p = Path(args.path)

    # Auto-detect: results.json files or raw JSONL logs
    if p.is_file() and p.suffix == ".json":
        # Single results.json file
        print("Analyzing results.json: {}".format(args.path), file=sys.stderr)
        results = analyze_results_json(args.path, repo_path=args.repo,
                                       pruner_bin=args.pruner, verbose=args.verbose)
    elif p.is_dir():
        # Check for both formats
        json_files = sorted(p.glob("*.json"))
        jsonl_files = find_jsonl_files(args.path)

        if json_files and not jsonl_files:
            # Only results.json files
            for jf in json_files:
                print("Analyzing results.json: {}".format(jf.name), file=sys.stderr)
                results.extend(analyze_results_json(str(jf), repo_path=args.repo,
                                                    pruner_bin=args.pruner, verbose=args.verbose))
        elif jsonl_files:
            # Raw JSONL logs (preferred — more detail)
            print("Found {} JSONL files".format(len(jsonl_files)), file=sys.stderr)
            for f in jsonl_files:
                r = analyze_log(f, repo_path=args.repo, pruner_bin=args.pruner, verbose=args.verbose)
                if r:
                    results.append(r)
        else:
            print("No JSONL or results.json files found at {}".format(args.path), file=sys.stderr)
            sys.exit(1)
    else:
        # Single JSONL file
        jsonl_files = find_jsonl_files(args.path)
        if not jsonl_files:
            print("No files found at {}".format(args.path), file=sys.stderr)
            sys.exit(1)
        print("Found {} JSONL files".format(len(jsonl_files)), file=sys.stderr)
        for f in jsonl_files:
            r = analyze_log(f, repo_path=args.repo, pruner_bin=args.pruner, verbose=args.verbose)
            if r:
                results.append(r)

    if results:
        print_summary(results)
        # JSON to stdout
        json.dump(results, sys.stdout, indent=2)
        print()


if __name__ == "__main__":
    main()
