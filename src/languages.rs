//! Language detection, test classification, and ignore patterns.
//!

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
    Go,
    Java,
    C,
    Cpp,
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
        "go" => Some(Language::Go),
        "java" => Some(Language::Java),
        "c" | "h" => Some(Language::C),
        "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hxx" | "hh" | "h++" => Some(Language::Cpp),
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
        // Java convention: FooTest.java, FooTests.java
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && (stem.ends_with("Test") || stem.ends_with("Tests"))
        {
            return true;
        }
    }

    // The component check above handles both Unix and Windows separators.
    // On Windows, also check with normalized separators for string-based path matching.
    if cfg!(windows) {
        let normalized = path_str.replace('\\', "/");
        return normalized.contains("/tests/")
            || normalized.contains("/test/")
            || normalized.contains("/__tests__/");
    }

    path_str.contains("/tests/") || path_str.contains("/test/") || path_str.contains("/__tests__/")
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

static IGNORED_DIRS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        ".git",
        ".hg",
        ".svn",
        "node_modules",
        "__pycache__",
        ".tox",
        ".mypy_cache",
        ".pytest_cache",
        "venv",
        ".venv",
        "env",
        "vendor",
        ".eggs",
        "dist",
        "build",
        "target",
        ".next",
        ".nuxt",
        ".pruner",
        ".ruff_cache",
    ]
    .into_iter()
    .collect()
});

static IGNORED_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "pyc", "pyo", "so", "dylib", "dll", "exe", "o", "a", "class", "jar", "war", "zip", "tar",
        "gz", "bz2", "png", "jpg", "jpeg", "gif", "ico", "svg", "woff", "woff2", "ttf", "eot",
        "mp3", "mp4", "avi", "mov", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "lock",
    ]
    .into_iter()
    .collect()
});

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_detect_language_js_variants() {
        assert_eq!(
            detect_language(Path::new("app.mjs")),
            Some(Language::JavaScript)
        );
        assert_eq!(
            detect_language(Path::new("app.cjs")),
            Some(Language::JavaScript)
        );
        assert_eq!(
            detect_language(Path::new("app.js")),
            Some(Language::JavaScript)
        );
    }

    #[test]
    fn test_detect_language_tsx_jsx() {
        assert_eq!(detect_language(Path::new("App.tsx")), Some(Language::Tsx));
        assert_eq!(detect_language(Path::new("App.jsx")), Some(Language::Tsx));
    }

    #[test]
    fn test_detect_language_go() {
        assert_eq!(detect_language(Path::new("main.go")), Some(Language::Go));
    }

    #[test]
    fn test_detect_language_java() {
        assert_eq!(
            detect_language(Path::new("Main.java")),
            Some(Language::Java)
        );
    }

    #[test]
    fn test_detect_language_c() {
        assert_eq!(detect_language(Path::new("main.c")), Some(Language::C));
        assert_eq!(detect_language(Path::new("header.h")), Some(Language::C));
    }

    #[test]
    fn test_detect_language_cpp() {
        assert_eq!(detect_language(Path::new("main.cpp")), Some(Language::Cpp));
        assert_eq!(detect_language(Path::new("main.cc")), Some(Language::Cpp));
        assert_eq!(detect_language(Path::new("main.cxx")), Some(Language::Cpp));
        assert_eq!(
            detect_language(Path::new("header.hpp")),
            Some(Language::Cpp)
        );
        assert_eq!(
            detect_language(Path::new("header.hxx")),
            Some(Language::Cpp)
        );
        assert_eq!(detect_language(Path::new("header.hh")), Some(Language::Cpp));
    }

    #[test]
    fn test_detect_language_unsupported() {
        assert_eq!(detect_language(Path::new("file.rb")), None);
    }

    #[test]
    fn test_detect_language_no_extension() {
        assert_eq!(detect_language(Path::new("Makefile")), None);
    }

    #[test]
    fn test_is_test_file_prefix() {
        assert!(is_test_file(Path::new("test_main.py")));
    }

    #[test]
    fn test_is_test_file_infixes() {
        assert!(is_test_file(Path::new("auth_test.py")));
        assert!(is_test_file(Path::new("auth.test.js")));
        assert!(is_test_file(Path::new("auth_spec.rb")));
        assert!(is_test_file(Path::new("auth.spec.ts")));
    }

    #[test]
    fn test_is_test_file_java_suffix() {
        assert!(is_test_file(Path::new("AuthHandlerTest.java")));
        assert!(is_test_file(Path::new("AuthHandlerTests.java")));
        assert!(!is_test_file(Path::new("TestUtils.java")));
    }

    #[test]
    fn test_is_test_file_directories() {
        assert!(is_test_file(Path::new("tests/main.py")));
        assert!(is_test_file(Path::new("__tests__/App.test.js")));
        assert!(is_test_file(Path::new("spec/models.rb")));
    }

    #[test]
    fn test_is_test_file_not_test() {
        assert!(!is_test_file(Path::new("src/main.py")));
        assert!(!is_test_file(Path::new("lib/auth.rs")));
    }

    #[test]
    fn test_is_ignored_file_binary() {
        assert!(is_ignored_file(Path::new("lib.so")));
        assert!(is_ignored_file(Path::new("app.exe")));
        assert!(is_ignored_file(Path::new("image.png")));
        assert!(is_ignored_file(Path::new("Cargo.lock")));
    }

    #[test]
    fn test_is_ignored_file_no_extension() {
        assert!(!is_ignored_file(Path::new("Makefile")));
    }

    #[test]
    fn test_is_ignored_file_code() {
        assert!(!is_ignored_file(Path::new("main.py")));
        assert!(!is_ignored_file(Path::new("app.rs")));
    }

    #[test]
    fn test_is_ignored_dir() {
        assert!(is_ignored_dir("node_modules"));
        assert!(is_ignored_dir(".git"));
        assert!(is_ignored_dir("__pycache__"));
        assert!(is_ignored_dir("target"));
        assert!(!is_ignored_dir("src"));
        assert!(!is_ignored_dir("lib"));
    }
}
