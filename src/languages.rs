//! Language detection, test/config classification, and ignore patterns.
//!
#![allow(dead_code)]

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

/// Languages with full tree-sitter parsing support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Rust,
}

/// Detect language from file extension. Returns None for unsupported extensions.
pub fn detect_language(path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "py" => Some(Language::Python),
        "js" | "mjs" | "cjs" => Some(Language::JavaScript),
        "ts" => Some(Language::TypeScript),
        "tsx" | "jsx" => Some(Language::Tsx),
        "rs" => Some(Language::Rust),
        _ => None,
    }
}

/// Check if a file is a test file based on path conventions.
pub fn is_test_file(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    // Parent directory names
    for component in path.components() {
        let s = component.as_os_str().to_string_lossy();
        if TEST_DIRS.contains(s.as_ref()) {
            return true;
        }
    }

    // File name patterns
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with("test_") {
            return true;
        }
        for infix in &["_test.", ".test.", "_spec.", ".spec."] {
            if name.contains(infix) {
                return true;
            }
        }
    }

    path_str.contains("/tests/") || path_str.contains("/test/") || path_str.contains("/__tests__/")
}

/// Check if a file is a config file.
pub fn is_config_file(path: &Path) -> bool {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_lowercase(),
        None => return false,
    };

    CONFIG_PATTERNS.iter().any(|p| name.contains(p))
}

/// Check if a directory should be skipped during indexing.
pub fn is_ignored_dir(name: &str) -> bool {
    IGNORED_DIRS.contains(name)
}

/// Check if a file should be skipped based on extension.
pub fn is_ignored_file(path: &Path) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e,
        None => return false,
    };
    IGNORED_EXTENSIONS.contains(ext)
}

static TEST_DIRS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["tests", "test", "__tests__", "spec", "testing"]
        .into_iter()
        .collect()
});

static CONFIG_PATTERNS: &[&str] = &[
    "config", "settings", ".env", "docker-compose", "dockerfile", "makefile",
    "cmakelists", "setup.py", "setup.cfg", "pyproject.toml", "package.json",
    "tsconfig", "webpack", "babel", ".eslint", ".prettier", "cargo.toml",
    "go.mod", "gemfile", "requirements", "pipfile",
];

static IGNORED_DIRS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        ".git", ".hg", ".svn",
        "node_modules", "__pycache__", ".tox", ".mypy_cache", ".pytest_cache",
        "venv", ".venv", "env", "vendor", ".eggs",
        "dist", "build", "target", ".next", ".nuxt",
        ".pruner", ".ruff_cache",
    ]
    .into_iter()
    .collect()
});

static IGNORED_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "pyc", "pyo", "so", "dylib", "dll", "exe", "o", "a", "class", "jar", "war",
        "zip", "tar", "gz", "bz2",
        "png", "jpg", "jpeg", "gif", "ico", "svg", "woff", "woff2", "ttf", "eot",
        "mp3", "mp4", "avi", "mov",
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
        "lock",
    ]
    .into_iter()
    .collect()
});
