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
        r#""category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs","display":"Vital Signs"}]}]"#,
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
    assert!(l.resource.contains(r#""dataAbsentReason":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/data-absent-reason","code":"unknown","display":"Unknown"}]}"#), "{}", l.resource);
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
        // lowering nothing never panics and yields a resource of the right type;
        // an extension's profile is not a resource and is refused
        if p.resource_type == "Extension" {
            assert!(lower_bytes(p.url, b"{}").is_err(), "{}", p.url);
            continue;
        }
        let l = lower_bytes(p.url, b"{}").unwrap();
        assert!(l.resource.starts_with(&format!("{{\"resourceType\":\"{}\"", p.resource_type)), "{}", l.resource);
    }
}

const Q: &str = "http://hl7.org/fhir/StructureDefinition/Questionnaire";
const QR: &str = "http://hl7.org/fhir/StructureDefinition/QuestionnaireResponse";

#[test]
fn a_questionnaire_and_its_response_lower_from_the_base_definitions() {
    // the form: items are one slot the host gives whole (a recursive
    // backbone — the nested group is the same `item` the base spec defines
    // by contentReference)
    let form = lower(
        Q,
        r#"{"url":"http://example.org/Questionnaire/intake","version":"1","status":"active","title":"Intake",
            "item":[{"linkId":"name","type":"string","required":true},
                    {"linkId":"history","type":"group","item":[{"linkId":"history.when","type":"date"}]}]}"#,
    );
    assert!(form.validation.is_valid(), "{}", wire::report(&form.validation));
    let r = &form.resource;
    assert!(r.starts_with(r#"{"resourceType":"Questionnaire","meta":{"profile":["http://hl7.org/fhir/StructureDefinition/Questionnaire"]}"#), "{r}");
    assert!(r.contains(r#""item":[{"linkId":"history.when","type":"date"}]"#), "the nested item is kept whole: {r}");
    // the answers: lowered from their slots, then held to the form the way
    // any response is (0.16.0) — the base profile declared on it changes nothing
    let v = Validator::new().with_questionnaire(form.resource.as_bytes());
    let answers = v
        .lower(
            QR,
            r#"{"questionnaire":"http://example.org/Questionnaire/intake|1","status":"completed","subject":"Patient/1",
                "authored":"2026-09-29T09:00:00+13:00",
                "item":[{"linkId":"name","answer":[{"valueString":"Alex"}]},
                        {"linkId":"history","item":[{"linkId":"history.when","answer":[{"valueDate":"2010-04-01"}]}]}]}"#
                .as_bytes(),
        )
        .unwrap();
    assert!(answers.validation.is_valid(), "{}", wire::report(&answers.validation));
    assert!(
        answers.resource.contains(r#""questionnaire":"http://example.org/Questionnaire/intake|1""#),
        "{}",
        answers.resource
    );
    // a response that skips the form's required question is invalid on the
    // way out, by the form's rule, not lowering's
    let short = v
        .lower(QR, r#"{"questionnaire":"http://example.org/Questionnaire/intake|1","status":"completed","item":[{"linkId":"history","item":[{"linkId":"history.when","answer":[{"valueDate":"2010-04-01"}]}]}]}"#.as_bytes())
        .unwrap();
    assert!(!short.validation.is_valid());
    let report = wire::report(&short.validation);
    assert!(report.contains("No response answer found for required item 'name'"), "{report}");
    // and the wrong kind for a slot is lowering's own finding
    let bad = lower(QR, r#"{"status":"completed","authored":"yesterday","item":"name=Alex"}"#);
    let report = wire::report(&bad.validation);
    assert!(
        report.contains("'authored' expects a dateTime") && report.contains("'item' expects a BackboneElement"),
        "{report}"
    );
}

/// The base profile a lowered resource declares must ask exactly what the
/// bare base check asks — including beneath the recursive `item` the base
/// defines by `contentReference`, which a profile's snapshot enumerates once.
#[test]
fn a_declared_base_profile_still_checks_what_the_base_requires_beneath_a_content_reference() {
    let errors = |v: &fhir_pods_core::Validation| -> Vec<String> {
        v.report
            .issues
            .iter()
            .filter(|i| i.severity == fhir_pods_core::validate::Severity::Error)
            .map(|i| format!("{} | {}", i.path, i.message))
            .collect()
    };
    let v = Validator::new();
    // a nested item with no linkId, and one whose type is not an item-type
    for (url, bare) in [
        (
            QR,
            r#"{"resourceType":"QuestionnaireResponse","status":"completed","item":[{"linkId":"g","item":[{"answer":[{"valueString":"x"}]}]}]}"#,
        ),
        (
            Q,
            r#"{"resourceType":"Questionnaire","status":"active","item":[{"linkId":"g","type":"group","item":[{"type":"bogus-type"}]}]}"#,
        ),
    ] {
        let without = v.validate(bare.as_bytes());
        assert!(!without.is_valid(), "{}", wire::report(&without));
        let declared = bare.replacen(r#""status""#, &format!(r#""meta":{{"profile":["{url}"]}},"status""#,), 1);
        let with = v.validate(declared.as_bytes());
        assert_eq!(errors(&with), errors(&without), "declaring {url} changed the findings");
        // and lowering, which always declares, says the same
        let inputs = bare.replacen(r#""resourceType":"QuestionnaireResponse","#, "", 1).replacen(
            r#""resourceType":"Questionnaire","#,
            "",
            1,
        );
        let lowered = v.lower(url, inputs.as_bytes()).unwrap();
        assert_eq!(errors(&lowered.validation), errors(&without), "lowering into {url} changed the findings");
    }
}

#[test]
fn an_extension_profile_is_not_a_resource_to_lower() {
    // compiled so its placement and content are checked, never lowered into as a resource
    let err = fhir_pods_core::lower_bytes(
        "http://hl7.org/fhir/StructureDefinition/patient-birthPlace",
        br#"{"valueAddress":{"city":"x"}}"#,
    )
    .expect_err("an extension profile is refused");
    assert!(matches!(err, fhir_pods_core::lower::LowerError::UnknownProfile));
    assert!(fhir_pods_core::resource_profile("http://hl7.org/fhir/StructureDefinition/patient-birthPlace").is_none());
    assert!(fhir_pods_core::resource_profile(HR).is_some());
}
