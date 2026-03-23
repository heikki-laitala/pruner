#!/usr/bin/env python3
"""A/B test: brief+read vs full-dump vs no-pruner (simulated) on a real repo.

Usage:
    python3 tests/ab_test.py [/path/to/repo]

Defaults to /tmp/pruner-bench/openclaw. Requires `pruner` release binary.
"""

import subprocess, json, re, os, sys

REPO = sys.argv[1] if len(sys.argv) > 1 else "/tmp/pruner-bench/openclaw"
BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../target/release/pruner")

QUERIES = [
    ("cross_package", "how does a message flow from webhook to channel handler"),
    ("narrow_fix", "fix WebSocket reconnection timeout"),
    ("understanding", "how does the skill execution pipeline work"),
    ("cross_cutting", "add correlation ID across middleware and handlers"),
    ("data_flow", "how does authentication token validation work"),
]


def estimate_tokens(text):
    return len(re.findall(r"\w+|[^\w\s]|\n", text))


def run_cmd(args):
    r = subprocess.run(args, capture_output=True, text=True, timeout=120)
    return r.stdout, r.stderr


def get_file_tokens(repo, path):
    """Read a file and estimate its tokens."""
    full = os.path.join(repo, path)
    try:
        with open(full) as f:
            return estimate_tokens(f.read())
    except Exception:
        return 0


def main():
    assert os.path.exists(BIN), f"pruner binary not found at {BIN} — run cargo build --release"
    assert os.path.isdir(REPO), f"repo not found at {REPO}"

    # Ensure indexed
    subprocess.run([BIN, "index", REPO], capture_output=True, check=True)

    results = []

    for category, query in QUERIES:
        print(f"\n=== [{category}] {query} ===", file=sys.stderr)

        # Strategy A: Brief + targeted reads
        brief_out, _ = run_cmd([BIN, "context", REPO, query, "--brief", "--format", "json"])
        brief_json = json.loads(brief_out)
        brief_tokens = estimate_tokens(brief_out)

        key_files = [f["path"] for f in brief_json["key_files"]]
        read_tokens = sum(get_file_tokens(REPO, p) for p in key_files)
        strategy_a = brief_tokens + read_tokens

        # Strategy B: Full context dump
        full_out, _ = run_cmd([BIN, "context", REPO, query, "--format", "json"])
        full_tokens = estimate_tokens(full_out)

        # Strategy C: No pruner — simulated vanilla Claude Code exploration
        # (glob for structure, grep for keywords, read relevant + some irrelevant files)
        est_out, _ = run_cmd([BIN, "estimate", REPO, query, "--json-output"])
        est = json.loads(est_out)
        no_pruner_tokens = est["without_pruner"]["total_tokens"]

        print(f"  Brief:         {brief_tokens:>6} tokens ({len(key_files)} files listed)", file=sys.stderr)
        print(f"  + File reads:  {read_tokens:>6} tokens", file=sys.stderr)
        print(f"  = Strategy A:  {strategy_a:>6} tokens (brief + read {len(key_files)} files)", file=sys.stderr)
        print(f"  Strategy B:    {full_tokens:>6} tokens (full context dump)", file=sys.stderr)
        print(f"  Strategy C:    {no_pruner_tokens:>6} tokens (no pruner, explore)", file=sys.stderr)

        a_vs_b = ((strategy_a - full_tokens) / full_tokens * 100) if full_tokens else 0
        a_vs_c = ((strategy_a - no_pruner_tokens) / no_pruner_tokens * 100) if no_pruner_tokens else 0
        b_vs_c = ((full_tokens - no_pruner_tokens) / no_pruner_tokens * 100) if no_pruner_tokens else 0

        results.append({
            "category": category,
            "query": query,
            "brief_tokens": brief_tokens,
            "read_tokens": read_tokens,
            "key_files": len(key_files),
            "strategy_a": strategy_a,
            "strategy_b": full_tokens,
            "strategy_c": no_pruner_tokens,
            "a_vs_b_pct": round(a_vs_b, 1),
            "a_vs_c_pct": round(a_vs_c, 1),
            "b_vs_c_pct": round(b_vs_c, 1),
        })

    # Print JSON results to stdout
    print(json.dumps(results, indent=2))

    # Print summary table to stderr
    print("\n=== Summary ===", file=sys.stderr)
    print(f"{'Category':<16} {'A (brief+read)':>14} {'B (full dump)':>14} {'C (no pruner)':>14} {'A vs B':>8} {'A vs C':>8}", file=sys.stderr)
    print("-" * 82, file=sys.stderr)
    for r in results:
        print(
            f"{r['category']:<16} {r['strategy_a']:>14,} {r['strategy_b']:>14,} {r['strategy_c']:>14,} {r['a_vs_b_pct']:>+7.1f}% {r['a_vs_c_pct']:>+7.1f}%",
            file=sys.stderr,
        )

    avg_a = sum(r["strategy_a"] for r in results) // len(results)
    avg_b = sum(r["strategy_b"] for r in results) // len(results)
    avg_c = sum(r["strategy_c"] for r in results) // len(results)
    avg_a_vs_b = sum(r["a_vs_b_pct"] for r in results) / len(results)
    avg_a_vs_c = sum(r["a_vs_c_pct"] for r in results) / len(results)
    print("-" * 82, file=sys.stderr)
    print(
        f"{'Average':<16} {avg_a:>14,} {avg_b:>14,} {avg_c:>14,} {avg_a_vs_b:>+7.1f}% {avg_a_vs_c:>+7.1f}%",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
