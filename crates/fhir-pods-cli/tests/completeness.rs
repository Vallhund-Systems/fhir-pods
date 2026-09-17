mod common;

use common::{array, call, parse, result, Files};
use fhir_pods_core::{node::Node, wire, Validator};

const AU: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient";
const HR: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";
const DR: &str = "http://hl7.org/fhir/uv/ips/StructureDefinition/DiagnosticReport-uv-ips";
const TABLE: &str = include_str!("fixtures/terminology/diagnostics-status.txt");
const REPORT: &str = r#"{"resourceType":"DiagnosticReport","id":"synthetic-report","meta":{"profile":["http://hl7.org/fhir/uv/ips/StructureDefinition/DiagnosticReport-uv-ips"]},"status":"final","code":{"text":"Synthetic report"},"subject":{"reference":"Patient/synthetic"},"effectiveDateTime":"2026-09-17T10:00:00Z"}"#;

#[test]
fn slots_expose_types_cardinality_choices_and_must_support_without_input() {
    let report = result(&call(&["slots", HR], b""), 0);
    let slots = array(report.member("slots").unwrap());
    let value = slots.iter().find(|s| s.member("path").and_then(Node::as_str) == Some("valueQuantity.value")).unwrap();
    assert_eq!(value.member("type").and_then(Node::as_str), Some("decimal"));
    assert_eq!(value.member("max"), Some(&Node::Number("1".into())));
    assert!(matches!(value.member("mustSupport"), Some(Node::Bool(_))));
    let patient = result(&call(&["slots", AU], b""), 0);
    assert!(array(patient.member("slots").unwrap()).iter().any(|s| s.member("max") == Some(&Node::Null)));
    let audit = result(
        &call(&["slots", "http://hl7.org/fhir/us/core/StructureDefinition/us-core-diagnosticreport-note"], b""),
        0,
    );
    assert!(array(audit.member("slots").unwrap()).iter().any(|s| !array(s.member("choices").unwrap()).is_empty()));
    assert_eq!(result(&call(&["slots", &format!("{HR}|2.0.0")], b""), 0), report);
    assert!(result(&call(&["slots", "https://example.org/not-compiled"], b""), 1).member("error").is_some());
    for args in [vec!["slots"], vec!["slots", HR, "-"], vec!["slots", HR, "--terminology", "table.txt"]] {
        result(&call(&args, b""), 2);
    }
}

#[test]
fn must_support_gaps_are_information_while_invalid_resources_still_fail() {
    let patient = format!(
        r#"{{"resourceType":"Patient","meta":{{"profile":["{AU}"]}},"identifier":[{{"system":"https://example.org/ids","value":"synthetic"}}],"name":[{{"family":"Example"}}],"gender":"unknown","birthDate":"1980"}}"#
    );
    let report = result(&call(&["must-support"], patient.as_bytes()), 0);
    let gaps = array(report.member("mustSupportGaps").unwrap());
    assert!(!gaps.is_empty());
    let core = Validator::new();
    let expected =
        Node::Array(core.must_support_gaps(patient.as_bytes()).iter().map(|s| Node::Str((*s).into())).collect());
    assert_eq!(report.member("mustSupportGaps"), Some(&expected));
    assert_eq!(report.member("validation"), Some(&parse(wire::report(&core.validate(patient.as_bytes())).as_bytes())));
    let invalid = result(&call(&["must-support"], br#"{"resourceType":"NotAResource"}"#), 1);
    assert_eq!(invalid.member("mustSupportGaps"), Some(&Node::Array(vec![])));
    assert_eq!(invalid.member("validation").unwrap().member("valid"), Some(&Node::Bool(false)));
}

#[test]
fn terminology_load_resolves_needs_and_rejects_nonmembers_without_changing_bundled_answers() {
    let files = Files::new();
    let table = files.write("complete table.txt", TABLE);
    let before = result(&call(&["needs"], REPORT.as_bytes()), 0);
    let needs = array(before.member("needs").unwrap());
    assert_eq!(needs.len(), 1);
    assert_eq!(
        needs[0].member("valueSet").and_then(Node::as_str),
        Some("http://hl7.org/fhir/uv/ips/ValueSet/diagnostics-report-status-uv-ips|2.0.1")
    );
    assert_eq!(needs[0].member("system").and_then(Node::as_str), Some(""));
    assert_eq!(needs[0].member("code").and_then(Node::as_str), Some("final"));
    let after = result(&call(&["needs", "--terminology", table.to_str().unwrap()], REPORT.as_bytes()), 0);
    assert_eq!(after.member("needs"), Some(&Node::Array(vec![])));
    let negative = REPORT.replace("\"final\"", "\"entered-in-error\"");
    let rejected = result(&call(&["needs", "--terminology", table.to_str().unwrap()], negative.as_bytes()), 1);
    assert_eq!(rejected.member("needs"), Some(&Node::Array(vec![])));
    assert_eq!(rejected.member("validation").unwrap().member("valid"), Some(&Node::Bool(false)));
    let resource = files.write("report.json", REPORT);
    let stdin_table =
        result(&call(&["validate", resource.to_str().unwrap(), "--terminology", "-"], TABLE.as_bytes()), 0);
    assert_eq!(after.member("validation"), Some(&stdin_table));
    // Loading an empty copy of a bundled set must not override the core.
    let conflict =
        files.write("conflict.txt", "fhir-pods-terminology 1\n= http://hl7.org/fhir/ValueSet/administrative-gender\n");
    result(
        &call(
            &["validate", "--terminology", conflict.to_str().unwrap()],
            format!(r#"{{"resourceType":"Patient","meta":{{"profile":["{AU}"]}},"identifier":[{{"system":"https://example.org/ids","value":"synthetic"}}],"name":[{{"family":"Example"}}],"gender":"female","birthDate":"1980"}}"#).as_bytes(),
        ),
        0,
    );
}

#[test]
fn lowering_and_lifting_use_the_supplied_terminology_in_their_own_reports() {
    let files = Files::new();
    let table = files.write("complete.txt", TABLE);
    let inputs = r#"{"status":"entered-in-error","code":{"text":"Synthetic report"},"subject":"Patient/synthetic","effectiveDateTime":"2026-09-17T10:00:00Z"}"#;
    result(&call(&["lower", DR], inputs.as_bytes()), 0);
    let lowered = result(&call(&["lower", DR, "--terminology", table.to_str().unwrap()], inputs.as_bytes()), 1);
    assert_eq!(lowered.member("validation").unwrap().member("valid"), Some(&Node::Bool(false)));
    let resource = lowered.member("resource").unwrap().to_json();
    let lifted = result(&call(&["lift", DR, "--terminology", table.to_str().unwrap()], resource.as_bytes()), 1);
    assert_eq!(lifted.member("validation").unwrap().member("valid"), Some(&Node::Bool(false)));
    assert!(lifted.member("inputs").is_some());
}

#[test]
fn terminology_configuration_errors_precede_input_and_unsupported_modes_are_explicit() {
    let files = Files::new();
    let bad = files.write("bad.txt", "not a compiled terminology table");
    let empty = files.write("empty.txt", "");
    for table in [bad.to_str().unwrap(), empty.to_str().unwrap()] {
        let error = result(&call(&["validate", "/no/such/input", "--terminology", table], b""), 2);
        assert!(error.member("error").and_then(Node::as_str).unwrap().starts_with("invalid terminology"));
    }
    for args in [
        vec!["validate", "--terminology"],
        vec!["validate", "--terminology", "-"],
        vec!["validate", "--terminology", "one", "--terminology", "two"],
        vec!["validate", "--terminology", "/no/such/table"],
        vec!["deidentify", "--policy", "policy.json", "--terminology", "table.txt"],
        vec!["v2", "--structure", "ORU_R01", "--terminology", "table.txt"],
    ] {
        result(&call(&args, b""), 2);
    }
    let resource = files.write("resource.json", REPORT);
    result(&call(&["validate", resource.to_str().unwrap(), "--terminology", "-"], b"\xff"), 2);
}

#[test]
fn valid_operation_outcome_and_applied_actions_are_complete() {
    let patient = r#"{"resourceType":"Patient","id":"synthetic","text":{"status":"generated","div":"<div xmlns=\"http://www.w3.org/1999/xhtml\">Synthetic example</div>"},"name":[{"family":"Example"}],"birthDate":"1980-04-12"}"#;
    let outcome = result(&call(&["validate", "--outcome"], patient.as_bytes()), 0);
    assert_eq!(outcome.member("resourceType").and_then(Node::as_str), Some("OperationOutcome"));
    let issues = array(outcome.member("issue").unwrap());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].member("severity").and_then(Node::as_str), Some("information"));
    assert_eq!(issues[0].member("code").and_then(Node::as_str), Some("informational"));
    let files = Files::new();
    let policy = files.write("policy.json", r#"{"salt":"synthetic-cli-test","today":"2026-09-17"}"#);
    let deid = result(&call(&["deidentify", "--policy", policy.to_str().unwrap()], patient.as_bytes()), 0);
    let applied = array(deid.member("applied").unwrap());
    for (path, action) in [("Patient.name", "remove"), ("Patient.birthDate", "year")] {
        assert!(
            applied.iter().any(|a| a.member("path").and_then(Node::as_str) == Some(path)
                && a.member("action").and_then(Node::as_str) == Some(action)),
            "{applied:?}"
        );
    }
    assert_eq!(deid.member("resource").unwrap().member("birthDate").and_then(Node::as_str), Some("1980"));
}

#[test]
fn document_placement_is_serialized_and_points_at_an_actual_section_entry() {
    let inputs = br#"{"resources":[{"resourceType":"Patient","id":"p","birthDate":"1980-04-12","identifier":[{"system":"https://example.org/ids","value":"synthetic-p"}],"name":[{"family":"Example"}],"gender":"unknown"},{"resourceType":"Practitioner","id":"a"},{"resourceType":"Condition","id":"c","clinicalStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-clinical","code":"active"}]},"category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-category","code":"problem-list-item"}]}],"code":{"text":"Synthetic condition"},"subject":{"reference":"Patient/p"}}],"author":"Practitioner/a","date":"2026-09-17T10:00:00Z"}"#;
    let report = result(&call(&["document", "ips"], inputs), 0);
    let placed = array(report.member("placed").unwrap());
    assert_eq!(placed.len(), 2);
    assert!(placed.iter().any(|p| p.member("section").and_then(Node::as_str) == Some("")));
    let condition = placed.iter().find(|p| p.member("section").and_then(Node::as_str) == Some("11450-4")).unwrap();
    let url = condition.member("fullUrl").unwrap();
    let entries = array(report.member("document").unwrap().member("entry").unwrap());
    assert!(entries.iter().any(|e| e.member("fullUrl") == Some(url)
        && e.member("resource").unwrap().member("resourceType").and_then(Node::as_str) == Some("Condition")));
    let composition = entries[0].member("resource").unwrap();
    assert!(array(composition.member("section").unwrap()).iter().filter_map(|s| s.member("entry")).any(|refs| array(
        refs
    )
    .iter()
    .any(|r| r.member("reference") == Some(url))));
}

#[cfg(unix)]
#[test]
fn read_only_stdout_and_broken_pipes_return_exit_two() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let files = Files::new();
    let readonly = files.write("read-only-output", "unchanged");
    for command in ["validate", "--help", "--version"] {
        let mut child = Command::new("sh")
            .args([
                "-c",
                "exec \"$1\" \"$2\" 1< \"$3\"",
                "output-test",
                env!("CARGO_BIN_EXE_fhir-pods"),
                command,
                readonly.to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if command == "validate" {
            child.stdin.take().unwrap().write_all(br#"{"resourceType":"Patient"}"#).unwrap();
        }
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(2), "read-only stdout: {command}");
        assert!(out.stdout.is_empty());
        assert_eq!(std::fs::read_to_string(&readonly).unwrap(), "unchanged");
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_fhir-pods"))
        .arg("validate")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    child.stdin.take().unwrap().write_all(br#"{"resourceType":"Patient"}"#).unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(2));
}

#[test]
fn version_and_help_describe_the_delivered_surface() {
    let version = call(&["--version"], b"");
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(version.stdout, format!("fhir-pods {}\n", env!("CARGO_PKG_VERSION")).as_bytes());
    let help = call(&["--help"], b"");
    let text = std::str::from_utf8(&help.stdout).unwrap();
    for part in ["slots PROFILE_URL", "must-support", "--terminology TABLE", "v2 --structure"] {
        assert!(text.contains(part));
    }
}

#[cfg(unix)]
#[test]
fn inherited_closed_stdout_is_reopened_by_the_rust_runtime() {
    // This is not an observable write failure: before main, Rust replaces
    // an inherited closed standard descriptor with /dev/null.
    let out = std::process::Command::new("sh")
        .args(["-c", "exec \"$1\" --version 1>&-", "output-test", env!("CARGO_BIN_EXE_fhir-pods")])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
}
