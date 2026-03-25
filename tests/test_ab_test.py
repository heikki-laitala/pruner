"""Unit tests for ab_test.py and ab_test_copilot.py scheduling and parsing logic.

Run with: uv run --with pytest pytest tests/test_ab_test.py -v
"""

import json
import os
import random
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from ab_test import interleaved_schedule, parse_stream, ensure_pruner_on_path, TASKS, PRUNER_BIN, WORK_DIR
from ab_test_copilot import (
    interleaved_schedule as copilot_interleaved_schedule,
    ensure_pruner_on_path as copilot_ensure_pruner_on_path,
    TASKS as COPILOT_TASKS,
    PRUNER_BIN as COPILOT_PRUNER_BIN,
    WORK_DIR as COPILOT_WORK_DIR,
)


class TestInterleavedSchedule:
    def test_all_tasks_both_sides(self):
        tasks = list(TASKS.items())
        schedule = interleaved_schedule(tasks)
        assert len(schedule) == len(tasks) * 2

        categories = {t[0] for t in tasks}
        for cat in categories:
            sides = [r[2] for r in schedule if r[0] == cat]
            assert sorted(sides) == ["with", "without"], f"{cat} missing a side"

    def test_no_adjacent_same_category(self):
        random.seed(42)
        tasks = list(TASKS.items())
        schedule = interleaved_schedule(tasks)
        for i in range(1, len(schedule)):
            assert schedule[i][0] != schedule[i - 1][0], (
                f"Adjacent same category at {i}: {schedule[i][0]}"
            )

    def test_no_adjacent_across_seeds(self):
        """Constraint holds across many random seeds."""
        tasks = list(TASKS.items())
        for seed in range(50):
            random.seed(seed)
            schedule = interleaved_schedule(tasks)
            for i in range(1, len(schedule)):
                assert schedule[i][0] != schedule[i - 1][0], (
                    f"seed={seed}: adjacent same category at {i}: {schedule[i][0]}"
                )

    def test_only_with(self):
        tasks = list(TASKS.items())
        schedule = interleaved_schedule(tasks, only="with")
        assert all(r[2] == "with" for r in schedule)
        assert len(schedule) == len(tasks)

    def test_only_without(self):
        tasks = list(TASKS.items())
        schedule = interleaved_schedule(tasks, only="without")
        assert all(r[2] == "without" for r in schedule)
        assert len(schedule) == len(tasks)

    def test_single_task(self):
        tasks = [("narrow_fix", "test prompt")]
        schedule = interleaved_schedule(tasks)
        assert len(schedule) == 2
        sides = {r[2] for r in schedule}
        assert sides == {"with", "without"}

    def test_two_tasks_no_adjacent(self):
        tasks = [("a", "p1"), ("b", "p2")]
        for seed in range(50):
            random.seed(seed)
            schedule = interleaved_schedule(tasks)
            for i in range(1, len(schedule)):
                assert schedule[i][0] != schedule[i - 1][0]

    def test_prompts_preserved(self):
        tasks = [("narrow_fix", "prompt A"), ("cross_package", "prompt B")]
        schedule = interleaved_schedule(tasks)
        prompts = {r[0]: r[1] for r in schedule}
        assert prompts["narrow_fix"] == "prompt A"
        assert prompts["cross_package"] == "prompt B"


class TestParseStream:
    def _make_stream(self, tool_calls=None, cost=0.5, turns=3,
                     input_tokens=1000, output_tokens=200):
        lines = []
        # Assistant message with tool calls
        content = []
        for tc in (tool_calls or []):
            content.append({"type": "tool_use", "name": tc, "input": {}})
        lines.append(json.dumps({
            "type": "assistant",
            "message": {
                "content": content,
                "usage": {
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "cache_read_input_tokens": 0,
                    "cache_creation_input_tokens": 0,
                },
            },
        }))
        # Result
        lines.append(json.dumps({
            "type": "result",
            "num_turns": turns,
            "total_cost_usd": cost,
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0,
            },
            "result": "test result",
        }))
        return "\n".join(lines)

    def test_basic_parse(self):
        stream = self._make_stream(tool_calls=["Grep", "Read"], cost=0.42, turns=5)
        result = parse_stream(stream)
        assert result is not None
        assert result["turns"] == 5
        assert result["cost_usd"] == 0.42
        assert result["tool_calls"] == 2
        assert len(result["tools"]) == 2
        assert result["tools"][0]["name"] == "Grep"

    def test_no_result_returns_none(self):
        result = parse_stream('{"type": "assistant", "message": {"content": []}}')
        assert result is None

    def test_empty_input(self):
        result = parse_stream("")
        assert result is None

    def test_invalid_json_lines_skipped(self):
        stream = "not json\n" + self._make_stream(cost=0.1, turns=1)
        result = parse_stream(stream)
        assert result is not None
        assert result["cost_usd"] == 0.1

    def test_token_aggregation(self):
        lines = []
        lines.append(json.dumps({
            "type": "assistant",
            "message": {
                "content": [],
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 50,
                    "cache_read_input_tokens": 500,
                    "cache_creation_input_tokens": 200,
                },
            },
        }))
        lines.append(json.dumps({
            "type": "result",
            "num_turns": 1,
            "total_cost_usd": 0.1,
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_read_input_tokens": 500,
                "cache_creation_input_tokens": 200,
            },
            "result": "",
        }))
        result = parse_stream("\n".join(lines))
        assert result["input_tokens"] == 100 + 500 + 200
        assert result["output_tokens"] == 50
        assert result["total_tokens"] == 100 + 500 + 200 + 50

    def test_per_message_usage_tracked(self):
        stream = self._make_stream(tool_calls=["Grep"])
        result = parse_stream(stream)
        assert len(result["per_message_usage"]) == 1
        assert result["per_message_usage"][0]["turn"] == 1


class TestEnsurePrunerOnPath:
    def test_creates_symlink(self):
        if not PRUNER_BIN.exists():
            return  # skip if not built
        bin_dir = ensure_pruner_on_path()
        link = bin_dir / "pruner"
        assert link.exists()
        assert link.is_symlink()
        assert link.resolve() == PRUNER_BIN.resolve()

    def test_symlink_is_executable(self):
        if not PRUNER_BIN.exists():
            return
        bin_dir = ensure_pruner_on_path()
        link = bin_dir / "pruner"
        assert os.access(link, os.X_OK)

    def test_prepended_path_finds_our_binary(self):
        if not PRUNER_BIN.exists():
            return
        bin_dir = ensure_pruner_on_path()
        env = os.environ.copy()
        env["PATH"] = f"{bin_dir}:{env.get('PATH', '')}"
        import subprocess
        result = subprocess.run(
            ["which", "pruner"], capture_output=True, text=True, env=env
        )
        assert result.returncode == 0
        found = Path(result.stdout.strip()).resolve()
        assert found == PRUNER_BIN.resolve()


class TestCopilotInterleavedSchedule:
    def test_all_tasks_both_sides(self):
        tasks = list(COPILOT_TASKS.items())
        schedule = copilot_interleaved_schedule(tasks)
        assert len(schedule) == len(tasks) * 2

        categories = {t[0] for t in tasks}
        for cat in categories:
            sides = [r[2] for r in schedule if r[0] == cat]
            assert sorted(sides) == ["with", "without"], f"{cat} missing a side"

    def test_no_adjacent_same_category(self):
        random.seed(42)
        tasks = list(COPILOT_TASKS.items())
        schedule = copilot_interleaved_schedule(tasks)
        for i in range(1, len(schedule)):
            assert schedule[i][0] != schedule[i - 1][0], (
                f"Adjacent same category at {i}: {schedule[i][0]}"
            )

    def test_only_with(self):
        tasks = list(COPILOT_TASKS.items())
        schedule = copilot_interleaved_schedule(tasks, only="with")
        assert all(r[2] == "with" for r in schedule)
        assert len(schedule) == len(tasks)

    def test_only_without(self):
        tasks = list(COPILOT_TASKS.items())
        schedule = copilot_interleaved_schedule(tasks, only="without")
        assert all(r[2] == "without" for r in schedule)
        assert len(schedule) == len(tasks)

    def test_multi_run_interleaved(self):
        """With --runs 2, repeated runs should also be interleaved."""
        tasks = [("a", "p1"), ("b", "p2")]
        for seed in range(20):
            random.seed(seed)
            schedule = copilot_interleaved_schedule(tasks, runs=2)
            # 2 tasks * 2 sides * 2 runs = 8 items
            assert len(schedule) == 8
            for i in range(1, len(schedule)):
                assert schedule[i][0] != schedule[i - 1][0], (
                    f"seed={seed}: adjacent same category at {i}: {schedule[i][0]}"
                )

    def test_returns_4_tuples(self):
        tasks = [("narrow_fix", "prompt")]
        schedule = copilot_interleaved_schedule(tasks)
        for item in schedule:
            assert len(item) == 4, f"Expected 4-tuple, got {len(item)}-tuple"
            cat, prompt, side, run_idx = item
            assert run_idx == 1


class TestCopilotEnsurePrunerOnPath:
    def test_creates_symlink(self):
        if not COPILOT_PRUNER_BIN.exists():
            return  # skip if not built
        bin_dir = copilot_ensure_pruner_on_path()
        link = bin_dir / "pruner"
        assert link.exists()
        assert link.is_symlink()
        assert link.resolve() == COPILOT_PRUNER_BIN.resolve()


if __name__ == "__main__":
    # uv run --with pytest pytest tests/test_ab_test.py -v
    import subprocess, sys
    sys.exit(subprocess.call(["uv", "run", "--with", "pytest", "pytest", __file__, "-v"]))
