//! Keyword extraction + heuristic relevance matching.
//!

use crate::db::{FileRow, IndexDb, SymbolRow, TraceRow};
use anyhow::Result;
use rust_stemmers::{Algorithm, Stemmer};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Result caps and thresholds
// ---------------------------------------------------------------------------

const MAX_TRACED_SYMBOLS: usize = 20;
const MAX_RESULT_SYMBOLS: usize = 40;
const MAX_RESULT_FILES: usize = 25;
const MAX_RESULT_TESTS: usize = 10;
const MIN_FILE_SCORE: i32 = 10;
const MIN_SYMBOL_SCORE: i32 = 15;
/// Drop results scoring below this fraction of the top result.
const SCORE_CUTOFF_RATIO: f64 = 0.25;
const TRACE_TIME_BUDGET: Duration = Duration::from_secs(10);

/// Keywords matching this fraction or more of files/symbols are noise.
const DYNAMIC_STOP_THRESHOLD: f64 = 0.30;
/// At least one keyword must match fewer than this fraction of files to proceed.
const MIN_SPECIFICITY_THRESHOLD: f64 = 0.05;

// ---------------------------------------------------------------------------
// Scoring weights — keyword matches
// ---------------------------------------------------------------------------

const EXACT_MATCH: i32 = 100;
const PREFIX_MATCH: i32 = 50;
const SUBSTRING_MATCH: i32 = 10;

// ---------------------------------------------------------------------------
// Scoring weights — file
// ---------------------------------------------------------------------------

const FILE_EXACT_STEM: i32 = 100;
const FILE_STEM_CONTAINS: i32 = 40;
const FILE_DIR_CONTAINS: i32 = 5;
const FILE_MULTI_KEYWORD_BONUS: i32 = 30;
const FILE_LANGUAGE_BONUS: i32 = 20;
const FILE_TEST_PENALTY: i32 = -5;
/// Stronger penalty for test files when query is not about testing.
const FILE_TEST_NON_TEST_QUERY_PENALTY: i32 = -25;
/// Penalty for generated/compiled code files.
const GENERATED_CODE_PENALTY: i32 = -40;
/// Bonus per matched symbol hosted in a file (cross-reference boost).
const FILE_SYMBOL_BOOST: i32 = 15;

// Directory penalties
const DIR_DOCS_PENALTY: i32 = -30;
const DIR_LOCALE_PENALTY: i32 = -50;
const DIR_VENDOR_PENALTY: i32 = -40;
const DIR_EXAMPLES_PENALTY: i32 = -15;
const DIR_ASSETS_PENALTY: i32 = -40;

// Minified/bundled penalties
const MINIFIED_EXT_PENALTY: i32 = -60;
const MINIFIED_RATIO_SEVERE: i32 = -200; // bytes_per_line > 1000
const MINIFIED_RATIO_MODERATE: i32 = -80; // bytes_per_line > 500

// ---------------------------------------------------------------------------
// Scoring weights — symbol
// ---------------------------------------------------------------------------

const SYM_FUNCTION_BONUS: i32 = 20;
const SYM_TYPE_BONUS: i32 = 5;

// ---------------------------------------------------------------------------
// Keyword extraction
// ---------------------------------------------------------------------------

/// Minimum length for sub-keywords split from compound identifiers.
/// Short fragments like "web" from "WebSocket" cause overly broad matches.
const MIN_SUB_KEYWORD_LEN: usize = 4;

/// Minimum length for a stemmed keyword to be useful.
/// Stems shorter than this (e.g., "us" from "use") are too broad for LIKE matching.
const MIN_STEM_LEN: usize = 4;

/// Snowball English stemmer, used to normalize query keywords so that
/// natural-language forms like "reconnection" match code identifiers like
/// "reconnect" and "reconnectPolicy".
static STEMMER: LazyLock<Stemmer> = LazyLock::new(|| Stemmer::create(Algorithm::English));

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

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
    fn empty(ask: &str, keywords: Vec<String>) -> Self {
        Self {
            ask: ask.to_string(),
            keywords,
            matching_files: vec![],
            matching_symbols: vec![],
            related_tests: vec![],
            execution_paths: vec![],
            subsystems: vec![],
        }
    }

    /// Aggregate relevance score for ranking across multiple repos.
    /// Combines file count, symbol count, execution path depth, and test coverage.
    pub fn relevance_score(&self) -> i32 {
        let file_score = self.matching_files.len() as i32 * 10;
        let symbol_score = self.matching_symbols.len() as i32 * 5;
        let path_score: i32 = self.execution_paths.iter().map(|p| p.len() as i32).sum();
        let test_score = self.related_tests.len() as i32 * 3;
        file_score + symbol_score + path_score + test_score
    }

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
        ids
    }
}

// ---------------------------------------------------------------------------
// analyze_query — main entry point
// ---------------------------------------------------------------------------

/// Analyze a natural language query against the index.
pub fn analyze_query(ask: &str, db: &IndexDb) -> Result<QueryResult> {
    // Detect non-code meta-questions early
    if is_meta_question(ask) {
        return Ok(QueryResult::empty(ask, vec![]));
    }

    let raw_keywords = extract_keywords(ask);
    // Check test intent BEFORE filtering, so "test" isn't lost as a stop-word
    let query_about_testing = is_query_about_testing(&raw_keywords);
    let (keywords, has_specific) = filter_low_specificity_keywords(&raw_keywords, db)?;

    // If no keyword is specific enough, return empty result
    if keywords.is_empty() || !has_specific {
        return Ok(QueryResult::empty(ask, keywords));
    }

    let (mut matching_files, matching_symbols) = gather_candidates(&keywords, db)?;

    // Add symbol host files so test-edge lookup covers them
    let mut seen_file_ids: HashSet<i64> = matching_files.iter().map(|f| f.id).collect();
    for sym in &matching_symbols {
        if seen_file_ids.insert(sym.file_id)
            && let Some(file) = db.get_file_by_path_id(sym.file_id)?
        {
            matching_files.push(file);
        }
    }

    let related_tests = find_related_tests(&matching_files, db)?;
    let file_scores = build_file_scores(
        &matching_files,
        &matching_symbols,
        &keywords,
        db,
        query_about_testing,
    )?;

    let (matching_symbols, top_symbols) =
        rank_and_filter_symbols(&matching_symbols, &keywords, &file_scores);
    let execution_paths = trace_paths(&top_symbols, db)?;

    // Graph expansion: add files discovered through execution paths that
    // weren't found by keyword matching. This helps when the query concept
    // (e.g. "authentication") doesn't appear in file/symbol names but is
    // reachable via the call graph from a seed match.
    let mut seen_file_ids: HashSet<i64> = matching_files.iter().map(|f| f.id).collect();
    for path in &execution_paths {
        for step in path {
            if let Some(file) = db.get_file_by_path(&step.file_path)?
                && seen_file_ids.insert(file.id)
            {
                matching_files.push(file);
            }
        }
    }

    let symbol_file_counts = count_symbols_per_file(&matching_symbols);
    let matching_files = rank_and_filter_files(
        &matching_files,
        &keywords,
        &symbol_file_counts,
        query_about_testing,
    );
    let mut related_tests = related_tests;
    related_tests.truncate(MAX_RESULT_TESTS);

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

// ---------------------------------------------------------------------------
// Non-code query detection
// ---------------------------------------------------------------------------

/// Detect meta-questions that don't benefit from codebase context.
/// Examples: "does pruner bring value?", "how should we improve this?",
/// "what's our testing strategy?", "summarize recent changes"
///
/// Only matches when the entire query looks like a process/meta question.
/// Uses whole-query substring matching but requires patterns to be specific
/// enough to avoid false positives on code queries like
/// "what is the status code returned by authenticate".
fn is_meta_question(ask: &str) -> bool {
    let lower = ask.to_lowercase();

    // Patterns that indicate meta/process questions rather than code questions.
    // Each pattern must be specific enough to not accidentally match code queries.
    const META_PATTERNS: &[&str] = &[
        "bring value",
        "how should we",
        "what should we",
        "should we use",
        "do we need",
        "is it worth",
        "what's our strategy",
        "what is our strategy",
        "summarize recent",
        "summarize the recent",
        "what changed recently",
        "who worked on",
        "when was the last",
        "how long will it",
        "how long would it",
        "estimate the effort",
        "pros and cons",
        "compare the approaches",
        "what do you think",
        "give me an overview",
        "explain the architecture",
        "how does the team",
        "what's the status of",
        "what is the status of",
        "prioritize the",
        "what's the plan for",
        "what is the plan for",
    ];

    META_PATTERNS.iter().any(|p| lower.contains(p))
}

// ---------------------------------------------------------------------------
// Keyword specificity filtering
// ---------------------------------------------------------------------------

/// Minimum repo size for dynamic stop-word filtering to kick in.
/// Below this threshold, frequency analysis isn't meaningful.
const MIN_FILES_FOR_SPECIFICITY: i64 = 10;

/// Filter keywords by repo-specific frequency and check if any are specific enough.
/// Returns (filtered_keywords, has_specific) in a single pass over the DB.
///
/// - Keywords matching 30%+ of files/symbols are dropped (dynamic stop-words)
/// - has_specific is true if at least one surviving keyword matches <5% of files/symbols
/// - Skipped for small repos (<10 files) where frequency analysis isn't meaningful
fn filter_low_specificity_keywords(
    keywords: &[String],
    db: &IndexDb,
) -> Result<(Vec<String>, bool)> {
    let total_files = db.file_count()?;
    if total_files < MIN_FILES_FOR_SPECIFICITY {
        return Ok((keywords.to_vec(), true));
    }

    let total_symbols = db.symbol_count()?.max(1);

    let mut filtered = Vec::new();
    let mut has_specific = false;

    for kw in keywords {
        let file_hits = db.count_files_matching(kw)?;
        let symbol_hits = db.count_symbols_matching(kw)?;
        let file_ratio = file_hits as f64 / total_files as f64;
        let symbol_ratio = symbol_hits as f64 / total_symbols as f64;

        // Drop keywords that are too common (repo-specific stop words)
        if file_ratio >= DYNAMIC_STOP_THRESHOLD || symbol_ratio >= DYNAMIC_STOP_THRESHOLD {
            continue;
        }

        filtered.push(kw.clone());

        // Check if this keyword is specific enough
        if file_ratio < MIN_SPECIFICITY_THRESHOLD || symbol_ratio < MIN_SPECIFICITY_THRESHOLD {
            has_specific = true;
        }
    }
    Ok((filtered, has_specific))
}

// Keep individual functions for unit test access
#[cfg(test)]
fn filter_keywords_only(keywords: &[String], db: &IndexDb) -> Result<Vec<String>> {
    filter_low_specificity_keywords(keywords, db).map(|(kw, _)| kw)
}

#[cfg(test)]
fn has_specific_keyword(keywords: &[String], db: &IndexDb) -> Result<bool> {
    filter_low_specificity_keywords(keywords, db).map(|(_, has)| has)
}

// ---------------------------------------------------------------------------
// Phase 1: Gather candidates from DB
// ---------------------------------------------------------------------------

/// Search files and symbols across all keyword-matching strategies.
/// Also searches with stemmed keyword variants to find candidates that
/// the original form would miss (e.g., "reconnection" → "reconnect"
/// finds reconnect.ts).
fn gather_candidates(keywords: &[String], db: &IndexDb) -> Result<(Vec<FileRow>, Vec<SymbolRow>)> {
    let mut files = Vec::new();
    let mut symbols = Vec::new();
    let mut seen_files = HashSet::new();
    let mut seen_symbols = HashSet::new();

    // Build search terms: original keywords + their stems (deduplicated)
    let mut search_terms: Vec<String> = Vec::new();
    let mut seen_terms = HashSet::new();
    for kw in keywords {
        if seen_terms.insert(kw.clone()) {
            search_terms.push(kw.clone());
        }
        if let Some(stemmed) = stem_keyword(kw) {
            if !STOP_WORDS.contains(stemmed.as_str()) && seen_terms.insert(stemmed.clone()) {
                search_terms.push(stemmed);
            }
        }
    }

    for term in &search_terms {
        collect_dedup(&mut files, &mut seen_files, db.search_files(term)?, |f| f.id);

        collect_dedup(
            &mut symbols,
            &mut seen_symbols,
            db.search_symbols(term)?,
            |s| s.id,
        );
        // Skip expensive cross-reference searches for short keywords — they
        // produce too many false positives (e.g. "web" matching thousands).
        if term.len() >= MIN_SUB_KEYWORD_LEN {
            collect_dedup(
                &mut symbols,
                &mut seen_symbols,
                db.search_symbols_by_signature(term)?,
                |s| s.id,
            );
            collect_dedup(
                &mut symbols,
                &mut seen_symbols,
                db.search_callers_of(term)?,
                |s| s.id,
            );
        }

        for file_id in db.search_importing_files(term)? {
            if seen_files.insert(file_id)
                && let Some(file) = db.get_file_by_path_id(file_id)?
            {
                files.push(file);
            }
        }
    }

    Ok((files, symbols))
}

/// Append items to `dest`, skipping duplicates based on an ID extractor.
fn collect_dedup<T>(
    dest: &mut Vec<T>,
    seen: &mut HashSet<i64>,
    items: Vec<T>,
    id_fn: fn(&T) -> i64,
) {
    for item in items {
        if seen.insert(id_fn(&item)) {
            dest.push(item);
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 2: Find related tests
// ---------------------------------------------------------------------------

fn find_related_tests(files: &[FileRow], db: &IndexDb) -> Result<Vec<FileRow>> {
    let mut tests = Vec::new();
    let mut seen = HashSet::new();
    for file in files {
        for edge in db.edges_to_file(file.id, "tests")? {
            if let Some(src_file_id) = edge.source_file_id
                && seen.insert(src_file_id)
                && let Some(tf) = db.get_file_by_path_id(src_file_id)?
            {
                tests.push(tf);
            }
        }
    }
    Ok(tests)
}

// ---------------------------------------------------------------------------
// Phase 3: Score and rank
// ---------------------------------------------------------------------------

/// Build a file_id → score map covering both matched files and symbol host files.
fn build_file_scores(
    files: &[FileRow],
    symbols: &[SymbolRow],
    keywords: &[String],
    db: &IndexDb,
    query_about_testing: bool,
) -> Result<HashMap<i64, i32>> {
    let no_counts = HashMap::new();
    let scored = score_and_rank_files(files, keywords, &no_counts, query_about_testing);
    let mut map: HashMap<i64, i32> = scored.iter().map(|(f, s)| (f.id, *s)).collect();

    // Score files that host matched symbols but weren't in the file results.
    for sym in symbols {
        if !map.contains_key(&sym.file_id)
            && let Some(f) = db.get_file_by_path_id(sym.file_id)?
        {
            map.insert(f.id, score_file(&f, keywords));
        }
    }
    Ok(map)
}

/// Score, apply dynamic cutoff, and cap symbols. Returns (capped list, top symbols for tracing).
fn rank_and_filter_symbols(
    symbols: &[SymbolRow],
    keywords: &[String],
    file_scores: &HashMap<i64, i32>,
) -> (Vec<SymbolRow>, Vec<SymbolRow>) {
    let scored = score_and_rank_symbols(symbols, keywords, file_scores);
    let cutoff = dynamic_cutoff(&scored, MIN_SYMBOL_SCORE);

    let top: Vec<SymbolRow> = scored
        .iter()
        .filter(|(_, s)| *s >= cutoff)
        .take(MAX_TRACED_SYMBOLS)
        .map(|(sym, _)| (*sym).clone())
        .collect();

    let all: Vec<SymbolRow> = scored
        .into_iter()
        .filter(|(_, s)| *s >= cutoff)
        .take(MAX_RESULT_SYMBOLS)
        .map(|(sym, _)| sym.clone())
        .collect();

    (all, top)
}

/// Count how many matched symbols belong to each file.
fn count_symbols_per_file(symbols: &[SymbolRow]) -> HashMap<i64, usize> {
    let mut counts = HashMap::new();
    for sym in symbols {
        *counts.entry(sym.file_id).or_insert(0) += 1;
    }
    counts
}

/// Score, apply dynamic cutoff, and cap files.
fn rank_and_filter_files(
    files: &[FileRow],
    keywords: &[String],
    symbol_counts: &HashMap<i64, usize>,
    query_about_testing: bool,
) -> Vec<FileRow> {
    let scored = score_and_rank_files(files, keywords, symbol_counts, query_about_testing);
    let cutoff = dynamic_cutoff(&scored, MIN_FILE_SCORE);

    scored
        .into_iter()
        .filter(|(_, s)| *s >= cutoff)
        .take(MAX_RESULT_FILES)
        .map(|(f, _)| f.clone())
        .collect()
}

/// The higher of `min_score` or `SCORE_CUTOFF_RATIO` × the top result's score.
fn dynamic_cutoff<T>(scored: &[(T, i32)], min_score: i32) -> i32 {
    let top = scored.first().map(|(_, s)| *s).unwrap_or(0);
    min_score.max((top as f64 * SCORE_CUTOFF_RATIO) as i32)
}

// ---------------------------------------------------------------------------
// Phase 4: Trace execution paths
// ---------------------------------------------------------------------------

fn trace_paths(top_symbols: &[SymbolRow], db: &IndexDb) -> Result<Vec<Vec<PathStep>>> {
    let mut paths = Vec::new();
    let deadline = Instant::now() + TRACE_TIME_BUDGET;
    for sym in top_symbols {
        if Instant::now() >= deadline {
            break;
        }
        let path = trace_execution_path_cte(sym, db, 5)?;
        if path.len() > 1 {
            paths.push(path);
        }
    }
    Ok(paths)
}

/// Trace call graph from a symbol using a single SQL recursive CTE.
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

// ---------------------------------------------------------------------------
// Keyword extraction
// ---------------------------------------------------------------------------

/// Extract search keywords from a natural language query.
/// Handles quoted phrases ("rate limiter") and hyphenated compounds (claude-code).
pub fn extract_keywords(ask: &str) -> Vec<String> {
    let mut keywords = Vec::new();
    let mut seen = HashSet::new();

    // Phase 1: Extract quoted phrases
    let mut remaining = ask.to_string();
    for phrase in extract_quoted_phrases(ask) {
        let lower = phrase.to_lowercase();
        if seen.insert(lower.clone()) {
            keywords.push(lower);
        }
        // Remove quoted phrase from remaining text so words aren't double-counted
        remaining = remaining.replace(&format!("\"{phrase}\""), " ");
    }

    // Phase 2: Extract individual words from remaining text
    for word in remaining.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
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

        // Split camelCase / snake_case — only keep sub-parts long enough
        // to be meaningful. Short fragments cause overly broad LIKE matches.
        for sub in split_identifier(word) {
            let sub_lower = sub.to_lowercase();
            if sub_lower.len() >= MIN_SUB_KEYWORD_LEN
                && !STOP_WORDS.contains(sub_lower.as_str())
                && seen.insert(sub_lower.clone())
            {
                keywords.push(sub_lower);
            }
        }
    }

    keywords
}

/// Extract double-quoted phrases from a query string.
fn extract_quoted_phrases(ask: &str) -> Vec<String> {
    let mut phrases = Vec::new();
    let mut chars = ask.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            let phrase: String = chars.by_ref().take_while(|&ch| ch != '"').collect();
            let trimmed = phrase.trim().to_string();
            if trimmed.contains(' ') {
                phrases.push(trimmed);
            }
        }
    }
    phrases
}

/// Stem a keyword using the Snowball English stemmer.
/// Returns the stemmed form if it differs from the original and is long enough
/// to be useful in LIKE queries. Returns `None` if stemming produced no change
/// or the stem is too short.
fn stem_keyword(kw: &str) -> Option<String> {
    let stemmed = STEMMER.stem(kw);
    let s = stemmed.as_ref();
    if s == kw || s.len() < MIN_STEM_LEN {
        return None;
    }
    Some(s.to_string())
}

/// Split a camelCase or snake_case identifier into parts.
fn split_identifier(s: &str) -> Vec<String> {
    let mut parts = Vec::new();

    if s.contains('_') {
        for part in s.split('_') {
            if !part.is_empty() {
                parts.push(part.to_string());
            }
        }
        return parts;
    }

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

    if parts.len() > 1 { parts } else { Vec::new() }
}

// ---------------------------------------------------------------------------
// Symbol scoring
// ---------------------------------------------------------------------------

fn score_symbol(sym: &SymbolRow, keywords: &[String], file_scores: &HashMap<i64, i32>) -> i32 {
    let name_lower = sym.name.to_lowercase();
    let name_stem = STEMMER.stem(&name_lower);
    let mut score: i32 = 0;

    for kw in keywords {
        let kw_stem = STEMMER.stem(kw);
        if name_lower == *kw {
            score += EXACT_MATCH;
        } else if name_lower.starts_with(kw) || kw.starts_with(&name_lower) {
            // Bidirectional prefix: "reconnect" matches "reconnectPolicy" (forward)
            // and "auth" matches keyword "authent" (reverse — abbreviation in code)
            score += PREFIX_MATCH;
        } else if name_lower.contains(kw.as_str()) {
            score += SUBSTRING_MATCH;
        } else if *name_stem == *kw_stem && kw_stem.len() >= MIN_STEM_LEN {
            // Stem match: "reconnection" and "reconnect" both stem to "reconnect"
            score += PREFIX_MATCH;
        }
    }

    match sym.kind.as_str() {
        "function" | "method" => score += SYM_FUNCTION_BONUS,
        "class" | "struct" | "trait" | "interface" => score += SYM_TYPE_BONUS,
        _ => {}
    }

    // Propagate negative file quality into symbol score
    if let Some(&fs) = file_scores.get(&sym.file_id)
        && fs < 0
    {
        score += fs;
    }

    score
}

fn score_and_rank_symbols<'a>(
    symbols: &'a [SymbolRow],
    keywords: &[String],
    file_scores: &HashMap<i64, i32>,
) -> Vec<(&'a SymbolRow, i32)> {
    let mut scored: Vec<_> = symbols
        .iter()
        .map(|s| (s, score_symbol(s, keywords, file_scores)))
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name)));
    scored
}

// ---------------------------------------------------------------------------
// File scoring
// ---------------------------------------------------------------------------

fn score_file(file: &FileRow, keywords: &[String]) -> i32 {
    let path_lower = file.path.to_lowercase();

    let keyword_score = score_file_keywords(&path_lower, keywords);
    let quality_score = score_file_quality(&path_lower, file);

    keyword_score + quality_score
}

/// Score how well keywords match the file path/name.
fn score_file_keywords(path_lower: &str, keywords: &[String]) -> i32 {
    let filename = path_lower.rsplit('/').next().unwrap_or(path_lower);
    let stem = filename
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(filename);

    let mut score: i32 = 0;
    let mut filename_hits = 0;

    // Pre-stem the file stem parts for stem-based matching below.
    // File stems may be hyphenated (e.g., "reconnect-policy"), so stem each part.
    let stem_parts_stemmed: Vec<String> = stem
        .split(|c: char| c == '-' || c == '_' || c == '.')
        .filter(|s| !s.is_empty())
        .map(|s| STEMMER.stem(s).into_owned())
        .collect();

    for kw in keywords {
        let kw_stemmed = STEMMER.stem(kw);
        if stem == *kw {
            score += FILE_EXACT_STEM;
            filename_hits += 1;
        } else if stem.contains(kw.as_str()) {
            score += FILE_STEM_CONTAINS;
            filename_hits += 1;
        } else if kw_stemmed.len() >= MIN_STEM_LEN
            && stem_parts_stemmed
                .iter()
                .any(|sp| *sp == *kw_stemmed && sp.len() >= MIN_STEM_LEN)
        {
            // Stem match: keyword "reconnection" stems to "reconnect",
            // file stem part "reconnect" also stems to "reconnect" → match
            score += FILE_STEM_CONTAINS;
            filename_hits += 1;
        } else if path_lower.contains(kw.as_str()) {
            score += FILE_DIR_CONTAINS;
        }
    }

    if filename_hits >= 2 {
        score += FILE_MULTI_KEYWORD_BONUS * (filename_hits - 1);
    }

    score
}

/// Score file quality: language, directory, minification, test status.
fn score_file_quality(path_lower: &str, file: &FileRow) -> i32 {
    let mut score: i32 = 0;

    // Directory penalties
    for segment in path_lower.split('/') {
        score += match segment {
            "docs" | "doc" | "documentation" => DIR_DOCS_PENALTY,
            "zh-cn" | "zh-tw" | "ja" | "ko" | "fr" | "de" | "es" | "pt" | "ru" | "locale"
            | "locales" | "i18n" | "translations" | "l10n" => DIR_LOCALE_PENALTY,
            "vendor" | "node_modules" | "third_party" | "third-party" => DIR_VENDOR_PENALTY,
            "examples" | "example" | "samples" | "sample" => DIR_EXAMPLES_PENALTY,
            "assets" | "dist" | "build" | "out" | "generated" | ".generated" => DIR_ASSETS_PENALTY,
            _ => 0,
        };
    }

    // Minified file extension
    if path_lower.ends_with(".min.js")
        || path_lower.ends_with(".min.css")
        || path_lower.ends_with(".bundle.js")
        || path_lower.ends_with(".bundle.css")
    {
        score += MINIFIED_EXT_PENALTY;
    }

    // High bytes-per-line ratio suggests minified/generated content
    if file.line_count > 0 {
        let bpl = file.size / file.line_count;
        if bpl > 1000 {
            score += MINIFIED_RATIO_SEVERE;
        } else if bpl > 500 {
            score += MINIFIED_RATIO_MODERATE;
        }
    }

    if file.language.is_some() {
        score += FILE_LANGUAGE_BONUS;
    }
    if file.is_test {
        score += FILE_TEST_PENALTY;
    }

    score
}

/// Score and rank files. Boosts files containing matched symbols and
/// penalizes duplicate filenames.
fn score_and_rank_files<'a>(
    files: &'a [FileRow],
    keywords: &[String],
    symbol_counts: &HashMap<i64, usize>,
    query_about_testing: bool,
) -> Vec<(&'a FileRow, i32)> {
    let mut scored: Vec<_> = files
        .iter()
        .map(|f| {
            let base = score_file(f, keywords);
            let sym_boost =
                symbol_counts.get(&f.id).copied().unwrap_or(0) as i32 * FILE_SYMBOL_BOOST;
            let mut score = base + sym_boost;

            // Stronger test file penalty when query isn't about testing
            if f.is_test && !query_about_testing {
                score += FILE_TEST_NON_TEST_QUERY_PENALTY;
            }

            // Penalize generated/compiled code
            if is_generated_code(&f.path) {
                score += GENERATED_CODE_PENALTY;
            }

            (f, score)
        })
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.path.cmp(&b.0.path)));

    // 3rd+ file with same name gets halved score
    let mut name_counts: HashMap<&str, usize> = HashMap::new();
    let mut had_dupes = false;
    for (file, score) in &mut scored {
        let filename = file.path.rsplit('/').next().unwrap_or(&file.path);
        let count = name_counts.entry(filename).or_insert(0);
        *count += 1;
        if *count > 2 {
            *score /= 2;
            had_dupes = true;
        }
    }
    if had_dupes {
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.path.cmp(&b.0.path)));
    }
    scored
}

/// Check if the query is about testing (test files should not be penalized).
fn is_query_about_testing(keywords: &[String]) -> bool {
    const TEST_KEYWORDS: &[&str] = &[
        "test", "tests", "testing", "spec", "specs", "unittest", "pytest", "jest", "mocha",
        "coverage", "mock", "mocks", "fixture", "fixtures",
    ];
    keywords
        .iter()
        .any(|kw| TEST_KEYWORDS.contains(&kw.as_str()))
}

/// Detect generated/compiled code from file path patterns.
fn is_generated_code(path: &str) -> bool {
    let path_lower = path.to_lowercase();
    let filename = path_lower.rsplit('/').next().unwrap_or(&path_lower);

    // Generated file patterns
    filename.ends_with(".generated.ts")
        || filename.ends_with(".generated.js")
        || filename.ends_with(".gen.go")
        || filename.ends_with(".pb.go")
        || filename.ends_with(".pb.rs")
        || filename.ends_with("_generated.rs")
        || filename.ends_with("_generated.py")
        || filename.ends_with(".g.dart")
        || filename.ends_with(".freezed.dart")
        // Common generated directories
        || path_lower.contains("/generated/")
        || path_lower.contains("/__generated__/")
        || path_lower.contains("/.generated/")
}

// ---------------------------------------------------------------------------
// Subsystem inference
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Static data
// ---------------------------------------------------------------------------

static SCAFFOLD_DIRS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["src", "lib", "app", "pkg", "cmd", "internal"]
        .into_iter()
        .collect()
});

static STOP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "a",
        "an",
        "the",
        "is",
        "are",
        "was",
        "were",
        "be",
        "been",
        "being",
        "have",
        "has",
        "had",
        "do",
        "does",
        "did",
        "will",
        "would",
        "shall",
        "should",
        "may",
        "might",
        "must",
        "can",
        "could",
        "i",
        "me",
        "my",
        "we",
        "our",
        "you",
        "your",
        "he",
        "she",
        "it",
        "they",
        "them",
        "their",
        "this",
        "that",
        "these",
        "those",
        "what",
        "which",
        "who",
        "whom",
        "where",
        "when",
        "why",
        "how",
        "not",
        "no",
        "nor",
        "but",
        "or",
        "and",
        "if",
        "then",
        "else",
        "than",
        "too",
        "very",
        "just",
        "about",
        "above",
        "after",
        "again",
        "all",
        "also",
        "any",
        "because",
        "before",
        "between",
        "both",
        "by",
        "each",
        "for",
        "from",
        "get",
        "got",
        "here",
        "in",
        "into",
        "of",
        "on",
        "once",
        "only",
        "other",
        "out",
        "over",
        "own",
        "same",
        "so",
        "some",
        "such",
        "there",
        "through",
        "to",
        "under",
        "until",
        "up",
        "want",
        "with",
        "fix",
        "add",
        "make",
        "use",
        "find",
        "show",
        "change",
        "update",
        "need",
        "like",
        "work",
        "look",
        "way",
        "new",
        "file",
        "files",
        "code",
        "implement",
        "create",
    ]
    .into_iter()
    .collect()
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::IndexDb;

    fn no_file_scores() -> HashMap<i64, i32> {
        HashMap::new()
    }

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
            id: 1,
            file_id: 1,
            name: "login".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string()], &no_file_scores());
        assert_eq!(score, EXACT_MATCH + SYM_FUNCTION_BONUS);
    }

    #[test]
    fn test_score_symbol_prefix_match() {
        let sym = SymbolRow {
            id: 1,
            file_id: 1,
            name: "login_user".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string()], &no_file_scores());
        assert_eq!(score, PREFIX_MATCH + SYM_FUNCTION_BONUS);
    }

    #[test]
    fn test_score_symbol_substring_match() {
        let sym = SymbolRow {
            id: 1,
            file_id: 1,
            name: "handle_login_request".into(),
            kind: "struct".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string()], &no_file_scores());
        assert_eq!(score, SUBSTRING_MATCH + SYM_TYPE_BONUS);
    }

    #[test]
    fn test_score_and_rank_symbols() {
        let symbols = vec![
            SymbolRow {
                id: 1,
                file_id: 1,
                name: "handle_timeout".into(),
                kind: "function".into(),
                line_start: 1,
                line_end: 10,
                signature: None,
                file_path: "a.rs".into(),
            },
            SymbolRow {
                id: 2,
                file_id: 1,
                name: "timeout".into(),
                kind: "function".into(),
                line_start: 20,
                line_end: 30,
                signature: None,
                file_path: "a.rs".into(),
            },
            SymbolRow {
                id: 3,
                file_id: 1,
                name: "timeout_handler".into(),
                kind: "function".into(),
                line_start: 40,
                line_end: 50,
                signature: None,
                file_path: "a.rs".into(),
            },
        ];
        let ranked = score_and_rank_symbols(&symbols, &["timeout".to_string()], &no_file_scores());
        assert_eq!(ranked[0].0.id, 2); // exact match first
        assert_eq!(ranked[1].0.id, 3); // prefix match second
        assert_eq!(ranked[2].0.id, 1); // substring match last
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
        let parts = split_identifier("login");
        assert!(parts.is_empty());
    }

    #[test]
    fn test_split_identifier_all_upper() {
        let parts = split_identifier("API");
        assert_eq!(parts, vec!["A", "P", "I"]);
    }

    #[test]
    fn test_score_symbol_no_match() {
        let sym = SymbolRow {
            id: 1,
            file_id: 1,
            name: "unrelated".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string()], &no_file_scores());
        assert_eq!(score, SYM_FUNCTION_BONUS);
    }

    #[test]
    fn test_score_symbol_multiple_keywords() {
        let sym = SymbolRow {
            id: 1,
            file_id: 1,
            name: "login_handler".into(),
            kind: "method".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "a.rs".into(),
        };
        let score = score_symbol(
            &sym,
            &["login".to_string(), "handler".to_string()],
            &no_file_scores(),
        );
        // login: prefix 50, handler: substring 10 => 60 + 20 method bonus
        assert_eq!(score, PREFIX_MATCH + SUBSTRING_MATCH + SYM_FUNCTION_BONUS);
    }

    #[test]
    fn test_score_symbol_unknown_kind() {
        let sym = SymbolRow {
            id: 1,
            file_id: 1,
            name: "login".into(),
            kind: "variable".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["login".to_string()], &no_file_scores());
        assert_eq!(score, EXACT_MATCH);
    }

    #[test]
    fn test_score_and_rank_empty() {
        let ranked = score_and_rank_symbols(&[], &["login".to_string()], &no_file_scores());
        assert!(ranked.is_empty());
    }

    #[test]
    fn test_all_relevant_file_ids() {
        let result = QueryResult {
            ask: "test".into(),
            keywords: vec![],
            matching_files: vec![FileRow {
                id: 1,
                path: "a.rs".into(),
                language: None,
                size: 0,
                line_count: 0,
                is_test: false,
            }],
            matching_symbols: vec![SymbolRow {
                id: 10,
                file_id: 2,
                name: "foo".into(),
                kind: "function".into(),
                line_start: 1,
                line_end: 10,
                signature: None,
                file_path: "b.rs".into(),
            }],
            related_tests: vec![FileRow {
                id: 3,
                path: "test_a.rs".into(),
                language: None,
                size: 0,
                line_count: 0,
                is_test: true,
            }],
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
            matching_files: vec![FileRow {
                id: 1,
                path: "a.rs".into(),
                language: None,
                size: 0,
                line_count: 0,
                is_test: false,
            }],
            matching_symbols: vec![SymbolRow {
                id: 10,
                file_id: 1,
                name: "foo".into(),
                kind: "function".into(),
                line_start: 1,
                line_end: 10,
                signature: None,
                file_path: "a.rs".into(),
            }],
            related_tests: vec![],
            execution_paths: vec![],
            subsystems: vec![],
        };
        let ids = result.all_relevant_file_ids();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn test_score_file_exact_filename_match() {
        let file = FileRow {
            id: 1,
            path: "src/auth/websocket.rs".into(),
            language: Some("rust".into()),
            size: 200,
            line_count: 50,
            is_test: false,
        };
        let score = score_file(&file, &["websocket".to_string()]);
        assert_eq!(score, FILE_EXACT_STEM + FILE_LANGUAGE_BONUS);
    }

    #[test]
    fn test_score_file_docs_penalty() {
        let file = FileRow {
            id: 1,
            path: "docs/api/websocket.md".into(),
            language: None,
            size: 500,
            line_count: 100,
            is_test: false,
        };
        let score = score_file(&file, &["websocket".to_string()]);
        assert_eq!(score, FILE_EXACT_STEM + DIR_DOCS_PENALTY);
    }

    #[test]
    fn test_score_file_locale_penalty() {
        let file = FileRow {
            id: 1,
            path: "docs/zh-CN/api/websocket.md".into(),
            language: None,
            size: 500,
            line_count: 100,
            is_test: false,
        };
        let score = score_file(&file, &["websocket".to_string()]);
        assert_eq!(
            score,
            FILE_EXACT_STEM + DIR_DOCS_PENALTY + DIR_LOCALE_PENALTY
        );
    }

    #[test]
    fn test_score_file_source_beats_docs() {
        let src = FileRow {
            id: 1,
            path: "src/net/websocket.rs".into(),
            language: Some("rust".into()),
            size: 200,
            line_count: 50,
            is_test: false,
        };
        let doc = FileRow {
            id: 2,
            path: "docs/api/websocket.md".into(),
            language: None,
            size: 500,
            line_count: 100,
            is_test: false,
        };
        let src_score = score_file(&src, &["websocket".to_string()]);
        let doc_score = score_file(&doc, &["websocket".to_string()]);
        assert!(
            src_score > doc_score,
            "source ({src_score}) should beat doc ({doc_score})"
        );
    }

    #[test]
    fn test_score_file_multiple_keywords() {
        let file = FileRow {
            id: 1,
            path: "src/auth/token_validator.rs".into(),
            language: Some("rust".into()),
            size: 200,
            line_count: 50,
            is_test: false,
        };
        let score = score_file(
            &file,
            &[
                "auth".to_string(),
                "token".to_string(),
                "validator".to_string(),
            ],
        );
        // stem "token_validator" contains "token" (40) + "validator" (40) → 2 hits → +30 multi bonus
        // "auth" dir only (5) + language (20)
        let expected = FILE_STEM_CONTAINS * 2
            + FILE_MULTI_KEYWORD_BONUS
            + FILE_DIR_CONTAINS
            + FILE_LANGUAGE_BONUS;
        assert_eq!(score, expected);
    }

    #[test]
    fn test_score_file_dir_only_match_below_threshold() {
        let file = FileRow {
            id: 1,
            path: "src/auth/utils.rs".into(),
            language: Some("rust".into()),
            size: 100,
            line_count: 20,
            is_test: false,
        };
        let score = score_file(&file, &["auth".to_string()]);
        assert_eq!(score, FILE_DIR_CONTAINS + FILE_LANGUAGE_BONUS);
        let score2 = score_file(&file, &["handler".to_string()]);
        assert_eq!(score2, FILE_LANGUAGE_BONUS);
    }

    #[test]
    fn test_score_and_rank_files_ordering() {
        let files = vec![
            FileRow {
                id: 1,
                path: "docs/zh-CN/websocket.md".into(),
                language: None,
                size: 500,
                line_count: 100,
                is_test: false,
            },
            FileRow {
                id: 2,
                path: "src/net/websocket.rs".into(),
                language: Some("rust".into()),
                size: 200,
                line_count: 50,
                is_test: false,
            },
            FileRow {
                id: 3,
                path: "docs/websocket.md".into(),
                language: None,
                size: 300,
                line_count: 60,
                is_test: false,
            },
        ];
        let no_counts = HashMap::new();
        let ranked = score_and_rank_files(&files, &["websocket".to_string()], &no_counts, false);
        assert_eq!(ranked[0].0.id, 2, "source file should rank first");
        assert_eq!(ranked[1].0.id, 3, "docs should rank second");
        assert_eq!(ranked[2].0.id, 1, "zh-CN docs should rank last");
    }

    #[test]
    fn test_infer_subsystems_root_file_becomes_subsystem() {
        let files = vec![FileRow {
            id: 1,
            path: "Makefile".into(),
            language: None,
            size: 0,
            line_count: 0,
            is_test: false,
        }];
        let subs = infer_subsystems(&files);
        assert_eq!(subs, vec!["Makefile"]);
    }

    #[test]
    fn test_infer_subsystems_dedup() {
        let files = vec![
            FileRow {
                id: 1,
                path: "src/auth/login.py".into(),
                language: None,
                size: 0,
                line_count: 0,
                is_test: false,
            },
            FileRow {
                id: 2,
                path: "src/auth/register.py".into(),
                language: None,
                size: 0,
                line_count: 0,
                is_test: false,
            },
        ];
        let subs = infer_subsystems(&files);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0], "auth");
    }

    #[test]
    fn test_infer_subsystems() {
        let files = vec![
            FileRow {
                id: 1,
                path: "src/auth/login.py".into(),
                language: None,
                size: 0,
                line_count: 0,
                is_test: false,
            },
            FileRow {
                id: 2,
                path: "src/api/routes.py".into(),
                language: None,
                size: 0,
                line_count: 0,
                is_test: false,
            },
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
        assert!(
            result
                .matching_files
                .iter()
                .any(|f| f.path.contains("login"))
        );
        assert!(result.matching_symbols.iter().any(|s| s.name == "login"));
        assert!(result.keywords.contains(&"login".to_string()));
        assert!(result.keywords.contains(&"authentication".to_string()));
        Ok(())
    }

    #[test]
    fn test_analyze_query_deduplicates() -> anyhow::Result<()> {
        let db = IndexDb::open_memory()?;
        let fid = db.insert_file("src/login.rs", Some("rust"), 100, 20, false, 0)?;
        db.insert_symbol(fid, "login", "function", 1, 10, None, None)?;

        let result = analyze_query("login", &db)?;
        let login_files: Vec<_> = result
            .matching_files
            .iter()
            .filter(|f| f.path.contains("login"))
            .collect();
        assert_eq!(login_files.len(), 1);
        let login_syms: Vec<_> = result
            .matching_symbols
            .iter()
            .filter(|s| s.name == "login")
            .collect();
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
        assert!(
            result
                .related_tests
                .iter()
                .any(|t| t.path.contains("test_auth"))
        );
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
            id: a,
            file_id: fid,
            name: "a".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "src/lib.rs".into(),
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
            id: a,
            file_id: fid,
            name: "isolated".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "src/lib.rs".into(),
        };
        let path = trace_execution_path_cte(&start, &db, 5)?;
        assert_eq!(path.len(), 1);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Dynamic stop-words and keyword specificity
    // -----------------------------------------------------------------------

    #[test]
    fn test_filter_low_specificity_keywords_drops_common() {
        // "value" appears in 4/10 files (40%) → should be dropped
        let db = IndexDb::open_memory().unwrap();
        for i in 0..10 {
            let name = if i < 4 {
                format!("src/value_{i}.py")
            } else {
                format!("src/other_{i}.py")
            };
            db.insert_file(&name, Some("python"), 100, 10, false, 0)
                .unwrap();
        }
        let keywords = vec!["value".to_string(), "auth".to_string()];
        let filtered = filter_keywords_only(&keywords, &db).unwrap();
        assert!(!filtered.contains(&"value".to_string()));
        assert!(filtered.contains(&"auth".to_string()));
    }

    #[test]
    fn test_filter_low_specificity_keeps_specific_keywords() {
        // "auth" appears in 1/20 files (5%) → should be kept
        let db = IndexDb::open_memory().unwrap();
        db.insert_file("src/auth.py", Some("python"), 100, 10, false, 0)
            .unwrap();
        for i in 0..19 {
            db.insert_file(
                &format!("src/module_{i}.py"),
                Some("python"),
                100,
                10,
                false,
                0,
            )
            .unwrap();
        }
        let keywords = vec!["auth".to_string()];
        let filtered = filter_keywords_only(&keywords, &db).unwrap();
        assert!(filtered.contains(&"auth".to_string()));
    }

    #[test]
    fn test_filter_low_specificity_checks_symbol_frequency() {
        // Keyword doesn't match file paths but matches 40% of symbols → dropped
        let db = IndexDb::open_memory().unwrap();
        for i in 0..10 {
            let fid = db
                .insert_file(
                    &format!("src/mod_{i}.py"),
                    Some("python"),
                    100,
                    10,
                    false,
                    0,
                )
                .unwrap();
            // 4 out of 10 files have a symbol containing "value"
            if i < 4 {
                db.insert_symbol(fid, "get_value", "function", i * 10, i * 10 + 5, None, None)
                    .unwrap();
            } else {
                db.insert_symbol(fid, "process", "function", i * 10, i * 10 + 5, None, None)
                    .unwrap();
            }
        }
        let keywords = vec!["value".to_string()];
        let filtered = filter_keywords_only(&keywords, &db).unwrap();
        assert!(!filtered.contains(&"value".to_string()));
    }

    #[test]
    fn test_has_specific_keyword() {
        let db = IndexDb::open_memory().unwrap();
        let fid = db
            .insert_file("src/auth.py", Some("python"), 100, 10, false, 0)
            .unwrap();
        db.insert_symbol(fid, "authenticate", "function", 1, 10, None, None)
            .unwrap();
        for i in 0..20 {
            let fid = db
                .insert_file(
                    &format!("src/module_{i}.py"),
                    Some("python"),
                    100,
                    10,
                    false,
                    0,
                )
                .unwrap();
            db.insert_symbol(
                fid,
                &format!("module_{i}_init"),
                "function",
                1,
                10,
                None,
                None,
            )
            .unwrap();
        }
        // "auth" matches 1/21 files ≈ 4.8% < 5% → specific
        assert!(has_specific_keyword(&["auth".to_string()], &db).unwrap());
        // "module" matches 20/21 files ≈ 95% and 20/21 symbols → not specific
        assert!(!has_specific_keyword(&["module".to_string()], &db).unwrap());
    }

    #[test]
    fn test_analyze_query_returns_empty_for_nonspecific_query() {
        // All keywords match 30%+ of files → no results
        let db = IndexDb::open_memory().unwrap();
        for i in 0..10 {
            db.insert_file(
                &format!("src/value_{i}.py"),
                Some("python"),
                100,
                10,
                false,
                0,
            )
            .unwrap();
        }
        let result = analyze_query("find the value", &db).unwrap();
        // "find" and "the" are stop words, "value" matches 100% of files → empty
        assert!(result.matching_files.is_empty());
        assert!(result.matching_symbols.is_empty());
    }

    // -----------------------------------------------------------------------
    // Multi-word phrase handling
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_keywords_preserves_hyphenated_phrases() {
        let kws = extract_keywords("fix the claude-code integration");
        // Should contain "claude-code" as a phrase
        assert!(kws.contains(&"claude-code".to_string()));
    }

    #[test]
    fn test_extract_keywords_quoted_phrase() {
        let kws = extract_keywords("find \"rate limiter\" in the code");
        assert!(kws.contains(&"rate limiter".to_string()));
    }

    // -----------------------------------------------------------------------
    // Non-code query detection (#9)
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_meta_question_detects_process_questions() {
        assert!(is_meta_question("does pruner bring value?"));
        assert!(is_meta_question("how should we improve this?"));
        assert!(is_meta_question("what's the status of the migration?"));
        assert!(is_meta_question("summarize recent changes"));
        assert!(is_meta_question("can you summarize recent changes"));
        assert!(is_meta_question("what is the plan for next sprint?"));
    }

    #[test]
    fn test_is_meta_question_allows_code_questions() {
        assert!(!is_meta_question("fix the login bug"));
        assert!(!is_meta_question("how does authentication work?"));
        assert!(!is_meta_question("add rate limiting to the API"));
        assert!(!is_meta_question("where is the JWT validation?"));
        // P1 fix: code queries containing meta-pattern substrings must not be blocked
        assert!(!is_meta_question(
            "what is the status code returned by authenticate"
        ));
        assert!(!is_meta_question("what is the plan_id field used for?"));
    }

    #[test]
    fn test_analyze_query_returns_empty_for_meta_question() {
        let db = IndexDb::open_memory().unwrap();
        db.insert_file("src/auth.rs", Some("rust"), 100, 10, false, 0)
            .unwrap();
        let result = analyze_query("does pruner bring value?", &db).unwrap();
        assert!(result.matching_files.is_empty());
        assert!(result.matching_symbols.is_empty());
    }

    // -----------------------------------------------------------------------
    // Negative scoring signals (#11)
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_query_about_testing() {
        assert!(is_query_about_testing(&["test".into(), "auth".into()]));
        assert!(is_query_about_testing(&["jest".into(), "config".into()]));
        assert!(!is_query_about_testing(&["auth".into(), "login".into()]));
    }

    #[test]
    fn test_is_generated_code() {
        assert!(is_generated_code("src/types.generated.ts"));
        assert!(is_generated_code("api/service.pb.go"));
        assert!(is_generated_code("src/__generated__/schema.ts"));
        assert!(is_generated_code("lib/model_generated.rs"));
        assert!(!is_generated_code("src/auth/login.rs"));
        assert!(!is_generated_code("src/utils/helpers.ts"));
    }

    #[test]
    fn test_test_files_penalized_for_non_test_query() {
        let test_file = FileRow {
            id: 1,
            path: "tests/test_auth.rs".into(),
            language: Some("rust".into()),
            size: 200,
            line_count: 20,
            is_test: true,
        };
        let src_file = FileRow {
            id: 2,
            path: "src/auth.rs".into(),
            language: Some("rust".into()),
            size: 200,
            line_count: 20,
            is_test: false,
        };
        let keywords = vec!["auth".to_string()];
        let no_counts = HashMap::new();
        let files = [test_file, src_file];
        let ranked = score_and_rank_files(&files, &keywords, &no_counts, false);
        // Source file should rank higher than test file for non-test query
        assert_eq!(ranked[0].0.path, "src/auth.rs");
    }

    #[test]
    fn test_test_files_not_penalized_for_test_query() {
        let test_file = FileRow {
            id: 1,
            path: "tests/test_auth.rs".into(),
            language: Some("rust".into()),
            size: 200,
            line_count: 20,
            is_test: true,
        };
        let src_file = FileRow {
            id: 2,
            path: "src/auth.rs".into(),
            language: Some("rust".into()),
            size: 200,
            line_count: 20,
            is_test: false,
        };
        // "test" keyword means query is about testing
        let keywords = vec!["test".to_string(), "auth".to_string()];
        let no_counts = HashMap::new();
        let files = [test_file, src_file];
        let ranked = score_and_rank_files(&files, &keywords, &no_counts, true);
        // Test file should not be heavily penalized when query is about testing
        let test_score = ranked.iter().find(|(f, _)| f.is_test).unwrap().1;
        let src_score = ranked.iter().find(|(f, _)| !f.is_test).unwrap().1;
        // Test file should score close to or higher than source (not -25 penalty)
        assert!(test_score > src_score - 10);
    }

    #[test]
    fn test_generated_files_penalized() {
        let gen_file = FileRow {
            id: 1,
            path: "src/types.generated.ts".into(),
            language: Some("typescript".into()),
            size: 5000,
            line_count: 200,
            is_test: false,
        };
        let src_file = FileRow {
            id: 2,
            path: "src/types.ts".into(),
            language: Some("typescript".into()),
            size: 200,
            line_count: 20,
            is_test: false,
        };
        let keywords = vec!["types".to_string()];
        let no_counts = HashMap::new();
        let files = [gen_file, src_file];
        let ranked = score_and_rank_files(&files, &keywords, &no_counts, false);
        // Source file should rank higher than generated file
        assert_eq!(ranked[0].0.path, "src/types.ts");
    }

    #[test]
    fn test_relevance_score_proportional() {
        let empty = QueryResult {
            ask: "test".into(),
            keywords: vec![],
            matching_files: vec![],
            matching_symbols: vec![],
            related_tests: vec![],
            execution_paths: vec![],
            subsystems: vec![],
        };
        assert_eq!(empty.relevance_score(), 0);

        let db = IndexDb::open_memory().unwrap();
        db.insert_file("a.py", Some("python"), 10, 100, false, 0)
            .unwrap();
        let files = db.search_files("a.py").unwrap();
        let with_files = QueryResult {
            matching_files: files,
            ..empty
        };
        assert!(with_files.relevance_score() > 0);
    }

    #[test]
    fn test_file_sort_deterministic_with_equal_scores() {
        let files = vec![
            FileRow {
                id: 1,
                path: "src/zebra.rs".into(),
                language: Some("rust".into()),
                size: 100,
                line_count: 10,
                is_test: false,
            },
            FileRow {
                id: 2,
                path: "src/alpha.rs".into(),
                language: Some("rust".into()),
                size: 100,
                line_count: 10,
                is_test: false,
            },
        ];
        // Both files have no keyword match → equal scores
        let ranked = score_and_rank_files(&files, &["unrelated".into()], &HashMap::new(), false);
        assert_eq!(ranked.len(), 2);
        // Alphabetical tiebreaker: alpha before zebra
        assert!(ranked[0].0.path < ranked[1].0.path);
    }

    #[test]
    fn test_symbol_sort_deterministic_with_equal_scores() {
        let symbols = vec![
            SymbolRow {
                id: 1,
                file_id: 1,
                name: "zebra".into(),
                kind: "function".into(),
                line_start: 1,
                line_end: 5,
                signature: None,
                file_path: "a.rs".into(),
            },
            SymbolRow {
                id: 2,
                file_id: 1,
                name: "alpha".into(),
                kind: "function".into(),
                line_start: 10,
                line_end: 15,
                signature: None,
                file_path: "a.rs".into(),
            },
        ];
        // Neither matches "unrelated" → equal scores
        let ranked = score_and_rank_symbols(&symbols, &["unrelated".into()], &no_file_scores());
        assert_eq!(ranked.len(), 2);
        // Alphabetical tiebreaker: alpha before zebra
        assert_eq!(ranked[0].0.name, "alpha");
        assert_eq!(ranked[1].0.name, "zebra");
    }

    // -----------------------------------------------------------------------
    // Stemming tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_stem_keyword_basic() {
        // Natural language → code form
        assert_eq!(stem_keyword("reconnection"), Some("reconnect".into()));
        assert_eq!(stem_keyword("authentication"), Some("authent".into()));
        assert_eq!(stem_keyword("validation"), Some("valid".into()));
        assert_eq!(stem_keyword("loading"), Some("load".into()));
    }

    #[test]
    fn test_stem_keyword_no_change() {
        // Already a root form or too short to stem
        assert_eq!(stem_keyword("reconnect"), None);
        assert_eq!(stem_keyword("auth"), None);
        assert_eq!(stem_keyword("load"), None);
    }

    #[test]
    fn test_stem_keyword_too_short() {
        // Stem shorter than MIN_STEM_LEN should return None
        assert_eq!(stem_keyword("use"), None); // would stem to "us" (2 chars)
    }

    #[test]
    fn test_score_symbol_stem_match() {
        // keyword "reconnection" should match symbol "reconnect" via stem
        let sym = SymbolRow {
            id: 1,
            file_id: 1,
            name: "reconnect".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["reconnection".to_string()], &no_file_scores());
        assert_eq!(score, PREFIX_MATCH + SYM_FUNCTION_BONUS);
    }

    #[test]
    fn test_score_symbol_stem_no_false_positive() {
        // "routes" stems to "rout", "router" stems to "router" — different stems
        let sym = SymbolRow {
            id: 1,
            file_id: 1,
            name: "router".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "a.rs".into(),
        };
        // "routes" contains "rout" which is a substring of "router", so it matches
        // via substring, not stem. Stem match would be: stem("routes")="rout",
        // stem("router")="router" — different, so no stem match.
        let score = score_symbol(&sym, &["routes".to_string()], &no_file_scores());
        // "router" contains "rout" (from kw "routes"? No — "routes" is the kw,
        // check: "router".contains("routes") = false. starts_with? No.
        // Stem check: stem("routes")="rout", stem("router")="router" — different.
        // So no keyword match at all, just function bonus.
        assert_eq!(score, SYM_FUNCTION_BONUS);
    }

    #[test]
    fn test_score_file_stem_match() {
        // keyword "reconnection" should match file "reconnect.ts" via stem
        let file = FileRow {
            id: 1,
            path: "extensions/whatsapp/src/reconnect.ts".into(),
            language: Some("typescript".into()),
            size: 1500,
            line_count: 50,
            is_test: false,
        };
        let score = score_file(&file, &["reconnection".to_string()]);
        assert_eq!(score, FILE_STEM_CONTAINS + FILE_LANGUAGE_BONUS);
    }

    #[test]
    fn test_score_file_stem_hyphenated() {
        // keyword "reconnection" should match "reconnect-policy.ts" via stem
        let file = FileRow {
            id: 1,
            path: "extensions/slack/src/monitor/reconnect-policy.ts".into(),
            language: Some("typescript".into()),
            size: 2000,
            line_count: 60,
            is_test: false,
        };
        let score = score_file(&file, &["reconnection".to_string()]);
        assert_eq!(score, FILE_STEM_CONTAINS + FILE_LANGUAGE_BONUS);
    }

    #[test]
    fn test_score_symbol_bidirectional_prefix() {
        // "auth" in code should match keyword "authentication" via reverse prefix
        let sym = SymbolRow {
            id: 1,
            file_id: 1,
            name: "auth".into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 10,
            signature: None,
            file_path: "a.rs".into(),
        };
        let score = score_symbol(&sym, &["authentication".to_string()], &no_file_scores());
        // "authentication".starts_with("auth") → true → PREFIX_MATCH
        assert_eq!(score, PREFIX_MATCH + SYM_FUNCTION_BONUS);
    }
}
