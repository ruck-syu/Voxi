#!/usr/bin/env bash
set -euo pipefail

# This runs before signing and, on pushes, inside the publisher's lock.
base="$1"
head="$2"
pack=vocab/vocab.json

if [[ "$base" != 0000000000000000000000000000000000000000 ]]; then
  git cat-file -e "$base^{commit}"
  if git diff --quiet "$base" "$head" -- "$pack"; then
    exit 0
  elif [[ "$?" != 1 ]]; then
    exit 1
  fi
fi
echo "pack=true" >> "$GITHUB_OUTPUT"

check_floor() {
  python3 -c '
import json, pathlib, sys

current = json.load(sys.stdin)["version"]
candidate = json.loads(pathlib.Path("vocab/vocab.json").read_bytes())["version"]
if type(current) is not int or current < 1 or type(candidate) is not int or candidate <= current:
    sys.exit(f"Vocabulary version {candidate} must exceed {sys.argv[1]} version {current}")
' "$1"
}

if git cat-file -e "$base:$pack" 2>/dev/null; then
  git show "$base:$pack" | check_floor base
fi

release=$(gh api --paginate "repos/$GITHUB_REPOSITORY/releases?per_page=100" \
  --jq '.[] | select(.tag_name == "vocab-latest") | [.prerelease, .draft] | @tsv')
if [[ -z "$release" ]]; then
  exit 0
fi
if [[ "$release" != $'true\tfalse' ]]; then
  echo "::error::vocab-latest must be a published prerelease"
  exit 1
fi
gh release download vocab-latest --repo "$GITHUB_REPOSITORY" --pattern vocab.json --output - | check_floor published
echo "VOCAB_RELEASE_EXISTS=true" >> "$GITHUB_ENV"
