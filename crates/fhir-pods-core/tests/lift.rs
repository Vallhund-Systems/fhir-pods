mod common;

use fhir_pods_core::lift::LiftError;
use fhir_pods_core::{wire, Validator};

const HR: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";
const INPUTS: &str = r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-16T08:00:00+10:00","valueQuantity.value":72,"valueQuantity.unit":"beats/minute"}"#;

#[test]
fn a_lowered_resource_lifts_back_to_its_inputs_and_lowers_again_to_the_same_resource() {
    let v = Validator::new();
    let lowered = v.lower(HR, INPUTS.as_bytes()).unwrap();
    let lifted = v.lift(HR, lowered.resource.as_bytes()).unwrap();
    assert!(lifted.validation.is_valid(), "{}", wire::report(&lifted.validation));
    assert!(lifted.missing.is_empty() && lifted.extras.is_empty(), "{:?} {:?}", lifted.missing, lifted.extras);
    // the slots come back — the fixed parts (the codings, the category, the unit's system) do not
    let i = &lifted.inputs;
    assert!(i.contains(r#""status":"final""#) && i.contains(r#""subject":{"reference":"Patient/1"}"#), "{i}");
    assert!(i.contains(r#""effectiveDateTime":"2026-09-16T08:00:00+10:00""#), "{i}");
    assert!(i.contains(r#""valueQuantity.value":72"#) && i.contains(r#""valueQuantity.unit":"beats/minute""#), "{i}");
    assert!(!i.contains("8867-4") && !i.contains("vital-signs") && !i.contains("unitsofmeasure"), "{i}");
    assert!(!i.contains(r#""valueQuantity":{"#), "the quantity is lifted through its parts, not whole: {i}");
    // and lower the same resource again
    let again = v.lower(HR, lifted.inputs.as_bytes()).unwrap();
    assert_eq!(again.resource, lowered.resource);
}

#[test]
fn what_no_slot_covers_is_named_and_a_missing_required_slot_is_too() {
    let v = Validator::new();
    let lowered = v.lower(HR, INPUTS.as_bytes()).unwrap();
    let with_note = lowered.resource.replace(
        r#""status":"final""#,
        r#""status":"final","note":[{"text":"a note"}],"issued":"2026-09-16T08:00:00Z""#,
    );
    let lifted = v.lift(HR, with_note.as_bytes()).unwrap();
    assert!(lifted.validation.is_valid(), "{}", wire::report(&lifted.validation));
    assert_eq!(lifted.extras, vec!["issued".to_string(), "note".to_string()]);
    let without_subject = lowered.resource.replace(r#""subject":{"reference":"Patient/1"},"#, "");
    let lifted = v.lift(HR, without_subject.as_bytes()).unwrap();
    assert!(!lifted.validation.is_valid());
    assert!(lifted.missing.contains(&"subject"), "{:?}", lifted.missing);
    assert!(!lifted.inputs.contains("subject"));
}

#[test]
fn slices_lift_by_the_profiles_own_discriminators() {
    let v = Validator::new();
    let bp = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-bloodpressure";
    let inputs = r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-16T08:00:00+10:00","component:SystolicBP.valueQuantity.value":120,"component:SystolicBP.valueQuantity.unit":"mmHg","component:DiastolicBP.valueQuantity.value":80,"component:DiastolicBP.valueQuantity.unit":"mmHg"}"#;
    let lowered = v.lower(bp, inputs.as_bytes()).unwrap();
    assert!(lowered.validation.is_valid(), "{}", wire::report(&lowered.validation));
    let lifted = v.lift(bp, lowered.resource.as_bytes()).unwrap();
    assert!(lifted.inputs.contains(r#""component:SystolicBP.valueQuantity.value":120"#), "{}", lifted.inputs);
    assert!(lifted.inputs.contains(r#""component:DiastolicBP.valueQuantity.value":80"#), "{}", lifted.inputs);
    assert!(lifted.extras.is_empty(), "{:?}", lifted.extras);
    assert_eq!(v.lower(bp, lifted.inputs.as_bytes()).unwrap().resource, lowered.resource);
}

#[test]
fn the_profile_may_come_from_the_resource_itself_and_errors_are_named() {
    let v = Validator::new();
    let lowered = v.lower(HR, INPUTS.as_bytes()).unwrap();
    let by_declaration = v.lift("", lowered.resource.as_bytes()).unwrap();
    assert_eq!(by_declaration.inputs, v.lift(HR, lowered.resource.as_bytes()).unwrap().inputs);
    assert_eq!(v.lift("http://example.org/nope", lowered.resource.as_bytes()).unwrap_err(), LiftError::UnknownProfile);
    assert_eq!(
        v.lift("", br#"{"resourceType":"Observation","status":"final","code":{"text":"x"}}"#).unwrap_err(),
        LiftError::NoProfileDeclared
    );
    assert_eq!(v.lift(HR, br#"{"resourceType":"Patient"}"#).unwrap_err(), LiftError::WrongResourceType);
    assert_eq!(v.lift(HR, b"[]").unwrap_err(), LiftError::NotAResource);
    // a resource that does not declare the profile is still held to it
    let undeclared = lowered.resource.replace(&format!(r#""meta":{{"profile":["{HR}"]}},"#), "");
    let lifted = v.lift(HR, undeclared.as_bytes()).unwrap();
    assert!(lifted.validation.profiles.contains(&"AU Core Heart Rate"), "{:?}", lifted.validation.profiles);
    assert!(lifted.validation.is_valid(), "{}", wire::report(&lifted.validation));
}

/// F034 / F035: an audit record round-trips whole — its optional fixed
/// `action` comes back, and slots behind a gate nothing opened (a user that
/// was never given) are not missing.
#[test]
fn an_audit_record_lifts_and_lowers_to_itself_with_nothing_missing() {
    let v = Validator::new();
    let no_user = r#"{"recorded":"2026-09-16T10:00:00Z","client":{"who":"Device/app-7","network":"10.0.0.7"},"server":{"who":{"display":"fhir.example.org"},"network":"https://fhir.example.org/r4"},"source":{"observer":"Device/audit"},"data":"Observation/hr1"}"#;
    for kind in ["create", "read", "update"] {
        let a = v.audit(kind, no_user.as_bytes()).unwrap();
        assert!(a.validation.is_valid(), "{kind}: {}", wire::report(&a.validation));
        assert!(a.resource.contains(r#""action":""#), "{kind}: {}", a.resource);
        let lifted = v.lift("", a.resource.as_bytes()).unwrap();
        assert!(lifted.missing.is_empty(), "{kind}: missing {:?}\n{}", lifted.missing, lifted.inputs);
        assert!(lifted.extras.is_empty(), "{kind}: extras {:?}", lifted.extras);
        assert!(lifted.inputs.contains(r#""action":""#), "{kind}: the fixed action is lifted: {}", lifted.inputs);
        let profile = a.resource.split(r#""profile":[""#).nth(1).unwrap().split('"').next().unwrap();
        let again = v.lower(profile, lifted.inputs.as_bytes()).unwrap();
        assert!(again.resource.contains(r#""action":""#), "{kind}: the action survives lowering: {}", again.resource);
        assert!(again.validation.is_valid(), "{kind}: {}", wire::report(&again.validation));
    }
}
