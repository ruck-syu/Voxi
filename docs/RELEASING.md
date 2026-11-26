# Releases

Every pull request and every push to `main` that touches the app runs
[`.github/workflows/release.yml`](../.github/workflows/release.yml).

## CI quality gate and publishing

On pull requests, the `quality` job:

- checks version agreement,
- builds the frontend,
- runs both Rust test suites,
- runs the rule-only cross-build equivalence suite in both feature sets, and
- runs strict Clippy in both feature sets.

On `main`, the release job requires that gate, assigns one version to every
manifest and lockfile, builds a universal app for Apple silicon and Intel with
`--no-default-features`, signs it, verifies signatures, effective entitlements,
and both CPU slices, and only then publishes:

- the `.dmg` for new installs,
- a signed update bundle and `latest.json` for in-app updates, and
- release notes listing every change since the previous version.

Pushes that only touch `website/` or `docs/` don't build a new app.

## Versioning

[`app/scripts/next-version.mjs`](../app/scripts/next-version.mjs) is the single source of truth for release gating, SemVer bumping, and release notes:

- **Guard:** pushes to `main` skip the release unless `app/` or `crates/` changed since the newest published `vX.Y.Z` tag (and the change is not marked `[skip release]`).
- **Automatic bump:** computed from PR titles (first line of GitHub merge commit bodies) and direct commits since the newest `vX.Y.Z` tag:
  - Default (any unprefixed or `fix:` / `chore:` / etc. title): **patch** (`0.0.3` → `0.0.4`)
  - `feat:` or `feat(scope):`: **minor** (`0.0.3` → `0.1.0`)
  - `<type>!:` (e.g. `feat!:`) or a `BREAKING CHANGE:` footer: **major** from 1.0 on; before 1.0 it bumps **minor** (`0.1.4` → `0.2.0`). Cut 1.0.0 with a manual `major` run.
  - `[skip release]` in the PR title or commit message: skips publishing on that push
- **Manual trigger:** running the workflow via `workflow_dispatch` always releases and accepts an optional `bump` input (`auto`, `patch`, `minor`, or `major`).
- **Monotonicity & failed runs:** release tags are only created when a release is published, so pull request runs and failed runs never burn a version.
- **Release notes:** generated once from the PR titles / commits since the previous tag and used for both the GitHub release body and the in-app updater's `latest.json` `notes` field.

The manifests in the repository stay at `0.0.0`, so a local build is never
mistaken for a release; only the release job writes the real version via
`node app/scripts/check-versions.mjs --set "$VERSION"`.

## Signing and notarization

Signing has two modes, chosen by which secrets exist:

- **Self-signed (current):** `VOXI_SIGNING_CERT` and
  `VOXI_SIGNING_CERT_PASSWORD` hold the "Voxi Self-Signed" certificate and its
  `.p12` password. The app keeps one identity across versions, so macOS
  remembers its folder access, but it is not notarized and Gatekeeper asks once
  on first launch.
- **Developer ID:** also set `APPLE_SIGNING_IDENTITY` and `APPLE_TEAM_ID` (the
  signing identity and team), and `APPLE_API_KEY`, `APPLE_API_ISSUER`, and
  `APPLE_API_KEY_P8` (notarization API credentials), and replace
  `VOXI_SIGNING_CERT` with the Developer ID certificate. The release is then
  notarized and stapled, and Gatekeeper acceptance is verified.

Setting only some of the Apple secrets fails the release. Both modes need
`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (update
signing). Everyday local, rule-based use needs none of them.
