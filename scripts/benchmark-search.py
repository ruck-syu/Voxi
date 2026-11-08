#!/usr/bin/env python3
"""Benchmark actual Voxi search code and preserve raw samples + Markdown."""
import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CRATE = ROOT / "app/src-tauri"


def capture(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def cached_build(output):
    """Use one coherent dependency set from an existing Cargo test build."""
    target = CRATE / "target/debug"
    fingerprints = target / ".fingerprint"
    needed = {"serde", "serde_json", "regex", "chrono", "tempfile", "objc2",
              "objc2_foundation", "objc2_app_kit", "objc2_pdf_kit"}
    artifacts = {}
    for directory in fingerprints.iterdir():
        for name in needed:
            fingerprint = directory / f"lib-{name}"
            if fingerprint.is_file():
                number = int.from_bytes(bytes.fromhex(fingerprint.read_text()), "little")
                artifact = target / "deps" / f"lib{name}-{directory.name.rsplit('-', 1)[1]}.rlib"
                if artifact.exists():
                    artifacts[(name, number)] = artifact
    candidates = sorted(fingerprints.glob("voxi-*/test-lib-voxi_lib.json"),
                        key=lambda p: p.stat().st_mtime, reverse=True)
    for candidate in candidates:
        metadata = json.loads(candidate.read_text())
        deps = [(name, number) for _, name, _, number in metadata["deps"] if name in needed]
        if {name for name, _ in deps} == needed and all(dep in artifacts for dep in deps):
            command = ["rustc", "--edition=2024", "-C", "opt-level=3",
                       str(CRATE / "benches/search.rs"), "-L", f"dependency={target / 'deps'}",
                       "-o", str(output)]
            for dep in deps:
                command += ["--extern", f"{dep[0]}={artifacts[dep]}"]
            subprocess.run(command, cwd=CRATE, env=dict(os.environ, CARGO_MANIFEST_DIR=str(CRATE)), check=True)
            return {"mode": "cached-deps", "rustc_flags": "--edition=2024 -C opt-level=3",
                    "dependencies": {name: artifacts[(name, number)].name for name, number in deps},
                    "note": "Production search modules optimized at level 3; dependencies reused from a debug/test build. Compare with this same build mode, not a fully optimized Cargo release."}
    raise SystemExit("No coherent cached dependency set. Run without --cached-deps to build with Cargo.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cached-deps", action="store_true", help="reuse local dependencies without network access")
    parser.add_argument("--output", type=Path, help="Markdown output path; JSON saved alongside")
    args = parser.parse_args()
    date = datetime.datetime.now(datetime.timezone.utc).isoformat()
    output = (args.output or ROOT / "docs/benchmarks" / f"search-{date[:10]}.md").resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    raw = output.with_suffix(".json")
    sources = [*sorted((CRATE / "src").rglob("*.rs")), CRATE / "Cargo.toml",
               CRATE / "benches/search.rs", Path(__file__).resolve()]
    hashes = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
    metadata = {"timestamp_utc": date, "commit": capture("git", "rev-parse", "HEAD"),
                "git_status": capture("git", "status", "--short"), "source_sha256": hashes,
                "os": capture("sw_vers"), "cpu": capture("sysctl", "-n", "machdep.cpu.brand_string"),
                "memory_bytes": int(capture("sysctl", "-n", "hw.memsize")),
                "architecture": platform.machine(), "rustc": capture("rustc", "--version")}
    env = dict(os.environ, VOXI_BENCH_OUTPUT=str(raw))
    with tempfile.TemporaryDirectory(prefix="voxi-benchmark-build-") as temp:
        if args.cached_deps:
            binary = Path(temp) / "search"
            metadata["build"] = cached_build(binary)
            subprocess.run([str(binary)], cwd=CRATE, env=env, check=True)
        else:
            metadata["build"] = {"mode": "cargo-release", "note": "cargo bench --bench search (release benchmark profile)"}
            subprocess.run(["cargo", "bench", "--bench", "search"], cwd=CRATE, env=env, check=True)
    data = json.loads(raw.read_text())
    data["metadata"] = metadata
    raw.write_text(json.dumps(data, indent=2) + "\n")
    def outcome_label(outcome):
        if outcome.startswith("Search incomplete"): return "incomplete (blocked)"
        if outcome.startswith("Search cancelled"): return "cancelled"
        if outcome.startswith("Couldn't find") or outcome.startswith("No files match"): return "not found"
        return outcome

    rows = []
    incomplete = []
    for result in data["results"]:
        m = result["measurement"]
        samples = sorted(m["samples_ms"])
        counts = sorted(set(m["observed_matches"]))
        if any(count != m["expected_matches"] for count in counts):
            incomplete.append(f"- `{result['fixture']}` / `{result['operation']}`: returned {counts}, expected {m['expected_matches']}.")
        rows.append(f"| {result['fixture']} | {result['operation']} | {statistics.median(samples):.3f} | "
                    f"{samples[math.ceil(.95 * len(samples)) - 1]:.3f} | "
                    f"{','.join(map(str, counts))} / {m['expected_matches']} | "
                    f"{', '.join(sorted(set(outcome_label(o) for o in m['outcomes'])))} |")
    command = "rtk proxy python3 scripts/benchmark-search.py" + (" --cached-deps" if args.cached_deps else "")
    report = f"""# Voxi search benchmark baseline

Recorded: {date}  
Commit: `{metadata['commit']}`  
Working-tree changes at capture: `{metadata['git_status'].replace(chr(10), '; ') or 'none'}`

## Environment

- CPU: {metadata['cpu']} ({metadata['architecture']}); RAM: {metadata['memory_bytes'] / 1024**3:.0f} GiB.
- OS: {metadata['os'].replace(chr(10), '; ')}.
- Compiler: {metadata['rustc']}.
- Build mode: **{metadata['build']['mode']}**. {metadata['build']['note']}

## Method

The benchmark imports the production Rust parser, planner and filesystem walker directly.
Each scenario has {data['warmups']} untimed warmups and {data['repeats']} measured iterations,
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
{chr(10).join(rows)}

## Findings

Exact-name searches perform one complete walk. Normalized-name searches perform
an exact walk followed by a second walk and candidate matching; missing-name
searches also invoke Spotlight. Their timings include these different paths.
The raw samples preserve each outcome so a fallback failure cannot be mistaken for a match.

The following cases did not return all fixture ground truth (check the outcome: an explicit incomplete-search refusal prevents a partial file operation):

{chr(10).join(incomplete) or 'None.'}

An `incomplete (blocked)` outcome is a deliberate refusal to create a preview from
partial discovery. An `ambiguous` outcome requires a user selection; its candidate
count is not an automatically executable action count. Do not treat faster early
refusals as equivalent to a complete successful search.

## Comparison procedure

Run from the repository root:

```sh
{command} --output docs/benchmarks/search-next.md
```

Use the same machine, build mode, fixture layout, warmups and sample count.
Run under similar background load. Compare p50, p95 and found/expected for each row.
The normal fully optimized build is available by omitting `--cached-deps`; establish a separate baseline for that mode.
The runner and fixture live in `scripts/benchmark-search.py` and `app/src-tauri/benches/search.rs`.
All samples, outcomes, source SHA-256 hashes and environment metadata are saved in [{raw.name}]({raw.name}).
Source hashes identify the measured code even when the benchmark files are uncommitted.
"""
    output.write_text(report)
    print(f"Saved {output}\nRaw samples: {raw}")


if __name__ == "__main__":
    main()
