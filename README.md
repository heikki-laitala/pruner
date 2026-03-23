# Pruner

Synthetic code context engine for LLM coding tasks. Indexes a repository structurally, infers relevant execution paths from natural language asks, and generates compact LLM-ready context packages — all without using an LLM for indexing.

## Install

### Quick install (pre-built binary)

```bash
curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | sh
```

This downloads the latest release binary for your platform (macOS/Linux, x86_64/arm64) and installs it to `~/.local/bin/`.

Options:

```bash
# Install with Claude Code hook (best performance)
curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | sh -s -- --hook --global

# Install to a custom directory
curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | sh -s -- --dir /usr/local/bin

# Install a specific version
curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | sh -s -- --version v0.1.0
```

### Build from source

Requires Rust (1.85+) and a C compiler (for tree-sitter):

```bash
# macOS
xcode-select --install

# Ubuntu/Debian
sudo apt install build-essential

# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then:

```bash
make install
# or: cargo install --path .
```

This builds a release binary and installs it to `~/.cargo/bin/pruner`.

### Set up a project

After installing the binary, set up pruner in your project:

```bash
pruner init /path/to/project          # skill mode (works with any AI agent)
pruner init /path/to/project --hook   # hook mode (Claude Code only, best performance)
pruner init --global                  # install globally to ~/.claude/
pruner index /path/to/project         # index the codebase (re-run after major changes)
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

Real Claude Code (opus) sessions on [openclaw/openclaw](https://github.com/openclaw/openclaw) (9,794 files, 30,695 symbols). Each task run twice: once with pruner installed, once vanilla. Sessions run in parallel on separate clones. It takes around 1 minute to index openclaw codebase.

### Results (prompt-submit hook — current)

Pruner runs as a `UserPromptSubmit` hook that injects context before Claude starts thinking. Zero tool calls for navigation. Pruner auto-detects task scope: focused context with code snippets (~10-15K tokens) for broad tasks, brief pointers (~3K tokens) for narrow tasks. N=1 per task — results have variance.

| Task | Prompt | Without | With | Δ cost | Δ tools | Δ time |
|------|--------|--------:|-----:|-------:|--------:|-------:|
| Narrow fix | "What files handle WebSocket reconnection in this repo? List the file paths and briefly explain what each does." | $0.28 / 16 tools | $0.30 / 15 tools | +6% | -6% | -3% |
| Cross-package | "How does a message flow from a webhook received by an extension to the core message handler in this repo? Trace the path through the key files." | $0.48 / 35 tools | $0.36 / 12 tools | **-24%** | **-66%** | **-46%** |
| Understanding | "How does the plugin/extension loading system work in this repo? What are the key files and entry points?" | $0.33 / 49 tools | $0.22 / 7 tools | **-32%** | **-86%** | **-58%** |
| Data flow | "How does authentication and token validation work in this repo? List the key files and describe the flow." | $0.34 / 42 tools | $0.23 / 10 tools | **-32%** | **-76%** | **-56%** |
| Implement | "Implement a health check endpoint that returns JSON with the server version and uptime. Find where HTTP routes are registered and add it there." | $0.82 / 51 tools | $0.57 / 21 tools | **-30%** | **-59%** | **-53%** |
| Implement (large) | "Add a rate limiting system for incoming messages. Create a RateLimiter class that tracks per-channel message counts with a sliding window. Integrate it into the message routing pipeline. Add configuration options and unit tests." | $1.21 / 86 tools | $0.72 / 29 tools | **-41%** | **-66%** | **-62%** |

### Results (skill mode — previous)

Earlier approach where pruner ran as a skill (1 tool call). Kept for reference.

| Task | Prompt | Without | With | Δ cost | Δ tools | Δ time |
|------|--------|--------:|-----:|-------:|--------:|-------:|
| Narrow fix | "What files handle WebSocket reconnection in this repo? List the file paths and briefly explain what each does." | $0.45 / 27 tools | $0.22 / 7 tools | **-50%** | **-74%** | **-69%** |
| Cross-package | "How does a message flow from a webhook received by an extension to the core message handler in this repo? Trace the path through the key files." | $0.61 / 58 tools | $0.48 / 19 tools | **-22%** | **-67%** | **-50%** |
| Understanding | "How does the plugin/extension loading system work in this repo? What are the key files and entry points?" | $0.40 / 57 tools | $0.35 / 19 tools | **-12%** | **-67%** | **-43%** |
| Data flow | "How does authentication and token validation work in this repo? List the key files and describe the flow." | $0.45 / 55 tools | $0.44 / 54 tools | -2% | -2% | 0% |
| Implement | "Implement a health check endpoint that returns JSON with the server version and uptime. Find where HTTP routes are registered and add it there." | $0.66 / 48 tools | $0.70 / 23 tools | +6% | **-52%** | **-34%** |
| Implement (large) | "Add a rate limiting system for incoming messages. Create a RateLimiter class that tracks per-channel message counts with a sliding window. Integrate it into the message routing pipeline. Add configuration options and unit tests." | $0.98 / 69 tools | $0.64 / 28 tools | **-35%** | **-59%** | **-50%** |

### What the data shows

**Hook mode saves cost on 4 out of 5 tasks.** The prompt-submit hook injects context before Claude starts — zero tool calls for navigation. Cost savings range from -24% to -32% on broad tasks, with the narrow fix being the only breakeven (+6%).

**Tool calls drop dramatically** across all broad tasks (-59% to -86%). Pruner's pre-computed context replaces grep/glob/read exploration chains.

**Data flow is no longer the hard case.** Previously -2% with the skill approach, now -32% with the hook. The hook injects context before Claude decides to spawn its own subagent, preempting the re-exploration pattern.

**Vanilla Claude has high variance.** Without pruner, Claude's strategy varies significantly between runs of the same task. The implement scenario cost $0.66 in one run and $0.82 in another; narrow fix ranged from $0.28 to $0.45. Claude sometimes spawns cheap subagents (2 opus turns + 40 subagent tool calls), sometimes does everything on the main thread (20 opus turns + 50 tool calls). This makes A/B results noisy — N=1 per task means individual numbers can shift ±30%. The directional trend (pruner saves on broad tasks) is consistent across runs, but exact percentages vary. With pruner, behavior is more predictable: 7-29 tool calls across all tasks.

**Token count is misleading.** Pruner shows higher raw token counts because its output is included in every subsequent API call. But cost depends on cache hits (cheap) vs fresh tokens (expensive). Fewer tool calls = fewer fresh tokens = lower cost.

### When to use pruner

- **Large implementation tasks**: 41% cheaper, 62% faster — biggest win. More exploration saved = more value.
- **Any broad task on a large codebase**: 24-32% cheaper, 46-58% faster.
- **Small implementation tasks**: 30% cheaper, 53% faster.
- **Cross-package tracing**: 24% cheaper, 46% faster.
- **Understanding / data flow**: 32% cheaper, 56-58% faster.
- **Narrow tasks**: Breakeven — vanilla Claude is already efficient on focused queries.

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

## Development

### Make targets

| Command | Description |
|---------|-------------|
| `make build` | Debug build |
| `make release` | Release build (optimized) |
| `make install` | Build release and install to `~/.cargo/bin/` |
| `make test` | Run unit + integration tests |
| `make test-unit` | Run unit tests only |
| `make test-integration` | Run integration tests only |
| `make bench` | Run benchmarks on a real repo (clones openclaw, ~2 min) |
| `make lint` | Run clippy with warnings as errors |
| `make format` | Format code with rustfmt |
| `make check` | Lint + test |
| `make clean` | Remove build artifacts and `.pruner/` |
| `make run ARGS="..."` | Run pruner with arguments, e.g. `make run ARGS="index . -v"` |
| `make index` | Index the current repo |

### Cargo equivalents

```bash
cargo build                          # debug build
cargo build --release                # release build
cargo install --path .               # install to ~/.cargo/bin/
cargo test --bin pruner --test integration  # unit + integration tests
cargo test --lib                     # unit tests only
cargo test --test bench -- --nocapture      # benchmarks
cargo clippy -- -D warnings          # lint
cargo fmt                            # format
```

## Limitations

- Call graph is best-effort — dynamic dispatch, string-based lookups, and indirect calls are not tracked
- Query analysis uses keyword matching with heuristic scoring, not semantic understanding
- Import resolution is heuristic (module name -> file path mapping)
- Relevance scoring can miss results when keywords don't appear in file paths or symbol names (e.g., a function that handles authentication but is named `validateRequest`)
- On very large repos (10K+ files), full mode produces ~55-70K tokens — the default auto mode caps output at ~10-15K tokens

## Claude Code integration

Two modes available:

### Hook mode (recommended for Claude Code)

Context is injected automatically before Claude starts thinking — zero tool calls.

```bash
pruner init /path/to/project --hook
pruner index /path/to/project
```

### Skill mode (works with any AI agent)

Claude calls `pruner context` as a tool. Works with Claude Code, Codex, Copilot, etc.

```bash
pruner init /path/to/project
pruner index /path/to/project
```

### Global install

Install once for all projects:

```bash
pruner init --global          # skill mode
pruner init --global --hook   # hook mode
```

### What happens automatically

Once set up, when you ask Claude Code to do something like "fix the login flow":

1. Pruner provides context (injected via hook, or Claude runs `pruner context`)
2. Auto-detects task scope: focused context with code snippets (~10-15K tokens) or brief pointers (~3K tokens)
3. Claude works directly from the output — no grep/glob exploration needed
4. If a snippet is truncated, Claude reads the specific file using the file:line pointers

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
