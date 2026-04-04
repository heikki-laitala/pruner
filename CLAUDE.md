# Pruner

## Automatic code context

Before making code changes, fixing bugs, refactoring, or answering questions about this codebase:

1. **Get context**: `pruner context . "<task description>"` — auto-detects task scope. Returns brief pointers for narrow tasks, focused context with code snippets for broad tasks.
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

## Validating Changes

When changing scoring, ranking, context output, or anything that affects pruner's suggestions:

1. **Posthoc analysis first** (~5 seconds, no API calls):
   ```bash
   cargo build --release
   python3 tests/posthoc_analysis.py tests/ab-tests/fast_implement_n10.json --repo /tmp/pruner-bench/nest --pruner ./target/release/pruner
   ```
   Check: did recall improve? Did precision change? Which files are still missed?

2. **Fast A/B test if posthoc looks good** (~15-20 min, ~$1-2):
   ```bash
   python3 tests/ab_test.py --fast --task implement --rounds 5 --save-raw --validate-cache
   ```
   Check: did cost/tools/time improve in real Claude sessions?

3. **Full A/B test for final validation** (~45 min, ~$5):
   ```bash
   python3 tests/ab_test.py --save-raw --validate-cache
   ```

Step 1 replaces a 20+ min A/B test with a 5-second offline check. Only proceed to step 2 when posthoc shows improvement. Step 3 is for release validation only.

The test repos need to be cloned first (the A/B test harness does this automatically, or manually: `git clone https://github.com/nestjs/nest.git /tmp/pruner-bench/nest`).

## Conventions

- **Git**: Conventional commits (`feat:`, `fix:`, `chore:`, `refactor:`, `test:`, `ci:`). No Co-Authored-By trailer. No "Generated with Claude Code" footer in PR descriptions.