#!/usr/bin/env bash
# Reports the browser build's .wasm size and fails if the gzipped size is over
# budget. Usage: scripts/wasm-size.sh [path/to/pdsim_web.wasm]
# The budget (bytes, gzip -9) can be overridden with WASM_GZIP_BUDGET.
set -euo pipefail

wasm="${1:-app/dist/pdsim_web.wasm}"
# 2 MiB: the size the browser download should stay under (docs/wasm-port.md).
budget="${WASM_GZIP_BUDGET:-2097152}"

if [[ ! -f "$wasm" ]]; then
  echo "no such file: $wasm (run 'npm run build:web' in app/ first)" >&2
  exit 1
fi
raw=$(stat -c %s "$wasm")
gz=$(gzip -9 -c "$wasm" | wc -c)
pct=$((gz * 100 / budget))
line="pdsim_web.wasm: $raw bytes, $gz gzipped ($pct% of the $budget-byte budget)"
echo "$line"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  echo "### wasm size" >>"$GITHUB_STEP_SUMMARY"
  echo "$line" >>"$GITHUB_STEP_SUMMARY"
fi
if ((gz > budget)); then
  echo "::error::pdsim_web.wasm is $gz bytes gzipped, over the $budget-byte budget" >&2
  exit 1
fi
