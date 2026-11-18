# Parser rule fixes

Scope: parser and preview regressions only. No model, checkpoint, gate, frozen bank, evaluator manifest, or evaluation output was edited. The 120-case independent evaluation 001 review and captured outcomes were checked for the named cases before implementation. Its existing results were not rewritten.

## Changes

- `parse/phrase.rs`: recognize “archive files” as the archive type and parse explicit DMG words as exactly `.dmg`; generic “installers” remains the documented `.dmg` and `.pkg` family. Detect filter clauses accidentally appended to destinations, preserving quoted destination literals.
- `parse/files.rs`: move recognized, valid post-destination filters onto the source before parsing. Contradictory, invalid-date, missing-unit, and unparseable tails refuse instead of becoming unfiltered transfers into a folder with a longer name.
- `parse/rename.rs`: treat the first quoted numbering example as a format template. Its text before the trailing digits is the base, and those digits set the width.
- `parse/refusal.rs`: classify explicit web search, file-content replacement, and the underspecified “somewhere for these files” folder request as terminal refusals. Quoted literal values and a local folder named `web` remain valid.
- `parse/rule_regressions.rs`, `plan.rs`: add parser, scheduling, and temporary-fixture preview regressions. Numbering preview verifies concrete `shot-001.png` and `shot-002.jpg` names; transfer fixtures verify archive selection excludes `notes.txt` and DMG trash selection excludes `.pkg`.

The older `field-001` numbering form remains `base=field-`, width 3. The implementation derives names from the actual plan, not from a frozen label.

## Verification

- Focused parser regressions: 4 passed.
- Numbering preview fixture: passed.
- Archive and DMG preview fixture: passed.
- Full `cargo test --lib`: 247 passed, 18 ignored, 2 failed. Both failures are listed below; neither was hidden by changing corpus expectations.
- `haiku_300_per_case_report`: 299/300; per-case output is [haiku-300-current.jsonl](haiku-300-current.jsonl). Its sole difference is `H085` / `10-haiku-300.txt:103`, `list DMG files in Downloads`: old expected `installers`, actual `.dmg` only. The expected broad installer set contradicts the explicit extension and README/website semantics.
- Combination audit: `COMBO_ONLY=' to Archive'`, `COMBO_EXAMPLES=0`, `COMBO_DUMP` unset; **0 / 120,960 mismatches**. The full unfiltered audit took 69.51 seconds and produced the same summary. This checks generated move/copy combinations, not overall model readiness.

## Existing expectation conflicts for adjudication

No bulk expected-output changes were made.

- Three corpus rows encode explicit DMG wording as all installers: `10-haiku-300.txt:103` (`list DMG files in Downloads`), `2-ranges-sizes-types.txt:53` (same request), and `12-bank-2.txt:9` (`zap all dmgs in Downloads`). Exact suffix selection is `.dmg`; `.pkg` remains included only for the broader installer type.
- `13-bank-3.txt:29` (`delete Downloads dmg files, mate`) has the same broad-installer label conflict.
- The shared fallback guard test `bounded_transfers_preserve_operation_roles_and_both_size_limits` expected certifying a rewrite for `move files from Downloads to Desktop between 10 and 50 MB`. The original now parses directly with both valid size bounds, so certification returns `OriginalAccepted`. This test expectation needs parent adjudication; `fallback_guard.rs` was not edited.

The combined full-suite failure count includes the corpus test and the shared guard test above. The individual parser and planner regressions pass.

## Replay limits and source hashes

The frozen candidate 001 inputs and captured outcomes were used as the before evidence. I did not find an isolated candidate-001 parser binary in the assigned checkout to run a paired executable replay; after-state evidence is from the focused parser tests, real temporary-fixture plans, and the saved 300-row development replay. All fixture files were created under Rust temporary directories; no filesystem actions were executed.

Current source SHA-256 values:

```text
4cd9db87cecf73395e4c718e0ac472ec74858ab58b88a4f5200e8ef85d4c7c79  app/src-tauri/src/parse/phrase.rs
125df9bc9f93ab8ddcf2a4ceb59d10a4158ce6bc798937cd053aa0a2f1923370  app/src-tauri/src/parse/files.rs
64d987b57bfa4397acda4aceb089014caae12a3f6dd1e1df2f882854ef01db3f  app/src-tauri/src/parse/rename.rs
33c0af336c9933d9d1205078267f431a5fc6171ebe390251d89593c2f038addf  app/src-tauri/src/parse/mod.rs
85251dd9a821da935cd601f6c6d80649e1aabb2d14ecd574e352c625264ac3d5  app/src-tauri/src/parse/refusal.rs
7c1c45b92a1e205ae28b57352d6bbe1d8b8aa2d70fa4de751397496619b386e9  app/src-tauri/src/parse/rule_regressions.rs
2bac56b9daf1df85e7b50120564eef85764e82460c4ba3bc4d1d422ebbd6ab54  app/src-tauri/src/plan.rs
```

This report makes no overall model-readiness claim. The next independent bank should evaluate the integrated candidate.
