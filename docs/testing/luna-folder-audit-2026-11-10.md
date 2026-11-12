# Luna folder phrasing audit — 2026-11-10

## Run and coverage

Ran the report-only ignored test against the compiled Rust `parse::parse` and `Planner`:

```sh
cd app/src-tauri
rtk cargo test --lib luna_folder -- --ignored --nocapture
```

Result: **1 test passed**, 184 filtered out. The audit produced 120 JSONL rows. Each row records expected and actual serialized intent arguments, parse acceptance, and preview status plus normalized target paths. Preview plans use a fresh temporary home fixture per phrase. No plan was confirmed or executed, and no user Documents or Downloads directory was accessed.

Coverage includes 1 exact seed control for `create a folder in Documents/Dev called Toggle Wars.`, 85 supported passes, 8 expected unsafe planner refusals, 4 genuinely ambiguous duplicate-source cases, and 1 ambiguous name phrase. It exercises create name-first and location-first forms, called/named, quoting and spaces, nested relative paths, a fixture-contained absolute path, tilde paths, explicit folder rename/move/copy/trash, and unsafe names and destinations.

| Class and outcome | Rows |
| --- | ---: |
| Supported pass | 85 |
| Supported wrong interpretation | 21 |
| Supported grammar refusal | 1 |
| Ambiguous source correctly returned ambiguous | 4 |
| Ambiguous phrase recorded, not scored as a failure | 1 |
| Unsafe input safely refused by planner | 8 |
| **Total** | **120** |

The JSONL contains **21 wrong-interpretation rows**, plus **1 grammar refusal**. The ambiguous `Build and Test` row is separate and is not included in those 21. One wrong interpretation produces a changing preview with incorrect targets; the other 20 wrong interpretations and the grammar refusal produce no actions in this fixture. Multiple rows exercise the same underlying issue. The blocked fixture outcomes do not prove that a shortened name could never match another real folder.

## Findings

### Incorrect changing preview — high severity

- `create folders named "Alpha and Beta" in Documents/Dev` — expected one folder named `Alpha and Beta`. The parser returns two names, `Alpha` and `Beta`; the preview contains `create:Documents/Dev/Alpha` and `create:Documents/Dev/Beta`.

### Wrong parse, then planner refusal — medium severity

These phrases intend the explicitly named `Project/Child Folder` folder. The parser drops the final `Folder` token and returns `Project/Child`; the planner then reports it cannot find `Child` in `~/Project`, with no preview actions:

- `move folder Project/Child Folder to Documents/Archive`
- `copy folder Project/Child Folder to Documents/Archive`
- `trash folder Project/Child Folder`
- `rename folder Project/Child Folder to New Child`
- `move folder "Project/Child Folder" to Documents/Archive`
- `copy folder 'Project/Child Folder' to Documents/Archive`
- `move Project/Child Folder into Downloads`
- `send folder Project/Child Folder to Downloads`
- `duplicate Project/Child Folder to Downloads`
- `copy Project/Child Folder into Desktop`
- `copy folder "Project/Child Folder" to Desktop`
- `trash Project/Child Folder`
- `remove Project/Child Folder`
- `get rid of Project/Child Folder`
- `rename Project/Child Folder as New Child`
- `change the name of Project/Child Folder to New Child`

`put Project/Child Folder in Downloads` is refused at parse time instead of producing the expected move intent. It creates no preview actions.

The parser also truncates a bare folder name ending in `Folder`:

- `delete folder Other Folder`
- `move Other Folder into Documents`
- `copy Other Folder into Documents`
- `rename Other Folder to Renamed Folder`

It parses the source as `Other`, and the planner cannot find `Other` in the standard roots. These rows use the fixture's extra allowed root named `Other Folder`; this observed failure is the parser truncation. The separate known bare-name lookup gap for an extra allowed root such as `Project` was **not independently exercised** in this phrase set.

### Ambiguity and expectation correction

- `create a folder in Documents/Dev called Build and Test` produces two folders, `Build` and `Test`. The phrase could be read as a folder name or a list, so it is recorded as ambiguous and excluded from definite failures. The quoted `Alpha and Beta` command above removes that ambiguity and remains a definite wrong interpretation.
- `rename/move/copy/trash Toggle Wars` with two matching fixture folders correctly results in planner ambiguity, without actions.

## Expected safety behavior

Planner refusals for traversal-like names, dot-prefixed names, absolute folder names, and moving or copying a folder into itself are expected safeguards. Tilde locations inside the fixture home and an absolute location inside the fixture's allowed Documents tree both previewed the intended target successfully. The `/tmp` destination was rejected. No finding here indicates a safety bypass.

## Artifacts and limitations

- Harness: [luna_folder_audit.rs](../../app/src-tauri/src/luna_folder_audit.rs)
- Full row data: [luna-folder-audit-2026-11-10.jsonl](luna-folder-audit-2026-11-10.jsonl)

The fixture objects used for rename, move, copy, and trash are empty, so the audited preview checks folder target resolution and action paths without involving child-file behavior. The audit is report-only and does not assert that discovered mismatches are fixed. No production code was changed.
