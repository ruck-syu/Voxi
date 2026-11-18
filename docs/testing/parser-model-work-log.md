# Parser/model completion work log

Started 2026-11-15. Each stage receives a separate Luna review. Existing unrelated
working-tree edits are preserved. Integration is conditional on measured gates.

## Gates fixed before fresh-set evaluation

- Zero accepted requests labelled REFUSE and zero accepted file-changing misreads.
- At least 10% of supported requests refused by the rules become correct intents,
  without regressing accepted rule results.
- Local CPU single-sentence p95 inference latency at most 200 ms after warmup.
- Exported model plus tokenizer at most 25,000,000 bytes; runtime dependency size
  must be reported separately before a shipped package can be approved.
- Slots must restore exact strings and reject malformed outputs. Guards and planner
  restrictions remain required. No direct execution from model text is permitted.

A failed candidate is rejected; gates and fresh labels will not be relaxed to
obtain integration. Previously inspected banks are development evidence.

## Step 1 — refreshed inputs

Complete; see the refresh report. Luna's review accepted the stage and found two
provenance issues: enforce the evaluator script hash, and record model/input/output
hashes when inference runs. The evaluator now enforces its saved script hash;
new runs must be refreshed after evaluator changes. Inference records checkpoint,
tokenizer, input, output, environment, settings, and script hashes.

Review: [Luna review — evaluation refresh (step 1)](t3-thread://v2/59c98117-176b-4499-bc4c-278e704b935e/thread:delegated-task:command%3Amcp%3Ab07eb128-51c8-42fe-b333-64a4e473e86a%3Adelegate-task%3A06da7996-2485-4486-a8b6-d550165bd692)

## Step 2 — guarded v3 baseline

Complete; [report](parser-model-v3-2026-11-15.md) and raw manifests/results preserve
both FP32 and qnnpack runs. Safety and artifact-size gates fail. Luna accepted
the baseline and flagged runner-source hash enforcement. The comparator now
verifies the archived `inference-script.py` against the recorded hash. The
actual FP32 and int8 runner copies were recovered from matching provenance/current
sources and verified before archival. New runs archive their source automatically.
The quantized state file remains explicitly distinguished from a full export.

Review: [Luna review — guarded v3 baseline (step 2)](t3-thread://v2/59c98117-176b-4499-bc4c-278e704b935e/thread:delegated-task:command%3Amcp%3Ab07eb128-51c8-42fe-b333-64a4e473e86a%3Adelegate-task%3A9ab60ae7-2cc1-404f-a8cd-50a09ef4bba8)

## Step 3 — remaining-case review

[Review decisions and fixtures](parser-review-2026-11-15/README.md) preserve all
494 cases, 17 separate label proposals and 67 actual planner outcomes. Three
targeted parser fixes have zero previously-correct regressions or new accepted
misreads over every one of the original 16,731 rows. The full grammar grid completed: all ten printed categories report zero wrong
outcomes over 860,049 cases, in seven report-only tests (429.19 seconds). Raw
output is `step3-grid.log`; checked category counts are in the review `grid.json`.

Review: [Luna review — parser and planner error review (step 3)](t3-thread://v2/59c98117-176b-4499-bc4c-278e704b935e/thread:delegated-task:command%3Amcp%3Ab07eb128-51c8-42fe-b333-64a4e473e86a%3Adelegate-task%3Ab4d2005a-5c7d-4414-930a-16246db1488f)

Luna found no concrete scoped bugs and accepted the stated stage limits.

## Fresh test freeze

An independent Luna author consulted only the public README and intent schema,
not parser implementation, previous failures, banks, training data or model outputs.
A first 100-row authoring attempt did not produce files; a new author completed
64 rows in eight categories, including scheduling. The set is agent-labelled,
not human-labelled, and is limited evidence. It is kept out of training and tuning.

Frozen JSONL SHA-256:
`5a8ba26a42e594519d9093c6a58969d66b1f5c3fbebdbd6e469a45ccfedd3b10`.

Authorship: [Luna — freeze independently labelled 64-case test set](t3-thread://v2/59c98117-176b-4499-bc4c-278e704b935e/thread:delegated-task:command%3Amcp%3Ab07eb128-51c8-42fe-b333-64a4e473e86a%3Adelegate-task%3Ac224860d-7358-4cb0-8123-71e296325cc0)

## Remaining stages

4. Test typed slots, then train and validate a small controlled development experiment.
5. Freeze independently authored labels, compare candidates, audit every accepted
   file-changing error/refusal violation, and integrate only a qualifying candidate.

## Step 4 — placeholder prototype

[The controlled experiment](parser-slots-experiment-2026-11-15.md) completed using
1,600 development training rows and 400 grouped validation rows. Exact substitution
and refusal tests pass; the current slot candidate has lower useful coverage than
its matched raw control. This prototype does not justify a full retrain. The
candidate remains available for the fixed fresh comparison, without tuning on it.

Review: [Luna review — exact slots and controlled experiment (step 4)](t3-thread://v2/59c98117-176b-4499-bc4c-278e704b935e/thread:delegated-task:command%3Amcp%3Ab07eb128-51c8-42fe-b333-64a4e473e86a%3Adelegate-task%3Ae91b19e1-b9ee-49f8-8919-7c347ec5d893)

Luna accepted the bounded experiment and flagged un-slotted literal additions.
The fix rejects those detected values and broadens known extra-action checks;
all development validation configurations were rerun with unchanged score counts.
Slot rejection attribution changes from 57/116 to 60/113 slot/semantic refusals.
This fix and revalidation preceded viewing any fresh outcomes. General semantic
proof is not claimed.

## Step 5 — fresh validation in progress

The first frozen 64-case bank is preserved unchanged, including its raw scores.
It exposed label-quality problems: explicit nulls versus omitted options, invalid
camelCase rename fields, omitted generic file types, decimal-versus-binary unit
assumptions, and no supported requests refused by the rules. Its raw error counts
are not reliable semantic accuracy and cannot measure fallback recovery. No model,
production parser, behavioral guard, gate, or label was tuned to those failures.

A second independent Luna author saw only README.md and the Intent schema, plus
explicit serialization and binary-unit conventions, and froze a new bank with
natural paraphrases as well as the required boundary categories. It is not a
post-hoc correction of the first bank. All comparisons retain their frozen labels;
new labels are validated through Rust Intent serialization before model scoring.
The agent-authored banks are limited evidence and are not human-labelled holdouts.

Final-bank frozen SHA-256:
`5f6cb98125b068c24187799c15f5007ea5e9e3f682dd431df70429585ddcfa1a`.

Authorship: [Luna — freeze a schema-checked final test bank](t3-thread://v2/59c98117-176b-4499-bc4c-278e704b935e/thread:delegated-task:command%3Amcp%3Ab07eb128-51c8-42fe-b333-64a4e473e86a%3Adelegate-task%3Ad66f6372-e874-4812-8c61-b5d4ee00e0c8)

## Step 5 — validation decision

Reference final-bank run: `fresh-final-003`. All models, current rules, guards and
frozen labels remain unchanged after outputs were viewed. Test-only schema and
scheduler/application-boundary validation were added; the boundary correction
changes only final-064. Raw earlier results and every label remain preserved.

[Final validation report](parser-model-final-validation-2026-11-15.md) records
43 accepted-error records across four candidates (15 unique cases), separating
confirmed name/intent corruption from representation differences and label
assumptions. Every file-changing misread and REFUSE violation was reviewed.
All candidates fail the fixed integration gate. The conditional export/native
Rust inference wiring was not performed; this is a no-go validation outcome,
not completion of the model-integration milestone.

Final checks: 223 Rust unit tests pass (18 ignored), 14 Python evaluation/slot
tests pass, all 860,049 grid cases report zero wrong outcomes, and diff whitespace
checks pass. No model-generated action was executed.

Review: [Luna review — final validation and integration gate (step 5)](t3-thread://v2/59c98117-176b-4499-bc4c-278e704b935e/thread:delegated-task:command%3Amcp%3Ab07eb128-51c8-42fe-b333-64a4e473e86a%3Adelegate-task%3Aea861c08-f20b-42f2-a7ee-3a5c4264e72d)

Luna accepted stage-5 validation with the no-go outcome, found no further
required evaluation work, and explicitly did not approve the integration
milestone. The only finding was a report typo, now corrected. All five stages
have their requested Luna review and findings have been addressed. The
conditional integration remains unchecked because no candidate qualifies.
