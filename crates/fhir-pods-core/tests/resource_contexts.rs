use fhir_pods_core::{
    terminology::{Bundled, Chain, Expansions, Recorder},
    terminology_context::ResourceContexts,
    wire, Validator,
};
const SET: &str = "https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1";
const SYSTEM: &str = "https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1";
fn table(code: &str) -> String {
    format!("fhir-pods-terminology 1\n= {SET}\n{SYSTEM}\t{code}\n")
}
fn quoted(text: &str) -> String {
    let mut s = String::from("\"");
    wire::escape(&mut s, text);
    s.push('"');
    s
}
fn install() -> String {
    format!(
        r#"{{"format":"fhir-pods.terminology-contexts.v1","contexts":[{{"terminology":{},"resources":[{{"resourceType":"Patient","id":"one"}}]}},{{"terminology":{},"resources":[{{"resourceType":"Patient","id":"two"}}]}}]}}"#,
        quoted(&table("AAA")),
        quoted(&table("BBB"))
    )
}
fn patient(id: &str, code: &str) -> String {
    format!(
        r#"{{"resourceType":"Patient","id":"{id}","meta":{{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]}},"identifier":[{{"system":"https://example.invalid/mrn","value":"{id}"}}],"name":[{{"family":"Synthetic"}}],"gender":"unknown","birthDate":"1970-01-01","_birthDate":{{"extension":[{{"url":"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator","valueCoding":{{"system":"{SYSTEM}","code":"{code}"}}}}]}}}}"#
    )
}
fn collection(resources: &[String]) -> String {
    format!(
        r#"{{"resourceType":"Bundle","type":"collection","entry":[{}]}}"#,
        resources
            .iter()
            .enumerate()
            .map(|(i, r)| format!(r#"{{"fullUrl":"urn:uuid:00000000-0000-4000-8000-{i:012}","resource":{r}}}"#))
            .collect::<Vec<_>>()
            .join(",")
    )
}
#[test]
fn same_profile_uses_each_resources_original_table_in_full_and_nested_collections() {
    let contexts = ResourceContexts::parse(install().as_bytes()).unwrap();
    let global = Expansions::parse(&table("BBB")).unwrap();
    let tx = Chain(&[&Bundled, &contexts, &global]);
    let v = Validator::new();
    for (id, own, other) in [("one", "AAA", "BBB"), ("two", "BBB", "AAA")] {
        let good = v.validate_with(patient(id, own).as_bytes(), &tx);
        assert!(good.is_valid(), "{good:?}");
        assert!(!v.validate_with(patient(id, other).as_bytes(), &tx).is_valid());
    }
    let good = collection(&[patient("one", "AAA"), patient("two", "BBB")]);
    assert!(v.validate_with(good.as_bytes(), &tx).is_valid());
    let bad = collection(&[patient("one", "BBB"), patient("two", "AAA")]);
    assert!(!v.validate_with(bad.as_bytes(), &tx).is_valid());
    let nested = collection(&[format!(r#"{{"id":"inner",{}"#, &bad[1..])]);
    assert!(!v.validate_with(nested.as_bytes(), &tx).is_valid());
}
#[test]
fn reference_target_conformance_switches_to_targets_owner_and_shape() {
    let config = install().replace(
        r#""resourceType":"Patient","id":"one"}"#,
        r#""resourceType":"Patient","id":"one"},{"resourceType":"RelatedPerson","id":"relative"}"#,
    );
    let contexts = ResourceContexts::parse(config.as_bytes()).unwrap();
    let tx = Chain(&[&Bundled, &contexts]);
    let v = Validator::new();
    let related=r#"{"resourceType":"RelatedPerson","id":"relative","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-relatedperson"]},"name":[{"family":"Synthetic"}],"patient":{"reference":"Patient/two"}}"#.to_string();
    let good = v.validate_with(collection(&[patient("two", "BBB"), related.clone()]).as_bytes(), &tx);
    assert!(good.is_valid(), "{good:?}");
    let bad = v.validate_with(collection(&[patient("two", "AAA"), related]).as_bytes(), &tx);
    assert!(!bad.is_valid());
    assert!(bad.report.issues.iter().any(|i| i.id == "reference.target-profile"), "{bad:?}");
}
#[test]
fn unknown_scoped_sets_record_needs_without_borrowing_global_membership() {
    let empty = install().replace(&quoted(&table("AAA")), &quoted("fhir-pods-terminology 1\n"));
    let contexts = ResourceContexts::parse(empty.as_bytes()).unwrap();
    let global = Expansions::parse(&table("AAA")).unwrap();
    let recorder = Recorder::new();
    let tx = Chain(&[&Bundled, &contexts, &global, &recorder]);
    let result = Validator::new().validate_with(patient("one", "AAA").as_bytes(), &tx);
    assert!(result.report.issues.iter().any(|i| i.id == "binding.unknown"), "{result:?}");
    assert_eq!(recorder.needs().len(), 1);
}
#[test]
fn unbound_resources_preserve_the_configured_provider() {
    let contexts = ResourceContexts::parse(install().as_bytes()).unwrap();
    let global = Expansions::parse(&table("CCC")).unwrap();
    let tx = Chain(&[&Bundled, &contexts, &global]);
    assert!(Validator::new().validate_with(patient("other", "CCC").as_bytes(), &tx).is_valid());
}
#[test]
fn malformed_or_ambiguous_ownership_is_refused() {
    for bad in [
        install().replace("\"id\":\"two\"", "\"id\":\"one\""),
        install().replace("\"id\":\"one\"", "\"id\":1"),
        install().replace("\"Patient\"", "true"),
        install().replace("\"Patient\"", "\"Bogus\""),
        install().replace("\"id\":\"one\"", "\"id\":\"bad/id\""),
        install().replace("\"terminology\":", "\"extra\":true,\"terminology\":"),
        install().replace("fhir-pods-terminology 1", "invalid table"),
        install().replace("\"id\":\"one\"", "\"id\":\"one\",\"id\":\"other\""),
    ] {
        assert!(ResourceContexts::parse(bad.as_bytes()).is_none(), "{bad}");
    }
    assert!(ResourceContexts::parse(br#"{"format":"fhir-pods.terminology-contexts.v1","contexts":[]}"#).is_some());
}
#[test]
fn repeated_scoped_resource_identity_is_not_a_unique_owner() {
    let contexts = ResourceContexts::parse(install().as_bytes()).unwrap();
    let tx = Chain(&[&Bundled, &contexts]);
    let input = collection(&[patient("one", "AAA"), patient("one", "AAA")]);
    let result = Validator::new().validate_with(input.as_bytes(), &tx);
    assert!(result.report.issues.iter().any(|i| i.id == "terminology.context-ambiguous"), "{result:?}");
}

#[test]
fn contained_ids_can_repeat_in_fhir_but_not_under_a_shared_operational_key() {
    let nested = |id: &str| {
        format!(
            r##"{{"resourceType":"Basic","id":"{id}","code":{{"text":"Synthetic"}},"subject":{{"reference":"#one"}},"contained":[{}]}}"##,
            patient("one", "AAA")
        )
    };
    let input = collection(&[nested("parent-one"), nested("parent-two")]);
    let global = Expansions::parse(&table("AAA")).unwrap();
    let baseline = Validator::new().validate_with(input.as_bytes(), &Chain(&[&Bundled, &global]));
    assert!(baseline.is_valid(), "{baseline:?}");
    let contexts = ResourceContexts::parse(install().as_bytes()).unwrap();
    let result = Validator::new().validate_with(input.as_bytes(), &Chain(&[&Bundled, &contexts, &global]));
    assert!(result.report.issues.iter().any(|i| i.id == "terminology.context-ambiguous"), "{result:?}");
}

#[test]
fn extensions_without_an_owner_profile_retain_the_containing_resources_context() {
    let contexts = ResourceContexts::parse(install().as_bytes()).unwrap();
    let global = Expansions::parse(&table("CCC")).unwrap();
    let tx = Chain(&[&Bundled, &contexts, &global]);
    let bare = |id, code| {
        patient(id, code)
            .replace(r#","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]}"#, "")
    };
    let good = collection(&[bare("one", "AAA"), bare("two", "BBB")]);
    let result = Validator::new().validate_with(good.as_bytes(), &tx);
    assert!(result.is_valid(), "{result:?}");
    let bad = collection(&[bare("one", "BBB"), bare("two", "AAA")]);
    let result = Validator::new().validate_with(bad.as_bytes(), &tx);
    assert!(!result.is_valid(), "{result:?}");
    assert!(result.report.issues.iter().any(|i| i.id == "binding.not-member"));
}
