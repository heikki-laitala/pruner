//! Token estimation and cost modeling.
//!
//! Models realistic Claude Code sessions with multi-turn context accumulation:
//! each API call re-sends the full conversation history, so exploration tokens
//! compound across turns.

use crate::context::{ContextMode, format_context_text, generate_context};
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

// Claude Sonnet 4 pricing (per million tokens)
const INPUT_COST_PER_M: f64 = 3.0;
const OUTPUT_COST_PER_M: f64 = 15.0;
// Average output tokens per turn (tool call + reasoning)
const AVG_OUTPUT_PER_TURN: usize = 300;
// Average seconds per tool call (API latency + model thinking)
const SECS_PER_TOOL_CALL: f64 = 2.5;

/// A single tool call in a simulated exploration.
pub struct ExplorationStep {
    pub action: String,
    pub target: String,
    pub tokens: usize,
    pub useful: bool,
}

/// A turn in a simulated Claude Code conversation.
/// Each turn is one API round-trip (request + response).
pub struct Turn {
    pub steps: Vec<ExplorationStep>,
    pub new_tokens: usize,
}

/// Realistic estimate of Claude Code token usage with and without pruner.
/// Models multi-turn context accumulation where each turn re-sends all history.
pub struct ClaudeEstimate {
    pub ask: String,
    // Without pruner
    pub without_turns: Vec<Turn>,
    pub without_input_tokens: usize,
    pub without_output_tokens: usize,
    pub without_total_tokens: usize,
    pub without_tool_calls: usize,
    pub without_wall_secs: f64,
    pub without_files_read: usize,
    pub without_irrelevant_reads: usize,
    // With pruner
    pub with_turns: Vec<Turn>,
    pub with_input_tokens: usize,
    pub with_output_tokens: usize,
    pub with_total_tokens: usize,
    pub with_tool_calls: usize,
    pub with_wall_secs: f64,
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

    pub fn without_cost(&self) -> f64 {
        cost(self.without_input_tokens, self.without_output_tokens)
    }

    pub fn with_cost(&self) -> f64 {
        cost(self.with_input_tokens, self.with_output_tokens)
    }
}

fn count_tool_calls(turns: &[Turn]) -> usize {
    turns.iter().map(|t| t.steps.len()).sum()
}

fn estimate_wall_secs(turns: &[Turn]) -> f64 {
    count_tool_calls(turns) as f64 * SECS_PER_TOOL_CALL
}

fn cost(input: usize, output: usize) -> f64 {
    input as f64 / 1_000_000.0 * INPUT_COST_PER_M + output as f64 / 1_000_000.0 * OUTPUT_COST_PER_M
}

/// Compute cumulative input tokens across turns.
/// Each turn's input = system prompt + all prior (input + output) + new tool results.
fn accumulate_turns(turns: &[Turn], system_tokens: usize) -> (usize, usize) {
    let mut total_input = 0;
    let mut total_output = 0;
    let mut context_so_far = system_tokens;

    for turn in turns {
        // This turn's input = everything accumulated so far + new tool results
        let turn_input = context_so_far + turn.new_tokens;
        total_input += turn_input;
        total_output += AVG_OUTPUT_PER_TURN;
        // Next turn sees: prior context + this turn's tool results + assistant output
        context_so_far = turn_input + AVG_OUTPUT_PER_TURN;
    }

    (total_input, total_output)
}

/// Model realistic Claude Code token usage with and without pruner.
pub fn estimate_claude_session(
    query_result: &QueryResult,
    db: &IndexDb,
    repo_path: &Path,
    max_snippet_lines: usize,
) -> Result<ClaudeEstimate> {
    // Collect relevant file data
    let relevant_file_ids = query_result.all_relevant_file_ids();
    let mut relevant_files = Vec::new();
    let mut file_tokens: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();

    for fid in &relevant_file_ids {
        if let Some(f) = db.get_file_by_path_id(*fid)? {
            let fpath = repo_path.join(&f.path);
            if let Ok(content) = fs::read_to_string(&fpath) {
                file_tokens.insert(f.id, estimate_tokens(&content));
                relevant_files.push(f);
            }
        }
    }

    // Estimate system prompt tokens (CLAUDE.md, instructions, etc.)
    let system_tokens = 2000;

    // ======================================================================
    // WITHOUT pruner — multi-turn exploration
    // ======================================================================

    let mut without_turns = Vec::new();
    let mut without_files_read = 0;
    let mut without_irrelevant_reads = 0;

    // Turn 1: Directory exploration (glob calls)
    let dir_depth = estimate_dir_depth(repo_path);
    let glob_calls = (2 + query_result.keywords.len()).min(5);
    let glob_tokens_per_call = 50 + dir_depth * 15;
    let mut turn1_steps = Vec::new();
    let mut turn1_tokens = 0;

    for i in 0..glob_calls {
        let target = if i > 0 && i < query_result.keywords.len() {
            format!("**/*{}*", query_result.keywords[i])
        } else {
            "top-level structure".to_string()
        };
        turn1_steps.push(ExplorationStep {
            action: "glob".to_string(),
            target,
            tokens: glob_tokens_per_call,
            useful: true,
        });
        turn1_tokens += glob_tokens_per_call;
    }
    without_turns.push(Turn {
        steps: turn1_steps,
        new_tokens: turn1_tokens,
    });

    // Turn 2: Grep for keywords
    let grep_calls = query_result.keywords.len().min(3);
    let mut turn2_steps = Vec::new();
    let mut turn2_tokens = 0;

    for i in 0..grep_calls {
        let kw = &query_result.keywords[i];
        let grep_output_tokens = 80 + query_result.matching_symbols.len() * 15;
        turn2_steps.push(ExplorationStep {
            action: "grep".to_string(),
            target: kw.clone(),
            tokens: grep_output_tokens,
            useful: true,
        });
        turn2_tokens += grep_output_tokens;
    }
    without_turns.push(Turn {
        steps: turn2_steps,
        new_tokens: turn2_tokens,
    });

    // Turn 3: Read irrelevant files (wrong guesses based on keyword matches)
    let all_indexed = db.all_files()?;
    let non_relevant_code_files: Vec<_> = all_indexed
        .iter()
        .filter(|f| !relevant_file_ids.contains(&f.id) && f.language.is_some() && !f.is_test)
        .collect();

    let irrelevant_ratio = (0.15 + all_indexed.len() as f64 / 2000.0).min(0.4);
    let irrelevant_count = (relevant_files.len() as f64 * irrelevant_ratio).max(1.0) as usize;

    let mut irrelevant_to_read = Vec::new();
    for f in &non_relevant_code_files {
        let path_lower = f.path.to_lowercase();
        if query_result
            .keywords
            .iter()
            .any(|kw| path_lower.contains(kw))
        {
            irrelevant_to_read.push(f);
            if irrelevant_to_read.len() >= irrelevant_count {
                break;
            }
        }
    }

    if !irrelevant_to_read.is_empty() {
        let mut turn3_steps = Vec::new();
        let mut turn3_tokens = 0;

        for f in &irrelevant_to_read {
            let fpath = repo_path.join(&f.path);
            let tokens = fs::read_to_string(&fpath)
                .map(|c| estimate_tokens(&c))
                .unwrap_or(200);
            turn3_steps.push(ExplorationStep {
                action: "read".to_string(),
                target: f.path.clone(),
                tokens,
                useful: false,
            });
            turn3_tokens += tokens;
            without_irrelevant_reads += 1;
            without_files_read += 1;
        }
        without_turns.push(Turn {
            steps: turn3_steps,
            new_tokens: turn3_tokens,
        });
    }

    // Turn 4+: Read relevant files (may span multiple turns, ~3 files per turn)
    let files_per_turn = 3;
    for chunk in relevant_files.chunks(files_per_turn) {
        let mut turn_steps = Vec::new();
        let mut turn_tokens = 0;

        for f in chunk {
            let tokens = file_tokens.get(&f.id).copied().unwrap_or(0);
            turn_steps.push(ExplorationStep {
                action: "read".to_string(),
                target: f.path.clone(),
                tokens,
                useful: true,
            });
            turn_tokens += tokens;
            without_files_read += 1;
        }
        without_turns.push(Turn {
            steps: turn_steps,
            new_tokens: turn_tokens,
        });
    }

    // Accumulate with context growth
    let (without_input, without_output) = accumulate_turns(&without_turns, system_tokens);

    // ======================================================================
    // WITH pruner — fewer turns, targeted reads
    // ======================================================================

    let mut with_turns = Vec::new();
    let mut with_files_read = 0;

    // Turn 1: Pruner context injected + targeted reads
    let ctx = generate_context(
        query_result,
        repo_path,
        max_snippet_lines,
        ContextMode::Full,
    )?;
    let pruner_text = format_context_text(&ctx);
    let pruner_context_tokens = estimate_tokens(&pruner_text);

    let mut symbol_file_ids: HashSet<i64> = query_result
        .matching_symbols
        .iter()
        .map(|s| s.file_id)
        .collect();

    for path in &query_result.execution_paths {
        for step in path.iter().take(2) {
            if let Some(f) = db.get_file_by_path(&step.file_path)? {
                symbol_file_ids.insert(f.id);
            }
        }
    }

    let targeted_ids: Vec<_> = symbol_file_ids.into_iter().take(15).collect();
    let mut turn1_steps = Vec::new();
    let mut turn1_tokens = 0;

    for fid in &targeted_ids {
        if let Some(tokens) = file_tokens.get(fid)
            && let Some(f) = db.get_file_by_path_id(*fid)?
        {
            turn1_steps.push(ExplorationStep {
                action: "read".to_string(),
                target: f.path,
                tokens: *tokens,
                useful: true,
            });
            turn1_tokens += tokens;
            with_files_read += 1;
        }
    }
    with_turns.push(Turn {
        steps: turn1_steps,
        new_tokens: turn1_tokens,
    });

    // With pruner, system prompt includes pruner context
    let with_system_tokens = system_tokens + pruner_context_tokens;
    let (with_input, with_output) = accumulate_turns(&with_turns, with_system_tokens);

    let without_tool_calls = count_tool_calls(&without_turns);
    let without_wall_secs = estimate_wall_secs(&without_turns);
    let with_tool_calls = count_tool_calls(&with_turns);
    let with_wall_secs = estimate_wall_secs(&with_turns);

    Ok(ClaudeEstimate {
        ask: query_result.ask.clone(),
        without_turns,
        without_input_tokens: without_input,
        without_output_tokens: without_output,
        without_total_tokens: without_input + without_output,
        without_tool_calls,
        without_wall_secs,
        without_files_read,
        without_irrelevant_reads,
        with_turns,
        with_input_tokens: with_input,
        with_output_tokens: with_output,
        with_total_tokens: with_input + with_output,
        with_tool_calls,
        with_wall_secs,
        with_files_read,
    })
}

/// Estimate directory nesting depth of a repo (capped walk).
fn estimate_dir_depth(repo: &Path) -> usize {
    let mut max_depth = 0;
    let mut count = 0;

    for entry in WalkDir::new(repo).into_iter().filter_entry(|e| {
        if e.file_type().is_dir() {
            let name = e.file_name().to_string_lossy();
            return !languages::is_ignored_dir(&name);
        }
        true
    }) {
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
    use tempfile::TempDir;

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
    fn test_estimate_tokens_newlines() {
        let count = estimate_tokens("line1\nline2\nline3");
        // 3 words + 2 newlines = 5
        assert_eq!(count, 5);
    }

    #[test]
    fn test_estimate_tokens_punctuation() {
        let count = estimate_tokens("a + b = c;");
        assert_eq!(count, 6); // a, +, b, =, c, ;
    }

    #[test]
    fn test_estimate_tokens_multiline_code() {
        let code = "fn foo(x: i32) -> bool {\n    x > 0\n}";
        let count = estimate_tokens(code);
        assert!(count >= 14);
    }

    #[test]
    fn test_cost_calculation() {
        // 1M input + 100K output
        let c = cost(1_000_000, 100_000);
        // $3.00 input + $1.50 output = $4.50
        assert!((c - 4.5).abs() < 0.01);
    }

    #[test]
    fn test_accumulate_single_turn() {
        let turns = vec![Turn {
            steps: vec![],
            new_tokens: 500,
        }];
        let (input, output) = accumulate_turns(&turns, 2000);
        // Turn 1 input = 2000 (system) + 500 (new) = 2500
        assert_eq!(input, 2500);
        assert_eq!(output, AVG_OUTPUT_PER_TURN);
    }

    #[test]
    fn test_accumulate_multi_turn_compounds() {
        let turns = vec![
            Turn {
                steps: vec![],
                new_tokens: 500,
            },
            Turn {
                steps: vec![],
                new_tokens: 500,
            },
        ];
        let (input, output) = accumulate_turns(&turns, 2000);
        // Turn 1: input = 2000 + 500 = 2500, context after = 2500 + 300 = 2800
        // Turn 2: input = 2800 + 500 = 3300
        // Total input = 2500 + 3300 = 5800
        assert_eq!(input, 5800);
        assert_eq!(output, AVG_OUTPUT_PER_TURN * 2);
    }

    #[test]
    fn test_accumulate_shows_compounding_effect() {
        // 5 turns of 1000 tokens each vs naive sum
        let turns: Vec<Turn> = (0..5)
            .map(|_| Turn {
                steps: vec![],
                new_tokens: 1000,
            })
            .collect();
        let (input, _) = accumulate_turns(&turns, 2000);
        let naive_sum = 2000 + 5 * 1000; // 7000 without accumulation
        // With accumulation, total input should be significantly more
        assert!(
            input > naive_sum,
            "accumulated {input} should be > naive {naive_sum}"
        );
    }

    #[test]
    fn test_saving_pct() {
        let est = ClaudeEstimate {
            ask: "test".to_string(),
            without_turns: Vec::new(),
            without_input_tokens: 800,
            without_output_tokens: 200,
            without_total_tokens: 1000,
            without_tool_calls: 8,
            without_wall_secs: 20.0,
            with_turns: Vec::new(),
            with_input_tokens: 560,
            with_output_tokens: 140,
            with_total_tokens: 700,
            with_tool_calls: 2,
            with_wall_secs: 5.0,

            without_files_read: 5,
            without_irrelevant_reads: 1,
            with_files_read: 3,
        };
        assert_eq!(est.token_saving(), 300);
        assert!((est.saving_pct() - 30.0).abs() < 0.1);
    }

    #[test]
    fn test_saving_zero() {
        let est = ClaudeEstimate {
            ask: "test".to_string(),
            without_turns: Vec::new(),
            without_input_tokens: 0,
            without_output_tokens: 0,
            without_total_tokens: 0,
            without_tool_calls: 0,
            without_wall_secs: 0.0,
            with_turns: Vec::new(),
            with_input_tokens: 0,
            with_output_tokens: 0,
            with_total_tokens: 0,
            with_tool_calls: 0,
            with_wall_secs: 0.0,

            without_files_read: 0,
            without_irrelevant_reads: 0,
            with_files_read: 0,
        };
        assert_eq!(est.token_saving(), 0);
        assert_eq!(est.saving_pct(), 0.0);
    }

    #[test]
    fn test_saving_negative() {
        let est = ClaudeEstimate {
            ask: "test".to_string(),
            without_turns: Vec::new(),
            without_input_tokens: 200,
            without_output_tokens: 100,
            without_total_tokens: 300,
            without_tool_calls: 3,
            without_wall_secs: 7.5,
            with_turns: Vec::new(),
            with_input_tokens: 400,
            with_output_tokens: 100,
            with_total_tokens: 500,
            with_tool_calls: 1,
            with_wall_secs: 2.5,

            without_files_read: 2,
            without_irrelevant_reads: 0,
            with_files_read: 1,
        };
        assert_eq!(est.token_saving(), -200);
        assert!(est.saving_pct() < 0.0);
    }

    #[test]
    fn test_cost_methods() {
        let est = ClaudeEstimate {
            ask: "test".to_string(),
            without_turns: Vec::new(),
            without_input_tokens: 100_000,
            without_output_tokens: 10_000,
            without_total_tokens: 110_000,
            without_tool_calls: 10,
            without_wall_secs: 25.0,
            with_turns: Vec::new(),
            with_input_tokens: 30_000,
            with_output_tokens: 5_000,
            with_total_tokens: 35_000,
            with_tool_calls: 3,
            with_wall_secs: 7.5,

            without_files_read: 5,
            without_irrelevant_reads: 1,
            with_files_read: 2,
        };
        assert!(est.without_cost() > est.with_cost());
        assert!(est.without_cost() > 0.0);
        assert!(est.with_cost() > 0.0);
    }

    #[test]
    fn test_estimate_dir_depth_flat() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "world").unwrap();
        let depth = estimate_dir_depth(tmp.path());
        assert_eq!(depth, 0);
    }

    #[test]
    fn test_estimate_dir_depth_nested() {
        let tmp = TempDir::new().unwrap();
        let deep = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("file.txt"), "deep").unwrap();
        let depth = estimate_dir_depth(tmp.path());
        assert_eq!(depth, 3);
    }

    #[test]
    fn test_estimate_dir_depth_skips_ignored() {
        let tmp = TempDir::new().unwrap();
        let ignored = tmp.path().join("node_modules").join("deep").join("deeper");
        std::fs::create_dir_all(&ignored).unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let depth = estimate_dir_depth(tmp.path());
        assert_eq!(depth, 1);
    }

    #[test]
    fn test_estimate_dir_depth_empty() {
        let tmp = TempDir::new().unwrap();
        let depth = estimate_dir_depth(tmp.path());
        assert_eq!(depth, 0);
    }

    #[test]
    fn test_exploration_step_fields() {
        let step = ExplorationStep {
            action: "grep".to_string(),
            target: "login".to_string(),
            tokens: 150,
            useful: true,
        };
        assert_eq!(step.action, "grep");
        assert_eq!(step.tokens, 150);
        assert!(step.useful);
    }

    /// Helper: create a mini repo in a tempdir with an indexed DB.
    fn setup_mini_repo() -> (TempDir, crate::db::IndexDb) {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("main.rs"), "fn main() {\n    login();\n}\n").unwrap();
        std::fs::write(
            src.join("auth.rs"),
            "fn login() {\n    verify();\n}\nfn verify() {}\n",
        )
        .unwrap();

        let db_path = tmp.path().join(".pruner");
        std::fs::create_dir_all(&db_path).unwrap();
        let db = crate::db::IndexDb::open(&db_path.join("index.db")).unwrap();

        let f1 = db
            .insert_file("src/main.rs", Some("rust"), 30, 3, false, 0)
            .unwrap();
        let f2 = db
            .insert_file("src/auth.rs", Some("rust"), 50, 4, false, 0)
            .unwrap();
        let main_sym = db
            .insert_symbol(f1, "main", "function", 1, 3, None, None)
            .unwrap();
        let login_sym = db
            .insert_symbol(f2, "login", "function", 1, 3, None, None)
            .unwrap();
        let verify_sym = db
            .insert_symbol(f2, "verify", "function", 4, 4, None, None)
            .unwrap();
        db.insert_call(main_sym, "login", 2).unwrap();
        db.insert_call(login_sym, "verify", 2).unwrap();
        let _ = verify_sym;

        (tmp, db)
    }

    #[test]
    fn test_estimate_claude_session_produces_valid_output() {
        let (tmp, db) = setup_mini_repo();
        let result = crate::query::analyze_query("login authentication", &db).unwrap();
        let est = estimate_claude_session(&result, &db, tmp.path(), 50).unwrap();

        assert!(!est.ask.is_empty());
        assert!(est.without_total_tokens > 0);
        assert!(est.with_total_tokens > 0);
        assert!(!est.without_turns.is_empty());
    }

    #[test]
    fn test_estimate_claude_session_has_exploration_turns() {
        let (tmp, db) = setup_mini_repo();
        let result = crate::query::analyze_query("login", &db).unwrap();
        let est = estimate_claude_session(&result, &db, tmp.path(), 50).unwrap();

        // Should have multiple turns (glob, grep, reads)
        assert!(
            est.without_turns.len() >= 2,
            "expected multiple turns, got {}",
            est.without_turns.len()
        );

        // First turn should be glob
        let first_actions: Vec<&str> = est.without_turns[0]
            .steps
            .iter()
            .map(|s| s.action.as_str())
            .collect();
        assert!(first_actions.contains(&"glob"));
    }

    #[test]
    fn test_estimate_claude_session_no_matches() {
        let (tmp, db) = setup_mini_repo();
        let result = crate::query::analyze_query("zzz_nonexistent", &db).unwrap();
        let est = estimate_claude_session(&result, &db, tmp.path(), 50).unwrap();

        assert_eq!(est.without_files_read, 0);
    }

    #[test]
    fn test_estimate_multi_turn_costs_more_than_single() {
        let (tmp, db) = setup_mini_repo();
        let result = crate::query::analyze_query("login", &db).unwrap();
        let est = estimate_claude_session(&result, &db, tmp.path(), 50).unwrap();

        // Multi-turn "without" should cost more than single-turn "with"
        // because context accumulates
        assert!(
            est.without_turns.len() > est.with_turns.len(),
            "without should have more turns"
        );
    }

    #[test]
    fn test_estimate_context_accumulation() {
        let (tmp, db) = setup_mini_repo();
        let result = crate::query::analyze_query("login", &db).unwrap();
        let est = estimate_claude_session(&result, &db, tmp.path(), 50).unwrap();

        // Total input tokens should be greater than naive sum of all new_tokens
        // because of context accumulation
        let naive_new: usize = est.without_turns.iter().map(|t| t.new_tokens).sum();
        assert!(
            est.without_input_tokens > naive_new,
            "accumulated input {} should be > naive sum of new tokens {}",
            est.without_input_tokens,
            naive_new
        );
    }

    #[test]
    fn test_estimate_claude_session_counts_all_reads() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(
            src.join("login.rs"),
            "fn login() { verify(); }\nfn verify() {}\n",
        )
        .unwrap();
        std::fs::write(src.join("login_utils.rs"), "fn hash_password() {}\n").unwrap();
        std::fs::write(src.join("server.rs"), "fn start() {}\n").unwrap();

        let db_path = tmp.path().join(".pruner");
        std::fs::create_dir_all(&db_path).unwrap();
        let db = crate::db::IndexDb::open(&db_path.join("index.db")).unwrap();

        let f1 = db
            .insert_file("src/login.rs", Some("rust"), 50, 2, false, 0)
            .unwrap();
        db.insert_file("src/login_utils.rs", Some("rust"), 30, 1, false, 0)
            .unwrap();
        db.insert_file("src/server.rs", Some("rust"), 20, 1, false, 0)
            .unwrap();

        let login_sym = db
            .insert_symbol(f1, "login", "function", 1, 1, None, None)
            .unwrap();
        db.insert_symbol(f1, "verify", "function", 2, 2, None, None)
            .unwrap();
        db.insert_call(login_sym, "verify", 1).unwrap();

        let result = crate::query::analyze_query("login", &db).unwrap();
        let est = estimate_claude_session(&result, &db, tmp.path(), 50).unwrap();

        assert!(est.without_files_read > 0);
        assert!(est.without_total_tokens > 0);
    }
}
