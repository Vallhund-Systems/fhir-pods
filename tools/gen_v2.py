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


def parse_ref(tok):
    """`PID-7`, `OBX-5.1`, `XPN.12`, `CX.4.1`, `PRT-4` → (segment|datatype, field, component, sub)."""
    m = REF.match(tok)
    if not m:
        return None
    seg, f, c, s, dc, ds = m.groups()
    if f is not None:
        return (seg, int(f), int(c or 0), int(s or 0))
    if dc is not None:
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
        self.toks = re.findall(r'"[^"]*"|\(|\)|[^\s()]+', t)
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
        # `X AND Y VALUED` / `X AND Y NOT VALUED`: a shared predicate
        refs = [ref]
        save = self.i
        while self.eat("AND"):
            t2 = self.peek()
            r2 = parse_ref(t2) if t2 else None
            nxt = self.toks[self.i + 1].upper() if r2 and self.i + 1 < len(self.toks) else None
            if r2 and nxt in ("VALUED", "NOT", "IS", "ARE", "AREA"):
                self.i += 1
                refs.append(r2)
            else:
                self.i = save
                break
            save = self.i
        pred = self.predicate()
        out = None
        for r in refs:
            p = (pred[0], r) + tuple(pred[1:])
            out = p if out is None else ("and", out, p)
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
        if tok is None or not (tok.startswith('"') and tok.endswith('"')):
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
        return f"CondExpr::Equals({r}, {rs(e[2])})"
    if kind == "not_equals":
        return f"CondExpr::NotEquals({r}, {rs(e[2])})"
    if kind == "in":
        return f"CondExpr::In({r}, &[{', '.join(rs(x) for x in e[2])}])"
    if kind == "not_in":
        return f"CondExpr::NotIn({r}, &[{', '.join(rs(x) for x in e[2])}])"
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
    if not antlr:
        return None, "narrative condition: " + narrative[0][:80]
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
    return contexts


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

ext_contexts = {}


def main():
    global ext_contexts
    pkg, out_path = sys.argv[1], sys.argv[2]
    ext_pkg = sys.argv[3] if len(sys.argv) > 3 else os.path.join(os.path.dirname(pkg), "..", "package-ext", "package")
    ext_contexts = load_extension_contexts(ext_pkg)
    cms = {}
    for f in glob.glob(os.path.join(pkg, "ConceptMap-*.json")):
        c = json.load(open(f))
        cms[c["id"]] = c
    ids = set(cms)
    stats = {"rows": 0, "compiled": 0, "skipped": {}, "context moves": 0, "concatenations": 0}

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

    def emit_rows(rows, kind, fhir=None, owner=None):
        """rows: list of (source code, source display, source typeinfo,
        target, target typeinfo); `fhir` the datatype map's own FHIR type,
        whose name some rows prefix their targets with (`Quantity.code`);
        `owner` the FHIR type the rows fill, for extension contexts."""
        out = []
        moves = context_moves(rows, owner) if owner else {}
        stats["context moves"] += len(moves)
        for src, sdisplay, sinfo, t, tinfo in rows:
            stats["rows"] += 1
            display = (t.get("code") or "").strip()
            if fhir and display.lower().startswith(fhir.lower() + "."):
                display = display[len(fhir) + 1:]
            for block, moved in moves.items():
                if display == block or display.startswith(block + "."):
                    display = moved + display[len(block):]
            sref = parse_ref(src) if kind != "message" else None
            reason = None
            if kind == "datatype":
                if sref is None or sref[2] == 0:
                    reason = f"source form: {src}"
            elif kind == "segment":
                if sref is None:
                    reason = f"source form: {src}"
            tgt = parse_target(display)
            if tgt is None and reason is None:
                reason = f"target form: {display}"
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
        lines += ["    " + l for l in emit_rows(list(rows_of(cms[n])), "datatype", fhir=fhir, owner=owner)]
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
        for src, _sdisplay, sinfo, t, tinfo in rows_of(cms[n]):
            stats["rows"] += 1
            path = src.strip()
            seg = path.rsplit(".", 1)[-1].split(":")[-1]
            # `ADT_A01:follow:PID.ROL` — a segment that follows another
            if ":follow:" in path:
                seg = path.rsplit(".", 1)[-1]
            group = path.rsplit(".", 1)[0] if "." in path else path
            if ":follow:" in group:
                group = group.split(":follow:")[0]
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
            if not tm:
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
          f"{stats['context moves']} extension blocks moved to their context), skipped {stats['rows'] - stats['compiled']}")
    for why, n in sorted(stats["skipped"].items(), key=lambda kv: -kv[1]):
        print(f"  skipped {n:>4}  {why}")


if __name__ == "__main__":
    main()
