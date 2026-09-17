#!/usr/bin/env python3
"""Generate only synthetic privacy witnesses; no real patient data."""
import copy
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
POLICY = {"salt": "synthetic-probe-salt-only", "today": "2026-09-16", "base": "safe-harbor"}
cases = []


def add(name, resource, markers, reason, **checks):
    file = name + ".json"
    (HERE / file).write_text(json.dumps(resource, ensure_ascii=False, indent=2) + "\n")
    cases.append({"id": name, "file": file, "markers": markers, "reason": reason, **checks})


name = {"resourceType": "Patient", "id": "p", "extension": [
    {"url": "https://example.org/synthetic-name", "valueHumanName": {"family": "PHI_EXTENSION_NAME"}}]}
add("extension-human-name", name, ["PHI_EXTENSION_NAME"], "HumanName must be handled by its type inside an extension.")
contained = {"resourceType": "Observation", "id": "o", "status": "final", "code": {"text": "Synthetic observation"},
             "subject": {"reference": "#p"}, "contained": [{"resourceType": "Patient", "id": "p", "name": [{"family": "PHI_CONTAINED_NAME"}]}]}
add("contained-patient", contained, ["PHI_CONTAINED_NAME"], "Contained resources must retain local reference integrity while removing names.", contained_reference=True)
annotation = {"resourceType": "Condition", "id": "c", "subject": {"reference": "Patient/p"},
              "note": [{"authorString": "PHI_NOTE_AUTHOR", "text": "PHI_NOTE_TEXT"}]}
add("annotation", annotation, ["PHI_NOTE_AUTHOR", "PHI_NOTE_TEXT"], "Annotation author and body both carry identifiers.")
attachment = {"resourceType": "Patient", "id": "p", "photo": [{"title": "PHI_PHOTO_TITLE", "url": "https://example.org/PHI_PHOTO_URL"}]}
add("attachment", attachment, ["PHI_PHOTO_TITLE", "PHI_PHOTO_URL"], "Removing an Attachment must cover title and URL.")
reference = {"resourceType": "Observation", "id": "o", "status": "final", "code": {"text": "Synthetic observation"},
             "subject": {"reference": "Patient/p", "display": "PHI_REFERENCE_DISPLAY"}}
add("reference-display", reference, ["PHI_REFERENCE_DISPLAY"], "Reference display strings must be removed.")
bundle_resources = copy.deepcopy([name, contained, annotation, attachment, reference])
bundle_resources[3]["id"] = "p-photo"
bundle_resources[4]["id"] = "o-display"
bundle = {"resourceType": "Bundle", "type": "collection", "entry": [
    {"fullUrl": f"https://example.org/fhir/{r['resourceType']}/{r['id']}", "resource": r}
    for r in bundle_resources]}
add("bundle-nesting", bundle, ["PHI_EXTENSION_NAME", "PHI_CONTAINED_NAME", "PHI_NOTE_AUTHOR", "PHI_NOTE_TEXT", "PHI_PHOTO_TITLE", "PHI_PHOTO_URL", "PHI_REFERENCE_DISPLAY"],
    "The same type policies must work through Bundle.entry.resource.")
for order in ("system-first", "value-first"):
    identifier = {"system": "https://example.org/ids", "value": "患者識別子Ω"}
    if order == "value-first":
        identifier = dict(reversed(list(identifier.items())))
    add("unicode-identifier-" + order, {"resourceType": "Patient", "identifier": [identifier]}, ["患者識別子Ω"],
        "F008/F014: Unicode identifiers must change, independently of JSON member order.", same_identifier_group="unicode")
for age, expected in ((100, 100), (1100, 1080)):
    add(f"age-{age}-months", {"resourceType": "Condition", "subject": {"reference": "Patient/p"},
        "onsetAge": {"value": age, "system": "http://unitsofmeasure.org", "code": "mo"}}, [],
        "F015: clamp only ages above 89 years, in the original unit; record the change.", age_value=expected, clamp_recorded=age != expected)
add("old-birth-date", {"resourceType": "Patient", "birthDate": "1910-06-12"}, ["1910", "06-12"],
    "Birth years identifying someone over 89 must be aggregated.")
add("extension-free-text", {"resourceType": "Patient", "extension": [
    {"url": "https://example.org/synthetic-patient-note", "valueString": "PHI_GENERIC_EXTENSION"}]}, ["PHI_GENERIC_EXTENSION"],
    "A valid extension can contain an identifier in a generic string; test beyond the built-in type checker.")
add("codeable-concept-text", {"resourceType": "Observation", "status": "final", "code": {"text": "Synthetic measurement for PHI_CODE_TEXT"}},
    ["PHI_CODE_TEXT"], "CodeableConcept.text is free text and may contain an identifier.")
add("retention-control", {"resourceType": "Patient", "name": [{"family": "PHI_CONTROL_NAME"}]}, ["PHI_CONTROL_NAME"],
    "Positive control: no base policy deliberately retains the name; both leak detectors must notice.",
    policy={**POLICY, "base": "none"}, retain_control=True)
(HERE / "manifest.json").write_text(json.dumps({"policy": POLICY, "cases": cases}, ensure_ascii=False, indent=2) + "\n")
