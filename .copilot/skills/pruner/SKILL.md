---
name: pruner
description: Generate code context for coding tasks. Run before code changes, bug fixes, or refactoring to get execution paths, key files, symbols, tests, and snippets.
metadata:
  tags: context, indexing, code-analysis, tree-sitter
---

# Pruner — Code Context Engine

Run one command to get context:

```bash
pruner context /absolute/path/to/repo "<the user's ask>"
```

Use an absolute repo path. Avoid `cd ... && pruner context .` in tool calls.

## Workflow

1. Run `pruner context` first.
2. Work directly from the output.
3. Read source files only if a snippet is truncated or you need nearby lines.
4. Do not re-explore the same query with grep/glob right after pruner.

## More detail

- `pruner show-symbol /path/to/repo "<name>"` for callers/callees/signature.
- `pruner show-file /path/to/repo "<path>"` for symbols/imports in one file.

## Modes

- `--brief` metadata only (~3K tokens)
- `--full` uncapped detail (~50-70K tokens on large repos)
