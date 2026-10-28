//! Stable rule ids: every finding carries one, the table documents every
//! one with the severity a host may see it at, nothing in the page is absent
//! from the table (or the other way round), and nothing the lanes produce
//! falls through to `other.*`.

use fhir_pods_core::rules::{rule_id, RULES};
use fhir_pods_core::validate::IssueType;

/// The page is generated from the table (`tools/gen_rules_doc.py`), and this
/// holds the two together in **both** directions: every id in the table is on
/// the page with its severity and meaning verbatim, and every id the page
/// lists is in the table. A host reads the page; a hand-edit to either side,
/// or a stale regeneration, fails here.
#[test]
fn every_rule_id_is_documented_and_every_documented_id_is_real() {
    let docs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/rules.md")).unwrap();
    for (id, severity, meaning) in RULES {
        assert!(
            docs.contains(&format!("| `{id}` | {severity} | {meaning} |")),
            "docs/rules.md does not carry `{id}` with its severity and meaning (regenerate it)"
        );
    }
    // the other direction: an id on the page that no longer exists in the
    // table (a rename, a hand-edited row) is just as much a drift
    let listed: std::collections::BTreeSet<&str> = docs
        .lines()
        .filter(|l| l.matches('|').count() == 4) // an id row: id, severity, meaning
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split_once('`'))
        .map(|(id, _)| id)
        .collect();
    let table: std::collections::BTreeSet<&str> = RULES.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(listed, table, "docs/rules.md and the RULES table list different ids");
    // every id the classifier can return is in the table (or an invariant / fallback family)
    let samples = [
        (IssueType::Structure, "could not parse JSON at byte 3: x"),
        (IssueType::Structure, "resourceType is required"),
        (IssueType::Structure, "Patient.foo is not an element of Patient"),
        (IssueType::Required, "Patient.gender is required (min 1, found 0)"),
        (IssueType::CodeInvalid, "No code provided, and a code is required from the value set x"),
        (IssueType::CodeInvalid, "None of the codings provided are in the value set 'x', and a coding from this value set is required"),
        (IssueType::Structure, "Invalid Resource target type. Found Flag, but expected one of (Patient)"),
        (IssueType::Invariant, "cmp-1: A section must contain at least one of text, entries, or sub-sections"),
        (IssueType::Invariant, "que-7: If the operator is 'exists', the value must be a boolean"),
        (IssueType::NotSupported, "txt-1 not evaluated: outside the supported FHIRPath core"),
        (IssueType::Structure, "LinkId 'p2.q99' not found in questionnaire"),
        (IssueType::BusinessRule, "No response answer found for required item 'p1.age'"),
        (IssueType::CodeInvalid, "The code 'x' in the system 'y' is not in the options value set (z) specified by the questionnaire. Terminology Error: …"),
        (IssueType::Value, "Relative URLs must be of the format [ResourceName]/[id]. Encountered 1"),
        // the reviewer's reachable corners: keys the shape heuristic alone would miss, and
        // messages the first table did not claim
        (IssueType::Invariant, "sdf-8a: In any snapshot, all the elements must be in the same order as the base"),
        (IssueType::Invariant, "pvc2: Minimum count must be less than or equal to maximum count of part."),
        (IssueType::Invariant, "hasMemberOrComponent: component and hasMember are mutually exclusive"),
        (IssueType::Invariant, "zzz-not-listed-9: a key no table lists"),
        (IssueType::Value, "Bundle.entry[0].fullUrl is not a valid uri"),
        (IssueType::Structure, "Answer value must be one of the 5 types [Coding, date, time, integer, string] not boolean"),
        (IssueType::BusinessRule, "The answer to the item x is a string, but answerConstraint is 'optionsOnly', so the answer has to be one of the options the questionnaire specifies"),
        (IssueType::BusinessRule, "The mime type text/html is not valid for this answer (allowed = [text/plain])"),
        (IssueType::BusinessRule, "The resource type 'Device' in the reference Device/1 is not allowed (allowed = [Patient])"),
        (IssueType::BusinessRule, "The answer 'x' does not conform to the expected format 'nnn' (regex '[0-9]{3}')"),
        (IssueType::BusinessRule, "The value 3 kg has a unit that is not valid (allowed = [mg])"),
        (IssueType::BusinessRule, "No LinkId, so can't be validated"),
        (IssueType::CodeInvalid, "Coding has no system. A code with no system has no defined meaning, and it cannot be validated. A system should be provided"),
    ];
    let expected = [
        "parse.json",
        "parse.resource-type",
        "structure.unknown-element",
        "structure.required",
        "binding.no-code",
        "binding.not-member",
        "reference.target-type",
        "inv.cmp-1",
        "inv.que-7",
        "not-supported.fhirpath",
        "questionnaire.linkid-unknown",
        "questionnaire.required-answer",
        "questionnaire.answer-not-in-set",
        "reference.shape",
        "inv.sdf-8a",
        "inv.pvc2",
        "inv.hasMemberOrComponent",
        "inv.other",
        "value.invalid",
        "questionnaire.answer-type",
        "questionnaire.answer-not-in-set",
        "questionnaire.answer-content",
        "questionnaire.answer-content",
        "questionnaire.answer-content",
        "questionnaire.answer-content",
        "questionnaire.linkid-unknown",
        "questionnaire.answer-no-system",
    ];
    for ((code, message), want) in samples.iter().zip(expected) {
        assert_eq!(rule_id(*code, message), want, "{message}");
    }
}

#[test]
fn a_report_and_an_operation_outcome_carry_the_id() {
    let v = fhir_pods_core::validate_bytes(br#"{"resourceType":"Patient","gender":"mutated"}"#);
    assert!(v.report.issues.iter().any(|i| i.id == "binding.not-member"), "{:?}", v.report.issues);
    let json = fhir_pods_core::wire::report(&v);
    assert!(json.contains(r#""id":"binding.not-member""#), "{json}");
    let outcome = v.report.to_operation_outcome();
    assert!(outcome.contains(r#"{"system":"urn:fhir-pods:rule","code":"binding.not-member"}"#), "{outcome}");
}

/// Every finding on the release corpus (when it is present) has a real id:
/// a message no family claims is a defect of the rules table, not a
/// finding a host can act on.
#[test]
fn nothing_on_the_corpus_falls_through_to_other() {
    let conformance = concat!(env!("CARGO_MANIFEST_DIR"), "/../../conformance");
    let mut dirs = vec![format!("{conformance}/corpus"), format!("{conformance}/instrument/out/cases")];
    if let Ok(classes) = std::fs::read_dir(format!("{conformance}/mutations")) {
        dirs.extend(classes.flatten().filter(|e| e.path().is_dir()).map(|e| e.path().to_string_lossy().into_owned()));
    }
    let mut unclassified = std::collections::BTreeMap::new();
    let mut seen = 0usize;
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") || name.starts_with('_') || name.ends_with("manifest.json") {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else { continue };
            let v = fhir_pods_core::validate_bytes(&bytes);
            seen += 1;
            for i in &v.report.issues {
                if i.id.starts_with("other.") || i.id == "inv.other" {
                    unclassified.entry(i.message.chars().take(80).collect::<String>()).or_insert(i.id);
                }
            }
        }
    }
    if seen == 0 {
        return; // no derived corpora on this machine
    }
    assert!(unclassified.is_empty(), "unclassified findings: {unclassified:#?}");
}

/// The documented severity is measured, not asserted: every severity the
/// lanes on this machine actually produce for an id must be one the page
/// lists for it. Without the derived corpora it checks the committed probe
/// and narrative fixtures alone, and says the corpus floor was skipped.
#[test]
fn every_severity_the_lanes_produce_is_documented() {
    use fhir_pods_core::validate::Severity;
    let conformance = concat!(env!("CARGO_MANIFEST_DIR"), "/../../conformance");
    let mut dirs = vec![
        format!("{conformance}/corpus"),
        format!("{conformance}/instrument/out/cases"),
        format!("{conformance}/probes"),
        format!("{conformance}/narrative"),
        format!("{conformance}/coded-answers"),
    ];
    if let Ok(classes) = std::fs::read_dir(format!("{conformance}/mutations")) {
        dirs.extend(classes.flatten().filter(|e| e.path().is_dir()).map(|e| e.path().to_string_lossy().into_owned()));
    }
    let documented = |id: &str| -> Vec<&'static str> {
        let key = if id.starts_with("inv.") && id != "inv.other" { "inv.<key>" } else { id };
        let (_, severity, _) = RULES.iter().find(|(r, _, _)| *r == key).expect("every finding's id is in the table");
        ["fatal", "error", "warning", "information"].into_iter().filter(|s| severity.contains(s)).collect()
    };
    let validator = fhir_pods_core::Validator::new();
    let mut seen = 0usize;
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json")
                || name.starts_with('_')
                || name.contains("manifest")
                || name.contains("evidence")
            {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else { continue };
            seen += 1;
            for i in validator.validate(&bytes).report.issues {
                let severity = match i.severity {
                    Severity::Fatal => "fatal",
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                    Severity::Information => "information",
                };
                let allowed = documented(i.id);
                assert!(
                    allowed.contains(&severity) || allowed.is_empty(),
                    "{}: `{}` arrived at {severity}, which docs/rules.md does not list ({allowed:?}) — {}",
                    name,
                    i.id,
                    i.message
                );
            }
        }
    }
    // the floor guards the derived corpora (`conformance/run.py` builds
    // `corpus/`); on a fresh tree only the committed probe and narrative
    // fixtures exist — every one of them was still checked above, and the
    // floor is said to be skipped rather than passed
    if std::path::Path::new(&format!("{conformance}/corpus")).is_dir() {
        assert!(seen > 100, "only {seen} files: the corpora look truncated");
    } else {
        eprintln!("corpus floor skipped: {conformance}/corpus is not built (run conformance/run.py); {seen} files checked without it");
    }
}
