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
rtk /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/export-venv/bin/python crates/voxi-tiny-native/export_gguf.py \
  --source /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15/legacy/out-v3/model_t5-efficient-tiny-v3 \
  --output /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/model
rtk env CARGO_TARGET_DIR=/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/target cargo build --release --locked --manifest-path crates/voxi-tiny-native/Cargo.toml
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

It reports typed `InvalidInput`, `InputTooLong`, `NoEos`, `PackageMismatch`, or `Model` errors. It also implements `std::error::Error`.
`load_with_threads` creates a dedicated Rayon pool for each model instance. It
works after the host has initialized Rayon and after failed or repeated model
loads; it never reconfigures the global pool. It rejects thread counts outside
1–64. `load()` is a one-thread convenience wrapper. The API is blocking; use a
worker thread in the app, and keep normal cancellation and request-generation
checks around the inference job.
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
rtk proxy printf '%s\n' '{"id":"1","text":"find files in Downloads"}' | \
  rtk proxy /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/target/release/voxi-tiny-native \
  --model-dir /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/model --threads 1
```

## Evidence

`candidate-manifest.json` records original/export/tool/runtime hashes and the
measured package/runtime sizes. The measured model package is 18,985,400 bytes:
GGUF 16,562,016; tokenizer 2,422,598; config 786. This is below the 25,000,000
byte cap. The native arm64 release executable is 7,595,040 bytes; `otool -L`
shows only the OS-provided `libSystem` and `libiconv` dylibs, so there are no
additional bundled runtime libraries. Cargo dependency versions are pinned by
the checked-in `Cargo.lock`.

On this arm64 Mac, the post-fix 300-row parity run is in
`/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/parity-verified-package/`.
It samples 300 evenly spread rows from the pre-existing 9,940-row
`generated-development` subset in the old v3 FP32 predictions; it uses no new
holdout. The old saved Python FP32 rewrite is the reference, not a new Python
inference run. All 300 native Q8 rewrites match it exactly; there are no
mismatches to list. Three warmups precede 40 evenly spread timings. Timing
includes tokenization, encoder/decoder inference, and decode, excludes model
load, and reports 67.04 ms p95 / 41.50 ms median (index floor of sorted 40
samples at `floor((n-1)*0.95)`). Model load was 107.55 ms.

Failure behavior was exercised on-device: missing model directory and invalid
GGUF report load errors; malformed JSON, absent/non-string text, and empty
text produce per-line errors; an over-limit 416-token sentence is rejected
without truncation; an under-limit input whose model output does not reach EOS
returns `max_new_tokens reached without eos`. This last deliberately long
generation took about 453 ms in the current smoke run,
so the 200 ms p95 evidence applies to the
specified known-development sample, not a worst-case 64-token output guarantee.

Re-run parity and latency with:

```sh
rtk python3 crates/voxi-tiny-native/run_parity.py \
  --reference /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15/v3-local-fp32/predictions.jsonl \
  --binary /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/target/release/voxi-tiny-native \
  --model-dir /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/model \
  --output /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/parity-q8-new
```

Re-run the failure cases and save their raw evidence with:

```sh
rtk python3 crates/voxi-tiny-native/failure_smoke.py \
  --binary /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/target/release/voxi-tiny-native \
  --model-dir /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/model \
  --output /Users/vishalshekhar/.local/share/voxi/experiments/parser-model-readiness-2026-11-16/native/failure-path-results.json
```

This establishes an export/runtime prototype, not overall readiness: the
independent untouched-bank and semantic-guard results belong to the parent
integration task.

## Parent integration hardening

The parent replaced process-global pool initialization with a per-instance pool,
matched PyTorch first-index argmax tie handling, and flushed CLI responses before
stdin EOF. Two unit regressions pass, along with a real-model multiple-instance
and failed-load retry test after host pool initialization. `parent-runtime-checks.json`
records a live streaming response before stdin closes and rechecked package hashes.

The current candidate manifest supersedes the original benchmark/source fields.
Current parity evidence is under `native/parity-verified-package` in the external
experiment root, with 300/300 generated-development rewrites matching FP32. The
separate adversarial 300 commands match on 297: T032, T073 and S003 differ after
quantization. Thus the export is not universally output-equivalent to FP32.
None of those three commands is an effective model addition when original rules
have priority; the final certificate evaluation must still verify that claim.

## Package identity

The loader pins the versioned `MODEL_PACKAGE_ID` to SHA-256 hashes of all three
package files. It parses the exact verified buffers, including GGUF, rather than
reading the paths again after verification. A valid but replaced config/tokenizer
or weights file therefore fails closed as `PackageMismatch`. This closes the
packaging finding in `../review-native-before-freeze.md`; new exports require an
explicit candidate/version update and validation, not an unverified runtime switch.
The failure harness includes a valid JSON config with a changed decoder-start
ID and requires rejection before input/inference.
