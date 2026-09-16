//! FHIRPath: parse arbitrary text; evaluate whatever parses against a fixed
//! resource so the evaluator is exercised too.
#![no_main]
use libfuzzer_sys::fuzz_target;

const DOC: &[u8] = br#"{"resourceType":"Observation","status":"final","code":{"coding":[{"system":"http://loinc.org","code":"8867-4"}]},"component":[{"code":{"text":"x"}}],"valueQuantity":{"value":72,"code":"/min"},"deceasedBoolean":true}"#;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else { return };
    if let Ok(expr) = fhir_pods_core::path::parse(src) {
        if let Ok(parsed) = fhir_pods_core::document::Parsed::parse(DOC) {
            let _ = fhir_pods_core::path::check(&expr, &parsed.doc());
        }
    }
});
