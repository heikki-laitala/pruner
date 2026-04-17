//! Integration tests using fixture repositories.
//!
//! Each fixture is a small realistic multi-file project that tests
//! indexing correctness, query relevance, and context quality.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

/// Helper: copy a fixture into a temp dir so we don't pollute the repo with .pruner/ dirs.
fn setup_fixture(fixture_name: &str) -> TempDir {
    let fixture_src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture_name);
    let tmp = TempDir::new().unwrap();
    copy_dir_all(&fixture_src, tmp.path()).unwrap();
    tmp
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest = dst.join(entry.file_name());
        if ty.is_dir() {
            std::fs::create_dir_all(&dest)?;
            copy_dir_all(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

fn pruner() -> Command {
    let mut cmd = Command::cargo_bin("pruner").unwrap();
    // Disable freshness cache so incremental tests can detect changes immediately
    cmd.env("PRUNER_RECHECK_SECS", "0");
    cmd
}

/// Helper: index a fixture dir, return the path string.
fn index_fixture(dir: &TempDir) -> String {
    let path = dir.path().to_str().unwrap().to_string();
    pruner().args(["index", &path]).assert().success();
    path
}

/// Helper: get context JSON for a query.
fn context_json(path: &str, query: &str) -> serde_json::Value {
    let output = pruner()
        .args(["context", path, query, "--format", "json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"))
}

/// Helper: get context JSON with --full mode (forces snippets regardless of auto-detect).
fn context_json_full(path: &str, query: &str) -> serde_json::Value {
    let output = pruner()
        .args(["context", path, query, "--format", "json", "--full"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"))
}

mod init {
    use super::*;

    #[test]
    fn init_copilot_skill_creates_skill_and_instructions_only() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        pruner()
            .args(["init", path, "--copilot-skill"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Installed Copilot skill"));

        assert!(
            dir.path().join(".copilot/skills/pruner/SKILL.md").exists(),
            "should create Copilot skill file"
        );
        assert!(
            dir.path().join(".github/copilot-instructions.md").exists(),
            "should create repo Copilot instructions"
        );
        assert!(
            !dir.path().join(".claude/skills/pruner/SKILL.md").exists(),
            "Copilot-only init should not create Claude skill file"
        );
        assert!(
            dir.path().join(".pruner/index.db").exists(),
            "init should auto-run indexing"
        );
    }

    #[test]
    fn init_copilot_hook_creates_hook_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        pruner()
            .args(["init", path, "--copilot-hook"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Installed Copilot hook config"));

        assert!(
            dir.path()
                .join(".github/hooks/pruner-context.json")
                .exists(),
            "should create Copilot hook config"
        );
        assert!(
            dir.path().join(".github/hooks/pruner-context.sh").exists(),
            "should create Copilot hook bash script"
        );
        assert!(
            dir.path().join(".github/hooks/pruner-context.ps1").exists(),
            "should create Copilot hook powershell script"
        );
        assert!(
            dir.path().join(".pruner/index.db").exists(),
            "init should auto-run indexing"
        );
    }
}

mod uninstall {
    use super::*;

    #[test]
    fn uninstall_removes_project_claude_integration() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        // Init with hook
        pruner().args(["init", path, "--hook"]).assert().success();

        // Verify files exist
        assert!(dir.path().join(".claude/skills/pruner/SKILL.md").exists());
        assert!(dir.path().join(".claude/hooks/pruner-context.sh").exists());
        assert!(dir.path().join(".claude/settings.json").exists());
        assert!(dir.path().join("CLAUDE.md").exists());
        assert!(dir.path().join(".pruner/index.db").exists());

        // Uninstall
        pruner()
            .args(["uninstall", path])
            .assert()
            .success()
            .stdout(predicate::str::contains("Removed"));

        // Verify pruner files are gone
        assert!(!dir.path().join(".claude/skills/pruner/SKILL.md").exists());
        assert!(!dir.path().join(".claude/hooks/pruner-context.sh").exists());

        // .pruner/ should still exist without --purge
        assert!(dir.path().join(".pruner").exists());
    }

    #[test]
    fn uninstall_purge_removes_index() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        pruner().args(["init", path, "--hook"]).assert().success();

        assert!(dir.path().join(".pruner/index.db").exists());

        pruner()
            .args(["uninstall", path, "--purge"])
            .assert()
            .success();

        assert!(!dir.path().join(".pruner").exists());
    }

    #[test]
    fn uninstall_removes_copilot_project_integration() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        pruner()
            .args(["init", path, "--copilot-hook"])
            .assert()
            .success();

        assert!(
            dir.path()
                .join(".github/hooks/pruner-context.json")
                .exists()
        );
        assert!(dir.path().join(".github/hooks/pruner-context.sh").exists());

        pruner().args(["uninstall", path]).assert().success();

        assert!(
            !dir.path()
                .join(".github/hooks/pruner-context.json")
                .exists()
        );
        assert!(!dir.path().join(".github/hooks/pruner-context.sh").exists());
    }

    #[test]
    fn uninstall_help_shows_options() {
        pruner()
            .args(["uninstall", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--purge"));
    }
}

// ============================================================================
// Status
// ============================================================================

mod status {
    use super::*;

    #[test]
    fn status_shows_version() {
        pruner()
            .args(["status"])
            .assert()
            .success()
            .stdout(predicate::str::contains("pruner v"));
    }

    #[test]
    fn status_shows_global_section() {
        pruner()
            .args(["status"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Global integrations:"));
    }

    #[test]
    fn status_with_repo_shows_project() {
        let dir = setup_fixture("python_webapp");
        let path = dir.path().to_str().unwrap();

        // Init first
        pruner().args(["init", path, "--hook"]).assert().success();

        let output = pruner().args(["status", path]).assert().success();

        output
            .stdout(predicate::str::contains("Claude Code:"))
            .stdout(predicate::str::contains("Index:"))
            .stdout(predicate::str::contains(".gitignore:"));
    }

    #[test]
    fn status_shows_not_installed_for_empty_project() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        pruner()
            .args(["status", path])
            .assert()
            .success()
            .stdout(predicate::str::contains("not installed"))
            .stdout(predicate::str::contains("not found"));
    }
}

// ============================================================================
// Unsupported repos
// ============================================================================

mod unsupported {
    use super::*;

    #[test]
    fn index_reports_zero_symbols_for_unsupported_repo() {
        let dir = TempDir::new().unwrap();
        // Create a markdown-only repo
        std::fs::write(dir.path().join("README.md"), "# Hello").unwrap();

        pruner()
            .args(["index", dir.path().to_str().unwrap()])
            .assert()
            .success()
            .stdout(predicate::str::contains("0 symbols"));
    }

    #[test]
    fn context_skips_repo_with_no_supported_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "# Hello").unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();

        let output = pruner()
            .args(["context", dir.path().to_str().unwrap(), "hello"])
            .output()
            .unwrap();

        // Should fail gracefully (non-zero exit or empty output)
        assert!(
            !output.status.success() || output.stdout.is_empty(),
            "should not produce context for unsupported repo"
        );
        assert!(
            !dir.path().join(".pruner").exists(),
            ".pruner/ should not remain for unsupported repos"
        );
    }
}

// ============================================================================
// Interactive-use restrictiveness: skip injection when pruner has no match
// ============================================================================

mod restrictive_injection {
    use super::*;

    #[test]
    fn junk_prompt_with_no_matches_emits_empty_stdout() {
        // A prompt that has no chance of matching any indexed symbol must not
        // inject the authority header + empty scaffolding. Hooks read stdout
        // directly, so empty stdout == no injection.
        let dir = setup_fixture("ts_package");
        let path = index_fixture(&dir);

        let output = pruner()
            .args(["context", &path, "thanks"])
            .output()
            .unwrap();

        assert!(output.status.success(), "pruner should exit 0");
        assert!(
            output.stdout.is_empty(),
            "stdout must be empty when no files/symbols/paths match, got:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[test]
    fn real_prompt_still_produces_output() {
        // Sanity check: the empty-match bail must not swallow legit queries.
        let dir = setup_fixture("ts_package");
        let path = index_fixture(&dir);

        let output = pruner()
            .args(["context", &path, "handleLogin"])
            .output()
            .unwrap();

        assert!(output.status.success());
        assert!(
            !output.stdout.is_empty(),
            "legit query must still produce output"
        );
    }
}

// ============================================================================
// Multi-repo (meta-repo pattern)
// ============================================================================

mod multi_repo {
    use super::*;

    /// Create a fake .git dir so pruner detects this as a git sub-repo.
    fn make_git_dir(path: &Path) {
        std::fs::create_dir_all(path.join(".git")).unwrap();
    }

    #[test]
    fn index_discovers_subrepos() {
        // Meta-repo with two git sub-repos, neither indexed yet
        let meta = TempDir::new().unwrap();

        let sub1 = meta.path().join("webapp");
        let fixture1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_webapp");
        std::fs::create_dir_all(&sub1).unwrap();
        copy_dir_all(&fixture1, &sub1).unwrap();
        make_git_dir(&sub1);

        let sub2 = meta.path().join("backend");
        let fixture2 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_crate");
        std::fs::create_dir_all(&sub2).unwrap();
        copy_dir_all(&fixture2, &sub2).unwrap();
        make_git_dir(&sub2);

        // Index from meta-repo level — should index each sub-repo separately
        let output = pruner()
            .args(["index", meta.path().to_str().unwrap()])
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "index should succeed: {stderr}");
        assert!(
            stderr.contains("Meta-repo detected"),
            "should detect meta-repo, got: {stderr}"
        );

        // Each sub-repo should have its own index
        assert!(
            sub1.join(".pruner/index.db").exists(),
            "webapp should be indexed"
        );
        assert!(
            sub2.join(".pruner/index.db").exists(),
            "backend should be indexed"
        );
        // Parent should NOT have an index
        assert!(
            !meta.path().join(".pruner/index.db").exists(),
            "meta-repo should not have flat index"
        );
    }

    #[test]
    fn context_discovers_subrepos() {
        // Meta-repo with two git sub-repos, pre-indexed
        let meta = TempDir::new().unwrap();

        let sub1 = meta.path().join("webapp");
        let fixture1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_webapp");
        std::fs::create_dir_all(&sub1).unwrap();
        copy_dir_all(&fixture1, &sub1).unwrap();
        make_git_dir(&sub1);
        pruner()
            .args(["index", sub1.to_str().unwrap()])
            .assert()
            .success();

        let sub2 = meta.path().join("backend");
        let fixture2 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_crate");
        std::fs::create_dir_all(&sub2).unwrap();
        copy_dir_all(&fixture2, &sub2).unwrap();
        make_git_dir(&sub2);
        pruner()
            .args(["index", sub2.to_str().unwrap()])
            .assert()
            .success();

        // Run context from meta-repo level
        let output = pruner()
            .args([
                "context",
                meta.path().to_str().unwrap(),
                "login authentication",
            ])
            .output()
            .unwrap();

        assert!(output.status.success(), "context should succeed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            stderr.contains("Multi-repo mode"),
            "should report multi-repo mode, got stderr: {stderr}"
        );
        assert!(
            stdout.contains("# Repo:"),
            "should have repo header in output, got: {stdout}"
        );
    }

    #[test]
    fn context_auto_indexes_unindexed_subrepos() {
        let meta = TempDir::new().unwrap();

        // Sub-repo with .git but NOT pre-indexed
        let sub1 = meta.path().join("webapp");
        let fixture1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_webapp");
        std::fs::create_dir_all(&sub1).unwrap();
        copy_dir_all(&fixture1, &sub1).unwrap();
        make_git_dir(&sub1);

        assert!(
            !sub1.join(".pruner/index.db").exists(),
            "should not be indexed yet"
        );

        // Context should auto-index the sub-repo and return results
        let output = pruner()
            .args(["context", meta.path().to_str().unwrap(), "login"])
            .output()
            .unwrap();

        assert!(output.status.success(), "context should succeed");
        assert!(
            sub1.join(".pruner/index.db").exists(),
            "sub-repo should be auto-indexed"
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("# Repo:"),
            "should have repo header, got: {stdout}"
        );
    }

    #[test]
    fn context_multi_repo_ranks_by_relevance() {
        // Meta-repo with two sub-repos: webapp has auth symbols, rust_crate does not.
        // Query for "authenticate" should put webapp first.
        let meta = TempDir::new().unwrap();

        // Sub-repo that should score HIGH for "authenticate"
        let sub_webapp = meta.path().join("webapp");
        let fixture1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_webapp");
        std::fs::create_dir_all(&sub_webapp).unwrap();
        copy_dir_all(&fixture1, &sub_webapp).unwrap();
        make_git_dir(&sub_webapp);
        pruner()
            .args(["index", sub_webapp.to_str().unwrap()])
            .assert()
            .success();

        // Sub-repo that should score LOW for "authenticate"
        let sub_rust = meta.path().join("rust_crate");
        let fixture2 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_crate");
        std::fs::create_dir_all(&sub_rust).unwrap();
        copy_dir_all(&fixture2, &sub_rust).unwrap();
        make_git_dir(&sub_rust);
        pruner()
            .args(["index", sub_rust.to_str().unwrap()])
            .assert()
            .success();

        let output = pruner()
            .args([
                "context",
                meta.path().to_str().unwrap(),
                "authenticate user login",
            ])
            .output()
            .unwrap();

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Verify scoring info is logged
        assert!(
            stderr.contains("Including webapp"),
            "webapp should be included, got stderr: {stderr}"
        );

        // Multi-repo header should be present
        assert!(
            stdout.contains("**Multi-repo context:**"),
            "should have multi-repo header, got: {stdout}"
        );

        // webapp should appear first in output (highest score)
        let webapp_pos = stdout.find("# Repo: webapp");
        assert!(
            webapp_pos.is_some(),
            "webapp should appear in output, got: {stdout}"
        );

        // If rust_crate appears, it should be after webapp
        if let Some(rust_pos) = stdout.find("# Repo: rust_crate") {
            assert!(
                webapp_pos.unwrap() < rust_pos,
                "webapp should appear before rust_crate (higher relevance)"
            );
        }
    }

    #[test]
    fn context_skips_non_git_dirs() {
        let meta = TempDir::new().unwrap();

        // One real sub-repo with .git
        let sub1 = meta.path().join("webapp");
        let fixture1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_webapp");
        std::fs::create_dir_all(&sub1).unwrap();
        copy_dir_all(&fixture1, &sub1).unwrap();
        make_git_dir(&sub1);
        pruner()
            .args(["index", sub1.to_str().unwrap()])
            .assert()
            .success();

        // Empty dir without .git — should be ignored
        std::fs::create_dir_all(meta.path().join("docs")).unwrap();

        let output = pruner()
            .args(["context", meta.path().to_str().unwrap(), "login"])
            .output()
            .unwrap();

        assert!(output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("1 sub-repos"),
            "should find only 1 sub-repo, got: {stderr}"
        );
    }

    #[test]
    fn index_includes_root_by_default() {
        // Meta-repo with a sub-repo and root-level files
        let meta = TempDir::new().unwrap();

        // Sub-repo
        let sub1 = meta.path().join("webapp");
        let fixture1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_webapp");
        std::fs::create_dir_all(&sub1).unwrap();
        copy_dir_all(&fixture1, &sub1).unwrap();
        make_git_dir(&sub1);

        // Root-level file (not in any sub-repo)
        std::fs::write(
            meta.path().join("shared.py"),
            "def shared_helper():\n    return 42\n",
        )
        .unwrap();

        // Index from meta-repo level — should index root AND sub-repos
        let output = pruner()
            .args(["index", meta.path().to_str().unwrap()])
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "index should succeed: {stderr}");

        // Sub-repo should have its own index
        assert!(
            sub1.join(".pruner/index.db").exists(),
            "webapp should be indexed"
        );
        // Root should ALSO have an index now
        assert!(
            meta.path().join(".pruner/index.db").exists(),
            "root should be indexed by default"
        );
    }

    #[test]
    fn index_no_root_skips_root() {
        // --no-root flag should preserve old behavior (skip root indexing)
        let meta = TempDir::new().unwrap();

        let sub1 = meta.path().join("webapp");
        let fixture1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_webapp");
        std::fs::create_dir_all(&sub1).unwrap();
        copy_dir_all(&fixture1, &sub1).unwrap();
        make_git_dir(&sub1);

        std::fs::write(
            meta.path().join("shared.py"),
            "def shared_helper():\n    return 42\n",
        )
        .unwrap();

        let output = pruner()
            .args(["index", "--no-root", meta.path().to_str().unwrap()])
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "index should succeed: {stderr}");

        assert!(
            sub1.join(".pruner/index.db").exists(),
            "webapp should be indexed"
        );
        assert!(
            !meta.path().join(".pruner/index.db").exists(),
            "root should NOT be indexed with --no-root"
        );
    }

    #[test]
    fn context_includes_root_files() {
        // Root-level files should appear in multi-repo context
        let meta = TempDir::new().unwrap();

        let sub1 = meta.path().join("webapp");
        let fixture1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_webapp");
        std::fs::create_dir_all(&sub1).unwrap();
        copy_dir_all(&fixture1, &sub1).unwrap();
        make_git_dir(&sub1);

        // Root-level file with a distinctive function name
        std::fs::write(
            meta.path().join("shared_utils.py"),
            "def compute_shared_metric():\n    return 99\n",
        )
        .unwrap();

        // Index everything (root + sub-repos)
        pruner()
            .args(["index", meta.path().to_str().unwrap()])
            .assert()
            .success();

        // Query for the root-level function
        let output = pruner()
            .args([
                "context",
                meta.path().to_str().unwrap(),
                "compute shared metric",
            ])
            .output()
            .unwrap();

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Root files should be found
        assert!(
            stdout.contains("shared_utils.py"),
            "root-level file should appear in context, got: {stdout}"
        );
    }
}

// ============================================================================
// Hook scripts
// ============================================================================

#[cfg(not(target_os = "windows"))]
mod hooks {
    use super::*;
    use std::process::Command;

    /// Path to the pruner binary built by cargo
    fn pruner_bin() -> std::path::PathBuf {
        assert_cmd::cargo::cargo_bin("pruner")
    }

    /// Source path for a hook script
    fn hook_script(rel_path: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(rel_path)
    }

    /// Set up an indexed fixture and return the temp dir
    fn indexed_fixture() -> TempDir {
        let dir = setup_fixture("python_webapp");
        index_fixture(&dir);
        dir
    }

    /// Run a bash hook script with given JSON on stdin, pruner on PATH.
    /// Returns (exit_code, stdout, stderr).
    fn run_hook(
        script: &Path,
        stdin_json: &str,
        env_vars: &[(&str, &str)],
    ) -> (i32, String, String) {
        let bin = pruner_bin();
        let bin_dir = bin.parent().unwrap();
        let path_env = format!(
            "{}:{}",
            bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        run_hook_with_path(script, stdin_json, &path_env, env_vars)
    }

    /// Run a bash hook with a custom PATH (for testing missing binary).
    fn run_hook_with_path(
        script: &Path,
        stdin_json: &str,
        path: &str,
        env_vars: &[(&str, &str)],
    ) -> (i32, String, String) {
        let mut cmd = Command::new("bash");
        cmd.arg(script)
            .env("PATH", path)
            .env("PRUNER_RECHECK_SECS", "0")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        cmd.env_remove("USERPROFILE");

        for (key, val) in env_vars {
            cmd.env(key, val);
        }

        let mut child = cmd.spawn().expect("failed to spawn bash");
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin_json.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();

        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    }

    // -- Claude Code hook tests --

    #[test]
    fn claude_hook_produces_context_output() {
        let dir = indexed_fixture();
        let script = hook_script(".claude/hooks/pruner-context.sh");
        let json = r#"{"prompt": "login authentication"}"#;
        let env = [("CLAUDE_PROJECT_DIR", dir.path().to_str().unwrap())];

        let (code, stdout, _stderr) = run_hook(&script, json, &env);

        assert_eq!(code, 0);
        assert!(
            stdout.contains("Pruner context"),
            "should contain context header, got: {stdout}"
        );
        assert!(
            stdout.contains("Do not re-explore"),
            "should contain usage instruction"
        );
    }

    #[test]
    fn claude_hook_empty_prompt_exits_silently() {
        let script = hook_script(".claude/hooks/pruner-context.sh");
        let json = r#"{"prompt": ""}"#;

        let (code, stdout, _stderr) = run_hook(&script, json, &[]);

        assert_eq!(code, 0);
        assert!(
            stdout.is_empty(),
            "should produce no output for empty prompt"
        );
    }

    #[test]
    fn claude_hook_missing_prompt_exits_silently() {
        let script = hook_script(".claude/hooks/pruner-context.sh");
        let json = r#"{"other": "field"}"#;

        let (code, stdout, _stderr) = run_hook(&script, json, &[]);

        assert_eq!(code, 0);
        assert!(
            stdout.is_empty(),
            "should produce no output for missing prompt"
        );
    }

    #[test]
    fn claude_hook_no_binary_exits_silently() {
        let script = hook_script(".claude/hooks/pruner-context.sh");
        let json = r#"{"prompt": "test query"}"#;

        // Minimal PATH with only system dirs — no pruner binary
        let (code, stdout, _stderr) = run_hook_with_path(
            &script,
            json,
            "/usr/bin:/bin",
            &[
                ("HOME", "/nonexistent"),
                ("CLAUDE_PROJECT_DIR", "/nonexistent"),
            ],
        );

        assert_eq!(code, 0);
        assert!(
            stdout.is_empty(),
            "should produce no output when pruner not found"
        );
    }

    #[test]
    fn claude_hook_skips_non_repo_directory() {
        let dir = TempDir::new().unwrap(); // empty dir, no .git or .pruner
        let script = hook_script(".claude/hooks/pruner-context.sh");
        let json = r#"{"prompt": "test query"}"#;
        let env = [("CLAUDE_PROJECT_DIR", dir.path().to_str().unwrap())];

        let (code, stdout, _stderr) = run_hook(&script, json, &env);

        assert_eq!(code, 0);
        assert!(
            stdout.is_empty(),
            "should produce no output for non-repo directory"
        );
        assert!(
            !dir.path().join(".pruner").exists(),
            "should not create .pruner/ in non-repo directory"
        );
    }

    // -- Copilot hook tests --

    #[test]
    fn copilot_hook_writes_context_file() {
        let dir = indexed_fixture();
        let script = hook_script(".copilot/hooks/pruner-context.sh");
        let json = format!(
            r#"{{"prompt": "login authentication", "cwd": "{}"}}"#,
            dir.path().display()
        );

        let (code, stdout, _stderr) = run_hook(&script, &json, &[]);

        assert_eq!(code, 0);
        assert!(stdout.is_empty(), "copilot hook writes to file, not stdout");

        let context_file = dir.path().join(".pruner/copilot-context.md");
        assert!(context_file.exists(), "should create copilot-context.md");

        let content = std::fs::read_to_string(&context_file).unwrap();
        assert!(
            content.contains("Pruner context"),
            "context file should contain header, got: {content}"
        );
        assert!(
            content.contains("Do not re-explore"),
            "context file should contain usage instruction"
        );
    }

    #[test]
    fn copilot_hook_empty_prompt_exits_silently() {
        let dir = TempDir::new().unwrap();
        let script = hook_script(".copilot/hooks/pruner-context.sh");
        let json = format!(r#"{{"prompt": "", "cwd": "{}"}}"#, dir.path().display());

        let (code, stdout, _stderr) = run_hook(&script, &json, &[]);

        assert_eq!(code, 0);
        assert!(stdout.is_empty());
        assert!(
            !dir.path().join(".pruner/copilot-context.md").exists(),
            "should not create context file for empty prompt"
        );
    }
}

// ============================================================================
// Python webapp fixture
// ============================================================================

mod python_webapp {
    use super::*;

    #[test]
    fn index_finds_all_files() {
        let dir = setup_fixture("python_webapp");
        pruner()
            .args(["index", dir.path().to_str().unwrap(), "-v"])
            .assert()
            .success()
            .stdout(predicate::str::contains("4 files"));
    }

    #[test]
    fn index_detects_test_file() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        pruner()
            .args(["stats", &path])
            .assert()
            .success()
            .stdout(predicate::str::contains("Files:   4"));
    }

    #[test]
    fn context_authenticate_finds_service_function() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        let json = context_json(&path, "authenticate user");
        let symbols: Vec<&str> = json["key_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();

        assert!(
            symbols.contains(&"authenticate_user"),
            "should find authenticate_user in key_symbols, got: {symbols:?}"
        );
    }

    #[test]
    fn context_login_finds_handler() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        let json = context_json(&path, "login handler");
        let symbols: Vec<&str> = json["key_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();

        assert!(
            symbols.contains(&"login_handler"),
            "should find login_handler, got: {symbols:?}"
        );
    }

    #[test]
    fn context_user_profile_finds_service() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        let json = context_json(&path, "user profile");
        let symbols: Vec<&str> = json["key_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();

        assert!(
            symbols.contains(&"get_user_profile") || symbols.contains(&"profile_handler"),
            "should find profile-related symbols, got: {symbols:?}"
        );
    }

    #[test]
    fn context_authenticate_references_services_file() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        let json = context_json(&path, "authenticate_user");
        let all_text = serde_json::to_string(&json).unwrap();

        // authenticate_user is defined in services.py, should appear in symbols or snippets
        assert!(
            all_text.contains("services.py"),
            "context should reference services.py via symbols or snippets"
        );
    }

    #[test]
    fn context_login_has_execution_paths() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        let json = context_json(&path, "login_handler");
        let paths = json["execution_paths"].as_array().unwrap();

        assert!(
            !paths.is_empty(),
            "should have execution paths from login_handler"
        );
    }

    #[test]
    fn context_brief_writes_context_file() {
        let dir = setup_fixture("python_webapp");

        pruner()
            .args([
                "context",
                dir.path().to_str().unwrap(),
                "login flow",
                "--brief",
            ])
            .assert()
            .success();

        let context_file = dir.path().join(".pruner/context.md");
        assert!(
            context_file.exists(),
            "should write .pruner/context.md in brief mode"
        );
        let content = std::fs::read_to_string(&context_file).unwrap();
        assert!(
            content.contains("login"),
            "context.md should contain login-related content"
        );
    }

    #[test]
    fn show_file_displays_symbols() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-file", &path, "services.py"])
            .assert()
            .success()
            .stdout(predicate::str::contains("authenticate_user"))
            .stdout(predicate::str::contains("get_user_profile"));
    }

    #[test]
    fn show_symbol_finds_function() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "authenticate_user"])
            .assert()
            .success()
            .stdout(predicate::str::contains("function"))
            .stdout(predicate::str::contains("services.py"));
    }

    #[test]
    fn context_has_snippets_with_code() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        let json = context_json_full(&path, "authenticate_user");
        let snippets = json["snippets"].as_array().unwrap();

        assert!(!snippets.is_empty(), "should have code snippets");

        let has_auth_snippet = snippets
            .iter()
            .any(|s| s["symbol"].as_str().unwrap() == "authenticate_user");
        assert!(
            has_auth_snippet,
            "should have snippet for authenticate_user"
        );
    }

    #[test]
    fn test_edges_link_test_to_source() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        // Query by filename keyword so we get matching files (test edges resolve from files)
        let json = context_json(&path, "services");
        let tests = json["relevant_tests"].as_array().unwrap();

        assert!(
            !tests.is_empty(),
            "should find related test files when querying 'services'"
        );

        let test_paths: Vec<&str> = tests.iter().map(|t| t["path"].as_str().unwrap()).collect();
        assert!(
            test_paths.iter().any(|p| p.contains("test_services")),
            "should link test_services.py as related test, got: {test_paths:?}"
        );
    }
}

// ============================================================================
// TypeScript package fixture
// ============================================================================

mod ts_package {
    use super::*;

    #[test]
    fn index_finds_all_files() {
        let dir = setup_fixture("ts_package");
        pruner()
            .args(["index", dir.path().to_str().unwrap(), "-v"])
            .assert()
            .success()
            .stdout(predicate::str::contains("4 files"));
    }

    #[test]
    fn context_validate_token_finds_auth() {
        let dir = setup_fixture("ts_package");
        let path = index_fixture(&dir);

        let json = context_json(&path, "validateToken");
        let symbols: Vec<&str> = json["key_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();

        assert!(
            symbols.contains(&"validateToken"),
            "should find validateToken, got: {symbols:?}"
        );
    }

    #[test]
    fn context_handle_login_finds_handler() {
        let dir = setup_fixture("ts_package");
        let path = index_fixture(&dir);

        let json = context_json(&path, "handleLogin");
        let symbols: Vec<&str> = json["key_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();

        assert!(
            symbols.contains(&"handleLogin"),
            "should find handleLogin, got: {symbols:?}"
        );
    }

    #[test]
    fn context_session_finds_cross_file_symbols() {
        let dir = setup_fixture("ts_package");
        let path = index_fixture(&dir);

        let json = context_json(&path, "createSession");
        let symbols: Vec<&str> = json["key_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();

        assert!(
            symbols.contains(&"createSession") || symbols.contains(&"createSessionRecord"),
            "should find session creation symbols across files, got: {symbols:?}"
        );
    }

    #[test]
    fn context_handle_login_has_execution_paths() {
        let dir = setup_fixture("ts_package");
        let path = index_fixture(&dir);

        let json = context_json(&path, "handleLogin");
        let paths = json["execution_paths"].as_array().unwrap();

        assert!(
            !paths.is_empty(),
            "should have execution paths from handleLogin"
        );
    }

    #[test]
    fn context_includes_snippets() {
        let dir = setup_fixture("ts_package");
        let path = index_fixture(&dir);

        let json = context_json_full(&path, "handleLogin");
        let snippets = json["snippets"].as_array().unwrap();

        assert!(!snippets.is_empty(), "should include code snippets");
    }

    #[test]
    fn test_file_detected_as_test() {
        let dir = setup_fixture("ts_package");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-file", &path, "api/handler.test.ts"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Test: true"));
    }

    #[test]
    fn context_login_traces_to_db_layer() {
        let dir = setup_fixture("ts_package");
        let path = index_fixture(&dir);

        let json = context_json(&path, "handleLogin");
        let all_text = serde_json::to_string(&json).unwrap();

        // Execution paths or snippets should reference the db layer
        assert!(
            all_text.contains("findUser") || all_text.contains("db.ts"),
            "handleLogin context should trace to db layer"
        );
    }
}

// ============================================================================
// Rust crate fixture
// ============================================================================

mod rust_crate {
    use super::*;

    #[test]
    fn index_finds_all_files() {
        let dir = setup_fixture("rust_crate");
        pruner()
            .args(["index", dir.path().to_str().unwrap(), "-v"])
            .assert()
            .success()
            .stdout(predicate::str::contains("3 files"));
    }

    #[test]
    fn context_session_finds_db_function() {
        let dir = setup_fixture("rust_crate");
        let path = index_fixture(&dir);

        let json = context_json(&path, "create_session");
        let symbols: Vec<&str> = json["key_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();

        assert!(
            symbols.contains(&"create_session"),
            "should find create_session, got: {symbols:?}"
        );
    }

    #[test]
    fn context_server_finds_handler() {
        let dir = setup_fixture("rust_crate");
        let path = index_fixture(&dir);

        let json = context_json(&path, "handle_requests");
        let symbols: Vec<&str> = json["key_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();

        assert!(
            symbols.contains(&"handle_requests"),
            "should find handle_requests, got: {symbols:?}"
        );
    }

    #[test]
    fn context_handle_requests_traces_call_chain() {
        let dir = setup_fixture("rust_crate");
        let path = index_fixture(&dir);

        let json = context_json(&path, "handle_requests");
        let paths = json["execution_paths"].as_array().unwrap();

        assert!(
            !paths.is_empty(),
            "should trace execution paths from handle_requests"
        );

        // Path should include get_user or create_session
        let path_text = serde_json::to_string(&paths).unwrap();
        assert!(
            path_text.contains("get_user") || path_text.contains("create_session"),
            "execution path should trace to db functions, got: {path_text}"
        );
    }

    #[test]
    fn show_symbol_finds_struct() {
        let dir = setup_fixture("rust_crate");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "Pool"])
            .assert()
            .success()
            .stdout(predicate::str::contains("struct"));
    }

    #[test]
    fn context_broad_query_finds_multiple_symbols() {
        let dir = setup_fixture("rust_crate");
        let path = index_fixture(&dir);

        // A broad query should surface symbols from multiple files
        let json = context_json(&path, "start handle_requests create_pool get_user");
        let symbols: Vec<&str> = json["key_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();

        assert!(
            symbols.len() >= 2,
            "broad query should find symbols from multiple files, got: {symbols:?}"
        );
    }
}

// ============================================================================
// Incremental indexing tests
// ============================================================================

mod incremental {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn unchanged_repo_skips_reindex() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        // Second run should not show incremental update
        let output = pruner()
            .args(["context", &path, "login", "--brief"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !stderr.contains("Incremental update"),
            "should not re-index unchanged repo, got: {stderr}"
        );
    }

    #[test]
    fn modified_file_triggers_incremental() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        // Wait for mtime to change, then modify a file
        thread::sleep(Duration::from_secs(1));
        let services = dir.path().join("services.py");
        let content = std::fs::read_to_string(&services).unwrap();
        std::fs::write(&services, format!("{content}\ndef new_function(): pass\n")).unwrap();

        // Should detect the change
        let output = pruner()
            .args(["context", &path, "login", "--brief"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            stderr.contains("Incremental update") && stderr.contains("1 new/modified"),
            "should detect modified file, got: {stderr}"
        );
    }

    #[test]
    fn deleted_file_triggers_incremental() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        // Delete a file
        std::fs::remove_file(dir.path().join("models.py")).unwrap();

        // Should detect the deletion
        let output = pruner()
            .args(["context", &path, "login", "--brief"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            stderr.contains("Incremental update") && stderr.contains("1 deleted"),
            "should detect deleted file, got: {stderr}"
        );

        // Stats should show fewer files
        pruner()
            .args(["stats", &path])
            .assert()
            .success()
            .stdout(predicate::str::contains("Files:   3"));
    }

    #[test]
    fn new_file_triggers_incremental() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        // Add a new file
        std::fs::write(
            dir.path().join("utils.py"),
            "def format_response(data):\n    return str(data)\n",
        )
        .unwrap();

        // Should detect the new file
        let output = pruner()
            .args(["context", &path, "format", "--brief"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            stderr.contains("Incremental update") && stderr.contains("1 new/modified"),
            "should detect new file, got: {stderr}"
        );

        // Stats should show more files
        pruner()
            .args(["stats", &path])
            .assert()
            .success()
            .stdout(predicate::str::contains("Files:   5"));
    }
}

// ============================================================================
// ============================================================================
// Optional: real repo tests (run with PRUNER_TEST_REPO=/path/to/repo)
// ============================================================================

#[test]
fn real_repo_baseline() {
    let repo_path = match std::env::var("PRUNER_TEST_REPO") {
        Ok(p) => p,
        Err(_) => return, // Skip if not set
    };

    // Index should succeed
    pruner()
        .args(["index", &repo_path, "-v"])
        .assert()
        .success();

    // Should have reasonable counts
    let output = pruner().args(["stats", &repo_path]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse file count from "Files:   N"
    let file_count: i64 = stdout
        .lines()
        .find(|l| l.starts_with("Files:"))
        .and_then(|l| l.split_whitespace().last())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);

    assert!(
        file_count > 10,
        "real repo should have >10 files, got {file_count}"
    );

    // Query should find something
    let output = pruner()
        .args(["query", &repo_path, "handler request"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("Matching symbols:") || stdout.contains("Matching files:"),
        "query should produce results on real repo"
    );
}

// ============================================================================
// Go service fixture
// ============================================================================

mod go_service {
    use super::*;

    #[test]
    fn index_succeeds() {
        let dir = setup_fixture("go_service");
        let path = dir.path().to_str().unwrap().to_string();
        pruner().args(["index", &path]).assert().success();
    }

    #[test]
    fn finds_go_symbols() {
        let dir = setup_fixture("go_service");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "HandleLogin"])
            .assert()
            .success()
            .stdout(predicate::str::contains("method"))
            .stdout(predicate::str::contains("auth.go"));
    }

    #[test]
    fn finds_go_structs() {
        let dir = setup_fixture("go_service");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "AuthHandler"])
            .assert()
            .success()
            .stdout(predicate::str::contains("struct"));
    }

    #[test]
    fn query_finds_auth_symbols() {
        let dir = setup_fixture("go_service");
        let path = index_fixture(&dir);

        pruner()
            .args(["query", &path, "HandleLogin authentication"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Matching symbols: 3"));
    }

    #[test]
    fn context_includes_snippets() {
        let dir = setup_fixture("go_service");
        let path = index_fixture(&dir);

        let json = context_json_full(&path, "HandleLogin");
        let snippets = json["snippets"].as_array().unwrap();

        assert!(!snippets.is_empty(), "should include Go code snippets");
    }

    #[test]
    fn detects_go_test_files() {
        let dir = setup_fixture("go_service");
        let path = index_fixture(&dir);

        let json = context_json(&path, "HandleLogin");
        let tests = json["relevant_tests"].as_array().unwrap();

        assert!(
            tests
                .iter()
                .any(|t| { t["path"].as_str().unwrap_or("").contains("auth_test.go") }),
            "should detect Go test file"
        );
    }

    #[test]
    fn context_has_execution_paths() {
        let dir = setup_fixture("go_service");
        let path = index_fixture(&dir);

        let json = context_json_full(&path, "HandleLogin");
        let paths = json["execution_paths"].as_array().unwrap();

        assert!(
            !paths.is_empty(),
            "should have execution paths from HandleLogin"
        );
    }
}

// ============================================================================
// Java project fixture
// ============================================================================

mod java_project {
    use super::*;

    #[test]
    fn index_succeeds() {
        let dir = setup_fixture("java_project");
        let path = dir.path().to_str().unwrap().to_string();
        pruner().args(["index", &path]).assert().success();
    }

    #[test]
    fn finds_java_classes() {
        let dir = setup_fixture("java_project");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "AuthHandler"])
            .assert()
            .success()
            .stdout(predicate::str::contains("class"));
    }

    #[test]
    fn finds_java_methods() {
        let dir = setup_fixture("java_project");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "authenticate"])
            .assert()
            .success()
            .stdout(predicate::str::contains("method"))
            .stdout(predicate::str::contains("AuthHandler.java"));
    }

    #[test]
    fn query_finds_auth_symbols() {
        let dir = setup_fixture("java_project");
        let path = index_fixture(&dir);

        pruner()
            .args(["query", &path, "authenticate user"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Matching symbols:"));
    }

    #[test]
    fn context_includes_snippets() {
        let dir = setup_fixture("java_project");
        let path = index_fixture(&dir);

        let json = context_json_full(&path, "authenticate");
        let snippets = json["snippets"].as_array().unwrap();

        assert!(!snippets.is_empty(), "should include Java code snippets");
    }

    #[test]
    fn detects_java_test_files() {
        let dir = setup_fixture("java_project");
        let path = index_fixture(&dir);

        let json = context_json(&path, "authenticate");
        let tests = json["relevant_tests"].as_array().unwrap();

        assert!(
            tests.iter().any(|t| {
                t["path"]
                    .as_str()
                    .unwrap_or("")
                    .contains("AuthHandlerTest.java")
            }),
            "should detect Java test file"
        );
    }

    #[test]
    fn context_has_execution_paths() {
        let dir = setup_fixture("java_project");
        let path = index_fixture(&dir);

        let json = context_json_full(&path, "authenticate");
        let paths = json["execution_paths"].as_array().unwrap();

        assert!(
            !paths.is_empty(),
            "should have execution paths from authenticate"
        );
    }
}

mod c_project {
    use super::*;

    #[test]
    fn index_succeeds() {
        let dir = setup_fixture("c_project");
        let path = dir.path().to_str().unwrap().to_string();
        pruner().args(["index", &path]).assert().success();
    }

    #[test]
    fn finds_c_functions() {
        let dir = setup_fixture("c_project");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "authenticate"])
            .assert()
            .success()
            .stdout(predicate::str::contains("function"))
            .stdout(predicate::str::contains("auth.c"));
    }

    #[test]
    fn finds_c_structs() {
        let dir = setup_fixture("c_project");
        let path = index_fixture(&dir);

        // User is a typedef in user.h
        pruner()
            .args(["show-symbol", &path, "User"])
            .assert()
            .success()
            .stdout(predicate::str::contains("type"));
    }

    #[test]
    fn detects_c_test_files() {
        let dir = setup_fixture("c_project");
        let path = index_fixture(&dir);

        let json = context_json(&path, "authenticate");
        let tests = json["relevant_tests"].as_array().unwrap();

        assert!(
            tests
                .iter()
                .any(|t| t["path"].as_str().unwrap_or("").contains("test_auth.c")),
            "should detect C test file"
        );
    }

    #[test]
    fn context_has_execution_paths() {
        let dir = setup_fixture("c_project");
        let path = index_fixture(&dir);

        let json = context_json_full(&path, "authenticate");
        let paths = json["execution_paths"].as_array().unwrap();

        assert!(
            !paths.is_empty(),
            "should have execution paths from authenticate"
        );
    }
}

mod cpp_project {
    use super::*;

    #[test]
    fn index_succeeds() {
        let dir = setup_fixture("cpp_project");
        let path = dir.path().to_str().unwrap().to_string();
        pruner().args(["index", &path]).assert().success();
    }

    #[test]
    fn finds_cpp_classes() {
        let dir = setup_fixture("cpp_project");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "AuthService"])
            .assert()
            .success()
            .stdout(predicate::str::contains("class"));
    }

    #[test]
    fn finds_cpp_methods() {
        let dir = setup_fixture("cpp_project");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "authenticate"])
            .assert()
            .success()
            .stdout(predicate::str::contains("method"));
    }

    #[test]
    fn finds_cpp_namespaces() {
        let dir = setup_fixture("cpp_project");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "auth"])
            .assert()
            .success()
            .stdout(predicate::str::contains("namespace"));
    }

    #[test]
    fn context_includes_snippets() {
        let dir = setup_fixture("cpp_project");
        let path = index_fixture(&dir);

        let json = context_json_full(&path, "authenticate");
        let snippets = json["snippets"].as_array().unwrap();

        assert!(!snippets.is_empty(), "should include C++ code snippets");
    }

    #[test]
    fn detects_cpp_test_files() {
        let dir = setup_fixture("cpp_project");
        let path = index_fixture(&dir);

        // Query for "test authenticate" so test files aren't penalized
        let json = context_json_full(&path, "test authenticate");
        let symbols = json["key_symbols"].as_array().unwrap();

        assert!(
            symbols.iter().any(|s| s["name"]
                .as_str()
                .unwrap_or("")
                .contains("test_authenticate")),
            "should find test_authenticate symbol"
        );
    }
}

// C# project fixture
// ============================================================================

mod csharp_project {
    use super::*;

    #[test]
    fn index_succeeds() {
        let dir = setup_fixture("csharp_project");
        let path = dir.path().to_str().unwrap().to_string();
        pruner().args(["index", &path]).assert().success();
    }

    #[test]
    fn finds_csharp_classes() {
        let dir = setup_fixture("csharp_project");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "AuthHandler"])
            .assert()
            .success()
            .stdout(predicate::str::contains("class"));
    }

    #[test]
    fn finds_csharp_methods() {
        let dir = setup_fixture("csharp_project");
        let path = index_fixture(&dir);

        pruner()
            .args(["show-symbol", &path, "Authenticate"])
            .assert()
            .success()
            .stdout(predicate::str::contains("method"))
            .stdout(predicate::str::contains("AuthHandler.cs"));
    }

    #[test]
    fn query_finds_auth_symbols() {
        let dir = setup_fixture("csharp_project");
        let path = index_fixture(&dir);

        pruner()
            .args(["query", &path, "authenticate user"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Matching symbols:"));
    }

    #[test]
    fn context_includes_snippets() {
        let dir = setup_fixture("csharp_project");
        let path = index_fixture(&dir);

        let json = context_json_full(&path, "authenticate");
        let snippets = json["snippets"].as_array().unwrap();

        assert!(!snippets.is_empty(), "should include C# code snippets");
    }

    #[test]
    fn detects_csharp_test_files() {
        let dir = setup_fixture("csharp_project");
        let path = index_fixture(&dir);

        let json = context_json(&path, "authenticate");
        let tests = json["relevant_tests"].as_array().unwrap();

        assert!(
            tests.iter().any(|t| {
                t["path"]
                    .as_str()
                    .unwrap_or("")
                    .contains("AuthHandlerTests.cs")
            }),
            "should detect C# test file"
        );
    }

    #[test]
    fn context_has_execution_paths() {
        let dir = setup_fixture("csharp_project");
        let path = index_fixture(&dir);

        let json = context_json_full(&path, "authenticate");
        let paths = json["execution_paths"].as_array().unwrap();

        assert!(
            !paths.is_empty(),
            "should have execution paths from Authenticate"
        );
    }
}

#[cfg(test)]
mod upgrade {
    use assert_cmd::Command;
    use predicates::prelude::*;

    fn pruner() -> Command {
        Command::cargo_bin("pruner").unwrap()
    }

    #[test]
    fn upgrade_check_shows_version_info() {
        let output = pruner().args(["upgrade", "--check"]).output().unwrap();

        // Allow network failures (GitHub API rate limiting in CI)
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}");
        if output.status.success() {
            assert!(
                combined.contains("v0.1.") || combined.contains("up to date"),
                "unexpected output: stdout={stdout} stderr={stderr}"
            );
        } else {
            assert!(
                stderr.contains("403") || stderr.contains("Failed to fetch"),
                "unexpected error: {stderr}"
            );
        }
    }

    #[test]
    fn upgrade_check_does_not_modify_binary() {
        let exe = assert_cmd::cargo::cargo_bin("pruner");
        let before = std::fs::metadata(&exe).unwrap().modified().unwrap();

        // Ignore exit code — may fail due to network
        let _ = pruner().args(["upgrade", "--check"]).output();

        let after = std::fs::metadata(&exe).unwrap().modified().unwrap();
        assert_eq!(before, after, "Binary should not be modified by --check");
    }

    #[test]
    fn upgrade_help_shows_options() {
        pruner()
            .args(["upgrade", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--check"))
            .stdout(predicate::str::contains("--version"));
    }
}
