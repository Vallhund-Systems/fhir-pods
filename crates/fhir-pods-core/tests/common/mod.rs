//! Fixtures and helpers shared by the integration tests.
#![allow(dead_code)]

use fhir_pods_core::path::Verdict;
use fhir_pods_core::{probe, validate_bytes};

pub fn probe_str(s: &str) -> String {
    probe(s.as_bytes())
}

pub fn fp(expr: &str, doc: &str) -> Verdict {
    use fhir_pods_core::document::Parsed;
    use fhir_pods_core::path::{check, parse};
    let e = parse(expr).unwrap_or_else(|_| panic!("should parse: {expr}"));
    let d = Parsed::parse(doc.as_bytes()).unwrap();
    check(&e, &d.doc())
}

pub fn validate(s: &str) -> fhir_pods_core::Validation {
    validate_bytes(s.as_bytes())
}

pub const PATIENT_ID: &str = r#""identifier":[{"system":"http://hospital.example.org/mrn","value":"1"}]"#;
pub const PATIENT_OK: &str = r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"identifier":[{"system":"http://hospital.example.org/mrn","value":"1"}],"name":[{"family":"Doe","use":"official"}],"gender":"male","birthDate":"1970-01-01"}"#;
pub const HEARTRATE_OK: &str = r#"{"resourceType":"Observation","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate"]},"status":"final","category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs"}]}],"code":{"coding":[{"system":"http://loinc.org","code":"8867-4"},{"system":"http://snomed.info/sct","code":"364075005"}]},"subject":{"reference":"Patient/1"},"effectiveDateTime":"2026-09-15T10:00:00Z","valueQuantity":{"value":72,"unit":"beats/minute","system":"http://unitsofmeasure.org","code":"/min"}}"#;
pub const BP_OK: &str = r#"{"resourceType":"Observation","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-bloodpressure"]},"status":"final","category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs"}]}],"code":{"coding":[{"system":"http://loinc.org","code":"85354-9"},{"system":"http://snomed.info/sct","code":"75367002"}]},"subject":{"reference":"Patient/1"},"effectiveDateTime":"2026-09-15T10:00:00Z","component":[{"code":{"coding":[{"system":"http://loinc.org","code":"8480-6"},{"system":"http://snomed.info/sct","code":"271649006"}]},"valueQuantity":{"value":120,"unit":"mmHg","system":"http://unitsofmeasure.org","code":"mm[Hg]"}},{"code":{"coding":[{"system":"http://loinc.org","code":"8462-4"},{"system":"http://snomed.info/sct","code":"271650006"}]},"valueQuantity":{"value":80,"unit":"mmHg","system":"http://unitsofmeasure.org","code":"mm[Hg]"}}]}"#;
