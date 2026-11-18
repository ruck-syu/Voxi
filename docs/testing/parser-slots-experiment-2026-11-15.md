# Exact-slot prototype and controlled experiment

Stage 4 is an experiment, not a claim that placeholders improve Voxi accuracy.
The independently authored fresh set stayed unopened during training and tuning.

## Substitution and guards

`scripts/parser_slots.py` extracts typed `NAME1`, `FOLDER1` and `NUM1` slots
from the input sentence alone. It keeps each original byte string, quote style,
case, Unicode name, path and quantity spelling. Whole numerical quantities carry
their units; a canonical equivalent such as 2560 MB may map to an existing
2.5 GB slot, which restores the exact original 2.5 GB. Equal values can reuse a
stable slot; multiplicities must match. Different original spellings are not
silently merged. Ambiguous or malformed extraction fails closed.

Restoration rejects missing, extra, duplicated or malformed markers, incompatible
source/destination/output roles, extra units, reversed page endpoints, and reversed
rename replacement values. Literal input that collides with reserved slot syntax
is refused. Quoted names protect apostrophes and whitespace. The original frozen
copy/drop/unit guard still scores restored text; additional conservative semantic
checks retain actions, explicit filter directions/quantities, unsupported-request
boundaries and symbolic relative dates. These checks are bounded heuristics,
not a proof of semantic equivalence.

`this week`, `last month`, `today` and similar text remain symbolic. The model
does not receive calendar dates substituted for them; Rust resolves them using
the fixed evaluation/runtime date. The prototype is Python experiment code and
has not been ported into the app.

Six focused test methods cover quoted names, Unicode, apostrophes, paths, repeated
values, decimals, unit conversions, page ranges, rename replacement direction,
reserved-marker collisions, invalid slots, action changes, dropped/inverted
filters, unsupported requests and invented calendar dates. All ten Python
evaluation/slot test methods pass.

## Matched experiment

Source: preserved `gen/clean.jsonl`, current refusal membership from refresh-005,
and unchanged original targets/intents. Only examples representable by the
input-only extractor were eligible. The selection exclusions and exact data hashes
are retained in `slots-experiment-001/data-manifest.json`; many examples are outside
this bounded extractor's coverage. In particular, incompatible target values/roles
and malformed quotes are excluded rather than repaired using expected intents.

There are **1,600 training examples** (1,180 supported, 420 REFUSE) and **400
validation examples** (282 supported, 118 REFUSE). Supported examples are grouped
by canonical target before the deterministic 80/20 assignment; a target group
cannot cross this experiment's train/validation boundary. REFUSE examples are
grouped by exact input text. These remain generated development data: the original
v3 checkpoint was already trained on this development distribution. This split
is not a new holdout from v3 pretraining/fine-tuning and cannot establish
generalization. A fresh independent evaluation follows separately.

Both matched fine-tunes start at the same preserved v3 checkpoint, use the same
examples, seed 19, batch size 32, AdamW learning rate 0.0003 and weight decay 0.01,
three epochs/150 optimizer steps, two CPU threads and 64-token input/target limits.
Raw control took 40.39 seconds; slots took 44.72 seconds. Validation uses greedy
generation, the actual Rust parser and frozen copy/drop/unit guards. Both matched
candidates also use the same conservative semantic guard. A separate v3 plus
semantic-guard ablation shows that guard's coverage cost.

| Configuration | Correct supported / 282 | Refused supported | Accepted misreads | Correct refusals / 118 |
|---|---:|---:|---:|---:|
| Original v3, frozen guards | 282 | 0 | 0 | 118 |
| Original v3 + semantic guard | 135 | 147 | 0 | 118 |
| Matched raw fine-tune + semantic guard | 134 | 147 | 1 | 118 |
| Matched slot fine-tune + semantic guard | 110 | 172 | 0 | 118 |

The slot candidate had 57 slot rejections and 116 semantic rejections; these
remain visible in prediction records. The matched raw control had 148 semantic
rejections. The raw control's one accepted misread is potentially file-changing.
No candidate accepted a REFUSE example on this selected development sample.

Luna's review found that marker checks alone could allow an extra un-slotted
filename or number. Restoration now also refuses detected literal values outside
the slots, and the additional-action heuristic covers all known action families.
Three new malformed-output cases and a chained-resize case are covered. Re-running
all validation configurations under `*-review-fix` preserved every score above;
the slot rejection attribution becomes 60 slot rejections and 113 semantic
rejections. Original outputs remain preserved. General semantic safety is still
not established by these bounded checks.

**Decision:** this bounded prototype does not justify a full placeholder retrain.
Slots preserve values in verified round trips, but the current extractor/model
reduces useful coverage relative to both controls. No full retrain is scheduled,
and the successful unit checks are not presented as an accuracy improvement.
The small trained slot checkpoint is retained for the planned fresh comparison.

## Provenance and reproduction

Durable root:
`/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15`.
All models, train/validation rows, losses, per-sentence outputs, Rust scores,
inference scripts/helper hashes and inventories are in `slots-experiment-001/`.
`rust-sources.json` records the source inventory at scoring; refresh-006 captures
the corresponding parser working tree and evaluator. The production parser has
not changed since the step-3 fixes. The page-role check was tightened while the
fine-tunes ran; every one of the 2,000 stored training/validation encodings and
targets was rechecked against the final implementation and matched exactly.
Original archived sources and final inference helper copies remain separate.

```sh
rtk proxy <root>/runtime/bin/python scripts/parser_slot_experiment.py \
  --experiment <root> --output-name <unused-experiment-name> \
  --train-limit 1600 --validation-limit 400 --epochs 3
rtk proxy python3 -m unittest discover -s scripts -p 'test_parser*.py' -v
```

The script never reads the fresh set. Compact scores and input hashes are in
[the JSON report](parser-slots-experiment-2026-11-15.json). No inference was
integrated into Voxi, and no app execution was driven by these experiments.
