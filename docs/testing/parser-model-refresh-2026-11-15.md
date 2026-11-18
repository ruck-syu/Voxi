# Parser/model evaluation refresh — 2026-11-15

Stage 1 of [the next steps](parser-model-next-steps.md) is complete. Every
evaluation sentence now has its frozen expected intent, current Rust rule
outcome, and fallback eligibility. No parser behavior, model weights, guards,
or expected labels changed during this refresh. New v3 inference is next.

## Durable experiment

Location (outside Git):

```text
/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15/
  manifest.json                 # hashes of all 283 preserved feasibility files
  requirements-legacy.txt        # installed packages captured from the old venv
  legacy/                       # 1,381,651,810 bytes: scripts, data, models, logs
  provenance/                   # original HEAD, full working diff, worktree copy
  refresh-002/
    manifest.json               # parser/toolchain provenance and output hashes
    provenance/                 # worktree snapshot for the refresh
    evaluation.jsonl            # 16,731 labelled rows with current rule results
    fallback.jsonl              # 10,963 currently refused rows, with stable IDs
    label-conflicts.jsonl       # empty: canonical targets match frozen intents
    rules-score/                # Rust input/output JSONL and complete cargo log
    target-score/               # diagnostic canonical-target scoring and log
    validation-all-refuse/      # synthetic pipeline check, not model results
```

The temporary original was left intact. Virtual environments and Python bytecode
were excluded from the copy; dependencies were recorded. The refresh itself uses
Python's standard library and Cargo, with no inference environment dependency.
`refresh-001` is an earlier equivalent run; `refresh-002` is the reference run
with toolchain provenance and the final evaluation tool implementation.

Parser HEAD: `13793d458a77c80535e9ff2e24c8c8376fb3382f`, plus the
preserved uncommitted working tree. HEAD alone does not identify this parser.
Refresh captured at `2026-11-15T14:21:31.154709+00:00` using Rust/Cargo 1.96.0.
The parser clock remains **2026-10-28**, matching the original evaluation.

The snapshot contains the checkpoint, tokenizer, generation configuration,
training/inference scripts, and exact guards. Historical generation used input
truncation at 64 tokens, greedy generation (`num_beams=1`, no sampling), at most
64 new tokens, and decoding with cleanup disabled. Captured installed package
versions describe the environment found today; they do not establish the original
Kaggle training environment. Historical 72 ms CPU timing and estimated int8 size
remain Kaggle measurements, not local runtime validation or an exported package.

## Frozen labels and split boundaries

Generated paraphrase labels come from preserved `seeds.json` intent values;
generated refusal labels remain `!`. Saved corpus/bank intent labels are copied
unchanged. Canonical targets are scored separately as diagnostics and never used
to overwrite expected intents. There are zero canonical-target/intent conflicts;
this does **not** resolve the 17 natural-language date-endpoint label questions
or other ambiguity reviews in the next-steps document.

The generated set is development data. The 399 `corpus-test` rows retain the
original frozen split; the other 2,263 corpus rows overlap training/regression
work. Corpus rows are included once, not counted twice. The banks are reported
separately and labelled previously inspected evaluation data. None is described
as a new untouched holdout. No bank failure texts were inspected to tune grammar.

The machine-readable baseline is
[parser-model-refresh-2026-11-15.json](parser-model-refresh-2026-11-15.json).
Its generated development counts reproduce the existing findings: 2,378 correct
supported intents, 8,078 refused supported requests, 334 accepted misreads,
1,862 correct refusals, and 160 accepted requests labelled REFUSE.

The old v3 prediction file misses **13 newly refused bank inputs** (4 in bank-2,
9 in bank-3). Another **362 historical predictions** are no longer eligible for
fallback, including 350 corpus-test rows and 12 bank rows. The refreshed fallback
also includes generated development inputs and all corpus refusals for regression
coverage, which the historical inference list did not cover.

## Reproduce and continue

From the repository root, reproduce the rule refresh in a new, unused run folder:

```sh
rtk proxy python3 scripts/parser_model_eval.py refresh \
  --experiment /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15 \
  --run refresh-003
```

The tool verifies every preserved artifact hash before running the actual Rust
`feas_score` bridge. It preserves the parser working tree, fixed date, toolchain,
full Cargo output, current outcomes, fallback membership, and artifact hashes.
Existing run folders cannot be overwritten.

For stage 2, run the frozen v3 checkpoint on every row of `refresh-002/fallback.jsonl`
and save one prediction per ID with this shape:

```json
{"id":"bank-2:0","text":"<exact original text>","rewrite":"<model output>"}
```

Then score those outputs using:

```sh
rtk proxy python3 scripts/parser_model_eval.py compare \
  --experiment /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15 \
  --run refresh-002 \
  --predictions /absolute/path/to/v3-predictions.jsonl
```

The comparison reads refreshed rules, rejects missing/extra/duplicate predictions
or changed input text, and checks the Rust sources and Cargo inputs still match
the refresh. It parses rewrites in Rust, applies the preserved `guard.safe`, and
retains accepted rule results even when those results are wrong. Model refusals,
parser refusals, guard rejections, raw rewrite outcomes, and final decisions stay
visible per sentence. Summary counts distinguish correct intents, correct
refusals, refused supported requests, accepted misreads, and accepted REFUSE labels.
Accepted errors are split into read-only and potentially file-changing intents;
these categories do not establish planner permission to execute.

The original `legacy/compare.py` is historical evidence. It must not be used to
claim results for the refreshed parser. New predictions must cover the full
current fallback set, not just IDs present in old outputs.

## Key SHA-256 hashes

All artifacts have byte counts and hashes in the experiment manifests. Key files:

```text
evaluation.jsonl
647d15ba76e7df0e0595fe20aaab78c2414e417a70776b3a0df0156316f1a063
fallback.jsonl
86718d04d73f9cdd384fac85cf4cfbdf5d13912c062411bbf5fd932b70c62f19
legacy/out-v3/model_t5-efficient-tiny-v3/model.safetensors
c90edf63e27c839d4e56b90a328e3e07776bc7ee7eb6d2adaf91eec3419a8d86
legacy/out-v3/model_t5-efficient-tiny-v3/tokenizer.json
4629b0bb9b2c52f0a1364a8db3f609697e2ca4179e3d5ad3470a3267b4aeedd6
legacy/guard.py
87e514f9154768076fceb805b3cc3f3cbc7ec34a6bce6eb98a81f102dff2834c
legacy/gen/clean.jsonl
820d9d79792d495bf5430b4f355a847ccc0fab5b321029b86cd0212e5dabaa35
legacy/seeds.json
370cc44504b6568d5fdef1e1115c522c15acbeae6a7c91f14326612ba8b5563b
```

## Validation

- `rtk cargo test --lib` from `app/src-tauri`: 220 passed, 17 ignored.
- `rtk proxy python3 -m unittest discover -s scripts -p test_parser_model_eval.py -v`:
  4 passed. These check fixed-label loading, incomplete/stale prediction rejection,
  accepted-rule precedence, and visible guard/model refusal outcomes.
- Full rule scoring: 16,731 rows. Separate canonical-target diagnostic: 10,790 rows.
- Synthetic all-REFUSE predictions exercise the full Rust/guard comparison path;
  every combined result equals its rules-only result. This is a pipeline check,
  not a v3 model evaluation.

No parser changes were made, so the previously completed 860,049-case grammar
grid was not repeated. No retraining, inference integration, local model latency
measurement, planner execution, or fresh held-out evaluation has been performed.
