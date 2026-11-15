#!/usr/bin/env bash
# Checks that the Emscripten build (run under Node) produces byte-identical
# reports to the native build for the Lua projects, and (with the Playdate SDK and built pdex.elf files)
# for the C projects: the WASM build runs them on pdsim-thumb, the native one on Unicorn. Exit 0 only if all match.
set -euo pipefail
cd "$(dirname "$0")/.."
# A skip must be visible in CI logs, not silent.
skipped() {
  echo "SKIPPED (no SDK): $1"
  [ -z "${GITHUB_ACTIONS:-}" ] || echo "::warning::wasm parity SKIPPED (no SDK): $1"
}
projects=(examples/lua-stress calibration/bench-lua)
# C games need the SDK's C_API headers and a built Source/pdex.elf.
if [ -n "${PLAYDATE_SDK_PATH:-}" ] && [ -d "$PLAYDATE_SDK_PATH/C_API" ]; then
  for p in examples/c-stress calibration/bench-c; do
    if [ -f "$p/Source/pdex.elf" ]; then projects+=("$p"); else skipped "$p has no Source/pdex.elf"; fi
  done
else
  skipped "C projects (PLAYDATE_SDK_PATH not set)"
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cargo build --release -p pdsim-cli
scripts/wasm-node.sh build --release -p pdsim-cli --no-default-features --features thumb

fail=0
for p in "${projects[@]}"; do
  name=$(basename "$p")
  cpu_native=(); cpu_wasm=()
  if [ -f "$p/Source/pdex.elf" ]; then cpu_native=(--cpu unicorn); cpu_wasm=(--cpu thumb); fi
  target/release/pdsim run "$p" --frames 300 -q ${cpu_native[@]+"${cpu_native[@]}"} --report "$tmp/$name-native.json"
  node target/wasm32-unknown-emscripten/release/pdsim.js run "$p" --frames 300 -q ${cpu_wasm[@]+"${cpu_wasm[@]}"} --report "$tmp/$name-wasm.json"
  if ! cmp -s "$tmp/$name-native.json" "$tmp/$name-wasm.json"; then
    echo "parity FAILED: $p"
    diff "$tmp/$name-native.json" "$tmp/$name-wasm.json" | head -20 || true
    fail=1
  fi
done
[ "$fail" = 0 ] || exit 1
echo "parity OK: ${#projects[@]} projects"
