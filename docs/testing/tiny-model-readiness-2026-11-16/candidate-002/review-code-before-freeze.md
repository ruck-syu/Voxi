# Candidate 002 code review before freeze

## Scope

Reviewed `CANDIDATE-002-PLAN.md` and the original readiness plan; the fallback slot and semantic guard implementations; the parser regressions and changes summarized in `rule-fixes/REPORT.md`; the harness and evaluator builder; the schedule routing in `lib.rs`; planner preview fixtures; and the four legacy expectation adjudications. I did not inspect independent-bank-002 artifacts or new holdout labels/outputs, change code/model/data, or run a model evaluation.

## Findings

No independently reproducible unsafe acceptance or other code defect found in the reviewed candidate-002 paths.

The slot path first requires a grammar miss, then limits preparation to its explicit folder-creation contract and fully quoted literal members. It binds only captured source spans, requires the parsed candidate to preserve every unique alias and its parent role, restores source values by alias identity, and returns the parsed model intent only after certification. Duplicate source values remain distinct bindings; duplicate, missing, extra, or role-swapped model aliases fail closed. Separator parsing skips consumed byte positions, quoted commas remain within a member, and malformed list shapes fail closed. The crate-private bound certifier is only reached after `prepare` has checked routing priority.

The raw fallback still uses the independent source and candidate certificates. Operation, source/destination, bounds and units are compared as typed intents; unsupported source clauses are checked before certification. Recognized scheduled misses do not reach inference in the candidate harness, while original rule-accepted schedules keep their existing path. Unknown schedule wording is rejected by the unsupported-clause/contract gates rather than treated as a literal folder request. The harness performs actual native inference, carries a prepared request through that same call, records errors, and leaves model output in the normal parser/planner path; it does not execute actions.

The parser changes and expectation adjudications inspected are consistent with the stated narrow corrections. In particular, explicit DMG now means `.dmg`, with `.pkg` retained for generic installers; the four legacy rows changed are documented with per-file hashes and fixture rationale. Planner fixtures assert planned output against temporary files and check that planning does not perform the proposed change.

## Limits

This is a source review, not a new native build or full-suite run. Previously reported selected-combination results and test totals were not independently rerun. The app integration, stale-request/worker handling, planner behavior at integration boundaries, and confirmation flow remain release integration requirements; this review does not establish full release readiness or holdout performance.
