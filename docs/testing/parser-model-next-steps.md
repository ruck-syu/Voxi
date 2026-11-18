# Parser and model fallback: next steps

Updated: 2026-11-15

## Current state

The parser has completed three rounds of rule fixes. The latest validation is:

- 223 unit tests pass, including the 2,662-command regression corpus.
- All 860,049 generated command-grid cases report zero mismatches.
- On the generated development set, 330 accepted paraphrase misreads and 158
  accepted refusal examples remain. These counts are not a production error rate.
- The tiny v3 model was selected during feasibility work. It has not been
  integrated into Voxi; the existing model-comparison results predate the rule fixes.
- Rule changes, audit artifacts, and the findings note are currently uncommitted.
- Evaluation, slot prototyping, independent-bank validation and per-stage Luna
  reviews are complete. No candidate passed integration gates; the conditional
  model integration remains unperformed.
- Evaluation refresh is complete; the durable snapshot, commands, hashes, and
  baseline are in [the refresh report](parser-model-refresh-2026-11-15.md).
  Guarded v3 has now been re-evaluated; see [the v3 report](parser-model-v3-2026-11-15.md).

See [the detailed findings](parser-rule-followup-2026-11-15.md) and
[the remaining-case audit](parser-audit-2026-11-15/after.md).

## 1. Refresh the evaluation inputs

Do this before interpreting any new model results.

- [x] Preserve the current parser revision, working diff, model checkpoint,
      tokenizer, inference settings, guard implementation, and evaluation date.
- [x] Move the feasibility scripts, generated data, model artifacts, and metadata
      from temporary storage into a durable experiment directory. Keep large
      artifacts outside Git and record their locations and hashes.
- [x] Regenerate rule outcomes for every evaluation sentence using the current
      Rust parser and the same fixed date used by the original evaluation.
- [x] Rebuild the fallback input list from sentences the current rules refuse.
      Include newly refused sentences; remove sentences now handled by the rules.
- [x] Update the comparison script to read these refreshed outcomes rather than
      saved historical `rules` fields. Keep development data and held-out sets separate.

**Done when:** every evaluation row has its expected intent, current rule outcome,
and fallback eligibility, with reproducible commands and artifact hashes.

The old `compare.py` cannot establish the effect of the parser fixes without
this refresh. Reusing its old fallback inputs would miss newly refused commands.
Use `scripts/parser_model_eval.py compare` with complete predictions keyed by
the IDs in the refreshed `fallback.jsonl`; the historical script is preserved
only as an experiment artifact.

## 2. Re-evaluate tiny v3 with its guards

Measure the current system before training another model.

- [x] Run the existing tiny v3 checkpoint on the refreshed fallback inputs.
- [x] Score each rewrite through the actual Rust parser and the existing inference
      guards. Keep refusals and rejected rewrites visible in the output.
- [x] Combine results using the actual decision path: keep the rule result when
      accepted; consider the guarded model result only when the rules refuse.
- [x] Save per-sentence outputs and summarize correct intents, refusals, accepted
      misreads, guard rejections, and accepted rewrites of requests labelled REFUSE.
- [x] Separate read-only misreads from commands that could change files. Parser
      acceptance alone does not demonstrate that the planner permits execution.
- [x] Measure inference latency and artifact size on the intended local runtime;
      label earlier Kaggle timings separately.

**Done when:** a reproducible rules-only versus rules-plus-v3 report identifies
where the remaining errors occur: rules, model rewrites, guards, or labels.

## 3. Review the remaining errors

Use the preserved cases rather than changing behavior to improve a headline count.

- [x] Review the 17 flagged one-day date-endpoint label conflicts against the
      documented inclusive-range behavior. Record justified corrections separately.
- [x] Review ambiguous item-versus-set and folder expressions such as `Archive PDFs`.
      Preserve literal-name behavior; refuse or clarify when intent is uncertain.
- [x] Check the 67 refusal examples flagged for planner-scope or hidden-path review
      at the planner boundary, with filesystem fixtures where appropriate.
- [x] Decide how recipient/cloud wording should be distinguished from a legitimate
      local folder with the same name. Avoid blacklisting arbitrary folder names.
- [x] Review archive-versus-image compression ambiguity and the remaining time,
      unsupported-request, and grammar categories.
- [x] Add targeted regression cases for confirmed bugs, fix them, and rerun the
      affected checks. Preserve complete before/after outcomes so newly introduced
      misreads and correct-to-refused regressions are measurable.

**Done when:** confirmed bugs, label corrections, planner restrictions, and product
ambiguities are documented separately, with no silent changes to expected labels.

## 4. Prototype placeholders before retraining

Treat placeholders as an experiment, not a demonstrated accuracy improvement.

- [x] Replace filenames, folders, and numerical values with stable typed slots
      such as `NAME1`, `FOLDER1`, and `NUM1`, preserving exact original values.
- [x] Verify round trips for quoted names, Unicode, apostrophes, paths, repeated
      values, decimal sizes, units, page ranges, and rename replacement direction.
- [x] Reject rewrites with missing, duplicated, invented, or incompatible slots.
      Keep semantic guards for actions, filters, units, and unsupported requests.
- [x] Keep relative dates symbolic. Resolve `this week` and similar expressions
      in Rust using the evaluation/runtime date, rather than asking the model
      to invent calendar dates.
- [x] Build a small training and validation experiment from development data.
      Compare with the refreshed v3 baseline before scheduling a full retrain.

**Done when:** substitution restores values exactly, malformed outputs are refused,
and a controlled experiment demonstrates whether retraining is worthwhile.

Multiple candidate rewrites were already evaluated and found unhelpful. Do not
repeat that experiment without a new hypothesis.

## 5. Validate on a fresh test set before integration

- [x] Freeze a new independently labelled test set before inspecting model failures.
      Cover literal names, unsupported actions, exclusions, dates, units, ambiguous
      requests, scheduling boundaries, and destructive operations.
- [x] Keep it out of training and grammar tuning. Report development-set results
      separately; do not describe previously inspected banks as pristine holdouts.
- [x] Compare rules alone, guarded v3, and any placeholder-trained candidate using
      the same parser revision, fixed date, guards, and scoring definitions.
- [x] Review every accepted destructive misread and refusal violation. Set explicit
      acceptance criteria for safety, useful coverage, latency, and package size.
- [ ] Integrate the selected model only after those criteria are met: export the
      artifact, add Rust inference, port/validate guards, and wire fallback to rule
      refusals while retaining previews, confirmation, and planner restrictions.

**Done when:** the selected approach meets documented acceptance criteria on the
untouched set and the intended local runtime. Model integration remains future work.

**Validation outcome:** no candidate qualifies. The conditional integration box
remains unchecked: safety, coverage for shared-guard candidates, artifact size
and native-runtime criteria are unmet. This is a recorded no-go decision, not an
integrated model. See [the final validation report](parser-model-final-validation-2026-11-15.md).

## Commands and artifact locations

From `app/src-tauri`, run the ordinary regression suite:

```sh
rtk cargo test --lib
```

For relevant parser changes, capture the full grid output:

```sh
rtk proxy cargo test --lib combo -- --ignored --nocapture --test-threads=1
```

The grids are report-only tests. Verify every printed mismatch count; a successful
exit code alone does not prove zero mismatches.

From the repository root, reproduce the error categorization with:

```sh
rtk proxy python3 scripts/parser_error_audit.py --input-dir <feas/gen> --output <report-prefix>
```

The feasibility workspace was last located at:

```text
/private/tmp/claude-501/-Users-vishalshekhar-Documents-dev-Voxi/1a9f1083-3c3f-414e-9d95-09a43d46e4b9/scratchpad/feas
```

Its `rules_check.py`, `compare.py`, model outputs, and training artifacts have
now been copied, with SHA-256 verification, to
`/Users/vishalshekhar/.local/share/voxi/experiments/parser-model-2026-11-15/legacy`.
Use that durable copy for subsequent work; the temporary original was preserved.
The repository already contains categorized snapshots in
`docs/testing/parser-audit-2026-11-15/` and the audit tool in
`scripts/parser_error_audit.py`.
