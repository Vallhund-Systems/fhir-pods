#!/usr/bin/env bash
# Build the wasm pod's artifact: the FFI crate for wasm32-unknown-unknown,
# size-optimised, shrunk further with wasm-opt when it is on PATH.
#
#   ./scripts/build-wasm.sh
#   → pods/wasm/dist/fhir_pods.wasm
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/pods/wasm/dist"
SRC="$ROOT/target/wasm32-unknown-unknown/release/fhir_pods_ffi.wasm"

echo "==> ensuring wasm32 target"
rustup target add wasm32-unknown-unknown >/dev/null

echo "==> building (release, opt-level z, lto, panic=abort)"
cargo build -q -p fhir-pods-ffi --release --target wasm32-unknown-unknown

mkdir -p "$DIST"
if command -v wasm-opt >/dev/null 2>&1; then
  echo "==> wasm-opt -Oz"
  wasm-opt -Oz --strip-debug --strip-producers "$SRC" -o "$DIST/fhir_pods.wasm"
else
  echo "==> wasm-opt not on PATH (brew install binaryen to shrink further); copying as built"
  cp "$SRC" "$DIST/fhir_pods.wasm"
fi

echo "==> wrote $DIST/fhir_pods.wasm ($(wc -c < "$DIST/fhir_pods.wasm" | tr -d ' ') bytes; gzip: $(gzip -9c "$DIST/fhir_pods.wasm" | wc -c | tr -d ' '))"
