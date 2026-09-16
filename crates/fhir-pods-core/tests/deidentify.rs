mod common;
use common::*;

use fhir_pods_core::deidentify::{check, Action, DeidError, Policy, PolicyError};
use fhir_pods_core::hash::{hex, hmac_sha256, sha256};
use fhir_pods_core::{wire, Validator};

// --- the hash ---

#[test]
fn sha256_and_hmac_match_the_published_vectors() {
    assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    // two blocks, with the length straddling the padding boundary
    assert_eq!(
        hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(hex(&sha256(&[b'a'; 56])), "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a");
    assert_eq!(
        hex(&hmac_sha256(b"key", b"The quick brown fox jumps over the lazy dog")),
        "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
    );
    // a key longer than a block is hashed first (RFC 4231 case 6)
    assert_eq!(
        hex(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")),
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
}

#[test]
fn pseudonyms_keep_the_shape_of_what_they_replace() {
    let mrn = r#"{"resourceType":"Patient","identifier":[{"system":"http://h.example.org/mrn","value":"12345"},{"system":"urn:ietf:rfc:3986","value":"urn:uuid:8e3c1d2a-1b2c-4d5e-9f00-0a1b2c3d4e5f"},{"system":"http://hl7.org/fhir/sid/us-npi","value":"1234567893"},{"system":"http://ns.electronichealth.net.au/id/hi/ihi/1.0","value":"8003608833357361"}],"gender":"male"}"#;
    let d = deid(mrn, &Policy::safe_harbor(b"s"));
    let values: Vec<&str> = d.resource.split("\"value\":\"").skip(1).map(|s| s.split('"').next().unwrap()).collect();
    assert_eq!(values.len(), 4, "{}", d.resource);
    // same length and character classes; digits stay digits, a URI keeps its punctuation
    assert!(values[0].len() == 5 && values[0].chars().all(|c| c.is_ascii_digit()) && values[0] != "12345");
    assert!(
        values[1].starts_with(|c: char| c.is_ascii_lowercase())
            && values[1].matches('-').count() == 4
            && values[1].len() == 45,
        "{}",
        values[1]
    );
    // an NPI (Luhn over 80840 + 10 digits) and an IHI (16-digit Luhn) still pass their checks
    let luhn = |s: &str| {
        s.chars()
            .rev()
            .enumerate()
            .map(|(i, c)| {
                let d = c.to_digit(10).unwrap();
                if i % 2 == 1 {
                    let x = d * 2;
                    if x > 9 {
                        x - 9
                    } else {
                        x
                    }
                } else {
                    d
                }
            })
            .sum::<u32>()
            % 10
            == 0
    };
    assert!(
        values[2].len() == 10 && luhn(&format!("80840{}", values[2])) && values[2] != "1234567893",
        "{}",
        values[2]
    );
    assert!(values[3].len() == 16 && luhn(values[3]) && values[3] != "8003608833357361", "{}", values[3]);
}

// --- the walk ---

const PATIENT: &str = r#"{"resourceType":"Patient","id":"p1","meta":{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-patient"],"versionId":"3","lastUpdated":"2026-09-16T10:00:00Z"},
"text":{"status":"generated","div":"<div xmlns=\"http://www.w3.org/1999/xhtml\">Jane Doe</div>"},
"identifier":[{"system":"http://hospital.example.org/mrn","value":"12345"}],
"name":[{"family":"Doe","given":["Jane"],"use":"official"}],
"telecom":[{"system":"phone","value":"0400 000 000"}],
"gender":"female","birthDate":"1970-06-15",
"_birthDate":{"extension":[{"url":"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator","valueCoding":{"system":"https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1","code":"AAA"}}]},
"address":[{"line":["1 Example St"],"city":"Sydney","state":"NSW","postalCode":"2000","country":"AU"}],
"photo":[{"contentType":"image/jpeg","data":"AAAA"}],
"contact":[{"name":{"family":"Doe","given":["John"]},"telecom":[{"system":"phone","value":"1"}]}],
"generalPractitioner":[{"reference":"Practitioner/gp-9","display":"Dr Who"}],
"managingOrganization":{"reference":"Organization/org-1"}}"#;

fn deid(json: &str, policy: &Policy) -> fhir_pods_core::deidentify::Deidentified {
    Validator::new().deidentify(json.as_bytes(), policy).expect("deidentifiable")
}

#[test]
fn safe_harbor_strips_the_eighteen_and_keeps_the_copy_conformant() {
    let d = deid(PATIENT, &Policy::safe_harbor(b"secret-salt").today(2026));
    let r = &d.resource;
    // names, contacts, narrative, photo, address below state, versionId, lastUpdated: gone
    for gone in [
        "Jane",
        "Doe",
        "0400",
        "Example St",
        "Sydney",
        "2000",
        "image/jpeg",
        "Dr Who",
        "versionId",
        "lastUpdated",
        "\"div\"",
    ] {
        assert!(!r.contains(gone), "{gone} survived: {r}");
    }
    // the required name is masked, not dropped, so the AU Core profile still holds
    assert!(r.contains(r#""name":[{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/data-absent-reason","valueCode":"masked"}]}]"#), "{r}");
    assert!(r.contains(r#""address":[{"state":"NSW","country":"AU"}]"#), "{r}");
    assert!(r.contains(r#""birthDate":"1970""#), "{r}");
    // the date-accuracy extension on the carrier survives (nothing identifying in it)
    assert!(
        r.contains(
            r#""_birthDate":{"extension":[{"url":"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator""#
        ),
        "{r}"
    );
    // identifiers and references are pseudonymised, systems and types kept
    assert!(
        r.contains(r#""identifier":[{"system":"http://hospital.example.org/mrn","value":""#) && !r.contains("12345"),
        "{r}"
    );
    assert!(r.contains(r#""reference":"Practitioner/"#) && !r.contains("gp-9"), "{r}");
    assert!(!r.contains("\"id\":\"p1\""), "{r}");
    assert!(r.contains("\"gender\":\"female\""));
    // conformant, and nothing left for the checker
    assert!(d.validation.is_valid(), "{}", wire::report(&d.validation));
    assert_eq!(d.remaining, vec![], "{r}");
    assert!(d.applied.iter().any(|a| a.path == "Patient.name" && a.action == Action::Mask), "{:?}", d.applied);
    assert!(d.applied.iter().any(|a| a.path == "Patient.telecom" && a.action == Action::Remove));
    // the original fails the check on every count
    let before = check(PATIENT.as_bytes());
    for what in [
        "HumanName present",
        "ContactPoint present",
        "Narrative present",
        "Attachment present",
        "address below state",
        "date finer than a year",
        "instant finer than a year",
        "reference display",
    ] {
        assert!(before.iter().any(|f| f.what == what), "missing {what} in {before:?}");
    }
}

#[test]
fn pseudonyms_are_consistent_keyed_and_typed() {
    let a = deid(PATIENT, &Policy::safe_harbor(b"salt-A"));
    let b = deid(PATIENT, &Policy::safe_harbor(b"salt-A"));
    let c = deid(PATIENT, &Policy::safe_harbor(b"salt-B"));
    assert_eq!(a.resource, b.resource, "deterministic under one salt");
    assert_ne!(a.resource, c.resource, "a different salt maps differently");
    // the resource's own id and a reference to it agree
    let obs =
        r#"{"resourceType":"Observation","status":"final","code":{"text":"x"},"subject":{"reference":"Patient/p1"}}"#;
    let o = deid(obs, &Policy::safe_harbor(b"salt-A"));
    let pid = a.resource.split("\"id\":\"").nth(1).unwrap().split('"').next().unwrap();
    assert!(o.resource.contains(&format!("\"reference\":\"Patient/{pid}\"")), "{} vs {pid}", o.resource);
    // contained references keep their `#` and match the contained id
    let mr = r##"{"resourceType":"MedicationRequest","status":"active","intent":"order","contained":[{"resourceType":"Medication","id":"med1","code":{"text":"m"}}],"medicationReference":{"reference":"#med1"},"subject":{"reference":"Patient/p1"}}"##;
    let m = deid(mr, &Policy::safe_harbor(b"salt-A"));
    let cid = m.resource.split("\"id\":\"").nth(1).unwrap().split('"').next().unwrap();
    assert_ne!(cid, "med1");
    assert!(m.resource.contains(&format!("\"reference\":\"#{cid}\"")), "{}", m.resource);
    // an absolute reference keeps its base
    let abs = r#"{"resourceType":"Observation","status":"final","code":{"text":"x"},"subject":{"reference":"https://fhir.example.org/r4/Patient/p1/_history/2"}}"#;
    let o = deid(abs, &Policy::safe_harbor(b"salt-A"));
    assert!(
        o.resource.contains(&format!("\"reference\":\"https://fhir.example.org/r4/Patient/{pid}\"")),
        "{}",
        o.resource
    );
}

#[test]
fn ages_over_89_and_instants_and_unknown_elements() {
    let old = PATIENT.replace("1970-06-15", "1930-01-01");
    let d = deid(&old, &Policy::safe_harbor(b"s").today(2026));
    // AU Core requires birthDate: over 89 it is masked rather than aged
    assert!(!d.resource.contains("1930") && d.resource.contains(r#""_birthDate":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/data-absent-reason","valueCode":"masked"}]}"#), "{}", d.resource);
    assert!(d.validation.is_valid(), "{}", wire::report(&d.validation));
    // without today, the year stays
    let d = deid(&old, &Policy::safe_harbor(b"s"));
    assert!(d.resource.contains(r#""birthDate":"1930""#));
    // an Age quantity is clamped
    let cond = r#"{"resourceType":"Condition","code":{"text":"c"},"subject":{"reference":"Patient/1"},"onsetAge":{"value":93,"comparator":">","unit":"a","system":"http://unitsofmeasure.org","code":"a"}}"#;
    let d = deid(cond, &Policy::safe_harbor(b"s"));
    assert!(d.resource.contains(r#""onsetAge":{"value":90,"unit":"a""#), "{}", d.resource);
    // an element the base spec does not define cannot be typed: removed, recorded
    let odd = r#"{"resourceType":"Patient","gender":"male","favouriteColour":"blue"}"#;
    let d = deid(odd, &Policy::safe_harbor(b"s"));
    assert!(!d.resource.contains("favouriteColour"));
    assert!(d.applied.iter().any(|a| a.path == "Patient.favouriteColour" && a.action == Action::Remove));
    // an unknown resource type is refused, never guessed
    assert_eq!(
        Validator::new().deidentify(br#"{"resourceType":"Nope"}"#, &Policy::safe_harbor(b"s")).unwrap_err(),
        DeidError::UnknownResourceType
    );
    assert_eq!(Validator::new().deidentify(b"[1]", &Policy::safe_harbor(b"s")).unwrap_err(), DeidError::NotAResource);
}

#[test]
fn policy_rules_override_by_specificity_and_parse_from_json() {
    // keep the city; drop the gender — which AU Core requires, so it is masked
    let p = Policy::safe_harbor(b"s").rule("Patient.address.city", Action::Keep).rule("Patient.gender", Action::Remove);
    let d = deid(PATIENT, &p);
    assert!(d.resource.contains(r#""city":"Sydney""#) && !d.resource.contains("female"), "{}", d.resource);
    assert!(d.resource.contains(r#""_gender":{"extension":[{"url":"http://hl7.org/fhir/StructureDefinition/data-absent-reason","valueCode":"masked"}]}"#), "{}", d.resource);
    // the checker still objects to the city — the policy chose to keep it
    assert!(d.remaining.iter().any(|f| f.what == "address below state"));
    // the same through JSON, on top of safe harbor
    let json = br#"{"salt":"s","today":"2026-09-16","rules":[{"target":"Patient.address.city","action":"keep"},{"target":"Patient.gender","action":"remove"}]}"#;
    let pj = Policy::from_json(json).unwrap();
    assert_eq!(pj.today_year, Some(2026));
    assert_eq!(deid(PATIENT, &pj).resource, d.resource);
    // base none keeps everything
    let none = Policy::from_json(br#"{"salt":"s","base":"none"}"#).unwrap();
    let d = deid(PATIENT, &none);
    assert!(d.resource.contains("Jane") && d.resource.contains("0400"));
    assert!(!d.remaining.is_empty());
    // errors
    assert_eq!(Policy::from_json(b"{}").unwrap_err(), PolicyError::MissingSalt);
    assert_eq!(Policy::from_json(br#"{"salt":"s","base":"other"}"#).unwrap_err(), PolicyError::UnknownBase);
    assert_eq!(Policy::from_json(br#"{"salt":"s","today":"soon"}"#).unwrap_err(), PolicyError::BadToday);
    assert_eq!(
        Policy::from_json(br#"{"salt":"s","rules":[{"target":"x","action":"vanish"}]}"#).unwrap_err(),
        PolicyError::BadRule(0)
    );
    assert_eq!(Policy::from_json(b"[]").unwrap_err(), PolicyError::NotAnObject);
}

#[test]
fn every_registry_example_shape_survives_the_walk() {
    // the shared fixtures: de-identifiable, well-formed, and clean for the checker
    let v = Validator::new();
    for src in [PATIENT_OK, HEARTRATE_OK] {
        let d = v.deidentify(src.as_bytes(), &Policy::safe_harbor(b"s")).unwrap();
        fhir_pods_core::json::parse(d.resource.as_bytes()).expect("well-formed");
        assert_eq!(d.remaining, vec![], "{}", d.resource);
    }
    // conformance "where the profile permits": a vital sign's effective[x]
    // must be precise to the day (vs-1), which a year-only date is not — the
    // validation says so, and a policy may choose to keep that one date
    let d = v.deidentify(HEARTRATE_OK.as_bytes(), &Policy::safe_harbor(b"s")).unwrap();
    assert!(wire::report(&d.validation).contains("vs-1"), "{}", wire::report(&d.validation));
    let keep_effective = Policy::safe_harbor(b"s").rule("Observation.effectiveDateTime", Action::Keep);
    let d = v.deidentify(HEARTRATE_OK.as_bytes(), &keep_effective).unwrap();
    assert!(d.validation.is_valid(), "{}", wire::report(&d.validation));
    assert!(d.remaining.iter().any(|f| f.what == "date finer than a year"));
    // the emptied contact backbone element is gone, not left as `{}`
    let d = v.deidentify(PATIENT.as_bytes(), &Policy::safe_harbor(b"s")).unwrap();
    assert!(!d.resource.contains("\"contact\""), "{}", d.resource);
    assert!(d.applied.iter().any(|a| a.path == "Patient.contact" && a.action == Action::Remove));
}
