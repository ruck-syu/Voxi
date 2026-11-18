# Parser rule follow-up — 2026-11-15

Continues the fallback feasibility work. This pass changes the shipped rule
parser only; it does not install a model, change inference guards, or retrain.

## Fixes

- Extract explicit size comparisons before suffix comparisons. Previously,
  `over 9 MB and under 2 MB` consumed `9 MB and under` first, then read
  `over 2 MB`, reversing the bounds into a valid interval. Contradictory
  bounds are now refused; valid intervals and `9 MB and under` still work.
- Refuse unquoted `?` wildcards as well as `*`. Use the existing quote scanner
  so both single and double quotes protect literal names and apostrophes
  inside words remain valid.
- Refuse explicit requests to skip Trash or confirmation, and second actions
  such as `zip notes.pdf as backup.zip then delete notes.pdf`. Archive output
  captures previously swallowed these instructions as output names.
- Refuse subjective source descriptions such as `heavy videos`, `important
  docs`, `junk`, and `unused files`. Quoted literal names, explicit name filters,
  filenames with extensions, and numerical size filters remain supported.

## Validation

`cd app/src-tauri && rtk cargo test --lib`: 216 passed, 17 ignored.

`rtk proxy cargo test --lib combo -- --ignored --nocapture --test-threads=1`:
7 report-only audits passed; all 860,049 generated cases reported zero
mismatches. Checked the actual per-grid counts, not only the test exit code.
Full captured output: `/tmp/voxi-combo-followup-2026-11-15.log`.

`rtk git diff --check`: clean.

Generated-data check on the same 10,790 paraphrases and 2,022 refusal examples
used by the previous agent:

- Correct paraphrases: 2,037 (previous handoff: 2,035).
- Refused paraphrases: 8,157 (previous handoff: 8,139).
- Accepted paraphrase misreads: 596 (previous handoff: 616).
- Accepted refusal examples: 271 (previous handoff: 389).

These are development-set diagnostics, not fresh held-out accuracy. No bank
sentences were inspected or copied to implement this pass. Previously generated
examples supplied the failure categories. Some labels need review: e.g. the
refusal data includes system paths that are separately subject to planner
restrictions. An accepted parse is not proof that the operation can execute.

The feasibility dataset and scripts remain in the prior agent's temporary
scratchpad at
`/private/tmp/claude-501/-Users-vishalshekhar-Documents-dev-Voxi/1a9f1083-3c3f-414e-9d95-09a43d46e4b9/scratchpad/feas`.
Run `.venv/bin/python rules_check.py` there to reproduce these diagnostics.
The new regression tests are in
`src/parse/phrase.rs` and `src/parse/rewrite.rs` and do not depend on it.

## Remaining work

- Triage the 271 accepted refusal examples and the 596 paraphrase misreads.
- Implement and validate placeholder substitution for names and numbers before
  retraining. This pass does not demonstrate that slots improve accuracy.
- Keep relative dates symbolic during training and resolve them in Rust.
- Regenerate rule outcomes and fallback inputs before rescoring models. The
  existing `compare.py` uses saved bank `rules` fields and old model inputs;
  after rule changes these are stale, and newly refused inputs need inference.
- Evaluate on a fresh test set before drawing production accuracy conclusions.
- Multiple candidate rewrites were already measured by the prior agent and
  rejected as unhelpful; do not repeat that experiment without a new hypothesis.

## Second pass

Continues fixing generated-data errors without reading the held-out banks or
changing their labels.

Changes:

- Handle the complete `Would you mind` opener before stripping `Would you`.
  Otherwise `mind` was corrected to `find`, producing searches for unsupported
  requests such as encryption. Explicit gerunds are normalized separately.
- Resolve batch name sources (`PDF names`, `names of PDFs`, `every file name`)
  consistently for lowercase, date-prefix, and text-prefix operations.
- Recognize possessive single-file renames and explicit output labels such as
  `one file called combined.pdf` and `a single file, combined.pdf`.
- Preserve name filters in `whose name is`, `have ... in the title`, quoted
  name-before-type expressions, and explicitly quantified singular types such
  as `each scan PDF`.
- Resolve possessive folders for filtered sources, while leaving dates such as
  `today's screenshots` and literal filenames such as `King's report.pdf` alone.
- Read conditional size clauses and location verbs without including them in
  source folder names. Handle `there are` in counts, multiword destinations
  before a comma, and explicit folder labels.
- Keep a disjunction's `or` visible until it can be refused. It previously could
  vanish after the two filters were extracted. Supported forms such as `1 GB
  or more` and `on or before July 4` still work.
- Route explicit `dated` calendar days through calendar validation, including
  impossible dates and bare dates before file types. ISO dates embedded in
  screenshot filenames remain literal.
- Reject unsupported time descriptions instead of treating them as source
  folders, including suffixes after explicit filenames. Quoted names remain
  literal. This is conservative refusal, not new seasonal date support.
- Refuse content edits such as `remove background from photo.png`, including
  after polite openers. Validate sorter source folders and folder-creation
  instructions so chained text cannot silently become folder names.

Generated-data diagnostics (same development set as the first pass):

- Correct paraphrases: **2,158 / 10,790** (first pass: 2,037).
- Refused paraphrases: **8,168 / 10,790** (first pass: 8,157).
- Accepted paraphrase misreads: **464 / 10,790** (first pass: 596).
- Accepted refusal examples: **171 / 2,022** (first pass: 271).

`rtk cargo test --lib`: **219 passed, 17 ignored**, including the 2,662-command
regression corpus and new full-intent, refusal, and literal-name cases.
`rtk git diff --check`: clean.

Interpretation of what remains:

- Some generated targets disagree with existing documented parser semantics.
  There are 41 cases whose only differing field is the end-date bound, including
  inclusive ranges labelled with exclusive canonical targets. This pass does
  not change range semantics to fit those labels.
- Sources such as `Archive PDFs` are ambiguous between a folder-qualified set
  and a literal item name. This pass avoids guessing arbitrary capitalized
  folders, which previously broke literal-name regressions.
- Accepted refusal examples include system paths and hidden files that require
  planner checks. The existing `paths_outside_allowed_folders_are_refused` test
  covers refusal of `~/Library/x.txt` and path traversal; accepting an intent
  in this report does not establish that it can execute.
- Recipient/cloud requests need a product decision or disambiguation. A local
  folder can legitimately have the same name as a recipient or service; the
  parser should not blindly blacklist those names.
- No retraining, inference integration, or fresh bank accuracy claim is made.

Second-pass grid verification: **zero mismatches across all 860,049 cases**,
7 report-only audits completed. Raw counts were captured and checked explicitly
in `/tmp/voxi-combo-followup-2026-11-15-pass2.log`. A targeted rerun after the
last prefix-source change also reports zero mismatches for 40,320 batch rename
and 48 single rename cases; see
`/tmp/voxi-combo-rename-followup-2026-11-15-pass2.log`.

## Third pass: classification and targeted grammar fixes

A reproducible audit now lives in `scripts/parser_error_audit.py`. It reads the
existing generated diagnostics without modifying inputs or expected labels,
and writes a categorized JSONL plus summary with source hashes. Snapshots:

- `docs/testing/parser-audit-2026-11-15/before.jsonl` and `before.md`: all 464
  starting paraphrase misreads and 171 accepted refusal examples.
- `docs/testing/parser-audit-2026-11-15/after.jsonl` and `after.md`: all remaining
  cases after this pass. Categories are triage hints, not corrected labels.

Reproduce with:

```sh
rtk proxy python3 scripts/parser_error_audit.py --input-dir <feas/gen> --output docs/testing/parser-audit-2026-11-15/after
```

### Audit correction

The second-pass note grouped all 41 `before`-only differences as date-label
conflicts. The detailed audit supports **17** straightforward one-day endpoint
label conflicts. Many others were real grammar errors: `from August 1 on` was
limited to August 1 rather than interpreted as `since August 1`. This pass fixes
continuing dates while preserving `from August 1 on Desktop` as one date and a
location. It also supports the existing onward variants after `dated`.

Of the 160 remaining accepted refusal examples, 67 are flagged for planner-scope
or hidden-path review, 41 for recipient/cloud versus local-folder ambiguity,
and 6 for archive versus image-compression ambiguity. These hints are not proof
that every such request is valid or safe; they identify cases where changing
parser labels requires more context. No labels or model weights were changed.

### Grammar changes

- Read `weigh over/under` and `within N days` as precise filters; remove location
  and predicate syntax before it can become part of a name or source folder.
- Handle repeated query wrappers (`count how many`, `how many of`, `show a list
  of`), history phrasings, and explicit count/politeness closers.
- Recognize name contexts using `inside/within/under`, possessive file names,
  `labelled/labeled`, and `whose names include`. Known-folder names after explicit
  quantifiers stay folder scopes rather than becoming filename filters.
- Treat `leaving originals alone` as redundant only on a copy. Refuse a move
  with that instruction. Quoted destination names containing the same words
  remain literal.
- Accept explicit folder locations with `to` and `within`, repeated `for` in a
  name list, and `titled`. Location separators are scanned outside quotes;
  `make folders named "Photos to Documents"` creates one literal folder.
- Refuse unsupported exclusions (`excluding`, `apart from`, `files bar ...`),
  regex character ranges, explicit permanent-delete variants, and requests to
  delete Trash contents. Quoted pattern names and ordinary bracketed filenames
  remain literal.
- Restrict destination-first `X needs ...` shorthand to an explicit set, with
  separate copy handling. Unsupported needs-sorting/needs-new-folder requests
  are refused instead of moving an item named `sorting` or `new folder`.
- Preserve `just now` as an unsupported time unit instead of stripping `now`
  and treating the leftover `just` as part of a source name.

### Development-set validation

Same 10,790 paraphrases and 2,022 refusal examples as prior passes:

- Correct paraphrases: **2,378** (starting this pass: 2,158).
- Refused paraphrases: **8,078** (starting this pass: 8,168).
- Accepted paraphrase misreads: **334** (starting this pass: 464).
- Accepted refusal examples: **160** (starting this pass: 171).

The final error-text sets are subsets of the starting sets: zero newly accepted
misread texts and zero newly accepted refusal texts. A full-outcome comparison
saved before the final refinement also found zero previously correct
paraphrases regressed during that refinement. The initial snapshot preserves
errors only, so this latter check is not a full-turn correct-to-refused audit.

`rtk cargo test --lib`: **220 passed, 17 ignored**, including all 2,662 corpus
commands. Regression cases cover full intents, onward/date-location boundaries,
copy versus move instructions, unsupported patterns, and literal quoted names.
`rtk git diff --check`: clean.

These remain development-set diagnostics. The next model comparison still
requires refreshed rule outcomes and fallback inputs, followed by fresh held-out
evaluation. No claimed production error rate is inferred from these counts.

Third-pass final grid verification: **zero mismatches across all 860,049 cases**.
All 10 printed categories were checked, in 7 report-only tests (367.04 seconds).
Captured output: `/tmp/voxi-combo-followup-2026-11-15-pass3.log`.
An earlier grid run was stopped when the before/after audit exposed new grammar
errors; this completed run covers the final parser code.
