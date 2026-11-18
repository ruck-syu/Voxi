# Source replay audit summary

Parent verification: the raw/canonical 70/200 count below includes the echoed input text and must not be interpreted as paraphrase accuracy. The corrected intent/schedule comparison is in `verified-summary.json`: both sources match 180/192 accepted canonical references, including 30/42 references with substantively different wording. These remain agreement counts, not accuracy; frozen labels and Luna review annotations contain documented issues.

**Scope:** parser, scheduler, planner source replay only. No actions were executed. Installed 0.1.14 runtime is outside this report; parent separately checked its binary against the published release executable.

- Corpus: 300 rows; SHA-256 `851900248b9706c151eb94f9547a65d3f9541c43e8cc3ecbf0d0684ee1cd0fc8`.
- Fixture integrity: `fixture_unchanged=True`.
- Both source variants produced 300 result rows. Exact result and per-module source hashes are in `manifest.json` and `source-summary.json.

## Outcomes

| Source | Raw stages | Refusal labels matched by raw refusal | Accepted despite refusal label | Refusal despite acceptance label | Exact raw/canonical matches | Canonical invalid/refused |
|---|---|---:|---:|---:|---:|---:|
| dev | accepted: 224, parser_refusal: 73, schedule_refusal: 3 | 64 | 35 | 12 | 70/200 | 8/200 |
| production-source | accepted: 227, parser_refusal: 70, schedule_refusal: 3 | 61 | 38 | 12 | 70/200 | 8/200 |

Accepted despite refusal label means parser produced an intent; planner may still return a problem or a plan. It does not by itself establish whether the result is safe. Exact raw/canonical equality is not a semantic correctness score. Neither source produced planner `ambiguous` results. Eight canonical outcomes refuse (7 parser, 1 scheduler), so they are invalid/refused references. Of 200 rows with canonical outcomes, 69 canonical strings repeat the command verbatim and are not independent comparisons.

## Concrete raw outcomes (dev)

- **F076** `move PDFs from Downloads to Documents larger than 1 GB smaller than 100 MB` — refusal label `True`; raw stage `accepted`. Intent: `{"intent": "move", "src": {"filter": {"type": "pdfs"}, "folder": "Downloads", "kind": "set"}, "to": "Documents larger than 1 GB smaller than 100 MB"}`. Planner: `{"actions": [{"kind": "create_folder", "path": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Downloads/Documents larger than 1 GB smaller than 100 MB"}, {"from": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Downloads/report.pdf", "kind": "move", "to": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Downloads/Documents larger than 1 GB smaller than 100 MB/report.pdf"}, {"from": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Downloads/scan.pdf", "kind": "move", "to": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Downloads/Documents larger than 1 GB smaller than 100 MB/scan.pdf"}], "status": "steps", "title": "Move 2 files to Documents larger than 1 GB smaller than 100 MB", "warnings": []}`.
- **F070** `move files from Downloads to Desktop between 10 and 50 MB` — refusal label `False`; raw stage `parser_refusal`. Intent: `null`. Planner: `null`.
- **F096** `delete everything` — refusal label `True`; raw stage `accepted`. Intent: `{"intent": "trash", "src": {"filter": {"type": "files"}, "folder": null, "kind": "set"}}`. Planner: `{"message": "To be safe, clean-up needs a filter, like “older than 90 days”, “installers”, or “larger than 500 MB”.", "status": "problem"}`.
- **T010** `Rename report.pdf to Documents/report.pdf` — refusal label `True`; raw stage `accepted`. Intent: `{"intent": "rename", "new_name": "Documents/report.pdf", "src": {"folder": null, "kind": "item", "name": "report.pdf"}}`. Planner: `{"message": "New names can't contain “/”.", "status": "problem"}`.
- **T030** `Replace spaces with slashes in filenames in Downloads` — refusal label `True`; raw stage `accepted`. Intent: `{"find": " ", "intent": "replace_in_names", "replace": "slashes", "src": {"filter": {"type": "files"}, "folder": "Downloads", "kind": "set"}}`. Planner: `{"actions": [{"kind": "rename", "newName": "NotesslashestoslashesSelf.txt", "path": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Downloads/Notes to Self.txt"}, {"kind": "rename", "newName": "TripslashesatslashesHome.txt", "path": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Downloads/Trip at Home.txt"}], "status": "steps", "title": "Rename: 11 files in Downloads", "warnings": ["9 names already fine, left alone."]}`.
- **T066** `Extract pages 0 to 3 from report.pdf into excerpt.pdf` — refusal label `True`; raw stage `accepted`. Intent: `{"from": 0, "intent": "extract_pages", "output": "excerpt.pdf", "src": {"folder": null, "kind": "item", "name": "report.pdf"}, "to": 3}`. Planner: `{"message": "report.pdf has 3 pages; pick a range from 1 to 3.", "status": "problem"}`.
- **S022** `Move Notes to Self.txt from Documents to Desktop after 1 hour` — refusal label `False`; raw stage `parser_refusal`. Intent: `null`. Planner: `null`.
- **S031** `Move files from Downloads to Backup after 2 hours and 15 minutes` — refusal label `False`; raw stage `schedule_refusal`. Intent: `null`. Planner: `null`.
- **F001** `move PDFs from Downloads to Documents` — refusal label `False`; raw stage `accepted`. Intent: `{"intent": "move", "src": {"filter": {"type": "pdfs"}, "folder": "Downloads", "kind": "set"}, "to": "Documents"}`. Planner: `{"actions": [{"from": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Downloads/report.pdf", "kind": "move", "to": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Documents/report.pdf"}, {"from": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Downloads/scan.pdf", "kind": "move", "to": "/var/folders/dv/6gy5d8fx3g52r7kmcf_msgwc0000gn/T/voxi-dual-source-ormera6x/fixture-home/Documents/scan.pdf"}], "status": "steps", "title": "Move 2 files to Documents", "warnings": []}`.

F076 accepts contradictory size-filter wording and treats the trailing words as destination text. F096 accepts “delete everything” but planner stops for a missing safety filter. T010 accepts a slash-containing rename intent and planner rejects it. T030 treats “slashes” as literal replacement text and plans renames. F070 and S022 are parser refusals despite acceptance labels. S031 is a schedule refusal for “2 hours and 15 minutes”; its canonical form also refuses. T066 parses an extraction intent, then planner rejects page 0 because the synthetic PDF has 3 pages.

## Dev versus release-tag source

The raw `actual` objects differ on 29/300 commands; parser/acceptance stage flips occur on F085, T045, and T065 (all dev refusals, production-source accepts). F001 also differs materially: the v0.1.14 planner includes nested `Downloads/Archive/old.pdf`, while dev plans only direct files. T065 production-source accepts a merge plan whose output name includes the literal phrase “and delete originals.” These are extracted-source comparison findings, not runtime observations. Full raw objects for these cases are in `source-summary.json`.

## Limits

- Harness calls extracted Rust parser, scheduler, and planner modules with a synthetic fixture. It does not call the app UI or installed binary, test OS permissions, or execute actions.
- Parser date is local date; replay was run 2026-11-16. Schedule time is fixed at 2026-11-16 12:00 +05:30.
- `fixture_unchanged=true` verifies the replay fixture only. Planned actions are data and were not executed.
- Frozen human/model labels are not a semantic oracle. Canonical equality is useful for regression comparison but cannot establish that either interpretation is correct.
- See `manifest.json` for extracted source revisions and module hashes, and `dev-results.jsonl` / `production-source-results.jsonl` for raw per-row results.
