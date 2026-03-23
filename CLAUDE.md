## Pruner — automatic code context

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase:

1. **Get context**: `pruner context . "<task description>"` — returns focused context (~10-15K tokens) with key files, symbols, execution paths, and code snippets.
2. **Work directly** from the output. Only read source files if a snippet is truncated or you need surrounding context.
3. **Do not re-explore** — pruner already searched the index. Skip grep/glob for the same keywords.

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

## Conventions

- **Git**: Conventional commits (`feat:`, `fix:`, `chore:`, `refactor:`, `test:`, `ci:`). No Co-Authored-By trailer. No "Generated with Claude Code" footer in PR descriptions.