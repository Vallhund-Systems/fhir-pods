mod common;

use fhir_pods_core::assemble::{AssembleError, Kind};
use fhir_pods_core::{wire, Validator};

const PATIENT: &str = r#"{"resourceType":"Patient","id":"p1","identifier":[{"system":"http://ns.electronichealth.net.au/id/hi/ihi/1.0","value":"8003608833357361"}],"name":[{"family":"Doe","given":["Jane"]}],"gender":"female","birthDate":"1970-06-15"}"#;
const PRACTITIONER: &str = r#"{"resourceType":"Practitioner","id":"gp1","identifier":[{"system":"http://ns.electronichealth.net.au/id/hi/hpii/1.0","value":"8003619900015717"}],"name":[{"family":"Who","given":["Doctor"]}]}"#;
const CONDITION: &str = r#"{"resourceType":"Condition","id":"c1","clinicalStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-clinical","code":"active"}]},"category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/condition-category","code":"problem-list-item"}]}],"code":{"text":"Asthma"},"subject":{"reference":"Patient/p1"},"onsetDateTime":"2010"}"#;
const ALLERGY: &str = r#"{"resourceType":"AllergyIntolerance","id":"a1","clinicalStatus":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/allergyintolerance-clinical","code":"active"}]},"code":{"text":"Penicillin"},"patient":{"reference":"Patient/p1"}}"#;
const MEDICATION: &str = r#"{"resourceType":"MedicationStatement","id":"m1","status":"active","medicationCodeableConcept":{"text":"Salbutamol inhaler"},"subject":{"reference":"Patient/p1"},"effectiveDateTime":"2026"}"#;
const HEART_RATE: &str = r#"{"resourceType":"Observation","id":"hr1","status":"final","category":[{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/observation-category","code":"vital-signs"}]}],"code":{"coding":[{"system":"http://loinc.org","code":"8867-4"}]},"subject":{"reference":"Patient/p1"},"effectiveDateTime":"2026-09-16T08:00:00+10:00","valueQuantity":{"value":72,"unit":"/min","system":"http://unitsofmeasure.org","code":"/min"}}"#;

fn inputs(resources: &[&str], extra: &str) -> String {
    format!(
        r#"{{"resources":[{}],"author":"Practitioner/gp1","date":"2026-09-16T10:00:00+10:00","title":"Summary"{extra}}}"#,
        resources.join(",")
    )
}

#[test]
fn an_au_patient_summary_from_a_patient_and_four_resources() {
    let v = Validator::new();
    let a = v
        .document("au-ps", inputs(&[PATIENT, PRACTITIONER, CONDITION, ALLERGY, MEDICATION, HEART_RATE], "").as_bytes())
        .unwrap();
    let d = &a.document;
    assert!(a.validation.is_valid(), "{}", wire::report(&a.validation));
    // a document bundle with the composition first, the patient second
    assert!(d.starts_with(r#"{"resourceType":"Bundle","id":""#) && d.contains(r#""type":"document""#), "{d}");
    let first_two: Vec<&str> =
        d.matches(r#""resourceType":"Composition""#).chain(d.matches(r#""resourceType":"Patient""#)).collect();
    assert_eq!(first_two.len(), 2);
    assert!(d.find(r#""resourceType":"Composition""#) < d.find(r#""resourceType":"Patient""#));
    // sections come from the profile, with LOINC codes; each resource is in its section
    for (code, title) in [
        ("11450-4", "Problems"),
        ("48765-2", "Allergies and Intolerances"),
        ("10160-0", "Medications"),
        ("8716-3", "Vital Signs"),
    ] {
        assert!(
            d.contains(&format!(
                r#""title":"{title}","code":{{"coding":[{{"system":"http://loinc.org","code":"{code}"}}]}}"#
            )),
            "{title}: {d}"
        );
    }
    assert!(!d.contains("Immunizations"), "an empty optional section is left out");
    // references were rewritten to the entries' fullUrls, consistently
    assert!(!d.contains(r#""reference":"Patient/p1""#), "{d}");
    let patient_url = d.split(r#""fullUrl":""#).nth(2).unwrap().split('"').next().unwrap();
    assert!(patient_url.starts_with("urn:uuid:"));
    assert!(d.contains(&format!(r#""subject":{{"reference":"{patient_url}"}}"#)));
    assert!(d.matches(&format!(r#""reference":"{patient_url}""#)).count() >= 5, "{d}");
    // the narrative is generated and says what is in each section
    assert!(
        d.contains("<td>Asthma</td>") && d.contains("<td>Penicillin</td>") && d.contains("<td>Salbutamol inhaler</td>"),
        "{d}"
    );
    assert!(d.contains(r#""status":"generated""#));
    // placement is reported
    assert!(
        a.placed.iter().any(|(_, s)| s == "11450-4") && a.placed.iter().any(|(_, s)| s.is_empty()),
        "{:?}",
        a.placed
    );
    // the same inputs make the same document
    let again = v
        .document("au-ps", inputs(&[PATIENT, PRACTITIONER, CONDITION, ALLERGY, MEDICATION, HEART_RATE], "").as_bytes())
        .unwrap();
    assert_eq!(again.document, a.document);
}

#[test]
fn entries_arrive_as_bundle_entries_too_and_unreachable_ones_are_left_out() {
    let v = Validator::new();
    // the practitioner given as a Bundle entry with its own fullUrl; an Organization nothing reaches
    let org = r#"{"resourceType":"Organization","id":"o9","name":"Nowhere Clinic"}"#;
    let json = format!(
        r#"{{"resources":[{PATIENT},{{"fullUrl":"urn:uuid:11111111-2222-4333-8444-555555555555","resource":{PRACTITIONER}}},{CONDITION},{org}],"author":["urn:uuid:11111111-2222-4333-8444-555555555555"],"date":"2026-09-16T10:00:00+10:00"}}"#
    );
    let a = v.document("ips", json.as_bytes()).unwrap();
    assert!(a.validation.is_valid(), "{}", wire::report(&a.validation));
    assert!(a.document.contains(r#""fullUrl":"urn:uuid:11111111-2222-4333-8444-555555555555""#), "{}", a.document);
    assert!(
        a.document.contains(r#""author":[{"reference":"urn:uuid:11111111-2222-4333-8444-555555555555"}]"#),
        "{}",
        a.document
    );
    assert_eq!(a.omitted, vec!["Organization/o9".to_string()]);
    assert!(!a.document.contains("Nowhere Clinic"));
}

#[test]
fn required_sections_with_nothing_in_them_say_so_and_the_document_still_conforms() {
    let v = Validator::new();
    let a = v.document("ips", inputs(&[PATIENT, PRACTITIONER], "").as_bytes()).unwrap();
    assert!(a.validation.is_valid(), "{}", wire::report(&a.validation));
    let d = &a.document;
    for title in ["Problems", "Allergies and Intolerances", "Medications"] {
        assert!(d.contains(&format!(r#""title":"{title}""#)), "{title} missing: {d}");
    }
    assert_eq!(d.matches(r#""emptyReason":{"coding":[{"system":"http://terminology.hl7.org/CodeSystem/list-empty-reason","code":"unavailable"}]}"#).count(), 3, "{d}");
    assert!(!d.contains("no-known"), "no clinical fact is invented for an empty section");
    assert!(d.contains("No information available"));
}

#[test]
fn inputs_are_checked_before_anything_is_built() {
    let v = Validator::new();
    assert_eq!(v.document("cda", inputs(&[PATIENT], "").as_bytes()).unwrap_err(), AssembleError::UnknownKind);
    assert_eq!(v.document("ips", b"[]").unwrap_err(), AssembleError::InputsNotAnObject);
    assert_eq!(v.document("ips", br#"{"resources":"x"}"#).unwrap_err(), AssembleError::ResourcesNotAnArray);
    assert_eq!(v.document("ips", inputs(&[PRACTITIONER], "").as_bytes()).unwrap_err(), AssembleError::NoPatient);
    assert_eq!(
        v.document("ips", format!(r#"{{"resources":[{PATIENT}],"date":"2026-09-16T10:00:00+10:00"}}"#).as_bytes())
            .unwrap_err(),
        AssembleError::Missing("author")
    );
    // a date that is not an instant needs a timestamp
    let e = v
        .document(
            "ips",
            format!(r#"{{"resources":[{PATIENT}],"author":"Practitioner/x","date":"2026-09-16"}}"#).as_bytes(),
        )
        .unwrap_err();
    assert!(matches!(e, AssembleError::Missing(m) if m.starts_with("timestamp")), "{e:?}");
    let ok = v.document("ips", format!(r#"{{"resources":[{PATIENT}],"author":"Practitioner/x","date":"2026-09-16","timestamp":"2026-09-16T10:00:00Z"}}"#).as_bytes()).unwrap();
    assert!(ok.document.contains(r#""timestamp":"2026-09-16T10:00:00Z""#));
    assert_eq!(Kind::parse("au-ps"), Some(Kind::AuPs));
}

#[test]
fn a_non_conformant_resource_shows_up_in_the_validation_not_as_a_crash() {
    let v = Validator::new();
    // an AU PS Condition needs a category; without it the entry matches no AU PS slice
    let bare = r#"{"resourceType":"Condition","id":"c2","code":{"text":"x"},"subject":{"reference":"Patient/p1"}}"#;
    let a = v.document("au-ps", inputs(&[PATIENT, PRACTITIONER, bare], "").as_bytes()).unwrap();
    let r = wire::report(&a.validation);
    assert!(!a.validation.is_valid(), "{r}");
    assert!(r.contains("Condition"), "{r}");
}
