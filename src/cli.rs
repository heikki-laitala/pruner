//! CLI interface.
//!

use crate::context::{self, detect_mode, format_context_json, format_context_summary, format_context_text, ContextMode};
use crate::db::IndexDb;
use crate::indexer;
use crate::query;
use crate::tokens;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const INDEX_DIR: &str = ".pruner";
const DB_NAME: &str = "index.db";

#[derive(Parser)]
#[command(name = "pruner", version, about = "Synthetic code context engine for LLM coding tasks")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index a repository
    Index {
        /// Path to the repository
        #[arg(default_value = ".")]
        repo: PathBuf,
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    /// Query the index
    Query {
        /// Path to the repository
        repo: PathBuf,
        /// Natural language query
        ask: String,
        /// Output as JSON
        #[arg(long)]
        json_output: bool,
    },
    /// Generate LLM context
    Context {
        /// Path to the repository
        repo: PathBuf,
        /// Natural language task description
        ask: String,
        /// Output format
        #[arg(long, default_value = "text")]
        format: String,
        /// Max lines per snippet
        #[arg(long, default_value = "50")]
        max_snippet_lines: usize,
        /// Brief mode: metadata only, no snippets (~3K tokens)
        #[arg(long)]
        brief: bool,
        /// Full mode: uncapped output (~50-70K tokens on large repos)
        #[arg(long)]
        full: bool,
        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Show file details from index
    ShowFile {
        /// Path to the repository
        repo: PathBuf,
        /// File path within the repo
        path: String,
    },
    /// Show symbol details from index
    ShowSymbol {
        /// Path to the repository
        repo: PathBuf,
        /// Symbol name
        name: String,
    },
    /// Show index statistics
    Stats {
        /// Path to the repository
        #[arg(default_value = ".")]
        repo: PathBuf,
    },
    /// Measure token usage: pruner context vs naive full-file inclusion
    Measure {
        /// Path to the repository
        repo: PathBuf,
        /// Natural language query
        ask: String,
        /// Max lines per snippet
        #[arg(long, default_value = "50")]
        max_snippet_lines: usize,
        /// Output as JSON
        #[arg(long)]
        json_output: bool,
    },
    /// Set up pruner in a project (skill, hook, CLAUDE.md)
    Init {
        /// Path to the project
        #[arg(default_value = ".")]
        repo: PathBuf,
        /// Install prompt-submit hook (Claude Code only, better performance)
        #[arg(long)]
        hook: bool,
        /// Install globally (~/.claude/) instead of project-local
        #[arg(long)]
        global: bool,
    },
    /// Estimate realistic Claude Code token usage with and without pruner
    Estimate {
        /// Path to the repository
        repo: PathBuf,
        /// Natural language query
        ask: String,
        /// Max lines per snippet
        #[arg(long, default_value = "50")]
        max_snippet_lines: usize,
        /// Output as JSON
        #[arg(long)]
        json_output: bool,
        /// Show individual exploration steps
        #[arg(long)]
        show_steps: bool,
    },
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { repo, hook, global } => cmd_init(&repo, hook, global),
        Commands::Index { repo, verbose } => cmd_index(&repo, verbose),
        Commands::Query { repo, ask, json_output } => cmd_query(&repo, &ask, json_output),
        Commands::Context { repo, ask, format, max_snippet_lines, brief, full, output } => {
            let mode = if brief { ContextMode::Brief } else if full { ContextMode::Full } else { ContextMode::Auto };
            cmd_context(&repo, &ask, &format, max_snippet_lines, mode, output.as_deref())
        }
        Commands::ShowFile { repo, path } => cmd_show_file(&repo, &path),
        Commands::ShowSymbol { repo, name } => cmd_show_symbol(&repo, &name),
        Commands::Stats { repo } => cmd_stats(&repo),
        Commands::Measure { repo, ask, max_snippet_lines, json_output } => {
            cmd_measure(&repo, &ask, max_snippet_lines, json_output)
        }
        Commands::Estimate { repo, ask, max_snippet_lines, json_output, show_steps } => {
            cmd_estimate(&repo, &ask, max_snippet_lines, json_output, show_steps)
        }
    }
}

fn db_path(repo: &Path) -> PathBuf {
    repo.join(INDEX_DIR).join(DB_NAME)
}

fn ensure_index_dir(repo: &Path) -> Result<()> {
    let dir = repo.join(INDEX_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(())
}

fn open_db(repo: &Path) -> Result<IndexDb> {
    let path = db_path(repo);
    if !path.exists() {
        anyhow::bail!("No index found at {}. Run `pruner index` first.", path.display());
    }
    IndexDb::open(&path)
}

/// Minimum seconds between incremental index checks.
/// Avoids re-walking the entire repo on every `context` call.
/// Override with PRUNER_RECHECK_SECS=0 to force a check every time.
const DEFAULT_RECHECK_SECS: u64 = 300;

fn open_or_create_db(repo: &Path, verbose: bool) -> Result<IndexDb> {
    let path = db_path(repo);
    if !path.exists() {
        eprintln!("No index found. Indexing {}...", repo.display());
        ensure_index_dir(repo)?;
        let db = IndexDb::open(&path)?;
        let repo_path = repo.canonicalize()?;
        let stats = indexer::index_repo(&repo_path, &db, verbose)?;
        eprintln!(
            "Indexed {} files, {} symbols, {} imports, {} calls, {} edges ({} skipped)",
            stats.files, stats.symbols, stats.imports, stats.calls, stats.edges, stats.skipped
        );
        return Ok(db);
    }

    // Skip incremental walk if the index was checked recently
    if is_index_fresh(&path) {
        return IndexDb::open(&path);
    }

    // Try incremental update
    let db = IndexDb::open(&path)?;
    let repo_path = repo.canonicalize()?;
    if let Some(stats) = indexer::index_repo_incremental(&repo_path, &db, verbose)? {
        eprintln!(
            "Incremental update: {} new/modified, {} unchanged, {} deleted ({} skipped)",
            stats.files, stats.unchanged, stats.deleted, stats.skipped
        );
    }
    Ok(db)
}

/// Check if the index DB was modified recently enough to skip re-checking.
fn is_index_fresh(db_path: &Path) -> bool {
    let recheck_secs = std::env::var("PRUNER_RECHECK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_RECHECK_SECS);

    let Ok(meta) = fs::metadata(db_path) else { return false };
    let Ok(modified) = meta.modified() else { return false };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else { return false };
    elapsed.as_secs() < recheck_secs
}

fn format_index_age(repo: &Path) -> String {
    let path = db_path(repo);
    let Ok(meta) = fs::metadata(&path) else { return String::new() };
    let Ok(modified) = meta.modified() else { return String::new() };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else { return String::new() };

    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

const SKILL_SKILL_MD: &str = include_str!("../.claude/skills/pruner/SKILL.skill.md");
const SKILL_HOOK_MD: &str = include_str!("../.claude/skills/pruner/SKILL.hook.md");
const HOOK_SCRIPT: &str = include_str!("../.claude/hooks/pruner-context.sh");
const CLAUDE_TEMPLATE: &str = include_str!("../CLAUDE.template.md");

fn cmd_init(repo: &Path, hook: bool, global: bool) -> Result<()> {
    let base = if global {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
            .join(".claude")
    } else {
        repo.join(".claude")
    };

    // Install skill
    let skill_dir = base.join("skills").join("pruner");
    fs::create_dir_all(&skill_dir)?;
    let skill_content = if hook { SKILL_HOOK_MD } else { SKILL_SKILL_MD };
    fs::write(skill_dir.join("SKILL.md"), skill_content)?;
    println!("Installed skill -> {}", skill_dir.join("SKILL.md").display());

    // Install hook if requested
    if hook {
        let hook_dir = base.join("hooks");
        fs::create_dir_all(&hook_dir)?;
        let hook_path = hook_dir.join("pruner-context.sh");
        fs::write(&hook_path, HOOK_SCRIPT)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))?;
        }
        println!("Installed hook  -> {}", hook_path.display());

        // Write hook settings
        let settings_path = base.join("settings.json");
        let mut settings: serde_json::Value = if settings_path.exists() {
            serde_json::from_str(&fs::read_to_string(&settings_path)?)?
        } else {
            serde_json::json!({})
        };
        settings["hooks"] = serde_json::json!({
            "UserPromptSubmit": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": hook_path.to_str().unwrap(),
                    "timeout": 60
                }]
            }]
        });
        fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)?;
        println!("Updated settings -> {}", settings_path.display());
    }

    // Append CLAUDE.md instructions (project-local only)
    if !global {
        let claude_md = repo.join("CLAUDE.md");
        let current = if claude_md.exists() {
            fs::read_to_string(&claude_md)?
        } else {
            String::new()
        };
        if !current.contains("pruner context") {
            let mut f = fs::OpenOptions::new().create(true).append(true).open(&claude_md)?;
            use std::io::Write;
            write!(f, "\n{CLAUDE_TEMPLATE}")?;
            println!("Updated CLAUDE.md -> {}", claude_md.display());
        } else {
            println!("CLAUDE.md already has pruner instructions");
        }
    }

    if !global {
        println!("\nNext: pruner index {}", repo.display());
    }

    Ok(())
}

fn cmd_index(repo: &Path, verbose: bool) -> Result<()> {
    ensure_index_dir(repo)?;
    let path = db_path(repo);
    let db = IndexDb::open(&path)?;
    let repo_path = repo.canonicalize()?;

    eprintln!("Indexing {}...", repo_path.display());
    let stats = indexer::index_repo(&repo_path, &db, verbose)?;
    println!(
        "Indexed {} files, {} symbols, {} imports, {} calls, {} edges ({} skipped)",
        stats.files, stats.symbols, stats.imports, stats.calls, stats.edges, stats.skipped
    );
    Ok(())
}

fn cmd_query(repo: &Path, ask: &str, json_output: bool) -> Result<()> {
    let db = open_db(repo)?;
    let result = query::analyze_query(ask, &db)?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "ask": result.ask,
            "keywords": result.keywords,
            "subsystems": result.subsystems,
            "matching_files": result.matching_files.iter().map(|f| &f.path).collect::<Vec<_>>(),
            "matching_symbols": result.matching_symbols.iter().map(|s| &s.name).collect::<Vec<_>>(),
            "related_tests": result.related_tests.iter().map(|t| &t.path).collect::<Vec<_>>(),
            "execution_paths": result.execution_paths.len(),
        }))?);
    } else {
        println!("Keywords: {}", result.keywords.join(", "));
        println!("Subsystems: {}", result.subsystems.join(", "));
        println!("Matching files: {}", result.matching_files.len());
        println!("Matching symbols: {}", result.matching_symbols.len());
        println!("Related tests: {}", result.related_tests.len());
        println!("Execution paths: {}", result.execution_paths.len());
    }
    Ok(())
}

fn cmd_context(
    repo: &Path,
    ask: &str,
    fmt: &str,
    max_snippet_lines: usize,
    mode: ContextMode,
    output: Option<&Path>,
) -> Result<()> {
    let db = open_or_create_db(repo, false)?;
    let repo_path = repo.canonicalize()?;
    let result = query::analyze_query(ask, &db)?;

    // Resolve auto mode and report the decision
    let resolved = if mode == ContextMode::Auto {
        let detected = detect_mode(&result);
        let label = match detected {
            ContextMode::Brief => "brief (narrow task: few files, single subsystem)",
            ContextMode::Focused => "focused (broad task: multiple files/subsystems)",
            _ => unreachable!(),
        };
        eprintln!("Mode: auto → {label}");
        detected
    } else {
        mode
    };

    let ctx = context::generate_context(&result, &repo_path, max_snippet_lines, resolved)?;

    if resolved == ContextMode::Brief {
        // Write *full* context to .pruner/context.md so the LLM can drill deeper
        let full_ctx = context::generate_context(&result, &repo_path, max_snippet_lines, ContextMode::Full)?;
        let ctx_path = repo_path.join(INDEX_DIR).join("context.md");
        let full_text = format_context_text(&full_ctx);
        fs::write(&ctx_path, &full_text)?;

        match fmt {
            "json" => println!("{}", format_context_json(&ctx)?),
            _ => {
                let summary = format_context_summary(&ctx);
                let age = format_index_age(repo);
                if !age.is_empty() {
                    eprintln!("Index age: {age}");
                }
                print!("{summary}");
                eprintln!("Full context: {}", ctx_path.display());
            }
        }
    } else {
        // Focused (default) and Full modes: print full text with snippets
        match fmt {
            "json" => println!("{}", format_context_json(&ctx)?),
            "both" => {
                println!("{}", format_context_text(&ctx));
                if let Some(out) = output {
                    fs::write(out.join("context.json"), format_context_json(&ctx)?)?;
                    fs::write(out.join("context.md"), format_context_text(&ctx))?;
                }
            }
            _ => println!("{}", format_context_text(&ctx)),
        }
    }

    Ok(())
}

fn cmd_show_file(repo: &Path, path: &str) -> Result<()> {
    let db = open_db(repo)?;
    let file = db
        .get_file_by_path(path)?
        .ok_or_else(|| anyhow::anyhow!("File not found in index: {path}"))?;

    println!("Path: {}", file.path);
    println!("Language: {}", file.language.as_deref().unwrap_or("unknown"));
    println!("Lines: {}", file.line_count);
    println!("Size: {} bytes", file.size);
    println!("Test: {}", file.is_test);

    let symbols = db.symbols_for_file(file.id)?;
    if !symbols.is_empty() {
        println!("\nSymbols:");
        for s in &symbols {
            println!(
                "  {} ({}) lines {}-{}",
                s.name, s.kind, s.line_start, s.line_end
            );
        }
    }

    let imports = db.imports_for_file(file.id)?;
    if !imports.is_empty() {
        println!("\nImports:");
        for i in &imports {
            if let Some(names) = &i.names {
                println!("  {} ({})", i.module, names);
            } else {
                println!("  {}", i.module);
            }
        }
    }

    Ok(())
}

fn cmd_show_symbol(repo: &Path, name: &str) -> Result<()> {
    let db = open_db(repo)?;
    let symbols = db.search_symbols(name)?;

    if symbols.is_empty() {
        println!("No symbols matching '{name}'");
        return Ok(());
    }

    for s in &symbols {
        println!("{} ({}) — {}:{}-{}", s.name, s.kind, s.file_path, s.line_start, s.line_end);
        if let Some(sig) = &s.signature {
            println!("  Signature: {sig}");
        }

        let calls = db.calls_by_symbol(s.id)?;
        if !calls.is_empty() {
            println!("  Calls:");
            for c in &calls {
                println!("    {} (line {})", c.callee_name, c.line);
            }
        }

        let callers = db.callers_of(&s.name)?;
        if !callers.is_empty() {
            println!("  Called by:");
            for c in &callers {
                println!("    {} ({})", c.name, c.file_path);
            }
        }
        println!();
    }

    Ok(())
}

fn cmd_stats(repo: &Path) -> Result<()> {
    let db = open_db(repo)?;
    println!("Files:   {}", db.file_count()?);
    println!("Symbols: {}", db.symbol_count()?);
    println!("Imports: {}", db.import_count()?);
    println!("Calls:   {}", db.call_count()?);
    println!("Edges:   {}", db.edge_count()?);
    Ok(())
}

fn cmd_measure(repo: &Path, ask: &str, max_snippet_lines: usize, json_output: bool) -> Result<()> {
    let db = open_db(repo)?;
    let repo_path = repo.canonicalize()?;
    let result = query::analyze_query(ask, &db)?;
    let m = tokens::measure(&result, &db, &repo_path, max_snippet_lines)?;

    if json_output {
        let output = serde_json::json!({
            "ask": m.ask,
            "repo_total": {"files": m.repo_total_files, "tokens": m.repo_total_tokens},
            "naive": {
                "files": m.naive_files.len(),
                "lines": m.naive_lines,
                "tokens": m.naive_tokens,
            },
            "pruner": {
                "files": m.pruner_files,
                "symbols": m.pruner_symbols,
                "snippets": m.pruner_snippets,
                "tokens_text": m.pruner_tokens_text,
                "tokens_json": m.pruner_tokens_json,
            },
            "reduction_vs_naive_pct": (m.reduction_vs_naive() * 10.0).round() / 10.0,
            "reduction_vs_repo_pct": (m.reduction_vs_repo() * 10.0).round() / 10.0,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Token usage measurement for: {}", m.ask);
        println!();
        println!("Whole repo (baseline):");
        println!("  {} files, ~{} tokens", m.repo_total_files, m.repo_total_tokens);
        println!();
        println!("Naive (full content of matching files):");
        println!(
            "  {} files, {} lines, ~{} tokens",
            m.naive_files.len(),
            m.naive_lines,
            m.naive_tokens
        );
        for f in &m.naive_files {
            println!("    {f}");
        }
        println!();
        println!("Pruner (structured context):");
        println!(
            "  {} files, {} symbols, {} snippets",
            m.pruner_files, m.pruner_symbols, m.pruner_snippets
        );
        println!(
            "  ~{} tokens (text) / ~{} tokens (json)",
            m.pruner_tokens_text, m.pruner_tokens_json
        );
        println!();
        println!("Savings:");
        println!(
            "  vs naive:      {:+.1}% tokens ({:+})",
            m.reduction_vs_naive(),
            m.naive_tokens as i64 - m.pruner_tokens_text as i64
        );
        println!(
            "  vs whole repo: {:+.1}% tokens ({:+})",
            m.reduction_vs_repo(),
            m.repo_total_tokens as i64 - m.pruner_tokens_text as i64
        );
    }
    Ok(())
}

fn cmd_estimate(
    repo: &Path,
    ask: &str,
    max_snippet_lines: usize,
    json_output: bool,
    show_steps: bool,
) -> Result<()> {
    let db = open_db(repo)?;
    let repo_path = repo.canonicalize()?;
    let result = query::analyze_query(ask, &db)?;
    let est = tokens::estimate_claude_session(&result, &db, &repo_path, max_snippet_lines)?;

    if json_output {
        let output = serde_json::json!({
            "ask": est.ask,
            "without_pruner": {
                "exploration_tokens": est.without_exploration_tokens,
                "relevant_read_tokens": est.without_relevant_tokens,
                "total_tokens": est.without_total_tokens,
                "files_read": est.without_files_read,
                "irrelevant_reads": est.without_irrelevant_reads,
            },
            "with_pruner": {
                "pruner_context_tokens": est.with_pruner_context_tokens,
                "targeted_read_tokens": est.with_targeted_read_tokens,
                "total_tokens": est.with_total_tokens,
                "files_read": est.with_files_read,
            },
            "saving_tokens": est.token_saving(),
            "saving_pct": (est.saving_pct() * 10.0).round() / 10.0,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Claude Code session estimate for: {}", est.ask);
        println!();

        println!("WITHOUT pruner (explore -> read):");
        println!(
            "  Exploration (glob/grep):   ~{} tokens",
            est.without_exploration_tokens
        );
        let irrelevant_detail = if est.without_irrelevant_reads > 0 {
            format!(" ({} irrelevant)", est.without_irrelevant_reads)
        } else {
            String::new()
        };
        println!(
            "  Reading files:             ~{} tokens ({} files)",
            est.without_relevant_tokens,
            est.relevant_files.len()
        );
        let wasted: usize = est
            .without_steps
            .iter()
            .filter(|s| !s.useful)
            .map(|s| s.tokens)
            .sum();
        println!(
            "  Wasted on wrong files:     ~{} tokens{}",
            wasted, irrelevant_detail
        );
        println!(
            "  Total:                     ~{} tokens ({} files read)",
            est.without_total_tokens, est.without_files_read
        );

        println!();
        println!("WITH pruner (context -> targeted read):");
        println!(
            "  Pruner context output:     ~{} tokens",
            est.with_pruner_context_tokens
        );
        println!(
            "  Targeted file reads:       ~{} tokens ({} files)",
            est.with_targeted_read_tokens, est.with_files_read
        );
        println!("  Total:                     ~{} tokens", est.with_total_tokens);

        println!();
        let saving_sign = if est.saving_pct() >= 0.0 { "+" } else { "" };
        println!(
            "Estimated saving: {}{:.1}% ({:+} tokens)",
            saving_sign,
            est.saving_pct(),
            est.token_saving()
        );

        if show_steps {
            println!();
            println!("Exploration steps (without pruner):");
            for step in &est.without_steps {
                let marker = if step.useful { "  " } else { "* " };
                println!(
                    "  {}{:18} {:50} ~{} tokens",
                    marker, step.action, step.target, step.tokens
                );
            }
            println!("  (* = wasted on irrelevant content)");
        }
    }
    Ok(())
}
