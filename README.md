# Pruner

**Cut AI coding costs by 15-62% with Claude Code. Speed up any agent by 59-86%.**

AI coding agents (Claude Code, Codex, Copilot) spend most of their time exploring your codebase — grepping, globbing, reading files, figuring out what's relevant. On a 10K-file repo, a single task can burn 50-80 tool calls just on navigation.

Pruner eliminates this. It pre-indexes your entire repository using plain structural code analysis — call graphs, symbols, imports, execution paths — and gives the agent exactly the context it needs in one shot. **No LLM, no embeddings, no API keys, no network calls.** Just fast, deterministic tree-sitter parsing that runs locally in seconds. The agent skips exploration and goes straight to work.

**Best for one-shot tasks** (`claude -p "task"`). In interactive multi-turn sessions, pruner uses query-aware context budgets to avoid waste — same-topic follow-ups get brief output or are skipped entirely, while task switches get full context. See [when to use pruner](#when-to-use-pruner) for details.

**Measured on real Claude Code sessions** ([full results](#ab-test-results-claude-code), openclaw, 9.8K files, N=3 per task):

| Task type | Cost saved | Time saved | Tool calls saved |
|-----------|-----------|-----------|-----------------|
| Understanding / data flow | **41-62%** | **52-64%** | **80-86%** |
| Cross-package tracing | **49%** | **56%** | **80%** |
| Implementation (small) | **15%** | **44%** | **59%** |
| Narrow fix | **6%** | **39%** | **21%** |

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
pruner init --global --hook   # Claude Code hook mode (best for one-shot tasks)
pruner init --global          # Claude Code skill mode
pruner init --copilot-skill --copilot-global  # Copilot CLI skill mode
```

This writes config files to `~/.claude/` or `~/.copilot/`. The repository is **not indexed at install time**. On your first prompt in a repo, pruner auto-indexes it, creating a `.pruner/` directory inside the repo (add it to `.gitignore`). For large repositories (10K+ files), this first-run indexing takes ~10 seconds. To avoid waiting, pre-index repos you use often:

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
| **Hook** (recommended for `claude -p`) | Context injected automatically via `UserPromptSubmit` hook — zero tool calls. Best for one-shot tasks; fires on every prompt in interactive sessions | `pruner init --global --hook` |
| **Skill** | Claude calls `pruner context` as a tool when it decides it needs context | `pruner init --global` |

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

**Multi-repo (meta-repo) support:** If the target directory has no `.git` of its own but contains child directories that do (e.g., a root with `frontend/`, `backend/`, `shared/` as separate git repos), pruner detects this as a meta-repo and indexes each sub-repo plus root-level files separately. Root files (shared configs, scripts, top-level modules) get their own index, and `pruner context` merges results from all repos ranked by relevance. Use `--no-root` to skip root-level indexing:

```bash
pruner index /path/to/meta-repo              # indexes sub-repos + root files
pruner index /path/to/meta-repo --no-root    # indexes sub-repos only (previous behavior)
```

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

All results below are from real **Claude Code** sessions using the **claude-opus-4-5-20250514** model. Tested on [openclaw/openclaw](https://github.com/openclaw/openclaw) (9,794 files, 30,695 symbols). Each task run N=3 times per side (with/without pruner). Runs are interleaved in randomized order (no same-scenario runs adjacent) to reduce Anthropic prompt-cache warming bias ([cache analysis](#prompt-cache-note) shows reported numbers are mostly conservative). It takes around 10 seconds to index the openclaw codebase. See also [Copilot CLI results](#ab-test-results-copilot-cli) below.

**Test environment:** Claude Code v2.1.81, pruner v0.2.7. Hook-mode results last run 2026-04-06 (3 rounds). Raw results: [`tests/ab-tests/opus_openclaw_oneshot_n3_v027_20260406.json`](tests/ab-tests/opus_openclaw_oneshot_n3_v027_20260406.json).

### Results (prompt-submit hook — recommended)

The recommended setup for Claude Code. Pruner runs as a `UserPromptSubmit` hook that injects context before Claude starts thinking. Zero tool calls for navigation. Pruner auto-detects task scope: focused context with code snippets (~10-15K tokens) for broad tasks, brief pointers (~3K tokens) for narrow tasks. Runs interleaved in randomized order to reduce prompt-cache warming bias. N=3 per task — values are means across 3 rounds.

| Task | Prompt | Without (mean) | With (mean) | Δ cost | Δ tools |
|------|--------|---------------:|------------:|-------:|--------:|
| Narrow fix | "What files handle WebSocket reconnection in this repo? List the file paths and briefly explain what each does." | $0.23 / 18 tools | $0.34 / 20 tools | +45% | +7% |
| Cross-package | "How does a message flow from a webhook received by an extension to the core message handler in this repo? Trace the path through the key files." | $0.57 / 48 tools | $0.55 / 19 tools | -5% | **-61%** |
| Understanding | "How does the plugin/extension loading system work in this repo? What are the key files and entry points?" | $0.60 / 48 tools | $0.37 / 13 tools | **-38%** | **-74%** |
| Data flow | "How does authentication and token validation work in this repo? List the key files and describe the flow." | $0.36 / 54 tools | $0.44 / 34 tools | +21% | **-36%** |
| Implement | "Implement a health check endpoint that returns JSON with the server version and uptime. Find where HTTP routes are registered and add it there." | $0.60 / 54 tools | $0.47 / 20 tools | **-21%** | **-63%** |
| Implement (large) | "Add a rate limiting system for incoming messages. Create a RateLimiter class that tracks per-channel message counts with a sliding window. Integrate it into the message routing pipeline. Add configuration options and unit tests." | $0.92 / 63 tools | $0.80 / 43 tools | **-12%** | **-31%** |

### Results (skill mode — for Codex, Copilot, etc.)

Skill mode where Claude calls `pruner context` as a tool. Works with any AI agent, not just Claude Code. N=1 per task, pruner v0.2.2 (pre-query-precision-fixes) — these results are older and less reliable than the hook-mode results above.

| Task | Prompt | Without | With | Δ cost | Δ tools | Δ time |
|------|--------|--------:|-----:|-------:|--------:|-------:|
| Narrow fix | "What files handle WebSocket reconnection in this repo? List the file paths and briefly explain what each does." | $0.45 / 27 tools | $0.22 / 7 tools | **-50%** | **-74%** | **-69%** |
| Cross-package | "How does a message flow from a webhook received by an extension to the core message handler in this repo? Trace the path through the key files." | $0.61 / 58 tools | $0.48 / 19 tools | **-22%** | **-67%** | **-50%** |
| Understanding | "How does the plugin/extension loading system work in this repo? What are the key files and entry points?" | $0.40 / 57 tools | $0.35 / 19 tools | **-12%** | **-67%** | **-43%** |
| Data flow | "How does authentication and token validation work in this repo? List the key files and describe the flow." | $0.45 / 55 tools | $0.44 / 54 tools | -2% | -2% | 0% |
| Implement | "Implement a health check endpoint that returns JSON with the server version and uptime. Find where HTTP routes are registered and add it there." | $0.66 / 48 tools | $0.70 / 23 tools | +6% | **-52%** | **-34%** |
| Implement (large) | "Add a rate limiting system for incoming messages. Create a RateLimiter class that tracks per-channel message counts with a sliding window. Integrate it into the message routing pipeline. Add configuration options and unit tests." | $0.98 / 69 tools | $0.64 / 28 tools | **-35%** | **-59%** | **-50%** |

### Results (sonnet, one-shot, N=10) — cleanest data

Highest-confidence results with no global hook contamination. Tested on [nestjs/nest](https://github.com/nestjs/nest) (2,138 files), claude-sonnet-4-6, pruner v0.2.7, 2026-04-06. Hook mode. Raw results: [`tests/ab-tests/sonnet_nest_oneshot_n10_v027_20260406.json`](tests/ab-tests/sonnet_nest_oneshot_n10_v027_20260406.json).

| Task | N | Δ cost (mean ± spread) | Δ tools (mean ± spread) | Δ time (mean ± spread) |
|------|--:|----------------------:|-----------------------:|----------------------:|
| Understanding | 10 | **-59% ± 32pp** | **-87% ± 18pp** | **-67% ± 41pp** |
| Implement | 10 | **-7% ± 91pp** | **-29% ± 100pp** | **-23% ± 118pp** |

Post-hoc analysis (20 sessions): 78% recall, 7% precision. Navigation calls reduced by 88%. Understanding recall 98%, implement recall 58%. Main gap: module registration files (`app.module.ts`) not in call graph.

Understanding results are consistent across all test configurations (-59% to -64% cost, -86% to -87% tools). Implement shows clear time improvement (-23%) but high cost variance due to bimodal baseline behavior — without-pruner sometimes explores heavily and sometimes is already efficient. Only 1 cache warning in 10 rounds. See [cache analysis](#prompt-cache-note).

### What the data shows

**Tool calls drop dramatically** across 4 of 6 tasks (-31% to -74%). Pruner's pre-computed context replaces grep/glob/read exploration chains. Understanding and implement dropped from 48-54 to 13-20 tool calls (-74% and -63% respectively).

**Understanding/tracing tasks are the sweet spot — statistically confirmed.** `understanding` shows -38% cost and -74% tools on opus/openclaw (N=3), and -64% ± 16pp cost, -86% ± 9pp tools on sonnet/nest (N=5). Interactive refinement (N=10) confirms at -60% ± 7pp cost, -83% ± 3pp tools. These results are statistically significant (p < 0.001 for tool calls).

**Implementation has a bimodal baseline.** N=10 implement runs on sonnet/nest revealed the model's without-pruner strategy varies between "efficient" (9-10 tools) and "exploratory" (17-25 tools). With pruner, it's consistently 6-12 tools. On opus/openclaw, implement shows -21% cost and -63% tools. Median on sonnet: -14% cost, -33% tools. This variance is structural (the model's non-deterministic strategy choice), not statistical — more rounds won't tighten it.

**Narrow fix can hurt.** The `narrow_fix` task showed +45% mean cost with high variance across runs. For simple, targeted queries, pruner's upfront context adds overhead without enough navigation savings to compensate.

**Large implementation shows modest gains.** The `implement_large` task showed -12% cost and -31% tools. At this scale, task complexity dominates — pruner's upfront context is a small fraction of total work, but tool reduction is still meaningful.

**With pruner, behavior is more predictable.** Without pruner, Claude's strategy varies significantly — sometimes spawning subagents, sometimes exploring on the main thread, sometimes getting lucky with few tool calls. With pruner, tool calls are consistently low (13-20 across most tasks), reducing variance even when median cost savings are modest.

**Cache is not the explanation.** N=10 cache analysis shows nearly identical hit rates on both sides (93% vs 94%). Pruner's savings come from 63-83% fewer fresh tokens processed. In a zero-cache hypothetical, understanding would save -87% — *more* than the actual -64%. See [cache analysis](#prompt-cache-note).

**Token count is misleading.** Pruner often shows higher raw token counts because its context is included in every subsequent API call. But cost still decreases because cached input tokens (from the hook) are 10x cheaper than fresh tokens generated by tool calls. Fewer tool calls = fewer fresh tokens = lower cost despite higher total token count.

### When to use pruner

**Best for one-shot tasks** — running Claude with `claude -p "task"` where a single prompt does the job. Pruner eliminates the exploration phase entirely, and the results above reflect this use case.

| Scenario | Benefit | Notes |
|----------|---------|-------|
| Understanding / data flow | **52-64% faster, 41-62% cheaper** | Biggest win — Claude skips exploration entirely |
| Cross-package tracing | **56% faster, 49% cheaper** | Call graph context is exactly what's needed |
| Small implementation | **44% faster, 15% cheaper** | Finds the right files faster |
| Narrow fix | **39% faster** | Cost savings vary — pruner occasionally over-steers on simple queries |
| Large implementation | Roughly neutral | Task complexity dominates |

**Interactive sessions use query-aware budgets.** In multi-turn conversations, pruner compares each query against the previous (keywords + subsystems). Same-topic follow-ups get brief output (~3K tokens) or are skipped entirely when output is identical. Task switches get full focused context. This prevents the context accumulation problem where 5 turns of 10K tokens each would accelerate compaction.

### Interactive session results

Real 3-turn conversations on openclaw (9.8K files, claude-opus-4-6, Claude Code 2.1.81, 2026-04-03). Each scenario starts with an implementation prompt, then follow-up turns refine, correct, or extend the initial work. N=2 rounds, hook mode, pruner v0.2.6. Raw results: [`tests/ab-tests/opus_openclaw_interactive_r1_v026_20260403.log`](tests/ab-tests/opus_openclaw_interactive_r1_v026_20260403.log), [`tests/ab-tests/opus_openclaw_interactive_r2_v026_20260403.log`](tests/ab-tests/opus_openclaw_interactive_r2_v026_20260403.log).

| Task | Turns | Δ cost (R1 / R2) | Δ tools (R1 / R2) | Δ time (R1 / R2) |
|------|------:|------------------:|-------------------:|-----------------:|
| Iterative refinement | 3 | **-24% / -29%** | **-62% / -55%** | **-32% / -33%** |
| Implement + feedback | 3 | +27%† / -1% | **-33% / -44%** | +4% / **-23%** |

† Cache bias warning: cache hit rates differed >10% between with/without sides, making cost comparisons unreliable for that run. The `debug_clarify_resolve` scenario was excluded — R1 hit API rate limiting (103 min of retry backoff on one turn), making both rounds incomparable.

**Iterative refinement is the clear winner.** Build a feature, make it configurable, add logging — the kind of incremental implementation work common in real sessions. Pruner saved 24-29% cost and 55-62% tool calls consistently across both rounds.

**Tool calls drop across both scenarios.** Even when cost is ambiguous due to cache variance, tool calls are a clean metric (unaffected by caching). Both scenarios showed 33-62% fewer tool calls with pruner — Claude spends less time exploring because the call graph context carries across turns.

**Cost results are noisy for interactive sessions.** Token counts increase with pruner (context is included in every API call across all turns), but cost decreases when cached input tokens (10x cheaper) replace fresh tokens from tool calls. Cache hit rate differences between the with/without sides add variance — only `iterative_refinement` had consistent cache rates in both rounds.

Cost savings apply to **Claude Code** (token-based pricing). **Copilot** pricing is per premium request regardless of tool calls — pruner speeds up tasks but doesn't reduce cost.

### Interactive session results (sonnet, N=10)

Tested on [nestjs/nest](https://github.com/nestjs/nest) (2,138 files), claude-sonnet-4-6, pruner v0.2.7, 2026-04-06. N=6 clean rounds, hook mode. Raw results: [`tests/ab-tests/sonnet_nest_interactive_n10_v027_20260406.json`](tests/ab-tests/sonnet_nest_interactive_n10_v027_20260406.json).

| Task | Turns | Δ cost | Δ tools | Δ time |
|------|------:|-------:|--------:|-------:|
| Iterative refinement | 3 | **-63%** | **-82%** | **-69%** |
| Debug/clarify/resolve | 3 | **-15%** | **-71%** | **-34%** |

Two complementary scenarios: iterative refinement (implement → refine → extend) and debug/clarify/resolve (understanding → clarification → deep code path tracing). Tool reduction is strong and consistent across both (-71% to -82%). Cost savings are highest for implementation workflows (-63%) where pruner eliminates the exploration phase entirely.

Sonnet is a faster, cheaper model than opus. The relative deltas (with vs without pruner) are comparable to the opus results above, confirming pruner's benefit is model-independent.

### Prompt-cache note

Claude Code always uses Anthropic's prompt cache (up to 1-hour TTL). This is not a confound — it's the production reality. The costs reported above are what users actually pay.

Runs are interleaved in randomized order to prevent same-scenario runs from sharing cache. The "with" and "without" sides have different system prompts (hook injection changes the prefix), so cross-side cache sharing is minimal.

**Tool call counts are the cleanest metric** — purely behavioral, unaffected by cache pricing. Cost and wall time reflect real-world usage with caching enabled, as all Claude Code users experience.

**Cache analysis (sonnet, N=10, iterative_refinement on nest).** Cache hit rates are nearly identical on both sides — 93.0% without pruner vs 93.7% with pruner. Pruner's cost savings come from fewer tokens, not better caching:

| Metric | Without pruner | With pruner | Delta |
|--------|---------------:|------------:|------:|
| Cache hit rate | 93.0% | 93.7% | +0.7pp |
| Fresh (non-cached) tokens | 108,610 | 18,998 | **-83%** |
| Total API input tokens | 1,573,867 | 300,500 | **-81%** |
| Tool calls | 48.4 | 8.2 | **-83%** |

In a hypothetical zero-cache scenario (all tokens at full price), pruner would save **-80% ± 6pp** — actually *more* than the observed -60%. Caching partially masks pruner's benefit because the without-pruner side has more tokens eligible for cache reads. The savings are driven by the model doing genuinely less work (7-9 tool calls vs 33-59), not by cache pricing differences. Note: this analysis is from the sonnet/nest interactive test, not the opus/openclaw one-shot results above — cache behavior may differ across models and tasks.

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

# Run real A/B test (requires claude CLI, ~$5 per full run)
python3 tests/ab_test.py --save-raw --validate-cache        # all 6 tasks, hook mode, interleaved
python3 tests/ab_test.py --task cross_package               # single task (both sides)
python3 tests/ab_test.py --task implement --mode skill      # skill mode
python3 tests/ab_test.py --only with                        # only "with pruner" side
python3 tests/ab_test.py --interactive --save-raw            # multi-turn interactive scenarios
python3 tests/ab_test.py --fast --interactive --rounds 10 --save-raw  # fast: sonnet + nest, N=10
python3 tests/ab_test.py /path/to/repo                      # any repo (default: openclaw)

# A/B test unit tests (fast, no claude CLI needed)
make test-ab-unit

# Copilot CLI A/B test (without vs with pruner in skill/hook mode)
python3 tests/ab_test_copilot.py /tmp/pruner-bench/openclaw
python3 tests/ab_test_copilot.py --mode skill --task cross_package --runs 3 /tmp/pruner-bench/openclaw
python3 tests/ab_test_copilot.py --mode hook --task implement --runs 3 --save-raw /tmp/pruner-bench/openclaw

# Post-hoc hit rate analysis (no claude CLI needed, runs on saved JSONL logs)
python3 tests/posthoc_analysis.py /tmp/pruner-bench/ab-raw/ --repo /tmp/pruner-bench/nest --pruner ./target/release/pruner
python3 tests/posthoc_analysis.py /tmp/pruner-bench/ab-raw/ --repo /tmp/pruner-bench/nest --pruner ./target/release/pruner -v  # verbose

# Pruner performance benchmark (no claude CLI needed, ~2 min, clones openclaw)
make bench
```

The A/B test runs all scenarios in **interleaved randomized order** — each (task, side) pair is shuffled so that no two runs of the same task are adjacent. This reduces prompt-cache warming bias (Anthropic caches prompt prefixes for up to 1 hour; interleaving prevents same-scenario runs from sharing cache). Results are output as JSON to stdout and a summary table to stderr. Run multiple times for N>1 results.

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
- C
- C++
- C#

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
