#!/usr/bin/env python3
"""Build synthetic assembly inputs covering every section-routing resource kind."""
import copy
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
cases = []
PATIENT = {"resourceType": "Patient", "id": "p", "identifier": [{"system": "https://example.org/ids", "value": "synthetic-patient"}],
           "name": [{"family": "Synthetic", "given": ["Patient"]}], "gender": "female", "birthDate": "1980-04-12"}
AUTHOR = {"resourceType": "Practitioner", "id": "a", "name": [{"family": "Synthetic", "given": ["Author"]}]}
SUBJECT = {"reference": "Patient/p"}
DATE = "2026-09-16T12:00:00Z"


def coding(system, code):
    return {"coding": [{"system": system, "code": code}]}


def inputs(*resources):
    return {"resources": copy.deepcopy([PATIENT, AUTHOR, *resources]), "author": "Practitioner/a", "date": DATE}


def add(name, data, reason, **checks):
    file = name + ".json"
    (HERE / file).write_text(json.dumps(data, indent=2) + "\n")
    cases.append({"id": name, "file": file, "expect_valid": True, "reason": reason, **checks})


add("minimal", inputs(), "Empty required sections use data-absent-reason; supporting patient and author resolve.")
condition = {"resourceType": "Condition", "id": "c", "clinicalStatus": coding("http://terminology.hl7.org/CodeSystem/condition-clinical", "active"),
             "category": [coding("http://terminology.hl7.org/CodeSystem/condition-category", "problem-list-item")],
             "code": {"text": "Synthetic condition"}, "subject": SUBJECT}
add("condition-active", inputs(condition), "Active Condition belongs in problems.", section="11450-4", placed_type="Condition")
past = copy.deepcopy(condition)
past["clinicalStatus"]["coding"][0]["code"] = "resolved"
add("condition-past", inputs(past), "Resolved Condition belongs in history of past problems.", section="11348-0", placed_type="Condition")
resources = [
    ("allergy", "48765-2", {"resourceType": "AllergyIntolerance", "id": "allergy", "clinicalStatus": coding("http://terminology.hl7.org/CodeSystem/allergyintolerance-clinical", "active"), "code": {"text": "Synthetic allergen"}, "patient": SUBJECT}),
    ("medication-statement", "10160-0", {"resourceType": "MedicationStatement", "id": "ms", "status": "active", "medicationCodeableConcept": {"text": "Synthetic medication"}, "subject": SUBJECT, "effectiveDateTime": DATE}),
    ("medication-request", "10160-0", {"resourceType": "MedicationRequest", "id": "mr", "status": "active", "intent": "order", "medicationCodeableConcept": {"text": "Synthetic medication"}, "subject": SUBJECT, "authoredOn": DATE, "requester": {"reference": "Practitioner/a"}}),
    ("immunization", "11369-6", {"resourceType": "Immunization", "id": "imm", "status": "completed", "vaccineCode": {"text": "Synthetic vaccine"}, "patient": SUBJECT, "occurrenceDateTime": DATE, "primarySource": True}),
    ("procedure", "47519-4", {"resourceType": "Procedure", "id": "proc", "status": "completed", "code": {"text": "Synthetic procedure"}, "subject": SUBJECT, "performedDateTime": DATE}),
    ("device-use", "46264-8", {"resourceType": "DeviceUseStatement", "id": "use", "status": "active", "subject": SUBJECT, "timingDateTime": DATE, "device": {"reference": "Device/d"}}),
    ("diagnostic-report", "30954-2", {"resourceType": "DiagnosticReport", "id": "dr", "status": "final", "category": [coding("http://terminology.hl7.org/CodeSystem/v2-0074", "LAB")], "code": {"text": "Synthetic panel"}, "subject": SUBJECT, "effectiveDateTime": DATE}),
    ("care-plan", "18776-5", {"resourceType": "CarePlan", "id": "cp", "status": "active", "intent": "plan", "subject": SUBJECT}),
    ("immunization-recommendation", "18776-5", {"resourceType": "ImmunizationRecommendation", "id": "ir", "patient": SUBJECT, "date": DATE, "recommendation": [{"vaccineCode": [{"text": "Synthetic vaccine"}], "forecastStatus": coding("http://terminology.hl7.org/CodeSystem/immunization-recommendation-status", "due")}]}),
    ("flag", "104605-1", {"resourceType": "Flag", "id": "flag", "status": "active", "code": {"text": "Synthetic alert"}, "subject": SUBJECT}),
    ("consent", "42348-3", {"resourceType": "Consent", "id": "consent", "status": "active", "scope": coding("http://terminology.hl7.org/CodeSystem/consentscope", "adr"), "category": [coding("http://loinc.org", "59284-0")], "patient": SUBJECT, "policyRule": coding("http://terminology.hl7.org/CodeSystem/consentpolicycodes", "cric")}),
    ("clinical-impression", "47420-5", {"resourceType": "ClinicalImpression", "id": "ci", "status": "completed", "subject": SUBJECT}),
]
for name, section, resource in resources:
    supporting = [{"resourceType": "Device", "id": "d", "type": {"text": "Synthetic device"}}] if name == "device-use" else []
    add(name, inputs(resource, *supporting), "Route " + resource["resourceType"] + " and retain linked supporting resources.", section=section, placed_type=resource["resourceType"])
vital = {"resourceType": "Observation", "id": "hr", "status": "final", "category": [coding("http://terminology.hl7.org/CodeSystem/observation-category", "vital-signs")],
         "code": coding("http://loinc.org", "8867-4"), "subject": SUBJECT, "effectiveDateTime": DATE,
         "valueQuantity": {"value": 72, "system": "http://unitsofmeasure.org", "code": "/min", "unit": "/min"}}
add("vital-sign", inputs(vital), "A complete heart-rate observation belongs in vital signs.", section="8716-3", placed_type="Observation")
for name, loinc, section, category in [("social-history", "72166-2", "29762-2", "social-history"), ("pregnancy", "82810-3", "10162-6", "survey"), ("laboratory", "718-7", "30954-2", "laboratory")]:
    observation = {"resourceType": "Observation", "id": name, "status": "final", "category": [coding("http://terminology.hl7.org/CodeSystem/observation-category", category)],
                   "code": coding("http://loinc.org", loinc), "subject": SUBJECT, "effectiveDateTime": DATE,
                   "dataAbsentReason": coding("http://terminology.hl7.org/CodeSystem/data-absent-reason", "unknown")}
    add(name, inputs(observation), "Exercise the " + name + " Observation route.", section=section, placed_type="Observation")
bad = inputs()
bad["resources"].pop(0)
add("no-patient", bad, "Missing Patient must reject the request.", expect_valid=False, expect_error="inputs.resources must include the Patient")
bad = inputs()
bad["author"] = "Practitioner/outside"
add("outside-author", bad, "An author outside the document must be reported by local validation or rejection.", expect_valid=False, unresolved="Practitioner/outside", omitted=["Practitioner/a"])
add("orphan-resource", inputs({"resourceType": "Organization", "id": "orphan", "name": "Synthetic unreferenced organization"}),
    "An unreachable resource must be reported in omitted and absent from the document.", omitted=["Organization/orphan"])
bad = copy.deepcopy(vital)
bad["effectiveDateTime"] = "2026"
add("vital-year-only", inputs(bad), "Vital signs require day precision (vs-1), even when supplied without meta.profile.", expect_valid=False, section="8716-3", placed_type="Observation")
changed = inputs()
changed["resources"][0]["name"][0]["given"] = ["Different"]
add("changed-content", changed, "F016: same counts and metadata but changed patient content must change document identities.", identity_differs_from="minimal")
bad = inputs(condition)
bad["resources"][2]["subject"] = {"reference": "https://other.example.org/fhir/Patient/p"}
add("foreign-patient-reference", bad, "F009: an absolute reference on another server must never bind to local Patient/p.", preserve_reference="https://other.example.org/fhir/Patient/p")
(HERE / "manifest.json").write_text(json.dumps({"kinds": ["ips", "au-ps"], "cases": cases}, indent=2) + "\n")
