mod common;
use common::*;

// --- the terminology seam ---

#[test]
fn host_terminology_answers_what_the_bundled_sets_cannot() {
    use fhir_pods_core::terminology::{Bundled, Chain, Membership, TerminologyProvider};
    use fhir_pods_core::{validate_bytes, validate_bytes_with, wire};
    // the date-accuracy extension's coding is bound (required) to an
    // NCTS-hosted set the core does not bundle; telecom.system, bundled, is
    // never asked about
    let patient = PATIENT_OK.replace(
        "\"gender\"",
        "\"_birthDate\":{\"extension\":[{\"url\":\"http://hl7.org.au/fhir/StructureDefinition/date-accuracy-indicator\",\"valueCoding\":{\"system\":\"https://healthterminologies.gov.au/fhir/CodeSystem/date-accuracy-indicator-1\",\"code\":\"AAA\"}}]},\"telecom\":[{\"system\":\"phone\",\"value\":\"1\"}],\"gender\"",
    );
    let bundled = wire::report(&validate_bytes(patient.as_bytes()));
    assert!(
        bundled.contains(
            "no bundled terminology for https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1"
        ),
        "{bundled}"
    );

    struct Host(Membership, core::cell::Cell<usize>);
    impl TerminologyProvider for Host {
        fn validate_code(&self, vs: &str, system: &str, code: &str) -> Membership {
            self.1.set(self.1.get() + 1);
            assert!(
                vs.starts_with("https://healthterminologies.gov.au/fhir/ValueSet/date-accuracy-indicator-1"),
                "asked about {vs} {system} {code}"
            );
            self.0
        }
    }
    let yes = Host(Membership::Member, Default::default());
    let r = wire::report(&validate_bytes_with(patient.as_bytes(), &Chain(&[&Bundled, &yes])));
    assert!(!r.contains("date-accuracy-indicator"), "{r}");
    assert_eq!(yes.1.get(), 1, "asked once, only for the set the bundle lacks");

    let no = Host(Membership::NotMember, Default::default());
    let r = wire::report(&validate_bytes_with(patient.as_bytes(), &Chain(&[&Bundled, &no])));
    assert!(r.contains("\"message\":\"'AAA' is not in the bound value set\""), "{r}");
    assert!(r.contains("\"valid\":false"), "{r}");
}

#[test]
fn codeable_concept_binding_is_member_if_any_coding_is() {
    use fhir_pods_core::terminology::{Membership, TerminologyProvider};
    use fhir_pods_core::{validate_bytes_with, wire};
    struct Only(&'static str);
    impl TerminologyProvider for Only {
        fn validate_code(&self, _vs: &str, _system: &str, code: &str) -> Membership {
            if code == self.0 {
                Membership::Member
            } else {
                Membership::NotMember
            }
        }
    }
    // Condition.clinicalStatus: required binding, CodeableConcept.
    let cond = |code: &str| {
        format!(
            r#"{{"resourceType":"Condition","meta":{{"profile":["http://hl7.org.au/fhir/core/StructureDefinition/au-core-condition"]}},"clinicalStatus":{{"coding":[{{"system":"http://terminology.hl7.org/CodeSystem/condition-clinical","code":"{code}"}},{{"system":"http://example.org/other","code":"x"}}]}},"code":{{"text":"c"}},"subject":{{"reference":"Patient/1"}}}}"#
        )
    };
    let ok = wire::report(&validate_bytes_with(cond("active").as_bytes(), &Only("active")));
    assert!(!ok.contains("clinicalStatus"), "{ok}");
    let bad = wire::report(&validate_bytes_with(cond("bogus").as_bytes(), &Only("active")));
    assert!(bad.contains("Condition.clinicalStatus") && bad.contains("not in the bound value set"), "{bad}");
}
