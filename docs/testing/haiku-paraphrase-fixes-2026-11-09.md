# Paraphrase interpretation fixes — 2026-11-09

The same 428-row Haiku audit was rerun against the fixes without changing its
phrases or expectations. The original report and JSONL remain the baseline.
Results: `haiku-paraphrase-audit-fixed-2026-11-09.jsonl`.

## Fixed

All ten wrong interpretations in the 302-paraphrase set now produce the expected
intent and arguments:

- P066: `the word draft` means the replacement text `draft`.
- P070/P088: `filenames` is recognized as filename grammar.
- P071: `names in documents` preserves the explicit Documents folder instead
  of expanding to document-type files in the default folders.
- P078: `space characters` means a space.
- P154: `into a file called combined.pdf` produces `combined.pdf`.
- P168: `into one page each` is understood as splitting the source PDF.
- P206: `screenshot files` preserves screenshot type and the screenshot search
  roots, instead of a substring search among arbitrary default-folder files.
- P268/P269: `the file report.pdf in Finder` resolves `report.pdf`.

N013, `send report.pdf to my email`, is now refused. It no longer creates a local
`email` folder and moves the source there. Literal folder names can be quoted
(`to "email"`) or given as explicit paths (`to Downloads/email`).

Quoted replacement text and output names remain literal, including
`"word draft"`, `"space characters"`, and `"a file called combined.pdf"`.

## Validation

- All 43 seed controls retain their expected meaning.
- All ten targeted paraphrase rows pass; the paraphrase set has 281 passes and
  21 refusals, with zero wrong interpretations.
- Contextual results retain 16 passes, four expected refusals, and two recorded
  unsupported forms, with zero wrong actions.
- Five new default regression tests cover semantic equivalence, quote
  preservation, email destinations, and real temporary-folder previews that
  isolate Documents and exclude non-image screenshot-name decoys.
- Full library suite: **176 passed, 0 failed, 6 ignored**.

Commands, run from `app/src-tauri`:

```sh
rtk proxy env VOXI_PARAPHRASE_AUDIT_REPORT=/tmp/voxi-paraphrase-after.jsonl cargo test --lib paraphrase_audit -- --ignored --nocapture
rtk cargo test --lib
```

These tests do not execute file operations. Native checks use previews only.

The debug app bundle and frontend build passed and the app was relaunched.
Native checks confirmed that `send report.pdf to my email` is refused, and
`merge all PDFs in Downloads into a file called combined.pdf` previews
`Merge 42 PDFs into combined.pdf`, with destination `~/Downloads/combined.pdf`.
The preview was cancelled; no file operation was confirmed.

## Remaining limitations

The 21 safe paraphrase refusals (including unsupported verbs and compound spoken
numbers), three previously known explicit-dot extension gaps, extra-root bare
`Project` lookup issue, and two recorded first-N contextual forms are outside
these interpretation fixes. They are not claimed as fixed. Parser-accepted
commands which are correctly refused by the planner remain unchanged. The
intentional `merge PDFs` default of `merged.pdf` also remains supported.
