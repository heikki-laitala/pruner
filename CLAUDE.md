## Pruner — automatic code context

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase:

1. **Orient**: `pruner context . "<task description>" --brief` — prints a compact table of contents (~500 tokens): key files, symbols with locations, execution path count.
2. **Read**: Open the top 3-5 key files listed. Use symbol locations (file:line) to jump to relevant code.
3. **Deep dive** (only if needed): `pruner context . "<task>"` (no `--brief`) for full execution paths and snippets.

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