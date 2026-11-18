# Rust fallback semantic certificate

`app/src-tauri/src/fallback_guard.rs` provides `certify(original, candidate)`.
It returns the candidate `Intent` only when the original is a true
`parse_classified` `GrammarMiss`, the candidate parses, unsupported clauses are
absent, and an independently extracted contract exactly equals the parsed
candidate. It returns a typed `Rejection` otherwise. Accepted rules and typed
hard refusals are terminal. The app must continue to route recognized schedules
through its scheduler before model fallback; this certificate also catches
unquoted recurrence wording if given a grammar miss, including unsupported
recurrences for which the scheduler extracted `None`.

## Supported domain

The certified domain is intentionally small:

* `make/create [folder(s)] NAME [and NAME ...] in/on PARENT`, with exact
  parsed names, order, and parent. A name may not contain a path separator.
  Quoted names are treated as literals by the clause scanner, while candidate
  acceptance still depends on the product parser supporting that exact form.
* A limited explicit pixel width form (`make NAME 1200 pixels wide`) that can
  be certified against the parser's resize intent. Width, source and image
  family must match exactly. Percent/quality, height, ranges and unit conversion
  are not supported.

* `move/copy files from SOURCE to DESTINATION between LOWER and UPPER MB`, with
  an optional explicit `MB` after either endpoint. Source and destination must
  each be one literal folder name, without path separators. Bounds must be
  finite, nonnegative and strictly increasing. Both parsed bounds, the file
  set, operation and exact source/destination roles must equal the extracted
  contract. This supports the F070 range rewrite without trusting the parser
  to prove that the original request's range survived.

Everything else is ineligible, including renames, text replacement, sorts,
other filters, PDFs, archives, cleanup, unresolved list references, scheduling,
multi-action chains, overwrite/replace-original behavior, sharpening, encryption
and exclusions. This is not a complete natural-language semantic parser. The
model may recover only paraphrases expressed inside these contracts.
Unsupported requests remain refused. No returned intent executes; scheduler,
planner, preview and confirmation are downstream responsibilities.

## Regression evidence

Rust unit tests in the module use the actual `parse_classified` entry point and
exercise the certificate. They cover the T035 folder recovery, the F070 bounded
move, operation and role swaps, dropped/changed bounds, added filters/actions,
invalid ranges, path scope, literal preservation, the 12 reviewed additions,
requested-action loss, S029 recurrence loss, resize percent/quality versus
pixel width, and changed numeric width. The 12 reviewed additions remain **1
accepted (T035)** and **11 rejected** (including the unsupported T036
nested-folder candidate).

The expanded guard was replayed through an isolated source/planner harness on
the same previously inspected 300 rows. It found **49 current `GrammarMiss`
rows**, accepting exactly **F070 and T035**; both are documented supported
misses and their saved rewrites parse to the independently extracted exact
intents. No other row was added, and none of the 11 reviewed bad additions or
the previously recorded 300-case dangerous additions regressed. This is a
bounded recovery result, not an overall accuracy estimate: labels/outcomes in
the 300-row development evidence are heuristic, and the other 47 grammar misses
are not claimed as a labeled supported-denominator. The independent bank was
not read or evaluated.

Case-level decisions, including each saved rewrite and the actual parser and
certificate results, are in [expanded-v2-300-results.jsonl](expanded-v2-300-results.jsonl).
Its SHA-256 is `0d5b4a9f58d40c8f1bb97bb2366565a66b239e4a822e999479f1d3fb02df9c3a`.
The concise replay summary is in
[expanded-v2-300-summary.json](expanded-v2-300-summary.json). The expanded Rust
source SHA-256 is
`3c34261431b46b345aeab6276d7bb7626ccacec9c6a91964be3f0c42ca80ff8b`.
The replay used saved model rewrites from the previous October 28 reference-date
experiment; this does not establish behavior for other model snapshots or dates.

The module itself has no Tauri dependency. To use it from an isolated native
CLI, compile it alongside the same `model` and `parse` modules (or an equivalent
crate exposing those types and `parse_classified`); no Python runtime is needed.

## Parent boundary hardening

Contracts require explicit folder-creation wording and exact literal boundaries.
Unquoted multiword prose, unresolved deictic references, bare numbers and
unspecified counts such as “some folders” are not certified literal names;
quoted names retain their exact spelling and punctuation. Size-range certification
requires an explicit MB unit and uses the existing larger-than/smaller-than filter
semantics (strict bounds). No model-generated unit is inferred from a unitless
source request. Filtered resize sources remain outside this bounded domain.

### Shared range-unit notation

The public command reference explicitly supports “between 10 and 50 MB”. The
upper MB unit applies to both endpoints, as in the existing Rust filter parser;
writing “between 10 MB and 50 MB” yields the same bounds. A wholly unitless
range is refused, and mixed-unit ranges remain outside this certificate. This
clarifies “explicit MB bounds”: a separate lower-unit token is not required.
