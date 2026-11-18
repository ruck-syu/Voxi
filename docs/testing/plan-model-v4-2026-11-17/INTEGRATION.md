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

Eligible file changes and compound requests are sent to the model before the
rule parser, including their original schedule wording. The model returns typed
steps and a symbolic schedule. Quotes bind literal values on the host, preserving
Unicode exactly and refusing omitted, invented or embedded aliases. Output
must match the supported protocol, operation set, file types, previous-folder
references and schedule bounds. Only then is it converted to typed app intents.
No additional input-language patterns interpret the model's command.

Existing explicit hard refusals remain terminal, except the multiple-action
classification which now admits model interpretation. Read-only and other
recognized command kinds, exact saved shortcuts and previous-result scopes keep
their existing paths. A model REFUSE falls back to the rules; invalid model output
or inference failure is an error and never a partially accepted model plan.

Planning still resolves actual filesystem names, enforces allowed roots, reserves
non-clobbering outputs, blocks unsupported file dependencies, and shows the normal
preview. Model previews include a reminder to review source folders and schedule.
File changes and scheduling still require confirmation. No generated text is
executed as a shell command.

A recurring model task persists the approved typed `Intent` in the new optional
`Rule.modelIntent` field. At future runs, the ordinary planner expands that same
intent against current files; it does not call the model or reinterpret its text.
Old jobs deserialize with no model intent and retain their original behavior.
One-time tasks continue to persist their exact previewed actions and fingerprints.
Cancellation and request-generation checks surround model inference and preview
publication. Model work runs on the existing blocking planning worker, not the UI.

## Verification and remaining limits

- 267 Rust tests pass with the model feature; 23 opt-in/report tests are ignored.
- 261 Rust tests pass without the model feature.
- The frontend TypeScript/Vite build passes.
- Real-model native test: the known daily create-folder/move-PDFs sentence produces
  two typed steps and a daily repeat.
- Real-model app test: preview changes nothing, confirmation saves the schedule,
  the stored typed intent survives reload, a later run works even if its text is
  deliberately unparseable, and one undo reverses the run.
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

Re-run the real-model checks and report-only audit from `app/src-tauri`:

```sh
rtk proxy env VOXI_PLAN_MODEL_DIR="$HOME/.local/share/voxi/experiments/plan-model-v4c-2026-11-17/native-model" VOXI_PLAN_AUDIT_OUT=/tmp/voxi-native-plan-audit.jsonl cargo test --lib --features tiny-model structured_model_ -- --ignored --nocapture --test-threads=1
```
