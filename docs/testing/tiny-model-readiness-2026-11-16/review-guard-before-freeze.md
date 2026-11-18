# Fallback guard review before freeze

Reviewed `app/src-tauri/src/fallback_guard.rs` and the report-only replay harness. I did not read or use the independent bank. This review covers the Rust certificate and its current scheduler boundary; it does not approve release or execution.

## Finding

### P1 — A unitless lower range endpoint is certified

At `fallback_guard.rs:53`, the first bound's `MB` marker is optional (`(?:MB)?`), but the range contract in the guard README says each endpoint has an explicit MB unit, and the parent hardening requirement says missing range units must be rejected. `bounded_transfer` then assigns the lower number directly to `larger_than_mb` (`:75-81`) without an explicit unit in the original text.

Reproduction:

```text
original:  Move files from Downloads to Desktop between 10 and 50 MB
candidate: Move files larger than 10 MB smaller than 50 MB in Downloads to Desktop
```

`bounded_transfer(original)` extracts `10` and `50`, constructs a `Move` intent, and the candidate parser can express that same intent. Thus, if the original remains a `GrammarMiss` as expected for this range wording, `certify` returns `Ok` even though the source never states that `10` is in MB. The candidate's added unit is model-supplied semantics. The regex should require `MB` after both captured endpoints (case-insensitively), with a regression asserting that the unitless-first-bound variant is rejected.

Severity is P1 because the certificate authorizes a file-changing operation with a filter boundary the original did not explicitly unit. The strict `>`/`<` interpretation is already a documented limitation of the bounded range family; this finding is specifically the missing source unit.

## Review coverage and limits

- The contract comparison independently preserves move versus copy, source/destination roles, both numeric bounds, and the file-set filter. Its anchored expression rejects trailing clauses and reversed role order; literal validation blocks paths, unresolved one-word references/counts, and unquoted multiword prose. Quoted names preserve punctuation and reserved words.
- Folder creation is intentionally narrower than general folder requests: unquoted ambiguous or multiword names, unspecified names/counts, and paths are refused. That is conservative coverage, not a defect in this fallback certificate.
- Resize is limited to explicit positive pixel widths and parser-supported image subjects. It does not certify percent/quality, height, unit conversion, or filtered resize sources. These are documented limits.
- The unsupported-clause scanner is a denylist, not a general language understanding layer. The three anchored contracts plus exact `Intent` equality provide the main semantic boundary; users must not read this as proving arbitrary language equivalent. In particular, integration must retain the stated requirement that recognized schedules are handled before fallback and that unresolved schedule wording is never removed.
- `certify` is not an execution authorization. The harness reparses/plans, and the application must preserve scheduler, planner, preview, and confirmation gates. This review did not inspect protected holdout material or measure holdout outcomes.

## Disposition

Fix the missing lower-bound unit and add its regression before freezing this guard. I found no additional confirmed wrong-acceptance case in the bounded domains during this review. Confidence is bounded to the contracts and code paths described above; it is not a claim of complete natural-language coverage.
