# Voxi vocabulary packs

The vocabulary pack adds alternate wording for existing single commands. Voxi tries its built-in parser first. A grammar miss can match a vocabulary rule, whose expansion must then parse as a built-in command. Existing matches and hard refusals keep their original results.

## Add a rule

1. Edit `vocab.json` and increase its integer `version`. Keep `schema` at 1 and use `minAppVersion` for the lowest compatible release. Unstamped `0.0.0` builds accept any supported minimum version.
2. Give the rule a unique lowercase ID and an `intent` matching the built-in expansion, such as `images_to_pdf`. The allowlist permits reversible file actions and read-only queries. `trash` is excluded because it removes files, `undo` can reverse an unrelated operation, and `reveal` opens an external Finder window. `chain` and `recent_actions` are also excluded.
3. Anchor the case-insensitive regex with `^` and `$`. Outside captures, use literal words and whitespace, with non-capturing groups and alternation if needed. Give the pattern at least one numbered capture. Each capture covers one or more complete whitespace-delimited tokens and must leave quoted literals intact. Captures cannot nest or repeat as a group. Dictation normalization turns numbers such as “one” into digits.
4. Write `replace` with lowercase parser keywords and each numbered capture exactly once (`$1`, `$2`, etc.), separated by single spaces. Preserve the user's names, paths, types, numbers and filters. Type aliases such as “jpg” and “raw” use the built-in parser's extension lists. Quote names that would otherwise read as filter words. Include examples using the original command in `input` and the exact built-in expansion in `expect`.
5. Add the phrasing and expected intent to `vocab/corpus.txt`. The parser tests load it alongside the built-in corpus. Keep the pack cumulative so those phrases continue to work on updated installations.
6. Have the vocabulary key holder sign the exact JSON and commit the detached signature alongside it. The app checks that the signed trusted comment matches the pack's schema and version.

From the repository root, with `jq` and `minisign` installed.

```sh
schema=$(jq -r '.schema' vocab/vocab.json)
version=$(jq -r '.version' vocab/vocab.json)
minisign -S -s ~/.voxi-keys/vocab.key \
  -t "Voxi vocabulary schema=$schema version=$version" -m vocab/vocab.json
minisign -V -p vocab/vocab.pub -m vocab/vocab.json
(cd app && npm ci && npm run build)
(cd app/src-tauri && env -u VOXI_PLAN_MODEL_DIR -u VOXI_TINY_MODEL_DIR rustup run stable cargo test --locked --lib vocab::tests)
(cd app/src-tauri && env -u VOXI_PLAN_MODEL_DIR -u VOXI_TINY_MODEL_DIR rustup run stable cargo test --locked --lib plan::tests::messy::vocab_)
(cd app/src-tauri && env -u VOXI_PLAN_MODEL_DIR -u VOXI_TINY_MODEL_DIR rustup run stable cargo test --locked --lib parse::corpus_tests::corpus_commands_build_the_intended_intent)
```

New shortcuts and recurring tasks save the built-in expansion they were validated with. Scheduled one-off tasks use the actions approved in their preview. If an older repeating task saved alias text, review it and save it again to capture the intended expansion.

## Publish

Add two repository secrets under GitHub **Settings → Secrets and variables → Actions**. Set `VOCAB_SIGNING_KEY` to the contents of the encrypted vocabulary key file and `VOCAB_SIGNING_KEY_PASSWORD` to its password. Keep both files outside the repository.

The Vocabulary workflow validates PRs and can be run manually for validation. It publishes when a push to `main` changes `vocab/vocab.json`. PRs and publication both require a changed pack to advance beyond the base and published versions. CI signs and verifies the pack again, then updates the assets on the `vocab-latest` prerelease. If the words need to be reverted, publish the previous rules with a new, higher version.

Voxi loads the bundled and cached packs on a background thread at startup. It checks for downloads at most once per 24 hours. It verifies the signature, app compatibility and version before caching the pack and activating it in the running app. Local diagnostics record the active `vocab_version`.
