#!/usr/bin/env python3
"""A/B test: real Claude Code sessions with and without pruner on a real repo.

Sets up two clones of the test repo:
  A — with pruner skill + CLAUDE.md instructions installed
  B — vanilla (no pruner)

Runs Claude Code (opus) on identical tasks in parallel, measures actual
token usage, tool calls, cost, and turns.

Usage:
    python3 tests/ab_test.py [/path/to/repo]

Requires:
  - `claude` CLI installed and logged in
  - `pruner` release binary built (cargo build --release)

Default repo: /tmp/pruner-bench/openclaw
"""

import subprocess, json, os, sys, time, shutil
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

REPO = sys.argv[1] if len(sys.argv) > 1 else "/tmp/pruner-bench/openclaw"
PRUNER_DIR = Path(__file__).resolve().parent.parent
PRUNER_BIN = PRUNER_DIR / "target" / "release" / "pruner"
SKILL_SRC = PRUNER_DIR / ".claude" / "skills" / "pruner" / "SKILL.md"
CLAUDE_TEMPLATE = PRUNER_DIR / "CLAUDE.template.md"

# Test workspace: two clones — one with pruner, one without
WORK_DIR = Path("/tmp/pruner-bench/ab-workspace")
CLONE_WITH = WORK_DIR / "with-pruner"
CLONE_WITHOUT = WORK_DIR / "without-pruner"

MODEL = "opus"
MAX_TURNS = 15

# Tasks to test — designed to require different exploration strategies.
# Prompts reference "this repo" since claude runs inside the clone.
TASKS = [
    (
        "narrow_fix",
        "What files handle WebSocket reconnection in this repo? "
        "List the file paths and briefly explain what each does.",
    ),
    (
        "cross_package",
        "How does a message flow from a webhook received by an extension "
        "to the core message handler in this repo? Trace the path through the key files.",
    ),
    (
        "understanding",
        "How does the plugin/extension loading system work in this repo? "
        "What are the key files and entry points?",
    ),
    (
        "data_flow",
        "How does authentication and token validation work in this repo? "
        "List the key files and describe the flow.",
    ),
]


def setup_clones():
    """Create two copies of the repo: one with pruner, one without."""
    WORK_DIR.mkdir(parents=True, exist_ok=True)

    for clone_path, label in [(CLONE_WITH, "with-pruner"), (CLONE_WITHOUT, "without-pruner")]:
        if clone_path.exists():
            print(f"  Reusing existing clone: {clone_path}", file=sys.stderr)
            continue
        print(f"  Copying {REPO} -> {clone_path} ...", file=sys.stderr)
        shutil.copytree(REPO, clone_path, symlinks=True,
                        ignore=shutil.ignore_patterns('.pruner'))

    # Install pruner in the "with" clone
    skill_dir = CLONE_WITH / ".claude" / "skills" / "pruner"
    skill_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(SKILL_SRC, skill_dir / "SKILL.md")

    # Append pruner instructions to CLAUDE.md
    claude_md = CLONE_WITH / "CLAUDE.md"
    template_text = CLAUDE_TEMPLATE.read_text()
    current = claude_md.read_text() if claude_md.exists() else ""
    if "pruner context" not in current:
        with open(claude_md, "a") as f:
            f.write("\n" + template_text)

    # Index the with-pruner clone
    print("  Indexing with-pruner clone ...", file=sys.stderr)
    subprocess.run(
        [str(PRUNER_BIN), "index", str(CLONE_WITH)],
        capture_output=True, check=True,
    )

    # Remove any pruner artifacts from the "without" clone
    for p in [CLONE_WITHOUT / ".claude" / "skills" / "pruner",
              CLONE_WITHOUT / ".pruner"]:
        if p.exists():
            shutil.rmtree(p)

    print("  Setup complete.", file=sys.stderr)


def run_claude(prompt, repo_dir, label=""):
    """Run claude -p inside the repo directory and return parsed results."""
    # Use a wrapper script to cd into the repo before running claude,
    # ensuring Claude picks up that repo's CLAUDE.md and skills.
    wrapper = WORK_DIR / "run_claude.sh"
    if not wrapper.exists():
        wrapper.write_text("#!/bin/bash\ncd \"$1\" && shift && exec claude \"$@\"\n")
        wrapper.chmod(0o755)

    args = [
        str(wrapper), str(repo_dir),
        "-p", prompt,
        "--output-format", "stream-json",
        "--verbose",
        "--max-turns", str(MAX_TURNS),
        "--model", MODEL,
        "--permission-mode", "bypassPermissions",
        "--no-session-persistence",
    ]

    start = time.time()
    proc = subprocess.run(args, capture_output=True, text=True, timeout=600)
    wall_time = time.time() - start

    tools = []
    result_data = None

    for line in proc.stdout.splitlines():
        try:
            d = json.loads(line)
        except json.JSONDecodeError:
            continue

        if d.get("type") == "assistant":
            for c in d.get("message", {}).get("content", []):
                if c.get("type") == "tool_use":
                    tools.append({
                        "name": c["name"],
                        "input_preview": str(c.get("input", {}))[:200],
                    })

        if d.get("type") == "result":
            result_data = d

    if not result_data:
        print(f"  WARN [{label}]: no result data", file=sys.stderr)
        return None

    u = result_data.get("usage", {})
    input_tokens = (
        u.get("input_tokens", 0)
        + u.get("cache_read_input_tokens", 0)
        + u.get("cache_creation_input_tokens", 0)
    )
    output_tokens = u.get("output_tokens", 0)

    return {
        "turns": result_data.get("num_turns", 0),
        "cost_usd": result_data.get("total_cost_usd", 0),
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": input_tokens + output_tokens,
        "tool_calls": len(tools),
        "tools": tools,
        "wall_time_s": round(wall_time, 1),
        "result_preview": result_data.get("result", "")[:300],
    }


def run_task(category, prompt):
    """Run one task with and without pruner (in parallel)."""
    print(f"\n=== [{category}] Starting both modes in parallel ===", file=sys.stderr)

    with ThreadPoolExecutor(max_workers=2) as pool:
        future_without = pool.submit(
            run_claude, prompt, CLONE_WITHOUT, f"{category}/without"
        )
        future_with = pool.submit(
            run_claude, prompt, CLONE_WITH, f"{category}/with"
        )

        without = future_without.result()
        with_p = future_with.result()

    if without:
        print(
            f"  WITHOUT: turns={without['turns']} tools={without['tool_calls']} "
            f"tokens={without['total_tokens']:,} cost=${without['cost_usd']:.4f} "
            f"time={without['wall_time_s']}s",
            file=sys.stderr,
        )
        for t in without["tools"]:
            print(f"    {t['name']}: {t['input_preview'][:80]}", file=sys.stderr)

    if with_p:
        print(
            f"  WITH:    turns={with_p['turns']} tools={with_p['tool_calls']} "
            f"tokens={with_p['total_tokens']:,} cost=${with_p['cost_usd']:.4f} "
            f"time={with_p['wall_time_s']}s",
            file=sys.stderr,
        )
        for t in with_p["tools"]:
            print(f"    {t['name']}: {t['input_preview'][:80]}", file=sys.stderr)

    return without, with_p


def main():
    assert shutil.which("claude"), "claude CLI not found"
    assert PRUNER_BIN.exists(), f"pruner not found at {PRUNER_BIN} — run cargo build --release"
    assert Path(REPO).is_dir(), f"repo not found at {REPO}"

    print("Setting up test clones ...", file=sys.stderr)
    setup_clones()

    results = []

    for category, prompt in TASKS:
        without, with_p = run_task(category, prompt)

        if without and with_p:
            token_delta = (
                (with_p["total_tokens"] - without["total_tokens"])
                / without["total_tokens"] * 100
                if without["total_tokens"] else 0
            )
            cost_delta = (
                (with_p["cost_usd"] - without["cost_usd"])
                / without["cost_usd"] * 100
                if without["cost_usd"] else 0
            )
            results.append({
                "category": category,
                "without": without,
                "with_pruner": with_p,
                "token_delta_pct": round(token_delta, 1),
                "cost_delta_pct": round(cost_delta, 1),
            })
        else:
            results.append({
                "category": category,
                "without": without,
                "with_pruner": with_p,
                "token_delta_pct": None,
                "cost_delta_pct": None,
            })

    # JSON to stdout
    print(json.dumps(results, indent=2))

    # Summary table to stderr
    print("\n=== Summary ===", file=sys.stderr)
    valid = [r for r in results if r["token_delta_pct"] is not None]
    print(
        f"{'Task':<16} {'W/O tokens':>12} {'W/ tokens':>12} {'Δ tokens':>10} "
        f"{'W/O cost':>10} {'W/ cost':>10} {'Δ cost':>10} "
        f"{'W/O tools':>10} {'W/ tools':>10}",
        file=sys.stderr,
    )
    print("-" * 112, file=sys.stderr)
    for r in valid:
        w = r["without"]
        p = r["with_pruner"]
        print(
            f"{r['category']:<16} {w['total_tokens']:>12,} {p['total_tokens']:>12,} "
            f"{r['token_delta_pct']:>+9.1f}% "
            f"${w['cost_usd']:>9.4f} ${p['cost_usd']:>9.4f} "
            f"{r['cost_delta_pct']:>+9.1f}% "
            f"{w['tool_calls']:>10} {p['tool_calls']:>10}",
            file=sys.stderr,
        )

    if valid:
        avg_token = sum(r["token_delta_pct"] for r in valid) / len(valid)
        avg_cost = sum(r["cost_delta_pct"] for r in valid) / len(valid)
        print("-" * 112, file=sys.stderr)
        print(
            f"{'Average':<16} {'':>12} {'':>12} {avg_token:>+9.1f}% "
            f"{'':>10} {'':>10} {avg_cost:>+9.1f}%",
            file=sys.stderr,
        )


if __name__ == "__main__":
    main()
