#!/usr/bin/env python3
"""A/B benchmark for GitHub Copilot CLI with Pruner.

Compares Copilot behavior on identical prompts sequentially across modes:
  - without: vanilla Copilot (no pruner skill/hook)
  - skill:   Copilot with pruner skill + explicit pre-context prompt
  - hook:    Copilot with pruner userPromptSubmitted hook

Usage:
    python3 tests/ab_test_copilot.py [options] [/path/to/repo]

Examples:
    python3 tests/ab_test_copilot.py /tmp/pruner-bench/openclaw
    python3 tests/ab_test_copilot.py --task cross_package --runs 3 /tmp/pruner-bench/openclaw
    python3 tests/ab_test_copilot.py --mode hook --only with /tmp/pruner-bench/openclaw
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

PRUNER_DIR = Path(__file__).resolve().parent.parent
PRUNER_BIN = PRUNER_DIR / "target" / "release" / "pruner"

WORK_DIR = Path("/tmp/pruner-bench/copilot-ab-workspace")
RAW_DIR = Path("/tmp/pruner-bench/copilot-ab-raw")
CLONE_WITH = WORK_DIR / "with-pruner"
CLONE_WITHOUT = WORK_DIR / "without-pruner"

MODEL = "gpt-5.3-codex"
TIMEOUT_SECS = 900
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
    parser = argparse.ArgumentParser(description="A/B benchmark for Copilot CLI with pruner")
    parser.add_argument(
        "repo",
        nargs="?",
        default="/tmp/pruner-bench/openclaw",
        help="Path to test repo (default: /tmp/pruner-bench/openclaw)",
    )
    parser.add_argument("--task", choices=list(TASKS.keys()), help="Run only this task")
    parser.add_argument("--runs", type=int, default=1, help="Runs per task per side (default: 1)")
    parser.add_argument(
        "--mode",
        choices=["skill", "hook"],
        default="hook",
        help="Pruner delivery mode for WITH side (default: hook)",
    )
    parser.add_argument(
        "--only",
        choices=["with", "without"],
        help="Run only one side (with or without pruner)",
    )
    parser.add_argument("--save-raw", action="store_true", help="Save raw Copilot JSONL streams")
    parser.add_argument("--model", default=MODEL, help=f"Copilot model (default: {MODEL})")
    return parser.parse_args()


def assert_tools_available():
    assert shutil.which("copilot"), "copilot CLI not found"
    assert PRUNER_BIN.exists(), f"pruner not found at {PRUNER_BIN} — run cargo build --release"


def setup_clones(repo: str, mode: str):
    WORK_DIR.mkdir(parents=True, exist_ok=True)
    for clone_path in [CLONE_WITH, CLONE_WITHOUT]:
        if clone_path.exists():
            print(f"  Reusing clone: {clone_path}", file=sys.stderr)
        else:
            print(f"  Copying {repo} -> {clone_path}", file=sys.stderr)
            shutil.copytree(repo, clone_path, symlinks=True, ignore=shutil.ignore_patterns(".pruner"))
        # Reset to pinned commit for reproducibility
        subprocess.run(["git", "checkout", PINNED_COMMIT], cwd=clone_path,
                        capture_output=True, check=True)
        subprocess.run(["git", "checkout", "."], cwd=clone_path,
                        capture_output=True, check=True)
        subprocess.run(["git", "clean", "-fd"], cwd=clone_path,
                        capture_output=True, check=True)

    # Ensure pruner is removed from without clone
    for p in [
        CLONE_WITHOUT / ".copilot" / "skills" / "pruner",
        CLONE_WITHOUT / ".github" / "hooks" / "pruner-context.json",
        CLONE_WITHOUT / ".github" / "hooks" / "pruner-context.sh",
        CLONE_WITHOUT / ".github" / "hooks" / "pruner-context.ps1",
        CLONE_WITHOUT / ".pruner",
    ]:
        if p.is_dir():
            shutil.rmtree(p)
        elif p.exists():
            p.unlink()
    # Remove only pruner-related content from copilot-instructions.md, not the entire file
    instructions_file = CLONE_WITHOUT / ".github" / "copilot-instructions.md"
    if instructions_file.exists():
        text = instructions_file.read_text()
        marker = "## Pruner"
        idx = text.find(marker)
        if idx >= 0:
            instructions_file.write_text(text[:idx].rstrip() + "\n")

    # Reset with clone and install selected mode
    for p in [
        CLONE_WITH / ".copilot" / "skills" / "pruner",
        CLONE_WITH / ".github" / "hooks" / "pruner-context.json",
        CLONE_WITH / ".github" / "hooks" / "pruner-context.sh",
        CLONE_WITH / ".github" / "hooks" / "pruner-context.ps1",
        CLONE_WITH / ".github" / "copilot-instructions.md",
        CLONE_WITH / ".pruner",
    ]:
        if p.is_dir():
            shutil.rmtree(p)
        elif p.exists():
            p.unlink()

    init_args = [str(PRUNER_BIN), "init", str(CLONE_WITH)]
    if mode == "hook":
        init_args.append("--copilot-hook")
    elif mode == "skill":
        init_args.append("--copilot-skill")
    subprocess.run(init_args, check=True, capture_output=True, text=True)
    subprocess.run([str(PRUNER_BIN), "index", str(CLONE_WITH)], check=True, capture_output=True, text=True)


def build_prompt(task_prompt: str, mode: str) -> str:
    if mode == "skill":
        return (
            "First run `pruner context . \""
            + task_prompt.replace('"', '\\"')
            + "\"` and use that output directly. Then complete this task:\n\n"
            + task_prompt
        )
    if mode == "hook":
        # Hook mode: copilot-instructions.md already tells the model to check
        # .pruner/copilot-context.md.  Passing the raw task keeps pruner's
        # keyword extraction clean (no instruction noise in the query).
        return task_prompt
    # "without" mode: give equivalent exploration instruction for fairness
    return (
        "Explore the codebase to find relevant files and understand the structure. "
        "Then complete this task:\n\n"
        + task_prompt
    )


def parse_jsonl(stdout: str, label: str, save_path: Path | None = None):
    if save_path:
        save_path.parent.mkdir(parents=True, exist_ok=True)
        save_path.write_text(stdout)
        print(f"  Saved raw stream: {save_path}", file=sys.stderr)

    events = []
    for line in stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            continue

    if not events:
        print(f"  WARN [{label}] empty event stream", file=sys.stderr)
        return None

    tool_calls = []
    tool_exec_complete = []
    assistant_messages = []
    result_event = None

    for e in events:
        et = e.get("type", "")
        if et == "assistant.message":
            assistant_messages.append(e)
            for req in e.get("data", {}).get("toolRequests", []):
                tool_calls.append(req.get("name", ""))
        elif et == "tool.execution_complete":
            data = e.get("data", {})
            tool_exec_complete.append(
                {
                    "tool": data.get("toolName"),
                    "success": data.get("success"),
                }
            )
        elif et == "result":
            result_event = e

    usage = (result_event or {}).get("usage", {})
    total_api_ms = usage.get("totalApiDurationMs", 0)
    session_ms = usage.get("sessionDurationMs", 0)
    premium_requests = usage.get("premiumRequests", 0)

    return {
        "events": len(events),
        "assistant_messages": len(assistant_messages),
        "tool_calls": len(tool_calls),
        "tool_names": tool_calls,
        "tool_exec_complete": tool_exec_complete,
        "premium_requests": premium_requests,
        "total_api_duration_ms": total_api_ms,
        "session_duration_ms": session_ms,
        "result": (result_event or {}),
    }


def run_copilot(prompt: str, repo_dir: Path, model: str, label: str = "", save_raw: bool = False):
    args = [
        "copilot",
        "-p",
        prompt,
        "--output-format",
        "json",
        "--allow-all",
        "--allow-all-tools",
        "--experimental",
        "--no-color",
        "--stream",
        "off",
        "--model",
        model,
    ]
    # Isolate session config per run to avoid cross-run contamination from local/global settings.
    # Install hooks inline in config.json since --config-dir prevents repo-level hook discovery.
    cfg_dir = WORK_DIR / ".copilot-config" / label.replace("/", "_")
    if cfg_dir.exists():
        shutil.rmtree(cfg_dir)
    cfg_dir.mkdir(parents=True, exist_ok=True)
    repo_hooks = repo_dir / ".github" / "hooks"
    if repo_hooks.is_dir():
        hook_json = repo_hooks / "pruner-context.json"
        if hook_json.exists():
            hook_cfg = json.loads(hook_json.read_text())
            # Rewrite relative script paths to absolute paths
            for event_hooks in hook_cfg.get("hooks", {}).values():
                for h in event_hooks:
                    cwd = repo_dir / h.pop("cwd", ".")
                    if "bash" in h:
                        h["bash"] = str(cwd / h["bash"])
                    if "powershell" in h:
                        h["powershell"] = str(cwd / h["powershell"])
            cfg_config = {"hooks": hook_cfg["hooks"]}
            (cfg_dir / "config.json").write_text(json.dumps(cfg_config, indent=2))
    args.extend(["--config-dir", str(cfg_dir)])

    env = os.environ.copy()
    # Restrict custom instructions to repository-local files for fair A/B behavior.
    env.pop("COPILOT_CUSTOM_INSTRUCTIONS_DIRS", None)

    print(f"  Starting [{label}] ...", file=sys.stderr)
    start = time.time()
    proc = subprocess.run(
        args,
        cwd=repo_dir,
        capture_output=True,
        text=True,
        timeout=TIMEOUT_SECS,
        env=env,
    )
    wall_time = round(time.time() - start, 1)
    save_path = RAW_DIR / f"{label.replace('/', '_')}.jsonl" if save_raw else None
    parsed = parse_jsonl(proc.stdout, label, save_path)
    if parsed:
        parsed["wall_time_s"] = wall_time
        parsed["exit_code"] = proc.returncode
    return parsed


def reset_clone(clone_path: Path, mode: str | None = None):
    """Reset clone to pinned commit, discarding any changes from the previous task.

    If mode is given, re-installs pruner (hooks/skill) since git clean removes untracked files.
    """
    subprocess.run(["git", "checkout", "."], cwd=clone_path, capture_output=True, check=True)
    subprocess.run(["git", "clean", "-fd"], cwd=clone_path, capture_output=True, check=True)
    if mode:
        init_args = [str(PRUNER_BIN), "init", str(clone_path)]
        if mode == "hook":
            init_args.append("--copilot-hook")
        elif mode == "skill":
            init_args.append("--copilot-skill")
        subprocess.run(init_args, check=True, capture_output=True, text=True)


def run_task(task_name: str, prompt: str, mode: str, model: str, only: str | None, save_raw: bool, run_idx: int):
    print(f"\n{'=' * 64}", file=sys.stderr)
    print(f"  Task: {task_name} (run {run_idx})", file=sys.stderr)
    print(f"{'=' * 64}", file=sys.stderr)

    without = None
    with_pruner = None

    without_prompt = build_prompt(prompt, "without")
    with_prompt = build_prompt(prompt, mode)

    if only != "with":
        reset_clone(CLONE_WITHOUT)
        without = run_copilot(without_prompt, CLONE_WITHOUT, model, f"{task_name}/without/r{run_idx}", save_raw)
    if only != "without":
        reset_clone(CLONE_WITH, mode=mode)
        with_pruner = run_copilot(with_prompt, CLONE_WITH, model, f"{task_name}/with/r{run_idx}", save_raw)

    return without, with_pruner


def print_summary(entries):
    print(f"\n{'=' * 64}", file=sys.stderr)
    print("  SUMMARY", file=sys.stderr)
    print(f"{'=' * 64}", file=sys.stderr)
    print(
        f"  {'Task':<18} {'Run':>3} {'W/O tools':>10} {'W/ tools':>9} {'Δtools':>8} "
        f"{'W/O ms':>10} {'W/ ms':>10} {'Δtime':>8} {'W/O prem':>9} {'W/ prem':>8}",
        file=sys.stderr,
    )
    print("  " + "-" * 120, file=sys.stderr)
    for e in entries:
        w = e.get("without")
        p = e.get("with_pruner")
        if not w or not p:
            continue
        delta_tools = p["tool_calls"] - w["tool_calls"]
        time_delta_pct = (
            (p["wall_time_s"] - w["wall_time_s"]) / w["wall_time_s"] * 100 if w["wall_time_s"] else 0
        )
        print(
            f"  {e['task']:<18} {e['run']:>3} {w['tool_calls']:>10} {p['tool_calls']:>9} {delta_tools:>+8} "
            f"{w['session_duration_ms']:>10} {p['session_duration_ms']:>10} {time_delta_pct:>+7.0f}% "
            f"{w['premium_requests']:>9} {p['premium_requests']:>8}",
            file=sys.stderr,
        )


def main():
    args = parse_args()
    assert_tools_available()
    assert Path(args.repo).is_dir(), f"repo not found at {args.repo}"

    print(f"Setting up clones (mode={args.mode}) ...", file=sys.stderr)
    setup_clones(args.repo, args.mode)

    tasks = [(args.task, TASKS[args.task])] if args.task else list(TASKS.items())
    entries = []
    for task_name, prompt in tasks:
        for run_idx in range(1, args.runs + 1):
            without, with_pruner = run_task(
                task_name, prompt, args.mode, args.model, args.only, args.save_raw, run_idx
            )
            entry = {
                "task": task_name,
                "run": run_idx,
                "mode": args.mode,
                "without": without,
                "with_pruner": with_pruner,
            }
            entries.append(entry)

    print(json.dumps(entries, indent=2))
    print_summary(entries)


if __name__ == "__main__":
    main()
