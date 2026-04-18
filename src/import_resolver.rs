//! Import-path resolution for edge-graph expansion.
//!
//! Given an unresolved import string extracted by the parser, return the candidate
//! target paths that exist in the indexed repo. Per-language rules: relative paths
//! for TS/JS/Python/C/C++, dotted-path walks for Python/Rust/Java/C#, and
//! go.mod-prefix stripping for Go.

use crate::languages::Language;
use std::collections::HashSet;
use std::path::Path;

pub struct ResolverContext<'a> {
    pub all_paths: &'a HashSet<String>,
    pub go_module: Option<&'a str>,
    pub rust_crate_roots: &'a [String],
}

pub fn resolve_import(
    importer: &str,
    module: &str,
    language: Language,
    ctx: &ResolverContext,
) -> Vec<String> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            resolve_ts_js(importer, module, ctx)
        }
        Language::Python => resolve_python(importer, module, ctx),
        Language::Rust => resolve_rust(importer, module, ctx),
        Language::Go => resolve_go(module, ctx),
        Language::Java => resolve_java(module, ctx),
        Language::Csharp => resolve_csharp(module, ctx),
        Language::C | Language::Cpp => resolve_c_cpp(importer, module, ctx),
    }
}

/// Collapse `./` and `../` segments in a path string. Returns normalized relative path.
fn normalize_rel(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn parent_dir(path: &str) -> &str {
    Path::new(path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("")
}

// -- TypeScript / JavaScript -----------------------------------------------

fn resolve_ts_js(importer: &str, module: &str, ctx: &ResolverContext) -> Vec<String> {
    // V1: only relative imports. Skip `react`, `@nestjs/common`, etc.
    if !module.starts_with('.') {
        return Vec::new();
    }
    let dir = parent_dir(importer);
    let joined = if dir.is_empty() {
        module.to_string()
    } else {
        format!("{dir}/{module}")
    };
    let base = normalize_rel(&joined);
    const EXTS: &[&str] = &["ts", "tsx", "d.ts", "js", "jsx", "mjs", "cjs"];
    // Direct file hit first
    for ext in EXTS {
        let cand = format!("{base}.{ext}");
        if ctx.all_paths.contains(&cand) {
            return vec![cand];
        }
    }
    // Fall back to index file
    for ext in EXTS {
        let cand = format!("{base}/index.{ext}");
        if ctx.all_paths.contains(&cand) {
            return vec![cand];
        }
    }
    Vec::new()
}

// -- Python ----------------------------------------------------------------

fn resolve_python(importer: &str, module: &str, ctx: &ResolverContext) -> Vec<String> {
    let leading_dots = module.chars().take_while(|c| *c == '.').count();
    let rest = &module[leading_dots..];
    let parts: Vec<&str> = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split('.').collect()
    };

    if leading_dots > 0 {
        // Relative: `.` = same dir, `..` = one up, etc.
        let dir = parent_dir(importer);
        let mut base_parts: Vec<&str> = if dir.is_empty() {
            Vec::new()
        } else {
            dir.split('/').collect()
        };
        for _ in 1..leading_dots {
            base_parts.pop();
        }
        for p in &parts {
            base_parts.push(p);
        }
        let base = base_parts.join("/");
        return python_file_candidates(&base)
            .into_iter()
            .filter(|p| ctx.all_paths.contains(p))
            .collect();
    }

    // Absolute dotted: try top-level, then common package prefixes.
    let dotted = parts.join("/");
    let prefixes = ["", "src/", "lib/", "app/"];
    let mut out = Vec::new();
    for prefix in &prefixes {
        let base = format!("{prefix}{dotted}");
        for cand in python_file_candidates(&base) {
            if ctx.all_paths.contains(&cand) && !out.contains(&cand) {
                out.push(cand);
            }
        }
    }
    out
}

fn python_file_candidates(base: &str) -> Vec<String> {
    vec![format!("{base}.py"), format!("{base}/__init__.py")]
}

// -- Rust ------------------------------------------------------------------

fn resolve_rust(importer: &str, module: &str, ctx: &ResolverContext) -> Vec<String> {
    // `use foo::bar::{a, b}` — keep only the prefix before `{`.
    let head = module.split('{').next().unwrap_or(module).trim();
    let head = head.trim_end_matches(';').trim().trim_end_matches("::");
    let parts: Vec<&str> = head.split("::").filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Vec::new();
    }

    let (base_dir, rest): (String, &[&str]) = match parts[0] {
        "crate" => match find_crate_root_dir(importer, ctx.rust_crate_roots) {
            Some(d) => (d, &parts[1..]),
            None => return Vec::new(),
        },
        "super" => {
            let dir = parent_dir(importer);
            let up = parent_dir(dir).to_string();
            (up, &parts[1..])
        }
        "self" => (parent_dir(importer).to_string(), &parts[1..]),
        _ => return Vec::new(), // external crate — skip in V1
    };

    rust_walk_module_path(&base_dir, rest, ctx)
}

fn find_crate_root_dir(importer: &str, roots: &[String]) -> Option<String> {
    let mut best: Option<String> = None;
    let mut best_len: usize = 0;
    for root in roots {
        let root_dir = parent_dir(root);
        let prefix_check = if root_dir.is_empty() {
            String::new()
        } else {
            format!("{root_dir}/")
        };
        if (prefix_check.is_empty() || importer.starts_with(&prefix_check))
            && root_dir.len() >= best_len
        {
            best = Some(root_dir.to_string());
            best_len = root_dir.len();
        }
    }
    best
}

fn rust_walk_module_path(base_dir: &str, parts: &[&str], ctx: &ResolverContext) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = base_dir.to_string();
    for part in parts {
        if cur.is_empty() {
            cur = (*part).to_string();
        } else {
            cur = format!("{cur}/{part}");
        }
        let as_file = format!("{cur}.rs");
        let as_mod = format!("{cur}/mod.rs");
        if ctx.all_paths.contains(&as_file) && !out.contains(&as_file) {
            out.push(as_file);
        }
        if ctx.all_paths.contains(&as_mod) && !out.contains(&as_mod) {
            out.push(as_mod);
        }
    }
    out
}

// -- Go --------------------------------------------------------------------

fn resolve_go(module: &str, ctx: &ResolverContext) -> Vec<String> {
    let Some(go_mod) = ctx.go_module else {
        return Vec::new();
    };
    let rel = match module.strip_prefix(go_mod) {
        Some(r) => r.trim_start_matches('/'),
        None => return Vec::new(),
    };
    if rel.is_empty() {
        return Vec::new();
    }
    let prefix = format!("{rel}/");
    let mut out = Vec::new();
    for p in ctx.all_paths {
        if !p.ends_with(".go") || !p.starts_with(&prefix) {
            continue;
        }
        let inner = &p[prefix.len()..];
        if !inner.contains('/') {
            out.push(p.clone());
        }
    }
    out.sort();
    out
}

// -- Java ------------------------------------------------------------------

fn resolve_java(module: &str, ctx: &ResolverContext) -> Vec<String> {
    let trimmed = module.trim_end_matches(".*");
    if trimmed.is_empty() {
        return Vec::new();
    }
    let path = trimmed.replace('.', "/");
    let suffix = format!("/{path}.java");
    let top = format!("{path}.java");
    let mut out = Vec::new();
    for p in ctx.all_paths {
        if p.ends_with(&suffix) || *p == top {
            out.push(p.clone());
        }
    }
    out.sort();
    out
}

// -- C# --------------------------------------------------------------------

fn resolve_csharp(module: &str, ctx: &ResolverContext) -> Vec<String> {
    let trimmed = module.trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let dir = trimmed.replace('.', "/");
    let dir_marker = format!("/{dir}/");
    let top_prefix = format!("{dir}/");
    let mut out = Vec::new();
    for p in ctx.all_paths {
        if !p.ends_with(".cs") {
            continue;
        }
        if p.starts_with(&top_prefix) || p.contains(&dir_marker) {
            out.push(p.clone());
        }
    }
    // Utility-hub cap: a namespace touching >20 files is probably too broad.
    if out.len() > 20 {
        return Vec::new();
    }
    out.sort();
    out
}

// -- C / C++ ---------------------------------------------------------------

fn resolve_c_cpp(importer: &str, module: &str, ctx: &ResolverContext) -> Vec<String> {
    // Parser passes the filename inside the quotes (or brackets); we don't
    // distinguish here. System headers (<stdio.h>) won't be in the index.
    let trimmed = module.trim_matches(|c| c == '"' || c == '<' || c == '>');
    if trimmed.is_empty() {
        return Vec::new();
    }

    // Relative to importer's directory first.
    let dir = parent_dir(importer);
    let direct = if dir.is_empty() {
        trimmed.to_string()
    } else {
        normalize_rel(&format!("{dir}/{trimmed}"))
    };
    if ctx.all_paths.contains(&direct) {
        return vec![direct];
    }

    // Walk up the tree looking for `include/{header}`.
    let mut cur = dir.to_string();
    loop {
        let cand = if cur.is_empty() {
            format!("include/{trimmed}")
        } else {
            format!("{cur}/include/{trimmed}")
        };
        if ctx.all_paths.contains(&cand) {
            return vec![cand];
        }
        if cur.is_empty() {
            break;
        }
        cur = parent_dir(&cur).to_string();
    }

    // Last resort: filename suffix match (bounded).
    let suffix = format!("/{trimmed}");
    let mut out = Vec::new();
    for p in ctx.all_paths {
        if p.ends_with(&suffix) || *p == trimmed {
            out.push(p.clone());
            if out.len() >= 3 {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(paths: &[&str]) -> (HashSet<String>, Vec<String>) {
        let set: HashSet<String> = paths.iter().map(|s| s.to_string()).collect();
        let roots: Vec<String> = Vec::new();
        (set, roots)
    }

    // -- TS / JS ----------------------------------------------------------

    #[test]
    fn ts_relative_sibling_file() {
        let (paths, roots) = ctx_with(&["src/auth/service.ts", "src/auth/guard.ts"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import("src/auth/service.ts", "./guard", Language::TypeScript, &ctx);
        assert_eq!(out, vec!["src/auth/guard.ts"]);
    }

    #[test]
    fn ts_parent_dir_import() {
        let (paths, roots) = ctx_with(&["src/auth/service.ts", "src/shared/logger.ts"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import(
            "src/auth/service.ts",
            "../shared/logger",
            Language::TypeScript,
            &ctx,
        );
        assert_eq!(out, vec!["src/shared/logger.ts"]);
    }

    #[test]
    fn ts_index_file_fallback() {
        let (paths, roots) = ctx_with(&["src/auth/service.ts", "src/shared/index.ts"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import(
            "src/auth/service.ts",
            "../shared",
            Language::TypeScript,
            &ctx,
        );
        assert_eq!(out, vec!["src/shared/index.ts"]);
    }

    #[test]
    fn ts_skips_package_imports() {
        let (paths, roots) = ctx_with(&["src/auth/service.ts"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import(
            "src/auth/service.ts",
            "@nestjs/common",
            Language::TypeScript,
            &ctx,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn js_tries_multiple_extensions() {
        let (paths, roots) = ctx_with(&["app/main.js", "app/utils.mjs"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import("app/main.js", "./utils", Language::JavaScript, &ctx);
        assert_eq!(out, vec!["app/utils.mjs"]);
    }

    // -- Python -----------------------------------------------------------

    #[test]
    fn python_relative_one_dot() {
        let (paths, roots) = ctx_with(&["pkg/auth/service.py", "pkg/auth/helpers.py"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import("pkg/auth/service.py", ".helpers", Language::Python, &ctx);
        assert_eq!(out, vec!["pkg/auth/helpers.py"]);
    }

    #[test]
    fn python_relative_parent() {
        let (paths, roots) = ctx_with(&["pkg/auth/service.py", "pkg/shared/logger.py"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import(
            "pkg/auth/service.py",
            "..shared.logger",
            Language::Python,
            &ctx,
        );
        assert_eq!(out, vec!["pkg/shared/logger.py"]);
    }

    #[test]
    fn python_absolute_dotted() {
        let (paths, roots) = ctx_with(&["src/myapp/models/user.py", "src/myapp/__init__.py"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import("src/main.py", "myapp.models.user", Language::Python, &ctx);
        assert_eq!(out, vec!["src/myapp/models/user.py"]);
    }

    #[test]
    fn python_package_init_resolution() {
        let (paths, roots) = ctx_with(&["pkg/auth/__init__.py", "pkg/main.py"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import("pkg/main.py", ".auth", Language::Python, &ctx);
        assert_eq!(out, vec!["pkg/auth/__init__.py"]);
    }

    // -- Rust -------------------------------------------------------------

    #[test]
    fn rust_crate_root_resolution() {
        let (paths, _) = ctx_with(&["src/lib.rs", "src/auth/service.rs"]);
        let roots = vec!["src/lib.rs".to_string()];
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import(
            "src/lib.rs",
            "crate::auth::service::foo",
            Language::Rust,
            &ctx,
        );
        assert!(out.contains(&"src/auth/service.rs".to_string()));
    }

    #[test]
    fn rust_super_walks_up() {
        let (paths, _) = ctx_with(&["src/auth/service.rs", "src/shared.rs"]);
        let roots = vec!["src/lib.rs".to_string()];
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import(
            "src/auth/service.rs",
            "super::shared::Thing",
            Language::Rust,
            &ctx,
        );
        assert!(out.contains(&"src/shared.rs".to_string()));
    }

    #[test]
    fn rust_mod_rs_fallback() {
        let (paths, _) = ctx_with(&["src/lib.rs", "src/auth/mod.rs"]);
        let roots = vec!["src/lib.rs".to_string()];
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import("src/lib.rs", "crate::auth::foo", Language::Rust, &ctx);
        assert!(out.contains(&"src/auth/mod.rs".to_string()));
    }

    #[test]
    fn rust_skips_external_crates() {
        let (paths, _) = ctx_with(&["src/lib.rs"]);
        let roots = vec!["src/lib.rs".to_string()];
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import("src/lib.rs", "anyhow::Result", Language::Rust, &ctx);
        assert!(out.is_empty());
    }

    // -- Go ---------------------------------------------------------------

    #[test]
    fn go_strips_module_prefix() {
        let (paths, roots) = ctx_with(&[
            "internal/auth/service.go",
            "internal/auth/helpers.go",
            "cmd/main.go",
        ]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: Some("example.com/myapp"),
            rust_crate_roots: &roots,
        };
        let out = resolve_import(
            "cmd/main.go",
            "example.com/myapp/internal/auth",
            Language::Go,
            &ctx,
        );
        assert_eq!(
            out,
            vec![
                "internal/auth/helpers.go".to_string(),
                "internal/auth/service.go".to_string(),
            ]
        );
    }

    #[test]
    fn go_skips_external_imports() {
        let (paths, roots) = ctx_with(&["main.go"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: Some("example.com/myapp"),
            rust_crate_roots: &roots,
        };
        let out = resolve_import("main.go", "github.com/other/lib", Language::Go, &ctx);
        assert!(out.is_empty());
    }

    // -- Java -------------------------------------------------------------

    #[test]
    fn java_path_convention() {
        let (paths, roots) = ctx_with(&[
            "src/main/java/com/example/auth/Service.java",
            "src/main/java/com/example/Main.java",
        ]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import(
            "src/main/java/com/example/Main.java",
            "com.example.auth.Service",
            Language::Java,
            &ctx,
        );
        assert_eq!(
            out,
            vec!["src/main/java/com/example/auth/Service.java".to_string()]
        );
    }

    // -- C# ---------------------------------------------------------------

    #[test]
    fn csharp_namespace_matches_directory() {
        let (paths, roots) = ctx_with(&[
            "MyApp/Auth/LoginService.cs",
            "MyApp/Auth/TokenStore.cs",
            "MyApp/Program.cs",
        ]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import("MyApp/Program.cs", "MyApp.Auth", Language::Csharp, &ctx);
        assert_eq!(
            out,
            vec![
                "MyApp/Auth/LoginService.cs".to_string(),
                "MyApp/Auth/TokenStore.cs".to_string(),
            ]
        );
    }

    // -- C / C++ ----------------------------------------------------------

    #[test]
    fn c_relative_include() {
        let (paths, roots) = ctx_with(&["src/auth/service.c", "src/auth/service.h"]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import("src/auth/service.c", "service.h", Language::C, &ctx);
        assert_eq!(out, vec!["src/auth/service.h"]);
    }

    #[test]
    fn cpp_walks_up_for_include_dir() {
        let (paths, roots) = ctx_with(&[
            "src/engine/render.cpp",
            "src/include/geom.hpp",
            "src/engine/other.cpp",
        ]);
        let ctx = ResolverContext {
            all_paths: &paths,
            go_module: None,
            rust_crate_roots: &roots,
        };
        let out = resolve_import("src/engine/render.cpp", "geom.hpp", Language::Cpp, &ctx);
        assert_eq!(out, vec!["src/include/geom.hpp"]);
    }
}
