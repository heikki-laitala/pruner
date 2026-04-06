#!/usr/bin/env python3
"""A/B test: real Claude Code sessions with and without pruner on a real repo.

Sets up two clones of the test repo:
  A — with pruner installed (hook or skill mode)
  B — vanilla (no pruner)

Runs Claude Code on identical tasks, measures actual token usage,
tool calls, cost, and turns. Four modes: standard (with vs without pruner),
branch comparison (baseline vs feature pruner binary), interactive
(multi-turn conversations), and fast (sonnet + smaller repo for
quick iteration).

Examples:

    # Single-turn: all tasks, hook mode, interleaved schedule
    python3 tests/ab_test.py --save-raw

    # Fast iteration: sonnet + nest repo, 3 rounds
    python3 tests/ab_test.py --fast --rounds 3 --save-raw

    # Fast multi-turn: 5 rounds with cache validation
    python3 tests/ab_test.py --fast --interactive --rounds 5 --validate-cache --save-raw

    # Single-turn: one task, both sides
    python3 tests/ab_test.py --task narrow_fix --save-raw

    # Single-turn: one side only
    python3 tests/ab_test.py --task narrow_fix --only with --save-raw

    # Single-turn: skill mode instead of hook
    python3 tests/ab_test.py --task cross_package --mode skill --save-raw

    # Single-turn: custom repo
    python3 tests/ab_test.py /path/to/repo --task narrow_fix --save-raw

    # Override model (use sonnet with default repo)
    python3 tests/ab_test.py --model sonnet --task narrow_fix --save-raw

    # Branch comparison: main (baseline) vs current worktree (feature)
    python3 tests/ab_test.py --baseline-branch main --task narrow_fix --save-raw

    # Multi-turn: interactive conversation scenarios (3 user turns each)
    python3 tests/ab_test.py --interactive --save-raw

    # Multi-turn: single scenario
    python3 tests/ab_test.py --interactive --task implement_feedback_fix --save-raw

    # Multi-turn + branch comparison
    python3 tests/ab_test.py --interactive --baseline-branch main --task debug_clarify_resolve --save-raw

    # Cache validation (warn if cache hit rates differ >10% between sides)
    python3 tests/ab_test.py --task narrow_fix --validate-cache --save-raw

    # Unit tests (no claude CLI needed)
    uv run --with pytest pytest tests/test_ab_test.py -v

Options:
    --task TASK            Run only this task
    --mode hook|skill      Pruner delivery: hook (prompt-submit) or skill (tool call)
    --only SIDE            Run only one side (with/without or baseline/feature)
    --baseline-branch REF  Compare pruner from REF vs current worktree (feature)
    --interactive           Run interactive (multi-turn) conversation scenarios
    --fast                 Use sonnet model + smaller repo (nest) for quick iteration
    --model MODEL          Override model (default: opus, or sonnet with --fast)
    --rounds N             Run the full A/B test N times (default: 1)
    --save-raw             Save raw stream-json output (per-round subdirs with --rounds)
    --validate-cache       Warn if cache hit rates differ >10% between paired runs

Output (with --save-raw):
    /tmp/pruner-bench/ab-raw/{model}_{repo}_{mode}_n{rounds}_{timestamp}.json
    /tmp/pruner-bench/ab-raw/*.jsonl                Raw JSONL (single round)
    /tmp/pruner-bench/ab-raw/round0/*.jsonl         Raw JSONL (multi-round)

Requires:
  - `claude` CLI installed and logged in
  - `pruner` release binary built (cargo build --release)

Default repo: openclaw (~9.8K files) or nestjs/nest (~2.1K files with --fast)
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

CLONE_BASELINE = WORK_DIR / "baseline-pruner"
CLONE_FEATURE = WORK_DIR / "feature-pruner"
BASELINE_BIN_DIR = WORK_DIR / "baseline-bin"

MODEL = "opus"
MAX_TURNS = 15

# Default repo: openclaw (~9.8K files, TypeScript monorepo)
DEFAULT_REPO = "/tmp/pruner-bench/openclaw"
DEFAULT_REPO_URL = "https://github.com/openclaw/openclaw.git"
DEFAULT_PINNED_COMMIT = "fb602c9b02014ec9a8bc256c149b39861c1435ab"

# Fast repo: nestjs/nest (~2.1K files, TypeScript monorepo)
FAST_REPO = "/tmp/pruner-bench/nest"
FAST_REPO_URL = "https://github.com/nestjs/nest.git"
FAST_PINNED_COMMIT = "416830c3924b37ec354d4e15c14119519e389afc"  # v10.4.9

PINNED_COMMIT = DEFAULT_PINNED_COMMIT

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

# Tasks for --fast mode (nestjs/nest repo, ~2.1K TS files)
FAST_TASKS = {
    "understanding": (
        "How does the NestJS dependency injection system work? "
        "Trace how @Injectable() decorators and the injector resolve dependencies."
    ),
    "implement": (
        "Add a request timing interceptor that measures how long each request takes "
        "and logs it. Find where interceptors are registered and add it there."
    ),
}

FAST_MULTI_TURN_TASKS = {
    "iterative_refinement": [
        "Add a request timing interceptor that measures how long each request takes "
        "and adds an X-Response-Time header to the response.",
        "Make the header name configurable via an options object passed to the interceptor.",
        "Add a threshold option so the header is only added when response time exceeds it.",
    ],
}

MULTI_TURN_TASKS = {
    "implement_feedback_fix": [
        "Add a health check endpoint that returns JSON with server version and uptime. "
        "Find where HTTP routes are registered and add it there.",
        "The health check should use the existing Express app instance, not create a new "
        "HTTP server. Also return the Node.js version in the response.",
        "Add unit tests for the health check endpoint.",
    ],
    "debug_clarify_resolve": [
        "Why does authentication fail when the token has expired? "
        "Trace the auth flow and identify where expiration is checked.",
        "I mean the JWT validation logic specifically, not the login form. "
        "Which file handles token verification and what library does it use?",
        "Fix the token validation to return a clear 401 error with a message "
        "saying the token has expired, instead of a generic 500.",
    ],
    "iterative_refinement": [
        "Add a rate limiting system for incoming messages. Create a RateLimiter class "
        "that tracks per-channel message counts with a sliding window of 30 messages "
        "per 60 seconds.",
        "Make the rate limits configurable per channel via a config object passed to "
        "the constructor.",
        "Add logging that records when a channel hits its rate limit, including the "
        "channel ID and the current message count.",
    ],
}


def build_pruner_from_ref(ref, output_dir):
    """Build pruner from a git ref using git worktree.

    Creates a temporary worktree at the given ref, runs cargo build --release,
    copies the binary to output_dir/pruner, and cleans up the worktree.

    Returns Path to the built binary.
    """
    worktree_dir = WORK_DIR / "worktree-baseline"
    WORK_DIR.mkdir(parents=True, exist_ok=True)
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    # Clean up any existing worktree
    if worktree_dir.exists():
        subprocess.run(
            ["git", "worktree", "remove", "--force", str(worktree_dir)],
            cwd=PRUNER_DIR, capture_output=True,
        )
        # If worktree remove failed (e.g., not a worktree), just delete
        if worktree_dir.exists():
            shutil.rmtree(worktree_dir)

    # Also prune stale worktree entries
    subprocess.run(
        ["git", "worktree", "prune"],
        cwd=PRUNER_DIR, capture_output=True,
    )

    try:
        # Create worktree at the given ref
        print(f"  Creating worktree for {ref} ...", file=sys.stderr)
        subprocess.run(
            ["git", "worktree", "add", str(worktree_dir), ref],
            cwd=PRUNER_DIR, capture_output=True, check=True,
        )

        # Build release binary in the worktree
        print(f"  Building pruner from {ref} ...", file=sys.stderr)
        subprocess.run(
            ["cargo", "build", "--release"],
            cwd=worktree_dir, check=True,
        )

        # Copy binary to output directory
        built_binary = worktree_dir / "target" / "release" / "pruner"
        dest = output_dir / "pruner"
        shutil.copy2(built_binary, dest)
        print(f"  Baseline binary ready: {dest}", file=sys.stderr)
        return dest

    finally:
        # Always clean up the worktree
        if worktree_dir.exists():
            subprocess.run(
                ["git", "worktree", "remove", "--force", str(worktree_dir)],
                cwd=PRUNER_DIR, capture_output=True,
            )


def parse_args():
    parser = argparse.ArgumentParser(description="A/B test pruner with real Claude Code sessions")
    parser.add_argument("repo", nargs="?", default=None,
                        help="Path to test repo (default: openclaw, or express with --fast)")
    parser.add_argument("--task",
                        help="Run only this task")
    parser.add_argument("--only", choices=["with", "without", "baseline", "feature"],
                        help="Run only one side (with/without in standard mode, "
                             "baseline/feature in branch mode)")
    parser.add_argument("--mode", choices=["hook", "skill"], default="hook",
                        help="Pruner delivery mode: hook (prompt-submit) or skill (tool call)")
    parser.add_argument("--save-raw", action="store_true",
                        help="Save raw stream-json output for analysis")
    parser.add_argument("--validate-cache", action="store_true",
                        help="Warn if cache hit rates differ >10%% between paired runs")
    parser.add_argument("--baseline-branch", metavar="REF",
                        help="Compare pruner from REF (baseline) vs current worktree (feature)")
    parser.add_argument("--interactive", "--multi-turn", action="store_true",
                        dest="interactive",
                        help="Run interactive (multi-turn) conversation scenarios")
    parser.add_argument("--fast", action="store_true",
                        help="Fast iteration mode: use sonnet model and smaller repo (express)")
    parser.add_argument("--model", default=None,
                        help="Claude model to use (default: opus, or sonnet with --fast)")
    parser.add_argument("--rounds", type=int, default=1, metavar="N",
                        help="Run the full A/B test N times (default: 1)")
    return parser.parse_args()


def _clone_matches_repo(clone_path, repo):
    """Check if an existing clone contains the pinned commit."""
    try:
        result = subprocess.run(
            ["git", "cat-file", "-t", PINNED_COMMIT],
            cwd=clone_path, capture_output=True, text=True)
        return result.returncode == 0 and result.stdout.strip() == "commit"
    except Exception:
        return False


def setup_clones(repo, mode="hook"):
    """Create two copies of the repo: one with pruner, one without."""
    WORK_DIR.mkdir(parents=True, exist_ok=True)

    for clone_path, label in [(CLONE_WITH, "with-pruner"), (CLONE_WITHOUT, "without-pruner")]:
        if clone_path.exists():
            if _clone_matches_repo(clone_path, repo):
                print(f"  Reusing existing clone: {clone_path}", file=sys.stderr)
            else:
                print(f"  Replacing clone (different repo): {clone_path}", file=sys.stderr)
                shutil.rmtree(clone_path)
                shutil.copytree(repo, clone_path, symlinks=True,
                                ignore=shutil.ignore_patterns('.pruner'))
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

    install_pruner_in_clone(CLONE_WITH, PRUNER_BIN, mode)

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


def install_pruner_in_clone(clone_path, pruner_bin, mode="hook"):
    """Install pruner (hook or skill mode) and index in the given clone.

    Cleans previous pruner setup, installs the specified mode, appends
    CLAUDE.md instructions, and runs pruner index.
    """
    # Clean previous pruner setup
    for p in [clone_path / ".claude" / "skills" / "pruner",
              clone_path / ".claude" / "hooks"]:
        if p.exists():
            shutil.rmtree(p)
    settings_file = clone_path / ".claude" / "settings.json"
    if settings_file.exists():
        s = json.loads(settings_file.read_text())
        s.pop("hooks", None)
        settings_file.write_text(json.dumps(s, indent=2))

    # Remove old pruner instructions from CLAUDE.md
    claude_md = clone_path / "CLAUDE.md"
    if claude_md.exists():
        text = claude_md.read_text()
        marker = "## Pruner"
        idx = text.find(marker)
        if idx >= 0:
            claude_md.write_text(text[:idx].rstrip() + "\n")

    if mode == "hook":
        # Install hook
        hook_dir = clone_path / ".claude" / "hooks"
        hook_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(HOOK_SRC, hook_dir / "pruner-context.sh")
        (hook_dir / "pruner-context.sh").chmod(0o755)

        # Install hook settings
        settings = {}
        if settings_file.exists():
            settings = json.loads(settings_file.read_text())
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
        settings_file.write_text(json.dumps(settings, indent=2))

        # Install hook-mode skill
        skill_dir = clone_path / ".claude" / "skills" / "pruner"
        skill_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(SKILL_HOOK_SRC, skill_dir / "SKILL.md")

    elif mode == "skill":
        # Install skill-mode skill (Claude calls pruner as a tool)
        skill_dir = clone_path / ".claude" / "skills" / "pruner"
        skill_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(SKILL_SKILL_SRC, skill_dir / "SKILL.md")

    # Append pruner instructions to CLAUDE.md (skill mode only —
    # in hook mode, context is injected automatically by the hook)
    if mode == "skill":
        template_text = CLAUDE_TEMPLATE.read_text()
        current = claude_md.read_text() if claude_md.exists() else ""
        if "pruner context" not in current:
            with open(claude_md, "a") as f:
                f.write("\n" + template_text)

    # Index the clone
    label = Path(clone_path).name
    print(f"  Indexing {label} clone ...", file=sys.stderr)
    subprocess.run(
        [str(pruner_bin), "index", str(clone_path)],
        capture_output=True, check=True,
    )


def setup_clones_branch_mode(repo, baseline_ref, mode="hook"):
    """Set up two clones for branch comparison: both with pruner, different binaries.

    Builds pruner from baseline_ref for the control side.
    Uses PRUNER_BIN (current worktree) for the feature side.
    """
    WORK_DIR.mkdir(parents=True, exist_ok=True)

    # Build baseline binary from the given ref
    baseline_bin = build_pruner_from_ref(baseline_ref, BASELINE_BIN_DIR)

    for clone_path, label in [(CLONE_BASELINE, "baseline-pruner"),
                               (CLONE_FEATURE, "feature-pruner")]:
        if clone_path.exists():
            if _clone_matches_repo(clone_path, repo):
                print(f"  Reusing existing clone: {clone_path}", file=sys.stderr)
            else:
                print(f"  Replacing clone (different repo): {clone_path}", file=sys.stderr)
                shutil.rmtree(clone_path)
                shutil.copytree(repo, clone_path, symlinks=True,
                                ignore=shutil.ignore_patterns('.pruner'))
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

    # Install pruner in both clones with their respective binaries
    install_pruner_in_clone(CLONE_BASELINE, baseline_bin, mode)
    install_pruner_in_clone(CLONE_FEATURE, PRUNER_BIN, mode)

    # Remove any stray without-pruner artifacts
    for clone_path in [CLONE_BASELINE, CLONE_FEATURE]:
        pruner_dir = clone_path / ".pruner"
        if not pruner_dir.exists():
            # Should have been created by index, warn if missing
            print(f"  WARN: .pruner/ missing in {clone_path.name}", file=sys.stderr)

    print("  Branch mode setup complete.", file=sys.stderr)


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


def ensure_pruner_on_path(binary_path=None, label="default"):
    """Create a bin directory with a 'pruner' symlink to the given binary.

    Args:
        binary_path: Path to pruner binary. Defaults to PRUNER_BIN.
        label: Subdirectory label, allowing multiple bin dirs to coexist
               (e.g., "baseline" and "feature" for branch comparison).

    Returns the directory path.  This ensures the hook script finds the
    target binary first, regardless of what is installed system-wide.
    """
    if binary_path is None:
        binary_path = PRUNER_BIN
    bin_dir = WORK_DIR / "bin" / label
    bin_dir.mkdir(parents=True, exist_ok=True)
    link = bin_dir / "pruner"
    if link.exists() or link.is_symlink():
        link.unlink()
    link.symlink_to(Path(binary_path).resolve())
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

    Automatically detects mode from result keys (without/with_pruner or
    baseline/feature).
    """
    warnings = []
    for entry in results:
        # Detect which keys are present
        if entry.get("baseline") is not None or entry.get("feature") is not None:
            ctrl = entry.get("baseline")
            treat = entry.get("feature")
            ctrl_label, treat_label = "baseline", "feature"
        else:
            ctrl = entry.get("without")
            treat = entry.get("with_pruner")
            ctrl_label, treat_label = "without", "with"
        if not ctrl or not treat:
            continue
        rate_ctrl = ctrl.get("first_turn_cache_rate")
        rate_treat = treat.get("first_turn_cache_rate")
        if rate_ctrl is None or rate_treat is None:
            continue
        diff = abs(rate_treat - rate_ctrl)
        if diff > threshold:
            warnings.append(
                f"{entry['category']}: cache rate diff={diff:.0%} "
                f"({ctrl_label}={rate_ctrl:.0%}, {treat_label}={rate_treat:.0%})"
            )
    return warnings


def aggregate_multi_turn_results(turn_results):
    """Aggregate a list of per-turn parse_stream results into one summary.

    Returns a dict with the same top-level keys as single-turn results
    (cost_usd, tool_calls, total_tokens, etc.) so print_summary and
    validate_cache_symmetry work unchanged.
    """
    cost = 0.0
    tool_calls = 0
    input_tokens = 0
    output_tokens = 0
    turns = 0
    wall_time = 0.0
    tools = []
    per_message_usage = []
    per_user_turn = []
    failed_turns = []

    for i, result in enumerate(turn_results):
        if result is None:
            failed_turns.append(i)
            per_user_turn.append(None)
            continue
        cost += result.get("cost_usd", 0)
        tool_calls += result.get("tool_calls", 0)
        input_tokens += result.get("input_tokens", 0)
        output_tokens += result.get("output_tokens", 0)
        turns += result.get("turns", 0)
        wall_time += result.get("wall_time_s", 0)
        tools.extend(result.get("tools", []))
        per_message_usage.extend(result.get("per_message_usage", []))
        per_user_turn.append(result)

    return {
        "cost_usd": cost,
        "tool_calls": tool_calls,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": input_tokens + output_tokens,
        "turns": turns,
        "wall_time_s": round(wall_time, 1),
        "user_turns": len(turn_results),
        "tools": tools,
        "per_turn": [],  # agentic per-turn not meaningful in aggregate
        "per_message_usage": per_message_usage,
        "per_user_turn": per_user_turn,
        "failed_turns": failed_turns,
        "result_preview": "",
    }


def warmup_cache(repo_dir, bin_dir=None):
    """Run a minimal throwaway Claude prompt to prime the prompt cache.

    Anthropic's prompt cache (~1 hour TTL) caches the system prompt + tool
    schemas prefix. This warmup ensures both sides start with equally warm
    caches, eliminating asymmetric first-call cache advantages.

    Args:
        repo_dir: Directory to run Claude in.
        bin_dir: Pre-built bin directory with pruner symlink. If None, uses
                 ensure_pruner_on_path() default.
    """
    wrapper = WORK_DIR / "run_claude.sh"
    if not wrapper.exists():
        wrapper.write_text("#!/bin/bash\ncd \"$1\" && shift && exec claude \"$@\"\n")
        wrapper.chmod(0o755)

    if bin_dir is None:
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


def run_claude(prompt, repo_dir, label="", save_raw=False, bin_dir=None):
    """Run claude -p inside the repo directory and return parsed results.

    Args:
        bin_dir: Pre-built bin directory with pruner symlink. If None, uses
                 ensure_pruner_on_path() default.
    """
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
    if bin_dir is None:
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


def reset_clone(clone_path, reinstall_pruner=False, mode="hook", pruner_bin=None):
    """Reset clone to pinned commit, discarding any changes from the previous task.

    If reinstall_pruner is True, re-runs pruner init since git clean removes untracked files.

    Args:
        pruner_bin: Path to pruner binary for init/index. Defaults to PRUNER_BIN.
    """
    if pruner_bin is None:
        pruner_bin = PRUNER_BIN
    subprocess.run(["git", "checkout", "."], cwd=clone_path, capture_output=True, check=True)
    subprocess.run(["git", "clean", "-fd"], cwd=clone_path, capture_output=True, check=True)
    if reinstall_pruner:
        init_args = [str(pruner_bin), "init", str(clone_path)]
        if mode == "hook":
            init_args.append("--hook")
        subprocess.run(init_args, check=True, capture_output=True, text=True)
        # Re-index: git clean removes .pruner/ (including the index DB).
        # Without an index, the hook's pruner context call auto-indexes the
        # entire repo, exceeding the 60s hook timeout on large repos.
        subprocess.run(
            [str(pruner_bin), "index", str(clone_path)],
            check=True, capture_output=True, text=True,
        )


def run_single(category, prompt, side, mode="hook", save_raw=False,
               branch_mode=False):
    """Run one side of one task.

    In default mode: side is "with" or "without".
    In branch mode: side is "baseline" or "feature", both with pruner.
    """
    print(f"\n{'='*60}", file=sys.stderr)
    print(f"  Task: {category} [{side}]", file=sys.stderr)
    print(f"{'='*60}", file=sys.stderr)

    if branch_mode:
        if side == "baseline":
            baseline_bin = BASELINE_BIN_DIR / "pruner"
            bin_dir = ensure_pruner_on_path(binary_path=baseline_bin,
                                            label="baseline")
            reset_clone(CLONE_BASELINE, reinstall_pruner=True, mode=mode,
                        pruner_bin=baseline_bin)
            warmup_cache(CLONE_BASELINE, bin_dir=bin_dir)
            result = run_claude(prompt, CLONE_BASELINE,
                                f"{category}/baseline", save_raw,
                                bin_dir=bin_dir)
        else:
            bin_dir = ensure_pruner_on_path(label="feature")
            reset_clone(CLONE_FEATURE, reinstall_pruner=True, mode=mode)
            warmup_cache(CLONE_FEATURE, bin_dir=bin_dir)
            result = run_claude(prompt, CLONE_FEATURE,
                                f"{category}/feature", save_raw,
                                bin_dir=bin_dir)
    elif side == "with":
        reset_clone(CLONE_WITH, reinstall_pruner=True, mode=mode)
        warmup_cache(CLONE_WITH)
        result = run_claude(prompt, CLONE_WITH, f"{category}/with", save_raw)
    else:
        reset_clone(CLONE_WITHOUT)
        warmup_cache(CLONE_WITHOUT)
        result = run_claude(prompt, CLONE_WITHOUT, f"{category}/without",
                            save_raw)

    if result:
        compute_cache_hit_rate(result)
        print_detailed_results(f"{category}/{side}", result)
    return result


def clear_sessions(repo_dir):
    """Clear Claude session files for the given repo directory.

    Sessions are stored at ~/.claude/projects/{sanitized-cwd}/.
    We remove all .jsonl files there so -c doesn't resume a stale session.
    """
    repo_path = Path(repo_dir).resolve()
    # Claude sanitizes paths by replacing / with -
    sanitized = str(repo_path).replace("/", "-").lstrip("-")
    session_dir = Path.home() / ".claude" / "projects" / sanitized
    if session_dir.exists():
        for f in session_dir.glob("*.jsonl"):
            f.unlink()
        print(f"  Cleared sessions in {session_dir}", file=sys.stderr)


def run_claude_turn(prompt, repo_dir, turn_index, label="", save_raw=False,
                    bin_dir=None):
    """Run one turn of a multi-turn conversation.

    Turn 0 starts a new session (no --no-session-persistence so session persists).
    Turn 1+ continues the session with -c (--continue).
    """
    wrapper = WORK_DIR / "run_claude.sh"
    if not wrapper.exists():
        wrapper.write_text("#!/bin/bash\ncd \"$1\" && shift && exec claude \"$@\"\n")
        wrapper.chmod(0o755)

    args = [str(wrapper), str(repo_dir)]
    if turn_index > 0:
        args.extend(["-c"])
    args.extend([
        "-p", prompt,
        "--output-format", "stream-json",
        "--verbose",
        "--max-turns", str(MAX_TURNS),
        "--model", MODEL,
        "--permission-mode", "bypassPermissions",
    ])
    # No --no-session-persistence: session must persist for -c to work

    if bin_dir is None:
        bin_dir = ensure_pruner_on_path()
    env = os.environ.copy()
    env["PATH"] = f"{bin_dir}:{env.get('PATH', '')}"

    turn_label = f"{label}/turn{turn_index}"
    print(f"  Starting [{turn_label}] ...", file=sys.stderr)
    start = time.time()
    proc = subprocess.run(args, capture_output=True, text=True, timeout=600,
                          env=env)
    wall_time = time.time() - start

    save_path = (RAW_DIR / f"{turn_label.replace('/', '_')}.jsonl"
                 if save_raw else None)
    result = parse_stream(proc.stdout, turn_label, save_path)
    if result:
        result["wall_time_s"] = round(wall_time, 1)
    elif proc.stdout.strip():
        fail_path = RAW_DIR / f"{turn_label.replace('/', '_')}_FAILED.jsonl"
        fail_path.parent.mkdir(parents=True, exist_ok=True)
        fail_path.write_text(proc.stdout)
        print(f"  WARN [{turn_label}]: no result, raw saved to {fail_path}",
              file=sys.stderr)

    return result


def print_multi_turn_details(label, data):
    """Print per-user-turn breakdown for multi-turn results."""
    if not data or not data.get("per_user_turn"):
        return

    print(f"\n  [{label}] Multi-turn breakdown ({data['user_turns']} user turns):",
          file=sys.stderr)
    for i, turn in enumerate(data["per_user_turn"]):
        if turn is None:
            print(f"    Turn {i}: FAILED", file=sys.stderr)
            continue
        print(f"    Turn {i}: tools={turn['tool_calls']} "
              f"tokens={turn['total_tokens']:,} "
              f"cost=${turn['cost_usd']:.4f} "
              f"time={turn['wall_time_s']}s",
              file=sys.stderr)

    if data["failed_turns"]:
        print(f"  [{label}] Failed turns: {data['failed_turns']}", file=sys.stderr)


def run_multi_turn_single(category, prompts, side, mode="hook", save_raw=False,
                          branch_mode=False):
    """Run one side of one multi-turn task (all user turns in sequence)."""
    print(f"\n{'='*60}", file=sys.stderr)
    print(f"  Task: {category} [{side}] ({len(prompts)} turns)", file=sys.stderr)
    print(f"{'='*60}", file=sys.stderr)

    # Determine clone dir and bin_dir
    if branch_mode:
        if side == "baseline":
            baseline_bin = BASELINE_BIN_DIR / "pruner"
            bin_dir = ensure_pruner_on_path(binary_path=baseline_bin,
                                            label="baseline")
            clone_dir = CLONE_BASELINE
            reset_clone(clone_dir, reinstall_pruner=True, mode=mode,
                        pruner_bin=baseline_bin)
        else:
            bin_dir = ensure_pruner_on_path(label="feature")
            clone_dir = CLONE_FEATURE
            reset_clone(clone_dir, reinstall_pruner=True, mode=mode)
    elif side == "with":
        bin_dir = ensure_pruner_on_path()
        clone_dir = CLONE_WITH
        reset_clone(clone_dir, reinstall_pruner=True, mode=mode)
    else:
        bin_dir = ensure_pruner_on_path()
        clone_dir = CLONE_WITHOUT
        reset_clone(clone_dir)

    clear_sessions(clone_dir)
    warmup_cache(clone_dir, bin_dir=bin_dir)

    # Run each user turn
    turn_results = []
    for i, prompt in enumerate(prompts):
        result = run_claude_turn(prompt, clone_dir, i,
                                 f"{category}/{side}", save_raw,
                                 bin_dir=bin_dir)
        turn_results.append(result)
        if result:
            print_detailed_results(f"{category}/{side}/turn{i}", result)

    # Aggregate across turns
    aggregate = aggregate_multi_turn_results(turn_results)
    compute_cache_hit_rate(aggregate)
    print_multi_turn_details(f"{category}/{side}", aggregate)

    return aggregate


def interleaved_schedule(tasks, only=None, sides=("without", "with")):
    """Build a randomized run schedule where same-scenario runs are never adjacent.

    Each task produces two runs (one per side). The schedule interleaves them
    so that Anthropic's prompt cache (~1 hour TTL for eligible users) has less
    opportunity for cross-scenario cache contamination.

    Args:
        sides: Tuple of (control, treatment) side names.
               Default ("without", "with") for standard mode.
               Use ("baseline", "feature") for branch comparison mode.
    """
    side_a, side_b = sides
    runs = []
    for category, prompt in tasks:
        if only != side_b:
            runs.append((category, prompt, side_a))
        if only != side_a:
            runs.append((category, prompt, side_b))

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


def print_summary(results, branch_mode=False):
    """Print comparison summary table."""
    print(f"\n{'='*60}", file=sys.stderr)
    print("  SUMMARY", file=sys.stderr)
    print(f"{'='*60}", file=sys.stderr)

    # Determine result keys based on mode
    ctrl_key = "baseline" if branch_mode else "without"
    treat_key = "feature" if branch_mode else "with_pruner"
    ctrl_label = "Base" if branch_mode else "W/O"
    treat_label = "Feat" if branch_mode else "W/"

    valid = [r for r in results if r.get("token_delta_pct") is not None]
    if not valid:
        # Single-side results
        for r in results:
            side = r.get(ctrl_key) or r.get(treat_key)
            if side:
                label = ctrl_label if r.get(ctrl_key) else treat_label
                print(f"  {r['category']:<16} [{label}] tokens={side['total_tokens']:,} "
                      f"cost=${side['cost_usd']:.4f} tools={side['tool_calls']} "
                      f"time={side['wall_time_s']}s",
                      file=sys.stderr)
        return

    print(
        f"  {'Task':<16} {ctrl_label + ' tokens':>12} {treat_label + ' tokens':>12} {'Δ tok':>8} "
        f"{ctrl_label + ' cost':>10} {treat_label + ' cost':>10} {'Δ cost':>8} "
        f"{ctrl_label + ' tools':>10} {treat_label + ' tools':>10} {'Δ time':>10} "
        f"{'Cache ' + ctrl_label:>10} {'Cache ' + treat_label:>10}",
        file=sys.stderr,
    )
    print("  " + "-" * 140, file=sys.stderr)
    for r in valid:
        w = r[ctrl_key]
        p = r[treat_key]
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


def print_cross_round_summary(all_round_results, branch_mode=False):
    """Print aggregate statistics across multiple rounds."""
    ctrl_key = "baseline" if branch_mode else "without"
    treat_key = "feature" if branch_mode else "with_pruner"

    # Collect per-category deltas across rounds
    categories = {}
    for round_results in all_round_results:
        for entry in round_results:
            cat = entry["category"]
            ctrl = entry.get(ctrl_key)
            treat = entry.get(treat_key)
            if not ctrl or not treat:
                continue
            if cat not in categories:
                categories[cat] = {"cost": [], "tools": [], "time": []}

            if ctrl["cost_usd"]:
                categories[cat]["cost"].append(
                    (treat["cost_usd"] - ctrl["cost_usd"]) / ctrl["cost_usd"] * 100)
            if ctrl["tool_calls"]:
                categories[cat]["tools"].append(
                    (treat["tool_calls"] - ctrl["tool_calls"]) / ctrl["tool_calls"] * 100)
            if ctrl.get("wall_time_s") and treat.get("wall_time_s"):
                categories[cat]["time"].append(
                    (treat["wall_time_s"] - ctrl["wall_time_s"]) / ctrl["wall_time_s"] * 100)

    n = len(all_round_results)
    print(f"\n{'='*60}", file=sys.stderr)
    print(f"  CROSS-ROUND SUMMARY (N={n})", file=sys.stderr)
    print(f"{'='*60}", file=sys.stderr)
    print(f"  {'Task':<24} {'Δ cost':>16} {'Δ tools':>16} {'Δ time':>16}",
          file=sys.stderr)
    print(f"  {'':24} {'mean ± spread':>16} {'mean ± spread':>16} {'mean ± spread':>16}",
          file=sys.stderr)
    print("  " + "-" * 76, file=sys.stderr)

    for cat, deltas in categories.items():
        parts = []
        for metric in ["cost", "tools", "time"]:
            vals = deltas[metric]
            if len(vals) >= 2:
                mean = sum(vals) / len(vals)
                spread = max(vals) - min(vals)
                parts.append(f"{mean:+.0f}% ± {spread:.0f}pp")
            elif len(vals) == 1:
                parts.append(f"{vals[0]:+.0f}%")
            else:
                parts.append("N/A")
        print(f"  {cat:<24} {parts[0]:>16} {parts[1]:>16} {parts[2]:>16}",
              file=sys.stderr)


def ensure_repo_cloned(repo_path, repo_url, pinned_commit):
    """Clone the test repo if it doesn't exist, and verify the pinned commit."""
    repo = Path(repo_path)
    if repo.is_dir():
        return
    print(f"  Cloning {repo_url} -> {repo} ...", file=sys.stderr)
    repo.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "clone", repo_url, str(repo)],
                   check=True, capture_output=True)
    subprocess.run(["git", "checkout", pinned_commit], cwd=repo,
                   capture_output=True, check=True)


def run_one_round(args, round_num, model, repo, pinned_commit, task_dict,
                  total_rounds=1):
    """Run one complete round of A/B tests. Returns (results, branch_mode)."""
    global MODEL, PINNED_COMMIT, RAW_DIR
    MODEL = model

    # When running multiple rounds, save raw output in per-round subdirectories
    if total_rounds > 1 and args.save_raw:
        RAW_DIR = Path("/tmp/pruner-bench/ab-raw") / f"round{round_num}"
    else:
        RAW_DIR = Path("/tmp/pruner-bench/ab-raw")
    PINNED_COMMIT = pinned_commit

    branch_mode = args.baseline_branch is not None

    if round_num > 0:
        # Force fresh clones by removing workspace (avoids stale state)
        for p in [CLONE_WITH, CLONE_WITHOUT, CLONE_BASELINE, CLONE_FEATURE]:
            if p.exists():
                shutil.rmtree(p)

    if branch_mode:
        print(f"Setting up branch comparison: {args.baseline_branch} vs current "
              f"(mode={args.mode}) ...", file=sys.stderr)
        setup_clones_branch_mode(repo, args.baseline_branch, mode=args.mode)
        sides = ("baseline", "feature")
    else:
        print(f"Setting up test clones (mode={args.mode}) ...", file=sys.stderr)
        setup_clones(repo, mode=args.mode)
        sides = ("without", "with")

    # Select tasks
    if args.task:
        if args.task not in task_dict:
            print(f"ERROR: Unknown task '{args.task}'. "
                  f"Available: {', '.join(task_dict.keys())}", file=sys.stderr)
            sys.exit(1)
        tasks = [(args.task, task_dict[args.task])]
    else:
        tasks = list(task_dict.items())

    interactive_label = " (interactive)" if args.interactive else ""
    schedule = interleaved_schedule(tasks, only=args.only, sides=sides)
    print(f"\nRun schedule ({len(schedule)} runs){interactive_label}:", file=sys.stderr)
    for i, (cat, _, side) in enumerate(schedule):
        print(f"  {i+1}. {cat} [{side}]", file=sys.stderr)

    # Run all experiments
    run_results = {}
    for category, prompts_or_prompt, side in schedule:
        if args.interactive:
            result = run_multi_turn_single(
                category, prompts_or_prompt, side, mode=args.mode,
                save_raw=args.save_raw, branch_mode=branch_mode)
        else:
            result = run_single(
                category, prompts_or_prompt, side, mode=args.mode,
                save_raw=args.save_raw, branch_mode=branch_mode)
        run_results.setdefault(category, {})[side] = result

    # Assemble results
    ctrl_key = "baseline" if branch_mode else "without"
    treat_key = "feature" if branch_mode else "with"
    result_ctrl_key = ctrl_key
    result_treat_key = "feature" if branch_mode else "with_pruner"

    results = []
    for category, _ in tasks:
        data = run_results.get(category, {})
        ctrl = data.get(ctrl_key)
        treat = data.get(treat_key)
        entry = {"category": category,
                 result_ctrl_key: ctrl, result_treat_key: treat}
        if ctrl and treat:
            entry["token_delta_pct"] = round(
                ((treat["total_tokens"] - ctrl["total_tokens"])
                 / ctrl["total_tokens"] * 100)
                if ctrl["total_tokens"] else 0, 1
            )
            entry["cost_delta_pct"] = round(
                ((treat["cost_usd"] - ctrl["cost_usd"])
                 / ctrl["cost_usd"] * 100)
                if ctrl["cost_usd"] else 0, 1
            )
        results.append(entry)

    return results, branch_mode


def main():
    args = parse_args()
    branch_mode = args.baseline_branch is not None

    # Resolve model
    if args.model:
        model = args.model
    elif args.fast:
        model = "sonnet"
    else:
        model = "opus"

    # Resolve repo and pinned commit
    if args.fast:
        repo = args.repo or FAST_REPO
        pinned_commit = FAST_PINNED_COMMIT
        repo_url = FAST_REPO_URL
    else:
        repo = args.repo or DEFAULT_REPO
        pinned_commit = DEFAULT_PINNED_COMMIT
        repo_url = DEFAULT_REPO_URL

    # Resolve task dict
    if args.fast:
        task_dict = FAST_MULTI_TURN_TASKS if args.interactive else FAST_TASKS
    else:
        task_dict = MULTI_TURN_TASKS if args.interactive else TASKS

    assert shutil.which("claude"), "claude CLI not found"
    assert PRUNER_BIN.exists(), f"pruner not found at {PRUNER_BIN} — run cargo build --release"

    # Check for global pruner hook that would contaminate the "without" side
    home = Path.home()
    global_settings = home / ".claude" / "settings.json"
    if global_settings.exists():
        try:
            settings = json.loads(global_settings.read_text())
            hooks = settings.get("hooks", {})
            for event, hook_list in hooks.items():
                for hook in (hook_list if isinstance(hook_list, list) else []):
                    cmd = hook.get("command", "") if isinstance(hook, dict) else ""
                    if "pruner" in cmd.lower():
                        print(f"ERROR: Global pruner hook found in {global_settings} "
                              f"(event={event}, command={cmd!r}). This would contaminate "
                              f"the 'without' side of the A/B test. Remove it first.",
                              file=sys.stderr)
                        sys.exit(1)
        except (json.JSONDecodeError, KeyError):
            pass  # Settings file is malformed, not our problem

    # Auto-clone repo if needed
    ensure_repo_cloned(repo, repo_url, pinned_commit)
    assert Path(repo).is_dir(), f"repo not found at {repo}"

    # Validate --only matches the active mode
    if args.only:
        if branch_mode and args.only in ("with", "without"):
            print(f"ERROR: --only {args.only} is invalid with --baseline-branch; "
                  f"use --only baseline or --only feature", file=sys.stderr)
            sys.exit(1)
        if not branch_mode and args.only in ("baseline", "feature"):
            print(f"ERROR: --only {args.only} is only valid with --baseline-branch; "
                  f"use --only with or --only without", file=sys.stderr)
            sys.exit(1)

    fast_label = " [FAST]" if args.fast else ""
    print(f"\nA/B test config{fast_label}: model={model}, repo={Path(repo).name}, "
          f"rounds={args.rounds}", file=sys.stderr)

    all_round_results = []
    for round_num in range(args.rounds):
        if args.rounds > 1:
            print(f"\n{'#'*60}", file=sys.stderr)
            print(f"  ROUND {round_num + 1} / {args.rounds}", file=sys.stderr)
            print(f"{'#'*60}", file=sys.stderr)

        results, branch_mode = run_one_round(
            args, round_num, model, repo, pinned_commit, task_dict,
            total_rounds=args.rounds)
        all_round_results.append(results)

        # Print per-round summary
        print_summary(results, branch_mode=branch_mode)

        if args.validate_cache:
            cache_warnings = validate_cache_symmetry(results)
            if cache_warnings:
                print(f"\n  CACHE WARNINGS:", file=sys.stderr)
                for w in cache_warnings:
                    print(f"    {w}", file=sys.stderr)
            else:
                print(f"\n  Cache symmetry OK (all pairs within 10%)", file=sys.stderr)

    # JSON to stdout (all rounds)
    if args.rounds == 1:
        output = all_round_results[0]
    else:
        output = {"rounds": all_round_results}
    print(json.dumps(output, indent=2))

    # Save combined results to file when using --save-raw
    if args.save_raw:
        from datetime import datetime
        repo_name = Path(repo).name
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        mode_tag = "interactive" if args.interactive else "oneshot"
        filename = f"{model}_{repo_name}_{mode_tag}_n{args.rounds}_{timestamp}.json"
        results_dir = Path("/tmp/pruner-bench/ab-raw")
        results_dir.mkdir(parents=True, exist_ok=True)
        results_path = results_dir / filename
        results_path.write_text(json.dumps(output, indent=2))
        print(f"\n  Results saved to {results_path}", file=sys.stderr)

    # Cross-round summary for multi-round
    if args.rounds > 1:
        print_cross_round_summary(all_round_results, branch_mode=branch_mode)


if __name__ == "__main__":
    main()
