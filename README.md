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

```bash
pruner context . "fix WebSocket reconnection timeout"
pruner context . "add caching to the API" --format json
```

Pruner auto-detects task scope from query results and adjusts output:

| Mode | When | Files | Symbols | Snippets | Tokens |
|------|------|-------|---------|----------|--------|
| **Brief** (auto) | Narrow task: ≤3 files, 1 subsystem | 8 | 15 | 0 | ~3K |
| **Focused** (auto) | Broad task: many files/subsystems | 10 | 20 | 20 | ~10-15K |
| `--full` | Manual override | 25 | 40 | 40 | ~55-70K |
| `--brief` | Manual override | 8 | 15 | 0 | ~3K |

The default (auto) mode is designed for agent use: one call returns everything the LLM needs without follow-up exploration.

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

Real Claude Code (opus) sessions on [openclaw/openclaw](https://github.com/openclaw/openclaw) (9,794 files, 30,695 symbols). Each task run twice: once with pruner skill installed, once vanilla. Sessions run in parallel on separate clones.

### Results (auto mode — current)

Pruner auto-detects task scope and adjusts output. For broad tasks (most queries on large repos), it returns focused context with code snippets (~10-15K tokens). For narrow tasks, brief pointers (~3K tokens). N=1 per task — results have variance.

| Task | Without pruner | With pruner | Δ cost | Δ tool calls | Δ wall time |
|------|---------------:|------------:|-------:|-------------:|------------:|
| Narrow fix (WebSocket) | $0.45 / 27 tools | $0.22 / 7 tools | **-50%** | **-74%** | **-69%** |
| Cross-package flow | $0.61 / 58 tools | $0.48 / 19 tools | **-22%** | **-67%** | **-50%** |
| Understanding (plugins) | $0.40 / 57 tools | $0.35 / 19 tools | **-12%** | **-67%** | **-43%** |
| Data flow (auth) | $0.45 / 55 tools | $0.44 / 54 tools | -2% | -2% | 0% |

### What the data shows

**Pruner saves cost across all tasks.** The worst case (data_flow) is breakeven at -2%. The best case (narrow_fix) saves 50% cost and 69% wall time.

**Tool calls drop dramatically** on exploration-heavy tasks (-67% to -74%). Pruner's pre-computed context replaces grep/glob/read exploration chains.

**Vanilla Claude is unpredictable.** Without pruner, Claude's strategy varies between runs — sometimes efficient (13 tool calls), sometimes expensive (58 tool calls). With pruner, behavior is consistent: 7-19 tool calls.

**Token count is misleading.** Pruner shows higher raw token counts because its output is included in every subsequent API call. But cost depends on cache hits (cheap) vs fresh tokens (expensive). Fewer tool calls = fewer fresh tokens = lower cost.

**Data flow (auth) is the hard case.** Claude with pruner sometimes decides pruner's output isn't enough and spawns a subagent for deep exploration, negating the savings. This happens when the query topic is broad and pruner's keyword matching misses key files.

### When to use pruner

- **Any exploration-heavy task on a large codebase**: Pruner consistently reduces cost, tool calls, and wall time.
- **Cross-package tracing**: Biggest win — 22-50% cheaper, 50-69% faster.
- **Broad understanding questions**: 12% cheaper, 43% faster.
- **Breakeven worst case**: Even when pruner doesn't help much, it doesn't hurt.

### Reproduce

```bash
# Install pruner in PATH first
cargo build --release && ln -sf $(pwd)/target/release/pruner /usr/local/bin/pruner

# Run real A/B test (requires claude CLI, ~$2 per run)
python3 tests/ab_test.py                          # all tasks
python3 tests/ab_test.py --task cross_package      # single task
python3 tests/ab_test.py --task narrow_fix --save-raw  # with raw output
python3 tests/ab_test.py /path/to/repo             # any repo

# Quick benchmark (no claude CLI needed)
make bench
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

1. Auto-detect task scope: narrow (≤3 files, 1 subsystem) → brief; broad → focused
2. Apply mode limits (brief: 8 files, 0 snippets; focused: 10 files, 20 snippets; full: uncapped)
3. Extract code snippets from source files (focused/full modes, capped at 4000 chars per snippet)
4. Graph expansion: files discovered via execution paths added to candidates
5. Output as human-readable text and/or structured JSON

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
- On very large repos (10K+ files), full mode produces ~55-70K tokens — the default auto mode caps output at ~10-15K tokens

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

1. Run `pruner context . "fix the login flow"` (auto-indexes if needed)
2. Pruner auto-detects task scope and returns focused context with code snippets (~10-15K tokens) or brief pointers (~3K tokens)
3. Claude works directly from the output — no grep/glob exploration needed
4. If a snippet is truncated, Claude reads the specific file using the file:line pointers from the output

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

- **Standalone CLI, no server** — `pruner context . "task"` and done
- **Natural language -> context package** — takes a task description, infers execution paths, returns a complete package
- **Automatic execution path inference** — traces through the call graph to show how code flows
- **No LLM for indexing** — purely structural
- **Tradeoff** — MCP servers are more flexible (LLM can ask follow-ups), but cost more turns. Pruner is simpler and cheaper per query.

## Future work

- More language parsers (Go, Java, Ruby)
- Semantic search (embeddings) for queries that don't match symbol/file names
- Optional tiktoken integration for exact token counts
