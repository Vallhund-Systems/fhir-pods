//! Both JSON entrypoints — the zero-copy top-level scanner and the value tree.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = fhir_pods_core::json::scan_top_level(data);
    if let Ok(v) = fhir_pods_core::json::parse(data) {
        // exercise the navigator on whatever parsed
        let _ = fhir_pods_core::json::resolve(&v, "$this.value[x].code");
        let _ = fhir_pods_core::json::resolve(&v, "Patient.name.given");
    }
});
