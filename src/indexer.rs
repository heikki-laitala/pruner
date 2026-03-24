//! Repository walker + tree-sitter parsing -> DB.
//!

use crate::db::IndexDb;
use crate::languages;
use crate::parser;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use walkdir::WalkDir;

/// Stats returned after indexing.
#[derive(Debug, Default)]
pub struct IndexStats {
    pub files: usize,
    pub symbols: usize,
    pub imports: usize,
    pub calls: usize,
    pub edges: usize,
    pub skipped: usize,
    pub unchanged: usize,
    pub deleted: usize,
}

/// Get the mtime of a file as seconds since UNIX epoch.
fn file_mtime(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Full re-index: clears the database and indexes everything.
pub fn index_repo(repo_path: &Path, db: &IndexDb, verbose: bool) -> Result<IndexStats> {
    db.clear()?;
    index_files(repo_path, db, verbose, None)
}

/// Incremental index: only re-parses new/modified files, removes deleted ones.
/// Returns None if nothing changed, Some(stats) if re-indexing happened.
pub fn index_repo_incremental(
    repo_path: &Path,
    db: &IndexDb,
    verbose: bool,
) -> Result<Option<IndexStats>> {
    let existing = db.all_file_mtimes()?;
    let mut seen_paths = HashSet::new();
    let mut changed_paths = HashSet::new();
    let mut has_changes = false;

    // Walk repo to find new/modified/deleted files
    for entry in walk_repo(repo_path) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if languages::is_ignored_file(path) {
            continue;
        }
        let rel_path = path
            .strip_prefix(repo_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        seen_paths.insert(rel_path.clone());
        let current_mtime = file_mtime(path);

        match existing.get(&rel_path) {
            Some((_id, stored_mtime)) if *stored_mtime == current_mtime => {
                // Unchanged — skip
            }
            Some((id, _)) => {
                // Modified — delete old data, will re-index
                db.delete_file(*id)?;
                changed_paths.insert(rel_path);
                has_changes = true;
            }
            None => {
                // New file
                changed_paths.insert(rel_path);
                has_changes = true;
            }
        }
    }

    // Delete removed files
    for (path, (id, _)) in &existing {
        if !seen_paths.contains(path) {
            db.delete_file(*id)?;
            has_changes = true;
        }
    }

    if !has_changes {
        return Ok(None);
    }

    let deleted_count = existing.keys().filter(|p| !seen_paths.contains(*p)).count();

    // Re-index only changed files, then rebuild all edges
    let mut stats = index_files(repo_path, db, verbose, Some(&changed_paths))?;

    stats.unchanged = seen_paths.len() - stats.files;
    stats.deleted = deleted_count;

    Ok(Some(stats))
}

/// Walk the repo, filtering ignored directories.
fn walk_repo(repo_path: &Path) -> impl Iterator<Item = walkdir::Result<walkdir::DirEntry>> {
    WalkDir::new(repo_path).into_iter().filter_entry(|e| {
        if e.file_type().is_dir() {
            let name = e.file_name().to_string_lossy();
            return !languages::is_ignored_dir(&name);
        }
        true
    })
}

/// Index files into the DB. If `only_paths` is Some, only index those relative paths
/// (but still rebuild all edges from the full symbol set). None means index all files.
fn index_files(
    repo_path: &Path,
    db: &IndexDb,
    verbose: bool,
    only_paths: Option<&HashSet<String>>,
) -> Result<IndexStats> {
    let mut stats = IndexStats::default();

    // symbol_name -> Vec<(symbol_db_id, file_db_id)> for call resolution
    let mut symbol_map: HashMap<String, Vec<(i64, i64)>> = HashMap::new();
    // (caller_symbol_db_id, callee_name, line) for deferred call resolution
    let mut pending_calls: Vec<(i64, String, usize)> = Vec::new();

    for entry in walk_repo(repo_path) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if languages::is_ignored_file(path) {
            stats.skipped += 1;
            continue;
        }

        let rel_path = path
            .strip_prefix(repo_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // In incremental mode, skip files that don't need re-indexing
        // but still collect their symbols for edge resolution
        if only_paths.is_some_and(|paths| !paths.contains(&rel_path)) {
            // Collect existing symbols for call resolution
            if let Some(f) = db.get_file_by_path(&rel_path)? {
                for sym in db.symbols_for_file(f.id)? {
                    symbol_map
                        .entry(sym.name.clone())
                        .or_default()
                        .push((sym.id, f.id));
                }
            }
            stats.unchanged += 1;
            continue;
        }

        let language = languages::detect_language(path);
        let is_test = languages::is_test_file(path);
        let mtime = file_mtime(path);

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => {
                stats.skipped += 1;
                continue;
            }
        };

        let line_count = content.lines().count() as i64;
        let size = content.len() as i64;
        let lang_str = language.map(|l| format!("{l:?}").to_lowercase());

        let file_id = db.insert_file(
            &rel_path,
            lang_str.as_deref(),
            size,
            line_count,
            is_test,
            mtime,
        )?;
        stats.files += 1;

        if verbose {
            eprintln!("  {rel_path} ({} lines)", line_count);
        }

        // Parse if language is supported
        if let Some(lang) = language {
            let parse_result = match parser::parse_source(&content, lang) {
                Ok(r) => r,
                Err(e) => {
                    if verbose {
                        eprintln!("    parse error: {e}");
                    }
                    continue;
                }
            };

            let mut symbol_id_map: Vec<i64> = Vec::new();

            for sym in &parse_result.symbols {
                let parent_db_id = sym.parent_index.map(|pi| symbol_id_map[pi]);
                let sym_id = db.insert_symbol(
                    file_id,
                    &sym.name,
                    &sym.kind,
                    sym.line_start as i64,
                    sym.line_end as i64,
                    parent_db_id,
                    sym.signature.as_deref(),
                )?;
                symbol_id_map.push(sym_id);
                symbol_map
                    .entry(sym.name.clone())
                    .or_default()
                    .push((sym_id, file_id));
                stats.symbols += 1;
            }

            for imp in &parse_result.imports {
                db.insert_import(file_id, &imp.module, imp.names.as_deref())?;
                stats.imports += 1;
            }

            for call in &parse_result.calls {
                let caller_db_id = symbol_id_map[call.caller_index];
                pending_calls.push((caller_db_id, call.callee_name.clone(), call.line));
                stats.calls += 1;
            }
        }
    }

    // In incremental mode, collect calls from unchanged files before clearing edges
    if let Some(paths) = only_paths {
        for f in db.all_files()? {
            if paths.contains(&f.path) {
                continue; // Already collected from fresh parse above
            }
            for sym in db.symbols_for_file(f.id)? {
                for call in db.calls_by_symbol(sym.id)? {
                    pending_calls.push((sym.id, call.callee_name.clone(), call.line as usize));
                }
            }
        }
    }

    // Clear old edges and rebuild
    db.clear_edges()?;

    // Resolve calls to edges
    for (caller_id, callee_name, line) in &pending_calls {
        db.insert_call(*caller_id, callee_name, *line as i64)?;

        if let Some(targets) = symbol_map.get(callee_name.as_str()) {
            for (target_sym_id, target_file_id) in targets {
                db.insert_edge(
                    "calls",
                    None,
                    Some(*caller_id),
                    Some(*target_file_id),
                    Some(*target_sym_id),
                    None,
                )?;
                stats.edges += 1;
            }
        } else {
            db.insert_edge(
                "calls",
                None,
                Some(*caller_id),
                None,
                None,
                Some(callee_name),
            )?;
            stats.edges += 1;
        }
    }

    // Add contains edges for new files
    for f in db.all_files()? {
        db.insert_edge("contains", None, None, Some(f.id), None, None)?;
        stats.edges += 1;
    }

    // Build test edges
    build_test_edges(repo_path, db)?;

    Ok(stats)
}

/// Heuristic test edges: test files -> source files.
fn build_test_edges(_repo_path: &Path, db: &IndexDb) -> Result<()> {
    let all_files = db.all_files()?;
    let test_files: Vec<_> = all_files.iter().filter(|f| f.is_test).collect();
    let source_files: Vec<_> = all_files.iter().filter(|f| !f.is_test).collect();

    for tf in &test_files {
        let test_path = Path::new(&tf.path);
        let test_name = test_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

        // test_foo.py -> foo.py
        let base_name = test_name
            .strip_prefix("test_")
            .or_else(|| test_name.strip_suffix("_test"))
            .or_else(|| test_name.strip_suffix(".test"))
            .or_else(|| test_name.strip_suffix("_spec"))
            .or_else(|| test_name.strip_suffix(".spec"))
            .unwrap_or(test_name);

        for sf in &source_files {
            let src_name = Path::new(&sf.path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if src_name == base_name {
                db.insert_edge("tests", Some(tf.id), None, Some(sf.id), None, None)?;
            }
        }

        // Also match via imports
        let imports = db.imports_for_file(tf.id)?;
        for imp in &imports {
            for sf in &source_files {
                if sf.path.contains(&imp.module.replace('.', "/")) {
                    db.insert_edge("tests", Some(tf.id), None, Some(sf.id), None, None)?;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_repo() -> (TempDir, IndexDb) {
        let dir = TempDir::new().unwrap();
        let db = IndexDb::open_memory().unwrap();

        // Create a simple Python file
        fs::write(
            dir.path().join("main.py"),
            "def hello():\n    greet()\n\ndef greet():\n    print('hi')\n",
        )
        .unwrap();

        // Create a test file
        let test_dir = dir.path().join("tests");
        fs::create_dir(&test_dir).unwrap();
        fs::write(
            test_dir.join("test_main.py"),
            "from main import hello\n\ndef test_hello():\n    hello()\n",
        )
        .unwrap();

        (dir, db)
    }

    #[test]
    fn test_index_repo_basic() -> Result<()> {
        let (dir, db) = setup_test_repo();
        let stats = index_repo(dir.path(), &db, false)?;

        assert_eq!(stats.files, 2);
        assert!(stats.symbols >= 3); // hello, greet, test_hello
        assert!(stats.calls >= 1);
        Ok(())
    }

    #[test]
    fn test_index_creates_test_edges() -> Result<()> {
        let (dir, db) = setup_test_repo();
        index_repo(dir.path(), &db, false)?;

        let main_file = db.get_file_by_path("main.py")?.unwrap();
        let test_edges = db.edges_to_file(main_file.id, "tests")?;
        assert!(
            !test_edges.is_empty(),
            "should have test edges pointing to main.py"
        );
        Ok(())
    }

    #[test]
    fn test_index_skips_ignored_dirs() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;

        fs::write(dir.path().join("app.py"), "def run(): pass\n")?;
        let nm = dir.path().join("node_modules");
        fs::create_dir(&nm)?;
        fs::write(nm.join("lib.js"), "function x() {}")?;

        let stats = index_repo(dir.path(), &db, false)?;
        assert_eq!(stats.files, 1); // only app.py
        Ok(())
    }

    #[test]
    fn test_index_verbose_mode() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;
        fs::write(dir.path().join("app.py"), "def run(): pass\n")?;

        let stats = index_repo(dir.path(), &db, true)?;
        assert_eq!(stats.files, 1);
        Ok(())
    }

    #[test]
    fn test_incremental_no_changes() -> Result<()> {
        let (dir, db) = setup_test_repo();
        index_repo(dir.path(), &db, false)?;

        // Second call with no changes should return None
        let result = index_repo_incremental(dir.path(), &db, false)?;
        assert!(result.is_none());
        Ok(())
    }

    #[test]
    fn test_incremental_new_file() -> Result<()> {
        let (dir, db) = setup_test_repo();
        index_repo(dir.path(), &db, false)?;
        let initial_count = db.file_count()?;

        // Add a new file
        fs::write(
            dir.path().join("new_module.py"),
            "def new_func():\n    pass\n",
        )?;

        let result = index_repo_incremental(dir.path(), &db, false)?;
        assert!(result.is_some());
        let stats = result.unwrap();
        assert!(stats.files >= 1); // at least the new file was indexed

        assert!(db.file_count()? > initial_count);
        Ok(())
    }

    #[test]
    fn test_incremental_deleted_file() -> Result<()> {
        let (dir, db) = setup_test_repo();
        index_repo(dir.path(), &db, false)?;
        let initial_count = db.file_count()?;

        // Delete a file
        fs::remove_file(dir.path().join("main.py"))?;

        let result = index_repo_incremental(dir.path(), &db, false)?;
        assert!(result.is_some());
        let stats = result.unwrap();
        assert!(stats.deleted >= 1);

        assert!(db.file_count()? < initial_count);
        Ok(())
    }

    #[test]
    fn test_incremental_modified_file() -> Result<()> {
        let (dir, db) = setup_test_repo();
        index_repo(dir.path(), &db, false)?;

        // Modify a file (need to change mtime)
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(
            dir.path().join("main.py"),
            "def hello():\n    greet()\n\ndef greet():\n    print('hi')\n\ndef new_func():\n    pass\n",
        )?;

        let result = index_repo_incremental(dir.path(), &db, false)?;
        assert!(result.is_some());
        let stats = result.unwrap();
        assert!(stats.files >= 1); // modified file re-indexed
        Ok(())
    }

    #[test]
    fn test_index_skips_binary_files() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;

        fs::write(dir.path().join("app.py"), "def run(): pass\n")?;
        fs::write(dir.path().join("image.png"), &[0x89, 0x50, 0x4e, 0x47])?;
        fs::write(dir.path().join("lib.so"), &[0x7f, 0x45, 0x4c, 0x46])?;

        let stats = index_repo(dir.path(), &db, false)?;
        assert_eq!(stats.files, 1); // only app.py
        assert!(stats.skipped >= 2); // png + so
        Ok(())
    }

    #[test]
    fn test_index_unsupported_language() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;

        // .java is unsupported — file gets indexed but no symbols parsed
        fs::write(
            dir.path().join("Main.java"),
            "public class Main { public static void main(String[] args) {} }\n",
        )?;

        let stats = index_repo(dir.path(), &db, false)?;
        assert_eq!(stats.files, 1);
        assert_eq!(stats.symbols, 0); // no tree-sitter support
        Ok(())
    }

    #[test]
    fn test_file_mtime_nonexistent() {
        let mtime = file_mtime(Path::new("/nonexistent/path/file.rs"));
        assert_eq!(mtime, 0);
    }

    #[test]
    fn test_file_mtime_real_file() -> Result<()> {
        let dir = TempDir::new()?;
        let path = dir.path().join("test.txt");
        fs::write(&path, "hello")?;
        let mtime = file_mtime(&path);
        assert!(mtime > 0);
        Ok(())
    }

    #[test]
    fn test_incremental_skips_ignored_in_walk() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;

        fs::write(dir.path().join("app.py"), "def run(): pass\n")?;
        // Create an ignored file extension
        fs::write(dir.path().join("data.lock"), "locked")?;

        let stats = index_repo(dir.path(), &db, false)?;
        assert_eq!(stats.files, 1);

        // Incremental: add ignored file — should not trigger changes
        fs::write(dir.path().join("another.lock"), "locked2")?;
        let result = index_repo_incremental(dir.path(), &db, false)?;
        assert!(result.is_none()); // no changes to code files
        Ok(())
    }

    #[test]
    fn test_index_verbose_parse_error() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;

        // Valid file
        fs::write(dir.path().join("good.py"), "def hello(): pass\n")?;
        // Binary content in a .py file — parser may error
        fs::write(dir.path().join("bad.py"), &[0x00, 0x01, 0x02, 0xff])?;

        let stats = index_repo(dir.path(), &db, true)?;
        // At least the good file should be indexed
        assert!(stats.files >= 1);
        Ok(())
    }

    #[test]
    fn test_incremental_collects_calls_from_unchanged() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;

        // Create two files that call each other
        fs::write(dir.path().join("a.py"), "def foo():\n    bar()\n")?;
        fs::write(dir.path().join("b.py"), "def bar():\n    pass\n")?;

        index_repo(dir.path(), &db, false)?;

        // Add a new file to trigger incremental
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(dir.path().join("c.py"), "def baz():\n    foo()\n")?;

        let result = index_repo_incremental(dir.path(), &db, false)?;
        assert!(result.is_some());

        // Calls from unchanged files should be preserved
        assert!(db.call_count()? >= 2);
        Ok(())
    }
}
