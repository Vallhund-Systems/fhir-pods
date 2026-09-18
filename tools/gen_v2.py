#!/usr/bin/env python3
"""Compile HL7 v2-to-FHIR mappings (`hl7.fhir.uv.v2mappings`) into `'static`
tables for the v2 pod: datatype maps (`XPN` → `HumanName`), segment maps
(`PID` → `Patient`), message maps (`ORU_R01` → the resources of a message
Bundle) and vocabulary tables (`HL70001` → `administrative-gender`).

A row the pod can execute is compiled; a row it cannot — a narrative-only
condition, a prose assignment, a target path the engine does not take, a
reference to a resource no datatype map builds — is compiled as *skipped*
with its reason, so the converter can report what a message carried that
was not mapped. Nothing is approximated.

Usage:
    gen_v2.py <package dir> <out.rs>
"""
import glob
import json
import os
import re
import sys


def rs(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def typeinfo(exts):
    out = {}
    for x in exts or []:
        if x.get("url", "").endswith("/TypeInfo"):
            for y in x.get("extension", []):
                v = next((v for k, v in y.items() if k.startswith("value")), None)
                out[y["url"]] = v
    return out


# --- conditions ---------------------------------------------------------------

REF = re.compile(r"^([A-Z][A-Z0-9]{1,3})(?:-(\d+)(?:\.(\d+)(?:\.(\d+))?)?|\.(\d+)(?:\.(\d+))?)?$")


# The segments the package maps (filled in main): a condition that spells
# `ORC.1` for a segment's field (the maps' own slip; a datatype is never named
# like a segment) is read as `ORC-1`. One of the generator's stated corrections.
SEGMENT_NAMES = set()


def parse_ref(tok):
    """`PID-7`, `OBX-5.1`, `XPN.12`, `CX.4.1`, `PRT-4` → (segment|datatype, field, component, sub)."""
    m = REF.match(tok)
    if not m:
        return None
    seg, f, c, s, dc, ds = m.groups()
    if f is not None:
        return (seg, int(f), int(c or 0), int(s or 0))
    if dc is not None:
        if seg in SEGMENT_NAMES:
            return (seg, int(dc), int(ds or 0), 0)
        return (seg, 0, int(dc), int(ds or 0))
    return (seg, 0, 0, 0)


class CondParser:
    """The computable subset of the maps' `Computable-ANTLR` conditions:
    `IF X VALUED`, `X NOT VALUED`, `X EQUALS "v"`, `X NOT EQUALS "v"`,
    `X IS "v"`, `X IS NOT "v"`, `X IN ("a", "b")`, `X NOT IN (...)`,
    `X LST.count GREATER THAN n` / `LESS THAN OR EQUALS n`, `X LENGTH
    GREATER THAN n`, `X COUNT>n`, joined by AND / OR, with parentheses.
    Anything else raises, and the row is skipped with the text."""

    def __init__(self, text):
        t = text.strip()
        t = re.sub(r"\s+", " ", t)
        # tokens: strings, parens, words/refs/numbers
        self.toks = re.findall(r'"[^"]*"|\'[^\']*\'|\(|\)|,|[^\s(),]+', t)
        self.i = 0

    def peek(self):
        return self.toks[self.i] if self.i < len(self.toks) else None

    def eat(self, *words):
        for w in words:
            if self.i < len(self.toks) and self.toks[self.i].upper() == w:
                self.i += 1
                return True
        return False

    def expect(self, *words):
        if not self.eat(*words):
            raise ValueError(f"expected {words} at {self.peek()!r}")

    def parse(self):
        self.eat("IF")
        e = self.or_()
        if self.peek() is not None:
            raise ValueError(f"trailing {self.peek()!r}")
        return e

    def or_(self):
        left = self.and_()
        while self.eat("OR"):
            self.eat("IF")
            left = ("or", left, self.and_())
        return left

    def and_(self):
        left = self.atom()
        while self.eat("AND"):
            self.eat("IF")
            left = ("and", left, self.atom())
        return left

    def atom(self):
        if self.eat("("):
            self.eat("IF")
            e = self.or_()
            self.expect(")")
            return e
        if self.eat("NOT"):
            return ("not", self.atom())
        tok = self.peek()
        ref = parse_ref(tok) if tok else None
        if ref is None:
            raise ValueError(f"not a reference: {tok!r}")
        self.i += 1
        # `X AND Y VALUED`, `X AND Y AND Z NOT VALUED`, `X OR Y VALUED`: one
        # predicate shared by several references, joined one way
        refs = [ref]
        joiner = None
        j = self.i
        toks = self.toks
        while j + 1 < len(toks) and toks[j].upper() in ("AND", "OR") and (joiner is None or toks[j].upper() == joiner):
            r2 = parse_ref(toks[j + 1])
            if r2 is None:
                break
            refs.append(r2)
            joiner = toks[j].upper()
            j += 2
        nxt = toks[j].upper() if j < len(toks) else None
        if len(refs) > 1 and nxt in ("VALUED", "VALUE", "NOT", "IS", "ARE", "AREA", "IN", "EQUALS", "EQUAL", "EXISTS", "DOES"):
            self.i = j
        else:
            refs = [ref]
            joiner = None
        pred = self.predicate()
        out = None
        for r in refs:
            p = (pred[0], r) + tuple(pred[1:])
            out = p if out is None else (joiner.lower() if joiner else "and", out, p)
        return out

    def predicate(self):
        if self.eat("LST.COUNT", "LST.count", "COUNT"):
            if self.eat("GREATER"):
                self.expect("THAN")
                return ("count_gt", int(self.num()))
            if self.eat("LESS"):
                self.expect("THAN")
                if self.eat("OR"):
                    self.expect("EQUALS", "EQUAL")
                    return ("count_le", int(self.num()))
                return ("count_lt", int(self.num()))
            if self.eat("EQUALS", "="):
                return ("count_eq", int(self.num()))
            raise ValueError("count predicate")
        tok = self.peek()
        if tok and re.match(r"^COUNT[>=<]\d+$", tok.upper()):
            self.i += 1
            op, n = tok[5], int(tok[6:])
            return {">": ("count_gt", n), "=": ("count_eq", n), "<": ("count_lt", n)}[op]
        if self.eat("LENGTH"):
            self.expect("GREATER")
            self.expect("THAN")
            return ("length_gt", int(self.num()))
        if self.eat("VALUED", "VALUE", "EXISTS"):
            return ("valued",)
        if self.eat("IS", "ARE", "AREA"):
            if self.eat("NOT"):
                if self.eat("VALUED", "EMPTY"):
                    return ("not_valued",) if self.toks[self.i - 1].upper() == "VALUED" else ("valued",)
                if self.eat("IN"):
                    return ("not_in", self.list_())
                return ("not_equals", self.string())
            if self.eat("VALUED"):
                return ("valued",)
            if self.eat("EMPTY"):
                return ("not_valued",)
            if self.eat("IN"):
                return ("in", self.list_())
            return ("equals", self.string())
        if self.eat("NOT"):
            if self.eat("VALUED", "EXIST", "EXISTS"):
                return ("not_valued",)
            if self.eat("EQUALS", "EQUAL"):
                return ("not_equals", self.string())
            if self.eat("IN"):
                return ("not_in", self.list_())
            if self.peek() and (self.peek().startswith('"') or self.peek().startswith("'")):
                return ("not_equals", self.string())
            raise ValueError("NOT predicate")
        if self.eat("DOES"):
            self.expect("NOT")
            self.expect("EXIST", "EXISTS")
            return ("not_valued",)
        if self.eat("EQUALS", "EQUAL", "=", "=="):
            return ("equals", self.string())
        if self.eat("IN"):
            return ("in", self.list_())
        raise ValueError(f"predicate at {tok!r}")

    def num(self):
        tok = self.peek()
        if tok is None or not re.match(r"^\d+$", tok):
            raise ValueError(f"number at {tok!r}")
        self.i += 1
        return tok

    def string(self):
        tok = self.peek()
        if tok is None or not ((tok.startswith('"') and tok.endswith('"')) or (tok.startswith("'") and tok.endswith("'"))):
            raise ValueError(f"string at {tok!r}")
        self.i += 1
        return tok[1:-1]

    def list_(self):
        self.expect("(")
        out = []
        while True:
            tok = self.peek()
            if tok is None:
                raise ValueError("list")
            if tok == ",":
                self.i += 1
                continue
            if tok == ")":
                self.i += 1
                break
            if tok == ",":
                self.i += 1
                continue
            if tok.startswith('"'):
                out.append(tok.strip('",'))
                self.i += 1
                continue
            raise ValueError(f"list item {tok!r}")
        return out


def cond_rs(e):
    kind = e[0]
    if kind in ("and", "or"):
        return f"CondExpr::{kind.capitalize()}(&{cond_rs(e[1])}, &{cond_rs(e[2])})"
    if kind == "not":
        return f"CondExpr::Not(&{cond_rs(e[1])})"
    ref = e[1]
    r = f"Ref {{ name: {rs(ref[0])}, field: {ref[1]}, component: {ref[2]}, sub: {ref[3]} }}"
    if kind == "valued":
        return f"CondExpr::Valued({r})"
    if kind == "not_valued":
        return f"CondExpr::NotValued({r})"
    if kind == "equals":
        if ref[:2] == ("OBX", 2) and e[2] == "DTM":
            return f"CondExpr::In({r}, &[{rs('DTM')}, {rs('TS')}])"
        return f"CondExpr::Equals({r}, {rs(e[2])})"
    if kind == "not_equals":
        return f"CondExpr::NotEquals({r}, {rs(e[2])})"
    if kind in ("in", "not_in"):
        items = list(e[2])
        if ref[:2] == ("OBX", 2) and "DTM" in items and "TS" not in items:
            items.append("TS")  # TS is the pre-2.6 name of DTM
        return f"CondExpr::{'In' if kind == 'in' else 'NotIn'}({r}, &[{', '.join(rs(x) for x in items)}])"
    if kind == "count_gt":
        return f"CondExpr::CountGt({r}, {e[2]})"
    if kind == "count_le":
        return f"CondExpr::CountLe({r}, {e[2]})"
    if kind == "count_lt":
        return f"CondExpr::CountLe({r}, {e[2] - 1})"
    if kind == "count_eq":
        return f"CondExpr::CountEq({r}, {e[2]})"
    if kind == "length_gt":
        return f"CondExpr::LengthGt({r}, {e[2]})"
    raise ValueError(kind)


# Narratives that instruct *how* to map rather than *whether*: the row
# applies always, the instruction being what the engine does anyway (a
# code is carried through its vocabulary map when one is named, else as
# given).
INSTRUCTION_NARRATIVES = {
    "If mapped to FHIR, use Code System and Display",
}


def compile_cond(target):
    """The row's condition: (rust expr, None) or (None, reason) when it is
    narrative only or outside the grammar."""
    deps = target.get("dependsOn", [])
    antlr = [d.get("value") or d.get("display") or "" for d in deps if d.get("property") == "Computable-ANTLR"]
    narrative = [d.get("value") or d.get("display") or "" for d in deps if d.get("property") == "Narrative-Condition"]
    narrative = [n for n in narrative if n.strip() not in INSTRUCTION_NARRATIVES]
    if not antlr and not narrative:
        return "Cond::Always", None
    # a narrative beside a computable condition that reads as a condition
    # ("If …", "When …", "Only …", "Unless …") is a further gate the pod
    # cannot judge: the row is skipped and the narrative is the reason; a
    # remark ("Any markup in ST … may not present properly") is not a gate
    gates = [n for n in narrative if re.match(r"^\s*(if|when|only|unless|where)\b", n, re.I)]
    if not antlr or gates:
        return None, "narrative condition: " + (gates or narrative)[0][:80]
    try:
        e = CondParser(antlr[0]).parse()
        return f"Cond::When(&{cond_rs(e)})", None
    except (ValueError, IndexError) as ex:
        return None, f"condition outside the grammar: {antlr[0][:80]}"


# --- targets ------------------------------------------------------------------

# The choice types a `value[x]` element takes, for the maps' own spelling of
# them (`valuedateTime`, `valuecodeableconcept`) — matched without case.
CHOICE_TYPES = [
    "base64Binary", "boolean", "canonical", "code", "date", "dateTime", "decimal", "id", "instant", "integer",
    "markdown", "oid", "positiveInt", "string", "time", "unsignedInt", "uri", "url", "uuid", "Address", "Age",
    "Annotation", "Attachment", "CodeableConcept", "Coding", "ContactPoint", "Count", "Distance", "Duration",
    "HumanName", "Identifier", "Money", "Period", "Quantity", "Range", "Ratio", "Reference", "SampledData",
    "Signature", "Timing", "ContactDetail", "Contributor", "DataRequirement", "Expression",
    "ParameterDefinition", "RelatedArtifact", "TriggerDefinition", "UsageContext", "Dosage", "Meta",
]
_CHOICE_BY_LOWER = {t.lower(): t for t in CHOICE_TYPES}


def spell_choices(path):
    """`valuedateTime` → `valueDateTime`: a choice key spelt in the maps'
    case becomes the FHIR spelling, segment by segment."""
    out = []
    for seg in path.split("."):
        m = re.match(r"^(value|effective|onset|abatement|occurrence|performed|deceased|multipleBirth|serviced|timing|born|product|medication|item|manufacturer|subject|content|target|source|amount|used|start|end|scheduled|when|entered|valueRange)([A-Za-z0-9]+)(\[[^\]]*\])?$", seg)
        if m and m.group(2).lower() in _CHOICE_BY_LOWER:
            t = _CHOICE_BY_LOWER[m.group(2).lower()]
            seg = m.group(1) + t[0].upper() + t[1:] + (m.group(3) or "")
        out.append(seg)
    return ".".join(out)


def ref_rs(ref):
    return f"Ref {{ name: {rs(ref[0])}, field: {ref[1]}, component: {ref[2]}, sub: {ref[3]} }}"


def parse_concat(assign):
    """A concatenation assignment (`"+"+XTN.5+" "+XTN.6`, `"urn:oid:"+HD.2`)
    as Rust `Part`s, or None when it is anything else (prose, arithmetic,
    a lone literal)."""
    pieces = []
    for m in re.finditer(r'"([^"]*)"|([A-Z][A-Z0-9]*(?:[.-]\d+)+)|(\s*\+\s*)|(.)', assign.strip()):
        lit, ref, plus, other = m.groups()
        if other is not None:
            return None
        if plus is not None:
            continue
        if lit is not None:
            pieces.append(f"Part::Lit({rs(lit)})")
        else:
            r = parse_ref(ref)
            if r is None:
                return None
            pieces.append(f"Part::Ref({ref_rs(r)})")
    if len(pieces) < 2 or not any(p.startswith("Part::Ref") for p in pieces):
        return None
    return "&[" + ", ".join(pieces) + "]"


def load_extension_contexts(ext_pkg):
    """url → the elements an extension may sit on, from the extensions
    package (`ContactPoint.value`), for rows the maps place a level too high."""
    contexts = {}
    if not ext_pkg or not os.path.isdir(ext_pkg):
        return contexts
    for name in os.listdir(ext_pkg):
        if not name.startswith("StructureDefinition-") or not name.endswith(".json"):
            continue
        try:
            d = json.load(open(os.path.join(ext_pkg, name)))
        except ValueError:
            continue
        if d.get("type") != "Extension" or not d.get("url"):
            continue
        contexts[d["url"]] = [c.get("expression", "") for c in d.get("context", []) if c.get("type") == "element"]
        for e in (d.get("snapshot") or d.get("differential") or {}).get("element", []):
            if e.get("path") == "Extension.value[x]" and e.get("type"):
                ext_value_types[d["url"]] = [t.get("code", "") for t in e["type"]]
    return contexts


# The FHIR types' elements, from the R4 package: element path → {child name:
# [type codes]} (a choice's name is its stem, `value`), backbone paths keyed
# by their own path, content references resolved.
fhir_children = {}
fhir_choices = set()
fhir_content_refs = {}
FHIR_TYPES = set()


def load_fhir_types(r4_pkg):
    for name in os.listdir(r4_pkg):
        if not name.startswith("StructureDefinition-") or not name.endswith(".json"):
            continue
        try:
            d = json.load(open(os.path.join(r4_pkg, name)))
        except ValueError:
            continue
        if d.get("derivation") == "constraint" or d.get("kind") not in ("resource", "complex-type", "primitive-type"):
            continue
        FHIR_TYPES.add(d["type"])
        for e in (d.get("snapshot") or {}).get("element", []):
            path = e["path"]
            if "." not in path:
                continue
            parent, child = path.rsplit(".", 1)
            if child.endswith("[x]"):
                child = child[:-3]
                fhir_choices.add(f"{parent}.{child}")
            types = [t.get("code", "") for t in e.get("type", [])]
            types = [t.rsplit("/", 1)[-1] if t.startswith("http://hl7.org/fhirpath/System.") else t for t in types]
            fhir_children.setdefault(parent, {})[child] = types
            if e.get("contentReference"):
                fhir_content_refs[path] = e["contentReference"].lstrip("#")


def resolve_element(owner, path):
    """Walk a target path through the FHIR types. Returns (missing, type,
    definition path): `missing` names the first step that is no element of
    the type it lands on (`Extension.uri`, `Coding.value`,
    `Observation.valueSampedData`) or is None; `type` is what the path ends
    on (a datatype, a primitive, or a backbone's own path) and the
    definition path is where it sits (`Patient.identifier`, `Address.line`).
    `owner` is the type the path starts in; `[k]` positions and `_name`
    carriers are read through; a type the tables do not describe is not
    judged."""
    cur, dpath = owner, owner
    for seg in path.split("."):
        name = re.sub(r"\[[^\]]*\]$", "", seg).lstrip("_")
        if name in ("$this", "$value"):
            return None, cur, dpath
        kids = fhir_children.get(cur)
        if kids is None:
            return None, None, None
        if name in kids:
            if f"{cur}.{name}" in fhir_choices:
                return f"{cur}.{name}[x] spelt without its type", None, None
            types = kids[name]
            here = f"{cur}.{name}"
        else:
            # a choice spelt with its type: `valueString` → `value` + `string`
            hit = None
            for stem in kids:
                if f"{cur}.{stem}" in fhir_choices and name.startswith(stem) and len(name) > len(stem):
                    tail = name[len(stem):]
                    t = tail[0].lower() + tail[1:] if tail[0].lower() + tail[1:] in FHIR_TYPES else tail
                    if t in FHIR_TYPES and (t in kids[stem] or tail in kids[stem]):
                        hit = t
                        break
            if hit is None:
                return f"{cur}.{name}", None, None
            types, here = [hit], f"{cur}.{name}"
        dpath = f"{dpath}.{name}"
        if here in fhir_content_refs:
            cur = fhir_content_refs[here]
        elif here in fhir_children:
            cur = here  # a backbone: its children are keyed by its path
        elif types:
            cur = types[0]
        else:
            return None, None, None
    return None, cur, dpath


def missing_element(owner, path):
    return resolve_element(owner, path)[0]


def parse_target(code):
    """A target path: `name[1].given[2]`, `assigner(Organization)`,
    `identifier[n]`, `$value`, `extension[1].url`. Returns (path with
    positions kept as `[k]`, referenced resource type or None) — or None
    when the form is one the engine does not take (`x(Type.a.b)`,
    `[1..n]`, `[1-3]`)."""
    code = spell_choices(code.strip())
    if not code:
        return None
    if code in ("$this", "$value"):
        return (code, None)
    ref = None
    m = re.match(r"^([A-Za-z_\[\]\d.$]+)\(([A-Za-z]+)\)$", code)
    if m:
        code, ref = m.group(1), m.group(2)
    if "(" in code or ")" in code:
        return None
    if re.search(r"\[(n|each|\d+\.\.n|\d+-\d+)\]", code):
        code = re.sub(r"\[(n|each|\d+\.\.n|\d+-\d+)\]", "[n]", code)
    # a datatype target may start with the instance it fills: `[1].family`
    # is the one instance; a second and further are not built
    m2 = re.match(r"^\[(\d+)\]\.(.*)$", code)
    if m2:
        if m2.group(1) != "1":
            return None
        code = m2.group(2)
    if not re.match(r"^[A-Za-z_$][A-Za-z_\d]*(\[(\d+|n)\])?(\.[A-Za-z_][A-Za-z_\d]*(\[(\d+|n)\])?)*$", code):
        return None
    return (code, ref)


PRIMITIVES = {
    "string", "code", "uri", "url", "canonical", "id", "markdown", "boolean", "integer", "positiveInt",
    "unsignedInt", "decimal", "date", "dateTime", "instant", "time", "base64Binary", "oid", "uuid",
}


# The FHIR types the maps spell in lower case, for extension contexts.
FHIR_TYPE_NAMES = {t.lower(): t for t in [
    "Address", "Age", "Annotation", "Attachment", "CodeableConcept", "Coding", "ContactPoint", "Count", "Distance",
    "Duration", "HumanName", "Identifier", "Money", "Period", "Quantity", "Range", "Ratio", "Reference", "SampledData",
    "Signature", "Timing", "Dosage", "Meta", "Narrative", "Extension",
    "Account", "AllergyIntolerance", "Appointment", "AuditEvent", "Basic", "Bundle", "CareTeam", "Condition",
    "Consent", "Coverage", "Device", "DiagnosticReport", "DocumentReference", "Encounter", "EpisodeOfCare",
    "Immunization", "Location", "Medication", "MedicationAdministration", "MedicationRequest", "MessageHeader",
    "Observation", "Organization", "Patient", "Practitioner", "PractitionerRole", "Procedure", "Provenance",
    "RelatedPerson", "ServiceRequest", "Specimen", "Task", "SupplyRequest", "Schedule", "Slot",
]}

DATATYPE_NAMES = {
    "Address", "Age", "Annotation", "Attachment", "CodeableConcept", "Coding", "ContactPoint", "Count", "Distance",
    "Duration", "HumanName", "Identifier", "Money", "Period", "Quantity", "Range", "Ratio", "Reference", "SampledData",
    "Signature", "Timing", "Dosage", "Meta", "Narrative", "Extension",
}

ext_contexts = {}
ext_value_types = {}  # url → the types its value[x] may take

# The standard's abstract message syntax for the groups the maps name: how
# often each group may occur (HL7 v2.8 chapters 3, 4, 7, 9, 10). The maps
# carry a segment's cardinality but not its group's, and a walk that cannot
# tell a repeating group from an optional one files a second patient's
# results under the first. The IG's own wrappers (COMMON_ORDER, NEXT_OF_KIN,
# ORDER_DETAIL as a sibling) take the cardinality of what they wrap.
GROUP_CARDINALITY = {
    "ADT_A01.NEXT_OF_KIN": (0, -1), "ADT_A01.OBSERVATION": (0, -1), "ADT_A01.PROCEDURE": (0, -1), "ADT_A01.INSURANCE": (0, -1),
    "ADT_A02.OBSERVATION": (0, -1),
    "ADT_A05.NEXT_OF_KIN": (0, -1), "ADT_A05.OBSERVATION": (0, -1), "ADT_A05.PROCEDURE": (0, -1), "ADT_A05.INSURANCE": (0, -1),
    "ADT_A06.NEXT_OF_KIN": (0, -1), "ADT_A06.OBSERVATION": (0, -1), "ADT_A06.PROCEDURE": (0, -1), "ADT_A06.INSURANCE": (0, -1),
    "ADT_A09.OBSERVATION": (0, -1), "ADT_A11.OBSERVATION": (0, -1), "ADT_A17.OBSERVATION": (0, -1),
    "MDM_T02.COMMON_ORDER": (0, -1), "MDM_T02.COMMON_ORDER.TIMING": (0, -1), "MDM_T02.OBSERVATION": (0, -1),
    "OML_O21.PATIENT": (0, 1), "OML_O21.PATIENT.INSURANCE": (0, -1), "OML_O21.PATIENT.PATIENT_VISIT": (0, 1),
    "OML_O21.ORDER": (1, -1), "OML_O21.ORDER.TIMING": (0, -1), "OML_O21.ORDER.OBSERVATION_REQUEST": (0, 1),
    "OML_O21.ORDER.OBSERVATION_REQUEST.OBSERVATION": (0, -1), "OML_O21.ORDER.OBSERVATION_REQUEST.SPECIMEN": (0, -1),
    "ORM_O01.PATIENT": (0, 1), "ORM_O01.PATIENT.INSURANCE": (0, -1), "ORM_O01.PATIENT.VISIT": (0, 1),
    "ORM_O01.ORDER": (1, -1), "ORM_O01.ORDER_DETAIL": (0, -1), "ORM_O01.ORDER_DETAIL.CHOICE": (1, 1), "ORM_O01.ORDER_DETAIL.OBSERVATION": (0, -1),
    "ORU_R01.PATIENT_RESULT": (1, -1), "ORU_R01.PATIENT_RESULT.PATIENT": (0, 1),
    "ORU_R01.PATIENT_RESULT.PATIENT.PATIENT_OBSERVATION": (0, -1), "ORU_R01.PATIENT_RESULT.PATIENT.VISIT": (0, 1),
    "ORU_R01.PATIENT_RESULT.ORDER_OBSERVATION": (1, -1), "ORU_R01.PATIENT_RESULT.ORDER_OBSERVATION.COMMON_ORDER": (0, 1),
    "ORU_R01.PATIENT_RESULT.ORDER_OBSERVATION.OBSERVATION": (0, -1), "ORU_R01.PATIENT_RESULT.ORDER_OBSERVATION.SPECIMEN": (0, -1),
    "ORU_R01.PATIENT_RESULT.ORDER_OBSERVATION.SPECIMEN.SPECIMEN_OBSERVATION": (0, -1),
    "SIU_S12.PATIENT": (0, -1), "SIU_S12.RESOURCES": (1, -1), "SIU_S12.RESOURCES.SERVICE": (0, -1),
    "SIU_S12.RESOURCES.GENERAL_RESOURCE": (0, -1), "SIU_S12.RESOURCES.LOCATION_RESOURCE": (0, -1), "SIU_S12.RESOURCES.PERSONNEL_RESOURCE": (0, -1),
    "VXU_V04.PATIENT_VISIT": (0, 1), "VXU_V04.PERSON_OBSERVATION": (0, -1), "VXU_V04.ORDER": (0, -1), "VXU_V04.ORDER.OBSERVATION": (0, -1),
}

# Segments the standard places that the IG has no row for, inserted after the
# named row so the walk consumes them where they belong (an order's NTE after
# OBR would otherwise open the OBSERVATION group it cannot start): (after
# path, path, min, max, why).
STRUCTURE_SUPPLEMENT = {
    "ORU_R01": [("ORU_R01.PATIENT_RESULT.ORDER_OBSERVATION.OBR", "ORU_R01.PATIENT_RESULT.ORDER_OBSERVATION.NTE", 0, -1,
                 "no map row in the IG for the order's notes (NTE after OBR)")],
}


def main():
    global ext_contexts
    pkg, out_path = sys.argv[1], sys.argv[2]
    ext_pkg = sys.argv[3] if len(sys.argv) > 3 else os.path.join(os.path.dirname(pkg), "..", "package-ext", "package")
    r4_pkg = sys.argv[4] if len(sys.argv) > 4 else os.path.join(os.path.dirname(pkg), "..", "package-r4", "package")
    ext_contexts = load_extension_contexts(ext_pkg)
    load_fhir_types(r4_pkg)
    if len(fhir_children) < 500:
        sys.exit(f"{r4_pkg}: the R4 package's StructureDefinitions are needed to check the maps' target paths")
    cms = {}
    for f in glob.glob(os.path.join(pkg, "ConceptMap-*.json")):
        c = json.load(open(f))
        cms[c["id"]] = c
    for cid in cms:
        m0 = re.match(r"^segment-([a-z0-9]+)-", cid)
        if m0:
            SEGMENT_NAMES.add(m0.group(1).upper())
    ids = set(cms)
    stats = {"rows": 0, "compiled": 0, "skipped": {}, "context moves": 0, "value keys respelt": 0, "concatenations": 0, "component bindings": 0}

    def skip(why):
        stats["skipped"][why.split(":")[0]] = stats["skipped"].get(why.split(":")[0], 0) + 1

    lines = ["//! @generated by tools/gen_v2.py from hl7.fhir.uv.v2mappings 1.0.0 — do not edit by hand.",
             "//! What the pod executes of the HL7 v2-to-FHIR mappings, and what it does not (`Skipped`, with reasons).",
             "", "use super::compiled::*;", ""]

    def context_moves(rows, owner):
        """`extension[k]` → `_elem.extension[k]` for the extension rows whose
        definition says they sit on a primitive element of `owner`
        (`contactpoint-local` on `ContactPoint.value`), so every row of that
        block lands where the definition allows."""
        moves = {}
        for _src, _sd, _si, t, tinfo in rows:
            display = (t.get("code") or "").strip()
            m = re.match(r"^(extension\[\d+\])\.url$", display)
            a = (tinfo.get("assignment") or "").strip()
            if not m or not (a.startswith('"') and a.endswith('"')):
                continue
            url = a[1:-1]
            elems = [c for c in ext_contexts.get(url, []) if c.startswith(owner + ".") and c.count(".") == 1]
            if len(elems) == 1 and not any(c == owner or c in ("Element", "*") for c in ext_contexts.get(url, [])):
                moves[m.group(1)] = "_" + elems[0].split(".", 1)[1] + "." + m.group(1)
        return moves

    def context_refusals(rows, owner):
        """The extension blocks whose definition cannot sit where the map puts
        them (`namingsystem-checkDigit`, defined for NamingSystem, on an
        Identifier; `alternate-codes`, defined for code/string/Coding, on an
        Appointment). The block's element is resolved through the FHIR types:
        its type and its definition path (`Patient.identifier`) are what a
        context names. A datatype map's rows are used from many places, so a
        dotted resource context (`Patient.identifier`) is given the benefit
        of the doubt there. Placing a refused block makes FHIR no validator
        accepts, so its rows are skipped, with the reason."""
        generic = {"Element", "*", "BackboneElement", "Resource", "DomainResource"}
        refused = {}
        for _src, _sd, _si, t, tinfo in rows:
            display = (t.get("code") or "").strip()
            m = re.match(r"^(.*?)\.?(extension\[\d+\])\.url$", display)
            a = (tinfo.get("assignment") or "").strip()
            if not m or not (a.startswith('"') and a.endswith('"')):
                continue
            ctxs = ext_contexts.get(a[1:-1]) or []
            if not ctxs:
                continue
            parent = m.group(1)
            if parent:
                missing, ty, dpath = resolve_element(owner, parent)
                if missing or ty is None:
                    continue  # the target check reports the path
                ty = "BackboneElement" if "." in ty else ty
            else:
                ty, dpath = owner, owner
            in_datatype = owner in DATATYPE_NAMES
            ok = False
            for c in ctxs:
                if c in generic or c == ty or c == dpath:
                    ok = True
                elif "." in c and (in_datatype or c == f"{owner}.{dpath}" or dpath.endswith("." + c.split(".", 1)[1]) and c.split(".", 1)[0] == owner):
                    ok = True
                elif (ty in generic or ty == "BackboneElement") and c.split(".", 1)[0] == owner:
                    ok = True
            if not ok:
                block = (parent + "." if parent else "") + m.group(2)
                refused[block] = f"extension context: {a[1:-1].rsplit('/', 1)[-1]} is defined for {', '.join(ctxs)}, not {dpath} ({ty})"
        return refused

    def value_respellings(rows):
        """`extension[k].valueCode` where the extension's definition says its
        value is a string (`iso21090-ADXP-censusTract`): the definition
        decides the key — the value is the same, only the map's spelling of
        its type is wrong. Only a primitive for a primitive."""
        urls = {}
        for _src, _sd, _si, t, tinfo in rows:
            display = (t.get("code") or "").strip()
            m = re.match(r"^(.*extension\[\d+\])\.url$", display)
            a = (tinfo.get("assignment") or "").strip()
            if m and a.startswith('"') and a.endswith('"'):
                urls[m.group(1)] = a[1:-1]
        respell = {}
        for block, url in urls.items():
            types = ext_value_types.get(url) or []
            if len(types) == 1 and types[0] in PRIMITIVES:
                want = types[0]
                for _src, _sd, _si, t, _ti in rows:
                    display = spell_choices((t.get("code") or "").strip())
                    m = re.match(r"^" + re.escape(block) + r"\.value([A-Z][A-Za-z]+)$", display)
                    if m:
                        got = m.group(1)[0].lower() + m.group(1)[1:]
                        if got != want and got in PRIMITIVES:
                            respell[display] = f"{block}.value{want[0].upper()}{want[1:]}"
        return respell

    def emit_rows(rows, kind, fhir=None, owner=None, v2=None):
        """rows: list of (source code, source display, source typeinfo,
        target, target typeinfo); `fhir` the datatype map's own FHIR type,
        whose name some rows prefix their targets with (`Quantity.code`);
        `owner` the FHIR type the rows fill, for extension contexts."""
        out = []
        moves = context_moves(rows, owner) if owner else {}
        respell = value_respellings(rows)
        stats["value keys respelt"] += len(respell)
        refusals = context_refusals(rows, owner) if owner else {}
        stats["context moves"] += len(moves)
        for src, sdisplay, sinfo, t, tinfo in rows:
            stats["rows"] += 1
            display = (t.get("code") or "").strip()
            if fhir and display.lower().startswith(fhir.lower() + "."):
                display = display[len(fhir) + 1:]
            display = respell.get(spell_choices(display), display)
            for block, moved in moves.items():
                if display == block or display.startswith(block + "."):
                    display = moved + display[len(block):]
            sref = parse_ref(src) if kind != "message" else None
            reason = None
            for block, why in refusals.items():
                if display == block or display.startswith(block + "."):
                    reason = why
            if kind == "datatype":
                if sref is None or sref[2] == 0:
                    reason = f"source form: {src}"
            elif kind == "segment":
                if sref is None:
                    reason = f"source form: {src}"
            tgt = parse_target(display)
            if tgt is None and reason is None:
                reason = f"target form: {display}"
            # a path the FHIR type does not have (an IG typo — `extension.uri`,
            # `valueSampedData` — or an R5 element) is never written
            if reason is None and owner and tgt[0] not in ("$this", "$value"):
                gone = missing_element(owner, tgt[0])
                if gone is not None:
                    reason = f"no such element: {gone} (target {display})"
            assign = tinfo.get("assignment")
            fixed = None
            concat = None
            if assign is not None and reason is None:
                a = assign.strip()
                if a.startswith('"') and a.endswith('"') and len(a) >= 2 and a.count('"') == 2:
                    fixed = a[1:-1]
                else:
                    concat = parse_concat(a)
                    if concat is None:
                        reason = f"assignment: {a[:60]}"
                    else:
                        # what the row can see: the datatype being mapped (a datatype row) or the field's own type
                        carried = v2 or (sinfo.get("type") or "").split("(")[0]
                        foreign = sorted({m2.group(1) for m2 in re.finditer(r"Part::Ref\(Ref \{ name: \"([A-Z0-9]+)\", field: 0", concat)
                                          if m2.group(1) != carried and m2.group(1) not in SEGMENT_NAMES})
                        if foreign:
                            reason = f"assignment names {', '.join(foreign)}, which the row's source does not carry"
                            concat = None
            if reason is None and fixed is not None and tgt and tgt[1]:
                reason = "a literal for a reference"
            cond, creason = compile_cond(t)
            if reason is None and creason:
                reason = creason
            via = tinfo.get("mappedVia")
            via_rs = "Via::None"
            # a coding-system name (`CWE.3`, "Name of Coding System") to a
            # `system` uri: the IG says a vocabulary table gives the uri and
            # ships none; the engine resolves the standard names (table 0396)
            if reason is None and display.split(".")[-1] == "system" and re.search(r"coding system$", sdisplay, re.I):
                via_rs = "Via::CodeSystem"
                via = None
            if via and reason is None:
                if via.startswith("ConceptMap/table-"):
                    tid = via.split("/", 1)[1]
                    via_rs = f"Via::Table({rs(tid)})" if tid in ids else None
                    if via_rs is None:
                        reason = f"table missing: {tid}"
                else:
                    via_rs = "Via::Unspecified"
            ty = (tinfo.get("type") or "").split("(")[0]
            sty = (sinfo.get("type") or "").split("(")[0]
            f_, c_, s_ = (sref[1], sref[2], sref[3]) if sref else (0, 0, 0)
            if reason is not None:
                skip(reason)
                out.append(f"    Row {{ src: {rs(src)}, src_ty: {rs(sty)}, field: {f_}, component: {c_}, sub: {s_}, target: {rs(display)}, ty: {rs(ty)}, reference: None, fixed: None, concat: None, via: Via::None, cond: Cond::Skipped({rs(reason)}) }},")
                continue
            stats["compiled"] += 1
            if concat is not None:
                stats["concatenations"] += 1
            path, refty = tgt
            out.append(
                f"    Row {{ src: {rs(src)}, src_ty: {rs(sty)}, field: {f_}, component: {c_}, sub: {s_}, target: {rs(path)}, ty: {rs(ty)}, "
                f"reference: {('Some(' + rs(refty) + ')') if refty else 'None'}, fixed: {('Some(' + rs(fixed) + ')') if fixed is not None else 'None'}, "
                f"concat: {('Some(' + concat + ')') if concat is not None else 'None'}, "
                f"via: {via_rs}, cond: {cond} }},"
            )
        return out

    def rows_of(c):
        for g in c.get("group", []):
            for e in g.get("element", []):
                sinfo = typeinfo(e.get("extension"))
                for t in e.get("target", []):
                    yield (e.get("code") or "", e.get("display") or "", sinfo, t, typeinfo(t.get("extension")))

    # datatypes
    dts = sorted(n for n in cms if n.startswith("datatype-"))
    lines.append("pub static DATATYPES: &[DatatypeMap] = &[")
    for n in dts:
        m = re.match(r"^datatype-([a-z0-9]+)-(?:([a-z0-9]+)-)?to-([a-z0-9-]+)$", n)
        if not m:
            continue
        v2, qualifier, fhir = m.group(1).upper(), m.group(2), m.group(3)
        lines.append(f"    DatatypeMap {{ id: {rs(n)}, v2: {rs(v2)}, qualifier: {rs(qualifier or '')}, fhir: {rs(fhir)}, rows: &[")
        owner = FHIR_TYPE_NAMES.get(fhir.lower())
        lines += ["    " + l for l in emit_rows(list(rows_of(cms[n])), "datatype", fhir=fhir, owner=owner, v2=v2)]
        lines.append("    ] },")
    lines.append("];\n")

    # segments
    segs = sorted(n for n in cms if n.startswith("segment-"))
    lines.append("pub static SEGMENTS: &[SegmentMap] = &[")
    for n in segs:
        m = re.match(r"^segment-([a-z0-9]+)-(?:([a-z0-9-]+)-)?to-([a-z0-9-]+)$", n)
        if not m:
            continue
        seg, qualifier, res = m.group(1).upper(), m.group(2) or "", m.group(3)
        lines.append(f"    SegmentMap {{ id: {rs(n)}, segment: {rs(seg)}, qualifier: {rs(qualifier)}, resource: {rs(res)}, rows: &[")
        lines += ["    " + l for l in emit_rows(list(rows_of(cms[n])), "segment", owner=FHIR_TYPE_NAMES.get(res.lower()))]
        lines.append("    ] },")
    lines.append("];\n")

    # messages
    msgs = sorted(n for n in cms if n.startswith("message-"))
    lines.append("pub static MESSAGES: &[MessageMap] = &[")
    seg_by_key = {}
    for n in segs:
        m = re.match(r"^segment-([a-z0-9]+)-(?:([a-z0-9-]+)-)?to-([a-z0-9-]+)$", n)
        if m:
            seg_by_key.setdefault((m.group(1).upper(), m.group(3)), []).append(n)
    for n in msgs:
        m = re.match(r"^message-([a-z]+)-([a-z0-9]+)-to-bundle$", n)
        if not m:
            continue
        name = f"{m.group(1).upper()}_{m.group(2).upper()}"
        lines.append(f"    MessageMap {{ id: {rs(n)}, name: {rs(name)}, rows: &[")
        group_paths = []
        msg_rows = list(rows_of(cms[n]))
        # the standard's segments the IG has no row for, consumed at their
        # place so they neither start a group they cannot start nor hide
        for after, spath, smn, smx, why in STRUCTURE_SUPPLEMENT.get(name, []):
            idx = max(i for i, (src0, *_r) in enumerate(msg_rows) if src0.strip() == after)
            msg_rows.insert(idx + 1, (spath, "", {"cardinalityMin": smn, "cardinalityMax": smx}, {"code": ""}, {"supplement": why}))
        for src, _sdisplay, sinfo, t, tinfo in msg_rows:
            stats["rows"] += 1
            path = src.strip()
            seg = path.rsplit(".", 1)[-1].split(":")[-1]
            # `ADT_A01:follow:PID.ROL` — a segment that follows another
            if ":follow:" in path:
                seg = path.rsplit(".", 1)[-1]
            group = path.rsplit(".", 1)[0] if "." in path else path
            if ":follow:" in group:
                group = group.split(":follow:")[0]
            parts = group.split(".")
            for i in range(2, len(parts) + 1):
                gp = ".".join(parts[:i])
                if gp not in group_paths:
                    group_paths.append(gp)
            mn = sinfo.get("cardinalityMin")
            mx = sinfo.get("cardinalityMax")
            mn = 0 if mn is None else int(mn)
            mx = -1 if mx is None else int(mx)
            display = (t.get("code") or "").strip()
            # `Observation[2]`, or `Observation[2].note`: the segment's rows
            # land under `note` of that instance (the segment map's own
            # targets say so; checked below)
            tm = re.match(r"^([A-Z][A-Za-z]+)(?:\[([^\]]+)\])?(?:\.([a-z][A-Za-z.]*))?$", display)
            reason = None
            at = ""
            if tinfo.get("supplement"):
                reason = tinfo["supplement"]
                res, inst = "", "Each"
            elif not tm:
                reason = f"target form: {display}"
                res, inst = "", "Each"
            else:
                res = tm.group(1)
                mk = tm.group(2)
                at = tm.group(3) or ""
                inst = "Each" if mk in (None, "n", "each") or (mk and (".." in mk or "-" in mk)) else f"Numbered({mk})"
            cond, creason = compile_cond(t)
            if reason is None and creason:
                reason = creason
            map_id = None
            if reason is None:
                cands = seg_by_key.get((seg, res.lower()), [])
                if res == "Bundle":
                    cands = [c for c in cands]
                # an OBX whose value repeats goes through the component map, as
                # the IG's own comment says ("see OBX[Observation-Component] map")
                antlr = " ".join(d.get("value") or "" for d in t.get("dependsOn", []) if d.get("property") == "Computable-ANTLR")
                if seg == "OBX" and re.search(r"OBX-5\s+LST\.count\s+GREATER THAN 1", antlr) and "segment-obx-component-to-observation" in cands:
                    cands = ["segment-obx-component-to-observation"]
                    stats["component bindings"] += 1
                if len(cands) == 1:
                    map_id = cands[0]
                elif len(cands) > 1:
                    # prefer the unqualified map (`segment-pid-to-patient`)
                    plain = [c for c in cands if c == f"segment-{seg.lower()}-to-{res.lower()}"]
                    map_id = plain[0] if plain else None
                    if map_id is None:
                        reason = f"segment map ambiguous: {', '.join(cands)}"
                else:
                    reason = f"no segment map: {seg} to {res}"
            if reason is None and at and map_id is not None:
                targets = [(t2.get("code") or "").strip() for _s, _d, _si, t2, _ti in rows_of(cms[map_id])]
                if not all(t2 == at or t2.startswith(at + ".") or t2.startswith(at + "[") for t2 in targets if t2):
                    reason = f"target form: {display} (the segment map does not target {at})"
            if reason is not None:
                skip(reason)
                lines.append(f"        MsgRow {{ path: {rs(path)}, group: {rs(group)}, segment: {rs(seg)}, min: {mn}, max: {mx}, resource: {rs(res)}, instance: Instance::Each, at: \"\", map: None, cond: Cond::Skipped({rs(reason)}) }},")
                continue
            stats["compiled"] += 1
            lines.append(f"        MsgRow {{ path: {rs(path)}, group: {rs(group)}, segment: {rs(seg)}, min: {mn}, max: {mx}, resource: {rs(res)}, instance: Instance::{inst}, at: {rs(at)}, map: Some({rs(map_id)}), cond: {cond} }},")
        lines.append("    ], groups: &[")
        for gp in group_paths:
            if gp not in GROUP_CARDINALITY:
                sys.exit(f"gen_v2: no cardinality for group {gp} — add it to GROUP_CARDINALITY from the standard's abstract message syntax")
            gmn, gmx = GROUP_CARDINALITY[gp]
            lines.append(f"        Group {{ path: {rs(gp)}, min: {gmn}, max: {gmx} }},")
        lines.append("    ] },")
    lines.append("];\n")

    # tables
    tbls = sorted(n for n in cms if n.startswith("table-"))
    lines.append("pub static TABLES: &[TableMap] = &[")
    for n in tbls:
        c = cms[n]
        g = c["group"][0] if c.get("group") else {}
        src_sys = g.get("source") or ""
        tgt_sys = g.get("target") or ""
        pairs = []
        for gg in c.get("group", []):
            for e in gg.get("element", []):
                for t in e.get("target", []):
                    if t.get("code") and t.get("equivalence") in ("equivalent", "equal", "wider", "narrower", "relatedto", "inexact", None):
                        pairs.append((e["code"], t["code"], gg.get("target") or tgt_sys))
        lines.append(f"    TableMap {{ id: {rs(n)}, source: {rs(src_sys)}, target: {rs(tgt_sys)}, pairs: &[")
        for a, b, sysb in pairs:
            lines.append(f"        ({rs(a)}, {rs(b)}, {rs(sysb)}),")
        lines.append("    ] },")
    lines.append("];\n")

    with open(out_path, "w") as f:
        f.write("\n".join(lines) + "\n")
    print(f"wrote {out_path}: {len(dts)} datatype maps, {len(segs)} segment maps, {len(msgs)} message maps, {len(tbls)} tables; "
          f"rows {stats['rows']}, compiled {stats['compiled']} ({stats['concatenations']} concatenations, "
          f"{stats['context moves']} extension blocks moved to their context, {stats['value keys respelt']} value keys respelt to the definition's type, {stats['component bindings']} OBX rows bound to the component map), "
          f"skipped {stats['rows'] - stats['compiled']}")
    for why, n in sorted(stats["skipped"].items(), key=lambda kv: -kv[1]):
        print(f"  skipped {n:>4}  {why}")


if __name__ == "__main__":
    main()
