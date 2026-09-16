#![no_main]
use libfuzzer_sys::fuzz_target;

// The first byte picks a profile; the rest are the inputs.
fuzz_target!(|data: &[u8]| {
    let Some((&pick, inputs)) = data.split_first() else { return };
    let reg = fhir_pods_core::generated::REGISTRY;
    let profile = reg[pick as usize % reg.len()];
    let _ = fhir_pods_core::lower_bytes(profile.url, inputs);
});
