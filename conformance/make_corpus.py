#!/usr/bin/env python3
"""Emit the conformance corpus.

Valid fixtures are each IG package's own example instances (AU Core, US Core,
IPA)
— published, publisher-validated against each profile, the best possible
"valid" source — fetched from packages.fhir.org at pinned versions into
gitignored caches.
The corpus is *derived*, never committed: the package (and the SNOMED display
terms its examples carry) never lands in the repo, per the licence rule.

Broken fixtures are deterministic mutations of those examples plus a few
hand-written cases. Every fixture declares `meta.profile`, so profile
selection is automatic for both our validator and the oracle.
"""
import copy
import fnmatch
import glob
import io
import json
import os
import sys
import tarfile
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
CORPUS = os.path.join(HERE, "corpus")
AU = "http://hl7.org.au/fhir/core/StructureDefinition/"
US = "http://hl7.org/fhir/us/core/StructureDefinition/"
HR, PT = AU + "au-core-heartrate", AU + "au-core-patient"

# The IGs under test: (package id, version, canonical prefix, cache dir,
# fixture-name prefix). Each package's own examples are its valid fixtures.
# The oracle (run.py) is handed the same list.
IPA = "http://hl7.org/fhir/uv/ipa/StructureDefinition/"
IPS = "http://hl7.org/fhir/uv/ips/StructureDefinition/"
AUPS = "http://hl7.org.au/fhir/ps/StructureDefinition/"
BALP = "https://profiles.ihe.net/ITI/BALP/StructureDefinition/"
IGS = [
    ("hl7.fhir.au.core", "2.0.0", AU, "package", "example-"),
    ("hl7.fhir.us.core", "6.1.0", US, "package-us", "us-"),
    ("hl7.fhir.uv.ipa", "1.1.0", IPA, "package-ipa", "ipa-"),
    ("hl7.fhir.uv.ips", "2.0.1", IPS, "package-ips", "ips-"),
    ("hl7.fhir.au.ps", "1.0.0", AUPS, "package-au-ps", "aups-"),
    ("ihe.iti.balp", "1.1.4", BALP, "package-balp", "balp-"),
]

# Dependency packages whose datatype profiles slices inherit (no examples
# taken from them): fetched into the same gitignored caches so the generator
# (scripts/gen-profiles.sh) never depends on another tool's package cache.
DEPS = [
    ("hl7.fhir.au.base", "6.0.0", "package-au-base"),
    ("hl7.fhir.uv.extensions.r4", "5.3.0", "package-ext"),   # extensions IPS / AU PS slices inherit
    ("hl7.fhir.r4.core", "4.0.1", "package-r4"),   # the element-type table (tools/gen_typemap.py)
]


def ensure_package(pkg_id, pkg_ver, cache_dir):
    """The unpacked package's example dir, fetching the package once."""
    cache = os.path.join(HERE, cache_dir)
    examples = os.path.join(cache, "package", "example")
    if os.path.isdir(os.path.join(cache, "package")):
        return examples
    os.makedirs(cache, exist_ok=True)
    print(f"==> fetching {pkg_id}#{pkg_ver}")
    data = urllib.request.urlopen(f"https://packages.fhir.org/{pkg_id}/{pkg_ver}").read()
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as t:
        try:
            t.extractall(cache, filter="data")
        except TypeError:  # Python < 3.12
            t.extractall(cache)
    return examples


def profile_of(d, prefix):
    for p in (d.get("meta") or {}).get("profile") or []:
        if p.startswith(prefix):
            return p
    return None


def au_profile(d):
    return profile_of(d, AU)


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


UNPROFILED_OBSERVATION = {
    "resourceType": "Observation", "status": "final", "code": {"text": "x"}, "valueString": "y",
}

AU_PATIENT = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"

def user_agent(d):
    return next(a for a in d["agent"] if any(c.get("code") == "IRCP" for c in a.get("type", {}).get("coding", [])))


BALP_MUTATIONS = [
    ("AuditEvent-ex-auditBasicReadClient.json", "balp-user-requestor-false", lambda d: user_agent(d).__setitem__("requestor", False), False),
    ("AuditEvent-ex-auditBasicReadClient.json", "balp-user-requestor-missing", lambda d: user_agent(d).pop("requestor"), False),
    ("AuditEvent-ex-auditBasicReadClient.json", "balp-user-requestor-true", lambda d: user_agent(d).__setitem__("requestor", True), True),
]

HAND_WRITTEN = [
    # not a FHIR resource type at all (F002)
    ("not-a-resource", None, False, {"resourceType": "NotAResource"}),
    # No declared profile: the base resource applies (which we do not compile) — valid, and no profile claimed.
    ("observation-unprofiled", None, True, UNPROFILED_OBSERVATION),
    # the v0.5.1 review's findings (F010–F013) and the base-resource rules an
    # unprofiled resource still owes
    ("bundle-nested-profile-invalid", None, False, {
        "resourceType": "Bundle", "type": "collection",
        "entry": [{"resource": {"resourceType": "Basic", "code": {"text": "x"}}},
                  {"resource": {"resourceType": "Patient", "meta": {"profile": [AU_PATIENT]},
                                "name": [{"family": "x"}], "gender": "male", "birthDate": "1970"}}],
    }),
    ("contained-profile-invalid", None, False, {
        "resourceType": "Encounter", "status": "finished",
        "class": {"system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "AMB"},
        "contained": [{"resourceType": "Patient", "id": "x", "meta": {"profile": [AU_PATIENT]},
                       "name": [{"family": "x"}], "gender": "male", "birthDate": "1970"}],
        "subject": {"reference": "#x"},
    }),
    ("patient-declares-observation-profile", None, False, {
        "resourceType": "Patient", "meta": {"profile": [HR]}, "gender": "male",
    }),
    ("datetime-without-seconds", None, False, mutate(UNPROFILED_OBSERVATION, lambda d: d.__setitem__("effectiveDateTime", "2026-09-16T10:00+10:00"))),
    ("datetime-offset-beyond-14", None, False, mutate(UNPROFILED_OBSERVATION, lambda d: d.__setitem__("effectiveDateTime", "2026-09-16T10:00:00+15:00"))),
    ("datetime-year-zero", None, False, mutate(UNPROFILED_OBSERVATION, lambda d: d.__setitem__("effectiveDateTime", "0000-09-16"))),
    ("time-with-zone", None, False, mutate(UNPROFILED_OBSERVATION, lambda d: d.__setitem__("valueTime", "10:00:00Z"))),
    ("instant-without-date", None, False, mutate(UNPROFILED_OBSERVATION, lambda d: d.__setitem__("issued", "10:00:00Z"))),
    ("extension-value-via-carrier", None, True, {
        "resourceType": "Patient", "gender": "male",
        "extension": [{"url": "http://example.org/x", "_valueString": {"extension": [
            {"url": "http://hl7.org/fhir/StructureDefinition/data-absent-reason", "valueCode": "unknown"}]}}],
    }),
    ("unprofiled-encounter-missing-class", None, False, {"resourceType": "Encounter", "status": "finished"}),
    ("unprofiled-two-choice-keys", None, False, mutate(UNPROFILED_OBSERVATION, lambda d: d.__setitem__("valueInteger", 1))),
    ("unprofiled-unreferenced-contained", None, False, {
        "resourceType": "Encounter", "status": "finished",
        "class": {"system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "AMB"},
        "contained": [{"resourceType": "Patient", "id": "p"}],
    }),
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


def set_field(key, value):
    return lambda d: d.__setitem__(key, value)


def dangling_reference(d):
    """A document whose Condition points at a Patient that is not in the
    Bundle — a document must be self-contained."""
    for e in d.get("entry", []):
        r = e.get("resource") or {}
        if r.get("resourceType") == "Condition":
            r["subject"] = {"reference": "Patient/not-in-this-document"}
            return


def birthdate_accuracy_wrong_type(d):
    """The date-accuracy extension on `_birthDate` with a valueString — the
    extension profile allows only valueCoding. Probes that extensions carried
    on a primitive are validated at all."""
    d["_birthDate"] = {"extension": [
        {"url": "http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator", "valueString": "AAA"},
    ]}


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


# --- probes: each aimed at one feature the engine still defers ---

def wrong_diastolic_code(d):
    """Keep both components but corrupt the diastolic LOINC code — the
    `component min 2` cardinality still holds, so only nested component
    slicing can catch this."""
    for c in d.get("component", []):
        for x in (c.get("code") or {}).get("coding", []):
            if x.get("code") == "8462-4":
                x["code"] = "8462-9"


def add_bad_luhn_ihi(d):
    """Append an IHI whose check digit fails Luhn — the au-ihi profile's
    FHIRPath invariant (inv-ihi-value-2)."""
    d.setdefault("identifier", []).append({
        "type": {"coding": [{"system": "http://terminology.hl7.org/CodeSystem/v2-0203", "code": "NI"}]},
        "system": "http://ns.electronichealth.net.au/id/hi/ihi/1.0",
        "value": "8003608833357362",
    })


def split_type_codings(d):
    """An MRN whose `type` carries TWO codings — v2-0203 MR, and a local code
    NI under a different system. Matched by ONE coding, this is not the IHI
    slice (the pattern is one coding carrying both v2-0203 and NI); matched
    leaf-by-leaf it wrongly would be, and au-ihi's system + Luhn would fire.
    Expected valid."""
    for ident in d.get("identifier", []):
        for c in (ident.get("type") or {}).get("coding", []):
            if c.get("code") == "MR":
                ident["type"]["coding"].append({"system": "http://example.org/local", "code": "NI"})


def short_ihi(d):
    """An NI identifier of 15 digits — inv-ihi-0 (`matches('^([0-9]{16})$')`),
    the regex invariant; Luhn alone would not catch it."""
    d.setdefault("identifier", []).append({
        "type": {"coding": [{"system": "http://terminology.hl7.org/CodeSystem/v2-0203", "code": "NI"}]},
        "system": "http://ns.electronichealth.net.au/id/hi/ihi/1.0",
        "value": "800360883335736",
    })


def wrong_system_ihi(d):
    """An identifier typed NI (an IHI) whose system is not the IHI namespace —
    caught by au-ihi's fixed system, reached through the identifier slice.
    Needs no invariant, so this build should flip it."""
    d.setdefault("identifier", []).append({
        "type": {"coding": [{"system": "http://terminology.hl7.org/CodeSystem/v2-0203", "code": "NI"}]},
        "system": "urn:oid:1.2.36.146.595.217.0.1",
        "value": "8003608833357361",
    })


def drop_value_keep_no_dar(d):
    """No value[x] and no dataAbsentReason — the vitals `vs-2` invariant."""
    for k in [k for k in d if k.startswith("value")]:
        d.pop(k)
    d.pop("dataAbsentReason", None)


def subject_wrong_type(d):
    """subject referencing a Device. The reference validator does NOT enforce
    reference target type offline (it reported this valid), so this fixture
    expects VALID — a guard against us over-enforcing what the oracle doesn't."""
    d["subject"] = {"reference": "Device/1"}


# --- US Core probes: one per feature the second IG exercises differently ---

def drop_race_text(d):
    """Remove the required `text` sub-extension of the us-core-race complex
    extension — only reachable through the extension slice's profile
    (us-core-race: extension:text 1..1)."""
    for e in d.get("extension", []):
        if e.get("url", "").endswith("/us-core-race"):
            e["extension"] = [x for x in e.get("extension", []) if x.get("url") != "text"]


def drop_diastolic_us(d):
    """Remove the diastolic component (LOINC 8462-4) — us-core-blood-pressure
    component:diastolic 1..1."""
    d["component"] = [
        c for c in d.get("component", [])
        if not any(x.get("code") == "8462-4" for x in (c.get("code") or {}).get("coding", []))
    ]


def wrong_bp_unit(d):
    """mm[Hg] -> mmHg on the systolic component — a fixed UCUM code inside a
    component slice."""
    for c in d.get("component", []):
        q = c.get("valueQuantity")
        if q and q.get("code") == "mm[Hg]":
            q["code"] = "mmHg"
            break


def wrong_lab_category(d):
    """laboratory -> vital-signs: the us-core-observation-lab category slice
    (pattern on $this) then matches nothing; category:us-core 1..1."""
    for cat in d.get("category", []):
        for x in cat.get("coding", []):
            if x.get("code") == "laboratory":
                x["code"] = "vital-signs"


def drop_value_and_dar(d):
    for k in [k for k in d if k.startswith("value")]:
        d.pop(k)
    d.pop("dataAbsentReason", None)


def add_au_profile(d):
    """Declare a second profile (AU Core Patient) alongside US Core Patient:
    the validator must check both. Expected valid — the example satisfies AU
    Core too; the oracle decides."""
    d["meta"]["profile"].append(PT)


def escaped_slashes(d):
    """Legal JSON, PHP-style: every `/` in the coding systems written as `\\/`
    in the *document text* (see TEXT_MUTATIONS — a value-level edit would
    insert literal backslashes, which is a different, invalid resource).
    Byte-for-byte comparison of undecoded strings would miss the fixed
    systems and the slice discriminators. Expected valid."""


# Fixtures whose mutation is to the JSON text itself, applied after the
# resource is serialised: name -> text -> text.
TEXT_MUTATIONS = {
    "heartrate-escaped-slashes": lambda t: t.replace("http://loinc.org", "http:\\/\\/loinc.org").replace("http://snomed.info/sct", "http:\\/\\/snomed.info\\/sct"),
}


def versioned_profile(d):
    """`meta.profile` with a `|version` suffix — the canonical form. Expected
    valid: the profile must still be recognised (not fall through)."""
    d["meta"]["profile"] = [p + "|6.1.0" for p in d["meta"]["profile"]]


def same_instant_different_forms(d):
    """mdd-1: whenHandedOver >= whenPrepared — the same instant written with
    milliseconds and `Z` vs `+00:00`. Expected valid: a temporal comparison,
    not a string one."""
    d["whenPrepared"] = "2020-01-01T10:00:00+00:00"
    d["whenHandedOver"] = "2020-01-01T10:00:00.000Z"


def orphan_contained(d):
    """Point the medication reference away from the contained Medication, so
    the contained resource is referenced by nothing — dom-3 (needs typed
    traversal: descendants().as(canonical) etc.)."""
    d["medicationReference"] = {"reference": "Medication/elsewhere"}


# (example file glob, fixture name, mutation, expected verdict)
US_MUTATIONS = [
    ("MedicationRequest-medicationrequest-contained-oral-axid.json", "us-medreq-orphan-contained", orphan_contained, False),
    ("Patient-example.json", "us-patient-versioned-profile", versioned_profile, True),
    ("MedicationDispense-*.json", "us-meddispense-same-instant", same_instant_different_forms, True),
    ("Patient-example.json", "us-patient-two-profiles", add_au_profile, True),
    ("Patient-example.json", "us-patient-missing-name", drop("name"), False),
    ("Patient-example.json", "us-patient-bad-gender", lambda d: d.update(gender="banana"), False),
    ("Patient-example.json", "us-patient-race-missing-text", drop_race_text, False),
    ("Observation-blood-pressure.json", "us-bp-missing-diastolic", drop_diastolic_us, False),
    ("Observation-blood-pressure.json", "us-bp-wrong-unit", wrong_bp_unit, False),
    ("Observation-serum-sodium.json", "us-lab-missing-category", drop("category"), False),
    ("Observation-serum-sodium.json", "us-lab-wrong-category", wrong_lab_category, False),
    ("Observation-*smok*.json", "us-smoking-no-value", drop_value_and_dar, False),
    ("Condition-health-concern-example.json", "us-condition-missing-category", drop("category"), False),
    # The oracle reports this VALID (requester is must-support, not required in
    # 6.1.0) — kept as a guard against over-enforcing what the oracle does not.
    ("MedicationRequest-self-tylenol.json", "us-medreq-missing-requester", drop("requester"), True),
    ("Encounter-example-1.json", "us-encounter-missing-type", drop("type"), False),
    ("Immunization-imm-1.json", "us-immunization-missing-vaccinecode", drop("vaccineCode"), False),
    ("AllergyIntolerance-example.json", "us-allergy-missing-patient", drop("patient"), False),
    # the reference validator does not fail a document over a reference that does
    # not resolve inside it (a warning) — kept as a guard against over-enforcing
    ("Bundle-aups-basicsummary.json", "aups-dangling-reference", dangling_reference, True),
]


# (example file, fixture name, mutation, expected verdict)
MUTATIONS = [
    ("Observation-bloodpressure-1.json", "bloodpressure-wrong-diastolic-code", wrong_diastolic_code, False),
    ("Patient-ronny-irvine.json", "patient-ihi-bad-luhn", add_bad_luhn_ihi, False),
    ("Patient-ronny-irvine.json", "patient-ihi-wrong-system", wrong_system_ihi, False),
    ("Patient-ronny-irvine.json", "patient-ihi-15-digits", short_ihi, False),
    ("Patient-ronny-irvine.json", "patient-split-type-codings", split_type_codings, True),
    ("Patient-ronny-irvine.json", "patient-birthdate-accuracy-wrong-type", birthdate_accuracy_wrong_type, False),
    # the base spec's shape rules (the reviewer's F001 probes): JSON kind, unknown
    # element, array-ness, date format
    ("Patient-ronny-irvine.json", "patient-active-not-boolean", set_field("active", "not-a-boolean"), False),
    ("Patient-ronny-irvine.json", "patient-birthdate-number", set_field("birthDate", 123), False),
    ("Patient-ronny-irvine.json", "patient-managingorganization-string", set_field("managingOrganization", "not-an-object"), False),
    ("Patient-ronny-irvine.json", "patient-unknown-element", set_field("surpriseField", {"anything": "goes"}), False),
    ("Patient-ronny-irvine.json", "patient-name-not-an-array", lambda d: d.__setitem__("name", d["name"][0]), False),
    ("Patient-ronny-irvine.json", "patient-gender-array", set_field("gender", ["male"]), False),
    ("Patient-ronny-irvine.json", "patient-birthdate-not-a-date", set_field("birthDate", "not-a-date"), False),
    # an unresolvable declared profile is a warning for the reference validator, not an error
    ("Patient-ronny-irvine.json", "patient-unknown-declared-profile", lambda d: d["meta"]["profile"].append("https://example.org/unsupported-profile"), True),
    ("Observation-heartrate-1.json", "heartrate-escaped-slashes", escaped_slashes, True),
    ("Observation-heartrate-1.json", "heartrate-no-value-no-dar", drop_value_keep_no_dar, False),
    ("Observation-heartrate-1.json", "heartrate-subject-wrong-type", subject_wrong_type, True),
    ("Observation-bloodpressure-1.json", "bloodpressure-missing-diastolic", drop_diastolic, False),
    ("Observation-bodyweight-1.json", "bodyweight-bad-unit-code", bad_weight_unit, False),
    ("Condition-uti.json", "condition-missing-code", drop("code"), False),
    ("AllergyIntolerance-penicillin.json", "allergy-missing-code", drop("code"), False),
    ("Encounter-telehealth.json", "encounter-missing-class", drop("class"), False),
    ("MedicationRequest-simvastatin.json", "medicationrequest-missing-medication", drop_medication, False),
]


def main():
    os.makedirs(CORPUS, exist_ok=True)
    for f in glob.glob(os.path.join(CORPUS, "*.json")):  # derived: rebuild clean
        os.remove(f)
    manifest = []

    def emit(name, resource, profile, expect, ig):
        fname = f"{name}.json"
        text = json.dumps(resource, indent=2) + "\n"
        if name in TEXT_MUTATIONS:
            text = TEXT_MUTATIONS[name](text)
        with open(os.path.join(CORPUS, fname), "w") as fh:
            fh.write(text)
        manifest.append({"file": fname, "profile": profile, "expect": expect, "ig": ig})

    for pkg_id, pkg_ver, cache_dir in DEPS:
        ensure_package(pkg_id, pkg_ver, cache_dir)

    # The probe lane: hand-written fixtures with expected verdicts, each with
    # its reason, kept in conformance/probes/ (see fhir-pods-planning T009).
    probes_dir = os.path.join(HERE, "probes")
    probes_manifest = os.path.join(probes_dir, "manifest.json")
    if os.path.exists(probes_manifest):
        for p in json.load(open(probes_manifest)):
            d = json.load(open(os.path.join(probes_dir, p["file"])))
            emit("probe-" + p["file"][:-5], d, p.get("profile"), bool(p["expect"]), "probes")

    au_ig = f"{IGS[0][0]}#{IGS[0][1]}"
    for name, profile, expect, resource in HAND_WRITTEN:
        emit(name, resource, profile, expect, au_ig)

    # Every IG's own examples that declare one of its profiles: valid by
    # publication. Examples that declare no profile of the IG are left out —
    # neither side would apply the IG to them. A few are outside what a
    # library can check, and are named here with the reason.
    SKIP = {
        # the reference validator verifies Bundle.signature against its certificate;
        # cryptographic verification is not a validation concern the pods take on
        "aups-Bundle-aups-referral-endoconsult-autogen": "Bundle.signature verification is out of scope",
    }
    # Published examples the reference validator itself rejects when run without
    # a terminology server, as the oracle is: IPS's pregnancy-status and
    # pregnancy-outcome profiles differ only by value-set membership, so an
    # Observation matches both slices — "matches more than one slice". Expected
    # invalid here, for the same reason, on both sides.
    INVALID_WITHOUT_TERMINOLOGY = {
        "ips-Bundle-IPS-examples-Bundle-01",
        "ips-Bundle-IPS-examples-Bundle-with-immunization",
        "ips-Bundle-bundle-ips-all-sections",
        "aups-Bundle-aups-referral-endoconsult-curated",
    }
    au_examples, us_examples, balp_examples = {}, {}, {}
    for pkg_id, pkg_ver, prefix, cache_dir, name_prefix in IGS:
        examples_dir = ensure_package(pkg_id, pkg_ver, cache_dir)
        ig = f"{pkg_id}#{pkg_ver}"
        for path in sorted(glob.glob(os.path.join(examples_dir, "*.json"))):
            d = json.load(open(path))
            profile = profile_of(d, prefix)
            if not profile:
                continue
            {AU: au_examples, US: us_examples, BALP: balp_examples}.get(prefix, us_examples)[os.path.basename(path)] = d
            name = name_prefix + os.path.basename(path)[:-5]
            if name in SKIP:
                print(f"   skipping {name}: {SKIP[name]}")
                continue
            emit(name, d, profile, name not in INVALID_WITHOUT_TERMINOLOGY, ig)

    us_ig = f"{IGS[1][0]}#{IGS[1][1]}"
    for pattern, name, fn, expect in US_MUTATIONS:
        src = next((k for k in sorted(us_examples) if fnmatch.fnmatch(k, pattern)), None)
        if src is None:
            print(f"!! no US Core example matching {pattern}", file=sys.stderr)
            continue
        base = us_examples[src]
        emit(name, mutate(base, fn), profile_of(base, US), expect, us_ig)

    # the boolean pattern (agent:user.requestor = true) is a constraint like any other
    balp_ig = f"{IGS[5][0]}#{IGS[5][1]}"
    for src, name, fn, expect in BALP_MUTATIONS:
        base = balp_examples.get(src)
        if base is None:
            print(f"!! no BALP example {src}", file=sys.stderr)
            continue
        emit(name, mutate(base, fn), profile_of(base, BALP), expect, balp_ig)

    for src, name, fn, expect in MUTATIONS:
        base = au_examples.get(src)
        if base is None:
            print(f"!! no example {src}", file=sys.stderr)
            continue
        emit(name, mutate(base, fn), au_profile(base), expect, au_ig)

    with open(os.path.join(CORPUS, "manifest.json"), "w") as fh:
        json.dump(manifest, fh, indent=2)
        fh.write("\n")
    print(f"wrote {len(manifest)} fixtures + manifest.json -> {CORPUS}")


if __name__ == "__main__":
    main()
