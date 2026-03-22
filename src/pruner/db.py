"""SQLite database layer for the pruner index."""

import sqlite3
from pathlib import Path

SCHEMA = """
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT UNIQUE NOT NULL,
    language TEXT,
    size INTEGER,
    line_count INTEGER,
    is_test INTEGER DEFAULT 0,
    indexed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS symbols (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,  -- function, class, method
    line_start INTEGER,
    line_end INTEGER,
    parent_symbol_id INTEGER REFERENCES symbols(id),
    signature TEXT
);

CREATE TABLE IF NOT EXISTS imports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    module TEXT NOT NULL,
    names TEXT  -- comma-separated imported names, NULL for whole-module imports
);

CREATE TABLE IF NOT EXISTS calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    caller_symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    callee_name TEXT NOT NULL,
    line INTEGER
);

CREATE TABLE IF NOT EXISTS edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,  -- contains, calls, imports, tests, config
    source_file_id INTEGER REFERENCES files(id),
    source_symbol_id INTEGER REFERENCES symbols(id),
    target_file_id INTEGER REFERENCES files(id),
    target_symbol_id INTEGER REFERENCES symbols(id),
    target_name TEXT  -- for unresolved references
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
"""


class IndexDB:
    """SQLite-backed index store."""

    def __init__(self, db_path: str | Path):
        self.db_path = Path(db_path)
        self.conn = sqlite3.connect(str(self.db_path))
        self.conn.row_factory = sqlite3.Row
        self.conn.execute("PRAGMA journal_mode=WAL")
        self.conn.execute("PRAGMA foreign_keys=ON")
        self._init_schema()

    def _init_schema(self):
        self.conn.executescript(SCHEMA)
        self.conn.commit()

    def clear(self):
        """Drop all data (for re-indexing)."""
        for table in ["edges", "calls", "imports", "symbols", "files"]:
            self.conn.execute(f"DELETE FROM {table}")
        self.conn.commit()

    def insert_file(self, path: str, language: str | None, size: int, line_count: int, is_test: bool) -> int:
        cur = self.conn.execute(
            "INSERT OR REPLACE INTO files (path, language, size, line_count, is_test) VALUES (?, ?, ?, ?, ?)",
            (path, language, size, line_count, int(is_test)),
        )
        self.conn.commit()
        return cur.lastrowid

    def insert_symbol(self, file_id: int, name: str, kind: str, line_start: int, line_end: int,
                       parent_symbol_id: int | None = None, signature: str | None = None) -> int:
        cur = self.conn.execute(
            "INSERT INTO symbols (file_id, name, kind, line_start, line_end, parent_symbol_id, signature) "
            "VALUES (?, ?, ?, ?, ?, ?, ?)",
            (file_id, name, kind, line_start, line_end, parent_symbol_id, signature),
        )
        self.conn.commit()
        return cur.lastrowid

    def insert_import(self, file_id: int, module: str, names: str | None = None) -> int:
        cur = self.conn.execute(
            "INSERT INTO imports (file_id, module, names) VALUES (?, ?, ?)",
            (file_id, module, names),
        )
        self.conn.commit()
        return cur.lastrowid

    def insert_call(self, caller_symbol_id: int, callee_name: str, line: int | None = None) -> int:
        cur = self.conn.execute(
            "INSERT INTO calls (caller_symbol_id, callee_name, line) VALUES (?, ?, ?)",
            (caller_symbol_id, callee_name, line),
        )
        self.conn.commit()
        return cur.lastrowid

    def insert_edge(self, kind: str, source_file_id: int | None = None, source_symbol_id: int | None = None,
                    target_file_id: int | None = None, target_symbol_id: int | None = None,
                    target_name: str | None = None) -> int:
        cur = self.conn.execute(
            "INSERT INTO edges (kind, source_file_id, source_symbol_id, target_file_id, target_symbol_id, target_name) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (kind, source_file_id, source_symbol_id, target_file_id, target_symbol_id, target_name),
        )
        self.conn.commit()
        return cur.lastrowid

    def get_file(self, path: str) -> dict | None:
        row = self.conn.execute("SELECT * FROM files WHERE path = ?", (path,)).fetchone()
        return dict(row) if row else None

    def get_file_by_id(self, file_id: int) -> dict | None:
        row = self.conn.execute("SELECT * FROM files WHERE id = ?", (file_id,)).fetchone()
        return dict(row) if row else None

    def search_files(self, pattern: str) -> list[dict]:
        rows = self.conn.execute("SELECT * FROM files WHERE path LIKE ?", (f"%{pattern}%",)).fetchall()
        return [dict(r) for r in rows]

    def get_symbols_in_file(self, file_id: int) -> list[dict]:
        rows = self.conn.execute("SELECT * FROM symbols WHERE file_id = ? ORDER BY line_start", (file_id,)).fetchall()
        return [dict(r) for r in rows]

    def search_symbols(self, pattern: str) -> list[dict]:
        rows = self.conn.execute("SELECT * FROM symbols WHERE name LIKE ?", (f"%{pattern}%",)).fetchall()
        return [dict(r) for r in rows]

    def get_symbol_by_name(self, name: str) -> list[dict]:
        rows = self.conn.execute("SELECT * FROM symbols WHERE name = ?", (name,)).fetchall()
        return [dict(r) for r in rows]

    def get_imports_for_file(self, file_id: int) -> list[dict]:
        rows = self.conn.execute("SELECT * FROM imports WHERE file_id = ?", (file_id,)).fetchall()
        return [dict(r) for r in rows]

    def get_calls_for_symbol(self, symbol_id: int) -> list[dict]:
        rows = self.conn.execute("SELECT * FROM calls WHERE caller_symbol_id = ?", (symbol_id,)).fetchall()
        return [dict(r) for r in rows]

    def get_callers_of(self, callee_name: str) -> list[dict]:
        rows = self.conn.execute(
            "SELECT c.*, s.name as caller_name, s.file_id FROM calls c "
            "JOIN symbols s ON c.caller_symbol_id = s.id WHERE c.callee_name = ?",
            (callee_name,),
        ).fetchall()
        return [dict(r) for r in rows]

    def get_edges(self, kind: str | None = None, source_file_id: int | None = None,
                  target_file_id: int | None = None) -> list[dict]:
        query = "SELECT * FROM edges WHERE 1=1"
        params = []
        if kind:
            query += " AND kind = ?"
            params.append(kind)
        if source_file_id is not None:
            query += " AND source_file_id = ?"
            params.append(source_file_id)
        if target_file_id is not None:
            query += " AND target_file_id = ?"
            params.append(target_file_id)
        rows = self.conn.execute(query, params).fetchall()
        return [dict(r) for r in rows]

    def get_all_files(self) -> list[dict]:
        rows = self.conn.execute("SELECT * FROM files ORDER BY path").fetchall()
        return [dict(r) for r in rows]

    def get_test_files(self) -> list[dict]:
        rows = self.conn.execute("SELECT * FROM files WHERE is_test = 1 ORDER BY path").fetchall()
        return [dict(r) for r in rows]

    def get_stats(self) -> dict:
        stats = {}
        for table in ["files", "symbols", "imports", "calls", "edges"]:
            row = self.conn.execute(f"SELECT COUNT(*) as cnt FROM {table}").fetchone()
            stats[table] = row["cnt"]
        return stats

    def close(self):
        self.conn.close()
