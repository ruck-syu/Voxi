# Kaggle structured-plan training result

The new model was trained on Kaggle and downloaded with verified checkpoint
hashes. It generates multi-step plans with symbolic schedules, including the
user's exact sentence, but still makes file-selection and schedule errors on
new requests. No additional application natural-language rules were added in
this training work. After reviewing these results, the user explicitly requested
activation. It is now enabled in the opt-in development build; see
[INTEGRATION.md](INTEGRATION.md). The failed training gate remains recorded.

## Latest checkpoint

- Kaggle GPU run: https://www.kaggle.com/code/dedruck/voxi-plan-v4c
- Private data: https://www.kaggle.com/datasets/dedruck/voxi-plan-data-v4c
- Device: Tesla T4. Starting weights: the preserved trained v3 T5 Efficient Tiny.
- Training: 16,001 synthetic examples; 800 validation examples; 8 epochs;
  2,008 optimizer steps; batch size 64. The selected checkpoint is epoch 6,
  chosen by validation loss. The optimizer run took about 414 seconds.
- Weights and tokenizer:
  `/Users/vishalshekhar/.local/share/voxi/experiments/plan-model-v4c-2026-11-17/output/model-voxi-plan-v4/`.
- Full run/data/code/weight hashes: `candidate-003-run-manifest.json`.
  The downloaded weight hash differs from the starting checkpoint.

All three training candidates and their results remain preserved. The first
candidate's new brace tokens led to refusal-only generation; the second used
existing-vocabulary arrays; the latest adds literal binding and broader training
coverage. Detailed hypotheses, source hashes and unsuccessful results are in
`PLAN.md` and the candidate manifests. None of the three candidates was deployed.

## Actual model output for the known user sentence

Input:

> create a folder in downloads called PDFS and move pdfs into it everyday

The downloaded latest model generated:

```json
["plan",[["mkdir","PDFS","Downloads"],["move","pdfs","Downloads",["ref",0]]],["repeat",1,"days",null]]
```

This means create `Downloads/PDFS`, move PDFs from Downloads into the folder
created by step 0, and repeat once a day. This sentence was explicitly included
in training, so this demonstration is **not held-out evidence**. The record is
`known-user-example.json`. The case uses direct inference from the new weights,
not the app's rule-based chain parser.

## Fresh challenge outcome

`challenge-003.jsonl` was frozen before this candidate trained. It contains
30 supported requests and 20 requests that should be refused. On the 30 supported
requests, 20 plans matched exactly, seven were refused and three were wrong
accepted plans. All 20 unsupported requests were blocked: 19 explicit refusals
and one malformed output. The fixed semantic acceptance gate fails.

The three wrong plans were individually inspected:

- **S006:** every Wednesday at the default 9 am became an unspecified weekly
  repeat. The weekday/default time was lost.
- **S016:** a three-step request moved PDFs from Pictures instead of Downloads.
  The later copy step's source leaked into the earlier move step.
- **S027:** every Friday at 3 pm became a one-time task tomorrow at 3 pm.

The simple new-name PDF example S001 was also refused; memorizing the known
user sentence did not establish robust paraphrase/type-casing generalization.
These results do not justify routing new requests to this candidate in the app.
See `challenge-003-score.json` for every outcome that failed exact comparison.

The challenge was self-authored by the same agent before training, not labelled
by independent humans. Earlier challenge sets were never trained on, but after
inspection they became development evidence and were not reused as untouched
acceptance tests. These finite, synthetic counts are not a production accuracy
or safety estimate.

## Host codec and CPU checks

The model predicts structured operations, source/destination roles, references
and schedule fields. A generic quote scanner keeps quoted literal values outside
generation and binds them to collision-free aliases. Restoration rejects missing,
unknown or embedded aliases; it does not reconstruct or repair actions or time
semantics. Tests explicitly prove that wrong actions and schedules remain wrong.

During training, a tokenizer-only check of known training targets found that
SentencePiece inserts whitespace around virtual `~N~` markers. The host correction
was frozen in `host-codec-correction.json` before any candidate-003 predictions
were viewed. It normalizes only a complete virtual alias and restores the original
literal bytes. No model weight or challenge label was changed. Both the original
GPU postprocessing results and the raw predictions are preserved; the primary
scores replay the fixed host codec on those raw predictions.

Local CPU inference of all 50 challenge sentences agreed with the GPU after
applying the same host codec. Warm single-thread PyTorch CPU p95 was about
155 ms across this mixed supported/refusal set. This is not native Q8 latency,
and quick refusals account for much of the set. The FP32 checkpoint package is
64,691,296 bytes. Native quantization, the 25 MB package goal and native runtime
integration were not claimed or pursued for a candidate that fails semantics.

Ten local schema, alias-integrity and scoring tests pass. All data splits passed
token-limit and source-literal round-trip checks; 500 known training targets also
passed tokenizer-to-schema-to-literal round trips after the codec correction.

## Decision

Training and evaluation are complete. Preserve the trained artifact, generator,
raw predictions, error audit and reproducible commands. The training-time decision
was to keep the existing app model unchanged; the subsequent user-requested dev
activation does not change these evaluation results. Further work needs richer, independently labelled compositional
language data and a new frozen test set; adding more application phrasing rules
would not fix this model's errors.

Reproduce the latest semantic audit without training:

```sh
rtk proxy python3 scripts/plan_model/evaluate.py --restore-literals --input /Users/vishalshekhar/.local/share/voxi/experiments/plan-model-v4c-2026-11-17/output/challenge-predictions.jsonl --output /tmp/voxi-v4c-challenge-score.json
rtk proxy python3 -m unittest discover -s scripts/plan_model -p 'test_*.py'
```
