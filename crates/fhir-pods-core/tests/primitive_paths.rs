mod common;
use common::*;
use fhir_pods_core::validate::{IssueType, Severity};

#[test]
fn primitive_dates_identify_the_exact_nested_questionnaire_answer() {
    let json = r#"{"resourceType":"QuestionnaireResponse","status":"in-progress","item":[
      {"linkId":"first","answer":[{"valueDate":"2026-01-01"}]},
      {"linkId":"group","item":[{"linkId":"nested","answer":[{"valueDate":"2026-01-02"},{"valueDate":"not-a-date"}]}]}
    ]}"#;
    let result = validate(json);
    let issues: Vec<_> = result.report.issues.iter().filter(|issue| issue.id == "value.invalid").collect();
    assert_eq!(issues.len(), 1, "{}", probe_str(json));
    assert_eq!(issues[0].path, "QuestionnaireResponse.item[1].item[0].answer[1].valueDate");
    assert_eq!(issues[0].severity, Severity::Error);
    assert_eq!(issues[0].code, IssueType::Value);
    let expression = fhir_pods_core::wire::expression(&issues[0].path, issues[0].code);
    assert!(result.to_operation_outcome().contains(&format!(r#""expression":["{expression}"]"#)));
}

#[test]
fn primitive_array_values_keep_their_own_index() {
    let json = r#"{"resourceType":"Patient","meta":{"profile":["https://example.invalid/Profile/demo","relative-canonical"]}}"#;
    let result = validate(json);
    let issues: Vec<_> = result.report.issues.iter().filter(|issue| issue.id == "value.invalid").collect();
    assert_eq!(issues.len(), 1, "{}", probe_str(json));
    assert_eq!(issues[0].path, "Patient.meta.profile[1]");
    let expression = fhir_pods_core::wire::expression(&issues[0].path, issues[0].code);
    assert!(result.to_operation_outcome().contains(&format!(r#""expression":["{expression}"]"#)));
}

#[test]
fn primitive_json_kind_errors_locate_the_repeating_value() {
    let json = r#"{"resourceType":"Patient","name":[{"given":["First"]},{"given":["Second",42]}]}"#;
    let result = validate(json);
    let issue = result
        .report
        .issues
        .iter()
        .find(|issue| issue.message.contains("a string is a JSON string"))
        .unwrap_or_else(|| panic!("{}", probe_str(json)));
    assert_eq!(issue.path, "Patient.name[1].given[1]");
    assert_eq!(issue.severity, Severity::Error);
    assert_eq!(issue.code, IssueType::Value);
}

#[test]
fn wrong_complex_json_kind_keeps_the_same_instance_location() {
    let json = r#"{"resourceType":"Patient","name":[{"given":["First"]},42]}"#;
    let result = validate(json);
    let issue = result
        .report
        .issues
        .iter()
        .find(|issue| issue.message.contains("a HumanName is a JSON object"))
        .unwrap_or_else(|| panic!("{}", probe_str(json)));
    assert_eq!(issue.path, "Patient.name[1]");
}

#[test]
fn nested_resources_keep_their_outer_instance_location() {
    let json = r#"{"resourceType":"Bundle","type":"collection","entry":[
      {"fullUrl":"https://example.invalid/Patient/one","resource":{"resourceType":"Patient","id":"one","birthDate":"2026-01-01"}},
      {"fullUrl":"https://example.invalid/Patient/two","resource":{"resourceType":"Patient","id":"two","birthDate":"not-a-date"}}
    ]}"#;
    let result = validate(json);
    let issues: Vec<_> = result.report.issues.iter().filter(|issue| issue.id == "value.invalid").collect();
    assert_eq!(issues.len(), 1, "{}", probe_str(json));
    assert_eq!(issues[0].path, "Bundle.entry[1].resource.Patient.birthDate");
    let expression = fhir_pods_core::wire::expression(&issues[0].path, issues[0].code);
    assert!(result.to_operation_outcome().contains(&format!(r#""expression":["{expression}"]"#)));
}

#[test]
fn scalar_primitive_location_and_valid_values_are_unchanged() {
    let bad = validate(r#"{"resourceType":"Patient","birthDate":"not-a-date"}"#);
    let issue = bad.report.issues.iter().find(|issue| issue.id == "value.invalid").unwrap();
    assert_eq!(issue.path, "Patient.birthDate");
    let good = validate(
        r#"{"resourceType":"Patient","birthDate":"2026-01-01","meta":{"profile":["https://example.invalid/Profile/demo"]}}"#,
    );
    assert!(!good.report.issues.iter().any(|issue| issue.id == "value.invalid"));
}
