#![no_main]
use libfuzzer_sys::fuzz_target;

// Any bytes as an HL7 v2 message, under each of the option shapes: parse,
// convert, validate — never panic.
fuzz_target!(|data: &[u8]| {
    let Some((&pick, message)) = data.split_first() else { return };
    let options = match pick % 4 {
        0 => br#"{}"#.as_slice(),
        1 => br#"{"timezone":"+10:00"}"#.as_slice(),
        2 => br#"{"timezone":"Z","codeSystems":{"L":"http://example.org/local","99LAB":"urn:oid:1.2.3"}}"#.as_slice(),
        _ => message.get(..message.len().min(32)).unwrap_or(b""),
    };
    let Ok(options) = fhir_pods_core::v2::convert::Options::from_json(options) else { return };
    let v = fhir_pods_core::Validator::new();
    let _ = v.v2(message, &options);
});
