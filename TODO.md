# TODO

## Current status

Install flow is streamlined: `install.sh` + `pruner init` + `pruner index`.

A/B test infrastructure is solid: cache-aware warmup runs, `--validate-cache` flag, interleaved scheduling, `--baseline-branch` for feature impact measurement, and `--multi-turn` for interactive conversation scenarios.

**Best for one-shot tasks** (`claude -p "task"`). N=3 A/B test results on openclaw (9.8K files, v0.2.4, 2026-04-02):

| Task | Δ cost | Δ tools | Δ time |
|------|--------|---------|--------|
| Understanding | -62% | -86% | -64% |
| Cross-package | -49% | -80% | -56% |
| Data flow | -41% | -80% | -52% |
| Implement | -15% | -59% | -44% |
| Narrow fix | -6% | -21% | -39% |
| Implement large | -3% | -25% | +8% |

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

**Barrel file resolution:** `index.ts` re-export files are common. Pruner should follow re-exports to actual implementations rather than stopping at the barrel.

**React component trees:** Ink/React `<Component>` usage in JSX creates an implicit call graph. Tree-sitter can capture JSX element names — weight these as call edges.

**Dynamic imports:** `import()` calls are invisible to static call graphs. At minimum note "this module is dynamically imported by X".

**Compiled React output detection:** Claude-code contains React Compiler output (`_c()`, `$[0]` patterns). Detect `_c = require("react/compiler-runtime")` and deprioritize these files or note they're generated.

### 12. Prompt cache-friendly output

Claude Code uses Anthropic's prompt cache (up to 1-hour TTL for eligible users, 5-min otherwise). It has sophisticated cache-break detection that hashes system prompts, tool schemas, and cache_control markers. Any change in pruner's output between turns invalidates the cache suffix, potentially wasting 50-80K cached tokens.

**Status: Done.** Most items were already implemented; the remaining non-determinism (sort tiebreakers) is now fixed:

- **Stabilize output ordering**: ~~When scores are close (within 5%), sort alphabetically instead of by score~~ → Alphabetical tiebreakers on all 3 sort sites (symbol, file, post-dedup)
- **Consistent template structure**: Already consistent — identical markdown sections across queries
- **Two-part output**: YAGNI — output is already brief (~2.5K tokens) with query-specific content only
- **Output hashing**: Already implemented via budget system (`hash_output` + `last-query.json`)
- **Deterministic formatting**: Already clean — no timestamps, run IDs, or non-deterministic content

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

## Explored but rejected

### Subagent delegation
Tried having pruner instruct Claude to spawn a cheap sonnet subagent for implementation. Result: +292% cost, +336% time. The opus orchestration overhead plus subagent context duplication made it much worse. Claude also ignored the "don't re-explore" instruction and spawned its own Explore subagent first.
