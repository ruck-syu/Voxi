# Voxi vocabulary packs

The vocabulary pack adds alternate wording for existing single commands. Voxi tries its built-in parser first. A grammar miss can match a vocabulary rule, whose expansion must then parse as a built-in command. Existing matches and hard refusals keep their original results.

## Add a rule

1. Edit `vocab.json` and increase its integer `version`. Keep `schema` at 1 and use `minAppVersion` for the lowest compatible app version. The source tree uses `0.0.0` for unstamped builds, so a pack bundled and tested from this tree needs to support that version.
2. Give the rule a unique lowercase ID. Write a case-insensitive regex anchored with `^` and `$` for the whole normalized command. Dictation normalization turns numbers such as “one” into digits.
3. Write `replace` with lowercase parser keywords and numbered captures (`$1`, `$2`, etc.) separated by single spaces. Capture names, paths, extensions and numbers from the input. Include examples with the original command in `input` and the exact expanded command text in `expect`.
4. Add the phrasing and expected intent to `vocab/corpus.txt`. The parser tests load it alongside the built-in corpus. Keep the pack cumulative so those phrases continue to work on updated installations.
5. Have the vocabulary key holder sign the exact committed JSON. Commit the detached signature alongside it so local and release builds can verify their offline fallback.

From the repository root:

```sh
minisign -S -s ~/.voxi-keys/vocab.key -t 'Voxi vocabulary' -m vocab/vocab.json
minisign -V -p vocab/vocab.pub -m vocab/vocab.json
(cd app && npm ci && npm run build)
(cd app/src-tauri && env -u VOXI_PLAN_MODEL_DIR -u VOXI_TINY_MODEL_DIR rustup run stable cargo test --locked --lib vocab::tests)
(cd app/src-tauri && env -u VOXI_PLAN_MODEL_DIR -u VOXI_TINY_MODEL_DIR rustup run stable cargo test --locked --lib parse::corpus_tests::corpus_commands_build_the_intended_intent)
```

## Publish

Add two repository secrets under GitHub **Settings → Secrets and variables → Actions**. Set `VOCAB_SIGNING_KEY` to the contents of the encrypted vocabulary key file and `VOCAB_SIGNING_KEY_PASSWORD` to its password. Keep both files outside the repository.

The Vocabulary workflow validates PRs and publishes on pushes to `main` that touch this directory. It can also run manually from `main`. Publishing requires a version higher than the published one. CI signs and verifies the pack again, then updates the assets on the `vocab-latest` prerelease. If the words need to be reverted, publish the previous rules with a new, higher version.

Voxi checks on startup and at most once per 24 hours. It verifies the signature, app compatibility and version before caching the pack and activating it in the running app. Local diagnostics record the active `vocab_version`.
