---
name: pruner
description: Generate synthetic code context for LLM coding tasks. Automatically use this before making code changes, fixing bugs, refactoring, or answering questions about the codebase. Provides execution paths, relevant files, symbols, tests, and code snippets.
argument-hint: "<ask>"
allowed-tools: Bash(pruner *), Read, Grep, Glob
user-invocable: false
---

# Pruner — Code Context Engine

Run one command to get context. Default output is a brief summary (~2K tokens) with file/symbol pointers.

## Usage

```bash
# Brief pointers (default)
pruner context /absolute/path/to/repo "<the user's ask>"

# Detailed output with execution paths and code snippets
pruner context /absolute/path/to/repo "<the user's ask>" --detail
```

IMPORTANT: Always pass the repo as an absolute path argument. Do NOT use `cd <repo> && pruner context .` — the shell sandbox may reset the working directory.

## After running pruner

1. **Read the output** — use file/symbol pointers to navigate the codebase.
2. **Use `--detail` if needed** — when pointers aren't enough, re-run with `--detail` for execution paths and code snippets (~10-15K tokens).
3. **Read source files only if needed** — if a snippet is truncated or you need surrounding context, use the file:line pointers from the output.
4. **Do NOT re-explore** — pruner already searched the index. Do not grep or glob for the same keywords.

## When to escalate

- **Read `.pruner/context.md`** — if available, contains full execution paths and code snippets from the last hook run. Zero cost.
- **Run with `--detail`** — regenerates focused output (~10-15K tokens). Use for understanding/debugging queries.
- **Single symbol**: `pruner show-symbol /path/to/repo "<name>"` — signature, callers, callees.
- **Single file**: `pruner show-file /path/to/repo "<path>"` — all symbols and imports.

## Other modes

- `--detail` — execution paths + code snippets (~10-15K tokens). Use when brief pointers aren't enough.
- `--brief` — metadata only, no snippets (~2K tokens). Use when you only need file/symbol pointers.
- `--full` — uncapped output with all snippets (~50-70K tokens). Use for deep analysis.
