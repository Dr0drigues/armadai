#!/bin/sh
# Proposes a refreshed built-in tier table as a diff. Writes nothing in the repo.
set -eu
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

HOME="$tmp" ARMADAI_CONFIG_DIR="$tmp" cargo run -q -p armadai -- models sync
ARMADAI_CONFIG_DIR="$tmp" cargo run -q -p armadai-core --example propose_models -- "$(date -u +%F)" > "$tmp/models.json"

# Both sides go through the same normalizer so only values differ in the diff.
python3 -m json.tool --sort-keys crates/armadai-core/data/models.json > "$tmp/current.norm"
python3 -m json.tool --sort-keys "$tmp/models.json" > "$tmp/proposed.norm"
diff -u "$tmp/current.norm" "$tmp/proposed.norm" || true

echo
echo "Nothing was written. Check every id above against the vendors' official pages"
echo "(list in docs/wiki/models.md) before copying it into crates/armadai-core/data/models.json."
