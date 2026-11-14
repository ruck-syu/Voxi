# Triage: bank-1 misses (moves/copies and trash)

Source: `misses-1-moves-copies.txt` (139 misses) and `misses-2-trash.txt` (76 misses). Per-miss rows are in `triage-1-2.jsonl`.

## Counts per label

| label | count |
|---|---|
| gap | 170 |
| policy | 25 |
| wrong_parse | 18 |
| label_error | 2 |
| **total** | **215** |

## Clusters (sorted by count, all labels)

| cluster | count | label(s) | example 1 | example 2 |
|---|---|---|---|---|
| verb_synonym | 55 | gap | `drag the invoice into Archive please` | `toss the files bigger than 1 gigabyte in Downloads` |
| typo_verb | 37 | gap | `mvoe the report.pdf file to my Documents folder` | `coppy notes.txt over to my Desktop` |
| typo_destructive_verb | 25 | policy | `delte the claude.dmg file in Downloads` | `delet screenshots older than 45 days` |
| reorder_destination_first | 20 | gap | `to my Documents folder move the report.pdf file` | `to Archive send the invoice` |
| leading_filler | 20 | gap | `real quick transfer the invoice to Archive` | `just delete the files under 5 MB in Downloads thanks` |
| copy_phrasing | 17 | gap | `make a copy of notes.txt in my Desktop` | `back up my notes to Backup` |
| reorder_source_first | 15 | gap | `from Desktop move all my PDFs to Documents` | `from Downloads move photos into Pictures` |
| quantifier_any_every | 8 | wrong_parse (5), gap (3) | `relocate any PDF files in Downloads over to Documents` | `delete any screenshot older than 45 days for me` |
| trailing_filler | 5 | wrong_parse | `i want to bin all the installers in Downloads real quick` | `clean up the desktop real quick` |
| glob_wildcard | 4 | wrong_parse | `move *.pdf to Documents` | `move Downloads/* to Archive` |
| particle_placement | 3 | gap | `clean my desktop up please` | `throw Thumbs.db away` |
| dotdot_path | 2 | label_error | `move ../Downloads/report.pdf to Documents` | `copy ../notes.txt to Backup` |
| permanent_suffix | 2 | wrong_parse | `um delete report.pdf permanently` | `delete report.pdf for good` |
| type_noun_as_name | 1 | wrong_parse | `transfer every video file in Downloads to Movies` | (only one) |
| the_X_file | 1 | wrong_parse | `throw away the claude.dmg file that's in Downloads` | (only one) |

Note: `typo_destructive_verb` (policy) covers 25 typo'd delete/trash/remove/clean verbs. Refusing them is defensible; the decision is whether to keep that.

## wrong_parse (all 18)

Each produced a non-refusal intent the user did not ask for. Trailing "real quick" and "permanently"/"for good" were the most serious, because they change the target or drop a safety refusal.

1. `1-moves-copies.txt:18` relocate any PDF files in Downloads over to Documents: nameContains "any PDF" instead of pdfs filter
2. `1-moves-copies.txt:82` move any pdf files from Desktop to Documents: nameContains "any pdf"
3. `1-moves-copies.txt:179` transfer every video file in Downloads to Movies: item named "video"
4. `1-moves-copies.txt:251` transfer any photos in Pictures/Trip to Archive: item named "any photos"
5. `1-moves-copies.txt:318` move *.pdf to Documents: item named "*.pdf" (should refuse)
6. `1-moves-copies.txt:319` copy *.png to Backup: item named "*.png" (should refuse)
7. `1-moves-copies.txt:320` move Downloads/* to Archive: item named "Downloads/*" (should refuse)
8. `1-moves-copies.txt:321` copy *.jpg from Pictures to Backup: item named "*.jpg" (should refuse)
9. `2-trash.txt:7` i want to bin all the installers in Downloads real quick: folder "Downloads real quick"
10. `2-trash.txt:8` delete any installers from the Downloads folder: item named "any installers"
11. `2-trash.txt:18` throw away the claude.dmg file that's in Downloads: item named "claude.dmg file that's"
12. `2-trash.txt:100` delete last.pdf from Downloads real quick: folder "Downloads real quick"
13. `2-trash.txt:123` clean up the desktop real quick: item named "desktop real quick"
14. `2-trash.txt:132` clean up the Downloads folder real quick: item named "real quick" in Downloads
15. `2-trash.txt:171` please move everything to the trash real quick: move to folder "trash real quick" (not trash)
16. `2-trash.txt:197` throw away any PNG files older than 2 years in Downloads: nameContains "any PNG"
17. `2-trash.txt:302` um delete report.pdf permanently: trash item "report.pdf permanently" (should refuse)
18. `2-trash.txt:304` delete report.pdf for good: trash item "report.pdf for good" (should refuse)

## label_error (2): delete or correct

The corpus accepts `..` at parse time (`delete ../../etc/passwd`, `make folder ../secret on Desktop`) and enforces safety later, so the `!` expectation conflicts. Either delete these two misses or correct the expected value to what the parser already produces:

- `1-moves-copies.txt:322` move ../Downloads/report.pdf to Documents. Corrected expected: `{"intent":"move","src":{"kind":"item","name":"../Downloads/report.pdf"},"to":"Documents"}`
- `1-moves-copies.txt:323` copy ../notes.txt to Backup. Corrected expected: `{"intent":"copy","src":{"kind":"item","name":"../notes.txt"},"to":"Backup"}`
