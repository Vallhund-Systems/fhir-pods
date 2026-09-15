#!/usr/bin/env python3
"""Emit the conformance corpus.

Valid fixtures are the AU Core package's own example instances — published,
publisher-validated against each profile, the best possible "valid" source —
fetched from packages.fhir.org at a pinned version into a gitignored cache.
The corpus is *derived*, never committed: the package (and the SNOMED display
terms its examples carry) never lands in the repo, per the licence rule.

Broken fixtures are deterministic mutations of those examples plus a few
hand-written cases. Every fixture declares `meta.profile`, so profile
selection is automatic for both our validator and the oracle.
"""
import copy
import glob
import io
import json
import os
import sys
import tarfile
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
CORPUS = os.path.join(HERE, "corpus")
CACHE = os.path.join(HERE, "package")
PKG_ID, PKG_VER = "hl7.fhir.au.core", "2.0.0"
PKG_URL = f"https://packages.fhir.org/{PKG_ID}/{PKG_VER}"
AU = "http://hl7.org.au/fhir/core/StructureDefinition/"
HR, PT = AU + "au-core-heartrate", AU + "au-core-patient"


def ensure_package():
    """The unpacked package, fetching it once."""
    examples = os.path.join(CACHE, "package", "example")
    if os.path.isdir(examples):
        return examples
    os.makedirs(CACHE, exist_ok=True)
    print(f"==> fetching {PKG_ID}#{PKG_VER}")
    data = urllib.request.urlopen(PKG_URL).read()
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as t:
        try:
            t.extractall(CACHE, filter="data")
        except TypeError:  # Python < 3.12
            t.extractall(CACHE)
    return examples


def au_profile(d):
    for p in (d.get("meta") or {}).get("profile") or []:
        if p.startswith(AU):
            return p
    return None


def mutate(base, fn):
    d = copy.deepcopy(base)
    fn(d)
    return d


# --- hand-written cases (no display terms; safe to keep inline) ---

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
    # A realistic AU medical record number: the reference validator rejects
    # example.org URLs by policy, and an MRN needs type + assigner.
    "identifier": [{
        "type": {"coding": [{"system": "http://terminology.hl7.org/CodeSystem/v2-0203", "code": "MR"}], "text": "Medical Record Number"},
        "system": "urn:oid:1.2.36.146.595.217.0.1",
        "value": "12345",
        "assigner": {"display": "Royal Example Hospital"},
    }],
    "name": [{"family": "Doe", "given": ["Jane"], "use": "official"}],
    "gender": "female",
    "birthDate": "1970-01-01",
}


def swap_value_to_string(d):
    d.pop("valueQuantity")
    d["valueString"] = "72"


HAND_WRITTEN = [
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


# --- targeted mutations of IG examples: (example file, fixture name, mutation) ---

def drop(key):
    return lambda d: d.pop(key, None)


def drop_diastolic(d):
    """Remove the diastolic component (LOINC 8462-4) — probes nested component slicing."""
    d["component"] = [
        c for c in d.get("component", [])
        if not any(x.get("code") == "8462-4" for x in (c.get("code") or {}).get("coding", []))
    ]


def bad_weight_unit(d):
    """A unit code outside the bound body-weight unit set — probes bundled value sets."""
    d["valueQuantity"]["code"] = "kilos"


def drop_medication(d):
    """Remove medication[x] — the closed type-slicing on MedicationRequest."""
    for k in [k for k in d if k.startswith("medication")]:
        d.pop(k)


MUTATIONS = [
    ("Observation-bloodpressure-1.json", "bloodpressure-missing-diastolic", drop_diastolic),
    ("Observation-bodyweight-1.json", "bodyweight-bad-unit-code", bad_weight_unit),
    ("Condition-uti.json", "condition-missing-code", drop("code")),
    ("AllergyIntolerance-penicillin.json", "allergy-missing-code", drop("code")),
    ("Encounter-telehealth.json", "encounter-missing-class", drop("class")),
    ("MedicationRequest-simvastatin.json", "medicationrequest-missing-medication", drop_medication),
]


def main():
    examples_dir = ensure_package()
    os.makedirs(CORPUS, exist_ok=True)
    for f in glob.glob(os.path.join(CORPUS, "*.json")):  # derived: rebuild clean
        os.remove(f)
    manifest = []

    def emit(name, resource, profile, expect):
        fname = f"{name}.json"
        with open(os.path.join(CORPUS, fname), "w") as fh:
            json.dump(resource, fh, indent=2)
            fh.write("\n")
        manifest.append({"file": fname, "profile": profile, "expect": expect})

    for name, profile, expect, resource in HAND_WRITTEN:
        emit(name, resource, profile, expect)

    examples = {}
    for path in sorted(glob.glob(os.path.join(examples_dir, "*.json"))):
        d = json.load(open(path))
        profile = au_profile(d)
        if not profile:
            continue
        examples[os.path.basename(path)] = d
        emit("example-" + os.path.basename(path)[:-5], d, profile, True)

    for src, name, fn in MUTATIONS:
        base = examples.get(src)
        if base is None:
            print(f"!! no example {src}", file=sys.stderr)
            continue
        emit(name, mutate(base, fn), au_profile(base), False)

    with open(os.path.join(CORPUS, "manifest.json"), "w") as fh:
        json.dump(manifest, fh, indent=2)
        fh.write("\n")
    print(f"wrote {len(manifest)} fixtures + manifest.json -> {CORPUS}")


if __name__ == "__main__":
    main()
