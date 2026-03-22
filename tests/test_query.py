"""Tests for the query analysis module."""

import textwrap

import pytest

from pruner.db import IndexDB
from pruner.indexer import index_repo
from pruner.query import analyze_query, _extract_keywords


def test_extract_keywords():
    keywords = _extract_keywords("why is the login handler broken?")
    assert "login" in keywords
    assert "handler" in keywords
    assert "broken" in keywords
    # Stop words should be filtered
    assert "is" not in keywords
    assert "the" not in keywords
    assert "why" not in keywords


def test_extract_keywords_camelcase():
    keywords = _extract_keywords("fix getUserProfile method")
    assert "getuserprofile" in keywords or "getUserProfile" in keywords
    # Should also split camelCase
    assert "user" in keywords or "profile" in keywords


def test_extract_keywords_snake_case():
    keywords = _extract_keywords("update get_user_data function")
    assert "get_user_data" in keywords


@pytest.fixture
def indexed_repo(tmp_path):
    src = tmp_path / "src"
    src.mkdir()

    (src / "auth.py").write_text(textwrap.dedent("""
        def login(username, password):
            user = find_user(username)
            if user and verify_password(password, user.hash):
                return create_session(user)
            return None

        def logout(session):
            invalidate_session(session)

        def find_user(username):
            pass

        def verify_password(password, hash):
            pass

        def create_session(user):
            pass

        def invalidate_session(session):
            pass
    """))

    (src / "api.py").write_text(textwrap.dedent("""
        from src.auth import login, logout

        def handle_login(request):
            return login(request.username, request.password)

        def handle_logout(request):
            return logout(request.session)
    """))

    tests = tmp_path / "tests"
    tests.mkdir()

    (tests / "test_auth.py").write_text(textwrap.dedent("""
        from src.auth import login, logout

        def test_login():
            result = login("user", "pass")
            assert result is not None

        def test_logout():
            logout("session-123")
    """))

    db_path = tmp_path / ".pruner" / "index.db"
    db_path.parent.mkdir(parents=True)
    db = IndexDB(db_path)
    index_repo(tmp_path, db)
    yield db, tmp_path
    db.close()


def test_query_finds_files(indexed_repo):
    db, _ = indexed_repo
    result = analyze_query("auth login issue", db)
    file_paths = [f["path"] for f in result.matching_files]
    assert any("auth" in p for p in file_paths)


def test_query_finds_symbols(indexed_repo):
    db, _ = indexed_repo
    result = analyze_query("login handler broken", db)
    symbol_names = [s["name"] for s in result.matching_symbols]
    assert "login" in symbol_names


def test_query_finds_tests(indexed_repo):
    db, _ = indexed_repo
    result = analyze_query("auth login handler", db)
    # Tests should be found via test edges from auth.py
    test_paths = [t["path"] for t in result.related_tests]
    assert any("test_auth" in p for p in test_paths)


def test_query_builds_execution_paths(indexed_repo):
    db, _ = indexed_repo
    result = analyze_query("login", db)
    assert len(result.execution_paths) > 0


def test_query_infers_subsystems(indexed_repo):
    db, _ = indexed_repo
    result = analyze_query("login", db)
    assert len(result.subsystems) > 0
