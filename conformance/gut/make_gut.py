#!/usr/bin/env python3
"""The gut-path scenario: a synthetic clinical instrument shaped like a
sixteen-part gut-health questionnaire, and the resource chain a host builds
around it — the questionnaire as a FHIR `Questionnaire` with its own code
systems and value sets, `QuestionnaireResponse`s held to it, the score /
stool-form / severity `Observation`s derived from a response, a `Flag` for a
positive alarm item, a clinician-summary `Composition`, a referral
`ServiceRequest`, the research-export `Consent`, the de-identified export
`Bundle`, and the `AuditEvent` that records the export.

Everything here is synthetic. Nothing is copied from any vendor material or
any real instrument: the parts, items, codes and wording are placeholders
with the *shape* of such an instrument (nested groups, enableWhen, required
items, repeating groups, coded answers from a local value set, integer and
decimal scales, a Bristol-style 1–7 choice, an eleven-item boolean safety
screen), so the pod's handling of that shape can be measured end to end.

    python3 conformance/gut/make_gut.py        # writes conformance/gut/out/

`out/defs/` holds the definitions the oracle is given as an extra IG folder
(CodeSystem, ValueSets, Questionnaire); `out/cases/` the instances with a
`manifest.json` of expectations; `out/gut.terminology.txt` the compiled
table the pod loads for the local value sets.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "out")
DEFS = os.path.join(OUT, "defs")
CASES = os.path.join(OUT, "cases")

BASE = "https://example.org/gut"
CS_ITEMS = f"{BASE}/CodeSystem/answers"
CS_OBS = f"{BASE}/CodeSystem/observations"
Q_URL = f"{BASE}/Questionnaire/gut-instrument"
Q_VERSION = "1.0.0"
VS = {
    "scale04": f"{BASE}/ValueSet/scale-0-4",
    "bristol": f"{BASE}/ValueSet/bristol-1-7",
    "site": f"{BASE}/ValueSet/pain-site",
    "sex": f"{BASE}/ValueSet/sex-at-birth",
    "birthmode": f"{BASE}/ValueSet/birth-mode",
    "medclass": f"{BASE}/ValueSet/medication-class",
    "condition": f"{BASE}/ValueSet/history-condition",
    "relation": f"{BASE}/ValueSet/relation",
}
ANSWERS = {
    "scale04": [(str(i), f"Level {i}") for i in range(5)],
    "bristol": [(f"type-{i}", f"Bristol type {i}") for i in range(1, 8)],
    "site": [("epigastric", "Epigastric"), ("periumbilical", "Periumbilical"), ("lower-left", "Lower left"),
             ("lower-right", "Lower right"), ("diffuse", "Diffuse")],
    "sex": [("female", "Female"), ("male", "Male"), ("intersex", "Intersex")],
    "birthmode": [("vaginal", "Vaginal"), ("caesarean", "Caesarean"), ("unknown", "Unknown")],
    "medclass": [("ppi", "Proton-pump inhibitor"), ("nsaid", "NSAID"), ("antibiotic", "Antibiotic"),
                 ("opioid", "Opioid"), ("other", "Other")],
    "condition": [("ibs", "Irritable bowel syndrome"), ("ibd", "Inflammatory bowel disease"),
                  ("coeliac", "Coeliac disease"), ("gord", "Gastro-oesophageal reflux"), ("other", "Other")],
    "relation": [("parent", "Parent"), ("sibling", "Sibling"), ("child", "Child")],
}
OBS_CODES = [("triage-score", "Triage composite score"), ("stool-form", "Stool form, Bristol scale"),
             ("symptom-severity", "Symptom severity, 0 to 10"), ("alarm-item", "Positive alarm item")]


def coding(system, code, display=None):
    c = {"system": system, "code": code}
    if display:
        c["display"] = display
    return c


def choice(link, text, vs, required=False, repeats=False):
    it = {"linkId": link, "text": text, "type": "choice", "answerValueSet": VS[vs]}
    if required:
        it["required"] = True
    if repeats:
        it["repeats"] = True
    return it


def item(link, text, ty, required=False, repeats=False, **extra):
    it = {"linkId": link, "text": text, "type": ty}
    if required:
        it["required"] = True
    if repeats:
        it["repeats"] = True
    it.update(extra)
    return it


def group(link, text, items, required=False, repeats=False, enable=None):
    g = {"linkId": link, "text": text, "type": "group", "item": items}
    if required:
        g["required"] = True
    if repeats:
        g["repeats"] = True
    if enable:
        g["enableWhen"] = enable
    return g


def questionnaire():
    alarm = ["blood-in-stool", "black-stools", "weight-loss", "dysphagia", "vomiting", "worsening-pain",
             "fever", "nocturnal-diarrhoea", "bowel-habit-change", "dehydration", "family-history-gi-cancer"]
    parts = [
        group("p1", "Introduction and consent", [
            item("p1.consent", "Consent to the module", "boolean", required=True),
            item("p1.age", "Age in years", "integer", required=True),
            choice("p1.sex", "Sex at birth", "sex", required=True),
        ], required=True),
        group("p2", "Triage screen", [
            choice("p2.q1", "Overall gut health, self-rated", "scale04", required=True),
            choice("p2.q2", "Symptom frequency", "scale04", required=True),
            item("p2.q3", "Duration of symptoms in months", "integer", required=True),
            item("p2.q4", "Severity, 0 to 10", "integer", required=True),
            choice("p2.q5", "Functional impact", "scale04", required=True),
            choice("p2.q6", "Stool form, usual", "bristol", required=True),
            choice("p2.q7", "Abdominal pain and reflux", "scale04", required=True),
            item("p2.q8", "Onset narrative", "string"),
        ], required=True),
        group("p3", "Abdominal pain", [
            choice("p3.site", "Site", "site"),
            item("p3.intensity", "Intensity, 0 to 10", "integer"),
            item("p3.nocturnal", "Wakes from sleep", "boolean"),
        ]),
        group("p4", "Bowel health", [
            choice("p4.bristol", "Stool form over the last fortnight", "bristol"),
            item("p4.motions", "Motions per week", "integer"),
            item("p4.straining", "Straining", "boolean"),
            item("p4.blood", "Visible blood", "boolean"),
        ]),
        group("p5", "Reflux and upper GI", [
            choice("p5.heartburn", "Heartburn frequency", "scale04"),
            item("p5.dysphagia", "Difficulty swallowing", "boolean"),
            item("p5.ppi", "Acid-suppression use in days", "integer"),
        ]),
        group("p6", "Food and digestive response", [
            item("p6.triggers", "Trigger foods", "string", repeats=True),
            item("p6.latency", "Symptom latency in minutes", "integer"),
            item("p6.reproducible", "Reproducible", "boolean"),
        ]),
        group("p7", "Dietary diversity", [
            item("p7.plants", "Distinct plants per week", "integer"),
            item("p7.fermented", "Fermented foods per week", "integer"),
            item("p7.upf", "Ultra-processed share, 0 to 1", "decimal"),
        ]),
        group("p8", "Microbiome influences", [
            item("p8.antibiotics", "Antibiotic courses, last twelve months", "integer"),
            choice("p8.birth", "Birth mode", "birthmode"),
            item("p8.travel", "Travel-associated infection", "boolean"),
        ]),
        group("p9", "Medication and supplement review", [
            item("p9.name", "Medicine or supplement", "string", required=True),
            choice("p9.class", "Class", "medclass", required=True),
            item("p9.duration", "Duration in days", "integer"),
        ], repeats=True),
        group("p10", "Gut-brain axis", [
            item("p10.stress", "Perceived stress, 0 to 10", "integer"),
            choice("p10.coupling", "Symptom-to-stress coupling", "scale04"),
            item("p10.avoidance", "Avoidance behaviour", "boolean"),
        ]),
        group("p11", "Sleep, stress and recovery", [
            choice("p11.quality", "Sleep quality", "scale04"),
            item("p11.hours", "Hours of sleep", "decimal"),
            item("p11.nocturnal", "Night waking with symptoms", "boolean"),
        ]),
        group("p12", "Hormonal connection", [
            item("p12.cycleday", "Cycle day at response", "integer"),
            item("p12.phase-severity", "Phase-linked severity, 0 to 10", "integer"),
            item("p12.pregnant", "Currently pregnant", "boolean"),
        ], enable=[{"question": "p1.sex", "operator": "=",
                    "answerCoding": coding(CS_ITEMS, "female")}]),
        group("p13", "Gut-skin axis", [
            item("p13.skinflag", "Skin condition present", "boolean"),
            group("p13.detail", "Skin detail", [
                choice("p13.flares", "Flare frequency", "scale04"),
                item("p13.concordance", "Flares track gut symptoms", "boolean"),
            ], enable=[{"question": "p13.skinflag", "operator": "=", "answerBoolean": True}]),
        ]),
        group("p14", "Medical history", [
            choice("p14.condition", "Condition", "condition", required=True),
            item("p14.date", "Date of diagnosis", "date"),
        ], repeats=True),
        group("p15", "Family history", [
            choice("p15.relation", "Relation", "relation", required=True),
            choice("p15.condition", "Condition", "condition", required=True),
            item("p15.age", "Age at diagnosis", "integer"),
        ], repeats=True),
        group("p16", "Clinical safety screen", [
            item(f"p16.{a}", f"Alarm item: {a.replace('-', ' ')}", "boolean", required=True) for a in alarm
        ], required=True),
    ]
    return {
        "resourceType": "Questionnaire",
        "id": "gut-instrument",
        "url": Q_URL,
        "version": Q_VERSION,
        "name": "GutInstrument",
        "title": "Synthetic gut-health instrument",
        "status": "active",
        "subjectType": ["Patient"],
        "date": "2026-10-09",
        "item": parts,
    }


def code_system():
    concepts = []
    seen = set()
    for codes in ANSWERS.values():
        for code, display in codes:
            if code not in seen:
                seen.add(code)
                concepts.append({"code": code, "display": display})
    return {
        "resourceType": "CodeSystem", "id": "gut-answers", "url": CS_ITEMS, "version": "1.0.0",
        "name": "GutAnswers", "status": "active", "content": "complete", "caseSensitive": True,
        "concept": concepts,
    }


def obs_code_system():
    return {
        "resourceType": "CodeSystem", "id": "gut-observations", "url": CS_OBS, "version": "1.0.0",
        "name": "GutObservations", "status": "active", "content": "complete", "caseSensitive": True,
        "concept": [{"code": c, "display": d} for c, d in OBS_CODES],
    }


def value_set(key):
    return {
        "resourceType": "ValueSet", "id": f"gut-{key}", "url": VS[key], "version": "1.0.0",
        "name": "Gut" + key.capitalize(), "status": "active",
        "compose": {"include": [{"system": CS_ITEMS, "concept": [{"code": c, "display": d} for c, d in ANSWERS[key]]}]},
    }


def terminology_table():
    lines = ["fhir-pods-terminology 1"]
    for key, url in VS.items():
        lines.append(f"= {url}")
        for code, display in ANSWERS[key]:
            lines.append(f"{CS_ITEMS}\t{code}\t{display}")
    return "\n".join(lines) + "\n"


# --- responses -------------------------------------------------------------

def ans_coding(key, code):
    display = dict(ANSWERS[key])[code]
    return {"valueCoding": coding(CS_ITEMS, code, display)}


def qi(link, *answers, items=None):
    it = {"linkId": link}
    if answers:
        it["answer"] = list(answers)
    if items:
        it["item"] = items
    return it


def qg(link, items):
    return {"linkId": link, "item": items}


def safety(positive=None):
    alarm = ["blood-in-stool", "black-stools", "weight-loss", "dysphagia", "vomiting", "worsening-pain",
             "fever", "nocturnal-diarrhoea", "bowel-habit-change", "dehydration", "family-history-gi-cancer"]
    return qg("p16", [qi(f"p16.{a}", {"valueBoolean": a == positive}) for a in alarm])


def response_full(sex="female", positive=None, status="completed"):
    items = [
        qg("p1", [qi("p1.consent", {"valueBoolean": True}), qi("p1.age", {"valueInteger": 41}),
                  qi("p1.sex", ans_coding("sex", sex))]),
        qg("p2", [qi("p2.q1", ans_coding("scale04", "2")), qi("p2.q2", ans_coding("scale04", "3")),
                  qi("p2.q3", {"valueInteger": 18}), qi("p2.q4", {"valueInteger": 6}),
                  qi("p2.q5", ans_coding("scale04", "2")), qi("p2.q6", ans_coding("bristol", "type-6")),
                  qi("p2.q7", ans_coding("scale04", "3")), qi("p2.q8", {"valueString": "After a course of antibiotics."})]),
        qg("p3", [qi("p3.site", ans_coding("site", "lower-left")), qi("p3.intensity", {"valueInteger": 5}),
                  qi("p3.nocturnal", {"valueBoolean": False})]),
        qg("p4", [qi("p4.bristol", ans_coding("bristol", "type-6")), qi("p4.motions", {"valueInteger": 14}),
                  qi("p4.straining", {"valueBoolean": False}), qi("p4.blood", {"valueBoolean": False})]),
        qg("p5", [qi("p5.heartburn", ans_coding("scale04", "1")), qi("p5.dysphagia", {"valueBoolean": False}),
                  qi("p5.ppi", {"valueInteger": 0})]),
        qg("p6", [qi("p6.triggers", {"valueString": "Onions"}, {"valueString": "Wheat"}),
                  qi("p6.latency", {"valueInteger": 90}), qi("p6.reproducible", {"valueBoolean": True})]),
        qg("p7", [qi("p7.plants", {"valueInteger": 18}), qi("p7.fermented", {"valueInteger": 2}),
                  qi("p7.upf", {"valueDecimal": 0.35})]),
        qg("p8", [qi("p8.antibiotics", {"valueInteger": 1}), qi("p8.birth", ans_coding("birthmode", "vaginal")),
                  qi("p8.travel", {"valueBoolean": False})]),
        qg("p9", [qi("p9.name", {"valueString": "Omeprazole"}), qi("p9.class", ans_coding("medclass", "ppi")),
                  qi("p9.duration", {"valueInteger": 120})]),
        qg("p9", [qi("p9.name", {"valueString": "Probiotic"}), qi("p9.class", ans_coding("medclass", "other"))]),
        qg("p10", [qi("p10.stress", {"valueInteger": 7}), qi("p10.coupling", ans_coding("scale04", "3")),
                   qi("p10.avoidance", {"valueBoolean": True})]),
        qg("p11", [qi("p11.quality", ans_coding("scale04", "2")), qi("p11.hours", {"valueDecimal": 6.5}),
                   qi("p11.nocturnal", {"valueBoolean": False})]),
    ]
    if sex == "female":
        items.append(qg("p12", [qi("p12.cycleday", {"valueInteger": 12}), qi("p12.phase-severity", {"valueInteger": 4}),
                                qi("p12.pregnant", {"valueBoolean": False})]))
    items += [
        qg("p13", [qi("p13.skinflag", {"valueBoolean": True}),
                   qg("p13.detail", [qi("p13.flares", ans_coding("scale04", "2")),
                                     qi("p13.concordance", {"valueBoolean": True})])]),
        qg("p14", [qi("p14.condition", ans_coding("condition", "ibs")), qi("p14.date", {"valueDate": "2021-03"})]),
        qg("p15", [qi("p15.relation", ans_coding("relation", "parent")),
                   qi("p15.condition", ans_coding("condition", "coeliac")), qi("p15.age", {"valueInteger": 52})]),
        safety(positive),
    ]
    return {
        "resourceType": "QuestionnaireResponse", "id": "qr-full",
        "meta": {"profile": ["http://hl7.org/fhir/StructureDefinition/QuestionnaireResponse"]},
        "questionnaire": f"{Q_URL}|{Q_VERSION}",
        "status": status, "subject": {"reference": "Patient/p1"}, "authored": "2026-10-09T09:30:00+10:00",
        "item": items,
    }


def response_minimal():
    return {
        "resourceType": "QuestionnaireResponse", "id": "qr-minimal", "questionnaire": Q_URL,
        "status": "completed", "subject": {"reference": "Patient/p1"}, "authored": "2026-10-09T09:30:00+10:00",
        "item": [
            qg("p1", [qi("p1.consent", {"valueBoolean": True}), qi("p1.age", {"valueInteger": 30}),
                      qi("p1.sex", ans_coding("sex", "male"))]),
            qg("p2", [qi("p2.q1", ans_coding("scale04", "4")), qi("p2.q2", ans_coding("scale04", "0")),
                      qi("p2.q3", {"valueInteger": 0}), qi("p2.q4", {"valueInteger": 0}),
                      qi("p2.q5", ans_coding("scale04", "0")), qi("p2.q6", ans_coding("bristol", "type-4")),
                      qi("p2.q7", ans_coding("scale04", "0"))]),
            safety(),
        ],
    }


def observation(oid, code, value, derived="QuestionnaireResponse/qr-full"):
    o = {
        "resourceType": "Observation", "id": oid, "status": "final",
        "meta": {"profile": ["http://hl7.org/fhir/StructureDefinition/Observation"]},
        "category": [{"coding": [coding("http://terminology.hl7.org/CodeSystem/observation-category", "survey")]}],
        "code": {"coding": [coding(CS_OBS, code, dict(OBS_CODES)[code])]},
        "subject": {"reference": "Patient/p1"}, "effectiveDateTime": "2026-10-09T09:30:00+10:00",
        "derivedFrom": [{"reference": derived}],
    }
    o.update(value)
    return o


def patient():
    return {
        "resourceType": "Patient", "id": "p1",
        "meta": {"profile": ["http://hl7.org/fhir/StructureDefinition/Patient"]},
        "identifier": [{"type": {"coding": [coding("http://terminology.hl7.org/CodeSystem/v2-0203", "MR")]},
                        "system": f"{BASE}/member-id", "value": "GUT-000001"}],
        "name": [{"family": "Synthetic", "given": ["Case"]}], "gender": "female", "birthDate": "1985-04-12",
        "address": [{"city": "Sydney", "state": "NSW", "postalCode": "2000", "country": "AU"}],
    }


def device():
    return {
        "resourceType": "Device", "id": "gut-module",
        "meta": {"profile": ["http://hl7.org/fhir/StructureDefinition/Device"]},
        "status": "active", "manufacturer": "Synthetic",
        "deviceName": [{"name": "Gut module", "type": "user-friendly-name"}],
        "type": {"coding": [coding("http://snomed.info/sct", "706689003")]},
        "version": [{"value": Q_VERSION}],
    }


def flag():
    return {
        "resourceType": "Flag", "id": "alarm-1", "status": "active",
        "category": [{"coding": [coding("http://terminology.hl7.org/CodeSystem/flag-category", "clinical")]}],
        "code": {"coding": [coding(CS_OBS, "alarm-item", "Positive alarm item")], "text": "Blood in the stool"},
        "subject": {"reference": "Patient/p1"}, "period": {"start": "2026-10-09T09:30:00+10:00"},
    }


def composition():
    return {
        "resourceType": "Composition", "id": "summary-1", "status": "final",
        "type": {"coding": [coding("http://loinc.org", "34133-9", "Summary of episode note")]},
        "subject": {"reference": "Patient/p1"}, "date": "2026-10-09T10:00:00+10:00",
        "author": [{"reference": "Device/gut-module"}], "title": "Gut module clinician summary",
        "section": [
            {"title": "Triage", "entry": [{"reference": "Observation/score-1"}, {"reference": "Observation/severity-1"}]},
            {"title": "Alarm items", "entry": [{"reference": "Flag/alarm-1"}]},
        ],
    }


def service_request():
    return {
        "resourceType": "ServiceRequest", "id": "referral-1", "status": "active", "intent": "proposal",
        "priority": "urgent",
        "category": [{"coding": [coding("http://snomed.info/sct", "306206005")]}],
        "code": {"text": "Clinical review of positive alarm item"},
        "subject": {"reference": "Patient/p1"}, "authoredOn": "2026-10-09T10:00:00+10:00",
        # the reason is the alarm *observation*: `reasonReference` may point at a
        # Condition, Observation, DiagnosticReport or DocumentReference — never a
        # Flag (the first cut did, and the oracle refused the document)
        "reasonReference": [{"reference": "Observation/alarm-1"}],
        "supportingInfo": [{"reference": "QuestionnaireResponse/qr-full"}, {"reference": "Flag/alarm-1"}],
    }


def consent():
    return {
        "resourceType": "Consent", "id": "research-1", "status": "active",
        "scope": {"coding": [coding("http://terminology.hl7.org/CodeSystem/consentscope", "research")]},
        "category": [{"coding": [coding("http://loinc.org", "59284-0")]}],
        "patient": {"reference": "Patient/p1"}, "dateTime": "2026-10-09T09:00:00+10:00",
        "policyRule": {"coding": [coding("http://terminology.hl7.org/CodeSystem/v3-ActCode", "OPTIN")]},
        "provision": {"type": "permit", "purpose": [coding("http://terminology.hl7.org/CodeSystem/v3-ActReason", "HRESCH")]},
    }


def export_bundle(qr, observations):
    import uuid
    entries = [{"fullUrl": f"urn:uuid:{uuid.uuid5(uuid.NAMESPACE_URL, BASE + '/' + r['resourceType'] + '/' + r['id'])}",
                "resource": r} for r in [qr] + observations]
    return {"resourceType": "Bundle", "id": "export-1", "type": "collection", "entry": entries}


def audit_event():
    return {
        "resourceType": "AuditEvent", "id": "export-audit-1",
        "type": coding("http://terminology.hl7.org/CodeSystem/audit-event-type", "rest"),
        "subtype": [coding("http://hl7.org/fhir/restful-interaction", "create")],
        "action": "E", "recorded": "2026-10-09T10:05:00+10:00", "outcome": "0",
        "agent": [{"who": {"reference": "Device/gut-module"}, "requestor": True}],
        "source": {"observer": {"reference": "Device/gut-module"}},
        "entity": [{"what": {"reference": "Bundle/export-1"},
                    "type": coding("http://terminology.hl7.org/CodeSystem/audit-entity-type", "4")}],
    }


def main():
    for d in (DEFS, CASES):
        os.makedirs(d, exist_ok=True)
        for f in os.listdir(d):
            os.remove(os.path.join(d, f))
    defs = [questionnaire(), code_system(), obs_code_system()] + [value_set(k) for k in VS]
    for r in defs:
        json.dump(r, open(os.path.join(DEFS, f"{r['resourceType']}-{r['id']}.json"), "w"), indent=1)
    # the table the pod loads, compiled from the definitions the way a host
    # compiles a vendor's: the CodeSystem and ValueSets as files
    import subprocess
    files = sorted(os.path.join(DEFS, f) for f in os.listdir(DEFS) if f.startswith(("ValueSet-", "CodeSystem-")))
    table = subprocess.run([sys.executable, os.path.join(HERE, "..", "..", "tools", "compile_valueset.py"), *files],
                           capture_output=True, text=True)
    if table.returncode != 0:
        raise SystemExit(f"compile_valueset.py refused the definitions: {table.stderr.strip()}")
    open(os.path.join(OUT, "gut.terminology.txt"), "w").write(table.stdout)
    # the same sets and members as the hand-written table, whatever the order
    def parsed(text):
        sets, current = {}, None
        for line in text.splitlines()[1:]:
            if line.startswith("= "):
                current = line[2:]
                sets[current] = set()
            elif line.strip():
                sets[current].add(tuple(line.split("\t")[:2]))
        return sets
    assert parsed(table.stdout) == parsed(terminology_table()), "the compiled table differs from the hand-written one"

    full = response_full()
    cases = []

    def add(name, resource, expect, why, needs_q=True):
        json.dump(resource, open(os.path.join(CASES, f"{name}.json"), "w"), indent=1)
        cases.append({"file": f"{name}.json", "expect": expect, "why": why, "questionnaire": needs_q})

    # the member, the instrument itself, and the responses
    add("patient", patient(), True, "the member record the chain hangs off", False)
    add("device", device(), True, "the module as the author of what it produces", False)
    add("questionnaire", questionnaire(), True, "the instrument validates as a base R4 Questionnaire", False)
    add("qr-full", full, True, "a complete response, every part answered as the instrument allows")
    add("qr-minimal", response_minimal(), True, "required parts only; disabled and optional groups absent")
    add("qr-male", response_full(sex="male"), True, "hormonal part absent where enableWhen disables it")
    add("qr-alarm", response_full(positive="blood-in-stool"), True, "a positive alarm item is a valid answer")
    add("qr-in-progress", response_full(status="in-progress"), True, "an in-progress response")

    # invalid responses, one defect each
    r = response_full()
    r["item"][0]["item"] = [i for i in r["item"][0]["item"] if i["linkId"] != "p1.age"]
    add("qr-missing-required", r, False, "a required item (p1.age) unanswered")
    r = response_full(sex="male")
    r["item"].insert(12, qg("p12", [qi("p12.cycleday", {"valueInteger": 3})]))
    add("qr-disabled-answered", r, False, "the hormonal part answered though enableWhen disables it")
    r = response_full()
    r["item"][0]["item"][1] = qi("p1.age", {"valueString": "forty-one"})
    add("qr-wrong-type", r, False, "a string answer to an integer item")
    r = response_full()
    r["item"][1]["item"][5] = qi("p2.q6", {"valueCoding": coding(CS_ITEMS, "type-9", "Bristol type 9")})
    add("qr-answer-not-in-set", r, False, "a coded answer outside the item's value set")
    r = response_full()
    r["item"][1]["item"][3] = qi("p2.q4", {"valueInteger": 6}, {"valueInteger": 7})
    add("qr-repeats-violated", r, False, "two answers to a non-repeating item")
    r = response_full()
    r["item"][1]["item"].append(qi("p2.q99", {"valueInteger": 1}))
    add("qr-unknown-linkid", r, False, "an item the instrument does not define")
    r = response_full()
    r["questionnaire"] = f"{Q_URL}|9.9.9"
    add("qr-unknown-version", r, True, "a version the instrument does not carry: resolved by canonical alone, "
                                      "as validator_cli 6.10.4 resolves it (probed) — a host that pins versions checks this itself")
    r = response_full()
    r["item"][1]["item"][2] = qi("p2.q3", {"valueDecimal": 18.5})
    add("qr-decimal-for-integer", r, False, "a decimal answer to an integer item")
    r = response_full()
    del r["status"]
    add("qr-no-status", r, False, "status is required")
    r = response_full()
    r["item"][13]["item"][1] = qi("p14.date", {"valueDate": "2021-13"})
    add("qr-bad-date", r, False, "a date answer that is not a date")
    r = response_full()
    r["item"][1]["item"][0] = qi("p2.q1", {"valueCoding": coding(CS_ITEMS, "2")})
    add("qr-coding-no-display", r, True, "a coded answer without a display is still the code")
    r = response_full()
    r["item"][1]["item"][0] = qi("p2.q1", {"valueCoding": {"display": "Level 2"}})
    add("qr-coding-no-code", r, False, "a coded answer with a display and no code")
    r = response_full()
    r["item"][15] = qg("p16", r["item"][15]["item"][:10])
    add("qr-safety-item-missing", r, False, "one of the eleven required alarm items unanswered")

    # the derived chain
    score = observation("score-1", "triage-score", {"valueInteger": 19})
    stool = observation("stool-1", "stool-form", {"valueCodeableConcept": {"coding": [coding(CS_ITEMS, "type-6", "Bristol type 6")]}})
    severity = observation("severity-1", "symptom-severity", {"valueInteger": 6})
    alarm = observation("alarm-1", "alarm-item", {"valueBoolean": True})
    alarm["code"]["text"] = "Blood in the stool"
    for o in (score, stool, severity, alarm):
        add(f"obs-{o['id']}", o, True, "a derived observation", False)
    bad = observation("score-bad", "triage-score", {"valueInteger": 19})
    bad["status"] = "scored"
    add("obs-bad-status", bad, False, "an observation status outside the required set", False)
    bad = observation("score-nocode", "triage-score", {"valueInteger": 19})
    bad["code"] = {"text": "score"}
    add("obs-code-text-only", bad, True, "Observation.code is an example binding; text alone is allowed", False)
    add("flag-alarm", flag(), True, "an alarm flag", False)
    add("composition-summary", composition(), True, "a clinician summary", False)
    add("servicerequest-referral", service_request(), True, "an urgent referral", False)
    add("consent-research", consent(), True, "research-export consent", False)
    add("bundle-export", export_bundle(full, [score, stool, severity]), True, "the research export collection")
    add("auditevent-export", audit_event(), True, "the export audit record", False)
    json.dump(cases, open(os.path.join(CASES, "manifest.json"), "w"), indent=1)
    print(f"wrote {len(defs)} definitions, {len(cases)} cases")


if __name__ == "__main__":
    main()
