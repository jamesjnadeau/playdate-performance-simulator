#!/usr/bin/env bash
# Checks that the Emscripten build (run under Node) produces byte-identical
# reports to the native build for the Lua projects. Exit 0 only if all match.
set -euo pipefail
cd "$(dirname "$0")/.."
projects=(examples/lua-stress calibration/bench-lua)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cargo build --release -p pdsim-cli
scripts/wasm-node.sh build --release -p pdsim-cli --no-default-features

fail=0
for p in "${projects[@]}"; do
  name=$(basename "$p")
  target/release/pdsim run "$p" --frames 300 -q --report "$tmp/$name-native.json"
  node target/wasm32-unknown-emscripten/release/pdsim.js run "$p" --frames 300 -q --report "$tmp/$name-wasm.json"
  if ! cmp -s "$tmp/$name-native.json" "$tmp/$name-wasm.json"; then
    echo "parity FAILED: $p"
    diff "$tmp/$name-native.json" "$tmp/$name-wasm.json" | head -20 || true
    fail=1
  fi
done
[ "$fail" = 0 ] || exit 1
echo "parity OK: ${#projects[@]} projects"
