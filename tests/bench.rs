//! A/B benchmark test against a real repository.
//!
//! Clones openclaw/openclaw (or uses PRUNER_TEST_REPO), indexes it,
//! runs predefined queries, and captures metrics. Compares against
//! a stored baseline to detect regressions.
//!
//! Run: make bench

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const DEFAULT_REPO_URL: &str = "https://github.com/openclaw/openclaw.git";
const CACHE_DIR: &str = "/tmp/pruner-bench";
const QUERY_TIMEOUT: Duration = Duration::from_secs(120);

/// Queries representing different task types for A/B measurement.
const BENCHMARK_QUERIES: &[(&str, &str)] = &[
    ("cross_package", "how does a message flow from webhook to channel handler"),
    ("narrow_fix", "fix WebSocket reconnection timeout"),
    ("understanding", "how does the skill execution pipeline work"),
    ("cross_cutting", "add correlation ID across middleware and handlers"),
    ("data_flow", "how does authentication token validation work"),
];

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BenchResult {
    repo_url: String,
    index_stats: IndexStats,
    queries: Vec<QueryMetrics>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct IndexStats {
    files: i64,
    symbols: i64,
    imports: i64,
    calls: i64,
    edges: i64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct QueryMetrics {
    category: String,
    query: String,
    key_files: usize,
    key_symbols: usize,
    execution_paths: usize,
    snippets: usize,
    relevant_tests: usize,
    subsystems: Vec<String>,
    pruner_context_tokens: usize,
    duration_secs: f64,
}

fn pruner_bin() -> PathBuf {
    // Prefer release binary for performance
    let release = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/release/pruner");
    if release.exists() {
        return release;
    }
    // Fall back to debug
    let debug = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/debug/pruner");
    assert!(debug.exists(), "pruner binary not found — run cargo build first");
    debug
}

fn get_repo_path() -> PathBuf {
    if let Ok(p) = std::env::var("PRUNER_TEST_REPO") {
        return PathBuf::from(p);
    }

    let repo_dir = Path::new(CACHE_DIR).join("openclaw");
    if !repo_dir.join(".git").exists() {
        eprintln!("Cloning {DEFAULT_REPO_URL} to {CACHE_DIR}/openclaw ...");
        std::fs::create_dir_all(CACHE_DIR).unwrap();
        let status = Command::new("git")
            .args(["clone", "--depth", "1", DEFAULT_REPO_URL, repo_dir.to_str().unwrap()])
            .status()
            .expect("git clone failed");
        assert!(status.success(), "git clone failed with {status}");
    } else {
        eprintln!("Using cached repo at {}", repo_dir.display());
    }

    repo_dir
}

fn parse_stats(stdout: &str) -> IndexStats {
    let get = |prefix: &str| -> i64 {
        stdout
            .lines()
            .find(|l| l.starts_with(prefix))
            .and_then(|l| l.split_whitespace().last())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    IndexStats {
        files: get("Files:"),
        symbols: get("Symbols:"),
        imports: get("Imports:"),
        calls: get("Calls:"),
        edges: get("Edges:"),
    }
}

fn estimate_tokens(text: &str) -> usize {
    let re = regex::Regex::new(r"\w+|[^\w\s]|\n").unwrap();
    re.find_iter(text).count()
}

#[test]
fn bench_real_repo() {
    let repo_path = get_repo_path();
    let repo_str = repo_path.to_str().unwrap();
    let bin = pruner_bin();
    let repo_url = std::env::var("PRUNER_TEST_REPO")
        .unwrap_or_else(|_| DEFAULT_REPO_URL.to_string());

    eprintln!("Using binary: {}", bin.display());

    // Index
    eprintln!("\n=== Indexing {} ===", repo_path.display());
    let start = Instant::now();
    let output = Command::new(&bin)
        .args(["index", repo_str, "-v"])
        .output()
        .expect("pruner index failed");
    assert!(output.status.success(), "pruner index failed: {}", String::from_utf8_lossy(&output.stderr));
    eprintln!("Index time: {:.1}s", start.elapsed().as_secs_f64());
    eprintln!("{}", String::from_utf8_lossy(&output.stdout).trim());

    // Get stats
    let output = Command::new(&bin)
        .args(["stats", repo_str])
        .output()
        .unwrap();
    let stats_stdout = String::from_utf8_lossy(&output.stdout);
    let index_stats = parse_stats(&stats_stdout);

    eprintln!("Files: {}, Symbols: {}, Calls: {}, Edges: {}",
        index_stats.files, index_stats.symbols, index_stats.calls, index_stats.edges);

    assert!(index_stats.files > 50, "should index >50 files, got {}", index_stats.files);
    assert!(index_stats.symbols > 100, "should find >100 symbols, got {}", index_stats.symbols);

    // Run benchmark queries
    let mut queries = Vec::new();

    for (category, query) in BENCHMARK_QUERIES {
        eprintln!("\n--- Query [{category}]: \"{query}\" ---");

        let start = Instant::now();
        let child = Command::new(&bin)
            .args(["context", repo_str, query, "--format", "json"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("failed to spawn pruner");

        // Wait with timeout
        let output = match wait_with_timeout(child, QUERY_TIMEOUT) {
            Some(o) => o,
            None => {
                eprintln!("  TIMEOUT after {}s — skipping", QUERY_TIMEOUT.as_secs());
                queries.push(QueryMetrics {
                    category: category.to_string(),
                    query: query.to_string(),
                    key_files: 0,
                    key_symbols: 0,
                    execution_paths: 0,
                    snippets: 0,
                    relevant_tests: 0,
                    subsystems: Vec::new(),
                    pruner_context_tokens: 0,
                    duration_secs: QUERY_TIMEOUT.as_secs_f64(),
                });
                continue;
            }
        };
        let duration = start.elapsed();

        let stdout = String::from_utf8_lossy(&output.stdout);

        let json: serde_json::Value = match serde_json::from_str(&stdout) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("  WARN: failed to parse JSON ({e}), stderr: {}",
                    String::from_utf8_lossy(&output.stderr).lines().take(3).collect::<Vec<_>>().join(" | "));
                queries.push(QueryMetrics {
                    category: category.to_string(),
                    query: query.to_string(),
                    key_files: 0,
                    key_symbols: 0,
                    execution_paths: 0,
                    snippets: 0,
                    relevant_tests: 0,
                    subsystems: Vec::new(),
                    pruner_context_tokens: 0,
                    duration_secs: duration.as_secs_f64(),
                });
                continue;
            }
        };

        let key_files = json["key_files"].as_array().map_or(0, |a| a.len());
        let key_symbols = json["key_symbols"].as_array().map_or(0, |a| a.len());
        let execution_paths = json["execution_paths"].as_array().map_or(0, |a| a.len());
        let snippets = json["snippets"].as_array().map_or(0, |a| a.len());
        let relevant_tests = json["relevant_tests"].as_array().map_or(0, |a| a.len());
        let subsystems: Vec<String> = json["subsystems"]
            .as_array()
            .map_or(Vec::new(), |a| {
                a.iter().filter_map(|v| v.as_str().map(String::from)).collect()
            });

        let context_text = serde_json::to_string_pretty(&json).unwrap();
        let pruner_context_tokens = estimate_tokens(&context_text);

        eprintln!("  files={key_files} symbols={key_symbols} paths={execution_paths} snippets={snippets} tests={relevant_tests} tokens={pruner_context_tokens} time={:.1}s", duration.as_secs_f64());
        eprintln!("  subsystems: {}", if subsystems.is_empty() { "(none)".to_string() } else { subsystems.join(", ") });

        queries.push(QueryMetrics {
            category: category.to_string(),
            query: query.to_string(),
            key_files,
            key_symbols,
            execution_paths,
            snippets,
            relevant_tests,
            subsystems,
            pruner_context_tokens,
            duration_secs: duration.as_secs_f64(),
        });
    }

    let result = BenchResult {
        repo_url,
        index_stats,
        queries,
    };

    // Save results
    let results_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let results_path = results_dir.join("bench_results.json");
    let baseline_path = results_dir.join("bench_baseline.json");

    let result_json = serde_json::to_string_pretty(&result).unwrap();
    std::fs::write(&results_path, &result_json).unwrap();
    eprintln!("\n=== Results saved to {} ===", results_path.display());

    // Compare against baseline if it exists
    if baseline_path.exists() {
        eprintln!("\n=== Comparing against baseline ===\n");
        let baseline_str = std::fs::read_to_string(&baseline_path).unwrap();
        let baseline: BenchResult = serde_json::from_str(&baseline_str).unwrap();
        compare_results(&baseline, &result);
    } else {
        eprintln!("\nNo baseline found. To set current results as baseline:");
        eprintln!("  cp tests/bench_results.json tests/bench_baseline.json");
    }
}

fn wait_with_timeout(mut child: std::process::Child, timeout: Duration) -> Option<std::process::Output> {
    // Read stdout/stderr in background threads to avoid pipe deadlock
    // (large JSON output can exceed the OS pipe buffer, blocking the child).
    let stdout_handle = child.stdout.take().map(|s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut std::io::BufReader::new(s), &mut buf).unwrap_or(0);
            buf
        })
    });
    let stderr_handle = child.stderr.take().map(|s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut std::io::BufReader::new(s), &mut buf).unwrap_or(0);
            buf
        })
    });

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_handle.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
                let stderr = stderr_handle.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
                return Some(std::process::Output { status, stdout, stderr });
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return None,
        }
    }
}

fn compare_results(baseline: &BenchResult, current: &BenchResult) {
    let bi = &baseline.index_stats;
    let ci = &current.index_stats;

    print_delta("Files indexed", bi.files, ci.files);
    print_delta("Symbols found", bi.symbols, ci.symbols);
    print_delta("Calls tracked", bi.calls, ci.calls);
    print_delta("Edges built", bi.edges, ci.edges);

    let baseline_map: HashMap<&str, &QueryMetrics> = baseline
        .queries
        .iter()
        .map(|q| (q.category.as_str(), q))
        .collect();

    let mut regressions = Vec::new();

    for cq in &current.queries {
        eprintln!("\n[{}] \"{}\"", cq.category, cq.query);

        if let Some(bq) = baseline_map.get(cq.category.as_str()) {
            print_delta("  key_files", bq.key_files as i64, cq.key_files as i64);
            print_delta("  key_symbols", bq.key_symbols as i64, cq.key_symbols as i64);
            print_delta("  execution_paths", bq.execution_paths as i64, cq.execution_paths as i64);
            print_delta("  snippets", bq.snippets as i64, cq.snippets as i64);
            print_delta("  relevant_tests", bq.relevant_tests as i64, cq.relevant_tests as i64);
            print_delta("  context_tokens", bq.pruner_context_tokens as i64, cq.pruner_context_tokens as i64);
            print_delta_f("  duration", bq.duration_secs, cq.duration_secs);

            if cq.key_symbols < bq.key_symbols.saturating_sub(2) {
                regressions.push(format!(
                    "[{}] key_symbols dropped: {} -> {}",
                    cq.category, bq.key_symbols, cq.key_symbols
                ));
            }
            if cq.execution_paths == 0 && bq.execution_paths > 0 {
                regressions.push(format!(
                    "[{}] execution_paths dropped to 0 (was {})",
                    cq.category, bq.execution_paths
                ));
            }
        } else {
            eprintln!("  (no baseline for this category)");
        }
    }

    if !regressions.is_empty() {
        eprintln!("\n=== REGRESSIONS DETECTED ===");
        for r in &regressions {
            eprintln!("  REGRESSION: {r}");
        }
        panic!(
            "Benchmark regressions detected:\n{}",
            regressions.join("\n")
        );
    } else {
        eprintln!("\n=== No regressions detected ===");
    }
}

fn print_delta(label: &str, baseline: i64, current: i64) {
    let delta = current - baseline;
    let pct = if baseline > 0 {
        (delta as f64 / baseline as f64) * 100.0
    } else {
        0.0
    };
    let arrow = if delta > 0 { "+" } else if delta < 0 { "" } else { " " };
    eprintln!("{label}: {baseline} -> {current} ({arrow}{delta}, {arrow}{pct:.1}%)");
}

fn print_delta_f(label: &str, baseline: f64, current: f64) {
    let delta = current - baseline;
    let pct = if baseline > 0.0 {
        (delta / baseline) * 100.0
    } else {
        0.0
    };
    let arrow = if delta > 0.0 { "+" } else if delta < 0.0 { "" } else { " " };
    eprintln!("{label}: {baseline:.1}s -> {current:.1}s ({arrow}{delta:.1}s, {arrow}{pct:.1}%)");
}
