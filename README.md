# Pruner

Synthetic code context engine for LLM coding tasks. Indexes a repository structurally, infers relevant execution paths from natural language asks, and generates compact LLM-ready context packages — all without using an LLM for indexing.

## Install

```bash
uv sync
```

## Usage

### Index a repository

```bash
pruner index /path/to/repo
pruner index .              # current directory
pruner index . -v           # verbose output
```

This creates a `.pruner/index.db` SQLite database inside the repo.

### Query the index

```bash
pruner query . "why is login broken?"
pruner query . "memory recall issue" --json-output
```

Returns matching files, symbols, related tests, and execution paths.

### Generate LLM context

```bash
pruner context . "fix the authentication flow"
pruner context . "add caching to the API" --format json
pruner context . "refactor data pipeline" --format both -o output
```

Produces a structured context package with:

- Inferred execution paths
- Key files and their symbols
- Key symbols with call graphs
- Related tests
- Code snippets

### Measure token savings

```bash
pruner measure . "how does the parser extract symbols?"
pruner measure . "fix login flow" --json-output
```

Compares three strategies and shows token savings:

- **Whole repo** — every indexed file (baseline)
- **Naive** — full content of all files matching the query
- **Pruner** — structured context with snippets and metadata

### Inspect the index

```bash
pruner show-file . src/auth.py
pruner show-symbol . login
pruner stats .
```

## Architecture

```
src/pruner/
├── cli.py        # Click CLI interface
├── db.py         # SQLite schema and access layer
├── indexer.py     # Repository walker + tree-sitter parsing → DB
├── parser.py      # Tree-sitter based symbol/import/call extraction
├── query.py       # Keyword extraction + heuristic relevance matching
├── context.py     # Context package generation (text + JSON)
├── tokens.py      # Token estimation and usage measurement
└── languages.py   # Language detection, test/config classification
```

### Indexing pipeline

1. Walk repository files, skip ignored dirs (node_modules, .git, etc.)
2. Detect language from file extension
3. Parse supported languages (Python, JS, TS) with tree-sitter
4. Extract symbols (functions, classes, methods), imports, and call sites
5. Build graph edges: contains, calls, imports, tests
6. Store everything in SQLite

### Query analysis

1. Extract keywords from natural language ask (stop word removal, camelCase/snake_case splitting)
2. Search files and symbols by keyword
3. Find related tests via graph edges
4. Trace execution paths through call graph
5. Infer subsystems from file paths

### Context generation

1. Collect execution paths, key files, key symbols, related tests
2. Extract code snippets from source files
3. Output as human-readable text and/or structured JSON

## Supported languages

Full tree-sitter parsing (symbols, imports, calls):

- Python
- JavaScript
- TypeScript
- Rust

Basic indexing (files, metadata):

- All text files not in the ignore list

## Limitations

- Call graph is best-effort — dynamic dispatch, string-based lookups, and indirect calls are not tracked
- Query analysis uses keyword matching, not semantic understanding
- Import resolution is heuristic (module name → file path mapping)
- No incremental re-indexing yet (full re-index on each run)
- Only Python/JS/TS/Rust get full symbol extraction

## Claude Code integration

Pruner integrates with Claude Code so that Claude automatically runs `pruner context` before making code changes — no manual invocation needed.

### Setup

**1. Install pruner globally:**

```bash
cd /path/to/pruner
uv tool install .
```

Or with pip:

```bash
pip install -e /path/to/pruner
```

Verify it works:

```bash
pruner --version
```

**2. Copy two files into your target project:**

```bash
# The skill (teaches Claude how to use pruner)
mkdir -p /path/to/your-project/.claude/skills/pruner
cp /path/to/pruner/.claude/skills/pruner/SKILL.md \
   /path/to/your-project/.claude/skills/pruner/SKILL.md

# The CLAUDE.md snippet (tells Claude to use pruner automatically)
cat /path/to/pruner/CLAUDE.md >> /path/to/your-project/CLAUDE.md
```

Or install the skill globally for all projects:

```bash
mkdir -p ~/.claude/skills/pruner
cp /path/to/pruner/.claude/skills/pruner/SKILL.md \
   ~/.claude/skills/pruner/SKILL.md
```

**3. Index the target repo** (one-time, re-run after major changes):

```bash
cd /path/to/your-project
pruner index . -v
```

### What happens automatically

Once set up, when you ask Claude Code to do something like "fix the login flow", Claude will:

1. Check if `.pruner/index.db` exists (index if not)
2. Run `pruner context . "fix the login flow"` to find relevant code
3. Read the identified files and snippets
4. Use execution paths and call graphs to understand the change surface
5. Proceed with the task, informed by focused context

No `/pruner` command needed — Claude uses it as part of its normal workflow.

### How it works

Two mechanisms work together:

- **CLAUDE.md** — project-level instructions that Claude reads at the start of every conversation. Tells Claude to run `pruner context` before making changes.
- **SKILL.md** — teaches Claude the full pruner API (all commands, workflow, options). Claude auto-loads this when it needs to use pruner. Set to `user-invocable: false` so it triggers automatically, not as a slash command.

## Similar projects

Several tools tackle the same problem — reducing LLM token waste when working with large codebases. Most use tree-sitter for structural parsing. The key difference is **how** they deliver context to the LLM.

### MCP server approach (interactive exploration)

These run as persistent servers that the LLM queries during conversation:

| Project | Lang | Description |
|---------|------|-------------|
| [CodeRLM](https://github.com/JaredStewart/coderlm) | Rust | Tree-sitter indexing with `search`, `impl`, `callers`, `tests` API. LLM explores iteratively. File watching, Go/Java/Scala support. Multi-platform (Cursor, Windsurf, Copilot). |
| [codebase-memory-mcp](https://github.com/DeusData/codebase-memory-mcp) | Go | Knowledge graph with tree-sitter + SQLite. 64 languages, 14 MCP tools, call chains. |
| [AiDex](https://github.com/CSCSoftware/AiDex) | ? | Tree-sitter + SQLite MCP server. Claims 50x less context than grep. |
| [Srclight](https://github.com/srclight/srclight) | ? | Tree-sitter + SQLite FTS5, call graphs, optional embeddings. 25+ tools, 10 languages. |
| [jCodeMunch-MCP](https://github.com/jgravelle/jcodemunch-mcp) | ? | Tree-sitter AST parsing MCP server. Claims 95%+ token reduction. |
| [code-index-mcp](https://github.com/johnhuang316/code-index-mcp) | ? | General code indexing MCP server. |

### Batch/CLI approach (pre-packaged context)

These generate context upfront — no server needed:

| Project | Lang | Description |
|---------|------|-------------|
| **Pruner** | Python | Tree-sitter + SQLite, NL query → execution paths + key files + snippets. One command, no server. |
| [Aider repo-map](https://github.com/Aider-AI/aider) | Python | Tree-sitter + PageRank to select most-referenced symbols. Embedded in Aider, not standalone. |
| [Repomix](https://github.com/yamadashy/repomix) | JS | Packs entire repo into one file. No structural parsing — brute-force inclusion. |
| [code2prompt](https://github.com/mufeedvh/code2prompt) | Rust | Converts codebase to single LLM prompt with templating. No structural parsing. |

### How pruner differs

- **Standalone CLI, no server** — `pruner context . "task" --brief` and done. No daemon, no MCP protocol, no hooks.
- **Natural language → context package** — takes a task description, infers execution paths, returns a complete package. Most MCP tools expose individual query primitives that the LLM must chain together.
- **Automatic execution path inference** — traces through the call graph to show how code flows (A → B → C). Most tools leave path-tracing to the LLM.
- **No LLM for indexing** — purely structural. Some tools (code-graph-rag, ctx-sys) use embeddings during indexing.
- **Tradeoff** — MCP servers are more flexible (LLM can ask follow-up queries), but cost more LLM turns. Pruner is simpler and cheaper per query.

## Future work

- Incremental indexing (only re-parse changed files)
- More language parsers (Go, Java, Ruby)
- Smarter query heuristics (TF-IDF, path-based weighting)
- Config/entrypoint detection
- Optional tiktoken integration for exact token counts
- Watch mode for continuous indexing
