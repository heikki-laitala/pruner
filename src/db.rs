//! SQLite schema and data access layer.
//!

use anyhow::Result;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::Path;

/// Escape SQL LIKE metacharacters (`%`, `_`) so they match literally.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub struct IndexDb {
    conn: Connection,
}

impl IndexDb {
    /// Open or create the database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        db.create_tables()?;
        Ok(db)
    }

    /// Open an in-memory database (for testing).
    #[cfg(test)]
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        db.create_tables()?;
        Ok(db)
    }

    fn create_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS files (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                path        TEXT NOT NULL UNIQUE,
                language    TEXT,
                size        INTEGER NOT NULL DEFAULT 0,
                line_count  INTEGER NOT NULL DEFAULT 0,
                is_test     INTEGER NOT NULL DEFAULT 0,
                mtime       INTEGER NOT NULL DEFAULT 0,
                indexed_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id          INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                name             TEXT NOT NULL,
                kind             TEXT NOT NULL,
                line_start       INTEGER NOT NULL,
                line_end         INTEGER NOT NULL,
                parent_symbol_id INTEGER REFERENCES symbols(id) ON DELETE SET NULL,
                signature        TEXT
            );

            CREATE TABLE IF NOT EXISTS imports (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                module  TEXT NOT NULL,
                names   TEXT
            );

            CREATE TABLE IF NOT EXISTS calls (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                caller_symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
                callee_name      TEXT NOT NULL,
                line             INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS edges (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                kind             TEXT NOT NULL,
                source_file_id   INTEGER REFERENCES files(id) ON DELETE CASCADE,
                source_symbol_id INTEGER REFERENCES symbols(id) ON DELETE CASCADE,
                target_file_id   INTEGER REFERENCES files(id) ON DELETE CASCADE,
                target_symbol_id INTEGER REFERENCES symbols(id) ON DELETE CASCADE,
                target_name      TEXT
            );

            CREATE TABLE IF NOT EXISTS metadata (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id);
            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_imports_file ON imports(file_id);
            CREATE INDEX IF NOT EXISTS idx_imports_module ON imports(module);
            CREATE INDEX IF NOT EXISTS idx_calls_caller ON calls(caller_symbol_id);
            CREATE INDEX IF NOT EXISTS idx_calls_callee ON calls(callee_name);
            CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);
            CREATE INDEX IF NOT EXISTS idx_edges_source_file ON edges(source_file_id);
            CREATE INDEX IF NOT EXISTS idx_edges_target_file ON edges(target_file_id);
            CREATE INDEX IF NOT EXISTS idx_edges_target_name ON edges(target_name);
            ",
        )?;
        Ok(())
    }

    /// Begin an explicit transaction (disables auto-commit per statement).
    pub fn begin_transaction(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN")?;
        Ok(())
    }

    /// Commit the current transaction.
    pub fn commit_transaction(&self) -> Result<()> {
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    /// Roll back the current transaction.
    pub fn rollback_transaction(&self) -> Result<()> {
        self.conn.execute_batch("ROLLBACK")?;
        Ok(())
    }

    /// Set synchronous mode for performance tuning during bulk writes.
    pub fn set_synchronous_normal(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        Ok(())
    }

    /// Clear all data from the database.
    pub fn clear(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM edges; DELETE FROM calls; DELETE FROM imports;
             DELETE FROM symbols; DELETE FROM files;",
        )?;
        Ok(())
    }

    // -- Metadata --

    pub fn get_metadata(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM metadata WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        Ok(rows.next()?.map(|r| r.get(0).unwrap()))
    }

    pub fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    // -- Files --

    pub fn insert_file(
        &self,
        path: &str,
        language: Option<&str>,
        size: i64,
        line_count: i64,
        is_test: bool,
        mtime: i64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO files (path, language, size, line_count, is_test, mtime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![path, language, size, line_count, is_test as i32, mtime],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Delete a file and all its associated data (CASCADE handles symbols, imports, calls, edges).
    pub fn delete_file(&self, file_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE id = ?1", params![file_id])?;
        Ok(())
    }

    /// Get all file paths and their stored mtimes.
    pub fn all_file_mtimes(&self) -> Result<HashMap<String, (i64, i64)>> {
        let mut stmt = self.conn.prepare("SELECT id, path, mtime FROM files")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                (row.get::<_, i64>(0)?, row.get::<_, i64>(2)?),
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (path, id_mtime) = row?;
            map.insert(path, id_mtime);
        }
        Ok(map)
    }

    /// Delete all edges and calls.
    pub fn clear_edges(&self) -> Result<()> {
        self.conn
            .execute_batch("DELETE FROM edges; DELETE FROM calls;")?;
        Ok(())
    }

    /// Delete edges and calls involving specific file IDs.
    pub fn clear_edges_for_files(&self, file_ids: &[i64]) -> Result<()> {
        if file_ids.is_empty() {
            return Ok(());
        }
        let placeholders: String = file_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        // Delete calls for symbols in these files
        let sql = format!(
            "DELETE FROM calls WHERE caller_symbol_id IN (SELECT id FROM symbols WHERE file_id IN ({placeholders}))"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = file_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        stmt.execute(params.as_slice())?;

        // Delete edges where source or target is one of these files
        let sql = format!(
            "DELETE FROM edges WHERE source_file_id IN ({placeholders}) OR target_file_id IN ({placeholders})"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut double_params: Vec<&dyn rusqlite::types::ToSql> = Vec::new();
        for id in file_ids {
            double_params.push(id as &dyn rusqlite::types::ToSql);
        }
        for id in file_ids {
            double_params.push(id as &dyn rusqlite::types::ToSql);
        }
        stmt.execute(double_params.as_slice())?;

        Ok(())
    }

    /// Load all symbols as a name -> Vec<(symbol_id, file_id)> map in one query.
    pub fn all_symbol_map(&self) -> Result<HashMap<String, Vec<(i64, i64)>>> {
        let mut stmt = self.conn.prepare("SELECT id, file_id, name FROM symbols")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut map: HashMap<String, Vec<(i64, i64)>> = HashMap::new();
        for row in rows {
            let (id, file_id, name) = row?;
            map.entry(name).or_default().push((id, file_id));
        }
        Ok(map)
    }

    /// Load all calls in one query: (caller_symbol_id, callee_name, line).
    pub fn all_calls(&self) -> Result<Vec<(i64, String, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT caller_symbol_id, callee_name, line FROM calls")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_symbol(
        &self,
        file_id: i64,
        name: &str,
        kind: &str,
        line_start: i64,
        line_end: i64,
        parent_symbol_id: Option<i64>,
        signature: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO symbols (file_id, name, kind, line_start, line_end, parent_symbol_id, signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![file_id, name, kind, line_start, line_end, parent_symbol_id, signature],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_import(&self, file_id: i64, module: &str, names: Option<&str>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO imports (file_id, module, names) VALUES (?1, ?2, ?3)",
            params![file_id, module, names],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_call(&self, caller_symbol_id: i64, callee_name: &str, line: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO calls (caller_symbol_id, callee_name, line) VALUES (?1, ?2, ?3)",
            params![caller_symbol_id, callee_name, line],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_edge(
        &self,
        kind: &str,
        source_file_id: Option<i64>,
        source_symbol_id: Option<i64>,
        target_file_id: Option<i64>,
        target_symbol_id: Option<i64>,
        target_name: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO edges (kind, source_file_id, source_symbol_id, target_file_id, target_symbol_id, target_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![kind, source_file_id, source_symbol_id, target_file_id, target_symbol_id, target_name],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    // -- Queries --

    pub fn file_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?)
    }

    pub fn symbol_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?)
    }

    pub fn import_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))?)
    }

    pub fn call_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM calls", [], |r| r.get(0))?)
    }

    pub fn edge_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?)
    }

    /// Count how many files contain a keyword in their path.
    /// Used for keyword specificity scoring — a keyword matching 30%+ of files is noise.
    pub fn count_files_matching(&self, keyword: &str) -> Result<i64> {
        let escaped = escape_like(keyword);
        let pattern = format!("%{escaped}%");
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE path LIKE ?1 ESCAPE '\\'",
            params![pattern],
            |r| r.get(0),
        )?)
    }

    /// Count how many symbols contain a keyword in their name.
    pub fn count_symbols_matching(&self, keyword: &str) -> Result<i64> {
        let escaped = escape_like(keyword);
        let pattern = format!("%{escaped}%");
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM symbols WHERE name LIKE ?1 ESCAPE '\\'",
            params![pattern],
            |r| r.get(0),
        )?)
    }

    /// Search files by keyword in path.
    pub fn search_files(&self, keyword: &str) -> Result<Vec<FileRow>> {
        let pattern = format!("%{keyword}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, path, language, size, line_count, is_test FROM files WHERE path LIKE ?1",
        )?;
        let rows = stmt.query_map(params![pattern], FileRow::from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Search symbols by keyword in name.
    pub fn search_symbols(&self, keyword: &str) -> Result<Vec<SymbolRow>> {
        let pattern = format!("%{keyword}%");
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature, f.path
             FROM symbols s JOIN files f ON s.file_id = f.id
             WHERE s.name LIKE ?1",
        )?;
        let rows = stmt.query_map(params![pattern], SymbolRow::from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Search symbols by keyword in signature (parameter names, types).
    /// Limited to avoid full-table scans on large repos.
    pub fn search_symbols_by_signature(&self, keyword: &str) -> Result<Vec<SymbolRow>> {
        let pattern = format!("%{keyword}%");
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature, f.path
             FROM symbols s JOIN files f ON s.file_id = f.id
             WHERE s.signature LIKE ?1 AND s.name NOT LIKE ?1
             LIMIT 200",
        )?;
        let rows = stmt.query_map(params![pattern], SymbolRow::from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Find file IDs that import a module matching a keyword.
    pub fn search_importing_files(&self, keyword: &str) -> Result<Vec<i64>> {
        let pattern = format!("%{keyword}%");
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT file_id FROM imports WHERE module LIKE ?1 LIMIT 200")?;
        let rows = stmt.query_map(params![pattern], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Find symbols (callers) that call a function matching a keyword.
    /// Returns the caller symbol rows, limited to avoid expensive full scans.
    pub fn search_callers_of(&self, keyword: &str) -> Result<Vec<SymbolRow>> {
        let pattern = format!("%{keyword}%");
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature, f.path
             FROM calls c
             JOIN symbols s ON c.caller_symbol_id = s.id
             JOIN files f ON s.file_id = f.id
             WHERE c.callee_name LIKE ?1 AND s.name NOT LIKE ?1
             LIMIT 200",
        )?;
        let rows = stmt.query_map(params![pattern], SymbolRow::from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Find edges targeting a file.
    pub fn edges_to_file(&self, file_id: i64, kind: &str) -> Result<Vec<EdgeRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, source_file_id, source_symbol_id, target_file_id, target_symbol_id, target_name
             FROM edges WHERE target_file_id = ?1 AND kind = ?2",
        )?;
        let rows = stmt.query_map(params![file_id, kind], |row| {
            Ok(EdgeRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                source_file_id: row.get(2)?,
                source_symbol_id: row.get(3)?,
                target_file_id: row.get(4)?,
                target_symbol_id: row.get(5)?,
                target_name: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get calls made by a symbol.
    pub fn calls_by_symbol(&self, symbol_id: i64) -> Result<Vec<CallRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, caller_symbol_id, callee_name, line FROM calls WHERE caller_symbol_id = ?1",
        )?;
        let rows = stmt.query_map(params![symbol_id], |row| {
            Ok(CallRow {
                id: row.get(0)?,
                caller_symbol_id: row.get(1)?,
                callee_name: row.get(2)?,
                line: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Find symbols that call a given name.
    pub fn callers_of(&self, name: &str) -> Result<Vec<SymbolRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature, f.path
             FROM symbols s
             JOIN calls c ON c.caller_symbol_id = s.id
             JOIN files f ON s.file_id = f.id
             WHERE c.callee_name = ?1",
        )?;
        let rows = stmt.query_map(params![name], SymbolRow::from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get all files.
    pub fn all_files(&self) -> Result<Vec<FileRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, language, size, line_count, is_test FROM files")?;
        let rows = stmt.query_map([], FileRow::from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Return every symbol in the index. Used by the fuzzy rescue pass when
    /// strict LIKE retrieval returns nothing for a keyword, so typo'd prompts
    /// can still surface edit-distance-1 matches.
    pub fn all_symbols(&self) -> Result<Vec<SymbolRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature, f.path
             FROM symbols s JOIN files f ON s.file_id = f.id",
        )?;
        let rows = stmt.query_map([], SymbolRow::from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get file by ID.
    pub fn get_file_by_path_id(&self, id: i64) -> Result<Option<FileRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, language, size, line_count, is_test FROM files WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], FileRow::from_row)?;
        Ok(rows.next().transpose()?)
    }

    /// Get file by path.
    pub fn get_file_by_path(&self, path: &str) -> Result<Option<FileRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, language, size, line_count, is_test FROM files WHERE path = ?1",
        )?;
        let mut rows = stmt.query_map(params![path], FileRow::from_row)?;
        Ok(rows.next().transpose()?)
    }

    /// Get imports for a file.
    pub fn imports_for_file(&self, file_id: i64) -> Result<Vec<ImportRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, file_id, module, names FROM imports WHERE file_id = ?1")?;
        let rows = stmt.query_map(params![file_id], |row| {
            Ok(ImportRow {
                id: row.get(0)?,
                file_id: row.get(1)?,
                module: row.get(2)?,
                names: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Trace the call graph from a symbol using a recursive CTE.
    /// Returns callee symbols reachable within `max_depth` hops, ordered by depth.
    /// This replaces per-step DFS with a single SQL query, eliminating millions of
    /// round-trips on large repos.
    pub fn trace_call_graph(
        &self,
        start_symbol_id: i64,
        max_depth: usize,
    ) -> Result<Vec<TraceRow>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE cg(symbol_id, depth) AS (
                SELECT ?1, 0
                UNION
                SELECT s.id, cg.depth + 1
                FROM cg
                JOIN calls c ON c.caller_symbol_id = cg.symbol_id
                JOIN symbols s ON s.name = c.callee_name
                WHERE cg.depth < ?2
                  AND s.id != ?1
            )
            SELECT DISTINCT s.id, s.name, s.kind, f.path, s.line_start, cg.depth
            FROM cg
            JOIN symbols s ON s.id = cg.symbol_id
            JOIN files f ON s.file_id = f.id
            WHERE cg.depth > 0
            ORDER BY cg.depth, s.id
            LIMIT 50",
        )?;
        let rows = stmt.query_map(params![start_symbol_id, max_depth as i64], |row| {
            Ok(TraceRow {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                line_start: row.get(4)?,
                depth: row.get::<_, i64>(5)? as usize,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get symbols for a file.
    pub fn symbols_for_file(&self, file_id: i64) -> Result<Vec<SymbolRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature, f.path
             FROM symbols s JOIN files f ON s.file_id = f.id WHERE s.file_id = ?1",
        )?;
        let rows = stmt.query_map(params![file_id], SymbolRow::from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

// -- Row types --

#[derive(Debug, Clone)]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub language: Option<String>,
    pub size: i64,
    pub line_count: i64,
    pub is_test: bool,
}

impl FileRow {
    /// Map a row from: SELECT id, path, language, size, line_count, is_test FROM files
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            path: row.get(1)?,
            language: row.get(2)?,
            size: row.get(3)?,
            line_count: row.get(4)?,
            is_test: row.get::<_, i32>(5)? != 0,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SymbolRow {
    pub id: i64,
    pub file_id: i64,
    pub name: String,
    pub kind: String,
    pub line_start: i64,
    pub line_end: i64,
    pub signature: Option<String>,
    pub file_path: String,
}

impl SymbolRow {
    /// Map a row from: SELECT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature, f.path
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            file_id: row.get(1)?,
            name: row.get(2)?,
            kind: row.get(3)?,
            line_start: row.get(4)?,
            line_end: row.get(5)?,
            signature: row.get(6)?,
            file_path: row.get(7)?,
        })
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ImportRow {
    pub id: i64,
    pub file_id: i64,
    pub module: String,
    pub names: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CallRow {
    pub id: i64,
    pub caller_symbol_id: i64,
    pub callee_name: String,
    pub line: i64,
}

#[derive(Debug, Clone)]
pub struct TraceRow {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line_start: i64,
    pub depth: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EdgeRow {
    pub id: i64,
    pub kind: String,
    pub source_file_id: Option<i64>,
    pub source_symbol_id: Option<i64>,
    pub target_file_id: Option<i64>,
    pub target_symbol_id: Option<i64>,
    pub target_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_query_file() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let id = db.insert_file("src/main.rs", Some("rust"), 100, 10, false, 0)?;
        assert_eq!(db.file_count()?, 1);

        let file = db.get_file_by_path("src/main.rs")?.unwrap();
        assert_eq!(file.id, id);
        assert_eq!(file.language.as_deref(), Some("rust"));
        assert!(!file.is_test);
        Ok(())
    }

    #[test]
    fn test_insert_symbol_and_search() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let file_id = db.insert_file("src/lib.rs", Some("rust"), 200, 20, false, 0)?;
        db.insert_symbol(
            file_id,
            "parse_file",
            "function",
            10,
            30,
            None,
            Some("fn parse_file(path: &Path)"),
        )?;
        db.insert_symbol(file_id, "detect_language", "function", 35, 50, None, None)?;

        assert_eq!(db.symbol_count()?, 2);

        let results = db.search_symbols("parse")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "parse_file");
        Ok(())
    }

    #[test]
    fn test_calls_and_callers() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let file_id = db.insert_file("src/lib.rs", Some("rust"), 200, 20, false, 0)?;
        let caller_id = db.insert_symbol(file_id, "index_repo", "function", 1, 10, None, None)?;
        let _callee_id = db.insert_symbol(file_id, "parse_file", "function", 15, 30, None, None)?;
        db.insert_call(caller_id, "parse_file", 5)?;

        let calls = db.calls_by_symbol(caller_id)?;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].callee_name, "parse_file");

        let callers = db.callers_of("parse_file")?;
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].name, "index_repo");
        Ok(())
    }

    #[test]
    fn test_trace_call_graph_cte() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let f = db.insert_file("src/lib.rs", Some("rust"), 200, 50, false, 0)?;
        let a = db.insert_symbol(f, "a", "function", 1, 10, None, None)?;
        let b = db.insert_symbol(f, "b", "function", 11, 20, None, None)?;
        db.insert_symbol(f, "c", "function", 21, 30, None, None)?;
        // a -> b -> c
        db.insert_call(a, "b", 5)?;
        db.insert_call(b, "c", 15)?;

        let trace = db.trace_call_graph(a, 5)?;
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].name, "b");
        assert_eq!(trace[0].depth, 1);
        assert_eq!(trace[1].name, "c");
        assert_eq!(trace[1].depth, 2);
        Ok(())
    }

    #[test]
    fn test_trace_call_graph_respects_max_depth() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let f = db.insert_file("src/lib.rs", Some("rust"), 200, 50, false, 0)?;
        let a = db.insert_symbol(f, "a", "function", 1, 10, None, None)?;
        let b = db.insert_symbol(f, "b", "function", 11, 20, None, None)?;
        db.insert_symbol(f, "c", "function", 21, 30, None, None)?;
        db.insert_call(a, "b", 5)?;
        db.insert_call(b, "c", 15)?;

        let trace = db.trace_call_graph(a, 1)?;
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].name, "b");
        Ok(())
    }

    #[test]
    fn test_trace_call_graph_no_self_loops() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let f = db.insert_file("src/lib.rs", Some("rust"), 200, 50, false, 0)?;
        let a = db.insert_symbol(f, "a", "function", 1, 10, None, None)?;
        // a calls itself
        db.insert_call(a, "a", 5)?;

        let trace = db.trace_call_graph(a, 5)?;
        assert!(trace.is_empty());
        Ok(())
    }

    #[test]
    fn test_edges() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let f1 = db.insert_file("src/a.py", Some("python"), 100, 10, false, 0)?;
        let f2 = db.insert_file("tests/test_a.py", Some("python"), 50, 5, true, 0)?;
        db.insert_edge("tests", Some(f2), None, Some(f1), None, None)?;

        let edges = db.edges_to_file(f1, "tests")?;
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_file_id, Some(f2));
        Ok(())
    }

    #[test]
    fn test_clear() -> Result<()> {
        let db = IndexDb::open_memory()?;
        db.insert_file("a.py", Some("python"), 10, 1, false, 0)?;
        assert_eq!(db.file_count()?, 1);
        db.clear()?;
        assert_eq!(db.file_count()?, 0);
        Ok(())
    }

    #[test]
    fn test_delete_file_cascades() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let fid = db.insert_file("src/main.rs", Some("rust"), 100, 10, false, 0)?;
        let sid = db.insert_symbol(fid, "main", "function", 1, 10, None, None)?;
        db.insert_call(sid, "helper", 5)?;
        db.insert_import(fid, "std::io", Some("Read"))?;
        db.insert_edge("tests", None, None, Some(fid), None, None)?;

        assert_eq!(db.symbol_count()?, 1);
        assert_eq!(db.call_count()?, 1);
        assert_eq!(db.import_count()?, 1);

        db.delete_file(fid)?;
        assert_eq!(db.file_count()?, 0);
        assert_eq!(db.symbol_count()?, 0);
        assert_eq!(db.call_count()?, 0);
        assert_eq!(db.import_count()?, 0);
        Ok(())
    }

    #[test]
    fn test_all_file_mtimes() -> Result<()> {
        let db = IndexDb::open_memory()?;
        db.insert_file("a.rs", Some("rust"), 100, 10, false, 1000)?;
        db.insert_file("b.rs", Some("rust"), 200, 20, false, 2000)?;

        let mtimes = db.all_file_mtimes()?;
        assert_eq!(mtimes.len(), 2);
        assert_eq!(mtimes["a.rs"].1, 1000); // (id, mtime) — check mtime
        assert_eq!(mtimes["b.rs"].1, 2000);
        Ok(())
    }

    #[test]
    fn test_clear_edges() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let f1 = db.insert_file("a.rs", Some("rust"), 100, 10, false, 0)?;
        let f2 = db.insert_file("test_a.rs", Some("rust"), 50, 5, true, 0)?;
        let s = db.insert_symbol(f1, "foo", "function", 1, 10, None, None)?;
        db.insert_call(s, "bar", 5)?;
        db.insert_edge("tests", Some(f2), None, Some(f1), None, None)?;

        assert_eq!(db.call_count()?, 1);
        assert_eq!(db.edge_count()?, 1);

        db.clear_edges()?;
        assert_eq!(db.call_count()?, 0);
        assert_eq!(db.edge_count()?, 0);
        // files and symbols should remain
        assert_eq!(db.file_count()?, 2);
        assert_eq!(db.symbol_count()?, 1);
        Ok(())
    }

    #[test]
    fn test_all_files() -> Result<()> {
        let db = IndexDb::open_memory()?;
        db.insert_file("a.rs", Some("rust"), 100, 10, false, 0)?;
        db.insert_file("b.py", Some("python"), 200, 20, true, 0)?;

        let files = db.all_files()?;
        assert_eq!(files.len(), 2);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"a.rs"));
        assert!(paths.contains(&"b.py"));
        assert!(files.iter().any(|f| f.is_test));
        Ok(())
    }

    #[test]
    fn test_get_file_by_path_id() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let id = db.insert_file("src/lib.rs", Some("rust"), 100, 10, false, 0)?;

        let found = db.get_file_by_path_id(id)?;
        assert!(found.is_some());
        assert_eq!(found.unwrap().path, "src/lib.rs");

        let missing = db.get_file_by_path_id(9999)?;
        assert!(missing.is_none());
        Ok(())
    }

    #[test]
    fn test_get_file_by_path_not_found() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let result = db.get_file_by_path("nonexistent.rs")?;
        assert!(result.is_none());
        Ok(())
    }

    #[test]
    fn test_imports_for_file() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let fid = db.insert_file("main.py", Some("python"), 100, 10, false, 0)?;
        db.insert_import(fid, "os", None)?;
        db.insert_import(fid, "sys", Some("argv, exit"))?;

        let imports = db.imports_for_file(fid)?;
        assert_eq!(imports.len(), 2);
        assert!(
            imports
                .iter()
                .any(|i| i.module == "os" && i.names.is_none())
        );
        assert!(
            imports
                .iter()
                .any(|i| i.module == "sys" && i.names.as_deref() == Some("argv, exit"))
        );
        Ok(())
    }

    #[test]
    fn test_symbols_for_file() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let fid = db.insert_file("lib.rs", Some("rust"), 200, 50, false, 0)?;
        db.insert_symbol(fid, "foo", "function", 1, 10, None, None)?;
        db.insert_symbol(fid, "bar", "function", 11, 20, None, None)?;

        let other_fid = db.insert_file("other.rs", Some("rust"), 100, 10, false, 0)?;
        db.insert_symbol(other_fid, "baz", "function", 1, 5, None, None)?;

        let syms = db.symbols_for_file(fid)?;
        assert_eq!(syms.len(), 2);
        assert!(syms.iter().all(|s| s.file_path == "lib.rs"));
        Ok(())
    }

    #[test]
    fn test_search_files() -> Result<()> {
        let db = IndexDb::open_memory()?;
        db.insert_file("src/auth/login.rs", Some("rust"), 100, 10, false, 0)?;
        db.insert_file("src/api/routes.rs", Some("rust"), 200, 20, false, 0)?;
        db.insert_file("tests/test_auth.rs", Some("rust"), 50, 5, true, 0)?;

        let results = db.search_files("auth")?;
        assert_eq!(results.len(), 2); // login.rs path contains "auth", test_auth.rs too
        assert!(results.iter().all(|f| f.path.contains("auth")));

        let empty = db.search_files("nonexistent")?;
        assert!(empty.is_empty());
        Ok(())
    }

    #[test]
    fn test_search_symbols_empty() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let results = db.search_symbols("anything")?;
        assert!(results.is_empty());
        Ok(())
    }

    #[test]
    fn test_parent_symbol() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let fid = db.insert_file("lib.rs", Some("rust"), 200, 50, false, 0)?;
        let parent = db.insert_symbol(fid, "MyStruct", "struct", 1, 20, None, None)?;
        db.insert_symbol(
            fid,
            "my_method",
            "method",
            5,
            15,
            Some(parent),
            Some("fn my_method(&self)"),
        )?;

        let syms = db.symbols_for_file(fid)?;
        assert_eq!(syms.len(), 2);
        Ok(())
    }
}
