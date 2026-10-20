#!/usr/bin/env bash
# Build every artifact from a clean state and pack the pods into dist/ with
# checksums. Local only (this repo has no remote by design): the tarballs are
# what a host app vendors.
#
#   ./scripts/release.sh            # → dist/<version>/
set -euo pipefail
unset FEATURES   # a release ships the shipped registry only; host builds set FEATURES themselves

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/crates/fhir-pods-core/Cargo.toml" | head -1)"
DIST="$ROOT/dist/$VERSION"

echo "==> fhir-pods $VERSION"
# Tracked files must be committed: the artifacts come from the commit. Untracked
# files (another lane's work in flight) are not packed — the pods' package
# manifests list what ships — so they do not block a release; they are listed.
[ -z "$(git -C "$ROOT" status --porcelain --untracked-files=no)" ] || { echo "working tree not clean (tracked changes)" >&2; exit 1; }
UNTRACKED="$(git -C "$ROOT" status --porcelain --untracked-files=all | grep '^??' || true)"
[ -z "$UNTRACKED" ] || echo "==> note: untracked files present, not part of this release:"$'\n'"$UNTRACKED"
# The clean-room wall: the repo must never mention the host app's names. The
# pattern is assembled so this script itself does not trip it.
WALL="$(printf '%s|%s|%s' "aver""care" "avatars""global" "local""agent")"
git -C "$ROOT" grep -Il -iE "$WALL" -- . >/dev/null 2>&1 && { echo "clean-room check failed" >&2; exit 1; }

echo "==> tests"
(cd "$ROOT" && cargo test -q --workspace >/dev/null)
(cd "$ROOT" && cargo check -q -p fhir-pods-core --features host-profiles)   # the host-profile stub still builds
(cd "$ROOT" && cargo check -q -p fhir-pods-ffi --features host-profiles)    # and the pods' passthrough
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

# The cross-target builds exist only to be packed: once packed they are
# ~12 GB of dead weight per release, and cargo never collects them. The native
# debug and release profiles stay — the next test run reuses them; the instrument-path
# lane's host-profile build (target/host-profiles) goes with the rest.
echo "==> sweeping the packed cross-target builds"
rm -rf "$ROOT"/target/{wasm32-unknown-unknown,aarch64-apple-ios,aarch64-apple-ios-sim,aarch64-linux-android,x86_64-linux-android,thumbv7em-none-eabi,doc,host-profiles}
echo "    target/ is now $(du -sh "$ROOT/target" | cut -f1) — 'cargo clean' when the debug profile has grown past a few GB"
echo "==> tag with: git tag -a v$VERSION -m 'fhir-pods $VERSION'"
