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
    assert!(clean.member("resource").unwrap().member("name").is_none());
    let keep = files.write("policy.json", r#"{"salt":"synthetic-cli-test","base":"none"}"#);
    let output = result(&call(&["deidentify", "--policy", keep.to_str().unwrap()], PATIENT.as_bytes()), 1);
    assert_ne!(output.member("remaining"), Some(&Node::Array(vec![])));
    assert_eq!(output.member("validation").unwrap().member("valid"), Some(&Node::Bool(true)));
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
