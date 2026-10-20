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
    assert!(
        wrong.contains("looks like a RESTful server URL, so it must end with the correct type and id (/Patient/p)"),
        "{wrong}"
    );
    // a URL that does not look RESTful (no `Type/id` tail) is not judged — the
    // reference validator's own regex decides what looks RESTful
    let plain = probe_str(&bundle.replace("uin:uuid:example", "https://example.org/some-entry"));
    assert!(plain.contains("\"valid\":true"), "{plain}");
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
    // HumanName.use's required binding is the datatype's own: checked by the
    // shape walk at the indexed element, as validator_cli locates it
    assert!(
        r.contains("\"path\":\"Patient.name[0].use\"") && r.contains("('bogus') was not found in the value set"),
        "{r}"
    );
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
    assert!(
        r.contains("looks like a RESTful server URL, so it must end with the correct type and id (/Patient/p1)"),
        "{r}"
    );
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
    let empty = r##"{"resourceType":"Basic","text":{"status":"generated","div":"<div xmlns=\"http://www.w3.org/1999/xhtml\"> <p> </p><!-- x --> </div>"},"code":{"text":"x"}}"##;
    let r = probe_str(empty);
    assert!(
        r.contains("\"valid\":false") && r.contains("txt-2: The narrative SHALL have some non-whitespace content"),
        "{r}"
    );
    let said = empty.replace("<p> </p>", "<p>Something</p>");
    assert!(probe_str(&said).contains("\"valid\":true"), "{}", probe_str(&said));
    let entity = empty.replace("<p> </p>", "<p>&amp;</p>");
    assert!(probe_str(&entity).contains("\"valid\":true"), "{}", probe_str(&entity));
    let image = empty.replace("<p> </p>", "<img src=\\\"http://example.org/a.png\\\" alt=\\\"diagram\\\"/>");
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

#[test]
fn f060_extension_only_date_is_checked_by_the_computable_invariant() {
    let carrier = r#""_effectiveDateTime":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/data-absent-reason","valueCode":"unknown"}]}"#;
    let empty = HEARTRATE_OK.replace(r#""effectiveDateTime":"2026-09-15T10:00:00Z""#, carrier);
    let report = validate(&empty);
    assert!(!report.is_valid(), "{}", probe_str(&empty));
    let issues = &report.report.issues;
    assert!(issues.iter().any(|i| i.code == fhir_pods_core::validate::IssueType::Invariant
        && i.severity == fhir_pods_core::validate::Severity::Error
        && i.message.starts_with("vs-1:")));
    assert!(!issues.iter().any(|i| i.message.contains("vs-1 not evaluated")));
    let present = HEARTRATE_OK.replace(
        r#""effectiveDateTime":"2026-09-15T10:00:00Z""#,
        &format!(r#""effectiveDateTime":"2026-09-15T10:00:00Z",{carrier}"#),
    );
    assert!(validate(&present).is_valid(), "{}", probe_str(&present));
    assert!(validate(HEARTRATE_OK).is_valid());
    let period = HEARTRATE_OK
        .replace(
            "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate",
            "http://hl7.org/fhir/StructureDefinition/heartrate",
        )
        .replace(
            r#""effectiveDateTime":"2026-09-15T10:00:00Z""#,
            r#""effectivePeriod":{"start":"2026-09-15T10:00:00Z"}"#,
        );
    assert!(validate(&period).is_valid(), "{}", probe_str(&period));
    let cached = fhir_pods_core::Validator::new();
    assert!(cached.validate(period.as_bytes()).is_valid());
    assert!(!cached.validate(empty.as_bytes()).is_valid());
    let plain = r#"{"resourceType":"Patient","_birthDate":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/data-absent-reason","valueCode":"unknown"}]}}"#;
    assert!(validate(plain).is_valid(), "{}", probe_str(plain));
}

#[test]
fn f066_extension_parent_context_checks_the_canonical_and_primitive_value() {
    let alias = r#"{"url":"http://hl7.org/fhir/StructureDefinition/cqf-libraryAlias","valueString":"test"}"#;
    let parent = "http://hl7.org/fhir/StructureDefinition/cqf-library";
    let make = |url: &str, placement: &str| {
        format!(
            r#"{{"resourceType":"Patient","extension":[{{"url":"{url}","valueCanonical":"https://example.org/Library/test",{placement}}}]}}"#
        )
    };
    let right = make(parent, &format!(r#""_valueCanonical":{{"extension":[{alias}]}}"#));
    assert!(validate(&right).is_valid(), "{}", probe_str(&right));
    for url in ["https://example.org/other-parent", "http://hl7.org/fhir/StructureDefinition/cqf-library-other"] {
        for placement in
            [format!(r#""_valueCanonical":{{"extension":[{alias}]}}"#), format!(r#""extension":[{alias}]"#)]
        {
            let wrong = make(url, &placement);
            assert!(
                probe_str(&wrong)
                    .contains("The extension http://hl7.org/fhir/StructureDefinition/cqf-libraryAlias is not allowed"),
                "{}",
                probe_str(&wrong)
            );
        }
    }
    let root = format!(r#"{{"resourceType":"Patient","extension":[{alias}]}}"#);
    assert!(probe_str(&root).contains("is not allowed"), "{}", probe_str(&root));
}

#[test]
fn f066_fhirpath_context_selects_the_owner_from_the_nearest_resource() {
    let extension = r#"{"url":"http://hl7.org.au/fhir/StructureDefinition/medication-type","valueCoding":{"system":"http://terminology.hl7.org.au/CodeSystem/medication-type","code":"UPD"}}"#;
    let coding = format!(r#"{{"system":"https://example.org/codes","code":"test","extension":[{extension}]}}"#);
    let check = |src: &str, allowed: bool| {
        let report = probe_str(src);
        assert_eq!(
            !report.contains("The extension http://hl7.org.au/fhir/StructureDefinition/medication-type is not allowed"),
            allowed,
            "{report}"
        );
        assert!(!report.contains("Extension context for"), "{report}");
    };
    let right = format!(r#"{{"resourceType":"Medication","code":{{"coding":[{coding}]}}}}"#);
    check(&right, true);
    check(&format!(r#"{{"resourceType":"Medication","extension":[{extension}]}}"#), false);
    // Equal-looking codings in two positions: selecting code.coding must
    // not also permit an extension on Medication.form.coding.
    check(
        &format!(r#"{{"resourceType":"Medication","code":{{"coding":[{coding}]}},"form":{{"coding":[{coding}]}}}}"#),
        false,
    );
    let choice = format!(
        r#"{{"resourceType":"MedicationRequest","status":"active","intent":"order","subject":{{"reference":"Patient/p"}},"medicationCodeableConcept":{{"coding":[{coding}]}}}}"#
    );
    check(&choice, true);
    check(
        &format!(
            r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"fullUrl":"urn:uuid:11111111-1111-1111-1111-111111111111","resource":{right}}}]}}"#
        ),
        true,
    );
}

#[test]
fn f066_r5_context_alternatives_do_not_permit_an_unrelated_r4_owner() {
    let extension = r#"{"url":"http://hl7.org/fhir/StructureDefinition/patient-contactPriority","valuePositiveInt":1}"#;
    let right =
        format!(r#"{{"resourceType":"Patient","contact":[{{"name":{{"text":"test"}},"extension":[{extension}]}}]}}"#);
    assert!(validate(&right).is_valid(), "{}", probe_str(&right));
    let wrong = format!(
        r#"{{"resourceType":"Observation","status":"final","code":{{"text":"test"}},"extension":[{extension}]}}"#
    );
    let report = probe_str(&wrong);
    assert!(report.contains("is not allowed"), "{report}");
    assert!(!report.contains("Extension context for"), "{report}");
}

#[test]
fn f066_fhirpath_context_preserves_the_primitive_carrier_identity() {
    let derived = r#"{"url":"http://hl7.org/fhir/StructureDefinition/structuredefinition-conformance-derivedFrom","valueCanonical":"https://example.org/StructureDefinition/source"}"#;
    for value in ["", r#""valueInteger":1,"#] {
        let right = format!(
            r#"{{"resourceType":"Patient","extension":[{{"url":"http://hl7.org/fhir/StructureDefinition/structuredefinition-fmm",{value}"_valueInteger":{{"extension":[{derived}]}}}}]}}"#
        );
        assert!(validate(&right).is_valid(), "{}", probe_str(&right));
        let wrong = right.replace("structuredefinition-fmm\"", "other-parent\"");
        let report = probe_str(&wrong);
        assert!(report.contains("structuredefinition-conformance-derivedFrom is not allowed"), "{report}");
    }
}

/// 0.14.0: a slice with nothing fixed and a `required` binding where its
/// discriminator points is decided by membership — through the bundled
/// sets, a whole-system set, or the host's provider; a set nobody can
/// decide leaves the instance unassigned, with a warning, never an error.
#[test]
fn a_slice_discriminated_by_a_binding_is_decided_by_membership() {
    use fhir_pods_core::terminology::{Expansions, Membership, TerminologyProvider};
    use fhir_pods_core::{wire, Validator};
    let v = Validator::new();
    // AU Core Medication: `code.coding:pbs` is every code of the PBS item
    // system (a whole-system set — the system decides), `:amt` an AMT set
    // the pod does not ship (the seam)
    let pbs = br#"{"resourceType":"Medication","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-medication"]},
        "code":{"coding":[{"system":"http://pbs.gov.au/code/item","code":"10001J"}],"text":"Amoxicillin"}}"#;
    let r = v.validate(pbs);
    assert!(r.is_valid(), "{}", wire::report(&r));
    assert!(!r.report.issues.iter().any(|i| i.message.contains("could not be assigned")), "{}", wire::report(&r));
    // an AMT coding: the set is not bundled — unassigned, said, not an error
    let amt = br#"{"resourceType":"Medication","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-medication"]},
        "code":{"coding":[{"system":"http://snomed.info/sct","code":"21433011000036107"}],"text":"Amoxicillin"}}"#;
    let r = v.validate(amt);
    assert!(r.is_valid(), "{}", wire::report(&r));
    assert!(
        r.report.issues.iter().any(|i| i.severity == fhir_pods_core::validate::Severity::Warning
            && i.message.contains("could not be assigned to a slice")
            && i.message.contains("australian-medication-1")),
        "{}",
        wire::report(&r)
    );
    // with a table that answers for the AMT set, the coding is the slice's
    let table = Expansions::parse(
        "fhir-pods-terminology 1\n= https://healthterminologies.gov.au/fhir/ValueSet/australian-medication-1\nhttp://snomed.info/sct\t21433011000036107\t\n",
    )
    .unwrap();
    let r = v.validate_with(amt, &table);
    assert!(
        r.is_valid() && !r.report.issues.iter().any(|i| i.message.contains("could not be assigned")),
        "{}",
        wire::report(&r)
    );
    // and a provider that says NO puts it in neither slice: an error only where the slicing is closed
    struct Never;
    impl TerminologyProvider for Never {
        fn validate_code(&self, _: &str, _: &str, _: &str) -> Membership {
            Membership::NotMember
        }
    }
    let r = v.validate_with(amt, &Never);
    assert!(!r.report.issues.iter().any(|i| i.message.contains("could not be assigned")), "{}", wire::report(&r));

    // US Core Condition (problems / health concerns): `category:us-core` is
    // a required slice bound to a bundled two-code set — decided offline
    let problem = br#"{"resourceType":"Condition","meta":{"profile":["http://hl7.org/fhir/us/core/StructureDefinition/us-core-condition-problems-health-concerns"]},
        "clinicalStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-clinical","code":"active"}]},
        "category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-category","code":"problem-list-item"}]}],
        "code":{"coding":[{"system":"http://snomed.info/sct","code":"38341003"}]},"subject":{"reference":"Patient/1"}}"#;
    let r = v.validate(problem);
    assert!(r.is_valid(), "{}", wire::report(&r));
    let other =
        String::from_utf8_lossy(problem).replace(r#""code":"problem-list-item""#, r#""code":"encounter-diagnosis""#);
    let r = v.validate(other.as_bytes());
    assert!(!r.is_valid(), "encounter-diagnosis is not a problem or health concern: {}", wire::report(&r));
    assert!(r.report.issues.iter().any(|i| i.message.contains("category:us-core is required")), "{}", wire::report(&r));
}

/// 0.14.1: three more discriminator shapes compile — an Identifier slice
/// whose datatype profile binds `type` (AU's DVA number: membership), a
/// Reference slice discriminated by what it resolves to when the target
/// is a base vital-sign profile the registry carries, and a Bundle entry
/// whose `resource` is typed as the resource itself.
#[test]
fn identifier_type_bindings_resolved_targets_and_typed_entries_discriminate() {
    use fhir_pods_core::{wire, Validator};
    let v = Validator::new();
    // AU Core Patient: a DVA number is `identifier:dva` by the DVA-entitlement
    // type binding (bundled) — the slice's fixed system is then enforced
    let dva = br#"{"resourceType":"Patient","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"]},
        "identifier":[{"type":{"coding":[{"system":"http://terminology.hl7.org.au/CodeSystem/v2-0203","code":"DVG"}]},"system":"http://ns.electronichealth.net.au/id/dva","value":"NBUR9080"}],
        "name":[{"family":"Citizen"}],"gender":"female","birthDate":"1988-04-12"}"#;
    let r = v.validate(dva);
    assert!(r.is_valid(), "{}", wire::report(&r));
    assert!(
        !r.report.issues.iter().any(|i| i.message.contains("slice 'dva'")),
        "no longer deferred: {}",
        wire::report(&r)
    );
    let wrong_system =
        String::from_utf8_lossy(dva).replace("http://ns.electronichealth.net.au/id/dva", "http://example.org/dva");
    let r = v.validate(wrong_system.as_bytes());
    assert!(!r.is_valid(), "a DVA-typed identifier must carry the DVA system: {}", wire::report(&r));
    assert!(r.report.issues.iter().any(|i| i.path.contains("identifier:dva")), "{}", wire::report(&r));

    // AU Core MedicationRequest: `supportingInformation:bodyWeight` is the
    // reference that resolves to a base body-weight Observation (contained)
    let mr = br##"{"resourceType":"MedicationRequest","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-medicationrequest"]},
        "contained":[{"resourceType":"Observation","id":"wt","status":"final",
            "category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs"}]}],
            "code":{"coding":[{"system":"http://loinc.org","code":"29463-7"}]},"subject":{"reference":"Patient/1"},"effectiveDateTime":"2026-09-20",
            "valueQuantity":{"value":64,"unit":"kg","system":"http://unitsofmeasure.org","code":"kg"}}],
        "status":"active","intent":"order","medicationCodeableConcept":{"coding":[{"system":"http://pbs.gov.au/code/item","code":"10001J"}],"text":"Amoxicillin"},
        "subject":{"reference":"Patient/1"},"authoredOn":"2026-09-20","requester":{"reference":"Practitioner/1"},
        "supportingInformation":[{"reference":"#wt"},{"reference":"#wt"}]}"##;
    let r = v.validate(mr);
    assert!(
        r.report
            .issues
            .iter()
            .any(|i| i.path.contains("supportingInformation:bodyWeight") && i.message.contains("exceeds max 1")),
        "two body weights resolve to the bodyWeight slice (max 1): {}",
        wire::report(&r)
    );

    // AU PS Bundle: an entry whose resource is a Consent is `entry:consent`
    // by its type — the slice's own rules reach it (its fullUrl is required
    // there, and the finding is labelled with the slice)
    let bundle = r#"{"resourceType":"Bundle","meta":{"profile":["http://hl7.org.au/fhir/ps/StructureDefinition/au-ps-bundle"]},"type":"document","timestamp":"2026-09-21T10:00:00+10:00",
        "entry":[{"resource":{"resourceType":"Consent","id":"c1","status":"active","scope":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/consentscope","code":"patient-privacy"}]},"category":[{"coding":[{"system":"http://loinc.org","code":"59284-0"}]}]}}]}"#;
    let r = v.validate(bundle.as_bytes());
    assert!(
        r.report.issues.iter().any(|i| i.path == "Bundle.entry:consent.fullUrl"),
        "the Consent entry is the consent slice's: {}",
        wire::report(&r)
    );
}

/// 0.14.2: slices nest to any depth — AU Core's `medicationClass` extension
/// slice sits inside the AMT / PBS coding slice inside the medication type
/// slice, and its rules reach an instance three levels down.
#[test]
fn a_slice_three_levels_deep_is_enforced() {
    use fhir_pods_core::{wire, Validator};
    let v = Validator::new();
    let mr = |ext: &str| {
        format!(
            r#"{{"resourceType":"MedicationRequest","meta":{{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-medicationrequest"]}},
        "status":"active","intent":"order","medicationCodeableConcept":{{"coding":[{{"system":"http://pbs.gov.au/code/item","code":"10001J",
            "extension":[{ext}]}}],"text":"Amoxicillin"}},
        "subject":{{"reference":"Patient/1"}},"authoredOn":"2026-09-20","requester":{{"reference":"Practitioner/1"}}}}"#
        )
    };
    let one = r#"{"url":"http://hl7.org.au/fhir/StructureDefinition/medication-type","valueCoding":{"system":"http://terminology.hl7.org.au/CodeSystem/medication-type","code":"BPD"}}"#;
    let r = v.validate(mr(one).as_bytes());
    assert!(r.is_valid(), "{}", wire::report(&r));
    assert!(
        !r.report.issues.iter().any(|i| i.message.contains("was not checked")),
        "nothing deferred: {}",
        wire::report(&r)
    );
    // the slice is 0..1: two medication-type extensions on the PBS coding exceed it
    let two = format!("{one},{one}");
    let r = v.validate(mr(&two).as_bytes());
    assert!(
        r.report
            .issues
            .iter()
            .any(|i| i.path.contains("coding:pbs.extension:medicationClass") && i.message.contains("exceeds max 1")),
        "{}",
        wire::report(&r)
    );
}

/// A `CodeableConcept` under a required binding with no usable coding — text
/// only, a data-absent-reason extension, or Coding objects without code — is
/// an error, as validator_cli 6.10.4 reports it, bare and under a profile
/// alike. A standalone required-bound Coding with only system remains valid.
#[test]
fn a_required_bound_concept_with_no_coding_is_an_error_bare_and_profiled() {
    use fhir_pods_core::validate::Severity;
    let errors = |json: &str| -> Vec<String> {
        fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.path, i.message))
            .collect()
    };
    let allergy = |meta: &str, status: &str| {
        format!(
            r#"{{"resourceType":"AllergyIntolerance","id":"a1",{meta}"text":{{"status":"generated","div":"<div xmlns=\"http://www.w3.org/1999/xhtml\">Peanut.</div>"}},{status}"code":{{"coding":[{{"system":"http://snomed.info/sct","code":"91935009"}}]}},"patient":{{"reference":"Patient/p1"}}}}"#
        )
    };
    const AU: &str =
        r#""meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-allergyintolerance"]},"#;
    const DAR: &str = r#""clinicalStatus":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/data-absent-reason","valueCode":"unknown"}]},"#;
    const TEXT: &str = r#""clinicalStatus":{"text":"unknown"},"#;
    const CODELESS: &str = r#""clinicalStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/allergyintolerance-clinical","display":"Active"}]},"#;
    const CODED: &str = r#""clinicalStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/allergyintolerance-clinical","code":"active"}]},"#;
    for meta in ["", AU] {
        for status in [DAR, TEXT] {
            let e = errors(&allergy(meta, status));
            assert!(
                e.iter().any(|x| x.starts_with("AllergyIntolerance.clinicalStatus | No code provided, and a code is required from the value set http://hl7.org/fhir/ValueSet/allergyintolerance-clinical")),
                "meta={meta:?} status={status:?}: {e:?}"
            );
        }
        let e = errors(&allergy(meta, CODELESS));
        assert!(
            e.iter().any(|x| {
                x.starts_with("AllergyIntolerance.clinicalStatus | ")
                    && x.contains("codings")
                    && (x.contains("not found in the value set") || x.contains("not in the bound value set"))
            }),
            "meta={meta:?}: {e:?}"
        );
        assert!(errors(&allergy(meta, CODED)).is_empty(), "meta={meta:?}: {:?}", errors(&allergy(meta, CODED)));
    }
    let coding = r#"{"resourceType":"Encounter","status":"finished","class":{"system":"http://terminology.hl7.org/CodeSystem/v3-ActCode"}}"#;
    assert!(errors(coding).is_empty(), "{:?}", errors(coding));
}

/// A backbone the base defines by `contentReference` — a section's section,
/// an item's item — carries its invariants at every depth, as validator_cli
/// 6.10.4 evaluates them: `cmp-1` fails on an empty inner section, bare and
/// under a declared profile, reported at the depth it fails; a sectioned
/// inner section with text is clean. Found by the reasons lane on a
/// de-identified probe whose inner section lost its narrative.
#[test]
fn a_content_referenced_backbones_invariants_hold_at_every_depth() {
    use fhir_pods_core::validate::Severity;
    let errors = |json: &str| -> Vec<String> {
        fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.path, i.message))
            .collect()
    };
    const AU: &str = r#""meta":{"profile":["http://hl7.org.au/fhir/StructureDefinition/au-composition"]},"#;
    let composition = |meta: &str, inner: &str| {
        format!(
            r#"{{"resourceType":"Composition",{meta}"status":"final","type":{{"text":"Synthetic summary"}},"date":"2026-10-06T00:00:00Z","author":[{{"display":"Synthetic author"}}],"title":"Synthetic composition","section":[{{"title":"Outer","section":[{inner}]}}]}}"#
        )
    };
    const EMPTY: &str = r#"{"title":"Inner","mode":"working"}"#;
    const NESTED_EMPTY: &str = r#"{"title":"Inner","section":[{"title":"Innermost","mode":"working"}]}"#;
    const TEXTED: &str = r#"{"title":"Inner","mode":"working","text":{"status":"generated","div":"<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>Synthetic narrative</p></div>"}}"#;
    for meta in ["", AU] {
        let e = errors(&composition(meta, EMPTY));
        assert!(e.iter().any(|x| x.starts_with("Composition.section.section | cmp-1")), "meta={meta:?}: {e:?}");
        assert!(!e.iter().any(|x| x.starts_with("Composition.section | ")), "meta={meta:?}: {e:?}");
        let e = errors(&composition(meta, NESTED_EMPTY));
        assert!(e.iter().any(|x| x.starts_with("Composition.section.section.section | cmp-1")), "meta={meta:?}: {e:?}");
        assert!(!e.iter().any(|x| x.starts_with("Composition.section.section | ")), "meta={meta:?}: {e:?}");
        let e = errors(&composition(meta, TEXTED));
        assert!(e.is_empty(), "meta={meta:?}: {e:?}");
    }
    // an invariant on a child of the referenced backbone (que-7 on
    // `item.enableWhen`), and one whose copy lies elsewhere in the resource
    // (obs-3 on `component.referenceRange` → `Observation.referenceRange`);
    // each pair probed against validator_cli 6.10.4
    let questionnaire = |answer: &str| {
        format!(
            r#"{{"resourceType":"Questionnaire","status":"active","item":[{{"linkId":"g","type":"group","item":[{{"linkId":"q","type":"boolean","enableWhen":[{{"question":"g","operator":"exists",{answer}}}]}}]}}]}}"#
        )
    };
    let e = errors(&questionnaire(r#""answerString":"x""#));
    assert!(e.iter().any(|x| x.starts_with("Questionnaire.item.item.enableWhen | que-7")), "{e:?}");
    let e = errors(&questionnaire(r#""answerBoolean":true"#));
    assert!(e.is_empty(), "{e:?}");
    let e = errors(
        r#"{"resourceType":"Questionnaire","status":"active","item":[{"linkId":"g","type":"group","item":[{"linkId":"g2","type":"group"}]}]}"#,
    );
    assert!(e.iter().any(|x| x.starts_with("Questionnaire.item.item | que-1")), "{e:?}");
    let observation = |range: &str| {
        format!(
            r#"{{"resourceType":"Observation","status":"final","code":{{"text":"x"}},"component":[{{"code":{{"text":"y"}},"referenceRange":[{range}]}}]}}"#
        )
    };
    let e = errors(&observation(r#"{"type":{"text":"normal"}}"#));
    assert!(e.iter().any(|x| x.starts_with("Observation.component.referenceRange | obs-3")), "{e:?}");
    let e = errors(&observation(r#"{"text":"normal range"}"#));
    assert!(e.is_empty(), "{e:?}");
    // a copy carries the backbone's invariants and its own — a test
    // operation fails tst-7 and tst-8 together, a teardown's tst-7 and
    // tst-9, never a sibling copy's (no tst-9 on a test); probed, 6.10.4
    let testscript = |phase: &str, op: &str| {
        format!(
            r#"{{"resourceType":"TestScript","url":"http://example.org/ts","name":"T","status":"active",{phase}{{"action":[{{"operation":{{"type":{{"system":"http://terminology.hl7.org/CodeSystem/testscript-operation-codes","code":"read"}},"encodeRequestUrl":true{op}}}}}]}}{}}}"#,
            if phase.starts_with(r#""test""#) { "]" } else { "" }
        )
    };
    let keys = |e: &[String], at: &str| -> Vec<String> {
        e.iter()
            .filter_map(|x| x.strip_prefix(&format!("{at} | ")))
            .map(|m| m.split(':').next().unwrap_or("").to_string())
            .collect()
    };
    let e = errors(&testscript(r#""test":["#, ""));
    assert_eq!(keys(&e, "TestScript.test.action.operation"), ["tst-7", "tst-8"], "{e:?}");
    let e = errors(&testscript(r#""teardown":"#, ""));
    assert_eq!(keys(&e, "TestScript.teardown.action.operation"), ["tst-7", "tst-9"], "{e:?}");
    let e = errors(&testscript(r#""teardown":"#, r#","url":"/Patient/1""#));
    assert!(e.is_empty(), "{e:?}");
}

/// A profile's required binding to a set the pod cannot decide offline (an
/// IPS status subset) does not hide the base's: `status` of `mutated` under
/// the IPS laboratory profile is an error, as validator_cli 6.10.4 reports
/// it against the base set; `final` is not. Found by the mutation lane.
#[test]
fn a_profiles_undecidable_set_falls_back_to_the_base_binding() {
    use fhir_pods_core::validate::Severity;
    let at = |json: &str, path: &str| -> Vec<String> {
        fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error && i.path == path)
            .map(|i| i.message.clone())
            .collect()
    };
    let observation = |status: &str| {
        format!(
            r#"{{"resourceType":"Observation","meta":{{"profile":["http://hl7.org/fhir/uv/ips/StructureDefinition/Observation-results-laboratory-pathology-uv-ips"]}},"status":"{status}","category":[{{"coding":[{{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"laboratory"}}]}}],"code":{{"coding":[{{"system":"http://loinc.org","code":"718-7"}}]}},"subject":{{"reference":"Patient/p"}},"effectiveDateTime":"2026-01-01","valueQuantity":{{"value":13,"unit":"g/dL","system":"http://unitsofmeasure.org","code":"g/dL"}}}}"#
        )
    };
    let e = at(&observation("mutated"), "Observation.status");
    assert!(e.iter().any(|m| m.contains("not in the bound value set")), "{e:?}");
    assert!(at(&observation("final"), "Observation.status").is_empty());
}

/// A coding with no code under a required binding the pod cannot decide
/// offline (US Core's LOINC-built document types) is not judged — the
/// reference validator says nothing there — while a concept with no coding
/// at all is still "No code provided". Found by the mutation lane.
#[test]
fn a_code_less_coding_is_judged_only_where_the_set_can_be_decided() {
    use fhir_pods_core::validate::Severity;
    let at = |json: &str, path: &str| -> Vec<String> {
        fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error && i.path == path)
            .map(|i| i.message.clone())
            .collect()
    };
    let doc = |ty: &str| {
        format!(
            r#"{{"resourceType":"DocumentReference","meta":{{"profile":["http://hl7.org/fhir/us/core/StructureDefinition/us-core-documentreference"]}},"status":"current","type":{ty},"category":[{{"coding":[{{"system":"http://hl7.org/fhir/us/core/CodeSystem/us-core-documentreference-category","code":"clinical-note"}}]}}],"subject":{{"reference":"Patient/p"}},"content":[{{"attachment":{{"contentType":"text/plain","data":"eA=="}}}}]}}"#
        )
    };
    assert!(at(
        &doc(r#"{"coding":[{"system":"http://loinc.org","display":"Discharge summary"}]}"#),
        "DocumentReference.type"
    )
    .is_empty());
    let e = at(&doc(r#"{"text":"Discharge summary"}"#), "DocumentReference.type");
    assert!(e.iter().any(|m| m.starts_with("No code provided")), "{e:?}");
}

/// `Reference.type` is held to the element's targets — the base's with no
/// profile, the profile's under one — by bare type name, as validator_cli
/// 6.10.4 holds it: a canonical there is wrong too. Found by the mutation
/// lane.
#[test]
fn a_references_declared_type_is_held_to_the_elements_targets() {
    use fhir_pods_core::validate::Severity;
    let errors = |json: &str| -> Vec<String> {
        fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.path, i.message))
            .collect()
    };
    let immunization = |ty: &str| {
        format!(
            r#"{{"resourceType":"Immunization","status":"completed","vaccineCode":{{"text":"x"}},"patient":{{"reference":"Patient/1","type":"{ty}"}},"occurrenceDateTime":"2026-01-01"}}"#
        )
    };
    for bad in ["Organization", "Mutated", "http://hl7.org/fhir/StructureDefinition/Patient"] {
        let e = errors(&immunization(bad));
        assert!(
            e.iter().any(
                |x| x.starts_with("Immunization.patient | Invalid Resource target type") && x.contains("(Patient)")
            ),
            "{bad}: {e:?}"
        );
    }
    assert!(errors(&immunization("Patient")).is_empty(), "{:?}", errors(&immunization("Patient")));
    let e =
        errors(r#"{"resourceType":"Patient","managingOrganization":{"reference":"Organization/1","type":"Mutated"}}"#);
    assert!(e.iter().any(|x| x.starts_with("Patient.managingOrganization | Invalid Resource target type")), "{e:?}");
}

/// The reference validator implies a vital-sign profile from its own code
/// table, not the profile's magic code alone: `3141-9` is a body weight,
/// held to `bodyweight` (whose magic `29463-7` it then lacks); `8335-2` is
/// not. Each code probed against 6.10.4. Found by the mutation lane.
#[test]
fn a_vital_sign_profile_is_implied_by_the_reference_validators_code_table() {
    use fhir_pods_core::validate::Severity;
    let errors = |json: &str| -> Vec<String> {
        fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.path, i.message))
            .collect()
    };
    let vitals = |code: &str| {
        format!(
            r#"{{"resourceType":"Observation","status":"final","category":[{{"coding":[{{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs"}}]}}],"code":{{"coding":[{{"system":"http://loinc.org","code":"{code}"}}]}},"subject":{{"reference":"Patient/1"}},"effectiveDateTime":"2026-01-01","valueQuantity":{{"value":70,"unit":"kg","system":"http://unitsofmeasure.org","code":"kg"}}}}"#
        )
    };
    let e = errors(&vitals("3141-9"));
    assert!(e.iter().any(|x| x.starts_with("Observation.code")), "{e:?}");
    assert!(errors(&vitals("8335-2")).is_empty(), "{:?}", errors(&vitals("8335-2")));
    assert!(errors(&vitals("29463-7")).is_empty(), "{:?}", errors(&vitals("29463-7")));
}

/// A required binding beneath a choice (`Extension.value[x].type`, IHE
/// BALP's other-id) is judged by the instance's shape where the table cannot
/// type the path: a type with codings but no code, or text alone, is the
/// error the reference validator reports. Found by the mutation lane.
#[test]
fn a_binding_beneath_a_choice_is_judged_by_the_instances_shape() {
    use fhir_pods_core::validate::Severity;
    let errors = |json: &str| -> Vec<String> {
        fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.path, i.message))
            .collect()
    };
    let audit = |ty: &str| {
        format!(
            r#"{{"resourceType":"AuditEvent","type":{{"system":"http://terminology.hl7.org/CodeSystem/audit-event-type","code":"rest"}},"recorded":"2026-01-01T00:00:00Z","agent":[{{"extension":[{{"url":"https://profiles.ihe.net/ITI/BALP/StructureDefinition/ihe-otherId","valueIdentifier":{{"type":{ty},"value":"JohnDoe"}}}}],"requestor":true}}],"source":{{"observer":{{"display":"x"}}}}}}"#
        )
    };
    let coded = r#"{"coding":[{"system":"https://profiles.ihe.net/ITI/BALP/CodeSystem/OtherIdentifierTypes","code":"SAML-subject-id"}]}"#;
    assert!(errors(&audit(coded)).is_empty(), "{:?}", errors(&audit(coded)));
    let e = errors(&audit(
        r#"{"coding":[{"system":"https://profiles.ihe.net/ITI/BALP/CodeSystem/OtherIdentifierTypes","display":"x"}]}"#,
    ));
    assert!(e.iter().any(|x| x.contains("agent") && x.contains("not in the bound value set")), "{e:?}");
    let e = errors(&audit(r#"{"text":"x"}"#));
    assert!(e.iter().any(|x| x.contains("agent") && x.contains("No code provided")), "{e:?}");
}

/// A relative reference is `Type/id`: `1` alone is the reference validator's
/// "Relative URLs must be of the format [ResourceName]/[id]"; a type it does
/// not define (`NotAType/1`) passes the format and resolves nowhere — in a
/// document it is not held to the bundle either. Found by the mutation lane.
#[test]
fn a_relative_reference_has_the_type_slash_id_shape() {
    use fhir_pods_core::validate::Severity;
    let errors = |json: &str| -> Vec<String> {
        fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.path, i.message))
            .collect()
    };
    let obs = |r: &str| {
        format!(
            r#"{{"resourceType":"Observation","status":"final","code":{{"text":"x"}},"subject":{{"reference":"{r}"}}}}"#
        )
    };
    // the format rule applies where a reference resolves — inside a Bundle;
    // a standalone resource's bare id passes (probed, 6.10.4)
    let bundled = |r: &str| {
        format!(
            r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"fullUrl":"urn:uuid:7c0f3f2e-5b7d-4e0a-9c1f-1a2b3c4d5e6f","resource":{}}}]}}"#,
            obs(r)
        )
    };
    let e = errors(&bundled("1"));
    assert!(e.iter().any(|x| x.contains("Relative URLs must be of the format")), "{e:?}");
    assert!(errors(&obs("1")).is_empty(), "{:?}", errors(&obs("1")));
    assert!(errors(&bundled("NotAType/1")).is_empty(), "{:?}", errors(&bundled("NotAType/1")));
    assert!(errors(&bundled("Patient/1/_history/2")).is_empty());
    assert!(errors(&bundled("urn:uuid:9f1a7a0e-3c1e-4b3a-9f0b-2b0e1d6c5a11")).is_empty());
}

/// An empty string beside a carrier with content is a value absent, not a
/// value wrong — validator_cli 6.10.4 passes `"family": ""` with a
/// `_family` translation extension (and an empty `questionnaire` with its
/// carrier), while an empty string with no carrier is still not a value.
/// Found by the mutation lane.
#[test]
fn an_empty_string_beside_a_carrier_with_content_is_an_absent_value() {
    use fhir_pods_core::validate::Severity;
    let errors = |json: &str| -> Vec<String> {
        fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.path, i.message))
            .collect()
    };
    let carried = r#"{"resourceType":"Patient","name":[{"family":"","_family":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/translation","extension":[{"url":"lang","valueCode":"fr"},{"url":"content","valueString":"Forgeron"}]}]}}]}"#;
    assert!(errors(carried).is_empty(), "{:?}", errors(carried));
    let bare = r#"{"resourceType":"Patient","name":[{"family":""}]}"#;
    let e = errors(bare);
    assert!(e.iter().any(|x| x.starts_with("Patient.name.family | ")), "{e:?}");
}

/// Inside a Bundle, a reference that resolves to an entry is held to the
/// element's base targets by the target's own type — validator_cli 6.10.4
/// resolves by `fullUrl` (a `urn:uuid` entry by its uuid; a relative
/// `Type/id` only against a RESTful fullUrl), so a relative reference among
/// `urn:uuid` entries resolves nowhere and is not judged. Found by the
/// instrument-path lane: a referral whose reason was a Flag.
#[test]
fn a_resolved_reference_in_a_bundle_is_held_to_the_elements_base_targets() {
    use fhir_pods_core::validate::Severity;
    let errors = |json: &str| -> Vec<String> {
        fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.path, i.message))
            .collect()
    };
    let bundle = |reason: &str, target: &str| {
        format!(
            r#"{{"resourceType":"Bundle","type":"collection","entry":[
  {{"fullUrl":"urn:uuid:00000000-0000-4000-8000-000000000000","resource":{{"resourceType":"ServiceRequest","id":"s1","status":"active","intent":"proposal","code":{{"text":"x"}},"subject":{{"reference":"Patient/p1"}},"reasonReference":[{{"reference":"{reason}"}}]}}}},
  {{"fullUrl":"urn:uuid:00000001-0000-4000-8000-000000000001","resource":{target}}}]}}"#
        )
    };
    const FLAG: &str = r#"{"resourceType":"Flag","id":"f1","status":"active","code":{"text":"x"},"subject":{"reference":"Patient/p1"}}"#;
    const OBS: &str = r#"{"resourceType":"Observation","id":"o1","status":"final","code":{"text":"x"},"subject":{"reference":"Patient/p1"}}"#;
    let e = errors(&bundle("urn:uuid:00000001-0000-4000-8000-000000000001", FLAG));
    assert!(e.iter().any(|x| x.contains("Invalid Resource target type. Found Flag")), "{e:?}");
    assert!(errors(&bundle("urn:uuid:00000001-0000-4000-8000-000000000001", OBS)).is_empty());
    assert!(errors(&bundle("Flag/f1", FLAG)).is_empty(), "{:?}", errors(&bundle("Flag/f1", FLAG)));
}

/// The narrative's XHTML, held as validator_cli 6.10.4 holds it — every
/// case here probed, and the findings pinned by id and by message (the
/// reference validator's wording): txt-1 with the parser's own finding
/// beside it, txt-2 as it counts content, the content model, the link
/// rules, the entity rules, its own nesting limit. Found wanting by the
/// instrument-path review: a clinician summary's narrative was never checked.
#[test]
fn a_narrative_is_held_to_what_the_reference_validator_allows() {
    use fhir_pods_core::validate::Severity;
    const X: &str = r#"xmlns="http://www.w3.org/1999/xhtml""#;
    const NS: &str = "http://www.w3.org/1999/xhtml";
    let errors = |div: &str| -> Vec<String> {
        let json = format!(
            r#"{{"resourceType":"Patient","id":"p","text":{{"status":"generated","div":"{}"}},"gender":"female"}}"#,
            div.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let mut v: Vec<String> = fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.id, i.message))
            .collect();
        v.sort();
        v
    };
    let txt1 = "inv.txt-1 | txt-1: The narrative SHALL contain only the basic html formatting elements and attributes described in chapters 7-11 (except section 4 of chapter 9) and 15 of the HTML 4.0 standard, <a> elements (either name or href), images and internally contained style attributes".to_string();
    let txt2 = "inv.txt-2 | txt-2: The narrative SHALL have some non-whitespace content".to_string();
    // valid, as probed — including what the reference parser tolerates
    for ok in [
        format!(
            r#"<div {X}><p>Summary <b>bold</b> <a href="https://example.org/x">link</a></p><table><tr><td>a</td></tr></table></div>"#
        ),
        format!(r#"<div {X}><div><p>x</p></div></div>"#),
        format!(r#"<div {X} class="c" id="n1" lang="en"><p style="color:red">x</p></div>"#),
        format!(r#"<div {X}><img src="http://e.org/x.png" alt="t"/></div>"#),
        format!(r#"<div {X}><p>a&#160;b &amp; c &#xZZ; &#99999999999;</p></div>"#),
        format!(
            r##"<div {X}><a href="data:text/html,a b">t</a><a href="JavaScript:alert(1)">u</a><a href="#">v</a></div>"##
        ),
        format!(
            r##"{}<?xml version="1.0"?><!DOCTYPE html><!--->   <div {X}><p>t</p></div>text<p>after the root</p>"##,
            "\u{feff}"
        ),
        format!(r#"<h:div xmlns:h="{NS}"><h:p>t</h:p></h:div>"#),
        format!(r##"<div {X}><p id="s1">t</p><a href="#s1">u</a><a name="s2">v</a><a href="#s2">w</a></div>"##),
        format!(r#"<div {X}><table><thead><tr><th>t</th></tr></thead></table></div>"#),
        // (the root left open on purpose: an unclosed element at the end passes)
        format!(
            r#"<div {X}><p class=c title="a" title="b">t</p><map name="m"><area href="http://e.org" alt="a" shape="rect" coords="0,0,1,1" nohref="nohref"/></map>"#
        ),
        format!(r##"<div {X}><a href="#s9">forward</a><p id="s9">t</p></div>"##),
        format!(r#"<div {X}><p><span><b>x</b></span></p><![CDATA[<script>]]></div>"#),
    ] {
        assert!(errors(&ok).is_empty(), "{ok}: {:?}", errors(&ok));
    }
    // invalid, as probed: exactly these findings
    let cases: Vec<(String, Vec<String>)> = vec![
        (format!(r#"<div {X}><p>x</p><script>alert(1)</script><script>b</script></div>"#),
         vec![txt1.clone(), "narrative.element | Invalid element name in the XHTML ('script')".into()]),
        (format!(r#"<div {X}><p onclick="x()">x</p><p ärger="y">t</p></div>"#),
         vec![txt1.clone(), "narrative.attribute | Invalid attribute name in the XHTML ('onclick' on 'p')".into(), "narrative.attribute | Invalid attribute name in the XHTML ('ärger' on 'p')".into()]),
        (format!(r#"<div {X}><p>x</div>"#),
         vec![txt1.clone(), txt2.clone(), r#"narrative.parse | Error parsing XHTML: Malformed XHTML: Found "</div>" expecting "</p>""#.into()]),
        (format!(r#"<div {X}><p>a & b</p><script>x</script></div>"#),
         vec![txt1.clone(), txt2.clone(), "narrative.parse | Error parsing XHTML: unable to parse character reference ' b''".into()]),
        (format!(r#"<div {X}><p>a &#x110000; b</p></div>"#),
         vec![txt1.clone(), txt2.clone(), "narrative.parse | Error parsing XHTML: Not a valid Unicode code point: 0x110000".into()]),
        (format!("<div {X}>{}t{}</div>", "<span>".repeat(502), "</span>".repeat(502)),
         vec![txt1.clone(), txt2.clone(), "narrative.parse | Error parsing XHTML: XHTML nesting depth exceeds maximum of 500".into()]),
        (r#"<div><p>Summary</p></div>"#.to_string(),
         vec![format!("narrative.root | Wrong namespace on the XHTML ('null', should be '{NS}')")]),
        (format!(r#"<p {X}>x</p>"#),
         vec!["narrative.root | Wrong name on the XHTML ('p') - must start with div".into()]),
        (format!(r#"<div {X}><x:p>t</x:p></div>"#),
         vec![format!("narrative.root | Wrong namespace on the XHTML ('http://error/undefined-namespace', should be '{NS}')")]),
        (format!(r#"<div {X}><a href="java&#115;cript:alert(1)">x</a></div>"#),
         vec!["narrative.link | Hyperlink scheme 'javascript' in 'javascript:alert(1)' at 'div/a' for 'x' has active content, which is a security risk and not allowed".into()]),
        (format!(r#"<div {X}><a href="urn:x">t</a></div>"#),
         vec!["narrative.link | Hyperlink scheme 'urn' in 'urn:x' at 'div/a' for 't' is not a valid hyperlinkable scheme".into()]),
        (format!(r#"<div {X}><p>a &nbsp; d &copy;</p></div>"#),
         vec!["narrative.entity | Invalid entity in the XHTML ('&copy;')".into(), "narrative.entity | Invalid entity in the XHTML ('&nbsp;')".into()]),
        (format!(r#"<div {X}><li>t</li><li>u</li></div>"#),
         vec!["narrative.element | Elements of type li must be in a ul|ol, not a div at div/li".into()]),
        (format!(r#"<div {X}><table>text<tr><td>t</td></tr></table></div>"#),
         vec!["narrative.element | Elements of type table at div/table cannot have the following children: @text".into()]),
        (format!(r#"<div {X}><p><p>t</p></p></div>"#),
         vec!["narrative.element | Invalid element name inside a paragraph in the XHTML ('p')".into()]),
        (format!(r#"<div {X}><p><b><div>x</div></b></p></div>"#),
         vec!["narrative.element | Found block structures inside a paragraph/text element b at div/p/b: b/div".into(), "narrative.element | Found block structures inside a paragraph/text element p at div/p: p/b/div".into(), "narrative.element | Invalid element name inside a paragraph in the XHTML ('div')".into()]),
        (format!(r#"<div {X}><a href="http://e.org"><b><a href="http://e.org">t</a></b></a></div>"#),
         vec!["narrative.element | Elements of type a at div/a cannot contain nested a at a/b/a".into()]),
        (format!(r#"<div {X}><a href="">t</a></div>"#),
         vec!["narrative.link | Hyperlink at 'div/a' for 't' is empty".into(), "narrative.link | The URL is not valid because '(URL is empty)': ".into()]),
        (format!(r#"<div {X}><a href="http://e.org/a b&lt;c">t<b>u</b></a></div>"#),
         vec!["narrative.link | The URL is not valid because '(URL contains 2 Invalid Characters: ' ','<')': http://e.org/a b<c".into()]),
        // each occurrence counted, each character named once, in character order
        (format!(r#"<div {X}><a href="http://e.org/&lt;a b c">t</a><a href="http://e.org/x y z">u</a></div>"#),
         vec!["narrative.link | The URL is not valid because '(URL contains 3 Invalid Characters: ' ','<')': http://e.org/<a b c".into(), "narrative.link | The URL is not valid because '(URL contains 2 Invalid Characters: ' ')': http://e.org/x y z".into()]),
        (format!(r##"<div {X}><a href="#nowhere">t</a><a name="x">u</a><img src="#x" alt="y"/></div>"##),
         vec!["narrative.link | Hyperlink '#nowhere' at 'div/a' for 't' does not resolve".into(), "narrative.link | Image source '#x' at 'Patient.text.div/img' does not resolve".into()]),
        (format!(r#"<div {X}><P>t</P></div>"#),
         vec![txt1.clone(), "narrative.element | Invalid element name in the XHTML ('P')".into()]),
        (format!(r#"<div {X}><blink>words</blink></div>"#),
         vec![txt1.clone(), "narrative.element | Invalid element name in the XHTML ('blink')".into()]),
        (format!(r#"<div {X}><br/><!-- c --><p>&#32;</p></div>"#), vec![txt2.clone()]),
        (format!(r#"<div {X}><![CDATA[text]]></div>"#), vec![txt2.clone()]),
    ];
    // a fragment resolves to the resource's own id and to a contained resource's id,
    // and a narrative link to a contained resource is a reference to it (dom-3)
    let contained = |div: &str| -> Vec<String> {
        let json = format!(
            r#"{{"resourceType":"Patient","id":"p","contained":[{{"resourceType":"Binary","id":"c1","contentType":"image/png","data":"iVBORw0KGgo="}}],"text":{{"status":"generated","div":"{}"}},"gender":"female"}}"#,
            div.replace('"', "\\\"")
        );
        let mut v: Vec<String> = fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.id, i.message))
            .collect();
        v.sort();
        v
    };
    assert_eq!(contained(&format!(r##"<div {X}><img src="#c1" alt="y"/></div>"##)), Vec::<String>::new());
    assert_eq!(contained(&format!(r##"<div {X}><a href="#c1">t</a><a href="#p">u</a></div>"##)), Vec::<String>::new());
    assert_eq!(
        contained(&format!(r##"<div {X}><a href="#c9">t</a></div>"##)),
        vec![
            "inv.dom-3 | dom-3: The contained resource 'c1' is not referenced to from elsewhere in the containing resource nor does it refer to the containing resource".to_string(),
            "narrative.link | Hyperlink '#c9' at 'div/a' for 't' does not resolve".to_string()
        ]
    );
    // a narrative inside a contained resource is checked where it is
    let nested = format!(
        r##"{{"resourceType":"Patient","id":"p","contained":[{{"resourceType":"Organization","id":"o","name":"o","text":{{"status":"generated","div":"<div {}><script>x</script></div>"}}}}],"managingOrganization":{{"reference":"#o"}},"gender":"female"}}"##,
        X.replace('"', "\\\"")
    );
    let ids: Vec<String> = fhir_pods_core::validate_bytes(nested.as_bytes())
        .report
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| format!("{} @ {}", i.id, i.path))
        .collect();
    assert_eq!(
        ids,
        vec![
            "inv.txt-1 @ Patient.contained.text.div".to_string(),
            "narrative.element @ Patient.contained.text.div".to_string()
        ]
    );
    // the classifier never mistakes quoted narrative text for another family
    assert_eq!(
        errors(&format!(r#"<div {X}><a href="javascript:x">not evaluated: outside the supported FHIRPath core</a></div>"#)),
        vec!["narrative.link | Hyperlink scheme 'javascript' in 'javascript:x' at 'div/a' for 'not evaluated: outside the supported FHIRPath core' has active content, which is a security risk and not allowed".to_string()]
    );
    for (bad, want) in cases {
        let mut want = want;
        want.sort();
        assert_eq!(errors(&bad), want, "{bad}");
    }
}

/// Where the narrative's text simply stops, and what an image's source is
/// held to — the two things a host's report (2026-10-14) read the other way
/// round. Every case probed against validator_cli 6.10.4 and recorded in
/// `conformance/narrative/`: a closing tag that does not match is refused, a
/// start tag the text ends inside is not, and the hyperlink scheme rules are
/// not applied to an image at all.
#[test]
fn a_truncated_narrative_and_an_image_source_are_judged_as_the_oracle_judges_them() {
    use fhir_pods_core::validate::Severity;
    const X: &str = r#"xmlns="http://www.w3.org/1999/xhtml""#;
    const NS: &str = "http://www.w3.org/1999/xhtml";
    let errors = |div: &str| -> Vec<String> {
        let json = format!(
            r#"{{"resourceType":"Patient","id":"p","text":{{"status":"generated","div":"{}"}},"gender":"female"}}"#,
            div.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let mut v: Vec<String> = fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.id, i.message))
            .collect();
        v.sort();
        v
    };
    let txt1 = "inv.txt-1 | txt-1: The narrative SHALL contain only the basic html formatting elements and attributes described in chapters 7-11 (except section 4 of chapter 9) and 15 of the HTML 4.0 standard, <a> elements (either name or href), images and internally contained style attributes".to_string();
    let txt2 = "inv.txt-2 | txt-2: The narrative SHALL have some non-whitespace content".to_string();
    // tolerated: the text ends inside a start tag, which keeps the element and
    // the attributes read so far — and every image source but the four checked
    for ok in [
        format!(r#"<div {X}><p>Real content.</p><p"#),
        format!(r#"<div {X}><p>Real content.</p><p class="x"#),
        // an unterminated quoted value recovers at the tag's own `>`, so what
        // follows is text and nothing is reported (four forms, each probed)
        format!(r#"<div {X}><p class="x>Real content.</p></div>"#),
        format!(r#"<div {X}><p class='x>Real content.</p></div>"#),
        format!(r#"<div {X}><p class="x>Real content.</p>"#),
        format!(r#"<div {X}><p id="a" class="x>Real content.</p></div>"#),
        // a bare `#` is the document itself: a hyperlink may name it
        format!(r##"<div {X}><p><a href="#">t</a></p></div>"##),
        // a start tag ends at the first `>`, quoted or not: the title is cut
        // there, the rest is text, and the paragraph still closes (probed)
        format!(r#"<div {X}><p title="a>b">text</p></div>"#),
        format!(r#"<div {X}><p>Real content.</p><p class="#),
        format!(r#"<div {X}><p>t</p><img src="javascript:alert(1)" alt="a"/></div>"#),
        format!(r#"<div {X}><p>t</p><img src="vbscript:msgbox(1)" alt="a"/></div>"#),
        format!(r#"<div {X}><p>t</p><img src="file:///etc/passwd" alt="a"/></div>"#),
        format!(r#"<div {X}><p>t</p><img src="urn:uuid:9d4a" alt="a"/></div>"#),
        format!(r#"<div {X}><p>t</p><img src="cid:part1@e.org" alt="a"/></div>"#),
        format!(r#"<div {X}><p>t</p><img src="/assets/x.png" alt="a"/></div>"#),
        format!(r##"<div {X}><p id="s">t</p><img src="#s" alt="a"/></div>"##),
        format!(r##"<div {X}><p>t</p><img src="#p" alt="a"/></div>"##),
    ] {
        assert!(errors(&ok).is_empty(), "{ok}: {:?}", errors(&ok));
    }
    let cases: Vec<(String, Vec<String>)> = vec![
        // a closing tag the text ends inside: the one truncation refused —
        // and the failure leaves no document, so txt-2 goes with it
        (format!(r#"<div {X}><p>Real content.</p></div"#),
         vec![txt1.clone(), txt2.clone(), "narrative.parse | Error parsing XHTML: Unexpected termination of html source".into()]),
        (format!(r#"<div {X}><p>Real content.</p"#),
         vec![txt1.clone(), txt2.clone(), "narrative.parse | Error parsing XHTML: Unexpected termination of html source".into()]),
        // a `/` that no `>` follows
        (format!(r#"<div {X}><p>Real content.</p><p/"#),
         vec![txt1.clone(), txt2.clone(), "narrative.parse | Error parsing XHTML: unexpected non-end of element null::p".into()]),
        // a character that can begin no attribute name, truncation or not
        (format!(r#"<div {X}><p ="x">t</p></div>"#),
         vec![txt1.clone(), txt2.clone(), "narrative.parse | Error parsing XHTML: Unable to read attribute on <p>".into()]),
        (format!(r#"<div {X}><p !>t</p></div>"#),
         vec![txt1.clone(), txt2.clone(), "narrative.parse | Error parsing XHTML: Unable to read attribute on <p>".into()]),
        // a start tag the text ends inside leaves an empty narrative: txt-2
        // alone, no parse failure — and a truncated root keeps its xmlns,
        // where a bare one has no namespace at all
        (format!(r#"<div {X}><p"#), vec![txt2.clone()]),
        (format!(r#"<div {X}"#), vec![txt2.clone()]),
        ("<div".to_string(),
         vec![txt2.clone(), format!("narrative.root | Wrong namespace on the XHTML ('null', should be '{NS}')")]),
        // the four an image's source is held to: empty, invalid characters,
        // a data: URL's comma, a fragment that resolves
        (format!(r#"<div {X}><p>t</p><img src="" alt="a"/></div>"#),
         vec!["narrative.link | The URL is not valid because '(URL is empty)': ".into()]),
        (format!(r#"<div {X}><p>t</p><img src="data:image/png;base64" alt="a"/></div>"#),
         vec!["narrative.link | The URL is not valid because '(No data found in data: URL)': data:image/png;base64".into()]),
        (format!(r#"<div {X}><p>t</p><img src="http://e.org/a b.png" alt="a"/></div>"#),
         vec!["narrative.link | The URL is not valid because '(URL contains Invalid Character: ' ')': http://e.org/a b.png".into()]),
        (format!(r##"<div {X}><p>t</p><img src="#nowhere" alt="a"/></div>"##),
         vec!["narrative.link | Image source '#nowhere' at 'Patient.text.div/img' does not resolve".into()]),
        // an image may not: a bare `#` resolves to nothing (probed — the
        // hyperlink above passes, this does not)
        (format!(r##"<div {X}><p>t</p><img src="#" alt="a"/></div>"##),
         vec!["narrative.link | Image source '#' at 'Patient.text.div/img' does not resolve".into()]),
        // the same first-`>` rule where it costs: the img keeps `http://e.org/a`,
        // its `/>` is text, so it never closes and `</div>` meets `</img>`
        (format!(r#"<div {X}><p>t</p><img src="http://e.org/a>b.png" alt="a"/></div>"#),
         vec![txt1.clone(), txt2.clone(), r#"narrative.parse | Error parsing XHTML: Malformed XHTML: Found "</div>" expecting "</img>""#.into()]),
    ];
    for (bad, want) in cases {
        let mut want = want;
        want.sort();
        assert_eq!(errors(&bad), want, "{bad}");
    }
}

#[test]
fn an_element_the_base_types_simple_quantity_refuses_a_comparator() {
    use fhir_pods_core::validate::Severity;
    let errors = |json: &str| -> Vec<String> {
        let mut v: Vec<String> = fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} | {}", i.id, i.path))
            .collect();
        v.sort();
        v
    };
    let q =
        |c: &str| format!(r#"{{"value":3,{c}"unit":"mmol/L","system":"http://unitsofmeasure.org","code":"mmol/L"}}"#);
    let obs = |extra: &str| {
        format!(
            r#"{{"resourceType":"Observation","id":"o","status":"final","code":{{"coding":[{{"system":"http://example.org/o","code":"x"}}]}},{extra}}}"#
        )
    };
    // a plain Quantity may carry one; the SimpleQuantity elements beside it may not
    assert!(errors(&obs(&format!(r#""valueQuantity":{}"#, q(r#""comparator":"<","#)))).is_empty());
    assert!(errors(&obs(&format!(r#""referenceRange":[{{"low":{},"high":{}}}]"#, q(""), q("")))).is_empty());
    let both = |at: &str| vec![format!("inv.sqty-1 | {at}"), format!("structure.max | {at}")];
    // probed, validator_cli 6.10.4: `Quantity.comparator: max allowed = 0, but
    // found 1 (from …/SimpleQuantity|4.0.1)` and `sqty-1`, both errors, both at
    // the element — a resource's own, a datatype's (Range, Dosage), in a
    // repeating element, a contained resource, a Bundle entry
    assert_eq!(
        errors(&obs(&format!(r#""referenceRange":[{{"low":{}}}]"#, q(r#""comparator":">=","#)))),
        both("Observation.referenceRange[0].low")
    );
    assert_eq!(
        errors(&obs(&format!(r#""valueRange":{{"low":{}}}"#, q(r#""comparator":">","#)))),
        both("Observation.valueRange.low")
    );
    let mr = format!(
        r#"{{"resourceType":"MedicationRequest","status":"active","intent":"order","medicationCodeableConcept":{{"text":"m"}},"subject":{{"reference":"Patient/p"}},"dosageInstruction":[{{"doseAndRate":[{{"doseQuantity":{}}}]}}]}}"#,
        q(r#""comparator":">","#)
    );
    assert_eq!(errors(&mr), both("MedicationRequest.dosageInstruction[0].doseAndRate[0].doseQuantity"));
    let contained = format!(
        r##"{{"resourceType":"DiagnosticReport","status":"final","code":{{"text":"r"}},"result":[{{"reference":"#i"}}],"contained":[{}]}}"##,
        obs(&format!(r#""referenceRange":[{{"high":{}}}]"#, q(r#""comparator":"<=","#))).replacen(
            r#""id":"o""#,
            r#""id":"i""#,
            1
        )
    );
    assert_eq!(errors(&contained), both("DiagnosticReport.contained[0].Observation.referenceRange[0].high"));
    // present through its extension carrier alone is still present (probed)
    let carrier = obs(
        r#""referenceRange":[{"low":{"value":3,"_comparator":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/data-absent-reason","valueCode":"unknown"}]}}}]"#,
    );
    let got = errors(&carrier);
    assert!(got.contains(&"structure.max | Observation.referenceRange[0].low".to_string()), "{got:?}");
    assert!(got.contains(&"inv.sqty-1 | Observation.referenceRange[0].low".to_string()), "{got:?}");
}

#[test]
fn a_compiled_profile_carries_a_base_datatype_profile_the_base_element_does_not_name() {
    use fhir_pods_core::validate::Severity;
    // the one shipped profile that puts SimpleQuantity where the base does
    // not: an extension's value[x] (hl7.fhir.uv.extensions.r4)
    let carrying: Vec<(&str, &str)> = fhir_pods_core::REGISTRY
        .iter()
        .flat_map(|p| p.elements.iter().filter(|e| !e.datatype_profiles.is_empty()).map(move |e| (p.url, e.path)))
        .collect();
    assert_eq!(
        carrying,
        [("http://hl7.org/fhir/StructureDefinition/medicationdispense-quantityRemaining", "Extension.value[x]")]
    );
    let dispense = |c: &str| {
        format!(
            r#"{{"resourceType":"MedicationDispense","status":"completed","medicationCodeableConcept":{{"text":"m"}},"extension":[{{"url":"http://hl7.org/fhir/StructureDefinition/medicationdispense-quantityRemaining","valueQuantity":{{"value":20,{c}"unit":"tablet","system":"http://unitsofmeasure.org","code":"{{tbl}}"}}}}]}}"#
        )
    };
    let errors = |json: &str| -> Vec<String> {
        let mut v: Vec<String> = fhir_pods_core::validate_bytes(json.as_bytes())
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.id.to_string())
            .collect();
        v.sort();
        v
    };
    assert!(errors(&dispense("")).is_empty(), "{:?}", errors(&dispense("")));
    // probed, validator_cli 6.10.4: max 0 + sqty-1 at the extension's value
    assert_eq!(errors(&dispense(r#""comparator":"<","#)), ["inv.sqty-1", "structure.max"]);
}
