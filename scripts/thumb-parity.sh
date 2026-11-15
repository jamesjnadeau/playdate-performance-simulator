#!/usr/bin/env bash
# Checks that the pure-Rust Thumb interpreter (`--cpu thumb`) produces
# byte-identical reports to Unicorn (`--cpu unicorn`) for the C projects.
# Needs PLAYDATE_SDK_PATH (for pd_api.h) and the built ELFs; skips (exit 0)
# without them, or fails if PDSIM_REQUIRE_ELFS=1.
set -euo pipefail
cd "$(dirname "$0")/.."
projects=(examples/c-stress calibration/bench-c)

missing=0
for p in "${projects[@]}"; do
  [ -f "$p/Source/pdex.elf" ] || missing=1
done
[ -n "${PLAYDATE_SDK_PATH:-}" ] || missing=1
if [ "$missing" = 1 ]; then
  echo "thumb parity skipped: no ELFs (set PLAYDATE_SDK_PATH and build them)"
  [ "${PDSIM_REQUIRE_ELFS:-}" = 1 ] && exit 1
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cargo build --release -p pdsim-cli --features thumb

fail=0
for p in "${projects[@]}"; do
  name=$(basename "$p")
  for cpu in unicorn thumb; do
    target/release/pdsim run "$p" --frames 300 -q --cpu "$cpu" \
      --data-dir "$tmp/data-$name-$cpu" --report "$tmp/$name-$cpu.json"
  done
  if ! cmp -s "$tmp/$name-unicorn.json" "$tmp/$name-thumb.json"; then
    echo "thumb parity FAILED: $p"
    diff "$tmp/$name-unicorn.json" "$tmp/$name-thumb.json" | head -20 || true
    fail=1
  fi
done
[ "$fail" = 0 ] || exit 1
echo "thumb parity OK: ${#projects[@]} projects"
