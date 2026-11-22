# User-requested development activation

After the failed semantic evaluation was reported, the user explicitly asked to
enable the trained model. The opt-in native development app now uses v4c Q8,
identified by `voxi-plan-v4c-q8_0-2026-11-17`. This activation does not mean the
training safety/coverage/latency gates passed. Training results remain unchanged.

Run from `app`:

```sh
rtk npm run dev:model
```

The launcher enables the existing `tiny-model` Cargo feature and sets
`VOXI_PLAN_MODEL_DIR` to the exported v4c package under
`~/.local/share/voxi/experiments/plan-model-v4c-2026-11-17/native-model`.
The package is 18,985,148 bytes and contains Q8 weights, tokenizer and config.
All three file hashes are pinned in the native loader; v3's separately pinned
loader and weights remain available. See `native-package-manifest.json`.
Ordinary builds do not enable this model automatically.

## Application behavior

`VOXI_MODEL_ROUTING=fallback` is the default. The rules run first, and the model
can interpret eligible grammar misses and failed lookups for unquoted transfer
items that read like references. Each model step must account for the full
wording of its clause. Transfer readings require an explicit source folder
and a supported file type. Quote multiword names so Voxi can distinguish
them from qualifiers. Folder lookup errors and named items stay on the
rule path. Set `VOXI_MODEL_ROUTING=legacy` to try the model before eligible rules
and skip the retry after a rule lookup fails. The values `0`, `false`, and
`off` also select legacy mode, regardless of case. Voxi logs the mode at
startup when the structured model is enabled.

Hard refusals, read-only requests, saved shortcuts, and previous-result scopes
keep their rule paths. Trash commands and transfers into Trash or bin folders
are rule-only in both modes. Repeating tasks also require a rule interpretation.
One-time model schedules must match the scheduler's reading of the original
command.

The model returns typed steps and a symbolic schedule. The decoder preserves
quoted names and checks literals, supported operations, file types, folder
references, and schedule bounds. The planner checks the resolved filesystem
paths, allowed roots, and transfer destinations. The preview displays a model
notice above the steps and still requires confirmation. Cancellation and
request-generation checks cover inference and preview publication.

## Historical activation results and remaining limits

- The native Q8 audit processed all 50 frozen cases using the actual Rust literal
  adapter, intent compiler and scheduler. It exactly handled 21/30 supported
  requests, refused seven, and accepted two wrong schedules (S006 and S027).
  All 20 refusal requests were blocked. See `native-audit-summary.json`.

Quantization/runtime results differ from the earlier FP32 report and do not pass
the original semantic gate. S006 loses the named Wednesday/default-time semantics;
S027 turns Friday recurrence into one run tomorrow. These remain known errors.
The mixed 50-request native audit measured about 1.69 seconds p95 after optimizing
model/matrix dependencies; this is not a dedicated latency benchmark and does not
meet the old 200 ms goal. The current bounded decoder replays prefixes rather than
using an unvalidated KV-cache change. Model libraries are optimized in dev while
the app stays debuggable. No broader production-readiness claim is made.

The activation is development-only and uses local external artifacts. It does not
publish a release or bundle a Python runtime. Inference is native Rust/Candle.

Re-run the real-model checks and report-only audit from `app/src-tauri`.
The audit applies the fallback wording and schedule checks to all 50 model
outputs, including commands the app routes to rules. It counts Trash steps
in the frozen corpus as expected refusals.

```sh
rtk proxy env VOXI_PLAN_MODEL_DIR="$HOME/.local/share/voxi/experiments/plan-model-v4c-2026-11-17/native-model" VOXI_PLAN_AUDIT_OUT=/tmp/voxi-native-plan-audit.jsonl cargo test --locked --lib --features tiny-model structured_model_ -- --ignored --nocapture --test-threads=1
```
