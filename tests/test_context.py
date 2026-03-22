"""Tests for the context generation module."""

import json
import textwrap

import pytest

from pruner.db import IndexDB
from pruner.indexer import index_repo
from pruner.query import analyze_query
from pruner.context import generate_context, format_context_text, format_context_json


@pytest.fixture
def indexed_repo(tmp_path):
    src = tmp_path / "src"
    src.mkdir()

    (src / "processor.py").write_text(textwrap.dedent("""
        def process_data(input_data):
            validated = validate(input_data)
            transformed = transform(validated)
            return save(transformed)

        def validate(data):
            if not data:
                raise ValueError("empty data")
            return data

        def transform(data):
            return data.upper()

        def save(data):
            return {"status": "saved", "data": data}
    """))

    tests = tmp_path / "tests"
    tests.mkdir()

    (tests / "test_processor.py").write_text(textwrap.dedent("""
        from src.processor import process_data, validate

        def test_process_data():
            result = process_data("hello")
            assert result["status"] == "saved"

        def test_validate_empty():
            try:
                validate("")
            except ValueError:
                pass
    """))

    db_path = tmp_path / ".pruner" / "index.db"
    db_path.parent.mkdir(parents=True)
    db = IndexDB(db_path)
    index_repo(tmp_path, db)
    yield db, tmp_path
    db.close()


def test_generate_context(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("process data pipeline", db)
    ctx = generate_context(result, db, repo_path)

    assert ctx["ask"] == "process data pipeline"
    assert len(ctx["keywords"]) > 0


def test_context_has_snippets(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("process_data", db)
    ctx = generate_context(result, db, repo_path)

    # Should have code snippets for matching symbols
    if ctx["key_symbols"]:
        assert len(ctx["snippets"]) > 0


def test_format_text(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("process_data", db)
    ctx = generate_context(result, db, repo_path)
    text = format_context_text(ctx)

    assert "Context for:" in text
    assert "process_data" in text


def test_format_json(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("process_data", db)
    ctx = generate_context(result, db, repo_path)
    json_str = format_context_json(ctx)

    parsed = json.loads(json_str)
    assert parsed["ask"] == "process_data"
    assert "keywords" in parsed
    assert "key_files" in parsed


def test_context_includes_tests(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("processor", db)
    ctx = generate_context(result, db, repo_path)

    if ctx["relevant_tests"]:
        test_paths = [t["path"] for t in ctx["relevant_tests"]]
        assert any("test_processor" in p for p in test_paths)
