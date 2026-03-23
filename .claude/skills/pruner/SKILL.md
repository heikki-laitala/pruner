---
name: pruner
description: Generate synthetic code context for LLM coding tasks. Automatically use this before making code changes, fixing bugs, refactoring, or answering questions about the codebase. Provides execution paths, relevant files, symbols, tests, and code snippets.
argument-hint: "<ask>"
allowed-tools: Bash(pruner *), Read, Grep, Glob
user-invocable: false
---

# Pruner — Code Context Engine

Run one command to get focused context (~10-15K tokens) with key files, symbols, execution paths, and relevant code snippets. This replaces manual grep/glob exploration.

## Usage

```bash
pruner context /absolute/path/to/repo "<the user's ask>"
```

IMPORTANT: Always pass the repo as an absolute path argument. Do NOT use `cd <repo> && pruner context .` — the shell sandbox may reset the working directory.

This prints a focused context package containing:
- Keywords and subsystems
- Execution paths through the call graph
- Key files (top 10)
- Key symbols with locations (top 20)
- Code snippets for matched symbols (top 20, up to 30 lines each)
- Related tests

## After running pruner

1. **Read the output** — it contains the actual code snippets you need. In most cases you can proceed directly to the task.
2. **Read source files only if needed** — if a snippet is truncated or you need surrounding context, use the file:line pointers from the output.
3. **Do NOT re-explore** — pruner already searched the index. Do not grep or glob for the same keywords.

## When you need more detail

- **Single symbol**: `pruner show-symbol /path/to/repo "<name>"` — signature, callers, callees.
- **Single file**: `pruner show-file /path/to/repo "<path>"` — all symbols and imports.

## Other modes

- `--brief` — metadata only, no snippets (~3K tokens). Use when you only need file/symbol pointers.
- `--full` — uncapped output with all snippets (~50-70K tokens). Use for deep analysis.
