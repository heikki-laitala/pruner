//! Uninstall: remove pruner integrations (hooks, skills, config sections) and optionally the binary.

use anyhow::{Context, Result};
use std::fmt;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

fn clean_codex_hooks_json(path: &Path) {
    if !path.exists() {
        return;
    }
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let Ok(mut config) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };

    let mut changed = false;
    if let Some(hooks) = config.get_mut("hooks").and_then(|h| h.as_object_mut())
        && let Some(submit) = hooks
            .get_mut("UserPromptSubmit")
            .and_then(|s| s.as_array_mut())
    {
        for entry in submit.iter_mut() {
            if let Some(hook_arr) = entry.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                let before = hook_arr.len();
                hook_arr.retain(|h| {
                    !h.get("command")
                        .and_then(|c| c.as_str())
                        .is_some_and(|c| c.contains("pruner-context"))
                });
                if hook_arr.len() < before {
                    changed = true;
                }
            }
        }
        submit.retain(|entry| {
            entry
                .get("hooks")
                .and_then(|h| h.as_array())
                .is_none_or(|a| !a.is_empty())
        });
        if submit.is_empty() {
            hooks.remove("UserPromptSubmit");
        }
    }

    if changed {
        let remove_file = config
            .as_object()
            .and_then(|o| o.get("hooks"))
            .and_then(|h| h.as_object())
            .is_none_or(|h| h.is_empty());
        if remove_file {
            let _ = fs::remove_file(path);
            println!("  Removed {} (was pruner-only)", path.display());
        } else if let Ok(json) = serde_json::to_string_pretty(&config) {
            let _ = fs::write(path, json);
            println!("  Cleaned pruner hook from {}", path.display());
        }
    }
}

fn clean_codex_config_toml(path: &Path) {
    if !path.exists() {
        return;
    }
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let Ok(mut config) = content.parse::<toml::Value>() else {
        return;
    };

    let mut changed = false;
    if let Some(root) = config.as_table_mut()
        && let Some(features) = root.get_mut("features").and_then(|f| f.as_table_mut())
        && features.remove("codex_hooks").is_some()
    {
        changed = true;
        if features.is_empty() {
            root.remove("features");
        }
    }

    if changed {
        if config.as_table().is_some_and(|t| t.is_empty()) {
            let _ = fs::remove_file(path);
            println!("  Removed {} (was pruner-only)", path.display());
        } else if let Ok(toml_str) = toml::to_string_pretty(&config) {
            let _ = fs::write(path, toml_str);
            println!("  Cleaned pruner hook flag from {}", path.display());
        }
    }
}

/// Remove a file if it exists, printing what was removed.
fn remove_file(path: &Path) {
    if path.exists() {
        if let Err(e) = fs::remove_file(path) {
            eprintln!("  Warning: could not remove {}: {e}", path.display());
        } else {
            println!("  Removed {}", path.display());
        }
    }
}

/// Remove a directory if it exists, printing what was removed.
fn remove_dir(path: &Path) {
    if path.exists() {
        if let Err(e) = fs::remove_dir_all(path) {
            eprintln!("  Warning: could not remove {}: {e}", path.display());
        } else {
            println!("  Removed {}", path.display());
        }
    }
}

/// Remove the `## Pruner` section from a markdown file.
/// Returns true if the section was found and removed.
fn remove_pruner_section(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };

    const MARKER: &str = "## Pruner";
    let Some(start) = content.find(MARKER) else {
        return false;
    };

    let after_marker = start + MARKER.len();
    let end = content[after_marker..]
        .find("\n## ")
        .map(|i| after_marker + i + 1)
        .unwrap_or(content.len());

    let mut result = content[..start].to_string();
    if end < content.len() {
        result.push_str(&content[end..]);
    }

    // Trim trailing whitespace/newlines
    let result = result.trim_end().to_string();

    if result.is_empty() {
        // File had only the pruner section — remove it entirely
        let _ = fs::remove_file(path);
        println!("  Removed {} (was pruner-only)", path.display());
    } else {
        let _ = fs::write(path, format!("{result}\n"));
        println!("  Cleaned pruner section from {}", path.display());
    }
    true
}

/// Remove the pruner hook entry from a Claude settings.json file.
fn clean_settings_json(path: &Path) {
    if !path.exists() {
        return;
    }
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let Ok(mut settings) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };

    // Remove pruner hooks from UserPromptSubmit
    let mut changed = false;
    if let Some(hooks) = settings.get_mut("hooks") {
        if let Some(submit) = hooks.get_mut("UserPromptSubmit")
            && let Some(arr) = submit.as_array_mut()
        {
            // First pass: filter individual pruner hooks within each entry
            for entry in arr.iter_mut() {
                if let Some(hook_list) = entry.get_mut("hooks")
                    && let Some(hook_arr) = hook_list.as_array_mut()
                {
                    let before = hook_arr.len();
                    hook_arr.retain(|h| {
                        let is_pruner = h
                            .get("command")
                            .and_then(|c| c.as_str())
                            .is_some_and(|c| c.contains("pruner"));
                        !is_pruner
                    });
                    if hook_arr.len() < before {
                        changed = true;
                    }
                }
            }
            // Second pass: drop entries whose hooks array is now empty
            arr.retain(|entry| {
                entry
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .is_none_or(|a| !a.is_empty())
            });
            // If UserPromptSubmit is now empty, remove it
            if arr.is_empty() {
                hooks.as_object_mut().map(|m| m.remove("UserPromptSubmit"));
            }
        }
        // If hooks object is now empty, remove it
        if hooks.as_object().is_some_and(|m| m.is_empty()) {
            settings.as_object_mut().map(|m| m.remove("hooks"));
        }
    }

    if changed {
        if settings.as_object().is_some_and(|m| m.is_empty()) {
            let _ = fs::remove_file(path);
            println!("  Removed {} (was pruner-only)", path.display());
        } else {
            let _ = fs::write(path, serde_json::to_string_pretty(&settings).unwrap());
            println!("  Cleaned pruner hook from {}", path.display());
        }
    }
}

/// Remove `.pruner/` line from .gitignore.
fn clean_gitignore(path: &Path) {
    if !path.exists() {
        return;
    }
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };

    let lines: Vec<&str> = content.lines().collect();
    let filtered: Vec<&str> = lines
        .iter()
        .filter(|l| {
            let trimmed = l.trim();
            trimmed != ".pruner/" && trimmed != ".pruner"
        })
        .copied()
        .collect();

    if filtered.len() < lines.len() {
        let result = filtered.join("\n");
        let result = result.trim_end().to_string();
        if result.is_empty() {
            let _ = fs::remove_file(path);
            println!("  Removed {} (was pruner-only)", path.display());
        } else {
            let _ = fs::write(path, format!("{result}\n"));
            println!("  Cleaned .pruner/ from {}", path.display());
        }
    }
}

/// Remove global Claude integrations from ~/.claude/
fn uninstall_claude_global(home: &Path) {
    let claude = home.join(".claude");
    remove_file(&claude.join("hooks/pruner-context.sh"));
    remove_dir(&claude.join("skills/pruner"));
    clean_settings_json(&claude.join("settings.json"));
}

/// Remove global Copilot integrations from ~/.copilot/
fn uninstall_copilot_global(home: &Path) {
    let copilot = home.join(".copilot");
    remove_dir(&copilot.join("skills/pruner"));
    remove_pruner_section(&copilot.join("copilot-instructions.md"));
    // Global copilot hooks
    remove_file(&copilot.join("hooks/pruner-context.json"));
    remove_file(&copilot.join("hooks/pruner-context.sh"));
    remove_file(&copilot.join("hooks/pruner-context.ps1"));
}

/// Remove global Codex integrations from ~/.codex/
fn uninstall_codex_global(home: &Path) {
    let codex = home.join(".codex");
    remove_dir(&codex.join("skills/pruner"));
    remove_file(&codex.join("hooks/pruner-context.sh"));
    clean_codex_hooks_json(&codex.join("hooks.json"));
    clean_codex_config_toml(&codex.join("config.toml"));
}

/// Remove per-project Claude integrations
fn uninstall_claude_project(repo: &Path) {
    let claude = repo.join(".claude");
    remove_file(&claude.join("hooks/pruner-context.sh"));
    remove_dir(&claude.join("skills/pruner"));
    clean_settings_json(&claude.join("settings.json"));
    remove_pruner_section(&repo.join("CLAUDE.md"));
}

/// Remove per-project Copilot integrations
fn uninstall_copilot_project(repo: &Path) {
    remove_dir(&repo.join(".copilot/skills/pruner"));
    remove_pruner_section(&repo.join(".github/copilot-instructions.md"));
    remove_file(&repo.join(".github/hooks/pruner-context.json"));
    remove_file(&repo.join(".github/hooks/pruner-context.sh"));
    remove_file(&repo.join(".github/hooks/pruner-context.ps1"));
}

/// Remove per-project Codex integrations
fn uninstall_codex_project(repo: &Path) {
    remove_dir(&repo.join(".codex/skills/pruner"));
    remove_file(&repo.join(".codex/hooks/pruner-context.sh"));
    clean_codex_hooks_json(&repo.join(".codex/hooks.json"));
    clean_codex_config_toml(&repo.join(".codex/config.toml"));
    remove_pruner_section(&repo.join("AGENTS.md"));
}

// ---------------------------------------------------------------------------
// Scan: find leftover pruner traces across the filesystem
// ---------------------------------------------------------------------------

/// What kind of pruner trace was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TraceKind {
    PrunerDir,
    ClaudeSkillDir,
    CopilotSkillDir,
    CodexSkillDir,
    PrunerSection,
    SettingsHook,
    CodexHookConfig,
    HookFile,
    GitignoreEntry,
}

impl fmt::Display for TraceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TraceKind::PrunerDir => write!(f, ".pruner/ index"),
            TraceKind::ClaudeSkillDir => write!(f, "Claude skill"),
            TraceKind::CopilotSkillDir => write!(f, "Copilot skill"),
            TraceKind::CodexSkillDir => write!(f, "Codex skill"),
            TraceKind::PrunerSection => write!(f, "pruner section"),
            TraceKind::SettingsHook => write!(f, "settings hook"),
            TraceKind::CodexHookConfig => write!(f, "Codex hook config"),
            TraceKind::HookFile => write!(f, "hook file"),
            TraceKind::GitignoreEntry => write!(f, ".gitignore entry"),
        }
    }
}

/// A single pruner trace found on disk.
#[derive(Debug, Clone)]
pub(crate) struct FoundTrace {
    pub(crate) kind: TraceKind,
    pub(crate) path: PathBuf,
    pub(crate) project: PathBuf,
}

impl fmt::Display for FoundTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.path.display(), self.kind)
    }
}

impl FoundTrace {
    /// Remove this trace using the appropriate cleanup method.
    fn remove(&self) {
        match self.kind {
            TraceKind::PrunerDir
            | TraceKind::ClaudeSkillDir
            | TraceKind::CopilotSkillDir
            | TraceKind::CodexSkillDir => {
                remove_dir(&self.path);
            }
            TraceKind::HookFile => {
                remove_file(&self.path);
            }
            TraceKind::PrunerSection => {
                remove_pruner_section(&self.path);
            }
            TraceKind::SettingsHook => {
                clean_settings_json(&self.path);
            }
            TraceKind::CodexHookConfig => {
                if self.path.file_name().is_some_and(|n| n == "hooks.json") {
                    clean_codex_hooks_json(&self.path);
                } else {
                    clean_codex_config_toml(&self.path);
                }
            }
            TraceKind::GitignoreEntry => {
                clean_gitignore(&self.path);
            }
        }
    }
}

/// Infer the project root from a trace path by stripping known integration subdirs.
/// e.g. `/home/user/myproject/.claude/skills/pruner` -> `/home/user/myproject`
fn infer_project(path: &Path) -> PathBuf {
    const MARKERS: &[&str] = &[".claude", ".copilot", ".codex", ".github", ".pruner"];
    // Walk ancestors; the project root is the parent of the first known marker dir.
    for ancestor in path.ancestors() {
        if let Some(name) = ancestor.file_name()
            && MARKERS.iter().any(|m| name == *m)
        {
            return ancestor.parent().unwrap_or(ancestor).to_path_buf();
        }
    }
    // Fallback: parent directory
    path.parent().unwrap_or(path).to_path_buf()
}

/// Directories to skip during scan (never contain pruner traces, often large).
const SCAN_SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    "venv",
    ".venv",
    "build",
    "dist",
    ".Trash",
    "Library",
];

/// Check if a settings.json contains a pruner hook command (exact match on `pruner-context`).
fn has_pruner_hook_entry(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    // Walk hooks.UserPromptSubmit[].hooks[].command looking for pruner-context
    settings
        .get("hooks")
        .and_then(|h| h.get("UserPromptSubmit"))
        .and_then(|s| s.as_array())
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .is_some_and(|hooks| {
                        hooks.iter().any(|h| {
                            h.get("command")
                                .and_then(|c| c.as_str())
                                .is_some_and(|c| c.contains("pruner-context"))
                        })
                    })
            })
        })
}

/// Check if a directory entry is a pruner trace.
fn match_dir_trace(path: &Path, name: &str) -> Option<TraceKind> {
    match name {
        ".pruner" => Some(TraceKind::PrunerDir),
        "pruner" => {
            // Match .claude/skills/pruner or .copilot/skills/pruner
            let parent = path.parent()?;
            if parent.file_name()?.to_str()? != "skills" {
                return None;
            }
            match parent.parent()?.file_name()?.to_str()? {
                ".claude" => Some(TraceKind::ClaudeSkillDir),
                ".copilot" => Some(TraceKind::CopilotSkillDir),
                ".codex" => Some(TraceKind::CodexSkillDir),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Check if a file entry is a pruner trace.
fn match_file_trace(path: &Path, name: &str) -> Option<TraceKind> {
    match name {
        "pruner-context.sh" | "pruner-context.json" | "pruner-context.ps1" => {
            // Only match hook files inside known integration directories
            let in_hooks_dir = path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == "hooks");
            let parent_of_hooks = path.ancestors().nth(2).and_then(|p| p.file_name());
            let in_integration_dir = parent_of_hooks.is_some_and(|n| {
                n == ".claude" || n == ".copilot" || n == ".codex" || n == ".github"
            });
            (in_hooks_dir && in_integration_dir).then_some(TraceKind::HookFile)
        }
        "CLAUDE.md" | "AGENTS.md" | "copilot-instructions.md" => {
            crate::cli::has_pruner_section(path).then_some(TraceKind::PrunerSection)
        }
        "settings.json" => {
            let in_claude = path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == ".claude");
            (in_claude && has_pruner_hook_entry(path)).then_some(TraceKind::SettingsHook)
        }
        "hooks.json" => {
            let in_codex = path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == ".codex");
            (in_codex && crate::cli::has_codex_hook(path)).then_some(TraceKind::CodexHookConfig)
        }
        "config.toml" => {
            let in_codex = path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == ".codex");
            (in_codex && fs::read_to_string(path).is_ok_and(|c| c.contains("codex_hooks = true")))
                .then_some(TraceKind::CodexHookConfig)
        }
        ".gitignore" => {
            let content = fs::read_to_string(path).ok()?;
            content
                .lines()
                .any(|l| {
                    let t = l.trim();
                    t == ".pruner/" || t == ".pruner"
                })
                .then_some(TraceKind::GitignoreEntry)
        }
        _ => None,
    }
}

/// Scan a directory tree for leftover pruner traces.
pub(crate) fn scan_for_traces(root: &Path) -> Vec<FoundTrace> {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .max_depth(Some(6))
        .follow_links(false)
        .filter_entry(|e| {
            if e.file_type().is_some_and(|ft| ft.is_dir()) {
                let name = e.file_name().to_string_lossy();
                return !SCAN_SKIP_DIRS.contains(&name.as_ref());
            }
            true
        })
        .build();

    let mut traces = Vec::new();

    for entry in walker.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy();
        let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());

        if is_dir {
            if let Some(kind) = match_dir_trace(path, &name) {
                traces.push(FoundTrace {
                    kind,
                    project: infer_project(path),
                    path: path.to_path_buf(),
                });
            }
        } else {
            // Skip large files
            if entry.metadata().map(|m| m.len()).unwrap_or(0) > 1_000_000 {
                continue;
            }
            if let Some(kind) = match_file_trace(path, &name) {
                traces.push(FoundTrace {
                    kind,
                    project: infer_project(path),
                    path: path.to_path_buf(),
                });
            }
        }
    }

    // Sort by project then path for grouped display
    traces.sort_by(|a, b| (&a.project, &a.path).cmp(&(&b.project, &b.path)));
    traces
}

/// Print scan results grouped by project.
fn print_trace_summary(traces: &[FoundTrace]) {
    // Group by project
    let mut current_project: Option<&Path> = None;
    let mut project_count = 0;
    for trace in traces {
        if current_project != Some(&trace.project) {
            project_count += 1;
        }
        current_project = Some(&trace.project);
    }

    println!("Found pruner traces in {project_count} project(s):\n");

    current_project = None;
    for trace in traces {
        if current_project != Some(&trace.project) {
            println!("  {}/", trace.project.display());
            current_project = Some(&trace.project);
        }
        println!("    {} ({})", trace.path.display(), trace.kind);
    }
}

/// Read a single line from stdin.
fn read_line() -> String {
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);
    buf.trim().to_lowercase()
}

/// Prompt user for scan action: all, one-by-one, or skip.
fn prompt_scan_action() -> ScanAction {
    if !std::io::stdin().is_terminal() {
        return ScanAction::Skip;
    }
    print!("\nRemove? [a]ll / [o]ne-by-one / [s]kip (default: skip): ");
    let _ = std::io::stdout().flush();
    match read_line().chars().next() {
        Some('a') => ScanAction::All,
        Some('o') => ScanAction::OneByOne,
        _ => ScanAction::Skip,
    }
}

/// Prompt for a single yes/no decision.
fn prompt_yes_no(description: &str) -> bool {
    print!("  Remove {description}? [y/n]: ");
    let _ = std::io::stdout().flush();
    read_line().starts_with('y')
}

enum ScanAction {
    All,
    OneByOne,
    Skip,
}

/// Main uninstall entrypoint.
pub fn cmd_uninstall(repo: Option<&Path>, purge: bool) -> Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;

    if let Some(repo) = repo {
        // Per-project uninstall
        println!("Removing pruner from {}...", repo.display());

        uninstall_claude_project(repo);
        uninstall_copilot_project(repo);
        uninstall_codex_project(repo);
        clean_gitignore(&repo.join(".gitignore"));

        if purge {
            remove_dir(&repo.join(".pruner"));
        } else if repo.join(".pruner").exists() {
            println!(
                "\n  Note: .pruner/ index kept at {}. Use --purge to remove it.",
                repo.join(".pruner").display()
            );
        }

        println!("\nDone. Per-project pruner integration removed.");
    } else {
        // Global uninstall
        println!("Removing global pruner integrations...");

        uninstall_claude_global(&home);
        uninstall_copilot_global(&home);
        uninstall_codex_global(&home);

        // Scan for project-level traces
        println!("\nScanning for project-level pruner traces...");
        let traces = scan_for_traces(&home);

        if traces.is_empty() {
            println!("  No leftover traces found.");
        } else if purge {
            print_trace_summary(&traces);
            println!("\nRemoving all (--purge)...");
            for trace in &traces {
                trace.remove();
            }
        } else {
            print_trace_summary(&traces);
            match prompt_scan_action() {
                ScanAction::All => {
                    for trace in &traces {
                        trace.remove();
                    }
                }
                ScanAction::OneByOne => {
                    for trace in &traces {
                        if prompt_yes_no(&trace.to_string()) {
                            trace.remove();
                        }
                    }
                }
                ScanAction::Skip => {
                    println!("  Skipped. Run `pruner uninstall --purge` to remove all.");
                }
            }
        }

        // Remove the binary
        let exe = std::env::current_exe().context("Cannot determine binary path")?;
        println!("\nBinary: {}", exe.display());

        #[cfg(unix)]
        {
            if let Err(e) = fs::remove_file(&exe) {
                eprintln!("  Warning: could not remove binary: {e}");
                eprintln!("  Remove it manually: rm {}", exe.display());
            } else {
                println!("  Removed {}", exe.display());
            }
        }

        #[cfg(windows)]
        {
            println!("  Cannot delete a running executable on Windows.");
            println!("  Remove it manually: del \"{}\"", exe.display());
        }

        println!("\nDone. Global pruner integrations removed.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: create directories and an empty file.
    fn create_file(base: &Path, rel: &str) {
        let path = base.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "").unwrap();
    }

    #[test]
    fn test_scan_finds_pruner_dir() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("myproject");
        fs::create_dir_all(project.join(".pruner")).unwrap();
        fs::write(project.join(".pruner/index.db"), "fake").unwrap();

        let traces = scan_for_traces(dir.path());
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].kind, TraceKind::PrunerDir);
        assert_eq!(traces[0].project, project);
    }

    #[test]
    fn test_scan_finds_claude_skill_dir() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("myproject");
        create_file(&project, ".claude/skills/pruner/SKILL.md");

        let traces = scan_for_traces(dir.path());
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].kind, TraceKind::ClaudeSkillDir);
        assert_eq!(traces[0].project, project);
    }

    #[test]
    fn test_scan_finds_copilot_skill_dir() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("myproject");
        create_file(&project, ".copilot/skills/pruner/SKILL.md");

        let traces = scan_for_traces(dir.path());
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].kind, TraceKind::CopilotSkillDir);
        assert_eq!(traces[0].project, project);
    }

    #[test]
    fn test_scan_finds_hook_file() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("myproject");
        create_file(&project, ".claude/hooks/pruner-context.sh");

        let traces = scan_for_traces(dir.path());
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].kind, TraceKind::HookFile);
    }

    #[test]
    fn test_scan_skips_hook_file_outside_integration_dir() {
        let dir = tempfile::tempdir().unwrap();
        // A pruner-context.sh in a random directory should NOT be detected
        create_file(&dir.path().join("myproject"), "scripts/pruner-context.sh");

        let traces = scan_for_traces(dir.path());
        assert!(
            traces.is_empty(),
            "hook files outside integration dirs should be skipped"
        );
    }

    #[test]
    fn test_scan_finds_pruner_section_in_claude_md() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("myproject");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("CLAUDE.md"),
            "# Project\n\n## Pruner\n\nAuto-generated.\n",
        )
        .unwrap();

        let traces = scan_for_traces(dir.path());
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].kind, TraceKind::PrunerSection);
        assert_eq!(traces[0].project, project);
    }

    #[test]
    fn test_scan_finds_settings_hook() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("myproject");
        fs::create_dir_all(project.join(".claude")).unwrap();
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [{
                        "command": "pruner-context.sh"
                    }]
                }]
            }
        });
        fs::write(
            project.join(".claude/settings.json"),
            serde_json::to_string(&settings).unwrap(),
        )
        .unwrap();

        let traces = scan_for_traces(dir.path());
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].kind, TraceKind::SettingsHook);
        assert_eq!(traces[0].project, project);
    }

    #[test]
    fn test_scan_skips_non_pruner_hook_in_settings() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("myproject");
        fs::create_dir_all(project.join(".claude")).unwrap();
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [{
                        "command": "superpruner-tool.sh"
                    }]
                }]
            }
        });
        fs::write(
            project.join(".claude/settings.json"),
            serde_json::to_string(&settings).unwrap(),
        )
        .unwrap();

        let traces = scan_for_traces(dir.path());
        assert!(
            traces.is_empty(),
            "settings with non-pruner hooks should not be detected"
        );
    }

    #[test]
    fn test_scan_finds_gitignore_entry() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("myproject");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join(".gitignore"), "node_modules/\n.pruner/\n").unwrap();

        let traces = scan_for_traces(dir.path());
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].kind, TraceKind::GitignoreEntry);
        assert_eq!(traces[0].project, project);
    }

    #[test]
    fn test_scan_skips_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let hidden = dir.path().join("node_modules/somepackage");
        fs::create_dir_all(hidden.join(".pruner")).unwrap();
        fs::write(hidden.join(".pruner/index.db"), "fake").unwrap();

        let traces = scan_for_traces(dir.path());
        assert!(traces.is_empty(), "should skip node_modules");
    }

    #[test]
    fn test_scan_multiple_projects() {
        let dir = tempfile::tempdir().unwrap();

        let p1 = dir.path().join("project1");
        fs::create_dir_all(p1.join(".pruner")).unwrap();
        fs::write(p1.join(".pruner/index.db"), "fake").unwrap();

        let p2 = dir.path().join("project2");
        create_file(&p2, ".claude/skills/pruner/SKILL.md");

        let traces = scan_for_traces(dir.path());
        assert_eq!(traces.len(), 2);

        let projects: Vec<_> = traces.iter().map(|t| &t.project).collect();
        assert!(projects.contains(&&p1));
        assert!(projects.contains(&&p2));
    }

    #[test]
    fn test_scan_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let traces = scan_for_traces(dir.path());
        assert!(traces.is_empty());
    }

    #[test]
    fn test_found_trace_remove_deletes_dir() {
        let dir = tempfile::tempdir().unwrap();
        let pruner_dir = dir.path().join("project/.pruner");
        fs::create_dir_all(&pruner_dir).unwrap();
        fs::write(pruner_dir.join("index.db"), "fake").unwrap();

        let trace = FoundTrace {
            kind: TraceKind::PrunerDir,
            path: pruner_dir.clone(),
            project: dir.path().join("project"),
        };
        trace.remove();

        assert!(!pruner_dir.exists(), ".pruner/ should be removed");
    }

    #[test]
    fn test_found_trace_remove_cleans_section() {
        let dir = tempfile::tempdir().unwrap();
        let claude_md = dir.path().join("CLAUDE.md");
        fs::write(
            &claude_md,
            "# Project\n\n## Pruner\n\nStuff.\n\n## Other\n\nKeep.\n",
        )
        .unwrap();

        let trace = FoundTrace {
            kind: TraceKind::PrunerSection,
            path: claude_md.clone(),
            project: dir.path().to_path_buf(),
        };
        trace.remove();

        let content = fs::read_to_string(&claude_md).unwrap();
        assert!(!content.contains("Pruner"));
        assert!(content.contains("## Other"));
    }

    #[test]
    fn test_scan_finds_copilot_instructions_section() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("myproject");
        fs::create_dir_all(project.join(".github")).unwrap();
        fs::write(
            project.join(".github/copilot-instructions.md"),
            "# Instructions\n\n## Pruner\n\nAuto.\n",
        )
        .unwrap();

        let traces = scan_for_traces(dir.path());
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].kind, TraceKind::PrunerSection);
        assert_eq!(traces[0].project, project);
    }

    #[test]
    fn test_remove_pruner_section_from_markdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        fs::write(
            &path,
            "# My Project\n\nSome content.\n\n## Pruner\n\nPruner stuff here.\n\n## Other\n\nOther stuff.\n",
        )
        .unwrap();

        assert!(remove_pruner_section(&path));

        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("Pruner"));
        assert!(content.contains("# My Project"));
        assert!(content.contains("## Other"));
    }

    #[test]
    fn test_remove_pruner_section_only_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        fs::write(&path, "## Pruner\n\nPruner stuff only.\n").unwrap();

        assert!(remove_pruner_section(&path));
        assert!(
            !path.exists(),
            "File should be deleted when only pruner section"
        );
    }

    #[test]
    fn test_remove_pruner_section_not_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        fs::write(&path, "# My Project\n\nNo pruner here.\n").unwrap();

        assert!(!remove_pruner_section(&path));
    }

    #[test]
    fn test_clean_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        fs::write(&path, "node_modules/\n.pruner/\ntarget/\n").unwrap();

        clean_gitignore(&path);

        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains(".pruner"));
        assert!(content.contains("node_modules/"));
        assert!(content.contains("target/"));
    }

    #[test]
    fn test_clean_settings_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "matcher": "",
                    "hooks": [{
                        "type": "command",
                        "command": "/home/user/.claude/hooks/pruner-context.sh",
                        "timeout": 60
                    }]
                }]
            },
            "other_setting": true
        });
        fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();

        clean_settings_json(&path);

        let content = fs::read_to_string(&path).unwrap();
        let result: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(result.get("hooks").is_none(), "hooks should be removed");
        assert_eq!(
            result["other_setting"], true,
            "other settings should remain"
        );
    }

    // --- FoundTrace::remove() for remaining variants ---

    #[test]
    fn test_found_trace_remove_hook_file() {
        let dir = tempfile::tempdir().unwrap();
        let hook = dir.path().join(".claude/hooks/pruner-context.sh");
        fs::create_dir_all(hook.parent().unwrap()).unwrap();
        fs::write(&hook, "#!/bin/bash\npruner context").unwrap();

        let trace = FoundTrace {
            kind: TraceKind::HookFile,
            path: hook.clone(),
            project: dir.path().to_path_buf(),
        };
        trace.remove();
        assert!(!hook.exists(), "hook file should be removed");
    }

    #[test]
    fn test_found_trace_remove_settings_hook() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [{
                        "command": "pruner-context.sh"
                    }]
                }]
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();

        let trace = FoundTrace {
            kind: TraceKind::SettingsHook,
            path: path.clone(),
            project: dir.path().to_path_buf(),
        };
        trace.remove();
        // File should be removed entirely since it only had pruner hooks
        assert!(
            !path.exists(),
            "settings.json should be removed when pruner-only"
        );
    }

    #[test]
    fn test_found_trace_remove_gitignore_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        fs::write(&path, "node_modules/\n.pruner/\ntarget/\n").unwrap();

        let trace = FoundTrace {
            kind: TraceKind::GitignoreEntry,
            path: path.clone(),
            project: dir.path().to_path_buf(),
        };
        trace.remove();

        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains(".pruner"));
        assert!(content.contains("node_modules/"));
    }

    #[test]
    fn test_found_trace_remove_copilot_skill_dir() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join(".copilot/skills/pruner");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "skill content").unwrap();

        let trace = FoundTrace {
            kind: TraceKind::CopilotSkillDir,
            path: skill_dir.clone(),
            project: dir.path().to_path_buf(),
        };
        trace.remove();
        assert!(!skill_dir.exists(), "copilot skill dir should be removed");
    }

    // --- Display impls ---

    #[test]
    fn test_trace_kind_display() {
        assert_eq!(TraceKind::PrunerDir.to_string(), ".pruner/ index");
        assert_eq!(TraceKind::ClaudeSkillDir.to_string(), "Claude skill");
        assert_eq!(TraceKind::CopilotSkillDir.to_string(), "Copilot skill");
        assert_eq!(TraceKind::PrunerSection.to_string(), "pruner section");
        assert_eq!(TraceKind::SettingsHook.to_string(), "settings hook");
        assert_eq!(TraceKind::HookFile.to_string(), "hook file");
        assert_eq!(TraceKind::GitignoreEntry.to_string(), ".gitignore entry");
    }

    #[test]
    fn test_found_trace_display() {
        let trace = FoundTrace {
            kind: TraceKind::PrunerDir,
            path: PathBuf::from("/home/user/project/.pruner"),
            project: PathBuf::from("/home/user/project"),
        };
        let display = trace.to_string();
        assert!(display.contains(".pruner"));
        assert!(display.contains(".pruner/ index"));
    }

    // --- remove_file / remove_dir ---

    #[test]
    fn test_remove_file_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "data").unwrap();
        assert!(path.exists());
        remove_file(&path);
        assert!(!path.exists());
    }

    #[test]
    fn test_remove_file_nonexistent() {
        // Should not panic
        remove_file(Path::new("/tmp/pruner-nonexistent-file-test"));
    }

    #[test]
    fn test_remove_dir_existing() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("file.txt"), "data").unwrap();
        assert!(sub.exists());
        remove_dir(&sub);
        assert!(!sub.exists());
    }

    #[test]
    fn test_remove_dir_nonexistent() {
        // Should not panic
        remove_dir(Path::new("/tmp/pruner-nonexistent-dir-test"));
    }

    // --- clean_gitignore edge cases ---

    #[test]
    fn test_clean_gitignore_pruner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        fs::write(&path, ".pruner/\n").unwrap();
        clean_gitignore(&path);
        assert!(
            !path.exists(),
            "gitignore should be deleted when pruner-only"
        );
    }

    #[test]
    fn test_clean_gitignore_no_pruner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        fs::write(&path, "node_modules/\ntarget/\n").unwrap();
        clean_gitignore(&path);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("node_modules/"));
        assert!(content.contains("target/"));
    }

    #[test]
    fn test_clean_gitignore_nonexistent() {
        // Should not panic
        clean_gitignore(Path::new("/tmp/pruner-nonexistent-gitignore"));
    }

    // --- clean_settings_json edge cases ---

    #[test]
    fn test_clean_settings_json_pruner_only_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [{
                        "command": "pruner-context.sh"
                    }]
                }]
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();
        clean_settings_json(&path);
        assert!(
            !path.exists(),
            "settings.json should be removed when pruner-only"
        );
    }

    #[test]
    fn test_clean_settings_json_no_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"other": true}"#).unwrap();
        clean_settings_json(&path);
        // File should be unchanged
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("other"));
    }

    #[test]
    fn test_clean_settings_json_nonexistent() {
        // Should not panic
        clean_settings_json(Path::new("/tmp/pruner-nonexistent-settings"));
    }

    // --- infer_project ---

    #[test]
    fn test_infer_project_claude_dir() {
        let path = PathBuf::from("/home/user/project/.claude/skills/pruner");
        assert_eq!(infer_project(&path), PathBuf::from("/home/user/project"));
    }

    #[test]
    fn test_infer_project_pruner_dir() {
        let path = PathBuf::from("/home/user/project/.pruner");
        assert_eq!(infer_project(&path), PathBuf::from("/home/user/project"));
    }

    #[test]
    fn test_infer_project_github_dir() {
        let path = PathBuf::from("/home/user/project/.github/hooks/pruner-context.sh");
        assert_eq!(infer_project(&path), PathBuf::from("/home/user/project"));
    }

    #[test]
    fn test_infer_project_no_marker() {
        let path = PathBuf::from("/home/user/random/file.txt");
        assert_eq!(infer_project(&path), PathBuf::from("/home/user/random"));
    }

    // --- uninstall orchestrators ---

    #[test]
    fn test_uninstall_claude_project() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("project");
        // Set up traces
        create_file(&repo, ".claude/hooks/pruner-context.sh");
        create_file(&repo, ".claude/skills/pruner/SKILL.md");
        fs::write(repo.join("CLAUDE.md"), "# Project\n\n## Pruner\n\nStuff.\n").unwrap();

        uninstall_claude_project(&repo);

        assert!(!repo.join(".claude/hooks/pruner-context.sh").exists());
        assert!(!repo.join(".claude/skills/pruner").exists());
        // CLAUDE.md should be removed (was pruner-only content after project heading)
        // or cleaned
    }

    #[test]
    fn test_uninstall_copilot_project() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("project");
        create_file(&repo, ".copilot/skills/pruner/SKILL.md");
        create_file(&repo, ".github/hooks/pruner-context.json");
        create_file(&repo, ".github/hooks/pruner-context.sh");
        create_file(&repo, ".github/hooks/pruner-context.ps1");

        uninstall_copilot_project(&repo);

        assert!(!repo.join(".copilot/skills/pruner").exists());
        assert!(!repo.join(".github/hooks/pruner-context.json").exists());
        assert!(!repo.join(".github/hooks/pruner-context.sh").exists());
        assert!(!repo.join(".github/hooks/pruner-context.ps1").exists());
    }

    // --- has_pruner_hook_entry ---

    #[test]
    fn test_has_pruner_hook_entry_true() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [{ "command": "pruner-context.sh" }]
                }]
            }
        });
        fs::write(&path, serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(has_pruner_hook_entry(&path));
    }

    #[test]
    fn test_has_pruner_hook_entry_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [{ "command": "other-tool.sh" }]
                }]
            }
        });
        fs::write(&path, serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(!has_pruner_hook_entry(&path));
    }

    #[test]
    fn test_has_pruner_hook_entry_nonexistent() {
        assert!(!has_pruner_hook_entry(Path::new("/tmp/nonexistent")));
    }

    #[test]
    fn test_has_pruner_hook_entry_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "not json").unwrap();
        assert!(!has_pruner_hook_entry(&path));
    }

    // --- match_dir_trace / match_file_trace ---

    #[test]
    fn test_match_dir_trace_pruner() {
        let path = Path::new("/project/.pruner");
        assert_eq!(match_dir_trace(path, ".pruner"), Some(TraceKind::PrunerDir));
    }

    #[test]
    fn test_match_dir_trace_claude_skill() {
        let path = Path::new("/project/.claude/skills/pruner");
        assert_eq!(
            match_dir_trace(path, "pruner"),
            Some(TraceKind::ClaudeSkillDir)
        );
    }

    #[test]
    fn test_match_dir_trace_not_in_skills() {
        let path = Path::new("/project/.claude/other/pruner");
        assert_eq!(match_dir_trace(path, "pruner"), None);
    }

    #[test]
    fn test_match_dir_trace_unknown() {
        let path = Path::new("/project/random");
        assert_eq!(match_dir_trace(path, "random"), None);
    }

    #[test]
    fn test_match_file_trace_hook_in_integration_dir() {
        let path = Path::new("/project/.claude/hooks/pruner-context.sh");
        assert_eq!(
            match_file_trace(path, "pruner-context.sh"),
            Some(TraceKind::HookFile)
        );
    }

    #[test]
    fn test_match_file_trace_hook_outside_integration_dir() {
        let path = Path::new("/project/scripts/pruner-context.sh");
        assert_eq!(match_file_trace(path, "pruner-context.sh"), None);
    }

    #[test]
    fn test_match_file_trace_unknown_file() {
        let path = Path::new("/project/random.txt");
        assert_eq!(match_file_trace(path, "random.txt"), None);
    }

    #[test]
    fn test_clean_settings_json_mixed_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "matcher": "",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "/home/user/.claude/hooks/pruner-context.sh",
                            "timeout": 60
                        },
                        {
                            "type": "command",
                            "command": "/home/user/.claude/hooks/other-tool.sh",
                            "timeout": 30
                        }
                    ]
                }]
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();

        clean_settings_json(&path);

        let content = fs::read_to_string(&path).unwrap();
        let result: serde_json::Value = serde_json::from_str(&content).unwrap();
        // The entry should still exist with just the non-pruner hook
        let hooks = result["hooks"]["UserPromptSubmit"][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(hooks.len(), 1);
        assert!(hooks[0]["command"].as_str().unwrap().contains("other-tool"));
    }

    #[test]
    fn test_clean_codex_hooks_json_pruner_only_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        let hooks = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [{
                        "type": "command",
                        "command": "bash /home/u/.codex/hooks/pruner-context.sh"
                    }]
                }]
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&hooks).unwrap()).unwrap();

        clean_codex_hooks_json(&path);

        assert!(
            !path.exists(),
            "pruner-only hooks.json should be removed entirely"
        );
    }

    #[test]
    fn test_clean_codex_hooks_json_preserves_sibling_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        let hooks = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [
                        {
                            "type": "command",
                            "command": "bash /home/u/.codex/hooks/pruner-context.sh"
                        },
                        {
                            "type": "command",
                            "command": "bash /home/u/.codex/hooks/other.sh"
                        }
                    ]
                }]
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&hooks).unwrap()).unwrap();

        clean_codex_hooks_json(&path);

        let content = fs::read_to_string(&path).unwrap();
        let result: serde_json::Value = serde_json::from_str(&content).unwrap();
        let remaining = result["hooks"]["UserPromptSubmit"][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(
            remaining[0]["command"]
                .as_str()
                .unwrap()
                .contains("other.sh")
        );
    }

    #[test]
    fn test_clean_codex_config_toml_pruner_only_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[features]\ncodex_hooks = true\n").unwrap();

        clean_codex_config_toml(&path);

        assert!(
            !path.exists(),
            "pruner-only config.toml should be removed entirely"
        );
    }

    #[test]
    fn test_clean_codex_config_toml_preserves_other_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "model = \"gpt-5\"\n\n[features]\ncodex_hooks = true\nother_flag = true\n",
        )
        .unwrap();

        clean_codex_config_toml(&path);

        let content = fs::read_to_string(&path).unwrap();
        let parsed: toml::Value = content.parse().unwrap();
        assert_eq!(parsed["model"].as_str(), Some("gpt-5"));
        assert_eq!(parsed["features"]["other_flag"].as_bool(), Some(true));
        assert!(parsed["features"].get("codex_hooks").is_none());
    }
}
