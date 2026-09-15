#!/usr/bin/env bash
# Build the fhir-pods FFI static lib for iOS (device + simulator) and assemble an
# XCFramework the expo pod vendors. Requires Xcode + the iOS Rust targets.
#
# Run on macOS with Xcode installed:
#   ./scripts/build-ios-xcframework.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="fhir-pods-ffi"
LIB="libfhir_pods_ffi.a"
HEADERS="$ROOT/crates/fhir-pods-ffi/include"
DEST="$ROOT/pods/expo/ios/Frameworks"
XCF="$DEST/FhirPodsFFI.xcframework"

echo "==> ensuring iOS targets"
rustup target add aarch64-apple-ios aarch64-apple-ios-sim >/dev/null

echo "==> building device + simulator slices (release)"
cargo build -p "$CRATE" --release --target aarch64-apple-ios
cargo build -p "$CRATE" --release --target aarch64-apple-ios-sim

echo "==> assembling XCFramework"
rm -rf "$XCF"
mkdir -p "$DEST"
xcodebuild -create-xcframework \
  -library "$ROOT/target/aarch64-apple-ios/release/$LIB"     -headers "$HEADERS" \
  -library "$ROOT/target/aarch64-apple-ios-sim/release/$LIB" -headers "$HEADERS" \
  -output "$XCF"

echo "==> wrote $XCF"
