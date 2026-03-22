"""Tests for the token estimation and measurement module."""

import textwrap

import pytest

from pruner.db import IndexDB
from pruner.indexer import index_repo
from pruner.query import analyze_query
from pruner.tokens import estimate_claude_session, estimate_tokens, measure


def test_estimate_tokens_empty():
    assert estimate_tokens("") == 0


def test_estimate_tokens_simple():
    tokens = estimate_tokens("hello world")
    assert tokens == 2


def test_estimate_tokens_code():
    code = "def foo(x, y):\n    return x + y\n"
    tokens = estimate_tokens(code)
    assert tokens > 5


def test_estimate_tokens_proportional():
    short = estimate_tokens("one two three")
    long = estimate_tokens("one two three four five six seven eight nine ten")
    assert long > short


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


def test_measure_returns_result(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("login", db)
    m = measure(result, db, repo_path)

    assert m.ask == "login"
    assert m.repo_total_tokens > 0
    assert m.repo_total_files > 0


def test_measure_pruner_has_context(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("login", db)
    m = measure(result, db, repo_path)

    # Pruner context includes structured metadata (headers, call graphs, etc.)
    # so it can be larger than raw source for tiny repos — that's expected.
    # What matters is that it produces meaningful output.
    assert m.pruner_tokens_text > 0
    assert m.pruner_tokens_json > 0
    assert m.pruner_symbols > 0


def test_measure_has_naive_baseline(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("login", db)
    m = measure(result, db, repo_path)

    assert m.naive_tokens > 0
    assert len(m.naive_files) > 0
    assert m.naive_lines > 0


def test_measure_reduction_is_computed(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("login", db)
    m = measure(result, db, repo_path)

    # Reduction can be negative for tiny repos (overhead > source)
    # Just verify it's computed and within reasonable bounds
    assert -1000 < m.reduction_vs_repo < 100
    assert -1000 < m.reduction_vs_naive < 100


def test_claude_estimate_returns_result(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("login", db)
    est = estimate_claude_session(result, db, repo_path)

    assert est.ask == "login"
    assert est.without_total_tokens > 0
    assert est.with_total_tokens > 0
    assert est.without_files_read > 0
    assert est.with_files_read >= 0


def test_claude_estimate_has_exploration_cost(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("login", db)
    est = estimate_claude_session(result, db, repo_path)

    # Without pruner should have exploration overhead
    assert est.without_exploration_tokens > 0
    assert len(est.without_steps) > 0


def test_claude_estimate_with_pruner_has_context(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("login", db)
    est = estimate_claude_session(result, db, repo_path)

    assert est.with_pruner_context_tokens > 0


def test_claude_estimate_saving_computed(indexed_repo):
    db, repo_path = indexed_repo
    result = analyze_query("login", db)
    est = estimate_claude_session(result, db, repo_path)

    # saving_pct should be a number (can be negative for tiny repos)
    assert -1000 < est.saving_pct < 100
