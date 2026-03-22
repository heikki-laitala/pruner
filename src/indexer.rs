//! Repository walker + tree-sitter parsing -> DB.
//!

use crate::db::IndexDb;
use crate::languages;
use crate::parser;
use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
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
}

/// Index a repository into the database.
pub fn index_repo(repo_path: &Path, db: &IndexDb, verbose: bool) -> Result<IndexStats> {
    db.clear()?;
    let mut stats = IndexStats::default();

    // symbol_name -> Vec<(symbol_db_id, file_db_id)> for call resolution
    let mut symbol_map: HashMap<String, Vec<(i64, i64)>> = HashMap::new();
    // (caller_symbol_db_id, callee_name, line) for deferred call resolution
    let mut pending_calls: Vec<(i64, String, usize)> = Vec::new();

    for entry in WalkDir::new(repo_path)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy();
                return !languages::is_ignored_dir(&name);
            }
            true
        })
    {
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

        let language = languages::detect_language(path);
        let is_test = languages::is_test_file(path);

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
        )?;
        stats.files += 1;

        // Add contains edge
        db.insert_edge("contains", None, None, Some(file_id), None, None)?;
        stats.edges += 1;

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

            // Map from parser index -> db id
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

    // Resolve calls to edges
    for (caller_id, callee_name, _line) in &pending_calls {
        db.insert_call(*caller_id, callee_name, *_line as i64)?;

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
            // Unresolved call
            db.insert_edge("calls", None, Some(*caller_id), None, None, Some(callee_name))?;
            stats.edges += 1;
        }
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
        let test_name = test_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");

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
        assert!(!test_edges.is_empty(), "should have test edges pointing to main.py");
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
}
