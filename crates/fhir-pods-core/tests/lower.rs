mod common;

use fhir_pods_core::{lower_bytes, wire, Validator};

const HR: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";
const BP: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-bloodpressure";
const PT: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient";
const WEIGHT: &str = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-bodyweight";

fn lower(url: &str, inputs: &str) -> fhir_pods_core::lower::Lowered {
    Validator::new().lower(url, inputs.as_bytes()).expect("lowerable")
}

#[test]
fn a_heart_rate_from_four_inputs_is_a_conformant_observation() {
    let l = lower(
        HR,
        r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-16T08:00:00+10:00","valueQuantity.value":72,"valueQuantity.unit":"beats/minute"}"#,
    );
    assert!(l.validation.is_valid(), "{}", wire::report(&l.validation));
    let r = &l.resource;
    // the profile's skeleton is all there
    for expected in [
        r#""resourceType":"Observation""#,
        r#""meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate"]}"#,
        r#""category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs"}]}]"#,
        r#"{"system":"http://loinc.org","code":"8867-4"}"#,
        r#"{"system":"http://snomed.info/sct","code":"364075005"}"#,
        r#""subject":{"reference":"Patient/1"}"#,
        r#""valueQuantity":{"value":72,"unit":"beats/minute","system":"http://unitsofmeasure.org","code":"/min"}"#,
    ] {
        assert!(r.contains(expected), "missing {expected} in {r}");
    }
    // and the result re-validates through the ordinary path
    assert!(fhir_pods_core::validate_bytes(r.as_bytes()).is_valid());
}

#[test]
fn blood_pressure_places_each_component_in_its_own_slice() {
    let l = lower(
        BP,
        r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-16T08:00:00+10:00",
            "component:SystolicBP.valueQuantity.value":120,"component:SystolicBP.valueQuantity.unit":"mmHg",
            "component:DiastolicBP.valueQuantity.value":80,"component:DiastolicBP.valueQuantity.unit":"mmHg"}"#,
    );
    assert!(l.validation.is_valid(), "{}", wire::report(&l.validation));
    let r = &l.resource;
    assert!(r.contains(r#""code":"8480-6"}"#) && r.contains(r#""code":"8462-4"}"#), "{r}");
    assert!(
        r.contains(
            r#""valueQuantity":{"value":120,"unit":"mmHg","system":"http://unitsofmeasure.org","code":"mm[Hg]"}"#
        ),
        "{r}"
    );
    assert_eq!(r.matches(r#""code":"mm[Hg]""#).count(), 2, "{r}");
    // no value[x] on the panel itself (the profile forbids it)
    assert!(!r.contains(r#""valueQuantity":{"system"#), "{r}");
}

#[test]
fn a_missing_required_input_is_reported_not_guessed() {
    let l =
        lower(HR, r#"{"status":"final","subject":"Patient/1","valueQuantity.value":72,"valueQuantity.unit":"/min"}"#);
    assert!(!l.validation.is_valid());
    let r = wire::report(&l.validation);
    assert!(r.contains("Observation.effective[x] is required"), "{r}");
}

#[test]
fn an_absent_value_leaves_the_optional_type_slice_out() {
    // dataAbsentReason instead of a value: no dangling valueQuantity with only a unit
    let l = lower(
        HR,
        r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-16","dataAbsentReason":{"system":"http://terminology.hl7.org/CodeSystem/data-absent-reason","code":"unknown"}}"#,
    );
    assert!(l.validation.is_valid(), "{}", wire::report(&l.validation));
    assert!(!l.resource.contains("valueQuantity"), "{}", l.resource);
    assert!(l.resource.contains(r#""dataAbsentReason":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/data-absent-reason","code":"unknown"}]}"#));
}

#[test]
fn unknown_keys_wrong_kinds_and_fixed_collisions_are_issues() {
    let l = lower(
        HR,
        r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"not a date","valueQuantity.value":"72","valueQuantity.unit":"x","heartRate":72}"#,
    );
    let r = wire::report(&l.validation);
    assert!(r.contains("'heartRate' is not an input of AU Core Heart Rate"), "{r}");
    assert!(r.contains("'effectiveDateTime' expects a dateTime"), "{r}");
    assert!(r.contains("'valueQuantity.value' expects a decimal"), "{r}");
    // a host object that disagrees with a fixed leaf
    let l = lower(
        WEIGHT,
        r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-16","valueQuantity":{"value":70,"unit":"kg","system":"http://example.org/units","code":"kg"}}"#,
    );
    let r = wire::report(&l.validation);
    assert!(r.contains("'valueQuantity.system' is fixed to 'http://unitsofmeasure.org'"), "{r}");
}

#[test]
fn a_patient_from_datatype_objects_and_an_optional_slice() {
    let l = lower(
        PT,
        r#"{"identifier":{"system":"http://hospital.example.org/mrn","value":"12345"},
            "name":{"family":"Doe","given":["Jane"],"use":"official"},"gender":"female","birthDate":"1970-01-01",
            "identifier:ihi":{"value":"8003608833357361"}}"#,
    );
    assert!(l.validation.is_valid(), "{}", wire::report(&l.validation));
    let r = &l.resource;
    // the optional IHI slice came with au-ihi's fixed system and NI type
    assert!(r.contains(r#""system":"http://ns.electronichealth.net.au/id/hi/ihi/1.0""#), "{r}");
    assert!(r.contains(r#""code":"NI""#), "{r}");
    assert!(r.contains(r#""value":"8003608833357361""#), "{r}");
    // repeating slots given singly are wrapped
    assert!(r.contains(r#""name":[{"family":"Doe""#), "{r}");
    // a bad IHI is caught on the way out — lowering never trusts itself
    let bad = lower(
        PT,
        r#"{"identifier":{"system":"http://hospital.example.org/mrn","value":"1"},"name":{"family":"D"},"gender":"male","birthDate":"1970-01-01","identifier:ihi":{"value":"8003608833357362"}}"#,
    );
    assert!(wire::report(&bad.validation).contains("Luhn"));
}

#[test]
fn unknown_profile_and_non_object_inputs_are_errors_not_resources() {
    use fhir_pods_core::lower::LowerError;
    assert_eq!(lower_bytes("http://example.org/nope", b"{}").unwrap_err(), LowerError::UnknownProfile);
    assert_eq!(lower_bytes(HR, b"[1]").unwrap_err(), LowerError::InputsNotAnObject);
    assert_eq!(lower_bytes(HR, b"not json").unwrap_err(), LowerError::InputsNotAnObject);
    // a versioned URL is fine
    assert!(lower_bytes(&format!("{HR}|2.0.0"), b"{}").is_ok());
}

#[test]
fn every_registry_profile_exposes_its_slots() {
    for p in fhir_pods_core::generated::REGISTRY {
        for s in p.lowering.slots {
            assert!(!s.path.is_empty() && !s.ty.is_empty(), "{} {:?}", p.url, s);
            assert!(!s.path.contains("[x]") || s.ty == "choice", "{} {}", p.url, s.path);
        }
        // lowering nothing never panics and yields a resource of the right type
        let l = lower_bytes(p.url, b"{}").unwrap();
        assert!(l.resource.starts_with(&format!("{{\"resourceType\":\"{}\"", p.resource_type)), "{}", l.resource);
    }
}
