# Tiny v3 model versus production rules — 300 commands

Actual local model inference and the controlled production comparison are complete.
The guarded fallback has one confirmed useful recovery among its 12 additions.
Six additions produce wrong file-change previews; the planner stops the other five.
The raw model also repairs some rule misses, but changes literal names, operations,
filters and scheduling in other requests. These findings do not support integrating
this checkpoint and guard combination into the app.

## What ran

Luna ran the preserved T5 Efficient Tiny v3 checkpoint on **all 300 original
commands**, then separate Luna reviewers assessed file operations, transforms,
and scheduling/scope. These are offline experiments: the model is not integrated
into dev, and no planned action, confirmation, Finder navigation or scheduled job
was executed by this comparison.

The input is the same frozen corpus used for the earlier two-app test:
`../luna-dual-build-2026-11-16/commands.jsonl`, SHA-256
`851900248b9706c151eb94f9547a65d3f9541c43e8cc3ecbf0d0684ee1cd0fc8`.
Every output retains its ID and original command; all five primary JSONLs have
300 rows in the original order.

Production rules came from `v0.1.14`, commit
`44c74fa65fd0b4a8f9a9ad2c79d435e7c30e3ffb`, previously matched to the installed
production executable. Dev uses a captured dirty working tree atop
`13793d458a77c80535e9ff2e24c8c8376fb3382f`. All used source modules and binaries
were hashed. Both versions used the same fixture; schedule extraction used
November 16, 2026 at noon IST. Date parsing also uses the actual local date,
which was November 16 during this run. Planner-generated resize backup names
still depend on wall-clock time.

The model emits command text, not a structured Intent. Its rewrite therefore
passes through the captured **dev** parser, scheduler and planner. The experiment
compares these four configurations:

1. Production rules on original text.
2. Dev rules on original text, as context for isolating model effects.
3. Tiny v3 on every original command, with its raw rewrite parsed by dev.
4. Dev rules plus guarded tiny v3 fallback. Accepted rules win; only typed
   `GrammarMiss` errors are eligible. Typed hard refusals and scheduler/application
   refusals remain terminal. The frozen copy/unit guard, conservative semantic
   guard, parser acceptance and exact equality of recognized schedule data must pass. This does not prove unsupported schedule wording was preserved.

Raw inference on otherwise ineligible inputs is diagnostic only. It does not
authorize fallback. Each guarded record stores both the effective result and,
when eligible, the rejected or accepted candidate for inspection.

## Pipeline results

| Configuration | Parser/schedule accepted | Refused before planning | Change previews | Planner problem/choice | Read-only outcome recorded |
|---|---:|---:|---:|---:|---:|
| Production rules | 227 | 73 | 120 | 78 | 29 |
| Dev rules | 224 | 76 | 117 | 78 | 29 |
| Tiny v3 raw rewrites + dev | 243 | 57 | 123 | 87 | 33 |
| Dev + guarded tiny v3 | 236 | 64 | 124 | 83 | 29 |

These are **pipeline counts, not pass rates**. Accepted includes wrong intents
and commands that stop at planning. Read-only outcomes were recorded, not run.
The raw planner column includes four ambiguity results. Acceptance counts are
not an overall accuracy comparison.

Guard routing preserved all 224 dev-accepted outputs, 24 typed hard refusals and
three schedule refusals. Of 49 grammar misses, 12 model candidates passed all
configured checks. The other 37 remained refused. Rejection checks can overlap;
the per-row `decision` gives the first reason. One candidate had a different
extracted schedule and was already rejected by another check.

## What the model adds

The parent checked every effective guarded addition against its original request,
intent, schedule and actual fixture planner result. Full findings are in
[guarded-additions-review.jsonl](guarded-additions-review.jsonl).

| Result among 12 additions | Count | Cases |
|---|---:|---|
| Correct recovery with a preview | 1 | T035 |
| Wrong interpretation with a preview | 6 | T004, T045, T053, T058, T099, S029 |
| Wrong interpretation stopped by planner | 4 | T022, T074, T077, S077 |
| Unsupported nested-folder form stopped by planner | 1 | T036 |

The useful recovery is **T035**, “Make Archive and Backup folders on Desktop.”
Rules refuse that wording. The model rewrites it to a supported create-folders
command, preserving both names and the parent folder, and the fixture plans
exactly those two folders.

Examples of confirmed guarded failures:

- **T022:** “Remove the word draft from file names in Documents” becomes
  `delete files named "draft" in Documents`. This is a trash intent, not a name
  replacement. No fixture files match, so the planner stops. A missing match does
  not make the rewrite correct.
- **T053:** “Resize Pictures to 50 percent quality” becomes `resize Pictures to
  50 px`, and the fixture offers a 50-pixel-width resize of two images.
- **T004:** “Rename the file named "Archive" to Archive-old” becomes
  `rename "Archive" to Archive`. The suffix disappears; the preview contains a
  rename-to-the-same-name action.
- **T045:** “Sort Downloads by type and then by month” becomes a month-only sort.
  Production accepts a malformed source and stops at planning; dev rules refuse.
  Guarded fallback reintroduces acceptance and offers the partial operation.
- **S029:** “Sort Pictures by type every month” becomes `sort Pictures by month`.
  The resulting preview sorts immediately by modification month. Monthly
  recurrence is unsupported by the scheduler, so original and candidate extracted
  schedules are both null: schedule equality fails to catch the lost wording.
- **T058/T099:** Replacement of the original image and sharpening are silently
  dropped, while conversion or resize previews are still produced.

For **S077**, “Delete that file” becomes an unfiltered trash request with no prior
scope. The planner's explicit requirement for a cleanup filter blocks it. This
is a persistent planner restriction, unlike a stop caused merely by absent fixture
files. No execution or bypass of that restriction was tested.

The raw model has additional useful examples that the guarded pipeline does not
adopt: **F070** turns “between 10 and 50 MB” into the corresponding two size bounds;
**T024** correctly rewrites “Prefix every photo in Pictures with trip-.” F070 has
no fixture matches, and the treatment of exact size-boundary inclusivity is not
established here. These examples do not offset the confirmed wrong rewrites.

Raw-model regressions include **S010**, which changes `annual-report.pdf` to
`quarterly report.pdf` and drops the 8 pm schedule; **S057**, which changes a
2 GB upper bound to 1024 MB; and **S059**, which reverses spaces-to-dashes into
dashes-to-spaces. Guarded mode retains the original rule outputs for these
already accepted commands. Their preservation is not an inference improvement.

## Runtime, provenance and harness correction

The preserved local Python/PyTorch runtime used CPU FP32, two threads, seed zero,
local-only weights, greedy generation, batch size one, and 64-token input/output
limits. All 300 predictions took 9.402 seconds in the recorded loop, excluding
model loading. No input was truncated. A 40-case spread from that loop measured
33.83 ms median and 60.76 ms p95; this was not a separately warmed benchmark or
native app timing. Model and tokenizer files total 64,691,367 bytes and were
unchanged before and after inference.

The initial Luna harness reused a shared Cargo target binary after building
production last. The parent detected this, pinned a separate executable per
source version, and repeated source replay using the unchanged saved model
predictions. The corrected raw accepted count is 243, superseding the initial
242. Dev parser-refusal results now preserve schedule metadata. Final manifests
contain the actual fresh fixture inventories rather than copied historical ones.
The initial artifacts are retained in `superseded-initial-run/` and are not used
for conclusions.

[verify.py](verify.py) independently checked all corpus fields and hashes,
model/guard files, source snapshots, version-specific binaries, effective guarded
routing and fixture files. It also asserts that raw results have the dev typed
parser metadata, catching the initial wrong-binary error. See
[verified-summary.json](verified-summary.json).

After removing newly added diagnostic metadata and normalizing temporary root
paths, fresh original-rule replays match the earlier source audit on 295/300
cases per version. Four differences are wall-clock-derived resize backup names;
one is ordering among equally sized list rows. These are retained in the evidence.

## Review and interpretation limits

The 300 frozen expectations are heuristic, not human-validated gold labels.
The corpus was authored before this model run, but it is not certified independent
of all earlier model training/development data. Do not treat these results as a
held-out accuracy estimate. Canonical agreement is not semantic correctness.

All 300 cases have Luna annotations, but those annotations also require checking.
Some scheduling-review verdicts contradict their own quoted evidence: notably
S029 incorrectly calls the guarded month-sort result correct and claims its
recurrence is retained. The final analysis uses the effective intent and planner
record, with parent corrections recorded separately. Aggregate reviewer verdict
counts are not used as accuracy scores. [review-validation.json](review-validation.json)
records 72 annotation-level conflicts between verdict categories and recorded
parser/planner stages; these are excluded from scoring.
[review-corrections.jsonl](review-corrections.jsonl) preserves the parent corrections,
including the complete checked fallback-addition subset. The independent
[Luna methodology review](review-method.md) found no remaining blocking provenance,
binary-selection, count or routing defects and independently identified the S029
schedule-data limitation.

Fixtures contain real three-page PDFs, but media/archive placeholder bytes do
not validate conversion or extraction. No-match and planner guards do not prove
safety or correctness on arbitrary files. This comparison did not test confirmation,
execution, undo or scheduled task execution. The earlier, separate rules-versus-rules
audit did complete direct app UI submissions after Accessibility was enabled;
there was no macOS-access blocker to this offline model run.

## Evidence and reproduction

- [All 300 actual model predictions](model-predictions.jsonl)
- [Production rules](production-results.jsonl), [dev rules](dev-rules-results.jsonl)
- [Raw model results](model-raw-results.jsonl), [guarded results](model-guarded-results.jsonl)
- [Model and source manifest](manifest.json), [replay provenance](replay-manifest.json)
- [Parent verification](verified-summary.json), [all 12 guarded additions](guarded-additions-review.jsonl)
- [Luna methodology review](review-method.md), [review validation](review-validation.json), [parent corrections](review-corrections.jsonl)
- [Comparison chart](comparison.html)
- Luna annotations: [files](review-F.jsonl), [transforms](review-T.jsonl), [scheduling/scope](review-S.jsonl)

From the repository root, source-only verification does not repeat inference:

```sh
rtk proxy python3 docs/testing/tiny-model-vs-production-2026-11-16/verify.py
```

The local inference runner is [raw_diagnostic.py](raw_diagnostic.py). Source replay
and postprocessing run in order: `replay.py`, `compare.py`, `finalize.py`, `verify.py`.
These scripts regenerate result files; preserve this evidence before rerunning.
Use the recorded external experiment `runtime/bin/python` for model inference.
No application source, checkpoint, frozen corpus, or guard implementation was
changed for this run. No training or model/guard tuning followed the results.

## Modern Web Guidance

I used the `accessibility` guide for the comparison chart: “Provide semantic data
tables as alternatives.” The chart keeps its data in a semantic table, identifies
the series in text, and places numeric values outside colored bars.
