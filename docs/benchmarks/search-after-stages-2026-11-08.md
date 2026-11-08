# Voxi search benchmark baseline

Recorded: 2026-11-08T17:10:35.390054+00:00  
Commit: `5a1654573e0960e2f9abb1b401e0255f99551a5e`  
Working-tree changes at capture: `M README.md;  M app/src-tauri/Cargo.toml;  M app/src-tauri/src/lib.rs;  M app/src-tauri/src/plan.rs;  M app/src/App.tsx;  M app/src/api.ts;  M app/src/mock.ts; ?? app/src-tauri/benches/; ?? app/src-tauri/src/search.rs; ?? docs/benchmarks/; ?? scripts/`

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

| Fixture | Operation | p50 ms | p95 ms | Found / expected | Outcome |
|---|---|---:|---:|---:|---|
| flat-1k | walk | 1.695 | 1.984 | 1000 / 1000 | walk |
| flat-1k | group | 2.808 | 3.179 | 10 / 10 | plan |
| flat-1k | exact | 2.047 | 2.216 | 1 / 1 | plan |
| flat-1k | normalized | 2.309 | 2.713 | 1 / 1 | ambiguous |
| flat-1k | missing | 48.411 | 54.331 | 0 / 0 | not found |
| flat-10k | walk | 20.512 | 21.313 | 10000 / 10000 | walk |
| flat-10k | group | 27.816 | 28.994 | 100 / 100 | plan |
| flat-10k | exact | 21.460 | 22.765 | 1 / 1 | plan |
| flat-10k | normalized | 24.747 | 25.266 | 1 / 1 | ambiguous |
| flat-10k | missing | 71.597 | 72.954 | 0 / 0 | not found |
| nested-10k | walk | 15.172 | 15.867 | 10000 / 10000 | walk |
| nested-10k | group | 23.350 | 23.760 | 100 / 100 | plan |
| nested-10k | exact | 16.229 | 16.562 | 1 / 1 | plan |
| nested-10k | normalized | 19.712 | 20.259 | 1 / 1 | ambiguous |
| nested-10k | missing | 67.280 | 101.675 | 0 / 0 | not found |
| nested-40k | walk | 61.462 | 62.359 | 40000 / 40000 | walk |
| nested-40k | group | 93.661 | 96.383 | 400 / 400 | plan |
| nested-40k | exact | 64.538 | 65.740 | 1 / 1 | plan |
| nested-40k | normalized | 77.501 | 78.588 | 1 / 1 | ambiguous |
| nested-40k | missing | 129.268 | 133.768 | 0 / 0 | not found |
| budget-60k | walk | 75.515 | 76.474 | 49455 / 60000 | walk |
| budget-60k | group | 113.728 | 117.249 | 0 / 600 | incomplete (blocked) |
| budget-60k | exact | 79.481 | 86.733 | 0 / 1 | incomplete (blocked) |
| budget-60k | normalized | 79.345 | 80.215 | 0 / 1 | incomplete (blocked) |
| budget-60k | missing | 79.951 | 83.469 | 0 / 0 | incomplete (blocked) |
| depth-14 | group | 0.418 | 0.726 | 0 / 1 | incomplete (blocked) |

## Findings

Exact-name searches perform one complete walk. Normalized-name searches perform
an exact walk followed by a second walk and candidate matching; missing-name
searches also invoke Spotlight. Their timings include these different paths.
The raw samples preserve each outcome so a fallback failure cannot be mistaken for a match.

The following cases did not return all fixture ground truth (check the outcome: an explicit incomplete-search refusal prevents a partial file operation):

- `budget-60k` / `walk`: returned [49455], expected 60000.
- `budget-60k` / `group`: returned [0], expected 600.
- `budget-60k` / `exact`: returned [0], expected 1.
- `budget-60k` / `normalized`: returned [0], expected 1.
- `depth-14` / `group`: returned [0], expected 1.

An `incomplete (blocked)` outcome is a deliberate refusal to create a preview from
partial discovery. An `ambiguous` outcome requires a user selection; its candidate
count is not an automatically executable action count. Do not treat faster early
refusals as equivalent to a complete successful search.

## Comparison procedure

Run from the repository root:

```sh
rtk proxy python3 scripts/benchmark-search.py --cached-deps --output docs/benchmarks/search-next.md
```

Use the same machine, build mode, fixture layout, warmups and sample count.
Run under similar background load. Compare p50, p95 and found/expected for each row.
The normal fully optimized build is available by omitting `--cached-deps`; establish a separate baseline for that mode.
The runner and fixture live in `scripts/benchmark-search.py` and `app/src-tauri/benches/search.rs`.
All samples, outcomes, source SHA-256 hashes and environment metadata are saved in [search-after-stages-2026-11-08.json](search-after-stages-2026-11-08.json).
Source hashes identify the measured code even when the benchmark files are uncommitted.
