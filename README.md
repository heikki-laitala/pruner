# Pruner

Synthetic code context engine for LLM coding tasks. Indexes a repository structurally, infers relevant execution paths from natural language asks, and generates compact LLM-ready context packages — all without using an LLM for indexing.

## Install

```bash
cargo install --path .
```

Or build locally:

```bash
cargo build --release
# Binary at ./target/release/pruner
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

Pruner works in two phases: a cheap **brief** scan to orient, then **targeted reads** of source files.

**Phase 1 — Orient with brief mode (~3K tokens):**

```bash
pruner context . "fix WebSocket reconnection timeout" --brief
```

Prints a compact table of contents: key files, symbols with locations, shallow execution paths, and related tests. No snippets — just pointers. Also writes to `.pruner/context.md`.

**Phase 2 — Read what matters:**

Use the file paths and symbol locations from brief output to read only the relevant source files. For most tasks, reading 3-5 files is enough.

**Full mode** (when you need deep detail):

```bash
pruner context . "fix the authentication flow"
pruner context . "add caching to the API" --format json
```

Produces a complete context package with execution paths, key files, key symbols, related tests, and code snippets (~55-70K tokens on large repos).

| Mode | Files | Symbols | Paths | Snippets | Tokens |
|------|-------|---------|-------|----------|--------|
| Brief | 8 | 15 | 3 (shallow) | 0 | ~3K |
| Full | 25 | 40 | unlimited | 40 | ~55-70K |

### Measure token savings

```bash
pruner measure . "how does the parser extract symbols?"
pruner estimate . "fix login flow" --show-steps
```

Compares token usage strategies and estimates Claude Code session savings.

### Inspect the index

```bash
pruner show-file . src/auth.py
pruner show-symbol . login
pruner stats .
```

## A/B test results

Tested on [openclaw/openclaw](https://github.com/openclaw/openclaw) (9,794 files, 30,695 symbols). Three strategies compared across 5 query types:

- **A — Brief + read**: `pruner context --brief` (~3K tokens) then read the 8 key files it identifies
- **B — Full dump**: `pruner context` (full mode) dumped into conversation
- **C — No pruner**: Simulated vanilla Claude Code behavior (glob directory structure, grep for keywords, read relevant + some irrelevant files)

### Token usage per query

| Task type | A (brief+read) | B (full dump) | C (no pruner) | A vs B | A vs C |
|-----------|---------------:|-------------:|-------------:|-------:|-------:|
| Cross-package flow | 14,212 | 54,173 | 226,352 | -73.8% | -93.7% |
| Narrow fix (WebSocket) | 16,181 | 56,667 | 152,540 | -71.4% | -89.4% |
| Understanding (pipeline) | 17,829 | 63,631 | 162,757 | -72.0% | -89.0% |
| Cross-cutting (correlation ID) | 7,487 | 68,441 | 190,426 | -89.1% | -96.1% |
| Data flow (auth tokens) | 13,048 | 59,774 | 132,293 | -78.2% | -90.1% |
| **Average** | **13,751** | **60,537** | **172,873** | **-76.9%** | **-91.7%** |

### Key findings

**Brief + read uses 77% fewer tokens than full dump, 92% fewer than no pruner.** The two-phase approach (orient with ~3K token summary, then read source files) consistently beats dumping the full context package into the conversation.

**Cross-cutting tasks benefit most** (-89% vs full, -96% vs no pruner). When the task spans many subsystems but the key files are small, brief mode's targeted file list avoids loading irrelevant snippets and execution paths.

**Even the worst case saves 71%.** The narrow fix query (WebSocket reconnection) produces the smallest gap because the key files are larger (13K tokens), but still beats full dump by 71%.

### Where it helps less

**Exhaustive codebase scans.** Tasks like "find all console.log calls across every package" require grepping the entire codebase regardless. Pruner can't shortcut that.

**Tasks requiring full file reads.** When the LLM needs to read complete files to write correct code (not just understand them), the token savings from brief mode shrink since you're reading those files anyway. Brief mode's value is in telling you *which* files to read.

### Reproduce

```bash
make bench                              # full + brief benchmark against openclaw
python3 tests/ab_test.py                # A/B token comparison (default: openclaw)
python3 tests/ab_test.py /path/to/repo  # test against any repo
```

## Architecture

```
src/
├── main.rs        # Entry point
├── cli.rs         # clap CLI interface (8 commands)
├── db.rs          # SQLite schema and access layer
├── indexer.rs      # Repository walker + tree-sitter parsing -> DB
├── parser.rs       # Tree-sitter based symbol/import/call extraction
├── query.rs        # Keyword extraction + heuristic relevance matching
├── context.rs      # Context package generation (text + JSON)
├── tokens.rs       # Token estimation and usage measurement
└── languages.rs    # Language detection, test classification
```

### Indexing pipeline

1. Walk repository files, skip ignored dirs (node_modules, .git, etc.)
2. Detect language from file extension
3. Parse supported languages with tree-sitter
4. Extract symbols (functions, classes, methods), imports, and call sites
5. Build graph edges: contains, calls, imports, tests
6. Store everything in SQLite with WAL journaling

### Query analysis

1. **Keyword extraction** — stop word removal, camelCase/snake_case splitting, minimum sub-keyword length (4 chars) to avoid overly broad matches
2. **Candidate gathering** — search files by path, symbols by name, signature, callers, and importing files. Expensive cross-reference searches skipped for short keywords
3. **Scoring and ranking** — files scored by keyword match (exact stem, contains, directory), quality (language, minified/bundled detection, directory penalties for docs/locale/vendor/assets), and cross-reference boost (files hosting more matched symbols rank higher). Duplicate filenames penalized. Dynamic score cutoff drops results below 25% of the top score
4. **Symbol scoring** — exact/prefix/substring match + kind bonus (functions rank above variables) + negative file quality propagation (symbols in minified files penalized)
5. **Test discovery** — related tests found via graph edges
6. **Execution path tracing** — recursive CTE through call graph (depth 5), time-budgeted to 10 seconds
7. **Subsystem inference** — top-level directory names from matched files

### Context generation

1. Apply mode limits (brief: 8 files, 15 symbols, 3 shallow paths, 0 snippets; full: uncapped)
2. Extract code snippets from source files (full mode only, capped at 4000 chars per snippet)
3. Output as human-readable text and/or structured JSON

## Supported languages

Full tree-sitter parsing (symbols, imports, calls):

- Python
- JavaScript / TypeScript / TSX / JSX
- Rust

Basic indexing (files, metadata):

- All text files not in the ignore list

## Limitations

- Call graph is best-effort — dynamic dispatch, string-based lookups, and indirect calls are not tracked
- Query analysis uses keyword matching with heuristic scoring, not semantic understanding
- Import resolution is heuristic (module name -> file path mapping)
- Relevance scoring can miss results when keywords don't appear in file paths or symbol names (e.g., a function that handles authentication but is named `validateRequest`)
- On very large repos (10K+ files), full mode produces ~55-70K tokens — use brief mode for orientation

## Claude Code integration

Pruner integrates with Claude Code so that Claude automatically runs `pruner context` before making code changes.

### Setup

**1. Install pruner:**

```bash
cargo install --path /path/to/pruner
```

Verify:

```bash
pruner --version
```

**2. Copy the skill and CLAUDE.md into your target project:**

```bash
# The skill (teaches Claude how to use pruner)
mkdir -p /path/to/your-project/.claude/skills/pruner
cp /path/to/pruner/.claude/skills/pruner/SKILL.md \
   /path/to/your-project/.claude/skills/pruner/SKILL.md

# The CLAUDE.md snippet (tells Claude to use pruner automatically)
cat /path/to/pruner/CLAUDE.template.md >> /path/to/your-project/CLAUDE.md
```

Or install the skill globally:

```bash
mkdir -p ~/.claude/skills/pruner
cp /path/to/pruner/.claude/skills/pruner/SKILL.md \
   ~/.claude/skills/pruner/SKILL.md
```

**3. Index the target repo** (one-time, re-run after major changes):

```bash
pruner index /path/to/your-project -v
```

### What happens automatically

Once set up, when you ask Claude Code to do something like "fix the login flow", Claude will:

1. Run `pruner context . "fix the login flow" --brief` (~3K tokens — auto-indexes if needed)
2. Read the brief output: key files, symbols with locations, execution paths, related tests
3. Open the top 3-5 source files identified by pruner using file:line pointers
4. If the task requires deeper understanding, run full mode or use `pruner show-symbol` for specific call graphs
5. Proceed with the task, informed by focused context

The key insight: brief mode tells the LLM **where to look** (~3K tokens), not **everything about the code** (~60K tokens). The LLM then reads only what it needs.

## Similar projects

Several tools tackle the same problem. The key difference is **how** they deliver context to the LLM.

### MCP server approach (interactive exploration)

| Project | Lang | Description |
|---------|------|-------------|
| [CodeRLM](https://github.com/JaredStewart/coderlm) | Rust | Tree-sitter indexing with `search`, `impl`, `callers`, `tests` API. File watching, multi-IDE. |
| [codebase-memory-mcp](https://github.com/DeusData/codebase-memory-mcp) | Go | Knowledge graph with tree-sitter + SQLite. 64 languages, 14 MCP tools. |
| [Srclight](https://github.com/srclight/srclight) | ? | Tree-sitter + SQLite FTS5, call graphs, optional embeddings. 25+ tools. |

### Batch/CLI approach (pre-packaged context)

| Project | Lang | Description |
|---------|------|-------------|
| **Pruner** | Rust | Tree-sitter + SQLite, NL query -> execution paths + key files + snippets. One command, no server. |
| [Aider repo-map](https://github.com/Aider-AI/aider) | Python | Tree-sitter + PageRank to select most-referenced symbols. Embedded in Aider. |
| [Repomix](https://github.com/yamadashy/repomix) | JS | Packs entire repo into one file. No structural parsing. |

### How pruner differs

- **Standalone CLI, no server** — `pruner context . "task" --brief` and done
- **Natural language -> context package** — takes a task description, infers execution paths, returns a complete package
- **Automatic execution path inference** — traces through the call graph to show how code flows
- **No LLM for indexing** — purely structural
- **Tradeoff** — MCP servers are more flexible (LLM can ask follow-ups), but cost more turns. Pruner is simpler and cheaper per query.

## Future work

- More language parsers (Go, Java, Ruby)
- Semantic search (embeddings) for queries that don't match symbol/file names
- Optional tiktoken integration for exact token counts
