## Pruner — automatic code context

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase:

1. **Get context**: `pruner context . "<task description>"` — auto-detects task scope. Returns brief pointers for narrow tasks, focused context with code snippets for broad tasks.
2. **Work directly** from the output. Only read source files if a snippet is truncated or you need surrounding context.
3. **Do not re-explore** — pruner already searched the index. Skip grep/glob for the same keywords.
