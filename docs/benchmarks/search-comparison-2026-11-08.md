# Search improvements: comparison with saved baseline

Both runs use the same machine, compiler, cached dependency artifacts, fixture generator, 2 warmups and 15 measured samples. Production modules are optimized at level 3 with dependencies from the same debug/test build. These are warm-cache, in-process planning measurements; UI latency, memory and user selection time are excluded. Neither run executes actions or searches user folders.

Baseline: [report](search-baseline-2026-11-08.md), [raw samples](search-baseline-2026-11-08.json).  
After: [report](search-after-stages-2026-11-08.md), [raw samples](search-after-stages-2026-11-08.json).  
[Chart](search-comparison-2026-11-08.html).

## Timing and outcomes

Negative latency changes mean less time. Different outcomes must not be treated as equivalent success. The 60k/depth-limit cases now refuse to create a preview; early refusal is not a full-search performance win. Normalized fixture queries use loose word matching and now require explicit selection rather than automatically choosing the sole candidate. One prompt response is not benchmarked.

| Fixture | Operation | Before p50 ms | After p50 ms | Change | Outcome before → after |
|---|---|---:|---:|---:|---|
| flat-1k | walk | 1.580 | 1.695 | +7.3% | walk → walk |
| flat-1k | group | 2.351 | 2.808 | +19.5% | plan → plan |
| flat-1k | exact | 1.781 | 2.047 | +15.0% | plan → plan |
| flat-1k | normalized | 3.878 | 2.309 | -40.5% | plan → ambiguous |
| flat-1k | missing | 50.321 | 48.411 | -3.8% | not found → not found |
| flat-10k | walk | 20.642 | 20.512 | -0.6% | walk → walk |
| flat-10k | group | 26.783 | 27.816 | +3.9% | plan → plan |
| flat-10k | exact | 20.766 | 21.460 | +3.3% | plan → plan |
| flat-10k | normalized | 44.782 | 24.747 | -44.7% | plan → ambiguous |
| flat-10k | missing | 88.034 | 71.597 | -18.7% | not found → not found |
| nested-10k | walk | 14.668 | 15.172 | +3.4% | walk → walk |
| nested-10k | group | 23.053 | 23.350 | +1.3% | plan → plan |
| nested-10k | exact | 15.799 | 16.229 | +2.7% | plan → plan |
| nested-10k | normalized | 32.728 | 19.712 | -39.8% | plan → ambiguous |
| nested-10k | missing | 79.026 | 67.280 | -14.9% | not found → not found |
| nested-40k | walk | 58.042 | 61.462 | +5.9% | walk → walk |
| nested-40k | group | 97.144 | 93.661 | -3.6% | plan → plan |
| nested-40k | exact | 67.286 | 64.538 | -4.1% | plan → plan |
| nested-40k | normalized | 133.025 | 77.501 | -41.7% | plan → ambiguous |
| nested-40k | missing | 173.512 | 129.268 | -25.5% | not found → not found |
| budget-60k | walk | 71.535 | 75.515 | +5.6% | walk → walk |
| budget-60k | group | 109.702 | 113.728 | +3.7% | plan → incomplete / blocked |
| budget-60k | exact | 202.216 | 79.481 | -60.7% | not found → incomplete / blocked |
| budget-60k | normalized | 229.966 | 79.345 | -65.5% | not found → incomplete / blocked |
| budget-60k | missing | 204.895 | 79.951 | -61.0% | not found → incomplete / blocked |
| depth-14 | group | 0.405 | 0.418 | +3.1% | not found → incomplete / blocked |

## Interpretation

- One cached filesystem snapshot replaces the second filesystem traversal for approximate matching. The normalized-name rows measure the discovery time saved, with the selection behavior change stated above.
- Missing-name queries reuse the snapshot too, then consider conservative typo suggestions and the actual Spotlight fallback. Temporary fixtures are not a controlled Spotlight index benchmark.
- Exact and group searches include new completeness tracking and cancellation checks. Small differences in either direction may be ordinary machine-load variation; this run is not a statistical significance study.
- The 60k fixture still exceeds the entry budget, and the depth-14 fixture still exceeds the depth budget. These limits have not been raised. The improvement is explicit failure instead of silently acting on partial results or claiming a file is absent.
- Read-only commands can display partial results with an incomplete-search explanation. Mutating commands and shortcuts stop. A canceled or superseded UI request cannot publish a new executable preview.

## Verification

123 backend checks passed using the cached Rust test harness (one held-out test ignored). This includes the production parser corpus, native image/PDF/archive operations, nested move/undo, scan caching, unreadable directories, entry/depth/time limits, Spotlight failure/timeout/cancellation, partial read-only reporting and required selection of typo suggestions. The cached harness includes one read-only check of actual Downloads in addition to repository tests. The full Tauri library passed a metadata compilation with its matching cached dependencies, and `npm run build` passed. A packaged application was not rebuilt or installed, and native UI behavior has not been manually exercised for these changes.

To rerun, use the command in the after report with a new output filename. Keep the original baseline unchanged; compare both counts and outcomes alongside the raw timing samples. Source SHA-256 hashes in each raw file identify the measured code.
