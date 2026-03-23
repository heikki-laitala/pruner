# TODO

## Current status

Pruner saves 24-41% cost and 46-62% time on broad tasks (hook mode). Skill mode saves 35% on large features. Install flow is streamlined: `install.sh` + `pruner init` + `pruner index`.

A/B test results (hook mode, OpenClaw 9.8K files):
- Narrow fix: +6% cost (breakeven)
- Cross-package: **-24%** cost, -46% time
- Understanding: **-32%** cost, -58% time
- Data flow: **-32%** cost, -56% time
- Implement: **-30%** cost, -53% time
- Implement (large): **-41%** cost, -62% time

## 1. Complete code slices (zero follow-up reads) — platform-agnostic

Current snippets are 30-line truncations that often require Claude to Read the full file anyway. Since tree-sitter knows exact symbol boundaries, pruner should extract complete function/method bodies.

**Why this matters:**
- Each follow-up Read adds ~2-5K tokens to the opus conversation history
- If pruner gives Claude everything it needs upfront, Claude can go straight to implementation
- The goal: zero Read tool calls for understanding, only Read/Write for making changes

**Implementation:**
- In `context.rs`, use symbol start/end line info to extract full function bodies instead of fixed 30-line windows
- Cap individual function bodies at reasonable size (e.g., 100 lines)
- For the implementation scenario, include the function where new code should be added (e.g., the route registration function)

## 2. Edit-location hints — platform-agnostic

For implementation tasks, pruner can analyze the call graph to suggest exactly where to make changes, not just which files are relevant.

**Why this matters:**
- Turns "here are 10 relevant files" into "add your code at `src/routes.ts:45` inside `registerRoutes()`"
- Any agent (Claude Code, Copilot, Codex) skips the "figure out where to put it" phase entirely
- Biggest win for implementation tasks

**Implementation:**
- Detect "implement" / "add" / "create" keywords in the query
- Find insertion points: functions that register similar things (routes, handlers, tests)
- Output a "suggested edit location" section with file:line and surrounding context

## 3. More language parsers

Add tree-sitter parsers for Go, Java, Ruby. Currently only Python, JavaScript/TypeScript, and Rust have full symbol/import/call extraction.

## 4. Semantic search

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

## Explored but rejected

### Subagent delegation
Tried having pruner instruct Claude to spawn a cheap sonnet subagent for implementation. Result: +292% cost, +336% time. The opus orchestration overhead plus subagent context duplication made it much worse. Claude also ignored the "don't re-explore" instruction and spawned its own Explore subagent first.
