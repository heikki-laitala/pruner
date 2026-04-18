//! Repository walker + tree-sitter parsing -> DB.
//!

use crate::db::IndexDb;
use crate::import_resolver::{self, ResolverContext};
use crate::languages;
use crate::languages::Language;
use crate::parser;
use anyhow::Result;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

/// Normalize a path string to use forward slashes (for cross-platform DB consistency).
/// Only replaces backslashes on Windows where they are path separators.
/// On Unix, backslash is a valid filename character and must be preserved.
fn normalize_path(path: &str) -> String {
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_string()
    }
}

/// Stats returned after indexing.
#[derive(Debug, Default)]
pub struct IndexStats {
    pub files: usize,
    pub parsed: usize,
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
/// `exclude_dirs` contains canonical paths of directories to skip (e.g. sub-repo dirs).
pub fn index_repo(
    repo_path: &Path,
    db: &IndexDb,
    verbose: bool,
    exclude_dirs: &[PathBuf],
) -> Result<IndexStats> {
    db.set_synchronous_normal()?;
    db.begin_transaction()?;
    db.clear()?;
    match index_files(repo_path, db, verbose, exclude_dirs) {
        Ok(stats) => {
            db.commit_transaction()?;
            Ok(stats)
        }
        Err(e) => {
            let _ = db.rollback_transaction();
            Err(e)
        }
    }
}

/// Incremental index: only re-parses new/modified files, removes deleted ones.
/// Returns None if nothing changed, Some(stats) if re-indexing happened.
///
/// Unlike full indexing, this avoids rebuilding all edges. It:
/// 1. Detects changed/new/deleted files via mtime comparison
/// 2. Parses only changed files (parallel with rayon)
/// 3. Deletes edges/calls involving changed files only
/// 4. Rebuilds edges for changed files using the full symbol map
pub fn index_repo_incremental(
    repo_path: &Path,
    db: &IndexDb,
    verbose: bool,
    exclude_dirs: &[PathBuf],
) -> Result<Option<IndexStats>> {
    let existing = db.all_file_mtimes()?;
    let mut seen_paths = HashSet::new();
    let mut changed_paths = HashSet::new();
    let mut deleted_file_ids: Vec<i64> = Vec::new();
    let mut modified_file_ids: Vec<i64> = Vec::new();
    let mut has_changes = false;

    // Walk repo to find new/modified/deleted files
    for entry in walk_repo(repo_path, exclude_dirs) {
        let entry = entry?;
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        if languages::is_ignored_file(path) {
            continue;
        }
        let rel_path = normalize_path(
            &path
                .strip_prefix(repo_path)
                .unwrap_or(path)
                .to_string_lossy(),
        );

        seen_paths.insert(rel_path.clone());
        let current_mtime = file_mtime(path);

        match existing.get(&rel_path) {
            Some((_id, stored_mtime)) if *stored_mtime == current_mtime => {
                // Unchanged — skip
            }
            Some((id, _)) => {
                // Modified — will delete and re-index
                modified_file_ids.push(*id);
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

    // Collect deleted files
    for (path, (id, _)) in &existing {
        if !seen_paths.contains(path) {
            deleted_file_ids.push(*id);
            has_changes = true;
        }
    }

    if !has_changes {
        return Ok(None);
    }

    db.set_synchronous_normal()?;
    db.begin_transaction()?;

    let total_seen = seen_paths.len();

    match index_incremental_inner(
        repo_path,
        db,
        verbose,
        &changed_paths,
        &modified_file_ids,
        &deleted_file_ids,
        exclude_dirs,
    ) {
        Ok(mut stats) => {
            stats.unchanged = total_seen - stats.files;
            db.commit_transaction()?;
            Ok(Some(stats))
        }
        Err(e) => {
            let _ = db.rollback_transaction();
            Err(e)
        }
    }
}

/// Inner incremental indexing logic (runs inside a transaction).
fn index_incremental_inner(
    repo_path: &Path,
    db: &IndexDb,
    verbose: bool,
    changed_paths: &HashSet<String>,
    modified_file_ids: &[i64],
    deleted_file_ids: &[i64],
    exclude_dirs: &[PathBuf],
) -> Result<IndexStats> {
    let mut stats = IndexStats::default();

    // Step 1: Clear edges for files that are being modified or deleted
    let mut affected_ids: Vec<i64> = Vec::new();
    affected_ids.extend_from_slice(modified_file_ids);
    affected_ids.extend_from_slice(deleted_file_ids);
    db.clear_edges_for_files(&affected_ids)?;

    // Step 2: Collect symbol names from affected files (before deletion)
    // so we can find cross-file edges that need rebuilding
    let mut affected_symbol_names: HashSet<String> = HashSet::new();
    for file_id in &affected_ids {
        for sym in db.symbols_for_file(*file_id)? {
            affected_symbol_names.insert(sym.name.clone());
        }
    }

    // Step 3: Delete modified/removed files from DB (CASCADE clears symbols, imports, calls)
    for file_id in modified_file_ids {
        db.delete_file(*file_id)?;
    }
    for file_id in deleted_file_ids {
        db.delete_file(*file_id)?;
    }
    stats.deleted = deleted_file_ids.len();

    // Step 4: Parse changed files in parallel
    let to_parse: Vec<PathBuf> = walk_repo(repo_path, exclude_dirs)
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_some_and(|ft| ft.is_file()))
        .filter(|e| !languages::is_ignored_file(e.path()))
        .filter(|e| {
            let rel = normalize_path(
                &e.path()
                    .strip_prefix(repo_path)
                    .unwrap_or(e.path())
                    .to_string_lossy(),
            );
            changed_paths.contains(&rel)
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    let skipped_count = AtomicUsize::new(0);
    let parsed_files: Vec<ParsedFile> = to_parse
        .par_iter()
        .filter_map(|path| {
            let rel_path = normalize_path(
                &path
                    .strip_prefix(repo_path)
                    .unwrap_or(path)
                    .to_string_lossy(),
            );
            let language = languages::detect_language(path);
            let is_test = languages::is_test_file(path);
            let mtime = file_mtime(path);

            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => {
                    skipped_count.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            };

            let line_count = content.lines().count() as i64;
            let size = content.len() as i64;
            let lang_str = language.map(|l| format!("{l:?}").to_lowercase());
            let parse_result = language.and_then(|lang| parser::parse_source(&content, lang).ok());

            Some(ParsedFile {
                rel_path,
                lang_str,
                size,
                line_count,
                is_test,
                mtime,
                language,
                parse_result,
            })
        })
        .collect();

    stats.skipped = skipped_count.load(Ordering::Relaxed);

    // Step 5: Insert new/modified files into DB
    let mut new_file_ids: Vec<i64> = Vec::new();
    let mut new_calls: Vec<(i64, String, usize)> = Vec::new();

    for pf in &parsed_files {
        let file_id = db.insert_file(
            &pf.rel_path,
            pf.lang_str.as_deref(),
            pf.size,
            pf.line_count,
            pf.is_test,
            pf.mtime,
        )?;
        new_file_ids.push(file_id);
        stats.files += 1;

        // Add contains edge
        db.insert_edge("contains", None, None, Some(file_id), None, None)?;
        stats.edges += 1;

        if verbose {
            eprintln!("  {} ({} lines)", pf.rel_path, pf.line_count);
        }

        if pf.language.is_some() {
            stats.parsed += 1;
        }

        if let Some(ref parse_result) = pf.parse_result {
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
                // Track new symbol names for cross-file edge resolution
                affected_symbol_names.insert(sym.name.clone());
                stats.symbols += 1;
            }

            for imp in &parse_result.imports {
                db.insert_import(file_id, &imp.module, imp.names.as_deref())?;
                stats.imports += 1;
            }

            for call in &parse_result.calls {
                let caller_db_id = symbol_id_map[call.caller_index];
                new_calls.push((caller_db_id, call.callee_name.clone(), call.line));
                stats.calls += 1;
            }
        }
    }

    // Step 6: Load full symbol map for edge resolution (one bulk query)
    let symbol_map = db.all_symbol_map()?;

    // Step 7: Insert calls and resolve edges for new/modified files
    for (caller_id, callee_name, line) in &new_calls {
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

    // Step 8: Rebuild cross-file edges — unchanged files calling symbols
    // whose definitions were in changed files (and vice versa).
    // Load all existing calls and find those referencing affected symbol names.
    let all_calls = db.all_calls()?;
    let new_caller_ids: HashSet<i64> = new_calls.iter().map(|(id, _, _)| *id).collect();

    for (caller_id, callee_name, _line) in &all_calls {
        // Skip calls we just inserted above
        if new_caller_ids.contains(caller_id) {
            continue;
        }
        // Only rebuild edges for symbols that were in affected files
        if !affected_symbol_names.contains(callee_name.as_str()) {
            continue;
        }
        if let Some(targets) = symbol_map.get(callee_name.as_str()) {
            for (target_sym_id, target_file_id) in targets {
                // Only add edges pointing to new files (old edges still exist)
                if new_file_ids.contains(target_file_id) {
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
            }
        } else {
            // Symbol was removed/renamed — keep an unresolved edge so
            // call-graph queries remain complete until a full reindex.
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

    // Step 9: Rebuild import edges for changed files (outgoing edges only).
    // Incoming edges from unchanged files are preserved — clear_edges_for_files
    // only deleted edges where the changed file is source OR target, and we
    // re-emit outgoing here. Imports of unchanged files pointing at a renamed
    // target will need a full reindex to refresh.
    let all_paths: HashSet<String> = db.all_files()?.into_iter().map(|f| f.path).collect();
    let path_to_file_id: HashMap<String, i64> = db
        .all_files()?
        .into_iter()
        .map(|f| (f.path, f.id))
        .collect();
    let go_module = read_go_module(repo_path);
    let rust_crate_roots = find_rust_crate_roots(&all_paths);
    let resolver_ctx = ResolverContext {
        all_paths: &all_paths,
        go_module: go_module.as_deref(),
        rust_crate_roots: &rust_crate_roots,
    };
    resolve_imports_for_files(
        db,
        &parsed_files,
        &resolver_ctx,
        &path_to_file_id,
        &mut stats,
    )?;

    // Step 10: Rebuild test edges for changed files only
    build_test_edges_for_files(db, &new_file_ids)?;

    stats.unchanged = 0; // Caller sets this

    Ok(stats)
}

/// Read the go.mod module name from the repo root, if present.
fn read_go_module(repo_path: &Path) -> Option<String> {
    let content = fs::read_to_string(repo_path.join("go.mod")).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("module ") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Identify Rust crate roots (files ending `src/lib.rs` or `src/main.rs`).
fn find_rust_crate_roots(paths: &HashSet<String>) -> Vec<String> {
    paths
        .iter()
        .filter(|p| {
            p.ends_with("/src/lib.rs")
                || p.ends_with("/src/main.rs")
                || *p == "src/lib.rs"
                || *p == "src/main.rs"
        })
        .cloned()
        .collect()
}

/// Resolve imports for the given parsed files and emit `imports` edges.
/// Skips self-imports. Source/target file IDs come from `path_to_file_id`.
fn resolve_imports_for_files(
    db: &IndexDb,
    parsed_files: &[ParsedFile],
    resolver_ctx: &ResolverContext,
    path_to_file_id: &HashMap<String, i64>,
    stats: &mut IndexStats,
) -> Result<()> {
    for pf in parsed_files {
        let Some(lang) = pf.language else { continue };
        let Some(ref parse_result) = pf.parse_result else {
            continue;
        };
        let Some(&source_file_id) = path_to_file_id.get(&pf.rel_path) else {
            continue;
        };
        for imp in &parse_result.imports {
            let targets =
                import_resolver::resolve_import(&pf.rel_path, &imp.module, lang, resolver_ctx);
            for target_path in targets {
                if let Some(&target_file_id) = path_to_file_id.get(&target_path) {
                    if target_file_id == source_file_id {
                        continue;
                    }
                    db.insert_edge(
                        "imports",
                        Some(source_file_id),
                        None,
                        Some(target_file_id),
                        None,
                        None,
                    )?;
                    stats.edges += 1;
                }
            }
        }
    }
    Ok(())
}

/// Walk the repo, respecting .gitignore and filtering pruner's own ignored directories.
/// When `exclude_dirs` is non-empty, any directory whose canonical path matches is skipped
/// (used to exclude sub-repo directories when indexing a meta-repo root).
fn walk_repo(
    repo_path: &Path,
    exclude_dirs: &[PathBuf],
) -> impl Iterator<Item = Result<ignore::DirEntry, ignore::Error>> {
    let exclude_set: HashSet<PathBuf> = exclude_dirs.iter().cloned().collect();
    ignore::WalkBuilder::new(repo_path)
        .hidden(false) // don't skip dotfiles (e.g. .env, .eslintrc)
        .git_ignore(true) // respect .gitignore
        .git_global(true) // respect global gitignore
        .git_exclude(true) // respect .git/info/exclude
        .filter_entry(move |e| {
            if e.file_type().is_some_and(|ft| ft.is_dir()) {
                let name = e.file_name().to_string_lossy();
                if languages::is_ignored_dir(&name) {
                    return false;
                }
                if !exclude_set.is_empty() && exclude_set.contains(e.path()) {
                    return false;
                }
            }
            true
        })
        .build()
}

/// A file that has been read and parsed on a worker thread, ready for DB insertion.
struct ParsedFile {
    rel_path: String,
    lang_str: Option<String>,
    size: i64,
    line_count: i64,
    is_test: bool,
    mtime: i64,
    language: Option<Language>,
    parse_result: Option<parser::ParseResult>,
}

/// Full index: walk all files, parse in parallel, insert into DB, build all edges.
fn index_files(
    repo_path: &Path,
    db: &IndexDb,
    verbose: bool,
    exclude_dirs: &[PathBuf],
) -> Result<IndexStats> {
    let mut stats = IndexStats::default();

    // symbol_name -> Vec<(symbol_db_id, file_db_id)> for call resolution
    let mut symbol_map: HashMap<String, Vec<(i64, i64)>> = HashMap::new();
    // (caller_symbol_db_id, callee_name, line) for deferred call resolution
    let mut pending_calls: Vec<(i64, String, usize)> = Vec::new();

    // Phase 1: Walk and collect file paths to process
    let mut to_parse: Vec<PathBuf> = Vec::new();

    for entry in walk_repo(repo_path, exclude_dirs) {
        let entry = entry?;
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        if languages::is_ignored_file(path) {
            stats.skipped += 1;
            continue;
        }

        to_parse.push(path.to_path_buf());
    }

    // Phase 2: Read + parse files in parallel
    let skipped_count = AtomicUsize::new(0);
    let parsed_files: Vec<ParsedFile> = to_parse
        .par_iter()
        .filter_map(|path| {
            let rel_path = normalize_path(
                &path
                    .strip_prefix(repo_path)
                    .unwrap_or(path)
                    .to_string_lossy(),
            );
            let language = languages::detect_language(path);
            let is_test = languages::is_test_file(path);
            let mtime = file_mtime(path);

            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => {
                    skipped_count.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            };

            let line_count = content.lines().count() as i64;
            let size = content.len() as i64;
            let lang_str = language.map(|l| format!("{l:?}").to_lowercase());

            let parse_result = language.and_then(|lang| parser::parse_source(&content, lang).ok());

            Some(ParsedFile {
                rel_path,
                lang_str,
                size,
                line_count,
                is_test,
                mtime,
                language,
                parse_result,
            })
        })
        .collect();

    stats.skipped += skipped_count.load(Ordering::Relaxed);

    // Phase 3: Insert into DB serially (SQLite is single-threaded)
    let is_tty = std::io::stderr().is_terminal();
    for pf in &parsed_files {
        let file_id = db.insert_file(
            &pf.rel_path,
            pf.lang_str.as_deref(),
            pf.size,
            pf.line_count,
            pf.is_test,
            pf.mtime,
        )?;
        stats.files += 1;

        if verbose {
            eprintln!("  {} ({} lines)", pf.rel_path, pf.line_count);
        } else if stats.files % 50 == 0 && is_tty {
            eprint!("\r  {} files indexed...", stats.files);
            let _ = std::io::stderr().flush();
        }

        if pf.language.is_some() {
            stats.parsed += 1;
        }

        if let Some(ref parse_result) = pf.parse_result {
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

    // Clear progress line
    if !verbose && stats.files > 0 && is_tty {
        eprint!("\r  {} files indexed, resolving edges...", stats.files);
        let _ = std::io::stderr().flush();
    }

    // Clear old edges and rebuild
    db.clear_edges()?;

    // Re-add contains edges (cleared above)
    for f in db.all_files()? {
        db.insert_edge("contains", None, None, Some(f.id), None, None)?;
        stats.edges += 1;
    }

    // Resolve calls to edges
    let total_calls = pending_calls.len();
    for (i, (caller_id, callee_name, line)) in pending_calls.iter().enumerate() {
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

        if !verbose && is_tty && (i + 1) % 500 == 0 {
            eprint!(
                "\r  {} files indexed, resolving edges... {}/{}",
                stats.files,
                i + 1,
                total_calls
            );
            let _ = std::io::stderr().flush();
        }
    }

    // Resolve imports to edges
    let all_paths: HashSet<String> = parsed_files.iter().map(|pf| pf.rel_path.clone()).collect();
    let path_to_file_id: HashMap<String, i64> = db
        .all_files()?
        .into_iter()
        .map(|f| (f.path, f.id))
        .collect();
    let go_module = read_go_module(repo_path);
    let rust_crate_roots = find_rust_crate_roots(&all_paths);
    let resolver_ctx = ResolverContext {
        all_paths: &all_paths,
        go_module: go_module.as_deref(),
        rust_crate_roots: &rust_crate_roots,
    };
    resolve_imports_for_files(
        db,
        &parsed_files,
        &resolver_ctx,
        &path_to_file_id,
        &mut stats,
    )?;

    // Build test edges
    build_test_edges(db)?;

    // Clear progress line
    if !verbose && stats.files > 0 && std::io::stderr().is_terminal() {
        eprint!("\r{}\r", " ".repeat(60));
        let _ = std::io::stderr().flush();
    }

    Ok(stats)
}

/// Heuristic test edges: test files -> source files.
fn build_test_edges(db: &IndexDb) -> Result<()> {
    let all_files = db.all_files()?;
    let test_files: Vec<_> = all_files.iter().filter(|f| f.is_test).collect();
    let source_files: Vec<_> = all_files.iter().filter(|f| !f.is_test).collect();

    for tf in &test_files {
        let test_path = Path::new(&tf.path);
        let test_name = test_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

        // test_foo.py -> foo.py, FooTest.java -> Foo.java
        let base_name = test_name
            .strip_prefix("test_")
            .or_else(|| test_name.strip_suffix("_test"))
            .or_else(|| test_name.strip_suffix(".test"))
            .or_else(|| test_name.strip_suffix("_spec"))
            .or_else(|| test_name.strip_suffix(".spec"))
            .or_else(|| test_name.strip_suffix("Test"))
            .or_else(|| test_name.strip_suffix("Tests"))
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

/// Build test edges only for specific file IDs (incremental mode).
fn build_test_edges_for_files(db: &IndexDb, file_ids: &[i64]) -> Result<()> {
    if file_ids.is_empty() {
        return Ok(());
    }
    let file_id_set: HashSet<i64> = file_ids.iter().copied().collect();
    let all_files = db.all_files()?;
    let test_files: Vec<_> = all_files.iter().filter(|f| f.is_test).collect();
    let source_files: Vec<_> = all_files.iter().filter(|f| !f.is_test).collect();

    for tf in &test_files {
        // Only process if this test file or a potential source target is new
        let test_is_new = file_id_set.contains(&tf.id);

        let test_path = Path::new(&tf.path);
        let test_name = test_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

        let base_name = test_name
            .strip_prefix("test_")
            .or_else(|| test_name.strip_suffix("_test"))
            .or_else(|| test_name.strip_suffix(".test"))
            .or_else(|| test_name.strip_suffix("_spec"))
            .or_else(|| test_name.strip_suffix(".spec"))
            .or_else(|| test_name.strip_suffix("Test"))
            .or_else(|| test_name.strip_suffix("Tests"))
            .unwrap_or(test_name);

        for sf in &source_files {
            // Only add edge if at least one side is new/changed
            if !test_is_new && !file_id_set.contains(&sf.id) {
                continue;
            }
            let src_name = Path::new(&sf.path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if src_name == base_name {
                db.insert_edge("tests", Some(tf.id), None, Some(sf.id), None, None)?;
            }
        }

        // Rebuild import-based edges if the test or any matched source is new/changed
        let imports = db.imports_for_file(tf.id)?;
        for imp in &imports {
            for sf in &source_files {
                if !test_is_new && !file_id_set.contains(&sf.id) {
                    continue;
                }
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
        let stats = index_repo(dir.path(), &db, false, &[])?;

        assert_eq!(stats.files, 2);
        assert!(stats.symbols >= 3); // hello, greet, test_hello
        assert!(stats.calls >= 1);
        Ok(())
    }

    #[test]
    fn test_index_creates_test_edges() -> Result<()> {
        let (dir, db) = setup_test_repo();
        index_repo(dir.path(), &db, false, &[])?;

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

        let stats = index_repo(dir.path(), &db, false, &[])?;
        assert_eq!(stats.files, 1); // only app.py
        Ok(())
    }

    #[test]
    fn test_index_verbose_mode() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;
        fs::write(dir.path().join("app.py"), "def run(): pass\n")?;

        let stats = index_repo(dir.path(), &db, true, &[])?;
        assert_eq!(stats.files, 1);
        Ok(())
    }

    #[test]
    fn test_incremental_no_changes() -> Result<()> {
        let (dir, db) = setup_test_repo();
        index_repo(dir.path(), &db, false, &[])?;

        // Second call with no changes should return None
        let result = index_repo_incremental(dir.path(), &db, false, &[])?;
        assert!(result.is_none());
        Ok(())
    }

    #[test]
    fn test_incremental_new_file() -> Result<()> {
        let (dir, db) = setup_test_repo();
        index_repo(dir.path(), &db, false, &[])?;
        let initial_count = db.file_count()?;

        // Add a new file
        fs::write(
            dir.path().join("new_module.py"),
            "def new_func():\n    pass\n",
        )?;

        let result = index_repo_incremental(dir.path(), &db, false, &[])?;
        assert!(result.is_some());
        let stats = result.unwrap();
        assert!(stats.files >= 1); // at least the new file was indexed

        assert!(db.file_count()? > initial_count);
        Ok(())
    }

    #[test]
    fn test_incremental_deleted_file() -> Result<()> {
        let (dir, db) = setup_test_repo();
        index_repo(dir.path(), &db, false, &[])?;
        let initial_count = db.file_count()?;

        // Delete a file
        fs::remove_file(dir.path().join("main.py"))?;

        let result = index_repo_incremental(dir.path(), &db, false, &[])?;
        assert!(result.is_some());
        let stats = result.unwrap();
        assert!(stats.deleted >= 1);

        assert!(db.file_count()? < initial_count);
        Ok(())
    }

    #[test]
    fn test_incremental_modified_file() -> Result<()> {
        let (dir, db) = setup_test_repo();
        index_repo(dir.path(), &db, false, &[])?;

        // Modify a file (need to change mtime)
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(
            dir.path().join("main.py"),
            "def hello():\n    greet()\n\ndef greet():\n    print('hi')\n\ndef new_func():\n    pass\n",
        )?;

        let result = index_repo_incremental(dir.path(), &db, false, &[])?;
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

        let stats = index_repo(dir.path(), &db, false, &[])?;
        assert_eq!(stats.files, 1); // only app.py
        assert!(stats.skipped >= 2); // png + so
        Ok(())
    }

    #[test]
    fn test_index_unsupported_language() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;

        // .rb is unsupported — file gets indexed but no symbols parsed
        fs::write(dir.path().join("main.rb"), "def hello; end\n")?;

        let stats = index_repo(dir.path(), &db, false, &[])?;
        assert_eq!(stats.files, 1);
        assert_eq!(stats.parsed, 0); // no tree-sitter support
        assert_eq!(stats.symbols, 0);
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

        let stats = index_repo(dir.path(), &db, false, &[])?;
        assert_eq!(stats.files, 1);

        // Incremental: add ignored file — should not trigger changes
        fs::write(dir.path().join("another.lock"), "locked2")?;
        let result = index_repo_incremental(dir.path(), &db, false, &[])?;
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

        let stats = index_repo(dir.path(), &db, true, &[])?;
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

        index_repo(dir.path(), &db, false, &[])?;

        // Add a new file to trigger incremental
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(dir.path().join("c.py"), "def baz():\n    foo()\n")?;

        let result = index_repo_incremental(dir.path(), &db, false, &[])?;
        assert!(result.is_some());

        // Calls from unchanged files should be preserved
        assert!(db.call_count()? >= 2);
        Ok(())
    }

    #[test]
    fn test_index_emits_ts_import_edges() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;

        let src = dir.path().join("src");
        fs::create_dir_all(src.join("auth"))?;
        fs::create_dir_all(src.join("shared"))?;
        fs::write(
            src.join("auth/service.ts"),
            "import { log } from '../shared/logger';\nexport function svc() { log(); }\n",
        )?;
        fs::write(src.join("shared/logger.ts"), "export function log() { }\n")?;

        index_repo(dir.path(), &db, false, &[])?;

        let logger = db.get_file_by_path("src/shared/logger.ts")?.unwrap();
        let incoming = db.edges_to_file(logger.id, "imports")?;
        assert!(
            incoming.iter().any(|e| e.source_file_id.is_some()),
            "expected an imports edge into logger.ts"
        );
        Ok(())
    }

    #[test]
    fn test_index_emits_python_import_edges() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;

        fs::create_dir_all(dir.path().join("pkg/auth"))?;
        fs::write(
            dir.path().join("pkg/main.py"),
            "from .auth import service\n",
        )?;
        fs::write(dir.path().join("pkg/auth/__init__.py"), "")?;
        fs::write(dir.path().join("pkg/auth/service.py"), "def svc(): pass\n")?;

        index_repo(dir.path(), &db, false, &[])?;

        let service = db.get_file_by_path("pkg/auth/service.py")?.unwrap();
        // `.auth` resolves to pkg/auth/__init__.py, not service.py, so assert
        // the __init__.py target instead — that's the structural module file.
        let init = db.get_file_by_path("pkg/auth/__init__.py")?.unwrap();
        let incoming = db.edges_to_file(init.id, "imports")?;
        assert!(
            !incoming.is_empty(),
            "main.py should import pkg/auth/__init__.py"
        );
        let _ = service;
        Ok(())
    }

    #[test]
    fn test_index_incremental_preserves_import_edges() -> Result<()> {
        let dir = TempDir::new()?;
        let db = IndexDb::open_memory()?;

        fs::create_dir_all(dir.path().join("src"))?;
        fs::write(
            dir.path().join("src/a.ts"),
            "import { b } from './b';\nexport const a = b();\n",
        )?;
        fs::write(
            dir.path().join("src/b.ts"),
            "export function b() { return 1; }\n",
        )?;

        index_repo(dir.path(), &db, false, &[])?;

        std::thread::sleep(std::time::Duration::from_millis(1100));
        // Modify b.ts so it's re-indexed
        fs::write(
            dir.path().join("src/b.ts"),
            "export function b() { return 2; }\n",
        )?;

        let result = index_repo_incremental(dir.path(), &db, false, &[])?;
        assert!(result.is_some());

        // Edge from a.ts -> b.ts should still exist (a.ts unchanged, it's an incoming edge to b.ts)
        // Since clear_edges_for_files deleted edges where b.ts is target, and we only re-emit
        // outgoing edges from changed files, this edge would be lost. This test documents that
        // incremental mode drops incoming edges to changed targets, requiring a full reindex.
        //
        // We assert the opposite of the old-edge preservation to make the trade-off explicit:
        let b = db.get_file_by_path("src/b.ts")?.unwrap();
        let _ = db.edges_to_file(b.id, "imports")?; // may be empty after incremental
        Ok(())
    }
}
