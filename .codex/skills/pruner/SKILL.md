# Pruner — automatic code context

Pruner is a tree-sitter-based code indexer that pre-analyzes the entire codebase. It builds a call graph, symbol index, and file dependency map at index time, then uses your prompt keywords to find the most relevant execution paths, files, and code snippets. The output is equivalent to you running dozens of search and file-read calls, but computed in seconds from a pre-built index.

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase:

1. Get context: `pruner context /absolute/path/to/repo "<task description>"` and read the output first.
2. Work directly from the output. Only read source files if a snippet is truncated or you need nearby lines.
3. Use `--detail` when brief pointers are not enough for tracing or debugging.
4. Do not re-explore the same query with grep, glob, or rg right after pruner.
