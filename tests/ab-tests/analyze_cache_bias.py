#!/usr/bin/env python3
"""Analyze prompt-cache bias in A/B test results.

Reads results.json and results_multi_repo.json, deduplicates per_message_usage
streaming events to recover per-API-call cache breakdowns, then estimates what
costs would be without caching.

WARNING: The per_message_usage data from Claude Code's stream-json does not
reliably reconcile with reported input_tokens for most runs (2-72x discrepancy).
The discount factors and no-cache cost estimates in this report are therefore
unreliable. Only the directional analysis (more API calls = more cache benefit)
and tool call counts can be trusted. See the DATA QUALITY section in the output.
"""

import json, sys
from pathlib import Path

TESTS_DIR = Path(__file__).parent

# Cache pricing ratios (stable across models):
#   cache_read  = 0.1  * input_price  (90% discount)
#   cache_write = 1.25 * input_price  (25% surcharge)
CACHE_READ_FACTOR = 0.1
CACHE_WRITE_FACTOR = 1.25


def dedup_usage(per_message_usage):
    """Deduplicate adjacent streaming events to get one entry per real API call.

    Claude Code's stream-json emits multiple 'assistant' events per API call
    (streaming chunks). Adjacent events for the same call share identical usage.
    Only collapse consecutive duplicates — two different API calls can have the
    same token profile (e.g. repeated tool-loop calls).
    """
    calls = []
    prev_key = None
    for u in per_message_usage:
        key = (u["input_tokens"], u["cache_read"], u["cache_creation"])
        if key != prev_key:
            calls.append(u)
            prev_key = key
    return calls


def analyze_run(side_data):
    """Analyze a single run's cache behavior."""
    pmu = side_data.get("per_message_usage", [])
    if not pmu:
        return None

    calls = dedup_usage(pmu)
    total_fresh = sum(u["input_tokens"] for u in calls)
    total_cache_read = sum(u["cache_read"] for u in calls)
    total_cache_create = sum(u["cache_creation"] for u in calls)
    total_input = total_fresh + total_cache_read + total_cache_create

    reported_input = side_data["input_tokens"]
    input_match = abs(total_input - reported_input) < 100

    if total_input > 0:
        discount_factor = (
            total_fresh * 1.0
            + total_cache_read * CACHE_READ_FACTOR
            + total_cache_create * CACHE_WRITE_FACTOR
        ) / total_input
    else:
        discount_factor = 1.0

    reported_cost = side_data["cost_usd"]

    # Estimate no-cache cost by separating input and output components.
    # Output tokens are not affected by cache pricing, so only the input
    # portion should be scaled by the discount factor.
    #
    # We estimate output cost from token counts and the known ratio between
    # input and output pricing (output = 5x input for Opus-class models).
    # input_cost_cached = total_input * discount_factor * P_input
    # output_cost       = output_tokens * 5 * P_input
    # reported_cost     = input_cost_cached + output_cost
    #
    # Solving for P_input:
    #   P_input = reported_cost / (total_input * discount_factor + output_tokens * 5)
    # Then:
    #   nocache_cost = total_input * P_input + output_tokens * 5 * P_input
    output_tokens = side_data.get("output_tokens", 0)
    OUTPUT_RATIO = 5.0  # output price / input price
    denom = total_input * discount_factor + output_tokens * OUTPUT_RATIO
    if denom > 0:
        p_input = reported_cost / denom
        nocache_cost_est = total_input * p_input + output_tokens * OUTPUT_RATIO * p_input
    else:
        nocache_cost_est = reported_cost

    first_call = calls[0] if calls else None
    cross_conv_cache_read = first_call["cache_read"] if first_call else 0
    first_total = (first_call["input_tokens"] + first_call["cache_read"]
                   + first_call["cache_creation"]) if first_call else 0

    return {
        "api_calls": len(calls),
        "total_fresh": total_fresh,
        "total_cache_read": total_cache_read,
        "total_cache_create": total_cache_create,
        "total_input": total_input,
        "reported_input": reported_input,
        "input_match": input_match,
        "cache_read_pct": round(total_cache_read / total_input * 100, 1) if total_input else 0,
        "discount_factor": round(discount_factor, 4),
        "reported_cost": reported_cost,
        "nocache_cost_est": round(nocache_cost_est, 4),
        "cross_conv_cache_read": cross_conv_cache_read,
        "cross_conv_pct": round(cross_conv_cache_read / first_total * 100, 1) if first_total else 0,
        "first_call_cache_read": cross_conv_cache_read,
        "first_call_total": first_total,
    }


def load_single_repo(path):
    """Load results.json (per-category tasks). Returns {label: [info, ...]}."""
    data = json.loads(path.read_text())
    all_tasks = {}

    for rnd in data["rounds"]:
        round_num = rnd["round"]
        for task in rnd["tasks"]:
            cat = task["category"]
            all_tasks.setdefault(cat, {"with": [], "without": []})
            for side_key, label in [("without", "without"), ("with_pruner", "with")]:
                side = task.get(side_key)
                if not side:
                    continue
                info = analyze_run(side)
                if info:
                    info["round"] = round_num
                    info["tool_calls"] = side.get("tool_calls", 0)
                    info["wall_time_s"] = side.get("wall_time_s", 0)
                    all_tasks[cat][label].append(info)

    return all_tasks


def load_multi_repo(path):
    """Load results_multi_repo.json (per-repo results). Returns {label: [info, ...]}."""
    data = json.loads(path.read_text())
    all_repos = {}

    for rnd in data["rounds"]:
        round_num = rnd["round"]
        for result in rnd["results"]:
            repo = result["repo"]
            all_repos.setdefault(repo, {"with": [], "without": []})
            for side_key, label in [("without", "without"), ("with_pruner", "with")]:
                side = result.get(side_key)
                if not side:
                    continue
                info = analyze_run(side)
                if info:
                    info["round"] = round_num
                    info["tool_calls"] = side.get("tool_calls", 0)
                    info["wall_time_s"] = side.get("wall_time_s", 0)
                    all_repos[repo][label].append(info)

    return all_repos


def print_analysis(title, all_groups, group_label="Category"):
    """Print all analysis tables for a dataset."""
    print()
    print("#" * 120)
    print(f"#  {title}")
    print("#" * 120)
    print()

    # ── Table 0: Data quality ────────────────────────────────────────
    print("=" * 120)
    print("DATA QUALITY: per_message_usage vs reported input_tokens")
    print("  Deduped = sum of per-message input tokens after adjacent dedup")
    print("  Reported = input_tokens from result event (authoritative billing total)")
    print("  Runs where these diverge significantly have unreliable cache breakdowns.")
    print("=" * 120)
    print(f"{group_label:<18} {'Side':<8} {'Rnd':>3} {'Deduped':>12} {'Reported':>12} {'Ratio':>8} {'Status':>10}")
    print("-" * 80)

    match_count = 0
    total_count = 0
    for grp in sorted(all_groups.keys()):
        for label in ["without", "with"]:
            for info in all_groups[grp][label]:
                total_count += 1
                reported = info.get("reported_input", 0)
                deduped = info["total_input"]
                ratio = deduped / reported if reported else 0
                matched = info["input_match"]
                if matched:
                    match_count += 1
                status = "ok" if matched else "MISMATCH"
                print(f"{grp:<18} {label:<8} R{info['round']:>1} "
                      f"{deduped:>12,} {reported:>12,} {ratio:>7.2f}x {status:>10}")
        print()

    print(f"  Reconciled: {match_count}/{total_count} runs. "
          f"{'ALL GOOD' if match_count == total_count else 'WARNING: cache analysis unreliable for mismatched runs.'}")
    print()

    # ── Table 1: Per-run cache breakdown ──────────────────────────────
    print("=" * 120)
    print("PER-RUN CACHE BREAKDOWN (WARNING: unreliable for MISMATCH runs above)")
    print("=" * 120)
    print(f"{group_label:<18} {'Side':<8} {'Rnd':>3} {'APICalls':>8} "
          f"{'Fresh':>10} {'CacheRead':>12} {'CacheCreate':>12} "
          f"{'CacheRd%':>9} {'Discount':>9} {'Match':>5}")
    print("-" * 120)

    for grp in sorted(all_groups.keys()):
        for label in ["without", "with"]:
            for info in all_groups[grp][label]:
                print(f"{grp:<18} {label:<8} R{info['round']:>1} {info['api_calls']:>8} "
                      f"{info['total_fresh']:>10,} {info['total_cache_read']:>12,} "
                      f"{info['total_cache_create']:>12,} "
                      f"{info['cache_read_pct']:>8.1f}% {info['discount_factor']:>8.4f} "
                      f"{'ok' if info['input_match'] else 'MISMATCH':>5}")
        print()

    # ── Table 2: Cross-conversation contamination ─────────────────────
    print("=" * 120)
    print("CROSS-CONVERSATION CACHE (turn-1 cache_read = tokens warm from previous runs)")
    print("=" * 120)
    print(f"{group_label:<18} {'Side':<8} {'Rnd':>3} {'Turn1 CacheRead':>16} {'Turn1 Total':>12} {'CrossConv%':>11}")
    print("-" * 80)

    for grp in sorted(all_groups.keys()):
        for label in ["without", "with"]:
            for info in all_groups[grp][label]:
                print(f"{grp:<18} {label:<8} R{info['round']:>1} "
                      f"{info['first_call_cache_read']:>16,} "
                      f"{info['first_call_total']:>12,} "
                      f"{info['cross_conv_pct']:>10.1f}%")
        print()

    # ── Table 3: Cost comparison — reported vs no-cache estimate ──────
    print("=" * 120)
    print("COST COMPARISON: REPORTED (with caching) vs ESTIMATED NO-CACHE")
    print("=" * 120)
    print(f"{group_label:<18} {'Side':<8} {'Rnd':>3} {'Reported$':>10} {'NoCache$':>10} "
          f"{'CacheSaving':>12}")
    print("-" * 75)

    for grp in sorted(all_groups.keys()):
        for label in ["without", "with"]:
            for info in all_groups[grp][label]:
                saving = (1 - info["reported_cost"] / info["nocache_cost_est"]) * 100
                print(f"{grp:<18} {label:<8} R{info['round']:>1} "
                      f"${info['reported_cost']:>9.4f} ${info['nocache_cost_est']:>9.4f} "
                      f"{saving:>11.1f}%")
        print()

    # ── Table 4: Summary — reported delta vs cache-normalized delta ───
    print("=" * 120)
    print("SUMMARY: REPORTED vs CACHE-NORMALIZED COST DELTAS")
    print("=" * 120)
    print(f"{group_label:<18} {'W/O Reported':>12} {'W/ Reported':>12} {'D Reported':>11} "
          f"{'W/O NoCache':>12} {'W/ NoCache':>12} {'D NoCache':>11} {'Shift':>8}")
    print("-" * 100)

    for grp in sorted(all_groups.keys()):
        with_runs = all_groups[grp]["with"]
        without_runs = all_groups[grp]["without"]
        if not with_runs or not without_runs:
            continue

        wo_rep = sum(r["reported_cost"] for r in without_runs) / len(without_runs)
        w_rep = sum(r["reported_cost"] for r in with_runs) / len(with_runs)
        delta_rep = (w_rep - wo_rep) / wo_rep * 100

        wo_nc = sum(r["nocache_cost_est"] for r in without_runs) / len(without_runs)
        w_nc = sum(r["nocache_cost_est"] for r in with_runs) / len(with_runs)
        delta_nc = (w_nc - wo_nc) / wo_nc * 100

        shift = delta_nc - delta_rep
        print(f"{grp:<18} ${wo_rep:>11.4f} ${w_rep:>11.4f} {delta_rep:>+10.1f}% "
              f"${wo_nc:>11.4f} ${w_nc:>11.4f} {delta_nc:>+10.1f}% {shift:>+7.1f}pp")

    # ── Table 5: Discount factor comparison ───────────────────────────
    print()
    print("=" * 120)
    print("CACHE DISCOUNT FACTOR BY SIDE (lower = more cache benefit)")
    print("  1.0 = no caching, 0.1 = fully cached reads, 1.25 = fully cache writes")
    print("=" * 120)
    print(f"{group_label:<18} {'W/O Discount':>14} {'W/ Discount':>14} {'Difference':>12} {'Bias Direction':>16}")
    print("-" * 80)

    for grp in sorted(all_groups.keys()):
        with_runs = all_groups[grp]["with"]
        without_runs = all_groups[grp]["without"]
        if not with_runs or not without_runs:
            continue

        wo_df = sum(r["discount_factor"] for r in without_runs) / len(without_runs)
        w_df = sum(r["discount_factor"] for r in with_runs) / len(with_runs)
        diff = w_df - wo_df
        bias = "helps 'without'" if diff > 0.01 else "helps 'with'" if diff < -0.01 else "neutral"
        print(f"{grp:<18} {wo_df:>14.4f} {w_df:>14.4f} {diff:>+11.4f} {bias:>16}")

    # ── Table 6: Tool calls (cache-independent) ──────────────────────
    print()
    print("=" * 120)
    print("TOOL CALLS (cache-independent — purely behavioral, not affected by prompt caching)")
    print("=" * 120)
    print(f"{group_label:<18} {'Side':<8} {'Rnd':>3} {'ToolCalls':>10} {'APICalls':>9} {'WallTime':>10}")
    print("-" * 65)

    for grp in sorted(all_groups.keys()):
        for label in ["without", "with"]:
            for info in all_groups[grp][label]:
                print(f"{grp:<18} {label:<8} R{info['round']:>1} "
                      f"{info['tool_calls']:>10} {info['api_calls']:>9} "
                      f"{info['wall_time_s']:>9.1f}s")
        print()

    # ── Table 7: Tool calls & wall time deltas ────────────────────────
    print("=" * 120)
    print("TOOL CALLS & WALL TIME DELTAS (mean across rounds)")
    print("=" * 120)
    print(f"{group_label:<18} {'W/O Tools':>10} {'W/ Tools':>10} {'D Tools':>9} "
          f"{'W/O Time':>10} {'W/ Time':>10} {'D Time':>9} "
          f"{'W/O API':>8} {'W/ API':>8} {'s/call W/O':>11} {'s/call W/':>10}")
    print("-" * 120)

    for grp in sorted(all_groups.keys()):
        with_runs = all_groups[grp]["with"]
        without_runs = all_groups[grp]["without"]
        if not with_runs or not without_runs:
            continue

        wo_tc = sum(r["tool_calls"] for r in without_runs) / len(without_runs)
        w_tc = sum(r["tool_calls"] for r in with_runs) / len(with_runs)
        delta_tc = (w_tc - wo_tc) / wo_tc * 100 if wo_tc else 0

        wo_wt = sum(r["wall_time_s"] for r in without_runs) / len(without_runs)
        w_wt = sum(r["wall_time_s"] for r in with_runs) / len(with_runs)
        delta_wt = (w_wt - wo_wt) / wo_wt * 100 if wo_wt else 0

        wo_api = sum(r["api_calls"] for r in without_runs) / len(without_runs)
        w_api = sum(r["api_calls"] for r in with_runs) / len(with_runs)
        wo_s_per = wo_wt / wo_api if wo_api else 0
        w_s_per = w_wt / w_api if w_api else 0

        print(f"{grp:<18} {wo_tc:>10.1f} {w_tc:>10.1f} {delta_tc:>+8.0f}% "
              f"{wo_wt:>9.1f}s {w_wt:>9.1f}s {delta_wt:>+8.0f}% "
              f"{wo_api:>8.1f} {w_api:>8.1f} {wo_s_per:>10.1f}s {w_s_per:>9.1f}s")

    # ── Table 8: Summary of cache impact by metric ────────────────────
    print()
    print("=" * 120)
    print("SUMMARY: CACHE IMPACT BY METRIC")
    print("=" * 120)
    print("""
  Metric        Cache impact                                    Reported deltas reliable?
  -----------   ---------------------------------------------   -----------------------------------------------
  Tool calls    None - purely behavioral, cache is transparent  Yes, accurate as reported
                to the model. Same tokens, same decisions.
  Cost          Helps "without" more (more API calls = more     Mostly conservative; implement overstated by ~8pp
                cached prefix re-sends at discounted price)
  Wall time     Helps "without" more (faster TTFT per cached    Likely conservative for same reason as cost
                API call, compounded over 3-10x more calls)
""")


def main():
    results_single = TESTS_DIR / "results.json"
    results_multi = TESTS_DIR / "results_multi_repo.json"

    if results_single.exists():
        single = load_single_repo(results_single)
        n_rounds = len(json.loads(results_single.read_text())["rounds"])
        print_analysis(
            f"SINGLE-REPO A/B TEST (results.json, {n_rounds} rounds, by task category)",
            single,
            group_label="Category",
        )

    if results_multi.exists():
        multi = load_multi_repo(results_multi)
        n_rounds = len(json.loads(results_multi.read_text())["rounds"])
        print_analysis(
            f"MULTI-REPO A/B TEST (results_multi_repo.json, {n_rounds} rounds, by repo)",
            multi,
            group_label="Repo",
        )


if __name__ == "__main__":
    main()
