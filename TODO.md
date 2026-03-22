# TODO

## Critical: Query performance on large repos

Benchmark on OpenClaw (9.5k files, 30k symbols, 165k calls) shows all 5 queries timeout at 120s. Pruner is unusable on repos this size.

**Root cause:** `trace_execution_path` in `query.rs` does DFS through the call graph via individual SQLite queries per symbol. Broad keywords match hundreds of symbols, each triggering depth-5 DFS through 165k calls = millions of DB round-trips.

**Fix (in priority order):**

1. **Cap matching symbols** — limit to top 20 before tracing execution paths. Currently every match gets traced.
2. **In-memory call graph** — load adjacency list once at query time instead of per-step `calls_by_symbol` DB queries.
3. **Time budget** — abort execution path tracing after 10s, return what we have.
4. **Smarter keyword matching** — "WebSocket reconnection timeout" shouldn't match every function with "timeout" in its name. Need relevance scoring (TF-IDF or at minimum exact-match-first ranking).

## Critical: First query returns empty JSON

The cross_package query ("how does a message flow from webhook to channel handler") returned empty stdout on OpenClaw. Likely the context command crashes or produces no output when matching too many files. Need to investigate and handle gracefully.

## Important: Context is too broad on large repos

The narrow_fix query ("fix WebSocket reconnection timeout") matched 169 files, 592 symbols, 494 execution paths. That's not focused context — it's a dump. Pruner's value is replacing exploration with precision, and this is the opposite.

**Fix:**
- Add relevance scoring so not every keyword-matching symbol is included
- Rank results: exact match > prefix match > substring match
- Limit output: top 10 files, top 20 symbols, top 5 execution paths (brief mode already does this, but the query itself is slow because it computes everything before limiting)

## Bench baseline

After fixing the above, run `make bench` and save the baseline:
```bash
cp tests/bench_results.json tests/bench_baseline.json
```

Future changes can then be measured for regression.
