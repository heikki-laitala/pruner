//! Token estimation and usage measurement.
//!

use crate::context::{self, format_context_text, generate_context};
use crate::db::IndexDb;
use crate::languages;
use crate::query::QueryResult;
use anyhow::Result;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use walkdir::WalkDir;

/// Estimate token count using a regex heuristic.
/// Approximates ~10-15% accuracy vs real tokenizers.
pub fn estimate_tokens(text: &str) -> usize {
    TOKEN_RE.find_iter(text).count()
}

static TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\w+|[^\w\s]|\n").unwrap());

/// Token usage comparison between naive and pruner approaches.
pub struct Measurement {
    pub ask: String,
    pub naive_files: Vec<String>,
    pub naive_tokens: usize,
    pub naive_lines: usize,
    pub pruner_tokens_text: usize,
    pub pruner_tokens_json: usize,
    pub pruner_files: usize,
    pub pruner_symbols: usize,
    pub pruner_snippets: usize,
    pub repo_total_tokens: usize,
    pub repo_total_files: usize,
}

impl Measurement {
    pub fn reduction_vs_naive(&self) -> f64 {
        if self.naive_tokens == 0 {
            return 0.0;
        }
        (1.0 - self.pruner_tokens_text as f64 / self.naive_tokens as f64) * 100.0
    }

    pub fn reduction_vs_repo(&self) -> f64 {
        if self.repo_total_tokens == 0 {
            return 0.0;
        }
        (1.0 - self.pruner_tokens_text as f64 / self.repo_total_tokens as f64) * 100.0
    }
}

/// Measure token usage: pruner context vs naive full-file inclusion.
pub fn measure(
    query_result: &QueryResult,
    db: &IndexDb,
    repo_path: &Path,
    max_snippet_lines: usize,
) -> Result<Measurement> {
    // Generate pruner context
    let ctx = generate_context(query_result, db, repo_path, max_snippet_lines, false)?;
    let text_output = format_context_text(&ctx);
    let json_output = context::format_context_json(&ctx)?;

    // Naive approach: read full contents of all relevant files
    let relevant_file_ids = query_result.all_relevant_file_ids();
    let mut naive_content = String::new();
    let mut naive_files = Vec::new();
    let mut naive_lines: usize = 0;

    for fid in &relevant_file_ids {
        if let Some(f) = db.get_file_by_path_id(*fid)? {
            let fpath = repo_path.join(&f.path);
            if let Ok(content) = fs::read_to_string(&fpath) {
                naive_content.push_str(&format!("\n\n--- {} ---\n\n{}", f.path, content));
                naive_files.push(f.path.clone());
                naive_lines += content.lines().count() + 1;
            }
        }
    }

    // Whole repo baseline
    let mut repo_tokens = 0;
    let mut repo_files = 0;
    for f in db.all_files()? {
        let fpath = repo_path.join(&f.path);
        if let Ok(content) = fs::read_to_string(&fpath) {
            repo_tokens += estimate_tokens(&content);
            repo_files += 1;
        }
    }

    Ok(Measurement {
        ask: query_result.ask.clone(),
        naive_files,
        naive_tokens: estimate_tokens(&naive_content),
        naive_lines,
        pruner_tokens_text: estimate_tokens(&text_output),
        pruner_tokens_json: estimate_tokens(&json_output),
        pruner_files: ctx.key_files.len(),
        pruner_symbols: ctx.key_symbols.len(),
        pruner_snippets: ctx.snippets.len(),
        repo_total_tokens: repo_tokens,
        repo_total_files: repo_files,
    })
}

/// A single step in a simulated Claude Code exploration.
pub struct ExplorationStep {
    pub action: String,
    pub target: String,
    pub tokens: usize,
    pub useful: bool,
}

/// Realistic estimate of Claude Code token usage with and without pruner.
pub struct ClaudeEstimate {
    pub ask: String,
    pub without_steps: Vec<ExplorationStep>,
    pub without_exploration_tokens: usize,
    pub without_relevant_tokens: usize,
    pub without_total_tokens: usize,
    pub with_pruner_context_tokens: usize,
    pub with_targeted_read_tokens: usize,
    pub with_total_tokens: usize,
    pub relevant_files: Vec<String>,
    pub without_files_read: usize,
    pub without_irrelevant_reads: usize,
    pub with_files_read: usize,
}

impl ClaudeEstimate {
    pub fn token_saving(&self) -> i64 {
        self.without_total_tokens as i64 - self.with_total_tokens as i64
    }

    pub fn saving_pct(&self) -> f64 {
        if self.without_total_tokens == 0 {
            return 0.0;
        }
        (1.0 - self.with_total_tokens as f64 / self.without_total_tokens as f64) * 100.0
    }
}

/// Model realistic Claude Code token usage with and without pruner.
pub fn estimate_claude_session(
    query_result: &QueryResult,
    db: &IndexDb,
    repo_path: &Path,
    max_snippet_lines: usize,
) -> Result<ClaudeEstimate> {
    let mut est = ClaudeEstimate {
        ask: query_result.ask.clone(),
        without_steps: Vec::new(),
        without_exploration_tokens: 0,
        without_relevant_tokens: 0,
        without_total_tokens: 0,
        with_pruner_context_tokens: 0,
        with_targeted_read_tokens: 0,
        with_total_tokens: 0,
        relevant_files: Vec::new(),
        without_files_read: 0,
        without_irrelevant_reads: 0,
        with_files_read: 0,
    };

    // Collect relevant file data
    let relevant_file_ids = query_result.all_relevant_file_ids();
    let mut relevant_files = Vec::new();
    let mut file_tokens: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();

    for fid in &relevant_file_ids {
        if let Some(f) = db.get_file_by_path_id(*fid)? {
            let fpath = repo_path.join(&f.path);
            if let Ok(content) = fs::read_to_string(&fpath) {
                file_tokens.insert(f.id, estimate_tokens(&content));
                est.relevant_files.push(f.path.clone());
                relevant_files.push(f);
            }
        }
    }

    // --- WITHOUT pruner ---

    // Step 1: Directory exploration (2-5 glob calls)
    let dir_depth = estimate_dir_depth(repo_path);
    let glob_calls = (2 + query_result.keywords.len()).min(5);
    let glob_tokens_per_call = 50 + dir_depth * 15;

    for i in 0..glob_calls {
        let target = if i > 0 && i < query_result.keywords.len() {
            format!("**/*{}*", query_result.keywords[i])
        } else {
            "top-level structure".to_string()
        };
        est.without_steps.push(ExplorationStep {
            action: "glob".to_string(),
            target,
            tokens: glob_tokens_per_call,
            useful: true,
        });
        est.without_exploration_tokens += glob_tokens_per_call;
    }

    // Step 2: Grep for keywords (2-3 calls)
    let grep_calls = query_result.keywords.len().min(3);
    for i in 0..grep_calls {
        let kw = &query_result.keywords[i];
        let grep_output_tokens = 80 + query_result.matching_symbols.len() * 15;
        est.without_steps.push(ExplorationStep {
            action: "grep".to_string(),
            target: kw.clone(),
            tokens: grep_output_tokens,
            useful: true,
        });
        est.without_exploration_tokens += grep_output_tokens;
    }

    // Step 3: Read files — some relevant, some not
    let all_indexed = db.all_files()?;
    let non_relevant_code_files: Vec<_> = all_indexed
        .iter()
        .filter(|f| {
            !relevant_file_ids.contains(&f.id) && f.language.is_some() && !f.is_test
        })
        .collect();

    // Irrelevant reads
    let irrelevant_ratio = (0.15 + all_indexed.len() as f64 / 2000.0).min(0.4);
    let irrelevant_count = (relevant_files.len() as f64 * irrelevant_ratio).max(1.0) as usize;

    let mut irrelevant_to_read = Vec::new();
    for f in &non_relevant_code_files {
        let path_lower = f.path.to_lowercase();
        if query_result.keywords.iter().any(|kw| path_lower.contains(kw)) {
            irrelevant_to_read.push(f);
            if irrelevant_to_read.len() >= irrelevant_count {
                break;
            }
        }
    }

    for f in &irrelevant_to_read {
        let fpath = repo_path.join(&f.path);
        let tokens = fs::read_to_string(&fpath)
            .map(|c| estimate_tokens(&c))
            .unwrap_or(200);
        est.without_steps.push(ExplorationStep {
            action: "read_irrelevant".to_string(),
            target: f.path.clone(),
            tokens,
            useful: false,
        });
        est.without_exploration_tokens += tokens;
        est.without_irrelevant_reads += 1;
    }

    // Relevant reads
    for f in &relevant_files {
        let tokens = file_tokens.get(&f.id).copied().unwrap_or(0);
        est.without_steps.push(ExplorationStep {
            action: "read".to_string(),
            target: f.path.clone(),
            tokens,
            useful: true,
        });
        est.without_relevant_tokens += tokens;
    }

    est.without_files_read = relevant_files.len() + irrelevant_to_read.len();
    est.without_total_tokens = est.without_exploration_tokens + est.without_relevant_tokens;

    // --- WITH pruner ---

    // Step 1: pruner context output
    let ctx = generate_context(query_result, db, repo_path, max_snippet_lines, false)?;
    let pruner_text = format_context_text(&ctx);
    est.with_pruner_context_tokens = estimate_tokens(&pruner_text);

    // Step 2: Read top key files
    let mut symbol_file_ids: HashSet<i64> = query_result
        .matching_symbols
        .iter()
        .map(|s| s.file_id)
        .collect();

    // Include files from execution paths (first 2 steps)
    for path in &query_result.execution_paths {
        for step in path.iter().take(2) {
            // We don't have file_id on PathStep, so search by file_path
            if let Some(f) = db.get_file_by_path(&step.file_path)? {
                symbol_file_ids.insert(f.id);
            }
        }
    }

    let targeted_ids: Vec<_> = symbol_file_ids.into_iter().take(15).collect();
    for fid in &targeted_ids {
        if let Some(tokens) = file_tokens.get(fid) {
            est.with_targeted_read_tokens += tokens;
        }
    }

    est.with_files_read = targeted_ids.len();
    est.with_total_tokens = est.with_pruner_context_tokens + est.with_targeted_read_tokens;

    Ok(est)
}

/// Estimate directory nesting depth of a repo (capped walk).
fn estimate_dir_depth(repo: &Path) -> usize {
    let mut max_depth = 0;
    let mut count = 0;

    for entry in WalkDir::new(repo)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy();
                return !languages::is_ignored_dir(&name);
            }
            true
        })
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_dir() {
            continue;
        }
        let depth = entry.depth();
        if depth > max_depth {
            max_depth = depth;
        }
        count += 1;
        if count > 200 {
            break;
        }
    }

    max_depth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens_simple() {
        let count = estimate_tokens("hello world");
        assert_eq!(count, 2);
    }

    #[test]
    fn test_estimate_tokens_code() {
        let count = estimate_tokens("fn main() { println!(\"hello\"); }");
        assert!(count > 5);
    }

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_measurement_reduction() {
        let m = Measurement {
            ask: "test".to_string(),
            naive_files: vec!["a.py".to_string()],
            naive_tokens: 1000,
            naive_lines: 100,
            pruner_tokens_text: 300,
            pruner_tokens_json: 400,
            pruner_files: 1,
            pruner_symbols: 5,
            pruner_snippets: 3,
            repo_total_tokens: 10000,
            repo_total_files: 50,
        };
        assert!((m.reduction_vs_naive() - 70.0).abs() < 0.1);
        assert!((m.reduction_vs_repo() - 97.0).abs() < 0.1);
    }

    #[test]
    fn test_estimate_saving_pct() {
        let est = ClaudeEstimate {
            ask: "test".to_string(),
            without_steps: Vec::new(),
            without_exploration_tokens: 200,
            without_relevant_tokens: 800,
            without_total_tokens: 1000,
            with_pruner_context_tokens: 300,
            with_targeted_read_tokens: 400,
            with_total_tokens: 700,
            relevant_files: Vec::new(),
            without_files_read: 5,
            without_irrelevant_reads: 1,
            with_files_read: 3,
        };
        assert_eq!(est.token_saving(), 300);
        assert!((est.saving_pct() - 30.0).abs() < 0.1);
    }
}
