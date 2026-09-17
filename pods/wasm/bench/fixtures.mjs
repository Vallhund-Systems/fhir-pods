// Synthetic, fixed inputs. Shared by the browser smoke and throughput runner.
export const patientProfile = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient";
export const patient = {
  resourceType: "Patient", id: "sample-patient", meta: { profile: [patientProfile] },
  identifier: [{ system: "https://example.org/mrn", value: "sample-001" }],
  name: [{ family: "Example", given: ["Synthetic"] }], gender: "unknown", birthDate: "1980-02-29",
};
export const heartRate = {
  resourceType: "Observation", id: "sample-heart-rate",
  meta: { profile: ["http://hl7.org/fhir/StructureDefinition/heartrate"] },
  status: "final",
  category: [{ coding: [{ system: "http://terminology.hl7.org/CodeSystem/observation-category", code: "vital-signs" }] }],
  code: { coding: [{ system: "http://loinc.org", code: "8867-4" }] },
  subject: { reference: "Patient/sample-patient" }, effectiveDateTime: "2026-09-16T12:00:00Z",
  valueQuantity: { value: 72, system: "http://unitsofmeasure.org", code: "/min", unit: "/min" },
};
export const practitioner = {
  resourceType: "Practitioner", id: "sample-practitioner", name: [{ family: "Example" }],
};
export const fixtures = [
  { name: "au-patient", valid: true, json: JSON.stringify(patient) },
  { name: "base-heart-rate", valid: true, json: JSON.stringify(heartRate) },
  { name: "au-patient-missing-required", valid: false, json: JSON.stringify({ resourceType: "Patient", meta: { profile: [patientProfile] } }) },
  { name: "malformed-json", valid: false, json: '{"resourceType":' },
];
