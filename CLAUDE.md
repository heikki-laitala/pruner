## Pruner — automatic code context

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase, run pruner to get focused context:

```bash
pruner context . "<task description>" --brief
```

This auto-indexes if needed, prints a compact summary, and writes full context to `.pruner/context.md`. Read only the key files listed. Use Read/Grep on `.pruner/context.md` for details (snippets, call graphs, imports).

## Engineering Principles

### KISS

Prefer straightforward control flow. Keep error paths obvious and localized.

### YAGNI

Do not add interfaces, config keys, or abstractions without a concrete caller. No speculative features.

### DRY (Rule of Three)

Duplicate small local logic when it preserves clarity. Extract shared helpers only after three repeated, stable patterns.

### TDD

Write tests first. Red → Green → Refactor. New features and bug fixes start with a failing test that defines the expected behavior before writing implementation code.

### Secure by Default

Never log secrets or tokens. Validate at system boundaries. Keep network/filesystem/shell scope narrow.