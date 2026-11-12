# Bank 1: paraphrase bank baseline (2026-11-11)

`app/src-tauri/eval/bank-1/` holds 2,037 commands. Haiku agents wrote them as
rewordings of 252 commands taken from the regression corpus and held-out sets, so each one keeps the
original command's expected intent. Its status is **spent**: its misses were used to write
`src/parse/rewrite.rs`, so it no longer measures accuracy. A fresh bank 2 is
needed for that.

Run it with:
`HELDOUT=bank-1 cargo test --lib heldout -- --ignored --nocapture`

## Baseline

**627 of 2,037 wrong (30.8%).** The first run found 632 misses. Five were bad
test lines. The three lines that expected `..` paths to be refused at parse time
were removed, because the corpus accepts `..` there and the planner refuses it.
Two folder-case labels were corrected.

| File | Misses |
|---|---|
| 1-moves-copies | 137 |
| 2-trash | 76 |
| 3-list-count-size | 123 |
| 4-renames | 91 |
| 5-folders-sorting | 65 |
| 6-pdf-images-archives | 46 |
| 7-dictation-undo | 89 |

Each miss is labelled in `triage-*.jsonl`, with summaries in `triage-*.md`.
The first-run misses are listed in `baseline-misses-2026-11-11.txt`.

| Label | Count | Meaning |
|---|---|---|
| gap | 518 | The expected intent is right; the parser should produce it |
| wrong_parse | 70 | The parser produced a different intent, not a refusal |
| policy | 39 | Either answer is defensible; needs a decision |
| label_error | 5 | Fixed (see above) |

## What to fix, in order

1. **Filler words leak into names and folders (wrong_parse, about 45).**
   "uh", "thanks", "real quick", "period" and "for good" become part of the
   name or folder, for example `uh report.pdf` and `Downloads uh`. The planner
   then fails to find the file, so nothing wrong would run, but the message is
   confusing. One normalization pass, removing filler tokens outside quotes,
   fixes most of these.
2. **Phrasing that becomes a move (wrong_parse, high priority).**
   "put the prefix 2026- on every file" and "put a date in front of
   screenshots" parse as moves. "move everything to the trash real quick"
   becomes a move to a folder called "trash real quick".
3. **"give X the new name Y" (10).** The word "new" leaks into the source name.
4. **Leading filler (92)** such as "hey can you", "real quick", "i want to" and
   "okay so".
5. **Verb synonyms (74)** such as transfer, drag, relocate, bin, toss, pull up and
   display, and **typos in verbs (158)** such as mvoe, coppy and sned. These are the
   two biggest clusters.
6. **Commands with the destination first (20) or the source first (15)**, for
   example "to Desktop move X" and "from Downloads, move all PDFs to Documents".
7. Rewordings of copy, rename, replace, prefix, count and size, question forms,
   and folder phrasing ("set up a folder called").

## Policy questions

- **Typos in destructive verbs (25)**, such as "delte", "remvoe" and "trsh". Today
  Voxi refuses them, which is arguably safer. Fuzzy matching would still show a
  preview before anything runs.
- **"delete X permanently", globs (`*.pdf`)**: decided. Both are now refused
  (see below).
- **Multiple parents** ("make a folder called Taxes in Documents and
  Pictures") become a parent folder named "Documents and Pictures".

## After the fix (2026-11-11)

**120 of 2,037 wrong (5.9%)**, down from 627 (30.8%). Wrong intents (the parser
doing something other than what was asked) fell from 70 to 3. All 3 are
caught later: a slash in a new name is refused by the planner, and the other
two fail to find a folder.

- The 1,917 bank commands that now pass moved into the must-pass corpus,
  `src/parse/corpus/11-bank-1.txt`. The corpus grew from 628 to 2,545 commands.
- The 120 misses stay in `eval/bank-1/` as a backlog. 26 of them are typos in
  delete verbs, which stay refused until the policy is decided.
- `heldout-1` is unchanged (0 of 100 wrong) and `heldout-2` is unchanged (9 of 100).

What changed (`src/parse/rewrite.rs`, plus quantifiers in `phrase.rs`):

- **Fillers** such as "uh", "um", "real quick", "thanks", "like", "okay so",
  "pleese" and "i need to see" are removed anywhere outside quotes.
- **Misspelled verbs** are corrected by one edit (mvoe, coppy, lsit,
  orgainze), but only when the command can't be read as typed, so "take that
  back" stays an undo. Typos near a destructive verb (delte, remvoe, trsah,
  claen) are never corrected.
- **Reorderings**: "to Desktop move X", "from Downloads, move X to Y", and "in
  Documents make a folder called X".
- **Rewordings** that map to existing commands: "X should be called Y", "give X the new name
  Y", "put 2026- in front of X", "put a date in front of X", "back up X to Y",
  "make a copy of X in Y", "number of X", "make X into a jpeg", "pull pages 2 to 5
  out of X", "split up X", "into a zip called Y", "what PDFs are in X", "pull
  up", and "toss", "chuck" or "clear out" for delete.
- **Quantifiers**: "every video file" and "any PDF files" become sets by type.
  "every dot mov file" becomes `.mov` files.
- **Always refused**: wildcards outside quotes (`*.pdf`); "permanently", "for
  good" and "shred"; and emptying the Trash.

## Feedback loop

Reports from the app's "Wrong result?" link are saved to `reports.jsonl` in the
app's data folder; Settings → Learning shows them in Finder.
`node scripts/feedback-to-corpus.mjs <reports.jsonl>` turns them into
`eval/inbox/reports-<date>.txt` with `=> ?` placeholders, which a person
fills in before the lines move into `eval/` or the corpus.
