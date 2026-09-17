use std::fs;
use std::io::Write;
use std::process::{Command, Output, Stdio};

use fhir_pods_core::{json, node::Node};

// Same synthetic host inputs as the v0.6.0 audit conformance corpus. Expected
// resources are frozen fixtures, not outputs computed by the API under test.
const BASE: &str = r#"{
  "recorded": "2026-09-16T10:00:00Z",
  "client": {"who": "Device/app-7", "network": "10.0.0.7"},
  "server": {"who": {"display": "fhir.example.org"}, "network": "https://fhir.example.org/r4"},
  "user": {"who": "Practitioner/gp1", "name": "Dr Who"},
  "source": {"observer": "Device/audit-collector", "site": "clinic-1"},
  "data": "Observation/hr1",
  "transaction": "3f2a1c9e-7b1d-4f0e-9a8c-1d2e3f4a5b6c"
}"#;

fn inputs(kind: &str) -> String {
    let mut input = BASE.to_owned();
    if kind.starts_with("patient-") {
        input = input.replace(r#""data":"#, r#""patient": "Patient/p1", "data":"#);
    }
    if kind.ends_with("query") {
        input = input.replace(r#""data": "Observation/hr1""#, r#""query": "GET /r4/Observation?patient=p1&_count=10""#);
    }
    input
}

fn call(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fhir-pods"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn result(output: &Output, code: i32) -> Node {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
    Node::from_value(&json::parse(&output.stdout).unwrap())
}

macro_rules! audit_case {
    ($test:ident, $kind:literal) => {
        #[test]
        fn $test() {
            let output = result(&call(&["audit", $kind], inputs($kind).as_bytes()), 0);
            let expected = Node::from_value(
                &json::parse(include_bytes!(concat!("fixtures/audit/audit-", $kind, ".json"))).unwrap(),
            );
            assert_eq!(output.member("resource"), Some(&expected));
            assert_eq!(output.member("validation").unwrap().member("valid"), Some(&Node::Bool(true)));
        }
    };
}

audit_case!(audit_read_matches_corpus, "read");
audit_case!(audit_patient_read_matches_corpus, "patient-read");
audit_case!(audit_query_matches_corpus, "query");
audit_case!(audit_patient_query_matches_corpus, "patient-query");
audit_case!(audit_create_matches_corpus, "create");
audit_case!(audit_patient_create_matches_corpus, "patient-create");
audit_case!(audit_update_matches_corpus, "update");
audit_case!(audit_patient_update_matches_corpus, "patient-update");
audit_case!(audit_delete_matches_corpus, "delete");
audit_case!(audit_patient_delete_matches_corpus, "patient-delete");

#[test]
fn audit_file_and_explicit_stdin_match_default_stdin() {
    let path = std::env::temp_dir().join(format!("fhir-pods-cli-audit-{} inputs.json", std::process::id()));
    fs::write(&path, BASE).unwrap();
    let file = call(&["audit", "read", path.to_str().unwrap()], b"");
    fs::remove_file(path).unwrap();
    let implicit = result(&call(&["audit", "read"], BASE.as_bytes()), 0);
    assert_eq!(result(&file, 0), implicit);
    assert_eq!(result(&call(&["audit", "read", "-"], BASE.as_bytes()), 0), implicit);
}

#[test]
fn audit_rejected_inputs_are_json_on_stdout_with_exit_one() {
    for (kind, input) in [
        ("read", b"{".as_slice()),
        ("read", b"\xff".as_slice()),
        ("read", b"[]".as_slice()),
        ("read", b"{}".as_slice()),
        ("patient-read", BASE.as_bytes()),
        ("query", BASE.as_bytes()),
    ] {
        let output = result(&call(&["audit", kind], input), 1);
        assert!(output.member("error").and_then(Node::as_str).is_some());
        assert!(output.member("resource").is_none());
    }
}

#[test]
fn audit_invalid_generated_record_keeps_resource_and_failed_report() {
    let input = BASE.replace(r#""name": "Dr Who""#, r#""name": "Dr Who", "requestor": false"#);
    let output = result(&call(&["audit", "read"], input.as_bytes()), 1);
    assert_eq!(output.member("resource").unwrap().member("resourceType").and_then(Node::as_str), Some("AuditEvent"));
    assert_eq!(output.member("validation").unwrap().member("valid"), Some(&Node::Bool(false)));
    assert!(output.member("error").is_none());
}

#[test]
fn audit_usage_errors_precede_input_reading() {
    for args in [
        vec!["audit"],
        vec!["audit", "unknown", "/no/such/fhir-pods-input"],
        vec!["audit", "read", "--outcome"],
        vec!["audit", "read", "a", "b"],
        vec!["audit", "read", "/no/such/fhir-pods-input"],
    ] {
        let output = call(&args, b"");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error = Node::from_value(&json::parse(&output.stderr).unwrap());
        assert!(error.member("error").and_then(Node::as_str).is_some());
        if args.get(1) == Some(&"unknown") {
            assert_eq!(error.member("error").and_then(Node::as_str), Some("unknown audit kind; use --help"));
        }
    }
}

#[test]
fn lift_matches_core_and_round_trips_lowered_copies_of_ten_audit_resources() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audit");
    let manifest = Node::from_value(&json::parse(&fs::read(fixtures.join("manifest.json")).unwrap()).unwrap());
    let Some(Node::Array(cases)) = manifest.member("cases") else { panic!("manifest cases") };
    assert_eq!(cases.len(), 10);
    for case in cases {
        let file = case.member("file").and_then(Node::as_str).unwrap();
        let profile = case.member("profile").and_then(Node::as_str).unwrap();
        let data = fs::read(fixtures.join(file)).unwrap();
        // Some valid records still report missing slots in the core. Check
        // the CLI's full result and exit contract independently of whether
        // the engine can populate every labelled slot.
        let native = fhir_pods_core::Validator::new().lift(profile, &data).unwrap();
        assert!(native.validation.is_valid(), "{file}");
        let lifted = result(&call(&["lift", profile], &data), i32::from(!native.missing.is_empty()));
        let missing = Node::Array(native.missing.iter().map(|s| Node::Str((*s).into())).collect());
        assert_eq!(lifted.member("missing"), Some(&missing), "{file}");
        assert_eq!(lifted.member("extras"), Some(&Node::Array(vec![])), "{file}");
        let inputs = Node::from_value(&json::parse(native.inputs.as_bytes()).unwrap());
        let validation =
            Node::from_value(&json::parse(fhir_pods_core::wire::report(&native.validation).as_bytes()).unwrap());
        assert_eq!(lifted.member("inputs"), Some(&inputs), "{file}");
        assert_eq!(lifted.member("validation"), Some(&validation), "{file}");

        // Round-trip the lowered copy. This deliberately does not establish
        // preservation of fields outside slots in the original audit record.
        let lowered = result(&call(&["lower", profile], inputs.to_json().as_bytes()), 0);
        let resource = lowered.member("resource").unwrap().to_json();
        let expected = fhir_pods_core::Validator::new().lift(profile, resource.as_bytes()).unwrap();
        let lifted = result(&call(&["lift", profile], resource.as_bytes()), i32::from(!expected.missing.is_empty()));
        let again = result(&call(&["lower", profile], lifted.member("inputs").unwrap().to_json().as_bytes()), 0);
        assert_eq!(again, lowered, "{file}");
    }
}
