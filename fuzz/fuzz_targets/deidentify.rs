#![no_main]
use libfuzzer_sys::fuzz_target;

use fhir_pods_core::deidentify::Policy;
use fhir_pods_core::Validator;

// Whatever the bytes, de-identification never panics; when it produces a
// copy, the copy is well-formed JSON and the Safe Harbor check finds nothing.
fuzz_target!(|data: &[u8]| {
    static V: std::sync::OnceLock<Validator> = std::sync::OnceLock::new();
    let v = V.get_or_init(Validator::new);
    if let Ok(d) = v.deidentify(data, &Policy::safe_harbor(b"fuzz").today(2026)) {
        fhir_pods_core::json::parse(d.resource.as_bytes()).expect("well-formed");
        assert!(d.remaining.is_empty(), "{:?} in {}", d.remaining, d.resource);
    }
});
