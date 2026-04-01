#!/usr/bin/env python3
"""A/B test: real Claude Code sessions with and without pruner on a real repo.

Sets up two clones of the test repo:
  A — with pruner installed (hook or skill mode)
  B — vanilla (no pruner)

Runs Claude Code (opus) on identical tasks sequentially, measures actual
token usage, tool calls, cost, and turns.

Usage:
    python3 tests/ab_test.py [options] [/path/to/repo]

    --task TASK          Run only this task
    --mode hook|skill    Pruner delivery: hook (prompt-submit) or skill (tool call)
    --only with|without  Run only one side
    --save-raw           Save raw stream-json output to /tmp/pruner-bench/ab-raw/

Requires:
  - `claude` CLI installed and logged in
  - `pruner` release binary built (cargo build --release)

Default repo: /tmp/pruner-bench/openclaw
"""

import subprocess, json, os, sys, time, shutil, argparse, random
from pathlib import Path

PRUNER_DIR = Path(__file__).resolve().parent.parent
PRUNER_BIN = PRUNER_DIR / "target" / "release" / "pruner"
SKILL_HOOK_SRC = PRUNER_DIR / ".claude" / "skills" / "pruner" / "SKILL.hook.md"
SKILL_SKILL_SRC = PRUNER_DIR / ".claude" / "skills" / "pruner" / "SKILL.skill.md"
HOOK_SRC = PRUNER_DIR / ".claude" / "hooks" / "pruner-context.sh"
CLAUDE_TEMPLATE = PRUNER_DIR / "CLAUDE.template.md"

WORK_DIR = Path("/tmp/pruner-bench/ab-workspace")
RAW_DIR = Path("/tmp/pruner-bench/ab-raw")
CLONE_WITH = WORK_DIR / "with-pruner"
CLONE_WITHOUT = WORK_DIR / "without-pruner"

MODEL = "opus"
MAX_TURNS = 15
PINNED_COMMIT = "fb602c9b02014ec9a8bc256c149b39861c1435ab"

TASKS = {
    "narrow_fix": (
        "What files handle WebSocket reconnection in this repo? "
        "List the file paths and briefly explain what each does."
    ),
    "cross_package": (
        "How does a message flow from a webhook received by an extension "
        "to the core message handler in this repo? Trace the path through the key files."
    ),
    "understanding": (
        "How does the plugin/extension loading system work in this repo? "
        "What are the key files and entry points?"
    ),
    "data_flow": (
        "How does authentication and token validation work in this repo? "
        "List the key files and describe the flow."
    ),
    "implement": (
        "Implement a health check endpoint that returns JSON with the server version "
        "and uptime. Find where HTTP routes are registered and add it there."
    ),
    "implement_large": (
        "Add a rate limiting system for incoming messages. Create a RateLimiter class "
        "that tracks per-channel message counts with a sliding window (default: 30 messages "
        "per 60 seconds). Integrate it into the message routing pipeline so that messages "
        "exceeding the limit are rejected with a user-friendly reply. Add configuration "
        "options to set custom limits per channel. Include unit tests."
    ),
}


def parse_args():
    parser = argparse.ArgumentParser(description="A/B test pruner with real Claude Code sessions")
    parser.add_argument("repo", nargs="?", default="/tmp/pruner-bench/openclaw",
                        help="Path to test repo (default: /tmp/pruner-bench/openclaw)")
    parser.add_argument("--task", choices=list(TASKS.keys()),
                        help="Run only this task")
    parser.add_argument("--only", choices=["with", "without"],
                        help="Run only one side (with or without pruner)")
    parser.add_argument("--mode", choices=["hook", "skill"], default="hook",
                        help="Pruner delivery mode: hook (prompt-submit) or skill (tool call)")
    parser.add_argument("--save-raw", action="store_true",
                        help="Save raw stream-json output for analysis")
    parser.add_argument("--validate-cache", action="store_true",
                        help="Warn if cache hit rates differ >10%% between paired runs")
    return parser.parse_args()


def setup_clones(repo, mode="hook"):
    """Create two copies of the repo: one with pruner, one without."""
    WORK_DIR.mkdir(parents=True, exist_ok=True)

    for clone_path, label in [(CLONE_WITH, "with-pruner"), (CLONE_WITHOUT, "without-pruner")]:
        if clone_path.exists():
            print(f"  Reusing existing clone: {clone_path}", file=sys.stderr)
        else:
            print(f"  Copying {repo} -> {clone_path} ...", file=sys.stderr)
            shutil.copytree(repo, clone_path, symlinks=True,
                            ignore=shutil.ignore_patterns('.pruner'))
        # Reset to pinned commit for reproducibility
        subprocess.run(["git", "checkout", PINNED_COMMIT], cwd=clone_path,
                        capture_output=True, check=True)
        subprocess.run(["git", "checkout", "."], cwd=clone_path,
                        capture_output=True, check=True)
        subprocess.run(["git", "clean", "-fd"], cwd=clone_path,
                        capture_output=True, check=True)

    print(f"  Pruner mode: {mode}", file=sys.stderr)

    # Clean previous pruner setup from the "with" clone
    for p in [CLONE_WITH / ".claude" / "skills" / "pruner",
              CLONE_WITH / ".claude" / "hooks"]:
        if p.exists():
            shutil.rmtree(p)
    with_settings_file = CLONE_WITH / ".claude" / "settings.json"
    if with_settings_file.exists():
        s = json.loads(with_settings_file.read_text())
        s.pop("hooks", None)
        with_settings_file.write_text(json.dumps(s, indent=2))

    # Remove old pruner instructions from CLAUDE.md
    claude_md = CLONE_WITH / "CLAUDE.md"
    if claude_md.exists():
        text = claude_md.read_text()
        marker = "## Pruner"
        idx = text.find(marker)
        if idx >= 0:
            claude_md.write_text(text[:idx].rstrip() + "\n")

    if mode == "hook":
        # Install hook
        hook_dir = CLONE_WITH / ".claude" / "hooks"
        hook_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(HOOK_SRC, hook_dir / "pruner-context.sh")
        (hook_dir / "pruner-context.sh").chmod(0o755)

        # Install hook settings
        settings = {}
        if with_settings_file.exists():
            settings = json.loads(with_settings_file.read_text())
        settings["hooks"] = {
            "UserPromptSubmit": [
                {
                    "matcher": "",
                    "hooks": [
                        {
                            "type": "command",
                            "command": str(hook_dir / "pruner-context.sh"),
                            "timeout": 60,
                        }
                    ],
                }
            ]
        }
        with_settings_file.write_text(json.dumps(settings, indent=2))

        # Install hook-mode skill
        skill_dir = CLONE_WITH / ".claude" / "skills" / "pruner"
        skill_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(SKILL_HOOK_SRC, skill_dir / "SKILL.md")

    elif mode == "skill":
        # Install skill-mode skill (Claude calls pruner as a tool)
        skill_dir = CLONE_WITH / ".claude" / "skills" / "pruner"
        skill_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(SKILL_SKILL_SRC, skill_dir / "SKILL.md")

    # Append pruner instructions to CLAUDE.md
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
              CLONE_WITHOUT / ".claude" / "hooks",
              CLONE_WITHOUT / ".pruner"]:
        if p.exists():
            shutil.rmtree(p)
    without_settings = CLONE_WITHOUT / ".claude" / "settings.json"
    if without_settings.exists():
        s = json.loads(without_settings.read_text())
        s.pop("hooks", None)
        without_settings.write_text(json.dumps(s, indent=2))

    print("  Setup complete.", file=sys.stderr)


def parse_stream(stdout, label="", save_path=None):
    """Parse stream-json output into structured results with per-turn breakdown."""
    if save_path:
        save_path.parent.mkdir(parents=True, exist_ok=True)
        save_path.write_text(stdout)
        print(f"  Raw output saved to {save_path}", file=sys.stderr)

    tools = []
    turns = []  # per-turn breakdown
    current_turn_tools = []
    current_turn_num = 0
    result_data = None

    # Track per-message token usage from usage events
    per_message_usage = []

    for line in stdout.splitlines():
        try:
            d = json.loads(line)
        except json.JSONDecodeError:
            continue

        # Track assistant messages and their tool calls
        if d.get("type") == "assistant":
            msg = d.get("message", {})
            usage = msg.get("usage", {})

            # Save per-turn tools from previous turn
            if current_turn_tools:
                turns.append({
                    "turn": current_turn_num,
                    "tools": current_turn_tools,
                })
            current_turn_num += 1
            current_turn_tools = []

            for c in msg.get("content", []):
                if c.get("type") == "tool_use":
                    tool_info = {
                        "name": c["name"],
                        "input_preview": str(c.get("input", {}))[:300],
                    }
                    tools.append(tool_info)
                    current_turn_tools.append(tool_info)

            if usage:
                per_message_usage.append({
                    "turn": current_turn_num,
                    "input_tokens": usage.get("input_tokens", 0),
                    "output_tokens": usage.get("output_tokens", 0),
                    "cache_read": usage.get("cache_read_input_tokens", 0),
                    "cache_creation": usage.get("cache_creation_input_tokens", 0),
                })

        if d.get("type") == "result":
            result_data = d

    # Don't forget last turn
    if current_turn_tools:
        turns.append({
            "turn": current_turn_num,
            "tools": current_turn_tools,
        })

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
        "per_turn": turns,
        "per_message_usage": per_message_usage,
        "result_preview": result_data.get("result", "")[:500],
    }


def ensure_pruner_on_path():
    """Create a bin directory with a 'pruner' symlink to the release binary.

    Returns the directory path.  This ensures the hook script finds our
    freshly-built binary first, regardless of what is installed system-wide.
    """
    bin_dir = WORK_DIR / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    link = bin_dir / "pruner"
    if link.exists() or link.is_symlink():
        link.unlink()
    link.symlink_to(PRUNER_BIN.resolve())
    return bin_dir


def compute_cache_hit_rate(result):
    """Compute first-turn cache hit rate from per_message_usage.

    Returns the fraction of first-turn input tokens that came from cache.
    The first turn is the best signal for whether the prompt prefix was warm,
    since it sends the system prompt + tool schemas before any conversation.

    Mutates result dict to add 'first_turn_cache_rate' key.
    Returns the rate (float 0.0-1.0) or None if no usage data.
    """
    pmu = result.get("per_message_usage", [])
    if not pmu:
        return None
    first = pmu[0]
    total = first["input_tokens"] + first["cache_read"] + first["cache_creation"]
    if total == 0:
        return None
    rate = first["cache_read"] / total
    result["first_turn_cache_rate"] = rate
    return rate


def validate_cache_symmetry(results, threshold=0.10):
    """Check if cache hit rates differ significantly between paired runs.

    Returns a list of warning strings for pairs where the absolute difference
    in first_turn_cache_rate exceeds the threshold.
    """
    warnings = []
    for entry in results:
        without = entry.get("without")
        with_p = entry.get("with_pruner")
        if not without or not with_p:
            continue
        rate_wo = without.get("first_turn_cache_rate")
        rate_w = with_p.get("first_turn_cache_rate")
        if rate_wo is None or rate_w is None:
            continue
        diff = abs(rate_w - rate_wo)
        if diff > threshold:
            warnings.append(
                f"{entry['category']}: cache rate diff={diff:.0%} "
                f"(without={rate_wo:.0%}, with={rate_w:.0%})"
            )
    return warnings


def warmup_cache(repo_dir):
    """Run a minimal throwaway Claude prompt to prime the prompt cache.

    Anthropic's prompt cache (~1 hour TTL) caches the system prompt + tool
    schemas prefix. This warmup ensures both sides start with equally warm
    caches, eliminating asymmetric first-call cache advantages.
    """
    wrapper = WORK_DIR / "run_claude.sh"
    if not wrapper.exists():
        wrapper.write_text("#!/bin/bash\ncd \"$1\" && shift && exec claude \"$@\"\n")
        wrapper.chmod(0o755)

    bin_dir = ensure_pruner_on_path()
    env = os.environ.copy()
    env["PATH"] = f"{bin_dir}:{env.get('PATH', '')}"

    label = Path(repo_dir).name
    print(f"  Cache warmup [{label}] ...", file=sys.stderr)

    try:
        subprocess.run(
            [str(wrapper), str(repo_dir),
             "-p", "hello",
             "--output-format", "stream-json",
             "--max-turns", "1",
             "--model", MODEL,
             "--permission-mode", "bypassPermissions",
             "--no-session-persistence"],
            capture_output=True, text=True, timeout=120, env=env,
        )
    except subprocess.TimeoutExpired:
        print(f"  Cache warmup [{label}] timed out (continuing)", file=sys.stderr)


def run_claude(prompt, repo_dir, label="", save_raw=False):
    """Run claude -p inside the repo directory and return parsed results."""
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

    # Prepend our release binary to PATH so the hook script uses it
    bin_dir = ensure_pruner_on_path()
    env = os.environ.copy()
    env["PATH"] = f"{bin_dir}:{env.get('PATH', '')}"

    print(f"  Starting [{label}] ...", file=sys.stderr)
    start = time.time()
    proc = subprocess.run(args, capture_output=True, text=True, timeout=600, env=env)
    wall_time = time.time() - start

    save_path = RAW_DIR / f"{label.replace('/', '_')}.jsonl" if save_raw else None

    result = parse_stream(proc.stdout, label, save_path)
    if result:
        result["wall_time_s"] = round(wall_time, 1)
    elif proc.stdout.strip():
        # Save raw output on failure for debugging
        fail_path = RAW_DIR / f"{label.replace('/', '_')}_FAILED.jsonl"
        fail_path.parent.mkdir(parents=True, exist_ok=True)
        fail_path.write_text(proc.stdout)
        print(f"  WARN [{label}]: no result, raw saved to {fail_path}", file=sys.stderr)

    return result


def print_detailed_results(label, data):
    """Print detailed per-turn breakdown."""
    if not data:
        print(f"\n  [{label}] No data", file=sys.stderr)
        return

    cache_str = ""
    cache_rate = data.get("first_turn_cache_rate")
    if cache_rate is not None:
        cache_str = f" cache={cache_rate:.0%}"
    print(f"\n  [{label}] Summary: turns={data['turns']} tools={data['tool_calls']} "
          f"tokens={data['total_tokens']:,} (in={data['input_tokens']:,} out={data['output_tokens']:,}) "
          f"cost=${data['cost_usd']:.4f} time={data['wall_time_s']}s{cache_str}",
          file=sys.stderr)

    # Per-turn tool breakdown
    print(f"\n  [{label}] Per-turn tools:", file=sys.stderr)
    for turn in data["per_turn"]:
        tool_names = [t["name"] for t in turn["tools"]]
        print(f"    Turn {turn['turn']}: {', '.join(tool_names)}", file=sys.stderr)

    # Per-message token growth
    if data["per_message_usage"]:
        print(f"\n  [{label}] Per-message input tokens (shows context growth):", file=sys.stderr)
        for u in data["per_message_usage"]:
            total_in = u["input_tokens"] + u["cache_read"] + u["cache_creation"]
            print(f"    Turn {u['turn']}: {total_in:,} input "
                  f"(fresh={u['input_tokens']:,} cache_read={u['cache_read']:,} "
                  f"cache_create={u['cache_creation']:,}) "
                  f"+ {u['output_tokens']:,} output",
                  file=sys.stderr)

    # Full tool call details
    print(f"\n  [{label}] All tool calls:", file=sys.stderr)
    for i, t in enumerate(data["tools"], 1):
        print(f"    {i}. {t['name']}: {t['input_preview'][:120]}", file=sys.stderr)


def reset_clone(clone_path, reinstall_pruner=False, mode="hook"):
    """Reset clone to pinned commit, discarding any changes from the previous task.

    If reinstall_pruner is True, re-runs pruner init since git clean removes untracked files.
    """
    subprocess.run(["git", "checkout", "."], cwd=clone_path, capture_output=True, check=True)
    subprocess.run(["git", "clean", "-fd"], cwd=clone_path, capture_output=True, check=True)
    if reinstall_pruner:
        init_args = [str(PRUNER_BIN), "init", str(clone_path)]
        if mode == "hook":
            init_args.append("--hook")
        subprocess.run(init_args, check=True, capture_output=True, text=True)
        # Re-index: git clean removes .pruner/ (including the index DB).
        # Without an index, the hook's pruner context call auto-indexes the
        # entire repo, exceeding the 60s hook timeout on large repos.
        subprocess.run(
            [str(PRUNER_BIN), "index", str(clone_path)],
            check=True, capture_output=True, text=True,
        )


def run_single(category, prompt, side, mode="hook", save_raw=False):
    """Run one side (with or without) of one task."""
    print(f"\n{'='*60}", file=sys.stderr)
    print(f"  Task: {category} [{side}]", file=sys.stderr)
    print(f"{'='*60}", file=sys.stderr)

    if side == "with":
        reset_clone(CLONE_WITH, reinstall_pruner=True, mode=mode)
        warmup_cache(CLONE_WITH)
        result = run_claude(prompt, CLONE_WITH, f"{category}/with", save_raw)
    else:
        reset_clone(CLONE_WITHOUT)
        warmup_cache(CLONE_WITHOUT)
        result = run_claude(prompt, CLONE_WITHOUT, f"{category}/without", save_raw)

    if result:
        compute_cache_hit_rate(result)
        print_detailed_results(f"{category}/{side}", result)
    return result


def interleaved_schedule(tasks, only=None):
    """Build a randomized run schedule where same-scenario runs are never adjacent.

    Each task produces two runs (with/without). The schedule interleaves them
    so that Anthropic's prompt cache (~1 hour TTL for eligible users) has less
    opportunity for cross-scenario cache contamination.
    """
    runs = []
    for category, prompt in tasks:
        if only != "with":
            runs.append((category, prompt, "without"))
        if only != "without":
            runs.append((category, prompt, "with"))

    # Shuffle with constraint: same category cannot be adjacent
    for _ in range(200):
        random.shuffle(runs)
        valid = True
        for i in range(1, len(runs)):
            if runs[i][0] == runs[i - 1][0]:
                valid = False
                break
        if valid:
            break
    else:
        # Fallback: deterministic interleave (with/without alternating)
        runs.sort(key=lambda r: (r[2], r[0]))

    return runs


def print_summary(results):
    """Print comparison summary table."""
    print(f"\n{'='*60}", file=sys.stderr)
    print("  SUMMARY", file=sys.stderr)
    print(f"{'='*60}", file=sys.stderr)

    valid = [r for r in results if r.get("token_delta_pct") is not None]
    if not valid:
        # Single-side results
        for r in results:
            side = r.get("without") or r.get("with_pruner")
            if side:
                label = "without" if r.get("without") else "with"
                print(f"  {r['category']:<16} [{label}] tokens={side['total_tokens']:,} "
                      f"cost=${side['cost_usd']:.4f} tools={side['tool_calls']} "
                      f"time={side['wall_time_s']}s",
                      file=sys.stderr)
        return

    print(
        f"  {'Task':<16} {'W/O tokens':>12} {'W/ tokens':>12} {'Δ tok':>8} "
        f"{'W/O cost':>10} {'W/ cost':>10} {'Δ cost':>8} "
        f"{'W/O tools':>10} {'W/ tools':>10} {'Δ time':>10} "
        f"{'Cache W/O':>10} {'Cache W/':>10}",
        file=sys.stderr,
    )
    print("  " + "-" * 140, file=sys.stderr)
    for r in valid:
        w = r["without"]
        p = r["with_pruner"]
        time_delta = ""
        if w["wall_time_s"] and p["wall_time_s"]:
            td = (p["wall_time_s"] - w["wall_time_s"]) / w["wall_time_s"] * 100
            time_delta = f"{td:+.0f}%"
        cache_wo = w.get("first_turn_cache_rate")
        cache_w = p.get("first_turn_cache_rate")
        cache_wo_str = f"{cache_wo:.0%}" if cache_wo is not None else "N/A"
        cache_w_str = f"{cache_w:.0%}" if cache_w is not None else "N/A"
        print(
            f"  {r['category']:<16} {w['total_tokens']:>12,} {p['total_tokens']:>12,} "
            f"{r['token_delta_pct']:>+7.0f}% "
            f"${w['cost_usd']:>9.4f} ${p['cost_usd']:>9.4f} "
            f"{r['cost_delta_pct']:>+7.0f}% "
            f"{w['tool_calls']:>10} {p['tool_calls']:>10} "
            f"{time_delta:>10} "
            f"{cache_wo_str:>10} {cache_w_str:>10}",
            file=sys.stderr,
        )


def main():
    args = parse_args()

    assert shutil.which("claude"), "claude CLI not found"
    assert PRUNER_BIN.exists(), f"pruner not found at {PRUNER_BIN} — run cargo build --release"
    assert Path(args.repo).is_dir(), f"repo not found at {args.repo}"

    print(f"Setting up test clones (mode={args.mode}) ...", file=sys.stderr)
    setup_clones(args.repo, mode=args.mode)

    # Select tasks
    if args.task:
        tasks = [(args.task, TASKS[args.task])]
    else:
        tasks = list(TASKS.items())

    # Build interleaved schedule: randomized order, same scenario never adjacent
    schedule = interleaved_schedule(tasks, only=args.only)
    print(f"\nRun schedule ({len(schedule)} runs):", file=sys.stderr)
    for i, (cat, _, side) in enumerate(schedule):
        print(f"  {i+1}. {cat} [{side}]", file=sys.stderr)

    # Run all experiments
    run_results = {}  # category -> {"without": ..., "with": ...}
    for category, prompt, side in schedule:
        result = run_single(category, prompt, side, mode=args.mode, save_raw=args.save_raw)
        run_results.setdefault(category, {})[side] = result

    # Assemble results
    results = []
    for category, _ in tasks:
        data = run_results.get(category, {})
        without = data.get("without")
        with_p = data.get("with")
        entry = {"category": category, "without": without, "with_pruner": with_p}
        if without and with_p:
            entry["token_delta_pct"] = round(
                ((with_p["total_tokens"] - without["total_tokens"])
                 / without["total_tokens"] * 100)
                if without["total_tokens"] else 0, 1
            )
            entry["cost_delta_pct"] = round(
                ((with_p["cost_usd"] - without["cost_usd"])
                 / without["cost_usd"] * 100)
                if without["cost_usd"] else 0, 1
            )
        results.append(entry)

    # JSON to stdout
    print(json.dumps(results, indent=2))

    # Summary to stderr
    print_summary(results)

    # Cache symmetry validation
    if args.validate_cache:
        cache_warnings = validate_cache_symmetry(results)
        if cache_warnings:
            print(f"\n  CACHE WARNINGS:", file=sys.stderr)
            for w in cache_warnings:
                print(f"    {w}", file=sys.stderr)
        else:
            print(f"\n  Cache symmetry OK (all pairs within 10%)", file=sys.stderr)


if __name__ == "__main__":
    main()
