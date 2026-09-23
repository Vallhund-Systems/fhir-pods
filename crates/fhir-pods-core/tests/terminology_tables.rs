mod common;
use common::*;

use fhir_pods_core::terminology::{Bundled, Chain, Expansions, Membership, Need, Recorder, TerminologyProvider};
use fhir_pods_core::{terminology_needs, validate_bytes, validate_bytes_with, wire, Validator};

// --- the generated bundled sets ---

#[test]
fn the_registry_s_base_sets_are_bundled_from_the_packages() {
    use fhir_pods_core::value_sets::{display, membership};
    // a FHIR code system (bare code)
    assert_eq!(membership("http://hl7.org/fhir/ValueSet/contact-point-system|4.0.1", "", "phone"), Some(true));
    assert_eq!(membership("http://hl7.org/fhir/ValueSet/contact-point-system", "", "telepathy"), Some(false));
    // an HL7 terminology code system, with and without the system
    assert_eq!(
        membership(
            "http://hl7.org/fhir/ValueSet/condition-clinical",
            "http://terminology.hl7.org/CodeSystem/condition-clinical",
            "active"
        ),
        Some(true)
    );
    assert_eq!(
        membership("http://hl7.org/fhir/ValueSet/condition-clinical", "http://example.org/other", "active"),
        Some(false)
    );
    // a set composed of enumerated concepts (immunization-status ⊂ event-status)
    assert_eq!(membership("http://hl7.org/fhir/ValueSet/immunization-status", "", "completed"), Some(true));
    assert_eq!(membership("http://hl7.org/fhir/ValueSet/immunization-status", "", "preparation"), Some(false));
    // licensed / hosted-elsewhere sets are left to the seam
    assert_eq!(membership("http://hl7.org/fhir/us/core/ValueSet/omb-race-category", "", "2106-3"), None);
    assert_eq!(
        membership("https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1", "", "AAA"),
        None
    );
    // displays come from the code systems
    assert_eq!(
        display("http://terminology.hl7.org/CodeSystem/observation-category", "vital-signs"),
        Some("Vital Signs")
    );
    assert_eq!(display("http://loinc.org", "8867-4"), None);
    // never a SNOMED or LOINC code
    for (_, codes) in fhir_pods_core::value_sets::sets() {
        assert!(codes.iter().all(|(s, _, _)| *s != "http://snomed.info/sct" && *s != "http://loinc.org"));
    }
}

#[test]
fn a_bundled_binding_is_an_error_when_the_code_is_outside_the_set() {
    let patient =
        PATIENT_OK.replace("\"gender\"", "\"telecom\":[{\"system\":\"telepathy\",\"value\":\"1\"}],\"gender\"");
    let r = wire::report(&validate_bytes(patient.as_bytes()));
    // a datatype's own required binding (ContactPoint.system), checked by the
    // shape walk on every ContactPoint wherever it sits, profile or none —
    // worded and located as validator_cli words it
    assert!(
        r.contains("\"path\":\"Patient.telecom[0].system\",\"message\":\"The value provided ('telepathy') was not found in the value set 'http://hl7.org/fhir/ValueSet/contact-point-system', and a code is required from this value set\""),
        "{r}"
    );
    let ok = PATIENT_OK.replace("\"gender\"", "\"telecom\":[{\"system\":\"phone\",\"value\":\"1\"}],\"gender\"");
    let r = wire::report(&validate_bytes(ok.as_bytes()));
    assert!(!r.contains("telecom"), "{r}");
}

// --- the recorder: what a host must resolve ---

/// An AU Core patient whose birthDate carries the date-accuracy extension —
/// bound to an NCTS-hosted set the core does not bundle.
fn patient_with_date_accuracy(code: &str) -> String {
    PATIENT_OK.replace(
        "\"gender\"",
        &format!(
            "\"_birthDate\":{{\"extension\":[{{\"url\":\"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator\",\"valueCoding\":{{\"system\":\"https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\",\"code\":\"{code}\"}}}}]}},\"gender\""
        ),
    )
}

#[test]
fn needs_lists_exactly_what_the_bundled_sets_could_not_decide() {
    let p = patient_with_date_accuracy("AAA");
    let needs = terminology_needs(p.as_bytes());
    assert_eq!(
        needs,
        vec![Need {
            value_set: "https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1".into(),
            system: "https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1".into(),
            code: "AAA".into(),
        }]
    );
    // the same through a Validator, and nothing for a resource the bundle covers
    assert_eq!(Validator::new().terminology_needs(p.as_bytes()), needs);
    assert!(Validator::new().terminology_needs(PATIENT_OK.as_bytes()).is_empty());
    // the recorder itself never decides
    let rec = Recorder::new();
    assert_eq!(rec.validate_code("x", "y", "z"), Membership::Unknown);
    assert_eq!(rec.validate_code("x", "y", "z"), Membership::Unknown);
    assert_eq!(rec.needs().len(), 1, "deduplicated");
}

// --- compiled expansions ---

const TABLE: &str = "\
# a table compile_valueset.py could have written
fhir-pods-terminology 1
= https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1|1.0.0
https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\tAAA\tAccurate
https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\tAAU\tDay unknown

= http://example.org/ValueSet/hr-codes
http://loinc.org\t8867-4\tHeart rate
";

#[test]
fn a_compiled_table_is_complete_for_its_sets_and_unknown_elsewhere() {
    let t = Expansions::parse(TABLE).unwrap();
    assert_eq!(t.value_sets().collect::<Vec<_>>().len(), 2);
    assert_eq!(t.len(), 3);
    let vs = "https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1";
    let cs = "https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1";
    assert_eq!(t.validate_code(vs, cs, "AAA"), Membership::Member);
    assert_eq!(t.validate_code(&format!("{vs}|2.0.0"), "", "AAU"), Membership::Member);
    assert_eq!(t.validate_code(vs, cs, "ZZZ"), Membership::NotMember);
    assert_eq!(t.validate_code(vs, "http://other", "AAA"), Membership::NotMember);
    assert_eq!(t.validate_code("http://example.org/ValueSet/unknown", cs, "AAA"), Membership::Unknown);
    assert_eq!(t.display("http://loinc.org", "8867-4").as_deref(), Some("Heart rate"));
    assert_eq!(t.display("http://loinc.org", "0000-0"), None);

    // through validation, after the bundled sets
    let ok = patient_with_date_accuracy("AAA");
    let r = wire::report(&validate_bytes_with(ok.as_bytes(), &Chain(&[&Bundled, &t])));
    assert!(!r.contains("date-accuracy"), "{r}");
    let bad = patient_with_date_accuracy("ZZZ");
    let r = wire::report(&validate_bytes_with(bad.as_bytes(), &Chain(&[&Bundled, &t])));
    assert!(r.contains("'ZZZ' is not in the bound value set"), "{r}");
    let r = wire::report(&validate_bytes(bad.as_bytes()));
    assert!(
        r.contains(
            "no bundled terminology for https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1"
        ),
        "{r}"
    );
}

#[test]
fn table_parse_errors_name_the_line() {
    assert_eq!(Expansions::parse("").unwrap_err().line, 0);
    let e = Expansions::parse("fhir-pods-terminology 2\n").unwrap_err();
    assert_eq!((e.line, e.msg), (1, "expected header `fhir-pods-terminology 1`"));
    let e = Expansions::parse("fhir-pods-terminology 1\nhttp://s\tc\n").unwrap_err();
    assert_eq!(e.line, 2);
    let e = Expansions::parse("fhir-pods-terminology 1\n= http://vs\nnot-a-row\n").unwrap_err();
    assert_eq!((e.line, e.msg), (3, "expected `system<TAB>code[<TAB>display]`"));
    // CRLF and duplicate sets merge
    let t =
        Expansions::parse("fhir-pods-terminology 1\r\n= http://vs\r\nhttp://s\ta\r\n= http://vs\r\nhttp://s\tb\r\n")
            .unwrap();
    assert_eq!(t.len(), 2);
}

// --- displays into lowering ---

#[test]
fn lowering_fills_displays_the_seam_knows() {
    let hr = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-heartrate";
    let inputs = r#"{"status":"final","subject":"Patient/1","effectiveDateTime":"2026-09-16","valueQuantity.value":72,"valueQuantity.unit":"/min"}"#;
    let v = Validator::new();
    // bundled: the fixed category coding gets the code system's display
    let l = v.lower(hr, inputs.as_bytes()).unwrap();
    assert!(l.validation.is_valid(), "{}", wire::report(&l.validation));
    assert!(l.resource.contains(r#"{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs","display":"Vital Signs"}"#), "{}", l.resource);
    // LOINC is not bundled: no display on the fixed code…
    assert!(l.resource.contains(r#"{"system":"http://loinc.org","code":"8867-4"}"#), "{}", l.resource);
    // …until a compiled table supplies one
    let t = Expansions::parse(TABLE).unwrap();
    let l = v.lower_with(hr, inputs.as_bytes(), &Chain(&[&Bundled, &t])).unwrap();
    assert!(
        l.resource.contains(r#"{"system":"http://loinc.org","code":"8867-4","display":"Heart rate"}"#),
        "{}",
        l.resource
    );
    // a host coding keeps the display it gave
    let cond = "http://hl7.org.au/fhir/core/StructureDefinition/au-core-condition";
    let inputs = r#"{"clinicalStatus":{"system":"http://terminology.hl7.org/CodeSystem/condition-clinical","code":"active","display":"Mine"},"code":{"text":"c"},"subject":"Patient/1"}"#;
    let l = v.lower(cond, inputs.as_bytes()).unwrap();
    assert!(l.resource.contains(r#""code":"active","display":"Mine""#), "{}", l.resource);
    let inputs = r#"{"clinicalStatus":{"system":"http://terminology.hl7.org/CodeSystem/condition-clinical","code":"active"},"code":{"text":"c"},"subject":"Patient/1"}"#;
    let l = v.lower(cond, inputs.as_bytes()).unwrap();
    assert!(l.resource.contains(r#""code":"active","display":"Active""#), "{}", l.resource);
}
