mod common;
use common::*;
use fhir_pods_core::validate_bytes;

// --- wire: OperationOutcome ---

#[test]
fn operation_outcome_is_spec_shaped() {
    let v = validate_bytes(br#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"active":true}"#);
    let o = v.to_operation_outcome();
    assert!(o.starts_with("{\"resourceType\":\"OperationOutcome\",\"issue\":["), "{o}");
    assert!(o.contains("\"severity\":\"error\",\"code\":\"required\""), "{o}");
    // A missing element is located at its parent, as the reference validator does.
    assert!(o.contains("\"expression\":[\"Patient\"]"), "{o}");
}

#[test]
fn operation_outcome_success_carries_one_informational_issue() {
    // With narrative, so dom-6 (a warning) is quiet and the outcome is clean.
    let with_text = PATIENT_OK.replace(
        "\"gender\"",
        "\"text\":{\"status\":\"generated\",\"div\":\"<div xmlns=\\\"http://www.w3.org/1999/xhtml\\\"/>\"},\"gender\"",
    );
    let v = validate_bytes(with_text.as_bytes());
    assert!(v.is_valid());
    let o = v.to_operation_outcome();
    assert!(o.contains("\"severity\":\"information\",\"code\":\"informational\""), "{o}");
}

#[test]
fn parse_failure_is_fatal() {
    let v = validate_bytes(b"not json");
    assert!(!v.is_valid());
    assert!(v.to_operation_outcome().contains("\"severity\":\"fatal\",\"code\":\"structure\""));
}

#[test]
fn wire_expression_locates_missing_element_at_parent() {
    use fhir_pods_core::validate::IssueType;
    use fhir_pods_core::wire::expression;
    assert_eq!(expression("Patient.gender", IssueType::Required), "Patient");
    assert_eq!(expression("Observation.code.coding:snomedHeartRateCode", IssueType::Required), "Observation.code");
}

#[test]
fn wire_expression_rewrites_slice_labels_to_fhirpath() {
    use fhir_pods_core::validate::IssueType;
    use fhir_pods_core::wire::expression;
    assert_eq!(
        expression("Observation.value[x]:valueQuantity.code", IssueType::Value),
        "Observation.value.ofType(Quantity).code"
    );
    assert_eq!(
        expression("Observation.category:VSCat.coding.system", IssueType::Value),
        "Observation.category.coding.system"
    );
}

#[test]
fn f022_control_characters_are_escaped_never_replaced() {
    let mut out = String::new();
    fhir_pods_core::wire::escape(&mut out, "line one\nline two\ttab\rcr \u{1} \"q\" \\");
    assert_eq!(out, r#"line one\nline two\ttab\rcr \u0001 \"q\" \\"#);
    // through an operation: a user's name with a newline comes back as it went in
    let a = fhir_pods_core::Validator::new()
        .audit("read", br#"{"recorded":"2026-09-16T10:00:00Z","client":{"who":"Device/a","network":"10.0.0.7"},"server":{"who":"Device/b","network":"10.0.0.8"},"user":{"who":"Practitioner/p","name":"Dr\nWho"},"source":{"observer":"Device/c"},"data":"Observation/o"}"#)
        .unwrap();
    assert!(a.resource.contains(r#""name":"Dr\nWho""#), "{}", a.resource);
    let parsed = fhir_pods_core::json::parse(a.resource.as_bytes()).unwrap();
    assert!(!a.resource.is_empty() && matches!(parsed, fhir_pods_core::json::Value::Object(_)));
}
