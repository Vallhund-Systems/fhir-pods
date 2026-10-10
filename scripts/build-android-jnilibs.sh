#!/usr/bin/env bash
# Build the fhir-pods FFI shared lib for Android (arm64 devices + the x86_64
# emulator) into the expo pod's jniLibs. No cargo-ndk needed: cargo just has to
# be pointed at the NDK's clang wrappers.
#
#   ANDROID_HOME=~/Library/Android/sdk ./scripts/build-android-jnilibs.sh
#   → pods/expo/android/src/main/jniLibs/{arm64-v8a,x86_64}/libfhir_pods_ffi.so
# FEATURES="host-profiles" builds the host's own compiled profiles in (scripts/gen-host-profiles.sh).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/pods/expo/android/src/main/jniLibs"
API="${ANDROID_API:-24}"                      # Expo's minSdk
ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"

# Newest installed NDK unless ANDROID_NDK_HOME is set.
NDK="${ANDROID_NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)}"
HOST_TAG="$(ls "$NDK/toolchains/llvm/prebuilt")"
BIN="$NDK/toolchains/llvm/prebuilt/$HOST_TAG/bin"
[ -x "$BIN/aarch64-linux-android$API-clang" ] || { echo "NDK clang not found under $BIN" >&2; exit 1; }
echo "==> NDK $(basename "$NDK"), API $API"

echo "==> ensuring Android targets"
rustup target add aarch64-linux-android x86_64-linux-android >/dev/null

# 16 KB page alignment: required for Android 15+ (Google Play, targetSdk 35+).
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"

build() { # <rust target> <abi> <clang triple prefix>
  local target="$1" abi="$2" triple="$3"
  local var="CARGO_TARGET_$(echo "$target" | tr '[:lower:]-' '[:upper:]_')_LINKER"
  echo "==> $abi ($target)"
  env "$var=$BIN/$triple$API-clang" cargo build -q -p fhir-pods-ffi --release --target "$target" ${FEATURES:+--features "$FEATURES"}
  mkdir -p "$DEST/$abi"
  cp "$ROOT/target/$target/release/libfhir_pods_ffi.so" "$DEST/$abi/"
  echo "    $(wc -c < "$DEST/$abi/libfhir_pods_ffi.so" | tr -d ' ') bytes"
}

build aarch64-linux-android arm64-v8a aarch64-linux-android
build x86_64-linux-android  x86_64    x86_64-linux-android

echo "==> wrote $DEST"
