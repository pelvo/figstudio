#!/usr/bin/env bash
# Golden check for the MCP tools: record every tool's reply at <rev> and at the working
# tree, then diff them. An empty diff means the MCP output did not change.
#
#   scripts/mcp-golden.sh <base-rev>       e.g. scripts/mcp-golden.sh HEAD
#
# The base is a temporary worktree under target/golden-base. The working tree is this
# checkout. Both record into target/golden-{base,head}. Needs FIGSTUDIO_TOOLCHAIN (default 1.99.0).
set -euo pipefail
cd "$(dirname "$0")/.."

rev="${1:?usage: scripts/mcp-golden.sh <base-rev>}"
tc="${FIGSTUDIO_TOOLCHAIN:-1.99.0}"
root="$PWD"
base_tree="$root/target/golden-base"
base_out="$root/target/golden-base-out"
head_out="$root/target/golden-head-out"

cleanup() {
  git worktree remove --force "$base_tree" 2>/dev/null || true
}
trap cleanup EXIT

run_golden() {
  local dir="$1" out="$2"
  rm -rf "$out"
  (cd "$dir" && FIGSTUDIO_GOLDEN_DIR="$out" cargo +"$tc" test --release \
    -p figstudio-mcp --test golden -- --ignored >/dev/null)
}

step() { printf '\n== %s\n' "$*"; }

cleanup
mkdir -p target
step "base worktree at $rev"
git worktree add --detach "$base_tree" "$rev" >/dev/null
# The base tree may lack the harness (older rev): copy it in so both sides run the same test.
cp "$root/apps/figstudio-mcp/tests/golden.rs" "$base_tree/apps/figstudio-mcp/tests/golden.rs"

step "record base ($rev)"
run_golden "$base_tree" "$base_out"
step "record working tree"
run_golden "$root" "$head_out"

step "diff -r base working tree"
if diff -r "$base_out" "$head_out"; then
  echo "golden: identical ($(find "$base_out" -name '*.json' | wc -l | tr -d ' ') records)"
else
  echo "golden: DIFFERENT (see above)" >&2
  exit 1
fi
