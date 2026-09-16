//! The whole pipeline on arbitrary bytes: scan, parse, profile selection,
//! engine (slicing, inheritance, invariants), both wire renderings.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let v = fhir_pods_core::validate_bytes(data);
    let outcome = v.to_operation_outcome();
    let _ = fhir_pods_core::json::parse(outcome.as_bytes());
    let _ = fhir_pods_core::wire::report(&v);
});
