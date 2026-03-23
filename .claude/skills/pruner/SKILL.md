---
name: pruner
description: Generate synthetic code context for LLM coding tasks. Automatically use this before making code changes, fixing bugs, refactoring, or answering questions about the codebase. Provides execution paths, relevant files, symbols, tests, and code snippets.
argument-hint: "<ask>"
allowed-tools: Bash(pruner *), Read, Grep, Glob
user-invocable: false
---

# Pruner — Code Context Engine

Pruner works in two phases: a cheap **brief** scan to orient you, then **targeted reads** of the actual source files it identifies. Do NOT dump full context into the conversation — use brief to decide *where* to look, then read only what you need.

## Phase 1: Orient — run brief

```bash
pruner context /absolute/path/to/repo "<the user's ask>" --brief
```

IMPORTANT: Always pass the repo as an absolute path argument. Do NOT use `cd <repo> && pruner context .` — the shell sandbox may reset the working directory.

This prints a compact table of contents (~500 tokens): keywords, key files, key symbols with locations, execution path count, and related tests. No snippets — just pointers.

## Phase 2: Read — open the source files

1. **Read the top 3-5 key files** listed in the brief output. These are the files most relevant to the task — read them directly with the Read tool.
2. **Use symbol locations** (file:line) from the brief output to jump to specific functions/classes rather than reading entire large files.
3. **Read related test files** if the task involves changing behavior.

## When you need more detail

- **Deep call graph**: `pruner context /path/to/repo "<ask>"` (without `--brief`) writes full execution paths, snippets, and call graphs. Only use this when you need to trace a complex flow across many files.
- **Single symbol**: `pruner show-symbol /path/to/repo "<name>"` shows a symbol's signature, callers, and callees.
- **Single file**: `pruner show-file /path/to/repo "<path>"` shows all symbols and imports in a file.

## What NOT to do

- **Do NOT Glob** to explore directory structure — pruner already identified relevant files.
- **Do NOT Grep the codebase** for keywords — pruner already searched the index.
- **Do NOT read full context into conversation** when brief is sufficient — it wastes tokens.
- Only Glob/Grep to verify a specific detail not in pruner's output (e.g., checking package.json for a dependency).
