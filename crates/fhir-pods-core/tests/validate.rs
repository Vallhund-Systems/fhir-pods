mod common;
use common::*;

#[test]
fn t043_versioned_document_references_select_the_requested_entry_version() {
    let document = r#"{"resourceType":"Bundle","type":"document",
      "identifier":{"system":"https://example.org/docs","value":"test"},"timestamp":"2026-09-18T00:00:00Z",
      "entry":[
        {"fullUrl":"https://example.org/Composition/c","resource":{"resourceType":"Composition","id":"c","status":"final","type":{"text":"Test"},"date":"2026-09-18T00:00:00Z","author":[{"reference":"Practitioner/a"}],"title":"Test","section":[{"entry":[{"reference":"Observation/o/_history/1"},{"reference":"https://example.org/Observation/o/_history/2"}]}]}},
        {"fullUrl":"https://example.org/Observation/o","resource":{"resourceType":"Observation","id":"o","meta":{"versionId":"1"},"status":"final","code":{"text":"Test"}}},
        {"fullUrl":"https://example.org/Observation/o","resource":{"resourceType":"Observation","id":"o","meta":{"versionId":"2"},"status":"final","code":{"text":"Test"}}},
        {"fullUrl":"https://example.org/Practitioner/a","resource":{"resourceType":"Practitioner","id":"a"}}
      ]}"#;
    let valid = probe_str(document);
    assert!(valid.contains("\"valid\":true"), "{valid}");
    let missing = probe_str(&document.replace("/_history/2", "/_history/3"));
    assert!(missing.contains("/_history/3' is not in the bundle"), "{missing}");
    let malformed = probe_str(&document.replace("/_history/2", "/_history/2/extra"));
    assert!(malformed.contains("/_history/2/extra' is not in the bundle"), "{malformed}");
    let no_version = probe_str(&document.replace(r#""versionId":"2""#, r#""versionId":"3""#));
    assert!(no_version.contains("/_history/2' is not in the bundle"), "{no_version}");
}

#[test]
fn t043_non_http_bundle_identities_are_not_rest_resource_urls() {
    let bundle = r#"{"resourceType":"Bundle","type":"collection","entry":[{"fullUrl":"uin:uuid:example","resource":{"resourceType":"Patient","id":"p"}}]}"#;
    let valid = probe_str(bundle);
    assert!(valid.contains("\"valid\":true"), "{valid}");
    let wrong = probe_str(&bundle.replace("uin:uuid:example", "https://example.org/Patient/other"));
    assert!(wrong.contains("does not end in the resource's type and id (Patient/p)"), "{wrong}");
}

#[test]
fn t043_primitive_array_placeholders_need_content_at_the_same_index() {
    let ok = r#"{"resourceType":"Patient","name":[{"given":["a",null],"_given":[null,{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/data-absent-reason","valueCode":"unknown"}]}]}]}"#;
    assert!(validate(ok).is_valid(), "{}", probe_str(ok));
    for names in [
        r#"{"given":["a"],"_given":[{"id":"test"},null]}"#,
        r#"{"given":["a",null],"_given":[{"id":"test"}]}"#,
        r#"{"given":[null],"_given":[null]}"#,
    ] {
        let patient = format!(r#"{{"resourceType":"Patient","name":[{names}]}}"#);
        assert!(!validate(&patient).is_valid(), "{patient}");
    }
}

#[test]
fn t043_resource_id_cannot_carry_extensions() {
    let patient = r#"{"resourceType":"Patient","id":"p","_id":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/data-absent-reason","valueCode":"unknown"}]}}"#;
    let report = probe_str(patient);
    assert!(report.contains("Resource.id cannot carry extensions"), "{report}");
}

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
    // a real string element (Patient has no `note`; an unknown element is now an error)
    let r = probe_str(&PATIENT_OK.replace("\"family\":\"Doe\"", "\"family\":\"a } \\\" tricky\""));
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
fn non_profiled_resource_has_no_profile_but_owes_the_base_resource_its_required_elements() {
    let r = probe_str(
        r#"{"resourceType":"Encounter","status":"finished","class":{"system":"http://terminology.hl7.org/CodeSystem/v3-ActCode","code":"AMB"}}"#,
    );
    assert!(r.contains("\"valid\":true") && !r.contains("\"profile\""), "{r}");
    // the base resource's own required elements, with no profile to restate them
    let r = probe_str(r#"{"resourceType":"Encounter","status":"finished"}"#);
    assert!(r.contains("\"valid\":false") && r.contains("Encounter.class is required"), "{r}");
    // one key per choice element, profile or no profile
    let r = probe_str(
        r#"{"resourceType":"Observation","status":"final","code":{"text":"x"},"valueString":"a","valueInteger":1}"#,
    );
    assert!(r.contains("Observation.value[x] allows one choice; found valueString, valueInteger"), "{r}");
    // dom-3: a contained resource nothing refers to
    let r = probe_str(
        r#"{"resourceType":"Encounter","status":"finished","class":{"code":"AMB"},"contained":[{"resourceType":"Patient","id":"p"}]}"#,
    );
    assert!(r.contains("dom-3"), "{r}");
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
    // the most specific base profile applies; the generic vital-signs one it derives from is carried by it
    assert_eq!(r.profiles, vec!["Observation Body Weight Profile"], "{text}");
    assert!(text.contains("vs-1") && text.contains("allows the types"), "{text}");
    // a code that is not a vital sign implies nothing
    let other = obs.replace("29463-7", "2339-0").replace("vital-signs", "laboratory");
    let r = fhir_pods_core::validate_bytes(other.as_bytes());
    assert!(r.profiles.is_empty(), "{:?}", r.profiles);
}

// --- the v0.5.1 review's findings, pinned ---

#[test]
fn f010_resources_nested_in_a_bundle_are_held_to_their_own_declared_profiles() {
    // an AU Core Patient without an identifier, inside a collection with no profile of its own
    let bad = r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},"name":[{"family":"x"}],"gender":"male","birthDate":"1970"}"#;
    let bundle = format!(
        r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"resource":{{"resourceType":"Basic","code":{{"text":"x"}}}}}},{{"resource":{bad}}}]}}"#
    );
    let r = probe_str(&bundle);
    assert!(r.contains("\"valid\":false"), "{r}");
    assert!(r.contains("\"path\":\"Bundle.entry[1].resource.Patient.identifier\""), "{r}");
    // the same profile inside `contained`, with the outer resource unprofiled
    let contained = format!(
        r##"{{"resourceType":"Encounter","status":"finished","class":{{"code":"AMB"}},"contained":[{bad}],"subject":{{"reference":"#x"}}}}"##
    );
    let r = probe_str(&contained.replace(r#""name":[{"family":"x"}]"#, r#""id":"x","name":[{"family":"x"}]"#));
    assert!(r.contains("Encounter.contained[0].Patient.identifier"), "{r}");
    // and two levels down: a Parameters resource carrying the bundle
    let params = format!(r#"{{"resourceType":"Parameters","parameter":[{{"name":"b","resource":{bundle}}}]}}"#);
    let r = probe_str(&params);
    assert!(r.contains("Parameters.parameter[0].resource.Bundle.entry[1].resource.Patient.identifier"), "{r}");
}

#[test]
fn f011_a_declared_profile_for_another_resource_type_is_an_error_not_silence() {
    let r = probe_str(
        r#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate"]},"gender":"male"}"#,
    );
    assert!(r.contains("\"valid\":false") && r.contains("applies to Observation, not to a Patient"), "{r}");
}

#[test]
fn f012_dates_and_times_are_held_to_the_full_grammar() {
    let obs = |eff: &str| {
        format!(
            r#"{{"resourceType":"Observation","status":"final","code":{{"text":"x"}},"effectiveDateTime":"{eff}"}}"#
        )
    };
    for (value, ok) in [
        ("2026-09-16T10:00:00+10:00", true),
        ("2026-09-16T10:00+10:00", false), // seconds are required once there is a time
        ("2026-09-16T10:00:00", false),    // and so is a zone
        ("2026-09-16T10:00:00+15:00", false), // offsets stop at 14:00
        ("2026-09-16T10:00:00+10:60", false),
        ("2026-09-16T10:00:00-14:00", true),
        ("0000-09-16", false), // year 0 is outside the grammar
        ("2026-09", true),
    ] {
        let r = probe_str(&obs(value));
        assert_eq!(r.contains("\"valid\":true"), ok, "{value}: {r}");
    }
    let obs_time = |t: &str| {
        format!(r#"{{"resourceType":"Observation","status":"final","code":{{"text":"x"}},"valueTime":"{t}"}}"#)
    };
    assert!(probe_str(&obs_time("10:00:00")).contains("\"valid\":true"));
    assert!(probe_str(&obs_time("10:00:00Z")).contains("\"valid\":false"), "a time has no zone");
    assert!(probe_str(&obs_time("10:00")).contains("\"valid\":false"), "a time has seconds");
    let inst =
        |t: &str| format!(r#"{{"resourceType":"Observation","status":"final","code":{{"text":"x"}},"issued":"{t}"}}"#);
    assert!(probe_str(&inst("2026-09-16T10:00:00Z")).contains("\"valid\":true"));
    assert!(probe_str(&inst("10:00:00Z")).contains("\"valid\":false"), "an instant has a date");
}

#[test]
fn f026_an_unprofiled_bundle_owes_its_identity_and_the_base_bundle_rules() {
    let entry = |full: &str, id: &str, extra: &str| {
        format!(r#"{{"fullUrl":"{full}","resource":{{"resourceType":"Patient","id":"{id}","gender":"male"{extra}}}}}"#)
    };
    let bundle = |kind: &str, top: &str, entries: &[String]| {
        format!(r#"{{"resourceType":"Bundle","type":"{kind}"{top},"entry":[{}]}}"#, entries.join(","))
    };
    let ok = bundle("collection", "", &[entry("https://x.example.org/fhir/Patient/p1", "p1", "")]);
    assert!(probe_str(&ok).contains("\"valid\":true"), "{}", probe_str(&ok));
    let r = probe_str(&bundle("collection", "", &[entry("https://x.example.org/fhir/Patient/other", "p1", "")]));
    assert!(r.contains("does not end in the resource's type and id (Patient/p1)"), "{r}");
    let r = probe_str(&bundle("collection", "", &[entry("https://x.example.org/fhir/Observation/p1", "p1", "")]));
    assert!(r.contains("\"valid\":false"), "{r}");
    // a urn is its own identity
    let r = probe_str(&bundle("collection", "", &[entry("urn:uuid:00000000-0000-4000-8000-000000000001", "p1", "")]));
    assert!(r.contains("\"valid\":true"), "{r}");
    // bdl-7: the same fullUrl twice, unless the versions differ
    let dup = entry("https://x.example.org/fhir/Patient/p1", "p1", "");
    let r = probe_str(&bundle("collection", "", &[dup.clone(), dup.clone()]));
    assert!(r.contains("bdl-7"), "{r}");
    let v1 = entry("https://x.example.org/fhir/Patient/p1", "p1", r#","meta":{"versionId":"1"}"#);
    let v2 = entry("https://x.example.org/fhir/Patient/p1", "p1", r#","meta":{"versionId":"2"}"#);
    assert!(probe_str(&bundle("collection", "", &[v1, v2])).contains("\"valid\":true"));
    // bdl-1: total only on a searchset or history; bdl-3: a transaction entry needs its request
    assert!(probe_str(&bundle("collection", r#","total":1"#, std::slice::from_ref(&dup))).contains("bdl-1"));
    assert!(probe_str(&bundle("transaction", "", std::slice::from_ref(&dup))).contains("bdl-3"));
    let with_request =
        dup.replace(r#""gender":"male"}}"#, r#""gender":"male"},"request":{"method":"PUT","url":"Patient/p1"}}"#);
    assert!(probe_str(&bundle("transaction", "", &[with_request])).contains("\"valid\":true"));
}

#[test]
fn an_extension_sits_where_its_definition_allows() {
    let bt = r#"{"url":"http://hl7.org/fhir/StructureDefinition/patient-birthTime","valueDateTime":"1970-06-15T10:30:00+10:00"}"#;
    // on the birth date's carrier: allowed
    let ok = format!(r#"{{"resourceType":"Patient","birthDate":"1970-06-15","_birthDate":{{"extension":[{bt}]}}}}"#);
    assert!(probe_str(&ok).contains("\"valid\":true"), "{}", probe_str(&ok));
    // on the resource itself, or on another element: not allowed, as the reference validator has it
    let root = format!(r#"{{"resourceType":"Patient","birthDate":"1970-06-15","extension":[{bt}]}}"#);
    let r = probe_str(&root);
    assert!(
        r.contains("\"valid\":false")
            && r.contains("The extension http://hl7.org/fhir/StructureDefinition/patient-birthTime is not allowed to be used at this point (this element is [Element, Patient, Resource, DomainResource]; allowed = [Patient.birthDate, Person.birthDate, RelatedPerson.birthDate, Practitioner.birthDate])"),
        "{r}"
    );
    let choice = format!(
        r#"{{"resourceType":"Patient","deceasedDateTime":"2026-01-01","_deceasedDateTime":{{"extension":[{bt}]}}}}"#
    );
    let r = probe_str(&choice);
    assert!(
        r.contains("\"valid\":false") && r.contains("this element is [Element, dateTime, Patient.deceased[x]]"),
        "{r}"
    );
    // a recursive backbone: the owner answers to its definition path, cycles collapsed (F051)
    let ordinal = r##"{"resourceType":"Questionnaire","status":"active","item":[{"linkId":"1","type":"group","item":[{"linkId":"1.1","type":"choice","answerOption":[{"valueCoding":{"code":"a"},"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/ordinalValue","valueDecimal":1}]}]}]}]}"##;
    assert!(probe_str(ordinal).contains("\"valid\":true"), "{}", probe_str(ordinal));
    let misplaced = ordinal.replace(r#""answerOption":[{"valueCoding":{"code":"a"},"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/ordinalValue","valueDecimal":1}]}]"#, r#""answerOption":[{"valueCoding":{"code":"a"}}],"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/ordinalValue","valueDecimal":1}]"#);
    assert!(
        probe_str(&misplaced)
            .contains("this element is [Element, BackboneElement, Questionnaire.item.item, Questionnaire.item]"),
        "{}",
        probe_str(&misplaced)
    );
    // inside a Bundle entry the owner is judged from the nested resource, not the Bundle
    let nested = format!(
        r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"fullUrl":"urn:uuid:5b8e1672-0000-4000-8000-000000000001","resource":{ok}}}]}}"#
    );
    assert!(probe_str(&nested).contains("\"valid\":true"), "{}", probe_str(&nested));
}

#[test]
fn a_canonical_is_absolute_and_a_narrative_says_something() {
    // canonical: absolute, or a fragment; a relative one is an error, as the reference validator has it
    let abs = r##"{"resourceType":"ActivityDefinition","status":"active","library":["http://example.org/fhir/Library/zika|1.0"]}"##;
    assert!(probe_str(abs).contains("\"valid\":true"), "{}", probe_str(abs));
    let frag = abs.replace("http://example.org/fhir/Library/zika|1.0", "#lib");
    assert!(probe_str(&frag).contains("\"valid\":true"), "{}", probe_str(&frag));
    let rel = abs.replace("http://example.org/fhir/Library/zika|1.0", "Library/zika");
    let r = probe_str(&rel);
    assert!(r.contains("\"valid\":false") && r.contains("library is not a valid canonical"), "{r}");
    // a uri may be relative
    let uri = r##"{"resourceType":"Basic","code":{"coding":[{"system":"codes/local","code":"x"}]}}"##;
    assert!(probe_str(uri).contains("\"valid\":true"), "{}", probe_str(uri));
    // txt-2: a narrative of whitespace, tags and comments only is an error; one with text is fine
    let empty = r##"{"resourceType":"Basic","text":{"status":"generated","div":"<div xmlns=\"http://www.w3.org/1999/xhtml\"> <p> </p><!-- x --> &nbsp;</div>"},"code":{"text":"x"}}"##;
    let r = probe_str(empty);
    assert!(
        r.contains("\"valid\":false") && r.contains("txt-2: Basic.text.div must have some non-whitespace content"),
        "{r}"
    );
    let said = empty.replace("<p> </p>", "<p>Something</p>");
    assert!(probe_str(&said).contains("\"valid\":true"), "{}", probe_str(&said));
    let entity = empty.replace("<p> </p>", "<p>&amp;</p>");
    assert!(probe_str(&entity).contains("\"valid\":true"), "{}", probe_str(&entity));
    let image = empty.replace("<p> </p>", "<img src=\\\"#a\\\" alt=\\\"diagram\\\"/>");
    assert!(probe_str(&image).contains("\"valid\":true"), "{}", probe_str(&image));
}

#[test]
fn a_coding_in_a_base_code_system_is_a_known_code() {
    let ok = r##"{"resourceType":"AdverseEvent","actuality":"actual","subject":{"reference":"Patient/p1"},"category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/adverse-event-category","code":"wrong-patient"}]}]}"##;
    assert!(probe_str(ok).contains("\"valid\":true"), "{}", probe_str(ok));
    // R4's own copy of the system has other codes than the terminology package's: either version's code is known
    let older = ok.replace("wrong-patient", "product-use-error");
    assert!(probe_str(&older).contains("\"valid\":true"), "{}", probe_str(&older));
    let unknown = ok.replace("wrong-patient", "wrong-planet");
    let r = probe_str(&unknown);
    assert!(
        r.contains("\"valid\":false")
            && r.contains("Unknown code 'wrong-planet' in the CodeSystem 'http://terminology.hl7.org/CodeSystem/adverse-event-category'"),
        "{r}"
    );
    // a quantity's coded unit is a code in a system too
    let med = |unit: &str| {
        format!(
            r##"{{"resourceType":"Medication","ingredient":[{{"itemCodeableConcept":{{"text":"x"}},"strength":{{"numerator":{{"value":1,"system":"http://unitsofmeasure.org","code":"mg"}},"denominator":{{"value":1,"system":"http://terminology.hl7.org/CodeSystem/v3-orderableDrugForm","code":"{unit}"}}}}}}]}}"##
        )
    };
    assert!(probe_str(&med("TAB")).contains("\"valid\":true"), "{}", probe_str(&med("TAB")));
    assert!(probe_str(&med("Tab")).contains("Unknown code 'Tab'"), "{}", probe_str(&med("Tab")));
    // a system whose caseSensitive is absent accepts any case (v2-0136 in R4 core says nothing)
    let yn = |code: &str| {
        format!(
            r##"{{"resourceType":"Basic","code":{{"coding":[{{"system":"http://terminology.hl7.org/CodeSystem/v2-0136","code":"{code}"}}]}}}}"##
        )
    };
    assert!(probe_str(&yn("y")).contains("\"valid\":true"), "{}", probe_str(&yn("y")));
    assert!(probe_str(&yn("Y")).contains("\"valid\":true"), "{}", probe_str(&yn("Y")));
    assert!(probe_str(&yn("maybe")).contains("Unknown code 'maybe'"), "{}", probe_str(&yn("maybe")));
    // a code carrying a line break is lexically a code and matches no concept
    assert!(probe_str(&yn("N\\nY")).contains("Unknown code"), "{}", probe_str(&yn("N\\nY")));
    // a code the newer terminology package added (observation-category `symptom`, THO 7.x)
    let sym = ok.replace("adverse-event-category", "observation-category").replace("wrong-patient", "symptom");
    assert!(probe_str(&sym).contains("\"valid\":true"), "{}", probe_str(&sym));
    // the "code system" of URIs
    let uri = |code: &str| {
        format!(
            r##"{{"resourceType":"Basic","code":{{"coding":[{{"system":"urn:ietf:rfc:3986","code":"{code}"}}]}}}}"##
        )
    };
    assert!(probe_str(&uri("http://example.org/x")).contains("\"valid\":true"));
    assert!(probe_str(&uri("RicardianContract"))
        .contains("Unknown code 'RicardianContract' in the CodeSystem 'urn:ietf:rfc:3986'"));
    // a system the pod does not carry says nothing about the code
    let foreign = ok
        .replace("http://terminology.hl7.org/CodeSystem/adverse-event-category", "http://example.org/local")
        .replace("wrong-patient", "anything");
    assert!(probe_str(&foreign).contains("\"valid\":true"), "{}", probe_str(&foreign));
    // a version on the system is not part of its identity
    let versioned = ok.replace("adverse-event-category\"", "adverse-event-category|1.0.0\"");
    assert!(probe_str(&versioned).contains("\"valid\":true"), "{}", probe_str(&versioned));
}

#[test]
fn an_extension_context_is_matched_the_three_ways_the_reference_validator_matches_it() {
    // the path from the nearest enclosing datatype's definition (`Dosage.site`), not the owner's own type
    let dosage = r##"{"resourceType":"MedicationRequest","status":"active","intent":"order","medicationCodeableConcept":{"text":"x"},"subject":{"reference":"Patient/1"},"dosageInstruction":[{"site":{"text":"arm","extension":[{"url":"http://hl7.org/fhir/StructureDefinition/bodySite","valueReference":{"reference":"BodyStructure/1"}}]}}]}"##;
    assert!(probe_str(dosage).contains("\"valid\":true"), "{}", probe_str(dosage));
    // the literal path, a content reference uncollapsed (`ValueSet.expansion.contains.designation`)
    let vs = r##"{"resourceType":"ValueSet","status":"active","expansion":{"timestamp":"2020-01-01T00:00:00Z","contains":[{"system":"http://snomed.info/sct","code":"1","display":"d","designation":[{"value":"v","extension":[{"url":"http://hl7.org/fhir/StructureDefinition/coding-sctdescid","valueId":"123"}]}]}]}}"##;
    assert!(probe_str(vs).contains("\"valid\":true"), "{}", probe_str(vs));
    // the literal path through a datatype's own backbone (`StructureDefinition.differential.element.binding.valueSet`)
    let sd = r##"{"resourceType":"StructureDefinition","url":"http://example.org/sd/x","name":"X","status":"draft","kind":"resource","abstract":false,"type":"Patient","baseDefinition":"http://hl7.org/fhir/StructureDefinition/Patient","derivation":"constraint","differential":{"element":[{"id":"Patient.gender","path":"Patient.gender","binding":{"strength":"required","valueSet":"http://hl7.org/fhir/ValueSet/administrative-gender","_valueSet":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/elementdefinition-conceptmap","valueCanonical":"http://example.org/ConceptMap/cm"}]}}}]}}"##;
    assert!(probe_str(sd).contains("\"valid\":true"), "{}", probe_str(sd));
    // a choice spelled the R5 way in the context (`Parameters.parameter.value`)
    let params = r##"{"resourceType":"Parameters","parameter":[{"name":"a","valueString":"s","_valueString":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/cqf-isEmptyList","valueBoolean":true}]}}]}"##;
    assert!(probe_str(params).contains("\"valid\":true"), "{}", probe_str(params));
    // and a wrong place is still a wrong place
    let wrong = dosage
        .replace("bodySite", "patient-birthTime")
        .replace(r#""valueReference":{"reference":"BodyStructure/1"}"#, r#""valueDateTime":"2020-01-01T00:00:00Z""#);
    assert!(
        probe_str(&wrong).contains("patient-birthTime is not allowed to be used at this point"),
        "{}",
        probe_str(&wrong)
    );
}

#[test]
fn base64_oids_and_uuids_are_well_formed() {
    let ok = r##"{"resourceType":"DocumentReference","status":"current","content":[{"attachment":{"contentType":"text/plain","data":"aGVsbG8="}}]}"##;
    assert!(probe_str(ok).contains("\"valid\":true"), "{}", probe_str(ok));
    // Binary.data is the content itself: not held to the lexical check, as the reference validator has it
    let binary = r##"{"resourceType":"Binary","contentType":"text/plain","data":"aGVs bG8"}"##;
    assert!(probe_str(binary).contains("\"valid\":true"), "{}", probe_str(binary));
    let space = ok.replace("aGVsbG8=", "aGVs bG8=");
    assert!(probe_str(&space).contains("data is not a valid base64Binary"), "{}", probe_str(&space));
    let short = ok.replace("aGVsbG8=", "aGVsbG8");
    assert!(probe_str(&short).contains("data is not a valid base64Binary"), "{}", probe_str(&short));
    let wrapped = ok.replace("aGVsbG8=", "aGVs\\r\\nbG8=");
    assert!(probe_str(&wrapped).contains("\"valid\":true"), "{}", probe_str(&wrapped));
    let sys = |s: &str| format!(r##"{{"resourceType":"Basic","code":{{"coding":[{{"system":"{s}","code":"x"}}]}}}}"##);
    assert!(probe_str(&sys("urn:oid:2.16.840.1.113883.4.1")).contains("\"valid\":true"));
    for bad in [
        "urn:oid:2.16.840.01",
        "urn:oid:3.1",
        "urn:oid:2",
        "urn:uuid:5B8E1672-0000-4000-8000-000000000001",
        "urn:uuid:not-a-uuid",
    ] {
        assert!(probe_str(&sys(bad)).contains("system is not a valid uri"), "{bad}: {}", probe_str(&sys(bad)));
    }
    assert!(probe_str(&sys("urn:uuid:5b8e1672-0000-4000-8000-000000000001")).contains("\"valid\":true"));
}

#[test]
fn an_identifier_under_rfc_3986_carries_a_uri() {
    let ok =
        r##"{"resourceType":"Organization","identifier":[{"system":"urn:ietf:rfc:3986","value":"urn:oid:1.2.3"}]}"##;
    assert!(probe_str(ok).contains("\"valid\":true"), "{}", probe_str(ok));
    let bare = ok.replace("urn:oid:1.2.3", "1.2.3");
    let r = probe_str(&bare);
    assert!(
        r.contains("\"valid\":false")
            && r.contains("if identifier.system is 'urn:ietf:rfc:3986', then the identifier.value must be a full URI (e.g. start with a scheme), not '1.2.3'"),
        "{r}"
    );
    // any other system takes the value as given
    let other = ok.replace("urn:ietf:rfc:3986", "http://example.org/ids").replace("urn:oid:1.2.3", "1.2.3");
    assert!(probe_str(&other).contains("\"valid\":true"), "{}", probe_str(&other));
}

#[test]
fn a_local_reference_names_a_contained_resource_or_is_ref_1() {
    let with = r##"{"resourceType":"MedicationRequest","status":"active","intent":"order","contained":[{"resourceType":"Medication","id":"med1","code":{"text":"x"}}],"medicationReference":{"reference":"#med1"},"subject":{"reference":"Patient/p1"}}"##;
    assert!(probe_str(with).contains("\"valid\":true"), "{}", probe_str(with));
    let without = with.replace(r#""contained":[{"resourceType":"Medication","id":"med1","code":{"text":"x"}}],"#, "");
    let r = probe_str(&without);
    assert!(r.contains("\"valid\":false") && r.contains("ref-1: reference '#med1' names no contained resource"), "{r}");
    // a contained resource may name a sibling of the resource that contains it
    let sibling = r##"{"resourceType":"MedicationRequest","status":"active","intent":"order","contained":[{"resourceType":"Medication","id":"m","code":{"text":"x"}},{"resourceType":"Practitioner","id":"p","name":[{"family":"D"}]}],"medicationReference":{"reference":"#m"},"requester":{"reference":"#p"},"subject":{"reference":"Patient/p1"}}"##;
    assert!(probe_str(sibling).contains("\"valid\":true"), "{}", probe_str(sibling));
}

#[test]
fn an_empty_extension_carrier_is_an_element_with_nothing_in_it() {
    let r = probe_str(r#"{"resourceType":"Patient","gender":"male","_gender":{}}"#);
    assert!(r.contains("\"valid\":false") && r.contains("extension carrier must have some content"), "{r}");
}

#[test]
fn f013_an_extension_whose_value_arrives_through_its_carrier_satisfies_ext_1() {
    let r = probe_str(
        r#"{"resourceType":"Patient","gender":"male","extension":[{"url":"http://example.org/x","_valueString":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/data-absent-reason","valueCode":"unknown"}]}}]}"#,
    );
    assert!(!r.contains("ext-1"), "{r}");
    // but an extension with neither is still ext-1
    let r = probe_str(r#"{"resourceType":"Patient","gender":"male","extension":[{"url":"http://example.org/x"}]}"#);
    assert!(r.contains("ext-1"), "{r}");
}

/// F033: an entry that carries a resource names it with a fullUrl, except a
/// transaction's or batch's create.
#[test]
fn a_bundle_entry_with_a_resource_carries_a_full_url() {
    let no_url =
        r#"{"resourceType":"Bundle","type":"collection","entry":[{"resource":{"resourceType":"Patient","id":"p1"}}]}"#;
    let r = probe_str(no_url);
    assert!(r.contains("\"valid\":false") && r.contains("Bundle entry missing fullUrl"), "{r}");
    let with_url = no_url.replace(r#"{"resource":"#, r#"{"fullUrl":"http://example.org/fhir/Patient/p1","resource":"#);
    assert!(probe_str(&with_url).contains("\"valid\":true"), "{}", probe_str(&with_url));
    let create = r#"{"resourceType":"Bundle","type":"transaction","entry":[{"resource":{"resourceType":"Patient"},"request":{"method":"POST","url":"Patient"}}]}"#;
    assert!(probe_str(create).contains("\"valid\":true"), "{}", probe_str(create));
    let put = create
        .replace(r#""method":"POST","url":"Patient""#, r#""method":"PUT","url":"Patient/p1""#)
        .replace(r#"{"resourceType":"Patient"}"#, r#"{"resourceType":"Patient","id":"p1"}"#);
    assert!(probe_str(&put).contains("Bundle entry missing fullUrl"), "{}", probe_str(&put));
    // a response's entries answer requests: no identity of their own needed
    let response = r#"{"resourceType":"Bundle","type":"batch-response","entry":[{"resource":{"resourceType":"OperationOutcome","issue":[{"severity":"information","code":"informational"}]},"response":{"status":"200 OK"}}]}"#;
    assert!(probe_str(response).contains("\"valid\":true"), "{}", probe_str(response));
}

#[test]
fn ids_and_codes_are_lexically_well_formed() {
    let with_id = |id: &str| format!(r##"{{"resourceType":"Patient","id":"{id}"}}"##);
    assert!(probe_str(&with_id("ok.1-2")).contains("\"valid\":true"));
    assert!(probe_str(&with_id("p_bad id!")).contains("is not a valid id"));
    assert!(probe_str(&with_id(&"a".repeat(65))).contains("is not a valid id"));
    assert!(probe_str(&with_id(&"a".repeat(64))).contains("\"valid\":true"));
    let with_code = |code: &str| {
        format!(
            r##"{{"resourceType":"Basic","code":{{"coding":[{{"system":"http://example.org/x","code":"{code}"}}]}}}}"##
        )
    };
    assert!(probe_str(&with_code("a b")).contains("\"valid\":true"));
    for bad in [" a", "a ", "a  b", "a\\u00a0"] {
        assert!(probe_str(&with_code(bad)).contains("is not a valid code"), "{bad:?}: {}", probe_str(&with_code(bad)));
    }
}
