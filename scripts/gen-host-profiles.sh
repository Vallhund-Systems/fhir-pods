#!/usr/bin/env bash
# Compile a host's own profiles into crates/fhir-pods-core/src/generated_host,
# merged into the registry by `--features host-profiles` (the pods' build
# scripts take FEATURES="host-profiles"). The host's packages are primaries —
# every resource profile, Identifier and complex Extension profile in them is
# compiled, and any other datatype profile of theirs an element is held to
# alone; the pinned packages (the shipped registry's) are what they may
# derive from and slice by. The registry is written only when the whole
# package compiles: a canonical the shipped registry already holds is
# refused, and so is a definition without a snapshot (a differential alone
# compiles to nothing). The profiles' own invariant keys get stable rule ids.
#
#   ./scripts/gen-host-profiles.sh <package dir> [<package dir> ...]
#   ./scripts/gen-host-profiles.sh --reset          # back to the stub
#
# A package dir holds the StructureDefinitions (with snapshots) as JSON. Host
# value sets and code systems are not bundled: give them to the pod as a
# terminology table (tools/compile_valueset.py) or a terminology provider.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
C="$ROOT/conformance"
SRC="$ROOT/crates/fhir-pods-core/src"
OUT="$SRC/generated_host"
STUB='// The host'"'"'s own compiled profiles — a stub until scripts/gen-host-profiles.sh
// writes the registry for a host package (see README, "Host profiles"). Built
// in with `--features host-profiles`; the shipped registry stays as it is.
use crate::compiled::CompiledProfile;

pub static REGISTRY: &[&CompiledProfile] = &[];

pub static DATATYPE_PROFILE_ELEMENTS: &[(&str, &str, &str, &str, &str)] = &[];

pub static SOURCE_SHA256: &[(&str, &str)] = &[];'

if [ "${1:-}" = "--reset" ]; then
  rm -rf "$OUT" && mkdir -p "$OUT" && printf '%s\n' "$STUB" > "$OUT/mod.rs"
  python3 "$ROOT/tools/gen_rules.py" "$SRC"    # the invariant ids back to the shipped keys
  echo "==> $OUT reset to the stub"
  exit 0
fi
[ $# -ge 1 ] || { sed -n '2,17p' "$0"; exit 2; }
for d in "$@"; do [ -d "$d" ] || { echo "no such package dir: $d" >&2; exit 1; }; done

# the shipped registry's packages, as what a host profile derives from
PINNED=(
  "$C/package/package" "$C/package-us/package" "$C/package-ipa/package" "$C/package-ips/package"
  "$C/package-au-ps/package" "$C/package-balp/package" "$C/package-au-base/package" "$C/package-ext/package"
  "$C/package-uk/package" "$C/package-ca/package" "$C/package-nz/package"
)
R4="$C/package-r4/package"
for d in "${PINNED[@]}" "$R4"; do
  [ -d "$d" ] || { echo "missing pinned package dir $d — run: python3 conformance/make_corpus.py" >&2; exit 1; }
done

HOSTS=("$@")
# provenance by package name, never by path (the registry file is tracked)
PROVENANCE="$(python3 - "${HOSTS[@]}" <<'PY'
import json, os, sys
names = []
for d in sys.argv[1:]:
    try:
        p = json.load(open(os.path.join(d, "package.json")))
        names.append(f"{p.get('name', os.path.basename(d))} {p.get('version', '')}".strip())
    except Exception:
        names.append(os.path.basename(os.path.abspath(d)))
print("host packages: " + " + ".join(names))
PY
)"

# Compile and fingerprint the same private snapshot of the host package. Reading
# a mutable source again after generation could attest bytes never compiled.
WORK="$(mktemp -d "${TMPDIR:-/tmp}/fhir-pods-host.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
TMP="$WORK/generated"
mkdir "$TMP"
python3 "$ROOT/tools/host_profile_sources.py" stage "$WORK/inputs" "${HOSTS[@]}"
HOST_COUNT=${#HOSTS[@]}
# the host's own package dirs: what a refusal names (the staged copies are
# private temporary paths)
SOURCES=("${HOSTS[@]}")
HOSTS=()
for ((i=0; i<HOST_COUNT; i++)); do HOSTS+=("$WORK/inputs/package-$i"); done

# What the host package compiles (the generator's own rule: a constraint
# with a snapshot, on a resource or an Identifier / complex Extension, and any
# other datatype profile of the host's an element is held to alone) must not
# be a canonical the shipped registry holds; a definition the generator would
# skip for want of a snapshot is refused by name. Judged before anything is
# generated (tools/host_profile_sources.py precheck).
python3 - "$SRC/generated" "$ROOT/tools" "$HOST_COUNT" "${HOSTS[@]}" "${SOURCES[@]}" <<'PY'
import re, sys, glob, os
shipped_dir, tools, count = sys.argv[1], sys.argv[2], int(sys.argv[3])
hosts, sources = sys.argv[4:4 + count], sys.argv[4 + count:]
sys.path.insert(0, tools)
import host_profile_sources
def url_of(path):
    m = re.search(r'^\s+url: "([^"]+)"', open(path).read(), re.M)
    return m[1] if m else None
shipped = {url_of(p) for p in glob.glob(os.path.join(shipped_dir, "*.rs"))} - {None}
host_profile_sources.precheck(shipped, hosts, sources)
PY

# Generated into the private workspace; installed only after all checks pass.
# R4 core is a plain dependency here (not --base): the base resources are
# already in the shipped registry
python3 "$ROOT/tools/gen_all.py" --primaries "${#HOSTS[@]}" "$TMP" "$PROVENANCE" "${HOSTS[@]}" "${PINNED[@]}" "$R4" >/dev/null
# the module is include!d: inner doc comments become plain comments
sed -i.bak 's|^//!|//|' "$TMP/mod.rs" && rm -f "$TMP/mod.rs.bak"

# A dependency's datatype profile the generator re-emitted for the host's
# slices is pruned — the shipped copy serves. The generator's IMPLIED table
# is the shipped one's.
python3 - "$TMP" "$SRC/generated" <<'PY'
import re, sys, glob, os
out, shipped_dir = sys.argv[1], sys.argv[2]
def url_of(path):
    m = re.search(r'^\s+url: "([^"]+)"', open(path).read(), re.M)
    return m[1] if m else None
shipped = {url_of(p) for p in glob.glob(os.path.join(shipped_dir, "*.rs"))} - {None}
pruned, kept = [], []
for p in sorted(glob.glob(os.path.join(out, "*.rs"))):
    if p.endswith("mod.rs"):
        continue
    url = url_of(p)
    if url is None:
        continue  # intern.rs, the shared string table
    name = os.path.basename(p)[:-3]
    if url in shipped:
        os.remove(p); pruned.append(name)
    else:
        kept.append(name)
mod = os.path.join(out, "mod.rs")
text = open(mod).read()
text = re.sub(r"\n(?:///[^\n]*\n)*pub static IMPLIED[^;]*;\n", "\n", text, flags=re.S)
lines = [l for l in text.split("\n") if not any(l == f"pub mod {n};" or l.strip() == f"&{n}::PROFILE," for n in pruned)]
# a pruned dependency's datatype-profile rows are the shipped table's
lines = [l for l in lines if not any(l.strip().startswith(f'("{u}",') for u in shipped)]
open(mod, "w").write("\n".join(lines))
print(f"==> {len(kept)} host profile(s) compiled" + (f"; {len(pruned)} shipped dependencies pruned" if pruned else ""))
PY

python3 "$ROOT/tools/host_profile_sources.py" record "$TMP" "${HOSTS[@]}"
rm -rf "$OUT" && mv "$TMP" "$OUT"
rm -rf "$WORK" && trap - EXIT
rustfmt --edition 2021 "$OUT"/*.rs 2>/dev/null || true
python3 "$ROOT/tools/gen_rules.py" "$SRC"    # the host's invariant keys get stable ids too
echo "==> $OUT written; build with: cargo build --features host-profiles   (pods: FEATURES=host-profiles scripts/build-*.sh)"
