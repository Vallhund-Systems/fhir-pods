#[allow(dead_code)]
mod common;

use common::{array, call, result};
use fhir_pods_core::node::Node;

// A record as a host lowers it against the base definitions (the same
// synthetic patient as the core's export tests), given to the CLI as-is.
const RECORD: &str = r#"{
  "resources": [
    {"resourceType": "Patient", "id": "u1", "meta": {"profile": ["http://hl7.org/fhir/StructureDefinition/Patient"]},
     "identifier": [{"system": "urn:ietf:rfc:3986", "value": "urn:uuid:5b3a2b6e-1c3f-4b8e-9a9e-0c1d2e3f4a5b"}],
     "name": [{"family": "Citizen", "given": ["Alex"]}], "gender": "female", "birthDate": "1988-04-12"},
    {"resourceType": "Observation", "id": "hr1", "meta": {"profile": ["http://hl7.org/fhir/StructureDefinition/heartrate"]},
     "status": "final", "subject": {"reference": "Patient/u1"}, "effectiveDateTime": "2026-09-20T08:00:00+10:00",
     "category": [{"coding": [{"system": "http://terminology.hl7.org/CodeSystem/observation-category", "code": "vital-signs"}]}],
     "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
     "valueQuantity": {"value": 72, "unit": "beats/minute", "system": "http://unitsofmeasure.org", "code": "/min"}}
  ],
  "author": "Patient/u1",
  "date": "2026-09-21T10:00:00+10:00",
  "title": "My health record"
}"#;

fn text(node: &Node) -> &str {
    node.as_str().unwrap()
}

#[test]
fn a_base_record_is_exported_for_a_region_with_its_document_and_the_verdicts_decide_the_exit() {
    let out = result(&call(&["export", "au"], RECORD.as_bytes()), 0);
    assert_eq!(text(out.member("region").unwrap().member("name").unwrap()), "au");
    let resources = array(out.member("resources").unwrap());
    assert_eq!(resources.len(), 2);
    let names: Vec<&str> =
        resources.iter().map(|r| text(r.member("profile").unwrap().member("name").unwrap())).collect();
    assert_eq!(names, ["AU PS Patient", "AU Core Heart Rate"]);
    for r in resources {
        assert_eq!(r.member("validation").unwrap().member("valid"), Some(&Node::Bool(true)));
        assert!(array(r.member("missing").unwrap()).is_empty());
        assert_eq!(r.member("note"), Some(&Node::Null));
    }
    let document = out.member("document").unwrap();
    assert_eq!(text(document.member("kind").unwrap()), "au-ps");
    assert_eq!(document.member("validation").unwrap().member("valid"), Some(&Node::Bool(true)));
    assert_eq!(out.member("audit"), Some(&Node::Null));

    // a region whose profile wants what the record lacks: exit 1, the gap named
    let bare = RECORD
        .replace(r#", "gender": "female", "birthDate": "1988-04-12""#, "")
        .replace(r#""title": "My health record""#, r#""title": "My health record", "document": false"#);
    let out = result(&call(&["export", "us"], bare.as_bytes()), 1);
    let patient = &array(out.member("resources").unwrap())[0];
    let missing: Vec<&str> = array(patient.member("missing").unwrap()).iter().map(text).collect();
    assert_eq!(missing, ["gender"]);
    assert_eq!(out.member("document"), Some(&Node::Null));
}

#[test]
fn an_unknown_region_and_an_export_that_cannot_start_are_usage_and_rejection() {
    let usage = call(&["export", "in"], RECORD.as_bytes());
    assert_eq!(usage.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&usage.stderr).contains("unknown export region"));
    let none = call(&["export"], RECORD.as_bytes());
    assert_eq!(none.status.code(), Some(2));
    // a document asked for with no Patient among the resources
    let no_patient =
        RECORD.replace(r#"{"resourceType": "Patient", "id": "u1","#, r#"{"resourceType": "Practitioner", "id": "u1","#);
    let rejected = call(&["export", "ips"], no_patient.as_bytes());
    assert_eq!(rejected.status.code(), Some(1), "{}", String::from_utf8_lossy(&rejected.stderr));
    assert!(String::from_utf8_lossy(&rejected.stdout).contains("document: inputs.resources must include the Patient"));
}
