# Haiku paraphrase audit (2026-11-09)

## Parent assessment after review

The raw counts and expectations below are preserved as the audit baseline. Review of the harness, JSONL, and production code qualifies several findings:

- **N021 (`merge PDFs`) is intended default behavior, not a confirmed defect.** `parse/pdf.rs` explicitly supplies `merged.pdf` when no output is named. The phrase contains a PDF source filter, and the normal preview still requires confirmation. Its negative expectation should be revised in a future audit rather than treated as an unsafe invented action.
- **Parser acceptance followed by a planner safety refusal is not an end-to-end safety failure.** Allowed-path checks and the unfiltered clean-up guard deliberately live in the planner. N001, N009–N012, and N033–N036 demonstrate those guards working. Requiring the parser itself to reject them is an architectural preference, not a correctness requirement.
- **P071 remains a meaningful scope error.** Lowercase `documents` is interpreted as a document category instead of the explicit source folder in this phrasing. This can select files outside Documents, even though the particular fixture produced no rename actions.
- **P206 remains a meaningful type/scope error, with a correction to the explanation below.** Screenshot-type searches normally cover every allowed search root, not only the Screenshots folder. This variant instead produces a filename substring filter over the default Downloads/Desktop roots. It can omit screenshots and include other file types with `screenshot` in their names.
- **N013 remains a misleading destination interpretation.** Email delivery is unsupported, yet the phrase produces a preview to create a local `email` folder and move the source into it. Confirmation prevents automatic execution but does not correct the interpretation.
- **The seven `Project` bare-name planner failures deserve follow-up.** Their placement at an extra allowed root exposes a root-name lookup limitation; those rows are parser passes, but dismissing all seven as fixture artifacts overstates planner coverage.

The main actionable clusters are scope/type preservation, destination intent (including email), output-name filler, and natural filename wording. The passing default suite does not mean these audit mismatches have been fixed; the new audit is ignored and report-only. No production changes were made for this audit.

Report-only audit of the English command grammar. It asks whether the compiled parser and planner keep the meaning of supported commands when the user paraphrases them, and whether unsupported or unsafe phrasing is refused. Scope is parser and planner only: no window, no UI, no `confirm_plan`, no execution. The audit does not claim native UI or execution coverage.

- Per-case results: `haiku-paraphrase-audit-2026-11-09.jsonl` (428 rows, every line valid JSON)
- Inputs: `paraphrase-audit-2026-11-09/` (`seeds.jsonl`, `variants.txt`, `contextual.txt`)
- Harness: `app/src-tauri/src/paraphrase_audit.rs` (test-only, all tests `#[ignore]`)

## Counts

| Set | Rows | Meaning |
|---|---|---|
| Seeds | 43 | Supported commands covering all 27 `Intent` variants |
| Paraphrase variants | 302 | Semantic-preserving rewrites, expected = seed meaning |
| Explicit-dot extension variants | 3 | Known-gap candidates (see Duplicates) |
| Negatives | 40 | Unsupported or unsafe phrasing; must not be accepted |
| Ambiguous | 18 | Recorded only; no pass/fail |
| Seed controls | 43 | Each seed run as its own row; checks the seed meaning against the parser |
| Contextual | 22 | Through `plan_input` (scope and follow-ups); result-fixture snapshot |
| **Total** | **428** | |

Variants that are only capitalization or punctuation changes of the seed: 24 of 302 (8%). Axis labels: 110 distinct (filler, verb alias, article, folder preposition and suffix, name/location order, named/called, quoted filenames with keywords, date/size phrasing, spoken numbers and dot, singular/plural, question form, contextual references).

## Results

| Class | Pass | Refused (parser) | Wrong interpretation | Accepted negative | Ambiguous (recorded) | Other |
|---|---|---|---|---|---|---|
| Paraphrase (P) | 271 | 21 | 10 | — | — | — |
| Explicit-dot (K) | 0 | 0 | 3 | — | — | — |
| Seed control (E) | 43 | 0 | 0 | — | — | — |
| Negative (N) | — | — | — | 13 accepted | — | 27 refused (pass) |
| Ambiguous (A) | — | 7 | — | — | 11 accepted | — |
| Contextual (C) | 16 | — | 0 | — | 2 | 4 refused (pass) |

Net: 302 paraphrases, 271 correctly parsed. Of the 271 passes, 43 parsed correctly but the planner returned a problem. Those are fixture artifacts, not parser failures: 25 "no files match" or "nothing to undo", 11 placeholder-byte PDFs ("Couldn't read ... as a PDF"), and 7 bare-name lookups of `Project` that do not search the extra-folder root. Each planner problem is listed in the JSONL `planner` field.

Mechanical severity (from the harness, JSONL `severity`): high 3, medium 22, low 22, info 20. Two rows are escalated in the assessed severities below because their consequence is wider than the fixture showed.

## Failures and risks

### Accepted with the wrong meaning (paraphrase set, 10 rows)

| ID | Phrase (exact) | Expected | Actual | Planner | Mechanical | Assessed |
|---|---|---|---|---|---|---|
| P154 | `merge all PDFs in Downloads into a file called combined.pdf` | output `combined.pdf` | output `a file called combined.pdf` | 1 merge step | high | **high** |
| P071 | `replace draft with final in the names in documents` | folder `documents`, files | no folder; file type `documents` | no-op, "9 names already fine" (4 files in Documents, so scope widened) | medium | **high** (scope) |
| P206 | `delete screenshot files older than 30 days` | type screenshots, 30 days | name contains `screenshot`, no folder | searched Downloads and Desktop, not Screenshots | medium | **high** (scope) |
| P066 | `replace the word draft with final in names in Documents` | find `draft` | find `word draft` | no-op | medium | medium |
| P078 | `replace space characters with dashes in files in Downloads` | find ` ` | find `space characters` | no-op | medium | medium |
| P070 | `replace draft with final in filenames in Documents` | files in Documents | item named `filenames` | not found | medium | medium |
| P088 | `lowercase filenames in Downloads` | files in Downloads | item named `filenames` | not found | medium | medium |
| P168 | `split report.pdf into one page each` | item `report.pdf` | item `report.pdf into 1 page each` | not found | medium | medium |
| P268 | `reveal the file report.pdf in Finder` | target `report.pdf` | target `file report.pdf` | not found | medium | low (read-only) |
| P269 | `show the file report.pdf in Finder` | target `report.pdf` | target `file report.pdf` | not found | medium | low (read-only) |

Two patterns: (a) `filenames` one word is read as an item name, and (b) trailing or inner words (`into a file called`, `into 1 page each`, `word`, `space characters`) are kept as literal arguments instead of being stripped.

### Accepted negatives (13 rows)

| ID | Phrase | What the parser built | Planner | Assessed |
|---|---|---|---|---|
| N013 | `send report.pdf to my email` | move `report.pdf` to folder `email` | **2 steps: create folder `email`, move** | **high** |
| N021 | `merge PDFs` | merge all PDFs in default folders into `merged.pdf` | 1 merge step, invented output name | **high** |
| N001 | `delete everything on my Mac` | trash all files in folder `Mac` (invented) | blocked: clean-up needs a filter | medium |
| N009 | `delete the Documents folder` | trash all files in Documents, no filter | blocked (filter guard) | medium |
| N033 | `clean up Downloads` | trash all files in Downloads, no filter | blocked (filter guard); docs say clean-up needs a filter | medium |
| N034 | `delete all files` | trash all files, no folder | blocked (filter guard) | medium |
| N035 | `trash everything` | same | blocked (filter guard) | medium |
| N036 | `move everything to Trash` | same | blocked (filter guard) | medium |
| N010 | `move ~/Library to Trash` | trash item `~/Library` | blocked: outside Voxi's folders | medium |
| N011 | `delete /System/Library` | trash item `/System/Library` | blocked: outside Voxi's folders | medium |
| N012 | `move ../../secret.txt to Desktop` | move item with `..` | blocked: `..` not allowed | medium |
| N022 | `split pages` | split files with extension `pages` | no match | medium |
| N028 | `open a terminal` | reveal `a terminal` | not found | low |

The planner's filter guard and path checks stopped N001 to N036 from producing changes. The parser should refuse these itself. N013 and N021 are the real concern: the planner produces a changing preview with invented arguments (a new folder, an invented output file). A confirmation step still stands between them and execution, but the preview is misleading.

### Refused paraphrases (21 rows, all safe)

- Verb or synonym not in grammar (12): `put a date prefix on files in Screenshots` (P059), `change draft to final in names in Documents` (P069), `swap spaces for dashes in files in Downloads` (P074), `make file names lowercase in Downloads` (P085), `tidy Downloads by type` (P113), `break report.pdf into pages` (P164), `pull pages 2 to 5 out of report.pdf` (P172), `make one PDF from images in Scans named receipts` (P177), `uncompress archive.zip into Downloads/archive` (P194), `what did you do recently` (P282), `make photo.jpg 1200 pixels wide` (P296), `rename images in Trip as trip-001, trip-002 in order` (P301).
- Compound spoken numbers (4): `one hundred MB` (P020), `one thousand two hundred pixels` (P140), `sixteen hundred pixels` (P146), `five hundred MB` (P229). The docs promise "dictated numbers like thirty" only.
- Phrase-structure gaps (5): `put all PDFs from Downloads in Documents` (P013), `move any file containing invoice` (P024), `move files with names containing invoice` (P027), `add date prefix to file in screenshots` (P062, singular only; the plural form passes), `list duplicate file in Downloads` (P257).

### Known-gap duplicates (3 rows, not new)

| ID | Phrase | Parser output | Duplicate of |
|---|---|---|---|
| K001 | `move all .3mf files from Downloads to Documents` | name contains `.3mf` | Extension audit: explicit-dot suffixes become name filters (`docs/testing/extension-audit-baseline.md`) |
| K002 | `count .3mf files in Documents` | name contains `.3mf` | same |
| K003 | `list .tar.gz files in Downloads` | name contains `.tar.gz` | same, plus the compound-suffix gap |

These confirm the existing gap; I found no new extension behavior.

### Ambiguous rows (recorded, not counted)

- Accepted (11): `erase screenshots older than 30 days` → trash (safe; Voxi never hard-deletes); `delete all PDFs` → trash PDFs across default folders (scope is a judgment call); `revert last` → undo; `save photo.heic as png` → convert (matches docs "writes a PNG copy"); `show files modified last week` → created-date filter (planner uses creation time, not modification); `archive Project` → zip; `remove spaces from names in Downloads` → replace space with empty string; `group screenshots by month` → sort by month; `send notes.txt to Desktop` → **move**, not copy; `count files in Documents` → count all; `list files in Downloads from today` → today.
- Refused (7): `permanently delete screenshots older than 30 days`, `delete all my photos from Pictures without asking`, `make a copy of notes.txt on Desktop`, `list big files in Downloads`, `undo everything`, `organize my Downloads`, `move report.pdf over there`.

### Contextual (through `plan_input`)

- Pass (16): row references (`delete the second file`, `delete 4`, `move 5 to Desktop`, `move the first file`, `delete the last file`, `rename the third file`, `delete four`), pronouns (`delete them`, `move all of them`, `delete everything`, `rename it to final.pdf` with one result), named items (`delete the file named bravo.txt`, `trash the file bravo.txt`, `delete bravo.txt`), `move the second file`, `rename the first file to "last"`.
- Refused, correctly (4): `delete 9` and `delete the 6th file` (out of range); `delete the second file` with no listing (not understood); `move report.pdf to Desktop` (not in the listed results).
- Unsupported, recorded (2): `move the first two files to Desktop`, `delete the first 3 files` (not understood).
- No wrong actions after correction (see objections).

## Objections and expectation changes (disclosed)

1. **C18** (`rename the first file to "last"`): first expectation was `last`. The first run produced `last.zip`; `website/commands.html` says a rename keeps the extension when left off, so I corrected the expectation to `last.zip`. This was changed after seeing output, based on the documented rule.
2. **P021** (`move files larger than 100 mb from downloads to external`): expectation was `External` (copied from the seed); the input is lowercase `external`. The spec allows only root-folder casing to be normalized, so I changed the expectation to the literal `external`. This was my harness error, not a parser error.
3. **P040, P084, P146**: lowercase non-root folder names (`backup`, `trip`) are literal expectations, set before the first run, per the same rule.
4. **A013** (`remove spaces from names in Downloads`): the docs list this as an alias of "replace spaces with dashes", but "remove" reads as deleting the spaces. The parser's empty-string replacement is the natural reading. The docs grouping is ambiguous and worth clarifying.
5. **A015** (`send notes.txt to Desktop`): "send" is ambiguous between copy and move. The parser chooses the more destructive move. I would prefer copy or a confirmation.
6. **N-set boundary**: "delete all PDFs" (A003) is counted as ambiguous, not a negative, because a type filter is present.

## Limitations

- Parser and planner only. No window, no UI, no `confirm_plan`, no real file change. The planner was called for previews; the only `exec` path in `plan_input` (Direct results, e.g. Reveal) was not reached by any contextual command.
- Fixture homes live in temp directories. Spotlight calls are scoped to the fixture's allowed roots (`-onlyin`). The planner's `~` expands to the fixture home. Absolute paths outside the fixture, such as `/System/Library`, are rejected by the allowed-folder check before any filesystem lookup (the planner reported "outside the folders Voxi may change").
- Placeholder bytes are not real PDFs or images. Planner outputs for PDF split/merge and image convert are not meaningful; CoreGraphics logs "PDF error" lines during the run.
- Age and size filters match nothing in the fixture; those planner "no match" outputs are fixture artifacts.
- Expectations are my derivation from the docs and seed meaning. Seed controls pass (43/43), which shows the parser agrees with the seeds, not that the seeds are right. The objections above list where I am least sure.
- Only 3 explicit-dot rows, all in the known gap; this audit does not expand extension coverage.
- Contextual setup ordering depends on file creation times; the fixture sleeps 1.1 s between files.
- No model was called during the audit: the phrases are fixed, written by the test sub-agent, and checked by the compiled parser and planner. No child agents were spawned.

## Reproduction

From `app/src-tauri`:

```sh
rtk proxy env VOXI_PARAPHRASE_AUDIT_REPORT=/Users/vishalshekhar/Documents/dev/Voxi/docs/testing/haiku-paraphrase-audit-2026-11-09.jsonl cargo test --lib paraphrase_audit -- --ignored --nocapture
rtk proxy cargo test --lib
```

Last observed: report test ok (428 rows, 1 passed); full suite **171 passed, 0 failed, 6 ignored** (5 pre-existing, plus this report test).

## Artifacts

- New: `docs/testing/haiku-paraphrase-audit-2026-11-09.md`, `docs/testing/haiku-paraphrase-audit-2026-11-09.jsonl`
- New: `docs/testing/paraphrase-audit-2026-11-09/{seeds.jsonl,variants.txt,contextual.txt}`
- New: `app/src-tauri/src/paraphrase_audit.rs`
- Modified: `app/src-tauri/src/lib.rs` (two lines: `#[cfg(test)] mod paraphrase_audit;`), on top of existing uncommitted changes
- Unchanged: `docs/testing/haiku-300-*`, `docs/testing/extension-audit-*`, parser and planner production code

No commit, push, or production change was made.
