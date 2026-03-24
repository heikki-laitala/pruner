## Pruner — automatic code context

Pruner is a tree-sitter-based code indexer that pre-analyzes the entire codebase. It builds a call graph, symbol index, and file dependency map at index time, then uses your prompt keywords to find the most relevant execution paths, files, and code snippets. The output is **equivalent to you running dozens of grep/glob/view calls** — but computed in seconds from a pre-built index.

A background hook generates `.pruner/copilot-context.md` on every prompt. **Always check for it before exploring the codebase.**

1. **Check for `.pruner/copilot-context.md`** using `cat .pruner/copilot-context.md` (bash) or `Get-Content .pruner\copilot-context.md` (PowerShell). Do not use glob — dotfiles may be hidden from glob. If the file doesn't exist yet, the hook may still be running. Wait 5 seconds and check again. Retry up to 3 times.
2. **Read and trust the context.** It contains the key files, symbols, call chains, and code snippets for your task — extracted from a complete index of the repo. You do not need to verify these pointers with additional searches.
3. **If the file still doesn't exist after retries**, fall back to running `pruner context . "<task description>"` manually.
4. **Work directly** from pruner output. Only read source files if a snippet is truncated or you need surrounding lines.
5. **Do not re-explore** — pruner already searched the full index. Skip grep/glob/rg for the same keywords. Go straight to reading the files pruner pointed you to.
