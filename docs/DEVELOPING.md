# Developing Voxi

Voxi is a [Tauri](https://tauri.app) app: Rust for parsing and file work, React
for the window.

## Running locally

```bash
cd app && npm install
cd app && npm run tauri dev
```

Development builds run as `"Voxi Dev"` (`dev.voxi.app.dev`), with their own
settings, history, schedules, and login item, so they never touch an installed
Voxi on the same Mac. Give the dev build a different hotkey if both run at once.

`npm run dev` alone (from `app/`) serves the window in a browser with a stand-in
backend (`app/src/mock.ts`), useful for design work.

## Tests and parser evaluation

Tests cover one sentence per catalog row, safety rules, undo, and real runs of
`sips`, `ditto`, and PDFKit in a temporary folder:

```bash
cd app/src-tauri && cargo test --lib
```

The parser also runs a corpus of typed commands, one per line, in
`app/src-tauri/src/parse/corpus/`, and every line must pass.

Held-out sets in `app/src-tauri/eval/` measure how the parser handles phrasing
it wasn't tuned on:

```bash
cd app/src-tauri && HELDOUT=heldout-3 cargo test --lib heldout -- --ignored --nocapture
```

A set stops measuring accuracy once it has been used to fix something, so write
a new one before changing the parser.

### Converting wrong-result feedback to test candidates

Developers can turn local wrong-result reports into parser test candidates with
`scripts/feedback-to-corpus.mjs`, which also flags reports that parsed but
failed in planning.

## Experimental local plan model (`dev:model`)

The released product is **rule-only**. Release builds are compiled with
`--no-default-features`, so no model runtime or weights can enter a published
app regardless of the environment. This path exists only as an experimental
development opt-in via the `tiny-model` Cargo feature together with the
`VOXI_PLAN_MODEL_DIR` or `VOXI_TINY_MODEL_DIR` environment variables; neither
weights nor model accuracy are release requirements. With those variables unset,
the feature-enabled build produces the same public outcomes as the rule-only
build.

To launch the native development app with the trained structured-plan v4c model:

```bash
cd app && npm run dev:model
```

This opt-in build uses the local exported package at
`~/.local/share/voxi/experiments/plan-model-v4c-2026-11-17/native-model`; set
`VOXI_PLAN_MODEL_DIR` to use a different location for the same verified package.
The loader checks all three file hashes. Weights stay outside Git. Inference
runs locally on a worker.

Routing and safety rules:

- The default `VOXI_MODEL_ROUTING=fallback` tries the rules first. Eligible
  grammar misses and missing, unquoted transfer items that read like references
  can go to the model for a one-off plan.
- Set `VOXI_MODEL_ROUTING=legacy` to try the model before eligible rules (`0`,
  `false`, and `off` also select legacy mode).
- Hard refusals, shortcuts, and active result lists keep their existing paths.
- Trash commands and repeating tasks require a rule interpretation in both
  modes.
- All model plans go through path validation and confirmation, and their
  previews show a notice above the steps.

This model was enabled at the user's request despite the documented evaluation
errors; see the [training report](testing/plan-model-v4-2026-11-17/REPORT.md).

Run the model integration checks and report-only audit against the real local
package:

```bash
cd app/src-tauri
VOXI_PLAN_MODEL_DIR="$HOME/.local/share/voxi/experiments/plan-model-v4c-2026-11-17/native-model" VOXI_PLAN_AUDIT_OUT=/tmp/voxi-native-plan-audit.jsonl cargo test --locked --lib --features tiny-model structured_model_ -- --ignored --nocapture --test-threads=1
```

## Website

`website/` is a static site with no build step. Its look comes from the app
icon: paper, ink, and the red smile that doubles as a sound wave, which ripples
in the hero while the demo command is typed.

Serve it locally with:

```bash
python3 -m http.server -d website
```

In Vercel, import this repo and set **Root Directory** to `website`. Pushes that
only touch the website or docs don't build a new app.

## Releases

See [RELEASING.md](RELEASING.md) for the CI quality gate, versioning, and macOS
signing/notarization configuration.
