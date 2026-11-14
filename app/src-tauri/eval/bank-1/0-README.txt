# Bank 1: large held-out paraphrase bank
#
# Status: spent. Its misses chose the rewrite rules; passing lines moved to src/parse/corpus/11-bank-1.txt, and what is left here is the backlog.
#
# Each `.txt` file groups variants under `# seed: <command>`. Every variant means
# exactly what its seed means, so it carries the seed's expected intent (or `!`).
# Run: `HELDOUT=bank-1 cargo test --lib heldout -- --ignored --nocapture`.
