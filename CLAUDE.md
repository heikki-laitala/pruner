## Pruner — automatic code context

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase, run pruner to get focused context:

```bash
pruner context . "<task description>" --format text
```

If `.pruner/index.db` does not exist, index first:

```bash
pruner index .
```

Use the output (execution paths, key files, symbols, tests) to understand the relevant code before proceeding. This saves tokens and improves accuracy.
