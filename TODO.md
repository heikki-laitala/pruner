# TODO

## Current status

Install flow is streamlined: `install.sh` + `pruner init` + `pruner index`.

**A/B test results are currently invalid.** Previous results showed 24-62% cost savings, but the test setup did not account for Claude Code's 1-hour prompt cache TTL. The cache means the control (no-pruner) side may benefit from cached context across runs, biasing results. A valid A/B test setup that isolates cache effects is needed before any performance claims can be made.

## Highest priority

### 1. Cache-aware A/B test setup

The current A/B test (`tests/ab_test.py`) does not account for Claude Code's 1-hour prompt cache TTL. When runs execute sequentially, the second run benefits from cached context regardless of whether pruner was active, biasing results. No performance claims can be made until this is fixed.

**Implementation:**

- Ensure each run starts with a cold cache (new session ID, or wait for TTL expiry, or use `--no-cache` if available)
- Alternatively, run both sides with equal cache warmth: warm-up run (discarded) → measured run for each side
- Log cache hit rates per run (`cache_read_input_tokens` vs `input_tokens`) so bias is visible in results
- Add a `--validate-cache` flag that aborts if cache hit rate differs >10% between sides

### 2. Feature impact measurement (main vs feature branch)

To verify that TODO improvements actually bring value, we need A/B tests that compare `main` (baseline) against a feature branch, not just "with pruner" vs "without pruner".

**Why this matters:**

- Current setup only answers "does pruner help vs no pruner?" — it cannot measure whether a specific change (e.g., query precision, turn-aware budget) improves pruner itself
- Without this, we ship features blindly and cannot prioritize the roadmap based on measured impact

**Implementation:**

- Add `--baseline-branch <ref>` flag to `ab_test.py`: builds pruner from that ref for the control side, and from the current worktree for the treatment side
- Both sides run with pruner enabled — the only difference is the pruner binary version
- Reuse the interleaved schedule and cache-aware setup from #1
- Output a delta table: cost/time/tools for baseline branch vs feature branch
- Use this to validate each TODO item before merging: if a feature doesn't measurably improve results, reconsider it

## High priority

### 3. Query precision — fix noisy output (highest ROI)

Pruner's biggest failure mode: broad keywords match too many irrelevant symbols, producing noisy context that wastes tokens and can mislead the model. Observed directly on claude-code (1,900 files): query "does pruner bring value" returned `IdeAutoConnectDialog`, `pruneRemovedPluginHooks`, `goToLine` — none relevant.

**Why this matters:**

- Noisy output is worse than no output — it wastes tokens AND can send the model down dead ends
- This is the root cause of the implement_large variance (+233% time outlier where Claude read every suggested file but never wrote code)
- Tool call reductions (-61% to -87%) are trustworthy, but noise undermines the value

**Implementation:**

- **Dynamic stop-words**: If a keyword appears in 30%+ of files in the repo, drop it. Static stop-word list is insufficient — "value", "go", "now" aren't on the list but are noise in code repos
- **Minimum keyword specificity score**: After stop-word filtering, require at least one keyword with specificity (appears in <5% of files) to proceed. Otherwise emit nothing
- **Confidence threshold**: Below a score threshold, output "No relevant context found" instead of low-confidence results. Zero output is better than misleading output
- **Multi-word phrase handling**: "claude-code" as a compound should be treated differently than "claude" + "code" separately. Detect hyphenated/quoted terms and match them as phrases

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

### 5. Turn-aware token budget

Claude Code's auto-compaction triggers at ~167K tokens (for 200K Opus context). Pruner's hook output is NOT in the compactable tools list — it persists as a `user-prompt-submit-hook` message until full compaction. This means pruner's 10-15K tokens sit in the context permanently, unlike Grep/Glob/Read results which get micro-compacted.

**Why this matters:**

- First prompt in a session has zero context — focused mode is high value
- By turn 10+, the model has already read key files — re-injecting 10K tokens of context is wasteful and accelerates compaction
- Unlike tool results, hook output survives micro-compaction (advantage: structural context persists; disadvantage: tokens aren't reclaimed)
- Each turn's pruner output is additive — 5 turns × 10K tokens = 50K tokens of pruner output alone
- At that rate, pruner output alone triggers compaction ~3 turns earlier

**Implementation:**

- Accept `--session-turn <n>` flag (hook script can detect turn count)
- Turn 1: focused mode (full snippets, execution paths, ~10-15K tokens)
- Turn 2-3: brief mode (pointers only, no snippets, ~3K tokens)
- Turn 4+: minimal mode (only if query targets a new subsystem not covered in prior turns, ~1K tokens)
- If query is similar to a recent query (within 2 min): emit nothing and let prior context suffice
- Consider: hash output and skip injection if identical to previous turn

### 6. Deferred context mode (two-phase output)

Claude Code defers MCP tool schemas to save context: tools are listed by name only, and full schemas are fetched on-demand via ToolSearchTool. Pruner could adopt the same pattern.

**Why this matters:**

- Current hook mode dumps 10-15K tokens upfront regardless of whether the model needs all of it
- Many queries only need the file pointers (model reads the files itself)
- Full execution paths and snippets are only needed for understanding/tracing tasks

**Implementation:**

- **Phase 1 (hook):** Inject brief summary (~2K tokens): keywords detected, subsystems identified, top 8 file pointers with one-line descriptions, top 10 symbols
- **Phase 2 (skill, on-demand):** Model calls `pruner context --detail` to get full execution paths, code snippets, and expanded analysis
- Hook output includes a note: "Run `pruner context --detail` for execution paths and code snippets"
- This mirrors Claude Code's own deferred loading pattern and cuts upfront cost by 80%

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

### 9. Detect and handle non-code queries

Meta-questions ("does pruner bring value?", "how should we improve this?") don't benefit from codebase context. Pruner should detect these and return nothing.

**Implementation:**

- Heuristic: if no keyword has specificity <10% of files, the query is probably not about the code
- Check for question patterns about tooling, process, or meta-topics
- Return empty output with a note: "Query does not appear to target specific code"

### 10. TypeScript/JS-specific improvements

**Barrel file resolution:** `index.ts` re-export files are common. Pruner should follow re-exports to actual implementations rather than stopping at the barrel.

**React component trees:** Ink/React `<Component>` usage in JSX creates an implicit call graph. Tree-sitter can capture JSX element names — weight these as call edges.

**Dynamic imports:** `import()` calls are invisible to static call graphs. At minimum note "this module is dynamically imported by X".

**Compiled React output detection:** Claude-code contains React Compiler output (`_c()`, `$[0]` patterns). Detect `_c = require("react/compiler-runtime")` and deprioritize these files or note they're generated.

### 11. Negative scoring signals

Currently scoring is positive-only (keyword match, call graph proximity). Add penalties:

- **Generated/compiled code**: React Compiler output, bundled files, minified code
- **Test files when query isn't about testing**: If query is "how does auth work", test files add noise
- **Vendored directories**: Large single-author dirs with no recent changes (e.g., `native-ts/yoga-layout/`)
- **Documentation/locale/assets dirs**: Already partially handled, but weight should be stronger

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

Add tree-sitter parsers for additional languages. Currently Python, JavaScript/TypeScript, Rust, Go, Java, C, and C++ have full symbol/import/call extraction.

### 17. Semantic search

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
- [x] Go, Java, C, C++ parsers

## Explored but rejected

### Subagent delegation
Tried having pruner instruct Claude to spawn a cheap sonnet subagent for implementation. Result: +292% cost, +336% time. The opus orchestration overhead plus subagent context duplication made it much worse. Claude also ignored the "don't re-explore" instruction and spawned its own Explore subagent first.
