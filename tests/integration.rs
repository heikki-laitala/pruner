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
// Cross-cutting: measure command
// ============================================================================

mod measure {
    use super::*;

    #[test]
    fn measure_produces_token_comparison() {
        let dir = setup_fixture("python_webapp");
        let path = index_fixture(&dir);

        pruner()
            .args(["measure", &path, "authenticate user"])
            .assert()
            .success()
            .stdout(predicate::str::contains("token"));
    }
}

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
