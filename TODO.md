# TODO

## Problem statement

Pruner preprocesses the entire codebase (call graphs, symbols, imports) but vanilla Claude Code matches or beats pruner on cost by delegating exploration to cheap subagents. Pruner saves tool calls and wall time, but doesn't consistently save cost. We need to leverage the preprocessing advantage more aggressively.

A/B test baseline (implement scenario on OpenClaw, 9.8K files):
- Without pruner: $0.66 / 48 tools / 122.6s
- With pruner (auto mode): $0.70 / 23 tools / 80.7s (+6% cost, -52% tools, -34% time)

## 1. Prompt-submit hook (zero tool calls)

Install pruner as a Claude Code `prompt_submit` hook instead of a skill. When the user submits a prompt, the hook runs `pruner context` and injects the output into the conversation before Claude starts thinking.

**Why this matters:**
- Eliminates the tool call that runs pruner (saves one opus round-trip)
- Context is present from turn 1, gets cached by the API on all subsequent turns (cheap)
- Claude never spends opus tokens deciding whether/how to use pruner

**Implementation:**
- Add a hook config to `.claude/settings.json` that runs `pruner context . "$PROMPT"` on prompt_submit
- Hook output appears as user-context, not as tool-call result
- Need to figure out how hook output is injected (env var? stdin? appended to prompt?)

## 2. Complete code slices (zero follow-up reads)

Current snippets are 30-line truncations that often require Claude to Read the full file anyway. Since tree-sitter knows exact symbol boundaries, pruner should extract complete function/method bodies.

**Why this matters:**
- Each follow-up Read adds ~2-5K tokens to the opus conversation history
- If pruner gives Claude everything it needs upfront, Claude can go straight to implementation
- The goal: zero Read tool calls for understanding, only Read/Write for making changes

**Implementation:**
- In `context.rs`, use symbol start/end line info to extract full function bodies instead of fixed 30-line windows
- Cap individual function bodies at reasonable size (e.g., 100 lines)
- For the implementation scenario, include the function where new code should be added (e.g., the route registration function)

## 3. Edit-location hints

For implementation tasks, pruner can analyze the call graph to suggest exactly where to make changes, not just which files are relevant.

**Why this matters:**
- Turns "here are 10 relevant files" into "add your code at `src/routes.ts:45` inside `registerRoutes()`"
- Claude skips the "figure out where to put it" phase entirely
- Biggest win for implementation tasks (currently our weakest scenario)

**Implementation:**
- Detect "implement" / "add" / "create" keywords in the query
- Find insertion points: functions that register similar things (routes, handlers, tests)
- Output a "suggested edit location" section with file:line and surrounding context

## 4. Explored but rejected

### Subagent delegation (feat/subagent-delegation branch)
Tried having pruner instruct Claude to spawn a cheap sonnet subagent for implementation. Result: +292% cost, +336% time. The opus orchestration overhead plus subagent context duplication made it much worse. Claude also ignored the "don't re-explore" instruction and spawned its own Explore subagent first.
