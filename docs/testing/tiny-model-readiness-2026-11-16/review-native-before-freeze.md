# Native integration review — before freeze

Reviewed the current native crate, CLI, GGUF exporter, lockfile, manifest, README, parity harness and parent runtime checks. This is a review of the native candidate only; it is not app-release approval.

## Findings

### Medium: package identity is not checked by the native loader

[`native/src/lib.rs:67`](/Users/vishalshekhar/Documents/dev/Voxi/docs/testing/tiny-model-readiness-2026-11-16/native/src/lib.rs:67) checks that config, GGUF and tokenizer files exist, then parses/loads them. It does not check their hashes or verify that the three files belong to the same package. The manifest records exact hashes in `native/candidate-manifest.json`, but those values are not consumed by `load`.

The current recorded package hashes match the external files, and the smoke test catches a missing directory and a GGUF with invalid magic. Those checks do not catch a valid GGUF paired with a different valid tokenizer/config, or a valid but replaced GGUF. Such a package may load and return rewrites under the wrong artifact identity. Before shipping package files through an integrity-unmanaged path, bind loading to a versioned package manifest/checksums (or enforce the same check in the packaging/installation layer) and add a valid-file mismatch failure case. If the app installer guarantees immutable, verified package contents, document that contract at the integration boundary.

This is a packaging/integration gate, not evidence that the currently measured candidate is mismatched. The current manifest and external artifact hashes were verified during this review.

## Checks and evidence

- Exporter copies the checkpoint config/tokenizer, omits the tied `lm_head.weight`, quantizes eligible matrices including shared embedding to Q8_0, and keeps other tensors F32. This matches the documented Candle T5 loader path; source tensors/dimensions are validated by that loader when loading the exported artifact.
- Runtime disables tokenizer truncation before validating the 64-token limit; adds the tokenizer's configured special tokens; seeds decoder from configured start ID; uses greedy decoding through configured EOS and fails closed on missing EOS. It replays the decoder prefix with cache disabled, as documented. Each `NativeRewriter` owns its Rayon pool, and `&mut self` serializes calls per instance. The blocking API correctly leaves worker scheduling and stale-request checks to the caller.
- CLI input errors are returned per line with null rewrite, output is flushed after each line, and package load errors exit before reading stdin. Parent runtime evidence reports live streaming, post-global-pool initialization, repeated instance and failed-load retry checks.
- Verified all seven manifest tool/source hashes: exporter, parity harness, failure harness, Rust library, Rust CLI, Cargo.toml and Cargo.lock. Verified external GGUF, native predictions, latency log, failure report and FP32 reference hashes against the manifest. Recorded package size sums to 18,985,400 bytes; native arm64 binary is recorded as 7,595,488 bytes with only the stated OS dynamic libraries.
- Benchmark protocol is provenance-limited as documented: saved FP32 outputs over a generated-development sample, 300 exact matches and 40 sampled timings after warmup; it is not a new Python inference comparison, worst-case latency bound, or independent validation. The distinct adversarial sample has three quantization differences (T032, T073, S003); universal FP32 equivalence must not be claimed.

No native inference correctness blocker was confirmed from the reviewed implementation/evidence. Semantic routing, policy checks, request-generation checks, independent-bank results and app integration remain outside this native review. No holdout data was inspected and no code/model changes were made.
