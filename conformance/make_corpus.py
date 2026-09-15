#!/usr/bin/env python3
"""Emit the conformance corpus: valid AU Core instances plus deliberately broken
mutations of them, and a manifest recording each file's profile URL and the
expected verdict. Deterministic, so the oracle's results are reproducible."""
import copy
import json
import os

CORPUS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "corpus")
HR = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate"
PT = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"

HEARTRATE = {
    "resourceType": "Observation",
    "meta": {"profile": [HR]},
    "status": "final",
    "category": [{"coding": [{"system": "http://terminology.hl7.org/CodeSystem/observation-category", "code": "vital-signs"}]}],
    "code": {"coding": [
        {"system": "http://loinc.org", "code": "8867-4"},
        {"system": "http://snomed.info/sct", "code": "364075005"},
    ]},
    "subject": {"reference": "Patient/1"},
    "effectiveDateTime": "2026-09-15T10:00:00Z",
    "valueQuantity": {"value": 72, "unit": "beats/minute", "system": "http://unitsofmeasure.org", "code": "/min"},
}

PATIENT = {
    "resourceType": "Patient",
    "meta": {"profile": [PT]},
    "identifier": [{"system": "http://example.org/mrn", "value": "12345"}],
    "name": [{"family": "Doe", "given": ["Jane"], "use": "official"}],
    "gender": "female",
    "birthDate": "1970-01-01",
}


def mutate(base, fn):
    d = copy.deepcopy(base)
    fn(d)
    return d


def swap_value_to_string(d):
    d.pop("valueQuantity")
    d["valueString"] = "72"


CASES = [
    ("heartrate-valid", HR, True, HEARTRATE),
    ("heartrate-missing-snomed", HR, False, mutate(HEARTRATE, lambda d: d["code"]["coding"].pop(1))),
    ("heartrate-bad-ucum-code", HR, False, mutate(HEARTRATE, lambda d: d["valueQuantity"].update(code="min"))),
    ("heartrate-valuestring", HR, False, mutate(HEARTRATE, swap_value_to_string)),
    ("heartrate-missing-status", HR, False, mutate(HEARTRATE, lambda d: d.pop("status"))),
    ("patient-valid", PT, True, PATIENT),
    ("patient-missing-gender", PT, False, mutate(PATIENT, lambda d: d.pop("gender"))),
    ("patient-bad-gender", PT, False, mutate(PATIENT, lambda d: d.update(gender="banana"))),
    ("patient-bad-name-use", PT, False, mutate(PATIENT, lambda d: d["name"][0].update(use="bogus"))),
]


def main():
    os.makedirs(CORPUS, exist_ok=True)
    manifest = []
    for name, profile, expect, resource in CASES:
        fname = f"{name}.json"
        with open(os.path.join(CORPUS, fname), "w") as f:
            json.dump(resource, f, indent=2)
            f.write("\n")
        manifest.append({"file": fname, "profile": profile, "expect": expect})
    with open(os.path.join(CORPUS, "manifest.json"), "w") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")
    print(f"wrote {len(manifest)} fixtures + manifest.json -> {CORPUS}")


if __name__ == "__main__":
    main()
