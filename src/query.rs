//! Keyword extraction + heuristic relevance matching.
//!

use crate::db::{FileRow, IndexDb, SymbolRow, TraceRow};
use anyhow::Result;
use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

const MAX_TRACED_SYMBOLS: usize = 20;
const MAX_RESULT_SYMBOLS: usize = 100;
const MAX_RESULT_FILES: usize = 50;
const MAX_RESULT_TESTS: usize = 20;
const TRACE_TIME_BUDGET: Duration = Duration::from_secs(10);

/// Result of analyzing a natural language query against the index.
#[derive(Debug)]
pub struct QueryResult {
    pub ask: String,
    pub keywords: Vec<String>,
    pub matching_files: Vec<FileRow>,
    pub matching_symbols: Vec<SymbolRow>,
    pub related_tests: Vec<FileRow>,
    pub execution_paths: Vec<Vec<PathStep>>,
    pub subsystems: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PathStep {
    #[allow(dead_code)]
    pub symbol_id: i64,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line_start: i64,
    pub depth: usize,
}

impl From<TraceRow> for PathStep {
    fn from(row: TraceRow) -> Self {
        Self {
            symbol_id: row.id,
            name: row.name,
            kind: row.kind,
            file_path: row.file_path,
            line_start: row.line_start,
            depth: row.depth,
        }
    }
}

impl QueryResult {
    /// All unique file IDs referenced in this result.
    pub fn all_relevant_file_ids(&self) -> HashSet<i64> {
        let mut ids = HashSet::new();
        for f in &self.matching_files {
            ids.insert(f.id);
        }
        for s in &self.matching_symbols {
            ids.insert(s.file_id);
        }
        for t in &self.related_tests {
            ids.insert(t.id);
        }
        // execution_paths don't carry file_id yet
        ids
    }
}

/// Analyze a natural language query against the index.
pub fn analyze_query(ask: &str, db: &IndexDb) -> Result<QueryResult> {
    let keywords = extract_keywords(ask);
    let mut matching_files = Vec::new();
    let mut matching_symbols = Vec::new();
    let mut seen_file_ids = HashSet::new();
    let mut seen_symbol_ids = HashSet::new();

    for kw in &keywords {
        for file in db.search_files(kw)? {
            if seen_file_ids.insert(file.id) {
                matching_files.push(file);
            }
        }
        for sym in db.search_symbols(kw)? {
            if seen_symbol_ids.insert(sym.id) {
                matching_symbols.push(sym);
            }
        }
    }

    // Find related tests
    let mut related_tests = Vec::new();
    let mut seen_test_ids = HashSet::new();
    for file in &matching_files {
        for edge in db.edges_to_file(file.id, "tests")? {
            if let Some(src_file_id) = edge.source_file_id
                && seen_test_ids.insert(src_file_id)
                && let Some(tf) = db.get_file_by_path_id(src_file_id)?
            {
                related_tests.push(tf);
            }
        }
    }

    // Score and rank symbols, then cap results
    let scored_symbols = score_and_rank_symbols(&matching_symbols, &keywords);
    let top_symbols: Vec<&SymbolRow> = scored_symbols
        .iter()
        .take(MAX_TRACED_SYMBOLS)
        .map(|(sym, _)| *sym)
        .collect();

    // Cap matching_symbols to top N by relevance score
    let matching_symbols: Vec<SymbolRow> = scored_symbols
        .into_iter()
        .take(MAX_RESULT_SYMBOLS)
        .map(|(sym, _)| sym.clone())
        .collect();

    // Cap matching files and tests
    matching_files.truncate(MAX_RESULT_FILES);
    related_tests.truncate(MAX_RESULT_TESTS);

    // Trace execution paths with time budget
    let mut execution_paths = Vec::new();
    let trace_deadline = Instant::now() + TRACE_TIME_BUDGET;
    for sym in &top_symbols {
        if Instant::now() >= trace_deadline {
            break;
        }
        let path = trace_execution_path_cte(sym, db, 5)?;
        if path.len() > 1 {
            execution_paths.push(path);
        }
    }

    // Infer subsystems
    let subsystems = infer_subsystems(&matching_files);

    Ok(QueryResult {
        ask: ask.to_string(),
        keywords,
        matching_files,
        matching_symbols,
        related_tests,
        execution_paths,
        subsystems,
    })
}

/// Extract search keywords from a natural language query.
pub fn extract_keywords(ask: &str) -> Vec<String> {
    let mut keywords = Vec::new();
    let mut seen = HashSet::new();

    // Split on non-alphanumeric (keep underscores/hyphens)
    for word in ask.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        let lower = word.to_lowercase();
        if STOP_WORDS.contains(lower.as_str()) {
            continue;
        }
        if seen.insert(lower.clone()) {
            keywords.push(lower);
        }

        // Split camelCase / snake_case
        for sub in split_identifier(word) {
            let sub_lower = sub.to_lowercase();
            if !STOP_WORDS.contains(sub_lower.as_str()) && seen.insert(sub_lower.clone()) {
                keywords.push(sub_lower);
            }
        }
    }

    keywords
}

/// Split a camelCase or snake_case identifier into parts.
fn split_identifier(s: &str) -> Vec<String> {
    let mut parts = Vec::new();

    // snake_case split
    if s.contains('_') {
        for part in s.split('_') {
            if !part.is_empty() {
                parts.push(part.to_string());
            }
        }
        return parts;
    }

    // camelCase split
    let mut current = String::new();
    for ch in s.chars() {
        if ch.is_uppercase() && !current.is_empty() {
            parts.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current);
    }

    // Only return if we actually split something
    if parts.len() > 1 { parts } else { Vec::new() }
}

/// Score a symbol's relevance to the query keywords.
/// Higher score = more relevant.
fn score_symbol(sym: &SymbolRow, keywords: &[String]) -> i32 {
    let name_lower = sym.name.to_lowercase();
    let mut score: i32 = 0;

    for kw in keywords {
        if name_lower == *kw {
            score += 100; // exact match
        } else if name_lower.starts_with(kw) {
            score += 50; // prefix match
        } else if name_lower.contains(kw) {
            score += 10; // substring match
        }
    }

    // Bonus for callable symbols (more likely to have useful execution paths)
    match sym.kind.as_str() {
        "function" | "method" => score += 20,
        "class" | "struct" | "trait" | "interface" => score += 5,
        _ => {}
    }

    score
}

/// Score and rank symbols by relevance, returning (symbol, score) pairs sorted descending.
fn score_and_rank_symbols<'a>(
    symbols: &'a [SymbolRow],
    keywords: &[String],
) -> Vec<(&'a SymbolRow, i32)> {
    let mut scored: Vec<(&SymbolRow, i32)> = symbols
        .iter()
        .map(|s| (s, score_symbol(s, keywords)))
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored
}

/// Trace call graph from a symbol using a single SQL recursive CTE.
/// Replaces the per-step DFS that caused millions of DB round-trips on large repos.
fn trace_execution_path_cte(
    start: &SymbolRow,
    db: &IndexDb,
    max_depth: usize,
) -> Result<Vec<PathStep>> {
    let rows = db.trace_call_graph(start.id, max_depth)?;

    let mut path = vec![PathStep {
        symbol_id: start.id,
        name: start.name.clone(),
        kind: start.kind.clone(),
        file_path: start.file_path.clone(),
        line_start: start.line_start,
        depth: 0,
    }];
    path.extend(rows.into_iter().map(PathStep::from));

    Ok(path)
}

/// Infer subsystems from file paths.
fn infer_subsystems(files: &[FileRow]) -> Vec<String> {
    let mut subsystems = HashSet::new();

    for f in files {
        let parts: Vec<&str> = f.path.split('/').collect();
        for part in &parts {
            if !SCAFFOLD_DIRS.contains(part) && !part.contains('.') && !part.is_empty() {
                subsystems.insert(part.to_string());
                break;
            }
        }
    }

    let mut result: Vec<_> = subsystems.into_iter().collect();
    result.sort();
    result
}

static SCAFFOLD_DIRS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["src", "lib", "app", "pkg", "cmd", "internal"]
        .into_iter()
        .collect()
});

static STOP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would", "shall",
        "should", "may", "might", "must", "can", "could",
        "i", "me", "my", "we", "our", "you", "your", "he", "she", "it",
        "they", "them", "their", "this", "that", "these", "those",
        "what", "which", "who", "whom", "where", "when", "why", "how",
        "not", "no", "nor", "but", "or", "and", "if", "then", "else",
        "than", "too", "very", "just", "about", "above", "after", "again",
        "all", "also", "any", "because", "before", "between", "both",
        "by", "each", "for", "from", "get", "got", "here", "in", "into",
        "of", "on", "once", "only", "other", "out", "over", "own", "same",
        "so", "some", "such", "there", "through", "to", "under", "until",
        "up", "want", "with",
        "fix", "add", "make", "use", "find", "show", "change", "update",
        "need", "like", "work", "look", "way", "new", "file", "files",
        "code", "implement", "create",
    ]
    .into_iter()
    .collect()
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::IndexDb;

    #[test]
    fn test_extract_keywords_basic() {
        let kws = extract_keywords("why is login broken?");
        assert!(kws.contains(&"login".to_string()));
        assert!(kws.contains(&"broken".to_string()));
        assert!(!kws.contains(&"is".to_string()));
        assert!(!kws.contains(&"why".to_string()));
    }

    #[test]
    fn test_extract_keywords_camel_case() {
        let kws = extract_keywords("handleUserRequest");
        assert!(kws.contains(&"handleuserrequest".to_string()));
        assert!(kws.contains(&"handle".to_string()));
        assert!(kws.contains(&"request".to_string()));
    }

    #[test]
    fn test_extract_keywords_snake_case() {
        let kws = extract_keywords("parse_auth_token");
        assert!(kws.contains(&"parse_auth_token".to_string()));
        assert!(kws.contains(&"parse".to_string()));
        assert!(kws.contains(&"auth".to_string()));
        assert!(kws.contains(&"token".to_string()));
    }

    #[test]
    fn test_extract_keywords_deduplication() {
        let kws = extract_keywords("login login LOGIN");
        let login_count = kws.iter().filter(|k| *k == "login").count();
        assert_eq!(login_count, 1);
    }

    #[test]
    fn test_split_identifier_camel() {
        let parts = split_identifier("handleUserRequest");
        assert_eq!(parts, vec!["handle", "User", "Request"]);
    }

    #[test]
    fn test_split_identifier_snake() {
        let parts = split_identifier("parse_auth_token");
        assert_eq!(parts, vec!["parse", "auth", "token"]);
    }

    #[test]
    fn test_score_symbol_exact_match() {
        let sym = SymbolRow {
            id: 1, file_id: 1, name: "login".into(), kind: "function".into(),
            line_start: 1, line_end: 10, signature: None, file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string()]);
        assert_eq!(score, 120); // 100 exact + 20 function bonus
    }

    #[test]
    fn test_score_symbol_prefix_match() {
        let sym = SymbolRow {
            id: 1, file_id: 1, name: "login_user".into(), kind: "function".into(),
            line_start: 1, line_end: 10, signature: None, file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string()]);
        assert_eq!(score, 70); // 50 prefix + 20 function bonus
    }

    #[test]
    fn test_score_symbol_substring_match() {
        let sym = SymbolRow {
            id: 1, file_id: 1, name: "handle_login_request".into(), kind: "struct".into(),
            line_start: 1, line_end: 10, signature: None, file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string()]);
        assert_eq!(score, 15); // 10 substring + 5 struct bonus
    }

    #[test]
    fn test_score_and_rank_symbols() {
        let symbols = vec![
            SymbolRow {
                id: 1, file_id: 1, name: "handle_timeout".into(), kind: "function".into(),
                line_start: 1, line_end: 10, signature: None, file_path: "a.rs".into(),
            },
            SymbolRow {
                id: 2, file_id: 1, name: "timeout".into(), kind: "function".into(),
                line_start: 20, line_end: 30, signature: None, file_path: "a.rs".into(),
            },
            SymbolRow {
                id: 3, file_id: 1, name: "timeout_handler".into(), kind: "function".into(),
                line_start: 40, line_end: 50, signature: None, file_path: "a.rs".into(),
            },
        ];
        let ranked = score_and_rank_symbols(&symbols, &["timeout".to_string()]);
        assert_eq!(ranked[0].0.id, 2); // exact match first (100 + 20 = 120)
        assert_eq!(ranked[1].0.id, 3); // prefix match second (50 + 20 = 70)
        assert_eq!(ranked[2].0.id, 1); // substring match last (10 + 20 = 30)
    }

    #[test]
    fn test_extract_keywords_empty() {
        let kws = extract_keywords("");
        assert!(kws.is_empty());
    }

    #[test]
    fn test_extract_keywords_all_stop_words() {
        let kws = extract_keywords("the is a an");
        assert!(kws.is_empty());
    }

    #[test]
    fn test_extract_keywords_mixed_separators() {
        let kws = extract_keywords("auth/login.handler");
        assert!(kws.contains(&"auth".to_string()));
        assert!(kws.contains(&"login".to_string()));
        assert!(kws.contains(&"handler".to_string()));
    }

    #[test]
    fn test_extract_keywords_hyphenated() {
        let kws = extract_keywords("rate-limiter");
        assert!(kws.contains(&"rate-limiter".to_string()));
    }

    #[test]
    fn test_split_identifier_no_split() {
        // Single word — nothing to split
        let parts = split_identifier("login");
        assert!(parts.is_empty());
    }

    #[test]
    fn test_split_identifier_all_upper() {
        // Each uppercase char triggers a split: "A", "P", "I"
        let parts = split_identifier("API");
        assert_eq!(parts, vec!["A", "P", "I"]);
    }

    #[test]
    fn test_score_symbol_no_match() {
        let sym = SymbolRow {
            id: 1, file_id: 1, name: "unrelated".into(), kind: "function".into(),
            line_start: 1, line_end: 10, signature: None, file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string()]);
        assert_eq!(score, 20); // only function bonus, no keyword match
    }

    #[test]
    fn test_score_symbol_multiple_keywords() {
        let sym = SymbolRow {
            id: 1, file_id: 1, name: "login_handler".into(), kind: "method".into(),
            line_start: 1, line_end: 10, signature: None, file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string(), "handler".to_string()]);
        // login: prefix 50, handler: substring 10 => 60 + 20 method bonus
        assert_eq!(score, 80);
    }

    #[test]
    fn test_score_symbol_unknown_kind() {
        let sym = SymbolRow {
            id: 1, file_id: 1, name: "login".into(), kind: "variable".into(),
            line_start: 1, line_end: 10, signature: None, file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string()]);
        assert_eq!(score, 100); // exact match, no kind bonus
    }

    #[test]
    fn test_score_and_rank_empty() {
        let ranked = score_and_rank_symbols(&[], &["login".to_string()]);
        assert!(ranked.is_empty());
    }

    #[test]
    fn test_all_relevant_file_ids() {
        let result = QueryResult {
            ask: "test".into(),
            keywords: vec![],
            matching_files: vec![
                FileRow { id: 1, path: "a.rs".into(), language: None, size: 0, line_count: 0, is_test: false },
            ],
            matching_symbols: vec![
                SymbolRow { id: 10, file_id: 2, name: "foo".into(), kind: "function".into(),
                    line_start: 1, line_end: 10, signature: None, file_path: "b.rs".into() },
            ],
            related_tests: vec![
                FileRow { id: 3, path: "test_a.rs".into(), language: None, size: 0, line_count: 0, is_test: true },
            ],
            execution_paths: vec![],
            subsystems: vec![],
        };
        let ids = result.all_relevant_file_ids();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
        assert!(ids.contains(&3));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn test_all_relevant_file_ids_dedup() {
        let result = QueryResult {
            ask: "test".into(),
            keywords: vec![],
            matching_files: vec![
                FileRow { id: 1, path: "a.rs".into(), language: None, size: 0, line_count: 0, is_test: false },
            ],
            matching_symbols: vec![
                SymbolRow { id: 10, file_id: 1, name: "foo".into(), kind: "function".into(),
                    line_start: 1, line_end: 10, signature: None, file_path: "a.rs".into() },
            ],
            related_tests: vec![],
            execution_paths: vec![],
            subsystems: vec![],
        };
        let ids = result.all_relevant_file_ids();
        assert_eq!(ids.len(), 1); // same file_id deduped
    }

    #[test]
    fn test_infer_subsystems_root_file_becomes_subsystem() {
        // "Makefile" has no dot, so it gets picked as a subsystem
        let files = vec![
            FileRow { id: 1, path: "Makefile".into(), language: None, size: 0, line_count: 0, is_test: false },
        ];
        let subs = infer_subsystems(&files);
        assert_eq!(subs, vec!["Makefile"]);
    }

    #[test]
    fn test_infer_subsystems_dedup() {
        let files = vec![
            FileRow { id: 1, path: "src/auth/login.py".into(), language: None, size: 0, line_count: 0, is_test: false },
            FileRow { id: 2, path: "src/auth/register.py".into(), language: None, size: 0, line_count: 0, is_test: false },
        ];
        let subs = infer_subsystems(&files);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0], "auth");
    }

    #[test]
    fn test_infer_subsystems() {
        let files = vec![
            FileRow { id: 1, path: "src/auth/login.py".into(), language: None, size: 0, line_count: 0, is_test: false },
            FileRow { id: 2, path: "src/api/routes.py".into(), language: None, size: 0, line_count: 0, is_test: false },
        ];
        let subs = infer_subsystems(&files);
        assert!(subs.contains(&"auth".to_string()));
        assert!(subs.contains(&"api".to_string()));
        assert!(!subs.contains(&"src".to_string()));
    }

    #[test]
    fn test_analyze_query_finds_files_and_symbols() -> anyhow::Result<()> {
        let db = IndexDb::open_memory()?;
        let fid = db.insert_file("src/auth/login.rs", Some("rust"), 100, 20, false, 0)?;
        db.insert_symbol(fid, "login", "function", 1, 10, None, None)?;
        db.insert_symbol(fid, "verify_password", "function", 11, 20, None, None)?;

        let result = analyze_query("login authentication", &db)?;
        assert!(result.matching_files.iter().any(|f| f.path.contains("login")));
        assert!(result.matching_symbols.iter().any(|s| s.name == "login"));
        assert!(result.keywords.contains(&"login".to_string()));
        assert!(result.keywords.contains(&"authentication".to_string()));
        Ok(())
    }

    #[test]
    fn test_analyze_query_deduplicates() -> anyhow::Result<()> {
        let db = IndexDb::open_memory()?;
        let fid = db.insert_file("src/login.rs", Some("rust"), 100, 20, false, 0)?;
        // "login" keyword matches both the file path and symbol name
        db.insert_symbol(fid, "login", "function", 1, 10, None, None)?;

        let result = analyze_query("login", &db)?;
        // File should appear only once even though it matches on path
        let login_files: Vec<_> = result.matching_files.iter().filter(|f| f.path.contains("login")).collect();
        assert_eq!(login_files.len(), 1);
        // Symbol should appear only once
        let login_syms: Vec<_> = result.matching_symbols.iter().filter(|s| s.name == "login").collect();
        assert_eq!(login_syms.len(), 1);
        Ok(())
    }

    #[test]
    fn test_analyze_query_finds_related_tests() -> anyhow::Result<()> {
        let db = IndexDb::open_memory()?;
        let src = db.insert_file("src/auth.rs", Some("rust"), 100, 20, false, 0)?;
        let test = db.insert_file("tests/test_auth.rs", Some("rust"), 50, 10, true, 0)?;
        db.insert_edge("tests", Some(test), None, Some(src), None, None)?;

        let result = analyze_query("auth", &db)?;
        assert!(!result.related_tests.is_empty());
        assert!(result.related_tests.iter().any(|t| t.path.contains("test_auth")));
        Ok(())
    }

    #[test]
    fn test_analyze_query_traces_execution_paths() -> anyhow::Result<()> {
        let db = IndexDb::open_memory()?;
        let fid = db.insert_file("src/handler.rs", Some("rust"), 200, 50, false, 0)?;
        let handler = db.insert_symbol(fid, "handle_request", "function", 1, 10, None, None)?;
        db.insert_symbol(fid, "validate", "function", 11, 20, None, None)?;
        db.insert_call(handler, "validate", 5)?;

        let result = analyze_query("handle_request", &db)?;
        assert!(!result.execution_paths.is_empty());
        // First path should start with handle_request and include validate
        let path = &result.execution_paths[0];
        assert!(path.iter().any(|s| s.name == "handle_request"));
        assert!(path.iter().any(|s| s.name == "validate"));
        Ok(())
    }

    #[test]
    fn test_analyze_query_no_matches() -> anyhow::Result<()> {
        let db = IndexDb::open_memory()?;
        db.insert_file("src/main.rs", Some("rust"), 100, 10, false, 0)?;

        let result = analyze_query("nonexistent_symbol", &db)?;
        assert!(result.matching_symbols.is_empty());
        assert!(result.execution_paths.is_empty());
        Ok(())
    }

    #[test]
    fn test_analyze_query_infers_subsystems() -> anyhow::Result<()> {
        let db = IndexDb::open_memory()?;
        db.insert_file("src/auth/login.rs", Some("rust"), 100, 20, false, 0)?;
        db.insert_file("src/api/handler.rs", Some("rust"), 200, 40, false, 0)?;

        let result = analyze_query("auth api", &db)?;
        assert!(result.subsystems.contains(&"auth".to_string()));
        assert!(result.subsystems.contains(&"api".to_string()));
        Ok(())
    }

    #[test]
    fn test_trace_execution_path_cte_builds_path() -> anyhow::Result<()> {
        let db = IndexDb::open_memory()?;
        let fid = db.insert_file("src/lib.rs", Some("rust"), 200, 50, false, 0)?;
        let a = db.insert_symbol(fid, "a", "function", 1, 10, None, None)?;
        db.insert_symbol(fid, "b", "function", 11, 20, None, None)?;
        db.insert_call(a, "b", 5)?;

        let start = SymbolRow {
            id: a, file_id: fid, name: "a".into(), kind: "function".into(),
            line_start: 1, line_end: 10, signature: None, file_path: "src/lib.rs".into(),
        };
        let path = trace_execution_path_cte(&start, &db, 5)?;
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].name, "a");
        assert_eq!(path[0].depth, 0);
        assert_eq!(path[1].name, "b");
        assert_eq!(path[1].depth, 1);
        Ok(())
    }

    #[test]
    fn test_trace_execution_path_cte_no_calls() -> anyhow::Result<()> {
        let db = IndexDb::open_memory()?;
        let fid = db.insert_file("src/lib.rs", Some("rust"), 200, 50, false, 0)?;
        let a = db.insert_symbol(fid, "isolated", "function", 1, 10, None, None)?;

        let start = SymbolRow {
            id: a, file_id: fid, name: "isolated".into(), kind: "function".into(),
            line_start: 1, line_end: 10, signature: None, file_path: "src/lib.rs".into(),
        };
        let path = trace_execution_path_cte(&start, &db, 5)?;
        assert_eq!(path.len(), 1); // only the start node
        Ok(())
    }
}
