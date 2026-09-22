//! Independent T050 regression cases: compare content and identity coverage,
//! not just validation of the output.
use fhir_pods_core::export::ExportError;
use fhir_pods_core::node::Node;
use fhir_pods_core::{json, Validator};

const BASE: &str = "http://hl7.org/fhir/StructureDefinition/";
const AU_HR: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";

fn parse(text: &str) -> Node {
    Node::from_value(&json::parse(text.as_bytes()).unwrap())
}
fn patient(v: &Validator) -> Node {
    parse(&v.lower(&format!("{BASE}Patient"), br#"{"id":"u1","identifier":[{"system":"https://example.org/mrn","value":"synthetic"}],"name":[{"family":"Example"}],"gender":"female","birthDate":"1988-04-12"}"#).unwrap().resource)
}
fn heart_rate(v: &Validator) -> Node {
    parse(&v.lower(&format!("{BASE}heartrate"), br#"{"id":"hr1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T08:00:00+10:00","valueQuantity.value":72}"#).unwrap().resource)
}
fn inputs(resources: &[Node]) -> Vec<u8> {
    Node::Object(vec![("resources".into(), Node::Array(resources.to_vec())), ("document".into(), Node::Bool(false))])
        .to_json()
        .into_bytes()
}
fn put(node: &mut Node, key: &str, value: Node) {
    *node.member_or_insert(key, Node::Null) = value;
}

#[test]
fn quantity_qualifiers_extensions_and_fixed_primitive_carriers_are_preserved_or_named() {
    let v = Validator::new();
    let mut hr = heart_rate(&v);
    let quantity = hr.member_mut("valueQuantity").unwrap();
    put(quantity, "comparator", Node::Str("<".into()));
    put(
        quantity,
        "extension",
        parse(r#"[{"url":"https://example.org/precision","valueString":"synthetic-precision"}]"#),
    );
    put(
        quantity,
        "_code",
        parse(r#"{"extension":[{"url":"https://example.org/precision","valueString":"synthetic-carrier"}]}"#),
    );
    assert!(v.validate(hr.to_json().as_bytes()).is_valid());
    let lifted = v.lift(AU_HR, hr.to_json().as_bytes()).unwrap();
    for path in ["valueQuantity.comparator", "valueQuantity.extension", "valueQuantity._code"] {
        assert!(lifted.extras.iter().any(|p| p == path), "unextracted {path}: {:?}", lifted.extras);
    }
    for region in ["au", "us", "ips"] {
        let result = v.export(region, &inputs(&[patient(&v), hr.clone()])).unwrap();
        let output = &result.resources[1];
        let actual = parse(&output.resource);
        let Node::Object(expected) = hr.member("valueQuantity").unwrap() else { panic!() };
        let Node::Object(actual_quantity) = actual.member("valueQuantity").unwrap() else { panic!() };
        assert_eq!(actual_quantity.len(), expected.len());
        for (key, value) in expected {
            assert_eq!(actual.member("valueQuantity").unwrap().member(key), Some(value), "{region}: {key}");
        }
        assert!(output.validation.is_valid(), "{region}: {:?}", output.validation);
        assert!(output.extras.is_empty(), "{region}: {:?}", output.extras);
        for path in &lifted.extras {
            assert!(output.carried.contains(path), "{region}: {path} not reported as carried");
        }
    }
}

#[test]
fn an_uncovered_primitive_and_its_carrier_are_copied_once() {
    let v = Validator::new();
    let mut hr = heart_rate(&v);
    put(&mut hr, "issued", Node::Str("2026-09-20T08:01:00Z".into()));
    let carrier = parse(r#"{"extension":[{"url":"https://example.org/precision","valueString":"synthetic-issued"}]}"#);
    put(&mut hr, "_issued", carrier.clone());
    let result = v.export("au", &inputs(&[patient(&v), hr.clone()])).unwrap();
    let resource = &result.resources[1].resource;
    let actual = parse(resource);
    assert_eq!(actual.member("issued"), hr.member("issued"));
    assert_eq!(actual.member("_issued"), Some(&carrier));
    assert_eq!(resource.matches("\"_issued\":").count(), 1);
    assert!(result.resources[1].validation.is_valid());
}

#[test]
fn qualifiers_beneath_rebuilt_slices_are_never_silently_lost() {
    let v = Validator::new();
    let mut bp = parse(&v.lower(&format!("{BASE}bp"), br#"{"id":"bp1","status":"final","subject":"Patient/u1","effectiveDateTime":"2026-09-20T08:00:00+10:00","component:SystolicBP.valueQuantity.value":120,"component:DiastolicBP.valueQuantity.value":80}"#).unwrap().resource);
    let Node::Array(components) = bp.member_mut("component").unwrap() else { panic!() };
    put(components[0].member_mut("valueQuantity").unwrap(), "comparator", Node::Str("<".into()));
    assert!(v.validate(bp.to_json().as_bytes()).is_valid());
    for region in ["au", "us", "ips"] {
        let result = v.export(region, &inputs(&[patient(&v), bp.clone()])).unwrap();
        let output = &result.resources[1];
        assert!(
            output.extras.iter().any(|p| p == "component.valueQuantity.comparator"),
            "{region}: {:?}",
            output.extras
        );
        assert!(!output.carried.iter().any(|p| p.contains("comparator")));
    }
}

#[test]
fn a_conflicting_fixed_code_refuses_reprofiling_without_changing_the_measurement() {
    let v = Validator::new();
    let mut hr = heart_rate(&v);
    let quantity = hr.member_mut("valueQuantity").unwrap();
    put(quantity, "code", Node::Str("Hz".into()));
    put(quantity, "unit", Node::Str("Hz".into()));
    let lifted = v.lift(AU_HR, hr.to_json().as_bytes()).unwrap();
    assert!(lifted.conflicts.iter().any(|c| c.path == "valueQuantity.code" && c.given == "Hz" && c.fixed == "/min"));
    for region in ["au", "us", "ips"] {
        let result = v.export(region, &inputs(&[patient(&v), hr.clone()])).unwrap();
        let output = &result.resources[1];
        assert!(output.profile.is_none());
        assert_eq!(parse(&output.resource), hr);
        assert!(output.note.as_deref().unwrap().contains("not re-profiled"));
    }
}

#[test]
fn patient_references_distinguish_versions_foreign_ids_and_unresolved_authorities() {
    let v = Validator::new();
    for region in ["au", "us", "ips"] {
        for reference in ["Patient/u1", "Patient/u1/_history/7"] {
            let mut hr = heart_rate(&v);
            put(hr.member_mut("subject").unwrap(), "reference", Node::Str(reference.into()));
            let output = v.export(region, &inputs(&[patient(&v), hr])).unwrap();
            assert!(!output.resources[1].notes.iter().any(|n| n.contains("patient identity")), "{region}: {reference}");
        }
        for reference in [
            "Patient/OTHER",
            "Patient/OTHER/_history/7",
            "https://records.example/r4/Patient/OTHER",
            "https://records.example/r4/Patient/OTHER/_history/7",
        ] {
            let mut hr = heart_rate(&v);
            put(hr.member_mut("subject").unwrap(), "reference", Node::Str(reference.into()));
            assert_eq!(
                v.export(region, &inputs(&[patient(&v), hr])).unwrap_err(),
                ExportError::NotThisPatient(1, reference.into())
            );
        }
        for reference in [
            "https://records.example/r4/Patient/u1",
            "https://records.example/r4/Patient/u1/_history/7",
            "urn:uuid:aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
            "#local-patient",
        ] {
            let mut hr = heart_rate(&v);
            put(hr.member_mut("subject").unwrap(), "reference", Node::Str(reference.into()));
            let output = v.export(region, &inputs(&[patient(&v), hr.clone()])).unwrap();
            assert!(
                output.resources[1]
                    .notes
                    .iter()
                    .any(|n| n.contains("patient identity not established") && n.contains(reference)),
                "{region}: {reference}"
            );
            assert_eq!(parse(&output.resources[1].resource).member("subject"), hr.member("subject"));
        }
    }
}

#[test]
fn missing_record_identity_and_logical_references_have_explicit_coverage_notes() {
    let v = Validator::new();
    let hr = heart_rate(&v);
    let output = v.export("au", &inputs(std::slice::from_ref(&hr))).unwrap();
    assert!(output.resources[0].notes.iter().any(|n| n.contains("patient identity not established")));
    let mut p = patient(&v);
    let Node::Object(members) = &mut p else { panic!() };
    members.retain(|(k, _)| k != "id");
    let output = v.export("au", &inputs(&[p, hr.clone()])).unwrap();
    assert!(output.resources[1].notes.iter().any(|n| n.contains("patient identity not established")));
    let mut logical = hr;
    put(
        &mut logical,
        "subject",
        parse(r#"{"identifier":{"system":"https://example.org/patient-id","value":"synthetic"}}"#),
    );
    let output = v.export("au", &inputs(&[patient(&v), logical])).unwrap();
    assert!(output.resources[1].notes.iter().any(|n| n.contains("patient identity not established")));
}
