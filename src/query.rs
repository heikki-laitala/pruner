//! Keyword extraction + heuristic relevance matching.
//!
#![allow(dead_code)]

use crate::db::{FileRow, IndexDb, SymbolRow};
use anyhow::Result;
use std::collections::HashSet;
use std::sync::LazyLock;

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
    pub symbol_id: i64,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line_start: i64,
    pub depth: usize,
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

    // Trace execution paths from matching symbols
    let mut execution_paths = Vec::new();
    for sym in &matching_symbols {
        let path = trace_execution_path(sym, db, 5)?;
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
            keywords.push(lower.clone());
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
            parts.push(current.clone());
            current.clear();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current);
    }

    // Only return if we actually split something
    if parts.len() > 1 { parts } else { Vec::new() }
}

/// DFS through call graph from a symbol.
fn trace_execution_path(
    start: &SymbolRow,
    db: &IndexDb,
    max_depth: usize,
) -> Result<Vec<PathStep>> {
    let mut path = vec![PathStep {
        symbol_id: start.id,
        name: start.name.clone(),
        kind: start.kind.clone(),
        file_path: start.file_path.clone(),
        line_start: start.line_start,
        depth: 0,
    }];

    let mut visited = HashSet::new();
    visited.insert(start.id);

    trace_calls_dfs(start.id, db, &mut path, &mut visited, 1, max_depth)?;

    Ok(path)
}

fn trace_calls_dfs(
    symbol_id: i64,
    db: &IndexDb,
    path: &mut Vec<PathStep>,
    visited: &mut HashSet<i64>,
    depth: usize,
    max_depth: usize,
) -> Result<()> {
    if depth > max_depth {
        return Ok(());
    }

    let calls = db.calls_by_symbol(symbol_id)?;
    let mut branch_count = 0;

    for call in &calls {
        if branch_count >= 3 {
            break;
        }

        // Try to resolve callee
        let targets = db.search_symbols(&call.callee_name)?;
        if let Some(target) = targets.first()
            && visited.insert(target.id)
        {
            path.push(PathStep {
                symbol_id: target.id,
                name: target.name.clone(),
                kind: target.kind.clone(),
                file_path: target.file_path.clone(),
                line_start: target.line_start,
                depth,
            });
            branch_count += 1;

            trace_calls_dfs(target.id, db, path, visited, depth + 1, max_depth)?;
        }
    }

    Ok(())
}

/// Infer subsystems from file paths.
fn infer_subsystems(files: &[FileRow]) -> Vec<String> {
    let scaffold = HashSet::from(["src", "lib", "app", "pkg", "cmd", "internal"]);
    let mut subsystems = HashSet::new();

    for f in files {
        let parts: Vec<&str> = f.path.split('/').collect();
        for part in &parts {
            if !scaffold.contains(part) && !part.contains('.') && !part.is_empty() {
                subsystems.insert(part.to_string());
                break;
            }
        }
    }

    let mut result: Vec<_> = subsystems.into_iter().collect();
    result.sort();
    result
}

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
}
