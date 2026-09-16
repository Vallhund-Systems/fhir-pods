#!/usr/bin/env bash
# Build the FFI staticlib for the host and drive it from C — proves the ABI the
# way every native pod consumes it, with no Swift/Kotlin in the loop.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${TMPDIR:-/tmp}/fhir-pods-smoke"

echo "==> building host staticlib (release)"
cargo build -q -p fhir-pods-ffi --release

echo "==> compiling + linking c/smoke.c"
cc -std=c11 -Wall -Wextra \
   -I "$ROOT/crates/fhir-pods-ffi/include" \
   "$ROOT/crates/fhir-pods-ffi/c/smoke.c" \
   "$ROOT/target/release/libfhir_pods_ffi.a" \
   -o "$OUT"

"$OUT"
