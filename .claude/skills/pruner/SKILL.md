---
name: pruner
description: Generate synthetic code context for LLM coding tasks. Automatically use this before making code changes, fixing bugs, refactoring, or answering questions about the codebase. Provides execution paths, relevant files, symbols, tests, and code snippets.
argument-hint: "<ask>"
allowed-tools: Bash(pruner *), Read, Grep, Glob
user-invocable: false
---

# Pruner — Code Context Engine

## Workflow

### Step 1: Run one command

```bash
pruner context . "<the user's ask>" --brief
```

This auto-indexes if needed, prints a compact summary to stdout, and writes full context to `.pruner/context.md`.

### Step 2: Read only what pruner identified

- **Read the key files** listed in the summary.
- If you need more detail (snippets, call graphs, imports), **Read or Grep `.pruner/context.md`** for that symbol name.
- **Do NOT Glob** to explore directory structure — pruner already identified relevant files.
- **Do NOT Grep the codebase** for keywords — pruner already searched the index.
- Only Glob/Grep the codebase to verify a specific detail not in pruner's output (e.g., checking package.json for a dependency).

### Step 3: Proceed with the task

Use the execution paths and key symbols to understand the change surface, then proceed.
