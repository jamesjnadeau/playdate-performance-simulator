#!/usr/bin/env bash
# Runs cargo for wasm32-unknown-emscripten, with NODERAWFS so the result runs
# under Node against the real filesystem. Usage: scripts/wasm-node.sh test -p pdsim-lua
set -euo pipefail
if ! command -v emcc >/dev/null 2>&1; then
  # shellcheck disable=SC1091
  source "${EMSDK:-$HOME/emsdk}/emsdk_env.sh" >/dev/null
fi
EMCC_CFLAGS="-sNODERAWFS=1" exec cargo "$@" --target wasm32-unknown-emscripten
