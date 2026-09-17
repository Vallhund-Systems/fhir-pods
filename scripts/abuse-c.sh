#!/usr/bin/env bash
# Every native ABI entrypoint, memory-valid hostile inputs, and allocation order.
# Optional SANITIZE=1 instruments the C caller; it does not instrument Rust.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$(mktemp -d "${TMPDIR:-/tmp}/fhir-pods-abuse.XXXXXX")"
trap 'rm -rf "$OUT"' EXIT
cd "$ROOT"
cargo build -q -p fhir-pods-ffi --release
FLAGS=(-std=c11 -Wall -Wextra -Werror)
if [[ "${SANITIZE:-0}" == 1 ]]; then
  FLAGS+=(-fsanitize=address,undefined -fno-omit-frame-pointer)
fi
LINK=()
if [[ "$(uname -s)" == Linux ]]; then LINK+=(-ldl -lpthread -lm); fi
cc "${FLAGS[@]}" -I crates/fhir-pods-ffi/include crates/fhir-pods-ffi/c/abuse.c \
  target/release/libfhir_pods_ffi.a "${LINK[@]}" -o "$OUT/abuse"
python3 - "$OUT/abuse" <<'PY'
import json
import subprocess
import sys
result = subprocess.run([sys.argv[1]], capture_output=True, timeout=60)
sys.stderr.buffer.write(result.stderr)
if result.returncode:
    raise SystemExit(f"C ABI harness failed: exit {result.returncode}")
lines = result.stdout.splitlines()
if not lines:
    raise SystemExit("No output documents were checked")
for i, line in enumerate(lines):
    value = json.loads(line.decode('utf-8', errors='strict'))
    if not isinstance(value, dict):
        raise SystemExit(f"Output {i} is not a JSON object")
print(f"Checked UTF-8 and JSON syntax for all {len(lines)} ABI result buffers.")
PY
