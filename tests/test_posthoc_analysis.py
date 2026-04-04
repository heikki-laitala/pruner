"""Unit tests for posthoc_analysis.py."""

import json
import os
import tempfile
import pytest

from posthoc_analysis import (
    extract_tool_calls,
    extract_files_used,
    detect_workspace_prefix,
    detect_task_query,
    is_followup_turn,
    NAVIGATION_TOOLS,
    PRODUCTIVE_TOOLS,
)


def make_assistant_entry(tool_name, input_dict):
    """Create a minimal JSONL assistant entry with a tool use."""
    return json.dumps({
        "type": "assistant",
        "message": {
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_test",
                    "name": tool_name,
                    "input": input_dict,
                }
            ]
        }
    })


def make_jsonl_file(entries):
    """Write entries to a temp JSONL file, return path."""
    f = tempfile.NamedTemporaryFile(mode="w", suffix=".jsonl", delete=False)
    for e in entries:
        f.write(e + "\n")
    f.close()
    return f.name


class TestExtractToolCalls:
    def test_read_tool(self):
        path = make_jsonl_file([
            make_assistant_entry("Read", {"file_path": "/tmp/repo/src/main.ts"})
        ])
        try:
            calls = extract_tool_calls(path)
            assert len(calls) == 1
            assert calls[0]["tool"] == "Read"
            assert calls[0]["file_path"] == "/tmp/repo/src/main.ts"
            assert not calls[0]["is_navigation"]
        finally:
            os.unlink(path)

    def test_edit_tool(self):
        path = make_jsonl_file([
            make_assistant_entry("Edit", {
                "file_path": "/tmp/repo/src/auth.ts",
                "old_string": "foo",
                "new_string": "bar",
            })
        ])
        try:
            calls = extract_tool_calls(path)
            assert len(calls) == 1
            assert calls[0]["tool"] == "Edit"
            assert calls[0]["file_path"] == "/tmp/repo/src/auth.ts"
        finally:
            os.unlink(path)

    def test_write_tool(self):
        path = make_jsonl_file([
            make_assistant_entry("Write", {
                "file_path": "/tmp/repo/src/new.ts",
                "content": "export class New {}",
            })
        ])
        try:
            calls = extract_tool_calls(path)
            assert len(calls) == 1
            assert calls[0]["tool"] == "Write"
            assert calls[0]["file_path"] == "/tmp/repo/src/new.ts"
        finally:
            os.unlink(path)

    def test_navigation_tools(self):
        path = make_jsonl_file([
            make_assistant_entry("Grep", {"pattern": "auth", "path": "/tmp/repo"}),
            make_assistant_entry("Glob", {"pattern": "**/*.ts", "path": "/tmp/repo"}),
            make_assistant_entry("Bash", {"command": "ls /tmp/repo"}),
            make_assistant_entry("Agent", {"description": "explore", "prompt": "find files"}),
        ])
        try:
            calls = extract_tool_calls(path)
            assert len(calls) == 4
            assert all(c["is_navigation"] for c in calls)
        finally:
            os.unlink(path)

    def test_skips_non_assistant(self):
        entries = [
            json.dumps({"type": "system", "subtype": "init"}),
            json.dumps({"type": "user", "message": {"content": "hello"}}),
            make_assistant_entry("Read", {"file_path": "/tmp/repo/a.ts"}),
        ]
        path = make_jsonl_file(entries)
        try:
            calls = extract_tool_calls(path)
            assert len(calls) == 1
        finally:
            os.unlink(path)

    def test_multiple_tools_in_one_message(self):
        entry = json.dumps({
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "tool_use", "id": "t1", "name": "Read",
                     "input": {"file_path": "/tmp/a.ts"}},
                    {"type": "tool_use", "id": "t2", "name": "Read",
                     "input": {"file_path": "/tmp/b.ts"}},
                ]
            }
        })
        path = make_jsonl_file([entry])
        try:
            calls = extract_tool_calls(path)
            assert len(calls) == 2
        finally:
            os.unlink(path)


class TestExtractFilesUsed:
    def test_separates_read_and_write(self):
        calls = [
            {"tool": "Read", "file_path": "/ws/src/a.ts", "is_navigation": False, "input": {}},
            {"tool": "Write", "file_path": "/ws/src/new.ts", "is_navigation": False, "input": {}},
            {"tool": "Edit", "file_path": "/ws/src/b.ts", "is_navigation": False, "input": {}},
        ]
        read, written = extract_files_used(calls, "/ws/")
        assert read == {"src/a.ts", "src/b.ts"}
        assert written == {"src/new.ts"}

    def test_written_then_edited_counts_as_written(self):
        calls = [
            {"tool": "Write", "file_path": "/ws/src/new.ts", "is_navigation": False, "input": {}},
            {"tool": "Edit", "file_path": "/ws/src/new.ts", "is_navigation": False, "input": {}},
        ]
        read, written = extract_files_used(calls, "/ws/")
        assert "src/new.ts" not in read
        assert "src/new.ts" in written

    def test_strips_workspace_prefix(self):
        calls = [
            {"tool": "Read", "file_path": "/tmp/pruner-bench/ab-workspace/with-pruner/src/a.ts",
             "is_navigation": False, "input": {}},
        ]
        read, _ = extract_files_used(calls, "/tmp/pruner-bench/ab-workspace/with-pruner/")
        assert read == {"src/a.ts"}

    def test_ignores_navigation_tools(self):
        calls = [
            {"tool": "Grep", "file_path": None, "is_navigation": True, "input": {}},
            {"tool": "Read", "file_path": "/ws/a.ts", "is_navigation": False, "input": {}},
        ]
        read, written = extract_files_used(calls, "/ws/")
        assert read == {"a.ts"}
        assert written == set()


class TestDetectWorkspacePrefix:
    def test_with_pruner(self):
        calls = [{"file_path": "/tmp/pruner-bench/ab-workspace/with-pruner/src/a.ts"}]
        assert detect_workspace_prefix(calls) == "/tmp/pruner-bench/ab-workspace/with-pruner/"

    def test_without_pruner(self):
        calls = [{"file_path": "/tmp/pruner-bench/ab-workspace/without-pruner/pkg/b.go"}]
        assert detect_workspace_prefix(calls) == "/tmp/pruner-bench/ab-workspace/without-pruner/"

    def test_no_match(self):
        calls = [{"file_path": "/home/user/project/src/a.ts"}]
        assert detect_workspace_prefix(calls) is None


class TestDetectTaskQuery:
    def test_implement(self):
        task, query = detect_task_query("/tmp/round0/implement_with.jsonl")
        assert task == "implement"
        assert query is not None

    def test_understanding(self):
        task, query = detect_task_query("/tmp/round0/understanding_without.jsonl")
        assert task == "understanding"
        assert query is not None

    def test_iterative_refinement(self):
        task, query = detect_task_query("/tmp/round0/iterative_refinement_with_turn0.jsonl")
        assert task == "iterative_refinement"
        assert query is not None

    def test_unknown(self):
        task, query = detect_task_query("/tmp/round0/foobar_with.jsonl")
        assert task == "foobar"
        assert query is None


class TestIsFollowupTurn:
    def test_turn0_is_not_followup(self):
        assert not is_followup_turn("implement_with_turn0.jsonl")

    def test_turn1_is_followup(self):
        assert is_followup_turn("implement_with_turn1.jsonl")

    def test_turn2_is_followup(self):
        assert is_followup_turn("implement_with_turn2.jsonl")

    def test_no_turn_is_not_followup(self):
        assert not is_followup_turn("implement_with.jsonl")

    def test_turn10_is_followup(self):
        assert is_followup_turn("implement_with_turn10.jsonl")
