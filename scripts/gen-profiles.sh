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
  "$C/package-ips/package"      # hl7.fhir.uv.ips 2.0.1
  "$C/package-au-ps/package"    # hl7.fhir.au.ps 1.0.0
  "$C/package-balp/package"     # ihe.iti.balp 1.1.4
  "$C/package-au-base/package"  # hl7.fhir.au.base 6.0.0 — every profile, since 0.9.1 (its examples are in the corpus)
  "$C/package-ext/package"      # hl7.fhir.uv.extensions.r4 5.3.0 — every extension, since 0.9.1 (contexts and values checked for all)
  "$C/package-uk/package"       # fhir.r4.ukcore.stu2 2.1.0 — since 0.13.0 (export region `uk`)
  "$C/package-ca/package"       # hl7.fhir.ca.baseline 1.2.0 — since 0.13.0 (export region `ca`)
  "$C/package-nz/package"       # fhir.org.nz.ig.base 3.1.0 — since 0.13.0 (export region `nz`)
)
DEPS=(
)
THO="$C/package-tho/package"      # hl7.terminology.r4 7.3.0 — the code systems' content (tools/gen_codesystems.py)
R4="$C/package-r4/package"        # hl7.fhir.r4.core 4.0.1 — the element-type table, the implied vital-sign profiles, the base AuditEvent / Provenance and patient-record resources (tools/gen_all.py BASE_RESOURCES)
PROVENANCE="hl7.fhir.au.core 2.0.0 + hl7.fhir.us.core 6.1.0 + hl7.fhir.uv.ipa 1.1.0 + hl7.fhir.uv.ips 2.0.1 + hl7.fhir.au.ps 1.0.0 + ihe.iti.balp 1.1.4 + hl7.fhir.au.base 6.0.0 + hl7.fhir.uv.extensions.r4 5.3.0 + fhir.r4.ukcore.stu2 2.1.0 + hl7.fhir.ca.baseline 1.2.0 + fhir.org.nz.ig.base 3.1.0 + hl7.fhir.r4.core 4.0.1 (AuditEvent, Provenance)"

for d in "${PRIMARIES[@]}" ${DEPS[@]+"${DEPS[@]}"} "$R4" "$THO"; do
  [ -d "$d" ] || { echo "missing package dir $d — run: python3 conformance/make_corpus.py" >&2; exit 1; }
done

rm -f "$OUT"/*.rs
python3 "$ROOT/tools/gen_all.py" --primaries "${#PRIMARIES[@]}" --base "$R4" "$OUT" "$PROVENANCE" "${PRIMARIES[@]}" ${DEPS[@]+"${DEPS[@]}"}
python3 "$ROOT/tools/gen_typemap.py" "$R4" "$OUT" "$ROOT/crates/fhir-pods-core/src/typemap.rs"
python3 "$ROOT/tools/gen_rules.py" "$ROOT/crates/fhir-pods-core/src"   # every invariant key → its stable rule id
python3 "$ROOT/tools/gen_xhtml_entities.py"                          # the HTML entity names the narrative check refuses
python3 "$ROOT/tools/gen_rules_doc.py"                                 # docs/rules.md from the RULES table
python3 "$ROOT/tools/gen_valuesets.py" "$OUT" "$ROOT/crates/fhir-pods-core/src/value_sets_table.rs" "$R4" "${PRIMARIES[@]}" ${DEPS[@]+"${DEPS[@]}"} "$THO"
# every definition the packages hold under HL7's own URL spaces (a `uri` there that names nothing is an error)
python3 "$ROOT/tools/gen_known_urls.py" "$ROOT/crates/fhir-pods-core/src/known_urls.rs" "${PRIMARIES[@]}" ${DEPS[@]+"${DEPS[@]}"} "$R4" "$THO"
# the base code systems' content: the union of R4 core's and THO's copies
python3 "$ROOT/tools/gen_codesystems.py" "$R4" "$THO" "$ROOT/crates/fhir-pods-core/src/code_systems_table.rs"
# the HL7 v2 maps (hl7.fhir.uv.v2mappings 1.0.0) compiled to the v2 pod's tables
python3 "$ROOT/tools/gen_v2.py" "$C/package-v2mappings/package" "$ROOT/crates/fhir-pods-core/src/v2/generated.rs" "$C/package-ext/package" "$R4"

# generated output is formatted like the rest of the crate
(cd "$ROOT" && cargo fmt -q --all)
