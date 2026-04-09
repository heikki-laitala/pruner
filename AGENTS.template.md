## Pruner — automatic code context

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase:

1. Get context: `pruner context . "<task description>"` and read the output first.
2. Work directly from the output. Only read source files if a snippet is truncated or you need surrounding context.
3. Use `--detail` when brief pointers are not enough for tracing or debugging.
4. Do not re-explore the same query with grep, glob, or rg right after pruner.
