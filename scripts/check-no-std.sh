#!/usr/bin/env bash
# Prove fhir-pods-core is genuinely no_std: build it for a bare-metal target that
# has no operating system at all. If this compiles, nothing in the core touches
# std — the thesis ("no I/O, no OS coupling") is enforced by the compiler.
#
# A library crate needs no allocator or panic handler to build for a no_std
# target (the final artifact supplies those), so this is a pure purity check.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="thumbv7em-none-eabi"
cd "$ROOT"

echo "==> host build (no_std attribute active outside tests)"
cargo build -p fhir-pods-core

echo "==> ensuring bare-metal target: $TARGET"
rustup target add "$TARGET" >/dev/null

echo "==> bare-metal build"
cargo build -p fhir-pods-core --target "$TARGET"

echo "OK: fhir-pods-core builds with no operating system."
