#!/usr/bin/env python3
"""Write the coded-answer probes: every shape a coded answer (and a Coding
outside a questionnaire) can take with a piece missing, under every way a
Questionnaire can name its options. Synthetic; run once, commit the output,
then measure with `run.py`. The expectations in `manifest.json` are the
reference validator's measured verdicts, written by `run.py --oracle-only
--write-expectations` and reviewed, never reasoned.
"""
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFS = HERE / "defs"
BASE = "https://example.org/probe"
CS = f"{BASE}/CodeSystem/answers"
OTHER = f"{BASE}/CodeSystem/other"
VS = f"{BASE}/ValueSet/answers"
VS_ENUM = f"{BASE}/ValueSet/answers-enumerated"
VS_UNVERSIONED = f"{BASE}/ValueSet/answers-unversioned"
CONCEPTS = [("yes", "Yes"), ("no", "No"), ("unsure", "Unsure")]


def write(path, resource):
    path.write_text(json.dumps(resource, indent=2) + "\n")


def contained_vs(label="name", whole_system=False):
    include = {"system": CS} if whole_system else {"system": CS, "concept": [{"code": c, "display": d} for c, d in CONCEPTS]}
    vs = {"resourceType": "ValueSet", "id": "vs", "status": "active", "compose": {"include": [include]}}
    if label in ("name", "url", "title", "name-title"):
        vs["url"] = f"{BASE}/ValueSet/contained"
    if label in ("name", "name-title"):
        vs["name"] = "ContainedAnswers"
    if label in ("title", "name-title"):
        vs["title"] = "Contained answers"
    return vs


def item(shape, required):
    it = {"linkId": "a", "text": "Answer?", "type": "choice", "required": required}
    if shape == "options":
        it["answerOption"] = [{"valueCoding": {"system": CS, "code": c, "display": d}} for c, d in CONCEPTS]
    elif shape == "canonical-vs":
        it["answerValueSet"] = VS
    elif shape == "canonical-enumerated-vs":
        it["answerValueSet"] = VS_ENUM
    elif shape == "canonical-unversioned-vs":
        it["answerValueSet"] = VS_UNVERSIONED
    else:
        it["answerValueSet"] = "#vs"
    return it


def questionnaire(shape, required, label):
    q = {"resourceType": "Questionnaire", "id": "q", "status": "active", "item": [item(shape, required)]}
    if shape == "contained-vs":
        q["contained"] = [contained_vs(label)]
    return q


def loaded_url(required, label):
    """A Questionnaire with a contained ValueSet cannot itself be contained
    (dom-2), so that shape is given by canonical: loaded into both engines."""
    return f"{BASE}/Questionnaire/{'required' if required else 'optional'}-{label}"


ANSWERS = {
    "in-set": {"valueCoding": {"system": CS, "code": "yes", "display": "Yes"}},
    "not-in-set": {"valueCoding": {"system": CS, "code": "maybe"}},
    "other-system": {"valueCoding": {"system": OTHER, "code": "yes"}},
    "code-only-member": {"valueCoding": {"code": "yes", "display": "Yes"}},
    "code-only-nonmember": {"valueCoding": {"code": "maybe"}},
    "code-only-member-bare": {"valueCoding": {"code": "yes"}},
    "display-only": {"valueCoding": {"display": "Yes"}},
    "system-only": {"valueCoding": {"system": CS}},
    "system-display": {"valueCoding": {"system": CS, "display": "Yes"}},
    "string": {"valueString": "yes"},
}
OPTIONAL = ["code-only-member", "display-only", "system-only"]
SHAPES = ["contained-vs", "canonical-vs", "options", "qr-contained-vs"]
LOADED = [(True, "name"), (True, "title"), (True, "url"), (True, "none"), (False, "name")]


def response(shape, required, answer, label="name"):
    qr = {"resourceType": "QuestionnaireResponse", "id": "r", "status": "completed",
          "item": [{"linkId": "a", "answer": [ANSWERS[answer]]}]}
    if shape == "contained-vs":
        qr["questionnaire"] = loaded_url(required, label)
    else:
        qr["questionnaire"] = "#q"
        contained = [questionnaire(shape, required, label)]
        if shape in ("qr-contained-vs", "qr-contained-system-vs"):
            contained.append(contained_vs(label, whole_system=shape == "qr-contained-system-vs"))
        qr["contained"] = contained
    return qr


def defs():
    write(DEFS / "CodeSystem-answers.json", {
        "resourceType": "CodeSystem", "id": "answers", "url": CS, "version": "1.0.0", "name": "ProbeAnswers",
        "status": "active", "content": "complete", "concept": [{"code": c, "display": d} for c, d in CONCEPTS]})
    write(DEFS / "ValueSet-answers.json", {
        "resourceType": "ValueSet", "id": "answers", "url": VS, "version": "1.0.0", "name": "ProbeAnswers",
        "status": "active", "compose": {"include": [{"system": CS}]}})
    write(DEFS / "ValueSet-answers-unversioned.json", {
        "resourceType": "ValueSet", "id": "answers-unversioned", "url": VS_UNVERSIONED,
        "name": "ProbeAnswersUnversioned", "status": "active", "compose": {"include": [{"system": CS}]}})
    write(DEFS / "ValueSet-answers-enumerated.json", {
        "resourceType": "ValueSet", "id": "answers-enumerated", "url": VS_ENUM, "version": "1.0.0",
        "name": "ProbeAnswersEnumerated", "status": "active",
        "compose": {"include": [{"system": CS, "concept": [{"code": c, "display": d} for c, d in CONCEPTS]}]}})
    for required, label in LOADED:
        q = questionnaire("contained-vs", required, label)
        url = loaded_url(required, label)
        name = url.rsplit("/", 1)[1]
        q.update({"id": name, "url": url, "version": "1.0.0"})
        write(DEFS / f"Questionnaire-{name}.json", q)


def outside():
    """The same shapes on a Coding outside a questionnaire, where the rule
    path would be the general binding check."""
    subject = {"reference": "Patient/p"}
    clinical = "http://terminology.hl7.org/CodeSystem/condition-clinical"
    out = {}
    for name, coding in {
        "coded": {"system": clinical, "code": "active"},
        "code-only-member": {"code": "active"},
        "code-only-nonmember": {"code": "bogus"},
        "display-only": {"display": "Active"},
    }.items():
        out[f"outside-required-concept-{name}"] = {
            "resourceType": "Condition", "id": "c", "clinicalStatus": {"coding": [coding]},
            "code": {"text": "Probe"}, "subject": subject}
    for name, coding in {
        "code-only-member": {"code": "AMB"},
        "code-only-nonmember": {"code": "bogus"},
    }.items():
        out[f"outside-required-coding-{name}"] = {
            "resourceType": "Encounter", "id": "e", "status": "finished", "class": coding, "subject": subject}
    out["outside-example-concept-code-only"] = {
        "resourceType": "Observation", "id": "o", "status": "final", "code": {"coding": [{"code": "1234-5"}]},
        "subject": subject}
    out["outside-unbound-concept-code-only"] = {
        "resourceType": "Observation", "id": "o", "status": "final", "code": {"text": "Probe"},
        "subject": subject, "valueCodeableConcept": {"coding": [{"code": "yes"}]}}
    return out


ANSWER_WORDS = {
    "in-set": "system and code, the code in the set (control)",
    "not-in-set": "system and code, the code not in the code system or the set",
    "other-system": "a code from a system the set does not include",
    "code-only-member-bare": "a code with no system (no display), the code in the set",
    "code-only-member": "a code with no system, the code in the set",
    "code-only-nonmember": "a code with no system, the code not in the set",
    "display-only": "a display alone: no code, no system",
    "system-only": "a system alone: no code",
    "system-display": "a system and a display: no code",
    "string": "a valueString on a choice item",
    "coded": "system and code (control)",
}
SHAPE_WORDS = {
    "contained-vs": "the answerValueSet contained in the Questionnaire, loaded by canonical (dom-2 forbids a contained Questionnaire containing one)",
    "canonical-vs": "the answerValueSet by canonical, a set including the whole code system",
    "canonical-unversioned-vs": "the answerValueSet by canonical, a set including the whole code system, no version",
    "canonical-enumerated-vs": "the answerValueSet by canonical, a set enumerating its codes",
    "options": "inline answerOption codings",
    "qr-contained-system-vs": "the answerValueSet contained beside the Questionnaire in the response, including the whole code system",
    "qr-contained-vs": "the answerValueSet contained beside the Questionnaire in the response (fhir-test-cases' nested-questionnaire-nested-valueset shape)",
}


def describe(name):
    """What the probe is, in words: the shape, the answer, the label."""
    if name.startswith("outside-"):
        where = {
            "outside-required-concept": "Condition.clinicalStatus, a required CodeableConcept binding",
            "outside-required-coding": "Encounter.class, a Coding (extensible binding in R4)",
            "outside-example-concept": "Observation.code, an example binding",
            "outside-unbound-concept": "Observation.valueCodeableConcept, no binding",
        }
        prefix = next(p for p in where if name.startswith(p))
        rest = name[len(prefix) + 1:]
        answer = ANSWER_WORDS.get(rest, "a code with no system")
        return f"Outside a questionnaire — {where[prefix]}: {answer}."
    shape = next(s for s in sorted(SHAPE_WORDS, key=len, reverse=True) if name.startswith(s + "-"))
    rest = name[len(shape) + 1:]
    label = None
    if "-label-" in rest:
        label, rest = rest.split("-label-")
        required = "required"
    else:
        required, rest = rest.split("-", 1)
    text = f"{SHAPE_WORDS[shape]}; {required} item; {ANSWER_WORDS[rest]}"
    if label:
        text += f"; the contained set labelled by {label.replace('-', ' and ')} (what the message calls it)"
    return text[0].upper() + text[1:] + "."


def main():
    DEFS.mkdir(exist_ok=True)
    defs()
    cases = {}
    for shape in SHAPES:
        for answer in ANSWERS:
            cases[f"{shape}-required-{answer}"] = response(shape, True, answer)
        for answer in OPTIONAL:
            cases[f"{shape}-optional-{answer}"] = response(shape, False, answer)
    # whether the set enumerates its codes or includes a whole system
    for shape in ("canonical-enumerated-vs", "qr-contained-system-vs", "canonical-unversioned-vs"):
        for answer in ("in-set", "not-in-set", "code-only-member", "code-only-nonmember", "display-only", "system-only"):
            cases[f"{shape}-required-{answer}"] = response(shape, True, answer)
    # what the message calls a contained set
    cases["qr-contained-vs-name-title-label-not-in-set"] = response("qr-contained-vs", True, "not-in-set", "name-title")
    for label in ("title", "url", "none"):
        for answer in ("not-in-set", "code-only-member"):
            cases[f"contained-vs-{label}-label-{answer}"] = response("contained-vs", True, answer, label)
            cases[f"qr-contained-vs-{label}-label-{answer}"] = response("qr-contained-vs", True, answer, label)
    cases.update(outside())
    for name, resource in cases.items():
        write(HERE / f"{name}.json", resource)
    manifest_path = HERE / "manifest.json"
    old = {m["file"]: m for m in json.loads(manifest_path.read_text())} if manifest_path.exists() else {}
    manifest = []
    for n in cases:
        entry = old.get(f"{n}.json", {"file": f"{n}.json", "expect": None})
        entry["reason"] = describe(n)
        manifest.append(entry)
    write(manifest_path, manifest)
    print(f"{len(cases)} probes")


if __name__ == "__main__":
    main()
