## Skill routing

When the user's request matches an available skill, invoke it via the Skill tool. Route only to skills in the session's available-skills list; answer directly for quick questions or small scoped edits.

Key routing rules:
- Product ideas/brainstorming → invoke /office-hours
- Strategy/scope → invoke /plan-ceo-review
- Architecture → invoke /plan-eng-review
- Design system/plan review → invoke /design-consultation or /plan-design-review
- Full review pipeline → invoke /autoplan
- Bugs/errors → fix them directly: one read-only check of the cause, or a focused failing test first when the cause is unclear; then fix it and run that test
- QA/testing site behavior → invoke /qa or /qa-only
- Code review/diff check → invoke /review
- Visual polish → invoke /design-review
- Building UI → once it renders, invoke /design-review --build <preview url> (opt-in)
- Ship/deploy/PR → invoke /ship (merge + deploy: the user types /land-and-deploy)
- Save progress → invoke /context-save
- Resume context → invoke /context-restore
- Author a backlog-ready spec/issue → invoke /spec

## Bug fixes

For every user-reported failure, add a planner test in `app/src-tauri/src/plan/messy_tests.rs` that reproduces the folder layout. If grammar was involved, also add a parser corpus case.

## Testing

Run the same gate as the `quality` job in `.github/workflows/release.yml`. Use rustup stable for Cargo (`rustup run stable cargo …`), because Homebrew's `rustc` may come first on PATH and CI uses the latest stable.

- Frontend, from `app/`: `npm ci && node scripts/check-versions.mjs && node --test scripts/*.test.mjs && npm run lint && npm run test:finder-handoff && npm run build`
- Command requests, from the repo root: `node --test .github/scripts/command-requests.test.mjs`
- Rust, from `app/src-tauri/`, each with `env -u VOXI_PLAN_MODEL_DIR -u VOXI_TINY_MODEL_DIR`:
  - `cargo test --locked --lib` and `cargo test --locked --lib --features tiny-model`
  - `cargo test --locked --test rule_only_equivalence`, with and without `--features tiny-model`. Run both `--lib` suites first, in the same target directory: the equivalence test reads their builds and fails on a missing or stale one.
  - `cargo fmt --check`
  - `cargo clippy --locked --all-targets -- -D warnings`, with and without `--features tiny-model`
