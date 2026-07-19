#!/usr/bin/env bash
# Convenience script: build pid-limited as wasm32-unknown-unknown so the rig
# can load it. Run from anywhere; resolves paths relative to this script.

set -euo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)"
CELLS_DIR="${SCRIPT_DIR}/../../cells"

cd "${CELLS_DIR}"

CELL="${1:-pid-limited}"
CELL_SYMBOL="${CELL//-/_}"

echo "[build-wasm] building ${CELL} for wasm32-unknown-unknown ..."
cargo build --release --no-default-features --target wasm32-unknown-unknown -p "${CELL}"

OUT="${CELLS_DIR}/target/wasm32-unknown-unknown/release/${CELL_SYMBOL}.wasm"
echo "[build-wasm] artefact: ${OUT}"
ls -la "${OUT}"
