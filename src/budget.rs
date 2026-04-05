//! Query-aware context budget.
//!
//! Compares the current query's keywords and subsystems against the previous
//! query (stored in `.pruner/last-query.json`) to decide how much context to
//! inject.  This avoids re-injecting 10-15K tokens on follow-up prompts about
//! the same topic while still providing full context on task switches.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Metadata saved after each query for comparison with the next.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastQuery {
    pub keywords: Vec<String>,
    pub subsystems: Vec<String>,
    /// SHA-256 hex digest of the formatted output (for identical-output detection).
    pub output_hash: Option<String>,
}

/// Budget decision: how much context to inject.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Budget {
    /// New topic or first query — use normal auto-detection (brief/focused).
    Full,
    /// Same topic as previous — force brief mode.
    Brief,
    /// Output would be identical to previous — emit nothing.
    Skip,
}

/// Check if two keywords are similar enough to count as a match.
/// Matches exactly, or if one is a prefix of the other (≥4 chars).
#[cfg(test)]
fn keywords_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    short.len() >= 4 && long.starts_with(short)
}

/// Compute fuzzy Jaccard similarity between two keyword sets.
/// Uses prefix matching (≥4 chars) so "auth" matches "authentication".
/// Returns 0.0 when both are empty (no overlap signal).
#[cfg(test)]
pub fn jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    // Count matches: for each item in a, check if any item in b matches
    let matches_a = a
        .iter()
        .filter(|x| b.iter().any(|y| keywords_match(x, y)))
        .count();
    let matches_b = b
        .iter()
        .filter(|y| a.iter().any(|x| keywords_match(x, y)))
        .count();
    // Intersection = average of bidirectional match counts
    let intersection = (matches_a + matches_b) as f64 / 2.0;
    // Union = |a| + |b| - intersection
    let union = a.len() as f64 + b.len() as f64 - intersection;
    if union <= 0.0 {
        return 0.0;
    }
    intersection / union
}

/// Thresholds for budget decisions.
/// Set at 0.35 to catch follow-up queries that share 2-3 keywords with the
/// previous query but add noise words like "handle", "case", "also".
#[cfg(test)]
const SAME_TOPIC_THRESHOLD: f64 = 0.35;

/// Decide the context budget by comparing current query against previous.
#[cfg(test)]
pub fn decide_budget(
    current_keywords: &[String],
    current_subsystems: &[String],
    previous: &LastQuery,
    output_hash: Option<&str>,
) -> Budget {
    // Identical output → skip entirely
    if let (Some(current_hash), Some(prev_hash)) = (output_hash, &previous.output_hash)
        && current_hash == prev_hash
    {
        return Budget::Skip;
    }

    // Combine keyword and subsystem similarity (keywords weighted more).
    // When either side has no subsystems, rely on keywords alone — empty
    // subsystems just means pruner couldn't infer one, not a topic change.
    let kw_sim = jaccard(current_keywords, &previous.keywords);
    let combined = if current_subsystems.is_empty() || previous.subsystems.is_empty() {
        kw_sim
    } else {
        let ss_sim = jaccard(current_subsystems, &previous.subsystems);
        kw_sim * 0.7 + ss_sim * 0.3
    };

    if combined >= SAME_TOPIC_THRESHOLD {
        Budget::Brief
    } else {
        Budget::Full
    }
}

/// Load previous query metadata from disk.
pub fn load_last_query(pruner_dir: &Path) -> Result<Option<LastQuery>> {
    let path = pruner_dir.join("last-query.json");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)?;
    let last: LastQuery = serde_json::from_str(&content)?;
    Ok(Some(last))
}

/// Save current query metadata to disk for next comparison.
pub fn save_last_query(pruner_dir: &Path, query: &LastQuery) -> Result<()> {
    let path = pruner_dir.join("last-query.json");
    let content = serde_json::to_string_pretty(query)?;
    fs::write(&path, content)?;
    Ok(())
}

/// Compute SHA-256 hex digest of a string.
pub fn hash_output(output: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    output.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- jaccard ---

    #[test]
    fn test_jaccard_identical() {
        let a = vec!["auth".into(), "token".into(), "login".into()];
        assert_eq!(jaccard(&a, &a), 1.0);
    }

    #[test]
    fn test_jaccard_disjoint() {
        let a = vec!["auth".into(), "token".into()];
        let b = vec!["rate".into(), "limit".into()];
        assert_eq!(jaccard(&a, &b), 0.0);
    }

    #[test]
    fn test_jaccard_partial() {
        let a = vec!["auth".into(), "token".into(), "login".into()];
        let b = vec!["auth".into(), "token".into(), "middleware".into()];
        // intersection=2 (exact: auth, token), union=4 → 0.5
        assert!((jaccard(&a, &b) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_prefix_match() {
        let a = vec!["auth".into(), "token".into()];
        let b = vec!["authentication".into(), "token".into()];
        // "auth" matches "authentication" via prefix, "token" exact → 2/2 = 1.0
        assert!((jaccard(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_short_prefix_no_match() {
        let a = vec!["api".into(), "token".into()];
        let b = vec!["application".into(), "token".into()];
        // "api" is only 3 chars — no prefix match; "token" exact → 1/3 ≈ 0.333
        let sim = jaccard(&a, &b);
        assert!((sim - 1.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_keywords_match_exact() {
        assert!(keywords_match("auth", "auth"));
    }

    #[test]
    fn test_keywords_match_prefix() {
        assert!(keywords_match("auth", "authentication"));
        assert!(keywords_match("authentication", "auth"));
    }

    #[test]
    fn test_keywords_match_short_no_match() {
        // "api" is 3 chars, below minimum 4
        assert!(!keywords_match("api", "application"));
    }

    #[test]
    fn test_jaccard_both_empty() {
        let empty: Vec<String> = vec![];
        assert_eq!(jaccard(&empty, &empty), 0.0);
    }

    #[test]
    fn test_jaccard_one_empty() {
        let a = vec!["auth".into()];
        let empty: Vec<String> = vec![];
        assert_eq!(jaccard(&a, &empty), 0.0);
    }

    // --- decide_budget ---

    #[test]
    fn test_budget_identical_output() {
        let prev = LastQuery {
            keywords: vec!["auth".into()],
            subsystems: vec!["src".into()],
            output_hash: Some("abc123".into()),
        };
        let result = decide_budget(&["auth".into()], &["src".into()], &prev, Some("abc123"));
        assert_eq!(result, Budget::Skip);
    }

    #[test]
    fn test_budget_same_topic() {
        let prev = LastQuery {
            keywords: vec!["auth".into(), "token".into(), "login".into()],
            subsystems: vec!["src/auth".into()],
            output_hash: None,
        };
        // Same keywords, same subsystem → high similarity → Brief
        let result = decide_budget(
            &["auth".into(), "token".into(), "login".into()],
            &["src/auth".into()],
            &prev,
            None,
        );
        assert_eq!(result, Budget::Brief);
    }

    #[test]
    fn test_budget_new_topic() {
        let prev = LastQuery {
            keywords: vec!["auth".into(), "token".into()],
            subsystems: vec!["src/auth".into()],
            output_hash: None,
        };
        // Completely different keywords and subsystem → Full
        let result = decide_budget(
            &["rate".into(), "limit".into(), "throttle".into()],
            &["src/api".into()],
            &prev,
            None,
        );
        assert_eq!(result, Budget::Full);
    }

    #[test]
    fn test_budget_partially_overlapping() {
        let prev = LastQuery {
            keywords: vec!["auth".into(), "token".into(), "validate".into()],
            subsystems: vec!["src/auth".into()],
            output_hash: None,
        };
        // One keyword overlaps, different subsystem
        // kw: intersection=1, union=5 → 0.2; ss: 0.0 → combined=0.14 → Full
        let result = decide_budget(
            &["auth".into(), "middleware".into(), "cors".into()],
            &["src/server".into()],
            &prev,
            None,
        );
        assert_eq!(result, Budget::Full);
    }

    #[test]
    fn test_budget_different_hash_same_keywords() {
        let prev = LastQuery {
            keywords: vec!["auth".into(), "token".into()],
            subsystems: vec!["src/auth".into()],
            output_hash: Some("hash1".into()),
        };
        // Same keywords but different output hash → Brief (not Skip)
        let result = decide_budget(
            &["auth".into(), "token".into()],
            &["src/auth".into()],
            &prev,
            Some("hash2"),
        );
        assert_eq!(result, Budget::Brief);
    }

    // --- persistence ---

    #[test]
    fn test_save_and_load_last_query() {
        let dir = TempDir::new().unwrap();
        let query = LastQuery {
            keywords: vec!["auth".into(), "token".into()],
            subsystems: vec!["src/auth".into()],
            output_hash: Some("abc123".into()),
        };
        save_last_query(dir.path(), &query).unwrap();
        let loaded = load_last_query(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.keywords, query.keywords);
        assert_eq!(loaded.subsystems, query.subsystems);
        assert_eq!(loaded.output_hash, query.output_hash);
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = TempDir::new().unwrap();
        let loaded = load_last_query(dir.path()).unwrap();
        assert!(loaded.is_none());
    }

    // --- hash_output ---

    #[test]
    fn test_hash_output_deterministic() {
        let h1 = hash_output("hello world");
        let h2 = hash_output("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_output_different_inputs() {
        let h1 = hash_output("hello");
        let h2 = hash_output("world");
        assert_ne!(h1, h2);
    }
}
