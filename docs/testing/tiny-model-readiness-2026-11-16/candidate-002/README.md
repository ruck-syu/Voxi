# Candidate 002 literal slot binding

## API

`voxi_lib::fallback_slots::prepare(original)` returns a `PreparedRewrite` only
when `parse_classified(original)` is exactly `GrammarMiss`, there is no typed
hard refusal or unsupported clause, and the source is inside the existing
folder-creation certificate. The caller sends `PreparedRewrite::model_input`
to native `NativeRewriter::rewrite` and passes that same result to
`PreparedRewrite::finish(Result<String, String>)`. `finish` applies the normal
semantic certificate to the bound request and model output, verifies every
alias/role/multiplicity, then returns the parsed typed `Intent` with exact
source literal values restored. A model error or failed check returns an error;
the source contract alone never creates an Intent.

Aliases are virtual per-request ordinary vocabulary tokens (`Archive`,
`Backup`, `Desktop`, etc.). Selection avoids case-insensitive collisions with
the complete source and other aliases. Original user values are stored
byte-exactly and restored after alias-output certification. Comma separators
are normalized to `and` only in a list whose members are independently parsed
as quoted literals. Straight and curly quote delimiters are supported; inner
punctuation, apostrophes, Unicode and case are retained in the binding.

This candidate currently binds only folder names and their parent folder.
Folder-list preparation requires quoted members, so counts, unspecified names,
deictic references, paths and unquoted prose remain unsupported. Scheduling,
extra actions, exclusions, and other unsupported clauses remain terminal.
Accepted rule results and typed hard refusals never enter the API. The normal
scheduler, planner, preview and confirmation remain downstream.

## Development inference evidence

`native-folder-list-recovery.jsonl` records three actual native Q8 rewrites for
the inspected development cases H011, H041 and H076. All 3/3 recover through
the source API contract (the corresponding API fixtures assert exact prepared
inputs and feed the captured native outputs through `finish`). The native
outputs required no value repair: they exactly named the generated aliases.
The three cases are all the quoted multi-folder cases among the four supported
grammar misses in the previously inspected 120-row bank 001. H050 is an unzip
destination case outside this folder-list hypothesis and remains rejected.
Thus the development comparison is 3/4 versus candidate 001's 0/4; this is
not independent readiness evidence, and bank 001 is not held out.

The original 300-case guard evidence remains a regression baseline: its 49
grammar misses included exactly the already certified F070 and T035, with no
other additions. Candidate 002 does not broaden the guard contracts for those
rows; focused guard tests still cover all 12 reviewed additions (T035 only is
accepted), F070 shared-MB-unit behavior, operation/role/bound changes and
extra clauses. See `../guard/README.md` and its immutable case reports. The
implementation-specific Rust tests exercise actual `parse_classified`, prepare
and complete, alias omission/duplication/role swaps, unsupported clauses,
Unicode exactness, parser priority and native-result fixtures.

## Source and runtime identity

At the recorded implementation snapshot:

* `fallback_slots.rs`: SHA-256 `4c7bd462a967376c5f9c7a97bee962d8febd9e5d7d8fd9b9b75cd4f5c18ea54a`
* `fallback_guard.rs`: SHA-256 `e952ada828179c651463bfa5ba6b2144829cf6cde338e92c6f66cff2c916901d`
* Native executable: SHA-256 `e18208f4ab4250aff646b5239bd4b55b19de3db77cf1118b0524c63b4b9b17ed`
* Q8 weights: SHA-256 `10ee3c175bf71a77f9104cb6293416721bc81a174b97a1e6dc52715543986245`

The pinned model package remains the 18,985,400-byte package recorded in
`../native/candidate-manifest.json`; this host preprocessing change does not
alter weights, tokenizer, exporter or native runtime. These values identify the
development run and must be refreshed if source changes.
