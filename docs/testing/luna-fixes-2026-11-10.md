# Luna wording fixes — 2026-11-10

## Implemented

- Both folder-creation word orders split lists only outside quotes. `create
  folders named "Alpha and Beta" in Documents/Dev` previews one folder.
- A folder/item prefix is removed once, without repeatedly stripping literal
  words from the name. Quoted `Project/Child Folder`, including a leading
  spoken `folder`, retains the complete path. Capitalized literal `Folder`
  suffixes are preserved in unquoted names as well.
- An extension identifies an explicit filename before its source directory.
  `Notes from Monday.pdf` no longer becomes an extra folder/date argument.
  Existing refusals for wildcards, multiple files, and incompatible filters
  remain covered by the full corpus.
- Sources accept `located in`, `under`, `the file in <folder> called <name>`,
  and `<folder> file <name>`. Leading scope such as `in Documents/Dev move ...`
  applies only when the command does not already specify another source scope.
- Added explicit count, filename-query and rename phrasings, plus display and
  discard aliases. `put ... in ...` is a move; `put PDFs ... together as ...`
  is a merge. The two forms are tested separately to prevent scope widening.
- Named allowed roots can be resolved as items. Copying a named root is
  supported; moving, renaming, and trashing an allowed root itself are refused.

## Validation

The unchanged Luna expectations were run against the actual parser and planner.
The original baseline JSONL snapshots are retained under the original report
filenames and explicit `-baseline-` filenames. Rerunning the ignored harnesses
now writes `-fixed-` results rather than overwriting those snapshots.

- Folder audit: 120 cases. 104 supported passes; eight safety refusals and four
  ambiguous-source cases pass; one unquoted conjunction is recorded as
  ambiguous. Three expected-preview mismatches remain because the audit asks
  to move/rename/trash an allowed root, which should be refused. There are no
  remaining wrong parser interpretations in this folder set.
- File audit: 448 cases. Of 216 fresh phrasings, 185 match their expected intent
  and arguments, 30 are refused, and one is a planner-guarded expectation
  mismatch: `archive Documents/Dev as Toggle Wars.zip` parses as archiving a
  folder, while the audit expects a set of files. The fixture separately
  registers Dev as an allowed root and the planner refuses zipping that root.
- Ten additional targeted nested-path cases and all three contextual cases
  pass. Older known explicit-dot extension gaps remain separate.
- Full library suite: **181 passed, 0 failed, 8 ignored**. The four new default
  regression tests include exact preview paths with a shorter decoy folder,
  quote preservation, root protection, and reordered source arguments.

Commands, from `app/src-tauri`:

```sh
rtk cargo test --lib
rtk cargo test --lib luna_ -- --ignored --nocapture
```

Results: [folders](luna-folder-audit-fixed-2026-11-10.jsonl) and
[file commands](luna-command-audit-fixed-2026-11-10.jsonl).

The frontend and debug app bundle built successfully. The updated local build
was installed at `/Applications/Voxi.app`, with the preceding app retained in
`~/Library/Application Support/Voxi/local-build-backups.noindex/`.
Native checks confirmed:

- Quoted `Alpha and Beta` previews one folder in Documents/Dev.
- `in Downloads list STL files` lists the five matching real files.
- `move the file in Downloads/AEROGUARD_Clip_Case called
  AEROGUARD_Clip_Case - Kickstand leg.stl to Desktop` previews exactly that file
  with destination Desktop.

No file operation was confirmed; the final move preview was cancelled.

## Deliberate limits

Compound instructions (`copy ... and put it ...`), rename wording with no clear
source/target separator, and unsupported archive/PDF output forms still refuse
rather than guess. This does not claim that every generated paraphrase is now
supported. Content validation of placeholder PDF/image fixtures and actual
execution are not audited. Planner guards in these fixtures do not guarantee
that all arbitrary filesystem contents would yield the same refusal.
