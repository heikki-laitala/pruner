---
name: pruner
description: Generate synthetic code context for LLM coding tasks. Automatically use this before making code changes, fixing bugs, refactoring, or answering questions about the codebase. Provides execution paths, relevant files, symbols, tests, and code snippets.
argument-hint: "<ask>"
allowed-tools: Bash(pruner *), Read, Grep, Glob
user-invocable: false
---

# Pruner — Code Context Engine

Run one command to get context. Pruner auto-detects whether the task is narrow or broad and adjusts output accordingly:
- **Narrow tasks** (few files, single subsystem): brief output (~3K tokens) — just pointers
- **Broad tasks** (many files, multiple subsystems): focused output (~10-15K tokens) — includes code snippets

## Usage

```bash
pruner context /absolute/path/to/repo "<the user's ask>"
```

IMPORTANT: Always pass the repo as an absolute path argument. Do NOT use `cd <repo> && pruner context .` — the shell sandbox may reset the working directory.

## After running pruner

1. **Read the output** — for broad tasks it contains actual code snippets. For narrow tasks it gives file/symbol pointers.
2. **Read source files only if needed** — if a snippet is truncated or you need surrounding context, use the file:line pointers from the output.
3. **Do NOT re-explore** — pruner already searched the index. Do not grep or glob for the same keywords.

## When you need more detail

- **Single symbol**: `pruner show-symbol /path/to/repo "<name>"` — signature, callers, callees.
- **Single file**: `pruner show-file /path/to/repo "<path>"` — all symbols and imports.

## Other modes

- `--brief` — metadata only, no snippets (~3K tokens). Use when you only need file/symbol pointers.
- `--full` — uncapped output with all snippets (~50-70K tokens). Use for deep analysis.
