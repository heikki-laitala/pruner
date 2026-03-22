---
name: pruner
description: Generate synthetic code context for LLM coding tasks. Automatically use this before making code changes, fixing bugs, refactoring, or answering questions about the codebase. Provides execution paths, relevant files, symbols, tests, and code snippets.
argument-hint: "<ask>"
allowed-tools: Bash(pruner *), Read, Grep, Glob
user-invocable: false
---

# Pruner — Code Context Engine

Pruner indexes a repository structurally and generates compact, LLM-ready context packages from natural language asks.

## When to use

Use pruner automatically before making code changes, answering codebase questions, planning implementations, or finding relevant tests.

## Commands

If `.pruner/index.db` does not exist, index first: `pruner index . -v`

Primary command:

```bash
pruner context . "<the user's ask>" --format text
```

## Workflow

1. Run `pruner context . "<the user's ask>"`
2. Read ONLY the key files listed in pruner's output — do NOT glob or grep to "explore" the codebase independently. Pruner already did that work.
3. Use the execution paths and call graphs to understand the change surface
4. Proceed with the task

## Critical: Trust pruner output

- **Do NOT run Glob calls** to explore directory structure — pruner already identified the relevant files and paths.
- **Do NOT run Grep calls** to search for keywords — pruner already searched the index for matching symbols, files, and call sites.
- **Only use Read** on files that pruner identified as key files or that appear in execution paths/snippets.
- The only valid reason to Glob/Grep after pruner is if you need to verify a very specific detail not covered in the context output (e.g., checking if a dependency exists in package.json).
- Redundant exploration after pruner wastes tokens and time — the whole point of pruner is to eliminate exploration overhead.
