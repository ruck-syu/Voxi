# Independent evaluation 001 semantic review

Reviewed all 120 frozen H001–H120 cases against `independent-bank.jsonl`, `outcomes.jsonl`, `native-predictions.jsonl`, `scored/comparison.jsonl`, `scored/summary.json`, the frozen candidate provenance records, and the documented command semantics in `README.md`, `website/commands.html`, and `app/src-tauri/src/model.rs`. Planner and scheduler observations are taken from the captured outcomes; schedule due times are interpreted at the fixture clock of **2026-11-16 12:00 IST**. Per-case judgments and proposed corrections are in [`review.jsonl`](review.jsonl).

## Candidate result

The guarded pipeline exactly matched the rules pipeline on these outcomes. There were **zero effective model additions** and **zero recoveries** of supported requests refused by the rules. Across all 120 cases, 19 rule parses were classified as grammar misses; only four of those are clear supported requests: H011, H041, H050, and H076. The other refusal requests must not be counted as missed supported coverage. The four supported rule refusals are H011, H041, H050, and H076. Candidate 001 therefore fails the tested utility objective (0/4 recoveries); it is not ready on this evidence. This does not establish zero total application errors, nor does a no-match fixture prove general safety. The frozen provenance records (`frozen-candidate.json` and `candidate-001-archive.json`) document that the candidate was fixed before outcomes were available.

The captured rewrite proposals for the four supported misses changed literal names or paths. The guard rejected those altered values, preserving correct refusal behavior. These cases demonstrate a useful guard property, not useful model recovery.

## Raw frozen-label comparison

The scorer's strict results are retained as reported and are **not accuracy estimates**. Both rules and guarded had the same counts:

| Strict outcome | Rows |
|---|---:|
| Intent and schedule match frozen label | 37 |
| Intent disagrees with frozen label | 32 |
| Supported label refused by rules | 4 |
| Schedule disagrees with frozen label | 3 |
| Repeat adjudication required | 4 |
| Refusal label refused | 30 |
| Refusal label accepted by parser | 10 |

The 32 strict intent disagreements include representation-only differences (for example, omitted filter type versus explicit `files`, and equivalent date-range encodings) and erroneous frozen labels. The per-row review distinguishes those from material parser errors. In particular, H008 asks for underscores while its frozen intent says dashes; H019/H048 treat a requested output name as a source-name filter or omit it; H040 encodes the `shot-001` exemplar incorrectly; and H042's frozen label says Downloads although the text says Documents. Corrections are proposals only: bank inputs and labels remain unchanged.

## Confirmed meaningful issues

- **Supported grammar coverage:** H011, H041, H050, and H076 are clear supported commands for creating multiple folders or unzipping to an explicit nested destination, but the parser grammar misses them. These are concrete coverage opportunities; candidate 001 did not recover them.
- **Wrong accepted intent:** H019 misreads `named receipts` as a source filter instead of the output name. H040 treats the full `shot-001` example as a numbering base. H066 and H071 broaden the specifically requested `.dmg` set to every installer type, which can change what a repeating trash task acts on. H036 interprets “archive files” as a filename substring rather than the archive file type. H085, H103, H115, and H120 turn unsupported or underspecified language into filename/folder operations. H111–H112 produce schedule interpretations that do not honor the requested constraints.
- **Safety policy and planner stages:** H087 parses a move but the planner blocks `/System/Library` as outside allowed folders. H093–H094 parse unfiltered cleanup/delete commands but the planner blocks them because cleanup requires a filter. These are correctly enforced planner policies, not parser refusals. Conversely, H095 is labeled a refusal despite being an ordinary supported move whose parser produced a plan.
- **Fixture limits:** Several supported commands have correct parsed intent but planner no-match or ambiguity because the named file/folder is absent from the fixture (for example H001/H005 and H020). Those outcomes do not establish parser failures or universal safety. The review records actual planner status rather than inferring intent from plan success.

## Schedule adjudication and limits

Every repeat schedule is checked using its semantic due instant and rule, not by comparing only rule strings. The observed due instants for the valid repeat cases H066–H072 and H075 are checked against the fixed fixture clock; in particular “every other week” correctly yields **2026-11-30 12:00 IST**. H064 has a schedule in the outcome although the frozen label omitted one. H075 likewise has an omitted frozen schedule label. H111 extracts a due time for an out-of-range 101-hour request, and H112 selects a one-time tomorrow schedule while dropping a conflicting repeat clause; neither is a valid interpretation of the request. H108–H110 do not yield an executable schedule for unsupported interval forms.

A parser refusal is distinct from an intrinsic planner-policy block, a correct intent with no fixture match, or an accepted but wrong partial intent. This bank is finite and fixture-bound; no-match does not prove universal safety. Since guarded inference is only a fallback after rules and is blocked from overriding existing rule outputs, existing rule defects remain rule defects rather than model regressions.
