//! The lowering proof. For every corpus example that declares a registry
//! profile, read the values at the profile's slots out of the example, lower
//! them, and write the result as `lowered-<name>.json` next to it — so the
//! conformance harness can put every lowered resource in front of the
//! reference validator. Prints one line per example: whether our own
//! validation of the lowered resource is clean.
//!
//!     cargo run -p fhir-pods-core --example lower_corpus -- conformance/corpus
use std::{env, fs};

use fhir_pods_core::document::{Document, Parsed};
use fhir_pods_core::{generated, wire, Validator};

fn main() {
    let dir = env::args().nth(1).unwrap_or_else(|| "conformance/corpus".into());
    let validator = Validator::new();
    let (mut n, mut clean) = (0, 0);
    let mut manifest_lines = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&dir).expect("corpus dir").flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".json")
            || name.starts_with('_')
            || name.ends_with("manifest.json")
            || name.starts_with("lowered-")
            || name.starts_with("deid-")
            || name.starts_with("audit-")
        {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let Ok(parsed) = Parsed::parse(&bytes) else { continue };
        let doc = parsed.doc();
        let declared = doc.scalars("$this.meta.profile");
        let Some(profile) =
            generated::REGISTRY.iter().find(|p| declared.iter().any(|u| u.split('|').next() == Some(p.url)))
        else {
            continue;
        };
        // only publisher-valid examples are lowered: a broken probe's slots are broken too;
        // a document (Bundle, Composition) is assembled by the document pod, not lowered from slots
        if !validator.validate(&bytes).is_valid() || matches!(profile.resource_type, "Bundle" | "Composition") {
            continue;
        }
        // lift: the example read back into the profile's slots — what a host
        // maps to its own model; then lowered again, for the oracle to judge
        let lifted = validator.lift(profile.url, &bytes).expect("a registry example lifts");
        let inputs = lifted.inputs;
        if std::env::var_os("FHIR_PODS_TRACE").is_some() {
            eprintln!("{name} inputs: {inputs}");
        }
        let lowered = validator.lower(profile.url, inputs.as_bytes()).expect("lowerable");
        n += 1;
        let ok = lowered.validation.is_valid();
        if ok {
            clean += 1;
        }
        let out = path.with_file_name(format!("lowered-{name}"));
        fs::write(&out, format!("{}\n", lowered.resource)).unwrap();
        manifest_lines.push(format!(
            "{{\"file\": \"lowered-{name}\", \"profile\": \"{}\", \"expect\": {ok}, \"ig\": \"lowered\"}}",
            profile.url
        ));
        let status = if ok { "ok".to_string() } else { wire::report(&lowered.validation) };
        println!("{:<60} {status}", format!("lowered-{name}"));
    }
    fs::write(format!("{dir}/lowered-manifest.json"), format!("[\n{}\n]\n", manifest_lines.join(",\n"))).unwrap();
    println!("lowered {n} examples; {clean} validate clean on our side");
    // a lowered copy's verdict is recorded in the manifest and judged by the
    // oracle: a copy that is not clean (a contained resource lifted whose
    // referencing element is no slot — dom-3) is a disagreement only if the
    // oracle disagrees. Only a copy that could not be built is a failure.
}
