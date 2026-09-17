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
member. So an expansion is compiled only when it is complete: a paged
expansion (`offset`, `count`, or `total` beyond what it carries) is
refused unless every page of the set is given, and the pages add up to
`total`; a limited expansion (`limitedExpansion`, `too-costly`) is refused.
Nothing is inferred from a response merely parsing. Keep the licence of
what you compile: a SNOMED CT expansion is licensed content and belongs in
the host's own build, never in the pods.

Usage:
    compile_valueset.py [--no-display] <expanded-valueset.json> [...] > terminology.txt
"""
import json
import sys


def rows_of(vs):
    """(system, code, display) rows of one expansion, hierarchy flattened."""
    stack = list((vs.get("expansion") or {}).get("contains") or [])
    rows = set()
    while stack:
        c = stack.pop()
        stack.extend(c.get("contains", []))  # a hierarchical expansion nests
        if c.get("abstract") or c.get("inactive"):
            continue
        if c.get("system") and c.get("code"):
            display = (c.get("display") or "").replace("\t", " ").replace("\n", " ")
            rows.add((c["system"], c["code"], display))
    return rows


def params_of(vs):
    out = {}
    for p in (vs.get("expansion") or {}).get("parameter", []) or []:
        v = next((v for k, v in p.items() if k.startswith("value")), None)
        out.setdefault(p.get("name"), []).append(v)
    return out


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    with_display = "--no-display" not in sys.argv
    if not args:
        sys.exit(__doc__)
    # every page of a set, by url
    sets = {}
    for path in args:
        vs = json.load(open(path))
        if vs.get("resourceType") != "ValueSet":
            sys.exit(f"{path}: not a ValueSet")
        url = vs.get("url")
        if not url:
            sys.exit(f"{path}: ValueSet has no url")
        exp = vs.get("expansion")
        if exp is None or exp.get("contains") is None:
            sys.exit(f"{path}: no expansion — run $expand first")
        params = params_of(vs)
        if any(v is True for v in params.get("limitedExpansion", [])) or "too-costly" in params:
            sys.exit(f"{path}: {url}: the expansion is limited (limitedExpansion / too-costly) — not a complete set")
        if any(f.get("code") == "too-costly" for f in ((vs.get("issue") or []))):
            sys.exit(f"{path}: {url}: the expansion is too costly — not a complete set")
        entry = sets.setdefault(url, {"rows": set(), "pages": [], "total": None})
        entry["rows"] |= rows_of(vs)
        offset = exp.get("offset")
        if offset is None:
            offset = next((int(v) for v in params.get("offset", []) if v is not None), 0)
        count = next((int(v) for v in params.get("count", []) if v is not None), None)
        entry["pages"].append((path, offset, len(rows_of(vs)), count))
        if exp.get("total") is not None:
            if entry["total"] not in (None, exp["total"]):
                sys.exit(f"{path}: {url}: pages disagree on total ({entry['total']} vs {exp['total']})")
            entry["total"] = exp["total"]
    out = ["fhir-pods-terminology 1"]
    for url in sorted(sets):
        e = sets[url]
        pages = sorted(e["pages"], key=lambda p: p[1])
        paged = len(pages) > 1 or any(off > 0 or cnt is not None for _, off, _, cnt in pages)
        carried = sum(n for _, _, n, _ in pages)
        if e["total"] is not None:
            if carried != e["total"]:
                sys.exit(f"{url}: the pages given carry {carried} of {e['total']} members — give every page, or an unpaged $expand")
        elif paged:
            sys.exit(f"{url}: a paged expansion without a total cannot be shown complete — give an unpaged $expand")
        # pages must be contiguous from 0 when offsets are stated
        expect = 0
        for path, off, n, _ in pages:
            if paged and off != expect:
                sys.exit(f"{path}: {url}: page at offset {off} — expected {expect}; a page is missing")
            expect = off + n
        out.append(f"= {url}")
        for system, code, display in sorted(e["rows"]):
            display = display if with_display else ""
            out.append(f"{system}\t{code}\t{display}" if display else f"{system}\t{code}")
    sys.stdout.write("\n".join(out) + "\n")


if __name__ == "__main__":
    main()
