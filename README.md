# Pruner

**Cut AI coding costs by 24-41% with Claude Code. Speed up any agent by 46-62%.**

AI coding agents (Claude Code, Codex, Copilot) spend most of their time exploring your codebase — grepping, globbing, reading files, figuring out what's relevant. On a 10K-file repo, a single task can burn 50-80 tool calls just on navigation.

Pruner eliminates this. It pre-indexes your entire repository using plain structural code analysis — call graphs, symbols, imports, execution paths — and gives the agent exactly the context it needs in one shot. **No LLM, no embeddings, no API keys, no network calls.** Just fast, deterministic tree-sitter parsing that runs locally in seconds. The agent skips exploration and goes straight to work.

**Measured on real Claude Code sessions** ([full results](#ab-test-results-claude-code), openclaw, 9.8K files):

| Task type | Cost saved | Time saved | Tool calls saved |
|-----------|-----------|-----------|-----------------|
| Large feature implementation | **41%** | **62%** | **66%** |
| Understanding / data flow | **32%** | **56-58%** | **76-86%** |
| Small implementation | **30%** | **53%** | **59%** |
| Cross-package tracing | **24%** | **46%** | **66%** |

Works with **Claude Code** (recommended, via prompt-submit hook), **Codex**, **Copilot**, or any agent that can run a CLI command. Claude Code users save on both cost and time. Copilot users save time ([Copilot results](#ab-test-results-copilot-cli)) — Copilot pricing is per premium request regardless of tool calls, so pruner speeds up tasks without affecting cost.

## Install

```bash
curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | bash
```

The installer downloads the binary and walks you through setup — which agent (Claude Code, Copilot, or both) and install mode (global or per-project).

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.ps1 | iex
```

For CI or non-interactive use, pass flags to skip the prompts:

```bash
# Claude Code hook mode, global (recommended)
curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | bash -s -- --hook --global

# Copilot CLI skill, global
curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | bash -s -- --copilot-skill --copilot-global

# Just install the binary, no setup
curl -sSf https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.sh | bash -s -- --no-interactive
```

<details>
<summary>Build from source</summary>

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

</details>

## Setup

Two approaches: **global** (install once, works in every repo) or **per-project** (adds config files to the repo). The install script handles this interactively, but you can also run `pruner init` manually.

### Global install (recommended)

Install once — pruner works automatically in every git repository:

```bash
pruner init --global --hook   # Claude Code hook mode (best performance)
pruner init --global          # Claude Code skill mode
pruner init --copilot-skill --copilot-global  # Copilot CLI skill mode
```

This writes config files to `~/.claude/` or `~/.copilot/` — nothing is added to your repositories. The repository is **not indexed at install time**. On your first prompt in a repo, pruner auto-indexes it, creating a `.pruner/` directory. For large repositories (10K+ files), this first-run indexing can take 30-60 seconds. To avoid waiting, pre-index repos you use often:

```bash
pruner index /path/to/project
```

Add `.pruner/` to your `.gitignore` (global install does not modify it automatically):

```bash
echo '.pruner/' >> .gitignore
```

### Per-project install

Adds pruner config files directly to the repository (useful for teams):

```bash
pruner init /path/to/project --hook          # Claude Code hook mode
pruner init /path/to/project                 # Claude Code skill mode
pruner init /path/to/project --copilot-skill # Copilot CLI skill mode
pruner init /path/to/project --copilot-hook  # Copilot CLI hook mode
```

This creates config files inside the repo (`.claude/` or `.copilot/`), updates `.gitignore` to exclude `.pruner/`, and auto-indexes the project.

### Claude Code integration

Two modes available:

| Mode | How it works | Setup |
|------|-------------|-------|
| **Hook** (recommended) | Context injected automatically via `UserPromptSubmit` hook — zero tool calls | `pruner init --global --hook` |
| **Skill** | Claude calls `pruner context` as a tool when it needs context | `pruner init --global` |

**What gets installed (global):**

| File | Purpose |
|------|---------|
| `~/.claude/skills/pruner/SKILL.md` | Skill definition — tells Claude how to use pruner |
| `~/.claude/hooks/pruner-context.sh` | Hook script (hook mode only) |
| `~/.claude/settings.json` | Hook configuration (hook mode only) |

**Note on global skill mode:** Global install does not modify the repository's `CLAUDE.md`. In skill mode, Claude relies on auto-invocation from the skill description alone. For more reliable behavior, run `pruner init /path/to/project` on repos where you want the extra guidance — this adds a pruner section to `CLAUDE.md` and is safe to run on repos that already have the global skill. Hook mode does not have this limitation since the hook fires automatically regardless of `CLAUDE.md`.

**What gets installed (per-project):**

| File | Purpose |
|------|---------|
| `.claude/skills/pruner/SKILL.md` | Skill definition |
| `.claude/hooks/pruner-context.sh` | Hook script (hook mode only) |
| `.claude/settings.json` | Hook configuration (hook mode only) |
| `CLAUDE.md` | Pruner usage guidance (created if missing) |
| `.gitignore` | `.pruner/` entry added |

**What happens at runtime:** When you start Claude Code in a git repository and ask a question, pruner auto-indexes the repo (creating `.pruner/` with a SQLite database), then returns relevant context. On subsequent prompts, incremental indexing updates only changed files. The `.pruner/` directory is only created inside git repositories — pruner skips non-repo directories.

**Note for global install:** `.gitignore` is not modified automatically. Add `.pruner/` to each repo's `.gitignore` (or your global gitignore) to avoid committing the index.

### Copilot CLI integration

| Mode | How it works | Setup |
|------|-------------|-------|
| **Skill** | Copilot calls `pruner context` as a tool | `pruner init --copilot-skill --copilot-global` |
| **Hook** (experimental) | Background hook writes `.pruner/copilot-context.md` | `pruner init /path/to/project --copilot-hook` |

**Skill mode** creates:
- `.copilot/skills/pruner/SKILL.md` (global: `~/.copilot/`)
- `.github/copilot-instructions.md` guidance (or `~/.copilot/copilot-instructions.md` for global)

Then in Copilot CLI:

```text
/skills add pruner
/skills run pruner "fix login token refresh bug"
```

**Hook mode** (per-project only) creates:
- `.github/hooks/pruner-context.json` + `.sh` + `.ps1`

Requires `--experimental` flag in Copilot CLI 1.0.x. The hook runs on `userPromptSubmitted` and writes `.pruner/copilot-context.md`.

**Note:** Copilot's `userPromptSubmitted` hook is observational — the model doesn't wait for it to complete before starting. On large repos, the model may start exploring before the context file is written. For reliable results, use **skill mode**.

### What happens automatically

Once set up, when you ask the agent to do something like "fix the login flow":

1. Pruner provides context (injected via hook, or the agent runs `pruner context`)
2. Auto-detects task scope: focused context with code snippets (~10-15K tokens) or brief pointers (~3K tokens)
3. The agent works directly from the output — no grep/glob exploration needed
4. If a snippet is truncated, the agent reads the specific file using the file:line pointers

## CLI reference

### Index a repository

```bash
pruner index /path/to/repo
pruner index .              # current directory
pruner index . -v           # verbose output
```

This creates a `.pruner/` directory inside the repo containing the SQLite index database.

**Indexing is automatic.** You don't need to run `pruner index` manually — `pruner context` auto-indexes on first run if no index exists. After that, it runs incremental updates when the index is older than 5 minutes (checks for new, modified, and deleted files). Override with `PRUNER_RECHECK_SECS=0` to force a check every time.

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

### Estimate token savings

```bash
pruner estimate . "fix login flow" --show-steps
```

Simulates a Claude Code session with and without pruner to estimate token savings.

### Check installation status

```bash
pruner status              # show global integrations
pruner status /path/to/repo  # show global + per-project integrations
```

Shows which integrations are installed (Claude/Copilot skills, hooks, CLAUDE.md), index age, and .gitignore status.

### Upgrade and uninstall

```bash
pruner upgrade              # upgrade to latest release
pruner upgrade --check      # check if update available (no changes)
pruner upgrade --version v0.1.5  # install specific version

pruner uninstall            # remove global integrations + binary
pruner uninstall /path/to/repo   # remove per-project integration
pruner uninstall /path/to/repo --purge  # also remove .pruner/ index
```

### Inspect the index

```bash
pruner show-file . src/auth.py
pruner show-symbol . login
pruner stats .
```

## A/B test results (Claude Code)

All results below are from real **Claude Code** sessions using the **opus** model. Tested on [openclaw/openclaw](https://github.com/openclaw/openclaw) (9,794 files, 30,695 symbols). Each task run twice: once with pruner installed, once vanilla Claude Code. Sessions run sequentially on separate clones. It takes around 1 minute to index openclaw codebase. See also [Copilot CLI results](#ab-test-results-copilot-cli) below.

### Results (prompt-submit hook — recommended)

The recommended setup for Claude Code. Pruner runs as a `UserPromptSubmit` hook that injects context before Claude starts thinking. Zero tool calls for navigation. Pruner auto-detects task scope: focused context with code snippets (~10-15K tokens) for broad tasks, brief pointers (~3K tokens) for narrow tasks. N=1 per task — results have variance.

| Task | Prompt | Without | With | Δ cost | Δ tools | Δ time |
|------|--------|--------:|-----:|-------:|--------:|-------:|
| Narrow fix | "What files handle WebSocket reconnection in this repo? List the file paths and briefly explain what each does." | $0.28 / 16 tools | $0.30 / 15 tools | +6% | -6% | -3% |
| Cross-package | "How does a message flow from a webhook received by an extension to the core message handler in this repo? Trace the path through the key files." | $0.48 / 35 tools | $0.36 / 12 tools | **-24%** | **-66%** | **-46%** |
| Understanding | "How does the plugin/extension loading system work in this repo? What are the key files and entry points?" | $0.33 / 49 tools | $0.22 / 7 tools | **-32%** | **-86%** | **-58%** |
| Data flow | "How does authentication and token validation work in this repo? List the key files and describe the flow." | $0.34 / 42 tools | $0.23 / 10 tools | **-32%** | **-76%** | **-56%** |
| Implement | "Implement a health check endpoint that returns JSON with the server version and uptime. Find where HTTP routes are registered and add it there." | $0.82 / 51 tools | $0.57 / 21 tools | **-30%** | **-59%** | **-53%** |
| Implement (large) | "Add a rate limiting system for incoming messages. Create a RateLimiter class that tracks per-channel message counts with a sliding window. Integrate it into the message routing pipeline. Add configuration options and unit tests." | $1.21 / 86 tools | $0.72 / 29 tools | **-41%** | **-66%** | **-62%** |

### Results (skill mode — for Codex, Copilot, etc.)

Skill mode where Claude calls `pruner context` as a tool. Works with any AI agent, not just Claude Code.

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

- **Large implementation tasks**: 62% faster, 41% cheaper (Claude Code) — biggest win. More exploration saved = more value.
- **Any broad task on a large codebase**: 46-58% faster, 24-32% cheaper (Claude Code).
- **Small implementation tasks**: 53% faster, 30% cheaper (Claude Code).
- **Cross-package tracing**: 46% faster, 24% cheaper (Claude Code).
- **Understanding / data flow**: 56-58% faster, 32% cheaper (Claude Code).
- **Narrow tasks**: Breakeven — the agent is already efficient on focused queries.

Cost savings apply to **Claude Code** (token-based pricing). **Copilot** pricing is per premium request regardless of tool calls — pruner speeds up tasks but doesn't reduce cost.

## A/B test results (Copilot CLI)

### Results (skill mode — Copilot runs pruner as a tool)

Tested with **Copilot CLI** using the **gpt-5.3-codex** model on the same openclaw repo (9,794 files). The "with" side prompt instructs Copilot to run `pruner context` and use the output before exploring. Each task run once per side, sequentially. Repo pinned to a fixed commit for reproducibility. N=1 — results have variance.

| Task | Without | With (skill) | Δ tools | Δ time | Premium requests |
|------|--------:|-------------:|--------:|-------:|----------------:|
| Understanding | 72 tools / 242s | 45 tools / 203s | **-38%** | **-16%** | 1 → 1 |
| Cross-package | 102 tools / 391s | 71 tools / 296s | **-30%** | **-24%** | 1 → 1 |
| Data flow | 90 tools / 338s | 57 tools / 271s | **-37%** | **-20%** | 1 → 1 |
| Narrow fix | 48 tools / 252s | 103 tools / 402s | +115% | +59% | 1 → 1 |

Broad tasks (understanding, cross-package, data flow) see 16-24% faster completion and 30-38% fewer tool calls. Narrow tasks can regress — the extra context overhead outweighs the benefit when Copilot is already efficient on focused queries. Premium requests are 1 per session regardless of tool count, so pruner provides **speed improvements** but not cost savings for Copilot.

### Results (hook mode — background hook writes context file)

Same setup, but pruner runs as a `userPromptSubmitted` hook that writes `.pruner/copilot-context.md`. The model reads the file and uses it as starting context.

| Task | Without | With (hook) | Δ tools | Δ time | Premium requests |
|------|--------:|------------:|--------:|-------:|----------------:|
| Understanding | 94 tools / 293s | 45 tools / 201s | **-52%** | **-31%** | 1 → 1 |

Hook and skill mode produce similar "with pruner" numbers (45 tools in both). The larger delta in hook mode reflects natural variance in the "without" baseline across runs.

**Context trust is model-dependent.** Whether hook or skill mode works better depends on how much the model trusts externally provided context. In skill mode, the model calls `pruner context` itself — it requested the data, so it's more likely to use it directly. In hook mode, the context appears as a pre-generated file the model didn't ask for, and more capable models (like gpt-5.3-codex) tend to second-guess it, reading the file but then re-exploring with 50-70 tool calls anyway. Simpler models are often more instruction-following and accept the provided context at face value, making hook mode more effective for them. The instructions must explain what pruner is (tree-sitter indexer, call graph, full codebase index) so the model understands the context is authoritative, not cached notes — without this explanation, even compliant models may distrust the file. Note: Copilot's `glob` tool skips dotfiles, so instructions must tell the model to use `cat` (not glob) to read `.pruner/copilot-context.md`.

### Reproduce

```bash
# Install pruner
make install

# Run real A/B test (requires claude CLI, ~$2 per run)
python3 tests/ab_test.py                                    # all tasks, hook mode
python3 tests/ab_test.py --task cross_package               # single task
python3 tests/ab_test.py --task implement --mode skill      # skill mode
python3 tests/ab_test.py --task narrow_fix --save-raw       # save raw output
python3 tests/ab_test.py /path/to/repo                      # any repo

# Copilot CLI A/B test (without vs with pruner in skill/hook mode)
python3 tests/ab_test_copilot.py /tmp/pruner-bench/openclaw
python3 tests/ab_test_copilot.py --mode skill --task cross_package --runs 3 /tmp/pruner-bench/openclaw
python3 tests/ab_test_copilot.py --mode hook --task implement --runs 3 --save-raw /tmp/pruner-bench/openclaw

# Pruner performance benchmark (no claude CLI needed, ~2 min, clones openclaw)
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

### Supported languages

Full tree-sitter parsing (symbols, imports, calls):

- Python
- JavaScript / TypeScript / TSX / JSX
- Rust
- Go
- Java

Basic indexing (files, metadata):

- All text files not in the ignore list

### Indexing pipeline

1. Walk repository files, skip ignored dirs (node_modules, .git, etc.)
2. Detect language from file extension
3. Parse supported languages with tree-sitter
4. Extract symbols (functions, classes, methods), imports, and call sites
5. Build graph edges: contains, calls, tests. Store imports separately
6. Store everything in SQLite with WAL journaling

**Incremental updates:** On subsequent runs, pruner compares file modification times against the index. Only new/modified files are re-parsed; deleted files are removed. If the index was checked within the last 5 minutes, the walk is skipped entirely.

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

### Limitations

- Call graph is best-effort — dynamic dispatch, string-based lookups, and indirect calls are not tracked
- Query analysis uses keyword matching with heuristic scoring, not semantic understanding
- Import resolution is heuristic (module name -> file path mapping)
- Relevance scoring can miss results when keywords don't appear in file paths or symbol names (e.g., a function that handles authentication but is named `validateRequest`)
- On very large repos (10K+ files), full mode produces ~55-70K tokens — the default auto mode caps output at ~10-15K tokens

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
