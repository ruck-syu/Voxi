# Guarded v3 baseline — 2026-11-15

Step 2 uses the original checkpoint, frozen labels and guards, current Rust parser,
and fixed date 2026-10-28. No training or parser changes preceded this baseline.
Generated development results overlap training and are not held-out accuracy.
All other sets below were previously inspected; none is a new pristine holdout.

## Decisions and errors

Rules win whenever accepted. Each currently refused input was generated locally
with the preserved checkpoint and tokenizer. Inference and comparison manifests
verify input, predictions, checkpoint/tokenizer, evaluator and parser provenance.

| Set | Rules correct intents | Combined correct intents | Rules misreads | Combined misreads | Rules REFUSE violations | Combined REFUSE violations |
|---|---:|---:|---:|---:|---:|---:|
| generated-development | 2378 | 10418 | 334 | 363 | 160 | 165 |
| corpus | 1965 | 1965 | 0 | 0 | 0 | 9 |
| corpus-test | 350 | 350 | 0 | 0 | 0 | 2 |
| bank-2 | 130 | 329 | 45 | 78 | 0 | 0 |
| bank-3 | 173 | 453 | 79 | 108 | 0 | 0 |
| heldout-1 | 86 | 86 | 0 | 0 | 0 | 1 |
| heldout-2 | 64 | 67 | 0 | 0 | 4 | 7 |

The model improves coverage but introduces accepted errors despite the guards.
The generated set gains 8,040 correct intents while adding 29 accepted misreads
and 5 accepted REFUSE labels. Bank-2 gains 199 correct intents but adds 33
accepted misreads; bank-3 gains 280 but adds 29. These gains do not meet the
zero-error safety gate. Existing accepted rule errors remain untouched.

All per-sentence raw rewrites, parser outcomes, guard decisions and final intents
are in `refresh-004/comparison-v3/outcomes.jsonl`. `accepted-errors.jsonl` records
every accepted error, its rules/model origin and original label. Read-only and
potentially mutating counts are separate in the summary. Planner permission is
not inferred from parser acceptance. `model_refusal` means a rewrite refused by
the parser, including explicit REFUSE; guard rejections remain a separate decision.

## Local runtime and size

On this Apple ARM Mac, PyTorch CPU FP32 with two threads processed 10,963
inputs in 34.43 seconds in batches of 32. Forty single-sentence samples spread
across the inputs, after three warmups, measured median 31.65 ms and p95 54.37 ms
including tokenization/generation/decoding. No input was truncated. The actual
FP32 checkpoint, configs and tokenizer occupy 64,691,367 bytes.

The original Kaggle estimate of 15.6 MB int8 is a parameter-count estimate, not
a measured export size. Local qnnpack quantization creates a 48,784,541-byte
state file before tokenizer/configs. The CPU wheel initially raised NoQEngine;
the runner now explicitly selects its available qnnpack backend. Qnnpack uses
deprecated PyTorch dynamic quantization APIs in this pinned experiment only.
Its single-sentence median is 36.56 ms and p95 58.19 ms. The native app runtime
and distributable dependency size remain unvalidated. Both current artifacts
fail the fixed 25 MB model-plus-tokenizer gate.

## Reproduction

Durable root: `/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15`.
The pinned `runtime/` environment was recreated using `requirements-legacy.txt`.

```sh
rtk proxy python3 scripts/parser_model_eval.py refresh --experiment <root> --run <new-run>
rtk proxy <root>/runtime/bin/python scripts/parser_model_infer.py \
  --model <root>/legacy/out-v3/model_t5-efficient-tiny-v3 \
  --input <root>/<new-run>/fallback.jsonl --output <root>/<new-inference-dir>
rtk proxy python3 scripts/parser_model_eval.py compare \
  --experiment <root> --run <new-run> \
  --predictions <root>/<new-inference-dir>/predictions.jsonl
```

Add `--precision int8` for a separately named qnnpack run; score it separately.
Reference FP32 inputs used refresh-002, whose fallback content hash is identical
to refresh-004. The comparator verifies content, not the input filename.

Reference artifacts: `v3-local-fp32`, `v3-local-int8-qnnpack`,
`refresh-004/comparison-v3`, and `refresh-004/comparison-v3-int8`.
Compact results are in [the JSON summary](parser-model-v3-2026-11-15.json).

No integration is justified by this baseline. Continue with error review and a
controlled placeholder experiment, then evaluate against frozen independent labels.
