# Luna dual-build command audit — 2026-11-16

## Build identity

The development app is running from the current working tree. `npm run tauri dev`
confirmed the Rust binary was current (`Finished dev profile ... in 0.51s`) and
started it at 02:23:49 IST; Vite serves the current frontend at localhost:1420.
The old dev session exited during initial investigation, so it was restarted.

The installed production app is **0.1.14**, built November 14. Its executable
SHA-256 exactly matches the executable extracted from the published universal
release archive. The matching source tag is `v0.1.14`, commit
`44c74fa65fd0b4a8f9a9ad2c79d435e7c30e3ffb`.

**Both apps use the rule parser.** The local model experiment was rejected for
integration; dev has newer rule and planner fixes, not a deployed model parser.
See [build-verification.json](build-verification.json) for paths and binary hashes.

At continuation on November 16 at 10:39 IST, production was still running and
dev had stopped. Dev was restarted with `npm run tauri dev`; Cargo confirmed
the current binary in 0.76s, and Vite returned HTTP 200 on port 1420. Both binary
hashes still match the tested executables. See
[resume-verification.json](resume-verification.json). The completed tests were not rerun.

## Test coverage

Luna (`gpt-6-luna`) subagents authored three 100-case batches, then independently
reviewed duplicate sentences and labels before any outcomes were inspected.
[commands.jsonl](commands.jsonl) freezes 300 unique sentences, including 99 original
refusal labels, under SHA-256
`851900248b9706c151eb94f9547a65d3f9541c43e8cc3ecbf0d0684ee1cd0fc8`.
Coverage includes quoted command words, nested sources, qualifier attachment,
contradictory bounds, exclusions, chained operations, archive/PDF output names,
schedule/name collisions, positional references, and protected paths.

Both running apps completed **300/300 direct UI submissions each (600 total)**.
Every submitted input was verified, and every result settled. File-changing
previews and schedule previews were never confirmed. The runner reset preview
and list scope between cases and restored the clipboard.

| Native UI category | Dev | Production |
|---|---:|---:|
| Change/schedule preview | 41 | 44 |
| Not understood | 68 | 68 |
| Choice required | 6 | 6 |
| Information or planner/permission problem | 185 | 182 |

These categories describe captured UI states, not pass/fail. Real folders
include missing fixture names and permission/search-limit guards.


The source comparison separately ran the same 300 cases through each version's
unmodified Rust parser, scheduler, scope preparation and planner in isolated
temporary crates. Both used the same synthetic fixture. Fixture contents, sizes
and modification times were unchanged afterward. No planned action was executed.
PDF fixtures have three real pages; image/archive placeholder bytes do not test
actual media conversion or decompression.

## Controlled source results

| Pipeline outcome | Latest dev | Production 0.1.14 source |
|---|---:|---:|
| File-change preview | 117 | 120 |
| Planner problem | 78 | 78 |
| Parser or schedule refusal | 76 | 73 |
| Read-only result/action recorded without execution | 29 | 29 |
| Total | 300 | 300 |

These are pipeline counts, not pass rates. In particular, a preview can
misinterpret a request, and a missing fixture match can prevent an otherwise
valid command from producing a preview.

### Confirmed improvements in dev

- **F085:** `move files from Desktop to Documents excluding files larger than 1 GB`.
  Dev refuses the unsupported exclusion. Production source treats the exclusion
  as destination text and offers a move into that newly named folder.
- **T045:** `Sort Downloads by type and then by month`. Dev refuses the second
  operation; production source accepts a malformed source folder, then its
  planner stops in the fixture.
- **T065:** `Merge all PDFs in Downloads into combined.pdf and delete the originals`.
  Dev refuses the chained operation. Production source offers a merge whose
  output name swallows `and delete originals`; it does not actually plan deletion.

### Remaining interpretation failures

- **F076, both sources:** `move PDFs from Downloads to Documents larger than 1 GB smaller than 100 MB`.
  The contradictory size wording becomes part of the destination, and the
  fixture produces unfiltered PDF moves into a newly named folder.
- **F080/F081/F094, both:** repeated size bounds, conflicting days or a missing
  size unit after the destination similarly become destination text.
- **S012, both:** `Move files from Downloads to Documents before noon tomorrow`
  produces destination `Documents before noon tomorrow`, with no schedule.
  That schedule phrase is not documented, but silently converting it to a
  destination is an interpretation problem.
- **T100, both:** `Zip Backup into Backup.zip with maximum compression` creates
  an archive plan with the compression instruction in its filename.
- **F070, both:** a move with the size-range phrase `between 10 and 50 MB` is
  refused. Its supplied canonical reference is also refused, so canonical
  equality alone cannot validate the expectation.

F001 also exposes an intentional newer planner rule: dev moves only direct
Downloads PDFs; production source also includes nested `Downloads/Archive/old.pdf`.
The current README documents the dev behavior.

### Direct UI confirmation

The actual installed production app offered T065 as a merge preview named
`combined.pdf and delete originals.pdf`. The actual dev app refused the same
sentence. Dev also refused F085 and T045 in the UI; the production app reached
search/planner errors instead of rejecting their grammar. Several other source
misreads stopped at the real app's search depth or permission guards, so the
fixture's incorrect previews are not all reproduced as live previews.

## Scoring and review limits

There is **no defensible overall accuracy percentage** from these labels.
Some refusal labels describe supported or ambiguous requests. Examples include
type-filtered moves without a named source, a repeat interval with a clock time,
and replacement of spaces in one explicitly quoted file. Unquoted `Archive`
is ambiguous between a literal item and a file-type noun.

The first Luna summary's raw `70/200` comparison included echoed input strings.
The corrected projection compares intent fields and schedule semantics only:
both sources match **180/192 accepted canonical references**. Of these, only
42 use wording that differs beyond capitalization, and 30 of those match.
Eight canonical references refuse. These figures measure agreement with the
supplied references, not correctness.

Three Luna agents also reviewed all 300 pairs of structured outcomes. Parent
validation found five review assertions inconsistent with captured stages/plans,
including calling dev's F085 refusal an accepted move. Those raw annotations are
preserved, but their aggregate verdict counts are not used as accuracy scores.
The inconsistencies and additional label cautions are recorded in
[verified-summary.json](verified-summary.json).

Actual UI runs use the user's real folders rather than the synthetic fixture.
Missing sample filenames, unreadable folders and search depth/entry limits can
stop planning; those outcomes do not establish parser correctness or universal
safety. Source replay is the controlled comparison. Confirmation, file-changing execution,
undo effects and scheduled task execution were not tested. The read-only
“Open Downloads in Finder” command navigates Finder immediately as designed.

## Evidence and reproduction

- [Verified summary and evidence checks](verified-summary.json)
- [Source manifest and module hashes](manifest.json)
- [Dev source results](dev-results.jsonl)
- [Production source results](production-source-results.jsonl)
- [Dev native UI results](dev-runtime-results.jsonl)
- [Production native UI results](production-runtime-results.jsonl)
- [Original Luna source summary, with parent correction](source-summary.md)
- [Luna semantic reviews: files](review-F.jsonl), [transforms](review-T.jsonl), [scheduling/scope](review-S.jsonl)
- [Pre-outcome corpus adjustments](corpus-label-adjustments.jsonl)

From the repository root:

```sh
rtk proxy python3 docs/testing/luna-dual-build-2026-11-16/replay.py
rtk proxy python3 docs/testing/luna-dual-build-2026-11-16/summarize.py
```

The native runner requires Accessibility, an unlocked Mac, and the exact target
PID. It verifies each nonempty input before sending one Enter, captures the
result and cancels pending previews. It preserves the clipboard. Its output is
write-once to retain evidence. An optional `--fast` resets each completed result
with Escape, clearing preview and list scope before the next case.

No application implementation files were changed for this audit.

## Modern Web Guidance

I used the `accessibility` guide for the inline results chart: “Provide semantic
data tables as alternatives.” The chart includes a semantic table and textual
description, and its numeric labels appear outside colored segments so they do
not depend on foreground contrast against the series colors.
