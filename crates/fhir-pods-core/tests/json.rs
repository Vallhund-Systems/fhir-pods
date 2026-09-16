mod common;
use common::*;

// --- JSON strings are decoded before anything compares them ---

#[test]
fn json_escapes_are_decoded() {
    use fhir_pods_core::json::{parse, Value};
    let v = parse(br#"{"a\u006de":"http:\/\/loinc.org","q":"say \"hi\"\n","emoji":"\ud83d\ude00","lone":"\ud83d"}"#)
        .unwrap();
    let Value::Object(kvs) = v else { panic!() };
    let get = |k: &str| kvs.iter().find(|(kk, _)| kk.as_ref() == k).map(|(_, v)| v.clone()).unwrap();
    assert_eq!(get("ame"), Value::Str("http://loinc.org".into()));
    assert_eq!(get("q"), Value::Str("say \"hi\"\n".into()));
    assert_eq!(get("emoji"), Value::Str("\u{1F600}".into()));
    assert_eq!(get("lone"), Value::Str("\u{FFFD}".into()));
    assert!(parse(br#""\x""#).is_err());
    assert!(parse(br#"{} trailing"#).is_err());
    assert!(parse(b"{}  \n").is_ok());
}

#[test]
fn escaped_slashes_still_match_fixed_systems_and_slices() {
    let r = probe_str(&HEARTRATE_OK.replace("http://loinc.org", "http:\\/\\/loinc.org"));
    assert!(r.contains("\"valid\":true"), "{r}");
}

#[test]
fn primitive_extension_nulls_are_placeholders() {
    use fhir_pods_core::document::{Document, Parsed};
    let d = Parsed::parse(br#"{"resourceType":"Patient","name":[{"_given":[null,{"extension":[{"url":"http://x","valueString":"y"}]}]}]}"#).unwrap();
    assert_eq!(d.doc().child_counts("Patient.name.given"), vec![1]);
}
