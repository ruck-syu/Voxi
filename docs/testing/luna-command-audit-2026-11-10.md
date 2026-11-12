# Luna file command audit — 2026-11-10

The ignored report-only harness exercises the compiled Rust parser and Planner against independent full-intent expectations. It executes no actions and uses only temporary fixture homes.

## Coverage and results

- 216 fresh natural phrases: 76 exact intent-and-argument passes, 110 parser refusals, and 30 parsed mismatches guarded by the Planner (no steps produced).
- 161 established corpus controls: 150 passes, 8 refusals, and 3 guarded mismatches.
- 10 additional targeted nested-path examples: all 10 match their expected parser intents.
- 58 negative/ambiguous examples: 28 safely refused, 18 ambiguity-recorded, and 12 accepted negatives. Accepted negatives are expected to be inspected; these are inherited controls, not new discoveries.
- 3 contextual references resolved with `scope::prepare` against a temporary snapshot; contextual move/copy/trash plans were asserted. The nested item lookup and move plans were also asserted against exact Planner results.
- Fresh coverage spans list/show/count, move/copy/rename/trash, zip/PDF output names, Documents/Dev, multiword names, qualifier reordering, and named/called wording. PDF/image fixtures use placeholder bytes, so content validation for those formats is not claimed.

Fresh accepted mismatches have **no planner steps**. For example, `move the Documents/Dev file Project Plan.pdf into the Backup folder` parses as a move of the literal item `Documents/Dev file Project Plan.pdf`, then the planner refuses it because it cannot resolve the source. `move Project Plan.pdf located in Documents/Dev across to Backup` retains `located` as part of the item name and is also refused by the planner. `show the file in Documents/Dev called Notes from Monday.pdf` is misread as a date/name filter and a malformed folder, then refused. These are medium-severity interpretation mismatches with a planner safety guard, not executable wrong actions. `in Documents/Dev show Project Plan.pdf` is refused at parse time.

The report separates fresh cases (`new_phrase`), older corpus controls (`control`), inherited negative/ambiguous cases, and contextual cases. Previously known compound-number/alias refusals, explicit-dot extension gaps, extra-root bare-name lookup, and first-N references are not reclassified as newly discovered here. Planner root/path/filter refusals count as successful safety guards, not unsafe execution.

## Reproduction and artifacts

Run from `app/src-tauri`:

```sh
rtk cargo test --lib luna_command -- --ignored --nocapture
```

The ignored test writes one JSONL row per case, with expected and actual full intent arguments, planner outcome, severity, and category, to [luna-command-audit-2026-11-10.jsonl](luna-command-audit-2026-11-10.jsonl). The harness is in `app/src-tauri/src/luna_command_audit.rs`; it is ignored so report mismatches do not fail the default suite.

Limitation: paths and file contents are temporary fixtures; no actual user folders, application automation, confirmation, or execution are involved. This audit makes no production changes.

Parent verification: both ignored audit harnesses passed, and the default library suite passed with 177 tests and 8 ignored. Ten X-prefixed targeted cases were reclassified separately from negatives; their results did not change. Fresh cases reuse wording templates across three file names, so 216 rows do not represent 216 distinct grammar structures. A parser pass does not guarantee planner success for every row: only the specific nested-path and contextual assertions described above check exact preview outcomes. Planner-blocked mismatch results describe these fixtures, not a guarantee for arbitrary filesystem contents.
