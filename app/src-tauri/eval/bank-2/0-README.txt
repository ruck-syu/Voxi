# Bank 2: everyday lingo
#
# Status: partly spent. Its chuck/toss/throw/zap and "trash can" misses chose the
# rules in src/parse/rewrite.rs, and its vids/clips/footage and screenies/screen
# grabs/screencaps misses chose type words in src/parse/phrase.rs, and its
# gimme/lemme/pls misses chose filler rules in src/parse/rewrite.rs; those lines
# moved to src/parse/corpus/12-bank-2.txt.
# The rest have not been used to choose fixes, so they still measure how Voxi
# handles slang it was never tuned on ("yeet", "nuke", "chonky", "gimme",
# "ctrl z", British "bin" and "bung", texting shorthand like "pls" and "rn").
#
# Each `.txt` file groups variants under `# seed: <command>`. Every variant means
# exactly what its seed means, so it carries the seed's expected intent.
# Run: `HELDOUT=bank-2 cargo test --lib heldout -- --ignored --nocapture`.
# Once a miss is used to pick a fix, mark this bank spent, as with bank-1.
