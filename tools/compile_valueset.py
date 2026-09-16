#!/usr/bin/env python3
"""Compile value-set expansions into the terminology table the pods load at
run time (`Expansions` in the core; `fhir_pods_terminology_load` on the ABI;
`loadTerminology` in the wasm / Expo pods).

Input: one or more FHIR `ValueSet` resources carrying an `expansion` — what a
terminology server returns from `$expand`, e.g.

    curl -s 'https://r4.ontoserver.csiro.au/fhir/ValueSet/$expand?url=<value-set-url>' > vs.json
    curl -s -H 'Authorization: Bearer <token>' \\
         'https://api.healthterminologies.gov.au/integration/R4/fhir/ValueSet/$expand?url=<value-set-url>' > vs.json

Output: the line format —

    fhir-pods-terminology 1
    = <value-set url>
    <system>\\t<code>\\t<display>

A table is *complete* for the sets it lists: a code not listed is not a
member. Keep the licence of what you compile: a SNOMED CT expansion is
licensed content and belongs in the host's own build, never in the pods.

Usage:
    compile_valueset.py [--no-display] <expanded-valueset.json> [...] > terminology.txt
"""
import json
import sys


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    with_display = "--no-display" not in sys.argv
    if not args:
        sys.exit(__doc__)
    out = ["fhir-pods-terminology 1"]
    for path in args:
        vs = json.load(open(path))
        if vs.get("resourceType") != "ValueSet":
            sys.exit(f"{path}: not a ValueSet")
        url = vs.get("url")
        if not url:
            sys.exit(f"{path}: ValueSet has no url")
        contains = (vs.get("expansion") or {}).get("contains")
        if contains is None:
            sys.exit(f"{path}: no expansion — run $expand first")
        out.append(f"= {url}")
        stack = list(contains)
        rows = set()
        while stack:
            c = stack.pop()
            stack.extend(c.get("contains", []))  # a hierarchical expansion nests
            if c.get("abstract") or c.get("inactive"):
                continue
            if c.get("system") and c.get("code"):
                display = (c.get("display") or "") if with_display else ""
                rows.add((c["system"], c["code"], display.replace("\t", " ").replace("\n", " ")))
        for system, code, display in sorted(rows):
            out.append(f"{system}\t{code}\t{display}" if display else f"{system}\t{code}")
    sys.stdout.write("\n".join(out) + "\n")


if __name__ == "__main__":
    main()
