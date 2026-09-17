#![no_main]
use libfuzzer_sys::fuzz_target;

// Any bytes as an HL7 v2 message: parse, convert, validate — never panic.
fuzz_target!(|data: &[u8]| {
    let v = fhir_pods_core::Validator::new();
    let _ = v.v2(data, &fhir_pods_core::v2::convert::Options::default());
});
