# Remaining-case review — 2026-11-15

This records decisions for all 494 preserved development errors. It separates
confirmed parser bugs, proposed label corrections, planner restrictions and
product ambiguities. The categories are not evidence that every original label
is correct. Raw labels and the original reports remain unchanged.

## Confirmed bugs fixed

- Explicit `put … into a zip called …` requests previously lost the ZIP phrase
  during canonicalization and selected move or PDF merge. The request now keeps
  its archive action. `put PDFs together as out.zip` also selects ZIP without
  narrowing a generic files source to PDFs. A move to a literal `Archive.zip`
  folder retains move semantics.
- Image recompression requests such as `compress photos … to make them smaller`
  are refused instead of creating an archive called `make them smaller`.
- Unsupported `hidden files/folders` selection is refused instead of applying
  a filename substring filter to visible files. Quoted literal names and
  explicit `files named hidden` filters remain supported.

Targeted full-intent/refusal/literal tests are in `parse/archives.rs`.

## Labels

All **17** flagged one-day upper-bound conflicts have separate proposals in
`label-corrections.jsonl`. The parser's inclusive range contract stores an
exclusive `before` boundary one day after the stated end day. Each proposal
changes that field only and records the original intent, proposed intent,
reason and `applied_to_scoring: false`. Original baseline results are untouched.

Other potential label conflicts remain contextual: `from last year` is already
documented by the regression contract as a rolling 365-day interval, and words
such as `from Documents` caused false-positive time triage hints. Folder typos
can be real literal folder names. These are documented for interpretation rather
than silently corrected to match implementation.

## Planner boundary

`planner.jsonl` contains an actual planner outcome for each of the **67** flagged
sentences using temporary homes with existing outside-root Library/Applications
and hidden sentinels, ordinary allowed files, and selected legitimate local names.
No actions were executed. Results: **63 planner refusals, 1 parser refusal,
1 ambiguity requiring a choice, and 2 previews**.

The two previews are copying a local `photo library` directory to a local
`Google Photos` directory, and trashing a local `Downloads/Keychains` directory.
These demonstrate why a name blacklist would be incorrect. They do not prove
that the generated refusal labels describe a safe intended request. A failure to
find a file in these fixtures is also not a universal security restriction.

The non-ignored boundary regression tests separately prove that existing
`~/Library`/`~/.ssh`, absolute system paths and parent traversal are blocked;
local service/person/protected-looking folder names remain valid; discovery
skips hidden entries; and an **explicit hidden path inside an allowed root**
is permitted by the existing contract. Hidden discovery and explicit path access
are different behaviors. Tests also preserve concrete preview paths and do not
infer execution permission from a parse.

Run the complete preserved-case fixture audit from `app/src-tauri`:

```sh
rtk proxy env \
  VOXI_PLANNER_AUDIT_INPUT=<repo>/docs/testing/parser-audit-2026-11-15/after.jsonl \
  VOXI_PLANNER_AUDIT_OUTPUT=<output.jsonl> \
  cargo test --lib model_audit_all_flagged_planner_cases -- --ignored --nocapture
```

## Product decisions

- **Item versus set:** preserve literal names such as `Archive PDFs`. Capitalization
  alone never establishes a folder scope. Explicit `PDFs in Archive` identifies a
  set; quoted `"Archive PDFs"` identifies a literal item. Uncertain filesystem
  matches require a choice instead of an arbitrary folder interpretation.
- **Recipients/cloud:** file commands have local filesystem semantics only.
  A destination named `Dropbox`, `Priya` or `Google Photos` can be legitimate.
  The planner resolves or creates a local destination and previews the concrete
  paths. No upload, sharing, messaging or network transport is inferred. Explicit
  transport requests are unsupported; the 41 generated labels remain unchanged
  because their intended remote/local meaning cannot be inferred from names alone.
- **Compression:** `compress` remains the documented ZIP synonym, including when
  the input is an image. It does not promise image recompression. Explicit quality
  or smaller-image requests are refused; numerical `resize … to 640 px` and
  explicit `zip` are supported ways to disambiguate.
- **Remaining grammar:** the per-case decision inventory preserves the 145
  item/set ambiguities and the remaining source/output/filter/time/unsupported
  categories. Broad guesses would damage literal-name behavior. These unresolved
  product/grammar limits remain visible as errors/refusals and in acceptance gates;
  this review does not claim every generated paraphrase is now understood.

## Complete before/after evidence

The durable experiment root is
`/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15`.
`refresh-004/evaluation.jsonl` preserves every pre-fix outcome;
`refresh-005/evaluation.jsonl` preserves every post-fix outcome for the same
16,731 IDs and original labels. `step3-transitions.jsonl` records every change.

There are **7 changed outcomes**: four misreads become correct, two accepted
REFUSE labels become refusals, and one remaining item/set error changes action
without becoming correct. Across the full evaluation, **zero previously correct
outcomes regress and zero newly accepted misreads are introduced**. Generated
accepted misreads are now 330 and accepted REFUSE labels 158. Corpus and bank
outcomes are unchanged. Detailed transitions are in `summary.json`.

Validation: 223 unit tests passed, 18 ignored, before the test-only scheduling
bridge addition; targeted archive tests passed afterward. Full grid results are
recorded in the work log once its report-only mismatch counts are verified.

The scheduler bridge is an evaluation-only extension for the separately frozen
fresh set: it scores scheduling before parsing, marks pre-parser scheduling
failures, and records whether a valid intent is scheduled. It does not change
production parser or scheduling behavior.
