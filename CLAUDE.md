## Pruner — automatic code context

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase, run pruner to get focused context:

```bash
pruner context . "<task description>" --brief
```

This auto-indexes if needed, prints a compact summary, and writes full context to `.pruner/context.md`. Read only the key files listed. Use Read/Grep on `.pruner/context.md` for details (snippets, call graphs, imports).
