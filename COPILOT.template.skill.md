## Pruner — automatic code context

Pruner is a tree-sitter-based code indexer that pre-analyzes the entire codebase. It builds a call graph, symbol index, and file dependency map at index time, then uses your prompt keywords to find the most relevant execution paths, files, and code snippets. The output is **equivalent to you running dozens of grep/glob/view calls** — but computed in seconds from a pre-built index.

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase:

1. **Get context**: `pruner context . "<task description>"` — returns the key files, symbols, call chains, and code snippets for your task.
2. **Read and trust the context.** You do not need to verify these pointers with additional searches. Only read source files if a snippet is truncated or you need surrounding lines.
3. **Do not re-explore** — pruner already searched the full index. Skip grep/glob/rg for the same keywords. Go straight to reading the files pruner pointed you to.
