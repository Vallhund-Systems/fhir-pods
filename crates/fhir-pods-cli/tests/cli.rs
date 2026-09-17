use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use fhir_pods_core::{json, node::Node};

const PROFILE: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient";
const PATIENT: &str = r#"{"resourceType":"Patient","id":"synthetic-patient","name":[{"family":"Example"}],"gender":"unknown","birthDate":"1980"}"#;

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

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fhir-pods-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn write(&self, name: &str, data: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, data).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn validate_stdin_and_file_have_identical_verdicts() {
    let files = Files::new();
    let path = files.write("patient with spaces.json", PATIENT);
    let a = result(&call(&["validate"], PATIENT.as_bytes()), 0);
    let b = result(&call(&["validate", path.to_str().unwrap()], b""), 0);
    assert_eq!(a, b);
    assert_eq!(a.member("valid"), Some(&Node::Bool(true)));
}

#[test]
fn malformed_and_nonconformant_inputs_are_verdicts_not_process_errors() {
    for input in [b"{".as_slice(), b"\xff", br#"{"resourceType":"Observation"}"#] {
        let output = result(&call(&["validate", "-"], input), 1);
        assert_eq!(output.member("valid"), Some(&Node::Bool(false)));
        let outcome = result(&call(&["validate", "--outcome"], input), 1);
        assert_eq!(outcome.member("resourceType").and_then(Node::as_str), Some("OperationOutcome"));
    }
}

#[test]
fn lower_emits_resource_and_failed_validation_without_hiding_it() {
    let valid = br#"{"identifier":[{"system":"https://example.org/mrn","value":"synthetic-001"}],"name":[{"family":"Example"}],"gender":"unknown","birthDate":"1980"}"#;
    let output = result(&call(&["lower", PROFILE], valid), 0);
    assert_eq!(output.member("resource").unwrap().member("resourceType").and_then(Node::as_str), Some("Patient"));
    let invalid = result(&call(&["lower", PROFILE], b"{}"), 1);
    assert_eq!(invalid.member("validation").unwrap().member("valid"), Some(&Node::Bool(false)));
    let rejected = result(&call(&["lower", "https://example.org/not-compiled"], b"{}"), 1);
    assert!(rejected.member("error").is_some());
}

#[test]
fn deidentify_checks_remaining_findings_and_accepts_policy_on_stdin() {
    let files = Files::new();
    let resource = files.write("patient.json", PATIENT);
    let policy = r#"{"salt":"synthetic-cli-test","today":"2026-09-16"}"#;
    let clean = result(&call(&["deidentify", "--policy", "-", resource.to_str().unwrap()], policy.as_bytes()), 0);
    assert_eq!(clean.member("remaining"), Some(&Node::Array(vec![])));
    assert_eq!(clean.member("residual"), Some(&Node::Array(vec![])));
    assert!(clean.member("resource").unwrap().member("name").is_none());
    let keep = files.write("policy.json", r#"{"salt":"synthetic-cli-test","base":"none"}"#);
    let output = result(&call(&["deidentify", "--policy", keep.to_str().unwrap()], PATIENT.as_bytes()), 1);
    assert_ne!(output.member("remaining"), Some(&Node::Array(vec![])));
    assert_eq!(output.member("validation").unwrap().member("valid"), Some(&Node::Bool(true)));
}

#[test]
fn deidentify_reports_explicitly_kept_text_even_with_no_remaining_findings() {
    let files = Files::new();
    let policy = files.write(
        "keep.json",
        r#"{"salt":"synthetic-cli-test","rules":[{"target":"CodeableConcept.text","action":"keep"}]}"#,
    );
    let input = r#"{"resourceType":"Observation","status":"final","code":{"text":"SYNTHETIC_KEEP"}}"#;
    let output = result(&call(&["deidentify", "--policy", policy.to_str().unwrap()], input.as_bytes()), 0);
    assert_eq!(output.member("remaining"), Some(&Node::Array(vec![])));
    assert_eq!(output.member("validation").unwrap().member("valid"), Some(&Node::Bool(true)));
    let expected =
        Node::from_value(&json::parse(br#"[{"path":"Observation.code.text","what":"free text kept"}]"#).unwrap());
    assert_eq!(output.member("residual"), Some(&expected));
    assert_eq!(
        output.member("resource").unwrap().member("code").unwrap().member("text").and_then(Node::as_str),
        Some("SYNTHETIC_KEEP")
    );
}

#[test]
fn document_and_needs_emit_inspectable_reports() {
    let input = format!(
        r#"{{"resources":[{PATIENT},{{"resourceType":"Practitioner","id":"author","name":[{{"family":"Example"}}]}}],"author":"Practitioner/author","date":"2026-09-16T12:00:00Z"}}"#
    );
    let doc = result(&call(&["document", "ips"], input.as_bytes()), 0);
    assert_eq!(doc.member("document").unwrap().member("type").and_then(Node::as_str), Some("document"));
    assert_eq!(doc.member("omitted"), Some(&Node::Array(vec![])));
    assert!(result(&call(&["document", "ips"], b"{}"), 1).member("error").is_some());
    let needs = result(&call(&["needs"], br#"{"resourceType":"NoSuchResource"}"#), 1);
    assert_eq!(needs.member("validation").unwrap().member("valid"), Some(&Node::Bool(false)));
    assert_eq!(needs.member("needs"), Some(&Node::Array(vec![])));
}

#[test]
fn usage_and_io_failures_have_separate_exit_and_stderr() {
    for args in [
        vec![],
        vec!["unknown"],
        vec!["lower"],
        vec!["validate", "--policy", "x"],
        vec!["validate", "a", "b"],
        vec!["deidentify", "--policy", "-"],
        vec!["document", "unknown"],
        vec!["validate", "/no/such/fhir-pods-input"],
    ] {
        let output = call(&args, b"");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(Node::from_value(&json::parse(&output.stderr).unwrap()).member("error").is_some());
    }
}

#[test]
fn literal_option_filename_and_help_work() {
    let files = Files::new();
    files.write("--help", PATIENT);
    let output = Command::new(env!("CARGO_BIN_EXE_fhir-pods"))
        .current_dir(&files.0)
        .args(["validate", "--", "--help"])
        .output()
        .unwrap();
    result(&output, 0);
    let output = call(&["--help"], b"");
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8(output.stdout).unwrap().contains("deidentify --policy"));
}

const HR: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";
const HR_INPUTS: &[u8] = br#"{"status":"final","subject":{"reference":"Patient/synthetic","display":"Line\n\"two\"\\tab\t"},"effectiveDateTime":"2026-09-16T08:00:00+10:00","valueQuantity.value":72,"valueQuantity.unit":"beats/minute"}"#;

fn heart_rate() -> Node {
    result(&call(&["lower", HR], HR_INPUTS), 0).member("resource").unwrap().clone()
}

#[test]
fn lift_selects_explicit_or_declared_profile_and_accepts_files_and_stdin() {
    let files = Files::new();
    let resource = heart_rate();
    let bytes = resource.to_json();
    let path = files.write("resource with spaces.json", &bytes);
    let lifted = result(&call(&["lift", HR], bytes.as_bytes()), 0);
    for args in [vec!["lift", HR, "-"], vec!["lift", ""], vec!["lift", HR, path.to_str().unwrap()]] {
        assert_eq!(result(&call(&args, bytes.as_bytes()), 0), lifted);
    }
    assert_eq!(lifted.member("missing"), Some(&Node::Array(vec![])));
    assert_eq!(lifted.member("extras"), Some(&Node::Array(vec![])));
    let inputs = lifted.member("inputs").unwrap();
    let expected = Node::from_value(&json::parse(HR_INPUTS).unwrap());
    assert_eq!(inputs, &expected);
    let again = result(&call(&["lower", HR], inputs.to_json().as_bytes()), 0);
    assert_eq!(again.member("resource"), Some(&resource));
    assert_eq!(again.member("validation"), lifted.member("validation"));

    // A literal option-like filename must remain readable after --.
    files.write("--help", &bytes);
    let output = Command::new(env!("CARGO_BIN_EXE_fhir-pods"))
        .current_dir(&files.0)
        .args(["lift", "", "--", "--help"])
        .output()
        .unwrap();
    assert_eq!(result(&output, 0), lifted);
}

#[test]
fn lift_reports_extras_without_failing_and_escapes_untrusted_paths() {
    let mut resource = heart_rate();
    let Node::Object(ref mut members) = resource else { panic!("resource object") };
    members.push(("issued".into(), Node::Str("2026-09-16T08:00:00Z".into())));
    let lifted = result(&call(&["lift", HR], resource.to_json().as_bytes()), 0);
    assert_eq!(lifted.member("extras"), Some(&Node::Array(vec![Node::Str("issued".into())])));
    assert!(lifted.member("inputs").unwrap().member("issued").is_none());

    // An unknown key is invalid FHIR but must still form a valid JSON report.
    let key = "synthetic\"\n\\雪";
    let Node::Object(ref mut members) = resource else { unreachable!() };
    members.push((key.into(), Node::Bool(true)));
    let lifted = result(&call(&["lift", HR], resource.to_json().as_bytes()), 1);
    assert_eq!(lifted.member("missing"), Some(&Node::Array(vec![])));
    assert_eq!(lifted.member("extras"), Some(&Node::Array(vec![Node::Str("issued".into()), Node::Str(key.into())])));
    assert_eq!(lifted.member("validation").unwrap().member("valid"), Some(&Node::Bool(false)));
}

#[test]
fn lift_applies_an_undeclared_profile_and_keeps_missing_slots_with_failed_validation() {
    // This is valid against base Patient but not AU Core Patient. The CLI
    // must return the lift verdict, rather than validating only declarations.
    result(&call(&["validate"], PATIENT.as_bytes()), 0);
    let lifted = result(&call(&["lift", PROFILE], PATIENT.as_bytes()), 1);
    assert_eq!(lifted.member("missing"), Some(&Node::Array(vec![Node::Str("identifier".into())])));
    assert_eq!(lifted.member("validation").unwrap().member("valid"), Some(&Node::Bool(false)));
    assert!(lifted.member("inputs").unwrap().member("name").is_some());
    assert!(lifted.member("error").is_none());
}

#[test]
fn lift_rejected_resources_and_profiles_are_json_errors_on_stdout() {
    let wrong = b"{\"resourceType\":\"Observation\"}".as_slice();
    for (profile, input) in [
        (PROFILE, b"{".as_slice()),
        (PROFILE, b"\xff".as_slice()),
        (PROFILE, b"[]".as_slice()),
        (PROFILE, b"{}".as_slice()),
        (PROFILE, wrong),
        ("https://example.org/not-compiled", PATIENT.as_bytes()),
        ("", PATIENT.as_bytes()),
    ] {
        let lifted = result(&call(&["lift", profile], input), 1);
        assert!(lifted.member("error").and_then(Node::as_str).is_some());
        assert!(lifted.member("inputs").is_none());
    }
}

#[test]
fn lift_usage_and_io_errors_are_json_on_stderr() {
    for args in [
        vec!["lift"],
        vec!["lift", PROFILE, "--outcome"],
        vec!["lift", "", "a", "b"],
        vec!["lift", "", "/no/such/fhir-pods-input"],
    ] {
        let output = call(&args, b"");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(Node::from_value(&json::parse(&output.stderr).unwrap()).member("error").is_some());
    }
    let help = call(&["--help"], b"");
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8(help.stdout).unwrap().contains("lift PROFILE_URL|\"\""));
}
