---
name: pruner
description: Generate synthetic code context for LLM coding tasks. Automatically use this before making code changes, fixing bugs, refactoring, or answering questions about the codebase. Provides execution paths, relevant files, symbols, tests, and code snippets.
argument-hint: "<ask>"
allowed-tools: Bash(pruner *), Read, Grep, Glob
user-invocable: false
---

# Pruner — Code Context Engine

Pruner context is **automatically injected** into your conversation via a prompt-submit hook. You do NOT need to run `pruner context` manually — it has already run before you see the user's message.

## How it works

A hook runs `pruner context` on every prompt submit and injects the output as additional context. Look for the "Pruner context (pre-computed codebase analysis)" section in the conversation — that's the pruner output.

## Working with pruner context

1. **Use the injected context directly** — it contains relevant files, symbols, execution paths, and code snippets.
2. **Read source files only if needed** — if a snippet is truncated or you need surrounding context, use the file:line pointers from the output.
3. **Do NOT re-explore** — pruner already searched the index. Do not grep or glob for the same keywords.
4. **Do NOT run `pruner context` again** — it already ran via the hook.

## When you need more detail

- **Single symbol**: `pruner show-symbol /path/to/repo "<name>"` — signature, callers, callees.
- **Single file**: `pruner show-file /path/to/repo "<path>"` — all symbols and imports.

## Other modes

- `--brief` — metadata only, no snippets (~3K tokens). Use when you only need file/symbol pointers.
- `--full` — uncapped output with all snippets (~50-70K tokens). Use for deep analysis.
