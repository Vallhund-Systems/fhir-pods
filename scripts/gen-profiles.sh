#!/usr/bin/env bash
# Regenerate crates/fhir-pods-core/src/generated from the pinned packages.
# Packages are fetched by conformance/make_corpus.py into gitignored caches
# under conformance/ (run it once first, or run conformance/run.py).
#
#   ./scripts/gen-profiles.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
C="$ROOT/conformance"
OUT="$ROOT/crates/fhir-pods-core/src/generated"

# The IGs under test (primary: their resource profiles are compiled), then the
# dependency packages whose datatype profiles slices inherit. Keep in step
# with conformance/make_corpus.py (IGS + DEPS).
PRIMARIES=(
  "$C/package/package"          # hl7.fhir.au.core 2.0.0
  "$C/package-us/package"       # hl7.fhir.us.core 6.1.0
  "$C/package-ipa/package"      # hl7.fhir.uv.ipa 1.1.0
)
DEPS=(
  "$C/package-au-base/package"  # hl7.fhir.au.base 6.0.0
)
R4="$C/package-r4/package"        # hl7.fhir.r4.core 4.0.1 — the element-type table only
PROVENANCE="hl7.fhir.au.core 2.0.0 + hl7.fhir.us.core 6.1.0 + hl7.fhir.uv.ipa 1.1.0 + hl7.fhir.au.base 6.0.0"

for d in "${PRIMARIES[@]}" "${DEPS[@]}" "$R4"; do
  [ -d "$d" ] || { echo "missing package dir $d — run: python3 conformance/make_corpus.py" >&2; exit 1; }
done

rm -f "$OUT"/*.rs
python3 "$ROOT/tools/gen_all.py" --primaries "${#PRIMARIES[@]}" --base "$R4" "$OUT" "$PROVENANCE" "${PRIMARIES[@]}" "${DEPS[@]}"
python3 "$ROOT/tools/gen_typemap.py" "$R4" "$OUT" "$ROOT/crates/fhir-pods-core/src/typemap.rs"
python3 "$ROOT/tools/gen_valuesets.py" "$OUT" "$ROOT/crates/fhir-pods-core/src/value_sets_table.rs" "$R4" "${PRIMARIES[@]}" "${DEPS[@]}"

# generated output is formatted like the rest of the crate
(cd "$ROOT" && cargo fmt -q --all)
