//! SQLite schema and data access layer.
//!
//! Python reference: src/pruner/db.py

// Row structs include all DB columns for completeness even if not all fields
// are read yet. Methods mirror the Python API for feature parity.
#![allow(dead_code)]

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

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

    /// Clear all data from the database.
    pub fn clear(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM edges; DELETE FROM calls; DELETE FROM imports;
             DELETE FROM symbols; DELETE FROM files;",
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
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO files (path, language, size, line_count, is_test)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![path, language, size, line_count, is_test as i32],
        )?;
        Ok(self.conn.last_insert_rowid())
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
        Ok(self.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?)
    }

    pub fn symbol_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?)
    }

    pub fn import_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))?)
    }

    pub fn call_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM calls", [], |r| r.get(0))?)
    }

    pub fn edge_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?)
    }

    /// Search files by keyword in path.
    pub fn search_files(&self, keyword: &str) -> Result<Vec<FileRow>> {
        let pattern = format!("%{keyword}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, path, language, size, line_count, is_test FROM files WHERE path LIKE ?1",
        )?;
        let rows = stmt.query_map(params![pattern], |row| {
            Ok(FileRow {
                id: row.get(0)?,
                path: row.get(1)?,
                language: row.get(2)?,
                size: row.get(3)?,
                line_count: row.get(4)?,
                is_test: row.get::<_, i32>(5)? != 0,
            })
        })?;
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
        let rows = stmt.query_map(params![pattern], |row| {
            Ok(SymbolRow {
                id: row.get(0)?,
                file_id: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                line_start: row.get(4)?,
                line_end: row.get(5)?,
                signature: row.get(6)?,
                file_path: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Find edges by kind originating from a file.
    pub fn edges_from_file(&self, file_id: i64, kind: &str) -> Result<Vec<EdgeRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, source_file_id, source_symbol_id, target_file_id, target_symbol_id, target_name
             FROM edges WHERE source_file_id = ?1 AND kind = ?2",
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
        let rows = stmt.query_map(params![name], |row| {
            Ok(SymbolRow {
                id: row.get(0)?,
                file_id: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                line_start: row.get(4)?,
                line_end: row.get(5)?,
                signature: row.get(6)?,
                file_path: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get symbol by ID.
    pub fn get_symbol(&self, id: i64) -> Result<Option<SymbolRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature, f.path
             FROM symbols s JOIN files f ON s.file_id = f.id WHERE s.id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(SymbolRow {
                id: row.get(0)?,
                file_id: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                line_start: row.get(4)?,
                line_end: row.get(5)?,
                signature: row.get(6)?,
                file_path: row.get(7)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Get all files.
    pub fn all_files(&self) -> Result<Vec<FileRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, language, size, line_count, is_test FROM files",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FileRow {
                id: row.get(0)?,
                path: row.get(1)?,
                language: row.get(2)?,
                size: row.get(3)?,
                line_count: row.get(4)?,
                is_test: row.get::<_, i32>(5)? != 0,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get file by ID.
    pub fn get_file_by_path_id(&self, id: i64) -> Result<Option<FileRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, language, size, line_count, is_test FROM files WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(FileRow {
                id: row.get(0)?,
                path: row.get(1)?,
                language: row.get(2)?,
                size: row.get(3)?,
                line_count: row.get(4)?,
                is_test: row.get::<_, i32>(5)? != 0,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Get file by path.
    pub fn get_file_by_path(&self, path: &str) -> Result<Option<FileRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, language, size, line_count, is_test FROM files WHERE path = ?1",
        )?;
        let mut rows = stmt.query_map(params![path], |row| {
            Ok(FileRow {
                id: row.get(0)?,
                path: row.get(1)?,
                language: row.get(2)?,
                size: row.get(3)?,
                line_count: row.get(4)?,
                is_test: row.get::<_, i32>(5)? != 0,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// Get imports for a file.
    pub fn imports_for_file(&self, file_id: i64) -> Result<Vec<ImportRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, module, names FROM imports WHERE file_id = ?1",
        )?;
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

    /// Get symbols for a file.
    pub fn symbols_for_file(&self, file_id: i64) -> Result<Vec<SymbolRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.kind, s.line_start, s.line_end, s.signature, f.path
             FROM symbols s JOIN files f ON s.file_id = f.id WHERE s.file_id = ?1",
        )?;
        let rows = stmt.query_map(params![file_id], |row| {
            Ok(SymbolRow {
                id: row.get(0)?,
                file_id: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                line_start: row.get(4)?,
                line_end: row.get(5)?,
                signature: row.get(6)?,
                file_path: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Expose the connection for transactions (used by indexer).
    pub fn conn(&self) -> &Connection {
        &self.conn
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

#[derive(Debug, Clone)]
pub struct ImportRow {
    pub id: i64,
    pub file_id: i64,
    pub module: String,
    pub names: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CallRow {
    pub id: i64,
    pub caller_symbol_id: i64,
    pub callee_name: String,
    pub line: i64,
}

#[derive(Debug, Clone)]
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
        let id = db.insert_file("src/main.rs", Some("rust"), 100, 10, false)?;
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
        let file_id = db.insert_file("src/lib.rs", Some("rust"), 200, 20, false)?;
        db.insert_symbol(file_id, "parse_file", "function", 10, 30, None, Some("fn parse_file(path: &Path)"))?;
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
        let file_id = db.insert_file("src/lib.rs", Some("rust"), 200, 20, false)?;
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
    fn test_edges() -> Result<()> {
        let db = IndexDb::open_memory()?;
        let f1 = db.insert_file("src/a.py", Some("python"), 100, 10, false)?;
        let f2 = db.insert_file("tests/test_a.py", Some("python"), 50, 5, true)?;
        db.insert_edge("tests", Some(f2), None, Some(f1), None, None)?;

        let edges = db.edges_to_file(f1, "tests")?;
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_file_id, Some(f2));
        Ok(())
    }

    #[test]
    fn test_clear() -> Result<()> {
        let db = IndexDb::open_memory()?;
        db.insert_file("a.py", Some("python"), 10, 1, false)?;
        assert_eq!(db.file_count()?, 1);
        db.clear()?;
        assert_eq!(db.file_count()?, 0);
        Ok(())
    }
}
