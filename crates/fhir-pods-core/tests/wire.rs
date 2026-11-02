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
        "\"text\":{\"status\":\"generated\",\"div\":\"<div xmlns=\\\"http://www.w3.org/1999/xhtml\\\">A patient.</div>\"},\"gender\"",
    );
    let v = validate_bytes(with_text.as_bytes());
    assert!(v.is_valid());
    let o = v.to_operation_outcome();
    // every slice of AU Core Patient is discriminated (0.14.1 compiles the
    // DVA identifier by membership): the one "all OK" issue
    assert!(o.contains("\"severity\":\"information\",\"code\":\"informational\""), "{o}");
    // a profile with a slice the compiler still cannot discriminate (AU
    // Core Pathology Result's `category:specificDiscipline`: a `value`
    // discriminator on a code the profile neither fixes nor binds as
    // required): the outcome says so (information) in place of "all OK"
    let path = r#"{"resourceType":"Observation","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-diagnosticresult-path"]},
        "text":{"status":"generated","div":"<div xmlns=\"http://www.w3.org/1999/xhtml\">A result.</div>"},
        "status":"final","category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"laboratory"}]},
        {"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v2-0074","code":"CH"}]}],
        "code":{"coding":[{"system":"http://loinc.org","code":"2339-0"}]},"subject":{"reference":"Patient/1"},
        "effectiveDateTime":"2026-09-20T08:00:00+10:00","performer":[{"reference":"Practitioner/1"}],
        "valueQuantity":{"value":5.4,"unit":"mmol/L","system":"http://unitsofmeasure.org","code":"mmol/L"}}"#;
    let v = validate_bytes(path.as_bytes());
    let o = v.to_operation_outcome();
    assert!(o.contains("\"severity\":\"information\",\"code\":\"not-supported\""), "{o}");
    assert!(o.contains("slice 'specificDiscipline' of Observation.category in AU Core Pathology Result Observation was not checked"), "{o}");
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
fn wire_expression_writes_an_extension_value_by_its_type() {
    use fhir_pods_core::validate::IssueType;
    use fhir_pods_core::wire::expression;
    // validator_cli 6.10.4's spelling (conformance/probes/ext-*)
    assert_eq!(
        expression("MedicationDispense.extension[0].valueQuantity", IssueType::Structure),
        "MedicationDispense.extension[0].value.ofType(Quantity)"
    );
    assert_eq!(
        expression("Patient.name[0].extension[0].valueCode", IssueType::CodeInvalid),
        "Patient.name[0].extension[0].value.ofType(code)"
    );
    assert_eq!(
        expression("Patient.extension[0].extension[1].valueString", IssueType::Structure),
        "Patient.extension[0].extension[1].value.ofType(string)"
    );
    assert_eq!(
        expression("Patient.modifierExtension[0].valueDateTime", IssueType::Value),
        "Patient.modifierExtension[0].value.ofType(dateTime)"
    );
    // a primitive type of a choice by its lower-case type code
    assert_eq!(
        expression("Patient.deceased[x]:deceasedDateTime.extension[0]", IssueType::Structure),
        "Patient.deceased.ofType(dateTime).extension[0]"
    );
    // an extension slice's cardinality is located at the element its extensions are on
    assert_eq!(expression("Patient.address[0].extension:suburb", IssueType::Structure), "Patient.address[0]");
    assert_eq!(expression("Observation.category:VSCat", IssueType::Structure), "Observation.category");
    // only an extension's value: an element's own concrete key is as before
    assert_eq!(expression("Observation.valueQuantity", IssueType::Value), "Observation.valueQuantity");
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
