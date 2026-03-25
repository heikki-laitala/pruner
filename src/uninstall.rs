//! Uninstall: remove pruner integrations (hooks, skills, config sections) and optionally the binary.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

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
        if let Some(submit) = hooks.get_mut("UserPromptSubmit") {
            if let Some(arr) = submit.as_array_mut() {
                let before = arr.len();
                arr.retain(|entry| {
                    // Remove entries whose hooks contain "pruner" in the command
                    let dominated_by_pruner = entry
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .is_some_and(|hooks| {
                            hooks.iter().all(|h| {
                                h.get("command")
                                    .and_then(|c| c.as_str())
                                    .is_some_and(|c| c.contains("pruner"))
                            })
                        });
                    !dominated_by_pruner
                });
                if arr.len() < before {
                    changed = true;
                }
                // If UserPromptSubmit is now empty, remove it
                if arr.is_empty() {
                    hooks.as_object_mut().map(|m| m.remove("UserPromptSubmit"));
                }
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

/// Main uninstall entrypoint.
pub fn cmd_uninstall(repo: Option<&Path>, purge: bool) -> Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;

    if let Some(repo) = repo {
        // Per-project uninstall
        println!("Removing pruner from {}...", repo.display());

        uninstall_claude_project(repo);
        uninstall_copilot_project(repo);
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

        // Remove the binary
        let exe = std::env::current_exe().context("Cannot determine binary path")?;
        println!("\nBinary: {}", exe.display());

        // On Windows, self-delete isn't straightforward. Use self_replace to swap
        // with an empty file, then schedule cleanup. On Unix, just remove.
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
            // On Windows we can't delete a running exe. Print instructions.
            println!("  Cannot delete a running executable on Windows.");
            println!("  Remove it manually: del \"{}\"", exe.display());
        }

        if purge {
            println!("\nNote: --purge only removes .pruner/ in per-project mode.");
            println!("To remove index data from repos, run: pruner uninstall <repo> --purge");
        }

        println!("\nDone. Global pruner integrations removed.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
}
