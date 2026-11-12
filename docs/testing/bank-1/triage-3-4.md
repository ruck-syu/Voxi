# Triage: bank-1 misses 3 (list/count/size) and 4 (renames)

Total misses: 215 (123 in file 3, 92 in file 4). One triage line per miss in `triage-3-4.jsonl`.

## Counts per label

| label | count |
|---|---|
| gap | 195 |
| wrong_parse | 16 |
| policy | 3 |
| label_error | 1 |

## Clusters (sorted by count)

| label | cluster | count | example 1 | example 2 |
|---|---|---|---|---|
| gap | typo_verb | 62 | lsit PDFs in Downloads | lisst the videos in Downloads |
| gap | leading_filler | 32 | i need to see the PDFs in Downloads | i need to see the videos in Downloads quickly |
| gap | rename_phrasing | 19 | report.pdf should be called final report.pdf | could you call the file report.pdf final report.pdf instead |
| gap | question_form | 15 | what pdfs are in Downloads? | got any PDFs in Downloads? |
| gap | count_phrasing | 14 | number of PDFs in Documents | i need the count of PDFs in Documents |
| gap | verb_synonym | 14 | pull up the PDF files in Downloads | pull up the Word documents in Documents |
| gap | replace_phrasing | 13 | change spaces to underscores in names of files in Downloads | swap spaces for underscores in names of files in Downloads |
| gap | prefix_phrasing | 11 | put Invoice- in front of all PDFs in Downloads | stick the prefix Invoice- on all PDFs in Downloads |
| wrong_parse | new_name_leak | 10 | could you give the Claude dmg the new name Claude | can you give notes.txt the new name meeting notes.txt |
| gap | size_phrasing | 7 | what size is the Documents folder | i need to know how big the Documents folder is |
| gap | typo | 4 | hwo many screenshots are on my Desktop | hwo big is the Documents folder |
| gap | case_phrasing | 2 | change all the file names in Desktop to lowercase | change the names of PDFs in Documents to lowercase |
| gap | numbering_phrasing | 2 | give the photos in Trip numbered names like trip-001 | give the PDFs in Scans numbers like scan-01, scan-02 |
| policy | yes_no_existence | 2 | any duplicates in Documents? | are there duplicate files in Downloads? |
| wrong_parse | prefix_phrasing | 2 | can you put the prefix 2026- on every file in Documents | put a date in front of screenshots on the Desktop |
| wrong_parse | question_form | 2 | what's in the Pictures/Trip folder | i need to find where report.pdf is |
| label_error | path_in_new_name | 1 | rename report.pdf to ../final.pdf |  |
| policy | path_in_new_name | 1 | rename report.pdf to a/final.pdf |  |
| wrong_parse | numbering_phrasing | 1 | number the PDFs in Scans as scan-01 and scan-02 |  |
| wrong_parse | trailing_filler | 1 | i need to find the file named passport.pdf quickly |  |

## wrong_parse (non-refusal, different intent)

- `3-list-count-size.txt:52` `i need to find the file named passport.pdf quickly`: got `{"intent":"list","src":{"filter":{"nameContains":"passport.pdf quickly","type":"files"},"kind":"set"}}` (trailing_filler). 'quickly' absorbed into nameContains; file-named search became set filter
- `3-list-count-size.txt:102` `what's in the Pictures/Trip folder`: got `{"intent":"list","src":{"kind":"item","name":"Pictures/Trip"}}` (question_form). folder question became item lookup named 'Pictures/Trip'
- `3-list-count-size.txt:305` `i need to find where report.pdf is`: got `{"intent":"list","src":{"kind":"item","name":"where report.pdf is"}}` (question_form). where-is question became item named 'where report.pdf is'
- `4-renames.txt:41` `could you give the Claude dmg the new name Claude`: got `{"intent":"rename","new_name":"Claude","src":{"kind":"item","name":"Claude dmg new"}}` (new_name_leak). 'new' from 'new name' leaked into source name
- `4-renames.txt:51` `can you give notes.txt the new name meeting notes.txt`: got `{"intent":"rename","new_name":"meeting notes.txt","src":{"kind":"item","name":"notes.txt new"}}` (new_name_leak). 'new' from 'new name' leaked into source name
- `4-renames.txt:71` `could you give "old notes.txt" the new name "new notes.txt"`: got `{"intent":"rename","new_name":"new notes.txt","src":{"kind":"item","name":"\"old notes.txt\" new"}}` (new_name_leak). 'new' from 'new name' leaked into quoted source name
- `4-renames.txt:81` `could you give invoice (final).pdf the new name invoice-v2.pdf`: got `{"intent":"rename","new_name":"invoice-v2.pdf","src":{"kind":"item","name":"invoice (final).pdf new"}}` (new_name_leak). 'new' from 'new name' leaked into source name
- `4-renames.txt:91` `could you give report.pdf the new name "Report 2026.pdf"`: got `{"intent":"rename","new_name":"Report 2026.pdf","src":{"kind":"item","name":"report.pdf new"}}` (new_name_leak). 'new' from 'new name' leaked into source name
- `4-renames.txt:101` `could you give Scan 001.pdf the new name receipt-001.pdf`: got `{"intent":"rename","new_name":"receipt-001.pdf","src":{"kind":"item","name":"Scan 001.pdf new"}}` (new_name_leak). 'new' from 'new name' leaked into source name
- `4-renames.txt:111` `could you give report.pdf the new name final.pdf`: got `{"intent":"rename","new_name":"final.pdf","src":{"kind":"item","name":"report.pdf new"}}` (new_name_leak). 'new' from 'new name' leaked into source name
- `4-renames.txt:121` `could you give scan001.pdf the new name Lease agreement.pdf`: got `{"intent":"rename","new_name":"Lease agreement.pdf","src":{"kind":"item","name":"scan001.pdf new"}}` (new_name_leak). 'new' from 'new name' leaked into source name
- `4-renames.txt:141` `could you give the folder Scans the new name Scanned Documents`: got `{"intent":"rename","new_name":"Scanned Documents","src":{"kind":"item","name":"Scans new"}}` (new_name_leak). 'new' from 'new name' leaked into source name
- `4-renames.txt:151` `could you give Holiday photos the new name Vacation`: got `{"intent":"rename","new_name":"Vacation","src":{"kind":"item","name":"Holiday photos new"}}` (new_name_leak). 'new' from 'new name' leaked into source name
- `4-renames.txt:171` `can you put the prefix 2026- on every file in Documents`: got `{"intent":"move","src":{"folder":"files","kind":"item","name":"prefix 2026-"},"to":"Documents"}` (prefix_phrasing). 'move' intent on file 'prefix 2026-'; would move a file
- `4-renames.txt:207` `put a date in front of screenshots on the Desktop`: got `{"intent":"move","src":{"kind":"item","name":"a date"},"to":"front of screenshots on Desktop"}` (prefix_phrasing). 'move' intent on file 'a date'; would move a file
- `4-renames.txt:281` `number the PDFs in Scans as scan-01 and scan-02`: got `{"base":"scan-01 and scan-","intent":"number","src":{"filter":{"type":"pdfs"},"folder":"Scans","kind":"set"},"width":2}` (numbering_phrasing). 'and' form makes base 'scan-01 and scan-'

Two `move` results (4-renames.txt:171 and :207) would move real files if run, so fix those first.

## label_error

- `4-renames.txt:321` `rename report.pdf to ../final.pdf`: the corpus accepts `..` in names at parse time and refuses later (see `delete ../../etc/passwd`, `make folder ../secret on Desktop`). Correct the expected to the rename intent, `{"intent":"rename","new_name":"../final.pdf","src":{"kind":"item","name":"report.pdf"}}`, and move the refusal to an execution-time test.

## policy

- `3-list-count-size.txt:278` `any duplicates in Documents?`: yes/no existence question; corpus refuses 'are there any duplicate PDFs'
- `3-list-count-size.txt:287` `are there duplicate files in Downloads?`: yes/no existence question; corpus refuses 'are there any duplicate PDFs'
- `4-renames.txt:323` `rename report.pdf to a/final.pdf`: slash in new name; no corpus precedent; either answer defensible

## Notes

- `tell me how big Downloads is` is refused in the corpus (`7-folders-unsupported.txt:27`). The `tell me how many ...` misses are labeled `gap:leading_filler` on the basis that `how many` is supported. Confirm that the corpus refusal is not a deliberate policy against `tell me`.
- `rename the Claude dmg to Claude` is accepted in the corpus, so the `Claude dmg` source name is kept as expected (not a label error).
- Folder names in lowercase (`downloads`, `desktop`, `home`) are accepted in the corpus, so those expectations are kept.
