## Pruner — automatic code context

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase:

1. **Orient**: `pruner context . "<task description>" --brief` — prints a compact table of contents (~500 tokens): key files, symbols with locations, execution path count. Also writes full context to `.pruner/context.md`.
2. **Read**: Open the top 3-5 key files listed. Use symbol locations (file:line) to jump to relevant code.
3. **Drill deeper** (only if needed): Grep `.pruner/context.md` for a symbol name to get its snippet and call graph.
