# Voxi search benchmark baseline

Recorded: 2026-11-08T16:08:40.087714+00:00  
Commit: `5a1654573e0960e2f9abb1b401e0255f99551a5e`  
Working-tree changes at capture: `M app/src-tauri/Cargo.toml; ?? app/src-tauri/benches/; ?? docs/benchmarks/; ?? scripts/`

## Environment

- CPU: Apple M1 Pro (arm64); RAM: 32 GiB.
- OS: ProductName:		macOS; ProductVersion:		27.0.1; BuildVersion:		26A434.
- Compiler: rustc 1.96.0 (ac68faa20 2026-05-25) (Homebrew).
- Build mode: **cached-deps**. Production search modules optimized at level 3; dependencies reused from a debug/test build. Compare with this same build mode, not a fully optimized Cargo release.

## Method

The benchmark imports the production Rust parser, planner and filesystem walker directly.
Each scenario has 2 untimed warmups and 15 measured iterations,
executed sequentially. p50 is the sample median; p95 is nearest-rank.
With 15 samples, p95 is the maximum observed sample; treat it as an indicative baseline, not a precise tail-latency estimate.
All commands only build previews: no action executor is called, and no user files are searched or changed.
Fixtures contain empty files under disposable temporary homes on this machine's local filesystem.
Fixture setup, cleanup, compilation and history loading are outside the measured intervals.
These are warm-cache measurements; caches are not flushed. Other applications remain running.
Peak memory, cold-cache behavior and UI latency are not measured.

- **walk**: enumerate files using the production recursive walker; reported count is files visited.
- **group**: parse and plan `Delete all the Aeroguard files from Downloads`; one matching PDF per 100 files.
- **exact**: parse and plan deletion of a unique exact filename.
- **normalized**: the same filename dictated with spaces instead of underscores and dots.
- **missing**: absent filename; includes exact scan, approximate scan and the actual Spotlight fallback.
- **flat** fixtures: all files immediately inside Downloads.
- **nested** and **budget** fixtures: 100 files per batch, 10 batches per group, two directory levels.
- **depth-14**: one matching file under 14 nested directories.

Search limits at baseline: depth 12, 50,000 directory entries per walk, Spotlight up to 5,000 hits / 2 seconds.
Directory entries consume the visit budget too. Expected counts are fixture ground truth, not the walk's own output.
Spotlight indexing of temporary fixtures is uncontrolled; missing results and timings must not be interpreted as indexed-search throughput.

## Results

Times are milliseconds. Counts must be compared along with speed: a fast incomplete search is not a successful optimization.

| Fixture | Operation | p50 ms | p95 ms | Found / expected |
|---|---|---:|---:|---:|
| flat-1k | walk | 1.580 | 1.919 | 1000 / 1000 |
| flat-1k | group | 2.351 | 2.976 | 10 / 10 |
| flat-1k | exact | 1.781 | 2.171 | 1 / 1 |
| flat-1k | normalized | 3.878 | 4.108 | 1 / 1 |
| flat-1k | missing | 50.321 | 60.776 | 0 / 0 |
| flat-10k | walk | 20.642 | 21.458 | 10000 / 10000 |
| flat-10k | group | 26.783 | 28.256 | 100 / 100 |
| flat-10k | exact | 20.766 | 21.634 | 1 / 1 |
| flat-10k | normalized | 44.782 | 52.300 | 1 / 1 |
| flat-10k | missing | 88.034 | 104.931 | 0 / 0 |
| nested-10k | walk | 14.668 | 15.695 | 10000 / 10000 |
| nested-10k | group | 23.053 | 24.643 | 100 / 100 |
| nested-10k | exact | 15.799 | 16.451 | 1 / 1 |
| nested-10k | normalized | 32.728 | 34.478 | 1 / 1 |
| nested-10k | missing | 79.026 | 86.234 | 0 / 0 |
| nested-40k | walk | 58.042 | 60.059 | 40000 / 40000 |
| nested-40k | group | 97.144 | 101.683 | 400 / 400 |
| nested-40k | exact | 67.286 | 73.303 | 1 / 1 |
| nested-40k | normalized | 133.025 | 137.455 | 1 / 1 |
| nested-40k | missing | 173.512 | 181.520 | 0 / 0 |
| budget-60k | walk | 71.535 | 72.531 | 49455 / 60000 |
| budget-60k | group | 109.702 | 116.599 | 495 / 600 |
| budget-60k | exact | 202.216 | 207.013 | 0 / 1 |
| budget-60k | normalized | 229.966 | 238.577 | 0 / 1 |
| budget-60k | missing | 204.895 | 210.860 | 0 / 0 |
| depth-14 | group | 0.405 | 0.411 | 0 / 1 |

## Findings

Exact-name searches perform one complete walk. Normalized-name searches perform
an exact walk followed by a second walk and candidate matching; missing-name
searches also invoke Spotlight. Their timings include these different paths.
The raw samples preserve each outcome so a fallback failure cannot be mistaken for a match.

The following cases failed to return all fixture ground truth:

- `budget-60k` / `walk`: returned [49455], expected 60000.
- `budget-60k` / `group`: returned [495], expected 600.
- `budget-60k` / `exact`: returned [0], expected 1.
- `budget-60k` / `normalized`: returned [0], expected 1.
- `depth-14` / `group`: returned [0], expected 1.

These limits currently produce ordinary partial results or "not found" messages;
there is no explicit incomplete-search result. Completeness reporting should be
improved before treating lower times in capped searches as performance wins.

## Comparison procedure

Run from the repository root:

```sh
rtk proxy python3 scripts/benchmark-search.py --cached-deps --output docs/benchmarks/search-next.md
```

Use the same machine, build mode, fixture layout, warmups and sample count.
Run under similar background load. Compare p50, p95 and found/expected for each row.
The normal fully optimized build is available by omitting `--cached-deps`; establish a separate baseline for that mode.
The runner and fixture live in `scripts/benchmark-search.py` and `app/src-tauri/benches/search.rs`.
All samples, outcomes, source SHA-256 hashes and environment metadata are saved in [search-baseline-2026-11-08.json](search-baseline-2026-11-08.json).
Source hashes identify the measured code even when the benchmark files are uncommitted.
