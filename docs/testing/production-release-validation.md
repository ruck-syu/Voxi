# Production release validation — 2026-11-17

This is the T10 evidence record for branch `fix-login-item`, HEAD `8dcf441`, including
the uncommitted working tree. It records the locally reproducible gate and the rule
planner baseline. It is **not** a signed-artifact certification: every check that
needs Apple credentials, an installed build, an Intel Mac, or a person at the
keyboard is listed as OWNER-REQUIRED at the end and remains unchecked.

No commit, push, stash, reset, checkout, or restore was performed. No code was
edited for this task; only `README.md`, `docs/production-readiness.md`, and this
file were written. Every Cargo command was run from `app/src-tauri` with
`CARGO_TARGET_DIR=/tmp/voxi-target-final`. No command was killed for hanging.

## Environment

| Item | Value (verbatim) |
|---|---|
| Date | 2026-11-17 |
| `sw_vers` | `ProductName: macOS`, `ProductVersion: 27.0.1`, `BuildVersion: 26A434` |
| `rustc -V` | `rustc 1.96.0 (ac68faa20 2026-05-25) (Homebrew)` |
| `cargo -V` | `cargo 1.96.0 (30a34c682 2026-05-25) (Homebrew)` |
| `node -v` | `v26.0.0` |
| `npm -v` | `11.12.1` |
| `uname -m` | `arm64` |

The release workflow pins Node 22 and floating stable Rust; the versions above
are the local gate machine, not the CI toolchain. Both model environment
variables were unset for the equivalence runs (`env -u VOXI_PLAN_MODEL_DIR -u
VOXI_TINY_MODEL_DIR`). The release workflow keeps them empty globally
(`.github/workflows/release.yml:23-24`) and unsets them explicitly for its
no-weights test steps (`:47-59`); release builds exclude the model feature.

## Gate results (verbatim)

### 1. `CARGO_TARGET_DIR=/tmp/voxi-target-final cargo test --lib --locked`

Exit 0. Header and final result lines, verbatim (350+ individual `test ... ok`
lines omitted here for length; the run output was captured in full at execution):

```
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.83s
     Running unittests src/lib.rs (/tmp/voxi-target-final/debug/deps/voxi_lib-987e56a4becac17a)

running 352 tests
...
test result: ok. 334 passed; 0 failed; 18 ignored; 0 measured; 0 filtered out; finished in 3.37s
```

The 18 ignored tests in the default suite are report-only parser/planner audits
(e.g. `combo_audit::combo_audit`, `parse::corpus_tests::haiku_300_per_case_report`,
`plan::tests::extension_audit_planner_preview_controls`); a passing run does not
exercise them.

### 2. `CARGO_TARGET_DIR=/tmp/voxi-target-final cargo test --lib --locked --features tiny-model`

Exit 0. Final result line, verbatim:

```
test result: ok. 350 passed; 0 failed; 23 ignored; 0 measured; 0 filtered out; finished in 3.32s
```

The extra ignored test is `tiny_model::tests::trained_model_recovers_into_a_preview_without_changing_files`,
which requires the trained local model and `VOXI_TINY_MODEL_DIR`. No weights were
present or used.

### 3. Rule-only cross-build equivalence, both feature sets

Both lib suites above were built first so the integration test could register the
exact test executables, as documented in `tests/rule_only_equivalence.rs`.

`env -u VOXI_PLAN_MODEL_DIR -u VOXI_TINY_MODEL_DIR CARGO_TARGET_DIR=/tmp/voxi-target-final cargo test --locked --test rule_only_equivalence`

```
   Compiling voxi v0.1.0 (/Users/vishalshekhar/Documents/dev/Voxi/app/src-tauri)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 5.04s
     Running tests/rule_only_equivalence.rs (/tmp/voxi-target-final/debug/deps/rule_only_equivalence-021407af65ac8d42)

running 3 tests
test missing_variant_has_actionable_build_instructions ... ok
test registration_selects_exact_artifact_and_rejects_stale_binary ... ok
test serialized_public_outcomes_match_without_models ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.87s
```

`env -u VOXI_PLAN_MODEL_DIR -u VOXI_TINY_MODEL_DIR CARGO_TARGET_DIR=/tmp/voxi-target-final cargo test --locked --test rule_only_equivalence --features tiny-model`

```
   Compiling voxi v0.1.0 (/Users/vishalshekhar/Documents/dev/Voxi/app/src-tauri)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 5.63s
     Running tests/rule_only_equivalence.rs (/tmp/voxi-target-final/debug/deps/rule_only_equivalence-8e76572e2e483de0)

running 3 tests
test missing_variant_has_actionable_build_instructions ... ok
test registration_selects_exact_artifact_and_rejects_stale_binary ... ok
test serialized_public_outcomes_match_without_models ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.84s
```

The differential test (`tests/rule_only_equivalence.rs::serialized_public_outcomes_match_without_models`)
runs `lib.rs::result_context_tests::rule_only_probe` in two separately built
binaries, removes both model env vars, and compares serialized public outcomes,
filesystem side effects, History and the operation journal. Its fixture covers
hard refusals, empty/invalid/oversized input, a one-time schedule, ambiguity,
a partial execution failure, and the resulting side effects.

### 4. Strict Clippy, both feature sets

`CARGO_TARGET_DIR=/tmp/voxi-target-final cargo clippy --all-targets --locked -- -D warnings`

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.24s
CLIPPY_DEFAULT_EXIT=0
```

`CARGO_TARGET_DIR=/tmp/voxi-target-final cargo clippy --all-targets --locked --features tiny-model -- -D warnings`

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.34s
CLIPPY_FEATURE_EXIT=0
```

Both exit 0 with no lint diagnostics. These cover the library, integration test,
and benchmark targets in both feature sets.

### 5. Frontend build and permission checks

`cd app && npm run build && node scripts/test-permissions.mjs`

The build emitted a non-fatal npm config warning and completed:

```
npm warn Unknown user config "allow-scripts". This will stop working in the next major version of npm. See `npm help npmrc` for supported config options.

> app@0.1.0 build
> tsc && vite build

vite v8.3.5 building client environment for production...
transforming...
✓ 33 modules transformed.
rendering chunks...
computing gzip size...
...
dist/assets/index-CcmA6D85.js                                          273.91 kB │ gzip: 85.59 kB

✓ built in 111ms
---BUILD_EXIT=0---
```

The permission script output, verbatim:

```
ok   request() catches a rejected requestFolderAccess
ok   request() sets a readable error state
ok   Retry button is rendered and calls request()
ok   error regions use role="alert" and aria-live
ok   focus moves to the recovery control on failure
ok   openPrivacySettings is awaited
ok   open-settings failure sets an error state
ok   Files and Folders path is shown
ok   Privacy & Security path is shown
ok   Full Disk Access is explained
ok   FDA is not required by default
ok   FDA scoped to locations outside Desktop/Documents/Downloads
ok   errorMessage helper is exported
ok   mock scenario permission-reject
ok   mock scenario permission-denied
ok   mock scenario permission-flaky
ok   mock scenario settings-fail
ok   focus style for error message
ok   MAX_COMMAND_BYTES is 16,384 in the UI
ok   command box measures UTF-8 bytes
ok   exact backend refusal message is used
ok   oversized command is never submitted
ok   command message is an alert linked by aria-describedby
ok   shortcut expansions are capped at eight commands
ok   shortcut expansion bytes are summed against the cap
ok   shortcut save is disabled while over the cap
ok   Remove uses a separate canRemove flag, not canSave
ok   canRemove validates the list without this shortcut
ok   shortcut message is an inline alert
ok   empty length messages are hidden, not truncated text
ok   mock refuses oversized commands
ok   mock refuses oversized shortcut expansions
ok   bundle contains "Full Disk Access"
ok   bundle contains "Files and Folders"
ok   bundle contains "Retry"
ok   bundle contains "check folder access"
ok   bundle contains "open System Settings"
ok   bundle contains "That command is too long."
Permissions checks passed (38 checks). Manual VoiceOver/keyboard walkthrough still required.
PERMISSIONS_EXIT=0
```

### 6. Rule planner baseline

`CARGO_TARGET_DIR=/tmp/voxi-target-final cargo bench --bench search`

The bench harness requires `VOXI_BENCH_OUTPUT`; without it the first invocation
failed with `thread 'main' panicked at benches/search.rs:120:53: set VOXI_BENCH_OUTPUT: NotPresent`
(exit 101). The successful rerun used
`VOXI_BENCH_OUTPUT=/tmp/voxi-bench-results.json`. The harness emits no timings to
stdout; it writes 26 JSON records (2 warmups, 15 repeats per measurement) to that
file. Cargo output, verbatim (repeated Cargo `output filename collision` warnings
for the four `voxi_lib` artifact formats removed here):

```
   Compiling voxi v0.1.0 (/Users/vishalshekhar/Documents/dev/Voxi/app/src-tauri)
    Finished `bench` profile [optimized] target(s) in 53.60s
     Running benches/search.rs (/tmp/voxi-target-final/release/deps/search-aa2066d890a7629f)
Benchmarking flat-1k (1000 files)
Benchmarking flat-10k (10000 files)
Benchmarking nested-10k (10000 files)
Benchmarking nested-40k (40000 files)
Benchmarking budget-60k (60000 files)
BENCH_EXIT=0
```

Rule planner baseline, p50/p95 in milliseconds computed from the JSON samples by
linear interpolation. These measure the production parser + walker + planner in a
benchmark binary against temporary fixtures — **not** packaged-app UI latency or
cold start. `expected/observed` are the planner's action counts (p50).

| Fixture | Operation | expected / observed | Outcome | p50 ms | p95 ms |
|---|---|---|---|---:|---:|
| flat-1k | walk | 1000 / 1000 | walk | 1.533 | 1.659 |
| flat-1k | group | 10 / 10 | plan | 2.344 | 2.534 |
| flat-1k | exact | 1 / 1 | plan | 1.653 | 1.754 |
| flat-1k | normalized | 1 / 1 | ambiguous | 1.880 | 2.014 |
| flat-1k | missing | 0 / 0 | not found | 46.692 | 47.180 |
| flat-10k | walk | 10000 / 10000 | walk | 19.302 | 19.617 |
| flat-10k | group | 100 / 100 | plan | 26.046 | 26.654 |
| flat-10k | exact | 1 / 1 | plan | 20.227 | 20.516 |
| flat-10k | normalized | 1 / 1 | ambiguous | 22.965 | 23.427 |
| flat-10k | missing | 0 / 0 | not found | 68.577 | 72.133 |
| nested-10k | walk | 10000 / 10000 | walk | 14.596 | 15.363 |
| nested-10k | group | 100 / 0 | matches only in subfolders, left alone | 19.577 | 20.695 |
| nested-10k | exact | 1 / 1 | plan | 15.189 | 16.271 |
| nested-10k | normalized | 1 / 1 | ambiguous | 18.102 | 19.195 |
| nested-10k | missing | 0 / 0 | not found | 64.261 | 65.922 |
| nested-40k | walk | 40000 / 40000 | walk | 58.479 | 60.319 |
| nested-40k | group | 400 / 0 | matches only in subfolders, left alone | 78.131 | 80.371 |
| nested-40k | exact | 1 / 1 | plan | 61.731 | 63.921 |
| nested-40k | normalized | 1 / 1 | ambiguous | 71.637 | 73.462 |
| nested-40k | missing | 0 / 0 | not found | 121.342 | 124.598 |
| budget-60k | walk | 60000 / 49455 | walk (entry cap reached) | 72.318 | 74.980 |
| budget-60k | group | 600 / 0 | incomplete: entry limit reached | 94.745 | 97.722 |
| budget-60k | exact | 1 / 0 | incomplete: entry limit reached | 75.744 | 76.734 |
| budget-60k | normalized | 1 / 0 | incomplete: entry limit reached | 76.039 | 77.056 |
| budget-60k | missing | 0 / 0 | incomplete: entry limit reached | 75.409 | 76.665 |
| depth-14 | group | 1 / 0 | incomplete: depth limit reached | 0.326 | 0.344 |

Notable behavior, not a defect: the `nested-*` group rows report zero actions
because group moves/copies deliberately take only direct children of the named
folder and leave subfolder matches alone; the 50,000-entry and depth-12 caps
produce explicit "Search incomplete" reasons at 60k files and 14 levels. The
slowest row is an absent filename on 40k files (p95 124.6 ms), still within the
proposed 250 ms rule-preview bar, but this is a benchmark binary, not the app.

## OWNER-REQUIRED validation (cannot be performed here)

These are deliberately unchecked. The owner must complete and record each one
before release signoff; passing unit tests do not waive them.

- [ ] Configure all **nine GitHub Actions secrets** consumed by
      `.github/workflows/release.yml`: `VOXI_SIGNING_CERT`,
      `VOXI_SIGNING_CERT_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_TEAM_ID`,
      `APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_P8`,
      `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
      Confirm the Developer ID Application identity matches
      `APPLE_SIGNING_IDENTITY` and the Team ID matches `APPLE_TEAM_ID` on the
      downloaded artifact; confirm the updater public key corresponds to the
      private key.
- [ ] Mark the `quality` job / PR check as **required** in branch protection, and
      confirm repository workflow permissions allow the release to publish.
- [ ] **Signed-artifact verification** on the built and downloaded app/DMG:
      `codesign --verify --deep --strict --verbose=2 Voxi.app`,
      `codesign -d --verbose=4 --entitlements :- Voxi.app` (confirm hardened
      runtime flag and empty/minimal effective entitlements),
      `xcrun stapler validate Voxi.app` and the DMG,
      `spctl --assess --type execute --verbose=4 Voxi.app`,
      `lipo -archs .../MacOS/voxi` (must show `arm64` and `x86_64`).
- [ ] **Updater end-to-end**: a valid signed update installs and restarts
      successfully, and a tampered update is rejected by the embedded public key.
- [ ] **Accessibility walkthrough** by a person: VoiceOver plus keyboard-only
      navigation, at 200% zoom and with reduced motion, covering onboarding,
      ambiguity choices, preview confirm/undo, all settings panes, and error
      states.
- [ ] **Hardware performance**: at least **20 cold launches** and **30 previews
      per workload** on both Intel and Apple Silicon, recording p50/p95, app
      version, OS build, CPU, RAM, file counts, and storage type (SSD/HDD). Cold
      launch to ready menu-bar/input should meet p95 ≤ 2 s; rule preview p95
      ≤ 250 ms at 1k files and ≤ 1 s at 10k. The benchmark above is not a
      substitute.
- [ ] **Installed-app lifecycle on a clean account**: logout and reboot,
      enable/disable Start at Login, in-place upgrade, a duplicate copy of the
      app, and permission revoke/regrant — verifying OS registration readback,
      no second login agent, recovery notices, and that file access keeps working
      after each step.
