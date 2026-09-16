#!/usr/bin/env python3
"""Generate a Rust `CompiledProfile` from a FHIR StructureDefinition snapshot.

The codegen step of fhir-pods: read the pre-computed `snapshot.element` the
published packages already ship, and emit a compiled, embeddable profile (our
own IR — see docs/domain-model.md).

Emits:
  * one `ElementConstraint` per non-sliced element carrying an enforceable
    constraint — cardinality (min / bounded max), a scalar fixed value, or a
    required-strength binding on a coded element. Nested paths are emitted;
    the engine evaluates cardinality per parent instance.
  * one `Slicing` per sliced element with at least one discriminable slice.
    A slice's discriminators are resolved from the snapshot, in order:
      - `type` on `$this`     → the concrete choice key (`valueQuantity`)
      - `value` / `pattern`   → the slice sub-element's scalar fixed value at the
                                 discriminator path → a scalar fixed inside a
                                 nested slice → the extension URL → the slice's
                                 own object pattern → what the slice's external
                                 profile fixes at that path.
    An object pattern (`patternCodeableConcept`) becomes a `Pattern`
    discriminator: ONE instance of the repeating element must carry every leaf.
    Slice sub-element constraints are emitted with `$this.`-relative paths.
  * every `constraint` (FHIRPath invariant) with the element's type codes.
  * the profile's must-support element paths.

Deferred (listed in the file header, not emitted): nested slicing (an id with
two ':'), slices whose discriminator cannot be resolved, the object pattern
itself (its cardinality is kept), and mustSupport-only elements.

Licence rule: only scalar fixed[x] values are emitted (uri/code/string). A
fixed SNOMED coding therefore contributes `system` + `code` — its display is
never emitted (the code is a reference; the display is licensed content).

Usage:
    gen_profile.py <StructureDefinition.json> <out.rs> "<package id version>"
"""
import json
import os
import sys
from dataclasses import dataclass, field

SCALAR = (str, int, float)

# JSON-shape rules the base spec attaches to every element; not profile
# semantics, and they use FHIRPath outside the core (children(), value xor).
STRUCTURAL_INVARIANTS = {"ele-1", "ext-1"}


# --- the snapshot's constraints, read ----------------------------------------

def scalar_fixed(e):
    """The element's scalar fixed[x] / pattern[x] value, if any."""
    for k, v in e.items():
        if k.startswith(("fixed", "pattern")) and isinstance(v, SCALAR) and not isinstance(v, bool):
            return str(v)
    return None


def object_fixed(e):
    """The element's object / array fixed[x] / pattern[x], if any."""
    for k, v in e.items():
        if k.startswith(("fixed", "pattern")) and isinstance(v, (dict, list)):
            return v
    return None


def binding_of(e):
    """`(value_set, kind)` for a required binding on a coded element; the kind
    says how the engine reads the instance (`code` / `Coding` / `CodeableConcept`)."""
    binding = e.get("binding") or {}
    if binding.get("strength") != "required" or not binding.get("valueSet"):
        return None
    types = [t.get("code") for t in e.get("type", [])]
    for kind in ("code", "Coding", "CodeableConcept"):
        if kind in types:
            return (binding["valueSet"], kind)
    return None


@dataclass
class Rule:
    """What the engine enforces on one element path."""
    min: int = 0
    max: int | None = None      # None = unbounded
    fixed: str | None = None
    binding: tuple | None = None

    @staticmethod
    def of(e):
        mx = e.get("max")
        return Rule(
            min=e.get("min", 0) or 0,
            max=int(mx) if mx not in (None, "*") else None,
            fixed=scalar_fixed(e),
            binding=binding_of(e),
        )

    def enforceable(self):
        return self.min >= 1 or self.max is not None or self.fixed is not None or self.binding is not None

    def merge(self, other):
        """Tighten with another rule on the same path (the stricter of each)."""
        self.min = max(self.min, other.min)
        if other.max is not None:
            self.max = other.max if self.max is None else min(self.max, other.max)
        if other.fixed is not None:
            self.fixed = other.fixed
        if other.binding:
            self.binding = other.binding


@dataclass
class SliceSpec:
    min: int = 0
    max: int | None = None
    type: str | None = None
    profile: str | None = None
    subs: dict = field(default_factory=dict)       # relative path -> Rule
    deep: dict = field(default_factory=dict)       # relative path -> [scalar fixed in a nested slice]
    objpat: dict = field(default_factory=dict)     # relative path ("" = the slice itself) -> pattern object


@dataclass
class SlicingSpec:
    closed: bool = False
    disc: list = field(default_factory=list)       # [(type, path)]
    slices: dict = field(default_factory=dict)     # name -> SliceSpec


def choice_key(sliced_path, type_code):
    """`Observation.value[x]` + `Quantity` → `valueQuantity`."""
    base = sliced_path.rsplit(".", 1)[-1].removesuffix("[x]")
    return base + type_code[0].upper() + type_code[1:]


def pattern_groups(obj, anchor="", rel=""):
    """The scalar leaves of a pattern / fixed object, grouped by the innermost
    repeating element each sits in: `(here, groups)` where `here` is this
    level's own leaves and `groups` is `[(anchor, [(rel, value)])]` for every
    array element below. A pattern `{coding: [{system: S, code: C}]}` means
    "some coding carries BOTH S and C" — one group anchored at `coding` — not
    two independent conditions."""
    here, groups = [], []
    if isinstance(obj, dict):
        for k, v in obj.items():
            sub = f"{rel}.{k}" if rel else k
            if isinstance(v, list):
                list_anchor = f"{anchor}.{sub}" if anchor else sub
                for item in v:
                    h, g = pattern_groups(item, list_anchor, "")
                    if h:
                        groups.append((list_anchor, h))
                    groups += g
            else:
                h, g = pattern_groups(v, anchor, sub)
                here += h
                groups += g
    elif isinstance(obj, SCALAR) and not isinstance(obj, bool):
        here.append((rel, str(obj)))
    return here, groups


def groups_of(obj, base=""):
    """All groups of a pattern object placed at `base` (relative to `$this`):
    the root-level leaves as one group anchored at `base`, plus the array
    groups below it."""
    here, groups = pattern_groups(obj)
    out = [(base, here)] if here else []
    for anchor, pairs in groups:
        out.append((f"{base}.{anchor}" if base else anchor, pairs))
    return out


def profile_discriminators(ref, dpath):
    """What an external profile fixes at `dpath` (relative to its root type):
    its pattern/fixed object as groups (see `pattern_groups`), plus scalar
    fixeds on `dpath`'s children. These become the slice's discriminators."""
    if not ref:
        return []
    root = ref.get("type")
    want = f"{root}.{dpath}"
    groups = []
    for e in (ref.get("snapshot") or {}).get("element", []):
        p, eid = e.get("path", ""), e.get("id", "")
        if ":" in eid:
            continue
        if p == want:
            obj = object_fixed(e)
            if obj is not None:
                groups += groups_of(obj, dpath)
            fx = scalar_fixed(e)
            if fx is not None:
                groups.append(("", [(dpath, fx)]))
        elif p.startswith(want + "."):
            fx = scalar_fixed(e)
            if fx is not None:
                groups.append(("", [(p[len(root) + 1:], fx)]))
    seen, out = set(), []
    for g in groups:
        key = (g[0], tuple(g[1]))
        if key not in seen:
            seen.add(key)
            out.append(g)
    return out


def group_discriminators(groups):
    """Discriminator tuples for pattern groups: a lone leaf is a plain value
    check; two or more leaves in one group must be matched by ONE instance of
    the group's anchor element."""
    out = []
    for anchor, pairs in groups:
        base = "$this" + (f".{anchor}" if anchor else "")
        if len(pairs) == 1:
            rel, v = pairs[0]
            out.append(("value", base + (f".{rel}" if rel else ""), v))
        else:
            out.append(("pattern", base, tuple(pairs)))
    return out


def invariants_of(sd):
    """(path, key, severity, human, expression, types) for every constraint on
    a non-sliced element, including the root. Slice-scoped constraints are
    left out: evaluated on every instance of the path they would over-apply."""
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
            # The element's type code(s): what `$this is dateTime` resolves
            # against, and — for a choice `[x]` — which concrete keys the
            # engine enumerates. The root's type is the resource itself.
            types = [sd.get("type")] if "." not in path else [t["code"] for t in e.get("type", []) if t.get("code")]
            out.append((path, key, c.get("severity", "error"), c.get("human", ""), expr, types))
    return out


def must_support_of(sd):
    """Paths of plain (unsliced) elements flagged mustSupport. Not a validation
    rule — validators don't check it — but a client conforming to the IG is
    expected to populate them, so the engine can report gaps as information."""
    return sorted(
        e["path"]
        for e in (sd.get("snapshot") or {}).get("element", [])
        if "." in e.get("path", "") and ":" not in e.get("id", "") and e.get("mustSupport")
    )


# --- collect, then resolve ------------------------------------------------------

@dataclass
class Compiled:
    resource_type: str
    elements: dict                 # path -> Rule
    slicings: list                 # [(sliced path, closed, [(name, SliceSpec, [discriminator tuples])])]
    deferred: list                 # [(reason, element id)]


def collect(sd):
    """One pass over the snapshot: element rules, and every slicing with its
    slices' cardinality, type, profile, sub-rules, nested-slice fixeds and
    object patterns — everything discriminator resolution needs."""
    elements, slicings, deferred = {}, {}, []
    for e in (sd.get("snapshot") or {}).get("element", []):
        path, eid = e.get("path", ""), e.get("id", "")
        if "." not in path:
            continue
        if "slicing" in e:
            sg = slicings.setdefault(path, SlicingSpec())
            sg.closed = e["slicing"].get("rules") == "closed"
            sg.disc = [(d.get("type"), d.get("path")) for d in e["slicing"].get("discriminator", [])]
        if ":" not in eid:
            # a plain element: an object pattern is not enforced, but the cardinality is
            if object_fixed(e) is not None:
                deferred.append(("object pattern", eid))
            rule = Rule.of(e)
            if rule.enforceable():
                elements.setdefault(path, Rule()).merge(rule)
            elif e.get("mustSupport") or (e.get("binding") or {}).get("strength") in ("required", "extensible"):
                deferred.append(("mustSupport / non-required binding only", eid))
            continue

        head, tail = eid.split(":", 1)
        sg = slicings.setdefault(head, SlicingSpec())
        if ":" in tail:
            # Nested slicing is not enforced — but a nested slice's scalar
            # fixed values are what an OUTER slice's `value` discriminator
            # resolves to (BP: component:SystolicBP is discriminated by
            # code.coding.code, whose value lives in
            # component:SystolicBP.code.coding:SBPCode.code).
            deferred.append(("nested slicing", eid))
            name = tail.split(".", 1)[0].split(":", 1)[0]
            sl = sg.slices.setdefault(name, SliceSpec())
            fixed = scalar_fixed(e)
            if fixed is not None and path.startswith(head + "."):
                sl.deep.setdefault(path[len(head) + 1:], []).append(fixed)
            continue
        if "." not in tail:
            # the slice element itself
            types = e.get("type", [])
            sl = sg.slices.setdefault(tail, SliceSpec())
            rule = Rule.of(e)
            sl.min, sl.max = rule.min, rule.max
            sl.type = types[0].get("code") if types else None
            sl.profile = (types[0].get("profile") or [None])[0] if types else None
            obj = object_fixed(e)
            if obj is not None:
                sl.objpat[""] = obj      # what a `pattern $this` discriminator resolves to
            continue
        # a slice sub-element
        name, _ = tail.split(".", 1)
        sl = sg.slices.setdefault(name, SliceSpec())
        if not path.startswith(head + "."):
            deferred.append(("sub-element outside its slice", eid))
            continue
        rel = path[len(head) + 1:]
        obj = object_fixed(e)
        if obj is not None:
            sl.objpat[rel] = obj         # discriminable by, not enforced; cardinality kept
            deferred.append(("object pattern", eid))
        rule = Rule.of(e)
        if rule.enforceable():
            sl.subs.setdefault(rel, Rule()).merge(rule)
        elif e.get("mustSupport"):
            deferred.append(("mustSupport only", eid))
    return elements, slicings, deferred


def resolve_discriminators(sliced_path, sg, sl, load):
    """The slice's discriminator tuples, or None when one cannot be resolved
    from the snapshot (nothing is invented)."""
    discs = []
    if not sg.disc:
        return None
    for dtype, dpath in sg.disc:
        if dtype == "type" and dpath == "$this":
            if not sl.type:
                return None
            discs.append(("type", choice_key(sliced_path, sl.type)))
            continue
        if dtype not in ("value", "pattern"):
            return None
        sub = sl.subs.get(dpath)
        val = sub.fixed if sub else None
        if val is None and sl.deep.get(dpath):
            val = sl.deep[dpath][0]      # first in snapshot order — the canonical coding
        if val is None and dpath == "url" and sl.profile:
            val = sl.profile             # an extension slice: its URL
        if val is None:
            key = "" if dpath == "$this" else dpath
            if sl.objpat.get(key):
                discs.extend(group_discriminators(groups_of(sl.objpat[key], key)))
                continue
        if val is None and sl.profile and load:
            groups = profile_discriminators(load(sl.profile), dpath)
            if groups:
                discs.extend(group_discriminators(groups))
                continue
        if val is None:
            return None
        discs.append(("value", "$this." + dpath, val))
    return discs


def build(sd, load=None):
    """`load(url)` resolves a referenced profile's StructureDefinition (or None)
    so a slice can be discriminated by, and inherit, an external profile."""
    elements, slicings, deferred = collect(sd)
    out_slicings = []
    for sliced_path, sg in slicings.items():
        emitted = []
        for name, sl in sg.slices.items():
            discs = resolve_discriminators(sliced_path, sg, sl, load)
            if discs is None:
                deferred.append(("undiscriminable slice", f"{sliced_path}:{name}"))
                continue
            emitted.append((name, sl, discs))
        if emitted:
            out_slicings.append((sliced_path, sg.closed, emitted))
    return Compiled(sd["type"], elements, out_slicings, deferred)


# --- emit ---------------------------------------------------------------------

def rs(s):
    return '"' + str(s).replace("\\", "\\\\").replace('"', '\\"') + '"'


def max_rs(mx):
    return f"Max::Bounded({mx})" if mx is not None else "Max::Star"


def el_line(path, r, indent):
    fx = f"Some({rs(r.fixed)})" if r.fixed is not None else "None"
    if r.binding:
        vs, kind = r.binding
        bd = f"Some(Binding {{ value_set: {rs(vs)}, kind: BindingKind::{kind[0].upper() + kind[1:]} }})"
    else:
        bd = "None"
    return (
        " " * indent
        + f"ElementConstraint {{ path: {rs(path)}, min: {r.min}, max: {max_rs(r.max)}, fixed: {fx}, required_binding: {bd} }},"
    )


def emit(sd, provenance, load=None):
    """The Rust source for the profile, and `(elements, slices, deferred)` counts."""
    c = build(sd, load)
    name = sd.get("title") or sd.get("name") or sd.get("id")
    invariants = invariants_of(sd)
    n_slices = sum(len(s[2]) for s in c.slicings)
    out = [
        f"//! @generated by tools/gen_profile.py from {provenance} — do not edit by hand.",
        f"//! Profile: {name}  <{sd.get('url')}>",
        f"//! {len(c.elements)} element constraint(s), {n_slices} slice(s), {len(invariants)} invariant(s); "
        f"{len(c.deferred)} deferred (see gen_profile.py).",
        "",
        "use crate::compiled::*;",
        "",
        "pub static PROFILE: CompiledProfile = CompiledProfile {",
        f"    url: {rs(sd.get('url'))},",
        f"    name: {rs(name)},",
        f"    resource_type: {rs(c.resource_type)},",
        "    elements: &[",
    ]
    for path in sorted(c.elements):
        out.append(el_line(path, c.elements[path], 8))
    out += ["    ],", "    slicings: &["]
    for sliced_path, closed, slices in c.slicings:
        out.append(f"        Slicing {{ path: {rs(sliced_path)}, closed: {str(closed).lower()}, slices: &[")
        for sname, sl, discs in slices:
            out.append(f"            Slice {{ name: {rs(sname)}, discriminators: &[")
            for d in discs:
                if d[0] == "type":
                    out.append(f"                Discriminator::Type {{ key: {rs(d[1])} }},")
                elif d[0] == "pattern":
                    pairs = ", ".join(f"({rs(rel)}, {rs(v)})" for rel, v in d[2])
                    out.append(f"                Discriminator::Pattern {{ path: {rs(d[1])}, pairs: &[{pairs}] }},")
                else:
                    out.append(f"                Discriminator::Value {{ path: {rs(d[1])}, value: {rs(d[2])} }},")
            prof = f"Some({rs(sl.profile)})" if sl.profile else "None"
            if sl.subs:
                out.append(f"            ], min: {sl.min}, max: {max_rs(sl.max)}, elements: &[")
                for rel in sorted(sl.subs):
                    out.append(el_line("$this." + rel, sl.subs[rel], 16))
                out.append(f"            ], profile: {prof} }},")
            else:
                out.append(f"            ], min: {sl.min}, max: {max_rs(sl.max)}, elements: &[], profile: {prof} }},")
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
    return "\n".join(out), (len(c.elements), n_slices, len(c.deferred))


def main():
    sd_path, out_path, provenance = sys.argv[1], sys.argv[2], sys.argv[3]
    sd = json.load(open(sd_path))
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    text, (ne, ns, nd) = emit(sd, provenance)
    with open(out_path, "w") as f:
        f.write(text)
    print(f"wrote {out_path}: {ne} elements + {ns} slices enforced, {nd} deferred")


if __name__ == "__main__":
    main()
