mod common;
use common::*;

// --- no declared profile: the base resource applies, which we do not compile ---

#[test]
fn unprofiled_resource_is_valid_and_claims_no_profile() {
    let r = probe_str(
        r#"{"resourceType":"Observation","status":"final","code":{"text":"HR"},"valueQuantity":{"value":60}}"#,
    );
    assert!(r.contains("\"valid\":true") && !r.contains("\"profile\""), "{r}");
    // a validator may carry a jurisdiction default for a resource type
    let v = fhir_pods_core::Validator::new()
        .with_default_profile("http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient");
    let bare = br#"{"resourceType":"Patient","active":true}"#;
    assert!(fhir_pods_core::wire::report(&v.validate(bare)).contains("Patient.identifier is required"));
    assert!(fhir_pods_core::wire::report(&fhir_pods_core::validate_bytes(bare)).contains("\"valid\":true"));
    // an unknown default URL is ignored, never guessed
    let v2 = fhir_pods_core::Validator::new().with_default_profile("http://example.org/nope");
    assert!(fhir_pods_core::wire::report(&v2.validate(bare)).contains("\"valid\":true"));
}

#[test]
fn versioned_canonical_selects_the_profile() {
    let r = probe_str(&PATIENT_OK.replace("au-core-patient\"", "au-core-patient|2.0.0\""));
    assert!(r.contains("\"profile\":\"AU Core Patient\""), "{r}");
}

#[test]
fn resource_type_must_be_a_string() {
    let r = probe_str(r#"{"resourceType":5}"#);
    assert!(r.contains("resourceType must be a string") && r.contains("\"valid\":false"), "{r}");
}

#[test]
fn tricky_string_does_not_end_object_early() {
    let r = probe_str(&PATIENT_OK.replace("\"gender\"", "\"note\":\"a } \\\" tricky\",\"gender\""));
    assert!(r.contains("\"valid\":true"), "{r}");
}

// --- Patient (generated au-core-patient) ---

#[test]
fn valid_patient() {
    let r = probe_str(PATIENT_OK);
    assert!(r.contains("\"valid\":true"), "{r}");
    assert!(r.contains("AU Core Patient"), "{r}");
}

#[test]
fn patient_missing_mandatory_is_error() {
    let r = probe_str(
        r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"active":true}"#,
    );
    assert!(r.contains("Patient.identifier is required"), "{r}");
    assert!(r.contains("Patient.name is required"), "{r}");
    assert!(r.contains("Patient.gender is required"), "{r}");
    assert!(r.contains("Patient.birthDate is required"), "{r}");
}

#[test]
fn patient_bad_gender_fails_required_binding() {
    let r = probe_str(&PATIENT_OK.replace("\"male\"", "\"banana\""));
    assert!(r.contains("not in the bound value set"), "{r}");
    assert!(r.contains("\"code\":\"code-invalid\""), "{r}");
}

#[test]
fn primitive_with_only_extension_sibling_counts_as_present() {
    // FHIR JSON: a primitive carrying only extensions is `_birthDate` with no `birthDate`.
    let r = probe_str(&PATIENT_OK.replace(
        "\"birthDate\":\"1970-01-01\"",
        "\"_birthDate\":{\"extension\":[{\"url\":\"http://hl7.org/fhir/StructureDefinition/data-absent-reason\",\"valueCode\":\"unknown\"}]}",
    ));
    assert!(r.contains("\"valid\":true"), "{r}");
    assert!(!r.contains("birthDate is required"), "{r}");
}

#[test]
fn nested_binding_on_name_use() {
    let r = probe_str(&PATIENT_OK.replace("\"official\"", "\"bogus\""));
    assert!(r.contains("Patient.name.use"), "{r}");
}

#[test]
fn nested_min_fires_per_parent_instance() {
    let r = probe_str(&PATIENT_OK.replace(
        "\"birthDate\"",
        "\"communication\":[{\"language\":{\"text\":\"en\"}},{\"preferred\":true}],\"birthDate\"",
    ));
    assert_eq!(r.matches("Patient.communication.language is required").count(), 1, "{r}");
}

#[test]
fn nested_min_does_not_fire_when_parent_absent() {
    let r = probe_str(PATIENT_OK);
    assert!(!r.contains("communication.language"), "{r}");
}

#[test]
fn nested_max_exceeded_per_parent() {
    let r = probe_str(&PATIENT_OK.replace(
        "\"birthDate\"",
        "\"communication\":[{\"language\":[{\"text\":\"en\"},{\"text\":\"fr\"}]}],\"birthDate\"",
    ));
    assert!(r.contains("Patient.communication.language exceeds max 1"), "{r}");
}

#[test]
fn link_type_binding_nested() {
    let bad = PATIENT_OK.replace(
        "\"birthDate\"",
        "\"link\":[{\"other\":{\"reference\":\"Patient/2\"},\"type\":\"nope\"}],\"birthDate\"",
    );
    assert!(probe_str(&bad).contains("Patient.link.type"));
}

#[test]
fn identifier_slice_inherits_au_ihi_profile() {
    // An NI-typed identifier is the `ihi` slice, whose profile (au-ihi)
    // fixes the system — reached through the slice's profile, recursively,
    // with the issue relabelled onto the slice.
    let ni = |system: &str| {
        PATIENT_OK.replace(
            PATIENT_ID,
            &format!(
                r#""identifier":[{{"type":{{"coding":[{{"system":"http://terminology.hl7.org/CodeSystem/v2-0203","code":"NI"}}]}},"system":"{system}","value":"8003608833357361"}}]"#
            ),
        )
    };
    let wrong = probe_str(&ni("urn:oid:1.2.36.146.595.217.0.1"));
    assert!(wrong.contains("Patient.identifier:ihi.system"), "{wrong}");
    assert!(wrong.contains("\"valid\":false"), "{wrong}");
    let right = probe_str(&ni("http://ns.electronichealth.net.au/id/hi/ihi/1.0"));
    assert!(right.contains("\"valid\":true"), "{right}");
}

#[test]
fn a_pattern_discriminator_needs_one_coding_carrying_every_leaf() {
    // MR under v2-0203 plus a *local* NI: no single coding is the IHI pattern
    // (v2-0203 + NI), so this is not the `ihi` slice and au-ihi must not fire.
    let split = PATIENT_OK.replace(
        PATIENT_ID,
        r#""identifier":[{"type":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/v2-0203","code":"MR"},{"system":"http://example.org/local","code":"NI"}]},"system":"http://hospital.example.org/mrn","value":"1"}]"#,
    );
    let r = probe_str(&split);
    assert!(!r.contains("identifier:ihi"), "{r}");
    assert!(r.contains("\"valid\":true"), "{r}");
}

// --- Observation: generated au-core-heartrate, selected via meta.profile ---

#[test]
fn heartrate_valid_via_declared_profile() {
    let r = probe_str(HEARTRATE_OK);
    assert!(r.contains("\"valid\":true"), "{r}");
    assert!(r.contains("Heart"), "{r}");
}

#[test]
fn heartrate_missing_snomed_slice_is_error() {
    let r = probe_str(&HEARTRATE_OK.replace(r#",{"system":"http://snomed.info/sct","code":"364075005"}"#, ""));
    assert!(r.contains("Observation.code.coding:snomedHeartRateCode is required"), "{r}");
}

#[test]
fn heartrate_wrong_ucum_code_fails_slice_fixed() {
    let r = probe_str(&HEARTRATE_OK.replace("\"code\":\"/min\"", "\"code\":\"min\""));
    assert!(r.contains("Observation.value[x]:valueQuantity.code must be '/min'"), "{r}");
}

#[test]
fn heartrate_closed_value_slicing_rejects_other_type() {
    let r = probe_str(&HEARTRATE_OK.replace(
        r#""valueQuantity":{"value":72,"unit":"beats/minute","system":"http://unitsofmeasure.org","code":"/min"}"#,
        r#""valueString":"72""#,
    ));
    assert!(r.contains("matching no slice"), "{r}");
}

// --- Observation: generated au-core-bloodpressure — component slices whose
//     discriminator values live inside nested slices ---

#[test]
fn bloodpressure_valid_with_component_slices() {
    let r = probe_str(BP_OK);
    assert!(r.contains("\"valid\":true"), "{r}");
    assert!(r.contains("Blood Pressure"), "{r}");
}

#[test]
fn bloodpressure_wrong_diastolic_code_fails_component_slice() {
    // Two components remain, so `component min 2` still holds — only the
    // slice discriminator (resolved through a nested slice) can catch it.
    let r = probe_str(&BP_OK.replace("\"code\":\"8462-4\"", "\"code\":\"8462-9\""));
    assert!(r.contains("Observation.component:DiastolicBP is required"), "{r}");
}

#[test]
fn bloodpressure_component_wrong_unit_fails_slice_fixed() {
    let r = probe_str(&BP_OK.replacen("\"code\":\"mm[Hg]\"", "\"code\":\"mmHg\"", 1));
    assert!(r.contains("Observation.component:SystolicBP.value[x].code must be 'mm[Hg]'"), "{r}");
}

// --- several declared profiles ---

#[test]
fn every_declared_profile_applies() {
    let both = PATIENT_OK.replace(
        "\"resourceType\":\"Patient\",",
        "\"resourceType\":\"Patient\",\"meta\":{\"profile\":[\"http://hl7.org/fhir/us/core/StructureDefinition/us-core-patient\",\"http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient\"]},",
    );
    let r = probe_str(&both);
    assert!(r.contains("\"profile\":\"US Core Patient Profile\""), "{r}");
    assert!(r.contains("\"profiles\":[\"US Core Patient Profile\",\"AU Core Patient\"]"), "{r}");
    // an AU-only finding shows up alongside the US ones
    let gender_bad = probe_str(&both.replace("\"male\"", "\"banana\""));
    assert_eq!(
        gender_bad.matches("is not in the bound value set").count(),
        1,
        "shared finding reported once: {gender_bad}"
    );
    let single = probe_str(PATIENT_OK);
    assert!(!single.contains("\"profiles\""), "{single}");
}

// --- must-support (information, not conformance) ---

#[test]
fn must_support_gaps_are_reported_where_the_parent_is_present() {
    use fhir_pods_core::must_support_gaps;
    let us = r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org/fhir/us/core/StructureDefinition/us-core-patient"]},"identifier":[{"system":"http://h.example.org/mrn","value":"1"}],"name":[{"family":"D"}],"gender":"male"}"#;
    let gaps = must_support_gaps(us.as_bytes());
    // absent must-support elements at the top level… (US Core flags
    // telecom's children MS but not telecom itself, so it is not a gap)
    assert!(!gaps.contains(&"Patient.telecom"), "{gaps:?}");
    assert!(gaps.contains(&"Patient.birthDate"), "{gaps:?}");
    assert!(gaps.contains(&"Patient.address"), "{gaps:?}");
    // …and under a present parent — but not under an absent one
    assert!(gaps.contains(&"Patient.name.given"), "{gaps:?}");
    assert!(!gaps.contains(&"Patient.address.line"), "{gaps:?}");
    assert!(!gaps.contains(&"Patient.telecom.system"), "{gaps:?}");
    // populated ones are not gaps
    assert!(!gaps.contains(&"Patient.identifier.value"), "{gaps:?}");
    // it is not validation: the same resource validates independently
    assert!(must_support_gaps(b"not json").is_empty());
    assert!(must_support_gaps(br#"{"resourceType":"Encounter"}"#).is_empty());
}

// --- shared ---

#[test]
fn non_profiled_resource_has_no_profile() {
    let r = probe_str(r#"{"resourceType":"Encounter","status":"finished"}"#);
    assert!(r.contains("\"valid\":true"), "{r}");
}

#[test]
fn garbage_reports_a_parse_error() {
    let r = probe_str("not json");
    assert!(r.contains("could not parse"), "{r}");
}

#[test]
fn slice_scoped_invariants_run_on_the_slice_only_through_both_entry_points() {
    // US Core Organization: identifier:NPI must be 10 digits (us-core-16), identifier:CLIA 10 with a D third (us-core-18)
    let org = |npi: &str, clia: &str| {
        format!(
            r#"{{"resourceType":"Organization","meta":{{"profile":["http://hl7.org/fhir/us/core/StructureDefinition/us-core-organization"]}},"identifier":[{{"system":"http://hl7.org/fhir/sid/us-npi","value":"{npi}"}},{{"system":"urn:oid:2.16.840.1.113883.4.7","value":"{clia}"}},{{"system":"http://example.org/other","value":"x"}}],"active":true,"name":"Acme"}}"#
        )
    };
    let bad = org("123", "12X4567890");
    for report in [
        fhir_pods_core::wire::report(&fhir_pods_core::validate_bytes(bad.as_bytes())),
        fhir_pods_core::wire::report(&fhir_pods_core::Validator::new().validate(bad.as_bytes())),
    ] {
        assert!(report.contains("\"path\":\"Organization.identifier:NPI\",\"message\":\"us-core-16"), "{report}");
        assert!(report.contains("\"path\":\"Organization.identifier:CLIA\",\"message\":\"us-core-18"), "{report}");
        // the third identifier belongs to no slice: nothing scoped to a slice fires on it
        assert!(!report.contains("Organization.identifier\",\"message\":\"us-core-1"), "{report}");
    }
    let ok = org("1234567893", "12D4567890");
    let report = fhir_pods_core::wire::report(&fhir_pods_core::Validator::new().validate(ok.as_bytes()));
    assert!(!report.contains("us-core-16") && !report.contains("us-core-18"), "{report}");
}

#[test]
fn an_observation_with_a_vital_sign_loinc_code_is_held_to_the_base_profile() {
    // no declared profile, LOINC 29463-7: the base bodyweight profile applies (spec rule) — a
    // valueString is not what it allows, and a year-only date fails vs-1
    let obs = r#"{"resourceType":"Observation","status":"final","category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs"}]}],"code":{"coding":[{"system":"http://loinc.org","code":"29463-7"}]},"subject":{"reference":"Patient/1"},"effectiveDateTime":"2026","valueString":"heavy"}"#;
    let r = fhir_pods_core::validate_bytes(obs.as_bytes());
    let text = fhir_pods_core::wire::report(&r);
    assert!(
        r.profiles.iter().any(|p| p.contains("Body Weight")) && r.profiles.iter().any(|p| p.contains("Vital Signs")),
        "{text}"
    );
    assert!(text.contains("vs-1") && text.contains("allows the types"), "{text}");
    // a code that is not a vital sign implies nothing
    let other = obs.replace("29463-7", "2339-0").replace("vital-signs", "laboratory");
    let r = fhir_pods_core::validate_bytes(other.as_bytes());
    assert!(r.profiles.is_empty(), "{:?}", r.profiles);
}
