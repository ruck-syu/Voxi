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

Versions are `0.2.N`: each release is one past the newest published `v0.2.N`
tag, so pull request runs and failed runs never use up a number. Change `SERIES`
in the workflow to start a new line.

The manifests in the repository stay at `0.0.0`, so a local build is never
mistaken for a release; only the release job writes the real version.

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
