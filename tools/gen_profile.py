#!/usr/bin/env python3
"""Generate a Rust `CompiledProfile` from a FHIR StructureDefinition snapshot.

The codegen step of fhir-pods: read the pre-computed `snapshot.element` the
published FHIR/AU-Core packages already ship, and emit a compiled, embeddable
profile (our own IR — no FHIRSchema dependency). See docs/domain-model.md.

Emits:
  * one `ElementConstraint` per non-sliced element carrying an enforceable
    constraint — cardinality (min / bounded max), a scalar fixed value, or a
    required-strength binding on a code. Nested paths are emitted; the engine
    evaluates cardinality per parent instance.
  * one `Slicing` per sliced element with at least one discriminable slice.
    A slice's discriminators are resolved from the snapshot:
      - `type` on `$this`     → the concrete choice key (`valueQuantity`)
      - `value` / `pattern`   → the slice sub-element's scalar fixed value at the
                                 discriminator path (or, for extension slices
                                 discriminated by `url`, the slice's type profile)
    Slice sub-element constraints are emitted with `$this.`-relative paths and
    validated against each matched instance.

Deferred (counted, not emitted): nested slicing (an id with two ':'), slices
whose discriminator value isn't in the snapshot (e.g. `pattern` on `type` where
the slice only references an external profile), non-scalar fixed/pattern
objects, and mustSupport-only elements.

Licence rule: only scalar fixed[x] values are emitted (uri/code/string). A fixed
SNOMED coding therefore contributes `system` + `code` — its display is never
emitted (the code is a reference; the display is licensed content).

Usage:
    gen_profile.py <StructureDefinition.json> <out.rs> "<package id version>"
"""
import json
import os
import sys

SCALAR = (str, int, float)


def scalar_fixed(e):
    """The element's scalar fixed[x] / pattern[x] value, if any."""
    for k, v in e.items():
        if (k.startswith("fixed") or k.startswith("pattern")) and isinstance(v, SCALAR) and not isinstance(v, bool):
            return str(v)
    return None


def has_object_fixed(e):
    return any(
        (k.startswith("fixed") or k.startswith("pattern")) and isinstance(v, (dict, list))
        for k, v in e.items()
    )


def constraint(e):
    """(min, max|None, fixed, required_binding, enforceable) for an element."""
    min_ = e.get("min", 0) or 0
    max_ = e.get("max")
    bounded = max_ is not None and max_ != "*"
    fixed = scalar_fixed(e)
    binding = e.get("binding") or {}
    types = [t.get("code") for t in e.get("type", [])]
    # A required binding on a coded element. The kind says how the engine
    # reads the instance: a `code` is the code itself; a Coding is
    # system+code; a CodeableConcept is a member if any of its codings is.
    bind = None
    if binding.get("strength") == "required" and binding.get("valueSet"):
        for kind in ("code", "Coding", "CodeableConcept"):
            if kind in types:
                bind = (binding["valueSet"], kind)
                break
    enforceable = min_ >= 1 or bounded or fixed is not None or bind is not None
    return min_, (int(max_) if bounded else None), fixed, bind, enforceable


def merge(rules, key, min_, max_, fixed, bind):
    r = rules.setdefault(key, {"min": 0, "max": None, "fixed": None, "binding": None})
    r["min"] = max(r["min"], min_)
    if max_ is not None:
        r["max"] = max_ if r["max"] is None else min(r["max"], max_)
    if fixed is not None:
        r["fixed"] = fixed
    if bind:
        r["binding"] = bind


def choice_key(sliced_path, type_code):
    """`Observation.value[x]` + `Quantity` → `valueQuantity`."""
    base = sliced_path.rsplit(".", 1)[-1].removesuffix("[x]")
    return base + type_code[0].upper() + type_code[1:]


def flatten(obj, prefix=""):
    """Leaf (relative path, scalar) pairs of a pattern/fixed object. Arrays
    contribute their elements' leaves — a pattern on a repeating element means
    'contains an element matching this'."""
    out = []
    if isinstance(obj, dict):
        for k, v in obj.items():
            out += flatten(v, f"{prefix}.{k}" if prefix else k)
    elif isinstance(obj, list):
        for v in obj:
            out += flatten(v, prefix)
    elif isinstance(obj, SCALAR) and not isinstance(obj, bool):
        out.append((prefix, str(obj)))
    return out


def profile_discriminators(ref, dpath):
    """What an external profile fixes at `dpath` (relative to its root type):
    its pattern/fixed object flattened, plus scalar fixeds on `dpath`'s
    children. These become the slice's Value discriminators."""
    if not ref:
        return []
    root = ref.get("type")
    want = f"{root}.{dpath}"
    pairs = []
    for e in (ref.get("snapshot") or {}).get("element", []):
        p, eid = e.get("path", ""), e.get("id", "")
        if ":" in eid:
            continue
        if p == want:
            for k, v in e.items():
                if k.startswith(("pattern", "fixed")):
                    pairs += [(f"{dpath}.{sub}" if sub else dpath, val) for sub, val in flatten(v)]
        elif p.startswith(want + "."):
            fx = scalar_fixed(e)
            if fx is not None:
                pairs.append((p[len(root) + 1:], fx))
    seen, out = set(), []
    for pair in pairs:
        if pair not in seen:
            seen.add(pair)
            out.append(pair)
    return out


# JSON-shape rules the base spec attaches to every element; not profile
# semantics, and they use FHIRPath outside the core (children(), value xor).
STRUCTURAL_INVARIANTS = {"ele-1", "ext-1"}


def invariants_of(sd):
    """(path, key, severity, human, expression) for every constraint on a
    non-sliced element, including the root. Slice-scoped constraints are left
    out: evaluated on every instance of the path they would over-apply."""
    out, seen = [], set()
    for e in (sd.get("snapshot") or {}).get("element", []):
        if ":" in e.get("id", ""):
            continue
        path = e.get("path", "")
        for c in e.get("constraint", []):
            key, expr = c.get("key"), c.get("expression")
            if not key or not expr or key in STRUCTURAL_INVARIANTS or (path, key) in seen:
                continue
            seen.add((path, key))
            # The element's type code(s): what `$this is dateTime` / `$this as
            # dateTime` resolve against, and — for a choice `[x]` — which
            # concrete keys the engine enumerates.
            types = [t.get("code") for t in e.get("type", []) if t.get("code")]
            if "." not in path:
                types = [sd.get("type")]  # the root: its type is the resource
            out.append((path, key, c.get("severity", "error"), c.get("human", ""), expr, types))
    return out


def must_support_of(sd):
    """Paths of plain (unsliced) elements flagged mustSupport. Not a validation
    rule — validators don't check it — but a client conforming to the IG is
    expected to populate them, so the engine can report gaps as information."""
    out = []
    for e in (sd.get("snapshot") or {}).get("element", []):
        path, eid = e.get("path", ""), e.get("id", "")
        if "." in path and ":" not in eid and e.get("mustSupport"):
            out.append(path)
    return out


def build(sd, load=None):
    """`load(url)` resolves a referenced profile's StructureDefinition (or None)
    so a slice can be discriminated by, and inherit, an external profile."""
    resource_type = sd["type"]
    elements = {}     # path -> rule
    slicings = {}     # sliced path -> {"closed", "disc": [(type, path)], "slices": {name: {...}}}
    deferred = 0

    for e in (sd.get("snapshot") or {}).get("element", []):
        path, eid = e.get("path", ""), e.get("id", "")
        if "." not in path:
            continue
        if "slicing" in e:
            s = e["slicing"]
            sg = slicings.setdefault(path, {"closed": False, "disc": [], "slices": {}})
            sg["closed"] = s.get("rules") == "closed"
            sg["disc"] = [(d.get("type"), d.get("path")) for d in s.get("discriminator", [])]

        if ":" in eid:
            head, tail = eid.split(":", 1)
            if ":" in tail:                                  # nested slicing
                # Not enforced yet — but a nested slice's scalar fixed values are
                # what an OUTER slice's `value` discriminator resolves to. BP:
                # component:SystolicBP is discriminated by code.coding.code,
                # whose value lives in component:SystolicBP.code.coding:SBPCode
                # .code. Record them keyed by the label-stripped relative path.
                deferred += 1
                name = tail.split(".", 1)[0].split(":", 1)[0]
                sg = slicings.setdefault(head, {"closed": False, "disc": [], "slices": {}})
                sl = sg["slices"].setdefault(name, {"min": 0, "max": None, "type": None, "profile": None, "subs": {}})
                fixed = scalar_fixed(e)
                if fixed is not None and path.startswith(head + "."):
                    sl.setdefault("deep", {}).setdefault(path[len(head) + 1:], []).append(fixed)
                continue
            sg = slicings.setdefault(head, {"closed": False, "disc": [], "slices": {}})
            if "." not in tail:                              # the slice element itself
                types = e.get("type", [])
                sl = sg["slices"].setdefault(tail, {"min": 0, "max": None, "type": None, "profile": None, "subs": {}})
                sl["min"] = e.get("min", 0) or 0
                mx = e.get("max")
                sl["max"] = int(mx) if mx not in (None, "*") else None
                sl["type"] = types[0].get("code") if types else None
                sl["profile"] = (types[0].get("profile") or [None])[0] if types else None
                if has_object_fixed(e):
                    # The slice element's own pattern (a patternCodeableConcept
                    # on `category:us-core`, say) — what a `pattern $this`
                    # discriminator resolves to. Keyed by the empty path.
                    for k, v in e.items():
                        if k.startswith(("pattern", "fixed")) and isinstance(v, (dict, list)):
                            sl.setdefault("objpat", {})[""] = v
                continue
            name, _ = tail.split(".", 1)                     # a slice sub-element
            sl = sg["slices"].setdefault(name, {"min": 0, "max": None, "type": None, "profile": None, "subs": {}})
            if not path.startswith(head + "."):
                deferred += 1
                continue
            rel = path[len(head) + 1:]
            if has_object_fixed(e):
                # Not enforced (pattern semantics differ from fixed), but the
                # slice can be discriminated by it.
                for k, v in e.items():
                    if k.startswith(("pattern", "fixed")) and isinstance(v, (dict, list)):
                        sl.setdefault("objpat", {})[rel] = v
                deferred += 1
                continue
            min_, max_, fixed, bind, enforceable = constraint(e)
            if enforceable:
                merge(sl["subs"], rel, min_, max_, fixed, bind)
            elif e.get("mustSupport"):
                deferred += 1
            continue

        # a plain, non-sliced element
        if has_object_fixed(e):
            deferred += 1
            continue
        min_, max_, fixed, bind, enforceable = constraint(e)
        if enforceable:
            merge(elements, path, min_, max_, fixed, bind)
        elif e.get("mustSupport") or (e.get("binding") or {}).get("strength") in ("required", "extensible"):
            deferred += 1

    out_slicings = []
    for sliced_path, sg in slicings.items():
        emitted = []
        for name, sl in sg["slices"].items():
            discs, ok = [], bool(sg["disc"])
            for dtype, dpath in sg["disc"]:
                if dtype == "type" and dpath == "$this":
                    if not sl["type"]:
                        ok = False
                        break
                    discs.append(("type", choice_key(sliced_path, sl["type"])))
                elif dtype in ("value", "pattern"):
                    sub = sl["subs"].get(dpath)
                    val = sub["fixed"] if sub else None
                    if val is None:
                        # Fall through to a value fixed inside a nested slice
                        # (first in snapshot order — the canonical coding).
                        deep = sl.get("deep", {}).get(dpath)
                        if deep:
                            val = deep[0]
                    if val is None and dpath == "url" and sl["profile"]:
                        val = sl["profile"]
                    if val is None:
                        # The slice's own object pattern at the discriminator
                        # path (a patternCodeableConcept on `type`, say), or on
                        # the slice element itself when the path is `$this`.
                        key = "" if dpath == "$this" else dpath
                        obj = sl.get("objpat", {}).get(key)
                        if obj:
                            discs.extend(
                                ("value", "$this" + "".join(f".{seg}" for seg in (key, s) if seg), v)
                                for s, v in flatten(obj)
                            )
                            continue
                    if val is None and sl["profile"] and load:
                        # The slice inherits an external profile: discriminate
                        # by what that profile fixes at the discriminator path
                        # (e.g. au-ihi's `type` pattern → coding NI).
                        pairs = profile_discriminators(load(sl["profile"]), dpath)
                        if pairs:
                            discs.extend(("value", "$this." + p, v) for p, v in pairs)
                            continue
                    if val is None:
                        ok = False
                        break
                    discs.append(("value", "$this." + dpath, val))
                else:
                    ok = False
                    break
            if not ok:
                deferred += 1
                continue
            emitted.append((name, sl, discs))
        if emitted:
            out_slicings.append((sliced_path, sg["closed"], emitted))
    return resource_type, elements, out_slicings, deferred


def rs(s):
    return '"' + str(s).replace("\\", "\\\\").replace('"', '\\"') + '"'


def el_line(path, r, indent):
    mx = f"Max::Bounded({r['max']})" if r["max"] is not None else "Max::Star"
    fx = f"Some({rs(r['fixed'])})" if r["fixed"] is not None else "None"
    if r["binding"]:
        vs, kind = r["binding"]
        bd = f"Some(Binding {{ value_set: {rs(vs)}, kind: BindingKind::{kind[0].upper() + kind[1:]} }})"
    else:
        bd = "None"
    return (
        " " * indent
        + f"ElementConstraint {{ path: {rs(path)}, min: {r['min']}, max: {mx}, fixed: {fx}, required_binding: {bd} }},"
    )


def emit(sd, provenance, load=None):
    resource_type, elements, slicings, deferred = build(sd, load)
    name = sd.get("title") or sd.get("name") or sd.get("id")
    n_slices = sum(len(s[2]) for s in slicings)
    invariants = invariants_of(sd)
    imports = "CompiledProfile, ElementConstraint, Max"
    has_binding = any(r["binding"] for r in elements.values()) or any(
        r["binding"] for _, _, sl in slicings for _, spec, _ in sl for r in spec["subs"].values()
    )
    if has_binding:
        imports += ", Binding, BindingKind"
    if slicings:
        imports += ", Discriminator, Slice, Slicing"
    if invariants:
        imports += ", InvariantSeverity, ProfileInvariant"
    out = [
        f"//! @generated by tools/gen_profile.py from {provenance} — do not edit by hand.",
        f"//! Profile: {name}  <{sd.get('url')}>",
        f"//! Coverage: {len(elements)} element constraint(s) + {n_slices} slice(s) + {len(invariants)} invariant(s) emitted; {deferred} deferred",
        "//! (nested slicing / undiscriminable slices / object patterns / mustSupport-only).",
        "//! Invariants outside the supported FHIRPath core are skipped at runtime.",
        "",
        f"use crate::compiled::{{{imports}}};",
        "",
        "pub static PROFILE: CompiledProfile = CompiledProfile {",
        f"    url: {rs(sd.get('url'))},",
        f"    name: {rs(name)},",
        f"    resource_type: {rs(resource_type)},",
        f"    deferred: {deferred},",
        "    elements: &[",
    ]
    for path in sorted(elements):
        out.append(el_line(path, elements[path], 8))
    out += ["    ],", "    slicings: &["]
    for sliced_path, closed, slices in slicings:
        out.append(f"        Slicing {{ path: {rs(sliced_path)}, closed: {str(closed).lower()}, slices: &[")
        for sname, sl, discs in slices:
            mx = f"Max::Bounded({sl['max']})" if sl["max"] is not None else "Max::Star"
            prof = f"Some({rs(sl['profile'])})" if sl.get("profile") else "None"
            out.append(f"            Slice {{ name: {rs(sname)}, min: {sl['min']}, max: {mx}, profile: {prof}, discriminators: &[")
            for d in discs:
                if d[0] == "type":
                    out.append(f"                Discriminator::Type {{ key: {rs(d[1])} }},")
                else:
                    out.append(f"                Discriminator::Value {{ path: {rs(d[1])}, value: {rs(d[2])} }},")
            out.append("            ], elements: &[")
            for rel in sorted(sl["subs"]):
                out.append(el_line("$this." + rel, sl["subs"][rel], 16))
            out.append("            ] },")
        out.append("        ] },")
    out += ["    ],", "    invariants: &["]
    for path, key, severity, human, expr, types in invariants:
        sev = "InvariantSeverity::Error" if severity == "error" else "InvariantSeverity::Warning"
        tys = ", ".join(rs(t) for t in types)
        out.append(
            f"        ProfileInvariant {{ path: {rs(path)}, key: {rs(key)}, severity: {sev}, "
            f"human: {rs(human)}, expression: {rs(expr)}, types: &[{tys}] }},"
        )
    out += ["    ],", "    must_support: &["]
    for path in must_support_of(sd):
        out.append(f"        {rs(path)},")
    out += ["    ],", "};", ""]
    return "\n".join(out)


def main():
    sd_path, out_path, provenance = sys.argv[1], sys.argv[2], sys.argv[3]
    sd = json.load(open(sd_path))
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    with open(out_path, "w") as f:
        f.write(emit(sd, provenance))
    _, elements, slicings, deferred = build(sd)
    n_slices = sum(len(s[2]) for s in slicings)
    print(f"wrote {out_path}: {len(elements)} elements + {n_slices} slices enforced, {deferred} deferred")


if __name__ == "__main__":
    main()
