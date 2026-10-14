//! Host options: the two strictness calls a host may make, and the refusal
//! of an options document that does not say what it means.
//!
//! The defaults are the reference validator's own behaviour — an
//! unresolvable profile is a warning (D004, probed:
//! `conformance/probes/unknown-declared-profile.json`), and a narrative
//! reference out of the resource is accepted (`conformance/narrative/`). A
//! host asks for more; nothing asks for less.

use fhir_pods_core::options::{ExternalNarrativeLinks, Options, OptionsError, UnknownProfile};
use fhir_pods_core::validate::Severity;
use fhir_pods_core::Validator;

/// A Patient declaring a profile nothing has compiled.
const DECLARED: &[u8] = br#"{"resourceType":"Patient","id":"p","meta":{"profile":["http://example.org/StructureDefinition/not-compiled"]},"gender":"female"}"#;

/// A narrative whose image is fetched from somewhere else entirely.
const EXTERNAL_IMAGE: &str = r#"{"resourceType":"Patient","id":"p","text":{"status":"generated","div":"<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>Summary.</p><img src=\"http://example.org/x.png\" alt=\"t\"/></div>"},"gender":"female"}"#;

fn finding(v: &fhir_pods_core::Validation, id: &str) -> Option<Severity> {
    v.report.issues.iter().find(|i| i.id == id).map(|i| i.severity)
}

#[test]
fn an_uncompiled_declared_profile_is_a_warning_by_default_and_an_error_when_the_host_fails_closed() {
    let mut validator = Validator::new();
    let v = validator.validate(DECLARED);
    assert_eq!(finding(&v, "profile.unknown"), Some(Severity::Warning));
    assert!(v.is_valid(), "the oracle's severity leaves the resource valid: {:?}", v.report.issues);

    validator.set_options(Options { unknown_profile: UnknownProfile::Error, ..Options::default() });
    let v = validator.validate(DECLARED);
    assert_eq!(finding(&v, "profile.unknown"), Some(Severity::Error));
    assert!(!v.is_valid(), "fail closed: the host will not accept an unverified claim");
    // the same finding, named the same way: only the severity moved
    let issue = v.report.issues.iter().find(|i| i.id == "profile.unknown").unwrap();
    assert!(issue.message.contains("is not compiled here, so it was not checked"), "{}", issue.message);
    assert_eq!(issue.path, "meta.profile");

    // cleared — back to the oracle's behaviour
    validator.set_options(Options::default());
    assert_eq!(finding(&validator.validate(DECLARED), "profile.unknown"), Some(Severity::Warning));
    assert!(validator.validate(DECLARED).is_valid());
}

#[test]
fn a_narrative_reference_out_of_the_resource_is_refused_only_when_the_host_asks() {
    let mut validator = Validator::new();
    assert!(validator.validate(EXTERNAL_IMAGE.as_bytes()).is_valid());

    validator.set_options(Options { external_narrative_links: ExternalNarrativeLinks::Refuse, ..Options::default() });
    let v = validator.validate(EXTERNAL_IMAGE.as_bytes());
    assert!(!v.is_valid());
    let issue = v.report.issues.iter().find(|i| i.id == "narrative.external-link").expect("the refusal");
    assert_eq!(issue.severity, Severity::Error);
    assert_eq!(
        issue.message,
        "The narrative image source 'http://example.org/x.png' at 'Patient.text.div/img' is outside the resource"
    );
    // a hyperlink the same way, and `javascript:` in an image — which the
    // oracle does not check at all — with it
    let active = EXTERNAL_IMAGE.replace("http://example.org/x.png", "javascript:alert(1)");
    assert!(!validator.validate(active.as_bytes()).is_valid());
    let link = EXTERNAL_IMAGE
        .replace(r#"<img src=\"http://example.org/x.png\" alt=\"t\"/>"#, r#"<a href=\"file:///etc/x\">t</a>"#);
    let v = validator.validate(link.as_bytes());
    assert_eq!(
        v.report.issues.iter().find(|i| i.id == "narrative.external-link").map(|i| i.message.as_str()),
        Some("The narrative hyperlink 'file:///etc/x' at 'div/a' is outside the resource")
    );

    // what stays inside the resource is untouched: a fragment that resolves,
    // and a self-contained data: URL
    let inside = EXTERNAL_IMAGE.replace("http://example.org/x.png", "#p");
    assert!(validator.validate(inside.as_bytes()).is_valid());
    let data = EXTERNAL_IMAGE.replace("http://example.org/x.png", "data:image/png;base64,iVBORw0KGgo=");
    assert!(validator.validate(data.as_bytes()).is_valid());

    // the option never moves txt-1: it is the invariant's own question
    assert!(!v.report.issues.iter().any(|i| i.id == "inv.txt-1"), "{:?}", v.report.issues);

    validator.set_options(Options::default());
    assert!(validator.validate(EXTERNAL_IMAGE.as_bytes()).is_valid());
}

#[test]
fn an_options_document_that_does_not_say_what_it_means_is_refused() {
    assert_eq!(Options::from_json(b""), Ok(Options::default()));
    assert_eq!(Options::from_json(b"  \n"), Ok(Options::default()));
    assert_eq!(Options::from_json(b"{}"), Ok(Options::default()));
    assert_eq!(
        Options::from_json(br#"{"unknownProfile":"error","externalNarrativeLinks":"refuse"}"#),
        Ok(Options {
            unknown_profile: UnknownProfile::Error,
            external_narrative_links: ExternalNarrativeLinks::Refuse
        })
    );
    // a typo in a strictness setting must not read as "off"
    assert_eq!(
        Options::from_json(br#"{"unknownProfle":"error"}"#),
        Err(OptionsError::UnknownMember("unknownProfle".into()))
    );
    assert_eq!(
        Options::from_json(br#"{"unknownProfile":"fatal"}"#),
        Err(OptionsError::BadValue("unknownProfile".into()))
    );
    assert_eq!(Options::from_json(br#"{"unknownProfile":true}"#), Err(OptionsError::BadValue("unknownProfile".into())));
    assert_eq!(
        Options::from_json(br#"{"externalNarrativeLinks":"deny"}"#),
        Err(OptionsError::BadValue("externalNarrativeLinks".into()))
    );
    // a member given twice: which one the host meant is a guess
    assert_eq!(
        Options::from_json(br#"{"unknownProfile":"error","unknownProfile":"warning"}"#),
        Err(OptionsError::DuplicateMember("unknownProfile".into()))
    );
    assert_eq!(Options::from_json(b"[]"), Err(OptionsError::NotAnObject));
    assert_eq!(Options::from_json(b"not json"), Err(OptionsError::NotAnObject));
    assert_eq!(Options::from_json(br#""error""#), Err(OptionsError::NotAnObject));
}

#[test]
fn options_survive_a_questionnaire_load_and_a_load_survives_the_options() {
    const FORM: &[u8] = br#"{"resourceType":"Questionnaire","id":"q","url":"http://example.org/Questionnaire/q","status":"active","item":[{"linkId":"a","type":"string","required":true}]}"#;
    const RESPONSE: &[u8] = br#"{"resourceType":"QuestionnaireResponse","id":"r","questionnaire":"http://example.org/Questionnaire/q","status":"completed"}"#;
    let mut validator = Validator::new();
    validator.set_options(Options { unknown_profile: UnknownProfile::Error, ..Options::default() });
    assert!(validator.load_questionnaire(FORM));
    assert_eq!(validator.options().unknown_profile, UnknownProfile::Error);
    // the loaded form is still in reach, and the options still in force
    let v = validator.validate(RESPONSE);
    assert!(v.report.issues.iter().any(|i| i.id == "questionnaire.required-answer"), "{:?}", v.report.issues);
    assert_eq!(finding(&validator.validate(DECLARED), "profile.unknown"), Some(Severity::Error));
}
