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

### 6. Deferred context mode (two-phase output)

Claude Code defers MCP tool schemas to save context: tools are listed by name only, and full schemas are fetched on-demand via ToolSearchTool. Pruner could adopt the same pattern.

**Why this matters:**

- Current hook mode dumps 10-15K tokens upfront regardless of whether the model needs all of it
- Many queries only need the file pointers (model reads the files itself)
- Full execution paths and snippets are only needed for understanding/tracing tasks

**Implementation:**

- **`pruner context` (default, hook):** Emit brief summary (~2K tokens): keywords detected, subsystems identified, top 8 file pointers with one-line descriptions, top 10 symbols. Include inline instruction: `→ Run pruner context --detail for execution paths and code snippets`. No CLAUDE.md change needed — the instruction travels with the hook output.
- **`pruner context --detail` (on-demand, skill):** Return full focused output — execution paths, code snippets, expanded analysis. This is what the hook gives today (~10-15K tokens). Model calls it only when brief pointers aren't enough.
- **SKILL.md:** Already defines `pruner context`. Add `--detail` flag documentation so the model knows when and how to invoke it.
- **Hook script:** Change default `pruner context` invocation (no flags needed — brief becomes the default). The `--detail` flag is only used by the model via skill/tool call.
- **Interactive session benefit:** On turn 0, model sees brief pointers and pulls `--detail` if needed. On follow-up turns, brief pointers are enough (or skipped by query-aware budget), cutting per-turn token cost from ~10K to ~2K or zero.
- Mirrors Claude Code's own deferred loading pattern (ToolSearchTool) and cuts upfront cost by ~80%.

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

### 8. Structural ranking beats mtime — tell the model

Claude Code's Grep sorts by modification time (most recent first), Glob also sorts by mtime. Both cap results (250 and 100 respectively). This means recent-but-irrelevant files rank above old-but-critical files.

**Why this matters:**

- Pruner's call-graph-based ranking is strictly better than mtime for structural relevance
- But the model doesn't know this — it may re-grep and override pruner's suggestions with mtime-sorted results
- Making this explicit helps the model trust pruner over its own exploration

**Implementation:**

- Add a brief note in pruner output: "Files ranked by structural relevance (call graph + keyword specificity), not recency"
- For key files, include why they ranked high: "auth.ts: 5 callers, matches 3 query keywords"
- This transparency helps the model make informed decisions about when to trust pruner vs explore further
- Include an authority header in the output: "Pre-computed from full codebase index (tree-sitter call graph, N files, N symbols)". Claude Code's system prompt tells the model to treat hook output as "coming from the user" but doesn't tell it how much to trust the data. This header helps the model calibrate — it's an authoritative structural index, not cached notes or heuristic guesses. Particularly important because more capable models (observed with gpt-5.3-codex in Copilot tests) tend to second-guess externally provided context and re-explore anyway.

## Medium priority

### 10. TypeScript/JS-specific improvements

**Barrel file resolution:** `index.ts` re-export files are common. Pruner should follow re-exports to actual implementations rather than stopping at the barrel.

**React component trees:** Ink/React `<Component>` usage in JSX creates an implicit call graph. Tree-sitter can capture JSX element names — weight these as call edges.

**Dynamic imports:** `import()` calls are invisible to static call graphs. At minimum note "this module is dynamically imported by X".

**Compiled React output detection:** Claude-code contains React Compiler output (`_c()`, `$[0]` patterns). Detect `_c = require("react/compiler-runtime")` and deprioritize these files or note they're generated.

### 12. Prompt cache-friendly output

Claude Code uses Anthropic's prompt cache (up to 1-hour TTL for eligible users, 5-min otherwise). It has sophisticated cache-break detection that hashes system prompts, tool schemas, and cache_control markers. Any change in pruner's output between turns invalidates the cache suffix, potentially wasting 50-80K cached tokens.

**Implementation:**

- **Stabilize output ordering**: When scores are close (within 5%), sort alphabetically instead of by score
- **Consistent template structure**: Identical markdown sections across queries — only content changes
- **Two-part output**: Stable "project structure" section (cached, changes rarely) + volatile "query-specific" section (recomputed per prompt)
- **Output hashing**: Hash pruner's output and compare with `.pruner/last-output-hash`. If identical, return a "no change" signal so the hook script skips injection entirely — prevents unnecessary cache invalidation
- **Deterministic formatting**: Avoid timestamps, run IDs, or any non-deterministic content in the output

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

## Explored but rejected

### Subagent delegation
Tried having pruner instruct Claude to spawn a cheap sonnet subagent for implementation. Result: +292% cost, +336% time. The opus orchestration overhead plus subagent context duplication made it much worse. Claude also ignored the "don't re-explore" instruction and spawned its own Explore subagent first.
