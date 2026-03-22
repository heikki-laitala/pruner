"""Tests for the indexer module."""

import textwrap

import pytest

from pruner.db import IndexDB
from pruner.indexer import index_repo


@pytest.fixture
def sample_repo(tmp_path):
    """Create a small sample Python repo for testing."""
    src = tmp_path / "src"
    src.mkdir()

    (src / "main.py").write_text(textwrap.dedent("""
        from src.utils import helper

        def main():
            result = helper("test")
            print(result)

        if __name__ == "__main__":
            main()
    """))

    (src / "utils.py").write_text(textwrap.dedent("""
        def helper(value):
            return process(value.upper())

        def process(data):
            return f"processed: {data}"
    """))

    tests = tmp_path / "tests"
    tests.mkdir()

    (tests / "test_utils.py").write_text(textwrap.dedent("""
        from src.utils import helper, process

        def test_helper():
            assert helper("x") == "processed: X"

        def test_process():
            assert process("y") == "processed: y"
    """))

    return tmp_path


@pytest.fixture
def db(tmp_path):
    db_path = tmp_path / ".pruner" / "index.db"
    db_path.parent.mkdir(parents=True)
    db = IndexDB(db_path)
    yield db
    db.close()


def test_index_repo(sample_repo, db):
    stats = index_repo(sample_repo, db)
    assert stats["files"] >= 3
    assert stats["symbols"] > 0
    assert stats["imports"] > 0


def test_index_creates_symbols(sample_repo, db):
    index_repo(sample_repo, db)
    symbols = db.search_symbols("helper")
    assert len(symbols) >= 1
    assert any(s["name"] == "helper" for s in symbols)


def test_index_creates_edges(sample_repo, db):
    index_repo(sample_repo, db)
    # Should have contains edges
    edges = db.get_edges(kind="contains")
    assert len(edges) > 0


def test_index_detects_tests(sample_repo, db):
    index_repo(sample_repo, db)
    test_files = db.get_test_files()
    assert len(test_files) >= 1
    assert any("test_utils" in t["path"] for t in test_files)


def test_index_skips_ignored(sample_repo, db):
    # Create a node_modules dir that should be skipped
    nm = sample_repo / "node_modules"
    nm.mkdir()
    (nm / "pkg.js").write_text("module.exports = {}")

    index_repo(sample_repo, db)
    files = db.get_all_files()
    assert not any("node_modules" in f["path"] for f in files)


def test_index_builds_test_edges(sample_repo, db):
    index_repo(sample_repo, db)
    # test_utils.py should be detected as a test file
    test_files = db.get_test_files()
    assert any("test_utils" in t["path"] for t in test_files)
    # Should have at least some test edges
    edges = db.get_edges(kind="tests")
    assert len(edges) >= 1
