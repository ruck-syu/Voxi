# Triage: bank-1 misses 5, 6, 7

Total MISS lines: 202. JSONL: `/Users/vishalshekhar/Documents/dev/Voxi/docs/testing/bank-1/triage-5-6-7.jsonl` (202 lines).

## Counts per label

| Label | Count |
|---|---|
| gap | 153 |
| wrong_parse | 36 |
| policy | 11 |
| label_error | 2 |

For `gap`, the cluster is the `gap:<cluster>` name.

## Clusters (sorted by count)

| Label:cluster | Count | Example 1 | Example 2 |
|---|---|---|---|
| gap:typo_verb | 59 | `mkae a folder called Taxes in Documents` | `creat folder Receipts in Documents` |
| gap:leading_filler | 40 | `pleese new folder called Receipts` | `please help me organize my pictures by month` |
| gap:folder_phrasing | 25 | `in Documents make a folder called Taxes` | `set up a folder called Taxes in the Documents folder` |
| wrong_parse:mid_filler | 12 | `move the uh report dot pdf file to my Documents folder` | `shift the uh zip files from Downloads to Archives` |
| wrong_parse:trailing_filler | 11 | `move the report dot pdf file to my Documents folder like thanks` | `so copy my notes to Backup uh thanks` |
| policy:undo_target | 7 | `um undo the last move` | `okay so undo the last move` |
| gap:extract_phrasing | 6 | `could you pull pages 2 to 5 out of report.pdf into excerpt.pdf` | `extract pages 2 to 5 from report.pdf and save as excerpt.pdf` |
| wrong_parse:set_phrasing | 6 | `um move every dot mov file from Downloads to Videos` | `okay so move every dot mov file from Downloads to Videos` |
| gap:mid_filler | 6 | `undo the uh last change` | `list my uh recent changes` |
| gap:verb_synonym | 5 | `file Documents away by month` | `tidy the downloads by date` |
| wrong_parse:particle_word | 4 | `could you move the Invoices folder over from Documents to Archive` | `split report.pdf apart into separate pages` |
| gap:convert_phrasing | 4 | `make screenshots into a PDF` | `make photo.HEIC into a jpg` |
| gap:undo_phrasing | 3 | `please take back what you uh just did` | `so like please take back what you just did` |
| label_error:expected_casing | 2 | `could you merge all pdfs in downloads into combined.pdf` | `could you join the pdfs in desktop as summary.pdf` |
| policy:vague_target | 2 | `delete the uh stuff in Downloads` | `like delete the stuff in Downloads` |
| gap:trailing_filler | 2 | `undo the last change uh thanks` | `show my recent actions uh please` |
| policy:history_question | 2 | `what uh changed recently` | `so like what changed recently` |
| wrong_parse:multi_target_parent | 1 | `make a folder called Taxes in Documents and Pictures` |  |
| wrong_parse:split_phrasing | 1 | `split scan.pdf into each page` |  |
| wrong_parse:archive_phrasing | 1 | `compress Downloads/Project into a zip called Project.zip` |  |
| gap:archive_phrasing | 1 | `unzip the archive.zip file` |  |
| gap:number_words | 1 | `so like get rid of the screenshots in Desktop older than a week` |  |
| gap:item_phrasing | 1 | `so like delete the file that's on the desktop called tmp dot txt` |  |

## wrong_parse (every line)

- `5-folders-sorting.txt:300` `could you move the Invoices folder over from Documents to Archive`: particle_word. "over" kept in item name, giving "Invoices folder over".
- `5-folders-sorting.txt:319` `make a folder called Taxes in Documents and Pictures`: multi_target_parent. "and Pictures" folded into parent; corpus refuses this shape.
- `6-pdf-images-archives.txt:44` `split report.pdf apart into separate pages`: particle_word. "apart" kept in file name, giving "report.pdf apart".
- `6-pdf-images-archives.txt:45` `split up report.pdf into separate pages`: particle_word. "up" kept in file name, giving "up report.pdf".
- `6-pdf-images-archives.txt:53` `split scan.pdf into each page`: split_phrasing. "into each page" not recognized; whole tail became file name.
- `6-pdf-images-archives.txt:54` `split up scan.pdf into individual pages`: particle_word. "up" kept in file name, giving "up scan.pdf".
- `6-pdf-images-archives.txt:180` `compress Downloads/Project into a zip called Project.zip`: archive_phrasing. "a zip called" kept in output name, giving "a zip called Project.zip".
- `7-dictation-undo.txt:5` `move the uh report dot pdf file to my Documents folder`: mid_filler. "uh" leaks into file name, giving "uh report.pdf".
- `7-dictation-undo.txt:8` `move the report dot pdf file to my Documents folder like thanks`: trailing_filler. "like" leaks into destination, giving "Documents folder like".
- `7-dictation-undo.txt:33` `so copy my notes to Backup uh thanks`: trailing_filler. "uh" leaks into destination, giving "Backup uh".
- `7-dictation-undo.txt:38` `shift the uh zip files from Downloads to Archives`: mid_filler. "uh zip" becomes a name filter instead of archives type.
- `7-dictation-undo.txt:55` `send the uh invoice to Archive`: mid_filler. "uh" leaks into item name, giving "uh invoice".
- `7-dictation-undo.txt:63` `put the uh tax form into Documents`: mid_filler. "uh" leaks into item name, giving "uh tax form".
- `7-dictation-undo.txt:76` `move report final v2 dot docx to Documents uh thanks`: trailing_filler. "uh" leaks into destination, giving "Documents uh".
- `7-dictation-undo.txt:80` `relocate the uh zip files in Downloads to Archives`: mid_filler. "uh zip" becomes a name filter instead of archives type.
- `7-dictation-undo.txt:92` `change the name of draft dot docx to final dot docx uh thanks`: trailing_filler. "uh" leaks into new name, giving "final.docx uh".
- `7-dictation-undo.txt:100` `change the name of notes dot txt to meeting notes dot txt uh thanks`: trailing_filler. "uh" leaks into new name, giving "meeting notes.txt uh".
- `7-dictation-undo.txt:108` `rename report dot pdf as summary dot pdf uh thanks`: trailing_filler. "uh" leaks into new name, giving "summary.pdf uh".
- `7-dictation-undo.txt:116` `delete uh everything`: mid_filler. "uh" leaks into item name; trash target becomes "uh everything".
- `7-dictation-undo.txt:124` `delete everything in Downloads uh thanks`: trailing_filler. "uh" leaks into folder, giving "Downloads uh".
- `7-dictation-undo.txt:132` `move everything to the uh trash`: mid_filler. "uh" breaks "trash": becomes a move of every file to "uh trash".
- `7-dictation-undo.txt:140` `clean up my Downloads folder uh thanks`: trailing_filler. "uh" becomes a single-item trash target named "uh".
- `7-dictation-undo.txt:164` `delete the file that's on the desktop called tmp dot txt uh thanks`: trailing_filler. "uh" turns item into set filter nameContains "tmp.txt uh".
- `7-dictation-undo.txt:172` `move the video called intro dot mov to Videos uh thanks`: trailing_filler. "uh" leaks into destination, giving "Videos uh".
- `7-dictation-undo.txt:174` `um move every dot mov file from Downloads to Videos`: set_phrasing. "every dot mov file" read as single file named every.mov.
- `7-dictation-undo.txt:175` `okay so move every dot mov file from Downloads to Videos`: set_phrasing. "every dot mov file" read as single file named every.mov.
- `7-dictation-undo.txt:176` `move every dot mov file from Downloads to Videos thanks`: set_phrasing. "every dot mov file" read as single file named every.mov.
- `7-dictation-undo.txt:177` `uh move every dot mov file from Downloads to Videos please`: set_phrasing. "every dot mov file" read as single file named every.mov.
- `7-dictation-undo.txt:178` `move every dot mov file from Downloads to Videos period`: set_phrasing. "every dot mov file" read as single file named every.mov.
- `7-dictation-undo.txt:180` `move every dot mov file from Downloads to Videos uh thanks`: set_phrasing. "every dot mov" read as file name; "uh" leaks into destination.
- `7-dictation-undo.txt:188` `move the screenshots on my desktop to Pictures uh thanks`: trailing_filler. "uh" leaks into destination, giving "Pictures uh".
- `7-dictation-undo.txt:196` `move my uh installers to the trash`: mid_filler. "uh" makes trash target item "uh installers" not the installers set.
- `7-dictation-undo.txt:257` `show my uh history`: mid_filler. "uh" leaks into list item name, giving "uh history".
- `7-dictation-undo.txt:313` `how many PDFs are uh in Documents`: mid_filler. "uh" makes count target "PDFs are uh".
- `7-dictation-undo.txt:321` `list every uh zip in Downloads`: mid_filler. "uh" makes list target item "uh zip".
- `7-dictation-undo.txt:329` `move the uh tax return from Downloads into Taxes`: mid_filler. "uh" leaks into item name, giving "uh tax return".

## label_error (delete or correct)

- `6-pdf-images-archives.txt:14` `could you merge all pdfs in downloads into combined.pdf`: Expected "Downloads" but spoken "downloads"; corpus keeps spoken folder case. Correct expected to `{"intent":"merge_pdf","output":"combined.pdf","src":{"filter":{"type":"pdfs"},"folder":"downloads","kind":"set"}}`.
- `6-pdf-images-archives.txt:23` `could you join the pdfs in desktop as summary.pdf`: Expected "Desktop" but spoken "desktop"; corpus keeps spoken folder case. Correct expected to `{"intent":"merge_pdf","output":"summary.pdf","src":{"filter":{"type":"pdfs"},"folder":"desktop","kind":"set"}}`.

## policy (parser answer defensible either way)

- `7-dictation-undo.txt:144` `delete the uh stuff in Downloads`: "the stuff" is vague; corpus refuses vague delete targets.
- `7-dictation-undo.txt:146` `like delete the stuff in Downloads`: "the stuff" is vague; corpus refuses vague delete targets.
- `7-dictation-undo.txt:231` `um undo the last move`: undo has no target; corpus refuses "undo the last delete".
- `7-dictation-undo.txt:232` `okay so undo the last move`: undo has no target; corpus refuses "undo the last delete".
- `7-dictation-undo.txt:233` `undo the uh last move`: undo has no target; corpus refuses "undo the last delete".
- `7-dictation-undo.txt:234` `undo the last move thanks`: undo has no target; corpus refuses "undo the last delete".
- `7-dictation-undo.txt:235` `undo the last move please`: undo has no target; corpus refuses "undo the last delete".
- `7-dictation-undo.txt:236` `so like undo the last move`: undo has no target; corpus refuses "undo the last delete".
- `7-dictation-undo.txt:237` `undo the last move period`: undo has no target; corpus refuses "undo the last delete".
- `7-dictation-undo.txt:281` `what uh changed recently`: history question refused in corpus ("what did I just do").
- `7-dictation-undo.txt:284` `so like what changed recently`: history question refused in corpus ("what did I just do").
