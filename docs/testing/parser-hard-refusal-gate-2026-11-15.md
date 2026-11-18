# Hard refusal gate — 2026-11-15

Known deliberate refusals are now terminal. A model may run only after a Rust
grammar miss; it may not reinterpret a hard refusal as a supported operation.
This changes routing and adds explicit refusal rules. Model weights are unchanged,
and model inference is still not integrated into the app.

## Rust contract

`parse::parse_classified` returns an accepted intent, `ParseFailure::GrammarMiss`,
or `ParseFailure::HardRefusal(reason)`. Only the grammar-miss variant authorizes
fallback. `parse::parse` retains its existing `Result<Intent, String>` contract
for current app callers and uses the same policy internally. Its string error
must not be interpreted as authorization to call a model.

The policy includes existing restrictions (permanent deletion, confirmation/Trash
bypasses, wildcards, unsupported content edits and related refusals) plus explicit
unsupported upload/encryption/permission/transport operations, exclusions and
second actions. Failed date/value/filter validation is terminal after ordinary
parsing and verb correction fail. Leading folder scopes preserve refusal status.

Rules inspect unquoted syntax and distinguish action words from filename suffixes.
`copy "upload instructions.txt" to "Drive"`, `copy upload.pdf to Documents`, and
ordinary local destinations such as Upload or Dropbox remain supported. Explicit
network transport such as `via AirDrop` is refused. Quoted instruction-like names
remain literal. These rules recognize known syntax; they are not a complete
natural-language understanding or semantic-safety proof.

## Fallback enforcement

- The Rust evaluation bridge emits `parse_status`, `hard_refusal_reason` and
  `fallback_eligible`. It clears and recomputes caller-supplied routing/scheduling
  flags each time a sentence or rewrite is scored.
- Both evaluation paths select only authoritative grammar misses. Hard refusals
  and scheduler/application restrictions stay visible as terminal decisions.
- Prediction files cannot contain hard-refused IDs. Unknown historical routing
  metadata and inconsistent eligibility booleans are rejected; snapshots must be
  refreshed after the policy change.
- The inference CLI re-scores every original input with the current Rust parser
  **before importing Torch or loading a checkpoint**. A stale or forged fallback
  list containing a hard refusal causes a closed failure, rather than generation.
  Mixed accepted/blocked lists are rejected; callers must rebuild the eligible list.
- Model rewrites go through the same Rust policy and the existing copy/drop/unit
  guards. New hard-refused operations in model output are rejected. This change
  does not claim to solve every remaining name/action/filter preservation error.

## Validation evidence

The durable experiment root is
`/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15`.

`refusal-gate-001` preserves current outcomes for all original 16,731 evaluation
rows and their unchanged labels. It records 5,763 accepted rule results, 8,952
grammar misses and 2,016 hard refusals. All previously correct intent/refusal
outcomes remain correct. The only three changed parse outcomes are unsupported
AirDrop/Bluetooth/WhatsApp transport requests previously accepted as local folder
names; all three now refuse. These are development diagnostics, not a new holdout
or a claim that v3 meets integration criteria.

`refusal-gate-routing-001` records actual Rust scores for five hard-refusal cases,
a quoted literal and a grammar miss. Forged `parse_status`/eligibility input flags
were overwritten. An actual CLI invocation using an upload request and a deliberately
nonexistent checkpoint failed at the refusal gate before model imports/loading.
A separate real v3 invocation on `Could notes.pdf be relocated into Documents`
passed the policy and generated `move "notes.pdf" to Documents`. No commands
were executed against user files.

Targeted and ordinary Rust tests cover terminal classification, grammar misses,
scope propagation, invalid values, quote/literal handling and the unchanged UI
error contract. Python tests cover terminal combining, forged/stale metadata,
prediction coverage, generated hard refusals and model-loading prevention.
Final checks: **226 Rust tests passed, 18 Python tests passed, and all 860,049
grid cases reported zero wrong outcomes** across all ten printed categories.
`git diff --check` is clean. The normalized-empty guard was added while the grid
ran; it does not affect the generated commands, which retain nonempty normalized
text. Current unit tests and a complete 16,731-row refresh passed afterward.
Every parse outcome is unchanged by that last guard; the punctuation-only corpus
request `?` additionally becomes a terminal malformed-input refusal instead of a
grammar miss. The final reference snapshot is `refusal-gate-003`, with 8,951
authoritative grammar misses and 2,017 hard refusals.
[Verification JSON](parser-hard-refusal-gate-2026-11-15.json).

## Reproduction

```sh
rtk proxy python3 scripts/parser_model_eval.py refresh --experiment <root> --run <new-run>
rtk proxy <root>/runtime/bin/python scripts/parser_model_infer.py \
  --model <root>/legacy/out-v3/model_t5-efficient-tiny-v3 \
  --input <root>/<new-run>/fallback.jsonl --output <root>/<new-inference-dir>
rtk proxy python3 scripts/parser_model_eval.py compare --experiment <root> \
  --run <new-run> --predictions <root>/<new-inference-dir>/predictions.jsonl
```

New slot-training experiments must also specify `--run <new-run>` so preparation
uses current typed refusal outcomes. Historical reports and their archived code,
inputs and model predictions remain preserved; old fallback files do not grant
permission to bypass the current policy.

## Review

[Luna review — terminal refusal gate](t3-thread://v2/59c98117-176b-4499-bc4c-278e704b935e/thread:delegated-task:command%3Amcp%3Ab07eb128-51c8-42fe-b333-64a4e473e86a%3Adelegate-task%3Ad57cdf75-9db3-466f-9b22-bace22237748)

Luna approved the stated refusal-policy/evaluation-routing scope with no blocking
findings. It did not approve model integration or an accuracy claim. Its summary
mentioned a completed full grid prematurely: at review return, the grid process
was still running and only two category counts were available. Handoff waits for
independent inspection of every final printed category and the terminal result.

The grid has now completed and all ten counts have been independently verified.
The final normalized-empty safeguard was added after review and checked by the
current Rust suite, direct Rust scoring and a full refresh.
