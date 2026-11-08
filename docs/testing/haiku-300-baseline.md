# Haiku 300 baseline: Voxi parser and planner

Generated from the real Rust parser run on `app/src-tauri/src/parse/corpus/10-haiku-300.txt`. Expectations were written before the first run and were **not** changed afterwards. Questionable expectations are listed separately below.

## Summary

- Corpus: `app/src-tauri/src/parse/corpus/10-haiku-300.txt`, 300 rows. 150 expect an intent, 150 expect a refusal.
- Parser result: **254 pass, 46 fail** (15.3% mismatches).
- Planner fixtures (temporary homes, `plan.rs` tests named `haiku_*`): 12 pass, 3 fail by design. The failures are the three planner-level findings below.
- Existing corpus files (`1-…` to `9-…`) were not modified by this work, and all of their rows still pass. All 46 failing rows come from file 10.

## How to run

```sh
cd app/src-tauri
# Regression runner (all corpus files, including file 10). Fails with the 46 known rows.
cargo test --lib corpus_commands_build_the_intended_intent
# Per-case report for file 10 only; writes JSON lines (never fails).
cargo test --lib haiku_300_per_case_report -- --ignored --nocapture
# Optional: choose the output file
VOXI_CASE_REPORT=/tmp/haiku.jsonl cargo test --lib haiku_300_per_case_report -- --ignored --nocapture
# Planner fixtures (3 fail by design)
cargo test --lib haiku_
```

Each JSON line in `docs/testing/haiku-300-baseline.jsonl` has `id` (H001 to H300, in file order), `line`, `category`, `command`, `expected`, `actual`, `pass`. Re-run the report after a fix and diff the files to see which rows changed.

## What was tested, and how

- **Parser-tested (300 of 300):** each phrase went through `parse::parse()` exactly as Enter does (`corpus_tests.rs`). This is text in, intent JSON or refusal out. It is not a GUI test, and there was no microphone or dictation input.
- **Planner-tested (class fixtures only, not the 300 phrases):** `plan.rs` has 15 `haiku_*` tests on temporary homes. Some use phrases that also appear in the corpus; the rest use equivalent phrasing. Each test only calls `Planner::plan`. Nothing runs an action; the tests check that trash never hard-deletes and that planning leaves files in place.
- **Not tested:** the native app window, Enter, Esc, the preview UI, real Trash moves, undo, and any real user folders. Spotlight is queried by the planner for name fallbacks, and results are filtered to the temp home.

## Categories

| Category | IDs | Rows | Pass | Fail | Expected |
|---|---|---:|---:|---:|---|
| [A] Everyday moves, copies, renames, and deletes of one file or a whole set (supported) H001-H020 | H001-H020 | 20 | 20 | 0 | intent |
| [B] Read-only questions, undo, history, and reveal in Finder (supported) H021-H040 | H021-H040 | 20 | 20 | 0 | intent |
| [C] Dates, ages, months, and day words, against the fixed day 2026-10-28 (supported) H041-H065 | H041-H065 | 25 | 25 | 0 | intent |
| [D] Size filters in MB, GB, and spoken units (supported) H066-H077 | H066-H077 | 12 | 12 | 0 | intent |
| [E] File-type words, extension families, and singular or plural subjects (supported) H078-H092 | H078-H092 | 15 | 15 | 0 | intent |
| [F] Quoted and exact names, last.pdf and "last", curly quotes, and spoken dots (supported) H093-H107 | H093-H107 | 15 | 15 | 0 | intent |
| [G] Folders, subfolders, concrete paths, and "folder" as a spoken word (supported) H108-H122 | H108-H122 | 15 | 15 | 0 | intent |
| [H] Batch renames, numbering, prefixes, spoken replacements, and PDF or image tools (supported) H123-H137 | H123-H137 | 15 | 15 | 0 | intent |
| [I] Zip, folders, sorting, conversion, quoted renames, copies, and a spoken number (supported) H138-H150 | H138-H150 | 13 | 13 | 0 | intent |
| [R1] Relative, ordinal, and pronoun references with no resolved file (refused) H151-H180 | H151-H180 | 30 | 21 | 9 | refusal |
| [R2] Missing source, destination, or new name (refused) H181-H200 | H181-H200 | 20 | 13 | 7 | refusal |
| [R3] Several targets, joined lists, exclusions, and exceptions (refused) H201-H220 | H201-H220 | 20 | 20 | 0 | refusal |
| [R4] Permanent deletes, sharing, unsupported verbs, superlatives, and unsupported undo (refused) H221-H245 | H221-H245 | 25 | 19 | 6 | refusal |
| [R5] Contradictory, invalid, or unreadable dates, ages, and sizes (refused) H246-H265 | H246-H265 | 20 | 6 | 14 | refusal |
| [R6] Names that contain joining words, command words, filter words, or wildcards (refused) H266-H285 | H266-H285 | 20 | 15 | 5 | refusal |
| [R7] Spoken time words, invalid years, and vague periods (refused) H286-H300 | H286-H300 | 15 | 10 | 5 | refusal |

## Mismatches (46)

Severity is about what could happen if the intent reached the planner and was confirmed. It is not a count of how often it happens. Input text is the exact command. Expected and actual are the JSON the corpus runner compared.

| ID | Line | Input | Expected | Actual | Severity | Why |
|---|---:|---|---|---|---|---|
| H155 | 183 | `delete the first file in Downloads` | `!` | `{"intent":"trash","src":{"folder":"Downloads","kind":"item","name":"first"}}` | High | Ordinal "first" is read as an item name. Planner fixture did not plan a change here (ambiguous), but the intent is wrong and relies on a fuzzy match. |
| H156 | 184 | `move the second file in Downloads to Archive` | `!` | `{"intent":"move","src":{"folder":"Downloads","kind":"item","name":"second"},"to":"Archive"}` | High | Ordinal "second" becomes item "second". Planner fixture CONFIRMED: plans Move of Downloads/second.pdf (plus CreateFolder Desktop/Archive) with no choice requested. |
| H165 | 193 | `delete the one from yesterday` | `!` | `{"intent":"trash","src":{"filter":{"day":"yesterday","type":"files"},"folder":"1","kind":"set"}}` | Medium | "one" becomes the folder "1" (normalize turns number words into digits). Planner would fail to find folder "1" unless one exists, but the day filter is still applied to a guessed folder. |
| H168 | 196 | `delete the file from before` | `!` | `{"intent":"trash","src":{"filter":{"type":"files"},"folder":"before","kind":"set"}}` | Medium | "from before" is read as folder "before". Unfiltered type, so the planner's clean-up guard refuses it today; the parser still builds a destructive intent. |
| H169 | 197 | `delete the thing I downloaded yesterday` | `!` | `{"intent":"trash","src":{"filter":{"day":"yesterday","type":"files"},"folder":"thing","kind":"set"}}` | Medium | "I downloaded" is dropped, then "thing" becomes folder "thing". If a folder named thing exists under an allowed root, yesterday's files in it would be trashed. |
| H170 | 198 | `move the one I took yesterday to Archive` | `!` | `{"intent":"move","src":{"filter":{"day":"yesterday","type":"files"},"folder":"1","kind":"set"},"to":"Archive"}` | Medium | "one" becomes folder "1" with a day filter. Same class as H165, for a move. |
| H178 | 206 | `delete the file you were looking at` | `!` | `{"intent":"trash","src":{"kind":"item","name":"you were looking at"}}` | Medium | Conversational reference "you were looking at" becomes an item name. Planner resolves by fuzzy name; no refusal at parse time. |
| H179 | 207 | `move the file we talked about to Archive` | `!` | `{"intent":"move","src":{"kind":"item","name":"we talked about"},"to":"Archive"}` | Medium | Conversational reference "we talked about" becomes an item name, then a move. Same class as H178. |
| H180 | 208 | `delete the thing on my screen` | `!` | `{"intent":"trash","src":{"folder":"screen","kind":"item","name":"thing"}}` | Medium | "the thing" becomes item "thing" in folder "screen". A wrong folder or fuzzy name could move a different file. |
| H185 | 215 | `move the file to Documents` | `!` | `{"intent":"move","src":{"filter":{"type":"files"},"kind":"set"},"to":"Documents"}` | High | "the file" with no name parses as Set of ALL files (type words: "file" maps to Files). Planner fixture CONFIRMED: plans a move of files in Downloads and Desktop with no refusal. |
| H191 | 221 | `copy notes.txt to the` | `!` | `{"intent":"copy","src":{"kind":"item","name":"notes.txt"},"to":"the"}` | Medium | Destination "the" (from "to the" with no name after) is accepted; planner would create a folder called "the" under Desktop. |
| H192 | 222 | `delete from Downloads` | `!` | `{"intent":"trash","src":{"filter":{"type":"files"},"folder":"Downloads","kind":"set"}}` | Medium | "delete from Downloads" parses as trash of every file in Downloads. The planner refuses unfiltered clean-up (plan.rs To be safe guard), so this is safe only because of that backstop. |
| H193 | 223 | `move from Downloads to Documents` | `!` | `{"intent":"move","src":{"filter":{"type":"files"},"folder":"Downloads","kind":"set"},"to":"Documents"}` | High | "move from Downloads" with no subject parses as move of ALL files in Downloads. Planner fixture CONFIRMED: returns a plan rather than a refusal. |
| H194 | 224 | `list in` | `!` | `{"intent":"list","src":{"kind":"item","name":"in"}}` | Low | Read-only. "list in" becomes a list of item "in"; planner finds nothing. |
| H196 | 226 | `show size` | `!` | `{"intent":"list","src":{"kind":"item","name":"size"}}` | Low | Read-only. "show size" (no "of") becomes a list of item "size". |
| H199 | 229 | `move "" to Documents` | `!` | `{"intent":"move","src":{"kind":"item","name":""},"to":"Documents"}` | Medium | Empty quoted name "" becomes item "" (phrase.rs:440 accepts any quoted text). Planner was not tested with an empty name. |
| H225 | 259 | `erase everything in Documents` | `!` | `{"intent":"trash","src":{"filter":{"type":"files"},"folder":"Documents","kind":"set"}}` | Medium | "erase everything in Documents" (erase = trash) builds an unfiltered trash. The planner refuses it (tested for Downloads), so the parser is the only gap. Expectation questionable: see flagged list. |
| H227 | 261 | `compress Project into Project.zip` | `!` | `{"intent":"zip","output":"Project.zip","src":{"kind":"item","name":"Project"}}` | Low | "compress" is a documented-looking synonym of zip in parse/archives.rs:13. Expectation questionable: see flagged list. |
| H236 | 270 | `clean up my desktop mess` | `!` | `{"intent":"trash","src":{"kind":"item","name":"desktop mess"}}` | Medium | "clean up my desktop mess" becomes trash of item "desktop mess". Unfiltered item trash of a name that does not exist; fuzzy resolution could pick a file. |
| H238 | 272 | `sort the downloads by date` | `!` | `{"folder":"downloads","intent":"sort_by_month"}` | Medium | "by date" is silently treated as "by month", so Downloads is reorganized into month folders. Creates folders and moves files without the user asking for that sort. |
| H239 | 273 | `undo the previous move` | `!` | `{"intent":"undo"}` | Medium | "undo the previous move" undoes the newest change. If the user made two changes, the wrong one is reversed. Expectation questionable: see flagged list. |
| H242 | 276 | `what's the size of the largest PDF` | `!` | `{"folder":"largest PDF","intent":"size"}` | Low | Read-only. The size_of pattern takes "largest PDF" as a folder name. |
| H246 | 282 | `list files larger than 10 MB and smaller than 5 MB in Downloads` | `!` | `{"intent":"list","src":{"filter":{"largerThanMb":10.0,"smallerThanMb":5.0,"type":"files"},"folder":"Downloads","kind":"set"}}` | Medium | Contradictory size range (larger than 10 MB and smaller than 5 MB) is accepted with no refusal. Read-only here, but the same filter would reach destructive commands. |
| H247 | 283 | `list files larger than 10 MB smaller than 5 MB in Downloads` | `!` | `{"intent":"list","src":{"filter":{"largerThanMb":10.0,"smallerThanMb":5.0,"type":"files"},"folder":"Downloads","kind":"set"}}` | Medium | Same contradictory size range as H246, without the "and". |
| H249 | 285 | `list files older than 30 days from today` | `!` | `{"intent":"list","src":{"filter":{"day":"today","olderThanDays":30,"type":"files"},"kind":"set"}}` | Low | Contradictory day and age filters (today and older than 30 days) are accepted. Empty result, but unrefused. |
| H251 | 287 | `list files from the last 2 days or last month` | `!` | `{"intent":"list","src":{"filter":{"newerThanDays":30,"type":"files"},"kind":"set"}}` | Medium | Two "last N" phrases joined by "or": the parser keeps only the last one (phrase.rs:324-327 overwrites on each match) and silently drops "last 2 days". |
| H255 | 291 | `list screenshots taken in month 13` | `!` | `{"intent":"list","src":{"filter":{"type":"screenshots"},"folder":"month 13","kind":"set"}}` | Low | "in month 13" becomes folder "month 13". Read-only, not found. |
| H256 | 292 | `delete files larger than 1 GB but under 500 MB in Downloads` | `!` | `{"intent":"trash","src":{"filter":{"largerThanMb":1024.0,"smallerThanMb":500.0,"type":"files"},"folder":"Downloads","kind":"set"}}` | Medium | Destructive command with an empty size window (larger than 1 GB and under 500 MB). No contradiction check (phrase.rs extract_filters). |
| H257 | 293 | `move PDFs created after yesterday before today to Archive` | `!` | `{"intent":"move","src":{"filter":{"before":"2026-10-28","since":"2026-10-28","type":"pdfs"},"kind":"set"},"to":"Archive"}` | Low | Empty window (since and before both yesterday to today). Builds a valid intent that matches nothing. Expectation questionable: see flagged list. |
| H258 | 294 | `delete screenshots from today older than a week` | `!` | `{"intent":"trash","src":{"filter":{"day":"today","olderThanDays":7,"type":"screenshots"},"kind":"set"}}` | Medium | Destructive command with a contradictory day and age (today and older than a week). No refusal, so it is a silent no-op or, if the filter logic changes, a wrong trash. |
| H259 | 295 | `list files newer than 2 days` | `!` | `{"intent":"list","src":{"kind":"item","name":"files newer than 2 days"}}` | Low | "newer than" is not a supported phrase, so it becomes item "files newer than 2 days". Read-only, not found. |
| H260 | 296 | `list files in the last 0 days` | `!` | `{"intent":"list","src":{"filter":{"newerThanDays":0,"type":"files"},"kind":"set"}}` | Low | "last 0 days" builds newerThanDays 0. Expectation questionable: see flagged list. |
| H261 | 297 | `list files from March 2026 before March 2025` | `!` | `{"intent":"list","src":{"filter":{"before":"2025-03-01","month":3,"type":"files","year":2026},"kind":"set"}}` | Low | Contradictory month/year and before filter (March 2026 before March 2025). Read-only. No contradiction check. |
| H262 | 298 | `list files bigger than 5 in Downloads` | `!` | `{"intent":"list","src":{"filter":{"largerThanMb":5.0,"type":"files"},"folder":"Downloads","kind":"set"}}` | Medium | A size with no unit ("bigger than 5") silently means 5 MB (SIZE_LARGER makes the unit optional). Expectation questionable: see flagged list. |
| H263 | 299 | `list files created in 1850 in Downloads` | `!` | `{"intent":"list","src":{"filter":{"type":"files"},"folder":"1850 in Downloads","kind":"set"}}` | Low | "in 1850" is not a valid year, so the folder becomes "1850 in Downloads". Read-only, not found. |
| H264 | 300 | `delete files larger than 5 GB but smaller than 1 GB` | `!` | `{"intent":"trash","src":{"filter":{"largerThanMb":5120.0,"smallerThanMb":1024.0,"type":"files"},"kind":"set"}}` | Medium | Destructive command with an empty size window (larger than 5 GB but smaller than 1 GB). No contradiction check. |
| H266 | 304 | `move Notes to Self.txt to Documents` | `!` | `{"intent":"move","src":{"kind":"item","name":"Notes"},"to":"Self.txt to Documents"}` | High | " to " inside a file name splits the move (move pattern is case-insensitive, files.rs:15). Item "Notes" would be moved to a folder named "Self.txt to Documents" (created). |
| H267 | 305 | `move Plan to Phase 2.pdf to Archive` | `!` | `{"intent":"move","src":{"kind":"item","name":"Plan"},"to":"Phase 2.pdf to Archive"}` | High | Same split: moves item "Plan" to a folder named "Phase 2.pdf to Archive". |
| H268 | 306 | `copy Trip to Lisbon.jpg to Pictures` | `!` | `{"intent":"copy","src":{"kind":"item","name":"Trip"},"to":"Lisbon.jpg to Pictures"}` | Medium | Same split for copy (files.rs:8): copies item "Trip" to a folder named "Lisbon.jpg to Pictures". |
| H269 | 307 | `move the file Ideas to Drafts.md to Archive` | `!` | `{"intent":"move","src":{"kind":"item","name":"Ideas"},"to":"Drafts.md to Archive"}` | High | Same split: moves item "Ideas" to a folder named "Drafts.md to Archive". |
| H270 | 308 | `rename Notes to Self.txt to Journal.txt` | `!` | `{"intent":"rename","new_name":"Self.txt to Journal.txt","src":{"kind":"item","name":"Notes"}}` | High | Same split for rename (rename.rs:76): renames item "Notes" to "Self.txt to Journal.txt". |
| H288 | 328 | `list files from the year 3000` | `!` | `{"intent":"list","src":{"filter":{"type":"files"},"folder":"year 3000","kind":"set"}}` | Low | "the year 3000" becomes folder "year 3000". Read-only. |
| H289 | 329 | `delete files older than -5 days` | `!` | `{"intent":"trash","src":{"kind":"item","name":"files older than -5 days"}}` | Medium | Destructive command with an unrecognized age unit ("-5 days"). Parses as trash of item "files older than -5 days"; not found, but fuzzy resolution could pick a file. |
| H290 | 330 | `list files from the 1st week of 2026` | `!` | `{"intent":"list","src":{"filter":{"type":"files"},"folder":"1st week of 2026","kind":"set"}}` | Low | "the 1st week of 2026" becomes folder "1st week of 2026". Read-only. |
| H291 | 331 | `list screenshots from last fortnight` | `!` | `{"intent":"list","src":{"filter":{"type":"screenshots"},"folder":"last fortnight","kind":"set"}}` | Low | "last fortnight" is unsupported; it becomes folder "last fortnight". Read-only. |
| H292 | 332 | `move files older than 2 hours to Archive` | `!` | `{"intent":"move","src":{"kind":"item","name":"files older than 2 hours"},"to":"Archive"}` | Medium | "older than 2 hours" (unsupported unit) becomes a move of item "files older than 2 hours". Not found, but fuzzy resolution could pick a file. |

## Concrete risks, most severe first

1. **Ordinals and pronouns become file names (H155, H156, H178, H179, H180).** Code: `parse/phrase.rs:534-538` makes any unrecognized subject an `Item` through `clean_item`. The only guard is the `vague()` whitelist (`phrase.rs:553-573`), which covers `last`, `previous`, `newest`, `biggest`, `recent`, and so on, but not `first`, `second`, `thing`, `you were looking at`, or `we talked about`.
   - **Planner repro, confirmed:** with `Downloads/second.pdf` present, `move the second file in Downloads to Archive` plans `Move Downloads/second.pdf` and `CreateFolder Desktop/Archive`. No choice is requested, although the README says loose or word matches must always ask the user (README.md, "Individual filename searches"). The fuzzy word match is accepted at `plan.rs` `resolve_item` (`found.len()==1 && strong_name_match`).
2. **Missing subject becomes a whole-folder or whole-folder-type action (H185, H193, H192, H168).** `phrase.rs:379` maps `file` and `files` to `Files`, so "the file" becomes all files. `phrase.rs:496` only refuses an empty subject when no folder is given, so "from Downloads" becomes every file in Downloads.
   - **Planner repro, confirmed:** `move the file to Documents` and `move from Downloads to Documents` both return a plan, not a refusal. Only clean-up (trash) has a planner guard (`plan.rs:757`, "To be safe, clean-up needs a filter"). Move and copy have no such guard.
   - Trash without a filter is refused by the planner. The parser still builds the intent (H192, H225). Keep the planner guard, and consider also refusing at parse time.
3. **" to " inside a file name splits the command (H266, H267, H268, H269, H270).** The move, copy, and rename patterns are compiled case-insensitively (`parse/mod.rs` `pat`), and their lazy `(.+?)\s+(?:to|into)` takes the first " to ". The destination then becomes `Self.txt to Documents`, which `bad_destination` (`phrase.rs:624-627`) does not reject. The resulting plan moves or renames the wrong item. Fix: refuse when the destination contains " to ", or prefer quoted names.
4. **Contradictory filters are accepted (H246, H247, H256, H258, H264, H249, H261).** `phrase.rs:310-365` (`extract_filters`) sets each filter with no consistency check. A destructive command with an empty window should match nothing. The planner was not run on these rows, so that is unverified. A later filter change could make it match the wrong files.
5. **Repeated filters silently drop one value (H251).** The `take` closures for `NEWER`, `OLDER`, and the size regexes overwrite the value on each match (`phrase.rs:316-327`, `357-364`). "last 2 days or last month" keeps only the month.
6. **Silent unit defaults (H262).** `SIZE_LARGER` and `SIZE_SMALLER` make the unit optional (`phrase.rs:105-115`), so "bigger than 5" means 5 MB without saying so.
7. **Unrecognized date and folder phrases become folder names (H165, H170, H255, H263, H288, H290, H291, H168, H169, H180).** The `FOLDER` regex (`phrase.rs:172`) splits at the first `from|in|on|inside|within`, so "in month 13", "in 1850", "from before", and "from the year 3000" are read as folder names. Number words are also converted globally (`phrase.rs:45`), so "one" becomes folder "1".
8. **Unsupported units and phrasing fall through to item names (H259, H289, H292, H236).** "hours", "fortnight", "newer than", and "clean up … mess" have no pattern, so they become file names. Planner resolution then fuzzy-matches them.
9. **Empty quoted name (H199).** `phrase.rs:440-442` returns `Item ""` for `""`. Refuse empty names at parse time.
10. **Compress and sort-by-date are guesses (H227, H238).** `parse/archives.rs:13` maps "compress" to zip, and "by date" becomes month sorting. Both are plausible, but each changes what the user asked for without saying so.

## Planner fixture results (`plan.rs`, `haiku_*`)

| Test | Result | Class |
|---|---|---|
| `haiku_explicit_filename_touches_only_the_named_file` | pass | Explicit filename targets only the named file (also `delete "last"`) |
| `haiku_type_group_stays_in_its_folder` | pass | Scoped type group stays in its folder |
| `haiku_group_includes_subfolders_but_skips_hidden_ones` | pass | Recursion included, hidden folders skipped |
| `haiku_name_group_does_not_reach_other_folders` | pass | Name filter does not broaden to other folders |
| `haiku_size_filter_selects_only_matching_files` | pass | Size filter selects only large files |
| `haiku_same_name_in_two_folders_needs_a_choice` | pass | Same name in two folders requires selection |
| `haiku_typo_never_picks_a_file_silently` | pass | Typo does not silently select a file |
| `haiku_paths_outside_voxi_folders_are_refused` | pass | Paths outside allowed folders and `..` refused |
| `haiku_unfiltered_clean_up_is_refused_by_the_planner` | pass | Unfiltered trash refused ("To be safe") |
| `haiku_delete_plans_a_trash_move_and_touches_nothing` | pass | Delete plans Trash only; planning moves nothing |
| `haiku_unresolved_references_never_reach_the_planner` | pass | Unresolved references fail at parse, before planning |
| `haiku_listing_is_read_only` | pass | List is read-only |
| `haiku_ordinal_references_do_not_plan_changes` | FAIL | H156 plans a move of a fuzzy match, no choice asked (confirmed) |
| `haiku_unscoped_file_move_is_refused_by_the_planner` | FAIL | H185: "move the file to Documents" returns a plan (confirmed) |
| `haiku_move_from_folder_without_a_subject_is_refused` | FAIL | H193: "move from Downloads to Documents" returns a plan (confirmed) |

The three FAIL rows are left in place on purpose, for the parent to fix. Planner tests do not change expectations.

## Questionable expectations (flagged, not changed)

These are rows where the expectation is a judgment call, or where a reasonable reading differs. Each row keeps its original expectation. Decide on each one before changing the expectation.

| ID | Input | Expectation | Question | Status |
|---|---|---|---|---|
| H227 | `compress Project into Project.zip` | `!` | compress is a reasonable synonym for zip; "!" may be too strict. The parser maps it to zip today, so this is a product call. | FAIL |
| H239 | `undo the previous move` | `!` | The documented rule is "undo last". "undo the previous move" may be a supported synonym of undo, or ambiguous. The expectation here is a refusal. | FAIL |
| H225 | `erase everything in Documents` | `!` | Whether the parser or the planner owns the bulk clean-up guard is open. The planner refuses today (tested for Downloads). The expectation is a parser refusal. | FAIL |
| H257 | `move PDFs created after yesterday before today to Archive` | `!` | since and before are both set to today, so the window is empty. The expectation calls it contradictory. It may instead be valid and empty. | FAIL |
| H260 | `list files in the last 0 days` | `!` | "last 0 days" may be an invalid count (refuse) or a valid zero-width window. The expectation is a refusal. | FAIL |
| H262 | `list files bigger than 5 in Downloads` | `!` | Corpus has no precedent for a unit-less size. The expectation is a refusal. MB as a default would also be defensible. | FAIL |
| H261 | `list files from March 2026 before March 2025` | `!` | The two dates conflict, so the expectation is a refusal. The intent may also be read as "before the March 2025 window" with a valid empty result. | FAIL |
| H150 | `move Chapter one to Archive` | `{"intent":"move","src":{"kind":"item","name":"Chapter 1"},"to":"Archive"}` | Spoken number "one" becomes "1" in the item name, so "Chapter one" files are only found if the file is named "Chapter 1". Passes, but the expectation is a judgment. | PASS |

## Per-case results (all 300)

Machine-readable: `docs/testing/haiku-300-baseline.jsonl`. This table repeats the status and input for each row.

| ID | Line | Result | Category | Input |
|---|---:|---|---|---|
| H001 | 11 | pass | A | `please move the invoice.pdf file over to Archive` |
| H002 | 12 | pass | A | `copy the Q3 budget.xlsx to my Backup folder` |
| H003 | 13 | pass | A | `move every PDF in Documents to Archive` |
| H004 | 14 | pass | A | `move the screenshots from Desktop to Pictures` |
| H005 | 15 | pass | A | `copy all the images in Pictures to Backup` |
| H006 | 16 | pass | A | `move the video files from Downloads to Movies` |
| H007 | 17 | pass | A | `move Desktop screenshots to Archive` |
| H008 | 18 | pass | A | `rename notes.txt to meeting notes.txt` |
| H009 | 19 | pass | A | `rename "Draft 2" to "Final"` |
| H010 | 20 | pass | A | `copy the file called Lease Agreement.pdf to Documents` |
| H011 | 21 | pass | A | `move the invoice from Downloads to Archive` |
| H012 | 22 | pass | A | `delete the claude.dmg file in Downloads` |
| H013 | 23 | pass | A | `move the Project folder to Archive` |
| H014 | 24 | pass | A | `copy the Thesis folder into Backup` |
| H015 | 25 | pass | A | `move report.pdf to Documents` |
| H016 | 26 | pass | A | `move the 2025 tax return.pdf to Documents` |
| H017 | 27 | pass | A | `move Invoice Final v2.pdf to Archive` |
| H018 | 28 | pass | A | `put photo.heic into Pictures` |
| H019 | 29 | pass | A | `send thesis draft dot docx to Documents` |
| H020 | 30 | pass | A | `move screenshot 2026-10-01 at 9.41.12 AM.png to Pictures` |
| H021 | 33 | pass | B | `list PDFs in Downloads` |
| H022 | 34 | pass | B | `how many screenshots are on my Desktop` |
| H023 | 35 | pass | B | `count the images in Pictures` |
| H024 | 36 | pass | B | `show the size of Downloads` |
| H025 | 37 | pass | B | `how big is the Documents folder` |
| H026 | 38 | pass | B | `list duplicate files in Downloads` |
| H027 | 39 | pass | B | `find duplicates in Documents` |
| H028 | 40 | pass | B | `what is in Downloads` |
| H029 | 41 | pass | B | `what's on my Desktop` |
| H030 | 42 | pass | B | `show me files in Downloads` |
| H031 | 43 | pass | B | `what files did I download yesterday` |
| H032 | 44 | pass | B | `undo last` |
| H033 | 45 | pass | B | `undo that` |
| H034 | 46 | pass | B | `show recent actions` |
| H035 | 47 | pass | B | `open Downloads in Finder` |
| H036 | 48 | pass | B | `reveal the file named report.pdf` |
| H037 | 49 | pass | B | `show me where invoice.pdf is` |
| H038 | 50 | pass | B | `count PDFs in Documents` |
| H039 | 51 | pass | B | `list everything in Desktop` |
| H040 | 52 | pass | B | `undo the last delete` |
| H041 | 55 | pass | C | `list screenshots from yesterday` |
| H042 | 56 | pass | C | `show PDFs from today` |
| H043 | 57 | pass | C | `list PDFs older than 30 days in Documents` |
| H044 | 58 | pass | C | `list files older than 3 months in Downloads` |
| H045 | 59 | pass | C | `list files older than a year in Downloads` |
| H046 | 60 | pass | C | `find screenshots from the last 2 weeks` |
| H047 | 61 | pass | C | `list screenshots from last week` |
| H048 | 62 | pass | C | `list files I downloaded in the last 3 days in Downloads` |
| H049 | 63 | pass | C | `list screenshots from July 2026` |
| H050 | 64 | pass | C | `list files in Documents since March 2026` |
| H051 | 65 | pass | C | `list files in Documents before March 2026` |
| H052 | 66 | pass | C | `list PDFs created after August 2026` |
| H053 | 67 | pass | C | `list files until June 30` |
| H054 | 68 | pass | C | `list screenshots from this month` |
| H055 | 69 | pass | C | `list PDFs from this year` |
| H056 | 70 | pass | C | `count photos taken in August` |
| H057 | 71 | pass | C | `delete screenshots older than 45 days` |
| H058 | 72 | pass | C | `move PDFs from last month to Archive` |
| H059 | 73 | pass | C | `list files from the last 6 months in Downloads` |
| H060 | 74 | pass | C | `list files from yesterday in Downloads` |
| H061 | 75 | pass | C | `list files in Documents older than 2 years` |
| H062 | 76 | pass | C | `move the screenshots taken in March 2025 to Archive` |
| H063 | 77 | pass | C | `list photos from June 2026` |
| H064 | 78 | pass | C | `list photos in 2024` |
| H065 | 79 | pass | C | `list the files from the past week in Desktop` |
| H066 | 82 | pass | D | `list files larger than 100 MB in Downloads` |
| H067 | 83 | pass | D | `delete files larger than 1 GB in Downloads` |
| H068 | 84 | pass | D | `list files bigger than 500 MB in Desktop` |
| H069 | 85 | pass | D | `list files smaller than 1 MB in Documents` |
| H070 | 86 | pass | D | `list files over 2 GB in Downloads` |
| H071 | 87 | pass | D | `list files between 10 MB and 50 MB in Downloads` |
| H072 | 88 | pass | D | `list files at least 250 MB in Downloads` |
| H073 | 89 | pass | D | `show files in Downloads at most 20 MB` |
| H074 | 90 | pass | D | `list files that are 500 MB or more in Downloads` |
| H075 | 91 | pass | D | `delete files under 5 MB in Downloads` |
| H076 | 92 | pass | D | `move installers over 200 MB from Downloads to External` |
| H077 | 93 | pass | D | `list files larger than 1 gigabyte in Downloads` |
| H078 | 96 | pass | E | `list the Word documents in Documents` |
| H079 | 97 | pass | E | `count the spreadsheets in Downloads` |
| H080 | 98 | pass | E | `move text files from Desktop to Documents` |
| H081 | 99 | pass | E | `delete the zip files in Downloads` |
| H082 | 100 | pass | E | `move all MP4s from Movies to Videos` |
| H083 | 101 | pass | E | `count the png files in Desktop` |
| H084 | 102 | pass | E | `show me the installers in Downloads` |
| H085 | 103 | pass | E | `list DMG files in Downloads` |
| H086 | 104 | pass | E | `move audio files from Downloads to Music` |
| H087 | 105 | pass | E | `list the PDF files in Documents` |
| H088 | 106 | pass | E | `list the JPEGs in Pictures` |
| H089 | 107 | pass | E | `list HEIC photos in Pictures` |
| H090 | 108 | pass | E | `list the photos in Pictures named cat` |
| H091 | 109 | pass | E | `list the Excel files in Documents` |
| H092 | 110 | pass | E | `list the text files in Documents` |
| H093 | 113 | pass | F | `delete "last"` |
| H094 | 114 | pass | F | `delete last.pdf` |
| H095 | 115 | pass | F | `move "last" to Archive` |
| H096 | 116 | pass | F | `copy last.txt to Desktop` |
| H097 | 117 | pass | F | `delete the file "report.pdf"` |
| H098 | 118 | pass | F | `rename "old notes.txt" to "new notes.txt"` |
| H099 | 119 | pass | F | `move “Q3 report.pdf” to Documents` |
| H100 | 120 | pass | F | `move 'Tax Notes.pdf' to Documents` |
| H101 | 121 | pass | F | `delete files named "budget" in Downloads` |
| H102 | 122 | pass | F | `delete last.pdf from Downloads` |
| H103 | 123 | pass | F | `move Ben & Jerry.pdf to Archive` |
| H104 | 124 | pass | F | `rename invoice (final).pdf to invoice-v2.pdf` |
| H105 | 125 | pass | F | `delete draft_v1.docx` |
| H106 | 126 | pass | F | `move the 2026-01-15 receipt.pdf to Archive` |
| H107 | 127 | pass | F | `move the report dot pdf to Documents` |
| H108 | 130 | pass | G | `move ~/Desktop/receipt.pdf to Documents` |
| H109 | 131 | pass | G | `move Downloads/Invoices/march.pdf to Archive` |
| H110 | 132 | pass | G | `list files in Downloads/Invoices` |
| H111 | 133 | pass | G | `list PDFs in the Downloads folder` |
| H112 | 134 | pass | G | `list PDFs in my Documents folder` |
| H113 | 135 | pass | G | `sort Documents by month` |
| H114 | 136 | pass | G | `list files in Documents/Taxes` |
| H115 | 137 | pass | G | `move photos from Pictures/Trip to Archive` |
| H116 | 138 | pass | G | `list screenshots in Desktop/Screenshots` |
| H117 | 139 | pass | G | `copy the Notes folder to Backup` |
| H118 | 140 | pass | G | `list PDFs inside Documents/Scans` |
| H119 | 141 | pass | G | `show size of Documents/Taxes` |
| H120 | 142 | pass | G | `count PDFs in Downloads/Invoices` |
| H121 | 143 | pass | G | `move the Downloads/Notes folder to Archive` |
| H122 | 144 | pass | G | `list files in Pictures/Trip folder` |
| H123 | 147 | pass | H | `replace spaces with underscores in file names in Pictures` |
| H124 | 148 | pass | H | `replace dashes with spaces in names of files in Documents` |
| H125 | 149 | pass | H | `add prefix 2026- to files in Downloads` |
| H126 | 150 | pass | H | `add prefix Invoice dash to PDFs in Documents` |
| H127 | 151 | pass | H | `add a date prefix to screenshots in Desktop` |
| H128 | 152 | pass | H | `number PDFs in Scans as scan-01, scan-02` |
| H129 | 153 | pass | H | `lowercase the names of PDFs in Documents` |
| H130 | 154 | pass | H | `remove spaces from names in Documents` |
| H131 | 155 | pass | H | `convert IMG_2041.heic to jpg` |
| H132 | 156 | pass | H | `resize the PNGs in Desktop to 800 pixels wide` |
| H133 | 157 | pass | H | `merge PDFs in Documents named invoice into invoices-2026.pdf` |
| H134 | 158 | pass | H | `split scan.pdf into pages` |
| H135 | 159 | pass | H | `extract pages 1 to 3 from book.pdf into intro.pdf` |
| H136 | 160 | pass | H | `make a PDF from images in Scans` |
| H137 | 161 | pass | H | `unzip Backup.zip` |
| H138 | 164 | pass | I | `zip Thesis folder into Thesis.zip` |
| H139 | 165 | pass | I | `zip the Photos folder` |
| H140 | 166 | pass | I | `create folder Receipts in Documents` |
| H141 | 167 | pass | I | `sort Pictures by type` |
| H142 | 168 | pass | I | `convert all PNGs in Desktop to jpg` |
| H143 | 169 | pass | I | `rename report.pdf to "Report 2026.pdf"` |
| H144 | 170 | pass | I | `rename Scan 001.pdf to receipt-001.pdf` |
| H145 | 171 | pass | I | `rename the file report.pdf as final.pdf` |
| H146 | 172 | pass | I | `copy notes.txt into Backup/2026` |
| H147 | 173 | pass | I | `copy the JPEGs from Pictures into Backup/Photos` |
| H148 | 174 | pass | I | `move installers from Downloads to External Drive` |
| H149 | 175 | pass | I | `put the Q3 invoice into Archive` |
| H150 | 176 | pass | I | `move Chapter one to Archive` |
| H151 | 179 | pass | R1 | `delete the last file` |
| H152 | 180 | pass | R1 | `move the previous file to Documents` |
| H153 | 181 | pass | R1 | `copy the last one to Desktop` |
| H154 | 182 | pass | R1 | `rename the last file to final.pdf` |
| H155 | 183 | **FAIL** | R1 | `delete the first file in Downloads` |
| H156 | 184 | **FAIL** | R1 | `move the second file in Downloads to Archive` |
| H157 | 185 | pass | R1 | `delete the newest file` |
| H158 | 186 | pass | R1 | `move the latest screenshot to Pictures` |
| H159 | 187 | pass | R1 | `delete the most recent file in Downloads` |
| H160 | 188 | pass | R1 | `delete them` |
| H161 | 189 | pass | R1 | `move it to Archive` |
| H162 | 190 | pass | R1 | `copy that to Desktop` |
| H163 | 191 | pass | R1 | `delete those in Downloads` |
| H164 | 192 | pass | R1 | `move these files to Archive` |
| H165 | 193 | **FAIL** | R1 | `delete the one from yesterday` |
| H166 | 194 | pass | R1 | `move the old one to Archive` |
| H167 | 195 | pass | R1 | `rename it to report-final.pdf` |
| H168 | 196 | **FAIL** | R1 | `delete the file from before` |
| H169 | 197 | **FAIL** | R1 | `delete the thing I downloaded yesterday` |
| H170 | 198 | **FAIL** | R1 | `move the one I took yesterday to Archive` |
| H171 | 199 | pass | R1 | `delete the big file` |
| H172 | 200 | pass | R1 | `delete the old file in Downloads` |
| H173 | 201 | pass | R1 | `rename the newest screenshot to today.png` |
| H174 | 202 | pass | R1 | `delete the biggest file in Downloads` |
| H175 | 203 | pass | R1 | `move the largest PDF to Documents` |
| H176 | 204 | pass | R1 | `delete the smallest screenshot` |
| H177 | 205 | pass | R1 | `copy the file I sent you yesterday to Desktop` |
| H178 | 206 | **FAIL** | R1 | `delete the file you were looking at` |
| H179 | 207 | **FAIL** | R1 | `move the file we talked about to Archive` |
| H180 | 208 | **FAIL** | R1 | `delete the thing on my screen` |
| H181 | 211 | pass | R2 | `move report.pdf` |
| H182 | 212 | pass | R2 | `move to Documents` |
| H183 | 213 | pass | R2 | `copy to Desktop` |
| H184 | 214 | pass | R2 | `delete` |
| H185 | 215 | **FAIL** | R2 | `move the file to Documents` |
| H186 | 216 | pass | R2 | `move files to` |
| H187 | 217 | pass | R2 | `rename to final.pdf` |
| H188 | 218 | pass | R2 | `rename report.pdf to` |
| H189 | 219 | pass | R2 | `move my PDFs to` |
| H190 | 220 | pass | R2 | `send them` |
| H191 | 221 | **FAIL** | R2 | `copy notes.txt to the` |
| H192 | 222 | **FAIL** | R2 | `delete from Downloads` |
| H193 | 223 | **FAIL** | R2 | `move from Downloads to Documents` |
| H194 | 224 | **FAIL** | R2 | `list in` |
| H195 | 225 | pass | R2 | `count` |
| H196 | 226 | **FAIL** | R2 | `show size` |
| H197 | 227 | pass | R2 | `reveal` |
| H198 | 228 | pass | R2 | `open` |
| H199 | 229 | **FAIL** | R2 | `move "" to Documents` |
| H200 | 230 | pass | R2 | `move notes.txt in Documents` |
| H201 | 233 | pass | R3 | `move report.pdf and invoice.pdf to Documents` |
| H202 | 234 | pass | R3 | `delete the PDFs and images in Downloads` |
| H203 | 235 | pass | R3 | `copy notes.txt and todo.txt to Desktop` |
| H204 | 236 | pass | R3 | `move everything except PDFs from Downloads to Archive` |
| H205 | 237 | pass | R3 | `delete all PDFs in Downloads except invoices` |
| H206 | 238 | pass | R3 | `move all files but screenshots from Desktop to Archive` |
| H207 | 239 | pass | R3 | `delete installers other than DMG files from Downloads` |
| H208 | 240 | pass | R3 | `move report.pdf to Documents and Pictures` |
| H209 | 241 | pass | R3 | `copy PDFs to Documents and Backup` |
| H210 | 242 | pass | R3 | `delete old screenshots or installers in Downloads` |
| H211 | 243 | pass | R3 | `list PDFs and images in Documents` |
| H212 | 244 | pass | R3 | `count PDFs in Downloads and Desktop` |
| H213 | 245 | pass | R3 | `delete everything in Downloads without PDFs` |
| H214 | 246 | pass | R3 | `move screenshots not from today to Archive` |
| H215 | 247 | pass | R3 | `delete PDFs then images in Downloads` |
| H216 | 248 | pass | R3 | `move old invoices to Archive and receipts to Finance` |
| H217 | 249 | pass | R3 | `move installers to Trash and PDFs to Archive` |
| H218 | 250 | pass | R3 | `delete Desktop and Downloads screenshots` |
| H219 | 251 | pass | R3 | `copy the report to Desktop, then move it to Archive` |
| H220 | 252 | pass | R3 | `move report.pdf or invoice.pdf to Documents` |
| H221 | 255 | pass | R4 | `permanently delete report.pdf` |
| H222 | 256 | pass | R4 | `shred the old drafts in Downloads` |
| H223 | 257 | pass | R4 | `empty the trash` |
| H224 | 258 | pass | R4 | `wipe Downloads` |
| H225 | 259 | **FAIL** | R4 | `erase everything in Documents` |
| H226 | 260 | pass | R4 | `email the invoice to my accountant` |
| H227 | 261 | **FAIL** | R4 | `compress Project into Project.zip` |
| H228 | 262 | pass | R4 | `upload the PDFs in Documents to Dropbox` |
| H229 | 263 | pass | R4 | `share report.pdf with Alex` |
| H230 | 264 | pass | R4 | `rename all my PDFs with dates` |
| H231 | 265 | pass | R4 | `delete duplicate files in Downloads` |
| H232 | 266 | pass | R4 | `find big files in Downloads and delete them` |
| H233 | 267 | pass | R4 | `delete the files that are not needed` |
| H234 | 268 | pass | R4 | `move the files that look like invoices to Archive` |
| H235 | 269 | pass | R4 | `delete anything older than 10 years that looks unused` |
| H236 | 270 | **FAIL** | R4 | `clean up my desktop mess` |
| H237 | 271 | pass | R4 | `sort my files by size` |
| H238 | 272 | **FAIL** | R4 | `sort the downloads by date` |
| H239 | 273 | **FAIL** | R4 | `undo the previous move` |
| H240 | 274 | pass | R4 | `undo 3 moves` |
| H241 | 275 | pass | R4 | `redo last move` |
| H242 | 276 | **FAIL** | R4 | `what's the size of the largest PDF` |
| H243 | 277 | pass | R4 | `which is the newest file in Downloads` |
| H244 | 278 | pass | R4 | `show me the newest PDF in Documents` |
| H245 | 279 | pass | R4 | `what is the biggest file in Downloads` |
| H246 | 282 | **FAIL** | R5 | `list files larger than 10 MB and smaller than 5 MB in Downloads` |
| H247 | 283 | **FAIL** | R5 | `list files larger than 10 MB smaller than 5 MB in Downloads` |
| H248 | 284 | pass | R5 | `delete files older than 10 days and newer than 5 days` |
| H249 | 285 | **FAIL** | R5 | `list files older than 30 days from today` |
| H250 | 286 | pass | R5 | `list PDFs from today or yesterday` |
| H251 | 287 | **FAIL** | R5 | `list files from the last 2 days or last month` |
| H252 | 288 | pass | R5 | `list files older than 4000000000 years` |
| H253 | 289 | pass | R5 | `list photos from February 30 in Pictures` |
| H254 | 290 | pass | R5 | `list files from 2026-13-01 in Downloads` |
| H255 | 291 | **FAIL** | R5 | `list screenshots taken in month 13` |
| H256 | 292 | **FAIL** | R5 | `delete files larger than 1 GB but under 500 MB in Downloads` |
| H257 | 293 | **FAIL** | R5 | `move PDFs created after yesterday before today to Archive` |
| H258 | 294 | **FAIL** | R5 | `delete screenshots from today older than a week` |
| H259 | 295 | **FAIL** | R5 | `list files newer than 2 days` |
| H260 | 296 | **FAIL** | R5 | `list files in the last 0 days` |
| H261 | 297 | **FAIL** | R5 | `list files from March 2026 before March 2025` |
| H262 | 298 | **FAIL** | R5 | `list files bigger than 5 in Downloads` |
| H263 | 299 | **FAIL** | R5 | `list files created in 1850 in Downloads` |
| H264 | 300 | **FAIL** | R5 | `delete files larger than 5 GB but smaller than 1 GB` |
| H265 | 301 | pass | R5 | `list files over 10 days old and under 1 week old` |
| H266 | 304 | **FAIL** | R6 | `move Notes to Self.txt to Documents` |
| H267 | 305 | **FAIL** | R6 | `move Plan to Phase 2.pdf to Archive` |
| H268 | 306 | **FAIL** | R6 | `copy Trip to Lisbon.jpg to Pictures` |
| H269 | 307 | **FAIL** | R6 | `move the file Ideas to Drafts.md to Archive` |
| H270 | 308 | **FAIL** | R6 | `rename Notes to Self.txt to Journal.txt` |
| H271 | 309 | pass | R6 | `delete files named report into invoice.pdf in Downloads` |
| H272 | 310 | pass | R6 | `delete files named budget or invoice in Downloads` |
| H273 | 311 | pass | R6 | `delete files containing budget and invoice in Downloads` |
| H274 | 312 | pass | R6 | `list photos named cat or dog in Pictures` |
| H275 | 313 | pass | R6 | `delete the Aeroguard files and the Acme files` |
| H276 | 314 | pass | R6 | `move photos named report.pdf to Archive` |
| H277 | 315 | pass | R6 | `delete videos named invoice.pdf` |
| H278 | 316 | pass | R6 | `list screenshots named budget.pdf in Desktop` |
| H279 | 317 | pass | R6 | `move PDF files named cat.jpg to Archive` |
| H280 | 318 | pass | R6 | `delete the file called stuff*.pdf` |
| H281 | 319 | pass | R6 | `list files named ?.txt in Downloads` |
| H282 | 320 | pass | R6 | `move files with spaces in their names to Archive` |
| H283 | 321 | pass | R6 | `delete files with numbers in their names in Downloads` |
| H284 | 322 | pass | R6 | `delete old files whose names contain invoice and are older than 2 years` |
| H285 | 323 | pass | R6 | `move the files called Project Plan and Notes to Archive` |
| H286 | 326 | pass | R7 | `list files from 2026-02-30 in Downloads` |
| H287 | 327 | pass | R7 | `list screenshots taken on July 32` |
| H288 | 328 | **FAIL** | R7 | `list files from the year 3000` |
| H289 | 329 | **FAIL** | R7 | `delete files older than -5 days` |
| H290 | 330 | **FAIL** | R7 | `list files from the 1st week of 2026` |
| H291 | 331 | **FAIL** | R7 | `list screenshots from last fortnight` |
| H292 | 332 | **FAIL** | R7 | `move files older than 2 hours to Archive` |
| H293 | 333 | pass | R7 | `delete installers from last night` |
| H294 | 334 | pass | R7 | `list files from this morning` |
| H295 | 335 | pass | R7 | `list files from the weekend` |
| H296 | 336 | pass | R7 | `move screenshots from two weeks before last to Archive` |
| H297 | 337 | pass | R7 | `delete files from 3 days ago in Downloads` |
| H298 | 338 | pass | R7 | `move screenshots from tonight to Pictures` |
| H299 | 339 | pass | R7 | `list PDFs from a month ago` |
| H300 | 340 | pass | R7 | `delete the files from an hour ago` |
