# Methodology and provenance review

## Findings

### Medium: schedule equality cannot detect omitted unsupported schedule language

[`compare.py`](compare.py) defines schedule preservation by equality of the scheduler's extracted schedule objects (around the guarded-row construction). When an original phrase is unsupported, both original and rewritten schedules can be `null`, so equality does not prove that the wording or intended recurrence was retained. This occurs in S029: the original is “Sort Pictures by type every month,” while the rewrite is `sort Pictures by month`; both extracted schedules are null, and the guarded candidate is accepted as an immediate month sort. The per-row evidence is in [`model-guarded-results.jsonl`](model-guarded-results.jsonl) at S029; the source command is in the frozen corpus at S029. The report explicitly discloses this failure and correctly describes the effective result as a monthly-recurrence loss. Treat schedule equality as preservation of *recognized schedule data only*, not a general guarantee that schedule intent is preserved. For future fallback use, unsupported schedule-like source text should be terminal or explicitly detected before a candidate can pass.

## Checks completed

- Ran `rtk python3 docs/testing/tiny-model-vs-production-2026-11-16/verify.py`. It completed successfully and rechecked the 300-row corpus hash and alignment, source snapshot and binary hashes, checkpoint/tokenizer hashes, fixture inventory, output hashes, model rewrite execution through the dev binary, and preservation of noneligible rule outputs/refusals.
- Independently counted stages in all four final replay outputs: production accepted 227, dev accepted 224, raw rewrites through dev accepted 243, and guarded dev accepted 236. Each file contains 300 rows. The report's refusal totals reconcile as parser plus schedule refusals; the verified split is production 70+3, dev 73+3, raw 56+1, guarded 61+3.
- Confirmed replay creates separate pinned `dev-audit` and `production-source-audit` executables after each build, and the final manifest identifies production source `v0.1.14` / commit `44c74fa65fd0b4a8f9a9ad2c79d435e7c30e3ffb` and the captured dirty dev source. `compare.py` checks the dev executable hash before replaying generated text. The initial shared-target/prod-binary mistake is confined to `superseded-initial-run/` and called out in the report.
- Confirmed guarded eligibility is derived from dev `parse_classified`: only `GrammarMiss` plus parser-refusal stage is eligible. Typed hard refusals and schedule/application refusals remain terminal; accepted dev results remain effective. Candidate parser acceptance is required after both guards and schedule-data equality.
- Confirmed inference provenance records CPU FP32, local-only checkpoint, greedy decoding, fixed seed/thread settings, 64-token limits, no truncated inputs, unchanged model file hashes, and the prediction hash. The report's 9.402 seconds is total loop time, not model-load time; the 40-row median/p95 are identified as an un-warmed single-sentence diagnostic rather than app latency.
- Confirmed fixed schedule time is noon IST on November 16, 2026, matching the reported local date. Fixture inventory and source replay are explicitly not installed-app execution. The report does not present heuristic corpus labels as gold or claim an accuracy score, and it disclaims execution/safety conclusions.
- Confirmed the guarded-addition review examples cited in the report match the recorded planner results, including the T035 recovery and the misread/additional planner-stop examples. The report also discloses that semantic reviews can contradict their evidence and that raw reviewer verdicts are not aggregated.

## Conclusion

No blocking provenance, wrong-binary, manifest/hash, count, or routing defect remains in the final comparison. The material limitation above is already disclosed in the report; preserve that qualification anywhere the schedule guard is summarized. This review was limited to methodology and provenance and did not rerun inference or execute plans.
