# Haiku 300: fixes and final results

All 300 phrases pass after auditing four expectations. All original phrases and their order are retained. The original Markdown baseline and JSONL baseline are preserved for comparison.

## Results

| Check | Baseline | Final |
|---|---:|---:|
| Haiku parser phrases | 254 / 300 | 300 / 300 |
| Original corpus phrases | 328 / 328 | 328 / 328 |
| Haiku planner / parse-before-plan fixtures | 12 / 15 | 19 / 19 |
| Full backend library suite | 137 passed, 4 failed, 2 ignored | 150 passed, 0 failed, 2 ignored |

The ignored per-case report test was also run explicitly and passed. The other ignored test is the unrelated held-out evaluation, which was not used to choose expectations. The normal corpus test executes all 628 regression phrases on every library test run.

Of the original 46 mismatches, **43 were fixed without changing their expected outcomes**. Three were expectation errors (H225, H227, H238). One originally passing phrase, H040, was changed to refuse an operation-specific undo for the same reason H239 was already expected to refuse. The resulting corpus contains 152 intent expectations and 148 refusals. Parser outputs changed for 44 of the 300 phrases.

## What changed

- Unresolved ordinal and conversational references stop at parsing, before filesystem traversal. This also applies to opening files and size queries. Explicit names and quoted literals still work.
- Missing sources and empty quoted names no longer expand into file groups or broad searches. An explicit quantifier such as every file remains supported, including file-name batch commands.
- Move, copy, and rename split on exactly one unquoted separator. Ambiguous bare names are refused; quoted names such as Notes to Self.txt preserve their full spelling.
- Quoted words are protected from number normalization, date extraction, and command delimiters. Scoped quoted filenames and folders such as last week have fixture coverage.
- Repeated date, age, and size bounds cannot overwrite earlier constraints. Contradictory ranges, empty date windows, unreadable dates, zero-day lookbacks, and missing size units are refused.
- Incomplete destinations and unreadable filters no longer fall through to guessed filenames or folder names. Direct empty-item resolution also stops before searching.
- Undo still targets the newest change. Operation-specific requests such as undo the previous move or undo the last delete are refused because Voxi cannot select that operation from history.

## Expectation audit

These are the only expectation changes. No phrases were dropped, replaced, or reordered. The original expected and observed values remain in haiku-300-baseline.jsonl.

| ID | Phrase | Change | Reason |
|---|---|---|---|
| H040 | `undo the last delete` | Intent → refusal | A named operation (last delete) cannot safely select the newest change when that change may be a move or rename. It now refuses consistently with H239 (previous move). Use undo last or undo the last change. This is a deliberate safety behavior change, not an original failing row. |
| H225 | `erase everything in Documents` | Refusal → trash intent; planner still refuses | The established parser accepts everything as a group; the planner owns the unfiltered trash guard. Existing corpus 4 already specifies this separation. A new Documents fixture verifies this exact phrase cannot produce a change plan. |
| H227 | `compress Project into Project.zip` | Refusal → ZIP intent | compress is an existing alias in parse/archives.rs. No behavior change was necessary. |
| H238 | `sort the downloads by date` | Refusal → sort-by-month intent | Corpus 7 already explicitly expects sort my Desktop by date to group by month. The new expectation is aligned with that existing contract. No behavior change was necessary. |

## Categories

| Category | Cases | Baseline passes | Final passes |
|---|---:|---:|---:|
| [A] | 20 | 20 | 20 |
| [B] | 20 | 20 | 20 |
| [C] | 25 | 25 | 25 |
| [D] | 12 | 12 | 12 |
| [E] | 15 | 15 | 15 |
| [F] | 15 | 15 | 15 |
| [G] | 15 | 15 | 15 |
| [H] | 15 | 15 | 15 |
| [I] | 13 | 13 | 13 |
| [R1] | 30 | 21 | 30 |
| [R2] | 20 | 13 | 20 |
| [R3] | 20 | 20 | 20 |
| [R4] | 25 | 19 | 25 |
| [R5] | 20 | 6 | 20 |
| [R6] | 20 | 15 | 20 |
| [R7] | 15 | 10 | 15 |

## Originally failing rows

| ID | Phrase | Resolution |
|---|---|---|
| H155 | `delete the first file in Downloads` | Parser fixed; original expectation retained |
| H156 | `move the second file in Downloads to Archive` | Parser fixed; original expectation retained |
| H165 | `delete the one from yesterday` | Parser fixed; original expectation retained |
| H168 | `delete the file from before` | Parser fixed; original expectation retained |
| H169 | `delete the thing I downloaded yesterday` | Parser fixed; original expectation retained |
| H170 | `move the one I took yesterday to Archive` | Parser fixed; original expectation retained |
| H178 | `delete the file you were looking at` | Parser fixed; original expectation retained |
| H179 | `move the file we talked about to Archive` | Parser fixed; original expectation retained |
| H180 | `delete the thing on my screen` | Parser fixed; original expectation retained |
| H185 | `move the file to Documents` | Parser fixed; original expectation retained |
| H191 | `copy notes.txt to the` | Parser fixed; original expectation retained |
| H192 | `delete from Downloads` | Parser fixed; original expectation retained |
| H193 | `move from Downloads to Documents` | Parser fixed; original expectation retained |
| H194 | `list in` | Parser fixed; original expectation retained |
| H196 | `show size` | Parser fixed; original expectation retained |
| H199 | `move "" to Documents` | Parser fixed; original expectation retained |
| H225 | `erase everything in Documents` | Expectation corrected; see audit |
| H227 | `compress Project into Project.zip` | Expectation corrected; see audit |
| H236 | `clean up my desktop mess` | Parser fixed; original expectation retained |
| H238 | `sort the downloads by date` | Expectation corrected; see audit |
| H239 | `undo the previous move` | Parser fixed; original expectation retained |
| H242 | `what's the size of the largest PDF` | Parser fixed; original expectation retained |
| H246 | `list files larger than 10 MB and smaller than 5 MB in Downloads` | Parser fixed; original expectation retained |
| H247 | `list files larger than 10 MB smaller than 5 MB in Downloads` | Parser fixed; original expectation retained |
| H249 | `list files older than 30 days from today` | Parser fixed; original expectation retained |
| H251 | `list files from the last 2 days or last month` | Parser fixed; original expectation retained |
| H255 | `list screenshots taken in month 13` | Parser fixed; original expectation retained |
| H256 | `delete files larger than 1 GB but under 500 MB in Downloads` | Parser fixed; original expectation retained |
| H257 | `move PDFs created after yesterday before today to Archive` | Parser fixed; original expectation retained |
| H258 | `delete screenshots from today older than a week` | Parser fixed; original expectation retained |
| H259 | `list files newer than 2 days` | Parser fixed; original expectation retained |
| H260 | `list files in the last 0 days` | Parser fixed; original expectation retained |
| H261 | `list files from March 2026 before March 2025` | Parser fixed; original expectation retained |
| H262 | `list files bigger than 5 in Downloads` | Parser fixed; original expectation retained |
| H263 | `list files created in 1850 in Downloads` | Parser fixed; original expectation retained |
| H264 | `delete files larger than 5 GB but smaller than 1 GB` | Parser fixed; original expectation retained |
| H266 | `move Notes to Self.txt to Documents` | Parser fixed; original expectation retained |
| H267 | `move Plan to Phase 2.pdf to Archive` | Parser fixed; original expectation retained |
| H268 | `copy Trip to Lisbon.jpg to Pictures` | Parser fixed; original expectation retained |
| H269 | `move the file Ideas to Drafts.md to Archive` | Parser fixed; original expectation retained |
| H270 | `rename Notes to Self.txt to Journal.txt` | Parser fixed; original expectation retained |
| H288 | `list files from the year 3000` | Parser fixed; original expectation retained |
| H289 | `delete files older than -5 days` | Parser fixed; original expectation retained |
| H290 | `list files from the 1st week of 2026` | Parser fixed; original expectation retained |
| H291 | `list screenshots from last fortnight` | Parser fixed; original expectation retained |
| H292 | `move files older than 2 hours to Archive` | Parser fixed; original expectation retained |

## Reproduce

Run from the repository root:

```sh
cd app/src-tauri
rtk cargo test --lib
rtk cargo test --lib haiku_
rtk proxy env VOXI_CASE_REPORT=../../docs/testing/haiku-300-final.jsonl cargo test --lib haiku_300_per_case_report -- --ignored --nocapture
```

`haiku_300_per_case_report` writes actual outputs from the production parser, then asserts all 300 audited expectations match. The normal suite also asserts exactly 300 distinct Haiku phrases.

## Scope and limits

Every phrase was tested through the actual parser used by Enter. Nineteen fixture tests additionally cover planning, parse-before-plan refusals, recursive scope, exact/typo ambiguity, quoted names, size filters, unfiltered trash protection, and read-only behavior. These do not amount to 300 GUI or execution tests. Actions were never executed by these fixtures; all fixture files remain in temporary homes. Spotlight may be queried for fallback resolution, with results filtered to fixture roots. Age/date contradictions were checked at parsing with a pinned date (2026-10-28); actual birth-time-based selection was not newly fixture-tested.

## Local application checks

The macOS debug app built successfully, was signed locally, and was relaunched. Five representative refusals were submitted through its actual command bar: H155 (first file), H185 (missing source), H266 (ambiguous filename separator), H256 (contradictory sizes), and H251 (repeated lookback filters). All five showed the refusal and filename guidance, with no change preview. No actions were confirmed or executed. This is a smoke check, not native UI coverage of all 300 phrases.

The per-case native results are saved in [haiku-300-native-smoke.json](haiku-300-native-smoke.json).

## Artifacts

- [Original baseline](haiku-300-baseline.md)
- [Original per-case JSONL](haiku-300-baseline.jsonl)
- [Final per-case JSONL](haiku-300-final.jsonl)
- [300 regression phrases](../../app/src-tauri/src/parse/corpus/10-haiku-300.txt)

Original baseline JSONL SHA-256: `6876f5b12e3ec49b1c2f2cf09c2903a6a005f3a2196fbf120df82b46b3777546`.
