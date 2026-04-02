//! CLI interface.
//!

use crate::budget;
use crate::context::{
    self, ContextMode, detect_mode, format_context_json, format_context_summary,
    format_context_text,
};
use crate::db::IndexDb;
use crate::indexer;
use crate::query;
use crate::tokens;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};

/// Replace or append a `## Pruner` section in an instructions file.
/// If the file already has a `## Pruner` section, replace it (up to the next `## ` or EOF).
/// Otherwise, append the template.
fn upsert_pruner_section(path: &Path, template: &str) -> Result<()> {
    use std::io::Write;

    let current = if path.exists() {
        fs::read_to_string(path)?
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        String::new()
    };

    const MARKER: &str = "## Pruner";

    let new_content = if let Some(start) = current.find(MARKER) {
        // Find end of the pruner section: next ## heading or EOF
        let after_marker = start + MARKER.len();
        let end = current[after_marker..]
            .find("\n## ")
            .map(|i| after_marker + i + 1) // +1 to keep the newline before next heading
            .unwrap_or(current.len());
        let mut result = current[..start].to_string();
        result.push_str(template);
        if end < current.len() {
            let remainder = &current[end..];
            if !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(remainder);
        }
        result
    } else {
        // Append
        let mut result = current;
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(template);
        result
    };

    let mut f = fs::File::create(path)?;
    f.write_all(new_content.as_bytes())?;
    Ok(())
}
use std::process::Command;
use std::time::SystemTime;

const INDEX_DIR: &str = ".pruner";
const DB_NAME: &str = "index.db";
const META_GIT_HEAD: &str = "git_head";

#[derive(Parser)]
#[command(
    name = "pruner",
    version,
    about = "Synthetic code context engine for LLM coding tasks"
)]
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
    /// Set up pruner in a project (Claude/Copilot skills, hook, instructions)
    Init {
        /// Path to the project
        #[arg(default_value = ".")]
        repo: PathBuf,
        /// Install prompt-submit hook (Claude Code only, better performance)
        #[arg(long)]
        hook: bool,
        /// Install Claude skill globally (~/.claude/) instead of project-local
        #[arg(long)]
        global: bool,
        /// Install Copilot CLI skill and instructions
        #[arg(long)]
        copilot_skill: bool,
        /// Install Copilot CLI userPromptSubmitted hook (repo-local, writes .pruner/copilot-context.md)
        #[arg(long)]
        copilot_hook: bool,
        /// Install Copilot CLI skill globally (~/.copilot/)
        #[arg(long)]
        copilot_global: bool,
    },
    /// Remove pruner integrations (hooks, skills, config) and optionally the binary.
    /// Global uninstall scans ~/ for leftover project-level traces.
    Uninstall {
        /// Path to a project to remove per-project integrations (omit for global uninstall)
        repo: Option<PathBuf>,
        /// Remove all found traces without prompting (global) or remove .pruner/ index (per-project)
        #[arg(long)]
        purge: bool,
    },
    /// Upgrade pruner to the latest (or a specific) version
    Upgrade {
        /// Only check if an update is available, don't install
        #[arg(long)]
        check: bool,
        /// Install a specific version (e.g., v0.1.6)
        #[arg(long)]
        version: Option<String>,
    },
    /// Show current pruner installation status (global and per-project integrations)
    Status {
        /// Path to the project (omit to show only global status)
        repo: Option<PathBuf>,
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
        Commands::Init {
            repo,
            hook,
            global,
            copilot_skill,
            copilot_hook,
            copilot_global,
        } => cmd_init(
            &repo,
            hook,
            global,
            copilot_skill,
            copilot_hook,
            copilot_global,
        ),
        Commands::Index { repo, verbose } => cmd_index(&repo, verbose),
        Commands::Query {
            repo,
            ask,
            json_output,
        } => cmd_query(&repo, &ask, json_output),
        Commands::Context {
            repo,
            ask,
            format,
            max_snippet_lines,
            brief,
            full,
            output,
        } => {
            let mode = if brief {
                ContextMode::Brief
            } else if full {
                ContextMode::Full
            } else {
                ContextMode::Auto
            };
            cmd_context(
                &repo,
                &ask,
                &format,
                max_snippet_lines,
                mode,
                output.as_deref(),
            )
        }
        Commands::ShowFile { repo, path } => cmd_show_file(&repo, &path),
        Commands::ShowSymbol { repo, name } => cmd_show_symbol(&repo, &name),
        Commands::Stats { repo } => cmd_stats(&repo),
        Commands::Status { repo } => cmd_status(repo.as_deref()),
        Commands::Uninstall { repo, purge } => {
            crate::uninstall::cmd_uninstall(repo.as_deref(), purge)
        }
        Commands::Upgrade { check, version } => {
            crate::upgrade::cmd_upgrade(check, version.as_deref())
        }
        Commands::Estimate {
            repo,
            ask,
            max_snippet_lines,
            json_output,
            show_steps,
        } => cmd_estimate(&repo, &ask, max_snippet_lines, json_output, show_steps),
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
        anyhow::bail!(
            "No index found at {}. Run `pruner index` first.",
            path.display()
        );
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
        if stats.parsed == 0 {
            // No parseable source code — remove the empty index to avoid clutter
            drop(db);
            let _ = fs::remove_dir_all(repo.join(INDEX_DIR));
            anyhow::bail!("No supported source files found in {}", repo.display());
        }
        if let Some(head) = git_head(repo) {
            db.set_metadata(META_GIT_HEAD, &head)?;
        }
        eprintln!(
            "Indexed {} files, {} symbols, {} imports, {} calls, {} edges ({} skipped)",
            stats.files, stats.symbols, stats.imports, stats.calls, stats.edges, stats.skipped
        );
        return Ok(db);
    }

    let db = IndexDb::open(&path)?;
    let repo_path = repo.canonicalize()?;

    // Detect git branch/commit change — force incremental re-index
    let head_changed = has_git_head_changed(&db, repo);

    // Skip incremental walk if the index was checked recently and HEAD hasn't changed
    if !head_changed && is_index_fresh(&path) {
        return Ok(db);
    }

    // Try incremental update
    if let Some(stats) = indexer::index_repo_incremental(&repo_path, &db, verbose)? {
        eprintln!(
            "Incremental update: {} new/modified, {} unchanged, {} deleted ({} skipped)",
            stats.files, stats.unchanged, stats.deleted, stats.skipped
        );
    }
    if let Some(head) = git_head(repo) {
        db.set_metadata(META_GIT_HEAD, &head)?;
    }
    Ok(db)
}

/// Get the current git HEAD commit hash for a repo.
fn git_head(repo: &Path) -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Check if git HEAD has changed since the last index.
fn has_git_head_changed(db: &IndexDb, repo: &Path) -> bool {
    let current = git_head(repo);
    let stored = db.get_metadata(META_GIT_HEAD).ok().flatten();
    match (current, stored) {
        (Some(current), Some(stored)) => current != stored,
        (Some(_), None) => true, // first time tracking HEAD
        _ => false,              // not a git repo, skip
    }
}

/// Check if the index DB was modified recently enough to skip re-checking.
fn is_index_fresh(db_path: &Path) -> bool {
    let recheck_secs = std::env::var("PRUNER_RECHECK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_RECHECK_SECS);

    let Ok(meta) = fs::metadata(db_path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
        return false;
    };
    elapsed.as_secs() < recheck_secs
}

fn format_index_age(repo: &Path) -> String {
    let path = db_path(repo);
    let Ok(meta) = fs::metadata(&path) else {
        return String::new();
    };
    let Ok(modified) = meta.modified() else {
        return String::new();
    };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
        return String::new();
    };

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
const COPILOT_SKILL_MD: &str = include_str!("../.copilot/skills/pruner/SKILL.md");
const COPILOT_TEMPLATE_SKILL: &str = include_str!("../COPILOT.template.skill.md");
const COPILOT_TEMPLATE_HOOK: &str = include_str!("../COPILOT.template.hook.md");
const COPILOT_HOOK_JSON: &str = include_str!("../.copilot/hooks/pruner-context.json");
const COPILOT_HOOK_BASH: &str = include_str!("../.copilot/hooks/pruner-context.sh");
const COPILOT_HOOK_PS1: &str = include_str!("../.copilot/hooks/pruner-context.ps1");

fn cmd_init(
    repo: &Path,
    hook: bool,
    global: bool,
    copilot_skill: bool,
    copilot_hook: bool,
    copilot_global: bool,
) -> Result<()> {
    if copilot_hook && copilot_global {
        anyhow::bail!(
            "--copilot-hook is repository-local; do not combine it with --copilot-global"
        );
    }

    // Detect existing global install — skip project-level files if global hook is set up.
    // Hook injects context directly, so project-level skill/CLAUDE.md is redundant.
    // Global skill users still need per-repo CLAUDE.md for pruner instructions.
    let existing = crate::upgrade::detect_installed_integrations();
    let has_global_hook = existing.hook;

    let install_claude = (!copilot_skill && !copilot_global && !copilot_hook) || hook || global;

    // If running bare `pruner init` (no flags) and global hook is already installed,
    // skip project-level skill/CLAUDE.md — just do .gitignore + index.
    let bare_init = !hook && !global && !copilot_skill && !copilot_global && !copilot_hook;
    let skip_claude_project = bare_init && has_global_hook;

    if skip_claude_project {
        eprintln!("Global Claude hook detected — skipping project-level skill/CLAUDE.md");
    }

    if install_claude && !skip_claude_project {
        let claude_base = if global {
            dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
                .join(".claude")
        } else {
            repo.join(".claude")
        };

        // Install skill
        let skill_dir = claude_base.join("skills").join("pruner");
        fs::create_dir_all(&skill_dir)?;
        let skill_content = if hook { SKILL_HOOK_MD } else { SKILL_SKILL_MD };
        fs::write(skill_dir.join("SKILL.md"), skill_content)?;
        println!(
            "Installed Claude skill -> {}",
            skill_dir.join("SKILL.md").display()
        );

        // Install hook if requested
        if hook {
            let hook_dir = claude_base.join("hooks");
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
            let settings_path = claude_base.join("settings.json");
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
                        "command": path_to_hook_command(&hook_path),
                        "timeout": 60
                    }]
                }]
            });
            fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)?;
            println!("Updated settings -> {}", settings_path.display());
        }
    }

    if !global && !copilot_global {
        // Add .pruner/ to .gitignore
        let gitignore = repo.join(".gitignore");
        let gitignore_content = if gitignore.exists() {
            fs::read_to_string(&gitignore)?
        } else {
            String::new()
        };
        if !gitignore_content
            .lines()
            .any(|l| l.trim() == ".pruner/" || l.trim() == ".pruner")
        {
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&gitignore)?;
            use std::io::Write;
            if !gitignore_content.is_empty() && !gitignore_content.ends_with('\n') {
                writeln!(f)?;
            }
            writeln!(f, ".pruner/")?;
            println!("Updated .gitignore -> added .pruner/");
        }

        if install_claude && !skip_claude_project {
            let claude_md = repo.join("CLAUDE.md");
            upsert_pruner_section(&claude_md, CLAUDE_TEMPLATE)?;
            println!("Updated CLAUDE.md -> {}", claude_md.display());
        }
    }

    if copilot_skill || copilot_global || copilot_hook {
        let copilot_base = if copilot_global {
            dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
                .join(".copilot")
        } else {
            repo.join(".copilot")
        };

        if copilot_skill || copilot_global {
            let copilot_skill_dir = copilot_base.join("skills").join("pruner");
            fs::create_dir_all(&copilot_skill_dir)?;
            fs::write(copilot_skill_dir.join("SKILL.md"), COPILOT_SKILL_MD)?;
            println!(
                "Installed Copilot skill -> {}",
                copilot_skill_dir.join("SKILL.md").display()
            );
        }

        let copilot_instructions = if copilot_global {
            copilot_base.join("copilot-instructions.md")
        } else {
            repo.join(".github").join("copilot-instructions.md")
        };
        let template = if copilot_hook {
            COPILOT_TEMPLATE_HOOK
        } else {
            COPILOT_TEMPLATE_SKILL
        };
        upsert_pruner_section(&copilot_instructions, template)?;
        println!(
            "Updated Copilot instructions -> {}",
            copilot_instructions.display()
        );

        if copilot_hook {
            let hook_dir = repo.join(".github").join("hooks");
            fs::create_dir_all(&hook_dir)?;

            let hook_json_path = hook_dir.join("pruner-context.json");
            fs::write(&hook_json_path, COPILOT_HOOK_JSON)?;
            println!(
                "Installed Copilot hook config -> {}",
                hook_json_path.display()
            );

            let hook_bash_path = hook_dir.join("pruner-context.sh");
            fs::write(&hook_bash_path, COPILOT_HOOK_BASH)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&hook_bash_path, fs::Permissions::from_mode(0o755))?;
            }
            println!(
                "Installed Copilot hook bash -> {}",
                hook_bash_path.display()
            );

            let hook_ps_path = hook_dir.join("pruner-context.ps1");
            fs::write(&hook_ps_path, COPILOT_HOOK_PS1)?;
            println!("Installed Copilot hook pwsh -> {}", hook_ps_path.display());
        }
    }

    if (!global && install_claude) || ((copilot_skill || copilot_hook) && !copilot_global) {
        println!("\nIndexing {}...", repo.display());
        cmd_index(repo, false)?;
    }

    // Best-effort upgrade check (don't fail init if network is unavailable)
    if let Ok(latest) = crate::upgrade::check_latest_version() {
        let current = format!("v{}", env!("CARGO_PKG_VERSION"));
        if crate::upgrade::is_newer(&current, &latest) {
            println!("\nUpdate available: {current} -> {latest}");
            println!("Run `pruner upgrade` to update.");
        }
    }

    Ok(())
}

fn cmd_status(repo: Option<&Path>) -> Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));

    println!("pruner {version}");
    println!();

    // --- Global integrations ---
    println!("Global integrations:");

    let claude_dir = home.join(".claude");
    let claude_skill = claude_dir.join("skills/pruner/SKILL.md").exists();
    let claude_hook = claude_dir.join("hooks/pruner-context.sh").exists();
    let claude_settings_has_hook = has_pruner_hook(&claude_dir.join("settings.json"));

    if claude_skill || claude_hook {
        print!("  Claude Code: skill");
        if claude_hook || claude_settings_has_hook {
            print!(" + hook");
        }
        println!("  (~/.claude/)");
    } else {
        println!("  Claude Code: not installed");
    }

    let copilot_dir = home.join(".copilot");
    let copilot_skill = copilot_dir.join("skills/pruner/SKILL.md").exists();
    let copilot_instructions = has_pruner_section(&copilot_dir.join("copilot-instructions.md"));

    if copilot_skill || copilot_instructions {
        print!("  Copilot:     skill");
        if copilot_instructions {
            print!(" + instructions");
        }
        println!("  (~/.copilot/)");
    } else {
        println!("  Copilot:     not installed");
    }

    // --- Per-project integrations ---
    if let Some(repo) = repo {
        println!();
        println!("Project: {}", repo.display());

        let claude_dir = repo.join(".claude");
        let proj_claude_skill = claude_dir.join("skills/pruner/SKILL.md").exists();
        let proj_claude_hook = claude_dir.join("hooks/pruner-context.sh").exists();
        let proj_claude_settings = has_pruner_hook(&claude_dir.join("settings.json"));
        let proj_claude_md = has_pruner_section(&repo.join("CLAUDE.md"));

        if proj_claude_skill || proj_claude_hook || proj_claude_md {
            let mut parts = Vec::new();
            if proj_claude_skill {
                parts.push("skill");
            }
            if proj_claude_hook || proj_claude_settings {
                parts.push("hook");
            }
            if proj_claude_md {
                parts.push("CLAUDE.md");
            }
            println!("  Claude Code: {}", parts.join(" + "));
        } else {
            println!("  Claude Code: not installed");
        }

        let copilot_dir = repo.join(".copilot");
        let github_dir = repo.join(".github");
        let proj_copilot_skill = copilot_dir.join("skills/pruner/SKILL.md").exists();
        let proj_copilot_instructions =
            has_pruner_section(&github_dir.join("copilot-instructions.md"));
        let proj_copilot_hook = github_dir.join("hooks/pruner-context.json").exists();

        if proj_copilot_skill || proj_copilot_instructions || proj_copilot_hook {
            let mut parts = Vec::new();
            if proj_copilot_skill {
                parts.push("skill");
            }
            if proj_copilot_hook {
                parts.push("hook");
            }
            if proj_copilot_instructions {
                parts.push("instructions");
            }
            println!("  Copilot:     {}", parts.join(" + "));
        } else {
            println!("  Copilot:     not installed");
        }

        // Index status
        let index_path = repo.join(INDEX_DIR).join(DB_NAME);
        if index_path.exists() {
            let metadata = std::fs::metadata(&index_path)?;
            let modified = metadata.modified()?;
            let ago = SystemTime::now()
                .duration_since(modified)
                .unwrap_or_default();
            let ago_str = if ago.as_secs() < 60 {
                "just now".to_string()
            } else if ago.as_secs() < 3600 {
                format!("{}m ago", ago.as_secs() / 60)
            } else if ago.as_secs() < 86400 {
                format!("{}h ago", ago.as_secs() / 3600)
            } else {
                format!("{}d ago", ago.as_secs() / 86400)
            };
            println!(
                "  Index:       {} (updated {})",
                index_path.display(),
                ago_str
            );
        } else {
            println!("  Index:       not found (run `pruner index`)");
        }

        // .gitignore
        let gitignore = repo.join(".gitignore");
        if gitignore.exists() {
            let content = fs::read_to_string(&gitignore).unwrap_or_default();
            let has_entry = content
                .lines()
                .any(|l| l.trim() == ".pruner/" || l.trim() == ".pruner");
            if has_entry {
                println!("  .gitignore:  .pruner/ entry present");
            } else {
                println!("  .gitignore:  .pruner/ entry MISSING");
            }
        }
    } else {
        println!();
        println!("Tip: run `pruner status <path>` to see per-project integrations.");
    }

    // Check for updates (silently ignore network errors)
    if let Ok(latest) = crate::upgrade::check_latest_version()
        && crate::upgrade::is_newer(&version, &latest)
    {
        println!();
        println!("Update available: {version} -> {latest}");
        println!("Run `pruner upgrade` to install.");
    }

    Ok(())
}

/// Check if a settings.json file contains a pruner hook entry.
pub(crate) fn has_pruner_hook(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    content.contains("pruner")
}

/// Check if a markdown file contains a `## Pruner` section.
pub(crate) fn has_pruner_section(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    content.contains("## Pruner")
}

fn cmd_index(repo: &Path, verbose: bool) -> Result<()> {
    // Detect meta-repo: child directories with their own .git or index.
    // Only activate when the parent has no existing index (avoids submodule false positives).
    let db_file = db_path(repo);
    if !db_file.exists() {
        let subrepos = discover_subrepos(repo);
        if !subrepos.is_empty() {
            eprintln!(
                "Meta-repo detected: {} sub-repos found, indexing each separately",
                subrepos.len()
            );
            for subrepo in &subrepos {
                cmd_index_single(subrepo, verbose)?;
            }
            return Ok(());
        }
    }

    cmd_index_single(repo, verbose)
}

fn cmd_index_single(repo: &Path, verbose: bool) -> Result<()> {
    ensure_index_dir(repo)?;
    let path = db_path(repo);
    let db = IndexDb::open(&path)?;
    let repo_path = repo.canonicalize()?;

    eprintln!("Indexing {}...", repo_path.display());
    let stats = indexer::index_repo(&repo_path, &db, verbose)?;
    if let Some(head) = git_head(repo) {
        db.set_metadata(META_GIT_HEAD, &head)?;
    }
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
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ask": result.ask,
                "keywords": result.keywords,
                "subsystems": result.subsystems,
                "matching_files": result.matching_files.iter().map(|f| &f.path).collect::<Vec<_>>(),
                "matching_symbols": result.matching_symbols.iter().map(|s| &s.name).collect::<Vec<_>>(),
                "related_tests": result.related_tests.iter().map(|t| &t.path).collect::<Vec<_>>(),
                "execution_paths": result.execution_paths.len(),
            }))?
        );
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

/// Discover child directories that are git repos or already have a pruner index.
fn discover_subrepos(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut repos = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir()
            && (path.join(".git").exists() || path.join(INDEX_DIR).join(DB_NAME).exists())
        {
            repos.push(path);
        }
    }
    repos.sort();
    repos
}

fn cmd_context(
    repo: &Path,
    ask: &str,
    fmt: &str,
    max_snippet_lines: usize,
    mode: ContextMode,
    output: Option<&Path>,
) -> Result<()> {
    // Check for meta-repo pattern: child directories with their own .git or index.
    // Only activate when the parent has no index of its own (avoids submodule false positives).
    let db_file = db_path(repo);
    if !db_file.exists() {
        let subrepos = discover_subrepos(repo);
        if !subrepos.is_empty() {
            // Auto-index any sub-repos that don't have an index yet
            for subrepo in &subrepos {
                if !subrepo.join(INDEX_DIR).join(DB_NAME).exists() {
                    cmd_index_single(subrepo, false)?;
                }
            }
            return cmd_context_multi(repo, &subrepos, ask, fmt, max_snippet_lines, mode, output);
        }
    }

    let db = open_or_create_db(repo, false)?;
    let repo_path = repo.canonicalize()?;
    let result = query::analyze_query(ask, &db)?;

    let pruner_dir = repo_path.join(INDEX_DIR);
    let prev_query = if mode == ContextMode::Auto {
        budget::load_last_query(&pruner_dir).unwrap_or(None)
    } else {
        None
    };

    // Resolve auto mode: check query-aware budget first, then shape-based detection
    let resolved = if mode == ContextMode::Auto {
        let is_same_topic = prev_query.as_ref().is_some_and(|prev| {
            budget::decide_budget(&result.keywords, &result.subsystems, prev, None)
                == budget::Budget::Brief
        });
        if is_same_topic {
            eprintln!("Mode: auto → brief (same topic as previous query)");
            ContextMode::Brief
        } else {
            let detected = detect_mode(&result);
            let label = match detected {
                ContextMode::Brief => "brief (narrow task: few files, single subsystem)",
                ContextMode::Focused => "focused (broad task: multiple files/subsystems)",
                _ => unreachable!(),
            };
            eprintln!("Mode: auto → {label}");
            detected
        }
    } else {
        mode
    };

    let ctx = context::generate_context(&result, &repo_path, max_snippet_lines, resolved)?;

    // Compute output text and hash for identical-output detection
    let output_text = match fmt {
        "json" => format_context_json(&ctx)?,
        _ if resolved == ContextMode::Brief => format_context_summary(&ctx),
        _ => format_context_text(&ctx),
    };
    let output_hash = budget::hash_output(&output_text);

    // Skip entirely if output is identical to previous (auto mode only)
    if prev_query.as_ref().and_then(|p| p.output_hash.as_deref()) == Some(output_hash.as_str()) {
        eprintln!("Budget: skip (identical output to previous query)");
        let _ = budget::save_last_query(
            &pruner_dir,
            &budget::LastQuery {
                keywords: result.keywords,
                subsystems: result.subsystems,
                output_hash: Some(output_hash),
            },
        );
        return Ok(());
    }

    if resolved == ContextMode::Brief {
        // Write *full* context to .pruner/context.md so the LLM can drill deeper
        let full_ctx =
            context::generate_context(&result, &repo_path, max_snippet_lines, ContextMode::Full)?;
        let ctx_path = pruner_dir.join("context.md");
        let full_text = format_context_text(&full_ctx);
        fs::write(&ctx_path, &full_text)?;

        match fmt {
            "json" => println!("{}", format_context_json(&ctx)?),
            _ => {
                let age = format_index_age(repo);
                if !age.is_empty() {
                    eprintln!("Index age: {age}");
                }
                print!("{output_text}");
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
            _ => print!("{output_text}"),
        }
    }

    // Save current query metadata for next comparison
    if mode == ContextMode::Auto {
        let _ = budget::save_last_query(
            &pruner_dir,
            &budget::LastQuery {
                keywords: result.keywords,
                subsystems: result.subsystems,
                output_hash: Some(output_hash),
            },
        );
    }

    Ok(())
}

/// Minimum fraction of top subrepo score a subrepo must reach to be included.
const MULTI_REPO_SCORE_THRESHOLD: f64 = 0.3;

/// Run context across multiple sub-repos and combine output.
/// Scores each subrepo by relevance, drops low-scoring ones, and sorts by score.
fn cmd_context_multi(
    parent: &Path,
    subrepos: &[PathBuf],
    ask: &str,
    fmt: &str,
    max_snippet_lines: usize,
    mode: ContextMode,
    output: Option<&Path>,
) -> Result<()> {
    eprintln!("Multi-repo mode: {} sub-repos found", subrepos.len());

    // Phase 1: score all subrepos
    let mut scored: Vec<(&PathBuf, query::QueryResult, i32)> = Vec::new();
    for subrepo in subrepos {
        let db = open_or_create_db(subrepo, false)?;
        let result = query::analyze_query(ask, &db)?;

        if result.matching_files.is_empty() && result.matching_symbols.is_empty() {
            continue;
        }

        let score = result.relevance_score();
        scored.push((subrepo, result, score));
    }

    if scored.is_empty() {
        eprintln!("No relevant results found in any sub-repo.");
        return Ok(());
    }

    // Phase 2: filter out low-scoring subrepos relative to the best
    let max_score = scored.iter().map(|(_, _, s)| *s).max().unwrap_or(0);
    let threshold = (max_score as f64 * MULTI_REPO_SCORE_THRESHOLD) as i32;

    let mut skipped_names: Vec<String> = Vec::new();
    scored.retain(|(subrepo, _, score)| {
        let name = subrepo
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if *score < threshold {
            eprintln!("  Skipping {name} (score {score} < threshold {threshold})");
            skipped_names.push(name);
            false
        } else {
            eprintln!("  Including {name} (score {score})");
            true
        }
    });

    // Phase 3: sort by score descending (most relevant first)
    scored.sort_by(|a, b| b.2.cmp(&a.2));

    // Phase 4: generate context output with multi-repo header
    let mut combined_text = String::new();
    let mut combined_json: Vec<serde_json::Value> = Vec::new();

    // Inject multi-repo awareness header for the LLM
    let included_names: Vec<String> = scored
        .iter()
        .map(|(s, _, _)| {
            s.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        })
        .collect();

    if fmt != "json" {
        combined_text.push_str("**Multi-repo context:** results from ");
        combined_text.push_str(&included_names.join(", "));
        if !skipped_names.is_empty() {
            combined_text.push_str(&format!(
                " (skipped low-relevance: {})",
                skipped_names.join(", ")
            ));
        }
        combined_text.push_str("\n\n");
    }

    for (subrepo, result, _score) in &scored {
        let repo_path = subrepo.canonicalize()?;

        let resolved = if mode == ContextMode::Auto {
            detect_mode(result)
        } else {
            mode
        };

        let ctx = context::generate_context(result, &repo_path, max_snippet_lines, resolved)?;

        let repo_name = subrepo
            .strip_prefix(parent)
            .unwrap_or(subrepo)
            .display()
            .to_string();

        match fmt {
            "json" => {
                let mut json: serde_json::Value =
                    serde_json::from_str(&format_context_json(&ctx)?)?;
                json["repo"] = serde_json::Value::String(repo_name);
                combined_json.push(json);
            }
            _ => {
                if !combined_text.is_empty() {
                    combined_text.push('\n');
                }
                combined_text.push_str(&format!("# Repo: {repo_name}\n\n"));
                if resolved == ContextMode::Brief {
                    combined_text.push_str(&format_context_summary(&ctx));
                } else {
                    combined_text.push_str(&format_context_text(&ctx));
                }
            }
        }
    }

    match fmt {
        "json" => {
            let wrapper = serde_json::json!({
                "multi_repo": true,
                "included": included_names,
                "skipped": skipped_names,
                "repos": combined_json,
            });
            println!("{}", serde_json::to_string_pretty(&wrapper)?);
        }
        "both" => {
            let wrapper = serde_json::json!({
                "multi_repo": true,
                "included": included_names,
                "skipped": skipped_names,
                "repos": combined_json,
            });
            let json_str = serde_json::to_string_pretty(&wrapper)?;
            println!("{combined_text}");
            if let Some(out) = output {
                fs::write(out.join("context.json"), &json_str)?;
                fs::write(out.join("context.md"), &combined_text)?;
            }
        }
        _ => print!("{combined_text}"),
    }

    Ok(())
}

fn cmd_show_file(repo: &Path, path: &str) -> Result<()> {
    let db = open_db(repo)?;
    let file = db
        .get_file_by_path(path)?
        .ok_or_else(|| anyhow::anyhow!("File not found in index: {path}"))?;

    println!("Path: {}", file.path);
    println!(
        "Language: {}",
        file.language.as_deref().unwrap_or("unknown")
    );
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
        println!(
            "{} ({}) — {}:{}-{}",
            s.name, s.kind, s.file_path, s.line_start, s.line_end
        );
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

fn delta_pct(from: usize, to: usize) -> String {
    if from == 0 {
        return String::new();
    }
    let pct = (to as f64 - from as f64) / from as f64 * 100.0;
    format!("{pct:+.0}%")
}

fn delta_pct_f64(from: f64, to: f64) -> String {
    if from == 0.0 {
        return String::new();
    }
    let pct = (to - from) / from * 100.0;
    format!("{pct:+.0}%")
}

fn format_tokens(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn format_tokens_signed(n: i64) -> String {
    let abs = n.unsigned_abs() as usize;
    let formatted = format_tokens(abs);
    if n >= 0 {
        format!("+{formatted}")
    } else {
        format!("-{formatted}")
    }
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
                "turns": est.without_turns.len(),
                "tool_calls": est.without_tool_calls,
                "input_tokens": est.without_input_tokens,
                "output_tokens": est.without_output_tokens,
                "total_tokens": est.without_total_tokens,
                "files_read": est.without_files_read,
                "irrelevant_reads": est.without_irrelevant_reads,
                "wall_secs": est.without_wall_secs,
                "cost_usd": (est.without_cost() * 10000.0).round() / 10000.0,
            },
            "with_pruner": {
                "turns": est.with_turns.len(),
                "tool_calls": est.with_tool_calls,
                "input_tokens": est.with_input_tokens,
                "output_tokens": est.with_output_tokens,
                "total_tokens": est.with_total_tokens,
                "files_read": est.with_files_read,
                "wall_secs": est.with_wall_secs,
                "cost_usd": (est.with_cost() * 10000.0).round() / 10000.0,
            },
            "saving_tokens": est.token_saving(),
            "saving_pct": (est.saving_pct() * 10.0).round() / 10.0,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Claude Code session estimate for: \"{}\"", est.ask);
        println!();

        // Table header
        println!(
            "  {:<20} {:>12} {:>12} {:>10} {:>8}",
            "", "Without", "With pruner", "Delta", "Δ%"
        );
        println!("  {}", "-".repeat(66));

        // Turns
        let turn_delta = est.with_turns.len() as i64 - est.without_turns.len() as i64;
        println!(
            "  {:<20} {:>12} {:>12} {:>+10} {:>7}",
            "Turns",
            est.without_turns.len(),
            est.with_turns.len(),
            turn_delta,
            delta_pct(est.without_turns.len(), est.with_turns.len())
        );

        // Tool calls
        let tool_delta = est.with_tool_calls as i64 - est.without_tool_calls as i64;
        println!(
            "  {:<20} {:>12} {:>12} {:>+10} {:>7}",
            "Tool calls",
            est.without_tool_calls,
            est.with_tool_calls,
            tool_delta,
            delta_pct(est.without_tool_calls, est.with_tool_calls)
        );

        // Files read
        let file_delta = est.with_files_read as i64 - est.without_files_read as i64;
        let without_files_detail = if est.without_irrelevant_reads > 0 {
            format!(
                "{} ({} waste)",
                est.without_files_read, est.without_irrelevant_reads
            )
        } else {
            est.without_files_read.to_string()
        };
        println!(
            "  {:<20} {:>12} {:>12} {:>+10} {:>7}",
            "Files read",
            without_files_detail,
            est.with_files_read,
            file_delta,
            delta_pct(est.without_files_read, est.with_files_read)
        );

        // Input tokens
        let input_delta = est.with_input_tokens as i64 - est.without_input_tokens as i64;
        println!(
            "  {:<20} {:>12} {:>12} {:>10} {:>7}",
            "Input tokens",
            format_tokens(est.without_input_tokens),
            format_tokens(est.with_input_tokens),
            format_tokens_signed(input_delta),
            delta_pct(est.without_input_tokens, est.with_input_tokens)
        );

        // Output tokens
        let output_delta = est.with_output_tokens as i64 - est.without_output_tokens as i64;
        println!(
            "  {:<20} {:>12} {:>12} {:>10} {:>7}",
            "Output tokens",
            format_tokens(est.without_output_tokens),
            format_tokens(est.with_output_tokens),
            format_tokens_signed(output_delta),
            delta_pct(est.without_output_tokens, est.with_output_tokens)
        );

        // Total tokens
        println!(
            "  {:<20} {:>12} {:>12} {:>10} {:>7}",
            "Total tokens",
            format_tokens(est.without_total_tokens),
            format_tokens(est.with_total_tokens),
            format_tokens_signed(-est.token_saving()),
            delta_pct(est.without_total_tokens, est.with_total_tokens)
        );

        // Cost
        let cost_delta = est.with_cost() - est.without_cost();
        println!(
            "  {:<20} {:>11} {:>11} {:>+10} {:>7}",
            "Est. cost",
            format!("${:.4}", est.without_cost()),
            format!("${:.4}", est.with_cost()),
            format!("${:.4}", cost_delta),
            delta_pct_f64(est.without_cost(), est.with_cost())
        );

        // Wall time
        let time_delta = est.with_wall_secs - est.without_wall_secs;
        println!(
            "  {:<20} {:>11}s {:>11}s {:>+9}s {:>7}",
            "Est. wall time",
            format!("{:.0}", est.without_wall_secs),
            format!("{:.0}", est.with_wall_secs),
            format!("{:.0}", time_delta),
            delta_pct_f64(est.without_wall_secs, est.with_wall_secs)
        );

        println!();
        println!(
            "Estimated saving: {:.1}% tokens, {:.1}% cost",
            est.saving_pct(),
            if est.without_cost() > 0.0 {
                (1.0 - est.with_cost() / est.without_cost()) * 100.0
            } else {
                0.0
            }
        );
        println!("Note: models multi-turn context accumulation (each turn re-sends full history)");

        if show_steps {
            println!();
            println!("Without pruner — turn-by-turn breakdown:");
            for (i, turn) in est.without_turns.iter().enumerate() {
                println!("  Turn {} (+{} new tokens):", i + 1, turn.new_tokens);
                for step in &turn.steps {
                    let marker = if step.useful { " " } else { "*" };
                    println!(
                        "    {} {:10} {:40} ~{} tok",
                        marker, step.action, step.target, step.tokens
                    );
                }
            }
            println!("  (* = wasted on irrelevant content)");
        }
    }
    Ok(())
}

/// Convert a path to a forward-slash string for use in shell commands.
/// On Windows, `PathBuf::to_str` returns backslash-separated paths. Bash (used
/// by Claude Code to execute hooks) treats backslashes as escape sequences, so
/// `C:\Users\foo` becomes `C:Usersfoo`. Forward slashes work on all platforms.
fn path_to_hook_command(path: &std::path::Path) -> String {
    path.to_str().unwrap().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_hook_command_uses_forward_slashes() {
        // Windows-style path with backslashes — must be normalised to forward slashes
        // so that bash (used by Claude Code to run hooks) does not interpret them as
        // escape sequences (which would turn C:\Users\foo into C:Usersfoo).
        let hook_path = Path::new(r"C:\Users\testuser\.claude\hooks\pruner-context.sh");
        let command = path_to_hook_command(hook_path);
        assert!(
            !command.contains('\\'),
            "hook command path must use forward slashes for bash compatibility, got: {command}"
        );
        assert_eq!(command, "C:/Users/testuser/.claude/hooks/pruner-context.sh");
    }
}
