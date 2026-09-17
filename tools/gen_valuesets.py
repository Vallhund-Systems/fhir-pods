#!/usr/bin/env python3
"""Generate the bundled value-set table (`src/value_sets_table.rs`): every
value set the registry binds with `required` strength that can be expanded
offline from the cached packages' own `compose` — enumerated concepts, or a
whole code system the packages ship as `content: complete` (the FHIR and
HL7 v2/v3 code systems are licence-free and travel with `hl7.fhir.r4.core`).

A set is *skipped*, never approximated, when it needs content the packages
don't carry: anything on SNOMED CT or LOINC (licensed — those only ever
arrive through the terminology seam), VSAC / NCTS-hosted sets that ship as
a bare reference, IANA registries (`mimetypes`), or a `filter` we can't
evaluate. The engine answers `Unknown` for those and a host provider decides.

Usage:
    gen_valuesets.py <generated/profiles/dir> <out.rs> <package/dir> [<package/dir> ...]
"""
import glob
import json
import os
import re
import sys

LICENSED = {"http://snomed.info/sct", "http://loinc.org"}


def rs(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def walk_concepts(concepts, out):
    for c in concepts:
        out.append((c["code"], c.get("display")))
        walk_concepts(c.get("concept", []), out)


class Expander:
    def __init__(self, value_sets, code_systems):
        self.vs = value_sets
        self.cs = code_systems

    def display(self, system, code):
        c = self.cs.get(system)
        if not c:
            return None
        out = []
        walk_concepts(c.get("concept", []), out)
        for k, d in out:
            if k == code:
                return d
        return None

    def expand(self, url, seen=()):
        """`(codes, why)`: a sorted list of (system, code, display) — or
        `(None, reason)` when the set cannot be expanded offline."""
        url = url.split("|")[0]
        if url in seen:
            return None, "circular " + url
        j = self.vs.get(url)
        if not j:
            return None, "no ValueSet " + url
        res = {}
        for inc in (j.get("compose") or {}).get("include", []):
            if inc.get("valueSet"):
                for u in inc["valueSet"]:
                    r, why = self.expand(u, seen + (url,))
                    if r is None:
                        return None, why
                    for s, c, d in r:
                        res[(s, c)] = d
                continue
            system = inc.get("system")
            # SNOMED CT and LOINC content never ships, enumerated or not: the
            # seam (the reference validator, run offline, cannot confirm these
            # codes either — bundling them would part us from the oracle)
            if system in LICENSED:
                return None, "licensed " + system
            if inc.get("concept"):
                for c in inc["concept"]:
                    res[(system, c["code"])] = c.get("display") or self.display(system, c["code"])
            elif inc.get("filter"):
                return None, "filter on " + str(system)
            else:
                c = self.cs.get(system)
                if not c or c.get("content") != "complete":
                    return None, "no complete CodeSystem " + str(system)
                out = []
                walk_concepts(c.get("concept", []), out)
                for code, d in out:
                    res[(system, code)] = d
        for exc in (j.get("compose") or {}).get("exclude", []):
            if exc.get("filter") or not exc.get("concept"):
                return None, "exclude form"
            for c in exc["concept"]:
                res.pop((exc.get("system"), c["code"]), None)
        return sorted((s, c, d or "") for (s, c), d in res.items()), ""


def main():
    generated, out_path, pkgs = sys.argv[1], sys.argv[2], sys.argv[3:]
    urls = set()
    for f in glob.glob(os.path.join(generated, "*.rs")):
        urls |= set(re.findall(r'value_set: "([^"|]+)', open(f).read()))

    value_sets, code_systems = {}, {}
    for pkg in pkgs:  # first package wins — put the base spec first
        for f in glob.glob(os.path.join(pkg, "ValueSet-*.json")):
            j = json.load(open(f))
            value_sets.setdefault(j.get("url"), j)
        for f in glob.glob(os.path.join(pkg, "CodeSystem-*.json")):
            j = json.load(open(f))
            code_systems.setdefault(j.get("url"), j)

    ex = Expander(value_sets, code_systems)
    bundled, skipped = [], []
    for url in sorted(urls):
        codes, why = ex.expand(url)
        if codes is None:
            skipped.append((url, why))
        else:
            bundled.append((url, codes))

    # displays for the codings the registry fixes (`….coding.system` /
    # `….coding.code` pairs in the LOWERING tables) whose code system the
    # packages ship — never SNOMED or LOINC
    fixed = {}
    for f in glob.glob(os.path.join(generated, "*.rs")):
        for path, value in re.findall(r'\(("[^"]+"), ("[^"]*")\)', open(f).read()):
            path, value = json.loads(path), json.loads(value)
            parent, _, leaf = path.rpartition(".")
            if leaf in ("system", "code"):
                fixed.setdefault((f, parent), {})[leaf] = value
    displays = {}
    for pair in fixed.values():
        system, code = pair.get("system"), pair.get("code")
        if not system or not code or system in LICENSED:
            continue
        d = ex.display(system, code)
        if d:
            displays[(system, code)] = d

    n = sum(len(c) for _, c in bundled)
    with open(out_path, "w") as f:
        f.write("#![allow(clippy::type_complexity)]\n")
        f.write("//! GENERATED by tools/gen_valuesets.py — do not edit.\n")
        f.write("//! The value sets the registry binds (`required`) that expand offline from\n")
        f.write("//! the packages' own compose: licence-free FHIR / HL7 v2 / v3 content.\n")
        f.write("//! Each set is sorted by (system, code); displays come from the code system.\n")
        f.write("//!\n//! Not bundled (answered through the terminology seam):\n")
        for url, why in skipped:
            f.write(f"//! - `{url}` — {why}\n")
        f.write("\n/// (value set URL, sorted [(system, code, display)]), sorted by URL.\n")
        f.write("pub static SETS: &[(&str, &[(&str, &str, &str)])] = &[\n")
        for url, codes in bundled:
            f.write(f"    ({rs(url)}, &[\n")
            for s, c, d in codes:
                f.write(f"        ({rs(s)}, {rs(c)}, {rs(d)}),\n")
            f.write("    ]),\n")
        f.write("];\n\n")
        f.write("/// Displays for the codings the registry fixes, from the packages' code\n")
        f.write("/// systems: sorted (system, code, display).\n")
        f.write("pub static DISPLAYS: &[(&str, &str, &str)] = &[\n")
        for (system, code), d in sorted(displays.items()):
            f.write(f"    ({rs(system)}, {rs(code)}, {rs(d)}),\n")
        f.write("];\n")
    print(
        f"wrote {out_path}: {len(bundled)} sets, {n} codes, {len(displays)} fixed-coding displays; "
        f"{len(skipped)} sets left to the seam"
    )


if __name__ == "__main__":
    main()
