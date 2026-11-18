# Voxi production-readiness audit

Original audit 2026-11-17 on arm64 macOS, branch `fix-login-item`, HEAD `8dcf441`.
This revision (also 2026-11-17) re-checks the completed implementation in the
same uncommitted working tree against the code and the gate recorded in
[`docs/testing/production-release-validation.md`](testing/production-release-validation.md).
The release product is RULE-ONLY. `tiny-model` is an optional experimental
development feature; model accuracy or shipping weights are not release
requirements.

This is a source-and-command audit, not a signed-app certification. Checks that
need Apple credentials, an installed build, an Intel Mac, or a person at the
keyboard are marked **BLOCKED-OWNER** with the action required; they are not
claimed as PASS. The T10 gate was run locally with `CARGO_TARGET_DIR=/tmp/voxi-target-final`.
No commit, stash, reset, checkout, credential inspection, release upload,
permission change, or login-item mutation was performed.

## Reproduction and observed commands

Rust commands were run from `app/src-tauri`, frontend commands from `app`.
Verbatim output and environment are in the validation record. All Cargo commands
used `CARGO_TARGET_DIR=/tmp/voxi-target-final`. Toolchain: rustc/cargo 1.96.0,
Node 26.0.0, npm 11.12.1, arm64, macOS 27.0.1 (26A434). Release CI pins Node 22
and floating stable Rust.

| Check | Observed result |
|---|---|
| `cargo test --lib --locked` | Exit 0; 334 passed, 0 failed, 18 ignored; 3.37 s. |
| `cargo test --lib --locked --features tiny-model` | Exit 0; 350 passed, 0 failed, 23 ignored; 3.32 s. Extra ignored test needs real weights. |
| `env -u VOXI_PLAN_MODEL_DIR -u VOXI_TINY_MODEL_DIR cargo test --locked --test rule_only_equivalence` | Exit 0; 3 passed, 0 failed, 0.87 s. |
| Same with `--features tiny-model` | Exit 0; 3 passed, 0 failed, 0.84 s. |
| `cargo clippy --all-targets --locked -- -D warnings` | Exit 0, no diagnostics. |
| Same with `--features tiny-model` | Exit 0, no diagnostics. |
| `npm run build` | Exit 0; `tsc && vite build`; 33 modules; 273.91 kB main JS / 85.59 kB gzip; 111 ms. |
| `node scripts/test-permissions.mjs` | Exit 0; 38 checks passed; manual VoiceOver/keyboard walkthrough still required. |
| `cargo bench --bench search` with `VOXI_BENCH_OUTPUT` | Exit 0; 26 JSON records; planner p50/p95 recorded in validation record. |
| `node scripts/check-versions.mjs` | Exit 0; `Versions agree: 0.1.0`. |

Ignored tests are report-only audits (plus one real-weight model check in the
feature suite); neither suite proves those scenarios. Native bundling,
notarization, Intel execution, update installation, and browser/device
interaction were not run here.

## Production checklist

**PASS** means the narrowly stated criterion is evidenced in source or by the
gate above. **FAIL** means a concrete unmet criterion. **UNKNOWN** means an
artifact, measurement, or interactive test is still required but no owner action
has been assigned. **BLOCKED-OWNER** means only the owner can complete the check;
the exact action is given. A source PASS does not imply a runtime PASS.
Performance thresholds are proposed release acceptance bars, not all measured
results.

### Build, release, CI, and Rust

| ID | Status | Concrete production criterion | Evidence and finding |
|---|---|---|---|
| B01 | PASS | Frontend typecheck and production compilation succeed. | `npm run build` exit 0 above; `app/package.json:9`. |
| B02 | PASS | Default rule-only library suite passes. | 334 passed / 18 ignored above. |
| B03 | PASS | Experimental feature remains compilable and its ordinary tests pass. | 350 passed / 23 ignored above. Does not certify model quality. |
| B04 | PASS | Package, Cargo, and app versions agree in source and artifact. | `scripts/check-versions.mjs` asserts package, npm lock, npm lock root, Cargo, Cargo lock, and Tauri versions are equal; exit 0. `release.yml:99-103` sets one version via `--set`; `:175-177` reasserts it on the built `Info.plist`. Artifact re-check by owner. |
| B05 | PASS | Public releases fail closed unless signed by a valid Apple Developer ID Application identity. | `release.yml:105-135` requires 9 secrets, asserts `APPLE_SIGNING_IDENTITY == "Developer ID Application: … (TEAM_ID)"`, imports into a private keychain, and greps `find-identity`. Credential validity is owner-required (see OWNER-INPUT). |
| B06 | PASS | Release workflow notarizes and staples app/DMG, then verifies Gatekeeper before publishing. | `release.yml:148-203`: notarytool submit + Accepted assertion, stapler staple/validate, `spctl --assess` on app and DMG; publish is the first step allowed to create a release (`:237-244`). |
| B07 | BLOCKED-OWNER | Signed app has hardened runtime and a reviewed, minimal effective entitlement set. | Source enforces it: `tauri.conf.json:53` `hardenedRuntime: true`; `release.yml:163-169` requires the `runtime` flag and asserts empty effective entitlements; `:187` runs `spctl`. Owner must run `codesign -d --verbose=4 --entitlements :- Voxi.app` on the downloaded artifact and confirm. |
| B08 | PASS | Updater requires signatures and uses an HTTPS manifest endpoint. | `tauri.conf.json:55-63` enables updater artifacts, embeds pubkey, HTTPS GitHub releases; `app/src/updates.ts:41-58` uses the plugin with error handling. |
| B09 | BLOCKED-OWNER | Published update signature matches the embedded key; valid update installs, tampered update fails. | `release.yml:209-226` signs and verifies the updater payload and manifest presence. Owner must run the end-to-end install and tamper-rejection test with the real private key. |
| B10 | PASS | Release configuration builds both Apple silicon and Intel slices. | `release.yml:95-96` installs both targets; `:157` builds `--target universal-apple-darwin`; `:183-185` asserts both slices in the binary. |
| B11 | BLOCKED-OWNER | Packaged universal app contains both slices and works on supported Intel/arm64 macOS versions. | Owner must run `lipo -archs` on the artifact and launch/preview/confirm/undo on Intel and arm64 at the minimum supported macOS 12. |
| C01 | PASS | Every PR gates on default tests, feature tests without weights, strict Clippy, TS, and frontend build; release depends on those gates. | `release.yml:3-4` `pull_request` trigger; `quality` job (`:27-65`) runs version check, `npm run build`, both lib suites, equivalence in both feature sets, and strict Clippy in both feature sets; `release` job `needs: quality` (`:68`). Owner must mark `quality` required in branch protection. |
| R01 | PASS | `cargo clippy --all-targets --locked -- -D warnings` succeeds. | Exit 0 in both feature sets above, with no diagnostics. |
| R02 | PASS | Default production code has no unintended dead model helpers; intentional diagnostic code is scoped explicitly. | `lib.rs:4-11` gates `fallback_guard`, `fallback_slots`, `tiny_model`, `plan_model` behind `cfg(test)` or `feature="tiny-model"`; strict Clippy is clean in both feature sets. |
| R03 | PASS | Non-test lock/IO/environment failure cannot abort a file operation or silently bypass recovery. | `lib.rs:56-69` `checked_state!` converts poisoned locks into errors; `lib.rs:109` `home()` falls back, `:1001` rejects a missing home at startup; `outside_click.rs:19` uses `let Ok(...)`. Release keeps `panic="abort"` (`Cargo.toml:53`) but production file-op paths no longer unwrap locks/IO/env. Static regex expects remain programmer invariants, not user-IO panics. |

### Data safety and security

| ID | Status | Concrete production criterion | Evidence and finding |
|---|---|---|---|
| D01 | PASS | Settings replacement is atomic and durable. | `settings.rs:123-142` `atomic_write`: same-dir private temp, write, `sync_all`, macOS `F_FULLFSYNC` (`:105-119`), rename, directory `sync_all`; all failures propagate. |
| D02 | PASS | Corrupt settings/history are preserved, reported, recoverable. | `settings.rs:47-58` `preserve_store` writes `.corrupt-<timestamp>` backups; `:72-93` `load_store` emits a warning and sets `write_blocked`; history uses it at `history.rs:41-44`. Tests `storage_corruption_backup_future_rejection_and_read_errors`, `corrupt_history_preserved_future_rejected_missing_and_unreadable_distinct`. |
| D03 | PASS | History and schedules use durable atomic replacement and propagate save errors. | `history.rs:80-86` and `scheduler.rs:618-624` both call `settings::save_store` → `atomic_write`; `History::try_push` (`history.rs:59-68`) returns the error, and `execute_pending` reports it instead of promising durable undo. |
| D04 | PASS | Unreadable schedules are preserved, scheduling disabled, interrupted work not replayed. | `scheduler.rs:575-605`, `:587-599`: backup on load failure, store disabled, one-time interrupted job failed, repeating job advanced; tests `unreadable_schedule_data_is_preserved_without_blocking_startup`, `schedule_io_failures_disable_scheduling_without_overwriting_data`, `repeating_tasks_survive_reload_and_interrupted_runs`. |
| D05 | PASS | Explicit schema versions, validated migrations, safe rejection of future versions. | `settings.rs:12` `STORE_VERSION = 1`, `:60-70` `decode_store` accepts legacy unversioned JSON and rejects unknown versions, `:176-179` writes a version envelope. Tests `legacy_history_round_trip`, `legacy_schedule_migration_round_trip`, `storage_corruption_backup_future_rejection_and_read_errors`. |
| D06 | PASS | A crash or partial tool failure leaves durable evidence and a safe recovery/undo route. | `operation_journal.rs:106-148` write-ahead `run` syncs intent, in-flight index, and each inverse; `:69-78` `recover` imports known steps and refuses to replay; `:82-104` `acknowledge` archives exact bytes. Tests `crash_boundaries_never_replay_mutations`, `returned_failures_finish_and_allow_the_next_operation`, `review_keeps_journal_when_history_cannot_be_saved`; `result_context_tests::partial_failure_keeps_durable_undo_and_allows_next_command`. |
| D07 | PASS | No operation can clobber a destination created after preview/check. | `fsx.rs:257-270` `rename_exclusive` claims directories via `create_dir` and files via `hard_link`; `:282-341` `Staging`/`staged_output` revalidate parent dev/ino before publish. Tests `rename_and_restore_reject_destinations_created_after_preview`, `staged_publication_cannot_clobber`, `staged_publication_rejects_parent_swap`. |
| D08 | PASS | Unzip enforces entry, byte, ratio, time, and disk limits before/during extraction. | `exec.rs:142-157` `ArchiveLimits` (10,000 entries, 1 GiB/file, 5 GiB total, 100:1 ratio, 256 MiB reserve); `:162-221` strict header inspection; `:226-255` extracted-member verification; `:256-295` private snapshot, 60 s budgets, disk checks, ditto into staging. Tests `system_zip_limits_and_disk_margin`, `hostile_zip_metadata_is_rejected`. |
| S01 | PASS | Frontend capabilities confined to the command window, no broad shell/filesystem grant. | `capabilities/default.json:5-14`: main window only, core/window, updater, restart, open-dialog. Custom Rust commands remain a privileged surface with backend guards. |
| S02 | PASS | Restrictive baseline CSP with no remote script or unsafe-eval. | `tauri.conf.json:32-34`: `default-src 'self'`, explicit style/font/image origins, no `unsafe-eval`. Fonts are bundled; remote font origins are defense-in-depth cleanup, not a finding. |
| S03 | PASS | User commands do not become shell source or PATH-selected executors. | `exec.rs:78-140` `bounded_tool` uses `Command`/argv with absolute tool paths; no shell interpolation; fixed HTTPS link at `lib.rs:920`. |
| S04 | PASS | External tools have bounded runtime/output, cancellation, and cleanup. | `exec.rs:76-140` enforces deadlines, kills/reaps, bounds captured stdout/stderr, and detects cancellation; extraction shares a 60 s budget (`exec.rs:287-295`). Tests `tools_have_deadlines_cancellation_and_bounded_errors`, `normal_tool_exit_leaves_descendants_and_does_not_wait_for_pipe_eof`. |
| S05 | PASS | Baseline path validation rejects traversal and resolves symlinks before operations. | `fsx.rs` canonicalization/`revalidate_parent` and `exec.rs` rechecks; tests `rejects_parent_refs_and_outside_paths`, `moving_and_copying_a_symlink_to_a_folder_moves_the_link`. Race guarantee assessed in S06. |
| S06 | PASS | Filesystem mutation stays confined on parent/symlink swaps; archive members cannot escape. | `exec.rs:162-221` rejects absolute/parent/`.`/duplicate/backslash/colon/NUL member names, symlink modes, path/link extra fields, depth > 64; `:226-255` canonicalizes each extracted member and rejects symlinks and outside-root paths; `fsx.rs:315-323` revalidates parent identity at publish. Tests `hostile_zip_metadata_is_rejected`, `staged_publication_rejects_parent_swap`. |

### Rule-only policy, frontend, performance, operations

| ID | Status | Concrete production criterion | Evidence and finding |
|---|---|---|---|
| L01 | PASS | Ordinary release excludes local inference regardless of model env vars. | `Cargo.toml:16-21` optional dep, no default feature; release build `--no-default-features` (`release.yml:157`) with dependency-graph and bundled-model-file guards (`:151-155`, `:178-181`); `app/scripts/dev-model.mjs` opts in only for dev. Both release env vars are empty (`:23-24`). |
| L02 | PASS | Feature-enabled app with both model env vars unset has outcomes identical to rule-only. | `tests/rule_only_equivalence.rs::serialized_public_outcomes_match_without_models` passed in both feature sets with `env -u`; fixture covers refusals, invalid/oversized input, schedule, ambiguity, partial failure, side effects, History and journal (`lib.rs:1181-1213`). |
| F01 | PASS | Frontend TypeScript is strict and production compilation includes TS. | `app/tsconfig.json:18-21`, `package.json:9`; `npm run build` exit 0. |
| F02 | PASS | Command/settings/update flows represent visible failures and update waits are bounded. | `app/src/updates.ts:29-58` check timeout 20 s, download 10 min, catch + error state + `updater_failed` diagnostic; permission errors shown in `App.tsx:1740-1762`. |
| F03 | PASS | Permission request/open-settings failures are caught and shown with a retry path. | `App.tsx:1740-1762` catches both promises, sets error states, renders **Retry** and **Open System Settings**; `scripts/test-permissions.mjs` passes 38 checks including rejection/denial/retry and focus restoration. |
| F04 | PASS | Command labeling, explicit preview confirmation, Esc cancellation, IME-aware Enter, keyboard settings tabs. | Source evidence in `App.tsx` (via `example_tests`, label/status/tab markup); narrow source-level criterion. Interactive verification is F05. |
| F05 | BLOCKED-OWNER | Keyboard-only and VoiceOver users complete onboarding, ambiguity, confirm/undo, settings, and read errors at 200% zoom with reduced motion. | Labels/status/roles exist, but no interactive verification here. `test-permissions.mjs` explicitly reports "Manual VoiceOver/keyboard walkthrough still required." Owner must perform the walkthrough. |
| P01 | PASS | Walker caps depth/entries/time and reports incomplete/cancelled searches. | `search.rs` limits (depth 12, 50,000 entries/tree, 10 s); tests `limits_and_cancellation_are_explicit`; bench shows explicit "Search incomplete" at 60k files and depth 14. Per-tree cap is not a global cross-root budget. |
| P02 | PASS | Rule input and shortcut expansion have a documented, backend-enforced size budget. | `parse/mod.rs:48` `MAX_COMMAND_BYTES = 16_384`, checked at `:110-114`; `settings.rs:260` `check_shortcut_commands` enforces ≤8 commands and summed ≤16 KiB; tests `command_byte_budget_*`, `raw_shortcuts_are_bounded_before_trimming_or_normalizing`; frontend mirrors the cap (permissions script). |
| P03 | BLOCKED-OWNER | Packaged cold launch to ready menu-bar/input has p95 ≤ 2 s on arm64 and Intel. | No packaged cold-start measurement here; the bench measures planner workloads. Owner must record ≥20 cold launches per architecture with hardware details. |
| P04 | BLOCKED-OWNER | Rule preview p95 ≤ 250 ms at 1k files and ≤ 1 s at 10k local files. | Benchmark binary p95 is 2.53 ms (flat-1k group) and 26.65 ms (flat-10k group), but that is not the packaged app. Owner must measure ≥30 previews per workload in the installed app on both architectures. |
| O01 | PASS | Persistent diagnostics rotate by size/count, survive GUI launches, omit PII. | `diagnostics.rs:1-7` retention comment, `:21-26` three 1 MiB files, `:187-255` 0700 dir / 0600 files with ownership and symlink checks, `:78-89` Event has no string/path fields. Tests `rotation_is_bounded_and_private`, `every_entry_point_reduces_sensitive_errors_before_rotation`, `unwritable_sink_and_full_queue_never_fail_the_operation`. |
| O02 | PASS | Rule planning/file execution needs no hosted LLM and transmits no commands for inference. | Optional native dep (`Cargo.toml:21`) excluded from release; parser/executor local; updater is explicit network feature (`updates.ts:35-58`); feedback local (`learn.rs`). Not a packet-capture certification. |
| U01 | PASS | First-run flow explains and requests standard protected-folder access with usage strings and retry. | `Info.plist:8-15` four folder usage descriptions; `App.tsx:1777-1836` allow/skip/check-again/retry; `lib.rs:944-949` triggers TCC prompts during onboarding. Real TCC prompts remain owner-observable. |
| U02 | PASS | Permission help distinguishes folder grants from Full Disk Access with least-privilege guidance. | `App.tsx:1807-1810` names Files and Folders and explains FDA only for protected locations outside Desktop/Documents/Downloads; permissions script asserts both phrasings and "FDA is not required by default". |
| U03 | PASS | Development and App Translocation copies do not register as login owner. | `lib.rs:928-934` `owns_login_item` requires installed bundle path and rejects AppTranslocation; save/startup gated at `:844` and `:1010-1012`. |
| U04 | PASS | Login preference reflects actual OS registration; failed save rolls back. | `lib.rs:828-832` reads OS state in `get_settings`; `:843-860` reads before change, verifies after, and rolls back on save failure; `settings.rs:182-208` `login_transaction` restores OS and disk sides. Test `save_failure_restores_login_registration_and_preference`. |
| U05 | BLOCKED-OWNER | Installed app launches headlessly at login, stays menu-bar-only, survives logout/reboot/upgrade/duplicate/revoke-regrant. | `Info.plist:6-7` LSUIElement and U03 guard are source evidence only. Owner must run the clean-account lifecycle checklist. |
| A01 | PASS | README promises match safety, recovery, scheduling, experimental status, permissions, diagnostics, and install behavior. | `README.md` revised: no-clobber/undo with crash and journal limits, 200-entry history, recurring interruption vs one-time failure, Mark as reviewed, Files and Folders vs FDA, 16 KiB/8-command caps, rule-only release with dev `tiny-model` opt-in, signed/notarized install, nine secrets, diagnostics location/retention. Every command example was checked against corpus, `examples.json`, or a Rust parser/scheduler test literal; `list items in Downloads` was corrected to `list files in Downloads`. |

## Prioritized work plan — T1–T10 status

Each task's source implementation is complete and gated unless noted. Files are
repo-relative. Remaining items are owner-only and tracked above.

| Task | Status | Scope and residual work |
|---|---|---|
| T1 — release/versions | **Source DONE; owner remaining** | Developer ID signing, notarization, staple, Gatekeeper, universal build, one-version script implemented in `release.yml` + `scripts/check-versions.mjs`. Owner: configure 9 secrets and verify the signed artifact (B07, B09, B11). |
| T2 — durable versioned stores | **DONE** | `settings.rs` shared atomic/versioned store with `.corrupt-` backups; history and scheduler migrated to it (D01–D03, D05). |
| T3 — safe filesystem and bounded tools | **DONE** | `exec.rs`, `fsx.rs`, `pdf.rs`, `trash.rs`: no-replace claims, staging, `ArchiveLimits`, bounded tools (D07, D08, S04, S06). |
| T4 — PR checks | **Source DONE; owner remaining** | `quality` job added to `release.yml` with both feature sets, equivalence, Clippy, version check, build. Owner: mark it required in branch protection (C01). |
| T5 — journal/lock/error/login integration | **DONE** | `operation_journal.rs`, `checked_state!`, `login_transaction`, OS readback; equivalence test green (D06, R03, U04, L02). |
| T6 — permissions and accessible frontend | **Source DONE; owner remaining** | `App.tsx`, `api.ts`, `mock.ts`, `test-permissions.mjs` catch/retry/FDA copy (F03, U02). Owner: VoiceOver/keyboard walkthrough (F05). |
| T7 — bounded rule input | **DONE** | `parse/mod.rs` 16 KiB cap, shortcut aggregate cap, tests (P02). |
| T8 — private rotating diagnostics | **DONE** | `diagnostics.rs`, 3×1 MiB, 0600/0700, codes + op IDs only, redaction tests (O01). |
| T9 — Clippy/dead-code cleanup | **DONE** | Feature-gated helpers and mechanical lint fixes; strict Clippy clean in both feature sets (R01, R02). |
| T10 — docs and evidence | **Source DONE; owner remaining** | README corrected, this audit updated, validation record created with gate and bench. Owner: record artifact/update/accessibility/hardware/lifecycle evidence for the BLOCKED-OWNER items. |

## OWNER-INPUT and exact configuration

No secrets were read or requested in this audit. The workflow now references all
nine names; presence and validity are unknown here and block B07/B09/B11 and the
release itself.

| Need | Exact input | Configuration and acceptance |
|---|---|---|
| Developer ID distribution | Active Apple Developer Program account; **Developer ID Application certificate with private key exported as password-protected .p12**, its password, identity, and Team ID. | Secrets `VOXI_SIGNING_CERT`, `VOXI_SIGNING_CERT_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_TEAM_ID`. `release.yml:111-135` fails if absent/mismatched and restricts the keychain to the imported identity. Validate the chain and Team ID on the downloaded artifact. |
| Notarization | App Store Connect notarization API `.p8` key, Key ID, Issuer ID. | Secrets `APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_P8` (`:81-82`, `:109`, `:125`, `:135`); notarytool submit/staple/validate enforced at `:189-203`. Requires Accepted status and staple validation on app and DMG. |
| Updater identity | Tauri updater private key matching the embedded pubkey, and its password (empty only if intentionally unencrypted). | Secrets `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (`:83-84`). Valid signed install and tamper rejection must be demonstrated. |
| Release and branch protection | Owner/admin permission to require checks and publish releases. | Built-in `GITHUB_TOKEN` with `contents: write` (`:72-73`); mark the `quality` PR check required and confirm workflow permissions. |
| Hardware/accessibility validation | Intel and arm64 Macs, clean test account, supporting advertised macOS range. | Record app version, OS/hardware, codesign/TCC/login/update results, cold-launch and preview p50/p95, and lifecycle results in `docs/testing/production-release-validation.md`. Do not infer them from universal target configuration. |

## Final result

**48 checklist items: 41 PASS / 0 FAIL / 0 UNKNOWN / 7 BLOCKED-OWNER.**
No source-level FAIL remains. The seven BLOCKED-OWNER items (B07, B09, B11, F05,
P03, P04, U05) all need owner credentials, installed artifacts, hardware, or a
person; passing unit tests do not waive them. The release should not be signed
off for general distribution until they are checked and recorded.

Remaining gates, in priority order:

1. **P0 — Trusted distribution is unverified end to end (B07, B09, B11).** The
   workflow enforces signing, notarization, stapling, entitlements, slices and
   updater signatures, but the real credentials and a downloaded artifact must be
   verified, and the updater tested with valid and tampered payloads.
2. **P1 — No enforceable required PR check (C01).** The `quality` job exists and
   passes locally; the owner must mark it required in branch protection.
3. **P1 — Performance and accessibility are unmeasured on target hardware
   (F05, P03, P04).** Record cold launches, previews, and the walkthrough on Intel
   and Apple Silicon; the benchmark is a planner baseline only.
4. **P1 — Installed login/permission lifecycle is unverified (U05).** Exercise
   logout/reboot, disable, upgrade, duplicate copy, and revoke/regrant on a clean
   account.

Model evaluation errors do not block the agreed rule-only release. The default
feature exclusion stays intact: release builds use `--no-default-features` and
the workflow rejects any model runtime or weight file in the bundle.
