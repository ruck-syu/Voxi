# Final validation and integration decision

**Decision: no model qualifies for integration.** The refreshed evaluation,
remaining-case review, exact-slot prototype, controlled training experiment and
independent-bank comparisons are complete. The conditional export/Rust-inference
integration is **not performed** because its acceptance gates are unmet. No
candidate is approved for shipping, and the app continues to use its rule parser.

## Frozen independent evidence

Two separate 64-case banks were authored by Luna using the public README and
Intent schema, without inspecting parser code, training data or model failures.
Both original JSONLs and manifests remain unchanged. They are agent-authored,
not human-labelled, and provide limited evidence.

The initial bank had serialization and unit-convention problems and no supported
rule-refused positives. Its strict raw results remain in `fresh-evaluation-001`
and `fresh-evaluation-002`; all 33 unique accepted mismatches/violations have
individual reviews in `parser-fresh-initial-label-review-2026-11-15.jsonl`. Those
raw mismatch counts must not be presented as semantic accuracy.

A new independent author created the final bank with explicit schema helpers,
binary-unit conventions and unfamiliar natural paraphrases. The new author did
not see the initial bank or failures. It is a separately frozen bank, not a
post-hoc relabelling of the initial one. Its SHA-256 remains:

```text
5f6cb98125b068c24187799c15f5007ea5e9e3f682dd431df70429585ddcfa1a
```

The final bank covers natural paraphrases, literal names, units/filters, dates,
unsupported/exclusion requests, ambiguity, destructive operations and scheduling.
All positive labels passed a Rust Intent serialization check before inference.
Schema validity does not prove every semantic label: representation differences,
Windows-path assumptions and an incorrect digit-delay limit remain explicitly
reviewed. The author's midnight `freezeUTC` value is not used as chronology
evidence; the tool-delivered content hash and run provenance establish that the
bank was frozen before its outputs were examined.

No models, production parser behavior, behavioral guards, labels or gates were
tuned after viewing either bank. The banks stayed out of training and grammar
tuning. The only post-evaluation correction was to the test harness: it now
observes the app's existing refusal of scheduled read-only/Undo requests. The
actual app already performs that check in `lib.rs` after planning. This changed
only `final-064` from accepted to refused for every candidate; all other final
outcomes and all generated rewrites were unchanged. Earlier runs are preserved.

## Comparable final results

Reference run: `fresh-final-003` under the durable experiment root
`/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15`.
All candidates use the same Rust sources, fixed date 2026-10-28 at local noon,
frozen input labels, greedy generation and scoring definitions. Accepted rules
always win. Only current rule refusals reach fallback; scheduler/application
refusals are excluded from fallback. Schedule time and recurrence must survive
any accepted rewrite exactly.

All comparisons use the preserved copy/drop/unit guards. The shared-guard v3,
matched raw control and slot candidate also use the same additional conservative
semantic guard, making that comparison explicit. The original v3 configuration
is retained separately to show the effect of the additional guard.

These are **strict frozen-Intent counts**, including the representation and
label conflicts described above; they are not a production accuracy estimate.

| Configuration | Correct supported intents | Refused supported | Accepted misreads | Correct refusals | Accepted REFUSE labels |
|---|---:|---:|---:|---:|---:|
| Rules | 32 | 6 | 7 | 18 | 1 |
| Rules + original guarded v3 | 35 | 1 | 9 | 13 | 6 |
| Rules + v3, shared semantic guard | 32 | 6 | 7 | 16 | 3 |
| Rules + matched raw control, shared guard | 32 | 6 | 7 | 17 | 2 |
| Rules + slots, shared guard | 32 | 6 | 7 | 17 | 2 |

The original guarded v3 recovers 3 of the 6 supported rule refusals, while adding
accepted errors. The shared-guard configurations recover none on this small
bank. Useful coverage has only six opportunities here, so even the observed
50% recovery is weak generalization evidence.

## Every accepted error reviewed

`parser-model-fresh-final-review-2026-11-15.jsonl` records all **43 accepted-error
records across the four candidates**, covering **15 unique cases**, with original
labels, actual intents, decision origin and review. It includes every potentially
file-changing misread and every REFUSE violation, including read-only violations.
No label corrections were applied to these scores.

Confirmed problems include:

- Accepted rules change curly apostrophes inside literal source, replacement and
  new-folder names. A fallback cannot fix an intent already accepted by the rules.
- Original v3 drops parts of a source filename and rename replacement, while the
  preserved substring-based copy guard allows it.
- Original v3 rewrites unsupported upload/permission-change/content-edit requests
  into local operations, and drops an unsupported exclusion.
- The shared guard still permits some exclusion or compound-operation ambiguity;
  exact slot preservation alone is not semantic safety.

Other raw mismatches have different implications: splitting a relative path into
folder/basename can be equivalent to planner resolution of a slash-containing item
name; omitted generic `type: files` is a representation/default convention; a
Windows backslash decomposition is not established by the macOS contract; and
the 99 limit is for spoken numbers, whereas a numeric 101-hour delay is within
the existing one-minute-to-one-year limit. These do not erase the confirmed errors
or justify silently changing frozen labels. Parser acceptance is never described
as proof that the planner would execute an operation.

## Acceptance gates

The gates were fixed in the work log before fresh outputs were viewed. None was
relaxed after the results.

- **Safety: fail.** Zero accepted REFUSE labels and zero file-changing misreads
  are required. Raw scores fail, and confirmed literal-name/model errors remain
  even after separating label/representation questions.
- **Useful recovery: mixed/fail.** Original v3 exceeds the 10% threshold on this
  small bank but fails safety. Shared-guard raw/slot configurations recover 0/6.
- **Local CPU latency: pass for the experiment runtime.** Forty-sample benchmarks
  in the broader baseline and spread samples on the fresh inputs are recorded.
  Fresh p95 values are 32.99 ms (v3), 33.39 ms (v3/shared guard), 36.58 ms (raw
  control), and 37.57 ms (slots), below 200 ms. This is pinned Python/PyTorch CPU
  timing, including configured preprocessing/tokenization/generation/decoding;
  it does not validate a native Rust runtime.
- **Model artifact size: fail.** FP32 checkpoints plus tokenizer are approximately
  64.7 MB, above 25 MB. The measured qnnpack state file alone is 48,784,541 bytes
  before tokenizer/configs; it is not a complete quantized export. Kaggle's old
  15.6 MB estimate is not a measured package size.
- **Native runtime/package validation: not reached.** Installed experiment runtime
  files occupy 943,195,256 logical bytes; this is not a shipped package measurement.
  No eligible model was selected, so Rust inference, production guard ports and
  a distributable bundle were not introduced. Existing preview, confirmation,
  planner and scheduling restrictions remain in force.

The validation decision is a **no-go**, not completion of the integration
milestone. A future integration effort needs confirmed development-side fixes,
a candidate with a justified coverage/size hypothesis, and a newly frozen
independent test set before judging those changes. The inspected banks cannot
be reused as untouched holdouts.

## Reproduction and verification

```sh
rtk proxy <root>/runtime/bin/python scripts/parser_fresh_eval.py \
  --experiment <root> --output-name <unused-run-name> \
  --dataset docs/testing/parser-model-fresh-final-2026-11-15.jsonl
rtk proxy python3 -m unittest discover -s scripts -p 'test_parser*.py' -v
```

The run preserves the parser working tree, frozen labels, per-sentence rule and
model outcomes, all accepted errors, guard decisions, source hashes, inference
runner/helper sources, model/tokenizer hashes, package versions, timings and
gate outcomes. Input/model/runner/helper/prediction provenance is verified.

Final checks: **223 Rust unit tests passed, 18 ignored; 14 Python evaluation/slot
tests passed; all 860,049 command-grid cases report zero wrong outcomes**. The
grid covers the last production parser changes; subsequent Rust edits are in the
test-only evaluation bridge. `git diff --check` is clean. No model-generated
command was executed. All large artifacts remain outside Git.

Compact results: [final validation JSON](parser-model-final-validation-2026-11-15.json).
