#!/usr/bin/env python3
"""Batch-generate compiled profiles from one or more FHIR packages into
`src/generated/`, plus a `mod.rs` declaring each module and exposing a
`REGISTRY` of all compiled profiles — looked up at runtime by canonical URL.

The first N packages (`--primaries N`, default 1) are primary: their resource
profiles are generated. Every package contributes to the URL index that lets a
slice discriminate by, and inherit, an external profile — and the `Identifier`
and complex `Extension` datatype profiles from every package are generated so
those slices have something to conform to (e.g. `hl7.fhir.au.base`'s `au-ihi`
/ `au-medicarecardnumber`, US Core's `us-core-race`).

Usage:
    gen_all.py [--primaries N] <out/dir> "<provenance>" <primary pkg> [<pkg> ...]
"""
import glob
import re
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen_profile  # noqa: E402
from gen_profile import emit, emit_intern  # noqa: E402


BASE_VITALS = {
    "vitalsigns", "bp", "bodyheight", "bodyweight", "bodytemp", "bmi", "headcircum", "heartrate",
    "oxygensat", "resprate", "vitalspanel",
}
# The base spec's own resource definitions compiled as profiles, so a host can
# lower (and declare) them directly: the audit surface.
# Base resource definitions compiled as lowerable profiles: AuditEvent /
# Provenance for audit (0.6.0), and since 0.11.0 the resources a patient
# record is made of — the region-neutral vocabulary a host lowers into
# before `export` re-profiles for a jurisdiction.
BASE_RESOURCES = {
    "AuditEvent", "Provenance",
    "Patient", "Observation", "Condition", "AllergyIntolerance", "MedicationStatement", "MedicationRequest",
    "Medication", "Immunization", "Procedure", "Device", "DeviceUseStatement", "Practitioner", "Organization",
    # since 0.19.0: a form and its answers, so a host lowers a
    # QuestionnaireResponse it is then held to (0.16.0) — and the form itself
    "Questionnaire", "QuestionnaireResponse",
}



RUST_KEYWORDS = {"as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl",
                 "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "static", "struct",
                 "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn",
                 # reserved for future use
                 "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized",
                 "virtual", "yield", "try", "gen"}



def package_files(pkg, resource_type):
    """Every JSON file in a package dir whose `resourceType` is the one asked
    for, sorted. By content, not by file name: the HL7 packages name files
    `StructureDefinition-*.json`, UK Core `UKCore-*.json` /
    `Extension-*.json`, CA Baseline `structuredefinition-profile-*.json`."""
    out = []
    for path in sorted(glob.glob(os.path.join(pkg, "*.json"))):
        try:
            with open(path, "rb") as fh:
                head = fh.read(4096)
        except OSError:
            continue
        m = re.search(rb'"resourceType"\s*:\s*"([A-Za-z]+)"', head)
        if m and m.group(1).decode() == resource_type:
            out.append(path)
    return out

def mod_name(sd_id):
    """A profile id as a Rust module name: lower snake case, never starting
    with a digit, never a keyword."""
    m = "".join(c if c.isalnum() else "_" for c in sd_id).lower()
    if m[:1].isdigit():
        m = "p_" + m
    if m in RUST_KEYWORDS:
        m = m + "_"
    return m

def main():
    args = sys.argv[1:]
    primaries = 1
    base_dir = None
    if args[:1] == ["--primaries"]:
        primaries, args = int(args[1]), args[2:]
    if args[:1] == ["--base"]:
        base_dir, args = args[1], args[2:]
    out_dir, provenance, pkg_dirs = args[0], args[1], args[2:]
    if not pkg_dirs:
        sys.exit("usage: gen_all.py [--primaries N] <out/dir> <provenance> <primary pkg> [<pkg> ...]")
    os.makedirs(out_dir, exist_ok=True)

    # canonical URL -> file, across every package (the resolver's index)
    index, cache = {}, {}
    for pkg in pkg_dirs + ([base_dir] if base_dir else []):
        for path in package_files(pkg, "StructureDefinition"):
            sd = json.load(open(path))
            if sd.get("resourceType") == "StructureDefinition" and sd.get("url"):
                index.setdefault(sd["url"], path)

    def load(url):
        if url not in index:
            return None
        if url not in cache:
            cache[url] = json.load(open(index[url]))
        return cache[url]

    def is_profile(sd):
        return sd.get("derivation") == "constraint" and (sd.get("snapshot") or {}).get("element")

    # Datatype profiles a slice can inherit — Identifier (AU Base's IHI,
    # Medicare, DVA…) and complex Extensions (US Core's race / ethnicity…,
    # whose sub-extensions carry their own cardinality). From a primary IG,
    # all of them; from a dependency, only those a compiled profile
    # references (transitively), so a big extension package costs nothing.
    def datatype_profile(sd):
        return sd.get("kind") == "complex-type" and sd.get("type") in ("Identifier", "Extension")

    targets = []
    for i, pkg in enumerate(pkg_dirs[:primaries]):
        for path in package_files(pkg, "StructureDefinition"):
            sd = json.load(open(path))
            if is_profile(sd) and (sd.get("kind") == "resource" or datatype_profile(sd)):
                targets.append(sd)
    # what the primaries reference (a slice's datatype profile) or derive from
    # (their baseDefinition chain — an example may declare the base too)
    referenced, stack = set(), list(targets)
    while stack:
        sd = stack.pop()
        urls = [sd["baseDefinition"].split("|")[0]] if sd.get("baseDefinition") else []
        for e in (sd.get("snapshot") or {}).get("element", []):
            urls += [u.split("|")[0] for t in e.get("type", []) for u in t.get("profile", [])]
        for url in urls:
            if url not in referenced and load(url):
                referenced.add(url)
                stack.append(load(url))
    for pkg in pkg_dirs[primaries:]:
        for path in package_files(pkg, "StructureDefinition"):
            sd = json.load(open(path))
            if is_profile(sd) and sd.get("url") in referenced and (datatype_profile(sd) or sd.get("kind") == "resource"):
                targets.append(sd)
    # An IG's own datatype profile of any other type (`NzAddress`,
    # `NzContactpoint`, `au-dosage`, `CodeableConcept-uv-ips` and the
    # `Coding-uv-ips` it names) that a compiled profile holds an element to
    # alone: compiled too, from any package, so the engine can apply it
    # whole there (gen_profile.ig_datatype_profile). One only ever listed
    # beside its base datatype (`au-address`) has nothing to apply and is not
    # compiled; nor is the base package's own (`SimpleQuantity`), the walk's.
    gen_profile.LOAD = load
    have = {sd["url"] for sd in targets}
    over_base = set()
    stack = list(targets)
    while stack:
        sd = stack.pop()
        for e in (sd.get("snapshot") or {}).get("element", []):
            for t in e.get("type", []):
                code = t.get("code")
                if code in ("Extension", "Resource", "DomainResource") or code not in gen_profile.DATATYPE_CODES:
                    continue
                url = gen_profile.ig_datatype_profile(code, [p.split("|")[0] for p in t.get("profile", [])])
                dt = load(url) if url and " " not in url else None
                if dt and url not in have and is_profile(dt) and dt.get("kind") == "complex-type":
                    targets.append(dt)
                    have.add(url)
                    over_base.add(url)
                    stack.append(dt)

    # The base spec's vital-sign profiles: the reference validator applies
    # them to any Observation carrying their LOINC code (or, for the generic
    # one, the vital-signs category) because the spec requires it — so they
    # are compiled and applied here too, as `IMPLIED`.
    implied = []
    if base_dir:
        for path in package_files(base_dir, "StructureDefinition"):
            sd = json.load(open(path))
            if sd.get("id") in BASE_VITALS and (sd.get("snapshot") or {}).get("element"):
                targets.append(sd)
                implied.append(sd["url"])
            elif sd.get("id") in BASE_RESOURCES and (sd.get("snapshot") or {}).get("element"):
                targets.append(sd)

    mods = []
    used = {"intern", "mod"}  # the directory's own modules
    seen_urls = {}
    implied_mods = []
    for sd in targets:
        if sd["url"] in seen_urls:
            sys.exit(f"{sd['url']} is defined twice ({seen_urls[sd['url']]} and {sd.get('id')}): one registry entry per canonical")
        seen_urls[sd["url"]] = sd.get("id")
        # Resource.id is optional; a canonical still identifies an ad-hoc SD.
        sd_id = sd.get("id") or sd["url"].rstrip("/").rsplit("/", 1)[-1] or "profile"
        mod = mod_name(sd_id)
        # the same id in two packages (AU Base's and HL7's `medication-type`): distinct modules
        n = 2
        while mod in used:
            mod = f"{mod_name(sd_id)}_{n}"
            n += 1
        used.add(mod)
        if sd["url"] in implied:
            implied_mods.append(mod)
        # an IG datatype profile compiled for the elements held to it: only
        # what it adds over its base (the walk holds every instance to that)
        text, (ne, ns, nd) = emit(sd, provenance, load, differential=sd["url"] in over_base)
        with open(os.path.join(out_dir, f"{mod}.rs"), "w") as f:
            f.write(text)
        mods.append((mod, sd.get("title") or sd_id, ne, ns, nd))
    mods.sort()
    with open(os.path.join(out_dir, "intern.rs"), "w") as f:
        f.write(emit_intern())

    with open(os.path.join(out_dir, "mod.rs"), "w") as f:
        f.write("//! @generated by tools/gen_all.py — do not edit by hand.\n")
        f.write(f"//! {len(mods)} compiled profiles from {provenance}.\n\n")
        f.write("use crate::compiled::CompiledProfile;\n\n")
        f.write("pub mod intern;\n")
        for mod, *_ in mods:
            f.write(f"pub mod {mod};\n")
        f.write("\n/// Every compiled profile, looked up by canonical URL (`meta.profile`, or a slice's `profile`).\n")
        f.write("pub static REGISTRY: &[&CompiledProfile] = &[\n")
        for mod, *_ in mods:
            f.write(f"    &{mod}::PROFILE,\n")
        f.write("];\n")
        f.write("\n/// Every element a compiled profile holds to a base datatype profile its\n")
        f.write("/// base definition does not name (`SimpleQuantity` on an extension's\n")
        f.write("/// `value[x]`): (profile url, slice label or `\"\"`, element path — `$this.…`\n")
        f.write("/// inside a slice —, type code, datatype profile canonical). Sparse: the\n")
        f.write("/// engine filters it by profile and slice (`engine::check_datatype_profiles`).\n")
        f.write("pub static DATATYPE_PROFILE_ELEMENTS: &[(&str, &str, &str, &str, &str)] = &[\n")
        for row in sorted(gen_profile.DATATYPE_ROWS):
            f.write("    (" + ", ".join(gen_profile.rs(x) for x in row) + "),\n")
        f.write("];\n")
        f.write("\n/// The base spec's vital-sign profiles, applied to an Observation that carries\n")
        f.write("/// their fixed LOINC code (or the vital-signs category) whether or not it\n")
        f.write("/// declares them — the spec requires it, and the reference validator does it.\n")
        f.write("pub static IMPLIED: &[&CompiledProfile] = &[\n")
        for mod in sorted(implied_mods):
            f.write(f"    &{mod}::PROFILE,\n")
        f.write("];\n")

    for mod, title, ne, ns, nd in mods:
        print(f"  {mod:<34} {ne:>3} el {ns:>2} sl {nd:>2} def  {title}")
    print(f"wrote {len(mods)} profiles + mod.rs -> {out_dir}")


if __name__ == "__main__":
    main()
