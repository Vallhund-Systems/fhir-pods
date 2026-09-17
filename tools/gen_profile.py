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
import re
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
        if k.startswith(("fixed", "pattern")) and isinstance(v, SCALAR):
            return "true" if v is True else "false" if v is False else str(v)
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
    types: tuple = ()               # a choice element's allowed types
    patterns: tuple = ()            # a pattern / fixed object: [(anchor, [(rel, value)])] every instance must carry

    @staticmethod
    def of(e):
        mx = e.get("max")
        # a choice element's allowed types (the profile's list, which the
        # snapshot narrows from the base); a type slice element carries none
        types = ()
        if e.get("path", "").endswith("[x]") and ":" not in e.get("id", "").rsplit(".", 1)[-1]:
            types = tuple(dict.fromkeys(t["code"] for t in e.get("type", []) if t.get("code")))
        return Rule(
            min=e.get("min", 0) or 0,
            max=int(mx) if mx not in (None, "*") else None,
            fixed=scalar_fixed(e),
            binding=binding_of(e),
            types=types,
        )

    def enforceable(self):
        return (
            self.min >= 1
            or self.max is not None
            or self.fixed is not None
            or self.binding is not None
            or bool(self.types)
            or bool(self.patterns)
        )

    def merge(self, other):
        """Tighten with another rule on the same path (the stricter of each)."""
        self.min = max(self.min, other.min)
        if other.max is not None:
            self.max = other.max if self.max is None else min(self.max, other.max)
        if other.fixed is not None:
            self.fixed = other.fixed
        if other.binding:
            self.binding = other.binding
        if other.types:
            self.types = other.types
        if other.patterns:
            self.patterns = tuple(dict.fromkeys(self.patterns + other.patterns))


@dataclass
class SliceSpec:
    min: int = 0
    max: int | None = None
    type: str | None = None
    profile: str | None = None
    subs: dict = field(default_factory=dict)       # relative path -> Rule
    deep: dict = field(default_factory=dict)       # relative path -> [scalar fixed in a nested slice]
    objpat: dict = field(default_factory=dict)     # relative path ("" = the slice itself) -> pattern object
    invariants: list = field(default_factory=list) # (rel path or "", key, severity, human, expr, types)
    sub_profiles: dict = field(default_factory=dict)  # relative path -> [profile urls] (a Bundle entry's resource)
    target_profiles: list = field(default_factory=list)  # a Reference slice's targetProfile urls (for `resolve()` discriminators)
    slicings: dict = field(default_factory=dict)   # relative path -> SlicingSpec (a slicing inside this slice)
    compiled_slicings: list = field(default_factory=list)  # [(rel path, closed, [(name, SliceSpec, discs)])]


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


def slice_invariants(e, rel):
    """The constraints declared on a slice element (US Core's NPI / CLIA
    checks), scoped to that slice's instances — never to the whole path."""
    out = []
    for c in e.get("constraint", []):
        key, expr = c.get("key"), c.get("expression")
        if not key or not expr or key in STRUCTURAL_INVARIANTS:
            continue
        # a constraint the base spec's own types carry (`source` names the
        # base StructureDefinition) is the unsliced element's business
        if c.get("source", "").startswith("http://hl7.org/fhir/StructureDefinition/"):
            continue
        types = [t["code"] for t in e.get("type", []) if t.get("code")]
        out.append((rel, key, c.get("severity", "error"), c.get("human", ""), expr, types))
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


# --- lowering: the fixed skeleton and the variable slots ------------------------

# Types the engine knows how to coerce a shorthand into; anything else the
# host supplies as a JSON object of the right shape.
SLOT_TYPES = {
    "boolean", "integer", "positiveInt", "unsignedInt", "decimal", "string", "code", "id", "markdown",
    "uri", "url", "canonical", "oid", "uuid", "date", "dateTime", "instant", "time", "base64Binary",
    "Reference", "Quantity", "SimpleQuantity", "Age", "Duration", "Count", "Distance", "CodeableConcept",
    "Coding", "Period", "Identifier", "HumanName", "Address", "ContactPoint", "Attachment", "Annotation",
    "Range", "Ratio", "Timing", "Dosage", "Narrative", "Money", "Signature", "SampledData", "ContactDetail",
    "Contributor", "DataRequirement", "Expression", "ParameterDefinition", "RelatedArtifact", "TriggerDefinition",
    "UsageContext", "Meta",
}
SKIP_SLOTS = {"id", "meta", "implicitRules", "language", "text", "contained", "modifierExtension"}


def labelled(eid):
    """`Observation.component:SystolicBP.code.coding:SBPCode.code` →
    `component:SystolicBP.code.coding:SBPCode.code` (resource-relative, labels kept)."""
    return eid.split(".", 1)[1] if "." in eid else ""


def concrete_choice(e):
    """`value[x]` with exactly one type → its concrete key (`valueQuantity`)."""
    last = e.get("path", "").rsplit(".", 1)[-1]
    if not last.endswith("[x]"):
        return None
    types = [t.get("code") for t in e.get("type", []) if t.get("code")]
    if len(types) != 1:
        return None
    return last[:-3] + types[0][0].upper() + types[0][1:]


def with_concrete(label_path, elements_by_id, eid):
    """Rewrite each `[x]` segment of a labelled path to its concrete key: a
    type slice (`value[x]:valueQuantity`) becomes just `valueQuantity`; an
    unsliced single-typed choice (`component:SystolicBP.value[x]` typed
    Quantity) becomes `valueQuantity` too."""
    parts = label_path.split(".")
    prefix_ids = eid.split(".")
    out = []
    for i, seg in enumerate(parts):
        stem, _, label = seg.partition(":")
        if stem.endswith("[x]"):
            base = stem[:-3]
            if label and label.startswith(base):
                seg = label                              # a type slice: the label IS the key
            else:
                e = elements_by_id.get(".".join(prefix_ids[: i + 2]))
                key = concrete_choice(e) if e else None
                if key:
                    seg = key + (":" + label if label else "")
        out.append(seg)
    return ".".join(out)


def unlabelled(lp):
    return ".".join(seg.split(":")[0] for seg in lp.split("."))


def lowering_of(sd, load=None):
    """(fixed, slices, slots) — see `compiled::Lowering`."""
    els = (sd.get("snapshot") or {}).get("element", [])
    by_id = {e.get("id", ""): e for e in els}
    by_path = {e.get("path", "") for e in els if ":" not in e.get("id", "")}
    fixed, slices, slots = [], [], []
    has_fixed_below = set()   # labelled paths that have a fixed leaf beneath them
    fixed_decl_sites = []     # the elements fixed / pattern values are declared on (for gates)

    for e in els:
        eid, path = e.get("id", ""), e.get("path", "")
        if "." not in path:
            continue
        lp = with_concrete(labelled(eid), by_id, eid)
        # a slice element itself (a type slice keeps its place in the list under
        # its concrete key, so the engine knows it is optional)
        last_seg = eid.rsplit(".", 1)[-1]
        if ":" in last_seg:
            slices.append((lp, e.get("min", 0) or 0))
            # an extension slice is identified by its URL; an inherited profile fixes leaves of its own
            types = e.get("type", [])
            prof = (types[0].get("profile") or [None])[0] if types else None
            if prof and types[0].get("code") == "Extension":
                fixed.append((lp + ".url", prof))
            elif prof and load:
                ref = load(prof)
                if ref:
                    root = ref.get("type")
                    for re_ in (ref.get("snapshot") or {}).get("element", []):
                        if ":" in re_.get("id", ""):
                            continue
                        rel = re_.get("path", "")[len(root) + 1:]
                        if not rel:
                            continue
                        fx = scalar_fixed(re_)
                        if fx is not None:
                            fixed.append((lp + "." + rel, fx))
                        obj = object_fixed(re_)
                        if obj is not None:
                            for anchor, pairs in groups_of(obj, rel):
                                for sub, v in pairs:
                                    fixed.append((lp + "." + anchor + ("." + sub if sub else ""), v))
        fx = scalar_fixed(e)
        if fx is not None:
            fixed.append((lp, fx))
            fixed_decl_sites.append(lp)
        obj = object_fixed(e)
        if obj is not None:
            fixed_decl_sites.append(lp)
            for anchor, pairs in groups_of(obj, ""):
                for sub, v in pairs:
                    fixed.append((lp + ("." + anchor if anchor else "") + ("." + sub if sub else ""), v))

    # A fixed leaf suppresses its ancestors as slots — but only when it is
    # reached without an optional slice: a required MRN is still an input
    # even though the optional `identifier:ihi` slice fixes a system.
    required_slices = {lp for lp, mn in slices if mn >= 1}
    optional_slices = {lp for lp, mn in slices if mn == 0}

    def under_optional_slice(lp):
        parts = lp.split(".")
        return any(".".join(parts[:i]) in optional_slices for i in range(1, len(parts)))

    for lp, _ in fixed:
        if under_optional_slice(lp):
            continue
        for path_form in (lp, unlabelled(lp)):
            parts = path_form.split(".")
            for i in range(1, len(parts)):
                has_fixed_below.add(".".join(parts[:i]))
    sliced = {e.get("path", "") for e in els if "slicing" in e}
    type_of = {}
    for e in els:
        types = [t.get("code") for t in e.get("type", []) if t.get("code")]
        if types and "." in e.get("path", ""):
            type_of[with_concrete(labelled(e["id"]), by_id, e["id"])] = types[0]
    QUANTITY = {"Quantity", "SimpleQuantity", "Age", "Duration", "Count", "Distance", "MoneyQuantity"}
    DATATYPE_CONTAINERS = SLOT_TYPES - {"Quantity", "SimpleQuantity", "Age", "Duration", "Count", "Distance"}
    unsliced_backbones = {
        with_concrete(labelled(e["id"]), by_id, e["id"])
        for e in els
        if [t.get("code") for t in e.get("type", [])] == ["BackboneElement"] and e.get("path") not in sliced
        and ":" not in labelled(e["id"]).rsplit(".", 1)[-1]
    }
    slot_paths = set()

    # elements the profile's own invariants reach (`value.exists() or
    # dataAbsentReason.exists() …`): optional on their own, but one of them
    # has to be there, so each is an input
    reached = set()
    for e in els:
        base = e.get("path", "")
        for c in e.get("constraint", []):
            src = (c.get("source") or "").split("|")[0]
            if src and src != sd.get("url"):
                continue
            for name in re.findall(r"(?<![\w.$%'])([a-z][A-Za-z0-9]*)(?![\w(])", c.get("expression", "")):
                child = base + "." + name
                if child in by_path or child + "[x]" in by_path:
                    reached.add(child.split(".", 1)[1] + ("[x]" if child + "[x]" in by_path else ""))

    def add_slot(lp, ty, choices, mn, mx, ms):
        if lp not in slot_paths:
            slot_paths.add(lp)
            slots.append({"path": lp, "ty": ty, "choices": choices, "min": mn, "max": mx, "ms": ms})

    for e in els:
        eid, path = e.get("id", ""), e.get("path", "")
        if "." not in path:
            continue
        lp = with_concrete(labelled(eid), by_id, eid)
        first = lp.split(".")[0].split(":")[0]
        if first in SKIP_SLOTS or first == "extension" and ":" not in lp:
            continue
        # a sliced container that asks for more instances than its required
        # slices supply (BALP's disclosure: `entity` 2..* with one required
        # `patient` slice): the rest are plain inputs, whatever its slices fix
        own_slices = [m for (sp, m) in slices if sp.startswith(lp + ":") and "." not in sp[len(lp) + 1:]] if path in sliced else []
        spare_container = bool(own_slices) and (e.get("min", 0) or 0) > sum(own_slices)
        if scalar_fixed(e) is not None or object_fixed(e) is not None:
            continue
        # a labelled path is judged by its own slice's fixed leaves; the
        # unlabelled form only suppresses a plain element (a fixed
        # `entity:patient.role` must not hide `entity:data.role`)
        if (lp in has_fixed_below or (lp == unlabelled(lp) and unlabelled(lp) in has_fixed_below)) and not spare_container:
            continue
        if e.get("max") == "0":
            continue
        types = [t.get("code") for t in e.get("type", []) if t.get("code")]
        if not types or types[0] in ("Element", "Extension"):
            continue
        parts = lp.split(".")
        ancestors = [".".join(parts[:i]) for i in range(1, len(parts))]
        # beneath an unsliced group: the group is the slot (or nothing), never its parts
        if any(a in unsliced_backbones for a in ancestors):
            continue
        # a plain child of a sliced element: reached through the slices, not here
        if any(".".join(unlabelled(a).split(".")) in {p.split(".", 1)[1] for p in sliced} and ":" not in a.rsplit(".", 1)[-1] for a in ancestors):
            continue
        # a type slice the profile forbids (value[x]:valueQuantity 0..0) is not an input
        if any(lp == with_concrete(labelled(o["id"]), by_id, o["id"]) and o.get("max") == "0" for o in els if ":" in o.get("id", "")):
            continue
        if types[0] == "BackboneElement" and ":" in lp.rsplit(".", 1)[-1]:
            continue
        if path in sliced:
            # a sliced container: reached through its slices — unless none of
            # them is required, in which case the container is the input
            # (Provenance.agent with optional author / transmitter slices)
            # A top-level container (`category`, `subtype`) stays an input even
            # so: its slices' fixed leaves may sit behind an optional gate
            # (`category:us-core.coding`), and the whole element the host gives
            # is what reaches them. A nested one (`code.coding`) is reached
            # through its slices alone.
            if "." in lp and any(m >= 1 for m in own_slices) and not spare_container:
                continue
        required = (e.get("min", 0) or 0) >= 1
        ms = bool(e.get("mustSupport"))
        # a base definition (the spec's own AuditEvent, Provenance) fixes
        # nothing: every element is the host's to give
        is_base = sd.get("derivation") != "constraint"
        if not (required or ms or is_base or unlabelled(lp) in reached):
            continue
        mx = e.get("max")
        mx = int(mx) if mx not in (None, "*") else None
        # a part of a datatype (telecom.system): the datatype is the slot
        holder = next((a for a in ancestors if type_of.get(a) in DATATYPE_CONTAINERS and a not in has_fixed_below and unlabelled(a) not in has_fixed_below), None)
        if holder:
            he = by_id.get(sd["type"] + "." + holder.replace(":", ":")) or {}
            hmx = None
            add_slot(holder, type_of[holder], (), 0, hmx, True)
            continue
        last = path.rsplit(".", 1)[-1]
        if last.endswith("[x]") and len(types) > 1:
            choices = tuple(last[:-3] + t[0].upper() + t[1:] for t in types)
            ty = "choice"
        else:
            choices = ()
            ty = types[0]
        if ty != "choice" and ty not in SLOT_TYPES and ty != "BackboneElement":
            continue
        add_slot(lp, ty, choices, e.get("min", 0) or 0, mx, ms)

    # a Quantity the profile fixes the unit of (system + code) still needs its number
    for lp, _ in list(fixed):
        parent, _, leaf = lp.rpartition(".")
        if leaf in ("system", "code") and type_of.get(parent) in QUANTITY and parent + ".value" not in slot_paths:
            if under_optional_slice(parent):
                continue
            add_slot(parent + ".value", "decimal", (), 1, 1, True)
            add_slot(parent + ".unit", "string", (), 0, 1, True)
    # gates: an optional element (min 0) on the way to a fixed leaf — the leaf
    # applies only when an input reaches into it (component:SystolicBP
    # .valueQuantity stays absent when the host gives a dataAbsentReason)
    min_of = {}
    for e in els:
        if "." in e.get("path", ""):
            min_of[with_concrete(labelled(e["id"]), by_id, e["id"])] = e.get("min", 0) or 0
    gate_paths = {p for p, _ in slices}
    for decl in fixed_decl_sites:
        parts = decl.split(".")
        for i in range(1, len(parts) + 1):
            pre = ".".join(parts[:i])
            if pre not in gate_paths and min_of.get(pre) == 0:
                slices.append((pre, 0))
                gate_paths.add(pre)
    # dedupe fixed (an inherited leaf may repeat)
    seen, uniq = set(), []
    for f in fixed:
        if f not in seen:
            seen.add(f)
            uniq.append(f)
    return uniq, slices, slots


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
            if ":" in eid and eid.count(":") == 1 and "." in eid.split(":", 1)[1]:
                # a slicing declared inside a slice (`section:sectionProblems.entry`)
                head, tail = eid.split(":", 1)
                outer_name, rel = tail.split(".", 1)
                nsg = slicings.setdefault(head, SlicingSpec()).slices.setdefault(outer_name, SliceSpec()).slicings.setdefault(rel, SlicingSpec())
                nsg.closed = e["slicing"].get("rules") == "closed"
                nsg.disc = [(d.get("type"), d.get("path")) for d in e["slicing"].get("discriminator", [])]
            elif ":" not in eid:
                sg = slicings.setdefault(path, SlicingSpec())
                sg.closed = e["slicing"].get("rules") == "closed"
                sg.disc = [(d.get("type"), d.get("path")) for d in e["slicing"].get("discriminator", [])]
        if ":" not in eid:
            # a plain element: a pattern / fixed object is what every instance must carry
            rule = Rule.of(e)
            obj = object_fixed(e)
            if obj is not None:
                rule.patterns = tuple((a, tuple(p)) for a, p in groups_of(obj))
            if rule.enforceable():
                elements.setdefault(path, Rule()).merge(rule)
            elif e.get("mustSupport") or (e.get("binding") or {}).get("strength") in ("required", "extensible"):
                deferred.append(("mustSupport / non-required binding only", eid))
            continue

        head, tail = eid.split(":", 1)
        sg = slicings.setdefault(head, SlicingSpec())
        if ":" in tail:
            # A slice inside a slice (`section:sectionProblems.entry:problem`):
            # compiled one level down. Deeper nesting is deferred. Either way a
            # nested slice's scalar fixed values are what an OUTER slice's
            # `value` discriminator resolves to (BP: component:SystolicBP is
            # discriminated by code.coding.code, whose value lives in
            # component:SystolicBP.code.coding:SBPCode.code).
            outer_name, rest = tail.split(".", 1)
            sl = sg.slices.setdefault(outer_name, SliceSpec())
            fixed = scalar_fixed(e)
            if fixed is not None and path.startswith(head + "."):
                sl.deep.setdefault(path[len(head) + 1:], []).append(fixed)
            nested_path, nested_tail = rest.split(":", 1)
            if ":" in nested_tail:
                deferred.append(("nested slicing (three levels)", eid))
                continue
            nsg = sl.slicings.setdefault(nested_path, SlicingSpec())
            types = e.get("type", [])
            if "." not in nested_tail:
                nsl = nsg.slices.setdefault(nested_tail, SliceSpec())
                rule = Rule.of(e)
                nsl.min, nsl.max = rule.min, rule.max
                nsl.type = types[0].get("code") if types else None
                nsl.profile = (types[0].get("profile") or [None])[0] if types else None
                nsl.target_profiles = [p.split("|")[0] for t in types for p in t.get("targetProfile", [])]
                nsl.invariants += slice_invariants(e, "")
            else:
                nname, nrel = nested_tail.split(".", 1)
                nsl = nsg.slices.setdefault(nname, SliceSpec())
                rule = Rule.of(e)
                if rule.enforceable():
                    nsl.subs.setdefault(nrel, Rule()).merge(rule)
                nsl.invariants += slice_invariants(e, nrel)
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
                sl.subs.setdefault("", Rule()).patterns = tuple((a, tuple(p)) for a, p in groups_of(obj))
            sl.invariants += slice_invariants(e, "")
            continue
        # a slice sub-element
        name, _ = tail.split(".", 1)
        sl = sg.slices.setdefault(name, SliceSpec())
        if not path.startswith(head + "."):
            deferred.append(("sub-element outside its slice", eid))
            continue
        rel = path[len(head) + 1:]
        sl.invariants += slice_invariants(e, rel)
        profs = [p.split("|")[0] for t in e.get("type", []) for p in t.get("profile", [])]
        if profs:
            sl.sub_profiles[rel] = profs
        rule = Rule.of(e)
        obj = object_fixed(e)
        if obj is not None:
            sl.objpat[rel] = obj         # discriminable by, and enforced on the slice's instances
            rule.patterns = tuple((a, tuple(p)) for a, p in groups_of(obj))
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
        if dpath == "resolve()" and load:
            # a Reference slice discriminated by what it points at: the
            # target's type, or its conformance to a profile (base types
            # are not compiled, so they discriminate by type alone)
            sds = [(u, load(u)) for u in sl.target_profiles]
            if not sds or any(s is None for _, s in sds):
                return None
            if dtype == "type":
                discs.append(("resolved-type", sorted({s["type"] for _, s in sds})))
                continue
            if dtype == "profile":
                profs = [u for u, s in sds if s.get("derivation") == "constraint"]
                if profs:
                    discs.append(("resolved-profile", profs))
                elif not any(d[0] == "resolved-type" for d in discs):
                    discs.append(("resolved-type", sorted({s["type"] for _, s in sds})))
                continue
            return None
        if dtype == "type" and dpath == "$this":
            if not sl.type:
                return None
            discs.append(("type", choice_key(sliced_path, sl.type)))
            continue
        if dtype in ("type", "profile") and dpath in sl.sub_profiles and load:
            # a Bundle entry's resource: its type, or conformance to its profile
            sds = [load(u) for u in sl.sub_profiles[dpath]]
            if dtype == "type":
                types = {s.get("type") for s in sds if s}
                if len(types) == 1 and all(sds):
                    discs.append(("rtype", "$this." + dpath, types.pop()))
                    continue
            elif len(sds) == 1 and sds[0]:
                discs.append(("profile", "$this." + dpath, sl.sub_profiles[dpath][0]))
                continue
            return None
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

    def compile_slicing(sliced_path, sg, label):
        emitted = []
        for name, sl in sg.slices.items():
            discs = resolve_discriminators(sliced_path, sg, sl, load)
            if discs is None:
                deferred.append(("undiscriminable slice", f"{label}:{name}"))
                continue
            for rel, nsg in sl.slicings.items():
                nested = compile_slicing(sliced_path + "." + rel, nsg, f"{label}:{name}.{rel}")
                if nested:
                    sl.compiled_slicings.append((rel, nsg.closed, nested))
            emitted.append((name, sl, discs))
        return emitted

    out_slicings = []
    for sliced_path, sg in slicings.items():
        emitted = compile_slicing(sliced_path, sg, sliced_path)
        if emitted:
            out_slicings.append((sliced_path, sg.closed, emitted))
    return Compiled(sd["type"], elements, out_slicings, deferred)


# --- emit ---------------------------------------------------------------------

def rs(s):
    return '"' + str(s).replace("\\", "\\\\").replace('"', '\\"') + '"'


def max_rs(mx):
    return f"Max::Bounded({mx})" if mx is not None else "Max::Star"


# Lists of strings shared across the generated profiles — a choice element's
# types, a slot's choice keys, an invariant's types — are interned into
# `generated/intern.rs`, one static per distinct list, and referenced by
# name: the same `value[x]` list is not repeated per profile.
INTERN: dict[tuple, str] = {}


def strs(seq):
    """Rust source for a `&[&str]`: inline when short, interned when shared."""
    seq = tuple(seq)
    if len(seq) < 2:
        return "&[" + ", ".join(rs(s) for s in seq) + "]"
    name = INTERN.setdefault(seq, f"S{len(INTERN)}")
    return f"super::intern::{name}"


def emit_intern():
    """The `intern.rs` source for every list registered so far."""
    out = ["//! @generated by tools/gen_all.py — do not edit by hand.", "//! String lists shared across the compiled profiles.", ""]
    for seq, name in sorted(INTERN.items(), key=lambda kv: int(kv[1][1:])):
        out.append(f"pub static {name}: &[&str] = &[" + ", ".join(rs(s) for s in seq) + "];")
    return "\n".join(out) + "\n"


def el_line(path, r, indent):
    fx = f"Some({rs(r.fixed)})" if r.fixed is not None else "None"
    if r.binding:
        vs, kind = r.binding
        bd = f"Some(Binding {{ value_set: {rs(vs)}, kind: BindingKind::{kind[0].upper() + kind[1:]} }})"
    else:
        bd = "None"
    ty = strs(r.types)
    pats = ", ".join(f"({rs(a)}, &[" + ", ".join(f"({rs(rel)}, {rs(v)})" for rel, v in pairs) + "])" for a, pairs in r.patterns)
    path = path[:-1] if path.endswith("$this.") else path
    return (
        " " * indent
        + f"ElementConstraint {{ path: {rs(path)}, min: {r.min}, max: {max_rs(r.max)}, fixed: {fx}, required_binding: {bd}, "
        f"types: {ty}, patterns: &[{pats}] }},"
    )


def emit_slicing(path, closed, slices, indent):
    """The lines of one `Slicing { … }` literal — its slices' nested slicings recursively."""
    pad = " " * indent
    out = [f"{pad}Slicing {{ path: {rs(path)}, closed: {str(closed).lower()}, slices: &["]
    for sname, sl, discs in slices:
        out.append(f"{pad}    Slice {{ name: {rs(sname)}, discriminators: &[")
        for d in discs:
            if d[0] == "type":
                out.append(f"{pad}        Discriminator::Type {{ key: {rs(d[1])} }},")
            elif d[0] == "pattern":
                pairs = ", ".join(f"({rs(rel)}, {rs(v)})" for rel, v in d[2])
                out.append(f"{pad}        Discriminator::Pattern {{ path: {rs(d[1])}, pairs: &[{pairs}] }},")
            elif d[0] == "rtype":
                out.append(f"{pad}        Discriminator::ResourceType {{ path: {rs(d[1])}, ty: {rs(d[2])} }},")
            elif d[0] == "profile":
                out.append(f"{pad}        Discriminator::Profile {{ path: {rs(d[1])}, url: {rs(d[2])} }},")
            elif d[0] == "resolved-type":
                out.append(f"{pad}        Discriminator::ResolvedType {{ types: {strs(d[1])} }},")
            elif d[0] == "resolved-profile":
                out.append(f"{pad}        Discriminator::ResolvedProfile {{ urls: {strs(d[1])} }},")
            else:
                out.append(f"{pad}        Discriminator::Value {{ path: {rs(d[1])}, value: {rs(d[2])} }},")
        prof = f"Some({rs(sl.profile)})" if sl.profile else "None"
        out.append(f"{pad}    ], min: {sl.min}, max: {max_rs(sl.max)}, elements: &[")
        for rel in sorted(sl.subs):
            out.append(el_line("$this." + rel, sl.subs[rel], indent + 8))
        out.append(f"{pad}    ], invariants: &[")
        for rel, key, severity, human, expr, types in sl.invariants:
            sev = "InvariantSeverity::Error" if severity == "error" else "InvariantSeverity::Warning"
            ipath = "$this" if not rel else "$this." + rel
            out.append(
                f"{pad}        ProfileInvariant {{ path: {rs(ipath)}, key: {rs(key)}, severity: {sev}, "
                f"human: {rs(human)}, expression: {rs(expr)}, types: {strs(types)} }},"
            )
        out.append(f"{pad}    ], profiles: &[")
        for rel in sorted(sl.sub_profiles):
            out.append(f"{pad}        ({rs('$this.' + rel)}, {strs(sl.sub_profiles[rel])}),")
        out.append(f"{pad}    ], slicings: &[")
        for rel, nclosed, nslices in sl.compiled_slicings:
            out += emit_slicing("$this." + rel, nclosed, nslices, indent + 8)
        out.append(f"{pad}    ], profile: {prof} }},")
    out.append(f"{pad}] }},")
    return out


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
        f"    base: {'Some(' + rs(sd['baseDefinition'].split('|')[0]) + ')' if sd.get('baseDefinition') else 'None'},",
        f"    name: {rs(name)},",
        f"    resource_type: {rs(c.resource_type)},",
        "    elements: &[",
    ]
    for path in sorted(c.elements):
        out.append(el_line(path, c.elements[path], 8))
    out += ["    ],", "    slicings: &["]
    for sliced_path, closed, slices in c.slicings:
        out += emit_slicing(sliced_path, closed, slices, 8)
    out += ["    ],", "    invariants: &["]
    for path, key, severity, human, expr, types in invariants:
        sev = "InvariantSeverity::Error" if severity == "error" else "InvariantSeverity::Warning"
        out.append(
            f"        ProfileInvariant {{ path: {rs(path)}, key: {rs(key)}, severity: {sev}, "
            f"human: {rs(human)}, expression: {rs(expr)}, types: {strs(types)} }},"
        )
    out += ["    ],", "    lowering: &LOWERING,", "    must_support: &["]
    for path in must_support_of(sd):
        out.append(f"        {rs(path)},")
    out += ["    ],", "    contexts: &["]
    contexts = [(cx["type"], cx["expression"]) for cx in sd.get("context") or [] if cx.get("type") and cx.get("expression")]
    # `regex` is defined on ElementDefinition and used, by the spec itself, on
    # ElementDefinition.type; the reference validator accepts either place
    # (FHIR-13328), and so do we
    if sd.get("url") == "http://hl7.org/fhir/StructureDefinition/regex":
        for extra in ("ElementDefinition", "ElementDefinition.type"):
            if ("element", extra) not in contexts:
                contexts.append(("element", extra))
    for kind, expr in contexts:
        out.append(f"        ({rs(kind)}, {rs(expr)}),")
    out += ["    ],", "};", ""]
    fixed, slices, slots = lowering_of(sd, load)
    out += ["/// How to build a resource for this profile (see `compiled::Lowering`).", "pub static LOWERING: Lowering = Lowering {", "    fixed: &["]
    out += [f"        ({rs(p)}, {rs(v)})," for p, v in fixed]
    out += ["    ],", "    slices: &["]
    out += [f"        ({rs(p)}, {m})," for p, m in slices]
    out += ["    ],", "    slots: &["]
    for sl in slots:
        out.append(
            f"        Slot {{ path: {rs(sl['path'])}, ty: {rs(sl['ty'])}, choices: {strs(sl['choices'])}, min: {sl['min']}, "
            f"max: {max_rs(sl['max'])}, must_support: {str(sl['ms']).lower()} }},"
        )
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
