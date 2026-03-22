//! Context package generation (text + JSON).
//!
//! Python reference: src/pruner/context.py

use crate::db::IndexDb;
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

/// Brief mode limits.
struct Limits {
    max_files: usize,
    max_symbols: usize,
    max_paths: usize,
    max_snippets: usize,
    max_snippet_lines: usize,
}

impl Limits {
    fn brief() -> Self {
        Self { max_files: 10, max_symbols: 20, max_paths: 5, max_snippets: 15, max_snippet_lines: 10 }
    }

    fn full(max_snippet_lines: usize) -> Self {
        Self { max_files: 999, max_symbols: 999, max_paths: 999, max_snippets: 999, max_snippet_lines }
    }
}

/// Generate a context package from query results.
pub fn generate_context(
    query: &QueryResult,
    _db: &IndexDb,
    repo_path: &Path,
    max_snippet_lines: usize,
    brief: bool,
) -> Result<ContextPackage> {
    let limits = if brief { Limits::brief() } else { Limits::full(max_snippet_lines) };

    // Execution paths
    let execution_paths: Vec<Vec<PathEntry>> = query
        .execution_paths
        .iter()
        .take(limits.max_paths)
        .map(|path| {
            path.iter()
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
        .map(|t| TestFile { path: t.path.clone() })
        .collect();

    // Snippets
    let mut snippets = Vec::new();
    for sym in query.matching_symbols.iter().take(limits.max_snippets) {
        let file_path = repo_path.join(&sym.file_path);
        if let Ok(content) = fs::read_to_string(&file_path) {
            let lines: Vec<&str> = content.lines().collect();
            let start = (sym.line_start - 1).max(0) as usize;
            let end = (start + limits.max_snippet_lines).min(lines.len());
            let code = lines[start..end].join("\n");
            let truncated = end < sym.line_end as usize;

            snippets.push(Snippet {
                file: sym.file_path.clone(),
                symbol: sym.name.clone(),
                line_start: sym.line_start,
                line_end: end as i64 + 1,
                code: if truncated { format!("{code}\n  ...") } else { code },
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
    out.push_str(&format!("**Subsystems:** {}\n\n", ctx.subsystems.join(", ")));

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
                    step.symbol, step.kind, step.file, step.line
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
            out.push_str(&format!("- `{}` ({}, {} lines){}\n", f.path, lang, f.lines, test));
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
            out.push_str(&format!("### {} ({}:{}-{})\n", s.symbol, s.file, s.line_start, s.line_end));
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
            out.push_str(&format!("  {} ({}) — {}:{}\n", s.name, s.kind, s.file, s.line_start));
        }
    }

    if !ctx.relevant_tests.is_empty() {
        out.push_str("\nRelated tests:\n");
        for t in &ctx.relevant_tests {
            out.push_str(&format!("  {}\n", t.path));
        }
    }

    out
}

/// Format context as JSON string.
pub fn format_context_json(ctx: &ContextPackage) -> Result<String> {
    Ok(serde_json::to_string_pretty(ctx)?)
}
