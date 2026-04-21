# TODO

## Current status

Install flow is streamlined: `install.sh` + `pruner init` + `pruner index`.

A/B test infrastructure is solid: cache-aware warmup runs, `--validate-cache` flag, interleaved scheduling, `--baseline-branch` for feature impact measurement, and `--multi-turn` for interactive conversation scenarios.

**One-shot results** on openclaw (9.8K files, opus, N=3, v0.2.7, 2026-04-06):

| Task | Δ cost | Δ tools |
|------|--------|---------|
| Understanding | -38% | -74% |
| Cross-package | -5% | -61% |
| Implement | -21% | -63% |
| Data flow | +21% | -36% |
| Implement large | -12% | -31% |
| Narrow fix | +45% | +7% |

**Clean one-shot results** on NestJS/nest (2.1K files, sonnet, N=10, v0.2.7, 2026-04-06). No global hook contamination.

| Task | Δ cost | Δ tools | Δ time |
|------|--------|---------|--------|
| Understanding | -59% ± 32pp | -87% ± 18pp | -67% ± 41pp |
| Implement | -7% ± 91pp | -29% ± 100pp | -23% ± 118pp |

Post-hoc analysis (N=20 sessions): 78% recall, 7% precision. Navigation calls -88%. Understanding recall 98%, implement recall 58%. Main gap: module registration files (`app.module.ts`) not in call graph.

**Interactive sessions are usable** after query-aware budget (TODO #5). N=2 rounds, 3-turn conversations on openclaw (v0.2.6):

| Task | Δ cost (R1 / R2) | Δ tools (R1 / R2) | Δ time (R1 / R2) |
|------|------------------:|-------------------:|-----------------:|
| Iterative refinement | -24% / -29% | -62% / -55% | -32% / -33% |
| Implement + feedback | +27%† / -1% | -33% / -44% | +4% / -23% |

† Cache bias. Tool call reduction is consistent (33-62%); cost/time benefits are clear for iterative work, mixed for other patterns. Turn 0 captures most of the value (equivalent to one-shot). Follow-up turns get brief or skipped output, preventing the context accumulation problem seen before budget control.

**Statistically significant interactive results** on NestJS/nest (2.1K files, sonnet, N=10, v0.2.6, 2026-04-03):

| Task | Δ cost | Δ tools | Δ time |
|------|--------|---------|--------|
| Iterative refinement | -60% ± 7pp | -83% ± 3pp | -69% ± 12pp† |

† 1 of 10 rounds excluded (API rate limiting outlier: 1202s vs normal 35-43s). Tool calls are the cleanest metric: -83% with only 3pp standard deviation across 10 rounds.

**Clean interactive results** on NestJS/nest (2.1K files, sonnet, N=10, v0.2.7, 2026-04-06):

| Task | Δ cost | Δ tools | Δ time |
|------|--------|---------|--------|
| Iterative refinement | -63% | -82% | -69% |
| Debug/clarify/resolve | -15% | -71% | -34% |

N=6 clean rounds (7 completed, round 1 excluded for cache asymmetry, round 8 timed out). Iterative refinement (implement → refine → extend) shows strong wins. Debug/clarify/resolve (understanding → clarification → deep trace) shows clear tool reduction with moderate cost savings.

## High priority

### 4. IDE/session context awareness

Claude Code auto-injects: IDE selection (selected lines), open file path, LSP diagnostics, CLAUDE.md, and changed files diff. Pruner is blind to all of this, so it can't bias results toward what the user is looking at.

**Why this matters:**

- If the user is staring at `src/utils/auth.ts`, pruner should bias toward auth-related call chains
- If CLAUDE.md describes the architecture, pruner's output may duplicate it
- Files the model will already see from IDE injection don't need snippets — just the surrounding call graph

**Implementation:**

- Accept `--open-file <path>` flag from hook script. Boost files in the same module/package/directory
- Accept `--selected-text <text>` flag. Extract symbols from selection and add them as query terms
- Read CLAUDE.md if present, extract directory/subsystem hints to boost scoring
- Skip snippets for files already covered by IDE selection


### 7. Pruner output competes with 40+ attachment types

Claude Code injects context from 40+ sources per turn: IDE selection, open files, LSP diagnostics, CLAUDE.md hierarchy, changed files, todo/task reminders, relevant memories, deferred tool deltas, team context, and more. Pruner's hook output is one of many signals competing for the model's attention.

**Why this matters:**

- The model receives 20-30K tokens of auto-injected context before it even sees the user's prompt
- Adding 10-15K of pruner context on top may cause the model to skim or ignore parts
- Pruner's value is highest when other context sources are sparse (terminal-only, no IDE, no CLAUDE.md)

**Implementation:**

- Detect context richness: if CLAUDE.md exists and is substantial (>2K chars), reduce pruner output
- If `--open-file` is provided (IDE active), focus on call graph around that file rather than broad results
- Cap total pruner output at a percentage of the model's context window (e.g., 5% = 10K for 200K context)
- In the hook script: check if CLAUDE.md or `.claude/rules/` exist and pass `--has-project-docs` flag

### Tighter result set (precision improvement)

Pruner suggests ~63 files on average but Claude only uses ~4-11. 6% precision means 94% of suggestions are noise the model must wade through.

**Evidence:** openclaw posthoc shows mean 63.2 files suggested, 3.9 hits, 6.8 misses. The model is ignoring most of what pruner suggests.

**Implementation:** Lower MAX_RESULT_FILES or make dynamic cutoff more aggressive. IDF weighting (#1 above) may naturally help by creating larger score gaps between relevant and irrelevant files.

**Validation:** Posthoc precision should rise from 6% toward 15-20%+ without recall dropping.

### Call graph depth for missed routing files

The most common misses (`dispatch.ts`, `dispatch-from-config.ts`, `resolve-route.ts`, `inbound-worker.ts`) are in the message routing pipeline — connected to matched files via call chains but not reached by the execution path tracer.

**Evidence:** These files appear as misses in 2-4 out of 3 rounds across multiple tasks. They're structurally related to matched symbols but apparently beyond the trace depth or time budget.

**Implementation:** Check if these files are reachable in the call graph from top-matched symbols. If the tracer's depth limit (5) or time budget (10s) is cutting them off, consider increasing depth for high-IDF keyword matches or adding a second-hop file expansion.

**Validation:** Posthoc on cross_package and data_flow tasks — these have the most routing misses.

## Medium priority

### 9. Faster evaluation feedback loop

Current A/B tests run full Claude sessions end-to-end (43 min to 4+ hours per N=2 interactive run). With N=5-10 needed per scenario for statistical significance, validating a single change can take days. This blocks iteration on all other improvements.

**Offline evaluation (no API calls):**

- ~~**Post-hoc hit rate analysis:** Script that reads existing raw JSONL logs and correlates pruner's suggested files with Claude's actual Read/Edit calls. Measures precision (did Claude use what pruner suggested?) and recall (did Claude read files pruner missed?). Runs in seconds on saved logs.~~ **Done** (`tests/posthoc_analysis.py`). Results: recall 73% overall (100% understanding, 97% iterative refinement, 42% implement). Precision 9% (suggests ~25 files, Claude uses ~3-5). Navigation calls drop 80%. Most common implement miss: module registration files (`app.module.ts`) not in call graph.
- **Token budget simulation:** Replay recorded queries through budget logic to measure skip/brief/focused rates without running Claude. Validates budget changes instantly.
- **Output diff measurement:** Run `pruner context` with old vs new code on the same queries, compare token counts and file rankings. No Claude needed.

**Cheaper live evaluation:**

- **Sonnet instead of Opus for A/B tests:** 5-10x faster, much cheaper. Relative delta (with vs without pruner) should hold. Reserve Opus for final validation only.
- **Single-turn proxy:** Interactive benefit mostly comes from turn 0. Test changes with one-shot A/B (minutes, not hours) and only run multi-turn for budget-specific changes.
- **Smaller test repo:** openclaw is 9.8K files. A 1-2K file repo would run proportionally faster for quick iteration.

### 10. TypeScript/JS-specific improvements

~~**React component trees:** Ink/React `<Component>` usage in JSX creates an implicit call graph. Tree-sitter can capture JSX element names — weight these as call edges.~~ **Done.** Uppercase JSX elements (`<Header />`, `<Nav.Menu />`) extracted as call edges. HTML elements (`<div>`) excluded.

~~**Dynamic imports:** `import()` calls are invisible to static call graphs. At minimum note "this module is dynamically imported by X".~~ **Done.** `import('./path')` expressions extracted as import entries.

~~**Re-export tracking:** `export { X } from './module'` and `export * from './module'` are common in barrel files. These should be captured so the call graph can follow re-export chains.~~ **Done.** Re-exports now captured as import entries with source module.

**Barrel file resolution:** `index.ts` re-export files are common. Pruner should follow re-exports to actual implementations rather than stopping at the barrel. (Partially addressed by re-export tracking above — full resolution requires indexer-level cross-file re-export chain following.)

**Compiled React output detection:** Claude-code contains React Compiler output (`_c()`, `$[0]` patterns). Detect `_c = require("react/compiler-runtime")` and deprioritize these files or note they're generated. (Requires content-based detection at index time — deferred.)

### 12. Prompt cache-friendly output

Claude Code uses Anthropic's prompt cache (up to 1-hour TTL for eligible users, 5-min otherwise). It has sophisticated cache-break detection that hashes system prompts, tool schemas, and cache_control markers. Any change in pruner's output between turns invalidates the cache suffix, potentially wasting 50-80K cached tokens.

**Status: Done.** Most items were already implemented; the remaining non-determinism (sort tiebreakers) is now fixed:

- **Stabilize output ordering**: ~~When scores are close (within 5%), sort alphabetically instead of by score~~ → Alphabetical tiebreakers on all 3 sort sites (symbol, file, post-dedup)
- **Consistent template structure**: Already consistent — identical markdown sections across queries
- **Two-part output**: YAGNI — output is already brief (~2.5K tokens) with query-specific content only
- **Output hashing**: Already implemented via budget system (`hash_output` + `last-query.json`)
- **Deterministic formatting**: Already clean — no timestamps, run IDs, or non-deterministic content

## Code quality / refactor

Code-level improvements surfaced from a 2026-04-19 audit. Unlike the product items above (which affect what pruner suggests), these affect how the codebase evolves: refactor opportunities, dev-loop velocity, and a couple of latent bugs.

### 20. Split `query.rs` (2830 lines) and centralize scoring weights

Natural seams exist at: keyword extraction/stemming/fuzzy (~lines 776–889), symbol/file scoring (`score_symbol`/`score_file*`, ~891–1144), trace paths (`trace_paths`/`trace_execution_path_cte`, ~734–774), subsystems (`infer_subsystems`, 156 lines at 1181). Scoring constants (`EXACT_MATCH=100`, `FILE_TEST_PENALTY=-25`, fuzzy/substring/prefix weights, ~16 total) are scattered magic numbers.

**Why this matters:**

- TODO #4 (IDE/session context awareness) and TODO #7 (context-rich environments) both need to vary scoring based on runtime signals; today there's no single place to plug that in
- A/B tuning scoring requires a code edit + rebuild every time
- The file is past the point where it fits in one mental model

**Implementation:**

- `query/mod.rs`, `query/keywords.rs`, `query/scoring.rs`, `query/trace.rs`, `query/subsystems.rs`
- `struct ScoringWeights { exact_match: i32, fuzzy_match: i32, ... }` with `Default` impl holding current constants
- Pass `&ScoringWeights` through `score_symbol`/`score_file`; optionally load overrides from env or `.pruner/weights.toml`

### 21. Extract per-tool integrations from `cli.rs` (2286 lines)

`cmd_init` (~260 lines) and `cmd_status` (~220 lines) mix filesystem ops, JSON/TOML mutation, and CLI presentation for Claude Code, Copilot, and Codex. The Codex-specific helpers (`has_codex_hooks_enabled`, `enable_codex_hooks`, `upsert_codex_hook`, `codex_hook_command`) are duplicated in shape in `uninstall.rs` (1562 lines).

**Why this matters:**

- Each tool has install/uninstall/status logic split across two giant files
- Adding a new tool target (Cursor, Aider, etc.) requires touching both files in multiple places
- Hard to unit-test integration logic — cli.rs has only 8 tests for ~45 functions vs. 58 in uninstall.rs

**Implementation:**

- `integrations/mod.rs` with `trait Integration { fn name(..); fn is_installed(..); fn install(..); fn uninstall(..); fn status(..) }`
- `integrations/claude_code.rs`, `integrations/copilot.rs`, `integrations/codex.rs`
- `cmd_init`/`cmd_status`/uninstall command iterate over `&[Box<dyn Integration>]`

### 23. Offline query replay subcommand (unblock scoring iteration)

TODO #9 already flags evaluation as the bottleneck for iterating on scoring. The posthoc script exists but runs against saved Claude sessions — it doesn't let you compare "files pruner would suggest at HEAD vs. branch" directly.

**Implementation:**

- `pruner replay <session.jsonl>` reads user prompts from a saved session and runs `analyze_query` against the current index
- Emits the suggested file list + scores per query
- Paired with a diff tool: `pruner replay session.jsonl --baseline-ref main` runs replay against both revisions (via git worktree, already used by `--baseline-branch` in A/B tests) and diffs the results
- Unblocks fast iteration on #20's `ScoringWeights`

### 24. `infer_subsystems` O(n·m) path parsing (query.rs:1181, 156 lines)

Iterates every file, splits each path on `/`, checks each component against a scaffold-dirs set, allocates `String`s freely. Runs per-query, scales with index size.

**Implementation:**

- Precompute the scaffold-dirs as `&'static HashSet<&str>` (not `Vec`)
- Work on `&str` path components without allocating
- Cache the subsystem result on `FileRow` at index time instead of recomputing per query (bigger change but right long-term fix)

### 25. Parallelize scoring with rayon on large candidate pools

`score_and_rank_files` and `score_and_rank_symbols` are sequential. `rayon` is already a dep (used by `indexer.rs`). On 10k-file repos the scoring pass shows up in hook latency.

**Implementation:** `.par_iter().map(|f| (f, score_file(..))).collect()`. Gated behind a candidate-count threshold so small repos don't pay thread-pool startup.

## Low priority / future

### 13. Post-session accuracy feedback loop

Pruner fires once and never learns whether its output was useful. Claude Code's `stream-json` output reveals what the model actually did.

**Implementation:**

- Post-session analyzer reads stream-json and correlates pruner's suggested files with model's actual file reads/edits
- "If model immediately reads a file pruner suggested" → pruner was helpful
- "If model greps for something pruner didn't mention" → pruner missed it
- "If model ignores pruner entirely" → query was noise
- Use this data to tune scoring weights over time

## Existing items (carried forward)

### 14. Complete code slices (zero follow-up reads) — platform-agnostic

Current snippets are 30-line truncations that often require Claude to Read the full file anyway. Since tree-sitter knows exact symbol boundaries, pruner should extract complete function/method bodies.

**Why this matters:**

- Each follow-up Read adds ~2-5K tokens to the opus conversation history
- If pruner gives Claude everything it needs upfront, Claude can go straight to implementation
- The goal: zero Read tool calls for understanding, only Read/Write for making changes

**Implementation:**

- In `context.rs`, use symbol start/end line info to extract full function bodies instead of fixed 30-line windows
- Cap individual function bodies at reasonable size (e.g., 100 lines)
- For the implementation scenario, include the function where new code should be added (e.g., the route registration function)

### 15. Edit-location hints — platform-agnostic

For implementation tasks, pruner can analyze the call graph to suggest exactly where to make changes, not just which files are relevant.

**Why this matters:**

- Turns "here are 10 relevant files" into "add your code at `src/routes.ts:45` inside `registerRoutes()`"
- Any agent (Claude Code, Copilot, Codex) skips the "figure out where to put it" phase entirely
- Biggest win for implementation tasks

**Implementation:**

- Detect "implement" / "add" / "create" keywords in the query
- Find insertion points: functions that register similar things (routes, handlers, tests)
- Output a "suggested edit location" section with file:line and surrounding context

### 16. More language parsers

Add tree-sitter parsers for additional languages. Currently Python, JavaScript/TypeScript, Rust, Go, Java, C, C++, and C# have full symbol/import/call extraction.

### 18. Semantic search

Add optional embedding-based search for queries that don't match symbol/file names (e.g., a function that handles authentication but is named `validateRequest`).

## Done

- [x] Prompt-submit hook (zero tool calls) — Claude Code only
- [x] `pruner init` command for easy project setup
- [x] `install.sh` for pre-built binary installation
- [x] CI + release GitHub Actions workflows
- [x] A/B test with `--mode hook|skill` flag
- [x] Auto-detect task scope (brief vs focused)
- [x] Focused mode with code snippets (~10-15K tokens)
- [x] Graph expansion via execution paths
- [x] Query performance fix (was timing out on large repos)
- [x] Relevance scoring (exact > prefix > substring, quality penalties)
- [x] Go, Java, C, C++, C# parsers
- [x] Cache-aware A/B test setup (warmup runs, `--validate-cache` flag, cache hit rate logging)
- [x] Feature impact measurement (`--baseline-branch <ref>` builds two pruner versions via git worktree)
- [x] Query precision: dynamic stop-words, keyword specificity filtering, multi-word phrase handling
- [x] Non-code query detection (meta-questions return empty results)
- [x] Negative scoring: test files penalized for non-test queries, generated code detection
- [x] Multi-turn A/B test (`--multi-turn` flag for interactive conversation scenarios)
- [x] Query precision fixes: meta-question false positive reduction, SQL LIKE wildcard escaping, test-intent ordering before specificity filtering
- [x] Root directory indexing for multi-repo setups (`--no-root` flag to opt out)
- [x] Query-aware context budget (same-topic → brief, identical output → skip, new topic → focused)
- [x] Post-hoc hit rate analysis (`tests/posthoc_analysis.py`) — offline precision/recall from saved JSONL logs
- [x] Fast A/B test mode (`--fast` sonnet + nest, `--rounds N`, `--interactive`)
- [x] Deferred context mode: brief default (~2.5K tokens), `--detail` for full output. Full context written to `.pruner/context.md` for zero-cost escalation. A/B tested: -24% cost, -17% tools on implement tasks (N=3)
- [x] Structural ranking transparency: authority header with index stats, per-file reasons (symbol/keyword hit counts), ranking note. A/B tested: neutral on cost/tools (N=3), no regression
- [x] Prompt cache-friendly output: deterministic sort ordering via alphabetical tiebreakers on all sort sites. Output hashing and skip already implemented via budget system. No timestamps or non-deterministic content
- [x] Keyword stemming + bidirectional prefix matching (`rust-stemmers` Snowball English). Stem-based candidate gathering and scoring fallback. No posthoc recall change yet — narrow_fix bottleneck is keyword quality ("handle" drowning out "reconnection"), not stemming
- [x] Keyword IDF weighting: `idf = min(file_idf, sym_idf)` with stem-aware hit counts. Rare keywords contribute more to scoring. Posthoc: openclaw recall 40% → 43%, implement 79% → 83%, narrow_fix 0% → 40%
- [x] TS/JS parser: JSX components as call edges, dynamic `import()` as imports, re-export tracking (`export { X } from './module'`). Barrel file full resolution and React Compiler detection deferred
- [x] Uninstall: surface cleanup errors instead of swallowing them. `let _ = fs::…` replaced with warning-collecting helpers; "Cleanup completed with warnings:" block printed when non-empty, best-effort semantics preserved
- [x] Installer: non-UTF-8 hook paths return an `anyhow` error with clear message instead of panicking via `to_str().unwrap()`
- [x] Split `parser.rs` (3308 lines) into per-language submodules under `src/parser/` with a shared `common` module (`node_text`, `normalize_type_name`). C++ reuses C's call/signature/type/typedef helpers via `pub(super)`. Rejected the `LanguageAdapter` trait direction — each language does one unified tree walk, so splitting into `extract_symbols`/`extract_imports`/`extract_calls` would triple traversal cost without buying anything

## Explored but rejected

### Subagent delegation
Tried having pruner instruct Claude to spawn a cheap sonnet subagent for implementation. Result: +292% cost, +336% time. The opus orchestration overhead plus subagent context duplication made it much worse. Claude also ignored the "don't re-explore" instruction and spawned its own Explore subagent first.
