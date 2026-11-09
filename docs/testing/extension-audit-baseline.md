# Extension handling audit baseline

Status: executable audit complete for the finite inventory below. Extensions do not have an exhaustive global registry, so this report does not claim literal exhaustive coverage.

## Contracts examined

- Parser extension words and families: `app/src-tauri/src/parse/phrase.rs`, `EXT_WORDS`, `WORD_TYPES`, and `type_word`.
- Planner type families: `IMAGE_EXTS`, `VIDEO_EXTS`, `DOC_EXTS`, `ARCHIVE_EXTS`, `INSTALLER_EXTS`, `AUDIO_EXTS`, and `FileType::accepts_ext` in `app/src-tauri/src/model.rs`.
- Planner uses `Path::extension()` lowercased (`plan.rs::ext_of`/`matches_prepared`), so a literal compound filter such as `ext=["tar.gz"]` does not match a file ending in `.tar.gz`; it sees only `gz`. Recursive set listing skips directories and does not leave the requested folder.
- Established aliases: JPG/JPEG and HEIC/HEIF share extension aliases. The parser's `raw` word maps to a raw-camera family. Category words such as `docs`, `images`, and `archives` map to model types. Aeroguard remains a name group, not an extension.

## Inventory

`extension-audit-catalog.txt` has 347 distinct simple suffix spellings in 352 entries (five repeated entries), covering all literals in the parser/model tables and 250+ common formats across the requested domains. `extension-audit-compound-suffixes.txt` has 11 endings. `Dockerfile` is a filename control, not an extension. Digit-containing formats are included. Uppercase behavior is exercised by the STL fixtures.

## Parser outcomes

The opt-in Rust test parses every catalog suffix in `list .<suffix> files in Downloads`. It produced 1,396 JSONL rows at [extension-audit-results.jsonl](/Users/vishalshekhar/Documents/dev/Voxi/docs/testing/extension-audit-results.jsonl): 347 catalog suffixes, 1,024 deterministic generated suffixes, 19 explicit boundary/compound samples, and 6 case/singular/plural/list/count phrase variants.

Of 347 catalog phrases, 37 were recognized as extension or category filter syntax and 310 were parsed as `nameContains` filters. Of 1,043 generated/boundary phrases, 0 were recognized as extension filters; all 1,043 became `nameContains` filters. All six phrase variants were recognized. The 1,353 explicit-dot misses are against the independent expectation that leading-dot suffix syntax identifies an extension filter. Examples: `list .3mf files in Downloads` becomes a name filter for `.3mf`; generated `.0` becomes a name filter for `.0`. The parser did not refuse these commands.

The JSONL separates `recognized_extension_syntax`, `supports_requested_suffix`, and `exact_alias_match`; a recognized type/category is not automatically a precise suffix match. Of 37 recognized catalog cases, all 37 filters include the requested suffix and 34 exactly match the expected extension alias list. `.dmg`, `.pdf`, and `.zip` use installer/PDF/archive categories rather than suffix arrays. PDF is semantically an exact single-suffix match; installers and archives can include additional suffixes and deserve review for explicit-dot requests. The established RAW/JPG/HEIC alias expectations also differ from strictly literal extension matching. Expected alias lists preserve JPG/JPEG, HEIC/HEIF, and the current raw-camera family. These results characterize parser outputs, not file matching.

Generated sampling uses xorshift32 seed `0x564f5849`, alphabet `[a-z0-9]`, lengths 1–8, and 1,024 unique non-catalog samples. Nineteen additional samples cover lengths over 8, hyphens, underscores, Unicode, and every one of the 11 compound endings in the checked-in compound catalog. `has_extension`'s 8-character ASCII rule is a filename-classification heuristic, not a general syntax rejection rule.

## Planner outcomes

Two ignored planner audits use temporary fixture homes and only produce previews. The STL audit checks list, count, move, and trash forms with normal nested matches, uppercase suffix, `STL guide.txt`, `STL-backup.zip`, `model.stl.txt`, a directory named `folder.stl`, an extensionless `Dockerfile`, and a same-suffix file outside Downloads. The expected three matching files are returned, and no actions execute.

The separate direct-planner matrix at [extension-planner-results.jsonl](/Users/vishalshekhar/Documents/dev/Voxi/docs/testing/extension-planner-results.jsonl) covers OBJ, 3MF, BMP, `tar.gz`, and custom `zzcustom`. Four simple-suffix cases match correctly. The literal `tar.gz` filter fails: expected `archive.tar.gz`, observed zero files. The parent corrected the original audit, which had silently replaced this request with a `gz` filter before testing it. This uses real `Planner::plan` with explicit `Source::Set` filters, so it checks planner matching independently of parser support. It does not imply the parser recognizes those suffixes.

## Reproduction and test status

From `app/src-tauri`:

```sh
rtk proxy env VOXI_EXTENSION_AUDIT_REPORT=../../docs/testing/extension-audit-results.jsonl cargo test --lib extension_inventory_audit_jsonl -- --ignored --nocapture
rtk proxy env VOXI_EXTENSION_PLANNER_REPORT=../../docs/testing/extension-planner-results.jsonl cargo test --lib extension_audit_planner_suffix_matrix_jsonl -- --ignored --nocapture
rtk cargo test --lib extension_audit_planner_preview_controls -- --ignored --nocapture
rtk cargo test --lib
```

Observed: inventory audit passed as a report-producing test; planner matrix recorded 4 matches and 1 compound-suffix failure; preview controls passed; full `cargo test --lib` passed 152 tests with 5 ignored (three new opt-in audit tests and two pre-existing ignored tests). Two pre-existing dead-code warnings appeared for `matches` and `walk`. No plan was confirmed or executed.

## Parent review notes

Bare unknown words remain ambiguous, so generated cases deliberately use leading-dot syntax. `jpg/jpeg`, `heic/heif`, and `raw` family expectations follow existing parser contracts; extending those to unrelated formats would be speculative. Explicit-dot `.dmg` and `.zip` become broader categories. `.pdf` also uses a category representation but that category accepts only PDF, so it is not an overmatching finding. Parser acceptance and planner matching are counted separately above. Catalog rows are a broad finite inventory, not a claim that every suffix has one universal meaning.

## Parent verification and interpretation

The parent reran all three opt-in audits after correcting the catalog pass counter and the compound-suffix test, checked every catalog suffix against the production parser/model literal tables, and verified the JSONL totals. The final audit contains 347 catalog cases, 1,043 generated/boundary/compound cases, and six phrase controls. All 11 listed compound endings were actually parsed. The ordinary full suite still protects existing behavior; opt-in report tests deliberately record known mismatches rather than turn them into passing feature assertions.

No extension-handling production code was changed by this audit. The next implementation should handle explicit extension syntax generically and preserve literal compound suffixes, while keeping ambiguous bare name groups such as Aeroguard distinct. Repeatedly adding individual suffixes to the small parser table will not cover custom extensions.
