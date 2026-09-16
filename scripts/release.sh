#!/usr/bin/env bash
# Build every artifact from a clean state and pack the pods into dist/ with
# checksums. Local only (this repo has no remote by design): the tarballs are
# what a host app vendors.
#
#   ./scripts/release.sh            # → dist/<version>/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/crates/fhir-pods-core/Cargo.toml" | head -1)"
DIST="$ROOT/dist/$VERSION"

echo "==> fhir-pods $VERSION"
[ -z "$(git -C "$ROOT" status --porcelain)" ] || { echo "working tree not clean" >&2; exit 1; }
# The clean-room wall: the repo must never mention the host app's names. The
# pattern is assembled so this script itself does not trip it.
WALL="$(printf '%s|%s|%s' "aver""care" "avatars""global" "local""agent")"
git -C "$ROOT" grep -Il -iE "$WALL" -- . >/dev/null 2>&1 && { echo "clean-room check failed" >&2; exit 1; }

echo "==> tests"
(cd "$ROOT" && cargo test -q --workspace >/dev/null)
"$ROOT/scripts/check-no-std.sh" >/dev/null
"$ROOT/scripts/smoke-c.sh" >/dev/null

echo "==> artifacts"
"$ROOT/scripts/build-wasm.sh"
(cd "$ROOT/pods/wasm" && node --test test/*.test.mjs >/dev/null)
"$ROOT/scripts/build-ios-xcframework.sh"
"$ROOT/scripts/build-android-jnilibs.sh"

echo "==> packing"
rm -rf "$DIST" && mkdir -p "$DIST"
(cd "$ROOT/pods/wasm" && npm pack --pack-destination "$DIST" >/dev/null)
(cd "$ROOT/pods/expo" && npm pack --pack-destination "$DIST" >/dev/null)
cp "$ROOT/pods/wasm/dist/fhir_pods.wasm" "$DIST/fhir_pods-$VERSION.wasm"
cp "$ROOT/crates/fhir-pods-ffi/include/fhir_pods.h" "$DIST/"
(cd "$DIST" && shasum -a 256 -- * > SHA256SUMS)

echo "==> $DIST"
ls -la "$DIST" | sed 's/^/    /'
echo "==> tag with: git tag -a v$VERSION -m 'fhir-pods $VERSION'"
