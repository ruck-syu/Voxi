# Tiny T5 native inference prototype

This isolated crate loads the unchanged T5 Efficient Tiny v3 checkpoint as a
Candle `quantized_t5::T5ForConditionalGeneration` model. It uses full
encoder/decoder generation, the original tied embedding and tokenizer, T5
relative position biases, CPU-only deterministic greedy decoding, EOS handling,
and a hard input-token limit. It is a prototype candidate for the parent
integration; it is not wired into the app and does not authorize or execute
model output.

## Reproduce the external export and build

The frozen checkpoint is at
`/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15/legacy/out-v3/model_t5-efficient-tiny-v3`.
Generated weights, the Python exporter environment, and Cargo target output stay
under `/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/`.

```sh
rtk python3 -m venv /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/export-venv
rtk /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/export-venv/bin/python -m pip install gguf==0.19.0 safetensors==0.8.0 numpy==2.5.5
rtk /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/export-venv/bin/python docs/testing/tiny-model-readiness-2026-11-16/native/export_gguf.py \
  --source /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15/legacy/out-v3/model_t5-efficient-tiny-v3 \
  --output /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/model
rtk env CARGO_TARGET_DIR=/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/target cargo build --release --locked --manifest-path docs/testing/tiny-model-readiness-2026-11-16/native/Cargo.toml
```

Exporter uses GGUF `Q8_0` for every 2D tensor whose row dimension is a multiple
of 32, including `shared.weight` (the 32,101 by 256 shared embedding). Small
relative-attention-bias matrices and 1D layer norms remain F32. The tied
`lm_head.weight` duplicate is omitted; Candle uses `shared.weight` for the
output projection. The GGUF tensor names and dimensions are validated by
Candle's own T5 loader. The config and tokenizer are copied without editing.

## Callable API

Library interface in `src/lib.rs`:

```rust
let mut model = voxi_tiny_native::NativeRewriter::load_with_threads(model_dir, 1)?; // once
let rewrite: Result<String, voxi_tiny_native::NativeError> = model.rewrite(text);
```

It reports typed `InvalidInput`, `InputTooLong`, `NoEos`, or `Model` errors.
`load_with_threads` initializes Candle's process-wide Rayon pool, so call it
before other host code initializes Rayon. It rejects thread counts outside
1–64 and reports an error if the host initialized Rayon first. `load()` is a
one-thread convenience wrapper.
Tokenizer-internal truncation is disabled explicitly before encoding; inputs
over 64 actual tokens are rejected. Decode starts with configured token 0,
greedily selects argmax up to 64 new tokens, stops on configured EOS token 1,
and returns `NoEos` if EOS is absent. `use_cache` is disabled because Candle's
T5 KV cache expects incremental one-token calls; this implementation replays
the decoder prefix at each step. The CLI loads once and then accepts JSONL
`{"id":"...","text":"..."}` on stdin, emitting
`{"id":...,"text":...,"rewrite":...,"elapsed_ms":...}` with an optional
`error`. Set `--threads N` (1–64); the benchmark used one CPU thread. Errors
have a null `rewrite`. Missing/corrupt model files fail loading before input is
read. The library can be called by a parent-owned policy layer; its output must
remain subject to that layer's semantic checks and normal preview/confirmation.

Example:

```sh
rtk printf '%s\n' '{"id":"1","text":"find files in Downloads"}' | \
  /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/target/release/voxi-tiny-native \
  --model-dir /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/model --threads 1
```

## Evidence

`candidate-manifest.json` records original/export/tool/runtime hashes and the
measured package/runtime sizes. The measured model package is 18,985,400 bytes:
GGUF 16,562,016; tokenizer 2,422,598; config 786. This is below the 25,000,000
byte cap. The native arm64 release executable is 7,570,288 bytes; `otool -L`
shows only the OS-provided `libSystem` and `libiconv` dylibs, so there are no
additional bundled runtime libraries. Cargo dependency versions are pinned by
the checked-in `Cargo.lock`.

On this arm64 Mac, the post-fix 300-row parity run is in
`/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/parity-q8-api-final/`.
It samples 300 evenly spread rows from the pre-existing 9,940-row
`generated-development` subset in the old v3 FP32 predictions; it uses no new
holdout. The old saved Python FP32 rewrite is the reference, not a new Python
inference run. All 300 native Q8 rewrites match it exactly; there are no
mismatches to list. Three warmups precede 40 evenly spread timings. Timing
includes tokenization, encoder/decoder inference, and decode, excludes model
load, and reports 82.85 ms p95 / 48.53 ms median (index floor of sorted 40
samples at `floor((n-1)*0.95)`). Model load was 43.28 ms.

Failure behavior was exercised on-device: missing model directory and invalid
GGUF report load errors; malformed JSON, absent/non-string text, and empty
text produce per-line errors; an over-limit 416-token sentence is rejected
without truncation; an under-limit input whose model output does not reach EOS
returns `max_new_tokens reached without eos`. This last deliberately long
generation took about 631 ms in the final smoke run (380 ms in an earlier run),
so the 200 ms p95 evidence applies to the
specified known-development sample, not a worst-case 64-token output guarantee.

Re-run parity and latency with:

```sh
rtk python3 docs/testing/tiny-model-readiness-2026-11-16/native/run_parity.py \
  --reference /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15/v3-local-fp32/predictions.jsonl \
  --binary /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/target/release/voxi-tiny-native \
  --model-dir /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/model \
  --output /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/parity-q8-new
```

Re-run the failure cases and save their raw evidence with:

```sh
rtk python3 docs/testing/tiny-model-readiness-2026-11-16/native/failure_smoke.py \
  --binary /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/target/release/voxi-tiny-native \
  --model-dir /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/model \
  --output /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/failure-path-results.json
```

This establishes an export/runtime prototype, not overall readiness: the
independent untouched-bank and semantic-guard results belong to the parent
integration task.
