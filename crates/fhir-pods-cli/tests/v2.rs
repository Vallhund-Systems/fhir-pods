mod common;

use common::{array, call, parse, result, Files};
use fhir_pods_core::{hash, node::Node, v2::Options, wire, Validator};
use std::fs;
use std::path::Path;
use std::process::Command;

const ORU: &str = include_str!("fixtures/v2/oru-r01.hl7");

fn strings(values: &[String]) -> Node {
    Node::Array(values.iter().cloned().map(Node::Str).collect())
}

fn resources<'a>(report: &'a Node, kind: &str) -> Vec<&'a Node> {
    array(report.member("bundle").unwrap().member("entry").unwrap())
        .iter()
        .filter_map(|entry| entry.member("resource"))
        .filter(|resource| resource.member("resourceType").and_then(Node::as_str) == Some(kind))
        .collect()
}

#[test]
fn four_synthetic_messages_preserve_complete_core_reports_and_fixture_hashes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v2");
    let manifest = parse(&fs::read(dir.join("manifest.json")).unwrap());
    let cases = array(manifest.member("cases").unwrap());
    assert_eq!(cases.len(), 4);
    for case in cases {
        let file = case.member("file").and_then(Node::as_str).unwrap();
        let bytes = fs::read(dir.join(file)).unwrap();
        assert_eq!(hash::hex(&hash::sha256(&bytes)), case.member("sha256").and_then(Node::as_str).unwrap());
        let core = Validator::new().v2(&bytes, &Options::default()).unwrap();
        let exit = i32::from(!core.validation.is_valid() || !core.unplaced.is_empty());
        let actual = result(&call(&["v2"], &bytes), exit);
        assert_eq!(actual.member("bundle"), Some(&parse(core.bundle.as_bytes())), "{file}");
        assert_eq!(actual.member("validation"), Some(&parse(wire::report(&core.validation).as_bytes())), "{file}");
        assert_eq!(actual.member("unplaced"), Some(&strings(&core.unplaced)), "{file}");
        assert_eq!(actual.member("unmapped"), Some(&strings(&core.unmapped)), "{file}");
        let skipped = array(actual.member("skipped").unwrap());
        assert_eq!(skipped.len(), core.skipped.len(), "{file}");
        for (actual, expected) in skipped.iter().zip(core.skipped.iter()) {
            for (key, value) in [("at", &expected.at), ("target", &expected.target), ("why", &expected.why)] {
                assert_eq!(actual.member(key).and_then(Node::as_str), Some(value.as_str()), "{file}");
            }
        }
    }
}

#[test]
fn two_orders_keep_four_observations_and_mapping_limits_remain_visible_on_success() {
    let actual = result(&call(&["v2"], ORU.as_bytes()), 0);
    assert_eq!(resources(&actual, "Patient").len(), 1);
    assert_eq!(resources(&actual, "Observation").len(), 4);
    let reports = resources(&actual, "DiagnosticReport");
    assert_eq!(reports.len(), 2);
    let first = array(reports[0].member("result").unwrap());
    let second = array(reports[1].member("result").unwrap());
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert!(first.iter().all(|reference| !second.contains(reference)));
    assert!(!array(actual.member("skipped").unwrap()).is_empty());
    assert!(!array(actual.member("unmapped").unwrap()).is_empty());
    assert_eq!(actual.member("unplaced"), Some(&Node::Array(vec![])));
    assert_eq!(resources(&actual, "Observation")[3].member("valueString").and_then(Node::as_str), Some("No growth"));
}

#[test]
fn v2_accepts_files_stdin_and_segment_line_endings() {
    let files = Files::new();
    let path = files.write("message with spaces.hl7", ORU);
    let expected = result(&call(&["v2", path.to_str().unwrap()], b""), 0);
    for separator in ["\n", "\r", "\r\n"] {
        let message = ORU.replace('\n', separator);
        let actual = result(&call(&["v2", "-"], message.as_bytes()), 0);
        // IDs derive from the original bytes, so separator changes can
        // change IDs. The CLI must preserve the input bytes.
        let core = Validator::new().v2(message.as_bytes(), &Options::default()).unwrap();
        assert_eq!(actual.member("bundle"), Some(&parse(core.bundle.as_bytes())));
        assert_eq!(resources(&actual, "Observation").len(), 4);
        assert_eq!(actual.member("skipped"), expected.member("skipped"));
    }
    files.write("--help", ORU);
    let out = Command::new(env!("CARGO_BIN_EXE_fhir-pods"))
        .current_dir(&files.0)
        .args(["v2", "--", "--help"])
        .output()
        .unwrap();
    assert_eq!(result(&out, 0), expected);
}

#[test]
fn supplied_timezone_and_code_systems_change_the_intended_values() {
    let message = ORU.replace("20260916093000+1000", "20260916093000").replace("^LN", "^LOCAL");
    let actual = result(
        &call(
            &[
                "v2",
                "--timezone",
                "+10:00",
                "--code-system",
                "LOCAL=https://example.org/local",
                "--code-system",
                "SECOND=https://example.org/second",
            ],
            message.as_bytes(),
        ),
        0,
    );
    let observation = resources(&actual, "Observation")[0];
    assert_eq!(observation.member("effectiveDateTime").and_then(Node::as_str), Some("2026-09-16T09:30:00+10:00"));
    let coding = &array(observation.member("code").unwrap().member("coding").unwrap())[0];
    assert_eq!(coding.member("system").and_then(Node::as_str), Some("https://example.org/local"));
    assert_eq!(coding.member("code").and_then(Node::as_str), Some("2093-3"));
    let without_zone = ORU.replace("20260916093000+1000", "20260916093000");
    let default = result(&call(&["v2"], without_zone.as_bytes()), 0);
    assert_eq!(
        resources(&default, "Observation")[0].member("effectiveDateTime").and_then(Node::as_str),
        Some("2026-09-16")
    );
    assert!(array(default.member("skipped").unwrap()).iter().any(|s| s
        .member("why")
        .and_then(Node::as_str)
        .unwrap()
        .contains("without a zone")));
}

#[test]
fn unplaced_segments_fail_even_with_a_valid_bundle_and_invalid_bundles_stay_inspectable() {
    let actual = result(&call(&["v2"], format!("{ORU}ZPI|synthetic\n").as_bytes()), 1);
    assert_eq!(actual.member("validation").unwrap().member("valid"), Some(&Node::Bool(true)));
    assert_eq!(actual.member("unplaced"), Some(&Node::Array(vec![Node::Str("ZPI[1]".into())])));
    let adt = result(&call(&["v2"], include_bytes!("fixtures/v2/adt-a01.hl7")), 1);
    assert_eq!(adt.member("validation").unwrap().member("valid"), Some(&Node::Bool(false)));
    assert!(!resources(&adt, "Patient").is_empty());
    assert!(adt.member("error").is_none());
    // This fixture deliberately lacks required FHIR data. An invalid verdict
    // alone must not hide unrelated bad codes or a shifted v2 field.
    let errors: Vec<_> = array(adt.member("validation").unwrap().member("issues").unwrap())
        .iter()
        .filter(|issue| issue.member("severity").and_then(Node::as_str) == Some("error"))
        .map(|issue| {
            (
                issue.member("code").and_then(Node::as_str).unwrap(),
                issue.member("message").and_then(Node::as_str).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        errors,
        vec![
            // MSH-5 "GHH LAB, INC." lands in a url per the v2 maps: not a valid url (0.15.0 checks whitespace)
            ("value", "Bundle.entry.resource.destination.endpoint is not a valid url"),
            ("required", "Provenance.agent is required (min 1, found 0)"),
            ("required", "Provenance.target is required (min 1, found 0)"),
            // PID-3's "SS" identifier system: the IG's literal carried a trailing dot,
            // corrected as spelling by the generator (0.17.0) — no URL error now
            // PID-13/14, NK1-5 and the contact's phone: a ContactPoint value with no
            // system — cpt-2 on every ContactPoint, profile or none (0.15.0)
            ("invariant", "cpt-2: A system is required if a value is provided."),
            ("invariant", "cpt-2: A system is required if a value is provided."),
            ("invariant", "cpt-2: A system is required if a value is provided."),
            ("invariant", "cpt-2: A system is required if a value is provided."),
            ("required", "Coverage.status is required (min 1, found 0)"),
        ],
        "these are exactly validator_cli 6.10.4's errors on this fixture (conformance/reasons.py)"
    );
    let patient = resources(&adt, "Patient")[0];
    let contact = &array(patient.member("contact").unwrap())[0];
    assert!(array(contact.member("relationship").unwrap())
        .contains(&parse(br#"{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v2-0131","code":"N"}]}"#)));
    let coverage = resources(&adt, "Coverage")[0];
    assert_eq!(
        coverage.member("relationship"),
        Some(&parse(
            br#"{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v3-RoleCode","code":"ONESELF"}]}"#
        ))
    );
    assert!(coverage.member("type").is_none());
    // IN1-18 now carries the birth date; its absent mapping remains visible.
    assert!(array(adt.member("unmapped").unwrap()).contains(&Node::Str("IN1[1]-18".into())));
}

#[test]
fn conversion_rejections_are_json_while_option_and_io_errors_are_usage_failures() {
    for input in [b"not a message".as_slice(), b"\xff", b"MSH|^~\\&|A|B|C|D|20260101||ZZZ^Z99|1|P|2.5\r"] {
        assert!(result(&call(&["v2"], input), 1).member("error").and_then(Node::as_str).is_some());
    }
    for args in [
        vec!["v2", "--timezone"],
        vec!["v2", "--timezone", "AEST"],
        vec!["v2", "--timezone", "+1000"],
        vec!["v2", "--timezone", "Z", "--timezone", "Z"],
        vec!["v2", "--code-system", "L"],
        vec!["v2", "--code-system", "L="],
        vec!["v2", "--code-system", "=https://example.org"],
        vec!["v2", "--code-system", "L=one", "--code-system", "L=two"],
        vec!["v2", "--outcome"],
        vec!["v2", "a", "b"],
        vec!["v2", "/no/such/fhir-pods-input"],
        vec!["validate", "--timezone", "Z"],
    ] {
        result(&call(&args, b""), 2);
    }
}

#[test]
fn structure_inspection_needs_no_input_and_rejects_mixed_modes() {
    let actual = result(&call(&["v2", "--structure", "ORU_R01"], b""), 0);
    assert_eq!(actual.member("structure").and_then(Node::as_str), Some("ORU_R01"));
    let tree = actual.member("tree").and_then(Node::as_str).unwrap();
    assert!(tree.contains("ORDER_OBSERVATION") && tree.contains("OBX"));
    assert!(result(&call(&["v2", "--structure", "UNKNOWN"], b""), 1).member("error").is_some());
    for args in [vec!["v2", "--structure", "ORU_R01", "-"], vec!["v2", "--structure", "ORU_R01", "--timezone", "Z"]] {
        result(&call(&args, b""), 2);
    }
}
