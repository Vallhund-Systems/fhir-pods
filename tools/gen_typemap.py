#!/usr/bin/env python3
"""Generate the element-type table (`src/typemap.rs`) from the base FHIR
package: for every element of every resource type and complex datatype, its
FHIR type and whether it repeats. This is what lets a JSON node know its type — `descendants().as(canonical)`,
`value.ofType(Quantity)`, `$this is dateTime` on arbitrary nodes — without the
document carrying any type information itself.

`id` / `extension` / `modifierExtension` are implicit (the lookup knows
them). Every resource type is included because a contained resource, or a
Bundle entry, can be any of them — dom-3 walks `descendants().as(uri)`
through the contained resources, and typing must not stop at the registry.

Beside the type codes: the datatype profiles the package defines
(`SimpleQuantity`, `MoneyQuantity`) and every base element whose type names
one (`ELEMENT_PROFILES`), which the shape walk applies where it meets them.

Usage:
    gen_typemap.py <r4.core/package/dir> <generated/profiles/dir> <out.rs>
"""
import glob
import json


def rs(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'

import os
import re
import sys

IMPLICIT = {"id", "extension", "modifierExtension"}


def applicable_datatype_profiles(pkg, datatypes):
    """The package's datatype profiles the shape walk can apply whole —
    `(url, version, name, type, prohibited, invariants)`, sorted by name —
    and the canonicals of the ones it cannot. A datatype profile is a
    `constraint` StructureDefinition on a complex datatype other than
    `Extension` (an extension's own profile is the registry's).
    Applicable whole means its differential says nothing but invariants on
    the root and `max 0` on elements directly beneath it."""
    out, skipped = [], []
    for p in sorted(glob.glob(os.path.join(pkg, "StructureDefinition-*.json"))):
        d = json.load(open(p))
        if d.get("derivation") != "constraint" or d.get("type") not in datatypes or d.get("type") == "Extension":
            continue
        ty = d["type"]
        prohibited, invariants, whole = [], [], True
        for e in d["differential"]["element"]:
            path = e["path"]
            said = {k for k in e if k not in ("id", "path", "definition", "comment", "short", "requirements",
                                               "mapping", "isSummary", "isModifier", "min", "max", "constraint")}
            if said or e.get("sliceName"):
                whole = False
            elif path == ty:
                if e.get("min", 0) != 0 or e.get("max", "*") != "*":
                    whole = False
                for c in e.get("constraint", []):
                    if not c.get("key") or not c.get("expression"):
                        whole = False
                        continue
                    invariants.append((c["key"], c.get("severity") == "error", c.get("human", ""), c["expression"]))
            elif path.count(".") == 1 and path.startswith(ty + ".") and e.get("max") == "0" and not e.get("constraint"):
                prohibited.append(path.split(".", 1)[1])
            else:
                whole = False
        if whole:
            out.append((d["url"], d["version"], d["name"], ty, prohibited, invariants))
        else:
            skipped.append(d["url"])
    out.sort(key=lambda x: x[2])
    return out, skipped


def element_datatype_profiles(types, sds, index, profiles, skipped):
    """Every base element whose `type[].profile` names a datatype profile, as
    `(typing context of its parent, element key, profile index)`, sorted. The
    context is the one the shape walk is in when it meets the element: the
    type itself, or the bare path of a backbone / inline element; a choice's
    key is the concrete key of the profiled alternative (`doseQuantity`). A
    profile no applicable one answers to is refused, never dropped."""
    rows = []
    for t in types:
        ctx_of = {t: t}
        for e in sds[t]["snapshot"]["element"]:
            codes = [x.get("code") for x in e.get("type", [])]
            if codes in (["BackboneElement"], ["Element"]):
                ctx_of[e["path"]] = e["path"]
        for e in sds[t]["snapshot"]["element"]:
            path = e["path"]
            for x in e.get("type", []):
                for url in x.get("profile", []):
                    bare = url.split("|")[0]
                    if bare not in index:
                        why = "is not applicable whole" if bare in skipped else "is not a datatype profile the package defines"
                        raise SystemExit(f"{path}: type profile {url} {why}")
                    if profiles[index[bare]][3] != x["code"]:
                        raise SystemExit(f"{path}: {url} constrains {profiles[index[bare]][3]}, not {x['code']}")
                    parent, name = path.rsplit(".", 1)
                    if parent not in ctx_of:
                        raise SystemExit(f"{path}: no typing context for {parent}")
                    if name.endswith("[x]"):
                        name = name[:-3] + x["code"][0].upper() + x["code"][1:]
                    rows.append((ctx_of[parent], name, index[bare]))
    rows.sort()
    if len(set((c, k) for c, k, _ in rows)) != len(rows):
        raise SystemExit("an element names two datatype profiles")
    return rows


def main():
    pkg, generated, out_path = sys.argv[1], sys.argv[2], sys.argv[3]

    sds = {}
    for p in glob.glob(os.path.join(pkg, "StructureDefinition-*.json")):
        d = json.load(open(p))
        if d.get("derivation") == "specialization" and d.get("kind") in ("resource", "complex-type"):
            sds[d["type"]] = d

    # every resource type: a contained resource or a Bundle entry can be any of them
    types = sorted(sds)
    # per type: (relative element path, type codes, repeats), sorted by path
    groups = {}
    dictionary = {}

    def dict_index(code):
        if code not in dictionary:
            dictionary[code] = len(dictionary)
        return dictionary[code]

    n = 0
    for t in types:
        rows = []
        for e in sds[t]["snapshot"]["element"]:
            path = e["path"]
            if "." not in path or path.rsplit(".", 1)[1] in IMPLICIT:
                continue
            if e.get("contentReference"):
                codes = ["#" + e["contentReference"].split("#")[-1]]
            else:
                # a primitive's value element (and `Extension.url`, `Element.id`) is
                # typed with a FHIRPath System type; the FHIR type rides on it
                codes = []
                for x in e.get("type", []):
                    if not x.get("code"):
                        continue
                    fhir_type = next(
                        (ext.get("valueUrl") for ext in x.get("extension", [])
                         if ext.get("url") == "http://hl7.org/fhir/StructureDefinition/structuredefinition-fhir-type"),
                        None,
                    )
                    codes.append(fhir_type or x["code"].rsplit("/", 1)[-1].replace("System.", ""))
            if not codes:
                continue
            # a backbone / inline element is its own typing context: keep its
            # full path in the dictionary (`#Observation.component`, like a
            # content reference; `~Timing.repeat` for a plain Element)
            if codes == ["BackboneElement"]:
                codes = ["#" + path]
            elif codes == ["Element"]:
                codes = ["~" + path]
            repeats = e.get("max") not in (None, "1", "0")
            required = (e.get("min") or 0) >= 1
            rows.append((path.split(".", 1)[1], "|".join(codes), repeats, required))
        rows.sort()
        groups[t] = rows
        n += len(rows)
    for t in types:
        for _, code, _, _ in groups[t]:
            dict_index(code)

    with open(out_path, "w") as f:
        f.write("//! GENERATED by tools/gen_typemap.py from hl7.fhir.r4.core — do not edit.\n")
        f.write("//! The element-type table, packed: `TABLE` holds every element of every\n")
        f.write("//! resource and complex datatype as (path relative to its type, type-code\n")
        f.write("//! index, repeats), grouped by type in `TYPES` order and sorted within a\n")
        f.write("//! type by path; `RANGES[i]` is type `TYPES[i]`'s span of `TABLE`.\n\n")
        f.write("use crate::compiled::{Binding, BindingKind};\n\n")
        f.write("/// Every resource and complex datatype name, sorted.\n")
        f.write("pub static TYPES: &[&str] = &[\n")
        for t in types:
            f.write(f'    "{t}",\n')
        f.write("];\n\n")
        f.write("/// The resource types among them (for `is Resource` / `is DomainResource`), sorted.\n")
        f.write("pub static RESOURCES: &[&str] = &[\n")
        for t in types:
            if sds[t]["kind"] == "resource":
                f.write(f'    "{t}",\n')
        f.write("];\n\n")
        f.write("/// `TABLE[start..end]` for each of `TYPES`.\n")
        f.write("pub static RANGES: &[(u32, u32)] = &[\n")
        pos = 0
        for t in types:
            f.write(f"    ({pos}, {pos + len(groups[t])}),\n")
            pos += len(groups[t])
        f.write("];\n\n")
        # DomainResource's own invariants (dom-1 … dom-6): what every domain
        # resource owes, profile or no profile
        dom = sds.get("DomainResource") or {}
        f.write("/// `DomainResource`'s root invariants: (key, is_error, human, expression).\n")
        f.write("pub static DOMAIN_INVARIANTS: &[(&str, bool, &str, &str)] = &[\n")
        for e in (dom.get("snapshot") or {}).get("element", []):
            if e.get("path") != "DomainResource":
                continue
            for c in e.get("constraint", []):
                if c.get("key") and c.get("expression"):
                    f.write(f'    ({rs(c["key"])}, {str(c.get("severity") == "error").lower()}, {rs(c.get("human", ""))}, {rs(c["expression"])}),\n')
        f.write("];\n\n")
        # Bundle's own invariants (bdl-1 … bdl-12), root and entry: what every
        # Bundle owes when no profile applies
        bsd = sds.get("Bundle") or {}
        f.write("/// `Bundle`'s invariants: (element path, key, is_error, human, expression).\n")
        f.write("pub static BUNDLE_INVARIANTS: &[(&str, &str, bool, &str, &str)] = &[\n")
        for e in (bsd.get("snapshot") or {}).get("element", []):
            for c in e.get("constraint", []):
                if c.get("key", "").startswith("bdl") and c.get("expression"):
                    f.write(f'    ({rs(e["path"])}, {rs(c["key"])}, {str(c.get("severity") == "error").lower()}, {rs(c.get("human", ""))}, {rs(c["expression"])}),\n')
        f.write("];\n\n")
        # Every resource's own invariants (org-1, pat-1, cmp-1, obs-6 …), on
        # the root or on any element: what a resource owes when no profile
        # applies (a profile's snapshot restates them). dom-*/ele-*/ext-*
        # are checked elsewhere; bdl-* above.
        f.write("/// Every resource's own invariants, root and element: (resource type,\n")
        f.write("/// element path, key, is_error, human, expression, the element's types —\n")
        f.write("/// for a choice `[x]`, the concrete keys to evaluate).\n")
        f.write("#[allow(clippy::type_complexity)]\n")
        f.write("pub static RESOURCE_INVARIANTS: &[(&str, &str, &str, bool, &str, &str, &[&str])] = &[\n")
        for t in types:
            sd = sds[t]
            if sd.get("kind") != "resource" or t in ("Bundle", "DomainResource", "Resource"):
                continue
            seen = set()
            for e in (sd.get("snapshot") or {}).get("element", []):
                if ":" in e.get("id", ""):
                    continue
                path = e.get("path", "")
                for c in e.get("constraint", []):
                    key, expr = c.get("key", ""), c.get("expression")
                    if not key or not expr or key.split("-")[0] in ("dom", "ele", "ext", "bdl") or (path, key) in seen:
                        continue
                    seen.add((path, key))
                    etypes = [t] if "." not in path else [x["code"] for x in e.get("type", []) if x.get("code")]
                    tys = ", ".join(rs(x) for x in etypes)
                    f.write(f'    ({rs(t)}, {rs(path)}, {rs(key)}, {str(c.get("severity") == "error").lower()}, {rs(c.get("human", ""))}, {rs(expr)}, &[{tys}]),\n')
        f.write("];\n\n")
        # Every datatype's own invariants (cpt-2, per-1, qty-3, rng-2, tim-* …):
        # what an element of that type owes wherever it sits — a resource's
        # own element, an extension's value, a datatype inside a datatype —
        # so the shape walk evaluates them at every typed object it visits.
        # The context is spelled as the walk's typing context: the type
        # itself, or the bare `Type.path` of an inline element. Constraints on a
        # primitive-typed element (eld-3, txt-1) have no object to visit;
        # ele-/ext- are structural, ref-1 is the walk's own contained check.
        f.write("/// Every datatype's own invariants: (typing context, key, is_error, human,\n")
        f.write("/// expression) — evaluated on every object of that context the shape walk meets;\n")
        f.write("/// a backbone or inline element's context is its bare path (`Timing.repeat`).\n")
        f.write("pub static DATATYPE_INVARIANTS: &[(&str, &str, bool, &str, &str)] = &[\n")
        for t in types:
            sd = sds[t]
            if sd.get("kind") != "complex-type":
                continue
            seen = set()
            for e in (sd.get("snapshot") or {}).get("element", []):
                if ":" in e.get("id", ""):
                    continue
                path = e.get("path", "")
                etypes = [x["code"] for x in e.get("type", []) if x.get("code")]
                if "." in path and (not etypes or etypes[0][:1].islower()):
                    continue
                ctx = path  # the walk's context for an inline element is its bare path
                for c in e.get("constraint", []):
                    key, expr = c.get("key", ""), c.get("expression")
                    if not key or not expr or key.split("-")[0] in ("ele", "ext", "ref") or "htmlChecks" in expr or (path, key) in seen:
                        continue
                    seen.add((path, key))
                    f.write(f'    ({rs(ctx)}, {rs(key)}, {str(c.get("severity") == "error").lower()}, {rs(c.get("human", ""))}, {rs(expr)}),\n')
        f.write("];\n\n")
        # The datatype profiles the base package defines (`SimpleQuantity`,
        # `MoneyQuantity`): what an element whose type names one owes beyond
        # its type. R4 spells such an element `{"code": "Quantity", "profile":
        # [".../SimpleQuantity"]}`, so the type code alone (above) reads it as
        # a plain Quantity. A profile here is a constraint on a datatype whose
        # differential the shape walk applies whole: invariants on the root,
        # elements prohibited (`max 0`) directly under it. A profile any
        # other way (`elementdefinition-de` slices its extensions) is left
        # out, and one a base element names is refused below if it is not
        # applicable whole — never applied in part.
        profiles, skipped = applicable_datatype_profiles(pkg, {t for t in types if sds[t]["kind"] == "complex-type"})
        index = {url: i for i, (url, _, _, _, _, _) in enumerate(profiles)}
        element_profiles = element_datatype_profiles(types, sds, index, profiles, skipped)
        f.write("/// The datatype profiles the base package defines, each as the shape walk\n")
        f.write("/// applies it: (name, canonical `url|version` — as the reference validator\n")
        f.write("/// names it —, the datatype it constrains, the elements it prohibits\n")
        f.write("/// (`max 0`), its own invariants (key, is_error, human, expression)).\n")
        if skipped:
            f.write("/// Not applicable whole, so not listed (no base element names them):\n")
            for s in sorted(skipped):
                f.write(f"/// `{s}`\n")
        f.write("#[allow(clippy::type_complexity)]\n")
        f.write("pub static DATATYPE_PROFILES: &[(&str, &str, &str, &[&str], &[(&str, bool, &str, &str)])] = &[\n")
        for url, version, name, ty, prohibited, invariants in profiles:
            inv = ", ".join(f"({rs(k)}, {str(err).lower()}, {rs(h)}, {rs(x)})" for k, err, h, x in invariants)
            f.write(f'    ({rs(name)}, {rs(url + "|" + version)}, {rs(ty)}, &[{", ".join(rs(p) for p in prohibited)}], &[{inv}]),\n')
        f.write("];\n\n")
        f.write(f"/// Every base element whose type names one of `DATATYPE_PROFILES` ({len(element_profiles)} in\n")
        f.write("/// this package): (typing context of its parent, element key — a choice's\n")
        f.write("/// concrete key, `doseQuantity` —, index into `DATATYPE_PROFILES`), sorted\n")
        f.write("/// by context then key.\n")
        f.write("pub static ELEMENT_PROFILES: &[(&str, &str, u8)] = &[\n")
        for ctx, key, i in element_profiles:
            f.write(f"    ({rs(ctx)}, {rs(key)}, {i}),\n")
        f.write("];\n\n")
        # Every required binding the base definitions place on a coded
        # element — a resource's own (`Patient.gender`), a backbone's
        # (`Encounter.location.status`), a datatype's wherever it sits
        # (`ContactPoint.system`, `Timing.repeat.periodUnit`) — keyed by the
        # typing context the shape walk is in and the element's key. A
        # choice element is left out (none in R4 binds `[x]` required).
        f.write("/// Every required binding the base definitions place on a coded element:\n")
        f.write("/// (typing context, element key, binding) — checked by the shape walk at\n")
        f.write("/// every object of that context, profile or none (a datatype's), or with\n")
        f.write("/// none (a resource's, which a profile's snapshot restates).\n")
        f.write("pub static BASE_BINDINGS: &[(&str, &str, Binding)] = &[\n")
        for t in types:
            ctx_of = {t: t}
            for e in sds[t]["snapshot"]["element"]:
                codes = [x.get("code") for x in e.get("type", [])]
                if codes in (["BackboneElement"], ["Element"]):
                    ctx_of[e["path"]] = e["path"]  # the walk's context: the bare path
            for e in sds[t]["snapshot"]["element"]:
                path = e["path"]
                b = e.get("binding") or {}
                if "." not in path or path.endswith("[x]") or b.get("strength") != "required" or not b.get("valueSet"):
                    continue
                parent, key = path.rsplit(".", 1)
                codes = [x.get("code") for x in e.get("type", [])]
                kind = {"code": "Code", "Coding": "Coding", "CodeableConcept": "CodeableConcept"}.get(codes[0] if len(codes) == 1 else None)
                if not kind or parent not in ctx_of:
                    continue
                vs = b["valueSet"].split("|")[0]
                f.write(f'    ({rs(ctx_of[parent])}, {rs(key)}, Binding {{ value_set: {rs(vs)}, kind: BindingKind::{kind} }}),\n')
        f.write("];\n\n")
        # the resource types a base Reference element may point at — what
        # `Reference.type` is held to (the reference validator checks the
        # declared type against the element's targets even with no profile)
        f.write("/// The target resource types of every base Reference element with named\n")
        f.write("/// targets: (element path, `Type|Type…`). An element that may point at\n")
        f.write("/// any resource is left out.\n")
        f.write("pub static REFERENCE_TARGETS: &[(&str, &str)] = &[\n")
        base_sd = "http://hl7.org/fhir/StructureDefinition/"
        for t in types:
            if sds[t]["kind"] != "resource":
                continue
            for e in sds[t]["snapshot"]["element"]:
                refs = [x for x in e.get("type", []) if x.get("code") == "Reference"]
                if not refs or "." not in e["path"]:
                    continue
                targets = []
                for x in refs:
                    for tp in x.get("targetProfile", []):
                        if not tp.startswith(base_sd):
                            continue
                        targets.append(tp[len(base_sd):])
                if not targets or any(x in ("Resource", "DomainResource") for x in targets):
                    continue
                f.write(f'    ({rs(e["path"])}, {rs("|".join(sorted(set(targets))))}),\n')
        f.write("];\n\n")
        f.write("/// The distinct type codes: `type|type…`, `#Backbone.path` (a backbone or\n")
        f.write("/// content reference), `~Element.path` (an inline Element).\n")
        f.write("pub static CODES: &[&str] = &[\n")
        for code in dictionary:
            f.write(f'    "{code}",\n')
        f.write("];\n\n")
        f.write("/// (relative path, `CODES` index, repeats, required).\n")
        f.write("pub static TABLE: &[(&str, u16, bool, bool)] = &[\n")
        for t in types:
            for rel, code, repeats, required in groups[t]:
                f.write(f'    ("{rel}", {dictionary[code]}, {str(repeats).lower()}, {str(required).lower()}),\n')
        f.write("];\n")
    print(f"wrote {out_path}: {n} entries, {len(types)} types, {len(dictionary)} codes, "
          f"{len(profiles)} datatype profiles on {len(element_profiles)} elements")


if __name__ == "__main__":
    main()
