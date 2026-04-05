//! Context package generation (text + JSON).
//!

use crate::query::QueryResult;
use anyhow::Result;
use serde::Serialize;
use std::fs;
use std::path::Path;

/// Generated context package.
#[derive(Debug, Serialize)]
pub struct ContextPackage {
    pub ask: String,
    pub keywords: Vec<String>,
    pub subsystems: Vec<String>,
    pub execution_paths: Vec<Vec<PathEntry>>,
    pub key_files: Vec<KeyFile>,
    pub key_symbols: Vec<KeySymbol>,
    pub relevant_tests: Vec<TestFile>,
    pub snippets: Vec<Snippet>,
}

#[derive(Debug, Serialize)]
pub struct PathEntry {
    pub symbol: String,
    pub kind: String,
    pub file: String,
    pub line: i64,
    pub depth: usize,
}

#[derive(Debug, Serialize)]
pub struct KeyFile {
    pub path: String,
    pub language: Option<String>,
    pub lines: i64,
    pub is_test: bool,
}

#[derive(Debug, Serialize)]
pub struct KeySymbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line_start: i64,
    pub line_end: i64,
    pub signature: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestFile {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct Snippet {
    pub file: String,
    pub symbol: String,
    pub line_start: i64,
    pub line_end: i64,
    pub code: String,
}

/// Output mode controls how much context is generated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContextMode {
    /// Auto: detect from query results — brief for narrow, focused for broad
    Auto,
    /// Brief: metadata only, no snippets (~3K tokens)
    Brief,
    /// Focused: metadata + key snippets (~10-15K tokens) — the default for agent use
    Focused,
    /// Full: everything, uncapped snippets (~50-70K tokens on large repos)
    Full,
}

/// Classify a query result as narrow or broad based on match spread.
pub fn detect_mode(query: &QueryResult) -> ContextMode {
    let file_count = query.matching_files.len();
    let subsystem_count = query.subsystems.len();
    let path_count = query.execution_paths.len();

    // Narrow: few files concentrated in one subsystem
    if file_count <= 3 && subsystem_count <= 1 {
        return ContextMode::Brief;
    }

    // Also narrow: few files even across subsystems, no execution paths
    if file_count <= 5 && path_count == 0 {
        return ContextMode::Brief;
    }

    ContextMode::Focused
}

struct Limits {
    max_files: usize,
    max_symbols: usize,
    max_paths: usize,
    max_path_depth: usize,
    max_steps_per_path: usize,
    max_tests: usize,
    max_snippets: usize,
    max_snippet_lines: usize,
}

impl Limits {
    fn for_mode(mode: ContextMode, max_snippet_lines: usize) -> Self {
        match mode {
            ContextMode::Auto => Self::for_mode(ContextMode::Focused, max_snippet_lines),
            ContextMode::Brief => Self {
                max_files: 8,
                max_symbols: 15,
                max_paths: 3,
                max_path_depth: 2,
                max_steps_per_path: 10,
                max_tests: 5,
                max_snippets: 0,
                max_snippet_lines: 0,
            },
            ContextMode::Focused => Self {
                max_files: 10,
                max_symbols: 20,
                max_paths: 5,
                max_path_depth: 3,
                max_steps_per_path: 15,
                max_tests: 8,
                max_snippets: 20,
                max_snippet_lines: max_snippet_lines.min(30),
            },
            ContextMode::Full => Self {
                max_files: 999,
                max_symbols: 999,
                max_paths: 999,
                max_path_depth: 999,
                max_steps_per_path: 999,
                max_tests: 999,
                max_snippets: 999,
                max_snippet_lines,
            },
        }
    }
}

/// Generate a context package from query results.
pub fn generate_context(
    query: &QueryResult,
    repo_path: &Path,
    max_snippet_lines: usize,
    mode: ContextMode,
) -> Result<ContextPackage> {
    let resolved = if mode == ContextMode::Auto {
        detect_mode(query)
    } else {
        mode
    };
    let limits = Limits::for_mode(resolved, max_snippet_lines);

    // Execution paths
    let execution_paths: Vec<Vec<PathEntry>> = query
        .execution_paths
        .iter()
        .take(limits.max_paths)
        .map(|path| {
            path.iter()
                .filter(|step| step.depth <= limits.max_path_depth)
                .take(limits.max_steps_per_path)
                .map(|step| PathEntry {
                    symbol: step.name.clone(),
                    kind: step.kind.clone(),
                    file: step.file_path.clone(),
                    line: step.line_start,
                    depth: step.depth,
                })
                .collect()
        })
        .collect();

    // Key files
    let key_files: Vec<KeyFile> = query
        .matching_files
        .iter()
        .take(limits.max_files)
        .map(|f| KeyFile {
            path: f.path.clone(),
            language: f.language.clone(),
            lines: f.line_count,
            is_test: f.is_test,
        })
        .collect();

    // Key symbols
    let key_symbols: Vec<KeySymbol> = query
        .matching_symbols
        .iter()
        .take(limits.max_symbols)
        .map(|s| KeySymbol {
            name: s.name.clone(),
            kind: s.kind.clone(),
            file: s.file_path.clone(),
            line_start: s.line_start,
            line_end: s.line_end,
            signature: s.signature.clone(),
        })
        .collect();

    // Related tests
    let relevant_tests: Vec<TestFile> = query
        .related_tests
        .iter()
        .take(limits.max_tests)
        .map(|t| TestFile {
            path: t.path.clone(),
        })
        .collect();

    // Snippets — cap both line count and character count to avoid
    // minified/bundled files blowing up context size.
    const MAX_SNIPPET_CHARS: usize = 4000;
    let mut snippets = Vec::new();
    for sym in query.matching_symbols.iter().take(limits.max_snippets) {
        let file_path = repo_path.join(&sym.file_path);
        if let Ok(content) = fs::read_to_string(&file_path) {
            let lines: Vec<&str> = content.lines().collect();
            let start = (sym.line_start - 1).max(0) as usize;
            let end = (start + limits.max_snippet_lines).min(lines.len());
            let code = lines[start..end].join("\n");
            let truncated = end < sym.line_end as usize || code.len() > MAX_SNIPPET_CHARS;

            // Truncate to character limit
            let code = if code.len() > MAX_SNIPPET_CHARS {
                format!("{}...", &code[..MAX_SNIPPET_CHARS])
            } else {
                code
            };

            snippets.push(Snippet {
                file: sym.file_path.clone(),
                symbol: sym.name.clone(),
                line_start: sym.line_start,
                line_end: end as i64 + 1,
                code: if truncated {
                    format!("{code}\n  ...")
                } else {
                    code
                },
            });
        }
    }

    Ok(ContextPackage {
        ask: query.ask.clone(),
        keywords: query.keywords.clone(),
        subsystems: query.subsystems.clone(),
        execution_paths,
        key_files,
        key_symbols,
        relevant_tests,
        snippets,
    })
}

/// Format context as human-readable text.
pub fn format_context_text(ctx: &ContextPackage) -> String {
    let mut out = String::new();

    out.push_str(&format!("# Context: {}\n\n", ctx.ask));
    out.push_str(&format!("**Keywords:** {}\n", ctx.keywords.join(", ")));
    out.push_str(&format!(
        "**Subsystems:** {}\n\n",
        ctx.subsystems.join(", ")
    ));

    // Execution paths
    if !ctx.execution_paths.is_empty() {
        out.push_str("## Execution Paths\n\n");
        for (i, path) in ctx.execution_paths.iter().enumerate() {
            out.push_str(&format!("### Path {}\n", i + 1));
            for step in path {
                let indent = "  ".repeat(step.depth);
                out.push_str(&format!(
                    "{indent}{} `{}` ({}) — {}:{}\n",
                    if step.depth > 0 { "→" } else { "" },
                    step.symbol,
                    step.kind,
                    step.file,
                    step.line
                ));
            }
            out.push('\n');
        }
    }

    // Key files
    if !ctx.key_files.is_empty() {
        out.push_str("## Key Files\n\n");
        for f in &ctx.key_files {
            let lang = f.language.as_deref().unwrap_or("?");
            let test = if f.is_test { " [test]" } else { "" };
            out.push_str(&format!(
                "- `{}` ({}, {} lines){}\n",
                f.path, lang, f.lines, test
            ));
        }
        out.push('\n');
    }

    // Key symbols
    if !ctx.key_symbols.is_empty() {
        out.push_str("## Key Symbols\n\n");
        for s in &ctx.key_symbols {
            let sig = s.signature.as_deref().unwrap_or(&s.name);
            out.push_str(&format!(
                "- `{}` ({}) — {}:{}-{}\n",
                sig, s.kind, s.file, s.line_start, s.line_end
            ));
        }
        out.push('\n');
    }

    // Tests
    if !ctx.relevant_tests.is_empty() {
        out.push_str("## Related Tests\n\n");
        for t in &ctx.relevant_tests {
            out.push_str(&format!("- `{}`\n", t.path));
        }
        out.push('\n');
    }

    // Snippets
    if !ctx.snippets.is_empty() {
        out.push_str("## Code Snippets\n\n");
        for s in &ctx.snippets {
            out.push_str(&format!(
                "### {} ({}:{}-{})\n",
                s.symbol, s.file, s.line_start, s.line_end
            ));
            out.push_str("```\n");
            out.push_str(&s.code);
            out.push_str("\n```\n\n");
        }
    }

    out
}

/// Format a compact summary for stdout (brief mode).
pub fn format_context_summary(ctx: &ContextPackage) -> String {
    let mut out = String::new();

    out.push_str(&format!("Keywords: {}\n", ctx.keywords.join(", ")));
    out.push_str(&format!("Subsystems: {}\n", ctx.subsystems.join(", ")));
    out.push_str(&format!("Execution paths: {}\n", ctx.execution_paths.len()));

    if !ctx.key_files.is_empty() {
        out.push_str("\nKey files:\n");
        for f in &ctx.key_files {
            out.push_str(&format!("  {}\n", f.path));
        }
    }

    if !ctx.key_symbols.is_empty() {
        out.push_str("\nKey symbols:\n");
        for s in &ctx.key_symbols {
            out.push_str(&format!(
                "  {} ({}) — {}:{}\n",
                s.name, s.kind, s.file, s.line_start
            ));
        }
    }

    if !ctx.relevant_tests.is_empty() {
        out.push_str("\nRelated tests:\n");
        for t in &ctx.relevant_tests {
            out.push_str(&format!("  {}\n", t.path));
        }
    }

    out.push_str("\nUse this context to work directly. Only read source files if a snippet is truncated. Do not re-explore with grep/glob for the same keywords.\n");
    out.push_str("For deep understanding or debugging, read .pruner/context.md for full execution paths and code snippets.\n");

    out
}

/// Format context as JSON string.
pub fn format_context_json(ctx: &ContextPackage) -> Result<String> {
    Ok(serde_json::to_string_pretty(ctx)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_context() -> ContextPackage {
        ContextPackage {
            ask: "how does login work".to_string(),
            keywords: vec!["login".to_string(), "auth".to_string()],
            subsystems: vec!["auth".to_string()],
            execution_paths: vec![vec![
                PathEntry {
                    symbol: "handle_login".into(),
                    kind: "function".into(),
                    file: "src/auth.rs".into(),
                    line: 10,
                    depth: 0,
                },
                PathEntry {
                    symbol: "verify".into(),
                    kind: "function".into(),
                    file: "src/auth.rs".into(),
                    line: 25,
                    depth: 1,
                },
            ]],
            key_files: vec![KeyFile {
                path: "src/auth.rs".into(),
                language: Some("rust".into()),
                lines: 50,
                is_test: false,
            }],
            key_symbols: vec![
                KeySymbol {
                    name: "handle_login".into(),
                    kind: "function".into(),
                    file: "src/auth.rs".into(),
                    line_start: 10,
                    line_end: 20,
                    signature: Some("fn handle_login(req: Request)".into()),
                },
                KeySymbol {
                    name: "verify".into(),
                    kind: "function".into(),
                    file: "src/auth.rs".into(),
                    line_start: 25,
                    line_end: 35,
                    signature: None,
                },
            ],
            relevant_tests: vec![TestFile {
                path: "tests/test_auth.rs".into(),
            }],
            snippets: vec![Snippet {
                file: "src/auth.rs".into(),
                symbol: "handle_login".into(),
                line_start: 10,
                line_end: 15,
                code: "fn handle_login(req: Request) {\n    verify(req);\n}".into(),
            }],
        }
    }

    #[test]
    fn test_format_context_text_contains_sections() {
        let ctx = sample_context();
        let text = format_context_text(&ctx);

        assert!(text.contains("# Context: how does login work"));
        assert!(text.contains("**Keywords:** login, auth"));
        assert!(text.contains("## Execution Paths"));
        assert!(text.contains("handle_login"));
        assert!(text.contains("## Key Files"));
        assert!(text.contains("src/auth.rs"));
        assert!(text.contains("## Key Symbols"));
        assert!(text.contains("fn handle_login(req: Request)"));
        assert!(text.contains("## Related Tests"));
        assert!(text.contains("test_auth.rs"));
        assert!(text.contains("## Code Snippets"));
        assert!(text.contains("verify(req)"));
    }

    #[test]
    fn test_format_context_text_empty() {
        let ctx = ContextPackage {
            ask: "nothing".into(),
            keywords: vec![],
            subsystems: vec![],
            execution_paths: vec![],
            key_files: vec![],
            key_symbols: vec![],
            relevant_tests: vec![],
            snippets: vec![],
        };
        let text = format_context_text(&ctx);
        assert!(text.contains("# Context: nothing"));
        assert!(!text.contains("## Execution Paths"));
        assert!(!text.contains("## Key Files"));
    }

    #[test]
    fn test_format_context_summary() {
        let ctx = sample_context();
        let summary = format_context_summary(&ctx);

        assert!(summary.contains("Keywords: login, auth"));
        assert!(summary.contains("Subsystems: auth"));
        assert!(summary.contains("Execution paths: 1"));
        assert!(summary.contains("Key files:"));
        assert!(summary.contains("src/auth.rs"));
        assert!(summary.contains("Key symbols:"));
        assert!(summary.contains("handle_login (function)"));
        assert!(summary.contains("Related tests:"));
        assert!(summary.contains("test_auth.rs"));
    }

    #[test]
    fn test_format_context_summary_empty() {
        let ctx = ContextPackage {
            ask: "nothing".into(),
            keywords: vec![],
            subsystems: vec![],
            execution_paths: vec![],
            key_files: vec![],
            key_symbols: vec![],
            relevant_tests: vec![],
            snippets: vec![],
        };
        let summary = format_context_summary(&ctx);
        assert!(!summary.contains("Key files:"));
        assert!(!summary.contains("Key symbols:"));
    }

    #[test]
    fn test_format_context_json_valid() {
        let ctx = sample_context();
        let json_str = format_context_json(&ctx).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["ask"], "how does login work");
        assert!(parsed["key_files"].as_array().unwrap().len() > 0);
        assert!(parsed["execution_paths"].as_array().unwrap().len() > 0);
    }

    #[test]
    fn test_format_context_text_test_file_marker() {
        let ctx = ContextPackage {
            ask: "test".into(),
            keywords: vec![],
            subsystems: vec![],
            execution_paths: vec![],
            key_files: vec![KeyFile {
                path: "tests/test_auth.rs".into(),
                language: Some("rust".into()),
                lines: 30,
                is_test: true,
            }],
            key_symbols: vec![],
            relevant_tests: vec![],
            snippets: vec![],
        };
        let text = format_context_text(&ctx);
        assert!(text.contains("[test]"));
    }

    #[test]
    fn test_generate_context_brief_mode() {
        use crate::db::IndexDb;
        use crate::query;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("login.rs"),
            "fn login() {\n    verify();\n}\nfn verify() {}\n",
        )
        .unwrap();

        let db_path = tmp.path().join(".pruner");
        std::fs::create_dir_all(&db_path).unwrap();
        let db = IndexDb::open(&db_path.join("index.db")).unwrap();

        let fid = db
            .insert_file("src/login.rs", Some("rust"), 50, 4, false, 0)
            .unwrap();
        let s = db
            .insert_symbol(fid, "login", "function", 1, 3, None, None)
            .unwrap();
        db.insert_symbol(fid, "verify", "function", 4, 4, None, None)
            .unwrap();
        db.insert_call(s, "verify", 2).unwrap();

        let result = query::analyze_query("login", &db).unwrap();
        let ctx = generate_context(&result, tmp.path(), 50, ContextMode::Brief).unwrap();

        assert!(!ctx.key_files.is_empty());
        // Brief mode limits snippets to 10 lines
        for snippet in &ctx.snippets {
            let line_count = snippet.code.lines().count();
            assert!(line_count <= 10);
        }
    }

    #[test]
    fn test_format_context_text_symbol_without_signature() {
        let ctx = ContextPackage {
            ask: "test".into(),
            keywords: vec![],
            subsystems: vec![],
            execution_paths: vec![],
            key_files: vec![],
            key_symbols: vec![KeySymbol {
                name: "foo".into(),
                kind: "function".into(),
                file: "a.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: None,
            }],
            relevant_tests: vec![],
            snippets: vec![],
        };
        let text = format_context_text(&ctx);
        // When no signature, should use the name
        assert!(text.contains("`foo` (function)"));
    }

    #[test]
    fn test_detect_mode_narrow() {
        use crate::db::FileRow;
        let result = QueryResult {
            ask: "fix login".into(),
            keywords: vec!["login".into()],
            matching_files: vec![FileRow {
                id: 1,
                path: "src/auth.rs".into(),
                language: Some("rust".into()),
                size: 100,
                line_count: 50,
                is_test: false,
            }],
            matching_symbols: vec![],
            related_tests: vec![],
            execution_paths: vec![],
            subsystems: vec!["src".into()],
        };
        assert_eq!(detect_mode(&result), ContextMode::Brief);
    }

    #[test]
    fn test_detect_mode_broad() {
        use crate::db::FileRow;
        let make_file = |id, path: &str| FileRow {
            id,
            path: path.into(),
            language: Some("rust".into()),
            size: 100,
            line_count: 50,
            is_test: false,
        };
        let result = QueryResult {
            ask: "how does auth flow work".into(),
            keywords: vec!["auth".into(), "flow".into()],
            matching_files: vec![
                make_file(1, "src/auth/login.rs"),
                make_file(2, "src/auth/token.rs"),
                make_file(3, "src/middleware/session.rs"),
                make_file(4, "src/api/handler.rs"),
                make_file(5, "src/gateway/validate.rs"),
            ],
            matching_symbols: vec![],
            related_tests: vec![],
            execution_paths: vec![vec![]],
            subsystems: vec!["auth".into(), "middleware".into(), "api".into()],
        };
        assert_eq!(detect_mode(&result), ContextMode::Focused);
    }
}
