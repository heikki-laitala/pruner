"""Tests for language detection and file classification."""

from pathlib import Path

from pruner.languages import detect_language, is_test_file, is_config_file, should_ignore


def test_detect_python():
    assert detect_language(Path("foo.py")) == "python"


def test_detect_javascript():
    assert detect_language(Path("foo.js")) == "javascript"
    assert detect_language(Path("foo.jsx")) == "javascript"


def test_detect_typescript():
    assert detect_language(Path("foo.ts")) == "typescript"
    assert detect_language(Path("foo.tsx")) == "typescript"


def test_detect_unknown():
    assert detect_language(Path("foo.xyz")) is None


def test_is_test_file():
    assert is_test_file(Path("tests/test_main.py"))
    assert is_test_file(Path("test_main.py"))
    assert is_test_file(Path("src/main.test.js"))
    assert is_test_file(Path("__tests__/main.js"))
    assert is_test_file(Path("src/main_test.py"))
    assert not is_test_file(Path("src/main.py"))
    assert not is_test_file(Path("src/contest.py"))  # "test" substring should not match


def test_is_config_file():
    assert is_config_file(Path("pyproject.toml"))
    assert is_config_file(Path("package.json"))
    assert is_config_file(Path("Dockerfile"))
    assert not is_config_file(Path("src/main.py"))


def test_should_ignore():
    assert should_ignore(Path("node_modules/pkg/index.js"))
    assert should_ignore(Path("__pycache__/main.cpython-312.pyc"))
    assert should_ignore(Path("image.png"))
    assert should_ignore(Path(".git/HEAD"))
    assert not should_ignore(Path("src/main.py"))
