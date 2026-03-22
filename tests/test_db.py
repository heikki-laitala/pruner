"""Tests for the database layer."""


import pytest

from pruner.db import IndexDB


@pytest.fixture
def db(tmp_path):
    db = IndexDB(tmp_path / "test.db")
    yield db
    db.close()


def test_insert_and_get_file(db):
    db.insert_file("src/main.py", "python", 1024, 50, False)
    f = db.get_file("src/main.py")
    assert f is not None
    assert f["path"] == "src/main.py"
    assert f["language"] == "python"
    assert f["size"] == 1024
    assert f["line_count"] == 50
    assert f["is_test"] == 0


def test_insert_and_search_symbols(db):
    fid = db.insert_file("src/main.py", "python", 100, 10, False)
    db.insert_symbol(fid, "my_function", "function", 1, 10, signature="def my_function(x, y)")

    results = db.search_symbols("my_func")
    assert len(results) == 1
    assert results[0]["name"] == "my_function"
    assert results[0]["kind"] == "function"


def test_insert_import(db):
    fid = db.insert_file("src/main.py", "python", 100, 10, False)
    db.insert_import(fid, "os", "path,getcwd")
    imports = db.get_imports_for_file(fid)
    assert len(imports) == 1
    assert imports[0]["module"] == "os"
    assert imports[0]["names"] == "path,getcwd"


def test_insert_call(db):
    fid = db.insert_file("src/main.py", "python", 100, 10, False)
    sid1 = db.insert_symbol(fid, "caller", "function", 1, 5)
    db.insert_symbol(fid, "callee", "function", 6, 10)
    db.insert_call(sid1, "callee", 3)

    calls = db.get_calls_for_symbol(sid1)
    assert len(calls) == 1
    assert calls[0]["callee_name"] == "callee"


def test_edges(db):
    fid1 = db.insert_file("src/main.py", "python", 100, 10, False)
    fid2 = db.insert_file("tests/test_main.py", "python", 100, 10, True)
    db.insert_edge("tests", source_file_id=fid2, target_file_id=fid1)

    edges = db.get_edges(kind="tests", target_file_id=fid1)
    assert len(edges) == 1
    assert edges[0]["source_file_id"] == fid2


def test_search_files(db):
    db.insert_file("src/auth/login.py", "python", 100, 10, False)
    db.insert_file("src/auth/logout.py", "python", 100, 10, False)
    db.insert_file("src/api/routes.py", "python", 100, 10, False)

    results = db.search_files("auth")
    assert len(results) == 2


def test_clear(db):
    db.insert_file("src/main.py", "python", 100, 10, False)
    db.clear()
    assert db.get_all_files() == []


def test_stats(db):
    fid = db.insert_file("src/main.py", "python", 100, 10, False)
    db.insert_symbol(fid, "foo", "function", 1, 5)
    stats = db.get_stats()
    assert stats["files"] == 1
    assert stats["symbols"] == 1


def test_get_callers_of(db):
    fid = db.insert_file("src/main.py", "python", 100, 20, False)
    sid1 = db.insert_symbol(fid, "caller_fn", "function", 1, 10)
    db.insert_symbol(fid, "target_fn", "function", 11, 20)
    db.insert_call(sid1, "target_fn", 5)

    callers = db.get_callers_of("target_fn")
    assert len(callers) == 1
    assert callers[0]["caller_name"] == "caller_fn"
