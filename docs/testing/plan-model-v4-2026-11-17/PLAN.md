# Structured command model v4

Requested: train on Kaggle to interpret multi-step requests and schedules,
instead of adding more natural-language rules. This run changes no app grammar
or model routing. Existing v3 weights remain intact.

## Target

The model emits `voxi-plan-v1` JSON, or `REFUSE`. Example:

```json
{"steps":[["mkdir","PDFS","Downloads"],["move","pdfs","Downloads",{"ref":0}]],"schedule":["repeat",1,"days",null]}
```

Steps support folder creation, typed-set move/copy/trash, and literal file
renaming. A destination reference names an earlier mkdir step, removing the
need for the model to invent a joined path. Schedules remain symbolic: repeat
interval and optional clock, weekday, relative delay, or next/tomorrow clock.
Rust remains responsible for allowed paths, planning, dates, execution and
confirmation. Structural validation does not prove correspondence with the input.

This schema deliberately does not represent conditional logic, per-step times,
filters beyond file type, unsupported remote operations, permanent deletion,
or bypassing confirmation. Those examples are labelled REFUSE.

## Data and provenance

Private dataset: https://www.kaggle.com/datasets/dedruck/voxi-plan-data-v4

Private GPU kernel: https://www.kaggle.com/code/dedruck/voxi-plan-v4

Artifacts stay outside Git under
`~/.local/share/voxi/experiments/plan-model-v4-2026-11-17/`.
`dataset/manifest.json` and `package-manifest.json` record hashes, source base
weights and split counts. The starting checkpoint is the preserved trained v3
T5 Efficient Tiny model. This is continued training, not replacement by another
model. Explicit JSON brace tokens are added and their embeddings trained.

There are 12,001 training rows, 600 validation rows and 50 frozen challenge
rows. The known user sentence is explicitly a training example, not held-out
evidence. Data is synthetic and contains no actual user filesystem contents.
Training and validation literal-name pools differ. Folder-creation wording in
the validation generator is also held out from the training generator.

The challenge set was authored and frozen by the same agent before training.
It is separate evaluation evidence, not an independently authored or
human-labelled holdout. Existing evaluation banks are not used for training.
Challenge labels are read only after saving the selected checkpoint. Selection
uses validation teacher-forced loss, not challenge results. A challenge that has
been inspected is never reused as untouched evidence for a tuned candidate.

## Training

Kaggle GPU required; fail rather than silently train on CPU. Eight epochs,
batch size 64, AdamW at 5e-4 with warmup and linear decay, seed 417. Select the
lowest validation-loss checkpoint. Disable training decoder cache; generation
uses greedy decoding with cache. No input or target truncation is allowed;
input maximum 192 tokens, generation maximum 256 tokens. Local tokenizer
preflight found maximum training lengths 54 input / 105 output tokens.

Record actual Kaggle accelerator, package versions, checkpoint/data/code hashes,
loss history, all predictions and errors. Export only after inspecting results.

## Predeclared acceptance

The structured candidate must produce zero schema-valid wrong plans or accepted
refusal requests on the frozen challenge, and exactly recover at least 90% of
supported challenge requests. Report validation separately. This is a finite
test gate, not proof of general safety. Literal corruption, dropped actions,
wrong references/direction, invented schedules and changed intervals all count
as errors. Invalid JSON is a refusal, never a partially accepted plan.

A candidate that fails remains an experiment and is not enabled in the app.
Native export, quantization parity, package size, local latency, stale-request
handling, planner and confirmation checks are additional integration gates.
The existing 25 MB package and 200 ms warm CPU p95 goals remain reported;
they are not inferred from GPU timing or model parameter count.

## Reproduce

```sh
rtk proxy python3 -m unittest discover -s scripts/plan_model -p 'test_*.py'
rtk proxy python3 scripts/plan_model/prepare.py --base <preserved-v3-model> --challenge docs/testing/plan-model-v4-2026-11-17/challenge.jsonl --output <new-artifact-directory>
rtk proxy kaggle datasets create -p <artifact-directory>/dataset --keep-tabular
rtk proxy kaggle datasets status dedruck/voxi-plan-data-v4
```

Wait until the private dataset is ready before pushing the GPU kernel:

```sh
rtk proxy kaggle kernels push -p <artifact-directory>/kernel --accelerator NvidiaTeslaT4 --timeout 3600
```

## Candidate 001 outcome and candidate 002 hypothesis

The first completed training run was kernel version 3 of `voxi-plan-v4` on a
Tesla T4, after two launches that exited before training because of dataset
availability/layout. Kaggle automatically expands uploaded ZIP resources; the
runner now discovers the dataset by its manifest and verifies the expanded
checkpoint against all pinned hashes.

Candidate 001 completed all 1,504 optimizer steps. It refused all 30 supported
challenge rows and all 507 supported validation rows (two validation outputs
were malformed). The 20 challenge refusal labels matched. Local CPU inference
matched the GPU decisions. It fails the useful-coverage gate and is not enabled.
See `challenge-score.json`, `candidate-001-run-manifest.json`, and the loss
history. Lower teacher-forced loss did not establish useful generation.

A diagnostic on the known training sentence found expected first token `{`,
but top first-token logit was the existing `RE` token beginning REFUSE. Both
cached and uncached generation returned REFUSE. The new token embedding norm
was about 91 versus about 208 for the existing vocabulary average. This points
to an output-format/initialization bottleneck; it is a hypothesis, not a proven
complete causal explanation.

Candidate 002 (`v4b`) changes only the wire encoding and uses the original v3
weights again. Its training JSONL is byte-identical to candidate 001. No tokens
are added and embeddings are not resized. `voxi-plan-v2` encodes the same
canonical target as:

```json
["plan",[["mkdir","PDFS","Downloads"],["move","pdfs","Downloads",["ref",0]]],["repeat",1,"days",null]]
```

The codec validates and restores the canonical object before scoring. Tokenizer
preflight confirms round-trip preservation of the known target, with maximum
training lengths 54 input / 92 output tokens. There are no new language rules.

Candidate 001's challenge is now development evidence. `challenge-002.jsonl`
contains 30 supported and 20 refusal requests authored/frozen before v4b training;
`challenge-002.manifest.json` records its hash and authorship limits. The same
acceptance bar applies. Kernel:
https://www.kaggle.com/code/dedruck/voxi-plan-v4b . Dataset:
https://www.kaggle.com/datasets/dedruck/voxi-plan-data-v4b . Artifacts are under
`~/.local/share/voxi/experiments/plan-model-v4b-2026-11-17/`.

## Candidate 002 outcome and candidate 003 hypothesis

Candidate 002 completed successfully on Kaggle. All 600 generated validation
rows matched, but its fresh challenge had only 16/30 exact supported plans,
eight supported refusals, six wrong accepted supported plans, two accepted
refusal requests, and three malformed outputs. Fifteen refusal labels matched.
This fails the fixed semantic gate. CPU decisions matched GPU on every row;
warm one-thread CPU p95 was about 136 ms, which does not establish native Q8
performance. The app still uses v3.

Reviewed wrong acceptances: a Japanese name lost its non-Latin characters;
daily became tomorrow once; a rename source changed to its target name; minutes
became hours; every-other-week became weekly; Friday became daily; email and
permissions requests became unrelated local file plans. Schema validity alone
would not stop these mistakes. See `challenge-002-score.json` and the preserved
run/CPU manifests. The generated validation result is not general accuracy.

Candidate 003 (`v4c`) uses the same existing-vocabulary wire format and restarts
from original v3 weights. It binds quoted values lexically, without command
grammar, to collision-free `~N~` aliases. Original byte-exact values remain on
the host. Training targets bind only literal-valued fields, preserving operation
and type words. Generation must retain every bound literal and may not invent
or embed an alias. This is a value-integrity check, not proof of action or role
semantics. REFUSE does not require reproducing literals.

Training expands generic capitalization, punctuation and politeness, arbitrary
rename literals, minute delays/repeats, arbitrary clock times including 24-hour
forms, weekday defaults, every-other-week/weekly/hourly expressions, and broad
unsupported-action/conditional/negation examples. No application natural-language
rules are added. There are 16,001 training and 800 validation examples. Every
row in train, validation and challenge passed exact preprocessor/target literal
round-trip and token-limit checks; maximum lengths were 55 input / 91 target.
Eight schema, alias and scoring unit tests pass.

The earlier two challenge banks are now development evidence only. Fresh
`challenge-003.jsonl` has 30 supported and 20 refusal rows; its hash and self-authored
limits were frozen in `challenge-003.manifest.json` before candidate 003 training.
The acceptance bar is unchanged. The GPU runner now also refuses missing EOS
and records raw model text separately from restored outputs.

Kernel: https://www.kaggle.com/code/dedruck/voxi-plan-v4c . Dataset:
https://www.kaggle.com/datasets/dedruck/voxi-plan-data-v4c . Artifacts:
`~/.local/share/voxi/experiments/plan-model-v4c-2026-11-17/`.

## Final result

See `REPORT.md`. Candidate 003 finished all eight epochs; the epoch-6 checkpoint
was selected by validation loss. The corrected host codec yields 20/30 exact
supported plans and blocks all 20 unsupported requests on the fresh challenge,
but three wrong accepted supported plans remain. The semantic gate fails. CPU
and GPU decisions agree after the same codec replay. The known training sentence
generates the intended daily two-step plan. Training artifacts and code are
preserved; no new model or input grammar was enabled in the application.
