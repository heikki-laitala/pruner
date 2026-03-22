"""Language detection and file classification."""

from pathlib import Path

LANGUAGE_EXTENSIONS: dict[str, str] = {
    ".py": "python",
    ".js": "javascript",
    ".jsx": "javascript",
    ".ts": "typescript",
    ".tsx": "typescript",
    ".go": "go",
    ".rs": "rust",
    ".rb": "ruby",
    ".java": "java",
    ".c": "c",
    ".h": "c",
    ".cpp": "cpp",
    ".hpp": "cpp",
    ".cs": "csharp",
    ".swift": "swift",
    ".kt": "kotlin",
    ".scala": "scala",
    ".sh": "shell",
    ".bash": "shell",
    ".zsh": "shell",
    ".lua": "lua",
    ".php": "php",
    ".r": "r",
    ".R": "r",
    ".sql": "sql",
    ".yaml": "yaml",
    ".yml": "yaml",
    ".toml": "toml",
    ".json": "json",
    ".xml": "xml",
    ".html": "html",
    ".css": "css",
    ".md": "markdown",
    ".rst": "rst",
}

TEST_DIR_NAMES = {"tests", "test", "__tests__", "spec", "testing"}
TEST_FILE_PREFIXES = ("test_",)
TEST_FILE_INFIXES = ("_test.", ".test.", "_spec.", ".spec.")

CONFIG_PATTERNS = [
    "config", "settings", ".env", "docker-compose", "dockerfile",
    "Makefile", "CMakeLists", "setup.py", "setup.cfg", "pyproject.toml",
    "package.json", "tsconfig", "webpack", "babel", ".eslint", ".prettier",
    "Cargo.toml", "go.mod", "Gemfile", "requirements", "Pipfile",
]

IGNORE_DIRS = {
    ".git", ".hg", ".svn", "node_modules", "__pycache__", ".tox",
    ".mypy_cache", ".pytest_cache", "venv", ".venv", "env",
    ".env", "dist", "build", "target", ".next", ".nuxt",
    "vendor", "third_party", ".eggs", "*.egg-info",
    ".pruner", ".ruff_cache",
}

IGNORE_EXTENSIONS = {
    ".pyc", ".pyo", ".so", ".dylib", ".dll", ".exe", ".o", ".a",
    ".class", ".jar", ".war", ".zip", ".tar", ".gz", ".bz2",
    ".png", ".jpg", ".jpeg", ".gif", ".ico", ".svg", ".woff",
    ".woff2", ".ttf", ".eot", ".mp3", ".mp4", ".avi", ".mov",
    ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx",
    ".lock",
}


def detect_language(path: Path) -> str | None:
    return LANGUAGE_EXTENSIONS.get(path.suffix.lower())


def is_test_file(path: Path) -> bool:
    # Check if any parent directory is a test directory
    for part in path.parts[:-1]:
        if part.lower() in TEST_DIR_NAMES:
            return True
    # Check filename patterns
    name = path.name.lower()
    if any(name.startswith(p) for p in TEST_FILE_PREFIXES):
        return True
    if any(p in name for p in TEST_FILE_INFIXES):
        return True
    return False


def is_config_file(path: Path) -> bool:
    s = str(path).lower()
    return any(p in s for p in CONFIG_PATTERNS)


def should_ignore(path: Path) -> bool:
    parts = path.parts
    for part in parts:
        if part in IGNORE_DIRS:
            return True
    if path.suffix.lower() in IGNORE_EXTENSIONS:
        return True
    return False
